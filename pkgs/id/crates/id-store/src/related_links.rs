//! Related path links in the sidecar payload (issue #11419; parent
//! #9114, program #9107; statements S3.21 "The metadata may contain
//! related path links" and S3.22 "Related links are path references,
//! not restricted to hash IDs"). The rules are settled by the design
//! record `doc/2026-11-17T22-00-00Z_design_id9114_related_path_links.md`
//! (PR #11418), sections 2–5; this module implements them on the
//! signed sidecar envelope of [`crate::sidecar_snapshot`] (#10242).
//! It does not change the CAS head (#10254).
//!
//! A link is a typed entry in the sidecar **payload** — the metadata
//! view carried by `SidecarEnvelopeV1.raw_payload_bytes` — never part
//! of the subject's content bytes, so adding or changing a link
//! produces a new snapshot and leaves the subject `ContentId`
//! unchanged (S3.11, S3.12). The links live under the
//! [`LINKS_VIEW_KEY`] entry of the view as an array of link records.
//!
//! - [`RelatedLink`] pairs an open-vocabulary `relation` with a
//!   [`LinkTarget`] of exactly four kinds (design §2, §4):
//!   [`LinkTarget::ContentRef`], [`LinkTarget::SnapshotRef`],
//!   [`LinkTarget::PathRef`] and [`LinkTarget::LocationRef`]. Any
//!   other kind arriving in a payload is rejected with
//!   [`LinkError::UnsupportedTargetKind`] (class
//!   `unsupported_target_kind`); external URLs are out of scope and
//!   would be a new target kind under the shape registry (#9113),
//!   not a path-link change.
//! - Link order in the payload is canonical: sorted by
//!   `(relation, canonical target bytes)` with exact duplicates
//!   collapsed, so the same set of links always yields the same
//!   payload bytes and therefore the same snapshot ID, regardless of
//!   the order the writer supplied them in (design §2). Use
//!   [`set_view_links`] to write and [`view_links`] to read;
//!   [`canonical_links`] exposes the canonical form itself.
//! - Relations are an open vocabulary of lowercase ASCII identifiers
//!   (`[a-z][a-z0-9_-]*`, design §4/Q5). No relation carries
//!   behaviour here, and unknown relations are preserved verbatim
//!   under the extras policy of #9095: readers keep them and never
//!   fail the read. [`is_valid_relation`] reports the vocabulary
//!   shape for callers that want it; nothing in this module rejects
//!   a link over its relation.
//! - [`PathRef`] is a path, not an ID (design §3): UTF-8 text with
//!   `/` as the only separator, stored in Unicode NFC (a non-NFC
//!   input is normalised at write, never rejected), relative to the
//!   namespace root of the sidecar's `namespace_id` — never to the
//!   sidecar's own location. A leading `/`, an empty segment, or a
//!   `.` / `..` segment is a write-time [`LinkError::InvalidPathRef`]
//!   (class `invalid_path_ref`); nothing is resolved away, because
//!   silently normalising `..` could point at a different object.
//!   The stored path is capped at [`MAX_PATH_REF_BYTES`] bytes; over
//!   the cap is [`LinkError::ResourceLimit`] (class
//!   `resource_limit`), never truncation. A `PathRef` carries no
//!   authority: resolving one is an authorisation decision made by
//!   the caller under its tenant context (#9083).
//! - Dangling links are first-class (design §5): writing a link
//!   never checks that its target exists, so [`resolve_link`] takes
//!   the caller's existence knowledge and returns
//!   [`LinkResolution::Dangling`] — a distinct result carrying the
//!   target, never an error and never a silent substitute (the
//!   summit's `MissingObject` convention). Readers preserve dangling
//!   links when they re-write a sidecar: a writer that dropped links
//!   it could not resolve would lose metadata, which S3.14 forbids.
//!   Deleting a target never cascades to links pointing at it.
//!
//! The crate-root export belongs to slice 9110-02; until that lands
//! the integration test includes this module (and
//! `sidecar_snapshot`) by path.

use std::collections::BTreeMap;
use std::fmt;

use unicode_normalization::UnicodeNormalization;

use id_core::canonical_cbor::{CborError, Value, encode_canonical};
use id_core::id_shape::{ContentId, HashProfile, IdShapeError};
use id_core::location_ref::{LocationError, LocationRef, TypedLocator};
use id_core::namespace_definition::NamespaceId;
use id_core::signed_envelope::EnvelopeId;

use crate::sidecar_snapshot::{SidecarEnvelopeV1, SidecarError, SnapshotId};

/// The metadata-view key under which a snapshot payload carries its
/// related links, as an array of link records.
pub const LINKS_VIEW_KEY: &str = "related_links";

/// Byte-length cap on the stored form of a [`PathRef`] (design §3,
/// Q4). Over the cap is `resource_limit`, never truncation.
pub const MAX_PATH_REF_BYTES: usize = 4096;

/// Why a related link, a path reference or a link-bearing payload
/// was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    /// The bytes or values are not canonical CBOR, or a link record
    /// carries an unknown field.
    Cbor(CborError),
    /// An ID inside a link record was rejected.
    Id(IdShapeError),
    /// A location target inside a link record was rejected.
    Location(LocationError),
    /// The carrying sidecar record was rejected.
    Sidecar(SidecarError),
    /// A field is missing or has the wrong type.
    Field {
        /// The field name.
        field: &'static str,
        /// What was wrong.
        problem: &'static str,
    },
    /// A path reference violates the §3 syntax: a leading `/`, an
    /// empty segment, or a `.` / `..` segment. Class
    /// `invalid_path_ref`.
    InvalidPathRef {
        /// What was wrong.
        problem: &'static str,
    },
    /// A stored path is over the [`MAX_PATH_REF_BYTES`] cap. Class
    /// `resource_limit`.
    ResourceLimit {
        /// The cap, in bytes.
        limit: usize,
        /// The stored length offered, in bytes.
        actual: usize,
    },
    /// A link target kind other than the four of design §2. Class
    /// `unsupported_target_kind`.
    UnsupportedTargetKind {
        /// The kind tag that was offered.
        kind: String,
    },
}

impl LinkError {
    /// A stable failure class. The design names `invalid_path_ref`,
    /// `resource_limit` and `unsupported_target_kind`; the rest
    /// follow the sidecar slice's classes.
    #[must_use]
    pub fn class(&self) -> &'static str {
        match self {
            Self::InvalidPathRef { .. } => "invalid_path_ref",
            Self::ResourceLimit { .. } | Self::Id(IdShapeError::ResourceLimit { .. }) => {
                "resource_limit"
            }
            Self::UnsupportedTargetKind { .. } => "unsupported_target_kind",
            Self::Sidecar(error) => error.class(),
            Self::Location(_) => "invalid_location_ref",
            Self::Cbor(_) | Self::Field { .. } | Self::Id(_) => "malformed",
        }
    }
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cbor(error) => write!(f, "link encoding: {error}"),
            Self::Id(error) => write!(f, "link identity: {error}"),
            Self::Location(error) => write!(f, "link location: {error}"),
            Self::Sidecar(error) => write!(f, "link sidecar: {error}"),
            Self::Field { field, problem } => write!(f, "field {field}: {problem}"),
            Self::InvalidPathRef { problem } => write!(f, "invalid path reference: {problem}"),
            Self::ResourceLimit { limit, actual } => {
                write!(
                    f,
                    "path reference is {actual} bytes, over the {limit}-byte cap"
                )
            }
            Self::UnsupportedTargetKind { kind } => {
                write!(f, "unsupported link target kind {kind:?}")
            }
        }
    }
}

impl std::error::Error for LinkError {}

impl From<CborError> for LinkError {
    fn from(error: CborError) -> Self {
        Self::Cbor(error)
    }
}

impl From<IdShapeError> for LinkError {
    fn from(error: IdShapeError) -> Self {
        Self::Id(error)
    }
}

impl From<LocationError> for LinkError {
    fn from(error: LocationError) -> Self {
        Self::Location(error)
    }
}

impl From<SidecarError> for LinkError {
    fn from(error: SidecarError) -> Self {
        Self::Sidecar(error)
    }
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn content_id_value(id: &ContentId) -> Value {
    // Keys in canonical order: "digest" < "profile". The same wire
    // shape the sidecar slice writes for every content ID.
    Value::Map(vec![
        (text("digest"), Value::Bytes(id.digest().to_vec())),
        (text("profile"), text(id.profile().token())),
    ])
}

/// Builds a map from `(key, value)` pairs, in canonical key order
/// (the encoder sorts too; this keeps the value itself canonical).
fn map(mut entries: Vec<(&'static str, Value)>) -> Value {
    entries.sort_by(|(a, _), (b, _)| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    Value::Map(entries.into_iter().map(|(k, v)| (text(k), v)).collect())
}

/// The entries of a map value, or a field error naming `field`.
fn map_entries<'a>(
    value: &'a Value,
    field: &'static str,
) -> Result<&'a [(Value, Value)], LinkError> {
    match value {
        Value::Map(entries) => Ok(entries),
        _ => Err(LinkError::Field {
            field,
            problem: "must be a map",
        }),
    }
}

fn get<'a>(entries: &'a [(Value, Value)], key: &str) -> Option<&'a Value> {
    entries
        .iter()
        .find(|(k, _)| matches!(k, Value::Text(name) if name == key))
        .map(|(_, v)| v)
}

fn required<'a>(
    entries: &'a [(Value, Value)],
    field: &'static str,
) -> Result<&'a Value, LinkError> {
    get(entries, field).ok_or(LinkError::Field {
        field,
        problem: "is missing",
    })
}

fn required_text(entries: &[(Value, Value)], field: &'static str) -> Result<String, LinkError> {
    match required(entries, field)? {
        Value::Text(value) => Ok(value.clone()),
        _ => Err(LinkError::Field {
            field,
            problem: "must be text",
        }),
    }
}

/// Rejects a map carrying a key outside `known`: every field of a
/// link record is critical, as in the sidecar slice.
fn check_known(entries: &[(Value, Value)], known: &[&str]) -> Result<(), LinkError> {
    for (key, _) in entries {
        let Value::Text(name) = key else {
            return Err(LinkError::Field {
                field: "link",
                problem: "keys must be text",
            });
        };
        if !known.contains(&name.as_str()) {
            return Err(LinkError::Cbor(CborError::UnknownCriticalField {
                field: name.clone(),
            }));
        }
    }
    Ok(())
}

fn content_id_from(value: &Value, field: &'static str) -> Result<ContentId, LinkError> {
    let entries = map_entries(value, field)?;
    check_known(entries, &["digest", "profile"])?;
    let token = required_text(entries, "profile")?;
    let profile = HashProfile::from_token(&token)
        .ok_or(IdShapeError::UnsupportedProfile { profile: token })?;
    let digest = match required(entries, "digest")? {
        Value::Bytes(bytes) => {
            <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_error| LinkError::Field {
                field,
                problem: "has the wrong length",
            })?
        }
        _ => {
            return Err(LinkError::Field {
                field,
                problem: "must be a byte string",
            });
        }
    };
    Ok(ContentId::new(profile, digest))
}

/// A path reference (design §3): a path relative to the namespace
/// root, naming things that may have no hash — a file in a
/// namespace, a folder, a not-yet-hashed object. The stored form is
/// Unicode NFC text with `/` as the only separator, no leading `/`,
/// and no empty, `.` or `..` segments. Construction is the only way
/// in, so a `PathRef` value always satisfies the syntax.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PathRef(String);

impl PathRef {
    /// Validates `raw` into its stored form: NFC-normalised (never
    /// rejected for being non-NFC), then checked against the §3
    /// syntax and the [`MAX_PATH_REF_BYTES`] cap.
    ///
    /// # Errors
    ///
    /// [`LinkError::ResourceLimit`] (class `resource_limit`) when
    /// the normalised form is over the cap;
    /// [`LinkError::InvalidPathRef`] (class `invalid_path_ref`) for
    /// a leading `/` or an empty, `.` or `..` segment.
    pub fn new(raw: &str) -> Result<Self, LinkError> {
        let stored: String = raw.nfc().collect();
        if stored.len() > MAX_PATH_REF_BYTES {
            return Err(LinkError::ResourceLimit {
                limit: MAX_PATH_REF_BYTES,
                actual: stored.len(),
            });
        }
        if stored.starts_with('/') {
            return Err(LinkError::InvalidPathRef {
                problem: "must be relative to the namespace root (no leading '/')",
            });
        }
        for segment in stored.split('/') {
            match segment {
                "" => {
                    return Err(LinkError::InvalidPathRef {
                        problem: "segments must not be empty",
                    });
                }
                "." => {
                    return Err(LinkError::InvalidPathRef {
                        problem: "'.' segments are not allowed",
                    });
                }
                ".." => {
                    return Err(LinkError::InvalidPathRef {
                        problem: "'..' segments are not allowed",
                    });
                }
                _ => {}
            }
        }
        Ok(Self(stored))
    }

    /// The stored (NFC) form of the path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PathRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether `relation` has the design §4 vocabulary shape: a
/// lowercase ASCII identifier, `[a-z][a-z0-9_-]*`. Advisory only —
/// the vocabulary is open, readers preserve unknown relations
/// verbatim, and nothing here rejects a link over its relation.
#[must_use]
pub fn is_valid_relation(relation: &str) -> bool {
    let mut chars = relation.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// What a [`RelatedLink`] points at: exactly the four kinds of
/// design §2. Anything else is not representable as a value of this
/// type and is rejected at the payload boundary with
/// `unsupported_target_kind` (see [`LinkTarget::from_value`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// A content ID.
    ContentRef(ContentId),
    /// A sidecar snapshot ID.
    SnapshotRef(SnapshotId),
    /// A path reference (§3).
    PathRef(PathRef),
    /// A typed location reference (#9115).
    LocationRef(LocationRef),
}

impl LinkTarget {
    /// The payload tag naming this target's kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::ContentRef(_) => "content",
            Self::SnapshotRef(_) => "snapshot",
            Self::PathRef(_) => "path",
            Self::LocationRef(_) => "location",
        }
    }

    /// The target as a canonical CBOR value: a map carrying its
    /// `kind` tag and the kind's payload. Its canonical encoding
    /// ([`LinkTarget::canonical_bytes`]) is what the canonical link
    /// ordering sorts by.
    pub fn to_value(&self) -> Value {
        match self {
            Self::ContentRef(id) => map(vec![
                ("id", content_id_value(id)),
                ("kind", text("content")),
            ]),
            Self::SnapshotRef(id) => map(vec![
                ("id", content_id_value(id.content_id())),
                ("kind", text("snapshot")),
            ]),
            Self::PathRef(path) => map(vec![("kind", text("path")), ("path", text(path.as_str()))]),
            Self::LocationRef(reference) => map(vec![
                ("kind", text("location")),
                (
                    "location",
                    map(vec![
                        ("locator", text(&reference.locator().to_tagged_string())),
                        ("namespace", text(reference.namespace().as_str())),
                        ("tenant_scope", text(reference.tenant_scope())),
                    ]),
                ),
            ]),
        }
    }

    /// The canonical CBOR bytes of [`LinkTarget::to_value`]: the
    /// ordering key of design §2.
    ///
    /// # Errors
    ///
    /// A canonical-encoding error.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, LinkError> {
        Ok(encode_canonical(&self.to_value())?)
    }

    /// Reads a target from a decoded canonical value.
    ///
    /// # Errors
    ///
    /// [`LinkError::UnsupportedTargetKind`] (class
    /// `unsupported_target_kind`) for a `kind` tag outside the four
    /// of design §2 — an external URL, an opaque blob, or a kind a
    /// later version added; the §3 errors for a malformed path
    /// target; [`LinkError::Field`] for a missing or mistyped field.
    pub fn from_value(value: &Value) -> Result<Self, LinkError> {
        let entries = map_entries(value, "target")?;
        let kind = required_text(entries, "kind")?;
        match kind.as_str() {
            "content" => {
                check_known(entries, &["id", "kind"])?;
                Ok(Self::ContentRef(content_id_from(
                    required(entries, "id")?,
                    "id",
                )?))
            }
            "snapshot" => {
                check_known(entries, &["id", "kind"])?;
                Ok(Self::SnapshotRef(EnvelopeId::from_content_id(
                    content_id_from(required(entries, "id")?, "id")?,
                )))
            }
            "path" => {
                check_known(entries, &["kind", "path"])?;
                Ok(Self::PathRef(PathRef::new(&required_text(
                    entries, "path",
                )?)?))
            }
            "location" => {
                check_known(entries, &["kind", "location"])?;
                let location = map_entries(required(entries, "location")?, "location")?;
                check_known(location, &["locator", "namespace", "tenant_scope"])?;
                let locator = TypedLocator::parse_tagged(&required_text(location, "locator")?)?;
                let namespace =
                    NamespaceId::new(required_text(location, "namespace")?).map_err(|_error| {
                        LinkError::Field {
                            field: "namespace",
                            problem: "is not a valid namespace id",
                        }
                    })?;
                Ok(Self::LocationRef(LocationRef::new(
                    locator,
                    namespace,
                    required_text(location, "tenant_scope")?,
                )?))
            }
            _ => Err(LinkError::UnsupportedTargetKind { kind }),
        }
    }
}

/// A typed related link (design §2): an open-vocabulary relation
/// plus its target. Carried in the sidecar payload under
/// [`LINKS_VIEW_KEY`]; never part of the subject's content bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelatedLink {
    /// The relation: an open vocabulary of lowercase ASCII
    /// identifiers (design §4), preserved verbatim on read.
    pub relation: String,
    /// What the link points at.
    pub target: LinkTarget,
}

impl RelatedLink {
    /// A link with `relation` to `target`. The relation is recorded
    /// verbatim; see [`is_valid_relation`] for the vocabulary shape.
    pub fn new(relation: impl Into<String>, target: LinkTarget) -> Self {
        Self {
            relation: relation.into(),
            target,
        }
    }

    /// The link as a canonical CBOR value.
    pub fn to_value(&self) -> Value {
        map(vec![
            ("target", self.target.to_value()),
            ("relation", text(&self.relation)),
        ])
    }

    /// Reads a link from a decoded canonical value. An unknown
    /// relation is preserved verbatim and never fails the read
    /// (design §2, §4).
    ///
    /// # Errors
    ///
    /// As [`LinkTarget::from_value`], or [`LinkError::Field`] for a
    /// missing or mistyped field.
    pub fn from_value(value: &Value) -> Result<Self, LinkError> {
        let entries = map_entries(value, "link")?;
        check_known(entries, &["relation", "target"])?;
        Ok(Self {
            relation: required_text(entries, "relation")?,
            target: LinkTarget::from_value(required(entries, "target")?)?,
        })
    }

    /// The canonical ordering key of design §2: the relation, then
    /// the target's canonical bytes.
    fn sort_key(&self) -> Result<(String, Vec<u8>), LinkError> {
        Ok((self.relation.clone(), self.target.canonical_bytes()?))
    }
}

/// The canonical form of a link set (design §2): sorted by
/// `(relation, canonical target bytes)`, exact duplicates
/// collapsed. Any input order of the same set gives the same
/// output, so payloads built from it hash identically.
///
/// # Errors
///
/// A canonical-encoding error.
pub fn canonical_links(links: &[RelatedLink]) -> Result<Vec<RelatedLink>, LinkError> {
    let mut keyed = Vec::with_capacity(links.len());
    for link in links {
        keyed.push((link.sort_key()?, link.clone()));
    }
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    keyed.dedup_by(|later, earlier| later.0 == earlier.0);
    Ok(keyed.into_iter().map(|(_, link)| link).collect())
}

/// The links a metadata view carries, in stored (canonical) order.
/// A view with no [`LINKS_VIEW_KEY`] entry carries no links.
///
/// # Errors
///
/// As [`RelatedLink::from_value`], or [`LinkError::Field`] when
/// the entry is not an array.
pub fn view_links(view: &BTreeMap<String, Value>) -> Result<Vec<RelatedLink>, LinkError> {
    let Some(value) = view.get(LINKS_VIEW_KEY) else {
        return Ok(Vec::new());
    };
    let Value::Array(items) = value else {
        return Err(LinkError::Field {
            field: "related_links",
            problem: "must be an array",
        });
    };
    items.iter().map(RelatedLink::from_value).collect()
}

/// Writes `links` into a metadata view in canonical form (design
/// §2), replacing any links the view already carries. Writing never
/// checks that a target exists: a dangling link is valid to store
/// (design §5). An empty link set removes the entry, so the
/// canonical payload of a link-less view carries no link key at
/// all (absent, never an empty array or null).
///
/// # Errors
///
/// A canonical-encoding error.
pub fn set_view_links(
    view: &mut BTreeMap<String, Value>,
    links: &[RelatedLink],
) -> Result<(), LinkError> {
    let canonical = canonical_links(links)?;
    if canonical.is_empty() {
        view.remove(LINKS_VIEW_KEY);
        return Ok(());
    }
    let items = canonical.iter().map(RelatedLink::to_value).collect();
    view.insert(LINKS_VIEW_KEY.to_owned(), Value::Array(items));
    Ok(())
}

/// The links a signed snapshot carries, read from its payload view.
///
/// # Errors
///
/// The snapshot's own view decoding failure, or as [`view_links`].
pub fn envelope_links(envelope: &SidecarEnvelopeV1) -> Result<Vec<RelatedLink>, LinkError> {
    view_links(&envelope.view()?)
}

/// The outcome of resolving one link (design §5): the target is
/// there, or it is [`LinkResolution::Dangling`] — a distinct result
/// carrying the target verbatim, never an error and never a silent
/// substitute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkResolution {
    /// The target exists under the resolver's knowledge.
    Resolved {
        /// The target that resolved.
        target: LinkTarget,
    },
    /// The target does not exist (deleted, moved, or never
    /// replicated here). The link itself stays stored and is
    /// preserved on rewrite; staleness is a read-time signal, not a
    /// write-time rule.
    Dangling {
        /// The target that did not resolve.
        target: LinkTarget,
    },
}

impl LinkResolution {
    /// The target this resolution is about, resolved or dangling.
    #[must_use]
    pub const fn target(&self) -> &LinkTarget {
        match self {
            Self::Resolved { target } | Self::Dangling { target } => target,
        }
    }

    /// Whether the target did not resolve.
    #[must_use]
    pub const fn is_dangling(&self) -> bool {
        matches!(self, Self::Dangling { .. })
    }
}

/// Resolves one link against the caller's existence knowledge:
/// `exists` decides, per target, whether the target is there (a
/// content or snapshot is in the store, a path resolves in the
/// namespace, a location is reachable). This module holds no store,
/// so existence is the caller's evidence; a target the caller
/// cannot vouch for resolves [`LinkResolution::Dangling`], never an
/// error. Resolution is read-only: it never rewrites or prunes the
/// link (design §5).
pub fn resolve_link(link: &RelatedLink, exists: impl Fn(&LinkTarget) -> bool) -> LinkResolution {
    if exists(&link.target) {
        LinkResolution::Resolved {
            target: link.target.clone(),
        }
    } else {
        LinkResolution::Dangling {
            target: link.target.clone(),
        }
    }
}

/// Resolves a link set, in the order given. See [`resolve_link`].
pub fn resolve_links(
    links: &[RelatedLink],
    exists: impl Fn(&LinkTarget) -> bool,
) -> Vec<LinkResolution> {
    links
        .iter()
        .map(|link| resolve_link(link, &exists))
        .collect()
}
