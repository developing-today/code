# Design (quota plane, Stage 3): predictive, self-tuning quota operations

Status: design record for issue #1177, under epic #1149 (quota plane
Stage 2) and the new Stage-3 epic #12694. Docs-only; no implementation
in this record.
Author: fleet agent `felix` (kind muse), 2026-11-22.
Written against `origin/main` at `bff67fda3`.
Hub copy: `files/stage-3-design-2026-11-22.md` in the usage-billing-hub
goal files, linked from that goal's GOAL.md (the home #1177's method
names); this repo record is the reviewed copy Drew's review reads.

Basis, and the inputs this record must answer rather than restate:

- Issue #1177 itself: four components (pace forecasting driving
  placement *before* caution zones; automatic pool re-weighting
  **proposals as repo PRs, never silent config edits**, with forecast
  evidence attached; anomaly detection — burn spikes, adapter schema
  drift, vendor plan changes — wired to the evidence-baseline diff
  protocol; a weekly design review folding forecast accuracy back
  into the rules), one design doc on ≥2 weeks of gatherer history,
  Drew reviews, then granular issues per component under a Stage-3
  epic.
- The Stage-2 design (`quota-plane-design-2026-10-26.md`, usage-
  billing-hub goal files) §6, which fixes Stage 3's scope and method,
  and §1, which fixes the architecture Stage 3 extends: the gatherer
  is a PM-stack service on a 15-minute tick; `quota-current.json` is
  the consumption contract; the snapshot store is the history.
- Standing quota doctrine (Drew, 2026-10-29/30, charter): pools
  should reach ~100% **at** their reset — running out early is fine
  when alternatives exist; a failed or stale quota read **never**
  idles a pool or stops dispatch (unknown is not closed); at a known
  100% the pool is closed until reset or a fresh read shows <100%;
  a 402/quota error is a wall, never a funds fallback. Stage 3's
  forecasts are advisory inputs to placement under this doctrine,
  never a new way for a pool to be closed by stale data.
- The controlled-change gate #1342 (A/B real task batches before any
  pool change lands): re-weighting proposals that become routine must
  pass through it.

## 0. Shape in one paragraph

Stage 3 adds a **forecast product** and three consumers to the plane
Stage 2 already runs. Each gatherer tick, the aggregator promotes its
existing per-window pace math (`quota-analytics.ts`, today used only
to render the /quota Analytics section) into a published forecast —
per pool/window: projected exhaustion instant, burn rate, sample
count, and an honesty state — written beside `quota-current.json` and
into the quota ledger. Placement consumes the forecast the same way
it consumes `quota-current` today: first in **shadow** (the
forecast-informed ordering logged beside the live ordering, no
effect), then live through the **existing** admission verdicts (a
projected wall before reset, beyond a margin, yields `pace_hold` for
new placement — demotion *before* the ≥85% caution zone, never a
hard close on a forecast). A weekly **proposal generator** turns
persistent forecast evidence into route-config **PRs** with the
evidence attached — the only path by which Stage 3 ever changes
configuration. Three **anomaly detectors** (burn spike, adapter
schema drift, vendor plan change) run beside the tick and record
findings as evidence-baseline diffs plus alert-ledger entries. A
**weekly review job** scores the week's forecasts against realized
outcomes and posts the accuracy report to the Stage-3 epic; rule
changes it warrants go through the same PR mechanism. No machine
learning in v1: the forecast is the measured least-squares pace the
stack already computes, hardened into a product with an accuracy
ledger, because the evidence base (§2) supports exactly that and no
more.

## 1. What exists today (measured, not assumed)

Read off the live laptop aggregator and `origin/main` on 2026-11-22:

- **The gatherer is live and current.** `gather-quota.ts` runs on the
  laptop on a 15-minute systemd tick; the heartbeat in
  `~/.local/state/pm-aggregator/quota-current.json` shows tick
  `2026-11-22T22:30:11Z`, `ok: true`, with per-provider results
  (`_antigravity`, `_claude`, `cline-pass`, `_openai`, …). Readings
  come from the single source (`opencode quota --json`, #6127); the
  tick also writes §4 snapshot records and Turso `agent_quota_records`
  rows, fail-soft per sink.
- **Pace projection already exists — as display math.**
  `src/quota-analytics.ts` (Drew's order, 2026-11-13) computes, per
  window, a least-squares burn rate over same-period readings, a
  projected wall instant, and one of four states (`walled`,
  `projected`, `no-wall-before-reset`, `insufficient-history`), with
  honesty rules: an unmeasurable projection is an explicit state with
  a reason, never an invented date. Nothing consumes it for a
  decision; it renders on /quota. Stage 3's forecast is this math,
  promoted — not a second forecasting implementation.
- **Admission already has the verdict Stage 3 needs.**
  `src/quota-admission.ts` (the #2732 normalized projection) returns
  `eligible | unknown | pace_hold | walled | stopped` per billing
  scope, and `quota-admission-shadow.ts` already runs admission in
  shadow. `quota-current.ts` overlays freshness: fresh-ok wins, stale
  or errored entries surface as state, never as current values.
  `quota-exhaustion.ts` handles realized exhaustion. What none of
  them take as input is a *projection*: admission today reacts at
  measured thresholds (caution ≥85%, wall at 100%).
- **The babysitter already consumes a per-pool projection.**
  `pool-state.json` (PR #12689, #1338) projects each tick into the
  claimant state dir — proof the "one more small projection file per
  tick, fail-soft" pattern is the house shape for feeding a new
  consumer.
- **The watchdog pattern exists.** `quota-watchdog-state.json` /
  `quota-watchdog-history.jsonl` in the same state dir are the
  precedent for a checker that runs beside the tick, keeps its own
  state, and records a history — the shape §6's detectors follow.
- **What does not exist:** no forecast is published as data; no
  accuracy record compares a projection to its outcome; no proposal
  mechanism exists (route config changes are hand-made PRs); no
  detector watches burn residuals, adapter parse shapes, or window
  limit changes; no weekly accuracy review exists. The Stage-2
  steward/meta-reviewer daily jobs have been silent since 2026-11-12
  (#1334 reconciliation) — the weekly review in §7 is a new job, not
  a revival of those, and its restore-or-retire question stays with
  #1334.

## 2. The evidence base — the ≥2-week precondition, verified

#1177's method requires the design to be grounded in ≥2 weeks of
gatherer history. Verified 2026-11-22 against the aggregator's
snapshot store (`~/.local/state/pm-aggregator/snapshots/` on the
laptop, the append-only §4 store; records carry `ts`, `serviceId`,
`metricId`, `value`, `limit`, `resetsAt`, `source`, `observedBy`):

- **Span: 2026-10-20T12:00:00Z → 2026-11-22T22:30:49Z — 33 days
  (≈4.7 weeks), 20,635 records** (`usage-2026-10.jsonl`: 5,243;
  `usage-2026-11.jsonl`: 15,392). 20,569 records fall in the gatherer
  era (≥2026-10-27, when scheduled gathering began per #1151). The
  precondition is met with more than double the required span.
- **Pool coverage is the routing set, densely sampled:**
  antigravity across its doors (6,572 + 3,361 + 178 + 142 records
  under its several ids), opencode-go across workspaces (1,649 +
  ~600 more), `anthropic:claude-pro` 675, `meta:high-usage` 297 +
  `meta` 286, `openai:codex` 283, `openai.alternate` 257,
  `cline-pass` 234 + 121, `opendesign` 372, command-code under three
  ids (98 + 87 + 84), `deepseek` 8. Sources: 20,252 `api`, 310
  `console`, 68 `manual`.
- **What this history can support:** per-window pace distributions
  and wall-before-reset frequencies for every routing pool (the
  backtest in slice S2); calibration of the caution margin in §4
  (how far ahead of a realized wall the projection first called it);
  replay material for the §6 detectors (the period contains real
  incidents — walls, the 2026-11-07 Turso sink death recorded in the
  hub GOAL.md, credential expiries).
- **What it cannot support, stated plainly:** (a) per-hour-of-day or
  per-weekday burn profiles per pool — 33 days gives ~4–5 samples of
  each weekday per window, too thin to tune on; v1 therefore uses the
  same-period linear pace only (§8, alternative B). (b) Clean
  per-pool series without a join: serviceIds drifted across the
  period (`antigravity` vs `_antigravity`, `command-code` vs
  `commandcode` vs `_commandcode`, `meta` vs `meta:high-usage`) as
  doors were renamed; any backtest must join across renames with an
  explicit mapping (slice S2 carries it as code, not prose). (c)
  SaaS/hub services (cloudflare 7 records, github 1) are not routing
  pools and are out of Stage 3's scope entirely.

## 3. Component 1 — the forecast product

**Decision D1: forecasts are computed once, in the aggregator, and
published as data; consumers never recompute.** Each gatherer tick,
after `quota-current.json` is written, the tick computes the forecast
from the same readings and writes:

- `quota-forecast.json` beside `quota-current.json`, same directory,
  same fail-soft posture (a forecast failure never fails the tick;
  the previous forecast stands and ages, exactly as a failed tick
  leaves the previous `quota-current.json` standing, #1268). Shape
  per pool/window: the `quota-analytics.ts` projection verbatim —
  `state`, `ratePtsPerHour`, `samples`, `projectedWallAt`,
  `wallsBeforeReset`, plus the reading's `measuredAt` and the
  window's `resetsAt` — under the canonical provider id sessions
  carry (the pool-state.json keying lesson from #1338: key by the
  consumer's id space, not the producer's).
- One ledger row per pool/window per tick into the Turso quota
  ledger, alongside the `agent_quota_records` reading rows, so the
  accuracy scoring in §7 joins forecasts to outcomes in one place.

**Decision D2: the forecast model in v1 is the existing pace math,
unchanged.** `quota-analytics.ts`'s least-squares same-period slope
with its tolerance for reset jitter and its honesty states *is* the
v1 forecaster. Stage 3's work is the product, the consumers, and the
accuracy ledger — the things that make any model, including this
one, safe to act on. A better model is a §7 output (proposed with
accuracy evidence), not a v1 input.

**Freshness contract:** a forecast is consumable only while its
underlying reading is fresh by the `quota-current.ts` overlay rules.
A stale forecast degrades to *no forecast* — consumers fall back to
today's measured-threshold behaviour. This is the stale-read
doctrine applied to predictions: unknown is not closed, and a
prediction is never stronger than the reading behind it.

## 4. Component 2 — forecast-driven placement

**Decision D3: shadow first, for at least one week, on the S2
score.** The forecast-informed ordering — burn-by-expiry ordering
adjusted so a pool projected to wall before its reset is demoted in
the ordering *before* it reaches the caution zone — is computed at
the same points the live ordering is computed (admission/claimant
path) and logged beside it as a per-decision diff record. The live
ordering is untouched. The week's diffs are scored against realized
outcomes via the accuracy ledger: did the pools the forecast demoted
actually wall, and did the pools it promoted hold? Slice S3's
acceptance is that summary, posted on the epic — the evidence on
which D4's go-live is decided.

**Decision D4: live effect enters through the existing admission
verdict, one pool, behind a flag.** When the shadow record warrants
it, admission gains one input: a window whose forecast says
`projected` with `wallsBeforeReset` and a projected wall at least
the configured margin before the reset yields `pace_hold` for new
placement on that pool. Everything about the rollout copies the
stack's existing postures: one named pool first; a backend-style
flag whose default is today's behaviour and whose flip back is the
whole rollback (the `PM_AGG_STORE_BACKEND` pattern); `pace_hold` is
a hold on *new* placement, never a stop of running work and never a
pool closure — at a *measured* 100% the existing wall rules apply,
unchanged. Margin and pool are configuration, set from the S2
calibration, and changed only via §5's proposal mechanism once live.

## 5. Component 3 — re-weighting proposals as PRs

**Decision D5: the only configuration Stage 3 ever changes, it
changes by pull request.** A weekly generator (the job in §7 runs
it, or it runs standalone) examines the week's forecast + accuracy
record and, when the evidence supports a durable change — a pool
weight, an ordering rule, a caution margin — opens a PR against the
route config carrying the full evidence pack: the forecasts behind
the proposal, their accuracy record (S2), the shadow diff summary
when the change class was shadow-tested (S3), and the expected
effect stated as a measurable prediction the next week's review can
score. No proposal, no change: the generator has no write path to
configuration except the PR, and no merge rights; merge authority
stays where it is (Felix/Drew). This is #1177's hard rule ("never
silent config edits") made structural rather than procedural.

**Gate:** a proposal *class* (e.g. margin changes) becomes routine
only after passing #1342's controlled A/B for that class; before
that, each proposal is individual and Drew-visible in the weekly
report. The generator's first PR is generated, not hand-written —
that is slice S6's acceptance, because a generator that has never
generated is a design, not a mechanism.

## 6. Component 4 — anomaly detection

**Decision D6: three detectors, one home, no new plumbing.** The
detectors run aggregator-side beside the tick, in the quota-watchdog
shape (own state file, own history JSONL, fail-soft), and every
finding lands in the two places findings already go: an
evidence-baseline diff record (the hub's per-service baseline
research record — the finding states what changed against the
baseline, in the baseline's terms) and the alerts ledger (the C3
record of detail, per the #12599 ruling). Detectors:

- **Burn spike:** realized burn over the last hour exceeds the
  forecast band (projected rate + the S2-measured residual spread
  for that pool) — a spike is defined against the pool's own
  forecast, not a global threshold, so a pool that always burns hot
  does not cry wolf.
- **Adapter schema drift:** a provider response whose parsed shape
  changes — fields appearing or disappearing against the adapter's
  recorded shape, or a parse falling into the framework's fail-closed
  `unparseable` state. The gatherer's adapters already fail closed
  (previous value ages, never zeros); the detector's job is to make
  the *drift itself* a finding at first occurrence, not after the
  aged value goes stale.
- **Vendor plan change:** a window's `limit` (the snapshot records'
  denominator) changes between consecutive records for the same
  service+metric, or a window kind appears/disappears for a pool.
  Plan changes are the silent killers of pace math — a changed
  denominator breaks pace continuity (the admission module already
  treats a changed denominator as breaking the series); the detector
  surfaces the break the tick it happens.

**Acceptance shape (slice S5):** each detector must fire on a
replayed historical incident from the snapshot store and stay silent
across a replayed clean week — detection quality is demonstrated
against the §2 history, not asserted.

## 7. Component 5 — the weekly review

**Decision D7: the review is a scheduled job that produces a
report and proposals, never edits.** Weekly (registered in
`doc/fleet-schedule-registry.md` in the same pass as its creation,
per the schedule rule), the job: (1) scores the week's forecasts
against realized outcomes from the accuracy ledger — per pool:
projected-vs-realized wall error, wall-before-reset precision and
recall, and the week's `pace_hold` outcomes once S4 is live;
(2) appends the §6 findings summary; (3) posts the report as a
comment on the Stage-3 epic #12694; (4) folds warranted rule changes
into §5 proposals. The loop's product is calibration: margins,
bands, and eventually the forecast model itself are tuned from
measured accuracy, in the open, one PR at a time. The first two
reports must each record at least one rule consequence — a proposal,
or an explicit no-change finding with the numbers — so the loop
cannot decay into a report nobody acts on.

## 8. Alternatives considered

- **A. Forecast inside the fork's quota subcommand.** Rejected: the
  subcommand is the single *read* source (#6125/#6127, and #2490
  keeps the read path unified); forecasting is a PM-stack product
  over history the fork does not hold (the snapshot store and ledger
  live aggregator-side). Putting the model in the reader would fork
  the math away from its evidence.
- **B. Richer models in v1 (hour-of-day profiles, exponential
  smoothing, per-pool learned bands).** Deferred, not rejected: §2
  shows the history is 33 days — enough for the linear pace and its
  calibration, too thin for profile models. The accuracy ledger
  (S2/S7) is precisely the instrument that will show when a richer
  model beats the linear one; §7's loop is where that proposal
  belongs.
- **C. Consumers compute their own forecasts from quota-current.**
  Rejected: divergent math per consumer, no shared forecast to
  score, and accuracy — the thing Stage 3 exists to close the loop
  on — becomes unmeasurable. D1's publish-once product is what makes
  S2 possible.
- **D. Direct config edits by the proposal generator, with a log.**
  Rejected outright: #1177's hard rule. A log of silent edits is
  still silent edits; the PR *is* the review surface.
- **E. Revive the silent steward/meta-reviewer jobs as the weekly
  review.** Rejected: their restore-or-retire is Drew's separate
  decision (#1334), their scope is daily model/route audit (Stage
  2's §3, tracked by #1173), and the weekly review's subject —
  forecast accuracy — did not exist when they were written.

## 9. Slice register (filed with this design)

Epic **#12694** — epic(quota-plane): Stage 3 — predictive,
self-tuning quota operations. Children in priority order; each is
independently landable, and each names its acceptance evidence in
its own body:

| # | Slice | Issue | Lands |
|---|-------|-------|-------|
| S1 | Forecast product: per-tick `quota-forecast.json` + ledger rows (D1, D2) | #12695 | P0 |
| S2 | Accuracy ledger + backtest harness over the snapshot store, rename-join as code (§2) | #12696 | P0 |
| S3 | Shadow placement: forecast ordering logged beside live ordering (D3) | #12697 | P1 |
| S4 | Live forecast-driven admission: `pace_hold` from projection, one pool, flagged (D4) | #12698 | P1 |
| S5 | Anomaly detectors: burn spike / schema drift / plan change (D6) | #12699 | P1 |
| S6 | Re-weighting proposal generator: evidence-packed PRs, PR-only write path (D5) | #12700 | P2 |
| S7 | Weekly accuracy review job + schedule-registry entry (D7) | #12701 | P2 |

Dependency shape: S2 needs S1's forecast shape (its backtest can
proceed against the analytics math directly, so the two can run in
either order); S3 needs S1; S4 needs S3's shadow record; S6 needs
S1+S2; S7 needs S2. S5 is independent of the forecast chain except
for the burn-spike band, which can start from the analytics math and
tighten when S2 lands. Gate for all: Drew's review of this design
(per #1177's method) — the issues are filed now so the work is
tracked and claimable the moment the review lands.

## 10. What this record does not do

- It does not change any live behaviour: no consumer reads a
  forecast today, no config is edited, no job is created. S7's job
  and S6's generator are slices, not faits accomplis.
- It does not decide the steward/meta-reviewer restore-or-retire
  question (#1334), the #1173 daily-audit catalog block, or #1159's
  review close — all named here only to keep them out of Stage 3's
  critical path.
- It does not re-open the single-source read path (#2490) or the
  admission module's scope discipline (#2732): Stage 3 consumes
  both as they stand.
- Forecast accuracy targets are deliberately not fixed in this
  record: S2's backtest produces the baselines, and the weekly
  review (§7) is where a target, if one is wanted, gets set from
  measurement rather than from this document's optimism.
