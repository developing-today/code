# The merged priority list

This is the project's single ranked list of next work: 30 items in 7 tiers,
merged on 2026-11-23/24 (Census B) from Drew's seed list on the programme
tracker [#13905](https://github.com/developing-today/code/issues/13905), the
four programme investigations of 2026-11-23 (fleet #13272, Atlas #13241, ID
#13240, mcpx #13264), the ID-100 list, and the books' own roadmap chapters.
Statuses are as of that census unless a later read is named. Where the
record has no timing, the census says so — no date in this chapter is
invented.

**Ordering, stated once:** execution substrate first (nothing else lands
without a trustworthy queue, build capacity and transport), then the
physical substrate (the cluster every heavy ambition assumes), then the data
substrate (backups, ingest, storage placement), then search intelligence,
then the ID platform build-out, then the gateway, PM and fork surfaces, then
the record itself. Within a tier, dependency order wins; ties break on
leverage — how much other work the item unblocks. [How priorities
work](how-priorities-work.md) states the doctrine in full.

**Types:** a *capability set* is a capability set to reach; a *problem* is a
problem to solve; a *milestone* is a milestone; a *shift goal* is a goal
that marks a shift. **Primary book** names the book whose high-level roadmap
chapter carries the item in full; every item appears under exactly one
primary book, and cross-linked items are repeated in the other books'
chapters as one-line pointers.

## Tier 0 — Execution substrate (fleet)

### 1. Fleet queue, claim and placement correctness

- **Type:** problem · **Primary book:** [Fleet](https://developing-today.github.io/code/fleet/development/high-level-roadmap.html)
- The queue broker, claim and renewal fencing, packet repair and placement
  must be provably correct — every other programme's work is dispatched
  through them.
- **Status:** in flight, substantially landed. The repair verb shipped (PR
  #13218) with a production drain of 37 repairs on broker generation 6; the
  double-serve defect (#13091) is fixed; placement criterion 1 was proven
  cross-seat (#13146, closed).
- **Rationale:** the dependency root of the whole project — a queue that
  double-serves or loses packets silently corrupts every other item's
  evidence. Trackers: #13272, #13191, #4986, #12236.

### 2. Containerised build and compile systems — in the cluster and in Actions

- **Type:** capability set · **Primary book:** [Fleet](https://developing-today.github.io/code/fleet/development/high-level-roadmap.html)
- Builds run in containers under a central coordinator — atomic slots, a
  true load view, global budgets — plus live short-lived exec sessions
  inside Actions runners, so no build depends on one laptop.
- **Status:** designed and in flight. The compile coordinator (#13765) is
  designed, its first packet parked on the platform-flag route; actions-exec
  (#13774) was building on 2026-11-23. The in-cluster half is not started —
  it waits on item 6.
- **Rationale:** build capacity is the second bottleneck after the queue;
  the Actions half proceeds now, the cluster half sequences behind the
  cluster itself.

### 3. Transport and P2P auth — one honest transport for agents and ID alike

- **Type:** shift goal · **Primary book:** [Fleet](https://developing-today.github.io/code/fleet/development/high-level-roadmap.html)
- Host selection without Tailscale, Iroh for non-Muse, a generic pool
  primitive, and capability auth over P2P transport (p2panda-first, honestly
  compared) — replacing today's accreted per-surface transports.
- **Status:** designed; both programme issues (#13752, #13779) filed
  2026-11-23 and open. Laptop-agent transport fallback is partially landed.
- **Long-term direction (Drew, recorded 2026-11-23):** multi-transport
  reach — the P2P/ID/fleet stack reachable over many alternate transports
  (Veilid-style relays, Tor, email, MQTT, MCP, IRC, chat bots,
  Gopher/Gemini, physical media), each adapter declaring its level (read,
  bidirectional, write, sync-equivalent), with asymmetric send and receive
  paths first-class. Its programme is being filed against #13752; it rides
  this item's line and is not a separate item.
- **Rationale:** every dynamic-server, federation and remote-exec ambition
  below assumes this answer; settling it early prevents three programmes
  building three transports.

## Tier 1 — Physical substrate (lab)

### 4. Switches integrated and racked

- **Type:** milestone · **Primary book:** [Lab Book](https://developing-today.github.io/code/lab/)
- The dedicated switches from the repo's switching research are installed,
  racked and integrated, so the basement machines share a real fabric
  instead of ad-hoc links.
- **Status:** not started (planned). Switch make, model and count are
  missing from the hardware record until the switching research
  (`doc/inventory/routing-and-switching.md`, `doc/hardware/`) is folded in;
  no single GitHub issue owns this item — #13905 is its first tracker.
- **Rationale:** first physical dependency — racking and networking precede
  servers presenting (item 5) and the cluster boot (item 6).

### 5. All servers online, auto-connecting and presenting to the cluster on boot

- **Type:** capability set · **Primary book:** [Lab Book](https://developing-today.github.io/code/lab/)
- Every server powers on, joins the network and presents itself to the
  cluster declaratively — no hand-built machine that just exists forever.
- **Status:** not started. No operating system is installed on any basement
  machine (deliberate); every CPU boots and all RAM works, nothing
  load-tested; basement power capacity is a named blocker; the build target
  is the 3–5 best, most identical machines first.
- **Rationale:** the cluster (item 6) is defined over machines that present
  themselves. Trackers: #1086, #10697.

### 6. Talos cluster booted live, accepting data on Cozystack

- **Type:** milestone · **Primary book:** [Lab Book](https://developing-today.github.io/code/lab/)
- A Talos Linux cluster bootstrapped declaratively with TALM, running
  Cozystack with Postgres (CloudNativePG), RustFS, NATS and FluxCD, storage
  replicated with DRBD and LINSTOR — live and accepting data. Hard rule of
  record: no HashiCorp tooling.
- **Status:** designed, not started. Design and per-node disk configuration
  exist (`infra/cluster/`; the disk answer is proven as design and local
  flow, #1812 closed via PR #5052); nothing is running. The v1.9-era test
  cluster was torn down (confirmed by Drew 2026-10-26) — a recorded reset.
- **Rationale:** the physical keystone — in-cluster builds (item 2),
  self-hosted ID servers (item 17) and the local leg of the backup mesh
  (item 7) all presume it. Trackers: #1086, #1817, #10697.

## Tier 2 — Data substrate (backups, ingest, storage placement)

### 7. Backups and storage automated, minimal duplication, one coherent complete backup set

- **Type:** problem · **Primary book:** [Atlas](https://developing-today.github.io/code/atlas/design/high-level-roadmap.html)
- Replace per-system backup timers and overlapping copies with one automated
  mesh: every store backed up, deduplicated by content, restorable as one
  coherent set.
- **Status:** partially done, mesh not started. What exists is the laptop's
  daily `atlas-backup` timer — one system's backup, automated, into the
  shared R2 bucket `supabase-backups`. The cross-system mesh (#9093) is
  designed only. The estate it must cover: 50 Turso databases (PM census,
  2026-11-23). Drew's rule of 2026-10-31 stands over it: keep all data,
  clean only byte-identical duplicates, keep backups (#3125).
- **Rationale:** safety precedes scale — ingest automation (item 8) and
  storage migration (item 11) both mutate the corpus and should not run
  ahead of a coherent restore story.

### 8. Atlas ingestion fully automated

- **Type:** capability set · **Primary book:** [Atlas](https://developing-today.github.io/code/atlas/design/high-level-roadmap.html)
- Continuous ingestion — event hooks where the source supports them,
  scheduled sweeps otherwise, delta-only where possible — replacing manual,
  script-driven imports, with a recount procedure so corpus counts never
  freeze silently again.
- **Status:** designed, acceptance unmet — the single biggest ingest gap in
  the Atlas census. Today all ingestion is manual; canonical counts are
  frozen at the 2026-10-31 publish (18,325 sessions, 122,124 messages,
  107,639 chunks). The staging tier's aggregator, embedder and
  session-close hook are explicitly not built.
- **Rationale:** everything in Tier 3 is only as fresh as this pipeline; it
  outranks coverage expansion (item 9) because automating the pipe precedes
  widening what flows through it. Trackers: #9097, #7969, #6018, #6028,
  #5670.

### 9. All repos and all ID objects into Atlas ingestion — full source coverage

- **Type:** capability set · **Primary book:** [Atlas](https://developing-today.github.io/code/atlas/design/high-level-roadmap.html)
- Every source on Drew's list actually flowing: Turso queue and PM data; all
  GitHub issues, PRs, commits and files for all in-scope organisations; all
  Muse conversations and data; all OpenCode sessions (with the
  thinking-traces investigation answered); all ID objects; plus the
  sources-inventory pass that hunts for missed sources.
- **Status:** in flight, early. Ingested on record: AI conversation
  sessions, the ChatGPT export and the sandbox-agents corpus (both interned
  into the content-addressed store 2026-11-23). GitHub comments have a
  dedicated extractor in code; ID objects and the queue/PM corpora have no
  live connector yet.
- **Rationale:** coverage is the multiplier on items 8, 12 and 13, sequenced
  after automation so new sources land in a pipeline. Trackers: #9097,
  #8622, #8626, #8623, #6025, #6024, #6052.

### 10. Atlas provenance complete — CAS, occurrences, catalog and shards

- **Type:** milestone · **Primary book:** [Atlas](https://developing-today.github.io/code/atlas/design/high-level-roadmap.html)
- Finish the provenance-first rebuild (#3125 phases 3–4): the historical
  chunk-occurrence backfill (about 127K occurrences dropped at first ingest;
  6,344 sessions owning zero chunks), the catalog database, the
  collection-by-month shard layout, and resolve/fetch-span as MCP tools.
- **Status:** in flight. Phase 1 and phase 2 slices landed — 12,092 files
  interned into the content-addressed store with verify proof on
  2026-11-23. Phases 3–4 are open.
- **Rationale:** provenance is the trust layer under deduplication, deletion
  propagation and the ID-graph publication (item 12).

### 11. Storage placement settled — Turso-to-D1 programme and Atlas D1 hot cache

- **Type:** shift goal · **Primary book:** [Atlas](https://developing-today.github.io/code/atlas/design/high-level-roadmap.html)
- Decide and execute where each store lives (Turso, Cloudflare D1, R2, or
  local) on measured economics, and land the per-tenant D1 hot cache in
  front of Turso for the multi-tenant serving shape.
- **Status:** in flight. The audit (#13844) merged via PR #13845 against the
  live-verified 50-database estate. The hot-cache schema and store landed
  (#13100, closed) with the master flag off — production search does not
  touch D1 today; freshness (#13101) and admission (#13103) are open, the
  read path (#13105) is gated on #13101, and invalidation (#13106) is held
  under a platform flag.
- **Rationale:** placement is a near one-way-door decision that items 12,
  17 and 25 build on, and the audit existing makes this the cheapest moment
  to settle it.

## Tier 3 — Search intelligence (Atlas and ID)

### 12. Atlas↔ID integration — search runs across the core system

- **Type:** shift goal · **Primary book:** [Atlas](https://developing-today.github.io/code/atlas/design/high-level-roadmap.html)
- End the split where ID has no live query path into Atlas: Atlas objects
  and provenance become ID-graph objects, ID content is vectorised into
  Atlas, and one retrieval core serves CLI, REST, MCP and ID.
- **Status:** designed, implementation not started. The shared ground in
  code today is only the content-hash v1 test vectors. Drew's "Atlas baked
  into ID" framing is recorded as a direction to explore, not a decision of
  record; the recorded decisions are #6062 (publication format) and #9109
  Part C.
- **Rationale:** the highest-leverage shift in the census — it converts two
  half-systems into one core, and items 13 and 15 both presume it.
  Trackers: #6062, #11182–#11186, #9109, #5755.

### 13. Rerank and recommender systems in Atlas

- **Type:** capability set · **Primary book:** [Atlas](https://developing-today.github.io/code/atlas/design/high-level-roadmap.html)
- A learned reranking stage in the serving path, gated on the golden set,
  and a recommender built from real retrieval behaviour — session
  co-occurrence neighbours, feedback signals, recommendation objects with
  per-class retention.
- **Status:** designed; serving does not exist. Production fusion is
  reciprocal-rank fusion over full-text and vector branches only; no learned
  reranker exists in the serving path; feedback is captured
  (`atlas-feedback/v1`) with no ranking consumer; recommendation generation
  does not exist — only the snapshot-paging state machine is in flight
  (#10158, PR #12873 open). The gate: a 311-query golden set (v3) plus 54
  held out (#8627).
- **Rationale:** deliberately after items 8–10 — a reranker evaluated on a
  frozen, provenance-poor corpus would bake in today's gaps.

### 14. Atlas as a multi-client search service — the product decision and its build

- **Type:** milestone · **Primary book:** [Atlas](https://developing-today.github.io/code/atlas/design/high-level-roadmap.html)
- Answer the open product question — Atlas as a multi-client service
  (OpenChamber and fork sessions, ID, agents as first-class consumers) or
  Drew's personal CLI — and build the service shape the answer implies,
  with per-agent identity instead of today's shared hosted identity.
- **Status:** in flight, decision open. A service shape exists de facto
  (the fleet search service, the Worker, and the mcpx `atlas` server are
  all live), but the thread-2 commitment on #12900 is unanswered, and all
  hosted MCP callers still share one identity (`hosted-per-user`, personal
  ceiling).
- **Rationale:** the decision gates real investment in identity, caching and
  ceilings; it sits after item 12 because ID-as-consumer is the strongest
  evidence for the multi-client answer. Trackers: #12900, #5350, #8625,
  #6805.

### 15. ID apps on one graph — URIs, transclusion, typed links, graph display

- **Type:** capability set · **Primary book:** [ID](https://developing-today.github.io/code/id/design/high-level-roadmap.html)
- The ID graph programme: apps (notes, boards, worlds) sharing one graph
  with stable URIs, transclusion, typed links, sneakernet bundles and a
  graph display.
- **Status:** designed and partially in flight. Boards and feeds landed in
  slices (for example #10237's code via PR #12932) but the chain sits open
  on stale labels with its close held under a platform flag; the unifying
  epic #13880 was filed 2026-11-24 during the census window.
- **Rationale:** first ID-platform item because it is the user-visible
  surface the store, hosting and federation work exists to serve — its
  requirements should steer them. Trackers: #13880, #2056, #9129, #5525,
  #3790.

## Tier 4 — ID platform build-out

### 16. ID store abstraction — one interface over redb, Turso, D1 and IndexedDB

- **Type:** capability set · **Primary book:** [ID](https://developing-today.github.io/code/id/design/high-level-roadmap.html)
- One store interface with pluggable backends and a shared conformance
  suite, so an ID object behaves identically on a local redb, a Turso or D1
  hosted store, or a browser IndexedDB.
- **Status:** in flight. Conformance fixtures landed (#10191, closed, PR
  #13194); lifecycle hooks' first slice landed (PR #13156). The chain's
  only remaining foundational step is #9082's adopt/defer/reject act on the
  IndexedDB profile.
- **Rationale:** every dynamic-hosting and offline item below is written
  against this interface — the ID tier's dependency root. Trackers: #9082,
  #9110, #10192, #5805, #5814, #5681.

### 17. Tunnels to spin up ID servers dynamically — hosted worlds and hosted editions

- **Type:** capability set · **Primary book:** [ID](https://developing-today.github.io/code/id/design/high-level-roadmap.html)
- Spin up an ID server on demand — through a tunnel, against a local store,
  a Turso store, or Cloudflare R2/D1 — with the same behaviour hosted or
  local, plus the Cloudflare hosted edition (per-user Durable Objects,
  SSO-to-ID binding, the signed-object bridge, export and import). The
  principle of record: local-first always; the hosted instance is the
  public backstop, never a requirement.
- **Status:** in flight, chain blocked at the head. #10693 to #10697 were
  all verified open with a blocked status at census, their queue tasks
  recorded as blocked until 2026-11-26; #10693 was enqueued (task 21463)
  with its ownership ruled. Both export/import code halves landed
  2026-11-23 (PRs #13201, #13212).
- **Rationale:** the seed that turns ID from a binary into a platform; it
  depends on items 3, 6 and 16. Trackers: #10693–#10698, #9078, #2852,
  #3818, #3810, #6276.

### 18. Discovery, registry and federation at scale

- **Type:** capability set · **Primary book:** [ID](https://developing-today.github.io/code/id/design/high-level-roadmap.html)
- Worlds findable and joinable across servers: publish-to-pkarr
  announcements, a discovery resolver (invite link, pkarr, registries, in
  that order), a public registry endpoint, and federation with boundary
  gates, jurisdiction routing and delete propagation.
- **Status:** designed, head slices blocked. The specific upstream blocker
  for #10706 and #10708 was unverified in the ID census (labels only) —
  recorded as a gap, not guessed; the census's recommendation is that this
  item's first slice be a blocker-identification read, not implementation.
- **Rationale:** federation before scale (item 19) — little value hardening
  a single-server world while joining across servers is still blocked.
  Trackers: #6277, #10706, #10708, #10712, #6289, #8281, #7787, #2091.

### 19. ID scale and performance hardening — measured limits at 10× and 100×

- **Type:** problem · **Primary book:** [ID](https://developing-today.github.io/code/id/design/high-level-roadmap.html)
- A generator and harness producing worlds at 10× and 100× current sizes,
  measuring join time, replay, frame cost and memory, ending in a measured
  limits table — with Drew's doctrine as the bar: a multiplier is proven by
  benchmark, never asserted.
- **Status:** in flight. Two of five measurement slices are closed with
  real numbers: the generator built 10× (11,001 worlds, 200,000 events in
  35.7 seconds, #13083) and frame cost measured flat to 1,000,000 events
  (#13085). Join-time benches (#13084) are held under a platform flag; the
  memory profile (#13086) is in progress. A codified wall was found:
  `WorldLimits` caps participants at 256 against tier targets of 320 at 10×
  and 3,200 at 100×.
- **Rationale:** measurement before optimisation, and before currency and
  federation load arrives. Trackers: #6281, #6304, #13087, #13088, #5685.

### 20. Production currencies v1, then the scaling path

- **Type:** milestone · **Primary book:** [ID](https://developing-today.github.io/code/id/design/high-level-roadmap.html)
- Ship production currencies v1 (graph-to-ledger reconciliation, the grant
  service, RateAuthorisation), then the designed scaling path: Redis, then
  multi-Turso, then TigerBeetle.
- **Status:** in flight, the v1 remainder parked (task 21343). Scaling is
  design-only, sequenced behind v1 by standing triage verdicts — a recorded
  reprioritisation: v1 first.
- **Rationale:** correctness for money outranks breadth, and v1 is close
  and parked, so finishing it is cheap. Trackers: #10070, #2320, #2290,
  #2294, #2292, #5676.

### 21. IndexedDB local caching in web apps

- **Type:** capability set · **Primary book:** [ID](https://developing-today.github.io/code/id/design/high-level-roadmap.html)
- Browser ID clients cache and work against a local IndexedDB store behind
  the same store interface (item 16), with conformance proven in a real
  browser engine.
- **Status:** prototype done; adoption decision open. The prototype
  (#10213, closed, PR #13219) verified, in a headless Chromium conformance
  suite: per-batch whole-database atomic scope, quota of about 10.7 GB, and
  `persisted: false`. What remains is #9082's decision plus production
  wiring.
- **Rationale:** the prototype de-risked the seed; it is the prerequisite
  half of the offline seed (item 23). Trackers: #9082, #10191, #6377.

### 22. Dynamic Android platform — many apps, push, touch, GPS, driver scope

- **Type:** shift goal · **Primary book:** [ID](https://developing-today.github.io/code/id/design/high-level-roadmap.html)
- One dynamic Android platform hosting many ID apps, with push
  notifications, touchscreen, GPS and driver scope exposed to the hosted
  apps — and the same shape for the Bevy native shell and the web app: one
  shell contract, many surfaces.
- **Status:** in flight, early. Android v1 slice 1 landed (PR #13445 via
  #13403); the Bevy design was adopted 2026-11-23 (#5781); rich
  presentation surfaces step 2a landed (PR #13214), with 2b (audio) and 2c
  (widget) remaining. The dynamic multi-app platform with the full driver
  scope is designed at outline level, not built.
- **Rationale:** the shell programme is where ID becomes tangible on the
  devices Drew carries; it follows the store and hosting items because
  every shell is a client of them. Trackers: #1831, #7938, #6297, #10930,
  #10931, #12817, #1071, #12657.

### 23. Service-worker offline and local-first web apps

- **Type:** capability set · **Primary book:** [ID](https://developing-today.github.io/code/id/design/high-level-roadmap.html)
- Web apps that keep working offline: a service-worker shell plus the
  IndexedDB cache (item 21), syncing when the network returns — the web
  expression of ID's local-first principle.
- **Status:** not started as a named item. The IndexedDB half is
  prototyped; no service-worker work was found in the ID census. No single
  owning issue exists — a census finding: one should be filed under the ID
  programme (#13240) rather than left implicit.
- **Rationale:** last in the ID tier by dependency (it composes items 16,
  17 and 21), not by value.

### 24. ESP32 hardware platform line — ID on microcontrollers

- **Type:** capability set · **Primary book:** [Lab Book](https://developing-today.github.io/code/lab/)
- ESP32-family boards as an ID hardware shell — sensors and LoRa connected
  to the ID project — a lab chapter today, its own line long-term.
- **Status:** designed; a side line. The hardware census records it as a
  side project of a side project for now — noted, not scheduled. Per-device
  verification lives in the sibling `developing-today/hardware-doc`
  repository. Tracker: #5783.
- **Rationale:** genuinely long-term — it inherits the shell contract (item
  22) and the lab's physical readiness (items 4–6), and blocks nothing.

## Tier 5 — Gateway, PM and forks surfaces

### 25. mcpx completion — supervisor, session resume, config tail, embedding surface

- **Type:** problem · **Primary book:** [mcpx](https://developing-today.github.io/code/program/high-level-roadmap.html)
- Finish the MCP gateway: a supervisor owning every stdio child's pipes for
  its whole life, cold-restart session resume completed, the settings
  effect-probe exemption tail burned to zero, and the embedding surfaces
  (Go API, C ABI, WASM, mcpx as its own upstream) decided.
- **Status:** in flight, close on several fronts. The daemon, about 50
  commands and 252 settings are live; the supervisor (#1384) and supervised
  stdio (#9963) have merged PRs with acceptance carried by #13264; session
  resume (#1414) has phases 1–2 landed; OAuth for remote servers (#1950)
  landed 2026-11-23. The exemption tail is exactly 8 settings: 5 unheld,
  3 held under platform flags (#8472, #8473).
- **Rationale:** mcpx is the tool gateway every agent uses daily, so its
  reliability items outrank its expansion items; it sits in Tier 5 because
  Tiers 0–4 are its dependencies, not its dependents. Trackers: #13264,
  #1900, #1877, #1940.

### 26. PM stack completion — backlog as derived state, hosted PM, push end-to-end

- **Type:** capability set · **Primary book:** PM book (tracker [#13900](https://github.com/developing-today/code/issues/13900); published subpath not yet final)
- The PM stack finishes its own roadmap: backlog state derived from
  database plus GitHub with markdown as a view, the hosted-editions PM
  service, decision sets v2, and the push path (aggregator, Web Push,
  phone, answer) deployed end-to-end.
- **Status:** in flight. Aggregator and console are live (29 routes, 21
  console views at census); the notify relay is built but not deployed, its
  answer routes unrouted. Decision sets v2 (#9335) landed acceptance and
  was live-verified 2026-11-13, awaiting close at census. Backlog derivation
  (#5007) carries a verdict of record: correctly blocked on phase ordering
  — do not start yet.
- **Rationale:** PM coordinates Tiers 0–4 rather than blocking them, and its
  blocked-on-ordering verdict is respected in this ranking. Trackers:
  #13504, #9060, #9078.

### 27. OpenCode and OpenChamber forks programme — one shared routing implementation

- **Type:** problem · **Primary book:** PM book (tracker [#13900](https://github.com/developing-today/code/issues/13900); published subpath not yet final)
- Converge the forks' duplicated machinery — AUDIT-7 Lane 7's top
  convergence candidate, with quota and routing fixes recurring in both
  forks (249 and 201 titles) — and finish the OpenCode 2 configuration and
  plugin migration.
- **Status:** in flight. Across the census window the forks' work was 66
  percent fix-titled — a reactive stream, which is itself the finding this
  item answers. OpenCode 2 work (#730) was active 2026-11-23.
- **Rationale:** real leverage (one implementation instead of two fix
  streams), but no other census item is blocked on it. Trackers: #730,
  #3286, #1858, #2841, #13874, #1334. Detail: [The OpenCode and OpenChamber
  forks](../systems/forks.md).

### 28. Fleet secrets, CA and quota plane — the trust and capacity layer

- **Type:** capability set · **Primary book:** [Fleet](https://developing-today.github.io/code/fleet/development/high-level-roadmap.html)
- The fleet CA and credentials design completed, the secrets broker hardened
  (no silent eviction on partial refresh), and the quota plane's staged
  acceptance (observation history, single-pool trial, recurring activation)
  finished.
- **Status:** in flight. Observation history is accumulating: the first
  qualifying record is dated 2026-11-23 02:30 UTC, with 63 records in 61 of
  62 quarter-hour buckets at census. The census carries the one hard
  near-term date in the whole list: the seven-day interval can first elapse
  **2026-11-30 at 02:30:06.420 UTC**. The weekly-review timer was absent on
  the laptop at inspection; its first scheduled invocation is #13591.
- **Rationale:** placed late deliberately — the quota doctrine (burn to
  100 percent at reset; a failed read never idles a pool) means today's
  manual stewardship works; this item makes it systematic. Trackers: #9155,
  #5504, #12694, #13591, #13081.

## Tier 6 — The record itself

### 29. Design-register burn-down — AUDIT-7 lanes 5 and 6

- **Type:** problem · **Primary book:** PM book (tracker [#13900](https://github.com/developing-today/code/issues/13900); published subpath not yet final)
- Close the design gap the audit measured: 312 issues demanding design, of
  which 111 have a design missing or very simple and 121 more deficient or
  partial — designed as families where the audit flags convergence (the
  Supabase-to-Turso family, the Atlas/ID hosted cluster, and #9085 before
  #9081).
- **Status:** in flight. The audit completed 2026-11-24; the first 28
  prioritised design packets plus 2 continuation packets are live in the
  queue. The audit's only "sufficient" design set in the repo is the
  pm-orchestration family (#9478, #9479, #9443, #9444, #9481–#9484).
- **Rationale:** designs gate implementation everywhere, but the audit
  already sequenced this work into packets; the merged list keeps it
  visible as one item rather than re-ranking its interior. Trackers:
  #13871, #13873, #8986.

### 30. The books and Spaces themselves — Code Book programme completion

- **Type:** milestone · **Primary book:** PM book (tracker [#13900](https://github.com/developing-today/code/issues/13900)); programme tracker [#13905](https://github.com/developing-today/code/issues/13905)
- This programme's own deliverables: the Code Book, the Lab Book, per-book
  high-level roadmap sections in the fleet, Atlas, ID and mcpx books (plus
  PM via its tracker), the Code Space and the Lab Space, and the AliExpress
  hardware-inventory issue.
- **Status:** in flight. The fleet, Atlas, ID and mcpx books all exist and
  deploy; the Code Book and Lab Book are this programme's remaining book
  builds; the AliExpress inventory is pending Drew's access step (no login
  attempted by the programme, per the tracker).
- **Rationale:** last by construction — the record describes the work; it
  does not precede it. Its per-book sections are nevertheless the delivery
  vehicle for this whole list.

## Census gaps carried forward

Census B closed with gaps it refused to paper over; this chapter carries
them forward unchanged:

1. Item 23 (service-worker offline) has no owning issue. Recommendation of
   record: file one under the ID programme (#13240).
2. Item 4 (switches) has no owning issue and an incomplete inventory; the
   Lab census (Census C) owns closing it.
3. The discovery blocker for #10706 and #10708 is unverified (item 18).
4. Book roadmap paths differ per book by design; the merged list does not
   force one path on all books.
5. Platform flag holds pace several items (#13106, #13084, #13036, the
   #10237-chain close, 3 mcpx exemptions). They are recorded per item as
   gates with reasons.
