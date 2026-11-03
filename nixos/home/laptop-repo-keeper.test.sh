#!/usr/bin/env bash
# Exercises the real keeper against temporary primary repos: tracked
# residue must block snapshot commits, while clean trees still snapshot.
# An untracked secret inside a wholly untracked directory (code#2268)
# must never be swept into a snapshot (status collapses it to `?? dir/`).
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
fail() {
  echo "FAIL: $1" >&2
  failures=$((failures + 1))
}
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

# A dirty primary that is BEHIND origin must not be snapshot-committed:
# the commit would land on a stale base, putting the primary ahead AND
# behind (diverged) at once, after which neither the keeper nor
# pm-git-sync can move it (code#4464).
ORIGIN_BARE="$TMP_ROOT/origin.git"
git init -q --bare -b main "$ORIGIN_BARE"
ADV="$TMP_ROOT/advancer"
git clone -q "$ORIGIN_BARE" "$ADV"
printf 'before\n' >"$ADV/tracked.txt"
git -C "$ADV" add -- tracked.txt
git -C "$ADV" commit -q -m initial
git -C "$ADV" push -q origin main

H3="$TMP_ROOT/behind-home"
R3="$H3/code"
git clone -q "$ORIGIN_BARE" "$R3"
printf 'upstream\n' >>"$ADV/tracked.txt"
git -C "$ADV" commit -q -am upstream
git -C "$ADV" push -q origin main
printf 'local edit\n' >>"$R3/tracked.txt"
run_keeper "$H3" "$TMP_ROOT/behind.log"
check 'behind case: no snapshot commit on stale base' '1' "$(git -C "$R3" rev-list --count HEAD)"
check 'behind case: tracked edit remains dirty' \
  'tracked.txt' "$(git -C "$R3" diff --name-only)"
grep -Fq -- 'NOT committed' "$TMP_ROOT/behind.log" ||
  fail 'behind case: report missing stale-base refusal'
grep -Fq -- 'behind origin/main' "$TMP_ROOT/behind.log" ||
  fail 'behind case: report missing behind count'

# The same dirty tree on an up-to-date base still snapshots and pushes.
H4="$TMP_ROOT/current-home"
R4="$H4/code"
git clone -q "$ORIGIN_BARE" "$R4"
printf 'local edit\n' >>"$R4/tracked.txt"
run_keeper "$H4" "$TMP_ROOT/current.log"
check 'current case: snapshot committed on up-to-date base' '3' "$(git -C "$R4" rev-list --count HEAD)"
check 'current case: snapshot pushed to origin' \
  "$(git -C "$R4" rev-parse HEAD)" "$(git --git-dir="$ORIGIN_BARE" rev-parse main)"

# A local-only repo has no upstream default to align to. In detached
# HEAD, skip it rather than falling back to main (which can move a clean
# checkout or produce an empty-branch warning for a dirty one).
H5="$TMP_ROOT/detached-clean-home"
R5="$H5/code"
new_repo "$R5"
git -C "$R5" checkout -q --detach HEAD
run_keeper "$H5" "$TMP_ROOT/detached-clean.log"
check 'detached clean case: remains detached' '' \
  "$(git -C "$R5" symbolic-ref --short -q HEAD || true)"
grep -Fq -- 'detached HEAD without origin — skipped' "$TMP_ROOT/detached-clean.log" ||
  fail 'detached clean case: report missing skip reason'
! grep -Fq -- 'primary was on  (clean)' "$TMP_ROOT/detached-clean.log" ||
  fail 'detached clean case: keeper attempted to move detached checkout'

H6="$TMP_ROOT/detached-dirty-home"
R6="$H6/code"
new_repo "$R6"
git -C "$R6" checkout -q --detach HEAD
printf 'preserve this edit\n' >>"$R6/tracked.txt"
run_keeper "$H6" "$TMP_ROOT/detached-dirty.log"
check 'detached dirty case: remains detached' '' \
  "$(git -C "$R6" symbolic-ref --short -q HEAD || true)"
check 'detached dirty case: tracked edit preserved' '1' \
  "$(git -C "$R6" diff --numstat | wc -l | tr -d ' ')"
grep -Fq -- 'detached HEAD without origin — skipped' "$TMP_ROOT/detached-dirty.log" ||
  fail 'detached dirty case: report missing skip reason'
! grep -Fq -- 'PRIMARY ON  ' "$TMP_ROOT/detached-dirty.log" ||
  fail 'detached dirty case: report contains an empty branch name'

# A secret inside a wholly untracked directory (code#2268) blocks the
# snapshot: default porcelain collapses it to `?? config/`, while the
# guard must inspect individual untracked paths.
H7="$TMP_ROOT/untracked-secret-home"
R7="$H7/code"
new_repo "$R7"
printf 'snapshot candidate\n' >>"$R7/tracked.txt"
mkdir -p "$R7/config"
printf 'synthetic fixture only; not a credential\n' >"$R7/config/credentials.json"
run_keeper "$H7" "$TMP_ROOT/untracked-secret.log"

check 'untracked-secret case: tracked snapshot is blocked' \
  '1' "$(git -C "$R7" rev-list --count HEAD)"
check 'untracked-secret case: tracked edit remains dirty' \
  'tracked.txt' "$(git -C "$R7" diff --name-only)"
check 'untracked-secret case: secret still untracked on disk' \
  '?? config/' "$(git -C "$R7" status --porcelain -- config)"
grep -Fq -- 'untracked file(s)' "$TMP_ROOT/untracked-secret.log" ||
  fail 'untracked-secret case: report missing untracked count'
grep -Fq -- 'secret-shaped path present' "$TMP_ROOT/untracked-secret.log" ||
  fail 'untracked-secret case: report missing secret-shaped-path refusal'

# An ordinary untracked directory does not trip the guard; the regular
# tracked snapshot still succeeds, without adding untracked files.
H8="$TMP_ROOT/untracked-ordinary-home"
R8="$H8/code"
new_repo "$R8"
printf 'snapshot candidate\n' >>"$R8/tracked.txt"
mkdir -p "$R8/notes"
printf 'ordinary fixture\n' >"$R8/notes/readme.txt"
run_keeper "$H8" "$TMP_ROOT/untracked-ordinary.log"
check 'untracked-ordinary case: tracked snapshot committed' \
  '2' "$(git -C "$R8" rev-list --count HEAD)"
check 'untracked-ordinary case: snapshot includes tracked file only' \
  'tracked.txt' "$(git -C "$R8" show HEAD --name-only --format='')"
check 'untracked-ordinary case: ordinary file remains untracked' \
  '?? notes/' "$(git -C "$R8" status --porcelain -- notes)"
grep -Fq -- 'committed 1 tracked entries on main' "$TMP_ROOT/untracked-ordinary.log" ||
  fail 'untracked-ordinary case: report missing successful snapshot'

if [ "$failures" -gt 0 ]; then
  echo "$failures failure(s)" >&2
  exit 1
fi
echo 'all laptop-repo-keeper tests passed'
