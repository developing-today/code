# PM-stack shared schema — per-service migration plan (slice 2)

Tracks [#1744](https://github.com/developing-today/code/issues/1744),
acceptance checkbox 2. Planning only: no store or service is migrated
here. Slice 1 ([inventory + schema map](SHARED-SCHEMA-MAP.md), PR #4692)
is the evidence base — every "current store" below is its row, and this
plan moves those rows, in order, without re-litigating the one-database /
namespaced-table decision or the #3142 storage choice.

## Constraints the plan inherits

- **#3142 (open) supersedes #1744's one-local-libSQL default** for new
  state: start new PM state in Supabase `muse-dev2`; keep Kaneo, Grist,
  and Pad databases separate unless a supported shared-database mode is
  proven. Moves below therefore land shared entities in Supabase
  (`muse-dev2`) table groups, not in a new local libSQL file, except
  where the service already runs on libSQL (projector).
- **Each product layer owns its source of truth; the projector is glue,
  not a source of truth** (see map's "How to read the map"). Migration
  moves the _storage location_, never the ownership.
- **Every move is independently shippable and reversible**: each move
  ships behind a read-fallback (dual-read or export-replay) and names its
  rollback before it lands. No flag-day cutovers.

## Preconditions (not moves — must land before Move 1)

- **P0 — `pmp_links` backup legs.** The only non-rebuildable
  cross-service store has no evidenced backup leg: the dashboard archive
  manifest stages aggregator files and does not include `projector.db`
  or a `pmp_links` export (map's "Registry backup legs" section,
  verified at this branch's base). Adding a `pmp_links` export to all
  three archive legs (local, R2, Drive) is a prerequisite for any
  migration touching the link registry. Named follow-up: backup-leg
  slice (does the export + verifies all three legs).
- **P1 — Target schema vetted.** Supabase `muse-dev2` table groups
  (`agg_*`, `shared_items`, `shared_links`, `shared_comments`,
  `shared_events`) created with the column contracts from the map;
  projector `pmp_*` group created alongside (new libSQL home or the
  existing file — location is a P1 decision, recorded in the move-2
  slice).

## Moves (ordered)

### Move 1 — Aggregator JSON stores → `agg_*` + shared entities

First because the aggregator's stores are the largest rebuildability
spread (map: comments/actions/annotations/views non-rebuildable;
feed/quota/heartbeat caches refreshable) and every other service reads
aggregator output. Per store, in this order inside the move:

1. Append-only / audit stores (`actions.json` → `agg_` actions +
   `shared_events` outcomes; `usage-snapshot-outbox.jsonl` drained then
   moved to `shared_events`/`agg_` ingestion).
2. User-authored durable stores (`comments.json` → `shared_comments`;
   `roadmap-annotations.json` → `agg_`; `views.json` → `agg_`;
   `alerts.*` incl. `incident_events` → `agg_` + `shared_events`).
3. Refreshable caches last (`fleet-feed.json`, `quota-current.json`,
   heartbeats, snapshots, `aa-last-good.json`), each cut over by
   pointing the writer at the table and keeping the file as a
   last-known-good fallback until the move is declared stable.

Rollback per store: writer points back at the file; files are retained
until the move's stability window closes. Reversibility is per store,
so Move 1 itself ships as a sequence of small PRs.

### Move 2 — Projector DB → shared groups, `pmp_links` as canonical `shared_links` source

Second because the projector is disposable _except_ `pmp_links`, and
Move 1 must be stable first (aggregator reads projector views for link
joins). Steps:

1. P0 (backup legs) verified present — hard gate, no exception.
2. `pmp_items` / `pmp_comments` projections dual-write to
   `shared_items` / `shared_comments`; views `pmp_v_links_items` /
   `pmp_v_subject_activity` recreated over the shared tables.
3. `pmp_links` promoted to the canonical `shared_links` source with the
   same `(subject, ref, relation)` contract; projector keeps a
   `pmp_`-namespaced read replica while consumers cut over.
4. Operational tables (`pmp_deliveries`, `pmp_delivery_errors`,
   `pmp_sync_state`, `pmp_push_events`) stay `pmp_`-namespaced;
   `pmp_push_events` maps to `shared_events` only if cross-service
   retention is wanted (deferred design choice, recorded in the map).

Rollback: `DATABASE_URL` points back at the file DB; dual-write order
means the file is never behind.

### Moves 3+ — Native service stores (each its own slice, #3142-gated)

Kaneo (Postgres), Grist (document volume), Pad (SQLite), ready-for-agent
(SQLite) keep their native databases and restore paths unless a
supported shared-database mode is proven (#3142). Each service gets its
own migration slice that projects cross-service records to
`shared_items` / `shared_events` while the native store remains source
of truth. No slice in this family starts before Moves 1–2 are declared
stable. Ready-for-agent additionally needs a schema audit first (map:
DB filename/schema/restore path not evidenced in checked-in code).

## Coordination with sibling issues

- #1738 (roadmap file integration) and #1739 (event log) design against
  the target groups named here, not against per-service files.
- #1288 (Grist live-apply) and #1734 (change-lag surfacing) consume the
  post-Move-1 subscription/polling behavior; Move 1's per-store PRs
  note which consumer each store unblocks.

## Follow-up slices (criterion 3 and beyond)

- **Backup-leg slice** (P0): `pmp_links` export on all three archive
  legs, verified.
- **Move-1 execution slices**: one per store group above.
- **Move-2 execution slice**: gated on P0 + Move-1 stability.
- **Moves-3+ slices**: one per native service, #3142-gated.
- **Criterion-3 slice**: after all moves, verify no private
  items/links/comments copies, one backup target, cross-service
  visibility without restarts — then check criterion 3.

## Verification for this slice

- [x] Map's projector.db / `pmp_` inventory re-verified against
      `projector/schema.sql`, `projector/src/store.ts`, and
      `projector/README.md` at this branch's base (tables, views,
      `file:data/projector.db` default, backup-gap statement all current;
      line citations drifted by a few lines, substance unchanged — map left
      untouched to keep this slice planning-only).
- [ ] Plan-doc lint/links check per repo tooling + CI on the slice PR.
