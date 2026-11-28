#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="${HOME}/.local/bin"
UNIT_DIR="${XDG_CONFIG_HOME:-${HOME}/.config}/systemd/user"
mkdir -p "$BIN_DIR" "$UNIT_DIR"
install -m 0755 "$ROOT/bin/cargo_target_janitor.py" "$BIN_DIR/cargo-target-janitor"
install -m 0644 "$ROOT/systemd/cargo-target-janitor.service" "$UNIT_DIR/"
install -m 0644 "$ROOT/systemd/cargo-target-janitor.timer" "$UNIT_DIR/"
systemctl --user daemon-reload
systemctl --user enable --now cargo-target-janitor.timer
systemctl --user status cargo-target-janitor.timer --no-pager
