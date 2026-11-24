---
created: 2026-11-23
last-modified: 2026-11-24
revision: 1
---

# The Iroh Blog — Complete Catalogue & Findings

**Compiled:** 2026-11-23 · **Scope:** every post on the iroh.computer blog index — **98 posts, 2023-02-17 → 2026-11-19** · **Method:** each post opened and read on iroh.computer; entries below record `Read: full` or `Read: skim` (short announcements only). No author byline is shown on any post page. Companion files in this folder: `protocol-census.md`, `talks-and-announcements.md`, `real-projects-and-community.md`.

---

## ⚠ Time-sensitive — read this first

1. **iroh-gossip 1.0 will break wire compatibility.** New ALPN, no backwards compatibility; **0.102.0 is the last release on the current wire protocol** (2026-11-03). Any gossip swarm must upgrade all at once or ship both versions.
2. **iroh-blobs is in a canary run-up to 1.0.** 0.104.0 is the last pre-canary release; the canary series changes the public API *and* the bao-tree RPC wire format — RPC clients and servers must update together (2026-10-28).
3. **iroh 1.5.0 (2026-11-19) is the last feature release of 2026.** Only 1.5.x patches until January; feature triage and PR labelling paused.
4. **Public-relay end-of-support dates** (from the 1.0 post, 2026-06-15): v0.35x relays supported through **2026-12-31**; v0.9x and 1.0.0-rcX through **2026-09-30** (already past); canary/RC lines get no support after 1.0 day.
5. **Security-upgrade train for relay operators:** 1.0.2 fixed a *pre-auth* relay DoS (one malformed frame could crash a relay); 1.1.0 fixed a crafted-datagram CPU-pin; 1.4.0 updated rustls for four RUSTSEC-2026 advisories; 1.4.2 fixed further relay/endpoint issues, several found by Anthropic's OSS Scanner. Self-hosted relays should be on 1.4.2+.
6. **iroh-blobs push vulnerability (2026-10-08):** blobs 0.94.0–0.103.0 with the default EventMask silently accepted `push` from any connecting peer (disk-fill risk). Patch releases (.1) are out and broken versions yanked; sendme fixed in 0.36.1. Anyone pinning blobs in that range should bump.
7. **Managed relays are authenticated by default** for Iroh Services deployments from June 2026 onward; pre-June relays stay open until the owner enables authentication (2026-07-30).

---

## Findings — what the blog actually says, across all 98 posts

### 1. The 1.0 story is a deliberate narrowing, and the wire is now frozen
"The road to iroh 1.0" (2026-07-09) is the single best design-history source: four years from an IPFS-compatible toolkit to *just* QUIC connections dialed by Ed25519 endpoint ID, via the Delta Chat second-device backup use case, BLAKE3 verified streaming (bao), Tailscale-style hole punching with relays, and pkarr-over-Mainline address lookup. Decisions that stuck: TLS with raw public keys; ALPN for protocol negotiation; protocols (blobs, gossip, docs) split into separate crates; a 0.35 LTS plus canary series to de-risk the run-up; and forking quinn into **noq** for true QUIC multipath. Explicit non-goals/deferrals: non-Ed25519 endpoint keys, post-quantum *signatures*, Encrypted ClientHello. iroh 1.0 shipped 2026-06-15 ("Dial Keys, not IPs") claiming wire + API stability, ~200M endpoints created on public relays in the prior 30 days, and ~95% direct traffic.

### 2. Identity stays Ed25519; post-quantum is key-exchange-only, opt-in
The PQ post (2026-05-19) separates the two risks cleanly: harvest-now-decrypt-later threatens the *key exchange* today, while broken Ed25519 threatens impersonation only once a cryptographically relevant quantum computer exists — no pre-harvest urgency. Hybrid X25519MLKEM768 is available via the aws-lc-rs provider (`tls-aws-lc-rs`, rustls prefer-post-quantum), prefer-not-force, ~1 KB each way; ring can't do it. Migrating EndpointIds off Ed25519 "awaits industry consensus". For our transport design (#13752) this confirms: NodeId-as-Ed25519 is a long-lived commitment in their roadmap, not a placeholder.

### 3. Discovery is the most actively re-thought layer
- **Address lookup:** STUN was replaced by QUIC Address Discovery (QAD) — OBSERVED_ADDRESS frames inside the encrypted QUIC connection, event-based via migration; exclusive since 0.90 (2025-09-01). At 1.0.0-rc.0, DHT and mDNS lookups moved *out* of core into the separate `iroh-address-lookups` crates.
- **Content discovery has two experimental directions, a year apart:** an in-protocol clean-room Kademlia DHT over iroh itself (2025-09-26, 32-byte keyspace, 1 KiB values sized to one QUIC MTU, minimal irpc Set/GetAll/FindNode — explicitly not production) and, a year later, **global content discovery on BitTorrent Mainline** (2026-09-30) with a tiny `udp-addr-index` service bridging host:port → EndpointId, `blake3.net`/`pkarr.net` link namespaces, a browser plugin + local gateway, and an `iroh-share` daemon. The 2026 post's recap names "writing our own DHT using iroh connections" as the possible successor — i.e. the 2025 DHT series is a design study, and Mainline is the shipping experiment.
- **Blinded address lookup is a proof-of-concept, NOT on the roadmap** (2026-10-13): endpoint IDs are public by construction (relays, DNS/mDNS/Mainline lookup, and the handshake all reveal them); the post sketches blinded keypairs but steers delegation questions to **rcan** capabilities instead. This directly confirms our census note.

### 4. Relays: stateless, increasingly authenticated, and the main attack surface
The relay story across the blog: relays store nothing and cannot read E2E-encrypted traffic; their job is rendezvous + fallback. Commercially, n0 now sells that operational burden back — shared relays on Iroh Services (Pro $19/mo, 5 MB/s, 100 GB included; Dedicated $199/mo/region) alongside free-but-increasingly-rate-limited public relays (endpoints get explicit rate-limit signals since 1.1.0) and free self-hosting (2026-09-08). Since June 2026, managed relays authenticate by default with **capability tokens bound to the endpoint's public key**, issued from the project API key, revocable by key rotation (2026-07-30); on the protocol side, 1.0.0-rc.1 replaced relay `AccessConfig` with a pluggable `AccessControl` trait — the hook point for our own relay policy. The security cadence is relentless and relay-centric (see the time-sensitive box): if we self-host relays, tracking their patch train is an operational requirement, not a nicety.

### 5. Hole punching moved *into* QUIC — and had a rough year
0.96 (2026-01-27) completed the switch from custom hole punching to QUIC NAT Traversal integrated with multipath — and shipped with a flagged regression (hole punching not re-triggered on most network changes, connections stranded on relays). 0.97–0.98 were largely NAT-traversal repair releases backed by an expanded patchbay test matrix; rc.1 added server-side hole punching behind hard NATs. Address-validation tokens (NEW_TOKEN) turned out to matter for real UX: they lift QUIC's 3× anti-amplification cap that otherwise stalls 0-RTT responses — an 8 KB blob fetch dropped ~418 ms → ~200 ms (2026-06-10). Custom transports (Tor live, BLE planned) arrived experimentally in 0.97, with a PathSelector trait by rc.1.

### 6. The ecosystem evidence is stronger than the marketing
Production stories carry numbers: Strada moved 150 TB in July 2026 with 90.7% peer-to-peer (~$123,700/yr of avoided AWS egress, modelled); Paycode runs PCI payment-terminal traffic over iroh on Windows 7-era hardware; Block's Buzz embeds MeshLLM to share spare compute for agents over iroh with membership-signed, policy-checked discovery and fenced session ownership (2026-09-22). Mesh LLM itself (2026-07-11) exposes pooled GPUs as one OpenAI-compatible API. First-party Swift/Kotlin/Python/JS bindings returned at 1.0 (2026-06-18). The embedded push is real but honest about cost: full iroh on ESP32 needs a pure-Rust crypto provider (ring/aws-lc-rs both fail on Xtensa), PSRAM, and size work; 1.2.0's n0-dns-resolver cut >10% of binary size. Iroh Services is becoming the commercial control plane (endpoint groups + attributes in 2.0, metrics, diagnostics).

### 7. Smaller findings worth keeping
- **irpc 1.0** (2026-11-10) stabilises the RPC layer under blobs/gossip — postcard over QUIC streams, cancellation-safe sends, additive protocol evolution via `skip_bad_requests`.
- **Performance receipts:** QUIC packet-rejection modes measured under spam (silent-ignore ≈36.6k ops/s/core vs full handshake ≈1k); BLAKE3's internal `hash_many` SIMD path hashes many small blobs ~17× faster combined with rayon; batched datagrams in 1.3.0 cut lock/wake-up overhead.
- **Error-handling churn:** snafu (0.90) → n0-error (0.95) within five releases — public error types in this ecosystem are still moving even as the wire froze.
- **An AI policy** appears in the 1.4.0 notes: AI-generated content in issues/PRs must sit inside details tags.
- **Direct-rate honesty:** their own worked example uses a 93.1% direct data rate and labels the 38–39× cost claim "back-of-the-napkin" — the number to quote is the direct-rate dependency, not the multiplier.

### Deltas against our existing census (`protocol-census.md` etc.)
- ✅ **Confirmed:** iroh 1.0 shipped 2026-06-15, current 1.5.0 (2026-11-19); Delta Chat's backup-transfer origin story; blinded lookup = PoC, rcan named as the delegation answer; sendme fix line (0.36.1) precedes the census's current 0.37.1.
- ⚠ **Sharpen:** the census should flag the **gossip 1.0 wire break** and the **blobs canary RPC wire change** as the two live migration hazards, and the **public-relay support end dates** (0.35x: 2026-12-31) for anything still pinned to old lines.
- ➕ **Add:** Mainline-based global content discovery + `udp-addr-index` + `iroh-share` (Sep 2026) as the current content-discovery experiment, superseding the 2025 in-protocol DHT series as "the shipping direction"; relay `AccessControl` trait + capability-token auth model as the reference design for authenticated self-hosted relays; NEW_TOKEN/0-RTT anti-amplification behaviour as a known latency factor for small-blob fetches.
- ➕ **Add (commercial):** Iroh Services pricing/shape (Pro $19, Dedicated $199/region, shared-relay auth) — corroborates the third-party stackgems figures in Part 3.

---

## Part 2 — The catalogue (all 98 posts, newest first)

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

### 2026-10-08 — iroh-blobs security fix
- Link: https://www.iroh.computer/blog/iroh-blobs-push-fix
- Author: not shown
- Kind: security
- Read: full
- Summary: iroh-blobs 0.103.1 (plus patch releases back to 0.97.1, with the broken versions yanked) fixes an EventMask bug: the shared request handler checked `mask.get` for every request type, so `push` requests were silently accepted under EventMask::DEFAULT and forwarded as events under ALL_READONLY — any peer that could connect to a provider could write arbitrary (content-addressed, verified) blobs into its store and fill its disk. All versions 0.94.0–0.103.0 are affected; sendme was also exposed while `sendme send` ran (fixed in sendme 0.36.1). Behaviour now matches the docs, which also means `get_many` and `observe` follow their own modes — anyone who gated access via `get` alone must set those modes too or they bypass the check.

### 2026-10-02 — Group your endpoints and deep dive into attributes
- Link: https://www.iroh.computer/blog/iroh-services-2-0
- Author: not shown
- Kind: ecosystem
- Read: full
- Summary: Iroh Services 2.0 adds endpoint groups (one named group per endpoint, e.g. staging/production/canary, changeable at runtime via `Client::set_group()`) and attributes (up to 128 custom key-value pairs such as region, app version, device type) so fleets of endpoints can be organised and filtered in the dashboard. It also fixes metrics delivery across reconnects (clients re-authenticate and resend the schema; `Client::shutdown()` attempts a final flush) and brings network diagnostics to WebAssembly/browser apps. The 2.0 release is breaking: protocol types move into `iroh-services-proto`, the client API no longer exposes rcan or irpc types directly, and `Error`/`BuildError` are now non-exhaustive.

### 2026-09-30 — Iroh global content discovery
- Link: https://www.iroh.computer/blog/iroh-global-content-discovery
- Author: not shown
- Kind: design deep-dive
- Read: full
- Summary: Motivated by IPFS's Shipyard shutdown, this design post builds permissionless global content discovery for iroh-blobs on BitTorrent's Mainline DHT, praising Mainline's extreme minimalism (single-packet queries, token-gated writes, millisecond lookups at global scale). Because Mainline provider records are bare host:port pairs behind NATs, n0 adds a tiny in-memory `udp-addr-index` service mapping a verified UDP host:port to a signed record (≤1 KiB, last-writer-wins, expiring) naming the EndpointId; providers announce SHA-1(BLAKE3 hash) as the Mainline infohash, and discovery verifies candidates with a BLAKE3 size query before downloading via normal iroh connections. On top sit `https://<hash>.blake3.net` content links and pkarr-based mutable names (`<key>.pkarr.net`), a browser plugin that rewrites both to a local gateway, and an `iroh-share` publishing daemon. The recap is candid about the limits: Mainline is unencrypted and easily blocked, provider IPs are public (no privacy), pkarr is Ed25519-only (not post-quantum), and a future in-house DHT over iroh connections is named as the possible successor — all implemented today only in the experimental iroh-content-discovery repo.

### 2026-09-28 — iroh 1.2.0 & 1.3.0 - Small features for more user control
- Link: https://www.iroh.computer/blog/iroh-1-3-0
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Two maintenance releases. 1.2.0 replaces hickory-resolver with the in-house n0-dns-resolver, cutting binary size by more than 10% (iroh now runs on ESP32 with 4 MiB flash out of the box) and making DNS fallback configurable (Deferred/Eager/IfSystemEmpty/Never modes; the old `with_nameserver(s)` builders are deprecated), and it surfaces relay authentication failures distinctly via `RelayStatus::auth_denied_reason` instead of treating them as transient reconnects. 1.3.0 adds batched datagram send/receive in noq (`send_many_datagrams`/`read_many_datagrams`, one lock and one driver wake-up per batch), makes ALPN negotiation follow Router registration order rather than alphabetical order, and fixes a blocked-actor stall when a relay send queue filled during the Initial packet. The post promises this calm rhythm continues, with bigger features gearing up before the end of the year.

### 2026-09-22 — Buzz: share spare compute and run open models together over p2p networks
- Link: https://www.iroh.computer/blog/buzz-agent-workspaces
- Author: not shown
- Kind: ecosystem
- Read: full
- Summary: A guest ecosystem post on Block's Buzz, an agent workspace where people and coding agents collaborate in channels, with MeshLLM built in so community members can opt in to share spare GPU/compute and run open models for each other's agents over iroh — no central server in the inference path, exposed to agents as an OpenAI-compatible endpoint at 127.0.0.1:9337/v1. Buzz community membership is the trust perimeter: serving targets become selectable only after member-signed, endpoint-bound, size-bounded discovery records pass local relay-policy checks; the Buzz relay owns membership, signed status, discovery and history while MeshLLM owns inference transport, allowlists and routing. Buzz's inter-relay mesh runs one iroh endpoint per relay runtime with attested readiness records and fenced tunnel traffic (a Redis CAS lease arbitrates session ownership; mesh membership is only a routing hint), and Goose's optional `roam` feature is named as the next step toward agent-to-agent communication over iroh.

### 2026-09-14 — iroh on Raspberry Pi Pico 2 RP2350
- Link: https://www.iroh.computer/blog/iroh-on-rp2350
- Author: not shown
- Kind: ecosystem
- Read: full
- Summary: A proof-of-concept port of iroh to the RP2350 on a Pimoroni Presto and Pico 2, achieved by bringing up a std Rust environment rather than forking iroh for no_std — reusing ESP-IDF/FreeRTOS target tricks, the CYW43 WiFi driver with lwIP, and PSRAM as the Rust heap via a custom _sbrk. Display scanout is buffered through core 1 and DMA line buffers in SRAM at about 31 Hz to share the PSRAM bus, and the endpoint runs a single-threaded tokio runtime after SNTP time sync, persisting its secret key in the last flash sector. The author calls the repo a hack but notes production RP2350 support is offered commercially.

### 2026-09-08 — Share relay servers to save money and heartache
- Link: https://www.iroh.computer/blog/shared-relays
- Author: not shown
- Kind: ecosystem
- Read: full
- Summary: Launches shared relay servers through Iroh Services: multiple projects share n0-operated relay, metrics and diagnostics infrastructure with per-project authentication keeping access and metrics separate. Pricing: Community $0 (public relays, variable rate limits), Pro $19/mo (shared relays, 5 MB/s, 100 GB egress included then $0.09/GB, 10,000 concurrent connections), Dedicated $199/mo per region plus Pro (unlimited rate, 250 GB included, custom DNS, 60,000 connections). The cost argument is worked in full: at 98% direct traffic, a 20 TB/month workload costs ~$46 on Iroh Pro versus ~$1,742 of AWS egress (a claimed ~38–39× saving, explicitly labelled back-of-the-napkin). The FAQ confirms public relays are not going away but are rate-limited more strictly as usage grows (with rate-limit notices surfaced to endpoints since iroh 1.1.0), relays store no data and cannot read end-to-end-encrypted traffic, Pro relay access can be gated by project API-key tokens, and self-hosting relays remains free.

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

### 2025-08-05 — iroh on QUIC Multipath
- Link: https://www.iroh.computer/blog/iroh-on-QUIC-multipath
- Author: not shown
- Kind: design deep-dive
- Read: full
- Summary: Explains how iroh currently multiplexes QUIC and holepunching datagrams over one socket via the magic socket, lying to the QUIC stack about paths so congestion control must be restarted on path changes. It argues for adopting the IETF QUIC-MULTIPATH draft in Quinn — per-path congestion control and RTT, seamless primary/backup path switching — plus QUIC-NAT-TRAVERSAL to move holepunching itself inside QUIC for encryption, loss recovery and anti-amplification protection. The work is in progress, changes the wire protocol, and is targeted to land in the 0.9x series before 1.0 freezes compatibility.

### 2025-08-01 — iroh 0.91.0 - Making relays use established standards
- Link: https://www.iroh.computer/blog/iroh-0-91-0-the-last-relay-break
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Announces the final relay wire-level breaking change: relays are now WebSocket-only, the relay protocol is split into handshake and messaging parts, and every relayed datagram carries an ECN byte to enable future relay congestion signalling. The handshake fixes a replay weakness by signing a server challenge, using TLS keying-material exporters (RFC 9729 concealed authentication) to avoid an extra round trip, with a fallback for browsers. Watcher APIs lose their unnecessary Results, default relay URLs change again, and the post maps how long the 0.90 canary and 0.35 relays will stay up.

### 2025-07-21 — IRPC A lightweight rpc crate for iroh protocols
- Link: https://www.iroh.computer/blog/irpc
- Author: not shown
- Kind: ecosystem
- Read: full
- Summary: Introduces irpc, a boilerplate-reducing crate that lets one serializable request enum serve in-process (tokio mpsc/oneshot, zero overhead), cross-process and cross-machine (iroh-quinn streams, postcard with varint length prefixes) RPC. A macro generates the request-with-channels enum and client dispatch for plain RPC, client/server streaming and bidi streaming, with compile-time checks on channel use. It deliberately does not abstract connection or serialization types, and covers error serialization (serde-error/anyhow) and detecting a closed remote via a Sender closed() wrapper.

### 2025-07-08 — iroh-blobs 0.90 - New Request Types and Features
- Link: https://www.iroh.computer/blog/iroh-blobs-0-90-new-features
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Details new iroh-blobs 0.90 protocol requests: GetMany for fetching several hashes/ranges in one sequential request, Push for uploads (PushMany planned before 1.0), and Observe, which streams bitfields of a peer's chunk availability. The remote API is split into per-node Remote execution and a stateful multi-provider Downloader driven by SupportedRequest and ContentDiscovery traits with a split strategy. Provider events gain per-node and per-request access control (notably for Push), batch adds return TempTags versus persistent tags, and the rewritten store tracks per-blob bitfield files to handle gaps.

### 2025-07-04 — iroh-blobs v0.90 - The Upgrade Guide
- Link: https://www.iroh.computer/blog/iroh-blobs-0-90-changes
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Frames blobs 0.90 as a complete rewrite of the filesystem and in-memory stores and API, replacing quic-rpc with irpc so a single API has zero in-process overhead. It explains the namespace-structured API, Progress structs that implement IntoFuture but can also be streamed, _with_opts methods plus convenience overloads and builders, and snafu concrete errors replacing anyhow. The Get protocol and on-disk store format stay compatible with 0.35 (plus a new bitfield file), small-blob performance improves sharply — sendme syncing the Linux kernel source dropped from about 8:12 to 3:45 — and production users are advised to wait one or two more releases.

### 2025-06-27 — iroh v0.90 - The Canary Series
- Link: https://www.iroh.computer/blog/iroh-0-90-the-canary-series
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Launches the 0.90 canary series leading to 1.0, maintained alongside 0.35.x, with no network/protocol stability guarantee until 1.0-rc — bleeding-edge and prototyping users should move, production networks should stay on 0.35. Highlights include concrete error types throughout, removal of x509 TLS options now that raw public keys are default, relays dropping STUN in favour of QUIC address discovery, and the new n0-watcher Watcher API for node_addr and home_relay. It also warns node_addr/home_relay timing semantics changed subtly, so code cannot assume a RelayUrl arrives with the first update.

### 2025-05-21 — 0-RTT When to use and when not to use
- Link: https://www.iroh.computer/blog/0rtt-api
- Author: not shown
- Kind: design deep-dive
- Read: full
- Summary: Walks through the normal TLS handshake versus 0-RTT resumption with pre-shared keys from NewSessionTicket messages, stressing replay attacks and loss of forward secrecy for early data, so 0-RTT suits only idempotent, cheap requests. It shows the into_0rtt() accept/connect patterns in an echo example and measures roughly 230ms 0-RTT rounds versus about 540-560ms full-handshake rounds on a Europe-to-Asia link. A key gotcha is single-use tickets: closing immediately starves the client of new tickets, fixed by keeping the connection open about two RTTs in a background task, plus a Wireshark/keylog walkthrough.

### 2025-05-13 — iroh v0.35 - Prepping for 1.0
- Link: https://www.iroh.computer/blog/iroh-0-35-prepping-for-1-0
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Declares 0.35 the last release before the protocol/network breaking change, to be maintained with patches while the next line heads to a release candidate (then described as 0.99). Changes are modest: metrics move from a global collector to per-Endpoint collection, and Router::spawn becomes synchronous and infallible. It previews the RC: x509 removal, old relay codec removal in favour of WebSockets, concrete errors replacing anyhow, and QUIC multipath/NAT traversal, and notes quic-rpc and iroh-node-utils will no longer be maintained, superseded by irpc.

### 2025-04-08 — The new BLAKE3 hazmat API
- Link: https://www.iroh.computer/blog/blake3-hazmat-api
- Author: not shown
- Kind: design deep-dive
- Read: full
- Summary: Explains why iroh-blobs needed BLAKE3 internals for verified streaming — hashing individual chunks and 16-chunk subtrees and combining chaining values — and why it maintained a fork of the BLAKE3 crate for 18 months to get a parallel hash_subtree function, at the cost of symbol collisions and drift from upstream. The new upstream hazmat API is a stable public API with a HasherExt trait for setting input offset and finalizing to non-root chaining values, plus merge_subtree_root/non_root functions. That lets iroh retire its fork, and the post urges other BLAKE3 forkers (Fleek, Fluence, S5) to do the same.

### 2025-03-18 — A richer tags api
- Link: https://www.iroh.computer/blog/a-richer-tags-api
- Author: not shown
- Kind: design deep-dive
- Read: full
- Summary: Explains tags in iroh-blobs as named pins: an arbitrary-byte name mapping to a hash plus Raw/HashSeq format, and the only protection against garbage collection, including indirectly via a tagged hash sequence. The expanded API gives full key-value behaviour — get, list by range or prefix, bulk delete by range/prefix and atomic rename — enabling patterns like expiring tags encoded with sortable timestamps and binary (node-id, entity-id) tag keys. It also covers lightweight in-memory temp tags for atomic multi-step adds and the collection convention of storing metadata as the first child of a hash sequence.

### 2025-03-18 — iroh v0.34 - Raw Public Keys in TLS
- Link: https://www.iroh.computer/blog/iroh-0-34-0-raw-public-keys
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Makes raw public keys in TLS certificates (per the TLS 1.3 RFC) the default, replacing the libp2p-derived self-signed x509 approach and its custom verification code, with .tls_x509() as a temporary opt-back-in. The post warns this is a wire-level break — pre-0.34 nodes cannot talk to raw-public-key nodes — and that the x509 option will be removed before 1.0. The release also ships the expanded iroh-blobs tags API and renames LocalSwarmDiscovery to MdnsDiscovery.

### 2025-03-12 — Blob store design challenges
- Link: https://www.iroh.computer/blog/blob-store-design-challenges
- Author: not shown
- Kind: design deep-dive
- Read: full
- Summary: A deep look at why storing BLAKE3 blobs plus bao outboards (kept only to 16 KiB chunk groups) is hard: databases (redb) are too slow and heavy for large files, pure filesystem storage collapses under millions of tiny blobs, so iroh-blobs uses a hybrid — blobs under a 16 KiB threshold inline in the database, larger ones as .data/.obao4 files with minimal metadata. It covers unknown-size arrivals, batched metadata transactions trading durability for throughput (fsync costs milliseconds, capping naive sync-per-chunk writes at about 3.2 MB/s), and keeping the validity bitfield in memory, syncing data/outboard first and only then persisting bitfield changes to stay crash-consistent.

### 2025-02-25 — iroh 0.33.0 - Browsers and Discovery and 0-RTT, oh my!
- Link: https://www.iroh.computer/blog/iroh-0-33-0-browsers-and-discovery-and-0-RTT-oh-my
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Ships three headline features: iroh and iroh-gossip compile to Wasm and run in browsers (relay-only, no direct connections, with echo and chat demos), Discovery can publish/resolve opaque user-defined data up to 245 UTF-8 bytes (e.g. supported ALPNs), and a new discovery_stream plus remote_info tooling lets apps build their own address books. It also enables 0-RTT via connect_with_opts/into_0rtt, caching up to 8 session tickets per endpoint for up to 32 endpoints in memory. Breaking changes remove the global DNS resolver and rework NodeInfo/Discovery publish signatures.

### 2025-02-21 — Consensus is Impossible
- Link: https://www.iroh.computer/blog/consensus-is-impossible
- Author: not shown
- Kind: community
- Read: full
- Summary: A short companion to a video dramatizing Nancy Lynch's impossibility results, including the FLP proof, showing certain consensus is impossible in an asynchronous distributed system with even one faulty node. It connects the theory to iroh: iroh-docs is a sync protocol guaranteeing eventual consistency rather than simultaneous agreement, and iroh-blobs sidesteps Byzantine faults via content addressing. It also explains the 2t+1 to 3t+1 shift when tolerating Byzantine nodes, meaning fewer than one-third of nodes can be faulty.

### 2025-02-12 — Update On FFI Bindings
- Link: https://www.iroh.computer/blog/ffi-updates
- Author: not shown
- Kind: ecosystem
- Read: full
- Summary: Announces that the Kotlin, Python, Swift and JavaScript FFI bindings will not be updated with iroh releases until the FFI experience improves, citing quality below the 'just works' bar and a risk that language-specific protocols fragment the ecosystem. Three paths are being explored: leaning on community bridges like flutter_rust_bridge and React Native UniFFI, building composable FFI generation across iroh protocols (blocked on UniFFI/maturin issues), or native per-language implementations after a 1.0 spec exists. Browser/Wasm support continues, existing iroh-ffi stays open to community PRs, and a decision is tied to the 1.0 roadmap in the second half of 2025.

### 2025-02-04 — iroh 0.32.0 - Browsers Alpha, QAD, and n0-future
- Link: https://www.iroh.computer/blog/iroh-0-32-0-browser-alpha-qad-and-n0-future
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Offers a browser alpha via a git branch: iroh compiles to wasm32 with wasm-bindgen, relay-only over WebSockets with no holepunching, and several features unsupported. It implements QUIC Address Discovery on the relays and races it against STUN in net-report, so high-latency, lossy networks that defeated UDP-based STUN/ICMP now get a delayed answer instead of none. It also introduces n0-future, a curated single crate re-exporting futures/tokio pieces that work on both native and Wasm, and makes discovery-pkarr-dht opt-in.

### 2025-01-15 — iroh 0.31.0 - Back At Fighting Fit
- Link: https://www.iroh.computer/blog/iroh-0-31-0-back-to-fighting-fit
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Fixes holiday regressions from 0.30, notably a connection flop where a PkarrPublisher refactor published direct addresses alongside the relay URL and discovery parsed only the first DNS record, often losing the relay URL needed to coordinate holepunching — rolled back, fixed, and covered with better tests — plus a DNS/pkarr dependency segfault on some Linux/Intel Mac systems. New features are relay access control (an access function, allowList or denyList on RelayConfig) and a programmatic PathSelection::RelayOnly test mode replacing the DEV_RELAY_ONLY env var. It also notes a relay-server deadlock refactor bug that never shipped in a release.

### 2024-12-17 — iroh 0.30.0 - Slimming Down
- Link: https://www.iroh.computer/blog/iroh-0-30-0-slimming-down
- Author: not shown
- Kind: release notes
- Read: full
- Summary: A cleanup release streamlining APIs ahead of 1.0: key types become top-level exports, NodeTicket moves to iroh_base and BlobTicket to iroh_blobs, and a unified Watchable/Watcher API replaces conn_type_stream and watch_home_relay for values like home_relay and connection type. Protocol setup examples for gossip, blobs and docs are shortened, ProtocolHandler no longer requires explicit Arcs, and many dependencies are cut across a series of PRs. MSRV rises from 1.76 to 1.81, with a long list of moved/removed exports.

### 2024-12-05 — iroh 0.29 - net is the new iroh
- Link: https://www.iroh.computer/blog/iroh-0-29-net-is-the-new-iroh
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Completes the slimming announced earlier: iroh-net is renamed to iroh, blobs/docs/gossip become separate crates, iroh is no longer a CLI, and the bundled Node is replaced by Endpoint plus Router, with an extensive upgrade guide showing how to rebuild node-like setups for each protocol or a custom one. Users converting from a default Node must explicitly enable discovery_n0. Relay changes include /relay/probe moving to /ping and a new default incoming rate limit of 4 KiB/s steady with 16 MiB burst.

### 2024-11-19 — Relay outage: A post-mortem
- Link: https://www.iroh.computer/blog/relay-down-a-post-mortem
- Author: not shown
- Kind: case study
- Read: full
- Summary: Post-mortem of the 5 November 2024 global relay outage: degraded from about 11:00 GMT, full outage by 17:00, only reported internally at 22:50 and resolved at 23:01, affecting new holepunched connections but not existing or direct ones. Root causes were a traffic spike (peaking at 100,000 concurrent connections and nearly 1 million new nodes), memory that grew and never freed (leaked tokio tasks/threads, fixed afterwards), and unbounded logging filling disks, compounded by no alerting and an underpowered Asia node. Fixes included quadrupling capacity, log rotation/limits, re-enabled alerting, load-shedding plans and load-simulation testing.

### 2024-11-06 — iroh 0.28.0 - Let them have crates
- Link: https://www.iroh.computer/blog/iroh-0-28-let-them-have-crates
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Starts the 1.0 slimming refactor: gossip, docs and blobs move to their own repos/crates, iroh-router is created, and portmapper/netwatcher are pulled out of iroh-net toward a future net-tools repo; 0.28.0 itself was yanked, with 0.28.1 the canonical version across the crates. It deprecates the bundled treatment of the n0 protocols, the CLI and the RPC/client API, all to be removed or relocated in 0.29, and adds StaticDiscovery for manually managed node addresses. The iroh-dns-server gains configurable pkarr rate limiting (disabled/simple/smart) from a Spacedrive contributor.

### 2024-10-28 — Iroh 1.0 Roadmap: The future will be connected directly
- Link: https://www.iroh.computer/blog/road-to-1-0
- Author: not shown
- Kind: roadmap
- Read: full
- Summary: Commits to iroh 1.0 in the second half of 2025, defining iroh's settled scope as dialing devices by node identifier plus tools for building protocols on direct connections — 'nothing left to remove'. Evidence of readiness cited includes about half a million unique nodes hitting the public network in 30 days (roughly double counting partner relays) and at least 40 projects building on iroh. Milestones are finishing the custom-protocol split, shipping in browsers, publishing a spec of the standards iroh composes, landing one round of network protocol breaks together, and finalizing the Rust API, with blobs aiming for 1.0 around the same time.

### 2024-10-24 — iroh 0.27.0 - Squashing Bugs and Taking Names
- Link: https://www.iroh.computer/blog/iroh-0-27-0-Squashing-Bugs-And-Taking-Names
- Author: not shown
- Kind: release notes
- Read: full
- Summary: A bug-fix release whose main fix makes relay connections reconnect proactively on failure instead of only when the node next tries to send — previously a receiving node whose relay connection died might never reconnect, a bug seen on long-running nodes. It simplifies staging-versus-production relay selection after a bug picked the wrong infrastructure, now governed solely by the IROH_FORCE_STAGING_RELAYS environment variable instead of test features. Adding default discovery services is streamlined to a single .discovery_n0() builder call replacing manual ConcurrentDiscovery assembly.

### 2024-10-21 — Lose your device, but keep your keys (FROST threshold signatures)
- Link: https://www.iroh.computer/blog/frost-threshold-signatures
- Author: not shown
- Kind: security
- Read: full
- Summary: Explores using FROST threshold signatures (via the ZCash Foundation's frost-ed25519 crate) to split an existing Ed25519 private key into shares — e.g. 2-of-3 — whose signatures verify as ordinary Ed25519, so the scheme works with iroh node/document keys, pkarr and the mainline DHT. It builds a prototype with local split/reconstruct/re-split commands and a remote two-round co-signing protocol over iroh, where a signer collects commitments and signature shares from co-signer daemons. The proposed setup puts shares on the user's device, a co-sign server and an offline backup (e.g. USB), so losing or compromising one device neither loses the key nor lets an attacker sign, and recovery is re-splitting into fresh shares.

### 2024-10-01 — iroh 0.26.0 - Say Hello to Your Neighbors
- Link: https://www.iroh.computer/blog/iroh-0-26-0-Say-Hello-to-Your-Neighbors
- Author: not shown
- Kind: release notes
- Read: full
- Summary: First release implementing the narrower definition of iroh: docs are now disabled by default and must be opted into via `enable_docs()` / `enableDocs`. It adds address provenance via `RemoteInfo` / `DirectAddrInfo.sources`, recording how and how long ago each address was learned, which lets applications find locally discovered peers (e.g. via local swarm discovery / mDNS in the last 10 minutes). Breaking changes include the `read_at` signature taking `ReadAtLen` and removal of `disable_docs`.

### 2024-09-23 — Smaller is better
- Link: https://www.iroh.computer/blog/smaller-is-better
- Author: not shown
- Kind: roadmap
- Read: full
- Summary: Announces a strategic reframing: iroh is its networking stack (dial by NodeID plus custom-protocol hooks), and blobs, gossip and docs are custom protocols on top, not the core. The rationale is that no teams use all three protocols the same way, nearly everyone uses iroh-net, and users want a smaller, stable API; the existing stack is not going away but will be labelled production-ready vs experimental. Transition was to complete in bulk by late October 2024 with v0.28.0, with a 1.0 roadmap aimed for 2025.

### 2024-09-17 — iroh 0.25.0 - Custom Protocols For All!
- Link: https://www.iroh.computer/blog/iroh-0-25-0-custom-protocols-for-all
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Adds the ability to write custom protocols in the FFI languages — Python, JavaScript, Kotlin and Swift — not just Rust, with full examples in each language. It also adds binding to a specific interface / `SocketAddr` via new `bind_addr_v4` / `bind_addr_v6` and `bind_random_port` APIs, replacing the old port-only `bind`. Breaking changes include removal of `node::Builder::bind_port`, gossip `update_direct_addresses` removal, and blobs GC consolidation to `Store::gc_run`.

### 2024-09-04 — iroh 0.24.0 - Upgrading to Quinn 11
- Link: https://www.iroh.computer/blog/iroh-0-24-0-quinn-11
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Dominated by the Quinn 0.10 to 0.11 upgrade, worked on since March, which tightens the `AsyncUdpSocket` coupling and changes the API: `Endpoint::accept` now returns `Incoming` (with `accept`/`retry`/`reject`) and `SendStream::finish` splits into `finish` and `stopped`. Performance is mixed: raw iroh-net throughput about 1.05x, 1-to-10 multi-node about 1.45x from per-connection drivers, but 1:1 blobs transfers regressed to as low as 0.6x for unclear reasons. The upgrade also deduplicates rustls to a single 0.23 copy and helps the Wasm plan.

### 2024-08-21 — iroh 0.23.0 - Welcoming Node.js to the family!
- Link: https://www.iroh.computer/blog/iroh-0-23-welcoming-nodejs-to-the-family
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Ships Node.js bindings as `@number0/iroh` on npm, built via napi in iroh-ffi, joining Rust, Python, Swift and Kotlin. It renames `ConnectionInfo`/`connections` to `RemoteInfo`/`remotes` because the address book holds nodes never connected to, moves `net` out from under `node` in both API and CLI (with `status`/`stats`/`shutdown` top-level), and adds a blobs batch API with temporary tags plus a `blob_events` hook for provider events. It also deprecates the V0/V1 blobs flat store, requiring migration before future removal.

### 2024-08-13 — Closing a QUIC Connection
- Link: https://www.iroh.computer/blog/closing-a-quic-connection
- Author: not shown
- Kind: design deep-dive
- Read: full
- Summary: Explains, via Quinn and RFC 9000 stream states, why the sender of the last data cannot safely close a QUIC connection: reaching Data Recvd only means the peer's QUIC stack acked the data, not that its application read it, and a CONNECTION_CLOSE can drop unread stream data. The only reliable pattern is the reverse: the sender waits on `Connection::closed`, the receiver reads the last data and then closes the connection with a custom code, optionally using `Endpoint::wait_idle` before closing the endpoint. Implementations like Quinn are lenient in practice, which hides the bug in testing.

### 2024-08-05 — iroh 0.22.0 - Cleanup on aisle five
- Link: https://www.iroh.computer/blog/iroh-0-22-cleanup-on-aisle-five
- Author: not shown
- Kind: release notes
- Read: full
- Summary: A stability and bug-fix release: the downloader now checks local storage before opening peer connections and fails with "No Providers" if the only provider is yourself, and direct connections to a known public address supplied via `NodeAddr` are now attempted immediately instead of waiting for ping/pong latency. Gossip is restructured so `Gossip::join` returns a `GossipTopic` that quits when all handles drop, removing the `dispatcher` module entirely. Relay types in iroh-net are also reorganised under `relay::server`.

### 2024-07-31 — Async Rust Challenges in Iroh
- Link: https://www.iroh.computer/blog/async-rust-challenges-in-iroh
- Author: not shown
- Kind: design deep-dive
- Read: full
- Summary: A candid catalogue of async Rust pain in a production p2p codebase built on Tokio and Quinn: bridging synchronous redb/file IO via worker threads and a custom LocalPool, Tokio functions that panic based on invisible thread-local runtime/LocalSet state, the impossibility of async cleanup in `Drop`, detached tasks swallowing panics, and buggy `futures` combinators. The team's responses include abandoning `futures` for futures-lite/futures-buffered, replacing flume (not cancel-safe) with async-channel, building their own local pool, and advocating structured concurrency and explicit runtime handles. It also reports suspected tracing span bugs triggered by panics during Drop.

### 2024-07-22 — iroh 0.21.0 - Fix, clean & polish
- Link: https://www.iroh.computer/blog/iroh-0-21-fix-clean-polish
- Author: not shown
- Kind: release notes
- Read: full
- Summary: A polish release whose headline fix replaces `tokio_util::LocalPoolHandle`, whose detached threads could truncate `Drop` IO like file closes and DB commits, with iroh's own gracefully-shutting-down `LocalPool`. Docker images for iroh, iroh-relay and iroh-dns-server are now published to Docker Hub and tested in CI, and CLI/FFI APIs are realigned to match Rust (e.g. plural `blobs`/`docs`/`authors`). Protocol-breaking change: relay clients now dial `/relay` instead of `/derp`, so 0.21 nodes require relays >= 0.20, and automatic storage of known peers in iroh-net is removed.

### 2024-07-11 — Iroh content discovery experiments
- Link: https://www.iroh.computer/blog/iroh-content-discovery
- Author: not shown
- Kind: design deep-dive
- Read: full
- Summary: Explains why iroh has no global content discovery yet — finding providers for a BLAKE3 hash at global scale without unsustainable cost or collapse is hard — and surveys options: context-scoped search, the BitTorrent mainline DHT (which cannot store NodeIDs or NATed providers), other DHTs, trackers, and DNS. The proposed design uses the mainline DHT only to find trackers, with signed announces, strict rate limiting, and validation by probing a random 1 KiB chunk before storing an announce. An experimental implementation lives in iroh-experiments (`iroh-mainline-content-discovery`, tracker, and swarmie), with a pluggable mechanism for iroh-blobs planned.

### 2024-07-09 — iroh 0.20.0 - More ways to connect
- Link: https://www.iroh.computer/blog/iroh-0-20-more-ways-to-connect
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Adds local-network dial-by-NodeID with no internet via `LocalSwarmDiscovery`, an mDNS-like swarm-discovery service enabled by default. Relay clients and servers gain WebSocket support as the first step toward iroh in the browser, gossip gets a much simpler node-level subscribe API, and the Python/Swift/Kotlin FFI becomes async where Rust is async. It also ships Dockerfiles for iroh, iroh-relay and iroh-dns-server, and makes Pkarr publishing emit direct addresses when relay access is disabled.

### 2024-07-04 — Iroh and DAGs
- Link: https://www.iroh.computer/blog/iroh-dag-sync
- Author: not shown
- Kind: design deep-dive
- Read: full
- Summary: Shows how to sync arbitrary DAGs (IPLD/IPFS, git histories, event logs) on top of iroh-blobs, which natively only offers BLAKE3 blobs and hash sequences. The design adds side tables mapping generic IPLD hashes to BLAKE3 hashes (populated only after verification) and caching extracted links, then uses a custom protocol with deterministic traversals: both sides execute the same traversal, the sender streams hashes/data in that order, and the receiver zips it with its locally generated CIDs, validating BLAKE3 incrementally and the IPLD hash on completion — achieving a whole deep DAG sync in a single request/response. A runnable demo in iroh-experiments imports CAR files and supports partial traversals (e.g. excluding raw leaves).

### 2024-07-01 — Iroh & the Web
- Link: https://www.iroh.computer/blog/iroh-and-the-web
- Author: not shown
- Kind: roadmap
- Read: full
- Summary: Lays out the plan to bring iroh to browsers despite browsers having no raw UDP and WebTransport requiring DNS/TLS server endpoints, which blocks true peer dialing. The roadmap is phased: Phase 0 relay connections over WebSockets, Phase 1 compiling iroh-net to WebAssembly by cfg-gating UDP paths so browser nodes can talk to native nodes, Phase 2 bringing blobs, gossip and docs to Wasm, and Phase 3 exploring WebRTC data channels for direct browser peer-to-peer. Relays, which already have public IPs, DNS names and TLS certs and cannot read encrypted traffic, are the bridge that makes the early phases possible.

### 2024-06-27 — iroh 0.19.0 - Make it your own
- Link: https://www.iroh.computer/blog/iroh-0-19-make-it-your-own
- Author: not shown
- Kind: release notes
- Read: full
- Summary: A customisation release: custom protocols can now be registered on a Node via the new `ProtocolBuilder`, docs can be disabled with `Builder::disable_docs()` so the docs engine and protocol are not spawned, and `Iroh::connect_addr` plus `Node::my_rpc_port` allow a client in another process (e.g. Docker) to talk to a node. The release was delayed by a rare high-load bug traced to RPC channels not being cancel-safe, fixed in quic-rpc. The relay server config format also changes, with independent enable flags and bind addresses for relay, STUN and metrics.

### 2024-06-17 — Healing Connections After Network Migration
- Link: https://www.iroh.computer/blog/healing-connections
- Author: not shown
- Kind: design deep-dive
- Read: full
- Summary: Explains how iroh connections survive Wi-Fi switches, hotspots and VPN changes because they are bound to NodeID, not IP address. With a relay, the flow is network-change detection, netcheck (STUN/ICMP probes to learn new public addresses), then an encrypted `CallMeMaybe` disco message via the relay carrying the new addresses, with traffic continuing over the relay until hole-punching restores a direct path. Without a relay, healing still works if one side has a public address, using periodic disco `Ping` messages (checked every 5 seconds) whose source IP the receiver associates with the sender's NodeID; if both sides end up behind NATs the connection closes.

### 2024-06-07 — iroh 0.18.0 - Small, but helpful
- Link: https://www.iroh.computer/blog/iroh-0-18-small-but-helpful
- Author: not shown
- Kind: release notes
- Read: full
- Summary: A small release that changes client subsystem access from fields to methods (`iroh.blobs` to `iroh.blobs()`), deprecating the fields for removal in the next release. Gossip's previously fixed 4 KiB maximum message size becomes configurable via `Gossip::max_message_size`, and netcheck is made much faster and non-blocking by a new contributor. Other API changes touch iroh-blobs tag/drop utilities and collection loading.

### 2024-05-24 — iroh 0.17.0 - Everything Is A Little Better
- Link: https://www.iroh.computer/blog/iroh-0-17-0-everything-is-a-little-better
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Broad quality release: simpler docs APIs with a default author per node and `import_and_subscribe` with `PendingContentReady` events, HTTP/HTTPS proxy support (including auth) from environment variables, and new OpenBSD/FreeBSD/NetBSD support. `MagicEndpoint` is renamed to `Endpoint`, MSRV rises to 1.76, and `iroh-base` now compiles to Wasm as "Phase 0". A new `iroh-perf` tool candidly shows iroh-net throughput below raw Quinn (187 vs 359 MiB/s on macOS; 885 vs 1031 MiB/s on Linux), framing performance work to come.

### 2024-05-20 — Why We Forked Quinn
- Link: https://www.iroh.computer/blog/why-we-forked-quinn
- Author: not shown
- Kind: design deep-dive
- Read: full
- Summary: Explains the Quinn fork: the MagicSocket switches a connection from a relayed path to a direct path under QUIC's feet, so Quinn's congestion controller, unaware of the path change, takes too long to settle to expected latency. Resetting congestion-controller state on path change cut settling time on poor connections from about 20 seconds to about 3 seconds, but required touching Quinn internals with no public API, hence the fork — kept deliberately small to track upstream. Longer term the team hopes QUIC multipath, address discovery and NAT traversal drafts will let MagicSocket functionality move into Quinn itself and be contributed upstream.

### 2024-05-13 — Iroh 0.16 - A Better Client!
- Link: https://www.iroh.computer/blog/iroh-0-16-a-better-client
- Author: not shown
- Kind: release notes
- Read: full
- Summary: An API-focused release making `iroh::client` the clear entry point, and absorbing the Quinn fork: `MagicEndpoint::accept`/`connect` now return iroh's own `Accept`/`Connecting` types instead of Quinn's, with the QUIC API re-exported from iroh so users need only one QUIC dependency. Crates are renamed — `iroh-bytes` to `iroh-blobs` and `iroh-sync` to `iroh-docs` — with many client type renames to match. It also adds `iroh doctor connect`/`accept` echo tests reporting connection type live, and `iroh doctor plot` for terminal plotting of node metrics.

### 2024-05-06 — Dial by NodeID, no address required!
- Link: https://www.iroh.computer/blog/iroh-dns
- Author: not shown
- Kind: design deep-dive
- Read: full
- Summary: Introduces global node discovery so a peer can be dialed with only its NodeID, without a relay URL, direct addresses, or a long, expiring ticket. Nodes publish a signed Pkarr packet containing their NodeID and home relay URL to an iroh-dns server, and others resolve `_iroh.<z32-node-id>.<origin>` TXT records via ordinary DNS (default origin dns.iroh.link, run by n0), with `relay=` and `addr=` attributes defined. The design is intentionally built on old standards (DNS plus Pkarr), is currently federated, and sketches future work toward fully peer-to-peer resolution via the BitTorrent mainline DHT, plus mDNS local discovery and content discovery.

### 2024-04-29 — Iroh 0.15.0 - Shut me down
- Link: https://www.iroh.computer/blog/iroh-0-15-0-shut-me-down
- Author: not shown
- Kind: release notes
- Read: full
- Summary: Marks the move to a regular two-week release cadence and delivers graceful MagicEndpoint shutdown (eliminating the `quinn::endpoint` I/O error on exit), an integrated queued downloader shared by docs and blobs with unified progress and multi-source requests (with `Direct` mode preserving old behaviour), and expanded dial-by-NodeID support. Tickets now default to NodeID-only (`AddrInfoOptions::Id`) and are much shorter, with relay/address inclusion opt-in via `--addr-options`. Prebuilt musl binaries for `iroh`, `iroh-dns-server` and `iroh-relay` are attached to releases for the first time.

### 2024-04-18 — Iroh 0.14.0 - Dial the world
- Link: https://www.iroh.computer/blog/iroh-0-14-0-dial-the-world
- Author: not shown
- Kind: release notes
- Read: full
- Summary: The first release allowing dialing by NodeID alone, via a new `iroh-dns-server` that looks up node information over DNS using the Pkarr format, defaulting to n0's server but self-hostable. A refactored relay handshake drops a full roundtrip but is protocol-breaking: new relays cannot talk to 0.13 nodes, so old relays were to be kept running for at least another 4 weeks, with new relay hostnames `use1-1.relay.iroh.network` (US) and `euw1-1.relay.iroh.network` (EU). It also adds a basic Authors API (create/list/export/import/delete) and upgrades redb to v2.0.

### 2024-02-08 — Iroh global node discovery
- Link: https://www.iroh.computer/blog/iroh-global-node-discovery
- Author: not shown
- Kind: design deep-dive
- Read: full
- Summary: An early experiment (predating the built-in DNS discovery) implementing global node discovery as a pluggable `Discovery` trait using Pkarr over the BitTorrent mainline DHT (BEP44 signed mutable data): nodes publish signed DNS-format records mapping their ed25519 NodeID to direct addresses and DERP relay URL, and resolvers verify signatures so the worst case is a stale record. It is shipped as the separate `iroh-pkarr-node-discovery` crate with a minimal chat example, deliberately not built in because publishing leaks rough location and private network details, many nodes do not want to be publicly reachable, and it would add dependencies — a critique that also weighs centralised (Cloudflare Workers/KV cost) and plain DNS alternatives.

### 2023-02-17 — A new direction for iroh
- Link: https://www.iroh.computer/blog/a-new-direction-for-iroh
- Author: not shown
- Kind: roadmap
- Read: full
- Summary: The pivot post: after 9 months and 5 releases building a Rust IPFS implementation interoperable with kubo ("Plan A"), measurements showed it far from theoretical limits — needing at least 2,000 simultaneous connections as a gateway, ~1,000 messages to retrieve a single 256 KB block, seconds for DHT queries, and unreliable peer connections. The team announces breaking changes under "Plan B": define an IPFS-like system with quantified targets (order of 200 peers, less than 1 message per block, 95%+ reliability), build to those numbers, then bridge back to IPFS, while renaming the old implementation to Beetle in maintenance mode. An update clarifies iroh remains an IPFS implementation (still using CIDs) and was not breaking from all IPFS specs.

---

## Part 3 — Talks & notable third-party coverage

The full talk census (conference talks, n0 team appearances, announcements) lives in **`talks-and-announcements.md`** in this folder, and adoption/project coverage in **`real-projects-and-community.md`**. Highlights and the third-party pieces found by this pass:

**From the blog itself (verified — linked from iroh.computer posts):**
- **"BLAKE3 and Bao deep dive"** (YouTube, linked from *Iroh global content discovery*, 2026-09-30) — the canonical visual explanation of BLAKE3 verified streaming, outboard encoding, and range requests as used by iroh-blobs.
- The **message-framing tutorial** (2025-08-12) is adapted from an iroh YouTube video — the blog's beginner on-ramp mirrors their video content.

**Third-party coverage (researched 2026-11-23; engagement signals as observed that day):**
- **Hacker News — iroh 1.0 launch thread:** 847 points / 267 comments. Dominant mental model in the discussion: "Tailscale at the application layer"; recurring questions about hosted-relay pricing and lock-in, and positioning confusion vs libp2p/Tailscale.
- **Hacker News — Mesh LLM:** hit the front page (179 points) — the distributed-inference-on-iroh story travels well outside the Rust/P2P niche.
- **mickeyzzc — iroh vs libp2p deep dive:** a technical comparison post; useful as the most substantive independent architecture comparison found.
- **skill-issue.dev — vanta-node / iroh-gossip in production:** an adoption write-up running iroh-gossip in a real deployment.
- **gordo.design — Tape Music Suite's Tailscale→Iroh migration:** practitioner migration story; notable friction point — the author had to build a React Native bridge because no official RN package exists.
- **tismtek — iroh explainer/limits post:** an independent explainer that also catalogues limitations.
- **stackgems — iroh review:** relays-focused review; calls public relays dev/test-grade and quotes hosted pricing (Pro from $19/mo, dedicated from $199/mo/region) — **corroborated exactly** by the official shared-relays post in Part 2 (2026-09-08).
- **Fedimint 0.12 release notes:** guardian P2P and the client API migrated to Iroh 1.0, with dual-stack support for older federations; their Iroh 1.0 upgrade was *not* wire-compatible for guardians, requiring coordinated upgrade windows — a real-world echo of the wire-break hazards flagged at the top of this file.
- **Tangled — TRANSPORT-RESEARCH (ATProto PDS evaluation):** evaluated iroh as transport and found its BLAKE3 content addressing incompatible with ATProto CIDs; transport-only use remains possible. A useful boundary case: iroh's content layer does not transplant onto CID-based systems.

**Verification labels:** blog-linked items and the pricing corroboration are verified against primary sources in this pass. HN point/comment counts and the third-party summaries are as observed during the 2026-11-23 research pass (listing-level for the individual blog posts — each was located and characterised, not line-by-line audited). The Iroh Discord is login-gated and GitHub Discussions are public; community-signal detail is in `real-projects-and-community.md`.

---

*Catalogue assembled from four parallel reader passes over the blog index plus a gap-fill pass, 2026-11-23. Chunk working files: `blog-chunks/chunk-A.md` … `chunk-D.md`.*


## Change log

- r1 (2026-11-24): Migrated from hatch-personal workspace/iroh-investigation/ under #13877; content unchanged.
