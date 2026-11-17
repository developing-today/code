//! Provider declaration model — the #5898 decision record implemented
//! (issue #8362; record `doc/2026-11-11T00-18-00Z_decision_id5898_provider_declaration_model.md`).
//!
//! One signed provider manifest per (provider identity, instance) is the
//! sole declaration artefact (D1). This module defines the D2 field set,
//! the canonical encoding the provider signature covers (the concrete
//! encoding choice issue #8362 scope item 1 delegates), the registry's
//! dynamic-filing path (signature verification at admission and on every
//! change, D3), forward-only version/epoch enforcement (D4), the D6
//! lifecycle, and the filed-declaration view discovery reads (I5).
//!
//! The compile-time catalog in [`crate::world_caps`] is the degenerate
//! case (D1): [`project_static`] projects each static entry into the D2
//! shape with host-attested provenance and `ttl_class: static`, so a
//! built-in and a runtime-filed provider are indistinguishable downstream
//! of admission.
//!
//! The two open questions routed to parent #5686 (key-rotation semantics;
//! federation-scope admission) are NOT answered here: the record's
//! conservative default holds — a rotated key is a new provider identity
//! that must be admitted from `excluded` like any other.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use sha2::{Digest as _, Sha256};

use crate::world_caps::{
    ArgumentSchema, ArgumentType, CLIENT_PROVIDER_NAMESPACE, DefaultCeiling, ProviderBound,
    ProviderKind, ProviderManifest, ProviderRegistry, ProviderSide, RequiredScope,
};

/// The declaration format version this implementation reads and writes.
/// The crypto suite is named by the format version, never negotiated per
/// manifest (D2): format 1 is ed25519 signatures over the canonical
/// encoding defined by [`ProviderDeclaration::canonical_bytes`].
pub const CURRENT_FORMAT_VERSION: u32 = 1;

/// Domain separator for the manifest canonical encoding.
const MANIFEST_DOMAIN: &[u8] = b"id.provider-manifest.v1";
/// Domain separator for the withdrawal canonical encoding (D6).
const WITHDRAWAL_DOMAIN: &[u8] = b"id.provider-withdrawal.v1";
/// Domain separator for deriving a built-in provider's host-attested
/// identity from its catalog name (D1 projection).
const HOST_ATTESTED_DOMAIN: &[u8] = b"id.host-attested-provider:";

/// Extension namespaces this implementation understands. An unknown
/// *required* extension is a refusal (D2); an unknown optional one is
/// ignored. `id.display` carries display metadata such as the
/// semantic-version string D4 allows as non-binding metadata.
pub const KNOWN_EXTENSION_NAMESPACES: &[&str] = &["id.display"];

/// Extension namespaces that would smuggle D2-prohibited contents
/// (credentials, grants, executable code, endpoint-as-identity) into a
/// declaration. Filing one is refused as prohibited content.
const PROHIBITED_EXTENSION_ROOTS: &[&str] = &[
    "credential",
    "credentials",
    "secret",
    "secrets",
    "grant",
    "grants",
    "code",
    "endpoint",
    "endpoints",
];

/// A provider identity: the ed25519 public key that *is* the provider
/// (#5905 D1). Names and transport addresses are never identity (I10).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProviderIdentity(pub [u8; 32]);

impl ProviderIdentity {
    /// The identity for a verifying key.
    #[must_use]
    pub fn from_verifying_key(key: &VerifyingKey) -> Self {
        Self(key.to_bytes())
    }

    /// The verifying key, if the bytes are a valid ed25519 point.
    pub fn verifying_key(&self) -> Result<VerifyingKey, DeclarationRefusal> {
        VerifyingKey::from_bytes(&self.0).map_err(|_| DeclarationRefusal::BadSignature)
    }

    /// Lowercase hex form, for records and audit display.
    #[must_use]
    pub fn to_hex(&self) -> String {
        to_hex(&self.0)
    }
}

/// Sensitivity class S0–S3 — a manifest fact, never a per-request
/// judgement (#5900, D2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SensitivityClass {
    /// Least sensitive.
    S0,
    /// Low sensitivity.
    S1,
    /// Elevated sensitivity.
    S2,
    /// Most sensitive.
    S3,
}

/// Observation vs actuation — different risk classes, always
/// distinguishable from the declaration alone (D2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectClass {
    /// Reads or reports state.
    Observation,
    /// Changes state in the world.
    Actuation,
}

/// A capability's contract version (D4, axis 3): same major = compatible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContractVersion {
    /// Breaking axis: schema or bounds semantics changed incompatibly.
    pub major: u32,
    /// Compatible axis: narrowing or optional additions only.
    pub minor: u32,
}

impl ContractVersion {
    /// Whether a consumer written against `other` may use this version:
    /// same major is compatible (parent F13.7 rule, consumed by D4).
    #[must_use]
    pub fn compatible_with(&self, other: &ContractVersion) -> bool {
        self.major == other.major
    }
}

/// Where a declaration is filed (D2 `scope`, discharging #5899 §3.1
/// inside the artefact).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeclarationScope {
    /// A world id.
    World(String),
    /// A server id.
    Server(String),
    /// A federation intent string.
    Federation(String),
}

/// Liveness class per #5899 §5 / #5904 presence classes (D2 `ttl_class`).
/// `Static` is the built-in class (D1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TtlClass {
    /// Built-in / host-attested: lives as long as the binary.
    Static,
    /// Session-bound: lives for its session (D5).
    Session,
    /// Heartbeat-kept: stale after this many seconds without a heartbeat.
    Heartbeat(u64),
}

/// A namespaced optional field (D2 `extensions`). Extensions describe;
/// they never authorise — no extension may widen bounds, scope, class,
/// or ceiling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestExtension {
    /// Namespace, e.g. `id.display`.
    pub namespace: String,
    /// A reader that does not know a required extension must refuse.
    pub required: bool,
    /// Opaque extension payload (canonical string form).
    pub value: String,
}

/// An owned bound in the existing bound vocabulary (D2 `bounds`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeclaredBound {
    /// Maximum serialized request argument size.
    ArgsBytes(usize),
    /// Maximum count or number of entries.
    Items(usize),
    /// Maximum identifier length in bytes.
    IdentifierBytes(usize),
    /// Maximum response page size in bytes.
    ResponseBytes(usize),
    /// Maximum distance into the future in milliseconds.
    HorizonMillis(u64),
    /// A per-world limit whose value is supplied by `WorldLimits`.
    WorldLimit(String),
}

impl From<&ProviderBound> for DeclaredBound {
    fn from(bound: &ProviderBound) -> Self {
        match bound {
            ProviderBound::ArgsBytes(n) => Self::ArgsBytes(*n),
            ProviderBound::Items(n) => Self::Items(*n),
            ProviderBound::IdentifierBytes(n) => Self::IdentifierBytes(*n),
            ProviderBound::ResponseBytes(n) => Self::ResponseBytes(*n),
            ProviderBound::HorizonMillis(n) => Self::HorizonMillis(*n),
            ProviderBound::WorldLimit(name) => Self::WorldLimit((*name).to_owned()),
        }
    }
}

/// One capability entry in a manifest (D2 per-capability field set).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityEntry {
    /// Stable within the provider across versions. Capability identity
    /// is the tuple (provider identity, capability id, manifest version).
    pub capability_id: String,
    /// This capability's contract version (D4, axis 3).
    pub contract_version: ContractVersion,
    /// Typed argument schema in WIT-shaped typed form (#5903 D2);
    /// free-form untyped schemas are not a declaration.
    pub argument_schema: String,
    /// The existing bound vocabulary.
    pub bounds: Vec<DeclaredBound>,
    /// Participant scope required for the effect.
    pub required_scope: RequiredScope,
    /// S0–S3, a manifest fact.
    pub sensitivity_class: SensitivityClass,
    /// Observation or actuation.
    pub effect_class: EffectClass,
    /// Ceiling posture; entries never enter above `excluded` by
    /// declaration alone (#5905 D2).
    pub default_ceiling: DefaultCeiling,
    /// Whether actuation may be queued for later execution; honoured
    /// only when the grant also declares it (#5904). Absent = never.
    pub store_and_forward: bool,
    /// Still invocable, shown as deprecated in discovery (D4).
    pub deprecated: bool,
}

/// The D2 signed field set: one provider instance's whole offer (D1 —
/// there are no partial or per-capability declaration documents).
///
/// Prohibited contents (D2) are enforced structurally and at filing: the
/// type has no field that could carry credentials, grant objects,
/// consumer identities, executable code, or a transport address as
/// identity, and [`DeclarationRegistry::file_signed`] refuses extension
/// namespaces that would smuggle them in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderDeclaration {
    /// Version of the declaration format itself (D4, axis 1).
    pub format_version: u32,
    /// The provider identity (keypair identity, #5905 D1).
    pub provider: ProviderIdentity,
    /// Instance discriminator: one identity may run several instances.
    pub instance: String,
    /// Human/registry name — a label, never identity (I10).
    pub name: String,
    /// Who holds the resource (parent F13.7).
    pub side: ProviderSide,
    /// Provider kind class.
    pub kind: ProviderKind,
    /// Monotonic integer version of this manifest (D4, axis 2).
    pub manifest_version: u64,
    /// Declaration epoch: monotonic per provider identity (D4).
    pub epoch: u64,
    /// Where this declaration is filed.
    pub scope: DeclarationScope,
    /// Liveness class.
    pub ttl_class: TtlClass,
    /// Issue time (unix seconds).
    pub issued_at: u64,
    /// Absolute expiry (unix seconds); use never renews it (#5904 §5).
    pub expires_at: Option<u64>,
    /// The complete capability set for this version (D1, D5).
    pub capabilities: Vec<CapabilityEntry>,
    /// Namespaced optional fields.
    pub extensions: Vec<ManifestExtension>,
}

/// Canonical-encoding writer: domain-separated, field-ordered,
/// length-prefixed. Deterministic by construction — the same field set
/// always produces the same bytes, which is what makes the digest a
/// content address (D3) and the signature meaningful.
struct Canon {
    buf: Vec<u8>,
}

impl Canon {
    fn new(domain: &[u8]) -> Self {
        let mut canon = Self { buf: Vec::new() };
        canon.bytes(domain);
        canon
    }

    fn bytes(&mut self, value: &[u8]) {
        self.buf
            .extend_from_slice(&(value.len() as u32).to_be_bytes());
        self.buf.extend_from_slice(value);
    }

    fn string(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.buf.extend_from_slice(&value.to_be_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.buf.extend_from_slice(&value.to_be_bytes());
    }

    fn boolean(&mut self, value: bool) {
        self.buf.push(u8::from(value));
    }

    fn opt_u64(&mut self, value: Option<u64>) {
        match value {
            Some(inner) => {
                self.buf.push(1);
                self.u64(inner);
            }
            None => self.buf.push(0),
        }
    }
}

fn side_tag(side: ProviderSide) -> u8 {
    match side {
        ProviderSide::Host => 0,
        ProviderSide::Client => 1,
    }
}

fn kind_tag(kind: ProviderKind) -> u8 {
    match kind {
        ProviderKind::Request => 0,
        ProviderKind::Subscription => 1,
        ProviderKind::Event => 2,
        ProviderKind::PlatformModule => 3,
    }
}

fn scope_tag(scope: &RequiredScope) -> u8 {
    match scope {
        RequiredScope::Join => 0,
        RequiredScope::Chat => 1,
        RequiredScope::ChatRead => 2,
    }
}

fn ceiling_tag(ceiling: DefaultCeiling) -> u8 {
    match ceiling {
        DefaultCeiling::Included => 0,
        DefaultCeiling::Excluded => 1,
    }
}

fn sensitivity_tag(class: SensitivityClass) -> u8 {
    match class {
        SensitivityClass::S0 => 0,
        SensitivityClass::S1 => 1,
        SensitivityClass::S2 => 2,
        SensitivityClass::S3 => 3,
    }
}

fn effect_tag(class: EffectClass) -> u8 {
    match class {
        EffectClass::Observation => 0,
        EffectClass::Actuation => 1,
    }
}

impl ProviderDeclaration {
    /// The canonical encoding of the complete signed field set (D3).
    ///
    /// Scope item 1 of #8362 (the concrete canonical-encoding choice):
    /// the ID object model's usual approach — a deterministic,
    /// domain-separated, field-ordered encoding with length-prefixed
    /// variable fields, in D2 field order. The signature field is not
    /// part of the declaration value, so it is excluded by construction.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = Canon::new(MANIFEST_DOMAIN);
        c.u32(self.format_version);
        c.bytes(&self.provider.0);
        c.string(&self.instance);
        c.string(&self.name);
        c.buf.push(side_tag(self.side));
        c.buf.push(kind_tag(self.kind));
        c.u64(self.manifest_version);
        c.u64(self.epoch);
        match &self.scope {
            DeclarationScope::World(id) => {
                c.buf.push(0);
                c.string(id);
            }
            DeclarationScope::Server(id) => {
                c.buf.push(1);
                c.string(id);
            }
            DeclarationScope::Federation(intent) => {
                c.buf.push(2);
                c.string(intent);
            }
        }
        match self.ttl_class {
            TtlClass::Static => c.buf.push(0),
            TtlClass::Session => c.buf.push(1),
            TtlClass::Heartbeat(secs) => {
                c.buf.push(2);
                c.u64(secs);
            }
        }
        c.u64(self.issued_at);
        c.opt_u64(self.expires_at);
        c.u32(self.capabilities.len() as u32);
        for cap in &self.capabilities {
            c.string(&cap.capability_id);
            c.u32(cap.contract_version.major);
            c.u32(cap.contract_version.minor);
            c.string(&cap.argument_schema);
            c.u32(cap.bounds.len() as u32);
            for bound in &cap.bounds {
                match bound {
                    DeclaredBound::ArgsBytes(n) => {
                        c.buf.push(0);
                        c.u64(*n as u64);
                    }
                    DeclaredBound::Items(n) => {
                        c.buf.push(1);
                        c.u64(*n as u64);
                    }
                    DeclaredBound::IdentifierBytes(n) => {
                        c.buf.push(2);
                        c.u64(*n as u64);
                    }
                    DeclaredBound::ResponseBytes(n) => {
                        c.buf.push(3);
                        c.u64(*n as u64);
                    }
                    DeclaredBound::HorizonMillis(n) => {
                        c.buf.push(4);
                        c.u64(*n);
                    }
                    DeclaredBound::WorldLimit(name) => {
                        c.buf.push(5);
                        c.string(name);
                    }
                }
            }
            c.buf.push(scope_tag(&cap.required_scope));
            c.buf.push(sensitivity_tag(cap.sensitivity_class));
            c.buf.push(effect_tag(cap.effect_class));
            c.buf.push(ceiling_tag(cap.default_ceiling));
            c.boolean(cap.store_and_forward);
            c.boolean(cap.deprecated);
        }
        c.u32(self.extensions.len() as u32);
        for ext in &self.extensions {
            c.string(&ext.namespace);
            c.boolean(ext.required);
            c.string(&ext.value);
        }
        c.buf
    }

    /// The manifest digest: the content address every other record
    /// cites — registry admission, discovery cards, grants (D3).
    #[must_use]
    pub fn digest(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.canonical_bytes());
        to_hex(&hasher.finalize())
    }

    /// The binding tuple's version axis for one capability: grants bind
    /// (provider identity, capability id, manifest version) (#5905 D6).
    #[must_use]
    pub fn capability(&self, capability_id: &str) -> Option<&CapabilityEntry> {
        self.capabilities
            .iter()
            .find(|cap| cap.capability_id == capability_id)
    }

    /// Shape validation that does not need registry state: the F13.7
    /// side contract (mirroring `ProviderManifest::side_contract_violation`
    /// for owned declarations), non-empty identifiers, a non-empty
    /// offer, and a coherent issue/expiry window.
    fn validate_shape(&self) -> Result<(), DeclarationRefusal> {
        if self.name.is_empty() || self.instance.is_empty() || self.capabilities.is_empty() {
            return Err(DeclarationRefusal::ProhibitedContent);
        }
        let in_client_namespace = self.name.starts_with(CLIENT_PROVIDER_NAMESPACE);
        match self.side {
            ProviderSide::Client => {
                if !in_client_namespace {
                    return Err(DeclarationRefusal::ProhibitedContent);
                }
                if self
                    .capabilities
                    .iter()
                    .any(|cap| cap.default_ceiling != DefaultCeiling::Excluded)
                {
                    return Err(DeclarationRefusal::ProhibitedContent);
                }
            }
            ProviderSide::Host => {
                if in_client_namespace {
                    return Err(DeclarationRefusal::ProhibitedContent);
                }
            }
        }
        if self
            .capabilities
            .iter()
            .any(|cap| cap.capability_id.is_empty() || cap.argument_schema.is_empty())
        {
            return Err(DeclarationRefusal::ProhibitedContent);
        }
        if let Some(expires_at) = self.expires_at {
            if expires_at <= self.issued_at {
                return Err(DeclarationRefusal::ProhibitedContent);
            }
        }
        for ext in &self.extensions {
            let root = ext.namespace.split('.').next().unwrap_or("");
            if PROHIBITED_EXTENSION_ROOTS.contains(&root) {
                return Err(DeclarationRefusal::ProhibitedContent);
            }
            if ext.required && !KNOWN_EXTENSION_NAMESPACES.contains(&ext.namespace.as_str()) {
                return Err(DeclarationRefusal::UnknownRequiredExtension);
            }
        }
        Ok(())
    }
}

/// A declaration plus its provider signature (D3). The manifest is
/// single-author — its provider; the registry never countersigns it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedManifest {
    /// The signed field set.
    pub declaration: ProviderDeclaration,
    /// Ed25519 signature over `declaration.canonical_bytes()`.
    pub signature: [u8; 64],
}

impl SignedManifest {
    /// Sign a declaration with the provider identity key.
    pub fn sign(declaration: ProviderDeclaration, key: &SigningKey) -> Self {
        let signature = key.sign(&declaration.canonical_bytes()).to_bytes();
        Self {
            declaration,
            signature,
        }
    }

    /// Verify the signature against the declared provider identity.
    /// An unsigned or unverifiable manifest is never filed (I2).
    pub fn verify_signature(&self) -> Result<(), DeclarationRefusal> {
        let key = self.declaration.provider.verifying_key()?;
        let signature = Signature::from_bytes(&self.signature);
        key.verify(&self.declaration.canonical_bytes(), &signature)
            .map_err(|_| DeclarationRefusal::BadSignature)
    }
}

/// A signed withdrawal (D6): an act of the provider identity, so neither
/// a bridge nor a compromised registry can forge one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedWithdrawal {
    /// The withdrawing provider identity.
    pub provider: ProviderIdentity,
    /// The instance withdrawn.
    pub instance: String,
    /// The epoch withdrawn.
    pub epoch: u64,
    /// The manifest version withdrawn.
    pub manifest_version: u64,
    /// Ed25519 signature over the withdrawal's canonical bytes.
    pub signature: [u8; 64],
}

impl SignedWithdrawal {
    /// Sign a withdrawal with the provider identity key.
    pub fn sign(
        provider: ProviderIdentity,
        instance: String,
        epoch: u64,
        manifest_version: u64,
        key: &SigningKey,
    ) -> Self {
        let mut withdrawal = Self {
            provider,
            instance,
            epoch,
            manifest_version,
            signature: [0u8; 64],
        };
        withdrawal.signature = key.sign(&withdrawal.canonical_bytes()).to_bytes();
        withdrawal
    }

    fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = Canon::new(WITHDRAWAL_DOMAIN);
        c.bytes(&self.provider.0);
        c.string(&self.instance);
        c.u64(self.epoch);
        c.u64(self.manifest_version);
        c.buf
    }

    fn verify_signature(&self) -> Result<(), DeclarationRefusal> {
        let key = self.provider.verifying_key()?;
        let signature = Signature::from_bytes(&self.signature);
        key.verify(&self.canonical_bytes(), &signature)
            .map_err(|_| DeclarationRefusal::BadSignature)
    }
}

/// A refusal with a stable, F11.13-class code: refusals are stable
/// codes, never silent drops (D3, D4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclarationRefusal {
    /// The reader does not know this format major version (D4 axis 1).
    UnknownFormatVersion,
    /// Signature missing, malformed, or not matching the identity (I2).
    BadSignature,
    /// A familiar name under a different key is a different provider;
    /// a registry name collision is a refusal, never a rebind (D2, I10).
    NameCollision,
    /// `manifest_version` not above the highest seen for its
    /// (identity, instance, epoch) (D4, I8).
    RollbackVersion,
    /// An epoch lower than the highest seen for the identity (D4, I8).
    RollbackEpoch,
    /// An unknown required extension (D2).
    UnknownRequiredExtension,
    /// The declaration carries (or smuggles) prohibited contents, or
    /// violates the side/shape contract (D2).
    ProhibitedContent,
    /// The manifest is already expired at filing time.
    ExpiredManifest,
    /// No filed record exists under the cited digest.
    UnknownManifest,
    /// The record exists but is in the wrong lifecycle state for the
    /// requested transition (D6).
    WrongLifecycleState,
}

impl DeclarationRefusal {
    /// The stable refusal code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnknownFormatVersion => "declaration.unknown_format_version",
            Self::BadSignature => "declaration.bad_signature",
            Self::NameCollision => "declaration.name_collision",
            Self::RollbackVersion => "declaration.rollback_version",
            Self::RollbackEpoch => "declaration.rollback_epoch",
            Self::UnknownRequiredExtension => "declaration.unknown_required_extension",
            Self::ProhibitedContent => "declaration.prohibited_content",
            Self::ExpiredManifest => "declaration.expired",
            Self::UnknownManifest => "declaration.unknown_manifest",
            Self::WrongLifecycleState => "declaration.wrong_lifecycle_state",
        }
    }
}

impl std::fmt::Display for DeclarationRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for DeclarationRefusal {}

/// The D6 lifecycle: filed → admitted → live → suspended → superseded /
/// stale → tombstoned. Availability is dynamic state held outside the
/// signed manifest (D5, I9) — no transition ever edits a signed field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleState {
    /// Signature-verified and stored, not yet admitted; discoverable
    /// by nobody (D6).
    Filed,
    /// The registry's admission record exists (D3).
    Admitted,
    /// Within TTL/heartbeat and not suspended; only live declarations
    /// appear in discovery views.
    Live,
    /// Registry-side suspension; contents unchanged (D5).
    Suspended,
    /// A higher `manifest_version` in the same epoch is live (D6).
    Superseded,
    /// Past expiry/TTL, awaiting tombstone (D6).
    Stale,
    /// Terminal: expired or withdrawn (D6).
    Tombstoned,
}

/// How a filed record's contents are attested (D1/D3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provenance {
    /// Signed by the provider identity key; verified at filing.
    Signed,
    /// Projected from the compile-time catalog at registry load; the
    /// binary itself is the provenance (D1 degenerate case).
    HostAttested,
}

/// The registry's admission act (D3): a separate registry record
/// referencing the manifest digest — never a countersignature on the
/// manifest. The provider answers for what it offered; the registry
/// answers for what it admitted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmissionRecord {
    /// The admitted manifest's digest.
    pub manifest_digest: String,
    /// Who admitted it (registry actor, or `host-binary` for the
    /// projected static catalog).
    pub admitted_by: String,
    /// Ceiling posture at admission. Runtime filings always enter at
    /// `excluded` (#5905 D2); host-attested projections carry the
    /// catalog entry's own posture so the static catalog behaves
    /// identically downstream (D1).
    pub ceiling: DefaultCeiling,
    /// Admission time (unix seconds).
    pub admitted_at: u64,
}

/// One filed declaration record: the immutable signed artefact plus its
/// dynamic registry state (D5 — the state lives outside the artefact).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FiledRecord {
    /// The declaration as filed. Never mutated after filing (I3).
    pub declaration: ProviderDeclaration,
    /// The manifest digest (content address).
    pub digest: String,
    /// How the contents are attested.
    pub provenance: Provenance,
    /// Current lifecycle state (D6).
    pub state: LifecycleState,
    /// The admission record, once admitted (D3).
    pub admission: Option<AdmissionRecord>,
}

/// A discovery card: the declaration view's projection of one live
/// filed record. Anything discoverable traces to exactly one filed
/// declaration record (I5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclarationCard {
    /// Registry name (a label, never identity).
    pub name: String,
    /// Provider identity.
    pub provider: ProviderIdentity,
    /// Instance discriminator.
    pub instance: String,
    /// Manifest version.
    pub manifest_version: u64,
    /// Declaration epoch.
    pub epoch: u64,
    /// The manifest digest this card traces to.
    pub digest: String,
    /// Capability ids in the live offer.
    pub capability_ids: Vec<String>,
}

/// The declaration registry: the dynamic-filing path of #8362 scope
/// item 4 plus the filed-declaration view of scope item 5.
///
/// In-memory reference implementation of the registry semantics; the
/// store that persists records must preserve the same rules — above
/// all I3 (a filed manifest version never changes) and I8 (versions
/// and epochs move forward only).
#[derive(Clone, Debug, Default)]
pub struct DeclarationRegistry {
    records: BTreeMap<String, FiledRecord>,
    name_owner: HashMap<String, ProviderIdentity>,
    highest: HashMap<(ProviderIdentity, String), (u64, u64)>,
}

impl DeclarationRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Load the compile-time catalog as host-attested declarations
    /// (D1 degenerate case): every static entry is projected into the
    /// D2 shape, filed, admitted by the host binary, and live, so
    /// downstream consumers see one model. Returns the digests in
    /// catalog order. Adding a built-in provider remains exactly "one
    /// manifest entry" — it appears here by projection alone.
    pub fn load_static_catalog(&mut self, registry: &ProviderRegistry) -> Vec<String> {
        let mut digests = Vec::new();
        for entry in registry.entries() {
            let declaration = project_static(entry);
            let digest = declaration.digest();
            let ceiling = entry.default_ceiling;
            self.records.insert(
                digest.clone(),
                FiledRecord {
                    declaration,
                    digest: digest.clone(),
                    provenance: Provenance::HostAttested,
                    state: LifecycleState::Live,
                    admission: Some(AdmissionRecord {
                        manifest_digest: digest.clone(),
                        admitted_by: "host-binary".to_owned(),
                        ceiling,
                        admitted_at: 0,
                    }),
                },
            );
            digests.push(digest);
        }
        digests
    }

    /// File a signed manifest (D3 verification point 1: at admission's
    /// door — here, at filing). Verification order: format version,
    /// signature, shape/prohibited contents, name collision, forward-
    /// only version/epoch, expiry. A refusal files nothing.
    ///
    /// On success the record is `Filed`: stored and verifiable, but
    /// admitted by nobody and discoverable by nobody (D6).
    pub fn file_signed(
        &mut self,
        signed: &SignedManifest,
        now: u64,
    ) -> Result<String, DeclarationRefusal> {
        let declaration = &signed.declaration;
        if declaration.format_version != CURRENT_FORMAT_VERSION {
            return Err(DeclarationRefusal::UnknownFormatVersion);
        }
        signed.verify_signature()?;
        declaration.validate_shape()?;
        if let Some(owner) = self.name_owner.get(&declaration.name) {
            if *owner != declaration.provider {
                return Err(DeclarationRefusal::NameCollision);
            }
        }
        let key = (declaration.provider, declaration.instance.clone());
        if let Some(&(highest_epoch, highest_version)) = self.highest.get(&key) {
            if declaration.epoch < highest_epoch {
                return Err(DeclarationRefusal::RollbackEpoch);
            }
            if declaration.epoch == highest_epoch && declaration.manifest_version <= highest_version
            {
                return Err(DeclarationRefusal::RollbackVersion);
            }
        }
        if let Some(expires_at) = declaration.expires_at {
            if expires_at <= now {
                return Err(DeclarationRefusal::ExpiredManifest);
            }
        }
        let digest = declaration.digest();
        self.highest
            .insert(key, (declaration.epoch, declaration.manifest_version));
        self.name_owner
            .insert(declaration.name.clone(), declaration.provider);
        self.records.insert(
            digest.clone(),
            FiledRecord {
                declaration: declaration.clone(),
                digest: digest.clone(),
                provenance: Provenance::Signed,
                state: LifecycleState::Filed,
                admission: None,
            },
        );
        Ok(digest)
    }

    /// Admit a filed manifest (D3): creates the separate admission
    /// record citing the digest. Runtime filings enter at
    /// `default_ceiling: excluded` (#5905 D2) — admission never widens
    /// a ceiling (I7).
    pub fn admit(
        &mut self,
        digest: &str,
        admitted_by: &str,
        now: u64,
    ) -> Result<AdmissionRecord, DeclarationRefusal> {
        let record = self
            .records
            .get_mut(digest)
            .ok_or(DeclarationRefusal::UnknownManifest)?;
        if record.state != LifecycleState::Filed {
            return Err(DeclarationRefusal::WrongLifecycleState);
        }
        let admission = AdmissionRecord {
            manifest_digest: digest.to_owned(),
            admitted_by: admitted_by.to_owned(),
            ceiling: DefaultCeiling::Excluded,
            admitted_at: now,
        };
        record.admission = Some(admission.clone());
        record.state = LifecycleState::Admitted;
        Ok(admission)
    }

    /// Move an admitted declaration live (D6). Going live supersedes
    /// every lower `manifest_version` of the same (provider, instance,
    /// epoch) that is admitted or live (D5: the discovery view updates
    /// atomically to the new version; superseded versions stay
    /// fetchable by digest, I4).
    pub fn mark_live(&mut self, digest: &str) -> Result<(), DeclarationRefusal> {
        let (provider, instance, epoch, version) = {
            let record = self
                .records
                .get(digest)
                .ok_or(DeclarationRefusal::UnknownManifest)?;
            if record.state != LifecycleState::Admitted {
                return Err(DeclarationRefusal::WrongLifecycleState);
            }
            (
                record.declaration.provider,
                record.declaration.instance.clone(),
                record.declaration.epoch,
                record.declaration.manifest_version,
            )
        };
        for record in self.records.values_mut() {
            let d = &record.declaration;
            if d.provider == provider
                && d.instance == instance
                && d.epoch == epoch
                && d.manifest_version < version
                && matches!(
                    record.state,
                    LifecycleState::Admitted | LifecycleState::Live
                )
            {
                record.state = LifecycleState::Superseded;
            }
        }
        let record = self
            .records
            .get_mut(digest)
            .ok_or(DeclarationRefusal::UnknownManifest)?;
        record.state = LifecycleState::Live;
        Ok(())
    }

    /// Suspend a live declaration (D5/D6): a registry-side state —
    /// contents unchanged, the manifest is not edited (I9).
    pub fn suspend(&mut self, digest: &str) -> Result<(), DeclarationRefusal> {
        let record = self
            .records
            .get_mut(digest)
            .ok_or(DeclarationRefusal::UnknownManifest)?;
        if record.state != LifecycleState::Live {
            return Err(DeclarationRefusal::WrongLifecycleState);
        }
        record.state = LifecycleState::Suspended;
        Ok(())
    }

    /// Resume a suspended declaration whose contents are unchanged
    /// (D5: re-grant of an OS permission resumes the same version).
    pub fn resume(&mut self, digest: &str) -> Result<(), DeclarationRefusal> {
        let record = self
            .records
            .get_mut(digest)
            .ok_or(DeclarationRefusal::UnknownManifest)?;
        if record.state != LifecycleState::Suspended {
            return Err(DeclarationRefusal::WrongLifecycleState);
        }
        record.state = LifecycleState::Live;
        Ok(())
    }

    /// Apply a signed withdrawal (D6): verified against the provider
    /// identity, then the matching record is tombstoned. A withdrawal
    /// naming a different provider, instance, epoch, or version than
    /// any filed record is refused as unknown.
    pub fn withdraw(&mut self, withdrawal: &SignedWithdrawal) -> Result<(), DeclarationRefusal> {
        withdrawal.verify_signature()?;
        let digest = self
            .records
            .values()
            .find(|record| {
                let d = &record.declaration;
                d.provider == withdrawal.provider
                    && d.instance == withdrawal.instance
                    && d.epoch == withdrawal.epoch
                    && d.manifest_version == withdrawal.manifest_version
                    && record.state != LifecycleState::Tombstoned
            })
            .map(|record| record.digest.clone())
            .ok_or(DeclarationRefusal::UnknownManifest)?;
        let record = self
            .records
            .get_mut(&digest)
            .ok_or(DeclarationRefusal::UnknownManifest)?;
        record.state = LifecycleState::Tombstoned;
        Ok(())
    }

    /// Advance time (D6): live or admitted declarations past their
    /// absolute expiry become stale; stale declarations are then
    /// tombstoned by [`DeclarationRegistry::sweep_tombstones`].
    /// Expiry is absolute — use never renews it (#5904 §5).
    pub fn tick(&mut self, now: u64) {
        for record in self.records.values_mut() {
            if matches!(
                record.state,
                LifecycleState::Live | LifecycleState::Admitted
            ) && record.declaration.expires_at.is_some_and(|e| e <= now)
            {
                record.state = LifecycleState::Stale;
            }
        }
    }

    /// Tombstone every stale record (D6 bookkeeping over expiry).
    pub fn sweep_tombstones(&mut self) {
        for record in self.records.values_mut() {
            if record.state == LifecycleState::Stale {
                record.state = LifecycleState::Tombstoned;
            }
        }
    }

    /// Fetch a filed record by digest, whatever its lifecycle state:
    /// audit must be able to reconstruct exactly what was offered when
    /// a grant was made (I4), including superseded versions.
    #[must_use]
    pub fn fetch_by_digest(&self, digest: &str) -> Option<&FiledRecord> {
        self.records.get(digest)
    }

    /// The declaration view discovery reads (#8362 scope item 5):
    /// cards for live declarations only — filed declarations are the
    /// only thing discovery can see (I5), and only live ones appear
    /// (#5899 via D6).
    #[must_use]
    pub fn declaration_view(&self) -> Vec<DeclarationCard> {
        self.records
            .values()
            .filter(|record| record.state == LifecycleState::Live)
            .map(|record| DeclarationCard {
                name: record.declaration.name.clone(),
                provider: record.declaration.provider,
                instance: record.declaration.instance.clone(),
                manifest_version: record.declaration.manifest_version,
                epoch: record.declaration.epoch,
                digest: record.digest.clone(),
                capability_ids: record
                    .declaration
                    .capabilities
                    .iter()
                    .map(|cap| cap.capability_id.clone())
                    .collect(),
            })
            .collect()
    }

    /// Resolve the grant-binding tuple (provider identity, capability
    /// id, manifest version) to the digest of the filed declaration a
    /// grant would bind (D3 verification point 3, #5905 D6). Only
    /// admitted-or-beyond records resolve: a merely filed declaration
    /// is grantable by nobody (D6). Superseded versions still resolve —
    /// existing grants keep their binding target until expiry (I4).
    #[must_use]
    pub fn resolve_tuple(
        &self,
        provider: &ProviderIdentity,
        capability_id: &str,
        manifest_version: u64,
    ) -> Option<String> {
        self.records
            .values()
            .find(|record| {
                matches!(
                    record.state,
                    LifecycleState::Admitted
                        | LifecycleState::Live
                        | LifecycleState::Suspended
                        | LifecycleState::Superseded
                ) && record.declaration.provider == *provider
                    && record.declaration.manifest_version == manifest_version
                    && record.declaration.capability(capability_id).is_some()
            })
            .map(|record| record.digest.clone())
    }

    /// Names in the declaration view whose admission posture includes
    /// them in default ceilings — the declaration-layer counterpart of
    /// `ProviderRegistry::default_ceiling_names`, used to prove the
    /// static catalog behaves identically downstream of admission (D1).
    #[must_use]
    pub fn default_ceiling_names(&self) -> BTreeSet<String> {
        self.records
            .values()
            .filter(|record| record.state == LifecycleState::Live)
            .filter(|record| {
                record
                    .admission
                    .as_ref()
                    .is_some_and(|a| a.ceiling == DefaultCeiling::Included)
            })
            .map(|record| record.declaration.name.clone())
            .collect()
    }
}

/// The host-attested identity for a built-in provider (D1): derived
/// deterministically from the catalog name. It is a provenance marker,
/// not a keypair — host-attested records are attested by the binary
/// that projected them, and no signature is ever expected under it.
#[must_use]
pub fn host_attested_identity(name: &str) -> ProviderIdentity {
    let mut hasher = Sha256::new();
    hasher.update(HOST_ATTESTED_DOMAIN);
    hasher.update(name.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&digest);
    ProviderIdentity(bytes)
}

/// Project one compile-time catalog entry into the D2 shape (D1,
/// #8362 scope item 3): host-attested provenance, `ttl_class: static`,
/// world scope, no expiry.
///
/// The legacy entry carries no sensitivity or effect fact; the
/// projection uses the least sensitivity class (S0) and derives the
/// effect class from the provider kind (requests actuate; subscriptions,
/// events, and platform modules observe). A provider that files its own
/// signed manifest states both facts for itself.
#[must_use]
pub fn project_static(entry: &ProviderManifest) -> ProviderDeclaration {
    let effect_class = match entry.kind {
        ProviderKind::Request => EffectClass::Actuation,
        ProviderKind::Subscription | ProviderKind::Event | ProviderKind::PlatformModule => {
            EffectClass::Observation
        }
    };
    ProviderDeclaration {
        format_version: CURRENT_FORMAT_VERSION,
        provider: host_attested_identity(entry.name),
        instance: "host".to_owned(),
        name: entry.name.to_owned(),
        side: entry.side,
        kind: entry.kind,
        manifest_version: u64::from(entry.version),
        epoch: 0,
        scope: DeclarationScope::World("local".to_owned()),
        ttl_class: TtlClass::Static,
        issued_at: 0,
        expires_at: None,
        capabilities: vec![CapabilityEntry {
            capability_id: entry.name.to_owned(),
            contract_version: ContractVersion {
                major: entry.version,
                minor: 0,
            },
            argument_schema: render_argument_schema(&entry.argument_schema),
            bounds: entry.bounds.iter().map(DeclaredBound::from).collect(),
            required_scope: entry.required_scope,
            sensitivity_class: SensitivityClass::S0,
            effect_class,
            default_ceiling: entry.default_ceiling,
            store_and_forward: false,
            deprecated: false,
        }],
        extensions: Vec::new(),
    }
}

/// Render a static argument schema in the WIT-shaped typed form D2
/// requires of declaration artefacts.
fn render_argument_schema(schema: &ArgumentSchema) -> String {
    match schema {
        ArgumentSchema::JsonObject(fields) => {
            let rendered: Vec<String> = fields
                .iter()
                .map(|field| {
                    let type_name = match field.value_type {
                        ArgumentType::String => "string",
                        ArgumentType::UnsignedInteger => "u64",
                        ArgumentType::Object => "object",
                    };
                    if field.required {
                        format!("{}: {}", field.name, type_name)
                    } else {
                        format!("{}: option<{}>", field.name, type_name)
                    }
                })
                .collect();
            format!("record {{ {} }}", rendered.join(", "))
        }
        ArgumentSchema::Subscription(spec) => format!("subscription({spec})"),
        ArgumentSchema::NotApplicable => "module".to_owned(),
    }
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::world_caps::PROVIDER_REGISTRY;

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn declaration(provider: &SigningKey, name: &str, version: u64) -> ProviderDeclaration {
        ProviderDeclaration {
            format_version: CURRENT_FORMAT_VERSION,
            provider: ProviderIdentity::from_verifying_key(&provider.verifying_key()),
            instance: "main".to_owned(),
            name: name.to_owned(),
            side: ProviderSide::Host,
            kind: ProviderKind::Request,
            manifest_version: version,
            epoch: 1,
            scope: DeclarationScope::World("world-1".to_owned()),
            ttl_class: TtlClass::Heartbeat(300),
            issued_at: 100,
            expires_at: Some(10_000),
            capabilities: vec![CapabilityEntry {
                capability_id: "echo".to_owned(),
                contract_version: ContractVersion { major: 1, minor: 0 },
                argument_schema: "record { text: string }".to_owned(),
                bounds: vec![DeclaredBound::ArgsBytes(4096)],
                required_scope: RequiredScope::Chat,
                sensitivity_class: SensitivityClass::S1,
                effect_class: EffectClass::Actuation,
                default_ceiling: DefaultCeiling::Excluded,
                store_and_forward: false,
                deprecated: false,
            }],
            extensions: Vec::new(),
        }
    }

    /// Oracle: #8362 acceptance 1 — a runtime-filed, signature-verified
    /// manifest is admitted, discovered, and grant-bindable by its
    /// (provider identity, capability id, manifest version) tuple.
    #[test]
    fn filed_signed_manifest_is_admitted_discovered_and_grant_bindable() {
        let provider = key(7);
        let signed = SignedManifest::sign(declaration(&provider, "echo.provider", 1), &provider);
        let identity = signed.declaration.provider;
        let mut registry = DeclarationRegistry::new();
        let digest = registry.file_signed(&signed, 200).expect("filing succeeds");
        // Filed is discoverable by nobody and grantable by nobody (D6).
        assert!(registry.declaration_view().is_empty());
        assert!(registry.resolve_tuple(&identity, "echo", 1).is_none());
        let admission = registry
            .admit(&digest, "registry-1", 210)
            .expect("admission");
        assert_eq!(admission.manifest_digest, digest);
        assert_eq!(admission.ceiling, DefaultCeiling::Excluded);
        registry.mark_live(&digest).expect("goes live");
        let view = registry.declaration_view();
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].digest, digest);
        assert_eq!(view[0].capability_ids, vec!["echo".to_owned()]);
        assert_eq!(
            registry.resolve_tuple(&identity, "echo", 1).as_deref(),
            Some(digest.as_str())
        );
        assert!(registry.resolve_tuple(&identity, "echo", 2).is_none());
        assert!(registry.resolve_tuple(&identity, "missing", 1).is_none());
    }

    /// Oracle: #8362 acceptance 2 — the static catalog behaves
    /// identically downstream of admission (D1 degenerate case): same
    /// names in the declaration view, same default-ceiling set.
    #[test]
    fn static_catalog_behaves_identically_downstream_of_admission() {
        let mut registry = DeclarationRegistry::new();
        let digests = registry.load_static_catalog(&PROVIDER_REGISTRY);
        assert_eq!(digests.len(), PROVIDER_REGISTRY.entries().len());
        let view_names: BTreeSet<String> = registry
            .declaration_view()
            .into_iter()
            .map(|card| card.name)
            .collect();
        assert_eq!(view_names, PROVIDER_REGISTRY.catalog_names());
        assert_eq!(
            registry.default_ceiling_names(),
            PROVIDER_REGISTRY.default_ceiling_names()
        );
        // Every projected record is host-attested, static, and cites a
        // digest a grant could bind.
        for digest in &digests {
            let record = registry.fetch_by_digest(digest).expect("projected record");
            assert_eq!(record.provenance, Provenance::HostAttested);
            assert_eq!(record.declaration.ttl_class, TtlClass::Static);
            assert_eq!(record.state, LifecycleState::Live);
        }
    }

    /// Oracle: #8362 acceptance 3 — rollback of `manifest_version` or
    /// `epoch` is refused with a stable code.
    #[test]
    fn manifest_version_and_epoch_rollback_refused_with_stable_codes() {
        let provider = key(9);
        let mut registry = DeclarationRegistry::new();
        let v2 = SignedManifest::sign(declaration(&provider, "rollback.provider", 2), &provider);
        registry.file_signed(&v2, 200).expect("v2 files");
        let v1 = SignedManifest::sign(declaration(&provider, "rollback.provider", 1), &provider);
        assert_eq!(
            registry.file_signed(&v1, 300),
            Err(DeclarationRefusal::RollbackVersion)
        );
        assert_eq!(
            DeclarationRefusal::RollbackVersion.code(),
            "declaration.rollback_version"
        );
        // Same version re-filed (any content change is a new version,
        // D4) is also a rollback refusal, never an in-place edit (I3).
        let v2_again =
            SignedManifest::sign(declaration(&provider, "rollback.provider", 2), &provider);
        assert_eq!(
            registry.file_signed(&v2_again, 300),
            Err(DeclarationRefusal::RollbackVersion)
        );
        let mut older_epoch = declaration(&provider, "rollback.provider", 3);
        older_epoch.epoch = 0;
        let old_epoch = SignedManifest::sign(older_epoch, &provider);
        assert_eq!(
            registry.file_signed(&old_epoch, 300),
            Err(DeclarationRefusal::RollbackEpoch)
        );
        assert_eq!(
            DeclarationRefusal::RollbackEpoch.code(),
            "declaration.rollback_epoch"
        );
    }

    /// Oracle: I2 — an unsigned or badly signed manifest is never
    /// filed, discovered, negotiated against, or granted against.
    #[test]
    fn bad_signature_is_refused_and_never_filed() {
        let provider = key(3);
        let other = key(4);
        // Signed by a different key than the declared identity.
        let forged = SignedManifest::sign(declaration(&provider, "forged.provider", 1), &other);
        let mut registry = DeclarationRegistry::new();
        assert_eq!(
            registry.file_signed(&forged, 200),
            Err(DeclarationRefusal::BadSignature)
        );
        assert!(registry.declaration_view().is_empty());
        // A tampered field after signing also fails verification.
        let mut tampered =
            SignedManifest::sign(declaration(&provider, "forged.provider", 1), &provider);
        tampered.declaration.capabilities[0].bounds = vec![DeclaredBound::ArgsBytes(1 << 30)];
        assert_eq!(
            registry.file_signed(&tampered, 200),
            Err(DeclarationRefusal::BadSignature)
        );
    }

    /// Oracle: D2/I10 — a familiar name under a different key is a
    /// different provider; the collision is a refusal, never a rebind.
    #[test]
    fn name_collision_under_different_key_is_refused() {
        let first = key(11);
        let second = key(12);
        let mut registry = DeclarationRegistry::new();
        let signed = SignedManifest::sign(declaration(&first, "shared.name", 1), &first);
        registry.file_signed(&signed, 200).expect("first files");
        let impostor = SignedManifest::sign(declaration(&second, "shared.name", 1), &second);
        assert_eq!(
            registry.file_signed(&impostor, 200),
            Err(DeclarationRefusal::NameCollision)
        );
    }

    /// Oracle: D2 extensions — an unknown required extension is a
    /// refusal; an unknown optional one is ignored; a prohibited
    /// namespace is refused as prohibited content.
    #[test]
    fn extension_rules_follow_d2() {
        let provider = key(13);
        let mut required = declaration(&provider, "ext.provider", 1);
        required.extensions = vec![ManifestExtension {
            namespace: "vendor.unknown".to_owned(),
            required: true,
            value: "x".to_owned(),
        }];
        let mut registry = DeclarationRegistry::new();
        assert_eq!(
            registry.file_signed(&SignedManifest::sign(required, &provider), 200),
            Err(DeclarationRefusal::UnknownRequiredExtension)
        );
        let mut optional = declaration(&provider, "ext.provider", 1);
        optional.extensions = vec![ManifestExtension {
            namespace: "vendor.unknown".to_owned(),
            required: false,
            value: "x".to_owned(),
        }];
        assert!(
            registry
                .file_signed(&SignedManifest::sign(optional, &provider), 200)
                .is_ok()
        );
        let mut smuggled = declaration(&provider, "ext.provider", 2);
        smuggled.extensions = vec![ManifestExtension {
            namespace: "secret.store".to_owned(),
            required: false,
            value: "x".to_owned(),
        }];
        assert_eq!(
            registry.file_signed(&SignedManifest::sign(smuggled, &provider), 200),
            Err(DeclarationRefusal::ProhibitedContent)
        );
    }

    /// Oracle: D5/I4 — a new live version supersedes the old one
    /// atomically in the view, while the superseded version stays
    /// fetchable and tuple-resolvable for grants already bound to it.
    #[test]
    fn superseded_version_stays_fetchable_and_resolvable() {
        let provider = key(15);
        let identity = ProviderIdentity::from_verifying_key(&provider.verifying_key());
        let mut registry = DeclarationRegistry::new();
        let v1 = SignedManifest::sign(declaration(&provider, "versioned.provider", 1), &provider);
        let d1 = registry.file_signed(&v1, 200).expect("v1 files");
        registry.admit(&d1, "registry-1", 200).expect("v1 admitted");
        registry.mark_live(&d1).expect("v1 live");
        let v2 = SignedManifest::sign(declaration(&provider, "versioned.provider", 2), &provider);
        let d2 = registry.file_signed(&v2, 300).expect("v2 files");
        registry.admit(&d2, "registry-1", 300).expect("v2 admitted");
        registry.mark_live(&d2).expect("v2 live");
        let view = registry.declaration_view();
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].digest, d2);
        let old = registry.fetch_by_digest(&d1).expect("v1 still fetchable");
        assert_eq!(old.state, LifecycleState::Superseded);
        assert_eq!(
            registry.resolve_tuple(&identity, "echo", 1).as_deref(),
            Some(d1.as_str())
        );
        assert_eq!(
            registry.resolve_tuple(&identity, "echo", 2).as_deref(),
            Some(d2.as_str())
        );
    }

    /// Oracle: D6 — withdrawal is a signed provider act; a forged
    /// withdrawal does nothing, a genuine one tombstones.
    #[test]
    fn withdrawal_requires_provider_signature_and_tombstones() {
        let provider = key(17);
        let other = key(18);
        let identity = ProviderIdentity::from_verifying_key(&provider.verifying_key());
        let mut registry = DeclarationRegistry::new();
        let signed = SignedManifest::sign(declaration(&provider, "leaving.provider", 1), &provider);
        let digest = registry.file_signed(&signed, 200).expect("files");
        registry
            .admit(&digest, "registry-1", 200)
            .expect("admitted");
        registry.mark_live(&digest).expect("live");
        let forged = SignedWithdrawal::sign(identity, "main".to_owned(), 1, 1, &other);
        assert_eq!(
            registry.withdraw(&forged),
            Err(DeclarationRefusal::BadSignature)
        );
        assert_eq!(registry.declaration_view().len(), 1);
        let genuine = SignedWithdrawal::sign(identity, "main".to_owned(), 1, 1, &provider);
        registry.withdraw(&genuine).expect("withdrawal applies");
        assert!(registry.declaration_view().is_empty());
        assert_eq!(
            registry
                .fetch_by_digest(&digest)
                .expect("record kept")
                .state,
            LifecycleState::Tombstoned
        );
    }

    /// Oracle: D6/#5904 §5 — expiry is absolute: an expired live
    /// declaration goes stale on tick and is tombstoned by the sweep;
    /// an already-expired manifest is refused at filing.
    #[test]
    fn expiry_is_absolute_and_terminal() {
        let provider = key(19);
        let mut registry = DeclarationRegistry::new();
        let signed = SignedManifest::sign(declaration(&provider, "brief.provider", 1), &provider);
        let digest = registry.file_signed(&signed, 200).expect("files");
        registry
            .admit(&digest, "registry-1", 200)
            .expect("admitted");
        registry.mark_live(&digest).expect("live");
        registry.tick(9_999);
        assert_eq!(registry.declaration_view().len(), 1);
        registry.tick(10_000);
        assert!(registry.declaration_view().is_empty());
        assert_eq!(
            registry
                .fetch_by_digest(&digest)
                .expect("record kept")
                .state,
            LifecycleState::Stale
        );
        registry.sweep_tombstones();
        assert_eq!(
            registry
                .fetch_by_digest(&digest)
                .expect("record kept")
                .state,
            LifecycleState::Tombstoned
        );
        let late = SignedManifest::sign(declaration(&provider, "brief.provider", 2), &provider);
        assert_eq!(
            registry.file_signed(&late, 10_001),
            Err(DeclarationRefusal::ExpiredManifest)
        );
    }

    /// Oracle: scope item 1 — the canonical encoding is deterministic,
    /// so the digest is a stable content address (D3) and the same
    /// declaration signed twice produces identical bytes.
    #[test]
    fn canonical_bytes_are_deterministic_and_digest_stable() {
        let provider = key(21);
        let a = declaration(&provider, "stable.provider", 1);
        let b = declaration(&provider, "stable.provider", 1);
        assert_eq!(a.canonical_bytes(), b.canonical_bytes());
        assert_eq!(a.digest(), b.digest());
        let mut changed = declaration(&provider, "stable.provider", 1);
        changed.manifest_version = 2;
        assert_ne!(a.digest(), changed.digest());
    }

    /// Oracle: D4 axis 1 — an unknown format version is refused, never
    /// best-effort parsed.
    #[test]
    fn unknown_format_version_is_refused() {
        let provider = key(23);
        let mut future = declaration(&provider, "future.provider", 1);
        future.format_version = CURRENT_FORMAT_VERSION + 1;
        let signed = SignedManifest::sign(future, &provider);
        let mut registry = DeclarationRegistry::new();
        assert_eq!(
            registry.file_signed(&signed, 200),
            Err(DeclarationRefusal::UnknownFormatVersion)
        );
    }

    /// Oracle: D5 — suspension is registry state, not a contents edit:
    /// suspend and resume move the same digest in and out of the view.
    #[test]
    fn suspension_is_state_not_contents() {
        let provider = key(25);
        let mut registry = DeclarationRegistry::new();
        let signed =
            SignedManifest::sign(declaration(&provider, "pausable.provider", 1), &provider);
        let digest = registry.file_signed(&signed, 200).expect("files");
        registry
            .admit(&digest, "registry-1", 200)
            .expect("admitted");
        registry.mark_live(&digest).expect("live");
        registry.suspend(&digest).expect("suspends");
        assert!(registry.declaration_view().is_empty());
        registry.resume(&digest).expect("resumes");
        let view = registry.declaration_view();
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].digest, digest);
    }
}
