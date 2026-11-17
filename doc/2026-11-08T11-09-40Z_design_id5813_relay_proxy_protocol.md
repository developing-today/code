# ID-aware Relay and Store-and-Forward Protocol

- **Date:** 2026-11-08
- **Status:** Decision record — protocol design only; no implementation is included.
- **Issue:** [#5813](https://github.com/developing-today/code/issues/5813)
- **Parent:** [#5674](https://github.com/developing-today/code/issues/5674), under epic #2056.
- **Related decisions:** #2036, #5671, #5791, #5794.
- **Sibling interfaces:** #5675, #5808–#5811, #5795, and the later security review #5815.
- **Evidence baseline:** `origin/main` at `e1b77bba0dfa1a0d975887dee6cf0053c9190eb7` (2026-11-08).

## 1. Decision summary

The standard relay is an **ID-aware, payload-opaque forwarder**. It may inspect the canonical outer envelope and the minimum routing metadata needed to validate and route a message, but it does not decrypt or interpret application payloads. It validates the origin and authorization required by #2036 before forwarding, and appends only the mutable relay-hop metadata that #2036 permits.

The protocol has two delivery profiles:

1. **Live forward:** bounded, volatile forwarding for currently reachable recipients. A relay never creates an offline backlog for #2036's ephemeral session traffic. If a live queue fills, it applies backpressure and then rejects or terminates the affected delivery explicitly; it does not silently drop or persist the live tail.
2. **Durable delivery assist:** optional, bounded store-and-forward for already-durable, origin-signed chain events. A relay may hold the encrypted event envelope until an end-to-end receipt or a finite expiry. The origin's durable event graph remains the source of truth; the relay copy is a delivery optimization, not a new authority or sole copy. If the relay copy expires or is lost, normal range sync recovers the event.

A **dumb byte-pipe** remains a valid transport adapter (for example, a WSS pair forwarder), but it is not the standard ID-aware relay: it cannot perform ID routing, pre-forward validation, or durable delivery assistance. A conforming relay can expose both roles as separate modes; an opaque pipe must not claim the guarantees of an ID-aware relay.

This resolves #5674's open relay-mode question without changing #2036's message semantics. In particular, it rejects offline persistence for ephemeral live messages and narrows store-and-forward to the durable class. Any parent-level choice to persist ephemeral traffic or to let a relay reinterpret origin data would conflict with this record and must be called out as a superseding decision, not introduced implicitly by an adapter.

## 2. What is already fixed

This record does not reopen these decisions:

- **Message classes and durability:** #2036 distinguishes ephemeral session traffic from chained, durable graph events. Ephemeral traffic has no offline backlog; durable facts are recovered through graph/range synchronization (`doc/2026-11-01_design_id2036.md:54-64`, `pkgs/id/src/event_store/sync.rs:333-373`).
- **Origin and hop semantics:** the origin envelope and stable message ID are immutable; the relay hop chain is mutable audit/loop-control metadata. The existing hop limit is four; a relay refuses a loop and does not forward a message twice (`doc/2026-11-01_design_id2036.md:67-80`).
- **Validation before forwarding:** origin signature, canonical envelope, and group authorization are checked before forwarding. Transport encryption does not replace ID identity (`doc/2026-11-01_design_id2036.md:73-78,111`; `pkgs/id/src/world_net.rs:8-10`).
- **Frame bound:** the current shared frame cap is 1 MiB. This record does not define a new fragmentation or chunking format (`doc/2026-11-01_design_id2036.md:95-96`; `pkgs/id/src/frames.rs:18-22`).
- **Transport fallback:** #5791 owns fallback ordering and failure classification. Relay selection does not add a competing fallback order (`doc/2026-11-05T13-38-00Z_design_id5791_transport_negotiation.md:20-35,196-209`).
- **Concrete rendezvous adapter:** #5794's signed rendezvous records and blind WebSocket pairing remain a valid concrete adapter. This protocol adds an ID-aware relay role; it does not replace or make that adapter authoritative (`doc/2026-11-05T15-10-00Z_design_cloudflare_workers_rendezvous_relay.md:30-48`).

## 3. Roles and logical messages

The following names describe protocol operations, not byte-level message names or discriminants. Their serialization, field numbering, version negotiation, and framing are owned by #5808, #5809, and #5810.

- **Publisher:** an ID owner or authorized client that registers the owner's currently reachable relay endpoints with its designated home relay. A publication is a bounded, expiring presence lease, not proof that the relay owns the identity.
- **Home relay:** the logical relay shard responsible for one recipient ID's current presence lease and durable delivery-assist queue. The home relay is not the identity authority.
- **Lookup:** resolve a recipient ID to a current home-relay hint and permitted endpoints. Lookup returns routing metadata only; it does not execute an ID query or filter application data. Query semantics and clearance rules belong to #5675.
- **Ingress relay:** the first ID-aware relay selected by the sender, possibly also the recipient's home relay.
- **Transit relay:** an authorized relay that forwards toward the home relay or a live recipient. Transit relays do not become alternate homes or durable copies.
- **Recipient:** the end-to-end ID peer that validates and consumes the original message and, when supported by the message class, returns an end-to-end receipt.

The conceptual operations are:

1. `PublishHome`: register or refresh the publisher's live endpoint lease at its home relay. The relay publishes only a signed/authorized, expiring route hint under the trust policy of #5795.
2. `LookupHome`: resolve a recipient ID to its current home-relay hint. A missing, expired, or unauthorized hint returns an explicit no-route result; it is not treated as successful delivery.
3. `Forward`: submit one existing ID envelope for live routing. Each ID-aware hop validates the origin and authorization required by #2036, checks the hop limit/loop rule, adds its own permitted hop record, and forwards the envelope without changing the origin content.
4. `StoreDurable`: optionally commit a durable-class envelope to the recipient's home queue. The acceptance response is issued only after the home queue has committed the bounded item. Repeated submissions with the same stable message ID are idempotent.
5. `FetchDurable` / `Receipt`: on reconnection, the recipient fetches queued durable items and returns an end-to-end receipt. A relay's storage acknowledgment means only “accepted into this relay's bounded queue”; only the recipient's authenticated receipt means “delivered.”
6. `Backpressure` / `Reject`: communicate available forwarding credit or an explicit reason the relay cannot accept the item, such as queue-full, quota, expired route, or invalid message. Exact wire representation is delegated to #5808-#5810.

## 4. Publication, lookup, and multi-relay behavior

### 4.1 Presence is a lease

The owner publishes its current endpoints to one **logical home**. The lease expires unless refreshed; an expired lease is never returned as current. A lookup returns the home relay identity, permitted endpoint hints, expiry, and the relay's advertised delivery profiles, limited to the fields permitted by #5795. The publication and lookup records do not change the identity's cryptographic authority.

The directory/rendezvous layer may be replicated for availability, but there is one logical home owner for a recipient at a time. Multiple physical front ends may serve that home partition. Failover to a different logical home requires an authorized ownership/lease transition; a transit relay must not invent one. Signed rendezvous mechanisms already chosen in #5794 remain compatible.

### 4.2 Forwarding across relays

The sender resolves the recipient, then applies #5791's transport fallback policy to reach a permitted ingress or home endpoint. A non-home ingress forwards toward the recipient's home; the home routes to a live recipient or, for a durable item, its bounded queue. A relay may be both ingress and home.

Every relay hop applies #2036's validation, duplicate, and loop rules. The existing four-hop cap applies; an adapter cannot raise it. A hop failure is returned to the sender or previous hop as a classified failure. The protocol does not promise transparent failover to an unapproved alternate home, and it does not perform a multi-relay storage transaction. An alternate endpoint may serve the same logical home only when authorized by the home/trust policy.

For a recipient with more than one live endpoint, the home relay routes within the recipient's authorized delivery set. Duplicate physical delivery is permitted only where the receiver's stable message ID makes it idempotent; it is never a second logical event. Endpoint selection policy is an implementation choice and must not weaken identity or group authorization.

### 4.3 Store-and-forward is not a second event store

Only durable-class, already-origin-signed chain events may enter `StoreDurable`. The home relay retains the encrypted envelope as a bounded delivery copy; it is not allowed to manufacture, rewrite, or become the source of truth for the event graph. Expired or unavailable copies are recovered through the existing graph/range-sync path.

Ephemeral session messages, keepalives, session tails, and other non-durable live traffic are never accepted into this queue. If their recipient is offline, the sender receives an explicit offline/no-route result. This preserves #2036's no-backlog rule and its allowance that uncheckpointed ephemeral tails may be lost.

Retention is finite: remove an item when a valid recipient receipt is recorded or when the earlier of the sender's signed expiry and the relay's configured maximum retention is reached. The maximum is advertised as relay policy and is bounded by the #5795 authorization/quotas. The protocol makes no promise that a relay copy survives its expiry or a relay outage; durable graph synchronization is the recovery path. A relay must not acknowledge `StoreDurable` before the item is committed under that relay's advertised durability policy.

## 5. Delivery guarantees and receipts

The protocol distinguishes these events so callers cannot mistake relay acceptance for peer delivery:

| Event | Meaning | Does not mean |
|---|---|---|
| `route_published` | An authorized endpoint lease was accepted and has an expiry. | The endpoint is currently healthy or the ID's authority moved to the relay. |
| `forward_accepted` | A relay validated the envelope and has bounded capacity to attempt this hop. | The next hop or recipient accepted it. |
| `store_accepted` | The home relay committed a durable-class copy under its configured finite retention policy. | The recipient received it or the relay is the authoritative event store. |
| `recipient_receipt` | The end-to-end recipient authenticated receipt of the stable message ID. | Any relay may assert delivery without the recipient. |

Live forwarding is best effort while the recipient is reachable and credits remain. It has no offline persistence guarantee. Durable delivery assistance is at-least-once until receipt or expiry, with stable-ID deduplication; the receiver must treat duplicate copies as the same logical event. The relay does not promise exactly-once transport. If no recipient receipt arrives before expiry, the sender can rely on graph/range synchronization for the durable event, not on an indefinitely retained relay copy.

## 6. Bounded backpressure, quotas, and scaling

### 6.1 Backpressure

- Each hop has bounded per-connection and per-recipient live queues, a bounded durable queue, and an overall resource cap. The limits are configuration/advertised policy, not an unbounded buffer hidden behind a transport adapter.
- Forwarding uses receiver-driven credits or an equivalent bounded-window signal. A downstream pause propagates upstream; a relay must not read unlimited data into memory while waiting for a slow recipient.
- When a cap is reached, the relay sends backpressure and then an explicit retryable rejection if the sender cannot pause. It rejects before acknowledging acceptance. It may disconnect a persistently non-compliant sender. It must not silently discard a durable item after `store_accepted`.
- Isolation is per peer/recipient and per relay resource pool: one slow recipient cannot consume all capacity or head-of-line block unrelated recipients.

### 6.2 Quotas

Per-identity, per-relay, and aggregate byte/item/rate quotas are enforced at the authorized home/ingress points. The limits and authorization source are supplied by #5795; #5813 does not invent a second broker credential, allowlist, or trust model. Quota exhaustion is an explicit rejection, not silent downgrade to another delivery guarantee. Retry hints are bounded and cannot extend a signed item expiry.

### 6.3 Horizontal scaling

Live forwarding is stateless with respect to durable message ownership: an ingress can validate and forward a frame without sticky application sessions, subject to the current socket and bounded queue. Presence leases and durable delivery queues are stateful, keyed by recipient ID at that recipient's logical home partition. Physical front ends may scale horizontally over that partition; the home lease prevents two unrelated partitions from independently claiming authority for the same recipient. Transit relays keep only bounded in-flight state and hop metadata, not durable replicas.

The directory may advertise health/latency and a relay's supported profiles, subject to #5795's metadata-minimization contract. Routing can use those hints only after the caller's selected transport fallback policy (#5791) and trust policy allow the endpoint. A health hint is advisory, not authorization.

## 7. Alternatives considered

1. **Pure dumb byte pipe as the only protocol — rejected.** It is useful as a simple adapter and remains supported, but it cannot resolve recipient IDs, validate before forwarding, enforce per-ID quotas, or distinguish live from durable delivery. It cannot satisfy #2036's relay validation requirements or this issue's routing and backpressure goals.
2. **Relay parses or transforms application payloads — rejected.** It would make the relay a second semantic authority, increase metadata/content exposure, and couple every adapter to application schemas. The relay reads only the validated outer routing contract; payload remains end-to-end protected and unchanged.
3. **Durable queue for all traffic — rejected.** It violates #2036's explicit no-backlog behavior for ephemeral sessions and creates unbounded custody/privacy obligations. Only already-durable chain events may use bounded delivery assistance.
4. **No store-and-forward at all — rejected as the v1 relay contract.** It would leave the parent-design's durable delivery-assist use case without a relay path. The chosen queue is optional, finite, home-owned, and non-authoritative; graph/range sync remains the recovery mechanism.
5. **Replicate durable queues at every hop or allow independent failover homes — rejected.** It multiplies sensitive copies, creates split-brain ownership and cross-relay commit problems, and makes receipts ambiguous. Transit is volatile; durable queue ownership stays at one logical home.

## 8. Dependencies and explicit boundaries

- **#5808 — wire schema:** owns the postcard envelope, discriminator numbers, extension area, and golden vectors. This record names logical operations only and does not choose their bytes or fields.
- **#5809 — version negotiation:** owns version ranges and forward-compatibility behavior. Relays use the negotiated frame contract; they must not silently reinterpret or downgrade it.
- **#5810 — framing and fragmentation:** owns carrier framing and fragmentation. This relay accepts complete frames up to the existing 1 MiB limit and adds no parallel chunking format.
- **#5811 — WSS/HTTPS adapter:** owns browser-carrier lifecycle details. WSS/HTTPS may carry these operations without changing their semantics.
- **#5795 — trust and broker authorization:** owns capabilities/allowlists, authorization, permitted metadata, and compromised-broker analysis. This record consumes that policy and does not define a second authorization scheme. Until that contract is settled, implementation must fail closed for remote publication, forwarding, and durable storage.
- **#5675 — query design:** owns query execution and filtering. Relay lookup returns route hints only; a relay does not execute, inspect, or filter application queries.
- **#5815 — security binding review:** reviews replay/downgrade and relay-security binding after this protocol decision. It may require an explicit follow-up decision but does not change the ownership above.

**Parent impact:** this record fills in #5674's relay/proxy operating choices and narrows its store-and-forward candidate to durable-class messages. It preserves the parent direction of one canonical ID wire format carried unchanged by multiple adapters. It does not re-decide #2036's origin, hop, or message-class semantics. If the final #5674 record chooses different durable/ephemeral behavior, that disagreement must be made explicit and reconciled before implementation.

## 9. Implementation invariants and conformance checks

An implementation conforms only if it can demonstrate all of the following through the relay interface:

1. A relay never forwards an ID-aware message before validating the origin signature, canonical envelope, group authorization, duplicate status, and hop-loop/limit rules required by #2036.
2. Forwarding does not modify the origin envelope or application payload; only permitted hop metadata is appended.
3. No ephemeral message is persisted for offline delivery. Offline recipients produce an explicit result, and no live queue exceeds its configured bound.
4. `store_accepted` is emitted only after bounded durable-queue commit. The queue deduplicates by stable message ID, expires at the stated finite boundary, and treats a recipient receipt as the only delivery confirmation.
5. Expired/lost durable relay copies remain recoverable through the origin event graph/range-sync path; the relay is never treated as the event's sole authority.
6. A slow recipient produces upstream backpressure or explicit rejection without unbounded memory growth or starvation of unrelated recipients.
7. Full quotas, invalid/expired leases, unauthorized operations, unreachable homes, and invalid frames fail explicitly; no adapter silently downgrades the guarantee or trust policy.
8. Multi-hop forwarding respects the #2036 hop ceiling and loop/duplicate rules. A transit hop cannot create a home lease or an independent durable copy.
9. Lookup responses expose only authorized, expiring routing hints. Health/latency is advisory and cannot confer authority.
10. Golden vectors, message discriminants, frame bytes, and negotiation tests come from #5808–#5810; this record adds no competing byte-level oracle.

The minimum conformance suite should exercise: (a) a direct and multi-relay route with unchanged origin bytes; (b) loop and duplicate rejection at the hop limit; (c) a slow-receiver credit pause and full-queue rejection; (d) durable queue acceptance only after commit, duplicate submission, receipt deletion, expiry, and range-sync recovery; and (e) offline ephemeral rejection with no persistent record. Each check has a decidable oracle: the expected origin-envelope hash/hop list, queue state, explicit protocol result, and/or recovered graph event.

## 10. Evidence map and follow-up

- `doc/2026-11-01_design_id2036.md:54-80,95-96,111` — message classes, origin/hop rules, validation gate, frame cap, and transport/identity separation.
- `pkgs/id/src/frames.rs:1-22` — length-prefix framing and current 1 MiB bound.
- `pkgs/id/src/event_store/ingest.rs:226-269` — event-ingest validation path.
- `pkgs/id/src/event_store/sync.rs:333-373` — durable graph range synchronization.
- `pkgs/id/src/world_session/mod.rs:1-16` and `types.rs:56-64` — transport-neutral session I/O seam.
- `pkgs/id/src/world/mod.rs:57-100,111-160` — bounded world limits as an implementation precedent.
- `doc/2026-11-05T13-38-00Z_design_id5791_transport_negotiation.md:20-35,196-209` — fallback ordering and explicit failure classes.
- `doc/2026-11-05T15-10-00Z_design_cloudflare_workers_rendezvous_relay.md:30-48` — signed rendezvous and blind WebSocket-pair adapter.
- GitHub issues #5674, #5675, #5795, #5808–#5811, and #5815 — parent and delegated design ownership.

This is a protocol design only. Implementation should wait for the parent and sibling wire/trust contracts to settle, then add a relay interface and conformance tests against this record and those owning decisions. The security binding review is explicitly assigned to #5815.
