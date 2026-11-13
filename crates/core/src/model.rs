use serde::{Deserialize, Serialize};

/// A contiguous piece of the source timeline. `start_ms..end_ms` always refers
/// to the original video, never to the edited output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speed: f64,
    pub kind: SegmentKind,
}

impl Segment {
    pub fn duration_ms(&self) -> u64 {
        self.end_ms.saturating_sub(self.start_ms)
    }

    pub fn output_duration_ms(&self) -> u64 {
        if self.speed <= 0.0 {
            0
        } else {
            (self.duration_ms() as f64 / self.speed).round() as u64
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentKind {
    Voice,
    NonVoice,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub source_duration_ms: u64,
    pub has_audio: bool,
    pub segments: Vec<Segment>,
}

impl Plan {
    pub fn total_output_ms(&self) -> u64 {
        self.segments.iter().map(Segment::output_duration_ms).sum()
    }

    /// The renderer relies on a gap-free, ordered plan. Fail loudly instead of
    /// silently dropping or duplicating a piece of the source timeline.
    pub fn validate(&self) -> Result<(), PlanError> {
        if self.segments.is_empty() {
            return Err(PlanError::Empty);
        }

        let mut expected = 0u64;
        for (index, segment) in self.segments.iter().enumerate() {
            if segment.end_ms <= segment.start_ms {
                return Err(PlanError::InvalidRange {
                    index,
                    start: segment.start_ms,
                    end: segment.end_ms,
                });
            }
            if !segment.speed.is_finite() || segment.speed <= 0.0 {
                return Err(PlanError::InvalidSpeed {
                    index,
                    speed: segment.speed,
                });
            }
            if segment.start_ms != expected {
                return Err(PlanError::NotContiguous {
                    index,
                    actual: segment.start_ms,
                    expected,
                });
            }
            expected = segment.end_ms;
        }

        if expected != self.source_duration_ms {
            return Err(PlanError::WrongEnd {
                actual: expected,
                expected: self.source_duration_ms,
            });
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    pub segments: Vec<TranscriptSegment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("plan has no segments")]
    Empty,
    #[error("segment {index} has invalid range {start}..{end}")]
    InvalidRange { index: usize, start: u64, end: u64 },
    #[error("segment {index} has invalid speed {speed}")]
    InvalidSpeed { index: usize, speed: f64 },
    #[error("segment {index} starts at {actual}ms, expected {expected}ms")]
    NotContiguous {
        index: usize,
        actual: u64,
        expected: u64,
    },
    #[error("plan ends at {actual}ms, expected {expected}ms")]
    WrongEnd { actual: u64, expected: u64 },
    #[error("invalid plan option {name}: {value}")]
    InvalidOption { name: &'static str, value: String },
}
