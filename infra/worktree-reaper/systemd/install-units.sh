#!/usr/bin/env bash
# Install the worktree-reaper user units on this machine.
# Usage: ./install-units.sh
# __WR_DIR__ in the unit templates is replaced with this directory's path
# inside the code checkout, so units always run the repo's scripts.
#
# If the units are installed from a LINKED worktree (the pre-merge
# bootstrap), the install tree would itself match the reaper's reap rule
# (pushed + clean + quiet). Two protections apply: the reaper always
# refuses to reap the tree it is running from, and this script also adds
# the install tree to the local config overlay's deny_paths. When you
# reinstall from the primary checkout after the PR merges, the overlay
# entry is harmless (primaries are never reaped anyway).
set -euo pipefail
WR_DIR="$(cd "$(dirname "$0")/.." && pwd)"
UNIT_DIR="$HOME/.config/systemd/user"
STATE_DIR="$HOME/.local/state/worktree-reaper"
mkdir -p "$UNIT_DIR" "$STATE_DIR"
for u in worktree-reaper.service worktree-reaper.timer; do
  sed "s#__WR_DIR__#$WR_DIR#g" "$WR_DIR/systemd/$u" > "$UNIT_DIR/$u"
done
# Denylist the install tree in the local overlay (deny lists union with
# the shipped config.json; the overlay lives in the state dir, never in
# the repo, so this cannot dirty the checkout).
python3 - "$STATE_DIR/config.local.json" "$WR_DIR" <<'PYEOF'
import json, os, sys
path, wr_dir = sys.argv[1], sys.argv[2]
# The install tree is the checkout root two levels above systemd/.
tree = os.path.abspath(os.path.join(wr_dir, "..", ".."))
cfg = {}
if os.path.isfile(path):
    with open(path) as fh:
        cfg = json.load(fh)
deny = cfg.setdefault("deny_paths", [])
if tree not in deny:
    deny.append(tree)
with open(path, "w") as fh:
    json.dump(cfg, fh, indent=2)
print(f"denylisted install tree in {path}: {tree}")
PYEOF
systemctl --user daemon-reload
systemctl --user enable --now worktree-reaper.timer
systemctl --user list-timers 'worktree-reaper*' --no-pager
