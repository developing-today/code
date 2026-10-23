# S10: heads orderer and frontier size in a 40-member group

## Question

Task 05 (heads orderer) and the design's `after` cap:

1. How large is the frontier (the heads an event names) in a 40-member DataScheme group under
   steady posting, concurrent posting, partitions that heal, and an offline member?
2. Merge rule: when a member's frontier reaches 32, its next event is a merge update that names up
   to 32 heads. Does the frontier stay at or below 32 in every pattern?
3. Does the heads orderer keep 40 members converged (every member holds the same secrets and reads
   every application message)?
4. Source check of the crate's orderer: one dependency per author, no pruning, and whether it holds
   messages with missing dependencies for a long time.
5. The smallest cap that works, and whether the merge rule is needed at 40 members.

## Method

### Setup

- p2panda-encryption 0.7.1 at p2panda rev `3e84cac48bba05c6857cce2ec433862b97f8d53c`, features
  `data_scheme`, `message_scheme`, `test_utils`. Same pins as `/home/user/work/spikes/Cargo.toml`;
  `Cargo.lock` copied from `/home/user/work/spikes/Cargo.lock`. Cargo pruned the copy to the 162
  packages this crate uses (the spikes lock has 584). The shared versions are unchanged:
  p2panda-encryption and p2panda-core 0.7.1, serde 1.0.229, serde_json 1.0.152.
- rustc/cargo 1.97.0. Built offline (`--offline`) into the shared target dir
  `/home/user/work/spikes/target`. The first debug run was stopped for speed and is not used.
  Numbers come from `--release`.
- 40 members, `TestDgm` membership, `Lifetime::default()` pre-keys, RNG seed 1, all in-process.
- Nothing outside this folder was written. No network, no nix.

### Commands

```
export CARGO_TARGET_DIR=/home/user/work/spikes/target
CARGO="$(rustup which --toolchain 1.97.0 cargo)"
cd /home/user/work/spikes-next/s10-heads
"$CARGO" build --release --offline
/home/user/work/spikes/target/release/s10-heads > logs/run-all.txt
```

`logs/run-all.txt` is the only result log (157 lines, 27 KB). Every number below is copied from it.

### Orderers

- **crate**: `enc::data_scheme::test_utils::ordering::MessageOrderer`, unchanged.
- **heads**: `src/heads.rs`. Names the heads of the author's view (processed messages that no
  processed message names), sorted by (sender, seq) and truncated to `cap`. Readiness, pending and
  release copy the crate's private `ordering.rs` logic. Options:
  - `track_app`: application messages are heads too ("all events", the design's `after`) or are
    not ("control only", the S3 measure).
  - `backlink`: always name the author's own latest event first, so truncation cannot drop the
    author's own chain. Off by default.

### Driver (`src/main.rs`)

- Events: `create`, `update`, `app` (chat post), and `merge` (an update published because the
  author's frontier was at the cap). Merges come before the author's event, as many as needed to get
  below the cap.
- Merge rule (cap runs): the driver merges while `heads >= cap`. The orderer refuses an application
  message whose frontier exceeds the cap.
- Scenarios (all 40 members, member 0 creates):
  - **a** steady: members post in turn (2 cycles), as app posts and as updates.
  - **b** burst: all 40 publish in one round (concurrent), then each posts once in turn.
  - **b2** storm: `k` rounds in which all 40 publish concurrently, then each posts once in turn.
  - **c** partition: group split into `p` and `40-p`; each side publishes concurrently for `k`
    rounds; the partition heals (every event reaches every member); then each posts once in turn.
  - **d** offline: member 20 misses 10 cycles (39 posts, plus one update per cycle); then it catches
    up in publication order or shuffled, and posts.
  - **e** lost update: member 5's update never reaches member 7, and then everyone keeps posting.
- Delivery: in publication order by default. `reorder`: each member hears each round in its own
  seeded random order (adversarial, the acceptance case).
- Measured per event: named dependencies (from the serialised `previous`), and the author's heads
  before the event (the frontier it would carry uncapped). Per member, after each delivery: heads
  and waiting count. The orderer records how long each waiting message waited, in arrivals.
- Convergence: all members hold identical sets of secret IDs and the same latest ID, the membership
  is all 40, and every member read every application message published by another member.

### Column key (`logs/run-all.txt`)

| column | meaning |
|---|---|
| evts | non-create events (updates + merges + app posts) |
| upd / mrg | updates (not merges) / merges published by the rule |
| nmd | most dependencies any event named (carried) |
| lst | dependencies named by the last event |
| hds | most heads an author held before an event (the frontier it would carry uncapped) |
| >=32 / >32 | events whose uncapped frontier was >= 32 / > 32 |
| vw | most heads any member held at any point (view frontier) |
| pk / lw | peak messages waiting at any member / longest wait (arrivals) |

The crate has no observable heads or waiting count (private fields), so those cells are `-`.

## Results

### 1. Main scenarios, publication order

| scenario | orderer | evts (upd/mrg) | nmd / lst | hds | >=32 / >32 | vw | pk / lw | converged |
|---|---|---|---|---|---|---|---|---|
| a steady, app posts x2 | crate | 80 (0/0) | 1 / 1 | - | - | - | - | yes |
| a steady, app posts x2 | heads, no cap | 80 (0/0) | 1 / 1 | 1 | 0 / 0 | 1 | 0 / 0 | yes |
| a steady, updates x2 | crate | 80 (80/0) | 40 / 40 | - | - | - | - | yes |
| a steady, updates x2 | heads, no cap | 80 (80/0) | 1 / 1 | 1 | 0 / 0 | 1 | 0 / 0 | yes |
| b burst (40 updates), then turns | crate | 80 (40/0) | 40 / 40 | - | - | - | - | yes |
| b burst (40 updates), then turns | heads, no cap | 80 (40/0) | 40 / 1 | 40 | 1 / 1 | 40 | 0 / 0 | yes |
| b burst (40 updates), then turns | heads, cap 32 | 81 (40/1) | 32 / 1 | 40 | 1 / 1 | 40 | 0 / 0 | yes |
| b burst (40 app posts), then turns | heads, no cap | 80 (0/0) | 40 / 1 | 40 | 1 / 1 | 40 | 0 / 0 | yes |
| b burst (40 app posts), then turns | heads, cap 32 | 81 (0/1) | 32 / 1 | 40 | 1 / 1 | 40 | 0 / 0 | yes |
| b2 storm x3 (40 concurrent updates per round) | crate | 160 (120/0) | 40 / 40 | - | - | - | - | yes |
| b2 storm x3 | heads, no cap | 160 (120/0) | 40 / 1 | 40 | 81 / 81 | 40 | 0 / 0 | yes |
| b2 storm x3 | heads, cap 32 | 161 (40/81) | 32 / 1 | 56 | 81 / 81 | **56** | 0 / 0 | yes |
| b2 storm x3 | heads, cap 32, designated merger | 122 (80/2) | 32 / 1 | 40 | 2 / 2 | 40 | 0 / 0 | yes |
| c partition p=2, k=3 (2 vs 38) | crate | 160 (120/0) | 40 / 40 | - | - | - | - | yes |
| c partition p=2, k=3 | heads, no cap | 160 (120/0) | 40 / 1 | 40 | 77 / 77 | 40 | 0 / 0 | yes |
| c partition p=2, k=3 | heads, cap 32 | 161 (44/77) | 32 / 1 | 52 | 77 / 77 | **52** | 0 / 0 | yes |
| c partition p=2, k=3 | heads, cap 32, designated merger | 85 (44/1) | 32 / 1 | 40 | 1 / 1 | 40 | 0 / 0 | yes |
| c partition p=10, k=3 (10 vs 30) | crate | 160 (120/0) | 40 / 40 | - | - | - | - | yes |
| c partition p=10, k=3 | heads, no cap | 160 (120/0) | 40 / 1 | 40 | 1 / 1 | 40 | 0 / 0 | yes |
| c partition p=10, k=3 | heads, cap 32 | 161 (120/1) | 32 / 1 | 40 | 1 / 1 | **40** | 0 / 0 | yes |
| c partition p=20, k=3 (20 vs 20) | crate | 160 (120/0) | 40 / 40 | - | - | - | - | yes |
| c partition p=20, k=3 | heads, no cap | 160 (120/0) | 40 / 1 | 40 | 1 / 1 | 40 | 0 / 0 | yes |
| c partition p=20, k=3 | heads, cap 32 | 161 (120/1) | 32 / 1 | 40 | 1 / 1 | **40** | 0 / 0 | yes |
| d offline member 20, 10 cycles, catch-up in order | crate | 401 (10/0) | 10 / 10 | - | - | - | - | yes |
| d offline member 20, catch-up in order | heads, no cap | 401 (10/0) | 1 / 1 | 1 | 0 / 0 | 1 | 0 / 0 | yes |
| d offline member 20, catch-up shuffled | heads, no cap | 401 (10/0) | 1 / 1 | 1 | 0 / 0 | 1 | **375 / 399** | yes |
| e lost update to member 7 | crate | 161 (1/0) | 2 / 2 | - | - | - | member 7 read 39 of 156, then none | stalled |
| e lost update to member 7 | heads, no cap | 161 (1/0) | 2 / 1 | 2 | 0 / 0 | 2 | **117 waiting at 7** | stalled |

Notes:
- Crate, steady updates: `nmd` reached 40 and was still 40 at the last event. Crate, app posts only:
  1, because only the creator had published a control message. Crate, offline: 10 = ten control
  publishers so far.
- Storm and partition: `hds` never exceeded 40 without a cap (one head per author). With a cap of
  32, `vw` went to 52–56 (the rule does not collapse concurrent merges, see §3).
- Lost update: member 7 read the 39 app messages sent before the loss, and nothing after it. Both
  orderers hold the 117 later messages waiting indefinitely.

### 2. Cap sweep, publication order (merge rule enforced)

| cap | storm x2: evts / merges / >=32 / vw | partition p=10 k=2: merges / vw | partition p=20 k=2: merges / vw | burst then turns: merges |
|---|---|---|---|---|
| 2 | 1679 / 1599 / 369 / 59 | 999 / 54 | 799 / 49 | 39 |
| 4 | 613 / 533 / 123 / 49 | 314 / 48 | 253 / 45 | 13 |
| 8 | 287 / 207 / 83 / 50 | 136 / 45 | 88 / 58 | 5 |
| 16 | 163 / 83 / 42 / 57 | 33 / 54 | 43 / 48 | 2 |
| 24 | 122 / 42 / 42 / 56 | 31 / 46 | 1 / 40 | 1 |
| 40 | 121 / 41 / 41 / 40 | 1 / 40 | 1 / 40 | 1 |

All converged. The cap-40 merge is the rule firing at exactly 40 (`>= cap`), which is harmless.

### 3. Adversarial delivery: each member hears each round in its own order

Storm x3 and partition p=2 k=3, two orders per cap. "no backlink" is the orderer as the driver
found it. "backlink" names the author's own latest event first.

| cap | no backlink, storm | no backlink, p=2 | backlink, storm: merges / peak waiting / longest wait | backlink, p=2: merges / peak / longest |
|---|---|---|---|---|
| 2 | **fails in both orders** (HPKE OpenError) | **fails in both orders** | 3159 / 1153 / 1520 (order 2: 3159 / 1181 / 1519) | 2855 / 1038 / 1368 |
| 4 | **fails in both orders** | **fails in both orders** | 1053 / 320 / 505 | 925 / 286 / 442 |
| 8 | **fails in both orders** | **fails in both orders** | 405 / 83 / 194 | 385 / 79 / 184 |
| 16 | **fails in both orders** | **fails in both orders** | 162 / 17 / 75 | 154 / 16 / 72 |
| 24 | **fails in both orders** | **fails in both orders** | 81 / 1 / 1 | 77 / 1 / 1 |
| 32 | converged (both orders) | converged (both orders) | 81 / 1 / 1 | 77 / 1 / 1 |
| 40 | converged (both orders) | converged (both orders) | 81 / 1 / 1 | 77 / 1 / 1 |
| none | converged (both orders) | converged (both orders) | 0 / 0 / 0 | 0 / 0 / 0 |

- Without the backlink, the failure is `Dcgka(TwoParty(Hpke(Decryption(OpenError))))`, returned by
  `receive` at member 0 (storm) or member 2 (partition). Examples from the log, order 1, cap 16:
  `[33,2]` at member 0 in the storm, `[35,2]` at member 2 in the partition. `receive` then drops that
  member's state, so the case stops.
- With the backlink, every case converged, in both orders, at every cap from 2 to 40 and uncapped.
  This supports the mechanism below. I did not trace the failing messages, so the mechanism is
  inferred from these runs and the code.
- Cap 32 without the backlink converged in both orders. That is two samples, not a proof.

### 4. Designated merger (only member 0 merges at the cap; other members at the cap skip the event)

| case | events (upd/mrg) | vw | converged |
|---|---|---|---|
| storm x3, cap 32, publication order | 122 (80/2) | 40 | yes |
| storm x3, cap 16, publication order | 124 (80/4) | 40 | yes |
| storm x3, cap 32, reordered | 122 (80/2) | 40 | yes |
| storm x3, cap 16, reordered | 124 (80/4) | 40 | yes |
| partition p=2 k=3, cap 32, publication order | 85 (44/1) | 40 | yes |
| partition p=20 k=3, cap 32, publication order | 161 (120/1) | 40 | yes |

The view stays at 40, but storms lose about a quarter of their events (122 vs 161), because members at
the cap wait for the merger.

### 5. Source checks (`/tmp/research/q/enc/p2panda-encryption-0.7.1`)

- **One dependency per author:** `test_utils/ordering.rs:55` declares
  `previous: HashMap<MemberId, MessageId>`. It is keyed by author, and `previous.values()` gives every
  message its dependencies (`:85`, `:111`). It is updated per author (`:98` for our own control
  messages, `:188` for each control message popped by `next_ready_message`). Measured: 40 named in
  steady updates, as above.
- **No pruning:** `previous` is never removed from (no `remove`, `retain`, `clear` or `drain` on it).
  `y.messages.insert` (`:135`) keeps every message. In `ordering.rs`, `pending` entries are removed
  only by `remove_pending` (`:94-100`, called from `process_pending`, `:132`), which runs when the
  missing dependency arrives. `OrdererError` is an empty enum with a TODO about memory (`:138-143`).
- **Holding messages with missing dependencies:** indefinitely. There is no timeout and no per-author
  cap. The lost-update run shows a stall that lasts until the end of the run. Peak waiting in the
  shuffled catch-up was 375 of 400 messages with the heads orderer. I did not measure the crate's
  own waiting count, because its fields are private.
- Processing order: `group.rs:252-268` processes ready control messages first, then application
  messages. `receive` returns an error, and drops the state, on any DCGKA failure (the `?` on
  `process_ready` at `group.rs:254` and `:263`).

## Verdicts

1. **Frontier size, 40 members (heads, no cap):** largest frontier 40, the number of members. One
   head per author holds without truncation. Steady state is 1. The uncapped frontier reached 32 or
   more in one event per 40-way burst (the first post after it), and in 77–81 events in the storm and
   p=2 patterns. Nothing exceeded 40.
2. **Merge rule at 32, per event:** holds. Every event named 32 or fewer, and the orderer's overflow
   guard for application messages never fired. Merges were all triggered by the rule.
3. **Merge rule at 32, per member view:** does not hold. A member holds 40 heads after any 40-way burst
   or partition heal, until its next event. In storms the view grows by 8 per round (40 → 48 → 56),
   because concurrent merges cannot name each other and only a sequential merge collapses the frontier
   (the burst case drops to 9). The designated-merger variant keeps the view at 40.
4. **Convergence:** every in-order run converged, at every cap from 2 to 40 and uncapped, including
   1,600–3,200 events in the small-cap storms, with no DCGKA error. Under adversarial order, the
   orderer converges only if the author's own latest event is always named (backlink), or if the cap
   is 32 or more (two orders tested).
5. **Is the merge rule needed at 40 members?** Yes, at cap 32. A 40-way burst or a heal of 40
   posters puts 40 heads in each view. Without a cap, no merge is needed, since the frontier is at
   most the number of members. A cap of 40 merges once, at exactly 40, in the burst case.
6. **Smallest cap that works:**
   - In publication order: 2 converges, but the view frontier is 40–59 for every cap below 40, and
     the cost at 2 is 1,599 merges in two storms.
   - Under adversarial order without the backlink: 32. Caps 2–24 fail in both orders.
   - With the backlink: 2 converges, but the cost is high: 3,159 merges at 2, 1,053 at 4, 405 at 8,
     and 162 at 16. From 24 up, the only merges are the storm's own 81. Peak waiting is 1,150 at 2,
     320 at 4, 83 at 8, 17 at 16, and 1 from 24 up.
   - Recommendation: 32, with the backlink, which costs no extra merges. 16 is the smallest cap I'd
     accept on cost, and below 8 the merge and waiting numbers grow steeply.
7. **Member offline for a long time (scenario d):** an offline member adds no heads. The crate keeps
   one entry per author forever. Catch-up in order waits for nothing. Catch-up shuffled holds up to
   375 of 400 messages waiting, and the longest wait was 399 arrivals.

## Surprising

- Truncating the heads can drop the author's own previous event. The 2SM ratchet then sees that
  author's events out of order, and DCGKA fails with an HPKE `OpenError` (§3). In-order tests never
  show it. The fix is to name the author's own latest event first (backlink). The design's
  per-author chain (`prev`/`seq`) would provide the same guarantee if the log layer enforces it, but
  this spike's readiness check (like the crate's) does not check it. That is a constraint the orderer
  spec must state.
- The view-level frontier cannot be bounded by a merge rule when merges are concurrent. The per-event
  cap holds, so the design's "`after` ≤ 32" is true, but "the frontier stays ≤ 32" is not.
- The crate's orderer converged in every publication-order scenario. In the lost-update run it
  stalled member 7 the same way the heads orderer did. I did not run the crate under reordered
  delivery, so I have no crate result for §3.

## Not tested

- Real transport, p2panda-sync, and per-author FIFO at the log layer. Our driver has no backlink
  readiness check, and no per-author order beyond what the orderer enforces.
- Membership changes (add, remove) during these scenarios. All runs were pure posting, merging and
  updates.
- Byte sizes of `after` and of welcomes. This spike counts hashes only.
- Groups other than 40, and sizes above 40. Wall-clock timing at scale.
- More than two reorder orders per cap, random (non-adversarial) delays, and message loss beyond the
  single lost update.
- The crate's own waiting count, which is not observable from outside.
- Heads and waiting in the crate's orderer, the message scheme, and BeeKEM.
- Designated merger under real latency or merger failure. Only member 0 was used, and its liveness
  cost was counted in events, not time.

## Files

- `src/main.rs`: driver, scenarios, measurements.
- `src/heads.rs`: heads orderer (with the backlink option).
- `logs/run-all.txt`: the full run (157 lines, 27 KB).
- `Cargo.toml`, `Cargo.lock`: standalone crate, pins copied from the spikes workspace.
