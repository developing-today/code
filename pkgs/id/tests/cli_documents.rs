//! End-to-end tests for `id doc` (P6.7c): publish, events, frontier, and history.
//!
//! Each node is a data directory with its own key. Every node in a test points at one event store
//! with `--event-store`, which stands in for the events sync will carry. The tests check what the
//! store holds, read through the library, and not only the text the CLI prints.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::str::FromStr;

use id::chain::{ChainExt, sign};
use id::documents::{DOCUMENT_KIND, DocumentRecord, REMOVAL_KIND};
use id::event_store::{EventStore, Ingest};
use id::tag_claims::SET_KIND;
use id::waiting::WaitingLimits;
use p2panda_core::{Hash, SigningKey};
use tempfile::TempDir;

/// One node: its data directory. The event store is shared by every node of a test.
struct Node {
    dir: PathBuf,
    store: PathBuf,
}

impl Node {
    /// Runs `id` as this node, with the shared event store.
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_id"))
            .arg("--data-dir")
            .arg(&self.dir)
            .arg("--event-store")
            .arg(&self.store)
            .args(args)
            .output()
            .expect("run id")
    }

    /// Runs a command that must succeed, and returns its stdout.
    fn ok(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// This node's account key, as `id id` prints it.
    fn key(&self) -> String {
        self.ok(&["id"]).trim().to_owned()
    }

    /// This node's signing key, read from its key file, as the store's events are signed.
    fn signing_key(&self) -> SigningKey {
        let bytes: [u8; 32] = std::fs::read(self.dir.join(".iroh-key"))
            .unwrap()
            .try_into()
            .unwrap();
        SigningKey::from_bytes(&bytes)
    }
}

fn node(root: &Path, name: &str, store: &Path) -> Node {
    Node {
        dir: root.join(name),
        store: store.to_path_buf(),
    }
}

/// The value of `key` in a line of `key=value` tokens.
fn field(line: &str, key: &str) -> String {
    line.split_whitespace()
        .find_map(|token| token.strip_prefix(&format!("{key}=")))
        .unwrap_or_else(|| panic!("no {key}= in {line:?}"))
        .to_owned()
}

fn hash(text: &str) -> Hash {
    Hash::from_str(text).expect("64 hex characters")
}

fn key_bytes(text: &str) -> [u8; 32] {
    *hash(text).as_bytes()
}

fn open(store: &Path) -> EventStore {
    EventStore::open(store, WaitingLimits::default(), 0).unwrap()
}

fn documents(store: &Path) -> Vec<DocumentRecord> {
    open(store).documents().unwrap()
}

/// Signs `ext` with `key`, after the key's last applied event, and ingests it, as sync would.
/// It returns the new event's ID. The store must accept it.
fn append(store: &Path, key: &SigningKey, ext: ChainExt) -> Hash {
    let mut store = open(store);
    let author = *key.verifying_key().as_bytes();
    let chain = store.chain(&author).unwrap().unwrap_or_default();
    let event = sign(key, chain.next_seq, chain.head, None, ext).unwrap();
    assert!(matches!(
        store.ingest(&event.bytes, 0).unwrap(),
        Ingest::Accepted { .. }
    ));
    event.id
}

/// Writes `body` to a file under `root` and returns its path.
fn file(root: &Path, name: &str, body: &[u8]) -> PathBuf {
    let path = root.join(name);
    std::fs::write(&path, body).unwrap();
    path
}

const BODY: &[u8] = b"# Minutes\n\nThe plan is agreed.";

/// Publishes `path` as `text/markdown` through `owner`, with `extra` arguments, and returns the
/// line it prints.
fn publish(owner: &Node, path: &Path, extra: &[&str]) -> String {
    let path = path.to_str().unwrap();
    let mut args = vec!["doc", "publish", path, "--media-type", "text/markdown"];
    args.extend_from_slice(extra);
    owner.ok(&args)
}

#[test]
fn test_publish_records_a_signed_document_event_and_prints_its_id() {
    let root = TempDir::new().unwrap();
    let store = root.path().join("events");
    let owner = node(root.path(), "owner", &store);
    let path = file(root.path(), "notes.md", BODY);

    let line = publish(&owner, &path, &[]);
    assert!(line.starts_with("publish "), "{line:?}");
    let doc = field(&line, "document");

    let records = documents(&store);
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.id, hash(&doc));
    assert_eq!(record.author, key_bytes(&owner.key()));
    assert_eq!(record.document.content, Hash::digest(BODY));
    assert_eq!(record.document.size, u64::try_from(BODY.len()).unwrap());
    assert_eq!(record.document.media_type, "text/markdown");
    assert_eq!(field(&line, "content"), Hash::digest(BODY).to_hex());
}

#[test]
fn test_publish_with_tags_sets_a_claim_per_tag() {
    let root = TempDir::new().unwrap();
    let store = root.path().join("events");
    let owner = node(root.path(), "owner", &store);
    let path = file(root.path(), "notes.md", BODY);

    let line = publish(&owner, &path, &["--tag", "topic=plans", "--tag", "draft"]);
    let doc = hash(&field(&line, "document"));

    let claims = open(&store).tag_claims().unwrap();
    let mut on_doc: Vec<(String, Option<String>)> = claims
        .iter()
        .filter(|claim| claim.subject == doc)
        .map(|claim| (claim.name.clone(), claim.value.clone()))
        .collect();
    on_doc.sort();
    assert_eq!(
        on_doc,
        vec![
            ("draft".to_owned(), None),
            ("topic".to_owned(), Some("plans".to_owned())),
        ]
    );
}

#[test]
fn test_events_lists_the_tags_and_a_removal_on_a_document() {
    let root = TempDir::new().unwrap();
    let store = root.path().join("events");
    let owner = node(root.path(), "owner", &store);
    let path = file(root.path(), "notes.md", BODY);
    let doc = field(
        &publish(&owner, &path, &["--tag", "topic=plans"]),
        "document",
    );

    append(
        &store,
        &owner.signing_key(),
        ChainExt {
            kind: REMOVAL_KIND.to_owned(),
            group: None,
            after: Vec::new(),
            tag: None,
            document: None,
            removes: Some(hash(&doc)),
            note: None,
        },
    );

    let listed = owner.ok(&["doc", "events", &doc]);
    let lines: Vec<&str> = listed.lines().collect();
    assert_eq!(lines.len(), 2, "{listed}");
    assert!(
        lines[0].starts_with(&format!("kind={SET_KIND} ")),
        "{listed}"
    );
    assert!(
        lines[1].starts_with(&format!("kind={REMOVAL_KIND} ")),
        "{listed}"
    );
    assert!(
        !listed.contains(DOCUMENT_KIND),
        "the put is not an event on the document: {listed}"
    );
}

#[test]
fn test_frontier_lists_each_authors_head() {
    let root = TempDir::new().unwrap();
    let store = root.path().join("events");
    let owner = node(root.path(), "owner", &store);
    let ann = node(root.path(), "ann", &store);
    let path = file(root.path(), "notes.md", BODY);
    publish(&owner, &path, &[]);
    publish(&ann, &path, &[]);

    let listed = owner.ok(&["doc", "frontier"]);
    let lines: Vec<&str> = listed.lines().collect();
    assert_eq!(lines.len(), 2, "{listed}");

    let saved = open(&store);
    for line in lines {
        let author = key_bytes(&field(line, "author"));
        let head = saved.chain(&author).unwrap().unwrap().head.unwrap();
        assert_eq!(field(line, "event"), head.to_hex(), "{line}");
    }
}

#[test]
fn test_history_lists_the_document_and_its_tags_in_causal_order() {
    let root = TempDir::new().unwrap();
    let store = root.path().join("events");
    let owner = node(root.path(), "owner", &store);
    let ann = node(root.path(), "ann", &store);
    let path = file(root.path(), "notes.md", BODY);
    let doc = field(&publish(&owner, &path, &[]), "document");
    let owner_set = field(
        &owner.ok(&["claim", "set", &doc, "topic", "plans"]),
        "event",
    );
    // Ann names the owner's tag in her event's causal past, because she saw it first.
    let ann_set = field(&ann.ok(&["claim", "set", &doc, "status", "ok"]), "event");

    let listed = owner.ok(&["doc", "history", &doc]);
    let ids: Vec<String> = listed.lines().map(|line| field(line, "event")).collect();
    assert_eq!(ids, vec![doc, owner_set, ann_set], "{listed}");
    assert!(
        listed
            .lines()
            .next()
            .unwrap()
            .starts_with(&format!("kind={DOCUMENT_KIND} "))
    );
}

#[test]
fn test_json_forms_print_one_object_for_publish_and_arrays_for_lists() {
    let root = TempDir::new().unwrap();
    let store = root.path().join("events");
    let owner = node(root.path(), "owner", &store);
    let path = file(root.path(), "notes.md", BODY);

    let published: serde_json::Value =
        serde_json::from_str(publish(&owner, &path, &["--json"]).trim()).unwrap();
    assert_eq!(published["command"], "publish");
    let doc = published["document"].as_str().unwrap().to_owned();
    assert_eq!(doc.len(), 64);

    let events: serde_json::Value =
        serde_json::from_str(owner.ok(&["doc", "events", &doc, "--json"]).trim()).unwrap();
    assert!(events.as_array().unwrap().is_empty());
    let frontier: serde_json::Value =
        serde_json::from_str(owner.ok(&["doc", "frontier", "--json"]).trim()).unwrap();
    assert_eq!(frontier.as_array().unwrap().len(), 1);
}

#[test]
fn test_publish_refuses_an_empty_media_type_and_a_missing_file() {
    let root = TempDir::new().unwrap();
    let store = root.path().join("events");
    let owner = node(root.path(), "owner", &store);
    let path = file(root.path(), "notes.md", BODY);

    let empty = owner.run(&["doc", "publish", path.to_str().unwrap(), "--media-type", ""]);
    assert!(!empty.status.success());
    let missing = owner.run(&[
        "doc",
        "publish",
        root.path().join("absent.md").to_str().unwrap(),
        "--media-type",
        "text/markdown",
    ]);
    assert!(!missing.status.success());
    assert!(
        documents(&store).is_empty(),
        "a refused publish records nothing"
    );
}

#[test]
fn test_a_subject_that_is_not_a_document_or_not_hex_is_refused() {
    let root = TempDir::new().unwrap();
    let store = root.path().join("events");
    let owner = node(root.path(), "owner", &store);

    let not_hex = owner.run(&["doc", "events", "not-hex"]);
    assert!(!not_hex.status.success());
    assert!(String::from_utf8_lossy(&not_hex.stderr).contains("not 64 hex characters"));
    let unknown = "0".repeat(64);
    let not_a_document = owner.run(&["doc", "history", &unknown]);
    assert!(!not_a_document.status.success());
    assert!(String::from_utf8_lossy(&not_a_document.stderr).contains("is not a document"));
}
