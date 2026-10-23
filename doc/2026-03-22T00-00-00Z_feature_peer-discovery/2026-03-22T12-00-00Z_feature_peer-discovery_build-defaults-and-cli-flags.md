# Peer Discovery: Build-Time Defaults and CLI Flags

Post-rollout update to [initial design](2026-03-22T00-00-00Z_feature_peer-discovery.md).
See [original plan](../../../.opencode/plans/2026-03-21_peer-discovery.md).

## Summary

This update adds a three-layer configuration cascade for peer discovery and a `--no-gossip` kill switch. The goal is to let operators customise discovery at build time (for custom binaries with baked-in bootstrap nodes) **and** at runtime (via CLI flags), while preserving sensible defaults out of the box.

## Configuration Cascade

Resolution flows top-to-bottom; later layers override earlier ones:

```text
1. Hardcoded constants          ← DEFAULT_TOPIC, DEFAULT_TOPIC_SECRET
   ↓
2. Build-time defaults.conf     ← embedded via include_str!(), parsed once (LazyLock)
   ↓
3. CLI flags                    ← --bootstrap, --topic, --topic-secret
   ↓
Effective config (ResolvedConfig)
```

Three negative flags modify which layers participate:

| Flag                     | Effect                                                                                                                                                                           |
| ------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--replace-defaults`     | Use **only** `defaults.conf` values for topic/secret, skip hardcoded constants. If `defaults.conf` has no entry for a field, that field is empty.                                |
| `--no-default-bootstrap` | Drop all bootstrap nodes from layers 1-2. Only CLI `--bootstrap` nodes survive.                                                                                                  |
| `--no-default-topic`     | Drop topic and secret from layers 1-2. Falls back to the hardcoded constants (`DEFAULT_TOPIC` / `DEFAULT_TOPIC_SECRET`). CLI `--topic` / `--topic-secret` still take precedence. |

Kill switches:

| Flag          | Command(s)       | Effect                                                                                                                                                                                                     |
| ------------- | ---------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--no-gossip` | `serve`          | Disable the entire gossip subsystem. No `Gossip` instance is created, no ALPN is registered, no topic is joined, no announce/receive/cleanup tasks run. The server still accepts blob and RPC connections. |
| `--no-mdns`   | `serve`, `peers` | Disable mDNS-based local network peer discovery. The iroh endpoint is created without `MdnsAddressLookup`.                                                                                                 |

## `defaults.conf` Format

Embedded at compile time from `src/defaults.conf` via `include_str!()`. Simple INI-like format:

```ini
[section]      # starts a section
# comment      # ignored
value          # one value per line; blank lines ignored
```

Sections:

- **`[bootstrap]`** -- collects all non-comment lines as node IDs.
- **`[topic]`** -- only the first non-comment line is used.
- **`[topic_secret]`** -- only the first non-comment line is used.
- Unknown sections are silently ignored.

The shipped file contains the same topic/secret as the hardcoded constants and **no bootstrap nodes**. Distributors building custom binaries can edit this file before compilation to bake in their own bootstrap nodes and/or private topic.

## Core Types

### `Defaults` (`src/discovery.rs`)

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Defaults {
    pub bootstrap: Vec<String>,
    pub topic: Option<String>,
    pub topic_secret: Option<String>,
}
```

Produced by `parse_defaults()`. Cached globally in a `LazyLock<Defaults>` (`PARSED_DEFAULTS`), accessed via `defaults() -> &'static Defaults`.

### `ResolvedConfig` (`src/discovery.rs`)

```rust
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub bootstrap: Vec<String>,
    pub topic: String,
    pub topic_secret: Vec<u8>,
}
```

All fields are fully resolved and ready to use. Produced by `resolve_config()`.

### `resolve_config()` (`src/discovery.rs`)

```rust
pub fn resolve_config(
    cli_bootstrap: &[String],
    cli_topic: Option<&str>,
    cli_topic_secret: Option<&str>,
    replace_defaults: bool,
    no_default_bootstrap: bool,
    no_default_topic: bool,
) -> ResolvedConfig
```

Single merge point called from exactly two sites:

- `cmd_serve()` in `src/commands/serve.rs`
- `discover_via_gossip()` in `src/commands/peers.rs`

Algorithm:

1. Start with hardcoded `DEFAULT_TOPIC` / `DEFAULT_TOPIC_SECRET`.
2. If `defaults.conf` has topic/secret, override. If `defaults.conf` has bootstrap entries, collect them.
3. If `replace_defaults`: skip step 1 entirely -- use only `defaults.conf` values.
4. Append CLI `cli_bootstrap` entries.
5. If CLI provides topic or secret, override.
6. If `no_default_bootstrap`: retain only CLI-provided bootstrap nodes.
7. If `no_default_topic`: drop non-hardcoded topic/secret overrides (hardcoded constants remain as fallback to avoid empty values). CLI `--topic`/`--topic-secret` still wins.

## CLI Flags

### `serve` command (all flags)

```
id serve [--ephemeral] [--no-relay] [--no-gossip] [--no-mdns]
         [--web] [--port PORT]
         [--bootstrap NODE_IDS] [--topic NAME] [--topic-secret SECRET]
         [--no-default-bootstrap] [--no-default-topic] [--replace-defaults]
```

New flags added in this update: `--no-gossip`, `--no-default-bootstrap`, `--no-default-topic`, `--replace-defaults`, `--no-mdns`.

When `--no-gossip` is set, `cmd_serve()` builds the router with only `META_ALPN` and `BLOBS_ALPN`. No `Gossip` instance exists for the lifetime of the process.

When `--no-mdns` is set, the endpoint is created without `MdnsAddressLookup`, disabling local network discovery.

### `peers` command (all flags)

```
id peers [--gossip] [--rpc] [--depth N] [--max-peers N] [--timeout N]
         [--bootstrap NODE_IDS] [--topic NAME] [--topic-secret SECRET]
         [--no-default-bootstrap] [--no-default-topic] [--replace-defaults]
         [--no-relay] [--no-mdns] [NODE]
```

New flags added in this update: `--no-default-bootstrap`, `--no-default-topic`, `--replace-defaults`, `--no-mdns`.

These three fields are added to `PeersOptions` and forwarded to `resolve_config()` inside `discover_via_gossip()`.

## Design Decisions

### Why `include_str!()` + `LazyLock` instead of `const fn` parsing

Rust's `const fn` cannot iterate strings or allocate `Vec`/`String` at compile time. Using `include_str!()` to embed the raw text and `LazyLock` to parse it once at first access is the simplest correct approach. The parse happens exactly once per process, costs microseconds, and the result lives for `'static`.

### Why `defaults.conf` ships the same values as the hardcoded constants

This is intentional. The file serves as a documented template for customisation. Out of the box, editing it changes nothing. Distributors who fork the project can edit `src/defaults.conf` to bake in private bootstrap nodes and topics without touching Rust source code.

### Why `--no-default-topic` falls back to hardcoded constants, not empty strings

An empty topic string or empty secret would cause a runtime error (DHT operations fail, gossip topic creation panics on empty input). Falling back to the well-known public defaults ensures the system always has a valid topic, even when defaults are stripped.

### Why `--no-gossip` is only on `serve`, not on `peers`

The `peers` command already has `--rpc` to skip gossip entirely. Adding `--no-gossip` to `peers` would be redundant. On `serve`, the flag is needed because gossip is otherwise always active -- there is no `--rpc` equivalent for the server.

## mDNS Local Peer Discovery

### Overview

mDNS (Multicast DNS) enables zero-configuration peer discovery on the local network. When enabled, iroh nodes automatically find each other without needing bootstrap nodes, relay servers, or DHT lookups -- useful for development, LAN deployments, and offline networks.

### Implementation

Uses iroh's built-in `address-lookup-mdns` feature flag, which depends on the `swarm-discovery` crate. This is a compile-time feature that adds mDNS address lookup to endpoint builders.

**Cargo.toml**: `iroh = { version = "0.97", features = ["address-lookup-mdns"] }`

**Endpoint builder pattern**:

```rust
use iroh::address_lookup::MdnsAddressLookup;

let mut builder = Endpoint::builder(presets::N0).secret_key(key);
if !no_mdns {
    builder = builder.address_lookup(MdnsAddressLookup::builder());
}
let endpoint = builder.bind().await?;
```

### Where mDNS is Enabled

| Location                                     | Behavior                                              |
| -------------------------------------------- | ----------------------------------------------------- |
| `cmd_serve()`                                | Conditional on `--no-mdns` flag                       |
| `discover_via_gossip()` (peers command)      | Conditional on `--no-mdns` flag                       |
| `query_remote_peers()` (peers command)       | Conditional on `--no-mdns` flag                       |
| `crawl_peers()` (peers command)              | Passes `no_mdns` through to `query_remote_peers()`    |
| `create_local_client_endpoint()` (client.rs) | Always enabled -- mDNS benefits local RPC connections |

### Relationship to `presets::N0`

`presets::N0` adds PkarrPublisher + DnsAddressLookup + default relay configuration. It does **not** include mDNS. The `MdnsAddressLookup` is added separately via `builder.address_lookup()`.

### Startup Output

When `--no-gossip` and `--no-mdns` are not set:

```
peers: gossip enabled (topic: id-default)
mdns: enabled
```

With `--no-mdns`:

```
peers: gossip enabled (topic: id-default)
mdns: disabled
```

## Bootstrap Deduplication

`resolve_config()` deduplicates bootstrap node IDs after merging all layers. This prevents the same node ID from appearing multiple times (e.g., when `defaults.conf` and CLI both specify the same bootstrap peer).

Implementation uses a `HashSet`-based `retain()` that preserves insertion order while removing duplicates:

```rust
let mut seen = HashSet::new();
config.bootstrap.retain(|id| seen.insert(id.clone()));
```

## Files Changed

| File                     | Change                                                                                                                                                                                                            |
| ------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/defaults.conf`      | **New** -- build-time defaults file, embedded via `include_str!()`                                                                                                                                                |
| `Cargo.toml`             | Changed `iroh = "0.97"` to `iroh = { version = "0.97", features = ["address-lookup-mdns"] }`                                                                                                                      |
| `src/discovery.rs`       | Added `Defaults`, `parse_defaults()`, `defaults()`, `DEFAULTS_CONF`, `PARSED_DEFAULTS` (LazyLock), `ResolvedConfig`, `resolve_config()` with bootstrap dedup, 14 new tests                                        |
| `src/cli.rs`             | Added `no_gossip`, `no_default_bootstrap`, `no_default_topic`, `replace_defaults`, `no_mdns` to `Serve`; added `no_default_bootstrap`, `no_default_topic`, `replace_defaults`, `no_mdns` to `Peers`; 11 new tests |
| `src/main.rs`            | Updated `Serve` and `Peers` dispatch to destructure and forward all new fields including `no_mdns`                                                                                                                |
| `src/commands/serve.rs`  | Rewrote `cmd_serve()` to accept `no_mdns` param; conditional `MdnsAddressLookup` on endpoint builder; mDNS status in startup output                                                                               |
| `src/commands/peers.rs`  | Added `no_mdns` to `PeersOptions`; wired into `discover_via_gossip()`, `query_remote_peers()`, and `crawl_peers()` endpoint builders                                                                              |
| `src/commands/client.rs` | Added `MdnsAddressLookup` to `create_local_client_endpoint()` (always-on)                                                                                                                                         |
| `src/lib.rs`             | Added re-exports: `Defaults`, `ResolvedConfig`, `defaults`, `parse_defaults`, `resolve_config`                                                                                                                    |

## Test Coverage

### `parse_defaults` (6 tests in `discovery.rs`)

- `test_parse_defaults_full` -- all sections populated
- `test_parse_defaults_empty` -- empty input
- `test_parse_defaults_comments_only` -- sections with only comments
- `test_parse_defaults_only_first_topic_value` -- verifies only first line used for topic
- `test_parse_defaults_unknown_section_ignored` -- unknown `[section]` headers
- `test_parse_defaults_embedded_file` -- the actual shipped `defaults.conf`

### `resolve_config` (8 tests in `discovery.rs`)

- `test_resolve_config_all_defaults` -- no CLI flags, pure defaults
- `test_resolve_config_cli_overrides` -- CLI topic/secret override
- `test_resolve_config_cli_bootstrap_appended_to_defaults` -- merge behaviour
- `test_resolve_config_no_default_bootstrap` -- `--no-default-bootstrap` strips defaults
- `test_resolve_config_no_default_topic` -- `--no-default-topic` fallback
- `test_resolve_config_no_default_topic_with_cli` -- `--no-default-topic` + CLI override
- `test_resolve_config_replace_defaults` -- `--replace-defaults` skips hardcoded
- `test_resolve_config_deduplicates_bootstrap` -- duplicate bootstrap entries are removed

### CLI flag parsing (11 new tests in `cli.rs`)

- `test_cli_parse_serve_no_gossip`
- `test_cli_parse_serve_no_default_bootstrap`
- `test_cli_parse_serve_no_default_topic`
- `test_cli_parse_serve_replace_defaults`
- `test_cli_parse_serve_all_new_flags` -- all five flags together (including `--no-mdns`)
- `test_cli_parse_serve_no_mdns` -- `--no-mdns` flag on serve
- `test_cli_parse_peers_no_default_bootstrap`
- `test_cli_parse_peers_no_default_topic`
- `test_cli_parse_peers_replace_defaults`
- `test_cli_parse_peers_gossip_mode` -- updated for new struct fields
- `test_cli_parse_peers_no_mdns` -- `--no-mdns` flag on peers

### Accessor (1 test in `discovery.rs`)

- `test_defaults_accessor` -- `defaults()` returns parsed embedded file

## Verification

| Check                                       | Result                                                        |
| ------------------------------------------- | ------------------------------------------------------------- |
| `cargo fmt`                                 | Clean                                                         |
| `cargo clippy --all-targets --all-features` | 0 warnings                                                    |
| `cargo check --all-features`                | Clean (swarm-discovery compiles)                              |
| `cargo test --all-features --lib`           | 352 passed, 1 pre-existing failure (unrelated bun asset test) |
| `just check`                                | Passes (same 1 pre-existing failure)                          |

## Not Yet Implemented

- **Bootstrap node provisioning** -- The `[bootstrap]` section in `defaults.conf` is empty. No public bootstrap infrastructure exists yet.

## References

- [Initial design document](2026-03-22T00-00-00Z_feature_peer-discovery.md)
- [Original plan](../../../.opencode/plans/2026-03-21_peer-discovery.md)
- `src/defaults.conf` -- the embedded defaults file
- `src/discovery.rs:72-299` -- all new types and functions
- `src/cli.rs:175-243` -- Serve flags, `src/cli.rs:814-878` -- Peers flags
- `src/commands/serve.rs:246-360` -- serve integration
- `src/commands/peers.rs:70-96,330-418` -- peers integration
