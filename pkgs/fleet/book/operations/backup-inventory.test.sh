#!/usr/bin/env bash
# backup-inventory.test.sh — drift guard for backup-inventory.md:
# every repo path the inventory cites must still exist in the tree.
# Run from anywhere:  bash pkgs/fleet/book/operations/backup-inventory.test.sh
# Argument contract (#3352): this script takes NO arguments; anything
# passed is a usage error (exit 2) before any file is read.
if [ "$#" -ne 0 ]; then
  echo "usage: backup-inventory.test.sh" >&2
  exit 2
fi
set -u
cd "$(dirname "$0")/../../../.." || exit 1

paths=(
  infra/github-backup/config.json
  infra/github-backup/systemd
  pkgs/cold-store-reaper/scripts/nightly-backup.sh
  pkgs/cold-store-reaper/scripts/export-turso.sh
  pkgs/cold-store-reaper/scripts/restore-drill.sh
  pkgs/agent-backup/backup.sh
  pkgs/agent-backup/README.md
  pkgs/id/book/operations/backup-and-restore.md
  doc/2026-11-23T16-34-18Z_design_backup_mesh/note.md
)

fail=0
for p in "${paths[@]}"; do
  if [ -e "$p" ]; then
    echo "ok: $p"
  else
    echo "MISSING: $p (cited by pkgs/fleet/book/operations/backup-inventory.md)" >&2
    fail=1
  fi
done
exit "$fail"
