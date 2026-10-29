#!/usr/bin/env bash
# Install the Cloudflare quota exporter on Drew's laptop (run ON the
# laptop). Copies the exporter + wrapper next to its output directory,
# installs the systemd user units, enables the 15-minute timer, and
# starts one run immediately. Idempotent.
set -euo pipefail

SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEST_DIR="$HOME/atlas/cloudflare-export"
UNIT_DIR="$HOME/.config/systemd/user"

command -v bun >/dev/null || { echo "install-laptop: bun not found on PATH" >&2; exit 1; }
command -v bws >/dev/null || { echo "install-laptop: bws not found on PATH" >&2; exit 1; }
command -v jq >/dev/null || { echo "install-laptop: jq not found on PATH" >&2; exit 1; }

mkdir -p "$DEST_DIR" "$UNIT_DIR"
install -m 0755 "$SRC_DIR/cloudflare-quota-export.mjs" "$DEST_DIR/cloudflare-quota-export.mjs"
install -m 0755 "$SRC_DIR/cloudflare-quota-export.sh" "$DEST_DIR/cloudflare-quota-export.sh"
install -m 0644 "$SRC_DIR/cloudflare-quota-export.service" "$UNIT_DIR/cloudflare-quota-export.service"
install -m 0644 "$SRC_DIR/cloudflare-quota-export.timer" "$UNIT_DIR/cloudflare-quota-export.timer"

systemctl --user daemon-reload
systemctl --user enable --now cloudflare-quota-export.timer
systemctl --user start cloudflare-quota-export.service || true

echo "install-laptop: installed to $DEST_DIR; timer state:"
systemctl --user list-timers cloudflare-quota-export.timer --no-pager || true
echo "install-laptop: latest export (if the first run succeeded):"
cat "$DEST_DIR/latest.json" 2>/dev/null || echo "(no latest.json yet — check: journalctl --user -u cloudflare-quota-export.service)"
