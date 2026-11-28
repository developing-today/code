//! End-to-end tests for the thread steps of `id talk` (#7849): propose a change, resolve a
//! thread, reject, withdraw and accept a proposal, and read the group's threads back.
//!
//! A thread step is written in a conversation group, so each test first puts a group into the
//! event store, as sync would bring it: a create signed by a founder key that is not a node,
//! naming the nodes as members. A proposal's object is a note the founder creates in the same
//! store. Nodes of one test share an event store with `--event-store`, except where a test needs
//! two nodes to act without seeing each other; then each has its own store, and the test moves
//! the events across as a sync session would.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use id::chain::{ChainExt, sign};
use id::event_store::{EventStore, Ingest};
use id::local_events::LocalEvents;
use id::membership::{AuthId, body::encode_body};
use id::waiting::WaitingLimits;
use p2panda_auth::{
    Access,
    group::{GroupAction, GroupMember},
};
use p2panda_core::{Hash, SigningKey};
use tempfile::TempDir;

/// The founder's secret: the key that creates the group and the note, and runs no node.
const FOUNDER: [u8; 32] = [7; 32];

/// The marker `talk threads` puts on an open proposal whose base is no longer a head.
const STALE: &str = "(stale: its base is no longer a head of the object)";

/// One node: its data directory and the event store it uses.
struct Node {
    dir: PathBuf,
    store: PathBuf,
}

impl Node {
    /// Runs `id` as this node, with its event store.
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

    /// This node's view of `group`'s threads.
    fn threads(&self, group: &str) -> String {
        self.ok(&["talk", "threads", group])
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

fn now_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

fn founder() -> SigningKey {
    SigningKey::from_bytes(&FOUNDER)
}

/// A key or ID in the 64 hex characters the thread lines use.
fn key_hex(key: &SigningKey) -> String {
    Hash::from_bytes(*key.verifying_key().as_bytes()).to_hex()
}

/// The group create the founder signs, naming `members` (as `id id` prints them) as writers,
/// and its body.
fn group_create(members: &[String]) -> (id::chain::Event, Vec<u8>) {
    let founder = founder();
    let mut initial_members = vec![(
        GroupMember::Individual(AuthId::Key(*founder.verifying_key().as_bytes())),
        Access::manage(),
    )];
    for member in members {
        let key = *Hash::from_str(member).unwrap().as_bytes();
        initial_members.push((GroupMember::Individual(AuthId::Key(key)), Access::write()));
    }
    let body = encode_body(&GroupAction::Create { initial_members }).unwrap();
    let ext = ChainExt {
        kind: "group.op".to_owned(),
        group: None,
        after: Vec::new(),
        tag: None,
        document: None,
        removes: None,
        note: None,
        link: None,
        checkpoint: None,
        rotation: None,
        chat: None,
        anchor: None,
        thread: None,
    };
    let event = sign(&founder, 0, None, Some(&body), ext).unwrap();
    (event, body)
}

/// Puts the group create into the event store at `store`, and returns the group's ID.
fn seed_group(store: &Path, create: &id::chain::Event, body: &[u8]) -> String {
    std::fs::create_dir_all(store).unwrap();
    let mut events = EventStore::open(store, WaitingLimits::default(), now_ms()).unwrap();
    let ingested = events
        .ingest_with_body(&create.bytes, Some(body), now_ms())
        .unwrap();
    assert!(matches!(ingested, Ingest::Accepted { .. }), "{ingested:?}");
    create.id.to_hex()
}

/// A group whose members are `members`, in the event store at `store`.
fn group(store: &Path, members: &[&Node]) -> String {
    let keys: Vec<String> = members.iter().map(|member| member.key()).collect();
    let (create, body) = group_create(&keys);
    seed_group(store, &create, &body)
}

/// The founder creates a note in the event store at `store`. Returns the note's ID and the
/// note as a thread line names an object, `<note>/<founder-key>`.
fn founder_note(store: &Path) -> (String, String) {
    let mut events = LocalEvents::open(store, founder()).unwrap();
    let note = events.create_note("text/markdown", b"# Draft\n").unwrap();
    (
        note.to_hex(),
        format!("{}/{}", note.to_hex(), key_hex(&founder())),
    )
}

/// A blob link to `bytes`, by `author` (a key as `id id` prints it).
fn blob(bytes: &[u8], author: &str) -> String {
    format!("{}/{author}", Hash::digest(bytes).to_hex())
}

/// A thread root's ID.
fn thread(seed: &[u8]) -> String {
    Hash::digest(seed).to_hex()
}

/// Proposes `content` on `object` at `base` as `node`, and returns the proposal's ID.
fn propose(
    node: &Node,
    group: &str,
    object: &str,
    base: &str,
    content: &str,
    thread: &str,
) -> String {
    let printed = node.ok(&[
        "talk",
        "propose",
        group,
        object,
        "--base",
        base,
        "--content",
        content,
        "--thread",
        thread,
    ]);
    assert!(printed.starts_with("propose "), "{printed:?}");
    assert_eq!(required(&printed, "kind"), "chat.proposal.v1");
    assert_eq!(required(&printed, "group"), group);
    required(&printed, "event")
}

/// The line of `threads` about proposal `proposal`.
fn proposal_line<'a>(threads: &'a str, proposal: &str) -> &'a str {
    threads
        .lines()
        .find(|line| line.starts_with(&format!("proposal {proposal} ")))
        .unwrap_or_else(|| panic!("no proposal {proposal} in {threads}"))
}

/// Moves every applied event of `author` from the store at `from` into the store at `to`, as a
/// sync session would.
fn carry(from: &Path, to: &Path, author: &str) {
    let author = *Hash::from_str(author).unwrap().as_bytes();
    let events = EventStore::open(from, WaitingLimits::default(), now_ms())
        .unwrap()
        .applied_events(&author, None, None, usize::MAX)
        .unwrap();
    assert!(!events.is_empty(), "nothing to carry");
    let mut target = EventStore::open(to, WaitingLimits::default(), now_ms()).unwrap();
    for event in events {
        let ingested = target
            .ingest_with_body(&event.bytes, None, now_ms())
            .unwrap();
        assert!(matches!(ingested, Ingest::Accepted { .. }), "{ingested:?}");
    }
}

#[test]
fn test_a_proposal_is_listed_open_in_the_groups_threads() {
    let root = TempDir::new().unwrap();
    let store = root.path().join("events");
    let owner = node(root.path(), "owner", &store);
    let proposer = node(root.path(), "proposer", &store);
    let outsider = node(root.path(), "outsider", &store);
    let group = group(&store, &[&owner, &proposer]);
    let (note, object) = founder_note(&store);
    let thread = thread(b"the wording thread");
    let content = blob(b"# Draft, reworded\n", &proposer.key());

    let proposal = propose(&proposer, &group, &object, &note, &content, &thread);

    // Every member reads it back open, on a base that is still the note's head.
    let threads = owner.threads(&group);
    assert_eq!(threads.lines().count(), 1, "{threads}");
    assert_eq!(
        proposal_line(&threads, &proposal),
        format!(
            "proposal {proposal} on object {object} at base {note} from thread {thread}, by {}: open",
            proposer.key()
        )
    );
    let json: serde_json::Value =
        serde_json::from_str(&owner.ok(&["talk", "threads", &group, "--json"])).unwrap();
    assert_eq!(json, serde_json::json!([threads.trim_end()]));

    // The store holds a proposal event signed by the proposing node.
    let stored = EventStore::open(&store, WaitingLimits::default(), now_ms())
        .unwrap()
        .event_by_id(&Hash::from_str(&proposal).unwrap())
        .unwrap()
        .expect("the store has the proposal's event");
    assert_eq!(stored.header.extensions.kind, "chat.proposal.v1");
    assert_eq!(
        Hash::from_bytes(*stored.header.verifying_key.as_bytes()).to_hex(),
        proposer.key()
    );

    // A node outside the group can neither propose nor read the threads.
    let stderr = outsider.refused(&[
        "talk",
        "propose",
        &group,
        &object,
        "--base",
        &note,
        "--content",
        &content,
        "--thread",
        &thread,
    ]);
    assert!(stderr.contains("not a member"), "{stderr}");
    let stderr = outsider.refused(&["talk", "threads", &group]);
    assert!(stderr.contains("not a member"), "{stderr}");

    // A step whose line breaks the grammar is refused before anything is signed.
    let stderr = proposer.refused(&[
        "talk",
        "propose",
        &group,
        &object,
        "--base",
        &note,
        "--content",
        "not-a-blob",
        "--thread",
        &thread,
    ]);
    assert!(stderr.contains("blob"), "{stderr}");
    assert_eq!(owner.threads(&group), threads);
}

#[test]
fn test_a_resolution_is_shown_and_concurrent_ones_stand_side_by_side() {
    let root = TempDir::new().unwrap();
    let store_a = root.path().join("events-a");
    let store_b = root.path().join("events-b");
    let owner = node(root.path(), "owner", &store_a);
    let reader = node(root.path(), "reader", &store_b);
    let (create, body) = group_create(&[owner.key(), reader.key()]);
    let group = seed_group(&store_a, &create, &body);
    assert_eq!(seed_group(&store_b, &create, &body), group);
    let thread = thread(b"should we rename it");

    let summary = blob(b"we keep the name", &owner.key());
    let decided = owner.ok(&["talk", "resolve", &group, &thread, "--decision", &summary]);
    assert!(decided.starts_with("resolve "), "{decided:?}");
    assert_eq!(required(&decided, "kind"), "chat.resolution.v1");
    let decision = required(&decided, "event");
    let decision_text = format!(
        "decision, summary {summary}, by {} (event {decision})",
        owner.key()
    );
    assert_eq!(
        owner.threads(&group),
        format!("thread {thread} resolved: {decision_text}\n")
    );

    // The reader resolves the same thread without having seen the owner's resolution.
    let no_action = required(
        &reader.ok(&["talk", "resolve", &group, &thread, "--no-action"]),
        "event",
    );
    carry(&store_a, &store_b, &owner.key());
    carry(&store_b, &store_a, &reader.key());

    // Neither resolution follows the other, so both are shown and none is chosen.
    let no_action_text = format!("no action, by {} (event {no_action})", reader.key());
    let threads = owner.threads(&group);
    assert!(
        threads.starts_with(&format!(
            "thread {thread} has 2 concurrent resolutions, none chosen:\n"
        )),
        "{threads}"
    );
    assert!(
        threads.contains(&format!("  {decision_text}\n")),
        "{threads}"
    );
    assert!(
        threads.contains(&format!("  {no_action_text}\n")),
        "{threads}"
    );
    assert_eq!(reader.threads(&group), threads);

    // An edit resolution names its object, and a resolution has one outcome.
    reader.refused(&["talk", "resolve", &group, &thread, "--edit", &decision]);
    reader.refused(&[
        "talk",
        "resolve",
        &group,
        &thread,
        "--decision",
        &summary,
        "--no-action",
    ]);
}

#[test]
fn test_accept_reject_and_withdraw_close_proposals_and_a_moved_note_marks_the_rest_stale() {
    let root = TempDir::new().unwrap();
    let store = root.path().join("events");
    let owner = node(root.path(), "owner", &store);
    let proposer = node(root.path(), "proposer", &store);
    let group = group(&store, &[&owner, &proposer]);
    let (note, object) = founder_note(&store);
    let thread = thread(b"tighten the draft");
    let accepted_bytes = b"# Draft, tightened\n";

    let contents: [&[u8]; 4] = [
        accepted_bytes,
        b"# Draft, longer\n",
        b"# Draft, renamed\n",
        b"# Draft, reordered\n",
    ];
    let [accepted, rejected, withdrawn, left] = contents.map(|bytes| {
        propose(
            &proposer,
            &group,
            &object,
            &note,
            &blob(bytes, &proposer.key()),
            &thread,
        )
    });
    let threads = owner.threads(&group);
    for proposal in [&accepted, &rejected, &withdrawn, &left] {
        let line = proposal_line(&threads, proposal);
        assert!(line.ends_with(": open"), "{line}");
    }

    // The owner accepts one by revising the note with its content, citing it.
    let file = root.path().join("tightened.md");
    std::fs::write(&file, accepted_bytes).unwrap();
    let printed = owner.ok(&[
        "talk",
        "accept",
        &group,
        &accepted,
        file.to_str().unwrap(),
        "--media-type",
        "text/markdown",
    ]);
    assert!(printed.starts_with("accept "), "{printed:?}");
    assert_eq!(required(&printed, "proposal"), accepted);
    assert_eq!(required(&printed, "note"), note);
    assert_eq!(required(&printed, "matches_proposal"), "true");
    let edit = required(&printed, "revision");

    // The edit moved the note on, so the proposals still open are stale against it.
    let threads = owner.threads(&group);
    assert!(
        proposal_line(&threads, &accepted).ends_with(&format!(": accepted by edit {edit}")),
        "{threads}"
    );
    for proposal in [&rejected, &withdrawn, &left] {
        let line = proposal_line(&threads, proposal);
        assert!(line.ends_with(&format!(": open {STALE}")), "{line}");
    }

    // The owner now writes the note, so its rejection counts; the proposer withdraws its own.
    let reason = blob(b"too long", &owner.key());
    let rejection = required(
        &owner.ok(&["talk", "reject", &group, &rejected, "--because", &reason]),
        "event",
    );
    let withdrawal = required(
        &proposer.ok(&["talk", "withdraw", &group, &withdrawn]),
        "event",
    );
    // The proposer does not write the note, so its rejection is signed but does not count.
    proposer.ok(&["talk", "reject", &group, &left, "--because", &reason]);

    let threads = owner.threads(&group);
    assert!(
        proposal_line(&threads, &rejected)
            .ends_with(&format!(": rejected by {rejection} (reason {reason})")),
        "{threads}"
    );
    assert!(
        proposal_line(&threads, &withdrawn).ends_with(&format!(": withdrawn by {withdrawal}")),
        "{threads}"
    );
    assert!(
        proposal_line(&threads, &left).ends_with(&format!(": open {STALE}")),
        "{threads}"
    );
    assert_eq!(proposer.threads(&group), threads);

    // A proposal this node cannot see in the group is not accepted.
    let unknown = Hash::digest(b"no such proposal").to_hex();
    let stderr = owner.refused(&[
        "talk",
        "accept",
        &group,
        &unknown,
        file.to_str().unwrap(),
        "--media-type",
        "text/markdown",
    ]);
    assert!(stderr.contains("no proposal"), "{stderr}");
}
