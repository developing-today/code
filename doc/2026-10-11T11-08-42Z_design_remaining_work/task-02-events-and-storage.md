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

## 2026-10-11T12-20-38Z Update: storage answers and open items

- Event store engine (N1): redb preferred. S6 measured redb against rusqlite: writes 93 µs vs 187 µs, subject lookup 24 µs vs 57 µs, name=value 5.7 µs vs 11.9 µs. Files 269 MB (135 MB compacted) vs 97 MB. RSS 193 MB vs 104 MB.
- redb downsides: one process per file (a second opener is refused, even read-only), larger files, no SQL, a younger project. iroh-blobs already uses redb, so it adds no dependency.
- Turso (crate `turso` 0.8.2, a Rust rewrite of SQLite) is not measured. It needs the S6 harness before any decision.
- Identity database (N2): libsql is the only SQLite-family dependency. It is optional behind the `web` feature and used in `src/web/identity.rs`. Keep it until N1 is settled.
- p2panda-store (N3): the traits are `LogStore`, `OperationStore`, `GroupsStore`, `OrdererStore`, `TopicStore`, `CursorStore`, `KeySecretsStore`. It also ships `SqliteStore` (sqlx), which conflicts with rusqlite 0.40 on libsqlite3-sys. Preferred: implement the traits over redb if they fit our schema. S7 tests this on real stores.
- Chain identity (N4): there is no `LogExt` in p2panda-core 0.7.1. ChainExt goes on every event.
- Power-loss tests (N13): S6 used SIGKILL only. Run LazyFS or a VM power cut before the engine is final.
- Waiting caps (#6): unchanged, pending the owner.
- Doc error (N17): `src/store.rs` says FsStore is SQLite. It is redb.

## 2026-10-11T12-49-09Z Update: tags spike and compaction (S8)

- S8 ran the observed-remove tag model with 300 random cases, and 3,000 in a soak. Every replica matched an independent oracle after every step.
- The clock must cover the whole causal past. A frontier-only clock leaves an add that the clearer had seen. The spec must say so.
- Metadata stays flat under churn: 117 B to 121 B across 20,001 operations. Tombstones would cost about 3 B per removal.
- Compaction: take a snapshot at a causal cut, then serve the tail. Dropping stable clear bodies without a snapshot brings back removed adds for a new device. That is a tested counterexample. Do not drop clear bodies without a snapshot.
- With the real `tuple.rs` before the fix, `"a"` matched `"a\0b"` in name and value prefixes. The `00 00` terminator fixes it. Subject-only prefixes are exact for 32-byte subjects.
