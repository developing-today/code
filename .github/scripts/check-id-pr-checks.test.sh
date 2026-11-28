#!/usr/bin/env bash
# Contract tests for check-id-pr-checks.sh (#6881): the guard passes a
# well-formed list — including one with cosmetic trailing whitespace
# or CRLF line endings — and fails with the right cause named for a
# genuine unlisted check, a genuine stale entry, and a duplicated
# entry (reported as a duplicate, never as stale). All cases run
# through the ID_FLAKE_CHECKS_FILE test hook against a fixture flake
# set of alpha/beta/gamma; no flake evaluation happens.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GUARD="$SCRIPT_DIR/check-id-pr-checks.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

FIXTURE="$TMP/flake-checks.txt"
printf 'alpha\nbeta\ngamma\n' >"$FIXTURE"

RC=0
run_guard() { # run_guard <list-file>
  RC=0
  ID_FLAKE_CHECKS_FILE="$FIXTURE" bash "$GUARD" "$1" >"$TMP/out" 2>&1 || RC=$?
}

expect_pass() { # expect_pass <name>
  if [ "$RC" -ne 0 ]; then
    echo "FAIL: $1 should have passed the guard" >&2
    cat "$TMP/out" >&2
    exit 1
  fi
  echo "pass: $1 passes"
}

expect_fail() { # expect_fail <name>
  if [ "$RC" -eq 0 ]; then
    echo "FAIL: $1 should have failed the guard" >&2
    cat "$TMP/out" >&2
    exit 1
  fi
  echo "pass: $1 fails"
}

# 1. Control: an exact list passes, and the pass count is the number
# of distinct checks.
printf 'alpha\nbeta\ngamma\n' >"$TMP/control.txt"
run_guard "$TMP/control.txt"
expect_pass "control list"
grep -q "3 checks" "$TMP/out"
echo "pass: control pass output counts 3 distinct checks"

# 2. Genuine unlisted: gamma defined by the flake but absent from the
# list fails, naming gamma as not listed.
printf 'alpha\nbeta\n' >"$TMP/unlisted.txt"
run_guard "$TMP/unlisted.txt"
expect_fail "list missing a flake check"
grep -q "flake check 'gamma' is not in" "$TMP/out"
echo "pass: unlisted failure names gamma"

# 3. Genuine stale: delta listed but not defined by the flake fails,
# naming delta as no longer defined.
printf 'alpha\nbeta\ngamma\ndelta\n' >"$TMP/stale.txt"
run_guard "$TMP/stale.txt"
expect_fail "list with a stale entry"
grep -q "lists 'delta', which pkgs/id/flake.nix no longer defines" "$TMP/out"
echo "pass: stale failure names delta"

# 4. Duplicate: beta listed twice fails as a duplicate by name — and
# is NOT also misreported as a stale entry.
printf 'alpha\nbeta\nbeta\ngamma\n' >"$TMP/duplicate.txt"
run_guard "$TMP/duplicate.txt"
expect_fail "list with a duplicated entry"
grep -q "lists 'beta' more than once" "$TMP/out"
if grep -q "lists 'beta', which pkgs/id/flake.nix no longer defines" "$TMP/out"; then
  echo "FAIL: duplicate beta was also misreported as stale" >&2
  cat "$TMP/out" >&2
  exit 1
fi
echo "pass: duplicate failure names beta as duplicate, not stale"

# 5. Trailing/leading whitespace: cosmetic whitespace around entries
# passes, as the same well-formed set.
printf 'alpha \n  beta\n\tgamma\t\n' >"$TMP/whitespace.txt"
run_guard "$TMP/whitespace.txt"
expect_pass "list with cosmetic whitespace"
grep -q "3 checks" "$TMP/out"
echo "pass: whitespace list counts 3 distinct checks"

# 6. CRLF: a list saved with CRLF line endings passes, as the same
# well-formed set.
printf 'alpha\r\nbeta\r\ngamma\r\n' >"$TMP/crlf.txt"
run_guard "$TMP/crlf.txt"
expect_pass "list with CRLF line endings"
grep -q "3 checks" "$TMP/out"
echo "pass: CRLF list counts 3 distinct checks"

echo "check-id-pr-checks.test.sh: all contract tests passed"
