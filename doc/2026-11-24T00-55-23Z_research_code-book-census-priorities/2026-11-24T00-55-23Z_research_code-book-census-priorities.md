# Census B — Merged priority list for the Code Book programme

- **Programme tracker:** [#13905 — Code Book + Lab Book programme: whole-project book, per-book roadmap sections, lab book, Code Space](https://github.com/developing-today/code/issues/13905) (open; created 2026-11-24T00:45:30Z = 2026-11-23 18:45 CST; verified live via the laptop fleet client in this pass).
- **Census date:** 2026-11-23/24. Research only — no repo edits were made for this census.
- **Method / sources.** (a) Drew's seed list as filed on #13905. (b) The four programme investigations of 2026-11-23 and their trackers: ID [#13240](https://github.com/developing-today/code/issues/13240), Atlas [#13241](https://github.com/developing-today/code/issues/13241), fleet [#13272](https://github.com/developing-today/code/issues/13272), mcpx [#13264](https://github.com/developing-today/code/issues/13264) — all verified open, all created 2026-11-23. (c) The newer programme issues named in the brief, all verified live in this pass: P2P capability auth + transport [#13779](https://github.com/developing-today/code/issues/13779), fleet transport & pooling [#13752](https://github.com/developing-today/code/issues/13752), Turso→D1 migration audit + programme [#13844](https://github.com/developing-today/code/issues/13844), compile coordinator [#13765](https://github.com/developing-today/code/issues/13765), actions-exec [#13774](https://github.com/developing-today/code/issues/13774), ID graph programme [#13880](https://github.com/developing-today/code/issues/13880). (d) The ID-100 list (`~/workspace/id-investigation/id-100-list-2026-11-23.md`: 46 newly enqueued + 29 already-tasked + sweep to 82 of 100). (e) The books' roadmap chapters as they exist on the laptop checkout `~/git/code` (verified in this pass): fleet — `pkgs/fleet/book/development/program-roadmap.md` + `pkgs/fleet/book/operations/program-issue-map.md` (190 open fleet issues in its census); atlas — `design/roadmap-and-issue-map.md` per `pkgs/atlas-search/book/SUMMARY.md`; id — `design/roadmap-and-issue-map.md` + `research/current-issue-map.md` per `pkgs/id/book/SUMMARY.md`; mcpx — the book is `pkgs/mcpx/docs/` (84 chapters) and has **no** programme-roadmap chapter yet — its roadmap lives in its trackers, headlined by the super-tracker [#1900](https://github.com/developing-today/code/issues/1900). (f) The AUDIT-7 umbrella [#13871 — Audit programme: seven-lane review of PRs, issues and designs (2026-11-23)](https://github.com/developing-today/code/issues/13871) (open; created 2026-11-23T23:57:31Z = 17:57 CST, matching the brief's ~17:56 filing). Its design lanes are cited, not re-derived: Lane 5/6 design review + design gaps = [#13873](https://github.com/developing-today/code/issues/13873) (312 issues demanding design; 111 designs missing/very simple; 121 more deficient/partial; 30 queue packets created; report PR [#13901](https://github.com/developing-today/code/pull/13901) merged); Lane 7 stray work = [#13874](https://github.com/developing-today/code/issues/13874) (18,594 items censused; report PR [#13904](https://github.com/developing-today/code/pull/13904) merged); Lane 1 PR sweep = [#13872](https://github.com/developing-today/code/issues/13872) (199 open PRs examined, 101 merged in the sweep).
- **Types.** `capability-set` = a capability set to reach · `problem` = a problem to solve · `milestone` = a milestone · `shift-goal` = a goal that marks a shift.
- **Status vocabulary.** done / in flight / designed / not started — each with its evidence. "Designed" means a design of record exists and implementation has not landed; "in flight" means code or queue work is actively landing.
- **Honesty notes.** Statuses are as of the 2026-11-23 censuses unless a later read is named. Where a seed has no single owning issue, that is stated plainly rather than papered over. Timing is in §3; where the record has none, §3 says so — no dates are invented anywhere in this file.

---

## 1. THE MERGED LIST — ordered across the whole project

Ordering logic, stated once: **execution substrate first** (nothing else lands without a trustworthy queue, build capacity, and transport), **then the physical substrate** (the cluster every heavy ambition assumes), **then the data substrate** (backups, ingest, storage placement — search and ID both eat from these), **then search intelligence** (Atlas↔ID, rerank/recommender), **then the ID platform build-out** (stores, dynamic servers, federation, scale, currency, shells), **then the gateway/PM/fork surfaces**, **then the record itself** (design register, books). Within a tier, dependency order wins; ties break on leverage (how much other work the item unblocks).

### Tier 0 — Execution substrate (fleet)

#### 1. Fleet queue, claim and placement correctness
- **Type:** problem
- **Description:** The queue broker, claim/renewal fencing, packet repair and placement must be provably correct — every other programme's work is dispatched through them.
- **Area:** fleet (primary). Cross-links: pm (PM orchestration consumes the same queue), all books (every lane's work rides it).
- **Most related:** [#13272 fleet programme tracker](https://github.com/developing-today/code/issues/13272) · [#13191 repair verb for quarantined packets](https://github.com/developing-today/code/issues/13191) · [#13091 double-serve defect](https://github.com/developing-today/code/issues/13091) (fix PR #13143 merged) · [#13146 placement improvement](https://github.com/developing-today/code/issues/13146) (criterion 1 proven cross-seat 2026-11-23, closed) · [#4986 placement framework v1](https://github.com/developing-today/code/issues/4986) · [#12236 every lane on the intake ladder](https://github.com/developing-today/code/issues/12236) · fleet book roadmap order 1–6 (`program-roadmap.md`, read in this pass).
- **Status today:** **in flight, substantially landed.** Repair verb shipped (PR #13218), production drain of 37 repairs on broker generation 6 recorded on #13191; double-serve fixed; the fleet book's roadmap puts "reconcile the production repair cohort" first, ahead of any queue-policy change.
- **Ordering rationale:** Dependency root of the whole project: a queue that double-serves or loses packets silently corrupts every other item's evidence. Highest leverage in the census.

#### 2. Containerised build/compile systems — in the cluster and in Actions (SEED)
- **Type:** capability-set
- **Description:** Builds and compiles run in containers with a central coordinator — atomic slots, a true load view, global budgets — plus live short-lived exec sessions inside Actions runners, so no build depends on one laptop.
- **Area:** fleet (primary). Cross-links: lab (the in-cluster half runs on the Talos cluster, item 6), pm (dispatch views).
- **Most related:** [#13765 compile coordinator](https://github.com/developing-today/code/issues/13765) (already tasked — task 21517 parked, flag-routed to a laptop OpenCode session per the ID-100 record) · [#13774 actions-exec](https://github.com/developing-today/code/issues/13774) (active work 2026-11-23, slices landing through 22:10Z) · [#13844 Turso→D1 audit's exec-door family #13404/#13657](https://github.com/developing-today/code/issues/13844) · fleet book roadmap stream F (build and test: mirror parity, image/cache identity).
- **Status today:** **designed + in flight.** Coordinator designed, first packet parked on the flag route; actions-exec building. The in-cluster half is **not started** — it waits on item 6.
- **Ordering rationale:** Build capacity is the second bottleneck after the queue; the Actions half can proceed now, the cluster half sequences behind the cluster itself.

#### 3. Transport and P2P auth — one honest transport for agents and ID alike
- **Type:** shift-goal
- **Description:** Host selection without Tailscale, iroh for non-Muse agents, a generic pool primitive, and capability auth over P2P transport (p2panda-first, honestly compared) — replacing today's accreted per-surface transports.
- **Area:** fleet (primary). Cross-links: id (ID's sync/relay rides the same decision), mcpx (daemon transports: ssh/tsnet today).
- **Most related:** [#13752 fleet transport & pooling](https://github.com/developing-today/code/issues/13752) · [#13779 capability auth + P2P transport investigation](https://github.com/developing-today/code/issues/13779) · [#10796 laptop-agent domain: automatic transport fallback](https://github.com/developing-today/code/issues/10796) · mcpx transports incl. [#1384 supervisor design](https://github.com/developing-today/code/issues/1384).
- **Status today:** **designed / investigation open.** Both programme issues filed 2026-11-23 and open; laptop-agent fallback partially landed (SSH leg PR'd 2026-11-22).
- **Ordering rationale:** Every dynamic-server, federation and remote-exec ambition below assumes this answer; settling it early prevents three programmes building three transports.

### Tier 1 — Physical substrate (lab)

#### 4. Switches integrated and racked (SEED)
- **Type:** milestone
- **Description:** The dedicated switches from the repo's switching research are installed, racked and integrated, so the basement machines share a real fabric instead of ad-hoc links.
- **Area:** lab (primary, sole). Cross-link: fleet (cluster networking assumptions).
- **Most related:** the code repo's dedicated switches research — `doc/inventory/routing-and-switching.md` and `doc/hardware/` (named in the hardware record; **no single GitHub issue owns this item** — the Lab programme on [#13905](https://github.com/developing-today/code/issues/13905) is its first tracker). Hardware state of record: ~12 Threadripper machines, ASUS Pro WS WRX80E-SAGE core boards, high-speed NICs whose exact spec is still TBD in inventory.
- **Status today:** **not started (planned).** Switch make/model/count is explicitly MISSING from the hardware record until the switching research is folded in; no OS is installed on any basement machine yet.
- **Ordering rationale:** First physical dependency: racking/networking precedes servers presenting (item 5) and the cluster boot (item 6). Cheap, unblocking, and entirely Drew-side/logistics-paced.

#### 5. All servers online, auto-connecting and presenting to the cluster on boot (SEED)
- **Type:** capability-set
- **Description:** Every server powers on, joins the network and presents itself to the cluster declaratively — no hand-built machine that "just exists forever."
- **Area:** lab (primary). Cross-links: fleet (enrollment is the same shape as fleet agent enrollment), id (hosted-worlds deployment pack targets these machines).
- **Most related:** [#1086 TALM + Talos bootstrap epic](https://github.com/developing-today/code/issues/1086) (named as parent of the cluster config in `infra/cluster/README.md`) · ID hosted-worlds P5 basement-cluster deployment pack [#10697](https://github.com/developing-today/code/issues/10697) · fleet agent enrollment ([#12236](https://github.com/developing-today/code/issues/12236) family).
- **Status today:** **not started.** Record: no OS installed on the basement fleet (deliberate); every CPU boots, all RAM works, nothing load-tested; basement power capacity is a named blocker; build target is 3–5 best/most-identical machines first.
- **Ordering rationale:** The cluster (item 6) is defined over machines that present themselves; hand-installing the first nodes would create exactly the snowflake state the programme forbids.

#### 6. Talos cluster booted live, accepting data on Cozystack (SEED)
- **Type:** milestone
- **Description:** A Talos Linux cluster bootstrapped declaratively with TALM, running Cozystack with Postgres (CNPG), RustFS, NATS, FluxCD, DRBD+LINSTOR replication — live and accepting data. Hard rule of record: no HashiCorp tooling.
- **Area:** lab (primary). Cross-links: fleet (in-cluster builds, item 2), id (hosted worlds deployment target), atlas (storage tiers).
- **Most related:** [#1086 TALM + Talos bootstrap epic](https://github.com/developing-today/code/issues/1086) · `doc/planning/roadmaps/basement-cluster.md` (the current programme doc; the v1.9-era test cluster in `doc/talos-addresses.md` is torn down, confirmed by Drew 2026-10-26) · [#10697 ID deployment pack](https://github.com/developing-today/code/issues/10697).
- **Status today:** **designed / not started.** Design and per-node disk config exist (`infra/cluster/`); nothing is running. Storage design sketch is Drew's, to be validated: 15–16× 16 TB HDDs (8 still boxed), ≥3 TB ECC RAM, RAID-10/RAID-100 NVMe tiers, 3-machine DRBD replication.
- **Ordering rationale:** The physical keystone: in-cluster builds (item 2), self-hosted ID servers (item 17), and the local leg of the backup mesh (item 7) all presume it. Ranked below its own prerequisites (items 4–5) by dependency, above everything that consumes it by leverage.

### Tier 2 — Data substrate (backups, ingest, storage placement)

#### 7. Backups + storage automated, minimal duplication, one coherent complete backup set (SEED)
- **Type:** problem
- **Description:** Replace per-system backup timers and overlapping copies with one automated mesh: every store backed up, deduplicated by content, restorable as one coherent set.
- **Area:** atlas (primary — the largest store and the only working backup today). Cross-links: fleet (queue/PM Turso stores), id (ID stores), lab (local leg on cluster storage), pm (the estate census: 50 Turso DBs, 22 Workers).
- **Most related:** [#9093 backup mesh — all-Turso 14d restic-zstd to R2, R2/laptop/Drive mesh, D1+laptop DBs into Turso](https://github.com/developing-today/code/issues/9093) (open) · [#9094 OpenCode DB backup economics](https://github.com/developing-today/code/issues/9094) · [#3125 provenance programme's backup + plain-extracts rule](https://github.com/developing-today/code/issues/3125) (Drew, 2026-10-31: keep ALL data, only byte-identical dupes cleaned, backups kept) · [#9098 embed-storage economics](https://github.com/developing-today/code/issues/9098) · [#9133 storage benchmark harness](https://github.com/developing-today/code/issues/9133).
- **Status today:** **partially done, mesh not started.** What exists: the laptop's daily `atlas-backup` timer (DB snapshot + extracts archive with verify proof, into the shared R2 bucket `supabase-backups` under `atlas/<DATE>/`) — one system's backup, automated. The cross-system mesh (#9093) is designed-only. Estate scale it must cover: 50 Turso DBs (live-verified 2026-11-23 in the PM census).
- **Ordering rationale:** Safety precedes scale: ingest automation (item 8) and storage migration (item 11) both mutate the corpus; they should not run ahead of a coherent restore story.

#### 8. Atlas ingestion fully automated (SEED)
- **Type:** capability-set
- **Description:** Continuous ingestion — event hooks where the source supports them, scheduled sweeps otherwise, delta-only where possible — replacing today's manual, script-driven imports, with a recount procedure so corpus counts never freeze silently again.
- **Area:** atlas (primary, sole).
- **Most related:** [#9097 continuous ingestion programme](https://github.com/developing-today/code/issues/9097) (open; workstream E of the v1 build-out under [#8983](https://github.com/developing-today/code/issues/8983)) · [#7969 recurring ingestion sweep runner + guardrail monitor](https://github.com/developing-today/code/issues/7969) (open, not built) · [#6018 long-term ingestion programme](https://github.com/developing-today/code/issues/6018) · [#6028 delta-only ingestion + offset embeds](https://github.com/developing-today/code/issues/6028) · [#5670 migrate ingest scripts into code + fleet binary](https://github.com/developing-today/code/issues/5670) · staging tier in code (`atlas_stage` v1) whose aggregator/embedder and session-close hook are explicitly **not built**.
- **Status today:** **designed, acceptance unmet — the single biggest ingest gap** (Atlas census, 2026-11-23). Today: all corpus ingestion is manual/script-driven; the only Atlas schedule on the laptop is the daily backup timer; canonical counts are frozen at the 2026-10-31 publish (18,325 sessions / 122,124 messages / 107,639 chunks).
- **Ordering rationale:** Everything in Tier 3 is only as fresh as this pipeline. It outranks coverage expansion (item 9) because automating the pipe precedes widening what flows through it.

#### 9. All repos / all ID objects into Atlas ingestion — full source coverage (SEED)
- **Type:** capability-set
- **Description:** Every source on Drew's list actually flowing: Turso queue + PM data; all GitHub issues/PRs/commits/files for all in-scope orgs; all Muse conversations and data; all OpenCode sessions (with the thinking-traces investigation answered); all ID objects across redb/Turso/hosted stores; plus the sources-inventory pass that hunts for missed sources.
- **Area:** atlas (primary). Cross-links: id (ID objects are a source, item 12 consumes the result), fleet (queue/PM data), pm (PM stack data).
- **Most related:** [#9097](https://github.com/developing-today/code/issues/9097) (the six-point source list and acceptance live here) · [#8622 corpus registry + memory-lancedb connector](https://github.com/developing-today/code/issues/8622) (slice 1a landed, PR #13319 merged) · [#8626 remaining corpus connectors](https://github.com/developing-today/code/issues/8626) · [#8623 embedding unification + per-corpus model identity](https://github.com/developing-today/code/issues/8623) · [#6025 first-pass repo/doc ingestion](https://github.com/developing-today/code/issues/6025) · [#6024 licensing gate for external GitHub corpus](https://github.com/developing-today/code/issues/6024) · [#6052 external-content three-zones policy](https://github.com/developing-today/code/issues/6052).
- **Status today:** **in flight, early.** Ingested on record: AI conversation sessions, the ChatGPT export and the sandbox-agents corpus (both interned into the CAS 2026-11-23). GitHub comments have a dedicated extractor in code; ID objects and queue/PM corpora have no live connector yet.
- **Ordering rationale:** Coverage is the multiplier on items 8, 12 and 13 — but it is sequenced after automation so new sources land in a pipeline, not in another manual pile.

#### 10. Atlas provenance complete — CAS, occurrences, catalog and shards (#3125 phases 3–4)
- **Type:** milestone
- **Description:** Finish the provenance-first rebuild: historical chunk-occurrence backfill (~127K occurrences dropped at first ingest; 6,344 sessions owning zero chunks), the catalog DB, collection×month shard layout, and resolve/fetch-span as MCP tools.
- **Area:** atlas (primary, sole).
- **Most related:** [#3125 provenance registry programme](https://github.com/developing-today/code/issues/3125) (open, priority/high) · cold-tier follow-ons [#8227 shard writer](https://github.com/developing-today/code/issues/8227) (already tasked, parked — needs a dedicated shift) –[#8231](https://github.com/developing-today/code/issues/8231) · [#9100 virtual filesystem over the CAS](https://github.com/developing-today/code/issues/9100).
- **Status today:** **in flight.** Phase 1 (occurrence-aware ingest) and phase 2 slices landed (12,092 files interned into the CAS with verify proof, 2026-11-23); phases 3–4 open.
- **Ordering rationale:** Provenance is the trust layer under dedup, deletion propagation and the ID-graph publication (item 12); finishing it before the recommender (item 13) keeps behavioural tables rebuildable.

#### 11. Storage placement settled — Turso→D1 programme + Atlas D1 hot cache
- **Type:** shift-goal
- **Description:** Decide and execute where each store lives (Turso vs Cloudflare D1 vs R2 vs local) on measured economics, and land the per-tenant D1 hot cache in front of Turso for the multi-tenant serving shape.
- **Area:** atlas (primary). Cross-links: id (the same placement question for hosted ID stores), pm (the audit's estate is PM-censused), fleet (fleet-platform stores are in the audit set).
- **Most related:** [#13844 Turso→D1 migration audit + programme](https://github.com/developing-today/code/issues/13844) (audit merged via PR #13845; 50-DB estate live-verified) · [#9134 D1 tenant hot-cache design](https://github.com/developing-today/code/issues/9134) (closed design) with slices [#13100 schema/store](https://github.com/developing-today/code/issues/13100) (**closed, landed**), [#13101 freshness](https://github.com/developing-today/code/issues/13101) + [#13103 admission](https://github.com/developing-today/code/issues/13103) (open, in review), [#13105 read-path](https://github.com/developing-today/code/issues/13105) (gated on #13101), [#13106 invalidation](https://github.com/developing-today/code/issues/13106) (**flag-held**), [#13107 measurement](https://github.com/developing-today/code/issues/13107) · [#9135 tenant store placement](https://github.com/developing-today/code/issues/9135) · [#8528 cost-aware placement simulator](https://github.com/developing-today/code/issues/8528).
- **Status today:** **in flight.** Hot-cache schema + store landed with the master flag OFF — production search does not touch D1 today. The migration programme is at audit stage.
- **Ordering rationale:** Placement is a one-way-door-ish decision that items 12, 17 and 25 all build on; the audit existing makes this the cheapest moment to settle it.

### Tier 3 — Search intelligence (Atlas × ID)

#### 12. Atlas↔ID integration — search runs across the core system (SEED)
- **Type:** shift-goal
- **Description:** End the split where ID has no live query path into Atlas and each server answers with basic per-server BM25-style queries: Atlas objects and provenance become ID-graph objects, ID content is vectorised into Atlas, and one retrieval core serves CLI, REST, MCP and ID.
- **Area:** atlas (primary). Cross-links: id (the graph side and its projectors live in the ID book's detail), mcpx (the MCP surface both sides share).
- **Most related:** [#6062 ID-graph publication format design](https://github.com/developing-today/code/issues/6062) (open, design-only) + projectors [#11182 versions/files/relations](https://github.com/developing-today/code/issues/11182), [#11183 attestations/ingest runs](https://github.com/developing-today/code/issues/11183), [#11184 projector operations](https://github.com/developing-today/code/issues/11184), [#11185 view presets](https://github.com/developing-today/code/issues/11185), [#11186 conformance audit](https://github.com/developing-today/code/issues/11186) (all open; #11182–#11185 enqueued in ID-100 at priority 70) · [#9109 #9106 Part C — storage economics, ID search, recommendations](https://github.com/developing-today/code/issues/9109) · [#9106 second ID programme epic](https://github.com/developing-today/code/issues/9106) · [#5755 fleet atlas-search fan-out over the shard core](https://github.com/developing-today/code/issues/5755).
- **Status today:** **designed, implementation not started.** Verified 2026-11-23: ID has **no live query path** into Atlas; the shared ground in code is only the content-hash v1 test vectors. Drew's "Atlas baked into ID" framing is recorded as a direction to explore, **not** a decision of record — the recorded decisions are #6062 (format) + #9109 Part C.
- **Ordering rationale:** The highest-leverage shift in the census: it converts two half-systems into one core. Ranked first in its tier because rerank/recommender quality (item 13) and ID-graph apps (item 15) both presume a unified corpus and ranking path.

#### 13. Rerank + recommender systems in Atlas (SEED)
- **Type:** capability-set
- **Description:** A learned reranking stage in the serving path (golden-set-gated) and a recommender built from real retrieval behaviour — session co-occurrence neighbours, feedback signals, recommendation objects with per-class retention and a "next 10" surface.
- **Area:** atlas (primary, sole; ID consumes via item 12).
- **Most related:** [#9140 recommendation objects, search history, retention classes](https://github.com/developing-today/code/issues/9140) (open, design) · [#10158 recommendation snapshot paging](https://github.com/developing-today/code/issues/10158) (open, in review; PR [#12873](https://github.com/developing-today/code/pull/12873) open, unmerged; hard dependency #9083) · [#8627 standing cross-agent eval sets](https://github.com/developing-today/code/issues/8627) (the quality oracle) · [#5733 Jaccard third-signal gate](https://github.com/developing-today/code/issues/5733) (eval results on record) · [#5773 query-understanding front-end](https://github.com/developing-today/code/issues/5773) · [#5774 query-time parameter surface](https://github.com/developing-today/code/issues/5774) · [#9092 search v1 build-out](https://github.com/developing-today/code/issues/9092).
- **Status today:** **designed; serving does not exist.** Verified: production fusion is reciprocal-rank fusion over FTS (BM25) + vector only; **no learned/cross-encoder reranker exists in the serving path**; feedback is captured (`atlas-feedback/v1`) with no ranking consumer found; recommendation *generation* does not exist — only the snapshot-paging state machine is in flight. Golden set: 311-query v3 + 54 held-out, the gate every candidate must pass.
- **Ordering rationale:** Deliberately after items 8–10: a reranker trained/evaluated on a frozen, provenance-poor corpus would bake in today's gaps. The eval gate (#8627) existing is what makes this safe to build next.

#### 14. Atlas as a multi-client search service — the product decision and its build
- **Type:** milestone
- **Description:** Answer the open product question — Atlas as a multi-client service (OpenChamber/fork sessions, ID, agents as first-class consumers) vs Drew's personal CLI — and build the service shape the answer implies, with per-agent identity instead of today's shared hosted identity.
- **Area:** atlas (primary). Cross-links: mcpx (per-client ceilings), fleet (`fleet search` is the client of record), id (consumer via item 12).
- **Most related:** [#12900 search improvements — thread 2 is this decision](https://github.com/developing-today/code/issues/12900) (open, in review; thread 1 parallel branches **landed** via PR #13039; thread 3 Rust binary substantially landed) · [#5350 epic: Atlas as Rust binary + multi-client service](https://github.com/developing-today/code/issues/5350) (open; children #5351–#5356 all closed) · [#8625 MCP surface + per-agent identity](https://github.com/developing-today/code/issues/8625) (open, blocked on cross-client acceptance) · [#6805 Worker cache eviction/invalidation](https://github.com/developing-today/code/issues/6805).
- **Status today:** **in flight, decision open.** A service shape already exists de facto (fleet search service + Worker + mcpx `atlas` server, all live), but the thread-2 commitment is unanswered on the issue, and all hosted MCP callers still share one identity (`hosted-per-user`, personal ceiling).
- **Ordering rationale:** The decision gates real investment in identity, caching and ceilings; it sits after item 12 because ID-as-consumer is the strongest evidence for the multi-client answer.

#### 15. ID apps on one graph — URIs, transclusion, typed links, graph display
- **Type:** capability-set
- **Description:** The ID graph programme: apps (notes, boards, worlds) sharing one graph with stable URIs, transclusion, typed links, sneakernet bundles and a graph display — the consumer-facing payoff of the ID platform epics.
- **Area:** id (primary). Cross-links: atlas (publication format, item 12), pm (boards lineage in the PM stack).
- **Most related:** [#13880 epic(id-apps): ID apps on one graph](https://github.com/developing-today/code/issues/13880) (open; created 2026-11-24T00:09:40Z — the newest programme issue in the census) · [#2056 epic: ID platform expansion](https://github.com/developing-today/code/issues/2056) · [#9129 versioning design](https://github.com/developing-today/code/issues/9129) + the blocked boards/feeds chain [#10237](https://github.com/developing-today/code/issues/10237) → [#10252](https://github.com/developing-today/code/issues/10252) → [#10269](https://github.com/developing-today/code/issues/10269) · [#5525 boards v1](https://github.com/developing-today/code/issues/5525) · [#3790 sticky-notes first app](https://github.com/developing-today/code/issues/3790).
- **Status today:** **designed + partially in flight.** Boards/feeds implementation landed in slices (e.g. #10237's code via PR #12932) but the chain sits open on stale labels with its close **flag-held**; the unifying epic #13880 was filed during this census window.
- **Ordering rationale:** First ID-platform item because it is the user-visible surface the store, hosting and federation work below exists to serve — its requirements should steer them.

### Tier 4 — ID platform build-out

#### 16. ID store abstraction — one interface over redb / Turso / D1 / IndexedDB
- **Type:** capability-set
- **Description:** One store interface with pluggable backends and a shared conformance suite, so an ID object behaves identically on a local redb, a Turso/D1 hosted store, or a browser IndexedDB.
- **Area:** id (primary). Cross-links: atlas (placement economics, item 11), lab (cluster stores).
- **Most related:** [#9082 store abstraction design](https://github.com/developing-today/code/issues/9082) (open, priority/high) · [#9110 extensibility architecture + conformance](https://github.com/developing-today/code/issues/9110) (open, priority/high) · [#10191 shared conformance fixtures](https://github.com/developing-today/code/issues/10191) (**closed**, PR #13194) · [#10192 lifecycle hooks + admission pipeline](https://github.com/developing-today/code/issues/10192) (open; first slice landed PR #13156) · [#5805 Postgres-over-HTTP adapter design](https://github.com/developing-today/code/issues/5805) · [#5814 adapter conformance harness](https://github.com/developing-today/code/issues/5814) · [#5681 serverless storage choice](https://github.com/developing-today/code/issues/5681).
- **Status today:** **in flight.** Conformance fixtures landed; the chain's only remaining foundational step is #9082's adopt/defer/reject act on the IndexedDB profile (see item 21).
- **Ordering rationale:** Every dynamic-hosting and offline item below is written against this interface; it is the ID tier's dependency root.

#### 17. Tunnels to spin up ID servers dynamically — hosted worlds and hosted editions (SEED)
- **Type:** capability-set
- **Description:** Spin up an ID server on demand — through a tunnel, against a local store, a Turso store, or Cloudflare R2/D1 — with the same behaviour hosted or local: host mode in the id binary, lifecycle as authority-frame actions, and the Cloudflare hosted edition (per-user Durable Objects, SSO→ID binding, signed-object bridge, export/import).
- **Area:** id (primary). Cross-links: fleet (tunnels/transport, item 3), lab (basement deployment pack runs on item 6), atlas (D1 cache shape shared, item 11).
- **Most related:** hosted-worlds chain [#10693 P1 host mode](https://github.com/developing-today/code/issues/10693) → [#10694 P2 lifecycle](https://github.com/developing-today/code/issues/10694) → [#10695 P3 restart/recovery](https://github.com/developing-today/code/issues/10695) → [#10696 P4 operator CLI](https://github.com/developing-today/code/issues/10696) → [#10697 P5 basement deployment pack](https://github.com/developing-today/code/issues/10697), + [#10698 P6 parity conformance](https://github.com/developing-today/code/issues/10698) · [#9078 hosted editions epic](https://github.com/developing-today/code/issues/9078) · [#2852 hosted provisioning + SSO binding](https://github.com/developing-today/code/issues/2852) · [#3818 hosted bridge v1](https://github.com/developing-today/code/issues/3818) (remainder [#13036](https://github.com/developing-today/code/issues/13036) flag-held) · [#3810 export/import](https://github.com/developing-today/code/issues/3810) (both code halves **landed** 2026-11-23, PRs #13201/#13212) · [#6276 hosted worlds core topic](https://github.com/developing-today/code/issues/6276) + custody [#6284](https://github.com/developing-today/code/issues/6284).
- **Status today:** **in flight, chain blocked at the head.** All of #10693–#10697 verified open + `status/blocked` at census; #10693 was enqueued in ID-100 (task 21463) with its ownership ruled (ID programme hosted-edition workstream, P1 in full). Phase-14 principle of record: local-first always; p2p behaviour identical to hosted; the hosted instance is the public backstop, never a requirement.
- **Ordering rationale:** This is the seed that turns ID from a binary into a platform; it depends on items 3, 6 and 16 and unlocks items 18–20's real deployments.

#### 18. Discovery, registry and federation at scale
- **Type:** capability-set
- **Description:** Worlds are findable and joinable across servers: publish-to-pkarr announcements, a discovery resolver (invite link, pkarr, registries in order), a public registry endpoint, and federation with boundary gates, jurisdiction routing and delete propagation.
- **Area:** id (primary, sole).
- **Most related:** [#6277 discovery/registry/federation core topic](https://github.com/developing-today/code/issues/6277) · [#10706 publish-to-pkarr](https://github.com/developing-today/code/issues/10706) (blocked) · [#10708 discovery resolver](https://github.com/developing-today/code/issues/10708) (blocked) · [#10712 public registry endpoint v2](https://github.com/developing-today/code/issues/10712) (blocked) · [#6289 federation outline](https://github.com/developing-today/code/issues/6289) · [#8281 gallery registry pkarr publication](https://github.com/developing-today/code/issues/8281) · [#7787 boundary enforcement](https://github.com/developing-today/code/issues/7787) (part 2 landed PR #13204) · jurisdiction chain [#3399](https://github.com/developing-today/code/issues/3399)–[#3404](https://github.com/developing-today/code/issues/3404) + [#5528](https://github.com/developing-today/code/issues/5528) · [#2091 federated delete](https://github.com/developing-today/code/issues/2091).
- **Status today:** **designed, head slices blocked.** The specific upstream blocker for #10706/#10708 was UNVERIFIED in the ID census (labels only) — recorded here as a gap, not guessed.
- **Ordering rationale:** Federation before scale (item 19): there is little value hardening a single-server world to 100× while joining across servers is still blocked.

#### 19. ID scale and performance hardening — measured limits at 10×/100×
- **Type:** problem
- **Description:** A generator + harness producing worlds at 10×/100× current sizes, measuring join time, replay, frame cost and memory on laptop- and server-class hosts, ending in a measured limits table — with Drew's performance doctrine (a multiplier is proven by benchmark, never asserted) as the bar.
- **Area:** id (primary). Cross-link: fleet (benchmark placement rides items 1–2).
- **Most related:** [#6281 scale core topic](https://github.com/developing-today/code/issues/6281) · [#6304 scale v1 design](https://github.com/developing-today/code/issues/6304) (open, in review; design doc landed PR #13090) · [#13083 stress-world generator](https://github.com/developing-today/code/issues/13083) (**closed** — 10× = 11,001 worlds / 200,000 events in 35.7 s) · [#13085 frame cost](https://github.com/developing-today/code/issues/13085) (**closed** — measured FLAT to 1M events) · [#13084 join-time benches](https://github.com/developing-today/code/issues/13084) (**flag-held**) · [#13086 memory profile](https://github.com/developing-today/code/issues/13086) (in progress) · [#13087 first 10× run](https://github.com/developing-today/code/issues/13087) · [#13088 100× feasibility](https://github.com/developing-today/code/issues/13088) · [#5685 DoS suite + benchmark programme](https://github.com/developing-today/code/issues/5685).
- **Status today:** **in flight.** Two of five measurement slices closed with real numbers; the chain is gated on the flag-held #13084 and the in-progress #13086. A codified wall was found: `WorldLimits` participants cap 256 vs tier targets 320 @10× / 3,200 @100× — a design decision the runs inherit.
- **Ordering rationale:** Measurement before optimisation, and before currency/federation load arrives; ranked below federation because its harness already exists and can run in parallel.

#### 20. Production currencies v1, then the scaling path
- **Type:** milestone
- **Description:** Ship production currencies v1 (graph↔ledger reconciliation, grant service, RateAuthorisation), then the designed scaling path Redis → multi-Turso → TigerBeetle.
- **Area:** id (primary, sole).
- **Most related:** [#10070 production currencies v1 remainder](https://github.com/developing-today/code/issues/10070) (already tasked — task 21343 parked) · [#2320 graph↔ledger reconciliation](https://github.com/developing-today/code/issues/2320) · [#2290 RateAuthorisation](https://github.com/developing-today/code/issues/2290) · [#2294 grant service](https://github.com/developing-today/code/issues/2294) · [#2292 TigerBeetle provisioning](https://github.com/developing-today/code/issues/2292) · [#5676 currency scaling design](https://github.com/developing-today/code/issues/5676).
- **Status today:** **in flight (v1 remainder parked).** The v1 line is the successor of the closed graph-native fold (#5522); scaling is design-only, sequenced behind v1 by standing triage verdicts.
- **Ordering rationale:** Money-shaped correctness outranks breadth: v1 is close and parked, so finishing it is cheap; the scaling path correctly waits for v1's measured load.

#### 21. IndexedDB local caching in web apps (SEED)
- **Type:** capability-set
- **Description:** Browser ID clients cache and work against a local IndexedDB store behind the same store interface (item 16), with conformance proven in a real browser engine.
- **Area:** id (primary, sole).
- **Most related:** [#10213 IndexedDB adapter prototype](https://github.com/developing-today/code/issues/10213) (**closed**, PR #13219) · [#9082 store abstraction — the adopt/defer/reject act](https://github.com/developing-today/code/issues/9082) · [#10191 conformance fixtures](https://github.com/developing-today/code/issues/10191) (closed; its `indexeddb` entry stays `reserved` by design until #9082 acts) · [#6377 decouple Web UI from core](https://github.com/developing-today/code/issues/6377) (open, in review) → [#6381 AI file management](https://github.com/developing-today/code/issues/6381) (blocked on #6377).
- **Status today:** **prototype done; adoption decision open.** Verified prototype verdict: per-batch whole-database atomic scope, quota ≈10.7 GB, `persisted: false` (headless Chromium conformance suite, `pkgs/id/browser/`).
- **Ordering rationale:** The prototype de-risked the seed; what remains is one decision (#9082) plus production wiring — small, and it is the prerequisite half of the offline seed (item 23).

#### 22. Dynamic Android platform — many apps, push, touch, GPS, driver scope (SEED)
- **Type:** shift-goal
- **Description:** One dynamic Android platform hosting many ID apps, with push notifications, touchscreen, GPS and driver scope exposed to the hosted apps — and the **same shape** for the Bevy/native shell and the web app: one shell contract, many surfaces.
- **Area:** id (primary). Cross-links: pm (push-notification lineage — the PM stack's Web Push is the working reference), lab (hardware shells).
- **Most related:** [#1831 Android app input interface / PoC](https://github.com/developing-today/code/issues/1831) (enqueued in ID-100 at priority 70, task 21480) · [#7938 native Android client v1](https://github.com/developing-today/code/issues/7938) (already tasked — task 21350 done; slice 1 landed PR #13445 via #13403) · [#6279 / #6297 more shells — mobile, native, hardware](https://github.com/developing-today/code/issues/6297) (v2 outline landed 2026-11-23) · [#10930 shells notification capability provider](https://github.com/developing-today/code/issues/10930) (ID-100, priority 70) · [#10931 shells conformance-suite packaging](https://github.com/developing-today/code/issues/10931) · [#12817 shells second native target](https://github.com/developing-today/code/issues/12817) · [#1071 native TUI client](https://github.com/developing-today/code/issues/1071) · [#5781 Bevy incorporation](https://github.com/developing-today/code/issues/5781) (design ADOPTED 2026-11-23, PR #7672 as design of record) · [#12657 rich presentation surfaces + Bevy renderers](https://github.com/developing-today/code/issues/12657) (step 2a landed PR #13214; 2b audio + 2c widget remain) · shell contract `pkgs/id/contract/shell-contract.v1.json`.
- **Status today:** **in flight, early.** Android v1 slice 1 landed; the dynamic multi-app platform with the full driver scope (push/GPS) is designed at outline level, not built. The Bevy shell exists as the native reference shell speaking the same contract.
- **Ordering rationale:** The shell programme is where ID becomes tangible on the devices Drew actually carries; it is ranked after the store/hosting items because every shell is a client of them — but its contract work (already landing) should not pause.

#### 23. Service-worker offline / local-first web apps (SEED)
- **Type:** capability-set
- **Description:** Web apps that keep working offline: service-worker shell + IndexedDB cache (item 21), syncing when the network returns — the web expression of ID's local-first principle.
- **Area:** id (primary). Cross-link: pm (the PM console is the other web surface that needs the same pattern).
- **Most related:** **No single owning issue exists** — this seed is the least-issued item in the census. Its substrate is [#9082](https://github.com/developing-today/code/issues/9082) + [#10213](https://github.com/developing-today/code/issues/10213) (closed) and the Phase-14 local-first principle recorded on [#6312](https://github.com/developing-today/code/issues/6312) / [#6304](https://github.com/developing-today/code/issues/6304); the web layer itself is `id-web` + [#6377](https://github.com/developing-today/code/issues/6377). A dedicated service-worker/offline issue is a **census finding**: it should be filed under the ID programme rather than left implicit.
- **Status today:** **not started as a named item.** The IndexedDB half is prototyped; no service-worker work was found in the ID census.
- **Ordering rationale:** Last in the ID tier by dependency (it composes items 16, 17 and 21), not by value — local-first is a stated principle, and this is its web proof.

#### 24. ESP32 hardware platform line — ID on microcontrollers
- **Type:** capability-set
- **Description:** ESP32-family boards as an ID hardware shell — sensors and LoRa connected to the ID project — a lab chapter today, its own line long-term.
- **Area:** lab (primary). Cross-link: id (the hardware shell crate `id-hw-shell` and the shells programme, item 22).
- **Most related:** [#5783 ESP32 hardware platform line](https://github.com/developing-today/code/issues/5783) (open, design; already tasked by a completed first pass, task 19892) · per-device verification docs in `developing-today/hardware-doc` (mostly ESP32 boards today — exact part names are verified there, not guessed here).
- **Status today:** **designed / side-line.** Recorded in the hardware census as "a side project of a side project for now — noted, not scheduled."
- **Ordering rationale:** Genuinely long-term: it inherits the shell contract (item 22) and the lab's physical readiness (items 4–6), and blocks nothing.

### Tier 5 — Gateway, PM and forks surfaces

#### 25. mcpx completion — supervisor, session resume, config tail, embedding surface
- **Type:** problem
- **Description:** Finish the MCP gateway: a supervisor owning every stdio child's pipes for its whole life (replacing the takeover handoff), cold-restart session resume completed, the settings effect-probe exemption tail burned to zero, and the embedding/API surfaces (Go API, C ABI, WASM, mcpx as its own upstream) decided.
- **Area:** mcpx (primary). Cross-links: atlas (the `atlas` MCP server is live through mcpx), id (ID's MCP bridge + code-mode interop), fleet (fleet MCP via mcpx designs).
- **Most related:** [#13264 mcpx programme tracker](https://github.com/developing-today/code/issues/13264) · [#1900 backlog super-tracker](https://github.com/developing-today/code/issues/1900) · [#1384 supervisor design](https://github.com/developing-today/code/issues/1384) + [#9963 Phase 3: supervised stdio by default](https://github.com/developing-today/code/issues/9963) (PRs merged; acceptance check by #13264) · [#1414 session-resume design](https://github.com/developing-today/code/issues/1414) (phases 1–2 landed; Phase 3 DELETE question open) · exemption tail = exactly 8 settings (5 unheld/unclaimed; 3 flag-held: `plugin.instructions`, `plugin.disabledTools` via [#8472](https://github.com/developing-today/code/issues/8472), `protoTasks.cancelGrace` via [#8473](https://github.com/developing-today/code/issues/8473)) · [#1877 embedding mcpx](https://github.com/developing-today/code/issues/1877) · [#1950 OAuth for remote servers](https://github.com/developing-today/code/issues/1950) (**landed** 2026-11-23, PR #12869) · [#1940 code-mode interop](https://github.com/developing-today/code/issues/1940) · fleet-MCP design-mill passes #10389/#10390/#10393/#10405 (all open).
- **Status today:** **in flight, close on several fronts.** Daemon + ~50 commands + 252 settings live; the book is the largest in the project (84 chapters, ~288k words) and is the Pages root. Atlas→mcpx integration is LIVE (the laptop's mcpx carries the `atlas` remote server).
- **Ordering rationale:** mcpx is the tool gateway every agent already uses daily, so its reliability items outrank its expansion items; it sits in Tier 5 because Tiers 0–4 are its dependencies, not its dependents.

#### 26. PM stack completion — backlog as derived state, hosted PM, push end-to-end
- **Type:** capability-set
- **Description:** The PM stack finishes its own roadmap: backlog state derived from DB + GitHub with markdown as a view, the hosted-editions PM service, decision sets v2, and the push path (aggregator → Web Push → phone → answer) deployed end-to-end.
- **Area:** pm (primary). Cross-links: fleet (queues/dispatch are shared), id (hosted editions are one epic, #9078), atlas (PM data is an ingest source, item 9).
- **Most related:** [#13504 PM platform tracker](https://github.com/developing-today/code/issues/13504) (gained 5 linked children in AUDIT-7 Lane 7) · [#9060 multi-board hierarchy + task genealogy epic](https://github.com/developing-today/code/issues/9060) · [#5007 backlog state derived from DB + GitHub](https://github.com/developing-today/code/issues/5007) (verdict of record: correctly blocked on phase ordering — do not start yet) · [#9335 decision sets v2](https://github.com/developing-today/code/issues/9335) (acceptance landed + live-verified 2026-11-13; awaiting close) · [#9078 hosted editions epic](https://github.com/developing-today/code/issues/9078) · PM-BOOK programme [#13900](https://github.com/developing-today/code/issues/13900) (book skeleton reconciled 2026-11-23: ~48 chapters; workers census 22, Turso census 50).
- **Status today:** **in flight.** Aggregator + console live (29 routes + 21 console views censused); the notify relay is built but **not deployed** (its answer routes unrouted — disagreement D5 in the PM survey).
- **Ordering rationale:** PM is the project's own coordination surface — valuable, but it coordinates Tiers 0–4 rather than blocking them; its blocked-on-ordering verdict (#5007) is respected in this ranking.

#### 27. OpenCode / OpenChamber forks programme — one shared routing implementation
- **Type:** problem
- **Description:** Converge the forks' duplicated machinery — AUDIT-7 Lane 7's top convergence candidate: quota/routing fixes recur in both forks (249 + 201 titles) — and finish OpenCode 2 config/plugin migration.
- **Area:** pm (primary — the forks programme is carried as the liftable section of the PM book). Cross-links: fleet (routing/quota doctrine is shared), mcpx (harness adapters + plugin v2).
- **Most related:** [#730 OpenCode 2 config validation](https://github.com/developing-today/code/issues/730) (active 2026-11-23) · [#3286 OpenCode 2 remainder](https://github.com/developing-today/code/issues/3286) · [#1858 mcpx opencode plugin v2](https://github.com/developing-today/code/issues/1858) + remainder [#2841](https://github.com/developing-today/code/issues/2841) (blocked) · [#13874 AUDIT-7 Lane 7 report](https://github.com/developing-today/code/issues/13874) (the convergence evidence) · [#1334 fleet pool & routing intelligence epic](https://github.com/developing-today/code/issues/1334) (slices landing 2026-11-23).
- **Status today:** **in flight.** Forks are 66% "fix"-titled work in the census window — a reactive stream, which is itself the finding this item answers.
- **Ordering rationale:** Real leverage (one implementation instead of two fix streams) but no other census item is blocked on it; it follows the surfaces it serves.

#### 28. Fleet secrets, CA and quota plane — the trust and capacity layer
- **Type:** capability-set
- **Description:** The fleet CA/credentials design completed, the secrets broker hardened (no silent eviction on partial refresh), and the quota plane's staged acceptance (S3 observation history → S4 single-pool trial → S7 recurring activation) finished.
- **Area:** fleet (primary). Cross-links: pm (admission policy is pm-stack-owned, consumed by fleet), mcpx (per-client ceilings).
- **Most related:** [#9155 Fleet CA, credentials & secrets v0.3 design mill](https://github.com/developing-today/code/issues/9155) (already tasked — task 17413 parked at expert ruling) · [#5504 secrets-broker partial-refresh eviction](https://github.com/developing-today/code/issues/5504) · [#12694 quota acceptance](https://github.com/developing-today/code/issues/12694) · [#13591 S7 installation follow-up](https://github.com/developing-today/code/issues/13591) · [#13081 secrets health-gate / cold restart](https://github.com/developing-today/code/issues/13081) · fleet book roadmap order 4–5.
- **Status today:** **in flight.** S3 history is accumulating (first qualifying record 2026-11-23 02:30 UTC; 63 records in 61 of 62 quarter-hour buckets at census); the weekly-review timer was absent on the laptop at inspection — S7 activation is the open tail.
- **Ordering rationale:** Placed late deliberately: the quota doctrine (burn to 100% at reset; a failed read never idles a pool) means today's manual stewardship works; this item makes it systematic rather than possible.

### Tier 6 — The record itself

#### 29. Design-register burn-down — AUDIT-7 L5/L6 (cite, don't re-derive)
- **Type:** problem
- **Description:** Close the design gap the audit measured: 111 issues whose design is missing/very simple and 121 more deficient/partial, designed as families where the audit flags convergence — the Supabase→Turso family (#5690–#5695), the Atlas/ID hosted cluster (#9068–#9155), and #9085 before #9081.
- **Area:** pm (primary — the register is cross-project). Cross-links: all books (each family's designs land in its own book).
- **Most related:** [#13871 AUDIT-7 umbrella](https://github.com/developing-today/code/issues/13871) · [#13873 Lane 6 design-gaps](https://github.com/developing-today/code/issues/13873) (report PR #13901 merged; 30 packets: P1 [#8986 design-register epic](https://github.com/developing-today/code/issues/8986), [#12950](https://github.com/developing-today/code/issues/12950), [#12951](https://github.com/developing-today/code/issues/12951), [#9521 Nix-cache content store](https://github.com/developing-today/code/issues/9521), [#2025 review-loop design](https://github.com/developing-today/code/issues/2025)) · [#8983](https://github.com/developing-today/code/issues/8983)/[#8986](https://github.com/developing-today/code/issues/8986) register epics · sufficient-design counterexample: the pm-orchestration family #9478/#9479/#9443/#9444/#9481–#9484 (the audit's only "sufficient" set in code).
- **Status today:** **in flight.** Audit complete 2026-11-24; the first 28 prioritised design packets + 2 continuation packets are live in the queue.
- **Ordering rationale:** Designs gate implementation everywhere, but the audit already sequenced this work into packets; the merged list's job is to keep it visible as one item, not to re-rank its interior.

#### 30. The books and Spaces themselves — Code Book programme completion
- **Type:** milestone
- **Description:** This programme's own deliverables: the Code Book, the Lab Book, per-book high-level sections in fleet/atlas/id/mcpx (+ PM via its tracker), the Code Space and Lab Space, and the AliExpress hardware-inventory issue.
- **Area:** pm (primary — programme coordination). Cross-links: all books carry a section (see §2).
- **Most related:** [#13905 programme tracker](https://github.com/developing-today/code/issues/13905) (slices: Census A/B/C, both books, per-book sections, PM coordination comment, AliExpress issue, Spaces, closing coverage audit) · [#13900 PM-BOOK programme](https://github.com/developing-today/code/issues/13900) · book plumbing of record: one combined Pages deploy in `mcpx-docs.yml`, siteUrls `/code/code/` and `/code/lab/`.
- **Status today:** **in flight.** Fleet/atlas/id/mcpx books all exist and deploy; the Code Book and Lab Book are this programme's remaining builds; the AliExpress inventory is pending Drew's access step (no login attempted by the programme, per the tracker).
- **Ordering rationale:** Last by construction — the record describes the work; it does not precede it. Its per-book sections (§2) are nevertheless the delivery vehicle for this whole list.

---

## 2. PER-BOOK ASSIGNMENT TABLE

Every merged-list item appears under its **primary** book in full; cross-linked items are repeated with a ◦ marker so each book's section can be built completely from its own list (Deliverable 2 completeness rule). Item numbers are §1 numbers.

### fleet — Fleet book (`pkgs/fleet/book/`, roadmap: `development/program-roadmap.md`, issue map: `operations/program-issue-map.md`)
| # | Item |
|---|---|
| 1 | Fleet queue, claim and placement correctness |
| 2 | Containerised build/compile systems — in the cluster and in Actions |
| 3 | Transport and P2P auth — one honest transport |
| 28 | Fleet secrets, CA and quota plane |
| ◦6 | Talos cluster live on Cozystack (fleet consumes in-cluster builds/hosting) |
| ◦7 | Backups + storage coherent set (fleet/queue Turso stores) |
| ◦9 | All repos / all ID objects into Atlas (queue + PM data as sources) |
| ◦14 | Atlas multi-client search service (`fleet search` is the client of record) |
| ◦17 | Dynamic ID servers via tunnels (transport leg) |
| ◦19 | ID scale harness (benchmark placement) |
| ◦25 | mcpx completion (fleet MCP via mcpx designs #10389/#10390/#10393/#10405) |
| ◦26 | PM stack completion (shared queues/dispatch) |
| ◦27 | Forks programme (shared routing/quota doctrine) |
| ◦29 | Design-register burn-down (fleet families) |
| ◦30 | Books and Spaces (fleet book section) |

### atlas — Atlas book (`pkgs/atlas-search/book/`, roadmap: `design/roadmap-and-issue-map.md`)
| # | Item |
|---|---|
| 7 | Backups + storage automated, coherent complete set |
| 8 | Atlas ingestion fully automated |
| 9 | All repos / all ID objects into Atlas ingestion |
| 10 | Atlas provenance complete (#3125 phases 3–4) |
| 11 | Storage placement — Turso→D1 programme + D1 hot cache |
| 12 | Atlas↔ID integration — search across the core system |
| 13 | Rerank + recommender systems in Atlas |
| 14 | Atlas as a multi-client search service |
| ◦6 | Talos cluster (storage tiers) |
| ◦15 | ID apps on one graph (publication consumer) |
| ◦17 | Dynamic ID servers (shared D1 cache shape) |
| ◦25 | mcpx completion (Atlas MCP surface, live) |
| ◦26 | PM stack data as an ingest source |
| ◦29 | Design-register burn-down (Atlas families) |
| ◦30 | Books and Spaces (atlas book section) |

### id — ID book (`pkgs/id/book/`, roadmap: `design/roadmap-and-issue-map.md`)
| # | Item |
|---|---|
| 15 | ID apps on one graph (#13880) |
| 16 | ID store abstraction (redb / Turso / D1 / IndexedDB) |
| 17 | Tunnels to spin up ID servers dynamically; hosted worlds + hosted editions |
| 18 | Discovery, registry and federation |
| 19 | Scale and performance hardening (10×/100×) |
| 20 | Production currencies v1 + scaling path |
| 21 | IndexedDB local caching in web apps |
| 22 | Dynamic Android platform (+ Bevy/web same shape) |
| 23 | Service-worker offline / local-first web apps |
| ◦3 | Transport and P2P auth (ID sync/relay) |
| ◦9 | All ID objects into Atlas ingestion (source side) |
| ◦11 | Storage placement (hosted ID stores) |
| ◦12 | Atlas↔ID integration (graph/projector side: #6062, #11182–#11186) |
| ◦13 | Rerank + recommender (ID search consumer, #9109 Part C) |
| ◦24 | ESP32 hardware line (hardware shell) |
| ◦25 | mcpx completion (ID MCP bridge, code-mode interop) |
| ◦29 | Design-register burn-down (ID families) |
| ◦30 | Books and Spaces (id book section) |

### mcpx — mcpx book (`pkgs/mcpx/docs/`, 84 chapters; no programme-roadmap chapter yet — trackers carry it)
| # | Item |
|---|---|
| 25 | mcpx completion — supervisor, session resume, config tail, embedding surface |
| ◦12 | Atlas↔ID integration (shared MCP surface) |
| ◦14 | Atlas multi-client service (per-client ceilings; `atlas` server live in mcpx) |
| ◦22 | Shells programme (harness adapters are the agent-side shells) |
| ◦27 | Forks programme (opencode plugin v2, harness adapters) |
| ◦28 | Quota plane (per-client ceilings design question) |
| ◦29 | Design-register burn-down (mcpx families: #1384/#9963 acceptance) |
| ◦30 | Books and Spaces (mcpx book section; mcpx book is the Pages root) |

### pm — Project Management book (`pm-stack/book/`, via PM-BOOK tracker #13900 — requested by comment, not edited by this programme)
| # | Item |
|---|---|
| 26 | PM stack completion — derived backlog, hosted PM, push end-to-end |
| 27 | OpenCode / OpenChamber forks programme (liftable PM-book section) |
| 29 | Design-register burn-down (cross-project register home) |
| 30 | Books and Spaces — programme coordination (Code Book owns the merged list) |
| ◦1 | Queue/placement correctness (PM orchestration subprogramme keeps its own design ordering) |
| ◦2 | Actions-exec / compile dispatch (PM dispatch views) |
| ◦7 | Backups estate (PM census: 50 Turso DBs, 22 Workers) |
| ◦9 | PM stack data into Atlas ingestion |
| ◦11 | Turso→D1 audit (PM-censused estate) |
| ◦15 | ID apps on one graph (boards lineage) |
| ◦17 | Hosted editions (PM service is one of the editions, #9078) |
| ◦22 | Android platform (Web Push reference implementation is PM's) |
| ◦23 | Offline/local-first (PM console as second web surface) |
| ◦28 | Quota plane (admission policy is pm-stack-owned) |

### lab — Lab Book (new, under #13905; cluster work lives here until it earns its own book)
| # | Item |
|---|---|
| 4 | Switches integrated and racked |
| 5 | All servers online, auto-connecting, presenting to the cluster on boot |
| 6 | Talos cluster booted live, accepting data on Cozystack |
| 24 | ESP32 hardware platform line (lab chapter; ID cross-link) |
| ◦2 | Containerised builds in the cluster (runs on item 6) |
| ◦7 | Backups local leg (cluster storage, DRBD/LINSTOR tiers) |
| ◦17 | Basement-cluster deployment pack for hosted ID worlds (#10697) |
| ◦30 | Lab Book + Lab Space + AliExpress inventory issue (this programme) |

**Completeness check:** items 1–30 each appear exactly once as a primary assignment: fleet 4 (1, 2, 3, 28) · atlas 8 (7–14) · id 9 (15–23) · mcpx 1 (25) · pm 4 (26, 27, 29, 30) · lab 4 (4, 5, 6, 24). Total 30. ✓

---

## 3. TIMING NOTES

Per item: when it entered the record, planned/projected dates where they exist, reprioritisations and misses with reasons. **Where the record has no timing data, this section says "no timing in record" — nothing here is projected by the census itself.**

| # | Timing in record |
|---|---|
| 1 | Entered as fleet programme [#13272](https://github.com/developing-today/code/issues/13272), 2026-11-23. Repair-verb production drain and generation-6 broker state are dated 2026-11-23 in the fleet book roadmap. No completion date in record. |
| 2 | [#13765](https://github.com/developing-today/code/issues/13765) and [#13774](https://github.com/developing-today/code/issues/13774) both filed 2026-11-23 (21:14Z / 21:21Z). Actions-exec slices were landing through 22:10Z the same day. No projected dates in record. |
| 3 | [#13752](https://github.com/developing-today/code/issues/13752) filed 2026-11-23T21:11Z; [#13779](https://github.com/developing-today/code/issues/13779) filed 2026-11-23T21:29Z. No projected dates in record. |
| 4 | No timing in record. Switch inventory itself is incomplete (make/model/count missing) — the Lab census under #13905 is the first dated step. |
| 5 | No timing in record. Hardware state (no OS installed, power-capacity blocker, 3–5 machines first) is as recorded 2026-10-20/21 in the hardware register; no reprioritisation recorded since. |
| 6 | The v1.9-era Talos test cluster was torn down (confirmed by Drew 2026-10-26) — a recorded **miss/reset**: the current programme (`basement-cluster.md`) restarts from declarative bootstrap. No live date in record. |
| 7 | Atlas daily backup timer is live today (no start date in record). [#9093](https://github.com/developing-today/code/issues/9093) mesh: no timing in record. Drew's keep-everything rule is dated 2026-10-31. |
| 8 | Continuous-ingestion design is dated by [#9097](https://github.com/developing-today/code/issues/9097)'s source list (Drew, 2026-11-12). **Miss of record:** canonical counts frozen at the 2026-10-31 publish because ingestion stayed manual — the freeze is the stated reason the recount procedure is an acceptance item. No projected automation date in record. |
| 9 | ChatGPT export + sandbox-agents corpus interned 2026-11-23. Thinking-traces investigation: no date in record. |
| 10 | [#3125](https://github.com/developing-today/code/issues/3125) direction dated 2026-10-31; phase-2 backfill slice landed 2026-11-23 (12,092 files, verify OK). Phases 3–4: no dates in record. |
| 11 | [#13844](https://github.com/developing-today/code/issues/13844) audit filed 2026-11-23T23:16Z; audit doc merged same day (PR #13845); live estate read ~23:05Z 2026-11-23. D1 hot-cache design merged 2026-11-23 (PR #13098); [#13107](https://github.com/developing-today/code/issues/13107)'s first tuning round is specified as "after ~2 weeks of counters" — the only relative projection in this item; counters cannot start until #13105 lands. #13106 is flag-held (reprioritisation by gate, not by date). |
| 12 | [#6062](https://github.com/developing-today/code/issues/6062) design open at census; projectors [#11182](https://github.com/developing-today/code/issues/11182)–[#11185](https://github.com/developing-today/code/issues/11185) enqueued 2026-11-23 (ID-100, priority 70). No projected dates in record. |
| 13 | Golden-set gate results on record from 2026-11-08 design onward; [#10158](https://github.com/developing-today/code/issues/10158)'s PR #12873 open at census. No ship date in record — the gate doctrine is explicit that a reranker that hurts on Atlas's own queries does not ship, whenever it is ready. |
| 14 | [#12900](https://github.com/developing-today/code/issues/12900) exploration opened by Drew 2026-11-04; moved into this repo 2026-11-22; thread 1 landed 2026-11-23 (PR #13039). Thread-2 decision: no date in record. |
| 15 | [#13880](https://github.com/developing-today/code/issues/13880) filed 2026-11-24T00:09Z — entered the merged list during this census. Boards chain [#10237](https://github.com/developing-today/code/issues/10237)→[#10252](https://github.com/developing-today/code/issues/10252)→[#10269](https://github.com/developing-today/code/issues/10269): implementation landed but close is flag-held — a recorded stall, reason = stale labels + held oracle re-run, not missing code. |
| 16 | Conformance fixtures [#10191](https://github.com/developing-today/code/issues/10191) closed via PR #13194 (2026-11-23 programme records). #9082's IndexedDB decision: no date in record. Sibling [#10215](https://github.com/developing-today/code/issues/10215) (NATS adapter) is tier **contingent** with a recorded review date of **2027-02-12** — the only far-future date in the census. |
| 17 | Hosted-worlds queue tasks were recorded blocked-until **2026-11-26** (programme records) — a dated gate, now the chain's pacing fact. Export/import halves landed 2026-11-23. #10693 ownership ruled and task enqueued (21463) 2026-11-23. |
| 18 | No timing in record; the #10706/#10708 upstream blocker is UNVERIFIED (a census gap carried forward honestly). |
| 19 | #13083/#13085 closed with measurements 2026-11-23 programme window; #13084 flag-held (no date — Drew item-specific approval); #13087/#13088 inherit the chain with no independent dates. |
| 20 | [#10070](https://github.com/developing-today/code/issues/10070) task 21343 parked at census; no date in record. Scaling (#5676) is explicitly sequenced behind v1 by triage verdicts — a recorded reprioritisation (v1 first). |
| 21 | [#10213](https://github.com/developing-today/code/issues/10213) closed via PR #13219 after both its blockers (#10160, #10191) closed — all in the 2026-11-23 window. Adoption decision: no date in record. |
| 22 | [#7938](https://github.com/developing-today/code/issues/7938) slice 1 landed (PR #13445) in the 2026-11-23 window; [#1831](https://github.com/developing-today/code/issues/1831) enqueued 2026-11-23 (task 21480, priority 70); [#5781](https://github.com/developing-today/code/issues/5781) Bevy design **adopted** 2026-11-23 — a recorded decision, ending its design phase. No platform ship date in record. |
| 23 | No timing in record — and no owning issue (see §1 item 23 finding). |
| 24 | No timing in record; explicitly "noted, not scheduled" in the hardware record. |
| 25 | Exemption-tail shrink log is dated per setting (2026-11-17 → 2026-11-23, #8467–#8474 chains); 11 sibling plugin exemptions cleared 2026-11-23. [#1950](https://github.com/developing-today/code/issues/1950) OAuth landed 2026-11-23 (PR #12869). Programme tracker [#13264](https://github.com/developing-today/code/issues/13264) filed 2026-11-23T12:39Z. No completion date in record. |
| 26 | [#9335](https://github.com/developing-today/code/issues/9335) acceptance landed + live-verified **2026-11-13** (awaiting close only — a recorded near-miss of process, not of work). [#5007](https://github.com/developing-today/code/issues/5007): verdict of record = correctly blocked on phase ordering; no start date. PM-BOOK skeleton reconciled 2026-11-23 ~18:45 CST. |
| 27 | OpenCode 2 work active 2026-11-23 (Lane DL done note). AUDIT-7 convergence evidence is dated 2026-11-23/24. No projected dates in record. |
| 28 | **The one hard near-term date in the census:** the quota S3 seven-day interval can first elapse **2026-11-30 at 02:30:06.420 UTC** (fleet book roadmap, from the first qualifying record 2026-11-23 02:30 UTC). S7's weekly-review timer was absent at inspection — [#13591](https://github.com/developing-today/code/issues/13591) tracks its first scheduled invocation; no date in record. [#9155](https://github.com/developing-today/code/issues/9155) task 17413 parked at expert ruling, no date. |
| 29 | AUDIT-7 filed 2026-11-23 ~17:56 CST ([#13871](https://github.com/developing-today/code/issues/13871) created 23:57:31Z); Lane 6 complete 2026-11-24T00:38Z with 30 packets live. Continuation packets (21579/21580) carry the next batches; no completion date in record. |
| 30 | Programme directive: Drew, 2026-11-23 ~18:43 CST; tracker [#13905](https://github.com/developing-today/code/issues/13905) created 2026-11-24T00:45:30Z. Slice checkboxes carry no dates in the tracker. AliExpress inventory: access step pending Drew — no date in record, no login attempted. |

---

## Appendix — census gaps carried forward (not silently dropped)

1. **Service-worker/offline (item 23) has no owning issue.** Recommendation: file one under the ID programme (#13240) so the seed stops being implicit.
2. **Switches (item 4) have no owning issue and an incomplete inventory.** The Lab census (Census C) owns closing this; until then the merged list cites the research docs, not an issue.
3. **Discovery blocker for #10706/#10708 is UNVERIFIED** (labels only in the ID census). Item 18's first slice should be a blocker-identification read, not implementation.
4. **Book roadmap paths differ per book** (fleet: `development/` + `operations/`; atlas/id: `design/roadmap-and-issue-map.md`; mcpx: none yet). Deliverable 2's sections should not force one path on all four books; mcpx needs its roadmap chapter created, the others need the §2 lists merged into the chapters that exist.
5. **Flag holds pace several items** (#13106, #13084, #13036, the #10237-chain close, 3 mcpx exemptions). They are recorded per item above as gates with reasons; per standing rule they are not re-asked as a batch in this census.
