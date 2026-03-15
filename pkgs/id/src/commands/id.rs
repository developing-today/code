//! ID command - display the node's public identity.
//!
//! The `id` command prints the public node ID derived from the keypair.
//! This ID is needed by other nodes to connect and transfer data.
//!
//! # Keypair Management
//!
//! The keypair is stored in `.id-key` and created on first use.
//! The same keypair is used by both `serve` and `id` commands.
//!
//! # Output Format
//!
//! The node ID is a 64-character hexadecimal string representing
//! the Ed25519 public key.
//!
//! # Example
//!
//! ```bash
//! $ id id
//! abc123def456...  # 64 hex characters
//! ```

use anyhow::Result;
use iroh_base::EndpointId;

use crate::store::load_or_create_keypair;
use crate::KEY_FILE;

/// Prints the node ID derived from the local keypair.
///
/// Loads the keypair from [`KEY_FILE`] (creating it if necessary)
/// and prints the public node ID to stdout.
///
/// # Example
///
/// ```rust,ignore
/// cmd_id().await?;
/// // Prints: abc123def456... (64 hex characters)
/// ```
pub async fn cmd_id() -> Result<()> {
    let key = load_or_create_keypair(KEY_FILE).await?;
    let node_id: EndpointId = key.public().into();
    println!("{}", node_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_load_keypair_creates_key_if_needed() {
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("test-key");
        let key_path_str = key_path.to_str().unwrap();

        // Should succeed and create a key file
        let result = load_or_create_keypair(key_path_str).await;
        assert!(result.is_ok());

        // Key file should exist
        assert!(key_path.exists());
    }

    #[tokio::test]
    async fn test_load_keypair_deterministic() {
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("test-key-deterministic");
        let key_path_str = key_path.to_str().unwrap();

        // Get ID twice - should be the same
        let key1 = load_or_create_keypair(key_path_str).await.unwrap();
        let id1: EndpointId = key1.public().into();

        let key2 = load_or_create_keypair(key_path_str).await.unwrap();
        let id2: EndpointId = key2.public().into();

        assert_eq!(id1, id2);
    }
}
