//! Frame builders and session fold oracles (issue #12818, slice 1).
//!
//! Oracles: the F13.1 contract file, through the contract's own
//! checks, and the recorded capability statement. Every built frame
//! must pass `check_client_frame` in its phase; every presented frame
//! must pass `check_server_frame` on the way in; the built frame types
//! must equal the statement's `sends` list, and the folded frame kinds
//! must cover its `presents` list.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::PathBuf;

use id_tui_shell::frames;
use id_tui_shell::session::{Session, SessionError, Update};
use serde_json::{Map, Value, json};

fn statement() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("capability-statement.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn kinds(statement: &Value, key: &str) -> BTreeSet<String> {
    statement[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|kind| kind.as_str().unwrap().to_owned())
        .collect()
}

fn system_event(sequence: u64, what: &str, name: &str) -> Value {
    let system = json!({"event": what, "participant": {"name": name}}).to_string();
    json!({
        "sequence": sequence,
        "kind": {"kind": "system", "data": {"event": system}},
    })
}

fn snapshot(current: u64) -> Value {
    json!({"type": "snapshot", "snapshot": {"current_sequence": current, "events": []}})
}

fn view(sequence: u64, token: Option<&str>) -> Value {
    json!({
        "type": "view",
        "sequence": sequence,
        "data": "screen",
        "view_revision": sequence,
        "resume_token": token,
    })
}

#[test]
fn join_builds_fresh_and_resume_frames() {
    let fresh = frames::join("cap", None, None).unwrap();
    assert_eq!(fresh["type"], json!("join"));
    assert!(fresh["after"].is_null());
    assert!(fresh["resume_token"].is_null());

    let resume = frames::join("cap", Some(7), Some("tok")).unwrap();
    assert_eq!(resume["after"], json!(7));
    assert_eq!(resume["resume_token"], json!("tok"));
}

#[test]
fn join_refuses_oversize_capability_and_token() {
    let long = "x".repeat(frames::MAX_CAPABILITY_BYTES + 1);
    assert!(frames::join(&long, None, None).is_err());
    let token = "t".repeat(frames::MAX_RESUME_TOKEN_BYTES + 1);
    assert!(frames::join("cap", Some(1), Some(&token)).is_err());
}

#[test]
fn records_chat_input_paste_resize_build_contract_frames() {
    let records = frames::records("cap", Some("room/"), None).unwrap();
    assert_eq!(records["type"], json!("records"));
    assert_eq!(
        frames::chat("hello", Some(3)).unwrap()["text"],
        json!("hello")
    );
    assert_eq!(
        frames::input(&[0x1b, 0x5b, 0x41], None).unwrap()["data_hex"],
        json!("1b5b41")
    );
    assert_eq!(frames::paste("pasted", None).unwrap()["type"], json!("paste"));
    assert!(frames::paste(&"p".repeat(frames::MAX_PASTE_BYTES + 1), None).is_err());
    let resize = frames::resize(80, 24).unwrap();
    assert_eq!((resize["width"].clone(), resize["height"].clone()), (json!(80), json!(24)));
    assert!(frames::resize(0, 24).is_err());
}

#[test]
fn form_result_carries_the_presented_token_and_values() {
    let mut values = Map::new();
    values.insert("when".to_owned(), json!("now"));
    let frame =
        frames::form_result("hw-choice", "abcd", "submit", &values, Some(5)).unwrap();
    assert_eq!(
        frame,
        json!({
            "type": "form_result",
            "form_id": "hw-choice",
            "presentation_token": "abcd",
            "action": "submit",
            "values": {"when": "now"},
            "screen_revision": 5,
        })
    );
}

#[test]
fn built_frame_types_are_exactly_the_statement_sends() {
    let mut values = Map::new();
    values.insert("k".to_owned(), json!(true));
    let built: BTreeSet<String> = [
        frames::join("cap", None, None).unwrap(),
        frames::records("cap", None, None).unwrap(),
        frames::chat("t", None).unwrap(),
        frames::input(b"x", None).unwrap(),
        frames::paste("t", None).unwrap(),
        frames::form_result("f", "00", "submit", &values, None).unwrap(),
        frames::resize(1, 1).unwrap(),
    ]
    .iter()
    .map(|frame| frame["type"].as_str().unwrap().to_owned())
    .collect();
    assert_eq!(built, kinds(&statement(), "sends"));
}

#[test]
fn snapshot_resets_the_fold_to_the_host_position() {
    let mut session = Session::new();
    assert_eq!(session.apply(&snapshot(3)).unwrap(), Update::Snapshot);
    assert_eq!(session.last_sequence(), 3);
    assert!(session.events().is_empty());
}

#[test]
fn events_and_views_fold_and_record_the_resume_token() {
    let mut session = Session::new();
    session.apply(&snapshot(3)).unwrap();
    let event = json!({"type": "event", "event": system_event(4, "joined", "bo")});
    assert_eq!(session.apply(&event).unwrap(), Update::Event);
    assert_eq!(session.last_sequence(), 4);
    assert_eq!(session.events().len(), 1);

    assert_eq!(session.apply(&view(5, Some("tok5"))).unwrap(), Update::View);
    assert_eq!(session.last_sequence(), 5);
    assert_eq!(session.resume_token(), Some("tok5"));
    assert!(session.view().is_some());
}

#[test]
fn records_and_form_status_are_presented_and_kept() {
    let mut session = Session::new();
    let records = json!({"type": "records", "sequence": 2, "records": {}, "next_after": null});
    assert_eq!(session.apply(&records).unwrap(), Update::Records);
    assert!(session.records().is_some());

    let status = json!({"type": "form_result_status", "form_id": "f", "ok": true});
    assert_eq!(session.apply(&status).unwrap(), Update::FormResultStatus);
    assert_eq!(session.last_form_status().unwrap()["ok"], json!(true));
}

#[test]
fn error_frame_refuses_the_session_and_is_not_content() {
    let mut session = Session::new();
    let refused = json!({"type": "error", "message": "admin denied"});
    assert_eq!(
        session.apply(&refused),
        Err(SessionError::Refused("admin denied".to_owned()))
    );
    assert!(session.view().is_none());
    assert!(session.events().is_empty());
}

#[test]
fn every_presented_frame_kind_passes_the_contract_oracle() {
    let contract_has = |kind: &str| {
        let text = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contract/shell-contract.v1.json"),
        )
        .unwrap();
        let contract: Value = serde_json::from_str(&text).unwrap();
        contract["server_frames"].get(kind).is_some()
    };
    for kind in kinds(&statement(), "presents") {
        assert!(contract_has(&kind), "presented {kind} is a contract server frame");
    }
}

#[test]
fn reconnect_with_contiguous_replay_resumes_without_divergence() {
    let mut session = Session::new();
    session.apply(&snapshot(3)).unwrap();
    session.apply(&view(3, Some("tok"))).unwrap();

    let join = frames::join("cap", Some(session.last_sequence()), session.resume_token()).unwrap();
    assert_eq!(join["after"], json!(3));
    session.begin_reconnect();

    let event = json!({"type": "event", "event": system_event(4, "joined", "bo")});
    session.apply(&event).unwrap();
    assert!(!session.diverged());
    assert_eq!(session.events().len(), 1, "contiguous replay folds in");
    assert_eq!(session.last_sequence(), 4);
}

/// Oracle: the history-identity rule. A replay that does not
/// continue from the recorded position is never merged: the folded
/// events and view are discarded and only the position advances,
/// until a fresh snapshot re-establishes state.
#[test]
fn reconnect_with_a_gap_discards_state_and_never_merges() {
    let mut session = Session::new();
    session.apply(&snapshot(3)).unwrap();
    session.apply(&view(3, Some("tok"))).unwrap();
    session.begin_reconnect();

    let gapped = json!({"type": "event", "event": system_event(9, "joined", "bo")});
    session.apply(&gapped).unwrap();
    assert!(session.diverged());
    assert!(session.events().is_empty(), "gapped event is not folded");
    assert!(session.view().is_none(), "stale view is dropped");
    assert_eq!(session.last_sequence(), 9);

    session.apply(&snapshot(9)).unwrap();
    assert!(!session.diverged(), "a fresh snapshot re-establishes state");
}
