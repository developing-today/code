# Design: adapter — WebSocket/WSS + HTTPS fallback (browser / restrictive-network path)

- Date: 2026-11-08
- Status: **decision record** — design only; no implementation is included in this change.
- Issue: [#5811](https://github.com/developing-today/code/issues/5811),
  "id: design: adapter: WebSocket/WSS + HTTPS fallback (from #5674)".
- Parent design: [#5674](https://github.com/developing-today/code/issues/5674),
  "id: design: Wire format & transport adapters — postcard schema, non-UDP
  carriers, relays/proxies", decomposition item 4. Programme epic:
  [#2056](https://github.com/developing-today/code/issues/2056).
- Sibling designs this record is written *against* (see §1.3):
  [#5808](https://github.com/developing-today/code/issues/5808) (postcard
  wire schema v1), [#5809](https://github.com/developing-today/code/issues/5809)
  (versioning/negotiation), [#5810](https://github.com/developing-today/code/issues/5810)
  (framing & fragmentation), [#5813](https://github.com/developing-today/code/issues/5813)
  (relay & proxy), [#5815](https://github.com/developing-today/code/issues/5815)
  (security binding).
- Already-settled neighbours this record does **not** re-decide:
  [#2036](https://github.com/developing-today/code/issues/2036) messaging
  semantics (`doc/2026-11-01_design_id2036.md`) and
  [#5791](https://github.com/developing-today/code/issues/5791) transport
  negotiation & fallback *ordering*
  (`doc/2026-11-05T13-38-00Z_design_id5791_transport_negotiation.md`).
- Evidence baseline: `origin/main` at `0a27a7f19f`, worktree
  `~/git/fleet/12897-5811-wss-https-adapter`, 2026-11-08. Verified at that
  baseline: no adapter decision record for #5811 exists in `doc/`; the
  WebSocket code that exists (`pkgs/id/src/web/`) is application-specific
  presentation, not the cross-carrier adapter contract.

## 1. What is already fixed (not re-decided here)

### 1.1 From the parent #5674 (adapter obligations, §4 of its body)

Every adapter spec must cover: discovery/addressing, connection lifecycle,
MTU/fragmentation, ordering and reliability guarantees (and how gaps are
surfaced upward), NAT/firewall traversal, and how #2036 identity binds to
the carrier — with the hard rule that **TLS is transport encryption only,
never a substitute for ID end-to-end identity**. Adapters **MUST NOT change
ID semantics; they only carry frames.**

### 1.2 From #2036 (messaging semantics)

- Two classes: **durable/chained** (origin-signed graph events, eventual,
  range-sync recoverable) and **ephemeral/live** (session-scoped
  MAC-authenticated frames with signed segment checkpoints; disconnected
  peers get **no backlog**; the uncheckpointed tail may be lost).
- **No new chunking wire protocol**; the shared frame cap is 1 MiB
  (`pkgs/id/src/frames.rs:18-22`), and larger objects use checked
  chunk/reassembly (the `encryption/welcome.rs` pattern) or
  content-addressed blob references.
- **Transport is not authority** (#2036 invariant 9): every path leads to
  the same ID validation.

### 1.3 From #5791 (negotiation & fallback ordering)

#5791 fixes the attempt order `direct UDP → relay-assisted UDP → TCP →
HTTP/HTTPS proxy`, the dumb-pipe broker trust model, the E2E boundary, and
the per-peer/per-network negotiation state machine with failure
classification. **This record decides what the WebSocket/WSS and HTTPS tiers
*are* and what they cost — not when the client walks to them.** Candidate
ordering, probing, memory, overrides and failure classification stay with
#5791.

### 1.4 Sibling designs not yet settled

#5808, #5809, #5810, #5813 and #5815 are all still open at this baseline
(#5808 blocked, #5810/#5813/#5815 in progress, #5809 queued). This record is
therefore written **against a named draft** of each, as the #5811 triage
(task 10214) required, and every such dependency is stated as an explicit
interface obligation in §6 rather than silently assumed.

## 2. Decision summary (chosen shape)

The browser/restrictive-network path is **one adapter with two carrier
modes over the same session**, selected by a capability probe, not by two
different protocols:

1. **Primary mode — WSS (WebSocket over TLS) as a byte-stream carrier.**
   The adapter is a *stream* carrier in #5810's framing class: it carries
   the existing length-prefixed frames (`frames.rs` `u32`-BE length +
   payload) unmodified, one ID frame per WebSocket **binary** message, with
   no ID-level framing added on top. It reuses the `SessionIo`-style seam
   already proven by `pkgs/id/src/web/world_ws.rs`: the protocol driver is
   transport-neutral and the adapter only moves whole frames.
2. **Fallback mode — HTTPS request/response (POST + long-poll or SSE
   response stream), same session, same frames.** When a network permits
   only plain HTTPS request/response (corporate proxy that blocks
   `Upgrade`, TLS-terminating middlebox that breaks long-lived sockets, or a
   browser/extension policy), the adapter switches to #5810's
   **store-and-forward / chunked request** carrier class: each request
   carries one or more length-prefixed frames, and the response stream
   carries the server→client direction. It is **not** a new protocol; it is
   the same frames with a request-scoped transport envelope.
3. **The fallback is a degradation of *timing*, never of *semantics* or
   *identity*.** Durable traffic is unaffected (it is eventual anyway and
   range-sync recovers it). Live traffic keeps its session ID, sequence and
   MAC; the adapter never reorders, retries a live frame, or synthesises
   acknowledgements. What changes is latency, head-of-line behaviour and
   the cost of reconnect — quantified in §4.
4. **Selection is by probe, then remembered per (peer, network).** The
   adapter offers WSS first; on a classified `upgrade_blocked` /
   `long_lived_socket_blocked` failure it degrades to HTTPS mode for that
   network. The *ordering* of this within the overall candidate list is
   #5791's; the adapter only reports the failure class and the mode it
   supports.

## 3. The adapter contract

### 3.1 Discovery and addressing

- Addressing is an **HTTPS origin plus a session path**, e.g.
  `https://<host>/id/session/<session-id>`, advertised in the peer's signed
  rendezvous record (the #5791/#5671 rendezvous path). No new discovery
  mechanism is invented: the browser cannot do UDP discovery, so the
  rendezvous record is the only viable address source.
- A **relay-backed** origin (an #5813 ID-aware relay) and a **direct**
  peer-served origin are the same address shape; the client cannot tell
  them apart from the URL alone, and does not need to — see §3.5.
- The browser cannot dial arbitrary peers; it can only reach origins that
  serve TLS on 443 and present a certificate the browser trusts. This is a
  hard capability limit, recorded in §7 as a consequence, not a defect.

### 3.2 Connection lifecycle

| Phase | WSS mode | HTTPS mode |
|---|---|---|
| Open | `GET` with `Upgrade: websocket`, subprotocol `id.v1` | `POST /id/session/<id>` with the first frame(s) |
| Authenticate | ID session header frame as the first application frame (never a TLS client cert as identity) | same — first frame in the first POST body |
| Steady state | full-duplex binary messages | client→server: POST batches; server→client: long-poll or SSE response stream |
| Keepalive | WebSocket ping/pong **plus** ID-level keepalive frame | ID-level keepalive frame only; the HTTP request itself is the liveness signal |
| Close | close frame with ID-level reason code mirrored | explicit close frame in a final POST; server ends the stream |
| Reconnect | new WebSocket, ID session resume by session ID + last-seen sequence | new POST chain, same resume rule |

- **The ID session outlives the carrier connection.** A dropped WebSocket or
  an expired long-poll is a *carrier* event; the ID session resumes by
  presenting its session ID and last accepted sequence. This is what makes
  the two modes interchangeable mid-session.
- **Idle and lifetime bounds are explicit and configurable** (the
  `WorldLimits` pattern, `pkgs/id/src/world/mod.rs:111-160`): max payload
  size = `MAX_FRAME_BYTES` (1 MiB, the `encode_frame` payload limit in
  `pkgs/id/src/frames.rs`); the WebSocket message limit is
  `MAX_FRAME_BYTES + 4`, because one binary message carries the whole
  length-prefixed frame (4-byte prefix + payload) — the existing
  `ws.max_message_size(MAX_FRAME_BYTES)` guard at
  `pkgs/id/src/web/world_ws.rs:582` must therefore be raised by 4 when
  this contract is implemented, or a maximum-size valid frame would be
  rejected. Also bounded: max frames per POST batch, max concurrent
  sessions per origin, and a server-side idle timeout.

### 3.3 MTU, framing and fragmentation

- **WSS mode:** the carrier has no MTU problem — WebSocket is a message
  stream over TCP. One ID frame per binary message; the payload ceiling
  is `MAX_FRAME_BYTES` and the message ceiling is `MAX_FRAME_BYTES + 4`
  (the 4-byte length prefix travels inside the message). No
  adapter-level fragmentation.
- **HTTPS mode:** a proxy may cap request body size well below 1 MiB. The
  adapter therefore **splits one ID frame across several POSTs only at the
  #5810 chunk boundary** (the checked chunk/reassembly pattern), never by
  inventing a second fragmentation scheme. Reassembly refuses oversized,
  reordered, altered or digest-mismatched chunks — the #2036 invariant 10
  rule, applied unchanged.
- **No ID-level chunking is added by this adapter.** If #5810's chunk
  format changes, this adapter changes with it; it does not own the format.

### 3.4 Ordering and reliability (what the adapter guarantees upward)

| Guarantee | WSS | HTTPS (long-poll / SSE) |
|---|---|---|
| Per-direction ordering | yes (TCP) | yes within one request chain; **no** across a reconnect |
| Reliable delivery of accepted frames | yes until close | yes for frames the server acknowledged in a response |
| Backpressure | WebSocket send buffer + TCP window | request/response cadence (a new POST only after the previous response) |
| Duplicate delivery possible | no | **yes** on retry after an ambiguous response — dedupe by ID sequence |
| Gap surfacing | connection close / sequence gap → session error | sequence gap after reconnect → session error, same rule |

- The adapter **never reorders** frames it accepted, and **never retries a
  live frame** on its own. Retry of a POST is an ambiguous-outcome case: the
  server dedupes by session sequence, so a duplicate is discarded, not
  re-delivered upward.
- Durable traffic is unaffected by any of this: it is eventual, and a
  missed frame is recovered by range sync (#2036 §5 invariant 4).

### 3.5 Identity and authentication binding

- **TLS is transport encryption only.** The browser's TLS session (and any
  client certificate a corporate proxy injects) is **never** ID identity.
  ID identity is the #2036 session header and per-frame MAC, verified above
  the adapter exactly as on the iroh path.
- A **TLS-terminating middlebox** (the common corporate case) therefore
  breaks confidentiality of the carrier but **cannot forge ID identity** —
  this is the #2036 invariant 9 guarantee, and it is the main reason the
  browser path is acceptable at all.
- **Origin authentication is separate from peer authentication.** The
  client authenticates the *relay/direct origin* by its TLS certificate
  (normal web PKI) and authenticates the *peer* by ID crypto. Conflating
  them is the failure mode this record forbids.
- An #5813 ID-aware relay in front of this adapter sees the same envelope
  metadata #5813 permits and nothing more; the adapter adds no metadata
  beyond the session path.

### 3.6 NAT / firewall traversal

- WSS on 443 traverses virtually every NAT and proxy that permits HTTPS
  egress; this is the adapter's entire reason to exist.
- No hole-punching, no STUN/TURN, no UDP: the browser path is
  **relay-or-direct-TLS only**. Where direct TLS to the peer is impossible,
  the #5813 relay is the path — which is why #5813 must be settled before
  this adapter freezes (§6).

## 4. What the fallback costs (latency and semantics)

The issue asks explicitly for the fallback's cost. These are the costs this
record commits to measuring and accepting:

### 4.1 Latency

| Path | Added latency vs. WSS | Source of the cost |
|---|---|---|
| WSS steady state | ~0 (one TCP/TLS round trip at open) | handshake only |
| HTTPS long-poll | **+1 RTT per client→server batch**, plus poll interval on server→client | request/response cadence; a long-poll only delivers when data or timeout fires |
| HTTPS SSE | +1 RTT per client→server batch; server→client near-streaming | SSE is one-way streaming; the client→server direction is still POST |
| Reconnect (either mode) | +TLS handshake (+1–2 RTT) + session resume | new connection, same session |

- **The honest statement:** HTTPS fallback is acceptable for durable
  traffic and for *low-rate* live traffic (presence, occasional cursors),
  and **not acceptable for high-rate live input** (keystroke-level
  collaboration, live media) — those degrade to unusable latency and should
  be refused or reduced to checkpoint-only mode by the layer above, not
  silently carried.
- Long-poll timeout is a tunable (default: 25 s, below typical 30 s proxy
  idle limits); a shorter timeout costs more RTTs, a longer one risks proxy
  drops. This is a configuration value, not a protocol constant.

### 4.2 Semantics

- **No offline queue is created.** A disconnected live stream loses its
  uncheckpointed tail, exactly as on the iroh path (#2036 §3). The fallback
  does **not** turn live traffic into store-and-forward delivery — that
  would be a semantic change, which the parent forbids.
- **Checkpoints still fire on the same thresholds** (256 frames / 2 s /
  explicit commit). The fallback changes *when* frames arrive, not *what*
  the checkpoint commits to. A long-poll gap can therefore make a segment
  arrive late but never makes it unverifiable.
- **Ordering across reconnect is not guaranteed**; the session resume rule
  and per-frame sequence make a gap *detectable*, which is the guarantee
  #2036 requires (a gap is detectable; a missing tail is not proof of a
  completed action).

### 4.3 Resource cost

- HTTPS mode costs more per frame (HTTP headers, request framing) and holds
  a request open per client — bounded by the per-origin session cap.
- Browsers cap concurrent connections per origin (commonly ~6 for
  HTTP/1.1); the adapter must therefore **multiplex sessions onto few
  connections** in HTTPS mode, which is a reason to prefer SSE (one
  streaming response per session) over many long-polls.

## 5. Alternatives considered and rejected

| Alternative | Why rejected |
|---|---|
| **WebSocket-only, no HTTPS fallback** | Fails exactly the networks the ticket exists for: proxies that block `Upgrade` and TLS-terminating middleboxes that break long-lived sockets. The parent names "most restrictive web environments, corporate proxies" as a required carrier. |
| **HTTPS-only (no WSS)** | Simpler, but pays the per-batch RTT on every frame even where WebSockets work; needlessly degrades the common case. |
| **A separate JSON/HTTP API for the browser path** | Would fork the wire format and violate "adapters MUST NOT change ID semantics" and #2036's no-new-chunking rule. The browser path must carry the same frames. |
| **WebRTC data channel as the browser path** | Lower latency and no relay, but requires signalling, ICE/STUN/TURN, and is blocked in many corporate networks — it is a *different* adapter (parent item 5, #5812), not a substitute for the restrictive-network path. |
| **Treat TLS client certificates / mTLS as ID identity** | Forbidden by the parent: TLS is transport encryption only; a proxy that terminates TLS would become an identity authority. |
| **Store-and-forward queue for live frames during a fallback gap** | Turns ephemeral traffic into durable delivery and creates an offline backlog #2036 explicitly forbids. |
| **Polling with a fixed short interval instead of long-poll/SSE** | Wastes requests and battery, multiplies proxy load, and still cannot beat long-poll latency. |
| **New chunking format for HTTPS bodies** | #2036 invariant 10 forbids a new chunking wire format; reuse #5810's checked chunk/reassembly. |

## 6. Dependencies on sibling designs (explicit interfaces, not assumptions)

This record is **decision-ready against named drafts**; the following must
be confirmed before the adapter freezes. Where a confirmed sibling decision
differs, this record is amended explicitly — it is not silently forked.

1. **#5808 (postcard wire schema):** the adapter carries whatever #5808
   freezes, byte-for-byte. Dependency: the schema must be
   carrier-independent (no field that assumes a datagram or a stream).
2. **#5809 (version negotiation):** the WebSocket subprotocol is `id.v1`;
   the schema version is negotiated **inside** the first ID frames, not by
   the subprotocol string. Dependency: #5809 must not require a
   carrier-level version channel this adapter cannot provide (the HTTPS
   mode has no subprotocol negotiation at all).
3. **#5810 (framing & fragmentation):** WSS is a *stream* carrier (length
   prefix, no extra framing); HTTPS is a *store-and-forward/chunked*
   carrier. Dependency: #5810 must define the chunk boundary this adapter
   splits POST bodies at.
4. **#5813 (relay & proxy):** the relay-backed origin, its auth, and its
   metadata visibility. Dependency: #5813 must permit a WSS/HTTPS front
   end and must state what a TLS-terminating proxy may see.
5. **#5815 (security binding):** TLS-terminating-middlebox behaviour,
   downgrade protection (forcing HTTPS mode to weaken something), and
   replay rules across reconnect. Dependency: #5815 must confirm that
   carrier downgrade is not an ID-level downgrade.

**Parent-change check (§8 of the #5811 triage):** no decision above changes
a #5674, #2036 or #5791 decision. Two parent *open questions* are touched
and recorded rather than forked:

- #5674's "do browser-facing adapters need a JSON/CBOR diagnostic variant?"
  — **not needed for this adapter**: the fallback is an HTTP transport for
  the same binary frames, not a JSON protocol. A diagnostic variant, if
  wanted, belongs to #5808.
- #5674's "who owns fragmentation" — this adapter defers entirely to
  #5810 and adds none.

## 7. Invariants implementation must hold

1. **Same frames, both modes.** A frame encoded for the iroh path decodes
   identically after passing through WSS or HTTPS mode; no adapter-level
   re-encoding, no field added or dropped.
2. **TLS is never identity.** No code path derives ID identity, authorization
   or session membership from a TLS session, client certificate, proxy
   header or origin hostname.
3. **The session outlives the carrier.** Reconnect resumes by session ID +
   last-seen sequence; a carrier drop never silently starts a new ID
   session or renumbers sequences.
4. **No live retry, no live queue.** The adapter never retries a live frame
   on its own and never buffers live frames for a disconnected peer.
   Ambiguous POST outcomes are deduped by sequence, not re-delivered.
5. **Bounded everything.** Payload ≤ `MAX_FRAME_BYTES`; WebSocket
   message ≤ `MAX_FRAME_BYTES + 4` (prefix included); POST batch ≤ the
   configured cap; sessions per origin ≤ the configured cap; idle timeout
   enforced. An oversized or malformed message closes the session with a
   defined error, never panics or hangs (the #5674 conformance rule).
6. **Gaps are surfaced, not hidden.** A sequence gap after reconnect is
   reported to the layer above as a session error; the adapter never
   fills it.
7. **Checkpoint thresholds unchanged.** The adapter does not alter the
   256-frame/2-second/commit checkpoint triggers.
8. **Downgrade is visible.** A switch from WSS to HTTPS mode is recorded in
   diagnostics with the failure class that caused it; it is never silent.
9. **No new chunking format.** HTTPS body splitting uses #5810's checked
   chunk/reassembly; reassembly refuses oversized, reordered, altered or
   digest-mismatched chunks.
10. **Both modes pass the same conformance suite** as the iroh path
    (golden vectors, round-trip, negative tests) plus the interop matrix
    case "client on WSS/HTTPS, peer on iroh, via relay".

## 8. Consequences and follow-on work

- **High-rate live input over the HTTPS fallback is a degraded mode.** The
  layer above must be able to ask the adapter for its mode and reduce or
  refuse high-rate live traffic accordingly. This is a small interface
  addition, not a protocol change.
- **Browsers cannot be first-class ID peers without a relay or a
  peer-served origin.** This is recorded as a known limitation of the
  browser path; direct browser-to-peer TLS requires the peer to present a
  browser-trusted certificate, which is rare.
- **Implementation is out of scope** for this record (the issue makes
  implementation timing a separate decision). The follow-on implementation
  work is tracked by the linked implementation issue below.

## 9. Evidence map

- **Parent obligations:** #5674 body §4 (adapter spec fields, TLS-is-not-identity)
  and §1/§3 (framing per carrier class, postcard schema).
- **Messaging semantics:** `doc/2026-11-01_design_id2036.md` §3 (classes),
  §4.3 (session/checkpoint), §4.5 (no new chunking, 1 MiB cap),
  §5 invariants 5, 6, 9, 10.
- **Negotiation/ordering (not re-decided):**
  `doc/2026-11-05T13-38-00Z_design_id5791_transport_negotiation.md` §1.1
  (attempt order), §2 (state machine), §6 (failure classes).
- **Code precedent for the transport-neutral seam:**
  `pkgs/id/src/world_session/mod.rs:1-16` (protocol owns behavior, transport
  only moves frames), `pkgs/id/src/world_session/types.rs:56-64`
  (`SessionIo`), `pkgs/id/src/web/world_ws.rs:582` (1 MiB guard),
  `pkgs/id/src/web/world_ws.rs:595-617` (`WsIo` implementation).
- **Frame format and cap:** `pkgs/id/src/frames.rs:1-22`.
- **Chunk/reassembly precedent:** `pkgs/id/src/encryption/welcome.rs:1-19, 31-48`.
- **Bounded-limits pattern:** `pkgs/id/src/world/mod.rs:111-160`.
- **Prior triage:** #5811 comments — task 10214 triage (2026-11-05,
  ordering blocker and scope) and task 12312 re-triage (2026-11-08,
  real-and-current, no decision record).
