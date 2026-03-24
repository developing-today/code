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
//! Keys use [`TupleEncoder`](crate::tuple::TupleEncoder) for sort-preserving
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
use iroh_docs::AuthorId;
use iroh_docs::NamespaceId;
use iroh_docs::api::Doc;
use iroh_docs::protocol::Docs;
use iroh_docs::store::Query;
use serde::{Deserialize, Serialize};

use crate::tuple::{TupleEncoder, decode};

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
            .finish()
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    /// The entity this tag describes (usually a filename).
    pub subject: String,
    /// The tag key (e.g., "label", "created", "pinned").
    pub key: String,
    /// Optional value. `None` for key-only tags like "pinned".
    pub value: Option<String>,
    /// Content hash of the entry data.
    pub hash: Hash,
    /// Entry timestamp (microseconds since epoch, from iroh-docs).
    pub timestamp: u64,
    /// Author who wrote this entry.
    pub author: AuthorId,
}

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
}

impl std::fmt::Debug for TagStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TagStore")
            .field("global", &self.global)
            .field("node", &self.node)
            .field("custom_count", &self.custom.len())
            .field("author", &self.author)
            .finish()
    }
}

// ============================================================================
// Key encoding
// ============================================================================

/// Encode an α (alpha) key: `(subject, key, value|null)`.
fn encode_alpha_key(subject: &str, key: &str, value: Option<&str>) -> Vec<u8> {
    let mut enc = TupleEncoder::new();
    enc.string(subject).string(key);
    match value {
        Some(v) => {
            enc.string(v);
        }
        None => {
            enc.null();
        }
    }
    enc.build()
}

/// Encode an Ω (omega) key: `(value|null, key, subject)` — inverted order.
fn encode_omega_key(subject: &str, key: &str, value: Option<&str>) -> Vec<u8> {
    let mut enc = TupleEncoder::new();
    match value {
        Some(v) => {
            enc.string(v);
        }
        None => {
            enc.null();
        }
    }
    enc.string(key).string(subject);
    enc.build()
}

/// Encode a prefix for α queries on a subject.
fn encode_alpha_subject_prefix(subject: &str) -> Vec<u8> {
    TupleEncoder::new().string(subject).build()
}

/// Encode a prefix for α queries on subject + key.
fn encode_alpha_subject_key_prefix(subject: &str, key: &str) -> Vec<u8> {
    TupleEncoder::new().string(subject).string(key).build()
}

/// Encode a prefix for Ω queries on a value.
fn encode_omega_value_prefix(value: &str) -> Vec<u8> {
    TupleEncoder::new().string(value).build()
}

/// Encode a prefix for Ω queries on null (key-only tags).
fn encode_omega_null_prefix() -> Vec<u8> {
    TupleEncoder::new().null().build()
}

/// Encode a prefix for Ω queries on value + key.
fn encode_omega_value_key_prefix(value: &str, key: &str) -> Vec<u8> {
    TupleEncoder::new().string(value).string(key).build()
}

/// Encode a prefix for Ω queries on null + key (key-only tags with specific key).
fn encode_omega_null_key_prefix(key: &str) -> Vec<u8> {
    TupleEncoder::new().null().string(key).build()
}

/// Decode an α key into `(subject, key, value?)`.
fn decode_alpha_key(raw: &[u8]) -> Result<(String, String, Option<String>)> {
    let fields = decode(raw)?;
    if fields.len() < 3 {
        bail!("α key has {} fields, expected 3", fields.len());
    }
    let subject = fields[0]
        .as_str()
        .context("α key field 0 (subject) is not a string")?
        .to_owned();
    let key = fields[1]
        .as_str()
        .context("α key field 1 (key) is not a string")?
        .to_owned();
    let value = if fields[2].is_null() {
        None
    } else {
        Some(
            fields[2]
                .as_str()
                .context("α key field 2 (value) is not a string")?
                .to_owned(),
        )
    };
    Ok((subject, key, value))
}

/// Decode an Ω key into `(subject, key, value?)`.
///
/// Ω keys are stored as `(value|null, key, subject)`, so we re-order
/// back to `(subject, key, value?)`.
fn decode_omega_key(raw: &[u8]) -> Result<(String, String, Option<String>)> {
    let fields = decode(raw)?;
    if fields.len() < 3 {
        bail!("Ω key has {} fields, expected 3", fields.len());
    }
    let value = if fields[0].is_null() {
        None
    } else {
        Some(
            fields[0]
                .as_str()
                .context("Ω key field 0 (value) is not a string")?
                .to_owned(),
        )
    };
    let key = fields[1]
        .as_str()
        .context("Ω key field 1 (key) is not a string")?
        .to_owned();
    let subject = fields[2]
        .as_str()
        .context("Ω key field 2 (subject) is not a string")?
        .to_owned();
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

        Ok(Self {
            global,
            node,
            custom,
            author,
            registry_path,
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
    /// * `subject` - The entity to tag (usually a filename)
    /// * `key` - The tag key
    /// * `value` - Optional tag value (`None` for key-only tags)
    /// * `data` - Payload bytes stored with the entry
    pub async fn set_tag(
        &self,
        ns: &NamespacePair,
        subject: &str,
        key: &str,
        value: Option<&str>,
        data: &[u8],
    ) -> Result<()> {
        let alpha_key = encode_alpha_key(subject, key, value);
        let omega_key = encode_omega_key(subject, key, value);

        // Write to α first, get the content hash.
        let hash = ns
            .alpha
            .set_bytes(self.author, alpha_key, data.to_vec())
            .await
            .context("writing α entry")?;

        // Write Ω with the same content hash.
        #[allow(clippy::cast_possible_truncation)]
        let size = data.len() as u64;
        ns.omega
            .set_hash(self.author, omega_key, hash, size)
            .await
            .context("writing Ω entry")?;

        Ok(())
    }

    /// Delete a specific tag from both α and Ω.
    ///
    /// Deletes entries matching the exact `(subject, key, value|null)` tuple.
    pub async fn del_tag(
        &self,
        ns: &NamespacePair,
        subject: &str,
        key: &str,
        value: Option<&str>,
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

        Ok(())
    }

    /// Delete all tags for a subject from both α and Ω.
    ///
    /// First queries α for all tags matching the subject prefix, then
    /// deletes the corresponding Ω entries individually.
    pub async fn del_all_tags(&self, ns: &NamespacePair, subject: &str) -> Result<usize> {
        // Find all tags for this subject so we can delete Ω entries.
        let tags = self.get_tags(ns, subject).await?;
        let count = tags.len();

        // Delete from Ω one by one (each has a different inverted key).
        for tag in &tags {
            let omega_key = encode_omega_key(subject, &tag.key, tag.value.as_deref());
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

        Ok(count)
    }
}

// ============================================================================
// Query operations
// ============================================================================

impl TagStore {
    /// Get all tags for a subject from an α namespace.
    pub async fn get_tags(&self, ns: &NamespacePair, subject: &str) -> Result<Vec<Tag>> {
        let prefix = encode_alpha_subject_prefix(subject);
        query_alpha_prefix(&ns.alpha, &prefix).await
    }

    /// Get tags for a subject with a specific key from an α namespace.
    pub async fn get_by_key(
        &self,
        ns: &NamespacePair,
        subject: &str,
        key: &str,
    ) -> Result<Vec<Tag>> {
        let prefix = encode_alpha_subject_key_prefix(subject, key);
        query_alpha_prefix(&ns.alpha, &prefix).await
    }

    /// Get a single exact tag from an α namespace.
    pub async fn get_exact(
        &self,
        ns: &NamespacePair,
        subject: &str,
        key: &str,
        value: Option<&str>,
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
    pub async fn find_by_value(&self, ns: &NamespacePair, value: &str) -> Result<Vec<Tag>> {
        let prefix = encode_omega_value_prefix(value);
        query_omega_prefix(&ns.omega, &prefix).await
    }

    /// Find all subjects with a specific `key=value` pair (via Ω).
    pub async fn find_by_key_value(
        &self,
        ns: &NamespacePair,
        key: &str,
        value: &str,
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
    pub async fn find_by_key_only(&self, ns: &NamespacePair, key: &str) -> Result<Vec<Tag>> {
        let prefix = encode_omega_null_key_prefix(key);
        query_omega_prefix(&ns.omega, &prefix).await
    }

    /// List all tags in an α namespace.
    pub async fn list_all(&self, ns: &NamespacePair) -> Result<Vec<Tag>> {
        let query = Query::all().build();
        let mut entries = ns.alpha.get_many(query).await?;
        let mut tags = Vec::new();
        while let Some(entry) = entries.try_next().await? {
            tags.push(entry_to_tag_alpha(&entry)?);
        }
        Ok(tags)
    }
}

// ============================================================================
// Internal query helpers
// ============================================================================

/// Query an α document by key prefix and decode results.
async fn query_alpha_prefix(doc: &Doc, prefix: &[u8]) -> Result<Vec<Tag>> {
    let query = Query::key_prefix(prefix.to_vec()).build();
    let mut entries = doc.get_many(query).await?;
    let mut tags = Vec::new();
    while let Some(entry) = entries.try_next().await? {
        tags.push(entry_to_tag_alpha(&entry)?);
    }
    Ok(tags)
}

/// Query an Ω document by key prefix and decode results.
async fn query_omega_prefix(doc: &Doc, prefix: &[u8]) -> Result<Vec<Tag>> {
    let query = Query::key_prefix(prefix.to_vec()).build();
    let mut entries = doc.get_many(query).await?;
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
async fn ensure_pair(docs: &Docs, pair_ids: &mut Option<PairIds>) -> Result<NamespacePair> {
    match pair_ids {
        Some(ids) if !ids.alpha.is_empty() && !ids.omega.is_empty() => {
            let alpha_id = parse_namespace_id(&ids.alpha)?;
            let omega_id = parse_namespace_id(&ids.omega)?;
            let alpha = docs
                .open(alpha_id)
                .await?
                .context("global α document not found in store")?;
            let omega = docs
                .open(omega_id)
                .await?
                .context("global Ω document not found in store")?;
            Ok(NamespacePair {
                alpha,
                omega,
                alpha_id,
                omega_id,
            })
        }
        _ => {
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
    }
}

/// Ensure an α/Ω pair from mutable `PairIds`, creating new docs if IDs are empty.
async fn ensure_pair_from_ids(docs: &Docs, ids: &mut PairIds) -> Result<NamespacePair> {
    if ids.alpha.is_empty() || ids.omega.is_empty() {
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
    } else {
        let alpha_id = parse_namespace_id(&ids.alpha)?;
        let omega_id = parse_namespace_id(&ids.omega)?;
        let alpha = docs
            .open(alpha_id)
            .await?
            .context("node α document not found in store")?;
        let omega = docs
            .open(omega_id)
            .await?
            .context("node Ω document not found in store")?;
        Ok(NamespacePair {
            alpha,
            omega,
            alpha_id,
            omega_id,
        })
    }
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
        let key = encode_alpha_key("README.md", "label", Some("rust"));
        let fields = decode(&key).unwrap();
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].as_str(), Some("README.md"));
        assert_eq!(fields[1].as_str(), Some("label"));
        assert_eq!(fields[2].as_str(), Some("rust"));
    }

    #[test]
    fn test_encode_alpha_key_without_value() {
        let key = encode_alpha_key("README.md", "pinned", None);
        let fields = decode(&key).unwrap();
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].as_str(), Some("README.md"));
        assert_eq!(fields[1].as_str(), Some("pinned"));
        assert!(fields[2].is_null());
    }

    #[test]
    fn test_encode_omega_key_with_value() {
        let key = encode_omega_key("README.md", "label", Some("rust"));
        let fields = decode(&key).unwrap();
        assert_eq!(fields.len(), 3);
        // Inverted order: (value, key, subject)
        assert_eq!(fields[0].as_str(), Some("rust"));
        assert_eq!(fields[1].as_str(), Some("label"));
        assert_eq!(fields[2].as_str(), Some("README.md"));
    }

    #[test]
    fn test_encode_omega_key_without_value() {
        let key = encode_omega_key("README.md", "pinned", None);
        let fields = decode(&key).unwrap();
        assert_eq!(fields.len(), 3);
        // Inverted: (null, key, subject)
        assert!(fields[0].is_null());
        assert_eq!(fields[1].as_str(), Some("pinned"));
        assert_eq!(fields[2].as_str(), Some("README.md"));
    }

    #[test]
    fn test_decode_alpha_key() {
        let raw = encode_alpha_key("file.txt", "tag", Some("v1"));
        let (subject, key, value) = decode_alpha_key(&raw).unwrap();
        assert_eq!(subject, "file.txt");
        assert_eq!(key, "tag");
        assert_eq!(value, Some("v1".to_owned()));
    }

    #[test]
    fn test_decode_alpha_key_null_value() {
        let raw = encode_alpha_key("file.txt", "pinned", None);
        let (subject, key, value) = decode_alpha_key(&raw).unwrap();
        assert_eq!(subject, "file.txt");
        assert_eq!(key, "pinned");
        assert_eq!(value, None);
    }

    #[test]
    fn test_decode_omega_key() {
        let raw = encode_omega_key("file.txt", "label", Some("rust"));
        let (subject, key, value) = decode_omega_key(&raw).unwrap();
        assert_eq!(subject, "file.txt");
        assert_eq!(key, "label");
        assert_eq!(value, Some("rust".to_owned()));
    }

    #[test]
    fn test_decode_omega_key_null() {
        let raw = encode_omega_key("file.txt", "pinned", None);
        let (subject, key, value) = decode_omega_key(&raw).unwrap();
        assert_eq!(subject, "file.txt");
        assert_eq!(key, "pinned");
        assert_eq!(value, None);
    }

    #[test]
    fn test_alpha_prefix_matches_subject() {
        let key = encode_alpha_key("README.md", "label", Some("rust"));
        let prefix = encode_alpha_subject_prefix("README.md");
        assert!(key.starts_with(&prefix));
    }

    #[test]
    fn test_alpha_prefix_matches_subject_key() {
        let key = encode_alpha_key("README.md", "label", Some("rust"));
        let prefix = encode_alpha_subject_key_prefix("README.md", "label");
        assert!(key.starts_with(&prefix));
    }

    #[test]
    fn test_alpha_prefix_does_not_match_other_subject() {
        let key = encode_alpha_key("README.md", "label", Some("rust"));
        let prefix = encode_alpha_subject_prefix("other.txt");
        assert!(!key.starts_with(&prefix));
    }

    #[test]
    fn test_omega_prefix_matches_value() {
        let key = encode_omega_key("README.md", "label", Some("rust"));
        let prefix = encode_omega_value_prefix("rust");
        assert!(key.starts_with(&prefix));
    }

    #[test]
    fn test_omega_prefix_matches_value_key() {
        let key = encode_omega_key("README.md", "label", Some("rust"));
        let prefix = encode_omega_value_key_prefix("rust", "label");
        assert!(key.starts_with(&prefix));
    }

    #[test]
    fn test_omega_null_prefix_matches_keyonly() {
        let key = encode_omega_key("README.md", "pinned", None);
        let prefix = encode_omega_null_prefix();
        assert!(key.starts_with(&prefix));
    }

    #[test]
    fn test_omega_null_prefix_does_not_match_valued() {
        let key = encode_omega_key("README.md", "label", Some("rust"));
        let prefix = encode_omega_null_prefix();
        assert!(!key.starts_with(&prefix));
    }

    #[test]
    fn test_omega_null_key_prefix() {
        let key = encode_omega_key("README.md", "pinned", None);
        let prefix = encode_omega_null_key_prefix("pinned");
        assert!(key.starts_with(&prefix));
    }

    #[test]
    fn test_omega_null_key_prefix_no_match() {
        let key = encode_omega_key("README.md", "pinned", None);
        let prefix = encode_omega_null_key_prefix("starred");
        assert!(!key.starts_with(&prefix));
    }

    #[test]
    fn test_alpha_sort_order() {
        let k1 = encode_alpha_key("a.txt", "label", Some("alpha"));
        let k2 = encode_alpha_key("a.txt", "label", Some("beta"));
        let k3 = encode_alpha_key("b.txt", "label", Some("alpha"));
        assert!(k1 < k2, "same subject+key, alpha < beta");
        assert!(k2 < k3, "a.txt < b.txt");
    }

    #[test]
    fn test_omega_sort_order() {
        let k1 = encode_omega_key("b.txt", "label", Some("alpha"));
        let k2 = encode_omega_key("a.txt", "label", Some("beta"));
        assert!(k1 < k2, "alpha < beta in Ω regardless of subject");
    }

    #[test]
    fn test_roundtrip_alpha_key() {
        let subjects = ["", "README.md", "path/to/file.txt", "日本語.md"];
        let keys = ["label", "created", ""];
        let values = [None, Some(""), Some("rust"), Some("日本語")];
        for subject in subjects {
            for key in keys {
                for value in &values {
                    let encoded = encode_alpha_key(subject, key, *value);
                    let (s, k, v) = decode_alpha_key(&encoded).unwrap();
                    assert_eq!(s, subject);
                    assert_eq!(k, key);
                    assert_eq!(v.as_deref(), *value);
                }
            }
        }
    }

    #[test]
    fn test_roundtrip_omega_key() {
        let subjects = ["", "README.md", "path/to/file.txt"];
        let keys = ["label", "created"];
        let values = [None, Some("rust"), Some("日本語")];
        for subject in subjects {
            for key in keys {
                for value in &values {
                    let encoded = encode_omega_key(subject, key, *value);
                    let (s, k, v) = decode_omega_key(&encoded).unwrap();
                    assert_eq!(s, subject);
                    assert_eq!(k, key);
                    assert_eq!(v.as_deref(), *value);
                }
            }
        }
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
}
