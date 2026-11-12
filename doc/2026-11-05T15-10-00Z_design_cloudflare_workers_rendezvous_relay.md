# Design: Cloudflare Workers rendezvous/relay component (for #5794)

- Date: 2026-11-05
- Status: **decision recorded; no implementation, deployment, account, or
  Cloudflare resource created.** This record closes the design question in
  #5794. Implementation timing is a separate decision, possibly months out.
- Parent: #5671 (ID transport fallback for UDP-denied networks v1). This
  record inherits #5671's decisions and changes none of them (see
  [Parent boundary](#parent-boundary--5671)).
- Programme: epic #2056. Related, distinct: #2046 (hosted-data edition),
  P7.2 pkarr/DoH lookup (see [Not P7.2](#not-p72)).
- Prior art in this repo:
  `doc/2026-10-28T23-34-14Z_investigation_cloudflare_workers_fit/` (§2.2 the
  Iroh wall; §6 fit map: Iroh relay / DERP-style relaying =
  fits-with-changes as an application-layer WebSocket relay),
  `doc/2026-10-14T10-18-19Z_design_p7_2_cloudflare_workers.md`,
  `doc/2026-10-31T23-00-00Z_design_cloudflare_worker_deployment_standard.md`.
- Platform facts checked 2026-11-05 against Cloudflare's own docs
  (time-sensitive; re-check before implementation):
  [Workers limits](https://developers.cloudflare.com/workers/platform/limits/),
  [TCP sockets](https://developers.cloudflare.com/workers/runtime-apis/tcp-sockets/),
  [Durable Objects WebSockets / hibernation](https://developers.cloudflare.com/durable-objects/best-practices/websockets/),
  [Durable Objects limits](https://developers.cloudflare.com/durable-objects/platform/limits/),
  [Durable Object state](https://developers.cloudflare.com/durable-objects/api/state/).
  Statements drawn from those docs are marked **[verified 2026-11-05]**;
  everything else is an architectural judgment of this record.

## Decision

Adopt a Workers component with **two separate, narrow roles**, and no third:

1. **Rendezvous (directory) role — the primary role.** A Worker over plain
   HTTPS serves and accepts _signed_ rendezvous records: an ID server's
   available transports, broker endpoints, and priority, in exactly the
   record semantics #5671 §3 defines. Records are keyed by endpoint
   identity, verified against the endpoint's key before storage, carry a
   TTL, and are served stale only as last-known-good behind the fresher
   authoritative answer, per #5671.
2. **Application-layer WebSocket relay role — the secondary, bounded role.**
   One Durable Object per relay session (or per endpoint pair) pairs two
   outbound WebSocket connections — client-side and server-side both
   connect _out_ to the Worker over 443 — and forwards opaque,
   end-to-end-encrypted frames between them. This is a rendezvous-then-pair
   broker for the case where _both_ ends are behind UDP-denied / NAT-only
   egress, which is exactly #5671 §2's second broker shape.

The component is **not** a transport peer: it speaks no QUIC, no UDP, and
no native ID wire protocol, and it never terminates ID payload encryption.

### Why this shape

- The platform wall is real and dispositive — for _endpoints_.
  **[verified 2026-11-05]** and repo prior art agree: Workers have no UDP
  socket API and no general inbound TCP listener (inbound TCP reaches a
  Worker only forwarded from another Worker over a service binding);
  outbound TCP exists via `connect()` from `cloudflare:sockets`. A
  Worker therefore cannot be a QUIC/iroh _endpoint_: it cannot join the
  swarm, accept QUIC connections, or hole-punch. **A relay forwarder is
  a different claim, and this record does not conflate the two:** iroh's
  relay protocol is HTTPS/WebSocket-shaped — endpoints connect out to
  the relay and it forwards encapsulated, encrypted packets without
  terminating QUIC — so the absence of UDP does _not_ by itself rule
  out a Workers implementation of relay forwarding (repo prior art §6
  rates exactly this fits-with-changes). Whether the pairing in
  Decision ¶2 can be made wire-compatible with the iroh relay protocol
  — rather than merely equivalent in shape — is an implementation-time
  evaluation this record explicitly leaves open and requires; it is
  not settled, in either direction, by the platform wall.
- What Workers uniquely buys is reachability and zero operations: a
  globally anycast HTTPS/WebSocket endpoint on 443 — the one egress path
  restricted networks reliably allow (#5671's motivating case) — with no
  server to run, patch, or keep awake. That is worth having for rendezvous
  (small, stateless-ish, latency-insensitive) and for pairing two
  already-encrypted endpoints; it is not worth re-platforming ID onto.
- Hibernation makes the relay role economically sane for idle long-lived
  pairings: **[verified 2026-11-05]** the Durable Objects Hibernation
  WebSocket API lets a DO be evicted from memory while its WebSockets stay
  connected, waking on message, with connection state restored from
  `serializeAttachment` data. Idle pairs must cost ~nothing or the relay
  is a standing bill for other people's silence.

## When Workers is the right shape — and when it is not

Right shape when **all** of these hold:

- The traffic being helped is the _fallback_ tiers of #5671 (TCP /
  HTTP-HTTPS), never the native UDP tiers — Workers is not in the path
  when a direct or relay-assisted UDP path exists.
- Both endpoints can connect outbound over 443 (directly or through their
  HTTP proxy), so the pair-by-two-outbound-WebSockets topology applies.
- **The egress path permits WebSocket upgrades, not merely HTTPS
  requests.** These are not the same capability: some proxies and
  middleboxes pass ordinary HTTPS but block or break the WebSocket
  upgrade. A network that allows HTTPS yet blocks WebSocket is _outside_
  this component's supported environment — a real coverage gap in
  exactly the restricted-network population #5671 targets, stated here
  rather than papered over: such networks fall to #5671's remaining
  tiers (HTTP CONNECT proxying to a broker, or a possible future
  polling tier — see Alternatives), not to this relay. (The rendezvous
  role is unaffected: it is plain HTTPS request/response and remains
  reachable from such networks.)
- The payload is already end-to-end encrypted between the true endpoints,
  so a blind frame forwarder satisfies the trust model (below).
- Convenience/default reachability matters more than sovereignty for this
  deployment: an operator is content that a third-party platform sees
  connection metadata and controls availability for this fallback leg.

A self-hosted `id` broker (#5671 §2's native role) is **required** instead
when any of these hold:

- UDP/QUIC _endpoint_ semantics or hole punching are needed, or the
  iroh-relay wire-compatibility evaluation (above) has been run and
  failed, so relaying must run somewhere that can speak the native
  protocol stack.
- Data-residency, sovereignty, or no-third-party-metadata requirements
  apply, or the deployment must survive a Cloudflare account/platform
  decision (suspension, quota, pricing change) — Workers availability is
  someone else's switch.
- Sustained bulk throughput is the point: per-message size and per-DO
  serialization (limits below) make the Workers relay a _usability_
  fallback for interactive and ordinary sync traffic, not a bulk pipe.
- Inbound TCP surfaces (e.g. SSH listeners) are needed; the relay pairs
  WebSockets only, and SSH-class traffic rides _inside_ the E2E tunnel the
  pair carries, terminated at the true endpoints — never at the Worker.

Both shapes are first-class and interoperable: they consume and produce
the same signed rendezvous records and the same #5671 broker contract, so
a client can fall back from one to the other by record priority alone.
That is #5671's open question "in `id`, Workers, or both" answered for
this component: **both, with the self-hosted broker as the sovereignty
and capability superset, and Workers as the zero-ops convenience leg.**
Ship order, if implementation is ever scheduled: rendezvous role first
(it also serves self-hosted brokers), relay role second.

## Limits (verified platform facts, 2026-11-05)

- **No UDP, no inbound TCP/UDP listen.** See above. Consequence: no QUIC,
  no hole punching, no native relay protocol — full stop.
- **Simultaneous outgoing connections: 6 per Worker invocation**
  **[verified 2026-11-05]** (fetch, `connect()`, and outbound WebSockets
  all count while in their opening phase). Consequence: the Worker front
  door must hand pairings to a Durable Object immediately; it must not
  fan out or hold many opening connections per invocation.
- **Hibernatable WebSockets per Durable Object: max 32,768**
  **[verified 2026-11-05]**, with CPU/memory the practical ceiling well
  below that. Consequence: one DO per session/pair (not per fleet) keeps
  blast radius and serialization honest; a per-pair DO is nowhere near
  the cap, by design.
- **WebSocket message size in Durable Objects: 32 MiB, received messages
  only** (Durable Objects limits doc, both storage backends)
  **[verified 2026-11-05]**. Consequence: the platform does not force
  small frames. This record nevertheless recommends an _application_
  frame cap well below 32 MiB (1 MiB is a reasonable starting point) as
  a deliberate judgment — smaller frames bound per-message CPU, memory,
  and head-of-line blocking on a shared pair — and that cap is ours to
  revisit, not a Cloudflare restriction. Above whichever cap is chosen,
  chunking is the _endpoints'_ framing layer's job; the relay forwards
  frames, it does not reassemble streams.
- **Memory 128 MB per Worker/DO; CPU is metered per request/event**
  **[verified 2026-11-05]**. Consequence: forwarding must be
  copy-light, backpressure must close or pause a flooding side rather
  than buffer, and no per-frame CPU-heavy work (no re-encryption, no
  content inspection) is permitted on the relay path — which the trust
  model already forbids.
- **Outbound TCP via `connect()` cannot reach Cloudflare IP ranges or
  private/localhost addresses, and port 25 is blocked**
  **[verified 2026-11-05]**. This record does not use `connect()` at all;
  the note exists so nobody later "simplifies" the relay onto it.
- **Free plan: 100,000 requests/day** **[verified 2026-11-05]**; a real
  deployment assumes a paid plan, and per-pair duration/message metering
  is a real operating cost that the abuse controls below exist to bound.
- Limits, pricing, and the "inbound TCP from other sources" roadmap are
  Cloudflare's to change; the implementation issue, when filed, must
  re-verify every figure in this section before building on it.

## Alternatives rejected

- **Workers as a QUIC/iroh endpoint (peer).** Impossible on the
  platform (no UDP), not merely hard. Rejected by physics, recorded so
  it is not re-proposed. This rejection covers endpoint-hood only: a
  Workers _relay forwarder_ speaking the iroh relay protocol over
  HTTPS/WebSocket is not rejected here — see the platform-wall bullet
  above; its wire compatibility is an open implementation-time
  evaluation, and the generic pairing in Decision ¶2 is the fallback
  if compatibility does not hold.
- **Relay over outbound `connect()` TCP from the Worker to the server.**
  Only helps servers with a public inbound TCP listener — precisely the
  servers that least need help — and cannot pair two NAT-only endpoints.
  The two-outbound-WebSocket pairing is strictly more general for the
  fallback case. Rejected.
- **HTTP request/response polling relay instead of WebSockets.**
  Rejected _as this component's primary shape_, on latency and
  request-metering cost for interactive traffic where WebSocket is
  available — not on reachability: the earlier premise that a WebSocket
  upgrade passes the same egress as ordinary HTTPS is false in general
  (see the supported-environment criteria above), and polling is in
  fact the _more_ reachable shape on HTTPS-only, WebSocket-blocking
  networks. Polling therefore stays a live candidate for a last-ditch
  tier _inside_ #5671's ordering covering that gap — a parent-level
  addition, not this component's choice, and not foreclosed by
  anything in this record.
- **Folding this into the P7.2 Worker.** See [Not P7.2](#not-p72):
  different protocol, different trust posture, different scaling shape.
  Rejected; shared deployment plumbing only.
- **No Workers component at all (self-hosted broker only).** Viable, and
  remains the required shape for the sovereignty cases above — but it
  gives up the one thing this component exists for: a rendezvous/relay
  that is reachable from a UDP-denied network with _zero_ infrastructure
  stood up in advance. Rejected as the whole answer; retained as the
  parity requirement below.

## Self-host parity

Parity is a contract, not a port:

- **Same records, same contract.** Any deployment — Workers or a
  self-hosted `id` node running the broker role — serves the same signed
  rendezvous record format and pairs endpoints under the same #5671
  broker semantics. A client must not be able to tell which shape it
  used except by endpoint address and observed latency.
- **Same trust properties.** The self-hosted broker is equally blind:
  E2E payloads, authenticated use, metadata minimization (below). Running
  your own broker buys metadata _custody_ and availability control, not
  extra payload access.
- **Same configuration surface.** One client configuration names an
  ordered set of rendezvous/relay endpoints of either shape; switching
  from the hosted Workers endpoint to a self-hosted one is a config
  change, never a client change.
- **Deliberate non-parity, stated:** only the self-hosted shape can grow
  UDP/QUIC relaying or inbound-TCP surfaces later. The Workers shape is
  frozen at HTTPS/WebSocket rendezvous + pairing by the platform wall.

## Abuse, authorization, and metadata invariants

Implementation of either role must hold all of these; they are the
acceptance tests of any future implementation issue:

1. **Authenticated by default (#5671 §4).** Rendezvous _writes_ require
   proof of possession of the endpoint key (the record's own signature,
   verified before storage). Relay pairing requires a capability or
   allowlist credential naming both endpoints; there is no open relay
   mode and no anonymous pairing.
2. **Blind pipe.** The relay forwards opaque frames only. It holds no
   session or content keys, performs no TLS termination of ID payloads
   (its own HTTPS/WebSocket TLS is transport, terminated at Cloudflare's
   edge — stated honestly: Cloudflare is trusted for _availability and
   metadata handling_, never for payload confidentiality, which the E2E
   layer provides), and cannot be configured into a mode where it can.
3. **Metadata minimization.** Cloudflare necessarily sees client IPs,
   timing, and volume for connections it carries — that exposure is the
   stated price of the convenience leg. The component itself adds
   nothing on top: no payload logging, no frame-content logging, no
   rendezvous-query logging beyond short-lived abuse counters, and
   retention for those counters is named in the implementation, not
   left as a default.
4. **Compromised-component bound.** Worst case from a compromised or
   malicious Workers deployment is metadata exposure plus denial of
   service (withholding, mis-pairing, stale records) — never content
   exposure and never record forgery: clients verify record signatures
   and endpoint identity end-to-end, so a bad component can fail a
   connection but cannot impersonate an endpoint or read a session.
5. **Abuse economics bounded.** Per-identity and per-IP rate limits on
   record writes and pairing, frame-size and pairing-duration caps, and
   idle-pair reaping are required before any public endpoint exists; an
   unauthenticated or runaway pairing must be cheap to kill and must
   not bill the operator unboundedly (metering facts above).
6. **Stale/spoofed rendezvous safety.** Records carry TTLs; clients
   verify signatures, prefer the freshest valid record across sources,
   and fall back to last-known-good on failure — per #5671 §3, unchanged.

## Not P7.2

The P7.2 design (`doc/2026-10-14T10-18-19Z_design_p7_2_cloudflare_workers.md`)
is a pkarr/DoH _lookup_ service: it stores signed pkarr packets and
answers DoH queries, and it explicitly excludes iroh relay/WebSocket
traffic, QAD, and NAT traversal. This component is a _transport_
rendezvous/relay: signed transport advertisements plus live WebSocket
pairing. They share a platform, a deployment standard, and a
"records are signed, the platform is metadata-trusted only" posture —
and nothing else: not an API, not a store, not a scaling shape (P7.2 is
one DO per public key for lookups; this relay is one DO per live pair).
Neither design's choices bind the other's, and implementing one is not
progress on the other.

## Parent boundary (#5671)

This record makes **no change** to #5671. Specifically inherited as-is:
the fallback order (direct UDP → relay-assisted UDP → TCP →
HTTP/HTTPS proxy), the broker-as-dumb-pipe role, the rendezvous record
semantics and substrate-neutrality, the trust model and E2E list, and the
non-goal of defeating policy-enforced egress the user is not authorized
to use. Two #5671 open questions are _informed_ but remain the parent's
to close in its own text: broker shape (this record argues "both", above)
and the cost/ownership model for a hosted relay (this record bounds it
via the abuse invariants; who pays for a shared hosted endpoint is still
an operator decision). If a future implementation finds it must change a
parent decision, that delta goes on #5671 explicitly — never silently
inside this component.

## Oracle for this decision

Decision-record-only work has no code test oracle. Its checks are:
(a) every platform limit above was read from Cloudflare's primary docs
on the date stated (links in the header); (b) consistency against #5671
(parent boundary section) and against the two named prior-art docs;
(c) the invariants section is written so each item can become a literal
acceptance test of the implementation issue, when one is filed.
