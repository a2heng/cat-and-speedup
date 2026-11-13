# Third-party notices

This project is MIT-licensed. It uses or interoperates with the following
third-party software; their licenses apply to their own code and binaries.

- **GPUI** (Zed Industries) — Apache-2.0. Desktop UI framework.
- **whisper-rs / whisper.cpp** — Unlicense / MIT. Built-in ASR backend.
- **FunASR** — MIT (code). Used as an external OpenAI-compatible service;
  model weights are distributed by their respective model cards.
- **Fun-ASR-Nano-2512** — model weights are not bundled. Check the model card
  before redistribution.
- **FFmpeg** — LGPL-2.1-or-later by default; some builds are GPL. This project
  invokes a user-provided `ffmpeg`/`ffprobe` binary and does not bundle it.
- **auto-editor** — Unlicense. Research reference for the silence-speed-up
  workflow; no source code is copied into this repository.
- Rust crates: see `Cargo.lock` for versions and each crate's license.

No model weights or FFmpeg binaries are committed to this repository.
