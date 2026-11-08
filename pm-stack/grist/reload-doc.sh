#!/usr/bin/env bash
# Force Grist to close and reopen a document, then verify one cell by API.
#
# Without --apply, the expected value should have been written by the
# external document editor before this script is run. With --apply, this
# script first applies one idempotent UpdateRecord action through Grist's
# supported API, then reloads and verifies it.
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage:
  ./reload-doc.sh DOC_ID TABLE_ID ROW_ID COLUMN_ID EXPECTED_VALUE [--apply]

Without --apply, reload and verify an externally-applied change.
With --apply, set the named cell to EXPECTED_VALUE through the Grist API
before reloading. Re-running --apply with the same value is a no-op.
EOF
  exit 2
}

[[ $# -eq 5 || $# -eq 6 ]] || usage
DOC_ID=$1
TABLE_ID=$2
ROW_ID=$3
COLUMN_ID=$4
EXPECTED_VALUE=$5
APPLY=false
if [[ $# -eq 6 ]]; then
  [[ $6 == --apply ]] || usage
  APPLY=true
fi

[[ ${DOC_ID} =~ ^[A-Za-z0-9_-]+$ ]] || usage
[[ ${TABLE_ID} =~ ^[A-Za-z0-9_-]+$ ]] || usage
[[ ${ROW_ID} =~ ^[1-9][0-9]*$ ]] || usage
[[ ${COLUMN_ID} =~ ^[A-Za-z0-9_-]+$ ]] || usage

cd "$(dirname "$0")"
SECRETS_FILE="${PM_STACK_SECRETS:-${PWD}/../../secrets/pm-stack.sops.yaml}"
export SOPS_AGE_KEY_FILE="${SOPS_AGE_KEY_FILE:-${HOME}/.config/sops/age/keys.txt}"
BASE="http://127.0.0.1:8484"

plain="$(sops --decrypt "${SECRETS_FILE}")"
API_KEY="$(printf '%s\n' "${plain}" | sed -n 's/^grist_api_key: //p' | tr -d "\"'")"
unset plain
if [[ -z ${API_KEY} ]]; then
  echo "grist_api_key missing from ${SECRETS_FILE}" >&2
  exit 1
fi
AUTH="Authorization: Bearer ${API_KEY}"
unset API_KEY

api() {
  curl --fail --silent --show-error --max-time 30 \
    -H "${AUTH}" -H 'Content-Type: application/json' "$@"
}

records_url="${BASE}/api/docs/${DOC_ID}/tables/${TABLE_ID}/records"
matches_expected() {
  jq -e --argjson row "${ROW_ID}" --arg column "${COLUMN_ID}" \
    --arg expected "${EXPECTED_VALUE}" \
    'any(.records[]; .id == $row and .fields[$column] == $expected)' \
    >/dev/null
}

if [[ ${APPLY} == true ]]; then
  current="$(api "${records_url}")"
  # A mismatch is expected when this option needs to apply its update.
  # shellcheck disable=SC2310
  if printf '%s' "${current}" | matches_expected; then
    echo "apply: cell already has requested value (no-op)"
  else
    action="$(jq -cn --arg table "${TABLE_ID}" --argjson row "${ROW_ID}" \
      --arg column "${COLUMN_ID}" --arg value "${EXPECTED_VALUE}" \
      '[["UpdateRecord", $table, $row, {($column): $value}]]')"
    api -X POST "${BASE}/api/docs/${DOC_ID}/apply" -d "${action}" >/dev/null
    echo "apply: updated requested cell through the Grist API"
  fi
fi

api -X POST "${BASE}/api/docs/${DOC_ID}/force-reload" >/dev/null
records="$(api "${records_url}")"
# A mismatch is the verification failure handled below.
# shellcheck disable=SC2310
if ! printf '%s' "${records}" | matches_expected; then
  echo "verification failed: ${TABLE_ID} row ${ROW_ID} column ${COLUMN_ID} did not match after reload" >&2
  exit 1
fi

echo "verified: ${DOC_ID}/${TABLE_ID} row ${ROW_ID} column ${COLUMN_ID} matches after force-reload"
