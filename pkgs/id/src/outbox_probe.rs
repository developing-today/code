// Probe (lane B, 2026-11-02): does Outbox::open replay validate the same
// state transitions the live API refuses? Writes crafted journal lines
// directly, as a journal left by another writer / a damaged store would.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::PathBuf;

use crate::directory::{Envelope, EnvelopeKind};
use crate::envelope_outbox::{Outcome, Outbox};

fn envelope(id: &str) -> Envelope {
    Envelope {
        kind: EnvelopeKind::FriendRequest,
        from: "ann".to_owned(),
        to: "bo".to_owned(),
        audience: "club".to_owned(),
        id: id.to_owned(),
        request: None,
        at: 1_000_000_000,
        signature: "00".to_owned(),
    }
}

fn target() -> String {
    iroh::SecretKey::from_bytes(&[7; 32]).public().to_string()
}

fn append(path: &PathBuf, line: &str) {
    let mut file = OpenOptions::new().append(true).open(path).unwrap();
    writeln!(file, "{line}").unwrap();
}

#[test]
fn probe_replay_after_finish_and_second_finish() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("outbox.jsonl");
    let mut outbox = Outbox::open(&path).unwrap();
    outbox.enqueue(&target(), envelope("e1")).unwrap();
    drop(outbox);
    append(&path, r#"{"op":"finished","id":"e1","attempts":1,"outcome":"delivered","detail":null}"#);
    append(&path, r#"{"op":"attempted","id":"e1","attempts":7,"next_at":999,"error":"later"}"#);
    append(&path, r#"{"op":"finished","id":"e1","attempts":8,"outcome":"gave_up","detail":"overwritten"}"#);
    let reopened = Outbox::open(&path).expect("replay accepts the journal");
    let entry = reopened.entry("e1").expect("e1 present");
    println!(
        "PROBE1 finish={:?} attempts={} last_error={:?}",
        entry.finish, entry.attempts, entry.last_error
    );
}

#[test]
fn probe_replay_attempts_regress_and_skip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("outbox.jsonl");
    let mut outbox = Outbox::open(&path).unwrap();
    outbox.enqueue(&target(), envelope("e2")).unwrap();
    drop(outbox);
    append(&path, r#"{"op":"attempted","id":"e2","attempts":5,"next_at":999,"error":"skip"}"#);
    append(&path, r#"{"op":"attempted","id":"e2","attempts":2,"next_at":999,"error":"regress"}"#);
    let reopened = Outbox::open(&path).expect("replay accepts the journal");
    let entry = reopened.entry("e2").expect("e2 present");
    println!("PROBE2 attempts={} last_error={:?}", entry.attempts, entry.last_error);
}

#[test]
fn probe_replay_withdraw_after_attempt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("outbox.jsonl");
    let mut outbox = Outbox::open(&path).unwrap();
    outbox.enqueue(&target(), envelope("e3")).unwrap();
    drop(outbox);
    append(&path, r#"{"op":"attempted","id":"e3","attempts":1,"next_at":999,"error":"tried"}"#);
    append(&path, r#"{"op":"withdrawn","id":"e3"}"#);
    let reopened = Outbox::open(&path).expect("replay accepts the journal");
    println!("PROBE3 e3 present after replay: {}", reopened.entry("e3").is_some());
}

#[test]
fn probe_live_path_refuses_the_same_transitions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("outbox.jsonl");
    let mut outbox = Outbox::open(&path).unwrap();
    outbox.enqueue(&target(), envelope("e4")).unwrap();
    outbox.record("e4", 1_000_000_000, Outcome::Delivered).unwrap();
    let record_after_finish = outbox.record("e4", 1_000_000_000, Outcome::Retry("x".to_owned()));
    println!("PROBE4 record-after-finish is_err: {}", record_after_finish.is_err());

    outbox.enqueue(&target(), envelope("e5")).unwrap();
    outbox.record("e5", 1_000_000_000, Outcome::Retry("down".to_owned())).unwrap();
    let withdraw_after_attempt = outbox.withdraw("e5");
    println!("PROBE5 withdraw-after-attempt is_err: {}", withdraw_after_attempt.is_err());
}
