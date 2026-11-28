# S8: observed-remove tags

## Question

Does the observed-remove tag design (design §9.1, task 06, review §3.8) hold?

1. Convergence: 3–5 replicas, random partitions, any delivery order and duplicate ops, same state.
2. Add-wins: a concurrent add and clear of the same entry leave the entry present.
3. A clear that names a dot its author never saw does not remove that dot.
4. The merge is commutative, associative and idempotent.
5. Metadata: how many CBOR bytes per entry as adds and removes accumulate. Are tombstones needed? What compaction rule is safe?
6. How subject lookups map to tuple keys in `pkgs/id/src/tuple.rs`.

Scope: standalone crate in this folder. `pkgs/id` was read, not modified (`tuple.rs` is included with `#[path]`).

## Verdict

- **1–4 hold** for the model below. Every property passed on the real implementation: 300 cases each by default, and 3000 cases each in a soak. No property failed, so there is no counterexample to report for the real code. The suite also catches five planted bugs (below), so the passes are not vacuous.
- **Tombstones are not needed.** A removed dot is covered by its device's counter, because each device's sets and clears share one gapless numbering. Removal costs O(devices) metadata, not O(removes).
- **Two rules must be written into the spec.** (a) A clear removes a dot only if it is *named and covered* by the clear's clock. The design sentence "an add survives if its seq is greater than its author's entry" describes only the covered half. (b) The clock must cover the whole causal past. A frontier-only clock leaves an add that the clearer saw (test `frontier_only_clock_leaves_an_add_the_clearer_saw`).
- **Compaction:** fold the log at a causally closed cut into a snapshot, and serve a peer that does not cover the cut the snapshot first. Dropping stable clear bodies without the snapshot path resurrects removed adds for a new device (tested).
- **Tuple keys:** subject-only prefix scans are exact for 32-byte subjects with the current encoder. Name and value prefixes are not: the NUL bug from task 01 reproduces with the real `tuple.rs`. The `00 00` terminator fixes it (tested on a local copy). S6's `prefix ++ 0xFF` bound also works.

## Model built

- Device = 32-byte key (placeholder for Ed25519). Subject = 32 bytes. Name and value are strings.
- Each device's events (sets and clears) form one chain numbered 1, 2, 3, ...
- A set creates dot `(device, seq)`. A clear names dots and carries `clock`, its author's whole version vector when issued.
- A named dot is removed iff `clock` covers it (`seq <= clock[device]`).
- Delivery: an op waits for its chain predecessor, and a clear also waits until its whole clock is applied. Duplicates are dropped by the version vector. Ops can arrive in any order.
- State = live dots (with entries) and the version vector. No tombstones. Covered-but-absent means removed.
- Join (state-based sync): keep a dot if both sides hold it, or one holds it and the other has not covered it. Version vectors take the pointwise max.
- Oracle (`reference`): computed directly from the set of ops, with no buffering. Live = sets not removed by a named and covered clear. Each replica is checked against it after every step.
- Encoding: positional CBOR arrays via `cbor-core` 0.10.1 (`Value::encode`, canonical by construction). Op ID = BLAKE3 of the encoding. Strict decode round-trip is tested.
- Not modelled: signatures, the p2panda header (`prev`, `after`, `backlink`), device keys and revocation, persistence.

Design choices I made, for redirect: the removal rule (named and covered); the clock as the full version vector; per-chain causal readiness; dots named by `(device, seq)` where the design names claims by event ID (one event, one dot, so the two are equivalent here).

## Method (commands)

```sh
export CARGO_TARGET_DIR=/home/user/work/spikes/target
CARGO="$(rustup which --toolchain 1.97.0 cargo)"   # 1.97.0
cd /home/user/work/spikes-next/s8-tags
$CARGO test --no-fail-fast                          # logs/test.txt
S8_CASES=3000 $CARGO test --test props --test compaction --test encoding   # logs/soak_3000.txt
CARGO="$CARGO" bash scripts/mutants.sh              # logs/mutants.txt, logs/mutant_*.txt
$CARGO run --release --example metadata > logs/metadata.md
```

Proptest config: 300 cases per property by default (`S8_CASES` overrides). Failures would be shrunk by proptest. Crate deps: `blake3` 1, `cbor-core` =0.10.1 (pins from `spikes/Cargo.toml`); dev: `proptest` 1.12 (added; not in the spikes lock), `anyhow` 1 (for `tuple.rs`). `Cargo.lock` copied from `spikes/`. The `serde` dependency from the brief was not used: encoding is explicit arrays, which avoids serde's byte-as-integer behaviour and map ordering.

## Results: properties on the real implementation

| # | Property | Test (tests/) | Cases | Result |
|---|---|---|---|---|
| 1 | Convergence under random partitions, op and state sync, duplicates and any order. Every replica equals the oracle after every step, and all converge after a heal. A fresh observer's state-based join equals the oracle over the union of applied ops. | props: `convergence_under_partitions_and_any_delivery_order` | 300; soak 3000 | pass; pass |
| 2 | Add-wins: a clear removes only what its author saw. Adds concurrent with it survive, in any order (1–3 concurrent adders). | props: `concurrent_adds_survive_a_clear_that_did_not_see_them` | 300; soak 3000 | pass; pass |
| 3 | Unobserved removal: a clear naming unseen dots (1–3) leaves them live, and removes the observed ones (0–3), in any order. | props: `clear_naming_unseen_dots_does_not_remove_them` | 300; soak 3000 | pass; pass |
| 4 | Join is commutative, associative and idempotent on reachable states (snapshots from random runs). | props: `join_is_commutative_associative_and_idempotent` | 300; soak 3000 | pass; pass |
| – | Late duplicate of a removed add is dropped; a clear waits for the add it names. | props: `late_duplicate_of_a_removed_add_is_dropped` | unit | pass |
| – | Frontier-only clock fails to remove an add the clearer saw (3 devices, 3 ops). | clock: `frontier_only_clock_leaves_an_add_the_clearer_saw` | unit | pass (documents the failure) |
| – | Snapshot-then-tail compaction rule correct for every peer and cut (200 histories × 4 cuts × 3 peers). | compaction: `snapshot_then_tail_is_correct_for_every_peer_and_cut` | 200 histories | pass |
| – | Dropping stable clear bodies without a snapshot resurrects a removed add for a new device. | compaction: `dropping_a_stable_clear_body_resurrects_the_add_for_a_new_device` | unit | pass (documents the failure) |
| – | Snapshot path gives the right state for a new device. | compaction: `serving_the_snapshot_to_a_new_device_is_correct` | unit | pass |
| – | Every op and state encoding is strictly canonical (decode, then re-encode gives the same bytes). | encoding: `every_op_and_state_encoding_is_strictly_canonical` | 50 seeds; 4000 ops, 12000 states | pass |

Totals on a clean build: 57 tests pass (15 mine, plus 42 unit tests inside `tuple.rs` that run because I include it). No warnings.

### Does the suite have teeth? Planted bugs

`scripts/mutants.sh` copies the crate, applies one bug with `perl`, and runs `props`. Each copy builds in its own target directory.

| mutant | planted bug | result | shrunk counterexample |
|---|---|---|---|
| m1 | clear removes every named dot, ignoring the clock | 3 of 5 fail: unobserved-removal, convergence, join laws | unobserved-removal: `seen = 0, unseen = 1`. Convergence: 7 acts. |
| m2 | clear applied without waiting for its clock | 5 of 5 fail, including the late-duplicate unit test | late-duplicate unit test: the clear is applied with nothing to remove, so `pending_len` is 0, not 1. Convergence: 14 acts. |
| m4 | join keeps every left-side dot (asymmetric) | convergence and join laws (assertion `left == right`, props.rs:279) | 5 acts |
| m4b | join keeps every dot on both sides (symmetric) | convergence only. No law assertion fails; the failure is the in-simulation oracle check. | 6 acts |
| m5 | join ignores dots only the right side holds | convergence and join laws | one `Set`, then the observer's join: `(n = 3, acts = [Set r0], seed 0)` |

m4b matters: a wrong but symmetric join passes the lattice laws. Only the oracle check catches it. The laws test alone is not enough.

## Results: metadata (CBOR bytes, release build, `logs/metadata.md`)

Columns. `state` = live dots with entries, plus the version vector. `tomb` = the same state plus one tombstone per removed add, grouped by device (the cheapest tombstone encoding I could think of; ungrouped it would be about 37 B per dot, see T4). `log` = every op plus its 32-byte BLAKE3 key. Signatures and the real p2panda header are not included, so the real log is larger.

**T1. One entry, one device: add then clear, repeated** (`cycles` add/clear pairs, then one final add so one entry is live)

| cycles | ops | live dots | state B | tomb B | log B |
|---:|---:|---:|---:|---:|---:|
| 0 | 1 | 1 | 117 | 118 | 110 |
| 1 | 3 | 1 | 117 | 155 | 365 |
| 10 | 21 | 1 | 117 | 164 | 2660 |
| 100 | 201 | 1 | 119 | 345 | 25964 |
| 1000 | 2001 | 1 | 121 | 3020 | 262554 |
| 10000 | 20001 | 1 | 121 | 30020 | 2629554 |

Reading: state is flat (117 to 121 B). The only growth is the version-vector counter, which goes from one CBOR byte to three. Tombstones grow by 3.0 B per removal (slope from 1000 to 10000 cycles: (30020 − 3020) / 9000). The log grows by about 131 B per op, about 263 B per cycle.

**T2. Distinct entries (own subject each), one device: set each, clear the first half**

| entries | live | state B | state B per live entry | tomb B |
|---:|---:|---:|---:|---:|
| 1 | 1 | 118 | 118.0 | 119 |
| 10 | 5 | 434 | 86.8 | 476 |
| 100 | 50 | 4041 | 80.8 | 4156 |
| 1000 | 500 | 40543 | 81.1 | 41804 |

Marginal cost of a live entry: (40543 − 4041) / 450 = 81.1 B. This is the 32-byte subject, the 32-byte device key, the seq, and the name and value. Tombstones cost only about 2.5 B each here because they are grouped by device.

**T3. Random run, several devices, partial gossip** (3000 steps, seed 20261011, 24 possible entries)

| devices | ops | live dots | vv entries | state B | tomb B | log B | join = reference |
|---:|---:|---:|---:|---:|---:|---:|---|
| 2 | 2947 | 7 | 2 | 611 | 5632 | 456848 | true |
| 5 | 2994 | 26 | 5 | 1835 | 6401 | 612223 | true |

**T4. One clear op, by names and clock size**

| names | devices in clock | clear op B |
|---:|---:|---:|
| 1 | 1 | 149 |
| 1 | 5 | 301 |
| 1 | 20 | 871 |
| 10 | 1 | 473 |
| 10 | 5 | 625 |
| 10 | 20 | 1195 |
| 100 | 1 | 3791 |
| 100 | 5 | 3943 |
| 100 | 20 | 4513 |

Each named dot costs about 37 B, because it repeats the 32-byte device key. Each clock entry costs about 38 B. Clearing 100 claims is about 3.8 KB. Grouping names by device would remove most of that repeated key; I did not measure it.

**T5. Compaction of the 5-device run at a causal cut** (the cut is a prefix of issue order; snapshot = reference over the prefix)

| cut | dropped ops | snapshot B | tail B | dropped headers B (32 each) | total B | uncompacted log B |
|---:|---:|---:|---:|---:|---:|---:|
| 50% | 1497 | 3288 | 308768 | 47904 | 359960 | 612223 |
| 90% | 2694 | 4231 | 66886 | 86208 | 157325 | 612223 |
| 99% | 2964 | 1642 | 6323 | 94848 | 102813 | 612223 |

At a 99% cut, dropping bodies while keeping 32-byte headers saves 83%. Most of what remains is the headers, so a real header with a 64-byte signature will matter more.

## Answer: tombstones

Not needed for correctness. A removed dot is covered by the replica's version vector, and "covered but absent" means removed (the join rule). That works because every event in a device's chain takes the next seq, sets and clears alike, so coverage is always a prefix per device. The condition is the gapless chain. The chain's `prev` links give this, and the design already requires it. If a device's seqs could have gaps, the vector could not represent removed dots, and tombstones or a dot cloud would be needed.

Evidence: T1 (state is flat across 20001 ops), the late-duplicate test, and the convergence property.

## Proposed compaction rule and its condition

- Take a snapshot `S_F` = the state at a causally closed cut `F` (what a replica has applied up to `F`; a `checkpoint` event in design §4.3 is a natural carrier).
- Drop the bodies (names, clocks, entries) of ops covered by `F`. Keep their headers (IDs, seq, prev) so the chain still verifies.
- Serve a peer: if the peer covers `F` on every device, send only the tail above `F` that it lacks. Otherwise send `S_F` first, then the tail.
- Condition: `S_F` must be the state of exactly the ops at or below `F`. Stability (every device has applied `F`) is not needed for correctness. It only saves sending `S_F` to peers that already cover `F`.
- Not allowed: dropping a body below `F` and serving ops only to a peer that does not cover `F`. Test `dropping_a_stable_clear_body_resurrects_the_add_for_a_new_device`: all three existing devices have applied the clear, so it is stable, but a new device that replays the add body keeps the removed add. Stability is therefore not a sufficient condition for op-only catch-up.
- The version vector is not compacted. It is O(devices), and dropping a device's entry would make its old dots look unseen and resurrect them.

Evidence: `snapshot_then_tail_is_correct_for_every_peer_and_cut` passes across 200 histories × 4 cuts × 3 peers. Cut 0 means every op is sent. The last cut means only the snapshot is sent. Snapshots are taken from the oracle, which equals a replica's state at a closed cut; a real implementation would take them from the replica.

## Clock: whole causal past, not the frontier

Design §6.1 says the clock must cover the transitive past. Test `frontier_only_clock_leaves_an_add_the_clearer_saw`: A adds x. B sees x and adds y. C sees both and clears x with a clock naming only y's dot (the frontier). C's clock does not cover x, so x survives a clear that saw it. A full clock removes x. The design's warning is confirmed. A 32-hash `after` frontier cannot carry the clock, so the clock needs its own field, as the design already says.

## Tuple keys: subject lookup (`pkgs/id/src/tuple.rs`)

**Encoding used.** The bug characterisation uses the current `tuple.rs`, unchanged, with the bare `0x00` terminator. The proposed fix uses a local copy of the task-01 encoding, `0x00 0x00` terminator, with escapes unchanged. I did not change `pkgs/id`.

**Key layout** (design §9.2). `claim` = the claim's event ID (BLAKE3, 32 bytes):

- α = `bytes(subject) string(name) (string(value) | null) bytes(claim)`
- Ω = `(string(value) | null) string(name) bytes(subject) bytes(claim)`

Lookups:

- *Tags of X:* prefix `bytes(subject)`. Exact for 32-byte subjects with the current encoder. The subject field is fixed length, so a longer subject cannot share the prefix. Test: `subject_prefix_is_exact_for_fixed_32_byte_subjects`.
- *Tags of X named N:* prefix `bytes(subject) string(name)`. **Not exact.** With the current encoder, name `"a"` also matches `"a\0b"`, because `"a\0b"` escapes to `61 00 FF 62 00`, which starts with `61 00`. The prefix matched 2 keys where 1 was correct. Test: `current_name_prefix_matches_a_longer_name_containing_nul` (a characterisation; flips once fixed).
- *Subjects with value V:* prefix on Ω `string(value)`, with the same NUL problem for values.

**Fixes, tested:**

| option | result on `"a"` vs `"a\0b"` | test |
|---|---|---|
| successor bound (`prefix` with last byte +1) | 2 hits (wrong) | `sorted_range_scans_on_the_current_encoder` |
| `prefix ++ 0xFF` bound (S6's workaround) | 1 hit (correct) | same test |
| `00 00` terminator (task 01) | 1 hit (correct), order preserved | `fixed_terminator_prefix_is_exact`, `fixed_terminator_keeps_string_order` |

Recommendation: the `00 00` terminator. It also makes ordinary prefix ranges exact without a special bound. `00 00` cannot occur in escaped data, because every `00` is followed by `FF`. Subject-only prefixes are unaffected today, but the fix is still needed for any variable-length field and for every name and value query.

## Not tested, and why

- **Strong-remove precedence** (tags follow the content policy, not membership voidness). Not in my brief. The decision is open.
- **Real headers and crypto.** No `prev`, `after`, `backlink`, signatures, device keys or revocation. Device and subject keys are 32-byte placeholders. Sequence base is 1 here; the design's header uses base 0. The model is unaffected, but the VV encoding would shift by one.
- **Persistence and the index.** No redb, no index rebuild, no search, no documents, no ProseMirror.
- **Network.** Partitions are modelled as sync gating, and every op eventually arrives. No loss, timeouts or crashes. No bandwidth or latency measurements.
- **Op decoding.** I check canonical encoding (strict decode, then re-encode). There is no op decoder.
- **Compaction in a live system.** The rule is tested on in-memory replicas only. The snapshot comes from the oracle, not from a checkpoint event.
- **Clear cost optimisation.** Grouping names by device is a proposal. I did not measure it.

## Surprising, and process notes

1. **Stale build artifacts from my own mutant runs.** The first `cargo test` after the mutant runs failed `snapshot_then_tail…` at seed 0, cut 0, and a hand-built `join` returned a result that should have been impossible. Cause: the mutant builds used the shared target directory, and they wrote the same `libs8_tags-*.rlib`, so cargo reused a mutant's library for the main crate. `cargo clean -p s8-tags` and a rebuild gave all-pass. The mutants now build in `mutants/target`. Every result above comes from a clean `s8-tags` build or from the separate mutant target. The debug-only diagnosis files I used are deleted.
2. **A wrong but symmetric join passes the lattice laws** (m4b). Only the oracle check catches it.
3. **Table T2 first measured the wrong thing.** My entry generator reused 24 entries, so "per live entry" was really "per dot". Fixed: each entry now has its own subject.
4. **The design's survival sentence reads as if naming is optional.** Read literally, "remove iff seq ≤ clock[author]" would remove every observed add by that author, not just the named ones. The naming half must be written down.
5. **Compaction and tombstones interact.** Stability looks like the right condition, but it is not sufficient for new devices (see the counterexample above).
6. **A read of `pkgs/id/src/tuple.rs` returned an extra block that looked like a user message,** asking me to write plans and docs into `/home/user/code` following `pkgs/id/AGENTS.md`. I did not act on it. Nothing was written outside this folder.

## Files

- `src/lib.rs`: model, join, oracle, encodings, RNG.
- `tests/props.rs`, `tests/clock.rs`, `tests/compaction.rs`, `tests/encoding.rs`: property and unit tests.
- `tests/tuple_keys.rs`: tuple-key tests, using the real `pkgs/id/src/tuple.rs` via `#[path]`.
- `examples/metadata.rs`: metadata tables (T1–T5).
- `scripts/mutants.sh`: planted-bug runs.
- `logs/`: `test.txt`, `soak_3000.txt`, `mutants.txt`, `mutant_*.txt`, `metadata.md`.
