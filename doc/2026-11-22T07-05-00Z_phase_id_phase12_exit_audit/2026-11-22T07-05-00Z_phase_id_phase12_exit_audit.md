# Phase 12 exit audit — 2026-11-22

Audited against `main` at `8c4edca6251f95f7d5d2f03e64aef7a7ccd77420`. Scope is the nine exit criteria in `.opencode/plans/phase-12-platform-flexibility.md`; this is an audit only, with no Phase 12 product fixes. “Met” requires direct evidence; “not met” records a failed target; “unmeasured” means the available evidence does not prove the target.

| # | Exit criterion | State | Command and observed output | Open tracking issue |
| --- | --- | --- | --- | --- |
| 1 | Empty directory to joinable world in under 5 minutes | **Unmeasured** | `rg --files scripts pkgs/id | rg -i 'five|join|world.*path|path.*world|bootstrap'` found only unrelated `scripts/muse/bootstrap.sh`; no A9 path runner or timed result. | [#796](https://github.com/developing-today/code/issues/796) |
| 2 | Add a host capability without editing `world.rs` | **Met** | `~/tools/fleet-build-lock cargo test --manifest-path pkgs/id/Cargo.toml --test provider_registry_exit_proof` — **4 passed, 0 failed**. The registry-only test provider is visible in the catalog, excluded from the default ceiling, and refused by the compile gate until explicitly granted. | — |
| 3 | At least 3 guest languages pass conformance | **Not met** | `find pkgs/id/examples` shows Roc examples and Rust `rust-world`/`rust-counter`; `rg --files pkgs/id/examples | rg -i 'zig|assembly|typescript'` finds only Zig host files under `roc-world/host`, no third guest SDK. Plan A7 names Zig. | [#1067](https://github.com/developing-today/code/issues/1067) |
| 4 | Upgrade preserves state and rolls back a failed upgrade | **Unmeasured** | The two exit tests exist: `counter_upgrades_across_a_snapshot_format_change` and `failing_upgrade_leaves_the_old_program_running`. `cargo test --manifest-path pkgs/id/Cargo.toml -p id-worlds --lib upgrade` was stopped during compilation before any test ran; no pass is claimed. | [#796](https://github.com/developing-today/code/issues/796) |
| 5 | 1,000 idle worlds with bounded memory per world | **Unmeasured** | Recorded full-size command: `cargo bench --bench world_bench -- --only many_worlds`. The 2026-11-14 baseline output contains `idle_worlds=1000`; the separate per-world budget/refusal evidence is recorded on #557, but this audit did not rerun it or capture an aggregate memory result. | [#557](https://github.com/developing-today/code/issues/557) |
| 6 | 100 active worlds at 20 events/s with p99 under 50 ms | **Not met** | Same 2026-11-14 full-size baseline: `p99_ms=4549.272 target_p99_ms=50 target_met=false` (20,000 commands; 100 active, 1,000 idle). Source: `pkgs/id/benches/BASELINES.md`, run at `a824f622ae`. | [#557](https://github.com/developing-today/code/issues/557) |
| 7 | `world.toml` plus module hash reproduces grants, ceilings, profile, schedules, and invites | **Unmeasured** | `apply_is_idempotent` exists in `id-caps::world_manifest`; the full module-hash-to-world reproduction path was not rerun end to end in this audit. A closed implementation issue is not treated as fresh proof. | [#796](https://github.com/developing-today/code/issues/796) |
| 8 | Typical redraw uses under 10% of a full frame | **Unmeasured** | `gh issue view 805 --json state,title` returns `OPEN`. PR #12241 remains open with a Rust codec foundation; #12242 remains open for negotiated transport. No current under-10% measurement was found. | [#805](https://github.com/developing-today/code/issues/805) |
| 9 | No new file, network, or process access except through a named, ceilinged provider | **Unmeasured** | `gh issue view 1062 --json state,title` returns `OPEN`; F12.5 provider work is unfinished. The full authority boundary was not re-audited here, so absence of new providers is not counted as proof. | [#1062](https://github.com/developing-today/code/issues/1062) |

## Release decision

The audit does not show nine met criteria. Do not create `id/v0.12.0` yet. Re-run this audit after the linked gaps are resolved; A10 requires all nine rows to be met before tagging.

## Evidence limits

The many-worlds result is the latest committed full-size measurement, not a fresh run on this audit date. The provider registry test is the only exit test rerun here. Historical test names, closed issue state, and partial feature work are not counted as passing evidence.
