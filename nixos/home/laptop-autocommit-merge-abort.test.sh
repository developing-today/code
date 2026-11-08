#!/usr/bin/env bash
# Regression test for #3367: the pull-conflict abort in laptop-autocommit.sh
# must resolve MERGE_HEAD to an absolute path, so a conflicted merge-pull is
# detected and aborted even when the script runs with a working directory
# other than the target repo (the systemd unit sets no WorkingDirectory).
# Scenario: two diverged clones of one origin; the behind clone is swept
# from a foreign cwd. Expect exit 1, the merge aborted, and no conflict
# markers staged or committed. With the old relative
# `rev-parse --git-path MERGE_HEAD`, the `[ -f ]` test misses and the sweep
# exits 0 leaving the conflict in place.
set -uo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$SELF_DIR/laptop-autocommit.sh"

TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT
failures=0
fail() {
  echo "FAIL: $1" >&2
  failures=$((failures + 1))
}

export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@t \
  GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t GIT_TERMINAL_PROMPT=0

ORIGIN="$TMP_ROOT/origin.git"
A="$TMP_ROOT/a"
B="$TMP_ROOT/b"
ELSEWHERE="$TMP_ROOT/elsewhere"
mkdir -p "$ELSEWHERE"

git init -q -b main --bare "$ORIGIN"
git clone -q "$ORIGIN" "$A"
git -C "$A" commit -q --allow-empty -m base --no-verify
echo base >"$A/file.txt"
git -C "$A" add file.txt
git -C "$A" commit -q -m "add file.txt"
git -C "$A" push -q origin main
git clone -q "$ORIGIN" "$B"

echo A-side >"$A/file.txt"
git -C "$A" commit -qam "a-side change"
git -C "$A" push -q origin main

echo B-side >"$B/file.txt"
git -C "$B" commit -qam "b-side change"

# Sweep the behind clone from a foreign working directory (the service case).
out="$(cd "$ELSEWHERE" && bash "$SCRIPT" "$B" 2>&1)"
code=$?

[ "$code" -eq 1 ] || fail "expected exit 1 on conflicted pull, got $code (out: $out)"
case "$out" in
*"MERGE CONFLICT"*) ;;
*) fail "expected MERGE CONFLICT report, got: $out" ;;
esac
[ ! -e "$B/.git/MERGE_HEAD" ] || fail "MERGE_HEAD still present: merge was not aborted"
unmerged="$(git -C "$B" diff --name-only --diff-filter=U || true)"
[ -z "$unmerged" ] || fail "unmerged paths remain after abort: $unmerged"
grep -q '<<<<<<<' "$B/file.txt" 2>/dev/null && fail "conflict markers left in file.txt"
staged="$(git -C "$B" diff --cached --name-only || true)"
[ -z "$staged" ] || fail "unexpected staged entries after abort: $staged"
before_push="$(git -C "$ORIGIN" rev-list --count main 2>/dev/null || echo 0)"

# A second sweep must not stage, commit, or push the conflict markers.
out2="$(cd "$ELSEWHERE" && bash "$SCRIPT" "$B" 2>&1)"
code2=$?
[ "$code2" -eq 1 ] || fail "second sweep: expected exit 1, got $code2 (out: $out2)"
after_push="$(git -C "$ORIGIN" rev-list --count main 2>/dev/null || echo 0)"
[ "$after_push" = "$before_push" ] || fail "origin gained commits across conflicted sweeps ($before_push -> $after_push)"
grep -q '<<<<<<<' "$B/file.txt" 2>/dev/null && fail "conflict markers appeared after second sweep"

if [ "$failures" -eq 0 ]; then echo "PASS: conflicted pull from foreign cwd aborts loudly, nothing staged or pushed"; fi
exit "$failures"
