# Peer Discovery via iroh-gossip + distributed-topic-tracker

**Status:** Complete (Phases 0-6, 8 done; Phase 7 is future work)
**Created:** 2026-03-21
**Updated:** 2026-03-22
**Goal:** Enable all online `id` servers to discover and list each other using iroh-gossip with automatic DHT-based bootstrapping via `distributed-topic-tracker`, without interfering with the existing `META_ALPN` protocol used for blob operations.

---

## Context

Currently, `id` servers have no mechanism to discover each other. Clients must already know a server's node ID to connect. This plan adds gossip-based peer discovery where servers advertise their presence (node ID, name, blob count) on a shared gossip topic, plus an RPC mechanism for querying known peers and a depth-based crawler for recursive peer discovery.

### Key Dependencies (already in Cargo.toml, unused in source)

- `iroh-gossip = { version = "0.96", features = ["net"] }` -- gossip pub/sub transport
- `distributed-topic-tracker` (git dep) -- automatic peer bootstrapping via BitTorrent mainline DHT, wraps iroh-gossip with zero-config discovery, encrypted/signed DHT records, and partition healing

### Design Decisions

1. **`distributed-topic-tracker` for bootstrapping** -- provides automatic peer discovery via BitTorrent DHT. Handles the hard problem of finding initial peers without any manual configuration. Wraps iroh-gossip with additional features: encrypted DHT records, Ed25519-signed announcements, bubble merge (partition healing), and split-brain detection.
2. **Gossip uses its own built-in ALPN** -- no new custom ALPN needed. Registered with the Router alongside meta and blobs.
3. **`ListPeers` added to `MetaRequest`/`MetaResponse`** -- pragmatic extension to the meta protocol for querying known peers via RPC. **Must be appended as the last variant** in both enums to preserve postcard wire compatibility (postcard uses positional enum discriminants).
4. **Depth-based crawling** -- the `peers` command supports `--depth N` to recursively query peers of peers, building a broader view of the network. Includes safeguards: `--max-peers` cap and per-crawl timeout.
5. **Global default topic with override** -- a well-known default topic (`id-peer-discovery-v1`) and secret allows all `id` servers to find each other automatically. Users can override with `--topic` and `--topic-secret` for private networks.
6. **Layered bootstrapping** -- primary: DHT auto-discovery; supplementary: `--bootstrap <node_ids>` for manual peer specification (useful for restricted networks, development, and testing).
7. **Server vs. caller modes** -- `--discover` flag tells the server to update its own peer list; default just returns results to the caller.

### Security Model

**Addressed by `distributed-topic-tracker`:**
- **Access control**: The shared `initial_secret` (topic secret) acts as a bearer token -- only nodes with the secret can decrypt DHT records and join the discovery topic
- **Signed records**: DHT records are Ed25519-signed, preventing spoofing of bootstrap announcements
- **Encrypted transport**: HPKE encryption on DHT records with per-minute key rotation

**Known limitations (document, don't block on):**
- The default global secret is public knowledge -- anyone can join the default topic
- `PeerAnnouncement` messages over gossip are not individually authenticated (a node could theoretically claim any `node_id` in a broadcast, though the gossip layer attributes messages to the sending peer)
- No rate limiting on gossip messages within the topic beyond what iroh-gossip provides
- Private topics are only as secure as the shared secret distribution

---

## Phase 0: Design Documentation

**Status:** Complete

Per project conventions ("document first, then implement"), create the design document before any code changes.

### Tasks

- [ ] Create `pkgs/id/docs/<UTC_RFC_DATETIME>_feature_peer-discovery/` folder
- [ ] Write design document covering:
  - Feature overview and motivation
  - Architecture: `distributed-topic-tracker` + iroh-gossip + `PeerDiscovery` application layer
  - Topic identity model (global default + override)
  - `PeerAnnouncement` wire format (postcard)
  - Bootstrapping hierarchy (DHT auto-discovery → manual `--bootstrap` → future Pkarr/DNS)
  - Depth-based crawling algorithm with safeguards
  - Security model and known limitations
  - CLI flags and usage examples
  - Web page design
  - Link back to this plan file
- [ ] Add "References" section linking to this plan

### Files

| File | Action |
|------|--------|
| `pkgs/id/docs/..._feature_peer-discovery/` | **Create** |

---

## Phase 1: Core Types & Discovery Module

**Status:** Complete
**New file:** `pkgs/id/src/discovery.rs`

### Tasks

- [ ] Define discovery constants:
  - `DEFAULT_TOPIC: &str = "id-peer-discovery-v1"` (used with `TopicId::new()`)
  - `DEFAULT_TOPIC_SECRET: &[u8] = b"id-public-discovery-v1"` (default shared secret for public network)
  - `ANNOUNCE_INTERVAL: Duration = Duration::from_secs(30)` (broadcast frequency)
  - `STALE_THRESHOLD: Duration = Duration::from_secs(120)` (4 missed heartbeats)
  - `STALE_CHECK_INTERVAL: Duration = Duration::from_secs(60)` (cleanup frequency)
- [ ] Define `PeerAnnouncement` struct (Serialize/Deserialize via postcard):
  - `node_id: NodeId`
  - `name: Option<String>`
  - `blob_count: u64`
  - `timestamp_secs: u64`
- [ ] Define `PeerInfo` struct (in-memory, not serialized):
  - `announcement: PeerAnnouncement`
  - `last_seen: Instant`
- [ ] Implement `PeerDiscovery` struct (`Arc<RwLock<HashMap<NodeId, PeerInfo>>>`):
  - `new() -> Self`
  - `update(announcement: PeerAnnouncement)` -- insert/update peer
  - `peers() -> Vec<PeerInfo>` -- return non-stale peers (>`STALE_THRESHOLD` = stale)
  - `remove_stale(max_age: Duration)` -- prune expired entries
  - `count() -> usize`
- [ ] Add `pub mod discovery;` to `pkgs/id/src/lib.rs`
- [ ] Add re-exports for `PeerAnnouncement`, `PeerInfo`, `PeerDiscovery`
- [ ] Unit tests for `PeerAnnouncement` serialization roundtrip
- [ ] Unit tests for `PeerDiscovery` (update, stale removal, count)

### Files

| File | Action |
|------|--------|
| `pkgs/id/src/discovery.rs` | **Create** |
| `pkgs/id/src/lib.rs` | Modify (add module + re-exports) |

---

## Phase 2: Protocol Extension -- ListPeers RPC

**Status:** Complete
**Modifies:** `pkgs/id/src/protocol.rs`

### Tasks

- [ ] **Append** `ListPeers` variant to `MetaRequest` as the **last variant** (critical: postcard uses positional discriminants; inserting mid-enum breaks wire compat)
- [ ] **Append** `ListPeers { peers: Vec<PeerAnnouncement> }` variant to `MetaResponse` as the **last variant**
- [ ] Update `MetaProtocol` struct to hold optional `PeerDiscovery`:
  - Change `MetaProtocol::new(store, peer_discovery)` signature
  - Store `Option<PeerDiscovery>` field
- [ ] Handle `MetaRequest::ListPeers` in the `accept()` method:
  - If `PeerDiscovery` is available, return current peers
  - If not, return empty list
- [ ] Update all call sites of `MetaProtocol::new()` (serve.rs)
- [ ] Unit tests for `ListPeers` request/response serialization

### Files

| File | Action |
|------|--------|
| `pkgs/id/src/protocol.rs` | Modify |

### Notes

- `MetaProtocol::new()` signature change affects `serve.rs` (Phase 3 handles this)
- The `PeerDiscovery` is `Option` so non-serve contexts (tests, etc.) don't need it

---

## Phase 3: Gossip Integration in Server

**Status:** Complete
**Modifies:** `pkgs/id/src/commands/serve.rs`, `pkgs/id/src/cli.rs`

### Tasks

- [ ] Add CLI flags to `Serve` command:
  - `--bootstrap <node_ids>` -- comma-separated node IDs for manual supplementary bootstrapping
  - `--topic <name>` -- gossip topic name (default: `DEFAULT_TOPIC`)
  - `--topic-secret <secret>` -- shared secret for topic access control (default: `DEFAULT_TOPIC_SECRET`)
- [ ] In `cmd_serve()`:
  1. Create `PeerDiscovery::new()`
  2. Create `Gossip::builder().spawn(endpoint.clone()).await?`
  3. Register gossip with Router: `.accept(iroh_gossip::net::GOSSIP_ALPN, gossip.clone())`
  4. Pass `PeerDiscovery` to `MetaProtocol::new(&store_handle, Some(peer_discovery.clone()))`
  5. Create `RecordPublisher` with topic and secret (from CLI flags or defaults)
  6. Call `gossip.subscribe_and_join_with_auto_discovery_no_wait(record_publisher)` (non-blocking variant -- don't block serve startup on DHT)
  7. If `--bootstrap` provided, additionally join those peers via `sender.join_peers()`
  8. Spawn background task:
     - Every `ANNOUNCE_INTERVAL` (~30s): broadcast `PeerAnnouncement` with current blob count (from store tags count)
     - Listen for incoming gossip events, deserialize `PeerAnnouncement`, call `peer_discovery.update()`
     - Every `STALE_CHECK_INTERVAL` (~60s): call `peer_discovery.remove_stale(STALE_THRESHOLD)`
  9. Print `peers: gossip enabled (topic: <topic_name>)` on startup
- [ ] Pass `PeerDiscovery` into web `AppState` if web feature is enabled
- [ ] Gracefully shut down gossip on Ctrl+C

### Files

| File | Action |
|------|--------|
| `pkgs/id/src/commands/serve.rs` | Modify (major) |
| `pkgs/id/src/cli.rs` | Modify (add flags to `Serve`) |

### API Pattern

```rust
use distributed_topic_tracker::{AutoDiscoveryGossip, RecordPublisher, TopicId};
use iroh_gossip::net::Gossip;

let gossip = Gossip::builder().spawn(endpoint.clone()).await?;

// Register with router
let router = Router::builder(endpoint)
    .accept(META_ALPN, meta)
    .accept(BLOBS_ALPN, blobs)
    .accept(iroh_gossip::net::GOSSIP_ALPN, gossip.clone())
    .spawn();

// Setup auto-discovery
let topic_id = TopicId::new(topic_name);
let record_publisher = RecordPublisher::new(
    topic_id,
    signing_key.verifying_key(),
    signing_key,
    None,  // default secret rotation
    topic_secret,
);

// Non-blocking join -- DHT discovery runs in background
let topic = gossip
    .subscribe_and_join_with_auto_discovery_no_wait(record_publisher)
    .await?;
let (sender, mut receiver) = topic.split().await?;

// Optionally join manual bootstrap peers
if !bootstrap_nodes.is_empty() {
    sender.join_peers(bootstrap_nodes, None).await?;
}
```

### Notes

- Use `_no_wait` variant so the server starts accepting connections immediately while DHT bootstrapping proceeds in the background
- `distributed-topic-tracker` automatically handles: DHT publishing, bubble merge, message overlap merge, and periodic record refresh
- The server's broadcast loop (PeerAnnouncement) is application-level, on top of the transport

---

## Phase 4: `peers` CLI Command

**Status:** Complete
**New file:** `pkgs/id/src/commands/peers.rs`

### Tasks

- [ ] Add `Peers` variant to `Command` enum in `cli.rs`:
  ```
  Peers {
      --gossip         Use gossip only (join topic, collect announcements)
      --rpc            Use RPC only (query server's ListPeers)
      --depth N        Recursive depth (default 1, -1 = unlimited)
      --max-peers N    Hard cap on total peers discovered (default 1000)
      --timeout N      Per-crawl timeout in seconds (default 30)
      --discover       Tell server to update its peer list
      --bootstrap      Comma-separated seed node IDs
      --topic          Gossip topic name (default: DEFAULT_TOPIC)
      --topic-secret   Shared secret (default: DEFAULT_TOPIC_SECRET)
      --no-relay       Disable relay servers
      node: Option     Optional specific node to query
  }
  ```
- [ ] Create `cmd_peers()` function:
  1. If `--rpc` (or default with serve running): send `MetaRequest::ListPeers` to local serve
  2. If `--gossip`: create endpoint, join gossip topic via auto-discovery, listen for N seconds, collect announcements
  3. Default: try RPC first, fall back to gossip
  4. **Depth crawling (with safeguards):**
     - Depth 0: return initial results (from gossip and/or provided bootstrap peers)
     - Depth 1+: for each discovered peer, connect and send `MetaRequest::ListPeers`, merge results
     - Depth -1: keep crawling until no new peers found **or** `--max-peers` cap hit **or** `--timeout` exceeded
     - Track visited nodes to avoid cycles
     - Log warning when max-peers cap is hit
  5. If `--discover`: also send discovered peers back to the local server (future: via a new RPC or gossip broadcast)
  6. Print results: `<node_id>\t<name>\t<blob_count>\t<last_seen>`
- [ ] Add to `commands/mod.rs` re-exports
- [ ] Add to `main.rs` command dispatch
- [ ] CLI parsing tests

### Files

| File | Action |
|------|--------|
| `pkgs/id/src/commands/peers.rs` | **Create** |
| `pkgs/id/src/commands/mod.rs` | Modify |
| `pkgs/id/src/cli.rs` | Modify |
| `pkgs/id/src/main.rs` | Modify (command dispatch) |

---

## Phase 5: REPL Integration

**Status:** Complete

### Tasks

- [ ] Add `peers` command to `execute_repl_command()` in `runner.rs`:
  - In connected mode (serve running): send `MetaRequest::ListPeers` and display
  - In local mode: print "no serve running, peers unavailable"
- [ ] Add `peers` to `print_help()`
- [ ] Support `peers @NODE_ID` to query a specific remote node's peer list

### Files

| File | Action |
|------|--------|
| `pkgs/id/src/repl/runner.rs` | Modify |

---

## Phase 6: Web Page (feature-gated under `web`)

**Status:** Complete

### Tasks

- [ ] Add `PeerDiscovery` (or `Option<PeerDiscovery>`) to `AppState` in `web/mod.rs`
- [ ] Add `/peers` route in `web/routes.rs`:
  - Handler queries `PeerDiscovery` state
  - Returns HTML table of peers (node ID, name, blob count, last seen)
  - Supports HTMX partial rendering
- [ ] Add `render_peers_page()` in `web/templates.rs`:
  - Terminal-styled table matching existing theme
  - Each peer row shows: node ID (truncated), name, blob count, relative time since last seen
  - Auto-refresh via HTMX polling (every ~10s)
- [ ] Wire `PeerDiscovery` from `cmd_serve()` into `AppState` construction
- [ ] Add navigation link to peers page in existing templates

### Files

| File | Action |
|------|--------|
| `pkgs/id/src/web/mod.rs` | Modify |
| `pkgs/id/src/web/routes.rs` | Modify |
| `pkgs/id/src/web/templates.rs` | Modify |
| `pkgs/id/src/commands/serve.rs` | Modify (pass PeerDiscovery to web) |

---

## Phase 7: Manual Bootstrap Fallback & Future Enhancements

**Status:** Not started (future work, not needed for MVP)

### Covered by Phase 3

The `--bootstrap <node_ids>` flag in Phase 3 provides manual peer specification as a supplementary bootstrap mechanism. This is useful when:
- DHT traffic is blocked by firewalls
- Running in restricted/isolated networks
- Local development and testing
- The BitTorrent DHT is slow or unreachable

### Bootstrap Hierarchy

1. **Primary**: `distributed-topic-tracker` auto-discovery via BitTorrent mainline DHT (automatic, zero-config)
2. **Supplementary**: `--bootstrap <node_ids>` for manual peer specification (combined with DHT results)
3. **Future stretch goal**: Pkarr/DNS-based discovery (publish known node IDs to DNS for another discovery channel)

### Practical Workflow

With DHT auto-discovery:
1. Server A starts -- publishes to DHT, waits for peers
2. Server B starts -- finds Server A via DHT, joins automatically
3. All subsequent servers discover the network via DHT

Without DHT (restricted network):
1. Server A starts with no bootstrap -- gossip topic is empty
2. Server B starts with `--bootstrap <A's node ID>` -- discovers A
3. Server C starts with `--bootstrap <A's or B's node ID>` -- discovers both

### Future: Pkarr/DNS Bootstrap

- Publish a well-known node ID to DNS (e.g., `_id-bootstrap.example.com`)
- On startup, resolve DNS to find initial bootstrap peers
- Acts as a third discovery channel alongside DHT and manual bootstrap
- Not needed for MVP -- DHT + manual bootstrap covers all practical scenarios

---

## Phase 8: Testing & Polish

**Status:** Complete (cargo check, cargo test, cargo fmt all pass; cargo clippy blocked by pre-existing E0514 environment issue)

### Tasks

- [ ] Run `just check` and fix any issues
- [ ] Verify all new tests pass
- [ ] Update module-level docstrings in modified files
- [ ] Update design document with any implementation-time changes
- [ ] Document security limitations in module-level docs
- [ ] Test scenarios:
  - Two servers discovering each other via DHT (if testable without real DHT)
  - Manual bootstrap between two local servers
  - `peers` CLI command output formatting
  - `peers --depth` crawling with cycle detection
  - `peers --max-peers` cap enforcement
  - REPL `peers` command
  - Web `/peers` page (if web feature enabled)
  - Stale peer removal
  - Private topic with custom secret

---

## Implementation Order

1. **Phase 0** -- Design documentation (doc-first per project conventions)
2. **Phase 1** -- Core types (discovery.rs) -- no dependencies
3. **Phase 2** -- Protocol extension (ListPeers) -- depends on Phase 1 types
4. **Phase 3** -- Server gossip integration -- depends on Phase 1 + 2
5. **Phase 4** -- CLI `peers` command -- depends on Phase 2 (uses ListPeers RPC)
6. **Phase 5** -- REPL integration -- depends on Phase 2
7. **Phase 6** -- Web page -- depends on Phase 1 + 3
8. **Phase 7** -- Bootstrap fallback (mostly done in Phase 3, this is documentation/future work)
9. **Phase 8** -- Testing + polish

Phases 4, 5, and 6 are independent of each other and can be done in any order after Phase 3.

---

## Key Risk: API Version Compatibility

### iroh-gossip v0.96

The `Gossip::builder().spawn()` API needs verification against v0.96 docs. The plan assumes:
- `Gossip::builder().spawn(endpoint).await?` creates the gossip instance
- Events include `Event::Received`, `Event::NeighborUp`, `Event::NeighborDown`
- Broadcasting is done via the `GossipSender` handle

### distributed-topic-tracker

The crate is pinned via git dependency. The `pkgs/dht/` prototype uses v0.2.5 with iroh-gossip v0.95, while `pkgs/id/` uses iroh-gossip v0.96. **Verify that `distributed-topic-tracker` is compatible with iroh-gossip v0.96** before starting Phase 3. If not, the crate may need updating or the integration approach adjusted.

Key API assumed:
- `AutoDiscoveryGossip` trait on `Gossip`
- `subscribe_and_join_with_auto_discovery_no_wait(RecordPublisher)` returns `Topic`
- `Topic::split()` returns `(GossipSender, GossipReceiver)`
- `GossipSender::broadcast(Vec<u8>)` for sending
- `GossipReceiver::next()` for receiving events

---

## Appendix: File Change Summary

| File | Phase | Action |
|------|-------|--------|
| `pkgs/id/docs/..._feature_peer-discovery/` | 0 | **Create** |
| `pkgs/id/src/discovery.rs` | 1 | **Create** |
| `pkgs/id/src/lib.rs` | 1 | Modify |
| `pkgs/id/src/protocol.rs` | 2 | Modify |
| `pkgs/id/src/commands/serve.rs` | 3, 6 | Modify (major) |
| `pkgs/id/src/cli.rs` | 3, 4 | Modify |
| `pkgs/id/src/commands/peers.rs` | 4 | **Create** |
| `pkgs/id/src/commands/mod.rs` | 4 | Modify |
| `pkgs/id/src/main.rs` | 4 | Modify |
| `pkgs/id/src/repl/runner.rs` | 5 | Modify |
| `pkgs/id/src/web/mod.rs` | 6 | Modify |
| `pkgs/id/src/web/routes.rs` | 6 | Modify |
| `pkgs/id/src/web/templates.rs` | 6 | Modify |
