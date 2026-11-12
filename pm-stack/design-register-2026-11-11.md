# 60-day design audit — REGISTER (synthesis)

## Coverage — what was combed, and the one gap

- **Combed:** Drew's inputs across the day-files (~/memory/2026-09-28.md → 2026-11-11.md),
  memory (MEMORY.md, memory topics, the memory bank) and the original chat messages across
  the **main chat and the side chats** (Turso, Cloudflare, Supabase, dashboard-status,
  Repo sweeps, Outstanding Questions, library-build, stewardship) as recorded in those
  day-files, plus the systems of record: GitHub issues in the four fleet repos, the code
  repo's doc/ design corpus, goal files, cron bodies, and the decisions ledger.
- **The effective window is Sep 28 – Nov 11, 2026, not Sep 12.** Sep 12–27 predates Muse
  entirely: the first Muse chat (the main chat) was created 2026-09-28T04:28:53Z, the
  earliest day-file is 2026-09-28, and zero fleet-repo issues were created before Sep 28.
  That zero is evidence-backed, not unsearched (see Appendix B, slice 1).
- **The one gap:** Drew's ChatGPT-era inputs survive only as an import summary
  (940 conversations, 2023-02-10 → 2026-10-17, imported 2026-10-19; topics in his 2026
  radar include Turso, Cozystack, Talos, RustFS). No raw export corpus exists on disk, so
  a literal day-by-day comb of Drew's pre-Muse inputs is not possible from local sources,
  and the summary itemizes no design asks in this audit's domains for Sep 12–27.
- **Second limit, stated plainly:** the code repo's 236-entry design-doc corpus (F1.4) is
  registered as a corpus, not entry-by-entry — per-entry issue states were not checked
  (335 lookups were out of scope for the records pass), and the open-issue census behind
  the records pass is title-keyword only, capped at 30 per term.

## Headline counts

- **Register entries: 117** — 42 top-level entries + 75 named sub-designs, deduped by
  design (not by mention) across the six slices and the records pass:
  Atlas 18 (2 top-level + 16 sub-designs) · ID & fleet 41 (4 parents + 37 sub-designs
  across ID / mcpx / OpenCode / orchestration) · PM stack 18 (2 parents + 16 sub-entries) ·
  Cloudflare 12 (9 top-level + 3 sub-designs) · Turso 5 · Adjacent infra 23
  (20 top-level + 3 cluster sub-designs).
- **Plus 131 idea-engine entries** registered as one parent (T3) with every title
  preserved in Appendix A.
- **Design status (across the 117):** the plurality are *built-into-running-system*
  (the fleet builds what Drew asks for, fast, usually the same day); the decided-but-only-
  *partial* band is the fat middle (Atlas service epic, ID Phase-14 ladder, PM coherence
  children, pool epic, unification slices); *drafted*-only designs cluster in Atlas search
  and the ID design mill; *none* (no design artifact) is rare and always flagged.
- **Never-issued: 43 items triaged** — **9 STILL-LIVE** (the filing manifest in §7),
  **29 DONE-WITHOUT-ISSUE** (built via cron/goal/direct build; register-only),
  **5 SUPERSEDED/DEAD** (register-only, closed); 3 further never-issued items deliberately
  left unclassed, with reasons (§7).
- **Status conflicts resolved in synthesis: 6** (detailed in place: A1.13 #3125's title
  vs its contents, F4.15 #8579 already closed, X15 #8264's actual identity, F1.2 the six
  decisions, X5.3 the cluster-epic supersede question, X18 #5717 vs the init fast-path).

Compiled 2026-11-11 from six day-slice extracts (2026-09-12 → 2026-11-11) plus the
records pass (design docs incl. the 236-entry doc/ appendix, the 131-entry idea log,
the decisions ledger, goals, and an open-issue census), with ~45 issue statuses
spot-verified live via `gh` during synthesis (marked **[live-checked 2026-11-11]**).
No issue number, status, or quote below is invented: every claim traces to a slice
line, a records-pass line, or a live `gh` check run in this pass.

Gap pass merged 2026-11-11: +7 entries (G1–G7, marked in place as A1.16, F3.7, F3.8,
F3.9, F4.18, P1.13, X20) found by scanning the places the main comb did not fully
cover — the full hatch-personal tree, sandbox + laptop scratch space, and the
OpenCode session corpus via Atlas (791 in-window sessions; Drew's direct OpenCode
sessions are not in the Muse-side record). Plus 3 origin corrections (F4.3
pool/routing origin 2026-10-05; multi-instance providers origin 2026-10-24, recorded
at F4.3; F1.2 forks-org origin 2026-10-12) and A1's stale landing-tier follow-up
closed (Turso Scaler, 2026-11-04). ChatGPT raw export verdict: definitively absent
from both machines; the Atlas-held copy (938 conversations, undated) was title-scanned
— no program-level design asks in the audit domains, so a raw export's yield for this
register would be marginal (dates, not designs). Full detail: GAP-ADDENDUM.md
alongside this register.

Field key per entry:
- **design_status**: none / drafted / decided / built-into-running-system
- **implementation_status**: not started / partial / shipped / shipped-then-superseded / abandoned-in-record
- **tracking**: issue links with state, goal, cron, doc-only, or NEVER-ISSUED
- **triage** (never-issued items only): (a) STILL-LIVE / (b) DONE-WITHOUT-ISSUE / (c) SUPERSEDED-DEAD

---

## 1. ATLAS

### A1. Atlas program — tiered AI knowledge base on a Turso Pro month (PARENT)
- first_asked: 2026-10-20 — Drew's "priority one": redeem the unredeemed free-month coupon for ANY Turso plan incl. Pro $499, build Atlas during the month, land on Developer $5.99 (Scaler $29 as pressure valve); redeem only after a readiness sprint proves the pipeline on Free.
- timeline of Drew's asks:
  - 2026-10-20 — design confirmed: hot set ≤8 GB in Turso, full corpus as portable SQLite shards in R2 + Backblaze B2, Supabase Postgres canonical relational, Workers query API + cached answers; DB-per-session/agent with nightly consolidation and search-before-research; corpus priority corrected by Drew the same night (zettel near-empty; real corpora = code docs + ALL CHATS).
  - 2026-10-20 — Drew's open concern recorded: not yet clear how vector DBs help long-term vs "just storing a bunch of data on my computer as a backup" (explanation given, conviction pending); coupon fine print asked, later waived by Drew ("don't worry about the coupon please I know", 2026-10-25).
  - 2026-10-21 — "magic environment" architecture sketch commissioned (A1.2).
  - 2026-10-25 — magic-environment v2; temporary-surplus designs (T2); embedding bake-off D1 ruling (A1.3).
  - 2026-10-26 — versioned chunks adopted as starting rule (A1.4).
  - 2026-10-28 — "migrate atlas to laptop off sandbox" (A1.6).
  - 2026-10-31 — publish v2 (A1.7); "how does search work / what's next" → search improvement path (A1.8).
  - 2026-11-03 — "Run the Atlas swap now" (A1.9).
  - 2026-11-04 — "Rust binary vendored from the code flake + multi-client service" (b+c), design approved, "don't set it aside — keep working until all done" (A1.12).
  - 2026-11-05 — Ask-the-ledger approved with a 👍 (A1.14).
  - 2026-11-10 — "no fucking integrate into atlas. ensure all is query able across all." (A1.15).
- domain: Atlas / Turso
- where: Turso side chat (508f0d06) + main chat
- tracking: NEVER-ISSUED as a program (in-slice record: design lived in docs); later pieces carry their own issues (A1.8–A1.15). Owning store: private project hatch-personal (turso-atlas). No single program epic exists.
- design_artifact: ~/hatch-personal/workspace/turso-atlas/docs/pro-month-plan.md (+ laptop-centric / anti-lock-in addendum 2026-10-20 ~22:51)
- design_status: decided
- implementation_status: partial (shipped in pieces — see sub-entries; no single completion record for the program as designed)
- followups_open: ~~final landing-tier confirmation (decide ~day 25 by measured hot-set size + active-DB count) never recorded as answered~~ — ANSWERED (gap pass 2026-11-11): Drew signed up for Turso Scaler on 2026-11-04 (recorded in ~/hatch-personal/workspace/scratch/atlas-scaler-revision/design.md — the design artifact behind A1.4's staging-tier lineage, §4 being the two-tier design #5669 tracks; its Appendix B carries the open questions for Drew as of 2026-11-04). The landing tier is Scaler. The program's individual follow-ups live under the sub-entries.

#### A1.1 Atlas collector (atlas-ingest.py) + full-corpus ingest + recording-coverage directive
- first_asked: 2026-10-20 — canonical archive must cover ALL conversations (Muse, OpenCode, Claude Code, local Codex, GitHub Copilot, Antigravity, Gemini + ChatGPT backups) with a maintained plan and a coverage proof (`atlas-ingest.py report`); collector built "for the Muse test but as a reusable template — config-driven default locations per client/machine"; Framework laptop (4 TB 990 Pro, always-on) = canonical warm store + ingest factory; buckets demoted to backup/distribution.
- tracking: NEVER-ISSUED (built directly in the private project) — triage **(b) DONE-WITHOUT-ISSUE**
- design_artifact: ~/hatch-personal/workspace/turso-atlas/ (scripts/atlas-ingest.py, data/atlas.db, README 5-step pipeline: collect → normalize → index locally → publish hot copy to Turso → agent access via search/get_chunk/add_finding)
- design_status: built-into-running-system
- implementation_status: shipped — adapters muse/chatgpt/claude-export implemented + tested; FULL ingest completed overnight 2026-10-20/21: 6,176 sessions, 23,208 messages, 26,849 unique chunks, atlas.db 530 MB, FTS verified live; atlas.db uploaded to Google Drive Cloud/atlas, byte-size verified (555,929,600 bytes).
- followups_open: opencode/claude-code/codex/copilot/antigravity/gemini adapters only TEMPLATED (their data lives on Drew's laptop/work machines, absent on the sandbox); atlas.db slimming TODO (text stored ~3×) — Drew explicitly does not care until multi-TB; laptop access via Tailscale promised by Drew, pending in-slice.

#### A1.2 Magic-environment architecture — one capture/query front door for all agents (v1 → v2)
- first_asked: 2026-10-21 (v1 sketch, Cloudflare side chat); 2026-10-25 (v2 rewrite, Cloudflare side chat bb341935, after Drew asked where the doc is kept).
- design: Turso as system-of-record (muse-meta coord_* seed: notes/message index, tag/link edges, claim store), Workers + MCP front door, Durable Object state machines (per-source ingestion, task lease/claim with alarm reaper, note refinement raw→tagged→linked→promoted-into-Emanote, open-question registry, per-chat compaction), R2 raw artifacts, basement mass storage as final archive/exit ramp; sized to fit Turso Scaler $29. v2 corrections: canonical store is LOCAL atlas.db (Turso = distribution/hot copy + coordination, never the only copy); classification ≠ embedding; DO state machines rescoped to edge-visible ones; §12 settles D1 export (SQL dump only) and tiered Matryoshka vectors (small in Turso, full-precision in Vectorize/local, D1 as projection only).
- tracking: NEVER-ISSUED (searched "magic environment") — triage **(c) SUPERSEDED/DEAD as a standalone build**: never green-lit as one build; its pieces were absorbed into A1.8/A1.12 (search service), F4.14/#8575 (one-door fleet tool), and the coordination stores. Register-only.
- design_artifact: ~/hatch-personal/workspace/turso-atlas/docs/design-magic-environment.md (v2, committed bd26e03); v1 at ~/workspace/cloudflare/design-magic-environment.md; 7 sibling Cloudflare research files rescued to hatch-personal/workspace/cloudflare/docs/
- design_status: drafted (v2 supersedes v1 where they conflict)
- implementation_status: not started (as a single build)
- followups_open: at v1 delivery Drew had not answered "start build step 1 (capture API into muse-meta) or take the design to the Turso chat first"; Containers line parked by Drew's 2026-10-21 decision ("do NOT need to be fully working right now").

#### A1.3 Embedding bake-off + anti-lock-in embedding rule
- first_asked: 2026-10-20 (rule adopted inside A1: chunk text canonical/immutable, embeddings derived + disposable in per-model tables vec_<model>, hybrid FTS/BM25 + vector, re-embed locally, promote only winners to Turso); 2026-10-25 Drew's D1 ruling: "test ALL bake-off options, including the hosted OpenAI arm (V6) as a priced reference point; local-only is not a principle".
- timeline: 2026-10-25 — Phase 0 harness built (scripts/build_vectors.py + eval_vectors.py + golden-queries.json, 12 → 40 queries; honest FTS baseline MRR 0.448 = gate-G1 bar). 2026-10-31 — golden v3 (311 queries): SNOWFLAKE WINS (768 = 176 hits / MRR 0.362; truncated-384 0.346; truncated-256 0.345 — all beat the live FTS+bge-small hybrid 0.328); Qwen3-Embedding-0.6B pass INCOMPLETE (38,752 / 101,471 chunks when the laptop pass died); Modal build in flight (bf16 equivalence check before full run, then embed the 62,719 missing).
- tracking: NEVER-ISSUED (searched "embedding bake-off atlas") — doc + harness only
- design_artifact: embedding-bakeoff-full-design.md §11 (hatch-personal, committed c123860); results at ~/atlas/v3/ on the laptop
- design_status: decided (D1 ruling + winner measured)
- implementation_status: partial — Snowflake arm complete and later production (A1.9); Qwen arm unscored until the Modal completion lands
- followups_open: golden set 40 → 50–100 (recorded pending 2026-10-26); Qwen scoring; reranker stage wiring gated on the fused-rerank result (see A1.8).

#### A1.4 Versioned chunks rule + staging tier + spike-DB cleanup decision
- first_asked: 2026-10-26 12:51 CDT — Drew adopted versioned chunks as the Atlas starting rule: keep previous versions of changed files, searchable on request; default search = current version only (version stamp per chunk set — git commit + date); versions share unchanged chunks (storage cost = diffs only); Turso hot copy current-only. Reversible.
- tracking: NEVER-ISSUED in-slice (searched "atlas versioned chunks"); later lineage: developing-today/code#6018 / #5669. #5669 **[live-checked 2026-11-11: OPEN]** — "atlas staging tier: versioned schema, registry, aggregator with merge-on-close promotion into three-level core".
- design_artifact: ruling recorded against the Oct 21–25 Atlas design docs (schema dimensions kind/topic/project/sensitivity + provenance/time)
- design_status: decided
- implementation_status: partial (staging-tier build tracked by #5669, still open)
- followups_open: (i) classification label vocabulary sign-off — the later classification pass ran under scheme-1.0 (A1.5), so this gate was overtaken; (ii) 2026-11-06: Drew was asked to approve deleting ~10 abandoned Atlas spike/probe Turso databases (or keep named exceptions) — presented as a decision item, NOT answered in the record; it hangs off the #5669 cleanup context and the decisions ledger, so it is tracked-but-unanswered, not never-issued.

#### A1.5 Corpus build-out — Turso delta publish + full-corpus classification (scheme-1.0)
- first_asked: 2026-10-26 → 2026-10-27 — publish the recovered Muse user turns to Turso; run the full-corpus classification the Oct 21–25 docs specify (kind/topic/project/sensitivity + provenance/time).
- tracking: no issue cited in the record for either piece — triage **(b) DONE-WITHOUT-ISSUE**
- design_artifact: turso-atlas/docs/turso-delta-publish-message-parts-2026-10-26.md (f1eecb4); docs/classification-full-run-2026-10-26.md + data/classification-full-v1.json (c5b28080)
- design_status: built-into-running-system
- implementation_status: shipped — delta publish live 2026-10-26 ~23:09 CDT (atlas = 10,195 sessions / 95,135 messages / 100,698 chunks, all vectored; 6,941 local-only chunks excluded, 0 leaked); classification COMPLETE 2026-10-27 ~01:10 CDT (18,325/18,325 canonical sessions labeled under scheme-1.0, all five gates PASS; sensitivity personal 16,009 · local-only 2,226 · secret-bearing 90).
- followups_open: recorded scheme friction — `shareable` structurally unreachable under the evidence floors; `chat-archive` unused; topic-vocabulary gaps on the fleet-repos ↔ agent-tooling ↔ infra-nix ↔ models-and-plans seam (primary-topic consistency 81.0%).

#### A1.6 Atlas migration — sandbox → laptop
- first_asked: 2026-10-28 06:05 CDT — "migrate atlas to laptop off sandbox": move the canonical Atlas workspace + DB to the laptop, retire the sandbox copy.
- tracking: NEVER-ISSUED (no issue cited) — triage **(b) DONE-WITHOUT-ISSUE, with a residual verification gap**: every later record operates Atlas from the laptop (~/atlas/v3, publish-v2 report, swap scripts), so the move evidently completed in practice; the formal close-out steps below were never recorded as verified.
- design_artifact: none; atlas-backup.py re-pointed (snapshots via sqlite3 .backup on the laptop; manifest source laptop:/home/user/atlas/turso-atlas/data/atlas.db)
- design_status: decided (directive)
- implementation_status: partial at record time — workspace copied; canonical DB (1,490,194,432 B, sha256 bc94bdbb…) transfer truncated once (caught by re-verification, partial deleted), redone as 8×200MB sha-verified chunks; chunk-ac failed when laptop SSH dropped ~06:11 CDT (tailnet outage); canonical intact on sandbox, nothing lost.
- followups_open: chunks ac–ah, reassembly, integrity/count verification, sandbox retirement, patched-backup proof run + atlas-local-backup-daily cron body update — all owed in the record; the backup cannot run natively on the laptop (no bws/gws in laptop ~/tools).

#### A1.7 Atlas publish v2 — full-corpus rebuild for Turso
- first_asked: 2026-10-31 — rebuild the Turso publish artifact under the magic-environment design (local atlas.db canonical, Turso = hot copy) with complete vectors, FTS, session labels, then swap in.
- tracking: NEVER-ISSUED for the v2 build itself (follow-ups commented on #3125) — triage **(b) DONE-WITHOUT-ISSUE**
- design_artifact: laptop report ~/atlas/turso-atlas/docs/turso-full-publish-v2-2026-10-31.md (that laptop tree is not a git checkout, so the report could not be committed from there)
- design_status: built-into-running-system
- implementation_status: shipped — BUILD COMPLETE + independently verified 2026-10-31 10:47 CDT (atlas-publish-v2-2026-10-31.db, 1.59 GiB: 18,325 sessions / 122,124 messages / 107,639 chunks / 107,639 FTS rows; vectors 100% reused, 0 newly embedded; canonical atlas.db re-verified unchanged).
- followups_open: v3 publish must type the vec column F32_BLOB(384) to unlock vector_top_k (v2's plain BLOB forces a ~4s full scan); filtered vector queries slow (15–60s — messages(session_id) index + session_id denormalized onto chunks recommended for v3). Both were addressed by the A1.9 swap (typed 768 table).

#### A1.8 atlas-search CLI + search improvement path
- first_asked: 2026-10-31 11:47 CDT — Drew's "how does search work / what's next"; agreed path: (1) atlas-search CLI; (2) query planner — NL → {filters, residual query text, sort} over closed topic/project vocabularies + deterministic date parsing; (3) retrieve-then-shape — over-retrieve top 100–300, rerank, roll chunks up to sessions, dedupe + MMR, context expansion; (4) multi-resolution indexing (session summaries as searchable units); (5) Snowflake swap (gated on fused-rerank result) + golden-set-gated reranker stage. DB-split answer given: splitting the corpus does NOT speed queries.
- tracking: CLI shipped via hatch-personal PR #9 (merged 7f8955ecb); improvement-path lineage developing-today/code#3125 **[live-checked 2026-11-11: OPEN]** (see A1.13 for the title conflict resolution)
- design_artifact: the CLI (installed on laptop + sandbox); improvement-path design in the 2026-10-31 day-file
- design_status: decided (path agreed)
- implementation_status: partial — CLI SHIPPED 2026-10-31 ~12:32 CDT (verified against BOTH backends with identical rankings; sensitivity defaults proven; search = FTS BM25 + vector fused by RRF, no intent/date/topic understanding yet); the SCHEDULED update loop is NOT built (publish = rebuild-file + destroy/recreate swap, ~6.5 min); incremental SQL publish designed, not built.
- followups_open: query planner, retrieve-then-shape rerank, multi-resolution indexing undesigned beyond the agreed path; local vec stores must join by CONTENT HASH not chunk_id (raw join silently covered only 93,879/107,639); no atlas-ro secret in Bitwarden (CLI mints via `turso db tokens create atlas --read-only`).

#### A1.9 Atlas production swap — Snowflake typed vectors
- first_asked: 2026-11-03 — Drew authorized with a tap ("Run the Atlas swap now"): move production Atlas search to the Snowflake-arctic-embed-m-v2.0 typed-vector table.
- tracking: lineage #3125 (OPEN, live-checked); defects filed as dezren39/hatch-personal#20
- design_artifact: ~/atlas/v3/swap-2026-11-03/ (swap scripts + runbook records)
- design_status: built-into-running-system
- implementation_status: shipped — swap COMPLETED 2026-11-03 ~23:42 CST: typed F32_BLOB(768) table vec_snowflake_m_v2_768_f32, 107,639 vectors, CLI defaults switched (hatch-personal PRs #17/#18/#19).
- followups_open: hatch-personal#20 — Turso writer-stall root cause (CREATE INDEX/REINDEX on a populated table; resolved on its own) and the vector-index-built-but-not-searchable remediation still owed; that remediation gates A1.11.

#### A1.10 Atlas canonical-session rebuild pass
- first_asked: 2026-11-01 — Drew approved (👍) rebuilding the Atlas session corpus canonical-only (reconstructed extracts of canonical sessions only).
- tracking: NEVER-ISSUED (searched "atlas rebuild reconstructed") — triage **(b) DONE-WITHOUT-ISSUE**
- design_artifact: none found
- design_status: built-into-running-system
- implementation_status: shipped — rebuild completed and verified in-chat (row counts and dedupe checked against the live Turso DB).
- followups_open: none recorded.

#### A1.11 Atlas search-improvement exploration + A/B
- first_asked: 2026-11-04 — "yes do that now": run the three-arm A/B (exact scan / index fetch-to-fill / index fixed-100-then-filter) and explore parallel branch execution, workers + temp result store + pull-until-satisfied service shape, nix-packaged binary.
- tracking: dezren39/hatch-personal#21 (exploration tracker; state not re-verified in this pass)
- design_artifact: ~/atlas/v3/swap-2026-11-03/run_ab.sh (A/B harness)
- design_status: drafted
- implementation_status: partial — A/B dispatched; index arms blocked at record time (vector index not searchable, probe errors "index not found"); the service-shape exploration fed directly into A1.12.
- followups_open: A/B completion depends on the searchable-index remediation in hatch-personal#20.

#### A1.12 Atlas as a Rust binary + multi-client search service (b+c)
- first_asked: 2026-11-04 — Drew chose options (b) AND (c) ("Rust binary vendored from the code flake + multi-client service"), approved the design and Q1/Q2/Q3 as recommended ~02:31 CST, then ~02:33 CST: "don't set it aside — keep working until all done".
- tracking: developing-today/code#5350 **[live-checked 2026-11-11: OPEN]** — "Epic: Atlas as a Rust binary + multi-client search service (b+c)"; steps #5351 (embedder parity, CLOSED — gate A passed bit-identity, PR #5367 merged), #5352 core lib, #5353 CLI mode, #5354 serve mode + temp store + protocol, #5355 nix packaging + deploy, #5356 golden-parity gate + cutover.
- design_artifact: doc/2026-11-04T08-07-13Z_design_atlas_binary_service/ in developing-today/code, merged via PR #5348 (9c5976340). Design decisions: ONE binary two modes; temp store in-memory TTL 10 min with per-job capability token (scratch Turso rejected, citing the writer stall); clients page an immutable fused ranking; backpressure ≤4 concurrent branch executions, 429+Retry-After; tailnet serve + per-client bearer tokens each carrying a MAX sensitivity ceiling; service holds READ-ONLY Turso token only; port 18760 proposed pending #5355.
- design_status: decided (approved by Drew)
- implementation_status: partial (step 1 done at record time; epic still open)
- followups_open: steps #5352–#5356; sensitivity-ceiling rule (only Drew's own CLI above `personal` in v1) and sandbox CLI defaulting to --via-service at cutover, per Drew's Q1–Q3 approvals.

#### A1.13 Atlas incremental publish / provenance registry (#3125) — CONFLICT RESOLVED
- first_asked: carry-forward of Drew's Atlas search-design answers (2026-10-31 → 2026-11-01): incremental SQL publish + provenance.
- tracking: developing-today/code#3125 **[live-checked 2026-11-11: OPEN]** — verified title: "atlas: provenance registry (chunk occurrences), content-addressed extracts, multi-DB vector resolution".
- **Conflict resolution (synthesis):** slice 5's record described #3125 as "incremental SQL publish, designed not built" and flagged the mismatch with the verified title. Slice 6 resolves it: the incremental-publish design was *recorded in* #3125 (body/comments) and CLI follow-up fixes were commented onto it — one issue carrying both the provenance-registry scope (its title) and the incremental-publish design (its contents). Both slice statements are true of different layers; the register treats them as one entry.
- design_status: drafted (incremental publish designed)
- implementation_status: not started — the record states the incremental publish / scheduled update loop was designed in #3125 but not built.
- followups_open: build the incremental publish loop; multi-resolution indexing and query planner remain design-only (A1.8).

#### A1.14 Ask-the-ledger — Atlas × Cloudflare AI Search
- first_asked: 2026-11-05 — Drew asked whether the idea is "basically Atlas", whether to implement it for Atlas and what AutoRAG costs; suggested building as-is but tying in Atlas with config to limit after free; approved the build with a 👍 ~09:33 CST.
- tracking: developing-today/code#6266 **[live-checked 2026-11-11: OPEN]** — "atlas: Ask-the-ledger — AI Search edge Q&A surface over non-secret snapshots (child of #6018)"
- design_artifact: results on #6266 (comment 6662862437); CLI at ~/tools/bin/ask-ledger
- design_status: built-into-running-system
- implementation_status: shipped same day — AI Search instance `ask-the-ledger` over R2 bucket `atlas-ledger-snapshots`, 10 allowlisted operational-corpus snapshots, retrieval-only (AI Gateway never used); new Turso DB `atlas-ask-ledger`; free caps in config (1k semantic + 1k full-text/mo, 5M ingestion tokens, 10 GB) with verified fallback to atlas-search BM25 at cap; secret tier structurally excluded and bucket-audited.
- followups_open: issue remains OPEN; snapshot refresh cadence not settled in the record.

#### A1.15 Fleet/agent memory integrates into Atlas (#1300 ruling)
- first_asked: 2026-11-10 — Drew rejected the recorded split (per-agent memory local, shared recall in Atlas): "what the fuck do you think atlas is supposed to be" … "no fucking integrate into atlas. ensure all is query able across all."
- tracking: developing-today/code#1300 **[live-checked 2026-11-11: CLOSED]** — "Roadmap: vector memory for OpenClaw (memory-lancedb now, fleet vector store later?)"; design record under #8567 (pkgs/atlas-search/docs/memory-integration-design-8567.md, per records pass); memory-integration slices in the records census: #8622 (slice 1 corpus registry + memory-lancedb connector), #8625 (slice 3 MCP query surface). Ledger item fleet-vector-store-1300 marked answered with this ruling.
- design_artifact: pkgs/atlas-search/docs/memory-integration-design-8567.md; ledger item fleet-vector-store-1300
- design_status: decided
- implementation_status: partial / not established in the slice record (slices #8622/#8625 exist per census; their states were not verified in this pass)
- followups_open: the integration build itself; also on the ledger at slice end: developing-today/code#8479 **[live-checked 2026-11-11: OPEN]** — "Atlas boundary remediation: secret-bearing rows still live in the production monolith and a local replica (#6021 wave 1)" — purge or reclassify 638 secret-bearing rows; ledger item atlas-secret-remediation-8479 still pending Drew.

#### A1.16 Modal GPU credit allocation design (sub-design in the A1.3 embedding lineage) — added by gap pass 2026-11-11
- first_asked: 2026-11-04 ~23:27 CST — Drew's directive: design + research ONLY (nothing spent or provisioned) on the best use of the Modal balance ($30/mo Starter credits + $100 one-time hackathon grant = $130; grant treated as spend-by ~Oct 2027).
- design: recurring $30/mo funds the Atlas delta-embedding + experiment rhythm; the one-time $100 funds one-shot corpus builds; a hard classification guardrail on what may be embedded off-box; a post-credit "GPU ladder" answering Drew's four questions about life after the credits. Dimension-identical to production (arctic-embed-m-v2.0, 768) — no migration.
- tracking: doc-only; no issue cites this design (the Modal Qwen/embed completion rides A1.3's follow-ups)
- design_artifact: ~/hatch-personal/workspace/scratch/modal-gpu-design-2026-11-04/design.md
- design_status: decided (allocation recommended in the doc)
- implementation_status: partial — the Modal embedding build it budgets was in flight per A1.3; the allocation/ledger itself has no completion record
- gap note: the register's only Modal mentions before this entry were the bake-off's Modal build lines (A1.3); this commissioned design had no entry.

### A2. agent-fs + llm-wiki investigation (Atlas pre-history)
- first_asked: 2026-09-29 — "what is agent-fs what is llm-wiki" + three reminders (Thu/Fri/Sat) to look into both further: agent-fs (shared, persistent, searchable filesystem for AI agents — MCP-native, S3-compatible) and llm-wiki (Karpathy's pattern: an LLM incrementally builds/maintains an interlinked markdown knowledge base).
- timeline: 2026-10-01/02 — reminders delivered; 2026-10-02 slice record: standing interest, no design or build; Drew's extra "remind me once more next Friday morning" (Fri 2026-10-09) never scheduled — the scope question (both topics or just the lab cluster) was asked and never answered.
- tracking: NEVER-ISSUED (searched "agent-fs llm-wiki" — no results); lived only as goal `look-into-agent-fs-and-llm-wiki-further` (goal_4ff7896f6699) + three runonce reminder crons — triage **(c) SUPERSEDED/DEAD as a standalone item**: the searchable-agent-knowledge direction was absorbed into the Atlas program (A1) three weeks later; no standalone deep-dive was ever recorded as completed.
- design_artifact: none (definitions/research summary in ~/memory/2026-09-29.md only)
- design_status: none (definition-level research)
- implementation_status: not started
- followups_open: none as a standalone item; its substance lives in A1.

---

## 2. ID & FLEET PLATFORM

### F1. ID platform program — the ID engine and its design mill (PARENT)
- first_asked: standing program across the window (worlds/ID engine predates the window's design burst; the in-window Drew asks are the sub-entries).
- timeline of Drew's asks:
  - 2026-10-29 — six pending design decisions laid out at Drew's request with options + recommendations (F1.2).
  - 2026-11-05 — Drew approved ALL seven Phase-14 next-horizons themes on code#1240 (F1.1).
  - 2026-11-10 — conversion-engine permanent schema/allocator decisions presented (F1.3) — still pending at window end.
- tracking: program epics in the records census include developing-today/code#2056 "epic: ID platform expansion — apps, comms, currency, hosting, boards (2026-10-28 directive, LIST 6–15)" and #2055 (fleet expansion & placement, cross-ref F4); design children by the hundred in the doc/ corpus (F1.4).
- design_status: built-into-running-system (the design mill itself is the running system: designs filed as doc/ records + issues)
- implementation_status: partial (per-issue; the ladder is the work)
- followups_open: the pending Drew rulings in the decisions ledger that belong to this program (records pass §3): replica-operator-806 (A/B/C policy), lifecycle-verdict-2311, design-adoption-1678 (opencode#1678 / PR #2032), watcher-scope-1441 (openchamber#1441), shutdown-delete-1414 — all ledger-tracked, awaiting Drew, none never-issued.

#### F1.1 ID Phase 14 — hosted, federated, local-first
- first_asked: 2026-11-05 — Drew's comment on code#1240 (16:06Z) approving ALL seven next-horizons themes (hosted worlds, discovery/federation, program ecosystem, more shells, agents-in-worlds, scale hardening, creator tooling), first items hosted worlds + discovery, agents-in-worlds as the differentiating spike. Drew's principles: "collaborative / shared / federated / local-first / p2p. we can be the backstop." — local-first always the goal AND a hosted version intended; anything built must be completely usable by others hosting small or large servers themselves; all comms through iroh on the backend, first-class clients.
- tracking: developing-today/code#6312 **[live-checked 2026-11-11: CLOSED]** — "id: Phase 14 — hosted, federated, local-first (plan of record)"; parent discussion #1240 **[live-checked: OPEN]**; 7 core topics #6276–#6282; 28 ladder issues #6283–#6311; roadmap PR #6313 left open for normal CI at record time.
- design_status: decided (plan of record, filed same day)
- implementation_status: partial — implementation is the ladder issues' own work.
- followups_open: the 28 ladder issues.

#### F1.2 Six design decisions laid out for Drew's rulings (2026-10-29)
- first_asked: 2026-10-29 — at Drew's request ("lay them out here briefly, options + recommendation"), six pending design decisions surfaced with substance: (1) opencode#587 adopt ONE shared QuotaSnapshot contract (REC: adopt); (2) code#157 fork transfer to a developing-today-forks org vs keep as declared exceptions (REC: keep); (3) code#160 account identity event — dedicated signed event (A) vs derive from earliest log (REC: A); (4) code#162 judge authority at the causal past (REC: 1); (5) code#163 group-action wire codec — new `group.event` family (REC: A); (6) code#812 public directory feed + presence (lean: opt-in members only, minimal fields, coarse presence).
- tracking: opencode#587 **[live-checked 2026-11-11: OPEN]** — "design(core): one quota signal, three consumers — shared QuotaSnapshot and weight-layer contract for #566/#567/#568"; code#157/#160/#162/#163 verified CLOSED by slice 5; code#812 **[live-checked: CLOSED]**.
- design_status: decided (five of six have since closed — rulings landed after the slice; all six were UNANSWERED at the 2026-10-29 flush)
- implementation_status: partial
- followups_open: opencode#587 remains open; per the record, opencode#566 is blocked until it is ruled.
- origin (corrected by gap pass 2026-11-11): item (2)'s question is older than the 2026-10-29 layout — on 2026-10-12 Drew, in a direct OpenCode session (opencode:ses_ed12b0eaaffe), asked for the forks-org question to be filed; that ask is the origin of code#157.

#### F1.3 ID conversion engine — permanent schema/allocator decisions (#5536)
- first_asked: presented 2026-11-10 as the assemble-first decision item (design-mill / #5536 lineage); the record shows the question pending, not Drew's answer.
- tracking: developing-today/code#5536 **[live-checked 2026-11-11: OPEN]** — "id: implement ID conversion engine - linked transfer pairs and rate authorisation (design #2063)"; decisions-ledger item id-engine-5536.
- design_artifact: design #2063 (doc/2026-11-01_design_id2063.md in the doc/ corpus)
- design_status: drafted (design #2063)
- implementation_status: not started (gated on the permanent decisions)
- followups_open: Drew's rulings on the permanent schema/allocator choices.

#### F1.4 The code repo doc/ design corpus (records-pass reconciliation)
- What it is: the repo's doc/ tree holds 392 entries total, 335 dated inside the window; by kind: design 197, feature 42, decision 24, investigation 23, research 9, phase 9, architecture 6, other ~25. The 236 design-kind entries (listed in full in records-pass.md's appendix) are overwhelmingly the ID platform design mill (design_idNNNN series, Oct 29–Nov 11, filename = its issue number) plus Atlas designs (atlas_* series, Nov 4–11) and early ID "worlds" architecture (Oct 7–19).
- tracking: per-entry issue state was NOT checked in the records pass (335 lookups out of scope); the design_idNNNN filename convention implies an issue. This register does not fabricate per-entry states.
- design_status: built-into-running-system (the mill)
- implementation_status: per-issue, unverified in bulk — stated plainly as a coverage limit of this register (see header).
- Named non-doc/ design docs reconciled from the records pass: doc/2026-10-24T11-05-38Z_design_project-management-stack/ (PM stack canonical design record, see P1); pkgs/atlas-search/docs/memory-integration-design-8567.md (see A1.15); docs/knowledge-layer-attachments-media-5844.md (#5844); docs/knowledge-layer-feature-matrix-6435.md (#6435); docs/spend-rollups-design-1259.md (#1259); docs/pm-stack/current-state-ownership-map-2026-11-05.md; pkgs/mcpx/docs/proposals.md (see F2).

### F2. mcpx platform program (PARENT)
- first_asked: 2026-09-30 — Drew's standing program: import mcpx from dezren39/nix to parity with his lootbin/lootbox MCP config, run opencode with the mcpx plugin, mcpx daemon as a systemd service restored by init, push fixes/issues/PRs back upstream.
- tracking: goal `nix-real-store-mcpx-opencode-setup` (goal_e877a932b054) + its 15-min check cron (later absorbed into the routine-task redesign, F4.2)
- design_status: built-into-running-system
- implementation_status: partial → shipped in phases (sub-entries)

#### F2.1 mcpx + opencode platform program — lootbox parity, daemon as a service
- first_asked: 2026-09-30 — "Yes, lootbox — mcpx with the same server list." (codebase-memory, codedb, fff, fff-nix, fff-worktree, chrome-devtools, context7)
- tracking: work tracked upstream in dezren39/nix (now darwin-nix), not in developing-today/code: PRs #194/#195/#197/#221 merged in-slice; issues #235 and #259 verified CLOSED in slice 2, #260, #291 (closed by PR #301), #304 (closed 2026-10-01 by upstream #306).
- design_artifact: ~/notes/architecture/mcpx-opencode-plan.md (phase-1 plan, findings incorporated 2026-09-30); demo script in the goal's files/
- design_status: built-into-running-system
- implementation_status: shipped — phase 2 complete 2026-09-30 07:16 CDT: mcpx 0.1.0 built and installed, daemon active as a systemd service (127.0.0.1:41001 + unix socket), opencode 1.18.31 + mcpx plugin recognized, 7/7 servers warmed at lootbox parity, end-to-end `fff_nix.grep` calls live through 2026-10-01.
- followups_open: gated on Drew at slice end — interactive `opencode auth login` (browser OAuth) and a Context7 API key (see F2.6).

#### F2.2 mcpx dedup — one canonical mcpx source in developing-today/code
- first_asked: 2026-10-19 — recorded directive: "remove the duplicate mcpx copy from dezren39/nix; first determine everything the nix copy implements that developing-today/code lacks and migrate each thing; file a GitHub issue BEFORE each thing; merge PRs whenever ready." (Drew had asked whether there were "two full source apps in 2 places?" — confirmed yes.)
- tracking: NEVER-ISSUED as one tracking issue (searched "mcpx dedup single source") — by the directive's own design, per-item issues were the tracking mechanism — triage **(b) DONE-WITHOUT-ISSUE**
- design_status: built-into-running-system
- implementation_status: shipped — coordinator ran the audit/migration the same day; same-day follow-up records "mcpx dedup — vendored copy in dezren39/nix removed (Drew's directive, ~11:51 CDT)", with nix packaging consuming mcpx via a pinned flake input on the code repo.
- followups_open: Drew had not yet said whether he also wants a submodule at the old mcpx path for local editing on top of the flake-input rewiring.

#### F2.3 mcpx scope-resolved roots design (decision record 0004)
- first_asked: one of Drew's four pickups ordered 2026-10-26 ~06:23 CDT; the design was reported COMPLETE to him — spec PR developing-today/code#957 (decision record 0004) consolidates dezren39/nix#817/#818/#819: a ResolvedScope sibling to Scope.Key plumbed Registry → Pool.Acquire → Instance → dial(), rootUri/workspaceFolders at initialize, placeholder validation, vanished root fails the spawn loudly; one shared codedb at global scope stays; index root is per-machine config.
- tracking: PR #957 MERGED (verified in slice 5); implementation issues #958 (slice 1 plumbing + spawn cwd), #960 (slice 2 substitution), #959 (slice 3 initialize workspace), #961 (slice 4 gopls worktree-scope flip + live proof)
- design_artifact: decision record 0004 in PR #957
- design_status: decided
- implementation_status: not started at record time (no slice implemented; slice 1 is a cross-package signature change)
- followups_open: slices #958–#961 implementation; nix trio #817–#819 stay open for the #309 re-homing pass; laptop/sandbox mcpx version skew noted (gopls 0.23.0 / codebase-memory-mcp 0.11.0 declarative vs 0.24.0 / 0.12.1 scripted).

#### F2.4 mcpx consolidation — modules originate with mcpx in code
- first_asked: 2026-11-03 — Drew's directions: (1) move nix's checkPhase adaptations INTO code's pkgs/mcpx/package.nix; (2) layering — daemon modules ORIGINATE with mcpx in code → opencode RE-EXPORTS alongside its own modules → openchamber passes both through → nix consumes via the single openchamber input; (3) dezren39/nix collapses to JUST the openchamber input. Also Drew: "why not code Gen as part of the nix build".
- tracking: developing-today/code#4798 CLOSED (step 1, PR #4799 merged); code#4801 CLOSED (module origin); developing-today-forks/opencode#1502; dezren39/nix#1384 (step 3 pending at record time); codegen-in-build PR #4807 OPEN at record time; pre-existing defect filed as code#4806.
- design_status: decided
- implementation_status: partial — steps 1–2 done at record time, step 3 (nix collapse) pending.
- followups_open: dezren39/nix#1384 completion; PR #4807.

#### F2.5 mcpx GitHub App-token failover proposal
- first_asked: filed 2026-11-08 ~22:07 CST as a proposal for Drew's next-up consideration (Drew-approval status NOT established in the record) — mcpx github failover across a list of GitHub App identities on soft blocks (429, 403 rate-limit markers, 502/503), stop on hard blocks (401, unmarked 403, 404, 422).
- tracking: developing-today/code#7948 CLOSED — "mcpx github: fail over across a list of GitHub App identities on soft blocks (rate limits), stop on hard blocks"
- design_status: drafted (proposal)
- implementation_status: not established
- followups_open: a discrepancy was flagged at filing — #7948's body describes a live mcpx github namespace that the live runtime denies exists (outbox OB-2026-11-09-302); an implementer must first establish the namespace state; whether Drew approved the proposal is not established.

#### F2.6 MCP server parity expansion — GitHub / Atlassian / Datadog logins for the mcpx set
- first_asked: recorded 2026-10-02 — upstream dezren39/nix#348 merged GitHub + Atlassian MCP servers into the lootbox set; the fleet-side decision was to NOT add them here because each needs Drew's own login (GitHub PAT, Atlassian browser OAuth, Datadog from #347) — "won't add knowingly-failing servers".
- tracking: NEVER-ISSUED fleet-side (no issue number cited anywhere in the record) — triage **(a) STILL-LIVE**: the three logins (opencode auth, Context7 API key, Datadog/GitHub/Atlassian OAuth) remained the standing mcpx-program gates at every watch run through 2026-10-11 and nothing tracks them as an ask.
- design_status: none
- implementation_status: not started (gated on Drew's physical logins)
- followups_open: the logins themselves — a Drew-only act; filed in the manifest below so the gate stops living only in watch-run text.

### F3. OpenCode platform (PARENT)

#### F3.1 OpenCode 2 fleet migration directive
- first_asked: 2026-10-24 — "my fleet should be exclusively using opencode2. please immediately fix anywhere that isn't" — migrate every fleet surface (sandbox server, laptop standalone, repo configs, plugins) from OpenCode v1 to v2.
- tracking: NEVER-ISSUED (searched "opencode 2 migration fleet" and "v2 migration" in the opencode fork — the record says an origin tracking issue was part of coordinator 3's launch, but no such issue is findable) — triage **(b) DONE-WITHOUT-ISSUE for the migration core** (the fleet demonstrably runs OpenCode 2.x everywhere in the November records), **with one (a) STILL-LIVE follow-up** (plugin ports + fork rebase, below).
- design_artifact: none found; recon facts (three breaking changes: plugin API, /api/* contracts, tui.json→cli.json) in the 2026-10-24 day-file
- design_status: decided (directive)
- implementation_status: partial — three coordinators launched the same night (sandbox server → 2.1.4; laptop standalone → 2.1.4 with OpenChamber's embedded v1 as a named exception; repo workstreams).
- followups_open: **v1 plugin ports (mcpx bridge, mcpx-session, subtask2) and the fork rebase were explicitly left unstarted in the record** — no later slice records them done → STILL-LIVE follow-up, in the manifest.

#### F3.2 One shared OpenCode serve — consolidation, ACP↔HTTP adapter, T3 Code attach
- first_asked: 2026-10-26 evening → 2026-10-27 — "Build any glue or tools needed; the goal is ONE opencode server that everything attaches to (OpenChamber, T3 Code, the ACP adapter) — explicitly NOT side-by-side servers." Prioritized by Drew: IPv4/build → sops → shared serve → adapter → T3 attach → memory-lancedb.
- tracking: developing-today/code#1243 CLOSED (verified in slice 5) — "OpenClaw: share one opencode serve via a small ACP-to-HTTP adapter"
- design_artifact: design + acceptance in #1243 (adapter: session/new → POST /session, prompt → message/prompt_async + /event SSE, permissions round-trip, cancel → abort); T3 recon finding: attach is built-in config, no fork needed.
- design_status: decided
- implementation_status: partial at slice record — shared-serve password pre-staged (bws opencode.server-password + 1Password), serve itself not yet stood up at the Oct 27 flush; the November fleet record runs on the single laptop server (see F3.3/F3.4), so the consolidation landed in practice.
- followups_open: memory-lancedb for OpenClaw approved, with roadmap code#1300 tracking the store question — resolved by Drew's Atlas-integration ruling (A1.15); phase 1 needs a small fixed-port embedding service (no stable embedding endpoint on amd).

#### F3.3 SSH-on-laptop vs attach-to-laptop-server benchmark + attach-first decision
- first_asked: 2026-10-27 — Drew: "make it a priority" (02:18 CDT): benchmark driving the laptop OpenCode server over SSH vs attaching to it, on identical scripted workloads, and decide.
- tracking: developing-today/code#1344 CLOSED (verified in slice 5)
- design_artifact: hub goal files/opencode-access-benchmark-2026-10-27.md (247 lines, committed) + decision record on #1344
- design_status: decided — DECISION (split): programmatic work → ATTACH (persistent forward + raw API + run --server); SSH → host admin/bootstrap/recovery; OpenChamber → human interactive, unchanged. Decisive finding: killing the transport mid-task in BOTH patterns left the turn to finish server-side.
- implementation_status: shipped (decision complete same morning ~02:49 CDT)
- followups_open: filed follow-ons — #1357 attach-first client helper, #1359 `opencode run` hangs on open non-tty stdin (bug), #1360 session create ignores ?directory= (bug), #1362 build-rev alignment, #1363 evaluate tailnet-bound serve.

#### F3.4 OpenCode-half redesign — at least half of queue work executed via OpenCode sessions
- first_asked: 2026-10-28 06:05 CDT — "redesign so at least half the tasks will use opencode": a deterministic claim-time split so ≥50% of queue tasks are executed by OpenCode sessions on the laptop server instead of direct Muse workers.
- tracking: design posted on developing-today/code#1425 (verified CLOSED; its title is the config-drift issue the design rode on); implementation #1654 (ledger + lane verb) CLOSED, #1655 (shift executor mode) CLOSED, #1656 (keeper binding-aware supervision) **[live-checked: OPEN]**, #1657 (daily lane-share measurement) **[live-checked: OPEN]**, #1658 (pilot rollout + extension) **[live-checked: OPEN]**.
- design_artifact: ~/workspace/goals/keep-the-four-repos-healthy/files/opencode-half-redesign-2026-10-28.md — claim-time assignment ledger + `repo-queue.py lane` verb, session-per-task on the laptop server, honest fallback reasons recorded as misses, daily lane-share tally + alert if rolling 7-day executed share <50% two days running.
- design_status: decided
- implementation_status: partial — design + implementation issues filed same morning; pilot on the code queue (live volume ~125 queued, ~105 terminal events/24h).
- followups_open: prerequisite step 0 — code#1357 attach-first helper still unbuilt at design time; #1656–#1658 remain open.

#### F3.5 OpenClaw route-only ruling — all OpenClaw traffic through the opencode-route door
- first_asked: 2026-10-31 08:22 CDT — Drew's ruling: OpenClaw must use ONLY the opencode-route provider door the fleet built (code#2229) — pass-through only; underlying pools reached exclusively through OpenCode routes. Supersedes the #3061 re-login plan (no Command Code re-auth).
- tracking: developing-today/code#2229 CLOSED (verified in slice 5; the opencode-route door build); correction commented on #3061 (comment 6543961097)
- design_artifact: rendered config source developing-today/code home/openclaw/default.nix
- design_status: decided
- implementation_status: partial — fix dispatched in-slice (branch fix/openclaw-route-only: remove commandcode/design-code/agy/codex-proxy providers, defaults → opencode-route/workhorse + specialist/speed fallbacks); live read showed agents.defaults.model still pointed at the direct commandcode chain — the expired login that killed the ACT-7 runs.
- followups_open: deploy (switch + gateway restart + live run + ACT-7 re-trigger) owed by the parent at record time.

#### F3.6 "Run my agent from your laptop in one command" — local CLI over the MCP servers
- first_asked: 2026-10-09 — Drew accepted the Idea card (idea_a041d85fa776456f802c5a356d6d58bd): a local CLI prototype on his laptop that runs his agent / reaches the MCP tool servers in one command.
- tracking: NEVER-ISSUED (searched "run my agent laptop CLI") — triage **(a) STILL-LIVE**: the activation check FAILED its premise and the setup questions were re-sent; no answer is recorded and nothing tracks the ask.
- design_artifact: activation contract on the Idea card ([needs_user]: laptop OS, shell command vs local HTTP endpoint vs both, auth approach agreed before any credentials are created) + reachability report at workspace/.jarvis/idea-executions/00de5750-7a91-4721-8e4b-6c9ce3a642e1/scratch/tailscale-reachability-check.md
- design_status: drafted (activation contract)
- implementation_status: not started — premise check: the 7 lootbox MCP servers are stdio children of the gateway on Drew's Mac, bound to 127.0.0.1 only: 0 of 7 reachable over Tailscale; all 7 do run locally via this VM's mcpx daemon. The CLI would have to talk to this VM's mcpx instead, or the Mac gateway would need its loopback binding changed.
- followups_open: Drew's answers (laptop OS, call method, auth preference) — in the manifest.

#### F3.7 *-direct providers emulation doctrine — "emulate, don't embed" — added by gap pass 2026-11-11
- first_asked: 2026-11-05 01:10 CST (vibe-direct embedding idea) → **corrected by Drew 2026-11-05 01:13 CST, verbatim: "no we emulate and don't actually need vibe at all except maybe for certain login calls. same for all direct."**
- design: every `*-direct` provider reproduces the first-party client's wire behavior in our own code (endpoints, request shapes, headers, attribution markers, model presets); vendor binaries are for login/bootstrap only; auth must auto-renew without the vendor binary or a browser. The embedding set (opencode#1709, children #1710–#1714) was closed as not-planned under this ruling.
- domain: ID & fleet / OpenCode platform (provider architecture)
- tracking: developing-today-forks/opencode#1717 **[live-checked 2026-11-11: OPEN]** — "Doctrine: *-direct providers EMULATE the first-party wire — vendor binaries for login/bootstrap only, auth must auto-renew" (created 2026-11-05T07:36Z)
- design_artifact: ~/hatch-personal/workspace/scratch/vibe-embed-2026-11-05/design.md
- design_status: decided (standing doctrine)
- implementation_status: partial / doctrinal — applies to each *-direct provider as built; no completion record
- gap note: before this entry the register mentioned this doctrine only as a parenthetical inside F4.16 (the 2026-11-05 auto-renewal entry) — it had no entry of its own and #1717 was cited nowhere in the register.

#### F3.8 OpenChamber retry-recovery controls — restart-with-new-model + force-next-pool-option — added by gap pass 2026-11-11
- first_asked: 2026-10-19 — Drew, in a direct OpenCode session (opencode:ses_eabfb2c22ffem): "Please make a new feature in OpenChamber and fully implement it so that when an error occurs (such as the plan limit or other errors that stop the progression but trigger retries)" there is a button to restart with a new model (model picker) and, when using route pools, a control that forces the next route-pool option — available at all times, configurable whether it force-stops or applies on the next message.
- tracking: implemented as OpenChamber PR #385 (prompt retry recovery); review issue developing-today-forks/openchamber#388 **[via gh search 2026-11-11: CLOSED]** — "PR #385 (prompt retry recovery) needs detailed review — settings restructure + sessionKnowledge API conflicts"
- design_artifact: none — the ask lives in the session; the build is PR #385
- design_status: decided (directive: "fully implement it")
- implementation_status: shipped (PR #385; review issue closed)
- gap note: no register entry before this one — the register's OpenChamber coverage was the fork/platform program level; this feature ask + build was absent.

#### F3.9 OpenChamber Android app parity — do desktop updates reach the app, or a custom fork app? — added by gap pass 2026-11-11
- first_asked: 2026-10-12 — Drew, in a direct OpenCode session (opencode:ses_ecfd4b73bffeo): "Review how the Open Chamber Android app works. Do updates to the desktop app reflect in the Android app or do we need a custom Android app? If so please implement all changes in the desktop and the mobile app and build an APK in our fork." (Followed 2026-10-17 by ses_eb649134affe: "fix mobile openchamber to have all the ui features and fix the mobile updater to use our fork".)
- tracking: no single issue for the parity question; the fork's mobile app is real and tracked in-repo (openchamber#173 android-device launch break, #390 phantom Gradle flavors, #1341 mobile-release CI — all CLOSED per gh search 2026-11-11)
- design_artifact: none
- design_status: none (question + conditional build order)
- implementation_status: partial — the fork carries the Android app and mobile fixes landed through October; no record ties them back to this ask as its acceptance check
- gap note: the register contained no Android/mobile entry at all before this one (its only "mobile" hit was the PM stack's one-URL yardstick).

### F4. Fleet orchestration & dispatch (PARENT)

#### F4.1 Fleet sweep/review job family (rotation, restructure, merge authority, PR review, reporting split, audits)
- first_asked: 2026-10-17 — "make a scheduled task to check and fix one issue in developing today code or developing today opencode or dt openchamber or dezren39/nix each execution... make 4 tasks that handle this and each runs every 2 hours, divided evenly 30 minutes apart... Think thoroughly and act autonomously... always do something."
- timeline of Drew's asks:
  - 2026-10-17 — four-repo issue-sweep rotation ordered; same day, the standing merge rule: "go ahead and merge if a pr seems good you can always fix it later if it's not." (replaced "Drew reviews / never merge own PRs" for the sweeps).
  - 2026-10-18 — PR diff review job: "also regularly review pr commits and diffs open or closed and make issues from useful findings there."
  - 2026-10-19 — fleet restructure (recorded directive ~21:38 CDT): three per-repo sweeps removed, ONE `issue-sweep-rotating` job every 40m picking a repo at random; new-issue-creator 15m→20m, pr-diff-review 30m→20m, boot-init-todo-check 20m→30m. Same day: side-chat reporting split + escalation rule approved ("escalate secrets, broken main, and blocked-on-me items to the main chat") — ESCALATION paragraph appended to all six job bodies. Same evening: job cadence/duration + billing audit; 2026-10-20 06:03 CDT: fleet usefulness audit (issue ledger Oct 17–19: opened 129, closed 161, net −32).
- tracking: NEVER-ISSUED as a family (searched "issue sweep rotation" — only Nov-era conversion tasks code#2156/#2138 exist, not the original asks) — triage **(b) DONE-WITHOUT-ISSUE**: implemented directly as cron jobs under goal keep-the-four-repos-healthy (goal_a243772844c3); the merge rule was applied via cron.update + GOAL.md + MEMORY.md and exercised the same morning (opencode PR #22, openchamber PR #37, dezren39/nix PR #379 merged under it).
- design_artifact: goal GOAL.md + per-repo backlogs in the goal's hidden_files/; ESCALATION paragraph in the six job bodies under the goal's crons/
- design_status: built-into-running-system
- implementation_status: shipped (and repeatedly restructured in place)
- followups_open: GOAL.md did not yet reflect the 2026-10-19 restructure at record time; stale "mcpx lives in BOTH code and nix" wording pending rewrite in the code-sweep, new-issue-creator, and pr-diff-review bodies (that wording was overtaken by F2.2/F2.4).

#### F4.2 Routine-task architecture redesign — one merged job, time-based sections, per-section todo allowlists
- first_asked: directives 2026-10-03 → 2026-10-04; simplification pass 2026-10-05 — Drew directed merging sandbox-boot-init (5m) + nix-mcpx-15min-check (15m) into a single job, then ruled: time-based per-section scheduling instead of a run counter ("a counter breaks if the job interval changes"); per-section todo_sources allowlists; mcpx rebuilds ONLY on a new upstream tag. 2026-10-05: "yes all these look good. apply 1 to 4" (timeout 900s→1800s, tiered upstream check, codified push pattern, less log noise; sweep cadence dynamic 30/90/120m).
- timeline: 2026-10-07 — related cadence directive: "cut the scheduled jobs to run half as often" — applied the same day (boot-init-todo-check 10m→20m, airthings-alerts 6h→12h, heartbeat 30m→1h).
- tracking: NEVER-ISSUED (searched "boot-init todo-check") — triage **(b) DONE-WITHOUT-ISSUE**
- design_artifact: cron job `boot-init-todo-check` + state file ~/workspace/goals/nix-real-store-mcpx-opencode-setup/hidden_files/schedule-state.json (sections boot-init / todo-sweep / deal-hunt with interval_minutes + todo_sources) and append-only events.log
- design_status: built-into-running-system
- implementation_status: shipped — verified working from 2026-10-04 00:24 onward; simplifications applied 2026-10-05 11:45–11:50 with cron.view read-back.
- followups_open: none recorded.

#### F4.3 Pool & routing intelligence epic — lane scale-out, dispatcher/babysitter, design architect + program tending
- first_asked: 2026-10-27 ~02:12 CDT (usage upgrade; more lanes for all types), followed by lane rebalance (02:23 — code 4 lanes, nix 1, opencode/openchamber 2 each), cadence retiming (02:30 — staggered starts), and DESIGN ARCHITECT + PROGRAM TENDING built at his direction (02:30): a 6-hourly design-architect job (one repo per run, ≤1 design per run, designs needing Drew carry user/review) and a daily program-tending job (epic status comments, roadmap-drift PRs, board corrections, aging user/review surfacing).
- origin (corrected by gap pass 2026-11-11): the pool/routing program's origin is ~3 weeks earlier than the epic — **2026-10-05**, Drew in a direct OpenCode session (opencode:ses_ef2cf2f): the tiered-fallback / provider-rollup ask — "rollup various providers of the same model, so all gpt-6 luna as one, in a certain order… rollup cheap/fast models and smart/slow models… review uses smart/slow and hard uses frontier/heavy and explore/triage use cheap/fast". Same design, not a new item — the 2026-10-27 epic is its scale-out. The session's "don't break opencode 1" constraint is the same v1 exception F3.1 records, from this source.
- origin — multi-instance providers (gap pass 2026-11-11): **2026-10-24**, Drew in a direct OpenCode session (opencode:ses_e93894d2dffe): "openai codex opencodego anthropic providers can each accept multiple… maybe like codex[customAlias]" — the origin of the multi-door provider system this program routes over; the alias syntax he proposed is not what shipped. Previously uncited in the register.
- tracking: developing-today/code#1334 **[live-checked 2026-11-11: OPEN]** — "epic(fleet): pool & routing intelligence — lanes everywhere, live dispatch + babysitting, quota stewardship, reviewer oversight, AA + measured-speed model"; children #1335–#1343 + #1351 (design architect) + #1352 (program tending).
- design_artifact: the epic + hidden_files instruction files (lane-shift, babysitter, dispatcher, pool-quota-steward, design-architect, program-tending) under the keep-the-four-repos-healthy goal
- design_status: built-into-running-system
- implementation_status: shipped — 13 new crons created and running (A/B lane shifts, babysitter 20m, dispatcher 30m, quota steward hourly, meta-reviewer + pool-intelligence daily, producer B lanes).
- followups_open: recorded PENDING CORRECTION — babysitter/dispatcher/steward instructions and #1337/#1338/#1339 were written with the RETIRED ≥85% caution-zone gate and still needed updating to Drew's 2026-10-27 burn-to-100%-then-closed rule.

#### F4.4 Executor-expansion directives — routing policy, OpenClaw jobs, scheduled-tasks migration, unconfigured jewels
- first_asked: 2026-10-29 02:22–02:27 CDT — Drew's batch of 5 directives: begin passing jobs to OpenClaw and review every OpenClaw session's results against OpenCode; pass things to Aster more often (smart design, thorough implementation) with worth-it MEASUREMENT; a routing policy (specialist vs workhorse, summit only rarely, sessions change their own model natively, subsessions select route pools, pool-aware managed-first preference); find places to use scheduled tasks on OpenCode, OpenClaw, Aster, ChatGPT, and Claude and replace a majority of the Muse tasks that launch OpenCode where sensible; hunt unconfigured "jewels" (AI in GitHub Actions, failure-triggered triage, light-triage spots). Scheduling corrections the same night: migrations are ADDITIVE FIRST (never remove the Muse side first); the 30-min OpenChamber cap is ours to turn off; token data exists in OpenCode's session DB (an exporter pulls it — #2072); review metrics are RELATIVE (cost-relative tokens + time + outcome quality).
- tracking: developing-today/code#2169 (routing policy) OPEN; #2170 (OpenClaw in #alignment) OPEN; #2171 (OpenClaw: finish setup, begin passing jobs, review vs OpenCode) OPEN; #2174 (OpenCode scheduled tasks migration) OPEN; #2177 (OpenClaw scheduled tasks) OPEN; #2176 (unconfigured jewels) CLOSED; #2072 (token exporter / unified observability) OPEN (all verified in slice 5).
- design_artifact: study findings in ~/workspace/research_notes/ (opencode-scheduled-tasks-20261029, openclaw-scheduled-tasks-20261029, aster-utilization-20261029, unconfigured-jewels-20261029); Aster dispatch sched-placement-design-20261029 in Slack #alignment
- design_status: drafted (studies landed same night)
- implementation_status: partial — first job ever passed to OpenClaw (smoke run, review row #1 on #2171); OpenCode study verdict: wave-1 migration ≈3.4B of ~5.1B daily Claude tokens, ceiling = pool quotas + laptop 12-session cap; #2170 carries Drew's correction: per-pair transport is an OPTION he permitted, not a decision.
- followups_open: per-system scheduling issues to be filed from the findings; Aster's ACK/RESULT/dispatch-ID contract still a proposal, not adopted practice (her substantive replies live only in Slack threads — transcription gap); OpenClaw ACP job passing at scale gated on PR #1728 + the NixOS switch.

#### F4.5 Quota full-utilization program + burn plan
- first_asked: 2026-10-29 — stop letting non-DeepSeek plans end periods under-used: maintain a theoretical full-utilization plan for EVERY pool's largest window at all times, burn rates measured from recorded history, cascade projections across consecutive resets, dated audibles (route-pool weight/priority changes, queue more work when under-pace), checked every 4–6h; over-pace → flag Drew (his bias: useful work sooner is good).
- tracking: developing-today/code#2173 **[live-checked 2026-11-11: CLOSED]** — "Full-quota utilization: burn-rate projections, cascade plan, and 4–6h checks"
- design_artifact: ~/workspace/research_notes/quota-burn-plan-20261029/plan.md; plan v1 posted on #2173 (comment 6490830792) with five dated audibles A1–A5
- design_status: decided
- implementation_status: shipped (plan v1 landed 2026-10-29 ~02:35 CDT)
- followups_open: next build step recorded — evolve the 6-hourly go-plan-expiry-watch into the 4–6h full-quota check; over-pace flag (Go Lab + ClinePass exhausting early) raised to Drew with a recommendation — NOT answered at flush; named data gaps: Antigravity weekly rows missing since Oct 27, Command Code reads failing, Meta file-export series stale.

#### F4.6 Machine pool — onboard additional machines as fleet job nodes
- first_asked: 2026-10-29 02:37 CDT — a separate job for onboarding additional machines as job nodes like the laptop, plus redesigning fleet structure around a machine pool.
- tracking: developing-today/code#2179 **[live-checked 2026-11-11: OPEN]** — "Machine pool: onboard additional machines as fleet job nodes and redes…"
- design_status: drafted (the issue itself)
- implementation_status: not started in-slice (no design doc recorded)
- followups_open: the fleet-structure redesign around the pool is the issue's open scope.

#### F4.7 Structural redesign — ONE fleet minder, dropped-thread catcher, Builder's Advocate
- first_asked: 2026-11-08 ~18:19 CST — Drew's structural rulings: (1) exactly ONE Muse minder job survives — checks ALL sessions and tasks, and must actively KICK, not just watch; (2) issue-making/priority/design functions: build ONE Muse version and ONE OpenChamber version, completely separate, rewritten from scratch per Drew's actual wants; (3) NEW HOURLY Muse task — dropped-thread catcher: reviews ALL chats from the past 2 hours + older open issues; catches problems mentioned then dropped; create an issue, or create an issue AND solve it; full authority to approve and handle whatever it believes it can; (4) blocked-claim validator duty in the same task: every "blocked, nothing can be done" claim assumed WRONG until validated.
- tracking: no single issue; role map developing-today/code#2131 CLOSED and #2152 CLOSED ("Fleet roles: Make / Do / Tend")
- design_artifact: the cron bodies themselves (fleet-minder 15m, dropped-thread-catcher hourly, builders-advocate 4h), created 2026-11-08 18:29–18:31 CST
- design_status: built-into-running-system
- implementation_status: shipped (Muse side) — absorption deletions of 11 absorbed Muse crons completed 2026-11-08 22:30–22:42 CST after the first clean advocate run.
- followups_open: OpenChamber-side recreations were still RUNNING at the 18:30 flush (Lane A; the OC builder's advocate text was delivered to Lane A, pending its handoff).

#### F4.8 One-queue model
- first_asked: 2026-11-10 — "there's just one queue." / "all items should be one queue. by default all lanes pull in priority order from items that are available and eligible for the worker to pick up… set 3 not code lanes and 5 code specific lanes on both openchamber and muse. All rest unfiltered" — refinements same evening: claim-time composable boolean filter expressions, don't tie items to lanes by default (20:02); claimer identity defined/stamped but usually not a filter (20:04); two-sided matching — union of task requirements and claimer requirements, both empty by default (20:06).
- tracking: recorded on developing-today/code#8097 (comments 6785478676 + 6785478985); implemented as PR developing-today/code#8438 (merged 2026-11-11T02:28:30Z, 2efbeb29); defect filed code#8456 (by-id claims don't enforce platform limits); OpenChamber side: queue task 14835 (PARKED at record time) + developing-today-forks/openchamber#2081.
- design_artifact: ~/workspace/goals/keep-the-four-repos-healthy/hidden_files/one-queue-gap-analysis-and-designation-2026-11-10.md
- design_status: built-into-running-system
- implementation_status: shipped — PR #8438: unscoped claim default, --filter expressions, payload requires enforced on pick/by-id/transfer. Deploy required the broker takeover restart + `UPDATE lanes SET queue='*'` to land WITH the broker deploy.
- followups_open: G7 defect #8456; OpenChamber designations (task 14835 parked).

#### F4.9 Lane naming law (FINAL)
- first_asked: 2026-11-10 21:29–21:35 CST — Drew's rulings: (1) "multiple letters is fine for separating lanes but the name should still make sense besides the shard"; (2) "what does claude have to do with anything" — never label lanes "claude"; (3) surface origin is platform metadata passed separately, never in the name. Accepted the scheme at 21:33 ("well that's at least sort of coherent") and amended at 21:35: "fine not-code instead of other".
- tracking: recorded on developing-today/code#8097; executed via the lane registry (no dedicated issue)
- design_artifact: the broker lane registry itself (36 rows, 17 enabled at final state)
- design_status: built-into-running-system
- implementation_status: shipped — law: name = ONE scope word (code / not-code / any) + ONE fresh shard letter unique within its scope group; applies to both surfaces including OpenChamber's 12 identically-named tasks. "Lane rename + trims: COMPLETE" per the 2026-11-11 record.
- followups_open: anomaly at record time — broker rows openchamber-a/openchamber-b reappeared enabled=1 after deletion; was being investigated.

#### F4.10 laptop-agent — resident execution agent / session proxy
- first_asked: 2026-11-04 ~01:53 CST — a resident agent on the laptop that all fleet instructions connect through (the session proxy), replacing per-command SSH spawns; warm broker follow-up ordered ~02:02 CST.
- tracking: developing-today/code#5343 CLOSED — "design: laptop-agent — resident execution agent on the laptop (replace per-command SSH spawns)"; phase-1 results on #5343 comment 6625727705. Records-pass sibling: code#5769 (OPEN per census) "Migration M2: one declaration for the fleet three (laptop-agent, secrets-broker, queue-broker)".
- design_artifact: design doc in repo-health goal files (laptop-agent-design-2026-11-04.md, records pass §1b)
- design_status: built-into-running-system
- implementation_status: shipped (phase 1) — daemon live on the laptop tailnet address :4799, tailnet-only, token auth.
- followups_open: phase 2 call-site migration; phase 3 NixOS declaration.

#### F4.11 Repo-queue + secrets brokers on the laptop
- first_asked: 2026-11-04 ~02:22 CST — port repo-queue to the laptop as a broker: TWO binaries (secrets broker pool; queue broker); all fleet instructions connect ONLY to the session proxy; build first as nix packages OUTSIDE the NixOS flake, then bake into the dezren39/nix flake.
- tracking: developing-today/code#5362 CLOSED — "design: repo-queue + secrets brokers on the laptop behind the session proxy"; broker-family census (records pass): #5769 M2 one declaration (OPEN), #5795 relay trust model & broker authorization (design), #5792 id broker / serve-proxy role (design), #5694 remote desktop through the fleet brokers (design), #5860 secrets-broker TTL=0 spin bug, #6767 secrets-broker stale-secret cache bug, #6369 alert when repo-queue broker transport is down, #6449 queue-validation inventory (Phase 4 of #6448).
- design_artifact: ~/workspace/goals/keep-the-four-repos-healthy/files/queue-secrets-brokers-design-2026-11-04.md
- design_status: built-into-running-system
- implementation_status: shipped same day (Rust workspace ~/tools/fleet-brokers; secrets-broker :4800, queue-broker :4801 on the laptop tailnet address, token auth; interim systemd user services active; secrets cache populated, 134 secrets).
- followups_open: at record time still pending — nix pkgs standalone build (dezren39/nix pkgs/fleet-brokers), end-to-end canary + shim cutover, fleet instruction/doc updates, flake bake.

#### F4.12 Secrets broker read-through on cache miss
- first_asked: 2026-11-05 — "broker should go get the secret pass through." — the broker must fetch from Bitwarden on a cache miss and pass the value through (populating cache), and a client must never exit 0 with an empty value.
- tracking: developing-today/code#6275 CLOSED — "secrets broker: no read-through on cache miss — newly minted secrets invisible up to 15 min, client returned empty"
- design_status: built-into-running-system
- implementation_status: shipped — Drew upgraded it from filed to IMPLEMENT 2026-11-05 10:18 CST (root cause: broker was cache-only with a 900s refresh TTL — morning-minted secrets invisible until refresh; this bit the ask-ledger build); lane dispatched; issue CLOSED per gh in slice 6.
- followups_open: none recorded.

#### F4.13 One `fleet` multicall binary epic
- first_asked: 2026-11-04 — design pass ordered by Drew: one `fleet` multicall binary covering search, queue, dispatch and proxy, with capability-selected builds.
- tracking: developing-today/code#5412 CLOSED — "epic: one `fleet` multicall binary — search, queue, dispatch, proxy — with capability-selected builds"; placement absorption #5413 CLOSED (F4.14); design note doc/2026-11-04T11-45-48Z_design_fleet_multicall_binary/ via PR #5425 (OPEN at record time); epic comment 6629870171 records decisions D1–D8.
- design_artifact: the design note; decisions D1–D8 on the epic
- design_status: decided (design) — recommends workspace pkgs/fleet/ in code; variants fleet/fleet-serve/fleet-connect; proxy keeps length-prefixed JSON + two-hop, adds versioned hello/capability frame; stub exit code 69; `id` stays separate; brokers become `fleet serve <role>`; laptop-agent stays Python in v1; dispatch = thin client. Census correction: the broker family had THREE source copies; canonical = sandbox ~/tools/fleet-brokers.
- implementation_status: not established (design-stage at record time)
- followups_open: Drew's picks on decisions D1–D8 not shown in the record read; dezren39/nix PR #1459 still OPEN at record time. Successor framing: F4.15/#8575 makes the `fleet` tool the single door — the two designs should be read together.

#### F4.14 Execution surfaces program + fleet-place placement system
- first_asked: 2026-11-04 — Drew chose "Do both" and ordered the full design ~04:53–05:40 CST: divvy work by state/supervision — LAPTOP = stateful/secret/live; GITHUB ACTIONS = stateless/reproducible/parallel; CODESPACES = interactive/overflow seats, never daemons/scheduled. Standing extensibility rule from Drew: new locations = registry entries only; new placement types = one adapter + kind registration; agent-facing interface frozen/versioned.
- tracking: developing-today/code#5413 CLOSED (placement absorption); fleet-side docs in dezren39/fleet
- design_artifact: dezren39/fleet docs/execution-surfaces.md (PR #58 merged 0153eb03); placement system on branch fleet-place in the developing-today/fleet fork (tools/fleet-surfaces.json registry AS DATA + tools/fleet-place dispatcher, one adapter per kind, app-only dispatch, spill cascade GHA → Blacksmith → Codespaces → QUEUED, never dropped; sessions with fp_ handles; R2 nix cache push per job)
- design_status: decided
- implementation_status: partial — fleet side DONE (fleet-run workflow_call + caller fleet-umbrella-verify.yml, PR #57 merged; org-run.yml in code dispatch-only per Drew's push-workflow ban + Blacksmith runner input, PR #5417 merged); placement push BLOCKED at record time pending grant G1.
- followups_open: grants batch G1–G4 and the preemptive backup app "developing-today fleet" were BLOCKED on a Drew-only GitHub sudo gate (Helium "Confirm access").

#### F4.15 Queue-state unification — the `fleet` tool as single door (#8575)
- first_asked: 2026-11-11 — Drew corrected the attribution (~01:02 CST: the broker concept and its two-places-to-keep-in-sync burden were Muse's design, not his ask), accepted the split ("OK that works": coordination may live in a DB; durable truth lives ONLY on GitHub; DB contents must pass the wipe test), and directed: the `fleet` tool is the single door for nearly all interactions, must handle all known/future edge cases and fallbacks itself; plans required per scenario (GitHub write failure, GitHub read failure, DB failure); DB keeps a cached copy of most GitHub details fed by a webhook to a Cloudflare Worker; local git clone kept updated at all times; backups to consider: Turso DB backed up into D1; code repo backed up into Cloudflare repos or equivalent. Drew also corrected "GitHub down = fleet down": git is distributed — lanes keep working claimed items on laptop-local branches, stack them, queue merges in the coordination DB, rebase on return. Then Drew RULED on the open questions and ordered full implementation ("Just do it all"): (1) staleness — never stop working, no threshold idles the fleet; (2) repo backups — all three: Cloudflare rehosting AND a plain mirror elsewhere AND R2 bundles.
- tracking: developing-today/code#8575 **[live-checked 2026-11-11: OPEN]** — "Design: one door, one home per fact — the fleet tool owns every route and every fallback"; rulings on #8575 comment 6789860516; seven implementation slices #8579–#8585, indexed on #8575 comment 6789883623. **Live state (this pass): #8579 CLOSED (slice 1, wipe-test audit — implementation has begun, one step beyond the slice record which had all seven open at filing); #8580, #8581, #8582, #8583, #8584, #8585 OPEN.**
- design_artifact: ~/workspace/fleet-unified-state-design-issue.md (issue body source); design doc for slice 1 in the doc/ corpus (doc/2026-11-11_design_id8579_queue_schema_wipe_test_audit.md)
- design_status: decided (ruled + implementation ordered)
- implementation_status: partial (1 of 7 slices closed)
- followups_open: slices #8580–#8585 delivery; slice 6 = #8584 (backups provisioning) parked awaiting Drew (see X17).

#### F4.16 Provider browser-auth auto-renewal standing rule
- first_asked: 2026-11-05 01:10 CST — Drew's standing rule: provider browser auth must auto-renew; keepers own renewal so lanes never stall on expired provider logins. (Paired doctrine same morning, 01:13 CST: "direct providers EMULATE, never embed".)
- tracking: developing-today/code#6120 **[live-checked 2026-11-11: OPEN]** — "Provider browser-auth auto-renewal — standing-rule follow-through (OpenDesign keeper landed; Meta / Mistral / Claude / ClinePass gaps)"
- design_status: decided (standing rule)
- implementation_status: partial — OpenDesign keeper landed; Meta / Mistral / Claude / ClinePass gaps open.
- followups_open: the four named provider gaps in #6120.

#### F4.17 Agent SDKs + always-on swarms investigation
- first_asked: 2026-10-21 — Drew's questions on agent SDKs and scheduling surfaces (which products let him trigger agents via API, incl. two Wispr-garbled names "Rockbot" and "Dots") and on always-on vs ephemeral agent swarms; his stated position recorded for verification: run jobs on his own computers and save quotas; a business plan's remote trigger buys reachability, not capability.
- tracking: NEVER-ISSUED as an issue; tracked as goal agent-sdks-scheduling-surfaces-investigation (goal_fe27e03b921a) — goal-tracked, register-only (not a triage-(a) item: the investigation vehicle exists and the open residue is a Drew-answer, also pursued via the decisions page's request-info flow for census-agent-sdks in November).
- design_artifact: ~/workspace/library-build/agent-sdks/RESEARCH.md (when done)
- design_status: drafted (investigation)
- implementation_status: partial — research delegated and RUNNING at flush; analysis of swarm control patterns (partition-by-construction, funnel, tournament) delivered to Drew in chat.
- followups_open: "Rockbot" and "Dots" unresolved — no product in Drew's records matches; Drew to confirm (candidates Dust, Devin, Lindy… unconfirmed); baseline noted: consumer Muse has NO public agent API/SDK (Drew filed a feature request 2026-09-29).

#### F4.18 Time-based priority schedules for route pools — added by gap pass 2026-11-11
- first_asked: 2026-10-12 — Drew, in a direct OpenCode session (opencode:ses_ecfab4789ffe): "expand how the config works to be able to set specific times to deprioritize or prioritize specific provider/model combinations and also per-provider or per-model/route pool, so that you can express… these are the peak hours and maybe choose another model if there are other options." Schedules by day of week + time + timezone + specific days of month; a default priority level (every model = 1); stickiness and user-override rules still apply on top.
- domain: ID & fleet / routing (pool program)
- tracking: NEVER-ISSUED at audit time — no matching issue found (gh searches: "peak hours schedule pool", openchamber/opencode title scans); the register's only "time-based" hit was the unrelated routine-job entry F4.2 — triage **(a) STILL-LIVE**; **filed 2026-11-11 as developing-today/code#8838** (see §7 manifest #9)
- design_artifact: none — the ask lives only in the session (design review commissioned, no artifact survives in the scanned trees)
- design_status: none
- implementation_status: not established — no record of schedules landing in the pool config in any scanned source
- followups_open: the whole item — whether time-based pool priority was ever built is unverified in every source the gap pass scanned.

---

## 3. PM STACK

### P1. PM stack program — one single URL for all projects (PARENT)
- first_asked: canonical design approved 2026-10-24 (doc/2026-10-24T11-05-38Z_design_project-management-stack/); ownership directive 2026-10-26 (P1.3).
- tracking: goal `pm-stack-single-url-project-hub-own-end-to-end` (its files/ are empty — per the records pass, work is tracked in the code#1184 epic instead); canonical design record in the code repo doc/ tree; planning-layer copy at pm-stack/pad/seed/plans-pm-stack-design.md; current-state ownership map at docs/pm-stack/current-state-ownership-map-2026-11-05.md.
- design_status: built-into-running-system
- implementation_status: partial (the coherence epic is the standing gap list)
- followups_open: the sub-entries.

#### P1.1 Project-tracking tooling evaluation — GitHub Projects vs Linear vs self-hosted
- first_asked: 2026-10-23 — "we'll probably go with projects but lets explore first": a comparison of GitHub Projects vs Linear first, then an open-source self-host exploration (Plane, Huly, OpenProject, WeKan…) and reviews of ready-for-agent and Pad, before committing the fleet's project tracking to a tool.
- tracking: no issue filed for the evaluation itself; its outcome (GitHub Projects, org Project #4) became the substrate for the PM stack.
- design_artifact: ~/workspace/your_files/github-projects-vs-linear-2026-10-23.md
- design_status: decided (Projects remains the lean; no final pick ceremony recorded in-slice)
- implementation_status: shipped as substrate — ready-for-agent (6056) and Pad (7777) were later adopted as stack components.
- followups_open: the personal-repo seam (an org Project can't hold dezren39/nix items) was noted unresolved at evaluation time.

#### P1.2 Dashboard slice 3 — pluggable pages, DNS register page, dashboard comments, agent/session identity registry
- first_asked: 2026-10-25 — Drew's direction filed as code#906: (1) pluggable dashboard pages (page = small module + one manifest entry) with the DNS register page first; (2) the dashboard stays PM-primary but becomes the general ops console; (3) comments on needs-user / needs-orchestrator / roadmap / dashboard objects in two kinds (message-to-Muse vs plain note) plus post-as-GitHub-comment; (4) an agent/session identity registry: stable ID + display name at launch, an OpenCode tool auto-injecting the caller's session ID, a collector extracting session IDs/parent links/token usage into a DB.
- tracking: developing-today/code#906 **[live-checked 2026-11-11: CLOSED]**; cross-referenced on code#902 (DNS register pipeline)
- design_status: built-into-running-system
- implementation_status: shipped (issue closed with the slice-3 implementation)
- followups_open: none recorded in-slice.

#### P1.3 PM stack coherence epic — full review at Drew's direction
- first_asked: 2026-10-26 22:35 CDT — Drew (dashboard-status side chat) found the multi-project issue set necessary but NOT sufficient and directed a FULL review of every PM stack page as coded AND running, every glued service/port, data freshness/adoption, and coherence — restating the yardstick: ONE single URL (mobile+desktop) showing all projects, quotas/usage, page health, needs-attention; remediation/basic triage from the dashboard (not read-only); queue visibility = next-up / in-progress / blocked / recently-done; hierarchy roadmap item > epic > issue with rollups; the PM stack treated as a top-tier project with its own progressing top-level roadmap item. Muse owns the PM stack improvement process end-to-end.
- tracking: developing-today/code#1184 **[live-checked 2026-11-11: OPEN]** — "epic(dashboard): PM stack coherence - single-URL, fresh data, triage-able, hierarchical" plus 34 net-new issues #1134–#1183 across front door/nav, freshness/honesty, health/observability, triage actions, queue visibility & hierarchy, quotas, contracts/perf/access. Records-pass siblings in the census: #1182 one public URL behind Cloudflare Access **[live-checked: OPEN]** (settled, not implemented), #1253 one identity/SSO (design), #1260 SLOs + self-healing (design), #1198 one status taxonomy, #1281 WIP limits + flow analytics, #2576 aggregator ingest fail-closed contract (design), #1744 centralize service data in one DB schema, #3989 move stateful services off the laptop + host failover.
- design_artifact: the #1184 epic + the review pack (~/workspace/pm-stack-review/: epic-body.md, code-audit.md, github-audit.md, filing manifests/results, wave2 files) + ~/workspace/pm-stack-coherence-design-2026-10-26.md + ~/workspace/pm-stack-visualization-design-2026-10-26.md (complexity-weighted visualization, #1122 Phase B)
- design_status: decided (review COMPLETE same night; epic seated for Project #4 with epic/now labels; scope separated from multi-project epic #1129 by a relationship comment)
- implementation_status: partial — the 34 child issues are the standing follow-up set. Review headline: /attention falsely showed "Needs User (0)" vs 11 published NEEDS USER — root cause #1110.
- followups_open: #1182 (public URL behind Cloudflare Access) needs Drew's go per the review; the child-issue set.

#### P1.4 Needs-user action model — design + contract issues #1290–#1293
- first_asked: requirements 2026-10-26 ~23:36 CDT → filed 2026-10-27 ~00:01 CDT: (1) the API must not say "absent" for the orchestration section; (2) every needs-user item must have a legible dashboard view (what it is, why it needs him, what happens if ignored) with CLEAR ACTIONS; (3) options in two kinds — STANDARD next-step options always available + CUSTOM choices the item suggests; (4) items may carry SURVEY/QUIZ QUESTIONS covering what is missing, and answering them unblocks the item; (5) per-item DELEGATION MODES — orchestrator first-pass then approve, or blanket auto ("the orchestrator can do all this unless it defines that it's not able to"), returning only with a structured inability declaration.
- tracking: developing-today/code#1290 (needsDrew item contract) CLOSED; #1291 (/attention action UI) CLOSED; #1292 (orchestrator delegation execution) CLOSED; #1293 (proposal review state handling) CLOSED; summary comment on epic #1184 (all four verified in slice 5).
- design_artifact: ~/workspace/pm-stack-needs-user-actions-design-2026-10-26.md — standard action set (Answer, Delegate first-pass, Delegate auto, Snooze until, Dismiss with reason, Escalate; proposals add approve/reject-proposal); design rules: new fields optional + validated per-item; delegation is state on the same item, never a second item; delegated items must reach proposal/done/unable within 2 orchestrator runs.
- design_status: built-into-running-system
- implementation_status: shipped (all four issues closed)
- followups_open: one decision left pending Drew's tap in the record — how auto mode is set (proposed default: per-item explicit opt-in only; a standing auto toggle for decision/missing-info/approval classes designed but OFF until Drew enables it; whyDrew classes access + policy hard-excluded from auto under any setting).

#### P1.5 PM-stack durability & host-independence design rulings
- first_asked: settled 2026-11-03 — Drew settled all 7 open questions of the pm-stack durability design in chat: stack state in backed-up durable storage; stack survives the laptop being off (Q2 superseded by his all-in-Supabase ruling, Q7 no exceptions); Q3 off-machine retention = daily generations 30 days + monthly 12 months across local + R2 + Drive; Q6 Project exports retained 1 year.
- tracking: developing-today/code#1258 CLOSED — "design(pm-stack): durability & host independence — stack state in backed-up durable storage, stack survives the laptop being off"; rulings landed as comment 6652204901 on 2026-11-05.
- design_status: decided + ruled
- implementation_status: partial — the rulings only reached the issue on 2026-11-05 after two failed postings sat in the action outbox (a flow-back failure, later addressed structurally by the dropped-thread catcher, F4.7); implementation of the retention scheme not verified in this slice's record.
- followups_open: retention-scheme implementation verification.

#### P1.6 PM stack under NixOS switch
- first_asked: 2026-10-27 directive: "make nixos switch deploy all these components like everything else. enable projector and all the other features." Renewed 2026-10-30 ~04:37 CDT: "make pm stack build run with nixos switch" — with his standing preference confirmed the same morning: when a stored fleet credential can do an infra step, Muse does it ("no you do that or use your auth token").
- tracking: no single issue cited (work tracked via the code repo module + an implementer branch); sibling pieces DO carry issues: fleet-feed collector fold-in developing-today/code#3363 **[live-checked: CLOSED]** (gap originally filed as #3347; PR dezren39/nix#3348 MERGED); pm-dispatch Supabase config PR developing-today/code#5129 merged 2026-11-03 after Drew's "fix all that — NixOS should manage" ruling, residual legacy references tracked by code#5120.
- design_artifact: developing-today/code nixos/services/pm-stack/default.nix (aggregator/projector/git-sync/quota-gatherer as home-manager user services out of the ~/git/code primary, env rendered from the pm-stack SOPS file)
- design_status: decided
- implementation_status: partial — module wired in (nixos/hosts/default.nix services.pm-stack.enable = true; aggregator + projector units live); Oct 30 implementer SPAWNED on branch feat/pm-stack-nixos-complete to close the gaps (pm-dispatch as a module unit, pm-tunnel, pm-kaneo oneshot + deploy.sh, pm-git-sync redeploy mapping for dispatch); NO switch run (needs Drew's sudo; a competing switch was already blocked by the summarize-pnpm-deps hash mismatch). The #3363 first switch FAILED (nix daemon ENOSPC, / at 100%); Drew's follow-on orders ("nixgc", "cleanup rust cache etc", "cleanup target", "start switch too") were RUNNING at that flush with outcomes unknown in that record; #3363 is CLOSED per gh.
- followups_open: activation note for Drew (exact switch command + what changes) owed on the implementer's completion; pre-existing fault found out of scope — pm-git-sync.service FAILED with a dead timer (needs a clean ~/git/code, prevented by session-state dirt); code#5120 — pm-stack/supabase-export + aggregator export test still reference the legacy (deleted) muse-dev project ref in source.

#### P1.7 pm-dispatch service — launch/track OpenCode sessions from GitHub Actions
- first_asked: 2026-10-30 (Repo sweeps chat) — build the dispatch service: new pm-dispatch service in pm-stack/dispatch/ beside the aggregator; auth = GitHub Actions OIDC audience "pm-dispatch" with the token's repository claim bound to the request's repo, static bearer fallback; POST /v1/sessions = issue first → enqueue → launch session → task PATCHed claimed with session_id; POST /v1/tasks = enqueue only. Drew also asked "gh runners use oidc? tied to my org or what?" (answered: per-repo binding, not an org-wide key) and ordered the Cloudflare tunnel hostname himself be bypassed — "no you do that or use your auth token."
- tracking: developing-today/code#2648 **[live-checked 2026-11-11: CLOSED]** — "feat(pm-stack): dispatch service — launch/track OpenCode sessions from…"; staged launch-pipeline design #2668 CLOSED; same-day hardening filings #2660 (post-claim comment/label failure cancels a live session's task), #2663 (ensureWorker memoizes first failure), #2664 (session directory unvalidated), #2665 (non-object JWT → 500 not 401).
- design_artifact: pm-stack/dispatch/ in developing-today/code (PRs #2650/#2652/#2655/#2658 merged); design doc files/designs/code-pm-dispatch-launch-pipeline.md (design-architect pass 19, #2668, adopting #2664+#2665 as Phase 1, #2660 Phase 2, #2663 Phase 3); sample caller pm-stack/dispatch/examples/dispatch-workflow.yml
- design_status: built-into-running-system
- implementation_status: shipped — built + deployed + smoke-verified live 2026-10-30 (user unit pm-dispatch.service, port 4879, loopback + tailnet; public https://dispatch.security.cab/health verified after Muse added the tunnel ingress + DNS via the Cloudflare API).
- followups_open: the design pass flagged Phase 1 (#2664/#2665) should land BEFORE the public hostname — the hostname went live first at Drew's explicit order, so those two fixes are the top hardening priority for an already-public endpoint; pm-dispatch also postdates the NixOS pm-stack module (P1.6).

#### P1.8 Quota plane — gatherer, usage hub, single-source `opencode quota`
- first_asked: 2026-10-27 — the PM-stack quota plane Drew's program called for: a 15-minute laptop gatherer unifying every provider's quota into quota-current.json for the aggregator, and a Meta adapter that reads the dev.meta.ai console without the session cookie ever leaving the page.
- timeline: 2026-10-25/26 — usage-hub designs (records pass §1b: usage-billing-hub goal files hub-design-2026-10-25.md, hub-v2-v3-design-2026-10-26.md, quota-plane-design-2026-10-26.md; epic code#1047 usage-hub SaaS completion; code#1149 quota-plane Stage 2 epic). 2026-10-27 — gatherer v1 + Meta Phase-1 exporter built. 2026-10-29 — burn plan (F4.5). 2026-11-05 01:25 CST — Drew's order: make `opencode quota` the single source of truth; the gatherer and dashboard read only from it.
- tracking: developing-today/code#1151 (quota-gatherer service) CLOSED (verified slice 5); #1301 (Meta adapter Phase 1) CLOSED; console investigation #1167; single-source: code#6125 **[live-checked: CLOSED]** — "Direct providers: full-function completeness + quota single-source (opencode quota subcommand)". Quota census (records pass): #1149 Stage 2 epic, #1177 Stage 3 predictive quota ops (design), #1175 /quota status view, #2490 one quota read path, #1270 plan-store/tracker reconciliation, #1115 managed quota-credential pattern (design), #1169 Google/Antigravity exception integration, #1159 ClinePass adapter, #1155 token-exception audit, #3983 exhaustion forecasts, #6031/#6033 watchdog designs, #2072 unified PM+queue+quota attribution, #5770 Migration M3 sandbox quota-recorder, #8022 quota-recorder spool alert. Owning goal: usage-billing-hub-token-quotas-per-service-saas-limits (GOAL.md current state 2026-11-08: daily quota reads running; known gaps — Claude quota read stale, Turso quota sink DB (muse-platform) dead since Nov 7, diagnosis owed).
- design_artifact: pm-stack/aggregator gatherer (PR #1309 merged) + pm-stack/meta-exporter (PR #1303 merged); the file-export convention defined by #1301 (~/atlas/meta-quota-export/latest.json atomic + history.jsonl append-only; fail states write a status doc with measuredAt:null and no values), adopted by the gatherer; a "usage-hub design" doc existed among Drew's Oct 25 files as the design antecedent.
- design_status: built-into-running-system
- implementation_status: shipped — gatherer LIVE 2026-10-27 ~01:56 CDT (systemd user timer *:0/15 on the laptop; 89 snapshot records; Meta exporter two consecutive scheduled ticks verified). Single-source DONE ~04:05 CST 2026-11-05: `opencode quota` subcommand in the fork (PRs #1722 + #1728 merged); gatherer rewired so quota-current.json is populated EXCLUSIVELY from `opencode quota --json` (code PRs #6136/#6190/#6191 merged); Supabase sink agent.quota_records with source "opencode-quota"; dashboard Refresh quota button (PR #6141).
- followups_open: production flip rides a unit drop-in (QUOTA_SUBCOMMAND → fork-built binary) until the nix switch ships the system binary — REMOVE the drop-in at switch; defect filed as opencode#1729; Claude errored every tick at first wiring (session expired, #1251); Go not split per-workspace in module records (known gap).

#### P1.9 General Board vs PM dashboard comparison → teardown
- first_asked: 2026-10-28 — Drew did NOT pick from the teardown widget; instead he asked (11:44 CDT) for a comparison of the General Board against the PM dashboard pages — is there anything unique to the board worth integrating into the PM stack before teardown? After the verdict he tapped "Tear down the board, and file the /health lag-count issue" (11:49 CDT).
- tracking: developing-today/code#1734 **[live-checked 2026-11-11: OPEN]** — "pm dashboard /health: per-source lag counts (behind-by-N + last sync)" — the comparison's one keep-worthy idea; the teardown itself was executed under Drew's approval without its own tracking issue. Board-family census items (records pass) belong to the ID board design line (#5677/#5862/#5856…), not this board.
- design_artifact: the code-verified comparison (board worker/src/index.js + agent/agent.py vs the aggregator's page set)
- design_status: decided
- implementation_status: shipped — verdict: almost nothing unique; everything already in the PM stack in richer form (board page, JSON twins, request→apply→report loop, /health liveness); unique-but-not-worth-porting: exact-case tag search, per-machine embedded replicas (contradicts PM's GitHub-as-data-plane design), sync presence-proof, always-on public edge page, free-form cross-machine posting. Board torn down (Turso DB general-dezren39 + Cloudflare Worker + agent); the board had held only 4 Oct-23 bring-up test entries and was never adopted.
- followups_open: #1734 (per-source lag counts for /health) open; the globalping-watch target removal for the board was the one held step pending Drew's explicit go at record time.

#### P1.10 Today/Board staleness — orchestration-pass re-enable question
- first_asked: 2026-11-01 — Drew reported the board/today pages broken/stale; diagnosis: /api/today.json frozen because the orchestration-pass Muse cron was disabled, and the board holds only 7 curated items (newest 2026-10-25). Drew was told re-enabling the hourly orchestration pass is his call as a cadence change.
- tracking: NEVER-ISSUED (cron/cadence question, no GitHub issue in the record) — triage **(c) SUPERSEDED/DEAD**: raised, unanswered in that conversation; orchestration passes are running later in the record, so the question was overtaken by events. Register-only.
- design_status: none
- implementation_status: abandoned-in-record (overtaken)

#### P1.11 PM-stack data-home migrations (records-pass reconciliation)
- first_asked: not in the day-slices; reconciled from the records-pass census so the register carries them:
  - Long-term migrate pm stack into Supabase: developing-today/code#3142 + phases #5005, #5006, #5007, #5008, slice #6815 (open per census).
  - Centralize service data in one DB schema: #1744 (open per census).
- design_status: drafted (per census titles)
- implementation_status: not verified in this pass
- followups_open: read together with T4 (Supabase → Turso migration wave) — the two migration directions coexist in the open-issue census and the register does not reconcile their ordering.

#### P1.12 Real notification design + PM-stack phases 1–3
- first_asked: 2026-11-11 — Drew rejected the 5-minute polling cron as "the solution" (stopgap only) ~01:12 CST and required the end state: (1) the PM-stack website pushes ITSELF, event-driven; (2) a waiting-on-him notification presents multiple choices he can answer, with write-back that unblocks work; (3) a scheduled task only as a dead-man's switch; (4) investigate moving the PM stack to Cloudflare Workers + Durable Objects; (5) investigate merging the ~5 glued tools into one coherent whole.
- timeline: 2026-11-11 01:25 CST — Drew's amendment: "you would move the data sources to cf too, possibly sync with turso and or laptop" — Phase 2's scope amended on #8569 (comment 6789797620): full data-source migration to Cloudflare with Turso/laptop sync, not just an edge relay.
- tracking: developing-today/code#8568 **[live-checked 2026-11-11: CLOSED]** — Phase 1 (missing producers into the existing Web Push throat); #8569 **[live-checked: OPEN]** — Phase 2 (CF edge relay + Durable Object dead-man's switch; scope amended per above); #8570 **[live-checked: OPEN]** — Phase 3 (consolidation: aggregator, quota gatherer, fleet-feed collector and projector into one pm-core service; dispatch stays separate). Design summary posted on code#8097 (comment 6789653127; #8097 now CLOSED).
- design_artifact: ~/workspace/goals/keep-the-four-repos-healthy/files/pm-stack-notifications-design-2026-11-11.md (completed by lane d8ad8320, 2026-11-11T07:18:34Z)
- design_status: decided — verdict: NO-GO moving board/aggregator to Workers (all data sources laptop-local/tailnet-only); GO on a small edge relay (1 Worker + 1 Durable Object).
- implementation_status: partial — Phase 1 implemented (PR #8597 aggregator + PR #8598 CLI producer; proof push accepted 2026-11-11 07:44:12Z; whether the push reached Drew's phone was never explicitly confirmed by Drew — formally still open at window end, though Drew demonstrably reached and used the decisions page via a push link on 2026-11-11).
- followups_open: Phases 2–3 (#8569, #8570); root cause of the ~dozen /board 503s on 2026-11-11 unproven.

#### P1.13 PM aggregator PWA + Web Push design (ancestor of the real-notification design, P1.12) — added by gap pass 2026-11-11
- first_asked: filed 2026-10-27 (issue) → design doc 2026-11-01 — an installable PWA + real Web Push for the PM aggregator so needs-Drew items and quota walls reach Drew's phone within a minute; branches on #1182 (one public URL behind Cloudflare Access) and specifies both branches because the access model's implementation was gated on Drew's recorded go.
- domain: PM stack
- tracking: developing-today/code#1254 **[live-checked 2026-11-11: CLOSED]** — "feat(aggregator): installable PWA + push notifications — NEEDS DREW and quota walls reach Drew's phone within a minute"; implementation filed as sub-issues of coherence epic #1184
- design_artifact: ~/hatch-personal/workspace/scratch/push-design.md (2026-11-01; greenfield confirmed on main: no PWA manifest, /sw.js an unregister kill switch, no Web Push code)
- design_status: drafted → decided in lineage (its Web Push throat is what #8568 later built producers into)
- implementation_status: shipped-then-superseded in lineage — the throat exists (P1.12 Phase 1 built into it); #1254 itself is closed
- gap note: the register's notification story previously started 2026-11-11 (P1.12) and never cited #1254 or this design, though P1.12's own text depends on "the existing Web Push throat" this design created.

### P2. Decisions system (PARENT)

#### P2.1 Pending-questions ledger / Outstanding Questions system
- first_asked: 2026-11-04 23:47 CST — Drew requested the Outstanding Questions side chat as the mirror/working surface for the hourly pending-questions job: one ledger where everything waiting on him accumulates with evidence, so nothing he is asked gets lost.
- tracking: no single GitHub issue; the artifact is the ledger itself; owning goal pending-decisions-hourly-questions-for-drew.
- design_artifact: ~/workspace/goals/pending-decisions-hourly-questions-for-drew/hidden_files/pending-questions.json — **ledger state (records pass, read 2026-11-11): 150 items total: answered 84, asked 34, pending 20, dropped 12.**
- design_status: built-into-running-system
- implementation_status: shipped + running — this ledger is the substrate the decisions page (P2.2) and #8568's answerable pushes build on.
- followups_open: the pending design rulings it carries are listed in P2.3.

#### P2.2 Decisions page — batched decision sets
- first_asked: 2026-11-11 13:46 CST — Drew's spec: one push per waiting SET; a page to update one or more items; per-item submit (submitted items go away) + submit-all-filled; partial answers allowed; defer to the orchestrator per question or for the whole set; bulk send-back via checkbox or "all remaining", with flavors "defer / defer-skeptical / defer-approve"-ish. v2 refinements from Drew's live use 14:09–14:20 CST: per-question "Request more information" + note; plain-words relabels ("Defer to orchestrator" / "Defer — leaning no/yes"); filled answers win over bulk defer; mixed flavors with precedence own-pick > filled answer > dropdown default; guided defer. Post-window refinement ordered 2026-11-11 16:16 CST: drafts in BOTH server-side storage and local IndexedDB.
- tracking: developing-today/code#8766 CLOSED — "Decisions page: batched decision sets — one push per set, per-item/partial submit, defer-to-orchestrator with leans (extends #8568)"; crash follow-up #8777 **[live-checked 2026-11-11: OPEN]** — "Decisions page 'crash' 2026-11-11: pm-aggregator + pm-dispatch SIGTERM'd mid-session, no self-heal (Restart=on-failure treats SIGTERM as clean)".
- design_artifact: the page itself in the pm-stack; flow-back via fleet-feed decisionResolutions → dropped-thread catcher Step 2C → ledger
- design_status: built-into-running-system
- implementation_status: shipped same day — v1 PR #8767 merged (eeee50578); v2 PR #8770 merged (416b6fe5ee); draft persistence PR #8778 (localStorage, after the 2026-11-11 16:01:09 CST external SIGTERM killed pm-aggregator + pm-dispatch — issue #8777), then PR #8784 merged (7f14ad823e, IndexedDB + server-side drafts, per Drew's 16:16 CST order). First set decisions-2026-11-11: 15 items / 24 questions, all resolved by Drew; the catcher applied all 24 resolutions (11 items closed, 4 gates unblocked: code#3039, #2025, #7610, #7676).
- followups_open: #8777 — who/what sent the external SIGTERM (a sigterm-witness was armed on both units the same evening; declarative form PR #8786; auditd proper filed as code#8787 to ride the switch — per the 2026-11-11 main-chat record).

#### P2.3 Decisions ledger — pending DESIGN rulings still awaiting Drew (records-pass §3)
Tracked in the ledger (NOT never-issued), listed so the register is complete. State: ledger read 2026-11-11.
| ledger id | subject |
|---|---|
| fleet-tiers-2230 | Fleet management tiers (#2230): which candidates get FULL/FULL-lite management vs umbrella-only? |
| lifecycle-verdict-2311 | Lifecycle-protocol verdict amendment (#2311): approve the draft amendment for adoption? |
| replica-operator-806 | Replica operator interface (#806 Phase 5): A/B/C policy ruling? |
| design-adoption-1678 | Design adoption (opencode#1678): adopt, amend, or reject the design in PR #2032? |
| watcher-scope-1441 | Watcher scope (openchamber#1441): watcherReady ownership signal + toolInFlight tracking scope? |
| shutdown-delete-1414 | Daemon shutdown semantics (#1414 Phase 3): skip the upstream DELETE and preserve the resumable session, or send DELETE and reinitialize? |
| luna-route-terra-low | Luna route: replace with Terra low — only the ChatGPT plan-pair seats, or abolish the whole luna-plan route? (The workhorse seat itself was settled 2026-11-11: back to Luna #high, PR #8735.) |
| atlas-secret-remediation-8479 | Atlas secret-bearing rows (#8479): purge or reclassify the 638 rows in monolith + replica? (see A1.15) |
| backups-provisioning-8584 | Fleet unification slice 6 backups (#8584): provision Cloudflare D1/R2, a mirror host/org, and Turso org access? (see X17) |
| turso-remedy-5468 | Turso remedy (#5468): prune/compact, scope sync, or raise the Turso plan? (see T5) |
| code1394-mimo-flag-superseded | Pool flag code#1394 (MiMo-V2.6-Pro specialist relief): close as superseded, or re-scope under #1342? |
| bws-runner-5451 | BWS live check (#5451): choose/provision a secure credentialed runner for read-only BWS access + SOPS decryption? |
Routine (non-design) pending ledger items at the same read: oc-nix-task-delete-disable, boats-dev-1815, hatch-personal-pr14-1361, mcpx-test-ci-1985, commandcode-refill-1411, mcpx-first-tag-1881, claude-renewal-reauth-8102 (note: the 2026-11-11 18:05 CT gated retry succeeded per the main-chat record; the ledger file still showed it pending at records-pass read time), submodule-worktrees-force-remove.

---

## 4. CLOUDFLARE

### C1. nix-cache — Nix binary cache on Cloudflare (Workers + R2 + KV + D1 + Queues + Cron) (PARENT)
- first_asked: 2026-10-05 — "a good example of a smart use of all the features of workers etc" / "sure build this out in a new repo and continue until you can run nix build against a flake or pkg and it lands and can be pulled down by nix when matching. assume content addressed derivations and set your nix conf accordingly." Drew asked for a showcase build using the full Workers feature set; approved the proposed Nix binary-cache architecture and ordered it built in a new repo until a content-addressed `nix build` round-trips through it.
- tracking: NEVER-ISSUED (searched "nix-cache" in developing-today/code — no matching issue; the developing-today/nix-cache repo has zero issues) — triage **(b) DONE-WITHOUT-ISSUE for the build** (built directly in its own repo), **with one (a) STILL-LIVE follow-up** (retention GC, below).
- design_artifact: repo developing-today/nix-cache (README documents the architecture; Durable Objects deliberately omitted with rationale) + ~/memory/topics/nix-cache.md
- design_status: built-into-running-system
- implementation_status: shipped — built and verified end-to-end the same day: Worker front door, R2 bucket nix-cache-nars, KV INDEX, D1 nix-cache-db, Queue nix-cache-indexing, daily cron orphan sweep; CA demo derivation built → signed → pushed → deleted locally → substituted back from the cache alone in 1.8s. (Still a live Worker in the 2026-11-11 Cloudflare inventory per the records pass.)
- followups_open: **no retention/LRU GC — the cron only sweeps orphans; churned objects accumulate ~30GB/mo (flagged in BENCHMARK.md) → STILL-LIVE, in the manifest.** Also: two 13-byte probe objects (nar/_probe_<ts>) left in the bucket — hand-rolled SigV4 DELETE from the sandbox kept failing; cleanup needs a real S3 client; never recorded as done. And "whatever Drew wants to cache for real" was never decided in the record.

#### C1.1 nix-cache benchmark program — ours vs Cachix vs Attic, speed + cost model
- first_asked: 2026-10-05 (ran into 2026-10-06) — "2 big things and a dozen small things ... compare speed ... cost of 1/10/50/100gb ... 1gb churn": benchmark 2 large + 12 small packages against competing caches, with a cost model at 1/10/50/100 GB and a 1 GB daily-churn scenario.
- tracking: NEVER-ISSUED (work product lives in the nix-cache repo) — triage **(b) DONE-WITHOUT-ISSUE**
- design_artifact: BENCHMARK.md in developing-today/nix-cache (commits cf0521f, ef3921d, 6867ef9)
- design_status: built-into-running-system (delivered analysis)
- implementation_status: shipped — ours $0 at 1–10 GB, ~$0.70 at 50 GB, ~$1.40 at 100 GB vs Cachix/Attic/Garnix tiers; custom-artifact round: ours push 29s/pull 4s (xz), Cachix push 8s/pull 9s (zstd); key finding: `cachix push` silently skips anything already on cache.nixos.org, so stock-package push comparisons are impossible by design.
- followups_open: the Attic comparison never completed — atticd server built and ran, but the attic CLIENT cannot be built here (links nix C++ dev libs unavailable for nix 2.35.2); documented in BENCHMARK.md (commit 30dcbf4), no path forward recorded — a dead end, recorded as such.

#### C1.2 zstd push path for nix-cache (nix-cache-push-zstd.sh)
- first_asked: 2026-10-06 — "yeah zstd": approve closing the push-speed gap by switching the push path from xz to zstd.
- tracking: NEVER-ISSUED — triage **(b) DONE-WITHOUT-ISSUE**
- design_artifact: nix-cache-push-zstd.sh in developing-today/nix-cache (commit ad8cd68)
- design_status: built-into-running-system
- implementation_status: shipped — `nix copy` hardcodes xz, so a custom script (nix-store --dump → zstd -3 → PUT nar + narinfo) was written; same 20 MB artifact: push 8s (ties Cachix), pull 6s (beats Cachix 9s).
- followups_open: none recorded.

#### C1.3 Presigned R2 upload URLs for nix-cache (bypass the 100 MB Worker body limit)
- first_asked: 2026-10-06 (live 2026-10-07) — "yes": Drew approved presigned R2 URLs for ALL objects (not just large ones: one upload path, Worker never touches the bytes) after the 100 MB Free/Pro request-body ceiling was surfaced.
- tracking: NEVER-ISSUED (searched "presign" in developing-today/code — zero results) — triage **(b) DONE-WITHOUT-ISSUE**
- design_artifact: POST /presign/<secret>/nar/<file> in the nix-cache Worker (aws4fetch-signed, commit 5648f3c); push script switched to the presign flow
- design_status: built-into-running-system
- implementation_status: shipped — built, then blocked on Worker secrets; went LIVE and verified end-to-end 2026-10-07 (presign 200 → signed R2 URL → PUT 200) once the R2 S3 credentials were set as Worker secrets.
- followups_open: none (the secret-delivery problem it depended on is C3).

### C2. Cloudflare credential architecture — scoped tokens by purpose + Bitwarden secrets backup
- first_asked: 2026-10-07 — "put all secrets on the disk in there" / "generate creds for a different purpose that just have access to cf Ai gateway and workers Ai for inference, read, etc": back up all on-disk secrets into Bitwarden, and use purpose-scoped Cloudflare credentials instead of one broad key.
- tracking: NEVER-ISSUED (searched "bitwarden secrets backup" in developing-today/code — no matching issue) — triage **(b) DONE-WITHOUT-ISSUE**
- design_artifact: Bitwarden project "developing today" items (cloudflare.api-token-account-wide, cloudflare.api-token-day-to-day, cloudflare.r2-worker-*, cloudflare.api-token-ai-inference, nix-cache.*, cachix.*, …) + idempotent script ~/tools/bws-backup-secrets.sh
- design_status: built-into-running-system
- implementation_status: shipped — account-wide admin token stored but NEVER placed in Worker secrets/env; day-to-day token minted (Workers Scripts/KV/R2/D1/Queues RW + Tail read, account-scoped); dedicated bucket-scoped R2 token for the nix-cache worker (Object RW on nix-cache-nars only, swapped in and re-verified); inference-only token minted (AI Gateway Read+Run, Workers AI Read+Write) and live-tested (embeddings returned); 5 on-disk secrets backed up to Bitwarden, each round-trip sha256-verified; UPLOAD_SECRET rotated consistently across Worker, local file, and Bitwarden.
- followups_open: Drew's plugin question left unanswered in the record (2026-10-07 04:45): "is that cfat key cloudfare workers Ai? I had a plugin but it's not accepting that token" — answered that the cfat key is full-admin and the inference key is the one to use; which plugin + what error still unknown. Also: Drew's broader Bitwarden sharing question (Organizations/Collections, Secrets Manager machine accounts, 2026-10-06) was answered but no decision recorded.

### C3. Credential receiver — a web form that sets Worker secrets without Muse seeing the values
- first_asked: 2026-10-06 — Drew's own idea, per the day-file: "run a localhost-only webserver I control, with username/password fields, get the R2 S3 credentials into it, then set them as Worker secrets" — a mechanism to get the R2 presign secrets into the nix-cache Worker without pasting secret values in chat.
- tracking: NEVER-ISSUED (searched "cred-receiver" — zero results) — triage **(c) SUPERSEDED/DEAD**: the purpose was achieved another way (the C2 token architecture set the secrets); the artifact itself is a paused husk (below).
- design_artifact: ~/tools/cred-receiver.py (localhost attempt) + ~/workspace/cred-receiver/ Vercel deployment (index.html + api/submit.js, project prj_sMkg6nYfeA3HLc1OocBlJH092c5J)
- design_status: drafted
- implementation_status: abandoned-in-record — localhost version FAILED (the managed browser runs on a remote VM; 127.0.0.1 is not the sandbox). Vercel version deployed and made public (SSO disabled), but the browser's vault-fill is domain-scoped so the saved Cloudflare entries couldn't be used on the vercel.app domain; Drew was given secure capture links instead. On 2026-10-07 Drew ordered the app deleted ("use your vercel integration to delete the old app"); the Vercel integration has no project-deletion tool, so it was PAUSED with his approval (both URLs verified 503).
- followups_open: the Vercel project still EXISTS (paused, not deleted) — full deletion requires Drew in the Vercel dashboard (project → Settings → Advanced → Delete Project). Never recorded as deleted. A Drew-only act; noted here, not in the filing manifest.

### C4. AI Gateway "opencode-gateway"
- first_asked: 2026-10-07 — Drew created the gateway himself in the Cloudflare dashboard, then asked: "what's the ai gateway name BTW?" — he stood up an AI Gateway named opencode-gateway for the fleet's inference traffic and confirmed the setup with Muse.
- tracking: NEVER-ISSUED (searched "AI Gateway opencode" — no matching issue) — triage **(b) DONE-WITHOUT-ISSUE** (dashboard object, exists by Drew's own act)
- design_artifact: none found (dashboard object; the inference-scoped token that serves it is recorded in C2)
- design_status: built-into-running-system
- implementation_status: shipped — created by Drew 2026-10-07 04:25 CDT; verified the same morning that the inference token (cloudflare.api-token-ai-inference) works with it as-is.
- followups_open: none recorded in-slice. (The standing fleet rule that AI Gateway is NOT a general inference path — and its one narrow exception — is C5.)

### C5. Grounded search plane + narrow AI Gateway exception
- first_asked: 2026-11-06 — Drew granted (👍) a NARROW AI Gateway exception: AI Gateway allowed ONLY as transport for the grounded search plane's Web Search API traffic — no inference, no other traffic; expansion needs a new ruling.
- tracking: NEVER-ISSUED (searched "grounded search plane"; "web search") — triage **(a) STILL-LIVE**: the ruling stands, the build is blocked on a billing choice that is Drew's to make, and nothing tracks it.
- design_artifact: ~/memory/topics/ai-gateway-search-exception-2026-11-06.md (the ruling record). (The idea-engine's own "Grounded search plane for agents" entry, 2026-11-05, is in Appendix A.)
- design_status: decided (ruling)
- implementation_status: not started — BLOCKED on billing: bake-off ran $0 spent: AI Gateway credit balance 0, HTTP 402 web_search_payment_required; replacement token vaulted as cloudflare.api-token-web-search.
- followups_open: awaiting Drew's choice — load ~$10 credits vs BYOK vs hold → in the manifest.

### C6. Registrar / DNS position (ruling on the registrar-consolidation proposal)
- first_asked: 2026-11-06 01:10 CST — Drew ruled on the "Registrar consolidation plane" proposal (the proposal itself was idea-engine-generated, not Drew-asked; Drew's ruling is the in-scope input): NO wholesale move to Cloudflare Registrar (single-provider lock-in); Porkbun as registrar is a DELIBERATE layer between us and Cloudflare — keep for the bulk of domains; some domains may move DNS hosting to Cloudflare; a subset may be fully transferred where it makes sense; final per-domain picks are Drew's, from a prepared classification.
- tracking: NEVER-ISSUED (searched "registrar consolidation") — triage **(a) STILL-LIVE**: the ruling stands and the deliverable it requires (the per-domain classification for Drew's picks) is not recorded as delivered.
- design_artifact: none found
- design_status: decided (ruling)
- implementation_status: not started (classification owed)
- followups_open: prepare the per-domain classification for Drew's picks → in the manifest. Related standing machinery: the DNS register (code#902 pipeline; register idea's first inventory pass completed 2026-10-25 per the records pass).

### C7. Lab cluster first Cloudflare Tunnel
- first_asked: 2026-11-06 — Drew approved (overnight priorities item 5, 2026-11-06 00:28 CST): "create it; use CF versions and/or nix flake for deploy/service" — the lab cluster's first Cloudflare Tunnel.
- tracking: developing-today/code#6455 CLOSED — "impl(cluster): first lab Cloudflare Tunnel — connector on amd, lab.security.cab seed ingress"
- design_status: built-into-running-system
- implementation_status: shipped — the tunnel is LIVE per the 2026-11-06 record (correcting a 2026-11-05 "zero tunnels" prep note).
- followups_open: none recorded. (Cluster parent: X5.)

### C8. Cloudflare Artifacts per-session workspaces (idea Drew probed, not an ask)
- first_asked: 2026-10-29 — an idea-engine delivery Drew actively probed ("how is that not just R2"; "won't all the code be remote / have to pull locally to edit or build?"): per-session agent workspaces as Cloudflare Artifacts git repos — each session forks a pinned base commit at the edge, the winner is exported back to GitHub as patch/PR, sibling forks reaped after 7 days; registry in Turso.
- tracking: NEVER-ISSUED (searched "cloudflare artifacts session workspaces") — triage: **idea-only, never an approved ask** — register-only, no filing proposed.
- design_artifact: the idea-engine log entry only (Appendix A, 2026-10-29)
- design_status: drafted (idea)
- implementation_status: not started — verdict delivered: a MAYBE (editing/building still happens locally; real benefits are narrow: near-free instant server-side forks, hard per-session isolation, relief from GitHub org clutter/rate limits); pilot (one repo, two diverging forks, export winner) NOT approved or started.
- followups_open: pilot decision rests with Drew; worktrees are holding per the delivered assessment.

### C9. Dual-exposure design — Tailscale Serve ↔ private Cloudflare Tunnel (borderline, placed here by synthesis)
- first_asked: confirmed to Drew 2026-10-21 (recorded in the 2026-10-22 day-file; slice 5 flagged it as dated just before its window and asked synthesis to place it): a declarative design direction for dual exposure of services via Tailscale Serve and a private Cloudflare Tunnel.
- tracking: no issue cited in the record
- design_status: drafted (design direction confirmed to Drew)
- implementation_status: not established in the record
- followups_open: none recorded. Placed in Cloudflare by synthesis per slice 5's flag; its substance overlaps C7 and the cluster program (X5).

Cross-references: Ask-the-ledger (A1.14) is Atlas × Cloudflare AI Search; PM-stack Phase 2 (P1.12, #8569) is the PM stack's Cloudflare migration; the idea engine (T3) generates Cloudflare designs daily — Appendix A.

---

## 5. TURSO

### T1. Turso coordination schema in muse-meta (fleet queue ancestor)
- first_asked: 2026-10-20 — recorded as "Coordination schema v1 created IN muse-meta (per Drew: one main DB for now, no per-topic split)": a single Turso DB (muse-meta) holding the fleet coordination tables — coord_workers, coord_tasks (queue + dedupe_key), coord_leases, coord_events (append-only), coord_checkpoints — seeded with his 4-worker model (sentinel/rnd/engineering/finance).
- tracking: NEVER-ISSUED (searched "coordination schema turso" — only Nov-era descendants code#5005/#5690, not this build) — triage **(b) DONE-WITHOUT-ISSUE**
- design_artifact: schema live in Turso DB muse-meta + ~/tools/turso-exec.py helper; plan research docs in the turso-atlas docs set (turso-plans-free-vs-developer.md, d1-vs-turso.md, turso-database-rust-rewrite.md)
- design_status: built-into-running-system
- implementation_status: shipped + verified same evening (first task row logged done; verified via pipeline SELECTs; `turso db shell` works).
- followups_open: the Phase 1 coordination cutover was left awaiting Drew's pick — Turso coord schema (this entry) vs the Supabase agent schema (X8) as the one coordination home. The fleet's later queue reality (repo-queue broker over Turso, F4.11/F4.8) grew from this line, but the formal pick is not recorded.

### T2. Turso temporary-surplus designs (better Turso use + one surplus month)
- first_asked: 2026-10-25 — "do some design work on ways to better use turso and especially... temporarily having more turso for a month": design ways to exploit a temporary higher Turso tier so the quota converts to durable artifacts/measurements/answers/procedures before downgrade.
- tracking: NEVER-ISSUED (searched "turso temporary surplus") — triage **(c) SUPERSEDED/DEAD as a program**: the surplus window it was designed for was the Atlas Pro month (A1); Part A's everyday patterns read as standing practice notes, but no B-design was ever picked up as a build and the month-frame has passed. Register-only.
- design_artifact: ~/hatch-personal/workspace/turso-atlas/docs/temporary-surplus-designs.md (also uploaded to Google Drive Cloud/atlas)
- design_status: drafted — Part A everyday patterns (coordination/job queues, per-machine findings DBs, backup catalog, telemetry, config DB, CI scratch DBs, Workers-fronted read APIs) + Part B surplus-month designs B1–B10; import-metering question settled alongside (imports meter ~0 rows/storage).
- implementation_status: not started (as designs)
- followups_open: the week-1 support-ticket harvest (5 questions) and dashboard confirmation of import metering during a Pro month were designed but not recorded as done.

### T3. Turso + Cloudflare idea engine — 131 dated ideas (PARENT)
- first_asked: 2026-10-20 (directive recorded in the cluster-goal setup): a standing engine that produces ONE concrete buildable idea per run using the Turso $5 plan and/or Cloudflare $5-tier features within Drew's budget rules (free tiers first; $5 plans in play; $25–30 "maybe later"; $75 OUT), and each run also advances a standing ledger by one small rotating step — (a) check one service's usage/limits/spend, (b) find one new free/cheap compute/storage/egress offer, (c) note hits/geo/latency for a property. Later the same day: "Idea-engine cron doubled (Drew's instruction): interval 8h → 4h."
- tracking: NEVER-ISSUED (implemented as cron `turso-cloudflare-idea-engine` under the lab-cluster goal; no GitHub issue) — the engine itself is triage **(b) DONE-WITHOUT-ISSUE** (it exists and runs); its OUTPUT is the register problem below.
- design_artifact: cron task body + ~/workspace/goals/stand-up-lab-cluster-talm-talos-cozystack-postgres-rustfs/files/idea-register-2026-10-20.md + hidden_files/cloud-ideas-log.md
- design_status: built-into-running-system
- implementation_status: shipped — cron created 2026-10-20 (8h, then 4h at Drew's instruction); first idea (BMC/hardware-audit snapshot CDN) delivered 15:01 CDT the same day; still running at window end (131st idea dated 2026-11-11).
- **The finding (records pass, verified):** the ideas log holds **131 dated idea entries (2026-10-20 → 2026-11-11)** plus 131 rotating ledger-step entries — and **NEITHER log records pickups**: the register contains zero `issues/` links; the ideas log contains no pickup/implemented markers. The only status notes anywhere are register-side: (1) the DNS/domain register idea — first inventory pass completed 2026-10-25, recurring machinery filed as code#902; (2) the 2026-11-04 brain-dump explosion rulings + investigation addendum. Cross-check note from the records pass: several early ideas visibly exist as live Workers in the 2026-11-11 Cloudflare inventory (agent-inbox, webhook-gateway — cf. the ideas of 2026-10-23) — the logs themselves never say so. **Every idea in Appendix A is therefore "pickup UNVERIFIED in the record" unless a register entry elsewhere says otherwise.**
- Drew rulings on individual ideas recorded in the slices: idea #26 ("one remote MCP server for all lab tools", 2026-10-24) — Drew: "that's what mcpx is built for" (do not re-propose; an edge front door would front mcpx, not replace it); the registrar-consolidation idea (2026-11-06) Drew ruled on substantively → C6; the grounded-search idea (2026-11-05) Drew ruled on → C5; the Artifacts workspaces idea (2026-10-29) Drew probed → C8; Ask-the-ledger (2026-11-05) Drew approved for build → A1.14.
- followups_open: every idea's recorded "first step" remains unstarted unless Drew approves a build — by the engine's design (parked unless Drew 👍s). All 131 titles are preserved in Appendix A, grouped by theme.

### T4. Supabase → Turso migration wave (records-pass reconciliation)
- first_asked: not in the day-slices; reconciled from the records-pass census:
  - developing-today/code#6581 "Supabase → Turso migration — merged wave plan (executes #5690–#5693)"; #5690 agent append streams → Turso; #5691 agent mirror_* analytics (20 tables) → Turso; #5692 sheet-derived staging (19 tables) → Turso; #5693 ci.events webhook ledger → Turso (all open per census).
  - Related open Turso items in the census: #5676 currency scaling design (Redis → multi-Turso → TigerBeetle), #5826 Stage B multi-Turso sharding by currency, #5681 serverless edition storage — D1 vs Turso vs both, #8545 turso_client pipeline bug, #6827 turso CLI rename/conversion gap, #8068 stale-pr-warden repoint to Turso fleet-queue, #6936 github-ci HMAC gate fails open (writes to Turso), #8589 fleet-queue turso_queue.rs split.
- design_status: drafted (wave plan)
- implementation_status: not verified in this pass
- followups_open: read with P1.11 — two data-home migration directions (into Supabase #3142; into Turso #6581) coexist in the open census; the register does not reconcile their ordering.

### T5. Turso remedy ruling (#5468)
- first_asked: decisions-ledger item turso-remedy-5468 (pending at the 2026-11-11 ledger read): prune/compact, scope sync, or raise the Turso plan?
- tracking: developing-today/code#5468 (open per census — "pm-stack(turso) mirror storage surging"); ledger-tracked, awaiting Drew — NOT never-issued.
- design_status: none (ruling pending)
- implementation_status: not started
- followups_open: Drew's pick. Related live figure from the fleet record: Turso storage 45%, rows-read 2% (quota snapshot 2026-11-11).

---

## 6. ADJACENT INFRA

### X1. Nix persistence design — a store that survives sandbox recycles, never-delete monotonic layout
- first_asked: 2026-09-29 — "anything safe turn it on even if we don't need it now" / "if I never want to delete anything can it all work?" Drew set the constraints: enable every safe nix experimental feature proactively (flakes, ca-derivations, recursive-nix, auto-optimise-store) to cut bandwidth/storage/compile time, and make the store persist across sandbox recycles under a never-delete, grow-only model.
- tracking: NEVER-ISSUED (searched `gh issue list -R dezren39/hatch --search "nix"` — no issues; work tracked via dezren39/hatch commits + docs only) — triage **(b) DONE-WITHOUT-ISSUE**
- design_artifact: ~/notes/architecture/nix-persistence.md (+ nix-install-flakes.md, capabilities.md) in dezren39/hatch
- design_status: built-into-running-system
- implementation_status: shipped — designed + built iteratively in-slice: bind-mount design disproven (per-exec mount namespaces), tarball restore proven (full /nix restore in 4.5s, zero network), final in-slice design = /nix symlink → ~/nix-persist (persistent btrfs) with an LD_PRELOAD taint-shim masking the symlink and eliding no-op chowns; boot-init restores/verifies it every boot. (This is the design the whole sandbox still runs on.)
- followups_open: replacing the LD_PRELOAD shim with `allow-symlinked-store = true` + nix's `real` store-dir setting was offered via options widget ("Yes, test dropping the shim") on 2026-09-30 — no answer recorded in-slice; Drew also asked "why would nix only partially support symlinks I think you should do some research" (2026-09-30) — research delivered (allow-symlinked-store only skips the LocalStore constructor guard; the rest of nix assumes canonical paths). The shim question is a Drew-answer residue, not a build ask — noted, not manifested.

### X2. Sol's six-point nix simplification plan — implement the full plan
- first_asked: 2026-09-30 — "Yes, implement the full plan": Drew pasted Sol's six-point review of the nix setup (drop the shim's symlink spoofing for `allow-symlinked-store`; split bootstrap vs package profiles; cheap/quiet bash startup with LD_PRELOAD scoped to the nix wrapper; one resolver/dispatcher with GC roots, pinned omnibin, no silent version substitution; on-demand recovery next to the cron, with repair lock + recursion guard; test empty `build-users-group =`) and approved implementing all of it over a smaller simplification pass.
- tracking: NEVER-ISSUED (dezren39/hatch commit 9729491 + architecture docs) — triage **(b) DONE-WITHOUT-ISSUE**
- design_artifact: dezren39/hatch commit 9729491 "Sol's nix plan steps 1-5: bootstrap split, dispatcher, on-demand repair"; updated notes/architecture docs + TOOLS.md
- design_status: built-into-running-system
- implementation_status: shipped — implementation COMPLETE 2026-09-30 ~05:58 CDT, verified (dispatcher resolves cowsay@3.8.4 in a fresh shell; simulated breakage repaired in 318ms). One verdict reversed during implementation: Sol's point 1 was wrong on deeper testing — `allow-symlinked-store` only bypasses the store-open guard, so the taint-shim stays (documented in the shim header).
- followups_open: none recorded (an installer incident during implementation — /bin/bash.real clobbered — was recovered in ~2 min and the installer hardened, same entry).

### X3. Nix consolidation follow-ons — one flake profile, omnibin as a flake input, nixbld traverse grant
- first_asked: 2026-09-30 ~06:09 CDT — "merge down profile etc etc so we only have the flake install profile plus the omni bin flake": a burst of follow-on directives — grant the nixbld group traverse-only access to /home/hatch and restore nixbld builds if a genuine nixbld-user build verifies; merge the bootstrap profile into the single default flake-install profile; make omnibin an input of the pin flake with our nixpkgs following omnibin's nixpkgs (for their cache hits) and switch the dispatcher run command to the local pin flake.
- tracking: NEVER-ISSUED — triage: **register-only, status unknown** (no completion record exists to classify it (a), (b), or (c) honestly — see followups).
- design_artifact: none found (execution task under the nix program; ~/tools/dispatcher.sh + pin flake are the artifacts)
- design_status: decided (directives)
- implementation_status: not recorded — launched in-slice (coordinator subagent spawned ~06:09 CDT; still RUNNING at the last in-slice record, with test results and a commit hash explicitly owed on completion). Completion is not recorded inside the window, and no later slice picks it up.
- followups_open: the owed completion report (nixbld build verification result, profile merge, flake-input switch, commit hash). Flagged in Appendix B as a record gap rather than manifested as still-live: the register cannot prove the work wasn't done.

### X4. Comprehensive environment knowledge base — "review everything you know… build more notes/"
- first_asked: 2026-09-30 — "thanks now please review everything you know have learned or explained to me across our time together. build more notes/ until you have a comprehensive store of what does and doesn't work, how to install nix with flakes, what it takes to make nix persist, what omnibin is and does, link to flake url, website, blog, etc from researching it. add known gaps in our ifnra improvements, break down each set of files and folders that came with the home dir…" — a full knowledge-base build of the agent's environment.
- tracking: NEVER-ISSUED (direct build, delivered as repo commits) — triage **(b) DONE-WITHOUT-ISSUE**
- design_artifact: notes/architecture/ in dezren39/hatch — 7 new docs (13 total): nix-install-flakes.md, omnibin.md, capabilities.md, system-profile.md, home-dir-inventory.md, disk-inventory.md, infra-gaps.md; commit 57efbb6; full find listings (683,465 entries) in notes/disk-listings/, gitignored, also pushed to Google Drive at Drew's follow-up request.
- design_status: built-into-running-system
- implementation_status: shipped — delivered same morning (2026-09-30 ~05:13 CDT), committed and pushed.
- followups_open: cleanup candidates identified but deliberately not deleted pending Drew's word: `.nix-portable/` (887M, superseded) and `tools/nix-portable/` (65M) — the nix tarballs he did order deleted were removed the same morning; these two were still awaiting his decision at slice end.

### X5. Lab cluster program — Talm / Talos / Cozystack / Postgres / RustFS (PARENT)
- first_asked: 2026-09-29 — "separately to look into standing up the lab cluster talm talos cozystack postgres rustfs" (explicitly a research prompt at first, with three reminders); massively expanded 2026-10-20.
- tracking: goal `stand-up-lab-cluster-talm-talos-cozystack-postgres-rustfs` (goal_ba17a5777c21) — the goal also owns the idea register/engine (T3), the DNS register, and the cold-store R2 design.
- design_status: decided (program)
- implementation_status: partial (program in progress across the whole window)

#### X5.1 Lab cluster stand-up research + reminders
- first_asked: 2026-09-29 (above); reminder series delivered 2026-10-01/02 in-slice with no new direction recorded.
- tracking: NEVER-ISSUED in-slice (searched "lab cluster talos cozystack" — the cluster epics it returned, code#1817 created 2026-10-28 and code#1086 created 2026-10-27, were both filed weeks AFTER the ask; in-slice it lived only as a goal + three runonce reminder crons) — triage **(b) DONE-WITHOUT-ISSUE** (the research container existed; the program was later issued properly — X5.2/X5.3).
- design_status: none in-slice (research not yet done; the goal is the container)
- implementation_status: parked-as-research in-slice.
- followups_open: Drew's "remind me once more next Friday morning" (Fri 2026-10-09) was never scheduled — the assistant asked whether it covers both research topics or just the lab cluster and got no reply in-slice.

#### X5.2 Cluster program restart — Talos/Cozystack brain-dump (epic + 7 child designs)
- first_asked: 2026-10-20 — recorded as "Drew's program brain-dump + role directive (06:59–07:04 CDT) — cluster program restarted, scope massively expanded": ~12 Threadripper machines, TALM-harvested declarative per-node Talos configs, CNPG Postgres, RustFS storage tiers (RAID10 HDD + NVMe tiers + DRBD/LINSTOR), Cloudflare tunnel, Tailscale, NATS, FluxCD running "the ID app"; hard constraints: no HashiCorp anything, everything bootstrappable/declarative; egress is scarce (2 Gbps fiber) so bulk bandwidth shunts to Cloudflare/Turso; cloud budget rules — Turso $5 + Cloudflare $5 Workers in play, $25–30 later maybe, $75 OUT.
- tracking: dezren39/nix#508 EPIC + children #501 (hardware inventory/stats), #502 (BMC/BIOS audit + KVM screenshot verification), #503 (TALM declarative disk config), #504 (storage pools), #505 (egress strategy), #506 (boats.dev research), #507 (OCI free-tier maximization) — #501 and #508 verified CLOSED in slice 4; children cited from the day-file record.
- design_artifact: ~/workspace/goals/stand-up-lab-cluster-talm-talos-cozystack-postgres-rustfs/files/idea-register-2026-10-20.md (facts / constraints / designs / tasks / open questions, + addenda); OCI free plan doc (goal files, 2026-10-20, records pass §1b).
- design_status: drafted — register written and delivered, all 8 issues filed and re-verified OPEN the same morning; the tracked item was reopened with the expanded scope.
- implementation_status: partial (program)
- followups_open: open questions left with Drew in the register: which app is "the ID app" FluxCD runs first; OCI account email/card; boats.dev context; exact NIC spec ("at least 200 Gb ports" as dictated, unconfirmed); Turso account/DB state.

#### X5.3 Basement-cluster project-structure spec + seeding
- first_asked: 2026-10-26 — Drew's correction + directive: the old v1.9-era Talos cluster was only a test and is torn down; every project needs a standard structure (standard home for description + dated status reports, high-level roadmap items/subcomponents, many detailed labeled cross-linked GitHub issues under them; cluster issues must state execution mode — online / hybrid / physical — explicitly; plus a complexity-weighted visualization). Then: Drew approved "Seed this for the cluster now."
- tracking: developing-today/code#1080–#1093 — 12 epics filed (inventory, firmware/BMC, power, network, storage, TALM+Talos, Cozystack, platform services, observability/backup, cloud extension, first workload, ESP32 wing); #1080 **[live-checked 2026-11-11: OPEN]**; detailed tasks #1094–#1106 as sub-issues; #1108 OPEN (project page + complexity-weighted visualization); program epic #1817 **[live-checked: OPEN]** — "EPIC: Basement Talos/Cozystack cluster program".
- design_artifact: goal workspace files/project-structure-spec-2026-10-26.md; planning/roadmaps/basement-cluster.md via PR #1082 (merged) + #1109 (merged)
- design_status: decided + seeded same evening (labels mode/online|hybrid|physical created on all four repos; all 12 epics seated in Project #4, Area=lab-cluster)
- implementation_status: partial (program)
- followups_open: PENDING Drew's decision at flush — close the overlapping older dezren39/nix#501–#508 filings as superseded (offered via options widget, unanswered). Note for synthesis: slice 4 verified #501 and #508 CLOSED, so that question was at least partly overtaken; the remaining children were not re-verified in this pass. Grist "Program roadmap" 6 rows to reconcile 1:1 with the 12 epics (#1108/#1144).

### X6. Cold-store backup design + nightly run
- first_asked: design doc dated 2026-10-25 (32 KB, verified present in the 2026-10-26 workspace recheck alongside the DNS register and usage-hub design); the system runs as the cold-store nightly backup over Turso + Supabase + D1 with Drive filing.
- tracking: NEVER-ISSUED (searched "cold store backup design" — no matching issue; the nightly's failures were fixed in place) — triage **(b) DONE-WITHOUT-ISSUE for the system**, with one **(a) STILL-LIVE filing debt** (below).
- design_artifact: cold-store design doc among the goal/project files + cold-store-r2-layout-design-2026-10-25.md (lab-cluster goal files, records pass §1b)
- design_status: built-into-running-system
- implementation_status: shipped / operating — nightly run FAILED at the Turso step 2026-10-27 (atlas DB ~1.13 GB dump hit caps; fix chain PRs #1373/#1383: compressed streaming rebuild + repair filter for libsql's malformed /dump schema rows) and ran "Turso full-house" clean 2026-10-28 with a Supabase S3 gap found and closed.
- followups_open: the permanent streaming repair filter in export-turso.sh was "in flight" at the Oct 27 record; **the upstream-quirk issue for libsql's malformed /dump schema rows was to be filed in developing-today/code and never was → STILL-LIVE filing debt, in the manifest** (small, but exactly the class this audit exists to catch).

### X7. Supabase DIY backups + infra doctrine + $30 ceiling
- first_asked: recorded decisions 2026-10-21 ~01:46 CDT (Supabase side chat): Supabase spend capped — "$0 Free, $25 Pro, or $30 total in practice"; backups are DIY because Free has none — pull from Postgres once or twice a day (configurable), content addressing with the hash embedded in the file location so identical files dedupe, stream to an S3-compatible endpoint without touching disk, run on Drew's laptop (Cloudflare Worker a possible alternative); doctrine: everything built at least with a script, ideally declarative and idempotent, do NOT use Terraform unless absolutely necessary; and an ongoing private architecture/roadmap doc.
- tracking: NEVER-ISSUED (tracked as goal_a2d5596827e1 "Supabase muse-dev platform & backups", no GitHub issue) — triage **(b) DONE-WITHOUT-ISSUE**
- design_artifact: ~/hatch-personal/workspace/supabase/ARCHITECTURE.md + ~/workspace/supabase/muse-dev/backup/ (supabase_backup.py, config.json, README.md, restore_api.py)
- design_status: built-into-running-system
- implementation_status: shipped + verified the same morning — R2 bucket `supabase-backups` created, pipeline streams zstd → SHA-256 → content-addressed key `pg/muse-dev/<yyyy>/<mm>/<sha256>.<kind>.<ext>` with manifest; restore test regenerated exactly 4,362 INSERTs matching export row count; cron `supabase-muse-dev-backup` every 12h.
- followups_open: API-transport dedupe is best-effort until deterministic ORDER BY is added; backups must not run concurrently (manifest is read-modify-write); full-fidelity pg_dump transport only works from Drew's laptop/normal networks (sandbox egress proxy blocks the pooler port).

### X8. Supabase agent schema + coordination platform (PM-stack ancestor)
- first_asked: built 2026-10-20 (Supabase side chat) from Drew's program direction the same evening as the Turso schema (T1): one Free project, separable schemas — `agent` (workers/tasks/leases/events/checkpoints; claim_task = SELECT FOR UPDATE SKIP LOCKED + lease + event, proven live), `ci` (GitHub CI events + recent_failures view, fed by a real webhook on developing-today/code), `ideas` (mirror of the idea registers), `ainventory` (benchmark workbook import).
- tracking: NEVER-ISSUED (tracked later as goal supabase-muse-dev-platform-backups; no GitHub issue in-slice) — triage **(b) DONE-WITHOUT-ISSUE**
- design_artifact: ~/workspace/supabase/muse-dev/migrations/0001–0005 + ~/hatch-personal/workspace/supabase/ARCHITECTURE.md; event-log design + parity docs in the goal files (2026-10-26/27, records pass §1b)
- design_status: built-into-running-system
- implementation_status: shipped + verified (claim_task proven live; Edge Function `github-ci` deployed, HMAC verified both directions; morning readout cron reads ci.recent_failures daily).
- followups_open: "Phase 1 coordination cutover still awaiting Drew's pick" (Turso coord schema vs Supabase agent schema as the one coordination home — same open pick as T1); designs in flight listed in ARCHITECTURE.md: agent.events pruning, per-worker credentials, pg_cron expire_leases. Later Supabase-platform items in the census: the muse-dev → muse-dev2 cutover miss that produced P1.6's #5129/#5120.

### X9. CI program (2026-10-18) — replace nix in GHA, caching gaps, Jules consolidation, concurrency groups, Blacksmith dual-track
- first_asked: 2026-10-18 — from Drew's 7 follow-up questions after the Actions audit: "make issues to replace nix everywhere in GHA" (stop building through nix in CI — the same code compiled twice: ~5.4 min of a 10.6 min mcpx run; use direct toolchains instead, keep flakes for local dev); plus the filed caching findings; plus concurrency groups (Drew approved adding them everywhere applicable: cancel-in-progress for PR/push CI, group-only for scheduled/bot workflows, release/publish/deploy skipped, all four repos); plus Blacksmith: "sign up for blacksmith using my credentials and wire it into my github however needed" and, superseding the either/or: "move opencode to github runners too though comment out the blacksmith parts and make clear comments marking which parts are only needed for setup in GitHub Actions default runners. We may later switch back or need to be able to `if/else` based on the runner that's used."
- tracking: developing-today/code#286 + developing-today-forks/opencode#49 + dezren39/nix#398 (replace nix in GHA — all verified CLOSED in slice 4); code#288 (CI caching gaps: no cachix/magic-nix-cache anywhere, Go build cache not cached, no artifact reuse between legs; acceptance: actions/cache keyed on go.sum content hashes — verified CLOSED); code#287 (Jules workflow consolidation — verified CLOSED); concurrency groups landed as PRs code#289, openchamber#58, opencode#50, nix#399 (all recorded MERGED 2026-10-18); Blacksmith: opencode#46 verified CLOSED ("CI: jobs target Blacksmith runners that never pick up — queue grows unbounded") + decision comment 6250974400 on it.
- design_status: decided
- implementation_status: shipped — "Concurrency rollout COMPLETE (2026-10-18 ~08:50 CDT)"; all three nix-in-GHA issues closed; Blacksmith signup completed later that morning (~09:57 CDT) and the dual-track design (GitHub-hosted runners with Blacksmith one comment-swap away, `# RUNNER-SWITCH:` markers) was the recorded decision; the runner-migration subagent was still RUNNING at the flush that recorded the dual-track order.
- followups_open: at first recording the signup was blocked on Drew's GitHub authenticator 2FA code (he confirmed he'd send it) — resolved same morning per the record.

### X10. Globalping account + outside-in availability watch
- first_asked: 2026-10-23 ~18:32 CDT — Drew ordered the Globalping account and a routine-ping watch: GitHub SSO sign-in, API token filed, hourly HTTP/ping probes of the fleet's public pages, silent unless something is down.
- tracking: NEVER-ISSUED (no issue cited in the record) — triage **(b) DONE-WITHOUT-ISSUE** (implemented directly as the globalping-watch cron; effectively cloud idea #19 — outside-in watchdog — with Globalping as probe network; cf. Appendix A 2026-10-23).
- design_status: built-into-running-system
- implementation_status: shipped — account DONE + hourly watch live 2026-10-23 ~18:41 CDT; still running at window end.
- followups_open: none recorded.

### X11. Account connection campaign — every service signed in on Muse's machine
- first_asked: 2026-10-20 — recorded rules: "hunt down every account; API keys where possible, OAuth wherever possible; NO new MFA unless a service forces it; every token Muse creates gets filed in Bitwarden under a clear name" — a campaign to get every service signed in/connected on this machine, then repeat the set on his laptop.
- tracking: NEVER-ISSUED (tracked as goal account-connection-campaign (goal_6f48d745f802), no GitHub issue) — triage **(b) DONE-WITHOUT-ISSUE as a vehicle** (the campaign is goal-tracked by design; per-account completions are the checklist's job).
- design_artifact: ~/workspace/goals/account-connection-campaign-every-service-signed-in-on-muse-s-machine/hidden_files/accounts-checklist.md (17 accounts w/ auth route + status)
- design_status: built-into-running-system (standing campaign)
- implementation_status: partial / standing — campaign opened 07:51 CDT with a 6h nudge cron surfacing one Drew-needed item at a time.
- followups_open: per-account completions tracked in the checklist (e.g. Plaid beyond Royal Credit Union stayed Drew-only in his own browser at record time; the November record shows Plaid verified fully linked 2026-11-08, 4 institutions / 13 accounts).

### X12. OpenCode server stood up on this sandbox (7-part request)
- first_asked: 2026-10-20 ~06:17 CDT — recorded as "OpenCode server setup on this sandbox (Drew's 7-part request)": stand up opencode serve as a boot-enabled service on the sandbox with his full config, a secrets/env bridge, sops identity, and provider auth sorted. (Log paraphrases the request; no verbatim quote recorded.)
- tracking: NEVER-ISSUED (executed directly, no GitHub issue cited) — triage **(b) DONE-WITHOUT-ISSUE**, implementation shipped-then-superseded.
- design_artifact: ~/tools/opencode.service + ~/tools/install-opencode-service.sh + TOOLS.md "opencode server on this sandbox" section
- design_status: built-into-running-system
- implementation_status: shipped-then-superseded — service live and verified end-to-end the same morning (session + message answered in ~5s; handed-off 3-step task completed in 13s); later superseded by the one-shared-serve consolidation on the laptop (F3.2) — the sandbox server is retired in the current fleet record.
- followups_open: provider auth boundary left open at build time: only the 3 Cloudflare providers credentialed on the box; claude-code/openai-codex/google/antigravity awaited Drew's choice (per-provider `opencode auth login` vs copy auth.json from his Mac).

### X13. Late-night shift standing rule — silence no longer means no work
- first_asked: 2026-10-25 17:39 CDT — "you should just build something as long as it's reversible": no reply to the nightly priorities list no longer means no overnight work; Muse starts the top-ranked fully-reversible candidates within the $0 cap; irreversible, spending, or Drew-reserved items wait for morning.
- tracking: NEVER-ISSUED — by design: the rule is encoded in the late-night-shift-priorities job body (step 6), not tracked as an issue — triage **(b) DONE-WITHOUT-ISSUE**
- design_artifact: the cron job definition; also recorded in ~/memory/topics/memory-full-text-2026-10-29.md
- design_status: built-into-running-system
- implementation_status: shipped — adopted and operating (invoked by later overnight runs, e.g. the 2026-11-05 run in the wider record).
- followups_open: none.

### X14. Subscription / model tracker
- first_asked: 2026-10-20 — recorded as delegated from Drew's morning requests (~06:17–06:47 CDT): a tracker doc of his AI subscriptions/models — renewals, costs, limits (the standing ledger the idea engine's rotating steps feed).
- tracking: NEVER-ISSUED (a workspace doc, no GitHub issue) — triage **(b) DONE-WITHOUT-ISSUE**
- design_artifact: ~/workspace/ai-subscriptions/tracker-draft.md
- design_status: built-into-running-system
- implementation_status: shipped — "Subscription/model tracker — COMPLETED and delivered (~06:35 CDT)" and expanded ~07:47 CDT.
- followups_open: none recorded.

### X15. Coordinated switch — spec, machinery, and the still-owed proven switch
- first_asked: spec 2026-11-10 15:11 CST — Drew's spec: a switch with the SAME code must not restart anything (same pid/uptime, verifiably); a switch WITH changes should hand ports over and keep sessions alive; "run a switch if it's ready." Drew restated the principle on 2026-11-11 after the PM-stack SIGTERM incident: a reset should transfer ports, never a bare kill.
- tracking: **no titled issue carries the coordinated switch** (records-pass census, verified: the "switch" title search finds only unrelated matches) — the switch is tracked in the stewardship side chat + switch-carrying PRs. Carrying/fixed items from the slice record: developing-today/code#8264 **[live-checked: CLOSED]** — verified title "Prod service started by its own unit runs a stale binary (3497ea2) baked into the runtime config snapshot — unit ExecStart ignored" (i.e. the pin-reconciliation item, now closed); #8255 (incumbent-defer fix), #8261 (shadow-supervisor respawn), #8729 (opencode.service ExecStart after the GC'd-binary failure), plus what the switch carries: board fix (PRs #8344/#8343/#8338), Open Design limiter (fork #2264 + pin #8387), quota gatherer, _claude renewal pin #8559 (fork #2270), broker producer #8598, watchdog ids #8727, pm-projector socket activation, #8729 + stopgap drop-in deletion + gcroot. Auditd-for-signal-attribution: code#8787 (filed 2026-11-11, to ride the switch); sigterm-witness declarative form: PR #8786.
- design_artifact: the switch machinery itself (socket activation adoption, shadow supervisor) + the stewardship goal's records
- design_status: decided (spec)
- implementation_status: partial — per the record at window end the coordinated switch was still pending (steward side chat owns it; last word ~06:25 CST 2026-11-11: build retrying after an atlas-search test failure at final assembly). Standing caution in the record: do NOT restart queue-broker until the deployed binary contains BOTH 6a792f60 (#8548) AND aba44429 (#8561).
- followups_open: one proven real switch still owed at window end; #8260 closing check + fork #2253 close ride it.

### X16. Build reuse program — why do builds start from scratch
- first_asked: 2026-11-11 — Drew pressed on why fleet cargo/nix builds keep starting from scratch instead of reusing prior build outputs; turned into a measured build-performance program.
- tracking: developing-today/code#8392 OPEN (per slice 6) — "Build performance: fleet cargo builds pile up — lock orphaned, per-worktree target dirs, GNU ld storm (measured 2026-11-10)"
- design_artifact: issue #8392 measurements
- design_status: drafted (measured)
- implementation_status: partial — fix landed: `~/tools/fleet-build-lock` per-package CARGO_TARGET_DIR; PR #8400 merged in-slice (build-reuse DONE per the wider record: atlas-search cold 38s → 1s); follow-on items filed #8573/#8574.
- followups_open: #8392 remains OPEN; #8573/#8574.

### X17. Fleet backups provisioning ask (#8584)
- first_asked: ledger item backups-provisioning-8584, surfaced to Drew 2026-11-11 04:33 CST: provision Cloudflare D1/R2, a mirror host/org, and Turso org access for fleet backups — or keep it parked.
- tracking: developing-today/code#8584 **[live-checked 2026-11-11: OPEN]** — "Fleet unification slice 6: backups — Turso into D1, repos to Cloudflare plus mirror plus R2" (slice 6 of F4.15; scope ruled by Drew on #8575: all three backup forms).
- design_status: decided (scope) / provisioning pending
- implementation_status: not started — parked, awaiting Drew's provisioning at window end.
- followups_open: Drew's provisioning decisions (D1/R2, mirror host/org, Turso org access).

### X18. Init.sh fast-path — marker check first + trigger-style Phase 1
- first_asked: 2026-10-05 — "yes all of it": Drew approved implementing both proposed fixes for the ~65s "already initialized" path: (A) move the boot-marker check to the top of init.sh (fast path ~65s → ~3s), (B) make the cron's Phase 1 trigger-style (compare /proc boot_id to the marker in ms; run init.sh only on mismatch).
- tracking: NEVER-ISSUED at the time (searched "init.sh fast" in developing-today/code — no Oct-2026 match). A later related issue, developing-today/code#5717 **[live-checked: CLOSED]** — "Init job duration: ~60s of every ~75s run is a telemetry du of nix-persist that times out and returns 'unknown'" — was filed off Nov-2026 measurements as a follow-on from #5700; it is a different (telemetry-cost) item and its closure does not record the fast-path landing.
- design_artifact: analysis in ~/memory/2026-10-05.md (11:50 CDT entry) — the expensive work (du -sh over nix-persist, 4 HTTPS round trips to cache.nixos.org) runs BEFORE the marker check.
- design_status: decided (approved)
- implementation_status: not started — approved but NOT DONE in the record: the day-file marks it "NOT YET DONE, interrupted by flush. PENDING". Later evidence agrees it never landed: code#5717's 2026-11-04 measurements still show init.sh costing 68–97s per run, ~60s of it the same nix-persist du timing out.
- triage: **(a) STILL-LIVE** — the clearest approved-but-dropped item in the audit; in the manifest.

### X19. Backups scoping family (records-pass reconciliation)
- first_asked: not in the day-slices; reconciled from the records pass so the register carries them:
  - Goal `hash-tree-manifest-for-backup-change-detection-scoping-done` — hash-tree manifest for backup change detection; scoping done per its title.
  - Goal `backblaze-personal-backup-for-the-framework-laptop` — Backblaze personal backup for the Framework laptop; no status line verified in the records pass.
  - PM-stack backups incident in the census: code#3869 (backups empty day-dirs).
- design_status: drafted (scoping)
- implementation_status: not verified in this pass
- followups_open: read with X6/X7/X17 — backups is the most fragmented domain in the register (cold-store nightly, Supabase DIY backups, fleet-backups provisioning, hash-tree scoping, Backblaze) with no single owning epic in the record.

### X20. Docs migration — hatch-personal / hatch / darwin-nix docs → developing-today/code canonical home — added by gap pass 2026-11-11
- first_asked: 2026-11-04 ~22:59–23:00 CST — Drew's directive: the three repos' accumulated documentation and design material belongs with the production code now that the fleet binaries are productionized; plan + enumeration first, the plan document itself moves nothing.
- domain: adjacent infra / fleet documentation
- tracking: developing-today/code#5983 **[live-checked 2026-11-11: OPEN]** — "Docs migration: hatch-personal/hatch/darwin-nix → code canonical home (tracking)"; batches #5976 (backup scripts & docs), #5978 (fleet designs & machinery), #5980 (architecture & how-it-works), #5981 (Atlas docs sweep), #5982 (darwin-nix non-flake docs); gap issue #5972 (fleet fallback connection mode)
- design_artifact: ~/hatch-personal/workspace/scratch/docs-migration-plan-2026-11-04/plan.md (482 / 475 / 474 tracked files classified across the three repos)
- design_status: decided (directive + filed plan)
- implementation_status: partial — tracked by #5983 and its batches, all open at check time
- gap note: zero hits in the register for #5983 or the migration before this entry.

---

## 7. NEVER-ISSUED TRIAGE — the dedicated section

Every register item whose tracking is NEVER-ISSUED was classified:
- **(a) STILL-LIVE — 9** (7 standalone asks + 2 live follow-ups of shipped systems; the 9th — F4.18 — added by the gap pass 2026-11-11): the filing manifest below.
- **(b) DONE-WITHOUT-ISSUE — 29**: A1.1 Atlas collector + ingest · A1.5 delta publish + classification · A1.6 Atlas migration (with residual verification gap) · A1.7 publish v2 · A1.10 canonical-session rebuild · C1 nix-cache build · C1.1 benchmark program · C1.2 zstd push · C1.3 presigned R2 uploads · C2 Cloudflare credential architecture · C4 AI Gateway opencode-gateway · F2.2 mcpx dedup · F3.1 OpenCode 2 migration (core) · F4.1 sweep/review job family · F4.2 routine-task redesign · T1 Turso coordination schema · T3 idea engine (the vehicle) · X1 nix persistence · X2 Sol's six-point plan · X4 environment knowledge base · X5.1 lab-cluster research container · X6 cold-store system · X7 Supabase DIY backups · X8 Supabase agent schema · X10 Globalping watch · X11 account-connection campaign (vehicle) · X12 sandbox OpenCode server · X13 late-night shift rule · X14 subscription/model tracker. Register-only; no filing needed.
- **(c) SUPERSEDED/DEAD — 5**: A1.2 magic-environment as a standalone build (absorbed piecemeal) · A2 agent-fs/llm-wiki standalone deep-dive (absorbed into Atlas) · T2 Turso temporary-surplus program (its month passed) · C3 credential receiver (purpose achieved another way; Vercel husk awaits Drew's dashboard deletion) · P1.10 orchestration-pass re-enable question (overtaken by events). Register-only, marked closed.
- **Deliberately unclassed — 3**: X3 nix consolidation follow-ons (status unknown — a record gap, not evidence either way); A1 the Atlas program itself (never issued as a program; its pieces are classed individually); F4.17 agent-SDKs investigation (goal-tracked vehicle exists; residue is a Drew-answer).

### STILL-LIVE NEVER-ISSUED — filing manifest (these 9 get filed)

**Filed 2026-11-11** — all 9 are now issues; the filed number rides beside each item below (items 1–8 in the first filing pass; item 9 by the gap pass). Tracking epic: developing-today/code#8805. Register committed to the PM stack as `pm-stack/design-register-2026-11-11.md` (PR #8804).

| # | Proposed issue title | Repo | Register ref | Why it is still live (one line) |
|---|---|---|---|---|
| 1 | init.sh fast-path: boot-marker check first + trigger-style Phase 1 (approved 2026-10-05, never landed) **[filed: developing-today/code#8806]** | developing-today/code | X18 | Drew approved it ("yes all of it"); 2026-11-04 measurements (code#5717) still show init costing 68–97s per run — the audit's clearest approved-but-dropped item. |
| 2 | nix-cache: retention/LRU GC — churned objects accumulate ~30GB/mo, orphan-sweep only **[filed: developing-today/nix-cache#7]** | developing-today/nix-cache | C1 follow-up | The cache is live and growing with no eviction; flagged in BENCHMARK.md, never built. |
| 3 | OpenCode 2 migration follow-through: port the v1 plugins (mcpx bridge, mcpx-session, subtask2) and rebase the fork **[filed: developing-today-forks/opencode#2290]** | developing-today-forks/opencode | F3.1 follow-up | Explicitly left unstarted in the migration record; no later slice records them done. |
| 4 | Grounded search plane: AI Gateway billing decision — load ~$10 credits vs BYOK vs hold **[filed: developing-today/code#8807]** | developing-today/code | C5 | Drew's narrow-exception ruling stands; the build is hard-blocked on HTTP 402 (zero credit balance) until he picks. |
| 5 | Registrar/DNS: per-domain classification for Drew's transfer/DNS picks (Porkbun stays the default layer) **[filed: developing-today/code#8808]** | developing-today/code | C6 | Drew's 2026-11-06 ruling requires a prepared classification for his per-domain picks; it is not recorded as delivered. |
| 6 | mcpx parity: GitHub / Atlassian / Datadog MCP server logins (Drew-gated OAuth/PAT) **[filed: developing-today/code#8809]** | developing-today/code | F2.6 | The standing gate in every mcpx watch run through 2026-10-11; tracked nowhere but watch text. |
| 7 | One-command local agent CLI via this VM's mcpx — needs Drew's OS / call-method / auth answers **[filed: developing-today/code#8810]** | developing-today/code | F3.6 | Drew accepted the idea; the premise check rerouted the design (Mac gateway unreachable, VM mcpx works) and the three setup answers were never recorded. |
| 8 | Cold-store: file the libsql /dump malformed-schema-rows quirk upstream (owed since 2026-10-27) **[filed: developing-today/code#8811]** | developing-today/code | X6 follow-up | The record says the upstream-quirk issue "was to be filed"; it never was. Small — and exactly the class this audit exists to catch. |
| 9 | Time-based priority schedules for route pools (peak-hours prioritize/deprioritize per provider/model/pool by day/time/timezone/day-of-month) **[filed: developing-today/code#8838]** | developing-today/code | F4.18 | Drew asked 2026-10-12 in a direct OpenCode session; never issued, no design artifact, no build record in any scanned source — found only by the gap pass's OpenCode-DB comb. |

Also noted but NOT manifested (Drew-only physical acts, not filings): delete the paused cred-receiver Vercel project in the Vercel dashboard (C3); the Atlas spike/probe DB delete-or-keep ruling (A1.4 — it is ledger-tracked, so not never-issued).

---

## Appendix A — The 131 idea-engine titles, grouped by theme

Source: records-pass §2 (the full dated list in cloud-ideas-log.md, 2026-10-20 → 2026-11-11). Grouping is this synthesis's; every title is preserved verbatim. Pickup status for ALL of them: **not recorded in the register or the log** (see T3), except the five Drew touched: Ask-the-ledger (built, A1.14), Grounded search plane (ruled + blocked, C5), Registrar consolidation plane (ruled, C6), Per-session agent workspaces in Cloudflare Artifacts (probed, C8), Remote MCP server for lab tools (ruled do-not-re-propose — mcpx owns it, F2). Two more visibly exist as live Workers per the records-pass cross-check (Agent & service inbox → agent-inbox; Edge webhook gateway → webhook-gateway) — the logs never say so.

### Edge / cache / delivery (18)
- BMC/hardware-audit snapshot CDN (DB-per-machine in Turso, thumbnails in R2) — 2026-10-20
- Zettel edge search (DB-per-zettel in Turso, static sites cached on Cloudflare) — 2026-10-20
- Drive passports (QR asset tags in Turso, photos in R2, scan page at the edge) — 2026-10-22
- Edge OCI registry for custom cluster images (blobs in R2, catalog in Turso, pulls served from the edge) — 2026-10-22
- Upstream package pull-through cache (Cache Reserve persists registry files at the edge, artifact catalog in Turso) — 2026-10-27
- Origin-shield cache plane (Smart Tiered Cache in front of every basement origin, Cache Reserve as the persistent upper tier, fill ledger in Turso) — 2026-11-03
- Worker-response memoization plane (Workers Cache in front of the read-heavy Workers, per-entrypoint split, Cache-Tag purges on write, policy + purge ledger in Turso) — 2026-11-09
- Edge image-transformation plane (Image Transformations over R2 originals, declared presets only, transformation ledger in Turso) — 2026-11-09
- Edge compression plane (Compression Rules per content class + end-to-end Brotli from basement origins, wire-verified nightly, savings ledger in Turso) — 2026-11-09
- Public-path rewrite plane (URL Rewrite Rules keep stable public URLs over reorganizable origin paths, rewrite ledger in Turso, wire-verified nightly) — 2026-11-09
- Early Hints preload plane (103 Early Hints replay a declared per-page preload manifest, hint ledger in Turso, wire-verified nightly) — 2026-11-10
- Next-page prefetch plane (Speed Brain speculation rules prefetch the likely next page from edge cache only, eligibility ledger in Turso) — 2026-11-10
- Font-origin exile plane (Cloudflare Fonts serves every Google Fonts stylesheet from the zone itself, eligibility ledger in Turso, wire-verified nightly) — 2026-11-11
- Markdown-for-agents, DIY on the $5 plan (a Worker honors `Accept: text/markdown` for the program's own pages, conversion ledger in Turso) — 2026-11-07
- Edge release train (gradual deployments for the live Workers, release ledger in Turso) — 2026-10-30
- Surge throttle (the Free zone's ONE rate-limiting rule aimed by the hits ledger, mitigation ledger in Turso) — 2026-11-02
- DIY virtual waiting room (FIFO admission queue in a Durable Object, queue ledger in Turso — because native Waiting Room is Business-only) — 2026-10-30
- Graceful-degradation failover (Load Balancing steers ID app basement → edge snapshot, failover ledger in Turso) — 2026-10-26

### Storage / backup / archive (19)
- Talos etcd + TALM recovery vault (snapshots in R2, catalog in Turso, restore served from the edge) — 2026-10-21
- Shared compile cache (sccache in R2, hit/miss ledger in Turso) — 2026-10-21
- RustFS cold-tier gateway (RustFS tiers to R2, catalog in Turso, reads served from the edge) — 2026-10-23
- Restore-drill sandbox (backups proven in a Workers Container, drill ledger in Turso) — 2026-10-23
- Drive-health lakehouse (SMART telemetry via Basin Pipelines into Iceberg in R2, alerts in Turso) — 2026-10-24
- DNS drift vault (versioned zone snapshots in R2, diff + renewal ledger in Turso, status board in KV) — 2026-10-26
- Git mirror vault (bare-repo tarballs in R2, catalog in Turso, restore served from the edge) — 2026-10-26
- Immutable archive retention (R2 Bucket Locks by prefix, retention ledger in Turso) — 2026-10-31
- S3-direct archive landing (R2 Event Notifications catalog what S3-native tools write, no Worker ingest in the path) — 2026-10-31
- Turso ledger export vault (nightly `.dump` exports of every Turso DB to R2, export catalog in D1) — 2026-11-01
- R2 cold-class tiering plane (lifecycle rules age archives into Infrequent Access, policy + class ledger in Turso) — 2026-11-01
- D1 recovery plane (Time Travel bookmarks proven by drill, exports to R2 for beyond-30-day retention, drill ledger in Turso) — 2026-11-07
- Sippy read-through migration plane (S3-compatible sources migrate into R2 only on first real read, migration ledger in Turso keyed by content hash, not eTag) — 2026-11-08
- Immutable backup vault (R2 Bucket Locks retention declared per bucket/prefix, lock ledger in Turso, nightly read-back diff) — 2026-11-10
- Durable Object state recovery plane (PITR bookmarks for every coordination DO, bookmark + safety-floor ledger in Turso) — 2026-11-08
- D1 read-replication plane (Sessions API bookmarks on the live D1 stores, replica-lag ledger in Turso) — 2026-11-08
- Lakehouse query plane (Basin SQL over the drive-health Iceberg tables, saved-query + scan ledger in Turso) — 2026-11-06
- Device-photo evidence pipeline (masters in R2, Cloudflare Images transformations at the edge, photo catalog in Turso) — 2026-10-30
- Recorded-session video library (masters in R2, adaptive viewing copies in Stream, catalog in Turso) — 2026-10-28

### Observability / metrics / forensics (17)
- Basement power/thermal sentinel (ESP32 telemetry in Turso, alert state in Durable Objects, dashboard cached at the edge) — 2026-10-21
- Outside-in synthetic watchdog (Cron probes from the edge, flap state in Durable Objects, incident ledger in Turso, status page in KV) — 2026-10-23
- Cluster metrics edge plane (Prometheus batches into Workers Analytics Engine, alert ledger in Turso, dashboard in KV) — 2026-10-24
- Edge log lake (one Tail Worker archives every Worker's execution logs to R2, error index in Turso) — 2026-10-25
- Scanner observatory + auto-WAF (probe ledger in Turso, one IP List feeding one WAF rule on security.cab) — 2026-10-25
- Upstream-fiber truth sentinel (Radar outage/BGP watch over Brightspeed, incident ledger in Turso) — 2026-10-29
- Fleet egress DNS observatory (one Gateway location for the basement, DNS policies in git, daily log export to Turso before the 24h Free retention expires) — 2026-10-29
- Property hits ledger for the non-proxied sites (Web Analytics beacon on every public property, nightly rollup in Turso) — 2026-10-30
- End-to-end trace spine (OTLP batches via Queues, OTel Collector in a wake-on-batch Workers Container, span index in Turso, raw spans in R2) — 2026-10-31
- Surge forensics lake (Logpush `http_requests` streamed to R2, per-request ground truth aggregated nightly into a Turso surge ledger) — 2026-11-06
- Fiber flow observatory (Cloudflare Network Flow on the edge router, threshold webhooks into a Turso flow ledger) — 2026-11-06
- Workers usage observatory (nightly GraphQL invocation harvest into a Turso usage ledger, month-end included-allowance forecast per script) — 2026-11-11
- DNS demand observatory (nightly `dnsAnalyticsAdaptiveGroups` harvest into a Turso DNS ledger, demand evidence for the domain register) — 2026-11-11
- Performance-regression observatory (scheduled Observatory Lighthouse tests per public page class, score ledger in Turso) — 2026-11-10
- Inside-out device experience plane (DEX tests run from Drew's own enrolled devices, results exported to R2 before the 7-day expiry, latency ledger in Turso) — 2026-11-04
- Cloudflare control-plane change sentinel (Audit Logs + inventory diff in Turso, snapshots in R2) — 2026-10-29
- Corpus link census (Browser Run `/crawl` sweeps the program's own live sites, link ledger in Turso, crawl snapshots in R2) — 2026-11-07

### Security / identity / access (29)
- BMC live-access broker (Cloudflare Access + Tunnel, exclusive session lock in a Durable Object, session ledger in Turso) — 2026-10-24
- AI Gateway front door for the agent stack (caching + rate limits + DLP in the request path, policy/budget registry in Turso) — 2026-10-24
- Remote MCP server for lab tools (one McpAgent Worker, tool registry + call log in Turso, OAuth state in KV) — 2026-10-24 — **Drew ruled: "that's what mcpx is built for" — do not re-propose**
- Agent action approval gate (Access-gated human approval, single-use action-bound token in a Durable Object, decision ledger in Turso) — 2026-10-27
- Keyless SSH for the fleet's Linux boxes (Access for Infrastructure short-lived certificates, target registry in Turso) — 2026-10-27
- Parked-domain anti-spoof shield (no-send DNS policy in git, DMARC aggregate reports parsed at the edge, spoof ledger in Turso) — 2026-10-27
- Certificate Transparency sentinel (CT-log watch over all 40 domains, issuance ledger in Turso, raw entries in R2) — 2026-10-28
- Per-session Turso credential vending (scoped tokens minted by a Worker, vending ledger the agent never touches) — 2026-10-28
- Device identity for fleet ingest (mTLS client certificates for every non-human caller, cert registry in Turso) — 2026-10-31
- Secrets Store consolidation (one account store for shared Worker secrets, rotation ledger in Turso) — 2026-10-30
- Turnstile gate for the program's human forms (pre-clearance at the edge, verdict ledger in Turso) — 2026-11-01
- One alert throat (every sentinel enqueues to one Queue, one Durable Object owns delivery + escalation, ack ledger in Turso) — 2026-11-01
- Positive-security ingest (DIY API Shield schema validation inside the ingest Workers, violation ledger in Turso) — 2026-11-02
- AI Labyrinth crawler sink (security.cab honeypots no-crawl-defying AI crawlers, crawler policy + event ledger in Turso) — 2026-11-02
- Client-side script sentinel (Client-Side Security script inventory for security.cab, script-diff + CSP-report ledger in Turso) — 2026-11-02
- Device-posture gate for high-privilege Access surfaces (WARP posture checks as Require rules, posture-failure ledger in Turso) — 2026-11-02
- Signed-agent passports (Web Bot Auth signatures on every outbound agent/crawler request, key registry in Turso) — 2026-11-03
- Internal TLS cert plane (cert-manager + Cloudflare DNS-01 for LAN-only services, cert registry in Turso) — 2026-11-03
- Per-job R2 credential broker (Temporary Credentials API mints one-bucket, one-prefix, minutes-lived S3 credentials; vending ledger in Turso) — 2026-11-04
- Third-party script exile (Zaraz runs every vendor tool server-side at the edge, event mirror + consent ledger in Turso) — 2026-11-04
- Pre-flight link scanner (Radar URL Scanner verdicts gate every link an agent or inbox touches, verdict ledger in Turso) — 2026-11-04
- Just-in-time Access plane (approval-required Access policies grant temporary auth to the high-privilege surfaces, grant ledger in Turso) — 2026-11-10
- Leaked-credential gate (WAF Leaked Credentials Detection verdicts handled in-Worker, verdict ledger in Turso) — 2026-11-10
- Header-truth plane (Response Header Transform Rules + Managed Transforms declared per zone, wire-verified nightly, header ledger in Turso) — 2026-11-08
- DNSSEC + CAA issuance-lock plane (signed zones + a CA allowlist for all 40 register domains, DS/CAA ledger in Turso) — 2026-11-08
- Per-caller burst shield (Workers Rate Limiting bindings keyed by caller + route inside every expensive Worker, throttle ledger in Turso) — 2026-11-08
- Crawler consent plane (AI Crawl Control policies + Content Signals declared per property, per-crawler ledger in Turso) — 2026-11-07
- Post-quantum leg plane (cloudflared `--post-quantum` strict mode on the secret-carrying tunnels, per-leg PQ ledger in Turso) — 2026-11-11
- Authenticated origin-pull plane (per-hostname AOP mTLS closes the origin-IP bypass on every direct-proxied origin, AOP ledger in Turso) — 2026-11-11

### Agents / compute / platform (48)
- OpenCode agent-run ledger (DB-per-project in Turso, live sessions in Durable Objects, artifacts in R2) — 2026-10-20
- Edge quorum witness (Durable Object tie-breaker for LINSTOR/DRBD, decision log in Turso) — 2026-10-21
- Cluster Postgres edge read plane (Hyperdrive cache/pool in front of Cozystack Postgres, Turso for API keys + audit) — 2026-10-21
- Spare-PC remote lab seats (Durable Object seat locks, reservation ledger in Turso) — 2026-10-21
- PR preview stacks (DB-per-PR in Turso, app in a Workers Container, routed by a Worker) — 2026-10-22
- Hardware-manual semantic search for audit agents (manuals in R2, embeddings in Vectorize, registry in Turso) — 2026-10-22
- First-boot bootstrap rendezvous (single-use TALM claim in a Durable Object, ledger in Turso, bundles in R2) — 2026-10-22
- Power-return boot orchestrator (a Cloudflare Workflow sequences Talos/Cozystack bring-up, run ledger in Turso) — 2026-10-22
- Agent & service inbox (catch-all Email Worker, mail/code ledger in Turso, raw mail in R2, replies via Email Service) — 2026-10-23
- Edge webhook gateway with durable replay (event ledger in D1, raw payloads in R2, delivery via Queues + Tunnel) — 2026-10-23
- Offline-first field kit (Turso embedded replicas on laptops/spare PCs, lag board in KV) — 2026-10-23
- Firmware release watchtower (vendor pages rendered by Browser Run, version registry in Turso, page snapshots in R2) — 2026-10-24
- Brain-dump voice intake (audio in R2, Whisper transcription at the edge, transcript ledger in Turso) — 2026-10-25
- WebRTC media relay for the basement (publish-once to Realtime SFU, TURN fallback, session ledger in Turso) — 2026-10-26
- ESP32 OTA rollout plane (firmware in R2, device registry in Turso, rollout state in a Durable Object) — 2026-10-26
- BMC screenshot reader (Workers AI vision extracts settings from audit screenshots, drift diff in Turso) — 2026-10-26
- Untrusted-code furnace (Sandbox SDK sandboxes execute agent-generated code, job ledger in Turso) — 2026-10-27
- Silent-job dead man's switch (job heartbeats in via a Worker, deadline state in a Durable Object, miss ledger in Turso) — 2026-10-27
- Agent probe foundry (agent-written TypeScript runs in one-shot Dynamic Workers against a narrow lab-data API, probe ledger in Turso) — 2026-10-28
- Point-in-time branch sandbox (Turso branches from a timestamp, branch registry in Turso, auto-reaper Cron Worker) — 2026-10-28
- Edge PXE boot vault (per-MAC iPXE menus from a Turso node registry, version-pinned boot artifacts in R2) — 2026-10-28
- Per-session agent workspaces in Cloudflare Artifacts (repo-per-session forks at the edge, fork registry in Turso) — 2026-10-29 — **see C8**
- Postgres-to-D1 edge read model (CDC delta sync from Cozystack Postgres via Queues, public reads served from D1, sync ledger in Turso) — 2026-10-29
- Parked-domain disposition plane (Bulk Redirects generated from the domain register, hit ledger in Turso) — 2026-10-29
- Edge event spine (K2 streams bridge cluster NATS to independent edge consumers, stream registry + consumer-lag ledger in Turso) — 2026-10-30
- Fleet work dispatch (jobs claimed by basement nodes via Queues HTTP pull consumers, dispatch ledger in Turso) — 2026-10-31
- Private TCP service fabric for edge Workers (Workers VPC Networks to the cluster's MQTT/Redis, service registry in Turso) — 2026-10-31
- Workers Builds build plane (every live Worker built on Cloudflare from git, build ledger in Turso) — 2026-11-01
- Live presence plane (hibernating WebSocket Durable Objects push fleet state, session ledger in Turso) — 2026-11-02
- Edge feature-flag plane (DIY flags in KV with an OpenFeature-shaped evaluator, flag definitions + change ledger in Turso) — 2026-11-03
- Backend-proximity placement plane (Smart Placement for the Turso/Hyperdrive-bound Workers, latency ledger in Turso) — 2026-11-03
- Tenant custom-domain plane (Cloudflare for SaaS custom hostnames front the ID app, tenant/hostname ledger in Turso) — 2026-11-03
- Brain-dump explosion first pass (Workers AI LLM drafts candidate zettel notes from each transcript, staged in Turso for review) — 2026-11-04
- Investigation addendum: incorporating agent notes, migrating design/inventory docs, merging, Emanote design, and dynamic elements — 2026-11-04
- Fleet clock-truth plane (NTS pinned in every TALM TimeSyncConfig, per-node clock ledger in Turso) — 2026-11-04
- Second-overlay management mesh (Cloudflare Mesh gives every device + one basement node a private Mesh IP, participant registry in Turso) — 2026-11-05
- Ask-the-ledger (AI Search over the program's own decision/state corpus, query + miss ledger in Turso) — 2026-11-05 — **built, see A1.14**
- Rolling upgrade train (a Cloudflare Workflow upgrades Talos/Cozystack one node at a time, health-gated, upgrade ledger in Turso) — 2026-11-05
- Grounded search plane for agents (Web Search API via AI Gateway, provider routing policy + search ledger in Turso) — 2026-11-05 — **ruled on, see C5**
- Tunnel born redundant (cloudflared replicas on two hosts/circuits from day one, connector-count ledger in Turso) — 2026-11-05
- Internal RPC mesh (service bindings make Worker-to-Worker calls private and request-fee-free, service graph in Turso) — 2026-11-05
- Registrar consolidation plane (at-cost Cloudflare Registrar transfers measured per-domain in Turso before any domain moves) — 2026-11-06 — **ruled on, see C6**
- Agent browser pool (Browser Run sessions leased through a Durable Object, session ledger in Turso, artifacts in R2) — 2026-11-06
- Ledger extraction plane (Clairic System One extraction turns receipts, renewal notices, and vendor notices into typed Turso rows, raw sources in R2) — 2026-11-06
- Pocket manual brain (Turso-native vector index inside the embedded replica — semantic search that survives a fiber cut) — 2026-11-07
- Turso desired-state provisioner (git manifest reconciled to DB-per-X, scoped platform token without delete, drift ledger in D1) — 2026-11-07
- Origin port-routing plane (Origin Rules destination-port overrides declared per hostname, routing ledger in Turso, wire-verified nightly) — 2026-11-09
- Per-surface settings plane (Configuration Rules declare SSL/security/buffering per hostname class, settings ledger in Turso, wire-verified nightly) — 2026-11-09

Theme counts: edge/cache 18 · storage 19 · observability 17 · security 29 · agents/compute 48 = **131** ✓ (matches the records-pass count).

---

## Appendix B — Coverage notes per slice

- **Slice 2026-09-12_21 (0 items):** evidence-backed zero. No day-files exist for 2026-09-12..21; the earliest daily log is 2026-09-28. No Muse chat existed in the window (the main chat was created 2026-09-28T04:28:53Z). GitHub: zero issues created 2026-09-01..27 in all four fleet repos. One method route failed honestly (direct muse.db archaeology timed out) and the question was answered via chat metadata instead.
- **Slice 2026-09-22_10-01 (7 items):** day-files exist only from 2026-09-28 (onboarding day); 09-22..27 has no record because the relationship begins 09-28. The slice predates Atlas/PM-stack/Cloudflare/Turso as topics — zero mentions; what exists is the substrate (nix persistence, mcpx program, lab-cluster + agent-fs research seeds). Excluded as out-of-scope plumbing: sandbox boot-init, Gmail labels/filters, FB Marketplace deal-hunt, dictation research, Muse-product feature requests.
- **Slice 2026-10-02_11 (14 items):** all 10 day-files read; Oct 10–11 contain no Drew design asks (routine watches only). Dominated by the Cloudflare account setup + nix-cache build (Oct 5–7). Atlas and Turso have NO Drew asks in this slice (all Atlas/Turso design activity is dated Oct 20+); the PM stack does not exist yet.
- **Slice 2026-10-12_21 (25 items):** all 10 day-files read; Oct 12–16 are dominated by automated sweep logs with almost no Drew input — Drew's design activity concentrates Oct 17–21. The PM stack as a named system does not yet exist; its ancestors here are the Turso coordination schema, the Supabase agent schema, and the magic-environment sketch.
- **Slice 2026-10-22_31 (34 items):** all 10 day-files combed (headers + keyword grep + full-section reads) plus memory-search passes. Borderline exclusion flagged and placed by synthesis: the Tailscale Serve ↔ CF Tunnel dual-exposure design (confirmed 2026-10-21) → C9. One record conflict flagged in place and resolved by synthesis: #3125 (see A1.13).
- **Slice 2026-11-01_11 (38 items):** all 11 day-files read via targeted extraction; they quote Drew verbatim and are the primary source (the main chat was not separately paged message-by-message). Over-inclusive by design; engine-generated ideas Drew never asked for are excluded except where Drew ruled on them.
- **Records pass:** workspace design docs (§1a/1b), the code repo doc/ corpus (392 entries; 335 in-window; 236 design-kind listed in its appendix), the idea register + 131-entry ideas log, the decisions ledger (150 items: 84 answered / 34 asked / 20 pending / 12 dropped), 52 goal directories (in-scope ones tabled), and an open-issue census of developing-today/code that is **title-keyword only and capped at 30 per term** — a baseline, not a complete tracked-set; absence from the census is not proof an issue was never filed. Per-entry doc/ issue states were not checked.
