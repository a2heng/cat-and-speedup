#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/env.sh"
OUT="${1:-$ROOT/out/smoke}"
mkdir -p "$OUT"

FFMPEG_BIN="${CAT_SPEEDUP_FFMPEG:-ffmpeg}"
if [[ "$FFMPEG_BIN" == "ffmpeg" ]]; then
  FFPROBE_BIN="ffprobe"
else
  FFPROBE_BIN="$(dirname "$FFMPEG_BIN")/ffprobe"
fi

# 6s fixture: 2s tone, 1s silence, 2s tone, 1s silence.
"$FFMPEG_BIN" -y -hide_banner -loglevel error \
  -f lavfi -i "sine=frequency=440:sample_rate=48000:duration=2" \
  -f lavfi -i "anullsrc=r=48000:cl=mono:d=1" \
  -f lavfi -i "sine=frequency=440:sample_rate=48000:duration=2" \
  -f lavfi -i "anullsrc=r=48000:cl=mono:d=1" \
  -f lavfi -i "color=c=0x202020:s=640x360:r=30:d=6" \
  -filter_complex "[0:a][1:a][2:a][3:a]concat=n=4:v=0:a=1[a]" \
  -map "[a]" -map 4:v -c:v libx264 -pix_fmt yuv420p -c:a aac -shortest \
  "$OUT/input.mp4"

CAT_SPEEDUP_FFMPEG="$FFMPEG_BIN" cargo run -q -p cat-speedup -- \
  analyze "$OUT/input.mp4" --speed 4 --margin 0.15 --output "$OUT/plan.json"

CAT_SPEEDUP_FFMPEG="$FFMPEG_BIN" cargo run -q -p cat-speedup -- \
  render "$OUT/input.mp4" "$OUT/output.mp4" --plan "$OUT/plan.json"

CAT_SPEEDUP_FFMPEG="$FFMPEG_BIN" cargo run -q -p cat-speedup -- \
  prepare-asr "$OUT/input.mp4" "$OUT/asr.wav" --map "$OUT/asr-map.json" \
  --speed 4 --margin 0.15

cat > "$OUT/transcript-edited.json" <<'JSON'
{"segments":[{"start_ms":500,"end_ms":1500,"text":"hello"}]}
JSON

CAT_SPEEDUP_FFMPEG="$FFMPEG_BIN" cargo run -q -p cat-speedup -- \
  map-transcript "$OUT/transcript-edited.json" "$OUT/asr-map.json" \
  --output "$OUT/transcript-original.json"

echo "output duration: $("$FFPROBE_BIN" -v error -show_entries format=duration -of default=nw=1:nk=1 "$OUT/output.mp4")s"
echo "asr duration:    $("$FFPROBE_BIN" -v error -show_entries format=duration -of default=nw=1:nk=1 "$OUT/asr.wav")s"
echo "artifacts:       $OUT"
