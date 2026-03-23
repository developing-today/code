# Peer Discovery via iroh-gossip + distributed-topic-tracker

See [original plan](../../../.opencode/plans/2026-03-21_peer-discovery.md)

## Overview

Peer discovery enables `id` servers to find and list each other automatically, without requiring users to manually exchange 64-character hex node IDs. This is the foundational feature for building a usable peer-to-peer network on top of `id`.

### Problem

Currently, connecting two `id` servers requires:
1. Server A starts and prints its node ID
2. The operator manually copies that node ID
3. Server B uses the node ID to connect

This is workable for 2-3 servers but doesn't scale. There is no mechanism for a server to discover other servers on the network.

### Solution

Add gossip-based peer discovery using two layers:

- **Transport layer**: `distributed-topic-tracker` (wraps `iroh-gossip`) provides automatic peer bootstrapping via the BitTorrent mainline DHT, with encrypted/signed records and partition healing.
- **Application layer**: `PeerAnnouncement` messages broadcast over the gossip channel carry metadata (node ID, name, blob count) that populate an in-memory `PeerDiscovery` peer table.

## Architecture

```text
+-------------------------------------------------------------------+
|                         id Server                                  |
+-------------------------------------------------------------------+
|  PeerDiscovery (in-memory peer table)                              |
|    - Updated by incoming gossip PeerAnnouncement messages          |
|    - Queried by MetaProtocol (ListPeers RPC)                       |
|    - Queried by Web UI (/peers route)                              |
+-------------------------------------------------------------------+
|  Application Gossip Layer                                          |
|    - Broadcasts PeerAnnouncement every ~30s                        |
|    - Listens for PeerAnnouncement from other servers               |
|    - Prunes stale peers (probes via ListPeers RPC before removal)  |
+-------------------------------------------------------------------+
|  distributed-topic-tracker                                         |
|    - Manages gossip topic subscription                             |
|    - Auto-discovers peers via BitTorrent mainline DHT              |
|    - Handles bubble merge (partition healing)                      |
|    - Handles message-overlap merge (split-brain detection)         |
|    - Publishes/retrieves encrypted, signed DHT records             |
+-------------------------------------------------------------------+
|  iroh-gossip (transport)                                           |
|    - QUIC-based gossip pub/sub                                     |
|    - Registered with Iroh Router via GOSSIP_ALPN                   |
+-------------------------------------------------------------------+
|  Iroh Endpoint (QUIC + relay + NAT traversal)                      |
+-------------------------------------------------------------------+
```

## Topic Identity Model

### Global Default

All `id` servers share a default discovery topic:
- **Topic**: `id-peer-discovery-v1` (hashed via SHA-512 by `TopicId::new()`)
- **Secret**: `id-public-discovery-v1` (shared secret for DHT record encryption)

This allows any `id` server to find any other server on the public network with zero configuration.

### Private Networks

Users can create isolated discovery networks with:
```bash
id serve --topic my-private-network --topic-secret my-secret-key
```

Only servers using the same topic and secret can discover each other. The secret controls access to DHT records -- without it, a node cannot decrypt bootstrap information.

## Wire Format

### PeerAnnouncement (postcard-serialized, broadcast over gossip)

```rust
struct PeerAnnouncement {
    node_id: NodeId,         // 32-byte Ed25519 public key
    name: Option<String>,    // Human-readable server name
    blob_count: u64,         // Number of blobs in store
    timestamp_secs: u64,     // Unix timestamp of announcement
}
```

Serialized with [postcard](https://docs.rs/postcard) for compact binary encoding. Typical size: ~50-100 bytes depending on name length.

### ListPeers RPC (over MetaProtocol)

Added as the **last variant** in `MetaRequest` and `MetaResponse` enums to preserve postcard wire compatibility with existing clients:

```rust
// Request
MetaRequest::ListPeers

// Response  
MetaResponse::ListPeers { peers: Vec<PeerAnnouncement> }
```

## Bootstrapping Hierarchy

Three discovery mechanisms, in priority order:

### 1. DHT Auto-Discovery (Primary)

`distributed-topic-tracker` queries the BitTorrent mainline DHT for mutable records associated with the topic. Records are encrypted with the topic secret and signed with Ed25519.

- **Fully automatic** -- no configuration needed
- **Takes 5-30 seconds** for initial DHT lookup
- Server uses `subscribe_and_join_with_auto_discovery_no_wait()` so it starts serving immediately while DHT bootstrapping proceeds in the background

### 2. Manual Bootstrap (Supplementary)

```bash
id serve --bootstrap abc123...,def456...
```

Manually specified peer node IDs are joined in addition to DHT-discovered peers. Useful when:
- DHT traffic is blocked by firewalls
- Running in restricted/isolated networks
- Local development and testing

### 3. Pkarr/DNS (Future)

Not implemented in MVP. A potential future channel where well-known bootstrap node IDs are published to DNS records.

## Depth-Based Peer Crawling

The `peers` CLI command supports recursive discovery:

```bash
# Direct peers only
id peers --depth 0

# Peers of peers (default)
id peers --depth 1

# Deep crawl with safeguards
id peers --depth -1 --max-peers 500 --timeout 15
```

### Algorithm

1. Start with initial peer set (from gossip and/or RPC)
2. For each undiscovered peer, send `MetaRequest::ListPeers` and merge results
3. Track visited nodes to avoid cycles
4. Stop when: depth limit reached, `--max-peers` cap hit, or `--timeout` exceeded
5. For `--depth -1` (unlimited): crawl until no new peers found, subject to `--max-peers` (default 1000) and `--timeout` (default 30s) safeguards

## Timing Parameters

| Parameter | Value | Purpose |
|-----------|-------|---------|
| `ANNOUNCE_INTERVAL` | 30s | How often servers broadcast PeerAnnouncement |
| `STALE_THRESHOLD` | 120s | Time since last announcement before a peer is considered stale |
| `STALE_CHECK_INTERVAL` | 60s | How often the server checks for stale peers |
| `PROBE_TIMEOUT` | 10s | Timeout for each individual stale peer probe |

A peer is stale after missing ~4 consecutive announcements. Before removal, stale peers are probed via a `ListPeers` RPC call. If the peer responds within the probe timeout, its `last_seen` timestamp is refreshed and it stays in the table. If the peer is unreachable or times out, it is removed. This prevents premature eviction of peers that are alive but experiencing temporary gossip delivery issues.

## Security Model

### Provided by distributed-topic-tracker

- **Access control**: The shared topic secret is used to derive HPKE encryption keys for DHT records. Only nodes with the correct secret can decrypt bootstrap information.
- **Signed records**: DHT records are Ed25519-signed, preventing spoofing of bootstrap announcements.
- **Key rotation**: Encryption keys rotate every minute via a pluggable KDF, limiting the window of exposure if a key is compromised.

### Known Limitations

- **Default secret is public**: The default global topic uses a well-known secret (`id-public-discovery-v1`). Anyone can join the default discovery network. This is by design for the public network.
- **Gossip messages are not individually authenticated**: `PeerAnnouncement` messages broadcast over gossip are attributed to the sender by the gossip layer, but the `node_id` field in the announcement is not cryptographically verified against the sender. A malicious node could theoretically claim a different node ID.
- **No rate limiting**: Beyond what iroh-gossip provides natively, there is no application-level rate limiting on gossip messages.
- **Private topics rely on secret secrecy**: The security of private discovery networks depends entirely on the shared secret not being leaked. There is no revocation mechanism.

## CLI Interface

### Server flags
```bash
id serve [--bootstrap <node_ids>] [--topic <name>] [--topic-secret <secret>]
```

### Peers command
```bash
id peers [--gossip] [--rpc] [--depth N] [--max-peers N] [--timeout N]
         [--bootstrap <node_ids>] [--topic <name>] [--topic-secret <secret>]
         [--no-relay] [node]
```

### REPL
```
> peers              # List known peers (via RPC to local serve)
> peers @NODE_ID     # Query a specific remote node's peer list
```

## Files Changed

| File | Change |
|------|--------|
| `src/discovery.rs` | **New** -- core types (PeerAnnouncement, PeerInfo, PeerDiscovery) |
| `src/lib.rs` | Add module + re-exports |
| `src/protocol.rs` | Add ListPeers to MetaRequest/MetaResponse |
| `src/cli.rs` | Add Peers command, add flags to Serve |
| `src/commands/serve.rs` | Gossip integration, background tasks |
| `src/commands/peers.rs` | **New** -- peers command implementation |
| `src/commands/mod.rs` | Add peers module |
| `src/main.rs` | Add Peers command dispatch |
| `src/repl/runner.rs` | Add peers REPL command |
| `src/web/mod.rs` | Add PeerDiscovery to AppState |
| `src/web/routes.rs` | Add /peers route |
| `src/web/templates.rs` | Add peers page template |

## Endpoint Lifecycle (iroh 0.97)

iroh 0.97 requires explicit `endpoint.close().await` before dropping an endpoint. Dropping without closing produces `ERROR iroh::socket: Endpoint dropped without calling Endpoint::close. Aborting ungracefully.`

### Pattern for one-time functions

Functions that create a temporary endpoint, do work, and return must close the endpoint on both success and error paths. The pattern used throughout the codebase:

```rust
let endpoint = Endpoint::builder(presets::N0).secret_key(key).bind().await?;
let result = async {
    // ... do work with endpoint ...
    Ok(value)
}.await;
endpoint.close().await;
result
```

### Endpoint ownership in peers code

| Function | Endpoint Lifecycle |
|----------|-------------------|
| `query_local_peers()` | Creates endpoint via `create_local_client_endpoint()`, closes after RPC |
| `query_remote_peers()` | Creates endpoint, closes after RPC |
| `discover_via_gossip()` | Creates endpoint, closes after gossip timeout |
| `crawl_peers()` | Delegates to `query_remote_peers()` (each call creates/closes its own endpoint) |
| `ReplContext::peers()` | Uses shared REPL endpoint (closed in `shutdown()`) |
| `ReplContext::peers_on_node()` | Uses shared REPL endpoint (closed in `shutdown()`) |
| `cmd_serve()` | Endpoint owned by Router (Router handles lifecycle) |

## References

- [Original plan](../../../.opencode/plans/2026-03-21_peer-discovery.md)
- [distributed-topic-tracker](https://github.com/rustonbsd/distributed-topic-tracker) -- DHT-based auto-discovery crate
- [iroh-gossip](https://docs.rs/iroh-gossip) -- gossip pub/sub transport
- [pkgs/dht/](../../) -- reference prototype using distributed-topic-tracker
