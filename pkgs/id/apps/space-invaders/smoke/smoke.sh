#!/usr/bin/env bash
# Playability smoke test for the space-invaders world (#14939):
# drives the built wasm through the plaza ABI with a scripted game
# (smoke/drive.mjs). Build the game first (program/build.sh).
# Usage: smoke.sh [path-to-space-invaders.wasm]
set -euo pipefail
# Resolve an explicit module path against the caller's cwd BEFORE the
# cd below: callers pass repo-root-relative paths (the id-roc-game
# workflow does), which would otherwise be resolved under smoke/ and
# never found. The default stays relative to this script's directory.
if [ $# -ge 1 ]; then
  case "$1" in
  /*) wasm="$1" ;;
  *) wasm="$PWD/$1" ;;
  esac
else
  wasm=""
fi
cd "$(dirname "${BASH_SOURCE[0]}")"
wasm="${wasm:-../program/space-invaders.wasm}"
if [ ! -f "$wasm" ]; then
  echo "smoke: no built module at $wasm — build the game first (program/build.sh)" >&2
  exit 1
fi
if ! command -v node >/dev/null 2>&1; then
  echo "smoke: node is required to drive the built module" >&2
  exit 1
fi
node drive.mjs "$wasm"
