# Phase 5: tags, documents, and search

- Plan: [phased plan](../../2026-10-11T11-08-42Z_design_remaining_work/plan-phases.md), section 10.
- Start commit: `c12448c9` (origin/main after Phase 3).
- Base branch `phase-5`. Tracks: `p5-tags` (P5.1, P5.2), then `p5-compact` (P5.3), `p5-docs` (P5.4, P5.6), `p5-search` (P5.5), `p5-alias` (P5.7).
- Decisions in force: tags are claims on any event or document ID, anyone may tag anything, attributed to the signing key; observed-remove clears; user subject is the first identity event (owner); aliases are tags shown contested, with server primary, prefix disambiguation; compaction by snapshot at a causal cut; content on removal kept.

## Steps

| Step                         | Track   | Status | Commits | Tests | Notes                  |
| ---------------------------- | ------- | ------ | ------- | ----- | ---------------------- |
| P5.1 Observed-remove tags    | tags    | todo   |         |       | S8 model               |
| P5.2 Tag index on tuple keys | tags    | todo   |         |       |                        |
| P5.3 Compaction              | compact | todo   |         |       |                        |
| P5.4 Documents               | docs    | todo   |         |       |                        |
| P5.5 Search                  | search  | todo   |         |       |                        |
| P5.6 Content on removal      | docs    | todo   |         |       | decision 4             |
| P5.7 Aliases as tags         | alias   | todo   |         |       | owner answers recorded |

## Decisions

- See above and the index.

## Open questions

- User tag subject needs an identity event (issue).

## Changes to the plan

- None yet.

## Review notes and regression tests

- To be filled.

## End

- Pending.

---

## 2026-10-12T16-45-04Z Progress: tags on the signed event, and compaction

Base: `b6f1a1d5` (phase-5). Track: `p5-bodies`. Commits, in order:

- `c999190e`, `9a91339b`, merged at `b6f1a1d5`: P5.1 observed-remove tags, P5.2 tag index (earlier).
- `2250f804` `feat(rust): tag body inside the signed tag event [P5.4 prep, #182]`.
- `79efff10` `feat(rust): compaction of tag claims and clears [P5.3, #184]`.

### Steps

| Step                      | Status          | Commits    | Tests                                                                            |
| ------------------------- | --------------- | ---------- | -------------------------------------------------------------------------------- |
| P5.1 Observed-remove tags | done            | `c999190e` | S8 tests in `tests/tag_claims.rs`                                                |
| P5.2 Tag index            | done            | `9a91339b` | `tests/tag_index.rs`                                                             |
| #182 Body in the event    | done            | `2250f804` | `test_two_replicas_that_exchange_only_events_converge_on_the_tag_set`            |
| P5.3 Compaction (#184)    | done, in memory | `79efff10` | `test_the_compacted_fold_agrees_with_the_causal_walk` (16 cases, 2 to 3 writers) |
| P5.4 Documents            | todo            |            |                                                                                  |
| P5.5 Search               | todo            |            |                                                                                  |
| P5.6 Content on removal   | todo            |            |                                                                                  |
| P5.7 Aliases as tags      | todo            |            |                                                                                  |

### Red, then green

- #182. Before `2250f804`, a replica that received only event bytes, as sync does, had an empty tag set: `left: {}`, with two claims expected. After it, the test passes.
- P5.3. With `compacted_claims` stubbed to remove nothing, the property failed at the minimal case `seed = 0, writers = 2, steps = 17`. The fold then passed all 16 cases. The failing seed is kept in `tests/tag_claims.proptest-regressions`.

### Decisions

- **#182: the tag body is in the signed header.** A tag's operation is the `tag` field of `ChainExt`. The header signs it, so an event carries its whole tag. Sync moves events alone, and no body is fetched. A tag event takes no payload body, and a payload beside one is refused. The `tag_ops` table, its write path, and the body codec (`encode_op`, `decode_op`, `MAX_TAG_BODY_BYTES`) are removed. The store format is now v8. A v7 store holds tag events whose operation is not in the header, so it is refused.
- **Value limit: 512 bytes, was 1024.** The header decoder (p2panda `AnyHeader::decode`) declares text and byte strings at most 512 bytes long. A value is text inside the header, so a 1024-byte value could never decode. A clear names at most 256 claims, which is within the 512-element array limit. **Owner to confirm** the 512-byte value limit.
- **Compaction is an in-memory fold.** `compacted_claims` makes one pass over the parent links. Each event gets the live claims in its past. A clear removes the claims it names that are in its parents' sets, and removed claims are dropped from later sets. So the per-clear walk over history is gone, and every set is bounded by the live claims. `live_claims` stays as the reference. Nothing is deleted from the store: all events are kept, and no snapshot is written. The fold runs on each `tag_claims()` call.
- **A clear from an author who had not seen a claim leaves it live.** This is unchanged. The property checks it against the writers' own record of what they had seen.

### Open questions

- **#185 (clear authority): untouched.** Owner decision. Anyone may clear a claim their author had seen.
- **#160 (user subject): untouched.** Owner decision. It needs the identity event.
- **N33 snapshot at a causal cut: not done.** The fold is in memory and nothing is persisted. A snapshot needs a decision on what a new device receives and how voiding interacts with it. Retention is P8.2.
- **Group bodies have the same sync gap.** Sync sends header bytes only (`store_traits.rs`, the operation the sync protocol sends). A group operation's body does not travel either. This branch leaves group bodies alone.
- **Lints.** The new code adds pedantic warnings of the same kinds the crate already has (a generic hasher on `HashMap` parameters, and a long first doc paragraph). These are deferred to P8.8.

### Test results

- `cargo test --lib tag`: 115 passed.
- `cargo test --test tag_claims`: 10 passed. `--test tag_index`: 4 passed.
- `cargo test --lib`: 1104 passed, 0 failed, 3 ignored.
- The other integration suites that build events: `chain_properties` 12, `production_ingest` 10, `voiding_scenarios` 12, `voiding_properties` 2, `event_store_properties` 11 (1 ignored), `regression_replay` 8. All pass.

### Changes to the plan

- P5.3 was "snapshot at a causal cut, then the tail (N33)". This track does the fold part, and the persisted snapshot is still open.
- P5.4 is no longer blocked on the body: a tag carries its body. Its "content events" are unaffected.

### Next

P5.4 to P5.7 are not started. P5.4 needs the document content events on the same header pattern, if it uses the same extension. #185 and #160 wait on the owner.

---

## 2026-10-12T17-16-20Z Progress: documents (P5.4)

Base: `37a8344b` (phase-5). Track: `p5-docs`. Commits, in order:

- `b36e26dc` `feat(rust): documents as signed events with content hashes [P5.4]`
- `9bcd86fa` `test(rust): documents converge over events and blob fetch [P5.4]`

### Steps

| Step           | Status                                                          | Commits                | Tests                                                          |
| -------------- | --------------------------------------------------------------- | ---------------------- | -------------------------------------------------------------- |
| P5.4 Documents | done for the event and the blob path. Live editing is not built | `b36e26dc`, `9bcd86fa` | `tests/documents.rs` (5), unit tests in `src/documents.rs` (4) |

### Red, then green

- Red. `tests/documents.rs` was written first. Its first build failed, because `id::documents` did not exist. With the types in place and `EventStore::documents` returning no rows, three tests failed at the record check (`left: 0`, `right: 1`). The signing check and the body refusal passed, as they should.
- Green. With `documents()` reading applied `document.put` events, `tests/documents.rs` passes 5 of 5.

### Decisions

- A document is a `document.put` event. Its header names the content by BLAKE3 hash, its size, and its media type (`ChainExt.document`, carried the same way the tag operation is, #182). The event takes no payload body, and a body beside it is refused on receipt.
- The bytes are a blob. iroh-blobs verifies them as they arrive, and the blob gate decides who may fetch them. A replica that keeps fetched bytes calls `DocumentRef::verify_content` first, which checks the length, then the hash. `blob_hash()` is the content hash, since both are BLAKE3 over the same bytes.
- Tags on a document are `tag.set` and `tag.clear` events whose subject is the document event's ID. A clear removes a tag claim, never the document, which stays in `documents()`.
- Store format: unchanged. The `document` field is left out of the encoding when it is `None`, so existing events keep their bytes. No version bump.
- Tag value limit: `MAX_VALUE_BYTES` stays at 512 (owner to confirm). Clear authority (#185) and the user subject (#160) are not touched.

### Changes to the plan

- The P5.4 row also asks for snapshots from the collab server, and a save on explicit save and on idle close. Not built. Nothing produces a `document.put` from the ProseMirror collab server. The trigger is task 10, item 6.
- Design §9.3's inline `json` and `bytes` payloads are not built. Bodies do not travel with sync (`store_traits.rs`), and the header holds byte strings of at most 512 bytes, so every document is a blob.
- The blob gate still has no reference index producer. The test gives `WorldAccess` a fixed table, as `tests/blob_gate.rs` does.
- The test hands event bytes to `EventStore::ingest`, the path sync takes once it has the bytes. It does not run a live `/id-sync/1` session.

### Open

- Two property tests fail on random seeds. Resolved by the discovery section below (`0f3b7eaa`). Both fail the same way on `37a8344b`, checked in a scratch copy of HEAD with its own target directory. Neither is caused by this track.
  - `tests/tag_claims.rs::test_the_compacted_fold_agrees_with_the_causal_walk` (P5.3). Seeds `12233682913237194312` (writers 3, steps 32) and `14490625886382135617` (writers 3, steps 23).
  - `tests/tag_claims.rs::test_the_live_set_is_the_oracle_under_any_delivery_order` (P5.1). Seed `9408785462023634370` (steps 28).
  - The seeds are not in `tests/tag_claims.proptest-regressions`, which stays at HEAD. The fix belongs to the tags track, and the seeds go in as regressions with it.
- Lib-only build (`--no-default-features`) fails on `src/log_sync.rs:49-50` and `src/groups/worlds.rs:30`: `crate::world` and `crate::world_net` are not gated. The same imports are at HEAD.
- `Staged::keeps` in `src/event_store/ingest.rs` has no call sites at HEAD (a dead-code warning, not from this track).

### Test results

- `cargo test --lib`: 1108 passed, 0 failed, 3 ignored. Before this track it was 1104; the 4 new are the unit tests in `src/documents.rs`.
- `cargo test --test documents`: 5 passed. Before the query existed, 2 passed and 3 failed.
- Also pass: `tag_index` 4, `voiding_scenarios` 12, `voiding_properties` 2, `chain_properties` 12, `production_ingest` 10, `regression_replay` 8, `blob_gate` 8, `event_store_properties` 11 (1 ignored).
- `tag_claims`: 8 passed, 2 failed. The two failures are the random properties under Open.
- Clippy on the lib and the tests: no new warnings from this track. The one from the new test file, a redundant clone, is removed.

### Next

- P5.5 search reads `tag_claims()`, and needs nothing from documents. A document is the subject of ordinary claims.
- P5.6 content on removal: no code yet. `documents()` keeps a document until its record says otherwise. The removal path is not tested here.
- P5.7 aliases are tags. The server primary alias needs a decision.
- Still open: the blob reference index producer, and the collab-server trigger for document events.

## 2026-10-12T20-04-02Z Discovery: the tag property failures were in the test history generator

Both properties failed at `37a8344b`, and the cause was in the oracle side, not the fold or the store. `history()` in `tests/tag_claims.rs` shadowed the writer count with a clear's claim count, so a clear's `after` named only the first 1 to 3 writers. A writer could have seen events its clear did not name. The oracle counts everything the author had seen, but the causal past read from the parent links did not include them. The fold and the causal walk agreed on every history, including the failing ones.

- Sweep over 5000 histories: with the shadow, the walk and the fold agree on all, and the fold differs from the oracle on 148. With the rename, there are no mismatches.
- Red, then green: `a1304850` adds the regressions and fails on both properties and both named tests. `0f3b7eaa` renames the variable, and everything passes.
- The regression file keeps `cc 0759f56f…`. It replays under both properties. `test_the_regression_histories_reach_the_oracle_through_every_path` runs the two histories from the task in every test run. `test_each_event_names_every_event_its_author_had_seen` checks each event's causal past against its author's record.
- `PROPTEST_RNG_SEED` selects the runner's RNG, not the input. The `(seed, steps, writers)` triples in the task are the inputs to `history()`, and the named tests use them directly.
- Suites after the fix: `cargo test --lib tag` 115 passed, `--test tag_claims` 12 passed (the 10 existing plus 2), `--test tag_index` 4 passed, `cargo test --lib` 1104 passed, 3 ignored. The 256-case runs of both properties pass, using a scratch copy with the case count set to 256, since the explicit `cases` in the config overrides `PROPTEST_CASES`.
