# Task 04: Sync, blobs, and transport

- Status: Designed. S4 is done. S7 (range sync on real stores) is open and gates this task. The `observe` default needs a decision.
- Index: [what is left](./2026-10-11T11-08-42Z_design_remaining_work.md). Depends on: [task 02](./task-02-events-and-storage.md) and S7.
- Design: [design §9.3, §11](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T03-44-33Z_design_groups_events_delegation.md), [review §3.3 and §3.7](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T08-08-56Z_design_review.md). Evidence: S1 (sync simulation), S4 (blobs).

## Goal

Events move between nodes by range reconciliation. Large payloads move as iroh-blobs. Relays are n0's by default, which is already done (see the [feature doc](../2026-10-11T11-08-42Z_feature_n0_relay_default/2026-10-11T11-08-42Z_feature_n0_relay_default.md)).

## Scope

1. **Range reconciliation.** Over `(author, seq)` keys, on a new ALPN `/id-sync/1`, with parameters K = 16 (fan-out) and T = 16 (leaf threshold). The items are the events themselves, so event bytes move in the same exchange. LogSync is not used.
2. **Blob payloads.** iroh-blobs `=0.103.1`. The blob ID is the body hash. A fetch needs the provider's EndpointId and an address, since there is no content routing.
3. **Gating.** `EventMask` in Intercept mode for `get` and `get_many`, with the connection-to-peer map that fails closed. A denial resets the stream with code 1. Clients read it with `GetError::iroh_error_code()`, so denial and absence look different.
4. **`observe`.** Its default mode serves requests, so existence and size can leak even when the bytes are gated. Gate it (recommended), or accept the leak. This is a decision.
5. **Size check.** The blob's `size` from the event is checked against policy before any fetch.
6. **Verified streaming.** Keep it. Leaves are 16 KiB, so one bad byte discards its leaf.

## Acceptance

- S7 (to run first): 1,000 chains with real events, over iroh. Bytes and round trips within the range of the S1 simulation (about 6–7 KB and 7–8 round trips for 2 new events).
- Gating tests on the real store: an allowed requester gets the bytes, a denied one gets reset code 1 and no bytes, an open provider serves anyone.
- A corrupted leaf is rejected and never held.
- The fetch test from S4 at 10 MiB, in release mode, as a timing check.

## Gotchas

- Providers serve any `get` by default, with or without a ticket. Gating is the only protection.
- A requester that gets a denial through `Downloader` sees `Unable to download`. Use the raw `get` to read the code.
- `execute_get_many` is the only way to gate `get_many` as a unit. The downloader splits it into per-hash `get`s.
- The iroh-blobs README still says the version is not production quality. The owner accepted that. Revisit before a release.

## Files

`pkgs/id/src/` sync module (new), `artifact.rs` and `envelope_net.rs` (their transports move to the new sync), blob store wiring in `commands/serve.rs`.

## Dependencies

No new crates expected. iroh-blobs is already in the lock file.

## 2026-10-11T12-20-38Z Update: observe and sync

- `observe` is the iroh-blobs ObserveRequest. It returns the raw bitfield for a hash: which bytes the provider holds, and the size. Default mode `None` answers any requester, so existence and size leak.
- Gating: `EventMask` `Intercept` decides each request. Preferred (#5): serve observe only to peers who could fetch the blob under the world's rules, with observe off by default. Alternative: disable observe and take the size from the event that references the blob.
- Range sync (N5, N6): S1 was an in-memory simulation. At 1,000 chains with 2 new events, range control is 6,198 B in 7 flights. LogSync control is 76,071 B in 2 flights. At 10,000 chains: 6,978 B in 8 flights, against LogSync 760,071 B. S1 also found LogSync cheaper at 200 new events on 1,000 chains.
- S1 could not test sequence gaps, XOR cancellation, bounds at 0x00 bytes, or interrupted sessions. S7 checks each on real stores.

## 2026-10-11T12-49-09Z Update: range sync on real stores (S7)

- Range sync over iroh on real redb stores works, and it matches the S1 simulation. Every run ended with equal stores, no duplicate inserts, and the true difference learned.
- 1,000 chains, 2 new events: 7 flights, 6,554 B control, 996 B data. S1 simulated 6,198 B. The gap is probably the 40-byte keys. 10,000 chains: 8 flights, 7,422 B control.
- Crossover (K=16, T=16): range wins control bytes up to about 33 new events at 1,000 chains, and about 360 at 10,000. LogSync wins flights everywhere (2 against 7 to 8). LogSync was not run on the real stores; those numbers are S1's.
- Parameters: keep K=16, T=16. For bulk catch-up, K=4 and T=8 save 28 to 33% of control at 200 new events, for 4 more flights. Avoid K=2 and K=32.
- Gap case: a height-only comparison would miss a sequence gap. Range catches it.
- XOR cancellation: the count check catches unequal sizes. Equal-size collisions can slip through; shown with crafted IDs. With real BLAKE3 IDs the chance is about 2^-128 per range.
- Interrupted session: the cut left whole batches (96 of 180). The restart sent only the 84 missing events, with no duplicates.
- Equal stores still cost about 21.6 KB on the wire, from the handshake and two small messages. That dominates small syncs.
- S1's split code would panic when K exceeds a range starting at key 0 (read, not run). The S7 port uses min(K, n).
- Not tested: LogSync over iroh, real round-trip time, bootstrap from an empty store, power loss.
