#!/usr/bin/env bash
# Source this before building: source scripts/env.sh
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# CMake ships with VS BuildTools on this machine.
VS_CMAKE="/c/Program Files (x86)/Microsoft Visual Studio/18/BuildTools/Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin"
if [[ -d "$VS_CMAKE" ]]; then
  case ":$PATH:" in
    *":$VS_CMAKE:"*) ;;
    *) export PATH="$VS_CMAKE:$PATH" ;;
  esac
fi

# bindgen needs libclang; we keep a copy under tools/ (gitignored).
if [[ -d "$ROOT/tools/libclang/clang/native" ]]; then
  export LIBCLANG_PATH="${LIBCLANG_PATH:-$ROOT/tools/libclang/clang/native}"
fi

if [[ -z "${CAT_SPEEDUP_FFMPEG:-}" && -x "$ROOT/tools/ffmpeg-9.0.2-full_build/bin/ffmpeg.exe" ]]; then
  export CAT_SPEEDUP_FFMPEG="$ROOT/tools/ffmpeg-9.0.2-full_build/bin/ffmpeg.exe"
fi

export CMAKE_BUILD_PARALLEL_LEVEL="${CMAKE_BUILD_PARALLEL_LEVEL:-8}"
