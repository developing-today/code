# Tests

Rust integration tests for the `id` crate. The layers, and when each runs, are
in the
[testing reference](../../../doc/2026-03-29T00-00-00Z_reference_testing_architecture/2026-03-29T00-00-00Z_reference_testing_architecture.md).

## Files

- `cli_integration.rs`: the `id` binary as separate processes. The `serve_tests`
  module needs the network and is skipped by `just test-sandbox`. The world tests
  skip, with a message, when `examples/roc-world/targets/wasm32/host.wasm` or a
  Roc binary is missing.
- `remote_write_access.rs`: remote writes over QUIC on loopback, with write
  authorization.
- `tuple_properties.rs`: property tests for the tuple encoding in `src/tuple.rs`,
  and a fixed-seed fuzz run.
- `chain_properties.rs`: property tests for the event header in `src/chain.rs`:
  round trips, canonical re-encoding, the `after` limit, and a decoder that never
  panics.
- `regression_replay.rs`: replays every case in `regressions/`.
- `crash/`: the crash-safety gate for redb (plan P1.4). A fault model behind
  `redb::StorageBackend` cuts the power at a storage call of a 50-transaction
  workload, and the oracle checks the reopened state. `corpus.tsv` is the static
  replay list. See `crash/main.rs`. `crash/store.rs` is the event store's first
  creation (plan P1.3): a simulated directory with its directory steps as crash
  points, and the in-place control that must fail.
- `common/mod.rs`: the tuple checker shared by the property tests and the replay.
- `common/chain.rs`: the event decoder's contract, shared the same way.
- `regressions/`: checked-in failing cases. See its [README](regressions/README.md).

## Planted-bug check (#1395)

The plan of record (P2.3/P2.8) requires more than property tests: a planted
bug in the voiding/frontier logic must actually fail the suites, so the
suites cannot silently pass a broken implementation.

- Run it on demand: `just planted-bug-check`, or directly
  `tests/planted-bug-check.sh`.
- The script first requires the unmutated baseline to pass, then applies
  each named mutation from `tests/planted/` one at a time and requires the
  relevant suites to FAIL, restoring the mutated files after every run:
  - `voiding-authority.patch`: the resolver-filter verdict in
    `Membership::is_effective`
    (`crates/id-core/src/membership/layer.rs`) is inverted, so the
    concurrent operations of a removed member take effect instead of
    being voided. Caught by `voiding_scenarios` (seven S2 tests fail
    with it planted). Plant history (task 6582): mutations at
    `Engine::void` in `src/groups/delegation.rs` SURVIVED these suites —
    they exercise authority voiding through the event store (membership
    adds and content) and have no delegation-grant coverage, so they
    never reach `Engine::void`; the plant moved to the exercised path.
    Delegation-grant voiding scenarios remain an open gap, separate
    from this check.
  - `frontier-mapping.patch`: a group operation no longer contributes
    itself to the auth frontier (the mapping test in
    `CausalIndex::combine`, `crates/id-core/src/membership/frontier.rs`).
    Caught by the `membership::frontier` unit tests and
    `membership_properties` / `membership_scenarios`.
- Exit 0 means baseline green and every planted bug caught; exit 1 names a
  mutation that survived (a real test gap); exit 2 means the baseline is
  red, a patch no longer applies, or a mutated file was dirty.

## Running

- `just test-full`: the complete Rust suite, then the regression replay and the
  tuple property tests at a higher case count.
- `cargo test --test tuple_properties`: the property tests alone.
  `PROPTEST_CASES` raises the proptest case count, and `ID_TUPLE_FUZZ_CASES`
  raises the fixed-seed run.
- `cargo test --test regression_replay`: the replay alone.
- `cargo test --test crash`: the sampled crash sweep, the lying-sync control, and the corpus replay.
- `just crash-gate`: every crash point of the redb crash model, and the full control, in release.
- `cargo test --all-features -- --ignored`: tests quarantined with
  `#[ignore = "<reason>"]`.

## Conventions

- Test function names start with `test_`, as `pkgs/id/AGENTS.md` requires.
- Test modules and test crates allow `clippy::unwrap_used` and
  `clippy::expect_used`.
- A test that touches the network or starts a process must have a deadline, so a
  hang fails the test instead of blocking the run.
