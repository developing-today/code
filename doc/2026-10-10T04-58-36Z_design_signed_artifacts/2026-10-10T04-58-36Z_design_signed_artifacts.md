# Signed artifacts, home servers, and cross-world access

## Intent

Accounts are portable where possible. A friend envelope, a friend removal, and
a home-server declaration are signed by the account that made them, so any
world can check and accept them whichever server delivered them. No server is
the authority for a fact about an account. Servers are only storage and
availability.

## Decisions (user, 2026-10-10)

1. **Cross-server friend envelopes:** yes, in the right circumstances, between two servers.
2. **Account portability:** yes, where possible. The account key is the identity, and an account is usable on a server that accepts it.
3. **Friend removal:** a signed artifact, replicated across servers. Servers check each account's declared home servers for new artifacts and events.
4. **Documents:** any world accepts documents signed by their originator. Acceptance does not depend on which server the connection came from.
5. **Moderator scope:** start with world-wide. A moderator of the world's own directory may change its isolation and cross-world settings.
6. **Outbox:** keep for now. TODO in `pkgs/id/TODO.md` to consider removing it later.
7. **Files:** an Unrestricted mode can reach files anywhere. Isolated worlds are read-only by default without an admin override.
8. **Cross-world write:** an Unisolated world can allow other worlds to write to it. A permissioned principal can force-enable write on an Unisolated world.

## Research: keyhive and p2panda

Web search was unavailable (HTTP 503), so this uses the two repository pages only. Deeper reading is still needed before any adoption.

- **keyhive** (Ink & Switch, Rust). `keyhive_core` provides signing, encryption and delegation. `beekem` is a concurrent TreeKEM for group keys. Auth-enabled sync lives in a separate project, Subduction. The README says pre-alpha, not audited, and not for production.
- **p2panda** (Rust). `p2panda-core` provides signed, hashed operations (Ed25519, BLAKE3). `p2panda-sync` syncs append-only logs. `p2panda-auth` provides decentralised group management with per-member permissions. `p2panda-encryption` provides group encryption. The README says APIs are not yet stable.

**Fit.** Both target the same problem: originator-signed data that replicates without a central server. Our existing `Envelope` and `LogRecord` already have that shape.

**Recommendation.** Do not add either as a dependency yet. Both are pre-alpha or unstable, and an unaudited auth library is a poor base for friend and access decisions. Adopt their model instead:
- per-author signed append-only logs with a sequence number and a hash link to the previous artifact (p2panda);
- capability-style grants that name their originator, which can be checked offline (keyhive).

Revisit `p2panda-auth` and `p2panda-sync` when they reach a stable release. Decide with the user before any adoption.

## Signed artifacts

One shape for everything an account asserts:

- `author`: the account's Ed25519 key.
- `seq`: per-author sequence number, starting at 1.
- `prev`: hash of the author's previous artifact, empty for seq 1.
- `kind`: one of `FriendRequest`, `FriendAccept`, `FriendRemove`, `HomeDeclared`, or a kind this server does not know.
- `body`: kind-specific fields, for example `{ a, b, request }` for a removal.
- `hash`, `signature`: as in `LogRecord`.

A world verifies the signature against `author`, checks the hash chain for gaps, and applies the kinds it understands. Unknown kinds are stored and not applied, so a newer peer does not break an older server.

### Friend removal

- A removal is an artifact signed by the account that removes. It names the friendship it ends and the request ids it covers.
- A stale request for a removed friendship is refused on receipt, because it names a request the removal covers. This also closes the re-created pending request case from the design doc's replay findings.
- Both sides apply the removal, so the servers converge.

### Home servers

- `HomeDeclared { account, servers: [{ url, world_id }] }` is signed by the account key and lists where that account's artifacts are held. A new declaration replaces the old one by `seq`.
- A server that holds data for an account pulls new artifacts from that account's declared home servers, asking for `seq > last_seen`. The response is verified per artifact, so the server that served it is not trusted.
- A server that holds an account serves that account's log to peers.
- Pushed envelopes (the outbox path) stay as the fast path. Pull is the catch-up path when a push was lost.

### Open points

- A stable server identity. `world_id` is the current audience, and it must not change across restarts for this to work.
- Pull cadence and the cost of polling many home servers.
- Whether a removal needs the other side's acknowledgement. Recommended: no. Removal is one-sided and effective when signed.

## Cross-world access (summary of the approved rules)

- **Isolated (default):** no cross-world reads or writes.
- **Unisolated:** cross-world reads between unisolated worlds, as before.
- **Cross-world write:** an Unisolated world turns it on for other Unisolated worlds. Off by default.
- **Force-enable:** a principal with the server operator's admin token can force write on for an Unisolated world. The world's own moderators cannot turn a forced setting off.
- **File scope:** Confined (the world's own root, the default) or Unrestricted (any path). Isolated worlds are read-only under Unrestricted unless an admin override enables write.

## Implementation order

1. Envelope delivery (in progress on `feat/id-envelope-delivery`): producer, push endpoint, flush loop, pending cap, outbox status.
2. Isolation write, force-enable, file scope (in progress on `feat/id-isolation-write`).
3. Signed artifacts: per-author log with `prev` links, `FriendRemove`, `HomeDeclared`, and pull from declared home servers. Depends on 1.
4. Section 3 network sync of the directory log. Depends on 3.
