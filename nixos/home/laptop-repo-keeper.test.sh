#!/usr/bin/env bash
# Exercises the real keeper against temporary primary repos: tracked
# residue must block snapshot commits, while clean trees still snapshot.
set -uo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$SELF_DIR/laptop-repo-keeper.sh"
[ -f "$SCRIPT" ] || {
  echo "FAIL: script not found at $SCRIPT" >&2
  exit 1
}

TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT
failures=0
fail() { echo "FAIL: $1" >&2; failures=$((failures + 1)); }
check() {
  if [ "$2" != "$3" ]; then fail "$1 — expected [$2], got [$3]"; fi
}

export GIT_AUTHOR_NAME="repo-keeper test"
export GIT_AUTHOR_EMAIL="test@example.invalid"
export GIT_COMMITTER_NAME="repo-keeper test"
export GIT_COMMITTER_EMAIL="test@example.invalid"
export GIT_CONFIG_COUNT=1
export GIT_CONFIG_KEY_0="commit.gpgsign"
export GIT_CONFIG_VALUE_0="false"

new_repo() { # $1 repository directory
  mkdir -p "$1"
  git init -q -b main "$1"
  printf 'before\n' >"$1/tracked.txt"
  git -C "$1" add -- tracked.txt
  git -C "$1" commit -q -m initial
}
run_keeper() { # $1 fixture HOME, $2 output log
  HOME="$1" bash "$SCRIPT" >"$2" 2>&1
}

# A tracked residue filename blocks the keeper's all-tracked snapshot,
# including the other ordinary tracked changes in the same tree.
H1="$TMP_ROOT/residue-home"
R1="$H1/code"
new_repo "$R1"
residue_paths=(
  'residue/file.bak'
  'residue/file.bak-2026-11-02'
  'residue/file.backup'
  'residue/.pre-warm-state'
  'residue/resume-build'
  'residue/proxy.mjs.new'
)
for path in "${residue_paths[@]}"; do
  mkdir -p "$R1/$(dirname "$path")"
  printf 'preserved fixture\n' >"$R1/$path"
done
git -C "$R1" add -- "${residue_paths[@]}"
git -C "$R1" commit -q -m 'add residue fixtures'
printf 'snapshot candidate\n' >>"$R1/tracked.txt"
run_keeper "$H1" "$TMP_ROOT/residue.log"

check 'residue case: no snapshot commit' '2' "$(git -C "$R1" rev-list --count HEAD)"
check 'residue case: ordinary tracked edit remains dirty' \
  'tracked.txt' "$(git -C "$R1" diff --name-only)"
grep -Fq -- 'TRACKED RESIDUE NOT committed' "$TMP_ROOT/residue.log" ||
  fail 'residue case: report missing tracked-residue refusal'
for path in "${residue_paths[@]}"; do
  grep -Fq -- "$path" "$TMP_ROOT/residue.log" ||
    fail "residue case: report missing $path"
done

# Without residue, the normal tracked-change snapshot still commits.
H2="$TMP_ROOT/clean-home"
R2="$H2/code"
new_repo "$R2"
printf 'snapshot candidate\n' >>"$R2/tracked.txt"
run_keeper "$H2" "$TMP_ROOT/clean.log"
check 'clean case: tracked snapshot committed' '2' "$(git -C "$R2" rev-list --count HEAD)"
grep -Fq -- 'committed 1 tracked entries on main' "$TMP_ROOT/clean.log" ||
  fail 'clean case: report missing successful snapshot'

if [ "$failures" -gt 0 ]; then
  echo "$failures failure(s)" >&2
  exit 1
fi
echo 'all laptop-repo-keeper tests passed'
