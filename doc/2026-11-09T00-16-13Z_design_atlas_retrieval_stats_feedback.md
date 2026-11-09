# Design: Atlas retrieval traces, statistics, and result feedback

- **Date:** 2026-11-09
- **Status:** Design record; implementation is split into linked follow-on issues.
- **Issue:** [#6027](https://github.com/developing-today/code/issues/6027)
- **Parent:** [#6018](https://github.com/developing-today/code/issues/6018), the wider Atlas ingestion program. This record owns only query traces, derived retrieval statistics, explicit result feedback, and their decision paths.
- **Related decisions and work:** [#3125](https://github.com/developing-today/code/issues/3125) provenance registry; [#6026](https://github.com/developing-today/code/issues/6026) content versioning and retention; [#5774](https://github.com/developing-today/code/issues/5774) query-time parameter surface; [#5733](https://github.com/developing-today/code/issues/5733) Jaccard rescore; [#5741](https://github.com/developing-today/code/issues/5741) SPLADE and eval work; [#6060](https://github.com/developing-today/code/issues/6060) one-model comparability decision; [#7647](https://github.com/developing-today/code/issues/7647) query-trace embedding-model provenance slice; [#6041](https://github.com/developing-today/code/issues/6041) retention rule engine; [#6046](https://github.com/developing-today/code/issues/6046) feedback-derived golden set; [#6047](https://github.com/developing-today/code/issues/6047) implicit-signal logging; [#6054](https://github.com/developing-today/code/issues/6054) source deletion propagation.

## 1. Intent and boundary

Atlas needs to answer, for a search and any result it returned or considered: what exact query/configuration/corpus state was used, where each candidate survived or was dropped, what a human or agent said about the result, and which concrete product or corpus decision may use that evidence.

This record decides the query-time trace contract, the derived statistics, explicit-feedback capture and normalization, trust/anti-gaming rules, and the signal-to-decision map. Traces and reports are themselves an ingestion source under #6018's **statistics and reports** category. This does not redesign that wider source set.

The existing designs remain inputs, not decisions to reopen here:

- #5774 owns the names and availability of effective query-time parameters. A trace snapshots the exact effective values and their version; it does not add or rename knobs.
- #5733 owns Jaccard's rescore and dedup semantics. #5741 owns the SPLADE investigation, signal weighting, and its signal-specific evaluation. This design records whichever stages actually ran; it does not choose a signal or its weight.
- #6060 fixes one embedding model across Atlas tiers. #7647 is the existing implementation slice for recording the query model and each candidate's model and surfacing mixed-model violations; this design incorporates that contract rather than replacing it.
- #3125 and #6026 own source/provenance identity and content-version/retention semantics. The trace references their immutable identities and follows their classification/deletion rules.
- #6046 owns feedback-derived golden-set promotion and eval gates; #6047 owns implicit signals. This record defines the explicit-feedback contract and the inputs they may consume, not the implementation of those sibling designs.

## 2. Decisions at a glance

1. **Capture at query time.** Scores, ranks, candidate membership, query variants, effective parameters, and shard snapshots are written while the query runs. They cannot be reconstructed reliably from a later corpus state.
2. **Record the full candidate lists at every stage, not only displayed top-K results.** Each stage records every candidate it considered, its score/rank when scored, and whether it emitted or pruned the candidate. Stage boundaries also record input/output counts and any truncation.
3. **Use a query header plus normalized per-candidate/per-stage rows.** A trace has one opaque `query_id`; result-stage rows join through a stable, global `result_id` and the immutable `embed_version_id`. Store exact effective parameters and per-shard snapshot markers.
4. **Keep trace evidence append-only and content-addressed.** The trace pins the exact embed-content version/hash and state it scored. It does not copy text into a lower-classification store. The trace/report source is registered as `statistics and reports`; retention and deletion leave explicit tombstones.
5. **Compute funnel statistics from complete trace rows.** Report per-shard/per-stage survival per file/embed and final best-rank distributions per file/embed for ranks 1, 2, 3, and 1–5, with denominators and query cohorts visible.
6. **Make explicit feedback result-scoped first.** Every result surface provides one-action thumbs up/down for each result. A separate query-level vote is allowed for an overall-search judgment and is never copied onto every result.
7. **Preserve provenance and separate trust strata.** Initial analysis weights are Drew 4, another authenticated human 2, an independent agent 0.5, and anonymous/unknown or self-feedback 0. Keep raw counts and strata alongside weighted totals; these weights never directly change live ranking.
8. **Normalize comments ourselves, with human approval.** Preserve the user's unmodified comment and context; an agent drafts structured labels/evidence, and Drew or a designated human curator approves or edits that normalization before it can affect a durable decision.
9. **Raw feedback is not an online ranking feature.** It informs investigation, retention dry-runs, product decisions, and the #6046 golden set. Any ranking change is a measured offline experiment under the relevant eval gate.

## 3. Query-trace record

### 3.1 Identity and event shape

`query_id` is an opaque, globally unique UUIDv7 generated at the first trusted Atlas boundary and propagated unchanged across CLI, service, UI, and agent-tool surfaces. It identifies one submitted search, not a user or a query string. Retries that are the same request reuse an idempotency key and do not create duplicate traces; a deliberate re-run gets a new `query_id` linked to the prior query when applicable.

The trace is logically one immutable header and a set of child rows:

| Record | Key fields |
| --- | --- |
| `query_trace` | `query_id`, schema version, timestamp, actor class and scoped principal reference, client/surface, raw query and ordered rewrite variants, query-class/planner metadata, effective parameter snapshot, Atlas build/config digest, query embedding model, completion/truncation status |
| `query_trace_shard` | `query_id`, shard ID/tier, immutable shard snapshot/head marker, manifest/version digest, shard scan status and candidate counts |
| `query_trace_candidate` | `query_id`, `result_id`, source/file ID and file-version ID, chunk/occurrence ID, embed ID and `embed_version_id`, exact embed-content hash and immutable content reference, source/provenance mark, tier/classification, per-candidate embedding model, shard/occurrence identity |
| `query_trace_stage` | `query_id`, `result_id`, stage ID/name/order, scorer/variant, shard where applicable, score or null, deterministic rank or null, input/considered/emitted/pruned state, cutoff K, prune reason, tie-break inputs |

The public interchange form may be JSON or a compact columnar equivalent; the logical fields and join keys above are the contract. Scores preserve the exact numeric value used for ordering (including score kind/units and scorer identity), not a rounded display value. Ranks are one-based, deterministic, and use the stage's actual tie-break rule. A score of zero is not a substitute for “not scored.”

### 3.2 Candidate/result identity

Every trace and feedback event uses `result_id` for the result identity and binds the vector materialization scored as `embed_version_id`. A file path, current rank, snippet, display URL, content hash alone, or backend row number is not an identity. **The current IDs are not sufficient:** `BranchHit` carries `chunk_id: i64` (`pkgs/atlas-search/crates/atlas-core/src/types.rs:97-105`), while the legacy local vector/corpus mapping joins via `content_hash` (`.../atlas-core/src/local.rs:59-72`). These identities are backend-local or content-based; neither guarantees a globally unique source occurrence across shards and content-version changes. The design therefore introduces:

- `result_id`: a globally unique, immutable UUID assigned to a chunk occurrence in a specific file version. Derive it deterministically (UUIDv5) from the `file_id`, `file_version_id`, immutable source-occurrence identity/span, and chunker/schema version. Identical text at two source locations remains two results. A new file version or changed chunk boundary creates a new result ID. Shard copies of one occurrence retain the same ID.
- `embed_version_id`: an immutable ID for one embedding materialization, derived from `result_id`, the hash of the exact canonical bytes sent to the embedder, the model-and-pipeline digest, and the embedding run/generation. Re-embedding or changing content/pipeline creates a new embed version without rewriting old feedback.

The storage contract verifies that the implementation can resolve these IDs back to the #3125 occurrence and #6026 `(file_id, version_seq)` lineage. Existing `chunk_id` remains a local lookup key, not a public join key. In either case:

- repeated appearances of the same result in one query deduplicate by `(query_id, result_id, embed_version_id)` while retaining each shard occurrence in the trace;
- a changed source version or changed embedded bytes is a distinct version and cannot inherit feedback silently;
- `source_file_id`, `file_version_id`, `embed_id`, and `embed_version_id` are first-class fields so file-level rollups do not infer identity from paths.

The trace stores the hash of the canonical bytes actually supplied to the embedder (`embed_content_hash`, SHA-256), a resolvable immutable content-addressed reference, and the version/state metadata that existed at query time. A hash without a retained/resolvable immutable reference is insufficient for “what exact content was scored.” The source object remains governed by #3125/#6026; trace ingestion must not duplicate it into a less-protected tier.

### 3.3 Stage coverage and query state

Capture a row for every candidate at each stage where it is considered, including candidates which fail the stage cutoff. A stage row records `considered=true`, score/rank when computed, `emitted` or `pruned`, cutoff, and a reason such as below-cutoff, policy-filtered, shard-unavailable, or scorer-error. Stage summaries record candidate counts entering/leaving, whether the list is complete, and whether any implementation limit truncated it. This preserves the distinction between “ranked below K,” “not examined,” and “not available.”

Stage IDs are stable names plus an ordinal/instance, for example `shard.vector`, `shard.bm25`, `merge.rrf`, `rescore.jaccard`, `rescore.splade`, and `final`. The actual ordered stage list comes from the running search, not a hard-coded assumption that every signal is enabled. A stage may have one row per signal/variant for a candidate; the fused/combined stage is recorded separately. A candidate not forwarded to a later stage has an explicit pruned/not-considered state rather than a fabricated score.

The header snapshots:

- the raw query and exact ordered rewritten/planned variants submitted to each scorer (with hashes and access classification);
- the opaque effective parameter vector plus defaults/preset/schema version, including all defaults, overrides, stage toggles, per-shard/global K values, signal weights/multipliers, rescore depths/thresholds, and hint/variant selection as actually used. #5774 owns the field names; this design only requires the exact effective vector and its version;
- query embedder/model and build/config identity;
- the immutable shard snapshot/head marker and manifest digest for every shard queried, plus failures or omissions. The current `snapshot_id()` hashes sorted shard names, tiers, and chunk counts (`.../atlas-core/src/shards/mod.rs:515-533`); retain it as a shard-set identity, but do not treat it alone as a content/head snapshot. The trace also needs per-shard manifest/head markers that change when the scored corpus state changes;
- the caller kind (`human`, `agent`, `service`), an access-scoped pseudonymous principal reference, and client surface. Do not put credentials or bearer tokens in a trace.

This makes a trace auditable against a changed corpus without replaying or recalculating scores. Replay is not required to reproduce future model behavior; the stored scores and immutable input/config references are the historical evidence.

### 3.4 Storage, ingestion, retention, and deletion

The logical trace store is append-only and query-addressable. Use separate header, shard, candidate, and stage relations/objects so aggregation can scan stage rows without copying query text. Index at minimum by `query_id`, `(result_id, embed_version_id)`, `file_version_id`, and time; shard/stage/time indexes support funnel reports. Raw query/comment text is access-controlled and excluded from routine aggregate reports. The existing search response carries no `query_id`, per-stage candidate scores, or embed-version identity, so these are additive trace fields rather than fields that can be reconstructed from today's envelope.

Register full traces, feedback, aggregate statistics, and decision reports as the #6018 ingestion source class **statistics and reports**. Ingestion carries source ID, schema version, creation time, access tier, and provenance; it does not flatten away the original query/result/version links.

Initial retention policy for this source:

- Keep the full query-time trace and raw query variants for **180 days** in the source's classification tier. This supports incident analysis, evaluation, and seasonal comparisons without making raw query content indefinite by default.
- Keep compact, query-free aggregates and decision reports for **3 years**, with cohort/sample counts and schema/aggregation version. They remain reports, not synthetic search feedback.
- Keep the exact content-addressed embed snapshot resolvable for at least the full-trace retention window. Apply the source content's retention/deletion decision if it is stricter.
- Apply #6026's versioning/retention and #6054's deletion propagation. If a source item is removed, purge its text/vector/content reference as policy requires and retain only a non-content tombstone identifying the purged version, deletion time, and policy reason. Redact query/comment text if it contains deleted or reclassified source material.
- A higher classification wins: query traces and reports are stored no lower than the most restrictive query, candidate, or feedback content they contain. Secret-tier query/result data never flows into an internal/public statistics corpus. Aggregation must preserve tier separation; cross-tier reports contain only explicitly approved, non-reidentifying counts.

Retention durations are defaults owned by this design's source class and may be changed only with measured storage/privacy review; #6041 consumes surface-history/last-surfaced aggregates for its dry-run rule engine but does not silently extend raw trace retention.

## 4. Retrieval-statistics aggregation

Statistics are derived views over complete query traces, not a second independent event stream. Every report includes its time window, query cohort, stage/schema version, eligible-query denominator, candidate coverage/truncation rate, and whether counts are raw or identity-weighted. Exclude incomplete traces from the primary rate denominator and report them separately; never treat missing stage data as a retrieval miss.

### 4.1 Shard-stage survival

For each `(shard_id, stage_id, file_version_id, embed_id, embed_version_id)` and selected query cohort, report:

- `incoming_queries` and `incoming_candidates`: queries/candidate occurrences that reached the stage from the shard;
- `survived_queries` and `survived_candidates`: distinct queries/candidate occurrences emitted from that stage into its next boundary/top-K;
- `survival_rate = survived_candidates / incoming_candidates`, plus the query-level rate `survived_queries / incoming_queries`;
- per-stage cutoff K, score/rank distribution, and explicit counts for filtered, not-scored, truncated, failed, and duplicate occurrences.

The shard occurrence is retained for survival accounting: if the same `result_id` is present in two shards, both shard paths are counted separately at shard stages. The merged/final views deduplicate to one result/version per query. Rollups may group by file or embed version, but they must not pool different versions and then imply the older version was the one that surfaced.

### 4.2 Final-rank distribution

After the final merge and all configured rescores, take the best final rank of any result belonging to each `(file_version_id, embed_id, embed_version_id)` for a query. Report both:

- **exclusive bins**: final best rank exactly 1, 2, 3, 4, 5, and 6+ / absent;
- **cumulative top-K counts**: best rank ≤1, ≤2, ≤3, and ≤5 (the required top 1, top 2, top 3, and top 1–5 views).

The denominator is queries where the file/embed version was eligible in a queried shard snapshot; also expose all-query and exposed-candidate denominators so a report cannot disguise absence as poor ranking. A query can contribute at most once per file version/embed version to the final-rank distribution, even if several chunks from that file appear. Keep result/chunk-level distributions available for debugging, but label them separately from file-level counts.

Reports preserve raw counts and the identity strata defined below. Ranking parameters and stage definitions are dimensions so before/after comparisons refer to the same cohort and pipeline. The default report breaks down by shard, tier, source/provenance class, query class, signal/stage, and embed/file version where sample size permits; suppress or coarsen small sensitive cohorts rather than leaking their query or content.

## 5. Explicit feedback capture and normalization

### 5.1 What a vote means and where it appears

The canonical explicit action is a **per-result** up/down vote bound to `(query_id, result_id, embed_version_id)`. On every surface that presents Atlas results, the up/down affordance sits beside each result and is usable in one action without opening a form or leaving the result. The clicked vote is acknowledged immediately; comment entry is optional and never blocks the vote.

Surfaces must preserve and return `query_id`, `result_id`, and `embed_version_id` with each result:

| Surface | One-action path |
| --- | --- |
| CLI / TUI | Each printed result exposes `atlas feedback <query_id> <result_id> <embed_version_id> up|down` as a one-command action; interactive clients also bind a single key to up/down. A search invocation may accept a result-scoped feedback option when the result ID is already known. |
| Service/API | Search responses return the three join IDs. A single `POST /feedback` with `vote` and those IDs records the vote; it does not require a second confirmation round trip. |
| Web/UI | Thumbs up/down controls are adjacent to every result on every search/results surface. A result never requires opening a detail pane to reach the controls. |
| Agent/tool clients | The result object carries the same IDs and the feedback tool accepts one vote call. The service records the authenticated agent principal and run provenance, not a client-asserted display name. |

A separate **query-level** vote is supported only for an overall-search judgment (for example, “none of these answered my question”). It has `scope=query`, a `query_id`, and no `result_id`/`embed_version_id`; it is never fanned out to every result. Do not infer a result's correctness from an overall query vote. The primary result-quality metric remains result-scoped.

An optional free-text comment may accompany either scope, up to **32,000 Unicode code points** (roughly an essay). Reject over-limit input with a clear message; never silently truncate. Preserve the original text exactly in its classification tier. The thumbs action remains complete if comment submission is cancelled or fails.

### 5.2 Feedback event and identity

Each immutable feedback event contains: `feedback_id`, idempotency key, `query_id`, scope, result/version keys when result-scoped, vote, optional original comment, timestamp, verified actor kind/principal reference, client surface, query actor kind, query/run/session identity, result-opened state known at vote time, and normalization status/version. Identity is assigned by the trusted service from authentication/session context, not accepted from user-controlled payload fields.

Maintain separate raw counts and a versioned weighted view:

| Feedback source | Initial analysis weight | Rule |
| --- | ---: | --- |
| Drew, authenticated owner and corpus curator | 4.0 | Highest-priority direct judgment; retain separately so it cannot be hidden by pooled votes. |
| Other authenticated human | 2.0 | Independent human signal; retain by principal so consensus can be measured. |
| Agent that did not issue/produce the query or result under evaluation | 0.5 | Advisory evidence only; never treated as independent human ground truth. |
| Anonymous/unknown identity | 0 | Retain for abuse/UX diagnostics, but do not use for corpus/ranking decisions. |
| Same agent, same run/session, or same principal that generated the query/result being judged | 0 | Mark as self-feedback; excluded from quality weights and golden-set promotion. |

Weights are an analysis convention, not a statement that agent votes are equivalent to human votes. Always show per-class raw counts and weighted counts; never publish only the pooled score. Version weight-policy changes so old reports can be recomputed. No identity class is permitted to directly rerank live results from this signal.

### 5.3 Duplicate, unopened, and self-feedback rules

- **Retries/duplicates:** an idempotency key makes transport retries one event. The active vote is unique per `(principal, query_id, scope, result_id, embed_version_id)` (or query-level key); a changed vote appends a revision and the latest revision is active. Preserve prior revisions for audit, but never count them as multiple voters. Repeated identical comments across many targets are flagged/rate-limited, not multiplied into consensus.
- **Unopened results:** record `opened_before_vote` as true/false/unknown. A vote on an unopened result is accepted (the snippet may be enough for a useful judgment), but is marked `unopened`; it is excluded from “used/verified result” labels and automatic golden-set promotion. It remains a separate UX/snippet signal for human review and cannot by itself trigger deletion or ranking changes.
- **Self-feedback:** compare authenticated actor/run identity with query issuer, result-producing agent, and relevant session. Self-feedback is stored for diagnostics with weight zero; it cannot train, promote a golden example, or count as independent validation. Independent agents can contribute only at the lower agent weight.
- **Version binding:** a vote always names the embed version the user saw. If the current result has changed, do not migrate the vote; the old version can be resolved through the trace/tombstone. Prevent one actor's repeated votes on the same result/version from masquerading as multiple feedback events.
- **Rate and abuse controls:** cap event rate per authenticated principal, retain rejected/flagged attempts as security telemetry only, and require human review before a suspicious burst enters any quality report.

### 5.4 Comment normalization is our job

The pipeline retains the verbatim comment plus its trace context (query variants, result/version, displayed rank/snippet, and stage trace) and creates a separate, versioned normalization record. A constrained analysis agent drafts structured labels and cites exact comment spans/trace fields as evidence. An authorized human curator (initially Drew; otherwise a designated human reviewer) accepts, edits, or rejects that draft. Only human-approved normalized labels feed retention/ranking/product decisions or golden-set promotion. Preserve both the raw comment and every normalization revision; never overwrite the user's words or silently convert an agent guess into ground truth.

Initial controlled labels include: `relevant`, `irrelevant`, `wrong_query_interpretation`, `wrong_rank_or_stage`, `stale_or_superseded`, `wrong_source_or_provenance`, `missing_context`, `duplicate_content`, `incomplete_result`, `unsafe_or_misclassified`, and `other`. Store polarity, target scope, evidence spans, confidence, normalizer model/version, reviewer and review time. A human may add a new label through a schema-versioned change; free-form label proliferation is not allowed.

## 6. Signal-to-decision map

| Signal | Permitted decision / next action | Guardrail |
| --- | --- | --- |
| Low shard-stage survival for a file/embed version | Inspect shard K, filters, candidate generation, and retrieval configuration; propose a controlled parameter experiment under #5774. | Compare only complete traces and equivalent cohorts; a low funnel count is not proof of irrelevance. |
| Strong shard survival but weak final top-1/2/3/5 placement | Investigate merge/rescore stage and run offline comparisons for the existing #5733/#5741 signals and #5774 parameters. | Never change a scorer from aggregate counts alone; use a versioned eval set/gate (#6046) before promotion. |
| Repeated, human-approved positive feedback on opened results | Evidence to retain the file/version, protect it from premature cold-tier/deletion, or promote a representative result into the golden set. | Positive votes are evidence, not an automatic exemption from classification, source deletion, or retention policy. |
| Repeated, human-approved negative feedback on opened results, normalized as stale/duplicate/unsafe | Candidate for source refresh, deduplication, classification review, or a retention dry-run under #6041. | Never auto-delete, auto-demote, or cross a tier boundary from thumbs alone; deletion rules (#6054 and source policy) still win. |
| Normalized `wrong_query_interpretation` / `wrong_rank_or_stage` | Product/search-pattern work: query planner, expansion, weights, or rescore experiments; create a follow-on when it needs code. | Keep #5774/#5733/#5741 as the owners of those interfaces and algorithms. |
| Normalized `wrong_source_or_provenance`, `missing_context`, `incomplete_result` | Ingestion, chunk-boundary, source-mark, or content-version follow-up; distinguish a corpus defect from a ranking defect. | Do not compensate for missing/incorrect corpus data by secretly changing ranking weights. |
| Query-level negative vote or approved essay with a corrected expected result | Candidate for #6046's versioned golden set and an eval case; correct the expected result through human review. | The query vote is not broadcast to every candidate; keep evaluator provenance and corpus version. |
| Click/open/copy/dwell/requery from #6047 | Triage and sampling: identify candidate cases for review or eval-set consideration. | Position-biased implicit signals are not direct ranking weights or standalone retention triggers. |
| Aggregate statistics/reports | Ingest as the #6018 `statistics and reports` source; use longitudinal coverage/freshness and report trends for product planning. | Reports retain lineage/schema/cohort provenance and obey tier isolation and deletion rules. |

## 7. Rejected alternatives

- **Recompute traces later from the current corpus:** rejected because scores/ranks depend on the query-time shard heads, embed bytes, model, and parameters; once any changes, the old trace is unknowable.
- **Log only the displayed top-K/final list:** rejected because it makes earlier shard-stage survival and cutoff effects impossible to compute and hides candidates removed before merge/rescore.
- **Store only aggregate counters:** rejected because it loses query/result/version joins, cannot explain anomalies, and cannot support feedback normalization or eval-set promotion.
- **Use file path or rank as result identity:** rejected because paths rename and ranks change; both would bind feedback to the wrong object over time.
- **One pooled thumbs count or a single query-level vote:** rejected because it erases actor trust and cannot say which result/version was judged. Keep result and query scope distinct.
- **Auto-rerank or auto-delete from raw thumbs/agent labels:** rejected due sparse/biased feedback, agent self-feedback, and the destructive cost of a false signal. Require human normalization plus offline eval/dry-run.
- **Discard essay comments or ask users to normalize them:** rejected because context-rich explanation is explicitly valuable and normalization is Atlas maintainers' responsibility.
- **Copy embed/query text into a lower-tier analytics warehouse:** rejected because traces inherit the strictest source/query classification and content-addressed references avoid uncontrolled copies.

## 8. Invariants

1. Every result/feedback association names `query_id`, stable `result_id`, and the exact `embed_version_id`; no join depends on path, rank, or current content.
2. Every score/rank is captured at query time with the stage, shard/variant, parameter set, model/config identity, and corpus snapshot that produced it.
3. A top-K survival statistic is emitted only when the entire relevant candidate list and cutoff state are known; truncation and failures are explicit.
4. Full candidate stage evidence is retained for the trace TTL; aggregation never changes or overwrites the source trace.
5. Raw feedback, actor class, vote revisions, comments, and human-approved normalization remain distinguishable and auditable.
6. Self-feedback and anonymous/unknown feedback never count as independent quality evidence. Agent signals remain separately visible and lower-weight than human signals.
7. No raw vote or implicit signal directly changes ranking, deletes content, overrides source policy, or crosses classification tiers.
8. Trace/report ingestion inherits the highest classification involved. Source deletion/reclassification can remove content and leave only a policy-authorized tombstone.
9. `statistics and reports` are first-class ingestion inputs with provenance, schema version, time, and retention; they are not anonymous counters detached from source decisions.
10. #5774, #5733, #5741, #6060, #3125, and #6026 retain ownership of their stated interfaces/decisions; this document only defines how trace/feedback evidence records and informs them.

## 9. Implementation split

Do not expand #6027 into an implementation umbrella. Work is split by data contract and consumer:

| Follow-on | Scope / dependency |
| --- | --- |
| [#7647](https://github.com/developing-today/code/issues/7647) (existing) | Record `query_embedding_model` and per-candidate `embedding_model`, and surface mixed-model breach evidence under #6060. Integrate these fields into the trace header/candidate schema below. |
| **Query-time trace capture and durable statistics/reports source** (new follow-on to file) | Implement the query header, shard snapshots, full candidate/stage rows, stable result identity verification, content/version pinning, classification-aware persistence, 180-day raw retention, and deletion tombstones. Depends on this design and composes with #7647. |
| **Shard survival and final-rank aggregation** (new follow-on to file) | Build versioned reports from complete traces: shard-stage survival per file/embed and exclusive/cumulative final-rank distributions at 1/2/3/1–5. Depends on durable trace rows. |
| **Explicit result feedback and normalization pipeline** (new follow-on to file) | Add one-action result thumbs on CLI/TUI, service/API, UI, and agent surfaces; query-level scope; optional essay; identity/duplicate/unopened/self-feedback handling; agent-drafted, human-approved normalization. Depends on trace IDs and auth context. |
| [#6046](https://github.com/developing-today/code/issues/6046) (existing) | Consume approved feedback/traces to build and maintain a versioned golden set and gate index/config changes. Depends on explicit feedback and durable trace IDs. |
| [#6047](https://github.com/developing-today/code/issues/6047) (existing) | Capture implicit signals separately, bound to the same query/result/embed-version IDs, with position-bias and privacy rules. It does not replace explicit feedback. |
| [#6041](https://github.com/developing-today/code/issues/6041) / [#6054](https://github.com/developing-today/code/issues/6054) (existing) | Consume surfaced-history aggregates for retention dry-runs and honor source deletion/tombstone behavior; neither may auto-delete from unreviewed feedback. |

The three new follow-ons are required to complete the implementation split; their issue URLs and IDs will be added here and linked from #6027 once GitHub content creation is available. The design itself is complete without re-deciding the adjacent work.

## 10. Validation checklist

- [x] Trace header has query ID, raw/rewritten variants, actor context, exact parameters, build/config identity, and shard snapshots.
- [x] Each result has per-stage shard/merge/rescore score/rank and explicit cutoff/pruning state, exact embed-content hash/reference/state, version, provenance, and tier.
- [x] Trace is a `statistics and reports` ingestion source with retention, classification, and deletion behavior.
- [x] Statistics define shard-stage per-file/embed survival and final best-rank distributions for 1, 2, 3, and 1–5 with complete denominators.
- [x] Every result surface has one-action thumbs; essay comments are optional and normalization belongs to Atlas maintainers with human approval.
- [x] Identity weights, query/result scope, unopened feedback, duplicate votes/comments, and agent self-feedback are explicit.
- [x] Each signal maps to a bounded retention, ranking, or product decision; alternatives and invariants are stated.
- [x] #5774, #5733, and #5741 are cited as dependencies, not decisions to re-decide.
