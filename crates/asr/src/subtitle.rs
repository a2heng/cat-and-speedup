use cat_speedup_core::{Transcript, TranscriptSegment};

/// Split ASR output into readable subtitle lines and distribute the original
/// segment duration by character count. Whisper often returns one long segment
/// for a whole clip; the UI needs shorter lines.
pub fn split_long_segments(transcript: Transcript, max_chars: usize) -> Transcript {
    let max_chars = max_chars.max(8);
    let mut output = Vec::new();

    for segment in transcript.segments {
        let pieces = split_text(&segment.text, max_chars);
        if pieces.is_empty() {
            continue;
        }

        let total_chars: usize = pieces.iter().map(|piece| piece.chars().count()).sum();
        let total_ms = segment.end_ms.saturating_sub(segment.start_ms);
        let mut cursor = segment.start_ms;

        for (index, piece) in pieces.iter().enumerate() {
            let chars = piece.chars().count();
            let is_last = index + 1 == pieces.len();
            let duration = if is_last {
                segment.end_ms.saturating_sub(cursor)
            } else if total_chars == 0 {
                0
            } else {
                ((total_ms as f64) * (chars as f64) / (total_chars as f64)).round() as u64
            };
            let end_ms = if is_last {
                segment.end_ms
            } else {
                (cursor + duration).min(segment.end_ms)
            };

            if end_ms > cursor {
                output.push(TranscriptSegment {
                    start_ms: cursor,
                    end_ms,
                    text: piece.clone(),
                });
            }
            cursor = end_ms;
        }
    }

    Transcript { segments: output }
}

fn split_text(text: &str, max_chars: usize) -> Vec<String> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }

    let mut sentences = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        if matches!(ch, '。' | '！' | '？' | '!' | '?' | ';' | '；') {
            sentences.push(current.trim().to_string());
            current.clear();
        }
    }
    if !current.trim().is_empty() {
        sentences.push(current.trim().to_string());
    }

    let mut pieces = Vec::new();
    let mut buffer = String::new();
    for sentence in sentences {
        for chunk in hard_split(&sentence, max_chars) {
            if buffer.chars().count() + chunk.chars().count() <= max_chars {
                if needs_space(&buffer, &chunk) {
                    buffer.push(' ');
                }
                buffer.push_str(&chunk);
            } else {
                if !buffer.is_empty() {
                    pieces.push(buffer.trim().to_string());
                }
                buffer = chunk;
            }
        }
    }
    if !buffer.trim().is_empty() {
        pieces.push(buffer.trim().to_string());
    }

    pieces
        .into_iter()
        .filter(|piece| !piece.is_empty())
        .collect()
}

fn needs_space(left: &str, right: &str) -> bool {
    left.chars()
        .last()
        .is_some_and(|ch| ch.is_ascii_alphanumeric())
        && right
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphanumeric())
}

fn hard_split(text: &str, max_chars: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < chars.len() {
        let limit = (start + max_chars).min(chars.len());
        let mut end = limit;

        if end < chars.len()
            && let Some(relative) = chars[start..limit]
                .iter()
                .rposition(|ch| matches!(ch, '，' | ',' | '、' | ' '))
            && relative >= max_chars / 3
        {
            end = start + relative + 1;
        }

        let chunk: String = chars[start..end].iter().collect();
        if !chunk.trim().is_empty() {
            chunks.push(chunk.trim().to_string());
        }
        start = end;
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_at_sentence_punctuation() {
        let transcript = Transcript {
            segments: vec![TranscriptSegment {
                start_ms: 0,
                end_ms: 10_000,
                text: "第一句话很长。第二句话很长！第三句话很长？".into(),
            }],
        };
        let split = split_long_segments(transcript, 8);
        assert_eq!(split.segments.len(), 3);
        assert_eq!(split.segments[2].end_ms, 10_000);
    }

    #[test]
    fn hard_splits_long_sentence() {
        let text = "这是一个没有任何标点但是非常非常长的句子".repeat(3);
        let transcript = Transcript {
            segments: vec![TranscriptSegment {
                start_ms: 0,
                end_ms: 9_000,
                text,
            }],
        };
        let split = split_long_segments(transcript, 16);
        assert!(split.segments.len() > 1);
        assert_eq!(split.segments.last().unwrap().end_ms, 9_000);
        for segment in &split.segments {
            assert!(segment.text.chars().count() <= 20);
        }
    }

    #[test]
    fn keeps_short_text_intact() {
        let transcript = Transcript {
            segments: vec![TranscriptSegment {
                start_ms: 100,
                end_ms: 900,
                text: "短句".into(),
            }],
        };
        let split = split_long_segments(transcript, 32);
        assert_eq!(split.segments.len(), 1);
        assert_eq!(split.segments[0].start_ms, 100);
        assert_eq!(split.segments[0].end_ms, 900);
    }
}
