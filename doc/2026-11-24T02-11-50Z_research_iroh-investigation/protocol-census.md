# Iroh Protocol Census

_Compiled 2026-11-23. Companion to the auth+transport programme
([developing-today/code#13779](https://github.com/developing-today/code/issues/13779))
and the fleet transport design
([#13752](https://github.com/developing-today/code/issues/13752)).
Every factual claim is linked to a primary source (n0-computer repos, iroh.computer,
crates.io, project repos) unless marked as third-party commentary._

---

## 1. iroh core

### 1.1 Where core stands: 1.0 shipped, now at 1.5.0

iroh reached stable **1.0.0 on 2026-06-15**, announced in the post
["Iroh 1.0 — Dial Keys, not IPs"](https://github.com/n0-computer/iroh.computer/blob/HEAD/src/app/blog/v1/page.mdx).
The 1.0 claims that matter:

- **Wire + API stability**: "an iroh v1 endpoint will be able to communicate with
  another iroh v1 endpoint, regardless of minor version or language"; any
  wire-breaking change now requires a major release
  ([1.0 post](https://github.com/n0-computer/iroh.computer/blob/HEAD/src/app/blog/v1/page.mdx)).
- **Official language bindings returned**: Python, Node.js, Swift, and Kotlin are
  officially supported alongside the Rust crate, via `iroh-ffi`
  ([1.0 post](https://github.com/n0-computer/iroh.computer/blob/HEAD/src/app/blog/v1/page.mdx)).
- **Scale claim**: n0's public relays saw "more than 200 million endpoints
  created, in the last 30 days alone" at 1.0 launch
  ([1.0 post](https://github.com/n0-computer/iroh.computer/blob/HEAD/src/app/blog/v1/page.mdx)).
- **Support schedule**: 0.35.x keeps public-relay support through 2026-12-31;
  the 0.9x canary line and 1.0.0-rc.x lost public-relay support on 2026-09-30
  ([1.0 post](https://github.com/n0-computer/iroh.computer/blob/HEAD/src/app/blog/v1/page.mdx)).

Post-1.0 release cadence has been fast — roughly monthly minors, all published
by n0's ramfox, per the
[crates.io record for `iroh`](https://crates.io/api/v1/crates/iroh)
(4.68M all-time downloads at census time):

| Version   | Date           | Headline changes                                                                                                                                                                                                                                                                                         |
| --------- | -------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1.0.0     | 2026-06-15     | Stable wire + API; FFI bindings; QUIC multipath; QUIC NAT traversal; custom transports                                                                                                                                                                                                                   |
| 1.1.0     | 2026-08-25     | Minor release ([crates.io](https://crates.io/api/v1/crates/iroh))                                                                                                                                                                                                                                        |
| 1.2.0     | 2026-09-09     | Minor release ([crates.io](https://crates.io/api/v1/crates/iroh))                                                                                                                                                                                                                                        |
| 1.3.0     | 2026-09-28     | Minor release ([crates.io](https://crates.io/api/v1/crates/iroh))                                                                                                                                                                                                                                        |
| 1.4.0     | 2026-10-20     | rustls RUSTSEC fixes; relay sends `X-Iroh-Endpoint-Id` header (NodeId header deprecated); faster relay startup; better local-only networking ([release notes](https://github.com/n0-computer/iroh/releases/tag/v1.4.0))                                                                                  |
| 1.4.1     | 2026-10-21     | Test/CI timeout fixes; rustls-webpki dep update ([changelog](https://raw.githubusercontent.com/n0-computer/iroh/main/CHANGELOG.md))                                                                                                                                                                      |
| 1.4.2     | 2026-11-19     | "Security Fixes for Relays and Endpoints" ([blog index](https://www.iroh.computer/blog))                                                                                                                                                                                                                 |
| **1.5.0** | **2026-11-19** | Exposes `noq::SendStream::set_affinity` (pin streams to paths), path wait helpers, relay handshake connection-leak fix; blog title: "Stream Affinity, Then Heads Down" ([changelog](https://raw.githubusercontent.com/n0-computer/iroh/main/CHANGELOG.md), [blog index](https://www.iroh.computer/blog)) |

### 1.2 The road to 1.0 (the 0.9x line, for context)

The pre-1.0 canary line shows what 1.0 actually consisted of, per the
[iroh 1.0 roadmap page](https://www.iroh.computer/roadmap) (last updated
2026-02-09):

- **v0.94.0** (2025-10-20): switch to `n0-error`; infallible
  `Connection::alpn`/`remote_id`; 0-RTT API work.
- **v0.95.1** (2025-11-05): **QUIC Multipath** support and **QUIC NAT traversal**
  landed; `EndpointHooks` for connection inspection/auth; `Discovery` renamed
  to **`AddressLookup`**.
- **v0.96.0** (2026-01-27): address prioritisation in `AddressLookup`;
  **custom transports**; "own all foreign types" API hygiene.
- **v0.97.0** (2026-02-23): published explicitly as a release candidate for
  community feedback.
- **1.0.0-rc.0 / rc.1** (May 2026): wire freeze process; a draft specification
  of the iroh protocol (open standards used + deviations) was a roadmap item,
  listing self-signed/raw-public-key TLS, QUIC, ICE-over-QUIC, STUN-over-QUIC,
  DNS discovery, Pkarr, mDNS, WebSockets, and the relay protocol as its parts
  ([roadmap](https://www.iroh.computer/roadmap)).

Structural changes across the whole arc: the bundled protocols (blobs, docs,
gossip) were pulled out of the `iroh` crate into their own repos (started in
0.28, ["Let them have crates"](https://www.iroh.computer/blog/iroh-0-28-let-them-have-crates)),
the CLI and the bundled `Node` were removed (0.29,
["net is the new iroh"](https://www.iroh.computer/blog/iroh-0-29-net-is-the-new-iroh)),
relay connections moved to WebSockets only (raw TCP relay connections
deprecated), the relay protocol was overhauled, TLS moved to raw public keys
(RFC 7250), and tickets moved into their own crate
([roadmap](https://www.iroh.computer/roadmap)).

### 1.3 Architecture as it now stands

- **Identity**: an Ed25519 keypair; the public key is the `EndpointId`
  (pre-1.0 name: `NodeId`). TLS 1.3 with raw public keys (RFC 7250) makes the
  key itself the transport identity — no PKI, no certificates
  ([source survey of the 1.0 tree](https://sparkles.petar-kirov.dev/research/iroh/)).
- **The socket layer** (formerly "magicsock"): one UDP socket multiplexing
  direct IP paths, relay paths, and custom transports; path selection and
  migration are handled below QUIC
  ([source survey](https://sparkles.petar-kirov.dev/research/iroh/)).
- **QUIC**: n0 now maintains its own QUIC implementation, **`noq`** — a fork of
  quinn extended with QUIC Multipath, QUIC Address Discovery (QAD), and an
  in-QUIC NAT-traversal protocol (QNT); it versions alongside iroh (noq 1.5.0
  shipped with iroh 1.5.0)
  ([source survey](https://sparkles.petar-kirov.dev/research/iroh/),
  [blog index](https://www.iroh.computer/blog)).
  The old DISCO UDP side-protocol + external STUN design was replaced by this
  QUIC-native machinery
  ([source survey](https://sparkles.petar-kirov.dev/research/iroh/)).
- **Relays**: the `iroh-relay` crate (in the iroh workspace) is "the relay
  server implementation. This is the code we run in production (and you can,
  too!)" ([iroh repo README](https://raw.githubusercontent.com/PaulOlteanu/iroh/main/README.md)).
  Relays are a fallback path and hole-punch coordination point, not a data
  destination; n0 operates the public relay fleet behind the default "n0"
  preset and rate-limits relayed traffic
  ([1.0 post](https://github.com/n0-computer/iroh.computer/blob/HEAD/src/app/blog/v1/page.mdx)).
  Relay allow/deny-list auth by endpoint ID was a roadmap item
  ([roadmap](https://www.iroh.computer/roadmap), issue #3086).
- **Address lookup** (renamed from "Discovery" pre-1.0): pluggable services,
  each with a publisher and/or resolver half; endpoints publish their own
  addressing info automatically and republish periodically. Shipped services:
  - **Pkarr publishing + DNS lookup** (the default global path — see §3):
    endpoints publish a signed pkarr packet (endpoint ID + home relay URL) to
    n0's `iroh-dns-server`; others resolve `_iroh.<z32-endpoint-id>.dns.iroh.link`
    TXT records through ordinary DNS
    ([address lookup docs](https://github.com/n0-computer/docs.iroh.computer/blob/HEAD/concepts/address-lookup.mdx)).
  - **Direct Mainline DHT** lookup (`DhtAddressLookup`) for the fully
    decentralised variant
    ([iroh source docs](https://www.cyberia.blog/soft3/radio/iroh/src/address_lookup/pkarr.rs/)).
  - **mDNS** for local networks, including full local-first operation with no
    internet ([1.0 post](https://github.com/n0-computer/iroh.computer/blob/HEAD/src/app/blog/v1/page.mdx)).
  - **Static/memory lookup** for tests and pre-configured peers.
  - Privacy work is active here: n0 published
    ["Blinded address lookup"](https://www.iroh.computer/blog) (2026-10-13) on
    hiding lookup metadata.
- **Protocols framing**: iroh's own model is that applications register
  **protocols** — a `ProtocolHandler` bound to an **ALPN** string, dispatched
  by the `Router` over a single endpoint. n0's framing: "Iroh Protocols are
  pluggable extensions that build on direct connections… Mix & match"
  ([protocol registry](https://www.iroh.computer/proto)). Any crate can define
  one; §2 and §4 are the census of what exists.
- **Custom transports**: 1.0 added support for plugging in non-UDP carriers
  (BLE, WiFi Aware, Tor; LoRa "under construction") under the same
  dial-by-key abstraction
  ([1.0 post](https://github.com/n0-computer/iroh.computer/blob/HEAD/src/app/blog/v1/page.mdx));
  the feature is still flagged `unstable-custom-transports` in the 1.5.0 crate
  ([crates.io](https://crates.io/api/v1/crates/iroh)).

### 1.4 What's announced as upcoming

The public roadmap page has not been updated since 2026-02-09 (it is a 1.0
roadmap), so post-1.0 direction is read from the blog and release train
([blog index](https://www.iroh.computer/blog)):

- **iroh-gossip → 1.0**: "A new iroh-gossip release and the road to 1.0"
  (2026-11-03) — gossip is the next family member being stabilised.
- **irpc 1.0 shipped** (2026-11-10, see §4.1) — the RPC layer is now stable.
- **"Heads down" after 1.5.0**: the 1.5.0 post's title ("Stream Affinity, Then
  Heads Down", 2026-11-19) signals a focus period rather than new headline
  features.
- **Blinded address lookup** (Oct 2026) points at lookup privacy as an active
  workstream.
- A **draft iroh protocol specification** was a 1.0-roadmap item
  ([roadmap](https://www.iroh.computer/roadmap)); the wire-freeze half of it is
  delivered by the 1.0 stability guarantee itself
  ([1.0 post](https://github.com/n0-computer/iroh.computer/blob/HEAD/src/app/blog/v1/page.mdx)).

---

## 2. The first-party protocol family (the "iroh prefix" projects)

All of these live in the [n0-computer GitHub org](https://github.com/n0-computer)
and are listed on the [protocol registry page](https://www.iroh.computer/proto)
where noted. Since 0.28–0.29 they are independent crates/repos that an
application adds explicitly — iroh core no longer bundles or privileges them
(["Let them have crates"](https://www.iroh.computer/blog/iroh-0-28-let-them-have-crates),
["net is the new iroh"](https://www.iroh.computer/blog/iroh-0-29-net-is-the-new-iroh)).

| Project                               | What it does                                                                                                                                                                                                          | ALPN                                | Maturity / latest                                                                                                                                                                                                                                                                                            |
| ------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **iroh-blobs**                        | Content-addressed blob transfer: BLAKE3-verified streaming (via `bao-tree`), resumable downloads, multi-provider download, persistent + in-memory stores, hash-and-format tickets                                     | `/iroh-bytes/4`                     | 0.103.0 shipped aligned with iroh 1.0 (2026-06-15); the most production-hardened of the family ([source survey](https://sparkles.petar-kirov.dev/research/iroh/), [migration record](https://github.com/axpdev-lab/aeroftp/blob/HEAD/docs/security-evidence/IROH-PEER-L0-ADVISORIES-2026-06.md))             |
| **iroh-gossip**                       | Topic-based broadcast overlay: HyParView partial-view membership + PlumTree eager/lazy-push broadcast tree                                                                                                            | `/iroh-gossip/1`                    | 0.101.0 (2026-06-15), 0.102.0 (2026-11-03), **0.103.0 (2026-11-16)**; explicitly on "the road to 1.0" ([docs.rs version list](https://docs.rs/crate/iroh-gossip/0.21.0), [blog index](https://www.iroh.computer/blog))                                                                                       |
| **iroh-docs**                         | Multi-writer, eventually consistent key-value sync ("Documents" on the registry page): entries keyed by (namespace, author, key), values stored as blobs, replicas reconcile via range-fingerprint set reconciliation | `/iroh-sync/1`                      | 0.101.0 (2026-06-15), incl. a redb 2.x store migration fix; the least-frequently released of the three core protocols ([changelog](https://github.com/n0-computer/iroh-docs/blob/HEAD/CHANGELOG.md), [source survey](https://sparkles.petar-kirov.dev/research/iroh/))                                       |
| **iroh-relay**                        | The relay server (and client) implementation; WebSocket-based relay protocol after the pre-1.0 overhaul                                                                                                               | — (relay protocol, not an app ALPN) | Ships inside the iroh workspace and versions with core (1.5.0); this is the code n0 runs for the public relays ([iroh repo README](https://raw.githubusercontent.com/PaulOlteanu/iroh/main/README.md))                                                                                                       |
| **iroh-willow**                       | Implementation of the [Willow protocol](https://willowprotocol.org) (data model + Meadowcap + Confidential Sync) using iroh for transport and iroh-blobs for payloads                                                 | (Willow sync ALPNs)                 | **Experimental / stale — see §2.1.** Crate 0.28.0 on docs.rs carries "This work is not released yet" ([docs.rs](https://docs.rs/iroh-willow/latest/iroh_willow/))                                                                                                                                            |
| **iroh-roq**                          | RTP over QUIC as an iroh protocol — packetises RTP media over QUIC streams/datagrams                                                                                                                                  | (roq ALPN)                          | Registry-listed protocol; the media building block under the `callme` demo ([protocol registry](https://www.iroh.computer/proto), [streaming docs](https://docs.iroh.computer/protocols/streaming))                                                                                                          |
| **iroh-live**                         | Live audio/video streaming over iroh using Media over QUIC: `iroh-moq` adapts iroh connections to MoQ sessions (via a WebTransport bridge), with publish/subscribe broadcasts and 1:1 "call" sugar                    | (MoQ ALPNs)                         | Active n0 repo (media livestreaming over iroh); the current home of n0's "iroh-calls" line of work — the earlier calls experiments were rebuilt as MoQ-based live streaming ([n0 org](https://github.com/n0-computer), [iroh-live deep dive](https://deepwiki.com/n0-computer/iroh-live/1.2-protocol-stack)) |
| **callme**                            | Demo app for iroh-roq: Opus voice calls, `callme-cli` + `callme-egui` (desktop/Android) clients                                                                                                                       | `/callme/rtc/0`                     | n0 demo, shows the roq stack end-to-end ([streaming docs](https://docs.iroh.computer/protocols/streaming), [callme architecture](https://deepwiki.com/n0-computer/callme/4.1-core-library-architecture))                                                                                                     |
| **iroh-content-discovery**            | "Iroh Content Discovery": a tracker protocol so blob providers/consumers can find each other for iroh-blobs content                                                                                                   | (own ALPN)                          | Registry-listed protocol ([protocol registry](https://www.iroh.computer/proto))                                                                                                                                                                                                                              |
| **iroh-ping**                         | Minimal ping protocol for iroh endpoints — the canonical "smallest real protocol" example                                                                                                                             | (own ALPN)                          | Registry-listed; example-grade by design ([protocol registry](https://www.iroh.computer/proto))                                                                                                                                                                                                              |
| **dumbpipe**                          | "Netcat over iroh": pipes stdin/stdout between endpoints by ticket; also `listen-tcp`/`connect-tcp` and `listen-unix`/`connect-unix` forwarding, `generate-ticket` for stable listeners                               | (own ALPN)                          | Standalone n0 tool, packaged widely (Homebrew, Nix, Termux packaging in progress); versions in lockstep with sendme (0.37.x line) ([Termux packaging issue](https://github.com/termux/termux-packages/issues/30605), [n0 org](https://github.com/n0-computer))                                               |
| **sendme**                            | File/directory transfer CLI on iroh-blobs: `sendme send <path>` prints a ticket, `sendme receive <ticket>` fetches with BLAKE3-verified, resumable streaming                                                          | (blobs)                             | **0.37.1** current; on Homebrew/Nix/Arch/Scoop/winget; the family's flagship end-user tool ([sendme page](https://www.iroh.computer/sendme), [docs.rs](https://docs.rs/crate/sendme/latest))                                                                                                                 |
| **iroh-doctor**                       | Standalone connectivity diagnostics CLI ("Iroh Doctor")                                                                                                                                                               | —                                   | Released as its own tool per the roadmap ([roadmap](https://www.iroh.computer/roadmap))                                                                                                                                                                                                                      |
| **iroh-dns-server**                   | The server behind `dns.iroh.link`: a pkarr relay + DNS resolver (see §3)                                                                                                                                              | —                                   | Ships in the iroh workspace; self-hostable ([address lookup docs](https://github.com/n0-computer/docs.iroh.computer/blob/HEAD/concepts/address-lookup.mdx))                                                                                                                                                  |
| **iroh-ffi**                          | Official bindings: Python, Node.js, Swift, Kotlin                                                                                                                                                                     | —                                   | Rebuilt on the stable 1.0 API and officially supported ([1.0 post](https://github.com/n0-computer/iroh.computer/blob/HEAD/src/app/blog/v1/page.mdx))                                                                                                                                                         |
| **noq**                               | n0's QUIC implementation (quinn fork): multipath, QAD, QNT                                                                                                                                                            | —                                   | Separate n0 repo; versions with iroh (noq 1.5.0) ([n0 org](https://github.com/n0-computer), [blog index](https://www.iroh.computer/blog))                                                                                                                                                                    |
| **net-tools / netwatch / portmapper** | Cross-platform networking utilities factored out of iroh (netcheck, port mapping, interface watching)                                                                                                                 | —                                   | Separate n0 repos/crates ([n0 org](https://github.com/n0-computer), [roadmap](https://www.iroh.computer/roadmap))                                                                                                                                                                                            |

### 2.1 iroh-willow: state, and why it is stale

The parent programme's WILLOW lane established the verdict (unreleased, built
on iroh 0.34-era APIs with patch-overrides to n0 forks); the census confirms
nothing has changed that picture:

- docs.rs still serves crate **0.28.0** with the notice "This work is not
  released yet" ([docs.rs](https://docs.rs/iroh-willow/latest/iroh_willow/)).
- Its dependencies are not crates.io releases but git patch-overrides to
  n0-computer forks of `willow-rs` (`willow-data-model`, `meadowcap`) and
  `willow-store`, plus a feature branch of iroh-blobs
  ([DeepWiki analysis](https://deepwiki.com/n0-computer/iroh-willow)) — the
  signature of a co-development snapshot, not a maintained release line.
- The version number (0.28) is keyed to the _old_ iroh numbering: there is no
  iroh-willow release aligned with iroh 0.9x, the 1.0 RCs, or 1.x, while
  blobs/gossip/docs all made that migration
  ([iroh-docs changelog](https://github.com/n0-computer/iroh-docs/blob/HEAD/CHANGELOG.md)).
- Independent corroboration: a third-party stack that vendored the iroh tree
  in Oct 2026 lists iroh-willow in its own inventory as "broken build" with no
  consumers ([cyberia.blog stack audit](https://www.cyberia.blog/soft3/radio/roadmap/soft3-radio/)).

**Why**: n0's protocol effort went into stabilising core (1.0), then gossip's
road to 1.0 and irpc 1.0 (§4.1); Willow's own sync spec (Confidential Sync) is
paused upstream, so the implementation has no stable target to converge on.
Treat iroh-willow as a research artefact: the design (Willow data model +
Meadowcap over iroh transport, payloads in iroh-blobs) is proven in code, but
adopting it means forking and porting it to iroh 1.x yourself.

---

## 3. pkarr

### 3.1 What it is

**Pkarr — Public-Key Addressable Resource Records.** An Ed25519 keypair signs a
small DNS packet (≤ 1000 bytes) and publishes it as a mutable item on the
**BitTorrent Mainline DHT** (BEP-44); anyone can resolve the packet by public
key — directly from the DHT or through an HTTP relay — and verify the
signature themselves. The public key thereby becomes a sovereign,
self-issued top-level domain
([pkarr README](https://raw.githubusercontent.com/dignifiedquire/pkarr/main/README.md),
[lib.rs crate page](https://lib.rs/crates/pkarr)).

Mechanics that matter:

- The DHT is ephemeral: records drop after a few hours, so publishers (or
  their friends/services) **republish periodically**; TTLs are clamped
  (min 5 min, max 24 h) and relays cache aggressively
  ([pkarr README](https://raw.githubusercontent.com/dignifiedquire/pkarr/main/README.md),
  [pkarr source](https://docs.rs/pkarr/5.0.0/src/pkarr/lib.rs.html)).
- **Relays** are trivial HTTP PUT/GET proxies in front of the DHT. The crate's
  defaults are `https://relay.pkarr.org` and `https://pkarr.pubky.org`
  ([pkarr source](https://docs.rs/pkarr/5.0.0/src/pkarr/lib.rs.html)) — i.e.
  operated by the pkarr/Pubky (Synonym) side of the ecosystem. Relays are
  untrusted by design: clients verify signatures end-to-end
  ([DeepWiki: network topology](https://deepwiki.com/pubky/pkarr/1.2-network-topology)).
- There are **two codebases to keep apart**: the upstream
  [pubky/pkarr](https://deepwiki.com/pubky/pkarr) (Synonym/Pubky; the crate is
  now at **5.0.0**, authors from Synonym, per its
  [Cargo.toml](https://docs.rs/crate/pkarr/5.0.0/source/Cargo.toml.orig)) and
  **n0's fork** (dignifiedquire/pkarr — Friedel Ziegelmayer of n0), which is
  the line iroh's address lookup was built against
  ([fork README](https://raw.githubusercontent.com/dignifiedquire/pkarr/main/README.md)).
- pkarr 5.x also carries an **"endpoints" spec** feature (pkarr.org/endpoints)
  for service-endpoint records
  ([pkarr 5.0 manifest](https://docs.rs/crate/pkarr/5.0.0/source/Cargo.toml.orig)).

### 3.2 Role one: a transport for small signed messages/data

A pkarr packet is just signed DNS records under a key — nothing in the format
restricts it to addresses. In practice the ecosystem uses it as a tiny,
globally resolvable, self-certifying data channel:

- **Pubky** publishes each user's homeserver pointer as a `_pubky` record in
  their packet; resolving a person _is_ resolving their pkarr packet
  ([pubky SDK docs](https://fastly-app.crates.io/crates/pubky)).
- **pkdns** (pubky/pkdns) publishes whole DNS zone files as pkarr packets,
  making public keys function as resolvable domains for ordinary DNS clients
  ([pkdns publishing docs](https://deepwiki.com/pubky/pkdns/6.2-publishing-records)).
- **iroh** puts exactly one thing in the packet today: the endpoint's home
  relay URL (and optionally direct addresses)
  ([address lookup docs](https://github.com/n0-computer/docs.iroh.computer/blob/HEAD/concepts/address-lookup.mdx)).

The constraints are the design: ≤1000 bytes, hours-scale ephemerality without
republishing, minute-scale TTLs. It is a rendezvous/config channel, not a
data plane — which is precisely the shape of "transporting messages" Drew
named: small signed payloads (pointers, invitations, session setup) that any
third party can carry without being trusted.

### 3.3 Role two: the initial discovery / relay bootstrap

This is the role iroh productionised. The problem: an `EndpointId` is all you
have; you need a _dialable_ address, and the endpoint's IP changes constantly.

- The endpoint signs a pkarr packet containing its **relay URL** and publishes
  it via HTTP PUT to a pkarr relay — by default **n0's own**
  `https://dns.iroh.link/pkarr`, served by `iroh-dns-server`, which doubles as
  a DNS resolver and deliberately does **not** touch the Mainline DHT (a
  central-but-untrusted convenience)
  ([iroh source docs](https://www.cyberia.blog/soft3/radio/iroh/src/address_lookup/pkarr.rs/)).
- A dialer resolves the ID either from that relay over HTTP (`PkarrResolver`)
  or through plain DNS — `_iroh.<z32-endpoint-id>.dns.iroh.link` TXT — so even
  non-pkarr software can bootstrap an iroh connection
  ([address lookup docs](https://github.com/n0-computer/docs.iroh.computer/blob/HEAD/concepts/address-lookup.mdx)).
- Defaults in code: packet TTL 30 s, republish every 5 minutes, relay-only
  address filter by default
  ([iroh source docs](https://www.cyberia.blog/soft3/radio/iroh/src/address_lookup/pkarr.rs/)).
- For deployments that want the fully decentralised path, `DhtAddressLookup`
  publishes/resolves through pkarr relays **and** the Mainline DHT directly
  ([iroh source docs](https://www.cyberia.blog/soft3/radio/iroh/src/address_lookup/pkarr.rs/)).

So pkarr is the bridge between "a key" and "a first connection": the relay
named in the packet then provides the always-on path and hole-punch
coordination until a direct path exists. Every other discovery mechanism
(mDNS, static) is local or manual; pkarr/DNS is the only global one iroh
ships by default.

### 3.4 Pubky — the flagship consumer

Pkarr's home ecosystem is **Pubky** (by Synonym): "a key-oriented,
user-controlled web" built from **Pubky Core** (pkarr identity/routing +
homeservers that store and serve a user's data) and the **Pubky App**
(pubky.app — social feeds, tagging, web-of-trust curation), plus **Pubky
Ring**, the mobile key/session manager
([Pubky overview](https://www.hnlbtc.group/bit-devs/resources/notes/pubky-infrastructure-for-decentralized-identity-management-data-routing-and-hosting/),
[launch post](https://medium.com/pubky/opening-pubky-4bdca7dc4c77)).
Pubky launched publicly in beta at BTC Prague (Aug 2025), positioned as a
"strict upgrade" to Nostr
([coverage](https://www.ainvest.com/news/pubky-launches-censorship-resistant-social-media-upgrade-2508/)).
Its SDK (`pubky` crate, 0.7.0) exposes signers, sessions, homeserver storage,
and pkdns publish/resolve as one facade, and its release train is what pulled
pkarr from 3.x to 5.x
([pubky-core releases](https://fastly-app.crates.io/crates/pubky)).
Pubky matters to this census because it is the proof that pkarr works as
consumer infrastructure at product scale — and the reason the upstream crate
is actively maintained independently of n0's fork.

---

## 4. Third-party protocols & libraries on iroh

### 4.1 RPC frameworks

- **irpc** (n0-computer/irpc): not strictly third-party — n0's own
  request-response RPC framework built _for_ iroh protocols (blobs/gossip/docs
  internals run on it). **irpc 1.0 shipped 2026-11-10** — "irpc 1.0 - Stable
  RPC for the iroh ecosystem" — making it a stable public building block for
  anyone writing an iroh protocol ([blog index](https://www.iroh.computer/blog)).
- Historically there were several community `iroh-rpc` crates; irpc is the
  survivor and the one with a stability commitment. Anyone designing a new
  iroh protocol today should start from irpc 1.0 rather than a community
  crate.

### 4.2 Real consumers with their own protocol layers

- **Delta Chat** — the highest-profile production deployment. Delta Chat uses
  iroh + iroh-gossip to power **webxdc realtime channels** (live collaborative
  mini-apps inside chats) since Delta Chat 1.48 (2024-11-20), after ~2 years
  of collaboration with n0; iroh is also behind Delta Chat's multi-device
  setup path. Bootstrap is deliberately _not_ pkarr/DHT: an Iroh ticket rides
  inside an end-to-end encrypted email message, so the existing chat graph is
  the discovery network. Implementation: `deltachat::peer_channels`
  ([Delta Chat announcement](https://delta.chat/en/2024-11-20-webxdc-realtime),
  [peer_channels API](https://rs.delta.chat/deltachat/peer_channels/index.html)).
- **Subduction / Keyhive (Ink & Switch)** — the transport story the parent
  programme's KEYHIVE lane tracked. `subduction_iroh` implements
  Subduction's `Transport` trait over iroh, with its own signed handshake
  (sender binding, timestamp, challenge, endpoint-id continuity) _above_
  iroh's TLS layer, because Subduction's threat model doesn't delegate
  identity to the transport
  ([PR #63](https://github.com/inkandswitch/subduction/pull/63),
  [docs.rs](https://docs.rs/subduction_iroh/latest/subduction_iroh/)).
  Subduction was **upgraded to iroh v1.0** in mid-2026, and **Teamtype**
  (formerly Ethersync) made the same jump — both recorded in Automerge's
  July 2026 ecosystem post
  ([Automerge blog](https://automerge.org/blog/2026-july/)). Keyhive
  integrates through the `subduction_keyhive` crate family, so the Keyline
  capability design now has a shipping iroh transport.
- **go-iroh** (tmc): an independent **Go implementation** of iroh — notable
  because it is a from-scratch port rather than an FFI binding, and evidence
  the 1.0 wire spec is stable enough to reimplement
  ([go-iroh site](https://go-iroh.tmc.dev), [awesome-iroh](https://github.com/n0-computer/awesome-iroh)).

### 4.3 Community protocols & tools (census)

From the community index [awesome-iroh](https://github.com/n0-computer/awesome-iroh)
(~414 stars; PR activity through Nov 2026, e.g.
[PR #99](https://github.com/n0-computer/awesome-iroh/pull/99)) and the n0 org:

| Project                              | What                                               | Notes                                                                               |
| ------------------------------------ | -------------------------------------------------- | ----------------------------------------------------------------------------------- |
| **iroh-ssh** (rustonbsd)             | SSH over iroh                                      | Community tool; the "just give me a shell" niche                                    |
| **pai-sho** (cablehead)              | Multi-port forwarding over iroh with reconnects    | The tooling niche dumbpipe doesn't cover (persistent forwards)                      |
| **ha-iroh**                          | Home Assistant integration                         | Iroh as remote-access for HA                                                        |
| **Godot iroh**                       | Iroh bindings/protocol for the Godot engine        | Listed on n0's protocol registry page ([registry](https://www.iroh.computer/proto)) |
| **Bones**                            | Game engine with iroh networking                   | Games vertical                                                                      |
| **Pigg**                             | Raspberry Pi GPIO over the network, iroh transport | Embedded/IoT vertical                                                               |
| **iroh-esp32-example**               | iroh on ESP32 (n0)                                 | Embedded proof-point for custom transports                                          |
| **Oku**                              | Distributed web browser on iroh                    | Browser vertical                                                                    |
| **Codux / Datum / Edky**             | Dev tools / data platforms on iroh                 | Listed in awesome-iroh                                                              |
| **Prime Intellect, Psyche, Tandemn** | Distributed AI training/inference on iroh          | The AI/ML cluster in awesome-iroh; largest-scale non-n0 usage signal                |
| **iroh-automerge experiments**       | Automerge documents over iroh transports           | Predecessor line to the Subduction/Keyhive work in §4.2                             |

Maturity read: the community layer is broad but shallow — many single-purpose
tools and integrations, few with release trains. The centre of gravity is
still n0's own family (§2) plus two production ecosystems: Delta Chat
(gossip) and the Automerge/Keyhive stack (custom transport). The registry
page's short list (Blobs, Gossip, Documents, roq, Godot, Content Discovery,
Ping) is n0's curated view of what counts as a "protocol"
([protocol registry](https://www.iroh.computer/proto)).

---

## 5. Census takeaways for the estate

1. **Core is done being designed.** iroh 1.5.0 (2026-11-19) sits on a frozen
   wire protocol with a stability guarantee, official bindings in four
   languages, and a maintained QUIC fork (noq). Building on iroh 1.x is no
   longer a bet on a moving target
   ([1.0 post](https://github.com/n0-computer/iroh.computer/blob/HEAD/src/app/blog/v1/page.mdx),
   [crates.io](https://crates.io/api/v1/crates/iroh)).
2. **The stabilisation queue is visible**: gossip → 1.0 next, irpc 1.0 done,
   blobs the de-facto stable data plane, docs the laggard, willow stalled
   (§2.1). For new protocol work: blobs + gossip + irpc are the supported
   toolkit.
3. **pkarr is load-bearing infrastructure twice over**: iroh's global
   bootstrap (via n0's central-but-untrusted relay/DNS bridge, with a real
   Mainline DHT option) and Pubky's identity layer (via Synonym's public
   relays). The two relay sets are different infrastructure run by different
   parties — do not conflate `dns.iroh.link` with `relay.pkarr.org` (§3).
4. **Discovery is a design choice, not a default**: Delta Chat proves a
   major consumer can ignore pkarr/DHT entirely and bootstrap from its own
   social graph via tickets — relevant to any estate design that already has
   an authenticated channel to bootstrap from (§4.2).
