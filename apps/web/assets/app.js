"use strict";

const $ = (id) => document.getElementById(id);

const S = {
  path: null,
  plan: null,
  media: null,
  subtitles: [],
  durationMs: 0,
  activeSubtitle: -1,
  currentSegmentKey: "",
  jobTimer: null,
};

function fmtTime(ms) {
  if (!isFinite(ms) || ms < 0) ms = 0;
  const total = Math.floor(ms / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const milli = Math.floor(ms % 1000);
  return h > 0
    ? `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`
    : `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}.${String(milli).padStart(3, "0").slice(0, 1)}`;
}

function fmtDuration(ms) {
  const total = Math.max(0, Math.round(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  return h > 0 ? `${h}h${String(m).padStart(2, "0")}m${String(s).padStart(2, "0")}s`
              : `${m}m${String(s).padStart(2, "0")}s`;
}

function fmtSize(bytes) {
  if (!bytes) return "";
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes, index = 0;
  while (value >= 1024 && index < units.length - 1) { value /= 1024; index++; }
  return `${value.toFixed(value >= 10 ? 0 : 1)}${units[index]}`;
}

function toast(message, ms = 5000) {
  const el = $("toast");
  el.textContent = message;
  el.classList.remove("hidden");
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => el.classList.add("hidden"), ms);
}

async function api(path, options) {
  const response = await fetch(path, options);
  const text = await response.text();
  let data = null;
  try { data = text ? JSON.parse(text) : null; } catch { data = { error: text }; }
  if (!response.ok) throw new Error((data && data.error) || response.statusText);
  return data;
}

// ---------------------------------------------------------------------------
// Library

async function initLibrary() {
  try {
    const data = await api("/api/roots");
    const box = $("roots");
    box.innerHTML = "";
    for (const root of data.roots) {
      const button = document.createElement("button");
      button.textContent = root.name;
      button.title = root.path;
      button.addEventListener("click", () => loadDir(root.path));
      box.appendChild(button);
    }
    loadDir(data.home);
  } catch (error) {
    toast(`加载视频库失败：${error.message}`);
  }
}

async function loadDir(path) {
  try {
    const data = await api(`/api/list?path=${encodeURIComponent(path)}`);
    $("path-input").value = data.path;
    $("go-up").disabled = !data.parent;
    $("go-up").dataset.parent = data.parent || "";
    const list = $("entries");
    list.innerHTML = "";
    if (!data.entries.length) {
      const li = document.createElement("li");
      li.className = "muted";
      li.textContent = "（空目录）";
      list.appendChild(li);
      return;
    }
    for (const entry of data.entries) {
      const li = document.createElement("li");
      li.title = entry.path;
      const icon = document.createElement("span");
      icon.className = "ico";
      icon.textContent = entry.is_dir ? "📁" : "🎬";
      const name = document.createElement("span");
      name.textContent = entry.name;
      name.style.overflow = "hidden";
      name.style.textOverflow = "ellipsis";
      li.append(icon, name);
      if (!entry.is_dir) {
        const size = document.createElement("span");
        size.className = "size";
        size.textContent = fmtSize(entry.size);
        li.appendChild(size);
      }
      if (entry.path === S.path) li.classList.add("active");
      li.addEventListener("click", () => {
        if (entry.is_dir) loadDir(entry.path);
        else selectVideo(entry.path);
      });
      list.appendChild(li);
    }
  } catch (error) {
    toast(`打开目录失败：${error.message}`);
  }
}

// ---------------------------------------------------------------------------
// Video + plan

async function selectVideo(path) {
  try {
    const data = await api(`/api/probe?path=${encodeURIComponent(path)}`);
    S.path = data.path;
    S.media = data.media;
    S.plan = null;
    S.subtitles = [];
    S.durationMs = data.media.duration_ms;

    $("video-title").textContent = data.name;
    const m = data.media;
    $("video-meta").textContent =
      `${fmtDuration(m.duration_ms)} · ${m.width || "?"}×${m.height || "?"} · ${m.fps ? m.fps.toFixed(2) : "?"} fps · ` +
      (m.has_audio ? `${m.audio_sample_rate || "?"} Hz ${m.audio_channels || "?"}ch` : "无音轨");
    $("player").src = `/api/media?path=${encodeURIComponent(data.path)}`;
    $("video-empty").classList.add("hidden");
    $("render").disabled = true;
    $("download").classList.add("hidden");
    $("render-job").classList.add("hidden");
    $("analyze-job").classList.add("hidden");
    resetPlanUI();
    document.querySelectorAll("#entries li").forEach((li) => {
      li.classList.toggle("active", li.title === data.path);
    });
  } catch (error) {
    toast(`读取视频失败：${error.message}`);
  }
}

function resetPlanUI() {
  $("timeline-segments").innerHTML = "";
  $("playhead").style.left = "0%";
  $("stats").innerHTML = "";
  $("subtitles").innerHTML = '<div class="empty small">分析后显示，点一条跳到原片</div>';
  $("subtitle-count").textContent = "";
  $("segment-now").textContent = "—";
  S.currentSegmentKey = "";
  S.activeSubtitle = -1;
}

function currentOptions() {
  return {
    path: S.path,
    mode: $("mode").value,
    speed: Number($("speed").value) || 8,
    noise_db: Number($("noise").value) || -30,
    min_silence_s: Number($("min-silence").value) || 0.5,
    margin_ms: Math.round(Number($("margin").value) || 0),
    min_gap_ms: Math.round((Number($("min-gap").value) || 5) * 1000),
  };
}

async function analyze() {
  if (!S.path) { toast("先选一个视频"); return; }
  $("analyze").disabled = true;
  $("analyze-job").classList.remove("hidden");
  setProgress("analyze", 0, "提交分析任务…");
  try {
    const { job_id } = await api("/api/analyze", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(currentOptions()),
    });
    const job = await pollJob(job_id, "analyze");
    applyPlan(job.result);
  } catch (error) {
    toast(`分析失败：${error.message}`);
  } finally {
    $("analyze").disabled = false;
  }
}

function applyPlan(result) {
  if (!result || !result.plan) { toast("分析结果为空"); return; }
  S.plan = result.plan;
  S.durationMs = result.plan.source_duration_ms;
  S.subtitles = result.subtitles || [];
  drawTimeline(result.plan);
  drawStats(result);
  drawSubtitles();
  $("render").disabled = false;
  setProgress("analyze", 1, `完成：${result.plan.segments.length} 段，输出 ${fmtDuration(result.output_duration_ms)}`);
}

function drawTimeline(plan) {
  const box = $("timeline-segments");
  box.innerHTML = "";
  const total = plan.source_duration_ms || 1;
  for (const segment of plan.segments) {
    const span = document.createElement("span");
    span.className = segment.kind === "voice" ? "voice" : "silence";
    span.style.left = `${(segment.start_ms / total) * 100}%`;
    span.style.width = `${((segment.end_ms - segment.start_ms) / total) * 100}%`;
    span.title = `${segment.kind === "voice" ? "人声" : "静音"} ${fmtTime(segment.start_ms)} - ${fmtTime(segment.end_ms)} · ${segment.speed}×`;
    box.appendChild(span);
  }
  updatePlayhead();
}

function drawStats(result) {
  const plan = result.plan;
  let voice = 0, silence = 0;
  for (const segment of plan.segments) {
    const d = segment.end_ms - segment.start_ms;
    if (segment.kind === "voice") voice += d; else silence += d;
  }
  const source = plan.source_duration_ms;
  const output = result.output_duration_ms;
  const saved = source > 0 ? (1 - output / source) * 100 : 0;
  const items = [
    ["原片时长", fmtDuration(source)],
    ["成片时长", fmtDuration(output)],
    ["节省", `${saved.toFixed(1)}%`],
    ["片段", `${plan.segments.length} 段`],
    ["人声", fmtDuration(voice)],
    ["需加速", fmtDuration(silence)],
    ["检测方式", result.mode === "vad" ? "VAD + ASR" : "响度阈值"],
    ["字幕", `${(result.subtitles || []).length} 条`],
  ];
  $("stats").innerHTML = items
    .map(([k, v]) => `<div class="stat"><div class="k">${k}</div><div class="v">${v}</div></div>`)
    .join("");
}

function drawSubtitles() {
  const box = $("subtitles");
  box.innerHTML = "";
  $("subtitle-count").textContent = S.subtitles.length ? `${S.subtitles.length} 条` : "";
  if (!S.subtitles.length) {
    box.innerHTML = '<div class="empty small">这次分析没有字幕（响度阈值模式不含字幕）</div>';
    return;
  }
  S.subtitles.forEach((line, index) => {
    const div = document.createElement("div");
    div.className = "line";
    div.dataset.index = index;
    div.innerHTML = `<div class="t">${fmtTime(line.start_ms)} → ${fmtTime(line.end_ms)}</div><div class="x"></div>`;
    div.querySelector(".x").textContent = line.text;
    div.addEventListener("click", () => {
      $("player").currentTime = line.start_ms / 1000;
      $("player").play().catch(() => {});
    });
    box.appendChild(div);
  });
}

// ---------------------------------------------------------------------------
// Playback

function segmentAt(ms) {
  if (!S.plan) return null;
  const segments = S.plan.segments;
  let low = 0, high = segments.length - 1;
  while (low <= high) {
    const mid = (low + high) >> 1;
    const segment = segments[mid];
    if (ms < segment.start_ms) high = mid - 1;
    else if (ms >= segment.end_ms) low = mid + 1;
    else return segment;
  }
  return null;
}

function updatePlayhead() {
  const player = $("player");
  const total = S.durationMs || player.duration * 1000 || 1;
  const ms = player.currentTime * 1000;
  $("playhead").style.left = `${Math.min(100, (ms / total) * 100)}%`;
}

function onTimeUpdate() {
  const player = $("player");
  const ms = player.currentTime * 1000;
  updatePlayhead();

  const segment = segmentAt(ms);
  if (segment) {
    const key = `${segment.start_ms}-${segment.end_ms}-${segment.speed}-${segment.kind}`;
    $("segment-now").textContent =
      `${segment.kind === "voice" ? "人声" : "静音"} ${fmtTime(segment.start_ms)} - ${fmtTime(segment.end_ms)} · ${segment.speed}×`;
    if ($("follow-plan").checked && !player.paused) {
      const rate = Math.min(Math.max(segment.speed, 0.25), 16);
      if (Math.abs(player.playbackRate - rate) > 0.01) player.playbackRate = rate;
      // Browsers cap playbackRate; for very high plan speeds skip the rest.
      if (segment.kind !== "voice" && segment.speed > 16 && S.currentSegmentKey !== key) {
        S.currentSegmentKey = key;
        player.currentTime = segment.end_ms / 1000;
        return;
      }
    } else if (Math.abs(player.playbackRate - 1) > 0.01) {
      player.playbackRate = 1;
    }
    if (S.currentSegmentKey !== key) S.currentSegmentKey = key;
  } else if (Math.abs(player.playbackRate - 1) > 0.01) {
    player.playbackRate = 1;
  }

  let active = -1;
  for (let i = 0; i < S.subtitles.length; i++) {
    if (ms >= S.subtitles[i].start_ms && ms < S.subtitles[i].end_ms) { active = i; break; }
    if (S.subtitles[i].start_ms > ms) break;
  }
  if (active !== S.activeSubtitle) {
    S.activeSubtitle = active;
    document.querySelectorAll(".subtitles .line").forEach((node, index) => {
      node.classList.toggle("active", index === active);
      if (index === active) node.scrollIntoView({ block: "nearest", behavior: "smooth" });
    });
  }
}

// ---------------------------------------------------------------------------
// Jobs

function setProgress(kind, ratio, message) {
  const fill = $(`${kind}-bar`);
  const msg = $(`${kind}-msg`);
  const pct = $(`${kind}-pct`);
  const clamped = Math.max(0, Math.min(1, ratio || 0));
  if (fill) fill.style.width = `${clamped * 100}%`;
  if (msg) msg.textContent = message;
  if (pct) pct.textContent = `${Math.round(clamped * 100)}%`;
}

async function pollJob(id, kind) {
  const started = Date.now();
  for (;;) {
    const job = await api(`/api/jobs/${id}`);
    const ratio = job.progress == null ? 0 : job.progress;
    setProgress(kind, ratio, job.message || "处理中…");
    if (job.status === "done") return job;
    if (job.status === "error") throw new Error(job.error || "任务失败");
    if (Date.now() - started > 4 * 60 * 60 * 1000) throw new Error("任务超时");
    await new Promise((resolve) => setTimeout(resolve, 700));
  }
}

async function render() {
  if (!S.plan) { toast("先分析"); return; }
  $("render").disabled = true;
  $("download").classList.add("hidden");
  $("render-job").classList.remove("hidden");
  setProgress("render", 0, "提交导出任务…");
  try {
    const { job_id } = await api("/api/render", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        path: S.path,
        segments: S.plan.segments,
        source_duration_ms: S.plan.source_duration_ms,
        name: $("output-name").value.trim() || null,
      }),
    });
    const job = await pollJob(job_id, "render");
    const link = $("download");
    link.href = job.result.download_url;
    link.textContent = `下载 ${job.result.name}（${fmtDuration(job.result.output_duration_ms)}）`;
    link.classList.remove("hidden");
  } catch (error) {
    toast(`导出失败：${error.message}`);
  } finally {
    $("render").disabled = false;
  }
}

// ---------------------------------------------------------------------------
// Wiring

$("refresh-dir").addEventListener("click", () => loadDir($("path-input").value));
$("go-up").addEventListener("click", () => {
  const parent = $("go-up").dataset.parent;
  if (parent) loadDir(parent);
});
$("go-path").addEventListener("click", () => loadDir($("path-input").value.trim()));
$("path-input").addEventListener("keydown", (event) => {
  if (event.key === "Enter") loadDir($("path-input").value.trim());
});
$("analyze").addEventListener("click", analyze);
$("render").addEventListener("click", render);

const player = $("player");
player.addEventListener("timeupdate", onTimeUpdate);
player.addEventListener("seeked", onTimeUpdate);
player.addEventListener("loadedmetadata", () => {
  if (!S.media) S.durationMs = player.duration * 1000;
  updatePlayhead();
});

$("timeline").addEventListener("click", (event) => {
  if (!S.plan) return;
  const rect = $("timeline").getBoundingClientRect();
  const ratio = Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width));
  player.currentTime = (S.plan.source_duration_ms * ratio) / 1000;
});

initLibrary();
