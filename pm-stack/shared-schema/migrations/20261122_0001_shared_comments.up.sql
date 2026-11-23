-- #1744 Move 1, step 2 (first store): dashboard comments become the
-- cross-service shared_comments table. Target: the shared PM database
-- (live copies in Turso since the 2026-11-18 recovery; this PostgreSQL
-- dialect is the contract the Turso bridge in
-- aggregator/src/turso-stores.ts translates, as with 20261105_0001 and
-- 20261116_0001).
--
-- Additive and reversible: the table holds no rows until the
-- aggregator's PM_AGG_STORE_BACKEND=turso writer is enabled for
-- comments, and the down migration refuses to drop a non-empty table.
-- The file store (comments.json) remains the default backend and the
-- rollback target; exactly one backend is active at a time.
--
-- Column contract mirrors the DashComment record in
-- aggregator/src/model.ts and the record validator in
-- aggregator/src/comments.ts: typed columns for every field, with the
-- author kept as the same jsonb object contract as agg_actions.

CREATE TABLE public.shared_comments (
  id text PRIMARY KEY CHECK (btrim(id) <> ''),
  object_ref text NOT NULL CHECK (btrim(object_ref) <> ''),
  kind text NOT NULL CHECK (kind IN ('message', 'note', 'github')),
  author jsonb NOT NULL CHECK (
    jsonb_typeof(author) = 'object'
    AND coalesce(author->>'kind' IN ('drew', 'muse', 'agent'), false)
    AND nullif(btrim(author->>'id'), '') IS NOT NULL
    AND nullif(btrim(author->>'label'), '') IS NOT NULL
  ),
  author_verified boolean NOT NULL DEFAULT false,
  provenance text CHECK (provenance IN ('verified', 'asserted-legacy')),
  linked_identity jsonb CHECK (
    linked_identity IS NULL
    OR (
      jsonb_typeof(linked_identity) = 'object'
      AND nullif(btrim(linked_identity->>'source'), '') IS NOT NULL
      AND nullif(btrim(linked_identity->>'id'), '') IS NOT NULL
    )
  ),
  body text NOT NULL,
  created_at timestamptz NOT NULL,
  triage_state text CHECK (triage_state IN ('new', 'triaged')),
  github_ref text CHECK (github_ref IS NULL OR github_ref ~ '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+#[0-9]+$'),
  github_state text CHECK (github_state IN ('pending', 'posted', 'failed')),
  github_url text,
  github_error text,
  github_failed_at timestamptz,
  -- The app validator's one cross-field rule: a pending github comment
  -- must name the issue/PR it will post to.
  CONSTRAINT shared_comments_pending_github_ref_check CHECK (
    kind <> 'github' OR github_state <> 'pending' OR github_ref IS NOT NULL
  )
);

-- The hot reads are per-object threads, oldest first (forObject /
-- byObject in aggregator/src/comments.ts).
CREATE INDEX shared_comments_object_created_idx
  ON public.shared_comments (object_ref, created_at);

ALTER TABLE public.shared_comments ENABLE ROW LEVEL SECURITY;

-- One writer (pm-aggregator.service) under the service role. Comment
-- state transitions (triage, github posting results) update rows in
-- place; history is never deleted through the store.
GRANT SELECT, INSERT, UPDATE ON public.shared_comments TO service_role;

COMMENT ON TABLE public.shared_comments IS
  'Dashboard comments (#906 section 2) as the shared cross-service comment record (#1744); canonical when PM_AGG_STORE_BACKEND=turso, comments.json remains the file backend and rollback target.';
