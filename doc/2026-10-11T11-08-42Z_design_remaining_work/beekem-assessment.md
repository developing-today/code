# BeeKEM: why we are not adopting it now, and when to reconsider

- Date: 2026-10-11
- Status: Decided for now. DataScheme stays for every group size, with warnings (decision 9). BeeKEM is revisited under the conditions in section 4.
- Evidence: the BeeKEM spike, `spikes/beekem/RESULTS.md` (beekem 0.4.0, published on crates.io 2026-09-25, MSRV 1.90). The comparison numbers come from that spike. Where a number is an extrapolation, it says so.
- Related: [task 05](./task-05-encryption-and-history.md), [explainers](./explainers.md) (the welcome and epoch terms), [plan](./plan-phases.md).

## 1. What BeeKEM would replace

Today a group uses p2panda-encryption's DataScheme (DCGKA over 2SM with HPKE). A key update is a fresh secret sent to each other member as a separate two-party message. That is n-1 messages per update, and the same per remove. The welcome for a new member carries every epoch secret (146.6 B per epoch today; 40.0 B per epoch with the serde_bytes encoding, see scheme versions in the explainers).

BeeKEM is a tree-based group key agreement (a concurrent variant of TreeKEM). One update is one operation that changes a path in a binary tree. Its appeal is that the bytes per update grow with the log of the group size, and one operation can be sent to everyone at once.

## 2. Measured results

All numbers are from the spike. Steady-state update means one commit by one member in a settled group.

| Measure | Result |
| --- | --- |
| Steady-state update, 100 members | 2.8 KB |
| Steady-state update, 1,000 members | 3.9 KB |
| Steady-state update, 5,000 members | about 5.0 KB (extrapolated; not run in full) |
| Remove plus the root-restoring update, 1,000 members | about 5.3 KB |
| Committer CPU per update | 0.6 to 0.8 ms |
| Receiver merge plus verify | about 40 µs plus about 45 µs |
| First commit after a bulk build, 1,000 members | 122 KB (cost is O(n)) |
| First commit after a bulk build, 5,000 members | 603 KB (partial run) |
| Per-member state after a full fill, 1,000 members | 64 MB, with nothing pruned |
| Joiner replica right after creation, 1,000 members | 0.74 MB |
| Joiner replica after history, 1,000 members | 64 MB |
| DataScheme comparison under unicast | BeeKEM one operation is 7 to 16 times DataScheme's bytes |
| Break-even under fan-out | BeeKEM wins on bytes only from about 11 to 17 members |

The bytes comparison assumes one copy of the BeeKEM operation per recipient, which is what our unicast transport would do. The win needs a transport that can send one message to many members.

## 3. Difficulties

Each item says what is wrong, why it matters for us, and whether a fix is in the crate or in our code.

1. **No welcome or join API.** A new member cannot be given a small welcome. The only route the spike found is a copy of an existing member's replica. The copy carries that member's secrets and every epoch key, so it must be stripped before it is sent. A fresh replica cannot start from the latest update. This makes join the hardest part of the design. *Fix:* a crate change or our own join protocol with a path-only welcome. Not tested: a path-only welcome needs a crate change.
2. **State grows without bound.** Each member keeps the full operation history: 64 MB at 1,000 members after a full fill. Nothing in the crate prunes it. *Fix:* snapshots at causal cuts, the same idea as tag compaction. Not designed.
3. **The first commit after a bulk build is O(n).** 122 KB at 1,000 members. A group that starts large pays this once.
4. **Concurrency needs causal delivery.** Concurrent operations converge only when a member buffers out-of-order operations. Dropping them converged in 1 of 300 trials. So BeeKEM depends on our ordering layer (the heads orderer and its caps, see the explainers) being correct first.
5. **Liveness gaps under concurrency.** A concurrent commit leaves no root key until the next commit. A concurrent removal applies lazily, so a member can read a stale membership list for a while. Both need product rules for what the UI shows and what sends are allowed while a root is missing.
6. **Forward secrecy is not what the name suggests.** A later replica copy derives earlier roots. Post-compromise security after an in-group compromise holds only once the compromised member commits again. A removed member cannot derive the new root, which is the part we need.
7. **No authority or signature checks.** The crate accepts any operation and any key on add. Our layer must check every operation against the membership and delegation rules before the crate sees it. That is a large integration cost, and the crate's safety depends on it.
8. **Fan-out is required for the byte win.** Our transport sends each message to one recipient. Without a broadcast path (for example a gossip topic or a content-addressed blob that members fetch), BeeKEM costs more than DataScheme. We have no broadcast path in the design today.
9. **Migration is not in-place.** A DataScheme group cannot be converted into a BeeKEM group. The options are a new group with re-invites, a scheme switch at a barrier inside the same group (recommended if continuity matters), or running both for a while. Seeding the new root from the old secret was rejected: the crate has no seed API.
10. **Maturity.** Version 0.4.0, published 2026-09-25. The spike did not run the crate's own test suite, and did not review the code. The owner-facing risk is a small, new, single-maintainer dependency, not a known bug.
11. **Not tested:** a full fill at 5,000 members (about 40 minutes estimated, so the spike ran a 200-member partial fill); a removed member's bypass attempts; DataScheme's per-message bytes at 1,000 members (estimated, not measured). The last one is a Phase 4 measurement task (see the plan).

## 4. When to reconsider

Reconsider BeeKEM only when all of these are true:

1. **Join is solved.** The crate, or our code, provides a welcome for a new member that is small and does not carry other members' secrets.
2. **Authority is checked before the crate.** Our layer validates every operation against membership and delegation, and the test suite proves that a removed member's operation is rejected.
3. **A broadcast path exists.** One send reaches all members, so the per-update bytes saving is real. Candidates: an iroh-gossip topic per group (the crate is already in the product tree), or a content-addressed blob per operation.
4. **Measured need.** Real DataScheme update traffic at large sizes is a bottleneck. The measurement is the Phase 4 task that records update bytes and CPU at 100, 1,000 and 5,000 members, with our adapter and our transport. Trigger: the warnings for groups above 1,000 fire often, or a single update exceeds an agreed budget.
5. **State is bounded.** Pruning or snapshots keep per-member state under an agreed size.
6. **The crate matured.** A stable release with its own tests, and an independent review of the security properties we rely on.
7. **A migration plan the owner accepts.** The scheme switch at a barrier, or the new-group approach, with the transition period.

If conditions 1 and 2 hold and 3 does not, BeeKEM is still the wrong choice, because the bytes would be worse than DataScheme's.

## 5. What we do meanwhile

- DataScheme for all groups, with warnings at 96, 128 and 1,000 members (N11). The warnings are local to the server, and no group is blocked (decision 9).
- Measure DataScheme's own update costs at 100, 1,000 and 5,000 members in Phase 4. Write the results next to the BeeKEM table, so the comparison uses measured numbers on both sides.
- Keep the BeeKEM spike code and results in the repo, so the work can resume.
