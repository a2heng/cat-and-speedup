# Android

原生 Kotlin + Media3。主流程：**加入视频 → 转写并切分（只转写一次）→ 调参后“重新切分” → 导出视频 / 导出字幕**。

- 加入视频：SAF 选择，也支持从文件管理器 `VIEW` 打开（`video/*`）。
- 转写并切分：**ASR-first**。第一次按这个按钮时解码一遍音频、按 30 s 分块转写，用 token
  时间戳得到人声段；只有停顿 ≥ “最短加速间隙”（默认 5 秒）才加速，短停顿保持 1x。
  之后按钮变成“重新切分”，同一视频再按只重排计划，**不重新解码、不重新转写**。
  人声 1x、停顿 10x（默认），不会因为响度把小声说话误判成静音。
- 调参：跳过静音速度 / 最短加速间隙 / 人声前后留白，全部是滑块；改完立即重排计划，
  **不重新解码、不重新转写**。参数保存在 SharedPreferences，重启后还在；“恢复默认参数”
  会先弹确认，提示当前调整会丢失。
- 转写缓存：每支视频一份 transcript（内部存储，按 URI 哈希），重开同一个视频直接复用。
- 播放：默认“跟随加速”，按片段自动切 1x / Nx；点速度胶囊切换跟随。播放按钮在视频右上，
  下方整条进度条按计划着色：蓝色=人声、绿色=加速段，可拖动跳转。
- 字幕面板：原时间轴时间戳胶囊 + 正文的行式卡片，跟播高亮，点一条直接跳到原片位置；
  当前字幕固定两行高，超出自动缩小字号，不会把布局顶来顶去。
- 导出视频：Media3 Transformer 整条一次变速重编码（`SpeedProvider` 按源时间给速度），
  音画同步、保留源分辨率与帧率、变速不变调（Sonic），成片写到 `Movies/CatSpeedup/`。
- 导出字幕：标准 SRT（`HH:MM:SS,mmm`），写到 `Download/CatSpeedup/`。
- 没有模型时会退回响度阈值（`-30dB / 0.5s`）兜底，但界面上不再暴露这些参数。

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
