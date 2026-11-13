#!/usr/bin/env bash
# Download the Linux sherpa-onnx CLI used by the web UI for VAD + ASR.
# Windows keeps using the existing tools/sherpa-onnx/... build.
set -euo pipefail
cd "$(dirname "$0")/.."

version="${SHERPA_VERSION:-v2.0.1}"
name="sherpa-onnx-${version}-linux-x64-shared-no-tts"
url="https://github.com/k2-fsa/sherpa-onnx/releases/download/${version}/${name}.tar.bz2"
dest="tools/${name}"

if [[ -x "${dest}/bin/sherpa-onnx-vad-with-offline-asr" ]]; then
  echo "already present: ${dest}"
  exit 0
fi

mkdir -p tools
echo "downloading ${url}"
# Use the local proxy only when the direct connection is blocked.
if ! curl -fL --retry 3 -o "tools/${name}.tar.bz2" "${url}"; then
  echo "direct download failed, retrying through 127.0.0.1:10808" >&2
  curl -fL --retry 3 -x http://127.0.0.1:10808 -o "tools/${name}.tar.bz2" "${url}"
fi
tar xjf "tools/${name}.tar.bz2" -C tools
rm -f "tools/${name}.tar.bz2"
echo "ready: ${dest}"
