//! The authority-free MCP bridge surface for the agent-in-worlds spike.
//!
//! The bridge is a translator: the world host remains responsible for
//! credentials, authority, quota, and audit. This module only defines the
//! narrow v1 tool surface and the session-start definition pin.

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

/// The seven approved v1 MCP tools, in their pinned wire order.
#[must_use]
pub fn v1_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::new("world.join", "session.presence", "presence"),
        ToolDefinition::new("world.leave", "session.presence", "presence"),
        ToolDefinition::new("screen.read", "screen.read", "screen.read"),
        ToolDefinition::new("screen.subscribe", "screen.subscribe", "screen.read"),
        ToolDefinition::new("object.inspect", "object.inspect", "screen.read"),
        ToolDefinition::new("chat.post", "chat.post", "chat.post"),
        ToolDefinition::new("presence.set", "presence.set", "presence"),
    ]
}

/// Errors that terminate a bridge session before a changed tool definition
/// can be used as a poisoning or shadowing vector.
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
}

/// A bridge session with its tool-definition hash pinned at start.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BridgeSession {
    pinned_definition_hash: [u8; 32],
    terminated: bool,
}

impl BridgeSession {
    /// Start a session and pin the exact ordered definition bytes.
    pub fn start(definitions: &[ToolDefinition]) -> Result<Self, BridgeError> {
        Ok(Self {
            pinned_definition_hash: definition_hash(definitions)?,
            terminated: false,
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
}

fn definition_hash(definitions: &[ToolDefinition]) -> Result<[u8; 32], BridgeError> {
    let bytes = serde_json::to_vec(definitions).map_err(|_| BridgeError::InvalidDefinitions)?;
    Ok(Sha256::digest(bytes).into())
}
