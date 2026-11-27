//! Capability-statement check (issue #12818, shells v2 slice 2):
//! the recorded statement in `capability-statement.json` at the
//! crate root must match ground truth — the F13.1 contract
//! (`pkgs/id/contract/shell-contract.v1.json`) and the workflow
//! the issue names (join, read, act, reconnect under the v1
//! history-identity rule, at parity with the browser shell's
//! text surface).
//!
//! Decision D3 of the v2 outline records this statement *before*
//! implementation starts, so at this rung the pins are against
//! the contract and the issue: every claimed frame kind exists
//! in the contract, the claimed sets cover the workflow and
//! claim nothing beyond the text surface, and the exclusions
//! are named. When feature completion lands the frame builders
//! and the session fold in this crate, these tests grow the
//! same builder/fold oracles the native shell's statement check
//! carries (#10928) — a shell claiming frames it cannot present
//! is the failure mode the design exists to prevent.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::{Value, json};

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
    assert_eq!(statement["shell"], json!("id-tui-shell"));
    assert_eq!(statement["platform"], json!("terminal"));
    assert_eq!(
        statement["contract"]["name"], contract["contract"],
        "statement must name the contract it is a statement against"
    );
    assert_eq!(statement["contract"]["version"], contract["version"]);
    assert_eq!(
        id_tui_shell::CONTRACT_NAME,
        contract["contract"].as_str().expect("contract name"),
        "the crate's contract constant names the same contract"
    );
    assert_eq!(
        id_tui_shell::CONTRACT_VERSION,
        contract["version"].as_u64().expect("contract version"),
        "the crate's contract version matches the contract file"
    );
    assert!(
        statement["posture"]
            .as_str()
            .expect("posture")
            .contains("local-first"),
        "a shell statement records the local-first posture"
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

#[test]
fn embedded_statement_is_the_recorded_file() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("capability-statement.json");
    let on_disk = std::fs::read_to_string(&path).expect("read recorded statement");
    assert_eq!(
        id_tui_shell::CAPABILITY_STATEMENT_JSON,
        on_disk,
        "the statement embedded in the crate is the recorded file, verbatim"
    );
    assert_eq!(
        id_tui_shell::capability_statement().expect("embedded statement parses"),
        statement(),
        "the crate's parsed statement is the recorded statement"
    );
}

#[test]
fn statement_frames_exist_in_the_contract() {
    let statement = statement();
    let contract = contract();
    let client_frames = contract_kinds(&contract, "client_frames");
    for kind in kind_set(&statement, "sends") {
        assert!(
            client_frames.contains(&kind),
            "statement sends {kind}, which the contract does not declare as a client frame"
        );
    }
    let server_frames = contract_kinds(&contract, "server_frames");
    for kind in kind_set(&statement, "presents") {
        assert!(
            server_frames.contains(&kind),
            "statement presents {kind}, which the contract does not declare as a server frame"
        );
    }
}

#[test]
fn statement_covers_the_issue_workflow() {
    let statement = statement();
    let workflow: Vec<&str> = statement["workflow"]
        .as_array()
        .expect("workflow array")
        .iter()
        .map(|step| step.as_str().expect("workflow step"))
        .collect();
    assert_eq!(
        workflow,
        vec!["join", "read", "act", "reconnect"],
        "the statement records the workflow issue #12818 names, in its words"
    );

    let sends = kind_set(&statement, "sends");
    let presents = kind_set(&statement, "presents");
    // join + reconnect: open the session, and re-open it under
    // the history-identity rule (snapshot is the re-join read).
    assert!(sends.contains("join"), "the workflow joins");
    assert!(
        presents.contains("snapshot"),
        "a (re)join presents a snapshot"
    );
    // read: records queries and their replies, plus the live
    // event and view frames the panels render.
    assert!(sends.contains("records"), "read is records queries");
    assert!(
        presents.contains("records"),
        "read presents records replies"
    );
    assert!(presents.contains("event"), "read presents journal events");
    assert!(presents.contains("view"), "read presents views");
    // act: key input, paste, chat, and form answers.
    for kind in ["input", "paste", "chat", "form_result"] {
        assert!(sends.contains(kind), "act sends {kind}");
    }
    assert!(
        presents.contains("form_result_status"),
        "a form answer is presented its status"
    );
    assert!(
        presents.contains("error"),
        "a refusal is presented as error"
    );

    let reconnect = statement["reconnect"].as_str().expect("reconnect rule");
    assert!(
        reconnect.contains("history identity"),
        "reconnect names the v1 history-identity rule"
    );
    assert!(
        reconnect.contains("re-snapshot, never by merge"),
        "divergence resolves by re-snapshot, never by merge"
    );
}

#[test]
fn statement_claims_the_text_surface_not_the_native_gui_surface() {
    let statement = statement();
    let sends = kind_set(&statement, "sends");
    let presents = kind_set(&statement, "presents");
    // The parity anchor is the browser shell's text surface
    // (pkgs/id/web/src/world.ts, world-form.ts). The wider
    // native GUI shell's claims (#10928) are not the TUI's:
    // no screen-region subscription, no provider frames.
    for kind in ["screen_subscribe", "provider_offer", "provider_reply"] {
        assert!(
            !sends.contains(kind),
            "the TUI does not claim {kind}; that is the native GUI shell's surface"
        );
    }
    for kind in ["semantic_diff", "screen_subscription", "provider_request"] {
        assert!(
            !presents.contains(kind),
            "the TUI does not present {kind}; that is the native GUI shell's surface"
        );
    }
    // Nor the browser shell's authoring/hosting surface: the
    // TUI is a world-session client only.
    for kind in ["install_begin", "install_chunk", "install_end", "compile"] {
        assert!(
            !sends.contains(kind),
            "the TUI does not send {kind}; authoring is excluded by name"
        );
    }
}

#[test]
fn statement_declares_no_providers_and_names_its_exclusions() {
    let statement = statement();
    let providers = statement["providers"].as_array().expect("providers array");
    assert!(
        providers.is_empty(),
        "no F13.7 provider is named for the TUI in #1071, the v2 outline, or #12818; \
         the statement declares none rather than inventing one"
    );
    let excludes: Vec<&str> = statement["excludes"]
        .as_array()
        .expect("excludes array")
        .iter()
        .map(|exclusion| exclusion.as_str().expect("exclusion string"))
        .collect();
    // The v2 outline expects rich rendering and media to head
    // the list; the rest are the v1 shells' named exclusions
    // restated for a session client, plus the agent-profile
    // surface this statement does not claim.
    for named in [
        "rich rendering",
        "media",
        "provider offers",
        "world hosting and program authoring",
        "offline reading",
        "agent-profile surfaces",
    ] {
        assert!(
            excludes.iter().any(|exclusion| exclusion.contains(named)),
            "exclusions name {named:?}"
        );
    }
}
