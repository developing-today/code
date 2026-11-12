#!/usr/bin/env bash
# Seed the Grist roadmap document from seed/roadmap.json.
#
# Run on the host from this directory, after ./deploy.sh and the
# one-time account bootstrap (see ../README.md). Idempotent: if the
# Roadmap table already has rows, nothing is added.
#
# The API key is read from SOPS (grist_api_key) — it is the Grist
# admin account's API key, also filed in Bitwarden + 1Password.
set -euo pipefail

cd "$(dirname "$0")"
if ! command -v bun >/dev/null 2>&1; then
  echo "bun is required to validate roadmap Horizon values before seeding" >&2
  exit 1
fi
bun run validate-roadmap-seed.ts
SECRETS_FILE="${PM_STACK_SECRETS:-$PWD/../../secrets/pm-stack.sops.yaml}"
export SOPS_AGE_KEY_FILE="${SOPS_AGE_KEY_FILE:-$HOME/.config/sops/age/keys.txt}"
BASE="http://127.0.0.1:8484"

plain="$(sops --decrypt "$SECRETS_FILE")"
get() { printf '%s\n' "$plain" | sed -n "s/^$1: //p" | tr -d "\"'"; }
API_KEY="$(get grist_api_key)"
AUTH="Authorization: Bearer $API_KEY"

api() { curl --fail --silent -H "$AUTH" -H 'Content-Type: application/json' "$@"; }

# 1. Org + workspace + document (create what is missing).
# Note: Grist's create endpoints return the new id as a bare number,
# not an object — normalize both shapes.
ORG_ID="$(api "$BASE/api/orgs" | jq -r '.[0].id')"
WS_JSON="$(api "$BASE/api/orgs/$ORG_ID/workspaces")"
WS_ID="$(printf '%s' "$WS_JSON" |
  jq -r '.[] | select(.name == "Roadmap & registers") | .id' | head -1)"
if [[ -z $WS_ID ]]; then
  WS_ID="$(api -X POST "$BASE/api/orgs/$ORG_ID/workspaces" \
    -d '{"name": "Roadmap & registers"}' | jq -r 'if type == "object" then .id else . end')"
  echo "created workspace $WS_ID"
  WS_JSON="$(api "$BASE/api/orgs/$ORG_ID/workspaces")"
fi
# There is no GET /workspaces/{id}/docs — docs ride inside the
# workspace objects of the org workspaces list.
DOC_ID="$(printf '%s' "$WS_JSON" |
  jq -r '.[] | select(.name == "Roadmap & registers") | .docs[] |
         select(.name == "Program roadmap") | .id' | head -1)"
if [[ -z $DOC_ID ]]; then
  DOC_ID="$(api -X POST "$BASE/api/workspaces/$WS_ID/docs" \
    -d '{"name": "Program roadmap"}' | jq -r 'if type == "object" then .id else . end')"
  echo "created document $DOC_ID"
fi
echo "document: $DOC_ID"

# 2. Table (create from seed/roadmap.json if absent).
if ! api "$BASE/api/docs/$DOC_ID/tables" | jq -e '.tables[] | select(.id == "Roadmap")' >/dev/null; then
  # This Grist version's /apply takes the bare UserAction array as
  # the request body (not an {"actions": ...} wrapper).
  jq -c '[["AddTable", .table,
    [.columns[] | {id, type: (if .type == "Choice" then "Choice" else .type end),
                   isFormula: false}
                  + (if .choices then {widgetOptions: ({choices: .choices} | @json)} else {} end)]]]' \
    seed/roadmap.json |
    api -X POST "$BASE/api/docs/$DOC_ID/apply" -d @- >/dev/null
  echo "created table Roadmap"
fi

# 3. Rows (only into an empty table).
COUNT="$(api "$BASE/api/docs/$DOC_ID/tables/Roadmap/records" | jq '.records | length')"
if [[ $COUNT -gt 0 ]]; then
  echo "skip: Roadmap already has $COUNT rows"
  exit 0
fi
jq -c '(.rows | length) as $n | .rows as $rows |
  [["BulkAddRecord", .table,
    [range(1; $n + 1)],
    (reduce .columns[].id as $c ({}; .[$c] = [$rows[] | .[$c]]))]]' \
  seed/roadmap.json |
  api -X POST "$BASE/api/docs/$DOC_ID/apply" -d @- >/dev/null
echo "seeded $(jq '.rows | length' seed/roadmap.json) rows into Roadmap"
