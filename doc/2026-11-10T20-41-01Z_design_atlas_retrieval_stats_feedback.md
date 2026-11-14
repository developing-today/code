# Design: Atlas retrieval statistics, result feedback, and query trace

- Issue: #6027 (parent program: #6018, Atlas ingestion). Statements S21–S27 of the 2026-11-04 brief.
- Status: design record for task 13553. This document fixes the trace schema, the
  statistics aggregation model, the feedback capture + normalisation pipeline, and the
  decision map. It carries no implementation; §9 splits implementation into follow-ons.
- Inputs, not re-decided here: #5774 (query-time parameters, full-vector logging per
  run), #5733 (Jaccard rescore), #5741 (SPLADE), the sibling content versioning /
  retention design (`doc/2026-11-08T10-26-00Z_design_atlas_content_versioning_retention`),
  and the cold-tier shard design. Where this design needs a version or a tier, it
  references those designs' notions.

## 0. Verified current state (2026-11-10, origin/main a8ece4e188)

- A search result today is a `BranchHit { chunk_id: i64, dist: Option<f64> }`
  (`pkgs/atlas-search/crates/atlas-core/src/types.rs`): a chunk-level ID exists, but
  there is **no query-scoped result ID and no persisted trace**. Per-branch pools
  (`BranchPools { fts, vector }`) are fused by `fusion::fuse` into `FusedEntry`
  values, which carry an in-memory `rank_sources_json` provenance — computed, then
  discarded when the response is served.
- Pipeline stages, as the code names them: per-shard branch scan (FTS branch and
  vector branch) → fusion/merge → rescore (#5733 / #5741) → final ranking.
- `atlas-core/src/eval.rs` (`QueryRec`, `Qrel`, `Run`, `SignalReport`) is an
  **offline** evaluation harness over judged query sets. It is not a production
  trace and must not become one (§8); the trace below is the production counterpart
  and reuses its vocabulary where the meanings match.

## 1. Invariants

1. **Every served result is associable to its query.** No response surface may
   return a result without the `query_id` + `result_id` pair (§2) that lets an
   operator recover the full trace. A surface that cannot carry the pair does not
   serve results.
2. **Traces are append-only and written before the response is considered
   complete.** A trace write failure is a search failure (fail closed), not a
   silently untraced success — an untraced result can never receive feedback, be
   counted, or be debugged, which defeats the program.
3. **Feedback attaches to IDs, never to positions.** Positions change between
   identical queries; `result_id` (§2) does not.
4. **Raw feedback is never mutated.** Normalisation (§6) produces a derived
   record; the original vote + comment is preserved verbatim alongside it.
5. **Statistics are computed from traces, not from a parallel instrumentation
   path.** If a number cannot be derived from stored traces, the trace schema is
   wrong and gets amended — a second, divergent counting path is the failure mode
   this design exists to avoid.
6. **No signal without a decision.** Every statistic and feedback aggregate
   produced must name the decision it feeds (§7). An aggregate with no decision
   is removed, not merely unused.

## 2. Identity: query_id and result_id

- `query_id`: a UUIDv7 minted by the search service at request receipt (one per
  executed search, including retries as separate executions linked by
  `retry_of: Option<query_id>`). It is returned in every response envelope and
  printed by the CLI.
- `result_id`: `query_id` + the chunk's stable identity within that execution:
  `(query_id, chunk_id)`. `chunk_id` already exists (§0); the pair is unique per
  execution and stable for feedback, because feedback is always about *this
  result as served for this query*. Cross-query aggregation joins on `chunk_id`
  (and its file/embed identity, §3), never on `result_id`.
- File/embed identity for aggregation: each chunk resolves to `(source_file_id,
  embed_id, embed_content_hash)` via the chunk detail row and the versioning
  design's content version. The trace stores these resolved values (§3) so later
  re-embeds do not rewrite history: statistics describe the embed state that was
  actually scored.

## 3. Query trace schema

One JSON record per executed query, written by the search service at completion
(after final ranking, before the response is released). Storage: one JSONL file
per day per service instance under the Atlas data root, ingested into Atlas
itself as the "statistics and reports" source set (§4). The record:

```json
{
  "schema": "atlas-query-trace/v1",
  "query_id": "uuidv7",
  "retry_of": null,
  "ts": "RFC3339 UTC, execution start",
  "duration_ms": 0,
  "surface": "cli | service-api | ui:<name>",
  "requester": {"kind": "drew | agent | user", "id": "stable requester id"},
  "query_text_hash": "sha256 of the normalised query text",
  "query_text": "present only when the surface's retention class permits it; else null",
  "parameters": {
    "pool": 0, "top_k": 0,
    "vector_backend": "as in VectorBackend",
    "weights": {"fts": 0.0, "vector": 0.0, "rescore": "none | jaccard | splade"},
    "filters": "the effective filter set, serialised"
  },
  "results": [
    {
      "chunk_id": 0,
      "source_file_id": "...", "embed_id": "...",
      "embed_content_hash": "hash of the embed content actually scored",
      "embed_state": {"version": "per the versioning design", "tier": "hot | cold"},
      "provenance_mark": "...", "sensitivity": "...",
      "stages": {
        "shard": {"shard_id": "...", "branch": "fts | vector",
                  "rank_in_shard": 0, "score": 0.0},
        "merge": {"rank_after_fusion": 0, "fusion_score": 0.0,
                  "rank_sources": "the FusedEntry rank_sources_json, persisted"},
        "rescore": {"applied": true, "kind": "jaccard | splade", "score": 0.0},
        "final": {"rank": 0, "served": true}
      }
    }
  ],
  "pool_counts": {"per_shard": {"<shard_id>": {"fts": 0, "vector": 0}},
                  "merged": 0, "rescored": 0, "served": 0}
}
```

Notes on the shape:

- **Stage scores are per result, per stage** — shard rank/score, merge rank and
  fusion score, rescore score, final rank. A result that entered a shard pool
  but did not survive fusion still appears in `results` with
  `final.served: false` and no final rank; survival statistics (§5) are computed
  over pool entrants, not only over served results. The pool is bounded by the
  existing `pool` parameter, so this does not unboundedly grow the record.
- **`embed_content_hash` + `embed_state`** pin "the exact state of the embed
  content that was scored" (requirement 6) even after the source file is
  re-embedded or re-tiered.
- **`query_text_hash` always, `query_text` conditionally**: statistics and
  feedback normalisation need query grouping and context; surfaces whose
  retention class forbids storing raw query text still get full statistical use
  of the trace.
- Schema versioning: `atlas-query-trace/v1`; a v2 record is a new schema value,
  never a silent field redefinition (invariant 5 depends on stable derivations).

## 4. Trace retention — traces are corpus

- Trace JSONL files are registered in the ingestion source set as source class
  **"statistics and reports"** (the parent program's ingestion taxonomy), so
  traces are themselves searchable and reportable through Atlas rather than
  living in a side store nobody queries.
- Retention: raw traces retained **180 days** hot; after 180 days the per-result
  stage detail is compacted into the aggregate tables of §5 and the raw record
  is moved to the cold tier with the same retention as other cold corpus. A
  trace referenced by any feedback record (§6) is retained raw for as long as
  the feedback record exists — feedback whose evidence has been deleted is
  decoration.
- Deletion interplay: when the versioning/retention design's deletion triggers
  remove a source file, its chunks' trace entries are **not** deleted; they are
  the evidence that the retention decision was correct (§7). The embed content
  itself is never copied into the trace — only its hash and state — so traces
  cannot resurrect deleted content.

## 5. Statistics aggregation model

Computed by a batch job over stored traces (daily, over rolling 7-day and
28-day windows), into aggregate tables in the same "statistics and reports"
source class. All aggregates are per `(source_file_id, embed_id)` unless noted.

1. **Shard-stage survival** (requirement 1): for each embed, counts of queries
   where a chunk of that embed (a) entered any shard pool, (b) entered the
   top-K of its shard, (c) survived fusion into the merged set — survival
   rates (b/a) and (c/a), split by branch (FTS vs vector) and by shard.
2. **Final-rank distribution** (requirement 2): over queries where the embed
   was served, counts landing at final rank 1, rank 2, rank 3, ranks 1–5, and
   served-below-5 — the distribution, not a mean; a mean rank hides the
   difference between "usually first" and "occasionally first, mostly absent".
3. **Exposure**: queries where the embed was in a pool but never served
   (the §3 `served: false` population) — the candidate set for ranking
   investigations and for retention review (§7).
4. **Feedback aggregates**: per embed, weighted thumbs ratio and normalised
   comment category counts (§6), joined from feedback records on `chunk_id`.
   Reported alongside 1–3, never merged into them: retrieval statistics say
   what the system did; feedback says what a judge thought of it.

Aggregates are recomputed from raw traces on schema or definition change
(replayable by construction), never patched in place.

## 6. Feedback capture and normalisation

### 6.1 Capture

- **Per result**: thumbs up / thumbs down, one action from whatever surface
  returned the result (requirement 4):
  - CLI: `atlas feedback --query <query_id> --chunk <chunk_id> --up|--down
    [--comment <text | @file>]`, and an interactive post-search prompt that is
    a single keypress per result.
  - Service API: `POST /feedback` with `{query_id, chunk_id, vote, comment?}`
    and the same fields accepted inline on any surface that already talks to
    the service; the response envelope of every search carries the
    `query_id` the call needs (invariant 1).
  - UI surfaces: a thumbs affordance rendered with each result; one click, no
    navigation, no form required before the vote is recorded.
- **Per query**: a single overall thumbs + optional comment ("this whole
  answer set missed"), stored with `chunk_id: null`. Per-query feedback
  judges recall and ranking of the set; per-result feedback judges precision
  of an item. Both are first-class; neither is derived from the other.
- **Comment**: optional at any length up to essay size (requirement 5). The
  capture field imposes no practical length pressure (minimum supported
  64 KiB); long-form feedback is explicitly welcomed in the affordance copy.
- A feedback record: `{feedback_id, query_id, chunk_id | null, vote: up|down,
  comment: verbatim | null, requester (§6.3), ts, opened: bool (§6.4)}`.

### 6.2 Normalisation (our job, not the user's — requirement 5)

1. Raw records land unmodified (invariant 4).
2. An **agent normalisation pass** runs daily over new commented feedback,
   with the full context the user never has to supply: the query text (or its
   retention class), the trace (§3), the result's content, and its stage
   scores. It emits a structured derivative: category (one or more of
   `wrong-content`, `stale-content`, `right-file-wrong-chunk`,
   `ranking-too-low`, `ranking-too-high`, `missing-result`,
   `duplicate-or-near-duplicate`, `other`), the chunk/file the comment is
   actually about when it differs from the voted one, and a one-paragraph
   summary. Vote-only records skip the pass and aggregate directly.
3. A human spot-check (Drew, or a lane under the standing review rules)
   samples normalised records weekly; systematic normalisation errors are
   fixed by amending the pass, and derivatives are regenerated — raw records
   make regeneration always possible.

### 6.3 Identity and weighting

- Requester kinds: `drew` (Drew himself), `agent` (any fleet/OpenCode
  session), `user` (any other human user).
- Aggregation weights for §5.4 and §7 decisions: **Drew 1.0, other users
  1.0, agents 0.25** — agent judgement is real signal (agents are heavy
  search users) but it is not a substitute for a human's, and unweighted
  agent volume would swamp the human signal by count alone. Weights are
  parameters of the aggregation job, recorded in each aggregate's
  provenance, not constants buried in code.
- Identity is the service's requester identity; anonymous feedback is
  accepted and weighted 0.1, because a surface that demands identity before
  a thumbs vote will simply not receive votes (requirement 4's "zero
  instruction" bar).

### 6.4 Anti-gaming and quality rules

- **Unopened results**: surfaces report whether the result was opened/
  expanded before the vote. A down-vote on an unopened result is recorded
  (it is still signal about title/snippet quality) but is excluded from
  content-quality aggregates and counted separately as snippet signal.
- **Duplicates**: one active vote per `(requester, query_id, chunk_id)`;
  re-voting supersedes, keeping the superseded record in the raw store.
  Identical comments from one requester across many queries are collapsed
  in aggregates, not in the raw store.
- **Agent self-feedback loops**: an agent may not cast a weighted vote on a
  result served to a query that same agent (same session lineage) issued
  in the course of work it is itself evaluating — such votes are recorded
  with weight 0 and flagged `self-feedback`. Agents judging *other*
  sessions' retrievals is ordinary 0.25-weight signal.
- **Brigading by volume**: aggregates use unique-requester counts next to
  raw counts; a decision input (§7) requires the unique-requester figure,
  so one requester voting a thousand times moves nothing by itself.

## 7. Decision map — what each signal changes

| Signal (source) | Decision it feeds | Acting rule |
|---|---|---|
| Embed with high pool entry, near-zero survival + near-zero final-rank presence over 28 days (§5.1–5.3) | **Retention**: candidate for the versioning design's deletion/demotion triggers | Surfaced in the weekly statistics report; deletion still goes through the retention design's own gates — statistics nominate, they do not delete |
| Embed with sustained final-rank 1–3 presence (§5.2) | **Retention**: protected from demotion triggers | Demotion requires contradicting feedback evidence, not statistics alone |
| Weighted down-vote concentration on one embed (§5.4) with normalised category `stale-content` / `wrong-content` | **Retention + re-ingestion**: re-embed or retire the source | Category-specific: `stale-content` → re-ingest first; `wrong-content` → human review before any deletion |
| Category `ranking-too-low` recurring for a query class (§6.2) | **Ranking**: weight / rescore-parameter change candidates (#5774 parameters) | Tested through the eval harness (`eval.rs` qrels) before adoption; production feedback nominates, offline eval disposes |
| Branch survival skew (FTS vs vector, §5.1) shifting materially between windows | **Search patterns**: pool sizes, fusion weights, expansion policy | Same eval-gated path as ranking changes |
| Per-query down-votes with `missing-result` comments (§6.2) | **Product/ingestion**: new source candidates for the #6018 program | Filed as ingestion candidates with the comment as evidence |
| Agent self-feedback flags (§6.4) clustering in one workflow | **Product/process**: that workflow's retrieval step is redesigned, not re-weighted | Surfaced in the weekly report as a process finding |
| Any aggregate with no consumer after two reporting cycles | **This design**: aggregate is deleted (invariant 6) | Named in the weekly report before removal |

The weekly statistics report is itself an artifact in the "statistics and
reports" source class: decisions taken from it cite the aggregate and window,
so a ranking or retention change is traceable back to the traces that
motivated it.

## 8. Rejected alternatives

- **Reuse `eval.rs` as the production trace.** Its records are judged-set
  artefacts (qrels, runs) built for offline scoring; production traces are
  per-execution facts about unjudged queries. Conflating them would corrupt
  both. They share vocabulary and the §7 eval gate, nothing else.
- **Store traces only in the service's database, outside Atlas.** Then the
  statistics cannot be queried with the same tool they describe, and the
  parent program's "statistics and reports" source class stays empty.
  Traces-as-corpus is a requirement (§4), not a convenience.
- **Feedback keyed by rank position.** Positions are unstable across
  identical queries; feedback would attach to different content run to run.
- **Free-text-only feedback (no thumbs).** Fails requirement 4 outright:
  the bare minimum every user can do with zero instruction is the thumbs
  pair; essays are the optional layer on top.
- **Real-time aggregation.** Daily batch over traces is replayable and
  auditable; nothing in §7 acts faster than weekly anyway.

## 9. Implementation split (follow-ons)

This issue carries no implementation work itself. The split, filed as
linked follow-on issues when this design lands:

1. **Trace capture**: mint `query_id`, persist `FusedEntry` provenance and
   per-stage scores as `atlas-query-trace/v1` records (§§2–3); fail-closed
   trace write (invariant 2).
2. **Trace ingestion + retention**: register traces in the "statistics and
   reports" source class; 180-day hot retention + compaction + the
   feedback-reference exception (§4).
3. **Statistics aggregation + weekly report**: the §5 aggregates and the
   §7 weekly statistics report.
4. **Feedback capture surfaces**: the §6.1 API endpoint, CLI command/
   prompt, and UI affordance contract, with `opened` reporting (§6.4).
5. **Feedback normalisation + decision wiring**: the §6.2 agent pass, the
   §6.3 weighting in aggregation, and surfacing §7 nominations (retention
   candidates, ranking-change candidates) — no automated deletion or
   weight change ships in this follow-on; it produces nominations only.

Dependency order: 1 → 2 → 3; 4 is independent of 2–3 except for the
feedback-reference retention exception; 5 follows 3 + 4.
