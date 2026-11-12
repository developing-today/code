-- #5005: schema contract for orchestration state, phase 1 of #3142.
-- Target: Supabase muse-dev2, public schema. This DDL is not applied here;
-- orchestration.json remains authoritative until the writer-cutover slice.
--
-- Two tables, one per file the state lives in today:
--   orch_runs  — the run log (orchestrator-log.md's durable successor):
--                one append-only row per orchestration pass, carrying the
--                complete export document for that run.
--   orch_items — the current tier/classification state (the tiers half of
--                orchestration.json / orchestrator-state.json): one row per
--                classified item, replaced per run in the writer's
--                transaction so the table always equals the newest run.

CREATE TABLE public.orch_runs (
  run_id text PRIMARY KEY CHECK (btrim(run_id) <> ''),
  generated_at timestamptz NOT NULL,
  document_version integer NOT NULL CHECK (document_version >= 1),
  provenance jsonb NOT NULL CHECK (jsonb_typeof(provenance) = 'object'),
  document jsonb NOT NULL CHECK (jsonb_typeof(document) = 'object'),
  item_count integer NOT NULL CHECK (item_count >= 0),
  CONSTRAINT orch_runs_document_identity_check CHECK (
    document ->> 'runId' = run_id
    AND document ->> 'version' ~ '^[0-9]+$'
    AND (document ->> 'version')::integer = document_version
  )
);

CREATE TABLE public.orch_items (
  id text PRIMARY KEY CHECK (btrim(id) <> ''),
  tier text NOT NULL CHECK (tier IN ('needsDrew', 'needsOrchestrator')),
  kind text NOT NULL CHECK (btrim(kind) <> ''),
  title text NOT NULL,
  source text NOT NULL,
  -- TierItem.state is optional in the writer contract (aggregator
  -- model.ts): a non-session item may carry no state at all — the
  -- contract corpus's decision item omits it. Absent state is NULL;
  -- a state that IS present must still be non-empty (#8856).
  state text CHECK (state IS NULL OR btrim(state) <> ''),
  reason text NOT NULL,
  ref text,
  since timestamptz NOT NULL,
  last_seen_at timestamptz NOT NULL,
  item jsonb NOT NULL CHECK (jsonb_typeof(item) = 'object'),
  first_seen_run_id text NOT NULL
    REFERENCES public.orch_runs (run_id) ON DELETE RESTRICT,
  last_seen_run_id text NOT NULL
    REFERENCES public.orch_runs (run_id) ON DELETE RESTRICT,
  CONSTRAINT orch_items_seen_order_check CHECK (last_seen_at >= since),
  CONSTRAINT orch_items_document_identity_check CHECK (item ->> 'id' = id)
);

CREATE INDEX orch_items_tier_state_idx
  ON public.orch_items (tier, state);

CREATE INDEX orch_items_last_seen_run_idx
  ON public.orch_items (last_seen_run_id);

ALTER TABLE public.orch_runs ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.orch_items ENABLE ROW LEVEL SECURITY;

-- The run log is append-only: service_role may read and append, never
-- rewrite or delete history. The state table is a per-run snapshot, so
-- the writer needs the full replace set.
GRANT SELECT, INSERT ON public.orch_runs TO service_role;
GRANT SELECT, INSERT, UPDATE, DELETE ON public.orch_items TO service_role;

COMMENT ON TABLE public.orch_runs IS
  'Orchestration pass run log (#5005); orchestration.json remains authoritative until the writer-cutover slice.';
COMMENT ON TABLE public.orch_items IS
  'Current orchestration tier/classification state (#5005); one row per classified item, replaced per run.';
