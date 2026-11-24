# Atlas — Data Sources & Consumers Inventory

Research for the Atlas program-planning prompt. Compiled 2026-11-23 from
first-hand `gh api` reads of developing-today/code issues and repo docs, the
hatch-personal `workspace/turso-atlas/` operational docs, and the two prior
research files — cross-referenced, not duplicated:

- ID/Atlas current state: `~/workspace/id-investigation/research-id-state.md` §5
  (Atlas definition, [#3125](https://github.com/developing-today/code/issues/3125)
  provenance direction, [#10412](https://github.com/developing-today/code/issues/10412)
  secret-tier review, [#6062](https://github.com/developing-today/code/issues/6062)
  ID-publication format, [#9109](https://github.com/developing-today/code/issues/9109) Part C).
- Embedding/vector technology options:
  `~/workspace/id-investigation/research-tech.md` §5.

Program homes: [#8983](https://github.com/developing-today/code/issues/8983)
(epic: Atlas program — unified search and memory index over every corpus,
OPEN; ~57–68 open `epic/atlas-program` issues) and
[#6018](https://github.com/developing-today/code/issues/6018) (Atlas ingestion
program, OPEN). Drew's standing decision, recorded on
[#1300](https://github.com/developing-today/code/issues/1300) and implemented by
[#8567](https://github.com/developing-today/code/issues/8567) (closed):
fleet/agent memory integrates **into Atlas** — one queryable surface across all
agents/corpora, no new standalone vector store, no per-agent silos.

**Headline state (verified on [#9097](https://github.com/developing-today/code/issues/9097),
2026-11-12): ingestion today is manual/script-driven.** The only Atlas schedule
on the laptop is the daily backup timer. Canonical counts are frozen at the
2026-10-31 publish — 18,325 sessions / 122,124 messages / 107,639 chunks —
while the Turso monolith has since grown to ~7.3 GB without a verified recount.
The staging tier and promote gate exist in code; the aggregator/embedder and
session-close hook slices are not built. Everything in §2 below is therefore
mostly _designed or filed_, not running.

---

## 1. Sources already in Atlas

### 1.1 The ingest pipeline as it exists

Per the operational README (`dezren39/hatch-personal:workspace/turso-atlas/README.md`):

1. **Collect** — `scripts/atlas-ingest.py` (hatch-personal
   `workspace/turso-atlas/scripts/`) runs on any machine (sandbox VM, Framework
   laptop, work Mac). Each AI client keeps conversations in a default on-disk
   location; the script has **per-client adapters + a per-machine
   `locations.json`** and reads those stores. Nothing is uploaded by this step.
2. **Normalize** — every conversation becomes one shape in a local SQLite file
   `data/atlas.db`: sessions → messages → chunks (~3,500 chars, deduplicated
   by content hash), plus `source_roots`, `chunk_occurrences` (every observed
   location of a chunk, with span and how the occurrence was established), and
   a `cas_registry` for content-addressed source locators. Re-ingest is
   idempotent for already-recorded occurrences. **This file is the durable
   asset**; everything else rebuilds from it + the sources.
3. **Index** — built locally from the chunks: FTS5 keyword index (in atlas.db)
   - per-model vector sets (~768-dim embeddings). Vectors are explicitly
     _derived and disposable_; a better model means building a second vector set
     beside the first from the same chunk text, never re-collecting.
4. **Publish** — a _copy_ of the hot index (keyword + current-best vectors over
   the most valuable chunks) is pushed to Turso, plus coordination/session
   DBs. Turso is distribution only; the full store stays on the laptop and
   Turso never holds the only copy of anything.
5. **Access** — the `atlas-search` CLI (§3.4).

Storage shape (per [#6018](https://github.com/developing-today/code/issues/6018),
verified 2026-11-04): 31 TursoDB shards — internal tier = 28 shards /
107,001 chunks; secret tier = exactly 1 shard / 638 chunks, with secret
material supposed to exist **only** in the 2 secret DBs (but see §4.3 — that
boundary is currently breached in the monolith). Production monolith stays
libSQL; TursoDB for all new databases; cold tiers → rustfs. Spend anchor:
Turso scaler $24.92/mo yearly is the plan ceiling.

### 1.2 Source classes actually feeding the corpus

| Source                                                                                                                                                                                                                                                                                                               | Evidence                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | Volume signal                                                                                    | Ingest path today                                                                                                                                                                                                                                                | Freshness                                               |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------- |
| **AI chat/session corpora** — the founding corpus. Client adapters read on-disk conversation stores; the session-logs class in the ingestion-operations design names **OpenCode session store** and **Muse session transcripts** explicitly (`doc/2026-11-09T05-12-00Z_design_atlas_ingestion_operations`, §2 table) | turso-atlas README step 1; #6019 design doc §2                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | The whole canonical count: 18,325 sessions / 107,639 chunks (2026-10-31)                         | Manual/script-driven runs of `atlas-ingest.py` per machine; designed trigger is event-driven (session end / file close stages finalized sessions only) — the session-close hook is **not built** ([#9097](https://github.com/developing-today/code/issues/9097)) | Frozen at 2026-10-31 publish; no verified recount since |
| **ChatGPT export**                                                                                                                                                                                                                                                                                                   | [#3125](https://github.com/developing-today/code/issues/3125) phase-2 backfill records (2026-11-23): the ChatGPT export's CAS hash matches the October design record; the backfill interned 12,092 files (~2.6 GB) into the Atlas CAS with `ATLAS-EXTRACTS VERIFY OK`                                                                                                                                                                                                                                                           | Part of the 12,092-file backfill                                                                 | One-shot export file, interned content-addressed under #3125 phase 2 (hatch-personal PR #67 added `--files-from` interning)                                                                                                                                      | Point-in-time export; no automatic refresh path         |
| **GitHub issue/PR comments — all own repos**                                                                                                                                                                                                                                                                         | [#6023](https://github.com/developing-today/code/issues/6023) **CLOSED**: "extract the GitHub corpus — all issue/PR comments from all our repos; external set gated on licensing"; writer landed: [#10688](https://github.com/developing-today/code/issues/10688) **CLOSED** "GitHub comments ingestion writer + extraction feed"; atlas-core ships an `atlas-github-comments` binary (`pkgs/atlas-search/crates/atlas-core/src/bin/atlas-github-comments.rs`)                                                                  | All comments across Drew's repos (org census: 102 admin orgs — count from fleet program records) | Extraction feed + ingestion writer exist; _continuous_ re-extraction is part of open [#9097](https://github.com/developing-today/code/issues/9097) item 2, not a running schedule                                                                                | First pass done; continuous updates not running         |
| **Code / config artifacts**                                                                                                                                                                                                                                                                                          | [#10788](https://github.com/developing-today/code/issues/10788) **CLOSED**: "stage code/config artifacts as embedding entries — item writer + atlas-stage ingest"; [#6025](https://github.com/developing-today/code/issues/6025) **OPEN**: first-pass corpus — all code repos, all findable docs, dezren39/cairn                                                                                                                                                                                                                | All code repos (first pass still open)                                                           | Staged via `atlas-stage`; the full first-pass corpus task (#6025) remains open                                                                                                                                                                                   | Partial                                                 |
| **Query traces / statistics & reports class**                                                                                                                                                                                                                                                                        | [#8246](https://github.com/developing-today/code/issues/8246) / [#10364](https://github.com/developing-today/code/issues/10364) **CLOSED**: trace ingestion + retention as statistics-and-reports corpus; [#10418](https://github.com/developing-today/code/issues/10418) **CLOSED**: statistics aggregation + weekly report from traces; atlas-core has `trace_ingest.rs` / `trace_stats.rs`                                                                                                                                   | Atlas's own query traces                                                                         | Built into atlas-core                                                                                                                                                                                                                                            | Landed                                                  |
| **Boot / reboot records, statistics snapshots, generated reports**                                                                                                                                                                                                                                                   | Designed source classes with triggers and inclusion tests in the #6019 operations design (`doc/2026-11-09T05-12-00Z_design_atlas_ingestion_operations` §2): boot records from `~/logs/init.jsonl` + init-status records per fleet host (event-driven on boot-completion); statistics = quota/statistical reports, provider usage snapshots, fleet health rollups (sweep on the report's own cadence, final version only); reports = generated briefs/sweep reports/design records outside the repo (event-driven on file close) | Per-host, per-period                                                                             | **Design only** — the recurring sweep runner is [#7969](https://github.com/developing-today/code/issues/7969) OPEN (runner core landed via [#11210](https://github.com/developing-today/code/issues/11210), closed)                                              | Not running as a schedule                               |

### 1.3 The semi-automated ingest design that governs all of the above

[#6019](https://github.com/developing-today/code/issues/6019)'s operations
design (closed; record on main, cited in §1.2) defines how any source joins:
**semi-automated** means discovery, filtering, staging, and accounting run
unattended; adding a source class, loosening an inclusion test, bulk backfills,
and plan changes are always human decisions. Every class must answer a stated
question ("what question would this ever answer?"); rejects are logged with the
same manifest shape as accepts (`no-question`, `duplicate-content`, `oversize`

> 256 MiB, `secret-tier-held`, `superseded-version`). Pipeline: delta-only
> discovery ([#6028](https://github.com/developing-today/code/issues/6028),
> span-anchored offset matching) → usefulness + classification-manifest filter →
> content-addressed landing in the **staging shard** ([#3125](https://github.com/developing-today/code/issues/3125))
> → promotion through the ingest gate ([#6057](https://github.com/developing-today/code/issues/6057):
> write → classify → dedup → provenance-check → publish; merge-on-close per
> [#5669](https://github.com/developing-today/code/issues/5669)) → per-batch
> in-toto-shaped attestation ([#6045](https://github.com/developing-today/code/issues/6045)).
> Disk/IO guardrails and the Turso plan ceiling are part of the same design.
> **Status: designed and partially built (gate + staging + runner core landed);
> the continuous runner ([#7969](https://github.com/developing-today/code/issues/7969))
> and continuous-ingestion program ([#9097](https://github.com/developing-today/code/issues/9097),
> `status/blocked`) are still open.**

---

## 2. Sources that are natural next ingest targets

Drew's own target list is already written down — [#9097](https://github.com/developing-today/code/issues/9097)
(Workstream E of the [#8983](https://github.com/developing-today/code/issues/8983)
program, 2026-11-12): "constantly updating, every source, internal only."
Its six items, plus the surrounding connector issues:

### 2.1 Fleet queue / broker data + PM stack data — #9097 item 1

- **Holder:** the queue broker's Turso backend on the laptop
  (`placement_sessions` table added 2026-11-23 by
  [#13146](https://github.com/developing-today/code/issues/13146) slice 1;
  tasks, sessions, claims, parks, dispositions) and the PM stack (aggregator
  state, boards, reconcile records).
- **Automatic ingest shape:** event hooks where the source supports them,
  scheduled sweeps otherwise (#9097's rule); delta-only via broker watermarks;
  the ingest watermark doubles as the corpus-change signal for cache policy.
- **Status:** named in #9097 (open, blocked); no connector built. Queue/PM
  content is operational data — internal tier by default; session transcripts
  attached to tasks inherit the session classification rules.

### 2.2 GitHub — everything, all orgs — #9097 item 2, extending #6023

- **Holder:** GitHub, via `gh` CLI / REST (the GitHub connector is banned by
  standing rule; all access is `gh api`). Scope per #9097: **all issues / PRs /
  commits / files for all orgs and repos in scope** (developing-today,
  developing-today-forks, Clustered-Computing, personal) — 102 admin orgs per
  fleet records.
- **What exists:** comments extraction is done (#6023 closed, #10688 writer
  landed). **What's missing:** issues/PRs as first-class objects (not just
  comments), commits, and repo files beyond the #6025 first pass; continuous
  delta updates (webhook or ETag/`since`-cursor sweeps) instead of one-shot
  extraction.
- **Constraint:** the **external** half (repos Drew interacts with but doesn't
  own) is gated on the licensing research —
  [#6024](https://github.com/developing-today/code/issues/6024) OPEN ("may we
  ingest external GitHub comments/discussions into Atlas?") and the
  three-zone external-content policy
  [#6052](https://github.com/developing-today/code/issues/6052) OPEN (deferred
  by Drew behind the [#6066](https://github.com/developing-today/code/issues/6066)
  networking epic per [#8567](https://github.com/developing-today/code/issues/8567)).
  Design may proceed; external ingestion may not be built until the gate
  answers. Per-repo license capture at ingest is already built
  ([#7608](https://github.com/developing-today/code/issues/7608), closed).

### 2.3 All Muse conversations and data — #9097 item 3

- **Holder:** the Muse platform itself — main chat + side chats, the memory
  tree (`~/MEMORY.md`, `~/memory/**`), artifacts/documents produced for Drew.
- **Automatic ingest shape:** **[unverified mechanism]** — no export/API path
  is recorded on any issue found; today Muse session material reaches Atlas
  only insofar as transcripts land in stores the client adapters already read.
  #9097 names it as a target with no connector issue yet; its sources-inventory
  acceptance item is where the mechanism gets decided.

### 2.4 All OpenCode sessions and data — #9097 item 4

- **Holder:** the OpenCode session store on the laptop (server at
  127.0.0.1:13420) + OpenChamber session data.
- **Automatic ingest shape:** session-end event hook (the unbuilt session-close
  hook from §1.2) or sweep over the session store; the design's session-logs
  class already covers OpenCode.
- **Open question filed in #9097 itself:** an explicit investigation of
  ingesting **thinking traces**, with a recommendation covering volume, value,
  and classification implications — undecided.

### 2.5 All ID objects — #9097 item 5 + the #6062 publication line

- **Holder:** ID's own stores — laptop redb, Turso ID DBs, hosted stores as
  they arrive.
- **Automatic ingest shape:** the reverse direction is the designed one today:
  Atlas objects publish _into_ the ID graph —
  [#6062](https://github.com/developing-today/code/issues/6062) (publication
  format design, OPEN) + projector set
  [#11182](https://github.com/developing-today/code/issues/11182)–[#11186](https://github.com/developing-today/code/issues/11186)
  (all OPEN, unstarted). #9097 item 5 asks for ID objects _into_ Atlas search
  as well; [#9109](https://github.com/developing-today/code/issues/9109) Part C
  states the end state: "automatic vectorization of ID content (Snowflake +
  the second embedder from Atlas
  [#9092](https://github.com/developing-today/code/issues/9092)) feeding
  built-in ID search." Neither direction is built; see
  `research-id-state.md` §5.2 for the full record.

### 2.6 The sources-inventory pass — #9097 item 6

#9097 makes the inventory itself a deliverable ("anything missed gets its own
connector issue"). It names: **notes/zettel** (dezren39/zettel +
developing-today/zettel, Emanote), **Fleet Reports Space records**,
**ask-ledger logs**, **quota/burn records**, **laptop state stores**. Related
connector work already filed under
[#8567](https://github.com/developing-today/code/issues/8567):

- [#8622](https://github.com/developing-today/code/issues/8622) OPEN — memory
  slice 1: corpus registry + **memory-lancedb connector** into Atlas staging
  (OpenClaw's local memory store is the first external corpus connector).
- [#8626](https://github.com/developing-today/code/issues/8626) OPEN — memory
  slice 4: remaining corpus connectors — **memory files, notes, repo docs,
  research reports**.
- [#9576](https://github.com/developing-today/code/issues/9576) OPEN — C8:
  Atlas ingestion of reports & pages + error-driven investigation boosts.
- Agent-notes placement (zettel vs repo vs directly into the ID graph) is
  [#6018](https://github.com/developing-today/code/issues/6018) topic 3, tied
  to [#5545](https://github.com/developing-today/code/issues/5545) /
  [#5686](https://github.com/developing-today/code/issues/5686).

### 2.7 Drew-world sources with no Atlas record found

Recorded factually, as candidates the inventory pass (#9097 item 6) has not
yet dispositioned — **no ingest issue exists for any of these** (searched
2026-11-23):

- **Gmail** — no Atlas ingest exists or is filed. Standing constraint: the
  Gmail connector is currently **NOT connected**, and all Gmail/Plaid
  background checks are **paused on Drew's order (2026-11-22)** regardless of
  connection state (`~/MEMORY.md` standing record). Any future mail ingest
  needs both reconnection and Drew lifting the pause; mail would also be a
  internal/private-tier source by default.
- **Google Drive / Docs** — not named in #9097's list; "all findable docs" in
  [#6025](https://github.com/developing-today/code/issues/6025) refers to repo
  docs. Drive holds Drew's documents/notebooks; an ingest would ride the
  Drive export/download surface. **[unverified — no design record found]**
- **Slack (Developing Today workspace)** — the #6019 design's chat-logs class
  covers "fleet chat exports (Slack/Discord-class sources as they are
  connected)": scheduled hourly sweep over the export delta, **channel
  allowlist, human-approved per channel**, bot/heartbeat posts excluded.
  Whether a live Slack export feed is connected today is **[unverified]**;
  no connector issue found.
- **CI run history** (GitHub Actions runs across the orgs) — not named as a
  source class; nearest neighbours are the statistics class (fleet health
  rollups) and trace ingestion. Would ingest via the Actions API on a sweep.
  **[no record found]**
- **The Turso databases themselves** (beyond Atlas's own shards) — the queue
  broker DB is §2.1; other fleet Turso DBs (fleet-config D1, ID DBs) fold
  into #9097 items 1/5 rather than a separate class.
- **The memory/topics corpus in hatch-personal** — Drew's curated long-term
  memory (`~/memory/topics/**`, `~/MEMORY.md`) is explicitly in scope via
  #9097 item 3 ("memory tree") and #8626 ("memory files").

### 2.8 Standing constraints on any new ingest

- **Tier assignment at ingest, per source class** — the classification
  manifest ([#5410](https://github.com/developing-today/code/issues/5410)) is
  the home for per-source tier rules (§4).
- **Secret-tier content is held out of automated sweeps entirely** until the
  local embedder lands ([#6059](https://github.com/developing-today/code/issues/6059),
  open/in-review) — never sent to a hosted embedder (§4.2).
- **External content** waits on #6024/#6052 (§2.2).
- **Keep-all rule** ([#3125](https://github.com/developing-today/code/issues/3125)):
  keep ALL data; only byte-identical duplicates are cleaned; backups AND
  plain extracts for every corpus; every chunk records _every_ place it was
  found (`chunk_occurrences`) — dedup must never silently drop provenance
  (it had dropped ~127K occurrences before #3125).
- **Plan ceiling:** Turso scaler $24.92/mo is a hard envelope; sweeps carry
  disk/IO guardrails and per-batch embedding-cost accounting
  ([#6058](https://github.com/developing-today/code/issues/6058) ledger design).

---

## 3. Consumers and their search needs

### 3.1 `fleet search` — the agent-facing CLI (live)

`pkgs/fleet/crates/fleet-search/src/cli.rs` (developing-today/code, main):
`fleet search` **is** the atlas-search client surface, ported flag-for-flag
with a parity guard (`.github/scripts/check-fleet-search-parity.sh`); the
fleet workspace deliberately carries no second embedder/backend.

- **Who:** every fleet agent/seat (Muse lanes, OpenCode sessions, other
  seats) looking up prior work — past decisions, session evidence, "have we
  seen this before" — across the corpora in §1. Drew's Atlas order
  (2026-11-22): `fleet search` documented for all agents/sessions, which
  closed [#12782](https://github.com/developing-today/code/issues/12782).
- **Query shape:** hybrid FTS+vector (default), `--mode fts|vector`; filters
  `--kind/--topic/--project/--client`, `--since/--until` (session dates),
  `--max-sensitivity` ceiling, `--group-by-session`, `--expand N` neighbours,
  `--with-message`, `--hydrate N`, `--pool N` per-branch candidates,
  `--k N` final results; `get-chunk <id>` returns one chunk with provenance.
  Default embedding model: Snowflake `snowflake-arctic-embed-m-v2.0`;
  `--embedder auto|fastembed|st`; vector backends `scan|index-fill|index-fixed`.
- **Routing/latency:** sandbox seat → via-service by default (job protocol
  against the laptop's loopback service, `http://127.0.0.1:18760`, through the
  laptop-ssh forward) with a loud standalone fallback; laptop seat →
  standalone by default. Service enforces ceilings server-side by client
  identity: `sandbox-scripts` is capped at `personal`, `drew-cli` at
  `secret-bearing`.
- **Freshness need vs reality:** agents assume the corpus is current; it is
  frozen at the 2026-10-31 publish (§1) until #9097/#7969 land. **This is the
  single largest consumer-facing gap.**

### 3.2 The Atlas MCP server — mcpx namespace (built, acceptance open)

[#8625](https://github.com/developing-today/code/issues/8625) OPEN — memory
slice 3: an MCP server (an mcpx namespace) in front of atlas-core's
`run_search`. The design's decided access pattern ([#8567](https://github.com/developing-today/code/issues/8567)
design record `pkgs/atlas-search/docs/memory-integration-design-8567.md`):
MCP for cross-agent access; native library access stays Atlas-internal only.

- **Landed:** [#12785](https://github.com/developing-today/code/issues/12785)
  CLOSED — MCP endpoint on the search Worker;
  [#12809](https://github.com/developing-today/code/issues/12809) CLOSED —
  Atlas MCP server wired into the laptop's mcpx daemon as a remote server
  (Drew's 2026-11-22 Atlas order, all three parts complete).
- **Contract (from #8625):** the MCP layer stamps the calling agent's
  identity, enforces its sensitivity ceiling and corpus scoping **inside the
  branch queries, never post-ranking**, and audit-logs every query. Every hit
  carries corpus + `source_ref` provenance.
- **Consumers named in the acceptance criteria:** OpenClaw, an OpenCode
  session, OpenChamber, and a Muse lane — each querying the unified index
  through the namespace with its own identity and ceiling. #8625 stays open
  on that cross-client acceptance verification.
- **Query shape:** same hybrid search as §3.1, identity-scoped; freshness
  need is the same corpus-currency gap.

### 3.3 ID search & recommendations — #9109 Part C (designed, unbuilt)

[#9109](https://github.com/developing-today/code/issues/9109) (Part C of the
[#9106](https://github.com/developing-today/code/issues/9106) epic) is the
program-level statement that ID search and recommendations are Atlas-facing:

- **Query shape:** search _over the ID object graph_ — automatic vectorization
  of ID content (Snowflake + the second embedder from
  [#9092](https://github.com/developing-today/code/issues/9092)) feeding
  built-in ID search; plus **recommendation surfaces**: one fan-out worker, a
  Durable Object accumulating a "next 10", recommendations as stored objects
  with per-class retention.
- **Also in Part C:** the storage-economics substrate ID search would run on
  — latency/characteristic benchmarks across D1 vs R2 vs laptop redb vs
  laptop Turso at various shard levels, the D1 tenant hot cache (the
  [#13100](https://github.com/developing-today/code/issues/13100)–#13107
  chain; #13106 flag-held), tenant store placement.
- **Missing today:** everything above the substrate — the #6062 publication
  format is undesigned-to-unbuilt, the projectors (#11182–#11186) are
  unstarted, and there is no ID-side query surface. Latency/freshness needs
  are not yet specified on any issue found **[unverified]**.

### 3.4 The `atlas-search` CLI / service — programmatic access today

From the turso-atlas README (§1.1 step 5) and the atlas-search Worker
(`pkgs/atlas-search/`): two JSON-first subcommands — `search` (hybrid FTS5 +
vector, reciprocal-rank fusion k=60, per-branch ranks and full provenance on
every result) and `get-chunk`. Backends: `--backend turso` (published DB over
HTTPS, read-only token resolved at runtime — env, Bitwarden by name, or
turso-CLI-minted; never stored in the repo) or `--backend local --db PATH`.
The Rust service (`atlas-core`: embed, classify, detect, eval, disclosure,
boundary, fusion, shards modules) is the same engine behind §3.1/§3.2, with a
query front-end ([#5773](https://github.com/developing-today/code/issues/5773)),
shard fan-out ([#5755](https://github.com/developing-today/code/issues/5755)),
and a full parameter surface with per-run vector logging
([#5774](https://github.com/developing-today/code/issues/5774)).
[#9092](https://github.com/developing-today/code/issues/9092) OPEN is the
search-v1 build-out: Workers fan-out, shard benchmarks, cache policy, second
embedder. **Missing for programmatic users:** any documented, versioned API
contract beyond the CLI/Worker surface and the MCP namespace — the
`atlas-proto` crate (`pkgs/atlas-search/crates/atlas-proto/`,
`query-parameters-v1.json`) is the seed of one.

### 3.5 OpenClaw / agent memory — the memory-integration consumer

[#8567](https://github.com/developing-today/code/issues/8567)'s corpora —
OpenClaw's memory-lancedb content, zettelkasten/Emanote notes, repo docs,
prior research reports — are consumed by agents (OpenClaw, OpenCode sessions,
OpenChamber, Muse lanes) as _memory_: recall-shaped queries ("what do we
know about X", prior decisions) rather than document search. Slices:
[#8622](https://github.com/developing-today/code/issues/8622) (registry +
connector), [#8625](https://github.com/developing-today/code/issues/8625)
(MCP surface, §3.2), [#8626](https://github.com/developing-today/code/issues/8626)
(remaining connectors), [#8623](https://github.com/developing-today/code/issues/8623)
/ [#8627](https://github.com/developing-today/code/issues/8627) (open children
of [#8983](https://github.com/developing-today/code/issues/8983)).
**Missing today:** the connectors themselves (#8622/#8626 open), so memory
recall still runs against per-agent silos (e.g. OpenClaw's local
memory-lancedb) instead of the unified index.

### 3.6 Evaluation as a consumer

Atlas's own quality loop consumes the corpus: golden-set eval
([#6046](https://github.com/developing-today/code/issues/6046) design;
[#8062](https://github.com/developing-today/code/issues/8062) records the
open seam — the evaluator scores supplied rankings, it does not run
retrieval), retrieval statistics + feedback
([#6027](https://github.com/developing-today/code/issues/6027) /
[#8245](https://github.com/developing-today/code/issues/8245)–[#8249](https://github.com/developing-today/code/issues/8249):
per-stage trace, thumbs up/down, result↔query trace with exact embed state).
Any query-planning/reranking work in the planning prompt inherits this as
its measurement prerequisite — see `research-id-state.md` appendix and the
v2–v4 baseline inventory (`doc/2026-11-11T03-45-00Z_design_atlas_v2_v4/research/01-baseline-inventory.md`).

---

## 4. Sensitivity / privacy model

### 4.1 The tier model — two namings, one ladder

- **Corpus tiers** (storage/classification, per
  [#6018](https://github.com/developing-today/code/issues/6018) /
  [#6021](https://github.com/developing-today/code/issues/6021)): **public /
  internal / secret** in the design records; physically, internal = 28 shards
  / 107,001 chunks, secret = exactly 1 shard / 638 chunks, and secret material
  may exist **only** in the 2 secret DBs.
- **Client-surface ceilings** (per `fleet search --help` and the turso-atlas
  README): `public < internal < private < sensitive`, default
  ceiling **internal** — private and sensitive material is excluded
  unless the ceiling is explicitly raised. Ceilings are enforced
  **server-side** and clamped per client identity (`sandbox-scripts` →
  personal; `drew-cli` → secret-bearing). The classification manifest
  ([#5410](https://github.com/developing-today/code/issues/5410); design
  `doc/2026-11-05T13-03-49Z_design_atlas_manifest_filter_surface`) declares
  the non-sensitivity filter dimensions and per-source tier rules, but code
  retains the sensitivity ladder — **a manifest cannot widen access**, and an
  invalid manifest falls back loudly without widening (v2–v4 baseline
  inventory V1-D08).
- Classification is enforced **before ranking** and again on each
  expansion/hydration step (V1-D01/D12) — a search can never surface, count,
  or leak the existence of chunks above the caller's ceiling.

### 4.2 The scanning + review machinery

- **Deterministic scanning (gitleaks-class patterns) + classifier pass +
  vector-neighbourhood check**: [#6021](https://github.com/developing-today/code/issues/6021)
  (CLOSED) revalidated the whole corpus — sweep the internal tier for
  secret-class content (credential-shaped strings, keys/tokens,
  personal/financial/medical details), including internal chunks that embed
  _close to_ secret chunks; verify the converse (secret tier holds only
  secret-class content) and that secret material exists nowhere else,
  including the production monolith and local replicas/caches.
- **The Layer 1 secret-tier review**: [#10412](https://github.com/developing-today/code/issues/10412)
  (CLOSED) — 47 candidate chunks (67 findings) + 45 candidate messages from
  the #6021 scan, each ruled confirmed/dismissed by Drew personally.
- **Presidio NER pass**: [#6048](https://github.com/developing-today/code/issues/6048)
  (CLOSED) — full internal-tier + sampled secret-tier PII detection at
  corpus scale.
- **Handling rule (absolute):** findings are handled at the secret tier's
  sensitivity — reports name counts, classes, and locations (DB + chunk IDs)
  and **never reproduce candidate content** in issues, chat, or logs.
  Confirmed mis-tiered chunks get a remediation path (move + re-embed at the
  correct tier, purge from the wrong tier including replicas) filed before
  the scan task closes.
- **Embedder rule:** secret-tier text is **never sent to a hosted embedder**.
  Until the local secret-tier embedder
  ([#6059](https://github.com/developing-today/code/issues/6059)) lands,
  secret-tier source classes are held out of automated sweeps entirely
  (`secret-tier-held` is a queue, not a rejection — held items re-offer
  automatically once the route exists). A dedicated secret-tier embedder
  design exists (`doc/2026-11-10T20-55-00Z_design_atlas_secret_tier_embedder`).
- **Cross-tier comparability:** one Snowflake model **and pipeline** across
  tiers (decision `doc/2026-11-08T10-44-00Z_decision_atlas_cross_tier_vector_comparability`);
  no arbitrary cross-model score merging — this is what makes the
  vector-neighbourhood mis-tier check in #6021 meaningful.

### 4.3 The known boundary breach — #8479

[#8479](https://github.com/developing-today/code/issues/8479) **OPEN** —
"Atlas boundary remediation: secret-bearing rows still live in the
production monolith and a local replica (#6021 wave 1)". The shard set's
boundaries are clean, but the same 638 sensitive chunks also live in
the production libSQL monolith and its local replica, plus pre-tiering
unclassified snapshots — so the standing claim "secret material exists only
in the 2 secret DBs" is currently **false outside the shard set**. Purge vs
secret-tier reclassification is undecided; per fleet records the issue waits
on production ingestion verification (a destructive purge gate). **Any new
ingest or consumer work must treat #8479 as a precondition for claiming the
tier boundary holds.**

### 4.4 What must never be ingested, or must be gated

- **Never:** secret-tier content into any non-secret store, replica, cache,
  or hosted embedder (§4.2); candidate secret content reproduced into
  issues/chat/logs at any point in the pipeline (§4.2 handling rule);
  external content above its zone policy (§2.2) — and per the deletion
  design (`doc/2026-11-09T00-42-59Z_design_atlas_deletion_propagation`),
  source deletions / repo privation must propagate into Atlas, including
  backup/restore handling, as a policy-governed purge distinct from ordinary
  retention retirement.
- **Gated on human approval (per #6019 design):** adding any new source
  class; loosening a class's inclusion test; adding a chat channel to the
  allowlist; bulk backfills beyond the runbook bound; any plan change;
  reclassifying a session to the secret tier before promotion.
- **Gated on decisions:** external GitHub corpus (#6024 licensing research +
  #6052 zone policy, both OPEN); OpenCode thinking traces (#9097's explicit
  investigation, undecided); Gmail anything (§2.7 — connector disconnected +
  Drew's pause order).
- **Not a gate but a rule:** sweeps never delete. Retention/retirement is a
  _report_ a sweep surfaces; erasure happens only through the
  deletion-propagation process or a human decision. Feedback/usefulness
  signals can propose movement or retirement, never authorize erasure
  (v2–v4 baseline inventory V1-D06/D13).

---

## Appendix — verification notes

- Verified first-hand in this pass (gh api, 2026-11-23): #6018, #6021,
  #8567, #8625, #8983, #9097, #9109 bodies; titles/states for #3125, #5410,
  #6023, #6024, #6025, #6029, #6048, #6052, #6057, #6059, #7969, #8479,
  #8622, #8626, #9092, #9576, #10412 (via prior pass), #10688, #10788,
  #11182–#11186; `pkgs/atlas-search/` + `pkgs/fleet/crates/fleet-search/`
  trees; `fleet-search/src/cli.rs` (the `fleet search` surface);
  `doc/2026-11-09T05-12-00Z_design_atlas_ingestion_operations` (source-class
  table); `doc/2026-11-11T03-45-00Z_design_atlas_v2_v4/research/01-baseline-inventory.md`;
  hatch-personal `workspace/turso-atlas/README.md` + atlas path census.
- Taken from the two prior research files (same-day, first-hand there):
  #3125's phase structure and counts, #10412's review counts, #6062's scope,
  the 57-open-`epic/atlas-program` census, the D1 hot-cache chain.
- Explicitly **[unverified]**: the mechanism for ingesting Muse platform
  data (#9097 item 3); whether a live Slack export feed is connected;
  Google Drive/Docs as a source (no record found); CI run history as a
  source (no record found); ID search latency/freshness requirements (not
  specified on any issue found); which specific client adapters exist in
  `atlas-ingest.py` beyond the OpenCode/Muse stores named in the
  #6019 design (the README describes adapters generically).
