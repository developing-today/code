# C4 — Quota forecasting: prediction record, scoring, and methodology log

Status: component design for issue #9572, under epic #9568 (reporting
architecture). Consumes the program design in [`note.md`](note.md) and
the Stage-3 quota-plane design
(`doc/2026-11-22T22-35-00Z_design_quota-plane-stage-3/`). This record is
#9572's acceptance item 1: the prediction record schema, the scoring
rules, and the methodology-change log format. The live-loop remainder
(acceptance items 2–3) is filed as granular issues C4.1–C4.6 (§8).
Author: fleet agent `felix` (kind muse), 2026-11-23. Written against
`origin/main` at `1edebd513`.

Statements answered: S2.3, S2.4, S2.5, S2.6, S2.7, S2.12 (C4 half),
S2.13, S2.14; forecast data out to C6 for S3.15.

## 0. Done check — what already exists, and what this design adds

C4 does not start from zero. The quota Stage-3 line (epic #12694,
landed 2026-11-22/23) built the exhaustion forecaster and its scoring
harness for the quota plane's own purposes:

- **S1 (#12695, PR #12724)** — `quota-forecast.json` published per
  gatherer tick: per pool/window, the `quota-analytics.ts`
  least-squares pace projection promoted to data (projected wall
  instant, burn rate, sample count, honesty state), plus one Turso
  ledger row per pool/window.
- **S2 (#12696, closed)** — `quota-forecast-accuracy.ts`: accuracy
  ledger + backtest over the snapshot store (20,663 records,
  2026-10-20 → 2026-11-23; 5,028 forecasts scored). Scores projected
  wall error, wall-before-reset precision/recall, first-call lead.
  Headline: 5h windows calibrate well; weekly windows run
  systematically late under linear pace.
- **S3 (#12697), S5 (#12699), S6 (#12700)** — shadow placement, anomaly
  detectors, re-weighting proposals: consumers and neighbours of the
  forecast, not C4 predictions themselves.

Against #9572's scope, that covers exactly one prediction kind —
per-pool exhaustion as a *point* projection — with scoring. It does
not cover, and this design specifies:

1. **A prediction record of record.** S1's forecast rows carry no
   `basis` (S2.14), no range as a first-class value, and no
   methodology version; they are a placement input, not the logged,
   scored prediction S2.5 demands for every C4 kind.
2. **Complete-downtime windows** (S2.4) — no component predicts the
   all-pools-walled interval.
3. **Reset durations as ranges** (S2.13) in Drew's formats — nothing
   predicts or renders them.
4. **Day-type and spike-day adjustments** (S2.6, S2.7) — the Stage-3
   design (§2, §8 alternative B) deliberately deferred these: at
   33 days of history each weekday cell held ~4–5 samples per window.
   This design defines the models and the sufficiency gate that
   admits them; it does not waive the gate.
5. **New model/plan arrival** (S2.3, second half) — S5 *detects* plan
   changes after the fact; nothing predicts arrivals.
6. **Forecast push alerts** (S2.12, C4 half) and the
   **methodology-change log** (S2.5's "methodology updates are
   themselves logged") — neither exists as a format.

**Gate check.** #9572 is not gated on the C3 chain (#9571 / PR
#12226). C4's substrate is the gatherer, the snapshot store, and the
Stage-3 forecast product — all live. Forecast alerts ride the
existing alerts ledger (the S5 `postIncident` precedent), not C3's
hook pipeline. Work gated on #12226 stays gated; none of it is C4's.

## 1. The prediction record

One record per prediction, append-only, written per gatherer tick
(or on the event that prompts it, for arrivals). A record is never
edited; a newer prediction for the same subject *supersedes* by id
link, so the accuracy loop can score what was actually believed at
each moment.

```jsonc
{
  "record_version": 1,
  "id": "c4:<kind>:<subject-key>:<issued-at-epoch-ms>",
  "kind": "pool_exhaustion" | "downtime_window"
        | "reset_duration" | "model_plan_arrival",
  "subject": {
    "pool": "<canonical provider id>",       // pool kinds
    "window": "<window name>",               // where windowed
    "pools": ["<provider id>", "..."],       // downtime_window: the set
    "plan_or_model": "<name>"                // arrival kind, when named
  },
  "issued_at": "<ISO-8601 UTC>",
  "basis": {
    "name": "trailing_30d" | "since_last_reset" | "since_config_change"
          | "past_day_vs_30d" | "reset_cadence" | "detector_history"
          | "vendor_cadence",
    "detail": "<what the basis was computed over, in words>",
    "config_change_ref": "<issue/PR ref>"   // since_config_change only
  },
  "inputs": {
    "series": "<snapshot-store series key(s) the basis read>",
    "samples": 0,                            // readings behind the basis
    "span": { "from": "<ISO>", "to": "<ISO>" },
    "adjustments": ["day_type:<cell>", "spike_day"]  // §5; [] in v1
  },
  "prediction": {
    // A range, always. A point estimate is the degenerate case and is
    // still recorded as a range (earliest == latest is refused; a
    // model that cannot bound its answer emits an honesty state).
    "earliest": "<ISO-8601 UTC>",
    "latest":   "<ISO-8601 UTC>",
    "likely":   "<ISO-8601 UTC>",           // optional point inside
    "rendered": "<the range in Drew's formats, §4 — stored, so what
                 was shown is exactly what was scored>"
  },
  "methodology_version": "<id from the methodology log, §6>",
  "state": "predicted" | "insufficient_history" | "no_basis"
         | "adjustment_unavailable",
  "state_reason": "<named reason when state != predicted>",
  "supersedes": "<record id>" | null
}
```

Rules:

- **Honesty first, mirroring the neighbourhood.** `insufficient_history`,
  `no_basis`, and `adjustment_unavailable` are published states with
  named reasons — never an invented range, never a silent absence.
  This is S1's discipline (`insufficient-history` passes through
  verbatim) extended to the new kinds; for arrivals, `no_basis` is
  the expected common state (§7).
- **`basis` is mandatory and singular.** One record, one named basis
  (S2.14). A prediction that blends bases is issued as separate
  records, one per basis, so scoring attributes accuracy to the basis
  that earned it. The past-day-vs-30d basis is a comparison basis:
  its record states both rates in `detail`.
- **`rendered` is stored, not recomputed.** Consumers (C6 pages,
  push alerts) display the stored string. What Drew read is what the
  scorer grades.
- **Storage.** Records append to a C4 prediction log beside the
  forecast product (`quota-predictions.jsonl` in the aggregator state
  dir, monthly rotation like the snapshot store) and mirror into the
  Turso ledger under a distinct source (`quota-prediction`), the
  S1 pattern (`FORECAST_LEDGER_SOURCE = "quota-forecast"` stays S1's;
  C4 records never mix into it).

## 2. Kind: pool_exhaustion

The record wraps S1's product; it does not reimplement it (Stage-3
D2: the model is `projectWindow`, unchanged). Per tick, each
pool/window whose S1 row is a live projection also gets a C4 record:

- `basis`: `since_last_reset` (the same-period series S1 assembles is
  the current reset period's readings) — and, once the store supports
  it, a parallel `trailing_30d` record per pool/window so the two
  bases' accuracy can be compared head-to-head.
- `prediction`: the projected wall instant becomes `likely`; the
  range is `likely` widened by the S2 ledger's measured error for
  that pool/window at that lead time (the backtest's signed-error
  quantiles; until a cell has ≥20 scored forecasts, the range is the
  pool's all-window error band; below that, `insufficient_history`).
  The range is therefore *calibrated by the accuracy record itself* —
  that is the loop S2.5 names.
- Scoring is S2's, unchanged: wall error, precision/recall,
  first-call lead. C4 adds range coverage (§3) on top.

## 3. Scoring rules (all kinds)

Every record is scored when its outcome is realized; open predictions
are unscored, never assumed. The accuracy record is the S2 ledger
extended per kind — one report C6/C7 consume.

- **Range coverage (all kinds).** Did the realized instant fall
  inside `[earliest, latest]`? Report coverage per kind/basis/
  methodology version, with mean range width beside it: coverage
  bought with useless width is named as such. Target band: a
  calibrated range covers ~80% of outcomes; persistent over- or
  under-coverage is a methodology-log trigger (§6).
- **pool_exhaustion.** S2's rules verbatim (realized wall = first
  reading ≥100% before reset; detections are not predictions).
- **downtime_window.** Start error and end error (signed minutes),
  plus interval overlap (intersection ÷ union of predicted vs
  realized window). A predicted window that never occurs scores as a
  false window; a realized complete-downtime with no prediction
  scores as a miss. Both count, per S2's precision/recall shape.
- **reset_duration.** Coverage + width on the reopening instant,
  scored per pool. (The duration is start-anchored: the wall instant
  is observed, the reopening is predicted.)
- **model_plan_arrival.** Hit/miss per record plus lead time on hits.
  `no_basis` states are not scored — they are the honest answer, and
  scoring them would pay for guessing.
- **First scored outcomes** for the new kinds publish with C4.1/C4.2's
  first live ticks (acceptance item 2's remainder), in the same
  ledger report S7's weekly review reads.

## 4. Range rendering (S2.13)

`rendered` is produced by one formatter from `(earliest, latest,
likely, issued_at)`, in America/Chicago wall-clock terms:

- **Duration form** (span < 48h): "between X and Y hours from now",
  X/Y rounded to the half hour, e.g. "between 4 and 6.5 hours from
  now".
- **Day form** (span ≥ 48h): "2.5 to 3.7 days"; mixed spans render
  compound: "2 days 4 hours to up to 3 days 16 hours".
- **Calendar form** (when the range crosses a day boundary Drew
  would name): "likely runs out between tomorrow afternoon and
  Sunday 2 am" — day names + dayparts (morning / afternoon /
  evening / night, or a named hour when the bound is sharp). The
  calendar form is preferred for downtime windows and any range
  whose latest bound falls on a different day than its earliest.
- The machine range never leaves the record; rendering is a view,
  and the stored string is the view at issuance.

## 5. Day-type and spike-day adjustments (S2.6, S2.7) — gated

Models, defined now, admitted by data:

- **Day-type cells.** For a pool/window, burn-rate distributions per
  cell: {weekday × daypart} at the coarsest grain the data supports
  (other Fridays; other weekends; same time-of-day on other days).
  An adjustment multiplies the v1 pace by the cell's measured ratio
  vs the pool's overall pace, and the record names the cell in
  `inputs.adjustments`.
- **Spike-day.** When a pool's trailing-7-day burn runs materially
  under its trailing-30-day pace (the S2.7 shape: ~10% light), the
  model predicts a compensating spike day (~+20%) inside the current
  window and says so explicitly: a `spike_day` adjustment entry plus
  the widened range it causes. No silent widening.
- **Sufficiency gate.** An adjustment ships for a cell only when the
  cell holds ≥8 independent samples (Stage-3 measured ~4–5 per
  weekday at 33 days — under gate; the store grows ~15 min/sample,
  so cells cross at different times and are admitted individually).
  Below the gate: `adjustment_unavailable`, unadjusted forecast
  stands. Admission is measured, not scheduled (C4.3 re-measures
  first).
- **Earn-in.** An admitted adjustment must beat the unadjusted
  forecast in the S2 harness over the same history before it becomes
  the published forecast; until then it publishes as a shadow record
  (the S3 pattern) and is scored alongside.

## 6. Methodology-change log (S2.5)

Append-only, one entry per methodology change, stored with the
prediction log and versioned into the repo with the code change that
implements it:

```jsonc
{
  "version": "c4-meth:<n>",            // referenced by every record
  "ts": "<ISO-8601 UTC>",
  "change": "<what changed, in one paragraph>",
  "reason": "<the accuracy evidence that warranted it>",
  "evidence": ["<ledger report / backtest refs>"],
  "expected_effect": "<what should improve, measurably>",
  "review_after": "<ISO date the effect is re-scored>"
}
```

Rules: methodology changes land as PRs (the Stage-3 S6 discipline —
evidence attached, never silent config edits); a record's
`methodology_version` is the version live at its issuance, so a
score is always attributable to the method that produced it; the
weekly review (S7) is the standing reader, and its warranted changes
enter through this log. "Relatively accurate" (S2.5) is operational:
per kind, coverage inside the §3 band and median wall/window error
under one gatherer tick span (15 min) at one-tick lead, sustained
four consecutive weeks in the ledger.

## 7. Kinds: downtime, reset duration, arrivals

- **downtime_window (C4.1, #13257).** From the per-pool exhaustion
  records + reset times: the interval where *every* routing pool's
  predicted wall has passed and the earliest reopening (any pool's
  reset) has not yet arrived. `earliest`/`latest` bound the
  all-walled interval from the member ranges. Sets are the routing
  pools only (Stage-3 §2's coverage), keyed by canonical provider
  id (the S1 keying lesson).
- **reset_duration (C4.2, #13258).** Basis `reset_cadence`: the
  pool's observed reset intervals in the snapshot store (resetsAt
  deltas across periods), giving the reopening range for a wall
  whose reset instant is not yet published by the provider; when
  resetsAt *is* known, the prediction is the wait until it, ranged
  by the cadence's observed jitter.
- **model_plan_arrival (C4.4, #13260).** Watch-based: bases are
  `detector_history` (S5's plan-change findings as the arrival
  record) and `vendor_cadence` where a cadence is actually recorded.
  Absent a basis, the state is `no_basis`. This kind is specified
  honesty-first because its evidence is the weakest; a speculative
  date is a defect, not a forecast.

## 8. Delivery and implementation sequence

- **Push alerts (C4.5, #13261)** ride the alerts ledger via the S5
  `postIncident` precedent, source `quota-prediction`, deduped on
  the prediction record id, thresholded per S3.15's selectivity
  (predicted downtime windows and predicted workhorse full-wall
  alert; ordinary walls do not). Alert text is the record's stored
  `rendered` string.
- **C6 surfaces** (S3.15: predicted workhorse full-wall, predicted
  wall-until) read the prediction log; C4 owns the data, C6 the
  pages — per the program boundaries, this design builds no page.
- **Sequence.** C4.1 downtime windows (#13257) → C4.6 demonstration
  against a known wall (#13262, #9572 acceptance item 3) → C4.2
  reset durations (#13258) → C4.5 push alerts (#13261) → C4.3
  day-type/spike, data-gated (#13259) → C4.4 arrivals (#13260).
  Live predictions for every open pool (acceptance item 2) are
  already served for exhaustion by S1; the item completes as
  C4.1/C4.2 publish their kinds with first scored outcomes.
