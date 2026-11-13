#!/usr/bin/env bash
# Copy the on-device ASR/VAD models into the Android app so the APK is
# self-contained (no adb push / side-load required).
#
# Models stay out of git (assets/models is gitignored); run this before
# building a distributable APK.
set -euo pipefail
cd "$(dirname "$0")/.."

XASR="models/sherpa-onnx-x-asr-zipformer-transducer-zh-en-int8-2026-06-03"
VAD="models/silero_vad.onnx"
DEST="apps/android/app/src/main/assets/models"

if [[ ! -d "$XASR" ]]; then
  echo "missing $XASR — run scripts/fetch-asr-android-model.sh first" >&2
  exit 1
fi
if [[ ! -f "$VAD" ]]; then
  echo "missing $VAD — run scripts/fetch-asr-android-model.sh first" >&2
  exit 1
fi

mkdir -p "$DEST/zipformer-zh-en"
cp -f "$XASR/tokens.txt" \
      "$XASR/encoder-epoch-99-avg-1.int8.onnx" \
      "$XASR/decoder-epoch-99-avg-1.onnx" \
      "$XASR/joiner-epoch-99-avg-1.int8.onnx" \
      "$DEST/zipformer-zh-en/"
cp -f "$VAD" "$DEST/silero_vad.onnx"

echo "bundled into $DEST:"
du -sh "$DEST"/* "$DEST"
