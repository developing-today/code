//! File operations, re-exported from `id-sync`.
//!
//! Step 3b of the pkgs/id crate split (#4406): the implementation of
//! this module lives in `id_sync::fileops`. This facade module holds
//! the re-exports and the tests that run against the facade's own
//! `LocalNode` (which stays here until step 7), so every historical
//! `id::fileops` path is unchanged.

pub use id_sync::fileops::*;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod replacement_metadata_tests {
    use crate::local::LocalNode;
    use crate::tags::Tag;

    use super::*;

    async fn put(node: &LocalNode, name: &str, content: &[u8]) {
        let guard = node
            .blobs()
            .add_bytes(content.to_vec())
            .temp_tag()
            .await
            .unwrap();
        node.ops().put(name, guard.hash()).await.unwrap();
    }

    async fn set_tag(node: &LocalNode, subject: &str, key: &str, value: &str) {
        node.tags()
            .set_tag(
                &node.tags().global,
                subject.as_bytes(),
                key.as_bytes(),
                Some(value.as_bytes()),
                b"",
            )
            .await
            .unwrap();
    }

    async fn set_singleton(node: &LocalNode, subject: &str, key: &str, value: &str) {
        node.tags()
            .set_singleton(
                &node.tags().global,
                subject.as_bytes(),
                key.as_bytes(),
                Some(value.as_bytes()),
                b"",
            )
            .await
            .unwrap();
    }

    async fn tags_of(node: &LocalNode, subject: &str) -> Vec<Tag> {
        node.tags()
            .get_tags(&node.tags().global, subject.as_bytes())
            .await
            .unwrap()
    }

    fn values(tags: &[Tag], key: &str) -> Vec<Vec<u8>> {
        tags.iter()
            .filter(|tag| tag.key.as_bytes() == key.as_bytes())
            .filter_map(|tag| tag.value.as_ref().map(|value| value.as_bytes().to_vec()))
            .collect()
    }

    fn has_tag(tags: &[Tag], key: &str, value: &str) -> bool {
        values(tags, key)
            .iter()
            .any(|found| found == value.as_bytes())
    }

    fn assert_single_value(tags: &[Tag], key: &str, value: &str) {
        assert_eq!(
            values(tags, key),
            vec![value.as_bytes().to_vec()],
            "{tags:?}"
        );
    }

    fn assert_archive_identity(tags: &[Tag], archive: &str) {
        assert_single_value(tags, "name", archive);
        assert_single_value(tags, "file", archive.rsplit('/').next().unwrap_or(archive));
        assert!(
            values(tags, "path").is_empty(),
            "archive inherited path: {tags:?}"
        );
    }

    async fn seed_replacement_pair(node: &LocalNode) {
        put(node, "source.txt", b"new content").await;
        put(node, "destination.txt", b"replaced content").await;

        set_tag(node, "source.txt", "shared", "source-value").await;
        set_tag(node, "source.txt", "source-only", "source-metadata").await;
        set_singleton(node, "source.txt", "created", "source-created").await;
        set_singleton(node, "source.txt", "modified", "source-modified").await;

        set_tag(node, "destination.txt", "shared", "replaced-value").await;
        set_tag(
            node,
            "destination.txt",
            "replaced-only",
            "replaced-metadata",
        )
        .await;
        set_tag(
            node,
            "destination.txt",
            "archive.previous",
            "earlier.archive.7",
        )
        .await;
        set_tag(node, "destination.txt", "path", "old/source/path.txt").await;
        set_singleton(node, "destination.txt", "created", "replaced-created").await;
        set_singleton(node, "destination.txt", "modified", "replaced-modified").await;
    }

    fn assert_replaced_metadata_on_archive(tags: &[Tag], archive: &str) {
        assert!(has_tag(tags, "shared", "replaced-value"), "{tags:?}");
        assert!(
            has_tag(tags, "replaced-only", "replaced-metadata"),
            "{tags:?}"
        );
        assert!(
            has_tag(tags, "archive.previous", "earlier.archive.7"),
            "{tags:?}"
        );
        assert_single_value(tags, "created", "replaced-created");
        assert_single_value(tags, "modified", "replaced-modified");
        assert_archive_identity(tags, archive);
        assert!(!has_tag(tags, "name", "destination.txt"), "{tags:?}");
        assert!(!has_tag(tags, "file", "destination.txt"), "{tags:?}");
    }

    #[tokio::test]
    async fn rename_with_archive_preserves_source_provenance_on_archive_alias() {
        let node = LocalNode::open(true).await.unwrap();
        put(&node, "before.txt", b"original").await;
        set_singleton(&node, "before.txt", "origin", "node-alpha").await;
        set_singleton(&node, "before.txt", "created", "1700000000").await;

        // The source's recorded provenance, exactly as stored.
        let source_tags = tags_of(&node, "before.txt").await;
        assert_single_value(&source_tags, "origin", "node-alpha");
        let source_created = values(&source_tags, "created");
        assert!(!source_created.is_empty(), "{source_tags:?}");

        let rename = node
            .ops()
            .rename("before.txt", "after.txt", true)
            .await
            .unwrap();
        let archive = rename.archived_original.unwrap();

        // #5383: the archive alias carries the source's recorded
        // provenance, not an unknown origin.
        let archive_tags = tags_of(&node, &archive).await;
        assert_single_value(&archive_tags, "origin", "node-alpha");
        assert_eq!(values(&archive_tags, "created"), source_created);

        // The live name keeps its provenance too (tags move with it).
        let live_tags = tags_of(&node, "after.txt").await;
        assert_single_value(&live_tags, "origin", "node-alpha");
        assert_eq!(values(&live_tags, "created"), source_created);

        node.shutdown().await.unwrap();
    }

    async fn rename_onto_existing_moves_replaced_metadata_and_refreshes_archive_identity() {
        let node = LocalNode::open(true).await.unwrap();
        seed_replacement_pair(&node).await;

        let outcome = node
            .ops()
            .rename("source.txt", "destination.txt", false)
            .await
            .unwrap();
        let archive = outcome.archived_replaced.expect("replaced file archived");

        let archived = tags_of(&node, &archive).await;
        assert_replaced_metadata_on_archive(&archived, &archive);

        let destination = tags_of(&node, "destination.txt").await;
        assert!(
            has_tag(&destination, "shared", "source-value"),
            "{destination:?}"
        );
        assert!(
            has_tag(&destination, "source-only", "source-metadata"),
            "{destination:?}"
        );
        assert!(
            !has_tag(&destination, "shared", "replaced-value"),
            "{destination:?}"
        );
        assert!(
            !has_tag(&destination, "replaced-only", "replaced-metadata"),
            "{destination:?}"
        );
        assert!(
            !has_tag(&destination, "archive.previous", "earlier.archive.7"),
            "{destination:?}"
        );
        assert!(
            !has_tag(&destination, "path", "old/source/path.txt"),
            "{destination:?}"
        );
        assert_single_value(&destination, "created", "source-created");
        assert_single_value(&destination, "modified", "source-modified");
        assert_single_value(&destination, "name", "destination.txt");
        assert_single_value(&destination, "file", "destination.txt");
        assert!(
            has_tag(&destination, "archive.replace", &archive),
            "{destination:?}"
        );

        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn copy_onto_existing_moves_replaced_metadata_and_keeps_single_fresh_stamps() {
        let node = LocalNode::open(true).await.unwrap();
        seed_replacement_pair(&node).await;

        let outcome = node
            .ops()
            .copy("source.txt", "destination.txt")
            .await
            .unwrap();
        let archive = outcome.archived_replaced.expect("replaced file archived");

        let archived = tags_of(&node, &archive).await;
        assert_replaced_metadata_on_archive(&archived, &archive);

        let destination = tags_of(&node, "destination.txt").await;
        assert!(
            has_tag(&destination, "shared", "source-value"),
            "{destination:?}"
        );
        assert!(
            has_tag(&destination, "source-only", "source-metadata"),
            "{destination:?}"
        );
        assert!(
            !has_tag(&destination, "shared", "replaced-value"),
            "{destination:?}"
        );
        assert!(
            !has_tag(&destination, "replaced-only", "replaced-metadata"),
            "{destination:?}"
        );
        assert!(
            !has_tag(&destination, "archive.previous", "earlier.archive.7"),
            "{destination:?}"
        );
        assert!(
            !has_tag(&destination, "path", "old/source/path.txt"),
            "{destination:?}"
        );
        assert_eq!(values(&destination, "created").len(), 1, "{destination:?}");
        assert_eq!(values(&destination, "modified").len(), 1, "{destination:?}");
        assert_single_value(&destination, "name", "destination.txt");
        assert_single_value(&destination, "file", "destination.txt");
        assert!(
            has_tag(&destination, "archive.replace", &archive),
            "{destination:?}"
        );

        let source = tags_of(&node, "source.txt").await;
        assert!(has_tag(&source, "shared", "source-value"), "{source:?}");
        assert!(
            has_tag(&source, "source-only", "source-metadata"),
            "{source:?}"
        );

        node.shutdown().await.unwrap();
    }

    /// A copy must not become a second owner of the source's archives
    /// (#4761): deleting the copy must leave the source's archives intact.
    #[tokio::test]
    async fn copy_does_not_inherit_source_archive_refs() {
        let node = LocalNode::open(true).await.unwrap();
        put(&node, "source.txt", b"source content").await;
        put(&node, "other.txt", b"other content").await;
        // Renaming onto source.txt with archiving enabled adds both
        // archive.rename and archive.replace refs to the live source.
        let rename_outcome = node
            .ops()
            .rename("other.txt", "source.txt", true)
            .await
            .unwrap();
        let source_archives = [
            rename_outcome
                .archived_original
                .expect("original source archived"),
            rename_outcome
                .archived_replaced
                .expect("replaced source archived"),
        ];
        let source_before = tags_of(&node, "source.txt").await;
        assert!(
            has_tag(&source_before, "archive.rename", &source_archives[0]),
            "{source_before:?}"
        );
        assert!(
            has_tag(&source_before, "archive.replace", &source_archives[1]),
            "{source_before:?}"
        );

        node.ops().copy("source.txt", "copy.txt").await.unwrap();
        let copy_tags = tags_of(&node, "copy.txt").await;
        assert!(
            values(&copy_tags, "archive.rename").is_empty(),
            "copy inherited archive.rename refs: {copy_tags:?}"
        );
        assert!(
            values(&copy_tags, "archive.replace").is_empty(),
            "copy inherited archive.replace refs: {copy_tags:?}"
        );

        // Deleting the copy must not touch the source's archive.
        let delete_outcome = node.ops().delete("copy.txt").await.unwrap();
        assert_eq!(
            delete_outcome.archives_removed, 0,
            "copy delete removed archives"
        );
        for archive in &source_archives {
            assert!(
                !tags_of(&node, archive).await.is_empty(),
                "source archive {archive} removed by copy delete"
            );
        }
        let source_after = tags_of(&node, "source.txt").await;
        assert!(
            has_tag(&source_after, "archive.rename", &source_archives[0]),
            "source lost its archive.rename ref: {source_after:?}"
        );
        assert!(
            has_tag(&source_after, "archive.replace", &source_archives[1]),
            "source lost its archive ref: {source_after:?}"
        );

        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn rename_onto_existing_preserves_replaced_tag_payloads() {
        let node = LocalNode::open(true).await.unwrap();
        put(&node, "source.txt", b"new content").await;
        put(&node, "destination.txt", b"replaced content").await;

        // A tag with a non-empty entry payload, as written for legacy
        // links by migrate_legacy_meta (payload carries the link target).
        let payload: &[u8] = b"hash:0123abcd";
        node.tags()
            .set_tag(
                &node.tags().global,
                b"destination.txt",
                b"link",
                Some(b"target"),
                payload,
            )
            .await
            .unwrap();
        let before = tags_of(&node, "destination.txt").await;
        let before = before
            .iter()
            .find(|tag| tag.key.as_bytes() == b"link")
            .expect("link tag seeded");
        assert_eq!(before.len as usize, payload.len(), "{before:?}");
        let before_hash = before.hash;

        let outcome = node
            .ops()
            .rename("source.txt", "destination.txt", false)
            .await
            .unwrap();
        let archive = outcome.archived_replaced.expect("replaced file archived");

        let archived = tags_of(&node, &archive).await;
        let moved = archived
            .iter()
            .find(|tag| tag.key.as_bytes() == b"link")
            .expect("link tag moved to archive");
        assert_eq!(moved.hash, before_hash, "{moved:?}");
        assert_eq!(moved.len as usize, payload.len(), "{moved:?}");

        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn delete_removes_archived_replaced_metadata() {
        let node = LocalNode::open(true).await.unwrap();
        seed_replacement_pair(&node).await;

        let outcome = node
            .ops()
            .rename("source.txt", "destination.txt", false)
            .await
            .unwrap();
        let archive = outcome.archived_replaced.expect("replaced file archived");
        assert!(
            !tags_of(&node, &archive).await.is_empty(),
            "archive carries the replaced file's metadata before delete"
        );

        let deleted = node.ops().delete("destination.txt").await.unwrap();
        assert_eq!(deleted.archives_removed, 1);
        assert!(
            tags_of(&node, &archive).await.is_empty(),
            "archive metadata removed with the archive"
        );

        node.shutdown().await.unwrap();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod delete_tests {
    use crate::local::LocalNode;

    use super::*;

    async fn put(node: &LocalNode, name: &str, data: &[u8]) {
        let guard = node
            .blobs()
            .add_bytes(data.to_vec())
            .temp_tag()
            .await
            .unwrap();
        node.ops().put(name, guard.hash()).await.unwrap();
    }

    #[tokio::test]
    async fn delete_after_archived_rename_removes_previous_name_archive() {
        let node = LocalNode::open(true).await.unwrap();
        put(&node, "before", b"original").await;

        let rename = node.ops().rename("before", "after", true).await.unwrap();
        let archive = rename.archived_original.unwrap();
        assert!(node.blobs().tags().get(&archive).await.unwrap().is_some());

        let deleted = node.ops().delete("after").await.unwrap();

        assert_eq!(deleted.archives_removed, 1);
        assert!(node.blobs().tags().get(&archive).await.unwrap().is_none());
        node.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn delete_after_repeated_replacements_removes_all_replaced_archives() {
        let node = LocalNode::open(true).await.unwrap();
        put(&node, "A", b"original").await;

        put(&node, "replacement-1", b"first replacement").await;
        let first = node
            .ops()
            .rename("replacement-1", "A", false)
            .await
            .unwrap()
            .archived_replaced
            .unwrap();
        put(&node, "replacement-2", b"second replacement").await;
        let second = node
            .ops()
            .rename("replacement-2", "A", false)
            .await
            .unwrap()
            .archived_replaced
            .unwrap();
        assert_ne!(first, second);

        node.ops().rename("A", "B", false).await.unwrap();
        let deleted = node.ops().delete("B").await.unwrap();

        assert_eq!(deleted.archives_removed, 2);
        assert!(node.blobs().tags().get(&first).await.unwrap().is_none());
        assert!(node.blobs().tags().get(&second).await.unwrap().is_none());
        node.shutdown().await.unwrap();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod name_rule_tests {
    use crate::local::LocalNode;

    use super::*;

    async fn put(node: &LocalNode, name: &str, data: &[u8]) {
        let guard = node
            .blobs()
            .add_bytes(data.to_vec())
            .temp_tag()
            .await
            .unwrap();
        node.ops().put(name, guard.hash()).await.unwrap();
    }

    fn assert_invalid(name: &str) {
        assert!(
            matches!(
                FileOps::validate_name(name),
                Err(FileOpError::InvalidName(_))
            ),
            "{name:?} must be refused as InvalidName"
        );
    }

    /// Padding and empty components are refused by the one canonical rule
    /// (#4102 / design #4766 Phase 3), so a name can never be stored in a
    /// form that rename/copy would normalise away.
    #[test]
    fn refuses_padding_and_empty_components() {
        for name in [
            " report.txt",
            "report.txt ",
            "\treport.txt",
            "report.txt\n",
            "a//b",
            "a/",
            "a\\",
            "a//b/c",
            "a/b/",
        ] {
            assert_invalid(name);
        }
    }

    /// The pre-existing refusals stay in place: rooted names, drive
    /// prefixes, and `.`/`..` traversal components.
    #[test]
    fn preserves_traversal_refusals() {
        for name in [
            "/etc/passwd",
            "\\windows",
            "C:file.txt",
            ".",
            "..",
            "a/./b",
            "a/../b",
        ] {
            assert_invalid(name);
        }
        assert!(matches!(
            FileOps::validate_name("   "),
            Err(FileOpError::EmptyName)
        ));
        // Padding is a refusal, but interior spaces in a name are not.
        assert!(FileOps::validate_name("my report.txt").is_ok());
        assert!(FileOps::validate_name("a/b").is_ok());
    }

    /// A padded `put` cannot create an untargetable twin, and a padded
    /// rename/copy target is refused instead of being silently trimmed.
    #[tokio::test]
    async fn padded_names_cannot_create_twins() {
        let node = LocalNode::open(true).await.unwrap();
        put(&node, "report.txt", b"report").await;

        let guard = node
            .blobs()
            .add_bytes(b"report".to_vec())
            .temp_tag()
            .await
            .unwrap();
        assert!(matches!(
            node.ops().put(" report.txt", guard.hash()).await,
            Err(FileOpError::InvalidName(_))
        ));
        assert!(
            node.blobs()
                .tags()
                .get(" report.txt")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            node.blobs()
                .tags()
                .get("report.txt")
                .await
                .unwrap()
                .is_some()
        );

        assert!(matches!(
            node.ops().rename("report.txt", "other.txt ", false).await,
            Err(FileOpError::InvalidName(_))
        ));
        assert!(
            node.blobs()
                .tags()
                .get("other.txt")
                .await
                .unwrap()
                .is_none()
        );

        assert!(matches!(
            node.ops().copy("report.txt", " copy.txt").await,
            Err(FileOpError::InvalidName(_))
        ));
        assert!(
            node.blobs()
                .tags()
                .get(" copy.txt")
                .await
                .unwrap()
                .is_none()
        );
        assert!(node.blobs().tags().get("copy.txt").await.unwrap().is_none());

        node.shutdown().await.unwrap();
    }

    /// Empty components are refused at put and at rename/copy targets alike.
    #[tokio::test]
    async fn empty_components_refused_across_put_rename_copy() {
        let node = LocalNode::open(true).await.unwrap();
        put(&node, "a/b", b"body").await;

        let guard = node
            .blobs()
            .add_bytes(b"body".to_vec())
            .temp_tag()
            .await
            .unwrap();
        for name in ["a//b", "a/", "a\\"] {
            assert!(matches!(
                node.ops().put(name, guard.hash()).await,
                Err(FileOpError::InvalidName(_))
            ));
            assert!(matches!(
                node.ops().rename("a/b", name, false).await,
                Err(FileOpError::InvalidName(_))
            ));
            assert!(matches!(
                node.ops().copy("a/b", name).await,
                Err(FileOpError::InvalidName(_))
            ));
            assert!(node.blobs().tags().get(name).await.unwrap().is_none());
        }

        // The canonical source is untouched and ordinary names still work.
        assert!(node.blobs().tags().get("a/b").await.unwrap().is_some());
        node.ops().copy("a/b", "a/c").await.unwrap();
        assert!(node.blobs().tags().get("a/c").await.unwrap().is_some());

        node.shutdown().await.unwrap();
    }
}
