//! Store module - handles blob storage and keypair management

use anyhow::{Result, anyhow};
use iroh_base::SecretKey;
use iroh_blobs::{
    api::Store,
    store::{fs::FsStore, mem::MemStore},
};
use tokio::fs as afs;

use crate::STORE_PATH;

/// Load or create an Ed25519 keypair from a file
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

/// Wrapper enum for persistent vs ephemeral store types
pub enum StoreType {
    Persistent(FsStore),
    Ephemeral(MemStore),
}

impl StoreType {
    /// Get a Store handle from this StoreType
    pub fn as_store(&self) -> Store {
        match self {
            StoreType::Persistent(s) => s.clone().into(),
            StoreType::Ephemeral(s) => s.clone().into(),
        }
    }

    /// Shutdown the store gracefully
    pub async fn shutdown(self) -> Result<()> {
        match self {
            StoreType::Persistent(s) => s.shutdown().await?,
            StoreType::Ephemeral(s) => s.shutdown().await?,
        }
        Ok(())
    }
}

/// Open a blob store (persistent or ephemeral)
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
