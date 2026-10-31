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
performance change, or harness change). `--quick` shrinks every
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
four benchmarks at full size on PRs touching `pkgs/id/**` and on
pushes to main, then diffs the medians against `baseline.txt` with
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
committed *chat* events, and `WorldCore::restore` applies committed chat
to core state directly — only committed *input* (and journal system)
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
