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
  replay list. See `crash/main.rs`.
- `common/mod.rs`: the tuple checker shared by the property tests and the replay.
- `common/chain.rs`: the event decoder's contract, shared the same way.
- `regressions/`: checked-in failing cases. See its [README](regressions/README.md).

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
