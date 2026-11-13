use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
#[cfg(feature = "whisper")]
use cut_speedup_asr::resolve_model;
#[cfg(feature = "whisper")]
use cut_speedup_asr::whisper::WhisperEngine;
use cut_speedup_asr::{
    AsrEngine, AsrOptions, DevicePolicy, HttpAsrEngine, SherpaEngine, SherpaModelKind,
    sherpa::{default_executable, default_model_dir},
};
use cut_speedup_core::{
    AsrMap, Ffmpeg, MediaInfo, Plan, PlanOptions, RenderOptions, Transcript, build_plan,
    map_transcript,
};
use clap::{Args, Parser, Subcommand};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use tempfile::TempDir;

#[derive(Debug, Parser)]
#[command(
    name = "cut-speedup",
    version,
    about = "Long-video silence accelerator"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Check ffmpeg/ffprobe discovery.
    Doctor,
    /// Probe the input and print an ordered segment plan.
    Analyze(AnalyzeArgs),
    /// Render the accelerated video.
    Render(RenderArgs),
    /// Extract voice-only audio plus an edited->original time map for ASR.
    PrepareAsr(PrepareAsrArgs),
    /// Transcribe voice-only audio and map subtitles back to the original timeline.
    Transcribe(Box<TranscribeArgs>),
    /// Map an ASR transcript from compact-audio time back to the original timeline.
    MapTranscript(MapTranscriptArgs),
}

#[derive(Debug, Clone, Args)]
struct DetectionArgs {
    /// Silence threshold passed to ffmpeg, e.g. -30dB.
    #[arg(long, default_value = "-30dB")]
    noise: String,
    /// Minimum silence duration in seconds.
    #[arg(long, default_value_t = 0.5)]
    min_silence: f64,
    /// Playback speed for non-voice segments.
    #[arg(long, default_value_t = 4.0)]
    speed: f64,
    /// Voice padding on both sides in seconds.
    #[arg(long, default_value_t = 0.15)]
    margin: f64,
}

impl DetectionArgs {
    fn plan_options(&self) -> Result<PlanOptions> {
        anyhow::ensure!(self.min_silence > 0.0, "min-silence must be > 0");
        anyhow::ensure!(self.speed > 0.0, "speed must be > 0");
        anyhow::ensure!(self.margin >= 0.0, "margin must be >= 0");
        Ok(PlanOptions {
            voice_speed: 1.0,
            silence_speed: self.speed,
            margin_ms: (self.margin * 1000.0).round() as u64,
        })
    }
}

#[derive(Debug, Clone, Args)]
struct SherpaArgs {
    /// Path to sherpa-onnx-offline (or CUT_SPEEDUP_SHERPA_EXE).
    #[arg(long)]
    sherpa_exe: Option<PathBuf>,
    /// Directory containing encoder_adaptor.int8.onnx / llm.int8.onnx / tokenizer.
    #[arg(long)]
    sherpa_model_dir: Option<PathBuf>,
    /// funasr-nano or paraformer.
    #[arg(long, default_value = "funasr-nano")]
    sherpa_kind: String,
}

#[derive(Debug, Args)]
struct AnalyzeArgs {
    input: PathBuf,
    #[command(flatten)]
    detection: DetectionArgs,
    /// Write the plan JSON to a file instead of stdout.
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct RenderArgs {
    input: PathBuf,
    output: PathBuf,
    #[command(flatten)]
    detection: DetectionArgs,
    /// Reuse an existing plan JSON instead of detecting silence again.
    #[arg(long)]
    plan: Option<PathBuf>,
    #[arg(long, default_value_t = 20)]
    crf: u8,
    #[arg(long, default_value = "veryfast")]
    preset: String,
    #[arg(long, default_value = "libx264")]
    video_codec: String,
    #[arg(long, default_value = "aac")]
    audio_codec: String,
    #[arg(long, default_value = "192k")]
    audio_bitrate: String,
}

#[derive(Debug, Args)]
struct PrepareAsrArgs {
    input: PathBuf,
    output: PathBuf,
    #[command(flatten)]
    detection: DetectionArgs,
    /// Where to write the time map. Defaults to `<output>.map.json`.
    #[arg(long)]
    map: Option<PathBuf>,
    #[arg(long, default_value_t = 16_000)]
    sample_rate: u32,
    /// Merge voice spans separated by less than this many seconds.
    #[arg(long, default_value_t = 0.5)]
    merge_gap: f64,
}

#[derive(Debug, Args)]
struct TranscribeArgs {
    input: Option<PathBuf>,
    #[command(flatten)]
    detection: DetectionArgs,
    #[command(flatten)]
    sherpa: SherpaArgs,
    /// ASR backend: sherpa (offline FunASR), whisper, funasr (HTTP preset), http.
    #[arg(long, default_value = "sherpa")]
    engine: String,
    /// OpenAI-compatible transcription URL (for --engine http).
    #[arg(long)]
    asr_url: Option<String>,
    /// Model name sent to the HTTP ASR service.
    #[arg(long)]
    asr_model: Option<String>,
    /// Bearer token for the HTTP ASR service (or CUT_SPEEDUP_ASR_API_KEY).
    #[arg(long)]
    asr_api_key: Option<String>,
    #[arg(long, default_value_t = 600)]
    asr_timeout: u64,
    /// whisper.cpp model size (auto/tiny/base/small/medium/large-v3) or a .bin path.
    /// auto picks small on a GPU build and base on CPU.
    #[arg(long, default_value = "auto")]
    model: String,
    /// auto tries GPU then CPU; cpu/gpu force one.
    #[arg(long, default_value = "auto")]
    device: String,
    /// zh/en/ja/... or auto.
    #[arg(long, default_value = "auto")]
    language: String,
    /// Optional decoder prompt (defaults to a simplified-Chinese hint for zh).
    #[arg(long)]
    prompt: Option<String>,
    #[arg(long)]
    translate: bool,
    #[arg(long)]
    threads: Option<usize>,
    /// Do not download a missing model.
    #[arg(long)]
    no_download: bool,
    /// Save the compact ASR WAV here (and its map next to it).
    #[arg(long)]
    keep_audio: Option<PathBuf>,
    /// Use an existing compact ASR WAV (requires --map).
    #[arg(long)]
    audio: Option<PathBuf>,
    /// Use an existing edited->original map (requires --audio).
    #[arg(long)]
    map: Option<PathBuf>,
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct MapTranscriptArgs {
    transcript: PathBuf,
    map: PathBuf,
    #[arg(short, long)]
    output: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Doctor => doctor(),
        Command::Analyze(args) => analyze(args),
        Command::Render(args) => render(args),
        Command::PrepareAsr(args) => prepare_asr(args),
        Command::Transcribe(args) => transcribe(*args),
        Command::MapTranscript(args) => map_transcript_cmd(args),
    }
}

fn doctor() -> Result<()> {
    let ffmpeg = Ffmpeg::discover()?;
    println!("ffmpeg:  {}", ffmpeg.ffmpeg.display());
    println!("ffprobe: {}", ffmpeg.ffprobe.display());
    println!("version: {}", ffmpeg.version()?);
    #[cfg(feature = "whisper")]
    println!("whisper: {}", cut_speedup_asr::whisper_cpp_version());
    println!(
        "gpu:     {}",
        cut_speedup_asr::gpu_backend().unwrap_or("none (CPU-only build)")
    );
    println!("sherpa:  {}", default_executable().display());
    println!(
        "funasr:  {}",
        default_model_dir(SherpaModelKind::FunAsrNano).display()
    );

    Ok(())
}

fn analyze(args: AnalyzeArgs) -> Result<()> {
    let (_, info, plan) = analyze_input(&args.input, &args.detection)?;
    let json = serde_json::to_string_pretty(&plan)?;

    match args.output {
        Some(path) => {
            write_json(&path, &plan)?;
            eprintln!(
                "wrote {} ({} segments, {:.2}s -> {:.2}s)",
                path.display(),
                plan.segments.len(),
                plan.source_duration_ms as f64 / 1000.0,
                plan.total_output_ms() as f64 / 1000.0
            );
        }
        None => println!("{json}"),
    }

    eprintln!(
        "source: {}ms, audio: {}, output: {}ms",
        info.duration_ms,
        info.has_audio,
        plan.total_output_ms()
    );
    Ok(())
}

fn render(args: RenderArgs) -> Result<()> {
    let ffmpeg = Ffmpeg::discover()?;
    let info = ffmpeg.probe(&args.input)?;

    let plan = match args.plan {
        Some(path) => read_plan(&path)?,
        None => build_plan_for(&ffmpeg, &args.input, &info, &args.detection)?,
    };

    let options = RenderOptions {
        video_codec: args.video_codec,
        audio_codec: args.audio_codec,
        crf: args.crf,
        preset: args.preset,
        audio_bitrate: args.audio_bitrate,
    };

    let report = ffmpeg.render(&args.input, &plan, &args.output, &info, &options)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn prepare_asr(args: PrepareAsrArgs) -> Result<()> {
    anyhow::ensure!(args.sample_rate > 0, "sample-rate must be > 0");
    anyhow::ensure!(args.merge_gap >= 0.0, "merge-gap must be >= 0");

    let ffmpeg = Ffmpeg::discover()?;
    let info = ffmpeg.probe(&args.input)?;
    let plan = build_plan_for(&ffmpeg, &args.input, &info, &args.detection)?;
    let options = cut_speedup_core::ffmpeg::AsrOptions {
        sample_rate: args.sample_rate,
        merge_gap_ms: (args.merge_gap * 1000.0).round() as u64,
    };

    let map = ffmpeg.prepare_asr(&args.input, &plan, &args.output, &options)?;
    let map_path = args.map.unwrap_or_else(|| default_map_path(&args.output));
    write_json(&map_path, &map)?;

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "audio": args.output,
            "map": map_path,
            "spans": map.spans.len(),
            "edited_duration_ms": map.edited_duration_ms(),
        }))?
    );
    Ok(())
}

fn transcribe(args: TranscribeArgs) -> Result<()> {
    anyhow::ensure!(
        args.audio.is_some() == args.map.is_some(),
        "--audio and --map must be provided together"
    );

    let device: DevicePolicy = args
        .device
        .parse()
        .map_err(|error: String| anyhow::anyhow!(error))?;

    let mut temp_dir = None;
    let (audio_path, map) = if let (Some(audio), Some(map_path)) = (&args.audio, &args.map) {
        let map: AsrMap = read_json(map_path)?;
        (audio.clone(), map)
    } else {
        let input = args
            .input
            .as_ref()
            .context("input video is required unless --audio/--map are used")?;
        let ffmpeg = Ffmpeg::discover()?;
        let info = ffmpeg.probe(input)?;
        let plan = build_plan_for(&ffmpeg, input, &info, &args.detection)?;
        let asr_options = cut_speedup_core::ffmpeg::AsrOptions {
            sample_rate: 16_000,
            merge_gap_ms: 500,
        };

        let dir = TempDir::new()?;
        let audio = args
            .keep_audio
            .clone()
            .unwrap_or_else(|| dir.path().join("asr.wav"));
        let map = ffmpeg.prepare_asr(input, &plan, &audio, &asr_options)?;

        let map_path = if args.keep_audio.is_some() {
            let path = audio.with_extension("map.json");
            write_json(&path, &map)?;
            path
        } else {
            let path = dir.path().join("asr-map.json");
            write_json(&path, &map)?;
            path
        };
        eprintln!(
            "ASR audio: {} (map: {})",
            audio.display(),
            map_path.display()
        );

        temp_dir = Some(dir);
        (audio, map)
    };

    #[cfg(feature = "whisper")]
    let spec = {
        let model = if args.model.eq_ignore_ascii_case("auto") {
            if cut_speedup_asr::gpu_backend().is_some() {
                "small"
            } else {
                "base"
            }
        } else {
            args.model.as_str()
        };
        resolve_model(Some(model))
    };
    let default_asr_url = "http://127.0.0.1:8000/v1/audio/transcriptions";
    let mut engine: Box<dyn AsrEngine> = match args.engine.as_str() {
        "sherpa" | "funasr-offline" => {
            let kind = SherpaModelKind::parse(&args.sherpa.sherpa_kind)?;
            match (&args.sherpa.sherpa_exe, &args.sherpa.sherpa_model_dir) {
                (Some(exe), Some(dir)) => Box::new(SherpaEngine::new(exe, dir, kind)?),
                (Some(exe), None) => {
                    Box::new(SherpaEngine::new(exe, default_model_dir(kind), kind)?)
                }
                (None, Some(dir)) => Box::new(SherpaEngine::new(default_executable(), dir, kind)?),
                (None, None) => Box::new(SherpaEngine::from_env(kind)?),
            }
        }
        #[cfg(feature = "whisper")]
        "whisper" => Box::new(WhisperEngine::new(&spec, !args.no_download)?),
        "whisper" => anyhow::bail!(
            "this build has no whisper support; use --engine sherpa or rebuild with --features whisper"
        ),
        "funasr" | "http" => {
            let url = args
                .asr_url
                .clone()
                .or_else(|| std::env::var("CUT_SPEEDUP_ASR_URL").ok())
                .unwrap_or_else(|| default_asr_url.to_string());
            let api_key = args
                .asr_api_key
                .clone()
                .or_else(|| std::env::var("CUT_SPEEDUP_ASR_API_KEY").ok());
            let model = args
                .asr_model
                .clone()
                .or_else(|| std::env::var("CUT_SPEEDUP_ASR_MODEL").ok())
                .unwrap_or_else(|| {
                    if args.engine == "funasr" {
                        "FunAudioLLM/Fun-ASR-Nano-2512".to_string()
                    } else {
                        "whisper-1".to_string()
                    }
                });
            eprintln!("ASR service: {url} ({model})");
            Box::new(HttpAsrEngine::new(url, api_key, model, args.asr_timeout))
        }
        other => anyhow::bail!(
            "unknown ASR engine '{other}' (currently supported: whisper, funasr, http)"
        ),
    };
    let language = if args.language.eq_ignore_ascii_case("auto") {
        if args.engine == "funasr" {
            Some("zh".to_string())
        } else {
            None
        }
    } else {
        Some(args.language.clone())
    };
    let defaults = AsrOptions::default();
    let initial_prompt = args.prompt.clone().or_else(|| {
        (args.language.eq_ignore_ascii_case("zh") || args.engine == "funasr")
            .then(|| "以下是普通话的句子，请使用简体中文。".to_string())
    });
    let options = AsrOptions {
        language,
        translate: args.translate,
        initial_prompt,
        threads: args.threads.unwrap_or(defaults.threads),
        device,
    };

    let result = engine.transcribe_wav(&audio_path, &options)?;
    let transcript = map_transcript(&result.transcript, &map);

    match args.output {
        Some(path) => {
            write_json(&path, &transcript)?;
            eprintln!(
                "wrote {} ({} subtitle segments)",
                path.display(),
                transcript.segments.len()
            );
        }
        None => println!("{}", serde_json::to_string_pretty(&transcript)?),
    }

    eprintln!(
        "engine: {}, model: {}, device: {}, segments: {}",
        result.engine,
        result.model,
        result.device,
        transcript.segments.len()
    );

    drop(temp_dir);
    Ok(())
}

fn map_transcript_cmd(args: MapTranscriptArgs) -> Result<()> {
    let transcript: Transcript = read_json(&args.transcript)?;
    let map: AsrMap = read_json(&args.map)?;
    let mapped = map_transcript(&transcript, &map);

    match args.output {
        Some(path) => {
            write_json(&path, &mapped)?;
            eprintln!(
                "wrote {} ({} subtitle segments)",
                path.display(),
                mapped.segments.len()
            );
        }
        None => println!("{}", serde_json::to_string_pretty(&mapped)?),
    }
    Ok(())
}

fn analyze_input(input: &Path, detection: &DetectionArgs) -> Result<(Ffmpeg, MediaInfo, Plan)> {
    let ffmpeg = Ffmpeg::discover()?;
    let info = ffmpeg.probe(input)?;
    let plan = build_plan_for(&ffmpeg, input, &info, detection)?;
    Ok((ffmpeg, info, plan))
}

fn build_plan_for(
    ffmpeg: &Ffmpeg,
    input: &Path,
    info: &MediaInfo,
    detection: &DetectionArgs,
) -> Result<Plan> {
    let silences = if info.has_audio {
        ffmpeg.detect_silence(
            input,
            &detection.noise,
            detection.min_silence,
            info.duration_ms,
        )?
    } else {
        Vec::new()
    };

    build_plan(
        info.duration_ms,
        info.has_audio,
        &silences,
        &detection.plan_options()?,
    )
    .map_err(Into::into)
}

fn read_plan(path: &Path) -> Result<Plan> {
    let plan: Plan = read_json(path)?;
    plan.validate()?;
    Ok(plan)
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let text = serde_json::to_vec_pretty(value)?;
    fs::write(path, text).with_context(|| format!("failed to write {}", path.display()))
}

fn default_map_path(output: &Path) -> PathBuf {
    output.with_extension("map.json")
}
