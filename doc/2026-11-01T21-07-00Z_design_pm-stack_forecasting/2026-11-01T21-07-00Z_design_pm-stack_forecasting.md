# PM stack forecasting: velocity, quota walls, and stall risk

- Date: 2026-11-01
- Status: proposed design; implementation issues filed as children of [#1256](https://github.com/developing-today/code/issues/1256), itself under [epic #1184](https://github.com/developing-today/code/issues/1184)
- Tracking design: [#1256](https://github.com/developing-today/code/issues/1256)
- Related implementation: quota throughput [#3933](https://github.com/developing-today/code/issues/3933), flow analytics [#1281](https://github.com/developing-today/code/issues/1281), coverage [#1220], blocked-by graph [#2400]

## Summary

Add an explainable forecast layer to the PM-stack aggregator. It composes three kinds of observed history—GitHub issue/epic events, per-pool quota usage, and queue/blocker activity—into estimates, never commitments. Every result carries its observation window, input coverage, model version, and uncertainty. Roadmap and Projects consume the same forecast contract; the first version is deterministic and auditable, not machine-learned.

The design separates three outputs so missing data in one plane cannot make another look more certain:

1. **Epic/project completion range:** remaining direct child issues divided by observed completion throughput, with a range and explicit sample-quality level.
2. **Quota wall range:** an exhaustion interval relative to the next reset, using measured recent burn and the reset boundary. If measured rate or reset/limit is unavailable, return “unknown,” not a date.
3. **Stall risk:** an explainable score and the signals that contributed to it. A score is not a probability until backtesting establishes calibration.

## Current inputs and constraints

The aggregator has useful source and projection seams, but no complete forecasting history contract yet:

- **Quota:** [`model.ts:230–321`](../../pm-stack/aggregator/src/model.ts#L230-L321) defines quota windows, pools, reset times and burn segments; [`model.ts:1273–1349`](../../pm-stack/aggregator/src/model.ts#L1273-L1349) defines gathered per-window usage/freshness and usage snapshots. However, the [aggregator README:503–546](../../pm-stack/aggregator/README.md#L503-L546) says the aggregator does not yet read `quota-current.json`. Session burn attribution exists in [`collect-fleet-feed.ts:595–610`](../../pm-stack/aggregator/scripts/collect-fleet-feed.ts#L595-L610) and [`opencode.ts:276–292`](../../pm-stack/aggregator/src/sources/opencode.ts#L276-L292). [`derive.ts:932–1001`](../../pm-stack/aggregator/src/derive.ts#L932-L1001) already has a linear quota-pace estimate, but not an uncertainty interval. Issue [#3933](https://github.com/developing-today/code/issues/3933) owns the live burn-rate/history gap; populated schema fields alone must not be treated as populated live evidence.
- **Coverage:** [`coverage.ts:29–73`](../../pm-stack/aggregator/src/coverage.ts#L29-L73) and [`coverage.ts:90–190`](../../pm-stack/aggregator/src/coverage.ts#L90-L190) define aggregate repo counts, unavailable/null values, a fixed weekly anchor, and week-over-week deltas. This is a coverage/backlog census, not a closed-item event history. The current Projects and Program endpoints return current/baseline coverage rather than a frozen weekly history ([`server.ts:787–823`](../../pm-stack/aggregator/src/server.ts#L787-L823)). Therefore no delivery rate can be inferred from coverage totals alone. Issue [#1220](https://github.com/developing-today/code/issues/1220) remains the coverage input reference.
- **Dependencies:** [`projects.ts:23–51`](../../pm-stack/aggregator/src/projects.ts#L23-L51) fetches `updatedAt`, parent, `blockedBy`, and `blocking`, but not issue creation/closure times or dependency-edge event times. [`graph.ts:28–107`](../../pm-stack/aggregator/src/graph.ts#L28-L107) computes blocked counts/weights and longest chain, **not blocked age**. Existing orchestrator `since` values cover tier items and stalled goals ([`model.ts:512–527`](../../pm-stack/aggregator/src/model.ts#L512-L527), [`model.ts:614–638`](../../pm-stack/aggregator/src/model.ts#L614-L638)); they are not the age of a GitHub dependency edge. For graph blockers, a future history store can provide only a *first-observed lower bound* unless the source starts publishing edge timestamps. This corrects the packet's claim that `graph.ts` already contains blocked ages.
- **Hierarchy and status:** [`model.ts:64–109`](../../pm-stack/aggregator/src/model.ts#L64-L109) carries GitHub state, Project Status/Area/Horizon/Kind, parent/blocker edges, and `epicRef`. [`projects.ts:170–187`](../../pm-stack/aggregator/src/projects.ts#L170-L187) resolves an epic only when its parent chain is in the fetched pool; [`epics.ts:3–15`](../../pm-stack/aggregator/src/epics.ts#L3-L15) and [`epics.ts:25–53`](../../pm-stack/aggregator/src/epics.ts#L25-L53) roll up child counts/status mix/blocked count. GitHub closure and Project Status remain distinct and disagreements are surfaced ([`projects.ts:273–315`](../../pm-stack/aggregator/src/projects.ts#L273-L315)). Project #4 is a curated subset, not a repo-wide backlog ([`project-config.ts:338–341`](../../pm-stack/aggregator/src/project-config.ts#L338-L341)); every forecast must state its actual pool scope.
- **Adjacent work:** [#1281](https://github.com/developing-today/code/issues/1281) owns general WIP/time-in-status/throughput analytics, while #3933 owns live quota throughput and per-pool near-limit/wall rules. Forecasting should share history and pure derivation seams with those efforts rather than duplicate charts, quota extraction, or threshold rules. The current coverage snapshot is not sufficient by itself to satisfy #1281's implied item-level flow history either; align on one event/history source before adding a second store. The forecast-specific addition over #3933 is a time-to-exhaustion interval using its measured live rate, not another quota alert threshold.

## 1. Velocity and epic/project ETA

### Unit, history, and windows

- **History prerequisite:** introduce one versioned item-history source before claiming an ETA. Capture fully observed weekly scope and state transitions (including zero-completion weeks), closure/reopen time, epic/project membership, effort changes, and availability/freshness. Current `model.ts` has state and hierarchy, but not a closure timestamp; `coverage.ts` is aggregate. Never backfill an exact event time from today's status.
- **Throughput unit (recommended v1):** one direct native child issue closed is one completed unit; a merged PR is evidence/context, not another unit. Reopened children leave the completed count until closed again. Existing [`weights.ts:1–44`](../../pm-stack/aggregator/src/weights.ts#L1-L44) supports effort weights, but `l` is an open-ended floor and many items are unsized. Keep the count-based baseline explicit; show known weighted remaining work separately only if the model can account honestly for unsized/open-ended work. Do not infer size from title or PR count.
- **Windows:** use the trailing 4 complete observed weeks as primary and 8 weeks for stability comparison. A trailing 1-week value may show a recent-change signal, never the sole ETA denominator. A week is complete only if the input source was available for the full window; missing weeks are not zero-completion weeks.
- **Scope:** compute an epic from its direct native children and a project from the explicit fetched/project pool, with descendants deduplicated. Because `epicRef` is pool-bounded, report unresolved/missing parent count and pool scope. Separate GitHub completion from Project Status, and surface disagreements rather than treating one as the other.
- **Estimator:** resample complete weekly completion buckets (including observed zero weeks) to estimate a distribution for remaining units / weekly throughput. Publish P50 and a conservative P80 completion range with model version, sample size, window, and input coverage. If no weekly pace is measured, emit “no measurable pace”; never divide by zero or return an infinite date.

### Small samples and confidence

- Require at least 3 completed units and 4 complete observed weeks for a local numeric forecast. With less evidence, either use the containing project then portfolio-level rate with a visible **low confidence / borrowed baseline** tag, or say “not enough history” and omit dates if no complete comparable scope exists.
- Confidence is an evidence grade, not a probability: **high** requires at least 12 completed units across 8 complete observed weeks, adequate input coverage, and stable 4-/8-week rates; **medium** means at least 3 units across 4 complete weeks with material variation or a borrowed rate; **low** means a sparse/incomplete but usable comparable input; **unavailable** means no defensible rate. The predictive P50/P80 range remains separately visible.
- Display remaining-unit count, completed-unit count, observed windows, and any hierarchy/input exclusions beside the result. Never silently switch the count unit or baseline.

## 2. Per-pool quota-wall prediction

Reuse the existing linear pace derivation as the point-estimate baseline, then add an observed-rate interval once #3933 supplies a real live rate history. For each pool, compare observed remaining quota with measured burn up to the next reset. A wall is forecast only when the rate, remaining budget, and reset time are all known and fresh. Show whether the P50/P80 exhaustion time falls before or after reset; do not imply statistical confidence from the current single linear slope. If rate history is absent, stale, or zero, report “unknown/no recent burn” rather than “safe.”

The quota plane must expose (or keep exposing through #3933): pool/provider identity; current remaining amount and limit; reset instant and source age; timestamped usage/burn observations with their source and coverage; active-session attribution where measured; and explicit missing/estimated flags. The current `quota-current.json` is not read by the aggregator, so an implementation must close this live ingestion gap. A consumer must distinguish no usage from no usage data. Reuse #3933's existing near-limit/wall thresholds; this slice adds a time-to-exhaustion interval, not a second threshold system.

**Scheduled work:** v1's base case uses measured current burn, which already reflects observed active work. It does not pretend that queued issues have known token costs. If a source provides a concrete, attributable active-work schedule, it may be shown as a separately labelled scenario; never add an unmeasured task-size guess to the baseline. Quota wall and project completion are independent forecasts; no conversion of “issues remaining” to tokens is assumed.

## 3. Stall-risk score and surfacing

Use a transparent rule score, initially 0–100, with reason codes rather than an opaque model. A history-capture slice must first persist item state and first-observed blocker edges: current GitHub data has `updatedAt` but not meaningful-activity classification or closure/edge timestamps, and `graph.ts` has no blocked-age field. Until edge history exists, blocked age is a lower bound from first observed blocked state, not the true start time. Proposed initial weights (signals must be available and current; missing signals contribute nothing and reduce evidence coverage):

| Signal | Initial contribution |
|---|---:|
| Blocked edge remains unresolved for at least 7 days since first observed (lower bound, not native edge age) | +30 |
| No meaningful work event for at least 7 days (ignore synchronization-only updates; until event typing exists, show `updatedAt` proxy as such) | +20 |
| Queue claim has missed two expected heartbeat intervals or is approaching lease expiry without progress | +30 |
| At least 2 requeues/retries in the trailing 14 days | +20 |

Cap at 100. Recompute from live inputs, retain the contributing signals and their timestamps, and make the score versioned. For the initial product rule, **surface score ≥60 as “at risk”** on the epic/project detail and Roadmap row with reason codes. Send an item to the existing Needs-you/attention flow only for a concrete blocker or an expired/silent claim that needs an action; score alone is informational and must not create a user escalation. Orchestrator `since` can inform its existing tier/stalled-goal signals, but queue heartbeat/requeue history is not currently a general GitHub item field; mark it unknown until a source is added.

## 4. Presentation and shared contract

Roadmap and Projects render the same machine-readable forecast object. Minimum fields: `subjectRef`; `asOf`; `modelVersion`; `inputWindow`; `inputCoverage`; `remainingUnits`; `p50` and `p80` date/range (nullable); `confidence` (`high|medium|low|unavailable`); `baselineSource`; and structured `reasons`/`limitations`. Quota results additionally identify pool, next reset, remaining amount, and exhaustion interval. Stall results expose score, threshold, and contributing signals.

Render a range, evidence grade, and short input summary together, for example: “Apr 12–29 · low confidence · 5 of 28 completed children, trailing 28 days; using project baseline.” “No measurable pace” and “quota rate unavailable” are valid states. Do not render a single date as a commitment, bury low confidence in a tooltip, or show an exact percentage for an uncalibrated stall score. Keep the API and page wording aligned; freshness and missing-source state must be visible.

## 5. Backtesting and model retirement

Record a weekly as-of forecast snapshot with model version, cutoff time, observed inputs/source watermarks, output ranges, and confidence. At each later snapshot, compare the forecast with the actual issue/epic completion event and quota reset/usage history. Use rolling-origin evaluation: every prediction can see only data available at its original cutoff; no future labels or current-state reconstruction may leak into that prediction.

Evaluate ETA error (median absolute error) and P80 interval coverage by forecast horizon (2, 4, and 8 weeks) and confidence grade. Evaluate quota-wall classification against whether the pool actually hit its limit before reset. Report sample counts and missing/late observations. Keep a simple no-change/trailing-rate baseline. Do not tune on the same periods used to report performance. Do not label a model calibrated until it has at least 30 resolved forecasts over at least 8 weekly cutoffs; for a nominal P80 range, flag calibration for review if observed coverage is outside 70–90% across two successive eligible windows. A persistently worse model is disabled or reverted to “unavailable” until a reviewed replacement passes the same backtest.

## 6. Needs Drew

These are the explicit owner decisions; implementation may proceed with the proposed defaults only after these are answered or the owner accepts the defaults:

1. **What is one unit of work?** Default: one direct child issue, without size weighting. Decide whether the existing effort weights should ever change unit weight; `l` is open-ended and unsized work must remain visible.
2. **What counts as scheduled future quota demand?** Default: measured live burn only, with separately labelled scenarios for concretely scheduled/attributed work. Decide whether a future implementation should treat queued/Now items as committed demand or keep that unknown until token budgets are recorded.
3. **Where should forecast risk interrupt attention?** Default: score ≥60 is visible but informational; only a concrete blocker or dead/expired claim enters Needs-you. Confirm that no risk score alone should escalate to the user.
4. **How much persistence is acceptable for calibration history?** Default: retain the minimal weekly aggregates and source watermarks needed to reproduce evaluations, not raw provider prompts or unrelated session content. Confirm the retention horizon before storing longer-lived history.

## 7. Implementation slices

The implementation slices split along the aggregator's source/model/UI seams:

1. [#3981 — capture item history for forecasting and flow calculations](https://github.com/developing-today/code/issues/3981): durable item history and complete weekly buckets.
2. [#3982 — derive epic ETA bands and explainable stall risk](https://github.com/developing-today/code/issues/3982): pure ETA/stall derivation over that history.
3. [#3983 — forecast per-pool quota exhaustion intervals](https://github.com/developing-today/code/issues/3983): consume #3933's live data.
4. [#3984 — expose and render one forecast contract on Roadmap and Projects](https://github.com/developing-today/code/issues/3984): machine-first API and both views.
5. [#3986 — backtest forecasting models and report calibration](https://github.com/developing-today/code/issues/3986): rolling-origin backtests/calibration.

**Hierarchy constraint:** GitHub rejects new direct children once a parent has 100 sub-issues. A fresh `GET /issues/1184/sub_issues` returned 100, and the attempted direct link for #3981 returned HTTP 422 (“Parent cannot have more than 100 sub-issues”). To preserve native hierarchy without changing existing relationships, these five are children of #1256, which is already a native child of #1184. They are therefore descendants of the epic, not direct children. General throughput charts stay owned by #1281 and should reuse the item-history seam, not create a second collector. The same forecast contract is the integration boundary for both pages. Backtesting is downstream of the models and must not delay honest “unavailable” states in the first UI slice.

## References

- [#1256 — forecasting design](https://github.com/developing-today/code/issues/1256)
- [#1184 — PM stack coherence epic](https://github.com/developing-today/code/issues/1184)
- [#3933 — quota throughput data contract](https://github.com/developing-today/code/issues/3933)
- [#1281 — WIP limits and flow analytics](https://github.com/developing-today/code/issues/1281)
- [#1220 — project coverage snapshots](https://github.com/developing-today/code/issues/1220)
- [#2400 — blocked-by graph inputs](https://github.com/developing-today/code/issues/2400)
