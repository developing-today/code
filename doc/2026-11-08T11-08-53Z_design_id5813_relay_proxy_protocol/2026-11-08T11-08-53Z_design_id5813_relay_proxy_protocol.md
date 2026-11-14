# Design: ID relay and proxy protocol

- Date: 2026-11-08
- Status: **decision for the relay-service boundary**; wire integration is gated on the parent and sibling contracts below.
- Issue: #5813, child of #5674; programme #2056.
- Scope: application relay/proxy roles, routing, delivery, resource bounds, and service scaling. No implementation changes are included here.

## Decision

Use an **ID-aware, end-to-end opaque relay service** with separate discovery/lookup and publish/home-relay roles. The service understands authenticated routing and resource-control metadata, but it is not the source of truth for ID messages and does not become a general-purpose durable message store.

Direct connectivity remains the first path where available. A home relay can bridge a live connection while both sides are connected. Durable application facts remain in the signed event graph defined by #2036 and are recovered through its synchronization path; the relay does not retain and replay them. Transient live traffic is best-effort and is not queued for offline peers.

Lookup and publication are distinct operations. A client may query a bounded set of lookup services concurrently and may publish the same signed presence/route advertisement to more than one configured home relay. Each home relay grants an independent, expiring registration lease; partial success is visible and does not imply that every configured relay is current.

Relay queues and connection state are finite and volatile. Backpressure is explicit; a relay that cannot accept within its configured limits refuses or closes the affected stream rather than silently dropping data or growing an unbounded queue. A relay receipt confirms only the named transport step, never durable storage or application acceptance.

This decision does **not** define another application envelope, change #2036's message-validity or retention rules, or settle the pending parent wire-format and adapter decisions. Those are integration constraints, not permission to fork their contracts.

## 1. Boundaries and dependencies

### Existing application contract: #2036

`doc/2026-11-01_design_id2036.md` already owns application-message classes, the immutable origin-signed envelope, member-relay validation-before-forward, hop/loop metadata, live-session semantics, durable graph retention, and the shared 1 MiB frame limit. This record consumes those decisions:

- The relay service cannot make an invalid origin message valid, change its stable identity, or confer group membership.
- A member that forwards application messages still applies #2036's validation-before-forward and hop rules. This document does not create a second hop chain or a second message ID.
- Durable facts and checkpoints are recovered from the event graph; ephemeral live frames are not replayed after a disconnected session.
- Framing and checked chunking stay with the shared ID frame contract. No relay-specific chunking format is introduced.

The parent issue #5674 is still OPEN and its remainder is in progress (task 12244 was freshly handed off when this record was prepared). This record chooses the relay-service boundary only. It does not decide the postcard envelope, adapter architecture, or any other parent-owned wire choice.

### Sibling contracts

Fresh issue state on 2026-11-08 showed #5808 OPEN/blocked on #5674, #5809 OPEN/queued, and #5810 OPEN/queued. Their decision records have not settled the shared schema, version negotiation, or carrier framing. Consequently:

1. Relay control/data messages use the canonical envelope and framing selected by #5674, #5808, and #5810; this record allocates no discriminants, byte layout, or independent framing rules.
2. The version/capability negotiation and unknown-field behavior come from #5809. A relay must not silently discard a field that that contract marks critical.
3. The service behavior below is the decision to carry into those records. If a parent/sibling decision makes an interface detail incompatible, record the conflict explicitly on the relevant issue and revise this record additively; do not silently override the parent.

Issue #5795 owns broker authorization, allowlists/capabilities, metadata minimization, and the compromised-broker ceiling. It was OPEN/parked at verification. This design consumes that trust contract rather than restating its authorization policy. The relay interface assumes end-to-end protected application payloads and exposes only the routing/control metadata necessary to operate; #5795 must settle the exact authorization and metadata surface. Issue #5815 is the subsequent cross-adapter security-binding review and was already in progress; this record does not pre-empt it.

## 2. Relay roles

### Lookup service

A **lookup service** resolves a node or service identity to signed, time-bounded route advertisements. It is not the node's home, does not accept publication on the node's behalf, and does not own the returned identity or message. Its result is a set of candidates, not an authorization decision. The client verifies the advertisements and applies the normal ID trust and connection rules.

Lookup is a replaceable service boundary: the protocol does not require one operator, one DNS zone, or `iroh.computer`. Public defaults may be convenient, but deployments can configure their own lookup services and relay set. Resolver discovery and default selection remain with #5674; this record requires only that lookup be addressable/configurable rather than hard-coded as an unreplaceable authority.

For a lookup, query a configured, bounded fan-out concurrently rather than serially waiting on every endpoint. Merge valid responses until the caller's deadline, deduplicate equivalent signed advertisements, and prefer a usable direct route when one is available. A slow, unavailable, malformed, stale, or unauthorized response contributes no candidate and must not block other responses. The configured fan-out is finite and may be tuned by deployment; it is not a wire-format constant.

### Publish/home relay

A **home relay** is an endpoint to which a node publishes a signed, expiring route/presence advertisement and through which an authorized peer can establish a live proxied connection when direct connection fails. Registration, renewal, and withdrawal are authenticated under the capability/authorization rules owned by #5795. A route advertisement is not a message and does not grant authority to send one.

The home relay holds a lease mapping an authorized identity/service to the currently reachable connection or endpoint. It may forward live traffic only while the relevant endpoint/session is connected and authorized. It may not turn that lease into durable message retention. Leases expire without renewal; client restart or relay restart requires a fresh publication.

The relay may see the routing identifiers, endpoint/lease state, timing, traffic volume, and the protocol metadata required for resource enforcement. Application payloads remain opaque to it. The exact broker-observable metadata and capability checks are finalized by #5795 and reviewed across carriers by #5815.

### Multiple relays

Lookup sets and home-relay sets are configured independently; a node need not publish at every service it queries.

- **Lookup:** issue bounded concurrent queries and combine verified answers. Do not make one lookup provider authoritative or force sequential traversal of the whole configured list on the critical path. Additional endpoints may be tried within the request deadline when the initial fan-out yields no usable result.
- **Publication:** a node may publish to multiple home relays. Publish/renew/withdraw independently to every configured home in parallel. Each successful response represents only that relay's lease. One successful home is sufficient to advertise the route through that home; failure of another is reported as partial/degraded publication, not hidden as full success.
- **Use and recovery:** peers may use any currently valid advertised home. If one home fails, try another valid advertisement or the direct route. Do not merge relay-local leases into a global truth claim. After all leases expire, the node is simply not discoverable through those homes until it republishes.

This is active multi-home publication, not quorum publication: there is no global registry transaction or cross-relay consistency protocol. Independent signed advertisements and expiry make partial availability explicit without creating a central coordinator.

## 3. Delivery, receipts, and retention

The protocol distinguishes three events that must not be conflated:

1. **Accepted by relay:** the relay admitted bytes into its bounded volatile forwarding window. This is not a delivery or persistence guarantee.
2. **Forwarded by relay:** the relay handed the bytes to the next live transport connection. This is not evidence that the destination parsed, authorized, or applied them.
3. **Acknowledged by destination application:** an end-to-end protocol response, when the message class defines one, says the destination reached that class's stated acceptance point. The relay cannot synthesize this acknowledgement.

For a durable fact, the sender relies on the signed event and its application-level confirmation; a relay receipt is only a hop-level observation. If the recipient is offline or a live route disappears, the relay drops the volatile forwarding state. The sender/recipient later reconcile durable facts through the #2036 event-graph sync contract. The relay does not promise exactly-once delivery; retransmission and deduplication use the existing message identity and application semantics.

For transient/live traffic, forwarding is best-effort only for the lifetime of the connected session. A disconnect can lose in-flight and uncheckpointed frames. No offline mailbox, disk spool, replay window, or relay-owned retention period is provided. This matches #2036's distinction: durable facts are retained by the event graph, while live frames do not become durable merely because they traversed a relay.

Only route leases and bounded live connection/flow-control state are retained by the relay, in volatile memory and only until expiry or session close. Lease TTL/renewal values are deployment configuration negotiated under the shared version contract; payload retention is always zero. Restart loses leases and live sessions, after which clients reconnect and republish. There is no hidden disk-backed fallback.

## 4. Backpressure and quotas

Every implementation has finite limits at both connection and relay scope. Flow control uses explicit byte credit: a sender may not exceed the receiver's outstanding credit, and the relay returns credit only as bounded buffer space is released. A relay never reads an unlimited stream into memory while waiting for a slower destination.

The required quota dimensions are:

- maximum frame size, using the existing #2036 limit of 1 MiB (or a lower negotiated limit);
- maximum bytes in flight per proxied stream and aggregate per connection/identity;
- maximum concurrent proxied streams/connections per identity;
- maximum lookup fan-out and in-progress lookups per identity;
- maximum live home leases and renewals per identity; and
- aggregate relay memory/connection ceilings, with fair scheduling so one identity cannot consume the whole service.

All limits are hard, finite, configurable deployment values. The protocol negotiates or reports the effective limits using the shared #5809/#5810 contract; it does not encode an unbounded default. A stream that exhausts credit pauses until credit returns. A request that exceeds a quota receives a typed resource-exhausted/refused outcome; it is not silently truncated, persisted, or retried forever. Repeated abuse may be throttled or disconnected under #5795's authorization/abuse policy.

The frame cap is the only numeric limit fixed here because it is already owned by #2036. This record deliberately does not invent universal connection counts, memory budgets, or lease durations: those depend on deployment capacity and must be selected as finite configuration with the effective values observable to clients/operators.

## 5. State model and scaling

Choose **stateful-at-the-edge, stateless-across-relays**:

- Each home relay keeps only in-memory identity-to-lease bindings, live connection/session bindings, and bounded flow-control buffers.
- A relay instance owns its active connection state. Requests for a live route must reach that instance (connection affinity or an equivalent local routing mechanism); there is no shared durable session database.
- Lookup can be replicated or load-balanced because it returns signed, expiring advertisements and does not own payloads or application truth.
- Multiple home relays provide explicit client-driven redundancy. They do not share lease or queue state. A relay restart loses only its own leases and live sessions; the client republishes/reconnects to the remaining homes.
- Durable message recovery remains peer/event-graph synchronization, not relay failover or queue replay.

A wholly stateless proxy is rejected because live forwarding requires connection and flow-control state. A durable shared relay database is also rejected: it would make availability, retention, deletion, and broker compromise part of the application-data contract, duplicating the signed event graph and the separate #5795 trust design.

## 6. Alternatives considered

| Option                                                          | Decision   | Reason                                                                                                                                                                                                                                      |
| --------------------------------------------------------------- | ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Dumb opaque byte pipe with no ID-aware routing/control          | Reject     | It cannot distinguish lookup from home publication, enforce identity-scoped quotas, or report partial multi-home registration. Payload opacity is retained, but the service still needs authenticated routing and bounded resource control. |
| ID-aware relay with bounded live forwarding and expiring leases | **Choose** | Supports lookup/home-role separation, multiple independent homes, explicit receipts and backpressure, while keeping application validity and durable truth with ID peers.                                                                   |
| Durable store-and-forward broker for all traffic                | Reject     | Duplicates event-graph retention/sync, creates deletion and replay obligations, and makes broker storage/trust part of message validity. Live traffic is intentionally not replayed.                                                        |
| One mandatory home relay/provider                               | Reject     | Creates a discovery and availability single point of failure and prevents private/self-hosted deployments.                                                                                                                                  |
| Fully stateless relay with no session state                     | Reject     | Cannot implement live connection mapping or bounded per-stream flow control. Keep the state local, volatile, and bounded instead.                                                                                                           |

## 7. Invariants and implementation acceptance

1. **No new authority:** a route, relay receipt, transport identity, or successful lookup never grants membership or changes the validity of an origin-signed message.
2. **One application contract:** member forwarding, message IDs, hop limits, event retention, and live/durable classification follow #2036; this service adds no competing envelope or hop metadata.
3. **Separate roles:** lookup-only endpoints do not become home relays; publication to a home does not make it the global discovery authority.
4. **Explicit multi-home state:** each home lease succeeds, fails, renews, and expires independently; partial publication cannot be reported as full publication.
5. **Honest receipts:** relay acceptance/forwarding receipts are hop-local; only the destination application can produce an application acknowledgement.
6. **No relay payload retention:** data-plane buffers are volatile and bounded. Offline durable catch-up uses the event graph; offline live frames may be lost.
7. **Bounded resources:** every queue, stream, lookup, lease set, connection, and aggregate buffer has a finite cap and an explicit refusal/backpressure path.
8. **Restart is recoverable:** after relay restart, clients can detect expired/lost leases and republish; no durable message is lost solely because it was entrusted to a relay queue, because no such queue exists.
9. **Version and framing are shared:** unknown critical fields are handled per #5809; carrier framing and size limits follow #5674/#5810/#2036. No silent downgrade or relay-specific chunking is introduced.
10. **Security contract is reviewable:** broker authorization and metadata exposure are specified by #5795 and cross-adapter security binding by #5815 before implementation freezes those surfaces.

The implementation/conformance work should test concurrent lookup with an unavailable endpoint, multi-home partial publication and lease expiry, duplicate advertisements, slow-consumer credit exhaustion, quota refusal, disconnect loss of live frames, restart/republication, and the separation between relay receipts and destination acknowledgements. #5814 owns the broader adapter conformance/interop matrix.

## 8. Decision ledger and follow-up

- **No parent decision is changed.** #5674 remains the owner of the shared wire-format family, transport-adapter model, resolver defaults, and any parent-level discovery decision. This record contributes the relay service boundary and makes relay lookup configurable/self-hostable rather than selecting a global resolver.
- **No #2036 decision is changed.** Its application envelope, member-relay validation, hop/loop rules, durable event graph, live-session behavior, and frame cap remain authoritative.
- **No #5795 trust policy is duplicated.** Broker identity/capability rules, allowlists, metadata minimization, and compromised-broker analysis remain for #5795; this design requires those decisions to bind the relay operations above.
- **No wire details are preempted.** #5808, #5809, and #5810 must map these operations onto the shared message schema, negotiation, and framing. Their currently blocked/queued state is an integration gate, not a reason to silently invent a second format here.
- **Review order:** carry this contract to #5815 for the planned security-binding review and to #5814 for conformance coverage. The security review should confirm the opaque-payload and metadata assumptions against #5795 before implementation interfaces are frozen.

## Evidence map

- #5813 body and its 2026-11-05 parent cross-link comment: distinct relay/proxy design; relay lookup vs. publish/home roles and multi-relay behavior.
- #5813 task 10216 triage (comment `issuecomment-6662324895`) and task 12313 triage (comment `issuecomment-6717275182`): the design is additive to #2036, consumes #5795, aligns with #5808–#5810, and was re-verified as real/current and not gated on 2026-11-08.
- #5674 comments `issuecomment-6647788157` and `issuecomment-6662707975`: original multi-relay questions and the parent-level ordering/boundary relative to #2036.
- `doc/2026-11-01_design_id2036.md`: existing application relay, message lifecycle, framing, and retention contract.
- Fresh GitHub state checked 2026-11-08: #5674 OPEN/parked with task 12244 handed off; #5808 OPEN/blocked; #5809 and #5810 OPEN/queued; #5795 OPEN/parked; #5815 OPEN/in progress. These states are recorded to expose integration order, not to claim that the sibling decisions have landed.
