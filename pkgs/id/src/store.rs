//! Blob storage and keypair management.
//!
//! This module provides the storage layer for the ID system, handling both
//! content-addressed blob storage and Ed25519 keypair management for node
//! identity.
//!
//! # Storage Types
//!
//! Two storage modes are supported via [`StoreType`]:
//!
//! - **Persistent** ([`FsStore`]): SQLite-backed storage in `.iroh-store/`.
//!   Data survives restarts. Only one process can access at a time.
//!
//! - **Ephemeral** ([`MemStore`]): In-memory storage that is discarded on
//!   shutdown. Useful for testing or temporary operations.
//!
//! # Keypair Management
//!
//! Node identity is based on Ed25519 keypairs. The [`load_or_create_keypair`]
//! function handles lazy initialization - it creates a new keypair if the file
//! doesn't exist, or loads an existing one.
//!
//! # Example
//!
//! ```rust,ignore
//! use id::{open_store, load_or_create_keypair, KEY_FILE};
//!
//! # async fn example() -> anyhow::Result<()> {
//! // Open persistent storage
//! let store = open_store(false).await?;
//!
//! // Load or create node identity
//! let keypair = load_or_create_keypair(KEY_FILE).await?;
//! println!("Node ID: {}", keypair.public());
//!
//! // Use the store...
//! let api = store.as_store();
//! let hash = api.blobs().add_bytes(b"Hello".to_vec()).await?;
//! api.tags().set("greeting", hash.hash).await?;
//!
//! // Clean shutdown
//! store.shutdown().await?;
//! # Ok(())
//! # }
//! ```

use anyhow::{Result, anyhow};
use iroh_base::SecretKey;
use iroh_blobs::{
    api::Store,
    store::{fs::FsStore, mem::MemStore},
};
use tokio::fs as afs;

use crate::STORE_PATH;

/// Loads an existing Ed25519 keypair from a file, or creates a new one.
///
/// This function provides lazy initialization for node identity:
/// - If the file exists, it reads and parses the 32-byte secret key
/// - If the file doesn't exist, it generates a new random keypair and saves it
///
/// The keypair file contains just the 32-byte secret key in raw binary format.
/// The public key (node ID) can be derived from this.
///
/// # Arguments
///
/// * `path` - Path to the keypair file
///
/// # Returns
///
/// The Ed25519 secret key, which can be used to derive the public key (node ID).
///
/// # Errors
///
/// - Returns an error if the file exists but has invalid length (not 32 bytes)
/// - Returns an error if the file cannot be read or written
///
/// # Example
///
/// ```rust,ignore
/// use id::load_or_create_keypair;
///
/// # async fn example() -> anyhow::Result<()> {
/// let key = load_or_create_keypair(".my-key").await?;
/// println!("Public key (node ID): {}", key.public());
/// # Ok(())
/// # }
/// ```
pub async fn load_or_create_keypair(path: &str) -> Result<SecretKey> {
    match afs::read(path).await {
        Ok(bytes) => {
            let bytes: [u8; 32] = bytes
                .try_into()
                .map_err(|_| anyhow!("invalid key length"))?;
            Ok(SecretKey::from(bytes))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let key = SecretKey::generate(&mut rand::rng());
            afs::write(path, key.to_bytes()).await?;
            Ok(key)
        }
        Err(e) => Err(e.into()),
    }
}

/// Wrapper enum for persistent vs ephemeral blob stores.
///
/// This enum provides a unified interface over the two storage backends
/// supported by iroh-blobs. It allows the rest of the application to
/// work with either storage type transparently.
///
/// # Variants
///
/// * `Persistent` - File-system backed SQLite storage. Data survives restarts.
/// * `Ephemeral` - In-memory storage. Data is lost on shutdown.
///
/// # Example
///
/// ```rust,ignore
/// use id::{open_store, StoreType};
///
/// # async fn example() -> anyhow::Result<()> {
/// // Ephemeral for testing
/// let test_store = open_store(true).await?;
/// assert!(matches!(test_store, StoreType::Ephemeral(_)));
///
/// // Persistent for production
/// let prod_store = open_store(false).await?;
/// assert!(matches!(prod_store, StoreType::Persistent(_)));
/// # Ok(())
/// # }
/// ```
pub enum StoreType {
    /// File-system backed persistent storage using SQLite.
    ///
    /// Data is stored in the `.iroh-store/` directory. Only one process
    /// can access the database at a time due to SQLite locking.
    Persistent(FsStore),
    
    /// In-memory ephemeral storage.
    ///
    /// Useful for testing, temporary operations, or when you don't need
    /// data to persist. All data is lost when the store is shutdown.
    Ephemeral(MemStore),
}

impl StoreType {
    /// Gets a [`Store`] handle from this storage type.
    ///
    /// The returned `Store` provides the main API for blob and tag operations.
    /// This handle is cheaply cloneable.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use id::open_store;
    ///
    /// # async fn example() -> anyhow::Result<()> {
    /// let store_type = open_store(true).await?;
    /// let store = store_type.as_store();
    ///
    /// // Now use the store API
    /// let blobs = store.blobs();
    /// let tags = store.tags();
    /// # Ok(())
    /// # }
    /// ```
    pub fn as_store(&self) -> Store {
        match self {
            StoreType::Persistent(s) => s.clone().into(),
            StoreType::Ephemeral(s) => s.clone().into(),
        }
    }

    /// Shuts down the store gracefully.
    ///
    /// For persistent stores, this ensures all data is flushed to disk
    /// and the database is properly closed. For ephemeral stores, this
    /// simply releases the memory.
    ///
    /// **Important**: Always call this before dropping the store to ensure
    /// data integrity, especially for persistent stores.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use id::open_store;
    ///
    /// # async fn example() -> anyhow::Result<()> {
    /// let store = open_store(false).await?;
    /// // ... use the store ...
    /// store.shutdown().await?; // Clean shutdown
    /// # Ok(())
    /// # }
    /// ```
    pub async fn shutdown(self) -> Result<()> {
        match self {
            StoreType::Persistent(s) => s.shutdown().await?,
            StoreType::Ephemeral(s) => s.shutdown().await?,
        }
        Ok(())
    }
}

/// Opens a blob store, either persistent or ephemeral.
///
/// This is the main entry point for creating storage. Persistent stores
/// use the `.iroh-store/` directory in the current working directory.
///
/// # Arguments
///
/// * `ephemeral` - If `true`, creates an in-memory store. If `false`,
///   opens/creates a persistent SQLite-backed store.
///
/// # Returns
///
/// A [`StoreType`] wrapper that can be used to access the blob store.
///
/// # Errors
///
/// For persistent stores, returns an error if:
/// - The database file is locked by another process
/// - The database is corrupted
/// - Disk I/O fails
///
/// # Example
///
/// ```rust,ignore
/// use id::open_store;
///
/// # async fn example() -> anyhow::Result<()> {
/// // For production use
/// let store = open_store(false).await?;
///
/// // For testing
/// let test_store = open_store(true).await?;
/// # Ok(())
/// # }
/// ```
pub async fn open_store(ephemeral: bool) -> Result<StoreType> {
    if ephemeral {
        Ok(StoreType::Ephemeral(MemStore::new()))
    } else {
        let store = FsStore::load(STORE_PATH).await?;
        Ok(StoreType::Persistent(store))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_lite::StreamExt;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_ephemeral_store() {
        let store = open_store(true).await.unwrap();
        assert!(matches!(store, StoreType::Ephemeral(_)));
        store.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_ephemeral_store_add_blob() {
        let store_type = open_store(true).await.unwrap();
        let store = store_type.as_store();
        
        // Add a blob
        let data = b"hello world";
        let result = store.blobs().add_bytes(data.to_vec()).await.unwrap();
        
        // Verify we can read it back
        let read_data = store.blobs().get_bytes(result.hash).await.unwrap();
        assert_eq!(read_data.as_ref(), data);
        
        store_type.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_ephemeral_store_tags() {
        let store_type = open_store(true).await.unwrap();
        let store = store_type.as_store();
        
        // Add a blob
        let data = b"test content";
        let result = store.blobs().add_bytes(data.to_vec()).await.unwrap();
        
        // Set a tag
        store.tags().set("test-tag", result.hash).await.unwrap();
        
        // Read tag back
        let tag = store.tags().get("test-tag").await.unwrap();
        assert!(tag.is_some());
        assert_eq!(tag.unwrap().hash, result.hash);
        
        store_type.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_ephemeral_store_list_tags() {
        let store_type = open_store(true).await.unwrap();
        let store = store_type.as_store();
        
        // Add blobs and tags
        let data1 = b"content 1";
        let data2 = b"content 2";
        let result1 = store.blobs().add_bytes(data1.to_vec()).await.unwrap();
        let result2 = store.blobs().add_bytes(data2.to_vec()).await.unwrap();
        
        store.tags().set("tag1", result1.hash).await.unwrap();
        store.tags().set("tag2", result2.hash).await.unwrap();
        
        // List tags
        let mut list = store.tags().list().await.unwrap();
        let mut tags = Vec::new();
        while let Some(item) = list.next().await {
            let item = item.unwrap();
            let name = String::from_utf8_lossy(item.name.as_ref()).to_string();
            tags.push(name);
        }
        
        assert!(tags.contains(&"tag1".to_string()));
        assert!(tags.contains(&"tag2".to_string()));
        
        store_type.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_ephemeral_store_delete_tag() {
        let store_type = open_store(true).await.unwrap();
        let store = store_type.as_store();
        
        // Add a blob and tag
        let data = b"test";
        let result = store.blobs().add_bytes(data.to_vec()).await.unwrap();
        store.tags().set("to-delete", result.hash).await.unwrap();
        
        // Verify it exists
        assert!(store.tags().get("to-delete").await.unwrap().is_some());
        
        // Delete it
        store.tags().delete("to-delete").await.unwrap();
        
        // Verify it's gone
        assert!(store.tags().get("to-delete").await.unwrap().is_none());
        
        store_type.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_load_or_create_keypair_creates_new() {
        let tmp_dir = TempDir::new().unwrap();
        let key_path = tmp_dir.path().join("test-key");
        let key_path_str = key_path.to_str().unwrap();
        
        // Key shouldn't exist
        assert!(!key_path.exists());
        
        // Create it
        let key1 = load_or_create_keypair(key_path_str).await.unwrap();
        
        // File should now exist
        assert!(key_path.exists());
        
        // Loading again should return same key
        let key2 = load_or_create_keypair(key_path_str).await.unwrap();
        assert_eq!(key1.to_bytes(), key2.to_bytes());
    }

    #[tokio::test]
    async fn test_load_or_create_keypair_loads_existing() {
        let tmp_dir = TempDir::new().unwrap();
        let key_path = tmp_dir.path().join("existing-key");
        let key_path_str = key_path.to_str().unwrap();
        
        // Create a key manually
        let original_key = SecretKey::generate(&mut rand::rng());
        std::fs::write(&key_path, original_key.to_bytes()).unwrap();
        
        // Load it
        let loaded_key = load_or_create_keypair(key_path_str).await.unwrap();
        
        assert_eq!(original_key.to_bytes(), loaded_key.to_bytes());
    }

    #[tokio::test]
    async fn test_load_or_create_keypair_invalid_length() {
        let tmp_dir = TempDir::new().unwrap();
        let key_path = tmp_dir.path().join("bad-key");
        let key_path_str = key_path.to_str().unwrap();
        
        // Write invalid key (wrong length)
        std::fs::write(&key_path, b"too short").unwrap();
        
        // Should fail
        let result = load_or_create_keypair(key_path_str).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid key length"));
    }

    #[tokio::test]
    async fn test_store_type_as_store() {
        let ephemeral = open_store(true).await.unwrap();
        let _store = ephemeral.as_store(); // Should not panic
        ephemeral.shutdown().await.unwrap();
    }
}
