#!/usr/bin/env bash
# Failing-first enforcement for mcpx fix PRs (#1969, option C).
#
# Rule (pkgs/mcpx/docs/testing.md, "A test per bug"): every bug fix
# lands with at least one new test in the same PR, and the test fails
# without the fix. This check is the cheap, basic-tier enforcement of
# the presence half of that rule — it cannot prove a test fails
# without the fix (that stays a review item); it only proves a fix PR
# carries a test, or says why it does not.
#
# Match rules — a PR is in scope and is a "fix PR" when ALL of the
# scope rules and AT LEAST ONE of the fix rules match:
#
#   scope: at least one changed file is under pkgs/mcpx/**
#   fix 1: the PR title, lower-cased and trimmed, starts with "fix"
#          followed by one of ":", "(", "/", "!" or whitespace
#          (fix: ..., fix(mcpx): ..., fix(mcpx/daemon): ...)
#   fix 2: the PR body links a bug issue — a Fixes/Closes/Resolves
#          #<n> reference whose issue carries the `bug` or `kind/bug`
#          label. The workflow resolves the labels and passes the
#          result in as --linked-bug true; the script itself does no
#          network I/O, so its match logic stays shell-testable.
#
# A fix PR passes when ANY of these hold:
#   - it changes a test file under pkgs/mcpx/**: a *_test.go file or
#     a *.test.ts file, or
#   - its body contains a line `No test: <reason>` with a non-empty
#     reason. The reason is echoed in the check output (and in the
#     job summary when GITHUB_STEP_SUMMARY is set), so the skip is
#     attested in public, not silent.
#
# Anything else — a non-mcpx PR, a non-fix PR — passes untouched.
#
# Usage:
#   check-mcpx-fix-test.sh --title <title> --body-file <file> \
#     --files-file <file> [--linked-bug true|false]
# The file lists are one path per line (the workflow feeds them from
# the PR's changed-file list).
set -euo pipefail

TITLE=""
BODY_FILE=""
FILES_FILE=""
LINKED_BUG="false"

while [ $# -gt 0 ]; do
  case "$1" in
  --title)
    TITLE="$2"
    shift 2
    ;;
  --body-file)
    BODY_FILE="$2"
    shift 2
    ;;
  --files-file)
    FILES_FILE="$2"
    shift 2
    ;;
  --linked-bug)
    LINKED_BUG="$2"
    shift 2
    ;;
  *)
    echo "::error::check-mcpx-fix-test: unknown argument: $1" >&2
    exit 2
    ;;
  esac
done

if [ -z "$FILES_FILE" ] || [ ! -f "$FILES_FILE" ]; then
  echo "::error::check-mcpx-fix-test: --files-file <file> is required and must exist" >&2
  exit 2
fi
BODY=""
if [ -n "$BODY_FILE" ] && [ -f "$BODY_FILE" ]; then
  BODY="$(cat "$BODY_FILE")"
fi

summary() {
  # Make a line visible both in the log and, in CI, in the job summary.
  echo "$1"
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    echo "$1" >>"$GITHUB_STEP_SUMMARY"
  fi
}

# Scope: any changed file under pkgs/mcpx/**?
if ! grep -q '^pkgs/mcpx/' "$FILES_FILE"; then
  echo "mcpx-fix-test: OK - no changed file under pkgs/mcpx/, check not applicable (#1969)"
  exit 0
fi

# A test in the same PR: a *_test.go or *.test.ts file under pkgs/mcpx/.
HAS_TEST=0
while IFS= read -r file; do
  case "$file" in
  pkgs/mcpx/*_test.go | pkgs/mcpx/*.test.ts) HAS_TEST=1 ;;
  esac
done <"$FILES_FILE"

# Fix rule 1: title starts with "fix" (see header for the exact shape).
IS_FIX=0
FIX_WHY=""
lower_title="$(printf '%s' "$TITLE" | tr '[:upper:]' '[:lower:]' | sed -e 's/^[[:space:]]*//')"
case "$lower_title" in
fix:* | fix\(* | fix/* | fix\!* | fix\ * | fix)
  IS_FIX=1
  FIX_WHY="title starts with 'fix'"
  ;;
esac

# Fix rule 2: body links a bug issue (labels resolved by the workflow).
if [ "$LINKED_BUG" = "true" ]; then
  IS_FIX=1
  FIX_WHY="body links a bug-labelled issue (Fixes/Closes/Resolves)"
fi

if [ "$IS_FIX" -ne 1 ]; then
  echo "mcpx-fix-test: OK - not a fix PR (title does not start with 'fix', no bug-issue link), check not applicable (#1969)"
  exit 0
fi

if [ "$HAS_TEST" -eq 1 ]; then
  echo "mcpx-fix-test: OK - fix PR ($FIX_WHY) changes a *_test.go / *.test.ts file under pkgs/mcpx/ (#1969)"
  exit 0
fi

# The attested skip: `No test: <reason>` with a non-empty reason.
NO_TEST_REASON="$(printf '%s\n' "$BODY" | sed -n 's/^[[:space:]]*No test:[[:space:]]*//p' | head -n 1)"
if [ -n "$NO_TEST_REASON" ]; then
  summary "mcpx-fix-test: OK - fix PR ($FIX_WHY) carries no test, attested in the PR body: No test: $NO_TEST_REASON (#1969)"
  exit 0
fi

echo "::error::mcpx-fix-test: this looks like a fix PR ($FIX_WHY) touching pkgs/mcpx/**, but it changes no *_test.go or *.test.ts file under pkgs/mcpx/ and its body has no 'No test: <reason>' line. #1969 (pkgs/mcpx/docs/testing.md, 'A test per bug'): add the regression test in this PR, or add 'No test: <reason>' to the PR body saying why a test is genuinely expensive here."
exit 1
