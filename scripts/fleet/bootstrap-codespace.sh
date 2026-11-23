#!/usr/bin/env bash
# bootstrap-codespace.sh — bootstrap a 'codespace' fleet seat (developing-today/code, #12236).
# Delegates to scripts/fleet/bootstrap.sh --seat codespace; rejects --seat overrides (#3352).
set -euo pipefail
SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
for arg in "$@"; do
  case "$arg" in
  --seat | --seat=*)
    echo "fleet-bootstrap: usage error: ${0##*/} pins --seat codespace; do not pass $arg" >&2
    exit 2
    ;;
  esac
done
exec bash "$SELF_DIR/bootstrap.sh" --seat codespace "$@"
