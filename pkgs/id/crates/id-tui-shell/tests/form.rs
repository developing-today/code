//! Form answer oracles (issue #12818): the TUI answers a presented
//! form the way the SSH terminal presenter does, and checks the answer
//! against the form's own field model before anything is sent.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id_tui_shell::app::{App, Input, Step};
use id_tui_shell::form::{self, ANSWER_HINT};
use id_tui_shell::frames;
use id_tui_shell::keys::Key;
use id_tui_shell::present;
use id_tui_shell::session::Session;
use serde_json::{Map, Value, json};

const TOKEN: &str = "0123456789abcdef0123456789abcdef";
const FRESH: &str = "fedcba9876543210fedcba9876543210";

fn form_view(token: Option<&str>) -> Value {
    let mut view = json!({
        "type": "view",
        "sequence": 0,
        "data": "Ship the update?",
        "view_revision": 1,
        "resume_token": "r0",
        "surface": {
            "kind": "form",
            "version": 1,
            "form_id": "tui-choice",
            "presentation_token": token,
            "title": "Ship the update?",
            "fields": [
                {"kind": "choice", "id": "when", "label": "When", "required": true,
                 "options": ["now", "later"]},
                {"kind": "number", "id": "count", "label": "Count", "required": false,
                 "min": 1, "max": 3},
            ],
            "actions": [
                {"id": "submit", "label": "Confirm"},
                {"id": "cancel", "label": "Cancel", "kind": "cancel"},
            ],
        },
    });
    if token.is_none() {
        view["surface"]
            .as_object_mut()
            .unwrap()
            .remove("presentation_token");
    }
    view
}

fn presented(token: Option<&str>) -> Session {
    let mut session = Session::new();
    session
        .apply(&json!({"type": "snapshot", "snapshot": {"current_sequence": 0, "events": []}}))
        .unwrap();
    session.apply(&form_view(token)).unwrap();
    session
}

fn values(value: &Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

#[test]
fn a_presented_form_with_a_token_is_in_force() {
    let session = presented(Some(TOKEN));
    let active = form::active(&session).unwrap();
    assert_eq!(active.form_id, "tui-choice");
    assert_eq!(active.presentation_token.as_deref(), Some(TOKEN));
    assert!(
        form::active(&presented(None)).is_none(),
        "no token, no answer"
    );
}

#[test]
fn a_surface_less_view_has_no_form() {
    let mut session = Session::new();
    session
        .apply(&json!({"type": "snapshot", "snapshot": {"current_sequence": 0, "events": []}}))
        .unwrap();
    session
        .apply(
            &json!({"type": "view", "sequence": 0, "data": "Lobby", "view_revision": 1,
            "resume_token": null}),
        )
        .unwrap();
    assert!(form::active(&session).is_none());
}

#[test]
fn the_hosts_status_settles_the_form() {
    let mut accepted = presented(Some(TOKEN));
    accepted
        .apply(&json!({"type": "form_result_status", "form_id": "tui-choice", "ok": true}))
        .unwrap();
    assert!(form::active(&accepted).is_none());

    let mut retry = presented(Some(TOKEN));
    retry
        .apply(
            &json!({"type": "form_result_status", "form_id": "tui-choice", "ok": false,
            "error": "invalid", "presentation_token": FRESH}),
        )
        .unwrap();
    assert_eq!(
        form::active(&retry).unwrap().presentation_token.as_deref(),
        Some(FRESH),
        "a rejection with a fresh token re-presents the form"
    );

    let mut closed = presented(Some(TOKEN));
    closed
        .apply(
            &json!({"type": "form_result_status", "form_id": "tui-choice", "ok": false,
            "error": "form_unavailable"}),
        )
        .unwrap();
    assert!(form::active(&closed).is_none());
}

#[test]
fn a_valid_answer_is_the_builders_form_result() {
    let active = form::active(&presented(Some(TOKEN))).unwrap();
    let frame = form::answer(
        &active,
        r#" {"action": "submit", "values": {"when": "later", "count": 2}} "#,
    )
    .unwrap();
    assert_eq!(
        frame,
        frames::form_result(
            "tui-choice",
            TOKEN,
            "submit",
            &values(&json!({"when": "later", "count": 2})),
            None
        )
        .unwrap()
    );
}

#[test]
fn cancel_sends_the_cancel_action_without_values() {
    let active = form::active(&presented(Some(TOKEN))).unwrap();
    let frame = form::answer(&active, "/cancel").unwrap();
    assert_eq!(frame["action"], json!("cancel"));
    assert_eq!(frame["values"], json!({}));
}

#[test]
fn answers_the_form_refuses_stay_local() {
    let active = form::active(&presented(Some(TOKEN))).unwrap();
    for line in [
        "not json",
        "[1, 2]",
        r#"{"action": "submit"}"#,
        r#"{"action": "submit", "values": {}, "extra": 1}"#,
        r#"{"action": 1, "values": {}}"#,
        r#"{"action": "submit", "values": []}"#,
        r#"{"action": "launch", "values": {"when": "now"}}"#,
        r#"{"action": "submit", "values": {}}"#,
        r#"{"action": "submit", "values": {"when": "never"}}"#,
        r#"{"action": "submit", "values": {"when": "now", "count": 9}}"#,
        r#"{"action": "submit", "values": {"when": "now", "who": "me"}}"#,
        r#"{"action": "cancel", "values": {"when": "now"}}"#,
    ] {
        assert!(form::answer(&active, line).is_err(), "accepted {line}");
    }
}

#[test]
fn only_cancel_and_json_lines_are_answers() {
    assert!(form::is_answer("/cancel"));
    assert!(form::is_answer("  {\"action\": \"submit\"}"));
    assert!(!form::is_answer("hello"));
    assert!(!form::is_answer("/cancelled"));
}

#[test]
fn the_document_says_how_to_answer_a_form_in_force() {
    let session = presented(Some(TOKEN));
    assert_eq!(
        present::document_lines(&session).last().map(String::as_str),
        Some(ANSWER_HINT)
    );
    assert!(!present::document_lines(&presented(None)).contains(&ANSWER_HINT.to_owned()));
}

fn app_with_form() -> App {
    let mut app = App::new("tui-capability");
    app.opening_frames(80, 24).unwrap();
    app.handle(Input::Host(
        json!({"type": "snapshot", "snapshot": {"current_sequence": 0, "events": []}}),
    ));
    app.handle(Input::Host(form_view(Some(TOKEN))));
    app
}

#[test]
fn a_refused_answer_keeps_the_line_and_says_why() {
    let mut app = app_with_form();
    let line = r#"{"action": "submit", "values": {"when": "never"}}"#;
    app.handle(Input::Key(Key::Paste(line.to_owned())));
    assert_eq!(app.handle(Input::Key(Key::Enter)), None);
    assert_eq!(app.keys().prompt.text, line);
    assert!(app.keys().prompt.status.is_some());
}

#[test]
fn ordinary_lines_stay_chat_while_a_form_is_in_force() {
    let mut app = app_with_form();
    app.handle(Input::Key(Key::Paste("hello".to_owned())));
    assert_eq!(
        app.handle(Input::Key(Key::Enter)),
        Some(Step::Send(frames::chat("hello", None).unwrap()))
    );
}

#[test]
fn a_json_line_is_chat_when_no_form_is_in_force() {
    let mut app = App::new("tui-capability");
    let line = r#"{"action": "submit", "values": {}}"#;
    app.handle(Input::Key(Key::Paste(line.to_owned())));
    assert_eq!(
        app.handle(Input::Key(Key::Enter)),
        Some(Step::Send(frames::chat(line, None).unwrap()))
    );
}
