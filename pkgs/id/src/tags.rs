//! Metadata tag system backed by iroh-docs with α/Ω namespace pairs.
//!
//! Tags are stored in iroh-docs documents using tuple-encoded keys for
//! sort-preserving prefix queries. Each logical namespace is an α/Ω pair:
//!
//! - **α (Alpha)**: Primary key order `(subject, key, value|null)`
//! - **Ω (Omega)**: Inverted key order `(value|null, key, subject)`
//!
//! Both entries in a pair point to the same content hash, enabling efficient
//! lookups from either direction.
//!
//! # Namespace types
//!
//! - **Global**: Shared α/Ω pair, visible to all connected peers
//! - **Node**: Per-node α/Ω pair, private by default
//! - **Custom**: User-defined namespaces (single doc or paired)
//!
//! # Key encoding
//!
//! Keys use [`TupleEncoder`] for sort-preserving
//! binary encoding. A tag `(subject="README.md", key="label", value="rust")`
//! becomes:
//!
//! - α key: `tuple("README.md", "label", "rust")`
//! - Ω key: `tuple("rust", "label", "README.md")`
//!
//! Key-only tags (no value) use null as the third field:
//!
//! - α key: `tuple("README.md", "pinned", null)`
//! - Ω key: `tuple(null, "pinned", "README.md")`
//!
//! # Prefix queries
//!
//! - All tags for a file: α prefix `tuple("README.md")`
//! - All values of tag K: α prefix `tuple("README.md", "label")`
//! - All files with value V: Ω prefix `tuple("rust")`
//! - All files with K=V: Ω prefix `tuple("rust", "label")`
//! - All key-only tags: Ω prefix `tuple(null)`

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use futures_lite::StreamExt;
use iroh_blobs::Hash;
use iroh_blobs::api::Store;
use iroh_docs::AuthorId;
use iroh_docs::NamespaceId;
use iroh_docs::api::Doc;
use iroh_docs::protocol::Docs;
use iroh_docs::store::Query;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::tuple::{TupleEncoder, TupleValue, decode};

// ============================================================================
// Tag value type (binary-safe)
// ============================================================================

/// A tag field that stores arbitrary bytes but displays as UTF-8 when possible.
///
/// Wraps `Vec<u8>` and provides:
/// - UTF-8 display when valid, `<binary N bytes>` otherwise
/// - Transparent `Deref<Target=[u8]>` for byte-level operations
/// - `PartialEq<str>` for ergonomic string comparisons
/// - Lossy UTF-8 serialization for JSON (binary data uses replacement char)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TagValue(Vec<u8>);

/// Maximum display length (in bytes) before truncation in UI output.
pub const TAG_DISPLAY_MAX_BYTES: usize = 256;

impl TagValue {
    /// Create from a string.
    pub const fn from_string(s: String) -> Self {
        Self(s.into_bytes())
    }

    /// Create from raw bytes.
    pub const fn from_bytes(b: Vec<u8>) -> Self {
        Self(b)
    }

    /// Try to interpret as a UTF-8 string.
    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }

    /// Check if the bytes are valid UTF-8.
    pub fn is_utf8(&self) -> bool {
        std::str::from_utf8(&self.0).is_ok()
    }

    /// Get the raw bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Consume and return inner bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    /// Display as lossy UTF-8 (replacement chars for invalid sequences).
    pub fn display_lossy(&self) -> String {
        String::from_utf8_lossy(&self.0).into_owned()
    }

    /// Display truncated to max bytes, with UTF-8 try.
    pub fn display_truncated(&self, max: usize) -> String {
        if self.0.len() <= max {
            return self.display_lossy();
        }
        // Find a valid UTF-8 boundary near max
        let mut end = max;
        while end > 0 && !is_char_boundary(&self.0, end) {
            end -= 1;
        }
        let truncated = &self.0[..end];
        let s = String::from_utf8_lossy(truncated);
        format!("{s}...")
    }
}

/// Check if a byte index is a UTF-8 character boundary.
fn is_char_boundary(bytes: &[u8], index: usize) -> bool {
    if index == 0 || index >= bytes.len() {
        return true;
    }
    // A byte is a char boundary if it's not a continuation byte (10xxxxxx)
    bytes[index] & 0xC0 != 0x80
}

impl std::ops::Deref for TagValue {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Display for TagValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match std::str::from_utf8(&self.0) {
            Ok(s) => write!(f, "{s}"),
            Err(_) => write!(f, "<binary {} bytes>", self.0.len()),
        }
    }
}

impl PartialEq<str> for TagValue {
    fn eq(&self, other: &str) -> bool {
        self.0 == other.as_bytes()
    }
}

impl PartialEq<&str> for TagValue {
    fn eq(&self, other: &&str) -> bool {
        self.0 == other.as_bytes()
    }
}

impl From<String> for TagValue {
    fn from(s: String) -> Self {
        Self(s.into_bytes())
    }
}

impl From<&str> for TagValue {
    fn from(s: &str) -> Self {
        Self(s.as_bytes().to_vec())
    }
}

impl From<Vec<u8>> for TagValue {
    fn from(b: Vec<u8>) -> Self {
        Self(b)
    }
}

impl AsRef<[u8]> for TagValue {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Serialize for TagValue {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&String::from_utf8_lossy(&self.0))
    }
}

// ============================================================================
// Constants
// ============================================================================

/// Directory name for iroh-docs metadata storage.
const META_DIR: &str = ".iroh-meta";

/// Registry filename within the meta directory.
const REGISTRY_FILE: &str = "registry.json";

/// Current registry schema version.
const REGISTRY_VERSION: u32 = 1;

// ============================================================================
// Registry types (persisted as JSON)
// ============================================================================

/// Persistent registry mapping logical namespace names to iroh-docs `NamespaceId`s.
///
/// Stored at `{working_dir}/.iroh-meta/registry.json`. Since `iroh-docs` assigns
/// random `NamespaceId`s on `create()`, we must persist the mapping to reopen
/// the same documents across restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Registry {
    /// Schema version for forward compatibility.
    version: u32,
    /// Global α/Ω namespace pair IDs.
    global: Option<PairIds>,
    /// Per-node namespace pairs, keyed by node ID (hex).
    #[serde(default)]
    nodes: HashMap<String, PairIds>,
    /// Custom user-defined namespaces.
    #[serde(default)]
    custom: HashMap<String, CustomEntry>,
}

/// Stored IDs for an α/Ω pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PairIds {
    /// α (alpha) namespace ID as hex string.
    alpha: String,
    /// Ω (omega) namespace ID as hex string.
    omega: String,
}

/// A custom namespace entry — either a single doc or an α/Ω pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum CustomEntry {
    /// A single unpaired document.
    #[serde(rename = "single")]
    Single {
        /// Namespace ID as hex string.
        id: String,
    },
    /// An α/Ω paired namespace.
    #[serde(rename = "paired")]
    Paired(PairIds),
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            version: REGISTRY_VERSION,
            global: None,
            nodes: HashMap::new(),
            custom: HashMap::new(),
        }
    }
}

// ============================================================================
// Namespace pair
// ============================================================================

/// An α/Ω namespace pair with open document handles.
///
/// α uses primary key order `(subject, key, value|null)`.
/// Ω uses inverted order `(value|null, key, subject)`.
///
/// Both entries in a pair point to the same content hash for deduplication.
pub struct NamespacePair {
    /// α (alpha) document — primary key order.
    pub alpha: Doc,
    /// Ω (omega) document — inverted key order.
    pub omega: Doc,
    /// α namespace ID.
    pub alpha_id: NamespaceId,
    /// Ω namespace ID.
    pub omega_id: NamespaceId,
}

impl std::fmt::Debug for NamespacePair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NamespacePair")
            .field("alpha_id", &self.alpha_id)
            .field("omega_id", &self.omega_id)
            .finish_non_exhaustive()
    }
}

// ============================================================================
// Custom namespace
// ============================================================================

/// A custom user-defined namespace — either a single doc or an α/Ω pair.
pub enum CustomNamespace {
    /// A single unpaired document.
    Single(Doc),
    /// An α/Ω paired namespace.
    Paired(NamespacePair),
}

impl std::fmt::Debug for CustomNamespace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Single(_) => f.debug_tuple("Single").field(&"<Doc>").finish(),
            Self::Paired(pair) => f.debug_tuple("Paired").field(pair).finish(),
        }
    }
}

// ============================================================================
// Decoded tag
// ============================================================================

/// A decoded tag entry from a namespace.
///
/// All string-like fields use [`TagValue`] which stores arbitrary bytes
/// but tries to display as UTF-8. This allows tags with binary keys/values
/// while remaining ergonomic for the common string case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    /// The entity this tag describes (usually a filename).
    pub subject: TagValue,
    /// The tag key (e.g., "label", "created", "pinned").
    pub key: TagValue,
    /// Optional value. `None` for key-only tags like "pinned".
    pub value: Option<TagValue>,
    /// Content hash of the entry data.
    pub hash: Hash,
    /// Entry timestamp (microseconds since epoch, from iroh-docs).
    pub timestamp: u64,
    /// Author who wrote this entry.
    pub author: AuthorId,
}

// ============================================================================
// Tag events (broadcast to WebSocket clients)
// ============================================================================

/// A tag change event broadcast to WebSocket subscribers.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum TagEvent {
    /// A tag was set or updated.
    #[serde(rename = "set")]
    Set {
        /// Namespace name ("global", "node", or custom name).
        ns: String,
        /// Subject (file/entity) this tag belongs to.
        subject: String,
        /// Tag key (e.g. "label", "status").
        key: String,
        /// Optional tag value.
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<String>,
    },
    /// A tag was deleted.
    #[serde(rename = "del")]
    Del {
        /// Namespace name.
        ns: String,
        /// Subject the tag was deleted from.
        subject: String,
        /// Tag key that was deleted.
        key: String,
        /// Optional tag value that was deleted.
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<String>,
    },
    /// All tags for a subject were deleted.
    #[serde(rename = "del_all")]
    DelAll {
        /// Namespace name.
        ns: String,
        /// Subject whose tags were all deleted.
        subject: String,
    },
    /// Tags were transferred from one subject to another.
    #[serde(rename = "transfer")]
    Transfer {
        /// Namespace name.
        ns: String,
        /// Source subject.
        from: String,
        /// Destination subject.
        to: String,
        /// Number of tags transferred.
        count: usize,
    },
}

/// Capacity of the tag event broadcast channel.
const TAG_BROADCAST_CAPACITY: usize = 256;

// ============================================================================
// Tag store
// ============================================================================

/// Metadata tag store backed by iroh-docs.
///
/// Manages α/Ω namespace pairs for global, per-node, and custom namespaces.
/// All tag operations write to both α and Ω documents to maintain the
/// inverted index.
///
/// # Initialization
///
/// ```rust,ignore
/// let tag_store = TagStore::init(&docs, "node_id_hex").await?;
/// tag_store.set_tag(&tag_store.global, "README.md", "label", Some("rust"), b"").await?;
/// ```
pub struct TagStore {
    /// Global α/Ω namespace pair (shared across peers).
    pub global: NamespacePair,
    /// Per-node α/Ω namespace pair (private by default).
    pub node: NamespacePair,
    /// Custom user-defined namespaces.
    pub custom: HashMap<String, CustomNamespace>,
    /// Author ID for writing entries.
    pub author: AuthorId,
    /// Path to the registry file.
    registry_path: PathBuf,
    /// Broadcast sender for tag change events.
    broadcast: broadcast::Sender<TagEvent>,
}

impl std::fmt::Debug for TagStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TagStore")
            .field("global", &self.global)
            .field("node", &self.node)
            .field("custom_count", &self.custom.len())
            .field("author", &self.author)
            .finish_non_exhaustive()
    }
}

impl TagStore {
    /// Subscribe to tag change events.
    ///
    /// Returns a broadcast receiver that yields [`TagEvent`]s when tags are
    /// set, deleted, or transferred. Used by the WebSocket handler to push
    /// live updates to connected clients.
    pub fn subscribe(&self) -> broadcast::Receiver<TagEvent> {
        self.broadcast.subscribe()
    }

    /// Emit a tag event to all subscribers.
    ///
    /// Silently ignores send failures (no subscribers connected).
    fn emit(&self, event: TagEvent) {
        let _ = self.broadcast.send(event);
    }

    /// Resolve which namespace name a given pair belongs to.
    ///
    /// Returns `"global"`, `"node"`, or the custom namespace name.
    pub fn ns_name(&self, ns: &NamespacePair) -> String {
        if ns.alpha_id == self.global.alpha_id {
            "global".to_owned()
        } else if ns.alpha_id == self.node.alpha_id {
            "node".to_owned()
        } else {
            for (name, custom) in &self.custom {
                if let CustomNamespace::Paired(pair) = custom
                    && pair.alpha_id == ns.alpha_id
                {
                    return name.clone();
                }
            }
            "unknown".to_owned()
        }
    }
}

// ============================================================================
// Key encoding
// ============================================================================

/// Smart-encode a byte slice: uses `string()` for valid UTF-8 (backward
/// compatible, type tag 0x02), `bytes()` for non-UTF-8 (type tag 0x01).
fn encode_smart<'a>(enc: &'a mut TupleEncoder, data: &[u8]) -> &'a mut TupleEncoder {
    match std::str::from_utf8(data) {
        Ok(s) => enc.string(s),
        Err(_) => enc.bytes(data),
    }
}

/// Extract bytes from a decoded `TupleValue`, handling both String and Bytes variants.
fn tuple_value_to_tag_value(tv: &TupleValue) -> Option<TagValue> {
    match tv {
        TupleValue::String(s) => Some(TagValue::from(s.as_str())),
        TupleValue::Bytes(b) => Some(TagValue::from(b.clone())),
        _ => None,
    }
}

/// Encode an α (alpha) key: `(subject, key, value|null)`.
///
/// Uses smart encoding: UTF-8 bytes → `string()` (backward compatible),
/// non-UTF-8 bytes → `bytes()` (new capability).
fn encode_alpha_key(subject: &[u8], key: &[u8], value: Option<&[u8]>) -> Vec<u8> {
    let mut enc = TupleEncoder::new();
    encode_smart(&mut enc, subject);
    encode_smart(&mut enc, key);
    match value {
        Some(v) => {
            encode_smart(&mut enc, v);
        }
        None => {
            enc.null();
        }
    }
    enc.build()
}

/// Encode an Ω (omega) key: `(value|null, key, subject)` — inverted order.
fn encode_omega_key(subject: &[u8], key: &[u8], value: Option<&[u8]>) -> Vec<u8> {
    let mut enc = TupleEncoder::new();
    match value {
        Some(v) => {
            encode_smart(&mut enc, v);
        }
        None => {
            enc.null();
        }
    }
    encode_smart(&mut enc, key);
    encode_smart(&mut enc, subject);
    enc.build()
}

/// Encode a prefix for α queries on a subject.
fn encode_alpha_subject_prefix(subject: &[u8]) -> Vec<u8> {
    let mut enc = TupleEncoder::new();
    encode_smart(&mut enc, subject);
    enc.build()
}

/// Encode a prefix for α queries on subject + key.
fn encode_alpha_subject_key_prefix(subject: &[u8], key: &[u8]) -> Vec<u8> {
    let mut enc = TupleEncoder::new();
    encode_smart(&mut enc, subject);
    encode_smart(&mut enc, key);
    enc.build()
}

/// Encode a prefix for Ω queries on a value.
fn encode_omega_value_prefix(value: &[u8]) -> Vec<u8> {
    let mut enc = TupleEncoder::new();
    encode_smart(&mut enc, value);
    enc.build()
}

/// Encode a prefix for Ω queries on null (key-only tags).
fn encode_omega_null_prefix() -> Vec<u8> {
    TupleEncoder::new().null().build()
}

/// Encode a prefix for Ω queries on value + key.
fn encode_omega_value_key_prefix(value: &[u8], key: &[u8]) -> Vec<u8> {
    let mut enc = TupleEncoder::new();
    encode_smart(&mut enc, value);
    encode_smart(&mut enc, key);
    enc.build()
}

/// Encode a prefix for Ω queries on null + key (key-only tags with specific key).
fn encode_omega_null_key_prefix(key: &[u8]) -> Vec<u8> {
    let mut enc = TupleEncoder::new();
    encode_smart(enc.null(), key);
    enc.build()
}

/// Decode an α key into `(subject, key, value?)`.
///
/// Handles both `TupleValue::String` and `TupleValue::Bytes` for binary support.
fn decode_alpha_key(raw: &[u8]) -> Result<(TagValue, TagValue, Option<TagValue>)> {
    let fields = decode(raw)?;
    if fields.len() < 3 {
        bail!("α key has {} fields, expected 3", fields.len());
    }
    let subject = tuple_value_to_tag_value(&fields[0])
        .context("α key field 0 (subject) is not a string or bytes")?;
    let key = tuple_value_to_tag_value(&fields[1])
        .context("α key field 1 (key) is not a string or bytes")?;
    let value = if fields[2].is_null() {
        None
    } else {
        Some(
            tuple_value_to_tag_value(&fields[2])
                .context("α key field 2 (value) is not a string or bytes")?,
        )
    };
    Ok((subject, key, value))
}

/// Decode an Ω key into `(subject, key, value?)`.
///
/// Ω keys are stored as `(value|null, key, subject)`, so we re-order
/// back to `(subject, key, value?)`. Handles both String and Bytes variants.
fn decode_omega_key(raw: &[u8]) -> Result<(TagValue, TagValue, Option<TagValue>)> {
    let fields = decode(raw)?;
    if fields.len() < 3 {
        bail!("Ω key has {} fields, expected 3", fields.len());
    }
    let value = if fields[0].is_null() {
        None
    } else {
        Some(
            tuple_value_to_tag_value(&fields[0])
                .context("Ω key field 0 (value) is not a string or bytes")?,
        )
    };
    let key = tuple_value_to_tag_value(&fields[1])
        .context("Ω key field 1 (key) is not a string or bytes")?;
    let subject = tuple_value_to_tag_value(&fields[2])
        .context("Ω key field 2 (subject) is not a string or bytes")?;
    Ok((subject, key, value))
}

// ============================================================================
// Initialization
// ============================================================================

impl TagStore {
    /// Initialize the tag store, creating or reopening all namespaces.
    ///
    /// Loads the registry from disk (or creates a new one), ensures the global
    /// and per-node α/Ω pairs exist, and opens all registered custom namespaces.
    ///
    /// # Arguments
    ///
    /// * `docs` - The iroh-docs protocol instance (provides `DocsApi` via deref)
    /// * `node_id` - This node's public ID as hex string
    pub async fn init(docs: &Docs, node_id: &str) -> Result<Self> {
        let meta_dir = PathBuf::from(META_DIR);
        tokio::fs::create_dir_all(&meta_dir)
            .await
            .context("creating .iroh-meta directory")?;

        let registry_path = meta_dir.join(REGISTRY_FILE);
        let mut registry = load_registry(&registry_path).await?;

        // Get or create the default author.
        let author = docs.author_default().await?;

        // Global α/Ω pair.
        let global = ensure_pair(docs, &mut registry.global).await?;

        // Per-node α/Ω pair.
        let node = {
            let node_pair = registry
                .nodes
                .entry(node_id.to_owned())
                .or_insert_with(|| PairIds {
                    alpha: String::new(),
                    omega: String::new(),
                });
            ensure_pair_from_ids(docs, node_pair).await?
        };

        // Open custom namespaces.
        let mut custom = HashMap::new();
        for (name, entry) in &registry.custom {
            match entry {
                CustomEntry::Single { id } => {
                    let ns_id = parse_namespace_id(id)?;
                    if let Some(doc) = docs.open(ns_id).await? {
                        custom.insert(name.clone(), CustomNamespace::Single(doc));
                    }
                }
                CustomEntry::Paired(pair_ids) => {
                    let alpha_id = parse_namespace_id(&pair_ids.alpha)?;
                    let omega_id = parse_namespace_id(&pair_ids.omega)?;
                    if let (Some(alpha), Some(omega)) =
                        (docs.open(alpha_id).await?, docs.open(omega_id).await?)
                    {
                        custom.insert(
                            name.clone(),
                            CustomNamespace::Paired(NamespacePair {
                                alpha,
                                omega,
                                alpha_id,
                                omega_id,
                            }),
                        );
                    }
                }
            }
        }

        // Save registry (may have created new namespace IDs).
        save_registry(&registry_path, &registry).await?;

        let (broadcast, _) = broadcast::channel(TAG_BROADCAST_CAPACITY);

        Ok(Self {
            global,
            node,
            custom,
            author,
            registry_path,
            broadcast,
        })
    }

    /// Create a new custom namespace.
    ///
    /// # Arguments
    ///
    /// * `docs` - The iroh-docs protocol instance
    /// * `name` - Unique name for this custom namespace
    /// * `paired` - Whether to create an α/Ω pair (`true`) or a single doc (`false`)
    pub async fn create_custom(
        &mut self,
        docs: &Docs,
        name: &str,
        paired: bool,
    ) -> Result<&CustomNamespace> {
        if self.custom.contains_key(name) {
            bail!("custom namespace {name:?} already exists");
        }

        let mut registry = load_registry(&self.registry_path).await?;

        let ns = if paired {
            let alpha = docs.create().await?;
            let omega = docs.create().await?;
            let alpha_id = alpha.id();
            let omega_id = omega.id();
            registry.custom.insert(
                name.to_owned(),
                CustomEntry::Paired(PairIds {
                    alpha: alpha_id.to_string(),
                    omega: omega_id.to_string(),
                }),
            );
            CustomNamespace::Paired(NamespacePair {
                alpha,
                omega,
                alpha_id,
                omega_id,
            })
        } else {
            let doc = docs.create().await?;
            let doc_id = doc.id();
            registry.custom.insert(
                name.to_owned(),
                CustomEntry::Single {
                    id: doc_id.to_string(),
                },
            );
            CustomNamespace::Single(doc)
        };

        save_registry(&self.registry_path, &registry).await?;
        self.custom.insert(name.to_owned(), ns);

        self.custom
            .get(name)
            .context("just-inserted custom namespace missing")
    }

    /// Remove a custom namespace from the registry.
    ///
    /// Note: this does not delete the underlying iroh-docs documents.
    pub async fn remove_custom(&mut self, name: &str) -> Result<Option<CustomNamespace>> {
        let removed = self.custom.remove(name);
        if removed.is_some() {
            let mut registry = load_registry(&self.registry_path).await?;
            registry.custom.remove(name);
            save_registry(&self.registry_path, &registry).await?;
        }
        Ok(removed)
    }
}

// ============================================================================
// Tag CRUD operations
// ============================================================================

impl TagStore {
    /// Set a tag, writing to both α and Ω of the given namespace pair.
    ///
    /// The `data` bytes are stored as the entry value. Both α and Ω entries
    /// will reference the same content hash.
    ///
    /// # Arguments
    ///
    /// * `ns` - The namespace pair to write to
    /// * `subject` - The entity to tag (usually a filename) — arbitrary bytes
    /// * `key` - The tag key — arbitrary bytes
    /// * `value` - Optional tag value (`None` for key-only tags) — arbitrary bytes
    /// * `data` - Payload bytes stored with the entry
    pub async fn set_tag(
        &self,
        ns: &NamespacePair,
        subject: &[u8],
        key: &[u8],
        value: Option<&[u8]>,
        data: &[u8],
    ) -> Result<()> {
        let alpha_key = encode_alpha_key(subject, key, value);
        let omega_key = encode_omega_key(subject, key, value);

        // iroh-docs rejects empty entries, so use a single null byte as placeholder
        let payload: &[u8] = if data.is_empty() { &[0] } else { data };

        // Write to α first, get the content hash.
        let hash = ns
            .alpha
            .set_bytes(self.author, alpha_key, payload.to_vec())
            .await
            .context("writing α entry")?;

        // Write Ω with the same content hash.
        #[allow(clippy::cast_possible_truncation)]
        let size = payload.len() as u64;
        ns.omega
            .set_hash(self.author, omega_key, hash, size)
            .await
            .context("writing Ω entry")?;

        self.emit(TagEvent::Set {
            ns: self.ns_name(ns),
            subject: String::from_utf8_lossy(subject).into_owned(),
            key: String::from_utf8_lossy(key).into_owned(),
            value: value.map(|v| String::from_utf8_lossy(v).into_owned()),
        });

        Ok(())
    }

    /// Delete a specific tag from both α and Ω.
    ///
    /// Deletes entries matching the exact `(subject, key, value|null)` tuple.
    pub async fn del_tag(
        &self,
        ns: &NamespacePair,
        subject: &[u8],
        key: &[u8],
        value: Option<&[u8]>,
    ) -> Result<()> {
        let alpha_key = encode_alpha_key(subject, key, value);
        let omega_key = encode_omega_key(subject, key, value);

        ns.alpha
            .del(self.author, alpha_key)
            .await
            .context("deleting α entry")?;
        ns.omega
            .del(self.author, omega_key)
            .await
            .context("deleting Ω entry")?;

        self.emit(TagEvent::Del {
            ns: self.ns_name(ns),
            subject: String::from_utf8_lossy(subject).into_owned(),
            key: String::from_utf8_lossy(key).into_owned(),
            value: value.map(|v| String::from_utf8_lossy(v).into_owned()),
        });

        Ok(())
    }

    /// Delete all tags for a subject from both α and Ω.
    ///
    /// First queries α for all tags matching the subject prefix, then
    /// deletes the corresponding Ω entries individually.
    pub async fn del_all_tags(&self, ns: &NamespacePair, subject: &[u8]) -> Result<usize> {
        // Find all tags for this subject so we can delete Ω entries.
        let tags = self.get_tags(ns, subject).await?;
        let count = tags.len();

        // Delete from Ω one by one (each has a different inverted key).
        for tag in &tags {
            let omega_key = encode_omega_key(
                subject,
                &tag.key,
                tag.value.as_ref().map(TagValue::as_bytes),
            );
            ns.omega
                .del(self.author, omega_key)
                .await
                .context("deleting Ω entry for subject cleanup")?;
        }

        // Delete all α entries with the subject prefix.
        let alpha_prefix = encode_alpha_subject_prefix(subject);
        ns.alpha
            .del(self.author, alpha_prefix)
            .await
            .context("deleting α entries for subject")?;

        if count > 0 {
            self.emit(TagEvent::DelAll {
                ns: self.ns_name(ns),
                subject: String::from_utf8_lossy(subject).into_owned(),
            });
        }

        Ok(count)
    }

    /// Delete all tags for a subject+key, regardless of value.
    ///
    /// Queries for all entries with the given subject+key, then deletes each
    /// from both α and Ω. Returns the number of tags deleted.
    pub async fn del_by_key(
        &self,
        ns: &NamespacePair,
        subject: &[u8],
        key: &[u8],
    ) -> Result<usize> {
        let existing = self.get_by_key(ns, subject, key).await?;
        let count = existing.len();
        for tag in existing {
            self.del_tag(ns, subject, key, tag.value.as_ref().map(TagValue::as_bytes))
                .await?;
        }
        Ok(count)
    }

    /// Set a singleton tag — replaces any existing tags with the same subject+key.
    ///
    /// Removes all existing entries for `(subject, key, *)` before setting the new
    /// value. Use for tags where only one value should exist per subject+key pair,
    /// such as "created" or "modified" timestamps.
    pub async fn set_singleton(
        &self,
        ns: &NamespacePair,
        subject: &[u8],
        key: &[u8],
        value: Option<&[u8]>,
        data: &[u8],
    ) -> Result<()> {
        let existing = self.get_by_key(ns, subject, key).await?;
        for tag in existing {
            self.del_tag(ns, subject, key, tag.value.as_ref().map(TagValue::as_bytes))
                .await?;
        }
        self.set_tag(ns, subject, key, value, data).await
    }

    /// Set a tag only if no tag exists for this subject+key.
    ///
    /// Returns `true` if the tag was set, `false` if one already existed.
    pub async fn set_if_absent(
        &self,
        ns: &NamespacePair,
        subject: &[u8],
        key: &[u8],
        value: Option<&[u8]>,
        data: &[u8],
    ) -> Result<bool> {
        let existing = self.get_by_key(ns, subject, key).await?;
        if existing.is_empty() {
            self.set_tag(ns, subject, key, value, data).await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Transfer all tags from one subject to another.
    ///
    /// Copies each tag to the new subject, then deletes all tags from the old.
    /// Returns the number of tags transferred.
    pub async fn transfer_all_tags(
        &self,
        ns: &NamespacePair,
        from: &[u8],
        to: &[u8],
    ) -> Result<usize> {
        let tags = self.get_tags(ns, from).await?;
        let count = tags.len();
        for tag in &tags {
            self.set_tag(
                ns,
                to,
                &tag.key,
                tag.value.as_ref().map(TagValue::as_bytes),
                b"",
            )
            .await?;
        }
        self.del_all_tags(ns, from).await?;

        // Emit a single transfer event (the individual set/del events are already emitted
        // by set_tag and del_all_tags above, so this is a higher-level summary).
        self.emit(TagEvent::Transfer {
            ns: self.ns_name(ns),
            from: String::from_utf8_lossy(from).into_owned(),
            to: String::from_utf8_lossy(to).into_owned(),
            count,
        });

        Ok(count)
    }

    /// Copy all tags from one subject to another without deleting the originals.
    ///
    /// Returns the number of tags copied.
    pub async fn copy_all_tags(&self, ns: &NamespacePair, from: &[u8], to: &[u8]) -> Result<usize> {
        let tags = self.get_tags(ns, from).await?;
        let count = tags.len();
        for tag in &tags {
            self.set_tag(
                ns,
                to,
                &tag.key,
                tag.value.as_ref().map(TagValue::as_bytes),
                b"",
            )
            .await?;
        }
        Ok(count)
    }

    /// Automatically tag a subject with name/file/path metadata.
    ///
    /// Creates the following tags using [`TagStore::set_if_absent`] so existing values
    /// are never overwritten:
    ///
    /// - `name` → the display name (filename or custom name)
    /// - `file` → the file basename (last component of the path)
    /// - `path` → the full original path (only if `source_path` is provided)
    ///
    /// # Arguments
    ///
    /// * `ns` - Namespace pair to store tags in
    /// * `subject` - The subject (tag name / filename) as bytes
    /// * `source_path` - Optional original file path for the `path` tag
    pub async fn auto_tag(
        &self,
        ns: &NamespacePair,
        subject: &[u8],
        source_path: Option<&str>,
    ) -> Result<()> {
        // "name" tag = the subject itself (display name)
        let subject_str = String::from_utf8_lossy(subject);
        self.set_if_absent(ns, subject, b"name", Some(subject), b"")
            .await?;

        // "file" tag = basename (last path component)
        let basename = Path::new(subject_str.as_ref()).file_name().map_or_else(
            || subject_str.to_string(),
            |f| f.to_string_lossy().to_string(),
        );
        self.set_if_absent(ns, subject, b"file", Some(basename.as_bytes()), b"")
            .await?;

        // "path" tag = original source path (if available)
        if let Some(path) = source_path {
            self.set_if_absent(ns, subject, b"path", Some(path.as_bytes()), b"")
                .await?;
        }

        Ok(())
    }

    /// Migrate all existing files to have name/file/path auto-tags.
    ///
    /// Iterates every blob tag in the store, and for each one that lacks
    /// `name`/`file` tags, calls [`TagStore::auto_tag`]. Returns the number of
    /// subjects that were updated.
    ///
    /// # Arguments
    ///
    /// * `store` - The blob store to list tags from
    /// * `ns` - Namespace pair to store metadata tags in
    pub async fn migrate_tags(&self, store: &Store, ns: &NamespacePair) -> Result<usize> {
        let mut list = store.tags().list().await?;
        let mut migrated = 0usize;
        while let Some(item) = list.next().await {
            let item = item?;
            let name = String::from_utf8_lossy(item.name.as_ref()).to_string();

            // Skip internal/system tags (start with '.')
            if name.starts_with('.') {
                continue;
            }

            // Check if this subject already has a "name" tag
            let existing = self.get_by_key(ns, name.as_bytes(), b"name").await?;
            if existing.is_empty() {
                self.auto_tag(ns, name.as_bytes(), None).await?;
                migrated += 1;
            }
        }
        Ok(migrated)
    }
}

// ============================================================================
// Query operations
// ============================================================================

impl TagStore {
    /// Get all tags for a subject from an α namespace.
    pub async fn get_tags(&self, ns: &NamespacePair, subject: &[u8]) -> Result<Vec<Tag>> {
        let prefix = encode_alpha_subject_prefix(subject);
        query_alpha_prefix(&ns.alpha, &prefix).await
    }

    /// Get tags for a subject with a specific key from an α namespace.
    pub async fn get_by_key(
        &self,
        ns: &NamespacePair,
        subject: &[u8],
        key: &[u8],
    ) -> Result<Vec<Tag>> {
        let prefix = encode_alpha_subject_key_prefix(subject, key);
        query_alpha_prefix(&ns.alpha, &prefix).await
    }

    /// Get a single exact tag from an α namespace.
    pub async fn get_exact(
        &self,
        ns: &NamespacePair,
        subject: &[u8],
        key: &[u8],
        value: Option<&[u8]>,
    ) -> Result<Option<Tag>> {
        let exact_key = encode_alpha_key(subject, key, value);
        let query = Query::key_exact(exact_key);
        let entry = ns.alpha.get_one(query).await?;
        match entry {
            Some(e) => Ok(Some(entry_to_tag_alpha(&e)?)),
            None => Ok(None),
        }
    }

    /// Find all subjects that have a tag with a specific value (via Ω).
    pub async fn find_by_value(&self, ns: &NamespacePair, value: &[u8]) -> Result<Vec<Tag>> {
        let prefix = encode_omega_value_prefix(value);
        query_omega_prefix(&ns.omega, &prefix).await
    }

    /// Find all subjects with a specific `key=value` pair (via Ω).
    pub async fn find_by_key_value(
        &self,
        ns: &NamespacePair,
        key: &[u8],
        value: &[u8],
    ) -> Result<Vec<Tag>> {
        let prefix = encode_omega_value_key_prefix(value, key);
        query_omega_prefix(&ns.omega, &prefix).await
    }

    /// Find all key-only tags (via Ω, querying the null prefix).
    pub async fn find_key_only(&self, ns: &NamespacePair) -> Result<Vec<Tag>> {
        let prefix = encode_omega_null_prefix();
        query_omega_prefix(&ns.omega, &prefix).await
    }

    /// Find all subjects with a specific key-only tag (via Ω).
    pub async fn find_by_key_only(&self, ns: &NamespacePair, key: &[u8]) -> Result<Vec<Tag>> {
        let prefix = encode_omega_null_key_prefix(key);
        query_omega_prefix(&ns.omega, &prefix).await
    }

    /// List all tags in an α namespace.
    pub async fn list_all(&self, ns: &NamespacePair) -> Result<Vec<Tag>> {
        let query = Query::all().build();
        let entries = ns.alpha.get_many(query).await?;
        tokio::pin!(entries);
        let mut tags = Vec::new();
        while let Some(entry) = entries.try_next().await? {
            tags.push(entry_to_tag_alpha(&entry)?);
        }
        Ok(tags)
    }

    /// Find all tags with a specific key across all subjects.
    ///
    /// Performs a full α scan and filters in-memory. Suitable for small to
    /// moderate datasets (hundreds to low thousands of tags).
    pub async fn find_by_key(&self, ns: &NamespacePair, key: &[u8]) -> Result<Vec<Tag>> {
        let all = self.list_all(ns).await?;
        Ok(all
            .into_iter()
            .filter(|t| t.key.as_bytes() == key)
            .collect())
    }

    /// Search tags using structured query syntax.
    ///
    /// Parses the query string into [`SearchTerm`]s and executes each against
    /// the namespace. Multiple terms use AND semantics (intersection).
    ///
    /// An empty query returns all tags.
    pub async fn search_by_query(&self, ns: &NamespacePair, query: &str) -> Result<Vec<Tag>> {
        let terms = parse_search_query(query);

        if terms.is_empty() {
            return self.list_all(ns).await;
        }

        let mut results: Option<Vec<Tag>> = None;

        for term in &terms {
            let term_results = match term {
                SearchTerm::KeyOnly(key) => self.find_by_key(ns, key.as_bytes()).await?,
                SearchTerm::ValueOnly(value) => self.find_by_value(ns, value.as_bytes()).await?,
                SearchTerm::KeyValue(key, value) => {
                    self.find_by_key_value(ns, key.as_bytes(), value.as_bytes())
                        .await?
                }
                SearchTerm::Literal(text) | SearchTerm::BareWord(text) => {
                    let all = self.list_all(ns).await?;
                    let text_lower = text.to_lowercase();
                    all.into_iter()
                        .filter(|t| {
                            t.subject
                                .display_lossy()
                                .to_lowercase()
                                .contains(&text_lower)
                                || t.key.display_lossy().to_lowercase().contains(&text_lower)
                                || t.value.as_ref().is_some_and(|v| {
                                    v.display_lossy().to_lowercase().contains(&text_lower)
                                })
                        })
                        .collect()
                }
            };

            results = Some(match results {
                None => term_results,
                Some(existing) => {
                    // AND intersection: keep tags present in both sets
                    type TagTuple = (Vec<u8>, Vec<u8>, Option<Vec<u8>>);
                    let term_set: std::collections::HashSet<TagTuple> = term_results
                        .iter()
                        .map(|t| {
                            (
                                t.subject.as_bytes().to_vec(),
                                t.key.as_bytes().to_vec(),
                                t.value.as_ref().map(|v| v.as_bytes().to_vec()),
                            )
                        })
                        .collect();
                    existing
                        .into_iter()
                        .filter(|t| {
                            term_set.contains(&(
                                t.subject.as_bytes().to_vec(),
                                t.key.as_bytes().to_vec(),
                                t.value.as_ref().map(|v| v.as_bytes().to_vec()),
                            ))
                        })
                        .collect()
                }
            });
        }

        Ok(results.unwrap_or_default())
    }
}

// ============================================================================
// Internal query helpers
// ============================================================================

/// Query an α document by key prefix and decode results.
async fn query_alpha_prefix(doc: &Doc, prefix: &[u8]) -> Result<Vec<Tag>> {
    let query = Query::key_prefix(prefix).build();
    let entries = doc.get_many(query).await?;
    tokio::pin!(entries);
    let mut tags = Vec::new();
    while let Some(entry) = entries.try_next().await? {
        tags.push(entry_to_tag_alpha(&entry)?);
    }
    Ok(tags)
}

/// Query an Ω document by key prefix and decode results.
async fn query_omega_prefix(doc: &Doc, prefix: &[u8]) -> Result<Vec<Tag>> {
    let query = Query::key_prefix(prefix).build();
    let entries = doc.get_many(query).await?;
    tokio::pin!(entries);
    let mut tags = Vec::new();
    while let Some(entry) = entries.try_next().await? {
        tags.push(entry_to_tag_omega(&entry)?);
    }
    Ok(tags)
}

/// Convert an α entry to a [`Tag`].
fn entry_to_tag_alpha(entry: &iroh_docs::Entry) -> Result<Tag> {
    let (subject, key, value) = decode_alpha_key(entry.key())?;
    Ok(Tag {
        subject,
        key,
        value,
        hash: entry.content_hash(),
        timestamp: entry.timestamp(),
        author: entry.author(),
    })
}

/// Convert an Ω entry to a [`Tag`] (re-orders fields back to α order).
fn entry_to_tag_omega(entry: &iroh_docs::Entry) -> Result<Tag> {
    let (subject, key, value) = decode_omega_key(entry.key())?;
    Ok(Tag {
        subject,
        key,
        value,
        hash: entry.content_hash(),
        timestamp: entry.timestamp(),
        author: entry.author(),
    })
}

// ============================================================================
// Search query parser
// ============================================================================

/// A parsed search term from the structured search syntax.
///
/// Supports: `key:` (key only), `:value` (value only), `key:value` (pair),
/// `"quoted"` (literal across all fields), bare word (search all fields).
///
/// Quoted strings can participate in key:value syntax: `"key:":"value"` sets
/// key to `key:` and value to `value`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchTerm {
    /// Filter by key name (input: `name:`)
    KeyOnly(String),
    /// Filter by value (input: `:myfile.txt`)
    ValueOnly(String),
    /// Filter by key AND value (input: `name:myfile.txt`)
    KeyValue(String, String),
    /// Literal search across all fields (input: `"literal"`)
    Literal(String),
    /// Bare word search across subject/key/value
    BareWord(String),
}

/// Parse a search query string into structured [`SearchTerm`]s.
///
/// Terms are space-separated (respecting quoted strings). Each term follows:
///
/// - `"quoted text"` → Literal search across subject/key/value
/// - `key:` → Filter by key name
/// - `:value` → Filter by value
/// - `key:value` → Filter by exact key-value pair
/// - `bare` → Search across subject/key/value (case-insensitive contains)
///
/// Quoted strings can be used as key or value components:
///
/// - `"key:":":value"` → key is literally `key:`, value is literally `:value`
///
/// Multiple terms are `ANDed` together.
pub fn parse_search_query(input: &str) -> Vec<SearchTerm> {
    let mut terms = Vec::new();
    let input = input.trim();
    if input.is_empty() {
        return terms;
    }

    let mut i = 0;
    let bytes = input.as_bytes();

    while i < bytes.len() {
        // Skip whitespace
        while i < bytes.len() && bytes[i] == b' ' {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }

        // Extract one term (may contain quoted parts and colons)
        let start = i;
        let mut in_quotes = false;
        while i < bytes.len() {
            if bytes[i] == b'"' {
                in_quotes = !in_quotes;
                i += 1;
            } else if bytes[i] == b' ' && !in_quotes {
                break;
            } else {
                i += 1;
            }
        }

        let term_str = &input[start..i];
        if !term_str.is_empty() {
            terms.push(parse_single_term(term_str));
        }
    }

    terms
}

/// Parse a single whitespace-delimited term into a [`SearchTerm`].
fn parse_single_term(s: &str) -> SearchTerm {
    // Try to find an unquoted colon separator
    if let Some((left, right)) = split_on_unquoted_colon(s) {
        let key = strip_quotes(&left);
        let value = strip_quotes(&right);

        if key.is_empty() && !value.is_empty() {
            SearchTerm::ValueOnly(value)
        } else if !key.is_empty() && value.is_empty() {
            SearchTerm::KeyOnly(key)
        } else if !key.is_empty() && !value.is_empty() {
            SearchTerm::KeyValue(key, value)
        } else {
            // Both empty — bare colon, treat as bare word
            SearchTerm::BareWord(s.to_owned())
        }
    } else {
        // No unquoted colon
        let stripped = strip_quotes(s);
        if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
            SearchTerm::Literal(stripped)
        } else {
            SearchTerm::BareWord(stripped)
        }
    }
}

/// Split a string on the first colon that is NOT inside double quotes.
///
/// Returns `None` if no unquoted colon is found.
fn split_on_unquoted_colon(s: &str) -> Option<(String, String)> {
    let mut in_quotes = false;
    for (i, b) in s.bytes().enumerate() {
        if b == b'"' {
            in_quotes = !in_quotes;
        } else if b == b':' && !in_quotes {
            return Some((s[..i].to_string(), s[i + 1..].to_string()));
        }
    }
    None
}

/// Strip surrounding double quotes from a string, if present.
fn strip_quotes(s: &str) -> String {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_owned()
    }
}

// ============================================================================
// Registry I/O
// ============================================================================

/// Load the registry from disk, returning a default if it doesn't exist.
async fn load_registry(path: &Path) -> Result<Registry> {
    match tokio::fs::read(path).await {
        Ok(bytes) => {
            let registry: Registry =
                serde_json::from_slice(&bytes).context("parsing registry.json")?;
            Ok(registry)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Registry::default()),
        Err(e) => Err(e).context("reading registry.json"),
    }
}

/// Save the registry to disk atomically (write to temp then rename).
async fn save_registry(path: &Path, registry: &Registry) -> Result<()> {
    let json = serde_json::to_vec_pretty(registry).context("serializing registry")?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .context("creating registry parent directory")?;
    }
    // Atomic write: temp file then rename.
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, &json)
        .await
        .context("writing registry temp file")?;
    tokio::fs::rename(&tmp, path)
        .await
        .context("renaming registry temp to final")?;
    Ok(())
}

/// Parse a hex string into a [`NamespaceId`].
fn parse_namespace_id(hex: &str) -> Result<NamespaceId> {
    hex.parse::<NamespaceId>()
        .map_err(|e| anyhow::anyhow!("invalid namespace ID {hex:?}: {e}"))
}

// ============================================================================
// Namespace creation helpers
// ============================================================================

/// Ensure an α/Ω pair exists, creating new docs if needed.
///
/// If `pair_ids` is `None`, creates both docs and sets it to `Some(...)`.
/// If `pair_ids` is `Some(...)` with valid IDs, opens the existing docs.
/// If the docs no longer exist in the store (e.g., ephemeral mode restart),
/// creates fresh ones and updates the IDs.
async fn ensure_pair(docs: &Docs, pair_ids: &mut Option<PairIds>) -> Result<NamespacePair> {
    if let Some(ids) = pair_ids
        && !ids.alpha.is_empty()
        && !ids.omega.is_empty()
    {
        if let Ok(alpha_id) = parse_namespace_id(&ids.alpha)
            && let Ok(omega_id) = parse_namespace_id(&ids.omega)
        {
            // Try to open existing — if either is missing, fall through to create
            if let Ok(Some(alpha)) = docs.open(alpha_id).await
                && let Ok(Some(omega)) = docs.open(omega_id).await
            {
                return Ok(NamespacePair {
                    alpha,
                    omega,
                    alpha_id,
                    omega_id,
                });
            }
        }
        tracing::info!("namespace pair not found in store, creating fresh pair");
    }

    // Create fresh pair
    let alpha = docs.create().await?;
    let omega = docs.create().await?;
    let alpha_id = alpha.id();
    let omega_id = omega.id();
    *pair_ids = Some(PairIds {
        alpha: alpha_id.to_string(),
        omega: omega_id.to_string(),
    });
    Ok(NamespacePair {
        alpha,
        omega,
        alpha_id,
        omega_id,
    })
}

/// Ensure an α/Ω pair from mutable `PairIds`, creating new docs if IDs are empty
/// or if the referenced namespaces no longer exist in the store.
async fn ensure_pair_from_ids(docs: &Docs, ids: &mut PairIds) -> Result<NamespacePair> {
    if !ids.alpha.is_empty() && !ids.omega.is_empty() {
        if let Ok(alpha_id) = parse_namespace_id(&ids.alpha)
            && let Ok(omega_id) = parse_namespace_id(&ids.omega)
            && let Ok(Some(alpha)) = docs.open(alpha_id).await
            && let Ok(Some(omega)) = docs.open(omega_id).await
        {
            return Ok(NamespacePair {
                alpha,
                omega,
                alpha_id,
                omega_id,
            });
        }
        tracing::info!("node namespace pair not found in store, creating fresh pair");
    }

    // Create fresh pair
    let alpha = docs.create().await?;
    let omega = docs.create().await?;
    ids.alpha = alpha.id().to_string();
    ids.omega = omega.id().to_string();
    Ok(NamespacePair {
        alpha_id: alpha.id(),
        omega_id: omega.id(),
        alpha,
        omega,
    })
}

// ============================================================================
// Legacy MetaDoc system (backward compatibility)
// ============================================================================
// These functions maintain the old JSON-blob metadata system stored in the
// `.meta` iroh-blobs tag. They coexist with the new iroh-docs system during
// migration. Callers should migrate to TagStore methods over time.

use std::time::{SystemTime, UNIX_EPOCH};

/// Reserved iroh-blobs tag name for the legacy metadata document.
const META_TAG: &str = ".meta";

/// The legacy metadata document stored as a JSON blob.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MetaDoc {
    /// Schema version for forward compatibility.
    pub version: u32,
    /// All metadata tags.
    pub tags: Vec<MetaTag>,
}

/// A single legacy metadata tag.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaTag {
    /// Subject this tag describes (usually a filename like "README.md").
    pub subject: String,
    /// Tag key (e.g., "created", "category", "author").
    pub key: String,
    /// Optional value. `None` for key-only tags like "pinned".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Optional link to another entity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<MetaLink>,
    /// When this tag was created (unix seconds).
    pub created_at: u64,
    /// When this tag was last modified (unix seconds).
    pub modified_at: u64,
}

/// A link target — points to a content hash or a filename.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "target")]
pub enum MetaLink {
    /// Points to a content hash (hex string).
    Hash(String),
    /// Points to a filename / tag name.
    Name(String),
}

/// Get current unix timestamp in seconds.
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Load the legacy metadata document from the store.
///
/// Returns an empty `MetaDoc` if the `.meta` tag doesn't exist yet.
pub async fn load_meta(store: &Store) -> Result<MetaDoc> {
    let Some(tag_info) = store.tags().get(META_TAG).await? else {
        return Ok(MetaDoc::default());
    };

    let bytes = store.blobs().get_bytes(tag_info.hash).await?;
    let doc: MetaDoc = serde_json::from_slice(&bytes)?;
    Ok(doc)
}

/// Save the legacy metadata document to the store.
pub async fn save_meta(store: &Store, doc: &MetaDoc) -> Result<()> {
    let json = serde_json::to_vec(doc)?;
    let outcome = store.blobs().add_bytes(json).await?;
    store.tags().set(META_TAG, outcome.hash).await?;
    Ok(())
}

/// Check if a tag name is internal (starts with `.`).
pub fn is_internal_tag(name: &str) -> bool {
    name.starts_with('.')
}

/// Add a new tag to the legacy metadata document.
pub fn add_tag(
    doc: &mut MetaDoc,
    subject: &str,
    key: &str,
    value: Option<&str>,
    link: Option<MetaLink>,
) {
    let now = now_unix();
    doc.tags.push(MetaTag {
        subject: subject.to_owned(),
        key: key.to_owned(),
        value: value.map(ToOwned::to_owned),
        link,
        created_at: now,
        modified_at: now,
    });
}

/// Get created time for a subject from the legacy metadata.
pub fn get_created_time(doc: &MetaDoc, subject: &str) -> Option<u64> {
    doc.tags
        .iter()
        .find(|t| t.subject == subject && t.key == "created")
        .and_then(|t| t.value.as_deref().and_then(|v| v.parse().ok()))
}

/// Get modified time for a subject from the legacy metadata.
pub fn get_modified_time(doc: &MetaDoc, subject: &str) -> Option<u64> {
    doc.tags
        .iter()
        .find(|t| t.subject == subject && t.key == "modified")
        .and_then(|t| t.value.as_deref().and_then(|v| v.parse().ok()))
}

/// Set the `created` timestamp for a subject (only if not already set).
pub fn set_created(doc: &mut MetaDoc, subject: &str) {
    let exists = doc
        .tags
        .iter()
        .any(|t| t.subject == subject && t.key == "created");
    if !exists {
        let now = now_unix();
        add_tag(doc, subject, "created", Some(&now.to_string()), None);
    }
}

/// Set/update the `modified` timestamp for a subject.
pub fn set_modified(doc: &mut MetaDoc, subject: &str) {
    let now = now_unix();
    // Remove existing modified tag
    doc.tags
        .retain(|t| !(t.subject == subject && t.key == "modified"));
    add_tag(doc, subject, "modified", Some(&now.to_string()), None);
}

/// Transfer all tags from one subject to another.
pub fn transfer_tags(doc: &mut MetaDoc, from: &str, to: &str) {
    for tag in &mut doc.tags {
        if tag.subject == from {
            to.clone_into(&mut tag.subject);
        }
    }
}

/// Add an archive tag linking a subject to its archived copy.
pub fn add_archive_tag(
    doc: &mut MetaDoc,
    subject: &str,
    archive_name: &str,
    hash: &str,
    reason: &str,
) {
    add_tag(
        doc,
        subject,
        &format!("archive.{reason}"),
        Some(archive_name),
        Some(MetaLink::Hash(hash.to_owned())),
    );
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_alpha_key_with_value() {
        let key = encode_alpha_key(b"README.md", b"label", Some(b"rust"));
        let fields = decode(&key).unwrap();
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].as_str(), Some("README.md"));
        assert_eq!(fields[1].as_str(), Some("label"));
        assert_eq!(fields[2].as_str(), Some("rust"));
    }

    #[test]
    fn test_encode_alpha_key_without_value() {
        let key = encode_alpha_key(b"README.md", b"pinned", None);
        let fields = decode(&key).unwrap();
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].as_str(), Some("README.md"));
        assert_eq!(fields[1].as_str(), Some("pinned"));
        assert!(fields[2].is_null());
    }

    #[test]
    fn test_encode_omega_key_with_value() {
        let key = encode_omega_key(b"README.md", b"label", Some(b"rust"));
        let fields = decode(&key).unwrap();
        assert_eq!(fields.len(), 3);
        // Inverted order: (value, key, subject)
        assert_eq!(fields[0].as_str(), Some("rust"));
        assert_eq!(fields[1].as_str(), Some("label"));
        assert_eq!(fields[2].as_str(), Some("README.md"));
    }

    #[test]
    fn test_encode_omega_key_without_value() {
        let key = encode_omega_key(b"README.md", b"pinned", None);
        let fields = decode(&key).unwrap();
        assert_eq!(fields.len(), 3);
        // Inverted: (null, key, subject)
        assert!(fields[0].is_null());
        assert_eq!(fields[1].as_str(), Some("pinned"));
        assert_eq!(fields[2].as_str(), Some("README.md"));
    }

    #[test]
    fn test_decode_alpha_key() {
        let raw = encode_alpha_key(b"file.txt", b"tag", Some(b"v1"));
        let (subject, key, value) = decode_alpha_key(&raw).unwrap();
        assert_eq!(subject, "file.txt");
        assert_eq!(key, "tag");
        assert_eq!(value, Some(TagValue::from("v1")));
    }

    #[test]
    fn test_decode_alpha_key_null_value() {
        let raw = encode_alpha_key(b"file.txt", b"pinned", None);
        let (subject, key, value) = decode_alpha_key(&raw).unwrap();
        assert_eq!(subject, "file.txt");
        assert_eq!(key, "pinned");
        assert_eq!(value, None);
    }

    #[test]
    fn test_decode_omega_key() {
        let raw = encode_omega_key(b"file.txt", b"label", Some(b"rust"));
        let (subject, key, value) = decode_omega_key(&raw).unwrap();
        assert_eq!(subject, "file.txt");
        assert_eq!(key, "label");
        assert_eq!(value, Some(TagValue::from("rust")));
    }

    #[test]
    fn test_decode_omega_key_null() {
        let raw = encode_omega_key(b"file.txt", b"pinned", None);
        let (subject, key, value) = decode_omega_key(&raw).unwrap();
        assert_eq!(subject, "file.txt");
        assert_eq!(key, "pinned");
        assert_eq!(value, None);
    }

    #[test]
    fn test_alpha_prefix_matches_subject() {
        let key = encode_alpha_key(b"README.md", b"label", Some(b"rust"));
        let prefix = encode_alpha_subject_prefix(b"README.md");
        assert!(key.starts_with(&prefix));
    }

    #[test]
    fn test_alpha_prefix_matches_subject_key() {
        let key = encode_alpha_key(b"README.md", b"label", Some(b"rust"));
        let prefix = encode_alpha_subject_key_prefix(b"README.md", b"label");
        assert!(key.starts_with(&prefix));
    }

    #[test]
    fn test_alpha_prefix_does_not_match_other_subject() {
        let key = encode_alpha_key(b"README.md", b"label", Some(b"rust"));
        let prefix = encode_alpha_subject_prefix(b"other.txt");
        assert!(!key.starts_with(&prefix));
    }

    #[test]
    fn test_omega_prefix_matches_value() {
        let key = encode_omega_key(b"README.md", b"label", Some(b"rust"));
        let prefix = encode_omega_value_prefix(b"rust");
        assert!(key.starts_with(&prefix));
    }

    #[test]
    fn test_omega_prefix_matches_value_key() {
        let key = encode_omega_key(b"README.md", b"label", Some(b"rust"));
        let prefix = encode_omega_value_key_prefix(b"rust", b"label");
        assert!(key.starts_with(&prefix));
    }

    #[test]
    fn test_omega_null_prefix_matches_keyonly() {
        let key = encode_omega_key(b"README.md", b"pinned", None);
        let prefix = encode_omega_null_prefix();
        assert!(key.starts_with(&prefix));
    }

    #[test]
    fn test_omega_null_prefix_does_not_match_valued() {
        let key = encode_omega_key(b"README.md", b"label", Some(b"rust"));
        let prefix = encode_omega_null_prefix();
        assert!(!key.starts_with(&prefix));
    }

    #[test]
    fn test_omega_null_key_prefix() {
        let key = encode_omega_key(b"README.md", b"pinned", None);
        let prefix = encode_omega_null_key_prefix(b"pinned");
        assert!(key.starts_with(&prefix));
    }

    #[test]
    fn test_omega_null_key_prefix_no_match() {
        let key = encode_omega_key(b"README.md", b"pinned", None);
        let prefix = encode_omega_null_key_prefix(b"starred");
        assert!(!key.starts_with(&prefix));
    }

    #[test]
    fn test_alpha_sort_order() {
        let k1 = encode_alpha_key(b"a.txt", b"label", Some(b"alpha"));
        let k2 = encode_alpha_key(b"a.txt", b"label", Some(b"beta"));
        let k3 = encode_alpha_key(b"b.txt", b"label", Some(b"alpha"));
        assert!(k1 < k2, "same subject+key, alpha < beta");
        assert!(k2 < k3, "a.txt < b.txt");
    }

    #[test]
    fn test_omega_sort_order() {
        let k1 = encode_omega_key(b"b.txt", b"label", Some(b"alpha"));
        let k2 = encode_omega_key(b"a.txt", b"label", Some(b"beta"));
        assert!(k1 < k2, "alpha < beta in Ω regardless of subject");
    }

    #[test]
    fn test_roundtrip_alpha_key() {
        let subjects: &[&[u8]] = &[
            b"",
            b"README.md",
            b"path/to/file.txt",
            "日本語.md".as_bytes(),
        ];
        let keys: &[&[u8]] = &[b"label", b"created", b""];
        let values: &[Option<&[u8]>] = &[None, Some(b""), Some(b"rust"), Some("日本語".as_bytes())];
        for subject in subjects {
            for key in keys {
                for value in values {
                    let encoded = encode_alpha_key(subject, key, *value);
                    let (s, k, v) = decode_alpha_key(&encoded).unwrap();
                    assert_eq!(s.as_bytes(), *subject);
                    assert_eq!(k.as_bytes(), *key);
                    assert_eq!(v.as_ref().map(TagValue::as_bytes), value.as_deref());
                }
            }
        }
    }

    #[test]
    fn test_roundtrip_omega_key() {
        let subjects: &[&[u8]] = &[b"", b"README.md", b"path/to/file.txt"];
        let keys: &[&[u8]] = &[b"label", b"created"];
        let values: &[Option<&[u8]>] = &[None, Some(b"rust"), Some("日本語".as_bytes())];
        for subject in subjects {
            for key in keys {
                for value in values {
                    let encoded = encode_omega_key(subject, key, *value);
                    let (s, k, v) = decode_omega_key(&encoded).unwrap();
                    assert_eq!(s.as_bytes(), *subject);
                    assert_eq!(k.as_bytes(), *key);
                    assert_eq!(v.as_ref().map(TagValue::as_bytes), value.as_deref());
                }
            }
        }
    }

    // ========================================================================
    // TagEvent serialization tests
    // ========================================================================

    #[test]
    fn test_tag_event_set_serialization() {
        let event = TagEvent::Set {
            ns: "global".to_owned(),
            subject: "README.md".to_owned(),
            key: "label".to_owned(),
            value: Some("rust".to_owned()),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"set\""));
        assert!(json.contains("\"ns\":\"global\""));
        assert!(json.contains("\"subject\":\"README.md\""));
        assert!(json.contains("\"key\":\"label\""));
        assert!(json.contains("\"value\":\"rust\""));
    }

    #[test]
    fn test_tag_event_set_without_value() {
        let event = TagEvent::Set {
            ns: "global".to_owned(),
            subject: "README.md".to_owned(),
            key: "pinned".to_owned(),
            value: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        // value should be skipped when None
        assert!(!json.contains("\"value\""));
        assert!(json.contains("\"key\":\"pinned\""));
    }

    #[test]
    fn test_tag_event_del_serialization() {
        let event = TagEvent::Del {
            ns: "node".to_owned(),
            subject: "file.txt".to_owned(),
            key: "status".to_owned(),
            value: Some("draft".to_owned()),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"del\""));
        assert!(json.contains("\"ns\":\"node\""));
        assert!(json.contains("\"subject\":\"file.txt\""));
    }

    #[test]
    fn test_tag_event_del_all_serialization() {
        let event = TagEvent::DelAll {
            ns: "global".to_owned(),
            subject: "old-file.md".to_owned(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"del_all\""));
        assert!(json.contains("\"subject\":\"old-file.md\""));
        // Should not have key or value fields
        assert!(!json.contains("\"key\""));
        assert!(!json.contains("\"value\""));
    }

    #[test]
    fn test_tag_event_transfer_serialization() {
        let event = TagEvent::Transfer {
            ns: "global".to_owned(),
            from: "old.md".to_owned(),
            to: "new.md".to_owned(),
            count: 5,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"transfer\""));
        assert!(json.contains("\"from\":\"old.md\""));
        assert!(json.contains("\"to\":\"new.md\""));
        assert!(json.contains("\"count\":5"));
    }

    // ========================================================================
    // Special characters and edge cases
    // ========================================================================

    #[test]
    fn test_encode_decode_special_chars_in_subject() {
        let subjects: &[&[u8]] = &[
            b"file with spaces.txt",
            b"file/with/slashes.md",
            "emoji_🦀.rs".as_bytes(),
            b"dots...multiple...dots",
            b"",
        ];
        for subject in subjects {
            let encoded = encode_alpha_key(subject, b"key", Some(b"val"));
            let (s, k, v) = decode_alpha_key(&encoded).unwrap();
            assert_eq!(s.as_bytes(), *subject, "subject roundtrip failed");
            assert_eq!(k, "key");
            assert_eq!(v, Some(TagValue::from("val")));
        }
    }

    #[test]
    fn test_encode_decode_special_chars_in_key() {
        let keys: &[&[u8]] = &[
            b"key-with-dashes",
            b"key.with.dots",
            b"key_with_underscores",
            b"",
        ];
        for key in keys {
            let encoded = encode_alpha_key(b"file.txt", key, Some(b"val"));
            let (s, k, v) = decode_alpha_key(&encoded).unwrap();
            assert_eq!(s, "file.txt");
            assert_eq!(k.as_bytes(), *key, "key roundtrip failed");
            assert_eq!(v, Some(TagValue::from("val")));
        }
    }

    #[test]
    fn test_encode_decode_special_chars_in_value() {
        let values: &[Option<&[u8]>] = &[
            Some(b"value with spaces"),
            Some(b"value/with/slashes"),
            Some(b""),
            None,
        ];
        for value in values {
            let encoded = encode_alpha_key(b"file.txt", b"key", *value);
            let (s, k, v) = decode_alpha_key(&encoded).unwrap();
            assert_eq!(s, "file.txt");
            assert_eq!(k, "key");
            assert_eq!(
                v.as_ref().map(TagValue::as_bytes),
                value.as_deref(),
                "value roundtrip failed for: {value:?}"
            );
        }
    }

    #[test]
    fn test_alpha_key_prefix_isolation() {
        // Keys for "file.txt" should NOT match prefix for "file.txt.bak"
        let key = encode_alpha_key(b"file.txt", b"label", Some(b"v1"));
        let prefix = encode_alpha_subject_prefix(b"file.txt.bak");
        assert!(!key.starts_with(&prefix));
    }

    #[test]
    fn test_omega_key_different_values_sort_correctly() {
        // Omega keys group by value, so same value different subjects should be adjacent
        let k1 = encode_omega_key(b"a.txt", b"label", Some(b"rust"));
        let k2 = encode_omega_key(b"z.txt", b"label", Some(b"rust"));
        let k3 = encode_omega_key(b"a.txt", b"label", Some(b"python"));

        // rust > python alphabetically, so k3 (python) < k1 (rust)
        assert!(k3 < k1, "python < rust in Ω order");
        // Same value (rust), different subjects should both match same value prefix
        let prefix = encode_omega_value_prefix(b"rust");
        assert!(k1.starts_with(&prefix));
        assert!(k2.starts_with(&prefix));
        assert!(!k3.starts_with(&prefix));
    }

    #[test]
    fn test_registry_serde_roundtrip() {
        let registry = Registry {
            version: 1,
            global: Some(PairIds {
                alpha: "abcd1234".to_owned(),
                omega: "5678efgh".to_owned(),
            }),
            nodes: {
                let mut m = HashMap::new();
                m.insert(
                    "node1".to_owned(),
                    PairIds {
                        alpha: "node1alpha".to_owned(),
                        omega: "node1omega".to_owned(),
                    },
                );
                m
            },
            custom: {
                let mut m = HashMap::new();
                m.insert(
                    "my-tags".to_owned(),
                    CustomEntry::Single {
                        id: "custom1".to_owned(),
                    },
                );
                m.insert(
                    "paired-tags".to_owned(),
                    CustomEntry::Paired(PairIds {
                        alpha: "palpha".to_owned(),
                        omega: "pomega".to_owned(),
                    }),
                );
                m
            },
        };

        let json = serde_json::to_string_pretty(&registry).unwrap();
        let parsed: Registry = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.version, 1);
        assert!(parsed.global.is_some());
        let g = parsed.global.unwrap();
        assert_eq!(g.alpha, "abcd1234");
        assert_eq!(g.omega, "5678efgh");
        assert_eq!(parsed.nodes.len(), 1);
        assert_eq!(parsed.custom.len(), 2);
    }

    // ========================================================================
    // TagValue tests
    // ========================================================================

    #[test]
    fn test_tag_value_from_string() {
        let tv = TagValue::from("hello");
        assert_eq!(tv.as_str(), Some("hello"));
        assert!(tv.is_utf8());
        assert_eq!(tv.as_bytes(), b"hello");
        assert_eq!(format!("{tv}"), "hello");
    }

    #[test]
    fn test_tag_value_from_bytes_valid_utf8() {
        let tv = TagValue::from(b"hello".to_vec());
        assert_eq!(tv.as_str(), Some("hello"));
        assert!(tv.is_utf8());
    }

    #[test]
    fn test_tag_value_from_bytes_invalid_utf8() {
        let tv = TagValue::from(vec![0xFF, 0xFE, 0x01]);
        assert_eq!(tv.as_str(), None);
        assert!(!tv.is_utf8());
        assert_eq!(format!("{tv}"), "<binary 3 bytes>");
        assert_eq!(tv.display_lossy(), "\u{FFFD}\u{FFFD}\u{01}");
    }

    #[test]
    fn test_tag_value_partial_eq_str() {
        let tv = TagValue::from("hello");
        assert!(tv == *"hello");
        assert!(tv != *"world");
    }

    #[test]
    fn test_tag_value_display_truncated() {
        let long_str = "a".repeat(300);
        let tv = TagValue::from(long_str);
        let truncated = tv.display_truncated(10);
        assert!(truncated.ends_with("..."));
        assert!(truncated.len() < 20);
    }

    #[test]
    fn test_tag_value_serialize() {
        let tv = TagValue::from("hello");
        let json = serde_json::to_string(&tv).unwrap();
        assert_eq!(json, "\"hello\"");
    }

    #[test]
    fn test_tag_value_serialize_binary() {
        let tv = TagValue::from(vec![0xFF, 0xFE]);
        let json = serde_json::to_string(&tv).unwrap();
        // Should serialize as lossy UTF-8 with replacement chars
        assert!(json.contains('\u{FFFD}'));
    }

    // ========================================================================
    // Binary key/value roundtrip tests
    // ========================================================================

    #[test]
    fn test_binary_key_roundtrip() {
        let binary_key: &[u8] = &[0xFF, 0x00, 0xAB, 0xCD];
        let encoded = encode_alpha_key(b"file.txt", binary_key, Some(b"val"));
        let (s, k, v) = decode_alpha_key(&encoded).unwrap();
        assert_eq!(s, "file.txt");
        assert_eq!(k.as_bytes(), binary_key);
        assert!(!k.is_utf8());
        assert_eq!(v, Some(TagValue::from("val")));
    }

    #[test]
    fn test_binary_value_roundtrip() {
        let binary_val: &[u8] = &[0xDE, 0xAD, 0xBE, 0xEF];
        let encoded = encode_alpha_key(b"file.txt", b"data", Some(binary_val));
        let (s, k, v) = decode_alpha_key(&encoded).unwrap();
        assert_eq!(s, "file.txt");
        assert_eq!(k, "data");
        let v = v.unwrap();
        assert_eq!(v.as_bytes(), binary_val);
        assert!(!v.is_utf8());
    }

    #[test]
    fn test_binary_subject_roundtrip() {
        let binary_subj: &[u8] = &[0x80, 0xFF, 0xFE];
        let encoded = encode_alpha_key(binary_subj, b"key", Some(b"val"));
        let (s, k, v) = decode_alpha_key(&encoded).unwrap();
        assert_eq!(s.as_bytes(), binary_subj);
        assert!(!s.is_utf8());
        assert_eq!(k, "key");
        assert_eq!(v, Some(TagValue::from("val")));
    }

    #[test]
    fn test_binary_omega_roundtrip() {
        let binary_val: &[u8] = &[0xCA, 0xFE];
        let encoded = encode_omega_key(b"file.txt", b"key", Some(binary_val));
        let (s, k, v) = decode_omega_key(&encoded).unwrap();
        assert_eq!(s, "file.txt");
        assert_eq!(k, "key");
        let v = v.unwrap();
        assert_eq!(v.as_bytes(), binary_val);
    }

    #[test]
    fn test_binary_prefix_query() {
        let binary_val: &[u8] = &[0xCA, 0xFE];
        let key = encode_omega_key(b"file.txt", b"data", Some(binary_val));
        let prefix = encode_omega_value_prefix(binary_val);
        assert!(key.starts_with(&prefix));
    }

    #[test]
    fn test_utf8_backward_compat() {
        // UTF-8 data should use string encoding (type tag 0x02), not bytes (0x01)
        // This ensures backward compatibility with existing data
        let encoded = encode_alpha_key(b"file.txt", b"label", Some(b"rust"));
        let fields = decode(&encoded).unwrap();
        // All fields should be String variants (not Bytes)
        assert!(fields[0].as_str().is_some(), "subject should be String");
        assert!(fields[1].as_str().is_some(), "key should be String");
        assert!(fields[2].as_str().is_some(), "value should be String");
    }

    #[test]
    fn test_binary_uses_bytes_encoding() {
        // Non-UTF-8 data should use bytes encoding (type tag 0x01)
        let binary: &[u8] = &[0xFF, 0x00];
        let encoded = encode_alpha_key(b"file.txt", binary, Some(b"val"));
        let fields = decode(&encoded).unwrap();
        assert!(fields[0].as_str().is_some(), "UTF-8 subject → String");
        assert!(fields[1].as_bytes().is_some(), "binary key → Bytes");
        assert!(fields[2].as_str().is_some(), "UTF-8 value → String");
    }

    // ========================================================================
    // Search query parser tests
    // ========================================================================

    #[test]
    fn test_parse_search_empty() {
        assert!(parse_search_query("").is_empty());
        assert!(parse_search_query("   ").is_empty());
    }

    #[test]
    fn test_parse_search_bare_word() {
        let terms = parse_search_query("hello");
        assert_eq!(terms, vec![SearchTerm::BareWord("hello".to_owned())]);
    }

    #[test]
    fn test_parse_search_key_only() {
        let terms = parse_search_query("name:");
        assert_eq!(terms, vec![SearchTerm::KeyOnly("name".to_owned())]);
    }

    #[test]
    fn test_parse_search_value_only() {
        let terms = parse_search_query(":myfile.txt");
        assert_eq!(terms, vec![SearchTerm::ValueOnly("myfile.txt".to_owned())]);
    }

    #[test]
    fn test_parse_search_key_value() {
        let terms = parse_search_query("name:myfile.txt");
        assert_eq!(
            terms,
            vec![SearchTerm::KeyValue(
                "name".to_owned(),
                "myfile.txt".to_owned()
            )]
        );
    }

    #[test]
    fn test_parse_search_literal() {
        let terms = parse_search_query("\"hello world\"");
        assert_eq!(terms, vec![SearchTerm::Literal("hello world".to_owned())]);
    }

    #[test]
    fn test_parse_search_literal_with_colon() {
        // A quoted string containing a colon should be a literal search,
        // not a key:value pair.
        let terms = parse_search_query("\":key\"");
        assert_eq!(terms, vec![SearchTerm::Literal(":key".to_owned())]);
    }

    #[test]
    fn test_parse_search_quoted_key_value() {
        // "key:":":value" → KeyValue("key:", ":value")
        let terms = parse_search_query("\"key:\":\":value\"");
        assert_eq!(
            terms,
            vec![SearchTerm::KeyValue("key:".to_owned(), ":value".to_owned())]
        );
    }

    #[test]
    fn test_parse_search_multiple_terms() {
        let terms = parse_search_query("name: :high priority");
        assert_eq!(
            terms,
            vec![
                SearchTerm::KeyOnly("name".to_owned()),
                SearchTerm::ValueOnly("high".to_owned()),
                SearchTerm::BareWord("priority".to_owned()),
            ]
        );
    }

    #[test]
    fn test_parse_search_quoted_with_spaces() {
        let terms = parse_search_query("\"hello world\" name:foo");
        assert_eq!(
            terms,
            vec![
                SearchTerm::Literal("hello world".to_owned()),
                SearchTerm::KeyValue("name".to_owned(), "foo".to_owned()),
            ]
        );
    }

    #[test]
    fn test_parse_search_colon_only() {
        // A bare colon should not crash
        let terms = parse_search_query(":");
        assert_eq!(terms.len(), 1);
    }

    #[test]
    fn test_parse_search_empty_key_value() {
        // key: with trailing space
        let terms = parse_search_query("key: ");
        assert_eq!(terms, vec![SearchTerm::KeyOnly("key".to_owned())]);
    }

    #[test]
    fn test_parse_search_quoted_colon_in_key() {
        // Quoted key containing colon: "key:":":value"
        let terms = parse_search_query("\"key:\":\":value\"");
        assert_eq!(
            terms,
            vec![SearchTerm::KeyValue("key:".to_owned(), ":value".to_owned())]
        );
    }

    #[test]
    fn test_parse_search_bare_word_case() {
        let terms = parse_search_query("README");
        assert_eq!(terms, vec![SearchTerm::BareWord("README".to_owned())]);
    }

    #[test]
    fn test_tag_value_empty_string() {
        let tv = TagValue::from("");
        assert!(tv.is_utf8());
        assert_eq!(tv.as_str(), Some(""));
        assert!(tv.is_empty());
    }

    #[test]
    fn test_tag_value_display_binary() {
        let tv = TagValue::from(vec![0xFF, 0xFE]);
        assert!(!tv.is_utf8());
        let display = format!("{tv}");
        assert!(display.contains("binary"), "binary display: {display}");
    }

    #[test]
    fn test_tag_value_deref() {
        let tv = TagValue::from("hello");
        // Deref to [u8]
        assert_eq!(&*tv, b"hello");
        assert_eq!(tv.len(), 5);
    }
}
