#!/usr/bin/env python3
"""PC lab for the strict-segmentation -> assembly algorithm.

1. VAD+ASR gives strict speech segments (sentence-ish, gaps >= 250 ms).
2. Assembly merges segments whose gap is below --merge seconds; larger gaps
   become accelerated segments.
3. Optionally renders a preview video with the same plan so it can be watched.

Usage:
  python scripts/debug/plan_lab.py VIDEO [--merge 5] [--margin 150] [--speed 8]
                                        [--render out.mp4] [--head 60]
"""
from __future__ import annotations

import argparse
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FFMPEG = ROOT / "tools/ffmpeg-9.0.2-full_build/bin/ffmpeg.exe"
VAD_ASR = (
    ROOT
    / "tools/sherpa-onnx/sherpa-onnx-v2.0.1-win-x64-shared-MD-Release-no-tts/bin/sherpa-onnx-vad-with-offline-asr.exe"
)
VAD_MODEL = ROOT / "models/silero_vad.onnx"
MODEL = ROOT / "models/sherpa-onnx-x-asr-zipformer-transducer-zh-en-int8-2026-06-03"


def run(cmd: list[str], quiet: bool = False) -> str:
    result = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="ignore")
    if result.returncode != 0:
        sys.exit(f"command failed ({result.returncode}): {' '.join(cmd)}\n{result.stderr[-3000:]}")
    return "" if quiet else result.stdout + result.stderr


def ffmpeg() -> str:
    return shutil.which("ffmpeg") or str(FFMPEG)


def extract(wav: Path, video: Path) -> None:
    run(
        [
            ffmpeg(),
            "-y",
            "-hide_banner",
            "-i",
            str(video),
            "-vn",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-c:a",
            "pcm_s16le",
            str(wav),
        ],
        quiet=True,
    )


def vad_asr(wav: Path) -> list[tuple[int, int, str]]:
    output = run(
        [
            str(VAD_ASR),
            f"--silero-vad-model={VAD_MODEL}",
            "--silero-vad-min-silence-duration=0.25",
            "--silero-vad-min-speech-duration=0.1",
            f"--tokens={MODEL / 'tokens.txt'}",
            f"--encoder={MODEL / 'encoder-epoch-99-avg-1.int8.onnx'}",
            f"--decoder={MODEL / 'decoder-epoch-99-avg-1.onnx'}",
            f"--joiner={MODEL / 'joiner-epoch-99-avg-1.int8.onnx'}",
            "--num-threads=4",
            str(wav),
        ]
    )
    segments = []
    for line in output.splitlines():
        match = re.match(r"^\s*([0-9.]+)\s+--\s+([0-9.]+):\s*(.*)$", line)
        if match:
            segments.append(
                (
                    int(float(match.group(1)) * 1000),
                    int(float(match.group(2)) * 1000),
                    match.group(3).strip(),
                )
            )
    return segments


def assemble(
    duration_ms: int,
    strict: list[tuple[int, int]],
    merge_ms: int,
    margin_ms: int,
    speed: float,
):
    """Return plan segments: (start, end, speed, is_voice, source_of_voice)."""
    groups: list[tuple[int, int]] = []
    cursor_end = None
    for start, end in strict:
        if cursor_end is not None and start - cursor_end < merge_ms:
            groups[-1] = (groups[-1][0], end)
        else:
            groups.append((start, end))
        cursor_end = end

    plan = []
    cursor = 0
    raw_cursor = 0
    for start, end in groups:
        if start - raw_cursor >= merge_ms:
            voice_start = max(cursor, start - margin_ms)
            if voice_start > cursor:
                plan.append((cursor, voice_start, speed, False))
            voice_end = max(voice_start, min(duration_ms, end + margin_ms))
            plan.append((voice_start, voice_end, 1.0, True))
            cursor = voice_end
        else:
            voice_end = max(cursor, min(duration_ms, end + margin_ms))
            plan.append((cursor, voice_end, 1.0, True))
            cursor = voice_end
        raw_cursor = end
    if cursor < duration_ms:
        if duration_ms - raw_cursor >= merge_ms:
            plan.append((cursor, duration_ms, speed, False))
        else:
            plan.append((cursor, duration_ms, 1.0, True))
    return groups, plan


def atempo_chain(speed: float) -> str:
    parts = []
    remaining = speed
    while remaining > 2.0 + 1e-6:
        parts.append("atempo=2.0")
        remaining /= 2.0
    if abs(remaining - 1.0) > 1e-3:
        parts.append(f"atempo={remaining:.6f}")
    return ",".join(parts) if parts else "anull"


def render(video: Path, plan, out: Path, fps: int = 30) -> None:
    inputs = ["-i", str(video)]
    filters = []
    labels = []
    for index, (start, end, speed, voice) in enumerate(plan):
        start_s = start / 1000
        end_s = end / 1000
        filters.append(
            f"[0:v]trim=start={start_s:.3f}:end={end_s:.3f},setpts=(PTS-STARTPTS)/{speed:.4f}[v{index}]"
        )
        audio = f"atrim=start={start_s:.3f}:end={end_s:.3f},asetpts=PTS-STARTPTS"
        if speed != 1.0:
            audio += "," + atempo_chain(speed)
        filters.append(f"[0:a]{audio}[a{index}]")
        labels.append(f"[v{index}][a{index}]")
    filters.append(f"{''.join(labels)}concat=n={len(plan)}:v=1:a=1[outv][outa]")
    cmd = [
        ffmpeg(),
        "-y",
        "-hide_banner",
        *inputs,
        "-filter_complex",
        ";".join(filters),
        "-map",
        "[outv]",
        "-map",
        "[outa]",
        "-r",
        str(fps),
        "-fps_mode",
        "cfr",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-crf",
        "20",
        "-c:a",
        "aac",
        "-b:a",
        "128k",
        str(out),
    ]
    run(cmd)
    print(f"rendered: {out}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("video", type=Path)
    parser.add_argument("--merge", type=float, default=5.0)
    parser.add_argument("--margin", type=int, default=150)
    parser.add_argument("--speed", type=float, default=8.0)
    parser.add_argument("--head", type=float, default=60.0)
    parser.add_argument("--render", type=Path)
    args = parser.parse_args()

    merge_ms = int(args.merge * 1000)
    work = Path(tempfile.mkdtemp(prefix="cut-speedup-lab-"))
    try:
        wav = work / "audio.wav"
        extract(wav, args.video)
        strict_segments = vad_asr(wav)
        import wave

        with wave.open(str(wav), "rb") as handle:
            duration_ms = int(handle.getnframes() * 1000 / handle.getframerate())

        strict = [(s, e) for s, e, _ in strict_segments]
        print(f"strict segments: {len(strict)}")
        print(f"\nfirst {args.head:.0f}s (strict):")
        for start, end, text in strict_segments:
            if start > args.head * 1000:
                break
            print(f"  {start/1000:7.2f} -- {end/1000:7.2f}  {text}")

        groups, plan = assemble(duration_ms, strict, merge_ms, args.margin, args.speed)
        output = sum(
            int((end - start) / speed) if not voice else (end - start)
            for start, end, speed, voice in plan
        )
        gaps = [start - prev_end for (_, prev_end), (start, _) in zip(strict, strict[1:])]
        big_gaps = [g for g in gaps if g >= merge_ms]
        print(
            f"\nassembly: groups={len(groups)} plan_segments={len(plan)} "
            f"gaps>={args.merge}s={len(big_gaps)} "
            f"output={output/1000:.1f}s / {duration_ms/1000:.1f}s"
        )

        print(f"\nplan 0-{args.head:.0f}s:")
        for start, end, speed, voice in plan:
            if start > args.head * 1000:
                break
            kind = "voice" if voice else f"{speed:g}x"
            print(f"  {start:8d} -> {end:8d}  {kind}")

        if args.render:
            args.render.parent.mkdir(parents=True, exist_ok=True)
            render(args.video, plan, args.render)
    finally:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
