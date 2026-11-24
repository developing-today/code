//! The authority-free MCP bridge surface for the agent-in-worlds spike.
//!
//! The bridge is a translator: the world host remains responsible for
//! credentials, authority, quota, and audit. This module defines the
//! narrow v1 tool surface, the session-start definition pin, and the
//! translation itself (design #6300 sections 3 and 4.4): a tool call
//! becomes an F13.1 client frame, a local answer rendered from the
//! server frames observed so far, or a session close. The bridge holds
//! no credentials of its own; the one secret-shaped value it ever
//! handles is the agent's own join capability, passed through into the
//! join frame exactly as any shell presents it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// Names that are deliberately not tools on the bridge surface.
pub const FORBIDDEN_TOOL_NAMES: [&str; 5] = [
    "capability.mint",
    "credential.read",
    "audit.mutate",
    "world.export",
    "host.exec",
];

/// A v1 tool definition exposed to an MCP client.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolDefinition {
    /// Stable MCP tool name.
    pub name: String,
    /// F13.1 world command the host will receive after authority checks.
    pub command: String,
    /// Capability ceiling key enforced by the world host.
    pub ceiling: String,
}

impl ToolDefinition {
    fn new(name: &str, command: &str, ceiling: &str) -> Self {
        Self {
            name: name.to_owned(),
            command: command.to_owned(),
            ceiling: ceiling.to_owned(),
        }
    }
}

/// The eight approved v1 MCP tools, in their pinned wire order.
///
/// One entry is pinned but can never succeed: `presence.set` is refused
/// unconditionally by [`BridgeSession::tool_action`] with
/// [`BridgeError::NoFrameMapping`], because presence is the join itself
/// and no presence frame exists. A v1 client must treat `presence.set`
/// as a tool that always fails.
#[must_use]
pub fn v1_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::new("world.join", "session.presence", "presence"),
        ToolDefinition::new("world.leave", "session.presence", "presence"),
        ToolDefinition::new("screen.read", "screen.read", "screen.read"),
        ToolDefinition::new("screen.subscribe", "screen.subscribe", "screen.read"),
        ToolDefinition::new("object.inspect", "object.inspect", "screen.read"),
        ToolDefinition::new("program.invoke", "input", "program.invoke"),
        ToolDefinition::new("chat.post", "chat.post", "chat.post"),
        ToolDefinition::new("presence.set", "presence.set", "presence"),
    ]
}

/// Errors that terminate a bridge session before a changed tool definition
/// can be used as a poisoning or shadowing vector, and errors that refuse
/// a single tool call without ending the session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BridgeError {
    /// The definition set changed after the session was pinned.
    DefinitionChanged {
        /// Hash recorded at session start.
        expected: [u8; 32],
        /// Hash observed during the later check.
        actual: [u8; 32],
    },
    /// The definition set could not be serialized for hashing.
    InvalidDefinitions,
    /// A definition mismatch already terminated this session; no tool
    /// call may be translated afterwards.
    Terminated,
    /// The tool name is not on the pinned v1 surface.
    UnknownTool {
        /// The refused tool name.
        name: String,
    },
    /// The tool's arguments do not match its pinned shape.
    InvalidArguments {
        /// The tool whose arguments were refused.
        tool: &'static str,
        /// What was wrong with the arguments.
        problem: &'static str,
    },
    /// The tool is on the surface but has no F13.1 frame mapping in v1.
    NoFrameMapping {
        /// The tool without a mapping.
        tool: &'static str,
    },
    /// The tool needs a joined session and none is open.
    NotJoined {
        /// The tool that required a session.
        tool: &'static str,
    },
    /// `world.join` was called on an already joined session.
    AlreadyJoined,
}

/// What the bridge driver must do with one translated tool call.
///
/// The `Eq` derive is out of reach — frames are `serde_json::Value`,
/// which is not `Eq` — so the nursery suggestion to add it is refused.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, Debug, PartialEq)]
pub enum ToolAction {
    /// Send this F13.1 client frame (JSON) to the world host.
    SendFrame(serde_json::Value),
    /// Answer the call locally, from the server frames observed so far;
    /// the string is the tool result text.
    Local(String),
    /// End the session. `world.leave` maps here: presence in the F13.1
    /// contract is connection-scoped, so leaving is closing the
    /// connection — there is no leave frame.
    Close,
}

/// A bridge session: the pinned definition hash plus the screen state
/// the local tools are answered from. (Not `Eq`: the observed frames
/// are `serde_json::Value`.)
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, Debug, PartialEq)]
pub struct BridgeSession {
    pinned_definition_hash: [u8; 32],
    terminated: bool,
    joined: bool,
    subscribed: bool,
    screen_text: Option<String>,
    view_revision: u64,
    pending_changes: Vec<serde_json::Value>,
    objects: BTreeMap<String, serde_json::Value>,
}

impl BridgeSession {
    /// Start a session and pin the exact ordered definition bytes.
    pub fn start(definitions: &[ToolDefinition]) -> Result<Self, BridgeError> {
        Ok(Self {
            pinned_definition_hash: definition_hash(definitions)?,
            terminated: false,
            joined: false,
            subscribed: false,
            screen_text: None,
            view_revision: 0,
            pending_changes: Vec::new(),
            objects: BTreeMap::new(),
        })
    }

    /// Check that the server still serves the pinned definitions.
    ///
    /// Once a mismatch is observed the session stays terminated, even if a
    /// later check happens to present the original definitions.
    pub fn verify_definitions(
        &mut self,
        definitions: &[ToolDefinition],
    ) -> Result<(), BridgeError> {
        if self.terminated {
            return Err(BridgeError::DefinitionChanged {
                expected: self.pinned_definition_hash,
                actual: self.pinned_definition_hash,
            });
        }
        let actual = definition_hash(definitions)?;
        if actual != self.pinned_definition_hash {
            self.terminated = true;
            return Err(BridgeError::DefinitionChanged {
                expected: self.pinned_definition_hash,
                actual,
            });
        }
        Ok(())
    }

    /// Whether a definition mismatch has terminated this session.
    #[must_use]
    pub const fn is_terminated(&self) -> bool {
        self.terminated
    }

    /// The pinned hash, for audit/debug output without exposing definitions.
    #[must_use]
    pub const fn pinned_definition_hash(&self) -> [u8; 32] {
        self.pinned_definition_hash
    }

    /// Whether `world.join` has opened the world session.
    #[must_use]
    pub const fn is_joined(&self) -> bool {
        self.joined
    }

    /// Whether the host has acknowledged a `screen.subscribe`.
    #[must_use]
    pub const fn is_subscribed(&self) -> bool {
        self.subscribed
    }

    /// The latest full screen text observed, if any view has arrived.
    #[must_use]
    pub fn screen_text(&self) -> Option<&str> {
        self.screen_text.as_deref()
    }

    /// The newest screen revision observed on any frame that carries one.
    #[must_use]
    pub const fn view_revision(&self) -> u64 {
        self.view_revision
    }

    /// The semantic-diff changes observed since the last `screen.read`.
    #[must_use]
    pub fn pending_changes(&self) -> &[serde_json::Value] {
        &self.pending_changes
    }

    /// Translate one MCP tool call into the bridge driver's next step.
    ///
    /// Frame-backed tools produce [`ToolAction::SendFrame`] with the
    /// exact F13.1 client frame; `screen.read` and `object.inspect` are
    /// answered from the frames observed so far; `program.invoke`
    /// produces the single `input` frame that carries the invocation;
    /// `world.leave` produces [`ToolAction::Close`]. `presence.set` is refused with
    /// [`BridgeError::NoFrameMapping`]: presence in the F13.1 contract
    /// is the join itself, so there is no presence frame to send.
    pub fn tool_action(
        &mut self,
        name: &str,
        arguments: &serde_json::Value,
    ) -> Result<ToolAction, BridgeError> {
        if self.terminated {
            return Err(BridgeError::Terminated);
        }
        match name {
            "world.join" => self.join_action(arguments),
            "world.leave" => {
                self.require_joined("world.leave")?;
                self.joined = false;
                self.subscribed = false;
                Ok(ToolAction::Close)
            }
            "screen.subscribe" => {
                self.require_joined("screen.subscribe")?;
                Ok(ToolAction::SendFrame(subscribe_frame(arguments)?))
            }
            "screen.read" => {
                self.require_joined("screen.read")?;
                Ok(ToolAction::Local(self.screen_read()))
            }
            "chat.post" => {
                self.require_joined("chat.post")?;
                Ok(ToolAction::SendFrame(chat_frame(arguments)?))
            }
            "object.inspect" => {
                self.require_joined("object.inspect")?;
                Ok(ToolAction::Local(self.object_inspect(arguments)?))
            }
            "program.invoke" => {
                self.require_joined("program.invoke")?;
                Ok(ToolAction::SendFrame(invoke_frame(arguments)?))
            }
            "presence.set" => Err(BridgeError::NoFrameMapping {
                tool: "presence.set",
            }),
            _ => Err(BridgeError::UnknownTool {
                name: name.to_owned(),
            }),
        }
    }

    /// Observe one server frame, updating the screen state the local
    /// tools are answered from. Frames the bridge does not render —
    /// snapshots, events, errors — are ignored by design: the agent
    /// profile reads screens through views and semantic diffs.
    pub fn observe_frame(&mut self, frame: &serde_json::Value) {
        match frame.get("type").and_then(serde_json::Value::as_str) {
            Some("view") => {
                self.observe_revision(frame);
                // A patch view carries an empty `data` and a `patch`
                // instead; only a full view replaces the screen text.
                if frame.get("patch").is_none()
                    && let Some(data) = frame.get("data").and_then(serde_json::Value::as_str)
                {
                    self.screen_text = Some(data.to_owned());
                }
            }
            Some("semantic_diff") => {
                self.observe_revision(frame);
                if let Some(changes) = frame.get("changes").and_then(serde_json::Value::as_array) {
                    for change in changes {
                        if change.get("kind").and_then(serde_json::Value::as_str)
                            == Some("object_changed")
                            && let Some(entity_ref) = change.get("entity_ref")
                        {
                            self.objects.insert(entity_ref.to_string(), change.clone());
                        }
                        self.pending_changes.push(change.clone());
                    }
                }
            }
            Some("screen_subscription") => {
                self.subscribed = true;
                self.observe_revision(frame);
            }
            _ => {}
        }
    }

    /// Render the current screen for `screen.read`: the latest full
    /// view's text, then every semantic-diff change observed since the
    /// previous read. Reading drains the pending changes.
    pub fn screen_read(&mut self) -> String {
        let mut out = self
            .screen_text
            .clone()
            .unwrap_or_else(|| "(no screen received yet)".to_owned());
        if !self.pending_changes.is_empty() {
            out.push_str("\nchanges since last read:");
            for change in self.pending_changes.drain(..) {
                out.push('\n');
                if change.get("kind").and_then(serde_json::Value::as_str) == Some("chat_posted") {
                    out.push_str("chat_posted: ");
                    out.push_str(
                        change
                            .get("text")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or(""),
                    );
                } else {
                    out.push_str(&change.to_string());
                }
            }
        }
        out
    }

    fn join_action(&mut self, arguments: &serde_json::Value) -> Result<ToolAction, BridgeError> {
        if self.joined {
            return Err(BridgeError::AlreadyJoined);
        }
        let capability = arguments
            .get("capability")
            .and_then(serde_json::Value::as_str)
            .ok_or(BridgeError::InvalidArguments {
                tool: "world.join",
                problem: "capability must be a string",
            })?;
        let after = arguments.get("after").and_then(serde_json::Value::as_u64);
        self.joined = true;
        Ok(ToolAction::SendFrame(serde_json::json!({
            "type": "join",
            "capability": capability,
            "after": after,
            "agent_profile": true,
        })))
    }

    fn object_inspect(&self, arguments: &serde_json::Value) -> Result<String, BridgeError> {
        let entity_ref = arguments
            .get("entity_ref")
            .ok_or(BridgeError::InvalidArguments {
                tool: "object.inspect",
                problem: "entity_ref is required",
            })?;
        Ok(match self.objects.get(&entity_ref.to_string()) {
            Some(change) => change.to_string(),
            None => "object not in the observed screen state".to_owned(),
        })
    }

    const fn require_joined(&self, tool: &'static str) -> Result<(), BridgeError> {
        if self.joined {
            Ok(())
        } else {
            Err(BridgeError::NotJoined { tool })
        }
    }

    fn observe_revision(&mut self, frame: &serde_json::Value) {
        if let Some(revision) = frame
            .get("view_revision")
            .and_then(serde_json::Value::as_u64)
        {
            self.view_revision = revision;
        }
    }
}

/// Build the `screen_subscribe` frame for `screen.subscribe`: a region
/// and/or event classes, at least one of the two (the contract requires
/// a subscription to name something).
fn subscribe_frame(arguments: &serde_json::Value) -> Result<serde_json::Value, BridgeError> {
    let mut frame = serde_json::json!({ "type": "screen_subscribe" });
    if let Some(region) = arguments.get("region") {
        for key in ["x", "y", "width", "height"] {
            if region
                .get(key)
                .and_then(serde_json::Value::as_u64)
                .is_none()
            {
                return Err(BridgeError::InvalidArguments {
                    tool: "screen.subscribe",
                    problem: "region needs numeric x, y, width, height",
                });
            }
        }
        frame["region"] = region.clone();
    }
    if let Some(classes) = arguments.get("event_classes") {
        let valid = classes
            .as_array()
            .is_some_and(|classes| classes.iter().all(|class| class.as_str().is_some()));
        if !valid {
            return Err(BridgeError::InvalidArguments {
                tool: "screen.subscribe",
                problem: "event_classes must be an array of strings",
            });
        }
        frame["event_classes"] = classes.clone();
    }
    if frame.get("region").is_none() && frame.get("event_classes").is_none() {
        return Err(BridgeError::InvalidArguments {
            tool: "screen.subscribe",
            problem: "a region and/or event_classes is required",
        });
    }
    Ok(frame)
}

/// Build the `chat` frame for `chat.post`, carrying the caller's
/// stale-decision token when one is supplied (design section 4.3).
fn chat_frame(arguments: &serde_json::Value) -> Result<serde_json::Value, BridgeError> {
    let text = arguments
        .get("text")
        .and_then(serde_json::Value::as_str)
        .ok_or(BridgeError::InvalidArguments {
            tool: "chat.post",
            problem: "text must be a string",
        })?;
    let mut frame = serde_json::json!({ "type": "chat", "text": text });
    if let Some(revision) = arguments
        .get("screen_revision")
        .and_then(serde_json::Value::as_u64)
    {
        frame["screen_revision"] = serde_json::json!(revision);
    }
    Ok(frame)
}

/// Build the `input` frame for `program.invoke` (design section
/// 4.6): the one bounded program interaction. The call names the
/// program and the object it acts on; `arguments` is the invocation
/// payload in the program's own input language — the argument schema
/// is fixed in the role grant, and the spike's programs take a string
/// input — delivered as the frame's hex-encoded data. One call builds
/// exactly one frame: no chaining, and nothing else is sent. The
/// stale-decision token rides along when supplied (section 4.3); the
/// result returns as the host's view and semantic-diff frames, like
/// any other world change.
fn invoke_frame(arguments: &serde_json::Value) -> Result<serde_json::Value, BridgeError> {
    // The names are validated here and enforced there: which program
    // may be invoked is the policy point's allowlist call, and the
    // audit record names the program from the same arguments.
    for (field, problem) in [
        ("program", "program must be a non-empty string"),
        ("object", "object must be a non-empty string"),
    ] {
        if arguments
            .get(field)
            .and_then(serde_json::Value::as_str)
            .is_none_or(|value| value.is_empty())
        {
            return Err(BridgeError::InvalidArguments {
                tool: "program.invoke",
                problem,
            });
        }
    }
    let payload = arguments
        .get("arguments")
        .and_then(serde_json::Value::as_str)
        .ok_or(BridgeError::InvalidArguments {
            tool: "program.invoke",
            problem: "arguments must be a string",
        })?;
    let mut frame = serde_json::json!({
        "type": "input",
        "data_hex": hex_encode(payload.as_bytes()),
    });
    if let Some(revision) = arguments
        .get("screen_revision")
        .and_then(serde_json::Value::as_u64)
    {
        frame["screen_revision"] = serde_json::json!(revision);
    }
    Ok(frame)
}

/// Lowercase hex, the wire encoding of the `input` frame's data.
fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[usize::from(byte >> 4)] as char);
        out.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    out
}

fn definition_hash(definitions: &[ToolDefinition]) -> Result<[u8; 32], BridgeError> {
    let bytes = serde_json::to_vec(definitions).map_err(|_| BridgeError::InvalidDefinitions)?;
    Ok(Sha256::digest(bytes).into())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn started() -> BridgeSession {
        BridgeSession::start(&v1_tool_definitions()).unwrap()
    }

    fn joined() -> BridgeSession {
        let mut session = started();
        let action = session
            .tool_action("world.join", &json!({ "capability": "cap-1" }))
            .unwrap();
        assert_eq!(
            action,
            ToolAction::SendFrame(json!({
                "type": "join",
                "capability": "cap-1",
                "after": null,
                "agent_profile": true,
            }))
        );
        session
    }

    #[test]
    fn join_rejects_a_second_join_and_a_missing_capability() {
        let mut session = joined();
        assert_eq!(
            session.tool_action("world.join", &json!({ "capability": "cap-2" })),
            Err(BridgeError::AlreadyJoined)
        );
        let mut fresh = started();
        assert!(matches!(
            fresh.tool_action("world.join", &json!({})),
            Err(BridgeError::InvalidArguments {
                tool: "world.join",
                ..
            })
        ));
        assert!(!fresh.is_joined());
    }

    #[test]
    fn session_tools_require_a_join() {
        let mut session = started();
        for (name, args) in [
            ("world.leave", json!({})),
            (
                "screen.subscribe",
                json!({ "event_classes": ["chat_posted"] }),
            ),
            ("screen.read", json!({})),
            ("chat.post", json!({ "text": "hi" })),
            ("object.inspect", json!({ "entity_ref": 1 })),
            (
                "program.invoke",
                json!({ "program": "tic-tac-toe", "object": "board", "arguments": "4" }),
            ),
        ] {
            assert!(
                matches!(
                    session.tool_action(name, &args),
                    Err(BridgeError::NotJoined { .. })
                ),
                "{name} must require a join"
            );
        }
    }

    #[test]
    fn subscribe_frame_names_region_or_classes() {
        let mut session = joined();
        assert_eq!(
            session
                .tool_action(
                    "screen.subscribe",
                    &json!({ "event_classes": ["chat_posted"] })
                )
                .unwrap(),
            ToolAction::SendFrame(json!({
                "type": "screen_subscribe",
                "event_classes": ["chat_posted"],
            }))
        );
        assert_eq!(
            session
                .tool_action(
                    "screen.subscribe",
                    &json!({ "region": { "x": 0, "y": 1, "width": 5, "height": 2 } })
                )
                .unwrap(),
            ToolAction::SendFrame(json!({
                "type": "screen_subscribe",
                "region": { "x": 0, "y": 1, "width": 5, "height": 2 },
            }))
        );
        assert!(matches!(
            session.tool_action("screen.subscribe", &json!({})),
            Err(BridgeError::InvalidArguments {
                tool: "screen.subscribe",
                ..
            })
        ));
        assert!(matches!(
            session.tool_action(
                "screen.subscribe",
                &json!({ "region": { "x": 0, "y": 1, "width": 5 } })
            ),
            Err(BridgeError::InvalidArguments {
                tool: "screen.subscribe",
                ..
            })
        ));
    }

    #[test]
    fn chat_frame_carries_text_and_the_stale_decision_token() {
        let mut session = joined();
        assert_eq!(
            session
                .tool_action("chat.post", &json!({ "text": "hello" }))
                .unwrap(),
            ToolAction::SendFrame(json!({ "type": "chat", "text": "hello" }))
        );
        assert_eq!(
            session
                .tool_action(
                    "chat.post",
                    &json!({ "text": "hello", "screen_revision": 7 })
                )
                .unwrap(),
            ToolAction::SendFrame(json!({
                "type": "chat",
                "text": "hello",
                "screen_revision": 7,
            }))
        );
    }

    #[test]
    fn read_renders_the_view_then_drains_semantic_diffs() {
        let mut session = joined();
        session.observe_frame(&json!({
            "type": "view",
            "sequence": 1,
            "data": "board",
            "view_revision": 3,
        }));
        assert_eq!(session.screen_text(), Some("board"));
        assert_eq!(session.view_revision(), 3);
        session.observe_frame(&json!({
            "type": "semantic_diff",
            "sequence": 2,
            "view_revision": 3,
            "changes": [
                { "kind": "chat_posted", "participant_id": 2, "text": "hello agent" },
                { "kind": "object_changed", "participant_id": 2, "entity_ref": "cell-4" },
            ],
        }));
        assert_eq!(session.pending_changes().len(), 2);
        let read = session.screen_read();
        assert!(
            read.starts_with("board\nchanges since last read:"),
            "{read}"
        );
        assert!(read.contains("chat_posted: hello agent"), "{read}");
        assert!(read.contains("object_changed"), "{read}");
        // The read drained the diffs; a second read is the bare screen.
        assert_eq!(session.screen_read(), "board");
    }

    #[test]
    fn patch_views_do_not_blank_the_screen() {
        let mut session = joined();
        session.observe_frame(&json!({
            "type": "view",
            "sequence": 1,
            "data": "board",
            "view_revision": 1,
        }));
        session.observe_frame(&json!({
            "type": "view",
            "sequence": 2,
            "data": "",
            "view_revision": 2,
            "base_view_revision": 1,
            "patch": { "ops": [] },
        }));
        assert_eq!(session.screen_text(), Some("board"));
        assert_eq!(session.view_revision(), 2);
    }

    #[test]
    fn inspect_reads_the_observed_object_state() {
        let mut session = joined();
        session.observe_frame(&json!({
            "type": "semantic_diff",
            "sequence": 2,
            "view_revision": 1,
            "changes": [
                { "kind": "object_changed", "participant_id": 2, "entity_ref": "cell-4" },
            ],
        }));
        let found = session
            .tool_action("object.inspect", &json!({ "entity_ref": "cell-4" }))
            .unwrap();
        assert!(
            matches!(&found, ToolAction::Local(text) if text.contains("object_changed")),
            "{found:?}"
        );
        assert_eq!(
            session
                .tool_action("object.inspect", &json!({ "entity_ref": "cell-9" }))
                .unwrap(),
            ToolAction::Local("object not in the observed screen state".to_owned())
        );
    }

    #[test]
    fn leave_closes_and_presence_set_has_no_frame_mapping() {
        let mut session = joined();
        assert_eq!(
            session.tool_action("presence.set", &json!({ "state": "away" })),
            Err(BridgeError::NoFrameMapping {
                tool: "presence.set"
            })
        );
        assert_eq!(
            session.tool_action("world.leave", &json!({})).unwrap(),
            ToolAction::Close
        );
        assert!(!session.is_joined());
        assert!(matches!(
            session.tool_action("chat.post", &json!({ "text": "hi" })),
            Err(BridgeError::NotJoined { .. })
        ));
    }

    #[test]
    fn unknown_tools_and_terminated_sessions_are_refused() {
        let mut session = joined();
        assert_eq!(
            session.tool_action("host.exec", &json!({})),
            Err(BridgeError::UnknownTool {
                name: "host.exec".to_owned()
            })
        );
        let mut tampered = v1_tool_definitions();
        tampered[0].ceiling = "credential.read".to_owned();
        assert!(session.verify_definitions(&tampered).is_err());
        assert_eq!(
            session.tool_action("chat.post", &json!({ "text": "hi" })),
            Err(BridgeError::Terminated)
        );
    }

    #[test]
    fn invoke_builds_one_input_frame_from_the_call() {
        let mut session = joined();
        assert_eq!(
            session
                .tool_action(
                    "program.invoke",
                    &json!({ "program": "tic-tac-toe", "object": "board", "arguments": "4" })
                )
                .unwrap(),
            ToolAction::SendFrame(json!({ "type": "input", "data_hex": "34" }))
        );
        // The stale-decision token rides along when supplied.
        assert_eq!(
            session
                .tool_action(
                    "program.invoke",
                    &json!({
                        "program": "tic-tac-toe",
                        "object": "board",
                        "arguments": "0",
                        "screen_revision": 5,
                    })
                )
                .unwrap(),
            ToolAction::SendFrame(json!({
                "type": "input",
                "data_hex": "30",
                "screen_revision": 5,
            }))
        );
    }

    #[test]
    fn invoke_refuses_an_unnamed_program_object_or_non_string_arguments() {
        let mut session = joined();
        for arguments in [
            json!({ "object": "board", "arguments": "4" }),
            json!({ "program": "", "object": "board", "arguments": "4" }),
            json!({ "program": "tic-tac-toe", "arguments": "4" }),
            json!({ "program": "tic-tac-toe", "object": "", "arguments": "4" }),
            json!({ "program": "tic-tac-toe", "object": "board" }),
            json!({ "program": "tic-tac-toe", "object": "board", "arguments": 4 }),
        ] {
            assert!(
                matches!(
                    session.tool_action("program.invoke", &arguments),
                    Err(BridgeError::InvalidArguments {
                        tool: "program.invoke",
                        ..
                    })
                ),
                "{arguments} must be refused"
            );
        }
    }

    #[test]
    fn subscription_ack_marks_the_session_subscribed() {
        let mut session = joined();
        assert!(!session.is_subscribed());
        session.observe_frame(&json!({
            "type": "screen_subscription",
            "event_classes": ["chat_posted"],
            "view_revision": 4,
        }));
        assert!(session.is_subscribed());
        assert_eq!(session.view_revision(), 4);
    }
}
