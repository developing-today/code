# Design: per-function metering record shape and flow — where metering data goes if servers charge by function class

- **Date:** 2026-11-11
- **Status:** proposed design for review; no implementation is included here.
- **Issue:** [#6016](https://github.com/developing-today/code/issues/6016), part of the worlds design program [#5955](https://github.com/developing-today/code/issues/5955). Queue task 14819.
- **Parent topic:** [#5965](https://github.com/developing-today/code/issues/5965) (compile economics — penalty currency and gating matrix; this spec is referenced from that design, §9).
- **Inputs (both closed and merged; verified at main `2b3db37221`, 2026-11-11):**
  - [#5965](https://github.com/developing-today/code/issues/5965) / `doc/2026-11-05_design_id5965.md` — compile credits are compiler CPU-seconds, a budget **separate** from runtime fuel; the two compose only at tier standing (§5.3 of that design). Its §5.3 names this issue as the consumer of runtime metering data.
  - [#5966](https://github.com/developing-today/code/issues/5966) / `doc/2026-11-09T00-10-25Z_research_id5966_gas_timeouts_multithreading/` — metering is Wasmtime **fuel**, per binding, one independent store per binding; **actual consumption is debited, not the cap**; host work is charged in fuel-equivalent units from a single cost table; the engine version and cost-table version are recorded so cross-host comparisons stay honest (§5 invariants 3, 5, 7 of that research).
  - [#6001](https://github.com/developing-today/code/issues/6001) / `doc/2026-11-10T03-10-00Z_design_id_two_level_metering/` — the two-level budget shape (per-tick ceiling + rollover bucket) whose per-tick record (§3 step 6 of that design) this spec's event feeds: credit applied, balance before/after, spend, outcome, engine + cost-table version.
- **Scope boundary (from #6016, not re-argued):** prices, invoices, and payment are **non-goals**. This spec defines only the hooks and the record shape — the data a server *could* later charge or limit by function class (admin/patron tiers) without changing the execution model.

## Decision to make

Metering numbers exist today only inside the process that produced them. Decide (1) the exact shape of one per-function consumption event, (2) where events are aggregated — per-tick, per-session, per-account — and what each aggregation costs and flushes, and (3) what a downstream billing/quota reader may rely on, so that a later charging or limiting feature is a **reader** of this stream, never a change to the execution model.

## Recommendation

Emit **one immutable consumption event per binding invocation** (plus one per compile job, on a parallel shape with its own budget field), aggregate it **in memory per tick** into per-binding counters, flush per-tick aggregates into the **tick log** at tick commit, roll those up **per session** in memory with a bounded spill record, and roll sessions up **per account** in a durable append-only ledger keyed by `(account, world, function, day)`. Compile credits and runtime fuel are **distinct fields on distinct event kinds** and are never summed into one number (§4) — #5965's compose-don't-merge rule applied to the record layer. Every field in the event traces to a named consumer (§3.3); there are no speculative fields.

## Current state (evidence, verified at main `2b3db37221`)

| Existing fact | Design consequence |
| --- | --- |
| Per-call statistics exist per `GuestCall` kind (`init`, `update`, `view`, `layout`, `records`, `wants`, `snapshot`, `restore`): calls, total fuel, max, last (`pkgs/id/src/sandbox.rs:89-131`, `CallStats` / `CallMetrics`). They are **in memory only; never journaled** (#5966 §1). | The numbers this spec moves already exist at the call site (`metrics.record(GuestCall::…, fuel_spent(…))`, `sandbox.rs:352`). The work is a record + sink at that seam, not new metering. |
| Metering today is **per invocation of the world's guest, not per user function**: one `SandboxLimits.fuel` (default 10,000,000) applies to every invocation kind (`pkgs/id/crates/id-core/src/world_limits.rs:188-207`); the world calls `plaza_update` once per tick and user-function work inside shares one fuel account (#5966 §1). | The event's function identity (§3.1) is only truthful once #5966's one-store-per-binding structure lands. Until then an implementation may emit events with `function = the GuestCall kind` and `binding = the world itself` — the shape does not change when the binding structure does. That is the point of specifying the shape now. |
| `GuestCall` kinds name **host-to-guest export invocations**, not user functions and not guest-to-host capability calls (#5966 §1, Codex 4508414395 disposition). The guest is import-free today; there is no capability call to price. | `function_class` (§3.1) is a classification of the **binding's declared function**, never a relabelling of `GuestCall` kinds. Host-side charges price host *work product* (view/state bytes, intent count/size) in fuel-equivalent units, per #5966 §2.3 point 4. |
| Compile cost is metered separately and charged in compile credits (compiler CPU-seconds); runtime fuel is a separate budget; "no fuel is charged for compiling, and no compile credit buys fuel" (#5965 §5.1, §5.3). | Two event kinds (§3.2), two amount fields, no cross-kind arithmetic anywhere in the aggregation chain (§5). A reader that wants an author's total exposure reads both series; the record layer never merges them. |

## 1. Terms

- **Binding:** one user function bound into a world for one actor — the unit #5966 gives its own store and fuel account, and #5967's mapping invokes once per tick.
- **Function class:** the charging-relevant classification of a binding's function, declared at bind time: `admin`, `patron`, or `standard`. (These are the tier names a server *would* charge by, per #6016; the class is recorded whether or not any server ever charges. Renaming a class is a shape change, §6.)
- **Consumption event:** the immutable record of one invocation's (or one compile job's) measured consumption (§3). Events are facts about the past; nothing ever edits one.
- **Tick log:** the world's existing replayable per-tick record (#5966 §2.3 point 3, #5967 F10). The per-tick aggregate (§5.1) is an entry in it, so metering replays with the tick.
- **Ledger:** the durable per-account rollup store (§5.3). Append-only; corrections are new entries, never edits.

## 2. Flow at a glance

```
invocation ──► consumption event (in-memory ring, §3)
compile job ─► consumption event (kind = compile, §3.2)
                    │
        per-tick aggregate (§5.1) ──► tick log entry (durable with the tick)
                    │
        per-session rollup (§5.2) ──► session summary record (at session close;
                    │                  bounded spill every 1024 ticks)
        per-account rollup (§5.3) ──► ledger entry (account, world, function, day)
                    │
        consumers (§6): quota gate, tier standing, author-visible usage,
        adjudication audit — readers only, none in the execution path
```

No arrow points back into execution. A consumer being down, slow, or absent never blocks a tick; the tick log is the durability boundary and consumers catch up from it (§6.3).

## 3. The consumption event

### 3.1 Runtime event shape

One event per binding invocation, emitted at invocation end (including trapped invocations — a trap is an outcome, #5966 §3.3 point 6, and its consumption is real).

| Field | Type | Meaning |
| --- | --- | --- |
| `schema` | string, `"metering-event/v1"` | Shape version. Additive fields do not bump it; renames/removals do (§6.1). |
| `kind` | enum: `runtime` | Event family. Compile events use `kind = compile` (§3.2) and share only the identity fields. |
| `tick` | u64 | Tick index the invocation served. Replay derives the same index; wall-clock time is **never** a field (C4: fuel, not time, is the adjudicated currency, #5966 §5 invariant 2). |
| `world` | world id | The world the binding ran in. |
| `binding` | binding id | The binding invoked (owner + function identity per #5967). |
| `actor` | author/account key | Who the consumption is attributed to: the binding's owner. For a world-invoked binding this is the function author's key, not the world author's — charging follows the function's owner, which is what "charge by function class" means. |
| `function` | string | The binding's declared function name. |
| `function_class` | enum: `admin` \| `patron` \| `standard` | Declared at bind time (§1). |
| `call_kind` | enum: the `GuestCall` kinds | Which host-to-guest export carried the invocation (today's `CallMetrics` seam). Recorded for seam fidelity; it is **not** the function identity and no consumer may bill by it. |
| `fuel_spent` | u64 | **Actual** fuel consumed (before/after store read, #5966 §2.3 point 3), plus host-side charges in fuel-equivalent units from the single cost table, itemised in `host_charges`. On a fuel trap, the full granted budget (the mapping's existing debit rule). |
| `host_charges` | list of `(charge_kind, fuel_equivalent)` | Host work product the guest caused but did not execute: view bytes, state bytes, intent count/size, capability calls (future). Empty list is the common case today (import-free guest). |
| `budget` | record | The budget the spend drew against: `tick_ceiling`, `bucket_balance_before`, `bucket_balance_after`, `world_budget_remaining_after` (the #6001 §3 step-6 record, carried on the event so the tick aggregate is a pure sum). |
| `outcome` | enum: `ok` \| `fuel-exhausted` \| `timeout` \| `bucket-exhausted` \| `world-budget-exhausted` \| `error` | The invocation's outcome row (mapping failure table; `bucket-exhausted` is #6001 §7's row; a `bucket-exhausted` binding is not invoked and emits **no** event — its non-invocation is recorded in the tick aggregate's counters, §5.1, because there is no spend to attribute). |
| `engine_version` | string | Pinned Wasmtime version (`wasmtime = "=48.0.7"` today). A change is a new adjudication configuration (#5966 §5 invariant 7). |
| `cost_table_version` | string | Version of the single host-charge cost table (#5966 §2.3 point 4). |

### 3.2 Compile event shape

One event per compile job, sharing `schema`, `tick`-free identity (`world` replaced by the compile's target artifact hash where a world does not exist yet), `actor`, and versions. Distinct fields, per #5965 §5:

| Field | Meaning |
| --- | --- |
| `kind` | `compile` |
| `compile_credits` | Measured compiler CPU-seconds; **0 on a cache hit** (#5965 §5.1: hits are free — the behaviour the economics must reward); failed compiles carry the discounted measured cost (#5965 §5.1). |
| `compile_outcome` | `hit` \| `success` \| `failed` \| `delayed` (token-bucket delay is an event too: the delay is the penalty landing, and tier standing reads it). |
| `tier_at_compile` | The author's tier in the §6 matrix of #5965 (`free` \| `contributor` \| `patron` \| `admin`). Runtime events carry no tier field: runtime gating does not exist, and adding one would invite a consumer to invent it. |

There is **no** field anywhere that sums `fuel_spent` and `compile_credits`. They are different currencies measuring different resources (guest execution vs compiler CPU); a combined number would be precisely the merge #5965 §5.3 forbids.

### 3.3 Field-to-consumer trace (acceptance: no speculative fields)

| Field(s) | Consumer that requires it |
| --- | --- |
| `actor`, `function`, `function_class`, `world` | Per-account rollup (§5.3) — the charging/limiting key if a server charges by function class; tier standing (§6). |
| `fuel_spent`, `host_charges` | Quota gate (spend vs budget); author-visible usage (the Screeps property: accounting the user can see, #6016); adjudication audit (actuals, not caps). |
| `budget` | #6001 two-level metering: the bucket/ceiling state transition must be replayable from the record, or the bucket is not auditable. |
| `outcome` | Mapping failure table / strike discipline; a trapped invocation's spend is attributed only if its outcome is on the record. |
| `tick`, `binding` | Tick-log join and replay; per-session rollup grouping. |
| `call_kind` | Migration seam only: it lets today's per-`GuestCall` `CallMetrics` emit conforming events before per-binding stores land (§ Current state). Its consumer is the migration, and it is marked for review at `metering-event/v2`. |
| `engine_version`, `cost_table_version` | Adjudication: cross-host comparison is valid only within one configuration (#5966 §5 invariant 7); the rollup must be able to partition by configuration. |
| `compile_credits`, `compile_outcome`, `tier_at_compile` | #5965 token-bucket enforcement and tier standing — compile-side only. |

Any future field is admitted only by naming its consumer in this table in the same change that adds the field.

## 4. The compose-don't-merge rule at the record layer

#5965 §5.3: compile credits and runtime fuel compose only at tier standing. Concretely for records:

1. Aggregations (§5) sum **within** a kind and **within** a currency, never across. A rollup row is either a runtime row (fuel) or a compile row (credits).
2. Tier standing is a **derived view** that reads both series and applies the gating matrix; it is computed by its consumer, never stored as a merged total on an event or rollup.
3. An author's worlds keep running on runtime fuel regardless of compile-credit standing (#5965 §5.2 invariant) — no consumer of this stream may write a runtime admission decision from compile data. The record layer enforces this by giving the quota gate (§6) only the runtime series.

## 5. Aggregation points

### 5.1 Per-tick (hot path) — cost bound

- **Where:** in the host, at invocation end, beside the existing `CallMetrics::record` call. The event is appended to a **fixed-capacity in-memory ring** (one per world) and folded into the tick's per-binding counters; at tick commit the counters — not the raw events — are written as one tick-log entry.
- **Hot-path cost bound (the acceptance criterion, stated):** per invocation, aggregation is **one counter fold: O(1) arithmetic on ≤ 8 counters, zero allocation after ring warm-up, zero I/O**. No serialisation, no formatting, no locking beyond the world's existing tick lock. The per-tick flush is one tick-log append of the aggregated entry, O(bindings invoked this tick), paid once at commit where the tick log already writes. If a proposed implementation cannot state this bound, it is not this design.
- **Ring overflow:** the ring holds aggregates, so overflow loses only event-level detail, never totals: counters are folded **before** ring insertion, and a dropped-detail counter on the tick entry records how many events were folded without retention. Totals are exact under all conditions; detail is best-effort. (Billing-grade detail, if ever needed, is a consumer-side decision to widen the ring — config, not shape.)
- The tick entry also carries the count of `bucket-exhausted` non-invocations (§3.1) per binding, so a throttled function is visible without inventing a zero-spend event.

### 5.2 Per-session

- **Where:** the world session (one world instance's lifetime on one host). Session rollup is a running fold of tick entries by `(binding, function_class, outcome)`: fuel totals, invocation counts, trap counts, host-charge totals by kind.
- **Flush:** one session summary record at session close; plus a **spill record every 1024 ticks** for long sessions, so a crashed host loses at most 1023 ticks of session-level detail — and loses **no totals**, because the tick log (§5.1) is already durable and the session rollup is re-derivable from it. The session rollup is a cache of the tick log, never its replacement.

### 5.3 Per-account (durable ledger)

- **Key:** `(actor, world, function, function_class, day, kind, engine_version, cost_table_version)` — day is a UTC calendar bucket used only for rollup grouping; it is a ledger key, not an event field, and no fairness or adjudication rule reads it.
- **Flush mechanism:** an asynchronous rollup job folds session summary records (and, for compile events, the compile event stream) into ledger rows. It is **at-least-once with idempotent rows**: each ledger row carries the `(session, last tick)` / event-range watermark it covers, and re-folding the same range rewrites the identical row. A rollup outage delays visibility; it cannot lose or double-count, because the tick log and session records are the source and the watermark is the cursor.
- **Retention:** events (tick entries) follow tick-log retention; ledger rows are retained indefinitely (they are the smallest durable form: one row per key per day).

## 6. Consumer contract (billing/quota readers)

### 6.1 What a reader may assume

1. **Shape stability:** `metering-event/v1` fields are never renamed, removed, or re-typed within v1. Additive fields may appear; readers must ignore unknown fields. A breaking change is a new `schema` value, and both shapes are emitted during any migration window.
2. **Totals are exact; detail is best-effort** (§5.1). A reader that bills or limits must read **aggregates/ledger rows**, never raw ring detail.
3. **Actuals, not caps:** every amount is measured consumption (§3.3), except the trap rule (full granted budget on fuel exhaustion), which is itself the mapping's settled debit rule and is identifiable by `outcome = fuel-exhausted`.
4. **Currency separation:** runtime rows are fuel; compile rows are credits; no row mixes them (§4).
5. **Determinism partition:** amounts are comparable only within one `(engine_version, cost_table_version)` partition (§3.3).

### 6.2 What a reader may not do

- Feed a decision back into execution synchronously. Quota/limiting enforcement reads the **budget state** in the execution path (the #6001 bucket, the #5965 token bucket) — those are mechanisms that already exist; this stream is their **audit and visibility** layer, not a second enforcement path.
- Treat `call_kind` as function identity, or `function_class` as a price. The class is a recorded declaration; any price mapping is an operator decision outside this spec (and outside #6016).
- Infer wall-clock behaviour from records. There is no time field to infer from, deliberately.

### 6.3 Availability

Consumers read the tick log / ledger, never the live ring. A consumer outage has no execution effect (§2); a rollup outage delays ledger freshness and must surface as a stale-watermark signal on the ledger, so a reader can tell "no consumption" from "consumption not yet rolled up".

## 7. Non-goals (per #6016, explicit)

- No prices, price tables, invoices, or payment of any kind.
- No runtime implementation: no store restructuring, no bucket code, no rollup job. Implementation tranches cite this spec; the first is emitting conforming events at the `CallMetrics` seam with `binding = world` (§ Current state).
- No change to any budget, ceiling, or gating value in #5965, #5966, #6001, or #5967. This spec moves their numbers; it decides none of them.
- No cross-server federation of metering records (deferred with #5965's cross-server deferral).

## 8. Acceptance mapping (issue #6016 "done looks like")

| Asked | Settled in |
| --- | --- |
| Metering records: per-function consumption events — who, which function, which world, how much, which budget | §3.1 (shape), §3.3 (every field traced to a consumer), §4 (budgets stay distinct) |
| Aggregation points: per-tick (hot path, nearly free), per-session, per-account rollups | §5.1 (with the stated hot-path cost bound), §5.2, §5.3 (flush mechanisms named for each) |
| What is not built: prices, invoices, payment — only hooks and record shape | §7, §6.2 |
| A record-shape + flow spec referenced by the compile-economics design | §9 (the reference added to `doc/2026-11-05_design_id5965.md` by the same change that lands this spec) |

## 9. Reference from the compile-economics design

`doc/2026-11-05_design_id5965.md` §5.3 already names #6016 as the consumer of runtime metering data. The change landing this spec adds to that section a pointer to this document as the record-shape + flow specification that consumption follows, so a reader of the penalty design reaches the record definition in one hop.

## Sources

- Code (main `2b3db37221`): `pkgs/id/src/sandbox.rs:89-131` (`CallStats`/`CallMetrics`), `:352` (`metrics.record` seam), `:417` (`fuel_spent`); `pkgs/id/crates/id-core/src/world_limits.rs:188-207` (`SandboxLimits`, default fuel 10,000,000).
- Designs/research: `doc/2026-11-05_design_id5965.md` (§5.1–5.3, §6); `doc/2026-11-09T00-10-25Z_research_id5966_gas_timeouts_multithreading/` (§1, §2.3, §5); `doc/2026-11-10T03-10-00Z_design_id_two_level_metering/` (§3 step 6, §7).
- Issues: [#6016](https://github.com/developing-today/code/issues/6016), [#5965](https://github.com/developing-today/code/issues/5965), [#5966](https://github.com/developing-today/code/issues/5966), [#6001](https://github.com/developing-today/code/issues/6001), [#5967](https://github.com/developing-today/code/issues/5967), [#5955](https://github.com/developing-today/code/issues/5955). Triage note 6784623736 on #6016 (task 14338).
