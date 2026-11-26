use std::collections::BTreeSet;

use serde_json::json;

use super::*;
use crate::world_session::{ClientFrame, InviteResponse, ScreenRegion, ServerFrame};

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
        ServerFrame::FormResultStatus { .. } => "form_result_status",
        ServerFrame::View { .. } => "view",
        ServerFrame::ModuleInfo { .. } => "module_info",
        ServerFrame::Metrics { .. } => "metrics",
        ServerFrame::Records { .. } => "records",
        ServerFrame::RecordsTicket { .. } => "records_ticket",
        ServerFrame::Documents { .. } => "documents",
        ServerFrame::Document { .. } => "document",
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
        ServerFrame::PresentationPolicy { .. } => "presentation_policy",
        ServerFrame::CapsCeiling { .. } => "caps_ceiling",
        ServerFrame::Schedules { .. } => "schedules",
        ServerFrame::WorldState { .. } => "world_state",
        ServerFrame::Compiled { .. } => "compiled",
        ServerFrame::UploadReady { .. } => "upload_ready",
        ServerFrame::ModuleInstalled { .. } => "module_installed",
        ServerFrame::Revoked { .. } => "revoked",
        ServerFrame::ProgramChanged { .. } => "program_changed",
        ServerFrame::EventHistoryBegin { .. } => "event_history_begin",
        ServerFrame::EventHistoryEvent { .. } => "event_history_event",
        ServerFrame::Error { .. } => "error",
        ServerFrame::ProviderRequest { .. } => "provider_request",
        ServerFrame::InputRefused { .. } => "input_refused",
        ServerFrame::SemanticDiff { .. } => "semantic_diff",
        ServerFrame::ScreenSubscription { .. } => "screen_subscription",
    }
}

const SERVER_FRAME_TYPES: &[&str] = &[
    "snapshot",
    "event",
    "form_result_status",
    "view",
    "module_info",
    "metrics",
    "records",
    "records_ticket",
    "documents",
    "document",
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
    "presentation_policy",
    "caps_ceiling",
    "schedules",
    "world_state",
    "compiled",
    "upload_ready",
    "module_installed",
    "revoked",
    "program_changed",
    "event_history_begin",
    "event_history_event",
    "error",
    "provider_request",
    "input_refused",
    "semantic_diff",
    "screen_subscription",
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
        ServerFrame::FormResultStatus {
            form_id: "form-1",
            ok: false,
            presentation_token: Some("0123456789abcdef0123456789abcdef"),
            error: Some("bad_args"),
            reason: Some("field_invalid"),
        },
        ServerFrame::View {
            sequence: 1,
            data: "x",
            surface: None,
            view_revision: 0,
            base_view_revision: None,
            patch: None,
            resume_token: None,
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
        ServerFrame::Error {
            message: "no",
            code: None,
        },
        ServerFrame::Error {
            message: "denied",
            code: Some("actor_denied"),
        },
        ServerFrame::ProviderRequest {
            nonce: "ab12",
            provider: "client.file.read",
            args: serde_json::json!({"path": "notes.txt"}),
            deadline_ms: 5000,
        },
        ServerFrame::SemanticDiff {
            sequence: 7,
            view_revision: 3,
            base_view_revision: Some(2),
            changes: vec![
                serde_json::json!({"kind": "chat_posted", "participant_id": 1, "text": "hi"}),
            ],
            region: None,
        },
        ServerFrame::SemanticDiff {
            sequence: 8,
            view_revision: 4,
            base_view_revision: None,
            changes: vec![serde_json::json!({"kind": "region_changed", "row": 0, "text": "hi"})],
            region: Some(ScreenRegion {
                x: 0,
                y: 0,
                width: 10,
                height: 2,
            }),
        },
        ServerFrame::ScreenSubscription {
            region: Some(ScreenRegion {
                x: 0,
                y: 0,
                width: 10,
                height: 2,
            }),
            event_classes: Some(vec!["chat_posted".to_owned()]),
            view_revision: 3,
        },
        ServerFrame::ScreenSubscription {
            region: None,
            event_classes: None,
            view_revision: 0,
        },
        ServerFrame::Revoked {
            participant_id: 7,
            revoked: true,
        },
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

    // A signed upload: the optional signature object is part of the contract.
    let mut check = Conformance::new(contract);
    check.client(&json!({"type": "install_begin", "admin_token": "a", "total_bytes": 2, "seed": 1, "signature": {"public_key": "ab", "signature": "cd"}}));
    assert_eq!(check.phase(), Phase::Install);
    check.server(&json!({"type": "upload_ready", "chunk_bytes": 6144}));
    check.client(&json!({"type": "install_chunk", "offset": 0, "data_hex": "0000"}));
    check.client(&json!({"type": "install_end"}));
    assert!(check.violations().is_empty(), "{:?}", check.violations());

    // A joined upload (design #835 phase 3): install_begin from the
    // joined phase enters the upload phase, and install_end returns
    // to joined — not closed — so the session continues.
    let mut check = Conformance::new(contract);
    check.client(&json!({"type": "join", "capability": "c", "after": null}));
    check.server(&json!({"type": "snapshot", "snapshot": {}}));
    assert_eq!(check.phase(), Phase::Joined);
    check.client(&json!({"type": "install_begin", "total_bytes": 2, "seed": 1}));
    assert_eq!(check.phase(), Phase::Install);
    check.server(&json!({"type": "upload_ready", "chunk_bytes": 6144}));
    check.client(&json!({"type": "install_chunk", "offset": 0, "data_hex": "0000"}));
    check.client(&json!({"type": "install_end"}));
    assert_eq!(check.phase(), Phase::Joined);
    check.client(&json!({"type": "chat", "text": "hi"}));
    assert!(check.violations().is_empty(), "{:?}", check.violations());

    // A refused joined upload (an error instead of upload_ready)
    // likewise leaves the session joined, not closed.
    let mut check = Conformance::new(contract);
    check.client(&json!({"type": "join", "capability": "c", "after": null}));
    check.server(&json!({"type": "snapshot", "snapshot": {}}));
    check.client(&json!({"type": "install_begin", "total_bytes": 2, "seed": 1}));
    assert_eq!(check.phase(), Phase::Install);
    check.server(
        &json!({"type": "error", "message": "module install denied", "code": "actor_denied"}),
    );
    assert_eq!(check.phase(), Phase::Joined);
    assert!(check.violations().is_empty(), "{:?}", check.violations());

    assert!(check_transcript(contract, "not json").is_err());
    assert!(check_transcript(contract, r#"{"from": "peer", "frame": {}}"#).is_err());
}

#[test]
fn provider_frames_conform_only_where_the_contract_allows_them() {
    let contract = contract();

    // F13.7 step 2: the offer and the reply are client frames for a
    // joined session only — a provider frame in the opening phase is
    // a violation, so a session that has not joined can neither
    // advertise a provider nor answer a request.
    let offer = json!({"type": "provider_offer", "providers": ["client.file.read"]});
    assert_eq!(
        contract.check_client_frame(&offer, Phase::Joined).unwrap(),
        "provider_offer"
    );
    assert!(contract.check_client_frame(&offer, Phase::Opening).is_err());
    let parsed: ClientFrame = serde_json::from_value(offer).unwrap();
    assert!(matches!(parsed, ClientFrame::ProviderOffer { .. }));

    let reply =
        json!({"type": "provider_reply", "nonce": "ab12", "ok": true, "result_hex": "00ff"});
    assert_eq!(
        contract.check_client_frame(&reply, Phase::Joined).unwrap(),
        "provider_reply"
    );
    assert!(contract.check_client_frame(&reply, Phase::Query).is_err());
    let parsed: ClientFrame = serde_json::from_value(reply).unwrap();
    assert!(matches!(parsed, ClientFrame::ProviderReply { .. }));

    // The request is a server frame; an undeclared field (a session
    // or participant claim the design says never to trust from the
    // wire) is a violation.
    let request = json!({
        "type": "provider_request",
        "nonce": "ab12",
        "provider": "client.file.read",
        "args": {"path": "notes.txt"},
        "deadline_ms": 5000
    });
    assert_eq!(
        contract.check_server_frame(&request).unwrap(),
        "provider_request"
    );
    let mut claimed = request.clone();
    claimed["session"] = json!("someone-else");
    assert!(contract.check_server_frame(&claimed).is_err());

    // Nothing about the v1 frame set changed: every pre-existing
    // client frame type is still in the contract (the sync test
    // above pins the exact set both ways).
    assert!(contract.client_frames.contains_key("join"));
    assert!(contract.server_frames.contains_key("view"));
}

#[test]
fn pointer_and_hover_frames_conform_to_the_input_event_model() {
    // Oracle: the #1831 S2 design — activation events are journaled
    // (class event), hover is ephemeral, refusals name a reason from
    // the design's table, and scroll deltas are signed cell units.
    let contract = contract();
    assert_eq!(contract.revision, 17);
    assert_eq!(contract.client_frames["pointer"].class, FrameClass::Event);
    assert_eq!(contract.client_frames["hover"].class, FrameClass::Ephemeral);
    assert_eq!(
        contract.server_frames["input_refused"].class,
        FrameClass::Reply
    );

    let scroll = json!({
        "type": "pointer",
        "event": "scroll",
        "environment": "android",
        "pointer_id": 0,
        "x": 3,
        "y": 4,
        "dx": -2,
        "dy": 5
    });
    assert_eq!(
        contract.check_client_frame(&scroll, Phase::Joined).unwrap(),
        "pointer"
    );
    let parsed: ClientFrame = serde_json::from_value(scroll.clone()).unwrap();
    assert!(matches!(parsed, ClientFrame::Pointer { .. }));

    // A non-integer scroll delta is not a signed integer.
    let mut bad_scroll = scroll.clone();
    bad_scroll["dx"] = json!(1.5);
    assert!(
        contract
            .check_client_frame(&bad_scroll, Phase::Joined)
            .is_err()
    );

    // An event the model does not name is refused by the contract check.
    let mut mystery = scroll.clone();
    mystery["event"] = json!("pinch");
    assert!(
        contract
            .check_client_frame(&mystery, Phase::Joined)
            .is_err()
    );
    assert!(serde_json::from_value::<ClientFrame>(mystery).is_err());

    let hover = json!({
        "type": "hover",
        "event": "hover_move",
        "environment": "web",
        "pointer_id": 0,
        "x": 1,
        "y": 2
    });
    assert_eq!(
        contract.check_client_frame(&hover, Phase::Joined).unwrap(),
        "hover"
    );
    let parsed: ClientFrame = serde_json::from_value(hover).unwrap();
    assert!(matches!(parsed, ClientFrame::Hover { .. }));

    // The refusal answer: every reason in the design's table conforms,
    // and a reason outside the table does not.
    for reason in [
        "no_timing_source",
        "hover_unsupported",
        "context_menu_not_claimed",
        "reserved_by_browser",
        "no_secondary_button",
        "not_reported_by_terminal",
        "button_identity_unavailable",
        "no_tracking_mode",
        "gesture_not_in_subset",
        "multi_touch_deferred",
        "not_declared_by_app",
        "unknown_or_unsupported_event",
    ] {
        let refusal = json!({
            "type": "input_refused",
            "event": "hover_enter",
            "environment": "android",
            "reason": reason
        });
        assert_eq!(
            contract.check_server_frame(&refusal).unwrap(),
            "input_refused",
            "{reason}"
        );
    }
    let refusal = json!({
        "type": "input_refused",
        "event": "hover_enter",
        "reason": "not_a_reason"
    });
    assert!(contract.check_server_frame(&refusal).is_err());

    let frame = ServerFrame::InputRefused {
        event: "secondary_press",
        environment: Some(crate::world_session::InputEnvironment::Web),
        reason: crate::world_session::InputRefusalReason::ContextMenuNotClaimed,
    };
    let value = serde_json::to_value(&frame).unwrap();
    assert_eq!(value["type"], "input_refused");
    contract.check_server_frame(&value).unwrap();
}

#[test]
fn agent_profile_frames_conform_and_are_additive() {
    // Oracle: the #6300 design (doc/2026-11-09_design_id6300_agents_in_worlds_v1.md)
    // sections 4.2, 4.3, 8, 9.1 — the agent profile is an additive
    // revision: a capability flag on join, region subscriptions and
    // semantic-diff server frames for the agent profile, and the
    // stale-decision token on agent input frames. Human shells, which
    // never set the flag or send the new frames, are unaffected.
    let contract = contract();
    assert_eq!(contract.revision, 17);
    assert_eq!(
        contract.client_frames["screen_subscribe"].class,
        FrameClass::Ephemeral
    );
    assert_eq!(
        contract.server_frames["semantic_diff"].class,
        FrameClass::Ephemeral
    );
    assert_eq!(
        contract.server_frames["screen_subscription"].class,
        FrameClass::Reply
    );
    let flag = &contract.client_frames["join"].fields["agent_profile"];
    assert!(flag.optional && !flag.nullable, "join.agent_profile");
    for kind in ["input", "chat", "paste", "form_result", "pointer"] {
        let token = &contract.client_frames[kind].fields["screen_revision"];
        assert!(
            token.optional,
            "{kind}.screen_revision is the optional stale-decision token"
        );
    }

    // A full agent-profile session transcript conforms end to end.
    let transcript = [
        json!({"from": "client", "frame": {"type": "join", "capability": "cap", "after": null, "agent_profile": true}}),
        json!({"from": "server", "frame": {"type": "snapshot", "snapshot": {"sequence": 0}}}),
        json!({"from": "server", "frame": {"type": "view", "sequence": 0, "data": "hello", "view_revision": 0}}),
        json!({"from": "client", "frame": {"type": "screen_subscribe", "region": {"x": 0, "y": 0, "width": 10, "height": 2}, "event_classes": ["chat_posted"]}}),
        json!({"from": "server", "frame": {"type": "screen_subscription", "region": {"x": 0, "y": 0, "width": 10, "height": 2}, "event_classes": ["chat_posted"], "view_revision": 0}}),
        json!({"from": "server", "frame": {"type": "semantic_diff", "sequence": 1, "view_revision": 0, "changes": [{"kind": "chat_posted", "participant_id": 2, "text": "hi"}]}}),
        json!({"from": "client", "frame": {"type": "input", "data_hex": "6c", "screen_revision": 0}}),
        json!({"from": "server", "frame": {"type": "semantic_diff", "sequence": 1, "view_revision": 1, "base_view_revision": 0, "changes": [{"kind": "region_changed", "row": 0, "text": "hello agent"}], "region": {"x": 0, "y": 0, "width": 10, "height": 2}}}),
    ]
    .iter()
    .map(serde_json::Value::to_string)
    .collect::<Vec<_>>()
    .join("\n");
    let violations = check_transcript(contract, &transcript).unwrap();
    assert!(violations.is_empty(), "{violations:#?}\n{transcript}");

    // A subscription is a joined-phase frame: in the opening phase it is
    // a violation, like every other joined-only frame.
    let early = json!({"type": "screen_subscribe", "event_classes": []});
    assert!(contract.check_client_frame(&early, Phase::Opening).is_err());
    assert_eq!(
        contract.check_client_frame(&early, Phase::Joined).unwrap(),
        "screen_subscribe"
    );
    let parsed: ClientFrame = serde_json::from_value(early).unwrap();
    assert!(matches!(parsed, ClientFrame::ScreenSubscribe { .. }));

    // The projection rule binds diffs too: a semantic diff may not leak
    // a program seed past the client projection.
    let leaking = json!({"type": "semantic_diff", "sequence": 1, "view_revision": 0, "changes": [{"kind": "program_changed", "seed": 42}]});
    assert!(contract.check_server_frame(&leaking).is_err());
}

#[test]
fn a_duplicated_key_is_refused_in_the_contract_and_the_transcript() {
    // A lenient parse would ignore the unknown key and keep the last
    // "from", so both would pass without the refusal (#8794).
    let duplicated = SHELL_CONTRACT_JSON.replacen('{', r#"{"x":1,"x":2,"#, 1);
    assert!(ShellContract::parse(SHELL_CONTRACT_JSON).is_ok());
    assert!(
        ShellContract::parse(&duplicated)
            .unwrap_err()
            .contains("duplicate JSON object key")
    );
    let line =
        r#"{"from": "server", "from": "client", "frame": {"type": "info", "capability": "c"}}"#;
    assert!(check_transcript(contract(), &line.replacen(r#""from": "server", "#, "", 1)).is_ok());
    assert!(check_transcript(contract(), line).is_err());
}

/// Read a file from `pkgs/id/contract/` (the crate root is
/// `crates/id-worlds`, so the contract directory is two levels up).
fn contract_dir_file(path: &str) -> String {
    let full = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../contract")
        .join(path);
    std::fs::read_to_string(&full)
        .unwrap_or_else(|error| panic!("read {}: {error}", full.display()))
}

fn statement(sends: &[&str], presents: &[&str], workflow: &[&str]) -> CapabilityStatement {
    CapabilityStatement {
        sends: sends.iter().map(ToString::to_string).collect(),
        presents: presents.iter().map(ToString::to_string).collect(),
        providers: Vec::new(),
        workflow: workflow.iter().map(ToString::to_string).collect(),
    }
}

#[test]
fn the_shipped_suite_manifest_parses_and_names_every_v1_shell() {
    let suite = ShellSuite::embedded().unwrap();
    assert_eq!(suite.version, 1);
    let names: BTreeSet<&str> = suite
        .shells
        .iter()
        .map(|shell| shell.name.as_str())
        .collect();
    assert_eq!(
        names,
        BTreeSet::from(["browser", "hardware", "native", "terminal"])
    );
    // Every v1 shell must state its capabilities; the manifest is
    // the record, so a shell without a statement fails here first.
    for shell in &suite.shells {
        assert!(
            shell.statement.is_some(),
            "shell `{}` has no capability statement in the shipped suite manifest",
            shell.name
        );
    }
}

#[test]
fn the_shipped_suite_passes_on_its_recorded_sessions() {
    let suite = ShellSuite::embedded().unwrap();
    let mut sessions = std::collections::BTreeMap::new();
    for shell in &suite.shells {
        if let Some(path) = &shell.session {
            sessions.insert(shell.name.clone(), contract_dir_file(path));
        }
    }
    let entries: Vec<SuiteEntry<'_>> = suite
        .shells
        .iter()
        .map(|shell| SuiteEntry {
            name: &shell.name,
            statement: shell.statement.as_ref(),
            session: match sessions.get(&shell.name) {
                Some(text) => SessionSource::Recorded(text),
                None => SessionSource::Unrecorded,
            },
        })
        .collect();
    let verdicts = check_suite(contract(), &entries);
    for verdict in &verdicts {
        assert!(
            verdict.passed(),
            "shell `{}` failed the shipped suite: {:?}",
            verdict.name,
            verdict.failures
        );
    }
    let recorded: BTreeSet<&str> = verdicts
        .iter()
        .filter(|verdict| verdict.recorded)
        .map(|verdict| verdict.name.as_str())
        .collect();
    assert_eq!(recorded, BTreeSet::from(["browser", "native", "terminal"]));
}

#[test]
fn the_suite_fails_a_shell_with_no_capability_statement() {
    let entries = [SuiteEntry {
        name: "mystery",
        statement: None,
        session: SessionSource::Unrecorded,
    }];
    let verdicts = check_suite(contract(), &entries);
    assert_eq!(verdicts.len(), 1);
    assert!(!verdicts[0].passed());
    assert!(
        verdicts[0].failures[0].contains("no capability statement"),
        "{:?}",
        verdicts[0].failures
    );
}

#[test]
fn the_suite_fails_a_statement_that_claims_frames_the_contract_does_not_name() {
    let statement = statement(&["join", "teleport"], &["view"], &["join"]);
    let entries = [SuiteEntry {
        name: "dreamer",
        statement: Some(&statement),
        session: SessionSource::Unrecorded,
    }];
    let verdicts = check_suite(contract(), &entries);
    assert!(!verdicts[0].passed());
    assert!(
        verdicts[0]
            .failures
            .iter()
            .any(|failure| failure.contains("teleport")),
        "{:?}",
        verdicts[0].failures
    );
}

#[test]
fn the_suite_fails_a_session_that_sends_outside_its_statement() {
    let statement = statement(&["join"], &["snapshot", "view"], &["join"]);
    let session = [
        r#"{"from": "client", "frame": {"type": "join", "capability": "c"}}"#,
        r#"{"from": "server", "frame": {"type": "snapshot", "snapshot": {}}}"#,
        r#"{"from": "client", "frame": {"type": "chat", "text": "undeclared"}}"#,
    ]
    .join("\n");
    let entries = [SuiteEntry {
        name: "chatty",
        statement: Some(&statement),
        session: SessionSource::Recorded(&session),
    }];
    let verdicts = check_suite(contract(), &entries);
    assert!(!verdicts[0].passed());
    assert!(
        verdicts[0].failures.iter().any(|failure| failure
            .contains("outside its capability statement")
            && failure.contains("chat")),
        "{:?}",
        verdicts[0].failures
    );
}

#[test]
fn the_suite_fails_a_thin_session_that_never_exercises_its_workflow() {
    let statement = statement(
        &["join", "resize"],
        &["snapshot", "view"],
        &["join", "resize"],
    );
    let session = [
        r#"{"from": "client", "frame": {"type": "join", "capability": "c"}}"#,
        r#"{"from": "server", "frame": {"type": "snapshot", "snapshot": {}}}"#,
    ]
    .join("\n");
    let entries = [SuiteEntry {
        name: "thin",
        statement: Some(&statement),
        session: SessionSource::Recorded(&session),
    }];
    let verdicts = check_suite(contract(), &entries);
    assert!(!verdicts[0].passed());
    assert!(
        verdicts[0]
            .failures
            .iter()
            .any(|failure| failure.contains("never exercises") && failure.contains("resize")),
        "{:?}",
        verdicts[0].failures
    );
}

#[test]
fn the_suite_fails_a_session_that_violates_the_contract() {
    let statement = statement(&["chat", "join"], &["snapshot", "view"], &["join"]);
    let session = r#"{"from": "client", "frame": {"type": "chat", "text": "too early"}}"#;
    let entries = [SuiteEntry {
        name: "early",
        statement: Some(&statement),
        session: SessionSource::Recorded(session),
    }];
    let verdicts = check_suite(contract(), &entries);
    assert!(!verdicts[0].passed());
    assert!(
        verdicts[0]
            .failures
            .iter()
            .any(|failure| failure.contains("frame 0 (client)")),
        "{:?}",
        verdicts[0].failures
    );
}

#[test]
fn the_suite_fails_an_unreadable_recorded_session_and_passes_an_unrecorded_shell() {
    let statement = statement(&["join"], &["snapshot"], &["join"]);
    let entries = [
        SuiteEntry {
            name: "lost",
            statement: Some(&statement),
            session: SessionSource::Unreadable("no such file"),
        },
        SuiteEntry {
            name: "pending",
            statement: Some(&statement),
            session: SessionSource::Unrecorded,
        },
    ];
    let verdicts = check_suite(contract(), &entries);
    assert!(!verdicts[0].passed());
    assert!(verdicts[0].recorded);
    assert!(
        verdicts[0]
            .failures
            .iter()
            .any(|failure| failure.contains("could not be read")),
        "{:?}",
        verdicts[0].failures
    );
    assert!(verdicts[1].passed());
    assert!(!verdicts[1].recorded);
}
