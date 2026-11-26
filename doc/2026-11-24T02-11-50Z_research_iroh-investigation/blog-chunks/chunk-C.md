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
