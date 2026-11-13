#!/usr/bin/env bash
# Push the Android ASR model to a connected device.
#
# The phone uses the light X-ASR Zipformer zh-en int8 model (~168 MB) instead
# of the 972 MB FunASR-Nano that the PC uses. It goes to the app's external
# files dir, where OfflineAsr looks first:
#   /sdcard/Android/data/dev.a2heng.catspeedup/files/models/zipformer-zh-en
#
# Usage: bash scripts/push-asr-android.sh [serial]
set -euo pipefail

# Keep Git Bash/MSYS from rewriting /sdcard/... into a Windows path.
export MSYS_NO_PATHCONV=1

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PKG="dev.a2heng.catspeedup"
MODEL="$ROOT/models/sherpa-onnx-x-asr-zipformer-transducer-zh-en-int8-2026-06-03"
ROOT_DEST="/sdcard/Android/data/$PKG/files/models"
DEST="$ROOT_DEST/zipformer-zh-en"
ADB=(adb)
if [[ -n "${1:-}" ]]; then
  ADB=(adb -s "$1")
fi

# adb.exe is a native Windows binary, so it needs a Windows source path.
if command -v cygpath >/dev/null 2>&1; then
  WINMODEL="$(cygpath -w "$MODEL")"
else
  WINMODEL="$MODEL"
fi

if [[ ! -f "$MODEL/encoder-epoch-99-avg-1.int8.onnx" ]]; then
  echo "缺少模型：$MODEL" >&2
  echo "先运行 bash scripts/fetch-asr-android-model.sh" >&2
  exit 1
fi

"${ADB[@]}" shell "mkdir -p '$DEST' '$ROOT_DEST'"
for file in \
  encoder-epoch-99-avg-1.int8.onnx \
  decoder-epoch-99-avg-1.onnx \
  joiner-epoch-99-avg-1.int8.onnx \
  tokens.txt \
  bpe.model; do
  [[ -f "$MODEL/$file" ]] || continue
  "${ADB[@]}" push "$WINMODEL/$file" "$DEST/"
done

# Silero VAD lives next to the model folders.
if [[ -f "$ROOT/models/silero_vad.onnx" ]]; then
  if command -v cygpath >/dev/null 2>&1; then
    WINVAD="$(cygpath -w "$ROOT/models/silero_vad.onnx")"
  else
    WINVAD="$ROOT/models/silero_vad.onnx"
  fi
  "${ADB[@]}" push "$WINVAD" "$ROOT_DEST/silero_vad.onnx"
fi

# Keep only one model on the phone: old Paraformer / FunASR-Nano are unused.
for old in paraformer-zh funasr-nano; do
  if "${ADB[@]}" shell "[ -d '$ROOT_DEST/$old' ]" 2>/dev/null; then
    echo "删除手机上旧的 $old 模型"
    "${ADB[@]}" shell "rm -rf '$ROOT_DEST/$old'"
  fi
done

"${ADB[@]}" shell "ls -l '$DEST'"
