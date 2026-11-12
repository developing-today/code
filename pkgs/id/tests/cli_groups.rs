//! End-to-end tests for `id group` (P6.7a): create, policy, add, remove, role, and list.
//!
//! Each node is a data directory with its own key. Every node in a test points at one group log
//! with `--groups-log`, which stands in for the events sync will carry. The tests check what the
//! fold decided, read through the library, and not the text the CLI prints.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::str::FromStr;

use id::groups::log::GroupLog;
use id::groups::{Event, GroupView, Outcome, Refusal, Role};
use iroh_base::{EndpointAddr, SecretKey, TransportAddr};
use iroh_tickets::endpoint::EndpointTicket;
use p2panda_core::Hash;
use tempfile::TempDir;

/// One node: its data directory. The group log is shared by every node of a test.
struct Node {
    dir: PathBuf,
    log: PathBuf,
}

impl Node {
    /// Runs `id` as this node, with the shared group log.
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_id"))
            .arg("--data-dir")
            .arg(&self.dir)
            .arg("--groups-log")
            .arg(&self.log)
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

fn node(root: &Path, name: &str, log: &Path) -> Node {
    Node {
        dir: root.join(name),
        log: log.to_path_buf(),
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

fn group_view(log: &Path, group: &str) -> GroupView {
    GroupLog::open(log)
        .unwrap()
        .projection()
        .unwrap()
        .group(&hash(group))
        .cloned()
        .expect("the fold has the group")
}

fn outcome_of(log: &Path, event: &Hash) -> Option<Outcome> {
    GroupLog::open(log)
        .unwrap()
        .projection()
        .unwrap()
        .outcome(event)
        .cloned()
}

/// The last event in the log that `author` signed.
fn last_event_by(log: &Path, author: &[u8; 32]) -> Event {
    GroupLog::open(log)
        .unwrap()
        .events()
        .iter()
        .rev()
        .find(|event| event.author == *author)
        .cloned()
        .expect("the author has an event")
}

#[test]
fn test_group_create_add_list_role_and_remove_round_trip() {
    let root = TempDir::new().unwrap();
    let log = root.path().join("groups.jsonl");
    let owner = node(root.path(), "owner", &log);
    let ann = node(root.path(), "ann", &log);
    let ann_key = ann.key();

    let created = owner.ok(&["group", "create"]);
    let group = field(&created, "group");
    assert_eq!(field(&created, "outcome"), "applied");

    let added = owner.ok(&["group", "add", &group, &ann_key, "--role", "read"]);
    assert_eq!(field(&added, "outcome"), "applied");
    assert_eq!(field(&added, "role"), "read");

    let listed = owner.ok(&["group", "list"]);
    let line = listed
        .lines()
        .find(|line| field(line, "group") == group)
        .expect("the group is listed");
    assert_eq!(field(line, "active"), "true");
    assert_eq!(field(line, "members"), "2");

    owner.ok(&["group", "role", &group, &ann_key, "write"]);
    assert_eq!(
        group_view(&log, &group).members.get(&key_bytes(&ann_key)),
        Some(&Role::Write)
    );

    owner.ok(&["group", "remove", &group, &ann_key]);
    assert!(
        !group_view(&log, &group)
            .members
            .contains_key(&key_bytes(&ann_key))
    );
    let listed = owner.ok(&["group", "list"]);
    assert!(
        listed
            .lines()
            .any(|line| field(line, "group") == group && field(line, "members") == "1")
    );
}

#[test]
fn test_policy_shows_the_defaults_of_a_normal_group() {
    let root = TempDir::new().unwrap();
    let log = root.path().join("groups.jsonl");
    let owner = node(root.path(), "owner", &log);
    let group = field(&owner.ok(&["group", "create"]), "group");

    let policy = owner.ok(&["group", "policy", &group]);
    assert_eq!(field(&policy, "cap"), "none");
    assert_eq!(field(&policy, "join"), "manager_add");
    assert_eq!(field(&policy, "on_removed"), "continue");
    assert_eq!(field(&policy, "delegation"), "cascade");
}

#[test]
fn test_a_removed_member_cannot_add_and_the_fold_refuses_it() {
    let root = TempDir::new().unwrap();
    let log = root.path().join("groups.jsonl");
    let owner = node(root.path(), "owner", &log);
    let ann = node(root.path(), "ann", &log);
    let bob = node(root.path(), "bob", &log);
    let (ann_key, bob_key) = (ann.key(), bob.key());
    let group = field(&owner.ok(&["group", "create"]), "group");
    owner.ok(&["group", "add", &group, &ann_key, "--role", "manage"]);
    owner.ok(&["group", "remove", &group, &ann_key]);

    let attempt = ann.run(&["group", "add", &group, &bob_key, "--role", "read"]);
    assert!(!attempt.status.success(), "a removed member added a member");

    let refused = last_event_by(&log, &key_bytes(&ann_key));
    assert_eq!(
        outcome_of(&log, &refused.id),
        Some(Outcome::Refused(Refusal::NotActive))
    );
    assert!(
        !group_view(&log, &group)
            .members
            .contains_key(&key_bytes(&bob_key))
    );
}

#[test]
fn test_create_with_max_refuses_the_add_past_the_cap() {
    let root = TempDir::new().unwrap();
    let log = root.path().join("groups.jsonl");
    let owner = node(root.path(), "owner", &log);
    let ann = node(root.path(), "ann", &log);
    let bob = node(root.path(), "bob", &log);
    let (ann_key, bob_key) = (ann.key(), bob.key());
    let group = field(&owner.ok(&["group", "create", "--max", "2"]), "group");
    owner.ok(&["group", "add", &group, &ann_key, "--role", "read"]);

    let attempt = owner.run(&["group", "add", &group, &bob_key, "--role", "read"]);
    assert!(!attempt.status.success(), "the cap of 2 let a third in");
    let refused = last_event_by(&log, &key_bytes(&owner.key()));
    assert_eq!(
        outcome_of(&log, &refused.id),
        Some(Outcome::Refused(Refusal::OverCap { cap: 2 }))
    );
}

#[test]
fn test_admins_on_create_is_for_friendships_only() {
    let root = TempDir::new().unwrap();
    let log = root.path().join("groups.jsonl");
    let owner = node(root.path(), "owner", &log);

    let attempt = owner.run(&["group", "create", "--admins-on-create"]);
    assert!(!attempt.status.success());
    let stderr = String::from_utf8_lossy(&attempt.stderr);
    assert!(stderr.contains("friendships"), "stderr: {stderr}");
    assert!(GroupLog::open(&log).unwrap().events().is_empty());
}

#[test]
fn test_a_friendship_has_no_admin_unless_admins_on_create() {
    let root = TempDir::new().unwrap();
    let log = root.path().join("groups.jsonl");
    let owner = node(root.path(), "owner", &log);
    let ann = node(root.path(), "ann", &log);
    let bob = node(root.path(), "bob", &log);
    let owner_key = owner.key();

    let plain = field(
        &owner.ok(&[
            "group",
            "create",
            "--with",
            &ann.key(),
            "--template",
            "friendship",
        ]),
        "group",
    );
    let with_admins = field(
        &owner.ok(&[
            "group",
            "create",
            "--template",
            "friendship",
            "--with",
            &bob.key(),
            "--admins-on-create",
        ]),
        "group",
    );

    assert_eq!(
        group_view(&log, &plain).members.get(&key_bytes(&owner_key)),
        Some(&Role::Manage)
    );
    assert_eq!(
        group_view(&log, &with_admins)
            .members
            .get(&key_bytes(&owner_key)),
        Some(&Role::Admin)
    );
}

#[test]
fn test_json_form_prints_one_object_per_action() {
    let root = TempDir::new().unwrap();
    let log = root.path().join("groups.jsonl");
    let owner = node(root.path(), "owner", &log);
    let ann_key = node(root.path(), "ann", &log).key();

    let created: serde_json::Value =
        serde_json::from_str(owner.ok(&["group", "create", "--json"]).trim()).unwrap();
    let group = created["group"].as_str().unwrap().to_owned();
    assert_eq!(created["command"], "create");
    assert_eq!(created["outcome"], "applied");
    assert_eq!(group.len(), 64);

    let added: serde_json::Value = serde_json::from_str(
        owner
            .ok(&["group", "add", &group, &ann_key, "--json"])
            .trim(),
    )
    .unwrap();
    assert_eq!(added["member"], ann_key.as_str());
    assert_eq!(added["role"], "read");

    let listed: serde_json::Value =
        serde_json::from_str(owner.ok(&["group", "list", "--json"]).trim()).unwrap();
    let groups = listed.as_array().expect("list is an array");
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["group"], group.as_str());
    assert_eq!(groups[0]["members"], 2);
}

#[test]
fn test_group_join_help_says_unavailable() {
    let root = TempDir::new().unwrap();
    let log = root.path().join("groups.jsonl");
    let ann = node(root.path(), "ann", &log);

    let output = ann.run(&["group", "join", "--help"]);
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("Unavailable"), "help: {help}");
    assert!(help.contains("not implemented yet"), "help: {help}");
}

#[test]
fn test_group_join_refuses_a_valid_invitation_and_records_nothing() {
    let root = TempDir::new().unwrap();
    let log = root.path().join("groups.jsonl");
    let ann = node(root.path(), "ann", &log);

    // A well-formed endpoint ticket reaches the explicit unavailable path,
    // rather than only exercising invitation parsing failure.
    let node_id = SecretKey::from_bytes(&[7; 32]).public();
    let ip = "203.0.113.7:4433".parse().unwrap();
    let invite = EndpointTicket::new(EndpointAddr::from_parts(node_id, [TransportAddr::Ip(ip)]));
    let attempt = ann.run(&["group", "join", &invite.to_string()]);
    assert!(!attempt.status.success());
    let stderr = String::from_utf8_lossy(&attempt.stderr);
    assert!(
        stderr.contains("group join is unavailable"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("not implemented yet"), "stderr: {stderr}");
    assert!(GroupLog::open(&log).unwrap().events().is_empty());
}
