#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="${HOME}/.local/bin"
UNIT_DIR="${XDG_CONFIG_HOME:-${HOME}/.config}/systemd/user"
mkdir -p "$BIN_DIR" "$UNIT_DIR"
install -m 0755 "$ROOT/bin/yard_reaper.py" "$BIN_DIR/yard-reaper"
install -m 0644 "$ROOT/systemd/yard-reaper.service" "$UNIT_DIR/"
install -m 0644 "$ROOT/systemd/yard-reaper.timer" "$UNIT_DIR/"
# The yard reaper runs the cargo-target plan inside every run; the
# standalone janitor service is retired so the cap is enforced by
# exactly one tool (code#7093).
systemctl --user disable --now cargo-target-janitor.timer || true
systemctl --user daemon-reload
systemctl --user enable --now yard-reaper.timer
systemctl --user status yard-reaper.timer --no-pager
