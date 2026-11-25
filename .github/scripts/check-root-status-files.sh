#!/usr/bin/env bash
# Repo-root status-file guard (#12136).
#
# Fails when a file at the repository root has a name containing
# COMPLETE or SUMMARY (case-insensitive). Those names mark one-shot
# status documents ("implementation complete", slice summaries,
# completion reports) that go stale the moment they land; #12136
# removed 26 of them, plus two helper scripts and a committed chat
# transcript, in PR #13047. Status belongs in the issue tracker or
# under doc/, never as a new root file.
#
# Only the root is guarded: the same names inside a directory (for
# example the pm-stack/opencode-claimant/ copies the issue's evidence
# names) are that directory's own business.
#
# No grandfather list: the removal landed first (this guard is
# stacked on PR #13047), so the invariant holds on the whole tree
# from the day the guard lands. The repo's grandfather-list ratchet
# (check-production-line-limit.sh) exists for violations that take
# many PRs to burn down; here a single already-open PR removes them
# all, so a list would be dead entries the moment #13047 merges.
#
# Argument contract (#3352): the whole command line is the single
# optional positional argument below; anything else is a usage
# error (exit 2) before any work happens.
#
# Usage: check-root-status-files.sh [ROOT]   (default: repo root)
set -euo pipefail

[[ $# -le 1 ]] || {
  echo "Usage: check-root-status-files.sh [ROOT]" >&2
  exit 2
}

ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"

if [ ! -d "$ROOT" ]; then
  echo "::error::root status guard: root not found: $ROOT"
  exit 1
fi

fail=0
checked=0
while IFS= read -r -d '' file; do
  base="$(basename "$file")"
  lower="${base,,}"
  checked=$((checked + 1))
  case "$lower" in
  *complete* | *summary*)
    echo "::error::ROOT STATUS FILE $base: root files named *COMPLETE* / *SUMMARY* are one-shot status documents that go stale on landing (#12136). Record the status in the issue tracker or under doc/ instead."
    fail=1
    ;;
  esac
done < <(find "$ROOT" -maxdepth 1 \( -type f -o -type l \) -print0 | sort -z)

echo "root-status-guard: checked $checked root files"
if [ "$fail" -ne 0 ]; then
  echo "root-status-guard: FAILED - root status file(s) listed above"
  exit 1
fi
echo "root-status-guard: OK"
