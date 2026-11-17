#!/usr/bin/env bash
# Contract tests for .github/actions/pm-dispatch-queue/post-task.sh (#8950):
# transient pm-dispatch failures (HTTP 502/503/504, transport errors) are
# retried only when the request is safe to repeat; everything else fails on
# the first attempt. A fake curl on PATH replays a scripted response sequence.
set -euo pipefail

SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../actions/pm-dispatch-queue" && pwd)/post-task.sh"
ACTION_YML="$(dirname "$SCRIPT")/action.yml"
WATCHER_YML="$(cd "$(dirname "${BASH_SOURCE[0]}")/../workflows" && pwd)/main-broken.yml"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/bin"

cat >"$TMP/bin/curl" <<'EOF'
#!/usr/bin/env bash
# Fake curl: each call consumes the next line of $FAKE_CURL_SEQ ("<code>|<body>"
# or "exit:<n>") and records the call count.
n=$(($(cat "$FAKE_CURL_STATE" 2>/dev/null || echo 0) + 1))
echo "$n" >"$FAKE_CURL_STATE"
line="$(sed -n "${n}p" "$FAKE_CURL_SEQ")"
[[ -n $line ]] || line="500|sequence exhausted"
case "$line" in
  exit:*)
    echo "curl: (28) Operation timed out" >&2
    exit "${line#exit:}"
    ;;
esac
printf '%s\n%s' "${line#*|}" "${line%%|*}"
EOF
chmod +x "$TMP/bin/curl"

# run_post RETRYABLE SEQ_LINE...  -> sets RC, attempts, out, err
run_post() {
  local retryable="$1"
  shift
  printf '%s\n' "$@" >"$TMP/seq"
  rm -f "$TMP/state"
  RC=0
  PATH="$TMP/bin:$PATH" FAKE_CURL_SEQ="$TMP/seq" FAKE_CURL_STATE="$TMP/state" \
    OIDC_TOKEN=tok PAYLOAD='{"repo":"r","dedupe_key":"test-run-step"}' RETRYABLE="$retryable" \
    PM_DISPATCH_RETRY_DELAY=0 PM_DISPATCH_MAX_ATTEMPTS=3 \
    bash "$SCRIPT" >"$TMP/out" 2>"$TMP/err" || RC=$?
  attempts="$(cat "$TMP/state" 2>/dev/null || echo 0)"
}

fail() {
  echo "FAIL: $1" >&2
  echo "--- stdout" >&2
  cat "$TMP/out" >&2
  echo "--- stderr" >&2
  cat "$TMP/err" >&2
  exit 1
}

expect() { # name rc attempts
  [[ $RC -eq $2 ]] || fail "$1: exit $RC, expected $2"
  [[ $attempts -eq $3 ]] || fail "$1: $attempts attempts, expected $3"
  echo "pass: $1"
}

run_post true '201|{"task_id":7}'
expect "success first try" 0 1
grep -Fq '{"task_id":7}' "$TMP/out" || fail "success body not printed"

run_post true '502|error code: 502' '201|{"task_id":8}'
expect "retryable 502 then success" 0 2
grep -Fq '{"task_id":8}' "$TMP/out" || fail "retry body not printed"
grep -Fq 'transient failure (HTTP 502, attempt 1/3)' "$TMP/err" || fail "no retry warning"

run_post true '503|unavailable' '504|timeout' '201|{"task_id":9}'
expect "retryable 503 and 504 then success" 0 3

run_post true 'exit:28' '201|{"task_id":10}'
expect "retryable transport error then success" 0 2

run_post true '502|a' '502|b' '502|c' '201|{"task_id":11}'
expect "retryable 502 exhausts attempts" 1 3
grep -Fq 'failed (HTTP 502, attempt 3/3)' "$TMP/err" || fail "no final failure message"
[[ ! -s "$TMP/out" ]] || fail "failure must not print a body on stdout"

run_post false '502|error code: 502' '201|{"task_id":12}'
expect "non-retryable 502 fails immediately" 1 1

rm -f "$TMP/state"
RC=0
PATH="$TMP/bin:$PATH" FAKE_CURL_SEQ="$TMP/seq" FAKE_CURL_STATE="$TMP/state" \
  OIDC_TOKEN=tok PAYLOAD='{"repo":"r"}' RETRYABLE=true \
  PM_DISPATCH_RETRY_DELAY=0 PM_DISPATCH_MAX_ATTEMPTS=3 \
  bash "$SCRIPT" >"$TMP/out" 2>"$TMP/err" || RC=$?
[[ $RC -eq 1 ]] || fail "retry without dedupe_key should fail"
[[ ! -e "$TMP/state" ]] || fail "missing dedupe_key must not make a request"
grep -Fq 'retries require a non-empty caller-supplied dedupe_key' "$TMP/err" || fail "missing dedupe_key error not explained"
echo "pass: retries require a caller-supplied dedupe key"

# Issue-creation payloads must never be retried: the server creates the
# tracking issue before deduplicating the task, so a lost response followed
# by a retry would create a duplicate issue (Codex P2 on PR #8952).
rm -f "$TMP/state"
printf '%s\n' '502|error code: 502' '201|{"task_id":90}' >"$TMP/seq"
RC=0
PATH="$TMP/bin:$PATH" FAKE_CURL_SEQ="$TMP/seq" FAKE_CURL_STATE="$TMP/state" \
  OIDC_TOKEN=test-token RETRYABLE=true PAYLOAD='{"repo":"r","dedupe_key":"test-run-step","issue":true}' \
  PM_DISPATCH_RETRY_DELAY=0 PM_DISPATCH_MAX_ATTEMPTS=3 \
  bash "$SCRIPT" >"$TMP/out" 2>"$TMP/err" || RC=$?
[[ $RC -eq 1 ]] || fail "issue-creation retry should fail"
[[ ! -e "$TMP/state" ]] || fail "issue-creation payload must not make a request when retries are requested"
grep -Fq 'retries require an existing issue_number or issue=false' "$TMP/err" || fail "issue-creation refusal not explained"
echo "pass: retries refused for issue-creation payloads"

# Attaching to an existing issue IS retryable: no issue is created.
rm -f "$TMP/state"
printf '%s\n' '502|error code: 502' '201|{"task_id":91}' >"$TMP/seq"
RC=0
PATH="$TMP/bin:$PATH" FAKE_CURL_SEQ="$TMP/seq" FAKE_CURL_STATE="$TMP/state" \
  OIDC_TOKEN=test-token RETRYABLE=true PAYLOAD='{"repo":"r","dedupe_key":"test-run-step","issue":true,"issue_number":8950}' \
  PM_DISPATCH_RETRY_DELAY=0 PM_DISPATCH_MAX_ATTEMPTS=3 \
  bash "$SCRIPT" >"$TMP/out" 2>"$TMP/err" || RC=$?
attempts="$(cat "$TMP/state" 2>/dev/null || echo 0)"
[[ $RC -eq 0 ]] || fail "existing-issue retry should succeed"
[[ $attempts -eq 2 ]] || fail "existing-issue retry: $attempts attempts, expected 2"
grep -Fq '{"task_id":91}' "$TMP/out" || fail "existing-issue retry body not printed"
echo "pass: retries allowed when attaching to an existing issue"

run_post true '400|bad request' '201|{"task_id":13}'
expect "retryable 400 is never retried" 1 1

run_post true '404|issue not found' '201|{"task_id":14}'
expect "retryable 404 is never retried" 1 1

run_post true '500|boom' '201|{"task_id":15}'
expect "retryable 500 is never retried" 1 1

grep -Fq 'INPUT_DEDUPE_KEY: ${{ inputs.dedupe_key }}' "$ACTION_YML" || fail "action does not wire the dedupe_key input"
grep -Fq "jq --arg dedupe_key \"\$INPUT_DEDUPE_KEY\" '. + {dedupe_key: \$dedupe_key}'" "$ACTION_YML" || fail "action does not include the dedupe_key in its POST payload"
grep -Fq 'dedupe_key: main-broken-run-${{ steps.identify.outputs.run_id }}-dispatch-append' "$WATCHER_YML" || fail "watcher append call does not supply the per-run dedupe_key (#9546)"
grep -Fq '[ -n "$INPUT_ISSUE_NUMBER" ] || [ "$INPUT_ISSUE" != "true" ]' "$ACTION_YML" || fail "action does not restrict retries to existing-issue or no-issue requests"
echo "pass: action restricts retries to existing-issue or no-issue requests"
echo "pass: watcher append supplies the per-run key that makes retries idempotent and dedupes repeat firings for one run (#9546)"

echo "pm-dispatch-queue-post.test: all contract tests passed"
