# Authenticated peer announcements and bounded discovery

Issue: [#2944](https://github.com/developing-today/code/issues/2944)

## Intent

Public gossip currently accepts an announcement's self-asserted `node_id`, retains an unbounded number of peers, and probes every stale peer sequentially with a ten-second timeout. A forged identity can replace a peer, while a flood can grow the table and prolong cleanup. This change authenticates announcements, bounds retained peers, and limits each stale-cleanup pass.

## Design

- Keep the existing `PeerAnnouncement` data type and its postcard layout unchanged. It is also nested in `MetaResponse::ListPeers` under `/iroh-meta/2`; changing that layout here would silently break an unrelated wire protocol.
- Gossip uses a distinct, versioned frame containing the announcement and an Ed25519 signature. Sign a domain-separated, deterministic encoding of the version and every claim field (`node_id`, `name`, `blob_count`, and `timestamp_secs`). The signer must match the claimed `node_id`; receivers derive the verification key from that ID and use strict signature verification. The versioned frame is the only accepted gossip path: unsigned legacy frames are ignored, with no fallback that would keep the spoofable path alive. Existing freshness and per-peer timestamp-order checks remain in force as replay controls.
- Verify at both gossip ingress paths: the server's table update and the `peers` command's direct gossip collection. Do not treat Iroh's `delivered_from` relay endpoint as the original author.
- Cap the in-memory peer table at 1,000. Existing IDs may refresh at capacity. For a new ID at capacity, evict the stale peer with the oldest local `last_seen`, breaking ties by endpoint-ID bytes; if every retained peer is fresh, reject the new ID. This bounds state without allowing a fresh Sybil announcement to displace a fresh peer.
- Limit advertised names to 256 UTF-8 bytes and signed gossip frames to 1 KiB. Apply the peer-count cap to the CLI's temporary gossip result map as well, so both long-running and one-shot gossip ingestion have bounded state.
- Each cleanup tick probes at most four stale peers, selected in deterministic endpoint-ID order with a round-robin cursor. With the existing ten-second per-peer timeout, a pass is bounded to at most forty seconds. Refresh a reachable peer; remove a failed peer individually only if it is still stale, so a concurrent valid announcement cannot be deleted. Never run the global stale sweep after a partial probe batch.

## Compatibility and limits

New receivers intentionally ignore old unsigned gossip announcements; accepting them as a fallback would preserve the identity-forgery vulnerability. `PeerAnnouncement` and `/iroh-meta/2` remain unchanged in this scoped fix. Signatures prove control of the claimed endpoint key, not uniqueness of identities: a peer can still generate multiple valid keys, so the table cap remains necessary. Rate limiting and broader topic changes are outside this fix.

## Verification plan

- Test signed-frame round trips, unsupported versions, unsigned legacy frames, mismatched signing keys, and tampering with every signed claim field.
- Test capacity, existing-ID refresh at capacity, deterministic stale eviction, refusal when all entries are fresh, and the invariant that the table never exceeds its cap.
- Test oversized names and frames are rejected before they enter retained peer state.
- Test bounded round-robin stale selection and conditional removal when a concurrent refresh makes a probed peer fresh.
- Run `cargo test -p id discovery` and the focused serve/peer-gossip tests; then run formatting and relevant package checks.

## References

- [Peer discovery feature record](../2026-03-22T00-00-00Z_feature_peer-discovery/2026-03-22T00-00-00Z_feature_peer-discovery.md)
- [Issue #2944](https://github.com/developing-today/code/issues/2944)
- [`pkgs/id/crates/id-core/src/discovery.rs`](../../pkgs/id/crates/id-core/src/discovery.rs)
- [`pkgs/id/src/commands/serve.rs`](../../pkgs/id/src/commands/serve.rs)
- [`pkgs/id/src/commands/peers.rs`](../../pkgs/id/src/commands/peers.rs)
