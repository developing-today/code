# Boundary-drift measurement — atlas span reuse (#6038)

**Question (issue #6038, parent #6028):** span reuse assumes a span's
stored vector stays valid when the span is reused in a new version.
But an embedding is a function of the chunk's exact text _including
its window neighbors_: the same paragraph embedded with different
surrounding text is a different vector, and its rank for a given
query can move. How much?

**Answer in one line:** a lot, at every perturbation size measured —
even a one-sentence boundary shift flips top-10 membership for
5.8% of query/target pairs, and replacing one neighbor paragraph
flips 10.3–17.8%. The reuse-safety rule this justifies is at the
bottom: **reuse only byte-identical windows; re-embed on any
context change.**

## Method

- **Corpus:** the real Atlas corpus (`~/atlas/v3/corpus.db`,
  `messages` table; 114,165 messages scanned, 498,564 candidate
  targets). Paragraphs are blank-line splits. A target is a
  paragraph (200–2000 chars, ≥2 sentences) with a previous and a
  next paragraph in the same message. Deterministic sample of 50
  targets, seed 6038. Corpus text stays private: this report
  carries numbers and methodology only — no excerpts, no message
  ids, no query text.
- **Original window W0** = previous paragraph + target + next
  paragraph (blank-line joined) — the production chunk shape.
- **Perturbations** (per target):
  - `boundary_shift` — drop the first sentence of the _previous_
    paragraph only (the target paragraph stays byte-identical and
    the blank-line window shape is preserved); append the first
    sentence of the paragraph after `next` to the end of `next`
    when one exists (49 of 50 targets).
  - `neighbor_replace_prev` / `neighbor_replace_next` — replace
    that neighbor with the target paragraph of another sampled
    message (deterministic donors, offsets +7 / +13).
  - `target_alone` — the target paragraph with all context removed
    (bounding case).
- **Embedder:** the production pipeline, replicated exactly from
  `~/atlas/v3/embed_onnx.py` — pinned Snowflake
  `snowflake-arctic-embed-m-v2.0` (HF snapshot
  `95c2741480856aa9666782eb4afe11959938017f`), ONNX graph,
  AutoTokenizer (`add_special_tokens=True`), right-pad 0 /
  attention-mask 0, CLS pooling, L2 normalize. Documents embedded
  prompt-free (production corpus mode); queries embedded with the
  production `query: ` prompt (`atlas-core` `embed.rs`
  `QUERY_PROMPT`). Embedding 270 windows + 20 queries took 357 s
  on the laptop (16 threads).
- **Queries:** fixed set of 20 from the golden query set v3
  (`~/atlas/v3/golden-queries-v3.json`, every 16th of 311) — the
  same fixed set for every window variant.
- **Rank movement:** pool = the 50 original window vectors. For
  each query, `rank0` is W0's rank in the pool; `rank_p` is the
  perturbed vector's rank when it replaces W0 in the pool.
  Reported over all 1,000 query/target pairs per perturbation.

Reproduce: `pkgs/atlas-search/eval/boundary_drift_6038.py`
(laptop only; needs `~/atlas/v3-venv` and the LD_LIBRARY_PATH in
the script's docstring). Raw results JSON is produced by the run;
the numbers below are its readback.

## Results — cosine drift (1 − cosine similarity vs W0, n = 50 targets)

| perturbation          | median | p95   | mean  | max   |
| --------------------- | ------ | ----- | ----- | ----- |
| boundary_shift        | 0.045  | 0.175 | 0.060 | 0.301 |
| neighbor_replace_prev | 0.292  | 0.554 | 0.306 | 0.745 |
| neighbor_replace_next | 0.121  | 0.360 | 0.147 | 0.438 |
| target_alone          | 0.168  | 0.478 | 0.208 | 0.618 |

## Results — rank movement (pool of 50, 20 queries, 1,000 pairs per type)

| perturbation          | median \|Δrank\| | p95 \|Δrank\| | max \|Δrank\| | top-10 flips       |
| --------------------- | ---------------- | ------------- | ------------- | ------------------ |
| boundary_shift        | 2                | 10            | 22            | 58 / 1000 (5.8%)   |
| neighbor_replace_prev | 7                | 27            | 42            | 178 / 1000 (17.8%) |
| neighbor_replace_next | 4                | 21            | 36            | 103 / 1000 (10.3%) |
| target_alone          | 5                | 19            | 39            | 119 / 1000 (11.9%) |

Notes on the shape of the numbers:

- **No perturbation is rank-stable.** The smallest change measured
  — sliding the window boundary by one sentence — still has a p95
  cosine drift of 0.175 and moves a target's rank by 10+ places
  (in a pool of only 50) for 5% of query/target pairs.
- **The previous neighbor matters more than the next one**
  (median drift 0.292 vs 0.121 for a full replacement). The window's
  opening text is the larger context lever in this model; a design
  that treats both neighbors as equally cheap to change is wrong.
- **Removing all context (`target_alone`) drifts less than
  replacing the previous neighbor with foreign text** (median
  0.168 vs 0.292): wrong context is worse than no context.
- Rank and top-10 flip figures are measured in, and scoped to,
  this 50-window candidate pool; a flip rate is a property of the
  pool it was measured in, not a portable constant. The production
  pool (~100k chunks) is far denser near the top, so the production
  flip rate for a perturbed window will differ (more near-neighbors
  compete for the same ten slots); the figures here establish the
  mechanism and its size at pool scale 50, and the reuse rule below
  is written to hold regardless of the exact production rate.

## Reuse-safety rule (for the span-schema design, #6035)

> **A stored span vector may be reused only when the full window
> text that produced it — the span plus its recorded neighbors —
> is byte-identical in the new version. Any boundary shift or
> neighbor change, however small, invalidates the vector and the
> span must be re-embedded.** There is no measured "close enough"
> context change: the gentlest perturbation tested (one sentence
> of boundary shift) already flips top-10 membership ~1 time in
> 17 (at pool scale 50), and a single replaced neighbor flips it
> ~1 time in 6–10.
> Context identity — not span identity alone — is the reuse key:
> key stored vectors by the hash of the exact window text, and
> treat a context-class change (neighbor added / removed /
> replaced / boundary moved) as a re-embed trigger.

## Limitations

- 50 targets / 20 queries: distributions are indicative, not
  tight; medians are stable at this size, p95s are coarse.
- Donor neighbors are real corpus paragraphs from _other_
  messages — a harsher change than an edited version of the same
  neighbor. An edit-in-place neighbor change would land between
  `boundary_shift` and `neighbor_replace_*`; both endpoints of
  that interval already break rank stability, so the rule does
  not depend on where in the interval it falls.
- One model (the pinned production model), one corpus. The rule
  is stated for this pipeline; another embedder would need its
  own measurement before inheriting it.

## Corrections after Codex review (PR #7661)

The first version of this experiment had three defects, all caught
in Codex review of PR #7661 and fixed before merge:

1. `boundary_shift` originally split the _whole window_ into
   sentences and dropped the first one — when the previous
   paragraph had no terminal punctuation its first "sentence"
   merged into the target's first sentence, so the shift could eat
   into the target, and the window was rebuilt space-joined,
   losing the blank-line shape. The construction now drops only
   the previous paragraph's first sentence, keeps the target
   byte-identical, and preserves the window shape.
2. The model snapshot was selected as whichever HF cache snapshot
   sorted last. It is now pinned to
   `95c2741480856aa9666782eb4afe11959938017f` (the snapshot the
   reported run used — verified the only snapshot in the cache),
   and the script fails loudly if it is absent.
3. The top-10 flip rates were described as "the portable figure".
   They are pool-relative; the text above now scopes them to the
   measured 50-window pool.

The experiment was re-run end-to-end after the fixes. The
`boundary_shift` row above is the corrected run's output; every
other row reproduced the first run's numbers exactly (same seed,
same inputs, deterministic embedder), which is the regression
check that the fixes changed only what they were meant to change.
