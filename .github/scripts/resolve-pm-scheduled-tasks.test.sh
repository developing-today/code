#!/usr/bin/env bash
# Contract tests for pm-scheduled's task resolver (#5738).
set -euo pipefail

SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/resolve-pm-scheduled-tasks.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

run_resolver() {
  local schedule="$1" tasks_input="$2"
  RC=0
  if bash "$SCRIPT" "$schedule" "$tasks_input" >"$TMP/out" 2>"$TMP/err"; then
    RC=0
  else
    RC=$?
  fi
}

expect_success() {
  local name="$1" expected="$2" actual
  if [[ "$RC" -ne 0 ]]; then
    echo "FAIL: $name should succeed (exit $RC)" >&2
    cat "$TMP/err" >&2
    exit 1
  fi
  actual="$(cat "$TMP/out")"
  if [[ "$actual" != "$expected" ]]; then
    printf 'FAIL: %s output mismatch\nexpected:\n%s\nactual:\n%s\n' \
      "$name" "$expected" "$actual" >&2
    exit 1
  fi
  echo "pass: $name"
}

expect_invalid() {
  local name="$1" message="$2"
  if [[ "$RC" -eq 0 ]]; then
    echo "FAIL: $name should fail" >&2
    exit 1
  fi
  if [[ -s "$TMP/out" ]]; then
    echo "FAIL: $name produced partial output on invalid input" >&2
    cat "$TMP/out" >&2
    exit 1
  fi
  if ! grep -Fq -- "$message" "$TMP/err"; then
    echo "FAIL: $name did not explain the error: $message" >&2
    cat "$TMP/err" >&2
    exit 1
  fi
  echo "pass: $name rejects atomically"
}

run_resolver "unknown schedule" "bolt"
expect_success "valid single task" $'tasks=bolt\nhas_bolt=true\nhas_palette=false\nhas_suggestions=false'

run_resolver "unknown schedule" "bolt,palette"
expect_success "valid multiple tasks" $'tasks=bolt,palette\nhas_bolt=true\nhas_palette=true\nhas_suggestions=false'

run_resolver "unknown schedule" " bolt , palette "
expect_success "trims whitespace per token" $'tasks=bolt,palette\nhas_bolt=true\nhas_palette=true\nhas_suggestions=false'

run_resolver "unknown schedule" "no-such-task"
expect_invalid "unknown task" "unknown task token 'no-such-task'"

run_resolver "unknown schedule" "bolt,,palette"
expect_invalid "empty token" "empty task token at position 2"

run_resolver "unknown schedule" $'bolt, \t ,palette'
expect_invalid "whitespace-only token" "empty task token at position 2"

run_resolver "unknown schedule" "bolt,"
expect_invalid "trailing empty token" "empty task token at position 2"

run_resolver "unknown schedule" "bolt,not-a-task"
expect_invalid "mixed valid and invalid tasks" "unknown task token 'not-a-task'"

run_resolver "" ""
expect_success "empty input and schedule select all tasks" $'tasks=bolt,palette,suggestions\nhas_bolt=true\nhas_palette=true\nhas_suggestions=true'

run_resolver "" $' \t  '
expect_success "whitespace-only input falls back to all tasks" $'tasks=bolt,palette,suggestions\nhas_bolt=true\nhas_palette=true\nhas_suggestions=true'

run_resolver "not a cron schedule" ""
expect_success "unknown schedule falls back to all tasks" $'tasks=bolt,palette,suggestions\nhas_bolt=true\nhas_palette=true\nhas_suggestions=true'

run_resolver "0 13 * * *" ""
expect_success "known schedule mapping remains active" $'tasks=suggestions\nhas_bolt=false\nhas_palette=false\nhas_suggestions=true'

echo "resolve-pm-scheduled-tasks.test.sh: all contract tests passed"
