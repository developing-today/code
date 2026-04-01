//! Client identity and persistence module.
//!
//! Provides server-side client identity management for collaborative editing.
//! Each client registers with an optional display name and receives an opaque
//! token (base64-encoded signed payload) that can be stored in `localStorage`
//! for session persistence across page reloads and reconnects.
//!
//! # Design
//!
//! ```text
//! Client                              Server (IdentityStore)
//! ------                              ----------------------
//! POST /api/identity/register  -----> generate client_id
//!   { name?: "Alice" }                sign token(client_id, timestamp)
//!                              <----- { token, client_id, name }
//!
//! localStorage.setItem("id_token", token)
//!
//! WS /ws/collab/:id?token=... ------> verify token signature
//!                                     look up client identity
//!                                     populate cursor name
//! ```
//!
//! # Token Format
//!
//! Tokens are Ed25519-signed payloads containing the client UUID and a
//! creation timestamp. The server generates a keypair at startup and uses
//! it to sign/verify tokens. Tokens are base64url-encoded for safe use
//! in query parameters and `localStorage`.
//!
//! # Thread Safety
//!
//! The [`IdentityStore`] is designed for concurrent access using `Arc<RwLock<>>`
//! internally. Clone it freely across handlers.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tokio::sync::watch;

/// Maximum allowed display name length (bytes).
const MAX_NAME_LENGTH: usize = 64;

/// Maximum token age before it's considered expired (30 days in seconds).
const TOKEN_MAX_AGE_SECS: u64 = 30 * 24 * 60 * 60;

/// A client's identity information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientIdentity {
    /// Unique client identifier (hex-encoded random bytes, 32 chars).
    pub client_id: String,
    /// Optional display name chosen by the user.
    pub name: Option<String>,
    /// Unix timestamp (seconds) when this identity was created.
    pub created_at: u64,
    /// Unix timestamp (seconds) when the name was last updated.
    pub updated_at: u64,
}

/// The raw payload embedded in a signed token.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TokenPayload {
    /// The client's unique identifier.
    client_id: String,
    /// Unix timestamp (seconds) when the token was created.
    created_at: u64,
}

/// In-memory store for client identities.
///
/// Keyed by `client_id` (a hex string). Thread-safe via `Arc<RwLock<>>`.
#[derive(Debug, Clone)]
pub struct IdentityStore {
    /// Map of `client_id` to identity.
    clients: Arc<RwLock<HashMap<String, ClientIdentity>>>,
    /// Watch channels for name changes, keyed by `client_id`.
    /// WS handlers subscribe to get notified when a client's name changes.
    name_watchers: Arc<RwLock<HashMap<String, watch::Sender<Option<String>>>>>,
    /// Ed25519 signing key (generated at startup, used to sign tokens).
    signing_key: Arc<SigningKey>,
    /// Ed25519 verifying key (derived from signing key, used to verify tokens).
    verifying_key: VerifyingKey,
}

impl IdentityStore {
    /// Create a new identity store with a fresh Ed25519 keypair.
    pub fn new() -> Self {
        let signing_key = SigningKey::generate(&mut rand::rng());
        let verifying_key = signing_key.verifying_key();
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
            name_watchers: Arc::new(RwLock::new(HashMap::new())),
            signing_key: Arc::new(signing_key),
            verifying_key,
        }
    }

    /// Register a new client identity.
    ///
    /// Generates a unique client ID, stores the identity, and returns
    /// a signed token + the new identity.
    pub async fn register(
        &self,
        name: Option<String>,
    ) -> anyhow::Result<(String, ClientIdentity)> {
        let name = sanitize_name(name);
        let client_id = generate_client_id();
        let now = unix_timestamp();

        let identity = ClientIdentity {
            client_id: client_id.clone(),
            name: name.clone(),
            created_at: now,
            updated_at: now,
        };

        // Store the identity
        {
            let mut clients = self.clients.write().await;
            clients.insert(client_id.clone(), identity.clone());
        }

        // Create a watch channel for name changes
        {
            let (tx, _rx) = watch::channel(name);
            let mut watchers = self.name_watchers.write().await;
            watchers.insert(client_id.clone(), tx);
        }

        // Create and sign a token
        let token = self.create_token(&client_id, now)?;

        Ok((token, identity))
    }

    /// Verify a token and return the associated client identity (if valid).
    pub async fn verify_token(&self, token: &str) -> Option<ClientIdentity> {
        let payload = self.decode_token(token)?;
        let clients = self.clients.read().await;
        clients.get(&payload.client_id).cloned()
    }

    /// Verify a token and return the `client_id` without looking up the full identity.
    ///
    /// Useful for lightweight checks (e.g., WebSocket auth).
    pub fn verify_token_client_id(&self, token: &str) -> Option<String> {
        let payload = self.decode_token(token)?;
        Some(payload.client_id)
    }

    /// Update a client's display name.
    ///
    /// Returns the updated identity on success, or `None` if the client
    /// was not found.
    pub async fn update_name(
        &self,
        client_id: &str,
        name: Option<String>,
    ) -> Option<ClientIdentity> {
        let name = sanitize_name(name);
        let mut clients = self.clients.write().await;
        let identity = clients.get_mut(client_id)?;
        identity.name = name.clone();
        identity.updated_at = unix_timestamp();
        let result = identity.clone();
        drop(clients);

        // Notify any subscribed WS handlers of the name change
        {
            let watchers = self.name_watchers.read().await;
            if let Some(tx) = watchers.get(client_id) {
                // send_replace never fails (just overwrites the value)
                tx.send_replace(name);
            }
        }

        Some(result)
    }

    /// Look up a client identity by ID.
    pub async fn get(&self, client_id: &str) -> Option<ClientIdentity> {
        let clients = self.clients.read().await;
        clients.get(client_id).cloned()
    }

    /// Look up a client's display name by ID.
    ///
    /// Returns the name if set, or a short hash fallback (first 6 chars of ID).
    pub async fn get_display_name(&self, client_id: &str) -> String {
        let clients = self.clients.read().await;
        clients
            .get(client_id)
            .and_then(|c| c.name.clone())
            .unwrap_or_else(|| short_id(client_id))
    }

    /// Subscribe to name changes for a client.
    ///
    /// Returns a `watch::Receiver` that yields the current name immediately
    /// and notifies on subsequent changes. Returns `None` if the client
    /// is not registered.
    pub async fn subscribe_name(&self, client_id: &str) -> Option<watch::Receiver<Option<String>>> {
        let watchers = self.name_watchers.read().await;
        watchers.get(client_id).map(|tx| tx.subscribe())
    }

    /// Get the number of registered clients.
    pub async fn len(&self) -> usize {
        self.clients.read().await.len()
    }

    /// Check if the store is empty.
    pub async fn is_empty(&self) -> bool {
        self.clients.read().await.is_empty()
    }

    /// Create a signed token for the given client ID.
    fn create_token(&self, client_id: &str, created_at: u64) -> anyhow::Result<String> {
        let payload = TokenPayload {
            client_id: client_id.to_owned(),
            created_at,
        };

        // Serialize payload to JSON
        let payload_bytes = serde_json::to_vec(&payload)?;

        // Sign the payload
        let signature = self.signing_key.sign(&payload_bytes);

        // Combine: payload_len (4 bytes LE) + payload + signature (64 bytes)
        let payload_len = u32::try_from(payload_bytes.len())
            .map_err(|_| anyhow::anyhow!("Token payload too large"))?;
        let mut token_bytes = Vec::with_capacity(4 + payload_bytes.len() + 64);
        token_bytes.extend_from_slice(&payload_len.to_le_bytes());
        token_bytes.extend_from_slice(&payload_bytes);
        token_bytes.extend_from_slice(&signature.to_bytes());

        // Base64url encode (no padding) for safe use in URLs and localStorage
        Ok(base64url_encode(&token_bytes))
    }

    /// Decode and verify a token, returning the payload if valid.
    fn decode_token(&self, token: &str) -> Option<TokenPayload> {
        let token_bytes = base64url_decode(token)?;

        // Need at least 4 bytes for length + 64 bytes for signature
        if token_bytes.len() < 68 {
            return None;
        }

        // Extract payload length
        let payload_len_bytes: [u8; 4] = token_bytes[..4].try_into().ok()?;
        let payload_len = u32::from_le_bytes(payload_len_bytes) as usize;

        // Validate total length: 4 + payload_len + 64
        if token_bytes.len() != 4 + payload_len + 64 {
            return None;
        }

        let payload_bytes = &token_bytes[4..4 + payload_len];
        let signature_bytes = &token_bytes[4 + payload_len..];

        // Verify signature
        let signature = ed25519_dalek::Signature::from_bytes(signature_bytes.try_into().ok()?);
        self.verifying_key
            .verify(payload_bytes, &signature)
            .ok()?;

        // Deserialize payload
        let payload: TokenPayload = serde_json::from_slice(payload_bytes).ok()?;

        // Reject expired tokens (older than TOKEN_MAX_AGE_SECS)
        let now = unix_timestamp();
        if now.saturating_sub(payload.created_at) > TOKEN_MAX_AGE_SECS {
            return None;
        }

        Some(payload)
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Generate a random client ID (16 hex-encoded random bytes = 32 hex chars).
fn generate_client_id() -> String {
    let mut bytes = [0u8; 16];
    rand::fill(&mut bytes);
    hex_encode(&bytes)
}

/// Hex-encode bytes to a lowercase string.
fn hex_encode(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            use std::fmt::Write;
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Sanitize a display name: trim whitespace, enforce max length, return `None` if empty.
fn sanitize_name(name: Option<String>) -> Option<String> {
    name.map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty())
        .map(|n| {
            if n.len() > MAX_NAME_LENGTH {
                // Truncate at char boundary
                let mut end = MAX_NAME_LENGTH;
                while !n.is_char_boundary(end) && end > 0 {
                    end -= 1;
                }
                n[..end].to_owned()
            } else {
                n
            }
        })
}

/// Create a short display ID from a `client_id` (first 6 chars).
pub(crate) fn short_id(client_id: &str) -> String {
    client_id.chars().take(6).collect()
}

/// Get current Unix timestamp in seconds.
fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Base64url encode without padding (RFC 4648 section 5).
fn base64url_encode(data: &[u8]) -> String {
    let mut encoded = String::with_capacity((data.len() * 4 + 2) / 3);
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

    let mut i = 0;
    while i + 2 < data.len() {
        let b0 = data[i] as u32;
        let b1 = data[i + 1] as u32;
        let b2 = data[i + 2] as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        encoded.push(alphabet[((triple >> 18) & 0x3F) as usize] as char);
        encoded.push(alphabet[((triple >> 12) & 0x3F) as usize] as char);
        encoded.push(alphabet[((triple >> 6) & 0x3F) as usize] as char);
        encoded.push(alphabet[(triple & 0x3F) as usize] as char);
        i += 3;
    }

    let remaining = data.len() - i;
    if remaining == 2 {
        let b0 = data[i] as u32;
        let b1 = data[i + 1] as u32;
        let triple = (b0 << 16) | (b1 << 8);
        encoded.push(alphabet[((triple >> 18) & 0x3F) as usize] as char);
        encoded.push(alphabet[((triple >> 12) & 0x3F) as usize] as char);
        encoded.push(alphabet[((triple >> 6) & 0x3F) as usize] as char);
    } else if remaining == 1 {
        let b0 = data[i] as u32;
        let triple = b0 << 16;
        encoded.push(alphabet[((triple >> 18) & 0x3F) as usize] as char);
        encoded.push(alphabet[((triple >> 12) & 0x3F) as usize] as char);
    }

    encoded
}

/// Base64url decode without padding (RFC 4648 section 5).
fn base64url_decode(encoded: &str) -> Option<Vec<u8>> {
    let mut data = Vec::with_capacity(encoded.len() * 3 / 4);

    let decode_char = |c: u8| -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    };

    let bytes = encoded.as_bytes();
    let mut i = 0;

    while i + 3 < bytes.len() {
        let a = decode_char(bytes[i])?;
        let b = decode_char(bytes[i + 1])?;
        let c = decode_char(bytes[i + 2])?;
        let d = decode_char(bytes[i + 3])?;
        let triple = (a << 18) | (b << 12) | (c << 6) | d;
        data.push((triple >> 16) as u8);
        data.push((triple >> 8) as u8);
        data.push(triple as u8);
        i += 4;
    }

    let remaining = bytes.len() - i;
    if remaining == 3 {
        let a = decode_char(bytes[i])?;
        let b = decode_char(bytes[i + 1])?;
        let c = decode_char(bytes[i + 2])?;
        let triple = (a << 18) | (b << 12) | (c << 6);
        data.push((triple >> 16) as u8);
        data.push((triple >> 8) as u8);
    } else if remaining == 2 {
        let a = decode_char(bytes[i])?;
        let b = decode_char(bytes[i + 1])?;
        let triple = (a << 18) | (b << 12);
        data.push((triple >> 16) as u8);
    } else if remaining == 1 {
        // Invalid base64 - single char cannot represent anything
        return None;
    }

    Some(data)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_client_id() {
        let id = generate_client_id();
        assert_eq!(id.len(), 32, "Client ID should be 32 hex chars");
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()), "Should be hex");

        // IDs should be unique
        let id2 = generate_client_id();
        assert_ne!(id, id2, "Should generate unique IDs");
    }

    #[test]
    fn test_sanitize_name() {
        assert_eq!(sanitize_name(None), None);
        assert_eq!(sanitize_name(Some(String::new())), None);
        assert_eq!(sanitize_name(Some("  ".to_owned())), None);
        assert_eq!(
            sanitize_name(Some("  Alice  ".to_owned())),
            Some("Alice".to_owned())
        );
        assert_eq!(
            sanitize_name(Some("Bob".to_owned())),
            Some("Bob".to_owned())
        );

        // Long name truncation
        let long_name = "a".repeat(100);
        let sanitized = sanitize_name(Some(long_name)).unwrap();
        assert!(sanitized.len() <= MAX_NAME_LENGTH);
    }

    #[test]
    fn test_short_id() {
        assert_eq!(short_id("abcdef123456"), "abcdef");
        assert_eq!(short_id("abc"), "abc");
        assert_eq!(short_id(""), "");
    }

    #[test]
    fn test_base64url_roundtrip() {
        let data = b"hello world";
        let encoded = base64url_encode(data);
        let decoded = base64url_decode(&encoded).unwrap();
        assert_eq!(decoded, data);

        // Test with various lengths (padding edge cases)
        for len in 0..=32 {
            let data: Vec<u8> = (0..len).map(|i| i as u8).collect();
            let encoded = base64url_encode(&data);
            let decoded = base64url_decode(&encoded).unwrap();
            assert_eq!(decoded, data, "Failed for len={len}");
        }
    }

    #[test]
    fn test_base64url_no_padding() {
        let encoded = base64url_encode(b"test");
        assert!(!encoded.contains('='), "Should not contain padding");
        assert!(!encoded.contains('+'), "Should use - instead of +");
        assert!(!encoded.contains('/'), "Should use _ instead of /");
    }

    #[tokio::test]
    async fn test_register_and_verify() {
        let store = IdentityStore::new();

        let (token, identity) = store.register(Some("Alice".to_owned())).await.unwrap();
        assert_eq!(identity.name, Some("Alice".to_owned()));
        assert_eq!(identity.client_id.len(), 32);

        // Verify token returns the identity
        let verified = store.verify_token(&token).await.unwrap();
        assert_eq!(verified.client_id, identity.client_id);
        assert_eq!(verified.name, Some("Alice".to_owned()));
    }

    #[tokio::test]
    async fn test_register_without_name() {
        let store = IdentityStore::new();

        let (token, identity) = store.register(None).await.unwrap();
        assert_eq!(identity.name, None);

        let verified = store.verify_token(&token).await.unwrap();
        assert_eq!(verified.name, None);
    }

    #[tokio::test]
    async fn test_invalid_token() {
        let store = IdentityStore::new();

        assert!(store.verify_token("invalid").await.is_none());
        assert!(store.verify_token("").await.is_none());
        assert!(store.verify_token("AAAA").await.is_none());
    }

    #[tokio::test]
    async fn test_token_from_different_store() {
        let store1 = IdentityStore::new();
        let store2 = IdentityStore::new();

        let (token, _) = store1.register(Some("Alice".to_owned())).await.unwrap();

        // Token signed by store1 should not verify on store2
        assert!(store2.verify_token(&token).await.is_none());
    }

    #[tokio::test]
    async fn test_update_name() {
        let store = IdentityStore::new();

        let (_, identity) = store.register(Some("Alice".to_owned())).await.unwrap();

        // Update name
        let updated = store
            .update_name(&identity.client_id, Some("Bob".to_owned()))
            .await
            .unwrap();
        assert_eq!(updated.name, Some("Bob".to_owned()));
        assert!(updated.updated_at >= identity.updated_at);

        // Clear name
        let cleared = store
            .update_name(&identity.client_id, None)
            .await
            .unwrap();
        assert_eq!(cleared.name, None);
    }

    #[tokio::test]
    async fn test_update_nonexistent() {
        let store = IdentityStore::new();
        assert!(
            store
                .update_name("nonexistent", Some("X".to_owned()))
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_get_display_name() {
        let store = IdentityStore::new();

        let (_, identity) = store.register(Some("Alice".to_owned())).await.unwrap();
        assert_eq!(store.get_display_name(&identity.client_id).await, "Alice");

        // Without name, falls back to short ID
        let (_, nameless) = store.register(None).await.unwrap();
        let display = store.get_display_name(&nameless.client_id).await;
        assert_eq!(display.len(), 6);
        assert_eq!(display, short_id(&nameless.client_id));

        // Unknown client also gets short ID
        assert_eq!(
            store.get_display_name("abcdef1234567890").await,
            "abcdef"
        );
    }

    #[tokio::test]
    async fn test_verify_token_client_id() {
        let store = IdentityStore::new();
        let (token, identity) = store.register(None).await.unwrap();

        let client_id = store.verify_token_client_id(&token).unwrap();
        assert_eq!(client_id, identity.client_id);

        assert!(store.verify_token_client_id("bogus").is_none());
    }

    #[test]
    fn test_hex_encode() {
        assert_eq!(hex_encode(&[0x00, 0xff, 0xab]), "00ffab");
        assert_eq!(hex_encode(&[]), "");
    }

    #[tokio::test]
    async fn test_store_len() {
        let store = IdentityStore::new();
        assert!(store.is_empty().await);
        assert_eq!(store.len().await, 0);

        store.register(None).await.unwrap();
        assert!(!store.is_empty().await);
        assert_eq!(store.len().await, 1);

        store.register(None).await.unwrap();
        assert_eq!(store.len().await, 2);
    }

    #[tokio::test]
    async fn test_name_watch_channel() {
        let store = IdentityStore::new();
        let (_token, identity) = store.register(Some("Alice".to_owned())).await.unwrap();
        let cid = &identity.client_id;

        // Subscribe to name changes
        let mut rx = store.subscribe_name(cid).await.unwrap();

        // Initial value is the name at registration time
        assert_eq!(*rx.borrow(), Some("Alice".to_owned()));
        assert!(!rx.has_changed().unwrap());

        // Update the name
        store.update_name(cid, Some("Bob".to_owned())).await;
        assert!(rx.has_changed().unwrap());
        assert_eq!(*rx.borrow_and_update(), Some("Bob".to_owned()));

        // Clear the name
        store.update_name(cid, None).await;
        assert!(rx.has_changed().unwrap());
        assert_eq!(*rx.borrow_and_update(), None);

        // No further changes
        assert!(!rx.has_changed().unwrap());
    }

    #[tokio::test]
    async fn test_name_watch_no_subscription_for_unknown_client() {
        let store = IdentityStore::new();
        assert!(store.subscribe_name("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn test_token_expiry() {
        let store = IdentityStore::new();

        // Register a client normally — token should be valid
        let (token, identity) = store.register(None).await.unwrap();
        assert!(store.verify_token(&token).await.is_some());
        assert!(store.verify_token_client_id(&token).is_some());

        // Create a token with a timestamp older than TOKEN_MAX_AGE_SECS
        let old_timestamp = unix_timestamp().saturating_sub(TOKEN_MAX_AGE_SECS + 1);
        let expired_token = store
            .create_token(&identity.client_id, old_timestamp)
            .unwrap();

        // Expired token should be rejected
        assert!(store.verify_token(&expired_token).await.is_none());
        assert!(store.verify_token_client_id(&expired_token).is_none());
    }
}
