# cut-speedup

Long-video auto editor: keep speech at 1x, speed up non-speech, and show a transcript for fast review. Desktop UI in Rust + GPUI, Android in native Kotlin.

长视频自动剪辑：按时间轴保留人声段原速，非人声段加速；桌面 GPUI，Android 原生 Kotlin。桌面端默认 **完全离线** 运行，FunASR 内置。

## 状态

- [x] 阈值检测（FFmpeg `silencedetect`）
- [x] 有序片段计划（人声 1x / 非人声 Nx）
- [x] 分段渲染 + concat
- [x] ASR 预缩短音频 + 原时间轴映射
- [x] **内置 FunASR**：sherpa-onnx + Fun-ASR-Nano-2512 int8，离线
- [x] GPUI 桌面壳：打开视频、分析、转写字幕、出片、字幕点击跳帧
- [x] **Web 端（`apps/web`，端口 42006）**：视频库浏览、原片流式播放、VAD+ASR / 响度阈值两种切分、字幕跳转、单次 filter_complex 导出
- [x] Android：Media3 播放 + ASR-first 切分 / 跟随加速 / 本地 X-ASR / 滑块调参 / 导出视频 / 导出 SRT
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

或设 `CUT_SPEEDUP_FFMPEG`。

## 桌面端（GPUI，原生窗口）

```bash
cargo run -p cut-speedup-desktop
```

界面流程：打开视频 -> 自动分析 -> 转写字幕 -> 点击字幕看对应画面 -> 出片。全程本地。

## Web 端（浏览器，端口 42006）

```bash
bash scripts/web.sh          # 等同于 cargo run --release -p cut-speedup-web -- --port 42006
```

打开 `http://192.168.0.105:42006`（本机 `http://127.0.0.1:42006`）。

- 左侧浏览 `~/` 下的视频（默认列出 `Videos`、`下载`、Windows `Videos`、`D:`）；
- 中间是原片 `<video>`（Range 流式，可拖动）+ 片段色条（蓝=人声 1x，绿=加速）；
- 右侧可调检测方式与参数：**VAD + ASR**（与手机同一套 Silero VAD + X-ASR，带字幕）或**响度阈值**（快、无字幕）；分析在后台任务里跑，页面显示进度；
- 字幕跟播高亮、点击跳转；导出用一遍 `filter_complex` 完成（trim/atempo/concat），进度按输出时间显示，完成后直接下载。

Linux 上要跑 VAD 模式需要 sherpa-onnx 的 Linux CLI：

```bash
bash scripts/fetch-sherpa-linux.sh
```

Windows 继续用 `tools/sherpa-onnx/...` 里的 `.exe`，可用 `CUT_SPEEDUP_VAD_ASR` 指定任意路径。

### 切分 bug：VAD 一次喂 30 秒会丢开头

Android 旧实现把整段 30 s 解码音频一次性 `Vad.acceptWaveform()`，而 sherpa-onnx 的环形缓冲只有 `maxSpeechDuration`（30 s）；溢出后开头音频被丢掉，长视频的前 ~30–36 s 会被判成静音并加速。PC 复现：

```bash
LD_LIBRARY_PATH=tools/sherpa-onnx-v2.0.1-linux-x64-shared-no-tts/lib \
python3 scripts/debug/vad_chunk_probe.py 视频.mp4 \
  --lib tools/sherpa-onnx-v2.0.1-linux-x64-shared-no-tts/lib
```

`chunk=30s` 时第一段人声从 29.7 s 才开始；`chunk<=0.1s` 时从 1.0 s 开始。修复：喂 VAD 时按 100 ms 切片（`VAD_FEED_SAMPLES=1600`），Android 与 Web 都遵守。

## CLI

```bash
# 检查本地 FFmpeg / sherpa / FunASR 模型
cargo run -p cut-speedup -- doctor

# 单测 / 静态检查
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

# 端到端（合成 6 秒视频）
bash scripts/smoke.sh

# 分析
cargo run -p cut-speedup -- analyze input.mp4 --speed 4 --noise -30dB --min-silence 0.5 -o plan.json

# 出片
cargo run -p cut-speedup -- render input.mp4 output.mp4 --plan plan.json

# 字幕（默认：本地 sherpa + Fun-ASR-Nano，离线）
cargo run -p cut-speedup -- transcribe input.mp4 --language zh -o transcript.json

# 显式指定 sherpa
cargo run -p cut-speedup -- transcribe input.mp4 --engine sherpa --language zh -o transcript.json
```

`--engine` 可选：

- `sherpa`（默认）：本地 FunASR，离线。
- `whisper`：可选后端，需要 `--features whisper` 且首次构建要 CMake + libclang。
- `funasr` / `http`：只用于内部调试，连 OpenAI 兼容服务；普通使用不需要。

## 模型

默认模型：`sherpa-onnx-funasr-nano-int8-2025-12-30`（约 972 MB，中文/英文/日文 + 中文方言）。

可通过环境变量覆盖：

```bash
export CUT_SPEEDUP_SHERPA_EXE=/path/to/sherpa-onnx-offline
export CUT_SPEEDUP_SHERPA_MODEL=/path/to/model-dir
```

低配机器可换 Paraformer：

```bash
cargo run -p cut-speedup -- transcribe input.mp4 --engine sherpa --sherpa-kind paraformer --sherpa-model-dir models/sherpa-onnx-paraformer-zh-int8-2025-10-07
```

## Android

模型**打进 APK**（X-ASR Zipformer + Silero VAD，约 168 MB），装完即可离线转写，不需要再往手机推模型：

```bash
# 一次性：拉 sherpa-onnx Android JNI + X-ASR + Silero VAD
bash scripts/fetch-sherpa-android.sh
bash scripts/fetch-asr-android-model.sh
# 把模型复制到 app/src/main/assets/models（gitignore，打包前必跑）
bash scripts/bundle-android-models.sh

cd apps/android
./gradlew :app:assembleDebug   # app/build/outputs/apk/debug/app-debug.apk（约 210 MB）
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

`scripts/push-asr-android.sh` 仍保留：把模型推到 `files/models/` 可以覆盖 APK 内置版本，方便不重新打包就换模型。

界面流程：加入视频 → 转写并切分（Silero VAD 定句头句尾，ASR 只跑人声段并缓存）→ 调参后“重新切分”（只重排，不重新转写）→ 导出视频 / 导出字幕。
默认：人声 1x、停顿 10x、停顿 ≥5 秒才加速、人声前后留白 150ms；参数持久化，“恢复默认”有确认提示。
切分后播放默认跟随片段加速（人声 1x，静音 Nx），底部字幕面板按原时间轴显示，点一条跳到原片位置。

导出视频用 Media3 Transformer 逐段重编码再拼接，参数沿用源分辨率/帧率，成片在 `Movies/CutSpeedup/`；
字幕导出标准 SRT，在 `Download/CutSpeedup/`。转写按人声段切片（30 s/块），长视频不会整片进内存。

日志：`adb logcat -s CutSpeedup`（切分参数、片段边界、ASR 分块、变速点、导出进度都在里面）。

## 目录

- `crates/core`：检测、片段计划、ffmpeg 命令、ASR 映射
- `crates/asr`：`AsrEngine` trait、sherpa-onnx 离线 FunASR、可选 whisper、调试用 HTTP
- `crates/cli`：`cut-speedup` 命令行
- `apps/desktop`：GPUI 桌面端
- `apps/web`：浏览器端（axum，端口 42006；视频库 / 播放 / 分析 / 导出）
- `apps/android`：Android 原生播放器壳（切分 / 本地转写 / 变速导出）
- `scripts/web.sh`：编译并启动 Web 端
- `scripts/fetch-asr.sh`：下载 sherpa-onnx + FunASR Nano
- `scripts/fetch-sherpa-linux.sh`：下载 Web 端 VAD 用的 Linux sherpa-onnx CLI
- `scripts/debug/vad_chunk_probe.py`：复现 VAD 大块喂入丢音频的 PC 脚本
- `scripts/fetch-sherpa-android.sh`：下载 sherpa-onnx Android JNI 库
- `scripts/push-asr-android.sh`：把 FunASR Nano 推到手机

## License

MIT。第三方依赖与模型许可证见 `THIRD_PARTY.md`。
