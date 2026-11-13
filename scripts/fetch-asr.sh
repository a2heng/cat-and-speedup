#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SHERPA_DIR="$ROOT/tools/sherpa-onnx"
SHERPA_ZIP="$ROOT/tools/sherpa-onnx-win.tar.bz2"
MODEL_DIR="$ROOT/models/sherpa-onnx-funasr-nano-int8-2025-12-30"
MODEL_ARCHIVE="$ROOT/tools/sherpa-funasr-nano.tar.bz2"

mkdir -p "$ROOT/tools" "$ROOT/models"

if [[ ! -x "$SHERPA_DIR/sherpa-onnx-v2.0.1-win-x64-shared-MD-Release-no-tts/bin/sherpa-onnx-offline.exe" ]]; then
  echo "Downloading sherpa-onnx Windows runtime..."
  curl -L --fail --retry 3 \
    -o "$SHERPA_ZIP" \
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/v2.0.1/sherpa-onnx-v2.0.1-win-x64-shared-MD-Release-no-tts.tar.bz2"
  mkdir -p "$SHERPA_DIR"
  tar -xjf "$SHERPA_ZIP" -C "$SHERPA_DIR"
  rm -f "$SHERPA_ZIP"
fi

if [[ ! -f "$MODEL_DIR/llm.int8.onnx" ]]; then
  echo "Downloading Fun-ASR-Nano-2512 int8 model (~972 MB)..."
  curl -L --fail --retry 3 \
    -o "$MODEL_ARCHIVE" \
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-funasr-nano-int8-2025-12-30.tar.bz2"
  tar -xjf "$MODEL_ARCHIVE" -C "$ROOT/models"
  rm -f "$MODEL_ARCHIVE"
fi

echo "sherpa: $SHERPA_DIR/sherpa-onnx-v2.0.1-win-x64-shared-MD-Release-no-tts/bin/sherpa-onnx-offline.exe"
echo "model:  $MODEL_DIR"
