#!/usr/bin/env bash
# Regression test for issue #6180: the identity provider's static-file
# handler must confine every /static/... request to the static root.
#
# Exercises resolve_static_path — the same helper writeHttpResponse
# uses — both directly and through the handler, against a fixture tree:
#
#   <tmp>/.env                 (marker secret, two levels above static/)
#   <tmp>/provider/static/     (the static root, provider cwd = provider/)
#   <tmp>/provider/static/hello.txt
#   <tmp>/provider/static/escape.txt -> <tmp>/.env (symlink escape)
#
# Run from anywhere: bash pkgs/identity/provider/core-static-test.sh

PROVIDER_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
FIXTURE="$(mktemp -d)"
trap 'rm -rf "$FIXTURE"' EXIT

mkdir -p "$FIXTURE/provider/static"
printf 'TURSO_AUTH_TOKEN=MARKER-SECRET-6180\n' >"$FIXTURE/.env"
printf 'expected-bytes\n' >"$FIXTURE/provider/static/hello.txt"
ln -s "$FIXTURE/.env" "$FIXTURE/provider/static/escape.txt"

SESSION_HEADER_TO_BE_WRITTEN=""
cd "$FIXTURE/provider"
# shellcheck disable=SC1091
source "$PROVIDER_DIR/core.sh"

FAILURES=0
check() { # check <name> <condition:0/1>
  if [[ $2 -eq 0 ]]; then
    echo "ok - $1"
  else
    echo "FAIL - $1"
    FAILURES=$((FAILURES + 1))
  fi
}

serve() { # serve <request-path> -> handler output on stdout
  local REQUEST_PATH="$1"
  local REQUEST_METHOD="GET"
  writeHttpResponse
}

# --- the helper itself -------------------------------------------------
resolve_static_path "/static/hello.txt" >/dev/null
check "helper accepts a normal asset" $?

resolve_static_path "/static/../../.env" >/dev/null
[[ $? -eq 1 ]]
check "helper rejects ../ traversal as an escape (rc=1)" $?

resolve_static_path "/static/%2e%2e/%2e%2e/.env" >/dev/null
[[ $? -eq 1 ]]
check "helper rejects percent-encoded traversal as an escape (rc=1)" $?

resolve_static_path "/static/escape.txt" >/dev/null
[[ $? -eq 1 ]]
check "helper rejects a symlink pointing outside the root (rc=1)" $?

resolve_static_path "/static/nope.txt" >/dev/null
[[ $? -eq 2 ]]
check "helper reports a missing file as not-found (rc=2)" $?

# --- through the handler (the path-preserving client cases) ------------
OUT="$(serve "/static/../../.env")"
[[ $OUT == *"403"* && $OUT != *"MARKER-SECRET-6180"* ]]
check "handler: /static/../../.env gets 403 without the marker" $?

OUT="$(serve "/static/%2e%2e/%2e%2e/.env")"
[[ $OUT == *"403"* && $OUT != *"MARKER-SECRET-6180"* ]]
check "handler: encoded traversal gets 403 without the marker" $?

OUT="$(serve "/static/escape.txt")"
[[ $OUT == *"403"* && $OUT != *"MARKER-SECRET-6180"* ]]
check "handler: symlink escape gets 403 without the marker" $?

OUT="$(serve "/static/hello.txt")"
[[ $OUT == *"200"* && $OUT == *"expected-bytes"* ]]
check "handler: normal asset returns 200 with its expected bytes" $?

OUT="$(serve "/static/nope.txt")"
[[ $OUT == *"404"* ]]
check "handler: missing asset returns 404" $?

if [[ $FAILURES -gt 0 ]]; then
  echo "$FAILURES check(s) failed"
  exit 1
fi
echo "all checks passed"
