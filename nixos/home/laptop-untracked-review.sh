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
# Commit isolation (code#1751): both commits are scoped with literal
# pathspecs to exactly the paths this run produced, so pre-staged
# unrelated work is never swept into a review commit. If .gitignore
# has uncommitted changes of its own, the ignore step is skipped and
# reported instead — mutating or committing it would sweep that work.
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
  # .gitignore protection (code#1751): a .gitignore that exists on disk
  # with any uncommitted state — staged, unstaged, or present but
  # untracked — holds work this run did not produce. Appending to it or
  # committing it would sweep that work into the review's commit, so
  # the whole ignore step is skipped (and reported) for such a repo.
  gitignore_dirty=0
  if [ -e "$d/.gitignore" ] && [ -n "$(git -C "$d" status --porcelain -- .gitignore 2>/dev/null)" ]; then
    gitignore_dirty=1
  fi
  ignored_new=0
  ignored_skipped=0
  tracked_new=0
  tracked_files=()
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
        case "$f" in
          *'*'* | *'?'* | *'['* | *'\'* | '#'* | '!'* | *' ')
            # Unsafe as a .gitignore pattern: written literally it would
            # match more than this one path (glob characters), match
            # nothing (a leading # is a comment, ! negates), or be
            # trimmed (trailing space). Skip it rather than write a
            # pattern that lies about what it ignores (code#1751).
            say "- $name: junk NOT ignored (name is unsafe as a .gitignore pattern, left alone): $f"
            ;;
          *)
            if [ "$gitignore_dirty" -eq 1 ]; then
              ignored_skipped=$((ignored_skipped + 1))
              say "- $name: junk NOT ignored (.gitignore has uncommitted changes, left untouched): $f"
            elif ! grep -Fxq "$f" "$d/.gitignore" 2>/dev/null; then
              printf '%s\n' "$f" >>"$d/.gitignore"
              ignored_new=1
              say "- $name: junk ignored: $f"
            else
              say "- $name: junk ignored: $f"
            fi
            ;;
        esac
        ;;
      *)
        if [ "$mode" = "track" ]; then
          if git -C "$d" --literal-pathspecs add -- "$f"; then
            tracked_new=$((tracked_new + 1))
            tracked_files+=("$f")
            say "- $name: now tracking: $f"
          else
            say "- $name: FAILED to stage untracked file, left alone: $f"
          fi
        else
          say "- $name: untracked (public repo, report only): $f"
        fi
        ;;
    esac
  done < <(git -C "$d" ls-files --others --exclude-standard -z)
  if [ "$ignored_skipped" -gt 0 ]; then
    say "- $name: .gitignore has uncommitted changes — skipped $ignored_skipped junk ignore(s) this run (index and worktree preserved)"
  fi
  if [ "$ignored_new" -eq 1 ]; then
    # Partial commit of .gitignore alone (code#1751): commits the
    # worktree content of that one literal path and leaves every other
    # staged change exactly as it was. No separate `git add` — staging
    # is not needed for a pathspec commit and touching the index is
    # what this fix exists to avoid.
    if git -C "$d" --literal-pathspecs commit -q -m "untracked-review: ignore junk paths $DAY" -- .gitignore; then
      say "- $name: .gitignore updated"
    else
      say "- $name: .gitignore commit FAILED — ignore changes left in the worktree"
    fi
  fi
  if [ "$tracked_new" -gt 0 ]; then
    # Commit exactly the files this run staged, by literal pathspec
    # (code#1751): pre-staged unrelated work stays staged, and the
    # report line below fires only when the commit actually happened.
    if git -C "$d" --literal-pathspecs commit -q -m "untracked-review: track $tracked_new file(s) $DAY" -- "${tracked_files[@]}"; then
      say "- $name: committed $tracked_new newly tracked file(s)"
    else
      say "- $name: track commit FAILED — $tracked_new file(s) left staged"
    fi
  fi
}
review "$HOME/.openclaw" track
review "$HOME/hatch-personal" track
review "$HOME/hatch" report
printf '%s\n' "${LINES[@]}" >"$REPORT"
