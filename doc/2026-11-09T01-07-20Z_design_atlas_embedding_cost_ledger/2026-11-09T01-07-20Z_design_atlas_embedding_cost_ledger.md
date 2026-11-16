# Atlas embedding cost ledger: per-batch token/$ accounting with a delta reuse-ratio alert

Issue: [#6058](https://github.com/developing-today/code/issues/6058)
Parent: [#6019](https://github.com/developing-today/code/issues/6019)
(semi-automated ingestion operations — the ledger is the embedding-side
companion to #6019's plan-headroom accounting for Turso storage/writes).
Status: design only. No implementation is landed or claimed by this
document; the ledger is implemented after the delta protocol (#6028)
lands, because the ledger's chunk classification is defined by it.

## Intent

Embedding a corpus is cheap exactly once. Published prices verified for
#6058 (Sept–Oct 2026): OpenAI text-embedding-3-small $0.02/1M tokens
($0.01 batch), text-embedding-3-large $0.13/1M, self-hosted BGE-M3
≈ $0.001/1M at high GPU utilization. At corpus scale (~107,639 chunks ≈
27–54M tokens) a _full_ re-embed costs $0.27–$7.00 depending on route —
a rounding error against the standing plan cap.

The cost risk is therefore not the corpus; it is **churn**: re-embedding
the unchanged 95% on every version of every log, forever, amplified by
5–20× corpus growth from comments/docs expansion. Storage and DB-count
quotas bind before embedding spend does — _if deltas work_. The ledger
exists to prove, per batch and per source class, that deltas do work,
and to say so loudly the moment they stop working. It is an accounting
record with one alert rule, not a budget system.

## Decision

1. **One ledger row per ingest batch, append-only.** A row is written
   when a batch is published (and, with a distinct outcome marker, when
   a batch is rejected at the staging/publish gate, so failed ingests
   are accounted too).
2. **The batch key is #6045's batch key.** The ledger does not mint a
   second batch identity: `batch_id` is the same identifier the
   per-batch attestation record (#6045) carries, so a ledger row, its
   attestation, and its chunks join on one key.
3. **New vs reused is #6028's classification, not a new one.** A chunk
   is _reused_ when the delta protocol's span matching anchors it to a
   prior embed (including spans whose head/tail matched prior embeds);
   it is _new_ when the batch actually embeds it. The ledger records
   the counts the delta protocol already computes; it never re-derives
   them.
4. **Cost is frozen at write time.** Each row stores the model and the
   computed `usd` for the batch, priced with the unit price in force
   for that batch. Route changes — the secret-tier local embedder
   (#6059), Modal batch capacity (#6069), a hosted price change —
   apply to later rows only and never rewrite history.
5. **The reuse ratio is a protocol health signal, not a spend signal.**
   The single alert rule below fires on protocol suspicion, and its
   text says so.

## Ledger schema (per batch)

| Field               | Type                            | Meaning                                                                                                                                                                                                                |
| ------------------- | ------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `batch_id`          | string (key)                    | The batch key shared with the #6045 attestation record. One row per `batch_id`; re-emission of the same batch is idempotent on this key.                                                                               |
| `recorded_at`       | UTC timestamp                   | When the ledger row was written (batch publish/reject time).                                                                                                                                                           |
| `source_class`      | enum (below)                    | The class of source the batch ingested.                                                                                                                                                                                |
| `chunks_new`        | integer ≥ 0                     | Chunks this batch embedded (no prior-embed span match under #6028).                                                                                                                                                    |
| `chunks_reused`     | integer ≥ 0                     | Chunks anchored to prior embeds by #6028 span matching; not re-embedded, not re-tagged, not re-dated.                                                                                                                  |
| `tokens_embedded`   | integer ≥ 0                     | Tokens actually sent to the embedder: tokens of `chunks_new` only. Reused chunks contribute 0.                                                                                                                         |
| `model`             | string                          | Embedder identity for the batch (e.g. `text-embedding-3-small`, the #6059 local model id, a #6069 Modal route id), as reported by the route that ran.                                                                  |
| `usd`               | decimal ≥ 0                     | Computed cost of the batch: `tokens_embedded × unit price(model, recorded_at) / 1,000,000`, frozen at write time.                                                                                                      |
| `usd_per_1m_tokens` | decimal ≥ 0                     | The unit price used for `usd`, stored with the row so the computation is auditable after price changes. Local/self-hosted routes record their computed unit cost the same way (0 is not a valid shortcut for "local"). |
| `outcome`           | enum: `published` \| `rejected` | Whether the batch passed the staging/publish gate. Rejected batches still consumed embedding spend if they embedded before rejection.                                                                                  |
| `attestation_ref`   | string                          | Reference to the batch's #6045 attestation record (same `batch_id`; stored explicitly so the join survives either store moving).                                                                                       |

### Source classes

The initial class set follows #6019's source set; classes are added by
amending this design, never by free text at write time.

| `source_class`     | Append-mostly? | Notes                                                                |
| ------------------ | -------------- | -------------------------------------------------------------------- |
| `code-repo`        | no             | Edits rewrite files; moderate reuse expected, no alert baseline yet. |
| `docs`             | no             | Hand-edited; same posture as `code-repo`.                            |
| `logs`             | **yes**        | Append-only operational logs — the canonical churn risk.             |
| `chat-transcripts` | **yes**        | Sessions append; history is immutable once written.                  |
| `comments`         | **yes**        | Issue/PR comment streams append per thread.                          |

"Append-mostly" is a property of the class, recorded here, because the
alert rule is defined only over append-mostly classes: for them,
unchanged-prefix content dominates every batch by construction, so a
low reuse ratio cannot be explained by the source's shape.

## The reuse-ratio alert rule

For every published batch in an append-mostly source class:

```
reuse_ratio = chunks_reused / (chunks_new + chunks_reused)
ALERT when reuse_ratio < 0.90
```

The alert's framing is fixed and is part of this design:

> **Delta protocol broken, not budget.** On an append-mostly source,
> reuse below 90% means the delta path (#6028) failed to recognise
> unchanged content — spans were not matched, content was re-embedded
> wholesale, or anchoring broke. Investigate the protocol, not the
> invoice: at current prices even a full re-embed of the whole corpus
> is single-digit dollars, while silent churn compounds with corpus
> growth and burns the storage/write quotas that actually bind.

Guard rails on the rule itself:

- Batches with `chunks_new + chunks_reused` below a small floor
  (initial slot: 20 chunks, tunable by amendment) do not alert — a
  1-of-3 batch is noise, not a protocol verdict.
- Rejected batches never alert (they are accounted, not judged).
- The alert is per batch and names `batch_id`, `source_class`, the two
  chunk counts, and the computed ratio — everything needed to open the
  batch's attestation (#6045) and the delta run that produced it.
- There is no spend alert. Dollar thresholds belong to #6019's
  plan-headroom accounting; duplicating them here would train operators
  to read a protocol failure as a budget event.

## Per-source-class baseline slots (unpopulated)

Baselines turn the fixed 0.90 rule into a class-specific expectation
over time. The slots are defined now and **left unpopulated**: a
baseline invented before real ingest batches exist would be fiction.
Baselines are populated from the first real ingest batches — which
wait on the first-pass corpus ingestion (#6025) running over the delta
protocol (#6028).

| `source_class`     | `baseline_reuse_ratio` | `baseline_window` | `batches_observed` | Status                                 |
| ------------------ | ---------------------- | ----------------- | ------------------ | -------------------------------------- |
| `code-repo`        | —                      | —                 | 0                  | UNPOPULATED — blocked on #6025 / #6028 |
| `docs`             | —                      | —                 | 0                  | UNPOPULATED — blocked on #6025 / #6028 |
| `logs`             | —                      | —                 | 0                  | UNPOPULATED — blocked on #6025 / #6028 |
| `chat-transcripts` | —                      | —                 | 0                  | UNPOPULATED — blocked on #6025 / #6028 |
| `comments`         | —                      | —                 | 0                  | UNPOPULATED — blocked on #6025 / #6028 |

When populated, a baseline records the median reuse ratio over a named
window of consecutive published batches for that class. Until then the
fixed 0.90 append-mostly rule above is the only alert in force, and a
class's baseline never loosens it silently: changing the alert
threshold is an amendment to this design, recorded with the baseline
evidence that justifies it.

## Invariants

Every implementation of this ledger is tested against these
invariants; each names its oracle.

1. **Ratio range.** For every row, `0 ≤ reuse_ratio ≤ 1` when computed
   from its counts. Oracle: the stored `chunks_new` / `chunks_reused`
   on the same row.
2. **Count conservation.** `chunks_new + chunks_reused` equals the
   number of chunks the batch presented to the publish gate. Oracle:
   the batch's #6045 attestation subject count.
3. **Tokens count new work only.** `tokens_embedded` is the token
   total of the `chunks_new` chunks; reused chunks contribute 0.
   Oracle: the embedder call log for the batch.
4. **Frozen cost.** A row's `usd` and `usd_per_1m_tokens` never change
   after write, including across model/route changes (#6059, #6069)
   and hosted price changes. Oracle: re-reading the row after a price
   table change shows identical values.
5. **One row per batch.** `batch_id` is unique in the ledger; a
   re-emitted batch updates nothing and creates no duplicate. Oracle:
   row count grouped by `batch_id` (the #6045 key) is 1.
6. **Append-only.** Corrections are new rows referencing the corrected
   `batch_id` (a compensating row), never edits or deletions. Oracle:
   the ledger's write path exposes no update/delete operation.

## Worked example

A nightly `logs` batch presents 10,000 chunks. Delta matching (#6028)
anchors 9,700 to prior embeds; 300 are new, totalling 121,000 tokens,
embedded with text-embedding-3-small at $0.02/1M:

- `chunks_new = 300`, `chunks_reused = 9700`,
  `reuse_ratio = 9700 / 10000 = 0.97` — no alert.
- `tokens_embedded = 121000`, `usd = 121000 × 0.02 / 1e6 = $0.00242`.

The same batch with a broken delta path embeds all 10,000 chunks
(~4.1M tokens, `usd ≈ $0.082`): the dollars are still trivial — and
the alert fires, because `reuse_ratio = 0 < 0.90` on an append-mostly
class. That asymmetry is the design: the ledger's money column is
accounting; its ratio column is the smoke detector.

## Non-goals

- Implementing the ledger (code waits on #6028's batch semantics
  landing; this design fixes the schema so that implementation has
  one target).
- Populating baseline numbers before real ingest runs exist (#6025).
- Changing the delta protocol (#6028) or the attestation record
  (#6045) — the ledger consumes both as they are.
- Budget enforcement, spend caps, or dollar alerts — #6019 owns
  plan-headroom accounting; this ledger complements it per source
  class on the embedding side.

## Follow-ups

- Implementation packet for the ledger, filed when #6028 lands,
  scoped to: row emission at the staging/publish gate (sibling to
  #6045 attestation emission), the alert rule, and baseline
  population from the first #6025 batches.
- Route-cost inputs: when the secret-tier local embedder (#6059) and
  the Modal route (#6069) land, their per-batch unit-cost figures are
  recorded through `usd_per_1m_tokens` with no schema change — that is
  what the frozen-cost rule (Decision 4) is for.
