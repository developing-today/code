//! Transcript fixture check: the recorded client frames in
//! `tests/fixtures/transcript_step1.json` must match the shapes the
//! machine-readable F13.1 contract (`pkgs/id/contract/shell-contract.v1.json`)
//! declares — same frame kinds, same fields, values inside the
//! contract's bounds. This is the step-1 stand-in for the live
//! `id world contract --check` session harness (step 2).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id::shell_contract::{ShellContract, check_transcript};
use id_native_shell::{
    SessionState, chat_frame, input_frame, join_frame, paste_frame, resize_frame,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;

fn read_json(relative: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative);
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

#[test]
fn transcript_frames_match_contract_shapes() {
    let contract = read_json("../../contract/shell-contract.v1.json");
    let transcript = read_json("tests/fixtures/transcript_step1.json");
    let client_frames = contract["client_frames"]
        .as_object()
        .expect("contract client_frames");
    let server_frames = contract["server_frames"]
        .as_object()
        .expect("contract server_frames");

    let frames = transcript["frames"].as_array().expect("transcript frames");
    assert!(
        !frames.is_empty(),
        "transcript must record at least one frame"
    );
    for frame in frames {
        let kind = frame["type"].as_str().expect("frame type");
        let spec = client_frames
            .get(kind)
            .or_else(|| server_frames.get(kind))
            .unwrap_or_else(|| panic!("frame kind {kind} is not a contract frame"));
        let allowed: BTreeSet<&str> = spec["fields"]
            .as_object()
            .expect("contract fields")
            .keys()
            .map(String::as_str)
            .collect();
        for key in frame.as_object().expect("frame object").keys() {
            if key == "type" {
                continue;
            }
            assert!(
                allowed.contains(key.as_str()),
                "frame {kind} carries field {key} the contract does not declare"
            );
        }
        // Bounds the contract states numerically must hold for recorded frames.
        if kind == "paste" {
            let max = spec["fields"]["text"]["max_bytes"]
                .as_u64()
                .expect("paste bound") as usize;
            assert!(frame["text"].as_str().expect("paste text").len() <= max);
        }
        if kind == "resize" {
            for field in ["width", "height"] {
                let bound = &spec["fields"][field];
                let value = frame[field].as_u64().expect("resize value");
                assert!(
                    value >= bound["min"].as_u64().expect("min")
                        && value <= bound["max"].as_u64().expect("max")
                );
            }
        }
        if kind == "input" {
            let hex = frame["data_hex"].as_str().expect("input data_hex");
            assert!(
                hex.len() % 2 == 0 && hex.chars().all(|c| c.is_ascii_hexdigit()),
                "data_hex must be hex"
            );
        }
    }
}

#[test]
fn transcript_records_a_view_with_data() {
    let transcript = read_json("tests/fixtures/transcript_step1.json");
    let views: Vec<&Value> = transcript["frames"]
        .as_array()
        .expect("frames")
        .iter()
        .filter(|f| f["type"] == "view")
        .collect();
    assert_eq!(
        views.len(),
        1,
        "step-1 transcript records exactly one host view"
    );
    assert!(
        views[0]["data"]
            .as_str()
            .expect("view data")
            .contains("welcome")
    );
}

#[test]
fn native_session_builders_pass_the_f13_1_contract_checker() {
    let mut state = SessionState::default();
    let mut lines = Vec::new();
    let mut record = |from: &str, frame: Value| {
        lines.push(json!({"from": from, "frame": frame}).to_string());
    };

    record("client", join_frame("capability", None));
    for (frame, expected) in [
        (
            json!({"type":"snapshot", "snapshot":{"sequence":0}}),
            id_native_shell::ServerUpdate::Snapshot,
        ),
        (
            json!({"type":"event", "event":{"sequence":1,"kind":{"kind":"input"}}}),
            id_native_shell::ServerUpdate::Event,
        ),
        (
            json!({"type":"view", "sequence":1, "data":"welcome", "view_revision":0}),
            id_native_shell::ServerUpdate::View,
        ),
    ] {
        assert_eq!(state.apply_server_frame(frame.clone()).unwrap(), expected);
        record("server", frame);
    }

    record("client", input_frame(b"look\r"));
    record("client", paste_frame("a pasted line").unwrap());
    record("client", resize_frame(80, 24).unwrap());
    record("client", chat_frame("hello from the native shell"));

    let transcript = lines.join("\n");
    let contract = ShellContract::embedded().expect("embedded F13.1 contract");
    let violations = check_transcript(&contract, &transcript).expect("parse transcript");
    assert!(violations.is_empty(), "{violations:#?}\n{transcript}");
    assert_eq!(state.view().unwrap().text_projection(), "welcome");
}

#[test]
fn agent_profile_transcript_conforms_and_semantic_diffs_render() {
    // Agent profile (contract revision 11, #6300 sections 4.2/4.3/8):
    // an agent join, a region + event-class subscription, its
    // acknowledgement, semantic diffs, and an input carrying the
    // stale-decision token all conform to the same F13.1 contract, and
    // the native shell renders a semantic diff as world-term lines.
    let mut state = SessionState::default();
    let mut lines = Vec::new();
    let mut record = |from: &str, frame: Value| {
        lines.push(json!({"from": from, "frame": frame}).to_string());
    };

    record(
        "client",
        id_native_shell::agent_join_frame("capability", None),
    );
    for (frame, expected) in [
        (
            json!({"type":"snapshot", "snapshot":{"sequence":0}}),
            id_native_shell::ServerUpdate::Snapshot,
        ),
        (
            json!({"type":"view", "sequence":0, "data":"hello ada", "view_revision":0}),
            id_native_shell::ServerUpdate::View,
        ),
    ] {
        assert_eq!(state.apply_server_frame(frame.clone()).unwrap(), expected);
        record("server", frame);
    }

    record(
        "client",
        id_native_shell::screen_subscribe_frame(Some((0, 0, 10, 2)), Some(&["chat_posted"])),
    );
    let subscription = json!({
        "type": "screen_subscription",
        "region": {"x": 0, "y": 0, "width": 10, "height": 2},
        "event_classes": ["chat_posted"],
        "view_revision": 0
    });
    assert_eq!(
        state.apply_server_frame(subscription.clone()).unwrap(),
        id_native_shell::ServerUpdate::ScreenSubscription
    );
    record("server", subscription);
    assert_eq!(
        state.screen_subscription().unwrap()["view_revision"],
        json!(0)
    );

    let diff = json!({
        "type": "semantic_diff", "sequence": 1, "view_revision": 0,
        "changes": [
            {"kind": "chat_posted", "participant_id": 2, "text": "hi there"},
            {"kind": "region_changed", "row": 1, "text": "agent here"}
        ]
    });
    assert_eq!(
        state.apply_server_frame(diff.clone()).unwrap(),
        id_native_shell::ServerUpdate::SemanticDiff
    );
    record("server", diff);
    assert_eq!(state.semantic_diffs().len(), 1);
    assert_eq!(
        id_native_shell::semantic_diff_text(&state.semantic_diffs()[0]),
        "chat: hi there\nrow 1: agent here"
    );

    record(
        "client",
        id_native_shell::input_frame_with_revision(b"look\r", 0),
    );

    let transcript = lines.join("\n");
    let contract = ShellContract::embedded().expect("embedded F13.1 contract");
    let violations = check_transcript(&contract, &transcript).expect("parse transcript");
    assert!(violations.is_empty(), "{violations:#?}\n{transcript}");
}
