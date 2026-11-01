#!/usr/bin/env bash
# Install the github-backup user units on this machine.
# Usage: GHB_CODE_DIR=~/git/code ./install-units.sh
# __GB_DIR__ in the unit templates is replaced with this directory's path
# inside the code checkout, so units always run the repo's scripts.
set -euo pipefail
GB_DIR="$(cd "$(dirname "$0")/.." && pwd)"
UNIT_DIR="$HOME/.config/systemd/user"
mkdir -p "$UNIT_DIR"
for u in github-backup-fetch.service github-backup-fetch.timer \
  github-backup-reconcile.service github-backup-reconcile.timer \
  github-backup-drain.service github-backup-drain.timer; do
  sed "s#__GB_DIR__#$GB_DIR#g" "$GB_DIR/systemd/$u" >"$UNIT_DIR/$u"
done
systemctl --user daemon-reload
systemctl --user enable --now github-backup-fetch.timer \
  github-backup-reconcile.timer github-backup-drain.timer
systemctl --user list-timers 'github-backup-*' --no-pager
