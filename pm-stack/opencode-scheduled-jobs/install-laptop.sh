#!/usr/bin/env bash
# Install the managed OpenCode scheduled-job pilot on the laptop.
# Default installs the disabled unit pair; --enable is an explicit opt-in.
set -Eeuo pipefail

MODE=install-disabled
case "${1:-}" in
  "") ;;
  --enable) MODE=enable ;;
  --dry-run) MODE=dry-run ;;
  *) echo "usage: $0 [--dry-run|--enable]" >&2; exit 2 ;;
esac

SRC_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
JOB_SRC="$SRC_DIR/quota-freshness-summary"
DEST="$HOME/atlas/opencode-scheduled-jobs"
UNIT_DIR="$HOME/.config/systemd/user"
OPENCODE_BIN="$(command -v opencode || true)"

if [[ -z "$OPENCODE_BIN" ]]; then
  echo "install: opencode not found on PATH" >&2
  exit 1
fi

if [[ "$MODE" == dry-run ]]; then
  printf 'would install job files to %s and units to %s; timer remains disabled\n' "$DEST" "$UNIT_DIR"
  exit 0
fi

install -d -m 0700 "$DEST/quota-freshness-summary" "$UNIT_DIR"
install -m 0644 "$JOB_SRC/prompt.md" "$DEST/quota-freshness-summary/prompt.md"
install -m 0755 "$JOB_SRC/run.sh" "$DEST/quota-freshness-summary/run.sh"
install -m 0644 "$JOB_SRC/summarize.py" "$DEST/quota-freshness-summary/summarize.py"
sed "s|@OPENCODE_BIN@|$OPENCODE_BIN|g" \
  "$SRC_DIR/systemd/user/opencode-quota-freshness-summary.service" \
  >"$UNIT_DIR/opencode-quota-freshness-summary.service"
install -m 0644 "$SRC_DIR/systemd/user/opencode-quota-freshness-summary.timer" "$UNIT_DIR/opencode-quota-freshness-summary.timer"

systemctl --user daemon-reload
if [[ "$MODE" == enable ]]; then
  systemctl --user enable --now opencode-quota-freshness-summary.timer
  echo "install: timer enabled; inspect with systemctl --user list-timers opencode-quota-freshness-summary.timer"
else
  echo "install: files installed; timer remains disabled. Run '$0 --enable' only after explicit approval."
fi
