#!/usr/bin/env bash
set -euo pipefail

schema_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
migrations="$schema_dir/migrations"
up="$migrations/20261111_0001_orchestration_state.up.sql"
down="$migrations/20261111_0001_orchestration_state.down.sql"
container="pm-orch-schema-test-$$"

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is required for the disposable PostgreSQL migration test" >&2
  exit 1
fi
if docker container inspect "$container" >/dev/null 2>&1; then
  echo "refusing to reuse existing container: $container" >&2
  exit 1
fi

cleanup() {
  docker rm -f "$container" >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker run --detach --rm \
  --name "$container" \
  --env POSTGRES_HOST_AUTH_METHOD=trust \
  --env POSTGRES_DB=pm_orch_test \
  postgres:16-alpine >/dev/null

ready=false
for _ in $(seq 1 60); do
  if docker exec "$container" psql -X -qAt -U postgres -d pm_orch_test -c 'SELECT 1' >/dev/null 2>&1; then
    ready=true
    break
  fi
  sleep 1
done
if [[ "$ready" != true ]]; then
  echo "disposable PostgreSQL did not become ready" >&2
  exit 1
fi

psql_test() {
  docker exec -i "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d pm_orch_test "$@"
}

# Supabase's service_role carries BYPASSRLS; model that so the test proves
# the role can actually operate under the RLS model, not just hold grants.
psql_test -c 'CREATE ROLE service_role BYPASSRLS LOGIN;'
psql_test < "$up"

# A run row: the document's runId and version must match the columns.
psql_test -c "INSERT INTO public.orch_runs
  (run_id, generated_at, document_version, provenance, document, item_count)
  VALUES ('orch-2026-11-11T00:00:00Z', '2026-11-11T00:00:00Z', 1,
    '{\"sources\": [\"test\"]}',
    '{\"version\": 1, \"runId\": \"orch-2026-11-11T00:00:00Z\", \"tiers\": {\"needsDrew\": [], \"needsOrchestrator\": []}}',
    2);"

# One item per tier, in the shape the current orchestration.json carries.
psql_test -c "INSERT INTO public.orch_items
  (id, tier, kind, title, source, state, reason, ref, since, last_seen_at,
   item, first_seen_run_id, last_seen_run_id)
  VALUES ('item-drew-1', 'needsDrew', 'decision', 'Decide a thing',
    'test', 'open', 'Only Drew can resolve it', 'gh:developing-today/code#1',
    '2026-11-10T00:00:00Z', '2026-11-11T00:00:00Z',
    '{\"id\": \"item-drew-1\", \"kind\": \"decision\"}',
    'orch-2026-11-11T00:00:00Z', 'orch-2026-11-11T00:00:00Z');"
psql_test -c "INSERT INTO public.orch_items
  (id, tier, kind, title, source, state, reason, since, last_seen_at,
   item, first_seen_run_id, last_seen_run_id)
  VALUES ('item-orch-1', 'needsOrchestrator', 'stalled', 'Move a thing',
    'test', 'open', 'No movement in 48h',
    '2026-11-09T00:00:00Z', '2026-11-11T00:00:00Z',
    '{\"id\": \"item-orch-1\", \"kind\": \"stalled\"}',
    'orch-2026-11-11T00:00:00Z', 'orch-2026-11-11T00:00:00Z');"

privileges="$(psql_test -Atc "SELECT concat(
  (SELECT relrowsecurity::text FROM pg_class WHERE oid = 'public.orch_runs'::regclass), ':',
  (SELECT relrowsecurity::text FROM pg_class WHERE oid = 'public.orch_items'::regclass), ':',
  has_table_privilege('service_role', 'public.orch_runs', 'INSERT')::text, ':',
  has_table_privilege('service_role', 'public.orch_runs', 'UPDATE')::text, ':',
  has_table_privilege('service_role', 'public.orch_runs', 'DELETE')::text, ':',
  has_table_privilege('service_role', 'public.orch_items', 'DELETE')::text)")"
if [[ "$privileges" != "true:true:true:false:false:true" ]]; then
  echo "unexpected RLS/service_role privileges: $privileges" >&2
  exit 1
fi
echo "ok - RLS enabled; run log is append-only for service_role; state table is replaceable"

# The service_role must actually be able to operate under RLS (Supabase's
# service_role carries BYPASSRLS): append a run and read state back.
docker exec -i "$container" psql -X -v ON_ERROR_STOP=1 -U service_role -d pm_orch_test -c "INSERT INTO public.orch_runs
  (run_id, generated_at, document_version, provenance, document, item_count)
  VALUES ('orch-2026-11-11T01:00:00Z', '2026-11-11T01:00:00Z', 1,
    '{\"sources\": [\"test-2\"]}',
    '{\"version\": 1, \"runId\": \"orch-2026-11-11T01:00:00Z\", \"tiers\": {\"needsDrew\": [], \"needsOrchestrator\": []}}',
    0);"
svc_seen="$(docker exec -i "$container" psql -X -qAt -v ON_ERROR_STOP=1 -U service_role -d pm_orch_test -c "SELECT count(*) FROM public.orch_items")"
if [[ "$svc_seen" != "2" ]]; then
  echo "service_role could not read back the state rows under RLS: $svc_seen" >&2
  exit 1
fi
echo "ok - service_role can append runs and read state under RLS"

expect_rejected() {
  local label="$1"
  local sql="$2"
  if psql_test -c "$sql" >/dev/null 2>&1; then
    echo "expected database constraint rejection: $label" >&2
    exit 1
  fi
  echo "ok - rejected $label"
}

expect_rejected "run document with a mismatched runId" "INSERT INTO public.orch_runs
  (run_id, generated_at, document_version, provenance, document, item_count)
  VALUES ('orch-x', now(), 1, '{}', '{\"version\": 1, \"runId\": \"orch-y\"}', 0)"
expect_rejected "run document with a mismatched version" "INSERT INTO public.orch_runs
  (run_id, generated_at, document_version, provenance, document, item_count)
  VALUES ('orch-x', now(), 2, '{}', '{\"version\": 1, \"runId\": \"orch-x\"}', 0)"
expect_rejected "item in an unknown tier" "INSERT INTO public.orch_items
  (id, tier, kind, title, source, state, reason, since, last_seen_at,
   item, first_seen_run_id, last_seen_run_id)
  VALUES ('item-bad-tier', 'needsNobody', 'decision', 't', 'test', 'open', 'r',
    now(), now(), '{\"id\": \"item-bad-tier\"}',
    'orch-2026-11-11T00:00:00Z', 'orch-2026-11-11T00:00:00Z')"
expect_rejected "item whose document id differs from its key" "INSERT INTO public.orch_items
  (id, tier, kind, title, source, state, reason, since, last_seen_at,
   item, first_seen_run_id, last_seen_run_id)
  VALUES ('item-a', 'needsDrew', 'decision', 't', 'test', 'open', 'r',
    now(), now(), '{\"id\": \"item-b\"}',
    'orch-2026-11-11T00:00:00Z', 'orch-2026-11-11T00:00:00Z')"
expect_rejected "item pointing at a run that does not exist" "INSERT INTO public.orch_items
  (id, tier, kind, title, source, state, reason, since, last_seen_at,
   item, first_seen_run_id, last_seen_run_id)
  VALUES ('item-orphan', 'needsDrew', 'decision', 't', 'test', 'open', 'r',
    now(), now(), '{\"id\": \"item-orphan\"}',
    'orch-missing', 'orch-missing')"
expect_rejected "item last seen before it was first seen" "INSERT INTO public.orch_items
  (id, tier, kind, title, source, state, reason, since, last_seen_at,
   item, first_seen_run_id, last_seen_run_id)
  VALUES ('item-time-travel', 'needsDrew', 'decision', 't', 'test', 'open', 'r',
    '2026-11-11T00:00:00Z', '2026-11-10T00:00:00Z', '{\"id\": \"item-time-travel\"}',
    'orch-2026-11-11T00:00:00Z', 'orch-2026-11-11T00:00:00Z')"

# A valid item WITHOUT a state must import cleanly (#8856): TierItem.state
# is optional in the writer contract, and the contract corpus's decision
# item (aggregator/test/contract-corpus/v1/orchestration.json) omits it
# entirely. This insert mirrors that item's shape, with the state column
# left out (NULL), under the run row above.
psql_test -c "INSERT INTO public.orch_items
  (id, tier, kind, title, source, reason, ref, since, last_seen_at,
   item, first_seen_run_id, last_seen_run_id)
  VALUES ('example-owner/example-repo#812', 'needsDrew', 'decision',
    'Review an example design decision', 'backlog',
    'Scope requires an owner design decision.',
    'example-owner/example-repo#812',
    '2026-10-27T03:14:56Z', '2026-10-27T03:14:56Z',
    '{\"id\": \"example-owner/example-repo#812\", \"kind\": \"decision\"}',
    'orch-2026-11-11T00:00:00Z', 'orch-2026-11-11T00:00:00Z');"
echo "ok - state-less contract-corpus item imports (state is NULL)"

# Absent state is NULL; a present-but-blank state is still rejected.
expect_rejected "item with a blank state" "INSERT INTO public.orch_items
  (id, tier, kind, title, source, state, reason, since, last_seen_at,
   item, first_seen_run_id, last_seen_run_id)
  VALUES ('item-blank-state', 'needsDrew', 'decision', 't', 'test', '  ', 'r',
    now(), now(), '{\"id\": \"item-blank-state\"}',
    'orch-2026-11-11T00:00:00Z', 'orch-2026-11-11T00:00:00Z')"

# The per-run snapshot replace the writer contract prescribes: delete the
# old snapshot, insert the new run's rows, count parity with the run row.
psql_test -c "DELETE FROM public.orch_items;"
psql_test -c "INSERT INTO public.orch_items
  (id, tier, kind, title, source, state, reason, ref, since, last_seen_at,
   item, first_seen_run_id, last_seen_run_id)
  VALUES ('item-drew-1', 'needsDrew', 'decision', 'Decide a thing',
    'test', 'open', 'Only Drew can resolve it', 'gh:developing-today/code#1',
    '2026-11-10T00:00:00Z', '2026-11-11T01:00:00Z',
    '{\"id\": \"item-drew-1\", \"kind\": \"decision\"}',
    'orch-2026-11-11T00:00:00Z', 'orch-2026-11-11T01:00:00Z');"
echo "ok - snapshot replace carries first_seen_run_id forward"

if psql_test < "$down" >/dev/null 2>&1; then
  echo "down migration should refuse populated tables" >&2
  exit 1
fi
echo "ok - down migration protects non-empty tables"

psql_test -c 'DELETE FROM public.orch_items; DELETE FROM public.orch_runs;'
psql_test < "$down"
remaining="$(psql_test -Atc "SELECT (to_regclass('public.orch_runs') IS NULL AND to_regclass('public.orch_items') IS NULL)::text")"
if [[ "$remaining" != "true" ]]; then
  echo "down migration left schema objects behind" >&2
  exit 1
fi

echo "ok - orchestration migration constraints and rollback verified on disposable PostgreSQL 16"
