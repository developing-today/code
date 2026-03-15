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
//! │  put.rs │ get.rs │ find.rs │ list.rs │ serve.rs │ id.rs        │
//! ├─────────────────────────────────────────────────────────────────┤
//! │              Protocol Layer (protocol.rs)                       │
//! │  MetaRequest/Response │ FindMatch │ MetaProtocol               │
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
//! - [`commands`] - Command implementations (put, get, find, list, serve, etc.)
//! - [`protocol`] - Network protocol types for metadata exchange
//! - [`store`] - Blob storage and keypair management
//! - [`repl`] - Interactive REPL with shell-like features
//! - [`helpers`] - Utility functions for parsing and formatting
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
//! - `.iroh-store/` - SQLite database with blob data
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
//! - **Meta Protocol** (`/iroh-meta/1`): For metadata operations (list, find, delete, etc.)
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

pub mod cli;
pub mod commands;
pub mod helpers;
pub mod protocol;
pub mod repl;
pub mod store;

// Re-export commonly used types for convenience
pub use cli::{Cli, Command};
pub use protocol::{FindMatch, MatchKind, MetaProtocol, MetaRequest, MetaResponse, TaggedMatch};
pub use store::{StoreType, load_or_create_keypair, open_store};
pub use commands::{
    ServeInfo, ReplContext, ReplContextInner,
    cmd_id, cmd_serve, cmd_list, cmd_list_remote,
    cmd_put_hash, cmd_put_local_file, cmd_put_local_stdin, cmd_put_one, cmd_put_one_remote, cmd_put_multi,
    cmd_gethash, cmd_get_local, cmd_get_one, cmd_get_one_remote, cmd_get_multi,
    cmd_find, cmd_search, cmd_find_matches, cmd_show, cmd_peek,
    SearchOptions, PeekOptions,
    create_local_client_endpoint, create_serve_lock, get_serve_info, is_process_alive, remove_serve_lock,
};
pub use helpers::{parse_put_spec, parse_get_spec, print_match_cli, print_matches_cli, print_match_repl};
pub use repl::run_repl;

use anyhow::Result;
use std::path::PathBuf;

// ============================================================================
// Constants
// ============================================================================

/// Filename for the server's Ed25519 keypair.
///
/// This file is created in the working directory and contains the private key
/// used to identify this node. The public key derived from this is the node ID.
pub const KEY_FILE: &str = ".iroh-key";

/// Filename for the client's Ed25519 keypair.
///
/// Used when connecting to remote peers. Separate from the server key to allow
/// different identities for serving vs. connecting.
pub const CLIENT_KEY_FILE: &str = ".iroh-key-client";

/// Directory name for persistent blob storage.
///
/// Contains an SQLite database with blob data and metadata. Only one process
/// can access this at a time due to SQLite locking.
pub const STORE_PATH: &str = ".iroh-store";

/// Filename for the serve lock file.
///
/// Created when `id serve` is running. Contains the node ID and addresses
/// for local clients to connect. Automatically removed on clean shutdown.
pub const SERVE_LOCK: &str = ".iroh-serve.lock";

/// Application-Level Protocol Negotiation (ALPN) identifier for the meta protocol.
///
/// Used during QUIC handshake to identify connections for metadata operations
/// (list, find, delete, rename, etc.) as opposed to blob data transfer.
pub const META_ALPN: &[u8] = b"/iroh-meta/1";

// ============================================================================
// Utility Functions
// ============================================================================

/// Converts a potentially relative path to an absolute path.
///
/// If the path is already absolute, it is returned unchanged. Otherwise,
/// it is joined with the current working directory.
///
/// # Arguments
///
/// * `path` - The path to convert
///
/// # Returns
///
/// The absolute path, or an error if the current directory cannot be determined.
///
/// # Example
///
/// ```rust
/// use std::path::PathBuf;
/// use id::to_absolute;
///
/// let abs = to_absolute(&PathBuf::from("/already/absolute")).unwrap();
/// assert_eq!(abs, PathBuf::from("/already/absolute"));
///
/// // Relative paths are joined with the current directory
/// let rel = to_absolute(&PathBuf::from("relative/path")).unwrap();
/// assert!(rel.is_absolute());
/// ```
pub fn to_absolute(path: &PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.clone())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

/// Determines how a search query matches a string.
///
/// This function implements a priority-based matching system:
/// 1. **Exact**: The strings are identical
/// 2. **Prefix**: The haystack starts with the needle
/// 3. **Contains**: The haystack contains the needle somewhere
///
/// # Arguments
///
/// * `haystack` - The string to search within (e.g., filename or hash)
/// * `needle` - The search query
///
/// # Returns
///
/// * `Some(MatchKind::Exact)` if `haystack == needle`
/// * `Some(MatchKind::Prefix)` if `haystack.starts_with(needle)`
/// * `Some(MatchKind::Contains)` if `haystack.contains(needle)`
/// * `None` if no match
///
/// # Example
///
/// ```rust
/// use id::{match_kind, MatchKind};
///
/// assert_eq!(match_kind("hello.txt", "hello.txt"), Some(MatchKind::Exact));
/// assert_eq!(match_kind("hello.txt", "hello"), Some(MatchKind::Prefix));
/// assert_eq!(match_kind("say-hello.txt", "hello"), Some(MatchKind::Contains));
/// assert_eq!(match_kind("world.txt", "hello"), None);
/// ```
pub fn match_kind(haystack: &str, needle: &str) -> Option<MatchKind> {
    if haystack == needle {
        Some(MatchKind::Exact)
    } else if haystack.starts_with(needle) {
        Some(MatchKind::Prefix)
    } else if haystack.contains(needle) {
        Some(MatchKind::Contains)
    } else {
        None
    }
}

/// Checks if a string is a valid node ID format.
///
/// A valid node ID is exactly 64 hexadecimal characters (representing a
/// 32-byte Ed25519 public key). This function only validates the format,
/// not whether the node actually exists.
///
/// # Arguments
///
/// * `s` - The string to check
///
/// # Returns
///
/// `true` if the string is exactly 64 hex characters, `false` otherwise.
///
/// # Example
///
/// ```rust
/// use id::is_node_id;
///
/// // Valid node ID (64 hex chars)
/// assert!(is_node_id("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"));
///
/// // Invalid: too short
/// assert!(!is_node_id("0123456789abcdef"));
///
/// // Invalid: contains non-hex characters
/// assert!(!is_node_id("ghijklmnopqrstuv0123456789abcdef0123456789abcdef0123456789abcd"));
/// ```
pub fn is_node_id(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Parses items from stdin, splitting on common delimiters.
///
/// Reads all content from stdin and splits it on newlines, tabs, or commas.
/// Empty items and whitespace-only items are filtered out. This is useful
/// for batch operations where file lists are piped in.
///
/// # Returns
///
/// A vector of trimmed, non-empty strings.
///
/// # Example
///
/// ```bash
/// # From command line:
/// echo "file1.txt,file2.txt,file3.txt" | id put --stdin
/// ```
///
/// ```rust,no_run
/// use id::parse_stdin_items;
///
/// // If stdin contains "a.txt\nb.txt\tc.txt,d.txt"
/// let items = parse_stdin_items().unwrap();
/// // items = ["a.txt", "b.txt", "c.txt", "d.txt"]
/// ```
pub fn parse_stdin_items() -> Result<Vec<String>> {
    use std::io::Read;
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    Ok(input
        .split(|c| c == '\n' || c == '\t' || c == ',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect())
}

/// Reads input data from a file path or stdin.
///
/// If the input is "-", reads from stdin. Otherwise, reads from the specified
/// file path. This is a common pattern for CLI tools that accept either a
/// file or piped input.
///
/// # Arguments
///
/// * `input` - Either "-" for stdin, or a file path
///
/// # Returns
///
/// The raw bytes read from the source.
///
/// # Example
///
/// ```rust,ignore
/// use id::read_input;
///
/// # async fn example() -> anyhow::Result<()> {
/// // Read from file
/// let data = read_input("myfile.txt").await?;
///
/// // Read from stdin (when input is "-")
/// let stdin_data = read_input("-").await?;
/// # Ok(())
/// # }
/// ```
pub async fn read_input(input: &str) -> Result<Vec<u8>> {
    use std::io::Read;
    use tokio::fs as afs;
    
    if input == "-" {
        let mut data = Vec::new();
        std::io::stdin().read_to_end(&mut data)?;
        Ok(data)
    } else {
        Ok(afs::read(input).await?)
    }
}

/// Exports a blob to a file or stdout.
///
/// Retrieves the blob data by hash from the store and writes it to the
/// specified output. If output is "-", writes to stdout. Otherwise,
/// writes to a file at the given path (converting to absolute if needed).
///
/// # Arguments
///
/// * `store` - The blob store to read from
/// * `hash` - The hash of the blob to export
/// * `output` - Either "-" for stdout, or a file path
///
/// # Example
///
/// ```rust,ignore
/// use id::{open_store, export_blob};
/// use iroh_blobs::Hash;
///
/// # async fn example() -> anyhow::Result<()> {
/// let store_type = open_store(false).await?;
/// let store = store_type.as_store();
///
/// // Export to file
/// let hash: Hash = "abc123...".parse()?;
/// export_blob(&store, hash, "output.txt").await?;
///
/// // Export to stdout
/// export_blob(&store, hash, "-").await?;
/// # Ok(())
/// # }
/// ```
pub async fn export_blob(store: &iroh_blobs::api::Store, hash: iroh_blobs::Hash, output: &str) -> Result<()> {
    use std::io::Write;
    
    if output == "-" {
        let data = store.blobs().get_bytes(hash).await?;
        std::io::stdout().write_all(&data)?;
    } else {
        let path = to_absolute(&PathBuf::from(output))?;
        store.blobs().export(hash, &path).await?;
        eprintln!("exported: {} -> {}", hash, path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_is_node_id() {
        // Valid 64 hex char string
        assert!(is_node_id("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"));
        // Mixed case
        assert!(is_node_id("0123456789ABCDEF0123456789abcdef0123456789ABCDEF0123456789abcdef"));
        // Too short
        assert!(!is_node_id("0123456789abcdef"));
        // Too long
        assert!(!is_node_id("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0"));
        // Invalid chars
        assert!(!is_node_id("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdeg"));
    }

    #[test]
    fn test_is_node_id_empty() {
        assert!(!is_node_id(""));
    }

    #[test]
    fn test_is_node_id_spaces() {
        assert!(!is_node_id("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde "));
    }

    #[test]
    fn test_is_node_id_all_zeros() {
        assert!(is_node_id("0000000000000000000000000000000000000000000000000000000000000000"));
    }

    #[test]
    fn test_is_node_id_all_f() {
        assert!(is_node_id("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"));
        assert!(is_node_id("FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"));
    }

    #[test]
    fn test_match_kind() {
        // Exact match
        assert_eq!(match_kind("hello", "hello"), Some(MatchKind::Exact));
        // Prefix match
        assert_eq!(match_kind("hello world", "hello"), Some(MatchKind::Prefix));
        // Contains match
        assert_eq!(match_kind("say hello to me", "hello"), Some(MatchKind::Contains));
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
        assert_eq!(META_ALPN, b"/iroh-meta/1");
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
        export_blob(&store, result.hash, output_path.to_str().unwrap()).await.unwrap();
        
        // Verify content
        let read_data = std::fs::read(&output_path).unwrap();
        assert_eq!(read_data, data);
        
        store_type.shutdown().await.unwrap();
    }
}
