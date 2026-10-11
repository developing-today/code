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
