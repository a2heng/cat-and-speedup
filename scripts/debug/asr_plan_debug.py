#!/usr/bin/env python3
"""Offline debug harness: VAD + ASR + plan, same idea as the phone.

Uses sherpa-onnx's VAD+offline-ASR CLI with the same X-ASR Zipformer model,
parses the speech segments (`start -- end: text`), then rebuilds the plan with
the Android algorithm and compares against ffmpeg silence detection.

Usage:
  python scripts/debug/asr_plan_debug.py VIDEO [--gap 1] [--margin 150] [--speed 10]
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


def tool(name: str, fallback: Path) -> str:
    found = shutil.which(name)
    if found:
        return found
    if fallback.exists():
        return str(fallback)
    sys.exit(f"missing {name} / {fallback}")


def run(cmd: list[str]) -> str:
    result = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="ignore")
    if result.returncode != 0:
        sys.exit(f"command failed ({result.returncode}): {' '.join(cmd)}\n{result.stderr[-2000:]}")
    return result.stdout + result.stderr


def prepare_audio(video: Path, work: Path) -> Path:
    wav = work / "audio.wav"
    run(
        [
            tool("ffmpeg", FFMPEG),
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
        ]
    )
    return wav


def vad_asr(wav: Path) -> list[tuple[int, int, str]]:
    output = run(
        [
            str(VAD_ASR),
            f"--silero-vad-model={VAD_MODEL}",
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
            start = int(float(match.group(1)) * 1000)
            end = int(float(match.group(2)) * 1000)
            segments.append((start, end, match.group(3)))
    return segments


def merge_ranges(ranges: list[tuple[int, int]], gap_ms: int) -> list[tuple[int, int]]:
    merged: list[tuple[int, int]] = []
    for start, end in sorted(ranges):
        if end <= start:
            continue
        if merged and start - merged[-1][1] <= gap_ms:
            merged[-1] = (merged[-1][0], max(merged[-1][1], end))
        else:
            merged.append((start, end))
    return merged


def build_plan(
    duration_ms: int,
    speech: list[tuple[int, int]],
    gap_ms: int,
    margin_ms: int,
    speed: float,
):
    segments: list[tuple[int, int, float, bool]] = []
    merged = merge_ranges(speech, 0)
    cursor = 0
    raw_cursor = 0
    for start, end in merged:
        raw_gap = start - raw_cursor
        if raw_gap >= gap_ms:
            voice_start = max(cursor, start - margin_ms)
            if voice_start > cursor:
                add(segments, cursor, voice_start, speed, False)
            voice_end = max(voice_start, min(duration_ms, end + margin_ms))
            add(segments, voice_start, voice_end, 1.0, True)
            cursor = voice_end
        else:
            voice_end = max(cursor, min(duration_ms, end + margin_ms))
            add(segments, cursor, voice_end, 1.0, True)
            cursor = voice_end
        raw_cursor = end
    if cursor < duration_ms:
        if duration_ms - raw_cursor >= gap_ms:
            add(segments, cursor, duration_ms, speed, False)
        else:
            add(segments, cursor, duration_ms, 1.0, True)
    return segments


def add(segments, start, end, speed, voice):
    if end <= start:
        return
    if segments and segments[-1][3] == voice and segments[-1][2] == speed:
        segments[-1] = (segments[-1][0], end, speed, voice)
    else:
        segments.append((start, end, speed, voice))


def silence_spans(video: Path, threshold_db: int, min_s: float = 0.5):
    output = run(
        [
            tool("ffmpeg", FFMPEG),
            "-hide_banner",
            "-i",
            str(video),
            "-af",
            f"silencedetect=noise={threshold_db}dB:d={min_s}",
            "-f",
            "null",
            "-",
        ]
    )
    spans = []
    start = None
    for line in output.splitlines():
        match = re.search(r"silence_start: ([0-9.]+)", line)
        if match:
            start = int(float(match.group(1)) * 1000)
            continue
        match = re.search(r"silence_end: ([0-9.]+)", line)
        if match and start is not None:
            spans.append((start, int(float(match.group(1)) * 1000)))
            start = None
    return spans


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("video", type=Path)
    parser.add_argument("--gap", type=float, default=1.0)
    parser.add_argument("--margin", type=int, default=150)
    parser.add_argument("--speed", type=float, default=10.0)
    parser.add_argument("--truth-db", type=int, default=-45)
    args = parser.parse_args()

    gap_ms = int(args.gap * 1000)
    work = Path(tempfile.mkdtemp(prefix="cat-speedup-debug-"))
    print(f"work: {work}")
    try:
        wav = prepare_audio(args.video, work)
        # WAV duration from ffmpeg header via wave module
        import wave

        with wave.open(str(wav), "rb") as handle:
            total = int(handle.getnframes() * 1000 / handle.getframerate())

        segments = vad_asr(wav)
        speech = merge_ranges([(s, e) for s, e, _ in segments], 0)
        plan = build_plan(total, speech, gap_ms, args.margin, args.speed)
        output = sum(
            int((end - start) / speed) if not voice else (end - start)
            for start, end, speed, voice in plan
        )
        silences = [s for s in plan if not s[3]]
        print(
            f"speech_segments={len(speech)} plan_segments={len(plan)} "
            f"accelerated={len(silences)} output={output/1000:.1f}s / {total/1000:.1f}s"
        )

        truth = silence_spans(args.video, args.truth_db)
        long_truth = [s for s in truth if s[1] - s[0] >= gap_ms]
        print(f"ground truth: {len(truth)} silences, {len(long_truth)} >= {args.gap}s")

        print("\naccelerated segments (source ms):")
        for start, end, speed, voice in silences:
            print(f"  {start:8d} -> {end:8d}  {end-start:6d}ms  {speed}x")

        def inside_truth(start: int, end: int) -> bool:
            return any(start >= t_start - 150 and end <= t_end + 150 for t_start, t_end in truth)

        false_silence = 0
        for start, end, _, _ in silences:
            if not inside_truth(start, end):
                false_silence += 1
                print(f"  BAD: accelerates outside truth silence: {start}-{end}")

        missed = 0
        for t_start, t_end in long_truth:
            covered = any(start < t_end and end > t_start for start, end, _, _ in silences)
            if not covered:
                missed += 1
                print(f"  MISSED long silence: {t_start}-{t_end} ({t_end-t_start}ms)")
        print(f"false accelerated speech: {false_silence}; missed long silences: {missed}")
    finally:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
