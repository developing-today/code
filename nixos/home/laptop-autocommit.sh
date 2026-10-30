#!/usr/bin/env bash
# laptop-autocommit — one tracked-file sweep of one laptop repo
# (Drew, 2026-10-28; developing-today/code#1723). Mirrors the
# sandbox's hatch-autocommit discipline: only files git already
# tracks are committed, only on the repo's default branch, and the
# branch is pushed when ahead. Untracked files are never touched
# here — the daily laptop-untracked-review classifies those.
set -u
export GIT_TERMINAL_PROMPT=0
d="${1:-}"
[ -n "$d" ] && [ -d "$d/.git" ] || exit 0
[ -e "$d/.git/index.lock" ] && exit 0
db="$(git -C "$d" symbolic-ref --short refs/remotes/origin/HEAD 2>/dev/null | sed 's|^origin/||')"
[ -z "$db" ] && db=main
[ "$(git -C "$d" branch --show-current)" = "$db" ] || exit 0
git -C "$d" add -u || exit 0
if ! git -C "$d" diff --cached --quiet; then
  if git -C "$d" commit -q -m "autocommit: $(basename "$d") $(date '+%Y-%m-%d %H:%M')"; then
    echo "autocommit: committed tracked changes in $d"
  else
    echo "autocommit: commit failed in $d"
    exit 0
  fi
fi
ahead="$(git -C "$d" rev-list --count "@{u}..HEAD" 2>/dev/null || echo 0)"
if [ "${ahead:-0}" -gt 0 ]; then
  timeout 60 git -C "$d" push -q origin "$db" 2>/dev/null && echo "autocommit: pushed $ahead commit(s) in $d" || true
fi
exit 0
