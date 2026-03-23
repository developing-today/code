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
//! `b"/id/meta/1"`. This allows nodes to negotiate the correct protocol handler.
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
use iroh_blobs::{Hash, api::Store};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::discovery::{PeerAnnouncement, PeerDiscovery};

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
#[derive(Debug, Serialize, Deserialize)]
pub enum MetaRequest {
    /// Create or update a tag on the remote node.
    ///
    /// Associates `filename` with `hash` in the remote store. The blob
    /// content must already exist on the remote (transferred via Iroh's
    /// blob protocol).
    Put {
        /// The tag name to create or update.
        filename: String,
        /// The content hash to associate with this tag.
        hash: Hash,
    },
    /// Look up a tag by name on the remote node.
    ///
    /// Returns the hash associated with `filename`, if it exists.
    Get {
        /// The tag name to look up.
        filename: String,
    },
    /// List all tags on the remote node.
    ///
    /// Returns a list of (hash, name) pairs for all stored tags.
    List,
    /// Delete a tag from the remote node.
    ///
    /// Removes the tag but does not delete the underlying blob content.
    Delete {
        /// The tag name to delete.
        filename: String,
    },
    /// Rename a tag on the remote node.
    ///
    /// Atomically moves the tag from `from` to `to`. The old tag is
    /// deleted after the new one is created.
    Rename {
        /// The current tag name.
        from: String,
        /// The new tag name.
        to: String,
    },
    /// Copy a tag on the remote node.
    ///
    /// Creates a new tag `to` pointing to the same hash as `from`.
    Copy {
        /// The source tag name.
        from: String,
        /// The destination tag name.
        to: String,
    },
    /// Search for tags matching a query on the remote node.
    ///
    /// Searches both tag names and hashes, returning matches ranked
    /// by quality (exact > prefix > contains).
    Find {
        /// The search query (matched case-insensitively).
        query: String,
        /// If `true`, prioritize name matches over hash matches in results.
        prefer_name: bool,
    },
    /// Request the list of known peers from a remote node.
    ///
    /// Returns all peers that the remote node has discovered via gossip.
    /// If the remote node is not running peer discovery, it returns an
    /// empty list.
    ///
    /// **Wire compatibility note:** This variant MUST remain the last
    /// variant in the enum. Postcard uses positional discriminants, so
    /// inserting a new variant before this one would break wire compatibility
    /// with older nodes.
    ListPeers,
}

/// Responses from a remote node via the meta protocol.
///
/// Each variant corresponds to a [`MetaRequest`] variant and contains
/// the result of that operation.
#[derive(Debug, Serialize, Deserialize)]
pub enum MetaResponse {
    /// Response to [`MetaRequest::Put`].
    Put {
        /// Whether the tag was successfully created/updated.
        success: bool,
    },
    /// Response to [`MetaRequest::Get`].
    Get {
        /// The hash if found, or `None` if the tag doesn't exist.
        hash: Option<Hash>,
    },
    /// Response to [`MetaRequest::List`].
    List {
        /// All tags as (hash, name) pairs.
        items: Vec<(Hash, String)>,
    },
    /// Response to [`MetaRequest::Delete`].
    Delete {
        /// Whether the tag was successfully deleted.
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
        /// Matching tags, sorted by match quality.
        matches: Vec<FindMatch>,
    },
    /// Response to [`MetaRequest::ListPeers`].
    ///
    /// Contains the list of peers currently known to the node via gossip.
    /// May be empty if the node is not running peer discovery or has
    /// not yet discovered any peers.
    ///
    /// **Wire compatibility note:** This variant MUST remain the last
    /// variant in the enum. See [`MetaRequest::ListPeers`] for details.
    ListPeers {
        /// Known peers as announcements.
        peers: Vec<PeerAnnouncement>,
    },
}

/// Protocol handler for the meta protocol.
///
/// Implements Iroh's [`ProtocolHandler`] trait to handle incoming
/// meta protocol connections. When a remote node connects with the
/// `META_ALPN` protocol identifier, this handler processes the requests.
///
/// # Connection Handling
///
/// Each connection can contain multiple request/response pairs. The
/// handler reads requests in a loop until the connection is closed:
///
/// ```text
/// Connection opened
///     ↓
/// Accept bidirectional stream
///     ↓
/// Read request → Process → Send response
///     ↓
/// Loop until connection closed
/// ```
///
/// # Example
///
/// Creating a meta protocol handler for a store:
///
/// ```rust,ignore
/// use id::protocol::MetaProtocol;
/// use iroh_blobs::api::Store;
///
/// let store: Store = /* ... */;
/// let handler = MetaProtocol::new(&store, None);
///
/// // Register with router using META_ALPN
/// router.accept(META_ALPN, handler);
/// ```
#[derive(Clone, Debug)]
pub struct MetaProtocol {
    /// The blob store used for tag operations.
    pub store: Store,
    /// Optional peer discovery table for the `ListPeers` RPC.
    ///
    /// When `Some`, the handler returns known peers in response to
    /// `ListPeers` requests. When `None`, an empty list is returned.
    pub peer_discovery: Option<PeerDiscovery>,
}

impl MetaProtocol {
    /// Creates a new meta protocol handler for the given store.
    ///
    /// Returns an `Arc` for easy registration with Iroh's router.
    ///
    /// The `peer_discovery` parameter is optional. When provided, the
    /// handler will respond to `ListPeers` requests with the current
    /// peer table contents. When `None`, `ListPeers` returns an empty list.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use id::protocol::MetaProtocol;
    ///
    /// // Without peer discovery
    /// let handler = MetaProtocol::new(&store, None);
    ///
    /// // With peer discovery
    /// let discovery = PeerDiscovery::new();
    /// let handler = MetaProtocol::new(&store, Some(discovery));
    /// router.accept(META_ALPN, handler);
    /// ```
    pub fn new(store: &Store, peer_discovery: Option<PeerDiscovery>) -> Arc<Self> {
        Arc::new(Self {
            store: store.clone(),
            peer_discovery,
        })
    }

    /// Determines the match quality of a needle in a haystack.
    ///
    /// Returns the best applicable [`MatchKind`], or `None` if no match.
    /// Matching is case-sensitive; callers should lowercase both strings
    /// for case-insensitive matching.
    ///
    /// # Match Priority
    ///
    /// 1. [`MatchKind::Exact`] - strings are equal
    /// 2. [`MatchKind::Prefix`] - haystack starts with needle
    /// 3. [`MatchKind::Contains`] - haystack contains needle
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use id::protocol::{MetaProtocol, MatchKind};
    ///
    /// assert_eq!(MetaProtocol::match_kind("hello", "hello"), Some(MatchKind::Exact));
    /// assert_eq!(MetaProtocol::match_kind("hello world", "hello"), Some(MatchKind::Prefix));
    /// assert_eq!(MetaProtocol::match_kind("say hello", "hello"), Some(MatchKind::Contains));
    /// assert_eq!(MetaProtocol::match_kind("goodbye", "hello"), None);
    /// ```
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
}

impl ProtocolHandler for MetaProtocol {
    /// Handles an incoming meta protocol connection.
    ///
    /// Processes multiple request/response pairs on bidirectional QUIC streams
    /// until the connection is closed. Each request is deserialized, processed
    /// against the local store, and a response is sent back.
    ///
    /// # Request Processing
    ///
    /// - **Put**: Creates or updates a tag pointing to the given hash
    /// - **Get**: Looks up a tag by name and returns its hash
    /// - **List**: Returns all (hash, name) pairs in the store
    /// - **Delete**: Removes a tag from the store
    /// - **Rename**: Moves a tag from one name to another
    /// - **Copy**: Creates a new tag pointing to the same hash
    /// - **Find**: Searches tags by name/hash and returns ranked matches
    /// - **`ListPeers`**: Returns known peers from the gossip-based peer discovery table
    ///
    /// # Errors
    ///
    /// Returns `AcceptError` if:
    /// - Tag operations fail
    /// - Serialization/deserialization fails
    /// - Stream write fails
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        // Handle multiple requests per connection
        loop {
            let Ok((mut send, mut recv)) = conn.accept_bi().await else {
                break; // Connection closed
            };
            let Ok(buf) = recv.read_to_end(64 * 1024).await else {
                break;
            };
            let Ok(req): Result<MetaRequest, _> = postcard::from_bytes(&buf) else {
                break;
            };
            match req {
                MetaRequest::Put { filename, hash } => {
                    self.store
                        .tags()
                        .set(&filename, hash)
                        .await
                        .map_err(AcceptError::from_err)?;
                    let resp = postcard::to_allocvec(&MetaResponse::Put { success: true })
                        .map_err(AcceptError::from_err)?;
                    send.write_all(&resp).await.map_err(AcceptError::from_err)?;
                    send.finish()?;
                }
                MetaRequest::Get { filename } => {
                    let mut found: Option<Hash> = None;
                    if let Ok(Some(tag)) = self.store.tags().get(&filename).await {
                        found = Some(tag.hash);
                    } else if let Ok(mut list) = self.store.tags().list().await {
                        while let Some(item) = list.next().await {
                            let item = item.map_err(AcceptError::from_err)?;
                            if item.name.as_ref() == filename.as_bytes() {
                                found = Some(item.hash);
                                break;
                            }
                        }
                    }
                    let resp = postcard::to_allocvec(&MetaResponse::Get { hash: found })
                        .map_err(AcceptError::from_err)?;
                    send.write_all(&resp).await.map_err(AcceptError::from_err)?;
                    send.finish()?;
                }
                MetaRequest::List => {
                    let mut items = Vec::new();
                    if let Ok(mut list) = self.store.tags().list().await {
                        while let Some(item) = list.next().await {
                            if let Ok(item) = item {
                                let name = String::from_utf8_lossy(item.name.as_ref()).to_string();
                                items.push((item.hash, name));
                            }
                        }
                    }
                    let resp = postcard::to_allocvec(&MetaResponse::List { items })
                        .map_err(AcceptError::from_err)?;
                    send.write_all(&resp).await.map_err(AcceptError::from_err)?;
                    send.finish()?;
                }
                MetaRequest::Delete { filename } => {
                    let success = self.store.tags().delete(&filename).await.is_ok();
                    let resp = postcard::to_allocvec(&MetaResponse::Delete { success })
                        .map_err(AcceptError::from_err)?;
                    send.write_all(&resp).await.map_err(AcceptError::from_err)?;
                    send.finish()?;
                }
                MetaRequest::Rename { from, to } => {
                    let success = if let Ok(Some(tag)) = self.store.tags().get(&from).await {
                        let hash = tag.hash;
                        if self.store.tags().set(&to, hash).await.is_ok() {
                            self.store.tags().delete(&from).await.is_ok()
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    let resp = postcard::to_allocvec(&MetaResponse::Rename { success })
                        .map_err(AcceptError::from_err)?;
                    send.write_all(&resp).await.map_err(AcceptError::from_err)?;
                    send.finish()?;
                }
                MetaRequest::Copy { from, to } => {
                    let success = if let Ok(Some(tag)) = self.store.tags().get(&from).await {
                        self.store.tags().set(&to, tag.hash).await.is_ok()
                    } else {
                        false
                    };
                    let resp = postcard::to_allocvec(&MetaResponse::Copy { success })
                        .map_err(AcceptError::from_err)?;
                    send.write_all(&resp).await.map_err(AcceptError::from_err)?;
                    send.finish()?;
                }
                MetaRequest::Find { query, prefer_name } => {
                    let mut matches = Vec::new();
                    let query_lower = query.to_lowercase();

                    if let Ok(mut list) = self.store.tags().list().await {
                        while let Some(item) = list.next().await {
                            if let Ok(item) = item {
                                let name = String::from_utf8_lossy(item.name.as_ref()).to_string();
                                let hash_str = item.hash.to_string();
                                let name_lower = name.to_lowercase();

                                // Check name matches
                                if let Some(kind) = Self::match_kind(&name_lower, &query_lower) {
                                    matches.push(FindMatch {
                                        hash: item.hash,
                                        name: name.clone(),
                                        kind,
                                        is_hash_match: false,
                                    });
                                }
                                // Check hash matches (only if no name match or query looks like a hash)
                                else if let Some(kind) = Self::match_kind(&hash_str, &query_lower)
                                {
                                    matches.push(FindMatch {
                                        hash: item.hash,
                                        name,
                                        kind,
                                        is_hash_match: true,
                                    });
                                }
                            }
                        }
                    }

                    // Sort: by match kind first, then by preference (hash vs name)
                    matches.sort_by(|a, b| {
                        match a.kind.cmp(&b.kind) {
                            std::cmp::Ordering::Equal => {
                                // If prefer_name, name matches come first (is_hash_match=false < true)
                                // If prefer_hash (default), hash matches come first (is_hash_match=true < false)
                                if prefer_name {
                                    a.is_hash_match.cmp(&b.is_hash_match)
                                } else {
                                    b.is_hash_match.cmp(&a.is_hash_match)
                                }
                            }
                            other => other,
                        }
                    });

                    let resp = postcard::to_allocvec(&MetaResponse::Find { matches })
                        .map_err(AcceptError::from_err)?;
                    send.write_all(&resp).await.map_err(AcceptError::from_err)?;
                    send.finish()?;
                }
                MetaRequest::ListPeers => {
                    let peers = self
                        .peer_discovery
                        .as_ref()
                        .map(|pd| {
                            pd.peers()
                                .into_iter()
                                .map(|pi| pi.announcement)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let resp = postcard::to_allocvec(&MetaResponse::ListPeers { peers })
                        .map_err(AcceptError::from_err)?;
                    send.write_all(&resp).await.map_err(AcceptError::from_err)?;
                    send.finish()?;
                }
            }
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

    #[test]
    fn test_list_peers_is_last_discriminant() {
        // Verify that ListPeers has the highest discriminant value.
        // This test will fail at compile time if a new variant is added
        // after ListPeers (the match won't compile), reminding developers
        // to keep ListPeers last.
        let req = MetaRequest::ListPeers;
        let bytes = postcard::to_allocvec(&req).unwrap();
        // ListPeers should be discriminant 7 (0-indexed: Put=0, Get=1, List=2,
        // Delete=3, Rename=4, Copy=5, Find=6, ListPeers=7)
        assert_eq!(
            bytes[0], 7,
            "ListPeers should be the 8th variant (discriminant 7)"
        );

        let resp = MetaResponse::ListPeers { peers: vec![] };
        let bytes = postcard::to_allocvec(&resp).unwrap();
        assert_eq!(
            bytes[0], 7,
            "ListPeers response should be the 8th variant (discriminant 7)"
        );
    }

    #[test]
    fn test_existing_variants_unchanged_after_list_peers() {
        // Verify adding ListPeers didn't shift existing discriminants.
        // This is critical for wire compatibility.
        let put_bytes = postcard::to_allocvec(&MetaRequest::Put {
            filename: "x".to_owned(),
            hash: Hash::from_bytes([0u8; 32]),
        })
        .unwrap();
        assert_eq!(put_bytes[0], 0, "Put should still be discriminant 0");

        let list_bytes = postcard::to_allocvec(&MetaRequest::List).unwrap();
        assert_eq!(list_bytes[0], 2, "List should still be discriminant 2");

        let find_bytes = postcard::to_allocvec(&MetaRequest::Find {
            query: "x".to_owned(),
            prefer_name: false,
        })
        .unwrap();
        assert_eq!(find_bytes[0], 6, "Find should still be discriminant 6");
    }
}
