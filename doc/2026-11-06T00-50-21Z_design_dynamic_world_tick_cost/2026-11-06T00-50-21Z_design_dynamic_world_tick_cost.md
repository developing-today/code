# Dynamic-world per-tick cost model and bot-battle benchmark

- **Status:** Design proposal for [#5987](https://github.com/developing-today/code/issues/5987)
- **Date:** 2026-11-06
- **Parent:** [#5955](https://github.com/developing-today/code/issues/5955)
- **Benchmark programme:** [#5685](https://github.com/developing-today/code/issues/5685)

## Request and scope

Measure what the current `pkgs/id` world path can measure, define a mixed-strategy bot-battle workload, and keep its current implementation costs separate from the dynamic per-user function design that is still in progress. The goal is a usable cost model and a benchmark scenario that can later be plugged into the shared benchmark programme—not to claim performance for code that does not exist.

This design covers the current durable world input path, Wasm guest-call/fuel instrumentation, view generation and session delivery, plus the proposed N-user strategy loop. It does not decide the DLL ABI or hot-swap contract (#5964), per-function timeout/metering policy (#5966), or the complete per-user mapping design (#5967).

## Current-state cost model

For an accepted input in the current durable world path, the observable end-to-end cost is approximately:

```text
T_input_ack = T_actor_queue + T_guest_update + T_records
           + T_journal_encode/write/fsync + T_ack
```

When a session renders the resulting state, add a separate view/delivery path:

```text
T_session_update = T_input_ack + T_view_guest + T_screen_parse/diff
                 + T_frame_encode/send
```

These are operational decompositions, not claims that current telemetry isolates each term. The actor's accepted-input/update/commit path is in `src/world/actor.rs`; its participant-specific `View` command calls the current guest view in that same actor (`src/world/actor.rs`, `WorldCommand::View`). The `many_worlds` benchmark observes acknowledgement latency across the whole durable path; it does not split the actor, guest, records, or fsync terms. `session_fanout` observes broadcast delivery, not world view rendering or session framing.

The future dynamic-user model adds terms that current code does not implement:

```text
T_dynamic_tick(N) = T_static_world_tick
                  + Σ[i=1..N](T_dispatch_i + T_meter_i + T_user_fn_i)
                  + T_intent_merge + T_view(N) + T_commit
```

The per-user sum, intent merge, strategy change, and DLL load/swap terms remain **unknown** until the relevant design and implementation exist. They must not be filled with extrapolations from unrelated current benchmarks.

## Evidence and cost ranking

The committed results in `pkgs/id/benches/BASELINES.md` are full-size measurements from the documented reference laptop; `--quick` is a smoke profile and is never a baseline.

| Path / term                                                             | Evidence in current main                                                                                                                                                                                                                                                                                                                                                                                           | Interpretation                                                                                                                                                                                                                             |
| ----------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Durable input under many-world load                                     | `many_worlds`: 20,000 acknowledged guest inputs across 100 active durable worlds; the 2026-11-02 reference run reported p50 1,076.753 ms, p95 4,547.595 ms, p99 5,650.792 ms, 1,523.1 completed commands/s, against a 50 ms p99 target. Checkpoint/eviction/reopen state assertions passed.                                                                                                                        | This is an end-to-end, heavily queued durable-load result, **not** a normal single-world tick estimate. It shows this particular persistence/actor workload misses its latency target; it cannot attribute the delay to any one component. |
| Journal append + `sync_data`                                            | `journal_append_fsync`: median 5,180.353 µs per append in the 2026-10-28 reference run.                                                                                                                                                                                                                                                                                                                            | The append/fsync durability boundary is a material measured cost for durable mutations and can dominate a small in-process operation. It is not the cost of a dynamic function call.                                                       |
| Checkpoint compaction                                                   | `checkpoint_compaction`: median 10,971.985 µs per compaction in the same reference run.                                                                                                                                                                                                                                                                                                                            | Amortized maintenance cost; not paid as a separate compaction on every input.                                                                                                                                                              |
| Current Wasm call instruction budget                                    | `src/sandbox.rs` enables Wasmtime fuel consumption and records per-call-kind fuel totals/max/last values (lines 90–127, 157–158). In the checked-in territory full-house fixture, measured maxima are 831,551 fuel for update, 1,611,425 for view, 1,612,121 for layout, and 436,803 for records (`src/sandbox.rs`, lines 2609–2615).                                                                              | **Fuel is an instruction-budget unit, not elapsed time.** Within this fixture, view/layout use more fuel than update, and records use less; this is a useful work ranking but not a wall-clock latency ranking.                            |
| Guest replay host work                                                  | `guest_replay`: median 0.170 µs/event in the corrected reference run. The benchmark guest is an in-process `CountingProgram`, not the Wasm sandbox.                                                                                                                                                                                                                                                                | Host replay-path proxy only; do not compare it as a measured Wasm or per-user function cost.                                                                                                                                               |
| Session fan-out                                                         | `session_fanout`: median 1.437 µs per delivery in the reference run.                                                                                                                                                                                                                                                                                                                                               | Broadcast-channel delivery only. It excludes guest view generation, per-viewer projection, frame encoding, and real network transport.                                                                                                     |
| Wasm metering overhead in wall time                                     | Wasmtime fuel consumption is enabled, and per-call fuel is observable; there is no current benchmark that runs matched metered/unmetered code to isolate its time tax.                                                                                                                                                                                                                                             | **Unknown.** The configured/observed fuel values do not quantify the runtime overhead of fuel instrumentation.                                                                                                                             |
| Per-viewer view/projection time                                         | Actor `View` calls invoke the current program's `view` for the requested participant (`src/world/actor.rs`, lines 1417–1448); the full-house territory fuel test exercises the guest view. Session delivery, screen diff, and event projection are in `src/world_session/mod.rs`, but there is no current wall-time benchmark separating actor queueing, guest rendering, event projection, screen diff, and send. | Fuel offers a work-count signal for guest rendering; per-viewer latency and projection cost remain unmeasured.                                                                                                                             |
| Dynamic DLL boundary, per-user strategy dispatch/switch, cache hit/miss | The dynamic DLL and per-user mapping are separate open designs (#5964, #5967); current benchmarks contain no such workload.                                                                                                                                                                                                                                                                                        | **Not implemented; no defensible numeric estimate or relative ranking.** Measure after the interface exists.                                                                                                                               |

The evidence-backed ordering is therefore limited: for the cited durable workload, end-to-end queueing/persistence is the dominant observed latency; for the full-house territory guest, view/layout consume more Wasm fuel than update, then records. DLL-boundary, per-function wall-time metering overhead, and strategy/cache costs cannot yet be ranked against those values because the units and paths are not comparable—or do not exist.

## Bot-battle benchmark scenario

Add a `bot_battle` scenario to `world_bench` (or adapt it into the shared workload generator once #5891 settles) using the checked-in `examples/territory/territory.wasm` guest. Territory has an eight-seat, 16×8 board and produces useful grid/seat records. It is a better multi-user workload than tic-tac-toe for sustained load; tic-tac-toe can remain a short two-player acceptance smoke.

### Current-stack proxy profile

- Run one durable world with **N = 2, 4, and 8** guest participants, capped by territory's eight seats.
- Use deterministic scripted policies with distinct behavior: center-seeking; fixed-order frontier expansion; boundary defense; and seeded frontier choice, plus repeated policy instances to reach N. Seed any randomized policy explicitly and print the seed.
- For each round, derive one legal action per bot from the same pre-round records/view, then submit in stable participant order. Record stale/invalid/no-op actions rather than silently dropping them. Use the game’s `h/j/k/l` and space inputs; bound total rounds so the workload is reproducible.
- Keep world creation/module installation and warm-up outside the timed interval. Report them separately as cold-start costs if measured.
- Report per-input and per-round p50/p95/p99 acknowledgement latency, throughput, accepted/no-op counts, journal bytes, and final records. Verify the expected seat count and record state after the run.
- Measure direct `WorldHandle` input and view calls as one adapter. If measuring the session layer, use a separate adapter through `run_session_with`; report frame bytes and per-viewer view latency separately. Do not label direct actor calls as network/session performance.

This profile measures a multiplayer **world-action** workload on the current single guest. It does **not** simulate N separate user functions, per-user DLL switching, per-function budgets, or one atomic tick that merges N intents. Its name/output must label it `current_stack_proxy` until the per-user function interface is implemented.

### Future dynamic-function profile

When #5964/#5966/#5967 provide a real interface, add a second profile that evaluates N distinct user strategies against one shared pre-tick state, records one result per function (including timeout/fuel exhaustion/switch outcome), merges intents once, then commits one world tick. Report separately: DLL compile/load and cold swap; warm dispatch; per-function execution wall time and fuel; switch/cache hit/miss; merge time; view projection/render time per viewer; commit/fsync; and end-to-end tick latency. Vary N, strategy-switch frequency, and view-cache policy independently. This is the first profile that may be called a dynamic user-function bot battle.

The two profiles must never be combined into one headline number: their operation counts and semantics differ.

## Defaults and world-author levers

| Concern              | Current default / behavior                                                                                                                                                                          | World-author or operator lever                                                                                                                     | Design boundary                                                                                                                            |
| -------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| Guest execution      | One installed Wasm world program is compiled/instantiated through the existing sandbox; fuel consumption is enabled and guest calls are bounded.                                                    | Choose guest logic and per-call fuel profile within server limits.                                                                                 | A compiled per-user DLL pool and swap/reuse policy belong to #5964, not this cost-model change.                                            |
| Per-user strategies  | No current per-user strategy map or strategy-switch operation.                                                                                                                                      | None yet; design the map and switch trigger in #5967.                                                                                              | Do not claim that current world inputs model switching.                                                                                    |
| View generation      | `WorldHandle::view` asks the actor/program for a view; current guest work is repeated on request. Session code coalesces redraws over its existing short window and supports screen delta delivery. | Choose view complexity and viewport/layout behavior; clients/sessions determine when to request or deliver views.                                  | No measured view cache policy or cache hit-rate exists. Do not add a result cache without invalidation/version semantics and measurements. |
| Tick batching/rate   | Current actor processes input events serially and persists accepted mutations; it does not schedule a multi-function atomic tick.                                                                   | A world can encode a larger action in an input and an external driver can choose cadence, but that is not equivalent to an atomic multi-user tick. | Establish tick boundary, ordering, and partial-failure behavior in #5967 before optimizing batching.                                       |
| Tick-rate adaptation | No automatic dynamic-function-aware rate adaptation is established by this benchmark.                                                                                                               | An operator may select a fixed workload cadence for measurement.                                                                                   | Adaptive scheduling requires measured budgets and fairness rules; defer until per-function costs are measurable.                           |

## Integration with the benchmark programme

- **#5685:** use the shared baseline, workload generator, result format, and regression-gate programme; this issue contributes one world workload, not another load-generator or result store.
- **#5891:** adapt the bot policies to its shared workload API after that design settles. Until then, keep the hand-rolled `world_bench` adapter deterministic and explicitly local.
- **#5894:** record hardware, OS, Rust toolchain, guest/module hash, workload profile, and config with full-size results. A quick run is smoke evidence, not a baseline.
- **#5896:** treat direct actor, in-memory session, and network/interface runs as different matrix rows. Keep workload and world configuration identical where the interface comparison allows it.
- **#5897:** use machine-readable output and the programme’s noise/waiver/re-baseline process. Existing `id-bench-compare.sh` reports regressions over 20% non-blockingly; do not invent a separate gate here. Gate the current proxy first; only gate the dynamic profile after stable interface semantics and same-class baselines exist.
- **#5964 / #5966 / #5967:** consume the DLL, gas/timeout, and per-user mapping decisions; report missing implementation terms as unknown rather than making this issue decide those contracts.

## Initial verification plan

Run `cargo bench --bench world_bench -- --quick` from `pkgs/id/` to smoke the current harness. The bot-battle profile is specified, not runnable yet; its implementation should adapt the workload to #5891's shared generator when that design settles. Record the exact command and pass/fail. Quick numbers are not baseline-quality. Full-size reference runs and the baseline update belong with the shared programme's machine/environment and regression-gate decisions.

## Change log

- 2026-11-06: Initial evidence-based cost model and benchmark design for #5987; separates existing durable/Wasm evidence from unimplemented dynamic-function costs.
