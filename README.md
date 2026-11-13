# cat-and-speedup

Long-video auto editor: keep speech at 1x, speed up non-speech, and show a transcript for fast review. Desktop UI in Rust + GPUI, Android planned in native Kotlin/Compose.

长视频自动剪辑：按时间轴保留人声段原速，非人声段加速；桌面 GPUI，Android 原生 Kotlin。

## 第一版范围（PC）

- [x] 阈值检测（FFmpeg `silencedetect`）
- [x] 有序片段计划（人声 1x / 非人声 Nx）
- [x] 分段渲染 + concat
- [x] ASR 预缩短音频 + 原时间轴映射
- [x] 内置 whisper.cpp ASR（CPU；GPU 通过 feature 编译）
- [x] 通用 HTTP ASR 后端（OpenAI 兼容接口，默认接 Fun-ASR-Nano）
- [x] GPUI 桌面壳：打开视频、分析、转写字幕、出片、字幕点击跳帧
- [x] Android 播放器壳：Media3 视频 + 导入原时间轴字幕 JSON（APK 已构建，见 `apps/android/README.md`）
- [ ] 连续视频播放（第一版是“帧预览 + 字幕跳帧”，不是完整播放器）
- [ ] Android 设备端切分/加速

第一版先保证 **PC 能完成“看字幕 -> 编排 -> 出片”的闭环**；Android 与连续播放放到下一版，避免把未验证的端侧链路混进首个开源版本。

## 环境

需要 PATH 里有 `ffmpeg` / `ffprobe`，或用 `CAT_SPEEDUP_FFMPEG` 指向 ffmpeg 可执行文件。本机测试用的是项目内 `tools/ffmpeg-9.0.2-full_build/bin/ffmpeg.exe`（已 gitignore）。

whisper.cpp 通过 `whisper-rs` 静态编译，首次构建需要 CMake + libclang：

```bash
source scripts/env.sh   # 本机：VS BuildTools 的 cmake + tools/libclang
```

FunASR 用现成的 OpenAI 兼容服务，默认地址 `http://127.0.0.1:8000/v1/audio/transcriptions`，默认模型 `FunAudioLLM/Fun-ASR-Nano-2512`。可用环境变量改：

```bash
export CAT_SPEEDUP_ASR_URL=http://127.0.0.1:8000/v1/audio/transcriptions
export CAT_SPEEDUP_ASR_MODEL=FunAudioLLM/Fun-ASR-Nano-2512
```

## 桌面端

```bash
source scripts/env.sh
cargo run -p cat-speedup-desktop
```

界面流程：打开视频 -> 自动分析 -> 转写字幕 -> 点击字幕看对应画面 -> 出片。

## CLI

```bash
# 检查
cargo run -p cat-speedup -- doctor

# 单测 / 静态检查
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

# 端到端（合成 6 秒视频，跑完整链路）
bash scripts/smoke.sh

# 分析：输出有序片段计划
cargo run -p cat-speedup -- analyze input.mp4 --speed 4 --noise -30dB --min-silence 0.5 -o plan.json

# 出片
cargo run -p cat-speedup -- render input.mp4 output.mp4 --plan plan.json

# 给 ASR 准备语音段和映射
cargo run -p cat-speedup -- prepare-asr input.mp4 asr.wav --map asr-map.json

# 字幕：自动裁语音段 -> FunASR 服务 -> 映射回原时间轴
cargo run -p cat-speedup -- transcribe input.mp4 --engine funasr --language zh -o transcript.json

# 字幕：内置 whisper（离线，中文建议 small）
cargo run -p cat-speedup -- transcribe input.mp4 --model small --language zh -o transcript.json

# 或接任意 OpenAI 兼容服务
cargo run -p cat-speedup -- transcribe input.mp4 \
  --engine http \
  --asr-url http://127.0.0.1:8000/v1/audio/transcriptions \
  --asr-model <service-model> \
  -o transcript.json

# 把 ASR（短音频时间轴）的结果映射回原片时间轴
cargo run -p cat-speedup -- map-transcript transcript.json asr-map.json -o transcript-original.json
```

GPU（可选，需自备 Vulkan SDK 或 CUDA Toolkit）：

```bash
cargo build -p cat-speedup --features gpu-vulkan   # 或 gpu-cuda
```

`--device auto` 在带 GPU 后端的构建里优先 GPU，失败回退 CPU；CPU-only 构建会直接走 CPU。

## Android

```bash
cd apps/android
JAVA_HOME=/d/006_Android/jdk-25 ANDROID_HOME=/d/006_Android/sdk ./gradlew :app:assembleDebug
# app/build/outputs/apk/debug/app-debug.apk
```

第一版是播放器壳：选择视频、导入 PC 生成的 `transcript.json`、字幕跟播并点击跳转。设备端切分/加速还没接。

## 目录

- `crates/core`：检测、片段计划、ffmpeg 命令、ASR 映射
- `crates/asr`：`AsrEngine` trait、whisper.cpp、OpenAI 兼容 HTTP 后端
- `crates/cli`：`cat-speedup` 命令行
- `apps/desktop`：GPUI 桌面端
- `apps/android`：Android 播放器壳（Media3 + 字幕 JSON）
- `scripts/smoke.sh`：合成视频端到端验证

## 设计约定

- 片段严格按原时间轴顺序，不做重排。
- 检测只用响度/静音阈值，不引入模型。
- ASR 只用于字幕预览和快速编排，默认不烧字幕。
- 详细约束见 `AGENTS.md`。

## License

MIT。第三方依赖与运行时许可证见 `THIRD_PARTY.md`。
