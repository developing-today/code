//! Session loop oracles (issue #12818): the recorded sessions under
//! `contract/sessions/` drive [`App`] the way the binary does, so the
//! join, read, act and reconnect workflow is checked without a
//! terminal or a network.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use id_tui_shell::app::{App, Input, RECONNECTING, Step};
use id_tui_shell::keys::Key;
use id_tui_shell::render::Focus;
use serde_json::{Value, json};

const CAPABILITY: &str = "tui-capability";

/// The recorded frames of one session, split by who sent them.
fn recorded(name: &str) -> (Vec<Value>, Vec<Value>) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../contract/sessions")
        .join(name);
    let mut client = Vec::new();
    let mut server = Vec::new();
    for line in std::fs::read_to_string(path).unwrap().lines() {
        let entry: Value = serde_json::from_str(line).unwrap();
        let frame = entry["frame"].clone();
        match entry["from"].as_str().unwrap() {
            "client" => client.push(frame),
            "server" => server.push(frame),
            other => panic!("unexpected sender {other}"),
        }
    }
    (client, server)
}

fn host(app: &mut App, frame: &Value) {
    assert_eq!(app.handle(Input::Host(frame.clone())), None);
}

fn key(app: &mut App, key: Key) -> Option<Step> {
    app.handle(Input::Key(key))
}

#[test]
fn opening_frames_join_size_the_screen_and_ask_for_rooms() {
    let (client, _) = recorded("tui.jsonl");
    let mut app = App::new(CAPABILITY);
    let opening = app.opening_frames(80, 24).unwrap();
    assert_eq!(opening.len(), 3);
    assert_eq!(opening[0], client[0], "the recorded fresh join");
    assert_eq!(opening[1]["type"], json!("resize"));
    assert_eq!(opening[1]["width"], json!(80));
    assert_eq!(opening[1]["height"], json!(24));
    assert_eq!(opening[2], client[1], "the recorded records request");
}

#[test]
fn an_oversize_terminal_is_clamped_into_the_contract() {
    let mut app = App::new(CAPABILITY);
    let opening = app.opening_frames(u16::MAX, 0).unwrap();
    assert_eq!(opening[1]["width"], json!(500));
    assert_eq!(opening[1]["height"], json!(1));
}

#[test]
fn the_recorded_session_folds_and_keys_send_its_client_frames() {
    let (client, server) = recorded("tui.jsonl");
    let mut app = App::new(CAPABILITY);
    app.opening_frames(80, 24).unwrap();
    for frame in &server[..3] {
        host(&mut app, frame);
    }
    assert_eq!(app.session().last_sequence(), 4);
    assert_eq!(app.session().resume_token(), Some("r4"));

    assert_eq!(key(&mut app, Key::BackTab), None);
    assert_eq!(app.keys().prompt.focus, Focus::Document);
    assert_eq!(
        key(&mut app, Key::Char('l')),
        Some(Step::Send(client[2].clone()))
    );
    host(&mut app, &server[3]);

    assert_eq!(key(&mut app, Key::Tab), None);
    assert_eq!(app.keys().prompt.focus, Focus::Chat);
    for c in "hello".chars() {
        assert_eq!(key(&mut app, Key::Char(c)), None);
    }
    assert_eq!(
        key(&mut app, Key::Enter),
        Some(Step::Send(client[3].clone()))
    );
    host(&mut app, &server[4]);
    assert_eq!(app.session().last_sequence(), 6);

    let answer = r#"{"action":"submit","values":{"when":"now"}}"#;
    assert_eq!(key(&mut app, Key::Paste(answer.to_owned())), None);
    assert_eq!(
        key(&mut app, Key::Enter),
        Some(Step::Send(client[4].clone()))
    );
    assert!(app.keys().prompt.text.is_empty());
    host(&mut app, &server[5]);
    assert!(
        id_tui_shell::form::active(app.session()).is_none(),
        "an accepted form is no longer in force"
    );

    let chat = app.panels().chat.join("\n");
    assert!(chat.contains("ann: hello"), "{chat}");
}

#[test]
fn a_closed_stream_reconnects_with_the_recorded_resume() {
    let (_, server) = recorded("tui.jsonl");
    let (client, replay) = recorded("tui-reconnect.jsonl");
    let mut app = App::new(CAPABILITY);
    app.opening_frames(80, 24).unwrap();
    for frame in &server {
        host(&mut app, frame);
    }

    assert_eq!(app.handle(Input::HostClosed), Some(Step::Reconnect));
    assert_eq!(app.keys().prompt.status.as_deref(), Some(RECONNECTING));
    assert_eq!(app.join_frame().unwrap(), client[0]);
    assert_eq!(app.opening_frames(80, 24).unwrap()[0], client[0]);

    host(&mut app, &replay[0]);
    assert_eq!(
        app.keys().prompt.status,
        None,
        "the next host frame clears it"
    );
    host(&mut app, &replay[1]);
    assert_eq!(app.session().resume_token(), Some("r7"));
    assert!(!app.session().diverged());

    for c in "back".chars() {
        key(&mut app, Key::Char(c));
    }
    assert_eq!(
        key(&mut app, Key::Enter),
        Some(Step::Send(client[1].clone()))
    );
}

#[test]
fn a_refusal_ends_the_session() {
    let mut app = App::new(CAPABILITY);
    app.opening_frames(80, 24).unwrap();
    let step = app.handle(Input::Host(
        json!({"type": "error", "message": "admin denied"}),
    ));
    assert_eq!(step, Some(Step::End("admin denied".to_owned())));
}

#[test]
fn a_frame_the_contract_rejects_shows_as_status_and_keeps_going() {
    let mut app = App::new(CAPABILITY);
    app.opening_frames(80, 24).unwrap();
    assert_eq!(app.handle(Input::Host(json!({"type": "nonsense"}))), None);
    assert!(app.keys().prompt.status.is_some());
}

#[test]
fn ctrl_c_quits() {
    let mut app = App::new(CAPABILITY);
    assert_eq!(key(&mut app, Key::CtrlC), Some(Step::Quit));
}
