//! Social object envelope: profile tags, profile rules, validation
//! and the strict canonical encoding.
//!
//! Issue #13095 (remainder #14334); implementation design
//! `doc/2026-11-24T05-55-00Z_design_id13095_social-object-envelope`
//! §4–§5 slice S1; parent design #9128 §§2–5.
//!
//! [`SocialObject`] is the one envelope every social shape shares.
//! This module adds what the graph layer leaves out:
//!
//! - the five profile tags of #9128 §2 (`feed-entry`, `profile`,
//!   `topic`, `comment`, `reaction`) as stable wire tokens;
//! - a table of profile rules saying, per profile, whether
//!   `parent_ref` and `target_ref` are required or forbidden;
//! - a pure validator ([`SocialObject::validate`]) with typed errors
//!   that name the field or rule and never echo untrusted content;
//! - a strict canonical CBOR encoding ([`SocialObject::to_bytes`] /
//!   [`SocialObject::from_bytes`]) that fails closed on unknown
//!   fields, unknown profiles and unsupported schema versions.
//!
//! The validator checks shape only. It does not check that referenced
//! objects exist, that the author signed the envelope, that the
//! signer may post there, or what a `succeeds_ref` means: those
//! belong to storage, admission and the #9129 lineage rules.
//! `created_claim` is the signer's claim and is never checked or used
//! for ordering here.

use super::social::{SocialObject, SocialObjectKind};
use super::term::{ObjectRef, Term};
use crate::canonical_cbor::{Value, decode_canonical, encode_canonical};

/// The envelope schema version this reader writes and accepts.
/// Unknown versions fail closed (design §4).
pub const SOCIAL_SCHEMA_VERSION: u64 = 1;

/// Most bytes an envelope's author claim may take.
pub const MAX_AUTHOR_BYTES: usize = 256;

/// Most bytes one encoded envelope may take. An envelope holds a few
/// fixed-size references, so anything larger is refused unread.
pub const MAX_ENVELOPE_BYTES: usize = 4096;

const FIELD_OBJECT_ID: &str = "object_id";
const FIELD_PROFILE: &str = "profile";
const FIELD_AUTHOR: &str = "author";
const FIELD_CREATED_CLAIM: &str = "created_claim";
const FIELD_PARENT_REF: &str = "parent_ref";
const FIELD_CONTENT_REF: &str = "content_ref";
const FIELD_SCHEMA_VERSION: &str = "schema_version";
const FIELD_SUCCEEDS_REF: &str = "succeeds_ref";
const FIELD_TARGET_REF: &str = "target_ref";

const KNOWN_FIELDS: [&str; 9] = [
    FIELD_OBJECT_ID,
    FIELD_PROFILE,
    FIELD_AUTHOR,
    FIELD_CREATED_CLAIM,
    FIELD_PARENT_REF,
    FIELD_CONTENT_REF,
    FIELD_SCHEMA_VERSION,
    FIELD_SUCCEEDS_REF,
    FIELD_TARGET_REF,
];

/// Every profile, in tag order.
pub const ALL_PROFILES: [SocialObjectKind; 5] = [
    SocialObjectKind::FeedEntry,
    SocialObjectKind::Profile,
    SocialObjectKind::Topic,
    SocialObjectKind::Comment,
    SocialObjectKind::Reaction,
];

impl SocialObjectKind {
    /// The profile's wire tag (#9128 §2).
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::FeedEntry => "feed-entry",
            Self::Profile => "profile",
            Self::Topic => "topic",
            Self::Comment => "comment",
            Self::Reaction => "reaction",
        }
    }

    /// The profile named by `tag`, or `None` for any other text. Tags
    /// are exact and case-sensitive.
    #[must_use]
    pub fn from_tag(tag: &str) -> Option<Self> {
        ALL_PROFILES.into_iter().find(|kind| kind.tag() == tag)
    }

    /// The relation rules this profile's envelopes must meet.
    #[must_use]
    pub const fn rule(self) -> ProfileRule {
        use Requirement::{Forbidden, Required};
        match self {
            // A feed is a projection, not a stored list (#9128 §3):
            // an entry nests under nothing and names the object it
            // surfaces through `target_ref`.
            Self::FeedEntry => ProfileRule {
                parent: Forbidden,
                target: Required,
            },
            // A profile is a root object owned by one identity
            // (#9128 §4); other objects nest under it, never the
            // reverse.
            Self::Profile => ProfileRule {
                parent: Forbidden,
                target: Forbidden,
            },
            // A topic is admitted to a board's `topics` list and a
            // comment to a topic's `comments` list (#9128 §5); a
            // reaction reacts to its parent. Each names its parent.
            Self::Topic | Self::Comment | Self::Reaction => ProfileRule {
                parent: Required,
                target: Forbidden,
            },
        }
    }
}

/// Whether a profile must carry a relation or must leave it out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requirement {
    /// The relation must be present.
    Required,
    /// The relation must be absent.
    Forbidden,
}

/// The relation rules of one profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileRule {
    /// Whether `parent_ref` is required or forbidden.
    pub parent: Requirement,
    /// Whether `target_ref` is required or forbidden.
    pub target: Requirement,
}

/// Why an envelope was refused. No variant carries text from the
/// envelope itself, so an error is safe to log or show.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnvelopeError {
    /// The envelope's schema version is not one this reader knows.
    #[error("unsupported social schema version {0}")]
    UnsupportedSchema(u64),
    /// The profile tag is not one of the five known tags.
    #[error("unknown social profile tag")]
    UnknownProfile,
    /// A field is missing, malformed, of the wrong type or too large.
    #[error("social envelope field `{field}` {problem}")]
    Field {
        /// The field at fault.
        field: &'static str,
        /// What was wrong with it.
        problem: &'static str,
    },
    /// The envelope breaks one of its profile's relation rules.
    #[error("a {profile} envelope {rule}")]
    Relation {
        /// The profile's tag.
        profile: &'static str,
        /// The rule it broke.
        rule: &'static str,
    },
}

impl EnvelopeError {
    /// A stable failure class: `unsupported`, `malformed` or
    /// `relation`.
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::UnsupportedSchema(_) | Self::UnknownProfile => "unsupported",
            Self::Field { .. } => "malformed",
            Self::Relation { .. } => "relation",
        }
    }
}

const fn field(field: &'static str, problem: &'static str) -> EnvelopeError {
    EnvelopeError::Field { field, problem }
}

impl SocialObject {
    /// Check the envelope's shape: a supported schema version, a
    /// well-formed author claim, the profile's relation rules, and no
    /// relation that points at the object itself.
    ///
    /// # Errors
    ///
    /// The first rule the envelope breaks, as an [`EnvelopeError`].
    pub fn validate(&self) -> Result<(), EnvelopeError> {
        if self.schema_version != SOCIAL_SCHEMA_VERSION {
            return Err(EnvelopeError::UnsupportedSchema(self.schema_version));
        }
        if self.author.is_empty() {
            return Err(field(FIELD_AUTHOR, "is empty"));
        }
        if self.author.len() > MAX_AUTHOR_BYTES {
            return Err(field(FIELD_AUTHOR, "is too long"));
        }
        if self.author.chars().any(char::is_control) {
            return Err(field(FIELD_AUTHOR, "contains a control character"));
        }
        let profile = self.kind.tag();
        let rule = self.kind.rule();
        let relation = |rule: &'static str| EnvelopeError::Relation { profile, rule };
        match (rule.parent, self.parent_ref.is_some()) {
            (Requirement::Required, false) => return Err(relation("requires a parent_ref")),
            (Requirement::Forbidden, true) => return Err(relation("must not carry a parent_ref")),
            _ => {}
        }
        match (rule.target, self.target_ref.is_some()) {
            (Requirement::Required, false) => return Err(relation("requires a target_ref")),
            (Requirement::Forbidden, true) => return Err(relation("must not carry a target_ref")),
            _ => {}
        }
        let own = self.object_id;
        if self.parent_ref == Some(own) {
            return Err(relation("cannot be its own parent"));
        }
        if self.target_ref == Some(own) {
            return Err(relation("cannot target itself"));
        }
        if self.succeeds_ref == Some(own) {
            return Err(relation("cannot succeed itself"));
        }
        Ok(())
    }

    /// The envelope as a CBOR value, absent optional relations left
    /// out. The value is validated first, so an invalid envelope has
    /// no encoding.
    ///
    /// # Errors
    ///
    /// The envelope fails [`Self::validate`], or a reference cannot be
    /// encoded.
    pub fn to_value(&self) -> Result<Value, EnvelopeError> {
        self.validate()?;
        let mut entries = vec![
            (FIELD_OBJECT_ID, ref_value(FIELD_OBJECT_ID, self.object_id)?),
            (FIELD_PROFILE, Value::Text(self.kind.tag().to_owned())),
            (FIELD_AUTHOR, Value::Text(self.author.clone())),
            (FIELD_CREATED_CLAIM, Value::UInt(self.created_claim)),
            (
                FIELD_CONTENT_REF,
                ref_value(FIELD_CONTENT_REF, self.content_ref)?,
            ),
            (FIELD_SCHEMA_VERSION, Value::UInt(self.schema_version)),
        ];
        for (name, reference) in [
            (FIELD_PARENT_REF, self.parent_ref),
            (FIELD_SUCCEEDS_REF, self.succeeds_ref),
            (FIELD_TARGET_REF, self.target_ref),
        ] {
            if let Some(reference) = reference {
                entries.push((name, ref_value(name, reference)?));
            }
        }
        Ok(Value::Map(
            entries
                .into_iter()
                .map(|(name, value)| (Value::Text(name.to_owned()), value))
                .collect(),
        ))
    }

    /// The envelope's canonical CBOR bytes. Equal envelopes always
    /// encode to equal bytes.
    ///
    /// # Errors
    ///
    /// As [`Self::to_value`], or the value cannot be encoded.
    pub fn to_bytes(&self) -> Result<Vec<u8>, EnvelopeError> {
        let bytes = encode_canonical(&self.to_value()?)
            .map_err(|_err| field("envelope", "cannot be encoded"))?;
        if bytes.len() > MAX_ENVELOPE_BYTES {
            return Err(field("envelope", "is too large"));
        }
        Ok(bytes)
    }

    /// Read an envelope from its canonical CBOR bytes and validate
    /// it. Unknown fields, unknown profiles, unsupported schema
    /// versions, non-canonical encodings and envelopes that break a
    /// profile rule are all refused.
    ///
    /// # Errors
    ///
    /// The first problem found, as an [`EnvelopeError`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, EnvelopeError> {
        if bytes.len() > MAX_ENVELOPE_BYTES {
            return Err(field("envelope", "is too large"));
        }
        let value =
            decode_canonical(bytes).map_err(|_err| field("envelope", "is not canonical CBOR"))?;
        Self::from_value(&value)
    }

    /// Read an envelope from a decoded CBOR value and validate it.
    ///
    /// # Errors
    ///
    /// As [`Self::from_bytes`].
    pub fn from_value(value: &Value) -> Result<Self, EnvelopeError> {
        let Value::Map(entries) = value else {
            return Err(field("envelope", "is not a map"));
        };
        for (key, _) in entries {
            match key {
                Value::Text(name) if KNOWN_FIELDS.contains(&name.as_str()) => {}
                _ => return Err(field("envelope", "carries an unknown field")),
            }
        }
        let get = |name: &str| {
            entries
                .iter()
                .find(|(key, _)| matches!(key, Value::Text(k) if k == name))
                .map(|(_, value)| value)
        };
        let required = |name: &'static str| {
            get(name).ok_or(EnvelopeError::Field {
                field: name,
                problem: "is missing",
            })
        };
        let uint = |name: &'static str| match required(name)? {
            Value::UInt(number) => Ok(*number),
            _ => Err(field(name, "is not an unsigned integer")),
        };
        let text = |name: &'static str| match required(name)? {
            Value::Text(text) => Ok(text.clone()),
            _ => Err(field(name, "is not text")),
        };
        let optional_ref = |name: &'static str| get(name).map(|v| ref_from(name, v)).transpose();

        // Fail closed on the version before reading anything whose
        // meaning the version could change.
        let schema_version = uint(FIELD_SCHEMA_VERSION)?;
        if schema_version != SOCIAL_SCHEMA_VERSION {
            return Err(EnvelopeError::UnsupportedSchema(schema_version));
        }
        let kind = SocialObjectKind::from_tag(&text(FIELD_PROFILE)?)
            .ok_or(EnvelopeError::UnknownProfile)?;
        let object = Self {
            object_id: ref_from(FIELD_OBJECT_ID, required(FIELD_OBJECT_ID)?)?,
            kind,
            author: text(FIELD_AUTHOR)?,
            created_claim: uint(FIELD_CREATED_CLAIM)?,
            parent_ref: optional_ref(FIELD_PARENT_REF)?,
            content_ref: ref_from(FIELD_CONTENT_REF, required(FIELD_CONTENT_REF)?)?,
            schema_version,
            succeeds_ref: optional_ref(FIELD_SUCCEEDS_REF)?,
            target_ref: optional_ref(FIELD_TARGET_REF)?,
        };
        object.validate()?;
        Ok(object)
    }
}

fn ref_value(name: &'static str, reference: ObjectRef) -> Result<Value, EnvelopeError> {
    Term::Object(reference)
        .to_value()
        .map_err(|_err| field(name, "cannot be encoded"))
}

fn ref_from(name: &'static str, value: &Value) -> Result<ObjectRef, EnvelopeError> {
    let term = Term::from_value(value).map_err(|_err| field(name, "is not an object reference"))?;
    term.object_ref().copied().ok_or(EnvelopeError::Field {
        field: name,
        problem: "is not an object reference",
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::id_shape::{ContentId, HashProfile};
    use crate::signed_envelope::EnvelopeId;

    fn envelope_ref(n: u8) -> ObjectRef {
        ObjectRef::Envelope(EnvelopeId::from_content_id(ContentId::new(
            HashProfile::RawBlake3_256,
            [n; 32],
        )))
    }

    fn content_ref(n: u8) -> ObjectRef {
        ObjectRef::Content(ContentId::new(HashProfile::RawBlake3_256, [n; 32]))
    }

    /// A minimal valid envelope of each profile.
    fn valid(kind: SocialObjectKind) -> SocialObject {
        let rule = kind.rule();
        SocialObject {
            object_id: envelope_ref(1),
            kind,
            author: "alice".to_owned(),
            created_claim: 1_800_000_000,
            parent_ref: (rule.parent == Requirement::Required).then(|| envelope_ref(2)),
            content_ref: content_ref(3),
            schema_version: SOCIAL_SCHEMA_VERSION,
            succeeds_ref: None,
            target_ref: (rule.target == Requirement::Required).then(|| envelope_ref(4)),
        }
    }

    fn relation(kind: SocialObjectKind, rule: &'static str) -> EnvelopeError {
        EnvelopeError::Relation {
            profile: kind.tag(),
            rule,
        }
    }

    fn reencode(object: &SocialObject, edit: impl FnOnce(&mut Vec<(Value, Value)>)) -> Vec<u8> {
        let Value::Map(mut entries) = object.to_value().unwrap() else {
            panic!("an envelope encodes as a map");
        };
        edit(&mut entries);
        encode_canonical(&Value::Map(entries)).unwrap()
    }

    fn set(entries: &mut Vec<(Value, Value)>, name: &str, value: Value) {
        entries.retain(|(key, _)| !matches!(key, Value::Text(k) if k == name));
        entries.push((Value::Text(name.to_owned()), value));
    }

    #[test]
    fn test_tags_round_trip_and_are_exact() {
        assert_eq!(
            ALL_PROFILES.map(SocialObjectKind::tag),
            ["feed-entry", "profile", "topic", "comment", "reaction"]
        );
        for kind in ALL_PROFILES {
            assert_eq!(SocialObjectKind::from_tag(kind.tag()), Some(kind));
        }
        for other in ["Topic", "feed_entry", "board", "", "topic "] {
            assert_eq!(SocialObjectKind::from_tag(other), None, "{other:?}");
        }
    }

    #[test]
    fn test_each_profile_round_trips_canonically() {
        for kind in ALL_PROFILES {
            let mut object = valid(kind);
            object.validate().unwrap();
            let bytes = object.to_bytes().unwrap();
            assert_eq!(
                object.to_bytes().unwrap(),
                bytes,
                "{kind:?} is deterministic"
            );
            assert_eq!(
                SocialObject::from_bytes(&bytes).unwrap(),
                object,
                "{kind:?}"
            );

            // A version with a succeeds_ref round-trips too.
            object.succeeds_ref = Some(envelope_ref(9));
            let bytes = object.to_bytes().unwrap();
            assert_eq!(
                SocialObject::from_bytes(&bytes).unwrap(),
                object,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn test_profile_relation_rules() {
        use SocialObjectKind::{Comment, FeedEntry, Profile, Reaction, Topic};
        for kind in [Topic, Comment, Reaction] {
            let mut object = valid(kind);
            object.parent_ref = None;
            assert_eq!(
                object.validate(),
                Err(relation(kind, "requires a parent_ref"))
            );
            let mut object = valid(kind);
            object.target_ref = Some(envelope_ref(4));
            assert_eq!(
                object.validate(),
                Err(relation(kind, "must not carry a target_ref"))
            );
        }
        for kind in [Profile, FeedEntry] {
            let mut object = valid(kind);
            object.parent_ref = Some(envelope_ref(2));
            assert_eq!(
                object.validate(),
                Err(relation(kind, "must not carry a parent_ref"))
            );
        }
        let mut entry = valid(FeedEntry);
        entry.target_ref = None;
        assert_eq!(
            entry.validate(),
            Err(relation(FeedEntry, "requires a target_ref"))
        );
        let mut profile = valid(Profile);
        profile.target_ref = Some(envelope_ref(4));
        assert_eq!(
            profile.validate(),
            Err(relation(Profile, "must not carry a target_ref"))
        );
        // An invalid envelope has no encoding.
        assert!(entry.to_bytes().is_err());
    }

    #[test]
    fn test_self_references_are_refused() {
        let mut comment = valid(SocialObjectKind::Comment);
        comment.parent_ref = Some(comment.object_id);
        assert_eq!(
            comment.validate(),
            Err(relation(
                SocialObjectKind::Comment,
                "cannot be its own parent"
            ))
        );
        let mut entry = valid(SocialObjectKind::FeedEntry);
        entry.target_ref = Some(entry.object_id);
        assert_eq!(
            entry.validate(),
            Err(relation(
                SocialObjectKind::FeedEntry,
                "cannot target itself"
            ))
        );
        let mut profile = valid(SocialObjectKind::Profile);
        profile.succeeds_ref = Some(profile.object_id);
        assert_eq!(
            profile.validate(),
            Err(relation(SocialObjectKind::Profile, "cannot succeed itself"))
        );
    }

    #[test]
    fn test_author_claim_is_checked_and_never_echoed() {
        let mut object = valid(SocialObjectKind::Topic);
        object.author = String::new();
        assert_eq!(object.validate(), Err(field(FIELD_AUTHOR, "is empty")));
        object.author = "a".repeat(MAX_AUTHOR_BYTES);
        object.validate().unwrap();
        object.author.push('a');
        assert_eq!(object.validate(), Err(field(FIELD_AUTHOR, "is too long")));
        object.author = "secret-handle\u{7}".to_owned();
        let error = object.validate().unwrap_err();
        assert_eq!(error, field(FIELD_AUTHOR, "contains a control character"));
        assert!(!error.to_string().contains("secret-handle"));
        assert_eq!(error.class(), "malformed");
    }

    #[test]
    fn test_unsupported_schema_fails_closed() {
        for version in [0, SOCIAL_SCHEMA_VERSION + 1, u64::MAX] {
            let mut object = valid(SocialObjectKind::Reaction);
            object.schema_version = version;
            assert_eq!(
                object.validate(),
                Err(EnvelopeError::UnsupportedSchema(version))
            );
            let bytes = reencode(&valid(SocialObjectKind::Reaction), |entries| {
                set(entries, FIELD_SCHEMA_VERSION, Value::UInt(version));
            });
            let error = SocialObject::from_bytes(&bytes).unwrap_err();
            assert_eq!(error, EnvelopeError::UnsupportedSchema(version));
            assert_eq!(error.class(), "unsupported");
        }
    }

    #[test]
    fn test_decode_rejects_malformed_envelopes() {
        let base = valid(SocialObjectKind::Comment);
        let cases: Vec<(&str, Vec<u8>, EnvelopeError)> = vec![
            (
                "unknown profile",
                reencode(&base, |e| {
                    set(e, FIELD_PROFILE, Value::Text("board".into()));
                }),
                EnvelopeError::UnknownProfile,
            ),
            (
                "unknown field",
                reencode(&base, |e| set(e, "score", Value::UInt(1))),
                field("envelope", "carries an unknown field"),
            ),
            (
                "integer key",
                reencode(&base, |e| e.push((Value::UInt(1), Value::UInt(1)))),
                field("envelope", "carries an unknown field"),
            ),
            (
                "missing author",
                reencode(&base, |e| {
                    e.retain(|(k, _)| !matches!(k, Value::Text(t) if t == FIELD_AUTHOR));
                }),
                field(FIELD_AUTHOR, "is missing"),
            ),
            (
                "malformed object id",
                reencode(&base, |e| {
                    set(e, FIELD_OBJECT_ID, Value::Bytes(vec![1; 32]));
                }),
                field(FIELD_OBJECT_ID, "is not an object reference"),
            ),
            (
                "null parent",
                reencode(&base, |e| set(e, FIELD_PARENT_REF, Value::Null)),
                field(FIELD_PARENT_REF, "is not an object reference"),
            ),
            (
                "text claim",
                reencode(&base, |e| {
                    set(e, FIELD_CREATED_CLAIM, Value::Text("now".into()));
                }),
                field(FIELD_CREATED_CLAIM, "is not an unsigned integer"),
            ),
            (
                "comment with a target",
                reencode(&base, |e| {
                    set(
                        e,
                        FIELD_TARGET_REF,
                        ref_value(FIELD_TARGET_REF, envelope_ref(4)).unwrap(),
                    );
                }),
                relation(SocialObjectKind::Comment, "must not carry a target_ref"),
            ),
            (
                "not a map",
                encode_canonical(&Value::Array(vec![])).unwrap(),
                field("envelope", "is not a map"),
            ),
        ];
        for (name, bytes, expected) in cases {
            assert_eq!(SocialObject::from_bytes(&bytes), Err(expected), "{name}");
        }

        let mut trailing = base.to_bytes().unwrap();
        trailing.push(0);
        assert_eq!(
            SocialObject::from_bytes(&trailing),
            Err(field("envelope", "is not canonical CBOR"))
        );
        assert_eq!(
            SocialObject::from_bytes(&[0; MAX_ENVELOPE_BYTES + 1]),
            Err(field("envelope", "is too large"))
        );
    }
}
