#!/usr/bin/env bash
# Contract tests for the source snapshot guard (#6887).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
CHECK="$SCRIPT_DIR/check-id-source-snapshot.sh"
TMP="$(mktemp -d)"
trap 'rm -rf -- "$TMP"' EXIT

RC=0
run_check() { # run_check [arguments...]
  RC=0
  bash "$CHECK" "$@" >"$TMP/out" 2>&1 || RC=$?
}

expect_pass() { # expect_pass <name>
  if [ "$RC" -ne 0 ]; then
    echo "FAIL: $1 should have passed" >&2
    cat "$TMP/out" >&2
    exit 1
  fi
}

expect_fail() { # expect_fail <name> <diagnostic>
  if [ "$RC" -eq 0 ]; then
    echo "FAIL: $1 should have failed" >&2
    exit 1
  fi
  if ! grep -Fq -- "$2" "$TMP/out"; then
    echo "FAIL: $1 did not name '$2'" >&2
    cat "$TMP/out" >&2
    exit 1
  fi
}

ROOT="$TMP/repo"
mkdir -p "$ROOT/pkgs/id/sub"
git -C "$ROOT" init -q
git -C "$ROOT" config user.name fixture
git -C "$ROOT" config user.email fixture@example.invalid
printf 'alpha\n' >"$ROOT/pkgs/id/a.txt"
printf 'beta\n' >"$ROOT/pkgs/id/sub/b.txt"
git -C "$ROOT" add pkgs/id
git -C "$ROOT" commit -qm fixture

# Absolute and relative root/manifest arguments produce identical bytes.
run_check snapshot "$ROOT" "$TMP/absolute.manifest"
expect_pass "absolute-root snapshot"
(
  cd "$TMP"
  bash "$CHECK" snapshot repo relative.manifest >"$TMP/relative.out" 2>&1
)
cmp "$TMP/absolute.manifest" "$TMP/relative.manifest"
[ "$(wc -l <"$TMP/absolute.manifest")" -eq 2 ]

# A clean source copy verifies, while real content drift is still named.
cp -R "$ROOT/pkgs/id" "$TMP/source"
run_check verify "$ROOT" "$TMP/absolute.manifest" "$TMP/source"
expect_pass "matching source copy"
printf 'changed\n' >"$TMP/source/a.txt"
run_check verify "$ROOT" "$TMP/absolute.manifest" "$TMP/source"
expect_fail "genuine source mismatch" "a.txt"

# Empty tracked sets are rejected in both the git and digest-tree branches.
EMPTY_GIT="$TMP/empty-git"
mkdir -p "$EMPTY_GIT/pkgs/id"
git -C "$EMPTY_GIT" init -q
run_check snapshot "$EMPTY_GIT" "$TMP/empty-git.manifest"
expect_fail "empty git manifest" "recorded no files"
[ ! -e "$TMP/empty-git.manifest" ]

EMPTY_TREE="$TMP/empty-tree"
mkdir -p "$EMPTY_TREE/pkgs/id"
run_check snapshot "$EMPTY_TREE" "$TMP/empty-tree.manifest"
expect_fail "empty digest-tree manifest" "recorded no files"
[ ! -e "$TMP/empty-tree.manifest" ]

: >"$TMP/empty.manifest"
run_check verify "$ROOT" "$TMP/empty.manifest" "$TMP/source"
expect_fail "empty verify manifest" "manifest $TMP/empty.manifest is empty"

# Missing tracked files fail with context and leave no partial manifest.
rm "$ROOT/pkgs/id/sub/b.txt"
run_check snapshot "$ROOT" "$TMP/missing.manifest"
expect_fail "missing tracked file" "tracked file missing from worktree: $ROOT/pkgs/id/sub/b.txt"
[ ! -e "$TMP/missing.manifest" ]

echo "check-id-source-snapshot.test: all contract tests passed"
