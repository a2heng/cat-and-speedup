//! Web front-end for cut-speedup.
//!
//! Reuses the same planning model as the desktop app and the Android client:
//! a contiguous, ordered list of segments where speech stays at 1x and the
//! gaps between sentences are sped up. Two analysis back-ends are exposed:
//!
//! * `silence` — FFmpeg `silencedetect` loudness thresholds, fast and cheap.
//! * `vad` — Silero VAD + X-ASR (sherpa-onnx), the same pipeline the phone
//!   uses; also produces subtitles for quick review.
//!
//! Run with `cargo run -p cut-speedup-web -- --port 42006`.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use cut_speedup_core::{
    build_plan, Ffmpeg, PlanOptions, Segment, SegmentKind,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

const INDEX_HTML: &str = include_str!("../assets/index.html");
const APP_JS: &str = include_str!("../assets/app.js");
const STYLE_CSS: &str = include_str!("../assets/style.css");

const DEFAULT_PORT: u16 = 42_006;

// ---------------------------------------------------------------------------
// State

#[derive(Clone)]
struct AppState {
    root: PathBuf,
    roots: Vec<PathBuf>,
    out_dir: PathBuf,
    tmp_dir: PathBuf,
    ffmpeg: Arc<Ffmpeg>,
    jobs: Arc<Mutex<HashMap<String, JobView>>>,
    job_counter: Arc<AtomicU64>,
}

#[derive(Debug, Clone, Serialize)]
struct JobView {
    id: String,
    kind: String,
    status: String,
    progress: Option<f32>,
    message: String,
    result: Option<Value>,
    error: Option<String>,
    started_ms: u128,
    updated_ms: u128,
}

impl AppState {
    fn new() -> anyhow::Result<Self> {
        let root = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let root = root.canonicalize().unwrap_or(root);

        let mut roots = vec![root.clone()];
        for candidate in [
            root.join("Videos"),
            root.join("下载"),
            root.join("c/Users/a2heng/Videos"),
            root.join("d"),
        ] {
            if candidate.is_dir() {
                roots.push(candidate);
            }
        }

        let out_dir = workspace_root().join("out/web");
        let tmp_dir = out_dir.join("tmp");
        std::fs::create_dir_all(&out_dir)?;
        std::fs::create_dir_all(&tmp_dir)?;

        let ffmpeg = Ffmpeg::discover()?;
        Ok(Self {
            root,
            roots,
            out_dir,
            tmp_dir,
            ffmpeg: Arc::new(ffmpeg),
            jobs: Arc::new(Mutex::new(HashMap::new())),
            job_counter: Arc::new(AtomicU64::new(1)),
        })
    }

    fn new_job(&self, kind: &str, message: &str) -> String {
        let seq = self.job_counter.fetch_add(1, Ordering::Relaxed);
        let id = format!("job-{}-{seq}", now_ms());
        let job = JobView {
            id: id.clone(),
            kind: kind.to_string(),
            status: "running".to_string(),
            progress: Some(0.0),
            message: message.to_string(),
            result: None,
            error: None,
            started_ms: now_ms(),
            updated_ms: now_ms(),
        };
        self.jobs.lock().unwrap().insert(id.clone(), job);
        id
    }

    fn update_job(&self, id: &str, f: impl FnOnce(&mut JobView)) {
        if let Some(job) = self.jobs.lock().unwrap().get_mut(id) {
            f(job);
            job.updated_ms = now_ms();
        }
    }

    fn set_progress(&self, id: &str, progress: Option<f32>, message: impl Into<String>) {
        self.update_job(id, |job| {
            job.progress = progress;
            job.message = message.into();
        });
    }

    fn finish_job(&self, id: &str, result: Value) {
        self.update_job(id, |job| {
            job.status = "done".to_string();
            job.progress = Some(1.0);
            job.message = "完成".to_string();
            job.result = Some(result);
        });
    }

    fn fail_job(&self, id: &str, error: impl Into<String>) {
        let error = error.into();
        self.update_job(id, |job| {
            job.status = "error".to_string();
            job.error = Some(error.clone());
            job.message = error;
        });
    }

    fn safe_path(&self, raw: &str) -> Result<PathBuf, String> {
        let path = PathBuf::from(raw);
        if !path.is_absolute() {
            return Err("路径必须是绝对路径".into());
        }
        let canonical = path
            .canonicalize()
            .map_err(|error| format!("无法访问 {}: {error}", path.display()))?;
        if !canonical.starts_with(&self.root) {
            return Err(format!("只能访问 {} 下的文件", self.root.display()));
        }
        Ok(canonical)
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or_default()
}

fn workspace_root() -> PathBuf {
    // Prefer an explicit env var so systemd units can point at the checkout.
    if let Some(root) = std::env::var_os("CUT_SPEEDUP_ROOT") {
        return PathBuf::from(root);
    }
    // target/debug/deps -> project root
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.as_path();
        for _ in 0..4 {
            if let Some(parent) = dir.parent() {
                dir = parent;
                if dir.join("Cargo.toml").is_file() && dir.join("crates").is_dir() {
                    return dir.to_path_buf();
                }
            }
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

// ---------------------------------------------------------------------------
// HTTP plumbing

fn json_error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn bad_request(message: impl Into<String>) -> Response {
    json_error(StatusCode::BAD_REQUEST, message)
}

fn internal(message: impl Into<String>) -> Response {
    json_error(StatusCode::INTERNAL_SERVER_ERROR, message)
}

#[derive(Debug, Deserialize)]
struct PathQuery {
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DirQuery {
    path: Option<String>,
}

#[derive(Debug, Serialize)]
struct DirEntry {
    name: String,
    path: String,
    is_dir: bool,
    is_video: bool,
    size: u64,
}

fn is_video(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase())
            .as_deref(),
        Some("mp4" | "mkv" | "mov" | "m4v" | "webm" | "avi" | "ts" | "flv")
    )
}

async fn index_page() -> Response {
    static_response(INDEX_HTML, "text/html; charset=utf-8")
}

async fn app_js() -> Response {
    static_response(APP_JS, "text/javascript; charset=utf-8")
}

async fn style_css() -> Response {
    static_response(STYLE_CSS, "text/css; charset=utf-8")
}

fn static_response(body: &'static str, mime: &'static str) -> Response {
    let mut response = Response::new(Body::from(body));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    response
}

async fn api_roots(State(state): State<AppState>) -> Json<Value> {
    let roots: Vec<Value> = state
        .roots
        .iter()
        .map(|path| json!({ "path": path.display().to_string(), "name": display_name(path) }))
        .collect();
    Json(json!({ "roots": roots, "home": state.root.display().to_string() }))
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_string())
        .unwrap_or_else(|| path.display().to_string())
}

async fn api_list(
    State(state): State<AppState>,
    Query(query): Query<DirQuery>,
) -> Response {
    let path = match query.path {
        Some(value) if !value.is_empty() => match state.safe_path(&value) {
            Ok(path) => path,
            Err(error) => return bad_request(error),
        },
        _ => state.root.clone(),
    };

    let mut entries = Vec::new();
    let read = match std::fs::read_dir(&path) {
        Ok(read) => read,
        Err(error) => return bad_request(format!("无法读取目录 {}: {error}", path.display())),
    };
    for entry in read.flatten() {
        let entry_path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        let is_dir = meta.is_dir();
        let video = !is_dir && is_video(&entry_path);
        if !is_dir && !video {
            continue;
        }
        entries.push(DirEntry {
            name,
            path: entry_path.display().to_string(),
            is_dir,
            is_video: video,
            size: meta.len(),
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Json(json!({
        "path": path.display().to_string(),
        "parent": path.parent().filter(|parent| parent.starts_with(&state.root)).map(|parent| parent.display().to_string()),
        "entries": entries,
    }))
    .into_response()
}

async fn api_probe(
    State(state): State<AppState>,
    Query(query): Query<PathQuery>,
) -> Response {
    let Some(raw) = query.path else {
        return bad_request("缺少 path");
    };
    let path = match state.safe_path(&raw) {
        Ok(path) => path,
        Err(error) => return bad_request(error),
    };
    match state.ffmpeg.probe(&path) {
        Ok(info) => Json(json!({
            "path": path.display().to_string(),
            "name": display_name(&path),
            "media": info,
        }))
        .into_response(),
        Err(error) => internal(error.to_string()),
    }
}

/// Byte-range streaming so `<video>` can seek without downloading the file.
async fn api_media(
    State(state): State<AppState>,
    Query(query): Query<PathQuery>,
    headers: HeaderMap,
) -> Response {
    let Some(raw) = query.path else {
        return bad_request("缺少 path");
    };
    let path = match state.safe_path(&raw) {
        Ok(path) => path,
        Err(error) => return bad_request(error),
    };
    let metadata = match tokio::fs::metadata(&path).await {
        Ok(metadata) => metadata,
        Err(error) => return bad_request(format!("无法读取文件: {error}")),
    };
    let len = metadata.len();
    let mime = mime_for(&path);

    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| parse_range(value, len));

    let (start, end) = match range {
        Some(range) => range,
        None => (0, len.saturating_sub(1)),
    };

    if len == 0 {
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime)
            .body(Body::empty())
            .unwrap();
    }
    if start >= len || end < start {
        return Response::builder()
            .status(StatusCode::RANGE_NOT_SATISFIABLE)
            .header(header::CONTENT_RANGE, format!("bytes */{len}"))
            .body(Body::empty())
            .unwrap();
    }

    let end = end.min(len - 1);
    let length = end - start + 1;

    let mut file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(error) => return internal(format!("打开文件失败: {error}")),
    };
    if file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
        return internal("seek 失败");
    }
    let stream = ReaderStream::new(file.take(length));
    let status = if range.is_some() {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };

    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_LENGTH, length.to_string())
        .header(header::ACCEPT_RANGES, "bytes");
    if range.is_some() {
        builder = builder.header(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{len}"),
        );
    }
    builder.body(Body::from_stream(stream)).unwrap()
}

fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("mp4" | "m4v") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mkv") => "video/x-matroska",
        Some("mov") => "video/quicktime",
        _ => "application/octet-stream",
    }
}

fn parse_range(value: &str, len: u64) -> Option<(u64, u64)> {
    let rest = value.strip_prefix("bytes=")?;
    let (start_text, end_text) = rest.split_once('-')?;
    if start_text.is_empty() {
        // Suffix range: last N bytes.
        let suffix: u64 = end_text.parse().ok()?;
        if suffix == 0 || len == 0 {
            return None;
        }
        return Some((len.saturating_sub(suffix), len - 1));
    }
    let start: u64 = start_text.parse().ok()?;
    let end = if end_text.is_empty() {
        len.saturating_sub(1)
    } else {
        end_text.parse().ok()?
    };
    Some((start, end))
}

// ---------------------------------------------------------------------------
// Analysis

#[derive(Debug, Deserialize)]
struct AnalyzeRequest {
    path: String,
    #[serde(default = "default_mode")]
    mode: String,
    #[serde(default = "default_speed")]
    speed: f64,
    #[serde(default = "default_noise_db")]
    noise_db: f64,
    #[serde(default = "default_min_silence")]
    min_silence_s: f64,
    #[serde(default = "default_margin_ms")]
    margin_ms: u64,
    #[serde(default = "default_min_gap_ms")]
    min_gap_ms: u64,
}

fn default_mode() -> String {
    "vad".to_string()
}
fn default_speed() -> f64 {
    8.0
}
fn default_noise_db() -> f64 {
    -30.0
}
fn default_min_silence() -> f64 {
    0.5
}
fn default_margin_ms() -> u64 {
    150
}
fn default_min_gap_ms() -> u64 {
    5_000
}

async fn api_analyze(
    State(state): State<AppState>,
    Json(request): Json<AnalyzeRequest>,
) -> Response {
    let path = match state.safe_path(&request.path) {
        Ok(path) => path,
        Err(error) => return bad_request(error),
    };
    if request.speed <= 0.0 || !request.speed.is_finite() {
        return bad_request("speed 必须大于 0");
    }
    let id = state.new_job("analyze", "正在读取视频信息…");
    let job_id = id.clone();
    let task_state = state.clone();
    tokio::task::spawn_blocking(move || {
        if let Err(error) = run_analyze(&task_state, &id, path, request) {
            task_state.fail_job(&id, error.to_string());
        }
    });
    Json(json!({ "job_id": job_id })).into_response()
}

fn run_analyze(
    state: &AppState,
    id: &str,
    path: PathBuf,
    request: AnalyzeRequest,
) -> anyhow::Result<()> {
    let info = state.ffmpeg.probe(&path)?;
    if info.duration_ms == 0 {
        anyhow::bail!("无法获取视频时长");
    }
    state.set_progress(id, Some(0.03), "已读取视频信息，开始分析…");

    let (plan, subtitles, speech) = match request.mode.as_str() {
        "silence" => {
            let noise = format!("{}dB", request.noise_db.round() as i64);
            let silences = state.ffmpeg.detect_silence(
                &path,
                &noise,
                request.min_silence_s,
                info.duration_ms,
            )?;
            state.set_progress(id, Some(0.8), "生成片段计划…");
            let plan = build_plan(
                info.duration_ms,
                info.has_audio,
                &silences,
                &PlanOptions {
                    voice_speed: 1.0,
                    silence_speed: request.speed,
                    margin_ms: request.margin_ms,
                },
            )?;
            (plan, Vec::new(), Vec::new())
        }
        _ => {
            let (strict, subtitles) = run_vad(state, id, &path, info.duration_ms)?;
            let speech: Vec<(u64, u64)> = strict.iter().map(|(start, end, _)| (*start, *end)).collect();
            state.set_progress(id, Some(0.95), "生成片段计划…");
            let plan = assemble_plan(
                info.duration_ms,
                &speech,
                request.min_gap_ms,
                request.margin_ms,
                request.speed,
                info.has_audio,
            );
            (plan, subtitles, speech)
        }
    };

    plan.validate()?;
    let result = json!({
        "path": path.display().to_string(),
        "name": display_name(&path),
        "media": info,
        "mode": request.mode,
        "options": {
            "speed": request.speed,
            "noise_db": request.noise_db,
            "min_silence_s": request.min_silence_s,
            "margin_ms": request.margin_ms,
            "min_gap_ms": request.min_gap_ms,
        },
        "plan": plan,
        "output_duration_ms": plan.total_output_ms(),
        "speech": speech,
        "subtitles": subtitles,
    });
    state.finish_job(id, result);
    Ok(())
}

/// VAD + ASR through the sherpa-onnx CLI. Returns strict speech segments and
/// subtitle lines, both on the original timeline.
fn run_vad(
    state: &AppState,
    id: &str,
    path: &Path,
    duration_ms: u64,
) -> anyhow::Result<(Vec<(u64, u64, String)>, Vec<Value>)> {
    let root = workspace_root();
    let model = root.join("models/sherpa-onnx-x-asr-zipformer-transducer-zh-en-int8-2026-06-03");
    let vad_model = root.join("models/silero_vad.onnx");
    if !model.is_dir() || !vad_model.is_file() {
        anyhow::bail!(
            "缺少 VAD/ASR 模型：请先跑 scripts/fetch-asr-android-model.sh（需要 models/silero_vad.onnx 和 {}）",
            model.display()
        );
    }
    let (exe, lib_dir) = discover_vad_exe(&root)?;

    let wav = state.tmp_dir.join(format!("{id}.wav"));
    state.set_progress(id, Some(0.05), "提取 16 kHz 单声道音频…");
    let status = Command::new(&state.ffmpeg.ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error"])
        .arg("-i")
        .arg(path)
        .args(["-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
        .arg(&wav)
        .status()?;
    if !status.success() {
        anyhow::bail!("ffmpeg 提取音频失败");
    }

    state.set_progress(id, Some(0.1), "VAD 检测人声 + ASR 转写…");
    let mut command = Command::new(&exe);
    if let Some(lib_dir) = &lib_dir {
        command.env("LD_LIBRARY_PATH", lib_dir);
    }
    command
        .arg(format!("--silero-vad-model={}", vad_model.display()))
        .arg("--silero-vad-min-silence-duration=0.25")
        .arg("--silero-vad-min-speech-duration=0.1")
        .arg(format!("--tokens={}", model.join("tokens.txt").display()))
        .arg(format!(
            "--encoder={}",
            model.join("encoder-epoch-99-avg-1.int8.onnx").display()
        ))
        .arg(format!(
            "--decoder={}",
            model.join("decoder-epoch-99-avg-1.onnx").display()
        ))
        .arg(format!(
            "--joiner={}",
            model.join("joiner-epoch-99-avg-1.int8.onnx").display()
        ))
        .arg("--num-threads=4")
        .arg(&wav)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn()?;
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = std::io::BufReader::new(stdout);
    let mut segments: Vec<(u64, u64, String)> = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        let read = std::io::BufRead::read_line(&mut reader, &mut line)?;
        if read == 0 {
            break;
        }
        if let Some((start_ms, end_ms, text)) = parse_vad_line(line.trim_end()) {
            let progress = 0.1 + 0.85 * (end_ms as f32 / duration_ms.max(1) as f32).min(1.0);
            state.set_progress(
                id,
                Some(progress),
                format!("VAD/ASR：已处理到 {}", format_time(end_ms)),
            );
            segments.push((start_ms, end_ms, text));
        }
    }
    let status = child.wait()?;
    let _ = std::fs::remove_file(&wav);
    if !status.success() {
        anyhow::bail!("sherpa-onnx VAD/ASR 失败（{status}）");
    }
    let subtitles: Vec<Value> = segments
        .iter()
        .map(|(start, end, text)| {
            json!({ "start_ms": start, "end_ms": end, "text": text })
        })
        .collect();
    Ok((segments, subtitles))
}

fn discover_vad_exe(root: &Path) -> anyhow::Result<(PathBuf, Option<PathBuf>)> {
    if let Some(exe) = std::env::var_os("CUT_SPEEDUP_VAD_ASR") {
        return Ok((PathBuf::from(exe), None));
    }
    let linux_dir = root.join("tools/sherpa-onnx-v2.0.1-linux-x64-shared-no-tts");
    let linux_exe = linux_dir.join("bin/sherpa-onnx-vad-with-offline-asr");
    if linux_exe.is_file() {
        return Ok((linux_exe, Some(linux_dir.join("lib"))));
    }
    let win_exe = root.join(
        "tools/sherpa-onnx/sherpa-onnx-v2.0.1-win-x64-shared-MD-Release-no-tts/bin/sherpa-onnx-vad-with-offline-asr.exe",
    );
    if win_exe.is_file() {
        return Ok((win_exe, None));
    }
    anyhow::bail!(
        "找不到 sherpa-onnx VAD CLI，可设置 CUT_SPEEDUP_VAD_ASR，或执行 scripts/fetch-sherpa-linux.sh"
    )
}

fn parse_vad_line(line: &str) -> Option<(u64, u64, String)> {
    let (range, text) = line.split_once(':')?;
    let (start, end) = range.split_once("--")?;
    let start_ms = (start.trim().parse::<f64>().ok()? * 1000.0).round() as u64;
    let end_ms = (end.trim().parse::<f64>().ok()? * 1000.0).round() as u64;
    if end_ms <= start_ms {
        return None;
    }
    Some((start_ms, end_ms, text.trim().to_string()))
}

/// Merge strict VAD segments into sentence groups, then turn the remaining
/// gaps into accelerated segments (same rule as the phone).
fn assemble_plan(
    duration_ms: u64,
    strict: &[(u64, u64)],
    min_gap_ms: u64,
    margin_ms: u64,
    speed: f64,
    has_audio: bool,
) -> cut_speedup_core::Plan {
    let mut groups: Vec<(u64, u64)> = Vec::new();
    for &(start, end) in strict {
        let start = start.min(duration_ms);
        let end = end.min(duration_ms);
        if end <= start {
            continue;
        }
        if let Some(last) = groups.last_mut() {
            if start.saturating_sub(last.1) < min_gap_ms {
                last.1 = last.1.max(end);
                continue;
            }
        }
        groups.push((start, end));
    }

    let mut segments: Vec<Segment> = Vec::new();
    let mut cursor = 0u64;
    let mut raw_cursor = 0u64;
    for (start, end) in groups {
        if start.saturating_sub(raw_cursor) >= min_gap_ms {
            let voice_start = cursor.max(start.saturating_sub(margin_ms));
            if voice_start > cursor {
                push_segment(&mut segments, cursor, voice_start, speed, SegmentKind::NonVoice);
            }
            let voice_end = (end + margin_ms).min(duration_ms).max(voice_start);
            push_segment(&mut segments, voice_start, voice_end, 1.0, SegmentKind::Voice);
            cursor = voice_end;
        } else {
            let voice_end = (end + margin_ms).min(duration_ms).max(cursor);
            push_segment(&mut segments, cursor, voice_end, 1.0, SegmentKind::Voice);
            cursor = voice_end;
        }
        raw_cursor = end;
    }
    if cursor < duration_ms {
        if duration_ms.saturating_sub(raw_cursor) >= min_gap_ms {
            push_segment(&mut segments, cursor, duration_ms, speed, SegmentKind::NonVoice);
        } else {
            push_segment(&mut segments, cursor, duration_ms, 1.0, SegmentKind::Voice);
        }
    }
    if segments.is_empty() {
        push_segment(&mut segments, 0, duration_ms, speed, SegmentKind::NonVoice);
    }
    cut_speedup_core::Plan {
        source_duration_ms: duration_ms,
        has_audio,
        segments,
    }
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
    if let Some(last) = segments.last_mut() {
        if last.kind == kind && (last.speed - speed).abs() < 1e-9 {
            last.end_ms = end_ms;
            return;
        }
    }
    segments.push(Segment {
        start_ms,
        end_ms,
        speed,
        kind,
    });
}

fn format_time(ms: u64) -> String {
    let total = ms / 1000;
    let hours = total / 3600;
    let minutes = total / 60 % 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

// ---------------------------------------------------------------------------
// Rendering

#[derive(Debug, Deserialize)]
struct RenderRequest {
    path: String,
    segments: Vec<Segment>,
    source_duration_ms: u64,
    #[serde(default)]
    name: Option<String>,
}

async fn api_render(
    State(state): State<AppState>,
    Json(request): Json<RenderRequest>,
) -> Response {
    let path = match state.safe_path(&request.path) {
        Ok(path) => path,
        Err(error) => return bad_request(error),
    };
    let plan = cut_speedup_core::Plan {
        source_duration_ms: request.source_duration_ms,
        has_audio: true,
        segments: request.segments,
    };
    if let Err(error) = plan.validate() {
        return bad_request(format!("片段计划无效: {error}"));
    }

    let id = state.new_job("render", "准备导出…");
    let job_id = id.clone();
    let task_state = state.clone();
    let name = request.name;
    tokio::task::spawn_blocking(move || {
        if let Err(error) = run_render(&task_state, &id, path, plan, name) {
            task_state.fail_job(&id, error.to_string());
        }
    });
    Json(json!({ "job_id": job_id })).into_response()
}

fn run_render(
    state: &AppState,
    id: &str,
    path: PathBuf,
    plan: cut_speedup_core::Plan,
    name: Option<String>,
) -> anyhow::Result<()> {
    let info = state.ffmpeg.probe(&path)?;
    let stem = name
        .filter(|value| !value.trim().is_empty())
        .map(|value| sanitize_name(&value))
        .unwrap_or_else(|| {
            sanitize_name(
                path.file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or("output"),
            )
        });
    let stamp = now_ms();
    let output = state.out_dir.join(format!("{stem}-{stamp}.mp4"));
    let script = state.tmp_dir.join(format!("{id}.filter"));
    let filter = build_filter_script(&plan, info.has_audio, info.fps);
    std::fs::write(&script, filter)?;

    state.set_progress(id, Some(0.01), "开始编码…");
    let expected_ms = plan.total_output_ms().max(1);
    let mut command = Command::new(&state.ffmpeg.ffmpeg);
    command
        .args(["-y", "-hide_banner", "-nostdin", "-loglevel", "error"])
        .arg("-i")
        .arg(&path)
        .arg("-filter_complex_script")
        .arg(&script)
        .args(["-map", "[outv]"]);
    if info.has_audio {
        command.args(["-map", "[outa]"]);
    }
    if let Some(fps) = info.fps {
        command.args(["-r", &format!("{fps:.6}")]);
    }
    command
        .args([
            "-fps_mode", "cfr",
            "-c:v", "libx264",
            "-preset", "veryfast",
            "-crf", "20",
            "-pix_fmt", "yuv420p",
        ])
        .args(["-movflags", "+faststart"]);
    if info.has_audio {
        command.args(["-c:a", "aac", "-b:a", "128k"]);
    } else {
        command.arg("-an");
    }
    command
        .arg("-progress")
        .arg("pipe:1")
        .arg("-nostats")
        .arg(&output)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn()?;
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = std::io::BufReader::new(stdout);
    let mut line = String::new();
    loop {
        line.clear();
        let read = std::io::BufRead::read_line(&mut reader, &mut line)?;
        if read == 0 {
            break;
        }
        if let Some(value) = line.trim().strip_prefix("out_time_us=") {
            if let Ok(us) = value.parse::<u64>() {
                let done_ms = us / 1000;
                let progress = (done_ms as f32 / expected_ms as f32).clamp(0.0, 0.99);
                state.set_progress(
                    id,
                    Some(progress),
                    format!(
                        "编码中：{} / {}（{:.0}%）",
                        format_time(done_ms),
                        format_time(expected_ms),
                        progress * 100.0
                    ),
                );
            }
        }
    }
    let output_status = child.wait()?;
    let _ = std::fs::remove_file(&script);
    if !output_status.success() {
        let mut stderr = String::new();
        if let Some(mut pipe) = child.stderr.take() {
            use std::io::Read;
            let _ = pipe.read_to_string(&mut stderr);
        }
        anyhow::bail!("ffmpeg 导出失败:\n{}", stderr.lines().rev().take(15).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n"));
    }
    state.finish_job(
        id,
        json!({
            "output": output.display().to_string(),
            "name": output.file_name().and_then(|value| value.to_str()).unwrap_or("output.mp4"),
            "download_url": format!("/api/download?id={id}"),
            "output_duration_ms": expected_ms,
            "segments": plan.segments.len(),
        }),
    );
    Ok(())
}

fn sanitize_name(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|ch| if ch.is_alphanumeric() || "-_.".contains(ch) { ch } else { '-' })
        .collect();
    let cleaned = cleaned.trim_matches('-').to_string();
    if cleaned.is_empty() {
        "output".into()
    } else {
        cleaned
    }
}

fn build_filter_script(plan: &cut_speedup_core::Plan, has_audio: bool, fps: Option<f64>) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut labels = String::new();
    for (index, segment) in plan.segments.iter().enumerate() {
        let start = segment.start_ms as f64 / 1000.0;
        let end = segment.end_ms as f64 / 1000.0;
        let speed = segment.speed;
        lines.push(format!(
            "[0:v]trim=start={start:.3}:end={end:.3},setpts=(PTS-STARTPTS)/{speed:.6},setsar=1[v{index}]"
        ));
        if has_audio {
            let audio = if (speed - 1.0).abs() < 1e-9 {
                format!("[0:a]atrim=start={start:.3}:end={end:.3},asetpts=PTS-STARTPTS[a{index}]")
            } else {
                format!(
                    "[0:a]atrim=start={start:.3}:end={end:.3},asetpts=PTS-STARTPTS,{}[a{index}]",
                    atempo_chain(speed)
                )
            };
            lines.push(audio);
            labels.push_str(&format!("[v{index}][a{index}]"));
        } else {
            labels.push_str(&format!("[v{index}]"));
        }
    }
    let count = plan.segments.len();
    if has_audio {
        lines.push(format!(
            "{labels}concat=n={count}:v=1:a=1[outv][outa]"
        ));
    } else {
        lines.push(format!("{labels}concat=n={count}:v=1:a=0[outv]"));
    }
    let mut text = lines.join(";\n");
    text.push('\n');
    if let Some(fps) = fps {
        // Keep the concat result at the source frame rate; the CLI also passes -r.
        let _ = fps;
    }
    text
}

fn atempo_chain(mut speed: f64) -> String {
    let mut parts = Vec::new();
    while speed > 2.0 + 1e-9 {
        parts.push("atempo=2".to_string());
        speed /= 2.0;
    }
    if (speed - 1.0).abs() > 1e-9 {
        parts.push(format!("atempo={speed:.6}"));
    }
    if parts.is_empty() {
        "anull".to_string()
    } else {
        parts.join(",")
    }
}

async fn api_download(
    State(state): State<AppState>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let Some(id) = query.get("id") else {
        return bad_request("缺少 id");
    };
    let job = state.jobs.lock().unwrap().get(id).cloned();
    let Some(job) = job else {
        return json_error(StatusCode::NOT_FOUND, "任务不存在");
    };
    let Some(result) = job.result else {
        return bad_request("任务还没有结果");
    };
    let Some(output) = result.get("output").and_then(|value| value.as_str()) else {
        return bad_request("任务没有输出文件");
    };
    let path = PathBuf::from(output);
    let Ok(canonical) = path.canonicalize() else {
        return json_error(StatusCode::NOT_FOUND, "输出文件不存在");
    };
    if !canonical.starts_with(&state.out_dir) {
        return bad_request("非法输出路径");
    }
    let data = match std::fs::read(&canonical) {
        Ok(data) => data,
        Err(error) => return internal(format!("读取输出失败: {error}")),
    };
    let len = data.len();
    let name = canonical
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("output.mp4");
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "video/mp4")
        .header(header::CONTENT_LENGTH, len.to_string())
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{name}\""),
        )
        .body(Body::from(data))
        .unwrap()
}

async fn api_job(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    match state.jobs.lock().unwrap().get(&id).cloned() {
        Some(job) => Json(job).into_response(),
        None => json_error(StatusCode::NOT_FOUND, "任务不存在"),
    }
}

// ---------------------------------------------------------------------------
// Wiring

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut port = DEFAULT_PORT;
    let mut host = String::from("0.0.0.0");
    let args: Vec<String> = std::env::args().collect();
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--port" | "-p" => {
                if let Some(value) = args.get(index + 1) {
                    port = value.parse()?;
                    index += 1;
                }
            }
            "--host" => {
                if let Some(value) = args.get(index + 1) {
                    host = value.clone();
                    index += 1;
                }
            }
            "--help" | "-h" => {
                println!("cut-speedup-web [--host 0.0.0.0] [--port 42006]");
                return Ok(());
            }
            other => anyhow::bail!("未知参数 {other}"),
        }
        index += 1;
    }

    let state = AppState::new()?;
    let app = Router::new()
        .route("/", get(index_page))
        .route("/static/app.js", get(app_js))
        .route("/static/style.css", get(style_css))
        .route("/api/roots", get(api_roots))
        .route("/api/list", get(api_list))
        .route("/api/probe", get(api_probe))
        .route("/api/media", get(api_media))
        .route("/api/analyze", post(api_analyze))
        .route("/api/render", post(api_render))
        .route("/api/download", get(api_download))
        .route("/api/jobs/{id}", get(api_job))
        .with_state(state.clone());

    let addr: SocketAddr = format!("{host}:{port}").parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("cut-speedup web: http://{addr}");
    println!("workspace: {}", workspace_root().display());
    println!("output: {}", state.out_dir.display());
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembles_leading_gap_as_silence_only_when_long_enough() {
        let plan = assemble_plan(20_000, &[(2_000, 3_000)], 5_000, 150, 4.0, true);
        assert_eq!(plan.segments[0].kind, SegmentKind::Voice);
        assert_eq!(plan.segments[0].start_ms, 0);

        let plan = assemble_plan(20_000, &[(6_000, 7_000)], 5_000, 150, 4.0, true);
        assert_eq!(plan.segments[0].kind, SegmentKind::NonVoice);
        assert_eq!(plan.segments[0].end_ms, 5_850);
    }

    #[test]
    fn plan_covers_whole_timeline() {
        let plan = assemble_plan(
            60_000,
            &[(1_000, 2_000), (8_000, 9_000), (55_000, 56_000)],
            5_000,
            150,
            8.0,
            true,
        );
        plan.validate().unwrap();
        assert_eq!(plan.segments.first().unwrap().start_ms, 0);
        assert_eq!(plan.segments.last().unwrap().end_ms, 60_000);
    }

    #[test]
    fn parses_vad_line() {
        assert_eq!(
            parse_vad_line("12.345 -- 15.678: 你好"),
            Some((12_345, 15_678, "你好".to_string()))
        );
        assert_eq!(parse_vad_line("noise"), None);
    }
}
