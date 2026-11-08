# Design: Atlas storage growth model — churn × retention, with calibration of the v1 retention defaults

- **Issue:** #6061 (parent #6019, semi-automated ingestion operations)
- **Consumer:** the retention rule engine under #6026 (mechanics owned by
  #6026 / #6041; this record supplies the defaults it starts from)
- **Inputs consumed, not re-defined:** the ingest cost ledger under #6058
  (per-batch accounting), delta/offset reuse under #6028, the plan envelope
  under #6019.
- **Status:** design record. No implementation in this document's PR, and
  **no destructive retention action is enabled by this design** (§6).
- **Evidence:** issues #6019, #6026, #6028, #6043, #6058 as filed
  2026-11-04; the #6043 decision record
  (`doc/2026-11-05T15-50-41Z_decision_atlas_pitr_vs_retention/`).

## 1. Problem

Atlas keeps every version of every ingested file (#6026), old versions
roll hot → cold, and a version is retired only when policy says so. That
makes storage a function of two things operations can measure — how fast
sources churn, and how many versions retention keeps — and one thing it
must not discover by accident: the #6019 plan envelope (Turso scaler,
$24.92/mo today; storage and database-count quotas bind before embedding
spend, per #6058's costing). #6026 needs per-class retention defaults to
start from, and "keep everything forever, hot" is not a default, it is
the absence of one.

This record gives: the growth model (§2), the inputs and where each is
measured (§3), a worked projection (§4), the calibration procedure that
turns ledger data into retention defaults (§5), and the guardrails that
keep the model advisory until real churn data exists (§6).

## 2. The model

Per source class `s` (the classes are #6026's: code, docs, session logs,
chat logs, generated reports, boot/operational records), over one day:

| Symbol | Meaning |
|---|---|
| `F_s` | files (objects) in the class |
| `λ_s` | new versions per file per day (churn rate) |
| `A_s` | average full size of one version, bytes |
| `ρ_s` | span-reuse ratio of a new version: the fraction of its bytes already embedded as spans of prior versions (#6028's delta reuse ratio, §3) |
| `M_s` | marginal stored bytes of one new version |
| `V̄_s` | average retained versions per file under the class's retention policy |
| `α` | index amplification: stored bytes per content byte for vectors + search index |
| `β_s` | manifest/metadata bytes per version (span references, lineage, occurrence rows) |

A version's marginal storage is only its new spans plus its manifest —
the finding #6061 was filed from:

```
M_s = A_s × (1 − ρ_s) × (1 + α) + β_s
```

Steady-state stored bytes for the class, once retention is binding.
The ground truth is the **referenced span set**: every span any retained
version references must remain stored, including spans introduced before
the retention window. So the estimator is one full version footprint
(the newest version resolves only if all its spans are present) plus
the marginal bytes of each additional retained version:

```
S_s = F_s × [ A_s × (1 + α) + (V̄_s − 1) × M_s ]
```

Using `V̄_s × M_s` alone would understate the footprint exactly when
reuse is high: at `V̄_s = 1` it predicts `M_s` per file, while resolving
that one version requires its full `A_s × (1 + α)`.

split by tier by the class's hot window `H_s` (days) and cold window:

```
hot_s  = F_s × [ A_s × (1 + α) + max(0, min(V̄_s, λ_s × H_s) − 1) × M_s ]
cold_s = S_s − hot_s                            (rustfs tier, per the 2026-11-04 storage ruling)
```

Growth before retention binds (the v1 regime, §5) is the integral of
arrivals: `S_s(t) ≈ F_s × λ_s × t × M_s`, capped at the steady state
above once the oldest versions start retiring.

**Staging is modelled separately.** The staging tier (#5669,
merge-on-close promotion) holds archives that are not yet — and may
never be — corpus versions. Staging storage is `staging_rate ×
mean_residence_time`, reported as its own line, never folded into
`S_s`: folding it in would let a staging backlog masquerade as corpus
growth and distort the retention defaults calibrated in §5.

**Projection against the envelope.** The report the model produces, per
class and in total: current `S_s` by tier, projected `S_s` at 30/90/180
days under the calibrated parameters, the plan's storage quota, and
**months-to-envelope** at the current growth rate. Database count is
tracked alongside (shards are created by policy, not by churn, so it is
a step function reported as current count + planned shards, per #6019).

## 3. Inputs — every one measurable, each from a named source

| Input | Source | Notes |
|---|---|---|
| `λ_s`, `A_s` | ingest ledger (#6058), aggregated per class per day | versions landed and their full sizes, whether or not spans were reused |
| `ρ_s` | **byte-weighted** reuse from the ledger aggregates: `bytes_reused / (bytes_reused + bytes_new)` per class, where the byte sums are over span bytes | #6058 owns the ledger schema — per batch `(chunks_new, chunks_reused, tokens_embedded, model, $)` — but those fields are chunk *counts*, and a count ratio is not the byte fraction §2 requires when span sizes vary (the offset-matching design under #6028 allows variable-size spans and split subspans). The §7 aggregation therefore sums span bytes alongside the counts; the count ratio is never used as `ρ_s` |
| storage delta, write volume | #6019's per-run cost accounting | the ground truth the model's `S_s` is reconciled against (§5, step 4) |
| `α` | measured once per embedding model/index build: (vector + index bytes) ÷ content bytes | re-measured when the model or index changes; not a per-batch ledger concern |
| `β_s` | manifest row size × spans per version, from #6028's span format | small next to `A_s × (1 − ρ_s)` except for tiny, heavily-edited files; kept explicit so that case is visible |
| envelope | #6019 (plan, storage quota, DB quota, $ ceiling) | consumed as a constant between plan changes; a plan change is a recalibration trigger (§5) |

Classes #6026 lists whose files are append-mostly (session logs, chat
logs, boot records) are expected at `ρ_s ≥ 0.9` once delta ingestion
works; #6058's alert rule applies unchanged — reuse below 90% on an
append-mostly source means the delta protocol is broken, not that the
retention defaults are wrong, and calibration (§5) must not "fix" a
broken `ρ_s` by loosening retention.

## 4. Worked example (illustrative parameters, not measurements)

One append-mostly log class: `F = 2,000` files, `λ = 1` version/day,
`A = 200 KB`, `ρ = 0.95`, `α = 0.6`, `β = 2 KB`.

- `M = 200 KB × 0.05 × 1.6 + 2 KB ≈ 18 KB` per version — versus 320 KB
  had the version been stored whole with its index. Reuse is worth
  ~18× on this class; that is why `ρ_s` is a model input and not a
  constant.
- At `V̄ = 90` retained versions: `S ≈ 2,000 × (320 KB + 89 × 18 KB)
  ≈ 3.8 GB`, of which the 14-day hot window (the full newest version
  plus 13 marginal versions per file) is ≈ 1.1 GB.
- The same class at the conservative v1 assumption `ρ = 0` (§5) projects
  `M ≈ 322 KB` and would hit a 10 GB envelope line in ~16 days of
  arrivals — the conservative projection is supposed to look alarming;
  it is the bound that keeps v1 honest until `ρ` is measured, and the
  gap between the two projections is exactly what calibration closes.

## 5. Calibration procedure

**Fields consumed** (all from §3's named sources; no new telemetry is
created by this design): per class per day — versions landed, full
bytes landed, `chunks_new`, `chunks_reused`, storage delta, write
volume. Retention defaults are outputs (§5.3), never inputs fed back
into the same cycle that produced them.

### 5.1 Cadence

- **Weekly** recalibration for the first eight weeks of semi-automated
  ingestion (the period when `λ_s` and `ρ_s` are moving), then
  **monthly**, and additionally on any trigger event: a new source
  class, a plan/envelope change (#6019), an embedding-model or index
  change (`α` re-measure), or a #6058 reuse-ratio alert.
- No recalibration runs on fewer than 14 days of ledger data for a
  class; before that the class runs on the v1 defaults (§5.2) and the
  report says so.

### 5.2 Estimators, and the conservative start

- `λ_s`, `A_s`: trailing-28-day means. `ρ_s`: trailing-28-day
  aggregate of the §3 ratio, per class, never pooled across classes
  (pooling would let a high-reuse log class subsidise a low-reuse docs
  class in the defaults).
- **v1 defaults start conservative (keep more)** — #6061's requirement,
  stated as rules, not intentions:
  1. Until a class has two completed calibration cycles, its projection
     uses `ρ_s = 0` and the upper end of the churn range (mean + 1σ of
     daily arrivals). Retention defaults derived in that regime err
     toward keeping versions.
  2. A class's retention window may only be *shortened* by a
     calibration cycle whose data covers at least one full retention
     cycle at the current window.
  3. **Cold-tier moves precede deletions by at least one full retention
     cycle**: a version class must live cold for a full cycle before
     any deletion trigger is even evaluated against it. Deletion is
     the last step of the lifecycle, never the first response to a
     projection.

### 5.3 Outputs — the retention defaults handed to #6026 / #6041

Per class, each calibration cycle publishes one defaults row:

| Output | Meaning |
|---|---|
| `hot_days` | age below which versions stay hot (also: the latest version is always hot, per #6026's newest-resolution) |
| `cold_days` | age at which cold residence ends and trigger evaluation may begin (≥ `hot_days` + one full retention cycle, per §5.2 rule 3) |
| `max_versions` | the increments-trigger parameter: versions retained per file before the oldest become trigger-eligible |
| `min_age_days` | the date-trigger parameter: no version younger than this is trigger-eligible |
| `projected_months_to_envelope` | the §2 projection, carried on the row so a default is never read without its consequence |

The rule engine (#6026 / #6041) owns the trigger mechanics and the
access trigger (which consumes retrieval statistics, not ledger data —
out of this model's inputs by design). This record names the defaults
and their derivation only.

### 5.4 Reconciliation

Each cycle, the comparison is like-for-like over the same interval:
the projected **change** in `S_s` across the cycle against the measured
storage delta (#6019 accounting) for that same interval — or, where an
absolute footprint is reconstructed from the accounting, projected
`S_s` against that absolute figure. A total must never be compared
with a delta: a correctly predicted corpus with zero net growth would
otherwise register a miss every cycle. A miss of more than 25% in
either direction on the like-for-like comparison is a calibration
defect to be explained in the cycle report (wrong `ρ_s`, unmodelled
metadata, staging leakage into corpus storage) — not silently absorbed
by refitting. Two consecutive unexplained misses freeze default
changes for that class until the cause is found.

## 6. Guardrails

1. **This design enables no destructive action.** It produces defaults
   and projections. A version is retired only under #6026's rule,
   honouring #6043: **a named trigger AND the applicable class rule
   must both authorize retirement** (#6043 decision record, invariant
   3), and retirement must never orphan chunks still referenced by
   retained versions (#3125 occurrence semantics).
2. **PITR is not an input.** Turso point-in-time recovery is database
   disaster recovery (#6043); its windows never appear as retention
   parameters, and the model's cold tier is Atlas policy in rustfs,
   not a restore window.
3. **The envelope is a reporting line, not an automatic brake.** When a
   projection crosses 70% of the storage envelope within 180 days, the
   cycle report surfaces it under #6019's headroom accounting for a
   human decision (plan change, new source-class ruling, or a defaults
   change through §5) — the model never shortens a retention window on
   its own to fit the envelope.
4. **Keep-more is the failure default.** Missing ledger data, a broken
   feed, or an unreconciled projection (§5.4) all leave the previous,
   more conservative defaults in place.

## 7. Follow-ons (implementation, not this record)

- The ledger aggregation job that produces §3's per-class daily rows
  from #6058's per-batch ledger.
- The projection report in #6019's cost-report format (per class, per
  tier, months-to-envelope).
- The defaults-row publisher: versioned defaults rows (§5.3) the
  #6026 / #6041 rule engine reads, with the §5.2 conservative-start
  state per class.

## 8. Open questions

- `α` for the cold tier: cold versions in rustfs may drop the search
  index (resolvable but not searchable, per #6026's cold semantics) —
  if so, cold `M_s` loses the `α` term and the hot/cold split in §2
  needs the two-`α` form. The rule engine's cold-resolvability contract
  (#6026) decides this; the model takes it as a parameter either way.
- Whether generated reports (a class whose versions are re-derivable
  from their inputs) should carry a class rule permitting earlier
  retirement is a #6026 class-rule decision; this model supplies its
  (low) storage numbers and takes no position.
