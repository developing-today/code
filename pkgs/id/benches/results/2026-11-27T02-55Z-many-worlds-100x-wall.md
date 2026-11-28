# 100x many-worlds attempt — session admission wall

- **Attempted:** 2026-11-27 02:55–03:21 UTC
- **Target source:** `developing-today/code` `main`, `bc89926066a8064a3dcad7ac88635d3a23a94dde` (the run log's `Target HEAD`; the Fleet task's initial SHA note was stale and is superseded)
- **Fleet run:** [45931139557](https://github.com/developing-today/fleet/actions/runs/45931139557), `fleet-org`, terminal failed; target job ran 22m26s
- **Actual runner class:** GitHub-hosted Actions `ubuntu-24.04`, runner `GitHub Actions 1000348767`. This is the standard CI class, not #6304's server class. The prior 10x record identifies this class as 4 CPUs / AMD EPYC 7763 / 15 GB RAM; this run log does not include a per-run CPU model or storage-device report. NVMe-class storage is therefore unverified. Fleet's configured Blacksmith fallback is `blacksmith-4vcpu-ubuntu-2404`; it also cannot meet the design's ≥16-thread requirement. No configured Fleet runner qualified as server-class for this attempt.
- **Toolchain:** Rust 1.99.1 (`pkgs/id/rust-toolchain.toml` at the target SHA); optimized Cargo `bench` profile. Compilation succeeded.
- **Workload:** 100,000 idle worlds; 10,000 active worlds; 20 events/s/world; 10 seconds; hub open-world ceiling 11,000; `many_worlds` only.
- **Command:**

```sh
cd pkgs/id && ID_BENCH_MW_IDLE_WORLDS=100000 ID_BENCH_MW_ACTIVE_WORLDS=10000 ID_BENCH_MW_EVENTS_PER_SECOND=20 ID_BENCH_MW_DURATION_SECS=10 ID_BENCH_MW_MAX_OPEN_WORLDS=11000 cargo bench --bench world_bench -- --only many_worlds
```

- **Observed output:**

```text
Finished `bench` profile [optimized] target(s) in 11m 00s
Running benches/world_bench.rs (.../world_bench-a295d6ca13c578d8)
Error: server is busy; try again shortly
error: bench failed, to rerun pass `-p id --bench world_bench`
process didn't exit successfully: `.../world_bench-a295d6ca13c578d8 --only many_worlds --bench` (exit status: 1)
```

- **First proven wall:** hub session admission, not open-world admission or a performance measurement. At the target SHA, `HubLimits::default().max_sessions` is 1,024. `bench_many_worlds` creates and drops idle-world leases sequentially, then holds one lease per active world while building the active set. The call to `lease` acquires a session permit with `try_acquire_owned`; permit exhaustion maps to `ResolveError::Busy` (`world_hub.rs`). With the run's `max_open_worlds=11,000`, the configured 256-open-world ceiling is not the limiting ceiling. The 1,025th concurrently held active lease therefore meets the default session ceiling and returns the exact observed message before the timed load begins. This is a reproducible harness/configuration wall; no latency, throughput, checkpoint, or completion result was produced.
- **Qualification:** the run proves the first code-level admission wall at the requested 100x active-world workload, but it is **not** a server-class performance run. The runner registry exposes only standard GitHub-hosted runners and the 4-vCPU Blacksmith fallback; neither meets the host class in #6304 (at least 16 threads plus NVMe-class storage). The host qualification requirement remains outstanding.
- **Evidence:** the Actions log records the target SHA, runner label, command, successful compilation, exact error, and exit status. Relevant sources at the target SHA: `pkgs/id/benches/world_bench.rs` (`bench_many_worlds`, active lease loop), `pkgs/id/crates/id-worlds/src/world_hub.rs` (`HubLimits::default`, `WorldHub::lease`).
