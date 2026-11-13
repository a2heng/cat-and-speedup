#!/usr/bin/env bash
# Fetch the prebuilt sherpa-onnx Android JNI libraries into the app source set.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="v2.0.1"
CACHE="$ROOT/tools/sherpa-android"
ARCHIVE="$CACHE/sherpa-onnx-$VERSION-android.tar.bz2"
JNI_TARGET="$ROOT/apps/android/app/src/main/jniLibs"

if [[ ! -f "$ARCHIVE" ]]; then
  mkdir -p "$CACHE"
  echo "Downloading sherpa-onnx $VERSION Android libs (~45 MB)..."
  curl -L --fail --retry 3 \
    -o "$ARCHIVE" \
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/$VERSION/sherpa-onnx-$VERSION-android.tar.bz2"
fi

if [[ ! -d "$CACHE/jniLibs" ]]; then
  tar -xjf "$ARCHIVE" -C "$CACHE"
fi

mkdir -p "$JNI_TARGET"
for abi in "$CACHE"/jniLibs/*/; do
  name="$(basename "$abi")"
  rm -rf "$JNI_TARGET/$name"
  cp -r "$abi" "$JNI_TARGET/$name"
done

echo "jniLibs: $JNI_TARGET"
ls "$JNI_TARGET"
