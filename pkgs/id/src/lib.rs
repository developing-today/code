//! # ID - Peer-to-Peer File Sharing with Iroh
//!
//! `id` is a peer-to-peer file sharing tool and library built on the [Iroh](https://iroh.computer)
//! networking stack. It provides content-addressed blob storage with human-readable naming via tags,
//! enabling secure, decentralized file sharing between peers.
//!
//! ## Overview
//!
//! The system combines several key concepts:
//!
//! - **Content-Addressed Storage**: Files are stored and identified by their cryptographic hash
//!   (BLAKE3), ensuring data integrity and enabling deduplication.
//! - **Human-Readable Tags**: Files can be given meaningful names (tags) that map to their hashes,
//!   making them easier to find and share.
//! - **Peer-to-Peer Networking**: Uses Iroh's QUIC-based networking with NAT traversal via relay
//!   servers and hole punching for direct connections.
//! - **Node Identity**: Each node has a unique Ed25519 keypair, with the public key serving as
//!   the node ID (64 hex characters).
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                         CLI / REPL                              │
//! │  (main.rs - command dispatch, repl/runner.rs - interactive)    │
//! ├─────────────────────────────────────────────────────────────────┤
//! │                      Command Handlers                           │
//! │  put.rs │ get.rs │ find.rs │ list.rs │ serve.rs │ id.rs │ tag.rs│
//! ├─────────────────────────────────────────────────────────────────┤
//! │              Protocol Layer (protocol.rs)                       │
//! │  MetaRequest/Response │ FindMatch │ MetaProtocol               │
//! ├─────────────────────────────────────────────────────────────────┤
//! │            Metadata Tags (tags.rs / tuple.rs)                   │
//! │  TagStore │ NamespacePair │ TagEvent │ TupleEncoder             │
//! ├─────────────────────────────────────────────────────────────────┤
//! │               Storage Layer (store.rs)                          │
//! │  StoreType (Persistent/Ephemeral) │ Keypair Management          │
//! ├─────────────────────────────────────────────────────────────────┤
//! │                    Iroh Foundation                              │
//! │  iroh::Endpoint │ iroh_blobs::Store │ QUIC/Relay               │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Modules
//!
//! - [`cli`] - Command-line argument parsing with clap
//! - [`commands`] - Command implementations (put, get, find, list, serve, tag, etc.)
//! - [`protocol`] - Network protocol types for metadata exchange
//! - [`store`] - Blob storage and keypair management
//! - [`tags`] - Metadata tag storage using iroh-docs CRDT documents
//! - [`repl`] - Interactive REPL with shell-like features
//! - [`helpers`] - Utility functions for parsing and formatting
//!
//! ## Signing contract — sign only what receipt would accept
//!
//! Every public signing entry point in this crate family (`sign_*`,
//! `Signed*::sign`, and any public method whose purpose is producing a
//! signed object) follows one contract, recorded as design #6487 and
//! inventoried in the closing audit on issue #6488:
//!
//! > **Sign only what receipt would accept.** Mirror at sign time every
//! > receipt rule that is decidable from the signer's own inputs — the
//! > key, the object being signed, and the function's existing
//! > arguments. Receipt means whichever side consumes the object: a
//! > fold, a viewer, a verifier, or an accept function. A rule receipt
//! > enforces unconditionally, and the signer can decide, must not be
//! > signable-around: a signer that produces an object no consumer will
//! > ever count has reported success for a failure the caller discovers
//! > far from the signing call, if at all.
//!
//! The decidability line is part of the contract, in four classes:
//!
//! - **(a) intrinsic** — decidable from the object alone: a field's
//!   domain, a version, a roster's shape, an envelope's internal
//!   agreement.
//! - **(b) signer-relative** — decidable from the key plus the object:
//!   the author the key implies versus the object's issuer, players,
//!   roster, successor, source, or named sender.
//! - **(c) seam-extended** — decidable only if the seam takes one more
//!   input it already conceptually has (the agreement for a claim). The
//!   public seam changes — a checked sibling or an extra parameter —
//!   it never silently skips the check.
//! - **(d) receiver-state** — not decidable at the seam: the
//!   receiver's cursor, a per-world limit, a store's current epoch, the
//!   key being replaced in a recovery the inputs do not name, an
//!   assembled log's context. These rules stay receipt-only, and the
//!   closing inventory on issue #6488 names each one with its reason,
//!   so the line is auditable rather than implied.
//!
//! Two corollaries bind a new signer before it lands:
//!
//! - Refusal speaks the module's own vocabulary — the receipt error
//!   itself, or a typed sign error naming the rule — never a generic
//!   failure, and never a silently repaired object. Wire formats and
//!   receipt verdicts do not change under this contract.
//! - A bypass, where tests need one, is crate-private (or gated to
//!   tests) and named `*_unchecked` — the artifact pattern
//!   (`SignedArtifact::sign_unchecked`). There is never a second
//!   *public* unchecked signer.
//!
//! Adding a `sign_*` function without dispositioning it against this
//! contract — enforced for the decidable classes, or deliberately
//! receipt-only with the class-(d) reason named — is incomplete work.
//!
//! ## Quick Start
//!
//! ### As a Library
//!
//! ```rust,no_run
//! use id::{open_store, StoreType};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Open persistent storage
//!     let store = open_store(false).await?;
//!
//!     // Add a blob
//!     let data = b"Hello, world!";
//!     let hash = store.as_store().blobs().add_bytes(data.to_vec()).await?;
//!     println!("Stored with hash: {}", hash.hash);
//!
//!     // Create a named tag
//!     store.as_store().tags().set("greeting.txt", hash.hash).await?;
//!
//!     store.shutdown().await?;
//!     Ok(())
//! }
//! ```
//!
//! ### Command-Line Usage
//!
//! ```bash
//! # Store a file
//! id put myfile.txt
//!
//! # Store with custom name
//! id put myfile.txt:greeting.txt
//!
//! # Retrieve a file
//! id get greeting.txt
//!
//! # List all stored files
//! id list
//!
//! # Search for files
//! id search greeting
//!
//! # Start server for remote access
//! id serve
//!
//! # Interactive REPL
//! id repl
//! ```
//!
//! ## Storage Model
//!
//! Files are stored in two ways:
//!
//! 1. **By Hash**: The raw content is stored and addressed by its BLAKE3 hash.
//!    This is immutable and content-addressed.
//!
//! 2. **By Tag (Name)**: A human-readable name maps to a hash. Tags can be
//!    updated to point to different content, but the underlying blobs remain
//!    immutable.
//!
//! Storage locations (relative to working directory):
//! - `.iroh-store/` - redb database with blob data
//! - `.iroh-key` - Server Ed25519 keypair
//! - `.iroh-key-client` - Client keypair for remote connections
//! - `.iroh-serve.lock` - Lock file when serve is running
//!
//! ## Networking
//!
//! The system supports several networking modes:
//!
//! - **Local Mode**: Direct access to the local store (no server needed)
//! - **Client-Server**: Connect to a local `serve` instance via QUIC
//! - **Remote Peer**: Connect to any peer by their node ID
//!
//! Two protocols are used:
//! - **Blobs Protocol** (`/iroh-blobs/1`): For blob data transfer
//! - **Meta Protocol** (`/iroh-meta/3`): For metadata operations (list, find, delete, etc.)
//!
//! ## Features
//!
//! - **Batch Operations**: Put/get multiple files in one command
//! - **Stdin Support**: Pipe content directly (`echo "data" | id put myfile.txt`)
//! - **Flexible Search**: Find files by exact name, prefix, or substring match
//! - **Hash-Only Mode**: Store content without creating a named tag
//! - **Remote Operations**: All commands work with remote peers via `@NODE_ID` syntax
//!
//! ## Example: Remote File Sharing
//!
//! ```bash
//! # On machine A - start server and note the node ID
//! id serve
//! # Output: Node ID: abc123...
//!
//! # On machine B - put a file to machine A
//! id put abc123... myfile.txt
//!
//! # On machine B - get a file from machine A
//! id get abc123... myfile.txt
//! ```

pub use id_core::abuse;
pub use id_core::access;
#[cfg(feature = "world")]
pub mod artifact;
pub use id_events::attestation;
pub mod backup;
pub mod blob_gate;
#[cfg(feature = "world")]
pub mod boards;
pub use id_core::connectivity;
pub use id_store::chain;
pub use id_store::chat;
pub mod conversion;
pub mod currency;
pub mod currency_poc;
#[cfg(feature = "world")]
pub use id_directory::directory;
#[cfg(feature = "world")]
pub use id_directory::directory_auth;
#[cfg(feature = "world")]
pub use id_directory::directory_log;
pub use id_sync::dht;
#[cfg(feature = "world")]
pub mod keyset;
pub use id_core::discovery;
#[cfg(feature = "world")]
pub use id_directory::directory_mail;
#[cfg(feature = "world")]
pub use id_directory::directory_view;
pub mod didit;
pub use id_store::documents;
#[cfg(feature = "world")]
pub mod encryption;
#[cfg(feature = "world")]
pub mod envelope_net;
#[cfg(feature = "world")]
pub mod envelope_outbox;
pub use id_events::event_store;
pub use id_events::event_sync;
pub mod fileops;
pub mod foreign_libsql;
pub use id_core::frames;
pub mod grant_service;
pub mod grants;
pub mod groups;
pub mod hosted_binding;
pub mod hosted_export;
pub use id_events::idset;
pub mod input_framing;
pub use id_directory::invite;
pub use id_directory::invites;
pub use id_events::local_claims;
pub use id_events::local_events;
pub use id_store::issuer_rotation;
pub use id_sync::log_sync;
pub mod serve_proxy;
pub use id_core::membership;
#[cfg(feature = "world")]
pub use id_core::petname;
pub use id_store::note_links;
pub use id_store::notes;
pub mod pull_policy;
pub use id_core::residency;
pub mod references;
pub use id_core::rejection;
#[cfg(feature = "world")]
pub use id_directory::seitan;
pub use id_store::search;
#[cfg(feature = "world")]
pub use id_store::session_checkpoint;
pub use id_store::store;
#[cfg(feature = "sandbox")]
pub use id_worlds::sandbox;
#[cfg(feature = "world")]
pub use id_worlds::shell_contract;
#[cfg(feature = "sandbox")]
pub use id_worlds::world_dev;
pub mod store_traits;
pub mod structured_logging;
#[cfg(feature = "world")]
pub use id_caps::world_caps;
pub(crate) use id_core::is_format_char;
pub use id_core::sync_method;
#[cfg(test)]
pub(crate) use id_core::test_forgeries;
pub(crate) use id_core::text;
pub(crate) use id_core::time;
pub use id_core::tuple;
pub use id_core::waiting;
pub use id_store::tag_claims;
pub use id_store::tag_index;
pub use id_store::tags;
#[cfg(feature = "world")]
pub use id_worlds::world;
#[cfg(feature = "world")]
pub use id_worlds::world_cache;
pub use id_worlds::world_compile;
#[cfg(feature = "world")]
pub use id_worlds::world_doctor;
#[cfg(feature = "world")]
pub use id_worlds::world_fixture;

#[cfg(feature = "world")]
pub use id_caps::world_limits;
#[cfg(feature = "world")]
pub use id_caps::world_manifest;
#[cfg(feature = "world")]
pub use id_core::world_presentation;
pub use id_worlds::world_hub;
pub(crate) use id_worlds::world_line;
#[cfg(feature = "world")]
pub use id_worlds::world_native;
#[cfg(feature = "world")]
pub use id_worlds::world_net;
#[cfg(feature = "world")]
pub use id_worlds::world_records;
#[cfg(feature = "world")]
pub use id_worlds::world_replication_net;
#[cfg(feature = "world")]
pub use id_worlds::world_session;
#[cfg(feature = "ssh")]
pub use id_worlds::world_ssh;
#[cfg(feature = "world")]
pub use id_worlds::world_store;
#[cfg(feature = "world")]
pub(crate) use id_worlds::world_terminal;

#[cfg(feature = "web")]
pub use id_core::qr;
// Application layer extracted in split step 7b (#4411): the modules
// below live in the id-cli crate and are re-exported at their
// historical `id::` paths. `main.rs` consumes them through here.
pub use id_cli::{
    aliases, app_manifest, chat_presence, cli, commands, helpers, local, match_audit, meta_client,
    protocol, publications, publications_runtime, repl, server_config,
};

#[cfg(feature = "web")]
pub use id_web as web;

// Re-export commonly used types for convenience
pub use discovery::{
    ANNOUNCE_INTERVAL, DEFAULT_TOPIC, DEFAULT_TOPIC_SECRET, Defaults, MAX_PEER_NAME_BYTES,
    MAX_PEERS, PeerAnnouncement, PeerDiscovery, PeerInfo, ResolvedConfig, STALE_CHECK_INTERVAL,
    STALE_THRESHOLD, defaults, parse_defaults, resolve_config,
};
pub use id_cli::cli::{Cli, Command, KeyCommand, TagCommand};
pub use id_cli::commands::{
    PeekOptions, PeersOptions, ReplContext, ReplContextInner, SearchOptions, ServeInfo,
    ServeOptions, cmd_find, cmd_find_matches, cmd_get_local, cmd_get_multi, cmd_get_one,
    cmd_get_one_remote, cmd_gethash, cmd_id, cmd_list, cmd_list_remote, cmd_migrate_tags, cmd_peek,
    cmd_peers, cmd_put_hash, cmd_put_local_file, cmd_put_local_stdin, cmd_put_multi, cmd_put_one,
    cmd_put_one_remote, cmd_search, cmd_serve, cmd_show, cmd_tag, create_local_client_endpoint,
    create_serve_lock, get_serve_info, is_process_alive, remove_serve_lock,
};
pub use id_cli::helpers::{
    download_output_path, parse_get_spec, parse_put_spec, print_match_cli, print_match_repl,
    print_matches_cli,
};
pub use id_cli::protocol::match_kind;
pub use id_cli::protocol::{
    FindMatch, MatchKind, MetaProtocol, MetaRequest, MetaResponse, TaggedMatch,
};
pub use id_cli::repl::run_repl;
pub use store::{StoreType, load_or_create_keypair, open_store};

use anyhow::Result;
use std::path::PathBuf;

// ============================================================================
// Constants
// ============================================================================

pub use id_core::{CLIENT_KEY_FILE, KEY_FILE, SERVE_LOCK};

pub use id_store::STORE_PATH;

pub use id_sync::protocol::META_ALPN;

// ============================================================================
// Utility Functions
// ============================================================================

pub use id_cli::to_absolute;

pub use id_core::is_node_id;

pub use id_cli::parse_stdin_items;

pub use id_cli::read_input;

pub use id_cli::export_blob;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_is_node_id() {
        // Valid 64 hex char string
        assert!(is_node_id(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        ));
        // Mixed case
        assert!(is_node_id(
            "0123456789ABCDEF0123456789abcdef0123456789ABCDEF0123456789abcdef"
        ));
        // Too short
        assert!(!is_node_id("0123456789abcdef"));
        // Too long
        assert!(!is_node_id(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0"
        ));
        // Invalid chars
        assert!(!is_node_id(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdeg"
        ));
    }

    #[test]
    fn test_is_node_id_empty() {
        assert!(!is_node_id(""));
    }

    #[test]
    fn test_is_node_id_spaces() {
        assert!(!is_node_id(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde "
        ));
    }

    #[test]
    fn test_is_node_id_all_zeros() {
        assert!(is_node_id(
            "0000000000000000000000000000000000000000000000000000000000000000"
        ));
    }

    #[test]
    fn test_is_node_id_all_f() {
        assert!(is_node_id(
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
        ));
        assert!(is_node_id(
            "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"
        ));
    }

    #[test]
    fn test_match_kind() {
        // Exact match
        assert_eq!(match_kind("hello", "hello"), Some(MatchKind::Exact));
        // Prefix match
        assert_eq!(match_kind("hello world", "hello"), Some(MatchKind::Prefix));
        // Contains match
        assert_eq!(
            match_kind("say hello to me", "hello"),
            Some(MatchKind::Contains)
        );
        // No match
        assert_eq!(match_kind("goodbye", "hello"), None);
    }

    #[test]
    fn test_match_kind_empty_strings() {
        assert_eq!(match_kind("", ""), Some(MatchKind::Exact));
        // Empty needle: starts_with("") is true, so returns Prefix
        assert_eq!(match_kind("hello", ""), Some(MatchKind::Prefix));
        assert_eq!(match_kind("", "hello"), None);
    }

    #[test]
    fn test_match_kind_unicode() {
        assert_eq!(match_kind("héllo", "héllo"), Some(MatchKind::Exact));
        assert_eq!(match_kind("héllo world", "héllo"), Some(MatchKind::Prefix));
        assert_eq!(match_kind("say héllo", "héllo"), Some(MatchKind::Contains));
    }

    #[test]
    fn test_match_kind_case_sensitive() {
        assert_eq!(match_kind("Hello", "hello"), None);
        assert_eq!(match_kind("hello", "Hello"), None);
        assert_eq!(match_kind("HELLO", "hello"), None);
    }

    #[test]
    fn test_match_kind_ordering() {
        // Exact < Prefix < Contains
        assert!(MatchKind::Exact < MatchKind::Prefix);
        assert!(MatchKind::Prefix < MatchKind::Contains);
    }

    #[test]
    fn test_match_kind_special_chars() {
        assert_eq!(
            match_kind("test.file.txt", "test.file"),
            Some(MatchKind::Prefix)
        );
        assert_eq!(match_kind("path/to/file", "to"), Some(MatchKind::Contains));
    }

    #[test]
    fn test_to_absolute_already_absolute() {
        let path = PathBuf::from("/absolute/path/file.txt");
        let result = to_absolute(&path).unwrap();
        assert_eq!(result, path);
    }

    #[test]
    fn test_to_absolute_relative() {
        // Only run if we can get the current directory
        if let Ok(cwd) = std::env::current_dir() {
            let path = PathBuf::from("relative/path/file.txt");
            let result = to_absolute(&path).unwrap();
            assert!(result.is_absolute());
            assert!(result.ends_with("relative/path/file.txt"));
            assert!(result.starts_with(&cwd));
        }
    }

    #[test]
    fn test_to_absolute_current_dir() {
        // Only run if we can get the current directory
        if let Ok(_cwd) = std::env::current_dir() {
            let path = PathBuf::from(".");
            let result = to_absolute(&path).unwrap();
            assert!(result.is_absolute());
        }
    }

    #[test]
    fn test_constants() {
        assert_eq!(KEY_FILE, ".iroh-key");
        assert_eq!(CLIENT_KEY_FILE, ".iroh-key-client");
        assert_eq!(STORE_PATH, ".iroh-store");
        assert_eq!(SERVE_LOCK, ".iroh-serve.lock");
        assert_eq!(META_ALPN, b"/iroh-meta/3");
    }

    #[tokio::test]
    async fn test_read_input_from_file() {
        let tmp_dir = TempDir::new().unwrap();
        let file_path = tmp_dir.path().join("test.txt");
        std::fs::write(&file_path, b"test content").unwrap();

        let data = read_input(file_path.to_str().unwrap()).await.unwrap();
        assert_eq!(data, b"test content");
    }

    #[tokio::test]
    async fn test_read_input_nonexistent_file() {
        let result = read_input("/nonexistent/path/file.txt").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_export_blob_to_file() {
        // Create an ephemeral store
        let store_type = open_store(true).await.unwrap();
        let store = store_type.as_store();

        // Add a blob
        let data = b"export test content";
        let result = store.blobs().add_bytes(data.to_vec()).await.unwrap();

        // Export to file
        let tmp_dir = TempDir::new().unwrap();
        let output_path = tmp_dir.path().join("exported.txt");
        export_blob(&store, result.hash, output_path.to_str().unwrap())
            .await
            .unwrap();

        // Verify content
        let read_data = std::fs::read(&output_path).unwrap();
        assert_eq!(read_data, data);

        store_type.shutdown().await.unwrap();
    }
}
