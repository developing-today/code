# Phase 11: foundations for the next stage

See the [architecture record](../../doc/2026-10-17T15-00-00Z_architecture_next_stage_platform/2026-10-17T15-00-00Z_architecture_next_stage_platform.md) and the follow-on [Phase 12 plan](phase-12-platform-flexibility.md).

## Goal

Phase 9 and 10 added many capabilities quickly. Phase 11 makes them trustworthy, measurable, and easy to change before more is stacked on top. Nothing here adds a user-visible feature for its own sake. Each task either removes a known blocker, makes a cost visible, or removes a structural obstacle that Phase 12 needs.

## Evidence this plan starts from

| Observation | Source |
| --- | --- |
| The Nix check has never passed in recorded history (392 runs since 2026-03-25, none successful). | `id` workflow history |
| The disk filled on the runner (ENOSPC, then a linker bus error). Fixed by line-table debuginfo and no incremental cache in `mkCheck`. | CI runs 39023225921, 39023472983 |
| `relay_ops` reads the relay kit from outside the Nix source, which fails `id-test`, `id-test-web`, and the `id` package build (its `doCheck` runs `cargo test`). The NixOS VM tests then cannot build. | Issue #238, run 39032425456 |
| Firefox E2E: 12 failed, 2 flaky of 130 in the only run that reached a summary. One shared server and one browser serve all tests. | Issue #229 |
| The Rust unit suite takes about 480 seconds in CI. A full run with `--keep-going` takes over an hour. | CI logs |
| The bulk LogSync test is retried over a mid-stream reset that nobody has explained. | Issue #241 |
| Territory's styled view uses 78% of the 10M fuel limit with two seats, and each further seat adds about 0.8M, so six or more seats trap, and a trapped view poisons the world. Nothing reported it until F11.6. | PR #247 |
| `world.rs` is 6,120 lines (about 3,600 before its first test module). `world_session.rs` is 4,017 and `cli.rs` is 3,958. `cmd_world` has a 730 KB stack frame warning. | `wc -l`, clippy |
| Every example module is 0.5 to 0.7 MB. | `examples/*/*.wasm` |
| Joiners can read the install seed, chat, and nothing of host events, with no single place that decides visibility. | Issues #240, #242; roles record |
| Older binaries silently truncate an unreadable final journal line. | Issue #227, PR #244 |

## Workstreams

Tasks are numbered F11.n. Each names an exit criterion that a test or a CI job can check.

### A. Make CI trustworthy

This comes first, because every later task needs a signal it can believe.

- **F11.1 Relay kit location (#238).** Done on `main` by #249: the kit is vendored under `pkgs/id/tests/fixtures/relay`, the doc copy stays canonical, and a drift test fails whenever the doc copy is present and differs. Residual risk: drift is only caught in a full checkout, not in the Nix sandbox. Exit met: `id-test`, `id-test-web`, and the `id` package build no longer fail on the kit.
- **F11.2 Firefox E2E isolation (#229).** Reproduce locally per spec file, then give each spec file its own `id serve --ephemeral` server and port, and restart the browser per file. Exit: two consecutive green runs on one commit, and the E2E derivation under 10 minutes.
- **F11.3 Explain the LogSync flakes (#241).** `main` fixed the interrupted-session test (#254). The live failure on `main` is the bulk test hitting `Timeout(30s)`: it takes about 23.5s on an idle machine, so the shared 30s test deadline has no headroom, and #224 aligns it with the 60s production deadline. Still open: why a bulk transfer of that size takes 23s, and the connection reset seen only in `--keep-going` runs. Exit: both tests pass 50 consecutive runs under full-suite load without a retry, and the bulk test runs in under 10s.
- **F11.4 A fast lane.** Split CI into a lane that gives feedback in under 15 minutes (fmt, clippy, unit tests) and a full lane (integration, VM, E2E). Use `--keep-going` in both. Exit: median PR feedback for the fast lane under 15 minutes, measured over ten runs.
- **F11.5 Injected time.** Introduce a `Clock` that the actor and the scheduler read, with a manual clock for tests, so schedule and tick tests stop sleeping. Exit: no `sleep` in `world.rs` tests, and the schedule tests run in under one second.

### B. Make cost visible

- **F11.6 Per-call fuel metrics and budgets.** Record fuel used per guest call, add `id world measure`, and add a budget test for every checked-in example that fails when a call grows by more than 25 percent. Exit: the test exists and the headroom table is in the record. In progress on `platform-perf-budgets`.
- **F11.7 Host metrics and tracing.** Counters and histograms for actor queue depth, command latency, journal append and fsync latency, checkpoint duration, session count, and frame bytes. Expose them in `id world stats` and, when `--web` is on, as Prometheus text at an admin-token-protected path. Add `tracing` spans around each actor command. Exit: a test reads each metric after a scripted session, and the cost of metrics is under 2 percent in the benchmark below.
- **F11.8 Benchmarks.** A benchmark harness for guest replay of N events, journal append with fsync, checkpoint compaction, and session fan-out to K clients. Record baselines in the repository and run the harness in a non-blocking CI job that posts the numbers. Exit: baselines committed, and a regression of more than 20 percent is reported on the PR.
- **F11.9 Bytes per frame.** Measure full-screen frame sizes for each example at 80x24 and 200x60. This is the baseline that Phase 12 delta frames must beat. Exit: a table in the record.

### C. Make the code easy to change

- **F11.10 Split the large modules, with no behavior change.** `world.rs` into scopes and roles, journal entries and replay, the capability ledger and providers, the actor, and tests. `world_session.rs` into frames, session loop, and hub glue. `cli.rs` into one file per command group. Box large command variants so `cmd_world` stays under a stack budget. Exit: no file over 2,500 lines of production code, the full test suite passes unchanged, and `git diff --stat` shows moves only.
- **F11.11 One client projection.** A single function maps core events and snapshots to what a given scope may see, used by `events_after`, the snapshot, and the live stream. Decided by the project owner: the install seed is secret (done on `main` in #250 for `events_after` and the snapshot) and join-only participants do not see chat, with an explicit `history` scope to opt in to reading it (in progress on `fix-chat-visibility`, which also unifies the three paths and checks the live stream for a seed leak). Host events (#242) remain open. Exit: no code path serializes a `WorldEvent` to a client except through the projection, enforced by a test.
- **F11.12 Journal format management.** A version header, `id world journal check`, `id world journal backup`, and a writer that refuses to open a journal written by a newer format. This turns the rollback rule into tooling. Exit: a test opens a journal with a future version and gets a clear refusal, and `backup` produces a file that restores byte for byte.
- **F11.13 Error taxonomy.** Typed errors with stable codes and reasons on the wire, including the reason that `schedule.at` currently drops. Exit: every `CapError` carries a documented code, and the guide lists them.

### D. Make guests cheaper

- **F11.14 Cheaper `Screen`.** Replace the per-cell record list with row strings and style runs, so a full redraw costs less fuel. Exit: territory's view at or under 40 percent of the fuel limit, enforced by the F11.6 budget.
- **F11.15 Smaller modules.** Run `wasm-opt -Os` and strip in each `build.sh`. Exit: every checked-in module at or under 400 KB, with instantiate time measured.
- **F11.17 A view trap must not poison the world.** Today a guest view that traps (fuel, memory, or a guest bug) marks the program unhealthy for good. Degrade to an error screen that names the failing call, keep the program's state, retry on the next event, and count the failure in the F11.7 metrics. Exit: a guest that traps on `view` leaves the world accepting input, and the next healthy `view` recovers.
- **F11.18 Pinned toolchain for checked-in artifacts.** The checked-in territory module was built by an older system `roc` than the one pinned in `nix/roc.nix`, and that alone cost about 4.2M fuel per view. Rebuild every example with the pinned compiler, have each `build.sh` refuse a different one, and add a CI job that rebuilds the examples and checks behavior with the sandbox suite and the fuel budgets. Same-source builds are not byte-stable, so compare behavior, never hashes. Exit: the job fails when an example is built by another compiler.
- **F11.16 Replay self-check.** `id world verify` replays a world's journal twice, compares the records, and reports any divergence. This is the determinism guard for everything the guests do. Exit: a test with a deliberately nondeterministic guest fails the check.

## Sequencing

```text
F11.1 -> F11.2 -> F11.4          (CI trust, then speed)
F11.3, F11.5                     (parallel with the above)
F11.6 -> F11.18 -> F11.14 -> F11.15   (measure, pin the compiler, then optimise guests)
F11.7, F11.8 -> F11.9            (host metrics, benchmarks, baselines)
F11.10 -> F11.11 -> F11.12       (split, then project, then format)
F11.13, F11.16, F11.17           (anytime after F11.10; F11.17 can start earlier)
```

F11.10 should land before F11.12, because that task touches the code the split moves. F11.10 must wait until the open platform stack (#209 to #248) has merged: `world.rs`, `world_session.rs`, and `cli.rs` are edited by every one of those PRs, so a split now would conflict with all of them.

## Decisions

1. Relay kit (#238): settled on `main` by #249 with a vendored copy and a drift test, not a move. The move into `pkgs/id/deploy/relay/` was approved but is not needed now.
2. Seeds and chat (F11.11): seeds secret, join-only participants do not see chat, an explicit `history` scope opts in. Approved.
3. Firefox E2E (#229, F11.2): not yet decided. It remains the last red check once the sync tests are fixed.
4. SSH input modes: raw keystrokes by default, `cmd:` line mode as the explicit login-time entry, other variations later. Username grammar is `world-name` / `explore:` / `cmd:`. See the [decision record](../../doc/2026-10-24T20-18-37Z_decision_ssh_input_modes/2026-10-24T20-18-37Z_decision_ssh_input_modes.md) and the SSH command-grammar task (#546).

## Out of scope

New guest capabilities, new transports, and new roles. Those belong to [Phase 12](phase-12-platform-flexibility.md).

## References

- [Architecture record](../../doc/2026-10-17T15-00-00Z_architecture_next_stage_platform/2026-10-17T15-00-00Z_architecture_next_stage_platform.md)
- [Phase 12 plan](phase-12-platform-flexibility.md)
- [Phase 9 and 10 plan](phase-9-portable-platform.md)
