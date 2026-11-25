#!/usr/bin/env bash
# Playability smoke test for the space-invaders world (#14939):
# drives the built wasm through the plaza ABI with a scripted game
# (smoke/drive.mjs). Build the game first (program/build.sh).
# Usage: smoke.sh [path-to-space-invaders.wasm]
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
wasm="${1:-../program/space-invaders.wasm}"
if [ ! -f "$wasm" ]; then
  echo "smoke: no built module at $wasm — build the game first (program/build.sh)" >&2
  exit 1
fi
if ! command -v node >/dev/null 2>&1; then
  echo "smoke: node is required to drive the built module" >&2
  exit 1
fi
node drive.mjs "$wasm"
