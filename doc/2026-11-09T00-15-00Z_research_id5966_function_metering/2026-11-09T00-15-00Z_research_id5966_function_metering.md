# Research: per-function gas metering, timeouts, and multithreading for user functions

- **Date:** 2026-11-09
- **Status:** research; recommendations for review. No implementation is included.
- **Issue:** [#5966](https://github.com/developing-today/code/issues/5966), part of the worlds design program [#5955](https://github.com/developing-today/code/issues/5955) (statements S40–S42 of the 2026-11-04 brief).
- **Consumer:** the per-user function mapping design [#5967](https://github.com/developing-today/code/issues/5967) (landed, PR [#7625](https://github.com/developing-today/code/pull/7625)). That design owns the mapping, the failure table, and the fairness rule; it names #5966 as the owner of **the concrete cutoff mechanism behind its fuel-exhausted and timeout rows, and the fuel-cost table the per-function budget is spent against**. This document answers that seam and does not re-decide the mapping.
- **Related:** [#6001](https://github.com/developing-today/code/issues/6001) (two-level metering: burst bucket + per-tick ceiling) and [#6002](https://github.com/developing-today/code/issues/6002) (cooperative timeout contract) are the granular follow-ups filed from the prior-art pass; [#5964](https://github.com/developing-today/code/issues/5964) (hot-swap), [#5962](https://github.com/developing-today/code/issues/5962) (server invariants), [#5968](https://github.com/developing-today/code/issues/5968) (tick/intent).
- **Binding constraints from the issue (not re-argued here):** fuel-style budgets, not wall-clock, are the fairness mechanism for cross-host contests (C4, affirmed by Drew 2026-11-04 23:57 CST); wall-clock remains acceptable for single-host / cooperative execution.

## Executive summary

1. **Meter in Wasmtime fuel, per invocation, on a store per binding.** Fuel is the only mechanism that is deterministic, already enabled in this host, and cheap to attribute. It only becomes *per-function* if each user function is invoked in **its own store (ideally its own module instance)**: the store-level fuel counter cannot separate two functions that share one store and one module.
2. **Wall-clock timeouts are a backstop, not a bound.** The current guest has **no imports** and therefore cannot block in a host call; a non-yielding function is pure compute, and fuel already cuts it off. Epoch interruption becomes necessary only when (a) host imports/capabilities are added, or (b) we want a responsiveness cap for host-local runs. It is fast (single-digit percent) but non-deterministic, so it must never appear in adjudication.
3. **Do not attempt multithreaded user functions in v1.** Roc has no threads in the language, our sandbox disables everything needed for the Wasm threads proposal, and shared-memory concurrency is incompatible with the replayable-tick requirement. Parallelism belongs **across** bindings at the host level (one invocation per worker thread), not **inside** a function.
4. **Pin the engine for cross-host fairness.** Fuel counts are deterministic for a fixed Wasmtime version and configuration, but they are not portable across engine versions or between fuel-cost models. Cross-host contests must pin the Wasmtime version (and the cost table) or normalise consumption before comparing.

## 1. What the host enforces today (verified against origin/main e5a4fdcb)

| Fact | Evidence |
| --- | --- |
| Each world owns **one persistent Wasmtime instance**; the model lives inside the guest's linear memory. | `pkgs/id/src/sandbox.rs` module docs |
| Fuel is enabled on the engine (`config.consume_fuel(true)`), reset to the limit before every guest call, and read back after the call. | `pkgs/id/src/sandbox.rs:158`, `:240`, `:464`, `:417` (`fuel_spent`) |
| One `SandboxLimits.fuel` value applies to every invocation kind — default **10,000,000** fuel — alongside module (16 MiB), linear memory (16 MiB), and message (1 MiB) caps. | `pkgs/id/crates/id-core/src/world_limits.rs:188-207` |
| Per-call **statistics** exist per `GuestCall` kind (`init`, `update`, `view`, `layout`, `records`, `wants`, `snapshot`, `restore`): calls, total fuel, max, last. In memory only; never journaled. | `pkgs/id/src/sandbox.rs:86-100`, `:114-131` |
| The guest is import-free: no WASI, filesystem, sockets, clocks, or randomness; the export list is allowlisted. | `pkgs/id/src/sandbox.rs:160-200` |
| **No epoch interruption is configured anywhere** in the guest path. | no `epoch` in `pkgs/id/src/sandbox.rs`, `pkgs/id/src/world_session/`, `pkgs/id/crates/id-core/src/` |
| The native-worker path (compiler/worker spawn) is bounded by `RLIMIT_CPU` (120 s), `RLIMIT_AS`, `RLIMIT_FSIZE`, and a polled deadline; no per-call guest deadline. | `pkgs/id/src/world_compile.rs:209`, `:304-312`, `:684-686` |

Two consequences follow directly:

- **Metering today is per invocation of the world's guest, not per user function.** The world calls `plaza_update` once per tick; any user-function work inside that call shares one fuel account. Per-function budgets therefore require a structural decision: user functions must be invoked as **separate metered calls**, each with its own store and fuel limit, from the world's scheduling step.
- **A guest that never yields is already impossible to hang on** — it cannot call out, so it can only burn compute, and fuel stops it. The timeout question becomes urgent only when capabilities (host imports) arrive.

## 2. Question 1 — per-function gas metering

### 2.1 Options

| Mechanism | Determinism | Granularity | Overhead | Fit here |
| --- | --- | --- | --- | --- |
| **Wasmtime fuel** (`consume_fuel`) | Deterministic for a pinned engine version/config | Per store, per call; can be read before/after any call | Instrumented per basic block; measurable, commonly reported in the low tens of percent for compute-bound code | **Primary bound.** Already on; one store per binding gives exact attribution |
| **Instruction counting** (Ebb/`wasm-metering`-style rewrite, Wasmer metering middleware) | Deterministic; independent of engine version if the counter is ours | Per function via injected counter | Similar class of overhead; requires a module rewrite step and a cost model | Fallback if cross-version portability matters more than engine integration |
| **Host-side per-capability accounting** | Deterministic if every charge is a function of inputs (bytes, calls) | Per capability call | Near zero; does not cover pure compute | **Complement, not substitute.** Needed to charge host work (view materialisation, state size, intent resolution) that fuel cannot see |
| **Cooperative yield points** | Deterministic if the yield schedule is a function of fuel/count | Coarse, voluntary | Low | Useful for long *world-authored* steps and for host-call fairness; cannot bound a hostile function that ignores the convention |

### 2.2 What the evidence says

- Wasmtime documents fuel-based interruption as **"completely deterministic: the same program run with the same amount of fuel will always be interrupted at the same location"**, and explicitly notes the **downside is more overhead** than epoch interruption. It reports that epoch interruption is faster, "sometimes significant: in some measurements, up to 2-3x" ([Wasmtime: interrupting execution](https://docs.wasmtime.dev/examples-interrupting-wasm.html); [`Config` docs](https://docs.wasmtime.dev/api/wasmtime/struct.Config.html)).
- A Wasmtime issue measuring epoch interrupts with fuel *consumption* (but without fuel traps) reports **28–40% overhead** ([wasmtime#4109](https://github.com/bytecodealliance/wasmtime/issues/4109)). This is exactly our current configuration's shape if we add epochs on top of `consume_fuel` — the cost is real and should be measured in-repo, not assumed.
- These numbers are **directional and version-specific** (we pin `wasmtime = "=48.0.7"`, `pkgs/id/Cargo.toml:161`). They justify a benchmark gate (section 6), not a claim in a fairness rule.

### 2.3 Recommendation

1. **One store per binding invocation.** Each user-function invocation gets its own `Store`, its own fuel limit, and (strongly preferred) its own module instance. This is the only way "per-function gas" can be true: store-level fuel is a single counter. If user functions remain functions inside the world module, the world must instead pass an explicit budget into the call and trust the guest to debit it — i.e. no bound at all for a hostile guest, which defeats the purpose.
2. **Fuel limit per binding = the world's per-function budget** (the mapping design's §6). Set it before the call; read `fuel_before - store.get_fuel()` after; on `Trap::OutOfFuel` the invocation takes the *fuel-exhausted* row of the mapping's failure table and debits the whole budget.
3. **Debit and log actual consumption, not the cap.** The tick log needs actual fuel per binding for auditing and for the two-level metering design (#6001); the cap is only the cutoff.
4. **Add host-side charges in fuel-equivalent units** for work the guest causes but does not execute: view bytes delivered, state bytes returned, intent count/size, capability calls. Keep the unit table in one place (a `cost_table` constant) so the mapping's budgets and #6001's two levels spend against the same currency.
5. **Pin the engine version and cost table for adjudication.** Two hosts running different Wasmtime versions may charge different fuel for the same code. For a cross-host arena either (a) both hosts pin the same Wasmtime version, or (b) the adjudicator normalises by re-running the recorded program on the canonical version. Record the engine version in the tick log so a mismatch is detectable.
6. **Reject per-call `u64::MAX` escapes in metered paths.** `world_store.rs` already has a restore case that sets `wasm_fuel: Some(u64::MAX)` (`pkgs/id/src/world_store.rs:1521`); metered user-function calls must not inherit that pattern — the budget is always finite and never taken from untrusted input.

### 2.4 Cost-model caveat: fuel is not free even when it is deterministic

Fuel is charged per executed *operation* by the compiled code, so the same source-level algorithm can cost different amounts across rebuilds of the guest (optimisation changes), across Wasmtime versions, and between architectures if the codegen differs. That is acceptable for *bounding* (any finite cap is safe), but not for *comparing two contestants* unless both run the same artifact form on the same engine. This is the concrete reason the fairness rule says "fuel-style", not "fuel as measured on my laptop": the fair comparison is performed on a pinned adjudication configuration, and the log records enough to re-run it.

## 3. Question 2 — per-function timeouts

### 3.1 The two hard cases

The mapping design's failure table distinguishes **fuel exhaustion** (deterministic, portable) from **timeout** (wall-clock backstop, single-host only). They are different mechanisms with different guarantees:

| Case | What can happen | Bound |
| --- | --- | --- |
| Pure compute, no imports | Guest spins forever | **Fuel** (deterministic). Epoch is only a responsiveness bound |
| Host call blocks (future: capabilities, storage, network) | Guest waits inside the host | Fuel does **not** advance while the host is blocked; needs a host-side deadline or async cancellation |
| Native worker wedged | Process spins | `RLIMIT_CPU` + polled deadline + kill (already present for compilation) |

### 3.2 Mechanisms

- **Fuel cutoff** — deterministic, already implemented, and sufficient for today's import-free guest. It is the *only* mechanism that may appear in adjudication.
- **Epoch interruption** — `config.epoch_interruption(true)` plus `store.set_epoch_deadline(n)` and a ticker calling `engine.increment_epoch()`. Low overhead, wall-clock granularity (interrupts at the next check, not at an exact instant), non-deterministic across runs. Ideal as a *host-local responsiveness backstop* and for killing runaway code when a capability call is added.
- **Async / cooperative cancellation** — Wasmtime's async support (`call_async`) plus epochs lets the host apply `tokio::time::timeout` to a guest call and drop it; useful once host calls exist, and compatible with per-binding isolation.
- **Process isolation** — the native-worker path's model (rlimits + deadline + kill). Coarse and slow to start, but the only true containment for a native (non-Wasm) user program; keep it as the native fallback, not the per-tick path.
- **Cooperative yield points** — a *contract* (deadline in the view, checkpoint call) rather than an enforcement mechanism. It lets a well-behaved function return early with a partial result; it cannot bound a hostile one. Its design belongs to #6002.

### 3.3 Recommendation

1. **Fuel is the hard cutoff** for every metered invocation; the *timeout* row is produced only when a wall-clock backstop fires.
2. **Add epoch interruption now, as a host-local backstop with a generous deadline** (e.g. 2–5 s per invocation), enabled only for host-local execution. It costs little and protects against a future host-call stall. Keep it out of the adjudication path: record the row as `timeout`, never compare it across hosts.
3. **Do not build preemption for the import-free guest.** A pure-compute guest is fully bounded by fuel; adding OS-level preemption would spend complexity on a case that cannot occur today.
4. **When capabilities arrive, every host call gets its own deadline** and the capability layer reports a typed `Cancelled` result so the mapping's failure table stays complete without inventing a new row. The wall-clock budget for the whole invocation then becomes the min(remaining fuel time estimate, host-call deadlines, epoch deadline).
5. **The native path keeps its rlimits** and gains the same typed outcomes; it is not the per-tick execution path for user functions.

## 4. Question 3 — multithreading for user functions

### 4.1 Where things actually stand

- **Roc has no threads in the language.** Roc's model is functional with platform-provided effects; concurrency is a platform concern, and its async-style syntax (`.await`-like task syntax) is not an OS-thread model. Roc's public material does not advertise a threading feature, and the compiler is still pre-0.1 ([roc-lang.org](https://www.roc-lang.org/), [roc-lang/roc](https://github.com/roc-lang/roc)). Any "Roc threads" design would be a platform/effect question, not something user functions can rely on today. (Confidence: high that there is no stable language-level threads feature as of this writing; the community discussion history should be re-checked when #5964 lands, since the adapter ABI is the real constraint.)
- **The Wasm threads proposal exists** (shared memory + atomics + `memory.atomic.*`), and Wasmtime can enable it. But this host deliberately disables multi-memory and memory64 and forbids imports; the guest ABI is single-threaded and the world model is one persistent instance per world (`pkgs/id/src/sandbox.rs`).
- **Our own design already depends on single-threaded determinism.** The mapping design fixes a deterministic order, one invocation per binding, and a replayable tick log (#5967 §2, §7). Shared-memory concurrency inside a tick would make the outcome depend on interleaving and destroy that property.

### 4.2 Recommendation

1. **v1: user functions are single-threaded.** One invocation, one store, one outcome. No `shared` memory, no atomics, no threads import in the ABI allowlist.
2. **Parallelism is a host scheduling property, not a guest feature.** Run independent binding invocations on a bounded worker pool (one per invocation, each with its own store), collect results, and commit in the fixed order. This preserves determinism: the *set* of results is order-independent because bindings share no state during the tick; only the commit order is fixed. The per-tick world budget must be divided across workers deterministically, or the parallelism must not change the fuel each binding receives.
3. **If throughput inside one function ever becomes the need**, the only compatible shape is a separate "arena/non-replayable" mode: Wasm threads enabled, determinism explicitly waived, and contests in that mode not adjudicated by replay. Do not mix modes in one match. This is a deliberate non-goal for v1.
4. **Revisit when the Roc adapter lands** (#5964): if the adapter's ABI assumes a single linear memory and a single-threaded host, that settles it for the near term; record the finding there rather than reopening the question per world.

## 5. Invariants implementation must hold

1. **One invocation, one store, one fuel account, one outcome.** No two bindings share a store; no binding borrows another's fuel.
2. **Fuel is the only adjudicated bound.** Wall-clock rows are host-local and never compared across hosts.
3. **Fuel is debited from actual consumption**, with the cap as cutoff; the log records both, plus the engine version.
4. **No unbounded fuel path.** Finite budget on every metered call; `u64::MAX` only in explicitly non-metered admin paths, never for user functions.
5. **Host work is charged in the same currency** (view bytes, state bytes, intent size, capability calls) via one cost table.
6. **The guest stays import-free and single-threaded** until capabilities and threads are designed together, not separately.
7. **A change to the engine version or cost table invalidates cross-host comparisons** and must be recorded as a new adjudication configuration.

## 6. Measurement plan (what would turn the cited numbers into ours)

The issue asks for measured or cited overheads. The citations above are directional; the in-repo measurement is small and should gate the implementation:

1. **Harness:** a benchmark guest (e.g. integer sieve, Fibonacci, a JSON-ish parse loop, and a representative Roc world tick) compiled once to Wasm.
2. **Runs:** same guest under (a) no metering, (b) `consume_fuel` only, (c) fuel + epoch. Report wall time median/p95 over ≥ 1,000 iterations, and fuel per run.
3. **Metric that matters:** fuel per *representative* invocation, and the spread across runs at fixed input (must be zero for fuel). Wall-clock overhead is secondary and noisy.
4. **Acceptance for the pilot:** fuel-only overhead within the team's agreed envelope on the representative tick; epoch backstop cost measured separately; both recorded in the doc that lands the implementation.
5. **Calibration:** publish the fuel cost of the standard view/state/intent operations so #6001's two-level metering can be sized without guessing.

## 7. Open questions

- **Where does the user-function store boundary sit?** One store per invocation (clean attribution, instantiation cost per tick) vs. one store per binding reused across ticks (cheaper, but state and fuel accounting must be reset carefully). The mapping design assumes a persistent binding; the invocation model needs a decision before implementation. Recommendation: reuse the store *per binding* across ticks, resetting fuel each invocation, and snapshot state outside the store. Instantiation cost is then paid at bind time, matching #5964's hot-swap.
- **Does the world itself run on the same engine/limits as the user functions?** If worlds and user functions share an engine, epoch settings and the pinned version are shared; that is simpler and should be the default.
- **How are host-side charges converted to fuel?** A fixed unit table is enough for v1; a measured table is better once the harness exists. This is the seam with #6001.
- **What is the timeout value's authority?** The world author declares it, but the server must cap it (a world cannot grant itself an unbounded wall-clock backstop). The cap is a server invariant (#5962).
- **Roc's threading discussion history:** the summary here is a state-of-the-language statement, not a citation-by-citation history; if the adapter redesign (#5964) touches concurrency, the community threads should be surveyed then. No decision here depends on them.

## 8. Next steps

1. Land this research as the answer to #5966's deliverable; link it from #6001, #6002, #5964, and the mapping design #5967.
2. Open an implementation ticket for **per-binding stores + fuel budget plumbing** (the structural prerequisite), referencing §2.3 and §5.
3. Open an implementation ticket for **epoch backstop + typed timeout outcome** (host-local only), referencing §3.3.
4. Run the benchmark harness (§6) and publish the numbers in the implementation PR before any fairness value is hard-coded.
5. Keep #6001 (two-level metering) and #6002 (cooperative timeout) as the owners of the budget shape and the cooperative contract; this document supplies the mechanism they sit on.

## Sources

- Code (origin/main e5a4fdcb): `pkgs/id/src/sandbox.rs` (one persistent instance per world; `consume_fuel(true)`; per-call fuel reset and `fuel_spent`; `CallMetrics`; import-free export allowlist); `pkgs/id/crates/id-core/src/world_limits.rs:188-207` (`SandboxLimits`, default fuel 10,000,000); `pkgs/id/src/world_compile.rs:209,304-312,684-686` (native worker deadline and rlimits); `pkgs/id/src/world_store.rs:1521` (restore fuel escape); `pkgs/id/Cargo.toml:161` (`wasmtime = "=48.0.7"`).
- Issues/PRs: [#5966](https://github.com/developing-today/code/issues/5966), [#5955](https://github.com/developing-today/code/issues/5955), [#5967](https://github.com/developing-today/code/issues/5967), [#7625](https://github.com/developing-today/code/pull/7625), [#6001](https://github.com/developing-today/code/issues/6001), [#6002](https://github.com/developing-today/code/issues/6002), [#5964](https://github.com/developing-today/code/issues/5964), [#5962](https://github.com/developing-today/code/issues/5962).
- Wasmtime: [Interrupting execution](https://docs.wasmtime.dev/examples-interrupting-wasm.html) (fuel determinism; epoch lower overhead; up to 2–3x difference in some measurements); [`Config`](https://docs.wasmtime.dev/api/wasmtime/struct.Config.html) (fuel vs epochs, blocking-host-call interaction); [wasmtime#4109](https://github.com/bytecodealliance/wasmtime/issues/4109) (28–40% overhead for epoch + fuel consumption without fuel traps).
- Roc: [roc-lang.org](https://www.roc-lang.org/), [roc-lang/roc](https://github.com/roc-lang/roc) (language status; no advertised thread feature).
