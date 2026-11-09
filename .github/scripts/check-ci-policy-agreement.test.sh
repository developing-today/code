#!/usr/bin/env bash
# Contract tests for check-ci-policy-agreement.sh (#7090, design #7089).
#
# Every case builds a throwaway policy, workflow set, waiver file and
# issue-state fixture, then asserts the checker's verdict. Oracle: the
# tier→trigger mapping stated in .github/ci-policy.md plus GitHub's
# workflow-trigger syntax. No network, no gh.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK="$SCRIPT_DIR/check-ci-policy-agreement.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

RC=0
OUT=""

# new_root NAME — create an isolated repo root and set ROOT.
new_root() {
  ROOT="$TMP/$1"
  mkdir -p "$ROOT/.github/workflows"
  : >"$ROOT/.github/ci-policy-waivers.txt"
  printf '6929 OPEN\n6953 OPEN\n6959 OPEN\n6981 OPEN\n' >"$ROOT/issue-states.txt"
}

# policy ROW... — write a minimal agreement table.
policy() {
  {
    echo '| workflow | basic | main | nightly | PR gate | waiver |'
    echo '| --- | --- | --- | --- | --- | --- |'
    for row in "$@"; do echo "$row"; done
  } >"$ROOT/.github/ci-policy.md"
}

# workflow NAME BODY — write .github/workflows/NAME.yml.
workflow() {
  local name="$1" body="$2"
  printf 'name: %s\n%s\n' "$name" "$body" >"$ROOT/.github/workflows/$name.yml"
}

# compliant NAME — a workflow satisfying basic (path-gated), main, nightly.
compliant() {
  local name="$1"
  workflow "$name" "on:
  pull_request:
    paths:
      - \"pkgs/$name/**\"
  push:
    branches: [main]
  schedule:
    - cron: \"0 0 * * *\"
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - run: true"
  policy "| \`.github/workflows/$name.yml\` | yes | yes | yes | \`pkgs/$name/**\` | - |"
}

run_check() {
  RC=0
  OUT="$(CI_POLICY_ROOT="$ROOT" CI_POLICY_ISSUE_STATE_FIXTURE="$ROOT/issue-states.txt" \
    bash "$CHECK" 2>&1)" || RC=$?
}

expect_pass() { # expect_pass NAME
  if [ "$RC" -ne 0 ]; then
    echo "FAIL: $1 should have passed" >&2
    printf '%s\n' "$OUT" >&2
    exit 1
  fi
  echo "pass: $1"
}

expect_fail() { # expect_fail NAME PATTERN
  if [ "$RC" -eq 0 ]; then
    echo "FAIL: $1 should have failed" >&2
    printf '%s\n' "$OUT" >&2
    exit 1
  fi
  if ! grep -qF -- "$2" <<<"$OUT"; then
    echo "FAIL: $1 failed without naming '$2'" >&2
    printf '%s\n' "$OUT" >&2
    exit 1
  fi
  echo "pass: $1"
}

# 1. Compliant tree, no waivers.
new_root ok
compliant alpha
run_check
expect_pass "compliant workflow passes"

# 2. workflow_dispatch alone satisfies no tier.
new_root dispatch
workflow alpha 'on:
  workflow_dispatch:
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - run: true'
policy '| `.github/workflows/alpha.yml` | yes | yes | no | `pkgs/alpha/**` | - |'
run_check
expect_fail "dispatch-only workflow fails" "alpha.yml missing basic tier"

# 3. Basic present but main trigger missing.
new_root no-main
workflow alpha 'on:
  pull_request:
    paths:
      - "pkgs/alpha/**"
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - run: true'
policy '| `.github/workflows/alpha.yml` | yes | yes | no | `pkgs/alpha/**` | - |'
run_check
expect_fail "missing push-to-main fails" "alpha.yml missing main tier"

# 4. push on a non-main branch does not satisfy main.
new_root wrong-branch
workflow alpha 'on:
  push:
    branches: [dev]
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - run: true'
policy '| `.github/workflows/alpha.yml` | no | yes | no | - | - |'
run_check
expect_fail "push to dev fails main" "alpha.yml missing main tier"

# 5. PR trigger without the promised path gate fails basic.
new_root no-gate
workflow alpha 'on:
  pull_request:
    paths:
      - "docs/**"
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - run: true'
policy '| `.github/workflows/alpha.yml` | yes | no | no | `pkgs/alpha/**` | - |'
run_check
expect_fail "wrong PR path gate fails basic" "alpha.yml missing basic tier"

# 6. Nightly promise with no schedule fails.
new_root no-nightly
workflow alpha 'on:
  pull_request:
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - run: true'
policy '| `.github/workflows/alpha.yml` | yes | no | yes | - | - |'
run_check
expect_fail "missing schedule fails nightly" "alpha.yml missing nightly tier"

# 7. A policy row naming a nonexistent workflow fails.
new_root ghost
policy '| `.github/workflows/ghost.yml` | yes | yes | no | - | - |'
run_check
expect_fail "policy row for a nonexistent workflow fails" "names .github/workflows/ghost.yml but the file does not exist"

# 8. A waiver covers a real disagreement and is reported, not silent.
new_root waived
workflow alpha 'on:
  workflow_dispatch:
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - run: true'
policy '| `.github/workflows/alpha.yml` | yes | no | no | - | #6929 |'
printf '.github/workflows/alpha.yml basic #6929\n' >"$ROOT/.github/ci-policy-waivers.txt"
run_check
expect_pass "waiver covers a real disagreement"
grep -q "WAIVED .github/workflows/alpha.yml basic (#6929)" <<<"$OUT" || {
  echo "FAIL: waiver was not reported" >&2
  printf '%s\n' "$OUT" >&2
  exit 1
}

# 9. A stale waiver on a compliant leg fails.
new_root stale
compliant alpha
printf '.github/workflows/alpha.yml basic #6929\n' >"$ROOT/.github/ci-policy-waivers.txt"
run_check
expect_fail "waiver on a compliant leg is stale" "stale waiver"

# 10. A waiver issue outside the Phase 2–3 set fails.
new_root unknown-issue
workflow alpha 'on:
  workflow_dispatch:
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - run: true'
policy '| `.github/workflows/alpha.yml` | yes | no | no | - | #1234 |'
printf '.github/workflows/alpha.yml basic #1234\n' >"$ROOT/.github/ci-policy-waivers.txt"
run_check
expect_fail "waiver naming an unrelated issue fails" "not one of the live Phase 2–3 issues"

# 11. A closed waiver issue fails.
new_root closed-issue
workflow alpha 'on:
  workflow_dispatch:
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - run: true'
policy '| `.github/workflows/alpha.yml` | yes | no | no | - | #6929 |'
printf '.github/workflows/alpha.yml basic #6929\n' >"$ROOT/.github/ci-policy-waivers.txt"
printf '6929 CLOSED\n' >"$ROOT/issue-states.txt"
run_check
expect_fail "closed waiver issue fails" "waiver issue #6929 is closed"

# 12. A missing issue number fails.
new_root no-issue
workflow alpha 'on:
  workflow_dispatch:
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - run: true'
policy '| `.github/workflows/alpha.yml` | yes | no | no | - | - |'
printf '.github/workflows/alpha.yml basic\n' >"$ROOT/.github/ci-policy-waivers.txt"
run_check
expect_fail "waiver without an issue fails" "waiver line has no issue number"

# 13. An unknown leg fails.
new_root bad-leg
compliant alpha
printf '.github/workflows/alpha.yml monthly #6929\n' >"$ROOT/.github/ci-policy-waivers.txt"
run_check
expect_fail "waiver with an unknown leg fails" "waiver line has unknown leg"

# 14. A flow-style on: block is rejected rather than guessed.
new_root inline
workflow alpha 'on: [push]
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - run: true'
policy '| `.github/workflows/alpha.yml` | no | yes | no | - | - |'
run_check
expect_fail "inline on: block fails loudly" "cannot parse inline on: entry"

# 15. Workflows ci-policy does not name are ignored.
new_root out-of-scope
compliant alpha
workflow beta 'on:
  workflow_dispatch:
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - run: true'
run_check
expect_pass "unnamed workflow is out of scope"

echo "check-ci-policy-agreement.test.sh: OK"
