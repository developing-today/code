//! Protocol module - defines the meta protocol for remote operations

use futures_lite::StreamExt;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh_blobs::{api::Store, Hash};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Match quality for find/search operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MatchKind {
    Exact,    // Best: exact match
    Prefix,   // Good: starts with query
    Contains, // Okay: contains query
}

/// A single match result from find/search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindMatch {
    pub hash: Hash,
    pub name: String,
    pub kind: MatchKind,
    pub is_hash_match: bool, // true if matched against hash, false if matched against name
}

/// FindMatch with the query that matched (for multi-query support)
#[derive(Debug, Clone)]
pub struct TaggedMatch {
    pub query: String,
    pub hash: Hash,
    pub name: String,
    pub kind: MatchKind,
    pub is_hash_match: bool,
}

/// Requests that can be sent to a remote node
#[derive(Debug, Serialize, Deserialize)]
pub enum MetaRequest {
    Put { filename: String, hash: Hash },
    Get { filename: String },
    List,
    Delete { filename: String },
    Rename { from: String, to: String },
    Copy { from: String, to: String },
    Find { query: String, prefer_name: bool },
}

/// Responses from a remote node
#[derive(Debug, Serialize, Deserialize)]
pub enum MetaResponse {
    Put { success: bool },
    Get { hash: Option<Hash> },
    List { items: Vec<(Hash, String)> },
    Delete { success: bool },
    Rename { success: bool },
    Copy { success: bool },
    Find { matches: Vec<FindMatch> },
}

/// Protocol handler for metadata operations
#[derive(Clone, Debug)]
pub struct MetaProtocol {
    pub store: Store,
}

impl MetaProtocol {
    pub fn new(store: &Store) -> Arc<Self> {
        Arc::new(Self {
            store: store.clone(),
        })
    }

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
    async fn accept(&self, conn: Connection) -> std::result::Result<(), AcceptError> {
        // Handle multiple requests per connection
        loop {
            let (mut send, mut recv) = match conn.accept_bi().await {
                Ok(streams) => streams,
                Err(_) => break, // Connection closed
            };
            let buf = match recv.read_to_end(64 * 1024).await {
                Ok(buf) => buf,
                Err(_) => break,
            };
            let req: MetaRequest = match postcard::from_bytes(&buf) {
                Ok(req) => req,
                Err(_) => break,
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
                    } else {
                        if let Ok(mut list) = self.store.tags().list().await {
                            while let Some(item) = list.next().await {
                                let item = item.map_err(AcceptError::from_err)?;
                                if item.name.as_ref() == filename.as_bytes() {
                                    found = Some(item.hash);
                                    break;
                                }
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
            }
        }
        Ok(())
    }
}

#[cfg(test)]
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
        assert_eq!(MetaProtocol::match_kind("hello", ""), Some(MatchKind::Prefix));
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
            name: "test.txt".to_string(),
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
            query: "test".to_string(),
            hash,
            name: "test.txt".to_string(),
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
            filename: "test.txt".to_string(),
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
            filename: "myfile.txt".to_string(),
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
            filename: "to_delete.txt".to_string(),
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
            from: "old.txt".to_string(),
            to: "new.txt".to_string(),
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
            from: "source.txt".to_string(),
            to: "dest.txt".to_string(),
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
            query: "search term".to_string(),
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
                (hash1, "file1.txt".to_string()),
                (hash2, "file2.txt".to_string()),
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
            name: "found.txt".to_string(),
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
            name: "serialized.txt".to_string(),
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
}
