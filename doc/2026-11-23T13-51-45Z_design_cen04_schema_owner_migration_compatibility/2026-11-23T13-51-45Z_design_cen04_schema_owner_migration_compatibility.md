# CEN-04 — Cross-model schema owner and migration compatibility matrix

Issue: [#9434](https://github.com/developing-today/code/issues/9434)
(slice `CEN-04` of [#9198](https://github.com/developing-today/code/issues/9198),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement or
migrate.** This record elaborates the cross-model compatibility slice
of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225) into the schema-ownership matrix and field-admission gate
that schema-coupled implementation issues consume. It invents no
table, moves no data, and authorizes no migration; those remain
separately gated stages (see §9).

Governing decisions consumed, not re-opened:

- **One home per fact** ([#8575](https://github.com/developing-today/code/issues/8575),
  OPEN; direction Drew 2026-11-11): the queue's two durable stores
  were holding the same facts twice; the fix is ownership, not sync.
  Coordination (claims, leases, heartbeats, checkpoints, merge-order
  lists) lives in the database and is ephemeral by design — it passes
  the wipe test. Durable truth (issues, labels, PRs, outcomes a human
  must be able to read back) lives in GitHub. This record applies
  that principle fact by fact; it does not re-argue it.
- **Durable task model** ([#3424](https://github.com/developing-today/code/issues/3424),
  CLOSED): blockers are typed links (to tasks, PRs, issues), not
  free text; external waits name absolute instants; long parks are
  re-verified on a cadence. Free-text block reasons are legacy data,
  never silently promoted to typed dependencies.
- **Wipe-test audit** ([#8579](https://github.com/developing-today/code/issues/8579),
  CLOSED) and the unification slices #8580 (CLOSED, GitHub mirror in
  the database), #8581 (OPEN, verb contract v2), #8582 (CLOSED,
  merge-intent queue), #8583 (OPEN, local coordination replica),
  #8584 (OPEN, backups), #8585 (OPEN, label de-mirroring): consumed
  as the compatibility evidence base for the rows they touch.
- **QTI disposition** ([#9241](https://github.com/developing-today/code/issues/9241),
  CLOSED 2026-11-22): the Queue Table Improvements effort is **not
  located within the enumerated accessible surfaces** — a bounded
  research verdict with a field-owner crosswalk at the census pin
  (issue comment 6845605876), not a claim of global absence. §6
  states what that disposition does and does not gate now.
- **Census component** (`components/9198-requirement-crosswalk-current-state-census.md`):
  §3's table is the *domain-level* ownership crosswalk (which
  component owns which requirement family). This record is the
  *fact-level* matrix the same component's CEN-04 spec calls for;
  the two do not compete. The census's V1 facts are pinned at
  `a038f084`; where current main has moved past the pin, this record
  says so explicitly (see the priority row, §3.3).
- **Sibling records landed 2026-11-23**, consumed as contracts:
  CLI-01 (#9451, CLOSED — the `contracts.rs` wire types), CLI-02
  (PR #13255 — the parser produces validated intent for those types
  and persists nothing), HYD-05 (PR #13297 — cursor logical shape
  fixed; its storage expressly waits for this record, §9 of that
  record), HYD-07 (PR #13246 — attribution reads records, creates
  none outside its map), REL-06 (PR #13247 — persists nothing new;
  "any future persisted receipt field goes through CEN-04's matrix
  first"). §5 is the gate those sentences point at.
- **PM Stack schema line**: `pm-stack/SHARED-SCHEMA-MAP.md` (#1744)
  is the PM-side store inventory of record and
  `pm-stack/SHARED-SCHEMA-MIGRATION-PLAN.md` its move plan; #3142
  chose the PM storage direction. This record does not re-inventory
  PM stores; §3.14 points at that map and records the one live
  conflict that affects it.

Verified state at writing (2026-11-23, origin/main `ba7ea99e9`):

- No dated CEN-04 record exists under `doc/`;
  `census/test_schema_compatibility_matrix.py` does not exist; the
  census component carries the CEN-04 slice text but no filled
  fact-level matrix. This record is the first.
- The broker's production schema of record is the `SCHEMA` const in
  `pkgs/fleet/crates/fleet-queue/src/turso_queue/core.rs` (19
  statements: `tasks`, `leases`, `lanes`, `workers`, `checkpoints`,
  `repo_task_lane_ledger`, `events`, `creation_outbox`,
  `creation_outbox_circuit`, `merge_intents` via
  `crate::merge_intent::MERGE_INTENT_DDL`, `placement_sessions`).
  The numbered SQL files in `pkgs/fleet/migrations/` are the
  Supabase-era lineage the Turso store succeeded; they are history,
  not a second schema authority.
- The metadata store exists and its issue is CLOSED:
  [#9074](https://github.com/developing-today/code/issues/9074).
  Its DDL of record is `PM_METADATA_DDL` in
  `pkgs/fleet/crates/fleet/src/gh_metadata.rs`
  (`pm_metadata_items`), with the hosted worker's `MetadataStore`
  carrying the same shape behind its interface.
- Gate state for this slice: PR #9225 MERGED; #9241 CLOSED with the
  bounded disposition above. #9198 remains OPEN — it is this
  record's design parent and, per its own triage, stays open as
  parent until its children complete; the sibling records treat
  their parents (#9196/#9206/#9207) the same way. It gates the
  separately scoped implementation issue (§9), not this record.

## 1. Intent

Every schema-coupled design in this program keeps arriving at the
same unasked question: *this fact needs to persist — whose is it,
where does it live, and what already-existing thing is it not?* The
answers existed, but scattered: a principle in #8575, a task model
in #3424, an audit in #8579, a DDL const in the broker, a store map
in PM Stack, and hard-won name distinctions (`events` is not the
journal; `blocked_deps` is not the typed graph) carried only in
issue text. Slices have begun citing the absence directly — HYD-05
fixed its cursor's logical shape and refused to pick a table until
this record named the owner; REL-06 refused to persist a receipt
field until this matrix exists.

CEN-04 answers the question once, at fact level: for each selected
Fleet fact, one owner, one source of truth, the write and read APIs,
the legacy and adjacent surfaces with their exact mapping, a
disposition (reuse / extend / absent / conflict), and the migration
and rollback shape if the row ever moves. §5 then turns the matrix
into a standing gate: a persisted field that is not in this matrix
does not get written until its row is added by the same review that
approves the field.

## 2. The ownership rule

A **fact** here is a named, persisted piece of state that more than
one component reads or could read. Each fact has exactly one
**owner**: the component entitled to mutate it, through its own
write API. Ownership kinds used in the matrix:

- **System of record** — the durable home; other surfaces are
  projections or mirrors and must be rebuildable from it (GitHub for
  issue/PR truth; the broker store for queue coordination truth).
- **Coordination store** — durable enough to survive a process
  restart, ephemeral by design intent: wiping it loses in-flight
  coordination, never history (#8575's wipe test).
- **Request ledger** — an append-only record of *requests* and their
  outcomes (the aggregator action store). A ledger entry is evidence
  an effect was asked for; it is never the applied state itself.
- **Projection** — a rebuildable read model (Project boards, the
  projector hot ledger, the metadata mirror). A projection owns no
  fact; its rows cite the system of record they derive from.

Rules every row obeys:

1. **One writer.** Only the owner mutates a fact, and only through
   the owner's API. A second write path is a defect, not a fallback.
2. **No equivalence by name.** Two surfaces with similar names or
   overlapping columns are different facts until a matrix row says
   otherwise with evidence (§4 collects the standing refusals).
3. **Additive and reversible.** A change to a persisted shape is
   additive first, keeps the prior reader working, and has a
   disable-only rollback (§7). No silent fallback, no dual
   authority during a move.
4. **Unknown stays unknown.** Legacy or migrated data whose
   provenance cannot be established is recorded as unknown or
   quarantined, never upgraded by proximity to a similar field.

## 3. The compatibility matrix

Each row: **owner** (component + design authority) · **home** (the
source of truth, with its code location) · **write / read API** ·
**legacy & adjacent mapping** · **disposition** · **migration &
rollback** where a move is contemplated. "Broker" throughout is
`fleet-queue`'s Turso store and its verb surface (the hosted queue
door, #12589, answers the same envelope over the same contract and
is not a second store of these facts).

### 3.1 Queue work identity — task `id`, `queue`, `dedupe_key`

- **Owner:** the queue broker (#9206 lifecycle surface; storage
  owned by fleet-queue). **Home:** `tasks` in the broker `SCHEMA`
  (`turso_queue/core.rs`); `id` is broker-assigned, `dedupe_key`
  carries the packet's deterministic key.
- **Write API:** broker `enqueue` only, validated by
  `validate.rs` (repo-task/v1 packet). **Read API:** broker
  `list`/`show`; the GitHub issue a packet references is linked
  truth, never the task's identity.
- **Legacy mapping:** Supabase-era repo-queue rows were loaded into
  this store by the #8584-line backup/restore; duplicate
  `(queue, dedupe_key)` pairs in that data are why the dedupe index
  is deliberately non-unique and enqueue dedupe is read-then-insert.
  That tolerance is a data fact, not a license to create duplicates.
- **Disposition:** reuse. **Migration/rollback:** none contemplated;
  identity is the anchor every other row references.

### 3.2 Queue work state — `status`, `progress`, `result`, attempts

- **Owner:** the queue broker. **Home:** `tasks.status` (lifecycle
  vocabulary owned by #9206 / DST-01's crosswalk), `tasks.progress`,
  `tasks.result`, `tasks.attempts`/`max_attempts`, park/wait fields
  (`park_kind`, `wait_kind`, `ready_condition`, `not_before`).
- **Write API:** broker lifecycle verbs only (`claim`, `heartbeat`,
  `checkpoint`, `park`, `wake`, `release`, `complete`, `fail`,
  `cancel`, `block`, `unblock`). Completion is gated by the
  broker's disposition check (`disposition.rs`): a fix-class
  completion requires merged-PR evidence, so `tasks.result` records
  an outcome the broker verified, not a worker's claim.
- **Read API:** broker `show`/`list`/`timeline`; the hosted query
  door (#12587) reads the metadata store for GitHub-item state —
  a different fact (§3.10), not this one.
- **Legacy mapping:** GitHub issue open/closed state is *not* task
  state; the fallback-claim protocol (§3.13) exists precisely
  because the two are reconciled, never conflated. A task whose
  issue closed is dispositioned through the broker, not by copying
  the issue's state into `tasks.status`.
- **Disposition:** reuse. **Migration/rollback:** none contemplated.

### 3.3 Urgency and profile — `priority`, band labels

- **Owner:** the queue broker for the value; #9199/#9200 for the
  policy that interprets it. **Home:** `tasks.priority` (integer),
  written from the packet's `priority`; band constants
  `EXPEDITE_PRIORITY = 15` / `NEXT_UP_PRIORITY = 25` in
  `validate.rs` (#7030, PR #12464), with broker-side label
  enrichment filling an absent `payload.labels` from the GitHub
  mirror (`turso_queue/label_enrich.rs`, PR #12755) so banding can
  fire at selection.
- **Write API:** enqueue-time only, plus authorized label edits
  accepted through the conflict-aware normalization policy
  (C-R128). **Read API:** claim/selection ordering; Project/board
  projections display, never set, the value.
- **Legacy mapping — the pin has moved:** the census at `a038f084`
  recorded the PM dispatch adapter snapping `15→10`, `25→20`
  (`pm-stack/dispatch/src/validate.ts`). Current main preserves the
  ratified bands losslessly: `PRIORITY_SCALE` includes 15 and 25
  and `RATIFIED_BAND_VALUES = [15, 25]` are never aliased by
  snapping (DST-02, #9452). The ratified spellings `expedite=15` /
  `next-up=25` remain distinct from `do_now`; the historical
  450/1000 charter profile remains a candidate, not a mapping.
  GitHub labels are normalized input or projection by explicit
  mode (#8585 owns de-mirroring) — a label is never the priority
  source of truth.
- **Disposition:** reuse, with the adapter correction already
  landed. **Migration/rollback:** none contemplated.

### 3.4 Lease and claim — `leases`, claim fields, fencing

- **Owner:** the queue broker. **Home:** `leases` (`task_id` PK,
  `worker_id`, `lease_until`, `heartbeat_at`) plus the claim fields
  on `tasks` (`worker_id`, `claimed_at`, `lease_until`,
  `claim_provider`/`claim_model`/`claim_variant`/`claim_session_id`).
- **Write API:** broker `claim` (atomic select-and-claim of the
  same row), `heartbeat`, `release`, and the claim-transfer verbs.
  **Read API:** broker `show`/`list`; REL-02
  ([#9512](https://github.com/developing-today/code/issues/9512),
  OPEN) owns the claim/lease *contract compatibility* design —
  fencing vocabulary (`lease_generation` already exists as a
  `contracts.rs` Command field) and the shared-file sequencing edge
  on `turso_queue.rs` that HYD-05 §9 records.
- **Legacy mapping:** a GitHub fallback-claim marker (§3.13) is a
  claim *record* for broker-down operation; when the broker is up,
  the broker's lease is the fact and the marker is its receipt.
- **Disposition:** reuse; contract refinements extend via REL-02.
  **Migration/rollback:** none contemplated.

### 3.5 Blocked dependencies — `blocked_deps` and dep recheck

- **Owner:** the queue broker for the operational edge set; #9208
  for the typed relationship model the edges project to.
  **Home:** `tasks.blocked_deps` plus the recheck fields
  (`dep_next_recheck_at`, `dep_blocked_until`,
  `dep_recheck_seconds`); the broker's dep-recheck verb re-evaluates
  edges against live GitHub/queue state.
- **Write API:** broker `block`/`unblock` with typed targets (task,
  PR, issue — #3424 piece 2). **Read API:** claim exclusion (a task
  whose blocker is incomplete is not claimable) and `show`.
- **Legacy mapping:** free-text block reasons in
  `progress`/`last_error` are the pre-#3424 form. They are **not**
  dependencies: nothing may parse them into edges. A text reason
  becomes an edge only when a worker re-files it through `block`
  with a typed target. #3424's long-park re-verification is the
  recheck cadence above, not a second store.
- **Disposition:** extend (the typed graph contract is #9208's
  design surface; the broker columns are its operational subset,
  and any new persisted edge field enters via §5).
  **Migration/rollback:** additive only; removing an edge is an
  `unblock` operation with a receipt, never a data edit.

### 3.6 Journal and events — the broker `events` table

- **Owner:** the queue broker. **Home:** `events` (`id`,
  `worker_id`, `task_id`, `kind`, `detail`, `created_at`) — the
  per-task timeline the broker verbs append to and `timeline`
  reads.
- **Mapping, stated as a refusal:** `events` is **not** the #9208
  journal. The journal is the append-oriented decision record of
  the typed-relations design (component 9208); no journal table
  exists on main, and `events` rows do not become journal entries
  by renaming. `events` is also not PM Stack's `shared_events`
  (§3.14), which is a cross-service outcome projection owned by
  the aggregator line. Three surfaces, three facts.
- **Disposition:** reuse for `events`; **absent** for the journal
  (its home is decided by #9208's design, gated by §5 when filed).
  **Migration/rollback:** none contemplated for `events`.

### 3.7 Notes and checkpoints — `checkpoints`, authored notes

- **Owner:** the queue broker for checkpoints. **Home:**
  `checkpoints` (`worker_id`, `key`, `value`, `updated_at`) —
  worker progress state, written by the broker `checkpoint` verb,
  read back by the same worker's resume path.
- **Mapping, stated as a refusal:** a checkpoint is **not** a note.
  Notes — human/agent-authored content attached to work — are
  #9208's field-authority territory (who may write, whose edit
  wins, what syncs to GitHub). Checkpoint values are opaque worker
  state; note text is authored content. Neither may be stored in
  the other's home. GitHub comments are GitHub-owned authored
  content (§3.10); a note that mirrors one cites it, never copies
  authority from it.
- **Disposition:** reuse for `checkpoints`; notes **absent** as a
  Fleet-store fact until #9208 files its row via §5.
  **Migration/rollback:** none contemplated.

### 3.8 Cursors — hydration and ingestion continuation state

- **Owner:** the hydration/activity line (#9207) for the contract;
  the queue store's owner (fleet-queue) for any storage, per
  HYD-05 §9's naming of this record as the schema-owner gate.
  **Home:** **none exists.** No cursor table, column, or file is
  on main; the projector's reconciliation cursors (§3.14) are a
  projection-internal fact, not this one.
- **Contract fixed by HYD-05 (PR #13297), consumed here:** the
  cursor key is FDR-PMO-004's tuple (canonical resource, endpoint,
  query shape, API-version fingerprint, auth-scope class,
  generation); the stored value is the server's `Link: rel="next"`
  continuation verbatim — never a page number, offset, or
  timestamp; states are absent / in-progress / complete / unknown;
  page data and cursor commit atomically or not at all; a
  fingerprint change invalidates to a full snapshot; legacy
  `last_checked_at` is never trusted or converted (note the
  similarly named `tasks.last_checked_at` is a park-recheck field,
  §3.5's machinery — not an ingestion cursor).
- **Disposition:** **absent — gated.** Storage is admitted only
  when (a) HYD-01 (#9458) fixes the manifest generation the key
  scopes to, (b) REL-03 (#9513) fixes the ingest-intent
  coordination where storage is shared with the outbox, and
  (c) the implementation issue files this row's storage amendment
  (table/columns chosen by fleet-queue, additive, in the broker
  store — no new database) via §5.
  **Migration/rollback:** none exists to migrate; rollback of any
  future landing is disable-only per HYD-05 §8 (committed items
  and cursors stay in place; callers return to prior read
  behavior; nothing is deleted).

### 3.9 Operation intent — `fleet.command/v1`, receipts, outbox

- **Owner:** REL-03 ([#9513](https://github.com/developing-today/code/issues/9513),
  OPEN) for idempotency identity and the stable receipt; REL-04
  (#9514) for outbox atomicity. **Home today:** the wire types
  only — `contracts.rs` `Command` (`operation_id`,
  `idempotency_key`, `request_digest`, `expected_work_version`,
  `lease_generation`, …) landed by CLI-01 (#9451, CLOSED). **No
  persisted operation or receipt table exists on main.**
- **Mapping, stated as refusals:** `tasks.result` is a task
  outcome (§3.2), not an operation receipt; `creation_outbox`
  (below) is a scoped creation producer, not the general mutation
  outbox REL-04 designs; a GitHub comment marker (§3.13) is a
  coordination record, not a receipt. REL-06's rule stands: a
  receipt field that must persist beyond REL-03/REL-04's shapes
  enters via §5 before it is written.
- **The one landed outbox fact:** `creation_outbox` +
  `creation_outbox_circuit` in the broker `SCHEMA` — the scoped
  content-creation producer (`doc/fleet/creation-outbox.md`; one
  producer, dedupe by `(repo, dedup_key)`, payload hash, attempt
  and lease fields, `github_number`/`github_url` readback). Its
  scope is creation only; generalizing it by implication is
  refused (census §2).
- **Disposition:** **absent — gated** for operation/receipt
  persistence (REL-03/REL-04 designs, then §5); reuse for
  `creation_outbox` within its scope.
  **Migration/rollback:** none contemplated.

### 3.10 Project fields and item metadata — GitHub, the block, the mirror

- **Owner:** GitHub is the system of record for the item itself
  (title, body, state, labels, Project fields), written through
  the single Project writer / `fleet gh` surfaces. **Home of the
  Fleet-stamped metadata:** the pm-metadata block *inside the
  GitHub body* — the §5 provenance set stamped in full from fleet
  sources only, signed when a key is configured (#12586's
  signature; native create path #9073 slice 9; hosted stamping
  door #12585). The block travels with the item; there is no
  second copy to keep in sync.
- **The mirror:** `pm_metadata_items` (#9074, CLOSED; DDL of
  record `PM_METADATA_DDL` in `gh_metadata.rs`) is a **projection**
  — a queryable index over the blocks, keyed by `artifact_id`,
  carrying the extracted parent/session/lane/host/queue columns
  and `links_json`. It is rebuilt and healed by ingestion
  (#12590: webhook + sweeper re-read the item and refresh the row
  under the block's own stamped tenancy). The query door (#12587)
  reads the mirror and falls back to GitHub for not-yet-ingested
  items; divergence resolves in favor of the body (its §15 rule).
  A mirror row is never edited directly; the write path is
  stamp → GitHub → ingest.
- **Legacy mapping:** items without a block are first-class
  (absence, not error); ingestion ascertains and stamps a block
  for them rather than treating them as corrupt. Caller-supplied
  provenance is replaced wholesale at stamping — provenance has
  exactly one writer, the stamper.
- **Disposition:** reuse. **Migration/rollback:** the mirror is
  disposable by design (drop + re-ingest); the block's rollback is
  the stamper's, per #12585/#9073.

### 3.11 Merge intent — `merge_intents`

- **Owner:** the queue broker (fleet-queue). **Home:**
  `merge_intents` (#8582, CLOSED; DDL of record
  `MERGE_INTENT_DDL` in `merge_intent.rs`: stack id, position,
  repo, branch, base, …) — merge-order coordination for stacked
  work, replayed by the broker's stack machinery.
- **Mapping, stated as a refusal:** merge intent is **not** the
  creator outbox (§3.9) and not an operation intent (§3.9): it
  orders merges of existing branches; it creates nothing and
  carries no remote-effect receipt. The three are separate rows
  because they fail separately.
- **Disposition:** reuse. **Migration/rollback:** none
  contemplated.

### 3.12 Placement sessions — `placement_sessions`

- **Owner:** the placement line (fleet-queue `placement/`).
  **Home:** `placement_sessions` (#13146; the full session JSON in
  `record` is the contract, scalar columns are extractions for
  list/filter). Included here so the broker store's table list is
  fully owned in one place: this is dispatch-session state, not
  queue work state (§3.2) and not a lease (§3.4).
- **Disposition:** reuse. **Migration/rollback:** none
  contemplated.

### 3.13 Degraded-mode surfaces — replica and fallback markers

Two surfaces hold broker facts *away* from the broker store
without owning them; the matrix names them so nobody mistakes
either for a second authority:

- **Local coordination replica** (#8583, OPEN;
  `work/sqlite_replica.rs` under the `work/routing.rs` contract):
  records claims, leases, and checkpoints in a local SQLite file
  while the broker database is unreachable. Reconciliation on the
  broker's return surfaces partition double-claims in a
  `ReconcileReport`; they are **never silently merged**. The
  broker store remains the system of record; the replica is a
  resilience cache with a conflict report.
- **GitHub fallback-claim markers** (`fallback.rs`): when the
  broker cannot be claimed through, workers coordinate by
  machine-readable markers in GitHub issue comments (claim,
  heartbeat, release, completion with the full disposition
  record). The marker stream is the coordination record *for the
  outage window*; the broker consumes it on return. It is not a
  receipt store (§3.9) and not item metadata (§3.10).

### 3.14 PM Stack stores — pointer rows and one live conflict

PM-side stores are inventoried, owned, and migration-planned by
their own line: `pm-stack/SHARED-SCHEMA-MAP.md` (inventory of
record: aggregator stores, projector/link-registry `pmp_` group,
shared entity groups) and `pm-stack/SHARED-SCHEMA-MIGRATION-PLAN.md`
(per-move plan; every move independently shippable and reversible,
no flag-day cutovers). This record fixes only the cross-model
boundaries:

- **Aggregator action store** (`actions.ts`; remote home
  `public.agg_actions` under the #6815 backend switch, file
  backend retained for dev/test/rollback): a **request ledger**
  (§2) owned by the aggregator. It is not canonical applied
  state; downstream consumers apply effects and the outcomes —
  not the requests — are the cross-service facts.
- **Aggregator comments** moved to `public.shared_comments`
  (#1744, PR #12691) in the fleet-platform Turso database — the
  map's `shared_comments` row, landed; the file remains the
  default backend shape.
- **Projector** (webhook intake + hot ledger): a **projection**.
  It never writes back to GitHub/Kaneo/Pad/Grist and owns no
  fact; its delivery ACK is projection intake, never Fleet
  command acceptance. Its reconciliation cursors are internal
  to the projection (not §3.8 cursors).
- **`pmp_links`** (projector/link registry): human-authored link
  writes are unique, non-rebuildable data. Any future migration
  preserves them and all unrelated tables, under the existing
  backup/restore requirements; projection rows being disposable
  authorizes deleting nothing (the slice body's guardrail,
  restated as a matrix rule).
- **Conflict — gated:** #3142's direction starts new PM state in
  Supabase `muse-dev2`, and the map's target groups assume that
  home. The `muse-dev2` project no longer exists (verified
  2026-11-23 in #2796's closure record: the Bitwarden pointer
  resolves, the Management API returns 404, the project list is
  empty); disposition of the dead tooling and pointers is
  [#13245](https://github.com/developing-today/code/issues/13245).
  Until #13245 (or a successor decision) names the target home,
  PM rows keep their current homes, the map stands as the
  inventory of record, and no PM migration step that presumes
  `muse-dev2` may start. This is a conflict row in the oracle's
  sense: two named authorities' statements cannot both be
  executed, and the resolution belongs to the PM line, not to
  this record.

## 4. Name-equivalence refusals (collected)

The done check's negative list, in one place. Each pair is two
facts, however similar the names:

| Claimed equivalence | Refused because |
| --- | --- |
| broker `events` = #9208 journal | `events` is the broker's per-task timeline; the journal is the typed-relations decision record, unbuilt (§3.6) |
| broker `events` = PM `shared_events` | different owners (broker vs aggregator line), different facts (timeline vs cross-service outcomes) (§3.6, §3.14) |
| `blocked_deps` = free-text block reason | text is unqueryable legacy; only typed `block` targets are edges (§3.5) |
| `merge_intents` = creator outbox | merge ordering vs scoped creation; neither creates the other's effects (§3.11) |
| `creation_outbox` = general mutation outbox | the landed outbox is one scoped producer; the general outbox is REL-04's unbuilt design (§3.9) |
| `tasks.result` = operation receipt | a verified task outcome vs REL-03's per-operation idempotent receipt, unbuilt (§3.9) |
| checkpoint = note | opaque worker state vs authored content under field authority (§3.7) |
| GitHub label = priority | labels are normalized input/projection; `tasks.priority` is the fact (§3.3) |
| GitHub issue state = task state | reconciled by disposition, never copied (§3.2) |
| pm-metadata mirror row = the block | the block in the GitHub body is the record; the mirror is a rebuildable projection (§3.10) |
| projector ACK = Fleet command acceptance | projection intake vs the broker's authorized acceptance (§3.14) |
| replica row = broker row | the replica is a resilience cache; conflicts surface, never merge silently (§3.13) |
| fallback marker = receipt | a coordination record for an outage window, consumed by the broker on return (§3.13) |
| `tasks.last_checked_at` = ingestion cursor | park re-verification field vs HYD-05's continuation state, unbuilt (§3.8) |

## 5. The field-admission gate

This is the operative contract the sibling records cite. **A
persisted field that has no row in §3 may not be written** until
its row exists. Adding a row requires, in one review (normally the
PR that lands the design record amendment or the implementation
issue's design note):

1. **The fact**, named at field level (not "metadata", not
   "state").
2. **The owner** — one component, and the design authority that
   speaks for it. If two components both claim it, the row is a
   conflict row and nothing is written until it resolves.
3. **The home** — an existing store from §3, or an explicit
   `absent — gated` entry stating what must land first. A new
   database, queue, or service is never admitted by a row; that
   is a program decision outside this record.
4. **The write and read APIs** — the owner's existing verb surface,
   extended additively if needed.
5. **The legacy mapping** — every existing surface a reader could
   mistake for this fact, with its mapping or its §4 refusal.
6. **Disposition and rollback** — reuse / extend / absent /
   conflict, and the disable-only rollback shape (§7).

Two rows are pre-filed under this gate so consumers can cite
them: HYD-05's cursor storage (§3.8) and REL-03/REL-04's
operation/receipt persistence (§3.9). Both stand `absent — gated`
with their unblocking conditions named in their rows.

## 6. The QTI disposition and what it gates now

#9241 closed with QTI **not located within the enumerated
accessible surfaces** (bounded verdict; searches, surfaces, and
access limits are in its record). Consequences, stated once so no
slice has to re-derive them:

- QTI contributes **no schema, table, or field** to this matrix.
  There is nothing to be compatible *with*; the candidates the
  census named (#3424, #8575, #8579, #8580–#8585) are the
  compatibility evidence base, and §3 crosswalks against them.
- The delivery plan's hard gate — "QTI disposition plus
  #3424/#8575/#8579 and storage-owner agreement before overlapping
  persisted changes" — is therefore satisfiable: the disposition
  exists, §3 is the crosswalk, and §5 is the owner-agreement
  mechanism. Overlapping persisted changes proceed row by row
  under §5, not under a blanket QTI block.
- If QTI material surfaces later (the #9241 record names the
  recovery path), it re-opens **only the rows it actually
  overlaps**, as conflict rows under §5. It does not retroactively
  invalidate rows it does not touch, and no slice may cite its
  possible existence to block unrelated work.

## 7. Migration and rollback rules

For any row that ever moves (none is authorized by this record):

- **Additive and reversible, in the PM plan's move shape:**
  expand (new shape lands, old shape still written/read) →
  backfill → dual-read with the old reader authoritative until
  parity is evidenced → cutover → contract (old shape retired
  only with the owner's sign-off). No flag-day cutovers.
- **Preserve the readers and the receipts:** old readers keep
  working through the move; pending intents, queued tasks,
  in-flight leases, and remote-effect receipts are carried, never
  discarded. Rollback disables the new capability only; it does
  not reset state, delete history, or unwind remote effects that
  already landed.
- **Projections are the cheap direction:** a projection (§3.10's
  mirror, §3.14's hot ledger) is rebuilt, never migrated.
  Non-rebuildable data (human-authored links, authored notes,
  the action ledger's audit trail) is never in the projection
  class and gets the full move shape above, under the existing
  backup/restore requirements.
- **Schema spelling is compatibility surface:** existing queue,
  status, identity, Project-writer, and `fleet gh` spellings are
  preserved; nothing in a migration renames a ratified value
  (§3.3) or a verb.

## 8. Acceptance oracle, specified

The slice's first failing oracle, specified here for the
separately scoped implementation filing (which owns the test file
at the slice's named path,
`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/test_schema_compatibility_matrix.py`):

- **`test_schema_fact_has_one_owner_and_legacy_mapping`** — the
  matrix of §3, encoded as data (one entry per fact: owner, home,
  write API, legacy mapping, disposition). The test asserts:
  every fact named by the slice (queue work ID/state,
  urgency/profile, lease, blocked deps, journal/events,
  notes/checkpoints, cursors, operation intent, Project fields)
  has exactly one owner and one home; every `absent` fact names
  its gating condition; every §4 pair is present as a refusal,
  not a mapping; and the encoded matrix and this record's §3
  agree (the test reads the record, so the two cannot drift
  silently).
- **Negative fixtures:** a fact entered with two owners fails; a
  §4 pair entered as a mapping fails; a persisted field with no
  entry fails (§5's gate, mechanized); a PM row presuming the
  `muse-dev2` home fails while §3.14's conflict stands.
- The test is listed by exact name (`-- --list` equivalent for
  its runner) before execution, per the slice body; a zero-test
  filter passing is not acceptance. The oracle is offline: it
  reads this record and the package census, and touches no
  production store.

## 9. Gates and sequencing

Implementation against this record starts only when **all** of
the following hold:

1. This record is merged.
2. A separately scoped implementation issue is filed against it
   (the natural first filing is the §8 oracle plus the package
   census amendment that mirrors §3 into
   `components/9198-requirement-crosswalk-current-state-census.md`,
   this slice's owned component file — the record and the
   component must not drift, and the oracle is what proves it).
   The existing follow-on task 15944's packet gate is the packet
   owner's to amend; its satisfiable legs (PR #9225 merged,
   #9241 closed) are met, and the remaining leg — #9198, this
   record's parent — is a tracker that closes when its children
   complete, as Lane CC's 2026-11-23 gate note on #9434 already
   established. This record neither amends that packet nor waits
   on the parent, matching the sibling slices' pattern.
3. Per-row gates stand as written: cursor storage on HYD-01
   #9458 + REL-03 #9513 (§3.8); operation/receipt persistence on
   REL-03 #9513 + REL-04 #9514 (§3.9); the PM target home on
   #13245 (§3.14); any `turso_queue.rs` change honors REL-02
   #9512's shared-file sequencing edge.
4. Schema/migration steps, production rollout, and live
   acceptance each remain their own authorized stages with their
   own evidence. Nothing in this record is migration
   authorization (C-R013).

**Schedules:** this record creates, changes, and retires no
schedule on any surface; `doc/fleet-schedule-registry.md` is
untouched.

## 10. Requirement trace (design-level)

The slice's 18 allocated IDs, at design level: C-R012/C-R015 —
the census inspection and QTI-first reconciliation this record
executes (§3, §6). C-R013 — no broad unreviewed migration is
authorized (§9). C-R128 — priority labels derive from the
canonical score under a conflict-aware policy (§3.3). C-R158 —
the phased order (contracts and census before schema-coupled
work) is §9's sequencing. C-R162 — the design deliverables
(architecture map, schema/API contracts, migration plan, gates)
are this record plus §7. H-R020/H-R023 — the issue tree and the
acceptance/migration/rollback test in the handoff are §8/§9.
H-R024/H-R031 — original schema and command surfaces are
preserved in compatibility coverage (§7). H-R046/H-R059 —
acceptance and migration/rollback tests specified in the design
handoff (§8). O-R019 — QTI integration without a competing schema
(§6). O-R048 — risky schema work gated, invariants preserved
(§5, §7). P-R008 — logical responsibilities over existing
infrastructure, no new services (§2, §3). P-R027/P-R055/P-R070 —
end-to-end acceptance (blocked resume §3.5, verified merge/close
§3.2/§3.11, direct completion §3.2, timeout-safe checkpoint
§3.7) is the §8 oracle's scenario basis.

## 11. Non-goals

- No table, column, migration, or store is created, chosen, or
  moved by this record; the two `absent` rows (§3.8, §3.9) are
  decisions to *not yet* persist, with their conditions named.
- No new queue, database, scheduler, service, resolver,
  executable, or GitHub client (the slice body's boundary,
  restated).
- The typed graph (#9208), journal, notes, claim/lease contract
  (REL-02), idempotency (REL-03), outbox (REL-04), and hydration
  manifest (HYD-01) designs are consumed as authorities for
  their contracts; this record decides only ownership and
  compatibility between them.
- PM Stack's internal store moves stay owned by the #1744 line
  and its migration plan; §3.14 fixes boundaries, not PM
  internals.
