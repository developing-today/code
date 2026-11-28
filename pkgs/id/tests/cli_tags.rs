//! End-to-end tests for `id claim` (P6.7c): set, clear, list, and search tag claims.
//!
//! A tag is a claim: a `tag.set` event that names a subject (any event ID), a name, and an optional
//! value. `id claim clear` names the live claims it removes. Every node in a test points at one
//! event store with `--event-store`, which stands in for the events sync will carry. The tests
//! check what the store holds, read through the library, and not only the text the CLI prints.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::str::FromStr;

use id::event_store::EventStore;
use id::waiting::WaitingLimits;
use p2panda_core::Hash;
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
}

fn node(root: &Path, name: &str, store: &Path) -> Node {
    Node {
        dir: root.join(name),
        store: store.to_path_buf(),
    }
}

/// The value of `key` in a line of `key=value` tokens, or `None` when the line has no such token.
fn field(line: &str, key: &str) -> Option<String> {
    line.split_whitespace()
        .find_map(|token| token.strip_prefix(&format!("{key}=")))
        .map(str::to_owned)
}

/// The value of `key` in a line that must have it.
fn required(line: &str, key: &str) -> String {
    field(line, key).unwrap_or_else(|| panic!("no {key}= in {line:?}"))
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

/// Publishes a document as `node` and returns its ID, so the tags have a subject.
fn publish(node: &Node, name: &str) -> String {
    let path = node.dir.join(name);
    std::fs::create_dir_all(&node.dir).unwrap();
    std::fs::write(&path, b"# Notes\n").unwrap();
    let line = node.ok(&[
        "doc",
        "publish",
        path.to_str().unwrap(),
        "--media-type",
        "text/markdown",
    ]);
    required(&line, "document")
}

#[test]
fn test_claim_set_then_list_shows_the_live_claim_with_its_signer() {
    let root = TempDir::new().unwrap();
    let store = root.path().join("events");
    let owner = node(root.path(), "owner", &store);
    let doc = publish(&owner, "notes.md");

    let set = owner.ok(&["claim", "set", &doc, "topic", "plans"]);
    assert!(set.starts_with("set "), "{set:?}");
    assert_eq!(required(&set, "subject"), doc);
    assert_eq!(required(&set, "name"), "topic");
    assert_eq!(required(&set, "value"), "plans");
    let claim_event = required(&set, "event");

    let listed = owner.ok(&["claim", "list", &doc]);
    let lines: Vec<&str> = listed.lines().collect();
    assert_eq!(lines.len(), 1, "{listed}");
    assert_eq!(required(lines[0], "event"), claim_event);
    assert_eq!(required(lines[0], "author"), owner.key());

    // The store holds the claim, so the event was signed and verified on ingest.
    let claims = open(&store).tag_claims().unwrap();
    let claim = claims
        .iter()
        .find(|claim| claim.id == hash(&claim_event))
        .expect("the store has the claim");
    assert_eq!(claim.author, key_bytes(&owner.key()));
}

#[test]
fn test_a_bare_tag_has_no_value() {
    let root = TempDir::new().unwrap();
    let store = root.path().join("events");
    let owner = node(root.path(), "owner", &store);
    let doc = publish(&owner, "notes.md");

    let set = owner.ok(&["claim", "set", &doc, "draft"]);
    assert_eq!(field(&set, "value"), None, "{set}");
    let listed = owner.ok(&["claim", "list", &doc]);
    assert_eq!(field(listed.trim(), "name").as_deref(), Some("draft"));
    assert_eq!(field(listed.trim(), "value"), None, "{listed}");
}

#[test]
fn test_claim_clear_removes_the_live_claims_on_the_subject() {
    let root = TempDir::new().unwrap();
    let store = root.path().join("events");
    let owner = node(root.path(), "owner", &store);
    let doc = publish(&owner, "notes.md");
    owner.ok(&["claim", "set", &doc, "topic", "plans"]);

    let cleared = owner.ok(&["claim", "clear", &doc, "topic"]);
    assert!(cleared.starts_with("clear "), "{cleared:?}");
    assert_eq!(required(&cleared, "claims"), "1");
    assert_eq!(owner.ok(&["claim", "list", &doc]), "");
    assert!(
        open(&store)
            .tag_claims()
            .unwrap()
            .iter()
            .all(|claim| claim.subject != hash(&doc))
    );
}

#[test]
fn test_clear_with_no_live_claim_is_an_error() {
    let root = TempDir::new().unwrap();
    let store = root.path().join("events");
    let owner = node(root.path(), "owner", &store);
    let doc = publish(&owner, "notes.md");

    let output = owner.run(&["claim", "clear", &doc, "topic"]);
    assert!(!output.status.success());
    assert!(open(&store).tag_claims().unwrap().is_empty());
}

#[test]
fn test_search_matches_a_name_and_a_whole_value() {
    let root = TempDir::new().unwrap();
    let store = root.path().join("events");
    let owner = node(root.path(), "owner", &store);
    let first = publish(&owner, "a.md");
    let second = publish(&owner, "b.md");
    owner.ok(&["claim", "set", &first, "topic", "plans"]);
    owner.ok(&["claim", "set", &second, "topic", "notes"]);

    let exact = owner.ok(&["claim", "search", "topic", "plans"]);
    let lines: Vec<&str> = exact.lines().collect();
    assert_eq!(lines.len(), 1, "{exact}");
    assert_eq!(required(lines[0], "subject"), first);

    // A value matches as a whole field, so a longer value does not match.
    assert_eq!(owner.ok(&["claim", "search", "topic", "plan"]), "");

    let any = owner.ok(&["claim", "search", "topic"]);
    assert_eq!(any.lines().count(), 2, "{any}");
}

#[test]
fn test_list_and_search_print_one_array_element_per_claim_with_json() {
    let root = TempDir::new().unwrap();
    let store = root.path().join("events");
    let owner = node(root.path(), "owner", &store);
    let doc = publish(&owner, "notes.md");

    let set: serde_json::Value = serde_json::from_str(
        owner
            .ok(&["claim", "set", &doc, "topic", "plans", "--json"])
            .trim(),
    )
    .unwrap();
    assert_eq!(set["command"], "set");
    assert_eq!(set["name"], "topic");

    let listed: serde_json::Value =
        serde_json::from_str(owner.ok(&["claim", "list", &doc, "--json"]).trim()).unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    let found: serde_json::Value =
        serde_json::from_str(owner.ok(&["claim", "search", "topic", "--json"]).trim()).unwrap();
    assert_eq!(found.as_array().unwrap().len(), 1);
}

#[test]
fn test_a_name_that_is_empty_or_not_nfc_is_refused() {
    let root = TempDir::new().unwrap();
    let store = root.path().join("events");
    let owner = node(root.path(), "owner", &store);
    let doc = publish(&owner, "notes.md");

    assert!(!owner.run(&["claim", "set", &doc, ""]).status.success());
    // "e" and a combining acute accent is NFD, not NFC.
    assert!(
        !owner
            .run(&["claim", "set", &doc, "e\u{301}"])
            .status
            .success()
    );
    assert!(
        !owner
            .run(&["claim", "set", "not-hex", "topic"])
            .status
            .success()
    );
    assert!(
        open(&store)
            .tag_claims()
            .unwrap()
            .iter()
            .all(|claim| !claim.name.is_empty())
    );
}
