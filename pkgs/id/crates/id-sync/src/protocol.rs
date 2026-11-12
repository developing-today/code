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
//! The meta protocol uses the ALPN identifier defined in [`META_ALPN`]:
//! `b"/iroh-meta/3"`. This allows nodes to negotiate the correct protocol handler.
//!
//! # Usage Example
//!
//! ```rust,ignore
//! use id_sync::protocol::{MetaRequest, MetaResponse};
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

use std::{sync::Arc, time::Duration};

use futures_lite::StreamExt;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh_base::EndpointId;
use iroh_blobs::{Hash, api::Store};
use serde::{Deserialize, Serialize};
use tokio::{sync::Semaphore, task::JoinSet};

use crate::fileops::{FileOpError, FileOps};
use id_core::access::AccessPolicy;
use id_core::discovery::{PeerAnnouncement, PeerDiscovery};

/// Archive references are reserved for FileOps-generated metadata. Protocol
/// peers may neither create nor change them through the general tag API.
fn is_reserved_archive_reference_key(key: &[u8]) -> bool {
    matches!(key, b"archive.rename" | b"archive.replace")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod archive_reference_behavior_tests {
    use std::sync::Arc;

    use id_core::access::AccessPolicy;
    use id_store::tags::TagStore;
    use iroh::endpoint::{Endpoint, RelayMode, presets};
    use iroh_base::SecretKey;
    use iroh_blobs::{Hash, api::Store, store::mem::MemStore};
    use iroh_docs::protocol::Docs;
    use iroh_gossip::net::Gossip;

    use super::{MetaProtocol, MetaRequest, MetaResponse};

    #[tokio::test]
    async fn authorized_peer_cannot_forge_refs_and_delete_preserves_preexisting_foreign_archive() {
        let endpoint = Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap();
        let store: Store = MemStore::new().into();
        let gossip = Gossip::builder().spawn(endpoint.clone());
        let docs = Docs::memory()
            .spawn(endpoint.clone(), store.clone(), gossip)
            .await
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let tags = Arc::new(
            TagStore::init_in(&docs, "archive-reference-test", dir.path().join("meta"))
                .await
                .unwrap(),
        );
        let peer = SecretKey::from_bytes(&[42; 32]).public();
        let server = MetaProtocol::new(
            &store,
            None,
            Arc::clone(&tags),
            AccessPolicy::restricted([peer]),
            endpoint.id(),
            None,
        );
        let foreign_hash = Hash::from([7; 32]);
        let owned_hash = Hash::from([8; 32]);
        store
            .tags()
            .set("my-file", Hash::from([1; 32]))
            .await
            .unwrap();
        store
            .tags()
            .set("other-file.archive.123", foreign_hash)
            .await
            .unwrap();
        store
            .tags()
            .set("my-file.archive.124", owned_hash)
            .await
            .unwrap();
        tags.set_tag(
            &tags.global,
            b"other-file.archive.123",
            b"keep",
            Some(b"foreign metadata"),
            b"",
        )
        .await
        .unwrap();
        tags.set_tag(
            &tags.global,
            b"my-file.archive.124",
            b"keep",
            Some(b"owned metadata"),
            b"",
        )
        .await
        .unwrap();

        let rejected_set = server
            .handle(
                &peer,
                MetaRequest::SetTag {
                    subject: b"my-file".to_vec(),
                    key: b"archive.replace".to_vec(),
                    value: Some(b"other-file.archive.123".to_vec()),
                },
            )
            .await;
        assert!(matches!(rejected_set, MetaResponse::Error { .. }));
        let rejected_delete = server
            .handle(
                &peer,
                MetaRequest::DelTag {
                    subject: b"my-file".to_vec(),
                    key: b"archive.replace".to_vec(),
                    value: None,
                },
            )
            .await;
        assert!(matches!(rejected_delete, MetaResponse::Error { .. }));

        // Model legacy/persisted forged metadata written before the protocol
        // reserved these keys; deletion must still refuse to follow it.
        tags.set_tag(
            &tags.global,
            b"my-file",
            b"archive.replace",
            Some(b"other-file.archive.123"),
            b"",
        )
        .await
        .unwrap();
        let response = server
            .handle(
                &peer,
                MetaRequest::Delete {
                    filename: "my-file".to_owned(),
                },
            )
            .await;
        assert!(matches!(response, MetaResponse::Delete { success: true }));

        assert_eq!(
            store
                .tags()
                .get("other-file.archive.123")
                .await
                .unwrap()
                .unwrap()
                .hash,
            foreign_hash
        );
        let foreign_metadata = tags
            .get_tags(&tags.global, b"other-file.archive.123")
            .await
            .unwrap();
        assert!(
            foreign_metadata
                .iter()
                .any(|tag| tag.key.as_bytes() == b"keep")
        );
        assert!(
            store
                .tags()
                .get("my-file.archive.124")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            tags.get_tags(&tags.global, b"my-file.archive.124")
                .await
                .unwrap()
                .is_empty()
        );
        endpoint.close().await;
    }
}

#[cfg(test)]
mod reserved_archive_reference_tests {
    use super::is_reserved_archive_reference_key;

    #[test]
    fn archive_reference_keys_are_reserved_for_fileops() {
        assert!(is_reserved_archive_reference_key(b"archive.rename"));
        assert!(is_reserved_archive_reference_key(b"archive.replace"));
        assert!(!is_reserved_archive_reference_key(b"user.label"));
    }
}
use id_store::tags::TagStore;

/// Decode exactly one meta-protocol value, rejecting any trailing bytes.
///
/// Meta requests and responses are one-message-per-stream. Accepting a valid
/// postcard prefix would let a padded or concatenated stream be interpreted as
/// a different, complete message.
pub fn decode_meta_exact<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> anyhow::Result<T> {
    let (value, rest) = postcard::take_from_bytes(bytes)?;
    anyhow::ensure!(
        rest.is_empty(),
        "trailing bytes after meta-protocol value ({} bytes)",
        rest.len()
    );
    Ok(value)
}

/// Maximum time for one network phase (request read or response write) of a
/// meta stream. The phases are bounded separately on purpose: once a request
/// has been fully read, its handler runs to completion, so the deadline can
/// never cancel a non-transactional mutation (e.g. a rename) midway through.
const META_STREAM_DEADLINE: Duration = Duration::from_secs(60);

/// Concurrent meta requests allowed on one connection.
pub const MAX_META_STREAMS_PER_CONNECTION: usize = 16;

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
/// use id_sync::protocol::MatchKind;
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

/// Determines how a search query matches a string.
///
/// This function implements a priority-based matching system:
/// 1. **Exact**: The strings are identical
/// 2. **Prefix**: The haystack starts with the needle
/// 3. **Contains**: The haystack contains the needle somewhere
///
/// # Arguments
///
/// * `haystack` - The string to search within (e.g., filename or hash)
/// * `needle` - The search query
///
/// # Returns
///
/// * `Some(MatchKind::Exact)` if `haystack == needle`
/// * `Some(MatchKind::Prefix)` if `haystack.starts_with(needle)`
/// * `Some(MatchKind::Contains)` if `haystack.contains(needle)`
/// * `None` if no match
///
/// # Example
///
/// ```rust
/// use id_sync::protocol::{match_kind, MatchKind};
///
/// assert_eq!(match_kind("hello.txt", "hello.txt"), Some(MatchKind::Exact));
/// assert_eq!(match_kind("hello.txt", "hello"), Some(MatchKind::Prefix));
/// assert_eq!(match_kind("say-hello.txt", "hello"), Some(MatchKind::Contains));
/// assert_eq!(match_kind("world.txt", "hello"), None);
/// ```
pub fn match_kind(haystack: &str, needle: &str) -> Option<MatchKind> {
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

/// A single match result from find/search operations.
///
/// Represents one blob that matched a search query, including metadata
/// about how well it matched and whether the match was against the
/// blob's name or its hash.
///
/// # Example
///
/// ```rust
/// use id_sync::protocol::{FindMatch, MatchKind};
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
/// use id_sync::protocol::{TaggedMatch, MatchKind};
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
/// use id_sync::protocol::MetaRequest;
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

impl From<id_store::tags::Tag> for WireTag {
    fn from(t: id_store::tags::Tag) -> Self {
        Self {
            subject: t.subject.into_bytes(),
            key: t.key.into_bytes(),
            value: t.value.map(id_store::tags::TagValue::into_bytes),
        }
    }
}

/// Maximum size of a single request on the meta protocol (1 MiB).
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;

/// Application-Level Protocol Negotiation (ALPN) identifier for the meta protocol.
///
/// Used during QUIC handshake to identify connections for metadata operations
/// (list, find, delete, rename, etc.) as opposed to blob data transfer.
///
/// **Version 2** carries tag keys/values as raw bytes (binary-safe), defines
/// `DelTag` with no value as "delete every value of the key", and adds `Whoami`
/// and `Error`. It is not wire-compatible with `/iroh-meta/1`; the ALPN bump
/// makes old and new nodes fail the handshake cleanly instead of mis-decoding.
///
/// **Version 3** changes the `List` response item from `(hash, name)` pairs
/// to `ListEntry` records carrying the file's origin node and upload time
/// (#4973). That payload change is not wire-compatible with `/iroh-meta/2`
/// under postcard, so it takes this ALPN bump per the rule below.
///
/// Compatibility rule: within a version, only **append** enum variants. Any
/// other change to [`protocol::MetaRequest`] / [`protocol::MetaResponse`]
/// requires a new ALPN. `protocol::tests` pins every discriminant.
pub const META_ALPN: &[u8] = b"/iroh-meta/3";

/// How long a server waits for a just-pushed blob to finish arriving before a
/// `Put` naming it is refused.
pub const DEFAULT_BLOB_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Requests to a remote node via the meta protocol (`/iroh-meta/3`).
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
    /// Read the node's per-(local node, remote endpoint) frame/byte abuse
    /// counters, split by direction, plus the policy rates that produced them.
    ///
    /// Read-only. The counters cover world-protocol traffic admitted
    /// by the node's abuse evaluator; a node with no evaluator attached
    /// (a plain serve without a world, or a build without the world
    /// feature) records none and answers with an empty report.
    AbuseCounters,
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
/// One entry of a [`MetaResponse::List`] response (#4973).
///
/// `origin` is the node that first wrote the file, recorded by the
/// `origin` metadata tag at write time; `uploaded_at` is the file's
/// upload time (unix seconds) from its `created` tag. Files written
/// before those tags existed carry `None` for both and list with
/// origin unknown — the values are never guessed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListEntry {
    /// The content hash.
    pub hash: Hash,
    /// The tag name.
    pub name: String,
    /// Node id of the file's origin node, if recorded.
    pub origin: Option<String>,
    /// Upload time (unix seconds), if recorded.
    pub uploaded_at: Option<u64>,
}

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
        /// All names with their origin node and upload time (#4973).
        items: Vec<ListEntry>,
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
    /// Response to [`MetaRequest::AbuseCounters`].
    AbuseCounters {
        /// One row per (local node, remote endpoint), already sorted by
        /// local node then remote endpoint.
        rows: Vec<AbuseCountersRow>,
        /// The policy rates the counters are charged against, or `None`
        /// when the node exposes counters without a policy view.
        policy: Option<AbusePolicyView>,
    },
}

/// One row of [`MetaResponse::AbuseCounters`]: the frame and byte counters
/// admitted for one (local node, remote endpoint) pair, split by direction.
///
/// `unattributed` holds traffic admitted through a check that does not know
/// its direction; it is never guessed into `inbound` or `outbound`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbuseCountersRow {
    /// The local node's ID (32-byte verifying key).
    pub local: [u8; 32],
    /// The remote endpoint's ID (32-byte verifying key).
    pub remote: [u8; 32],
    /// Frames admitted inbound.
    pub inbound_frames: u64,
    /// Bytes admitted inbound.
    pub inbound_bytes: u64,
    /// Frames admitted outbound.
    pub outbound_frames: u64,
    /// Bytes admitted outbound.
    pub outbound_bytes: u64,
    /// Frames admitted without a known direction.
    pub unattributed_frames: u64,
    /// Bytes admitted without a known direction.
    pub unattributed_bytes: u64,
}

/// The authorized token-bucket policy rates shown alongside
/// [`AbuseCountersRow`]s.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbusePolicyView {
    /// Authorized frames admitted per second.
    pub frames_per_sec: u64,
    /// Authorized frame burst.
    pub frame_burst: u64,
    /// Authorized bytes admitted per second.
    pub bytes_per_sec: u64,
    /// Authorized byte burst.
    pub byte_burst: u64,
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
/// with [`META_ALPN`], each bidirectional stream carries one
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
    /// The node's abuse evaluator, when this surface exposes it. `None`
    /// answers [`MetaRequest::AbuseCounters`] with an empty report.
    pub abuse: Option<Arc<std::sync::Mutex<id_core::abuse::AbuseEvaluator>>>,
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
        abuse: Option<Arc<std::sync::Mutex<id_core::abuse::AbuseEvaluator>>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store: store.clone(),
            peer_discovery,
            tag_store,
            access,
            node_id,
            blob_wait: DEFAULT_BLOB_WAIT,
            abuse,
        })
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
        let ops = FileOps::new(&self.store, &self.tag_store)
            .with_blob_wait(self.blob_wait)
            .with_origin(remote.to_string());
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
                        items.push(ListEntry {
                            hash: item.hash,
                            name,
                            origin: None,
                            uploaded_at: None,
                        });
                    }
                }
                // Enrich with origin + upload time from the metadata tags
                // (#4973): `origin` is stamped at write time by FileOps,
                // `created` is the upload time. Files written before the
                // tags existed keep `None` and list as origin unknown.
                if let Ok(tags) = self.tag_store.list_all(ns).await {
                    let mut origins: std::collections::HashMap<String, String> =
                        std::collections::HashMap::new();
                    let mut created: std::collections::HashMap<String, u64> =
                        std::collections::HashMap::new();
                    for tag in &tags {
                        if tag.key == "origin" {
                            if let Some(value) = tag.value.as_ref().and_then(|v| v.as_str()) {
                                origins.insert(tag.subject.display_lossy(), value.to_owned());
                            }
                        } else if tag.key == "created"
                            && let Some(ts) = tag
                                .value
                                .as_ref()
                                .and_then(|v| v.as_str())
                                .and_then(|v| v.parse::<u64>().ok())
                        {
                            created.insert(tag.subject.display_lossy(), ts);
                        }
                    }
                    for item in &mut items {
                        item.origin = origins.get(&item.name).cloned();
                        item.uploaded_at = created.get(&item.name).copied();
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
            MetaRequest::SetTag { key, .. } if is_reserved_archive_reference_key(&key) => {
                // These keys are FileOps-owned references, not user metadata.
                // Allowing protocol peers to write them lets a later delete
                // remove an archive (and metadata) belonging to another file.
                MetaResponse::Error {
                    message: "archive reference tags are reserved".to_owned(),
                }
            }
            MetaRequest::DelTag { key, .. } if is_reserved_archive_reference_key(&key) => {
                MetaResponse::Error {
                    message: "archive reference tags are reserved".to_owned(),
                }
            }
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
            MetaRequest::AbuseCounters => {
                // No evaluator attached means no world traffic is recorded:
                // answer the empty report, not an error, so
                // `id peers --abuse` against a plain serve succeeds.
                let Some(abuse) = &self.abuse else {
                    return MetaResponse::AbuseCounters {
                        rows: Vec::new(),
                        policy: None,
                    };
                };
                // Copy the counters and the policy out under the lock, then
                // release it before sorting and building rows: that work
                // must not block world frame charges on the evaluator.
                let (mut snapshot, authorized) = {
                    let Ok(guard) = abuse.lock() else {
                        return MetaResponse::Error {
                            message:
                                "abuse counters are unavailable: the evaluator lock is poisoned"
                                    .to_owned(),
                        };
                    };
                    (
                        guard.counters().copied_directional(),
                        guard.policy().authorized,
                    )
                };
                snapshot.sort_by(|a, b| a.0.cmp(&b.0));
                let rows = snapshot
                    .into_iter()
                    .map(|((local, remote), counters)| AbuseCountersRow {
                        local,
                        remote,
                        inbound_frames: counters.inbound.frames,
                        inbound_bytes: counters.inbound.bytes,
                        outbound_frames: counters.outbound.frames,
                        outbound_bytes: counters.outbound.bytes,
                        unattributed_frames: counters.unattributed.frames,
                        unattributed_bytes: counters.unattributed.bytes,
                    })
                    .collect();
                MetaResponse::AbuseCounters {
                    rows,
                    policy: Some(AbusePolicyView {
                        frames_per_sec: authorized.frames_per_sec,
                        frame_burst: authorized.frame_burst,
                        bytes_per_sec: authorized.bytes_per_sec,
                        byte_burst: authorized.byte_burst,
                    }),
                }
            }
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
                if let Some(kind) = match_kind(&name.to_lowercase(), &query_lower) {
                    matches.push(FindMatch {
                        hash: item.hash,
                        name,
                        kind,
                        is_hash_match: false,
                    });
                } else if let Some(kind) = match_kind(&hash_str, &query_lower) {
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

    pub async fn accept_connection(
        &self,
        conn: Connection,
        stream_deadline: Duration,
    ) -> Result<(), AcceptError> {
        let remote = conn.remote_id();
        let permits = Arc::new(Semaphore::new(MAX_META_STREAMS_PER_CONNECTION));
        let mut streams = JoinSet::new();

        loop {
            tokio::select! {
                incoming = conn.accept_bi() => {
                    let Ok((mut send, recv)) = incoming else {
                        break;
                    };
                    let Ok(permit) = Arc::clone(&permits).try_acquire_owned() else {
                        // Refuse only this stream; the connection remains usable.
                        let _ = send.reset(1_u32.into());
                        continue;
                    };
                    let protocol = self.clone();
                    streams.spawn(async move {
                        let _permit = permit;
                        protocol
                            .handle_stream(remote, send, recv, stream_deadline)
                            .await;
                    });
                }
                Some(result) = streams.join_next(), if !streams.is_empty() => {
                    if let Err(error) = result {
                        tracing::warn!(%error, "meta stream handler task failed");
                    }
                }
            }
        }

        // The peer closed the connection. Drain admitted handlers instead
        // of aborting them: shutdown() would cancel a handler at its next
        // suspension point, and a mutating handler cancelled mid-flight
        // (e.g. FileOps::rename, which deletes the source before awaiting
        // the metadata transfer) can leave partial state behind. Each
        // handler's network phases stay bounded by the stream deadline,
        // so this drain terminates.
        while let Some(result) = streams.join_next().await {
            if let Err(error) = result {
                tracing::warn!(%error, "meta stream handler task failed");
            }
        }
        Ok(())
    }

    async fn handle_stream(
        &self,
        remote: EndpointId,
        mut send: iroh::endpoint::SendStream,
        mut recv: iroh::endpoint::RecvStream,
        stream_deadline: Duration,
    ) {
        // Phase 1: read the request under the deadline. A stalled or
        // trickling peer is cut off here, without affecting other streams.
        let buf = match tokio::time::timeout(stream_deadline, recv.read_to_end(MAX_REQUEST_BYTES))
            .await
        {
            Ok(Ok(buf)) => buf,
            Ok(Err(error)) => {
                tracing::debug!(%remote, %error, "meta stream read failed");
                return;
            }
            Err(_) => {
                tracing::debug!(%remote, ?stream_deadline, "meta stream read deadline exceeded");
                let _ = send.reset(1_u32.into());
                return;
            }
        };

        // Phase 2: handle the admitted request to completion. This is
        // deliberately NOT wrapped in the deadline: handlers perform
        // non-transactional mutations (e.g. FileOps::rename moves a blob and
        // then transfers metadata), and cancelling one midway would corrupt
        // state. The handler does no peer-controlled waiting — the network
        // phases on either side of it carry the deadline.
        let resp = match decode_meta_exact::<MetaRequest>(&buf) {
            Ok(req) => self.handle(&remote, req).await,
            Err(error) => MetaResponse::Error {
                message: format!("malformed request: {error}"),
            },
        };

        // Phase 3: write the response under the deadline; a peer that stops
        // reading is cut off and its stream reset.
        let written = tokio::time::timeout(stream_deadline, async {
            let bytes = postcard::to_allocvec(&resp).map_err(AcceptError::from_err)?;
            send.write_all(&bytes)
                .await
                .map_err(AcceptError::from_err)?;
            send.finish()?;
            Ok::<(), AcceptError>(())
        })
        .await;

        match written {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::debug!(%remote, %error, "meta stream failed"),
            Err(_) => {
                tracing::debug!(%remote, ?stream_deadline, "meta stream write deadline exceeded");
                let _ = send.reset(1_u32.into());
            }
        }
    }
}

impl ProtocolHandler for MetaProtocol {
    /// Serves requests on a connection until the peer closes it.
    /// Individual stream failures are isolated; active streams are stopped when
    /// the peer closes the connection.
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        self.accept_connection(conn, META_STREAM_DEADLINE).await
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use iroh::{
        EndpointAddr,
        endpoint::{Endpoint, RelayMode, presets},
        protocol::Router,
    };
    use iroh_base::{SecretKey, TransportAddr};
    use std::net::{Ipv4Addr, SocketAddr};

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
    fn meta_decode_requires_full_consumption() {
        let list = postcard::to_allocvec(&MetaRequest::List).unwrap();
        assert!(matches!(
            decode_meta_exact::<MetaRequest>(&list),
            Ok(MetaRequest::List)
        ));

        let mut padded_list = list.clone();
        padded_list.push(0);
        assert!(decode_meta_exact::<MetaRequest>(&padded_list).is_err());

        let mut garbage_list = list;
        garbage_list.extend_from_slice(b"GARBAGE");
        assert!(decode_meta_exact::<MetaRequest>(&garbage_list).is_err());

        let delete = MetaRequest::Delete {
            filename: "victim.txt".to_owned(),
        };
        let mut padded_delete = postcard::to_allocvec(&delete).unwrap();
        padded_delete.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        assert!(decode_meta_exact::<MetaRequest>(&padded_delete).is_err());

        let mut double_request = postcard::to_allocvec(&delete).unwrap();
        double_request.extend_from_slice(&postcard::to_allocvec(&MetaRequest::List).unwrap());
        assert!(decode_meta_exact::<MetaRequest>(&double_request).is_err());

        let response = MetaResponse::Delete { success: true };
        let mut padded_response = postcard::to_allocvec(&response).unwrap();
        padded_response.extend_from_slice(b"EXTRA");
        assert!(decode_meta_exact::<MetaResponse>(&padded_response).is_err());

        assert!(matches!(
            decode_meta_exact::<MetaResponse>(&postcard::to_allocvec(&response).unwrap()),
            Ok(MetaResponse::Delete { success: true })
        ));
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
                ListEntry {
                    hash: hash1,
                    name: "file1.txt".to_owned(),
                    origin: Some("node-a".to_owned()),
                    uploaded_at: Some(1_700_000_000),
                },
                ListEntry {
                    hash: hash2,
                    name: "file2.txt".to_owned(),
                    origin: None,
                    uploaded_at: None,
                },
            ],
        };
        let bytes = postcard::to_allocvec(&resp).unwrap();
        let decoded: MetaResponse = postcard::from_bytes(&bytes).unwrap();
        match decoded {
            MetaResponse::List { items } => {
                assert_eq!(items.len(), 2);
                assert_eq!(items[0].name, "file1.txt");
                assert_eq!(items[0].origin.as_deref(), Some("node-a"));
                assert_eq!(items[0].uploaded_at, Some(1_700_000_000));
                assert_eq!(items[1].name, "file2.txt");
                assert_eq!(items[1].origin, None);
                assert_eq!(items[1].uploaded_at, None);
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

    #[test]
    fn test_meta_request_abuse_counters_serialization() {
        let req = MetaRequest::AbuseCounters;
        let bytes = postcard::to_allocvec(&req).unwrap();
        let decoded: MetaRequest = postcard::from_bytes(&bytes).unwrap();
        assert!(matches!(decoded, MetaRequest::AbuseCounters));
    }

    #[test]
    fn test_meta_response_abuse_counters_serialization() {
        let resp = MetaResponse::AbuseCounters {
            rows: vec![AbuseCountersRow {
                local: [1u8; 32],
                remote: [2u8; 32],
                inbound_frames: 3,
                inbound_bytes: 300,
                outbound_frames: 4,
                outbound_bytes: 400,
                unattributed_frames: 5,
                unattributed_bytes: 500,
            }],
            policy: Some(AbusePolicyView {
                frames_per_sec: 100,
                frame_burst: 180,
                bytes_per_sec: 1_048_576,
                byte_burst: 2_097_152,
            }),
        };
        let bytes = postcard::to_allocvec(&resp).unwrap();
        let decoded: MetaResponse = postcard::from_bytes(&bytes).unwrap();
        match decoded {
            MetaResponse::AbuseCounters { rows, policy } => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].local, [1u8; 32]);
                assert_eq!(rows[0].remote, [2u8; 32]);
                assert_eq!(rows[0].inbound_frames, 3);
                assert_eq!(rows[0].inbound_bytes, 300);
                assert_eq!(rows[0].outbound_frames, 4);
                assert_eq!(rows[0].outbound_bytes, 400);
                assert_eq!(rows[0].unattributed_frames, 5);
                assert_eq!(rows[0].unattributed_bytes, 500);
                let view = policy.expect("policy");
                assert_eq!(view.frames_per_sec, 100);
                assert_eq!(view.frame_burst, 180);
                assert_eq!(view.bytes_per_sec, 1_048_576);
                assert_eq!(view.byte_burst, 2_097_152);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_meta_response_abuse_counters_empty_serialization() {
        let resp = MetaResponse::AbuseCounters {
            rows: vec![],
            policy: None,
        };
        let bytes = postcard::to_allocvec(&resp).unwrap();
        let decoded: MetaResponse = postcard::from_bytes(&bytes).unwrap();
        match decoded {
            MetaResponse::AbuseCounters { rows, policy } => {
                assert!(rows.is_empty());
                assert!(policy.is_none());
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
        // indices are the wire format of `/iroh-meta/3`: within a version only
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
            (MetaRequest::AbuseCounters, 14),
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
            (
                MetaResponse::AbuseCounters {
                    rows: vec![],
                    policy: None,
                },
                15,
            ),
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
            MetaRequest::AbuseCounters,
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
}
