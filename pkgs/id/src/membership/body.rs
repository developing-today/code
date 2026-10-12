//! The body of a group operation: how a membership change travels in an event's body (P2.3).
//!
//! An event's header commits to its body through `payload_hash` and `payload_size`. A body is a
//! group operation when it starts with the bytes of [`CLAIM`]. Such a body is the canonical CBOR
//! array
//!
//! ```text
//! ["group.op", version, kind, args]
//! ```
//!
//! where `version` is [`VERSION`] and `kind` and `args` are:
//!
//! | `kind`    | `args`                            | [`GroupAction`]   |
//! |-----------|-----------------------------------|-------------------|
//! | `create`  | array of `[member, access]` pairs | `Create`          |
//! | `join`    | `[member, access]`                | `Add`             |
//! | `remove`  | `[member]`                        | `Remove`          |
//! | `promote` | `[member, access]`                | `Promote`         |
//! | `demote`  | `[member, access]`                | `Demote`          |
//!
//! A member is `[kind, id]`: kind `0` is an individual and `1` a group. An `id` is a 33-byte
//! string: a tag byte (`0` for a key, `1` for a group hash), then the 32 bytes. An access is its
//! level as an unsigned integer: `0` pull, `1` read, `2` write, `3` manage. Access conditions are
//! not encoded, so [`encode_body`] refuses an access that has them.
//!
//! [`decode_body`] has two outcomes for a body that starts with [`CLAIM`]: it decodes, or it is
//! refused. A body that does not start with it is not a group operation, and it is content.
//! Friendship and group policy have no [`GroupAction`] form, so they are not encoded here.

use cbor_core::{DecodeOptions, Strictness, Value};
use p2panda_auth::group::{GroupAction, GroupMember};
use p2panda_auth::{Access, AccessLevel};
use p2panda_core::Hash;

use super::AuthId;

/// The text that opens a group operation body. It names the format, as an event's kind does.
pub const TAG: &str = "group.op";

/// The version of the body format this code writes and reads.
pub const VERSION: u64 = 1;

/// The most bytes a group operation body may have.
pub const MAX_BODY_BYTES: usize = 16 * 1024;

/// The most members a `create` may list.
pub const MAX_CREATE_MEMBERS: usize = 256;

/// The first bytes of every group operation body: an array of four items, then [`TAG`] as a text
/// string. A body that starts with these bytes claims to be a group operation.
pub const CLAIM: &[u8] = b"\x84\x68group.op";

/// Nesting limit for a body's CBOR. Matches the limit the header decoder uses.
const RECURSION_LIMIT: u16 = 8;

/// Declared element limit for one array. Matches the header decoder.
const LENGTH_LIMIT: u64 = 512;

/// Byte budget for speculative pre-allocation. Matches the header decoder.
const OOM_MITIGATION: usize = 64;

/// Why a body that claims to be a group operation was refused, or why one could not be encoded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BodyError {
    /// The body is longer than [`MAX_BODY_BYTES`].
    #[error("a group operation body is {0} bytes, the most allowed is {max}", max = MAX_BODY_BYTES)]
    Oversized(usize),
    /// The bytes are CBOR that strict decoding refuses. This includes a truncated body.
    #[error("the group operation body is not CBOR: {0}")]
    Cbor(String),
    /// The bytes decode, but they are not the canonical encoding of the value they hold.
    #[error("the group operation body is not canonical CBOR")]
    NotCanonical,
    /// The version is not [`VERSION`].
    #[error("the group operation body has version {0}, this code reads version {v}", v = VERSION)]
    Version(u64),
    /// The kind is not one of the kinds in the module table.
    #[error("the group operation kind {0:?} is unknown")]
    Kind(String),
    /// The body has the wrong shape for its kind: a missing item, an extra one, or a value of the
    /// wrong type or range.
    #[error("the group operation body has the wrong shape: {0}")]
    Shape(String),
    /// The action cannot be encoded: it has access conditions, or too many create members.
    #[error("the group action cannot be encoded: {0}")]
    Unencodable(String),
}

/// Encodes a group action as a group operation body.
///
/// # Errors
///
/// Returns [`BodyError::Unencodable`] for an access with conditions or a create with more than
/// [`MAX_CREATE_MEMBERS`] members, and [`BodyError::Oversized`] when the encoding is longer than
/// [`MAX_BODY_BYTES`].
pub fn encode_body(action: &GroupAction<AuthId, ()>) -> Result<Vec<u8>, BodyError> {
    let (kind, args) = match action {
        GroupAction::Create { initial_members } => {
            if initial_members.len() > MAX_CREATE_MEMBERS {
                return Err(BodyError::Unencodable(format!(
                    "{} members, the most is {MAX_CREATE_MEMBERS}",
                    initial_members.len()
                )));
            }
            let pairs = initial_members
                .iter()
                .map(|(member, access)| member_and_access(member, access))
                .collect::<Result<Vec<_>, _>>()?;
            ("create", Value::Array(pairs))
        }
        GroupAction::Add { member, access } => ("join", member_and_access(member, access)?),
        GroupAction::Remove { member } => ("remove", Value::Array(vec![member_value(member)])),
        GroupAction::Promote { member, access } => ("promote", member_and_access(member, access)?),
        GroupAction::Demote { member, access } => ("demote", member_and_access(member, access)?),
    };
    let body = Value::Array(vec![
        Value::TextString(TAG.into()),
        Value::Unsigned(VERSION),
        Value::TextString(kind.into()),
        args,
    ])
    .encode();
    if body.len() > MAX_BODY_BYTES {
        return Err(BodyError::Oversized(body.len()));
    }
    Ok(body)
}

/// Decodes a body as a group action.
///
/// Returns `Ok(None)` when the body does not start with [`CLAIM`]: it is content, and not a group
/// operation. Returns `Ok(Some(action))` for a body that decodes, and an error for one that claims
/// to be a group operation and does not decode.
///
/// # Errors
///
/// Returns the reason a claimed body was refused. See [`BodyError`].
pub fn decode_body(body: &[u8]) -> Result<Option<GroupAction<AuthId, ()>>, BodyError> {
    if !body.starts_with(CLAIM) {
        return Ok(None);
    }
    if body.len() > MAX_BODY_BYTES {
        return Err(BodyError::Oversized(body.len()));
    }
    let value = DecodeOptions::new()
        .strictness(Strictness::STRICT)
        .recursion_limit(RECURSION_LIMIT)
        .length_limit(LENGTH_LIMIT)
        .oom_mitigation(OOM_MITIGATION)
        .decode_owned(body)
        .map_err(|err| BodyError::Cbor(err.to_string()))?;
    if value.encode().as_slice() != body {
        return Err(BodyError::NotCanonical);
    }
    let [_tag, version, kind, args] = list(&value, 4)? else {
        return Err(BodyError::Shape(
            "a group operation has four items".to_owned(),
        ));
    };
    let version = unsigned(version)?;
    if version != VERSION {
        return Err(BodyError::Version(version));
    }
    let Value::TextString(kind) = kind else {
        return Err(BodyError::Shape("the kind is not text".to_owned()));
    };
    decode_kind(kind, args).map(Some)
}

/// The `[member, access]` pair of an action with one member and one access.
fn member_and_access(
    member: &GroupMember<AuthId>,
    access: &Access<()>,
) -> Result<Value<'static>, BodyError> {
    Ok(Value::Array(vec![
        member_value(member),
        access_value(access)?,
    ]))
}

/// `[kind, id]` for a member: kind `0` is an individual, `1` a group.
fn member_value(member: &GroupMember<AuthId>) -> Value<'static> {
    let (kind, id) = match member {
        GroupMember::Individual(id) => (0, id),
        GroupMember::Group(id) => (1, id),
    };
    Value::Array(vec![
        Value::Unsigned(kind),
        Value::ByteString(auth_bytes(id).into()),
    ])
}

/// The 33 bytes of an identity: a tag byte, then the key or the group hash.
fn auth_bytes(id: &AuthId) -> Vec<u8> {
    let (tag, bytes) = match id {
        AuthId::Key(key) => (0, key),
        AuthId::Group(hash) => (1, hash.as_bytes()),
    };
    let mut out = Vec::with_capacity(33);
    out.push(tag);
    out.extend_from_slice(bytes);
    out
}

/// The level of an access, as its unsigned integer. Refuses conditions.
fn access_value(access: &Access<()>) -> Result<Value<'static>, BodyError> {
    if access.conditions.is_some() {
        return Err(BodyError::Unencodable(
            "an access with conditions".to_owned(),
        ));
    }
    let level = match access.level {
        AccessLevel::Pull => 0,
        AccessLevel::Read => 1,
        AccessLevel::Write => 2,
        AccessLevel::Manage => 3,
    };
    Ok(Value::Unsigned(level))
}

/// Decodes the args of one kind. The kind table is the one in the module docs.
fn decode_kind(kind: &str, args: &Value<'static>) -> Result<GroupAction<AuthId, ()>, BodyError> {
    match kind {
        "create" => {
            let pairs = args
                .as_array()
                .map_err(|_err| BodyError::Shape("create takes a list of pairs".to_owned()))?;
            if pairs.len() > MAX_CREATE_MEMBERS {
                return Err(BodyError::Shape(format!(
                    "{} members, the most is {MAX_CREATE_MEMBERS}",
                    pairs.len()
                )));
            }
            let initial_members = pairs
                .iter()
                .map(|pair| {
                    let [member, access] = list(pair, 2)? else {
                        return Err(BodyError::Shape("a member pair has two items".to_owned()));
                    };
                    Ok((decode_member(member)?, decode_access(access)?))
                })
                .collect::<Result<Vec<_>, BodyError>>()?;
            Ok(GroupAction::Create { initial_members })
        }
        "join" | "promote" | "demote" => {
            let [member, access] = list(args, 2)? else {
                return Err(BodyError::Shape(format!(
                    "{kind} takes a member and an access"
                )));
            };
            let member = decode_member(member)?;
            let access = decode_access(access)?;
            Ok(match kind {
                "join" => GroupAction::Add { member, access },
                "promote" => GroupAction::Promote { member, access },
                _ => GroupAction::Demote { member, access },
            })
        }
        "remove" => {
            let [member] = list(args, 1)? else {
                return Err(BodyError::Shape("remove takes one member".to_owned()));
            };
            Ok(GroupAction::Remove {
                member: decode_member(member)?,
            })
        }
        other => Err(BodyError::Kind(other.to_owned())),
    }
}

/// `[kind, id]` back to a member.
fn decode_member(value: &Value<'static>) -> Result<GroupMember<AuthId>, BodyError> {
    let [kind, id] = list(value, 2)? else {
        return Err(BodyError::Shape("a member has two items".to_owned()));
    };
    let id = decode_auth(id)?;
    match unsigned(kind)? {
        0 => Ok(GroupMember::Individual(id)),
        1 => Ok(GroupMember::Group(id)),
        other => Err(BodyError::Shape(format!("member kind {other}"))),
    }
}

/// A 33-byte identity back to a key or a group.
fn decode_auth(value: &Value<'static>) -> Result<AuthId, BodyError> {
    let Value::ByteString(bytes) = value else {
        return Err(BodyError::Shape("an identity is a byte string".to_owned()));
    };
    let bytes: &[u8] = bytes;
    let Some((&tag, rest)) = bytes.split_first() else {
        return Err(BodyError::Shape("an identity is empty".to_owned()));
    };
    let Ok(id) = <[u8; 32]>::try_from(rest) else {
        return Err(BodyError::Shape(format!(
            "an identity is {} bytes, it must be 33",
            bytes.len()
        )));
    };
    match tag {
        0 => Ok(AuthId::Key(id)),
        1 => Ok(AuthId::Group(Hash::from(id))),
        other => Err(BodyError::Shape(format!("identity tag {other}"))),
    }
}

/// An access level back to an access with no conditions.
fn decode_access(value: &Value<'static>) -> Result<Access<()>, BodyError> {
    match unsigned(value)? {
        0 => Ok(Access::pull()),
        1 => Ok(Access::read()),
        2 => Ok(Access::write()),
        3 => Ok(Access::manage()),
        other => Err(BodyError::Shape(format!("access level {other}"))),
    }
}

/// The items of an array of exactly `len` items.
fn list<'a>(value: &'a Value<'static>, len: usize) -> Result<&'a [Value<'static>], BodyError> {
    match value.as_array() {
        Ok(items) if items.len() == len => Ok(items),
        _ => Err(BodyError::Shape(format!(
            "expected an array of {len} items"
        ))),
    }
}

/// An unsigned integer.
fn unsigned(value: &Value<'static>) -> Result<u64, BodyError> {
    match value {
        Value::Unsigned(number) => Ok(*number),
        _ => Err(BodyError::Shape("expected an unsigned integer".to_owned())),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use p2panda_core::SigningKey;

    fn key(who: u8) -> AuthId {
        let signing = SigningKey::from_bytes(&[who; 32]);
        AuthId::Key(*signing.verifying_key().as_bytes())
    }

    fn individual(who: u8) -> GroupMember<AuthId> {
        GroupMember::Individual(key(who))
    }

    /// One action of every kind, with every access level and both member kinds in use.
    fn every_kind() -> Vec<GroupAction<AuthId, ()>> {
        let group = AuthId::Group(Hash::digest(b"a group"));
        vec![
            GroupAction::Create {
                initial_members: vec![
                    (individual(1), Access::manage()),
                    (individual(2), Access::write()),
                    (GroupMember::Group(group), Access::read()),
                ],
            },
            GroupAction::Create {
                initial_members: Vec::new(),
            },
            GroupAction::Add {
                member: individual(3),
                access: Access::pull(),
            },
            GroupAction::Remove {
                member: individual(4),
            },
            GroupAction::Remove {
                member: GroupMember::Group(group),
            },
            GroupAction::Promote {
                member: individual(5),
                access: Access::manage(),
            },
            GroupAction::Demote {
                member: individual(6),
                access: Access::read(),
            },
        ]
    }

    /// The bytes of a valid `remove` body with one key member.
    fn remove_body() -> Vec<u8> {
        encode_body(&GroupAction::Remove {
            member: individual(9),
        })
        .unwrap()
    }

    #[test]
    fn test_every_kind_round_trips() {
        for action in every_kind() {
            let body = encode_body(&action).unwrap();
            assert!(body.starts_with(CLAIM), "the claim opens every body");
            assert_eq!(decode_body(&body).unwrap(), Some(action.clone()));
            // Canonical: encoding the decoded action gives the same bytes.
            let decoded = decode_body(&body).unwrap().unwrap();
            assert_eq!(encode_body(&decoded).unwrap(), body);
        }
    }

    #[test]
    fn test_the_claim_is_the_encoded_tag() {
        let tag = Value::TextString(TAG.into()).encode();
        assert_eq!(&CLAIM[1..], tag.as_slice());
        assert_eq!(CLAIM[0], 0x84, "an array of four items");
    }

    #[test]
    fn test_a_body_that_is_not_a_group_operation_is_content() {
        let bodies: [&[u8]; 3] = [b"", b"hello", &[0x84, 0x01, 0x02, 0x03, 0x04]];
        for body in bodies {
            assert_eq!(decode_body(body).unwrap(), None, "{body:?} is content");
        }
        // A map of the same shape as a content body, with no claim.
        let content = Value::Map(BTreeMap::from([(
            Value::TextString("kind".into()),
            Value::TextString("message".into()),
        )]))
        .encode();
        assert_eq!(decode_body(&content).unwrap(), None);
    }

    #[test]
    fn test_a_truncated_body_is_refused() {
        // Every proper prefix that still carries the claim must be refused, not decoded.
        let body = encode_body(&every_kind()[0]).unwrap();
        for cut in CLAIM.len()..body.len() {
            assert!(
                decode_body(&body[..cut]).is_err(),
                "a body cut at {cut} of {} is refused",
                body.len()
            );
        }
    }

    #[test]
    fn test_a_wrong_version_is_refused() {
        let body = Value::Array(vec![
            Value::TextString(TAG.into()),
            Value::Unsigned(VERSION + 1),
            Value::TextString("remove".into()),
            Value::Array(vec![Value::Array(vec![
                Value::Unsigned(0),
                Value::ByteString(auth_bytes(&key(9)).into()),
            ])]),
        ])
        .encode();
        assert_eq!(decode_body(&body), Err(BodyError::Version(VERSION + 1)));
    }

    #[test]
    fn test_an_unknown_kind_is_refused() {
        let body = Value::Array(vec![
            Value::TextString(TAG.into()),
            Value::Unsigned(VERSION),
            Value::TextString("friend.request".into()),
            Value::Array(Vec::new()),
        ])
        .encode();
        assert_eq!(
            decode_body(&body),
            Err(BodyError::Kind("friend.request".to_owned()))
        );
    }

    #[test]
    fn test_an_oversized_body_is_refused() {
        let body = [CLAIM, &vec![0u8; MAX_BODY_BYTES]].concat();
        assert!(matches!(decode_body(&body), Err(BodyError::Oversized(_))));
    }

    #[test]
    fn test_a_create_with_too_many_members_is_refused_both_ways() {
        let members: Vec<(GroupMember<AuthId>, Access<()>)> = (0..=MAX_CREATE_MEMBERS)
            .map(|who| (individual(u8::try_from(who % 200).unwrap()), Access::read()))
            .collect();
        let action = GroupAction::Create {
            initial_members: members,
        };
        assert!(matches!(
            encode_body(&action),
            Err(BodyError::Unencodable(_))
        ));
        // A body that lists them anyway is refused as a shape.
        let pair = Value::Array(vec![
            Value::Array(vec![
                Value::Unsigned(0),
                Value::ByteString(auth_bytes(&key(1)).into()),
            ]),
            Value::Unsigned(1),
        ]);
        let body = Value::Array(vec![
            Value::TextString(TAG.into()),
            Value::Unsigned(VERSION),
            Value::TextString("create".into()),
            Value::Array(vec![pair; MAX_CREATE_MEMBERS + 1]),
        ])
        .encode();
        assert!(matches!(decode_body(&body), Err(BodyError::Shape(_))));
    }

    #[test]
    fn test_an_access_with_conditions_is_not_encoded() {
        let action = GroupAction::Add {
            member: individual(1),
            access: Access {
                conditions: Some(()),
                level: AccessLevel::Read,
            },
        };
        assert!(matches!(
            encode_body(&action),
            Err(BodyError::Unencodable(_))
        ));
    }

    #[test]
    fn test_a_non_canonical_body_is_refused() {
        // The version 1 written as two bytes (0x18 0x01) instead of one. The value is the same,
        // so only the canonical check refuses it.
        let body = remove_body();
        let mut padded = body[..CLAIM.len()].to_vec();
        padded.extend_from_slice(&[0x18, 0x01]);
        padded.extend_from_slice(&body[CLAIM.len() + 1..]);
        assert!(decode_body(&padded).is_err());
    }

    #[test]
    fn test_a_malformed_member_is_refused() {
        let tagged = |tag: u8, len: usize| {
            let mut id = vec![tag];
            id.extend(std::iter::repeat_n(7u8, len));
            Value::Array(vec![
                Value::TextString(TAG.into()),
                Value::Unsigned(VERSION),
                Value::TextString("remove".into()),
                Value::Array(vec![Value::Array(vec![
                    Value::Unsigned(0),
                    Value::ByteString(id.into()),
                ])]),
            ])
            .encode()
        };
        // `tagged(tag, len)` is the tag byte plus `len` bytes: 33 bytes is the identity length.
        assert!(
            decode_body(&tagged(0, 31)).is_err(),
            "32 bytes is too short"
        );
        assert!(
            decode_body(&tagged(0, 32)).is_ok(),
            "33 bytes is an identity"
        );
        assert!(
            decode_body(&tagged(2, 32)).is_err(),
            "tag 2 is not an identity"
        );
        assert!(decode_body(&tagged(0, 33)).is_err(), "34 bytes is too long");
    }

    #[test]
    fn test_a_wrong_shape_is_refused() {
        let with_args = |kind: &str, args: Value<'static>| {
            Value::Array(vec![
                Value::TextString(TAG.into()),
                Value::Unsigned(VERSION),
                Value::TextString(kind.into()),
                args,
            ])
            .encode()
        };
        let member = Value::Array(vec![
            Value::Unsigned(0),
            Value::ByteString(auth_bytes(&key(1)).into()),
        ]);
        let read = Value::Unsigned(1);
        // A join needs a member and an access, so one item is missing and three is one too many.
        assert!(decode_body(&with_args("join", Value::Array(vec![member.clone()]))).is_err());
        assert!(
            decode_body(&with_args(
                "join",
                Value::Array(vec![member.clone(), read.clone(), read.clone()])
            ))
            .is_err()
        );
        // A remove with an access is one item too many.
        assert!(
            decode_body(&with_args(
                "remove",
                Value::Array(vec![member.clone(), read.clone()])
            ))
            .is_err()
        );
        // An access level outside 0 to 3, and a member kind outside 0 and 1.
        assert!(
            decode_body(&with_args(
                "join",
                Value::Array(vec![member.clone(), Value::Unsigned(4)])
            ))
            .is_err()
        );
        let bad_kind = Value::Array(vec![
            Value::Unsigned(2),
            Value::ByteString(auth_bytes(&key(1)).into()),
        ]);
        assert!(decode_body(&with_args("remove", Value::Array(vec![bad_kind]))).is_err());
        // An array of two has no claim, so it is content, whatever its items are.
        let short = Value::Array(vec![
            Value::TextString(TAG.into()),
            Value::Unsigned(VERSION),
        ])
        .encode();
        assert_eq!(decode_body(&short).unwrap(), None);
    }

    #[test]
    fn test_no_single_byte_change_panics() {
        // Every single-byte change of a valid body decodes or is refused. None panics.
        for action in every_kind() {
            let body = encode_body(&action).unwrap();
            for index in 0..body.len() {
                for flip in [0x01, 0x80, 0xFF] {
                    let mut changed = body.clone();
                    changed[index] ^= flip;
                    let _ = decode_body(&changed);
                }
            }
        }
    }
}
