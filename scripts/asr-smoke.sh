#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/env.sh"

INPUT="${1:-$ROOT/out/asr-test/input.mp4}"
OUT_DIR="$(dirname "$INPUT")"

if [[ ! -f "$INPUT" ]]; then
  echo "input not found: $INPUT" >&2
  echo "usage: bash scripts/asr-smoke.sh path/to/video.mp4" >&2
  exit 1
fi

cargo run -q -p cat-speedup -- transcribe "$INPUT" \
  --engine sherpa \
  --language zh \
  --output "$OUT_DIR/transcript.json"

echo "transcript: $OUT_DIR/transcript.json"
