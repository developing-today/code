//! The TUI shell's recorded sessions under the shell conformance
//! checks (issue #12818, "done = conformance on its recorded
//! session").
//!
//! Two sessions are committed under `contract/sessions/`:
//!
//! - `tui.jsonl`: a fresh connection through the text-native
//!   workflow. The shell joins, folds the snapshot and a presented
//!   form, reads records, acts (key input and chat) and answers the
//!   form, folding each host reply.
//! - `tui-reconnect.jsonl`: the next connection after that one
//!   dropped. The shell presents its last applied sequence and the
//!   host's resume token, folds the contiguous replay without
//!   divergence, and acts again.
//!
//! Each session must be exactly what this crate's frame builders and
//! session fold produce, and each must pass the suite's checks
//! (`check_suite`): every frame inside the F13.1 contract, every sent
//! frame inside the capability statement's `sends`, and every
//! workflow frame exercised. The suite manifest
//! (`contract/shell-suite.v1.json`) carries the TUI's entry with the
//! fresh session (issue #12821); its statement must say what this
//! crate's statement says.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::PathBuf;

use id::shell_contract::{
    CapabilityStatement, SessionSource, ShellContract, ShellSuite, SuiteEntry, check_suite,
};
use id_tui_shell::frames;
use id_tui_shell::session::{Session, Update};
use serde_json::{Map, Value, json};

const CAPABILITY: &str = "tui-capability";
const TOKEN: &str = "0123456789abcdef0123456789abcdef";

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn session_path(name: &str) -> PathBuf {
    manifest_dir().join("../../contract/sessions").join(name)
}

fn statement() -> Value {
    let path = manifest_dir().join("capability-statement.json");
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

/// The crate's statement in the suite's terms, with the client frames
/// a given session's workflow must exercise.
fn suite_statement(workflow: &[&str]) -> CapabilityStatement {
    let statement = statement();
    CapabilityStatement {
        sends: kinds(&statement, "sends"),
        presents: kinds(&statement, "presents"),
        providers: kinds(&statement, "providers").into_iter().collect(),
        workflow: workflow.iter().map(|kind| (*kind).to_owned()).collect(),
    }
}

struct Recorder {
    session: Session,
    transcript: Vec<Value>,
}

impl Recorder {
    fn new() -> Self {
        Self {
            session: Session::new(),
            transcript: Vec::new(),
        }
    }

    fn send(&mut self, frame: Value) {
        self.transcript
            .push(json!({"from": "client", "frame": frame}));
    }

    fn fold(&mut self, frame: Value) -> Update {
        let update = self.session.apply(&frame).unwrap();
        self.transcript
            .push(json!({"from": "server", "frame": frame}));
        update
    }

    fn take(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.transcript)
    }
}

fn form_view() -> Value {
    json!({
        "type": "view",
        "sequence": 4,
        "data": "Ship the update?",
        "view_revision": 1,
        "resume_token": "r4",
        "surface": {
            "kind": "form",
            "version": 1,
            "form_id": "tui-choice",
            "presentation_token": TOKEN,
            "title": "Ship the update?",
            "fields": [{
                "kind": "choice",
                "id": "when",
                "label": "When",
                "required": true,
                "options": ["now", "later"],
            }],
            "actions": [
                {"id": "submit", "label": "Confirm"},
                {"id": "cancel", "label": "Cancel", "kind": "cancel"},
            ],
        },
    })
}

/// Drive the shell through a fresh connection and then a reconnect,
/// returning each connection's transcript.
fn record_sessions() -> (Vec<Value>, Vec<Value>) {
    let mut tui = Recorder::new();

    // Join fresh: no history position, no resume token.
    tui.send(frames::join(CAPABILITY, None, None).unwrap());
    let update = tui.fold(json!({
        "type": "snapshot",
        "snapshot": {
            "world_id": "lobby",
            "current_sequence": 4,
            "participants": [{"id": 1, "display_name": "ann"}],
            "events": [],
        },
    }));
    assert_eq!(update, Update::Snapshot);
    assert_eq!(tui.session.last_sequence(), 4);

    // Read: the presented view (a form, rendered as a text panel)
    // and the room records.
    assert_eq!(tui.fold(form_view()), Update::View);
    assert_eq!(tui.session.resume_token(), Some("r4"));
    tui.send(frames::records(CAPABILITY, Some("room/"), None).unwrap());
    let update = tui.fold(json!({
        "type": "records",
        "sequence": 4,
        "records": {"room/lobby": "Lobby"},
        "next_after": null,
    }));
    assert_eq!(update, Update::Records);
    assert!(tui.session.records().is_some());

    // Act: a key, a chat line, and the answer to the presented form.
    tui.send(frames::input(b"l", None).unwrap());
    let update = tui.fold(json!({
        "type": "event",
        "event": {"sequence": 5, "participant_id": 1, "kind": {"kind": "input", "data": [108]}},
    }));
    assert_eq!(update, Update::Event);
    tui.send(frames::chat("hello", None).unwrap());
    let update = tui.fold(json!({
        "type": "event",
        "event": {"sequence": 6, "participant_id": 1, "kind": {"kind": "chat", "data": "hello"}},
    }));
    assert_eq!(update, Update::Event);
    assert_eq!(tui.session.events().len(), 2);
    assert_eq!(tui.session.last_sequence(), 6);

    let mut values = Map::new();
    values.insert("when".to_owned(), json!("now"));
    tui.send(frames::form_result("tui-choice", TOKEN, "submit", &values, None).unwrap());
    let update =
        tui.fold(json!({"type": "form_result_status", "form_id": "tui-choice", "ok": true}));
    assert_eq!(update, Update::FormResultStatus);
    assert_eq!(tui.session.last_form_status().unwrap()["ok"], json!(true));
    let fresh = tui.take();

    // The connection drops. Reconnect from the last applied sequence
    // with the host's resume token (the v1 history-identity rule).
    tui.session.begin_reconnect();
    tui.send(
        frames::join(
            CAPABILITY,
            Some(tui.session.last_sequence()),
            tui.session.resume_token(),
        )
        .unwrap(),
    );
    let update = tui.fold(json!({
        "type": "event",
        "event": {"sequence": 7, "participant_id": 1, "kind": {"kind": "input", "data": [97]}},
    }));
    assert_eq!(update, Update::Event);
    assert!(!tui.session.diverged(), "a contiguous replay resumes");
    assert_eq!(tui.session.events().len(), 3, "the replay folds in");
    let update = tui.fold(json!({
        "type": "view",
        "sequence": 7,
        "data": "Lobby",
        "view_revision": 2,
        "resume_token": "r7",
    }));
    assert_eq!(update, Update::View);
    assert_eq!(tui.session.resume_token(), Some("r7"));
    tui.send(frames::chat("back", None).unwrap());
    let update = tui.fold(json!({
        "type": "event",
        "event": {"sequence": 8, "participant_id": 1, "kind": {"kind": "chat", "data": "back"}},
    }));
    assert_eq!(update, Update::Event);
    assert_eq!(tui.session.last_sequence(), 8);
    let resumed = tui.take();

    (fresh, resumed)
}

fn committed(name: &str, recorded: &[Value]) -> String {
    let rendered: Vec<String> = recorded.iter().map(ToString::to_string).collect();
    let text = std::fs::read_to_string(session_path(name)).unwrap_or_else(|err| {
        panic!(
            "read {}: {err}; the TUI session is:\n{}",
            session_path(name).display(),
            rendered.join("\n")
        )
    });
    let lines: Vec<Value> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("session line parses"))
        .collect();
    assert_eq!(
        lines,
        recorded,
        "{name} drifted; the TUI session is now:\n{}",
        rendered.join("\n")
    );
    text
}

#[test]
fn the_recorded_sessions_are_what_this_shell_sends_and_folds() {
    let (fresh, resumed) = record_sessions();
    committed("tui.jsonl", &fresh);
    committed("tui-reconnect.jsonl", &resumed);
}

#[test]
fn the_recorded_sessions_pass_the_suite_checks() {
    let (fresh, resumed) = record_sessions();
    let contract = ShellContract::embedded().unwrap();
    let fresh_text = committed("tui.jsonl", &fresh);
    let resumed_text = committed("tui-reconnect.jsonl", &resumed);

    // The fresh session must exercise the whole join, read and act
    // workflow; the reconnect must at least rejoin.
    let fresh_statement = suite_statement(&["chat", "form_result", "input", "join", "records"]);
    let resumed_statement = suite_statement(&["chat", "join"]);
    for (name, statement, text) in [
        ("tui", &fresh_statement, fresh_text.as_str()),
        ("tui-reconnect", &resumed_statement, resumed_text.as_str()),
    ] {
        let verdicts = check_suite(
            contract,
            &[SuiteEntry {
                name,
                statement: Some(statement),
                session: SessionSource::Recorded(text),
            }],
        );
        assert_eq!(verdicts.len(), 1);
        let verdict = &verdicts[0];
        assert!(verdict.recorded, "{name} was checked as recorded");
        assert!(
            verdict.passed(),
            "{name} failed the suite: {:?}",
            verdict.failures
        );
        assert_eq!(
            verdict.frames,
            text.lines().filter(|l| !l.trim().is_empty()).count()
        );
    }
}

#[test]
fn the_statement_names_the_workflow_these_sessions_cover() {
    let statement = statement();
    assert_eq!(
        kinds(&statement, "workflow"),
        BTreeSet::from([
            "act".to_owned(),
            "join".to_owned(),
            "read".to_owned(),
            "reconnect".to_owned(),
        ])
    );
}

#[test]
fn the_suite_manifest_states_this_shells_capabilities() {
    let suite = ShellSuite::embedded().unwrap();
    let entry = suite
        .shells
        .iter()
        .find(|shell| shell.name == "tui")
        .expect("tui entry in the suite manifest");
    let manifest = entry.statement.as_ref().expect("tui capability statement");
    let crate_statement = statement();
    assert_eq!(manifest.sends, kinds(&crate_statement, "sends"));
    assert_eq!(manifest.presents, kinds(&crate_statement, "presents"));
    assert_eq!(
        manifest.providers.iter().cloned().collect::<BTreeSet<_>>(),
        kinds(&crate_statement, "providers")
    );
    // The manifest's workflow is the fresh session's: join, read
    // (records) and act (chat, input, form_result).
    assert_eq!(
        manifest.workflow,
        suite_statement(&["chat", "form_result", "input", "join", "records"]).workflow
    );
    assert_eq!(
        entry.session.as_deref(),
        Some("sessions/tui.jsonl"),
        "the manifest names the fresh TUI session"
    );
}
