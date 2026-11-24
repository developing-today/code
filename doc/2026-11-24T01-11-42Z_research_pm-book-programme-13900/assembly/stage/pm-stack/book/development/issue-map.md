# Project Management: prioritised issue map and dependency map

Snapshot: 2026-11-23 (America/Chicago). States for #13900 and #13844 were read through `fleet gh api` on 2026-11-23; bridge states were read through `fleet gh api` search the same day; register items carry the design register's own record of 2026-11-11 and are marked as such. A state in this chapter is a state at the snapshot. Read the live issue and its newest comments before acting on any item here.

## Prioritised list

### 1. The book programme — #13900

**#13900, "Project Management book — fifth house book, coverage audit, sidelines, Space" — open** at the snapshot (created 2026-11-23, last updated 2026-11-23 18:47 CST). Drew's directive of 2026-11-23 opened the programme. Its completion test, in Drew's words as recorded on the issue: every worker in the estate and every Turso database is covered in the PM book or in one or more of the four pre-existing books. The coverage ledger is the test's evidence: 22 workers censused (3 covered, 19 gap) and 50 Turso databases confirmed live (34 covered, 16 gap), with every gap assigned to the PM book or to an owning book. See [Keeping this book current](keeping-this-book-current.md) for the ledger's standing role after the programme closes.

The programme's slices, in the order its own record states the process — survey before structure, structure before prose, mechanical coverage audit before done:

| Slice | State at the snapshot, per the programme's comments on #13900 |
|---|---|
| PM-SURVEY: the stack's real surface, from the code | Complete. 50 primary pages and routes, 36 components, question sets F1–F8, disagreements D1–D14 |
| PM-CENSUS: the coverage ledger | Posted. The worker and Turso census above |
| TURSO-SETTLE: the ledger's 50-against-52 count discrepancy | Resolved. A live `turso db list` returned exactly 50; the audit's internal-shard figure was the sole error |
| LLM-EVIDENCE: the dated evidence pack for the LLM-plans chapters | Complete. 30 plan rows, 6 tier rows, 16 evidence items, every figure dated at its source |
| Writing lanes: the book's chapters, section by section | In progress at the snapshot. This chapter is one lane's output |
| Sidelines: the mcpx door Worker chapter for the mcpx book; the commander door and bridge coverage for the fleet book | Assigned in the programme body; the gaps they close are named in the ledger |
| Placement and deploy: the book at `pm-stack/book/` with `pm-stack/book.toml`, a flake output and build workflow in the fleet book's shape, and a subpath in the single Pages deploy | Recorded in the programme's placement correction and skeleton. The book's guessed placement in the issue body, `pkgs/pm-stack/docs/`, is superseded (D15) |
| The Project Management Space | After the book lands, per the programme body. Not started at the snapshot |

One coordination item lands on this programme from outside it: a comment on #13900 from the Code Book programme (#13905, open at the snapshot) records two requirements from Drew's 2026-11-23 directive for every book, this one included. The comment is the record; this programme incorporates the requirements, and the Code Book lane does not edit this book.

### 2. The Turso-to-D1 programme — #13844

**#13844, "Turso-to-D1 migration audit + programme" — open, labelled `status:in-review`** at the snapshot. Research and design only at its own record: no migrations have been made. The audit record landed through PR #13845 as a dated design record in `doc/`. This programme owns the decisions the data-stores chapter of this book names for three stores: `fleet-platform`, `fleet-mirror`, and the frozen eight staging and recovery copies.

Its measured findings, from the issue body: 50 Turso databases; `fleet-platform` is 87% of all estate writes (57.4M rows in the audit period) and is owned end to end by the PM stack's Workers; `fleet-queue` is 81% of all reads, a query-shape problem, not a store-location problem; `atlas` is 73% of storage and stays on Turso as the native vector store. Its groups, in its stated slice order:

| Order | Group | Content |
|---|---|---|
| Slice 1 | G1 MOVE-CLEAN | `fleet-platform` telemetry mirror and aggregates (`agent_mirror_*`, `agent_quota_records`, `agg_*`) to one D1 database, dual-write cutover. Derived, rebuildable, Worker-only access; the audit's recommended first slice |
| Slice 2 | G2, then G3 MOVE-CLEAN | The `fleet-platform` ingest families, then its operational ledgers (`alerts_*`, `decision_set_events`, `ci_events`, `ideas_*`, `orch_*`). G3 completes the `fleet-platform` evacuation |
| Slice 3 | G4 read-shape fix | `fleet-queue` stays on Turso; the fix is indexing and cursoring its pollers. The audit names this slice the real downgrade enabler |
| Later | G5 MOVE-WITH-BRIDGE | `fleet-queue` read-side mirrors: D1-first reads, Turso fallback, watermark sync |
| Later, low priority in the audit's own words | G6 MOVE-WITH-BRIDGE | `muse-meta` `coord_events` and small tables; `coord_tasks` and leases stay |
| Opportunistic | G8 MOVE-CLEAN | Small stores: `notes-submissions`, `bedside-and-away`, `voice-line-ledger`, `fleet-dispatch-jobs`. `great-star-lord` stays, by standing keep and as a third-party application's database |
| Housekeeping | The frozen eight | The 2026-11-17 and 2026-11-18 Supabase recovery and staging copies. The audit recommends export to object storage and delete, rather than migration; the decision is the programme's to take |

The audit's constraint stands over the whole order: moving G1–G3 alone does not permit a Turso plan downgrade, because the remaining reads and storage stay over the lower plan's lines; the downgrade needs the G4 read-shape fix as well. The programme's design review (AUDIT-7, recorded on the issue) returned a verdict of sound, with one naming note: slice 2's retirement of `fleet-platform` after parity depends on slice 1's dual-write and parity pattern proving out, and that dependency was implicit in the audit until the review named it. Where this programme's D1 direction and the Supabase direction of the shared-schema line name the same `agg_*` family, this book records a disagreement (D19) and states neither as settled for the overlap. See [PM stack research and implementation roadmap](program-roadmap.md).

### 3. The bridge programme

The bridge's durable coverage home is the fleet book, not this book; this book carries light coverage and coordinates. The programme body for #13900 records the shape: bridge coverage in the fleet book plus a `fleet` subcommand, with a follow-up issue filed by this programme, and the standalone bridge build in `pkgs/message-bridge` running separately — coordinate, do not rebuild. The coverage ledger records the `message-bridge` Worker as covered for concept and CLI in the fleet book, with its Worker deployment detail as the fleet sideline extension; the Worker is deployed by `message-bridge-deploy.yml`, and its standalone build was in flight at the census.

Bridge items read on 2026-11-23:

| Item | State |
|---|---|
| #12032, "feat(message-bridge): one bridge for Claude and ChatGPT, on bridge.security.cab" | Closed |
| #13213, "fleet bridge: per-verb --help for every bridge verb" | Closed |
| #13895, "Bridge: class/name notes, public wake signal, and Archer hook" | Open |
| #13907, "fleet bridge: send/get/inbox/watch for class/name notes — replacing ad-hoc bridge access" | Open |

Within the PM stack's own tree, `pm-stack/private-laptop-bridge/` is bridge smoke tooling; the production surface it serves beyond smoke tests is UNVERIFIED in the survey.

### 4. Open design items from the register

These items come from the design register of 2026-11-11 and its companion records. Their states are the register's record at that date; the snapshot did not re-read each one.

**The coherence set.** Epic #1184, the PM stack coherence epic, open at register time, with its 34 child issues #1134–#1183 as the standing follow-up set. Named siblings in the register's census: #1182 (one public URL behind Cloudflare Access, settled, not implemented, and recorded as needing Drew's go), #1253 (one identity and SSO, design), #1260 (SLOs and self-healing, design), #1198 (one status taxonomy), #1281 (WIP limits and flow analytics), #2576 (aggregator ingest fail-closed contract, design), #1744 (centralise service data in one schema — the shared-schema line), #3989 (move stateful services off the laptop, host failover — parent of the store-migration plan).

**The notification and decisions follow-ups.** #8569 and #8570 (notifications phases 2 and 3), #8777 (the 2026-11-11 SIGTERM that killed the aggregator and dispatch units mid-session, with no self-heal), and #1734 (per-source lag counts for the health page). Their order is the phase and follow-up order in [PM stack research and implementation roadmap](program-roadmap.md).

**The pending design rulings (register P2.3).** Twelve design rulings sit in the decisions ledger awaiting Drew, listed in the register by ledger id: `fleet-tiers-2230`, `lifecycle-verdict-2311`, `replica-operator-806`, `design-adoption-1678`, `watcher-scope-1441`, `shutdown-delete-1414`, `luna-route-terra-low`, `atlas-secret-remediation-8479`, `backups-provisioning-8584`, `turso-remedy-5468`, `code1394-mimo-flag-superseded`, `bws-runner-5451`. The ledger entry for the needs-user design carries one further pending decision: how auto delegation mode is set, with the proposed default recorded as per-item explicit opt-in only, and the access and policy classes hard-excluded from auto under any setting.

**The open lineage questions.** The lineage manifest records the multi-carrier epic labels — including `epic/pm-stack` and `epic/pm-stack-coherence` — as awaiting Drew's pick of a single home epic, and the survey records one open design question from that manifest: #1518, define code and config embeddings — unit, metadata, usage, and whether they share the prose embedding dump. The PM stack consumes no embeddings in its own code; that question belongs to the Atlas surface it names. See the models chapter of this book for the boundary.

## Dependency map

Edges only where a source states them. An edge reads "the first item waits on, builds on, or is gated by the second".

| Item | Depends on | Source of the edge |
|---|---|---|
| #13900's census and ledger | #13844's audit estate list: the ledger starts from the audit's 50-database list | #13900 programme body |
| #13900's chapters | The survey, then the structure: survey before structure, structure before prose, audit before done | #13900 programme body |
| #13900's completion test | The sideline chapters in the owning books, for the gaps assigned there | #13900 programme body and the coverage ledger's assignments |
| The Project Management Space | The book landing first | #13900 programme body: the Space comes after the book lands |
| The decisions page (P2.2) and #8568's answerable pushes | The pending-questions ledger (P2.1) as substrate | `pm-stack/decisions-ledger-system.md` |
| Notifications Phase 2 (#8569) and Phase 3 (#8570) | The shipped Phase 1 (#8568), in phase order | Design register, entry P1.12 |
| The PWA and Web Push design (P1.13) | Branches on #1182, the one-public-URL decision, because the access model's implementation was gated on Drew's recorded go | Design register, entry P1.13 |
| #13844 slice 2 (retire `fleet-platform` after parity) | Slice 1's dual-write and parity pattern proving out | AUDIT-7 design review, recorded on #13844 |
| A Turso plan downgrade under #13844 | The G4 read-shape fix, in addition to G1–G3 | #13844 issue body |
| Store-migration step 2, then step 3 (#6816) | Step 1's inventory and plan; the fence and exact-parity gate govern step 2's promotion | `pm-stack/STORE-MIGRATION-PLAN-6816.md` |
| The #3989 follow-on stores | The four #6815 stores landing first; #3989 is also #6816's parent | `pm-stack/SHARED-SCHEMA-MAP.md`; `pm-stack/STORE-MIGRATION-PLAN-6816.md` |
| The shared-schema target groups | #3142's direction, which supersedes #1744's one-local-libSQL default | `pm-stack/SHARED-SCHEMA-MAP.md` |
| Bridge coverage in this programme | The standalone `pkgs/message-bridge` build, as a coordination edge: the programme coordinates with it and does not rebuild it | #13900 programme body |

No other edges are stated in the sources this chapter grounds on. In particular, no source surveyed states an edge between #13900 and #13844 beyond the census's use of the audit's estate list, and no source states an edge between the bridge items #13895 and #13907 beyond their shared subject; this map records neither.

Sources. `fleet gh api` reads on 2026-11-23: `repos/developing-today/code/issues/13900` (state, title, and its slice comments), `repos/developing-today/code/issues/13844` (state, label, body, and the AUDIT-7 comment), issue search for the bridge items, and #13905 as named in the #13900 comment; `~/workspace/pm-book/coverage-ledger.md`; `~/workspace/pm-book/tracking-issue-body.md`; `~/workspace/pm-book/skeleton.md`; `~/workspace/pm-book/placement-correction.md`; repo documents `pm-stack/design-register-2026-11-11.md` (§3, P1.3, P1.11, P2.3), `pm-stack/lineage-manifest-2026-11-12.md`, `pm-stack/decisions-ledger-system.md`, `pm-stack/SHARED-SCHEMA-MAP.md`, `pm-stack/STORE-MIGRATION-PLAN-6816.md`; survey file `~/workspace/pm-book/survey/roadmap.md` (§3, the #1518 question).
