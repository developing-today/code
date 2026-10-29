#!/usr/bin/env bash
# laptop-repo-keeper — hourly sweep that keeps the laptop's git
# primaries healthy (Drew, 2026-10-28; developing-today/code#1723).
#
# For every primary clone it discovers on this machine:
#   - fetches origin (pruning stale refs)
#   - flags tracked files over 25 MiB (report only)
#   - puts the primary back on the default branch when it is clean
#     (a dirty primary on the wrong branch is reported, never forced)
#   - commits tracked-file changes on the default branch —
#     all-or-nothing: a tree containing a secret-shaped tracked
#     path is reported, never committed. Untracked files are only
#     counted here; the daily laptop-untracked-review classifies
#     them (oversized ones belong to the backup scripts)
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
LINES=()
say() {
  LINES+=("$1")
  printf '%s\n' "$1"
}

# Discovery (Drew 2026-10-28: the keeper checks ALL repos): every
# primary clone under $HOME to depth 3 — a .git DIRECTORY marks a
# primary (worktrees carry a .git file and stay the lanes' own
# business). Excluded: scratch (~/tmp, ~/backups, trash, caches),
# anything nested inside an already-found repo, and duplicate
# paths to one repo (~/code is a symlink to ~/git/code).
declare -A SEEN=()
REPOS=()
while IFS= read -r gd; do
  d="${gd%/.git}"
  case "$d" in
  "$HOME/tmp" | "$HOME/tmp/"* | "$HOME/backups" | "$HOME/backups/"* | \
    "$HOME/.cache" | "$HOME/.cache/"* | "$HOME/.local/share/Trash" | "$HOME/.local/share/Trash/"* | \
    "$HOME/tools.prelink"*) continue ;;
  esac
  nested=0
  for e in "${REPOS[@]}"; do
    case "$d" in
    "$e"/*) nested=1 ;;
    esac
  done
  [ "$nested" -eq 1 ] && continue
  real="$(readlink -f "$d")"
  [ -n "${SEEN[$real]:-}" ] && continue
  SEEN[$real]=1
  REPOS+=("$d")
done < <(find "$HOME" -maxdepth 3 -name .git -type d 2>/dev/null | sort)

say "# repo-keeper $TS"
for d in "${REPOS[@]}"; do
  name="$(basename "$d")"
  g() { git -C "$d" "$@"; }
  gd="$(g rev-parse --absolute-git-dir 2>/dev/null)"
  if [ -e "$d/.git/index.lock" ] || [ -e "$gd/MERGE_HEAD" ] || [ -d "$gd/rebase-merge" ] || [ -d "$gd/rebase-apply" ]; then
    say "- $name: BUSY (lock/merge/rebase in progress) — skipped"
    continue
  fi
  db="$(g symbolic-ref --short refs/remotes/origin/HEAD 2>/dev/null | sed 's|^origin/||')"
  [ -z "$db" ] && db=main
  has_origin=0
  g remote get-url origin >/dev/null 2>&1 && has_origin=1
  # A repo with no origin (e.g. the local clan-flakes working repos)
  # has no upstream default to align to: whatever branch it is on IS
  # its default — never flag it, never move it.
  if [ "$has_origin" -eq 0 ]; then
    db="$(g branch --show-current)"
    [ -z "$db" ] && db=main
  fi
  if [ "$has_origin" -eq 1 ]; then
    timeout 60 git -C "$d" fetch -q --prune origin 2>/dev/null || say "- $name: FETCH FAILED — continuing with local state"
  fi
  br="$(g branch --show-current)"
  dirty="$(g status --porcelain | grep -vc '^??')"
  untracked="$(g ls-files --others --exclude-standard | wc -l)"
  # repo-archive holds git bundles BY DESIGN — its large files are the
  # archive itself, not violations, so it is exempt from this flag.
  if [ "$name" != "repo-archive" ]; then
    big="$(cd "$d" && git ls-files -z | xargs -0 -r stat -c '%s %n' 2>/dev/null | awk -v t="$BIG_TRACKED" '$1 > t {printf "%s (%d MiB); ", $2, $1 / 1048576}')"
    [ -n "$big" ] && say "- $name: LARGE TRACKED FILES: $big"
  fi
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
    if g status --porcelain | grep -v '^??' | grep -Eiq "$SECRET_RE"; then
      say "- $name: DIRTY ($dirty tracked entries) NOT committed — secret-shaped path present"
    elif g add -u && g commit -q -m "repo-keeper: snapshot $name $TS"; then
      say "- $name: committed $dirty tracked entries on $db"
      dirty=0
    else
      say "- $name: COMMIT FAILED ($dirty tracked entries)"
    fi
  fi
  if [ "$untracked" -gt 0 ]; then
    say "- $name: $untracked untracked file(s) — left for the daily review"
  fi
  counts="0 0"
  if [ "$has_origin" -eq 1 ]; then
    counts="$(g rev-list --left-right --count "$db...origin/$db" 2>/dev/null || echo '0 0')"
  fi
  read -r ahead behind <<<"$counts"
  if [ "${behind:-0}" -gt 0 ] && [ "${ahead:-0}" -eq 0 ] && [ "$dirty" -eq 0 ]; then
    if timeout 60 git -C "$d" pull -q --ff-only origin "$db"; then
      say "- $name: pulled $behind commit(s) (fast-forward)"
    else
      say "- $name: PULL FAILED ($behind behind)"
    fi
  elif [ "${behind:-0}" -gt 0 ] && [ "${ahead:-0}" -gt 0 ]; then
    say "- $name: DIVERGED (ahead $ahead, behind $behind) — left alone"
  fi
  if [ "${ahead:-0}" -gt 0 ] && [ "$dirty" -eq 0 ]; then
    if timeout 60 git -C "$d" push -q origin "$db"; then
      say "- $name: pushed $ahead commit(s)"
    else
      say "- $name: PUSH FAILED ($ahead ahead)"
    fi
  fi
  say "- $name: ok ($db, worktrees: $wt)"
done
printf '%s\n' "${LINES[@]}" >"$REPORT_DIR/last-run.md"
