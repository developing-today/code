//! Metadata tag system for rich file metadata.
//!
//! Provides a structured tag system stored as a JSON document in the iroh-blobs
//! store. Tags support key-only or key-value entries, optional linking to
//! content hashes or filenames, and automatic create/modify timestamps.
//!
//! # Storage
//!
//! All metadata is stored as a single JSON blob referenced by the reserved
//! iroh-blobs tag `.meta`. The blob is loaded, modified in memory, and saved
//! back atomically.
//!
//! # Tag Model
//!
//! Each tag has:
//! - A **subject** (what entity the tag describes, usually a filename)
//! - A **key** (tag identifier like "created", "category", etc.)
//! - An optional **value** (for key-value tags)
//! - An optional **link** to a content hash or filename
//! - Automatic `created_at` and `modified_at` timestamps
//!
//! Keys are not forced unique — multiple tags with the same subject+key are allowed.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use iroh_blobs::api::Store;
use serde::{Deserialize, Serialize};

/// Reserved iroh-blobs tag name for the metadata document.
const META_TAG: &str = ".meta";

/// Current metadata document version.
const META_VERSION: u32 = 1;

/// The metadata document stored as a JSON blob.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaDoc {
    /// Schema version for forward compatibility.
    pub version: u32,
    /// All metadata tags.
    pub tags: Vec<MetaTag>,
}

/// A single metadata tag.
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

impl Default for MetaDoc {
    fn default() -> Self {
        Self {
            version: META_VERSION,
            tags: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Core I/O
// ---------------------------------------------------------------------------

/// Load the metadata document from the store.
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

/// Save the metadata document to the store.
///
/// Serializes to JSON, stores as a blob, and updates the `.meta` tag.
pub async fn save_meta(store: &Store, doc: &MetaDoc) -> Result<()> {
    let json = serde_json::to_vec(doc)?;
    let outcome = store.blobs().add_bytes(json).await?;
    store.tags().set(META_TAG, outcome.hash).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tag CRUD
// ---------------------------------------------------------------------------

/// Get current unix timestamp in seconds.
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Add a new tag to the metadata document.
///
/// Does not check for duplicates — multiple tags with the same subject+key
/// are allowed by design.
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

/// Remove all tags matching subject+key. Returns the removed tags.
pub fn remove_tags(doc: &mut MetaDoc, subject: &str, key: &str) -> Vec<MetaTag> {
    let mut removed = Vec::new();
    let mut i = 0;
    while i < doc.tags.len() {
        if doc.tags[i].subject == subject && doc.tags[i].key == key {
            removed.push(doc.tags.remove(i));
        } else {
            i += 1;
        }
    }
    removed
}

/// Remove all tags for a subject. Returns the removed tags.
pub fn remove_all_tags(doc: &mut MetaDoc, subject: &str) -> Vec<MetaTag> {
    let mut removed = Vec::new();
    let mut i = 0;
    while i < doc.tags.len() {
        if doc.tags[i].subject == subject {
            removed.push(doc.tags.remove(i));
        } else {
            i += 1;
        }
    }
    removed
}

/// Get all tags for a subject.
pub fn get_tags<'a>(doc: &'a MetaDoc, subject: &str) -> Vec<&'a MetaTag> {
    doc.tags.iter().filter(|t| t.subject == subject).collect()
}

/// Get all tags for a subject with a specific key.
pub fn get_by_key<'a>(doc: &'a MetaDoc, subject: &str, key: &str) -> Vec<&'a MetaTag> {
    doc.tags
        .iter()
        .filter(|t| t.subject == subject && t.key == key)
        .collect()
}

/// Find all subjects that have a tag with the given key (and optionally value).
pub fn find_subjects<'a>(doc: &'a MetaDoc, key: &str, value: Option<&str>) -> Vec<&'a str> {
    let mut subjects: Vec<&str> = doc
        .tags
        .iter()
        .filter(|t| {
            t.key == key
                && match value {
                    Some(v) => t.value.as_deref() == Some(v),
                    None => true,
                }
        })
        .map(|t| t.subject.as_str())
        .collect();
    subjects.sort_unstable();
    subjects.dedup();
    subjects
}

/// Set a unique tag — removes any existing tags with the same subject+key,
/// then adds the new one. Use for semantics like "created" or "modified"
/// where only one value makes sense.
pub fn set_unique_tag(
    doc: &mut MetaDoc,
    subject: &str,
    key: &str,
    value: Option<&str>,
    link: Option<MetaLink>,
) {
    remove_tags(doc, subject, key);
    add_tag(doc, subject, key, value, link);
}

// ---------------------------------------------------------------------------
// Convenience helpers for file operations
// ---------------------------------------------------------------------------

/// Set the "created" metadata tag for a file. Only sets if no "created" tag
/// already exists for this subject.
pub fn set_created(doc: &mut MetaDoc, name: &str) {
    let existing = get_by_key(doc, name, "created");
    if existing.is_empty() {
        let now = now_unix();
        set_unique_tag(doc, name, "created", Some(&now.to_string()), None);
    }
}

/// Set the "modified" metadata tag for a file (always replaces).
pub fn set_modified(doc: &mut MetaDoc, name: &str) {
    let now = now_unix();
    set_unique_tag(doc, name, "modified", Some(&now.to_string()), None);
}

/// Transfer all metadata tags from one subject to another (for rename).
///
/// Preserves original timestamps. Updates the modified time.
pub fn transfer_tags(doc: &mut MetaDoc, old_subject: &str, new_subject: &str) {
    for tag in &mut doc.tags {
        if tag.subject == old_subject {
            new_subject.clone_into(&mut tag.subject);
        }
        // Also update any links that point to the old name
        if let Some(MetaLink::Name(ref mut name)) = tag.link
            && name.as_str() == old_subject
        {
            new_subject.clone_into(name);
        }
    }
    set_modified(doc, new_subject);
}

/// Add an "archived" metadata tag linking the archive copy back to the original.
pub fn add_archive_tag(
    doc: &mut MetaDoc,
    original_name: &str,
    archive_tag_name: &str,
    hash: &str,
    operation: &str,
) {
    let now = now_unix();
    doc.tags.push(MetaTag {
        subject: original_name.to_owned(),
        key: "archived".to_owned(),
        value: Some(operation.to_owned()),
        link: Some(MetaLink::Hash(hash.to_owned())),
        created_at: now,
        modified_at: now,
    });
    // Also tag the archive entry itself so we can look up its parent
    doc.tags.push(MetaTag {
        subject: archive_tag_name.to_owned(),
        key: "archive_of".to_owned(),
        value: Some(operation.to_owned()),
        link: Some(MetaLink::Name(original_name.to_owned())),
        created_at: now,
        modified_at: now,
    });
}

/// Get the created timestamp for a file, if available.
pub fn get_created_time(doc: &MetaDoc, name: &str) -> Option<u64> {
    get_by_key(doc, name, "created")
        .first()
        .and_then(|t| t.value.as_deref())
        .and_then(|v| v.parse::<u64>().ok())
}

/// Get the modified timestamp for a file, if available.
pub fn get_modified_time(doc: &MetaDoc, name: &str) -> Option<u64> {
    get_by_key(doc, name, "modified")
        .first()
        .and_then(|t| t.value.as_deref())
        .and_then(|v| v.parse::<u64>().ok())
}

/// Check if a tag name is internal (starts with `.`).
///
/// Internal tags like `.meta` should be hidden from the file list.
pub fn is_internal_tag(name: &str) -> bool {
    name.starts_with('.')
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_meta_doc_default() {
        let doc = MetaDoc::default();
        assert_eq!(doc.version, 1);
        assert!(doc.tags.is_empty());
    }

    #[test]
    fn test_add_tag_key_only() {
        let mut doc = MetaDoc::default();
        add_tag(&mut doc, "README.md", "pinned", None, None);
        assert_eq!(doc.tags.len(), 1);
        assert_eq!(doc.tags[0].subject, "README.md");
        assert_eq!(doc.tags[0].key, "pinned");
        assert!(doc.tags[0].value.is_none());
        assert!(doc.tags[0].link.is_none());
        assert!(doc.tags[0].created_at > 0);
    }

    #[test]
    fn test_add_tag_key_value() {
        let mut doc = MetaDoc::default();
        add_tag(&mut doc, "README.md", "category", Some("docs"), None);
        assert_eq!(doc.tags[0].value.as_deref(), Some("docs"));
    }

    #[test]
    fn test_add_tag_with_link() {
        let mut doc = MetaDoc::default();
        add_tag(
            &mut doc,
            "README.md",
            "related",
            None,
            Some(MetaLink::Name("CHANGELOG.md".to_owned())),
        );
        assert_eq!(
            doc.tags[0].link,
            Some(MetaLink::Name("CHANGELOG.md".to_owned()))
        );
    }

    #[test]
    fn test_add_tag_with_hash_link() {
        let mut doc = MetaDoc::default();
        add_tag(
            &mut doc,
            "README.md",
            "snapshot",
            Some("v1.0"),
            Some(MetaLink::Hash("abc123".to_owned())),
        );
        assert_eq!(doc.tags[0].link, Some(MetaLink::Hash("abc123".to_owned())));
        assert_eq!(doc.tags[0].value.as_deref(), Some("v1.0"));
    }

    #[test]
    fn test_non_unique_keys() {
        let mut doc = MetaDoc::default();
        add_tag(&mut doc, "README.md", "tag", Some("rust"), None);
        add_tag(&mut doc, "README.md", "tag", Some("cli"), None);
        add_tag(&mut doc, "README.md", "tag", Some("p2p"), None);
        let tags = get_by_key(&doc, "README.md", "tag");
        assert_eq!(tags.len(), 3);
        let values: Vec<&str> = tags.iter().filter_map(|t| t.value.as_deref()).collect();
        assert_eq!(values, vec!["rust", "cli", "p2p"]);
    }

    #[test]
    fn test_remove_tags() {
        let mut doc = MetaDoc::default();
        add_tag(&mut doc, "README.md", "tag", Some("rust"), None);
        add_tag(&mut doc, "README.md", "tag", Some("cli"), None);
        add_tag(&mut doc, "README.md", "category", Some("docs"), None);
        let removed = remove_tags(&mut doc, "README.md", "tag");
        assert_eq!(removed.len(), 2);
        assert_eq!(doc.tags.len(), 1);
        assert_eq!(doc.tags[0].key, "category");
    }

    #[test]
    fn test_remove_all_tags() {
        let mut doc = MetaDoc::default();
        add_tag(&mut doc, "README.md", "tag", Some("rust"), None);
        add_tag(&mut doc, "README.md", "category", Some("docs"), None);
        add_tag(&mut doc, "other.txt", "tag", Some("text"), None);
        let removed = remove_all_tags(&mut doc, "README.md");
        assert_eq!(removed.len(), 2);
        assert_eq!(doc.tags.len(), 1);
        assert_eq!(doc.tags[0].subject, "other.txt");
    }

    #[test]
    fn test_get_tags() {
        let mut doc = MetaDoc::default();
        add_tag(&mut doc, "README.md", "tag", Some("rust"), None);
        add_tag(&mut doc, "other.txt", "tag", Some("text"), None);
        let tags = get_tags(&doc, "README.md");
        assert_eq!(tags.len(), 1);
    }

    #[test]
    fn test_find_subjects() {
        let mut doc = MetaDoc::default();
        add_tag(&mut doc, "README.md", "category", Some("docs"), None);
        add_tag(&mut doc, "main.rs", "category", Some("code"), None);
        add_tag(&mut doc, "notes.md", "category", Some("docs"), None);

        let all_categorized = find_subjects(&doc, "category", None);
        assert_eq!(all_categorized.len(), 3);

        let docs = find_subjects(&doc, "category", Some("docs"));
        assert_eq!(docs.len(), 2);
        assert!(docs.contains(&"README.md"));
        assert!(docs.contains(&"notes.md"));
    }

    #[test]
    fn test_set_unique_tag() {
        let mut doc = MetaDoc::default();
        set_unique_tag(&mut doc, "README.md", "modified", Some("100"), None);
        set_unique_tag(&mut doc, "README.md", "modified", Some("200"), None);
        let tags = get_by_key(&doc, "README.md", "modified");
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].value.as_deref(), Some("200"));
    }

    #[test]
    fn test_set_created_only_once() {
        let mut doc = MetaDoc::default();
        set_created(&mut doc, "README.md");
        let first = get_by_key(&doc, "README.md", "created")[0].value.clone();
        // Second call should not overwrite
        set_created(&mut doc, "README.md");
        let second = get_by_key(&doc, "README.md", "created")[0].value.clone();
        assert_eq!(first, second);
        assert_eq!(get_by_key(&doc, "README.md", "created").len(), 1);
    }

    #[test]
    fn test_transfer_tags() {
        let mut doc = MetaDoc::default();
        add_tag(&mut doc, "old.md", "created", Some("100"), None);
        add_tag(&mut doc, "old.md", "tag", Some("rust"), None);
        add_tag(
            &mut doc,
            "link.md",
            "related",
            None,
            Some(MetaLink::Name("old.md".to_owned())),
        );

        transfer_tags(&mut doc, "old.md", "new.md");

        // Tags moved
        assert!(get_tags(&doc, "old.md").is_empty());
        assert_eq!(get_by_key(&doc, "new.md", "created").len(), 1);
        assert_eq!(get_by_key(&doc, "new.md", "tag").len(), 1);
        // Modified added
        assert_eq!(get_by_key(&doc, "new.md", "modified").len(), 1);
        // Link updated
        let link_tags = get_tags(&doc, "link.md");
        assert_eq!(link_tags[0].link, Some(MetaLink::Name("new.md".to_owned())));
    }

    #[test]
    fn test_archive_tag() {
        let mut doc = MetaDoc::default();
        add_archive_tag(
            &mut doc,
            "README.md",
            "README.md.archive.12345",
            "hash1",
            "save",
        );
        let archived = get_by_key(&doc, "README.md", "archived");
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].value.as_deref(), Some("save"));
        assert_eq!(archived[0].link, Some(MetaLink::Hash("hash1".to_owned())));
        let archive_of = get_by_key(&doc, "README.md.archive.12345", "archive_of");
        assert_eq!(archive_of.len(), 1);
        assert_eq!(
            archive_of[0].link,
            Some(MetaLink::Name("README.md".to_owned()))
        );
    }

    #[test]
    fn test_created_modified_times() {
        let mut doc = MetaDoc::default();
        set_created(&mut doc, "test.md");
        set_modified(&mut doc, "test.md");
        assert!(get_created_time(&doc, "test.md").is_some());
        assert!(get_modified_time(&doc, "test.md").is_some());
        assert!(get_created_time(&doc, "missing.md").is_none());
    }

    #[test]
    fn test_is_internal_tag() {
        assert!(is_internal_tag(".meta"));
        assert!(is_internal_tag(".config"));
        assert!(!is_internal_tag("README.md"));
        assert!(!is_internal_tag("auto-2026-03-12T06:42:30.015Z"));
    }

    #[test]
    fn test_serialization_roundtrip() {
        let mut doc = MetaDoc::default();
        add_tag(&mut doc, "file.md", "key", Some("val"), None);
        add_tag(
            &mut doc,
            "file.md",
            "link",
            None,
            Some(MetaLink::Hash("abc".to_owned())),
        );

        let json = serde_json::to_string(&doc).unwrap();
        let parsed: MetaDoc = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.tags.len(), 2);
        assert_eq!(parsed.tags[0].key, "key");
        assert_eq!(parsed.tags[1].link, Some(MetaLink::Hash("abc".to_owned())));
    }
}
