#!/usr/bin/env bash
# Contract tests for check-workflows.sh (#5743): the validator passes
# a well-formed workflow and a well-formed composite action, and
# fails — with a diagnostic — on a broken ${{ }} expression, on an
# unknown input to a well-known action, on action metadata with no
# runs section, and on action metadata with an unsupported runtime.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK="$SCRIPT_DIR/check-workflows.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

RC=0
run_check() { # run_check FILE [FILE..]
  RC=0
  bash "$CHECK" "$@" >"$TMP/out" 2>&1 || RC=$?
}

expect_pass() { # expect_pass <name>
  if [ "$RC" -ne 0 ]; then
    echo "FAIL: $1 should have passed check-workflows" >&2
    cat "$TMP/out" >&2
    exit 1
  fi
  echo "pass: $1 passes"
}

expect_fail() { # expect_fail <name>
  if [ "$RC" -eq 0 ]; then
    echo "FAIL: $1 should have failed check-workflows" >&2
    cat "$TMP/out" >&2
    exit 1
  fi
  echo "pass: $1 fails"
}

# 1. A well-formed workflow passes.
cat >"$TMP/valid-workflow.yml" <<'EOF'
name: demo
on:
  push:
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo "hello ${{ github.sha }}"
EOF
run_check "$TMP/valid-workflow.yml"
expect_pass "valid workflow"

# 2. A broken ${{ }} expression fails.
cat >"$TMP/bad-expression.yml" <<'EOF'
name: demo
on:
  push:
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo "${{ github.event..name }}"
EOF
run_check "$TMP/bad-expression.yml"
expect_fail "workflow with a broken expression"

# 3. An unknown input to a well-known action fails. (checkout is
# pinned to v4 here because that is the newest major in actionlint
# 1.7.7's bundled action database; newer majors are not input-checked
# offline.)
cat >"$TMP/bad-input.yml" <<'EOF'
name: demo
on:
  push:
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          not-a-real-input: x
EOF
run_check "$TMP/bad-input.yml"
expect_fail "workflow passing an unknown input to actions/checkout"

# 4. A well-formed composite action passes.
mkdir -p "$TMP/good-action"
cat >"$TMP/good-action/action.yml" <<'EOF'
name: demo-action
description: A well-formed composite action fixture.
inputs:
  who:
    description: Who to greet
    required: false
runs:
  using: composite
  steps:
    - run: echo "hello"
      shell: bash
EOF
run_check "$TMP/good-action/action.yml"
expect_pass "valid composite action"

# 5. Action metadata with no runs section fails.
mkdir -p "$TMP/no-runs-action"
cat >"$TMP/no-runs-action/action.yml" <<'EOF'
name: demo-action
description: Missing its runs section entirely.
inputs:
  who:
    description: Who to greet
    required: false
EOF
run_check "$TMP/no-runs-action/action.yml"
expect_fail "action metadata with no runs section"

# 6. Action metadata with an unsupported runtime fails.
mkdir -p "$TMP/bad-using-action"
cat >"$TMP/bad-using-action/action.yml" <<'EOF'
name: demo-action
description: Declares a runtime GitHub does not support.
runs:
  using: banana
  steps:
    - run: echo "hello"
      shell: bash
EOF
run_check "$TMP/bad-using-action/action.yml"
expect_fail "action metadata with runs.using: banana"

echo "check-workflows.test: all contract tests passed"
