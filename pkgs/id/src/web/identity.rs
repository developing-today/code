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
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
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

/// In-memory store for client identities, optionally backed by encrypted `SQLite`.
///
/// Keyed by `client_id` (a hex string). Thread-safe via `Arc<RwLock<>>`.
/// When backed by a database, mutations are written through to `SQLite` for
/// persistence across server restarts.
#[derive(Clone)]
pub struct IdentityStore {
    /// Map of `client_id` to identity.
    clients: Arc<RwLock<HashMap<String, ClientIdentity>>>,
    /// Watch channels for name changes, keyed by `client_id`.
    /// WS handlers subscribe to get notified when a client's name changes.
    name_watchers: Arc<RwLock<HashMap<String, watch::Sender<Option<String>>>>>,
    /// Ed25519 signing key (derived from iroh secret key, or random for tests).
    signing_key: Arc<SigningKey>,
    /// Ed25519 verifying key (derived from signing key, used to verify tokens).
    verifying_key: VerifyingKey,
    /// `SQLite` database for persistent storage (`None` for ephemeral/test mode).
    db: Option<Arc<libsql::Database>>,
}

impl std::fmt::Debug for IdentityStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdentityStore")
            .field("clients", &self.clients)
            .field("signing_key", &"<SigningKey>")
            .field("db", &self.db.as_ref().map(|_| "<Database>"))
            .finish_non_exhaustive()
    }
}

impl IdentityStore {
    /// Create a new identity store backed by an encrypted `SQLite` database.
    ///
    /// Derives a signing key and DB encryption key from the given secret key
    /// bytes using HKDF-SHA256. The DB file is encrypted at rest using AES-256-GCM.
    /// All existing identities are loaded into memory on startup.
    ///
    /// # Arguments
    ///
    /// * `secret_key` - 32-byte secret key (from iroh `SecretKey::to_bytes()`)
    /// * `db_path` - Path for the `SQLite` database file (e.g., `.identity.db`)
    pub async fn new(secret_key: [u8; 32], db_path: PathBuf) -> anyhow::Result<Self> {
        // Derive signing key via HKDF-SHA256
        let hk = Hkdf::<Sha256>::new(None, &secret_key);
        let mut signing_bytes = [0u8; 32];
        hk.expand(b"id-identity-signing", &mut signing_bytes)
            .map_err(|_e| anyhow::anyhow!("HKDF expand failed for signing key"))?;
        let signing_key = SigningKey::from_bytes(&signing_bytes);
        let verifying_key = signing_key.verifying_key();

        // Derive DB encryption key via HKDF-SHA256
        let mut enc_bytes = [0u8; 32];
        hk.expand(b"id-identity-encryption", &mut enc_bytes)
            .map_err(|_e| anyhow::anyhow!("HKDF expand failed for encryption key"))?;
        let hex_key = hex_encode(&enc_bytes);

        // Open encrypted SQLite database
        let db_path_str = db_path.display().to_string();
        let uri = format!("file:{db_path_str}?cipher=aes256gcm&hexkey={hex_key}");
        let db = libsql::Builder::new_local(uri)
            .build()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to open identity database: {e}"))?;
        let conn = db
            .connect()
            .map_err(|e| anyhow::anyhow!("Failed to connect to identity database: {e}"))?;

        // Create table if it doesn't exist
        conn.execute(
            "CREATE TABLE IF NOT EXISTS identities (
                client_id TEXT PRIMARY KEY,
                name TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            )",
            (),
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create identities table: {e}"))?;

        // Load existing identities into memory
        let mut clients = HashMap::new();
        let mut name_watchers_map = HashMap::new();
        let mut rows = conn
            .query(
                "SELECT client_id, name, created_at, updated_at FROM identities",
                (),
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to query identities: {e}"))?;

        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read identity row: {e}"))?
        {
            let client_id = db_text(&row, 0)?;
            let name = db_opt_text(&row, 1)?;
            let created_at = db_u64(&row, 2)?;
            let updated_at = db_u64(&row, 3)?;

            let identity = ClientIdentity {
                client_id: client_id.clone(),
                name: name.clone(),
                created_at,
                updated_at,
            };

            let (tx, _rx) = watch::channel(name);
            name_watchers_map.insert(client_id.clone(), tx);
            clients.insert(client_id, identity);
        }

        let count = clients.len();
        if count > 0 {
            tracing::info!("[identity] Loaded {count} identities from database");
        }

        Ok(Self {
            clients: Arc::new(RwLock::new(clients)),
            name_watchers: Arc::new(RwLock::new(name_watchers_map)),
            signing_key: Arc::new(signing_key),
            verifying_key,
            db: Some(Arc::new(db)),
        })
    }

    /// Create an ephemeral identity store with no persistence.
    ///
    /// Generates a random signing key. Identities are lost when the store
    /// is dropped. Used in tests and when persistence is not needed.
    pub fn new_ephemeral() -> Self {
        let signing_key = SigningKey::generate(&mut rand::rng());
        let verifying_key = signing_key.verifying_key();
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
            name_watchers: Arc::new(RwLock::new(HashMap::new())),
            signing_key: Arc::new(signing_key),
            verifying_key,
            db: None,
        }
    }

    /// Register a new client identity.
    ///
    /// Generates a unique client ID, stores the identity, and returns
    /// a signed token + the new identity.
    pub async fn register(&self, name: Option<String>) -> anyhow::Result<(String, ClientIdentity)> {
        let name = sanitize_name(name);
        let client_id = generate_client_id();
        let now = unix_timestamp();

        let identity = ClientIdentity {
            client_id: client_id.clone(),
            name: name.clone(),
            created_at: now,
            updated_at: now,
        };

        // Persist to database first (if available)
        if let Some(db) = &self.db {
            let conn = db
                .connect()
                .map_err(|e| anyhow::anyhow!("DB connect: {e}"))?;
            conn.execute(
                "INSERT INTO identities (client_id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
                db_identity_params(&client_id, name.as_ref(), now, now),
            )
            .await
            .map_err(|e| anyhow::anyhow!("DB insert: {e}"))?;
        }

        // Store the identity in memory
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

    /// Verify a token and return a fresh token + the client identity.
    ///
    /// Used by `/api/identity/me` to renew the token on each page load.
    /// The returned token has a new `created_at`, resetting the 30-day expiry.
    pub async fn verify_and_refresh(&self, token: &str) -> Option<(String, ClientIdentity)> {
        let payload = self.decode_token(token)?;
        let clients = self.clients.read().await;
        let identity = clients.get(&payload.client_id).cloned()?;
        drop(clients);
        let fresh_token = self
            .create_token(&identity.client_id, unix_timestamp())
            .ok()?;
        Some((fresh_token, identity))
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
        identity.name.clone_from(&name);
        identity.updated_at = unix_timestamp();
        let result = identity.clone();
        drop(clients);

        // Persist to database (best-effort — in-memory is authoritative during session)
        if let Some(db) = &self.db
            && let Ok(conn) = db.connect()
        {
            let params = vec![
                name.as_ref()
                    .map_or(libsql::Value::Null, |n| libsql::Value::Text(n.clone())),
                libsql::Value::Integer(i64::try_from(result.updated_at).unwrap_or(0)),
                libsql::Value::Text(client_id.to_owned()),
            ];
            if let Err(e) = conn
                .execute(
                    "UPDATE identities SET name = ?1, updated_at = ?2 WHERE client_id = ?3",
                    params,
                )
                .await
            {
                tracing::warn!("[identity] Failed to persist name update: {e}");
            }
        }

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
        watchers.get(client_id).map(watch::Sender::subscribe)
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
            .map_err(|_e| anyhow::anyhow!("Token payload too large"))?;
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
        self.verifying_key.verify(payload_bytes, &signature).ok()?;

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

// =============================================================================
// Database Helpers
// =============================================================================

/// Extract a TEXT value from a database row.
fn db_text(row: &libsql::Row, idx: i32) -> anyhow::Result<String> {
    match row
        .get_value(idx)
        .map_err(|e| anyhow::anyhow!("Failed to get column {idx}: {e}"))?
    {
        libsql::Value::Text(s) => Ok(s),
        other => anyhow::bail!("Expected text at column {idx}, got {other:?}"),
    }
}

/// Extract an optional TEXT value from a database row.
fn db_opt_text(row: &libsql::Row, idx: i32) -> anyhow::Result<Option<String>> {
    match row
        .get_value(idx)
        .map_err(|e| anyhow::anyhow!("Failed to get column {idx}: {e}"))?
    {
        libsql::Value::Text(s) if !s.is_empty() => Ok(Some(s)),
        libsql::Value::Text(_) | libsql::Value::Null => Ok(None),
        other => anyhow::bail!("Expected text or null at column {idx}, got {other:?}"),
    }
}

/// Extract a u64 value from a database INTEGER column.
fn db_u64(row: &libsql::Row, idx: i32) -> anyhow::Result<u64> {
    match row
        .get_value(idx)
        .map_err(|e| anyhow::anyhow!("Failed to get column {idx}: {e}"))?
    {
        libsql::Value::Integer(n) => Ok(u64::try_from(n).unwrap_or(0)),
        other => anyhow::bail!("Expected integer at column {idx}, got {other:?}"),
    }
}

/// Convert identity fields to database parameter values for INSERT.
fn db_identity_params(
    client_id: &str,
    name: Option<&String>,
    created_at: u64,
    updated_at: u64,
) -> Vec<libsql::Value> {
    vec![
        libsql::Value::Text(client_id.to_owned()),
        name.map_or(libsql::Value::Null, |n| libsql::Value::Text(n.clone())),
        libsql::Value::Integer(i64::try_from(created_at).unwrap_or(0)),
        libsql::Value::Integer(i64::try_from(updated_at).unwrap_or(0)),
    ]
}

/// Base64url encode without padding (RFC 4648 section 5).
fn base64url_encode(data: &[u8]) -> String {
    let mut encoded = String::with_capacity((data.len() * 4).div_ceil(3));
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

    let mut i = 0;
    while i + 2 < data.len() {
        let b0 = u32::from(data[i]);
        let b1 = u32::from(data[i + 1]);
        let b2 = u32::from(data[i + 2]);
        let triple = (b0 << 16) | (b1 << 8) | b2;
        encoded.push(alphabet[((triple >> 18) & 0x3F) as usize] as char);
        encoded.push(alphabet[((triple >> 12) & 0x3F) as usize] as char);
        encoded.push(alphabet[((triple >> 6) & 0x3F) as usize] as char);
        encoded.push(alphabet[(triple & 0x3F) as usize] as char);
        i += 3;
    }

    let remaining = data.len() - i;
    if remaining == 2 {
        let b0 = u32::from(data[i]);
        let b1 = u32::from(data[i + 1]);
        let triple = (b0 << 16) | (b1 << 8);
        encoded.push(alphabet[((triple >> 18) & 0x3F) as usize] as char);
        encoded.push(alphabet[((triple >> 12) & 0x3F) as usize] as char);
        encoded.push(alphabet[((triple >> 6) & 0x3F) as usize] as char);
    } else if remaining == 1 {
        let b0 = u32::from(data[i]);
        let triple = b0 << 16;
        encoded.push(alphabet[((triple >> 18) & 0x3F) as usize] as char);
        encoded.push(alphabet[((triple >> 12) & 0x3F) as usize] as char);
    }

    encoded
}

/// Base64url decode without padding (RFC 4648 section 5).
#[allow(clippy::many_single_char_names, clippy::cast_possible_truncation)]
fn base64url_decode(encoded: &str) -> Option<Vec<u8>> {
    let mut data = Vec::with_capacity(encoded.len() * 3 / 4);

    let decode_char = |c: u8| -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some(u32::from(c - b'A')),
            b'a'..=b'z' => Some(u32::from(c - b'a' + 26)),
            b'0'..=b'9' => Some(u32::from(c - b'0' + 52)),
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
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
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
        let store = IdentityStore::new_ephemeral();

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
        let store = IdentityStore::new_ephemeral();

        let (token, identity) = store.register(None).await.unwrap();
        assert_eq!(identity.name, None);

        let verified = store.verify_token(&token).await.unwrap();
        assert_eq!(verified.name, None);
    }

    #[tokio::test]
    async fn test_invalid_token() {
        let store = IdentityStore::new_ephemeral();

        assert!(store.verify_token("invalid").await.is_none());
        assert!(store.verify_token("").await.is_none());
        assert!(store.verify_token("AAAA").await.is_none());
    }

    #[tokio::test]
    async fn test_token_from_different_store() {
        let store1 = IdentityStore::new_ephemeral();
        let store2 = IdentityStore::new_ephemeral();

        let (token, _) = store1.register(Some("Alice".to_owned())).await.unwrap();

        // Token signed by store1 should not verify on store2
        assert!(store2.verify_token(&token).await.is_none());
    }

    #[tokio::test]
    async fn test_update_name() {
        let store = IdentityStore::new_ephemeral();

        let (_, identity) = store.register(Some("Alice".to_owned())).await.unwrap();

        // Update name
        let updated = store
            .update_name(&identity.client_id, Some("Bob".to_owned()))
            .await
            .unwrap();
        assert_eq!(updated.name, Some("Bob".to_owned()));
        assert!(updated.updated_at >= identity.updated_at);

        // Clear name
        let cleared = store.update_name(&identity.client_id, None).await.unwrap();
        assert_eq!(cleared.name, None);
    }

    #[tokio::test]
    async fn test_update_nonexistent() {
        let store = IdentityStore::new_ephemeral();
        assert!(
            store
                .update_name("nonexistent", Some("X".to_owned()))
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_get_display_name() {
        let store = IdentityStore::new_ephemeral();

        let (_, identity) = store.register(Some("Alice".to_owned())).await.unwrap();
        assert_eq!(store.get_display_name(&identity.client_id).await, "Alice");

        // Without name, falls back to short ID
        let (_, nameless) = store.register(None).await.unwrap();
        let display = store.get_display_name(&nameless.client_id).await;
        assert_eq!(display.len(), 6);
        assert_eq!(display, short_id(&nameless.client_id));

        // Unknown client also gets short ID
        assert_eq!(store.get_display_name("abcdef1234567890").await, "abcdef");
    }

    #[tokio::test]
    async fn test_verify_token_client_id() {
        let store = IdentityStore::new_ephemeral();
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
        let store = IdentityStore::new_ephemeral();
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
        let store = IdentityStore::new_ephemeral();
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
        let store = IdentityStore::new_ephemeral();
        assert!(store.subscribe_name("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn test_token_expiry() {
        let store = IdentityStore::new_ephemeral();

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
