#!/usr/bin/env bash
# Usage: worker-preflight.sh
# Operator preflight: verifies this Git worktree is safe for worker Git operations.
# Accepts no positional arguments.

set -euo pipefail

if [[ $# -ne 0 ]]; then
  printf 'worker-preflight: usage error: unexpected argument(s): %s\n' "$*" >&2
  printf 'usage: worker-preflight.sh\n' >&2
  exit 2
fi

repo_root="$(git rev-parse --show-toplevel 2>/dev/null)" || {
  printf 'worker-preflight: must run inside a Git worktree\n' >&2
  exit 2
}

stray_ref='refs/heads/origin/main'
if git -C "$repo_root" show-ref --verify --quiet "$stray_ref"; then
  sha="$(git -C "$repo_root" rev-parse --verify "$stray_ref^{commit}" 2>/dev/null || printf 'unreadable')"
  printf 'worker-preflight: refusing to continue: local branch %s exists at %s and shadows refs/remotes/origin/main; stop and escalate before doing Git-based work.\n' "$stray_ref" "$sha" >&2
  exit 1
else
  status=$?
  if [[ $status -ne 1 ]]; then
    printf 'worker-preflight: could not verify %s (git show-ref exited %s)\n' "$stray_ref" "$status" >&2
    exit 2
  fi
fi

printf 'worker-preflight: clean (%s; no local %s)\n' "$repo_root" "$stray_ref"
