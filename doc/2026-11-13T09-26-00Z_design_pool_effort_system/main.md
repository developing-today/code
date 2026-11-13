# Pool & effort system — program design (code#9399)

**Owner:** summit design-mill session for queue item 15864 (relaunch of
2026-11-13). **Sources:** `brief.md` (Drew 03:26 CT), `brief-decisions-supplement.md`
(Drew 05:42 CT), the launch packet's standing rules. **Status:** living
document; versions layer, they never replace (design-mill invariant 4).

This file is the program map. It holds the architecture decisions every child
design is bound by, the version ladder, and the component boundaries. Detail
lives in the child designs under `designs/`, the evidence in `baseline/` (V1
audits) and `research/` (state of the art), and the statement-to-issue map in
`inventory/`.

## 0. Version ladder (used by every child design)

| Version | Meaning | Gate to enter |
| --- | --- | --- |
| **V1** | What exists on main and on the machine today (2026-11-13). Documented, not redesigned. | — |
| **V2** | Config-level and small-code changes that make the brief's asks expressible and honest: tiers, overlays, aliases, decision pool, executor, records. Hand-tuned values. | V1 audit merged. |
| **V3** | Measured and automatic: values derived from the stats store and trials; guards that fail on drift; outcome capture feeding scores. | V2 records exist long enough to be measured (≥ 14 days of events per decision being tuned). |
| **V4** | Learned and adaptive: bandit allocation inside a safe set, self-switching informed by calibrated decisions, fixpoint pool composition, hosted editions. | V3 metrics show the hand-tuned baseline is the limiting factor. |

## 1. Facts from V1 that bind the design

Each is cited in the baseline audits; they are listed here because getting
any of them wrong would invalidate a design.

1. **Effort is a per-leaf pin today, not a tier.** `#variant` pins a leaf;
   a pin anywhere in a route's expansion makes the whole route advertise no
   variants (`model-routes.ts:1269`). No pool exposes low/medium/high.
   (`baseline/v1-pools-efforts.md` §b)
2. **Silent remaps corrupt measurement.** DeepSeek `medium→high` on Go and
   ClinePass, ClinePass `low→high` on Pro; an undeclared remap is dropped
   silently (`model-routes.ts:1017-1021`). Any trial that reads `variant`
   from `route_attempt` without the remap table is wrong.
3. **The schema cannot express "strictest in/out/cache/total".** Only a
   per-target context cap exists (used by `limited` alone); no input or cache
   cap, no route-level limit.
4. **Most doors are failing.** 36 of 50 leaves at ≥ 90 % failure over 72 h on
   2026-11-13 (quota walls and dead doors are indistinguishable in
   `route_usage`; `route_attempt.error_*` can tell them apart). Every
   provider-specific choice in this design is therefore **provisional data**,
   carried in overlays and trials, never baked into the mechanism.
5. **Command Code is dead and was never a Jev door in code.** Jev is served
   by TypeSafe, OpenCode Zen (`jev-1.13-free` promo — live-probed 200 on
   2026-11-13), OpenRouter, and Vercel. Clef (Cloudflare Workers AI behind the
   fork façade) was ruled won't-do in opencode#1651. (Q5)
6. **No decision machinery exists.** No decision pools, default, retries, IDs,
   LLM decider, or durable record; OpenChamber keeps the latest decision per
   session in memory. Auto is configured (5 categories → pools, workhorse
   fallback) but not end-to-end verified, and not the default model.
7. **The event ledger already exists.** `opencode.db` holds `route_attempt`
   (649 k rows since 2026-10-24: provider, model, variant, outcome, error
   tag/code/status, TTFT, response ms, tokens incl. reasoning/cache, quota
   reset, failover, credential, continuation) and `route_decision` (2.43 M
   rows: candidates, chosen, reason). Only 50 k attempts carry `cost_usd`.
   The stats store **extends** this ledger; it does not replace it.
8. **The Cloudflare allowance is 10,000 neurons/day**, not a $10 credit, and
   paid plans have no hard stop. (Q3)

## 2. Architecture decisions (binding on child designs)

**AD1 — One canonical effort tier, resolved late, per leaf.** Callers ask for
`(route, tier)` with `tier ∈ {low, medium, high}`; `xhigh`, `max`, `ultra`,
`fast` are aliases of `high` until a real rung is implemented (brief §4).
The tier is resolved to a **native variant string per (door, model)** at
expansion time from a single rung table. A tier a leaf cannot honour is an
expansion-time **error recorded as `unsupported-tier`** (the leaf is skipped,
counted, never silently remapped). Remaps become explicit rung-table rows
(`deepseek-flash@go: medium → high, declared=true, reason=...`) so the record
always says what actually ran.

**AD2 — Medium is the pool as written; low and high are overlays.** Every
pool's current members are its `medium` tier (brief §4, "align current seats
as medium"). `low` and `high` are **keyed overlay sets** on top of medium,
with operations `set`, `add`, `remove`, `replace`, and `replace-effort`,
keyed by member identity (`ref` + optional door), never list index (R1:
Kustomize keyed merge / NixOS priorities). Conflicting equal-priority
operations are a config-check error. An empty or absent overlay means "same
as medium" (brief §4 "or don't overlay it"). A tier may instead declare
`members:` outright — a separate list — when scaling differs entirely.

**AD3 — Cross-pool equivalence is data, not code.** "Summit low ≈ workhorse
high" is expressed as an overlay that references another pool's tier
(`summit#low: { from: workhorse#high, … }`). Every equivalence row carries
provenance `measured | extrapolated-from-AA | aliased | ruled` and the quality
band it claims. The fork's existing composition-cycle guard covers tier
references (a tier reference is a composition edge).

**AD4 — Limits are per tier and strictest-wins.** A tier carries an optional
`limits: {input, output, cache, total}`; the effective limit of a leaf is the
minimum of the native model limit, the door's policy cap, the leaf's cap, and
the tier's cap. This needs a schema extension in the fork (input and cache
caps do not exist today) — filed as a fork slice, with V2 expressing only what
the current schema can (`context` caps per target).

**AD5 — Decisions are a route kind on the same machinery.** A decision pool
is a route whose leaves are **decision legs** of two kinds: `native` (a
systemone endpoint) and `llm` (a chat model driven by the versioned decider
prompt and validated against the request's schema). The fork's façade keeps
`/v1/systemone` byte-compatible with the standard; everything this program
adds rides in `x-`-prefixed envelope fields that the façade strips before a
native call. One config key names the default decision pool (`decision`).
Leg order is **free → plan → PAYG** with a burn-first budget leg for any
included allowance (Cloudflare neurons), and **Luna @100k #medium pinned
directly as the last resort** (never `small`). If every leg fails, the
request's (or config's) **default answer** is returned and recorded as such.

**AD6 — Traversal is visited-set, uniformly.** Every expansion — chat pools,
tier overlays, decision pools — carries one per-call visited set. A pool
already on it counts as fully processed (all weighted, round-robin, and
ordered options checked) and is skipped with outcome `cycle-skipped`, distinct
from `failed`. A 100 %-walled leg is never attempted (standing rule): it is
`walled-skipped`. Fixpoint-style mutual references are V4 research.

**AD7 — Every decision and every attempt is an addressable event.** Decision
IDs, prompt version + date, tokens, quota, time-to-decision, the invalid-JSON
path, and the answering leg are written to a durable `decision_record` beside
`route_attempt`/`route_decision` in `opencode.db` (V2), exported to the stats
store (V3). Feedback (caller/callee 0–5 correct and 0–5 wrong gauges) and
outcomes are **append-only** rows referencing the ID; nothing rewrites a
record (R3: Langfuse scores-after-the-fact).

**AD8 — The stats store extends the ledger, keeps full granularity, measures
growth.** V2: nightly export of `route_attempt` / `route_decision` /
`decision_record` / session facts to Parquet (partitioned by day), queried by
DuckDB, with a Type-2 `dim_model` (lineage, AA version and test date, variant
splits such as DeepSeek 1107 vs classic and Luna 6 vs 6.1) and a
`change_marker` table carrying expected vs actual deltas. Rollups use
mergeable states only (count/sum/min/max/Welford; log-histogram sketches for
quantiles, α = 1 %, verified by `research/prototypes/stats_rollup_oracle.py`);
every quantile is reported with its `n`. V3 adds R2 storage and the
Roadmap & Stats Space as a consumer.

**AD9 — Trials are pre-registered, non-inferiority, and never burn walled
doors.** Every model/effort swap is a named trial (the Luna round-robin
pattern, PR #9018) with a hypothesis, a margin, a sample-size target from R1
§4.4, a review date, and a revert condition. Bandit allocation (V4) only ever
draws inside the trial's safe set.

**AD10 — Self-switching is bounded and recorded.** A session may suggest or
switch its own model only to an `opencode-route/*` pool or an OpenChamber
category pool, through one tool whose calls are recorded as decisions (AD7).
A session cannot switch its own model in place today (verified); V2 delivers
switching as "launch the subsession on the chosen pool" plus guidance, V3
adds an in-place switch only where the fork supports it without breaking
prompt cache.

## 3. Component designs (children of this program)

| ID | Design | Themes (inventory) | File |
| --- | --- | --- | --- |
| D1 | Effort tiers, overlays, aliases, equivalence, per-tier limits | T3, T4, T7 | `designs/D1-effort-tiers.md` |
| D2 | OpenChamber categories, Auto, self-switching and sub-session guidance | T1, T2 | `designs/D2-categories-auto-switching.md` |
| D3 | Granular stats store and data basis | T5, T6 | `designs/D3-stats-store.md` |
| D4 | Trials, dominated-model replacement, variant splits, provider coverage | T7, T8 | `designs/D4-trials-coverage.md` |
| D5 | Decision system: endpoint, decision pools, executor, LLM-decider protocol, effort lean, conditional clauses, cycle-safe traversal | T9–T12, T14, T15, T17 | `designs/D5-decision-system.md` |
| D6 | Decision record, outcome capture, feedback gauges, PM review surface | T11, T13, T16 | `designs/D6-outcomes-review.md` |

Dependencies: D1 ← D3 (values) ← D4 (trials); D5 ← D1 (effort lean uses tier
classes), D5 → D6 (records), D6 → D3 (export). D2 consumes D1 (category
effort), D5 (Auto's classifier is a decision), and D6 (decisions recorded).

## 4. Standing rules carried into every slice

GitHub via `gh` only; merge commits only, never force-push main; CI runs on
main only; secrets by name only; a 100 %-walled pool leg is never attempted;
parked/abandoned design states are Drew's alone to set; code#9399 is an epic
under the EPIC GUARD.
