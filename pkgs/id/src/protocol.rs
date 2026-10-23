//! Network protocol types for remote node communication.
//!
//! This module defines the custom "meta" protocol used for peer-to-peer
//! metadata operations beyond basic blob transfer. While Iroh handles
//! blob content via its built-in protocol, this meta protocol enables:
//!
//! - **Tag management**: Create, list, delete, rename, and copy tags on remote nodes
//! - **Search operations**: Find blobs by name or hash with fuzzy matching
//! - **Metadata queries**: List all stored items on a remote node
//! - **Peer discovery**: Query a node's known peers from gossip-based discovery
//!
//! # Protocol Architecture
//!
//! ```text
//! ┌─────────────┐                    ┌─────────────┐
//! │   Client    │                    │   Server    │
//! │             │                    │             │
//! │  MetaRequest├───── QUIC ────────►│MetaProtocol │
//! │             │    (postcard)      │   handler   │
//! │             │◄───────────────────┤             │
//! │ MetaResponse│                    │             │
//! └─────────────┘                    └─────────────┘
//! ```
//!
//! Messages are serialized using [postcard](https://docs.rs/postcard) for
//! compact binary encoding. Each connection can handle multiple request/response
//! pairs using bidirectional QUIC streams.
//!
//! # Protocol Identifier
//!
//! The meta protocol uses the ALPN identifier defined in [`crate::META_ALPN`]:
//! `b"/iroh-meta/2"`. This allows nodes to negotiate the correct protocol handler.
//!
//! # Usage Example
//!
//! ```rust,ignore
//! use id::protocol::{MetaRequest, MetaResponse};
//!
//! // Create a request to find files matching "config"
//! let request = MetaRequest::Find {
//!     query: "config".to_string(),
//!     prefer_name: true,
//! };
//!
//! // Serialize and send over QUIC connection
//! let bytes = postcard::to_allocvec(&request)?;
//! send_stream.write_all(&bytes).await?;
//!
//! // Read and deserialize response
//! let response_bytes = recv_stream.read_to_end(64 * 1024).await?;
//! let response: MetaResponse = postcard::from_bytes(&response_bytes)?;
//! ```
//!
//! # Match Quality
//!
//! Search operations return results ranked by [`MatchKind`]:
//! - [`MatchKind::Exact`]: Query exactly equals the name/hash (best)
//! - [`MatchKind::Prefix`]: Name/hash starts with query (good)
//! - [`MatchKind::Contains`]: Name/hash contains query anywhere (okay)

use futures_lite::StreamExt;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh_base::EndpointId;
use iroh_blobs::{Hash, api::Store};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::access::AccessPolicy;
use crate::discovery::{PeerAnnouncement, PeerDiscovery};
use crate::fileops::{FileOpError, FileOps};
use crate::tags::TagStore;

/// Match quality for find/search operations.
///
/// Represents how closely a search query matches a blob's name or hash.
/// The variants are ordered by quality, with [`Exact`](MatchKind::Exact)
/// being the best match.
///
/// # Ordering
///
/// `MatchKind` implements `Ord` such that better matches compare less:
///
/// ```rust
/// use id::protocol::MatchKind;
///
/// assert!(MatchKind::Exact < MatchKind::Prefix);
/// assert!(MatchKind::Prefix < MatchKind::Contains);
/// ```
///
/// This allows sorting search results by quality using the natural ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MatchKind {
    /// Query exactly equals the target string.
    ///
    /// For example, query "config.json" matches name "config.json" exactly.
    Exact,
    /// Target string starts with the query.
    ///
    /// For example, query "config" matches name "config.json" as a prefix.
    Prefix,
    /// Target string contains the query somewhere.
    ///
    /// For example, query "fig" matches name "config.json" as contained.
    Contains,
}

/// A single match result from find/search operations.
///
/// Represents one blob that matched a search query, including metadata
/// about how well it matched and whether the match was against the
/// blob's name or its hash.
///
/// # Example
///
/// ```rust
/// use id::protocol::{FindMatch, MatchKind};
/// use iroh_blobs::Hash;
///
/// let m = FindMatch {
///     hash: Hash::from_bytes([0u8; 32]),
///     name: "example.txt".to_string(),
///     kind: MatchKind::Exact,
///     is_hash_match: false,
/// };
///
/// // This was a name match, not a hash match
/// assert!(!m.is_hash_match);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindMatch {
    /// The content hash of the matched blob.
    pub hash: Hash,
    /// The tag name associated with this blob.
    pub name: String,
    /// How well the query matched (exact, prefix, or contains).
    pub kind: MatchKind,
    /// Whether the match was against the hash (`true`) or name (`false`).
    ///
    /// When searching, both the name and hash are checked. This field
    /// indicates which one matched the query, useful for understanding
    /// the search result context.
    pub is_hash_match: bool,
}

/// A match result tagged with the query that produced it.
///
/// When searching with multiple queries, this struct associates each
/// match with the specific query that found it. This is used internally
/// for grouping and formatting multi-query search results.
///
/// # Example
///
/// ```rust
/// use id::protocol::{TaggedMatch, MatchKind};
/// use iroh_blobs::Hash;
///
/// let m = TaggedMatch {
///     query: "config".to_string(),
///     hash: Hash::from_bytes([0u8; 32]),
///     name: "config.json".to_string(),
///     kind: MatchKind::Prefix,
///     is_hash_match: false,
/// };
///
/// // The query "config" matched "config.json" as a prefix
/// assert_eq!(m.query, "config");
/// assert_eq!(m.kind, MatchKind::Prefix);
/// ```
#[derive(Debug, Clone)]
pub struct TaggedMatch {
    /// The search query that produced this match.
    pub query: String,
    /// The content hash of the matched blob.
    pub hash: Hash,
    /// The tag name associated with this blob.
    pub name: String,
    /// How well the query matched.
    pub kind: MatchKind,
    /// Whether the match was against the hash or name.
    pub is_hash_match: bool,
}

/// Requests that can be sent to a remote node via the meta protocol.
///
/// Each variant represents an operation that can be performed on a remote
/// node's blob store. The request is serialized with postcard and sent
/// over a QUIC bidirectional stream.
///
/// # Serialization
///
/// All variants are serializable for network transmission:
///
/// ```rust
/// use id::protocol::MetaRequest;
/// use iroh_blobs::Hash;
///
/// let req = MetaRequest::List;
/// let bytes = postcard::to_allocvec(&req).unwrap();
/// let decoded: MetaRequest = postcard::from_bytes(&bytes).unwrap();
/// ```
/// A metadata tag as carried on the wire.
///
/// Subject, key and value are raw bytes. The tag store is binary-safe, and the
/// v2 wire format no longer forces a lossy UTF-8 conversion on the way out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireTag {
    /// The subject (usually a filename).
    pub subject: Vec<u8>,
    /// The tag key.
    pub key: Vec<u8>,
    /// The tag value, `None` for key-only tags.
    pub value: Option<Vec<u8>>,
}

impl WireTag {
    /// Build a wire tag from UTF-8 strings (convenience for callers and tests).
    pub fn from_strs(subject: &str, key: &str, value: Option<&str>) -> Self {
        Self {
            subject: subject.as_bytes().to_vec(),
            key: key.as_bytes().to_vec(),
            value: value.map(|v| v.as_bytes().to_vec()),
        }
    }
}

impl From<crate::tags::Tag> for WireTag {
    fn from(t: crate::tags::Tag) -> Self {
        Self {
            subject: t.subject.into_bytes(),
            key: t.key.into_bytes(),
            value: t.value.map(crate::tags::TagValue::into_bytes),
        }
    }
}

/// Maximum size of a single request on the meta protocol (1 MiB).
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;

/// How long a server waits for a just-pushed blob to finish arriving before a
/// `Put` naming it is refused.
pub const DEFAULT_BLOB_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Requests to a remote node via the meta protocol (`/iroh-meta/2`).
///
/// **Compatibility rule:** postcard encodes enum variants by position. Within
/// a protocol version, new variants may only be *appended*; reordering,
/// inserting or changing a payload requires a new ALPN. The
/// `wire_format_is_pinned` test fixes every discriminant.
#[derive(Debug, Serialize, Deserialize)]
pub enum MetaRequest {
    /// Create or update a name on the remote node.
    ///
    /// Associates `filename` with `hash`. The blob content must already be on
    /// the remote (push it via the blobs protocol *first*): the server verifies
    /// the blob is present and refuses to create a dangling name.
    Put {
        /// The tag name to create or update.
        filename: String,
        /// The content hash to associate with this tag.
        hash: Hash,
    },
    /// Look up a name on the remote node, returning its hash if it exists.
    Get {
        /// The tag name to look up.
        filename: String,
    },
    /// List all names on the remote node as (hash, name) pairs.
    List,
    /// Delete a name from the remote node (hard delete).
    ///
    /// Removes the name, its metadata tags and its archive tags. The blob
    /// bytes remain until garbage collected.
    Delete {
        /// The tag name to delete.
        filename: String,
    },
    /// Rename a file on the remote node.
    ///
    /// Metadata follows the file; a file replaced at the destination and the
    /// original name are archived as `<name>.archive.<unix-ts>`.
    Rename {
        /// The current tag name.
        from: String,
        /// The new tag name.
        to: String,
    },
    /// Copy a file on the remote node.
    ///
    /// Creates `to` pointing at the same hash and duplicates the metadata.
    Copy {
        /// The source tag name.
        from: String,
        /// The destination tag name.
        to: String,
    },
    /// Search for names and hashes matching a query, ranked by match quality.
    Find {
        /// The search query (matched case-insensitively).
        query: String,
        /// If `true`, prioritize name matches over hash matches in results.
        prefer_name: bool,
    },
    /// Set a metadata tag (global namespace).
    SetTag {
        /// The subject (usually a filename) to tag.
        subject: Vec<u8>,
        /// The tag key (e.g. "author", "status").
        key: Vec<u8>,
        /// Optional tag value (arbitrary bytes).
        value: Option<Vec<u8>>,
    },
    /// Delete metadata tags (global namespace).
    DelTag {
        /// The subject (usually a filename).
        subject: Vec<u8>,
        /// The tag key to delete.
        key: Vec<u8>,
        /// A specific value to delete. `None` deletes **every** tag with this
        /// subject and key (including a value-less key-only tag).
        value: Option<Vec<u8>>,
    },
    /// List metadata tags for a subject (or every tag when `None`).
    GetTags {
        /// The subject (filename) to list tags for. `None` = list all.
        subject: Option<Vec<u8>>,
    },
    /// Search metadata tags with the structured query syntax.
    ///
    /// Supports `key:`, `:value`, `key:value`, `"literal"` and bare words;
    /// multiple terms are `AND`ed.
    SearchTags {
        /// The search query string.
        query: String,
    },
    /// Add `name`/`file` auto-tags to every blob name that lacks them.
    MigrateTags,
    /// Request the peers this node has discovered via gossip.
    ListPeers,
    /// Ask the node who it is and whether the caller may write to it.
    ///
    /// Lets clients give an actionable error instead of an opaque failure when
    /// a write is refused.
    Whoami,
}

impl MetaRequest {
    /// Whether this request mutates the store (and therefore needs write access).
    pub const fn is_write(&self) -> bool {
        matches!(
            self,
            Self::Put { .. }
                | Self::Delete { .. }
                | Self::Rename { .. }
                | Self::Copy { .. }
                | Self::SetTag { .. }
                | Self::DelTag { .. }
                | Self::MigrateTags
        )
    }
}

/// Responses from a remote node via the meta protocol.
///
/// Each variant corresponds to a [`MetaRequest`] variant. Any request may also
/// be answered with [`MetaResponse::Error`]. The same append-only rule as
/// [`MetaRequest`] applies.
#[derive(Debug, Serialize, Deserialize)]
pub enum MetaResponse {
    /// Response to [`MetaRequest::Put`].
    Put {
        /// Whether the name was created/updated.
        success: bool,
    },
    /// Response to [`MetaRequest::Get`].
    Get {
        /// The hash if found, or `None` if the name doesn't exist.
        hash: Option<Hash>,
    },
    /// Response to [`MetaRequest::List`].
    List {
        /// All names as (hash, name) pairs.
        items: Vec<(Hash, String)>,
    },
    /// Response to [`MetaRequest::Delete`].
    Delete {
        /// Whether the name existed and was deleted.
        success: bool,
    },
    /// Response to [`MetaRequest::Rename`].
    Rename {
        /// Whether the rename succeeded.
        success: bool,
    },
    /// Response to [`MetaRequest::Copy`].
    Copy {
        /// Whether the copy succeeded.
        success: bool,
    },
    /// Response to [`MetaRequest::Find`].
    Find {
        /// Matches, sorted by match quality.
        matches: Vec<FindMatch>,
    },
    /// Response to [`MetaRequest::SetTag`].
    SetTag {
        /// Whether the tag was set.
        success: bool,
    },
    /// Response to [`MetaRequest::DelTag`].
    DelTag {
        /// Whether at least one tag was deleted.
        success: bool,
        /// How many tags were deleted.
        deleted: u32,
    },
    /// Response to [`MetaRequest::GetTags`].
    GetTags {
        /// The matching tags.
        tags: Vec<WireTag>,
    },
    /// Response to [`MetaRequest::SearchTags`].
    SearchTags {
        /// The matching tags.
        tags: Vec<WireTag>,
    },
    /// Response to [`MetaRequest::MigrateTags`].
    MigrateTags {
        /// Number of subjects that were updated with auto-tags.
        migrated: usize,
    },
    /// Response to [`MetaRequest::ListPeers`].
    ListPeers {
        /// Known peers as announcements. Empty if peer discovery is off.
        peers: Vec<PeerAnnouncement>,
    },
    /// Response to [`MetaRequest::Whoami`].
    Whoami {
        /// The responding node's ID.
        node_id: EndpointId,
        /// The responding node's `id` version.
        version: String,
        /// Whether the *caller* may perform mutating requests.
        can_write: bool,
        /// Whether the node lets every peer write.
        open_writes: bool,
    },
    /// The request was refused or failed; `message` explains why.
    Error {
        /// Human-readable reason.
        message: String,
    },
}

impl MetaResponse {
    /// If this is an [`MetaResponse::Error`], return its message.
    pub fn error_message(&self) -> Option<&str> {
        match self {
            Self::Error { message } => Some(message),
            _ => None,
        }
    }
}

/// Protocol handler for the meta protocol.
///
/// Implements Iroh's [`ProtocolHandler`] trait. When a remote node connects
/// with [`crate::META_ALPN`], each bidirectional stream carries one
/// [`MetaRequest`] and is answered with one [`MetaResponse`]. Mutating
/// requests are checked against the [`AccessPolicy`] using the authenticated
/// identity of the connection.
///
/// ```text
/// Connection opened → identity = conn.remote_id()
///     ↓
/// Accept bidirectional stream
///     ↓
/// Read request → authorize (writes) → handle → send response
///     ↓
/// Loop until connection closed
/// ```
#[derive(Clone, Debug)]
pub struct MetaProtocol {
    /// The blob store used for name operations.
    pub store: Store,
    /// Optional peer discovery table for the `ListPeers` RPC.
    pub peer_discovery: Option<PeerDiscovery>,
    /// Metadata tag store (iroh-docs backed).
    pub tag_store: Arc<TagStore>,
    /// Who may perform mutating requests.
    pub access: AccessPolicy,
    /// This node's ID (reported by `Whoami`).
    pub node_id: EndpointId,
    /// How long a `Put` waits for a blob that is still arriving (see
    /// [`FileOps::with_blob_wait`]). Zero for in-process use.
    pub blob_wait: std::time::Duration,
}

impl MetaProtocol {
    /// Creates a new meta protocol handler.
    ///
    /// Returns an `Arc` for registration with Iroh's router.
    pub fn new(
        store: &Store,
        peer_discovery: Option<PeerDiscovery>,
        tag_store: Arc<TagStore>,
        access: AccessPolicy,
        node_id: EndpointId,
    ) -> Arc<Self> {
        Arc::new(Self {
            store: store.clone(),
            peer_discovery,
            tag_store,
            access,
            node_id,
            blob_wait: DEFAULT_BLOB_WAIT,
        })
    }

    /// Determines the match quality of a needle in a haystack.
    ///
    /// Returns the best applicable [`MatchKind`], or `None` if no match.
    /// Matching is case-sensitive; callers lowercase both for case-insensitive
    /// matching. Priority: exact, then prefix, then contains.
    fn match_kind(haystack: &str, needle: &str) -> Option<MatchKind> {
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

    /// Handle one request on behalf of `remote` and produce the response.
    ///
    /// This is the whole server-side behaviour of the protocol, independent of
    /// QUIC, so it can be exercised directly in tests.
    pub async fn handle(&self, remote: &EndpointId, req: MetaRequest) -> MetaResponse {
        if req.is_write() && !self.access.can_write(remote) {
            return MetaResponse::Error {
                message: format!(
                    "permission denied: node {remote} may not modify this store \
                     (ask the owner to run `id serve --allow-node {remote}`)"
                ),
            };
        }
        let ops = FileOps::new(&self.store, &self.tag_store).with_blob_wait(self.blob_wait);
        let ts = &self.tag_store;
        let ns = &ts.global;
        match req {
            MetaRequest::Put { filename, hash } => match ops.put(&filename, hash).await {
                Ok(()) => MetaResponse::Put { success: true },
                Err(e) => MetaResponse::Error {
                    message: e.to_string(),
                },
            },
            MetaRequest::Get { filename } => {
                let hash = self
                    .store
                    .tags()
                    .get(&filename)
                    .await
                    .ok()
                    .flatten()
                    .map(|t| t.hash);
                MetaResponse::Get { hash }
            }
            MetaRequest::List => {
                let mut items = Vec::new();
                if let Ok(mut list) = self.store.tags().list().await {
                    while let Some(Ok(item)) = list.next().await {
                        let name = String::from_utf8_lossy(item.name.as_ref()).into_owned();
                        items.push((item.hash, name));
                    }
                }
                MetaResponse::List { items }
            }
            MetaRequest::Delete { filename } => match ops.delete(&filename).await {
                Ok(outcome) => MetaResponse::Delete {
                    success: outcome.existed,
                },
                Err(e) => MetaResponse::Error {
                    message: e.to_string(),
                },
            },
            MetaRequest::Rename { from, to } => match ops.rename(&from, &to, true).await {
                Ok(_) => MetaResponse::Rename { success: true },
                Err(FileOpError::NotFound(_)) => MetaResponse::Rename { success: false },
                Err(e) => MetaResponse::Error {
                    message: e.to_string(),
                },
            },
            MetaRequest::Copy { from, to } => match ops.copy(&from, &to).await {
                Ok(_) => MetaResponse::Copy { success: true },
                Err(FileOpError::NotFound(_)) => MetaResponse::Copy { success: false },
                Err(e) => MetaResponse::Error {
                    message: e.to_string(),
                },
            },
            MetaRequest::Find { query, prefer_name } => MetaResponse::Find {
                matches: self.find(&query, prefer_name).await,
            },
            MetaRequest::SetTag {
                subject,
                key,
                value,
            } => match ts.set_tag(ns, &subject, &key, value.as_deref(), b"").await {
                Ok(()) => MetaResponse::SetTag { success: true },
                Err(e) => MetaResponse::Error {
                    message: format!("{e:#}"),
                },
            },
            MetaRequest::DelTag {
                subject,
                key,
                value,
            } => match ts
                .delete_matching(ns, &subject, &key, value.as_deref())
                .await
            {
                Ok(n) => MetaResponse::DelTag {
                    success: n > 0,
                    deleted: u32::try_from(n).unwrap_or(u32::MAX),
                },
                Err(e) => MetaResponse::Error {
                    message: format!("{e:#}"),
                },
            },
            MetaRequest::GetTags { subject } => {
                let result = match &subject {
                    Some(s) => ts.get_tags(ns, s).await,
                    None => ts.list_all(ns).await,
                };
                match result {
                    Ok(list) => MetaResponse::GetTags {
                        tags: list.into_iter().map(WireTag::from).collect(),
                    },
                    Err(e) => MetaResponse::Error {
                        message: format!("{e:#}"),
                    },
                }
            }
            MetaRequest::SearchTags { query } => match ts.search_by_query(ns, &query).await {
                Ok(list) => MetaResponse::SearchTags {
                    tags: list.into_iter().map(WireTag::from).collect(),
                },
                Err(e) => MetaResponse::Error {
                    message: format!("{e:#}"),
                },
            },
            MetaRequest::MigrateTags => match ts.migrate_tags(&self.store, ns).await {
                Ok(migrated) => MetaResponse::MigrateTags { migrated },
                Err(e) => MetaResponse::Error {
                    message: format!("{e:#}"),
                },
            },
            MetaRequest::ListPeers => {
                let peers = self
                    .peer_discovery
                    .as_ref()
                    .map(|pd| pd.peers().into_iter().map(|pi| pi.announcement).collect())
                    .unwrap_or_default();
                MetaResponse::ListPeers { peers }
            }
            MetaRequest::Whoami => MetaResponse::Whoami {
                node_id: self.node_id,
                version: env!("CARGO_PKG_VERSION").to_owned(),
                can_write: self.access.can_write(remote),
                open_writes: self.access.is_open(),
            },
        }
    }

    /// Rank blob names and hashes against `query`.
    async fn find(&self, query: &str, prefer_name: bool) -> Vec<FindMatch> {
        let mut matches = Vec::new();
        let query_lower = query.to_lowercase();
        if let Ok(mut list) = self.store.tags().list().await {
            while let Some(Ok(item)) = list.next().await {
                let name = String::from_utf8_lossy(item.name.as_ref()).into_owned();
                let hash_str = item.hash.to_string();
                if let Some(kind) = Self::match_kind(&name.to_lowercase(), &query_lower) {
                    matches.push(FindMatch {
                        hash: item.hash,
                        name,
                        kind,
                        is_hash_match: false,
                    });
                } else if let Some(kind) = Self::match_kind(&hash_str, &query_lower) {
                    matches.push(FindMatch {
                        hash: item.hash,
                        name,
                        kind,
                        is_hash_match: true,
                    });
                }
            }
        }
        // Best kind first; ties prefer name matches (`prefer_name`) or hash matches.
        matches.sort_by(|a, b| {
            a.kind.cmp(&b.kind).then_with(|| {
                if prefer_name {
                    a.is_hash_match.cmp(&b.is_hash_match)
                } else {
                    b.is_hash_match.cmp(&a.is_hash_match)
                }
            })
        });
        matches
    }
}

impl ProtocolHandler for MetaProtocol {
    /// Serves requests on a connection until the peer closes it.
    ///
    /// # Errors
    ///
    /// Returns `AcceptError` if a response cannot be serialized or written.
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let remote = conn.remote_id();
        while let Ok((mut send, mut recv)) = conn.accept_bi().await {
            let resp = match recv.read_to_end(MAX_REQUEST_BYTES).await {
                Ok(buf) => match postcard::from_bytes::<MetaRequest>(&buf) {
                    Ok(req) => self.handle(&remote, req).await,
                    Err(e) => MetaResponse::Error {
                        message: format!("malformed request: {e}"),
                    },
                },
                Err(e) => MetaResponse::Error {
                    message: format!("could not read request: {e}"),
                },
            };
            let bytes = postcard::to_allocvec(&resp).map_err(AcceptError::from_err)?;
            send.write_all(&bytes)
                .await
                .map_err(AcceptError::from_err)?;
            send.finish()?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn test_match_kind_exact() {
        assert_eq!(
            MetaProtocol::match_kind("hello", "hello"),
            Some(MatchKind::Exact)
        );
    }

    #[test]
    fn test_match_kind_prefix() {
        assert_eq!(
            MetaProtocol::match_kind("hello world", "hello"),
            Some(MatchKind::Prefix)
        );
    }

    #[test]
    fn test_match_kind_contains() {
        assert_eq!(
            MetaProtocol::match_kind("say hello", "hello"),
            Some(MatchKind::Contains)
        );
    }

    #[test]
    fn test_match_kind_none() {
        assert_eq!(MetaProtocol::match_kind("goodbye", "hello"), None);
    }

    #[test]
    fn test_match_kind_ordering() {
        // Exact < Prefix < Contains
        assert!(MatchKind::Exact < MatchKind::Prefix);
        assert!(MatchKind::Prefix < MatchKind::Contains);
    }

    #[test]
    fn test_match_kind_empty_string() {
        // Empty string matches as exact with empty
        assert_eq!(MetaProtocol::match_kind("", ""), Some(MatchKind::Exact));
        // Empty needle: starts_with("") is true, so returns Prefix
        assert_eq!(
            MetaProtocol::match_kind("hello", ""),
            Some(MatchKind::Prefix)
        );
        // Empty haystack with non-empty needle
        assert_eq!(MetaProtocol::match_kind("", "hello"), None);
    }

    #[test]
    fn test_match_kind_case_sensitive() {
        assert_eq!(MetaProtocol::match_kind("Hello", "hello"), None);
        assert_eq!(MetaProtocol::match_kind("HELLO", "hello"), None);
    }

    #[test]
    fn test_match_kind_special_chars() {
        assert_eq!(
            MetaProtocol::match_kind("test.file.txt", "test.file"),
            Some(MatchKind::Prefix)
        );
        assert_eq!(
            MetaProtocol::match_kind("path/to/file", "to"),
            Some(MatchKind::Contains)
        );
    }

    #[test]
    fn test_find_match_struct() {
        let hash = Hash::from_bytes([0u8; 32]);
        let m = FindMatch {
            hash,
            name: "test.txt".to_owned(),
            kind: MatchKind::Exact,
            is_hash_match: false,
        };
        assert_eq!(m.name, "test.txt");
        assert_eq!(m.kind, MatchKind::Exact);
        assert!(!m.is_hash_match);
    }

    #[test]
    fn test_tagged_match_struct() {
        let hash = Hash::from_bytes([0u8; 32]);
        let m = TaggedMatch {
            query: "test".to_owned(),
            hash,
            name: "test.txt".to_owned(),
            kind: MatchKind::Prefix,
            is_hash_match: true,
        };
        assert_eq!(m.query, "test");
        assert_eq!(m.kind, MatchKind::Prefix);
        assert!(m.is_hash_match);
    }

    #[test]
    fn test_meta_request_serialization() {
        // Test Put
        let req = MetaRequest::Put {
            filename: "test.txt".to_owned(),
            hash: Hash::from_bytes([0u8; 32]),
        };
        let bytes = postcard::to_allocvec(&req).unwrap();
        let decoded: MetaRequest = postcard::from_bytes(&bytes).unwrap();
        match decoded {
            MetaRequest::Put { filename, .. } => assert_eq!(filename, "test.txt"),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_meta_request_get_serialization() {
        let req = MetaRequest::Get {
            filename: "myfile.txt".to_owned(),
        };
        let bytes = postcard::to_allocvec(&req).unwrap();
        let decoded: MetaRequest = postcard::from_bytes(&bytes).unwrap();
        match decoded {
            MetaRequest::Get { filename } => assert_eq!(filename, "myfile.txt"),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_meta_request_list_serialization() {
        let req = MetaRequest::List;
        let bytes = postcard::to_allocvec(&req).unwrap();
        let decoded: MetaRequest = postcard::from_bytes(&bytes).unwrap();
        assert!(matches!(decoded, MetaRequest::List));
    }

    #[test]
    fn test_meta_request_delete_serialization() {
        let req = MetaRequest::Delete {
            filename: "to_delete.txt".to_owned(),
        };
        let bytes = postcard::to_allocvec(&req).unwrap();
        let decoded: MetaRequest = postcard::from_bytes(&bytes).unwrap();
        match decoded {
            MetaRequest::Delete { filename } => assert_eq!(filename, "to_delete.txt"),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_meta_request_rename_serialization() {
        let req = MetaRequest::Rename {
            from: "old.txt".to_owned(),
            to: "new.txt".to_owned(),
        };
        let bytes = postcard::to_allocvec(&req).unwrap();
        let decoded: MetaRequest = postcard::from_bytes(&bytes).unwrap();
        match decoded {
            MetaRequest::Rename { from, to } => {
                assert_eq!(from, "old.txt");
                assert_eq!(to, "new.txt");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_meta_request_copy_serialization() {
        let req = MetaRequest::Copy {
            from: "source.txt".to_owned(),
            to: "dest.txt".to_owned(),
        };
        let bytes = postcard::to_allocvec(&req).unwrap();
        let decoded: MetaRequest = postcard::from_bytes(&bytes).unwrap();
        match decoded {
            MetaRequest::Copy { from, to } => {
                assert_eq!(from, "source.txt");
                assert_eq!(to, "dest.txt");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_meta_request_find_serialization() {
        let req = MetaRequest::Find {
            query: "search term".to_owned(),
            prefer_name: true,
        };
        let bytes = postcard::to_allocvec(&req).unwrap();
        let decoded: MetaRequest = postcard::from_bytes(&bytes).unwrap();
        match decoded {
            MetaRequest::Find { query, prefer_name } => {
                assert_eq!(query, "search term");
                assert!(prefer_name);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_meta_response_put_serialization() {
        let resp = MetaResponse::Put { success: true };
        let bytes = postcard::to_allocvec(&resp).unwrap();
        let decoded: MetaResponse = postcard::from_bytes(&bytes).unwrap();
        match decoded {
            MetaResponse::Put { success } => assert!(success),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_meta_response_get_serialization() {
        let hash = Hash::from_bytes([1u8; 32]);
        let resp = MetaResponse::Get { hash: Some(hash) };
        let bytes = postcard::to_allocvec(&resp).unwrap();
        let decoded: MetaResponse = postcard::from_bytes(&bytes).unwrap();
        match decoded {
            MetaResponse::Get { hash: h } => {
                assert!(h.is_some());
                assert_eq!(h.unwrap(), hash);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_meta_response_get_none_serialization() {
        let resp = MetaResponse::Get { hash: None };
        let bytes = postcard::to_allocvec(&resp).unwrap();
        let decoded: MetaResponse = postcard::from_bytes(&bytes).unwrap();
        match decoded {
            MetaResponse::Get { hash } => assert!(hash.is_none()),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_meta_response_list_serialization() {
        let hash1 = Hash::from_bytes([1u8; 32]);
        let hash2 = Hash::from_bytes([2u8; 32]);
        let resp = MetaResponse::List {
            items: vec![
                (hash1, "file1.txt".to_owned()),
                (hash2, "file2.txt".to_owned()),
            ],
        };
        let bytes = postcard::to_allocvec(&resp).unwrap();
        let decoded: MetaResponse = postcard::from_bytes(&bytes).unwrap();
        match decoded {
            MetaResponse::List { items } => {
                assert_eq!(items.len(), 2);
                assert_eq!(items[0].1, "file1.txt");
                assert_eq!(items[1].1, "file2.txt");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_meta_response_find_serialization() {
        let hash = Hash::from_bytes([0u8; 32]);
        let matches = vec![FindMatch {
            hash,
            name: "found.txt".to_owned(),
            kind: MatchKind::Exact,
            is_hash_match: false,
        }];
        let resp = MetaResponse::Find { matches };
        let bytes = postcard::to_allocvec(&resp).unwrap();
        let decoded: MetaResponse = postcard::from_bytes(&bytes).unwrap();
        match decoded {
            MetaResponse::Find { matches } => {
                assert_eq!(matches.len(), 1);
                assert_eq!(matches[0].name, "found.txt");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_match_kind_serialization() {
        for kind in [MatchKind::Exact, MatchKind::Prefix, MatchKind::Contains] {
            let bytes = postcard::to_allocvec(&kind).unwrap();
            let decoded: MatchKind = postcard::from_bytes(&bytes).unwrap();
            assert_eq!(decoded, kind);
        }
    }

    #[test]
    fn test_find_match_serialization() {
        let hash = Hash::from_bytes([5u8; 32]);
        let m = FindMatch {
            hash,
            name: "serialized.txt".to_owned(),
            kind: MatchKind::Contains,
            is_hash_match: true,
        };
        let bytes = postcard::to_allocvec(&m).unwrap();
        let decoded: FindMatch = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(decoded.hash, hash);
        assert_eq!(decoded.name, "serialized.txt");
        assert_eq!(decoded.kind, MatchKind::Contains);
        assert!(decoded.is_hash_match);
    }

    #[test]
    fn test_meta_request_list_peers_serialization() {
        let req = MetaRequest::ListPeers;
        let bytes = postcard::to_allocvec(&req).unwrap();
        let decoded: MetaRequest = postcard::from_bytes(&bytes).unwrap();
        assert!(matches!(decoded, MetaRequest::ListPeers));
    }

    #[test]
    fn test_meta_response_list_peers_empty_serialization() {
        let resp = MetaResponse::ListPeers { peers: vec![] };
        let bytes = postcard::to_allocvec(&resp).unwrap();
        let decoded: MetaResponse = postcard::from_bytes(&bytes).unwrap();
        match decoded {
            MetaResponse::ListPeers { peers } => assert!(peers.is_empty()),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_meta_response_list_peers_with_peers_serialization() {
        use iroh_base::SecretKey;

        let node_id_1 = SecretKey::from_bytes(&[1u8; 32]).public();
        let node_id_2 = SecretKey::from_bytes(&[2u8; 32]).public();

        let peers = vec![
            PeerAnnouncement {
                node_id: node_id_1,
                name: Some("peer-alpha".to_owned()),
                blob_count: 42,
                timestamp_secs: 1_700_000_000,
            },
            PeerAnnouncement {
                node_id: node_id_2,
                name: None,
                blob_count: 0,
                timestamp_secs: 1_700_000_030,
            },
        ];
        let resp = MetaResponse::ListPeers { peers };
        let bytes = postcard::to_allocvec(&resp).unwrap();
        let decoded: MetaResponse = postcard::from_bytes(&bytes).unwrap();
        match decoded {
            MetaResponse::ListPeers {
                peers: decoded_peers,
            } => {
                assert_eq!(decoded_peers.len(), 2);
                assert_eq!(decoded_peers[0].node_id, node_id_1);
                assert_eq!(decoded_peers[0].name, Some("peer-alpha".to_owned()));
                assert_eq!(decoded_peers[0].blob_count, 42);
                assert_eq!(decoded_peers[1].node_id, node_id_2);
                assert_eq!(decoded_peers[1].name, None);
                assert_eq!(decoded_peers[1].blob_count, 0);
            }
            _ => panic!("Wrong variant"),
        }
    }

    // ========================================================================
    // Wire format
    // ========================================================================

    fn eid(n: u8) -> EndpointId {
        iroh_base::SecretKey::from_bytes(&[n; 32]).public()
    }

    #[test]
    fn wire_format_is_pinned() {
        // postcard encodes the enum variant index as the first byte. These
        // indices are the wire format of `/iroh-meta/2`: within a version only
        // append new variants; anything else needs a new ALPN.
        let h = Hash::from_bytes([0u8; 32]);
        let s = String::from("x");
        let requests: Vec<(MetaRequest, u8)> = vec![
            (
                MetaRequest::Put {
                    filename: s.clone(),
                    hash: h,
                },
                0,
            ),
            (
                MetaRequest::Get {
                    filename: s.clone(),
                },
                1,
            ),
            (MetaRequest::List, 2),
            (
                MetaRequest::Delete {
                    filename: s.clone(),
                },
                3,
            ),
            (
                MetaRequest::Rename {
                    from: s.clone(),
                    to: s.clone(),
                },
                4,
            ),
            (
                MetaRequest::Copy {
                    from: s.clone(),
                    to: s.clone(),
                },
                5,
            ),
            (
                MetaRequest::Find {
                    query: s.clone(),
                    prefer_name: false,
                },
                6,
            ),
            (
                MetaRequest::SetTag {
                    subject: vec![],
                    key: vec![],
                    value: None,
                },
                7,
            ),
            (
                MetaRequest::DelTag {
                    subject: vec![],
                    key: vec![],
                    value: None,
                },
                8,
            ),
            (MetaRequest::GetTags { subject: None }, 9),
            (MetaRequest::SearchTags { query: s.clone() }, 10),
            (MetaRequest::MigrateTags, 11),
            (MetaRequest::ListPeers, 12),
            (MetaRequest::Whoami, 13),
        ];
        for (req, idx) in &requests {
            let bytes = postcard::to_allocvec(req).unwrap();
            assert_eq!(bytes[0], *idx, "request {req:?}");
        }
        let responses: Vec<(MetaResponse, u8)> = vec![
            (MetaResponse::Put { success: true }, 0),
            (MetaResponse::Get { hash: None }, 1),
            (MetaResponse::List { items: vec![] }, 2),
            (MetaResponse::Delete { success: true }, 3),
            (MetaResponse::Rename { success: true }, 4),
            (MetaResponse::Copy { success: true }, 5),
            (MetaResponse::Find { matches: vec![] }, 6),
            (MetaResponse::SetTag { success: true }, 7),
            (
                MetaResponse::DelTag {
                    success: true,
                    deleted: 1,
                },
                8,
            ),
            (MetaResponse::GetTags { tags: vec![] }, 9),
            (MetaResponse::SearchTags { tags: vec![] }, 10),
            (MetaResponse::MigrateTags { migrated: 0 }, 11),
            (MetaResponse::ListPeers { peers: vec![] }, 12),
            (
                MetaResponse::Whoami {
                    node_id: eid(1),
                    version: "0".to_owned(),
                    can_write: true,
                    open_writes: false,
                },
                13,
            ),
            (MetaResponse::Error { message: s }, 14),
        ];
        for (resp, idx) in &responses {
            let bytes = postcard::to_allocvec(resp).unwrap();
            assert_eq!(bytes[0], *idx, "response {resp:?}");
        }
    }

    #[test]
    fn is_write_classifies_every_request() {
        let h = Hash::from_bytes([0u8; 32]);
        let s = String::from("x");
        for req in [
            MetaRequest::Put {
                filename: s.clone(),
                hash: h,
            },
            MetaRequest::Delete {
                filename: s.clone(),
            },
            MetaRequest::Rename {
                from: s.clone(),
                to: s.clone(),
            },
            MetaRequest::Copy {
                from: s.clone(),
                to: s.clone(),
            },
            MetaRequest::SetTag {
                subject: vec![],
                key: vec![],
                value: None,
            },
            MetaRequest::DelTag {
                subject: vec![],
                key: vec![],
                value: None,
            },
            MetaRequest::MigrateTags,
        ] {
            assert!(req.is_write(), "{req:?} must require write access");
        }
        for req in [
            MetaRequest::Get {
                filename: s.clone(),
            },
            MetaRequest::List,
            MetaRequest::Find {
                query: s.clone(),
                prefer_name: true,
            },
            MetaRequest::GetTags { subject: None },
            MetaRequest::SearchTags { query: s },
            MetaRequest::ListPeers,
            MetaRequest::Whoami,
        ] {
            assert!(!req.is_write(), "{req:?} is read-only");
        }
    }

    #[test]
    fn wire_tag_keeps_binary_values_intact() {
        let tag = WireTag {
            subject: b"f".to_vec(),
            key: b"k".to_vec(),
            value: Some(vec![0xff, 0xfe, 0x00, 0x80]),
        };
        let bytes = postcard::to_allocvec(&tag).unwrap();
        let back: WireTag = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(back, tag);
    }

    // ========================================================================
    // Handler behaviour against real iroh-blobs + iroh-docs (in memory)
    // ========================================================================

    use crate::local::LocalNode;

    async fn put(node: &LocalNode, name: &str, data: &[u8]) -> Hash {
        // A temp tag protects the blob without leaving an `auto-*` tag behind.
        let guard = node
            .blobs()
            .add_bytes(data.to_vec())
            .temp_tag()
            .await
            .unwrap();
        let hash = guard.hash();
        let resp = node
            .request(MetaRequest::Put {
                filename: name.to_owned(),
                hash,
            })
            .await;
        assert!(
            matches!(resp, MetaResponse::Put { success: true }),
            "{resp:?}"
        );
        hash
    }

    async fn set_tag(node: &LocalNode, subject: &str, key: &str, value: Option<&[u8]>) {
        let resp = node
            .request(MetaRequest::SetTag {
                subject: subject.as_bytes().to_vec(),
                key: key.as_bytes().to_vec(),
                value: value.map(<[u8]>::to_vec),
            })
            .await;
        assert!(
            matches!(resp, MetaResponse::SetTag { success: true }),
            "{resp:?}"
        );
    }

    async fn tags_of(node: &LocalNode, subject: &str) -> Vec<WireTag> {
        match node
            .request(MetaRequest::GetTags {
                subject: Some(subject.as_bytes().to_vec()),
            })
            .await
        {
            MetaResponse::GetTags { tags } => tags,
            other => panic!("unexpected {other:?}"),
        }
    }

    fn has_tag(tags: &[WireTag], key: &str, value: Option<&str>) -> bool {
        tags.iter()
            .any(|t| t.key == key.as_bytes() && t.value.as_deref() == value.map(str::as_bytes))
    }

    async fn names(node: &LocalNode) -> Vec<String> {
        match node.request(MetaRequest::List).await {
            MetaResponse::List { items } => items.into_iter().map(|(_, n)| n).collect(),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn put_refuses_a_blob_the_store_does_not_have() {
        let node = LocalNode::open(true).await.unwrap();
        let missing = Hash::from_bytes([7u8; 32]);
        let resp = node
            .request(MetaRequest::Put {
                filename: "ghost.txt".to_owned(),
                hash: missing,
            })
            .await;
        let msg = resp.error_message().expect("must be an error");
        assert!(msg.contains("not present"), "{msg}");
        assert!(
            names(&node).await.is_empty(),
            "no dangling name may be created"
        );
        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn put_records_created_modified_and_identity_tags() {
        let node = LocalNode::open(true).await.unwrap();
        put(&node, "dir/a.txt", b"hello").await;
        let tags = tags_of(&node, "dir/a.txt").await;
        assert!(tags.iter().any(|t| t.key == b"created"));
        assert!(tags.iter().any(|t| t.key == b"modified"));
        assert!(has_tag(&tags, "name", Some("dir/a.txt")));
        assert!(has_tag(&tags, "file", Some("a.txt")));
        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn deltag_without_value_deletes_every_value_of_the_key() {
        // Regression: the handler used to treat `None` as "the key-only tag",
        // so `id tag del FILE KEY` reported success and deleted nothing.
        let node = LocalNode::open(true).await.unwrap();
        put(&node, "f.md", b"x").await;
        set_tag(&node, "f.md", "color", Some(b"blue")).await;
        set_tag(&node, "f.md", "color", Some(b"green")).await;
        set_tag(&node, "f.md", "color", None).await;
        set_tag(&node, "f.md", "keep", Some(b"me")).await;

        let resp = node
            .request(MetaRequest::DelTag {
                subject: b"f.md".to_vec(),
                key: b"color".to_vec(),
                value: None,
            })
            .await;
        assert!(
            matches!(
                resp,
                MetaResponse::DelTag {
                    success: true,
                    deleted: 3
                }
            ),
            "{resp:?}"
        );
        let tags = tags_of(&node, "f.md").await;
        assert!(!tags.iter().any(|t| t.key == b"color"), "{tags:?}");
        assert!(has_tag(&tags, "keep", Some("me")));
        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn deltag_with_value_deletes_only_that_value() {
        let node = LocalNode::open(true).await.unwrap();
        put(&node, "f.md", b"x").await;
        set_tag(&node, "f.md", "color", Some(b"blue")).await;
        set_tag(&node, "f.md", "color", Some(b"green")).await;
        let resp = node
            .request(MetaRequest::DelTag {
                subject: b"f.md".to_vec(),
                key: b"color".to_vec(),
                value: Some(b"blue".to_vec()),
            })
            .await;
        assert!(
            matches!(
                resp,
                MetaResponse::DelTag {
                    success: true,
                    deleted: 1
                }
            ),
            "{resp:?}"
        );
        let tags = tags_of(&node, "f.md").await;
        assert!(has_tag(&tags, "color", Some("green")));
        assert!(!has_tag(&tags, "color", Some("blue")));

        let resp = node
            .request(MetaRequest::DelTag {
                subject: b"f.md".to_vec(),
                key: b"color".to_vec(),
                value: Some(b"absent".to_vec()),
            })
            .await;
        assert!(
            matches!(
                resp,
                MetaResponse::DelTag {
                    success: false,
                    deleted: 0
                }
            ),
            "{resp:?}"
        );
        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn binary_tag_values_survive_the_round_trip() {
        let node = LocalNode::open(true).await.unwrap();
        put(&node, "blob.bin", b"x").await;
        let value = [0xffu8, 0xfe, 0x00, 0x80, b'a'];
        set_tag(&node, "blob.bin", "checksum", Some(&value)).await;
        let tags = tags_of(&node, "blob.bin").await;
        let got = tags
            .iter()
            .find(|t| t.key == b"checksum")
            .and_then(|t| t.value.clone())
            .expect("checksum tag");
        assert_eq!(
            got, value,
            "binary value must not be mangled by lossy UTF-8"
        );
        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn copy_duplicates_metadata_and_refreshes_identity_tags() {
        // Regression: copy over the meta protocol only created a second name.
        let node = LocalNode::open(true).await.unwrap();
        let hash = put(&node, "a.txt", b"payload").await;
        set_tag(&node, "a.txt", "priority", Some(b"high")).await;

        let resp = node
            .request(MetaRequest::Copy {
                from: "a.txt".to_owned(),
                to: "b.txt".to_owned(),
            })
            .await;
        assert!(
            matches!(resp, MetaResponse::Copy { success: true }),
            "{resp:?}"
        );

        let resp = node
            .request(MetaRequest::Get {
                filename: "b.txt".to_owned(),
            })
            .await;
        assert!(
            matches!(resp, MetaResponse::Get { hash: Some(h) } if h == hash),
            "{resp:?}"
        );
        let tags = tags_of(&node, "b.txt").await;
        assert!(
            has_tag(&tags, "priority", Some("high")),
            "metadata copied: {tags:?}"
        );
        assert!(
            has_tag(&tags, "name", Some("b.txt")),
            "name follows the copy: {tags:?}"
        );
        assert!(has_tag(&tags, "file", Some("b.txt")));
        // The source is untouched.
        let src = tags_of(&node, "a.txt").await;
        assert!(has_tag(&src, "name", Some("a.txt")));
        assert!(has_tag(&src, "priority", Some("high")));
        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn rename_moves_metadata_and_archives_the_original() {
        let node = LocalNode::open(true).await.unwrap();
        let hash = put(&node, "old.txt", b"payload").await;
        set_tag(&node, "old.txt", "priority", Some(b"high")).await;

        let resp = node
            .request(MetaRequest::Rename {
                from: "old.txt".to_owned(),
                to: "new.txt".to_owned(),
            })
            .await;
        assert!(
            matches!(resp, MetaResponse::Rename { success: true }),
            "{resp:?}"
        );

        let resp = node
            .request(MetaRequest::Get {
                filename: "old.txt".to_owned(),
            })
            .await;
        assert!(matches!(resp, MetaResponse::Get { hash: None }), "{resp:?}");
        let resp = node
            .request(MetaRequest::Get {
                filename: "new.txt".to_owned(),
            })
            .await;
        assert!(
            matches!(resp, MetaResponse::Get { hash: Some(h) } if h == hash),
            "{resp:?}"
        );

        assert!(
            tags_of(&node, "old.txt").await.is_empty(),
            "metadata moved away"
        );
        let tags = tags_of(&node, "new.txt").await;
        assert!(has_tag(&tags, "priority", Some("high")));
        assert!(
            has_tag(&tags, "name", Some("new.txt")),
            "identity follows the rename: {tags:?}"
        );
        assert!(tags.iter().any(|t| t.key == b"archive.rename"));
        assert!(
            names(&node)
                .await
                .iter()
                .any(|n| n.starts_with("old.txt.archive."))
        );
        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn rename_and_copy_report_missing_sources() {
        let node = LocalNode::open(true).await.unwrap();
        let resp = node
            .request(MetaRequest::Rename {
                from: "nope".to_owned(),
                to: "x".to_owned(),
            })
            .await;
        assert!(
            matches!(resp, MetaResponse::Rename { success: false }),
            "{resp:?}"
        );
        let resp = node
            .request(MetaRequest::Copy {
                from: "nope".to_owned(),
                to: "x".to_owned(),
            })
            .await;
        assert!(
            matches!(resp, MetaResponse::Copy { success: false }),
            "{resp:?}"
        );
        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn delete_removes_name_metadata_and_archives() {
        // Regression: delete left metadata behind, so a re-created name
        // inherited the old tags.
        let node = LocalNode::open(true).await.unwrap();
        put(&node, "doomed.txt", b"x").await;
        set_tag(&node, "doomed.txt", "priority", Some(b"high")).await;
        let _ = node
            .request(MetaRequest::Rename {
                from: "doomed.txt".to_owned(),
                to: "doomed2.txt".to_owned(),
            })
            .await;
        // doomed.txt.archive.<ts> now exists; delete that file's remains too.
        let resp = node
            .request(MetaRequest::Delete {
                filename: "doomed2.txt".to_owned(),
            })
            .await;
        assert!(
            matches!(resp, MetaResponse::Delete { success: true }),
            "{resp:?}"
        );
        assert!(tags_of(&node, "doomed2.txt").await.is_empty());
        let resp = node
            .request(MetaRequest::Delete {
                filename: "doomed2.txt".to_owned(),
            })
            .await;
        assert!(
            matches!(resp, MetaResponse::Delete { success: false }),
            "{resp:?}"
        );

        // Re-creating the name starts clean.
        put(&node, "doomed2.txt", b"y").await;
        let tags = tags_of(&node, "doomed2.txt").await;
        assert!(!has_tag(&tags, "priority", Some("high")), "{tags:?}");
        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn delete_only_removes_numeric_archive_suffixes() {
        let node = LocalNode::open(true).await.unwrap();
        put(&node, "a", b"1").await;
        put(&node, "a.archive.notes", b"2").await; // a user file that merely shares the prefix
        let resp = node
            .request(MetaRequest::Delete {
                filename: "a".to_owned(),
            })
            .await;
        assert!(
            matches!(resp, MetaResponse::Delete { success: true }),
            "{resp:?}"
        );
        assert!(names(&node).await.contains(&"a.archive.notes".to_owned()));
        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn writes_are_denied_to_unlisted_nodes_but_reads_are_open() {
        let node = LocalNode::open(true).await.unwrap();
        let hash = put(&node, "pub.txt", b"public").await;
        let owner = eid(1);
        let stranger = eid(2);
        let proto = MetaProtocol::new(
            &node.blobs(),
            None,
            Arc::clone(node.tags()),
            AccessPolicy::restricted([owner]),
            eid(9),
        );

        // Reads are public.
        let resp = proto
            .handle(
                &stranger,
                MetaRequest::Get {
                    filename: "pub.txt".to_owned(),
                },
            )
            .await;
        assert!(
            matches!(resp, MetaResponse::Get { hash: Some(h) } if h == hash),
            "{resp:?}"
        );
        assert!(matches!(
            proto.handle(&stranger, MetaRequest::List).await,
            MetaResponse::List { .. }
        ));

        // Every kind of write is refused with an actionable message.
        for req in [
            MetaRequest::Put {
                filename: "x".to_owned(),
                hash,
            },
            MetaRequest::Delete {
                filename: "pub.txt".to_owned(),
            },
            MetaRequest::Rename {
                from: "pub.txt".to_owned(),
                to: "y".to_owned(),
            },
            MetaRequest::Copy {
                from: "pub.txt".to_owned(),
                to: "y".to_owned(),
            },
            MetaRequest::SetTag {
                subject: b"pub.txt".to_vec(),
                key: b"k".to_vec(),
                value: None,
            },
            MetaRequest::DelTag {
                subject: b"pub.txt".to_vec(),
                key: b"k".to_vec(),
                value: None,
            },
            MetaRequest::MigrateTags,
        ] {
            let resp = proto.handle(&stranger, req).await;
            let msg = resp.error_message().expect("write must be refused");
            assert!(msg.contains("permission denied"), "{msg}");
            assert!(
                msg.contains(&stranger.to_string()),
                "message names the node: {msg}"
            );
        }
        // Nothing changed.
        assert_eq!(names(&node).await, vec!["pub.txt".to_owned()]);

        // The listed node may write.
        let resp = proto
            .handle(
                &owner,
                MetaRequest::Copy {
                    from: "pub.txt".to_owned(),
                    to: "copy.txt".to_owned(),
                },
            )
            .await;
        assert!(
            matches!(resp, MetaResponse::Copy { success: true }),
            "{resp:?}"
        );
        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn whoami_reports_identity_and_write_permission() {
        let node = LocalNode::open(true).await.unwrap();
        let proto = MetaProtocol::new(
            &node.blobs(),
            None,
            Arc::clone(node.tags()),
            AccessPolicy::restricted([eid(1)]),
            eid(9),
        );
        match proto.handle(&eid(1), MetaRequest::Whoami).await {
            MetaResponse::Whoami {
                node_id,
                can_write,
                open_writes,
                ..
            } => {
                assert_eq!(node_id, eid(9));
                assert!(can_write);
                assert!(!open_writes);
            }
            other => panic!("{other:?}"),
        }
        match proto.handle(&eid(2), MetaRequest::Whoami).await {
            MetaResponse::Whoami { can_write, .. } => assert!(!can_write),
            other => panic!("{other:?}"),
        }
        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn find_ranks_name_matches_before_contains() {
        let node = LocalNode::open(true).await.unwrap();
        put(&node, "readme", b"1").await;
        put(&node, "readme.md", b"2").await;
        put(&node, "the-readme", b"3").await;
        let resp = node
            .request(MetaRequest::Find {
                query: "README".to_owned(),
                prefer_name: true,
            })
            .await;
        let MetaResponse::Find { matches } = resp else {
            panic!("{resp:?}")
        };
        let order: Vec<&str> = matches.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(order[0], "readme", "exact first: {order:?}");
        assert_eq!(order[1], "readme.md", "prefix second: {order:?}");
        assert_eq!(order[2], "the-readme", "contains last: {order:?}");
        node.shutdown().await.unwrap();
    }
}
