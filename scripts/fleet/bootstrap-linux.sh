#!/usr/bin/env bash
# bootstrap-linux.sh — bootstrap a 'linux' fleet seat (developing-today/code, #12236).
# Delegates to scripts/fleet/bootstrap.sh --seat linux; rejects --seat overrides (#3352).
set -euo pipefail
SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
for arg in "$@"; do
  case "$arg" in
    --seat|--seat=*)
      echo "fleet-bootstrap: usage error: ${0##*/} pins --seat linux; do not pass $arg" >&2
      exit 2
      ;;
  esac
done
exec bash "$SELF_DIR/bootstrap.sh" --seat linux "$@"
