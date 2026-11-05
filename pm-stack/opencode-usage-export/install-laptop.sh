#!/usr/bin/env bash
# Install the OpenCode usage exporter on Drew's laptop (run ON the laptop).
# Copies the exporter next to its output directory, renders the systemd
# user units with the laptop's node path, enables the 15-minute timer,
# and starts one run immediately. Idempotent.
set -euo pipefail

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEST_DIR="$HOME/atlas/opencode-usage-export"
UNIT_DIR="$HOME/.config/systemd/user"
NODE_BIN="$(command -v node || true)"

if [[ -z $NODE_BIN ]]; then
  echo "install-laptop: node not found on PATH" >&2
  exit 1
fi

mkdir -p "$DEST_DIR" "$UNIT_DIR"
install -m 0755 "$SRC_DIR/opencode-usage-export.mjs" "$DEST_DIR/opencode-usage-export.mjs"
sed "s|@NODE@|$NODE_BIN|g" "$SRC_DIR/opencode-usage-export.service" >"$UNIT_DIR/opencode-usage-export.service"
install -m 0644 "$SRC_DIR/opencode-usage-export.timer" "$UNIT_DIR/opencode-usage-export.timer"

systemctl --user daemon-reload
systemctl --user enable --now opencode-usage-export.timer
systemctl --user start opencode-usage-export.service || true

echo "install-laptop: installed to $DEST_DIR; timer state:"
systemctl --user list-timers opencode-usage-export.timer --no-pager || true
echo "install-laptop: latest export (if the first run succeeded):"
cat "$DEST_DIR/latest.json" 2>/dev/null || echo "(no latest.json yet — check: journalctl --user -u opencode-usage-export.service)"
