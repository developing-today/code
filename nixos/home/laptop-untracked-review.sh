#!/usr/bin/env bash
# laptop-untracked-review — daily classification of untracked files
# in the laptop's personal repos (Drew, 2026-10-28; code#1723).
# Mirrors the sandbox's daily review lanes:
#   - secret-shaped paths: reported, never touched
#   - files over 20 MiB: reported as backup material, never tracked
#     (for openclaw the report says whether the openclaw backup
#     already covers the path — too-big files land in the backup
#     scripts, not in git)
#   - obvious junk (*.tmp, *.bak-*, *~, .DS_Store): the path is
#     added to the repo's .gitignore and committed
#   - anything else: tracked + committed in the PRIVATE repos
#     (openclaw-personal, hatch-personal); in the PUBLIC hatch repo
#     it is reported only — nothing new goes public unattended
set -u
export GIT_TERMINAL_PROMPT=0
REPORT_DIR="$HOME/backups/repo-keeper"
mkdir -p "$REPORT_DIR"
DAY="$(date '+%Y-%m-%d')"
REPORT="$REPORT_DIR/untracked-review-$DAY.md"
BIG=20971520 # 20 MiB
SECRET_RE='\.env|secret|credential|\.pem$|\.key$|id_rsa|id_ed25519'
LINES=()
say() { LINES+=("$1"); printf '%s\n' "$1"; }
say "# untracked review $DAY"
review() {
  d="$1"
  mode="$2"
  [ -d "$d/.git" ] || return 0
  name="$(basename "$d")"
  db="$(git -C "$d" symbolic-ref --short refs/remotes/origin/HEAD 2>/dev/null | sed 's|^origin/||')"
  [ -z "$db" ] && db=main
  if [ "$(git -C "$d" branch --show-current)" != "$db" ]; then
    say "- $name: skipped (primary not on $db)"
    return 0
  fi
  ignored_new=0
  tracked_new=0
  while IFS= read -r -d '' f; do
    if printf '%s' "$f" | grep -Eiq "$SECRET_RE"; then
      say "- $name: SECRET-SHAPED untracked, left alone: $f"
      continue
    fi
    sz="$(stat -c '%s' "$d/$f" 2>/dev/null || echo 0)"
    if [ "$sz" -gt "$BIG" ]; then
      extra=""
      if [ "$name" = ".openclaw" ]; then
        case "$f" in
          tmp/* | cache/*) extra=" — NOT covered by openclaw-backup (excluded dir)" ;;
          *) extra=" — covered by openclaw-backup" ;;
        esac
      fi
      say "- $name: TOO BIG for git ($((sz / 1048576)) MiB), route to backup: $f$extra"
      continue
    fi
    case "$f" in
      *.tmp | *.bak-* | *~ | .DS_Store | */.DS_Store)
        if ! grep -Fxq "$f" "$d/.gitignore" 2>/dev/null; then
          printf '%s\n' "$f" >>"$d/.gitignore"
          ignored_new=1
        fi
        say "- $name: junk ignored: $f"
        ;;
      *)
        if [ "$mode" = "track" ]; then
          if git -C "$d" add -- "$f"; then
            tracked_new=$((tracked_new + 1))
            say "- $name: now tracking: $f"
          fi
        else
          say "- $name: untracked (public repo, report only): $f"
        fi
        ;;
    esac
  done < <(git -C "$d" ls-files --others --exclude-standard -z)
  if [ "$ignored_new" -eq 1 ]; then
    git -C "$d" add -- .gitignore && git -C "$d" commit -q -m "untracked-review: ignore junk paths $DAY" && say "- $name: .gitignore updated"
  fi
  if [ "$tracked_new" -gt 0 ]; then
    git -C "$d" commit -q -m "untracked-review: track $tracked_new file(s) $DAY" && say "- $name: committed $tracked_new newly tracked file(s)"
  fi
}
review "$HOME/.openclaw" track
review "$HOME/hatch-personal" track
review "$HOME/hatch" report
printf '%s\n' "${LINES[@]}" >"$REPORT"
