#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="${HOME}/.local/bin"
UNIT_DIR="${XDG_CONFIG_HOME:-${HOME}/.config}/systemd/user"
mkdir -p "$BIN_DIR" "$UNIT_DIR"
install -m 0755 "$ROOT/bin/worktree_janitor.py" "$BIN_DIR/worktree-janitor"
install -m 0644 "$ROOT/systemd/worktree-janitor.service" "$UNIT_DIR/"
install -m 0644 "$ROOT/systemd/worktree-janitor.timer" "$UNIT_DIR/"
systemctl --user daemon-reload
systemctl --user enable --now worktree-janitor.timer
systemctl --user status worktree-janitor.timer --no-pager
