# The id facade crate

Source: primary sources are [pkgs/id/src/lib.rs](https://github.com/developing-today/code/blob/main/pkgs/id/src/lib.rs), [pkgs/id/Cargo.toml](https://github.com/developing-today/code/blob/main/pkgs/id/Cargo.toml), and [pkgs/id/src/main.rs](https://github.com/developing-today/code/blob/main/pkgs/id/src/main.rs). This chapter is complete without following those links; they record provenance only.

The root package `id` (version 0.1.0, edition 2024, rust-version 1.96) is the facade for the workspace described in [The crates](crates.md). It is not a thin re-export shim. `pkgs/id/src/` owns 26 modules of first-party code, and `src/lib.rs` re-exports about 90 modules from ten sub-crates at `id::` paths, plus a curated set of individual types, functions, and constants. A Rust consumer depends on this one package and writes `id::` paths for the whole split family: files, tags, events, sync, directory, worlds, the command layer, and — under the `web` feature — the web server.

Two scope warnings belong at the front. First, the crate's self-description is stale: the root `Cargo.toml` description and the `src/lib.rs` title both read "A peer-to-peer file sharing CLI built with Iroh", and the compiled surface also includes a sandboxed world runtime, a home-server directory, group encryption, and a currency registry (disagreement LIB-13). Second, the `src/lib.rs` doc header still describes the pre-split architecture: its Architecture diagram and Modules list present `cli`, `commands`, `protocol`, `store`, `tags`, and `repl` as this crate's own modules and show no worlds, sync, events, directory, or currency layer at all. Every one of those modules moved out in split steps 2a to 7b; the header was not updated (disagreement LIB-3). A reader who learns the library from the rustdoc front page gets the 2025 single-crate architecture. This chapter documents the code, not the header. The header does carry one load-bearing piece of current documentation: the family-wide signing contract ("sign only what receipt would accept", design #6487, audit #6488) with its four decidability classes — (a) intrinsic, (b) signer-relative, (c) seam-extended, (d) receiver-state.

## The facade's own 26 modules

These are `pub mod` declarations with code in `pkgs/id/src/`. Feature gates are noted; unmarked modules are unconditional.

| Module               | Gate    | Content                                                                                                                     |
| -------------------- | ------- | --------------------------------------------------------------------------------------------------------------------------- |
| `artifact`           | `world` | Shim over the split artifact owners                                                                                         |
| `backup`             | —       | Backup and restore                                                                                                          |
| `blob_gate`          | —       | Blob gate at its historical path, over `id-store` / `id-groups` / `id-worlds` owners                                        |
| `boards`             | `world` | Game posts; runs guest packages through `id::sandbox`                                                                       |
| `conversion`         | —       | Format conversion                                                                                                           |
| `currency`           | —       | Currency surface; composes `id-currency-registry`                                                                           |
| `currency_poc`       | —       | TigerBeetle proof-of-concept harness for #2289                                                                              |
| `device_bridge`      | —       | Device bridge                                                                                                               |
| `didit`              | —       | The did-it counted view                                                                                                     |
| `encryption`         | `world` | Encryption at its historical path                                                                                           |
| `envelope_net`       | `world` | Envelope networking, over `id-sync` owners                                                                                  |
| `envelope_outbox`    | `world` | Envelope outbox, over `id-sync` owners                                                                                      |
| `fileops`            | —       | File operations at their historical path, over `id-sync`                                                                    |
| `foreign_libsql`     | —       | Foreign libSQL access                                                                                                       |
| `grant_service`      | —       | Grant service                                                                                                               |
| `grants`             | —       | Grants                                                                                                                      |
| `groups`             | —       | Group facade: `src/groups/mod.rs` re-exports from `id_events::groups`, `id_groups::groups`, and `id_worlds::groups::worlds` |
| `hosted_binding`     | —       | Hosted-edition binding ceremony                                                                                             |
| `hosted_export`      | —       | Hosted export                                                                                                               |
| `hosted_ingest`      | —       | Hosted ingest                                                                                                               |
| `input_framing`      | —       | Input framing                                                                                                               |
| `pull_policy`        | —       | Pull policy at its historical path, over `id-events`                                                                        |
| `references`         | —       | References                                                                                                                  |
| `serve_proxy`        | —       | Serve proxy                                                                                                                 |
| `store_traits`       | —       | Store traits at their historical path, over `id-store`                                                                      |
| `structured_logging` | —       | Structured logging configuration                                                                                            |

The binary entry point `src/main.rs` also stays in this package and consumes the library through `id::` paths exactly like an external consumer.

## The re-export surface

Each module below becomes available at the `id::` path shown. Feature gates are noted; unmarked items are unconditional.

From `id-core`: `abuse`, `access`, `admission_gate`, `connectivity`, `discovery`, `frames`, `holders`, `membership`, `rejection`, `residency`, `sync_method`, `tuple`, `waiting`; plus `petname` and `world_presentation` under `world`, and `qr` under `web`.

From `id-events`: `attestation`, `event_store`, `event_sync`, `idset`, `local_claims`, `local_events`.

From `id-store`: `admission`, `chain`, `chat`, `chat_anchor`, `documents`, `issuer_rotation`, `media`, `note_links`, `notes`, `search`, `store`, `tag_claims`, `tag_index`, `tags`; plus `session_checkpoint` under `world`.

From `id-sync`: `chained_message`, `dht`, `log_sync`, `transfer`. The `dht` module itself is feature-built inside `id-sync`.

From `id-directory`: `keyset`, `invite`, `invites` unconditionally; `directory`, `directory_auth`, `directory_log`, `directory_mail`, `directory_view`, and `seitan` under `world`.

From `id-caps`: `world_caps` unconditionally; `world_limits` and `world_manifest` under `world`.

From `id-worlds`: `world_compile` and `world_hub` unconditionally; `shell_contract`, `world`, `world_cache`, `world_doctor`, `world_fixture`, `world_native`, `world_net`, `world_records`, `world_replication_net`, `world_session`, `world_store` under `world`; `guest_burn`, `sandbox`, and `world_dev` under `sandbox`; `world_ssh` under `ssh`.

From `id-cli`, in one block: `aliases`, `app_manifest`, `chat_presence`, `cli`, `commands`, `helpers`, `local`, `match_audit`, `meta_client`, `protocol`, `publications`, `publications_runtime`, `repl`, `server_config`.

From `id-web`: the whole crate as `id::web`, under the `web` feature only.

ARCHITECTURE.md lists the root's `id-core` re-exports as 12 modules and concludes that modules not re-exported by `id` are internal to the workspace. The code disagrees: the facade also re-exports `id-core`'s `admission_gate`, `petname`, `world_presentation`, and `qr`, and about 90 modules from ten other crates in total (disagreement LIB-6). The re-export list in `src/lib.rs` is the public/internal line.

## Item-level re-exports

Beyond whole modules, `src/lib.rs` re-exports individual items as the convenience surface:

- From `id_core::discovery`, 17 items: `ANNOUNCE_INTERVAL`, `DEFAULT_TOPIC`, `DEFAULT_TOPIC_SECRET`, `Defaults`, `DefaultsMode`, `DefaultsProvenance`, `DefaultsSource`, `MAX_PEER_NAME_BYTES`, `MAX_PEERS`, `PeerAnnouncement`, `PeerDiscovery`, `PeerInfo`, `ResolvedConfig`, `STALE_CHECK_INTERVAL`, `STALE_THRESHOLD`, `defaults`, `defaults_provenance`, `resolve_config`. Note that `id::ResolvedConfig` is the discovery type from `id-core`. A second type with the same name, `id_config::ResolvedConfig`, exists in the workspace and is not exposed at `id::`.
- From `id_cli::cli`: `Cli`, `Command`, `KeyCommand`, `KeysetCommand`, `TagCommand`.
- From `id_cli::commands`: `PeekOptions`, `PeersOptions`, `ReplContext`, `ReplContextInner`, `SearchOptions`, `ServeInfo`, `ServeOptions`, and the functions `cmd_find`, `cmd_find_matches`, `cmd_get_local`, `cmd_get_multi`, `cmd_get_one`, `cmd_get_one_remote`, `cmd_gethash`, `cmd_id`, `cmd_list`, `cmd_list_remote`, `cmd_migrate_tags`, `cmd_peek`, `cmd_peers`, `cmd_put_hash`, `cmd_put_local_file`, `cmd_put_local_stdin`, `cmd_put_multi`, `cmd_put_one`, `cmd_put_one_remote`, `cmd_search`, `cmd_send`, `cmd_serve`, `cmd_show`, `cmd_tag`, `create_local_client_endpoint`, `create_serve_lock`, `get_serve_info`, `is_process_alive`, `remove_serve_lock`.
- From `id_cli::helpers`: `download_output_path`, `parse_get_spec`, `parse_put_spec`, `print_match_cli`, `print_match_repl`, `print_matches_cli`.
- From `id_cli::protocol`: `match_kind`, `FindMatch`, `MatchKind`, `MetaProtocol`, `MetaRequest`, `MetaResponse`, `TaggedMatch`.
- From `id_cli::repl`: `run_repl`.
- From `id_store::store`: `StoreType`, `load_or_create_keypair`, `open_store`.
- Constants: `CLIENT_KEY_FILE`, `KEY_FILE`, `SERVE_LOCK` from `id-core`; `STORE_PATH` from `id-store`; `META_ALPN` from `id-sync::protocol`.
- Functions: `to_absolute`, `parse_stdin_items`, `read_input`, `export_blob` from `id-cli`; `is_node_id` from `id-core`.

## What the facade does not expose

Nothing from five crates is available at `id::` paths: `id-config` (a dev-dependency of the facade only), `id-currency-registry` (consumed by the facade-internal `src/currency/gateway_poc.rs` under its own crate path), `id-render`, `id-import-notion`, `id-native-shell`, and `id-hw-shell`. A library consumer who wants those names the crates directly. The facade's role as the only public surface holds for the split family, not for these crates.

One documentation disagreement sits in the facade's own manifest: the `dht` feature comment in the root `Cargo.toml` states that `src/dht.rs` decides per server. No `src/dht.rs` exists in the facade; the module is `crates/id-sync/src/dht.rs`, re-exported as `id::dht` (disagreement LIB-15).

## Features

- `default = ["sandbox", "world"]`.
- `sandbox` forwards to `id-core/sandbox`, `id-caps/sandbox`, `id-worlds/sandbox`, and `id-cli/sandbox`, and implies `world`.
- `world` forwards to `id-core/world` and `id-cli/world`.
- `ssh` forwards to `id-worlds/ssh` and `id-cli/ssh`.
- `web` adds the optional `id-web` dependency and forwards to `id-core/web` and `id-cli/web`.
- `dht` forwards to `id-sync/dht` and `id-cli/dht`.
- `currency-poc-tb` adds the optional `tigerbeetle` dependency, the official TigerBeetle Rust client, for the #2289 live-cluster proof of concept.

## Dependents

Two production consumers depend on the facade from outside the split family: `id-native-shell` and `id-hw-shell`, each with `default-features = false, features = ["world"]` (see [Guest apps and shells as consumers](guests-and-shells.md)). Inside the workspace, `id-store` dev-depends on the facade: its `store_traits` tests run against the real `EventStore`, and a comment at `crates/id-store/Cargo.toml` in the `[dev-dependencies]` section states that production code in `id-store` never names the facade. Direct-crate-path use inside the facade's own `src/` runs the other way: mentions of `id_<crate>::` count `id-core` 32, `id-worlds` 23, `id-store` 23, `id-events` 16, `id-currency-registry` 15, `id-directory` 11, `id-cli` 11, `id-sync` 10, `id-groups` 7, and `id-caps` 5.

## Embedding examples from real call sites

The shipped binary is the first example. `pkgs/id/src/main.rs:10-13`:

```rust
use id::{Cli, Command, PeekOptions, PeersOptions, SearchOptions,
         ServeOptions, cmd_find, cmd_get_multi, cmd_gethash, cmd_id,
         cmd_list, cmd_migrate_tags, cmd_peek, cmd_peers, cmd_put_hash,
         cmd_put_multi, cmd_search, cmd_serve, cmd_show, cmd_tag, run_repl};
```

The same file calls `id::cli::normalize_args(std::env::args_os())` at line 202, reads structured-logging configuration through `id::structured_logging::{LogConfig, LogFormat}` at lines 98–99, and calls the `id-core` discovery module by its direct crate path at line 195: `let provenance = id_core::discovery::defaults_provenance();`.

The documented quick-start in the `src/lib.rs` header is the embedding form for the store: `open_store(false).await?`, then `store.as_store().blobs().add_bytes(…)` and `store.as_store().tags().set(…)`. All three entry points — `open_store`, `StoreType`, and `load_or_create_keypair` — reach the consumer through the facade's re-exports from `id_store::store`.

A world example comes from the facade's own game-post path. `pkgs/id/src/boards.rs:1592-1594` compiles and runs a guest package through the facade's `id::sandbox` re-export of `id_worlds::sandbox`:

```rust
let sandbox = crate::sandbox::Sandbox::compile(package_bytes,
    crate::sandbox::SandboxLimits::default())?;
let mut instance = sandbox.instantiate(0)?;
let genesis_bytes = instance.snapshot()?;
instance.restore(&genesis_bytes)?;
```

The integration test `pkgs/id/tests/first_app_poc.rs:33-45` composes three crates through facade paths in one import block — `id::{chain::{sign, ChainExt, Event}, documents::{DocumentRef, DOCUMENT_KIND}, event_store::{EventStore, Ingest}, idset::…, tag_claims::{TagOp, SET_KIND}, waiting::…}` — and builds the counted did-it view from `id-events` and `id-store` parts with no new protocol code. In total, `use id::` occurs 165 times under `pkgs/id/tests/`, which is the strongest evidence that the re-export list above is the real public API.
