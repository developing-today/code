//! Capability-statement check (issue #10928, shells v1 slice 2): the
//! recorded statement in `capability-statement.json` at the crate root
//! must match ground truth — the F13.1 contract
//! (`pkgs/id/contract/shell-contract.v1.json`), the frame builders this
//! crate actually ships, the server frames [`SessionState`] actually
//! folds, and the providers the [`ProviderAdapter`] actually offers.
//! If the shell's surface changes, this test fails until the statement
//! is updated to record the new truth; that is the point of recording
//! it (design `doc/2026-11-10T20-40-00Z_design_more_shells_v1`,
//! "The one gate": a shell is specified as a capability statement
//! against the contract).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::PathBuf;

use id::world_presentation::WorldSurface;
use id_native_shell::form::{FormPresenter, form_result_frame};
use id_native_shell::{
    CLIENT_FILE_READ_PROVIDER, LocalBounds, ProviderAdapter, ServerUpdate, SessionState,
    agent_join_frame, chat_frame, input_frame, input_frame_with_revision, join_frame, paste_frame,
    resize_frame, screen_subscribe_frame,
};
use serde_json::{Map, Value, json};

fn read_json(relative: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative);
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn statement() -> Value {
    read_json("capability-statement.json")
}

fn contract() -> Value {
    read_json("../../contract/shell-contract.v1.json")
}

fn kind_set(statement: &Value, key: &str) -> BTreeSet<String> {
    statement[key]
        .as_array()
        .unwrap_or_else(|| panic!("statement {key} is an array"))
        .iter()
        .map(|kind| {
            kind.as_str()
                .unwrap_or_else(|| panic!("statement {key} entries are strings"))
                .to_owned()
        })
        .collect()
}

fn contract_kinds(contract: &Value, key: &str) -> BTreeSet<String> {
    contract[key]
        .as_object()
        .unwrap_or_else(|| panic!("contract {key} is an object"))
        .keys()
        .cloned()
        .collect()
}

fn assert_sorted_unique(statement: &Value, key: &str) {
    let entries: Vec<&str> = statement[key]
        .as_array()
        .expect("array")
        .iter()
        .map(|kind| kind.as_str().expect("string"))
        .collect();
    let mut sorted = entries.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        entries, sorted,
        "statement {key} must be sorted and unique so diffs stay reviewable"
    );
}

#[test]
fn statement_is_well_formed_and_names_the_contract() {
    let statement = statement();
    let contract = contract();
    assert_eq!(statement["statement"], json!("shell-capability-statement"));
    assert_eq!(statement["shell"], json!("id-native-shell"));
    assert_eq!(statement["platform"], json!("linux-desktop"));
    assert_eq!(
        statement["contract"]["name"], contract["contract"],
        "statement must name the contract it is a statement against"
    );
    assert_eq!(statement["contract"]["version"], contract["version"]);
    assert!(
        statement["posture"]
            .as_str()
            .expect("posture")
            .contains("local-first"),
        "a v1 shell statement records the local-first posture"
    );
    let excludes = statement["excludes"].as_array().expect("excludes array");
    assert!(!excludes.is_empty(), "exclusions are named, not implied");
    for exclusion in excludes {
        assert!(
            !exclusion.as_str().expect("exclusion string").is_empty(),
            "each exclusion is a named statement"
        );
    }
    assert_sorted_unique(&statement, "sends");
    assert_sorted_unique(&statement, "presents");
}

/// Every client frame kind this shell can emit, produced by calling the
/// builders and session paths that emit it — the oracle for `sends`.
fn builder_sent_kinds() -> BTreeSet<String> {
    let mut kinds = BTreeSet::new();
    let mut record = |frame: &Value| {
        kinds.insert(
            frame["type"]
                .as_str()
                .expect("built frame has a type")
                .to_owned(),
        );
    };
    record(&join_frame("capability", None));
    record(&agent_join_frame("capability", None));
    record(&input_frame(b"x"));
    record(&input_frame_with_revision(b"x", 1));
    record(&paste_frame("hello").expect("paste within bounds"));
    record(&resize_frame(80, 24).expect("resize within bounds"));
    record(&chat_frame("hello"));
    record(&screen_subscribe_frame(
        Some((0, 0, 10, 2)),
        Some(&["chat_posted"]),
    ));

    // form_result: a trusted form surface presented, then answered.
    let surface: WorldSurface = serde_json::from_value(json!({
        "kind": "form",
        "version": 1,
        "form_id": "signup",
        "title": "Sign up",
        "presentation_token": "tok-1",
        "fields": [
            {"kind": "text", "id": "name", "label": "Name", "required": true, "max_length": 40}
        ],
        "actions": [
            {"id": "submit", "label": "Send"},
            {"id": "cancel", "label": "Cancel", "kind": "cancel"}
        ]
    }))
    .expect("form surface parses");
    let mut presenter = FormPresenter::new();
    presenter
        .present(Some(&surface))
        .expect("trusted form presents");
    let form = presenter.active().expect("form is active");
    record(&form_result_frame(form, "submit", &Map::new()).expect("form result builds"));

    // provider_offer / provider_reply: the adapter with the file-read
    // grant in force for one world.
    let mut adapter = ProviderAdapter::new();
    adapter
        .grant_file_read(
            "world",
            std::env::temp_dir(),
            LocalBounds::DESIGN_DEFAULT,
            true,
        )
        .expect("file-read grant");
    let offer = adapter.offer_frame("world").expect("a grant is offered");
    let offered: Vec<&str> = offer["providers"]
        .as_array()
        .expect("offer providers")
        .iter()
        .map(|provider| provider.as_str().expect("provider id"))
        .collect();
    assert_eq!(
        offered,
        vec![CLIENT_FILE_READ_PROVIDER],
        "the offer names exactly the declared providers"
    );
    record(&offer);
    let reply = adapter.handle_request(
        "world",
        &json!({
            "type": "provider_request", "nonce": "ab12",
            "provider": CLIENT_FILE_READ_PROVIDER,
            "args": {"path": "notes.txt"}, "deadline_ms": 5000
        }),
        1_000,
    );
    record(&reply);
    kinds
}

#[test]
fn statement_sends_exactly_what_the_builders_emit() {
    let statement = statement();
    let contract = contract();
    let stated = kind_set(&statement, "sends");
    let client_frames = contract_kinds(&contract, "client_frames");
    for kind in &stated {
        assert!(
            client_frames.contains(kind),
            "statement sends {kind}, which the contract does not declare as a client frame"
        );
    }
    assert_eq!(
        stated,
        builder_sent_kinds(),
        "statement sends must equal the kinds the shell's builders and provider adapter emit"
    );
}

/// Minimal contract-valid frames for each presented kind, with the
/// session fold each must produce — the oracle for `presents`.
fn presented_frames() -> Vec<(Value, ServerUpdate)> {
    vec![
        (
            json!({"type": "snapshot", "snapshot": {"sequence": 4}}),
            ServerUpdate::Snapshot,
        ),
        (
            json!({"type": "event", "event": {"sequence": 5, "kind": {"kind": "input"}}}),
            ServerUpdate::Event,
        ),
        (
            json!({"type": "view", "sequence": 5, "data": "fallback", "view_revision": 0}),
            ServerUpdate::View,
        ),
        (
            json!({
                "type": "semantic_diff", "sequence": 1, "view_revision": 0,
                "changes": [{"kind": "chat_posted", "participant_id": 2, "text": "hi there"}]
            }),
            ServerUpdate::SemanticDiff,
        ),
        (
            json!({
                "type": "screen_subscription",
                "region": {"x": 0, "y": 0, "width": 10, "height": 2},
                "event_classes": ["chat_posted"], "view_revision": 0
            }),
            ServerUpdate::ScreenSubscription,
        ),
        (
            json!({
                "type": "form_result_status", "form_id": "signup",
                "ok": false, "presentation_token": "tok-2"
            }),
            ServerUpdate::FormResultStatus,
        ),
        (
            json!({"type": "program_changed", "module_hash": "blake3:new", "sequence": 5}),
            ServerUpdate::ProgramChanged,
        ),
        (
            json!({
                "type": "provider_request", "nonce": "ab12",
                "provider": CLIENT_FILE_READ_PROVIDER,
                "args": {"path": "notes.txt"}, "deadline_ms": 5000
            }),
            ServerUpdate::ProviderRequest,
        ),
    ]
}

#[test]
fn statement_presents_exactly_what_the_session_folds() {
    let statement = statement();
    let contract = contract();
    let stated = kind_set(&statement, "presents");
    let server_frames = contract_kinds(&contract, "server_frames");
    for kind in &stated {
        assert!(
            server_frames.contains(kind),
            "statement presents {kind}, which the contract does not declare as a server frame"
        );
    }

    let mut state = SessionState::default();
    let mut folded = BTreeSet::new();
    for (frame, expected) in presented_frames() {
        let kind = frame["type"].as_str().expect("frame type").to_owned();
        assert_eq!(
            state
                .apply_server_frame(frame)
                .expect("presented frame folds"),
            expected,
            "{kind} folds as its named update"
        );
        folded.insert(kind);
    }
    // `error` is presented as a session refusal, not folded as content.
    let error = state
        .apply_server_frame(json!({"type": "error", "message": "nope", "code": "refused"}))
        .expect_err("an error frame refuses the session");
    assert!(
        error.to_string().contains("refused the session"),
        "error surfaces as a refusal, got: {error}"
    );
    folded.insert("error".to_owned());
    assert_eq!(
        stated, folded,
        "statement presents must equal the kinds the session state folds (plus error-as-refusal)"
    );

    // The fold is closed: contract server frames the statement does not
    // claim are rejected, never silently absorbed.
    for frame in [
        json!({"type": "module_info", "world_id": "w", "current_sequence": 1}),
        json!({"type": "worlds", "worlds": []}),
        json!({"type": "caps", "caps": {}}),
    ] {
        assert!(
            state.apply_server_frame(frame.clone()).is_err(),
            "{} is not in the statement and must be rejected",
            frame["type"]
        );
    }
}

#[test]
fn statement_providers_match_the_adapter() {
    let statement = statement();
    let providers = statement["providers"].as_array().expect("providers array");
    let ids: Vec<&str> = providers
        .iter()
        .map(|provider| provider["id"].as_str().expect("provider id"))
        .collect();
    assert_eq!(
        ids,
        vec![CLIENT_FILE_READ_PROVIDER],
        "the statement declares exactly the providers the adapter can offer"
    );
    for provider in providers {
        assert!(
            !provider["declaration"]
                .as_str()
                .expect("declaration")
                .is_empty(),
            "each provider records how it is declared"
        );
    }
}
