#!/usr/bin/env bash
# C3 health hook: OpenCode working. Fires on failure-to-connect.
# Usage: opencode-working.sh [--caller X] [--observed-at ISO] [--detail "..."]
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 "$HERE/run.py" --target opencode-working "$@"
