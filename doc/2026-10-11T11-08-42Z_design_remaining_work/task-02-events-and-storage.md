# Task 02: Event header, canonical CBOR, and storage

- Status: Designed and spiked (S1, S5, S6). The waiting caps need an owner decision.
- Index: [what is left](./2026-10-11T11-08-42Z_design_remaining_work.md). Depends on: [task 01](./task-01-phase0-fixes.md) (for the MSRV).
- Design: [design §4 and §9.2](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T03-44-33Z_design_groups_events_delegation.md), [review §3.1–3.2](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T08-08-56Z_design_review.md)
- Evidence: spikes S1 (header), S5 (canonical CBOR), S6 (storage), S2 (fork freeze). See the [spikes README](../2026-10-11T03-44-33Z_design_groups_events_delegation/spikes/README.md).

## Goal

Every signed change becomes an event in its author's chain. Events are p2panda-core headers with our fields, encoded canonically, and stored in redb. This replaces the JSON artifact chains and the directory log.

## Scope

1. **Header.** p2panda-core (git rev `3e84cac48bba05c6857cce2ec433862b97f8d53c`) `Header`, with a `ChainExt` extension holding:
   - `kind` (text);
   - `group` (optional hash);
   - `after` (at most 32 hashes, `MAX_AFTER`);
   - the domain tag `id.events.v1`.

   Sequence base is 0. The operation ID is the BLAKE3 hash of the encoded header, including the signature.

2. **Verification.** Decode from bytes only. `AnyHeader::decode` verifies signatures. On receipt, check `MAX_AFTER`, reject unknown extension keys, require canonical re-encoding, and run `verify_strict` over the re-encoded bytes. Never use typed `verify()`, `validate_header` on in-memory headers, or cached `hash()`.
3. **Canonical CBOR.** `cbor-core =0.10.1` with `serde`. The canonical rule is decode, then re-encode and compare. On top of it, four value rules: NFC text (including map keys), no floats, no `undefined` or other simple values, and no tags. The S5 wrapper (`canonical.rs`, 93 lines: `decode`, `encode`, `admit_key`, `verify`) is the starting point.
4. **Store (redb =4.3.0).**
   - Events by ID.
   - Chains keyed by `(author, seq)`, with uniqueness checked inside the same write transaction.
   - A fork-proof table, and a frozen flag for an author's chain from the fork point. The frozen events are kept but not applied.
   - A waiting table with a per-author cap and an expiry. Waiting events do not affect state.
   - Index tables belong to task 06.
5. **Migration.** None. Discard the existing artifact chains and directory logs, as the owner allowed. Remove the serde_json signing from `artifact.rs`.

## Decisions

- Waiting caps: 256 pending events per author, and 7 days. Proposed; the owner sets the final numbers.

## Acceptance

- S1: header round trip for the 16 shapes in the spike. A forged signature is rejected by `check_received`, and typed `verify` is not used.
- S5: golden vectors. The header operation ID is `90acd5f7c498ccb93dc33ce41f49d166140bef38c1f85cb95940f311b6e7f212`. The signature is `be31be241b5a9915194cafabb3d0ffb8f7a2be7a7a91a9a638449cebfcd7d96f9bde95d6782eb70606f2e85b95760a90d77dab845fdb1ecda843defa7e4ecf0f`. Include the four other vectors from the S5 results. The rejection matrix becomes tests, and a 10,000-value subset of the mutation test runs in CI.
- S6: durability under SIGKILL, with every batch fully present or fully absent. A duplicate `(author, seq)` is rejected in the same transaction.
- S2: two events at one position store the proof and freeze the chain. Later events from that author are held, not applied.
- Migration check: a fresh store opens, and the old data directory is ignored and reported.

## Gotchas

- `PruneFlag` makes p2panda's backlink check skip. Do not enable pruning until retention is designed (task 10).
- Sequence base 1 forces a phantom backlink on the first event. Use 0.
- p2panda-store (SQLite via sqlx) conflicts with rusqlite (libsqlite3-sys). Do not depend on p2panda-store.
- redb refuses a second opener, even read-only. The serving node owns the data directory.

## Files

New module, for example `pkgs/id/src/events/` (header, canonical, store). Retired later: the chain code in `artifact.rs` and the entry log in `directory.rs`.

## Dependencies

p2panda-core (git, pinned), `cbor-core =0.10.1`, `unicode-normalization` (NFC). Report each one per `pkgs/id/AGENTS.md`. See [task 11](./task-11-dependencies-and-upstream.md).
