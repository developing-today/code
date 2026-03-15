//! ID - A peer-to-peer file sharing library using Iroh
//!
//! This library provides content-addressed blob storage with human-readable naming via tags,
//! built on top of the Iroh networking stack.

pub mod cli;
pub mod commands;
pub mod helpers;
pub mod protocol;
pub mod repl;
pub mod store;

// Re-export commonly used types
pub use cli::{Cli, Command};
pub use protocol::{FindMatch, MatchKind, MetaProtocol, MetaRequest, MetaResponse, TaggedMatch};
pub use store::{StoreType, load_or_create_keypair, open_store};
pub use commands::{ServeInfo, create_local_client_endpoint, create_serve_lock, get_serve_info, is_process_alive, remove_serve_lock};
pub use helpers::{parse_put_spec, parse_get_spec, print_match_cli, print_matches_cli, print_match_repl};

use anyhow::Result;
use std::path::PathBuf;

// Constants
pub const KEY_FILE: &str = ".iroh-key";
pub const CLIENT_KEY_FILE: &str = ".iroh-key-client";
pub const STORE_PATH: &str = ".iroh-store";
pub const SERVE_LOCK: &str = ".iroh-serve.lock";
pub const META_ALPN: &[u8] = b"/iroh-meta/1";

/// Convert a path to absolute
pub fn to_absolute(path: &PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.clone())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

// shell_capture is in repl::input and re-exported from repl

/// Helper function for matching (used by find/search)
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

/// Check if a string looks like a node ID (64 hex chars)
pub fn is_node_id(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Parse items from stdin, splitting on newline, tab, or comma
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

/// Read input from file path or stdin
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

/// Export blob to file or stdout
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
        let path = PathBuf::from("relative/path/file.txt");
        let result = to_absolute(&path).unwrap();
        assert!(result.is_absolute());
        assert!(result.ends_with("relative/path/file.txt"));
    }

    #[test]
    fn test_to_absolute_current_dir() {
        let path = PathBuf::from(".");
        let result = to_absolute(&path).unwrap();
        assert!(result.is_absolute());
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
