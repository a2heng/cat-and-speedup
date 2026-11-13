#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use cut_speedup_asr::{
    AsrEngine, AsrOptions as EngineAsrOptions, DevicePolicy, SherpaEngine, SherpaModelKind,
};
use cut_speedup_core::{
    Ffmpeg, MediaInfo, Plan, RenderOptions, RenderReport, SegmentKind, Transcript, build_plan,
    ffmpeg::AsrOptions as CoreAsrOptions, map_transcript,
};
use gpui::{
    AnyElement, App, Application, Bounds, ClickEvent, Context, SharedString, Window, WindowBounds,
    WindowOptions, div, img, prelude::*, px, rgb, size,
};
use tempfile::TempDir;

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1180.0), px(760.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| AppState::new()),
        )
        .unwrap();
        cx.activate(true);
    });
}

struct AppState {
    input: Option<PathBuf>,
    plan: Option<Plan>,
    transcript: Option<Transcript>,
    poster: Option<Arc<Path>>,
    selected: Option<usize>,
    status: String,
    error: Option<String>,
    busy: bool,
}

impl AppState {
    fn new() -> Self {
        Self {
            input: None,
            plan: None,
            transcript: None,
            poster: None,
            selected: None,
            status: "打开一个视频开始（FunASR 本地离线运行）".into(),
            error: None,
            busy: false,
        }
    }

    fn open_file(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Video", &["mp4", "mov", "mkv", "webm", "avi", "m4v"])
            .pick_file()
        else {
            return;
        };

        self.input = Some(path.clone());
        self.plan = None;
        self.transcript = None;
        self.poster = None;
        self.selected = None;
        self.error = None;
        self.status = format!("已选择 {}", path.display());
        cx.notify();
        self.analyze(cx);
    }

    fn analyze(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self.input.clone() else {
            return;
        };

        self.run_task(
            cx,
            "分析中：检测静音并生成片段计划…",
            move || analyze_video(&input),
            |this, (plan, poster)| {
                this.status = format!(
                    "分析完成：{} 段，{:.1}s -> {:.1}s",
                    plan.segments.len(),
                    plan.source_duration_ms as f64 / 1000.0,
                    plan.total_output_ms() as f64 / 1000.0
                );
                this.plan = Some(plan);
                this.poster = Some(poster);
            },
        );
    }

    fn transcribe(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self.input.clone() else {
            return;
        };

        self.run_task(
            cx,
            "转写中：裁语音段 -> 本地 FunASR -> 映射回原时间轴…",
            move || transcribe_video(&input),
            |this, (plan, transcript)| {
                this.status = format!("字幕完成：{} 条", transcript.segments.len());
                this.plan = Some(plan);
                this.transcript = Some(transcript);
                this.selected = Some(0);
            },
        );
    }

    fn render(&mut self, cx: &mut Context<Self>) {
        let (Some(input), Some(plan)) = (self.input.clone(), self.plan.clone()) else {
            return;
        };
        let Some(output) = rfd::FileDialog::new()
            .set_file_name("output.mp4")
            .add_filter("MP4", &["mp4"])
            .save_file()
        else {
            return;
        };

        self.run_task(
            cx,
            "出片中：分段重编码 + 拼接…",
            move || render_video(&input, &plan, &output),
            |this, report| {
                this.status = format!(
                    "出片完成：{}（{} 段，{:.1}s）",
                    report.output.display(),
                    report.segments,
                    report.output_duration_ms as f64 / 1000.0
                );
            },
        );
    }

    fn select_subtitle(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(input) = self.input.clone() else {
            return;
        };
        let Some(segment) = self
            .transcript
            .as_ref()
            .and_then(|transcript| transcript.segments.get(index))
            .cloned()
        else {
            return;
        };

        self.selected = Some(index);
        let start_ms = segment.start_ms;
        let text = segment.text.clone();
        self.run_task(
            cx,
            "取字幕对应画面…",
            move || extract_poster(&input, start_ms),
            move |this, poster| {
                this.poster = Some(poster);
                this.status = format!("[{}] {}", format_time(start_ms), text);
            },
        );
    }

    fn run_task<T, F, D>(
        &mut self,
        cx: &mut Context<Self>,
        label: impl Into<String>,
        task: F,
        done: D,
    ) where
        T: Send + 'static,
        F: FnOnce() -> Result<T> + Send + 'static,
        D: FnOnce(&mut Self, T) + 'static,
    {
        if self.busy {
            return;
        }

        self.busy = true;
        self.error = None;
        self.status = label.into();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { task() }).await;

            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(value) => done(this, value),
                    Err(error) => {
                        this.status = "失败".into();
                        this.error = Some(format!("{error:#}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for AppState {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let can_analyze = self.input.is_some() && !self.busy;
        let can_render = self.input.is_some() && self.plan.is_some() && !self.busy;

        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .p_3()
            .border_b_1()
            .border_color(rgb(0x2b2b2b))
            .child(
                div()
                    .text_lg()
                    .font_weight(gpui::FontWeight::BOLD)
                    .child("cut-speedup"),
            )
            .child(div().flex_1())
            .child(action_button(
                "打开视频",
                !self.busy,
                cx.listener(|this, _, window, cx| this.open_file(window, cx)),
            ))
            .child(action_button(
                "分析",
                can_analyze,
                cx.listener(|this, _, _, cx| this.analyze(cx)),
            ))
            .child(action_button(
                "转写字幕",
                can_analyze,
                cx.listener(|this, _, _, cx| this.transcribe(cx)),
            ))
            .child(action_button(
                "出片",
                can_render,
                cx.listener(|this, _, _, cx| this.render(cx)),
            ));

        let poster: AnyElement = match &self.poster {
            Some(path) => img(path.clone())
                .max_w_full()
                .max_h_full()
                .into_any_element(),
            None => div()
                .flex()
                .items_center()
                .justify_center()
                .size_full()
                .bg(rgb(0x181818))
                .text_color(rgb(0x777777))
                .child("打开视频后显示预览帧")
                .into_any_element(),
        };

        let plan_summary = match &self.plan {
            Some(plan) => format!(
                "{} 段 · 原片 {:.1}s · 成片 {:.1}s",
                plan.segments.len(),
                plan.source_duration_ms as f64 / 1000.0,
                plan.total_output_ms() as f64 / 1000.0
            ),
            None => "尚未分析".to_string(),
        };

        let plan_rows: Vec<AnyElement> = self
            .plan
            .as_ref()
            .map(|plan| {
                plan.segments
                    .iter()
                    .map(|segment| {
                        let kind = match segment.kind {
                            SegmentKind::Voice => "人声",
                            SegmentKind::NonVoice => "静音",
                        };
                        div()
                            .flex()
                            .gap_2()
                            .text_sm()
                            .text_color(if segment.kind == SegmentKind::Voice {
                                rgb(0x9ad6a8)
                            } else {
                                rgb(0x8fb7e8)
                            })
                            .child(format!(
                                "{} - {}",
                                format_time(segment.start_ms),
                                format_time(segment.end_ms)
                            ))
                            .child(format!("{kind} ×{:.1}", segment.speed))
                            .into_any_element()
                    })
                    .collect()
            })
            .unwrap_or_default();

        let left = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .p_3()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .rounded_md()
                    .overflow_hidden()
                    .bg(rgb(0x181818))
                    .child(poster),
            )
            .child(div().text_sm().child(plan_summary))
            .child(
                div()
                    .id("segments")
                    .max_h(px(180.0))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(plan_rows),
            );

        let subtitle_rows: Vec<AnyElement> = self
            .transcript
            .as_ref()
            .map(|transcript| {
                transcript
                    .segments
                    .iter()
                    .enumerate()
                    .map(|(index, segment)| {
                        let selected = self.selected == Some(index);
                        div()
                            .id(SharedString::from(format!("subtitle-{index}")))
                            .flex()
                            .items_start()
                            .gap_2()
                            .p_2()
                            .rounded_sm()
                            .cursor_pointer()
                            .when(selected, |this| this.bg(rgb(0x252525)))
                            .hover(|this| this.bg(rgb(0x202020)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_subtitle(index, cx);
                            }))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x8fb7e8))
                                    .child(format_time(segment.start_ms)),
                            )
                            .child(div().flex_1().text_sm().child(segment.text.clone()))
                            .into_any_element()
                    })
                    .collect()
            })
            .unwrap_or_default();

        let right = div()
            .flex()
            .flex_col()
            .w(px(430.0))
            .border_l_1()
            .border_color(rgb(0x2b2b2b))
            .child(
                div()
                    .p_3()
                    .border_b_1()
                    .border_color(rgb(0x2b2b2b))
                    .child("字幕（点击跳转画面）"),
            )
            .child(
                div()
                    .id("subtitles")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(subtitle_rows),
            );

        let error = self.error.clone().map(|error| {
            div()
                .text_color(rgb(0xff7b72))
                .child(error)
                .into_any_element()
        });

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x111111))
            .text_color(rgb(0xeeeeee))
            .child(header)
            .child(div().flex().flex_1().min_h_0().child(left).child(right))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_2()
                    .border_t_1()
                    .border_color(rgb(0x2b2b2b))
                    .text_sm()
                    .child(self.status.clone())
                    .children(error),
            )
    }
}

fn action_button(
    label: &'static str,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let base = div()
        .id(label)
        .px_3()
        .py_1()
        .rounded_sm()
        .border_1()
        .border_color(rgb(0x3a3a3a))
        .bg(rgb(0x1d1d1d))
        .text_sm()
        .child(label);

    if enabled {
        base.cursor_pointer()
            .hover(|this| this.bg(rgb(0x2a2a2a)))
            .on_click(on_click)
            .into_any_element()
    } else {
        base.opacity(0.35).into_any_element()
    }
}

fn analyze_video(input: &Path) -> Result<(Plan, Arc<Path>)> {
    let ffmpeg = Ffmpeg::discover()?;
    let info = ffmpeg.probe(input)?;
    let plan = make_plan(&ffmpeg, input, &info)?;
    let at_ms = plan
        .segments
        .iter()
        .find(|segment| segment.kind == SegmentKind::Voice)
        .map(|segment| segment.start_ms)
        .unwrap_or(0);
    let poster = extract_poster(input, at_ms)?;
    Ok((plan, poster))
}

fn transcribe_video(input: &Path) -> Result<(Plan, Transcript)> {
    let ffmpeg = Ffmpeg::discover()?;
    let info = ffmpeg.probe(input)?;
    let plan = make_plan(&ffmpeg, input, &info)?;

    let temp = TempDir::new()?;
    let wav = temp.path().join("asr.wav");
    let map = ffmpeg.prepare_asr(
        input,
        &plan,
        &wav,
        &CoreAsrOptions {
            sample_rate: 16_000,
            merge_gap_ms: 500,
        },
    )?;

    let mut engine = SherpaEngine::from_env(SherpaModelKind::FunAsrNano)?;
    let result = engine.transcribe_wav(
        &wav,
        &EngineAsrOptions {
            language: Some("zh".into()),
            translate: false,
            initial_prompt: None,
            threads: 8,
            device: DevicePolicy::Cpu,
        },
    )?;

    Ok((plan, map_transcript(&result.transcript, &map)))
}

fn render_video(input: &Path, plan: &Plan, output: &Path) -> Result<RenderReport> {
    let ffmpeg = Ffmpeg::discover()?;
    let info = ffmpeg.probe(input)?;
    Ok(ffmpeg.render(input, plan, output, &info, &RenderOptions::default())?)
}

fn make_plan(ffmpeg: &Ffmpeg, input: &Path, info: &MediaInfo) -> Result<Plan> {
    let silences = if info.has_audio {
        ffmpeg.detect_silence(input, "-30dB", 0.5, info.duration_ms)?
    } else {
        Vec::new()
    };

    Ok(build_plan(
        info.duration_ms,
        info.has_audio,
        &silences,
        &Default::default(),
    )?)
}

fn extract_poster(input: &Path, at_ms: u64) -> Result<Arc<Path>> {
    let ffmpeg = Ffmpeg::discover()?;
    let dir = std::env::temp_dir().join("cut-speedup-ui");
    std::fs::create_dir_all(&dir)?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let path = dir.join(format!("frame-{nanos}.png"));
    ffmpeg.extract_frame(input, at_ms, &path)?;
    Ok(Arc::from(path.into_boxed_path()))
}

fn format_time(ms: u64) -> String {
    let total_seconds = ms / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    let millis = ms % 1000;
    format!("{minutes:02}:{seconds:02}.{millis:03}")
}
