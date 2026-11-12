#!/usr/bin/env bash
# post-task.sh — POST one repo-task packet to pm-dispatch /v1/tasks (#8950).
#
# Used by the pm-dispatch-queue composite action. Prints the response body on
# stdout and exits 0 on a 2xx; prints the failure to stderr and exits 1
# otherwise.
#
# A transient edge/origin failure (curl transport error, HTTP 000/502/503/504)
# is retried with a fixed backoff when RETRYABLE=true. The caller sets
# RETRYABLE=true only when the payload carries a caller-supplied, invocation-
# stable dedupe_key. Without it, replaying a POST could enqueue a duplicate.
#
# Environment:
#   OIDC_TOKEN                 required  bearer token (audience pm-dispatch)
#   PAYLOAD                    required  JSON request body
#   RETRYABLE                  optional  "true" to retry transient failures (default false)
#   PM_DISPATCH_URL            optional  default https://dispatch.security.cab/v1/tasks
#   PM_DISPATCH_MAX_ATTEMPTS   optional  default 3 (only used when RETRYABLE=true)
#   PM_DISPATCH_RETRY_DELAY    optional  seconds between attempts, default 10
#   PM_DISPATCH_MAX_TIME       optional  per-attempt curl --max-time seconds, default 60
set -euo pipefail

url="${PM_DISPATCH_URL:-https://dispatch.security.cab/v1/tasks}"
: "${OIDC_TOKEN:?post-task: OIDC_TOKEN is required}"
: "${PAYLOAD:?post-task: PAYLOAD is required}"

max_attempts=1
if [ "${RETRYABLE:-false}" = "true" ]; then
  if ! printf '%s' "$PAYLOAD" | jq -e '(.dedupe_key | type == "string") and (.dedupe_key | length > 0)' >/dev/null; then
    echo "::error::pm-dispatch-queue: retries require a non-empty caller-supplied dedupe_key in the payload" >&2
    exit 1
  fi
  max_attempts="${PM_DISPATCH_MAX_ATTEMPTS:-3}"
fi
delay="${PM_DISPATCH_RETRY_DELAY:-10}"
max_time="${PM_DISPATCH_MAX_TIME:-60}"

attempt=1
while :; do
  curl_status=0
  response="$(curl -sS --connect-timeout 15 --max-time "$max_time" -w $'\n%{http_code}' -X POST "$url" \
    -H "Authorization: Bearer $OIDC_TOKEN" \
    -H "Content-Type: application/json" \
    -d "$PAYLOAD")" || curl_status=$?
  if [ "$curl_status" -eq 0 ]; then
    http_code="${response##*$'\n'}"
    body="${response%$'\n'*}"
  else
    http_code=000
    body="curl exited with status $curl_status"
  fi

  if [ "$http_code" -ge 200 ] && [ "$http_code" -lt 300 ]; then
    echo "$body"
    exit 0
  fi

  case "$http_code" in
    000 | 502 | 503 | 504) transient=true ;;
    *) transient=false ;;
  esac

  if [ "$transient" = "true" ] && [ "$attempt" -lt "$max_attempts" ]; then
    echo "::warning::pm-dispatch-queue: pm-dispatch POST /v1/tasks transient failure (HTTP $http_code, attempt $attempt/$max_attempts); retrying in ${delay}s" >&2
    echo "$body" >&2
    sleep "$delay"
    attempt=$((attempt + 1))
    continue
  fi

  echo "::error::pm-dispatch-queue: pm-dispatch POST /v1/tasks failed (HTTP $http_code, attempt $attempt/$max_attempts):" >&2
  echo "$body" >&2
  exit 1
done
