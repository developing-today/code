#!/usr/bin/env bash
# Bring the service-owned checkout these units were installed from to
# origin/main before a backup run, so the units follow main without
# depending on anyone's working directory. The checkout is a mirror of
# main, never a place for commits of its own.
#
# Behaviour:
# - Already at origin/main: no-op.
# - Behind: reset the checkout to origin/main. The reset runs in a
#   child whose program text comes from argv, so rewriting this
#   script's own file mid-run cannot garble it.
# - Dirty tree (tracked modifications or untracked files): refuse —
#   a person has been working in the service checkout, and discarding
#   that silently is worse than a missed backup. The unit fails, and
#   the failure is the alarm.
# - Fetch failure (network down): warn and run with the current tree;
#   a transient network problem must not skip a backup.
# Command line: no arguments.
if [[ $# -ne 0 ]]; then
  printf 'Unknown argument: %q\n' "$1" >&2
  printf 'Usage: %s\n' "$0" >&2
  exit 2
fi
set -euo pipefail
FQ_DIR="$(cd "$(dirname "$0")/.." && pwd)"
ROOT="$(git -C "$FQ_DIR" rev-parse --show-toplevel)"
if ! git -C "$ROOT" fetch --quiet origin main; then
  echo "fleet-queue-backup: warning: fetch of origin/main failed; running with the current checkout" >&2
  exit 0
fi
if [[ -n "$(git -C "$ROOT" status --porcelain)" ]]; then
  echo "fleet-queue-backup: refusing to update a dirty service checkout: $ROOT" >&2
  echo "The checkout is service-owned; move any work to a lane worktree and clean it." >&2
  exit 1
fi
if [[ "$(git -C "$ROOT" rev-parse HEAD)" == "$(git -C "$ROOT" rev-parse origin/main)" ]]; then
  exit 0
fi
exec bash -c 'git -C "$1" checkout --quiet main && git -C "$1" reset --hard --quiet origin/main && echo "fleet-queue-backup: checkout now at $(git -C "$1" rev-parse --short HEAD)"' _ "$ROOT"
