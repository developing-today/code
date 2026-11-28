# Design: two-level metering — burst bucket + hard per-tick ceiling, per function

- **Date:** 2026-11-10
- **Status:** proposed design for review; no implementation is included here.
- **Issue:** [#6001](https://github.com/developing-today/code/issues/6001), part of the worlds design program [#5955](https://github.com/developing-today/code/issues/5955). Wave-2 follow-up from the prior-art research; parent topic issue [#5966](https://github.com/developing-today/code/issues/5966).
- **Doctrine:** fuel/metering is a C4 platform pillar (Drew, 2026-11-04 23:57 CST, primary record on #5966): research thoroughly, hunt holes, expect rework before it lands; test-environment-only first cuts are fine, architectural dead ends for later user-friendliness are not.
- **Inputs:** [#5966](https://github.com/developing-today/code/issues/5966) / [PR #7874](https://github.com/developing-today/code/pull/7874) (per-function gas research — owns the cutoff mechanism: Wasmtime fuel, one independent store per binding, actual-consumption debit, pinned engine + cost table); [#5967](https://github.com/developing-today/code/issues/5967) / [PR #7625](https://github.com/developing-today/code/pull/7625) (per-user function mapping — owns the per-function budget, the failure table, deterministic order, fairness); [#5962](https://github.com/developing-today/code/issues/5962) / [PR #7977](https://github.com/developing-today/code/pull/7977) (server invariants — owns budget scopes B2/B3 and enforcement classes; this design's bucket _throttles_ beneath B3's hard ceiling, exactly the seam that record names); [#6002](https://github.com/developing-today/code/issues/6002) (cooperative timeout contract — owns the voluntary-yield convention the self-observation API serves).
- **Consumer:** the mapping design (#5967). Its per-function budget is spent against the two-level shape defined here; its failure table gains one row (§7); nothing in #5967 is re-decided.
- **Deliverable, per the issue:** a metering design doc the mapping design can implement against, with the Screeps parameters mapped to ours term-by-term (§2).

## Decision in brief

Adopt the Screeps two-level shape, denominated entirely in fuel, per binding:

1. **Level 1 — hard per-tick ceiling.** A binding may never spend more than `tick_ceiling` fuel in one tick, no matter how much it has saved. Enforced by the mechanism #5966 already settled: the binding's store fuel is set to the ceiling before the call; exhaustion traps. Enforcement class: **hard termination** (the mapping's fuel-exhausted row).
2. **Level 2 — average budget with a rollover bucket.** Each tick the binding's bucket is credited `refill` fuel, up to `bucket_capacity`. A tick's spend is debited from the bucket. A binding that is idle or cheap accumulates the right to burst later; a binding that bursts must then live within the refill until the bucket recovers. Enforcement class at empty bucket: **throttle** (the invocation is skipped this tick, not killed — §7).
3. **The world ceiling composes above both.** The sum of all bindings' spend in a tick never exceeds the world's per-tick budget (B3 of #5962), admitted sequentially in mapping order (#5966 invariant 8). Per-function levels shape _when a function may spend_; the world ceiling shapes _whether the world may spend at all this tick_.
4. **Self-observation is a first-class API,** not a debug surface: a `getUsed()`-style read (§4) so well-behaved functions can throttle themselves cooperatively (#6002); hard termination stays the backstop for functions that don't.
5. **All three parameters are world-author configuration,** server-capped (§5). No numeric defaults are invented here: values are **TBD** pending the calibration harness of #5966 §6, and this record fixes the _relations_ between them that must hold for any values chosen.

## 1. What this design takes as given (and does not redo)

- **Currency is fuel, only fuel.** Wall-clock never enters accounting, admission, or adjudication (C4; #5966 §5 invariant 2). Bucket arithmetic is a function of tick index and recorded spend — never of time — so a tick replays identically from the tick log.
- **One binding, one store, one fuel account** (#5966 §2.3). The bucket belongs to the binding (owner + function), persists across ticks, and is reset by nothing except the rules in §3.
- **Actual consumption is debited, not the cap** (#5966 §2.3 point 3), including host-side charges in fuel-equivalent units from the single cost table (#5966 §2.3 point 4). A `getUsed()` read that hid host charges would be a lie the design cannot afford (§4).
- **Sequential world-budget admission** (#5966 §5 invariant 8): parallel execution is an optimisation that must reproduce the sequential outcome set exactly, bucket states included.

## 2. Screeps, term by term

Screeps is the production gold standard the issue names: per-tick CPU limit, an accumulating bucket capped at 10,000, burst draw up to 500 CPU in a tick, termination on overrun, and `Game.cpu.getUsed()` for cooperative self-throttling (https://docs.screeps.com/cpu-limit.html). The mapping:

| Screeps term                      | Screeps semantics                                                 | Ours                                                                                                           | Difference and why                                                                                                                                                                                                                                       |
| --------------------------------- | ----------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| CPU limit (per tick, per account) | Average sustainable spend per tick                                | `refill`: fuel credited to the binding's bucket per tick                                                       | Ours is **per function**, not per account — the adaptation the issue calls for. Per-account fairness is recovered by the world ceiling and the anti-splitting rule (§8), not by making the bucket per-account.                                           |
| Bucket                            | Unused limit accumulates, capped at 10,000                        | `bucket`: a per-binding fuel balance, credited `refill`/tick, capped at `bucket_capacity`                      | Screeps' cap is ~20 ticks of limit; ours is a configured multiple of `refill` (§5), because our tick cost distribution is not Screeps'.                                                                                                                  |
| Burst (up to 500/tick draw)       | A tick may spend far above the average by drawing the bucket down | Any spend above `refill` in a tick draws the bucket; bounded by `tick_ceiling` and by the bucket balance       | Screeps has one number (500) doing both jobs — burst allowance and hard cap. We separate them: the bucket bounds _how much saved credit exists_; the ceiling bounds _how fast it may be spent_. A full bucket must not license an unbounded single tick. |
| Hard per-tick termination         | Exceeding available CPU terminates the script for that tick       | Store fuel set to `min(tick_ceiling, bucket_balance)` before the call; trap on exhaustion = fuel-exhausted row | Same shape, fuel mechanism (#5966). Partial work in the tick is not committed (mapping failure table).                                                                                                                                                   |
| `Game.cpu.getUsed()`              | Code reads its own consumption and self-throttles                 | `getUsed()` / `getBucket()` / `getCeiling()` host reads (§4)                                                   | Screeps players built per-process sub-accounting in userland on top of `getUsed()` deltas; we ship the delta primitive per binding so world authors can build schedulers the same way (§4.3).                                                            |
| CPU as currency                   | Players budget, trade off, and plan around CPU                    | Fuel is the single currency for compute _and_ host work (cost table)                                           | One currency, one ledger (the tick log), no exchange rate between "compute fuel" and "host fuel".                                                                                                                                                        |
| Per-account scope                 | One player, one bucket                                            | Per binding, composed under one world ceiling                                                                  | See row 1. The composition is the design work; the levels themselves are Screeps'.                                                                                                                                                                       |

## 3. The accounting shape

Per binding, the persistent metering state is exactly:

```
bucket_balance : fuel (integer, 0 <= bucket_balance <= bucket_capacity)
spent_last_tick: fuel (recorded, for observability; not an input to the rules)
```

Per tick, in mapping order, for each binding:

1. **Credit.** `bucket_balance = min(bucket_capacity, bucket_balance + refill)`. Credit happens on tick boundaries only, once per tick, whether or not the binding is invoked. A binding created mid-run starts at `bucket_balance = refill` (one tick of credit — enough to run, not enough to burst; starting full would hand every new binding a free burst, §8).
2. **World admission.** If the world's remaining per-tick budget is zero, the binding takes the `world-budget-exhausted` outcome (mapping §6, unchanged). Its bucket keeps the credit — world exhaustion is not the function's fault and must not tax it.
3. **Bucket admission.** If `bucket_balance == 0`, the binding is **not invoked**: outcome `bucket-exhausted` (§7), enforcement class throttle. No trap, no strike.
4. **Invoke.** Set store fuel to `min(tick_ceiling, bucket_balance, world_remaining)`. Run under #5966's mechanism.
5. **Debit.** `spend = actual fuel consumed + host charges`. `bucket_balance -= spend`; debit the world budget by the same `spend`. On a trap, `spend` is the full amount the store was set to (the mapping's existing rule: exhaustion debits the whole budget granted).
6. **Record.** Tick log carries, per binding: credit applied, balance before/after, spend, outcome, engine + cost-table version (F10 of #5967; #5966 §5 invariant 3). The bucket state after the tick must be reproducible from the log alone — that is the replay oracle (§9).

Non-rules, stated because each was a real userland failure mode in Screeps-like systems: no borrowing against future refills; no transfer of balance between bindings (a transfer market is a different design and a Sybil hole, §8); no decay of balance other than the cap; no credit for ticks in which the _world_ did not run (a paused world accrues nothing — credit is per executed tick, so downtime is not a bucket-farming strategy).

## 4. Self-observation API

### 4.1 The reads

Three host-provided reads, available to the binding's function during its invocation:

- `getUsed()` → fuel spent so far **in this invocation**, including host charges posted up to the last charge point. Monotonic non-decreasing within the invocation.
- `getBucket()` → the binding's bucket balance as of invocation start, minus `getUsed()` — i.e. what remains spendable from the bucket this tick, before the ceiling is considered.
- `getCeiling()` → the effective hard limit for this invocation: `min(tick_ceiling, bucket_balance_at_start, world_remaining_at_admission)`.

### 4.2 Semantics that make the reads honest

- **Snapshot at charge points.** Fuel is read back from the store at host-call boundaries (#5966's readback), so `getUsed()` is exact at every point the guest can call it — the guest cannot execute unmeasured work between charge points, because all guest compute is fuel-metered by construction.
- **The reads themselves are charged** at the cost table's fixed host-read rate (small, constant, published). A read that were free would be an unmetered loop hole in any future capability surface; a read whose cost were variable would make `getUsed()` non-monotonic in effect. Constant charge, disclosed in the cost table.
- **No read can change an outcome.** The reads expose state; admission and debit are computed from the store counter and the log, never from values the guest claims to have seen. A function that ignores `getUsed()` gets exactly the hard-termination behaviour of §3 — cooperation is an optimisation for the well-behaved, never a requirement for safety. This is the seam with #6002: its cooperative contract (check-then-yield) is built on `getUsed()`; the yield outcome is `cooperative-yield`, distinct from every failure row.

### 4.3 The userland scheduler primitive

Screeps' evidence is that per-process accumulators built on `getUsed()` deltas are what users actually want. The per-binding tick-log record (§3 step 6) plus `getUsed()` gives a world author the same primitive one level up: a world-side scheduler function can read each child binding's last-tick spend from the log surface and apportion its own behaviour accordingly. We do not ship a scheduler; we guarantee the deltas are exact, per binding, and in the same currency — the three properties userland schedulers were built on.

## 5. Configuration: world-author parameters, server-capped

Bucket parameters are world-author configuration, not constants (the issue's requirement). The server caps every parameter (#5962: a world cannot grant itself an unbounded backstop — that rule is applied here to budgets).

| Parameter         | Meaning                         | Author may                                  | Server cap (relation, value TBD)                                                                                                                                                                                                                                                           |
| ----------------- | ------------------------------- | ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `refill`          | Fuel credited per tick          | Set per world; optionally per binding class | `refill <= world_per_tick_budget` (a single binding's average can never exceed the world's whole tick)                                                                                                                                                                                     |
| `bucket_capacity` | Max balance                     | Set per world                               | `bucket_capacity <= burst_ticks_max * refill`, with `burst_ticks_max` a server constant — the cap is a _duration_ of saving, so raising `refill` cannot silently multiply burst size                                                                                                       |
| `tick_ceiling`    | Hard per-tick spend per binding | Set per world; lower per binding            | `tick_ceiling <= min(bucket_capacity, world_per_tick_budget)` and `<=` the server per-invocation fuel maximum (today's landed default invocation limit is 10,000,000 fuel, `SandboxLimits.fuel`, #5962 E1 — the ceiling can lower it per binding, never raise it above the server maximum) |

Defaults for all three are **TBD**, set by the calibration harness (#5966 §6 step 5: publish the fuel cost of standard operations, then size budgets without guessing). What is _not_ TBD is the shape: any default set must satisfy `refill <= tick_ceiling <= bucket_capacity`, and a world that sets nothing gets the server defaults as a triple, never a mix of author and server values per parameter (mixed provenance is how a world ends up with a ceiling below its refill and a permanently throttled function — the configuration must be validated as a triple at bind time, hard refusal on an inconsistent triple).

Per-binding overrides may only _lower_ `tick_ceiling` and _lower_ effective `refill` (a world author rationing one expensive function), never raise either above the world values — the same lowerable-only pattern the codebase already uses for execution profiles (#5962 E1).

## 6. How the levels interact — worked cases

- **Steady function** spending `refill` per tick: bucket stays wherever it was; ceiling never binds. The two levels are invisible — the common case must feel like today's flat budget.
- **Bursty function** idle 9 ticks, heavy on the 10th: accumulates `10 * refill` (subject to cap), spends up to `min(tick_ceiling, balance)` on the heavy tick, then returns to refill-rate living while the bucket rebuilds. This is the Screeps burst, per function.
- **Greedy function** spending its ceiling every tick: drains the bucket in `bucket_capacity / (tick_ceiling - refill)` ticks, then is bucket-admission-throttled to an average of `refill` per tick. The ceiling shapes the burst; the refill shapes the average; neither alone gives both.
- **World under contention:** summed demand exceeds the world budget. Sequential admission serves bindings in mapping order until the world budget is spent; the rest take `world-budget-exhausted` with buckets intact (§3 step 2). Deterministic rotation/fairness across ticks is the mapping's rule (#5967), not this record's.

## 7. Outcome rows (the mapping failure table gains one)

| Outcome                             | Trigger                                            | Enforcement class (#5962)                                     | Strike?                                              | Bucket effect                                                 |
| ----------------------------------- | -------------------------------------------------- | ------------------------------------------------------------- | ---------------------------------------------------- | ------------------------------------------------------------- |
| `bucket-exhausted` (**new**)        | `bucket_balance == 0` at admission                 | Throttle — invocation skipped, retried next tick after credit | No — an empty bucket is arithmetic, not misbehaviour | Credit still applied this tick; balance unchanged by the skip |
| `fuel-exhausted` (existing)         | Trap at the invocation limit                       | Hard termination                                              | Per mapping (strike rules unchanged)                 | Full granted amount debited                                   |
| `world-budget-exhausted` (existing) | World budget spent before this binding's turn      | Throttle                                                      | No                                                   | Untouched (§3 step 2)                                         |
| `cooperative-yield` (#6002)         | Function checks `getUsed()` and yields voluntarily | — (normal completion path)                                    | No                                                   | Actual spend debited                                          |

The one genuinely new row is `bucket-exhausted`, and its class is the point: Screeps terminates on overrun because it has one level; with two levels, _admission_ failure (nothing saved) and _overrun_ (spent past the hard limit mid-flight) are different events and must not share a punishment.

## 8. Holes hunted (C4: expect this list to grow in review)

- **Function splitting.** Per-function buckets invite splitting one workload into N bindings to multiply refill. Mitigation is structural, not arithmetic: the world ceiling caps the aggregate, and the server-invariant record (§3/B3) needs a companion **per-owner aggregate refill cap** across all bindings in a world — named here as a required seam for #5962's implementation, since a per-binding design alone cannot close it.
- **Bucket farming by churn.** Unbind/rebind to reset state: a new binding starts at one `refill` (§3 step 1), strictly worse than keeping a farmed bucket, so churn never profits. Bind-time cost is #5964/#5965's seam, not this record's.
- **Idle-world farming.** Credit accrues per executed tick only (§3); a world cannot accrue while paused, and the cap bounds total stored burst regardless.
- **Ceiling laundering via host work.** If host charges were debited to a different account, a function could spend past its ceiling in host work. They are not: one currency, one debit (§1).
- **Replay divergence.** Bucket state is integer arithmetic over logged spends and tick indices — no wall-clock, no float, no host-dependent rounding. Any implementation that introduces a float or a time source into §3's rules breaks the replay oracle and is a defect against this design, not a variation of it.

## 9. Invariants implementation must hold (each with its oracle)

1. **Conservation.** For every binding and tick: `balance_after = min(cap, balance_before + refill) - spend`, with `0 <= spend <= min(tick_ceiling, balance_before + refill, world_remaining_at_admission)`. Oracle: a sequential reference model of §3, property-tested against the implementation over random tick/spend sequences.
2. **Ceiling absoluteness.** No invocation's recorded spend ever exceeds `tick_ceiling`, including host charges. Oracle: tick-log audit over the property-test corpus.
3. **Bucket isolation.** One binding's spend never changes another binding's balance (world budget excepted, and that exception is itself logged). Oracle: reference-model comparison with permuted binding orders — balances identical, only `world-budget-exhausted` outcomes may move, and only as sequential admission dictates.
4. **Replay identity.** Replaying the tick log reproduces every bucket balance exactly. Oracle: log-replay check in the tick pipeline.
5. **Monotonic self-observation.** Within one invocation, successive `getUsed()` reads never decrease. Oracle: instrumented guest in the measurement harness (#5966 §6).

## 10. Calibration (what sets the TBDs)

The defaults wait on #5966 §6's harness: fuel cost of a representative tick and of the standard view/state/intent operations. With those numbers, sizing is mechanical: `refill` = the representative function's average tick cost at the target headroom; `tick_ceiling` = the p99 tick cost the world is willing to tolerate from one function; `bucket_capacity` = `burst_ticks_max * refill` with `burst_ticks_max` chosen from the burst durations world authors actually program against (Screeps' ~20 ticks of limit is the only production data point; it is a starting hypothesis, not a default).

## 11. Open questions

- **Per-owner aggregate refill cap** (§8): value and scope (per world? per owner across worlds?) belong to the #5962 implementation; this design requires the seam to exist.
- **Bucket visibility for function users.** The function _author_ sees `getBucket()` in-flight; does the _owner_ get a per-binding balance surface out-of-band? #6017 owns observability surfaces; the tick log already carries the data.
- **Negative-space refill.** Should a world be able to set `refill = 0` for a binding — a pure burst-allowance function that runs only on saved credit it can never earn? Currently incoherent (it could never earn its first credit beyond the starting `refill`, which §3 sets to `refill` = 0); bind-time validation should refuse `refill = 0` unless this design is amended to want it.

## 12. Next steps

1. Land this record as #6001's deliverable; link it from #5966 §8's follow-ups, the mapping record (#5967), and #5962's record (the B2/B3 seam and the per-owner aggregate cap of §8).
2. Open an implementation ticket for the **bucket state + tick algorithm** (§3, §9) against the per-binding store work #5966 §8 already calls for — one ticket, since bucket state without per-binding stores has nothing to attach to.
3. Open an implementation ticket for the **self-observation reads** (§4) together with #6002's cooperative contract, which consumes them.
4. Run the calibration (§10) before any default is hard-coded; record the chosen triple in the implementation PR.

## Sources

- Issue #6001 (scope, Screeps model, done-looks-like); Drew's C4 ruling on #5966 (2026-11-04 23:57 CST).
- `doc/2026-11-09T00-10-25Z_research_id5966_gas_timeouts_multithreading/` (mechanism, invariants, measurement plan).
- `doc/2026-11-08T10-26-00Z_design_id_per_user_function_mapping/` (budget consumer, failure table, fairness).
- `doc/2026-11-09T05-15-00Z_design_id_server_invariants/` (budget scopes B2/B3, enforcement classes, E1 landed fuel default).
- Screeps CPU documentation: https://docs.screeps.com/cpu-limit.html ; https://wiki.screepspl.us/CPU/
