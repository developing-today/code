# Design: Atlas golden-set builder + eval gate

- **Issue:** #6046 (golden-set builder + eval gate)
- **Parent:** #6027 (retrieval statistics, feedback, and query traces)
- **Related:** #5741 / PR #6262 (atlas-core offline eval harness `atlas-eval/v1`), #5774 (retrieval parameter surface)
- **Status:** Design record. No implementation in this document's PR.
- **Oracle role:** Test oracle for retrieval evolution (per repo testing doctrine).

---

## 1. Problem and Objective

Atlas combines multiple retrieval signals (dense embeddings, FTS5/BM25, SPLADE, Jaccard rescoring) over repository code and documentation. Evolving retrieval components—modifying chunking boundaries, adjusting rescoring weights (#5774), swapping embedding models, or tuning candidate fan-out limits—must not rely on subjective inspection or ad-hoc queries.

Per the repository testing doctrine, every automated verification requires a decidable oracle. For retrieval quality, the oracle cannot be a static unit-test assertion; it is a **versioned golden evaluation set** coupled with **decision procedures over quantitative retrieval metrics** (nDCG@10, Recall@100, MRR@10).

This document specifies:
1. **Golden-set schema and versioning:** The data format feeding the landed `atlas-eval/v1` harness (#5741 / PR #6262).
2. **Promotion flow:** How negative user feedback (thumbs-down) and sampled live queries from parent issue #6027 are promoted into verified eval entries through human or agent confirmation.
3. **Eval-gate contract:** Which change classes must execute the eval gate, which metrics are evaluated, and the exact regression deltas that block a change from merging.
4. **Maintenance lifecycle:** How stale queries and vanishing chunk references are detected, repaired, or retired as the repository corpus mutates.

---

## 2. Consuming Harness and Test Oracle Contract

### 2.1 The Consuming Harness (`atlas_core::eval`)
Issue #5741 built and landed the offline evaluation engine in `pkgs/atlas-search/crates/atlas-core/src/eval.rs` (PR #6262, commit `c26e0585add36770e90d7b24480b9bb5b3564c5e`), executed via the CLI binary `atlas-eval <eval.json> <dev|test>`.

The golden-set builder does **not** create a separate or parallel evaluation harness. It produces input documents adhering directly to the landed `atlas-eval/v1` format:
- `queries`: Array of `{ "id": string, "class": string, "split": "dev" | "test", "text": string }`.
- `qrels`: Array of `{ "query": string, "doc": string, "grade": u32 }` where grade $\in [0, 30]$ and relevant $\ge 1$.
- `runs`: Evaluated signal candidate rankings `{ "signal": string, "results": { [query_id]: [doc_id, ...] } }`.

### 2.2 The Oracle Decision Procedure
Under the testing doctrine:
- **Oracle:** The versioned golden set (`queries` + ground-truth `qrels`) represents the ground truth of user intent.
- **Decision Procedure:** Comparing the candidate run ($R_{\text{cand}}$) against the baseline run ($R_{\text{base}}$) using metric delta thresholds $\Delta = M(R_{\text{cand}}) - M(R_{\text{base}})$. A change is rejected if $\Delta$ violates the gate contract (§5).

---

## 3. Golden-Set Schema and Versioning

### 3.1 Metadata-Enriched Schema
While the runner consumes `atlas-eval/v1`, the golden-set source repository maintains rich provenance metadata to enable auditability, freshness tracking, and confirmation attribution.

The master golden set is stored at `pkgs/atlas-search/eval/golden/atlas-golden-set.json` with the following schema:

```json
{
  "$schema": "atlas-golden-set/v1",
  "version": "1.2.0",
  "updated_at": "2026-11-08T10:45:00Z",
  "corpus_commit": "2d0150203d8ced0a880ea9215826571d03fd1716",
  "target_size_bounds": {
    "min": 50,
    "max": 200
  },
  "entries": [
    {
      "id": "q-code-auth-042",
      "class": "code",
      "split": "dev",
      "query_text": "claude-code oauth refresh token rotation write-back",
      "provenance": {
        "source": "negative_feedback",
        "trace_id": "tr-20261106-1818-0990",
        "session_id": "ses_e5e45419c71ffeATOufSycEqWJ",
        "promoted_at": "2026-11-06T14:30:00Z",
        "confirmed_by": "agent:repo-worker-code",
        "confirmation_notes": "User flagged missing refresh token write-back logic; ground truth chunk identified in claude-code-auth.ts"
      },
      "qrels": [
        {
          "doc_id": "chunk_sha256_9f8a12...:packages/ai/src/providers/claude-code-auth.ts:35-85",
          "file_path": "packages/ai/src/providers/claude-code-auth.ts",
          "span": [35, 85],
          "content_hash": "blake3:7b8f9e...",
          "grade": 3
        },
        {
          "doc_id": "chunk_sha256_3d21b4...:packages/ai/src/quota/providers/claude.ts:70-95",
          "file_path": "packages/ai/src/quota/providers/claude.ts",
          "span": [70, 95],
          "content_hash": "blake3:1c4a02...",
          "grade": 1
        }
      ]
    }
  ]
}
```

### 3.2 Compilation to `atlas-eval/v1`
A simple compilation step (`atlas-eval-compile`) strips internal provenance and emits the exact `atlas-eval/v1` JSON structure for evaluation runs:
- `doc_id` is passed as `doc` in `qrels`.
- Query `id`, `class`, `split`, and `query_text` (as `text`) populate `queries`.
- `runs` are **not** compiled from the master schema — the master set versions ground truth (`queries` + `qrels`) only, and the harness scores supplied rankings without executing retrieval (`EvalData.runs` is required input, §2.1). Rankings enter compilation as separate, freshly generated run files (§3.2.1); compilation merges them in (§3.2.2) and rejects stale or unpaired ones (§3.2.3).

#### 3.2.1 Run generation — where `runs` come from
A run-generation step (`atlas-eval-run`) executes every golden query against a pinned corpus/index snapshot and saves the resulting rankings, one file per run, in the exact `Run` shape the harness consumes: `{ "signal": string, "results": { [query_id]: [doc_id, ...] } }`, with up to the top-1000 doc_ids per query (the depth `Recall@1000` is scored at) in the same `doc_id` namespace as the qrels. The harness itself never runs retrieval, so a ranking that was not generated and saved by this step does not exist as far as the gate is concerned.

For a gated change, two runs are generated per signal under test:
- **Baseline run** — produced by the pre-change retrieval stack (main-branch code and configuration) and named `<signal>@base`.
- **Candidate run** — produced by the change's retrieval stack (the PR's code and configuration) and named `<signal>@cand`.

Both runs execute against the **same** corpus snapshot: the index built from the master schema's `corpus_commit`, over the identical query set and split selection. The only permitted difference between a baseline/candidate pair is the retrieval stack itself; a pair generated over different corpus snapshots measures corpus drift, not the change, and is invalid (§3.2.3). Each run file carries a sidecar manifest recording the producing stack's commit, the signal/configuration hash, the `corpus_commit` it ran against, and `generated_at`, so any reported metric can be traced to the exact rankings and stack that produced it.

#### 3.2.2 Compilation inputs and outputs
`atlas-eval-compile` takes the master golden set plus one or more run files and emits a single `atlas-eval/v1` document whose `runs` array is exactly the supplied runs (signal names preserved, including the `@base`/`@cand` suffixes the §5.3 delta procedure pairs on). Compilation performs no retrieval and invents no rankings: with no run files it refuses to emit a gate document at all, because the result would fail `EvalData` validation for missing `runs` — a queries/qrels-only document is a set-definition artifact, not an evaluation input.

#### 3.2.3 Freshness and pairing rules
- **Runs are generated per gate invocation, never versioned as ground truth.** Only `queries` + `qrels` are committed in the master set; run files are build/CI artifacts retained with the gate report for reproducibility. A saved ranking from an earlier change is never reused as a baseline or candidate — it was produced by a different stack or corpus and would score the gate against stale rankings.
- **Stale-ranking rejection:** compilation fails if any run file's manifest `corpus_commit` differs from the master schema's `corpus_commit`, or if a run names a query id absent from the master set, repeats a doc within one ranking, or duplicates another run's signal name (mirroring `EvalData::validate`).
- **Pairing completeness:** in gate mode, compilation fails unless every signal present as `<signal>@cand` also has its `<signal>@base` counterpart (and vice versa), so the §2.2/§5.3 deltas always compare a candidate against its own freshly generated baseline.

### 3.3 Set Size and Split Distribution
- **Target Size:** Kept between **50 and 200 real queries**. A corpus smaller than 50 lacks statistical variance across query classes; a set larger than 200 introduces maintenance drag and evaluation latency.
- **Split Ratio:**
  - **70% Dev split:** Available for iterative experimentation, parameter tuning, and ablation studies.
  - **30% Test split (Held-out):** Strictly reserved for final change verification. Never used for tuning or signal selection (`select_best_signal` explicitly rejects test reports).
- **Class Stratification:** Queries must span four standard classes:
  1. `code` (~40%): Precise symbol lookups, function implementations, type definitions.
  2. `documentation` (~25%): Markdown docs, architectural records, design specs.
  3. `error` (~20%): Diagnostic messages, stack traces, compiler errors, failure modes.
  4. `concept` (~15%): Multi-file abstractions, architectural patterns, workflow mechanics.

---

## 4. Promotion Flow from Feedback

The fuel for the golden set is user dissatisfaction (negative feedback) and real session queries.

### 4.1 Assumptions on Parent Issue #6027 (Telemetry and Traces)
The builder assumes issue #6027 implements query-trace and feedback logging with at least:
1. `trace_id`: Unique identifier for each query execution.
2. `query_text`: Literal query string issued by user or agent.
3. `results_returned`: Ordered list of `doc_id`s presented in top-k results.
4. `feedback`: Rating event (`thumbs_down`, `thumbs_up`, or click/selection on a rank > 5).
5. `user_correction`: Optional explicit user text indicating what was expected or missing.

### 4.2 The Promotion Pipeline

```
[Issue #6027 Traces]
        │
        ▼ (Filter: thumbs_down OR (no_click AND follow_up_refinement))
[Candidate Queue]
        │
        ▼
[Relevance Identification] ───► Agent searches codebase for true answer chunk
        │
        ▼
[Confirmation Step]        ───► Human / Reviewer Agent validates:
                                1. Is query reproducible and sensible?
                                2. Are qrels accurate (grade 1..3)?
                                3. Does it cover an uncovered failure mode?
        │
        ▼
[Split Assignment]         ───► Stratified assignment (70% Dev / 30% Test)
        │
        ▼
[Golden Set Commit]        ───► Added to atlas-golden-set.json via PR
```

### 4.3 Detailed Steps

1. **Extraction (Automated):**
   - The candidate extractor pulls all traces tagged with `feedback: "thumbs_down"`.
   - Sampled normal queries (top 5% by frequency across query classes) are also ingested to prevent the set from skewing entirely to impossible queries.

2. **Ground-Truth Identification:**
   - For a candidate query $Q$, an offline diagnosis subagent inspects the session context where the failure occurred.
   - It identifies the exact file and span containing the correct response that retrieval failed to surface in top-10.
   - Initial grades are assigned:
     - `3`: Canonical source (the exact function, design paragraph, or error handler).
     - `2`: Strong context (directly relevant caller, test case, or related doc section).
     - `1`: Weak context (peripheral mention or background concept).

3. **Confirmation Step (Human or Guard Agent):**
   - Candidate entries cannot enter the golden set automatically without confirmation.
   - Confirmation verifies:
     - The query is not ephemeral or garbage (e.g., accidental paste of a 5MB base64 string).
     - At least one chunk has grade $\ge 2$.
     - The chunk's `content_hash` is recorded against the current tree commit.

4. **Split Assignment:**
   - New entries are assigned to `dev` or `test` split to preserve the 70/30 distribution across each query class.

---

## 5. Eval-Gate Contract per Change Class

The eval gate runs on CI or local pre-merge verification before any retrieval-impacting change is merged to `main`.

### 5.1 Gated Change Classes

| Change Class | Examples / Surface | Primary Risk | Eval Requirement |
|---|---|---|---|
| **Rescore Weights** | #5774 parameter surface (`dense_weight`, `bm25_weight`, `splade_weight`, `jaccard_bonus`, fusion exponent) | Overfitting to one query type; degrading code retrieval for docs | Full dev + test run on static embeddings |
| **Chunking Policy** | Span length (e.g. 512 vs 1024 bytes), syntax-directed split points, header prepending | Boundary fragmentation; loss of context precision | Requires re-chunking golden docs; full dev + test run |
| **Embedding Model** | Upgrading dense encoder (e.g., nomic to bge), dimension changes, fine-tuned weights | Latent space drift; rank inversion | Full re-indexing of test corpus chunks; full dev + test run |
| **Fan-Out Limits** | Candidate retrieval limits (top-k dense pool, top-k sparse pool before reciprocal rank fusion) | Latency vs recall tradeoff; truncation of relevant chunks | Fast dev run over multi-signal fusion |

### 5.2 Metrics and Evaluation Criteria

Metrics are evaluated using `atlas_core::eval`:
- **nDCG@10:** Primary ranking quality metric (graded gains $2^{\text{grade}} - 1$, $\log_2$ discount).
- **Recall@100:** Candidate pool completeness (fraction of relevant docs found in top 100).
- **Recall@1000:** Extreme candidate pool ceiling.
- **MRR@10:** First-hit speed for reciprocal rank.

### 5.3 Blocking Deltas (The Decision Procedure)

Let $\Delta M = M_{\text{candidate}} - M_{\text{baseline}}$ on the evaluation set.

1. **Overall nDCG@10:**
   - **Dev Split:** $\Delta \text{nDCG@10} \ge -0.005$ (at most 0.5% drop allowed during multi-objective trade-offs).
   - **Held-out Test Split:** $\Delta \text{nDCG@10} \ge 0.000$ (strictly zero regression allowed on held-out test).
2. **Per-Class nDCG@10 Floor:**
   - For every class $c \in \{\text{code}, \text{documentation}, \text{error}, \text{concept}\}$:
     $\Delta \text{nDCG@10}_c \ge -0.020$.
   - **Protection Rule:** A change that boosts documentation retrieval (+0.05) by cannibalizing code retrieval (-0.03) is **blocked**.
3. **Recall@100 Gate:**
   - $\Delta \text{Recall@100} \ge -0.005$ overall.
   - Candidate generation changes must not discard relevant chunks before fusion.
4. **Empty Ranking Penalty:**
   - Any query producing 0 results where judged relevant docs exist in the corpus immediately fails the gate (regression count $> 0$).

---

## 6. Stale-Entry Retire and Repair Rules

A static golden set degrades as code is refactored, renamed, or deleted ("corpus drift"). Without maintenance, the eval gate will fossilize or fail due to ghost chunk references.

### 6.1 Staleness Audit Trigger
The staleness audit executes:
1. Whenever the repository undergoes major refactor PRs affecting paths referenced in `qrels`.
2. As a weekly automated queue task (`repo-health-code-pilot`).

### 6.2 Detection Mechanics
For each entry in `atlas-golden-set.json`:
1. Check if `file_path` exists on `origin/main`.
2. If `file_path` exists, compute current BLAKE3 hash of `span`.
3. If hash matches `content_hash`, entry is **FRESH**.
4. If hash diverges or file is moved, flag as **DRIFTED**.
5. If file is deleted, flag as **ORPHANED**.

### 6.3 Resolution Policies

```
                 [Audit Check]
                       │
         ┌─────────────┼─────────────┐
         ▼             ▼             ▼
      [Fresh]      [Drifted]     [Orphaned]
     (Keep as-is)      │             │
                       ▼             ▼
              [Semantic Equiv?]  [Alternative exists?]
                 ├── Yes: Update span & hash
                 └── No:  Retire entry
```

1. **Auto-Repair (Span Relocation):**
   - If git blame / log shows lines were shifted (line insertions elsewhere in file) but the semantic block remains byte-identical, the builder automatically adjusts `span` and preserves the `qrel`.
2. **Re-judging (Semantic Modification):**
   - If the referenced code was substantially edited, an agent or human reviewer is alerted with the diff.
   - If the new implementation still answers the query, `content_hash` and `span` are updated.
   - If the feature was completely removed, the `qrel` is marked retired.
3. **Retirement:**
   - A query where all $qrels \ge 2$ become orphaned is moved to `retired_entries` in `atlas-golden-set-archive.json`.
   - It is immediately removed from `atlas-golden-set.json` so it does not distort scoring.
4. **Capacity Management:**
   - If active queries exceed 200, the builder runs a redundancy analysis.
   - Queries with identical top-10 result profiles and identical class representations are pruned, keeping the query with higher diagnostic value (e.g. from negative feedback over random sample).

---

## 7. Implementation Boundaries & Follow-ups

This design explicitly separates concerns:
- **In Scope (Settled by this record):**
  - Contract and schema for `atlas-golden-set.json`.
  - Format compilation into `atlas-eval/v1` consumable by `atlas_core::eval`.
  - Promotion requirements and confirmation gates from #6027.
  - Quantitative gate thresholds per change class.
  - Drift and staleness resolution rules.
- **Out of Scope (Follow-up Tasks):**
  - Implementation of `atlas-golden-builder` CLI / service (follow-up issue under #6027).
  - Implementation of CI GitHub Action running `atlas-eval` on PRs (follow-up issue).
  - Telemetry ingestion backend in PM stack / Atlas daemon (owned by #6027).
