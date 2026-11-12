#!/usr/bin/env bash
# Regression test for #1807: laptop-autocommit.sh must not finalize a
# user's in-progress merge or cherry-pick. The every-minute sweep used to
# check only index.lock, so a staged merge/cherry-pick resolution was
# committed as an "autocommit:" commit and pushed, clearing MERGE_HEAD /
# CHERRY_PICK_HEAD and losing the picked message. The sweep must exit 0
# and touch nothing while an operation is in progress, and must still
# commit + push an ordinary tracked change afterwards.
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
W="$TMP_ROOT/w"

git init -q -b main --bare "$ORIGIN"
git clone -q "$ORIGIN" "$W"
echo base >"$W/file.txt"
git -C "$W" add file.txt
git -C "$W" commit -q -m "add file.txt"
git -C "$W" push -q origin main

origin_count() { git -C "$ORIGIN" rev-list --count main; }

# --- Case 1: merge in progress with a staged resolution -----------------
git -C "$W" checkout -q -b side
echo side >"$W/file.txt"
git -C "$W" commit -qam "side change"
git -C "$W" checkout -q main
echo main >"$W/file.txt"
git -C "$W" commit -qam "main change"
git -C "$W" push -q origin main
git -C "$W" merge side >/dev/null 2>&1 || true
[ -e "$W/.git/MERGE_HEAD" ] || fail "setup: MERGE_HEAD missing after conflicted merge"
echo resolved >"$W/file.txt"
git -C "$W" add file.txt
head_before="$(git -C "$W" rev-parse HEAD)"
origin_before="$(origin_count)"

out="$(bash "$SCRIPT" "$W" 2>&1)"
code=$?
[ "$code" -eq 0 ] || fail "merge case: expected exit 0, got $code (out: $out)"
[ "$(git -C "$W" rev-parse HEAD)" = "$head_before" ] || fail "merge case: sweep created a commit"
[ -e "$W/.git/MERGE_HEAD" ] || fail "merge case: MERGE_HEAD cleared by sweep"
[ "$(origin_count)" = "$origin_before" ] || fail "merge case: sweep pushed"
git -C "$W" merge --abort

# --- Case 2: cherry-pick in progress with a staged resolution -----------
git -C "$W" checkout -q -b pick side
echo picked >"$W/file.txt"
git -C "$W" commit -qam "picked change"
git -C "$W" checkout -q main
git -C "$W" cherry-pick pick >/dev/null 2>&1 || true
[ -e "$W/.git/CHERRY_PICK_HEAD" ] || fail "setup: CHERRY_PICK_HEAD missing after conflicted cherry-pick"
echo resolved >"$W/file.txt"
git -C "$W" add file.txt
head_before="$(git -C "$W" rev-parse HEAD)"
origin_before="$(origin_count)"

out="$(bash "$SCRIPT" "$W" 2>&1)"
code=$?
[ "$code" -eq 0 ] || fail "cherry-pick case: expected exit 0, got $code (out: $out)"
[ "$(git -C "$W" rev-parse HEAD)" = "$head_before" ] || fail "cherry-pick case: sweep created a commit"
[ -e "$W/.git/CHERRY_PICK_HEAD" ] || fail "cherry-pick case: CHERRY_PICK_HEAD cleared by sweep"
[ "$(origin_count)" = "$origin_before" ] || fail "cherry-pick case: sweep pushed"
git -C "$W" cherry-pick --abort

# --- Case 3: ordinary tracked change still commits and pushes -----------
echo ordinary >"$W/file.txt"
origin_before="$(origin_count)"
out="$(bash "$SCRIPT" "$W" 2>&1)"
code=$?
[ "$code" -eq 0 ] || fail "ordinary case: expected exit 0, got $code (out: $out)"
[ "$(origin_count)" = "$((origin_before + 1))" ] || fail "ordinary case: change was not pushed"
git -C "$W" log -1 --format=%s | grep -q '^autocommit:' || fail "ordinary case: tip commit is not the autocommit"

if [ "$failures" -eq 0 ]; then echo "PASS: in-progress merge/cherry-pick untouched; ordinary change still autocommitted and pushed"; fi
exit "$failures"
