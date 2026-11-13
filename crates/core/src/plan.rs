use crate::detect::SilenceSpan;
use crate::model::{Plan, PlanError, Segment, SegmentKind};

#[derive(Debug, Clone, PartialEq)]
pub struct PlanOptions {
    /// Playback speed for voice segments. Keep at 1.0 unless the user asks.
    pub voice_speed: f64,
    /// Playback speed for non-voice segments.
    pub silence_speed: f64,
    /// Extra voice padding on both sides of every active range, in ms.
    pub margin_ms: u64,
}

impl Default for PlanOptions {
    fn default() -> Self {
        Self {
            voice_speed: 1.0,
            silence_speed: 4.0,
            margin_ms: 150,
        }
    }
}

/// Build a gap-free plan that covers the whole source timeline.
///
/// Silence spans are the complement of voice. Voice is expanded by
/// `margin_ms` on both sides so cuts never clip the first/last syllable.
/// The resulting segments keep their original order; adjacent segments with
/// the same speed are merged only to keep the ffmpeg graph small.
pub fn build_plan(
    duration_ms: u64,
    has_audio: bool,
    silences: &[SilenceSpan],
    options: &PlanOptions,
) -> Result<Plan, PlanError> {
    validate_speed("voice_speed", options.voice_speed)?;
    validate_speed("silence_speed", options.silence_speed)?;

    if duration_ms == 0 {
        return Err(PlanError::Empty);
    }

    if !has_audio {
        let plan = Plan {
            source_duration_ms: duration_ms,
            has_audio,
            segments: vec![Segment {
                start_ms: 0,
                end_ms: duration_ms,
                speed: options.silence_speed,
                kind: SegmentKind::NonVoice,
            }],
        };
        plan.validate()?;
        return Ok(plan);
    }

    let silences = normalize_silences(silences, duration_ms);
    let mut voice_ranges = complement_voice_ranges(&silences, duration_ms);

    for range in &mut voice_ranges {
        range.start_ms = range.start_ms.saturating_sub(options.margin_ms);
        range.end_ms = range
            .end_ms
            .saturating_add(options.margin_ms)
            .min(duration_ms);
    }
    let voice_ranges = merge_ranges(voice_ranges);

    let mut segments = Vec::with_capacity(voice_ranges.len() * 2 + 1);
    let mut cursor = 0u64;

    for range in voice_ranges {
        if range.start_ms > cursor {
            push_segment(
                &mut segments,
                cursor,
                range.start_ms,
                options.silence_speed,
                SegmentKind::NonVoice,
            );
        }

        push_segment(
            &mut segments,
            range.start_ms,
            range.end_ms,
            options.voice_speed,
            SegmentKind::Voice,
        );
        cursor = range.end_ms;
    }

    if cursor < duration_ms {
        push_segment(
            &mut segments,
            cursor,
            duration_ms,
            options.silence_speed,
            SegmentKind::NonVoice,
        );
    }

    let plan = Plan {
        source_duration_ms: duration_ms,
        has_audio,
        segments,
    };
    plan.validate()?;
    Ok(plan)
}

fn validate_speed(name: &'static str, speed: f64) -> Result<(), PlanError> {
    if !speed.is_finite() || speed <= 0.0 {
        return Err(PlanError::InvalidOption {
            name,
            value: speed.to_string(),
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Range {
    start_ms: u64,
    end_ms: u64,
}

fn normalize_silences(silences: &[SilenceSpan], duration_ms: u64) -> Vec<Range> {
    let mut ranges: Vec<Range> = silences
        .iter()
        .filter_map(|span| {
            let start_ms = span.start_ms.min(duration_ms);
            let end_ms = span.end_ms.min(duration_ms);
            (end_ms > start_ms).then_some(Range { start_ms, end_ms })
        })
        .collect();
    ranges.sort_by_key(|range| range.start_ms);
    merge_ranges(ranges)
}

fn complement_voice_ranges(silences: &[Range], duration_ms: u64) -> Vec<Range> {
    let mut ranges = Vec::new();
    let mut cursor = 0u64;

    for silence in silences {
        if silence.start_ms > cursor {
            ranges.push(Range {
                start_ms: cursor,
                end_ms: silence.start_ms,
            });
        }
        cursor = cursor.max(silence.end_ms);
    }

    if cursor < duration_ms {
        ranges.push(Range {
            start_ms: cursor,
            end_ms: duration_ms,
        });
    }

    ranges
}

fn merge_ranges(mut ranges: Vec<Range>) -> Vec<Range> {
    ranges.sort_by_key(|range| range.start_ms);
    let mut merged: Vec<Range> = Vec::with_capacity(ranges.len());

    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start_ms <= last.end_ms => {
                last.end_ms = last.end_ms.max(range.end_ms);
            }
            _ => merged.push(range),
        }
    }

    merged
}

fn push_segment(
    segments: &mut Vec<Segment>,
    start_ms: u64,
    end_ms: u64,
    speed: f64,
    kind: SegmentKind,
) {
    if end_ms <= start_ms {
        return;
    }

    if let Some(last) = segments.last_mut()
        && last.kind == kind
        && (last.speed - speed).abs() < f64::EPSILON
    {
        last.end_ms = end_ms;
        return;
    }

    segments.push(Segment {
        start_ms,
        end_ms,
        speed,
        kind,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn silence(start_ms: u64, end_ms: u64) -> SilenceSpan {
        SilenceSpan { start_ms, end_ms }
    }

    #[test]
    fn no_silence_is_one_voice_segment() {
        let plan = build_plan(10_000, true, &[], &PlanOptions::default()).unwrap();
        assert_eq!(plan.segments.len(), 1);
        assert_eq!(plan.segments[0].kind, SegmentKind::Voice);
        assert_eq!(plan.segments[0].start_ms, 0);
        assert_eq!(plan.segments[0].end_ms, 10_000);
        assert_eq!(plan.total_output_ms(), 10_000);
    }

    #[test]
    fn middle_silence_becomes_ordered_three_segments() {
        let plan = build_plan(
            10_000,
            true,
            &[silence(4_000, 6_000)],
            &PlanOptions {
                margin_ms: 0,
                silence_speed: 4.0,
                ..Default::default()
            },
        )
        .unwrap();

        let kinds: Vec<_> = plan.segments.iter().map(|s| s.kind).collect();
        assert_eq!(
            kinds,
            vec![
                SegmentKind::Voice,
                SegmentKind::NonVoice,
                SegmentKind::Voice
            ]
        );
        assert_eq!(plan.segments[1].start_ms, 4_000);
        assert_eq!(plan.segments[1].end_ms, 6_000);
        assert_eq!(plan.segments[1].speed, 4.0);
        assert_eq!(plan.total_output_ms(), 4_000 + 500 + 4_000);
    }

    #[test]
    fn leading_and_trailing_silence_are_preserved() {
        let plan = build_plan(
            10_000,
            true,
            &[silence(0, 2_000), silence(8_000, 10_000)],
            &PlanOptions {
                margin_ms: 0,
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(plan.segments.len(), 3);
        assert_eq!(plan.segments[0].kind, SegmentKind::NonVoice);
        assert_eq!(plan.segments[1].kind, SegmentKind::Voice);
        assert_eq!(plan.segments[2].kind, SegmentKind::NonVoice);
        assert_eq!(plan.segments[1].start_ms, 2_000);
        assert_eq!(plan.segments[1].end_ms, 8_000);
    }

    #[test]
    fn margin_expands_voice_without_crossing_duration() {
        let plan = build_plan(
            10_000,
            true,
            &[silence(0, 2_000), silence(8_000, 10_000)],
            &PlanOptions {
                margin_ms: 500,
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(plan.segments[0].end_ms, 1_500);
        assert_eq!(plan.segments[1].start_ms, 1_500);
        assert_eq!(plan.segments[1].end_ms, 8_500);
        assert_eq!(plan.segments[2].start_ms, 8_500);
    }

    #[test]
    fn very_short_silence_is_absorbed_by_margin() {
        let plan = build_plan(
            10_000,
            true,
            &[silence(4_000, 4_200)],
            &PlanOptions {
                margin_ms: 150,
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(plan.segments.len(), 1);
        assert_eq!(plan.segments[0].kind, SegmentKind::Voice);
    }

    #[test]
    fn no_audio_speeds_the_whole_video() {
        let plan = build_plan(10_000, false, &[], &PlanOptions::default()).unwrap();
        assert_eq!(plan.segments.len(), 1);
        assert_eq!(plan.segments[0].kind, SegmentKind::NonVoice);
        assert_eq!(plan.segments[0].speed, 4.0);
        assert_eq!(plan.total_output_ms(), 2_500);
    }

    #[test]
    fn overlapping_input_silences_are_merged() {
        let plan = build_plan(
            10_000,
            true,
            &[silence(2_000, 5_000), silence(4_000, 6_000)],
            &PlanOptions {
                margin_ms: 0,
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(plan.segments.len(), 3);
        assert_eq!(plan.segments[1].start_ms, 2_000);
        assert_eq!(plan.segments[1].end_ms, 6_000);
    }

    #[test]
    fn plan_is_always_contiguous_and_covers_source() {
        let plan = build_plan(
            60_000,
            true,
            &[
                silence(0, 1_000),
                silence(3_000, 4_000),
                silence(20_000, 21_000),
                silence(59_000, 60_000),
            ],
            &PlanOptions::default(),
        )
        .unwrap();

        plan.validate().unwrap();
        assert_eq!(plan.segments.first().unwrap().start_ms, 0);
        assert_eq!(plan.segments.last().unwrap().end_ms, 60_000);
    }
}
