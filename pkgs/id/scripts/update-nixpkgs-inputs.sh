#!/usr/bin/env bash
# Wrapper: tries uv (via .py shebang) first, falls back to python3
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$SCRIPT_DIR/update-nixpkgs-inputs.py"

"$SCRIPT" "$@" || python3 "$SCRIPT" "$@"
