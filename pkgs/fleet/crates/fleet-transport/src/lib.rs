//! Fleet's reference transport routing core (S2 of code#13929).
//!
//! Declarations remain shared, language-neutral data in the root registry.
//! This crate resolves each envelope's outbound and reply directions
//! independently and checks each against its own declared capability ceiling.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// A declared adapter capability ceiling. This is deliberately not ordered:
/// `read` is receive-only, while `write-capable` is an outbound authority.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Level {
    /// The adapter does not carry this direction.
    None,
    /// Inbound observations only.
    Read,
    /// Messages can flow in both directions.
    Bidirectional,
    /// Outbound state-changing actions are possible, subject to a send gate.
    WriteCapable,
    /// The complete synchronization protocol can be carried.
    SyncEquivalent,
}

/// One direction of a route. The names mirror the declaration contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// The local stack sends an envelope to a peer.
    Send,
    /// The local stack expects to receive the response on the reply route.
    Receive,
}

/// Declared levels for the independent send and receive directions.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Levels {
    /// Outbound capability ceiling.
    pub send: Level,
    /// Inbound capability ceiling.
    pub receive: Level,
}

/// The declaration's human-readable gates, loaded from the shared registry.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct CapabilityGating {
    /// Gate protecting sends, including state-changing writes.
    pub send: String,
    /// Gate protecting receives.
    pub receive: String,
    /// How revocation reaches the adapter.
    pub revocation: String,
}

/// The subset of a language-neutral adapter declaration consumed by S2.
/// Other S1 fields are intentionally preserved in the JSON contract and are
/// ignored here until a later adapter slice needs them.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct AdapterDeclaration {
    /// Stable registry name.
    pub name: String,
    /// Version of this adapter declaration.
    pub version: u32,
    /// Per-direction declared ceilings.
    pub levels: Levels,
    /// Declared operation gates.
    pub capability_gating: CapabilityGating,
}

#[derive(Debug, Deserialize)]
struct RegistryDocument {
    schema_version: u32,
    adapters: Vec<AdapterDeclaration>,
}

/// The validated S1 declaration set used by route resolution.
#[derive(Clone, Debug, Default)]
pub struct Registry {
    adapters: BTreeMap<(String, u32), AdapterDeclaration>,
}

impl Registry {
    /// Build a registry from declarations, rejecting duplicate identities and
    /// malformed adapter names. A declaration's gate is checked when a
    /// write-capable operation is actually routed.
    pub fn new(adapters: Vec<AdapterDeclaration>) -> Result<Self, RouteError> {
        let mut registry = Self::default();
        for adapter in adapters {
            if adapter.name.trim().is_empty()
                || !adapter
                    .name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                || !adapter
                    .name
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_lowercase())
                || adapter.version == 0
            {
                return Err(RouteError::InvalidDeclaration {
                    adapter: adapter.name,
                    reason: "name or version is invalid".to_string(),
                });
            }
            if adapter.levels.send == Level::WriteCapable
                && adapter.capability_gating.send.trim().is_empty()
            {
                return Err(RouteError::MissingSendGate {
                    adapter: adapter.name,
                });
            }
            let key = (adapter.name.clone(), adapter.version);
            if registry.adapters.insert(key.clone(), adapter).is_some() {
                return Err(RouteError::DuplicateAdapter {
                    name: key.0,
                    version: key.1,
                });
            }
        }
        Ok(registry)
    }

    /// Parse schema-version-1 declarations from the shared S1 registry JSON.
    pub fn from_json(json: &str) -> Result<Self, RouteError> {
        let document: RegistryDocument = serde_json::from_str(json)
            .map_err(|error| RouteError::InvalidRegistry(error.to_string()))?;
        if document.schema_version != 1 {
            return Err(RouteError::UnsupportedSchema(document.schema_version));
        }
        Self::new(document.adapters)
    }

    /// Declarations in deterministic `(name, version)` order.
    pub fn adapters(&self) -> impl Iterator<Item = &AdapterDeclaration> {
        self.adapters.values()
    }

    /// Resolve the selected outbound target and the envelope's explicit
    /// reply route independently, checking each requested level against that
    /// adapter's own direction declaration.
    pub fn resolve<T>(
        &self,
        send: RouteTarget,
        send_level: Level,
        envelope: &Envelope<T>,
    ) -> Result<RoutePlan, RouteError> {
        if send.address.trim().is_empty() {
            return Err(RouteError::EmptyAddress {
                direction: Direction::Send,
            });
        }
        if envelope.reply_route.target.address.trim().is_empty() {
            return Err(RouteError::EmptyAddress {
                direction: Direction::Receive,
            });
        }

        let send_adapter = self.adapter_for(&send.adapter)?;
        let receive_adapter = self.adapter_for(&envelope.reply_route.target.adapter)?;
        self.enforce(send_adapter, Direction::Send, send_level)?;
        self.enforce(
            receive_adapter,
            Direction::Receive,
            envelope.reply_route.required_level,
        )?;

        Ok(RoutePlan {
            send,
            receive: envelope.reply_route.target.clone(),
        })
    }

    fn adapter_for(&self, adapter_ref: &AdapterRef) -> Result<&AdapterDeclaration, RouteError> {
        self.adapters
            .get(&(adapter_ref.name.clone(), adapter_ref.version))
            .ok_or_else(|| {
                let has_other_version = self
                    .adapters
                    .keys()
                    .any(|(name, _)| name == &adapter_ref.name);
                if has_other_version {
                    RouteError::VersionMismatch {
                        name: adapter_ref.name.clone(),
                        version: adapter_ref.version,
                    }
                } else {
                    RouteError::UnknownAdapter {
                        name: adapter_ref.name.clone(),
                    }
                }
            })
    }

    fn enforce(
        &self,
        adapter: &AdapterDeclaration,
        direction: Direction,
        required: Level,
    ) -> Result<(), RouteError> {
        let declared = match direction {
            Direction::Send => adapter.levels.send,
            Direction::Receive => adapter.levels.receive,
        };
        if direction == Direction::Send
            && required == Level::WriteCapable
            && adapter.capability_gating.send.trim().is_empty()
        {
            return Err(RouteError::MissingSendGate {
                adapter: adapter.name.clone(),
            });
        }
        if !supports(direction, declared, required) {
            return Err(RouteError::InsufficientLevel {
                adapter: adapter.name.clone(),
                direction,
                declared,
                required,
            });
        }
        Ok(())
    }
}

/// Whether a declaration satisfies an operation in one direction.
///
/// This is an explicit capability matrix, not an ordinal comparison. In
/// particular, read is receive-only and write-capable is send-only.
pub fn supports(direction: Direction, declared: Level, required: Level) -> bool {
    use Direction::{Receive, Send};
    use Level::{Bidirectional, None, Read, SyncEquivalent, WriteCapable};

    match (direction, required) {
        (Send, Read | None) | (Receive, WriteCapable | None) => false,
        (Send, Bidirectional) | (Receive, Bidirectional) => match direction {
            Send => matches!(declared, Bidirectional | WriteCapable | SyncEquivalent),
            Receive => matches!(declared, Bidirectional | SyncEquivalent),
        },
        (Send, WriteCapable) => matches!(declared, WriteCapable | SyncEquivalent),
        (Receive, Read) => matches!(declared, Read | Bidirectional | SyncEquivalent),
        (Send, SyncEquivalent) | (Receive, SyncEquivalent) => {
            matches!(declared, SyncEquivalent)
        }
    }
}

/// A versioned adapter identity used on the wire.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct AdapterRef {
    /// Adapter name from the registry.
    pub name: String,
    /// Adapter declaration version.
    pub version: u32,
}

/// Carrier-specific address paired with an adapter identity.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct RouteTarget {
    /// The adapter to use for this direction.
    pub adapter: AdapterRef,
    /// Address interpreted by that adapter; it is not an identity proof.
    pub address: String,
}

/// Route explicitly supplied for the response direction of an envelope.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ReplyRoute {
    /// Independent adapter/address for the expected reply.
    pub target: RouteTarget,
    /// Minimum receive capability required for the reply operation.
    pub required_level: Level,
}

/// An envelope's payload plus an explicit route for its response.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Envelope<T> {
    /// Application payload; S2 does not interpret it.
    pub payload: T,
    /// The reply path is explicit and need not match the send path.
    pub reply_route: ReplyRoute,
}

/// Independently resolved send and reply paths for one envelope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoutePlan {
    /// Selected outbound route.
    pub send: RouteTarget,
    /// Selected inbound/reply route from the envelope.
    pub receive: RouteTarget,
}

/// Fail-closed errors from declaration loading and route resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteError {
    /// Invalid adapter identity in a programmatically built registry.
    InvalidDeclaration { adapter: String, reason: String },
    /// Duplicate `(name, version)` in the declaration set.
    DuplicateAdapter { name: String, version: u32 },
    /// Registry JSON is malformed or omits required S1 fields.
    InvalidRegistry(String),
    /// This runtime only understands S1 schema version 1.
    UnsupportedSchema(u32),
    /// No registered adapter has this name.
    UnknownAdapter { name: String },
    /// The name exists, but not at the requested declaration version.
    VersionMismatch { name: String, version: u32 },
    /// The route's requested operation exceeds the selected direction.
    InsufficientLevel {
        adapter: String,
        direction: Direction,
        declared: Level,
        required: Level,
    },
    /// A write-capable send was requested without a declared send gate.
    MissingSendGate { adapter: String },
    /// The selected direction has no usable carrier address.
    EmptyAddress { direction: Direction },
}

impl fmt::Display for RouteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDeclaration { adapter, reason } => {
                write!(f, "invalid adapter declaration {adapter:?}: {reason}")
            }
            Self::DuplicateAdapter { name, version } => {
                write!(f, "duplicate adapter declaration {name}@{version}")
            }
            Self::InvalidRegistry(message) => {
                write!(f, "invalid transport adapter registry: {message}")
            }
            Self::UnsupportedSchema(version) => {
                write!(f, "unsupported transport adapter schema version {version}")
            }
            Self::UnknownAdapter { name } => write!(f, "unknown transport adapter {name:?}"),
            Self::VersionMismatch { name, version } => write!(
                f,
                "transport adapter {name:?} has no registered version {version}"
            ),
            Self::InsufficientLevel {
                adapter,
                direction,
                declared,
                required,
            } => write!(
                f,
                "adapter {adapter:?} declares {declared:?} for {direction:?}, below requested {required:?}"
            ),
            Self::MissingSendGate { adapter } => write!(
                f,
                "adapter {adapter:?} cannot send write-capable operations without a declared send gate"
            ),
            Self::EmptyAddress { direction } => {
                write!(f, "empty carrier address for {direction:?} route")
            }
        }
    }
}

impl std::error::Error for RouteError {}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn declaration(name: &str, send: Level, receive: Level, send_gate: &str) -> AdapterDeclaration {
        AdapterDeclaration {
            name: name.to_owned(),
            version: 1,
            levels: Levels { send, receive },
            capability_gating: CapabilityGating {
                send: send_gate.to_owned(),
                receive: "receive authorization".to_owned(),
                revocation: "receipt-time check".to_owned(),
            },
        }
    }

    fn target(adapter: &str, address: &str) -> RouteTarget {
        RouteTarget {
            adapter: AdapterRef {
                name: adapter.to_owned(),
                version: 1,
            },
            address: address.to_owned(),
        }
    }

    #[derive(Debug)]
    struct LoopbackAdapter(&'static str);

    impl LoopbackAdapter {
        fn send(&self, target: &RouteTarget, payload: &str) -> String {
            assert_eq!(target.adapter.name, self.0);
            format!("{} sent {payload} to {}", self.0, target.address)
        }

        fn receive_reply(&self, target: &RouteTarget, payload: &str) -> String {
            assert_eq!(target.adapter.name, self.0);
            format!("{} received {payload} at {}", self.0, target.address)
        }
    }

    #[test]
    fn loopback_request_uses_carrier_a_and_reply_uses_carrier_b() {
        let registry = Registry::new(vec![
            declaration(
                "carrier-a",
                Level::Bidirectional,
                Level::Bidirectional,
                "send gate",
            ),
            declaration("carrier-b", Level::Read, Level::Read, ""),
        ])
        .unwrap();
        let envelope = Envelope {
            payload: "request",
            reply_route: ReplyRoute {
                target: target("carrier-b", "back-to-sender"),
                required_level: Level::Read,
            },
        };
        let plan = registry
            .resolve(
                target("carrier-a", "to-peer"),
                Level::Bidirectional,
                &envelope,
            )
            .unwrap();
        let carrier_a = LoopbackAdapter("carrier-a");
        let carrier_b = LoopbackAdapter("carrier-b");

        assert_eq!(
            carrier_a.send(&plan.send, envelope.payload),
            "carrier-a sent request to to-peer"
        );
        assert_eq!(
            carrier_b.receive_reply(&plan.receive, "reply"),
            "carrier-b received reply at back-to-sender"
        );
    }

    #[test]
    fn an_authority_request_above_the_send_ceiling_is_rejected_as_downgrade() {
        let registry = Registry::new(vec![declaration(
            "message-only",
            Level::Bidirectional,
            Level::Read,
            "send gate",
        )])
        .unwrap();
        let envelope = Envelope {
            payload: "state change",
            reply_route: ReplyRoute {
                target: target("message-only", "reply"),
                required_level: Level::Read,
            },
        };

        let result = registry.resolve(
            target("message-only", "peer"),
            Level::WriteCapable,
            &envelope,
        );

        assert!(matches!(
            result,
            Err(RouteError::InsufficientLevel {
                direction: Direction::Send,
                ..
            })
        ));
    }

    #[test]
    fn write_capable_send_requires_a_declared_send_gate() {
        let error = Registry::new(vec![declaration(
            "ungated",
            Level::WriteCapable,
            Level::Read,
            "",
        )])
        .unwrap_err();

        assert!(matches!(error, RouteError::MissingSendGate { .. }));
    }

    #[test]
    fn registry_deserializes_the_shared_language_neutral_contract() {
        let json = include_str!("../../../../../config/transport-adapters/registry.json");
        let registry = Registry::from_json(json).unwrap();
        assert_eq!(registry.adapters().count(), 4);
    }

    #[test]
    fn operation_capability_matrix_matches_the_contract_oracle() {
        use Direction::{Receive, Send};
        use Level::{Bidirectional, None, Read, SyncEquivalent, WriteCapable};

        let cases = [
            (Send, None, Read, false),
            (Send, Read, Bidirectional, false),
            (Send, Bidirectional, Bidirectional, true),
            (Send, SyncEquivalent, WriteCapable, true),
            (Receive, Read, Read, true),
            (Receive, Bidirectional, Read, true),
            (Receive, Read, Bidirectional, false),
            (Receive, SyncEquivalent, SyncEquivalent, true),
            (Receive, WriteCapable, Read, false),
        ];
        for (direction, declared, required, expected) in cases {
            assert_eq!(
                supports(direction, declared, required),
                expected,
                "{direction:?} {declared:?} for {required:?}"
            );
        }
    }

    fn level_from_index(value: u8) -> Level {
        match value % 5 {
            0 => Level::None,
            1 => Level::Read,
            2 => Level::Bidirectional,
            3 => Level::WriteCapable,
            _ => Level::SyncEquivalent,
        }
    }

    // Independent named oracle: each row follows the level definitions and
    // asymmetric-routing rule in #13929 §4.2-4.3, not a numeric rank.
    fn capability_matrix_oracle(direction: Direction, declared: Level, required: Level) -> bool {
        match (direction, declared, required) {
            (
                Direction::Send,
                Level::Bidirectional | Level::WriteCapable | Level::SyncEquivalent,
                Level::Bidirectional,
            ) => true,
            (Direction::Send, Level::WriteCapable | Level::SyncEquivalent, Level::WriteCapable) => {
                true
            }
            (Direction::Send, Level::SyncEquivalent, Level::SyncEquivalent) => true,
            (
                Direction::Receive,
                Level::Read | Level::Bidirectional | Level::SyncEquivalent,
                Level::Read,
            ) => true,
            (
                Direction::Receive,
                Level::Bidirectional | Level::SyncEquivalent,
                Level::Bidirectional,
            ) => true,
            (Direction::Receive, Level::SyncEquivalent, Level::SyncEquivalent) => true,
            _ => false,
        }
    }

    proptest! {
        #[test]
        fn generated_route_plans_match_named_contract_oracle(
            send_declared_index in 0u8..5,
            receive_declared_index in 0u8..5,
            send_required_index in 0u8..5,
            receive_required_index in 0u8..5,
        ) {
            let send_declared = level_from_index(send_declared_index);
            let receive_declared = level_from_index(receive_declared_index);
            let send_required = level_from_index(send_required_index);
            let receive_required = level_from_index(receive_required_index);
            let registry = Registry::new(vec![
                declaration("send-path", send_declared, Level::None, "gate"),
                declaration("reply-path", Level::None, receive_declared, ""),
            ]).unwrap();
            let envelope = Envelope {
                payload: (),
                reply_route: ReplyRoute {
                    target: target("reply-path", "reply"),
                    required_level: receive_required,
                },
            };

            let route = registry.resolve(target("send-path", "peer"), send_required, &envelope);
            let expected = capability_matrix_oracle(Direction::Send, send_declared, send_required)
                && capability_matrix_oracle(Direction::Receive, receive_declared, receive_required);
            prop_assert_eq!(route.is_ok(), expected);
            if let Ok(plan) = route {
                prop_assert_eq!(plan.send.adapter.name, "send-path");
                prop_assert_eq!(plan.receive.adapter.name, "reply-path");
            }
        }
    }
}
