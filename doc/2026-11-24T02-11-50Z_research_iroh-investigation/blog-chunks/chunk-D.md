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
