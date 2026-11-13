#!/usr/bin/env bash
# Download the light Android ASR model: X-ASR Zipformer zh-en int8.
#
# Phone does not run FunASR-Nano (972 MB, OOM on phones). This model is ~168 MB
# and still returns token timestamps, which the subtitle list needs.
# Desktop keeps FunASR-Nano via scripts/fetch-asr.sh.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NAME="sherpa-onnx-x-asr-zipformer-transducer-zh-en-int8-2026-06-03"
TARGET="$ROOT/models/$NAME"
# GitHub is often slow from CN; prefer the Hugging Face mirror.
HF_BASE="https://hf-mirror.com/csukuangfj2/$NAME/resolve/main"

mkdir -p "$TARGET"

fetch() {
  local file="$1"
  if [[ -s "$TARGET/$file" ]]; then
    echo "exists: $TARGET/$file"
    return
  fi
  echo "Downloading $file..."
  curl -L --fail --retry 3 --retry-delay 2 \
    -o "$TARGET/$file" "$HF_BASE/$file"
}

fetch encoder-epoch-99-avg-1.int8.onnx
fetch decoder-epoch-99-avg-1.onnx
fetch joiner-epoch-99-avg-1.int8.onnx
fetch tokens.txt

ls -lh "$TARGET"
echo "model: $TARGET"
