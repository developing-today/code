# F11.8 benchmark baselines — how to re-baseline

Issue #539, slice 2 (issue #1512). Slice 1 (PR #1387) added the
`world_bench` harness in this directory; this file documents the
committed baselines in `baseline.txt` and how to regenerate them.
Slice 3 adds non-blocking CI regression reporting that diffs future
runs against these medians (>20%).

## Reference machine

- Laptop (fleet reference machine): AMD Ryzen 7 7840U w/ Radeon 780M
  Graphics, 16 threads, 123 GB RAM
- OS: Linux 6.18.55, NixOS (SMP PREEMPT_DYNAMIC)
- Toolchain: rustc 1.98.1, cargo 1.98.0 (release / `bench` profile)
- Code state: taken on branch `queue/1559-1512-bench-baselines`,
  based on commit `279c9739` (main at the time of the run)

Numbers are only comparable on the same machine class — the fsync
benchmarks in particular measure this machine's storage as much as
the code. The laptop was under shared fleet load (load average ~8)
during the baseline runs; the two full runs below still agreed to
within ~5%, because the medians are dominated by fsync and in-process
work rather than scheduling.

## Baseline run (committed in `baseline.txt`, 2026-10-28)

```text
bench journal_append_fsync unit=us_per_append median=5180.353 min=5128.725 reps=5
bench checkpoint_compaction unit=us_per_compact median=10971.985 min=10683.191 reps=5
bench guest_replay unit=us_per_event median=0.170 min=0.145 reps=5
bench session_fanout unit=us_per_delivery median=1.437 min=1.383 reps=5
```

## Reproducibility check (independent second full run, same machine)

```text
bench journal_append_fsync unit=us_per_append median=5195.900 min=5149.424 reps=5
bench checkpoint_compaction unit=us_per_compact median=10804.895 min=10723.657 reps=5
bench guest_replay unit=us_per_event median=0.149 min=0.145 reps=5
bench session_fanout unit=us_per_delivery median=1.519 min=1.435 reps=5
```

## How to regenerate

From `pkgs/id/` on the reference machine:

```sh
just bench            # = cargo bench --bench world_bench
# or, offline against a warm cargo cache:
cargo bench --offline --bench world_bench
```

This runs the full-size workloads (10,000 events replay; 2,000-entry
journals; 32 fan-out clients x 100 rounds; 5 reps each) and prints one
`bench <name> unit=<unit> median=<value> min=<value> reps=<n>` line
per benchmark. To re-baseline: replace the four `bench` lines in
`baseline.txt`, update the machine/date header there and the runs
above, and say why in the commit message (hardware change, deliberate
performance change, or harness change). A harness change that
alters a benchmark's `unit=` is a re-baseline event too: until
`baseline.txt` is updated, `id-bench-compare.sh` reports that
benchmark as incomparable (unit changed) with no percent and no
verdict, rather than comparing medians across units (#1619).
`--quick` shrinks every
workload to a smoke size — never commit `--quick` numbers as
baselines.

## Harness fix included with these baselines

The full-size `session_fanout` workload as shipped in slice 1 could
never complete: it sent all 100 rounds from one participant, and the
per-participant chat rate limit (50 messages per 10 s window,
`CHAT_RATE_BURST` in `src/world/mod.rs`) refused round 51 with
`chat rate exceeded; try again later`. The benchmark now rotates
across four senders (25 rounds each), which stays under the burst
cap while measuring the same delivery path. No production code
changed.

## CI regression reporting (slice 3, #1513)

The `id bench` workflow (`.github/workflows/id-bench.yml`) runs the
full-size workloads on pushes to main and on manual dispatch, then diffs
the medians against `baseline.txt` with
`.github/scripts/id-bench-compare.sh`. Any benchmark more than 20%
slower than baseline is named, with its percentage, in the job
summary and the uploaded `bench-output` artifact.

The reporting is deliberately non-blocking: it is a separate workflow
from the required `nix flake check` gate, and the compare script
always exits 0. The threshold is adjustable via the
`BENCH_REGRESSION_THRESHOLD_PCT` environment variable in the compare
step. Remember the caveat above: baselines were taken on the laptop
above, CI runners are a different machine class — a flag is a prompt
to investigate (ideally by re-running on the reference machine), not
proof of a regression.

## Correction — guest_replay baseline (2026-10-30, #1403)

The slice-2 `guest_replay` line above (median=0.146) was measured with a
harness whose guest saw **zero** events: the journal was built from
committed _chat_ events, and `WorldCore::restore` applies committed chat
to core state directly — only committed _input_ (and journal system)
events are put in `RestoredWorld.replay`. The old number was therefore
restore-only time divided by N, mislabeled as guest replay.

The harness now builds the journal from committed input events and
asserts the counting guest saw exactly N events before recording a
sample; an empty or short replay fails the bench run. The assertion was
verified against the old workload first: with only the assertion added,
the chat-based journal fails with `guest replay saw 0 of 200 committed
events`. `WorldCore::restore` semantics are unchanged — no production
code was touched.

New line, full-size workload (10,000 events, 5 reps) on the reference
laptop above, 2026-10-30:

```text
bench guest_replay unit=us_per_event median=0.170 min=0.145 reps=5
```

A second full run the same evening agreed on the median exactly
(median=0.170, min=0.134). The other three baselines are unchanged.

## Many-worlds scale measurement (#557, F11.8 remainder)

The `many_worlds` workload creates 1,000 durable idle worlds plus 100
simultaneously leased active worlds, then submits 20 accepted counter inputs
per active world per second for 10 seconds (20,000 commands). It records
per-command p50/p95/p99 from scheduled submission through acknowledgement. The
benchmark also records wake lateness (scheduled deadline to the Tokio task
starting the input call) and input/ack duration (input call start to its
acknowledgement). These component distributions are diagnostic: their
percentiles must not be added to estimate an end-to-end percentile. The active
worlds use the checked-in `roc-counter` guest. After the timed load, one
extra input per active world triggers its due checkpoint; these trigger inputs
are excluded from latency percentiles. The benchmark then evicts all 100 active
worlds, verifies each journal has a recent checkpoint, reopens the worlds
through `WorldHub`, and asserts the acknowledged sequence and records match.
`--quick` is a smoke
configuration only; its output is never a baseline. Per-world event phases are
staggered within each 50 ms interval to avoid an artificial all-world burst.

Reference measurement (2026-11-02; rustc 1.99.0, cargo 1.99.0; AMD Ryzen 7
7840U, 16 threads, Linux 6.18.55, NixOS):

```text
bench many_worlds unit=ms_per_command median=1076.753 min=6.368 reps=1 p50_ms=1076.753 p95_ms=4547.595 p99_ms=5650.792 target_p99_ms=50 target_met=false idle_worlds=1000 active_worlds=100 events_per_second=20 duration_secs=10 commands=20000 load_elapsed_secs=13.132 completed_events_per_second=1523.1 checkpoint_triggers=100 checkpoints=100 evicted=100 reopened=100
```

The checkpoint/reopen assertions passed for all 100 active worlds: each had a
checkpoint, was evicted, and reopened with the same sequence and records. The
latency target did **not** pass: p99 was 5,650.792 ms versus the 50 ms target,
and end-to-end acknowledgements averaged 1,523.1/s versus the scheduled
2,000/s. This result is recorded rather than reported as acceptance; #557's
latency criterion remains open for follow-up. The matching measurement is
appended to `baseline.txt`; the four existing F11.8 baselines above are
unchanged.

Re-measurement (2026-11-14; rustc 1.99.0, cargo 1.99.0; AMD Ryzen 7
7840U, 16 threads, Linux 6.18.60, NixOS; the laptop was under shared
fleet load during the run), at `main` a824f622ae, after the #8316
restore fix (PR #9995) unblocked a clean run:

```text
bench many_worlds unit=ms_per_command median=1065.462 min=4.841 reps=1 p50_ms=1065.462 p95_ms=3602.663 p99_ms=4549.272 target_p99_ms=50 target_met=false idle_worlds=1000 active_worlds=100 events_per_second=20 duration_secs=10 commands=20000 load_elapsed_secs=12.784 completed_events_per_second=1564.5 checkpoint_triggers=100 checkpoints=100 evicted=100 reopened=100
```

The checkpoint/reopen assertions passed again for all 100 active
worlds (checkpoints=100, evicted=100, reopened=100). The latency
target still did **not** pass: p99 was 4,549.272 ms versus the 50 ms
target — an improvement of 1,101.520 ms (−19.5%) against the
2026-11-02 reference above, with completed throughput up slightly
(1,564.5/s versus 1,523.1/s) — but queueing behind per-append
fdatasync still dominates, exactly as the #7064 profiling recorded
(journal_append_fsync this run: median 5,443.164 us, in line with the
5,180 us reference). The remaining gap is the group-commit work,
split to #8239 (journal `append_batch` + actor-side drain), which is
the next phase; #557 stays open until a recorded run shows
target_met=true. The matching measurement is appended to
`baseline.txt` under its own dated comment; the 2026-11-02 line is
unchanged.

## #557 p99 profile and fix campaign (2026-11-22, Muse sandbox)

Machine class: `sandbox` — a Muse sandbox VM, 4 visible CPUs under a
~1.5-core cgroup quota, ~6 GiB RAM, btrfs on a virtual block device,
shared with other build load. The harness writes world roots under
`TMPDIR`; these runs set `TMPDIR` to a directory on the btrfs home
volume because this sandbox's `/tmp` is a 512 MiB tmpfs where fsync
is free and the numbers would be meaningless. Sandbox numbers are
this machine's own and are not comparable to the reference laptop's
5,650.792 ms — what transfers is the cost breakdown and the
before/after chain, all measured on this machine with the same
profiler and configuration. `baseline.txt` / `baseline.jsonl` are
unchanged: they carry the reference machine class.

The profiler is the harness itself (commit `3e2607ac1`):
`ManyWorldsConfig::full()` accepts `ID_BENCH_MW_IDLE_WORLDS`,
`ID_BENCH_MW_ACTIVE_WORLDS`, `ID_BENCH_MW_EVENTS_PER_SECOND`, and
`ID_BENCH_MW_DURATION_SECS` (defaults are the committed full size),
and every `many_worlds` run now prints a `profile many_worlds ...`
line aggregating the active worlds' actor metrics — journal-append
and command latency histograms merged across worlds (exact means,
bucket-upper-bound percentiles), deepest actor queue depth, and
checkpoints completed during the load. Producing command:

```sh
cd pkgs/id
cargo bench --bench world_bench --no-run
TMPDIR=<btrfs dir> ./target/release/deps/world_bench-* --only many_worlds
```

Baseline (origin/main `0e82202a3` plus the profiler commit), full
configuration — 1,000 idle / 100 active, 20 events/s, 10 s,
20,000 commands:

```text
profile many_worlds journal_appends=20200 journal_append_mean_us=46457 journal_append_p50_us=50000 journal_append_p95_us=250000 journal_append_p99_us=250000 commands=20300 command_mean_us=46304 command_p50_us=50000 command_p95_us=250000 command_p99_us=250000 queue_depth_max=0 checkpoints_during_load=0
bench many_worlds unit=ms_per_command median=109337.796 min=22.150 reps=1 p50_ms=109337.796 p95_ms=211954.226 p99_ms=220672.952 target_p99_ms=50 target_met=false idle_worlds=1000 active_worlds=100 events_per_second=20 duration_secs=10 commands=20000 load_elapsed_secs=232.518 completed_events_per_second=86.0 checkpoint_triggers=100 checkpoints=100 evicted=100 reopened=100
```

Top three costs, as shares of the 220,672.952 ms p99:

1. **Tokio worker parking on the blocking journal rendezvous —
   ~99.9% of p99 (the backlog multiplier).** `JournalWriter::append`
   waited on a std mpsc rendezvous while the writer thread wrote and
   fsynced; world actors are `tokio::spawn` tasks, so each append
   parked one shared runtime worker for the whole sync. With W
   workers the hub caps at W appends per sync latency: measured
   throughput 86.0/s against the ceiling 4 / 46.5 ms ≈ 87/s, while
   the offered load is 2,000/s. Per-command deadline lateness is
   therefore almost entirely backlog behind that ceiling — in-actor
   service time is only ~46 ms of the 220,673 ms p99.
2. **Per-command `sync_data` on the journal writer thread — 46.5 ms
   mean at load, ~100% of command service time** (command mean
   46.3 ms vs journal-append mean 46.5 ms; the guest update,
   `wants()`, serialization, and channel hops are inside the noise).
   It is the S in cost 1's ceiling and the floor under service time:
   a lone append+sync costs ~42 ms on this storage even at 4 active
   worlds (quick profile: `journal_append_mean_us=41713`).
3. **Group commit never engages — appends per sync stays 1.0.**
   `queue_depth_max=0`: at every actor dequeue the queue was empty,
   so the #8239 actor-side drain (`handle_input_batch` /
   `append_batch`) never has a batch to coalesce. With one
   sequential producer per world the next input cannot exist until
   the current one is acknowledged, so no amortization of cost 2 is
   possible from inside one world; coalescing would have to happen
   across worlds, at the device.

Fix 1 (commit `e55fe23a1`) — cost 1: `recv_writer_reply` runs the
rendezvous wait under `tokio::task::block_in_place` on multi-thread
runtimes, so the runtime replaces the parked worker; everywhere
else the wait is direct. No durability or ordering change. Same
profiler, same configuration:

```text
profile many_worlds journal_appends=20200 journal_append_mean_us=50473 journal_append_p50_us=250000 journal_append_p95_us=250000 journal_append_p99_us=250000 commands=20300 command_mean_us=50268 command_p50_us=50000 command_p95_us=250000 command_p99_us=250000 queue_depth_max=0 checkpoints_during_load=0
bench many_worlds unit=ms_per_command median=544.287 min=10.655 reps=1 p50_ms=544.287 p95_ms=768.054 p99_ms=842.887 target_p99_ms=50 target_met=false idle_worlds=1000 active_worlds=100 events_per_second=20 duration_secs=10 commands=20000 load_elapsed_secs=10.891 completed_events_per_second=1836.3 checkpoint_triggers=100 checkpoints=100 evicted=100 reopened=100
```

p99 220,672.952 → 842.887 ms; throughput 86.0 → 1,836.3/s. The
ceiling is gone. Tests on this commit: `cargo test -p id-worlds
--lib` — 830 passed, 39 failed; every failure is an iroh `*_net`
connect timeout in this sandbox and fails identically on unmodified
origin/main (spot-checked 3/3); the journal, actor, hub, and
session suites pass.

Fix 2 (evaluated, rejected in this venue) — costs 2/3: cross-world
group commit, one `syncfs` per coalesced batch (2 ms window, 512
cap) from a per-filesystem coordinator in `world_store.rs`, with
per-append `sync_data` fallback. At quick size the journal-append
mean regressed 41.7 ms → 147.4 ms and the run failed in the
checkpoint phase (`world is unavailable`): on this sandbox a
`syncfs` flushes the entire shared filesystem — build artifacts
included — so the shared flush costs more than the per-file syncs
it replaces. The experiment was rejected on the quick measurement
without a full run and is not committed. It also identifies the
production caveat: `syncfs` cost scales with total filesystem
dirt, not journal dirt, so this design only pays on a filesystem
dedicated to world data.

Residual: p99 = 842.887 ms in this venue, target not met here. The
remaining latency is cost 2's tail at ~92% of the service ceiling:
append mean 50.5 ms against a 50 ms per-world tick puts the system
at saturation, where p99 is set by service-time variance, and no
clean code change lowers the mean — the venue's lone fdatasync is
~42 ms, already most of the 50 ms budget before any queueing. The
next step is therefore a measurement on hardware whose sync
latency leaves headroom, not another sandbox iteration: on the
reference laptop (lone append 5.2 ms) fix 1 alone moves the worker
ceiling from ~3,077/s to the device limit and per-world service to
the ~6 ms class, which is the regime where a sub-50 ms p99 is
decidable. #557 stays open until a recorded run — on any machine
class — shows `target_met=true`.
Latency-profile measurement (2026-11-22; rustc 1.99.0, cargo 1.99.0; AMD
Ryzen 7 7840U, 16 threads, Linux 6.18.60, NixOS; shared fleet build
activity, build-lock slot 1/2). Command, from `pkgs/id/`:

```sh
~/tools/fleet-build-lock cargo bench --bench world_bench -- --only many_worlds
```

```text
bench many_worlds unit=ms_per_command median=885.541 min=5.341 reps=1 p50_ms=885.541 p95_ms=4760.759 p99_ms=6142.516 wake_p50_ms=829.994 wake_p95_ms=4710.759 wake_p99_ms=6092.517 input_ack_p50_ms=10.283 input_ack_p95_ms=194.474 input_ack_p99_ms=379.032 target_p99_ms=50 target_met=false idle_worlds=1000 active_worlds=100 events_per_second=20 duration_secs=10 commands=20000 load_elapsed_secs=12.941 completed_events_per_second=1545.5 checkpoint_triggers=100 checkpoints=100 evicted=100 reopened=100
```

All 20,000 scheduled commands completed and the 100 checkpoint, eviction,
and reopen assertions passed. The measured end-to-end p99 was 6,142.516 ms;
wake-lateness p99 was 6,092.517 ms, while input/ack p99 was 379.032 ms.
This run therefore locates most of its observed tail before the input call
begins, in late task wake-up. Since the laptop was under shared build load,
this is a diagnostic profile rather than a clean comparison against the
2026-11-14 baseline; it does not establish the unloaded scheduler behavior.
The component p99s are not additive. The latency target remains unmet and
#557 stays open for follow-up. The matching line is appended to `baseline.txt`.

Post-fix measurement (2026-11-22; same laptop, same command, main
`44bdc15460` including the #12499 worker-parking fix, commit
`e55fe23a1`; checkout `/home/user/code-557-run` on the laptop, log
`~/bench557.log` there). The laptop was under heavy shared load for
the whole run (load average ~12–19; a concurrent workspace
check+test gate job held build-lock slot 2/2), comparable to or
heavier than the pre-fix entry above:

```text
bench many_worlds unit=ms_per_command median=20.125 min=7.513 reps=1 p50_ms=20.125 p95_ms=344.831 p99_ms=426.224 wake_p50_ms=1.113 wake_p95_ms=294.831 wake_p99_ms=376.226 input_ack_p50_ms=18.892 input_ack_p95_ms=77.141 input_ack_p99_ms=115.335 target_p99_ms=50 target_met=false idle_worlds=1000 active_worlds=100 events_per_second=20 duration_secs=10 commands=20000 load_elapsed_secs=10.015 completed_events_per_second=1997.0 checkpoint_triggers=100 checkpoints=100 evicted=100 reopened=100
```

Against the pre-fix entry: p50 885.541 → 20.125 ms, p99 6,142.516 →
426.224 ms, wake_p99 6,092.517 → 376.226 ms, completed 1,545.5 →
1,997.0/s of 2,000 offered (no backlog; load elapsed 10.015 s for a
10 s load). The fix removed the cost the profile named: the
wake-lateness tail collapsed ~16×. The 50 ms target is still not
met (`target_met=false`): the residual p99 is still wake-dominated
(376 of 426 ms) with input_ack_p99 at 115.335 ms, plausibly
load-inflated but not proven so by a single loaded run. The clean
next data point is an unloaded re-run of the same command; #557
stays open for it. The matching line is appended to `baseline.txt`.

## Fuel-spin isolation measurement (#1067, F12.10 exit criterion)

The `fuel_spin_isolation` workload measures one victim counter world
accepting inputs at 100 commands/s in two 3-second phases (300 scheduled
inputs each): first calm, then while hostile Wasm worlds spin. Each
hostile world runs a guest whose update loops until its fuel budget
(20,000,000 per spin) is exhausted and traps; a trapping call poisons
the instance, so every spin runs in a freshly instantiated world, and
the harness asserts that every spin ended in fuel exhaustion. Samples
are deadline-to-acknowledgement latencies, so a late acknowledgement
stays in the sample. `--quick` is a smoke configuration only (750 ms
phases, at most 4 spinners); its output is never a baseline - it passed
on 2026-11-10 with 75/75 samples in both phases and 556 exhausted
spins.

Reference measurement (2026-11-10; rustc 1.99.0, cargo 1.99.0; AMD
Ryzen 7 7840U, 16 threads, Linux 6.18.60, NixOS; the laptop was under
shared fleet load during the run):

```text
bench fuel_spin_isolation unit=ms_per_command median=2.084 min=0.428 reps=1 victim_p99_ms_calm=3.159 victim_p99_ms_spinning=7.952 ratio=2.517 target_ratio=1.10 target_met=false calm_samples=300 spinning_samples=300 spins=1821 spinners=15 phase_ms=3000 commands_per_second=100 spin_fuel=20000000 calm_elapsed_secs=3.041 spinning_elapsed_secs=3.116
```

The F12.10 target did **not** pass: with 15 spinners saturating the
remaining worker threads, the victim's p99 rose from 3.159 ms calm to
7.952 ms spinning - a ratio of 2.517 against `target_ratio=1.10`, with
1,821 fuel-exhausted spins observed during the phase and all 300
scheduled victim inputs acknowledged in each phase. This records the
gap the fair-scheduling work in #1067 has to close; the workload
implements no scheduler, quota, or back-pressure, and this result is
recorded rather than reported as acceptance. The matching measurement
is appended to `baseline.txt`; the existing baselines above are
unchanged.

## Fuel-spin isolation profile (#1067, F12.10 slice 1)

The measurement above recorded the gap but not where the spinning
phase's added latency sits. The workload now attributes each phase
(the #1067 slice-1 instrumentation): the victim driver records the
same sample split as `many_worlds` — deadline-to-acknowledgement,
driver wake lateness, input acknowledgement wait — and the bench
differences the victim actor's own journal-append and command
latency histograms across each phase from its metrics snapshots.
Producing command (laptop, reference machine; the bench binary was
built through `~/tools/fleet-build-lock cargo bench --bench
world_bench --no-run` at main `63f9558b9` plus the instrumentation):

```text
./target/release/deps/world_bench-<hash> --only fuel_spin_isolation
```

Fresh gap measurement at unmodified main `63f9558b9` first (same
command, laptop under heavy shared fleet load, load average ~17–30):
victim p99 calm 2.239 ms, spinning 38.168 ms, ratio 17.043,
`target_met=false`, 1,473 spins — the gap is load-sensitive and was
wider under this load than in the 2026-11-10 reference run (2.517);
no profile line exists at base. Two attributed runs with the
instrumentation (load averages ~20–24 and ~16–23):

```text
bench fuel_spin_isolation unit=ms_per_command median=2.992 min=0.222 reps=1 victim_p99_ms_calm=1.894 victim_p99_ms_spinning=21.924 ratio=11.575 target_ratio=1.10 target_met=false calm_samples=300 spinning_samples=300 spins=1059 spinners=15 phase_ms=3000 commands_per_second=100 spin_fuel=20000000 calm_elapsed_secs=3.041 spinning_elapsed_secs=3.139
profile fuel_spin_isolation calm_wake_p99_ms=1.795 calm_input_ack_p99_ms=0.201 spinning_wake_p99_ms=18.929 spinning_input_ack_p99_ms=5.086 calm_journal_appends=0 calm_journal_append_mean_us=0 calm_journal_append_p99_us=0 spinning_journal_appends=0 spinning_journal_append_mean_us=0 spinning_journal_append_p99_us=0 calm_command_mean_us=55 calm_command_p99_us=100 spinning_command_mean_us=130 spinning_command_p99_us=2000 victim_queue_depth_max=0 victim_checkpoints=0
bench fuel_spin_isolation unit=ms_per_command median=2.470 min=0.273 reps=1 victim_p99_ms_calm=1.904 victim_p99_ms_spinning=13.114 ratio=6.888 target_ratio=1.10 target_met=false calm_samples=300 spinning_samples=300 spins=1822 spinners=15 phase_ms=3000 commands_per_second=100 spin_fuel=20000000 calm_elapsed_secs=3.042 spinning_elapsed_secs=3.116
profile fuel_spin_isolation calm_wake_p99_ms=1.822 calm_input_ack_p99_ms=0.179 spinning_wake_p99_ms=13.092 spinning_input_ack_p99_ms=4.007 calm_journal_appends=0 calm_journal_append_mean_us=0 calm_journal_append_p99_us=0 spinning_journal_appends=0 spinning_journal_append_mean_us=0 spinning_journal_append_p99_us=0 calm_command_mean_us=48 calm_command_p99_us=500 spinning_command_mean_us=64 spinning_command_p99_us=500 victim_queue_depth_max=0 victim_checkpoints=0
```

Where the spinning phase's victim p99 goes (component percentiles
are diagnostic and do not add to the combined percentile, as with
`many_worlds`):

1. **Driver wake lateness — essentially the whole tail.** Spinning
   wake p99 was 18.929 ms of a 21.924 ms end-to-end p99 in run 1
   (~86%) and 13.092 ms of 13.114 ms in run 2 (~100%): the task
   submitting the victim's inputs cannot get a runtime worker to
   make the call. The 15 spinners run synchronous, fuel-limited
   guest calls on the shared multi-thread runtime, so every worker
   is occupied for the length of each burn.
2. **Input acknowledgement wait — second, same cause.** Spinning
   ack p99 rose from ~0.2 ms calm to 5.086 / 4.007 ms: the victim's
   actor task is itself a task on the same starved runtime, so it
   is polled late even though its work is nearly free.
3. **The victim's own service — negligible.** Its actor command
   latency averaged 55 µs calm vs 130 / 64 µs spinning (bucketed
   p99 at most 2,000 µs), its queue depth never exceeded 0, and it
   recorded no journal appends in either phase (the bench victim
   runs without a journal store), so none of the gap is journal or
   storage service.

This is the #557 disease in a second form: there, actor tasks
*parked* workers on a blocking journal wait (fixed by
`block_in_place`, #12499); here, guest execution *burns* workers
with CPU-bound synchronous calls, so the remedy is placement, not
unparking — guest calls must run where they cannot starve the
runtime (dedicated execution threads or a bounded blocking pool),
and bounding how many guests burn concurrently is the first piece
of the fair scheduler itself. Next slice, in the form the profile
supports: after that PR, the same command's profile line shows the
spinning wake p99 within ~2× of the calm wake p99 at the same load,
with the `many_worlds` numbers re-measured for regressions. The
first run's bench and profile lines are appended to `baseline.txt`;
the 2026-11-10 reference line is unchanged (the ratios above were
taken under heavier load and are not comparable to it).

## Fuel-spin isolation on the CI runner — noise-dominated, no verdict (2026-11-22, #1067)

Machine class: `ci-runner` (GitHub-hosted ubuntu-24.04 via the
fleet-run workflow; 4 visible CPUs, so the full profile runs with
3 spinners). Four runs of the exact full profile (3 s phases,
100 commands/s, 20,000,000 spin fuel), two on `main` (`c9fbfa0`) and
two on the #1067 blocking-context branch (`780ddbe`, guest calls
under `tokio::task::block_in_place`):

```text
bench fuel_spin_isolation unit=ms_per_command victim_p99_ms_calm=1.425 victim_p99_ms_spinning=3.562 ratio=2.500 spins=545 (main, run 45024947483)
bench fuel_spin_isolation unit=ms_per_command victim_p99_ms_calm=2.372 victim_p99_ms_spinning=5.147 ratio=2.170 spins=1591 (branch, run 45024947483)
bench fuel_spin_isolation unit=ms_per_command victim_p99_ms_calm=1.731 victim_p99_ms_spinning=6.693 ratio=3.867 spins=543 (branch, run 45027312468)
bench fuel_spin_isolation unit=ms_per_command victim_p99_ms_calm=1.439 victim_p99_ms_spinning=83.794 ratio=58.228 spins=525 (main, run 45027312468)
```

Identical code on this venue produced ratios of 2.500 and 58.228 —
the spinning-phase p99 over 300 samples on shared 4-vCPU runners is
dominated by neighbour noise, and the calm p99 varies by ~65%
between legs. This venue and profile therefore cannot decide
whether a change moves the ratio; a before/after pair has to run on
the reference machine class above (16 threads, 15 spinners), where
the 2.517 baseline was recorded. These lines are recorded so the
next #1067 worker does not spend runner time re-learning it; they
are not a baseline and nothing here is appended to `baseline.txt`.

## Benchmark matrix results bundle (#5896)

The four loadgen-core workloads (`journal_append_fsync`,
`checkpoint_compaction`, `guest_replay`, `session_fanout`) also emit
the per-interface benchmark matrix specified by the decision record
on #5896. Every run of `world_bench` that executes at least one of
them writes a `results.jsonl` bundle (path from `ID_BENCH_RESULTS`,
default `./results.jsonl` in the working directory):

- a **manifest record**: `schema_version`, `run_id`, `git_sha`
  (`ID_BENCH_GIT_SHA` / `GITHUB_SHA`), `started_at_utc`,
  `machine_class`, OS/arch, CPU threads, toolchain
  (`ID_BENCH_TOOLCHAIN`), build profile, loadgen generator version
  and seed;
- one **cell record** per planned (workload × interface ×
  configuration) tuple. Interfaces are `in_process` (the surface this
  harness measures) and the shell-contract transports `iroh`,
  `websocket`, `ssh-exec`; the configuration is #5894's
  `standard-default`, with the scale (`S1` for full runs, `smoke`
  for `--quick`) recorded as provenance. A cell with no adapter in
  this harness is `unsupported` with a reason — never omitted, never
  zero. A measured cell carries metric entries with metric,
  statistic, unit, value, sample count and the raw samples.

The comparison identity is `(machine_class, workload_id,
interface_id, configuration_id, metric, statistic)`.
`id-bench-compare.sh` compares two bundles by that identity (pass
`.jsonl` files instead of the legacy text files); it never compares
across machine classes, configurations, units, or workload/interface
IDs. The legacy `bench` lines and `baseline.txt` remain the
compatibility view for the `in_process` medians.

Machine classes are #5894's canonical pair: `fleet-laptop` (this
file's reference class; the default when `ID_BENCH_MACHINE_CLASS`
is unset) and `ci-runner` (relative-only; the `id bench` workflow
sets it explicitly). The workflow uploads `results.jsonl` with the
raw bench output in the `bench-output` artifact and links it from
the job summary, which is an index, not the record.

`baseline.jsonl` beside this file is the committed fleet-laptop
matrix baseline, produced by the same re-baseline procedure as
`baseline.txt`: a full-size run on the reference machine, committed
with its manifest (run id, commit, date) intact. `--quick` bundles
are smoke evidence only and are never committed as baselines. The
async workloads (`many_worlds`, `fuel_spin_isolation`) are not
matrix cells yet — their pacing is the loadgen record's later
executor steps — and keep their legacy lines above.

## Second-pass review: inline journal write (2026-11-22, sandbox)

A second pass over the event path audited the remaining seams (see
the PR for the full audit). One structural change: `FileJournal` no
longer hands each append to a dedicated writer thread over a channel
and rendezvous on its reply; it owns its `File` and writes inline
under the same `block_in_place` guard as the first fix. The writer
thread's queue was empty at every dequeue by construction — the
journal's only producer is its owning actor, rendezvous-blocked
until the in-flight append is durable — so the hop bought nothing,
and removing it also removes one thread per world. No durability or
ordering change.

Sandbox venue, TMPDIR on btrfs, full size (1,000 idle / 100 active,
20 ev/s, 10 s). The sandbox is shared and its load swung widely
between runs (recorded per run); latency percentiles swung ±2× with
ambient load while the mechanism metric (journal append mean)
stayed in one band. The change is **not measurable in this venue**:
append means overlap across runs, and no run shows a regression
attributable to it. `baseline.txt` is untouched (it carries the
reference machine class).

- Baseline (a968bf0f0), load avg 8.5 at start:
  `journal_append_mean_us=52713`, p50 580.966 ms, p99 1,455.158 ms,
  1,733.8 completed/s.
- Inline write, back-to-back after it, load avg 2.2 at start:
  `journal_append_mean_us=58407`, p50 1,270.688 ms, p99 2,735.997 ms,
  1,549.4 completed/s (a load burst during the load phase; the wake
  percentiles moved while `input_ack_p50` stayed flat at ~63 ms in
  both runs).
- Inline write, earlier run the same day at load avg ~1.4–4.6:
  `journal_append_mean_us=47713`, p50 109.730 ms, p99 1,003.589 ms,
  1,825.6 completed/s.
- Baseline under the day's heaviest contention (load avg ~11–14):
  `journal_append_mean_us=236445`, p99 37,764.450 ms, 411.8
  completed/s — recorded to show the venue's noise band, not as a
  comparison point.

All runs: `queue_depth_max=0`, checkpoints/evicted/reopened
100/100/100.
