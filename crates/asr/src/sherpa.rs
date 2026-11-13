use std::path::{Path, PathBuf};
use std::process::Command;

use cat_speedup_core::{Transcript, TranscriptSegment};
use serde::Deserialize;

use crate::{AsrEngine, AsrError, AsrOptions, AsrResult, split_long_segments};

/// Offline backend backed by the `sherpa-onnx-offline` executable.
///
/// The same model family is available for Windows and Android. Android uses the
/// sherpa-onnx Kotlin API with the same exported ONNX files; desktop shells out
/// to the official CLI so both platforms share model behavior.
pub struct SherpaEngine {
    executable: PathBuf,
    model_dir: PathBuf,
    kind: SherpaModelKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SherpaModelKind {
    FunAsrNano,
    Paraformer,
}

impl SherpaModelKind {
    pub fn parse(value: &str) -> Result<Self, AsrError> {
        match value.to_ascii_lowercase().as_str() {
            "funasr-nano" | "funasr_nano" | "nano" => Ok(Self::FunAsrNano),
            "paraformer" | "paraformer-zh" => Ok(Self::Paraformer),
            other => Err(AsrError::Engine(format!(
                "unknown sherpa model kind '{other}' (expected funasr-nano or paraformer)"
            ))),
        }
    }
}

impl SherpaEngine {
    pub fn new(
        executable: impl Into<PathBuf>,
        model_dir: impl Into<PathBuf>,
        kind: SherpaModelKind,
    ) -> Result<Self, AsrError> {
        let executable = executable.into();
        let model_dir = model_dir.into();
        if !executable.is_file() {
            return Err(AsrError::Engine(format!(
                "sherpa-onnx-offline not found at {}",
                executable.display()
            )));
        }
        if !model_dir.is_dir() {
            return Err(AsrError::Engine(format!(
                "sherpa model directory not found at {}",
                model_dir.display()
            )));
        }
        Ok(Self {
            executable,
            model_dir,
            kind,
        })
    }

    /// Resolve the local sherpa-onnx runtime and model directory.
    ///
    /// Desktop and CLI use the same model files. Override with
    /// `CAT_SPEEDUP_SHERPA_EXE` / `CAT_SPEEDUP_SHERPA_MODEL`.
    pub fn from_env(kind: SherpaModelKind) -> Result<Self, AsrError> {
        let executable = std::env::var_os("CAT_SPEEDUP_SHERPA_EXE")
            .map(PathBuf::from)
            .unwrap_or_else(default_executable);
        let model_dir = std::env::var_os("CAT_SPEEDUP_SHERPA_MODEL")
            .map(PathBuf::from)
            .unwrap_or_else(|| default_model_dir(kind));
        Self::new(executable, model_dir, kind)
    }
}

pub fn default_executable() -> PathBuf {
    let bundled = PathBuf::from(
        "tools/sherpa-onnx/sherpa-onnx-v2.0.1-win-x64-shared-MD-Release-no-tts/bin/sherpa-onnx-offline.exe",
    );
    if bundled.is_file() {
        bundled
    } else {
        PathBuf::from("sherpa-onnx-offline")
    }
}

pub fn default_model_dir(kind: SherpaModelKind) -> PathBuf {
    match kind {
        SherpaModelKind::FunAsrNano => {
            PathBuf::from("models/sherpa-onnx-funasr-nano-int8-2025-12-30")
        }
        SherpaModelKind::Paraformer => {
            PathBuf::from("models/sherpa-onnx-paraformer-zh-int8-2025-10-07")
        }
    }
}

impl AsrEngine for SherpaEngine {
    fn name(&self) -> &str {
        "sherpa-onnx"
    }

    fn transcribe_wav(&mut self, path: &Path, options: &AsrOptions) -> Result<AsrResult, AsrError> {
        let duration_ms = wav_duration_ms(path).unwrap_or(0);
        let mut command = Command::new(&self.executable);
        command
            .arg(format!("--num-threads={}", options.threads.max(1)))
            .arg("--provider=cpu");

        match self.kind {
            SherpaModelKind::FunAsrNano => {
                command
                    .arg(format!(
                        "--funasr-nano-encoder-adaptor={}",
                        self.model_dir.join("encoder_adaptor.int8.onnx").display()
                    ))
                    .arg(format!(
                        "--funasr-nano-llm={}",
                        self.model_dir.join("llm.int8.onnx").display()
                    ))
                    .arg(format!(
                        "--funasr-nano-tokenizer={}",
                        self.model_dir.join("Qwen3-0.6B").display()
                    ))
                    .arg(format!(
                        "--funasr-nano-embedding={}",
                        self.model_dir.join("embedding.int8.onnx").display()
                    ));
            }
            SherpaModelKind::Paraformer => {
                command
                    .arg(format!(
                        "--paraformer={}",
                        self.model_dir.join("model.int8.onnx").display()
                    ))
                    .arg(format!(
                        "--tokens={}",
                        self.model_dir.join("tokens.txt").display()
                    ));
            }
        }

        command.arg(path);

        let output = command.output().map_err(|error| {
            AsrError::Engine(format!(
                "failed to run {}: {error}",
                self.executable.display()
            ))
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let tail: Vec<&str> = stderr.lines().rev().take(15).collect();
            return Err(AsrError::Engine(format!(
                "sherpa-onnx failed ({}):\n{}",
                output.status,
                tail.into_iter().rev().collect::<Vec<_>>().join("\n")
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json_line = stdout
            .lines()
            .rev()
            .find(|line| line.trim_start().starts_with('{'))
            .ok_or_else(|| {
                AsrError::Engine(format!(
                    "sherpa-onnx produced no JSON result; stdout tail:\n{}",
                    stdout
                        .lines()
                        .rev()
                        .take(10)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<Vec<_>>()
                        .join("\n")
                ))
            })?;

        let response: SherpaResponse = serde_json::from_str(json_line.trim())
            .map_err(|error| AsrError::Engine(format!("invalid sherpa JSON: {error}")))?;

        let transcript = segments_from_response(response, duration_ms);
        Ok(AsrResult {
            transcript,
            engine: "sherpa-onnx".into(),
            model: self.model_dir.display().to_string(),
            device: "cpu".into(),
        })
    }
}

#[derive(Debug, Deserialize)]
struct SherpaResponse {
    #[serde(default)]
    text: String,
    #[serde(default)]
    tokens: Vec<String>,
    #[serde(default)]
    timestamps: Vec<f32>,
}

fn segments_from_response(response: SherpaResponse, duration_ms: u64) -> Transcript {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut start_ms = None;
    let mut last_end_ms = 0u64;

    for (index, token) in response.tokens.iter().enumerate() {
        let token = token.trim();
        if token.is_empty() || token.starts_with("<0x") {
            continue;
        }

        let token_start = response
            .timestamps
            .get(index)
            .map(|value| (value.max(0.0) * 1000.0).round() as u64)
            .unwrap_or(last_end_ms);
        let token_end = response
            .timestamps
            .get(index + 1)
            .map(|value| (value.max(0.0) * 1000.0).round() as u64)
            .unwrap_or(duration_ms.max(token_start));

        if start_ms.is_none() {
            start_ms = Some(token_start);
        }

        if needs_space(current.chars().last(), token.chars().next()) {
            current.push(' ');
        }
        current.push_str(token);
        last_end_ms = token_end;

        let is_sentence_end = token
            .chars()
            .any(|ch| matches!(ch, '。' | '！' | '？' | '!' | '?' | ';' | '；'));
        let too_long = current.chars().count() >= 32;

        if is_sentence_end || too_long {
            if let Some(start) = start_ms.take() {
                segments.push(TranscriptSegment {
                    start_ms: start,
                    end_ms: token_end.max(start),
                    text: current.trim().to_string(),
                });
            }
            current.clear();
        }
    }

    if !current.trim().is_empty() {
        let start = start_ms.unwrap_or(0);
        segments.push(TranscriptSegment {
            start_ms: start,
            end_ms: duration_ms.max(last_end_ms).max(start),
            text: current.trim().to_string(),
        });
    }

    if segments.is_empty() && !response.text.trim().is_empty() {
        segments.push(TranscriptSegment {
            start_ms: 0,
            end_ms: duration_ms,
            text: response.text.trim().to_string(),
        });
    }

    split_long_segments(Transcript { segments }, 32)
}

fn needs_space(left: Option<char>, right: Option<char>) -> bool {
    left.is_some_and(|ch| ch.is_ascii_alphanumeric())
        && right.is_some_and(|ch| ch.is_ascii_alphanumeric())
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
    fn builds_segments_from_tokens() {
        let response = SherpaResponse {
            text: "你好。".into(),
            tokens: vec!["你".into(), "好".into(), "。".into()],
            timestamps: vec![0.0, 0.5, 1.0],
        };
        let transcript = segments_from_response(response, 2000);
        assert_eq!(transcript.segments.len(), 1);
        assert_eq!(transcript.segments[0].start_ms, 0);
        assert_eq!(transcript.segments[0].text, "你好。");
    }

    #[test]
    fn joins_english_tokens_with_spaces() {
        let response = SherpaResponse {
            text: "hello world".into(),
            tokens: vec!["hello".into(), "world".into()],
            timestamps: vec![0.0, 0.6],
        };
        let transcript = segments_from_response(response, 1500);
        assert_eq!(transcript.segments[0].text, "hello world");
    }
}
