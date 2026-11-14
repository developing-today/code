# Design: Atlas chunk-boundary policy v2 — structural first

- **Issue:** [#6037](https://github.com/developing-today/code/issues/6037)
- **Parent:** [#6028](https://github.com/developing-today/code/issues/6028), still open
- **Related evidence:** [#6038](https://github.com/developing-today/code/issues/6038) boundary-drift measurement; [#6039](https://github.com/developing-today/code/issues/6039) overlap-budget experiment
- **Status:** policy proposal for follow-on implementation; no chunker or data migration is implemented here.
- **Evidence caveat:** the cited scratch brief `~/workspace/scratch/atlas-ingestion-brief-2026-11-04/prior-art-research.md` is unavailable. The qualitative prior-art finding below is preserved in #6037's body; measured defaults cite the landed #6038/#6039 reports instead.

## 1. Decision

Split at the source's authored structure before using token windows. Use a common 256–512 embedder-token budget (aim 384, hard maximum 512) so chunks remain large enough to carry a coherent unit while bounding embedding input. Set structural overlap to a **0–5% cap** and the fixed-window fallback to **10%**. Do not raise overlap above 20% without a source-specific evaluation showing a gain.

These are defaults, not a claim that every source class has been calibrated. The 5% structural cap is a conservative proposal: a splitter that keeps authored units intact should need less boundary repair than an arbitrary window. The #6039 experiment tested fixed 200-word windows, not structural splitting; it supports 10% for that tested windowing mode, and source-specific evaluation remains due for code, chat, and structurally grouped records.

## 2. Shared sizing and overlap rules

- Count content with the tokenizer used by the selected embedder; store metadata such as heading paths, speakers, and timestamps outside the content-token budget.
- Target 256–512 tokens and aim for 384. Combine adjacent complete units when a chunk is short; split an over-budget unit recursively at the next smaller structural boundary. If no structure remains, use the fixed-window fallback. A single indivisible oversize unit is split as a last resort and marked as such.
- Overlap percentage is measured against the target chunk's content-token count. For structural rows, retain only complete preceding units and only when they fit within the 5% cap; otherwise use zero overlap. Never cut a log record, message, turn, sentence, or declaration merely to manufacture overlap.
- For fixed windows, use token-level overlap at 10%. The general ceiling is 20%; the #6039 result found no measured benefit at 20% over 10% for its corpus and signal.

## 3. Per-source policy

| Source type | Splitter | Size target | Overlap default | Rationale |
|---|---|---|---|---|
| Markdown and prose documents | Heading path → block/paragraph → sentence → token fallback. Prefer ending at a paragraph boundary; carry the heading path as metadata. | **256–512 tokens; aim 384, cap 512.** Rationale: this uses the 256–512-token baseline recorded in #6037; the 384-token aim leaves room for multiple related paragraphs without treating the target as a hard minimum. | **0–5% cap**, whole preceding sentence/paragraph only if it fits. Rationale: structural headings and paragraph boundaries already limit arbitrary cuts, so overlap is only a small context bridge; this is a proposed lower default, not a result measured by #6039. | #6037's preserved research finding favors authored structural boundaries and recursive paragraph→sentence splitting over fixed or semantic splitting for technical text. #6039 measured fixed windows over repo prose: 10% recovered boundary coverage, while 20% added cost without measured coverage gain. Structural splitting is expected to need less overlap than that fixed-window result; validate the 5% cap on a source-specific golden set. |
| Code | Language-aware syntax tree: module/type/function/method definitions first; split oversized definitions at nested blocks/statements, then use token fallback. Store the enclosing symbol path as metadata. | **256–512 tokens; aim 384, cap 512.** Rationale: the shared baseline avoids oversized embedding windows; recursive splitting lets long definitions fit without making short declarations standalone noise. | **0–5% cap**, whole adjacent declaration/context unit only if it fits; otherwise zero. Rationale: keep the enclosing symbol path as metadata rather than duplicating a large parent declaration. | A definition is a stronger retrieval unit than an arbitrary line window, and preserves the relationship between a signature and its body. The issue-body finding favors structural splitting; #6039 explicitly did not measure code, so both size and overlap remain provisional pending a code-specific evaluation. |
| Logs | Parse complete records/events first; group adjacent records from one stream/session in timestamp order until the target is reached. Split an oversized record only within its payload, retaining record identity and timestamp metadata. | **256–512 tokens; aim 384, cap 512.** Rationale: the shared range bounds event groups while allowing short records to be grouped rather than indexed one at a time. | **0–5% cap**, previous complete record(s) only if within the cap; otherwise zero. Rationale: never split or partially duplicate a record for overlap. | Record boundaries preserve event fields and chronology. #6039's fixed-window log slice reached ≥80% boundary-span coverage at 10%; its report notes that an entry-aligned chunker could change the result. Therefore the lower structural default is a hypothesis to measure, not an extrapolation of the fixed-window result. |
| Chat and session stores | Keep sessions separate; group complete turns/messages in order, retaining speaker, timestamp, and session identifiers as metadata. Split an oversized message recursively at sentence/token boundaries while retaining its message ID. | **256–512 tokens; aim 384, cap 512.** Rationale: the shared range fits several short turns yet bounds long transcripts; grouping avoids tiny one-message chunks where turns are brief. | **0–5% cap**, last complete turn/message only if it fits; otherwise zero. Rationale: do not carry context across sessions. | Message/turn boundaries preserve speaker and conversational context, so a token-window overlap should be a limited supplement rather than the primary way to reconstruct context. Chat was not measured by #6039; these defaults require a chat-specific evaluation. |
| Fixed-window fallback | Recursive separators: paragraph → sentence → whitespace/word → embedder-token window. Use only where no reliable source structure is available or after a structural unit must be split. | **256–512 tokens; aim 384, cap 512.** Rationale: this follows the 256–512-token baseline preserved in #6037 and bounds each unstructured window. | **10% token overlap.** Rationale: this is higher than the structural cap because arbitrary windows can bisect a retrievable passage; do not exceed 20% absent measured gain. | #6039 tested fixed 200-word windows on repo prose and grouped git-log entries: 10% reached 1.000 boundary-span recall@10 with ≥80% coverage for both, while 20% added about 23–24% index/embed cost over 0% (about 12% over 10%) without measured coverage gain. The report used BM25 and a provisional 64-query set; dense retrieval and the #6027 golden-set rerun remain outstanding. |

The source-type table follows the issue-body finding that structural boundaries are the cheap default for already-segmented technical text, while semantic splitting adds an embedding pass and can over-fragment. Because the original scratch brief is missing, that qualitative comparison is recorded as an input from #6037, not independently re-validated here.

## 4. Existing chunks and policy changes

A policy change creates a new chunking-profile/generation; it does not rewrite existing chunk rows or vectors in place.

1. Keep the current generation readable while producing a candidate generation for each affected source version under the new policy.
2. Re-chunk and re-embed windows whose content or boundaries change. The #6038 real-corpus report found rank movement even after a one-sentence boundary shift (5.8% top-10 flips in its 50-window pool) and after a neighbor change (10.3–17.8%). Its reuse-safety rule is to reuse a vector only when the **full window text** that produced it is byte-identical; span-text identity alone is insufficient.
3. Evaluate boundary coverage/recall and index/embed cost against the same corpus and query set before cutover. Keep the old generation available for rollback; retire it only after readers and any chunk-ID references have been accounted for.
4. Do not assume that a prior chunk or span survives a new boundary policy. The exact mapping of old spans/IDs, version anchors, and partial-span reuse is provisional pending #6028 (and its span-schema follow-up #6035). The #6038 exact-window rule is a safety constraint for vectors, not a decision about the parent issue's span model.

This record authorizes no live re-chunking, cutover, deletion, or implementation. Those actions require the parent span/version contract and an implementation follow-up.

## 5. Provisional decisions and follow-ups

- **Provisional overlap:** 0–5% for structural splitters versus 10% for unstructured fixed windows. The relative ordering is intentional; per-source measurements may revise the values.
- **Provisional sizes:** 256–512 tokens across source types, based on the issue-body baseline and the #6039 window-scale experiment. Code and chat have not been measured at these sizes.
- **Provisional span interaction:** #6028 remains open. This policy does not settle span identity, version anchoring, or which spans survive re-chunking; coordinate the final rule with #6028/#6035 before implementation.
- **Evaluation owed:** repeat the prose/log comparison with #6027's golden set and dense retrieval when available; measure code and chat separately; compare structural 0–5% against fixed 10% on representative corpora.
- **No chunker exists in `pkgs/atlas-search`:** this is a policy record only. If the settled design requires a chunker in this repository, file a separate implementation packet.

## 6. Sources

- Issue [#6037](https://github.com/developing-today/code/issues/6037), including its preserved qualitative research finding and cited comparison article. The original 2026-11-04 scratch brief is unavailable.
- `pkgs/atlas-search/eval/overlap-report-6039.md` ([issue #6039](https://github.com/developing-today/code/issues/6039), merged PR #7604): fixed 200-word windows, repo prose/log slices, BM25, provisional golden set, and the 10% recommendation with its limits.
- `pkgs/atlas-search/eval/boundary-drift-6038.md` ([issue #6038](https://github.com/developing-today/code/issues/6038), merged PR #7661): measured context drift and the byte-identical full-window reuse constraint.
- [Issue #6028](https://github.com/developing-today/code/issues/6028) and [issue #6035](https://github.com/developing-today/code/issues/6035): open parent and span-schema design dependencies.
