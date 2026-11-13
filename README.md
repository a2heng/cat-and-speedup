# cat-and-speedup

Long-video auto editor: keep speech at 1x, speed up non-speech, and show a transcript for fast review. Desktop UI in Rust + GPUI, Android in native Kotlin.

长视频自动剪辑：按时间轴保留人声段原速，非人声段加速；桌面 GPUI，Android 原生 Kotlin。桌面端默认 **完全离线** 运行，FunASR 内置。

## 状态

- [x] 阈值检测（FFmpeg `silencedetect`）
- [x] 有序片段计划（人声 1x / 非人声 Nx）
- [x] 分段渲染 + concat
- [x] ASR 预缩短音频 + 原时间轴映射
- [x] **内置 FunASR**：sherpa-onnx + Fun-ASR-Nano-2512 int8，离线
- [x] GPUI 桌面壳：打开视频、分析、转写字幕、出片、字幕点击跳帧
- [x] Android：Media3 播放 + 设备端切分 / 跟随加速 / 本地 FunASR / 导出视频 / 导出 SRT
- [ ] 桌面连续播放（当前是帧预览 + 字幕跳帧）
- [ ] Android 转写进度可视化（当前只有状态栏文字 + logcat）

## 环境

桌面默认只用两样本地资源，不需要浏览器、不需要 `127.0.0.1` 服务：

```text
tools/sherpa-onnx/.../bin/sherpa-onnx-offline.exe
models/sherpa-onnx-funasr-nano-int8-2025-12-30/
```

它们不进 git；拉取命令：

```bash
bash scripts/fetch-asr.sh
```

还需要 FFmpeg。PATH 里没有时，项目会自动找：

```text
tools/ffmpeg-9.0.2-full_build/bin/ffmpeg.exe
```

或设 `CAT_SPEEDUP_FFMPEG`。

## 桌面端（GPUI，原生窗口）

```bash
cargo run -p cat-speedup-desktop
```

界面流程：打开视频 -> 自动分析 -> 转写字幕 -> 点击字幕看对应画面 -> 出片。全程本地。

## CLI

```bash
# 检查本地 FFmpeg / sherpa / FunASR 模型
cargo run -p cat-speedup -- doctor

# 单测 / 静态检查
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

# 端到端（合成 6 秒视频）
bash scripts/smoke.sh

# 分析
cargo run -p cat-speedup -- analyze input.mp4 --speed 4 --noise -30dB --min-silence 0.5 -o plan.json

# 出片
cargo run -p cat-speedup -- render input.mp4 output.mp4 --plan plan.json

# 字幕（默认：本地 sherpa + Fun-ASR-Nano，离线）
cargo run -p cat-speedup -- transcribe input.mp4 --language zh -o transcript.json

# 显式指定 sherpa
cargo run -p cat-speedup -- transcribe input.mp4 --engine sherpa --language zh -o transcript.json
```

`--engine` 可选：

- `sherpa`（默认）：本地 FunASR，离线。
- `whisper`：可选后端，需要 `--features whisper` 且首次构建要 CMake + libclang。
- `funasr` / `http`：只用于内部调试，连 OpenAI 兼容服务；普通使用不需要。

## 模型

默认模型：`sherpa-onnx-funasr-nano-int8-2025-12-30`（约 972 MB，中文/英文/日文 + 中文方言）。

可通过环境变量覆盖：

```bash
export CAT_SPEEDUP_SHERPA_EXE=/path/to/sherpa-onnx-offline
export CAT_SPEEDUP_SHERPA_MODEL=/path/to/model-dir
```

低配机器可换 Paraformer：

```bash
cargo run -p cat-speedup -- transcribe input.mp4 --engine sherpa --sherpa-kind paraformer --sherpa-model-dir models/sherpa-onnx-paraformer-zh-int8-2025-10-07
```

## Android

```bash
# 一次性：拉 sherpa-onnx Android JNI + 轻量 ASR 模型，再推到手机
bash scripts/fetch-sherpa-android.sh
bash scripts/fetch-asr-android-model.sh
bash scripts/push-asr-android.sh

cd apps/android
source /d/AndroidDevToolchain/env.sh   # 本机工具链，见全局 AGENTS.md
./gradlew :app:assembleDebug   # app/build/outputs/apk/debug/app-debug.apk
```

界面流程：加入视频 → 切分（响度阈值，可调倍速/阈值/最短静音/余量）→ 转写（设备端 X-ASR Zipformer）→ 导出视频 / 导出字幕。
切分后播放默认跟随片段加速（人声 1x，静音 Nx），底部字幕面板按原时间轴显示，点一条跳到原片位置。

导出视频用 Media3 Transformer 逐段重编码再拼接，参数沿用源分辨率/帧率，成片在 `Movies/CatSpeedup/`；
字幕导出标准 SRT，在 `Download/CatSpeedup/`。转写按人声段切片（30 s/块），长视频不会整片进内存。

日志：`adb logcat -s CatSpeedup`（切分参数、片段边界、ASR 分块、变速点、导出进度都在里面）。

## 目录

- `crates/core`：检测、片段计划、ffmpeg 命令、ASR 映射
- `crates/asr`：`AsrEngine` trait、sherpa-onnx 离线 FunASR、可选 whisper、调试用 HTTP
- `crates/cli`：`cat-speedup` 命令行
- `apps/desktop`：GPUI 桌面端
- `apps/android`：Android 原生播放器壳（切分 / 本地转写 / 变速导出）
- `scripts/fetch-asr.sh`：下载 sherpa-onnx + FunASR Nano
- `scripts/fetch-sherpa-android.sh`：下载 sherpa-onnx Android JNI 库
- `scripts/push-asr-android.sh`：把 FunASR Nano 推到手机

## License

MIT。第三方依赖与模型许可证见 `THIRD_PARTY.md`。
