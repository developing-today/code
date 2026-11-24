### 2026-11-19 — iroh 1.5.0 - Stream Affinity, Then Heads Down
- Link: https://www.iroh.computer/blog/iroh-1-5-0
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Iroh 1.5.0 adds stream affinity, letting an application pin a SendStream to a specific path (Sticky), leave path choice to the connection (Any), or pause it (Block), plus new PathListStream helpers wait_any, wait_none, and wait_closed. It also carries all 1.4.2 fixes plus two more security fixes — a relay handshake timeout leak and proxy passwords appearing in debug logs — supports relay URLs with a path prefix, and deprecates RELAY_PATH in favour of relative-path constants. The team says this is the last feature release of 2026, with only 1.5.x patches while they focus on security hardening until January.

### 2026-11-18 — iroh 1.4.2 - Security Fixes for Relays and Endpoints
- Link: https://www.iroh.computer/blog/iroh-1-4-2
- Author: not shown
- Kind: security
- Read: full
- Summary: A security release urging relay operators and app developers to upgrade both iroh-relay and iroh. Relay fixes stop an empty datagram or an oversized frame from disconnecting the recipient instead of the sender, and fix WebSocket ping floods and stuck non-reading clients that could exhaust memory or block shutdown, several found by Anthropic's OSS Scanner. Endpoint fixes stop a bad or misbehaving relay from halting all relay traffic, and the release also updates tokio, ctutils, and several n0 crates for RUSTSEC advisories.

### 2026-11-10 — irpc 1.0 - Stable RPC for the iroh ecosystem
- Link: https://www.iroh.computer/blog/irpc-1-0
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Irpc reaches 1.0 as the stable RPC library underlying iroh protocols such as blobs and gossip, using zero-overhead tokio channels in-process and length-prefixed postcard messages over QUIC streams cross-process. The stabilisation work reorganised errors into local, serialisation, and connection categories, made mpsc sends cancellation-safe via an extra end/cancel byte in a wire-compatible change, and added Handler::skip_bad_requests to allow additive protocol evolution. Tracing is now always in the public API, with QUIC stream IDs used as per-connection request IDs.

### 2026-11-03 — A new iroh-gossip release and the road to 1.0
- Link: https://www.iroh.computer/blog/iroh-gossip-road-to-1-0
- Author: not shown
- Kind: roadmap
- Read: full
- Summary: Iroh-gossip 0.102.0, described as the last release on the current wire protocol, fixes slow peers blocking the whole actor — patchbay tests show healthy-node p99 latency falling from tens of seconds to tens or hundreds of milliseconds — plus memory leaks, unused connections, lost joins, and Plumtree/HyParView simulation regressions. For the breaking 1.0, planned for a new ALPN with no backwards compatibility, the team proposes authorised topics via opaque app-verified tokens, one broadcast tree per sender, and a clear session-id connection model. The 0.102 release itself makes event/command enums and Metrics non-exhaustive.

### 2026-11-02 — How Strada built peer-to-peer media workflows with iroh
- Link: https://www.iroh.computer/blog/strada-direct-file-transfer
- Author: not shown
- Kind: case study
- Read: full
- Summary: Strada mounts remote drives as virtual drives for creative teams so large media files move directly between devices over iroh, with encrypted relays only as fallback. In July 2026, of 150 TB transferred only 13.9 TB crossed central infrastructure — 90.7% stayed peer-to-peer — which the post models as about $10,310/month or $123,700/year in avoided AWS egress costs versus an all-central design. Strada, founded in 2023 and downloaded by over 12,000 users, adopted iroh as an early v0.35 user with direct maintainer support via an enterprise partnership.

### 2026-10-28 — Announcing the iroh-blobs canary series
- Link: https://www.iroh.computer/blog/iroh-blobs-canary-series
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Iroh-blobs 0.104.0 is presented as the last release before a short canary series leading to 1.0, focused on minimising the public API for long-term stability. It fixes a denial-of-memory issue by loading hash sequences lazily in 1 KiB chunks, rejects non-canonical RangeSpecSeq encodings, stops truncated exports and silent local-store failures from reporting success, and fixes GC, poisoned-entry, deadlock, and panic bugs. Breaking changes remove arrayvec from the public API via Hash::fmt_short and change the bao-tree RPC wire format, so RPC clients and servers must update together; Downloader::new_with_opts is deprecated for a builder.

### 2026-10-19 — iroh 1.4.0 - Security updates and better local networking
- Link: https://www.iroh.computer/blog/iroh-1-4-0
- Author: not shown
- Kind: release notes
- Read: full
- Summary: A maintenance release that updates rustls for four RUSTSEC-2026 advisories, urging relay and DNS server operators to rebuild, and deprecates the X-Iroh-NodeId header for X-Iroh-Endpoint-Id. On local-only networks with no relays, endpoints now advertise LAN addresses immediately on network change instead of waiting for a relay probe, and the release adds relay startup, bounded path queue, captive-portal timeout, IPv6 relay, and HTTP proxy fixes, many from outside contributors. It also introduces a new AI policy requiring AI-generated content in issues/PRs to be placed inside details tags, and previews stream affinity for 1.5.

### 2026-10-13 — Blinded address lookup
- Link: https://www.iroh.computer/blog/blinded-address-lookup
- Author: not shown
- Kind: design deep-dive
- Read: full
- Summary: The post argues iroh endpoint IDs/public keys are not cryptographic secrets, because relays, DNS/mDNS/mainline address lookup, and the QUIC handshake can all reveal them, so they do not satisfy the Granovetter property. It sketches blinded keys — deriving a valid blinded keypair with a possibly time-rotating salt for publishing and lookup, plus encrypting lookup records — with a proof of concept for blinded mainline lookup, and similar sketches for relays and the handshake, while stressing this is not on the roadmap and not vetted. The recommended direction for delegated access is rcan, a simplified UCAN capability scheme, treating the endpoint ID like a home address and the capability as the key.

### 2026-10-09 — Iroh on tiny embedded devices
- Link: https://www.iroh.computer/blog/noq-on-an-esp32-c6
- Author: not shown
- Kind: design deep-dive
- Read: full
- Summary: Full iroh now fits on ESP32-class devices with 4 MiB flash, but for 2 MiB devices the post shows interoperating with a bare noq QUIC endpoint using raw Ed25519 public keys in TLS, a fixed port, and the pure-Rust rustls-rustcrypto provider, without multipath or hole punching. The resulting binary including FreeRTOS is 1.69 MiB (1.46 MiB optimised) versus 3.73 MiB for full iroh on an ESP32-C6, at the cost of relays, hole punching, and address lookup. The authors advise against this for most end-user IoT, suggesting it mainly for device-internal networks with one full-iroh controller, and note experimental Embassy/no_std work for noq.

### 2026-09-14 — iroh on Raspberry Pi Pico 2 RP2350
- Link: https://www.iroh.computer/blog/iroh-on-rp2350
- Author: not shown
- Kind: ecosystem
- Read: full
- Summary: A proof-of-concept port of iroh to the RP2350 on a Pimoroni Presto and Pico 2, achieved by bringing up a std Rust environment rather than forking iroh for no_std — reusing ESP-IDF/FreeRTOS target tricks, the CYW43 WiFi driver with lwIP, and PSRAM as the Rust heap via a custom _sbrk. Display scanout is buffered through core 1 and DMA line buffers in SRAM at about 31 Hz to share the PSRAM bus, and the endpoint runs a single-threaded tokio runtime after SNTP time sync, persisting its secret key in the last flash sector. The author calls the repo a hack but notes production RP2350 support is offered commercially.

### 2026-09-01 — iroh 1.1.0 - Security fixes
- Link: https://www.iroh.computer/blog/iroh-1-1-0
- Author: not shown
- Kind: security
- Read: full
- Summary: The first minor release after 1.0 adds small relay API additions and fixes security issues especially relevant to self-hosted relay operators. A crafted batched relay datagram could pin a receiving CPU core at 100% in a tight loop, and deserialising an EndpointAddr containing a CustomAddr could panic via an out-of-bounds index; the release also stops NAT-probe noise packets from flowing over the relay transport. A new relay Status::RateLimited message is surfaced as a warning to endpoints being rate-limited.

### 2026-07-30 — Protect your relays
- Link: https://www.iroh.computer/blog/authenticated-relays
- Author: not shown
- Kind: security
- Read: full
- Summary: Managed relays on Iroh Services deployed from June 2026 onward are now authenticated by default, because a relay URL ships in every client and cannot be revoked once leaked. Endpoints present a signed capability token in an Authorization Bearer header, issued by the project's API key, bound to that endpoint's public key, granting only relay use, and expiring after a configurable window; rotating or deleting the API key revokes its tokens and drops their connections. Pre-June 2026 relays stay open until owners enable authentication in settings, the iroh_services preset mints tokens automatically, and scoped tokens and per-endpoint revocation are planned.

### 2026-07-21 — iroh 1.0.3 - Steadier Connections
- Link: https://www.iroh.computer/blog/iroh-1-0-3
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Iroh 1.0.3, shipped with noq 1.1.0, fixes connection corner cases including more consistent path-validation timing, handling data for abandoned paths, an active-connection undercount, and handshake packet coalescing, eliminating two named overflow/assertion panics. An empty ALPN now returns a clear error instead of panicking, logs are quieter, and upstream quinn fixes were merged. The n0 preset now includes an HTTPS-based pkarr resolver as fallback when the DNS resolver fails.

### 2026-07-11 — Mesh LLM: distributed AI computing on iroh
- Link: https://www.iroh.computer/blog/mesh-llm
- Author: not shown
- Kind: ecosystem
- Read: full
- Summary: Mesh LLM pools GPUs and memory across many machines and exposes them as a single OpenAI-compatible API at localhost:9337/v1, serving requests locally, routing them to a peer that has the model loaded, or splitting a model by layer ranges across machines in its Skippy pipeline mode. Its catalogue covers 40+ models from 0.5B to 235B MoE, in an approximately 18 MB install with a plugin runtime exposed over MCP, HTTP, and mesh events. Every node is an iroh endpoint with no central server, using two regional relays as fallback and three ALPNs for the main mesh, owner control plane, and activation transport.

### 2026-07-09 — The road to iroh 1.0
- Link: https://www.iroh.computer/blog/the-road-to-iroh-1-0
- Author: not shown
- Kind: roadmap
- Read: full
- Summary: A four-year retrospective on pivoting from an IPFS-compatible implementation to just QUIC connections addressed by Ed25519 endpoint ID, via the DeltaChat backup use case, BLAKE3 verified streaming, Tailscale-style hole punching with relays, and pkarr over the BitTorrent mainline DHT for address lookup. Key decisions recounted include sticking with TLS using raw public keys, using ALPN for protocol negotiation, narrowing iroh from a batteries-included toolkit to connections only with other protocols in separate crates, running a 0.35 LTS plus canary series, and forking quinn into noq for true multipath. Deferred items include non-Ed25519 keys, post-quantum signatures (though post-quantum key exchange is supported), and Encrypted ClientHello.

### 2026-07-07 — iroh 1.0.2 - iroh-relay Security Fix
- Link: https://www.iroh.computer/blog/iroh-1-0-2
- Author: not shown
- Kind: security
- Read: full
- Summary: Iroh-relay 1.0.2 fixes a pre-auth denial-of-service: a missing length check on client-to-relay datagram frames let any connecting client crash a relay built with panic=abort with a single malformed frame shorter than an endpoint ID, and the fix adds extensive fuzz testing of relay protocol parsers; n0 public relays were already patched and 0.35 relays were unaffected. The release also makes per-client relay rate limits live-updatable via set_client_rate_limit without dropping existing connections. Relay operators are urged to upgrade now; clients are not affected.

### 2026-07-02 — An iroh powered smart fan
- Link: https://www.iroh.computer/blog/an-iroh-powered-smart-fan
- Author: not shown
- Kind: ecosystem
- Read: full
- Summary: A full end-to-end tutorial building a cloud-free smart fan on an ESP32-WROVER with 4 MiB PSRAM running full iroh, a DHT22 temperature/humidity sensor on GPIO 26, and a 5V PWM fan switched with hysteresis around a threshold. It defines an irpc protocol in a shared crate, serves it alongside echo via the Router, adds a live WebAssembly browser GUI, and evolves the protocol compatibly by appending RPC methods because postcard is not self-describing, adding a secret-gated SetThreshold call while keeping a public read-only view. The finished device is housed in a 3D-printed enclosure, persists its secret key in non-volatile memory for a stable endpoint ID, and is reachable globally via its home relay using a short ticket.

### 2026-06-29 — iroh 1.0.1 - Just Boring
- Link: https://www.iroh.computer/blog/iroh-1-0-1
- Author: not shown
- Kind: release notes
- Read: skim
- Summary: The first 1.x patch release is deliberately presented as having no headline feature, only a handful of corner-case bug fixes across iroh, netwatch, and mostly noq. The most visible change is that the Android DNS resolver no longer panics when the JNI context was not initialised, where panics can be caught. The team frames boring maintenance releases every few weeks as the expected rhythm for the stable series.

### 2026-06-18 — Iroh language support
- Link: https://www.iroh.computer/blog/iroh-language-support
- Author: not shown
- Kind: ecosystem
- Read: full
- Summary: First-party Swift, Kotlin, Python, and JavaScript bindings are announced, exposing QUIC data streams between endpoints dialed by EndpointId across iOS, macOS arm64, Android, Linux, Windows, and FreeRTOS, with a claimed 95–99% direct connection rate. An accelerometer demo streams positions at about 60 Hz over a bidirectional stream while showing live path, RTT, and hole-punching progress, and bindings can connect to Iroh Services for direct-vs-relay metrics and network diagnostics reports. Future work aims to add RPC, gossip, blobs, and documents to the bindings, plus mDNS and Bluetooth in the FFI once the underlying Rust APIs stabilise.
