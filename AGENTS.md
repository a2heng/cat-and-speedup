# cat-and-speedup

2026-11-13 创建的空仓库：未 `git init`，无代码、构建脚本、CI。别假设 `cargo`/`gradlew` 已可用。

## 产品硬约束

- 长视频按时间轴自动切分，片段严格保持原顺序；禁止按人声/静音分组或重排。
- 有人声段 1.0x 保留；无人声段按可配置倍速加速（不要写死倍速），输出音画同步。
- 保持分辨率、帧率与画面内容；编码参数尽量贴近源，除非明确要求改。
- 桌面 = GPUI（Rust）；Android = 规范原生 Kotlin（Gradle + Jetpack Compose）。Android 不要用 gpui-mobile/GPUI：GPUI 官方无 Android 后端，gpui-mobile 只是第三方 0.1.0，也不符合“原生 Kotlin”要求。
- ASR 只用于“看字幕对应原片、快速了解内容、辅助编排”，不是给成片加字幕；不默认烧字幕，后续需要再加独立导出。

## 检测策略（阈值）

- 检测就走响度/静音阈值（FFmpeg `silencedetect` 或自算 RMS/peak），不引入模型。干净录音只要底噪够低，这一层就够用，且便宜、可解释、易调参。
- 阈值、最短静音、片段边界余量都必须可配置；基线：`ffmpeg -i in.mp4 -af silencedetect=noise=-30dB:d=0.5 -f null -`。
- 已知误差：音乐/噪声会被当成有人声，导致该加速的没加速。这是可接受的 MVP 误差，先靠灵敏度、最短静音、边界余量调参，不上模型。
- 检测结果同时驱动成片分段和 ASR 预缩短，不要在两处各写一套阈值逻辑。

## ASR 定位（只做字幕预览）

- 目的：看字幕对应原片，马上知道内容、快速编排；不是给成片加字幕。ASR 失败不能阻塞成片渲染。
- ASR 前先裁掉长非人声段：保留语音两侧 100–200ms；短于 300–500ms 的语音间隙不要切，避免把一句话切碎。
- 裁出来的音频要带 `edited_ms -> original_ms` 映射，ASR 结果必须落回**原时间轴**；预览原片直接叠加，预览成片再按 Segment 计划映射。
- 字幕面板是核心入口：跟随播放高亮、点击字幕跳转视频；不要做成只读的附属文本。
- 字幕数据独立存 `Transcript { start_ms, end_ms, text }`，默认不烧进视频、不默认导出 SRT。

## 架构

- `crates/core`（Rust，无 UI）：阈值检测 → 有序 `Segment { start_ms, end_ms, speed }` 计划 → ffmpeg 分段渲染/拼接；同时产出 ASR 语音段 + 时间映射 + 原时间轴 `Transcript`。
- `crates/asr`：`AsrEngine` trait + whisper.cpp 实现（CPU 默认；`gpu-vulkan`/`gpu-cuda` feature 可选）+ 通用 OpenAI 兼容 HTTP 后端（FunASR/Qwen3-ASR/FireRedASR2 都能以本地服务接入）。模型按需下载到 `models/`（gitignore），也支持 `CAT_SPEEDUP_MODEL_DIR`。
- `crates/cli`：`cat-speedup` 命令（doctor/analyze/render/prepare-asr/transcribe/map-transcript）。桌面和 Android 都复用 core。
- `apps/desktop`：GPUI 桌面壳；文件选择、分析、FunASR 转写、字幕点击跳帧、出片。当前是帧预览，不是连续播放器。
- `apps/android`：Kotlin/Compose 播放器壳，Media3 播放 + 导入原时间轴字幕 JSON；设备端切分/加速未做，不接入 Cargo workspace。
- 桌面和 Android 首屏都要有“视频 + 字幕面板”：字幕跟播高亮、点击跳转，作为快速看内容和编排的主界面；成片渲染放在其后。
- 桌面 v1 已落地（帧预览 + 字幕跳帧）；连续播放和 Android 端待做。
- 桌面依赖 `gpui` 0.2.2（crates.io）；Android 用标准 Android Studio 工程，别混入 GPUI。

## 必须避开的坑

- 变速必须重编码：视频 `setpts=PTS/<speed>`，音频 `atempo=<speed>`；旧版 atempo 上限 2.0，>2x 需串联。
- 不要对源片段 `-c copy` 后直接 concat：切点不在关键帧会花屏/音画不同步；统一重编码后再 concat，或整条 `filter_complex` 一次出片。
- 手机源常是 VFR；拼接前统一 CFR（`-fps_mode cfr`），否则长片会漂移。
- 长视频禁止抽全帧或全片进内存（jumpcutter 的磁盘炸弹）；按块处理，临时文件放 cache 并清理。
- 片段边界留 100–200ms 余量防吞字；时间用整数毫秒，禁止浮点累加；无音轨按全片无人声处理。
- `whisper-rs-sys` 首次编译要 CMake + libclang：先 `source scripts/env.sh`（VS BuildTools 的 cmake + `tools/libclang`）。不要在 Windows 上设 `WHISPER_DONT_GENERATE_BINDINGS=1`，仓库自带 bindings 是 Linux 结构体尺寸，会直接编不过。
- `--device auto` 只有在本构建启用了 `gpu-vulkan`/`gpu-cuda` 时才可能用 GPU；CPU-only 构建必须如实显示 CPU，不能因为 `use_gpu(true)` 成功就报 GPU。
- Android：ffmpeg-kit 2025-01 已退役，用维护中的 fork 或 MediaCodec/Media3；native .so 必须支持 16KB page size（2025-11-01 起 Play 要求）；SAF `content://` 不能直接喂 ffmpeg，先复制到 cache。

## ASR 路线（2026-11 权衡，未全部落地）

- 默认/Android/CPU：FunASR，**`Fun-ASR-Nano-2512`（800M）** 是当前质量优先的选择；CPU 上要字级时间戳时退回 **Paraformer-zh + FSMN 分段 + CT-Punc**。
- 多语言：Fun-ASR-MLT-Nano（31 语言）或 SenseVoice。
- PC GPU 高质量：**Qwen3-ASR-1.7B + Qwen3-ForcedAligner-0.6B**（Apache-2.0，多语言/方言最广；时间戳要额外挂 Aligner，单次 5 分钟）。
- 中文精度上限/评测：**FireRedASR2-AED**（词级时间戳，单段 60s）；LLM 版精度更高但无时间戳、单段 40s。官方只测 Linux，端侧不优先。
- FireRed 的 CER 优势是厂商自测口径，必须用我们自己的中文长视频复测后再定默认。
- Android 只考虑 FunASR/ONNX 类运行时；Qwen3/FireRed 走 PC 本地 HTTP 服务，用 `--engine http` 接。

## 同类软件（2026-11 调研）

- auto-editor（Nim，Unlicense）：CLI，按响度切/加速，`-w:0 speed:8` 就是“静音段加速”的现成参照；它的 audio 方法也是单遍响度阈值（默认 0.04），说明阈值法足够做 MVP。
- jumpcutter（Python，MIT）：静音加速早期实现；逐帧导出，不适合长视频。
- TimeBolt / Recut / CapCut：商业去静音，以删除静音为主；本项目的差异是 GPUI 桌面 + 安卓原生长视频工作流。

## 命令

- **先 source 构建环境**：`source scripts/env.sh`（CMake + libclang + 本机 ffmpeg）。首次编译 `whisper-rs-sys` 必须要有这些。
- 本机没有全局 `ffmpeg`。测试用项目内 `tools/ffmpeg-9.0.2-full_build/bin/ffmpeg.exe`（已 gitignore）；其他机器设 `CAT_SPEEDUP_FFMPEG` 或把 `ffmpeg`/`ffprobe` 放进 PATH。
- 检查：`cargo run -p cat-speedup -- doctor`
- 单测：`cargo test --workspace`；静态检查：`cargo clippy --workspace --all-targets -- -D warnings`。注意 `default-members` 不含 `apps/desktop`，桌面单独 `cargo check -p cat-speedup-desktop`。
- 桌面：`cargo run -p cat-speedup-desktop`（第一次构建 GPUI 约 3 分钟；先 `source scripts/env.sh`）
- Android：`cd apps/android && JAVA_HOME=/d/006_Android/jdk-25 ANDROID_HOME=/d/006_Android/sdk ./gradlew :app:assembleDebug`（本机工具链固定 `D:\006_Android`：JDK 25 / Gradle 9.9.1 / AGP 9.5.1 / compileSdk 37；不要在项目里再放一套）。
- 端到端：`bash scripts/smoke.sh`（生成 6 秒合成视频，跑 analyze/render/prepare-asr/map-transcript）
- 分析：`cargo run -p cat-speedup -- analyze in.mp4 --speed 4 --margin 0.15 -o plan.json`
- 出片：`cargo run -p cat-speedup -- render in.mp4 out.mp4 --plan plan.json`
- ASR 预缩短：`cargo run -p cat-speedup -- prepare-asr in.mp4 asr.wav --map asr-map.json`
- 字幕（whisper）：`cargo run -p cat-speedup -- transcribe in.mp4 --model small --language zh -o transcript.json`（默认 auto -> CPU base / GPU small；中文建议 small）
- 字幕（FunASR 现成服务）：`cargo run -p cat-speedup -- transcribe in.mp4 --engine funasr --language zh -o transcript.json`（默认 `Fun-ASR-Nano-2512`，URL 默认 `http://127.0.0.1:8000/v1/audio/transcriptions`）
- HTTP ASR：`cargo run -p cat-speedup -- transcribe in.mp4 --engine http --asr-url <url> --asr-model <service-model> ...`
- 字幕回投：`cargo run -p cat-speedup -- map-transcript transcript.json asr-map.json -o transcript-original.json`
- GPU 构建：`cargo build -p cat-speedup --features gpu-vulkan`（需 Vulkan SDK；NVIDIA 也可 `gpu-cuda` + CUDA Toolkit）。CPU-only 时 `--device gpu` 会报错。
- 模型目录：默认 `models/`；可用 `CAT_SPEEDUP_MODEL_DIR` 改；`--model tiny|base|small|medium|large-v3` 或直接给 `.bin` 路径，缺失时自动从 hf-mirror 下载。
