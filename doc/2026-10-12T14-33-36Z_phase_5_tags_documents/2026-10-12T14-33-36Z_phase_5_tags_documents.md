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
