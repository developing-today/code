# Relay trust model & broker authorization — decision record (#5795)

- Status: design decision record for issue #5795 (from parent #5671),
  programme epic #2056. Design only — no code changed.
- Built on, not re-litigating: the merged #5813 relay & proxy record
  (`doc/2026-11-08T11-08-53Z_design_id5813_relay_proxy_protocol/`,
  PR #7653) which records the relay protocol and the #5795/#5815
  security boundaries, and #5791's negotiation/fallback record.
- Pattern: the fleet's design-cluster records (#5844, #5843, #5811,
  #5812).

## 1. What is already fixed (not re-decided here)

- **Parent #5671:** fallback order, broker shapes, the trust model
  at programme level (brokers/relays are untrusted infrastructure),
  and the E2E boundary (content is end-to-end protected between ID
  endpoints; intermediaries carry ciphertext and frames, never
  plaintext authority). Where this record would change a parent
  decision it says so explicitly — it changes none.
- **#5813:** the relay/proxy protocol itself — how frames are
  forwarded, queued, and acknowledged — and the security boundary
  that a relay is a dumb pipe. This record decides the
  _authorization_ layer #5813 assumes: who may ask a broker to do
  what, what the broker may learn, and what compromise buys an
  attacker.
- **#2036 / #5815:** message classes and the security binding —
  durable traffic is origin-signed and range-sync recoverable;
  ephemeral traffic is MAC-authenticated with signed segment
  checkpoints. The compromised-broker analysis in §4 stands on
  those properties, not on broker good behaviour.

## 2. Decision — broker authorization is per-session, endpoint-issued capabilities

A broker holds **no standing authority**. Everything a broker may
do for a session is granted by a capability, issued and signed by
an endpoint (or its home server acting for it), presented at
session setup, and verified by the broker before it acts:

- **Capability contents:** broker identity, the peer pair (by ID
  key), the traffic class(es) it covers, the action set (forward,
  carry signalling, provide traversal/TURN-style relaying), an
  expiry (session-scoped; hours at most), and the issuer's
  signature. Capabilities are non-transferable and cannot be
  widened, subdivided beyond their action set, or re-issued by the
  broker.
- **The allowlist is a deployment property, not a broker
  property.** Each deployment publishes the capability _types_ its
  brokers may be granted. The allowlist can never contain: content
  access or decryption assistance, identity assertion on behalf of
  a peer, durable storage beyond in-flight queueing (#5813's
  bounds), or authorization of other brokers. A capability type
  not on the allowlist is refused even if validly signed — defence
  in depth against a compromised issuer key being used to mint
  novel powers.
- **Revocation is by non-renewal plus delisting:** capabilities
  expire with the session, so eviction is: endpoints stop issuing
  capabilities naming the broker, and discovery stops advertising
  it. No CRL machinery — expiry is short enough that non-renewal
  is the revocation.
- **Rejected:** broker accounts with standing roles/permissions
  (a seized broker would inherit them all); a central authorization
  server brokers must call home to (new trusted party, availability
  coupling, and it learns every session); mutual-TLS-only
  authorization (authenticates the broker's _identity_, grants it
  everything — the exact confusion #5674's TLS rule forbids at the
  ID layer).

## 3. Decision — metadata minimisation: the broker sees routing, bucketed and short-lived

The broker's legitimate need is narrow: forward this frame to that
peer, now. It is given exactly that:

- **Visible:** the envelope routing fields the #5808 schema marks
  as carrier-visible (destination routing token, class, size
  bucket, expiry). Sender/recipient are named by per-session
  pseudonymous routing tokens where the session type allows it;
  where the durable class requires stable origin identity for
  signature verification, that verification happens at the
  endpoint, and the broker forwards the signed frame without
  needing the verified identity as a routing fact.
- **Bucketed, not exact:** frame sizes are padded/bucketed to the
  schema's size classes so content length is not a side channel;
  timing is the broker's own (it cannot be hidden from the
  forwarder) and is therefore treated as known-leak in §4, not
  pretended away.
- **Broker logs are minimised and retention-bounded:** forwarding
  events only (routing token pair, size bucket, timestamp,
  outcome), retained for a deployment-set window (default 7 days),
  never payload-derived data, never a cross-session correlation
  identifier beyond the routing tokens the sessions themselves
  presented.
- **Rejected:** full-fidelity broker telemetry "for debugging"
  (it is precisely the seized-broker prize); client-side-only
  minimisation promises without schema support (the carrier-visible
  field marking in #5808 is what makes this enforceable — an
  adapter that leaks extra fields is non-conformant, per #5812's
  contract).

## 4. Decision — compromised-broker analysis: what malice or seizure buys

Assume the broker is fully malicious or seized, live or after the
fact. The design goal: compromise degrades availability, never
confidentiality or integrity.

**A compromised broker CAN:**

- Drop, delay, reorder, or duplicate frames (liveness/quality
  attacks). Bounded by: #5791 multi-path fallback (peers walk to
  other brokers/paths), #5813 gap surfacing (loss is visible to the
  session layer, not silent), and capability non-renewal (§2) once
  detected.
- Observe the minimised metadata of §3 — routing-token pairs, size
  buckets, timing — and perform traffic analysis within it. This
  is the recorded, accepted leak; §3 exists to keep it this small.
- Refuse service selectively (targeted DoS against a peer pair it
  can recognise by routing token). Bounded by path diversity; a
  broker is never the only route in the #5791 walk.

**A compromised broker CANNOT:**

- Read content: payloads are E2E-protected end to end (#5671
  boundary); the broker holds no keys and capabilities grant no
  decryption assistance (§2 allowlist).
- Forge or alter frames undetectably: durable frames are
  origin-signed; ephemeral frames are MAC-authenticated with
  signed segment checkpoints (#2036/#5815). Alteration is
  detectable at the endpoint and the frame is rejected.
- Impersonate a peer: the ID handshake requires the peer's keys
  (#5812 §2.3); a broker terminating a connection cannot complete
  a handshake as someone else, and carrier authentication is never
  accepted as ID identity.
- Mint or widen capabilities: they are endpoint-signed and
  allowlist-bounded (§2); a broker presenting a self-issued
  capability is refused by peers and other brokers alike.
- Rewrite history: durable state is origin-signed and range-sync
  recoverable from the origin; a broker's in-flight queue is never
  authoritative, and its retained logs (§3) are the _only_ thing a
  seizure yields beyond live metadata — which is why they are
  minimised and short-lived.
- Escalate into other sessions: capabilities are per-session and
  per-peer-pair; one session's compromise grants nothing about any
  other.

**Detection obligations:** the session layer's gap/loss signals
(#5813 surfacing, consumed by #5791's failure classification) are
the compromise detector — sustained broker misbehaviour looks
exactly like sustained broker failure, and the response
(non-renewal + delisting, §2) is the same. No separate
broker-attestation machinery is introduced.

## 5. Implementation invariants

1. No code path lets a broker act without a valid, unexpired,
   allowlisted capability covering that action and peer pair.
2. Carrier-visible fields are exactly the #5808-marked set; an
   adapter or broker build that exposes more is non-conformant.
3. Broker logs contain only §3's fields and are deleted on the
   retention window without operator action.
4. Every CAN in §4 has its stated bound implemented (fallback,
   surfacing, non-renewal); a deployment with a single broker route
   is a known availability exception, recorded as such, never a
   silent default.
5. Capability verification failure is fail-closed and classified
   in #5791's failure vocabulary.

## 6. Follow-ons

- Implementation timing is a separate decision (per #5795,
  possibly months out); no implementation issues are filed here.
- When scheduled, the natural split: capability issuance/
  verification in the session layer; broker-side allowlist +
  minimised logging in the relay implementation (#5813's code);
  the §4 analysis as the security test plan (each CANNOT is a
  test: forge, alter, impersonate, mint, seize-and-read-logs).
