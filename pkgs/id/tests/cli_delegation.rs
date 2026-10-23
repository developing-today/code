//! End-to-end tests for `id delegate` and `id revoke` (P6.7b).
//!
//! Each node is a data directory with its own key. Every node in a test points at one group log
//! with `--groups-log`, which stands in for the events sync will carry. The tests check what the
//! fold decided, read through the library, and not the text the CLI prints.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::str::FromStr;

use id::groups::log::GroupLog;
use id::groups::policy::Delegation;
use id::groups::{Event, GroupView, Outcome, Projection, Refusal};
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

fn projection(log: &Path) -> Projection {
    GroupLog::open(log).unwrap().projection().unwrap()
}

fn group_view(log: &Path, group: &str) -> GroupView {
    projection(log)
        .group(&hash(group))
        .cloned()
        .expect("the fold has the group")
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

/// A group with `owner` as its Admin, and `ann` added as a reader.
fn group_with_reader(owner: &Node, ann: &Node) -> String {
    let group = field(&owner.ok(&["group", "create"]), "group");
    owner.ok(&["group", "add", &group, &ann.key(), "--role", "read"]);
    group
}

#[test]
fn test_a_delegate_acts_until_the_grant_is_revoked() {
    let root = TempDir::new().unwrap();
    let log = root.path().join("groups.jsonl");
    let owner = node(root.path(), "owner", &log);
    let ann = node(root.path(), "ann", &log);
    let bob = node(root.path(), "bob", &log);
    let carol = node(root.path(), "carol", &log);
    let group = group_with_reader(&owner, &ann);

    // The owner grants Ann the read scope, and lets her pass it on.
    let grant = field(
        &owner.ok(&[
            "delegate",
            &group,
            &ann.key(),
            "--scope",
            "read",
            "--redelegate",
        ]),
        "event",
    );
    assert_eq!(
        projection(&log).outcome(&hash(&grant)),
        Some(&Outcome::Applied)
    );

    // The delegate acts: Ann passes the scope on to Bob.
    let passed = field(
        &ann.ok(&["delegate", &group, &bob.key(), "--scope", "read"]),
        "event",
    );
    assert_eq!(
        projection(&log).outcome(&hash(&passed)),
        Some(&Outcome::Applied)
    );

    // The owner revokes the grant to Ann.
    let revoked = owner.ok(&["revoke", &group, &grant]);
    assert_eq!(field(&revoked, "outcome"), "applied");
    assert!(!projection(&log).grant_in_force(&hash(&grant)));

    // Ann's later action has no chain to the owner, so the fold refuses it.
    let later = ann.run(&["delegate", &group, &carol.key(), "--scope", "read"]);
    assert!(!later.status.success(), "a revoked delegate still acted");
    let refused = last_event_by(&log, &key_bytes(&ann.key()));
    assert_eq!(
        projection(&log).outcome(&refused.id),
        Some(&Outcome::Refused(Refusal::NoChain))
    );
    assert!(
        !group_view(&log, &group)
            .members
            .contains_key(&key_bytes(&carol.key()))
    );
}

#[test]
fn test_delegate_sets_the_delegation_mode_that_policy_shows() {
    let root = TempDir::new().unwrap();
    let log = root.path().join("groups.jsonl");
    let owner = node(root.path(), "owner", &log);
    let ann = node(root.path(), "ann", &log);
    let group = group_with_reader(&owner, &ann);
    assert_eq!(
        field(&owner.ok(&["group", "policy", &group]), "delegation"),
        "cascade"
    );

    let out = owner.ok(&[
        "delegate",
        &group,
        &ann.key(),
        "--scope",
        "read",
        "--mode",
        "keep_prior",
    ]);
    let policy_line = out
        .lines()
        .find(|line| line.starts_with("policy "))
        .expect("the mode change is printed");
    assert_eq!(field(policy_line, "delegation"), "keep_prior");

    assert_eq!(
        field(&owner.ok(&["group", "policy", &group]), "delegation"),
        "keep_prior"
    );
    assert_eq!(
        group_view(&log, &group).policy.delegation_mode(),
        Delegation::KeepPrior
    );
}
