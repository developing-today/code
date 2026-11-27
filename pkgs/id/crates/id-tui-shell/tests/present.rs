//! Three-panel projection oracles (issue #12818).
//!
//! The recorded sessions in `contract/sessions/` are the fixture: the
//! shell folds every host frame they carry, and the three panels must
//! read exactly as below. The projection is a pure function of the
//! fold, so these lines are what the terminal shows for that session.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use id_tui_shell::present::{self, DIVERGED, NO_MESSAGES, NO_RECORDS, NO_VIEW, Panels};
use id_tui_shell::session::Session;
use serde_json::{Value, json};

fn recorded(name: &str) -> Vec<Value> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../contract/sessions")
        .join(name);
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|entry| entry["from"] == "server")
        .map(|entry| entry["frame"].clone())
        .collect()
}

fn fold(session: &mut Session, frames: &[Value]) {
    for frame in frames {
        session.apply(frame).unwrap();
    }
}

fn lines(expected: &[&str]) -> Vec<String> {
    expected.iter().map(|line| (*line).to_owned()).collect()
}

#[test]
fn a_fresh_session_shows_placeholders_in_every_panel() {
    let panels = present::project(&Session::new());
    assert_eq!(
        panels,
        Panels {
            selector: lines(&[NO_RECORDS]),
            document: lines(&[NO_VIEW]),
            chat: lines(&[NO_MESSAGES]),
        }
    );
}

#[test]
fn the_recorded_session_projects_onto_three_panels() {
    let mut session = Session::new();
    fold(&mut session, &recorded("tui.jsonl"));
    let panels = present::project(&session);
    assert_eq!(panels.selector, lines(&["room/lobby  Lobby"]));
    assert_eq!(
        panels.document,
        lines(&[
            "Ship the update?",
            "1. When (choice: now, later)",
            "1. Confirm",
            "2. Cancel",
            "",
            "form tui-choice: accepted",
        ])
    );
    assert_eq!(
        panels.chat,
        lines(&["[5] ann sent input (1 byte)", "[6] ann: hello"])
    );
}

#[test]
fn the_reconnect_continues_the_chat_and_replaces_the_document() {
    let mut session = Session::new();
    fold(&mut session, &recorded("tui.jsonl"));
    session.begin_reconnect();
    fold(&mut session, &recorded("tui-reconnect.jsonl"));
    let panels = present::project(&session);
    assert_eq!(
        panels.document,
        lines(&["Lobby"]),
        "a surface-less view shows its data"
    );
    assert_eq!(
        panels.chat,
        lines(&[
            "[5] ann sent input (1 byte)",
            "[6] ann: hello",
            "[7] ann sent input (1 byte)",
            "[8] ann: back",
        ])
    );
}

#[test]
fn projection_is_deterministic() {
    let mut first = Session::new();
    let mut second = Session::new();
    fold(&mut first, &recorded("tui.jsonl"));
    fold(&mut second, &recorded("tui.jsonl"));
    assert_eq!(present::project(&first), present::project(&second));
}

fn snapshot(current: u64) -> Value {
    json!({"type": "snapshot", "snapshot": {
        "current_sequence": current,
        "participants": [{"id": 1, "display_name": "ann"}, {"id": 2, "display_name": "bo"}],
        "events": [],
    }})
}

fn event(sequence: u64, participant: u64, kind: Value) -> Value {
    json!({"type": "event", "event": {"sequence": sequence, "participant_id": participant, "kind": kind}})
}

#[test]
fn host_text_never_carries_a_terminal_escape() {
    let mut session = Session::new();
    session.apply(&snapshot(0)).unwrap();
    session
        .apply(&event(
            1,
            1,
            json!({"kind": "chat", "data": "hi\u{1b}[2Jthere\nnext"}),
        ))
        .unwrap();
    session
        .apply(&json!({"type": "view", "sequence": 1, "data": "a\u{1b}]0;title\u{7}b", "view_revision": 1}))
        .unwrap();
    session
        .apply(&json!({"type": "records", "sequence": 1, "records": {"room/\u{1b}x": "R\u{9b}"}}))
        .unwrap();
    let panels = present::project(&session);
    for line in panels
        .selector
        .iter()
        .chain(&panels.document)
        .chain(&panels.chat)
    {
        assert!(!line.chars().any(char::is_control), "control in {line:?}");
    }
    assert_eq!(
        panels.chat,
        lines(&["[1] ann: hi\u{fffd}[2Jthere\u{fffd}next"])
    );
    assert_eq!(panels.document, lines(&["a\u{fffd}]0;title\u{fffd}b"]));
}

#[test]
fn every_event_kind_has_a_chat_line() {
    let mut session = Session::new();
    session.apply(&snapshot(0)).unwrap();
    let kinds = [
        (1, json!({"kind": "chat", "data": "yo"})),
        (2, json!({"kind": "input", "data": [1, 2, 3]})),
        (
            1,
            json!({"kind": "entity_input", "data": {"entity_ref": "door/7", "input": [1]}}),
        ),
        (
            0,
            json!({"kind": "notification", "data": {"text": "doors open"}}),
        ),
        (0, json!({"kind": "system", "data": {"event": "{}"}})),
        (
            0,
            json!({"kind": "program_installed", "data": {"module_hash": "ab", "seed": 0}}),
        ),
        (9, json!({"kind": "chat", "data": "who"})),
    ];
    for (index, (participant, kind)) in kinds.into_iter().enumerate() {
        session
            .apply(&event(index as u64 + 1, participant, kind))
            .unwrap();
    }
    assert_eq!(
        present::chat_lines(&session),
        lines(&[
            "[1] ann: yo",
            "[2] bo sent input (3 bytes)",
            "[3] ann acted on door/7",
            "[4] notice: doors open",
            "[5] system event",
            "[6] program changed",
            "[7] #9: who",
        ])
    );
    assert_eq!(
        present::selector_lines(&session),
        lines(&[NO_RECORDS, "object door/7"])
    );
}

#[test]
fn records_list_in_key_order_with_the_continuation() {
    let mut session = Session::new();
    session
        .apply(&json!({"type": "records", "sequence": 0,
            "records": {"room/b": "B", "room/a": {"title": "A"}}, "next_after": "room/b"}))
        .unwrap();
    assert_eq!(
        present::selector_lines(&session),
        lines(&[
            "room/a  {\"title\":\"A\"}",
            "room/b  B",
            "more after room/b"
        ])
    );
}

#[test]
fn a_rejected_form_shows_why_while_the_form_is_presented() {
    let mut session = Session::new();
    for frame in recorded("tui.jsonl") {
        if frame["type"] != "form_result_status" {
            session.apply(&frame).unwrap();
        }
    }
    session
        .apply(
            &json!({"type": "form_result_status", "form_id": "tui-choice", "ok": false,
            "error": "stale presentation"}),
        )
        .unwrap();
    let document = present::document_lines(&session);
    assert_eq!(
        document.last().map(String::as_str),
        Some("form tui-choice: rejected: stale presentation")
    );
}

#[test]
fn a_diverged_reconnect_holds_no_document_until_the_fresh_snapshot() {
    let mut session = Session::new();
    fold(&mut session, &recorded("tui.jsonl"));
    session.begin_reconnect();
    session
        .apply(&event(12, 1, json!({"kind": "chat", "data": "gap"})))
        .unwrap();
    assert!(session.diverged());
    let panels = present::project(&session);
    assert_eq!(panels.document, lines(&[DIVERGED]));
    assert_eq!(panels.chat, lines(&[NO_MESSAGES]));
}
