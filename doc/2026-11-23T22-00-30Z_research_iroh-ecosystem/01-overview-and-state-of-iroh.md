# Overview and state of iroh

*Part of the [iroh ecosystem research record](2026-11-23T22-00-30Z_research_iroh-ecosystem.md).*

## Where core stands: 1.0 shipped, now at 1.5.0

Iroh reached stable **1.0.0 on 2026-06-15**, announced in
["Iroh 1.0 — Dial Keys, not IPs"](https://www.iroh.computer/blog/v1).
The claims that matter:

- **Wire and API stability.** A v1 endpoint can communicate with another
  v1 endpoint regardless of minor version or language; any
  wire-breaking change now requires a major release.
- **Official language bindings returned.** Python, Node.js, Swift and
  Kotlin are officially supported alongside the Rust crate, via
  `iroh-ffi`.
- **Scale.** At launch, n0's public relays had seen more than 200
  million endpoints created in the prior 30 days.
- **Support schedule.** The 0.35.x line keeps public-relay support
  through 2026-12-31; the 0.9x canary line and the 1.0 release
  candidates lost public-relay support on 2026-09-30.

The post-1.0 cadence has been fast — roughly monthly minors, 4.68M
all-time downloads at census time
([crates.io record for `iroh`](https://crates.io/api/v1/crates/iroh)):

| Version | Date | Headline changes |
|---|---|---|
| 1.0.0 | 2026-06-15 | Stable wire + API; FFI bindings; QUIC multipath; QUIC NAT traversal; custom transports |
| 1.1.0 | 2026-08-25 | Minor release |
| 1.2.0 | 2026-09-09 | Minor release |
| 1.3.0 | 2026-09-28 | Minor release |
| 1.4.0 | 2026-10-20 | rustls security fixes; relay sends an endpoint-ID header (the old NodeId header is deprecated); faster relay startup; better local-only networking ([release notes](https://github.com/n0-computer/iroh/releases/tag/v1.4.0)) |
| 1.4.1 | 2026-10-21 | Test/CI timeout fixes; dependency updates |
| 1.4.2 | 2026-11-19 | Security fixes for relays and endpoints |
| **1.5.0** | **2026-11-19** | Stream affinity (pin streams to paths), path wait helpers, a relay handshake connection-leak fix; the release post is titled "Stream Affinity, Then Heads Down" ([blog](https://www.iroh.computer/blog)) |

## How 1.0 was built: the 0.9x canary line

The pre-1.0 line shows what 1.0 actually consisted of, per the
[iroh 1.0 roadmap](https://www.iroh.computer/roadmap) (last updated
2026-02-09):

- **v0.94.0** (2025-10-20): infallible connection metadata APIs; 0-RTT
  API work.
- **v0.95.1** (2025-11-05): **QUIC Multipath** and **QUIC NAT
  traversal** landed; endpoint hooks for connection inspection and
  auth; `Discovery` renamed to **`AddressLookup`**.
- **v0.96.0** (2026-01-27): address prioritisation in address lookup;
  **custom transports**; API hygiene pass.
- **v0.97.0** (2026-02-23): published explicitly as a release candidate
  for community feedback.
- **1.0.0-rc.0 / rc.1** (May 2026): the wire freeze. A draft
  specification of the iroh protocol was a roadmap item, listing
  self-signed/raw-public-key TLS, QUIC, ICE-over-QUIC, STUN-over-QUIC,
  DNS discovery, pkarr, mDNS, WebSockets and the relay protocol as its
  parts.

Structural changes across the whole arc: the bundled protocols (blobs,
docs, gossip) were pulled out of the `iroh` crate into their own repos
(["Let them have crates"](https://www.iroh.computer/blog/iroh-0-28-let-them-have-crates)),
the CLI and the bundled node abstraction were removed
(["net is the new iroh"](https://www.iroh.computer/blog/iroh-0-29-net-is-the-new-iroh)),
relay connections moved to WebSockets only, the relay protocol was
overhauled, TLS moved to raw public keys (RFC 7250), and tickets moved
into their own crate.

## The architecture as it now stands

- **Identity.** An Ed25519 keypair; the public key is the `EndpointId`
  (pre-1.0 name: `NodeId`). TLS 1.3 with raw public keys (RFC 7250)
  makes the key itself the transport identity — no PKI, no
  certificates.
- **The socket layer** (formerly "magicsock"): one UDP socket
  multiplexing direct IP paths, relay paths and custom transports;
  path selection and migration are handled below QUIC.
- **QUIC.** n0 maintains its own QUIC implementation, **noq** — a fork
  of quinn extended with QUIC Multipath, QUIC Address Discovery and an
  in-QUIC NAT-traversal protocol. It versions alongside iroh (noq
  1.5.0 shipped with iroh 1.5.0), and it replaced the old side-protocol
  + external STUN design with QUIC-native machinery.
- **Relays.** The `iroh-relay` crate is the relay server implementation
  — the code n0 runs in production, and anyone can run it. Relays are
  a fallback path and a hole-punch coordination point, not a data
  destination; n0 operates the public relay fleet behind the default
  preset and rate-limits relayed traffic.
- **Address lookup** (renamed from "Discovery" before 1.0): pluggable
  services, each with a publisher and/or resolver half. Endpoints
  publish their own addressing info automatically and republish
  periodically. The shipped services are covered on the
  [pkarr and discovery page](03-pkarr-and-discovery.md): pkarr/DNS (the
  default global path), direct Mainline DHT lookup, mDNS for local
  networks, and static lookup for tests and pre-configured peers.
  Privacy work is active here — n0 published
  ["Blinded address lookup"](https://www.iroh.computer/blog) in
  October 2026, on hiding lookup metadata.
- **Protocols.** Applications register **protocols** — a handler bound
  to an ALPN string, dispatched by the router over a single endpoint.
  Any crate can define one; the
  [protocol registry](https://www.iroh.computer/proto) is n0's curated
  list, and the [protocol family page](02-protocol-family.md) is the
  full census.
- **Custom transports.** 1.0 added support for plugging in non-UDP
  carriers (BLE, WiFi Aware, Tor; LoRa under construction) under the
  same dial-by-key abstraction. The feature is still flagged
  `unstable-custom-transports` in the 1.5.0 crate.

## What is announced as upcoming

The public roadmap page has not been updated since 2026-02-09 — it is a
1.0 roadmap — so post-1.0 direction is read from the blog and the
release train:

- **iroh-gossip → 1.0.** A November 2026 post ("A new iroh-gossip
  release and the road to 1.0") names gossip as the next family member
  being stabilised.
- **irpc 1.0 shipped** (2026-11-10) — the RPC layer the data protocols
  are built on is now stable; see the
  [protocol family page](02-protocol-family.md).
- **"Heads down" after 1.5.0.** The 1.5.0 post's title signals a focus
  period rather than new headline features.
- **Blinded address lookup** (October 2026) points at lookup privacy
  as an active workstream.
- A **draft iroh protocol specification** was a 1.0-roadmap item; the
  wire-freeze half of it is delivered by the 1.0 stability guarantee
  itself.
