//! Crash recovery for local writes (#13314, acceptance 5).
//!
//! A child process writes to a file-backed store in a tight loop and reports
//! each `set` the store acknowledged. The parent kills it with SIGKILL at a
//! different point in each trial, reopens the store and checks:
//!
//! - every value reads back whole (never a half row): each value carries a
//!   header naming the write that produced it and a body derived from it;
//! - every acknowledged version is present, and at most one more version (a
//!   commit that landed before the acknowledgement was printed) exists;
//! - each version has exactly one pending outbox intent, and nothing else is
//!   in the outbox: no orphan intent for a write that did not land.
//!
//! The next trial re-runs the write sequence on the same file after the
//! kill, so recovery is also exercised as a resumed writer.
//!
//! The child is this same test binary, re-entered through
//! `crash_child_writer`, which does nothing unless the parent set
//! `FLEET_KV_CRASH_CHILD_DB`.

#![cfg(unix)]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use fleet_kv::{Identity, LocalStore};

const CHILD_ENV: &str = "FLEET_KV_CRASH_CHILD_DB";
const ROUND_ENV: &str = "FLEET_KV_CRASH_CHILD_FIRST_ROUND";
const KEYS: u64 = 3;
const NAMESPACE: &str = "crash";

fn identity() -> Identity<'static> {
    Identity::new("owner", "context", "ses_crash")
}

fn key_for(round: u64) -> String {
    format!("k{}", round % KEYS)
}

/// The value written in `round`: a header naming the round, then a body of
/// at least 48 KiB (many pages) derived from it.
fn value_for(round: u64) -> Vec<u8> {
    let header = format!("round={round}\n");
    let seed = format!("<{round}:{}>", key_for(round));
    let len = 48 * 1024 + usize::try_from(round % 5).unwrap_or(0) * 4096;
    let mut value = header.into_bytes();
    while value.len() < len {
        value.extend_from_slice(seed.as_bytes());
    }
    value.truncate(len);
    value
}

fn round_of(value: &[u8]) -> u64 {
    let text = String::from_utf8_lossy(&value[..value.len().min(32)]);
    let header = text.split('\n').next().expect("header line");
    header
        .strip_prefix("round=")
        .and_then(|round| round.parse().ok())
        .unwrap_or_else(|| panic!("value has no round header: {header:?}"))
}

/// Child mode: write forever, printing one `acked` line per acknowledged set.
/// Without the environment variable this test is a no-op.
#[test]
fn crash_child_writer() {
    let Some(path) = std::env::var_os(CHILD_ENV) else {
        return;
    };
    let first: u64 = std::env::var(ROUND_ENV)
        .ok()
        .and_then(|round| round.parse().ok())
        .unwrap_or(0);
    let store = LocalStore::open(PathBuf::from(path)).expect("child open");
    let mut stdout = std::io::stdout().lock();
    for round in first.. {
        let key = key_for(round);
        let version = store
            .set(&identity(), NAMESPACE, &key, &value_for(round))
            .expect("child set");
        writeln!(stdout, "acked {key} {version} {round}").expect("report");
        stdout.flush().expect("flush");
    }
}

struct Acked {
    version: u64,
    round: u64,
}

fn parse_ack(line: &str, acked: &mut Vec<(String, Acked)>) {
    if let Some(rest) = line.strip_prefix("acked ") {
        let mut fields = rest.split(' ');
        let key = fields.next().expect("key").to_owned();
        let version = fields.next().expect("version").parse().expect("version");
        let round = fields.next().expect("round").parse().expect("round");
        acked.push((key, Acked { version, round }));
    }
}

/// Run the child until it has acknowledged `writes` sets, then SIGKILL it.
/// Returns every acknowledgement it printed and the next unused round.
fn write_then_kill(db: &Path, first_round: u64, writes: usize) -> (Vec<(String, Acked)>, u64) {
    let mut child = Command::new(std::env::current_exe().expect("test binary"))
        .args([
            "--exact",
            "crash_child_writer",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, db)
        .env(ROUND_ENV, first_round.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn child writer");
    let mut lines = BufReader::new(child.stdout.take().expect("child stdout")).lines();
    let mut acked = Vec::new();
    while acked.len() < writes {
        let line = lines
            .next()
            .expect("child exited before the kill")
            .expect("read child");
        parse_ack(&line, &mut acked);
    }
    child.kill().expect("SIGKILL child");
    child.wait().expect("reap child");
    // Lines the child printed before it died are acknowledgements too.
    for line in lines {
        let Ok(line) = line else { break };
        parse_ack(&line, &mut acked);
    }
    let next = acked.last().map_or(first_round, |(_, ack)| ack.round + 1)
        // The write after the last acknowledgement may have committed; skip
        // its round so a resumed writer never reuses it.
        + 1;
    (acked, next)
}

fn check_recovered(db: &Path, acked: &BTreeMap<String, Acked>) {
    let store = LocalStore::open(db).expect("reopen after kill");
    let listed = store
        .list(&identity(), NAMESPACE, true)
        .expect("list after kill");
    let heads: BTreeMap<_, _> = listed
        .iter()
        .map(|entry| (entry.key.clone(), entry.version))
        .collect();
    for key in acked.keys() {
        assert!(heads.contains_key(key), "acknowledged key {key} is missing");
    }
    for (key, &head) in &heads {
        let value = store
            .get(&identity(), NAMESPACE, key)
            .expect("get after kill")
            .unwrap_or_else(|| panic!("{key} reads absent"));
        let round = round_of(&value);
        assert_eq!(
            value,
            value_for(round),
            "{key}: torn value for round {round}"
        );
        assert_eq!(key_for(round), *key, "{key}: value from another key");
        match acked.get(key) {
            Some(last) if head == last.version => {
                assert_eq!(
                    round, last.round,
                    "{key}: head is not the last acknowledged write"
                );
            }
            Some(last) => {
                assert_eq!(
                    head,
                    last.version + 1,
                    "{key}: more than one unacknowledged version"
                );
                assert!(
                    round > last.round,
                    "{key}: unacknowledged head went backwards"
                );
            }
            // The key's first write committed but was never acknowledged.
            None => assert_eq!(head, 1, "{key}: unacknowledged key has history"),
        }
        let outbox = store
            .outbox(&identity(), NAMESPACE, key)
            .expect("outbox after kill");
        let versions: Vec<u64> = outbox.iter().map(|intent| intent.version).collect();
        assert_eq!(
            versions,
            (1..=head).collect::<Vec<_>>(),
            "{key}: exactly one intent per committed version"
        );
        assert!(
            outbox
                .iter()
                .all(|intent| intent.status == "pending" && intent.op == "set"),
            "{key}: every intent is a pending set"
        );
    }
}

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn sigkill_mid_set_leaves_whole_rows_and_one_intent_per_version() {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = Scratch(
        std::env::temp_dir().join(format!("fleet-kv-crash-{}-{suffix}", std::process::id())),
    );
    std::fs::create_dir_all(&dir.0).expect("scratch dir");
    let db = dir.0.join("local.db");

    let mut acked: BTreeMap<String, Acked> = BTreeMap::new();
    let mut round = 0;
    for writes in [1, 4, 9, 2, 15, 6] {
        let (seen, next) = write_then_kill(&db, round, writes);
        assert!(
            seen.len() >= writes,
            "child acknowledged {} writes",
            seen.len()
        );
        for (key, ack) in seen {
            acked.insert(key, ack);
        }
        round = next;
        check_recovered(&db, &acked);
        // A version the previous child committed but never acknowledged is
        // still the key's head; fold the store's real heads in so the next
        // trial compares against what the store holds.
        let store = LocalStore::open(&db).expect("reopen");
        for entry in store.list(&identity(), NAMESPACE, true).expect("list") {
            let value = store
                .get(&identity(), NAMESPACE, &entry.key)
                .expect("get")
                .expect("present");
            acked.insert(
                entry.key,
                Acked {
                    version: entry.version,
                    round: round_of(&value),
                },
            );
        }
    }
    assert_eq!(acked.len(), usize::try_from(KEYS).unwrap_or(0));
}
