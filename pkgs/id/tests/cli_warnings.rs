//! End-to-end tests for group size tiers and warnings UI (P8.6).
//!
//! Driving the binary using the pattern in `tests/cli_groups.rs`:
//! - A normal group with default members has tier=0 and no warning.
//! - When member count reaches or exceeds 96, tier=96 and the size warning message is shown.
//! - JSON output contains the tier and warning fields.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::str::FromStr;

use id::groups::Role;
use id::groups::log::GroupLog;
use id::groups::warnings::Warning;
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

#[test]
fn test_default_group_has_tier_0_and_no_warning() {
    let root = TempDir::new().unwrap();
    let log = root.path().join("groups.jsonl");
    let owner = node(root.path(), "owner", &log);

    let created = owner.ok(&["group", "create"]);
    let group = field(&created, "group");

    // Policy line: tier=0 and no warning
    let policy = owner.ok(&["group", "policy", &group]);
    assert_eq!(field(&policy, "tier"), "0");
    assert!(!policy.contains("warning="));

    // Policy JSON: tier=0 and no warning key
    let policy_json: serde_json::Value =
        serde_json::from_str(owner.ok(&["group", "policy", &group, "--json"]).trim()).unwrap();
    assert_eq!(policy_json["tier"], 0);
    assert!(policy_json.get("warning").is_none());

    // List line: tier=0 and no warning
    let list = owner.ok(&["group", "list"]);
    let line = list
        .lines()
        .find(|l| field(l, "group") == group)
        .expect("group is listed");
    assert_eq!(field(line, "tier"), "0");
    assert!(!line.contains("warning="));

    // List JSON: tier=0 and no warning key
    let list_json: serde_json::Value =
        serde_json::from_str(owner.ok(&["group", "list", "--json"]).trim()).unwrap();
    let groups = list_json.as_array().expect("list is an array");
    let entry = groups
        .iter()
        .find(|g| g["group"] == group.as_str())
        .expect("group in json list");
    assert_eq!(entry["tier"], 0);
    assert!(entry.get("warning").is_none());
}

#[test]
fn test_group_at_96_members_has_tier_96_and_size_warning() {
    let root = TempDir::new().unwrap();
    let log = root.path().join("groups.jsonl");
    let owner = node(root.path(), "owner", &log);

    let created = owner.ok(&["group", "create"]);
    let group = field(&created, "group");
    let owner_key = key_bytes(&owner.key());
    let group_hash = hash(&group);

    // Add 95 members so total member count reaches 96 (owner + 95)
    let mut group_log = GroupLog::open(&log).unwrap();
    for i in 1..96 {
        let mut member_key = [0u8; 32];
        member_key[..8].copy_from_slice(&(i as u64).to_be_bytes());
        group_log
            .add(owner_key, group_hash, member_key, Role::Read)
            .unwrap();
    }

    let expected_warning = Warning::Size {
        threshold: 96,
        members: 96,
    }
    .message();

    // Policy line: tier=96 and size warning message shown
    let policy = owner.ok(&["group", "policy", &group]);
    assert_eq!(field(&policy, "tier"), "96");
    assert!(policy.contains(&expected_warning));

    // Policy JSON: tier=96 and warning string field
    let policy_json: serde_json::Value =
        serde_json::from_str(owner.ok(&["group", "policy", &group, "--json"]).trim()).unwrap();
    assert_eq!(policy_json["tier"], 96);
    assert_eq!(policy_json["warning"], expected_warning.as_str());

    // List line: tier=96 and size warning message shown
    let list = owner.ok(&["group", "list"]);
    let line = list
        .lines()
        .find(|l| field(l, "group") == group)
        .expect("group is listed");
    assert_eq!(field(line, "tier"), "96");
    assert_eq!(field(line, "members"), "96");
    assert!(line.contains(&expected_warning));

    // List JSON: tier=96 and warning string field
    let list_json: serde_json::Value =
        serde_json::from_str(owner.ok(&["group", "list", "--json"]).trim()).unwrap();
    let groups = list_json.as_array().expect("list is an array");
    let entry = groups
        .iter()
        .find(|g| g["group"] == group.as_str())
        .expect("group in json list");
    assert_eq!(entry["tier"], 96);
    assert_eq!(entry["members"], 96);
    assert_eq!(entry["warning"], expected_warning.as_str());
}

#[test]
fn test_group_tier_transitions_and_warning_messages() {
    let root = TempDir::new().unwrap();
    let log = root.path().join("groups.jsonl");
    let owner = node(root.path(), "owner", &log);

    let created = owner.ok(&["group", "create"]);
    let group = field(&created, "group");
    let owner_key = key_bytes(&owner.key());
    let group_hash = hash(&group);

    let mut group_log = GroupLog::open(&log).unwrap();
    for i in 1..100 {
        let mut member_key = [0u8; 32];
        member_key[..8].copy_from_slice(&(i as u64).to_be_bytes());
        group_log
            .add(owner_key, group_hash, member_key, Role::Read)
            .unwrap();
    }

    // At 100 members: tier=96, warning mentions 100 now
    let expected_warning_100 = Warning::Size {
        threshold: 96,
        members: 100,
    }
    .message();
    let policy_json: serde_json::Value =
        serde_json::from_str(owner.ok(&["group", "policy", &group, "--json"]).trim()).unwrap();
    assert_eq!(policy_json["tier"], 96);
    assert_eq!(policy_json["warning"], expected_warning_100.as_str());

    // Add up to 128 members (28 more)
    for i in 100..128 {
        let mut member_key = [0u8; 32];
        member_key[..8].copy_from_slice(&(i as u64).to_be_bytes());
        group_log
            .add(owner_key, group_hash, member_key, Role::Read)
            .unwrap();
    }

    // At 128 members: tier=128
    let expected_warning_128 = Warning::Size {
        threshold: 128,
        members: 128,
    }
    .message();
    let policy_json: serde_json::Value =
        serde_json::from_str(owner.ok(&["group", "policy", &group, "--json"]).trim()).unwrap();
    assert_eq!(policy_json["tier"], 128);
    assert_eq!(policy_json["warning"], expected_warning_128.as_str());
}
