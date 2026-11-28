-- #6815: canonical backing store for the four aggregator laptop-local
-- stores (actions, activity-ingestion, usage snapshots, aa-last-good).
-- Target: Supabase muse-dev2, public schema.
--
-- Prerequisite: apply 20261105_0001_action_event_store first. It owns
-- public.agg_actions, the typed contract for actions.json records; this
-- migration does not redefine it.
--
-- Additive and reversible: these tables hold no rows until the
-- aggregator's PM_AGG_STORE_BACKEND=supabase writer is enabled, and the
-- down migration refuses to drop a non-empty table.
--
-- #3142 table groups: agg_actions (existing contract) for actions,
-- agg_ ingestion for the checkpoints/dedupe cursor, agg_ usage history
-- for snapshot observations, and agg_ cache for the AA last-good
-- document (rebuildable from Supabase + the opencode config).

-- ---------------------------------------------------------------------
-- Activity ingestion (#3321): durable per-source cursor, dedupe memory,
-- gap ledger, and source status. One row per source; gaps are upserted
-- by id and never deleted (a gap is resolved in place).
-- ---------------------------------------------------------------------
CREATE TABLE public.agg_activity_ingestion (
  source text PRIMARY KEY CHECK (btrim(source) <> ''),
  checkpoint jsonb CHECK (checkpoint IS NULL OR jsonb_typeof(checkpoint) = 'object'),
  seen_ids text[] NOT NULL DEFAULT '{}',
  source_status jsonb CHECK (source_status IS NULL OR jsonb_typeof(source_status) = 'object'),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE public.agg_activity_gaps (
  id text PRIMARY KEY CHECK (btrim(id) <> ''),
  source text NOT NULL CHECK (btrim(source) <> ''),
  gap_from timestamptz NOT NULL,
  gap_to timestamptz NOT NULL,
  status text NOT NULL CHECK (status IN ('open', 'backfilled', 'unrecoverable')),
  detected_at timestamptz NOT NULL,
  resolved_at timestamptz,
  updated_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT agg_activity_gaps_window_check CHECK (gap_to >= gap_from),
  CONSTRAINT agg_activity_gaps_resolved_check CHECK (
    (status = 'open' AND resolved_at IS NULL) OR status <> 'open'
  )
);

CREATE INDEX agg_activity_gaps_source_from_idx
  ON public.agg_activity_gaps (source, gap_from);

-- ---------------------------------------------------------------------
-- Usage snapshot history (hub design §4). Append-only observations keyed
-- by the store's own dedupe key (service|metric|ts|value), so replaying a
-- writer or the backfill is idempotent. Raw history is retained in the
-- database; monthly rollup files are a file-store compaction concern and
-- are not written by the remote backend.
-- ---------------------------------------------------------------------
CREATE TABLE public.agg_usage_snapshots (
  snapshot_key text PRIMARY KEY CHECK (btrim(snapshot_key) <> ''),
  ts timestamptz NOT NULL,
  service_id text NOT NULL CHECK (btrim(service_id) <> ''),
  metric_id text NOT NULL CHECK (btrim(metric_id) <> ''),
  unit text NOT NULL CHECK (btrim(unit) <> ''),
  source text NOT NULL CHECK (btrim(source) <> ''),
  value jsonb NOT NULL CHECK (
    jsonb_typeof(value) = 'number'
    OR (
      jsonb_typeof(value) = 'object'
      AND jsonb_typeof(value->'low') = 'number'
      AND jsonb_typeof(value->'high') = 'number'
    )
  ),
  limit_value double precision CHECK (limit_value IS NULL OR limit_value >= 0),
  resets_at timestamptz,
  observed_by text NOT NULL CHECK (btrim(observed_by) <> ''),
  recorded_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX agg_usage_snapshots_ts_idx
  ON public.agg_usage_snapshots (ts DESC);

CREATE INDEX agg_usage_snapshots_service_ts_idx
  ON public.agg_usage_snapshots (service_id, ts DESC);

-- ---------------------------------------------------------------------
-- Generic canonical document row for aggregator-owned JSON documents that
-- have no typed contract yet. The only user in this slice is
-- aa-last-good.json (a rebuildable cache, but the canonical copy for the
-- collector while the backend is supabase).
-- ---------------------------------------------------------------------
CREATE TABLE public.agg_store_documents (
  store text PRIMARY KEY CHECK (btrim(store) <> ''),
  version integer NOT NULL CHECK (version >= 1),
  document jsonb NOT NULL,
  writer text,
  updated_at timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE public.agg_activity_ingestion ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.agg_activity_gaps ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.agg_usage_snapshots ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.agg_store_documents ENABLE ROW LEVEL SECURITY;

-- One writer (pm-aggregator.service / fleet-feed-collector) under the
-- service role. Snapshots are append-only; retention may delete old raw
-- rows, so DELETE is granted there only.
GRANT SELECT, INSERT, UPDATE ON public.agg_activity_ingestion TO service_role;
GRANT SELECT, INSERT, UPDATE ON public.agg_activity_gaps TO service_role;
GRANT SELECT, INSERT, DELETE ON public.agg_usage_snapshots TO service_role;
GRANT SELECT, INSERT, UPDATE ON public.agg_store_documents TO service_role;

COMMENT ON TABLE public.agg_activity_ingestion IS
  'Aggregator activity-ingestion cursors and dedupe memory (#3321); canonical when PM_AGG_STORE_BACKEND=supabase.';
COMMENT ON TABLE public.agg_activity_gaps IS
  'Aggregator activity-ingestion gap ledger (#3321); gaps resolve in place and are never deleted.';
COMMENT ON TABLE public.agg_usage_snapshots IS
  'Append-only usage observations (hub design §4); dedupe key is service|metric|ts|value.';
COMMENT ON TABLE public.agg_store_documents IS
  'Aggregator-owned canonical JSON documents without a typed contract; store names are stable keys (aa-last-good).';
