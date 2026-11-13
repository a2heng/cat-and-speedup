# Third-party notices

This project is MIT-licensed. It uses or interoperates with the following
third-party software; their licenses apply to their own code and binaries.

- **GPUI** (Zed Industries) — Apache-2.0. Desktop UI framework.
- **whisper-rs / whisper.cpp** — Unlicense / MIT. Built-in ASR backend.
- **FunASR** — MIT (code). The default runtime is sherpa-onnx; the
  **Fun-ASR-Nano-2512** weights are governed by the FunASR Model Open Source
  License Agreement v1.1 and are **not** bundled in this repository.
- **sherpa-onnx** — Apache-2.0. Local offline ASR runtime (Windows CLI and
  Android Kotlin API).
- **Fun-ASR-Nano-2512** — model weights are not bundled. Check the model card
  and the FunASR model license before redistribution.
- **X-ASR Zipformer zh-en int8** — Android default ASR weights (derived from
  https://github.com/Gilgamesh-J/X-ASR). Not bundled; check the upstream model
  license before redistribution.
- **AndroidX Media3** (ExoPlayer / Transformer / UI) — Apache-2.0. Android
  playback and speed-changing export.
- **JUnit 4** — Eclipse Public License 1.0. Android unit tests only.
- **FFmpeg** — LGPL-2.1-or-later by default; some builds are GPL. This project
  invokes a user-provided `ffmpeg`/`ffprobe` binary and does not bundle it.
- **auto-editor** — Unlicense. Research reference for the silence-speed-up
  workflow; no source code is copied into this repository.
- Rust crates: see `Cargo.lock` for versions and each crate's license.

No model weights or FFmpeg binaries are committed to this repository.
