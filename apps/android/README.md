# Android

第一版 Android 是“视频 + 字幕面板”的播放器壳：

- 选择本地视频，用 Media3 播放。
- 导入 PC / CLI 生成的 `transcript.json`（原时间轴）。
- 字幕跟播高亮，点击字幕跳转视频。

它不做设备端切分/加速，先把“看字幕对应视频、快速编排”的移动端入口跑通；后续再接 Media3 Transformer 或共享 core。

## 工具链

用本机 `D:\006_Android`，不要在项目里再放一套：

```bash
export JAVA_HOME=/d/006_Android/jdk-25
export ANDROID_HOME=/d/006_Android/sdk
export PATH="$JAVA_HOME/bin:$ANDROID_HOME/platform-tools:$PATH"
```

`local.properties` 指向 `D:/006_Android/sdk`（已 gitignore）。

版本：JDK 25 / Gradle 9.9.1 / AGP 9.5.1 / compileSdk 37 / build-tools 37.0.0。

## 构建

```bash
cd apps/android
./gradlew :app:assembleDebug
# APK: app/build/outputs/apk/debug/app-debug.apk
```

wrapper 的 distribution URL 用腾讯镜像，避免 `services.gradle.org` 慢。

## 字幕 JSON

沿用 CLI 的原时间轴格式：

```json
{
  "segments": [
    {"start_ms": 1985, "end_ms": 5412, "text": "你好，这是一段测试语音。"}
  ]
}
```

## 下一步

- 连续播放 + 字幕同步已经具备；后续接设备端静音检测与变速导出。
- 若走共享 Rust core，需要 `cargo-ndk` + JNI/UniFFI，并验证 16 KB page size。
