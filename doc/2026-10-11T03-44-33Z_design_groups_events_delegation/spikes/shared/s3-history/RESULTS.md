# S3: joiner history from the current epoch, and key-agreement ordering

## Question

Can DataScheme welcome a joiner with only the current epoch (`from_join`)? Is `full` native? Is post-compromise security (PCS) confirmed? How many dependencies does `Ordering` make a control message name, and can that exceed 32? How do pre-key lifetimes behave?

## Setup

- p2panda-encryption 0.7.1 at p2panda rev `3e84cac48bba05c6857cce2ec433862b97f8d53c`, from the cargo git checkout. Features `data_scheme`, `message_scheme`, `test_utils`. p2panda-core at the same rev.
- rustc and cargo 1.97.0 from rustup. Debug profile.
- Ordering is the crate's `MessageOrderer`, public under `test_utils`. Membership is `MemberDgm` (a copy of `TestDgm` with a fix, see below) in `base`, and stock `TestDgm` in `patched`.
- `s3-history/src/harness.rs` is included twice in `main.rs`: as `base` (unmodified crate) and `patched` (path copy). Both run the same scenarios.

## Commands run

```
cargo build -p s3-history
cargo run -q -p s3-history -- base       # logs/run-base-final.txt
cargo run -q -p s3-history -- patched    # logs/run-patched-3.txt
diff -ru <checkout>/p2panda-encryption patched/p2panda-encryption > logs/patch.diff
```

`logs/run-base-upstream-testdgm.txt` is the stock-`TestDgm` run that shows the membership bug.

## Measurements

### (a) Full history and PCS (base)

- Dan reads m1 from the welcome. Full history is native.
- After the welcome Dan is a member (`members=[0,1,2,3]`) and holds one secret.
- Carol processes the remove and gets `Removed`. She cannot read m2 (`UnknownGroupSecret`). An attacker holding Carol's pre-removal state cannot read m2 either.
- **In-group compromise is not covered.** An attacker holding a copy of a member's state, where the member stays in the group, reads the message sent after an `update` (scenario a2). Design §8.1 and §8.4 say a compromised member cannot read content after a rotation. That holds only for removed members.

### (b) Welcome contents

The welcome is one direct message. Its dump is `logs/a-full-history.diag`.

- **Plaintext:** `recipient` (3). `history`, the DGM state: `my_id` 0 (the adder) and `members` [2,1,0], the adder's membership before the add. The X3DH `identity_key` (sender), `ephemeral_key`, `prekey_id` (the joiner's), and `key_used`.
- **Encrypted:** `ciphertext`, an AEAD over `TwoPartyPlaintext { plaintext: <SecretBundle bytes>, receiver_new_secret, sender_new_verifying_key, sender_next_index }` (`x3dh.rs:110`, `two_party.rs:161`).
- **Full bundle:** the joiner's bundle holds every epoch secret. Adder bundles of 1, 4, 8 and 16 secrets give the joiner 1, 4, 8 and 16.
- **Size:** 747 bytes with one secret, 2,929 with sixteen: about 146 bytes per secret, against about 40 bytes raw. `TwoPartyPlaintext.plaintext` and `X3dhCiphertext.ciphertext` are `Vec<u8>` with no `serde_bytes`, so each byte costs one or two. HPKE's fields use `serde_bytes` (`hpke.rs:18,22`).
- **The add mints nothing.** The adder's bundle is unchanged by `add` (1 to 1, 4 to 4, 8 to 8, 16 to 16).
- Sizes vary by a few bytes between runs. I did not chase the cause.

### (c) from_join

**Feasible without a fork.** `Dcgka::add` takes the bundle as an argument. `EncryptionGroup::add` is the only caller that passes the full `&y.secrets` (`group.rs:127`).

- Unmodified crate, public APIs only (`base::add_from_join`, about 20 lines): `Dcgka::add(y, joiner, &SecretBundle::init(), rng)`, then the local steps `EncryptionGroup::add` runs.
- After the welcome Dan's bundle is empty. Dan cannot read m1, or a message sent between add and update. After `update`, Dan holds one secret, not Alice's pre-add secret. Dan reads m2 and still cannot read m1. The patched copy gives the same results.

Patch size (copy against checkout, `logs/patch.diff`):

- `group.rs`: 25 lines added, none changed. The method calls the module-private `process_local`, as `add` does. No visibility changed.
- `dcgka.rs`: 2 lines changed and 8 added, the membership fix below. It changes only the private `process_welcome` signature.
- `Cargo.toml`: 8 lines, build plumbing only (`workspace = true` became explicit versions).

What from_join needs beyond the method:

1. An `update` straight after the add. The add mints no secret, so messages sent in between use the old one.
2. The joiner's orderer must withhold pre-join application messages. If one is delivered, the welcome fails with `UnknownGroupSecret` (scenario c1). `receive` returns no state on error (`group.rs:464`), so the joiner's state is lost. Upstream should yield an undecryptable output instead. Not built.

### Membership bug (found in a and c)

`Dcgka::add` clones the adder's membership into the welcome before the add is applied (`dcgka.rs:267`). The joiner applies the add in `process_add`, then `process_welcome` overwrites its membership with the welcome's copy, which lacks the joiner. Stock `TestDgm::from_welcome` keeps that copy, so the joiner is never a member and never welcomed: `is_welcomed=false members=[0,1,2]`. It processes nothing after the welcome.

The crate does not catch this. Its tests call `Dcgka::add` (`tests.rs:161`, `615`) and check the bundle, not membership. `Network::add` has no callers. The fuzz target covers `message_scheme` only.

The fix, tested in the copy (`dcgka.rs`, 8 lines added and 2 changed): the joiner applies its own add after `from_welcome`. Stock `TestDgm` then welcomes Dan (`logs/run-patched-3.txt`). Until upstream takes it, our DGM's `from_welcome` must insert the joiner, as `MemberDgm` does.

### (d) Ordering dependencies (40 members)

| Case | Message | Named by the crate (per author) | Heads (minimum) |
|---|---|---|---|
| d1: 8 concurrent updates, then a merge | merge | 9 | 8 |
| d2: 40 concurrent updates, then a merge | merge | **40** | **40** |
| d3: 40 sequential updates | update | max 39 (7 over 32) | 1 |
| d3: 40 sequential updates | application | 40 (all over 32) | 1 |

- The crate's `MessageOrderer` names the latest control message from every author it has seen, and never prunes. Once 33 members have each published, every later message names more than 32.
- Heads are the known control messages that no other known message names: the smallest set that still names every last-known control message. They exceed 32 only when 33 or more control messages are concurrent and unmerged, as in d2.
- **Minimum change for a merge or checkpoint:**
  1. Our `Ordering` adapter names heads, not per-author latest. That is enough for causal readiness, because `queue` marks a message ready only when every named dependency is ready (`test_utils/ordering.rs:144-151`), so readiness is transitive. **Untested:** I did not run a heads-naming orderer.
  2. When heads would exceed 32, publish a merge `update` naming up to 32 of them. A second merge names the rest. No crate change, but the app must issue it.
  3. A no-op checkpoint cannot be expressed. `GroupMessageContent` has only `Control` and `Application` (`traits/message.rs`), and `receive` routes both. A checkpoint needs a new crate variant.

### (e) Pre-key lifetime

- Default lifetime is 7,257,600 s, 84 days (`key_bundle/lifetime.rs:9`), not 90.
- Existing 2SM sessions survive expiry: `update` succeeded after Bob's bundle expired, and Bob processed it (his bundle grew to two secrets).
- First contact is blocked: adding Dan with an expired bundle failed with `Dcgka(PreKeyRegistry(KeyBundlesExpired))`.
- The registry rejects an expired bundle on insert: `KeyBundle(Lifetime(InvalidLifetime))`.

## Pass or fail per exit criterion

| Criterion | Result |
|---|---|
| `from_join` feasible | **Pass.** Feasible without a fork. Needs the immediate update, the withholding rule, and the membership fix. |
| `full` native | **Pass.** Dan reads m1 from the welcome, and the welcome carries every epoch. |
| PCS | **Pass for removal. Fail for in-group compromise.** Design §8.1 and §8.4 need correcting. |
| Ordering counts | **Measured.** 40 exceeds 32 under the crate's naming, and under heads in the d2 burst. |

## Deviations

- Removed unused workspace deps from `s3-history/Cargo.toml` to avoid compiling iroh. Kept `p2panda-core`, `p2panda-encryption`, `anyhow`, `hex`, `serde_json`, `cbor-core`. Added `serde` for the membership derive, and the renamed path copy.
- The workspace `Cargo.lock` changed through these edits. I did not run `cargo update`.
- The copy cannot inherit the spikes workspace, so its `workspace = true` lines became explicit versions. A nested `[workspace]` table fails with "multiple workspace roots", so I left it out.
- In scenario c, the joiner receives the control log but not pre-join application messages.

## Blockers and what was tried

- Nothing blocked a verdict. The pinned rev compiled on the first try.
- The first baseline showed Dan decrypting m1 but never processing later messages. That led to the membership bug. A probe that returned `Ok([])` for an unwelcomed joiner made from_join look like a pass until the fix.
- Not done: a heads-naming orderer, a receive that tolerates undecryptable messages, and the `serde_bytes` fix. Described, not built.

## Recommendation

1. Keep `full` as the default. It is native.
2. Implement `from_join` in our layer with the public-API method, or propose the 25-line upstream method. Require an `update` straight after the add, and withhold pre-join application messages in our orderer.
3. Correct design §8.1 and §8.4. PCS holds against removal only. Against a device that stays in the group, the only fix is to remove it and re-add it.
4. Make our DGM's `from_welcome` insert the joiner. File the `dcgka.rs` membership bug upstream with the patch (8 lines added, 2 changed).
5. Ordering: name heads, and publish a merge update before the frontier reaches 32. Prototype the heads orderer before committing.
