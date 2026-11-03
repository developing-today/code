# Spend rollups: token and dollar attribution per project and per epic

- Date: 2026-11-03
- Status: proposed design for [#1259](https://github.com/developing-today/code/issues/1259), epic [#1184](https://github.com/developing-today/code/issues/1184) (PM stack coherence series)
- Tracking design: [#1259](https://github.com/developing-today/code/issues/1259)
- Related: forecasting [#1256](https://github.com/developing-today/code/issues/1256) (forecast-to-complete is consumed from that design, not rebuilt here), observability/attribution infrastructure [#2072](https://github.com/developing-today/code/issues/2072), activity river [#1261](https://github.com/developing-today/code/issues/1261), durability [#1258](https://github.com/developing-today/code/issues/1258)

Implementation follow-ups: [#4822](https://github.com/developing-today/code/issues/4822) attribution projection; [#4823](https://github.com/developing-today/code/issues/4823) rollup store; [#4824](https://github.com/developing-today/code/issues/4824) Projects/Roadmap rendering; [#4825](https://github.com/developing-today/code/issues/4825) cost anomalies; [#4826](https://github.com/developing-today/code/issues/4826) PAYG dollar attribution. They are filed as sub-issues of #1259 — itself a sub-issue of epic #1184 — because #1184 is already at GitHub's 100 sub-issue cap (verified 2026-11-03: direct linking returns HTTP 422 "Parent cannot have more than 100 sub-issues"). This follows the forecasting precedent (#1256's implementation issues are its own children, under the same epic) and keeps the slices inside the epic's tree; if Drew wants them as direct children of #1184, some existing #1184 sub-issues must be re-homed first — that is a Drew call, not made here.

No implementation is included in this note. It answers the five design questions in #1259 against the current code, and names every open question that needs Drew explicitly in the final section.

## Current inputs and constraints

The stack already collects everything this design needs; the gap is attribution and rollup, not collection. #1259's own framing binds: consume quota-plane data, do not rebuild collectors.

- **Per-session usage already exists.** `pm-stack/opencode-usage-export/opencode-usage-export.mjs` (slice 1 of #2072) exports per-session records from OpenCode's live store: session `id`, `parentId`, `agent`, `directory`, `model {id, providerID}`, per-session `tokens {input, output, reasoning, cacheRead, cacheWrite}`, a `cost` field, and `timeCreated`/`timeUpdated`, plus `byModel` breakdowns and totals. It reads the v2 tables only and fails closed (a stale store writes a failure document, never zeros) — spend attribution inherits that fail-closed property.
- **Pool quota already exists, in unlike units.** `pm-stack/aggregator/src/model.ts:229-261` defines `QuotaWindow {name, unit: "requests" | "tokens" | "usd" | "messages" | "other", limit, used, resetsAt, percent}` and pools that carry those windows. The exporters speak the same shape: `pm-stack/meta-exporter/meta-quota-export.mjs` emits windows with `usedPct` and `resetAt`, plus `freeCreditsUsd` and `paygBalanceUsd`. Percent-of-window is a real, first-class measure in the current system — it is not a token count and not a dollar figure.
- **Usage history already exists.** `pm-stack/aggregator/src/snapshots.ts:1-18` is an append-only usage snapshot store: one JSONL file per month (`usage-YYYY-MM.jsonl`) plus compacted rollup files that keep min/max/last per series. The exporters likewise append every tick to `history.jsonl`. Rollups in this design extend that store; they do not start a second history.
- **Real dollars already exist — at service level.** The charges pipeline (`pm-stack/aggregator/src/charges-collect.ts`, `charges-match.ts`, `charges-catalog.ts`; #1057) matches Plaid + Gmail charge candidates to services/pools and surfaces unmatched charges in the hub attention strip. That is the only place actual money enters the system. Per-session `cost` from OpenCode is a list-price estimate computed by the tool, not a charge Drew paid — the two must never be conflated (see Units).
- **Hierarchy already exists.** `pm-stack/aggregator/src/model.ts` (`ProjectPoolItem`, ~lines 64-109) carries `parentRef`, `blockedBy`/`blocks`, `area` (Project Area), and `epicRef` — the nearest sub-issue ancestor whose Project Kind is "Epic", resolved within the fetched pool. `pm-stack/aggregator/src/projects.ts` resolves parents/blockers; `pm-stack/aggregator/src/epics.ts:3-53` already rolls child counts, status mix, and blocked counts up per epic. Spend rollups reuse these seams; progress (% complete) comes from the same epic rollup so spend and progress can never disagree about scope.
- **Session-to-work binding already exists for queue work.** The repo-queue lane ledger binds a launched OpenCode session to its queue task at launch time (`repo-queue.py bind`; the claimant in `pm-stack/opencode-claimant/poll.py` polls for claimed, opencode-assigned, unbound tasks), and every queue task names its GitHub issue. Fleet worktrees are created per task under `~/git/fleet/<task-id>-<issue-number>-<slug>`, so a session's `directory` also encodes its task and issue.
- **Live per-pool burn attribution already exists.** `pm-stack/aggregator/src/sources/opencode.ts:320+` maps a session list to per-provider burn (`sessionBurnByProvider`), and the forecasting note (#1256) records session burn attribution in `collect-fleet-feed.ts` and the same source module. This design's attribution is the per-issue/epic sibling of that per-pool attribution, built on the same session records.
- **Anomaly surfacing already has a throat.** `pm-stack/aggregator/src/alerts.ts` (#3676) is the single alert intake: dedupe, delivery, escalation, and ack state in one ledger; warnings are page + ledger only in v1, never push. Needs-you is rendered from the tier source (`pm-stack/aggregator/src/render.ts:3548+`, `/attention`), and the brief/Today surfaces consume the same summary. Cost anomalies join that throat; they do not get a new notification path.

## 1. Attribution model

Attribution is a **versioned, pure projection** over immutable spend records — never a property baked into the records themselves (see Windows and history for why this matters when rules change).

**Spend record (input, already collected):** one root session's usage in a window: session id, root session id (via `parentId` chains), provider/pool, model, token counts, the exporter's `cost` estimate (kept, labelled, and quarantined — see Units), timestamps, and the session's `directory`.

**Resolution order** — first match wins, and the matching rule is recorded on every projected fact:

1. **Queue binding (exact).** The session is bound in the lane ledger to a queue task → the task's issue. This covers the fleet's queue-worked sessions, which are the majority of agent spend.
2. **Fleet worktree (exact).** The session's `directory` is a `~/git/fleet/<task>-<issue>-…` worktree → that task and issue.
3. **Epic/project resolution (derived).** From the issue, follow the existing hierarchy: `epicRef` (nearest Epic ancestor within the pool) for the epic; the Project pool item for the project. Project **Area is a display grouping, not an attribution key** — an item's Area never decides where its spend goes.
4. **Unattributed (explicit).** Anything that resolves no issue — interactive sessions, sessions in non-fleet directories, bindings whose parent chain leaves the fetched pool, source gaps — lands in an **unattributed bucket with a reason code**: `no-binding`, `no-worktree`, `parent-left-pool`, `source-gap`. Unattributed spend is a first-class row in every rollup and view, with its share of the total shown. It is never dropped, never spread pro-rata across epics, and never guessed onto the most recently active epic: a visible "we don't know" is the design's core honesty property, the same fail-closed stance the exporters already take (a failed tick is never a zero reading).

Sub-sessions roll up to their root session before attribution, so a root session and its children are never double-counted. A session that spans an epic boundary (worked two issues) attributes to its binding; if it genuinely served two tasks it was two root sessions in practice — the projection does not split a session across issues, because a split would be an invented fraction. The reconciliation invariant is testable and absolute: **sum(attributed) + unattributed = source totals, per unit, per window.**

## 2. Units: unlike measures stay unlike

Three unit families exist in the current system, and the rollup schema stores and renders them as three distinct measures forever:

| Measure | Source | What it means | Never |
|---|---|---|---|
| **Tokens per pool** | usage export `tokens`, gathered per provider/pool | Actual consumption, per pool (pools meter differently; a token total across pools is shown only as per-pool columns, not a blended rate) | converted to dollars |
| **PAYG dollars** | charges pipeline (#1057) matched charges only | Money Drew actually paid, per service/pool and period | merged with estimates; force-attributed when unmatched |
| **Plan-pool share-of-window** | quota windows' `percent` / `usedPct` against `resetsAt` | How much of a fixed plan window a scope consumed — the only honest measure for plan pools, whose price is flat | priced out at API list rates |

Two traps are named and banned in the schema, not just in prose:

- **OpenCode's per-session `cost`** is a list-price estimate. It may appear in session drill-down labelled exactly "list-price estimate"; it is excluded from the dollars column, and no rollup sums it with matched charges.
- **No exchange rate between families.** There is no tokens→dollars or share→dollars conversion anywhere in the design, including "for display only". A scope's spend is a three-column fact, not a number. Any future request for a single figure is a new design decision for Drew (see open questions), not an implementation detail.

## 3. Windows and history

- **Windows:** calendar week and calendar month rollups per project and per epic, built from the append-only snapshot history (`snapshots.ts`) and the exporters' `history.jsonl`. A window counts only if its sources were available for the whole window; a partial window is labelled **partial** wherever it renders, and is excluded from anomaly comparisons (an incomplete week must never read as a cheap week or a zero-closure week).
- **How far back:** from the start of retained snapshot history — and every rollup record carries its `coverageStart`, so the UI shows the actual horizon instead of implying one. Backfill before history exists is impossible and is not faked: pre-history spend is simply absent, and the coverage start says so.
- **Attribution rules change retroactively — by restatement, not rewriting.** Because attribution is a projection (§1), a rule change produces a new `attributionVersion`; rollups are recomputed from the immutable records and stored **alongside** the old version, labelled. Readers default to the newest version; the version is printed on the view. Raw records and old rollups are never edited in place — the same append-only discipline as the snapshot store and the activity river (#1261). A restated history is therefore always explainable: "these numbers changed because attribution v3 counts X differently", with v2 still inspectable.

## 4. Placement: spend beside progress

On **Projects and Roadmap**, the epic/project card gains a spend block rendered from the same rollup the epic counts come from (`epics.ts`), so scope can never drift between the two halves of the card:

- **% complete** (existing child-count rollup) beside **spend-to-date** for the current window and the epic's life-to-date, in the three unit columns of §2.
- **Forecast-to-complete** is consumed from the forecasting design (#1256) — its completion range and quota-wall outputs — and rendered only when that contract supplies it, with its confidence label. Absent forecast renders as "unknown", never as a number this design invents. This note deliberately does not define a second forecaster.
- **Unattributed** renders as its own row on both views at stack level, with the reason-code breakdown one click away; an epic card never silently absorbs it.
- Drill-down from any figure reaches the contributing sessions (id, model, tokens, window) — every rollup number is auditable to its records, matching the river's evidence-linked stance (#1261).

## 5. Cost-anomaly surfacing

Anomalies are computed from complete windows only, and each surfaced anomaly names its evidence (scope, window, both compared values, drill-down link). Four v1 classes:

1. **Burn without progress** — a scope consumes spend in a window while closing zero child issues in it.
2. **Single-session dominance** — one root session accounts for more than the threshold share of a scope's window spend (the "one runaway session" case).
3. **Unattributed growth** — the unattributed share of total spend rises window-over-window beyond the threshold (attribution is rotting; this is a data-quality alarm, not a spending alarm).
4. **Charge without activity** — a matched PAYG charge with no session/quota activity for that service in its period (paying for something unused, or a collector gap — either way Drew should see it).

Surfacing goes through the existing alert throat (`alerts.ts`, #3676): anomalies open deduped incidents (a continuing anomaly updates its incident, it does not re-post every tick), land in **Needs-you** (`/attention`) and the **Today**/brief summary, and are **warning class — page + ledger, no push** — per the alerts design's v1 rule. Threshold values are named constants in one place; the initial values are proposals, and setting them is one of Drew's questions below.

## Decisions needed from Drew

These are named explicitly, per #1259's acceptance. Defaults in the implementation issues assume the recommendations below; a different ruling changes the named slice, not the architecture.

1. **A single blended cost figure — ever?** Recommendation: **no.** Tokens per pool, PAYG dollars, and share-of-window stay three columns permanently (§2); any blended figure would be a fake conversion. If Drew ever wants one, it needs its own design and an explicit conversion policy. (Affects: slices 2, 3.)
2. **Interactive / non-queue session attribution.** Recommendation: leave them **unattributed by default** (reason `no-binding`/`no-worktree`) rather than inferring from directory or repo; inference can be added later as a new attribution version with restated history. Drew could instead rule that sessions in a repo primary clone attribute to that repo's project. (Affects: slice 1.)
3. **Anomaly thresholds and class.** Recommendation: single-session dominance at **50%** of a scope's window spend, unattributed growth at **+10 points** window-over-window, burn-without-progress on any complete window with spend and zero closures; all warning class, no push. (Affects: slice 4.)
4. **Per-session spend retention.** Recommendation: keep per-session records for as long as the snapshot store keeps usage history, aligned with whatever #1258 (durability) and #1261 (activity river) settle — no separate, shorter spend retention. If Drew wants a shorter horizon for session-level detail, rollups outlive it and the coverage start must say where detail ends. (Affects: slice 2.)
5. **List-price estimate visibility.** Recommendation: OpenCode's per-session `cost` shows **only** in session drill-down, labelled "list-price estimate", never on cards and never totalled. The alternative is suppressing it everywhere; showing it unlabelled is not an option. (Affects: slice 3.)

## Implementation follow-ups

- Slice 1 — [#4822](https://github.com/developing-today/code/issues/4822): attribution projection (session → issue/epic, unattributed with reason codes, reconciliation invariant).
- Slice 2 — [#4823](https://github.com/developing-today/code/issues/4823): weekly/monthly rollup store, unit-separated, versioned restatement, coverage start.
- Slice 3 — [#4824](https://github.com/developing-today/code/issues/4824): Projects/Roadmap cards — % complete beside three-column spend-to-date, forecast consumed from #1256, unattributed row, labelled list-price estimates in drill-down only.
- Slice 4 — [#4825](https://github.com/developing-today/code/issues/4825): cost-anomaly detection through the alert throat into Needs-you/Today, warning class, complete windows only.
- Slice 5 — [#4826](https://github.com/developing-today/code/issues/4826): PAYG dollar attribution via the charges pipeline; matched charges only; estimates excluded from dollars.

Each is filed with its own acceptance criteria and named checks (`bun test` + `bunx tsc --noEmit` in `pm-stack/aggregator`, run in CI). Sequencing: slice 1 → slice 2 → slices 3 and 4; slice 5 is independent of 3/4 once slice 2 exists.
