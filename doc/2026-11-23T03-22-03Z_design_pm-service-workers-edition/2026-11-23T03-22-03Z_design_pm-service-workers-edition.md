# PM service, Workers edition (component A)

Status: proposed V1 design for component A of #9078 (hosted editions program), issue #9079. The service-shape and writer-of-record recommendations (§1, §5) are **proposed, not decided** — ratification stays with the design mill or Drew, as with component F's placement recommendation. Everything else (data homes, compatibility statement, invariants) is written to hold under either ratification outcome, so implementation slices filed from this design do not need to wait for ratification unless a slice says otherwise.

## Context and constraints

The PM service today is the laptop stack: the aggregator (Bun/TypeScript, `pm-stack/aggregator`) is collector + dashboard + API in one app, `project-reconcile.sh` is the boards' single writer, and the aggregator's stores have already standardized on Turso (#6815: `PM_AGG_STORE_BACKEND=turso` live). This design is the hosted edition of that service: the same capabilities re-implemented in TypeScript as one hosted service on Cloudflare Workers, consuming the SAME data model — the board registry, hierarchy edges, primary-parent rule, and Metadata model from #9060's merged design (`doc/2026-11-12T22-50-00Z_design_pm-board-hierarchy-genealogy`) — so the hosted and laptop editions read the same world.

Binding inputs, treated as inputs rather than decisions:

- **Tier-1 parity (Drew amendment, 2026-11-12, on #9079).** The Workers edition is a primary edition alongside the Rust/native stack. All framing is parity + deliberate divergence; no reduced-sibling scope assumptions. "V1 parity list" below therefore means "the surfaces that make this a workable equivalent," not "a smaller product."
- **Component E (#9083, design merged).** Tenancy planes: one central tenancy DB (accounts, bindings, registry, roles — no content, no keys, no tokens), one Turso DB per tenant, one public DB. Durable Objects are per-tenant coordination keyed by `binding_id`; jurisdiction is a property of the object's name, fail closed. This design sits on E's planes and invents no parallel tenancy.
- **Component F (#9084, design merged).** Media/attachments are content-addressed references in the tenant DB; bytes live in the object plane; access is by live reference + minted capability, never by hash. PM content follows F's rule unchanged.
- **Identity is component H (#9086) consuming #9077.** Hosted editions authenticate via the SSO relying-party integration and resolve sessions to E's `binding_id`. This design consumes that seam; it does not design auth.
- **Settled neighbours.** #9062 (collector multi-board pool) and #9070 (genealogy read model + sentence renderer) are landed on the laptop edition; #9071 (dashboard surfaces) is in flight there. Parity is with the #9060 design first and the laptop implementation second — where a laptop surface is itself unfinished, the hosted edition implements from #9060 directly rather than waiting.

## 1. Service shape (Q1) — recommendation (proposed)

**One Worker, one hostname, route modules — not several Workers.** The aggregator is already one TypeScript app with collector, API, and dashboard modules; splitting the hosted edition into several Workers would multiply deploy/config surfaces and force service bindings for calls that are function calls today, buying isolation the PM service does not need (its modules share one data model and one auth seam). Modules:

| Module | Kind | Job |
| --- | --- | --- |
| `front-door` | stateless handler | SSO session → `binding_id` resolution (H/#9077 seam), jurisdiction routing per E, fail closed |
| `collector` | Cron Trigger + `IngestDO` | GitHub ingest (§3) |
| `read-api` | stateless handler | The #9060 §6 API shapes (§4) served from the tenant Turso DB |
| `dashboard` | stateless handler | Server-rendered surfaces (§4), same pages recipe as the laptop dashboard |
| `reconcile` | `ReconcileDO` | Hosted reconcile runs + writer-of-record enforcement (§5) |
| `live` | `BoardDO` | Live board state + presence (§2) |

**Durable Object inventory** (all per-board, keyed by the #9060 registry `boardId`; tenant-scoped objects additionally sit under E's jurisdiction rule):

| DO | Cardinality | Key | Responsibility | Explicitly not |
| --- | --- | --- | --- | --- |
| `BoardDO` | one per board | `boardId` | Live board state: the current snapshot's derived view state, WebSocket presence/subscriptions (hibernating), fan-out of snapshot-change notifications to connected dashboards | storing the snapshot (it lives in the tenant DB), any writes to GitHub |
| `IngestDO` | one per board | `boardId` | Serializes collector runs for the board; holds ingest cursors (ETag/last-snapshot watermark) and the board's GitHub rate-budget state | deciding what to fetch (the registry + query shape do), writing GitHub |
| `ReconcileDO` | one per board | `boardId` | Serializes hosted reconcile runs for the board; holds the writer-of-record lease state and checks the fencing epoch before every mutation batch (§5) | holding tenancy authority, bypassing the lease, reconciling boards it does not hold the lease for |

Rules that make the shape defensible, mirroring E's: routing is a pure function of (session → binding, board → tenant DB); no global mutable state on the hot path; every object hibernates — no always-on process; per-board objects, never one global PM object (a single object for all boards would be a hot partition and a single blast radius, E's recorded anti-goal).

## 2. Live state and presence

The laptop edition has no live surface; this is the hosted edition's one additive capability, recorded as a deliberate divergence in §6. `BoardDO` holds only derived, rebuildable state: when the collector lands a new snapshot in the tenant DB, it pokes the board's `BoardDO`, which re-derives view state and pushes change notices to subscribed dashboard sessions. Presence is ephemeral and board-scoped (who is viewing this board), held only in the DO, never written to the tenant DB. If `BoardDO` state is lost, it is rebuilt from the tenant DB snapshot — the DB is the record, the DO is a cache with subscribers.

## 3. Data homes and ingest (Q2)

**Board data lives in the tenant's Turso DB (component E's tenant plane). Full stop.** The PM store is one schema in that DB: board registry rows (the #9060 `BoardDef` shape, stored as data exactly as on the laptop), the merged item pool, hierarchy/link records (#9060 §11 shape: relation, status, provenance), Metadata copies (#9060 §12), roadmap rows, release-milestone snapshots (#9060 §4), reconcile findings, and the writer-of-record lease (§5). The central tenancy DB holds only what E says it holds — the tenant registry pointer to this DB — and no PM content, per E §2.3.

**D1 is rejected for board data**, with the reason named: the laptop edition already standardized its stores on Turso (#6815), and the same-world rule requires both editions to read one store family with one schema. A D1 copy would be a second SQL plane whose only job is mirroring the tenant DB — a divergence factory, not a cache. (Workers-side ephemeral caches — `BoardDO` state, CDN for public assets — are derived and rebuildable, and are not data homes.)

**Public boards** are served from E's public plane: a board is public exactly when a publication record says so, and its media follows F's publication-reference rule. There is no "public flag" on board rows.

**Ingest path: hosted collector on Cron Triggers.** A scheduled Worker invocation runs the collector per board in the registry: the same GraphQL pool query the laptop collector runs (fields, views, `parent`, `blockedBy`/`blocking`, labels, milestones, `subIssuesSummary`), parameterized by board, fail-soft per board (one board failing never empties the pool — #9060 §1 rule, both editions), merged into one pool in the tenant DB where cross-board ancestry resolves. Per-board serialization and cursors live in `IngestDO`. **GitHub identity** is the #9060 §1 answer, unchanged: per-org App installation tokens through the fleet rotation machinery (the Projects-only PAT cannot read the satellite boards). Token acquisition is a tenancy-Worker function; token values are never stored in any DB (E §2.3) — they are minted per run and die with it. During transition the laptop collector keeps running; which collector's snapshot is authoritative for a board is a writer-of-record question (§5), not a freshness race.

## 4. V1 parity list (Q3)

Must-have for the hosted edition to be a workable equivalent (each maps to a #9060 surface or a live laptop surface):

- Board views over the full registry (all boards, per-board audit), `/api/projects.json` in the #9060 §6 `projects[]` shape.
- Item detail with the **genealogy header + sentence** (#9070's read model, ported — it is pure derivation over the pool, so the port is the renderer + API, not a redesign), `/api/item/<id>.json` with `ancestors[]` and `release`.
- Roadmap: per-project roadmap and the overall priorities view, `/api/roadmap.json` with `priority`/`afterRefs`/`releaseRef`.
- Releases: milestone snapshots and close-out accounting cards, `/api/releases.json` (#9060 §4 semantics unchanged).
- Needs-attention / coverage + audit outputs (the collector-checked invariants of #9060 §3/§7, surfaced as findings).
- Quota/usage views (the aggregator's quota surfaces; the data already lands in the same Turso estate, #6815).
- Reconcile findings + drift reports (read side; the write side is §5).

**Explicitly deferred / deliberate divergences** (named, never silent): Kaneo, Pad, and Grist are separate products beside the PM service, not surfaces of it — out of scope in both directions. The pm-dispatch intake app and the pm-console gate are separate laptop apps; the hosted edition consumes H's sign-in directly instead of porting the console gate. Push notifications and the AA-explorer extras are laptop-aggregator accretions, not PM-service surfaces; they port, if ever, as their own slices. Live presence (§2) is hosted-only and additive.

## 5. Reconcile as hosted single writer (Q4) — recommendation (proposed)

#9060 §10 stands in both editions: **`project-reconcile.sh` remains the boards' single writer** — meaning exactly one writer *of record* per board at a time, and this design names the mechanism that keeps that true across two editions:

- **Writer-of-record lease**, one row per board in the tenant DB: `{board_id, writer: laptop|hosted, epoch, heartbeat_at}`. The epoch is a fencing counter: it increments on every flip, and every reconcile run — either edition — reads the row before its first mutation and aborts if it is not the recorded writer at the current epoch.
- **Hosted side:** `ReconcileDO` runs the hosted reconcile port (same checks, same additive-labels rule, same `## PM lineage`/Metadata mirroring as the laptop script — the reconcile *logic* is shared by specification and conformance fixtures, §7, not by shared runtime). It re-checks the lease epoch before each mutation batch, so a flip mid-run stops the run instead of double-writing.
- **Laptop side:** `project-reconcile.sh` gains one guard at startup — read the lease row; if the writer of record is `hosted`, exit as a read-only verifier (drift reports still produced, no writes). This is a small, named laptop change (slice 6), not a rewrite.
- **The flip** is a procedure, not a flag toggle: (1) quiesce the current writer (no run in flight, heartbeat stale or explicitly released), (2) increment the epoch and set the new writer, (3) the new writer's first run is verify-only against the last snapshot, (4) writes resume. Rollback is the same procedure in reverse. Per-board flips: boards move independently, so the transition can prove itself on one satellite board before the main board moves.
- **Stated limitation, honestly:** GitHub has no fencing primitive, so the lease cannot physically stop a writer that ignores it. The enforcement is the startup check + epoch re-checks + the reconcile findings report naming any write that arrived from a non-record writer (detectable via Metadata provenance, #9060 §12). That is the same enforcement class #9060 already accepts for closure gates ("GitHub cannot hard-block; the monitor is the enforcement surface").

## 6. Model-compatibility statement vs #9060 (acceptance)

| #9060 element | Hosted treatment | Divergence |
| --- | --- | --- |
| Board registry (§1, `BoardDef`) | Same shape, stored as rows in the tenant DB; registry is data in both editions | None |
| Field schema (Project #4 canonical) | Same canonical schema; collector fail-soft parse for satellite boards, both editions | None |
| Hierarchy levels + edges (§2) | Same edges; stored as rows + link records; GitHub native marks remain the GitHub-side representation | None |
| Primary parent (§3) | Rule unchanged; native parent canonical on GitHub, `primary-parent/*` label fallback, collector invariants checked by both editions | None |
| Link records (§11) | Same relation/status/provenance shape, stored in the tenant DB | Store location only (rows vs laptop store) — named, not silent |
| Metadata block (§12) | Canonical record unchanged; hosted collector parses and stamps identically; DB copy lives in the tenant Turso DB | The fleet metadata DB (#9074) is the fleet tool's surface; the PM tenant DB holds the PM copy. Same shape, two query surfaces, divergence named here — the conformance suite (§7) pins the shape so the surfaces cannot drift apart |
| Milestones, two kinds (§4) | Semantics unchanged; close-out snapshots stored as rows, served by `/api/releases.json` | None |
| Closure gates + re-parenting (§7) | Same gates; enforced by reconcile in whichever edition holds the lease; monitor surface is the findings report in both | None |
| Genealogy read model (§6) | Same derivation, ported renderer (#9070 precedent) | None |
| Live presence | Exists only in the hosted edition (`BoardDO`, §2) | **Additive divergence**: no model element changes; the laptop edition has no equivalent surface and loses nothing |
| Writer-of-record lease (§5) | New operational state with no #9060 counterpart | **Operational, not model**: #9060's "single writer" rule is preserved and mechanized; no data-model element is added or changed |

No other divergences. Anything a future slice needs that is not in this table is a new divergence and must be named on #9079 before it ships.

## 7. Invariants

1. **Same world.** One data model (#9060) and one store family (tenant Turso); any shape change lands in both editions' collectors in the same program step, pinned by a shared conformance fixture suite: same GitHub snapshot in → same API JSON out, laptop vs hosted.
2. **One writer of record per board at a time** (§5); every write carries Metadata provenance identifying its writer and epoch.
3. **Fail-soft per board** in the collector; **fail closed** on identity and jurisdiction (E's rules, inherited, not restated weaker).
4. **No PM content in the tenancy DB; no tokens or keys in any DB** (E §2.3, inherited).
5. **Labels are only ever added, never removed**, by reconcile in either edition (#9060 §10).
6. **The tenant DB is the record; every DO state is derived and rebuildable** (§2).

## 8. Implementation slices filed from this design

Filed as granular issues under #9079:

1. PM tenant store: schema + migrations (registry, pool, link records, Metadata copies, roadmap, release snapshots, writer-lease rows) in the tenant Turso DB.
2. Hosted collector: registry-driven multi-board GraphQL ingest on Cron Triggers + `IngestDO` (fail-soft, cursors, per-org App token minting per run).
3. Read API parity: `/api/projects.json`, `/api/item` ancestors + release, `/api/roadmap.json`, `/api/releases.json` per #9060 §6 shapes.
4. Dashboard surfaces: board views, genealogy header/sentence, roadmap + releases, needs-attention, quota views (server-rendered on Workers).
5. `BoardDO` live state + presence (snapshot-change fan-out, ephemeral board presence, rebuild-from-DB).
6. `ReconcileDO` + writer-of-record: hosted reconcile port, lease/epoch fencing, the laptop `project-reconcile.sh` lease guard, flip/rollback runbook (first flip: one satellite board).
7. Identity wiring: H/#9077 session → `binding_id`, E role checks on every route, fail-closed conformance tests.
8. Dual-edition conformance suite: shared snapshot fixtures, laptop-vs-hosted API equivalence, Metadata-shape pinning vs #9074.

## 9. Acceptance mapping (issue #9079)

- *Design merged under doc/ as part of #9078's build-out: service shape, DO inventory, data homes, ingest path, V1 parity list* — §1 (shape + DO inventory), §3 (data homes + ingest), §4 (parity list).
- *Model-compatibility statement vs #9060's design — divergences named, none silent* — §6 (three named divergences: link/Metadata store location, additive live presence, operational writer lease; all other elements: none).
- *Granular implementation issues filed under this issue* — §8, filed as sub-issues of #9079.

Open for ratification, deliberately not decided here: the §1 one-Worker recommendation and the §5 lease mechanism (mill/Drew); the timing of the first writer flip (operational, after slice 6 lands); multi-tenant PM beyond the operator's own tenancy (the data model is already per-tenant via E; serving other tenants' boards is a later program step, not a V1 slice).
