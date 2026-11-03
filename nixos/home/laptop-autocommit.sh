#!/usr/bin/env bash
# laptop-autocommit — one tracked-file sweep of one laptop repo
# (Drew, 2026-10-28; developing-today/code#1723). Mirrors the
# sandbox's hatch-autocommit discipline: only files git already
# tracks are committed, only on the repo's default branch, local
# commits are merge-pulled against origin and pushed back, stopping
# loudly only on real conflicts. (2026-10-31 fix: the original never
# pulled at all and swallowed push failures with || true, which
# wedged the laptop hatch-personal primary 2076 commits behind
# origin, invisibly — dezren39/hatch-personal#10.)
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
# sync: fetch, MERGE-pull when behind (never rebase — divergence
# self-heals as a merge commit), then push. A real merge conflict
# aborts loudly (exit 1, so the systemd unit shows failed) with the
# conflicted paths named; a rejected push likewise fails loudly
# instead of being swallowed. Local commits are always kept.
git -C "$d" fetch -q --no-tags origin "$db" || {
  echo "autocommit: fetch failed in $d (deferred)"
  exit 0
}
behind="$(git -C "$d" rev-list --count "HEAD..@{u}" 2>/dev/null || echo 0)"
if [ "${behind:-0}" -gt 0 ]; then
  stash_before="$(git -C "$d" stash list)"
  if ! git -C "$d" pull -q --no-rebase --autostash origin "$db"; then
    mh="$(git -C "$d" rev-parse --git-path MERGE_HEAD)"
    if [ -f "$mh" ]; then
      paths="$(git -C "$d" diff --name-only --diff-filter=U | paste -sd, -)"
      git -C "$d" merge --abort
      [ "$(git -C "$d" stash list)" != "$stash_before" ] && git -C "$d" stash pop -q
      echo "autocommit: MERGE CONFLICT syncing $d — conflicted paths: ${paths:-unknown} — aborted, local commits kept; a human must resolve (hatch-personal#10)"
      exit 1
    fi
    echo "autocommit: pull failed in $d (no merge in progress; deferred)"
    exit 0
  fi
  echo "autocommit: merge-pulled $behind commit(s) in $d"
fi
ahead="$(git -C "$d" rev-list --count "@{u}..HEAD" 2>/dev/null || echo 0)"
if [ "${ahead:-0}" -gt 0 ]; then
  if timeout 60 git -C "$d" push -q origin "$db"; then
    echo "autocommit: pushed $ahead commit(s) in $d"
  else
    echo "autocommit: PUSH FAILED in $d ($ahead commit(s) unpushed)"
    exit 1
  fi
fi
exit 0
