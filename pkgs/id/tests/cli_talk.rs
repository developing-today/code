//! End-to-end tests for `id talk` (#5834 slice 2, #14270): post on and list an object's talk page.
//!
//! Every node in a test points at one event store with `--event-store`, which stands in for the
//! events sync will carry. The tests check the store as well as the printed lines: a post is a
//! `talk.post.v1` event signed by the node that posted it.

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

    /// Runs a command that must fail, and returns its stderr.
    fn refused(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(!output.status.success(), "{args:?} succeeded");
        String::from_utf8_lossy(&output.stderr).into_owned()
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

/// The value of `key` in a line of `key=value` tokens.
fn required(line: &str, key: &str) -> String {
    line.split_whitespace()
        .find_map(|token| token.strip_prefix(&format!("{key}=")))
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("no {key}= in {line:?}"))
}

fn has(line: &str, key: &str) -> bool {
    line.split_whitespace()
        .any(|token| token.starts_with(&format!("{key}=")))
}

/// Publishes a document as `node` and returns its ID, so the talk page has an object.
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

/// The line of `listed` whose `post=` is `post`.
fn line_of<'a>(listed: &'a str, post: &str) -> &'a str {
    listed
        .lines()
        .find(|line| required(line, "post") == post)
        .unwrap_or_else(|| panic!("no post {post} in {listed}"))
}

#[test]
fn test_talk_post_then_list_attributes_the_post_to_its_signer() {
    let root = TempDir::new().unwrap();
    let store = root.path().join("events");
    let owner = node(root.path(), "owner", &store);
    let doc = publish(&owner, "notes.md");

    let posted = owner.ok(&["talk", "post", &doc, "needs a source"]);
    assert!(posted.starts_with("post "), "{posted:?}");
    assert_eq!(required(&posted, "object"), doc);
    assert_eq!(required(&posted, "author"), owner.key());
    let post = required(&posted, "post");
    assert!(post.starts_with("content/"), "{post}");
    let event = required(&posted, "event");

    let listed = owner.ok(&["talk", "list", &doc]);
    assert_eq!(listed.lines().count(), 1, "{listed}");
    let line = line_of(&listed, &post);
    assert_eq!(required(line, "event"), event);
    assert_eq!(required(line, "author"), owner.key());
    assert!(!has(line, "superseded_by"), "{line}");

    // The store holds a talk event signed by the posting node.
    let stored = EventStore::open(&store, WaitingLimits::default(), 0)
        .unwrap()
        .event_by_id(&Hash::from_str(&event).unwrap())
        .unwrap()
        .expect("the store has the post's event");
    assert_eq!(stored.header.extensions.kind, "talk.post.v1");
    assert_eq!(
        Hash::from_bytes(*stored.header.verifying_key.as_bytes()).to_hex(),
        owner.key()
    );
}

#[test]
fn test_a_correction_supersedes_its_authors_post_and_the_original_stays_listed() {
    let root = TempDir::new().unwrap();
    let store = root.path().join("events");
    let owner = node(root.path(), "owner", &store);
    let reader = node(root.path(), "reader", &store);
    let doc = publish(&owner, "notes.md");

    let original = required(&owner.ok(&["talk", "post", &doc, "teh wording"]), "post");
    let reply = reader.ok(&["talk", "post", &doc, "agreed", "--reply-to", &original]);
    assert_eq!(required(&reply, "reply_to"), original);
    assert_eq!(required(&reply, "author"), reader.key());

    // Another author's post cannot be superseded.
    reader.refused(&["talk", "post", &doc, "mine now", "--supersedes", &original]);

    let correction = required(
        &owner.ok(&[
            "talk",
            "post",
            &doc,
            "the wording",
            "--supersedes",
            &original,
        ]),
        "post",
    );
    let listed = owner.ok(&["talk", "list", &doc]);
    assert_eq!(listed.lines().count(), 3, "{listed}");
    let kept = line_of(&listed, &original);
    assert_eq!(required(kept, "superseded_by"), correction);
    assert_eq!(
        required(line_of(&listed, &correction), "supersedes"),
        original
    );

    let json: serde_json::Value =
        serde_json::from_str(&owner.ok(&["talk", "list", &doc, "--json"])).unwrap();
    let rows = json.as_array().expect("one JSON array");
    assert_eq!(rows.len(), 3);
    let original_row = rows
        .iter()
        .find(|row| row["post"] == original.as_str())
        .unwrap();
    assert_eq!(original_row["body"], "teh wording");
    assert_eq!(original_row["superseded_by"], correction.as_str());
}

#[test]
fn test_an_object_this_node_cannot_see_is_refused() {
    let root = TempDir::new().unwrap();
    let store = root.path().join("events");
    let owner = node(root.path(), "owner", &store);
    let unknown = Hash::digest(b"never stored").to_hex();

    let stderr = owner.refused(&["talk", "post", &unknown, "hello"]);
    assert!(stderr.contains("not visible"), "{stderr}");
    let stderr = owner.refused(&["talk", "list", &unknown]);
    assert!(stderr.contains("not visible"), "{stderr}");
}
