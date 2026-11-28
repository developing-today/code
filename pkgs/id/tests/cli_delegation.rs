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
use id::groups::{Body, Delegate, DeviceOp, Event, GroupView, Outcome, Projection, Refusal, Role};
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

/// Attenuation (design §7.2): a grantor passes on no more than it holds at that position. Ann
/// holds a delegated Read, so the real `id delegate ... --scope write` is refused by the fold as
/// `Attenuated`, and no grant is recorded for Bob.
#[test]
fn test_a_delegate_may_not_grant_above_its_scope() {
    let root = TempDir::new().unwrap();
    let log = root.path().join("groups.jsonl");
    let owner = node(root.path(), "owner", &log);
    let ann = node(root.path(), "ann", &log);
    let bob = node(root.path(), "bob", &log);
    let group = group_with_reader(&owner, &ann);
    owner.ok(&[
        "delegate",
        &group,
        &ann.key(),
        "--scope",
        "read",
        "--redelegate",
    ]);

    let refused = ann.run(&["delegate", &group, &bob.key(), "--scope", "write"]);
    assert!(
        !refused.status.success(),
        "a grant above the grantor's scope was accepted"
    );
    let event = last_event_by(&log, &key_bytes(&ann.key()));
    assert_eq!(
        projection(&log).outcome(&event.id),
        Some(&Outcome::Refused(Refusal::Attenuated)),
        "the refusal is attenuation: Ann holds Read and asked to grant Write"
    );
}

/// A veto (design §7) needs Manage. Bob is an ordinary reader, so the real `id revoke` of the
/// owner's grant to Ann is refused as `NotManager`, and the grant stays in force.
#[test]
fn test_revoke_by_a_non_manager_is_refused() {
    let root = TempDir::new().unwrap();
    let log = root.path().join("groups.jsonl");
    let owner = node(root.path(), "owner", &log);
    let ann = node(root.path(), "ann", &log);
    let bob = node(root.path(), "bob", &log);
    let group = group_with_reader(&owner, &ann);
    owner.ok(&["group", "add", &group, &bob.key(), "--role", "read"]);
    let grant = field(
        &owner.ok(&["delegate", &group, &ann.key(), "--scope", "read"]),
        "event",
    );

    let refused = bob.run(&["revoke", &group, &grant]);
    assert!(
        !refused.status.success(),
        "a reader without Manage revoked a grant"
    );
    let event = last_event_by(&log, &key_bytes(&bob.key()));
    assert_eq!(
        projection(&log).outcome(&event.id),
        Some(&Outcome::Refused(Refusal::NotManager)),
        "the refusal is standing: Bob holds no Manage at the veto's position"
    );
    assert!(
        projection(&log).grant_in_force(&hash(&grant)),
        "a refused veto took the grant out of force"
    );
}

/// A crossing (design §7.2, P2.7) may carry Write at most and must name a world. `id delegate`
/// has no cross-world flag yet, so the events are recorded through the same `GroupLog` the CLI
/// writes, and the fold's refusal is asserted by name for both halves of the rule.
#[test]
fn test_a_cross_world_grant_above_write_is_refused() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("groups.jsonl");
    let owner = node(root.path(), "owner", &path);
    let ann = node(root.path(), "ann", &path);
    let group = field(&owner.ok(&["group", "create"]), "group");
    let mut log = GroupLog::open(&path).unwrap();
    let author = key_bytes(&owner.key());
    let grantee = key_bytes(&ann.key());

    let over = log
        .record(
            author,
            Some(hash(&group)),
            Body::Cross {
                delegate: Delegate {
                    grantee,
                    scope: Role::Manage,
                    can_redelegate: false,
                    expires_at: None,
                },
                world: "another-world".to_owned(),
            },
        )
        .unwrap();
    assert_eq!(
        over.outcome,
        Outcome::Refused(Refusal::NotCrossable),
        "a crossing above Write was accepted"
    );

    let unnamed = log
        .record(
            author,
            Some(hash(&group)),
            Body::Cross {
                delegate: Delegate {
                    grantee,
                    scope: Role::Read,
                    can_redelegate: false,
                    expires_at: None,
                },
                world: String::new(),
            },
        )
        .unwrap();
    assert_eq!(
        unnamed.outcome,
        Outcome::Refused(Refusal::NotCrossable),
        "a crossing that names no world was accepted"
    );
}

/// A device acts with its account's authority capped by its own scope (design §8.3). A
/// Read-scoped device of the owner may not grant Write: the fold refuses it as `Attenuated`.
/// There is no CLI surface for acting as a device, so the events go through the CLI's own
/// `GroupLog`, as `id group` and `id delegate` write them.
#[test]
fn test_a_device_may_not_grant_above_its_scope() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("groups.jsonl");
    let owner = node(root.path(), "owner", &path);
    let bob = node(root.path(), "bob", &path);
    let group = field(&owner.ok(&["group", "create"]), "group");
    let mut log = GroupLog::open(&path).unwrap();
    let account = key_bytes(&owner.key());
    let device: [u8; 32] = [7; 32];

    let added = log
        .record(
            account,
            Some(hash(&group)),
            Body::Device(DeviceOp::Add {
                device,
                scope: Role::Read,
            }),
        )
        .unwrap();
    assert_eq!(
        added.outcome,
        Outcome::Applied,
        "the device was not authorised"
    );

    let refused = log
        .record(
            device,
            Some(hash(&group)),
            Body::Delegate(Delegate {
                grantee: key_bytes(&bob.key()),
                scope: Role::Write,
                can_redelegate: false,
                expires_at: None,
            }),
        )
        .unwrap();
    assert_eq!(
        refused.outcome,
        Outcome::Refused(Refusal::Attenuated),
        "a Read-scoped device granted Write"
    );
}
