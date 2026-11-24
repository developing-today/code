//! Universal signed-and-validated admission gate for both cores
//! (slice A9120-signature-gate, issue #10275; design part A,
//! cross-cutting invariants 1, 6 and 7 — statements S1.12–S1.20,
//! S3.4–S3.10, S4.19–S4.28).
//!
//! Every admission path — a remote producer's canonical bytes or a
//! local producer's already-constructed envelope — passes this one
//! gate before any state transition is accepted. Local producers
//! sign before entering; there is no unsigned-local exception
//! (invariant 1). The gate runs, in order:
//!
//! 1. **Semantic validation** — the body must encode under the
//!    canonical profile (labels bounded, strict field set) and the
//!    envelope must survive a canonical decode/encode round trip, so
//!    a hand-constructed value gets exactly the checks wire bytes
//!    get (invariant 7: malformed encodings and unknown critical
//!    fields reject).
//! 2. **Signature and payload verification** — the A9114 envelope
//!    verifies its detached signature against the body's key and the
//!    payload against the committed payload ID and length (the
//!    A9120 content core's digest-before-commit rule, recomputed on
//!    receipt per invariant 2).
//! 3. **Signer binding** — the body's `signer_id` must resolve in
//!    the signer directory to exactly the body's key, and the
//!    authenticated principal must be that signer.
//! 4. **Namespace/tenant authorization** — the injected
//!    [`AdmissionAuthorizer`] seam (the A9116-registry-state pattern)
//!    judges the authenticated context against the body's requested
//!    scope; the shipped [`ScopeBindingAuthorizer`] requires the
//!    body's context to equal the authenticated tenant scope
//!    (invariant 6). This slice consumes the #9083 authenticated
//!    tenant/scope context contract; it does not define tenancy.
//! 5. **Policy** — the body's domain must be admitted by the gate
//!    policy and the payload must fit the policy size budget.
//!
//! The only artifact of acceptance is an [`AdmissionReceipt`]. Its
//! fields are private and it has no constructor outside this
//! module: holding a receipt is proof the gate ran every check —
//! no route bypass exists.
//!
//! The crate-root export of this module belongs to slice 9110-02;
//! the model test includes it by path, per the sibling-slice
//! pattern.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::hash_profile::{DEFAULT_MAX_CONTENT_BYTES, ProfileDispatch};
use crate::id_shape::{HashProfile, IdShapeError};
use crate::signed_envelope::{Body, Envelope, EnvelopeError, EnvelopeId, PayloadId};

/// Longest admitted context field, in bytes. Mirrors the
/// A9116 registry-state context bound.
pub const MAX_CONTEXT_BYTES: usize = 255;

/// The authenticated tenant/scope context an admission is judged
/// against (the #9083 contract shape, mirroring A9116's
/// `MutationContext` field for field). Authentication itself
/// happens outside the gate; the gate consumes its result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionContext {
    principal: String,
    tenant_scope: String,
    authorization_scope: String,
}

fn checked_context(field: &'static str, text: &str) -> Result<(), AdmissionRejection> {
    if !text.is_empty()
        && text.len() <= MAX_CONTEXT_BYTES
        && text.bytes().all(|byte| byte.is_ascii_graphic())
    {
        Ok(())
    } else {
        Err(AdmissionRejection::InvalidContext {
            field,
            problem: "is not 1 to 255 visible ASCII characters",
        })
    }
}

impl AdmissionContext {
    /// A validated context.
    ///
    /// # Errors
    ///
    /// [`AdmissionRejection::InvalidContext`] when any field is
    /// empty, over [`MAX_CONTEXT_BYTES`], or not visible ASCII.
    pub fn new(
        principal: impl Into<String>,
        tenant_scope: impl Into<String>,
        authorization_scope: impl Into<String>,
    ) -> Result<Self, AdmissionRejection> {
        let principal = principal.into();
        let tenant_scope = tenant_scope.into();
        let authorization_scope = authorization_scope.into();
        checked_context("principal", &principal)?;
        checked_context("tenant_scope", &tenant_scope)?;
        checked_context("authorization_scope", &authorization_scope)?;
        Ok(Self {
            principal,
            tenant_scope,
            authorization_scope,
        })
    }

    /// The authenticated principal (the signer identity admissions
    /// are bound to).
    #[must_use]
    pub fn principal(&self) -> &str {
        &self.principal
    }

    /// The tenant scope the context is authenticated for.
    #[must_use]
    pub fn tenant_scope(&self) -> &str {
        &self.tenant_scope
    }

    /// The authorization scope the context presents (for example
    /// `admission:write`).
    #[must_use]
    pub fn authorization_scope(&self) -> &str {
        &self.authorization_scope
    }
}

/// Gate policy: which signing domains are admitted and how large a
/// payload may be. Mechanism, not behaviour — a deployment narrows
/// it, never widens it past the hard budget compiled into the
/// profile dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatePolicy {
    admitted_domains: BTreeSet<String>,
    max_payload_bytes: u64,
}

impl Default for GatePolicy {
    /// Admits every domain; payload budget is the profile
    /// dispatch's default content budget.
    fn default() -> Self {
        Self {
            admitted_domains: BTreeSet::new(),
            max_payload_bytes: DEFAULT_MAX_CONTENT_BYTES,
        }
    }
}

impl GatePolicy {
    /// A policy admitting only `domains` (an empty set admits
    /// every domain) with the default payload budget.
    #[must_use]
    pub fn admitting(domains: impl IntoIterator<Item = String>) -> Self {
        Self {
            admitted_domains: domains.into_iter().collect(),
            max_payload_bytes: DEFAULT_MAX_CONTENT_BYTES,
        }
    }

    /// Narrows the payload budget.
    #[must_use]
    pub const fn with_max_payload_bytes(mut self, max_payload_bytes: u64) -> Self {
        self.max_payload_bytes = max_payload_bytes;
        self
    }

    /// Whether `domain` is admitted (every domain is, when the
    /// admitted set is empty).
    #[must_use]
    pub fn admits_domain(&self, domain: &str) -> bool {
        self.admitted_domains.is_empty() || self.admitted_domains.contains(domain)
    }

    /// The payload size budget in bytes.
    #[must_use]
    pub const fn max_payload_bytes(&self) -> u64 {
        self.max_payload_bytes
    }
}

/// Signer binding source: resolves a signer identity to the key
/// registered for it. The gate refuses any signer the directory
/// does not know, and any body whose key is not the registered one.
pub trait SignerDirectory: Send + Sync {
    /// The registered Ed25519 public key for `signer_id`, if any.
    fn key_for(&self, signer_id: &str) -> Option<[u8; 32]>;
}

/// A [`SignerDirectory`] over a fixed registration table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StaticSignerDirectory {
    keys: BTreeMap<String, [u8; 32]>,
}

impl StaticSignerDirectory {
    /// An empty directory.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `signer_id` with its public key.
    pub fn register(&mut self, signer_id: impl Into<String>, key: [u8; 32]) {
        self.keys.insert(signer_id.into(), key);
    }

    /// Whether `signer_id` is registered.
    #[must_use]
    pub fn contains(&self, signer_id: &str) -> bool {
        self.keys.contains_key(signer_id)
    }
}

impl SignerDirectory for StaticSignerDirectory {
    fn key_for(&self, signer_id: &str) -> Option<[u8; 32]> {
        self.keys.get(signer_id).copied()
    }
}

/// Namespace/tenant authorization seam. Implementations judge the
/// authenticated context against the body's requested scope and
/// refuse with [`AdmissionRejection::Unauthorized`].
pub trait AdmissionAuthorizer: Send + Sync {
    /// Authorizes, or refuses, this admission.
    ///
    /// # Errors
    ///
    /// [`AdmissionRejection::Unauthorized`] when the context does
    /// not hold authority over the body's scope.
    fn authorize(&self, context: &AdmissionContext, body: &Body) -> Result<(), AdmissionRejection>;
}

/// The shipped authorizer: the body's context (its requested
/// namespace/tenant scope) must equal the authenticated tenant
/// scope exactly (invariant 6 — requested tenant/scope must match
/// authenticated context).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScopeBindingAuthorizer;

impl AdmissionAuthorizer for ScopeBindingAuthorizer {
    fn authorize(&self, context: &AdmissionContext, body: &Body) -> Result<(), AdmissionRejection> {
        if body.context == context.tenant_scope() {
            Ok(())
        } else {
            Err(AdmissionRejection::Unauthorized {
                problem: format!(
                    "body context {:?} is not the authenticated tenant scope {:?}",
                    body.context,
                    context.tenant_scope()
                ),
            })
        }
    }
}

/// Why an admission was refused. Each variant is one gate stage;
/// [`AdmissionRejection::class`] is the stable failure class the
/// model oracle compares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmissionRejection {
    /// A context field failed validation at construction.
    InvalidContext {
        /// The field name.
        field: &'static str,
        /// What was wrong.
        problem: &'static str,
    },
    /// The envelope is not a well-formed canonical record.
    Malformed(EnvelopeError),
    /// The version, suite or content profile is not one this gate
    /// verifies.
    Unsupported(EnvelopeError),
    /// The payload is not what the body commits (length or digest).
    Payload(EnvelopeError),
    /// The signature does not verify against the body's key.
    BadSignature,
    /// The signer is unknown, its registered key differs from the
    /// body's key, or the authenticated principal is not the signer.
    SignerBinding {
        /// The body's signer identity.
        signer_id: String,
        /// What was wrong.
        problem: &'static str,
    },
    /// The authenticated context does not hold authority over the
    /// body's requested scope.
    Unauthorized {
        /// What was refused, with the scopes involved.
        problem: String,
    },
    /// The gate policy refuses this domain or payload size.
    Policy {
        /// What the policy refused.
        problem: String,
    },
}

impl AdmissionRejection {
    /// A stable failure class: `malformed`, `unsupported`,
    /// `payload`, `bad_signature`, `signer_binding`,
    /// `unauthorized` or `policy`.
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::InvalidContext { .. } | Self::Malformed(_) => "malformed",
            Self::Unsupported(_) => "unsupported",
            Self::Payload(_) => "payload",
            Self::BadSignature => "bad_signature",
            Self::SignerBinding { .. } => "signer_binding",
            Self::Unauthorized { .. } => "unauthorized",
            Self::Policy { .. } => "policy",
        }
    }
}

impl fmt::Display for AdmissionRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidContext { field, problem } => {
                write!(f, "context field {field}: {problem}")
            }
            Self::Malformed(error) => write!(f, "malformed envelope: {error}"),
            Self::Unsupported(error) => write!(f, "unsupported envelope: {error}"),
            Self::Payload(error) => write!(f, "payload rejected: {error}"),
            Self::BadSignature => write!(f, "signature does not verify"),
            Self::SignerBinding { signer_id, problem } => {
                write!(f, "signer {signer_id:?}: {problem}")
            }
            Self::Unauthorized { problem } => write!(f, "unauthorized: {problem}"),
            Self::Policy { problem } => write!(f, "policy refusal: {problem}"),
        }
    }
}

impl std::error::Error for AdmissionRejection {}

/// Maps an envelope verification failure onto the gate stage that
/// owns it.
fn map_verify_error(error: EnvelopeError, signer_id: &str) -> AdmissionRejection {
    match error {
        EnvelopeError::BadSignature => AdmissionRejection::BadSignature,
        EnvelopeError::BadKey | EnvelopeError::KeyMismatch => AdmissionRejection::SignerBinding {
            signer_id: signer_id.to_owned(),
            problem: "the body key is not a valid Ed25519 public key",
        },
        EnvelopeError::UnsupportedVersion(_) | EnvelopeError::UnsupportedSuite(_) => {
            AdmissionRejection::Unsupported(error)
        }
        EnvelopeError::Id(IdShapeError::UnsupportedProfile { .. }) => {
            AdmissionRejection::Unsupported(error)
        }
        EnvelopeError::PayloadLength { .. } => AdmissionRejection::Payload(error),
        EnvelopeError::Id(IdShapeError::DigestMismatch)
        | EnvelopeError::Id(IdShapeError::InvalidProfileParameters { .. })
        | EnvelopeError::Id(IdShapeError::ResourceLimit { .. }) => {
            AdmissionRejection::Payload(error)
        }
        other => AdmissionRejection::Malformed(other),
    }
}

/// Proof of admission. Minted only by [`AdmissionGate::admit`] /
/// [`AdmissionGate::admit_canonical`] after every check passed;
/// the fields are private and there is no other constructor, so a
/// receipt cannot be forged by a route that skipped the gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionReceipt {
    envelope_id: EnvelopeId,
    payload_id: PayloadId,
    signer_id: String,
    tenant_scope: String,
    payload_length: u64,
}

impl AdmissionReceipt {
    /// The admitted envelope's identity.
    #[must_use]
    pub const fn envelope_id(&self) -> &EnvelopeId {
        &self.envelope_id
    }

    /// The admitted payload's identity.
    #[must_use]
    pub const fn payload_id(&self) -> &PayloadId {
        &self.payload_id
    }

    /// The signer the admission is bound to.
    #[must_use]
    pub fn signer_id(&self) -> &str {
        &self.signer_id
    }

    /// The tenant scope the admission was authorized under (the
    /// body's context, checked against the authenticated context).
    #[must_use]
    pub fn tenant_scope(&self) -> &str {
        &self.tenant_scope
    }

    /// The admitted payload length in bytes.
    #[must_use]
    pub const fn payload_length(&self) -> u64 {
        self.payload_length
    }
}

/// The admission gate. One gate, every path: [`Self::admit`] for
/// constructed envelopes (local producers) and
/// [`Self::admit_canonical`] for wire bytes run the identical
/// pipeline.
pub struct AdmissionGate {
    policy: GatePolicy,
    directory: Box<dyn SignerDirectory>,
    authorizer: Box<dyn AdmissionAuthorizer>,
    dispatch: ProfileDispatch,
}

impl AdmissionGate {
    /// A gate with the default profile dispatch (every compiled
    /// profile admitted, default content budget).
    #[must_use]
    pub fn new(
        policy: GatePolicy,
        directory: impl SignerDirectory + 'static,
        authorizer: impl AdmissionAuthorizer + 'static,
    ) -> Self {
        Self {
            policy,
            directory: Box::new(directory),
            authorizer: Box::new(authorizer),
            dispatch: ProfileDispatch::default(),
        }
    }

    /// A gate with an explicit profile dispatch.
    #[must_use]
    pub fn with_dispatch(
        policy: GatePolicy,
        directory: impl SignerDirectory + 'static,
        authorizer: impl AdmissionAuthorizer + 'static,
        dispatch: ProfileDispatch,
    ) -> Self {
        Self {
            policy,
            directory: Box::new(directory),
            authorizer: Box::new(authorizer),
            dispatch,
        }
    }

    /// Admits wire bytes: canonical decode, then the full
    /// pipeline. This is the remote-producer path; it shares every
    /// check with the local path.
    ///
    /// # Errors
    ///
    /// [`AdmissionRejection::Malformed`] for undecodable bytes,
    /// then as [`Self::admit`].
    pub fn admit_canonical(
        &self,
        bytes: &[u8],
        context: &AdmissionContext,
    ) -> Result<AdmissionReceipt, AdmissionRejection> {
        let envelope = Envelope::decode(bytes).map_err(AdmissionRejection::Malformed)?;
        self.admit(&envelope, context)
    }

    /// Runs the full admission pipeline over a constructed
    /// envelope (the local-producer path — the producer signed
    /// before entering, and the gate re-checks everything wire
    /// bytes would face).
    ///
    /// # Errors
    ///
    /// The first failing stage's [`AdmissionRejection`], in
    /// pipeline order: semantic validation, signature/payload
    /// verification, signer binding, authorization, policy.
    pub fn admit(
        &self,
        envelope: &Envelope,
        context: &AdmissionContext,
    ) -> Result<AdmissionReceipt, AdmissionRejection> {
        let body = &envelope.body;

        // Stage 1 — semantic validation. `to_value` enforces the
        // label bounds decode enforces; the canonical round trip
        // then proves the constructed value is byte-identical to
        // what the wire path would decode, so no field can smuggle
        // past the strict field set.
        body.to_value().map_err(AdmissionRejection::Malformed)?;
        let canonical = envelope
            .canonical_bytes()
            .map_err(AdmissionRejection::Malformed)?;
        let decoded = Envelope::decode(&canonical).map_err(AdmissionRejection::Malformed)?;
        if decoded != *envelope {
            return Err(AdmissionRejection::Malformed(EnvelopeError::Field {
                field: "envelope",
                problem: "is not in canonical form",
            }));
        }

        // Stage 2 — signature and payload verification (digest
        // recomputed on receipt; the signature covers the exact
        // canonical body).
        envelope
            .verify(&self.dispatch)
            .map_err(|error| map_verify_error(error, &body.signer_id))?;

        // Stage 3 — signer binding: the signer must be registered
        // with exactly this key, and the authenticated principal
        // must be the signer.
        match self.directory.key_for(&body.signer_id) {
            None => {
                return Err(AdmissionRejection::SignerBinding {
                    signer_id: body.signer_id.clone(),
                    problem: "signer is not registered in the directory",
                });
            }
            Some(registered) if registered != body.key => {
                return Err(AdmissionRejection::SignerBinding {
                    signer_id: body.signer_id.clone(),
                    problem: "registered key does not match the body key",
                });
            }
            Some(_) => {}
        }
        if context.principal() != body.signer_id {
            return Err(AdmissionRejection::SignerBinding {
                signer_id: body.signer_id.clone(),
                problem: "the authenticated principal is not the signer",
            });
        }

        // Stage 4 — namespace/tenant authorization (injected seam).
        self.authorizer.authorize(context, body)?;

        // Stage 5 — policy: admitted domain and payload budget.
        if !self.policy.admits_domain(&body.domain) {
            return Err(AdmissionRejection::Policy {
                problem: format!("domain {:?} is not admitted by policy", body.domain),
            });
        }
        if body.payload_length > self.policy.max_payload_bytes() {
            return Err(AdmissionRejection::Policy {
                problem: format!(
                    "payload of {} bytes exceeds the policy maximum of {} bytes",
                    body.payload_length,
                    self.policy.max_payload_bytes()
                ),
            });
        }

        // Every check passed: mint the receipt — the only accepted
        // state transition this slice produces.
        let envelope_id = envelope
            .id(HashProfile::RawBlake3_256)
            .map_err(AdmissionRejection::Malformed)?;
        Ok(AdmissionReceipt {
            envelope_id,
            payload_id: body.payload_id,
            signer_id: body.signer_id.clone(),
            tenant_scope: body.context.clone(),
            payload_length: body.payload_length,
        })
    }
}
