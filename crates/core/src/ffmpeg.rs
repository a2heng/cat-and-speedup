use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tempfile::TempDir;

use crate::asr::{AsrMap, AsrSpan};
use crate::detect::{SilenceSpan, parse_silencedetect};
use crate::model::{Plan, PlanError, Segment, SegmentKind};

#[derive(Debug, thiserror::Error)]
pub enum FfmpegError {
    #[error("ffmpeg not found: set CUT_SPEEDUP_FFMPEG or put ffmpeg/ffprobe on PATH")]
    NotFound,
    #[error("failed to run {program}: {source}")]
    Io {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{program} failed ({status})\n{stderr}")]
    Failed {
        program: String,
        status: String,
        stderr: String,
    },
    #[error("could not parse ffprobe output: {0}")]
    Probe(String),
    #[error("could not determine media duration")]
    MissingDuration,
    #[error("no voice segments to export for ASR")]
    NoVoice,
    #[error(transparent)]
    Plan(#[from] PlanError),
    #[error("io error: {0}")]
    IoPlain(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct Ffmpeg {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaInfo {
    pub duration_ms: u64,
    pub has_audio: bool,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub audio_sample_rate: Option<u32>,
    pub audio_channels: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct RenderOptions {
    pub video_codec: String,
    pub audio_codec: String,
    pub crf: u8,
    pub preset: String,
    pub audio_bitrate: String,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            video_codec: "libx264".into(),
            audio_codec: "aac".into(),
            crf: 20,
            preset: "veryfast".into(),
            audio_bitrate: "192k".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RenderReport {
    pub output: PathBuf,
    pub segments: usize,
    pub source_duration_ms: u64,
    pub output_duration_ms: u64,
}

#[derive(Debug, Clone)]
pub struct AsrOptions {
    pub sample_rate: u32,
    /// Merge voice spans separated by less than this gap to keep sentence context.
    pub merge_gap_ms: u64,
}

impl Default for AsrOptions {
    fn default() -> Self {
        Self {
            sample_rate: 16_000,
            merge_gap_ms: 500,
        }
    }
}

impl Ffmpeg {
    pub fn discover() -> Result<Self, FfmpegError> {
        if let Some(path) = env::var_os("CUT_SPEEDUP_FFMPEG").map(PathBuf::from) {
            return Self::from_ffmpeg(path);
        }
        if let Some(path) = env::var_os("FFMPEG_BIN").map(PathBuf::from) {
            return Self::from_ffmpeg(path);
        }

        // The bundled tools/ directory is a Windows build. Only prefer it on
        // Windows; on Linux/macOS use PATH so we never try to exec a PE file.
        if cfg!(windows) {
            for candidate in [
                PathBuf::from("tools/ffmpeg-9.0.2-full_build/bin/ffmpeg.exe"),
                PathBuf::from("tools/ffmpeg/bin/ffmpeg.exe"),
            ] {
                if candidate.is_file() {
                    return Self::from_ffmpeg(candidate);
                }
            }

            if let Ok(executable) = env::current_exe()
                && let Some(parent) = executable.parent()
            {
                let candidate = parent.join("ffmpeg.exe");
                if candidate.is_file() {
                    return Self::from_ffmpeg(candidate);
                }
            }
        }

        let ffmpeg = find_in_path("ffmpeg").ok_or(FfmpegError::NotFound)?;
        Self::with_ffmpeg(ffmpeg)
    }

    pub fn from_ffmpeg(path: impl Into<PathBuf>) -> Result<Self, FfmpegError> {
        let path = path.into();
        let ffmpeg = if path.is_file() {
            path
        } else {
            find_in_path(&path.to_string_lossy()).ok_or(FfmpegError::NotFound)?
        };
        Self::with_ffmpeg(ffmpeg)
    }

    fn with_ffmpeg(ffmpeg: PathBuf) -> Result<Self, FfmpegError> {
        let ffprobe_name = if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        };
        let sibling = ffmpeg.with_file_name(ffprobe_name);
        let ffprobe = if sibling.is_file() {
            sibling
        } else {
            find_in_path("ffprobe").ok_or(FfmpegError::NotFound)?
        };

        Ok(Self { ffmpeg, ffprobe })
    }

    pub fn version(&self) -> Result<String, FfmpegError> {
        let output = run(&self.ffmpeg, ["-version"])?;
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string())
    }

    pub fn probe(&self, input: &Path) -> Result<MediaInfo, FfmpegError> {
        let output = run(
            &self.ffprobe,
            [
                "-v".into(),
                "error".into(),
                "-show_entries".into(),
                "format=duration:stream=codec_type,width,height,avg_frame_rate,r_frame_rate,duration,sample_rate,channels".into(),
                "-of".into(),
                "json".into(),
                input.as_os_str().to_owned(),
            ],
        )?;

        let json: Value = serde_json::from_slice(&output.stdout)
            .map_err(|error| FfmpegError::Probe(error.to_string()))?;

        let duration_s = json["format"]["duration"]
            .as_str()
            .and_then(parse_positive_f64)
            .or_else(|| {
                json["streams"]
                    .as_array()?
                    .iter()
                    .find_map(|stream| stream["duration"].as_str().and_then(parse_positive_f64))
            })
            .ok_or(FfmpegError::MissingDuration)?;

        let streams = json["streams"].as_array().cloned().unwrap_or_default();
        let video = streams
            .iter()
            .find(|stream| stream["codec_type"].as_str() == Some("video"));
        let audio = streams
            .iter()
            .find(|stream| stream["codec_type"].as_str() == Some("audio"));

        Ok(MediaInfo {
            duration_ms: (duration_s * 1000.0).round() as u64,
            has_audio: audio.is_some(),
            width: video
                .and_then(|stream| stream["width"].as_u64())
                .map(|value| value as u32),
            height: video
                .and_then(|stream| stream["height"].as_u64())
                .map(|value| value as u32),
            fps: video.and_then(stream_fps),
            audio_sample_rate: audio
                .and_then(|stream| stream["sample_rate"].as_str())
                .and_then(|value| value.parse().ok()),
            audio_channels: audio
                .and_then(|stream| stream["channels"].as_u64())
                .map(|value| value as u32),
        })
    }

    pub fn detect_silence(
        &self,
        input: &Path,
        noise: &str,
        min_silence_s: f64,
        duration_ms: u64,
    ) -> Result<Vec<SilenceSpan>, FfmpegError> {
        let filter = format!("silencedetect=noise={noise}:d={min_silence_s}");
        let args: Vec<OsString> = vec![
            "-hide_banner".into(),
            "-nostdin".into(),
            "-nostats".into(),
            "-i".into(),
            input.as_os_str().to_owned(),
            "-vn".into(),
            "-map".into(),
            "0:a:0".into(),
            "-af".into(),
            filter.into(),
            "-f".into(),
            "null".into(),
            "-".into(),
        ];

        let output = run(&self.ffmpeg, args)?;
        Ok(parse_silencedetect(
            &String::from_utf8_lossy(&output.stderr),
            duration_ms,
        ))
    }

    pub fn render(
        &self,
        input: &Path,
        plan: &Plan,
        output: &Path,
        info: &MediaInfo,
        options: &RenderOptions,
    ) -> Result<RenderReport, FfmpegError> {
        plan.validate()?;
        create_parent(output)?;

        let temp = temp_dir_for(output)?;
        let mut parts = Vec::with_capacity(plan.segments.len());

        for (index, segment) in plan.segments.iter().enumerate() {
            let part = temp.path().join(format!("part_{index:05}.mp4"));
            self.render_segment(input, segment, &part, info, options)?;
            parts.push(part);
        }

        let list = temp.path().join("concat.txt");
        write_concat_list(&list, &parts)?;
        self.concat(&list, output)?;

        Ok(RenderReport {
            output: output.to_path_buf(),
            segments: plan.segments.len(),
            source_duration_ms: plan.source_duration_ms,
            output_duration_ms: plan.total_output_ms(),
        })
    }

    pub fn extract_frame(
        &self,
        input: &Path,
        at_ms: u64,
        output: &Path,
    ) -> Result<(), FfmpegError> {
        create_parent(output)?;
        let at_s = format!("{:.3}", at_ms as f64 / 1000.0);
        let args: Vec<OsString> = vec![
            "-y".into(),
            "-hide_banner".into(),
            "-nostdin".into(),
            "-loglevel".into(),
            "error".into(),
            "-ss".into(),
            at_s.into(),
            "-i".into(),
            input.as_os_str().to_owned(),
            "-frames:v".into(),
            "1".into(),
            "-q:v".into(),
            "2".into(),
            output.as_os_str().to_owned(),
        ];
        run(&self.ffmpeg, args).map(|_| ())
    }

    pub fn prepare_asr(
        &self,
        input: &Path,
        plan: &Plan,
        output: &Path,
        options: &AsrOptions,
    ) -> Result<AsrMap, FfmpegError> {
        if !plan.has_audio {
            return Err(FfmpegError::NoVoice);
        }

        let ranges = merge_voice_ranges(plan, options.merge_gap_ms);
        if ranges.is_empty() {
            return Err(FfmpegError::NoVoice);
        }

        create_parent(output)?;
        let temp = temp_dir_for(output)?;
        let mut parts = Vec::with_capacity(ranges.len());
        let mut spans = Vec::with_capacity(ranges.len());
        let mut edited_cursor = 0u64;

        for (index, (start_ms, end_ms)) in ranges.iter().copied().enumerate() {
            let part = temp.path().join(format!("asr_{index:05}.wav"));
            self.extract_wav(input, start_ms, end_ms, &part, options.sample_rate)?;

            let duration_ms = end_ms.saturating_sub(start_ms);
            spans.push(AsrSpan {
                edited_start_ms: edited_cursor,
                edited_end_ms: edited_cursor + duration_ms,
                original_start_ms: start_ms,
                original_end_ms: end_ms,
            });
            edited_cursor += duration_ms;
            parts.push(part);
        }

        let list = temp.path().join("asr-concat.txt");
        write_concat_list(&list, &parts)?;

        let args: Vec<OsString> = vec![
            "-y".into(),
            "-hide_banner".into(),
            "-nostdin".into(),
            "-loglevel".into(),
            "error".into(),
            "-f".into(),
            "concat".into(),
            "-safe".into(),
            "0".into(),
            "-i".into(),
            list.as_os_str().to_owned(),
            "-c:a".into(),
            "pcm_s16le".into(),
            "-ar".into(),
            options.sample_rate.to_string().into(),
            "-ac".into(),
            "1".into(),
            output.as_os_str().to_owned(),
        ];
        run(&self.ffmpeg, args)?;

        Ok(AsrMap {
            source_duration_ms: plan.source_duration_ms,
            sample_rate: options.sample_rate,
            spans,
        })
    }

    fn render_segment(
        &self,
        input: &Path,
        segment: &Segment,
        output: &Path,
        info: &MediaInfo,
        options: &RenderOptions,
    ) -> Result<(), FfmpegError> {
        let start_s = segment.start_ms as f64 / 1000.0;
        let duration_s = segment.duration_ms() as f64 / 1000.0;
        let speed = fmt_speed(segment.speed);

        let mut args: Vec<OsString> = vec![
            "-y".into(),
            "-hide_banner".into(),
            "-nostdin".into(),
            "-loglevel".into(),
            "error".into(),
            "-ss".into(),
            format!("{start_s:.3}").into(),
            "-t".into(),
            format!("{duration_s:.3}").into(),
            "-i".into(),
            input.as_os_str().to_owned(),
            "-t".into(),
            format!("{:.3}", segment.output_duration_ms() as f64 / 1000.0).into(),
            "-map".into(),
            "0:v:0".into(),
        ];

        if info.has_audio {
            args.extend(["-map".into(), "0:a:0".into()]);
        }

        args.extend([
            "-vf".into(),
            format!("setpts=(PTS-STARTPTS)/{speed}").into(),
        ]);

        if info.has_audio {
            args.extend([
                "-af".into(),
                format!("asetpts=PTS-STARTPTS,{}", atempo_chain(segment.speed)).into(),
            ]);
        }

        args.extend([
            "-c:v".into(),
            options.video_codec.clone().into(),
            "-preset".into(),
            options.preset.clone().into(),
            "-crf".into(),
            options.crf.to_string().into(),
            "-pix_fmt".into(),
            "yuv420p".into(),
        ]);

        if let Some(fps) = info.fps {
            args.extend(["-r".into(), fmt_speed(fps).into()]);
        }
        args.extend(["-fps_mode".into(), "cfr".into()]);

        if info.has_audio {
            args.extend([
                "-c:a".into(),
                options.audio_codec.clone().into(),
                "-b:a".into(),
                options.audio_bitrate.clone().into(),
                "-ar".into(),
                info.audio_sample_rate.unwrap_or(48_000).to_string().into(),
                "-ac".into(),
                info.audio_channels.unwrap_or(2).to_string().into(),
            ]);
        } else {
            args.push("-an".into());
        }

        args.extend([
            "-sn".into(),
            "-dn".into(),
            "-movflags".into(),
            "+faststart".into(),
            "-video_track_timescale".into(),
            "90000".into(),
            output.as_os_str().to_owned(),
        ]);

        run(&self.ffmpeg, args).map(|_| ())
    }

    fn concat(&self, list: &Path, output: &Path) -> Result<(), FfmpegError> {
        let args: Vec<OsString> = vec![
            "-y".into(),
            "-hide_banner".into(),
            "-nostdin".into(),
            "-loglevel".into(),
            "error".into(),
            "-f".into(),
            "concat".into(),
            "-safe".into(),
            "0".into(),
            "-i".into(),
            list.as_os_str().to_owned(),
            "-c".into(),
            "copy".into(),
            "-movflags".into(),
            "+faststart".into(),
            output.as_os_str().to_owned(),
        ];
        run(&self.ffmpeg, args).map(|_| ())
    }

    fn extract_wav(
        &self,
        input: &Path,
        start_ms: u64,
        end_ms: u64,
        output: &Path,
        sample_rate: u32,
    ) -> Result<(), FfmpegError> {
        let start_s = start_ms as f64 / 1000.0;
        let duration_s = end_ms.saturating_sub(start_ms) as f64 / 1000.0;
        let args: Vec<OsString> = vec![
            "-y".into(),
            "-hide_banner".into(),
            "-nostdin".into(),
            "-loglevel".into(),
            "error".into(),
            "-ss".into(),
            format!("{start_s:.3}").into(),
            "-t".into(),
            format!("{duration_s:.3}").into(),
            "-i".into(),
            input.as_os_str().to_owned(),
            "-vn".into(),
            "-map".into(),
            "0:a:0".into(),
            "-ac".into(),
            "1".into(),
            "-ar".into(),
            sample_rate.to_string().into(),
            "-c:a".into(),
            "pcm_s16le".into(),
            output.as_os_str().to_owned(),
        ];
        run(&self.ffmpeg, args).map(|_| ())
    }
}

fn merge_voice_ranges(plan: &Plan, merge_gap_ms: u64) -> Vec<(u64, u64)> {
    let mut ranges: Vec<(u64, u64)> = Vec::new();

    for segment in &plan.segments {
        if segment.kind != SegmentKind::Voice {
            continue;
        }

        match ranges.last_mut() {
            Some(last) if segment.start_ms <= last.1.saturating_add(merge_gap_ms) => {
                last.1 = last.1.max(segment.end_ms);
            }
            _ => ranges.push((segment.start_ms, segment.end_ms)),
        }
    }

    ranges
}

fn create_parent(output: &Path) -> Result<(), FfmpegError> {
    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    Ok(())
}

fn temp_dir_for(output: &Path) -> Result<TempDir, FfmpegError> {
    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));

    tempfile::Builder::new()
        .prefix(".cut-speedup-")
        .tempdir_in(parent)
        .or_else(|_| TempDir::new())
        .map_err(Into::into)
}

fn write_concat_list(path: &Path, parts: &[PathBuf]) -> Result<(), FfmpegError> {
    let mut text = String::new();
    for part in parts {
        let path = part.to_string_lossy().replace('\\', "/");
        let escaped = path.replace('\'', "'\\''");
        text.push_str(&format!("file '{escaped}'\n"));
    }
    fs::write(path, text)?;
    Ok(())
}

fn atempo_chain(mut speed: f64) -> String {
    let mut filters = Vec::new();
    while speed > 2.0 + 1e-9 {
        filters.push("atempo=2".to_string());
        speed /= 2.0;
    }
    while speed < 0.5 - 1e-9 {
        filters.push("atempo=0.5".to_string());
        speed /= 0.5;
    }
    filters.push(format!("atempo={}", fmt_speed(speed)));
    filters.join(",")
}

fn fmt_speed(value: f64) -> String {
    let mut text = format!("{value:.6}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

fn stream_fps(stream: &Value) -> Option<f64> {
    ["avg_frame_rate", "r_frame_rate"]
        .iter()
        .find_map(|key| stream[*key].as_str().and_then(parse_fps))
}

fn parse_fps(value: &str) -> Option<f64> {
    let (numerator, denominator) = value.split_once('/')?;
    let numerator: f64 = numerator.parse().ok()?;
    let denominator: f64 = denominator.parse().ok()?;
    (numerator > 0.0 && denominator > 0.0).then_some(numerator / denominator)
}

fn parse_positive_f64(value: &str) -> Option<f64> {
    value
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let mut names = vec![name.to_string()];
    if cfg!(windows) && !name.to_ascii_lowercase().ends_with(".exe") {
        names.insert(0, format!("{name}.exe"));
    }

    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths).find_map(|directory| {
            names
                .iter()
                .map(|name| directory.join(name))
                .find(|path| path.is_file())
        })
    })
}

fn run<I, S>(program: &Path, args: I) -> Result<Output, FfmpegError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|source| FfmpegError::Io {
            program: program.display().to_string(),
            source,
        })?;

    if !output.status.success() {
        return Err(FfmpegError::Failed {
            program: program.display().to_string(),
            status: output.status.to_string(),
            stderr: stderr_tail(&output.stderr),
        });
    }

    Ok(output)
}

fn stderr_tail(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().rev().take(20).collect();
    lines.into_iter().rev().collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::SilenceSpan;
    use crate::plan::{PlanOptions, build_plan};

    #[test]
    fn atempo_is_chained_above_two() {
        assert_eq!(atempo_chain(8.0), "atempo=2,atempo=2,atempo=2");
    }

    #[test]
    fn atempo_is_chained_below_half() {
        assert_eq!(atempo_chain(0.25), "atempo=0.5,atempo=0.5");
    }

    #[test]
    fn fmt_speed_has_no_trailing_zeros() {
        assert_eq!(fmt_speed(4.0), "4");
        assert_eq!(fmt_speed(2.5), "2.5");
        assert_eq!(fmt_speed(29.97002997), "29.97003");
    }

    #[test]
    fn merge_voice_ranges_respects_gap() {
        let plan = build_plan(
            10_000,
            true,
            &[SilenceSpan {
                start_ms: 4_000,
                end_ms: 5_000,
            }],
            &PlanOptions {
                margin_ms: 0,
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(
            merge_voice_ranges(&plan, 900),
            vec![(0, 4_000), (5_000, 10_000)]
        );
        assert_eq!(merge_voice_ranges(&plan, 1_000), vec![(0, 10_000)]);
    }
}
