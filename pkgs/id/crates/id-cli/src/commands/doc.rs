//! `id doc`: publish a file as a document, fetch a document's bytes, and list a
//! document's events, frontier and history (P6.7c): parse the arguments, act on the event store,
//! and print.
//!
//! The rules are the library's ([`id_events::local_events`]). This module only turns arguments into
//! calls and results into lines or JSON. The store is the stand-in for sync, as the group log is.
//!
//! Publish stores the file's bytes in the blob store under their hash before recording the
//! events, through the same blob path other writes use, so `content` can fetch them back by the
//! document's event ID. Serving a running node goes through it (ephemeral store plus a push,
//! as `put` does) instead of opening the persistent store the server holds.
//!
//! # Output
//!
//! `publish` prints one line, `publish document=<ID> content=<HASH> size=<N> media_type=<TYPE>`,
//! then one `set` line for each `--tag`. `content` exports the document's bytes to the output
//! path, or to stdout with `-`. The events, frontier and history print one line per row,
//! with no command word, and `kind=` (or the head's fields) first. With `--json` each action prints
//! one object with a `command` key, and each list prints one array.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use iroh_blobs::{
    ALPN as BLOBS_ALPN, Hash as BlobHash,
    protocol::{ChunkRanges, ChunkRangesSeq, PushRequest},
};
use p2panda_core::{Hash, SigningKey};
use serde_json::{Value, json};

use crate::cli::DocCommand;
use crate::commands::claim::print_set;
use crate::commands::get::cmd_gethash;
use crate::commands::group::{hex, object, tokens};
use crate::{
    KEY_FILE, create_local_client_endpoint, get_serve_info, load_or_create_keypair, open_store,
};
use id_events::local_events::{DIR_NAME, Entry, LocalEvents};
use id_groups::groups::log::hash_from_hex;

/// Opens the event store that the command reads and appends to, signed as this node. The store is
/// `--event-store`, or the default directory in the data directory.
///
/// # Errors
///
/// Fails if the node key cannot be read or created, or the store cannot be opened.
pub(super) async fn open_events(path: Option<PathBuf>) -> Result<LocalEvents> {
    let secret = load_or_create_keypair(KEY_FILE).await?;
    let dir = path.unwrap_or_else(|| PathBuf::from(DIR_NAME));
    LocalEvents::open(&dir, SigningKey::from_bytes(&secret.to_bytes()))
}

/// Stores `content` as a raw blob through the same blob path other writes use, and returns
/// its hash: into the persistent store directly, or (when a serve holds that store) into an
/// ephemeral store pushed to the serve, as `put` does.
/// The caller records the document event afterwards, which is what registers the
/// document-to-hash reference the blob gate reads (`EventStore::document_references`).
///
/// # Errors
///
/// Fails if the store cannot be opened, the bytes cannot be added, or (with a serve running)
/// the blob cannot be pushed to it.
async fn store_publish_bytes(content: &[u8]) -> Result<BlobHash> {
    if let Some(serve_info) = get_serve_info().await {
        let store = open_store(true).await?;
        let store_handle = store.as_store();
        let added = store_handle.blobs().add_bytes(content.to_vec()).await?;
        let hash = added.hash;
        let (endpoint, endpoint_addr) = create_local_client_endpoint(&serve_info).await?;
        let blobs_conn =
            id_core::connectivity::connect(&endpoint, endpoint_addr.clone(), BLOBS_ALPN).await?;
        let push_request =
            PushRequest::new(hash, ChunkRangesSeq::from_ranges([ChunkRanges::all()]));
        store_handle
            .remote()
            .execute_push(blobs_conn.clone(), push_request)
            .await?;
        blobs_conn.close(0u32.into(), b"done");
        endpoint.close().await;
        store.shutdown().await?;
        Ok(hash)
    } else {
        let store = open_store(false).await?;
        let store_handle = store.as_store();
        let added = store_handle.blobs().add_bytes(content.to_vec()).await?;
        let hash = added.hash;
        store.shutdown().await?;
        Ok(hash)
    }
}

/// The blob hash the document `document_hex` names: its `document.put` event's content hash.
/// This is the reference the publish step stored the bytes under.
///
/// # Errors
///
/// Fails if `document_hex` is not a document's event ID, or the store cannot be read.
fn document_content_hash(events: &LocalEvents, document_hex: &str) -> Result<BlobHash> {
    let document = hash_from_hex(document_hex)?;
    let history = events.history(document)?;
    history
        .iter()
        .find_map(|entry| match entry {
            Entry::Document { document, .. } => Some(document.blob_hash()),
            _ => None,
        })
        .with_context(|| format!("{document_hex} names no document.put event"))
}

/// Prints one action: a line `command key=value ...`, or one JSON object with `command` first.
pub(super) fn print_action(json: bool, command: &str, fields: Vec<(&str, Value)>) {
    if json {
        let mut with_command = vec![("command", json!(command))];
        with_command.extend(fields);
        println!("{}", object(&with_command));
    } else {
        println!("{command} {}", tokens(&fields));
    }
}

/// Prints rows: one line each, or one JSON array with `--json`.
pub(super) fn print_rows(json: bool, rows: &[Vec<(&str, Value)>]) {
    if json {
        let array: Vec<Value> = rows.iter().map(|row| object(row)).collect();
        println!("{}", Value::Array(array));
    } else {
        for row in rows {
            println!("{}", tokens(row));
        }
    }
}

/// Splits a `--tag` into its name and optional value: `name=value`, or `name` for a bare tag.
fn parse_tag(text: &str) -> (String, Option<String>) {
    match text.split_once('=') {
        Some((name, value)) => (name.to_owned(), Some(value.to_owned())),
        None => (text.to_owned(), None),
    }
}

/// The fields of one event in a document's history, as the CLI lists it.
fn entry_fields(entry: &Entry) -> Vec<(&'static str, Value)> {
    let mut fields = vec![
        ("kind", json!(entry.kind())),
        ("event", json!(entry.id().to_hex())),
        ("author", json!(hex(&entry.author()))),
    ];
    match entry {
        Entry::Document { document, .. } => fields.extend([
            ("content", json!(document.content.to_hex())),
            ("size", json!(document.size)),
            ("media_type", json!(document.media_type)),
        ]),
        Entry::Set {
            subject,
            name,
            value,
            ..
        } => {
            fields.push(("subject", json!(subject.to_hex())));
            fields.push(("name", json!(name)));
            if let Some(value) = value {
                fields.push(("value", json!(value)));
            }
        }
        Entry::Clear { claims, .. } => {
            let named: Vec<String> = claims.iter().map(Hash::to_hex).collect();
            fields.push(("claims", json!(named.join(","))));
        }
        Entry::Remove { document, .. } => fields.push(("document", json!(document.to_hex()))),
        Entry::NoteCreate { content, .. } => fields.extend([
            ("content", json!(content.content.to_hex())),
            ("size", json!(content.size)),
            ("media_type", json!(content.media_type)),
        ]),
        Entry::NoteRevise {
            note,
            content,
            parents,
            ..
        } => {
            fields.push(("note", json!(note.to_hex())));
            fields.push(("content", json!(content.content.to_hex())));
            fields.push(("size", json!(content.size)));
            fields.push(("media_type", json!(content.media_type)));
            let named: Vec<String> = parents.iter().map(Hash::to_hex).collect();
            fields.push(("parents", json!(named.join(","))));
        }
    }
    fields
}

/// Runs an `id doc` command.
///
/// # Errors
///
/// Fails if an argument is malformed, the file cannot be read, the store cannot be read or written,
/// or the library refuses the action. A refused publish records nothing.
pub async fn cmd_doc(path: Option<PathBuf>, json: bool, command: DocCommand) -> Result<()> {
    let mut events = open_events(path).await?;
    match command {
        DocCommand::Publish {
            file,
            media_type,
            tags,
        } => {
            let content =
                std::fs::read(&file).with_context(|| format!("read {}", file.display()))?;
            let tags: Vec<(String, Option<String>)> =
                tags.iter().map(|text| parse_tag(text)).collect();
            let stored = store_publish_bytes(&content).await?;
            let published = events.publish(&media_type, &content, &tags)?;
            if stored != published.document.blob_hash() {
                bail!(
                    "stored blob {stored} does not match the published content hash {}",
                    published.document.blob_hash()
                );
            }
            let fields = vec![
                ("document", json!(published.id.to_hex())),
                ("content", json!(published.document.content.to_hex())),
                ("size", json!(published.document.size)),
                ("media_type", json!(published.document.media_type)),
            ];
            print_action(json, "publish", fields);
            for (claim, (name, value)) in published.claims.iter().zip(&tags) {
                print_set(json, published.id, name, value.as_deref(), *claim);
            }
        }
        DocCommand::Events { document } => {
            let document = hash_from_hex(&document)?;
            let rows: Vec<Vec<(&str, Value)>> = events
                .events_on(document)?
                .iter()
                .map(entry_fields)
                .collect();
            print_rows(json, &rows);
        }
        DocCommand::History { document } => {
            let document = hash_from_hex(&document)?;
            let rows: Vec<Vec<(&str, Value)>> =
                events.history(document)?.iter().map(entry_fields).collect();
            print_rows(json, &rows);
        }
        DocCommand::Frontier => {
            let rows: Vec<Vec<(&str, Value)>> = events
                .frontier()?
                .iter()
                .map(|head| {
                    vec![
                        ("event", json!(head.event.to_hex())),
                        ("author", json!(hex(&head.author))),
                        ("seq", json!(head.seq)),
                    ]
                })
                .collect();
            print_rows(json, &rows);
        }
        DocCommand::Content { document, output } => {
            let hash = document_content_hash(&events, &document)?;
            cmd_gethash(&hash.to_string(), &output).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use id_core::waiting::AuthorKey;
    use id_store::documents::DocumentRef;

    fn hash(byte: u8) -> Hash {
        Hash::from_bytes([byte; 32])
    }

    fn author(byte: u8) -> AuthorKey {
        [byte; 32]
    }

    #[test]
    fn test_note_revise_listing_row_includes_size_and_media_type() {
        let content = DocumentRef::for_bytes("text/markdown", b"revised").unwrap();
        let entry = Entry::NoteRevise {
            id: hash(11),
            author: author(1),
            note: hash(10),
            content,
            parents: vec![hash(10)],
        };
        let fields = entry_fields(&entry);
        let get = |name: &str| -> Option<&Value> {
            fields
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value)
        };
        // The regression: revise rows omitted the revision's size and media type
        // while Document and NoteCreate rows rendered both (issue #4041).
        assert_eq!(get("size"), Some(&json!(7)));
        assert_eq!(get("media_type"), Some(&json!("text/markdown")));
        // The pre-existing arms are unchanged.
        assert_eq!(get("note"), Some(&json!(hash(10).to_hex())));
        assert_eq!(get("parents"), Some(&json!(hash(10).to_hex())));
    }

    fn test_events() -> (tempfile::TempDir, LocalEvents) {
        let dir = tempfile::TempDir::new().unwrap();
        let events = LocalEvents::open(dir.path(), SigningKey::from_bytes(&[7; 32])).unwrap();
        (dir, events)
    }

    #[test]
    fn test_document_content_hash_resolves_the_published_hash() {
        let (_dir, mut events) = test_events();
        let content = b"round-trip me";
        let published = events.publish("text/plain", content, &[]).unwrap();
        let resolved = document_content_hash(&events, &published.id.to_hex()).unwrap();
        assert_eq!(resolved, published.document.blob_hash());
    }

    #[test]
    fn test_document_content_hash_rejects_unknown_documents() {
        let (_dir, events) = test_events();
        assert!(document_content_hash(&events, &hash(9).to_hex()).is_err());
        assert!(document_content_hash(&events, "not-hex").is_err());
    }

    /// The #1008 pairing: the publish step stores the bytes under the hash the document
    /// event names, so the content is retrievable by that reference and the reference index
    /// the blob gate reads (`EventStore::document_references`) carries it.
    #[tokio::test]
    async fn test_published_bytes_are_retrievable_by_the_document_reference() {
        let (_dir, mut events) = test_events();
        let content = b"published bytes round-trip";
        let published = events.publish("text/plain", content, &[]).unwrap();

        let store = open_store(true).await.unwrap();
        let handle = store.as_store();
        let added = handle.blobs().add_bytes(content.to_vec()).await.unwrap();
        assert_eq!(added.hash, published.document.blob_hash());

        let fetched = handle.blobs().get_bytes(added.hash).await.unwrap();
        assert_eq!(&fetched[..], &content[..]);

        let references = events.document_references().unwrap();
        assert!(
            references
                .iter()
                .any(|(hash, _)| *hash == published.document.blob_hash()),
            "the published document registers its content hash where the gate reads it"
        );
        store.shutdown().await.unwrap();
    }
}
