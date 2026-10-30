#!/usr/bin/env bash
# Seed the Pad workspace with the PM stack's planning content.
#
# Run on the host from this directory, after ./deploy.sh and the
# one-time bootstrap (pad init — see ../README.md). Uses the REST
# API with the minted automation token (SOPS: pad_api_token), so it
# does not depend on any CLI session state. Idempotent: items whose
# title already exists in the target collection are skipped.
set -euo pipefail

cd "$(dirname "$0")"
SECRETS_FILE="${PM_STACK_SECRETS:-$PWD/../../secrets/pm-stack.sops.yaml}"
export SOPS_AGE_KEY_FILE="${SOPS_AGE_KEY_FILE:-$HOME/.config/sops/age/keys.txt}"
BASE="http://127.0.0.1:7777/api/v1/workspaces/developing-today"

plain="$(sops --decrypt "$SECRETS_FILE")"
get() { printf '%s\n' "$plain" | sed -n "s/^$1: //p" | tr -d "\"'"; }
TOKEN="$(get pad_api_token)"
AUTH="Authorization: Bearer $TOKEN"

seed_item() {
  local collection="$1" title="$2" file="$3"
  if curl --fail --silent -H "$AUTH" "$BASE/collections/$collection/items" |
    jq -e --arg t "$title" '[.[] | select(.title == $t)] | length > 0' >/dev/null; then
    echo "skip (exists): [$collection] $title"
    return
  fi
  jq -n --arg t "$title" --rawfile c "$file" '{title: $t, content: $c}' |
    curl --fail --silent -H "$AUTH" -H 'Content-Type: application/json' \
      -X POST "$BASE/collections/$collection/items" -d @- >/dev/null
  echo "created: [$collection] $title"
}

seed_item plans "Project-management stack — design & phase plan" seed/plans-pm-stack-design.md
seed_item plans "Phase 1 — as-built (Kaneo execution board)" seed/plans-phase-1-as-built.md
seed_item plans "Phase 2 — plan (dispatch, planning, roadmap layers)" seed/plans-phase-2-plan.md
