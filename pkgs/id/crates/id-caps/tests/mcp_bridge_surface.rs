//! Surface oracle for #7914: only the seven approved tools are exposed, and
//! a mid-session definition change terminates the bridge.

use id_caps::mcp_bridge::{BridgeError, BridgeSession, FORBIDDEN_TOOL_NAMES, v1_tool_definitions};

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
