# iroh-native ID transport in fleet — identity, postcards, format conversion

Date: 2026-11-24T05-55-51Z
Status: design record; no implementation authorized
Tracking issue: [developing-today/code#10391](https://github.com/developing-today/code/issues/10391)
Parent mill: [developing-today/code#10389](https://github.com/developing-today/code/issues/10389)
Queue task: 21681 (slice 1 — write/upgrade this design to the 9-point standard)

This is the design of record for pass 2 of the fleet-surface design mill:
fleet as an iroh-native client/initiator of the ID system, plus the two
proxy modes that cover what pass-1 pass-through cannot. It does not replace
pass 1 ([#10390](https://github.com/developing-today/code/issues/10390):
endpoint/credential resolution and raw mode) or pass 3 (ID binary REST
session + presentation model); where the two meet, §4.5 defines the seam
once and both passes reference it.

## 1. Problem and context

ID's native transport is iroh. Pass 1 of the mill (plain pass-through to
the same binary/endpoint with automatic endpoint and credential resolution)
cannot cover it: reaching an ID store that is remote-by-configuration
requires full iroh credentials and protocols on the fleet side — keypair,
relay/rendezvous dial, ALPN-selected QUIC streams — not an HTTP URL plus a
bearer token.

Evidence that this gap is real and load-bearing:

- The pass-2 issue text names the two levels explicitly: raw proxy
  (byte pipe, plus a relay-aware variant that uses iroh for transport
  while staying payload-opaque) and fleet-as-initiator (credential
  handling, byte streaming on stdout/stderr-style streams, postcard-native
  mode, conditional conversion to MessagePack/CBOR served over WebSocket
  or WebTransport for browser-capable frontends/backends).
- Iroh probe 2 (2026-11-15, reported on #10391): sandbox ↔ unmodified
  laptop iroh works through our own relay plus a sandbox-side CONNECT
  shim. The wall is precisely characterized — every sandbox TCP connection
  is redirected to a CONNECT-only HTTP proxy; stock iroh's relay dialer
  starts TLS immediately and dies; resolution is not the blocker. The
  working shape is sandbox-initiated through the shim, both directions of
  payload verified sha-identical (4 MiB, 10 MiB at ~0.23 MB/s — control
  class, not bulk pipe). Consequence for this design: the
  sandbox/egress-constrained caller MUST be the iroh initiator; the peer
  needs nothing special. That is exactly the level-2 shape.
- iroh core is a stable bet, not a moving target: 1.0 froze the wire
  protocol 2026-06-15; current is 1.5.0 (2026-11-19); the supported
  toolkit for new protocol work is blobs + gossip + irpc 1.0
  (`doc/2026-11-23T22-00-30Z_research_iroh-ecosystem/`). Fleet does not
  need to hedge the transport choice; it needs to own the client-side
  key lifecycle and the payload-shape boundary.
- ID already speaks postcard over iroh QUIC streams: `id-sync`
  `protocol.rs` defines the `MetaRequest`/`MetaResponse` meta protocol
  (ALPN `/iroh-meta/3`) with `postcard::to_allocvec` / `from_bytes` as
  the encode/decode pair, one request/response per bidirectional QUIC
  stream, and an explicit compatibility rule — postcard encodes enum
  variants by position, so variant order is wire-significant. Any fleet
  initiator must honor that rule and must not silently reframe it.
- Fleet already decided the adjacent transport question for mcpx
  (`doc/fleet/mcpx-identity-transport.md`): tsnet won there because mcpx
  is pure-Go with a CGO_ENABLED=0 release matrix. Fleet itself is Rust,
  so that constraint does not apply here — native iroh (the same `iroh`
  1.x crate family ID uses) is available without an FFI or sidecar tax.
  This design therefore assumes in-process iroh in the fleet binary.
- Context propagation ([#10357](https://github.com/developing-today/code/issues/10357))
  already plans the `FLEET_CTX_*` env contract, init-sourcing, and
  stamping of queue items with session/model/task identity. The
  per-session iroh keypair in this design extends that stamping, it does
  not invent a second one.

What breaks today without this design: any fleet command that must read
or write a remote ID store has no specified key to dial from, no rule for
whether fleet terminates the iroh connection or just pipes bytes, no rule
for when postcard is kept versus converted, and no browser-reachable
serving shape — so each caller would improvise all four, and
improvisations will diverge on key storage, framing, and encoding.

## 2. Goals and non-goals

### Goals

- Specify the identity/key lifecycle: generation, stamping, env-var
  exposure, storage, rotation/disposal.
- Specify the raw-proxy vs initiator modes, with a decision rule for when
  each is chosen (including the relay-aware proxy variant).
- Specify postcard-native handling and the exact conditions under which
  fleet converts to MessagePack/CBOR.
- Specify the WebSocket/WebTransport serving shape on the fleet side.
- Define the seam with pass 3 (ID binary REST serving side) exactly once.

### Non-goals

- No implementation, no refactor, no relay operation, no pkarr/relay
  infrastructure decision beyond consuming the existing relay (laptop
  `relay.security.cab` + public relays as configured). Relay selection
  policy belongs to the fleet-transport design
  (`doc/2026-11-23T21-10-36Z_design_fleet-transport-pooling/`).
- Pass-1 mechanics (subcommand tree, resolution order flag > settings >
  standard local endpoint, credential sources per service, raw-mode
  syntax) are inputs, not re-decided here.
- Pass-3 mechanics (ID REST session model, streaming resume, web
  presentation/bundles, terminal rendering) are not designed here; only
  the seam is.
- Passes 4–7 (MCP aggregation, embedding, fork upkeep, beyond-GitHub)
  are out of scope.
- No change to ID's postcard wire format, ALPN identifiers, or enum
  variant ordering. Fleet adapts to ID; ID does not move for fleet.
- No production key migration, no secret-store schema change, no queue
  broker change beyond stamping fields this design names.

## 3. Alternatives considered

| Alternative | Benefits | Costs / risks | Decision |
|---|---|---|---|
| Do nothing; leave iroh handling to ad-hoc callers and the ID binary | Zero design/implementation cost now; ID binary already dials iroh correctly when invoked directly. | Every fleet caller improvises keys, framing, and encoding; sandbox-constrained callers cannot reach remote ID at all (probe-2 wall); no browser path; context stamping and iroh identity diverge into two identity systems. Fails the mill's "one fleet surface" goal. | Rejected as the end state. Tolerated only as the interim until slice 2 lands. |
| Raw-proxy only (fleet never terminates iroh; always pipes bytes, optionally via a relay-aware iroh dial) | Smallest fleet surface; no postcard/CBOR knowledge in fleet; ALPN and enum-order fragility stay inside ID. | Cannot stamp, inspect, or convert; cannot serve browsers (no WS/WTransport without terminating); cannot do byte-streaming on stdout/stderr-style streams with fleet-side flow control; the CONNECT-shim lesson (initiator must own the dial) has nowhere to live. Leaves the issue's level 2 entirely unmet. | Rejected as sufficient; retained as one mode (see §4.2). |
| Always-initiator (fleet always terminates iroh, no proxy modes) | Single code path; uniform observability and conversion point. | Forces fleet to track every current and future ID ALPN/protocol (meta `/iroh-meta/3` today, blobs/gossip/irpc tomorrow); breaks opaque forward-compat for protocols fleet does not know; heavier than needed for pure byte moves and for the "something on the far end speaks the protocol" relay case. | Rejected. Both modes are needed; §4.2 gives the selection rule. |
| Sidecar iroh process (fleet shells out to the ID binary or a helper for every iroh dial) instead of in-process iroh | Isolates iroh version churn; mirrors the mcpx-sidecar fallback reasoning. | A second process to supervise per session; doubles the key-handoff problem (fleet must still pass the secret to the sidecar); latency and stream-plumbing cost on every call; no toolchain win (fleet is already Rust on iroh 1.x, unlike mcpx's pure-Go constraint). | Rejected. In-process iroh; fleet may still exec the ID binary where one exists (see §4.6), but the initiator path is a library, not a subprocess contract. |
| Postcard-always (never convert) | One encoding end to end; no conversion bugs; smallest spec. | Browsers have no postcard decoder and would each hand-roll one; large streams/file transfers buffer whole values (`from_bytes` semantics) instead of streaming; postcard's positional enum encoding makes cross-version skew silent-corrupt rather than loud-fail at the browser boundary. | Rejected. Postcard stays native fleet↔ID; conversion happens at the browser boundary under the conditions in §4.4. |
| Convert-always (fleet normalizes everything to CBOR/MessagePack internally) | Uniform internal representation; browser-ready by default. | Pays conversion on every call including pure Rust↔Rust paths that want postcard; risks semantic drift between the postcard schema and the converted schema; violates "fleet adapts to ID" by making CBOR the de-facto ID wire shape. | Rejected. Conversion is edge-triggered (§4.4), never the internal canonical form. |
| Serve browsers from the ID binary directly (no fleet-side WS/WTransport) | One fewer server surface in fleet. | Contradicts the pass boundary the issue sets (serving side for ID apps belongs to ID's REST mode, pass 3; fleet's client/initiator side is this pass); forces browsers to solve iroh dial + keys + CONNECT-shim traversal themselves, which probe 2 shows they cannot in constrained egress. | Rejected. Fleet serves WS/WTransport on its side (§4.5); ID serves its own REST side in pass 3; the seam is shared. |

## 4. Chosen approach

### 4.1 Identity and key lifecycle

- **Default: fleet generates one ephemeral ed25519 node key per session.**
  Generation happens at session start (the same point where #10357
  captures the context record), via the OS RNG through the `iroh`
  key type — never a hardcoded, checked-in, or logged key.
- **Caller override:** the caller may supply a secret key (`--id-secret`
  / `FLEET_ID_SECRET`, exact flag/env names are a pass-1 slice detail;
  both spellings must exist). Fleet uses it verbatim and never persists
  it. Absent the override, the ephemeral key is used.
- **Stamping:** the session public key (NodeId, 32-byte ed25519) travels
  everywhere the #10357 context stamp travels — attached to queue items
  fleet picks up, and exported into the environment when the stamp is
  dot-sourced (`FLEET_ID_NODE_ID` alongside `FLEET_CTX_*`). The secret
  key is exported ONLY as `FLEET_ID_SECRET` in the dot-sourced
  environment of the owning session; it is never attached to queue
  items, logs, trip records, or error messages.
- **Storage:** ephemeral secrets live in process memory only, zeroized on
  session end. No disk write, no secret-store write, no relay-side
  storage. A caller-supplied key is likewise never written by fleet;
  its persistence is the caller's business.
- **Rotation/disposal:** keys are single-session by default. Rotation
  within a session is a new session. Session end destroys the secret;
  already-issued tickets referencing the NodeId simply stop resolving —
  no revocation protocol, because nothing was published beyond the
  ticket holders.
- **Security properties:** NodeId is self-certifying (QUIC/TLS peer is
  its key); mutual auth comes from the dial (fleet pins the expected
  peer NodeId from configuration/ticket, ID sees fleet's NodeId and
  applies its own authorization — authorization itself is pass-3/not
  here). Relay exposure is bandwidth-abuse only (probe-2 finding for
  `relay.security.cab`); end-to-end bytes stay encrypted through the
  relay. Fleet MUST fail closed on identity mismatch and MUST NOT fall
  back to an unauthenticated route (inherited from the mcpx transport
  decision's no-silent-downgrade rule).

### 4.2 Modes and selection rule

Three modes, in increasing fleet involvement:

1. **Raw proxy.** Fleet splices bytes between caller and peer over an
   ordinary (non-iroh) transport. Fleet parses nothing, holds no iroh
   key for the flow, and is ALPN-agnostic. Use when the caller already
   holds a complete iroh ticket/connection context and fleet is only a
   byte mover (e.g. local unix-socket forward, `ssh -W`-style hop).
2. **Relay-aware proxy.** Fleet performs the iroh dial (needs a node
   key per §4.1 and a relay path, including the CONNECT-shim dialer in
   constrained egress) but treats the post-dial streams as opaque
   bytes. It selects ALPN and completes the handshake, then splices.
   Use when the far end speaks the protocol and fleet must traverse
   NAT/relay/egress on the caller's behalf, but the payload protocol is
   unknown to fleet or must not be inspected (forward-compat, sealed
   or experimental ALPNs).
3. **Initiator.** Fleet terminates the iroh connection: owns the dial,
   the ALPN negotiation, the per-stream request/response framing, and
   (where §4.4 triggers) the conversion and WS/WTransport serving. Use
   when the caller asked for ID semantics (tags, search, metadata,
   blobs via the meta protocol or its successors), for byte-streaming
   with fleet-side flow control, for postcard-native exchange, or for
   any browser-reachable path.

Selection rule (first match wins): caller passed `--raw` or an explicit
raw endpoint path → mode 1. Else the target ALPN/protocol is unknown to
fleet or caller passed `--opaque` → mode 2. Else → mode 3. A `--mode`
flag overrides all three for debugging; default is the rule, not a
fixed mode.

### 4.3 Byte streaming

In initiator mode, fleet exposes caller bytes in and bytes out on
stdout/stderr-style streams: stdin (or a `--in` file/socket) feeds
request streams, stdout/stderr carry response bytes and diagnostics.
Each iroh bidirectional QUIC stream carries one logical request/response
exchange (matching `id-sync`'s one-exchange-per-stream shape); fleet
multiplexes concurrent exchanges over the connection and enforces a
per-stream `64 KiB` default read cap inherited from the meta protocol's
`read_to_end` bound, configurable per call. Backpressure is QUIC
backpressure — fleet does not invent a second flow-control layer.

### 4.4 Postcard handling and conversion conditions

- **Postcard-native (fleet ↔ ID):** fleet speaks postcard exactly as ID
  does — `postcard::to_allocvec` / `take_from_bytes` with
  length-prefixed framing on the stream (4-byte big-endian length +
  postcard body; `take_from_bytes` rejects trailing/padded/concatenated
  payloads per the `decode_meta_exact` tests). Fleet pins the same
  `iroh`/`postcard` major versions ID builds against and honors the
  enum-position compatibility rule: adding or reordering variants is a
  wire break requiring an ALPN bump (the `/iroh-meta/3` precedent).
- **Why postcard strains at the edges:** (a) whole-value buffering —
  `from_bytes` needs the complete value, so multi-MB streams and file
  transfers cannot be processed incrementally in postcard form;
  (b) no schema evolution — positional variants fail silent-corrupt
  across skew instead of loud-reject; (c) no browser decoder — every
  web consumer would hand-roll one and disagree.
- **Conversion conditions (any one triggers):** the consumer is a
  browser or other non-Rust WS/WTransport client; OR the payload is a
  byte stream / blob larger than the postcard comfortable bound
  (default threshold 1 MiB per value — tunable per call); OR the
  exchange needs incremental/streaming delivery rather than
  request/response values. When triggered, fleet unwraps postcard from
  the iroh connection and re-encodes as CBOR (RFC 8949, preferred) or
  MessagePack (accepted where the consumer demands it) — field names
  preserved, enum variants encoded by NAME (string), never by position.
  Conversion is a fleet-edge function only; the iroh wire stays
  postcard and ID never sees CBOR/MessagePack from fleet's initiator.
- **Wire efficiency note:** for large transfers the converted form is
  chunked (default 256 KiB chunks, each its own WS message / WTransport
  stream frame with sequence numbers) so browsers stream instead of
  buffering; the chunking envelope is fleet's, documented in §4.5, and
  is not part of ID's protocol.

### 4.5 WebSocket / WebTransport serving shape (fleet side)

When §4.4 triggers, fleet serves the converted exchange locally:

- **Transports:** WebSocket (RFC 6455, binary frames) as the baseline
  every browser reaches; WebTransport (HTTP/3 datagrams + streams)
  where the fleet host has UDP/HTTP-3 egress. Same application framing
  on both; the client negotiates via `Sec-WebSocket-Protocol:
  fleet-id-cbor-v1` (or `-msgpack-v1`).
- **Envelope:** each message is one CBOR/MessagePack map:
  `{ v: 1, seq, kind: req|resp|chunk|error, alpn, payload, digest? }`.
  `alpn` names the ID protocol the payload came from (e.g.
  `/iroh-meta/3`); `digest` (BLAKE3) appears on chunked transfers for
  end-to-end integrity independent of TLS.
- **Auth:** the serving socket binds localhost by default; non-local
  bind requires the same session credential the initiator dialed with
  (bearer equal to the NodeId-derived session token — exact token
  construction is a pass-1 credential-source detail). No anonymous
  non-local serving.
- **Seam with pass 3 (stated once, referenced from both):** fleet's
  WS/WTransport socket is the CLIENT/INITIATOR side adapter — iroh in,
  WS out, owned by this pass. ID's REST mode (pass 3) owns the SERVING
  side for ID apps — REST endpoints, sessions with resume, bundles,
  terminal rendering. The envelope above is the contract between them:
  pass 3 MUST accept and produce the same `{v, seq, kind, alpn,
  payload, digest?}` framing when it bridges to fleet, so a browser
  cannot tell which side terminated iroh. Neither pass renames fields
  or forks the envelope without a joint ALPN-style version bump
  (`fleet-id-cbor-v2`).

### 4.6 Relationship to the ID binary and REST endpoints

Fleet MAY exec the ID binary to handle a connection, or hit an
ID-specific REST endpoint where one exists (pass-3 surface), instead of
using its in-process initiator — the choice is per call, default
in-process. When it execs or hits REST, the key lifecycle (§4.1) and
mode rule (§4.2) still apply: fleet supplies the key material and
decides the mode; the binary/endpoint is a mechanism, not a second
policy.

### 4.7 Data model and interfaces (summary contract)

- Session keypair: `{ node_id: [u8;32], secret: Zeroizing<[u8;32]> |
  CallerSupplied }`, memory-only, zeroized at session end.
- Stamp: `{ ..., fleet_node_id: NodeId }` on queue items and in the
  dot-sourced env (`FLEET_ID_NODE_ID`, plus `FLEET_ID_SECRET` in-session
  only). Never in logs.
- CLI shape (pass-1 owns exact syntax; semantics owned here):
  `fleet id connect [--id-secret|--raw|--opaque|--mode]`,
  `fleet id proxy` (modes 1–2), `fleet id stream` (mode-3 byte
  streaming), `fleet id postcard` (mode-3 value exchange),
  `fleet id serve-ws` (the §4.5 socket). Every command honors the
  pass-1 resolution order (flag > settings > standard local endpoint).
- Errors: identity mismatch, ALPN unknown in initiator mode, postcard
  trailing-bytes, conversion-threshold crossings, and non-local serve
  without credentials are all hard errors with distinct exit codes —
  never silent fallback.

## 5. Dependency-ordered build slices

Each slice is sized for one lane/session with its own acceptance
criteria. These are proposed slices; this design files none of them.

| Order | Slice and dependency | Acceptance criteria |
|---:|---|---|
| 1 | **Key lifecycle + stamping.** No iroh dial yet. Implement §4.1: per-session ephemeral generation, caller override, queue/env stamping alongside #10357, memory-only storage with zeroize, disposal semantics. | A test session shows a fresh NodeId per session; queue item carries `fleet_node_id`; dot-sourced env exposes `FLEET_ID_NODE_ID` (+ `FLEET_ID_SECRET` in-session only); secret appears in no log/trip record; session end zeroizes (no secret recoverable from process dump fixture). Caller-supplied key is used verbatim and never persisted. |
| 2 | **Raw + relay-aware proxy (modes 1–2).** Depends on slice 1 for keys. CONNECT-capable relay dialer + resolver override for the relay name (the probe-2 two-library-piece shape), opaque splice, `--raw/--opaque/--mode` selection. | Sandbox-egress-style test (CONNECT-only proxy fixture): mode 2 reaches a stock peer through the shim with TLS intact end to end; mode 1 byte-moves a caller-held ticket; stock peer needs no special code. Failure modes (relay unreachable, identity mismatch) error distinctly, never downgrade. |
| 3 | **Initiator: postcard-native exchange (mode 3, values).** Depends on slices 1–2. In-process iroh dial, ALPN `/iroh-meta/3`, length-prefixed postcard framing with `take_from_bytes` strictness, per-stream 64 KiB default cap, enum-position compat test. | Against a real test ID node: `List`/`Find`/tag round-trips succeed; padded/concatenated/trailing-byte payloads are rejected; variant-reorder fixture fails loudly (ALPN-bump path exercised); concurrent streams multiplex correctly. |
| 4 | **Initiator: byte streaming.** Depends on slice 3. stdin/stdout/stderr-style stream mapping, multiplex + QUIC backpressure, configurable caps. | 4 MiB and 10 MiB fixtures transfer sha-identical both directions through the relay fixture; slow-consumer test proves backpressure (bounded memory); stream teardown is clean on cancel. |
| 5 | **Conversion + WS/WTransport serving (edge).** Depends on slices 3–4. Threshold-triggered postcard→CBOR/MessagePack (variants by name), chunked envelope (§4.5), `fleet id serve-ws` localhost + credentialed non-local, `fleet-id-cbor-v1` negotiation. | Browser-fixture client (no postcard knowledge) completes a value exchange and a >1 MiB chunked transfer with per-chunk sequence + BLAKE3 digest verified; variant-by-name encoding proven against a skew fixture; non-local serve without credentials refused. |
| 6 | **Seam conformance with pass 3 + hardening.** Depends on slice 5 and the pass-3 REST surface. Joint envelope test, error-code catalog, CONNECT-shim + relay-matrix CI, docs. | The §4.5 envelope round-trips through both the fleet socket and the pass-3 REST bridge byte-identical; the error catalog has a test per code; CI covers direct + relay + CONNECT-proxy dial paths; this design's acceptance checklist (§9) is green. |

## 6. Rollout and rollback

- **Rollout:** slices land in order; each slice is independently
  revertible (feature-flagged initiator modes, defaulting to
  proxy-or-binary behavior until slice 3 is verified). Relay and key
  changes ride no production traffic until the slice-2 proxy fixture
  and slice-1 zeroize test pass in CI. Slice 5's serving socket binds
  localhost-only until credentialed non-local auth is proven. The
  first live remote-ID read runs verify-only (fetch + compare, no
  write) against a non-production ID store.
- **Rollback:** every slice rolls back by flag-flip to the previous
  mode (initiator → relay-aware proxy → raw proxy → exec-ID-binary),
  with no key material left behind (ephemeral keys die with the
  session; no migration to undo). A bad conversion encoding rolls back
  by serving postcard-native only (breaking browsers loudly, not
  silently corrupting). Chunking/envelope version bumps are additive
  (`-v2` alongside `-v1`); `-v1` is removed only after pass 3 confirms
  no consumer remains.
- **No-rollback-needed properties:** nothing here writes ID state by
  itself (reads and byte moves first); relay config and pkarr bootstrap
  are consumed, not operated, so there is no relay to roll back.

## 7. Verification plan

1. **Oracle — key lifecycle:** per-session uniqueness, stamp presence
   (queue + env), log-absence (grep CI over logs/trip records for
   secret material), zeroize fixture, caller-override verbatim use.
2. **Oracle — proxy:** CONNECT-only-proxy fixture proves mode 2 dials
   where stock dial dies (probe-2 replay); mode-1 byte-identity on 4/10
   MiB fixtures sha-compared; mismatch/downgrade tests prove fail-closed.
3. **Oracle — postcard:** strict-decode fixtures (padded, concatenated,
   truncated) rejected; enum-skew fixture fails loudly; live round-trip
   against a pinned ID version passes.
4. **Oracle — streaming:** 4/10 MiB sha-identical both directions;
   backpressure bounded-memory proof; cancel-cleanliness.
5. **Oracle — conversion/serving:** browser-fixture (Rust-free client)
   completes value + chunked transfers; digest verifies; variant-by-name
   proven; non-local auth refused without credentials; envelope
   byte-identical through the pass-3 bridge.
6. **Live proof:** sandbox-constrained caller → laptop stock ID node via
   own relay, initiated by fleet, sha-identical payload — the probe-2
   topology replayed through the shipped code path, at control-class
   throughput expectations (≈0.2 MB/s acknowledged, not a bulk promise).
7. **Regression:** existing pass-1 resolution tests still pass; no
   postcard/ALPN change without a version bump; CI matrix covers
   direct + relay + CONNECT-proxy dials.

## 8. Open questions and decisions

1. **Exact flag/env spellings (`--id-secret`, `FLEET_ID_SECRET`,
   `FLEET_ID_NODE_ID`, `--raw/--opaque/--mode`, `serve-ws` bind flags) —
   decider: design-mill owner under #10389 (pass-1 slice owner
   proposes).** Default: the names used in §§4.1–4.2/4.7 stand until
   pass 1 overrules them; pass 1 wins on syntax, this pass wins on
   semantics.
2. **Conversion threshold default (1 MiB) and chunk size (256 KiB) —
   decider: fleet transport owner (#13752) with ID performance input.**
   Default: ship 1 MiB / 256 KiB tunable per call; revisit with
   measurements from slice 4–5 fixtures.
3. **CBOR vs MessagePack priority at the browser edge — decider: pass-3
   owner (serving side) with fleet concurrence.** Default: CBOR
   preferred, MessagePack accepted; both encode variants by name.
4. **Non-local WS/WTransport credential construction (NodeId-derived
   session token shape) — decider: #10357 context-propagation owner
   (must compose with `FLEET_CTX_*`), Drew accountable.** Default: no
   non-local serving until this is specified and tested; localhost-only
   ships first.
5. **Which ID protocols beyond `/iroh-meta/3` the initiator must speak
   (blobs/gossip/irpc versions) and their ALPN registry — decider: ID
   maintainers, tracked against the iroh-ecosystem record.**
   Default: initiator supports `/iroh-meta/3` only; everything else
   goes relay-aware-proxy (mode 2) until an explicit per-ALPN slice is
   filed and the registry exists.
6. **Relay selection and self-hosted vs public relay policy —
   decider: fleet transport owner (#13752), not this pass.**
   Default: consume configured relay; this design names no new relay.

## 9. Record, tracking, and acceptance

This dated record is the design artifact for code#10391. Queue task
21681 tracks delivery of this design-only slice. Implementation slices
in §5 are proposed, not filed, and no implementation is authorized by
this record.

**Source-issue acceptance (from #10391), mapped:**

- Identity/key lifecycle (generation, stamping, env var, storage) —
  §4.1.
- Raw-proxy vs initiator modes and when each is chosen (incl.
  relay-aware variant) — §4.2.
- Postcard handling and MessagePack/CBOR conversion conditions — §4.4.
- WebSocket/WebTransport serving shape on the fleet side — §4.5, with
  the pass-3 seam stated once.

**Sufficient-design standard, mapped:**

- [x] Problem + context with evidence — §1.
- [x] Goals and explicit non-goals — §2.
- [x] Alternatives with honest tradeoffs, including do-nothing — §3.
- [x] Chosen approach (interfaces/contracts, data model, security and
  capability analysis) — §4.
- [x] Dependency-ordered build slices, lane-sized, each with acceptance
  criteria — §5.
- [x] Rollout + rollback plan — §6.
- [x] Verification plan (tests + live proof) — §7.
- [x] Open questions, each with a named decider — §8.
- [x] Recorded as a dated `doc/` record + tracking issue + queue item —
  this file, #10391, task 21681.
- [ ] After merge, link this document's permanent GitHub URL from
  #10391 and verify the link resolves.

### Evidence references

- #10391 (this pass), #10389 (mill parent), #10390 (pass 1),
  #10357 (context propagation).
- Iroh probe 2 report (2026-11-15 comment on #10391).
- `doc/2026-11-23T22-00-30Z_research_iroh-ecosystem/` (iroh 1.5.0,
  frozen wire, blobs+gossip+irpc toolkit).
- `doc/2026-11-23T21-10-36Z_design_fleet-transport-pooling/` (#13752
  fleet transport & pooling).
- `doc/fleet/mcpx-identity-transport.md` (tsnet decision; pure-Go
  constraint contrast; no-silent-downgrade rule).
- `pkgs/id/crates/id-sync/src/protocol.rs` (postcard meta protocol,
  ALPN `/iroh-meta/3`, strict-decode + enum-position rules).
- AUDIT-7 tracking: #13873 (lane 6 DESIGN-GAPS, sufficient-design
  standard source), #13871 (programme umbrella).
