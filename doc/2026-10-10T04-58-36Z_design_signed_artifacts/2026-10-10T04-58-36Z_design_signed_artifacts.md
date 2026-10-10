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

## Implementation: signed artifacts

Built: `artifact.rs` (per-author chains with `prev`, Ed25519 over the body, `FriendRemove` and `HomeDeclared`, pure `apply_page`, iroh pull on ALPN `id/artifacts/1`), `directory.rs` (removal and home-declaration effects, `artifacts.jsonl` replayed on open, a removal whose directory line was lost is reconciled), the outbox `Payload::Artifact`, and `serve.rs` registering the pull protocol.

Choices:
- Pull runs over iroh on the existing endpoint, one page per request, at most `PAGE_LIMIT` (100). The home-server address is an `EndpointId` (`HomeServer.endpoint`). `world_id` is kept beside it because the audience of a declaration is a world. `url` is kept only for the outbox push.
- A sequence gap or a fork at a held sequence number is refused. Unknown kinds are stored and not applied.

Strong remove, ported as rules from p2panda-auth 0.7.1 (`src/group/resolver.rs`, MIT OR Apache-2.0). No code was copied, so no SPDX header.
- Applied: concurrent removals of one friendship by both sides both apply and end it once (`concurrent_removals_of_one_friendship_by_both_sides_both_apply_and_end_it_once`).
- Not applied, and why: the concurrent-op filter needs causal references. Artifacts have no such references, and friend requests are envelopes that carry none. Cycles and delegation or demotion do not exist in a pairwise friendship, and remove-then-re-add needs the same references on requests.

Grants, from keyhive_core 0.6.0 (Apache-2.0): the attenuation and reachability rule is already in `world.rs`. `attenuate` requires a subset and `DELEGATE`, `effective_scopes` intersects every link and stops a delegation whose friendship has ended, and `revoke_tree` cascades. Nothing was ported. `HomeDeclared` is not a grant, so artifacts have no grant chain.

No NOTICE file exists under `pkgs/id`, so none was added.

Not built: pull scheduling; outbox push still uses HTTP `POST /artifact` to `HomeServer.url`; causal references on friend requests, which the concurrent-op filter needs.

Flags:
- Friend requests need a causal reference, and adding one changes the envelope protocol. This needs a decision before the concurrent-op filter can land.

---

## 2026-10-10T08-20-34Z Update: artifacts move from HTTP to iroh; generic kinds and causal references

Artifact transfer between servers now runs over iroh, like envelopes. The HTTP artifact route and the HTTP pull are removed. HTTP stays for the web UI only.

- ALPNs: `/id-artifact-push/1` pushes one signed artifact to a home server and gets a `Reply` (`applied`, `duplicate`, `refused`, `retry`). `/id-artifact-pull/1` returns one page of an account's chain. Both use the envelope framing (u32 length, JSON, 1 MiB frame cap) and are registered next to `/id-envelope/1` in `serve.rs` when the world feature is on.
- Why HTTP was removed: a home server is a peer that holds our chain. Sending its artifacts over the same node identity as envelopes means one address form (an endpoint ticket, or a bare node ID), no TLS hostname to declare, and no second transport to secure. The outbox queues pushes as `Payload::Artifact` under the key `artifact {target} {hash}`, with the same retry schedule as envelopes.
- Replaced: `HomeServer.url` is replaced by `HomeServer.endpoint` (ticket or node ID) plus `world_id`. The outbox `url` field is replaced by `target`. The HTTP `POST /artifact` route and the `id/artifacts/1` pull ALPN are removed; their replacements are the two ALPNs above.
- Targets: any recipient, envelope or artifact, accepts an endpoint ticket (which carries addresses) or a bare node ID (which needs discovery through the endpoint's address lookup). `check_target` is the single parser.

Causal references:
- Signed artifacts carry an optional `after`: up to 32 artifact hashes, each 64 lowercase hex characters. It is part of the signed body and is omitted when empty, so existing signatures are unchanged.
- Chosen over adding references to envelopes, because it keeps one signed format for all kinds.
- Not yet built: the consumer. The concurrent-op filter for strong remove is still unimplemented. It is the next step for `after`, and it is the only use that depends on it.

Generic signed data:
- `document`: a media type and a JSON value, at most 64 KiB serialized.
- `tag`: a subject hash, a name of 1 to 64 bytes, and an optional value of at most 256 bytes.
- Author tag: a `tag` named `author` whose value is the author's key, signed by that author over a document they published. Anyone can check the signature and the match with the author key.
- Documents and tags are stored, pulled and checked like other artifacts. They are never applied to the directory, so an unknown or generic kind cannot change friendship or membership.
- Publishing pushes the artifact to the signed-in account's own declared home servers. Ending a friendship pushes its removal artifact to the friend's declared home servers.

`sends_envelope` now covers friend removal and publishing too, so a server without an outbox refuses them before anything changes.

Not done: a CLI or explorer grammar for publishing documents and tags (only the directory action exists), and tickets in invite links (`node=` still takes a node ID).
