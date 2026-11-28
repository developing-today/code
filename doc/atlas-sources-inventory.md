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
| 2. GitHub — all orgs/repos                                                                   | item 2                                                                   | S2                          | LANDED here                  |
| 3. Muse conversations + data                                                                 | item 3                                                                   | S3 (this slice, task 22931) | LANDED here                  |
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

Verified 2026-11-26 against GitHub APIs and the merged #6023 implementation.
This inventory covers the in-scope _our-repos_ estate; external repositories
are a separate, enumerated candidate set and remain gated as noted below.

- **Source and scope:** GitHub API via `gh api` is the system of record for
  issues, pull requests, commits, repository files, and their activity.
  Enumerate all repositories owned by `developing-today`,
  `developing-today-forks`, `Clustered-Computing`, and personal account
  `dezren39`. On an all-visibility scope the first three organizations
  currently expose 47, 38, and 3 repositories respectively:
  `developing-today` is 31 public + 16 private = 47 total (`gh api
orgs/developing-today` reports `public_repos` 31 and
  `total_private_repos` 16, and a paginated `orgs/developing-today/repos`
  recount also returns 47; the earlier version of this section counted
  that org public-only as 31, which understated the estate - #15867).
  The authenticated, paginated
  `/user/repos?visibility=all&affiliation=owner` endpoint returned 546 repos
  whose owner is `dezren39` (536 are also visible from
  `users/dezren39/repos?type=owner`). The current live census is therefore
  634 repos across these four owners, not a current item/file count.
  `#6023`'s first estate run recorded 615 repos from its original three
  owners (the two developing-today orgs and `dezren39`) on 2026-11-16.
  That 615 is not directly comparable to an all-visibility recount: it
  equals today's count for those same three owners only when
  `developing-today` is counted public-only (31 + 38 + 546 = 615), while
  the all-visibility count for the same three owners is 631
  (47 + 38 + 546); the 3 Clustered-Computing repos then bring the
  four-owner total to 634. Recount with paginated `gh api` calls to
  `orgs/<org>/repos?type=all&per_page=100` and the authenticated
  `/user/repos?visibility=all&affiliation=owner&per_page=100` endpoint,
  filtering the latter to `owner.login == dezren39`; deduplicate by
  `owner/repo`. Record the endpoint, permissions, measured date, and returned
  count because visibility can differ by credential.
- **Store and volume:** the completed #6023 baseline extracted **35,819
  comment records** (35,749 issue comments, 69 review comments, 1 review)
  across 615 repos; the run skipped one HTTP-451-blocked repo. These are
  extraction counts reported on #6023, not a fresh count and not evidence that
  35,819 records were promoted into the canonical Atlas corpus. The feed emits
  `atlas-github-comment/v1` JSONL records for the Atlas writer; current
  published Atlas row/chunk counts are **UNVERIFIED**. Counts for issues,
  PRs, commits, and repository files—the broader #9097 item 2 corpus—are also
  **UNVERIFIED**. Recount those with paginated per-repo API enumeration:
  `issues?state=all` (separate `pull_request` objects from issues),
  `pulls?state=all`, `commits`, and the default-branch recursive Git tree
  (report truncated trees as incomplete and count unique tree paths, not blobs
  assumed from a partial response). Preserve the repo list and per-repo raw
  counts so removals, private-repo visibility, and inaccessible repositories
  are explicit.
- **Existing coverage and gaps:** #6023 is closed; PR #10688 merged the
  `atlas-github-comments` extractor and Atlas comment writer, and PR #10834
  added skip-and-log behavior for HTTP 451 repos. The feed is gh-API-only,
  paginated, sequential (rate-limit disciplined), watermark-incremental by
  per-repo `updated_at`, and persists state after each repo. It extracts issue
  comments, PR review comments, and PR reviews with thread context, author,
  and provenance. It does **not** yet inventory or extract full issue/PR
  descriptions, commits, or files. The live source code enumerates the two
  developing-today organizations and `dezren39`, but not `Clustered-Computing`;
  that org's three repos therefore need inclusion in any complete estate run.
  No scheduled recurring run is verified; #7969 is the related runner issue.
- **Update mechanism:** current delivered mechanism is a manually invoked,
  resumable `atlas-github-comments` CLI, not an always-running connector. Its
  comment feed uses API `updated_at` watermarks; the broader issues/PRs,
  commits, and files need source-specific cursors (issue/PR updates, commit
  history, and default-branch tree/object changes) before they can be called
  continuous. Target: webhook/event hooks where supported plus scheduled
  `updated_at`/commit/tree sweeps with durable per-repo watermarks and
  idempotent upserts; advance only after successful source processing, record
  inaccessible repos/errors, and publish per-repo counts. Do not imply the
  comment watermark covers commits or file changes.
- **External set and gates:** `pkgs/atlas-search/docs/external-set-enumeration.md`
  records 31 external candidates from fork upstreams and dependency manifests;
  account activity contributed no candidates within its documented 1,000-item
  search window (not an exhaustive history). The artifact is enumeration only:
  zero external comment text was extracted. External-content ingestion remains
  gated on licensing work #6024 and policy #6052; #7608's license-capture
  implementation does not itself clear those gates.
- **Owner:** #9097 owns this source inventory and continuous-ingestion
  acceptance. #6023 is the existing comments connector/extraction issue; its
  shipped feed is partial coverage, not a substitute for the broader issues,
  PRs, commits, and files inventory/ingestion. #7969 tracks recurring sweep
  runner work. No separate owner issue for the missing object/metadata feeds
  was verified in this slice.

### Group 3 — Claude conversations + data (S3)

Verified 2026-11-28 against `origin/main` in developing-today/code
(S3 slice, task 22931). Three sub-sources, one group. No slice edits
outside this section.

#### 3a. Main + side chats (conversation transcripts)

- **Store:** the Claude platform side. There is no repo-side store
  and no platform export/API path on record (2026-11-23
  sources/consumers research §2.3: "[unverified mechanism]" — no
  export/API path recorded on any issue found). Code check this
  slice: `pkgs/atlas-search/crates/atlas-core/src/` carries no
  Claude-session reader — `memory.rs` stages memory *files* (§3b),
  `github_comments.rs` + `bin/atlas-github-comments.rs` cover
  GitHub only. Session material reaches Atlas today only insofar
  as transcripts land in stores other connectors already read.
- **Fresh volume:** **UNVERIFIED in this slice** — no transcript
  count pulled (no export surface to count against). Recount: once
  an export surface exists, record drop-dir file count + bytes with
  `measured_at`; until then this line stays UNVERIFIED by rule, not
  by omission.
- **Current mechanism:** none.
- **Decided mechanism (S3 decides here):** export-drop sweep. The
  platform offers no hook, so event hooks are impossible until it
  does; the connector is a scheduled sweep over a laptop drop dir
  of transcript exports: intern content-addressed via the #3125
  phase-2 CAS path (working precedent: ChatGPT export, 12,092 files
  / ~2.6 GB interned with `ATLAS-EXTRACTS VERIFY OK`), stage via
  `atlas_stage` v1 with per-file export-date/mtime as `updated_at`,
  idempotent restage on content comparison (the same discipline
  `stage_memory_files` already implements for §3b). Delta key: file
  identity + content hash; full re-ingest is the exception per
  #6028. Internal tier by default; secret-tier transcripts are
  `secret-tier-held` until the local embedder route (#6059) exists.
- **Owner issue:** #9097 (this inventory is its acceptance item 1).
  **No connector issue exists yet for chats** — filing it is S7's
  connector-filings index (acceptance item 2); the issue must cite
  this section's store + recount steps.

#### 3b. Memory tree (MEMORY.md, topic files, daily logs)

- **Store:** the laptop memory tree (design
  `pkgs/atlas-search/docs/memory-integration-design-8567.md` §1:
  "The Claude memory tree (`MEMORY.md`, topic files, daily logs)";
  research §2.3: `~/MEMORY.md`, `~/memory/**`; §2.7:
  `~/memory/topics/**`). The connector scans a tree root for every
  `*.md` file in sorted tree-relative order.
- **Connector: BUILT.** PR #14521 (merged) — "stage fleet memory
  files into fleet-memory unit (#8626)": `scan_memory_files` +
  `stage_memory_files`
  (`pkgs/atlas-search/crates/atlas-core/src/memory.rs`), one
  `items` row plus its `chunks_raw` per markdown file, the file's
  mtime as `updated_at`, replay-safe re-staging on text/mtime
  comparison. Registry corpus `fleet-memory`, owner recorded as
  `muse`, Tier::Internal (design §1 corpus table +
  `MEMORY_CORPORA`). Sibling #8622 slice-1a connector (agent
  memory, `agent-memory` unit, owner `openclaw`): PR #13319, plus
  replay-safe #14024 and native LanceDB reader #15767.
- **Fresh volume:** **UNVERIFIED in this slice** — no laptop read
  run (this lane's seat does not hold Drew's laptop home).
  Recount on the laptop:
  `find ~/memory -name '*.md' | wc -l; du -sh ~/memory; ls -la ~/MEMORY.md`
  with `measured_at`.
- **Current mechanism:** manual connector runs (Atlas-wide current
  state per #9097: manual/script-driven; the only schedule is the
  daily backup timer).
- **Target mechanism:** scheduled sweep over the tree root on file
  mtime (mtime is already the connector's `updated_at`, so each
  poll is incremental by construction) + the #7969 runner once it
  lands; internal tier.
- **Owner issue:** #8626 (CLOSED 2026-11-25; fleet-memory half
  landed via #14521; sibling corpus connectors zettelkasten #14594,
  repo-docs #14652, research-reports #14872 — all merged). #8622
  (CLOSED 2026-11-24) owns the agent-memory connector. #9097 owns
  the inventory acceptance.

#### 3c. Artifacts / documents produced for Drew

- **Store:** UNVERIFIED as a separate store — no platform-side
  artifact location identified in this slice. Artifacts that land
  as files in the memory tree or repo doc trees are swept with
  those corpora (§3b / #8626 repo-docs); anything platform-side
  only rides the §3a export-drop path once it exists.
- **Fresh volume / mechanism:** UNVERIFIED / none standalone.
  Recount: inventory platform-side artifact locations on the
  laptop; if a location holds Drew's documents outside the two
  paths above, it gets its own store entry and connector issue
  via S7.
- **Owner issue:** #9097; connector filing (if a standalone store
  is found) via S7's index.

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
