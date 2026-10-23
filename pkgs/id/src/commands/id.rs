//! ID command - display the node's public identity.
//!
//! The `id` command prints the public node ID derived from the keypair.
//! This ID is needed by other nodes to connect and transfer data.
//!
//! # Keypair Management
//!
//! The server keypair is stored in `.iroh-key` and created on first use; the
//! client keypair (used when connecting to other nodes) lives in
//! `.iroh-key-client`. `id id --client` prints the latter, which is what a
//! server owner passes to `id serve --allow-node` to let this machine write.
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
use crate::{CLIENT_KEY_FILE, KEY_FILE};

/// Prints the node ID derived from the local keypair.
///
/// Loads the keypair from [`KEY_FILE`] (or [`CLIENT_KEY_FILE`] when `client`
/// is set), creating it if necessary, and prints the public node ID to stdout.
///
/// # Example
///
/// ```rust,ignore
/// cmd_id(false).await?;
/// // Prints: abc123def456... (64 hex characters)
/// ```
pub async fn cmd_id(client: bool) -> Result<()> {
    let path = if client { CLIENT_KEY_FILE } else { KEY_FILE };
    let key = load_or_create_keypair(path).await?;
    let node_id: EndpointId = key.public();
    println!("{node_id}");
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
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
        let id1: EndpointId = key1.public();

        let key2 = load_or_create_keypair(key_path_str).await.unwrap();
        let id2: EndpointId = key2.public();

        assert_eq!(id1, id2);
    }
}
