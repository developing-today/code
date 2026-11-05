#!/usr/bin/env bash
# Contract tests for check-mcpx-fix-test.sh (#1969): the guard fails a
# fix PR touching pkgs/mcpx/** that carries no test and no attested
# `No test: <reason>`, and passes every other shape.
set -euo pipefail

SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/check-mcpx-fix-test.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# run_case <name> <title> <linked-bug> <body> <file>...
# Writes the case inputs, runs the guard, leaves rc in $RC and the
# guard output in $TMP/out.
run_case() {
  local name="$1" title="$2" linked="$3" body="$4"
  shift 4
  printf '%s' "$body" >"$TMP/$name.body"
  : >"$TMP/$name.files"
  local f
  for f in "$@"; do echo "$f" >>"$TMP/$name.files"; done
  RC=0
  bash "$SCRIPT" --title "$title" --body-file "$TMP/$name.body" \
    --files-file "$TMP/$name.files" --linked-bug "$linked" \
    >"$TMP/out" 2>&1 || RC=$?
}

expect_fail() { # expect_fail <name>
  if [ "$RC" -eq 0 ]; then
    echo "FAIL: $1 should have failed the guard" >&2
    cat "$TMP/out" >&2
    exit 1
  fi
  echo "pass: $1 fails"
}

expect_pass() { # expect_pass <name>
  if [ "$RC" -ne 0 ]; then
    echo "FAIL: $1 should have passed the guard" >&2
    cat "$TMP/out" >&2
    exit 1
  fi
  echo "pass: $1 passes"
}

# 1. Fix title + mcpx production file + no test + no `No test:` -> fail.
run_case barefix "fix(mcpx): stop the daemon leaking sockets" false "" \
  "pkgs/mcpx/internal/daemon/registry.go"
expect_fail "fix title, no test"

# 2. Same PR with an attested skip -> pass, and the reason is visible.
run_case attested "fix(mcpx): stop the daemon leaking sockets" false \
  "No test: the leak only reproduces under the nightly stress harness" \
  "pkgs/mcpx/internal/daemon/registry.go"
expect_pass "fix title with a No test reason"
grep -q "No test: the leak only reproduces under the nightly stress harness" "$TMP/out"
echo "pass: No test reason is echoed in the check output"

# 3. `No test:` with an EMPTY reason is not an attestation -> fail.
run_case emptyreason "fix(mcpx): stop the daemon leaking sockets" false \
  "No test:" \
  "pkgs/mcpx/internal/daemon/registry.go"
expect_fail "fix title, empty No test reason"

# 4. Fix title + a Go regression test in the same PR -> pass.
run_case gotest "fix(mcpx): stop the daemon leaking sockets" false "" \
  "pkgs/mcpx/internal/daemon/registry.go" \
  "pkgs/mcpx/internal/daemon/registry_test.go"
expect_pass "fix title with a *_test.go change"

# 5. Fix title + a TypeScript test in the same PR -> pass.
run_case tstest "fix: plugin drops the session on exec" false "" \
  "pkgs/mcpx/plugin/opencode/mcpx/ops.ts" \
  "pkgs/mcpx/plugin/opencode/mcpx/ops.test.ts"
expect_pass "fix title with a *.test.ts change"

# 6. Non-fix PR touching mcpx, no test -> pass.
run_case docs "docs(mcpx): document the registry" false "" \
  "pkgs/mcpx/internal/daemon/registry.go"
expect_pass "non-fix mcpx PR"

# 7. Fix title but nothing under pkgs/mcpx/ -> pass (out of scope).
run_case otherpkg "fix(id): reject out-of-range dates" false "" \
  "pkgs/id/src/web/dates.rs"
expect_pass "fix PR outside pkgs/mcpx"

# 8. Plain title, but the body links a bug issue (workflow resolved
#    the label) and there is no test -> fail.
run_case linkedbug "mcpx: stop the daemon leaking sockets" true \
  "Fixes #4242" \
  "pkgs/mcpx/internal/daemon/registry.go"
expect_fail "bug-issue link, no test"

# 9. Same shape without the bug label on the linked issue -> pass.
run_case linkedfeature "mcpx: stop the daemon leaking sockets" false \
  "Fixes #4242" \
  "pkgs/mcpx/internal/daemon/registry.go"
expect_pass "non-bug issue link"

# 10. A test file OUTSIDE pkgs/mcpx/ does not satisfy an mcpx fix.
run_case outsidetest "fix(mcpx): stop the daemon leaking sockets" false "" \
  "pkgs/mcpx/internal/daemon/registry.go" \
  "pkgs/id/tests/registry_test.go"
expect_fail "test change outside pkgs/mcpx"

echo "check-mcpx-fix-test.test.sh: all contract tests passed"
