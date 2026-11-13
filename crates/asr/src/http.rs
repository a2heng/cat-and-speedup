use std::io::Write as _;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cat_speedup_core::{Transcript, TranscriptSegment};
use serde::Deserialize;

use crate::{AsrEngine, AsrError, AsrOptions, AsrResult, split_long_segments};

/// Generic OpenAI-compatible `/v1/audio/transcriptions` backend.
///
/// This is the escape hatch for running FunASR / Qwen3-ASR / FireRedASR2 as a
/// local HTTP service without linking their Python runtimes into the app.
pub struct HttpAsrEngine {
    endpoint: String,
    api_key: Option<String>,
    model: String,
    timeout: Duration,
}

impl HttpAsrEngine {
    pub fn new(
        endpoint: impl Into<String>,
        api_key: Option<String>,
        model: impl Into<String>,
        timeout_secs: u64,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            api_key,
            model: model.into(),
            timeout: Duration::from_secs(timeout_secs.max(10)),
        }
    }
}

impl AsrEngine for HttpAsrEngine {
    fn name(&self) -> &str {
        "http"
    }

    fn transcribe_wav(&mut self, path: &Path, options: &AsrOptions) -> Result<AsrResult, AsrError> {
        let audio = std::fs::read(path)?;
        let boundary = boundary();
        let body = multipart_body(&boundary, &self.model, options, &audio, "asr.wav");

        let agent = ureq::AgentBuilder::new().timeout(self.timeout).build();
        let content_type = format!("multipart/form-data; boundary={boundary}");
        let mut request = agent
            .post(&self.endpoint)
            .set("Content-Type", &content_type);
        if let Some(key) = &self.api_key {
            request = request.set("Authorization", &format!("Bearer {key}"));
        }

        let response = request.send_bytes(&body).map_err(|error| match error {
            ureq::Error::Status(code, response) => AsrError::Engine(format!(
                "HTTP {code}: {}",
                response.into_string().unwrap_or_default()
            )),
            other => AsrError::Engine(other.to_string()),
        })?;

        let text = response
            .into_string()
            .map_err(|error| AsrError::Engine(format!("could not read response: {error}")))?;
        let duration_ms = wav_duration_ms(path).unwrap_or(0);
        let transcript = parse_response(&text, duration_ms)?;

        Ok(AsrResult {
            transcript,
            engine: "http".into(),
            model: self.model.clone(),
            device: "remote".into(),
        })
    }
}

fn boundary() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("----cat-speedup-{}-{nanos}", std::process::id())
}

fn multipart_body(
    boundary: &str,
    model: &str,
    options: &AsrOptions,
    audio: &[u8],
    filename: &str,
) -> Vec<u8> {
    let mut body = Vec::with_capacity(audio.len() + 1024);

    push_field(&mut body, boundary, "model", model);
    push_field(&mut body, boundary, "response_format", "verbose_json");
    push_field(&mut body, boundary, "timestamp_granularities[]", "segment");
    push_field(&mut body, boundary, "temperature", "0");

    if let Some(language) = &options.language {
        push_field(&mut body, boundary, "language", language);
    }
    if let Some(prompt) = &options.initial_prompt {
        push_field(&mut body, boundary, "prompt", prompt);
    }

    write!(
        body,
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: audio/wav\r\n\r\n"
    )
    .expect("writing to Vec cannot fail");
    body.extend_from_slice(audio);
    body.extend_from_slice(b"\r\n");
    write!(body, "--{boundary}--\r\n").expect("writing to Vec cannot fail");
    body
}

fn push_field(body: &mut Vec<u8>, boundary: &str, name: &str, value: &str) {
    write!(
        body,
        "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
    )
    .expect("writing to Vec cannot fail");
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    #[serde(default)]
    text: String,
    #[serde(default)]
    segments: Vec<ApiSegment>,
}

#[derive(Debug, Deserialize)]
struct ApiSegment {
    start: f64,
    end: f64,
    #[serde(default)]
    text: String,
}

fn parse_response(json: &str, duration_ms: u64) -> Result<Transcript, AsrError> {
    let response: ApiResponse = serde_json::from_str(json)
        .map_err(|error| AsrError::Engine(format!("invalid JSON response: {error}")))?;

    let mut segments = Vec::new();
    for segment in response.segments {
        let start_ms = (segment.start.max(0.0) * 1000.0).round() as u64;
        let end_ms = (segment.end.max(0.0) * 1000.0).round() as u64;
        let text = segment.text.trim().to_string();
        if end_ms > start_ms && !text.is_empty() {
            segments.push(TranscriptSegment {
                start_ms,
                end_ms,
                text,
            });
        }
    }

    if segments.is_empty() && !response.text.trim().is_empty() {
        segments.push(TranscriptSegment {
            start_ms: 0,
            end_ms: duration_ms,
            text: response.text.trim().to_string(),
        });
    }

    Ok(split_long_segments(Transcript { segments }, 32))
}

fn wav_duration_ms(path: &Path) -> Option<u64> {
    let reader = hound::WavReader::open(path).ok()?;
    let spec = reader.spec();
    if spec.sample_rate == 0 {
        return None;
    }
    Some(reader.duration() as u64 * 1000 / spec.sample_rate as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openai_segments() {
        let json = r#"{
            "text": "你好世界",
            "segments": [
                {"start": 0.5, "end": 1.5, "text": "你好"},
                {"start": 1.5, "end": 2.5, "text": "世界"}
            ]
        }"#;
        let transcript = parse_response(json, 3000).unwrap();
        assert_eq!(transcript.segments.len(), 2);
        assert_eq!(transcript.segments[0].start_ms, 500);
        assert_eq!(transcript.segments[1].end_ms, 2500);
    }

    #[test]
    fn falls_back_to_plain_text() {
        let transcript = parse_response(r#"{"text":"没有时间戳的文本"}"#, 4000).unwrap();
        assert_eq!(transcript.segments.len(), 1);
        assert_eq!(transcript.segments[0].start_ms, 0);
        assert_eq!(transcript.segments[0].end_ms, 4000);
    }

    #[test]
    fn rejects_invalid_json() {
        assert!(parse_response("not json", 0).is_err());
    }
}
