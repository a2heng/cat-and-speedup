use serde::{Deserialize, Serialize};

/// A silence span on the original timeline, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SilenceSpan {
    pub start_ms: u64,
    pub end_ms: u64,
}

impl SilenceSpan {
    pub fn duration_ms(self) -> u64 {
        self.end_ms.saturating_sub(self.start_ms)
    }
}

/// Parse the stderr of `ffmpeg -af silencedetect=... -f null -`.
///
/// FFmpeg emits two lines per span:
/// `silence_start: 1.234` and
/// `silence_end: 2.345 | silence_duration: 1.111`.
/// A trailing `silence_start` without an end is closed at the media duration.
pub fn parse_silencedetect(stderr: &str, duration_ms: u64) -> Vec<SilenceSpan> {
    let mut spans = Vec::new();
    let mut open_start = None;

    for line in stderr.lines() {
        if let Some(rest) = line.split("silence_start:").nth(1) {
            if let Some(seconds) = parse_seconds(rest) {
                open_start = Some(seconds_to_ms(seconds));
            }
            continue;
        }

        if let Some(rest) = line.split("silence_end:").nth(1) {
            let end_text = rest.split('|').next().unwrap_or(rest);
            if let (Some(start_ms), Some(end_seconds)) =
                (open_start.take(), parse_seconds(end_text))
            {
                push_span(
                    &mut spans,
                    start_ms,
                    seconds_to_ms(end_seconds),
                    duration_ms,
                );
            }
        }
    }

    if let Some(start_ms) = open_start {
        push_span(&mut spans, start_ms, duration_ms, duration_ms);
    }

    spans
}

fn parse_seconds(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

fn seconds_to_ms(seconds: f64) -> u64 {
    (seconds.max(0.0) * 1000.0).round() as u64
}

fn push_span(spans: &mut Vec<SilenceSpan>, start_ms: u64, end_ms: u64, duration_ms: u64) {
    let start_ms = start_ms.min(duration_ms);
    let end_ms = end_ms.min(duration_ms);
    if end_ms > start_ms {
        spans.push(SilenceSpan { start_ms, end_ms });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_two_closed_spans() {
        let stderr = r#"
[silencedetect @ 000001] silence_start: 1.234
[silencedetect @ 000001] silence_end: 2.345 | silence_duration: 1.111
[silencedetect @ 000001] silence_start: 8
[silencedetect @ 000001] silence_end: 9.5 | silence_duration: 1.5
"#;
        assert_eq!(
            parse_silencedetect(stderr, 10_000),
            vec![
                SilenceSpan {
                    start_ms: 1234,
                    end_ms: 2345
                },
                SilenceSpan {
                    start_ms: 8000,
                    end_ms: 9500
                },
            ]
        );
    }

    #[test]
    fn closes_trailing_span_at_duration() {
        let stderr = "[silencedetect @ 0] silence_start: 7.25\n";
        assert_eq!(
            parse_silencedetect(stderr, 10_000),
            vec![SilenceSpan {
                start_ms: 7250,
                end_ms: 10_000
            }]
        );
    }

    #[test]
    fn ignores_malformed_or_empty_spans() {
        let stderr = r#"
[silencedetect @ 0] silence_start: nope
[silencedetect @ 0] silence_end: 2.0 | silence_duration: 2.0
[silencedetect @ 0] silence_start: 3.0
[silencedetect @ 0] silence_end: 3.0 | silence_duration: 0
"#;
        assert!(parse_silencedetect(stderr, 10_000).is_empty());
    }

    #[test]
    fn clamps_to_duration() {
        let stderr = r#"
[silencedetect @ 0] silence_start: 9.0
[silencedetect @ 0] silence_end: 12.0 | silence_duration: 3.0
"#;
        assert_eq!(
            parse_silencedetect(stderr, 10_000),
            vec![SilenceSpan {
                start_ms: 9000,
                end_ms: 10_000
            }]
        );
    }
}
