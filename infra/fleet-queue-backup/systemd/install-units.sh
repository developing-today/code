#!/usr/bin/env bash
# Install the fleet-queue-backup user units on this machine.
# __FQ_DIR__ in the unit templates is replaced with this directory's
# path inside the code checkout, so units always run the repo's
# scripts.
# Command line: no arguments.
if [[ $# -ne 0 ]]; then
  printf 'Unknown argument: %q\n' "$1" >&2
  printf 'Usage: %s (configure the checkout via its path, not arguments)\n' "$0" >&2
  exit 2
fi
set -euo pipefail
FQ_DIR="$(cd "$(dirname "$0")/.." && pwd)"
if ! GIT_DIR="$(git -C "$FQ_DIR" rev-parse --git-dir 2>/dev/null)" || \
  ! GIT_COMMON_DIR="$(git -C "$FQ_DIR" rev-parse --git-common-dir 2>/dev/null)"; then
  echo "fleet-queue-backup: refusing to install from a directory that is not a Git checkout: $FQ_DIR" >&2
  exit 1
fi
if [[ "$GIT_DIR" != "$GIT_COMMON_DIR" ]]; then
  echo "fleet-queue-backup: refusing to install from a Git worktree: $FQ_DIR" >&2
  echo "Install from a dedicated checkout instead; unit paths must not depend on a removable lane worktree." >&2
  exit 1
fi
UNIT_DIR="$HOME/.config/systemd/user"
mkdir -p "$UNIT_DIR"
for u in fleet-queue-backup-export.service fleet-queue-backup-export.timer \
  fleet-queue-backup-d1-home.service fleet-queue-backup-d1-home.timer; do
  sed "s#__FQ_DIR__#$FQ_DIR#g" "$FQ_DIR/systemd/$u" >"$UNIT_DIR/$u"
done
systemctl --user daemon-reload
systemctl --user enable --now fleet-queue-backup-export.timer
systemctl --user enable --now fleet-queue-backup-d1-home.timer
systemctl --user list-timers 'fleet-queue-backup-*' --no-pager
