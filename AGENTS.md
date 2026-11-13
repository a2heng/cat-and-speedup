# cut-speedup

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
- `crates/asr`：`AsrEngine` trait。默认 **sherpa-onnx + Fun-ASR-Nano-2512 int8**，完全离线；`whisper` 是可选 feature；OpenAI 兼容 HTTP 只用于内部调试。运行时在 `tools/sherpa-onnx`、模型在 `models/sherpa-onnx-funasr-nano-int8-2025-12-30`（均 gitignore）。
- `crates/cli`：`cut-speedup` 命令（doctor/analyze/render/prepare-asr/transcribe/map-transcript）。桌面和 Android 都复用 core。
- `apps/desktop`：GPUI 桌面壳；文件选择、分析、FunASR 转写、字幕点击跳帧、出片。当前是帧预览，不是连续播放器。
- `apps/web`：axum Web 端（固定端口 42006）。视频库浏览 + HTTP Range 播放；VAD+ASR（Linux 走 `sherpa-onnx-vad-with-offline-asr` CLI，与手机同模型）或响度阈值两种分析；字幕跳转；导出用**单遍 `filter_complex`**（trim/atempo/concat 一次编码），任务进度按 ffmpeg `-progress` 的 `out_time_us` 算。
- `apps/android`：原生 Kotlin（XML + AppCompat，不是 Compose）播放器壳，Media3 播放；设备端 **ASR-first**：转写和切分合并为一个按钮（首次“转写并切分”，之后“重新切分”），转写一次并缓存 transcript，用 token 时间戳生成有序片段计划；参数是滑块（静音速度（2–8x）/ 最短加速间隙 / 留白），改参数只重排计划；Media3 Transformer 变速导出视频，导出 SRT。不接入 Cargo workspace。
- 桌面和 Android 首屏都要有“视频 + 字幕面板”：字幕跟播高亮、点击跳转，作为快速看内容和编排的主界面；成片渲染放在其后。
- 桌面 v1 已落地（帧预览 + 字幕跳帧）；Android 端已落地切分/转写/加速播放/导出，桌面连续播放待做。
- 桌面依赖 `gpui` 0.2.2（crates.io）；Android 用标准 Android Studio 工程，别混入 GPUI。
- 两端共用同一套语义：有序 `Segment`、人声 1x、整数毫秒、音画同步；Android 不跑 ffmpeg，导出走 Media3 Transformer（整条一次变速重编码，`SpeedProvider` 按源时间给速度）。
- Android 长视频禁止整片进内存：解码流式，ASR 按 30 s 分块；音频多声道按“最大幅值”混单声道，避免相位抵消把语音判成静音。
- Android 的切分信源是 **VAD + ASR**：Silero VAD 给出精确句头/句尾，ASR 只在 VAD 人声段上跑并生成字幕；没有 VAD 模型才退回 token 时间戳。默认参数：人声 1x、停顿 8x、停顿 ≥5 s 才加速、人声前后留白 150 ms；参数持久化，“恢复默认”必须弹确认。
- Android 转写结果按视频 URI 缓存（内部 `files/transcripts/<sha1>.json`），调参、重启都不再转写。
- Android 日志统一 tag `CutSpeedup`（`adb logcat -s CutSpeedup`）：切分参数与片段、ASR 分块耗时、每次变速点、导出进度/编码结果都打印。

## 必须避开的坑

- 变速必须重编码：视频 `setpts=PTS/<speed>`，音频 `atempo=<speed>`；旧版 atempo 上限 2.0，>2x 需串联。
- 不要对源片段 `-c copy` 后直接 concat：切点不在关键帧会花屏/音画不同步；统一重编码后再 concat，或整条 `filter_complex` 一次出片。
- 手机源常是 VFR；拼接前统一 CFR（`-fps_mode cfr`），否则长片会漂移。
- 长视频禁止抽全帧或全片进内存（jumpcutter 的磁盘炸弹）；按块处理，临时文件放 cache 并清理。
- 片段边界留 100–200ms 余量防吞字；时间用整数毫秒，禁止浮点累加；无音轨按全片无人声处理。
- 默认构建**不需要** CMake/libclang；只有显式启用 `--features whisper` 才需要 `source scripts/env.sh`。不要在 Windows 上设 `WHISPER_DONT_GENERATE_BINDINGS=1`，仓库自带 bindings 是 Linux 结构体尺寸，会直接编不过。
- `--device auto` 只对 whisper 的 GPU feature 有意义；默认 sherpa 是 CPU。CPU-only 构建必须如实显示 CPU，不能因为 `use_gpu(true)` 成功就报 GPU。
- Android：ffmpeg-kit 2025-01 已退役，用维护中的 fork 或 MediaCodec/Media3；native .so 必须支持 16KB page size（2025-11-01 起 Play 要求）；SAF `content://` 不能直接喂 ffmpeg，先复制到 cache。
- **喂 Silero VAD 必须小于 1 秒切片**：sherpa-onnx 环形缓冲只有 `maxSpeechDuration × 16k`（默认 30 s）。整段 30 s 一次 `acceptWaveform` 会溢出并丢掉开头，表现就是长视频前 30–36 s 被判成静音并加速（2026-11-13 在 `record-33min.mp4` 上复现：30 s 一块时第一段人声从 29.72 s 才开始，0.1 s 一块时从 1.02 s 开始）。Android 用 `VAD_FEED_SAMPLES=1600`（100 ms）；PC 复现 `scripts/debug/vad_chunk_probe.py`。
- Web 导出**不要**沿用 CLI 的“逐段 ffmpeg 再 concat”（长视频几百段会很慢、临时文件多）；用一片 `filter_complex_script` 一次 trim/atempo/concat，`-progress pipe:1` 读 `out_time_us`。

## ASR 路线（2026-11）

- **PC 用好的，手机用轻的**：桌面默认 sherpa-onnx + **Fun-ASR-Nano-2512 int8**（约 972 MB，中文/英/日 + 7 方言/26 口音），走 `sherpa-onnx-offline`；Android 默认 **X-ASR Zipformer zh-en int8**（约 168 MB，**带 token 时间戳**），走同一套 sherpa-onnx Kotlin API。两端的 ASR 结果都落回原时间轴。
- Android 上的 Paraformer-zh int8（约 218 MB）在 sherpa-onnx v2.0.1 里 `timestamps` 为空，字幕只能按块均分，已降级为 fallback；`models/funasr-nano` 仍可识别但默认不用（972 MB 机型容易 OOM），推新模型时脚本会清掉旧目录。
- 可选高质量：Qwen3-ASR-1.7B + ForcedAligner（PC GPU）；FireRedASR2-AED（中文精度上限，PC 评测）。
- Qwen3/FireRed 走 PC 本地 HTTP 仅用于内部调试，不作为产品界面，也不进 Android。
- FireRed 的 CER 优势是厂商自测口径，必须用我们自己的中文长视频复测后再定默认。

## 同类软件（2026-11 调研）

- auto-editor（Nim，Unlicense）：CLI，按响度切/加速，`-w:0 speed:8` 就是“静音段加速”的现成参照；它的 audio 方法也是单遍响度阈值（默认 0.04），说明阈值法足够做 MVP。
- jumpcutter（Python，MIT）：静音加速早期实现；逐帧导出，不适合长视频。
- TimeBolt / Recut / CapCut：商业去静音，以删除静音为主；本项目的差异是 GPUI 桌面 + 安卓原生长视频工作流。

## 命令

- 默认构建不需要 CMake/libclang；只有 `--features whisper` 或 GPU whisper 才需要 `source scripts/env.sh`。
- Windows 没全局 `ffmpeg` 时，代码优先找 `tools/ffmpeg-9.0.2-full_build/bin/ffmpeg.exe`；Linux/macOS 走 PATH（本机 `/usr/bin/ffmpeg`）。任何平台都可用 `CUT_SPEEDUP_FFMPEG` 覆盖。
- Web：`bash scripts/web.sh`（或 `cargo run -p cut-speedup-web -- --port 42006`），浏览器开 `http://192.168.0.105:42006`。VAD 模式在 Linux 需要 `bash scripts/fetch-sherpa-linux.sh`（Windows 用 `tools/sherpa-onnx/...` 的 `.exe`）。
- Web 单测：`cargo test -p cut-speedup-web`。桌面不在 default-members，单独 `cargo check -p cut-speedup-desktop`。
- 下载本地 ASR 运行时 + 模型：`bash scripts/fetch-asr.sh`（sherpa-onnx Windows + Fun-ASR-Nano int8，约 1 GB，gitignore）。
- 检查：`cargo run -p cut-speedup -- doctor`（应显示 sherpa 路径和 funasr 模型目录）
- 单测：`cargo test`（default-members，不含 desktop 与 web）；`cargo test -p cut-speedup-web` 跑 Web。`--workspace` 会拉 GPUI 的 `xattr`，Linux 上当前编不过；clippy 同理，只查核心与 Web。
- 桌面：`cargo run -p cut-speedup-desktop`（GPUI 原生窗口；默认离线，不需要 URL；第一次构建 GPUI 约 3 分钟）
- Android：`cd apps/android && ./gradlew :app:assembleDebug`（先启用本机工具链，见全局 AGENTS.md / `env.sh`）。
- Android 单测：`cd apps/android && ./gradlew :app:testDebugUnitTest`。
- Android 资源：`bash scripts/fetch-sherpa-android.sh`（JNI 库，约 45 MB）；`bash scripts/fetch-asr-android-model.sh` + `bash scripts/push-asr-android.sh`（X-ASR 模型 + Silero VAD 推到手机）。
- Android 算法调试（PC）：`python scripts/debug/asr_plan_debug.py 视频.mp4 --gap 1 --margin 150 --speed 8`，走同一份 VAD+ASR 模型并输出切段对比。
- VAD 大块喂入丢音频复现（PC）：`LD_LIBRARY_PATH=tools/sherpa-onnx-v2.0.1-linux-x64-shared-no-tts/lib python3 scripts/debug/vad_chunk_probe.py 视频.mp4 --lib tools/sherpa-onnx-v2.0.1-linux-x64-shared-no-tts/lib`（30 s 块 → 第一段人声从 ~29.7 s 开始；0.1 s 块 → ~1.0 s）。
- Android 日志：`adb logcat -s CutSpeedup`。
- 端到端：`bash scripts/smoke.sh`（生成 6 秒合成视频，跑 analyze/render/prepare-asr/map-transcript）
- 分析：`cargo run -p cut-speedup -- analyze in.mp4 --speed 4 --margin 0.15 -o plan.json`
- 出片：`cargo run -p cut-speedup -- render in.mp4 out.mp4 --plan plan.json`
- ASR 预缩短：`cargo run -p cut-speedup -- prepare-asr in.mp4 asr.wav --map asr-map.json`
- 字幕（默认离线）：`cargo run -p cut-speedup -- transcribe in.mp4 --language zh -o transcript.json`（sherpa + Fun-ASR-Nano）
- 字幕（低配）：`--engine sherpa --sherpa-kind paraformer --sherpa-model-dir models/sherpa-onnx-paraformer-zh-int8-2025-10-07`
- 字幕（whisper，可选）：`cargo run -p cut-speedup --features whisper -- transcribe in.mp4 --engine whisper --model small --language zh -o transcript.json`
- HTTP ASR（仅调试）：`--engine http --asr-url <url> --asr-model <service-model>`
- 字幕回投：`cargo run -p cut-speedup -- map-transcript transcript.json asr-map.json -o transcript-original.json`
