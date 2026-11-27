#!/usr/bin/env bash
# Install the laptop-db-backup user units on this machine.
# __LDB_DIR__ in the unit templates is replaced with this directory's
# path inside the code checkout, so units always run the repo's
# scripts.
# Command line: no arguments.
if [[ $# -ne 0 ]]; then
  printf 'Unknown argument: %q\n' "$1" >&2
  printf 'Usage: %s (configure the checkout via its path, not arguments)\n' "$0" >&2
  exit 2
fi
set -euo pipefail
LDB_DIR="$(cd "$(dirname "$0")/.." && pwd)"
UNIT_DIR="$HOME/.config/systemd/user"
mkdir -p "$UNIT_DIR"
for u in laptop-db-backup-pm-projector.service laptop-db-backup-pm-projector.timer laptop-db-backup-fleet-metadata.service laptop-db-backup-fleet-metadata.timer laptop-db-backup-user-prompts.service laptop-db-backup-user-prompts.timer; do
  sed "s#__LDB_DIR__#$LDB_DIR#g" "$LDB_DIR/systemd/$u" >"$UNIT_DIR/$u"
done
systemctl --user daemon-reload
systemctl --user enable --now laptop-db-backup-pm-projector.timer laptop-db-backup-fleet-metadata.timer laptop-db-backup-user-prompts.timer
systemctl --user list-timers 'laptop-db-backup-*' --no-pager
