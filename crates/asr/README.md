# cut-speedup-asr

ASR 后端抽象。当前只有 whisper.cpp（`whisper` feature，默认开启）。

## 接口

```rust
pub trait AsrEngine {
    fn name(&self) -> &str;
    fn model(&self) -> &Path;
    fn transcribe_wav(
        &mut self,
        path: &Path,
        options: &AsrOptions,
    ) -> Result<AsrResult, AsrError>;
}
```

- 输入必须是 16 kHz / 16-bit / 单声道 WAV；`prepare-asr` 已按这个格式输出。
- 输出 `Transcript` 使用**紧凑音频时间轴**，再由 `cut-speedup map-transcript` 映射回原片。
- `AsrOptions.device`：`Auto` / `Cpu` / `Gpu`。只有编译时启用 `gpu-vulkan` 或 `gpu-cuda`，`Auto` 才可能真正走 GPU。
- 新后端只需实现 trait，然后在 CLI 的 `--engine` 分支里注册；不需要改 core。

## 后端

- `whisper`：内置 whisper.cpp，CPU 默认，GPU 走 feature。
- `http`：任意 OpenAI 兼容 `/v1/audio/transcriptions` 服务；适合把 FunASR / Qwen3-ASR / FireRedASR2 跑成本地服务后接进来。

HTTP 后端请求 `response_format=verbose_json` 和 `timestamp_granularities[]=segment`；服务返回 `segments` 时直接用它的时间戳，只返回 `text` 时按 WAV 时长回退成单段。

## 构建

`whisper-rs-sys` 首次构建需要：

- CMake（本机用 VS BuildTools 自带的）
- libclang（`LIBCLANG_PATH`）

```bash
source scripts/env.sh
cargo build -p cut-speedup
```

GPU（可选）：

```bash
cargo build -p cut-speedup --features gpu-vulkan   # 需要 Vulkan SDK
cargo build -p cut-speedup --features gpu-cuda     # 需要 CUDA Toolkit
```

## 模型

默认从 `models/` 读取 `ggml-{size}.bin`，缺失时从 hf-mirror 下载（失败再试 huggingface.co）。中文建议 `small`；`base`/`tiny` 更快但错字更多。

`CUT_SPEEDUP_MODEL_DIR` 可改模型目录；`CUT_SPEEDUP_MODEL` 可设默认模型。
