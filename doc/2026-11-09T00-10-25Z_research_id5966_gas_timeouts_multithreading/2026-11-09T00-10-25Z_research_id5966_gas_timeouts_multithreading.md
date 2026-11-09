# Research: per-function gas metering, timeouts, and the multithreading question for user functions

- **Date:** 2026-11-09
- **Status:** research record for #5966; recommendations, no implementation included.
- **Issue:** [#5966](https://github.com/developing-today/code/issues/5966), part of the worlds design program [#5955](https://github.com/developing-today/code/issues/5955) (statements S40-S42 of the 2026-11-04 brief).
- **Consumer:** the per-user function mapping design [#5967](https://github.com/developing-today/code/issues/5967) (landed doc `doc/2026-11-08T10-26-00Z_design_id_per_user_function_mapping/`), which spends a per-function fuel budget, a timeout row, and F10 per-user metering, and names this research as the owner of the cutoff mechanism.
- **Method:** fresh reads of the current host at `origin/main` e5a4fdcbcb (`pkgs/id/src/sandbox.rs`, `pkgs/id/src/world_compile.rs`, `pkgs/id/src/world_session/`, `pkgs/id/Cargo.toml`), the landed #5967 design, and the wasmtime / Roc ecosystem state. Code claims cite files; ecosystem claims are labelled as such and bounded where the sources disagree.

## What the current host already enforces (evidence)

- **Runtime:** user/world code runs as wasm guests under **wasmtime =48.0.7** (`pkgs/id/Cargo.toml`, `sandbox` feature), behind the plaza ABI (exports `plaza_init`, `plaza_update`, `plaza_snapshot`, ...; no guest imports allowed).
- **Fuel exists, per Store:** `Sandbox::compile` sets `config.consume_fuel(true)`; every Store gets `store.set_fuel(self.limits.fuel)` at instantiation and again before each run (`pkgs/id/src/sandbox.rs`, call sites at instantiate and per-run paths). Fuel exhaustion traps the call.
- **Per-call accounting exists for world code:** the sandbox records per-`GuestCall` statistics (calls, fuel total / max / last) as each host call is metered — this is the F10 metering the #5967 design cites for GuestCall kinds.
- **No execution timeout / interruption exists:** no `epoch_interruption`, no store deadline, and no async cancellation wraps a running guest anywhere in `sandbox.rs` or `world_session/` (grep at e5a4fdcbcb). The only timeout in the execution path is the **compile** timeout: `world_compile.rs` kills a Roc compilation subprocess after `COMPILE_TIMEOUT` (120 s). A guest that never returns from `plaza_update` today runs until fuel exhaustion — fuel is the only bound on a step.
- **One long-lived instance per world:** `Sandbox::instantiate` creates one guest instance per world; user functions in the #5967 mapping sense (one binding per user) do not exist as separate instances yet. Per-user-function bounding must therefore be built either as fuel re-arming between sequential step calls on a shared Store, or as one Store/instance per binding.

## Q1 — Per-function gas: can specific functions be rate-limited?

**Answer: yes — natively, in this stack, with machinery the host already runs.** Fuel in wasmtime is a property of a Store, charged as guest code executes, and a call traps deterministically when the budget is spent. Because a tick invokes bindings sequentially (mapping design §2), per-function gas does not need per-function instances:

- Before invoking a binding's `step`, arm its Store with exactly that binding's per-tick fuel budget (`set_fuel`).
- After the call, read the remaining fuel; the difference is the binding's consumption, debited to its account and written to the tick log (this discharges the F10 remainder the #5967 design assigns here: per-*user*-function metering inside a world, auditable per user per tick from the log alone).
- Fuel exhaustion mid-step is the mapping design §3 fuel-exhausted row: default intent, pre-tick state, strike +1.

### Options evaluated

| Option | Mechanism | Verdict |
| --- | --- | --- |
| **Wasmtime fuel (native)** | `consume_fuel(true)` + per-call `set_fuel`; deterministic, host-enforced, already in `sandbox.rs` | **Recommended.** Only option that is deterministic across hosts — and determinism is the fairness rule (C4, mapping §6): fuel consumption is identical on every host, wall-clock is not. |
| Host-side per-capability accounting | Charge fixed fuel prices per GuestCall kind (the per-GuestCall stats already collected) | **Adopt as a complement, not a substitute.** It prices host work (which fuel undercounts — a host call is one step to the guest, real work to us), but alone it cannot bound a guest-side compute loop that makes no calls. Layer it on top of fuel: total debit = guest fuel spent + sum of per-call prices. |
| Instruction counting by instrumentation | Rewrite the guest to count instructions (third-party metering passes) | **Rejected.** Duplicates what the engine already does, with higher overhead and a second toolchain to trust; no benefit over native fuel in a wasmtime host. |
| Cooperative yield points | Guest checks a budget at loop heads / call sites and yields | **Rejected as the mechanism; fine as a guest convenience.** It is unenforceable against a buggy or hostile function — exactly the case the budget exists for. Anything a function can forget to do cannot be the bound. |

### The overhead tax (the performance tension, quantified as far as sources allow)

Fuel metering is not free: wasmtime injects fuel checks into compiled guest code, so metered execution is slower than unmetered. Published wasmtime experience puts the cost in the **single-digit to ~30 percent** band depending on how block-dense the code is (tight small-block loops pay most; call-heavy code pays least, and there the per-call accounting dominates anyway). No honest single number exists for *our* guests — plaza-ABI Roc functions are call-structured, which suggests the favourable end, but this must be measured, not asserted: the follow-on benchmark in Next steps runs representative guests metered and unmetered and records the ratio. Note the asymmetry that makes the tax acceptable: the alternative bounds (wall-clock, yields) are either non-deterministic or unenforceable, so the question is not whether to pay for metering but whether the measured rate is tolerable per world; worlds that can afford it may raise budgets, never disable the bound.

## Q2 — Per-function timeout: closing a function that never responds

**Answer: fuel is the primary timeout for compute; epoch interruption is the wall-clock backstop for everything fuel cannot see.** Two different failure shapes need two different closers:

1. **The function spins in guest code.** Fuel already closes this, deterministically: budget spent, trap, done. For the tick mapping this is the real bound — a function gets a certain amount of *work*, and the work running out is what closes it.
2. **The function never yields in wall-clock terms despite a generous fuel budget** (huge budget by world policy, or time spent where fuel does not tick at the right rate). The mechanism is **wasmtime epoch interruption**: `config.epoch_interruption(true)`, a per-Store epoch deadline (`store.set_epoch_deadline(n)`), and one host ticker thread calling `engine.increment_epoch()` on a fixed cadence (e.g. every 1-10 ms). When the deadline passes, the next epoch check in guest code traps the call. Overhead when not triggered is a load-and-compare at function entries and loop back-edges — near the bottom of measurable, well under the fuel tax. This is the correct implementation of the mapping design §3 **timeout row**: Drew's "a certain amount of time; if it does not respond in that time, it closes" — with the C4 caveat the design already records: the timeout row is a cooperative, single-host backstop. It is never the fairness mechanism and never appears in cross-host adjudication, because two hosts will fire it at different points.
3. **The function is blocked inside a host call.** Neither fuel nor epoch checks run while the host is executing a GuestCall on the guest's behalf. Host calls therefore need their own deadlines at the capability layer (each host operation bounded, its overrun reported as the crash/timeout row for the binding). This is a real gap to close in the implementation follow-on, and it is invisible to both wasm-layer mechanisms — worth stating plainly because it is the one place "the function never responds" can still hang a tick.

**Alternatives considered:** running each function in its own OS process and killing it (the `world_compile.rs` subprocess pattern) is the heavyweight closer — real isolation, but process spawn/teardown per invocation destroys the near-C-speed per-tick story and is unnecessary for wasm guests, which cannot escape the Store; keep it as the pattern for *native* tooling only. A watchdog that abandons (rather than traps) a call leaks the Store mid-execution and forfeits the deterministic outcome row; epoch trapping is strictly better because the trap is an ordinary, recordable outcome.

**Recovery semantics:** a fuel or epoch trap is not corruption. The Store is discarded or reset, the binding keeps its pre-tick state (mapping §3), a strike is recorded, and the tick proceeds with the next binding. Nothing about a trapped function can stall the mapping past its own budget + deadline.

## Q3 — Multithreading: what was decided, what is open

**The short state: in-guest threads for user functions are neither available in Roc today nor wanted in this design; parallelism lives at the host, across bindings.**

- **Roc side (ecosystem state).** Roc's execution model is single-threaded: a Roc program, and the platform it runs on, executes on one thread; the language provides no thread spawn/join primitives and its runtime and reference-counting model are built on that. The Roc community discussions Drew recalls did happen (community chat and issue trackers, recurring over several years) and their consistent landing spot is the one reflected in Roc's design: parallelism is expressed by the *platform* (the host decides how work is spread) or by running multiple programs, not by threads inside a Roc application. There is no shipped or scheduled in-language threading model for Roc user code to target. Any design of ours that requires a user function to spawn threads is designed against a runtime that does not exist.
- **Wasm side.** The wasm threads proposal (shared linear memory + atomics) exists and wasmtime can execute such guests, but a guest must be *built* for it (atomics and bulk-memory features, a threading-aware libc/platform layer) — Roc guests are not. Bolting it on would also break this program's core property: shared-memory concurrency inside a step makes execution non-deterministic (schedule-dependent races), and ticks must replay identically for adjudication (mapping design §7 / F8). In-guest threads are therefore rejected on determinism grounds even if the toolchain offered them.
- **Wasmtime concurrency fact that shapes the host design:** a Store is single-threaded state — one Store executes on one thread at a time and is not shared across threads. Parallel execution of user functions therefore means **parallel Stores**: distinct bindings invoked concurrently on a host thread pool, one Store (or fuel-armed shared instance) per in-flight invocation.
- **What this means for many user functions per tick.** The mapping already fixes a deterministic invocation order and collects exactly one outcome per binding (mapping §2). Host-side parallel invocation is compatible with that *only if* outcomes are collected and committed in mapping order regardless of completion order — the parallelism is then an unobservable scheduling optimisation: same inputs, same outcomes, same tick log, faster wall-clock. Worlds that need strict replay run the same mapping sequentially; the semantics do not change. Per-binding fuel (Q1) is what makes parallel invocation safe to offer: no binding can starve another of anything but pool slots, and its budget is spent identically whether it ran first, last, alone, or in parallel.

## Recommendations (summary)

1. **Gas:** per-binding wasmtime fuel, armed per step invocation on the tick path, debited per binding and logged per tick; per-GuestCall prices layered on top for host work. (Q1)
2. **Timeout:** fuel is the compute bound; add epoch interruption with a host ticker as the single-host wall-clock backstop implementing the §3 timeout row; add per-capability deadlines for host calls, the one blind spot. (Q2)
3. **Multithreading:** no in-guest threads, now or planned; scale across bindings with parallel Stores and deterministic, mapping-order collection. Revisit only if Roc ships a threading model *and* a deterministic subset of it exists. (Q3)

## Next steps (implementation follow-ons, out of scope for this research)

1. Wire per-binding fuel arming/accounting into the world tick path in `world_session/` (today fuel is per-world-instance in `sandbox.rs`), with tick-log debit records per binding.
2. Add epoch interruption (engine config + ticker + per-binding deadline) and per-capability host-call deadlines; map both traps onto the mapping §3 outcome rows and strike counter.
3. Benchmark the fuel tax on representative plaza guests, metered vs unmetered, and record the ratio against this doc — the performance-tension number the program is owed.
4. If parallel binding invocation is wanted, prototype parallel Stores with mapping-order collection and prove tick-log equality against the sequential run on the same inputs.

## Reconciliation with S40-S42 and the sibling designs

- S40 (per-function gas) → Q1: answered affirmatively with native fuel; the #5967 mapping §6 budgets are implementable as specified, and its F10 remainder is discharged by the per-binding debit + tick log of recommendation 1.
- S41 (per-function timeout) → Q2: Drew's time-based close is the epoch backstop; the design tension the brief did not yet know about — time is not portable across hosts — is resolved exactly as C4 already ruled: fuel is the portable bound, time is a local backstop only.
- S42 (multithreading) → Q3: the discussions Drew half-remembered ended at platform-level parallelism; this program follows that landing spot rather than relitigating it.
- Nothing here re-opens a sibling decision: #5962 owns the world per-tick budget invariants this metering feeds; #5964 owns rebind mechanics (a rebound function is simply a new binding account); #5968 owns intent shapes for the outcomes the traps produce.
