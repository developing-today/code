#!/usr/bin/env bash
# Wrapper: tries uv (via .py shebang) first, falls back to python3 only
# when the script cannot be executed at all (exit 126/127, e.g. uv is
# missing) — never after a genuine update failure, which would re-run
# the whole update and mask the first run's results.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$SCRIPT_DIR/update-nixpkgs-inputs.py"

"$SCRIPT" "$@"
status=$?
if [ "$status" -eq 126 ] || [ "$status" -eq 127 ]; then
  python3 "$SCRIPT" "$@"
  status=$?
fi
exit "$status"
