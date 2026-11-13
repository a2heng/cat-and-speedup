#!/usr/bin/env bash
# Build and run the web UI. Port is fixed at 42006 for this machine.
set -euo pipefail
cd "$(dirname "$0")/.."
export CUT_SPEEDUP_ROOT="$PWD"
exec "${CARGO:-$HOME/.cargo/bin/cargo}" run --release -p cut-speedup-web -- \
  --host "${HOST:-0.0.0.0}" --port "${PORT:-42006}" "$@"
