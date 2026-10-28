#!/usr/bin/env bash
# laptop-repo-keeper — hourly sweep that keeps the laptop's git
# primaries healthy (Drew, 2026-10-28; developing-today/code#1723).
#
# For every primary clone it discovers on this machine:
#   - fetches origin (pruning stale refs)
#   - flags tracked files over 25 MiB (report only)
#   - puts the primary back on the default branch when it is clean
#     (a dirty primary on the wrong branch is reported, never forced)
#   - commits a dirty tree on the default branch — all-or-nothing:
#     a tree containing a secret-shaped path, or an untracked file
#     over 10 MiB, is reported, never committed
#   - pulls fast-forward when behind, pushes when ahead; never
#     force-pushes, never merges, never rebases
# Worktrees are counted for the report but never touched: they are
# the fleet lanes' own workspaces.
#
# Each run writes ~/backups/repo-keeper/last-run.md and echoes the
# same lines to stdout (the journal).
set -u
export GIT_TERMINAL_PROMPT=0
REPORT_DIR="$HOME/backups/repo-keeper"
mkdir -p "$REPORT_DIR"
TS="$(date '+%Y-%m-%d %H:%M %Z')"
SECRET_RE='\.env|secret|credential|\.pem$|\.key$|id_rsa|id_ed25519'
BIG_TRACKED=26214400 # 25 MiB
BIG_NEW=10485760     # 10 MiB
LINES=()
say() { LINES+=("$1"); printf '%s\n' "$1"; }

declare -A SEEN=()
REPOS=()
for d in "$HOME"/git/*/ "$HOME/code" "$HOME/hatch" "$HOME/hatch-personal" "$HOME/.openclaw"; do
  d="${d%/}"
  [ -d "$d/.git" ] || continue
  real="$(readlink -f "$d")"
  [ -n "${SEEN[$real]:-}" ] && continue
  SEEN[$real]=1
  REPOS+=("$d")
done
IFS=$'\n' REPOS=($(printf '%s\n' "${REPOS[@]}" | sort))
unset IFS

say "# repo-keeper $TS"
for d in "${REPOS[@]}"; do
  name="$(basename "$d")"
  g() { git -C "$d" "$@"; }
  gd="$(g rev-parse --git-dir 2>/dev/null)"
  if [ -e "$d/.git/index.lock" ] || [ -e "$gd/MERGE_HEAD" ] || [ -d "$gd/rebase-merge" ] || [ -d "$gd/rebase-apply" ]; then
    say "- $name: BUSY (lock/merge/rebase in progress) — skipped"
    continue
  fi
  db="$(g symbolic-ref --short refs/remotes/origin/HEAD 2>/dev/null | sed 's|^origin/||')"
  [ -z "$db" ] && db=main
  timeout 60 g fetch -q --prune origin 2>/dev/null || say "- $name: FETCH FAILED — continuing with local state"
  br="$(g branch --show-current)"
  dirty="$(g status --porcelain | wc -l)"
  big="$(cd "$d" && git ls-files -z | xargs -0 -r stat -c '%s %n' 2>/dev/null | awk -v t="$BIG_TRACKED" '$1 > t {printf "%s (%d MiB); ", $2, $1 / 1048576}')"
  [ -n "$big" ] && say "- $name: LARGE TRACKED FILES: $big"
  wt="$(g worktree list | wc -l)"
  if [ "$br" != "$db" ]; then
    if [ "$dirty" -eq 0 ]; then
      if g checkout -q "$db"; then
        say "- $name: primary was on $br (clean) — moved to $db"
        br="$db"
      else
        say "- $name: primary on $br (clean) but checkout of $db failed — left alone"
        continue
      fi
    else
      say "- $name: PRIMARY ON $br with $dirty dirty entries — left alone (needs a human)"
      continue
    fi
  fi
  if [ "$dirty" -gt 0 ]; then
    guarded=""
    if g status --porcelain | grep -Eiq "$SECRET_RE"; then
      guarded="secret-shaped path present"
    else
      while IFS= read -r -d '' f; do
        sz="$(stat -c '%s' "$d/$f" 2>/dev/null || echo 0)"
        if [ "$sz" -gt "$BIG_NEW" ]; then
          guarded="untracked file over 10 MiB: $f"
          break
        fi
      done < <(g ls-files --others --exclude-standard -z)
    fi
    if [ -n "$guarded" ]; then
      say "- $name: DIRTY ($dirty entries) NOT committed — $guarded"
    elif g add -A && g commit -q -m "repo-keeper: snapshot $name $TS"; then
      say "- $name: committed $dirty dirty entries on $db"
      dirty=0
    else
      say "- $name: COMMIT FAILED ($dirty dirty entries)"
    fi
  fi
  counts="$(g rev-list --left-right --count "$db...origin/$db" 2>/dev/null || echo '0 0')"
  read -r ahead behind <<<"$counts"
  if [ "${behind:-0}" -gt 0 ] && [ "${ahead:-0}" -eq 0 ] && [ "$dirty" -eq 0 ]; then
    if timeout 60 g pull -q --ff-only origin "$db"; then
      say "- $name: pulled $behind commit(s) (fast-forward)"
    else
      say "- $name: PULL FAILED ($behind behind)"
    fi
  elif [ "${behind:-0}" -gt 0 ] && [ "${ahead:-0}" -gt 0 ]; then
    say "- $name: DIVERGED (ahead $ahead, behind $behind) — left alone"
  fi
  if [ "${ahead:-0}" -gt 0 ] && [ "$dirty" -eq 0 ]; then
    if timeout 60 g push -q origin "$db"; then
      say "- $name: pushed $ahead commit(s)"
    else
      say "- $name: PUSH FAILED ($ahead ahead)"
    fi
  fi
  say "- $name: ok ($db, worktrees: $wt)"
done
printf '%s\n' "${LINES[@]}" >"$REPORT_DIR/last-run.md"
