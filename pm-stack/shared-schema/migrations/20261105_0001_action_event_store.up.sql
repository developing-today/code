-- #6103: first schema contract for PM aggregator actions and their outcomes.
-- Target: Supabase muse-dev2, public schema. This DDL is not applied here.

CREATE TABLE public.agg_actions (
  id text PRIMARY KEY CHECK (btrim(id) <> ''),
  client_token text CHECK (client_token IS NULL OR btrim(client_token) <> ''),
  kind text NOT NULL CHECK (kind IN ('triage', 'status-request', 'session', 'preference')),
  verb text NOT NULL,
  object_ref text NOT NULL CHECK (btrim(object_ref) <> ''),
  target_ref text,
  session_id text,
  session_source text,
  payload jsonb NOT NULL CHECK (jsonb_typeof(payload) = 'object'),
  author jsonb NOT NULL CHECK (
    jsonb_typeof(author) = 'object'
    AND coalesce(author->>'kind' IN ('drew', 'muse', 'agent'), false)
    AND nullif(btrim(author->>'id'), '') IS NOT NULL
    AND nullif(btrim(author->>'label'), '') IS NOT NULL
  ),
  created_at timestamptz NOT NULL,
  state text NOT NULL DEFAULT 'new'
    CHECK (state IN ('new', 'applied', 'rejected', 'superseded')),
  applied_at timestamptz,
  applied_by_run text,
  result_note text,
  supersedes_action_id text REFERENCES public.agg_actions(id) ON DELETE RESTRICT,
  duplicate_of_action_id text REFERENCES public.agg_actions(id) ON DELETE RESTRICT,
  CONSTRAINT agg_actions_verb_for_kind_check CHECK (
    (kind = 'triage' AND verb IN (
      'acknowledge', 'snooze', 'escalate', 'de-escalate', 'answer', 'delegate',
      'dismiss', 'approve-proposal', 'reject-proposal', 'mark-done', 'restore'
    ))
    OR (kind = 'status-request' AND verb = 'request-status')
    OR (kind = 'session' AND verb IN ('poke', 'answer', 'stop'))
    OR (kind = 'preference' AND verb IN (
      'set-delegation-default', 'set-needs-order', 'save-view', 'rename-view',
      'delete-view', 'set-default-view'
    ))
  ),
  CONSTRAINT agg_actions_result_timestamp_check CHECK (
    (state = 'applied' AND applied_at IS NOT NULL)
    OR (state = 'rejected'
        AND (applied_at IS NOT NULL OR duplicate_of_action_id IS NOT NULL))
    OR (state = 'new' AND applied_at IS NULL)
    OR state = 'superseded'
  ),
  CONSTRAINT agg_actions_not_self_superseded_check CHECK (
    supersedes_action_id IS NULL OR supersedes_action_id <> id
  ),
  CONSTRAINT agg_actions_not_self_duplicate_check CHECK (
    duplicate_of_action_id IS NULL OR duplicate_of_action_id <> id
  )
);

CREATE UNIQUE INDEX agg_actions_client_token_uq
  ON public.agg_actions (client_token)
  WHERE client_token IS NOT NULL;

CREATE INDEX agg_actions_state_created_at_idx
  ON public.agg_actions (state, created_at DESC);

CREATE INDEX agg_actions_object_created_at_idx
  ON public.agg_actions (object_ref, created_at DESC);

CREATE TABLE public.shared_events (
  id text PRIMARY KEY CHECK (btrim(id) <> ''),
  event_at timestamptz NOT NULL,
  ingested_at timestamptz NOT NULL DEFAULT now(),
  event_at_raw text CHECK (event_at_raw IS NULL OR btrim(event_at_raw) <> ''),
  ingested_at_raw text CHECK (ingested_at_raw IS NULL OR btrim(ingested_at_raw) <> ''),
  actor text NOT NULL CHECK (btrim(actor) <> ''),
  kind text NOT NULL CHECK (btrim(kind) <> ''),
  ref text NOT NULL CHECK (btrim(ref) <> ''),
  summary text NOT NULL,
  attribution jsonb NOT NULL DEFAULT '{"confidence":"unknown","asserted":false}'::jsonb
    CHECK (
      jsonb_typeof(attribution) = 'object'
      AND coalesce(attribution->>'confidence' IN ('authenticated', 'pre-sso', 'unknown'), false)
      AND coalesce(jsonb_typeof(attribution->'asserted') = 'boolean', false)
      AND (attribution->>'confidence' <> 'authenticated' OR attribution->'asserted' = 'true'::jsonb)
    ),
  source jsonb NOT NULL DEFAULT '{"identity":"pm-stack/aggregator"}'::jsonb
    CHECK (
      jsonb_typeof(source) = 'object'
      AND nullif(btrim(source->>'identity'), '') IS NOT NULL
    ),
  attribution_raw text CHECK (
    attribution_raw IS NULL
    OR jsonb_typeof(attribution_raw::jsonb) = 'object'
  ),
  source_raw text CHECK (
    source_raw IS NULL
    OR jsonb_typeof(source_raw::jsonb) = 'object'
  ),
  corrects_event_id text,
  migrated_from_version smallint CHECK (migrated_from_version IS NULL OR migrated_from_version = 1),
  prev_hash text NOT NULL CHECK (prev_hash ~ '^[0-9a-f]{64}$'),
  hash text NOT NULL CHECK (hash ~ '^[0-9a-f]{64}$'),
  action_id text REFERENCES public.agg_actions(id) ON DELETE RESTRICT,
  idempotency_key text CHECK (idempotency_key IS NULL OR btrim(idempotency_key) <> ''),
  CONSTRAINT shared_events_action_link_check CHECK (
    (
      kind IN ('action-applied', 'action-rejected')
      AND action_id IS NULL
      AND idempotency_key IS NULL
    )
    OR (
      kind IN ('action-applied', 'action-rejected')
      AND action_id IS NOT NULL
      AND idempotency_key IS NOT NULL
      AND idempotency_key = 'action-result:' || action_id
    )
    OR (kind NOT IN ('action-applied', 'action-rejected') AND action_id IS NULL)
  )
);

CREATE UNIQUE INDEX shared_events_idempotency_key_uq
  ON public.shared_events (idempotency_key)
  WHERE idempotency_key IS NOT NULL;

CREATE UNIQUE INDEX shared_events_action_outcome_uq
  ON public.shared_events (action_id)
  WHERE action_id IS NOT NULL;

CREATE INDEX shared_events_event_at_idx
  ON public.shared_events (event_at DESC);

CREATE INDEX shared_events_kind_ref_event_at_idx
  ON public.shared_events (kind, ref, event_at DESC);

ALTER TABLE public.agg_actions ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.shared_events ENABLE ROW LEVEL SECURITY;

GRANT SELECT, INSERT, UPDATE ON public.agg_actions TO service_role;
GRANT SELECT, INSERT, DELETE ON public.shared_events TO service_role;

COMMENT ON TABLE public.agg_actions IS
  'Aggregator-owned action records; JSON remains authoritative until a separate Move 1 cutover.';
COMMENT ON TABLE public.shared_events IS
  'Shared audit events; action-result rows link to agg_actions when written by the DB-backed store.';
