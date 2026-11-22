# Phase 12 exit audit — 2026-11-22

Step A1 of the 2026-11-21 results-first plan
([`2026-11-21_results-first.md`](2026-11-21_results-first.md)), filed as
[#12231](https://github.com/developing-today/code/issues/12231). One row per
exit criterion: the command, its output, and a verdict of met, not met, or
unmeasured. No fixes were made in this step; the audit records reality, it
does not change it. Every row that is not met links an open issue.

**Criteria source.** The nine criteria are the exit table in the Phase 12
plan, [`.opencode/plans/phase-12-platform-flexibility.md`](../../.opencode/plans/phase-12-platform-flexibility.md),
"What 'best in class' means here" (lines 13–23). The wording in the table
below is that table's, shortened only where noted.

**Placement.** This record sits next to the plan that ordered it, under the
same dated naming, because step A10's release notes must cite it and the
results-first plan is its ordering document.

**Measured against.** developing-today/code `main`:

- Tests and the `many_worlds` bench: `6bc7fe57b4` (2026-11-22T13:58:08Z),
  fleet-run [run 45030524013](https://github.com/developing-today/fleet/actions/runs/45030524013)
  (two legs: tests, bench).
- Follow-up run for the feature-gated conformance suite and the
  unlisted-host test: `666424e1c3` (2026-11-22T14:21:09Z), fleet-run
  [run 45032568757](https://github.com/developing-today/fleet/actions/runs/45032568757).

Test and bench commands ran on a GitHub-hosted fleet-run runner inside the
`pkgs/id` dev shell (`nix develop ./pkgs/id` from the repo root; each cargo
command below was run with `--manifest-path pkgs/id/Cargo.toml`). Doc and
issue-state checks ran with `grep` / `gh api`. Outputs below are trimmed to
the decisive lines; the full logs are the two run pages above.

**Tally: 5 met, 3 not met, 1 unmeasured, of 9.** The step A10 tag
(`id/v0.12.0`) stays blocked until a re-run of this audit shows nine rows
met.

| # | Criterion (target, from the exit table) | Verdict |
| - | --------------------------------------- | ------- |
| 1 | Time from empty directory to a running world a friend can join: under 5 minutes, one command per step, documented | **Unmeasured** |
| 2 | Adding a host capability: a new provider and its manifest, no edits to `world.rs` | **Met** |
| 3 | Guest languages with a passing conformance suite: at least 3 (Roc plus two of Rust, TypeScript or AssemblyScript, Zig) | **Not met** |
| 4 | Upgrading a running program: state preserved across versions, failed upgrade rolls back, proven by a test | **Met** |
| 5 | Idle worlds on one node: 1,000, with bounded memory per world | **Met** |
| 6 | Active worlds on one node: 100 worlds at 20 events per second each, with p99 command latency under 50 ms | **Not met** |
| 7 | Reproducing a world: `world.toml` plus a module hash recreates grants, ceilings, profile, schedules, and invites | **Met** |
| 8 | Bandwidth for a typical redraw: under 10 percent of the full-frame size (baseline from F11.9) | **Met** |
| 9 | Authority and sandbox: no new path to files, network, or processes except through a named, ceilinged provider | **Not met** |

## Row 1 — Empty directory to a joinable world in under 5 minutes: unmeasured

There is no documented procedure to execute and time. The command was a
search for one:

```text
$ find pkgs/id \( -iname "*quickstart*" -o -iname "*getting-started*" -o -iname "*tutorial*" -o -iname "*first-world*" \) -not -path "*/target/*"
(no results)
$ grep -n "## Quick Start" -A 3 pkgs/id/README.md
17:## Quick Start
18-
19-### With Nix (recommended)
```

The README's Quick Start covers building and serving the file-sharing CLI,
not creating a world a friend can join. The `id world` CLI surface exists
(`WorldJoinArgs` and siblings in `crates/id-cli/src/cli/world.rs`), but no
documented one-command-per-step path from an empty directory to a joinable
world exists in the tree, so the 5-minute target cannot be measured.
Tracked by [#12231](https://github.com/developing-today/code/issues/12231)
itself; no narrower open issue exists (searched 2026-11-22: quickstart,
getting started, tutorial).

## Row 2 — New provider with no edit to `world.rs`: met

Command:

```text
cargo test --test provider_registry_exit_proof
```

Output:

```text
test the_probe_is_visible_in_the_capability_catalog ... ok
test the_probe_stays_out_of_the_default_ceiling_until_a_ceiling_names_it ... ok
test the_compile_gate_knows_the_probe_and_refuses_it_until_granted ... ok
test result: ok. 4 passed; 0 failed
```

The probe provider is one registry manifest entry (the `test-provider`
feature in `id-caps`); nothing under the world dispatch learns about it,
and it appears in the catalog, stays out of the default ceiling until
named, and is known to the compile gate. This is F12.4's exit proof
(`pkgs/id/tests/provider_registry_exit_proof.rs`, #1061).

## Row 3 — At least 3 guest languages passing the conformance suite: not met

Commands:

```text
cargo test -p id-worlds --lib --features sandbox conformance
find pkgs/id/examples -name "*.wasm"
```

Output:

```text
test sandbox::tests::conformance_documented_exports_match_abi_md_signatures ... ok
test sandbox::tests::conformance_abi_version_export_returns_one ... ok
test sandbox::tests::conformance_buffer_ownership_and_shared_output_slot ... ok
test sandbox::tests::conformance_snapshot_restore_round_trip_is_byte_identical ... ok
test sandbox::tests::conformance_capability_protocol_replays_through_the_fixture_base ... ok
test result: ok. 5 passed; 0 failed

examples/lounge/lounge.wasm
examples/roc-chat/roc-chat.wasm
examples/roc-counter/counter.wasm
examples/roc-grid/roc-grid.wasm
examples/roc-screen/roc-screen.wasm
examples/roc-text-counter/text-counter.wasm
examples/rust-counter/counter.wasm
examples/territory/territory.wasm
```

The F12.7 conformance suite passes 5/5 — against the checked-in Roc
modules. Guests exist for exactly two languages: Roc (seven modules) and
Rust (`rust-counter`; `rust-world` has sources but no checked-in module).
There is no Zig, AssemblyScript, or TypeScript guest or SDK anywhere in
the tree, so the "at least 3" target fails on count before suite results
matter. Note the suite only compiles with the `sandbox` feature enabled
(`cargo test -p id-worlds --lib conformance` with default features runs 0
tests; `default = []` in `crates/id-worlds/Cargo.toml`).

#1065 (F12.8) is closed as completed (2026-11-09) and #1064 (F12.7) is
closed; the audit records the tree, which contradicts a third language
existing. No open issue covers the third language (searched 2026-11-22:
Zig, AssemblyScript, SDK), so this row is tracked by
[#12231](https://github.com/developing-today/code/issues/12231).

## Row 4 — Upgrade preserves state, failed upgrade rolls back: met

Command:

```text
cargo test -p id-worlds --lib upgrade
```

Output:

```text
test world::upgrade::tests::counter_upgrades_across_a_snapshot_format_change ... ok
test world::upgrade::tests::failing_upgrade_leaves_the_old_program_running ... ok
test world::upgrade::tests::program_without_migrate_export_carries_its_state ... ok
test world::upgrade::tests::oversized_migrated_state_rolls_back ... ok
test result: ok. 8 passed; 0 failed
```

State preserved across a snapshot format change, a deliberately failing
upgrade leaves the old program running, and an oversized migrated state
rolls back — F12.9's exit, proven by tests (#1066, closed).

## Row 5 — 1,000 idle worlds with bounded memory per world: met

Command (full-size configuration: 1,000 idle, 100 active, 20 events/s,
10 s):

```text
cargo bench --bench world_bench -- --only many_worlds
```

Output (the harness's own summary line; the same run as row 6):

```text
bench many_worlds unit=ms_per_command median=1.506 min=0.624 reps=1 p50_ms=1.506 p95_ms=2.228 p99_ms=3.653 wake_p50_ms=1.147 wake_p95_ms=1.830 wake_p99_ms=1.904 input_ack_p50_ms=0.318 input_ack_p95_ms=0.584 input_ack_p99_ms=2.522 target_p99_ms=50 target_met=true idle_worlds=1000 active_worlds=100 events_per_second=20 duration_secs=10 commands=20000 load_elapsed_secs=10.001 completed_events_per_second=1999.9 checkpoint_triggers=100 checkpoints=100 evicted=100 reopened=100
```

The harness materialized 1,000 durable idle worlds on one node, held them
non-resident through the hub's lazy-open and eviction path (its internal
checks require zero open worlds after eviction), and verified checkpoints
and records across eviction and reopen for all 100 active worlds
(`evicted=100 reopened=100`, exit code 0). Bounded memory per world is
the hub's open-world ceiling plus the per-world limits the harness runs
under; the resident set after eviction is zero. Measured on the
fleet-run runner; the idle/eviction behaviour is not the storage-bound
part of #557 (see row 6).

## Row 6 — 100 active worlds at 20 events/s, p99 under 50 ms: not met

Same command and run as row 5. On the fleet-run runner the target is met
with a wide margin: `p99_ms=3.653 target_met=true` (line quoted in row 5),
with the actor profile at `command_p99_us=10000` over 20,300 commands.

The criterion is nevertheless **not met**, because the result is
storage-class-bound and fails on the platform's reference machine. The
phase's baselines are taken on the fleet reference laptop
(`pkgs/id/benches/BASELINES.md`: "Reference machine — Laptop (fleet
reference machine)"), and on that machine the same harness measures far
over target: p99 5,650.792 ms recorded 2026-11-17 (agent note on #557),
and end-to-end p99 6,142 ms in the current #557 profile. The owning
issue [#557](https://github.com/developing-today/code/issues/557)
(F12.12) remains open on exactly this criterion. A re-run on the
reference laptop showing `target_met=true` flips this row.

## Row 7 — `world.toml` plus a module hash reproduces a world: met

Commands:

```text
cargo test -p id-caps world_manifest
cargo test -p id-caps world_caps
```

Output:

```text
test world_manifest::tests::an_empty_manifest_drifts_nothing ... ok
test world_manifest::tests::apply_is_idempotent ... ok
test world_manifest::tests::default_ceiling_round_trip_converges ... ok
test world_manifest::tests::drift_is_detected_after_a_manual_change ... ok
test result: ok. 38 passed; 0 failed

test world_caps::tests::test_provider_registry_matches_the_legacy_catalog_and_carries_manifests ... ok
test result: ok. 42 passed; 0 failed
```

Applying a manifest converges (a second apply changes nothing), drift is
detected after a manual change, and the ceiling round-trip converges —
F12.1's exit as specified ("apply twice and the second run changes
nothing; drift is detected after a manual `caps` change"). The apply
wiring and forking issues are closed (#807, #556).

## Row 8 — Typical redraw under 10 percent of the full frame: met

Command:

```text
cargo test -p id-core patch
```

Output:

```text
test world_presentation::tests::patch_parity_patched_client_equals_full_frame_client ... ok
test world_presentation::tests::patch_for_a_small_change_is_under_a_tenth_of_the_full_frame ... ok
test result: ok. 9 passed; 0 failed (across the id-core test binaries)
```

The criterion as written in the exit table — bandwidth for a typical
redraw under 10 percent of the full-frame size — is measured directly by
`patch_for_a_small_change_is_under_a_tenth_of_the_full_frame`, and the
parity test shows a patched client equals a full-frame client. Scope
note: this verdict covers the patch path the test measures. The rest of
F12.11 (binary encoding, compression) remains open in
[#805](https://github.com/developing-today/code/issues/805); the exit
table's row does not name those, so they do not change this row's
verdict, and #805 is not closed by it.

## Row 9 — No new reach except through a named, ceilinged provider: not met

Commands:

```text
cargo test -p id-caps world_caps
cargo test -p id-worlds --lib kv
cargo test -p id-worlds --lib blob
cargo test -p id-worlds --lib http_fetch
cargo test -p id-worlds --lib an_unlisted_host_is_refused
grep -rn '"notify' pkgs/id/crates/id-caps/src/world_caps.rs
```

Output:

```text
test result: ok. 42 passed; 0 failed   (id-caps world_caps: registry matches the catalog, manifests carried)
test world::tests::schedule_tests::kv_bounds_refuse_each_violation_with_a_reason ... ok
test world::tests::schedule_tests::kv_entries_replay_and_survive_compaction ... ok
test result: ok. 7 passed; 0 failed    (kv filter)
test world::tests::schedule_tests::blob_bounds_refuse_each_violation_with_a_reason ... ok
test result: ok. 16 passed; 0 failed   (blob filter)
test world::tests::schedule_tests::http_fetch_is_off_by_default ... ok
test world::tests::schedule_tests::http_fetch_refuses_redirects_off_the_list_and_past_the_hop_cap ... ok
test result: ok. 6 passed; 0 failed    (http_fetch filter)
test world::tests::schedule_tests::an_unlisted_host_is_refused ... ok
test result: ok. 1 passed; 0 failed
(grep for a notify provider in the catalog: no results)
```

Three of the four F12.5 providers now exist as named, ceilinged providers
with bound, refusal, and replay tests: `kv.*`, `blob.*`, and `http.fetch`
(off by default; an unlisted host is refused). The fourth, `notify`, does
not exist — no catalog entry, no dispatch. The phase's new-reach surface
is therefore not the complete named-provider set the plan specifies, and
[#1062](https://github.com/developing-today/code/issues/1062) (F12.5)
remains open. When `notify` lands with its bound/refusal/replay tests,
this row flips on a re-run of the commands above.

## What remains before the tag

- Row 1: a documented quickstart procedure exists and a timed run of it
  is recorded. Tracked by #12231.
- Row 3: a third guest language (Zig or AssemblyScript/TypeScript) with
  an SDK and an example passing the conformance suite. Tracked by #12231.
- Row 6: `many_worlds` at `target_met=true` on the reference laptop.
  Tracked by [#557](https://github.com/developing-today/code/issues/557).
- Row 9: the `notify` provider ships with bound, refusal, and replay
  tests. Tracked by [#1062](https://github.com/developing-today/code/issues/1062).

Step A10 (tag `id/v0.12.0`) runs only when a re-run of this audit shows
nine rows met.
