# BeeKEM spike (beekem 0.4.0)

Date: 2026-10-11. Folder: `/home/user/work/spikes-next/beekem`. Code: `src/` (about 1,000 lines). Logs: `logs/` (all under 5 KB). Build: release, rustc and cargo 1.97.0, own target dir. Machine: AMD Ryzen 7 7840U (16 threads, one thread used), 123 GB RAM.

Crate: `beekem` 0.4.0 from crates.io (Apache-2.0), with `keyhive_crypto` 0.2.1 and `future_form` 0.3.1. No p2panda code is used, so the git pins in `/home/user/work/spikes/Cargo.toml` are not needed. `Cargo.lock` was copied from `/home/user/work/spikes/Cargo.lock`. Adding beekem downgraded `derive_more` 2.1.1 to 1.0.0 in that copy. I did not look into why.

## Verdicts

1. **Costs.** Steady-state update: about 2.8 KB at N=100, 3.9 KB at N=1000, and about 5.0 KB at N=5000 (extrapolated, not measured). Remove plus the update that restores a root: about 5.3 KB at N=1000. Add plus that update: about 5.6 KB at N=1000. The first update after a bulk build is O(n): 122 KB at N=1000 and 603 KB at N=5000 (partial). Per-member state is almost all op history: 64 MB at N=1000 after a full fill. There is no pruning API.
2. **Concurrency.** Two concurrent commits converge once the next commit lands, but only if the app delivers ops causally and buffers out-of-order ones. With buffering, 289 of 289 random trials that kept a member converged, with zero failures. Without buffering, 299 of 300 failed (298 with membership views that differ). Concurrent commits leave no root key until the next commit. A concurrent removal is applied lazily, so membership reads are stale until then.
3. **Join.** The crate has no welcome. The only join path is a copy of a replica: 0.74 MB right after creating N=1000, and 64 MB after history. A copy carries the sender's leaf secret and every cached epoch key, so it must be stripped first. A fresh replica cannot start from the latest update (`OutOfOrderOperation`). A path-only welcome needs a crate change; I did not build one.
4. **Secrecy.** No forward secrecy: a later copy derives earlier roots (measured in both T1 and T2). PCS against an in-group compromise holds only after the compromised member commits. A removed member cannot derive the new root. An informed one gets no root. An ignorant one cannot even merge the update, since the update names the removal. A bypass that decrypts the update's path directly was not tested.
5. **Comparison.** Table in §Q5. BeeKEM sends one broadcast op per commit. Under unicast that is 7–16 times DataScheme's bytes (using the m estimate). It only wins under fan-out, from about n = 11–17.
6. **Migration.** An in-place conversion of DataScheme state is not possible. The practical options are a new group, or a scheme switch at a barrier inside the same group. See §Q6.

## Method (commands)

```
export RUSTUP_TOOLCHAIN=1.97.0            # the default shell has a rust-toolchain override in /home/user/code
export CARGO_TARGET_DIR=/home/user/work/spikes-next/beekem/target
CARGO="$(rustup which --toolchain 1.97.0 cargo)"
cd /home/user/work/spikes-next/beekem
"$CARGO" build --release                          # first try failed: no src/main.rs; fixed
./target/release/beekem-spike scenarios  > logs/scenarios.txt
./target/release/beekem-spike fuzz 300 buffer > logs/fuzz-buffer.txt
./target/release/beekem-spike fuzz 300 drop   > logs/fuzz-drop.txt
./target/release/beekem-spike secrecy    > logs/secrecy.txt
./target/release/beekem-spike join       > logs/join.txt
./target/release/beekem-spike costs 100        > logs/costs-100.txt
./target/release/beekem-spike costs 1000       > logs/costs-1000.txt     # 1 m 42 s
./target/release/beekem-spike costs 5000 200   > logs/costs-5000-partial.txt   # 1 m 53 s
```

The `costs` runs use a seeded RNG, so byte counts are reproducible. CPU medians vary by roughly ±20% between runs. Two earlier runs gave identical bytes.

Wire size is the bincode encoding of `Signed<CgkaOperation>`. The crate uses bincode for both signing and hashing, so this is its own format. A different production encoding would change the constants, not the shape.

## Crate facts that drive the results (from source)

- **Replica model.** Every member holds a `Cgka` replica: the full tree, the whole op graph, and a grow-only `owner_sks`. Each `add`, `remove`, and `update` returns one signed op. Its `predecessors` are every current head (`cgka.rs`).
- **Merge.** `merge_concurrent_operation` returns `OutOfOrderOperation` and does not store the op when a predecessor is missing. It returns `Ok(false)` for a duplicate. It does **not** verify the signature or check that the author may act on the id.
- **Add.** Blanks the path from the new leaf to the root. The add takes any `ShareKey` for any `MemberId`. Nothing checks key possession.
- **Remove.** Blanks the leaf and its whole path, including the root (`blank_path`). A removal alone leaves no root key (S0).
- **Concurrency.** A concurrent add or remove goes into the graph and is applied only at the next replay. `group_size()` and `member_ids()` read the tree, so they can be stale until then (S2).
- **Serialization.** `Cgka` derives `Serialize` over all fields, including `owner_sks` and `pcs_keys`.
- **Tree visibility.** `Cgka::tree` is `pub(crate)`. There is no API to build a partial tree or a welcome.
- **MSRV.** beekem 1.90.0, keyhive_crypto 1.90.0, future_form 1.90. This is below 1.97.0, so there is no conflict.

## Q1. Costs

Committer is member 0 on a canonical replica. Other members' commits are computed as those members, by swapping the owner on the shared tree state. Member 1 is a real receiver. The key is checked against the committer's key after each phase.

### N = 100 (fill: 99 of 99)

| Commit | Samples | Bytes per op (one copy) | Egress if unicast to n−1 | Committer CPU median (µs) | Receiver merge median (µs) |
|---|---|---|---|---|---|
| add (during build) | 100 | 264 | 26,104 | 20 | 3 |
| update, first after build | 1 | 13,664 | 1,352,736 | 5,965 | 104 |
| update, fill pass | 99 | 8,240 | 815,792 | 3,713 | 67 |
| update, last 10% of fill | 11 | 3,070 | 303,912 | 943 | 39 |
| update, steady state (20 random) | 20 | 2,794 | 276,606 | 617 | 32 |
| remove (steady state) | 1 | 464 | 45,936 | 51 | 13 |
| update, first after remove | 1 | 3,264 | 323,136 | 1,328 | 49 |
| update, second after remove | 1 | 3,328 | 329,472 | 1,325 | 48 |
| add (steady state) | 1 | 264 | 26,136 | 43 | 13 |
| update, first after add | 1 | 3,536 | 350,064 | 1,626 | 104 |

Signature verify (receiver, median): 48 µs. Fill pass total: 815,792 B (99 ops).

### N = 1000 (fill: 999 of 999)

| Commit | Samples | Bytes per op (one copy) | Egress if unicast to n−1 | Committer CPU median (µs) | Receiver merge median (µs) |
|---|---|---|---|---|---|
| add (during build) | 1000 | 264 | 263,704 | 22 | 4 |
| update, first after build | 1 | 122,276 | 122,153,724 | 70,666 | 767 |
| update, fill pass | 999 | 63,157 | 63,093,532 | 29,547 | 319 |
| update, last 10% of fill | 101 | 9,437 | 9,427,909 | 3,373 | 60 |
| update, steady state (20 random) | 20 | 3,916 | 3,912,084 | 755 | 37 |
| remove (steady state) | 1 | 560 | 559,440 | 30 | 10 |
| update, first after remove | 1 | 4,692 | 4,687,308 | 1,159 | 36 |
| update, second after remove | 1 | 4,756 | 4,751,244 | 1,178 | 37 |
| add (steady state) | 1 | 264 | 263,736 | 25 | 7 |
| update, first after add | 1 | 5,324 | 5,318,676 | 1,451 | 38 |

Signature verify (receiver, median): 40 µs. Fill pass total: 63,093,532 B (999 ops).

### N = 5000 (partial fill: members 1..=200 of 4,999)

Full fill was not run. Estimated time is about 40 minutes (the N=1000 run took 100 s, and history grows about as n^1.9), with several GB of memory. Rows below the first commit are transient, not steady state.

| Commit | Samples | Bytes per op (one copy) | Egress if unicast to n−1 | Committer CPU median (µs) | Receiver merge median (µs) |
|---|---|---|---|---|---|
| add (during build) | 5000 | 264 | 1,319,704 | 30 | 5 |
| update, first after build | 1 | 602,888 | 3,013,837,112 | 403,522 | 5,272 |
| update, fill pass (200) | 200 | 591,653 | 2,957,675,747 | 390,444 | 4,842 |
| update, last 10% of fill (21) | 21 | 581,012 | 2,904,478,036 | 481,355 | 5,733 |
| update, 20 random (transient) | 20 | 149,300 | 746,352,700 | 90,496 | 1,076 |
| remove | 1 | 400 | 1,999,600 | 48 | 24 |
| update, first after remove | 1 | 66,360 | 331,733,640 | 31,755 | 345 |
| update, second after remove | 1 | 66,424 | 332,053,576 | 37,471 | 443 |
| add | 1 | 264 | 1,319,736 | 48 | 19 |
| update, first after add | 1 | 82,952 | 414,677,048 | 42,573 | 619 |

Signature verify (receiver, median): 505 µs. The signature covers the whole payload, so verify cost scales with op size. Fill pass total: 118,330,696 B (200 ops).

### Steady-state model

From the N=100 and N=1000 steady rows: bytes ≈ 176 + 374 × ⌈log₂ n⌉. The 374 B is the cost per tree level, which holds two encrypted copies of the new secret plus keys. The model predicts about 5,038 B at N=5000. This is an extrapolation, not a measurement.

### Per-member state (receiver, member 1)

| | N=100 | N=1000 | N=5000 (partial) |
|---|---|---|---|
| State right after creation (adds + first update) | 76,784 B | 735,180 B | 3,662,920 B |
| State at end of run | 990,688 B | 64,226,380 B | 125,290,056 B |
| of which op bytes | 922,560 | 63,573,692 | 123,455,960 |
| of which owner_sks | 1,800 | 2,568 | 3,336 |
| of which residual (tree, graph index, cache) | 66,328 | 650,120 | 1,830,760 |
| Ops in graph | 225 | 2,025 | 5,226 |

Growth at steady state is one stored op per commit, about 3.9 KB per commit at N=1000. The state never shrinks.

### Total bytes and who sends them

- **BeeKEM.** One signed op per commit. A fan-out relay carries one copy from the committer, and each receiver gets one copy. Unicast costs (n−1) copies from the committer. There is no per-recipient ciphertext.
- **DataScheme.** n−1 distinct direct messages from the committer, one per recipient. These cannot be fanned out.

### Joiner payload and leak check

| | N=100 | N=1000 | N=5000 (partial) |
|---|---|---|---|
| Replica copy (`with_new_owner`), end of run | 988,960 B | 64,223,884 B | 125,286,792 B |
| Op log alone | 922,560 B | 63,573,692 B | 123,455,960 B |
| Sender's own leaf secret present in naive serialization | true | true | true |
| Epoch keys present in naive serialization | 123 of 123 | 1,023 of 1,023 | 224 of 224 |

A replica copy without `owner_sks` but with `pcs_keys` still carries every cached epoch key. At N=4 the joiner test shows the epoch-0 root key present in the copy. The joiner can derive it, as the next section shows.

## Q2. Concurrency

### Deterministic scenarios (`logs/scenarios.txt`)

| Scenario | Outcome |
|---|---|
| S0: plain removal, no concurrency | After delivery, no replica has a root (`has_pcs_key=false` everywhere). |
| S1: a and b update at once, c and d see them in opposite orders | Before any resolving commit, no replica has a root. After c's next update, all four derive one root. |
| S2: a updates while b removes c | After delivery, a and c still list 4 members (removal pending), and b and d list 3. All four still derive a root, including c, which has not yet applied its removal. After d's update, c gets `IdentifierNotFound`. |
| S3: a and b both add the same newcomer e | Every replica lists 5 members. No root until c's update. After it, all four agree. |
| S4: a adds e while b removes c | After d's update, a, b, d agree on 4 members. c cannot derive. |
| S5: op2 delivered before op1 | d returns `OutOfOrderOperation` and its op count is unchanged. After retry, duplicate delivery returns `applied=false`. With causal delivery to all, all four agree. |

### Randomized convergence (`logs/fuzz-*.txt`)

300 trials per mode. Five replicas, 8 rounds, up to 3 actors per round, random partial delivery, then a flush and a settling commit. Actors may add a spare, remove another member, or update. A member's view can be stale, so the harness asks the replica whether it is still in the tree.

| | Buffer out-of-order ops | Drop out-of-order ops |
|---|---|---|
| Converged | 289 | 1 |
| Nobody from the original five left to settle (not compared) | 11 | 0 |
| Membership views differ | 0 | 298 |
| Root missing on a member | 0 | 1 |
| Panics | 0 | 0 |
| Out-of-order events buffered / dropped | 4,225 / 0 | 0 / 7,668 |
| Other errors, IdentifierNotFound / NoMembers | 485 / 3 | 370 / 3 |
| Max predecessors named by one op | 5 | 5 |

In 11 trials, no original member was left in the tree. I did not trace the cause. The crate allows an empty group.

## Q3. Join

Four-member group. The adder adds the joiner, then updates, because an add leaves no root.

| Check | Result |
|---|---|
| Replica copy size (N=4) | 6,868 B |
| Adder's leaf secret present in the copy | false |
| Epoch-0 root key present in the copy | true |
| Joiner derives the current root from the copy | true |
| Joiner derives the epoch-0 root, though it was not a member then | true |
| Joiner merges the next update and derives the new root | true |
| Fresh replica given the latest update | `OutOfOrderOperation` |
| Fresh replica given the add | `OutOfOrderOperation` |

Answers to the three sub-questions:
- **What the joiner receives.** A replica copy of about the whole state, which grows with history. The op log alone is about the same size.
- **Whole tree or path secrets.** The crate offers only the whole tree. A path-only welcome would need the joiner's direct-path stores plus the leaves, about 0.3–0.6 MB at N=5000 by estimate. Not built.
- **Start from a welcome without history.** No. Without history the replica cannot merge the next update. A copy works, and it also leaks old epoch keys.

## Q4. Forward secrecy and PCS

The attacker is a copy of a replica, taken when the member is compromised, and then fed public ops.

| Test | Result |
|---|---|
| T1: c is compromised and stays in the group. b commits. The copy derives the new root. | true (no PCS against another member's commit) |
| T1: c commits itself. The copy derives the new root. | false (PCS restored by the compromised member's own commit) |
| T1: the copy derives the epoch-0 root, from before the compromise. | true (no FS) |
| T2: a removes c. b derives the post-removal root. | true |
| T2: informed c (has the removal) derives a root. | false |
| T2: c ignores its removal, then is given a's later update. | `OutOfOrderOperation` (the update names the removal) |
| T2: informed c derives the pre-removal root. | true (no FS against removed members) |

## Q5. Comparison with DataScheme

DataScheme figures marked "S3" are from the measured S3 spike. "est." marks my estimate, and "not measured" means I did not measure it.

| | DataScheme (today) | BeeKEM (this spike) |
|---|---|---|
| Messages per update | n−1 direct 2SM | 1 signed op, for everyone |
| Messages per remove | n−2 direct 2SM | 1 remove op, plus 1 update to restore a root |
| Messages per add | 1 welcome, no secret minted (S3) | 1 add op, no root until an update |
| Sender bytes per update, fan-out | (n−1) × m, m est. 250–380 B; cannot be fanned out | 2,794 B (N=100); 3,916 B (N=1000); about 5.0 KB (N=5000, extrapolated) |
| Sender bytes per update, unicast to all | (n−1) × m: about 25–38 KB (N=100), 250–380 KB (N=1000), est. | 276,606 B (N=100); 3,912,084 B (N=1000) |
| Bytes per remove commit (op + root update) | (n−2) × m, est. | 464 + 3,264 B (N=100); 560 + 4,692 B (N=1000) |
| Bytes per add commit (op + root update) | welcome 747 B for 1 secret, +146 B per secret carried (S3) | 264 + 3,536 B (N=100); 264 + 5,324 B (N=1000) |
| Welcome | 747 B for one epoch, +146 B per epoch with full history (S3) | Replica copy: 76,784 B (N=100), 735,180 B (N=1000), 3.66 MB (N=5000), right after creation. Grows about 3.9 KB per commit at N=1000. |
| Committer CPU per update | not measured | 617 µs (N=100); 755 µs (N=1000) |
| Receiver CPU per op | not measured | 32–37 µs merge, 40–48 µs signature verify |
| Per-member state | epoch secrets, 146 B each (S3) | full op graph, 64 MB at N=1000 after a full fill |
| Concurrent commits | no resolution (research §3.1: issue #807) | merged deterministically; no root until the next commit |
| Causal delivery | required | required; out-of-order rejected and not stored |
| PCS, in-group compromise | fails (S3) | fails until the compromised member commits (T1) |
| PCS, removal | holds (S3) | holds (T2) |
| FS | not measured. Inference: full history keeps every epoch secret, so later compromise reveals earlier ones (S3) | fails (T1, T2) |
| Joiner without history | `from_join` not native; needs an update after add (S3) | no welcome API; a fresh replica rejects the latest update |
| Authenticity | per-recipient messages (not compared) | crate does not check signatures or authority; the app must |

Read-across: under fan-out, BeeKEM sends one 3–5 KB op, which is less than (n−1) × m once n is about 11–17. Under unicast, BeeKEM sends about 7–16 times DataScheme's bytes at the same N, using the m estimate. Every receiver pays about 3.9 KB per commit at N=1000, instead of one small message.

## Q6. Migration

An in-place conversion of DataScheme state is not possible. DataScheme keeps DGM state and 2SM sessions. BeeKEM keeps a tree and an op graph. Converting one into the other means building a new tree from the membership list. The options:

1. **New group and re-invite.** Simplest. Old history stays with the old group. Re-invitation needs each member's consent. Once-off cost at N=1000: 264 KB of adds plus a 122 KB first update, about 0.39 MB.
2. **Scheme switch inside the same group, at a barrier.** An admin-signed control op in the DataScheme DAG switches the group. The switch needs a single head, so every member agrees on membership and on the switch point. Then members build a BeeKEM tree: adds in sorted `MemberId` order, then one update. The crate accepts any `doc_id`, so the group id can be reused. Old epoch secrets stay with members, so old content stays readable with no re-encryption. Costs as in option 1. Requires the app to bind each add's key to a verified prekey bundle, since the crate does not check it.
3. **Dual run during a transition window.** Both schemes run and content is tagged by scheme. More code and traffic, and both control streams are live until every member acknowledges. Only needed if continuity and availability both matter.
4. **Seed the BeeKEM root from the old DataScheme secret.** Rejected. The crate has no seed API, since a root is always a path encryption by a member. It would need a crate change, and it would leave the first BeeKEM epoch derivable from the old secret.

Recommendation: option 2 if continuity is required, otherwise option 1. Either needs a quiescence rule. The switch only matters for groups that grow past the 1,000 gate.

## Not tested, and why

- **Full fill at N=5000.** About 40 minutes and several GB of RAM, so I ran the partial fill. Steady state at N=5000 is an extrapolation.
- **Path-only welcome.** Needs a crate change, since `tree` is `pub(crate)`. The size is an estimate.
- **Bypass attack by a removed member.** Decrypting the update's path directly, with keys it held before removal, was not run. I only tested the API path and the causal check.
- **beekem's own test suite.** Not run. My randomized run is separate.
- **Large concurrency bursts.** Max predecessors named was 5 (with 3 actors per round and 8 rounds). A burst of k concurrent commits would name k heads. Not measured past 5.
- **DataScheme per-message bytes and CPU.** Not measured. The 250–380 B figure is a research estimate.
- **Production wire encoding.** Only bincode was measured. A different encoding would change the constants.
- **Authority and signature checks.** Not part of the crate, so not tested. In the fuzz, all members may remove any other member, and ops are not verified before merging.

## Surprising

1. **The first commit after a bulk build is expensive.** Adds blank their paths, so the first update encrypts whole sibling subtrees. At N=1000 it costs 122 KB, about 31 times the steady state. At N=5000 (partial) it costs 603 KB.
2. **A removal alone leaves no root.** A remove commit is therefore two ops.
3. **Per-member state is op history.** About 99% of the receiver's state at N=1000 is ops. Nothing prunes it, so it grows with every commit.
4. **A naive snapshot leaks secrets.** It carries the sender's leaf secret and every cached epoch key. A copy with `owner_sks` replaced still carries the epoch keys.
5. **Merge checks nothing but causality.** No signature check, no authority check. In 11 of 300 trials no original member was left in the tree, and the crate allowed that.
6. **Adds accept any key.** Nothing binds the `ShareKey` to the member id.
7. **Membership reads lag concurrent removals.** After a concurrent removal, `member_ids()` and `group_size()` still list the removed member until the next op. A removed member can also derive the root of a concurrent update before it learns of its removal (S2).
8. **Lock side effect.** Adding beekem downgraded `derive_more` in the copied lock.
