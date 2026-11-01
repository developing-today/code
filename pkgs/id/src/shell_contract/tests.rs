use std::collections::BTreeSet;

use serde_json::json;

use super::*;
use crate::world_session::{ClientFrame, InviteResponse, ServerFrame};

fn contract() -> &'static ShellContract {
    ShellContract::embedded().unwrap()
}

/// Every `type` serde accepts for [`ClientFrame`], read from its
/// unknown-variant error so the list cannot drift from the enum.
fn client_frame_variants() -> BTreeSet<String> {
    let error = serde_json::from_value::<ClientFrame>(json!({"type": "__shell_contract_probe__"}))
        .unwrap_err()
        .to_string();
    let (_, expected) = error.split_once("expected one of").unwrap();
    expected
        .split(',')
        .map(|name| name.trim().trim_matches('`').to_owned())
        .filter(|name| !name.is_empty())
        .collect()
}

/// Exhaustive on purpose: a new [`ServerFrame`] variant does not compile until
/// it is named here, and the test below then needs it in the contract.
fn server_frame_type(frame: &ServerFrame<'_>) -> &'static str {
    match frame {
        ServerFrame::Snapshot { .. } => "snapshot",
        ServerFrame::Event { .. } => "event",
        ServerFrame::View { .. } => "view",
        ServerFrame::ModuleInfo { .. } => "module_info",
        ServerFrame::Metrics { .. } => "metrics",
        ServerFrame::Records { .. } => "records",
        ServerFrame::RecordsTicket { .. } => "records_ticket",
        ServerFrame::ModuleBegin { .. } => "module_begin",
        ServerFrame::ModuleChunk { .. } => "module_chunk",
        ServerFrame::ModuleEnd { .. } => "module_end",
        ServerFrame::Invite(_) => "invite",
        ServerFrame::Worlds { .. } => "worlds",
        ServerFrame::WorldCreated { .. } => "world_created",
        ServerFrame::Isolation { .. } => "isolation",
        ServerFrame::Policy { .. } => "policy",
        ServerFrame::Caps { .. } => "caps",
        ServerFrame::ExecutionProfile { .. } => "execution_profile",
        ServerFrame::CapsCeiling { .. } => "caps_ceiling",
        ServerFrame::Schedules { .. } => "schedules",
        ServerFrame::WorldState { .. } => "world_state",
        ServerFrame::Compiled { .. } => "compiled",
        ServerFrame::UploadReady { .. } => "upload_ready",
        ServerFrame::ModuleInstalled { .. } => "module_installed",
        ServerFrame::ProgramChanged { .. } => "program_changed",
        ServerFrame::EventHistoryBegin { .. } => "event_history_begin",
        ServerFrame::EventHistoryEvent { .. } => "event_history_event",
        ServerFrame::Error { .. } => "error",
    }
}

const SERVER_FRAME_TYPES: &[&str] = &[
    "snapshot",
    "event",
    "view",
    "module_info",
    "metrics",
    "records",
    "records_ticket",
    "module_begin",
    "module_chunk",
    "module_end",
    "invite",
    "worlds",
    "world_created",
    "isolation",
    "policy",
    "caps",
    "execution_profile",
    "caps_ceiling",
    "schedules",
    "world_state",
    "compiled",
    "upload_ready",
    "module_installed",
    "program_changed",
    "event_history_begin",
    "event_history_event",
    "error",
];

#[test]
fn the_embedded_contract_is_version_one_and_matches_the_host_limits() {
    let contract = contract();
    assert_eq!(contract.name, "id-shell");
    assert_eq!(contract.version, SHELL_CONTRACT_VERSION);
    assert_eq!(contract.max_frame_bytes, crate::frames::MAX_FRAME_BYTES);
    assert_eq!(
        contract.transports["iroh"]["alpn"],
        std::str::from_utf8(crate::world_net::WORLD_ALPN).unwrap()
    );
    assert_eq!(
        contract.transports["iroh"]["alpn"].as_str().unwrap(),
        format!("/id-world/{SHELL_CONTRACT_VERSION}")
    );
    let join = &contract.client_frames["join"].fields["capability"];
    assert_eq!(
        join.max_bytes,
        Some(crate::world_session::MAX_CAPABILITY_BYTES)
    );
    let chunk = &contract.client_frames["install_chunk"].fields["data_hex"];
    assert_eq!(
        chunk.max_bytes,
        Some(crate::world_session::MAX_WORLD_MODULE_CHUNK_BYTES * 2)
    );
    let total = &contract.client_frames["install_begin"].fields["total_bytes"];
    assert_eq!(
        total.max,
        Some(crate::world_session::MAX_WORLD_MODULE_BYTES as u64)
    );
    let paste = &contract.client_frames["paste"].fields["text"];
    assert_eq!(paste.max_bytes, Some(4096));
    assert_eq!(contract.redacted_keys, ["seed"]);
}

#[test]
fn every_client_frame_variant_is_in_the_contract_and_nothing_else_is() {
    let named: BTreeSet<String> = contract().client_frames.keys().cloned().collect();
    assert_eq!(client_frame_variants(), named);
}

#[test]
fn every_server_frame_variant_is_in_the_contract_and_nothing_else_is() {
    let named: BTreeSet<&str> = contract()
        .server_frames
        .keys()
        .map(String::as_str)
        .collect();
    let listed: BTreeSet<&str> = SERVER_FRAME_TYPES.iter().copied().collect();
    assert_eq!(listed, named);
    // The exhaustive match agrees with serde for frames that are cheap to build.
    let invite = InviteResponse {
        capability: "cap".to_owned(),
        participant_id: 3,
        display_name: "ada".to_owned(),
    };
    let history_event = crate::world::WorldEvent {
        sequence: 1,
        participant_id: 1,
        kind: crate::world::WorldEventKind::Chat("x".to_owned()),
    };
    for frame in [
        ServerFrame::View {
            sequence: 1,
            data: "x",
            surface: None,
        },
        ServerFrame::RecordsTicket {
            ticket: "t".to_owned(),
        },
        ServerFrame::ModuleBegin {
            module_hash: "h".to_owned(),
            total_bytes: 2,
        },
        ServerFrame::ModuleChunk {
            offset: 0,
            data_hex: "00".to_owned(),
        },
        ServerFrame::ModuleEnd {
            module_hash: "h".to_owned(),
        },
        ServerFrame::Invite(&invite),
        ServerFrame::WorldCreated {
            world: "w",
            created: true,
        },
        ServerFrame::Isolation {
            world: "w",
            isolation: crate::world::Isolation::Isolated,
        },
        ServerFrame::Compiled {
            module_hash: "h".to_owned(),
            sequence: 1,
            diagnostics: String::new(),
        },
        ServerFrame::UploadReady { chunk_bytes: 6 },
        ServerFrame::ModuleInstalled {
            module_hash: "h",
            sequence: 1,
        },
        ServerFrame::ProgramChanged {
            module_hash: "h",
            sequence: 1,
        },
        ServerFrame::EventHistoryBegin {
            world_id: "w".to_owned(),
            module_hash: None,
            current_sequence: 1,
            complete: true,
            event_count: 1,
        },
        ServerFrame::EventHistoryEvent {
            event: &history_event,
        },
        ServerFrame::Error { message: "no" },
        ServerFrame::ModuleInfo {
            world_id: "w".to_owned(),
            active_module_hash: None,
            active_module_provenance: None,
            current_sequence: 0,
        },
    ] {
        let value = serde_json::to_value(&frame).unwrap();
        assert_eq!(value["type"], server_frame_type(&frame));
        contract()
            .check_server_frame(&value)
            .unwrap_or_else(|error| panic!("{value}: {error}"));
    }
}

#[test]
fn the_contracts_required_fields_are_exactly_what_the_host_requires() {
    let contract = contract();
    for (kind, spec) in &contract.client_frames {
        let sample = sample_client_frame(contract, kind, "cap");
        let phase = spec.phases[0];
        contract
            .check_client_frame(&sample, phase)
            .unwrap_or_else(|error| panic!("{kind} sample: {error}"));
        serde_json::from_value::<ClientFrame>(sample.clone())
            .unwrap_or_else(|error| panic!("{kind} sample {sample} is refused: {error}"));
        for (name, field) in &spec.fields {
            let mut changed = sample.clone();
            if field.optional {
                // Absent is always fine; `null` exactly when the contract says so.
                assert!(contract.check_client_frame(&changed, phase).is_ok());
                changed[name.as_str()] = Value::Null;
                assert_eq!(
                    serde_json::from_value::<ClientFrame>(changed.clone()).is_ok(),
                    field.nullable,
                    "{kind}.{name}: the contract and the host disagree about null"
                );
                assert_eq!(
                    contract.check_client_frame(&changed, phase).is_ok(),
                    field.nullable
                );
            } else {
                changed.as_object_mut().unwrap().remove(name);
                assert!(
                    serde_json::from_value::<ClientFrame>(changed.clone()).is_err(),
                    "{kind}.{name} is required in the contract but the host accepts it absent"
                );
                assert!(contract.check_client_frame(&changed, phase).is_err());
            }
        }
    }
}

#[test]
fn every_frame_has_a_class_and_every_client_frame_a_phase() {
    for (kind, spec) in &contract().client_frames {
        assert!(!spec.phases.is_empty(), "{kind}");
        assert!(
            !matches!(spec.class, FrameClass::Durable | FrameClass::Reply),
            "{kind} uses a server class"
        );
        assert_eq!(
            spec.phases.contains(&Phase::Opening),
            spec.opens.is_some(),
            "{kind}: an opening frame says which phase it opens"
        );
    }
    for (kind, spec) in &contract().server_frames {
        assert!(
            matches!(
                spec.class,
                FrameClass::Durable | FrameClass::Reply | FrameClass::Ephemeral
            ),
            "{kind}"
        );
    }
    // The journaled-vs-ephemeral boundary as the contract states it.
    let class = |kind: &str| contract().client_frames[kind].class;
    assert_eq!(class("resize"), FrameClass::Ephemeral);
    for kind in ["input", "paste", "chat"] {
        assert_eq!(class(kind), FrameClass::Event, "{kind}");
    }
    assert_eq!(
        contract().server_frames["view"].class,
        FrameClass::Ephemeral
    );
}

#[test]
fn the_check_names_what_a_frame_got_wrong() {
    let contract = contract();
    let joined = |frame: Value| {
        contract
            .check_client_frame(&frame, Phase::Joined)
            .map(str::to_owned)
    };
    assert_eq!(
        joined(json!({"type": "input", "data_hex": "0aFf"})).as_deref(),
        Ok("input")
    );
    assert!(
        joined(json!({"type": "teleport"}))
            .unwrap_err()
            .contains("not a client frame")
    );
    assert!(
        joined(json!({"type": "input"}))
            .unwrap_err()
            .contains("needs field")
    );
    assert!(
        joined(json!({"type": "input", "data_hex": "abc"}))
            .unwrap_err()
            .contains("hex")
    );
    assert!(
        joined(json!({"type": "chat", "text": "hi", "colour": "red"}))
            .unwrap_err()
            .contains("no field `colour`")
    );
    assert!(
        joined(json!({"type": "resize", "width": 501, "height": 10}))
            .unwrap_err()
            .contains("1..=500")
    );
    assert!(joined(json!({"type": "resize", "width": 80, "height": -1})).is_err());
    assert!(
        joined(json!({"type": "paste", "text": "x".repeat(4097)}))
            .unwrap_err()
            .contains("more than 4096")
    );
    assert!(
        joined(json!({"type": "attenuate", "name": "n", "scopes": [], "nested": {"uses": 1}}))
            .unwrap_err()
            .contains("`uses`")
    );
    assert!(
        joined(json!({"type": "download_module", "capability": "c", "module_hash": "h"}))
            .unwrap_err()
            .contains("joined phase")
    );
    assert!(
        contract
            .check_client_frame(&json!({"type": "input", "data_hex": "00"}), Phase::Opening)
            .is_err()
    );
    // The routing field is allowed on any frame, but only as a string.
    assert!(joined(json!({"type": "chat", "text": "hi", "world": "arena"})).is_ok());
    assert!(joined(json!({"type": "chat", "text": "hi", "world": 7})).is_err());
    assert!(joined(json!(["input"])).is_err());
    assert!(joined(json!({"type": "chat", "text": "x".repeat(2 * 1024 * 1024)})).is_err());
}

#[test]
fn the_check_refuses_a_seed_that_escaped_the_projection() {
    let contract = contract();
    let event = |seed: u64| {
        json!({"type": "event", "event": {"sequence": 1, "participant_id": 0,
            "kind": {"kind": "program_installed", "data": {"module_hash": "h", "seed": seed}}}})
    };
    assert_eq!(contract.check_server_frame(&event(0)), Ok("event"));
    assert!(
        contract
            .check_server_frame(&event(424_242))
            .unwrap_err()
            .contains("leaks `seed`")
    );
    let snapshot = json!({"type": "snapshot", "snapshot": {"events": [
        {"kind": {"kind": "program_installed", "data": {"seed": 9}}}
    ]}});
    assert!(contract.check_server_frame(&snapshot).is_err());
    assert!(
        contract
            .check_server_frame(&json!({"type": "error"}))
            .unwrap_err()
            .contains("needs field `message`")
    );
    assert!(
        contract
            .check_server_frame(&json!({"type": "view", "sequence": 1, "data": "", "extra": 1}))
            .is_err()
    );
}

#[test]
fn a_transcript_follows_the_session_phases() {
    let contract = contract();
    let transcript = [
        json!({"from": "client", "frame": {"type": "info", "capability": "c"}}),
        json!({"from": "server", "frame": {"type": "module_info", "world_id": "w",
            "active_module_hash": null, "current_sequence": 0}}),
        json!({"from": "client", "frame": {"type": "join", "capability": "c", "after": null}}),
        json!({"from": "server", "frame": {"type": "snapshot", "snapshot": {}}}),
        json!({"from": "client", "frame": {"type": "resize", "width": 80, "height": 24}}),
        json!({"from": "client", "frame": {"type": "input", "data_hex": "6869"}}),
    ]
    .iter()
    .map(Value::to_string)
    .collect::<Vec<_>>()
    .join("\n");
    assert_eq!(check_transcript(contract, &transcript).unwrap(), []);

    // An opening invite ends the session; a frame after it is a violation.
    let mut check = Conformance::new(contract);
    check.client(&json!({"type": "invite", "admin_token": "a", "display_name": "d"}));
    check.server(&json!({"type": "error", "message": "invite denied"}));
    assert_eq!(check.phase(), Phase::Closed);
    check.client(&json!({"type": "chat", "text": "hi"}));
    let violations = check.finish();
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert_eq!(violations[0].index, 2);
    assert_eq!(violations[0].side, Side::Client);
    assert!(violations[0].to_string().starts_with("frame 2 (client)"));

    // An upload: begin, chunks, end, then closed.
    let mut check = Conformance::new(contract);
    check
        .client(&json!({"type": "install_begin", "admin_token": "a", "total_bytes": 2, "seed": 1}));
    assert_eq!(check.phase(), Phase::Install);
    check.server(&json!({"type": "upload_ready", "chunk_bytes": 6144}));
    check.client(&json!({"type": "install_chunk", "offset": 0, "data_hex": "0000"}));
    check.client(&json!({"type": "install_end"}));
    assert_eq!(check.phase(), Phase::Closed);
    assert!(check.violations().is_empty(), "{:?}", check.violations());

    assert!(check_transcript(contract, "not json").is_err());
    assert!(check_transcript(contract, r#"{"from": "peer", "frame": {}}"#).is_err());
}
