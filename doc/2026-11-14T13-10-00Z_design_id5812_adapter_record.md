# Adapter decision record — WebRTC, TCP/TLS, local IPC (#5812)

- Status: design decision record for issue #5812 (from parent #5674),
  programme epic #2056. Design only — no code changed. Implementation
  timing is a separate decision (per the issue, possibly months out).
- Sibling records this is written against: #5811 (WSS/HTTPS adapter —
  `doc/2026-11-08T09-45-00Z_design_id5811_wss_https_adapter.md`), #5791
  (transport negotiation & fallback ordering —
  `doc/2026-11-05T13-38-00Z_design_id5791_transport_negotiation.md`),
  #5809 (versioning/negotiation), #5810 (framing & fragmentation),
  #5813 (relay & proxy), #5815 (security binding), #5808 (postcard
  wire schema v1).
- Pattern: the fleet's design-cluster records (#5844, #5843, #5811).

## 1. What is already fixed (not re-decided here)

- **Parent #5674 adapter obligations:** every adapter spec covers
  discovery/addressing, connection lifecycle, MTU/fragmentation,
  ordering and reliability guarantees (and how gaps surface upward),
  NAT/firewall traversal, and how #2036 identity binds to the
  carrier. Hard rules: **TLS is transport encryption only, never a
  substitute for ID end-to-end identity**, and **adapters MUST NOT
  change ID semantics — they only carry frames.**
- **#2036 semantics:** two classes — durable/chained (origin-signed,
  eventual, range-sync recoverable) and ephemeral/live
  (session-scoped, no backlog for disconnected peers). Shared frame
  cap 1 MiB; **transport is not authority** — every path leads to the
  same ID validation.
- **#5791 ordering:** `direct UDP → relay-assisted UDP → TCP →
HTTP/HTTPS proxy`; negotiation state machine and failure
  classification stay with #5791. #5811 settled what the WSS/HTTPS
  tiers are. This record settles what the TCP/TLS, WebRTC, and local
  IPC adapters are — not when the client walks to them.
- No parent decision is changed by this record. One refinement is
  recorded explicitly in §3 (WebRTC's position in the walk) as an
  interface obligation to #5791, not a silent fork.

## 2. The one adapter contract (all three adapters)

Every adapter in this record — and, per #5811, the WSS/HTTPS tiers —
presents the same contract to the session layer:

1. **Frames in, frames out.** The adapter carries postcard frames
   (#5808) unchanged. On stream carriers the framing is #5810's:
   4-byte big-endian length prefix + one postcard frame, ≤ 1 MiB.
   No adapter re-encodes, re-orders across frames, or interprets
   payloads.
2. **Class mapping is declared, not improvised.** Each adapter
   states how the durable/chained and ephemeral/live classes map
   onto it (§§3–5), and surfaces loss/reordering/duplication to the
   session layer as events — an adapter never silently papers over
   a gap the class semantics care about.
3. **Identity binds at the ID layer, always.** Whatever the carrier
   authenticates (TLS peer, DTLS fingerprint, OS peer credentials)
   is transport evidence only. The #2036 handshake runs over every
   adapter, including local IPC, and its result is the only identity
   the session may act on.
4. **Capabilities are advertised** per #5809 negotiation: adapter
   name, class support, max frame size. A peer that does not
   advertise an adapter does not get dialled on it.
5. **Lifecycle is uniform:** dial → ID handshake → frame exchange →
   half-close on class-appropriate drain → close; errors are
   classified into #5791's failure classes so the negotiation
   machine treats every adapter's failures the same way.

## 3. Decision — WebRTC adapter: signalling over ID, data channels per class

- **Shape.** RTCDataChannel between peers, established with ICE.
  Two channels (or one channel pair) per session: a **reliable,
  ordered** channel carries durable/chained traffic; an **unreliable,
  unordered** channel (maxRetransmits 0) carries ephemeral/live
  traffic, with loss surfaced per §2.2 — the class semantics already
  accept a lost uncheckpointed tail, so WebRTC's lossy mode is a
  native fit rather than an emulation.
- **Signalling is not a new service.** Offers/answers and ICE
  candidates are carried as #2036 control frames over any path that
  already works (relay, WSS, TCP) — the dumb-pipe trust model from
  #5791 applies: the signalling carrier learns metadata, never
  content, and cannot impersonate either peer because the ID
  handshake (§2.3) runs inside the data channel once it opens.
- **Traversal.** ICE with STUN; TURN where direct paths fail. A TURN
  relay is a dumb pipe under the #5813 model — it forwards encrypted
  DTLS traffic and holds no ID authority. DTLS encryption is
  transport encryption only (§1 hard rule).
- **MTU/fragmentation.** Data-channel message limits are
  implementation-defined and often well under 1 MiB; the adapter
  fragments per #5810 and reassembles before delivery, and
  advertises its real max message size in capabilities.
- **Position in the walk (explicit refinement, obligation to
  #5791).** WebRTC is attempted **inside the relay-assisted tier** —
  when a peer advertises WebRTC capability and direct UDP has
  failed, WebRTC (which brings its own traversal) is tried before
  falling to the TCP tier. This refines #5791's tier contents; it
  does not change the tier order. #5791's record should absorb this
  line when it is next revised; until then this record is the
  authority for WebRTC's placement.
- **Rejected:** a hosted signalling server as a required component
  (violates the dumb-pipe model and adds a trusted third party);
  WebRTC for server/relay infrastructure paths (its cost buys
  browser reachability and NAT traversal for peers, nothing for
  infrastructure); reliable-channel-only with head-of-line blocking
  for live traffic (defeats the ephemeral class).

## 4. Decision — TCP/TLS adapter: the #5791 TCP tier, length-prefixed postcard over TLS 1.3

- **Shape.** One TCP connection per peer session, TLS 1.3 required
  (no plaintext mode, no TLS < 1.3), carrying #5810 length-prefixed
  postcard frames. This is the adapter #5791's third tier names.
- **Classes.** TCP's ordered reliable stream carries both classes on
  the one connection, frames discriminated by the #5808 envelope —
  no second connection for live traffic; the session layer, not the
  adapter, enforces ephemeral no-backlog semantics (frames for a
  disconnected peer are dropped at the session layer, never queued
  by the adapter).
- **Identity.** TLS authenticates the connection (peer certificate
  or raw public key pinned to the advertised ID key where the
  deployment supports it); per §1, that is never sufficient — the
  #2036 handshake inside the tunnel is authoritative. A TLS peer
  whose ID handshake names a different identity is a hard failure,
  classified per #5791.
- **Addressing/discovery.** `host:port` endpoints advertised through
  the same discovery records as other adapters; the adapter is
  outbound-friendly (works through NAT/firewalls that permit
  outbound TCP) and makes no inbound-reachability promise —
  inbound-only peers rely on the relay tiers, per #5791.
- **Keepalive/timeout.** #2036 control/keepalive frames ride the
  connection; idle timeout and reconnect behaviour are the session
  layer's, surfaced through #5791's failure classification.
- **Rejected:** WebSocket framing for this tier (that is #5811's
  tier, chosen where HTTP infrastructure must be traversed — plain
  TCP pays no HTTP tax where it is allowed); per-class connections
  (doubles dial cost and failure modes for no semantic gain);
  opportunistic plaintext with upgrade (a downgrade surface the
  programme does not need).

## 5. Decision — Local IPC adapter: Unix socket / named pipe, same contract, OS credentials as transport evidence

- **Shape.** Same-machine only: Unix domain socket on POSIX,
  named pipe on Windows, carrying the identical #5810
  length-prefixed postcard stream as TCP. A client dials the
  daemon's well-known endpoint under the per-user runtime directory
  (socket file mode 0600; pipe ACL restricted to the owning user).
- **Classes.** Stream semantics identical to TCP: both classes on
  one connection, session layer enforces class behaviour.
- **Identity.** OS peer credentials (SO_PEERCRED / pipe client
  identity) are checked as a _pre-filter_ — a connection from a
  different OS user is refused before any frame is read — but per
  §2.3 the ID handshake still runs and is authoritative; two ID
  identities sharing one OS account (the common local-dev case)
  must remain distinguishable, which OS credentials alone cannot
  do.
- **No encryption at the transport layer** — the threat model is
  the OS account boundary, and adding TLS here would authenticate
  nothing the account boundary doesn't. This is the one adapter
  exempt from §4's TLS rule, recorded explicitly: the exemption is
  for same-machine IPC only and ends at any proxy/forward boundary
  (an IPC endpoint forwarded over a network becomes a TCP endpoint
  and §4 applies).
- **Lifecycle.** The ID daemon owns the listener; clients (CLI,
  TUI, local apps) dial per session. Stale socket files are
  reclaimed by the daemon on start after a liveness probe.
- **Rejected:** shared-memory / mmap transports (platform-specific,
  breaks the one-contract rule, and optimises a path that is not
  the bottleneck); skipping the ID handshake on IPC because the OS
  authenticated the peer (violates transport-is-not-authority);
  localhost TCP as the local transport (consumes ports, leaks onto
  network namespaces, and invites accidental external exposure —
  the failure #5811's presentation-layer WebSocket code already
  illustrates).

## 6. Implementation invariants

1. A postcard frame is byte-identical on every adapter; conformance
   tests replay the same frame corpus over TCP/TLS, WebRTC, IPC,
   and the #5811 tiers.
2. No adapter buffers or queues frames for a disconnected peer;
   backpressure and backlog policy live in the session layer.
3. Every adapter surfaces loss/reorder/duplicate events upward in
   #5791's failure/gap vocabulary; silent repair is a contract
   violation.
4. Carrier authentication (TLS, DTLS, OS credentials) never appears
   in ID-layer authorization decisions except as transport evidence
   attached to the handshake transcript.
5. Fragmentation is #5810's, once: adapters fragment at their real
   MTU and reassemble before delivery; the session layer never sees
   a partial frame.
6. Capability advertisement (§2.4) gates dialling: no advertisement,
   no attempt — fallback order is #5791's, with §3's WebRTC
   placement.

## 7. Follow-ons

- Implementation timing is a separate decision (per #5812, possibly
  months out); no implementation issues are filed by this record.
- When implementation is scheduled, the natural split is one packet
  per adapter, with §6.1's cross-adapter conformance corpus built
  first (it is the contract's proof).
- Interface obligation recorded for #5791: absorb §3's WebRTC
  placement into the negotiation record at its next revision.
