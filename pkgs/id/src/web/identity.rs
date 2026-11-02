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
//! # Persistence
//!
//! With a database path, records live in a redb file with one table, keyed by
//! client ID. Each value is `nonce (12 bytes) || AES-256-GCM ciphertext` of the
//! JSON record. The client ID is the associated data, so a value moved under
//! another key fails to authenticate. The AES key is derived from the server's
//! secret key with HKDF-SHA256.
//!
//! # Thread Safety
//!
//! The [`IdentityStore`] is designed for concurrent access using `Arc<RwLock<>>`
//! internally. Clone it freely across handlers.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit};
use anyhow::Context as _;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use hkdf::Hkdf;
use redb::{Database, DatabaseError, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tokio::sync::RwLock;
use tokio::sync::watch;

/// Maximum allowed display name length (Unicode scalar values).
const MAX_NAME_LENGTH: usize = 48;

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

/// In-memory store for client identities, optionally backed by an encrypted redb file.
///
/// Keyed by `client_id` (a hex string). Thread-safe via `Arc<RwLock<>>`.
/// When backed by a database, mutations are written through to the redb file for
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
    /// Encrypted redb file for persistent storage (`None` for ephemeral/test mode).
    db: Option<Arc<IdentityDb>>,
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
    /// Create a new identity store backed by an encrypted redb file.
    ///
    /// Derives a signing key and an encryption key from the given secret key
    /// bytes using HKDF-SHA256. Record values are encrypted with AES-256-GCM.
    /// All existing identities are authenticated and loaded into memory on
    /// startup. A record that fails authentication fails the open.
    ///
    /// # Arguments
    ///
    /// * `secret_key` - 32-byte secret key (from iroh `SecretKey::to_bytes()`)
    /// * `db_path` - Path for the redb file (e.g., `.identity.redb`)
    pub async fn new(secret_key: [u8; 32], db_path: PathBuf) -> anyhow::Result<Self> {
        let (signing_key, enc_bytes) = derive_keys(&secret_key)?;
        let verifying_key = signing_key.verifying_key();

        // Open the file and load every record. redb is synchronous, so this runs
        // on the blocking pool.
        let (db, identities) = run_blocking(move || IdentityDb::open(&db_path, &enc_bytes)).await?;

        let count = identities.len();
        let mut clients = HashMap::with_capacity(count);
        let mut name_watchers_map = HashMap::with_capacity(count);
        for identity in identities {
            let (tx, _rx) = watch::channel(identity.name.clone());
            name_watchers_map.insert(identity.client_id.clone(), tx);
            clients.insert(identity.client_id.clone(), identity);
        }
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
            let db = Arc::clone(db);
            let record = identity.clone();
            run_blocking(move || db.create(&record)).await?;
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

        // Persist to database (best-effort: in-memory is authoritative during session).
        // The write runs under the lock, so the file ends in the same order as memory.
        if let Some(db) = &self.db {
            let db = Arc::clone(db);
            let record = result.clone();
            if let Err(e) = run_blocking(move || db.update(&record)).await {
                tracing::warn!("[identity] Failed to persist name update: {e}");
            }
        }
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
        self.verifying_key
            .verify_strict(payload_bytes, &signature)
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

/// Sanitize a display name consistently for identities and cursor frames.
pub(super) fn sanitize_name(name: Option<String>) -> Option<String> {
    name.map(|name| {
        name.chars()
            .filter(|ch| !ch.is_control() && !is_bidi_override_or_isolate(*ch))
            .take(MAX_NAME_LENGTH)
            .collect::<String>()
            .trim()
            .to_owned()
    })
    .filter(|name| !name.is_empty())
}

fn is_bidi_override_or_isolate(ch: char) -> bool {
    matches!(
        ch,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
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
// Encrypted Store (redb)
// =============================================================================

/// The one table: client ID to sealed record.
const IDENTITIES: TableDefinition<'static, &str, &[u8]> = TableDefinition::new("identities");

/// AES-256-GCM nonce length. Every stored value starts with its own nonce.
const NONCE_LEN: usize = 12;

/// Run a redb call on tokio's blocking pool. Commits fsync, which must not stall
/// an async worker thread.
async fn run_blocking<T, F>(task: F) -> anyhow::Result<T>
where
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|e| anyhow::anyhow!("identity store task failed: {e}"))?
}

/// The redb file behind an [`IdentityStore`].
///
/// Each value is `nonce || ciphertext`: a fresh random 12-byte nonce, then the
/// AES-256-GCM ciphertext and tag of the JSON record. The client ID is the
/// associated data, so a value moved under another key fails to authenticate.
/// Client IDs stay in plaintext as keys; the record, names included, does not.
struct IdentityDb {
    db: Database,
    cipher: Aes256Gcm,
}

impl IdentityDb {
    /// Open the file at `path`, or create it if it does not exist, then [`load`](Self::load) it.
    ///
    /// A new file is built under a temporary name, so `path` only ever names a file that redb can
    /// open. The first commit makes the file a valid, empty store. It is then renamed into place,
    /// and the directory is synced before the store is returned, so no acknowledged write can
    /// depend on a rename that has not reached the disk. A crash during creation leaves at most a
    /// stale temporary file, which the next creation overwrites.
    fn open(path: &Path, key: &[u8; 32]) -> anyhow::Result<(Self, Vec<ClientIdentity>)> {
        if path.exists() {
            let db = Database::create(path).map_err(|e| store_error(path, e))?;
            return Self::load(db, key);
        }
        let temp = creating_path(path);
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp)
            .with_context(|| format!("failed to create identity store {}", temp.display()))?;
        let db = Database::builder()
            .create_file(file)
            .map_err(|e| store_error(&temp, e))?;
        let created = Self::load(db, key)?;
        std::fs::rename(&temp, path).with_context(|| {
            format!(
                "failed to move identity store {} into place",
                temp.display()
            )
        })?;
        sync_dir(parent_dir(path))
            .with_context(|| format!("failed to sync the directory of {}", path.display()))?;
        Ok(created)
    }

    /// Create the table if the file is new, then authenticate every record.
    ///
    /// A record that fails authentication stops the load: a wrong key or an
    /// altered file must not start a store that silently drops identities. On an
    /// empty file a wrong key cannot be seen until the first record exists.
    fn load(db: Database, key: &[u8; 32]) -> anyhow::Result<(Self, Vec<ClientIdentity>)> {
        let store = Self {
            db,
            cipher: Aes256Gcm::new(&(*key).into()),
        };
        let txn = store.db.begin_write()?;
        txn.open_table(IDENTITIES)?;
        txn.commit()?;
        let records = store.read_all()?;
        Ok((store, records))
    }

    /// Decrypt every record, in no particular order.
    fn read_all(&self) -> anyhow::Result<Vec<ClientIdentity>> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(IDENTITIES)?;
        let mut records = Vec::new();
        for entry in table.iter()? {
            let (client_id, value) = entry?;
            records.push(self.unseal(client_id.value(), value.value())?);
        }
        Ok(records)
    }

    /// Insert a record. A client ID that is already stored is an error (INSERT).
    fn create(&self, identity: &ClientIdentity) -> anyhow::Result<()> {
        let value = self.seal(identity)?;
        let txn = self.db.begin_write()?;
        {
            let mut table = txn.open_table(IDENTITIES)?;
            if table.get(identity.client_id.as_str())?.is_some() {
                anyhow::bail!("identity {} already exists", short_id(&identity.client_id));
            }
            table.insert(identity.client_id.as_str(), value.as_slice())?;
        }
        txn.commit()?;
        Ok(())
    }

    /// Overwrite the record stored under its client ID (UPDATE by key).
    fn update(&self, identity: &ClientIdentity) -> anyhow::Result<()> {
        let value = self.seal(identity)?;
        let txn = self.db.begin_write()?;
        {
            let mut table = txn.open_table(IDENTITIES)?;
            table.insert(identity.client_id.as_str(), value.as_slice())?;
        }
        txn.commit()?;
        Ok(())
    }

    /// Encrypt a record under a fresh random nonce, as `nonce || ciphertext`.
    ///
    /// Nonces are random, not counters, so the store needs no persisted nonce
    /// state. At 96 bits, a collision under one key is negligible for any write
    /// volume an identity store sees.
    fn seal(&self, identity: &ClientIdentity) -> anyhow::Result<Vec<u8>> {
        let plaintext = serde_json::to_vec(identity)?;
        let mut nonce = [0u8; NONCE_LEN];
        rand::fill(&mut nonce);
        let ciphertext = self
            .cipher
            .encrypt(
                &nonce.into(),
                Payload {
                    msg: &plaintext,
                    aad: identity.client_id.as_bytes(),
                },
            )
            .map_err(|_e| {
                anyhow::anyhow!(
                    "failed to encrypt identity {}",
                    short_id(&identity.client_id)
                )
            })?;
        let mut value = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        value.extend_from_slice(&nonce);
        value.extend_from_slice(&ciphertext);
        Ok(value)
    }

    /// Split and decrypt one stored value. `client_id` is its table key.
    fn unseal(&self, client_id: &str, value: &[u8]) -> anyhow::Result<ClientIdentity> {
        let Some((nonce, ciphertext)) = value.split_at_checked(NONCE_LEN) else {
            anyhow::bail!("identity {} is truncated", short_id(client_id));
        };
        let nonce: [u8; NONCE_LEN] = nonce.try_into()?;
        let plaintext = self
            .cipher
            .decrypt(
                &nonce.into(),
                Payload {
                    msg: ciphertext,
                    aad: client_id.as_bytes(),
                },
            )
            .map_err(|_e| {
                anyhow::anyhow!(
                    "identity {} failed authentication: the key is wrong or the file was altered",
                    short_id(client_id)
                )
            })?;
        Ok(serde_json::from_slice(&plaintext)?)
    }
}

/// The signing key for tokens, and the AES-256-GCM key for records, both derived from the secret
/// key with HKDF-SHA256.
fn derive_keys(secret_key: &[u8; 32]) -> anyhow::Result<(SigningKey, [u8; 32])> {
    let hk = Hkdf::<Sha256>::new(None, secret_key);
    let mut signing_bytes = [0u8; 32];
    hk.expand(b"id-identity-signing", &mut signing_bytes)
        .map_err(|_e| anyhow::anyhow!("HKDF expand failed for signing key"))?;
    let mut enc_bytes = [0u8; 32];
    hk.expand(b"id-identity-encryption", &mut enc_bytes)
        .map_err(|_e| anyhow::anyhow!("HKDF expand failed for encryption key"))?;
    Ok((SigningKey::from_bytes(&signing_bytes), enc_bytes))
}

/// The name a new store file is built under: the store's file name with `.creating` appended.
fn creating_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map_or_else(OsString::new, OsStr::to_os_string);
    name.push(".creating");
    path.with_file_name(name)
}

/// The directory that holds `path`. A bare file name lives in the current directory.
fn parent_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    }
}

/// Makes the renames in `dir` durable. Only Unix can open a directory to sync it, so elsewhere
/// this does nothing.
#[cfg(unix)]
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) -> std::io::Result<()> {
    Ok(())
}

fn store_error(path: &Path, error: DatabaseError) -> anyhow::Error {
    match error {
        DatabaseError::DatabaseAlreadyOpen => anyhow::anyhow!(
            "identity store {} is already open in another process or handle",
            path.display()
        ),
        other => anyhow::anyhow!("failed to open identity store {}: {other}", path.display()),
    }
}

/// The crash gate's view of the store (`tests/crash/identity.rs`). Not part of the public API.
///
/// The gate runs the store's own redb operations over a database it opened on its fault model, so
/// a power cut can land inside them. It reopens each disk image through [`open_path`], the file
/// path that [`IdentityStore::new`](super::IdentityStore::new) takes.
#[doc(hidden)]
pub mod crash_gate {
    pub use super::ClientIdentity;
    use super::{IdentityDb, derive_keys};
    use redb::Database;
    use std::path::{Path, PathBuf};

    /// A store opened by the gate.
    pub struct Store(IdentityDb);

    impl std::fmt::Debug for Store {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Store").finish_non_exhaustive()
        }
    }

    impl Store {
        /// Inserts an identity. A client ID that is already stored is an error.
        pub fn create(&self, identity: &ClientIdentity) -> anyhow::Result<()> {
            self.0.create(identity)
        }

        /// Overwrites the identity stored under its client ID.
        pub fn update(&self, identity: &ClientIdentity) -> anyhow::Result<()> {
            self.0.update(identity)
        }

        /// Every stored identity, decrypted and authenticated.
        pub fn records(&self) -> anyhow::Result<Vec<ClientIdentity>> {
            self.0.read_all()
        }
    }

    /// Loads a store from `db`, which the caller opened. A new file gets its table in the first
    /// commit, which this makes.
    pub fn open_database(db: Database, secret_key: &[u8; 32]) -> anyhow::Result<Store> {
        let (_, enc_key) = derive_keys(secret_key)?;
        let (store, _) = IdentityDb::load(db, &enc_key)?;
        Ok(Store(store))
    }

    /// Opens the store file at `path`, or creates it, as
    /// [`IdentityStore::new`](super::IdentityStore::new) does.
    pub fn open_path(path: &Path, secret_key: &[u8; 32]) -> anyhow::Result<Store> {
        let (_, enc_key) = derive_keys(secret_key)?;
        let (store, _) = IdentityDb::open(path, &enc_key)?;
        Ok(Store(store))
    }

    /// The name that a new store file is built under.
    pub fn creating_path(path: &Path) -> PathBuf {
        super::creating_path(path)
    }
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
        // Canonical spelling only (#1840): three characters carry
        // two bytes, so the final character's low 2 bits are unused.
        // The encoder zeroes them; a spelling that sets them decodes
        // to the same bytes, which made every token malleable into
        // alias strings that all verified. Refuse them here, at the
        // single choke point every verify path funnels through.
        if c & 0x03 != 0 {
            return None;
        }
        let triple = (a << 18) | (b << 12) | (c << 6);
        data.push((triple >> 16) as u8);
        data.push((triple >> 8) as u8);
    } else if remaining == 2 {
        let a = decode_char(bytes[i])?;
        let b = decode_char(bytes[i + 1])?;
        // Same refusal for the two-character final group: it carries
        // one byte, so the final character's low 4 bits are unused
        // and must be zero (#1840).
        if b & 0x0F != 0 {
            return None;
        }
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
    use crate::test_forgeries::{IDENTITY_KEY, identity_forgery, with_non_canonical_s};
    use proptest::prelude::*;
    use std::collections::HashSet;

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

        assert_eq!(
            sanitize_name(Some("A\n\t\u{001b}\u{202e}lice".to_owned())),
            Some("Alice".to_owned())
        );
        assert_eq!(
            sanitize_name(Some("A\u{061c}\u{200e}\u{200f}\u{202a}\u{202d}\u{202e}\u{2066}\u{2067}\u{2068}\u{2069}B".to_owned())),
            Some("AB".to_owned())
        );
        assert_eq!(
            sanitize_name(Some("a\u{200d}b\u{200c}c".to_owned())),
            Some("a\u{200d}b\u{200c}c".to_owned())
        );
        assert_eq!(
            sanitize_name(Some("こんにちは世界".to_owned())),
            Some("こんにちは世界".to_owned())
        );

        let long_name = "界".repeat(50);
        let sanitized = sanitize_name(Some(long_name)).unwrap();
        assert_eq!(sanitized.chars().count(), MAX_NAME_LENGTH);
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
    fn test_base64url_decode_refuses_trailing_bit_aliases() {
        // #1840: the final character of an unpadded encoding carries
        // unused low bits (2 for a 3-char final group, 4 for a 2-char
        // one). The canonical encoder zeroes them; a spelling that
        // sets them decodes to the same bytes, so a token had up to
        // 16 alias strings that all verified. Decode refuses the
        // non-canonical spellings outright.
        const ALPHABET: &[u8] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let alias = |encoded: &str, mask: u32| -> String {
            let mut bytes = encoded.as_bytes().to_vec();
            let last = bytes.len() - 1;
            let value = ALPHABET.iter().position(|&c| c == bytes[last]).unwrap() as u32;
            assert_eq!(value & mask, 0, "canonical encoding zeroes the trailing bits");
            bytes[last] = ALPHABET[(value | mask) as usize];
            String::from_utf8(bytes).unwrap()
        };
        // 2-byte input -> 3-char final group, low 2 bits unused.
        let data2 = [0xABu8, 0xCD];
        let enc2 = base64url_encode(&data2);
        assert_eq!(base64url_decode(&enc2).unwrap(), data2);
        assert!(base64url_decode(&alias(&enc2, 0b01)).is_none());
        assert!(base64url_decode(&alias(&enc2, 0b10)).is_none());
        // 1-byte input -> 2-char final group, low 4 bits unused.
        let data1 = [0xABu8];
        let enc1 = base64url_encode(&data1);
        assert_eq!(base64url_decode(&enc1).unwrap(), data1);
        assert!(base64url_decode(&alias(&enc1, 0b0001)).is_none());
        assert!(base64url_decode(&alias(&enc1, 0b1000)).is_none());
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
    async fn test_token_trailing_bit_alias_refused() {
        // #1840 at the token level: respelling a real issued token by
        // setting the final character's unused trailing bits must not
        // verify, while the canonical spelling still does. The token
        // byte length fixes the final group: %4==3 chars -> low 2 bits
        // unused, %4==2 chars -> low 4 bits unused.
        const ALPHABET: &[u8] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let store = IdentityStore::new_ephemeral();
        let (token, _) = store.register(Some("Alice".to_owned())).await.unwrap();
        assert!(store.verify_token(&token).await.is_some());
        let mask: u32 = match token.len() % 4 {
            3 => 0b01,
            2 => 0b0001,
            other => panic!("issued token should end in a partial group, got len%4={other}"),
        };
        let mut bytes = token.as_bytes().to_vec();
        let last = bytes.len() - 1;
        let value = ALPHABET.iter().position(|&c| c == bytes[last]).unwrap() as u32;
        assert_eq!(value & mask, 0, "canonical spelling zeroes the trailing bits");
        bytes[last] = ALPHABET[(value | mask) as usize];
        let alias = String::from_utf8(bytes).unwrap();
        assert_ne!(alias, token);
        assert!(store.verify_token(&alias).await.is_none());
        assert!(store.verify_token_client_id(&alias).is_none());
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

    /// A token with the given payload and 64 signature bytes, in the wire format.
    fn token_of(payload: &[u8], signature: &[u8; 64]) -> String {
        let mut bytes = u32::try_from(payload.len()).unwrap().to_le_bytes().to_vec();
        bytes.extend_from_slice(payload);
        bytes.extend_from_slice(signature);
        base64url_encode(&bytes)
    }

    #[test]
    fn a_token_with_a_non_canonical_s_is_refused() {
        let store = IdentityStore::new_ephemeral();
        let token = store.create_token("abc", unix_timestamp()).unwrap();
        assert!(store.decode_token(&token).is_some());
        let mut bytes = base64url_decode(&token).unwrap();
        let split = bytes.len() - 64;
        let raw: [u8; 64] = bytes[split..].try_into().unwrap();
        bytes[split..].copy_from_slice(&with_non_canonical_s(raw));
        assert!(store.decode_token(&base64url_encode(&bytes)).is_none());
    }

    #[test]
    fn a_token_under_a_small_order_key_is_refused() {
        // The key is derived from the server's secret, so a small-order key
        // cannot arrive through the API. Set it directly to test the check.
        let mut store = IdentityStore::new_ephemeral();
        store.verifying_key = VerifyingKey::from_bytes(&IDENTITY_KEY).unwrap();
        let payload = serde_json::to_vec(&TokenPayload {
            client_id: "abc".to_owned(),
            created_at: unix_timestamp(),
        })
        .unwrap();
        let token = token_of(&payload, &identity_forgery());
        assert!(store.decode_token(&token).is_none());
    }

    // -------------------------------------------------------------------------
    // Encrypted redb store
    // -------------------------------------------------------------------------

    /// Key for the in-memory table tests.
    const TABLE_KEY: [u8; 32] = [3u8; 32];

    /// Secret key for the store tests, which open real files.
    const STORE_SECRET: [u8; 32] = [7u8; 32];

    /// Set only in the child process of `test_second_process_cannot_open_store`.
    const PROBE_PATH_ENV: &str = "ID_IDENTITY_PROBE_PATH";

    fn identity_record(client_id: &str, name: Option<&str>) -> ClientIdentity {
        ClientIdentity {
            client_id: client_id.to_owned(),
            name: name.map(str::to_owned),
            created_at: 1_700_000_000,
            updated_at: 1_700_000_000,
        }
    }

    /// An empty redb file in memory, so the sealing tests never fsync.
    fn in_memory_file() -> Database {
        Database::builder()
            .create_with_backend(redb::backends::InMemoryBackend::new())
            .unwrap()
    }

    fn in_memory_store() -> IdentityDb {
        IdentityDb::load(in_memory_file(), &TABLE_KEY).unwrap().0
    }

    /// The value stored under `client_id`, exactly as the file holds it.
    fn raw_value(store: &IdentityDb, client_id: &str) -> Vec<u8> {
        let txn = store.db.begin_read().unwrap();
        let table = txn.open_table(IDENTITIES).unwrap();
        table.get(client_id).unwrap().unwrap().value().to_vec()
    }

    /// The nonce at the front of the value stored under `client_id`.
    fn raw_nonce(store: &IdentityDb, client_id: &str) -> [u8; NONCE_LEN] {
        raw_value(store, client_id)[..NONCE_LEN].try_into().unwrap()
    }

    #[test]
    fn test_record_round_trips_through_the_table() {
        let (_, empty) = IdentityDb::load(in_memory_file(), &TABLE_KEY).unwrap();
        assert!(empty.is_empty());

        let store = in_memory_store();
        store
            .create(&identity_record("aa11", Some("Alice")))
            .unwrap();
        store.create(&identity_record("bb22", None)).unwrap();

        let mut records = store.read_all().unwrap();
        records.sort_by(|a, b| a.client_id.cmp(&b.client_id));
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].client_id, "aa11");
        assert_eq!(records[0].name.as_deref(), Some("Alice"));
        assert_eq!(records[0].created_at, 1_700_000_000);
        assert_eq!(records[1].name, None);
    }

    #[test]
    fn test_stored_value_hides_the_name() {
        let store = in_memory_store();
        store
            .create(&identity_record("aa11", Some("Alice Wonderland")))
            .unwrap();
        let value = raw_value(&store, "aa11");
        assert!(value.len() > NONCE_LEN + 16, "nonce, record and tag");
        assert!(
            !value.windows(5).any(|w| w == b"Alice"),
            "the name must be encrypted"
        );
    }

    #[test]
    fn test_create_refuses_a_stored_client_id() {
        let store = in_memory_store();
        store
            .create(&identity_record("aa11", Some("Alice")))
            .unwrap();
        let err = store
            .create(&identity_record("aa11", Some("Mallory")))
            .unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");
        assert_eq!(store.read_all().unwrap()[0].name.as_deref(), Some("Alice"));
    }

    #[test]
    fn test_update_overwrites_the_record() {
        let store = in_memory_store();
        store
            .create(&identity_record("aa11", Some("Alice")))
            .unwrap();
        store.update(&identity_record("aa11", Some("Bob"))).unwrap();
        let records = store.read_all().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].name.as_deref(), Some("Bob"));
    }

    #[test]
    fn test_wrong_key_refuses_records_it_did_not_seal() {
        let store = in_memory_store();
        store
            .create(&identity_record("aa11", Some("Alice")))
            .unwrap();
        let IdentityDb { db, .. } = store;
        let err = IdentityDb::load(db, &[4u8; 32]).err().unwrap();
        assert!(err.to_string().contains("failed authentication"), "{err}");
    }

    #[test]
    fn test_altered_value_is_refused() {
        let store = in_memory_store();
        store
            .create(&identity_record("aa11", Some("Alice")))
            .unwrap();
        let value = raw_value(&store, "aa11");
        let opened = store.unseal("aa11", &value).unwrap();
        assert_eq!(opened.name.as_deref(), Some("Alice"));

        // Flipping any single bit of the nonce, ciphertext or tag must fail.
        for byte in 0..value.len() {
            for bit in 0..8u8 {
                let mut altered = value.clone();
                altered[byte] ^= 1u8 << bit;
                assert!(
                    store.unseal("aa11", &altered).is_err(),
                    "byte {byte} bit {bit} was accepted"
                );
            }
        }
        // The same bytes under another client ID: the ID is the associated data.
        assert!(store.unseal("bb22", &value).is_err());
        // Truncated: inside the nonce, and a whole nonce with no tag.
        assert!(store.unseal("aa11", &value[..5]).is_err());
        assert!(store.unseal("aa11", &value[..NONCE_LEN + 15]).is_err());
    }

    #[test]
    fn test_nonces_never_repeat_across_random_writes() {
        proptest!(ProptestConfig::with_cases(64), |(
            writes in prop::collection::vec((0usize..4, prop::option::of("[a-z]{0,12}")), 1..200),
        )| {
            let store = in_memory_store();
            let ids: Vec<String> = (0..4).map(|i| format!("{i:032x}")).collect();
            let mut seen = HashSet::new();
            for id in &ids {
                store.create(&identity_record(id, None)).unwrap();
                prop_assert!(seen.insert(raw_nonce(&store, id)), "nonce reused by create of {id}");
            }
            for (index, name) in &writes {
                let id = &ids[*index];
                store.update(&identity_record(id, name.as_deref())).unwrap();
                prop_assert!(seen.insert(raw_nonce(&store, id)), "nonce reused by update of {id}");
            }
        });
    }

    #[tokio::test]
    async fn test_records_survive_close_and_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.redb");
        let (token, client_id) = {
            let store = IdentityStore::new(STORE_SECRET, path.clone())
                .await
                .unwrap();
            let (token, identity) = store.register(Some("Alice".to_owned())).await.unwrap();
            store
                .update_name(&identity.client_id, Some("Bob".to_owned()))
                .await
                .unwrap();
            (token, identity.client_id)
        };

        let store = IdentityStore::new(STORE_SECRET, path).await.unwrap();
        let reopened = store.verify_token(&token).await.unwrap();
        assert_eq!(reopened.client_id, client_id);
        assert_eq!(reopened.name.as_deref(), Some("Bob"));
        assert_eq!(store.len().await, 1);
    }

    #[tokio::test]
    async fn test_wrong_key_refuses_to_open_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.redb");
        {
            let store = IdentityStore::new(STORE_SECRET, path.clone())
                .await
                .unwrap();
            store.register(Some("Alice".to_owned())).await.unwrap();
        }
        let err = IdentityStore::new([8u8; 32], path).await.unwrap_err();
        assert!(err.to_string().contains("failed authentication"), "{err}");
    }

    #[tokio::test]
    async fn test_creation_leaves_only_the_store_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.redb");
        let store = IdentityStore::new(STORE_SECRET, path.clone())
            .await
            .unwrap();
        assert!(path.exists());
        assert!(!creating_path(&path).exists());
        drop(store);
    }

    #[tokio::test]
    async fn test_stale_creating_file_is_replaced() {
        // What an interrupted creation leaves: bytes under the temporary name, no valid header.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.redb");
        std::fs::write(creating_path(&path), [0xAB; 4096]).unwrap();

        let store = IdentityStore::new(STORE_SECRET, path.clone())
            .await
            .unwrap();
        assert_eq!(store.len().await, 0);
        assert!(!creating_path(&path).exists());
    }

    #[tokio::test]
    async fn test_torn_store_file_is_refused() {
        // Creation never leaves a file at the store's name, so a torn one is not a state the store
        // makes. It is refused with an error, not replaced.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.redb");
        std::fs::write(&path, [0xAB; 4096]).unwrap();

        let err = IdentityStore::new(STORE_SECRET, path).await.unwrap_err();
        assert!(
            err.to_string().contains("failed to open identity store"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn test_tampered_file_refuses_to_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.redb");
        let client_id = {
            let store = IdentityStore::new(STORE_SECRET, path.clone())
                .await
                .unwrap();
            let (_, identity) = store.register(Some("Alice".to_owned())).await.unwrap();
            identity.client_id
        };

        // Flip the last bit of the stored tag, straight through redb.
        let file = Database::create(&path).unwrap();
        let txn = file.begin_write().unwrap();
        {
            let mut table = txn.open_table(IDENTITIES).unwrap();
            let mut value = table
                .get(client_id.as_str())
                .unwrap()
                .unwrap()
                .value()
                .to_vec();
            let last = value.len() - 1;
            value[last] ^= 1;
            table.insert(client_id.as_str(), value.as_slice()).unwrap();
        }
        txn.commit().unwrap();
        drop(file);

        let err = IdentityStore::new(STORE_SECRET, path).await.unwrap_err();
        assert!(err.to_string().contains("failed authentication"), "{err}");
    }

    #[tokio::test]
    async fn test_second_process_cannot_open_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.redb");
        let held = IdentityStore::new(STORE_SECRET, path.clone())
            .await
            .unwrap();
        held.register(Some("Alice".to_owned())).await.unwrap();

        // Run the probe test in a child process, which finds the path in its environment.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "web::identity::tests::test_second_open_probe",
                "--nocapture",
            ])
            .env(PROBE_PATH_ENV, &path)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "probe failed:\n{stdout}");
        assert!(
            stdout.contains("PROBE: identity store")
                && stdout.contains("already open in another process"),
            "the child should be refused with a clear error:\n{stdout}"
        );

        // The refused open leaves the file intact for the process that holds it.
        drop(held);
        let reopened = IdentityStore::new(STORE_SECRET, path).await.unwrap();
        assert_eq!(reopened.len().await, 1);
    }

    /// The child half of [`test_second_process_cannot_open_store`]. It does nothing
    /// unless the probe path is set, so an ordinary test run passes it with no effect.
    #[tokio::test]
    async fn test_second_open_probe() {
        let Some(path) = std::env::var_os(PROBE_PATH_ENV) else {
            return;
        };
        match IdentityStore::new(STORE_SECRET, PathBuf::from(path)).await {
            Ok(_) => println!("PROBE: opened"),
            Err(e) => println!("PROBE: {e}"),
        }
    }
}
