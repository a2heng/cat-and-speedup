#!/usr/bin/env python3
"""Reproduce / verify the Silero VAD "large chunk" data-loss bug on PC.

The Android app feeds the whole 30 s decode range into `Vad.acceptWaveform()`
in one call. sherpa-onnx's internal circular buffer is only
`max_speech_duration * sample_rate` samples (30 s here), so a full 30 s push
overflows it and everything except the tail is dropped. The first ~30-36 s of
a long recording then looks like silence and gets accelerated.

Run against the downloaded Linux sherpa-onnx runtime:

  python3 scripts/debug/vad_chunk_probe.py VIDEO \
      --lib tools/sherpa-onnx-v2.0.1-linux-x64-shared-no-tts/lib \
      --model models/silero_vad.onnx --seconds 180

Feed sizes of 30 s reproduce the bug; <=1 s does not.
"""
from __future__ import annotations

import argparse
import ctypes
import subprocess
import sys
import tempfile
import wave
from pathlib import Path


class SileroVadConfig(ctypes.Structure):
    _fields_ = [
        ("model", ctypes.c_char_p),
        ("threshold", ctypes.c_float),
        ("min_silence_duration", ctypes.c_float),
        ("min_speech_duration", ctypes.c_float),
        ("window_size", ctypes.c_int32),
        ("max_speech_duration", ctypes.c_float),
    ]


class TenVadConfig(ctypes.Structure):
    _fields_ = [
        ("model", ctypes.c_char_p),
        ("threshold", ctypes.c_float),
        ("min_silence_duration", ctypes.c_float),
        ("min_speech_duration", ctypes.c_float),
        ("window_size", ctypes.c_int32),
        ("max_speech_duration", ctypes.c_float),
    ]


class VadModelConfig(ctypes.Structure):
    _fields_ = [
        ("silero_vad", SileroVadConfig),
        ("sample_rate", ctypes.c_int32),
        ("num_threads", ctypes.c_int32),
        ("provider", ctypes.c_char_p),
        ("debug", ctypes.c_int32),
        ("ten_vad", TenVadConfig),
    ]


class SpeechSegment(ctypes.Structure):
    _fields_ = [
        ("start", ctypes.c_int64),
        ("samples", ctypes.POINTER(ctypes.c_float)),
        ("n", ctypes.c_int32),
    ]


def load(path: Path) -> ctypes.CDLL:
    lib = ctypes.CDLL(str(path))
    lib.SherpaOnnxCreateVoiceActivityDetector.restype = ctypes.c_void_p
    lib.SherpaOnnxCreateVoiceActivityDetector.argtypes = [
        ctypes.POINTER(VadModelConfig),
        ctypes.c_float,
    ]
    lib.SherpaOnnxDestroyVoiceActivityDetector.argtypes = [ctypes.c_void_p]
    lib.SherpaOnnxVoiceActivityDetectorAcceptWaveform.argtypes = [
        ctypes.c_void_p,
        ctypes.POINTER(ctypes.c_float),
        ctypes.c_int32,
    ]
    lib.SherpaOnnxVoiceActivityDetectorEmpty.argtypes = [ctypes.c_void_p]
    lib.SherpaOnnxVoiceActivityDetectorEmpty.restype = ctypes.c_int32
    lib.SherpaOnnxVoiceActivityDetectorFront.argtypes = [ctypes.c_void_p]
    lib.SherpaOnnxVoiceActivityDetectorFront.restype = ctypes.POINTER(SpeechSegment)
    lib.SherpaOnnxVoiceActivityDetectorPop.argtypes = [ctypes.c_void_p]
    lib.SherpaOnnxVoiceActivityDetectorFlush.argtypes = [ctypes.c_void_p]
    lib.SherpaOnnxDestroySpeechSegment.argtypes = [ctypes.POINTER(SpeechSegment)]
    return lib


def extract(video: Path, seconds: float, work: Path) -> Path:
    wav = work / "audio.wav"
    subprocess.run(
        [
            "ffmpeg", "-y", "-hide_banner", "-loglevel", "error",
            "-t", str(seconds), "-i", str(video), "-vn",
            "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le", str(wav),
        ],
        check=True,
    )
    return wav


def read_wav(path: Path) -> list[float]:
    with wave.open(str(path), "rb") as handle:
        raw = handle.readframes(handle.getnframes())
    import struct

    count = len(raw) // 2
    return [v / 32768.0 for v in struct.unpack(f"<{count}h", raw)]


def run(lib, model: Path, samples: list[float], chunk_s: float) -> list[tuple[float, float]]:
    config = VadModelConfig()
    config.silero_vad = SileroVadConfig(
        model=str(model).encode(),
        threshold=0.5,
        min_silence_duration=0.5,
        min_speech_duration=0.2,
        window_size=512,
        max_speech_duration=30.0,
    )
    config.sample_rate = 16000
    config.num_threads = 1
    config.provider = b"cpu"
    vad = lib.SherpaOnnxCreateVoiceActivityDetector(ctypes.byref(config), 30.0)
    if not vad:
        sys.exit("failed to create VAD")
    out: list[tuple[float, float]] = []
    array = (ctypes.c_float * len(samples))(*samples)
    chunk = max(1, int(chunk_s * 16000))
    fed = 0

    def drain(flush: bool = False) -> None:
        if flush:
            lib.SherpaOnnxVoiceActivityDetectorFlush(vad)
        while not lib.SherpaOnnxVoiceActivityDetectorEmpty(vad):
            seg = lib.SherpaOnnxVoiceActivityDetectorFront(vad)
            out.append((seg.contents.start / 16000.0, seg.contents.n / 16000.0))
            lib.SherpaOnnxDestroySpeechSegment(seg)
            lib.SherpaOnnxVoiceActivityDetectorPop(vad)

    while fed < len(samples):
        take = min(chunk, len(samples) - fed)
        lib.SherpaOnnxVoiceActivityDetectorAcceptWaveform(
            vad, ctypes.cast(ctypes.byref(array, fed * 4), ctypes.POINTER(ctypes.c_float)), take
        )
        fed += take
        drain()
    drain(flush=True)
    lib.SherpaOnnxDestroyVoiceActivityDetector(vad)
    return out


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("video", type=Path)
    parser.add_argument("--lib", type=Path, required=True,
                        help="directory with libsherpa-onnx-c-api.so")
    parser.add_argument("--model", type=Path, default=Path("models/silero_vad.onnx"))
    parser.add_argument("--seconds", type=float, default=180.0)
    parser.add_argument("--chunks", default="0.1,1,5,10,20,30",
                        help="comma separated feed sizes in seconds")
    args = parser.parse_args()

    work = Path(tempfile.mkdtemp(prefix="vad-chunk-"))
    wav = extract(args.video, args.seconds, work)
    samples = read_wav(wav)
    print(f"audio: {len(samples) / 16000:.1f}s -> {wav}")
    lib = load(args.lib / "libsherpa-onnx-c-api.so")
    for chunk_s in (float(x) for x in args.chunks.split(",")):
        segments = run(lib, args.model, samples, chunk_s)
        first = f"{segments[0][0]:.3f}s" if segments else "none"
        print(f"chunk={chunk_s:>5.2f}s  segments={len(segments):3d}  first_start={first}")
        if segments:
            print("   first 5:", ", ".join(f"{s:.2f}+{d:.2f}" for s, d in segments[:5]))


if __name__ == "__main__":
    main()
