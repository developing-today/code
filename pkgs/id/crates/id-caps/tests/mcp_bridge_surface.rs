//! Surface oracle for #7914: only the seven approved tools are exposed, and
//! a mid-session definition change terminates the bridge.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id_caps::mcp_bridge::{
    BridgeError, BridgeSession, FORBIDDEN_TOOL_NAMES, ToolAction, v1_tool_definitions,
};
use serde_json::json;

#[test]
fn v1_surface_excludes_forbidden_non_tools() {
    let definitions = v1_tool_definitions();
    let names: Vec<&str> = definitions
        .iter()
        .map(|definition| definition.name.as_str())
        .collect();

    assert_eq!(names.len(), 7);
    assert!(names.contains(&"world.join"));
    assert!(names.contains(&"world.leave"));
    assert!(names.contains(&"screen.read"));
    assert!(names.contains(&"screen.subscribe"));
    assert!(names.contains(&"object.inspect"));
    assert!(names.contains(&"chat.post"));
    assert!(names.contains(&"presence.set"));
    for forbidden in FORBIDDEN_TOOL_NAMES {
        assert!(!names.contains(&forbidden));
    }
}

#[test]
fn tampered_definition_terminates_session() {
    let definitions = v1_tool_definitions();
    let mut session = BridgeSession::start(&definitions).expect("the static surface serializes");
    let mut tampered = definitions.clone();
    tampered[0].ceiling = "credential.read".to_owned();

    let error = session
        .verify_definitions(&tampered)
        .expect_err("tampering must terminate");
    assert!(matches!(error, BridgeError::DefinitionChanged { .. }));
    assert!(session.is_terminated());
    assert!(session.verify_definitions(&definitions).is_err());
}

#[test]
fn every_surface_tool_translates_or_names_its_missing_mapping() {
    // Minimal valid arguments per tool, in an order that keeps the
    // session joined for the session-scoped tools.
    let calls: [(&str, serde_json::Value); 7] = [
        ("world.join", json!({ "capability": "cap" })),
        (
            "screen.subscribe",
            json!({ "event_classes": ["chat_posted"] }),
        ),
        ("screen.read", json!({})),
        ("chat.post", json!({ "text": "hi" })),
        ("object.inspect", json!({ "entity_ref": "cell-0" })),
        ("presence.set", json!({ "state": "away" })),
        ("world.leave", json!({})),
    ];
    let mut session =
        BridgeSession::start(&v1_tool_definitions()).expect("the static surface serializes");
    for (name, arguments) in calls {
        match session.tool_action(name, &arguments) {
            Ok(ToolAction::SendFrame(_) | ToolAction::Local(_) | ToolAction::Close) => {}
            Err(BridgeError::NoFrameMapping { tool }) => {
                assert_eq!(tool, "presence.set", "only presence.set lacks a mapping");
                assert_eq!(name, "presence.set");
            }
            other => panic!("{name} must translate, got {other:?}"),
        }
    }
    // The forbidden non-tools never translate.
    let mut session =
        BridgeSession::start(&v1_tool_definitions()).expect("the static surface serializes");
    for forbidden in FORBIDDEN_TOOL_NAMES {
        assert!(
            matches!(
                session.tool_action(forbidden, &json!({})),
                Err(BridgeError::UnknownTool { .. })
            ),
            "{forbidden} must stay off the bridge"
        );
    }
}
