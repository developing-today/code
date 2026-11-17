#!/usr/bin/env bash
# Regression test for #10559: the oversized-tracked-file guard in
# laptop-autocommit.sh must refuse (exit 1, naming the file and its
# size) before staging/committing when a tracked modified file exceeds
# the ~95 MB limit, so the every-minute sweep can never re-enter the
# death spiral of pushing a blob GitHub will always reject. A normal
# small-change sweep must be unaffected.
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
R="$TMP_ROOT/repo"

git init -q -b main --bare "$ORIGIN"
git clone -q "$ORIGIN" "$R"
git -C "$R" commit -q --allow-empty -m base --no-verify
echo base >"$R/small.txt"
git -C "$R" add small.txt
git -C "$R" commit -q -m "add small.txt"
git -C "$R" push -q origin main

# Case 1: a normal small change is committed and pushed as today.
echo change >>"$R/small.txt"
out="$(bash "$SCRIPT" "$R" 2>&1)"
code=$?
[ "$code" -eq 0 ] || fail "small change: expected exit 0, got $code (out: $out)"
case "$out" in
*"committed tracked changes"*) ;;
*) fail "small change: expected a commit, got: $out" ;;
esac
ahead="$(git -C "$R" rev-list --count "@{u}..HEAD")"
[ "$ahead" = "0" ] || fail "small change: expected the commit to be pushed, ahead=$ahead"

# Case 2: an oversized tracked file, modified, refuses the whole cycle.
truncate -s 96000000 "$R/big.bin"
git -C "$R" add big.bin
git -C "$R" commit -q -m "add big.bin"
git -C "$R" push -q origin main
printf 'X' | dd of="$R/big.bin" bs=1 seek=50000000 conv=notrunc 2>/dev/null
echo more >>"$R/small.txt"
before="$(git -C "$R" rev-parse HEAD)"
out="$(bash "$SCRIPT" "$R" 2>&1)"
code=$?
[ "$code" -eq 1 ] || fail "oversized: expected exit 1, got $code (out: $out)"
case "$out" in
*"OVERSIZED tracked file"*"big.bin"*"96000000"*) ;;
*) fail "oversized: expected the named-file size error, got: $out" ;;
esac
after="$(git -C "$R" rev-parse HEAD)"
[ "$before" = "$after" ] || fail "oversized: a commit was made despite the guard"
staged="$(git -C "$R" diff --cached --name-only)"
[ -z "$staged" ] || fail "oversized: files were staged despite the guard: $staged"

if [ "$failures" -gt 0 ]; then
  echo "$failures failure(s)" >&2
  exit 1
fi
echo "laptop-autocommit oversized-guard checks passed"
