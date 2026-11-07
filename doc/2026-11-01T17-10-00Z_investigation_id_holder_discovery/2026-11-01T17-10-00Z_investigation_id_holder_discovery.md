# Holder discovery implementation audit (#2260)

## Request and initial approach

Implement #2260: track blob-hash holders from sync and scoped announcements, answer candidate queries through a world's public server, populate the blob gate's hash-to-world reference index, and wire the fail-closed gate into `id serve`. Keep private-world holdings hidden from non-members and expire stale holders. The initial approach was to reuse `blob_gate::WorldAccess`, `PeerDiscovery`, the existing event/document stores, and the `id serve` protocol setup rather than introduce a new content-transfer protocol.

## Findings

- `blob_gate.rs` already makes fetch/observe decisions from a hash-to-world map and current membership. Its production reference map has no producer. `tests/blob_gate.rs` already covers member fetch, non-member refusal, and equal refusal responses for unknown versus forbidden hashes, but its graph is synthetic; it does not exercise `id serve` wiring.
- `LocalEvents::publish` records document events with `group: None` (`src/local_events.rs`). `EventStore::documents()` returns the document reference but not its group (`src/event_store/documents.rs`). Consequently this sync path cannot currently produce a document-hash-to-world index. Its range sync exchanges event records; it does not itself establish that a peer holds the referenced blob bytes.
- The gate's membership adapter wraps `groups::worlds::Worlds` (`src/blob_gate.rs`), while hosted `WorldHub` sessions authorize participants with join capabilities. No production adapter connecting those authority models to blob requests was found.
- Existing `PeerAnnouncement` messages contain only a self-asserted node ID, optional name, blob count, and timestamp (`src/discovery.rs`). Serve broadcasts them on the configured shared gossip topic (`src/commands/serve.rs`); they contain no hash holdings, world scope, or membership proof. No holder-query protocol or per-world private announcement channel was found.
- The retrieval investigation says private-world announcements must be membership-only and that the public-server rendezvous answers from the same provider state. The current tree has neither the scope source nor the private transport/authentication needed to do that safely.

## Decision and unresolved choices

Do not wire hash announcements onto the shared gossip topic or infer a world from an unscoped document. Either choice could disclose private holdings or grant access based on unrelated membership. Stop before protocol or serve changes until these choices are settled:

1. Which authoritative world model scopes blobs: group IDs in event documents, hosted-world join capabilities, or another graph? How does a document without a group/world scope behave?
2. How does a peer prove membership when requesting a private holder list, and how are private announcements transported only to members? Is private gossip deferred while only public holdings are announced?
3. What identifies a public world/server, and what authenticated request/response protocol lets that server answer holder queries without turning hash presence into a content-enumeration oracle?
4. Which sync signal proves a peer has blob bytes (including partial ranges), rather than only a signed document/event that names them?

The existing gate and its synthetic integration tests remain unchanged. This audit intentionally makes no claim that #2260 is implemented; it records the repository evidence and the safety boundary for follow-up design.
