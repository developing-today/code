### 2026-06-15 — Iroh 1.0 - Dial Keys, not IPs

- Link: https://www.iroh.computer/blog/v1
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Announces iroh 1.0, the first stable release after 65 versions and over four years, built around dialing cryptographic keys instead of IPs. Highlights include QUIC multipath, QUIC NAT traversal, local-first operation, WASM support, hooks, and custom transports such as BLE and Tor, plus restored official Python, Node.js, Swift, and Kotlin bindings. It asserts wire and API stability for v1, notes about 200 million endpoints created on public relays in the prior 30 days and typically 95% direct traffic, and sets relay support end dates for older lines.

### 2026-06-10 — Address validation tokens - what are they good for anyway?

- Link: https://www.iroh.computer/blog/address-validation-tokens
- Author: not shown
- Kind: design deep-dive
- Read: full
- Summary: Explains QUIC's 3x anti-amplification limit (3,600 bytes from a 1,200-byte Initial) and how it can stall large handshakes and, for iroh, 0-RTT responses over a few KiB. Address validation tokens (NEW_TOKEN) issued after a validated handshake lift the limit on reconnection; noq implements single-use checking with a constant-size two-stage bloom filter, disabled by default server-side via the bloom feature in noq-proto while client storage is on by default. A remote blob-service demo shows an 8,000-byte 0-RTT fetch dropping from about 418 ms with a mid-response stall to about 200 ms with tokens enabled, with per-path multipath tokens and multi-address tokens deferred until after 1.0.

### 2026-05-27 — iroh 1.0.0-rc.1 - The last one

- Link: https://www.iroh.computer/blog/iroh-1-0-0-rc-1
- Author: not shown
- Kind: release notes
- Read: full
- Summary: The final release candidate before 1.0 adds holepunching when the server, not just the client, is behind a hard NAT, a configurable PathSelector trait for custom transports, and a pluggable relay AccessControl trait replacing AccessConfig with on-connect/on-disconnect hooks and proactive disconnect. Endpoint close is made near-instant when the peer already closed, and remote-state races, path-abandonment scoping, and static musl builds are fixed. Breaking changes include renaming IncomingLocalAddr to LocalTransportAddr and the relay access-control redesign, with noq updated to 1.0.0-rc.1.

### 2026-05-19 — Iroh post-quantum key exchange

- Link: https://www.iroh.computer/blog/iroh-post-quantum-handshakes
- Author: not shown
- Kind: security
- Read: full
- Summary: Explains the harvest-now-decrypt-later risk to iroh's key exchange versus the impersonation risk if Ed25519 signatures are broken, for which there is no equivalent pre-harvest urgency. Iroh can opt into hybrid X25519MLKEM768 post-quantum key exchange by using the aws-lc-rs provider via the tls-aws-lc-rs feature and rustls prefer-post-quantum, at a cost of roughly 1 KB each way and a heavier dependency; ring does not support it. The post stresses this is prefer-not-force, does not protect against a future active attacker with a cryptographically relevant quantum computer, and that migrating EndpointIds away from Ed25519 awaits industry consensus on post-quantum signatures.

### 2026-05-11 — iroh 1.0.0-rc.0 - The first release candidate

- Link: https://www.iroh.computer/blog/iroh-1-0-0-rc-0
- Author: not shown
- Kind: release notes
- Read: full
- Summary: The first 1.0 release candidate, after four years and more than 50 releases, focuses on refining and stabilizing the API alongside noq. The path observation API is split into Connection::paths() snapshots and Connection::path_events() streams, IncomingLocalAddr replaces a plain IP for incoming connections, and many public types become non-exhaustive. DHT and mDNS address lookups move to separate crates in iroh-address-lookups and AccessLimit moves to iroh-util, MSRV rises to 1.91, and extensive breaking changes are listed across iroh, iroh-relay, and iroh-dns-server.

### 2026-04-17 — iroh 0.98.0 - Getting back to traversing NATs

- Link: https://www.iroh.computer/blog/iroh-0-98-0-getting-back-to-traversing-nats
- Author: not shown
- Kind: release notes
- Read: full
- Summary: A reliability release fixing NAT-traversal regressions introduced through 0.96–0.97 with multipath, QUIC NAT traversal, and the noq split, mostly via noq 0.18: abandoned-path handling, PTO capped at 2 seconds, NAT-traversal retries, and ack-eliciting tail-loss probes, backed by a larger patchbay test matrix. It also adds pluggable crypto backends (ring by default, aws-lc-rs, or a custom provider), a router incoming-connection filter for early rate limiting claimed at about 30x throughput for address rejection versus accept-and-close, and relay protocol v2 with version negotiation. Breaking changes include renaming ConcurrentAddressLookup to AddressLookupServices and removing Endpoint::empty_builder in favour of presets.

### 2026-04-16 — QUIC packet rejection in practice

- Link: https://www.iroh.computer/blog/quic-packet-rejection-in-practice
- Author: not shown
- Kind: performance
- Read: full
- Summary: Measures iroh's early QUIC rejection options against valid-looking ClientHello spam using a CPU-time-based echo-service harness, where a full handshake plus echo baseline is 1,084 ops/s per core. Silently ignoring is cheapest at about 36,576 ops/s direct and 31,980 via relay, reject is about 18,312 / 10,561 ops/s, and retry-then-reject — the first mode that validates the source address or endpoint ID for selective filtering — is about 10,629 / 5,640 ops/s, with ALPN rejection similarly cheap. Under 100 spam packets per valid request and at 10,000 spam packets per second, valid traffic still gets through with only modest throughput loss, and the post warns macOS UDP is a poor choice for high-rate endpoints compared with Linux.

### 2026-03-26 — iroh for payments (Paycode)

- Link: https://www.iroh.computer/blog/paycode
- Author: not shown
- Kind: case study
- Read: full
- Summary: Profiles Paycode in Mexico using iroh to connect payment terminals, point-of-sale systems, and highway toll-booth software for tap-to-pay without a local or cloud server as a single point of failure. The peer-to-peer channel acts as a blind command-and-control link so raw payment data stays inside the PCI-compliant terminal flow and only encrypted payloads leave the device. Despite legacy constraints — Windows 7 touch machines, dual-core CPUs with up to 8 GB RAM, mixed Ethernet and Wi-Fi — the team bundled the Rust library into a .NET 6 SDK and a Kotlin POS app, using QR-encoded iroh tickets and gossip discovery to start transactions.

### 2026-03-24 — Running iroh on an ESP32

- Link: https://www.iroh.computer/blog/iroh-on-esp32
- Author: not shown
- Kind: ecosystem
- Read: full
- Summary: Walks through getting a full iroh QUIC/TLS endpoint running on an ESP32 WROVER with 4 MiB flash and about 500 KiB internal RAM, using a special unmaintained branch of iroh and noq. Obstacles solved include a missing gethostname symbol, binary size (fixed with opt-level z, LTO, and codegen-units 1, reaching 88.53% of flash), registering eventfd VFS for tokio, enabling external PSRAM for allocations, and replacing ring/aws-lc-rs — neither works on Xtensa — with a minimal pure-Rust rustls-rustcrypto provider supporting only TLS13_AES_128_GCM_SHA256 and X25519, with relay certificate verification disabled. The result is a working echo endpoint reachable from stock crates.io iroh, with mainline support for 32-bit embedded targets promised gradually.

### 2026-03-19 — noq, noq, who's there?

- Link: https://www.iroh.computer/blog/noq-announcement
- Author: not shown
- Kind: ecosystem
- Read: full
- Summary: Announces noq ("number 0 QUIC"), n0's own general-purpose QUIC implementation, as a hard fork of Quinn after a period as a soft fork, arguing iroh's multipath, NAT traversal, and relay-as-path work diverged too far to impose on Quinn users. Noq has powered iroh since v0.96 and provides a full QUIC Multipath implementation with per-path congestion state, a production implementation of QUIC NAT Traversal, QUIC Address Discovery replacing STUN, greatly extended qlog including multipath events, and a WeakConnectionHandle type. Interop testing is done against picoquic, with collaboration with Quinn continuing where interests overlap.

### 2026-03-16 — iroh 0.97.0 - Custom Transports & noq

- Link: https://www.iroh.computer/blog/iroh-0-97-0-custom-transports-and-noq
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Introduces experimental custom transports behind the unstable-custom-transports feature — any unreliable datagram transport supporting at least 1,200-byte packets, with Tor and Nym implementations existing and BLE planned — and formally switches the QUIC implementation from the Quinn fork to the standalone noq/noq-proto crates. RelayService becomes public for embedding a relay in an axum app, TLS trust roots become configurable via CaRootsConfig, address lookup gains AddrFilter, and PathWatcher retains closed-path stats. Breaking changes include Endpoint::builder/bind now requiring a preset such as presets::N0 and Endpoint no longer closing gracefully on drop, so endpoint.close() must be awaited.

### 2026-03-11 — What if your security camera was secure?

- Link: https://www.iroh.computer/blog/secure-video-everywhere
- Author: not shown
- Kind: ecosystem
- Read: full
- Summary: Presents a Raspberry Pi prototype camera ("Tape Boy") combining iroh with Media over QUIC (MoQ) as an alternative to cloud/WebRTC-style cameras that encode multiple quality streams continuously. MoQ is poll-based, so the camera generates no video until a viewer connects, saving battery and bandwidth, while iroh provides the encrypted direct connection to reach the camera from anywhere, staying local when viewer and camera are on the same network. The post frames this as an IoT pattern that also extends to browser bridging via WebRTC, MoQ relays for conferencing and fan-out, and broadcast use with extra tracks such as timecode.

### 2026-02-27 — How QUIC rejects garbage packets

- Link: https://www.iroh.computer/blog/quic-packet-rejection
- Author: not shown
- Kind: security
- Read: full
- Summary: Explains, packet type by packet type, how QUIC and noq discard garbage cheaply before expensive work: invalid long-header packets by inspection, Initial packets via Retry tokens to validate a spoofable source address before user filtering by IP, region, or rate limit, and post-handshake filtering by verified endpoint ID and ALPN. For established connections, short-header packets are filtered by destination connection ID lookup, cheap packet-number decryption to drop duplicates and out-of-window numbers, and only then full AEAD decryption; relay connections can be filtered early by relay-reported endpoint ID. It closes with stateless-firewall recommendations, notably dropping sub-1,200-byte Initials and heavily rate-limiting long-header packets while lightly limiting short-header packets.

### 2026-01-27 — iroh 0.96.0 - The QUIC Multipaths to 1.0

- Link: https://www.iroh.computer/blog/iroh-0-96-0-the-quic-multipaths-to-1-0
- Author: not shown
- Kind: release notes
- Read: full
- Summary: A major wire-breaking release, described as one of the last before 1.0, adding QUIC multipath so a single connection retains multiple paths, and completing the switch from custom holepunching to QUIC NAT Traversal integrated into QUIC for encryption, loss recovery, and congestion control, with deviations from the draft including required multipath and REACH_OUT frames. The Connection API is unified as Connection<T> for handshake-completed and 0-RTT states, a new EndpointHooks system adds before_connect and after_handshake interception, TransportAddr replaces ConnectionType/conn_type, and Discovery is renamed to AddressLookup throughout. It carries a prominent known issue: holepunching is not re-triggered on most network changes, leaving connections on relays.

### 2026-01-27 — Use iroh with Tor for anonymous connections

- Link: https://www.iroh.computer/blog/tor-custom-transport
- Author: not shown
- Kind: ecosystem
- Read: full
- Summary: Shows building a Tor custom transport for iroh on top of a local Tor daemon, exploiting that onion services and iroh endpoints are both addressed by Ed25519 keys so an EndpointId mechanically translates to an onion address and a CustomAddr of type 0x544f52 ("TOR") with no separate lookup or tickets. The transport creates an onion service over the Tor control port reusing the endpoint's key, carries postcard-serialized Transmits over SOCKS5 TCP connections kept open for the life of the iroh connection, and is demonstrated with an echo example; for real privacy users must disable the built-in IP/relay transports and lookups. Both the transport and the custom-transport API are flagged experimental, with Tor cookie-auth support, daemon-liveness recovery, and an embedded Arti implementation listed as remaining work.

### 2025-11-05 — iroh 0.95.0 - A New Relay, Error Handling, and Connection API Improvements

- Link: https://www.iroh.computer/blog/iroh-0-95-0-new-relay
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Adds a new US West Coast public relay region, introduces the n0-error crate replacing snafu across iroh — StackError/AnyError types carrying call-site location for the whole error stack, shown when RUST_BACKTRACE or RUST_ERROR_LOCATION is set — and restructures 0-RTT with explicit OutgoingZeroRttConnection and IncomingZeroRttConnection types and a ZeroRttStatus result, while Connection::remote_id() and alpn() become infallible. ProtocolHandler::on_connecting is replaced by on_accepting taking an Accepting type, and iroh-dns-server upgrades to redb 3, requiring users to run 0.93 at least once to migrate its database. The post says only 0.96 remains in the 0.9x canary series before a 1.0 release candidate later that year.

### 2025-10-22 — iroh 0.94.0 - The Endpoint Takeover

- Link: https://www.iroh.computer/blog/iroh-0-94-0-the-endpoint-takeover
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Completes the rename from "node" to "endpoint" across the API — NodeAddr to EndpointAddr, NodeId to EndpointId, Endpoint::node_id to id and node_addr to addr — and moves tickets out of iroh-base into a new iroh-tickets crate. Endpoints can now insert and remove relays dynamically with Endpoint::insert_relay/remove_relay, discovery gets defaults (Pkarr and DNS) plus a Preset trait for reusable builder configuration, and EndpointAddr is rebuilt around a non-exhaustive TransportAddr enum (Relay/Ip) to future-proof for transports beyond UDP. Relay lifecycle notes say 0.90 support has ended, canary relays run 0.94 compatible with 0.93, and 0.35 relays will run until three months after 1.0.

### 2025-10-15 — Hashing multiple blobs with BLAKE3

- Link: https://www.iroh.computer/blog/hashing-multiple-blobs-with-BLAKE3
- Author: not shown
- Kind: performance
- Read: full
- Summary: Shows how to hash many small, same-size blobs fast by repurposing BLAKE3's internal Platform::hash_many SIMD routine — normally used for chunks of one blob — to compute independent root hashes, using the IV as key, counter 0 with IncrementCounter::No, and CHUNK_START / CHUNK_END|ROOT flags. On an M1 MacBook Pro hashing 1,048,576 blobs of 1,024 bytes, sequential hashing takes about 1.28 s, rayon alone about 152 ms (about 8.5x), SIMD alone about 547 ms (2.34x), and SIMD plus rayon about 75 ms, roughly 17x over baseline and about 5.4x faster than SHA2-256 in the combined mode. The post notes the constraints (batch size a multiple of MAX_SIMD_DEGREE, sizes a multiple of 64 bytes) and that a clean public API for this remains an open design question.

### 2025-10-13 — iroh-blobs 0.95 - New features

- Link: https://www.iroh.computer/blog/iroh-blobs-0-95-new-features
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Details iroh-blobs 0.95 features led by a new ConnectionPool in util::connection_pool that bounds concurrent connections with configurable idle/connect timeouts, max connections, and an on_connected hook that can, for example, wait for a direct path before handing a connection out, returning a lifetime-tracked ConnectionRef that must not be cloned into a bare Connection. It also adds abstract send/receive stream traits so the whole blobs protocol can be wrapped, e.g. with compression under a different ALPN as planned for sendme, and detailed provider events (from 0.93) implemented as an irpc protocol with per-request intercept masks to allow or deny serving by hash or node. Both generic pieces are slated to move to a separate utilities crate after iroh 1.0, with multiprovider downloads named as the next major feature.

### 2025-10-09 — iroh 0.93.0 - iroh services

- Link: https://www.iroh.computer/blog/iroh-0-93-iroh-online
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Adds Endpoint::online(), which waits until the endpoint is connected to a relay and has a direct address (with a recommended roughly 5-second timeout and a warning it never returns with RelayMode::Disabled), while removing home_relay, direct_addresses, add_node_addr, and remote_info in favour of an infallible node_addr(), watch_node_addr(), a StaticProvider discovery service for out-of-band addresses, and a stopgap Endpoint::latency(). Discovery services can now be added dynamically after endpoint creation, and wire-breaking changes include the mDNS service name changing from iroh.local.swarm to irohv1. It warns prominently that the 0.9x public relays only support 0.92 and 0.93, so users should upgrade to at least 0.92 to keep relaying, while the separate 0.35 relays are unaffected.

### 2025-09-26 — A DHT for iroh - Part 1, The Protocol

- Link: https://www.iroh.computer/blog/lets-write-a-dht-1
- Author: not shown
- Kind: design deep-dive
- Read: full
- Summary: Part 1 of an experimental series building a high-performance Kademlia DHT over iroh for content discovery in iroh-blobs, framed as not production-ready. It chooses a 32-byte keyspace (fitting BLAKE3 hashes and Ed25519 keys), caps values at 1,024 bytes so requests fit a single 1,200-byte QUIC MTU packet, and explains XOR-metric routing with k-buckets (at most 20 × 256 = 5,120 nodes, about 160 KiB) and iterative greedy lookup. The irpc/postcard protocol itself is minimal: Set and streaming GetAll for a multimap store with SetResponse outcomes such as ErrFull and ErrInvalid, plus FindNode returning at most about 20 NodeAddrs, with no Ping needed; value kinds mirror mainline features — BLAKE3 providers, Ed25519 signed messages, and small immutable BLAKE3 data.

### 2025-09-19 — iroh 0.92.0 - mDNS improvements

- Link: https://www.iroh.computer/blog/iroh-0-92-0-mdns-improvements
- Author: not shown
- Kind: release notes
- Read: full
- Summary: A small release centred on mDNS discovery, with two external-contributor features: passive mode via MdnsDiscovery::builder().advertise(false), allowing listening for local nodes without publishing your own, and expiry events, so Discovery::subscribe now streams DiscoveryEvent::Discovered/Expired instead of only DiscoveryItem. Other additions include configurable maximum TLS tickets (useful for many 0-RTT connections), serde support for net_report::Report, a Deref impl for PublicKey, and jitter on DNS retries. The breaking change is the subscribe stream type change and MdnsDiscovery::new gaining an advertise bool parameter.

### 2025-09-01 — Moving from STUN to QUIC Address Discovery

- Link: https://www.iroh.computer/blog/qad
- Author: not shown
- Kind: design deep-dive
- Read: full
- Summary: Explains reflexive transport addresses and NAT mapping types (endpoint-independent versus endpoint-dependent) as the basis for holepunching, then argues STUN's tiny used subset — transaction ID, request, matching response — lacks encryption, reliability, and congestion control. QUIC Address Discovery (QAD), negotiated in the QUIC handshake, instead delivers the observed address in an OBSERVED_ADDRESS frame over the encrypted connection, and because QUIC migrates on network changes it becomes event-based rather than request-response. Iroh and its relays supported both STUN and QAD from 0.32 and switched to QAD exclusively in 0.90; folding QAD into the normal relay QUIC connection instead of a special-purpose one is deferred until after 1.0.

### 2025-08-22 — Trying to get error backtraces in rust libraries right

- Link: https://www.iroh.computer/blog/error-handling-in-iroh
- Author: not shown
- Kind: design deep-dive
- Read: full
- Summary: Surveys Rust error handling — anyhow's single type with backtraces versus thiserror's matchable enums, which cannot reliably propagate backtraces because Rust has not stabilised backtrace propagation — and presents snafu, plus the in-house n0-snafu helpers, as the hybrid iroh adopted from v0.90 for concrete errors with full backtrace chains. Guidelines drawn from the conversion include scoping error enums to functions rather than modules, naming enums descriptively (e.g. ParseError over Error), and giving public-trait errors a Custom/User variant with constructors such as DiscoveryError::from_err. The post is candid about tradeoffs: structured errors cost boilerplate and anyhow remains faster to write, especially in tests. This snafu approach was later superseded by n0-error in 0.95.

### 2025-08-12 — Tutorial: Message Framing with iroh

- Link: https://www.iroh.computer/blog/message-framing-tutorial
- Author: not shown
- Kind: community
- Read: full
- Summary: A beginner tutorial, adapted from a YouTube video, for writing a first custom iroh protocol by length-prefix framing messages on a QUIC stream instead of thinking in raw bytes. It builds a single-file "smol" protocol (ALPN iroh/smol/0) with a Router and ProtocolHandler, implementing write_frame (a u8 length prefix, capping messages at 255 bytes, plus a note to use varints for larger messages) and read_frame (read the length, read_exact into a sized buffer, decode UTF-8, returning None at end of stream). Sender and receiver run in one main function sending "hi" and "fren!", and readers are pointed to the iroh-examples framed-messages example for structured data.
