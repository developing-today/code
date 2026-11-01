#!/usr/bin/env bash
# Tests for laptop-untracked-review.sh (code#1751): the review's two
# commits must be isolated to the literal paths the run itself produced.
# A pre-staged unrelated edit must survive a run still staged; the junk
# commit must contain .gitignore only; the track commit exactly the
# files this run staged (NUL-safe, unusual names included); a dirty
# .gitignore (staged or unstaged) must be left byte-identical with the
# skip reported; a junk name that is unsafe as a .gitignore pattern must
# be skipped, not written; and a failed commit must be reported, never
# silently swallowed. Drives the REAL script against fixture repos
# under a throwaway HOME.
set -uo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$SELF_DIR/laptop-untracked-review.sh"
[ -f "$SCRIPT" ] || { echo "FAIL: script not found at $SCRIPT" >&2; exit 1; }

TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT
failures=0
fail() { echo "FAIL: $1" >&2; failures=$((failures + 1)); }
check() { # $1 description, $2 expected, $3 actual
  if [ "$2" != "$3" ]; then fail "$1 — expected [$2], got [$3]"; fi
}
DAY="$(date '+%Y-%m-%d')"

export GIT_AUTHOR_NAME="untracked-review test"
export GIT_AUTHOR_EMAIL="test@example.invalid"
export GIT_COMMITTER_NAME="untracked-review test"
export GIT_COMMITTER_EMAIL="test@example.invalid"
export GIT_CONFIG_COUNT=1
export GIT_CONFIG_KEY_0="commit.gpgsign"
export GIT_CONFIG_VALUE_0="false"

new_repo() { # $1 repo dir — main branch, one initial commit
  mkdir -p "$1"
  git init -q -b main "$1"
  printf '# personal\n' >"$1/.gitignore"
  printf 'original\n' >"$1/tracked.txt"
  git -C "$1" add -- .gitignore tracked.txt
  git -C "$1" commit -q -m "initial"
}

new_home() { # $1 case name → echoes fixture HOME with both repos
  h="$TMP_ROOT/$1/home"
  mkdir -p "$h"
  new_repo "$h/hatch-personal"
  new_repo "$h/hatch"
  printf '%s' "$h"
}

run_review() { # $1 fixture HOME, $2 log file
  HOME="$1" bash "$SCRIPT" >"$2" 2>&1
}

commit_subjects() { git -C "$1" log --format=%s; }
commit_files() { git -C "$1" show --name-only --format= "$2" | sed '/^$/d' | sort; }
commit_count() { git -C "$1" rev-list --count HEAD; }
staged_names() { git -C "$1" diff --cached --name-only | sort; }

# --- Case 1: isolation of both commits (the original probe fixture) ---
H1="$(new_home case1)"
P1="$H1/hatch-personal"
printf 'half-finished edit\n' >>"$P1/tracked.txt"
git -C "$P1" add -- tracked.txt # pre-staged unrelated work
printf 'junk\n' >"$P1/notes.tmp"
printf 'note\n' >"$P1/newnote.md"
printf 'spaced\n' >"$P1/my notes.md"
printf 'k\n' >"$P1/api.key"
printf 'pub\n' >"$H1/hatch/pub.md"
run_review "$H1" "$TMP_ROOT/case1.log"
check "case1: three commits total" "3" "$(commit_count "$P1")"
check "case1: newest commit is the track commit" \
  "untracked-review: track 2 file(s) $DAY" "$(commit_subjects "$P1" | head -1)"
check "case1: second commit is the ignore commit" \
  "untracked-review: ignore junk paths $DAY" "$(commit_subjects "$P1" | sed -n 2p)"
check "case1: ignore commit contains .gitignore only" \
  ".gitignore" "$(commit_files "$P1" HEAD~1)"
check "case1: ignore commit adds the junk path" \
  "notes.tmp" "$(git -C "$P1" show HEAD~1:.gitignore | tail -1)"
check "case1: track commit contains exactly this run's files" \
  "$(printf 'my notes.md\nnewnote.md')" "$(commit_files "$P1" HEAD)"
check "case1: pre-staged edit still staged afterwards" \
  "tracked.txt" "$(staged_names "$P1")"
grep -Fq -- "- hatch-personal: .gitignore updated" "$TMP_ROOT/case1.log" \
  || fail "case1: report missing '.gitignore updated'"
grep -Fq -- "- hatch-personal: committed 2 newly tracked file(s)" "$TMP_ROOT/case1.log" \
  || fail "case1: report missing 'committed 2 newly tracked file(s)'"
grep -Fq -- "- hatch-personal: SECRET-SHAPED untracked, left alone: api.key" "$TMP_ROOT/case1.log" \
  || fail "case1: secret-shaped handling changed"
[ -n "$(git -C "$P1" ls-files --others --exclude-standard -- api.key)" ] \
  || fail "case1: api.key should still be untracked"
check "case1: public repo got no commits" "1" "$(commit_count "$H1/hatch")"
grep -Fq -- "- hatch: untracked (public repo, report only): pub.md" "$TMP_ROOT/case1.log" \
  || fail "case1: public report-only handling changed"

# --- Case 2: dirty .gitignore (unstaged) is never mutated or committed ---
H2="$(new_home case2)"
P2="$H2/hatch-personal"
printf '# wip\n' >>"$P2/.gitignore"
GI2_BEFORE="$(sha256sum "$P2/.gitignore" | cut -d' ' -f1)"
printf 'junk\n' >"$P2/junk.tmp"
printf 'ok\n' >"$P2/ok.md"
run_review "$H2" "$TMP_ROOT/case2.log"
check "case2: only the track commit happened" "2" "$(commit_count "$P2")"
check "case2: track commit contains ok.md only" \
  "ok.md" "$(commit_files "$P2" HEAD)"
check "case2: .gitignore byte-identical after the run" \
  "$GI2_BEFORE" "$(sha256sum "$P2/.gitignore" | cut -d' ' -f1)"
check "case2: .gitignore still unstaged-dirty" \
  " M .gitignore" "$(git -C "$P2" status --porcelain -- .gitignore)"
grep -Fq -- "- hatch-personal: junk NOT ignored (.gitignore has uncommitted changes, left untouched): junk.tmp" "$TMP_ROOT/case2.log" \
  || fail "case2: report missing per-file dirty-.gitignore skip"
grep -Fq -- "skipped 1 junk ignore(s) this run (index and worktree preserved)" "$TMP_ROOT/case2.log" \
  || fail "case2: report missing dirty-.gitignore summary"

# --- Case 2b: dirty .gitignore (staged) — skip, staged change preserved ---
H2B="$(new_home case2b)"
P2B="$H2B/hatch-personal"
printf '# staged wip\n' >>"$P2B/.gitignore"
git -C "$P2B" add -- .gitignore
printf 'junk\n' >"$P2B/junk.tmp"
printf 'ok\n' >"$P2B/ok.md"
run_review "$H2B" "$TMP_ROOT/case2b.log"
check "case2b: only the track commit happened" "2" "$(commit_count "$P2B")"
check "case2b: staged .gitignore change still staged" \
  ".gitignore" "$(staged_names "$P2B")"
grep -Fq -- "skipped 1 junk ignore(s) this run" "$TMP_ROOT/case2b.log" \
  || fail "case2b: report missing dirty-.gitignore summary"

# --- Case 3: junk name unsafe as a .gitignore pattern is skipped ---
H3="$(new_home case3)"
P3="$H3/hatch-personal"
printf 'junk\n' >"$P3/weird[1].tmp"
printf 'ok\n' >"$P3/ok2.md"
run_review "$H3" "$TMP_ROOT/case3.log"
check "case3: only the track commit happened" "2" "$(commit_count "$P3")"
check "case3: .gitignore untouched" \
  "# personal" "$(cat "$P3/.gitignore")"
grep -Fq -- "- hatch-personal: junk NOT ignored (name is unsafe as a .gitignore pattern, left alone): weird[1].tmp" "$TMP_ROOT/case3.log" \
  || fail "case3: report missing unsafe-pattern skip"
[ -n "$(git -C "$P3" ls-files --others --exclude-standard -- 'weird[1].tmp')" ] \
  || fail "case3: weird[1].tmp should still be untracked"

# --- Case 4: failed commits are reported, nothing is silently lost ---
H4="$(new_home case4)"
P4="$H4/hatch-personal"
printf '#!/bin/sh\nexit 1\n' >"$P4/.git/hooks/pre-commit"
chmod +x "$P4/.git/hooks/pre-commit"
printf 'junk\n' >"$P4/junk2.tmp"
printf 'ok\n' >"$P4/ok3.md"
run_review "$H4" "$TMP_ROOT/case4.log"
check "case4: no commits were made" "1" "$(commit_count "$P4")"
grep -Fq -- "- hatch-personal: .gitignore commit FAILED" "$TMP_ROOT/case4.log" \
  || fail "case4: report missing .gitignore commit failure"
grep -Fq -- "- hatch-personal: track commit FAILED" "$TMP_ROOT/case4.log" \
  || fail "case4: report missing track commit failure"
check "case4: tracked file left staged after the failed commit" \
  "ok3.md" "$(staged_names "$P4")"

if [ "$failures" -gt 0 ]; then
  echo "$failures failure(s)" >&2
  exit 1
fi
echo "all laptop-untracked-review tests passed"
