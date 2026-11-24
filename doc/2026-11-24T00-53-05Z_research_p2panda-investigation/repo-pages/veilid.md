# Veilid — an anonymity-first p2p framework, judged as a complement

Full lane notes: Part 1 of research lane VEILID-IPFS for
[#13779](https://github.com/developing-today/code/issues/13779), from
Veilid's book, developer manual, API docs, crates.io, and third-party
integrator material, read 2026-11-23. The IPFS/libp2p half of the same
lane has its [own page](ipfs-libp2p.md).

**Judgment up front: complement candidate, narrowly — not an alternate
base.** Veilid is a real, maintained, open-source p2p application
framework whose centre of gravity is _anonymity and routed privacy_, not
capability auth and not data sync. For this estate it competes with iroh
only at the transport layer, where it is weaker for our purposes, and it
offers exactly one thing iroh does not: receiver-private addressing and
an operational public DHT with watches. That earns it a watched pilot
slot for one specific job (below), not a platform role.

## What it is

Veilid is a peer-to-peer network and application framework from the Cult
of the Dead Cow / Veilid Foundation orbit: open source (MPL-2.0),
written in Rust (`veilid-core`), with bindings for Flutter/Dart, Python,
and WASM/TypeScript. It provides node identity, routed private
communication, a distributed hash table with mutable signed records,
app messaging/calls, and NAT traversal — packaged as a framework you
embed, closer in spirit to "libp2p as a product" than to a protocol
suite. Current release at research time: **veilid-core 0.5.8**
(2026-11-02), on a roughly monthly cadence — alive, pre-1.0, with
breaking changes still arriving in minor releases (a publicly
anticipated 1.0 had not landed).

Governance/funding: the **Veilid Foundation** is a nonprofit; there is no
VC war chest and no token. The team is small — the bus factor is a real
line item in any adoption decision — and the public network's size is
unpublished and, by all integrator accounts, small (hundreds-to-low-
thousands of nodes, not millions).

## The pieces, precisely

**Identity.** A node's identity is an Ed25519 keypair (the VLD0 crypto
suite: Ed25519 signatures, X25519 key agreement, XChaCha20-Poly1305
AEAD, BLAKE3 hashing, Argon2 for key derivation). Same primitive family
as the rest of this estate — a Veilid node ID is, mechanically, another
Ed25519 public key — but Veilid identities live in Veilid's own
namespacing and routing world; there is no bridging story to iroh
NodeIds beyond "both are Ed25519."

**DHT records — the strongest component.** Veilid's DHT is a Kademlia
store of _mutable, signed_ records:

- Records are owned by a keypair; each record has up to 32 subkeys, each
  holding a value of at most **1 MiB** (the practical per-value guidance
  is much smaller), versioned by **per-subkey sequence numbers** with
  last-writer-wins resolution.
- Two schemas ship: **DFLT** (owner-only writes) and **SMPL** (named
  additional writers, each granted exclusive subkey ranges) — i.e. a
  small, fixed menu of multi-writer policies, not a general permission
  system.
- A write at the maximum sequence number **permanently closes** a subkey
  — a deliberate one-way door worth knowing before designing on it.
- **Watches**: nodes can watch subkeys and be notified of changes, with
  expiration and count limits — the feature that makes the DHT usable as
  a rendezvous/signalling layer rather than a poll-loop.
- Offline behaviour is honest: writes made while disconnected queue and
  propagate on reconnect; reads may be stale by design (sequence numbers
  tell you _how_ stale relative to a known value).

**Routing and privacy — the actual point of the project.** Veilid routes
traffic through **safety routes** (sender-chosen relay hops) and
**private routes** (receiver-published multi-hop routes): a receiver is
addressed via an opaque **route blob**, and can receive messages
_without ever learning — or revealing — a NodeId or IP address_ to the
sender. Combined with per-message encryption this is a genuinely
different privacy posture from iroh's (where peers are addressed by, and
learn, each other's NodeIds). NAT traversal (its VICE system) handles
the usual home-router cases over UDP/TCP/WebSocket transports.

**App messaging.** `AppMessage` (fire-and-forget) and `AppCall`
(request/response) between routes/nodes, capped at **32,768 bytes** per
message. There is **no bulk transfer, no streaming file protocol, no
QUIC** — anything data-sized is the application's problem, chunked over
32 KB messages. This single limit disqualifies Veilid as a data-plane
transport for an estate whose sync payloads are operations and blobs.

## The word "capabilities" — a trap for survey readers

Veilid's own documentation uses "capabilities" for something entirely
different from this investigation's subject: **FourCC node-feature
advertisements** — bitflags a node publishes about which protocol
features it supports (`ROUT`, `SGNL`, `RLAY`, `DHTV`, `DHTW`, `APPM`,
with `TUNL` and `BLOC` defined but unimplemented). They are capability
_negotiation_ in the protocol-engineering sense, **not authorisation**:
there is no delegation, no attenuation, no expiry, no revocation —
nothing in Veilid answers "may this principal do this thing." Access
control in Veilid reduces to: DHT record ownership/schemas (write
permission by key), route-blob possession (reachability as a weak
capability — whoever holds the blob can send to the route), and whatever
the application builds on top. Any future claim that "Veilid provides
capabilities" should be read as the FourCC sense unless shown otherwise.

## Limits that matter for this estate

- **No Cloudflare/Workers story**: Veilid is a long-lived UDP/TCP node
  framework; there is no edge deployment model. (A WASM client exists,
  but a browser tab is a client, not an estate node.)
- **Pre-1.0, small team, no public audit**: a third-party integrator's
  assessment describes the cryptography as unaudited; the estate would
  be trusting VLD0 on the strength of standard primitives and one small
  team's implementation.
- **Anonymity costs performance and debuggability**: multi-hop routed
  delivery is latency-expensive and operationally opaque compared with
  iroh's direct-first-with-relay-fallback model; for an estate whose
  traffic is overwhelmingly _its own named seats talking to each other_,
  anonymity-from-each-other is mostly not the requirement — the seats
  are supposed to know who they are talking to.
- **Ecosystem gravity**: the app catalogue on the public network is
  small (chat demos, file-sharing experiments); there is no sync engine,
  no CRDT store, no group system — Veilid would be a _third_ framework
  to operate alongside iroh + p2panda, bought for one feature.

## The one proven composition — and the one estate-shaped use

There is a real production-shaped precedent for composing Veilid _with_
iroh rather than choosing between them: OpenArchive's **Save** app
(secure media archiving for human-rights defenders, in beta production)
uses **Veilid for encrypted p2p networking / anonymous connections and
iroh for data storage, replication, and verification**, hand-glued by
Hypha Co-operative. That is exactly the shape this lane would predict:
Veilid where _who can be found_ is the sensitive part, iroh where _moving
and verifying data_ is the job.

For this estate, the only use shaped like our problems is the
**rendezvous/discovery record layer**: hosts that are not on Tailscale
need a public place to publish "where to reach me / my current route"
that seats can watch — a signed, mutable, watched DHT record is a
reasonable fit for that narrow job (the
[#13752](https://github.com/developing-today/code/issues/13752) hosts
thread), and receiver-private addressing via route blobs is the one
capability iroh structurally does not offer, should a future feature
need "contactable but not identifiable."

**Disposition: watched pilot candidate for the rendezvous-record job
only.** Not a transport base (32 KB messages, no bulk, no QUIC), not an
auth layer (no authorisation model at all), not a sync layer (LWW
subkeys are a register, not a log). If the pilot never happens, nothing
else in the recommended stack misses Veilid.
