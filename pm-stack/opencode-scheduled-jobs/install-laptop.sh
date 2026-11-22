#!/usr/bin/env bash
# Install the managed OpenCode scheduled-job pilot on the laptop.
# Contract: install-laptop.sh [--dry-run|--enable]. Default installs without enabling.
set -Eeuo pipefail

MODE=install-disabled
seen=0
while (($#)); do
  case "$1" in
    --dry-run)
      ((seen == 0)) || { echo "usage: $0 [--dry-run|--enable]" >&2; exit 2; }
      MODE=dry-run
      seen=1
      ;;
    --enable)
      ((seen == 0)) || { echo "usage: $0 [--dry-run|--enable]" >&2; exit 2; }
      MODE=enable
      seen=1
      ;;
    *) echo "usage: $0 [--dry-run|--enable]" >&2; exit 2 ;;
  esac
  shift
done

SRC_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
JOB_SRC="$SRC_DIR/quota-freshness-summary"
DEST="$HOME/atlas/opencode-scheduled-jobs"
UNIT_DIR="$HOME/.config/systemd/user"
OPENCODE_BIN="$(command -v opencode || true)"
PYTHON_BIN="$(command -v python3 || true)"

if [[ -z "$OPENCODE_BIN" || -z "$PYTHON_BIN" ]]; then
  echo "install: opencode and python3 must be available on PATH" >&2
  exit 1
fi

if [[ "$MODE" == dry-run ]]; then
  printf 'would install job files to %s and units to %s; timer state unchanged\n' "$DEST" "$UNIT_DIR"
  exit 0
fi

install -d -m 0700 "$DEST/quota-freshness-summary" "$UNIT_DIR"
install -m 0644 "$JOB_SRC/prompt.md" "$DEST/quota-freshness-summary/prompt.md"
install -m 0755 "$JOB_SRC/run.sh" "$DEST/quota-freshness-summary/run.sh"
install -m 0644 "$JOB_SRC/summarize.py" "$DEST/quota-freshness-summary/summarize.py"
sed -e "s|@OPENCODE_BIN@|$OPENCODE_BIN|g" \
  -e "s|@PYTHON_BIN_DIR@|$(dirname -- "$PYTHON_BIN")|g" \
  "$SRC_DIR/systemd/user/opencode-quota-freshness-summary.service" \
  >"$UNIT_DIR/opencode-quota-freshness-summary.service"
install -m 0644 "$SRC_DIR/systemd/user/opencode-quota-freshness-summary.timer" "$UNIT_DIR/opencode-quota-freshness-summary.timer"

systemctl --user daemon-reload
if [[ "$MODE" == enable ]]; then
  systemctl --user enable --now opencode-quota-freshness-summary.timer
  echo "install: timer enabled; inspect with systemctl --user list-timers opencode-quota-freshness-summary.timer"
else
  echo "install: files installed; timer state unchanged. Enable explicitly with '$0 --enable'."
fi
