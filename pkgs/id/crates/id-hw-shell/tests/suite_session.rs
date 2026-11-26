//! The hardware shell's recorded session in the v1 shell
//! conformance suite (#10931).
//!
//! The suite manifest (`contract/shell-suite.v1.json`) names each
//! v1 shell's capability statement and recorded session. These tests
//! pin the hardware entry to this crate: the manifest's statement must
//! say what [`capability_statement`] says, and the committed session
//! (`contract/sessions/hardware.jsonl`) must be exactly the frames
//! this shell builds and folds through its required workflow: a wake
//! that joins, reads presence from the snapshot, is presented a
//! choice, confirms it, and folds the host's status reply and the
//! next durable event. Either drifting from the code fails here.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use id::shell_contract::ShellSuite;
use id_hw_shell::{HwShell, HwUpdate, capability_statement};
use serde_json::{Value, json};

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

fn session_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contract/sessions/hardware.jsonl")
}

fn names(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .expect("statement list")
        .iter()
        .map(|name| name.as_str().expect("frame name").to_owned())
        .collect()
}

/// Drive the shell through its required workflow and record every
/// frame it sends and every host frame it folds, in order.
fn record_workflow() -> Vec<Value> {
    let mut shell = HwShell::new();
    let mut transcript = Vec::new();
    let mut fold = |shell: &mut HwShell, transcript: &mut Vec<Value>, frame: Value| {
        let update = shell.apply_server_frame(&frame).unwrap();
        transcript.push(json!({"from": "server", "frame": frame}));
        update
    };

    let join = shell.join_frame("hw-capability").unwrap();
    transcript.push(json!({"from": "client", "frame": join}));
    let update = fold(
        &mut shell,
        &mut transcript,
        json!({
            "type": "snapshot",
            "snapshot": {
                "world_id": "lobby",
                "current_sequence": 4,
                "participants": [{"id": 1, "display_name": "ann"}],
            },
        }),
    );
    assert_eq!(update, HwUpdate::Snapshot);
    assert!(shell.presence().contains("ann"));

    let update = fold(
        &mut shell,
        &mut transcript,
        json!({
            "type": "view",
            "sequence": 4,
            "data": "Ship the update?",
            "view_revision": 1,
            "surface": {
                "kind": "form",
                "version": 1,
                "form_id": "hw-choice",
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
        }),
    );
    assert_eq!(update, HwUpdate::View);
    assert!(shell.choice().is_some());

    let selections = json!({"when": "now"}).as_object().unwrap().clone();
    let answer = shell.confirm("submit", &selections).unwrap();
    transcript.push(json!({"from": "client", "frame": answer}));

    let update = fold(
        &mut shell,
        &mut transcript,
        json!({"type": "form_result_status", "form_id": "hw-choice", "ok": true}),
    );
    assert_eq!(update, HwUpdate::FormResultStatus);
    assert_eq!(shell.last_status(), Some((true, None)));

    let update = fold(
        &mut shell,
        &mut transcript,
        json!({
            "type": "event",
            "event": {
                "sequence": 5,
                "participant_id": 1,
                "kind": {"kind": "input", "data": [112, 105, 110, 103]},
            },
        }),
    );
    assert_eq!(update, HwUpdate::Event);
    assert_eq!(shell.head(), 5);
    transcript
}

#[test]
fn the_suite_manifest_states_this_shells_capabilities() {
    let suite = ShellSuite::embedded().unwrap();
    let entry = suite
        .shells
        .iter()
        .find(|shell| shell.name == "hardware")
        .expect("hardware entry in the suite manifest");
    let manifest = entry
        .statement
        .as_ref()
        .expect("hardware capability statement");
    let crate_statement = capability_statement();
    assert_eq!(manifest.sends, names(&crate_statement["frames_sent"]));
    assert_eq!(
        manifest.presents,
        names(&crate_statement["server_frames_presented"])
    );
    assert_eq!(
        manifest.providers.iter().cloned().collect::<BTreeSet<_>>(),
        names(&crate_statement["providers_declared"])
    );
    // The required workflow is the whole posture: join on wake, and
    // the one deliberate act, confirming a presented choice.
    assert_eq!(
        manifest.workflow.iter().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from(["form_result".to_owned(), "join".to_owned()])
    );
    assert_eq!(
        entry.session.as_deref(),
        Some("sessions/hardware.jsonl"),
        "the manifest names the recorded hardware session"
    );
}

#[test]
fn the_recorded_session_is_what_this_shell_sends_and_folds() {
    let recorded = record_workflow();
    let rendered: Vec<String> = recorded.iter().map(ToString::to_string).collect();
    let text = std::fs::read_to_string(session_path()).unwrap_or_else(|err| {
        panic!(
            "read {}: {err}; the hardware shell's session is:\n{}",
            session_path().display(),
            rendered.join("\n")
        )
    });
    let committed: Vec<Value> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("session line parses"))
        .collect();
    assert_eq!(
        committed,
        recorded,
        "hardware session drifted; the hardware shell's session is now:\n{}",
        rendered.join("\n")
    );
}
