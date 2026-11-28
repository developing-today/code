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

## Planted-bug check (#1395, #1421)

The plan of record requires more than property tests: planted bugs in the
membership/voiding logic and the tag observed-remove fold must actually fail
their suites, so the suites cannot silently pass a broken implementation.

- Run it on demand: `just planted-bug-check`, or directly
  `tests/planted-bug-check.sh`.
- The script first requires the unmutated baseline to pass, then applies
  each named mutation from `tests/planted/` one at a time and requires the
  relevant suites to FAIL, reversing each applied patch after every run:
  - Layer 11 runs this on demand at phase end for membership and tags; it is
    not part of the per-PR gate (see the
    [remaining-work plan](../../../doc/2026-10-11T11-08-42Z_design_remaining_work/plan-phases.md)).
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
    The delegation-grant gap was closed by #7071: see the
    `delegation-voiding` plant below.
  - `delegation-voiding.patch`: the strong-removal gate at
    `Engine::void` (`src/groups/delegation.rs`) is inverted, so a grant
    concurrent with its grantor's removal stands under strong remove and
    is voided under weak remove. Caught by `delegation_voiding_scenarios`
    (#7071), which judge a grant, its grantor's concurrent removal, and
    the grantee's onward operation through the groups fold — the
    delegation-grant coverage the voiding suites never had.
  - `frontier-mapping.patch`: a group operation no longer contributes
    itself to the auth frontier (the mapping test in
    `CausalIndex::combine`, `crates/id-core/src/membership/frontier.rs`).
    Caught by the `membership::frontier` unit tests and
    `membership_properties` / `membership_scenarios`.
  - `tag-clear-unseen.patch`: the compacted tag fold removes a named claim
    without checking whether the clear saw it. Caught by
    `test_a_clear_of_an_unseen_claim_does_nothing_to_it` in the full
    `tag_claims` integration suite.
  - `tag-clear-transitive-history.patch`: the compacted tag fold forgets
    claims inherited through a parent event. Caught by
    `test_a_clear_removes_a_claim_its_frontier_reaches_only_through_another_author`
    in the full `tag_claims` integration suite.
- Exit 0 means baseline green and every planted bug caught; exit 1 names a
  mutation that survived or failed to trigger its named regression test (a real
  test gap); exit 2 means the baseline is red, a patch cannot be applied or
  reversed, or a mutated file was dirty.

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
