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

psql_test -c 'CREATE ROLE service_role;'
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
