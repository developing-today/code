# Atlas sources inventory (for #9097)

Parent: [#9097](https://github.com/developing-today/code/issues/9097) — continuous
ingestion, workstream E of [#8983](https://github.com/developing-today/code/issues/8983).
Acceptance item 1 of #9097: every source, its store, volume, update mechanism
(hook/sweep), and owner issue — including the thinking-traces decision.

**Ownership of this file:** one section per #9097 source group. Each slice owns
its group section; do not edit another slice's section. Append-only: a slice
adds its section (or extends it) without rewording settled sections.

| Group                                                                                        | #9097 item                                                               | Slice                       | State                        |
| -------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ | --------------------------- | ---------------------------- |
| 1. Turso queue / PM-stack                                                                    | item 1: fleet-queue data, PM-stack aggregator state / boards / reconcile | S1 (this slice, task 22929) | LANDED here                  |
| 2. GitHub — all orgs/repos                                                                   | item 2                                                                   | S2                          | PLACEHOLDER — not this slice |
| 3. Muse conversations + data                                                                 | item 3                                                                   | S3                          | PLACEHOLDER — not this slice |
| 4. OpenCode sessions + data (incl. thinking-traces decision)                                 | item 4                                                                   | S4                          | PLACEHOLDER — not this slice |
| 5. ID objects                                                                                | item 5                                                                   | S5                          | PLACEHOLDER — not this slice |
| 6. Inventory-pass extras (notes/zettel, Reports Space, ask-ledger, quota/burn, laptop state) | item 6                                                                   | S6                          | PLACEHOLDER — not this slice |
| 7. Continuous-update design + connector filings index                                        | acceptance items 2–3                                                     | S7                          | PLACEHOLDER — not this slice |

Corpus anchor (last verified census, NOT a live count): **18,325 sessions /
122,124 messages / 107,639 chunks** (2026-10-31 publish, recorded on #3125;
Turso monolith ~7.3 GB over the same 107,639 chunks + 29 topic shards per
#9092/#9097, 2026-11-12 — explicitly **without a verified recount**). No number
in this file may reuse the anchor as a current measurement; §3 defines the
recount procedure that replaces it.

---

## Group 1 — Turso queue data + PM-stack data (S1)

Verified 2026-11-24 against `origin/main` in developing-today/code.
LiveReads in this slice: queue-broker `health` + `list` (broker answers);
repo paths below read from the tree. Turso row counts and PM-store sizes that
need a store credential are recorded as last-verified/UNVERIFIED with the
exact recount command — never backfilled from the anchor.

### 1a. Fleet-queue data (queue broker's Turso backend)

- **Store:** dedicated Turso (libSQL) database **`fleet-queue`**, the queue
  broker's only backend. Code of record:
  `pkgs/fleet/crates/fleet-queue/src/turso_queue.rs` (+ `turso_queue/` —
  `core.rs`, `claim.rs`, `lifecycle.rs`, `admin.rs`, `placement_sessions.rs`,
  `stacks_outbox.rs`, …). Schema-created tables include `tasks`, `leases`,
  `lanes`, `workers`, `checkpoints`, `repo_task_lane_ledger`, `events`,
  `creation_outbox` (+ circuit), `reprioritize_sync_outbox`, `merge_intents`,
  `placement_sessions`, `actions_exec_sessions` (see `core.rs` DDL).
  Queue = coordination only (claims, leases, heartbeats); durable truth lives
  on GitHub (per `infra/fleet-queue-backup/README.md`). Analytics/backup copy:
  Cloudflare D1 `fleet-queue-backup` (daily export) + Turso home
  `d1-backup-fleet-queue-backup` (weekly class-D leg).
- **Fresh volume (measured live 2026-11-24 ~06:25 CST via `fleet queue health`,
  broker-resolved project `fleet-queue-dezren39.aws-us-east-1.turso.io`):**
  `repo-health-code-pilot`: 178 queued / 116 claimed / 273 parked / 279 blocked
  / 222 dep-blocked / 13,635 done / 788 cancelled / 5 failed / **15,274 total**;
  `repo-health-opencode-pilot`: 8 / 2 / 5 / 1 / 1 / 1,511 / 99 / 2 / 1,628;
  `repo-health-openchamber-pilot`: 14 / 2 / 4 / 3 / 3 / 1,721 / 75 / 0 / 1,819;
  `repo-health-nix-pilot`: 30 / 15 / 7 / 15 / 4 / 813 / 176 / 0 / 1,056.
  Per-table row counts inside the Turso DB (SELECT COUNT(*) per table):
  **UNVERIFIED in this slice** — needs the queue-store read path; see §3 step 2.
- **Current update mechanism:** manual. No queue→Atlas connector exists
  (confirmed by the 2026-11-23 sources-inventory research: "#9097 item 1 …
  no connector built"). Queue content changes constantly via broker verbs
  (claim/heartbeat/checkpoint/block/complete); nothing stages it to Atlas
  today. Atlas-side staging path that a connector would land into exists in
  code (`atlas_stage` v1, `pkgs/atlas-search/crates/atlas-core/src/stage.rs`;
  promotion gated on the embedding-model contract).
- **Target update mechanism:** broker-watermark delta sweep + event hook where
  the source supports it, per #9097's rule (event hooks where possible,
  scheduled sweeps otherwise; delta-only #6028; ingest watermark doubles as
  the corpus-change signal for cache policy). Concretely: poll per-(queue,
  stream) cursors over `tasks`/`events`/`checkpoints` (incremental, idempotent
  upserts keyed by task id + event seq), plus a `events-append`-fed hook for
  near-real-time task transitions; backfills bounded per the #6019 runbook
  (one queue + one range per run, human approval beyond 7 days / 10,000 items).
  Task-attached session transcripts inherit session classification rules;
  queue rows default to internal tier; secret-tier rows are `secret-tier-held`
  until the local embedder route (#6059) exists.
- **Owner issue:** #9097 (this inventory is its acceptance item 1). **No
  per-source connector issue exists yet for queue/PM** — filing it is
  acceptance item 2 of #9097 and belongs to slice S7's connector-filings
  index; the connector issue must cite this section's store + recount steps.

### 1b. PM-stack data (aggregator state, boards, reconcile)

Three sub-sources, one group. None has an Atlas connector today.

- **Aggregator state (cross-repo read view).**
  - Store: `pm-stack/aggregator/` — read-mostly projection over GitHub/Kaneo/
    Pad/Grist; disposable by design. Own local stores: dashboard-local comments
    (`src/` comments store), alerts store (`src/alerts.ts`), decision-events
    sync (`src/decision-events-sync.ts` → Turso `fleet-platform` ref), plus
    per-page render registry. Code: `pm-stack/aggregator/src/turso-stores.ts`
    (`getTursoStoreClient`), `src/push.ts`, `src/secrets.ts`.
  - Fresh volume: **UNVERIFIED in this slice** — no row-count query run
    against the aggregator's stores (needs store credentials + which Turso DB
    backs each `getTursoStoreClient` caller). Recount: §3 step 3.
  - Current mechanism: none (no export feed; the aggregator reads live APIs).
  - Target: scheduled sweep over the aggregator's own stores (comments,
    decision events, render registry) on each store's natural cadence +
    webhook where available; delta via per-store cursors; internal tier by
    default.
- **Boards (Kaneo execution boards + roadmap files + Grist mirror).**
  - Store: Kaneo (per-repo projects; upstream one-repo-per-project limit) as
    execution board; `planning/roadmaps/*.md` (roadmap of record, incl.
    `planning/roadmaps/pm-stack.md`) as the plan layer; Grist "Program
    roadmap" table as mirror (roadmap-sync job mirrors files→Grist; a
    Grist-only row is seed/mirror, never truth). Design record:
    `doc/2026-10-24T11-05-38Z_design_project-management-stack/`.
  - Fresh volume: **UNVERIFIED in this slice** — Kaneo task counts per project
    - roadmap-row counts + Grist row counts not pulled (needs Kaneo/Grist
      credentials). Recount: §3 step 4.
  - Current mechanism: none (roadmap-sync mirrors files→Grist only; nothing
    feeds Atlas).
  - Target: sweep over Kaneo tasks per project + roadmap files at rest (git is
    the cursor: new/changed rows per commit) + Grist mirror diff; board moves
    are activity events (comment-activity projection already exists in the
    projector — see below — and is the natural hook point).
- **Reconcile / projector (link registry + comment projection).**
  - Store: projector's Turso-backed projection (`pm-stack/projector/`):
    webhook intake (`POST /github/webhook`, delivery-GUID dedupe), reconcile
    reads (`POST /reconcile`, `bun run src/cli.ts reconcile`, per-(repo,
    stream) cursors, idempotent upserts), link registry (`/links`), joined
    views `pmp_v_links_items` / `pmp_v_subject_activity`. README:
    `pm-stack/projector/README.md`; contract #4302 (projection writes only
    what the event proves).
  - Fresh volume: **UNVERIFIED in this slice** — link/comment/delivery row
    counts not queried (needs projector DB credential). Recount: §3 step 5.
  - Current mechanism: the projector already reconciles GitHub→itself on
    webhook + poll; nothing projects projector→Atlas.
  - Target: consume the projector's own cursors/views as the delta feed
    (per-(repo, stream) cursors make each poll incremental already — the
    Atlas connector reuses them rather than re-polling GitHub); webhook
    delivery-GUID log doubles as the near-real-time hook. Internal tier;
    PR-interior comments included (the projector already captures what Kaneo
    never syncs).
- **Owner issue (all of 1b):** #9097. Same connector-filing note as 1a: no
  PM-stack connector issue exists; S7 files/links it.

---

## Recount procedure (per-source, so totals cannot freeze silently again)

Run on the laptop (the seat that holds the credentials). Every run records
`measured_at` (UTC), the exact command, and the raw number — then updates that
source's section above. Never copy a prior run's number forward. If a step
cannot run (credential/token/DB unreachable), record the step as FAILED with
the error and keep the previous number untouched and dated.

1. **Canonical corpus anchor replacement.** Query the canonical DB directly
   (laptop path `~/atlas/turso-atlas/data/atlas.db`, cf. `nixos/home/atlas-backup.sh`):
   `sqlite3 ~/atlas/turso-atlas/data/atlas.db "SELECT COUNT(*) FROM sessions; SELECT COUNT(*) FROM messages; SELECT COUNT(*) FROM chunks;"`
   plus Turso monolith size (DB size + chunk count over HTTPS with the
   read-only token resolved at runtime — env, Bitwarden `atlas-ro`, or
   turso-CLI-minted `--read-only`; never stored in the repo). Compare against
   18,325 / 122,124 / 107,639 (2026-10-31): any drift retires the anchor for
   that source; identical numbers still get a fresh `measured_at`.
2. **Fleet-queue store.** `fleet queue health` (all queues — the broker
   resolves the Turso project itself) for the headline per-queue totals, then
   per-table counts over the queue-store read path
   (`SELECT name FROM sqlite_master WHERE type='table'` → `SELECT COUNT(*)`
   per table: `tasks`, `leases`, `lanes`, `workers`, `checkpoints`,
   `repo_task_lane_ledger`, `events`, `creation_outbox`, `placement_sessions`,
   `actions_exec_sessions`, `merge_intents`). This slice measured the headline
   half (see §1a); the per-table half remains open until a credentialed read
   runs.
3. **Aggregator stores.** For each `getTursoStoreClient` caller in
   `pm-stack/aggregator/src/` (comments, alerts, decision events, render
   registry): record the backing DB + table, then `SELECT COUNT(*)` (+
   `MAX(updated_at)` where the table carries one) with the store's runtime
   credential. Record DB, table, count, watermark triple per store.
4. **Boards.** Kaneo: per-project task counts via the Kaneo API/export for
   each connected repo project; roadmap files: `git log --oneline -- <file>`
   cursor + row counts per `planning/roadmaps/*.md`; Grist: row count of the
   "Program roadmap" table. The three numbers must reconcile directionally
   (files = truth; Grist = mirror; Kaneo = execution layer) — a Grist-only
   surplus is seed/mirror, never a new source.
5. **Projector.** Against the projector DB: per-table counts for links,
   comments (by `kind`), deliveries/delivery-errors, plus
   `MAX(cursor)` per (repo, stream) from the reconcile state. Reuse the
   projector's own per-(repo, stream) cursors as the next sweep's `cursor_from`.
6. **Recording rule.** Each source section keeps `measured_at` + command +
   raw result. A scheduled sweep (slot: with the #7969 runner once it lands;
   until then, a dated manual re-run no older than the corpus publish it is
   compared against) re-runs steps 1–5. A publish whose numbers equal the
   previous publish without a fresh run is a defect, not a steady state.

---

## Placeholders for other slices (do not fill in S1)

### Group 2 — GitHub (S2)

Owner: S2. Scope per #9097 item 2: all issues/PRs/commits/files for all orgs
and repos in scope (developing-today, developing-today-forks,
Clustered-Computing, personal), extending #6023 beyond comments. Existing
ground: #6023 closed (comments extraction), #10688 writer landed
(`atlas-github-comments`), #7608 license capture landed; gates #6024/#6052
(external half) OPEN.

### Group 3 — Claude conversations + data (S3)

Owner: S3. Scope per #9097 item 3: main + side chats, memory tree, artifacts.
No export/API mechanism on record (2026-11-23 research: UNVERIFIED) — S3
decides the mechanism in this section.

### Group 4 — OpenCode sessions + data (S4)

Owner: S4. Scope per #9097 item 4: OpenCode session store (laptop server
127.0.0.1:13420) + OpenChamber session data, via session-close hook or store
sweep. Must include #9097's explicit **thinking-traces investigation +
recommendation** (volume, value, classification per workstream D) — undecided.

### Group 5 — ID objects (S5)

Owner: S5. Scope per #9097 item 5 + #6062 publication line: laptop redb, Turso
ID DBs, hosted stores as they arrive — into Atlas (reverse of the designed
Atlas→ID #6062/#11182–#11186 direction). End state per #9109 Part C:
automatic vectorization (Snowflake + second embedder) feeding ID search.

### Group 6 — Inventory-pass extras (S6)

Owner: S6. Scope per #9097 item 6: notes/zettel, Fleet Reports Space records,
ask-ledger logs, quota/burn records, laptop state stores; related open
connectors #8622 (memory-lancedb), #8626 (memory/notes/docs/research), #9576
(reports & pages). "Anything missed gets its own connector issue" — S6 lists
the disposition of each candidate from research §2.7 (Gmail paused 2026-11-22,
Drive, Slack allowlist, CI runs, non-Atlas Turso DBs).

### Group 7 — Continuous-update design + connector filings index (S7)

Owner: S7. #9097 acceptance items 2–3: per-source connector issues filed (or
existing ones linked: #6023, #8626, …) with acceptance criteria each; the
continuous-update design (hooks + sweeps + watermark); the canonical corpus
recount procedure's promotion from §3 of this file into the standing runbook.

---

## Verification log (this slice)

- 2026-11-24: `gh issue view 9097` (OPEN, labels kind/feature task/small-fix
  status/blocked queue/blocked work/assigned priority/normal effort/l
  status/triaged epic/atlas-program, acceptance unchecked); #8983 program home
  confirmed via #9097 body refs.
- 2026-11-24: tree reads on `origin/main` — `turso_queue.rs` header (fleet-queue
  DB identity, table set, broker-only backend), `turso_queue/` module list,
  `core.rs` DDL table names; `pm-stack/aggregator/` (read-mostly projection,
  own comment/alert/decision-event stores), `pm-stack/projector/README.md`
  (webhook + reconcile + link registry + views), `planning/roadmaps/pm-stack.md`
  (Kaneo / Pad / Grist mirror / aggregator roles), `infra/fleet-queue-backup/`
  (D1 copy + Turso home, queue = coordination only).
- 2026-11-24: live `fleet queue health` + `fleet queue list` via the broker —
  per-queue totals recorded in §1a. Per-table Turso counts, aggregator/Kaneo/
  Grist/projector credentialed counts: NOT run (need store credentials) —
  recorded as UNVERIFIED with exact commands in §3, per the "never assumed"
  rule.
- Prior art cited, not duplicated: #6019 operations design (§§2–6: source
  classes, delta→filter→land→promote pipeline, guardrails, plan ceiling,
  runbook); 2026-11-23 Atlas sources/consumers research (§2.1 item-1 holder
  - status "no connector built", §2.7 candidates, §4 gates).
