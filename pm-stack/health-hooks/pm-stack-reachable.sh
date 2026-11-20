#!/usr/bin/env bash
# C3 health hook: PM stack reachable. Fires on failure-to-connect.
# Usage: pm-stack-reachable.sh [--caller X] [--observed-at ISO] [--detail "..."]
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 "$HERE/run.py" --target pm-stack-reachable "$@"
