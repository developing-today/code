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

# Property tests use a fixed-seed generator so failures are reproducible.
# Oracle: successful input must emit the normalized token sequence and exact
# membership flags; inserting one invalid token must fail before stdout.
property_seed=5738
seed=$property_seed
property_random=0
property_rand() {
  seed=$(((seed * 1103515245 + 12345) % 2147483648))
  property_random=$seed
}

valid_names=(bolt palette suggestions)
whitespace=("" " " $'\t' "  " $' \t')
valid_property_cases=1000
for ((case_no = 1; case_no <= valid_property_cases; case_no++)); do
  property_rand
  token_count=$((property_random % 6 + 1))
  input=""
  expected_tasks=""
  expected_bolt=false
  expected_palette=false
  expected_suggestions=false

  for ((token_no = 0; token_no < token_count; token_no++)); do
    if ((token_no > 0)); then
      input+=,
      expected_tasks+=,
    fi

    property_rand
    token="${valid_names[$((property_random % ${#valid_names[@]}))]}"
    property_rand
    before="${whitespace[$((property_random % ${#whitespace[@]}))]}"
    property_rand
    after="${whitespace[$((property_random % ${#whitespace[@]}))]}"
    input+="${before}${token}${after}"
    expected_tasks+="$token"
    case "$token" in
      bolt) expected_bolt=true ;;
      palette) expected_palette=true ;;
      suggestions) expected_suggestions=true ;;
    esac
  done

  run_resolver "unknown schedule" "$input"
  expected=$'tasks='"$expected_tasks"$'\nhas_bolt='"$expected_bolt"$'\nhas_palette='"$expected_palette"$'\nhas_suggestions='"$expected_suggestions"
  actual="$(cat "$TMP/out")"
  if [[ "$RC" -ne 0 || "$actual" != "$expected" ]]; then
    printf 'FAIL: valid generated case %d violated normalization/flag oracle\ninput: %q\nexpected:\n%s\nactual:\n%s\n' \
      "$case_no" "$input" "$expected" "$actual" >&2
    cat "$TMP/err" >&2
    exit 1
  fi
done
echo "pass: $valid_property_cases generated valid lists match normalization/flag oracle (seed $property_seed)"

invalid_tokens=("not-a-task" "" $' \t ' "suggestion")
invalid_property_cases=1000
for ((case_no = 1; case_no <= invalid_property_cases; case_no++)); do
  property_rand
  valid_count=$((property_random % 5 + 1))
  property_rand
  invalid_position=$((property_random % (valid_count + 1)))
  property_rand
  invalid_token="${invalid_tokens[$((property_random % ${#invalid_tokens[@]}))]}"
  input=""

  for ((token_no = 0; token_no <= valid_count; token_no++)); do
    if ((token_no > 0)); then
      input+=,
    fi
    if ((token_no == invalid_position)); then
      input+="$invalid_token"
    else
      property_rand
      token="${valid_names[$((property_random % ${#valid_names[@]}))]}"
      property_rand
      before="${whitespace[$((property_random % ${#whitespace[@]}))]}"
      property_rand
      after="${whitespace[$((property_random % ${#whitespace[@]}))]}"
      input+="${before}${token}${after}"
    fi
  done

  run_resolver "unknown schedule" "$input"
  if [[ "$RC" -eq 0 || -s "$TMP/out" ]]; then
    printf 'FAIL: generated invalid case %d was accepted or emitted partial stdout\ninput: %q\nexit: %d\nstdout:\n' \
      "$case_no" "$input" "$RC" >&2
    cat "$TMP/out" >&2
    cat "$TMP/err" >&2
    exit 1
  fi
done
echo "pass: $invalid_property_cases generated invalid-token insertions reject atomically (seed $property_seed)"

echo "resolve-pm-scheduled-tasks.test.sh: all contract tests passed"
