# Task 01: Phase 0 fixes

- Status: Open. No design dependency. These fix current code and prepare the build.
- Index: [what is left](./2026-10-11T11-08-42Z_design_remaining_work.md)
- Design: [design §12–§13](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T03-44-33Z_design_groups_events_delegation.md), [review §5](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T08-08-56Z_design_review.md)

## 1. Tuple prefix bug

`pkgs/id/src/tuple.rs` escapes a NUL byte as `00 FF` and ends each field with a single `00`. An exact-match prefix for the field `a` therefore also matches the field `a\0b`, which encodes as `… a 00 FF … 00`. S6 reproduced this. `tags.rs` builds its index with this encoder, so a value containing NUL can match an unrelated query.

- Fix: end each field with `00 00`. `00 00` cannot occur inside escaped data, so the terminator is unambiguous. Check the nested-tuple markers the same way. The nested-tuple test around `tuple.rs:713` uses the old marker.
- Keep the ordering: `a` must still sort before `a\0b`, and `a` before `a\x01`.
- Acceptance: a regression test in `tuple.rs` (exact match for `a` does not match `a\0b`), plus the S6 selftest cases, plus an ordering test.

## 2. Pull pages capped by bytes

`artifact.rs` pages by item count (`PAGE_LIMIT` = 100). A page of 100 documents at 64 KiB each is about 6.4 MB, which cannot fit the 1 MiB frame (`MAX_FRAME_BYTES`). The current pull therefore fails for large accounts.

- Fix: page by bytes, with a target of about 256 KiB. A page always carries at least one item. An item larger than the target is sent alone.
- Acceptance: a test that serves 100 maximum-size documents and checks that no frame exceeds `MAX_FRAME_BYTES`, plus a single oversized item sent alone.

## 3. Rust version

`pkgs/id/Cargo.toml` declares `rust-version = "1.91.0"`. The p2panda crates need 1.96 (S1–S3 compiled them with 1.97.0). Raise it to `1.96`. The toolchain pin stays at 1.97.0.

## 4. Verification rule

Signed data is checked only from its encoded bytes, with `verify_strict`. Never call the typed `verify()` (p2panda-core's `Header::verify` returns `true` in non-test builds), `validate_header` on an in-memory header, or the cached `hash()`. Task 02 enforces this in the new code. Task 01 adds a review note and a test that fails if a typed verify call is added to the new modules.

## 5. Dial deadlines in the remaining modules

`connectivity::connect` (from the n0 change) wraps a dial in a 45 s deadline. Use it in `artifact.rs` (pull) and in `envelope_net.rs`. The envelope path has its own 30 s constant, so check that it wraps the whole delivery, lookup included.

- Acceptance: a dial to an ID whose lookup never answers returns an error within 45 s, in both modules.

## Acceptance for the task

- `cargo test --lib --features world`: only the three known `world_compile` failures (they need `host.wasm`).
- `cargo test --features world --test cli_integration`: all pass, skipping the known hang (`test_world_module_shared_over_iroh_and_web_presentations`) and the known Roc test.
- `rustfmt --check` is clean on the changed files. No `unwrap` or `expect` in non-test code.

## Files

`pkgs/id/src/tuple.rs`, `pkgs/id/src/tags.rs` (the test only, if needed), `pkgs/id/src/artifact.rs`, `pkgs/id/src/envelope_net.rs`, `pkgs/id/Cargo.toml`.

## 2026-10-11T12-20-38Z Update: tuple terminator fixed

- Done in commit `00dccae5` on `feat/id-n0-relay-default`. Bytes and strings end with `00 00`. A NUL inside a value stays `00 FF`. The decoder rejects a lone `00` and any escape other than `FF`.
- Cost: one byte per terminated field, not four.
- Keys written in the old format no longer decode. The data can be discarded.
- Tests: `test_full_field_prefix_matches_only_equal_values`, `test_prefix_full_field_ignores_longer_value_with_nul`, `test_sort_strings_with_nul`, `test_decode_single_nul_is_not_a_terminator`, and three exact-byte tests updated. `cargo test --lib tuple` 46 passed. Tags 71 passed. Full lib 748 passed, with the three known failures skipped. Doc tests and rustfmt pass.
- Clippy: pending a clean run in a fresh target directory. An earlier run mixed rustc builds and failed with E0514, which is an environment problem, not a lint.
- Still open in task 01: pull pages by bytes, MSRV (1.91 declared, 1.96 needed), dial deadlines in the remaining modules.

## 2026-10-11T17-58-40Z Update: round 2 results

- The round 2 answers are in the [explainers](./explainers.md), the [index](./2026-10-11T11-08-42Z_design_remaining_work.md), and the [round 2 results](./research-round2/README.md).
- The steps this task feeds are in the [phased plan](./plan-phases.md). Each step names its blocker and its tests.
