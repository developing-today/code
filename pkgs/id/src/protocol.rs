//! The meta protocol, re-exported from `id-sync`.
//!
//! Step 3b of the pkgs/id crate split (#4406): the implementation of
//! this module — the wire types, the [`MetaProtocol`] handler and the
//! framing limits — lives in `id_sync::protocol`. This facade module
//! holds the re-exports and the handler tests that run against the
//! facade's own `LocalNode` (which stays here until step 7), so every
//! historical `id::protocol` path is unchanged.

pub use id_sync::protocol::*;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use iroh::{
        EndpointAddr,
        endpoint::{Endpoint, RelayMode, presets},
        protocol::Router,
    };
    use iroh_base::{EndpointId, SecretKey, TransportAddr};
    use std::net::{Ipv4Addr, SocketAddr};

    use crate::{META_ALPN, local::LocalNode};

    use anyhow::Result;
    use iroh::endpoint::Connection;
    use iroh::protocol::{AcceptError, ProtocolHandler};
    use iroh_blobs::Hash;
    use std::sync::Arc;
    use std::time::Duration;

    use crate::access::AccessPolicy;

    fn eid(n: u8) -> EndpointId {
        iroh_base::SecretKey::from_bytes(&[n; 32]).public()
    }

    #[derive(Clone, Debug)]
    struct DeadlineMetaProtocol {
        protocol: Arc<MetaProtocol>,
        deadline: Duration,
    }

    impl ProtocolHandler for DeadlineMetaProtocol {
        async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
            self.protocol.accept_connection(conn, self.deadline).await
        }
    }

    struct MetaTestServer {
        node: LocalNode,
        endpoint: Endpoint,
        router: Router,
        addr: EndpointAddr,
    }

    async fn meta_test_server(deadline: Duration) -> MetaTestServer {
        let node = LocalNode::open(true).await.unwrap();
        let key = SecretKey::generate();
        let server_id = key.public();
        let endpoint = Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .secret_key(key)
            .bind_addr(std::net::SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .bind()
            .await
            .unwrap();
        let protocol = MetaProtocol::new(
            &node.blobs(),
            None,
            Arc::clone(node.tags()),
            AccessPolicy::open(),
            server_id,
            None,
        );
        let handler = DeadlineMetaProtocol { protocol, deadline };
        let router = Router::builder(endpoint.clone())
            .accept(META_ALPN, handler)
            .spawn();
        let addr = EndpointAddr::from_parts(
            server_id,
            endpoint.bound_sockets().into_iter().map(|address| {
                TransportAddr::Ip(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), address.port()))
            }),
        );
        MetaTestServer {
            node,
            endpoint,
            router,
            addr,
        }
    }

    async fn meta_test_client() -> Endpoint {
        Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap()
    }

    async fn shutdown_meta_test(server: MetaTestServer, client: Endpoint) {
        client.close().await;
        server.router.shutdown().await.unwrap();
        server.endpoint.close().await;
        server.node.shutdown().await.unwrap();
    }

    // ========================================================================
    // Handler behaviour against real iroh-blobs + iroh-docs (in memory)
    // ========================================================================

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
            MetaResponse::List { items } => items.into_iter().map(|e| e.name).collect(),
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn list_entries(node: &LocalNode) -> Vec<ListEntry> {
        match node.request(MetaRequest::List).await {
            MetaResponse::List { items } => items,
            other => panic!("unexpected {other:?}"),
        }
    }

    /// #4973: a file written through the meta protocol lists with the
    /// writing node as its origin and its `created` stamp as the upload
    /// time.
    #[tokio::test]
    async fn list_shows_origin_node_and_upload_time_for_new_files() {
        let node = LocalNode::open(true).await.unwrap();
        let guard = node
            .blobs()
            .add_bytes(b"hello origin".to_vec())
            .temp_tag()
            .await
            .unwrap();
        let resp = node
            .request(MetaRequest::Put {
                filename: "origin.txt".to_owned(),
                hash: guard.hash(),
            })
            .await;
        assert!(
            matches!(resp, MetaResponse::Put { success: true }),
            "{resp:?}"
        );

        let entries = list_entries(&node).await;
        let entry = entries
            .iter()
            .find(|e| e.name == "origin.txt")
            .expect("written file must list");
        assert_eq!(
            entry.origin.as_deref(),
            Some(node.node_id().to_string().as_str()),
            "origin must be the writing node"
        );
        assert!(
            entry.uploaded_at.is_some_and(|ts| ts > 0),
            "upload time must come from the created stamp: {entry:?}"
        );
    }

    /// #4973: a file that predates origin stamping (name tag only, no
    /// metadata tags) lists cleanly with origin/upload time unknown —
    /// never guessed.
    #[tokio::test]
    async fn list_shows_unknown_origin_for_pre_change_files() {
        let node = LocalNode::open(true).await.unwrap();
        let guard = node
            .blobs()
            .add_bytes(b"old file".to_vec())
            .temp_tag()
            .await
            .unwrap();
        node.blobs()
            .tags()
            .set("legacy.txt", guard.hash())
            .await
            .unwrap();

        let entries = list_entries(&node).await;
        let entry = entries
            .iter()
            .find(|e| e.name == "legacy.txt")
            .expect("legacy file must list");
        assert_eq!(entry.origin, None);
        assert_eq!(entry.uploaded_at, None);
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
    async fn abuse_counters_without_an_evaluator_answer_the_empty_report() {
        // Oracle: a node with no abuse evaluator attached (LocalNode never
        // attaches one, exactly like a plain `id serve` without `--world`)
        // records no world traffic, so AbuseCounters is the empty report —
        // never an error. `id peers --abuse` depends on this to succeed
        // with no rows (#4918).
        let node = LocalNode::open(true).await.unwrap();
        let resp = node.request(MetaRequest::AbuseCounters).await;
        assert!(
            matches!(
                resp,
                MetaResponse::AbuseCounters {
                    ref rows,
                    policy: None
                } if rows.is_empty()
            ),
            "expected the empty abuse report, got {resp:?}"
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
    async fn fileops_reject_unsafe_names_without_mutating_files() {
        let node = LocalNode::open(true).await.unwrap();
        let source_hash = put(&node, "source.txt", b"source").await;
        let upload = node
            .blobs()
            .add_bytes(b"candidate".to_vec())
            .temp_tag()
            .await
            .unwrap();
        let put_hash = upload.hash();
        let invalid_names = [
            "../escape.txt",
            "nested/../escape.txt",
            "./dot.txt",
            "/rooted.txt",
            r"..\escape.txt",
            r"nested\..\escape.txt",
            r"\rooted.txt",
            r"\\server\share.txt",
            r"C:\rooted.txt",
            "C:/rooted.txt",
            "C:relative.txt",
        ];

        for invalid in invalid_names {
            let put_response = node
                .request(MetaRequest::Put {
                    filename: invalid.to_owned(),
                    hash: put_hash,
                })
                .await;
            assert!(
                put_response
                    .error_message()
                    .is_some_and(|message| message.contains("safe relative path")),
                "put accepted unsafe name {invalid:?}: {put_response:?}"
            );

            let rename_response = node
                .request(MetaRequest::Rename {
                    from: "source.txt".to_owned(),
                    to: invalid.to_owned(),
                })
                .await;
            assert!(
                rename_response
                    .error_message()
                    .is_some_and(|message| message.contains("safe relative path")),
                "rename accepted unsafe name {invalid:?}: {rename_response:?}"
            );

            let copy_response = node
                .request(MetaRequest::Copy {
                    from: "source.txt".to_owned(),
                    to: invalid.to_owned(),
                })
                .await;
            assert!(
                copy_response
                    .error_message()
                    .is_some_and(|message| message.contains("safe relative path")),
                "copy accepted unsafe name {invalid:?}: {copy_response:?}"
            );

            assert_eq!(names(&node).await, vec!["source.txt".to_owned()]);
            assert!(tags_of(&node, invalid).await.is_empty());
            assert!(matches!(
                node.request(MetaRequest::Get {
                    filename: "source.txt".to_owned(),
                })
                .await,
                MetaResponse::Get { hash: Some(hash) } if hash == source_hash
            ));
        }

        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn fileops_rejects_archive_names_without_overwriting_existing_archives() {
        let node = LocalNode::open(true).await.unwrap();
        let archive_hash = put(&node, "report.txt", b"original").await;
        assert!(matches!(
            node.request(MetaRequest::Rename {
                from: "report.txt".to_owned(),
                to: "renamed.txt".to_owned(),
            })
            .await,
            MetaResponse::Rename { success: true }
        ));
        let archive_name = names(&node)
            .await
            .into_iter()
            .find(|name| name.starts_with("report.txt.archive."))
            .expect("rename should create an archive");
        let replacement_hash = put(&node, "source.txt", b"replacement").await;

        let put_response = node
            .request(MetaRequest::Put {
                filename: archive_name.clone(),
                hash: replacement_hash,
            })
            .await;
        assert!(
            put_response
                .error_message()
                .is_some_and(|message| message.contains("reserved for archives")),
            "put accepted archive-shaped name: {put_response:?}"
        );

        let rename_response = node
            .request(MetaRequest::Rename {
                from: "source.txt".to_owned(),
                to: archive_name.clone(),
            })
            .await;
        assert!(
            rename_response
                .error_message()
                .is_some_and(|message| message.contains("reserved for archives")),
            "rename accepted archive-shaped name: {rename_response:?}"
        );

        let copy_response = node
            .request(MetaRequest::Copy {
                from: "source.txt".to_owned(),
                to: archive_name.clone(),
            })
            .await;
        assert!(
            copy_response
                .error_message()
                .is_some_and(|message| message.contains("reserved for archives")),
            "copy accepted archive-shaped name: {copy_response:?}"
        );

        assert!(matches!(
            node.request(MetaRequest::Get {
                filename: archive_name.clone(),
            })
            .await,
            MetaResponse::Get { hash: Some(hash) } if hash == archive_hash
        ));
        assert!(names(&node).await.contains(&archive_name));
        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn fileops_accept_safe_nested_names_for_rename_and_copy() {
        let node = LocalNode::open(true).await.unwrap();
        let hash = put(&node, "dir/a.txt", b"hello").await;

        assert!(matches!(
            node.request(MetaRequest::Rename {
                from: "dir/a.txt".to_owned(),
                to: "renamed/deep.txt".to_owned(),
            })
            .await,
            MetaResponse::Rename { success: true }
        ));
        assert!(matches!(
            node.request(MetaRequest::Copy {
                from: "renamed/deep.txt".to_owned(),
                to: "copies/copy.txt".to_owned(),
            })
            .await,
            MetaResponse::Copy { success: true }
        ));

        assert!(matches!(
            node.request(MetaRequest::Get {
                filename: "renamed/deep.txt".to_owned(),
            })
            .await,
            MetaResponse::Get { hash: Some(found) } if found == hash
        ));
        assert!(matches!(
            node.request(MetaRequest::Get {
                filename: "copies/copy.txt".to_owned(),
            })
            .await,
            MetaResponse::Get { hash: Some(found) } if found == hash
        ));

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
            None,
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
            None,
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

    #[tokio::test]
    async fn stalled_meta_stream_does_not_block_later_requests_and_times_out() {
        let server = meta_test_server(Duration::from_millis(200)).await;
        let client = meta_test_client().await;
        let conn = client
            .connect(server.addr.clone(), META_ALPN)
            .await
            .unwrap();

        let (mut stalled_send, mut stalled_recv) = conn.open_bi().await.unwrap();
        stalled_send.write_all(b"\0").await.unwrap();

        let response = tokio::time::timeout(
            Duration::from_secs(1),
            crate::meta_client::request_over(&conn, &MetaRequest::Whoami),
        )
        .await
        .expect("a stalled stream must not block another request")
        .unwrap();
        assert!(
            matches!(response, MetaResponse::Whoami { .. }),
            "{response:?}"
        );

        let timed_out =
            tokio::time::timeout(Duration::from_secs(1), stalled_recv.read_to_end(64 * 1024))
                .await
                .expect("the stalled stream should be reset at its deadline");
        assert!(
            timed_out.is_err(),
            "timed-out stream unexpectedly completed: {timed_out:?}"
        );

        drop(stalled_send);
        drop(conn);
        shutdown_meta_test(server, client).await;
    }

    #[tokio::test]
    async fn excess_meta_streams_are_reset() {
        let server = meta_test_server(Duration::from_secs(5)).await;
        let client = meta_test_client().await;
        let conn = client
            .connect(server.addr.clone(), META_ALPN)
            .await
            .unwrap();
        let mut streams = Vec::new();

        for _ in 0..=MAX_META_STREAMS_PER_CONNECTION {
            let (mut send, recv) = conn.open_bi().await.unwrap();
            // A stream the client never writes to is never observed by the
            // server's accept loop (noq only surfaces a stream once it
            // carries data), so write one incomplete-request byte to make
            // each stream visible — the same trick the stalled-stream test
            // uses. Without this the excess stream is never reset and the
            // test times out regardless of the implementation.
            send.write_all(b" ").await.unwrap();
            streams.push((send, recv));
        }

        let excess = streams.last_mut().unwrap();
        let reset = tokio::time::timeout(Duration::from_secs(1), excess.1.read_to_end(64 * 1024))
            .await
            .expect("the excess stream should be reset promptly");
        assert!(
            reset.is_err(),
            "excess stream unexpectedly completed: {reset:?}"
        );

        drop(streams);
        drop(conn);
        shutdown_meta_test(server, client).await;
    }
}
