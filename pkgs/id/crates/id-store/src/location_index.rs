//! Per-scope location index harvested from complete signed sidecars,
//! with signed stale/delete tombstones and a reconciliation cursor
//! (slice A9115-complete-harvest, issue #10255; parent #9115; design
//! `part-a-addressing/versioned-design.md` section "#9115 — Existence
//! acknowledgements, location harvester, privacy", A-9115.2,
//! statements S3.26–S3.38).
//!
//! The index is a rebuildable projection (S3.36): its entire state is
//! the fold of validated complete signed sidecars
//! ([`crate::sidecar_snapshot::SidecarEnvelopeV1`], slice
//! A9114-sidecar-snapshot) and signed tombstones
//! ([`LocationTombstoneV1`], defined here) for one scope — one
//! namespace, one tenant scope. Nothing else mutates it, and
//! [`LocationIndex::rebuild`] from the same validated records yields
//! exactly the incrementally folded index at the same scope epoch.
//!
//! Harvest rules (A-9115.2):
//!
//! - a valid, authorized, visible location in a complete signed
//!   sidecar is always inserted; there is no opt-in field switch that
//!   silently omits otherwise-visible locations (S3.29: ALL location
//!   records found are inserted, not only a primary one);
//! - location assertions live in the sidecar's complete view under
//!   [`LOCATIONS_VIEW_KEY`] as an array of assertion maps
//!   `{content_id, occurrence_ref?, source_version_ref?, locator,
//!   namespace, tenant_scope, observed_at, access_class, signer}`;
//!   the namespace, tenant scope and signer are committed again in
//!   every assertion and must equal the signed body's, so an
//!   assertion can never smuggle a location into a scope its signer
//!   did not sign for;
//! - the locator is a [`TypedLocator`] in its canonical tagged text
//!   form (slice A9115-locator-types, #10200): R2, laptop and Drive
//!   locations never pretend to resolve identically (S3.30–S3.33);
//! - a location leaves the fresh state only through a signed
//!   tombstone: stale or deleted locations are not erased without a
//!   tombstone, so a later complete snapshot that simply omits a
//!   location changes nothing (S3.38, designed here);
//! - results expose `fresh | stale | deleted | unverified`,
//!   provenance (the snapshot or tombstone that last set the state)
//!   and occurrence/version lineage.
//!
//! Ordering and the reconciliation cursor: every folded event has a
//! total order within the scope. A snapshot at chain generation `g`
//! orders at `(g, 0, 0)`; a tombstone issued against generation `g`
//! (its basis) with per-scope sequence `s` orders at `(g, 1, s)`, so
//! a tombstone always folds after the snapshot it was issued against
//! and before any later snapshot. Incremental application enforces
//! the order (a non-successor snapshot or an out-of-order tombstone
//! is rejected, never reordered silently); rebuild sorts by it. The
//! [`ScopeCursor`] is the scope epoch: chain head plus last folded
//! order.
//!
//! The authenticated tenant/scope context is consumed, not defined,
//! here (#9083): the index is constructed for one scope and rejects
//! any record signed for another.
//!
//! The crate-root export belongs to slice 9110-02; until that lands
//! the integration test includes this module (and
//! `sidecar_snapshot`) by path.

use std::collections::BTreeMap;
use std::fmt;

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};

use id_core::canonical_cbor::{CborError, Value, decode_canonical, encode_canonical};
use id_core::hash_profile::ProfileDispatch;
use id_core::id_shape::{ContentId, HashProfile, IdShapeError};
use id_core::location_ref::{LocationError, TypedLocator};
use id_core::signed_envelope::{EnvelopeId, SIGNATURE_DOMAIN_PREFIX, SUITE_ED25519};

use crate::sidecar_snapshot::{SidecarEnvelopeV1, SidecarError, SnapshotId};

/// The record format version this module writes and reads.
pub const RECORD_VERSION: u64 = 1;

/// Signing domain of a location tombstone record.
pub const TOMBSTONE_DOMAIN: &str = "id.location-tombstone/v1";

/// The view key under which a complete sidecar view carries its
/// location assertions (an array of assertion maps).
pub const LOCATIONS_VIEW_KEY: &str = "locations";

/// Upper bound on namespace, tenant-scope, signer and reference
/// text, in bytes (mirrors the envelope-identities label bound).
pub const MAX_LABEL_BYTES: usize = 256;

/// Identity of a signed location tombstone: the envelope ID of its
/// complete signed record, computed after signing.
pub type TombstoneId = EnvelopeId;

/// The access class of a location assertion, as committed in the
/// sidecar view. The class decides the harvested state, never
/// whether a visible location is indexed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessClass {
    /// Publicly visible location.
    Public,
    /// Visible within the scope (the privacy rule's default:
    /// private within the caller-authorized namespace/tenant).
    Scope,
    /// A signed `stored_unverified` inventory observation
    /// (A-9115.1): an assertion about unverified storage, never
    /// proof of accepted or validated content. Harvested into the
    /// `unverified` state, distinct from `fresh`.
    StoredUnverified,
}

impl AccessClass {
    /// The token this class is committed under.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Scope => "scope",
            Self::StoredUnverified => "stored_unverified",
        }
    }

    /// Parses a committed access-class token.
    ///
    /// # Errors
    ///
    /// [`LocationIndexError::Field`] for an unknown token: classes
    /// are critical, never guessed.
    pub fn from_token(token: &str) -> Result<Self, LocationIndexError> {
        match token {
            "public" => Ok(Self::Public),
            "scope" => Ok(Self::Scope),
            "stored_unverified" => Ok(Self::StoredUnverified),
            _ => Err(LocationIndexError::Field {
                field: "access_class",
                problem: "is not a known access class",
            }),
        }
    }

    /// The index state an assertion of this class harvests into.
    const fn harvested_state(self) -> LocationState {
        match self {
            Self::StoredUnverified => LocationState::Unverified,
            Self::Public | Self::Scope => LocationState::Fresh,
        }
    }
}

/// The state of one indexed location, as exposed by queries
/// (A-9115.2: results expose `fresh | stale | deleted |
/// unverified`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationState {
    /// Asserted by a validated sidecar and not since tombstoned.
    Fresh,
    /// A signed stale tombstone named this location.
    Stale,
    /// A signed delete tombstone named this location.
    Deleted,
    /// Asserted only as unverified storage (A-9115.1).
    Unverified,
}

impl LocationState {
    /// The token this state is persisted and reported under.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Stale => "stale",
            Self::Deleted => "deleted",
            Self::Unverified => "unverified",
        }
    }

    /// Parses a persisted state token.
    ///
    /// # Errors
    ///
    /// [`LocationIndexError::Field`] for an unknown token.
    pub fn from_token(token: &str) -> Result<Self, LocationIndexError> {
        match token {
            "fresh" => Ok(Self::Fresh),
            "stale" => Ok(Self::Stale),
            "deleted" => Ok(Self::Deleted),
            "unverified" => Ok(Self::Unverified),
            _ => Err(LocationIndexError::Field {
                field: "state",
                problem: "is not a known location state",
            }),
        }
    }
}

/// One location assertion, decoded from a complete sidecar view
/// (design A-9115.2 shape). Namespace and tenant scope are carried
/// per the design and validated against the signed body at fold
/// time; they are not stored on the harvested entry, whose scope is
/// the index's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationAssertion {
    /// The content this location holds.
    pub content_id: ContentId,
    /// Optional occurrence lineage reference.
    pub occurrence_ref: Option<String>,
    /// Optional source-version lineage reference.
    pub source_version_ref: Option<String>,
    /// The typed location.
    pub locator: TypedLocator,
    /// Namespace the assertion claims; must equal the signed
    /// body's.
    pub namespace_id: String,
    /// Tenant scope the assertion claims; must equal the signed
    /// body's.
    pub tenant_scope: String,
    /// Observation time (unix milliseconds), advisory.
    pub observed_at: u64,
    /// The assertion's access class.
    pub access_class: AccessClass,
    /// Who asserts, as the signer names itself; must equal the
    /// signed body's signer.
    pub signer: String,
}

impl LocationAssertion {
    /// Decodes one assertion from its view value, strictly: every
    /// field is critical and unknown fields reject.
    ///
    /// # Errors
    ///
    /// [`LocationIndexError`] for any structural, ID or locator
    /// problem.
    pub fn from_value(value: &Value) -> Result<Self, LocationIndexError> {
        let fields = Fields::new(
            value,
            "location assertion",
            &[
                "access_class",
                "content_id",
                "locator",
                "namespace",
                "observed_at",
                "occurrence_ref",
                "signer",
                "source_version_ref",
                "tenant_scope",
            ],
        )?;
        let content_id = content_id_from(fields.required("content_id")?, "content_id")?;
        let locator_text = fields.text("locator")?;
        let locator = TypedLocator::parse_tagged(&locator_text)?;
        let access_class = AccessClass::from_token(&fields.text("access_class")?)?;
        let namespace_id = fields.text("namespace")?;
        check_label("namespace", &namespace_id)?;
        let tenant_scope = fields.text("tenant_scope")?;
        check_label("tenant_scope", &tenant_scope)?;
        let signer = fields.text("signer")?;
        check_label("signer", &signer)?;
        let occurrence_ref = fields.optional_text("occurrence_ref")?;
        if let Some(reference) = &occurrence_ref {
            check_label("occurrence_ref", reference)?;
        }
        let source_version_ref = fields.optional_text("source_version_ref")?;
        if let Some(reference) = &source_version_ref {
            check_label("source_version_ref", reference)?;
        }
        Ok(Self {
            content_id,
            occurrence_ref,
            source_version_ref,
            locator,
            namespace_id,
            tenant_scope,
            observed_at: fields.uint("observed_at")?,
            access_class,
            signer,
        })
    }

    /// The canonical view value of this assertion.
    ///
    /// # Errors
    ///
    /// Never in practice; kept symmetric with the record encoders.
    pub fn to_value(&self) -> Result<Value, LocationIndexError> {
        let mut entries = vec![
            ("access_class", text(self.access_class.token())),
            ("content_id", content_id_value(&self.content_id)),
            ("locator", text(&self.locator.to_tagged_string())),
            ("namespace", text(&self.namespace_id)),
            ("observed_at", Value::UInt(self.observed_at)),
            ("signer", text(&self.signer)),
            ("tenant_scope", text(&self.tenant_scope)),
        ];
        if let Some(reference) = &self.occurrence_ref {
            entries.push(("occurrence_ref", text(reference)));
        }
        if let Some(reference) = &self.source_version_ref {
            entries.push(("source_version_ref", text(reference)));
        }
        Ok(map(entries))
    }
}

/// What kind of removal a tombstone records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TombstoneKind {
    /// The location is stale: superseded or moved, not asserted
    /// gone.
    Stale,
    /// The location is deleted.
    Delete,
}

impl TombstoneKind {
    /// The token this kind is committed under.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Stale => "stale",
            Self::Delete => "delete",
        }
    }

    /// Parses a committed tombstone-kind token.
    ///
    /// # Errors
    ///
    /// [`LocationIndexError::Field`] for an unknown token.
    pub fn from_token(token: &str) -> Result<Self, LocationIndexError> {
        match token {
            "stale" => Ok(Self::Stale),
            "delete" => Ok(Self::Delete),
            _ => Err(LocationIndexError::Field {
                field: "kind",
                problem: "is not a known tombstone kind",
            }),
        }
    }

    /// The entry state this kind sets when it wins the fold.
    const fn applied_state(self) -> LocationState {
        match self {
            Self::Stale => LocationState::Stale,
            Self::Delete => LocationState::Deleted,
        }
    }
}

/// The canonical signed body of a location tombstone. Commits the
/// target (content + locator), the scope, the basis generation it
/// was issued against, its per-scope sequence, and the signer —
/// never the signature or its own tombstone ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TombstoneBodyV1 {
    /// The snapshot-chain generation this tombstone was issued
    /// against; the tombstone folds after that snapshot.
    pub basis_generation: u64,
    /// The content whose location is tombstoned.
    pub content_id: ContentId,
    /// Signing domain and context; [`TOMBSTONE_DOMAIN`] in V1.
    pub domain_context: String,
    /// Stale or delete.
    pub kind: TombstoneKind,
    /// The tombstoned location, in canonical tagged text form.
    pub locator: TypedLocator,
    /// The scope's namespace.
    pub namespace_id: String,
    /// Observation time (unix milliseconds), advisory.
    pub observed_at: u64,
    /// Per-scope tombstone sequence, monotone in fold order among
    /// tombstones sharing a basis generation.
    pub sequence: u64,
    /// Who signed, as the signer names itself.
    pub signer_id: String,
    /// The signer's Ed25519 public key, carried in full so the
    /// record verifies standalone.
    pub key: [u8; 32],
    /// The signature suite; `ed25519` in V1.
    pub suite: String,
    /// The scope's tenant scope.
    pub tenant_scope: String,
}

impl TombstoneBodyV1 {
    /// The body as a canonical CBOR value.
    ///
    /// # Errors
    ///
    /// Never in practice; kept symmetric with the other encoders.
    pub fn to_value(&self) -> Result<Value, LocationIndexError> {
        Ok(map(vec![
            ("basis_generation", Value::UInt(self.basis_generation)),
            ("content_id", content_id_value(&self.content_id)),
            ("domain_context", text(&self.domain_context)),
            ("key", Value::Bytes(self.key.to_vec())),
            ("kind", text(self.kind.token())),
            ("locator", text(&self.locator.to_tagged_string())),
            ("namespace", text(&self.namespace_id)),
            ("observed_at", Value::UInt(self.observed_at)),
            ("sequence", Value::UInt(self.sequence)),
            ("signer", text(&self.signer_id)),
            ("suite", text(&self.suite)),
            ("tenant_scope", text(&self.tenant_scope)),
            ("version", Value::UInt(RECORD_VERSION)),
        ]))
    }

    /// The canonical bytes of this body: the bytes the signature
    /// covers (under the domain framing) and the tombstone ID
    /// hashes together with the signature.
    ///
    /// # Errors
    ///
    /// A canonical-encoding error.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, LocationIndexError> {
        Ok(encode_canonical(&self.to_value()?)?)
    }

    /// Decodes a body from its canonical value, strictly.
    ///
    /// # Errors
    ///
    /// [`LocationIndexError`] for any structural problem, an
    /// unsupported version, or an invalid locator.
    pub fn from_value(value: &Value) -> Result<Self, LocationIndexError> {
        let fields = Fields::new(
            value,
            "tombstone body",
            &[
                "basis_generation",
                "content_id",
                "domain_context",
                "key",
                "kind",
                "locator",
                "namespace",
                "observed_at",
                "sequence",
                "signer",
                "suite",
                "tenant_scope",
                "version",
            ],
        )?;
        let version = fields.uint("version")?;
        if version != RECORD_VERSION {
            return Err(LocationIndexError::UnsupportedVersion(version));
        }
        let domain_context = fields.text("domain_context")?;
        if domain_context != TOMBSTONE_DOMAIN {
            return Err(LocationIndexError::Field {
                field: "domain_context",
                problem: "is not the location tombstone domain",
            });
        }
        let namespace_id = fields.text("namespace")?;
        check_label("namespace", &namespace_id)?;
        let tenant_scope = fields.text("tenant_scope")?;
        check_label("tenant_scope", &tenant_scope)?;
        let signer_id = fields.text("signer")?;
        check_label("signer", &signer_id)?;
        let key_bytes = fields.bytes("key")?;
        let key = <[u8; 32]>::try_from(key_bytes.as_slice()).map_err(|_error| {
            LocationIndexError::Field {
                field: "key",
                problem: "has the wrong length",
            }
        })?;
        let locator_text = fields.text("locator")?;
        Ok(Self {
            basis_generation: fields.uint("basis_generation")?,
            content_id: content_id_from(fields.required("content_id")?, "content_id")?,
            domain_context,
            kind: TombstoneKind::from_token(&fields.text("kind")?)?,
            locator: TypedLocator::parse_tagged(&locator_text)?,
            namespace_id,
            observed_at: fields.uint("observed_at")?,
            sequence: fields.uint("sequence")?,
            signer_id,
            key,
            suite: fields.text("suite")?,
            tenant_scope,
        })
    }
}

/// A signed location tombstone: the body and the detached
/// signature over the body's preimage, in the same envelope
/// discipline as the sidecar records (A-9114.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationTombstoneV1 {
    /// The signed body.
    pub body: TombstoneBodyV1,
    /// Ed25519 signature over the body's signature preimage.
    pub signature: [u8; 64],
}

impl LocationTombstoneV1 {
    /// Signs `body` with `key`.
    ///
    /// # Errors
    ///
    /// [`LocationIndexError::KeyMismatch`] when `key` is not the
    /// body's key, [`LocationIndexError::UnsupportedSuite`] for a
    /// non-Ed25519 suite, or an encoding error.
    pub fn sign(body: TombstoneBodyV1, key: &SigningKey) -> Result<Self, LocationIndexError> {
        if key.verifying_key().to_bytes() != body.key {
            return Err(LocationIndexError::KeyMismatch);
        }
        check_suite(&body.suite)?;
        let signature = key
            .sign(&signature_preimage(
                &body.domain_context,
                &body.canonical_bytes()?,
            )?)
            .to_bytes();
        Ok(Self { body, signature })
    }

    /// The tombstone as a canonical CBOR value.
    ///
    /// # Errors
    ///
    /// As [`TombstoneBodyV1::to_value`].
    pub fn to_value(&self) -> Result<Value, LocationIndexError> {
        Ok(map(vec![
            ("body", self.body.to_value()?),
            ("signature", Value::Bytes(self.signature.to_vec())),
        ]))
    }

    /// The canonical bytes of the complete signed record: the
    /// bytes the tombstone ID hashes.
    ///
    /// # Errors
    ///
    /// A canonical-encoding error.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, LocationIndexError> {
        Ok(encode_canonical(&self.to_value()?)?)
    }

    /// Decodes a canonical tombstone. Decoding checks structure
    /// only; call [`LocationTombstoneV1::verify`] before trusting
    /// it.
    ///
    /// # Errors
    ///
    /// [`LocationIndexError`] for non-canonical bytes or any field
    /// problem.
    pub fn decode(bytes: &[u8]) -> Result<Self, LocationIndexError> {
        let value = decode_canonical(bytes)?;
        let fields = Fields::new(&value, "tombstone", &["body", "signature"])?;
        let signature = fields.bytes("signature")?;
        Ok(Self {
            body: TombstoneBodyV1::from_value(fields.required("body")?)?,
            signature: <[u8; 64]>::try_from(signature.as_slice()).map_err(|_error| {
                LocationIndexError::Field {
                    field: "signature",
                    problem: "has the wrong length",
                }
            })?,
        })
    }

    /// The tombstone's identity: the envelope ID over its complete
    /// canonical bytes under the raw BLAKE3 profile (the profile
    /// the tombstone ID commits; tombstones carry no payload, so
    /// there is no payload profile to inherit).
    ///
    /// # Errors
    ///
    /// An encoding error.
    pub fn tombstone_id(&self) -> Result<TombstoneId, LocationIndexError> {
        Ok(EnvelopeId::from_content_id(ContentId::of_bytes(
            HashProfile::RawBlake3_256,
            &self.canonical_bytes()?,
        )?))
    }

    /// Verifies the suite and the signature against the body's
    /// key.
    ///
    /// # Errors
    ///
    /// [`LocationIndexError`], classed by
    /// [`LocationIndexError::class`].
    pub fn verify(&self) -> Result<(), LocationIndexError> {
        check_suite(&self.body.suite)?;
        let key = VerifyingKey::from_bytes(&self.body.key)
            .map_err(|_error| LocationIndexError::BadKey)?;
        key.verify_strict(
            &signature_preimage(&self.body.domain_context, &self.body.canonical_bytes()?)?,
            &Signature::from_bytes(&self.signature),
        )
        .map_err(|_error| LocationIndexError::BadSignature)
    }

    /// This tombstone's position in the scope's fold order:
    /// `(basis_generation, 1, sequence)`.
    const fn order(&self) -> EventOrder {
        EventOrder {
            generation: self.body.basis_generation,
            rank: 1,
            sequence: self.body.sequence,
        }
    }
}

/// The total order of one folded event within a scope: a snapshot
/// at generation `g` is `(g, 0, 0)`; a tombstone issued against
/// generation `g` is `(g, 1, sequence)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct EventOrder {
    generation: u64,
    rank: u8,
    sequence: u64,
}

/// The reconciliation cursor of one scope — its epoch. Records
/// the snapshot chain head and the last folded event order; two
/// indexes with equal cursors and equal entries are the same
/// projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeCursor {
    /// Generation of the chain head snapshot, if any.
    pub head_generation: Option<u64>,
    /// The chain head snapshot, if any.
    pub head_snapshot_id: Option<SnapshotId>,
    /// Basis generation of the last folded tombstone, if any.
    pub last_tombstone_basis: Option<u64>,
    /// Sequence of the last folded tombstone, if any.
    pub last_tombstone_sequence: Option<u64>,
}

/// One harvested location: its state, the lineage the assertion
/// carried, and its provenance — the record that last set the
/// state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationEntry {
    /// The content this location holds.
    pub content_id: ContentId,
    /// The typed location.
    pub locator: TypedLocator,
    /// The current state.
    pub state: LocationState,
    /// Occurrence lineage from the latest assertion, if any.
    pub occurrence_ref: Option<String>,
    /// Source-version lineage from the latest assertion, if any.
    pub source_version_ref: Option<String>,
    /// Latest observation time (assertion or tombstone), advisory.
    pub observed_at: u64,
    /// Access class of the latest assertion; `None` only for an
    /// entry created by a tombstone before any assertion folded.
    pub access_class: Option<AccessClass>,
    /// Signer of the record that last set the state.
    pub signer: String,
    /// The snapshot that last asserted this location, if any.
    pub asserted_by: Option<SnapshotId>,
    /// The generation of that snapshot, if any.
    pub asserted_generation: Option<u64>,
    /// The tombstone that last set the state, if the last event
    /// was a tombstone.
    pub tombstoned_by: Option<TombstoneId>,
}

/// The outcome of folding one complete sidecar: how many
/// assertions it carried and how they harvested. Withheld is
/// always zero — visible assertions are never omitted — and is
/// reported so a future visibility change cannot become silent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HarvestOutcome {
    /// Assertions decoded from the view.
    pub asserted: usize,
    /// Assertions harvested into `fresh`.
    pub fresh: usize,
    /// Assertions harvested into `unverified`.
    pub unverified: usize,
    /// Assertions withheld from the index. Always 0 in V1.
    pub withheld: usize,
}

/// Map key of one entry: content key then locator tagged form.
type EntryKey = (String, String);

fn entry_key(content_id: &ContentId, locator: &TypedLocator) -> EntryKey {
    (
        format!(
            "{}/{}",
            content_id.profile().token(),
            hex::encode(content_id.digest())
        ),
        locator.to_tagged_string(),
    )
}

/// The per-scope location index: a rebuildable projection folded
/// from validated sidecars and signed tombstones for one
/// namespace + tenant scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationIndex {
    namespace_id: String,
    tenant_scope: String,
    entries: BTreeMap<EntryKey, LocationEntry>,
    head_generation: Option<u64>,
    head_snapshot_id: Option<SnapshotId>,
    last_order: Option<EventOrder>,
    last_tombstone: Option<(u64, u64)>,
}

impl LocationIndex {
    /// An empty index for one scope.
    ///
    /// # Errors
    ///
    /// [`LocationIndexError::Field`] for an empty or over-long
    /// scope label.
    pub fn new(
        namespace_id: impl Into<String>,
        tenant_scope: impl Into<String>,
    ) -> Result<Self, LocationIndexError> {
        let namespace_id = namespace_id.into();
        check_label("namespace", &namespace_id)?;
        let tenant_scope = tenant_scope.into();
        check_label("tenant_scope", &tenant_scope)?;
        Ok(Self {
            namespace_id,
            tenant_scope,
            entries: BTreeMap::new(),
            head_generation: None,
            head_snapshot_id: None,
            last_order: None,
            last_tombstone: None,
        })
    }

    /// The namespace this index is scoped to.
    #[must_use]
    pub fn namespace_id(&self) -> &str {
        &self.namespace_id
    }

    /// The tenant scope this index is scoped to.
    #[must_use]
    pub fn tenant_scope(&self) -> &str {
        &self.tenant_scope
    }

    /// The scope's reconciliation cursor (its epoch).
    #[must_use]
    pub fn cursor(&self) -> ScopeCursor {
        ScopeCursor {
            head_generation: self.head_generation,
            head_snapshot_id: self.head_snapshot_id,
            last_tombstone_basis: self.last_tombstone.map(|(basis, _)| basis),
            last_tombstone_sequence: self.last_tombstone.map(|(_, sequence)| sequence),
        }
    }

    /// Number of harvested entries, in every state.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the index holds no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entry for one content at one location, if harvested.
    #[must_use]
    pub fn entry(&self, content_id: &ContentId, locator: &TypedLocator) -> Option<&LocationEntry> {
        self.entries.get(&entry_key(content_id, locator))
    }

    /// Every harvested location of one content, in locator order,
    /// each exposing its state, provenance and lineage. The query
    /// is confined to this index's scope by construction — there
    /// is no cross-scope query (privacy rule: queries filter
    /// authorization before returning candidates, and this index
    /// only ever holds its own scope).
    #[must_use]
    pub fn locations_for(&self, content_id: &ContentId) -> Vec<&LocationEntry> {
        let prefix = format!(
            "{}/{}",
            content_id.profile().token(),
            hex::encode(content_id.digest())
        );
        self.entries
            .range((prefix.clone(), String::new())..)
            .take_while(|(key, _)| key.0 == prefix)
            .map(|(_, entry)| entry)
            .collect()
    }

    /// All entries, in key order.
    pub fn entries(&self) -> impl Iterator<Item = &LocationEntry> {
        self.entries.values()
    }

    /// Folds one complete signed sidecar into the index.
    ///
    /// The envelope is verified here (payload commitment and
    /// signature), its scope must be this index's, and it must be
    /// the exact successor of the folded chain head — genesis
    /// (generation 0, no parent) first. Every visible assertion in
    /// its view is harvested; assertions naming another scope or
    /// another signer than the signed body's are rejected, and a
    /// rejected snapshot folds nothing.
    ///
    /// # Errors
    ///
    /// [`LocationIndexError`], classed by
    /// [`LocationIndexError::class`].
    pub fn apply_snapshot(
        &mut self,
        envelope: &SidecarEnvelopeV1,
        dispatch: &ProfileDispatch,
    ) -> Result<HarvestOutcome, LocationIndexError> {
        envelope.verify(dispatch)?;
        let body = &envelope.body;
        self.check_scope(&body.namespace_id, &body.tenant_scope)?;
        match (self.head_generation, self.head_snapshot_id) {
            (None, None) => {
                if body.generation != 0 || body.parent_snapshot_id.is_some() {
                    return Err(LocationIndexError::GenerationMismatch {
                        expected: 0,
                        actual: body.generation,
                    });
                }
            }
            (Some(head_generation), Some(head_id)) => {
                let expected = head_generation + 1;
                if body.generation != expected {
                    return Err(LocationIndexError::GenerationMismatch {
                        expected,
                        actual: body.generation,
                    });
                }
                if body.parent_snapshot_id != Some(head_id) {
                    return Err(LocationIndexError::ParentMismatch);
                }
            }
            // Generation and head ID advance together in this fold;
            // a split pair is an index invariant violation, never a
            // record property.
            (Some(_), None) | (None, Some(_)) => {
                return Err(LocationIndexError::Field {
                    field: "head",
                    problem: "index head generation and snapshot id disagree",
                });
            }
        }
        let snapshot_id = envelope.snapshot_id()?;
        let view = envelope.view()?;
        let assertions = decode_assertions(&view)?;
        for assertion in &assertions {
            if assertion.namespace_id != body.namespace_id
                || assertion.tenant_scope != body.tenant_scope
            {
                return Err(LocationIndexError::ScopeMismatch);
            }
            if assertion.signer != body.signer_id {
                return Err(LocationIndexError::AssertionSignerMismatch);
            }
        }
        let mut outcome = HarvestOutcome {
            asserted: assertions.len(),
            fresh: 0,
            unverified: 0,
            withheld: 0,
        };
        for assertion in assertions {
            let state = assertion.access_class.harvested_state();
            if state == LocationState::Fresh {
                outcome.fresh += 1;
            } else {
                // Access classes harvest fresh or unverified only.
                outcome.unverified += 1;
            }
            let key = entry_key(&assertion.content_id, &assertion.locator);
            self.entries.insert(
                key,
                LocationEntry {
                    content_id: assertion.content_id,
                    locator: assertion.locator,
                    state,
                    occurrence_ref: assertion.occurrence_ref,
                    source_version_ref: assertion.source_version_ref,
                    observed_at: assertion.observed_at,
                    access_class: Some(assertion.access_class),
                    signer: assertion.signer,
                    asserted_by: Some(snapshot_id),
                    asserted_generation: Some(body.generation),
                    tombstoned_by: None,
                },
            );
        }
        self.head_generation = Some(body.generation);
        self.head_snapshot_id = Some(snapshot_id);
        self.last_order = Some(EventOrder {
            generation: body.generation,
            rank: 0,
            sequence: 0,
        });
        Ok(outcome)
    }

    /// Folds one signed tombstone into the index.
    ///
    /// The tombstone is verified here, its scope must be this
    /// index's, its basis generation must already be folded, and
    /// its fold order must strictly follow the last folded event.
    /// A delete wins over every prior state; a stale tombstone
    /// never resurrects a deleted location. A tombstone naming a
    /// location never asserted creates the entry in the
    /// tombstone's state, so a later assertion (which folds later
    /// by construction) is what revives it — never an earlier one.
    ///
    /// # Errors
    ///
    /// [`LocationIndexError`], classed by
    /// [`LocationIndexError::class`].
    pub fn apply_tombstone(
        &mut self,
        tombstone: &LocationTombstoneV1,
    ) -> Result<(), LocationIndexError> {
        tombstone.verify()?;
        let body = &tombstone.body;
        self.check_scope(&body.namespace_id, &body.tenant_scope)?;
        let Some(head_generation) = self.head_generation else {
            return Err(LocationIndexError::BasisAhead {
                basis: body.basis_generation,
                head: None,
            });
        };
        if body.basis_generation > head_generation {
            return Err(LocationIndexError::BasisAhead {
                basis: body.basis_generation,
                head: Some(head_generation),
            });
        }
        let order = tombstone.order();
        if let Some(last) = self.last_order {
            if order <= last {
                return Err(LocationIndexError::OutOfOrder);
            }
        }
        let tombstone_id = tombstone.tombstone_id()?;
        let key = entry_key(&body.content_id, &body.locator);
        match self.entries.get_mut(&key) {
            Some(entry) => {
                if !(entry.state == LocationState::Deleted && body.kind == TombstoneKind::Stale) {
                    entry.state = body.kind.applied_state();
                }
                entry.observed_at = body.observed_at;
                entry.signer = body.signer_id.clone();
                entry.tombstoned_by = Some(tombstone_id);
            }
            None => {
                self.entries.insert(
                    key,
                    LocationEntry {
                        content_id: body.content_id,
                        locator: body.locator.clone(),
                        state: body.kind.applied_state(),
                        occurrence_ref: None,
                        source_version_ref: None,
                        observed_at: body.observed_at,
                        access_class: None,
                        signer: body.signer_id.clone(),
                        asserted_by: None,
                        asserted_generation: None,
                        tombstoned_by: Some(tombstone_id),
                    },
                );
            }
        }
        self.last_order = Some(order);
        self.last_tombstone = Some((body.basis_generation, body.sequence));
        Ok(())
    }

    /// Rebuilds an index from validated records in any arrival
    /// order (S3.27/S3.28: batch and on-receipt harvesting are a
    /// deployment choice, not a format difference — both folds are
    /// this fold). Snapshots sort by chain generation, tombstones
    /// by fold order; the result equals the incrementally folded
    /// index at the same scope epoch, or the first invalid record
    /// rejects the rebuild.
    ///
    /// # Errors
    ///
    /// [`LocationIndexError`] from the first record that does not
    /// validate or fold.
    pub fn rebuild(
        namespace_id: impl Into<String>,
        tenant_scope: impl Into<String>,
        snapshots: &[SidecarEnvelopeV1],
        tombstones: &[LocationTombstoneV1],
        dispatch: &ProfileDispatch,
    ) -> Result<Self, LocationIndexError> {
        let mut index = Self::new(namespace_id, tenant_scope)?;
        let mut snapshots: Vec<&SidecarEnvelopeV1> = snapshots.iter().collect();
        snapshots.sort_by_key(|envelope| envelope.body.generation);
        let mut tombstones: Vec<&LocationTombstoneV1> = tombstones.iter().collect();
        tombstones.sort_by_key(|tombstone| tombstone.order());
        // Merge the two sorted streams by fold order.
        let mut snap_iter = snapshots.into_iter().peekable();
        let mut tomb_iter = tombstones.into_iter().peekable();
        loop {
            let snap_order = snap_iter.peek().map(|envelope| EventOrder {
                generation: envelope.body.generation,
                rank: 0,
                sequence: 0,
            });
            let tomb_order = tomb_iter.peek().map(|tombstone| tombstone.order());
            match (snap_order, tomb_order) {
                (None, None) => break,
                (Some(_), None) => {
                    let envelope = snap_iter.next().ok_or(LocationIndexError::OutOfOrder)?;
                    index.apply_snapshot(envelope, dispatch)?;
                }
                (None, Some(_)) => {
                    let tombstone = tomb_iter.next().ok_or(LocationIndexError::OutOfOrder)?;
                    index.apply_tombstone(tombstone)?;
                }
                (Some(s), Some(t)) => {
                    if s < t {
                        let envelope = snap_iter.next().ok_or(LocationIndexError::OutOfOrder)?;
                        index.apply_snapshot(envelope, dispatch)?;
                    } else {
                        let tombstone = tomb_iter.next().ok_or(LocationIndexError::OutOfOrder)?;
                        index.apply_tombstone(tombstone)?;
                    }
                }
            }
        }
        Ok(index)
    }

    fn check_scope(
        &self,
        namespace_id: &str,
        tenant_scope: &str,
    ) -> Result<(), LocationIndexError> {
        if namespace_id != self.namespace_id || tenant_scope != self.tenant_scope {
            return Err(LocationIndexError::ScopeMismatch);
        }
        Ok(())
    }

    /// The index as a canonical CBOR value: scope, cursor and
    /// every entry, so the projection can be persisted and
    /// reloaded without losing provenance.
    ///
    /// # Errors
    ///
    /// A canonical-encoding error from an entry encoder.
    pub fn to_value(&self) -> Result<Value, LocationIndexError> {
        let mut entry_values = Vec::with_capacity(self.entries.len());
        for entry in self.entries.values() {
            entry_values.push(entry_to_value(entry)?);
        }
        let mut entries = vec![
            ("entries", Value::Array(entry_values)),
            ("namespace", text(&self.namespace_id)),
            ("tenant_scope", text(&self.tenant_scope)),
            ("version", Value::UInt(RECORD_VERSION)),
        ];
        if let (Some(generation), Some(id)) = (self.head_generation, self.head_snapshot_id) {
            entries.push(("head_generation", Value::UInt(generation)));
            entries.push(("head_snapshot_id", content_id_value(id.content_id())));
        }
        if let Some((basis, sequence)) = self.last_tombstone {
            entries.push(("last_tombstone_basis", Value::UInt(basis)));
            entries.push(("last_tombstone_sequence", Value::UInt(sequence)));
        }
        Ok(map(entries))
    }

    /// The canonical bytes of this index.
    ///
    /// # Errors
    ///
    /// As [`LocationIndex::to_value`].
    pub fn encode(&self) -> Result<Vec<u8>, LocationIndexError> {
        Ok(encode_canonical(&self.to_value()?)?)
    }

    /// Decodes a persisted index, strictly, re-validating every
    /// entry's locator and state.
    ///
    /// # Errors
    ///
    /// [`LocationIndexError`] for non-canonical bytes or any field
    /// problem.
    pub fn decode(bytes: &[u8]) -> Result<Self, LocationIndexError> {
        let value = decode_canonical(bytes)?;
        let fields = Fields::new(
            &value,
            "location index",
            &[
                "entries",
                "head_generation",
                "head_snapshot_id",
                "last_tombstone_basis",
                "last_tombstone_sequence",
                "namespace",
                "tenant_scope",
                "version",
            ],
        )?;
        let version = fields.uint("version")?;
        if version != RECORD_VERSION {
            return Err(LocationIndexError::UnsupportedVersion(version));
        }
        let mut index = Self::new(fields.text("namespace")?, fields.text("tenant_scope")?)?;
        let head_generation = fields.optional_uint("head_generation")?;
        let head_snapshot_id = match fields.optional("head_snapshot_id") {
            Some(value) => Some(EnvelopeId::from_content_id(content_id_from(
                value,
                "head_snapshot_id",
            )?)),
            None => None,
        };
        if head_generation.is_some() != head_snapshot_id.is_some() {
            return Err(LocationIndexError::Field {
                field: "head_snapshot_id",
                problem: "must be present exactly when a head generation is",
            });
        }
        index.head_generation = head_generation;
        index.head_snapshot_id = head_snapshot_id;
        let basis = fields.optional_uint("last_tombstone_basis")?;
        let sequence = fields.optional_uint("last_tombstone_sequence")?;
        match (basis, sequence) {
            (Some(basis), Some(sequence)) => {
                index.last_tombstone = Some((basis, sequence));
                index.last_order = Some(EventOrder {
                    generation: basis,
                    rank: 1,
                    sequence,
                });
            }
            (None, None) => {
                if let Some(generation) = head_generation {
                    index.last_order = Some(EventOrder {
                        generation,
                        rank: 0,
                        sequence: 0,
                    });
                }
            }
            _ => {
                return Err(LocationIndexError::Field {
                    field: "last_tombstone_sequence",
                    problem: "must be present exactly when a tombstone basis is",
                });
            }
        }
        let Some(Value::Array(items)) = fields.optional("entries") else {
            return Err(LocationIndexError::Field {
                field: "entries",
                problem: "must be an array",
            });
        };
        for item in items {
            let entry = entry_from_value(item)?;
            let key = entry_key(&entry.content_id, &entry.locator);
            index.entries.insert(key, entry);
        }
        Ok(index)
    }
}

fn entry_to_value(entry: &LocationEntry) -> Result<Value, LocationIndexError> {
    let mut entries = vec![
        ("content_id", content_id_value(&entry.content_id)),
        ("locator", text(&entry.locator.to_tagged_string())),
        ("observed_at", Value::UInt(entry.observed_at)),
        ("signer", text(&entry.signer)),
        ("state", text(entry.state.token())),
    ];
    if let Some(class) = entry.access_class {
        entries.push(("access_class", text(class.token())));
    }
    if let Some(id) = entry.asserted_by {
        entries.push(("asserted_by", content_id_value(id.content_id())));
    }
    if let Some(generation) = entry.asserted_generation {
        entries.push(("asserted_generation", Value::UInt(generation)));
    }
    if let Some(reference) = &entry.occurrence_ref {
        entries.push(("occurrence_ref", text(reference)));
    }
    if let Some(reference) = &entry.source_version_ref {
        entries.push(("source_version_ref", text(reference)));
    }
    if let Some(id) = entry.tombstoned_by {
        entries.push(("tombstoned_by", content_id_value(id.content_id())));
    }
    Ok(map(entries))
}

fn entry_from_value(value: &Value) -> Result<LocationEntry, LocationIndexError> {
    let fields = Fields::new(
        value,
        "location entry",
        &[
            "access_class",
            "asserted_by",
            "asserted_generation",
            "content_id",
            "locator",
            "observed_at",
            "occurrence_ref",
            "signer",
            "source_version_ref",
            "state",
            "tombstoned_by",
        ],
    )?;
    let locator_text = fields.text("locator")?;
    Ok(LocationEntry {
        content_id: content_id_from(fields.required("content_id")?, "content_id")?,
        locator: TypedLocator::parse_tagged(&locator_text)?,
        state: LocationState::from_token(&fields.text("state")?)?,
        occurrence_ref: fields.optional_text("occurrence_ref")?,
        source_version_ref: fields.optional_text("source_version_ref")?,
        observed_at: fields.uint("observed_at")?,
        access_class: match fields.optional("access_class") {
            Some(Value::Text(token)) => Some(AccessClass::from_token(token)?),
            Some(_) => {
                return Err(LocationIndexError::Field {
                    field: "access_class",
                    problem: "must be text",
                });
            }
            None => None,
        },
        signer: fields.text("signer")?,
        asserted_by: match fields.optional("asserted_by") {
            Some(value) => Some(EnvelopeId::from_content_id(content_id_from(
                value,
                "asserted_by",
            )?)),
            None => None,
        },
        asserted_generation: fields.optional_uint("asserted_generation")?,
        tombstoned_by: match fields.optional("tombstoned_by") {
            Some(value) => Some(EnvelopeId::from_content_id(content_id_from(
                value,
                "tombstoned_by",
            )?)),
            None => None,
        },
    })
}

/// Decodes the location assertions of one complete view. An
/// absent [`LOCATIONS_VIEW_KEY`] is a complete view with no
/// locations; a present one must be an array of assertion maps.
fn decode_assertions(
    view: &BTreeMap<String, Value>,
) -> Result<Vec<LocationAssertion>, LocationIndexError> {
    let Some(value) = view.get(LOCATIONS_VIEW_KEY) else {
        return Ok(Vec::new());
    };
    let Value::Array(items) = value else {
        return Err(LocationIndexError::Field {
            field: "locations",
            problem: "must be an array of assertion maps",
        });
    };
    items.iter().map(LocationAssertion::from_value).collect()
}

/// Why a location record, fold step or persisted index was
/// rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocationIndexError {
    /// The bytes are not canonical CBOR, or carry an unknown
    /// field.
    Cbor(CborError),
    /// An ID inside a record was rejected.
    Id(IdShapeError),
    /// A locator inside a record was rejected.
    Location(LocationError),
    /// A sidecar snapshot failed its own validation.
    Sidecar(SidecarError),
    /// A field is missing or has the wrong type or size.
    Field {
        /// The field name.
        field: &'static str,
        /// What was wrong.
        problem: &'static str,
    },
    /// The record version is not [`RECORD_VERSION`].
    UnsupportedVersion(u64),
    /// The suite is not one this reader verifies.
    UnsupportedSuite(String),
    /// The record's scope is not the index's scope.
    ScopeMismatch,
    /// An assertion's signer is not the signed body's signer.
    AssertionSignerMismatch,
    /// A snapshot generation is not the exact successor (or
    /// genesis) value.
    GenerationMismatch {
        /// The generation the fold required.
        expected: u64,
        /// The generation the record carries.
        actual: u64,
    },
    /// A snapshot's parent is not the folded chain head.
    ParentMismatch,
    /// A tombstone's basis generation has not been folded yet.
    BasisAhead {
        /// The basis generation the tombstone names.
        basis: u64,
        /// The folded head generation, if any.
        head: Option<u64>,
    },
    /// An event's fold order does not strictly follow the last
    /// folded event (a duplicate folds here too).
    OutOfOrder,
    /// The signing key does not match the body's committed key.
    KeyMismatch,
    /// The body's key is not a valid Ed25519 public key.
    BadKey,
    /// The signature does not verify.
    BadSignature,
}

impl LocationIndexError {
    /// The stable error-class token (the fold's error oracle
    /// compares classes, never message text).
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::Cbor(_) => "cbor",
            Self::Id(_) => "id",
            Self::Location(_) => "location",
            Self::Sidecar(_) => "sidecar",
            Self::Field { .. } => "field",
            Self::UnsupportedVersion(_) => "unsupported_version",
            Self::UnsupportedSuite(_) => "unsupported_suite",
            Self::ScopeMismatch => "scope_mismatch",
            Self::AssertionSignerMismatch => "assertion_signer_mismatch",
            Self::GenerationMismatch { .. } => "generation_mismatch",
            Self::ParentMismatch => "parent_mismatch",
            Self::BasisAhead { .. } => "basis_ahead",
            Self::OutOfOrder => "out_of_order",
            Self::KeyMismatch => "key_mismatch",
            Self::BadKey => "bad_key",
            Self::BadSignature => "bad_signature",
        }
    }
}

impl fmt::Display for LocationIndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cbor(error) => write!(f, "canonical CBOR rejected: {error}"),
            Self::Id(error) => write!(f, "ID rejected: {error}"),
            Self::Location(error) => write!(f, "locator rejected: {error}"),
            Self::Sidecar(error) => write!(f, "sidecar rejected: {error}"),
            Self::Field { field, problem } => write!(f, "field {field} {problem}"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported location record version {version}")
            }
            Self::UnsupportedSuite(suite) => write!(f, "unsupported signature suite {suite}"),
            Self::ScopeMismatch => write!(f, "record scope is not the index scope"),
            Self::AssertionSignerMismatch => {
                write!(f, "assertion signer is not the sidecar signer")
            }
            Self::GenerationMismatch { expected, actual } => {
                write!(f, "generation is {actual} but the fold requires {expected}")
            }
            Self::ParentMismatch => write!(f, "snapshot parent is not the folded chain head"),
            Self::BasisAhead { basis, head } => {
                write!(f, "tombstone basis {basis} is ahead of head {head:?}")
            }
            Self::OutOfOrder => write!(f, "event does not follow the last folded event"),
            Self::KeyMismatch => write!(f, "signing key does not match the committed key"),
            Self::BadKey => write!(f, "committed key is not a valid Ed25519 public key"),
            Self::BadSignature => write!(f, "signature does not verify"),
        }
    }
}

impl std::error::Error for LocationIndexError {}

impl From<CborError> for LocationIndexError {
    fn from(error: CborError) -> Self {
        Self::Cbor(error)
    }
}

impl From<IdShapeError> for LocationIndexError {
    fn from(error: IdShapeError) -> Self {
        Self::Id(error)
    }
}

impl From<LocationError> for LocationIndexError {
    fn from(error: LocationError) -> Self {
        Self::Location(error)
    }
}

impl From<SidecarError> for LocationIndexError {
    fn from(error: SidecarError) -> Self {
        Self::Sidecar(error)
    }
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn content_id_value(id: &ContentId) -> Value {
    // Keys in canonical order: "digest" < "profile".
    Value::Map(vec![
        (text("digest"), Value::Bytes(id.digest().to_vec())),
        (text("profile"), text(id.profile().token())),
    ])
}

/// Builds a map from `(key, value)` pairs, in canonical key
/// order (the encoder sorts too; this keeps the value itself
/// canonical).
fn map(mut entries: Vec<(&'static str, Value)>) -> Value {
    entries.sort_by(|(a, _), (b, _)| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    Value::Map(entries.into_iter().map(|(k, v)| (text(k), v)).collect())
}

const fn check_label(field: &'static str, label: &str) -> Result<(), LocationIndexError> {
    if label.is_empty() {
        return Err(LocationIndexError::Field {
            field,
            problem: "must not be empty",
        });
    }
    if label.len() > MAX_LABEL_BYTES {
        return Err(LocationIndexError::Field {
            field,
            problem: "is too long",
        });
    }
    Ok(())
}

fn check_suite(suite: &str) -> Result<(), LocationIndexError> {
    if suite == SUITE_ED25519 {
        Ok(())
    } else {
        Err(LocationIndexError::UnsupportedSuite(suite.to_owned()))
    }
}

/// The exact bytes a tombstone signature covers, in the
/// envelope-identities framing: `SIGNATURE_DOMAIN_PREFIX ||
/// u32_be(len(domain)) || domain || canonical_cbor(record)`.
fn signature_preimage(
    domain: &str,
    canonical_record: &[u8],
) -> Result<Vec<u8>, LocationIndexError> {
    let domain_len = u32::try_from(domain.len()).map_err(|_error| LocationIndexError::Field {
        field: "domain",
        problem: "is too long",
    })?;
    let mut preimage = Vec::with_capacity(
        SIGNATURE_DOMAIN_PREFIX.len() + 4 + domain.len() + canonical_record.len(),
    );
    preimage.extend_from_slice(SIGNATURE_DOMAIN_PREFIX);
    preimage.extend_from_slice(&domain_len.to_be_bytes());
    preimage.extend_from_slice(domain.as_bytes());
    preimage.extend_from_slice(canonical_record);
    Ok(preimage)
}

fn content_id_from(value: &Value, field: &'static str) -> Result<ContentId, LocationIndexError> {
    let fields = Fields::new(value, field, &["digest", "profile"])?;
    let token = fields.text("profile")?;
    let profile = HashProfile::from_token(&token)
        .ok_or(IdShapeError::UnsupportedProfile { profile: token })?;
    let digest = fixed_bytes(fields.required("digest")?, field)?;
    Ok(ContentId::new(profile, digest))
}

fn fixed_bytes<const N: usize>(
    value: &Value,
    field: &'static str,
) -> Result<[u8; N], LocationIndexError> {
    match value {
        Value::Bytes(bytes) => {
            <[u8; N]>::try_from(bytes.as_slice()).map_err(|_error| LocationIndexError::Field {
                field,
                problem: "has the wrong length",
            })
        }
        _ => Err(LocationIndexError::Field {
            field,
            problem: "must be a byte string",
        }),
    }
}

/// A decoded map whose every key is known.
struct Fields<'a> {
    entries: &'a [(Value, Value)],
}

impl<'a> Fields<'a> {
    fn new(
        value: &'a Value,
        record: &'static str,
        known: &[&'static str],
    ) -> Result<Self, LocationIndexError> {
        let Value::Map(entries) = value else {
            return Err(LocationIndexError::Field {
                field: record,
                problem: "must be a map",
            });
        };
        for (key, _) in entries {
            let Value::Text(name) = key else {
                return Err(LocationIndexError::Field {
                    field: record,
                    problem: "keys must be text",
                });
            };
            if !known.contains(&name.as_str()) {
                // Every V1 field is critical.
                return Err(LocationIndexError::Cbor(CborError::UnknownCriticalField {
                    field: name.clone(),
                }));
            }
        }
        Ok(Self { entries })
    }

    fn optional(&self, field: &str) -> Option<&'a Value> {
        self.entries
            .iter()
            .find(|(key, _)| matches!(key, Value::Text(name) if name == field))
            .map(|(_, value)| value)
    }

    fn required(&self, field: &'static str) -> Result<&'a Value, LocationIndexError> {
        self.optional(field).ok_or(LocationIndexError::Field {
            field,
            problem: "is missing",
        })
    }

    fn text(&self, field: &'static str) -> Result<String, LocationIndexError> {
        match self.required(field)? {
            Value::Text(value) => Ok(value.clone()),
            _ => Err(LocationIndexError::Field {
                field,
                problem: "must be text",
            }),
        }
    }

    fn optional_text(&self, field: &'static str) -> Result<Option<String>, LocationIndexError> {
        match self.optional(field) {
            None => Ok(None),
            Some(Value::Text(value)) => Ok(Some(value.clone())),
            Some(_) => Err(LocationIndexError::Field {
                field,
                problem: "must be text",
            }),
        }
    }

    fn uint(&self, field: &'static str) -> Result<u64, LocationIndexError> {
        match self.required(field)? {
            Value::UInt(value) => Ok(*value),
            _ => Err(LocationIndexError::Field {
                field,
                problem: "must be an unsigned integer",
            }),
        }
    }

    fn optional_uint(&self, field: &'static str) -> Result<Option<u64>, LocationIndexError> {
        match self.optional(field) {
            None => Ok(None),
            Some(Value::UInt(value)) => Ok(Some(*value)),
            Some(_) => Err(LocationIndexError::Field {
                field,
                problem: "must be an unsigned integer",
            }),
        }
    }

    fn bytes(&self, field: &'static str) -> Result<Vec<u8>, LocationIndexError> {
        match self.required(field)? {
            Value::Bytes(value) => Ok(value.clone()),
            _ => Err(LocationIndexError::Field {
                field,
                problem: "must be a byte string",
            }),
        }
    }
}
