//! Shared Command/Result contract, pair identity, version,
//! effect-separated reason-code registry, auth-vs-actor distinction
//! and fixtures — DST-01 (developing-today/code#9451; design parent
//! #9200, program #9196).
//!
//! This module is the offline contract slice of the PM orchestration
//! design (`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`):
//! the shared `Command`/`Result` envelope from `architecture.md`
//! ("Shared command and result interface"), the `(authority_id,
//! work_id)` pair identity, the reason-code registry and effect
//! classes from `components/9200-priority-distribution.md`, the
//! protected owner-policy reference from the same component, and the
//! digest framing from the architecture digest/serialization
//! contract. It is pure types and validation: no I/O, no queue, no
//! state — a rejection or an unsupported code/version is a typed
//! value that mutates nothing, and callers apply effects only after
//! [`Command::validate`] and an authorization decision they made
//! from an authenticated principal.
//!
//! Auth-vs-actor, stated once and enforced by the shapes below:
//! `actor_ref` is an audit/reference field, **not** authorization.
//! A client payload carrying an `authorized` field is a contract
//! violation ([`ContractError::ClientSuppliedAuthorization`]), never
//! proof. [`evaluate_authorization`] reads only the authenticated
//! principal supplied by the auth layer; it never looks at
//! `actor_ref`.
//!
//! Versions and identities that may exceed JavaScript's safe
//! integer range cross JSON as canonical decimal strings (no leading
//! zeros, no sign, no fraction) and are never coerced through
//! floating point. Unknown reason codes are never aliased to a known
//! code; unknown major schema versions are typed
//! [`ContractError::UnsupportedVersion`], not best-effort parsed.

use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Wire schema identifier of the shared command envelope.
pub const COMMAND_SCHEMA_VERSION: &str = "fleet.command/v1";
/// Wire schema identifier of the shared result envelope.
pub const RESULT_SCHEMA_VERSION: &str = "fleet.result/v1";
/// Wire schema identifier of the digest intent DTO.
pub const INTENT_SCHEMA_VERSION: &str = "fleet.command-intent/v1";
/// Wire schema identifier of the protected owner-policy observation.
pub const OWNER_POLICY_SCHEMA_VERSION: &str = "fleet.owner-priority-policy/v1";

/// Digest domain for request fingerprints (architecture registry).
pub const DIGEST_DOMAIN_COMMAND_INTENT: &str = "fleet.command-intent/v1";
/// Digest domain for hydration manifests (architecture registry).
pub const DIGEST_DOMAIN_CONTEXT_MANIFEST: &str = "fleet.context-manifest/v1";
/// Digest domain for cross-target selection order (architecture registry).
pub const DIGEST_DOMAIN_SELECTION_ORDER: &str = "fleet.selection-order/v1";
/// The complete digest-domain registry. A domain is selected only
/// from this list, never from arbitrary client text.
pub const DIGEST_DOMAINS: [&str; 3] = [
    DIGEST_DOMAIN_COMMAND_INTENT,
    DIGEST_DOMAIN_CONTEXT_MANIFEST,
    DIGEST_DOMAIN_SELECTION_ORDER,
];

/// A typed contract failure. Every variant is non-mutating: a value
/// of this type is the whole outcome of a rejected parse, validation
/// or authorization check, and no canonical state changes hands
/// alongside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractError {
    /// The payload does not match the v1 schema shapes.
    InvalidSchema {
        /// What was wrong, in one line.
        detail: String,
    },
    /// A schema/policy major version this contract does not support.
    UnsupportedVersion {
        /// The version string that was presented.
        schema_version: String,
    },
    /// A reason code that is not in the registry. Unknown codes are
    /// never aliased to a known code.
    UnsupportedReasonCode {
        /// The code string that was presented.
        code: String,
    },
    /// The client payload carried an `authorized` field. A client
    /// authorization claim is a contract violation, not proof.
    ClientSuppliedAuthorization,
    /// The auth layer supplied no principal, or the principal lacks
    /// the required capability.
    UnauthorizedInput,
    /// The mutation touches a protected owner-policy field/action
    /// without the owner action. Wire code: `OWNER_POLICY_PROTECTED`.
    OwnerPolicyProtected,
    /// The command's owner-policy reference is stale against the
    /// current policy. Wire code: `OWNER_POLICY_VERSION_CONFLICT`.
    OwnerPolicyVersionConflict,
    /// A priority-affecting mutation arrived without an
    /// owner-policy reference; missing policy fails closed.
    MissingOwnerPolicyRef,
    /// A reason code was asked to produce a field effect its
    /// registry entry does not permit.
    EffectNotPermitted {
        /// The reason code that was asked.
        code: ReasonCode,
        /// The field-effect kind that was requested.
        effect: FieldEffectKind,
    },
    /// A digest domain outside the registry.
    UnknownDigestDomain {
        /// The domain string that was presented.
        domain: String,
    },
    /// A non-finite number reached the canonical serializer.
    NonFiniteNumber,
}

impl ContractError {
    /// The stable wire error code for this failure.
    pub fn code(&self) -> &'static str {
        match self {
            ContractError::InvalidSchema { .. } => "invalid_schema",
            ContractError::UnsupportedVersion { .. } => "unsupported_version",
            ContractError::UnsupportedReasonCode { .. } => "unsupported_reason_code",
            ContractError::ClientSuppliedAuthorization => "client_supplied_authorization",
            ContractError::UnauthorizedInput => "unauthorized_input",
            ContractError::OwnerPolicyProtected => "OWNER_POLICY_PROTECTED",
            ContractError::OwnerPolicyVersionConflict => "OWNER_POLICY_VERSION_CONFLICT",
            ContractError::MissingOwnerPolicyRef => "owner_policy_ref_missing",
            ContractError::EffectNotPermitted { .. } => "effect_not_permitted",
            ContractError::UnknownDigestDomain { .. } => "unknown_digest_domain",
            ContractError::NonFiniteNumber => "invalid_schema",
        }
    }

    /// Contract errors never mutate canonical state: the check that
    /// produced the error is the whole transaction.
    pub fn mutates_canonical_state(&self) -> bool {
        false
    }
}

impl fmt::Display for ContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContractError::InvalidSchema { detail } => {
                write!(f, "invalid_schema: {detail}")
            }
            ContractError::UnsupportedVersion { schema_version } => {
                write!(f, "unsupported_version: {schema_version}")
            }
            ContractError::UnsupportedReasonCode { code } => {
                write!(f, "unsupported_reason_code: {code}")
            }
            ContractError::ClientSuppliedAuthorization => {
                write!(f, "client payload carried an `authorized` field")
            }
            ContractError::UnauthorizedInput => {
                write!(
                    f,
                    "unauthorized_input: no authenticated principal/capability"
                )
            }
            ContractError::OwnerPolicyProtected => write!(f, "OWNER_POLICY_PROTECTED"),
            ContractError::OwnerPolicyVersionConflict => {
                write!(f, "OWNER_POLICY_VERSION_CONFLICT")
            }
            ContractError::MissingOwnerPolicyRef => {
                write!(
                    f,
                    "owner_policy_ref_missing: priority mutation without a policy ref"
                )
            }
            ContractError::EffectNotPermitted { code, effect } => {
                write!(
                    f,
                    "effect_not_permitted: {} may not produce a {effect:?} field effect",
                    code.as_str()
                )
            }
            ContractError::UnknownDigestDomain { domain } => {
                write!(f, "unknown_digest_domain: {domain}")
            }
            ContractError::NonFiniteNumber => {
                write!(f, "invalid_schema: non-finite number")
            }
        }
    }
}

impl std::error::Error for ContractError {}

/// Whether `value` is a canonical decimal string: non-empty, ASCII
/// digits only, and no leading zero unless the value is exactly
/// `"0"`. Canonical strings are how integer IDs, versions,
/// generations and rank keys cross JSON without floating-point
/// coercion.
pub fn is_canonical_decimal(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|b| b.is_ascii_digit())
        && (value.len() == 1 || !value.starts_with('0'))
}

/// Compare two canonical decimal strings by integer value:
/// digit count first, then lexical digits. This is the
/// arbitrary-precision comparison that preserves `9 < 10` despite
/// the string wire encoding. Both inputs must be canonical.
pub fn cmp_canonical_decimal(a: &str, b: &str) -> Ordering {
    debug_assert!(is_canonical_decimal(a) && is_canonical_decimal(b));
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

fn require_canonical_decimal(field: &'static str, value: &str) -> Result<(), ContractError> {
    if is_canonical_decimal(value) {
        Ok(())
    } else {
        Err(ContractError::InvalidSchema {
            detail: format!("{field} is not a canonical decimal string: {value:?}"),
        })
    }
}

/// Canonical work identity: the string pair `(authority_id,
/// work_id)`. `fleet://work/842` is only local shorthand when the
/// authority context is unambiguous; the qualified form is
/// `fleet://<authority_id>/work/<work_id>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkRef {
    /// The issuing authority's identifier; compared bytewise.
    pub authority_id: String,
    /// Canonical unsigned decimal string; compared by integer value.
    pub work_id: String,
}

impl WorkRef {
    /// Build a work reference, validating the pair identity.
    pub fn new(
        authority_id: impl Into<String>,
        work_id: impl Into<String>,
    ) -> Result<Self, ContractError> {
        let work_ref = WorkRef {
            authority_id: authority_id.into(),
            work_id: work_id.into(),
        };
        work_ref.validate()?;
        Ok(work_ref)
    }

    /// Validate the pair identity's field shapes.
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.authority_id.is_empty() {
            return Err(ContractError::InvalidSchema {
                detail: "authority_id must not be empty".to_string(),
            });
        }
        require_canonical_decimal("work_id", &self.work_id)
    }

    /// The authority-qualified display URI for this reference.
    pub fn qualified_uri(&self) -> String {
        format!("fleet://{}/work/{}", self.authority_id, self.work_id)
    }
}

impl Ord for WorkRef {
    fn cmp(&self, other: &Self) -> Ordering {
        self.authority_id
            .cmp(&other.authority_id)
            .then_with(|| cmp_canonical_decimal(&self.work_id, &other.work_id))
    }
}

impl PartialOrd for WorkRef {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The headline effect class a reason code is permitted to produce.
/// Results separate canonical base mutation from derived effective
/// pressure, advisory hint, and rejection/conflict (no mutation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EffectClass {
    /// The code may accept a canonical base-field change.
    BaseMutation,
    /// The code may change derived effective pressure only; it
    /// never rewrites an intrinsic/base value.
    DerivedPressure,
    /// The code may raise an advisory/triage hint only.
    AdvisoryHint,
    /// The code rejects, conflicts or is ignored: no mutation.
    NoMutation,
}

/// One kind of field effect a command may attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldEffectKind {
    /// A canonical base-field change.
    Base,
    /// A derived effective-pressure change.
    Derived,
    /// An advisory/triage hint.
    Hint,
}

/// The field effects a registry entry permits, per the #9200
/// registry table's "Permitted effect class" and "Must not do"
/// columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectPermissions {
    /// May accept a canonical base-field change.
    pub base: bool,
    /// May change derived effective pressure.
    pub derived: bool,
    /// May raise an advisory/triage hint.
    pub hint: bool,
}

/// The effect-separated reason-code registry (proposed V2 baseline,
/// #9200): the shared registry for #9199/#9200/#9204. Reason codes
/// explain an operation; none grants authority by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReasonCode {
    /// Authorized human/CLI exact-value request.
    ManualExactUrgency,
    /// Authorized owner ruling with evidence ref.
    BriefRuling,
    /// Authenticated band label change and source event.
    LabelBandChange,
    /// Authenticated removal of final priority label.
    LabelRemovedNeedsTriage,
    /// Conflicting labels, ambiguous ordering or untrusted input.
    LabelConflict,
    /// Hard-edge path / graph revision pressure.
    DependencyPressure,
    /// Verified deadline evidence and expiry.
    DeadlineBoost,
    /// Expiry event and opportunity reference.
    OpportunityExpired,
    /// Authorized pivot evidence.
    StrategicPivot,
    /// Review interval + last meaningful triage + evidence.
    AgingReconsideration,
    /// Versioned sweep with cohort/rationale.
    ClumpRefinement,
    /// Boundary, confidence, cooldown and candidate transition.
    HysteresisSuppressed,
    /// Target operation ref proves a prior Fleet projection.
    ProjectionEcho,
    /// Expected work/snapshot version mismatch.
    StaleSnapshot,
    /// Auth layer lacks principal/capability.
    UnauthorizedInput,
}

impl ReasonCode {
    /// Every code in the registry, in registry-table order.
    pub const ALL: [ReasonCode; 15] = [
        ReasonCode::ManualExactUrgency,
        ReasonCode::BriefRuling,
        ReasonCode::LabelBandChange,
        ReasonCode::LabelRemovedNeedsTriage,
        ReasonCode::LabelConflict,
        ReasonCode::DependencyPressure,
        ReasonCode::DeadlineBoost,
        ReasonCode::OpportunityExpired,
        ReasonCode::StrategicPivot,
        ReasonCode::AgingReconsideration,
        ReasonCode::ClumpRefinement,
        ReasonCode::HysteresisSuppressed,
        ReasonCode::ProjectionEcho,
        ReasonCode::StaleSnapshot,
        ReasonCode::UnauthorizedInput,
    ];

    /// The wire (snake_case) spelling of this code.
    pub fn as_str(&self) -> &'static str {
        match self {
            ReasonCode::ManualExactUrgency => "manual_exact_urgency",
            ReasonCode::BriefRuling => "brief_ruling",
            ReasonCode::LabelBandChange => "label_band_change",
            ReasonCode::LabelRemovedNeedsTriage => "label_removed_needs_triage",
            ReasonCode::LabelConflict => "label_conflict",
            ReasonCode::DependencyPressure => "dependency_pressure",
            ReasonCode::DeadlineBoost => "deadline_boost",
            ReasonCode::OpportunityExpired => "opportunity_expired",
            ReasonCode::StrategicPivot => "strategic_pivot",
            ReasonCode::AgingReconsideration => "aging_reconsideration",
            ReasonCode::ClumpRefinement => "clump_refinement",
            ReasonCode::HysteresisSuppressed => "hysteresis_suppressed",
            ReasonCode::ProjectionEcho => "projection_echo",
            ReasonCode::StaleSnapshot => "stale_snapshot",
            ReasonCode::UnauthorizedInput => "unauthorized_input",
        }
    }

    /// Parse a wire code. An unknown code is a typed
    /// [`ContractError::UnsupportedReasonCode`] — never aliased to a
    /// known code, never silently accepted.
    pub fn parse(value: &str) -> Result<ReasonCode, ContractError> {
        ReasonCode::ALL
            .into_iter()
            .find(|code| code.as_str() == value)
            .ok_or_else(|| ContractError::UnsupportedReasonCode {
                code: value.to_string(),
            })
    }

    /// The headline effect class of this registry entry.
    pub fn effect_class(&self) -> EffectClass {
        match self {
            ReasonCode::ManualExactUrgency
            | ReasonCode::BriefRuling
            | ReasonCode::LabelBandChange
            | ReasonCode::ClumpRefinement => EffectClass::BaseMutation,
            ReasonCode::DependencyPressure
            | ReasonCode::DeadlineBoost
            | ReasonCode::OpportunityExpired => EffectClass::DerivedPressure,
            ReasonCode::LabelRemovedNeedsTriage
            | ReasonCode::StrategicPivot
            | ReasonCode::AgingReconsideration => EffectClass::AdvisoryHint,
            ReasonCode::LabelConflict
            | ReasonCode::HysteresisSuppressed
            | ReasonCode::ProjectionEcho
            | ReasonCode::StaleSnapshot
            | ReasonCode::UnauthorizedInput => EffectClass::NoMutation,
        }
    }

    /// The field effects this registry entry permits.
    pub fn effect_permissions(&self) -> EffectPermissions {
        match self {
            ReasonCode::ManualExactUrgency | ReasonCode::LabelBandChange => EffectPermissions {
                base: true,
                derived: false,
                hint: false,
            },
            // A ruling accepts only its named base fields, or
            // returns a triage hint if underspecified.
            ReasonCode::BriefRuling => EffectPermissions {
                base: true,
                derived: false,
                hint: true,
            },
            ReasonCode::ClumpRefinement => EffectPermissions {
                base: true,
                derived: false,
                hint: false,
            },
            ReasonCode::DependencyPressure | ReasonCode::DeadlineBoost => EffectPermissions {
                base: false,
                derived: true,
                hint: false,
            },
            // Expiry removes a temporary derived boost and creates
            // a retriage hint; it never touches a base value.
            ReasonCode::OpportunityExpired => EffectPermissions {
                base: false,
                derived: true,
                hint: true,
            },
            ReasonCode::LabelRemovedNeedsTriage
            | ReasonCode::StrategicPivot
            | ReasonCode::AgingReconsideration => EffectPermissions {
                base: false,
                derived: false,
                hint: true,
            },
            // Conflict/reject with a triage hint only; no mutation.
            ReasonCode::LabelConflict | ReasonCode::HysteresisSuppressed => EffectPermissions {
                base: false,
                derived: false,
                hint: true,
            },
            ReasonCode::ProjectionEcho
            | ReasonCode::StaleSnapshot
            | ReasonCode::UnauthorizedInput => EffectPermissions {
                base: false,
                derived: false,
                hint: false,
            },
        }
    }

    /// Check one attempted field effect against this entry. A
    /// refusal is typed and non-mutating: the registry check is the
    /// boundary an applier consults before touching canonical
    /// state, so an unsupported pairing applies nothing.
    pub fn check_field_effect(&self, effect: FieldEffectKind) -> Result<(), ContractError> {
        let permissions = self.effect_permissions();
        let permitted = match effect {
            FieldEffectKind::Base => permissions.base,
            FieldEffectKind::Derived => permissions.derived,
            FieldEffectKind::Hint => permissions.hint,
        };
        if permitted {
            Ok(())
        } else {
            Err(ContractError::EffectNotPermitted {
                code: *self,
                effect,
            })
        }
    }
}

/// The shared `Result.outcome` vocabulary (architecture.md). A
/// read-only operation can have a verified result without
/// completing the work item; do not invent a second receipt state
/// machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    /// The command was refused; nothing mutated.
    Rejected,
    /// The command was accepted.
    Accepted,
    /// The operation is running.
    Running,
    /// Accepted locally, awaiting sync to a target.
    PendingSync,
    /// Some effects applied; the rest are named in the result.
    Partial,
    /// A competing accepted state exists (stale version, divergent
    /// payload at a revision, or same key with a different digest).
    Conflict,
    /// The outcome cannot be established (e.g. a timeout after a
    /// remote effect); the receipt is resumable, not a fictional
    /// cancellation.
    Uncertain,
    /// The effect was verified end to end.
    VerifiedComplete,
}

impl Outcome {
    /// The wire spelling of this outcome.
    pub fn as_str(&self) -> &'static str {
        match self {
            Outcome::Rejected => "rejected",
            Outcome::Accepted => "accepted",
            Outcome::Running => "running",
            Outcome::PendingSync => "pending_sync",
            Outcome::Partial => "partial",
            Outcome::Conflict => "conflict",
            Outcome::Uncertain => "uncertain",
            Outcome::VerifiedComplete => "verified_complete",
        }
    }

    /// Parse a wire outcome; an unknown value is a schema failure,
    /// never a guessed outcome.
    pub fn parse(value: &str) -> Result<Outcome, ContractError> {
        match value {
            "rejected" => Ok(Outcome::Rejected),
            "accepted" => Ok(Outcome::Accepted),
            "running" => Ok(Outcome::Running),
            "pending_sync" => Ok(Outcome::PendingSync),
            "partial" => Ok(Outcome::Partial),
            "conflict" => Ok(Outcome::Conflict),
            "uncertain" => Ok(Outcome::Uncertain),
            "verified_complete" => Ok(Outcome::VerifiedComplete),
            _ => Err(ContractError::InvalidSchema {
                detail: format!("unknown outcome: {value:?}"),
            }),
        }
    }
}

/// A command's reason: a registry code plus evidence. The code is
/// carried as its wire string so an unknown code survives parsing
/// and fails typed at validation, instead of being silently
/// dropped or aliased.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reason {
    /// Wire reason code; resolve with [`Reason::code_parsed`].
    pub code: String,
    /// Evidence references backing the reason.
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    /// Optional confidence; a finite number when present.
    #[serde(default)]
    pub confidence: Option<f64>,
}

impl Reason {
    /// Resolve the wire code against the registry.
    pub fn code_parsed(&self) -> Result<ReasonCode, ContractError> {
        ReasonCode::parse(&self.code)
    }

    /// Validate the reason object.
    pub fn validate(&self) -> Result<(), ContractError> {
        self.code_parsed()?;
        if let Some(confidence) = self.confidence {
            if !confidence.is_finite() {
                return Err(ContractError::NonFiniteNumber);
            }
        }
        Ok(())
    }
}

/// The shared command envelope (`fleet.command/v1`). Optional
/// fields are explicitly nullable on the wire. This payload
/// deliberately has no `authorized` field: authentication supplies
/// the principal and capabilities out of band.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Command {
    /// `fleet.command/v1`.
    pub schema_version: String,
    /// This operation's identity.
    pub operation_id: String,
    /// Idempotency key; scoped by authenticated principal and
    /// authority at lookup time.
    pub idempotency_key: String,
    /// `sha256:` fingerprint of the intent DTO (see
    /// [`Command::compute_request_digest`]).
    pub request_digest: String,
    /// Canonical pair identity of the target work.
    pub work: WorkRef,
    /// The registered command kind.
    pub kind: String,
    /// Typed arguments, per the registered command schema.
    pub arguments: Value,
    /// Audit/reference field only — never authorization.
    pub actor_ref: String,
    /// Where the command came from.
    pub source_ref: String,
    /// Source event reference, when the command derives from one.
    #[serde(default)]
    pub source_event_ref: Option<String>,
    /// Transport delivery identity; deduplicated independently of
    /// the command identity, and excluded from the intent digest.
    #[serde(default)]
    pub delivery_id: Option<String>,
    /// Source revision, when applicable.
    #[serde(default)]
    pub source_revision: Option<String>,
    /// Producing session reference, when applicable.
    #[serde(default)]
    pub session_ref: Option<String>,
    /// Expected per-work version (canonical decimal string);
    /// prevents stale writers. Distinct from any snapshot ref.
    #[serde(default)]
    pub expected_work_version: Option<String>,
    /// Relay lease generation (canonical decimal string).
    #[serde(default)]
    pub lease_generation: Option<String>,
    /// The read snapshot the producer calculated from; may be stale.
    #[serde(default)]
    pub observed_snapshot_ref: Option<String>,
    /// The protected owner-policy reference the producer checked.
    #[serde(default)]
    pub owner_policy_ref: Option<String>,
    /// Why this command is being issued.
    pub reason: Reason,
    /// Dry-run creates no work-state, journal, outbox or remote
    /// mutation; it is an execution mode, excluded from the digest.
    pub dry_run: bool,
}

impl Command {
    /// Parse a command from JSON text. A top-level `authorized`
    /// field is rejected as a contract violation before any other
    /// interpretation: a client authorization claim is never proof.
    pub fn from_json(text: &str) -> Result<Command, ContractError> {
        let value: Value =
            serde_json::from_str(text).map_err(|err| ContractError::InvalidSchema {
                detail: format!("command is not valid JSON: {err}"),
            })?;
        if let Some(object) = value.as_object() {
            if object.contains_key("authorized") {
                return Err(ContractError::ClientSuppliedAuthorization);
            }
        }
        let command: Command =
            serde_json::from_value(value).map_err(|err| ContractError::InvalidSchema {
                detail: format!("command does not match fleet.command/v1: {err}"),
            })?;
        command.validate()?;
        Ok(command)
    }

    /// Serialize the command to its wire JSON form.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("command serialization is infallible")
    }

    /// Validate the envelope: supported version, pair identity,
    /// canonical decimal versions, registry reason code, digest
    /// shape. Unknown major versions and unknown reason codes are
    /// typed failures with no mutation.
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != COMMAND_SCHEMA_VERSION {
            return Err(ContractError::UnsupportedVersion {
                schema_version: self.schema_version.clone(),
            });
        }
        self.work.validate()?;
        if let Some(version) = &self.expected_work_version {
            require_canonical_decimal("expected_work_version", version)?;
        }
        if let Some(generation) = &self.lease_generation {
            require_canonical_decimal("lease_generation", generation)?;
        }
        self.reason.validate()?;
        if !is_digest_string(&self.request_digest) {
            return Err(ContractError::InvalidSchema {
                detail: format!(
                    "request_digest is not `sha256:` + 64 lowercase hex: {:?}",
                    self.request_digest
                ),
            });
        }
        Ok(())
    }

    /// The exact intent DTO the request digest fingerprints:
    /// validated, default-expanded intent — schema version,
    /// canonical work pair, kind, typed arguments, expected work
    /// version, lease generation, observed snapshot reference,
    /// owner-policy reference and reason. Operation/key identifiers,
    /// delivery attempt IDs, receipt times, output formatting,
    /// wait/timeout and the dry-run execution mode are excluded, so
    /// a retry with a new delivery ID or wait budget recovers the
    /// original operation, while an altered intent or precondition
    /// conflicts.
    pub fn intent_dto(&self) -> Value {
        serde_json::json!({
            "schema_version": INTENT_SCHEMA_VERSION,
            "command_schema_version": COMMAND_SCHEMA_VERSION,
            "work": {
                "authority_id": self.work.authority_id,
                "work_id": self.work.work_id,
            },
            "kind": self.kind,
            "arguments": self.arguments,
            "expected_work_version": self.expected_work_version,
            "lease_generation": self.lease_generation,
            "observed_snapshot_ref": self.observed_snapshot_ref,
            "owner_policy_ref": self.owner_policy_ref,
            "reason": {
                "code": self.reason.code,
                "evidence_refs": self.reason.evidence_refs,
                "confidence": self.reason.confidence,
            },
        })
    }

    /// Compute this command's request digest from its intent DTO.
    pub fn compute_request_digest(&self) -> Result<String, ContractError> {
        digest(DIGEST_DOMAIN_COMMAND_INTENT, &self.intent_dto())
    }

    /// Check the carried `request_digest` against a fresh
    /// computation. A match is a fingerprint, not authority: it
    /// never implies authorization.
    pub fn verify_request_digest(&self) -> Result<(), ContractError> {
        let computed = self.compute_request_digest()?;
        if computed == self.request_digest {
            Ok(())
        } else {
            Err(ContractError::InvalidSchema {
                detail: "request_digest does not match the intent DTO".to_string(),
            })
        }
    }
}

/// An authenticated principal as supplied by the auth layer — the
/// only input an authorization decision may read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedPrincipal {
    /// The authenticated principal's reference.
    pub principal_ref: String,
    /// Capabilities the auth layer verified for this principal.
    pub capabilities: Vec<String>,
}

/// The outcome of an authorization evaluation. The authorized form
/// names the authenticated principal — never the command's
/// `actor_ref`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorizationDecision {
    /// The principal holds the required capability.
    Authorized {
        /// The authenticated principal that was authorized.
        principal_ref: String,
    },
    /// Authorization failed closed, with the typed reason.
    Denied {
        /// Why authorization was denied.
        error: ContractError,
    },
}

/// Evaluate authorization for a command from the authenticated
/// principal alone. `actor_ref` is deliberately never consulted: a
/// client actor reference, however owner-like, is not
/// authorization, and no principal (or a principal without the
/// capability) denies with [`ContractError::UnauthorizedInput`] —
/// the same failure the `unauthorized_input` reason code records.
pub fn evaluate_authorization(
    _command: &Command,
    principal: Option<&AuthenticatedPrincipal>,
    required_capability: &str,
) -> AuthorizationDecision {
    match principal {
        Some(principal)
            if principal
                .capabilities
                .iter()
                .any(|capability| capability == required_capability) =>
        {
            AuthorizationDecision::Authorized {
                principal_ref: principal.principal_ref.clone(),
            }
        }
        _ => AuthorizationDecision::Denied {
            error: ContractError::UnauthorizedInput,
        },
    }
}

/// One field effect named by a result, with its requested,
/// observed and verified states kept separate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Effect {
    /// The field this effect names.
    pub field: String,
    /// The state the command requested, when applicable.
    #[serde(default)]
    pub requested: Option<String>,
    /// The state observed after the operation, when applicable.
    #[serde(default)]
    pub observed: Option<String>,
    /// The state verified end to end, when applicable.
    #[serde(default)]
    pub verified: Option<String>,
}

/// A typed error carried inside a result envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResultError {
    /// The stable wire error code.
    pub code: String,
    /// One-line human-readable detail.
    pub message: String,
}

impl From<&ContractError> for ResultError {
    fn from(error: &ContractError) -> Self {
        ResultError {
            code: error.code().to_string(),
            message: error.to_string(),
        }
    }
}

/// The shared result envelope (`fleet.result/v1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandResult {
    /// `fleet.result/v1`.
    pub schema_version: String,
    /// The operation this result answers.
    pub operation_id: String,
    /// Canonical pair identity of the work.
    pub work: WorkRef,
    /// The accepted per-work version after the operation
    /// (canonical decimal string; may exceed u64 range safely
    /// because it is never coerced through a float).
    pub work_version: String,
    /// Wire outcome; resolve with [`CommandResult::outcome_parsed`].
    pub outcome: String,
    /// Requested/observed/verified field effects.
    #[serde(default)]
    pub effects: Vec<Effect>,
    /// Non-fatal warnings.
    #[serde(default)]
    pub warnings: Vec<String>,
    /// The typed error, when the outcome carries one.
    #[serde(default)]
    pub error: Option<ResultError>,
    /// The suggested next action, when one exists.
    #[serde(default)]
    pub next_action: Option<String>,
    /// When the result was observed (authority UTC string,
    /// preserved verbatim).
    pub observed_at: String,
}

impl CommandResult {
    /// Resolve the wire outcome against the shared vocabulary.
    pub fn outcome_parsed(&self) -> Result<Outcome, ContractError> {
        Outcome::parse(&self.outcome)
    }

    /// Validate the envelope: supported version, pair identity,
    /// canonical decimal work version, known outcome.
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != RESULT_SCHEMA_VERSION {
            return Err(ContractError::UnsupportedVersion {
                schema_version: self.schema_version.clone(),
            });
        }
        self.work.validate()?;
        require_canonical_decimal("work_version", &self.work_version)?;
        self.outcome_parsed()?;
        Ok(())
    }

    /// Serialize the result to its wire JSON form.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("result serialization is infallible")
    }
}

/// A versioned observation of the protected owner priority policy
/// (`fleet.owner-priority-policy/v1`): the policy/configuration
/// authority's interface over existing policy state. Membership is
/// owner-accepted policy, not inferred from a mutable label or
/// score; removing the mirror label cannot unprotect an item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OwnerPriorityPolicy {
    /// `fleet.owner-priority-policy/v1`.
    pub schema_version: String,
    /// The canonical reference commands cite as `owner_policy_ref`.
    pub policy_ref: String,
    /// The policy version (canonical decimal string).
    pub version: String,
    /// Source/evidence references for the policy.
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    /// The trusted configured Drew owner-principal binding.
    pub owner_principal_ref: String,
    /// Canonical reference to the protected-membership snapshot.
    pub protected_membership_ref: String,
    /// Fields the policy protects (score, peer placement,
    /// priority-label intent, membership, ...).
    #[serde(default)]
    pub protected_fields: Vec<String>,
    /// Actions the policy protects.
    #[serde(default)]
    pub protected_actions: Vec<String>,
}

impl OwnerPriorityPolicy {
    /// Validate the observation: supported version and a canonical
    /// decimal policy version.
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != OWNER_POLICY_SCHEMA_VERSION {
            return Err(ContractError::UnsupportedVersion {
                schema_version: self.schema_version.clone(),
            });
        }
        require_canonical_decimal("policy version", &self.version)
    }

    /// Serialize the observation to its wire JSON form.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("policy serialization is infallible")
    }

    /// Parse an observation from JSON text.
    pub fn from_json(text: &str) -> Result<OwnerPriorityPolicy, ContractError> {
        let policy: OwnerPriorityPolicy =
            serde_json::from_str(text).map_err(|err| ContractError::InvalidSchema {
                detail: format!("policy does not match fleet.owner-priority-policy/v1: {err}"),
            })?;
        policy.validate()?;
        Ok(policy)
    }
}

/// Check a command's `owner_policy_ref` against the current policy
/// observation, atomically with the mutation it guards: a matching
/// reference passes; a stale reference conflicts with
/// [`ContractError::OwnerPolicyVersionConflict`]; a missing
/// reference fails closed with
/// [`ContractError::MissingOwnerPolicyRef`]. Pure reads do not need
/// this check; priority-affecting mutations do.
pub fn verify_owner_policy_ref(
    command: &Command,
    current: &OwnerPriorityPolicy,
) -> Result<(), ContractError> {
    match &command.owner_policy_ref {
        None => Err(ContractError::MissingOwnerPolicyRef),
        Some(reference) if reference == &current.policy_ref => Ok(()),
        Some(_) => Err(ContractError::OwnerPolicyVersionConflict),
    }
}

/// Whether `value` is a wire digest: `sha256:` plus exactly 64
/// lowercase hexadecimal characters.
pub fn is_digest_string(value: &str) -> bool {
    match value.strip_prefix("sha256:") {
        Some(hex) => {
            hex.len() == 64
                && hex
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }
        None => false,
    }
}

/// Compute a wire digest under a registered domain:
/// `"sha256:" + lower_hex(SHA256(ASCII(domain) || 0x00 ||
/// UTF8(JCS(dto))))` — exactly one NUL byte, no length prefix, BOM,
/// newline or other whitespace outside the canonical JSON bytes.
pub fn digest(domain: &str, dto: &Value) -> Result<String, ContractError> {
    if !DIGEST_DOMAINS.contains(&domain) {
        return Err(ContractError::UnknownDigestDomain {
            domain: domain.to_string(),
        });
    }
    let canonical = canonical_json(dto)?;
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0u8]);
    hasher.update(canonical.as_bytes());
    let bytes: [u8; 32] = hasher.finalize().into();
    let mut out = String::with_capacity(7 + 64);
    out.push_str("sha256:");
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    Ok(out)
}

/// Serialize a value as RFC 8785 (JCS) canonical JSON: object keys
/// sorted by their UTF-16 code-unit sequences, minimal string
/// escaping, arrays preserved in order, no whitespace. Strings are
/// preserved without Unicode normalization. Non-finite numbers are
/// rejected before hashing, as the contract requires.
pub fn canonical_json(value: &Value) -> Result<String, ContractError> {
    let mut out = String::new();
    write_canonical(value, &mut out)?;
    Ok(out)
}

fn write_canonical(value: &Value, out: &mut String) -> Result<(), ContractError> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(number) => write_canonical_number(number, out)?,
        Value::String(text) => write_jcs_string(text, out),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(item, out)?;
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|(a, _), (b, _)| cmp_utf16(a, b));
            out.push('{');
            for (index, (key, item)) in entries.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_jcs_string(key, out);
                out.push(':');
                write_canonical(item, out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

/// Compare two strings by their UTF-16 code-unit sequences, the
/// JCS property-name ordering.
fn cmp_utf16(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

fn write_canonical_number(
    number: &serde_json::Number,
    out: &mut String,
) -> Result<(), ContractError> {
    if let Some(integer) = number.as_i64() {
        out.push_str(&integer.to_string());
        return Ok(());
    }
    if let Some(integer) = number.as_u64() {
        out.push_str(&integer.to_string());
        return Ok(());
    }
    let float = number.as_f64().ok_or(ContractError::NonFiniteNumber)?;
    if !float.is_finite() {
        return Err(ContractError::NonFiniteNumber);
    }
    if float == float.trunc() && float.abs() < 1e21 {
        // Integral values serialize as plain decimals, never in
        // exponent form and never through a lossy float print.
        out.push_str(&format!("{}", float as i128));
        return Ok(());
    }
    out.push_str(&number.to_string());
    Ok(())
}

fn write_jcs_string(text: &str, out: &mut String) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{0009}' => out.push_str("\\t"),
            '\u{000A}' => out.push_str("\\n"),
            '\u{000C}' => out.push_str("\\f"),
            '\u{000D}' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fixture command: the #9200 component's JSON contract
    /// example, with its illustrative reason code replaced by the
    /// registry code the example describes (`label_band_change`)
    /// and a computed request digest.
    fn fixture_command() -> Command {
        let mut command = Command {
            schema_version: COMMAND_SCHEMA_VERSION.to_string(),
            operation_id: "op_fixture_1".to_string(),
            idempotency_key: "github-delivery-derived-key".to_string(),
            request_digest: format!("sha256:{}", "0".repeat(64)),
            work: WorkRef::new("fleet-local", "842").unwrap(),
            kind: "priority.set_band".to_string(),
            arguments: serde_json::json!({
                "profile_id": "live-ratified",
                "profile_version": "3",
                "taxonomy_version": "2",
                "band": "next-up",
                "canonical_priority": "25",
            }),
            actor_ref: "github:user:octocat".to_string(),
            source_ref: "github:developing-today/code".to_string(),
            source_event_ref: Some("github:delivery:9f2c".to_string()),
            delivery_id: Some("delivery-9f2c".to_string()),
            source_revision: None,
            session_ref: None,
            expected_work_version: Some("41".to_string()),
            lease_generation: None,
            observed_snapshot_ref: Some("queue:rev:118;graph:rev:57".to_string()),
            owner_policy_ref: Some("owner-priority-policy:v3".to_string()),
            reason: Reason {
                code: "label_band_change".to_string(),
                evidence_refs: vec!["github:event:555".to_string()],
                confidence: None,
            },
            dry_run: false,
        };
        command.request_digest = command.compute_request_digest().unwrap();
        command
    }

    fn fixture_policy() -> OwnerPriorityPolicy {
        OwnerPriorityPolicy {
            schema_version: OWNER_POLICY_SCHEMA_VERSION.to_string(),
            policy_ref: "owner-priority-policy:v3".to_string(),
            version: "3".to_string(),
            evidence_refs: vec!["github:issue:7030".to_string()],
            owner_principal_ref: "auth:principal:drew".to_string(),
            protected_membership_ref: "owner-policy:membership:v3".to_string(),
            protected_fields: vec![
                "intrinsic_urgency".to_string(),
                "peer_rank".to_string(),
                "priority_label_intent".to_string(),
                "membership".to_string(),
            ],
            protected_actions: vec!["priority.write".to_string()],
        }
    }

    /// DST-01 oracle: pair identity, the reason-code registry with
    /// its field-effect classes, auth-vs-actor, version handling
    /// and exact integer-string serialization. Oracle source: the
    /// #9200 registry table and the architecture identity/digest
    /// contracts; every expected value below is copied from those
    /// documents, not from the implementation's convenience.
    #[test]
    fn pmo_distribution_command_identity_reason_and_version() {
        // --- Pair identity: authority bytewise, then work ID by
        // integer value (9 < 10 despite the string encoding).
        let nine = WorkRef::new("fleet-local", "9").unwrap();
        let ten = WorkRef::new("fleet-local", "10").unwrap();
        assert!(nine < ten);
        assert!(WorkRef::new("a-host", "10").unwrap() < WorkRef::new("b-host", "9").unwrap());
        assert_eq!(
            WorkRef::new("fleet-local", "842").unwrap().qualified_uri(),
            "fleet://fleet-local/work/842"
        );
        for bad in ["", "007", "12a", "-1", " 12", "1.5"] {
            let error = WorkRef::new("fleet-local", bad).unwrap_err();
            assert_eq!(error.code(), "invalid_schema", "work_id {bad:?}");
            assert!(!error.mutates_canonical_state());
        }

        // --- The registry: every code round-trips, and each
        // entry's effect class and permissions match the #9200
        // registry table exactly.
        assert_eq!(ReasonCode::ALL.len(), 15);
        let expected: [(ReasonCode, EffectClass, EffectPermissions); 15] = [
            (
                ReasonCode::ManualExactUrgency,
                EffectClass::BaseMutation,
                EffectPermissions {
                    base: true,
                    derived: false,
                    hint: false,
                },
            ),
            (
                ReasonCode::BriefRuling,
                EffectClass::BaseMutation,
                EffectPermissions {
                    base: true,
                    derived: false,
                    hint: true,
                },
            ),
            (
                ReasonCode::LabelBandChange,
                EffectClass::BaseMutation,
                EffectPermissions {
                    base: true,
                    derived: false,
                    hint: false,
                },
            ),
            (
                ReasonCode::LabelRemovedNeedsTriage,
                EffectClass::AdvisoryHint,
                EffectPermissions {
                    base: false,
                    derived: false,
                    hint: true,
                },
            ),
            (
                ReasonCode::LabelConflict,
                EffectClass::NoMutation,
                EffectPermissions {
                    base: false,
                    derived: false,
                    hint: true,
                },
            ),
            (
                ReasonCode::DependencyPressure,
                EffectClass::DerivedPressure,
                EffectPermissions {
                    base: false,
                    derived: true,
                    hint: false,
                },
            ),
            (
                ReasonCode::DeadlineBoost,
                EffectClass::DerivedPressure,
                EffectPermissions {
                    base: false,
                    derived: true,
                    hint: false,
                },
            ),
            (
                ReasonCode::OpportunityExpired,
                EffectClass::DerivedPressure,
                EffectPermissions {
                    base: false,
                    derived: true,
                    hint: true,
                },
            ),
            (
                ReasonCode::StrategicPivot,
                EffectClass::AdvisoryHint,
                EffectPermissions {
                    base: false,
                    derived: false,
                    hint: true,
                },
            ),
            (
                ReasonCode::AgingReconsideration,
                EffectClass::AdvisoryHint,
                EffectPermissions {
                    base: false,
                    derived: false,
                    hint: true,
                },
            ),
            (
                ReasonCode::ClumpRefinement,
                EffectClass::BaseMutation,
                EffectPermissions {
                    base: true,
                    derived: false,
                    hint: false,
                },
            ),
            (
                ReasonCode::HysteresisSuppressed,
                EffectClass::NoMutation,
                EffectPermissions {
                    base: false,
                    derived: false,
                    hint: true,
                },
            ),
            (
                ReasonCode::ProjectionEcho,
                EffectClass::NoMutation,
                EffectPermissions {
                    base: false,
                    derived: false,
                    hint: false,
                },
            ),
            (
                ReasonCode::StaleSnapshot,
                EffectClass::NoMutation,
                EffectPermissions {
                    base: false,
                    derived: false,
                    hint: false,
                },
            ),
            (
                ReasonCode::UnauthorizedInput,
                EffectClass::NoMutation,
                EffectPermissions {
                    base: false,
                    derived: false,
                    hint: false,
                },
            ),
        ];
        for (code, class, permissions) in expected {
            assert_eq!(ReasonCode::parse(code.as_str()), Ok(code));
            assert_eq!(code.effect_class(), class, "{}", code.as_str());
            assert_eq!(code.effect_permissions(), permissions, "{}", code.as_str());
            // The field-effect boundary: permitted pairings pass,
            // every other pairing is a typed, non-mutating refusal.
            for (effect, permitted) in [
                (FieldEffectKind::Base, permissions.base),
                (FieldEffectKind::Derived, permissions.derived),
                (FieldEffectKind::Hint, permissions.hint),
            ] {
                match code.check_field_effect(effect) {
                    Ok(()) => assert!(permitted, "{} x {effect:?}", code.as_str()),
                    Err(error) => {
                        assert!(!permitted, "{} x {effect:?}", code.as_str());
                        assert_eq!(error.code(), "effect_not_permitted");
                        assert!(!error.mutates_canonical_state());
                    }
                }
            }
        }
        // Ratified bands are never snapped by a reason code: the
        // base-mutation codes carry the exact stored value through
        // (15/25 remain distinct from any derived pressure).
        assert_eq!(fixture_command().arguments["canonical_priority"], "25");

        // --- Unknown codes are typed, never aliased. The design
        // example's illustrative `priority_label_changed` is not a
        // registry code and must fail like any other unknown code.
        for unknown in ["priority_label_changed", "made_up_code", ""] {
            let error = ReasonCode::parse(unknown).unwrap_err();
            assert_eq!(error.code(), "unsupported_reason_code");
            assert!(!error.mutates_canonical_state());
        }
        let mut bad_reason = fixture_command();
        bad_reason.reason.code = "priority_label_changed".to_string();
        assert_eq!(
            bad_reason.validate().unwrap_err().code(),
            "unsupported_reason_code"
        );

        // --- Versions: unknown major schema is typed; versions
        // cross JSON as exact canonical decimal strings, including
        // beyond the u64 range, and a JSON number in their place is
        // a schema failure rather than a coercion.
        let mut future = fixture_command();
        future.schema_version = "fleet.command/v2".to_string();
        assert_eq!(future.validate().unwrap_err().code(), "unsupported_version");
        let result = CommandResult {
            schema_version: RESULT_SCHEMA_VERSION.to_string(),
            operation_id: "op_fixture_1".to_string(),
            work: WorkRef::new("fleet-local", "842").unwrap(),
            work_version: "18446744073709551617".to_string(),
            outcome: Outcome::VerifiedComplete.as_str().to_string(),
            effects: vec![Effect {
                field: "canonical_priority".to_string(),
                requested: Some("25".to_string()),
                observed: Some("25".to_string()),
                verified: Some("25".to_string()),
            }],
            warnings: vec![],
            error: None,
            next_action: None,
            observed_at: "2026-11-13T03:30:00Z".to_string(),
        };
        result.validate().unwrap();
        let result_json = result.to_json();
        assert!(
            result_json.contains("\"work_version\":\"18446744073709551617\""),
            "{result_json}"
        );
        let parsed_result: CommandResult = serde_json::from_str(&result_json).unwrap();
        assert_eq!(parsed_result, result);
        assert_eq!(
            parsed_result.outcome_parsed(),
            Ok(Outcome::VerifiedComplete)
        );
        assert_eq!(
            Outcome::parse("completed").unwrap_err().code(),
            "invalid_schema"
        );
        let command_json = fixture_command().to_json();
        assert!(command_json.contains("\"expected_work_version\":\"41\""));
        let numeric_version = command_json.replace(
            "\"expected_work_version\":\"41\"",
            "\"expected_work_version\":41",
        );
        assert_eq!(
            Command::from_json(&numeric_version).unwrap_err().code(),
            "invalid_schema"
        );

        // --- Auth vs actor: a client `authorized` field is a
        // contract violation; an owner-looking `actor_ref` without
        // an authenticated principal authorizes nothing; only the
        // principal's verified capability authorizes, and the
        // decision names the principal, never the actor.
        let claimed = command_json.replace(
            "\"schema_version\":\"fleet.command/v1\"",
            "\"schema_version\":\"fleet.command/v1\",\"authorized\":true",
        );
        assert_eq!(
            Command::from_json(&claimed).unwrap_err().code(),
            "client_supplied_authorization"
        );
        let mut impersonating = fixture_command();
        impersonating.actor_ref = "github:user:drewrypope".to_string();
        assert_eq!(
            evaluate_authorization(&impersonating, None, "priority.write"),
            AuthorizationDecision::Denied {
                error: ContractError::UnauthorizedInput
            }
        );
        let claimant = AuthenticatedPrincipal {
            principal_ref: "auth:principal:lane".to_string(),
            capabilities: vec!["queue.claim".to_string()],
        };
        assert_eq!(
            evaluate_authorization(&impersonating, Some(&claimant), "priority.write"),
            AuthorizationDecision::Denied {
                error: ContractError::UnauthorizedInput
            }
        );
        let owner = AuthenticatedPrincipal {
            principal_ref: "auth:principal:drew".to_string(),
            capabilities: vec!["priority.write".to_string()],
        };
        assert_eq!(
            evaluate_authorization(&impersonating, Some(&owner), "priority.write"),
            AuthorizationDecision::Authorized {
                principal_ref: "auth:principal:drew".to_string()
            }
        );

        // --- Digest: the published byte-framing golden vectors
        // (contracts/digest-vectors.json) reproduce exactly; the
        // intent digest ignores delivery/operation/dry-run changes
        // and binds intent/precondition changes.
        assert_eq!(
            digest(DIGEST_DOMAIN_COMMAND_INTENT, &serde_json::json!({})).unwrap(),
            "sha256:7b4e8eded8bf04b8fb33a201535a59259bbcca6f006cadd124b492b12539b327"
        );
        assert_eq!(
            digest(DIGEST_DOMAIN_CONTEXT_MANIFEST, &serde_json::json!({})).unwrap(),
            "sha256:981087fc35a1ca949810f8b8ec64c0448e47bfedad36904fc3ba89874e22faa6"
        );
        assert_eq!(
            digest(DIGEST_DOMAIN_SELECTION_ORDER, &serde_json::json!({})).unwrap(),
            "sha256:67bc22ebc065c180c2b3981f7c1e7f96c54dc3fabf66c915fcb3e222bcc20de5"
        );
        assert_eq!(
            digest("fleet.unregistered/v1", &serde_json::json!({}))
                .unwrap_err()
                .code(),
            "unknown_digest_domain"
        );
        assert_eq!(
            canonical_json(&serde_json::json!({ "b": 1, "a": 2 })).unwrap(),
            "{\"a\":2,\"b\":1}"
        );
        let command = fixture_command();
        command.verify_request_digest().unwrap();
        let mut replay = command.clone();
        replay.delivery_id = Some("delivery-other".to_string());
        replay.operation_id = "op_fixture_2".to_string();
        replay.dry_run = true;
        assert_eq!(
            replay.compute_request_digest().unwrap(),
            command.compute_request_digest().unwrap()
        );
        let mut altered = command.clone();
        altered.expected_work_version = Some("42".to_string());
        assert_ne!(
            altered.compute_request_digest().unwrap(),
            command.compute_request_digest().unwrap()
        );
    }

    /// DST-01 policy-ref oracle: the `fleet.owner-priority-policy/v1`
    /// observation and a command's `owner_policy_ref` round-trip
    /// exactly, and the ref check matches the architecture's
    /// protected-owner-policy semantics (match passes, stale
    /// conflicts, missing fails closed).
    #[test]
    fn pmo_distribution_command_policy_ref_round_trips() {
        // The policy observation round-trips byte-exactly, with
        // its DTO ID and version spelled as architecture.md fixes
        // them.
        let policy = fixture_policy();
        policy.validate().unwrap();
        let policy_json = policy.to_json();
        assert!(policy_json.contains("\"schema_version\":\"fleet.owner-priority-policy/v1\""));
        assert!(policy_json.contains("\"version\":\"3\""));
        let parsed_policy = OwnerPriorityPolicy::from_json(&policy_json).unwrap();
        assert_eq!(parsed_policy, policy);
        assert_eq!(parsed_policy.to_json(), policy_json);

        // A non-canonical policy version is a schema failure.
        let mut sloppy = fixture_policy();
        sloppy.version = "03".to_string();
        assert_eq!(sloppy.validate().unwrap_err().code(), "invalid_schema");

        // A command carrying the ref round-trips byte-exactly.
        let command = fixture_command();
        assert_eq!(
            command.owner_policy_ref.as_deref(),
            Some("owner-priority-policy:v3")
        );
        let command_json = command.to_json();
        let parsed_command = Command::from_json(&command_json).unwrap();
        assert_eq!(parsed_command, command);
        assert_eq!(parsed_command.to_json(), command_json);

        // An absent ref is explicitly null on the wire, and also
        // round-trips.
        let mut no_ref = fixture_command();
        no_ref.owner_policy_ref = None;
        no_ref.request_digest = no_ref.compute_request_digest().unwrap();
        let no_ref_json = no_ref.to_json();
        assert!(
            no_ref_json.contains("\"owner_policy_ref\":null"),
            "{no_ref_json}"
        );
        assert_eq!(Command::from_json(&no_ref_json).unwrap(), no_ref);

        // The atomic ref check: current ref passes; a stale ref
        // conflicts with OWNER_POLICY_VERSION_CONFLICT; a missing
        // ref fails closed. All failures are non-mutating.
        assert_eq!(verify_owner_policy_ref(&command, &policy), Ok(()));
        let mut stale = fixture_command();
        stale.owner_policy_ref = Some("owner-priority-policy:v2".to_string());
        let conflict = verify_owner_policy_ref(&stale, &policy).unwrap_err();
        assert_eq!(conflict.code(), "OWNER_POLICY_VERSION_CONFLICT");
        assert!(!conflict.mutates_canonical_state());
        let missing = verify_owner_policy_ref(&no_ref, &policy).unwrap_err();
        assert_eq!(missing.code(), "owner_policy_ref_missing");
        assert!(!missing.mutates_canonical_state());
        // The same failures surface through the shared Result error
        // shape with their wire codes intact.
        let carried = ResultError::from(&conflict);
        assert_eq!(carried.code, "OWNER_POLICY_VERSION_CONFLICT");
    }
}
