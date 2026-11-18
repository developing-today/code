# Time-of-day routing policy and config representation (#9378 stage 1)

Status: specification; docs only. This packet changes no config and no code.
Date: 2026-11-18.
Issue: [developing-today/code#9378](https://github.com/developing-today/code/issues/9378).
Dedupe key: `repo-health:v1:fix:developing-today/code:issue:9378`.
Queue task: 20400.

## Scope

Routing makes one static chain around the clock. #9378 asks for time-of-day
decisions when picking pools, models and efforts, using provider peak pricing
where it exists and quota-window pressure where it does not. This document
specifies the policy, the config representation, the guardrails, the
measurement, and the open questions. The live one-pool switch with
route_attempt evidence and the before/after burn analytics are later packets
and start from this spec.

## Prior art

- `experimental.model_route_windows` in `.opencode/opencode.jsonc` already
  holds `deepseek-peak-early` and `deepseek-peak-late` (Mon-Fri 01:00-04:00 and
  06:00-10:00 UTC). The background chain composes them into three stages (see
  the comment above the chain around lines 4018-4053). This is the one
  peak-pricing window that exists today.
- `doc/2026-11-17_design_time_based_route_priority.md` (#8838, closed) designs
  runtime schedules as ranking preferences that stay eligible as fallback. It
  does not cover holidays, pre-burn, guardrails for priority-10 work or walled
  legs, or route_attempt measurement. This spec uses its fail-open,
  ranking-only model and adds the rest.
- `doc/2026-11-13T09-26-00Z_design_pool_effort_system/brief.md` (line 149)
  ticketed #9378 as a sibling of #9399. The effort tiers in #9399 are not a
  dependency of this spec.
- `developing-today/opencode#821` adds date-aware windows (calendar and
  holiday exclusion). It is the fork side of the holiday question in §2.3.

## 1. Policy

### 1.1 Window classes

Every time-based rule places a pool, leg or effort in one of three classes:

| Class      | Meaning                                                                                     |
| ---------- | ------------------------------------------------------------------------------------------- |
| `peak`     | Provider pricing or quota pressure is high for this window. Prefer cheaper or flatter legs. |
| `off-peak` | Pricing is at its lowest. Deferrable work may run here.                                     |
| `neutral`  | No time rule applies. Current chain order holds.                                            |

A class changes ordering. It never removes a leg from eligibility.

### 1.2 Decision inputs

| Input                                | Source                                                            |
| ------------------------------------ | ----------------------------------------------------------------- |
| Local time in the window's timezone  | Config `timezone` on the named window                             |
| Day of week                          | Same                                                              |
| Holiday flag for the date            | Calendar, see §2.3 (until then: none)                             |
| Pool used % and projected wall time  | Quota analytics (`/quota#analytics`, `/api/quota-analytics.json`) |
| Hours until the pool's window resets | Quota record                                                      |

### 1.3 Allowed actions

- **A1, prefer flat tiers in peak.** During a `peak` window, a flat-tier leg
  moves ahead of a metered or peak-priced leg of the same class.
- **A2, shift deferrable batch work off-peak.** Work whose priority is batch
  (not interactive) may start in an `off-peak` window ahead of work that would
  run now. Priority-10 work is excluded (G1).
- **A3, effort by window for the workhorse class.** In `peak`, the workhorse
  class may lower effort one rung, never below the class floor set by its
  agent requirement. Off by default until the effort tiers of #9399 exist (Q4).
- **A4, pre-burn a pool that would expire unused.** When a pool's projected
  usage at reset is well below 100% and its reset is near, its priority rises
  so that interactive and batch work use it before it expires. Pre-burn never
  takes a pool past its own quota rules (G3).

## 2. Config representation

### 2.1 Mechanism

The policy is data in `.opencode/opencode.jsonc`. It extends the existing
mechanism and adds no hard-coded time branch to route code:

1. **Named windows** (existing `experimental.model_route_windows`). Each window
   names days, start, end and timezone. A window may carry a `class` of
   `peak`, `off-peak` or `preburn` (new, optional, default `peak` for
   compatibility with the DeepSeek windows).
2. **Gated chain positions** (existing). A chain position is gated on a window
   or on its inverse, exactly as the DeepSeek chain composes its three stages.
   Time policy for a pool is expressed by gating its positions, not by code.
3. **Pool binding** (proposed). A pool may name the windows whose class
   drives A1-A4, so that the route reads a declared policy rather than a
   pool name.

Illustrative shape (proposed, not applied):

```jsonc
"model_route_windows": {
  "deepseek-peak-early": { "daysOfWeek": ["mon","tue","wed","thu","fri"], "start": "01:00", "end": "04:00", "timezone": "UTC", "class": "peak" },
  "deepseek-peak-late":  { "daysOfWeek": ["mon","tue","wed","thu","fri"], "start": "06:00", "end": "10:00", "timezone": "UTC", "class": "peak" },
  "quota-preburn-evening": { "daysOfWeek": ["sat","sun"], "start": "18:00", "end": "23:59", "timezone": "America/Chicago", "class": "preburn" }
}
```

The `class` key and the pool binding are the only schema additions. Both are
optional, so the shipped DeepSeek windows keep working unchanged.

### 2.2 Ranking, not availability

Time windows rank candidates. Stickiness, explicit user adjustments, and
provider health keep their current precedence. A low-ranked target remains a
fallback. This matches the fail-open model of #8838.

### 2.3 Holidays and opencode#821

The window schema has no dates or holiday lists. The DeepSeek windows therefore
treat a weekday Chinese public holiday as peak, although DeepSeek prices it
off-peak (the comment at `.opencode/opencode.jsonc` around line 4908). This
spec does not solve that. It names the dependency: `developing-today/opencode#821`
adds date-aware windows and the holiday list. Until #821 lands, the holiday
input in §1.2 is treated as "no holiday" and holiday days are labelled in
analytics so the loss can be measured.

## 3. Guardrails

- **G1, interactive is never starved.** Priority-10 (interactive) work is never
  demoted or delayed by a `peak` or `off-peak` window. Only batch work is
  shifted by A2.
- **G2, a walled leg stays walled.** A leg at 100% quota is never made eligible
  again by a time window. A window can reorder legs; it cannot reset quota.
- **G3, existing quota rules still apply.** Plan quotas only. A quota error is a
  wall, not a pay-as-you-go fallback. The DeepSeek direct reserve stays the sole
  named exception (`.opencode/instructions/routing-policy.md` §3).
- **G4, no leg is removed.** Time policy never deletes a leg from the chain.
  Every fallback in `route-fallbacks.md` stays reachable in every window.

## 4. Measurement

### 4.1 route_attempt fields

Every route attempt records the following, so a time-based switch can be
measured against the analytics:

| Field                        | Meaning                                              |
| ---------------------------- | ---------------------------------------------------- |
| `window_names`               | Named windows matched at attempt time (may be empty) |
| `window_class`               | `peak`, `off-peak`, `preburn` or `neutral`           |
| `holiday`                    | Boolean from the calendar, `unknown` until #821      |
| `pool_used_pct`              | Pool quota used at attempt time                      |
| `hours_to_reset`             | Hours until the pool window resets                   |
| `priority_band`              | Interactive (10) or batch                            |
| `leg`, `effort`, `served_by` | Which leg, effort and provider served it             |
| `cost_usd`, `tokens`         | Cost and tokens for the attempt                      |

### 4.2 Before and after metrics, per pool

- **Cost per token by window class.** Median cost per 1M tokens in `peak`
  versus `off-peak`. Expected direction: lower in `peak` after A1, with no rise
  in failover count.
- **Quota burn by hour of day.** Share of pool quota consumed per hour, before
  and after. Expected direction: less quota expiring unused (A4), with no pool
  reaching 100% earlier than before.
- **Failover count.** Failovers per 100 attempts, guarding G1-G4.

The before window is the two weeks before the live switch. The after window is
the same length after it.

## 5. Open questions and defaults

| #   | Question                                                        | Default                                                                                                      |
| --- | --------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| Q1  | Pools without provider peak pricing: what is the window source? | Quota analytics hour pattern. No time windows for them until two weeks of data exist.                        |
| Q2  | How is a weekday holiday handled before #821?                   | Holiday flag `unknown`; treated as the schedule says; labelled in analytics.                                 |
| Q3  | Does pre-burn (A4) override a user pin?                         | No. Explicit user adjustments keep precedence (§2.2).                                                        |
| Q4  | Is effort lowered in `peak` (A3)?                               | Off until the #9399 effort tiers exist.                                                                      |
| Q5  | Which timezone for a provider that documents none?              | UTC.                                                                                                         |
| Q6  | Does this spec own the runtime?                                 | No. The runtime ranking belongs to #8838's design. This spec defines the values and the classes it ranks by. |

## Acceptance mapping

1. Policy: per-pool window classes, decision inputs, and allowed actions A1-A4
   are in §1.
2. Config: data in `opencode.jsonc`, extending `model_route_windows`, with no
   hard-coded time branch (§2.1). Holiday exclusion is handed to
   `opencode#821` (§2.3).
3. Guardrails: G1 (priority-10 never starved), G2 (walled leg stays walled), and
   G3 (existing quota rules) are in §3.
4. Measurement: route_attempt fields and the before/after metric per pool are
   in §4.
5. Open questions with defaults are in §5. No config or code is changed by
   this packet.
6. Linked from #9378 with the dedupe key above. No implementation files are
   touched.
