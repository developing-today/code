#!/usr/bin/env bash
set -euo pipefail

schema_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
migrations="$schema_dir/migrations"
up="$migrations/20261105_0001_action_event_store.up.sql"
down="$migrations/20261105_0001_action_event_store.down.sql"
container="pm-action-schema-test-$$"

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
  --env POSTGRES_DB=pm_actions_test \
  postgres:16-alpine >/dev/null

ready=false
for _ in $(seq 1 60); do
  if docker exec "$container" psql -X -qAt -U postgres -d pm_actions_test -c 'SELECT 1' >/dev/null 2>&1; then
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
  docker exec -i "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d pm_actions_test "$@"
}

# Supabase's service_role carries BYPASSRLS; model that so the test proves
# the role can actually operate under the RLS model, not just hold grants.
psql_test -c 'CREATE ROLE service_role BYPASSRLS LOGIN;'
psql_test < "$up"

# A pre-token action is valid, as are modern actions and the source's JSON
# object payload/author contracts.
psql_test -c "INSERT INTO public.agg_actions
  (id, kind, verb, object_ref, payload, author, created_at)
  VALUES ('da-legacy', 'triage', 'acknowledge', 'orch:legacy', '{}',
    '{\"kind\":\"drew\",\"id\":\"drew\",\"label\":\"Drew\"}', now());"
psql_test -c "INSERT INTO public.agg_actions
  (id, client_token, kind, verb, object_ref, payload, author, created_at)
  VALUES ('da-current', 'ct-current-12345678', 'triage', 'acknowledge',
    'orch:item-1', '{}',
    '{\"kind\":\"agent\",\"id\":\"session-1\",\"label\":\"agent\"}', now());"
psql_test -c "INSERT INTO public.agg_actions
  (id, kind, verb, object_ref, payload, author, created_at)
  VALUES ('da-legacy-2', 'triage', 'acknowledge', 'orch:legacy-2', '{}',
    '{\"kind\":\"drew\",\"id\":\"drew\",\"label\":\"Drew\"}', now());"

privileges="$(psql_test -Atc "SELECT concat(
  (SELECT relrowsecurity::text FROM pg_class WHERE oid = 'public.agg_actions'::regclass), ':',
  (SELECT relrowsecurity::text FROM pg_class WHERE oid = 'public.shared_events'::regclass), ':',
  has_table_privilege('service_role', 'public.agg_actions', 'UPDATE')::text, ':',
  has_table_privilege('service_role', 'public.agg_actions', 'DELETE')::text, ':',
  has_table_privilege('service_role', 'public.shared_events', 'DELETE')::text)")"
if [[ "$privileges" != "true:true:true:false:true" ]]; then
  echo "unexpected RLS/service_role privileges: $privileges" >&2
  exit 1
fi
echo "ok - RLS and service_role privileges"

# The service_role must actually be able to operate under RLS (Supabase's
# service_role carries BYPASSRLS); a role that only holds grants but is
# blocked by RLS would pass the privilege probe above while being
# unusable in production.
svc_hash="$(printf '%064d' 7)"
docker exec -i "$container" psql -X -v ON_ERROR_STOP=1 -U service_role -d pm_actions_test -c "INSERT INTO public.shared_events
  (id, event_at, actor, kind, ref, summary, prev_hash, hash)
  VALUES ('event-service-role', now(), 'run-svc', 'comment', 'orch:item-1',
    'service_role write probe', '$svc_hash', '$svc_hash');"
svc_seen="$(docker exec -i "$container" psql -X -qAt -v ON_ERROR_STOP=1 -U service_role -d pm_actions_test -c "SELECT count(*) FROM public.shared_events WHERE id = 'event-service-role'")"
if [[ "$svc_seen" != "1" ]]; then
  echo "service_role could not read back its own event row under RLS" >&2
  exit 1
fi
docker exec -i "$container" psql -X -v ON_ERROR_STOP=1 -U service_role -d pm_actions_test -c "DELETE FROM public.shared_events WHERE id = 'event-service-role';"
echo "ok - service_role can write and read under RLS"

expect_rejected() {
  local label="$1"
  local sql="$2"
  if psql_test -c "$sql" >/dev/null 2>&1; then
    echo "expected database constraint rejection: $label" >&2
    exit 1
  fi
  echo "ok - rejected $label"
}

expect_rejected "duplicate client token" "INSERT INTO public.agg_actions
  (id, client_token, kind, verb, object_ref, payload, author, created_at)
  VALUES ('da-duplicate-token', 'ct-current-12345678', 'triage', 'acknowledge',
    'orch:item-2', '{}',
    '{\"kind\":\"drew\",\"id\":\"drew\",\"label\":\"Drew\"}', now())"
expect_rejected "invalid action state" "INSERT INTO public.agg_actions
  (id, kind, verb, object_ref, payload, author, created_at, state)
  VALUES ('da-invalid-state', 'triage', 'acknowledge', 'orch:item-3', '{}',
    '{\"kind\":\"drew\",\"id\":\"drew\",\"label\":\"Drew\"}', now(), 'pending')"
expect_rejected "invalid kind/verb pair" "INSERT INTO public.agg_actions
  (id, kind, verb, object_ref, payload, author, created_at)
  VALUES ('da-invalid-verb', 'status-request', 'acknowledge', 'orch:item-4', '{}',
    '{\"kind\":\"drew\",\"id\":\"drew\",\"label\":\"Drew\"}', now())"
expect_rejected "non-object action payload" "INSERT INTO public.agg_actions
  (id, kind, verb, object_ref, payload, author, created_at)
  VALUES ('da-invalid-payload', 'triage', 'acknowledge', 'orch:item-5', '[]',
    '{\"kind\":\"drew\",\"id\":\"drew\",\"label\":\"Drew\"}', now())"
expect_rejected "incomplete action author" "INSERT INTO public.agg_actions
  (id, kind, verb, object_ref, payload, author, created_at)
  VALUES ('da-invalid-author', 'triage', 'acknowledge', 'orch:item-6', '{}',
    '{\"kind\":\"drew\",\"id\":\"drew\"}', now())"

zero_hash="$(printf '%064d' 0)"
one_hash="$(printf '%064d' 1)"
psql_test -c "INSERT INTO public.shared_events
  (id, event_at, actor, kind, ref, summary, prev_hash, hash, action_id, idempotency_key)
  VALUES ('event-current', now(), 'run-1', 'action-applied', 'orch:item-1',
    'triage/acknowledge applied by run-1', '$zero_hash', '$one_hash',
    'da-current', 'action-result:da-current');"

# Existing JSON action-result events do not carry an action ID. Both new DB
# linkage fields may stay null when such legacy events are replayed.
psql_test -c "INSERT INTO public.shared_events
  (id, event_at, actor, kind, ref, summary, prev_hash, hash)
  VALUES ('event-legacy', now(), 'run-old', 'action-rejected', 'orch:legacy',
    'triage/acknowledge rejected by run-old', '$zero_hash', '$one_hash');"

expect_rejected "orphan action link" "INSERT INTO public.shared_events
  (id, event_at, actor, kind, ref, summary, prev_hash, hash, action_id, idempotency_key)
  VALUES ('event-orphan', now(), 'run-2', 'action-applied', 'orch:item-1', 'outcome',
    '$zero_hash', '$one_hash', 'missing-action', 'action-result:missing-action')"
expect_rejected "action link without idempotency key" "INSERT INTO public.shared_events
  (id, event_at, actor, kind, ref, summary, prev_hash, hash, action_id)
  VALUES ('event-no-key', now(), 'run-2', 'action-applied', 'orch:item-1', 'outcome',
    '$zero_hash', '$one_hash', 'da-current')"
expect_rejected "action link with mismatched idempotency key" "INSERT INTO public.shared_events
  (id, event_at, actor, kind, ref, summary, prev_hash, hash, action_id, idempotency_key)
  VALUES ('event-wrong-key', now(), 'run-2', 'action-applied', 'orch:item-1', 'outcome',
    '$zero_hash', '$one_hash', 'da-current', 'action-result:another-action')"
expect_rejected "duplicate outcome idempotency key" "INSERT INTO public.shared_events
  (id, event_at, actor, kind, ref, summary, prev_hash, hash, action_id, idempotency_key)
  VALUES ('event-retry', now(), 'run-1', 'action-applied', 'orch:item-1', 'retry',
    '$zero_hash', '$one_hash', 'da-current', 'action-result:da-current')"
expect_rejected "action link on non-outcome event" "INSERT INTO public.shared_events
  (id, event_at, actor, kind, ref, summary, prev_hash, hash, action_id, idempotency_key)
  VALUES ('event-wrong-kind', now(), 'run-1', 'comment', 'orch:item-1', 'wrong kind',
    '$zero_hash', '$one_hash', 'da-current', 'action-result:da-current')"
expect_rejected "malformed hash" "INSERT INTO public.shared_events
  (id, event_at, actor, kind, ref, summary, prev_hash, hash)
  VALUES ('event-bad-hash', now(), 'run-old', 'comment', 'orch:item-1', 'bad hash',
    'not-a-hash', '$one_hash')"
expect_rejected "authenticated but unasserted attribution" "INSERT INTO public.shared_events
  (id, event_at, actor, kind, ref, summary, attribution, prev_hash, hash)
  VALUES ('event-bad-attribution', now(), 'run-old', 'comment', 'orch:item-1', 'bad attribution',
    '{\"confidence\":\"authenticated\",\"asserted\":false}', '$zero_hash', '$one_hash')"
expect_rejected "attribution missing the asserted flag" "INSERT INTO public.shared_events
  (id, event_at, actor, kind, ref, summary, attribution, prev_hash, hash)
  VALUES ('event-missing-asserted', now(), 'run-old', 'comment', 'orch:item-1', 'missing asserted',
    '{\"confidence\":\"authenticated\"}', '$zero_hash', '$one_hash')"
expect_rejected "applied action without a result timestamp" "INSERT INTO public.agg_actions
  (id, kind, verb, object_ref, payload, author, created_at, state)
  VALUES ('da-applied-no-ts', 'triage', 'acknowledge', 'orch:item-7', '{}',
    '{\"kind\":\"drew\",\"id\":\"drew\",\"label\":\"Drew\"}', now(), 'applied')"
expect_rejected "rejected non-duplicate action without a result timestamp" "INSERT INTO public.agg_actions
  (id, kind, verb, object_ref, payload, author, created_at, state)
  VALUES ('da-rejected-no-ts', 'triage', 'acknowledge', 'orch:item-8', '{}',
    '{\"kind\":\"drew\",\"id\":\"drew\",\"label\":\"Drew\"}', now(), 'rejected')"
# ActionsStore.add() creates duplicate attempts directly in state rejected
# with duplicateOf and a result note but no consumer timestamp; those rows
# are valid audit history and must import losslessly.
psql_test -c "INSERT INTO public.agg_actions
  (id, kind, verb, object_ref, payload, author, created_at, state, duplicate_of_action_id, result_note)
  VALUES ('da-duplicate-rejected', 'triage', 'acknowledge', 'orch:item-1', '{}',
    '{\"kind\":\"agent\",\"id\":\"session-2\",\"label\":\"agent\"}', now(), 'rejected',
    'da-current', 'Duplicate answer; existing action da-current is new.');"
echo "ok - rejected duplicate without a consumer timestamp is accepted"
expect_rejected "empty idempotency key" "INSERT INTO public.shared_events
  (id, event_at, actor, kind, ref, summary, prev_hash, hash, idempotency_key)
  VALUES ('event-empty-key', now(), 'run-old', 'comment', 'orch:item-1', 'empty key',
    '$zero_hash', '$one_hash', '')"

# The raw timestamp strings are the hash inputs: an offset-spelled original
# must round-trip verbatim even though timestamptz normalizes the instant.
psql_test -c "INSERT INTO public.shared_events
  (id, event_at, ingested_at, event_at_raw, ingested_at_raw, actor, kind, ref, summary, prev_hash, hash)
  VALUES ('event-raw-ts', '2026-11-05T03:00:00+03:00', '2026-11-05T03:00:01+03:00',
    '2026-11-05T03:00:00+03:00', '2026-11-05T03:00:01+03:00',
    'run-old', 'comment', 'orch:item-1', 'raw timestamp preservation',
    '$zero_hash', '$one_hash');"
raw_seen="$(psql_test -Atc "SELECT event_at_raw || '|' || ingested_at_raw || '|' || (event_at AT TIME ZONE 'UTC')::text FROM public.shared_events WHERE id = 'event-raw-ts'")"
if [[ "$raw_seen" != "2026-11-05T03:00:00+03:00|2026-11-05T03:00:01+03:00|2026-11-05 00:00:00" ]]; then
  echo "raw timestamp strings did not round-trip: $raw_seen" >&2
  exit 1
fi
echo "ok - raw timestamp strings preserved alongside normalized timestamptz"

# The raw JSON serializations are hash inputs too: canonicalFields hashes
# JSON.stringify of the original attribution/source objects, and jsonb
# reorders object keys (shortest first), so only the verbatim text
# reproduces the original hash input.
psql_test -c "INSERT INTO public.shared_events
  (id, event_at, actor, kind, ref, summary, attribution, attribution_raw, source, source_raw, prev_hash, hash)
  VALUES ('event-raw-json', now(), 'run-old', 'comment', 'orch:item-1', 'raw json preservation',
    '{\"confidence\":\"unknown\",\"asserted\":false}',
    '{\"confidence\":\"unknown\",\"asserted\":false}',
    '{\"identity\":\"pm-stack/aggregator\",\"channel\":\"test\"}',
    '{\"identity\":\"pm-stack/aggregator\",\"channel\":\"test\"}',
    '$zero_hash', '$one_hash');"
raw_json_seen="$(psql_test -Atc "SELECT attribution_raw || '|' || source_raw FROM public.shared_events WHERE id = 'event-raw-json'")"
if [[ "$raw_json_seen" != "{\"confidence\":\"unknown\",\"asserted\":false}|{\"identity\":\"pm-stack/aggregator\",\"channel\":\"test\"}" ]]; then
  echo "raw JSON serializations did not round-trip: $raw_json_seen" >&2
  exit 1
fi
echo "ok - raw JSON serializations preserved alongside jsonb"

if psql_test < "$down" >/dev/null 2>&1; then
  echo "down migration should refuse populated tables" >&2
  exit 1
fi
echo "ok - down migration protects non-empty tables"

psql_test -c 'DELETE FROM public.shared_events; DELETE FROM public.agg_actions;'
psql_test < "$down"
remaining="$(psql_test -Atc "SELECT (to_regclass('public.agg_actions') IS NULL AND to_regclass('public.shared_events') IS NULL)::text")"
if [[ "$remaining" != true ]]; then
  echo "down migration left schema objects behind" >&2
  exit 1
fi

echo "ok - migration constraints and rollback verified on disposable PostgreSQL 16"
