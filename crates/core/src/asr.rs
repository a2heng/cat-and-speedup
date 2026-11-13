use serde::{Deserialize, Serialize};

use crate::model::{Transcript, TranscriptSegment};

/// One concatenated voice span in the compact ASR audio.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AsrSpan {
    pub edited_start_ms: u64,
    pub edited_end_ms: u64,
    pub original_start_ms: u64,
    pub original_end_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AsrMap {
    pub source_duration_ms: u64,
    pub sample_rate: u32,
    pub spans: Vec<AsrSpan>,
}

impl AsrMap {
    pub fn edited_duration_ms(&self) -> u64 {
        self.spans
            .last()
            .map(|span| span.edited_end_ms)
            .unwrap_or(0)
    }

    /// Map a transcript produced from the compact ASR audio back onto the
    /// original video timeline. Subtitles that cross a cut are expanded to
    /// cover the removed gap; that is preferable to dropping text.
    pub fn map_transcript(&self, transcript: &Transcript) -> Transcript {
        let mut segments = Vec::new();

        for segment in &transcript.segments {
            let overlapping: Vec<&AsrSpan> = self
                .spans
                .iter()
                .filter(|span| {
                    segment.start_ms < span.edited_end_ms && segment.end_ms > span.edited_start_ms
                })
                .collect();

            let (Some(first), Some(last)) = (overlapping.first(), overlapping.last()) else {
                continue;
            };

            let start_offset = segment.start_ms.saturating_sub(first.edited_start_ms);
            let end_offset = segment.end_ms.saturating_sub(last.edited_start_ms);

            let start_ms = (first.original_start_ms + start_offset).min(first.original_end_ms);
            let end_ms = (last.original_start_ms + end_offset).min(last.original_end_ms);

            if end_ms > start_ms && !segment.text.trim().is_empty() {
                segments.push(TranscriptSegment {
                    start_ms,
                    end_ms,
                    text: segment.text.clone(),
                });
            }
        }

        segments.sort_by_key(|segment| (segment.start_ms, segment.end_ms));
        Transcript { segments }
    }
}

pub fn map_transcript(transcript: &Transcript, map: &AsrMap) -> Transcript {
    map.map_transcript(transcript)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> AsrMap {
        AsrMap {
            source_duration_ms: 10_000,
            sample_rate: 16_000,
            spans: vec![
                AsrSpan {
                    edited_start_ms: 0,
                    edited_end_ms: 1_000,
                    original_start_ms: 5_000,
                    original_end_ms: 6_000,
                },
                AsrSpan {
                    edited_start_ms: 1_000,
                    edited_end_ms: 2_000,
                    original_start_ms: 8_000,
                    original_end_ms: 9_000,
                },
            ],
        }
    }

    #[test]
    fn maps_within_one_span() {
        let transcript = Transcript {
            segments: vec![TranscriptSegment {
                start_ms: 200,
                end_ms: 700,
                text: "hello".into(),
            }],
        };
        let mapped = map().map_transcript(&transcript);
        assert_eq!(mapped.segments[0].start_ms, 5_200);
        assert_eq!(mapped.segments[0].end_ms, 5_700);
    }

    #[test]
    fn maps_across_a_cut_to_cover_the_gap() {
        let transcript = Transcript {
            segments: vec![TranscriptSegment {
                start_ms: 500,
                end_ms: 1_500,
                text: "across".into(),
            }],
        };
        let mapped = map().map_transcript(&transcript);
        assert_eq!(mapped.segments[0].start_ms, 5_500);
        assert_eq!(mapped.segments[0].end_ms, 8_500);
    }

    #[test]
    fn drops_segments_without_overlap_or_text() {
        let transcript = Transcript {
            segments: vec![
                TranscriptSegment {
                    start_ms: 5_000,
                    end_ms: 6_000,
                    text: "outside".into(),
                },
                TranscriptSegment {
                    start_ms: 0,
                    end_ms: 500,
                    text: "   ".into(),
                },
            ],
        };
        assert!(map().map_transcript(&transcript).segments.is_empty());
    }

    #[test]
    fn clamps_to_span_bounds() {
        let transcript = Transcript {
            segments: vec![TranscriptSegment {
                start_ms: 0,
                end_ms: 10_000,
                text: "long".into(),
            }],
        };
        let mapped = map().map_transcript(&transcript);
        assert_eq!(mapped.segments[0].start_ms, 5_000);
        assert_eq!(mapped.segments[0].end_ms, 9_000);
    }
}
