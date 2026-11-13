//! Versioned CBOR encoding for group-layer events (`group.event`).
//!
//! The format is deliberately separate from `group.op`: it carries the event
//! model in [`super::Event`] without changing p2panda-auth's `GroupAction`
//! contract. Its envelope is a canonical CBOR array:
//!
//! ```text
//! ["group.event", version, id, author, after, group, [kind, args]]
//! ```
//!
//! `id`, `author`, each hash in `after`, and a non-null `group` are 32-byte
//! byte strings. The body `kind` is an open text name; its args are a compact
//! array. Structured policy and create-policy values are canonical CBOR maps
//! using the model's stable field names. Version 1 kinds are `create`, `join`,
//! `friendship.decline`, `policy`, `member.add`, `member.remove`,
//! `member.set-role`, `delegate`, `cross`, `veto`, `device.add`, and
//! `device.revoke`. A friendship request is a `create` with the Friendship
//! template; `join` is its acceptance. Decline is distinct from dissolution:
//! dissolution remains the existing member-removal plus policy behavior.
//! Template codes are `0` Custom, `1` Friendship, `2` Channel, and `3` Private.
//!
//! Decoders require canonical CBOR, the exact envelope and per-kind arity,
//! known kinds, known version-1 fields, and no trailing data. Unknown kinds or
//! versions are errors; extensions require a new version. Events are bounded
//! to 1 MiB and 256 causal predecessors / create members. These limits apply
//! before decoding large structures as well as before encoding.

use std::collections::HashSet;

use cbor_core::{DecodeOptions, Strictness, Value};
use p2panda_core::Hash;

use super::policy::{RemoveMode, RemoveSuccessor, Template, TransferOpts};
use super::{Body, Create, Delegate, DeviceOp, Event, Key, MemberOp, Role};

/// The tag for this body family.
pub const TAG: &str = "group.event";

/// The version written and read by this implementation.
pub const VERSION: u64 = 1;

/// Maximum encoded event size, including the envelope.
pub const MAX_EVENT_BYTES: usize = 1024 * 1024;

/// Maximum number of causal predecessors in one event.
pub const MAX_AFTER: usize = 256;

/// Maximum number of members in a group creation event.
pub const MAX_CREATE_MEMBERS: usize = 256;

const RECURSION_LIMIT: u16 = 16;
const LENGTH_LIMIT: u64 = MAX_EVENT_BYTES as u64;
const OOM_MITIGATION: usize = 64;

/// A body accepted by the wire format. Decline is wire-level so existing
/// local group folding and friendship/dissolution semantics remain unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventBody {
    /// One of the existing group model's event bodies.
    Group(Body),
    /// Explicitly declines a pending friendship request.
    FriendshipDecline,
}

/// A group event in the wire family. It mirrors [`super::Event`], with the
/// additional wire-only friendship-decline body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupEvent {
    /// The event ID.
    pub id: Hash,
    /// The signing account.
    pub author: Key,
    /// The event's causal predecessors.
    pub after: Vec<Hash>,
    /// The named group, when applicable.
    pub group: Option<Hash>,
    /// The event body.
    pub body: EventBody,
}

impl From<Event> for GroupEvent {
    fn from(event: Event) -> Self {
        Self {
            id: event.id,
            author: event.author,
            after: event.after,
            group: event.group,
            body: EventBody::Group(event.body),
        }
    }
}

/// Why an event could not be encoded or decoded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EventError {
    /// The encoded event exceeds [`MAX_EVENT_BYTES`].
    #[error("a group event is {0} bytes, the most allowed is {max}", max = MAX_EVENT_BYTES)]
    Oversized(usize),
    /// Strict CBOR decoding failed.
    #[error("the group event is not CBOR: {0}")]
    Cbor(String),
    /// The bytes do not use the canonical encoding of their CBOR value.
    #[error("the group event is not canonical CBOR")]
    NotCanonical,
    /// The tag names a different body family.
    #[error("the group event tag is {0:?}, not {TAG:?}")]
    Tag(String),
    /// The wire version is not supported.
    #[error("the group event has version {0}, this code reads version {VERSION}")]
    Version(u64),
    /// The kind is not part of version 1.
    #[error("the group event kind {0:?} is unknown")]
    Kind(String),
    /// The envelope or body has an invalid shape or value.
    #[error("the group event has the wrong shape: {0}")]
    Shape(String),
}

/// Encode a model event as canonical CBOR.
///
/// # Errors
///
/// Returns [`EventError`] if the event exceeds a bound or contains an
/// unresolvable `ServerDefault` removal mode.
pub fn encode(event: &Event) -> Result<Vec<u8>, EventError> {
    encode_group_event(&GroupEvent {
        id: event.id,
        author: event.author,
        after: event.after.clone(),
        group: event.group,
        body: EventBody::Group(event.body.clone()),
    })
}

/// Encode a wire event, including an explicit friendship decline.
pub fn encode_group_event(event: &GroupEvent) -> Result<Vec<u8>, EventError> {
    check_bounds(event)?;
    let (kind, args) = body_value(&event.body)?;
    let after = event.after.iter().map(hash_value).collect();
    let value = Value::Array(vec![
        Value::TextString(TAG.to_owned().into()),
        Value::Unsigned(VERSION),
        hash_value(&event.id),
        key_value(&event.author),
        Value::Array(after),
        event.group.as_ref().map_or_else(Value::null, hash_value),
        Value::Array(vec![Value::TextString(kind.to_owned().into()), args]),
    ]);
    let bytes = value.encode();
    if bytes.len() > MAX_EVENT_BYTES {
        return Err(EventError::Oversized(bytes.len()));
    }
    Ok(bytes)
}

/// Decode one canonical `group.event` value.
///
/// # Errors
///
/// Returns [`EventError`] for malformed, non-canonical, unsupported, or
/// oversized input. Bytes after the single CBOR value are rejected.
pub fn decode(bytes: &[u8]) -> Result<GroupEvent, EventError> {
    if bytes.len() > MAX_EVENT_BYTES {
        return Err(EventError::Oversized(bytes.len()));
    }
    let value = DecodeOptions::new()
        .strictness(Strictness::STRICT)
        .recursion_limit(RECURSION_LIMIT)
        .length_limit(LENGTH_LIMIT)
        .oom_mitigation(OOM_MITIGATION)
        .decode_owned(bytes)
        .map_err(|error| EventError::Cbor(error.to_string()))?;
    if value.encode().as_slice() != bytes {
        return Err(EventError::NotCanonical);
    }
    let [tag, version, id, author, after, group, body] = list(&value, 7)?;
    let Value::TextString(tag) = tag else {
        return Err(EventError::Shape("tag is not text".to_owned()));
    };
    if tag != TAG {
        return Err(EventError::Tag(tag.to_string()));
    }
    let version = unsigned(version)?;
    if version != VERSION {
        return Err(EventError::Version(version));
    }
    let id = decode_hash(id, "event ID")?;
    let author = decode_key(author, "author")?;
    let after_values = array(after, "after")?;
    if after_values.len() > MAX_AFTER {
        return Err(EventError::Shape(format!(
            "{} predecessors, the most is {MAX_AFTER}",
            after_values.len()
        )));
    }
    let after = after_values
        .iter()
        .map(|value| decode_hash(value, "predecessor"))
        .collect::<Result<Vec<_>, _>>()?;
    let group = match group {
        value if is_null(value) => None,
        value => Some(decode_hash(value, "group ID")?),
    };
    let [kind, args] = list(body, 2)?;
    let Value::TextString(kind) = kind else {
        return Err(EventError::Shape("body kind is not text".to_owned()));
    };
    let body = decode_body(kind, args)?;
    let event = GroupEvent {
        id,
        author,
        after,
        group,
        body,
    };
    check_bounds(&event)?;
    Ok(event)
}

fn check_bounds(event: &GroupEvent) -> Result<(), EventError> {
    if event.after.len() > MAX_AFTER {
        return Err(EventError::Shape(format!(
            "{} predecessors, the most is {MAX_AFTER}",
            event.after.len()
        )));
    }
    if let EventBody::Group(Body::Create(create)) = &event.body
        && create.members.len() > MAX_CREATE_MEMBERS
    {
        return Err(EventError::Shape(format!(
            "{} create members, the most is {MAX_CREATE_MEMBERS}",
            create.members.len()
        )));
    }
    let mut seen_after = HashSet::with_capacity(event.after.len());
    for predecessor in &event.after {
        if *predecessor == event.id {
            return Err(EventError::Shape(
                "an event may not name itself as a predecessor".to_owned(),
            ));
        }
        if !seen_after.insert(predecessor) {
            return Err(EventError::Shape(
                "after names the same predecessor twice (duplicate predecessor)".to_owned(),
            ));
        }
    }
    if let EventBody::Group(Body::Create(create)) = &event.body {
        let mut seen_members = HashSet::with_capacity(create.members.len());
        for (key, _role) in &create.members {
            if !seen_members.insert(key) {
                return Err(EventError::Shape(
                    "create members name the same member twice (duplicate create member)"
                        .to_owned(),
                ));
            }
        }
    }
    if matches!(&event.body, EventBody::FriendshipDecline) && event.group.is_none() {
        return Err(EventError::Shape(
            "a friendship decline must name its group".to_owned(),
        ));
    }
    Ok(())
}

fn body_value(body: &EventBody) -> Result<(&'static str, Value<'static>), EventError> {
    let (kind, args) = match body {
        EventBody::FriendshipDecline => ("friendship.decline", Value::Array(Vec::new())),
        EventBody::Group(Body::Create(create)) => {
            let template = match create.template {
                Template::Custom => 0,
                Template::Friendship => 1,
                Template::Channel => 2,
                Template::Private => 3,
            };
            let members = create
                .members
                .iter()
                .map(|(key, role)| Value::Array(vec![key_value(key), role_value(*role)]))
                .collect();
            (
                "create",
                Value::Array(vec![
                    Value::Unsigned(template),
                    serialized(&create.policy)?,
                    Value::Array(members),
                    Value::from_bool(create.admins_on_create),
                ]),
            )
        }
        EventBody::Group(Body::Join) => ("join", Value::Array(Vec::new())),
        EventBody::Group(Body::Policy(policy)) => ("policy", serialized(policy)?),
        EventBody::Group(Body::Member(MemberOp::Add { member, role })) => (
            "member.add",
            Value::Array(vec![key_value(member), role_value(*role)]),
        ),
        EventBody::Group(Body::Member(MemberOp::Remove { member, mode })) => (
            "member.remove",
            Value::Array(vec![key_value(member), remove_mode_value(*mode)?]),
        ),
        EventBody::Group(Body::Member(MemberOp::SetRole { member, role })) => (
            "member.set-role",
            Value::Array(vec![key_value(member), role_value(*role)]),
        ),
        EventBody::Group(Body::Delegate(delegate)) => ("delegate", delegate_value(delegate)),
        EventBody::Group(Body::Cross { delegate, world }) => (
            "cross",
            Value::Array(vec![
                delegate_value(delegate),
                Value::TextString(world.clone().into()),
            ]),
        ),
        EventBody::Group(Body::Veto { grant }) => ("veto", hash_value(grant)),
        EventBody::Group(Body::Device(DeviceOp::Add { device, scope })) => (
            "device.add",
            Value::Array(vec![key_value(device), role_value(*scope)]),
        ),
        EventBody::Group(Body::Device(DeviceOp::Revoke { device })) => {
            ("device.revoke", key_value(device))
        }
    };
    Ok((kind, args))
}

fn decode_body(kind: &str, args: &Value<'static>) -> Result<EventBody, EventError> {
    let body = match kind {
        "create" => {
            let [template, policy, members, admins] = list(args, 4)?;
            let template = match unsigned(template)? {
                0 => Template::Custom,
                1 => Template::Friendship,
                2 => Template::Channel,
                3 => Template::Private,
                other => {
                    return Err(EventError::Shape(format!(
                        "unknown create template {other}"
                    )));
                }
            };
            let policy = deserialized(policy)?;
            let member_values = array(members, "create members")?;
            if member_values.len() > MAX_CREATE_MEMBERS {
                return Err(EventError::Shape(format!(
                    "{} create members, the most is {MAX_CREATE_MEMBERS}",
                    member_values.len()
                )));
            }
            let members = member_values
                .iter()
                .map(|member| {
                    let [key, role] = list(member, 2)?;
                    Ok((decode_key(key, "create member")?, decode_role(role)?))
                })
                .collect::<Result<Vec<_>, EventError>>()?;
            let admins_on_create = decode_bool(admins, "admins_on_create")?;
            Body::Create(Create {
                template,
                policy,
                members,
                admins_on_create,
            })
        }
        "join" => {
            empty_args(args, kind)?;
            Body::Join
        }
        "friendship.decline" => {
            empty_args(args, kind)?;
            return Ok(EventBody::FriendshipDecline);
        }
        "policy" => Body::Policy(deserialized(args)?),
        "member.add" => {
            let [member, role] = list(args, 2)?;
            Body::Member(MemberOp::Add {
                member: decode_key(member, "member")?,
                role: decode_role(role)?,
            })
        }
        "member.remove" => {
            let [member, mode] = list(args, 2)?;
            Body::Member(MemberOp::Remove {
                member: decode_key(member, "member")?,
                mode: decode_remove_mode(mode)?,
            })
        }
        "member.set-role" => {
            let [member, role] = list(args, 2)?;
            Body::Member(MemberOp::SetRole {
                member: decode_key(member, "member")?,
                role: decode_role(role)?,
            })
        }
        "delegate" => Body::Delegate(decode_delegate(args)?),
        "cross" => {
            let [delegate, world] = list(args, 2)?;
            let Value::TextString(world) = world else {
                return Err(EventError::Shape("cross world is not text".to_owned()));
            };
            Body::Cross {
                delegate: decode_delegate(delegate)?,
                world: world.to_string(),
            }
        }
        "veto" => Body::Veto {
            grant: decode_hash(args, "veto target")?,
        },
        "device.add" => {
            let [device, scope] = list(args, 2)?;
            Body::Device(DeviceOp::Add {
                device: decode_key(device, "device")?,
                scope: decode_role(scope)?,
            })
        }
        "device.revoke" => Body::Device(DeviceOp::Revoke {
            device: decode_key(args, "device")?,
        }),
        other => return Err(EventError::Kind(other.to_owned())),
    };
    Ok(EventBody::Group(body))
}

fn delegate_value(delegate: &Delegate) -> Value<'static> {
    Value::Array(vec![
        key_value(&delegate.grantee),
        role_value(delegate.scope),
        Value::from_bool(delegate.can_redelegate),
    ])
}

fn decode_delegate(value: &Value<'static>) -> Result<Delegate, EventError> {
    let [grantee, scope, can_redelegate] = list(value, 3)?;
    Ok(Delegate {
        grantee: decode_key(grantee, "delegate grantee")?,
        scope: decode_role(scope)?,
        can_redelegate: decode_bool(can_redelegate, "can_redelegate")?,
    })
}

fn remove_mode_value(mode: RemoveMode) -> Result<Value<'static>, EventError> {
    Ok(match mode {
        RemoveMode::Revoke => Value::Array(vec![Value::Unsigned(0)]),
        RemoveMode::Transfer(TransferOpts { successor }) => {
            let successor = match successor {
                RemoveSuccessor::Member(key) => {
                    Value::Array(vec![Value::Unsigned(0), key_value(&key)])
                }
                RemoveSuccessor::Admin => Value::Array(vec![Value::Unsigned(1)]),
                RemoveSuccessor::Designated(key) => {
                    Value::Array(vec![Value::Unsigned(2), key_value(&key)])
                }
                RemoveSuccessor::Actor => Value::Array(vec![Value::Unsigned(3)]),
            };
            Value::Array(vec![Value::Unsigned(1), successor])
        }
        RemoveMode::KeepPrior => Value::Array(vec![Value::Unsigned(2)]),
        RemoveMode::ReassignToGroup => Value::Array(vec![Value::Unsigned(3)]),
        RemoveMode::ServerDefault => {
            return Err(EventError::Shape(
                "ServerDefault must be resolved before an event is encoded".to_owned(),
            ));
        }
    })
}

fn decode_remove_mode(value: &Value<'static>) -> Result<RemoveMode, EventError> {
    let values = array(value, "remove mode")?;
    let Some(kind) = values.first() else {
        return Err(EventError::Shape("remove mode is empty".to_owned()));
    };
    match unsigned(kind)? {
        0 if values.len() == 1 => Ok(RemoveMode::Revoke),
        1 if values.len() == 2 => {
            let successor_values = array(&values[1], "transfer successor")?;
            let Some(kind) = successor_values.first() else {
                return Err(EventError::Shape("transfer successor is empty".to_owned()));
            };
            let successor = match (unsigned(kind)?, successor_values) {
                (0, [_, key]) => RemoveSuccessor::Member(decode_key(key, "member successor")?),
                (1, [_]) => RemoveSuccessor::Admin,
                (2, [_, key]) => {
                    RemoveSuccessor::Designated(decode_key(key, "designated successor")?)
                }
                (3, [_]) => RemoveSuccessor::Actor,
                (other, _) => {
                    return Err(EventError::Shape(format!(
                        "invalid transfer successor kind or arity {other}"
                    )));
                }
            };
            Ok(RemoveMode::Transfer(TransferOpts { successor }))
        }
        2 if values.len() == 1 => Ok(RemoveMode::KeepPrior),
        3 if values.len() == 1 => Ok(RemoveMode::ReassignToGroup),
        other => Err(EventError::Shape(format!("invalid remove mode {other}"))),
    }
}

fn role_value(role: Role) -> Value<'static> {
    Value::Unsigned(match role {
        Role::Pull => 0,
        Role::Read => 1,
        Role::Write => 2,
        Role::Manage => 3,
        Role::Admin => 4,
    })
}

fn decode_role(value: &Value<'static>) -> Result<Role, EventError> {
    match unsigned(value)? {
        0 => Ok(Role::Pull),
        1 => Ok(Role::Read),
        2 => Ok(Role::Write),
        3 => Ok(Role::Manage),
        4 => Ok(Role::Admin),
        other => Err(EventError::Shape(format!("unknown role {other}"))),
    }
}

fn serialized<T: serde::Serialize>(value: &T) -> Result<Value<'static>, EventError> {
    Value::serialized(value).map_err(|error| EventError::Shape(error.to_string()))
}

fn deserialized<T>(value: &Value<'static>) -> Result<T, EventError>
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let decoded: T = value
        .deserialized()
        .map_err(|error| EventError::Shape(error.to_string()))?;
    if serialized(&decoded)?.encode() != value.encode() {
        return Err(EventError::Shape(
            "structured args contain unknown, omitted, or non-version-1 fields".to_owned(),
        ));
    }
    Ok(decoded)
}

fn empty_args(value: &Value<'static>, kind: &str) -> Result<(), EventError> {
    if array(value, "empty body args")?.is_empty() {
        Ok(())
    } else {
        Err(EventError::Shape(format!("{kind} takes no args")))
    }
}

fn hash_value(hash: &Hash) -> Value<'static> {
    Value::ByteString(hash.as_bytes().to_vec().into())
}

fn key_value(key: &Key) -> Value<'static> {
    Value::ByteString(key.to_vec().into())
}

fn decode_hash(value: &Value<'static>, name: &str) -> Result<Hash, EventError> {
    let bytes = byte_string(value, name)?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| EventError::Shape(format!("{name} is not 32 bytes")))?;
    Ok(Hash::from_bytes(bytes))
}

fn decode_key(value: &Value<'static>, name: &str) -> Result<Key, EventError> {
    let bytes = byte_string(value, name)?;
    bytes
        .try_into()
        .map_err(|_| EventError::Shape(format!("{name} is {} bytes, not 32", bytes.len())))
}

fn is_null(value: &Value<'static>) -> bool {
    value.to_simple_value().ok() == Some(22) && value.encode() == Value::null().encode()
}

fn decode_bool(value: &Value<'static>, name: &str) -> Result<bool, EventError> {
    let boolean = value
        .to_bool()
        .map_err(|_| EventError::Shape(format!("{name} is not boolean")))?;
    if Value::from_bool(boolean).encode() != value.encode() {
        return Err(EventError::Shape(format!("{name} is not a plain boolean")));
    }
    Ok(boolean)
}

fn byte_string<'a>(value: &'a Value<'static>, name: &str) -> Result<&'a [u8], EventError> {
    match value {
        Value::ByteString(bytes) => Ok(bytes.as_ref()),
        _ => Err(EventError::Shape(format!("{name} is not a byte string"))),
    }
}

fn list<'a, const N: usize>(
    value: &'a Value<'static>,
    expected: usize,
) -> Result<&'a [Value<'static>; N], EventError> {
    let values = array(value, "array")?;
    if values.len() != expected || expected != N {
        return Err(EventError::Shape(format!(
            "array has {} items, expected {expected}",
            values.len()
        )));
    }
    values.try_into().map_err(|_| {
        EventError::Shape(format!(
            "array has {} items, expected {expected}",
            values.len()
        ))
    })
}

fn array<'a>(value: &'a Value<'static>, name: &str) -> Result<&'a [Value<'static>], EventError> {
    match value {
        Value::Array(values) => Ok(values),
        _ => Err(EventError::Shape(format!("{name} is not an array"))),
    }
}

fn unsigned(value: &Value<'static>) -> Result<u64, EventError> {
    match value {
        Value::Unsigned(value) => Ok(*value),
        _ => Err(EventError::Shape(
            "value is not an unsigned integer".to_owned(),
        )),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::groups::policy::{Delegation, History, OnMemberRemoved, Policy, Removal};

    fn sample(body: EventBody) -> GroupEvent {
        GroupEvent {
            id: Hash::from_bytes([1; 32]),
            author: [2; 32],
            after: vec![Hash::from_bytes([4; 32]), Hash::from_bytes([5; 32])],
            group: Some(Hash::from_bytes([3; 32])),
            body,
        }
    }

    fn bodies() -> Vec<EventBody> {
        let policy = Policy {
            delegation: Some(Delegation::Cascade),
            history: History::Recent(9),
            on_member_removed: OnMemberRemoved::Dissolve,
            removal: Removal::Weak,
            ..Policy::default()
        };
        vec![
            EventBody::Group(Body::Create(Create {
                template: Template::Friendship,
                policy: Policy::friendship(),
                members: vec![([6; 32], Role::Manage)],
                admins_on_create: true,
            })),
            EventBody::Group(Body::Join),
            EventBody::FriendshipDecline,
            EventBody::Group(Body::Policy(policy)),
            EventBody::Group(Body::Member(MemberOp::Add {
                member: [6; 32],
                role: Role::Read,
            })),
            EventBody::Group(Body::Member(MemberOp::Remove {
                member: [6; 32],
                mode: RemoveMode::Transfer(TransferOpts {
                    successor: RemoveSuccessor::Designated([7; 32]),
                }),
            })),
            EventBody::Group(Body::Member(MemberOp::SetRole {
                member: [6; 32],
                role: Role::Admin,
            })),
            EventBody::Group(Body::Delegate(Delegate {
                grantee: [6; 32],
                scope: Role::Write,
                can_redelegate: true,
            })),
            EventBody::Group(Body::Cross {
                delegate: Delegate {
                    grantee: [6; 32],
                    scope: Role::Read,
                    can_redelegate: false,
                },
                world: "another-world".to_owned(),
            }),
            EventBody::Group(Body::Veto {
                grant: Hash::from_bytes([8; 32]),
            }),
            EventBody::Group(Body::Device(DeviceOp::Add {
                device: [6; 32],
                scope: Role::Write,
            })),
            EventBody::Group(Body::Device(DeviceOp::Revoke { device: [6; 32] })),
        ]
    }

    #[test]
    fn group_event_wire_round_trips_every_kind() {
        for body in bodies() {
            let event = sample(body);
            let bytes = encode_group_event(&event).unwrap();
            assert_eq!(decode(&bytes).unwrap(), event);
        }
    }

    #[test]
    fn group_event_wire_has_a_stable_decline_golden_vector() {
        let event = GroupEvent {
            id: Hash::from_bytes([1; 32]),
            author: [2; 32],
            after: vec![],
            group: Some(Hash::from_bytes([3; 32])),
            body: EventBody::FriendshipDecline,
        };
        let mut expected = vec![0x87, 0x6b];
        expected.extend_from_slice(b"group.event");
        expected.extend_from_slice(&[0x01, 0x58, 0x20]);
        expected.extend_from_slice(&[1; 32]);
        expected.extend_from_slice(&[0x58, 0x20]);
        expected.extend_from_slice(&[2; 32]);
        expected.push(0x80);
        expected.extend_from_slice(&[0x58, 0x20]);
        expected.extend_from_slice(&[3; 32]);
        expected.extend_from_slice(&[0x82, 0x72]);
        expected.extend_from_slice(b"friendship.decline");
        expected.push(0x80);
        assert_eq!(encode_group_event(&event).unwrap(), expected);
    }

    #[test]
    fn group_event_wire_has_a_stable_join_golden_vector() {
        let event = GroupEvent {
            id: Hash::from_bytes([1; 32]),
            author: [2; 32],
            after: vec![],
            group: None,
            body: EventBody::Group(Body::Join),
        };
        let mut expected = vec![0x87, 0x6b];
        expected.extend_from_slice(b"group.event");
        expected.extend_from_slice(&[0x01, 0x58, 0x20]);
        expected.extend_from_slice(&[1; 32]);
        expected.extend_from_slice(&[0x58, 0x20]);
        expected.extend_from_slice(&[2; 32]);
        expected.extend_from_slice(&[0x80, 0xf6, 0x82, 0x64]);
        expected.extend_from_slice(b"join");
        expected.push(0x80);
        assert_eq!(encode_group_event(&event).unwrap(), expected);
    }

    #[test]
    fn group_event_wire_rejects_bad_tag_kind_version_trailing_data_and_size() {
        let event = sample(EventBody::FriendshipDecline);
        let valid = encode_group_event(&event).unwrap();

        let mut trailing = valid.clone();
        trailing.push(0);
        assert!(decode(&trailing).is_err());

        let mut noncanonical = valid.clone();
        noncanonical.splice(13..14, [0x18, 0x01]);
        assert!(decode(&noncanonical).is_err());

        assert!(matches!(
            decode(&vec![0; MAX_EVENT_BYTES + 1]),
            Err(EventError::Oversized(_))
        ));

        let unknown_kind = Value::Array(vec![
            Value::TextString(TAG.to_owned().into()),
            Value::Unsigned(VERSION),
            hash_value(&event.id),
            key_value(&event.author),
            Value::Array(vec![]),
            hash_value(event.group.as_ref().unwrap()),
            Value::Array(vec![
                Value::TextString("future.kind".to_owned().into()),
                Value::Array(vec![]),
            ]),
        ])
        .encode();
        assert!(matches!(decode(&unknown_kind), Err(EventError::Kind(_))));

        let wrong_tag = Value::Array(vec![
            Value::TextString("not.group.event".to_owned().into()),
            Value::Unsigned(VERSION),
            hash_value(&event.id),
            key_value(&event.author),
            Value::Array(vec![]),
            hash_value(event.group.as_ref().unwrap()),
            Value::Array(vec![
                Value::TextString("friendship.decline".to_owned().into()),
                Value::Array(vec![]),
            ]),
        ])
        .encode();
        assert!(matches!(decode(&wrong_tag), Err(EventError::Tag(_))));

        let wrong_version = Value::Array(vec![
            Value::TextString(TAG.to_owned().into()),
            Value::Unsigned(VERSION + 1),
            hash_value(&event.id),
            key_value(&event.author),
            Value::Array(vec![]),
            hash_value(event.group.as_ref().unwrap()),
            Value::Array(vec![
                Value::TextString("friendship.decline".to_owned().into()),
                Value::Array(vec![]),
            ]),
        ])
        .encode();
        assert!(matches!(
            decode(&wrong_version),
            Err(EventError::Version(_))
        ));

        let malformed_join = Value::Array(vec![
            Value::TextString(TAG.to_owned().into()),
            Value::Unsigned(VERSION),
            hash_value(&event.id),
            key_value(&event.author),
            Value::Array(vec![]),
            hash_value(event.group.as_ref().unwrap()),
            Value::Array(vec![
                Value::TextString("join".to_owned().into()),
                Value::Array(vec![Value::Unsigned(0)]),
            ]),
        ])
        .encode();
        assert!(matches!(decode(&malformed_join), Err(EventError::Shape(_))));
    }

    #[test]
    fn group_event_wire_rejects_duplicate_self_and_member_repeats() {
        // Duplicate predecessors: refused on encode and on decode.
        let mut dup_after = sample(EventBody::Group(Body::Join));
        dup_after.after = vec![Hash::from_bytes([4; 32]), Hash::from_bytes([4; 32])];
        assert!(matches!(
            encode_group_event(&dup_after),
            Err(EventError::Shape(_))
        ));
        let dup_after_wire = Value::Array(vec![
            Value::TextString(TAG.to_owned().into()),
            Value::Unsigned(VERSION),
            hash_value(&dup_after.id),
            key_value(&dup_after.author),
            Value::Array(vec![
                hash_value(&Hash::from_bytes([4; 32])),
                hash_value(&Hash::from_bytes([4; 32])),
            ]),
            hash_value(dup_after.group.as_ref().unwrap()),
            Value::Array(vec![
                Value::TextString("join".to_owned().into()),
                Value::Array(vec![]),
            ]),
        ])
        .encode();
        assert!(matches!(decode(&dup_after_wire), Err(EventError::Shape(_))));

        // Self-predecessor: refused on encode and on decode.
        let mut self_after = sample(EventBody::Group(Body::Join));
        self_after.after = vec![self_after.id];
        assert!(matches!(
            encode_group_event(&self_after),
            Err(EventError::Shape(_))
        ));
        let self_after_wire = Value::Array(vec![
            Value::TextString(TAG.to_owned().into()),
            Value::Unsigned(VERSION),
            hash_value(&self_after.id),
            key_value(&self_after.author),
            Value::Array(vec![hash_value(&self_after.id)]),
            hash_value(self_after.group.as_ref().unwrap()),
            Value::Array(vec![
                Value::TextString("join".to_owned().into()),
                Value::Array(vec![]),
            ]),
        ])
        .encode();
        assert!(matches!(
            decode(&self_after_wire),
            Err(EventError::Shape(_))
        ));

        // Duplicate create member (same key, conflicting roles): refused on
        // encode and on decode (the decode path rebuilds the create body
        // through the same shared gate).
        let dup_member = sample(EventBody::Group(Body::Create(Create {
            template: Template::Custom,
            policy: Policy::default(),
            members: vec![([6; 32], Role::Read), ([6; 32], Role::Admin)],
            admins_on_create: false,
        })));
        assert!(matches!(
            encode_group_event(&dup_member),
            Err(EventError::Shape(_))
        ));
        // Hand-built create wire bytes carrying the repeated member: build
        // them by encoding a valid single-member create and splicing a
        // second entry for the same key into the members array is brittle,
        // so decode is exercised through a members list constructed with
        // the codec's own value helpers via a raw Value envelope.
        let (kind, args) = {
            let single = sample(EventBody::Group(Body::Create(Create {
                template: Template::Custom,
                policy: Policy::default(),
                members: vec![([6; 32], Role::Read)],
                admins_on_create: false,
            })));
            let bytes = encode_group_event(&single).unwrap();
            let value = DecodeOptions::new()
                .strictness(Strictness::STRICT)
                .decode_owned(&bytes)
                .unwrap();
            let Value::Array(envelope) = value else {
                panic!("envelope is an array");
            };
            let Value::Array(body) = &envelope[6] else {
                panic!("body is an array");
            };
            let kind = body[0].clone();
            let Value::Array(create_args) = &body[1] else {
                panic!("create args are an array");
            };
            let mut create_args = create_args.clone();
            let Value::Array(members) = &create_args[2] else {
                panic!("members are an array");
            };
            let mut members = members.clone();
            members.push(Value::Array(vec![
                key_value(&[6; 32]),
                role_value(Role::Admin),
            ]));
            create_args[2] = Value::Array(members);
            (kind, Value::Array(create_args))
        };
        let dup_member_wire = Value::Array(vec![
            Value::TextString(TAG.to_owned().into()),
            Value::Unsigned(VERSION),
            hash_value(&dup_member.id),
            key_value(&dup_member.author),
            Value::Array(vec![]),
            hash_value(dup_member.group.as_ref().unwrap()),
            Value::Array(vec![kind, args]),
        ])
        .encode();
        assert!(matches!(
            decode(&dup_member_wire),
            Err(EventError::Shape(_))
        ));

        // Distinct predecessors and members still pass.
        assert!(encode_group_event(&sample(EventBody::Group(Body::Join))).is_ok());
    }

    #[test]
    fn group_event_wire_enforces_count_and_model_bounds() {
        let mut too_many_after = sample(EventBody::FriendshipDecline);
        too_many_after.after = vec![Hash::from_bytes([9; 32]); MAX_AFTER + 1];
        assert!(matches!(
            encode_group_event(&too_many_after),
            Err(EventError::Shape(_))
        ));

        let too_many_members = sample(EventBody::Group(Body::Create(Create {
            template: Template::Custom,
            policy: Policy::default(),
            members: vec![([6; 32], Role::Read); MAX_CREATE_MEMBERS + 1],
            admins_on_create: false,
        })));
        assert!(matches!(
            encode_group_event(&too_many_members),
            Err(EventError::Shape(_))
        ));

        let unresolved_remove = sample(EventBody::Group(Body::Member(MemberOp::Remove {
            member: [6; 32],
            mode: RemoveMode::ServerDefault,
        })));
        assert!(matches!(
            encode_group_event(&unresolved_remove),
            Err(EventError::Shape(_))
        ));

        let no_group_decline = GroupEvent {
            group: None,
            ..sample(EventBody::FriendshipDecline)
        };
        assert!(matches!(
            encode_group_event(&no_group_decline),
            Err(EventError::Shape(_))
        ));
    }
}
