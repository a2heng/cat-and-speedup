# Android

原生 Kotlin + Media3。界面只有一条主流程：**加入视频 → 切分 → 转写 → 导出视频 / 导出字幕**。

- 加入视频：SAF 选择，也支持从文件管理器 `VIEW` 打开（`video/*`）。
- 切分：设备端按响度阈值（等价 `silencedetect=noise=-30dB:d=0.5`）算有序片段计划。
  人声 1x、静音 Nx，参数在界面上直接改：静音倍速 / 判定阈值 dB / 最短静音 ms / 边界余量 ms。
- 播放：默认“跟随加速”，按片段自动切 1x / Nx；点速度胶囊切换跟随，暂停/进度条是自绘的紧凑控制条。
- 转写：设备端 sherpa-onnx + X-ASR Zipformer zh-en int8，先按计划只解码人声段（100–200 ms 余量），
  再按 30 s 分块送 ASR，结果落回原时间轴；失败不影响导出。
- 字幕面板：原时间轴时间戳胶囊 + 正文的行式卡片，跟播高亮，点一条直接跳到原片位置。
- 导出视频：Media3 Transformer 整条一次变速重编码（`SpeedProvider` 按源时间给速度），
  音画同步、保留源分辨率与帧率，成片写到 `Movies/CatSpeedup/`；长片耗时约等于编码时长。
- 导出字幕：标准 SRT（`HH:MM:SS,mmm`），写到 `Download/CatSpeedup/`。

## 工具链

先启用本机安卓工具链（本机是哪个 SDK/JDK 见全局 `AGENTS.md`，不要写进仓库）：

```bash
source <你的工具链>/env.sh   # 设置 JAVA_HOME / ANDROID_HOME / PATH
```

`local.properties` 指向本机 SDK（已 gitignore）。工程只依赖 JDK 21+、AGP 9.5.1、compileSdk 37。

## 构建 / 测试

```bash
cd apps/android
./gradlew :app:assembleDebug        # app/build/outputs/apk/debug/app-debug.apk
./gradlew :app:testDebugUnitTest    # Segment/Plan/SRT 单元测试
```

`app/src/main/jniLibs/` 来自 `scripts/fetch-sherpa-android.sh`，`app/src/main/assets/models/` 不打包模型，二者都 gitignore。

## ASR 模型（手机用轻量模型）

手机用 **X-ASR Zipformer zh-en int8**（约 168 MB，带 token 时间戳，字幕能按句跳转），
PC 才用 FunASR-Nano。模型不进 APK，手机上的查找顺序：

1. `<external files dir>/models/zipformer-zh-en`
2. `<internal files dir>/models/zipformer-zh-en`
3. assets（仅在手动打包时）

旧的 `models/paraformer-zh` / `models/funasr-nano` 仍可识别（Paraformer 在 sherpa 里
不返回时间戳，只能按块均分，已降级为 fallback）；推新模型时脚本会自动删掉它们。

```bash
bash scripts/fetch-asr-android-model.sh   # 下载到 models/
bash scripts/push-asr-android.sh          # 推到已连接手机
```

需要 64 位 ARM（`arm64-v8a`）。加载后常驻内存约 300–400 MB，比 FunASR-Nano 的 1.3 GB 低一个量级。

## 日志

统一 tag `CatSpeedup`：

```bash
adb logcat -s CatSpeedup
```

会打印切分参数与每段边界、静音检测结果、ASR 分块和耗时、每次变速点（含源时间）、导出进度与编码结果。
出错时带堆栈，先看这里再改。

无界面联调（adb）：

```bash
adb shell am start -a android.intent.action.VIEW -d "file:///sdcard/.../test.mp4" -t video/mp4 \
  --ez auto_analyze true --ez auto_transcribe true --ez auto_export true dev.a2heng.catspeedup
# 把当前布局画成 PNG，熄屏也能取（开发用）
adb shell am start ... --ez dump_ui true
```

## 已知限制

- 阈值法会把音乐/噪声当人声（MVP 误差，靠阈值、最短静音、余量调参）。
- 导出是整条变速重编码，长视频导出耗时接近一次编码时长（状态栏有百分比）。
- 字幕暂不烧进视频；导出视频与字幕是两个独立文件。
- 设备端不做 GPU：sherpa-onnx 固定 CPU。
