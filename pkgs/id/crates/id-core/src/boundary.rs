//! Boundary declarations and their composition rules (#5683 design step 2, issue #7782).
//!
//! Design: `doc/2026-11-08_design_id5683.md` §2.3, §2.4, §3, §7 (the record
//! itself is decomposition item 2 of that design, §12). This module is the
//! declaration record, its validity rules, and the intersection of server,
//! world and user declarations. It holds no storage, no evaluation of subject
//! evidence, and no enforcement: those are later steps of the same design.
//! The evidence-assurance fields (§3) are carried here as the declaration's
//! [`EvidenceRequirement`]; the assurance layer on `residency.rs::Evidence`
//! itself is decomposition item 1.
//!
//! The rules the record enforces, from the design:
//!
//! - **Off is the only default.** A declaration is disabled unless an explicit
//!   action enabled it, and an enabled declaration must name at least one
//!   [`EnforcementPoint`]: a boundary nobody enforces is a false promise, so
//!   [`BoundaryDeclaration::new`] rejects it with the named error
//!   [`DeclarationError::EnabledWithoutEnforcement`] (§2.3).
//! - **Every change is a new version.** A declaration is immutable: its
//!   [`BoundaryDeclaration::id`] is the BLAKE3 digest of the canonical
//!   encoding of its body, so editing any field produces a different id.
//!   Versions are monotone per scope; [`BoundaryDeclaration::ensure_supersedes`]
//!   checks a successor against its predecessor (§2.3).
//! - **Four failure behaviours only.** [`FailureBehaviour`] is deny, warn,
//!   step-up or quarantine, and nothing else (§2.4).
//! - **Composition is intersection, always.** [`effective`] composes the
//!   enabled server, world and user declarations by intersecting enforcement
//!   points and accepted evidence sources and by taking the strongest failure
//!   behaviour and the highest assurance bar, so the result can only narrow
//!   what any participant allows (§7). A combination with no common ground
//!   is an [`EffectiveConflict`] naming the denying scope, never a silent
//!   merge.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// A boundary capability this skeleton understands (§2.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    /// Where content is stored, processed, transported or described.
    Residency,
    /// Citizenship or equivalent membership claim.
    Citizenship,
    /// A versioned sanctions list.
    Sanctions,
    /// An intranet-style network identity boundary.
    Network,
}

/// What a boundary is about. The scope owns the rule; the subject is judged
/// by it (§2.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubjectClass {
    /// A person.
    User,
    /// An organisation.
    Org,
    /// A server.
    Server,
    /// A world.
    World,
}

/// The unit that owns a boundary: a server, a world, or an account (§2.2).
/// Assigned locally; a peer cannot remove it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum ScopeRef {
    /// A server scope, identified locally.
    Server(String),
    /// A world scope, identified locally.
    World(String),
    /// An account scope, identified locally.
    Account(String),
}

impl ScopeRef {
    /// The identifier inside the reference, whatever its kind.
    pub fn id(&self) -> &str {
        match self {
            ScopeRef::Server(id) | ScopeRef::World(id) | ScopeRef::Account(id) => id,
        }
    }
}

/// How strongly a piece of evidence was established (§3.2). Ordered: a
/// higher level satisfies any requirement at or below it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssuranceLevel {
    /// The subject says so (the design's `Self` level).
    SelfDeclared,
    /// An organisation the operator trusts vouches for the subject.
    OrgAttested,
    /// A trusted third party verified a document or fact.
    ProviderVerified,
}

/// The evidence a declaration demands (§2.3, §3): the minimum assurance
/// level it accepts, and the sources it accepts evidence from. An empty
/// `accepted_sources` set names no restriction on the source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRequirement {
    /// Minimum assurance level accepted.
    pub minimum_assurance: AssuranceLevel,
    /// Evidence sources accepted. Empty accepts any source.
    pub accepted_sources: BTreeSet<String>,
}

impl EvidenceRequirement {
    /// Whether evidence at `level` from `source` meets this requirement.
    pub fn accepts(&self, level: AssuranceLevel, source: &str) -> bool {
        level >= self.minimum_assurance
            && (self.accepted_sources.is_empty() || self.accepted_sources.contains(source))
    }
}

/// One of the four failure behaviours, and no more (§2.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureBehaviour {
    /// Refuse the action.
    Deny,
    /// Allow, tell the user, log.
    Warn,
    /// Allow after a stronger authentication.
    StepUp,
    /// Allow into a holding state pending review.
    Quarantine,
}

impl FailureBehaviour {
    /// How adverse this behaviour is when outcomes combine (§2.4: deny is
    /// the strongest action): higher wins in an intersection.
    pub const fn severity(self) -> u8 {
        match self {
            FailureBehaviour::Warn => 0,
            FailureBehaviour::StepUp => 1,
            FailureBehaviour::Quarantine => 2,
            FailureBehaviour::Deny => 3,
        }
    }
}

/// A surface the declaration is enforced at (§5.2, §8). An enabled
/// declaration must name at least one; an omitted surface is not covered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnforcementPoint {
    /// Account creation / signup.
    AccountCreation,
    /// Login to an existing account.
    Login,
    /// World membership changes.
    WorldMembership,
    /// World blob storage.
    BlobStorage,
    /// Bridge / federation traffic.
    BridgeFederation,
    /// Relay transit.
    RelayTransit,
    /// Granting a capability, and exercising it.
    CapabilityGrant,
}

/// A reference to one version of a versioned list (§2.3 `data_ref`, §5.1):
/// a sanctions or network list, supplied as data, never compiled in.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ListVersionRef {
    /// The list, named by its supplier.
    pub id: String,
    /// The list version in force for this declaration.
    pub version: u64,
}

/// Who enabled the declaration (§2.3 `declared_by`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ActorId(pub String);

/// The declaration body, everything the content-addressed id commits to
/// (§2.3). The `id` field of [`BoundaryDeclaration`] is derived from this
/// body, never part of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundaryDeclarationBody {
    /// The boundary capability this declaration governs.
    pub capability: CapabilityKind,
    /// The scope that owns the rule.
    pub scope: ScopeRef,
    /// What the rule judges.
    pub subject_class: SubjectClass,
    /// Minimum evidence assurance and accepted sources (§3).
    pub evidence_req: EvidenceRequirement,
    /// Surfaces this declaration is checked at. Non-empty when enabled.
    pub enforcement: BTreeSet<EnforcementPoint>,
    /// What a failed check does.
    pub failure: FailureBehaviour,
    /// The versioned list this boundary reads, if any (sanctions/network).
    pub data_ref: Option<ListVersionRef>,
    /// Whether the boundary is active. True only by explicit action.
    pub enabled: bool,
    /// Monotone version within the owning scope, starting at 1.
    pub version: u64,
    /// Who enabled it.
    pub declared_by: ActorId,
    /// Unix seconds.
    pub declared_at: u64,
}

/// The content-addressed id of a declaration: BLAKE3 over the canonical
/// body (§2.3). Edits are new ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DeclarationId([u8; 32]);

impl DeclarationId {
    /// The raw 32 digest bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Lower-case hexadecimal, as used in logs and references.
    pub fn to_hex(self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl std::fmt::Display for DeclarationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// Why a declaration was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DeclarationError {
    /// An enabled declaration names no enforcement point, so nobody
    /// enforces it: a false promise, rejected at declaration time (§2.3).
    #[error("enabled boundary declaration names no enforcement point")]
    EnabledWithoutEnforcement,
    /// Versions are monotone and start at one.
    #[error("boundary declaration version must be at least 1 and strictly increase")]
    InvalidVersion,
    /// A successor must keep the predecessor's scope and capability.
    #[error("boundary declaration successor changes scope or capability")]
    NotASuccessor,
    /// The scope, actor or list identity is empty.
    #[error("boundary declaration {0} is empty")]
    EmptyIdentity(&'static str),
}

/// A versioned, content-addressed boundary declaration (§2.3). Immutable
/// once built: every change is a new version with a new id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundaryDeclaration {
    /// The content-addressed id, derived from every other field.
    pub id: DeclarationId,
    /// The declaration body the id commits to.
    pub body: BoundaryDeclarationBody,
}

impl Default for BoundaryDeclaration {
    /// The only implicit default (§1, §2.3): off, enforcing nothing. The
    /// placeholder identities mark the record as never declared by anyone;
    /// it is inert (`enabled` is false) until a real declaration replaces it.
    fn default() -> Self {
        let body = BoundaryDeclarationBody {
            capability: CapabilityKind::Residency,
            scope: ScopeRef::Server("unset".into()),
            subject_class: SubjectClass::User,
            evidence_req: EvidenceRequirement {
                minimum_assurance: AssuranceLevel::SelfDeclared,
                accepted_sources: BTreeSet::new(),
            },
            enforcement: BTreeSet::new(),
            failure: FailureBehaviour::Deny,
            data_ref: None,
            enabled: false,
            version: 1,
            declared_by: ActorId("unset".into()),
            declared_at: 0,
        };
        let id = declaration_id(&body);
        Self { id, body }
    }
}

impl BoundaryDeclaration {
    /// Build a declaration, checking the validity rules. The id is derived,
    /// not supplied, so it can never disagree with the body.
    ///
    /// # Errors
    ///
    /// Returns [`DeclarationError`] when an enabled declaration has no
    /// enforcement points, the version is zero, or an identity is empty.
    pub fn new(body: BoundaryDeclarationBody) -> Result<Self, DeclarationError> {
        validate(&body)?;
        let id = declaration_id(&body);
        Ok(Self { id, body })
    }

    /// Whether this declaration is in force. Disabled is the default state
    /// of every boundary (§2.3).
    pub fn is_enabled(&self) -> bool {
        self.body.enabled
    }

    /// Check that `self` is a legal successor of `previous`: same scope and
    /// capability, and a strictly greater version (monotone, §2.3).
    ///
    /// # Errors
    ///
    /// Returns [`DeclarationError::NotASuccessor`] when scope or capability
    /// changed, and [`DeclarationError::InvalidVersion`] when the version
    /// did not strictly increase.
    pub fn ensure_supersedes(&self, previous: &Self) -> Result<(), DeclarationError> {
        if self.body.scope != previous.body.scope
            || self.body.capability != previous.body.capability
        {
            return Err(DeclarationError::NotASuccessor);
        }
        if self.body.version <= previous.body.version {
            return Err(DeclarationError::InvalidVersion);
        }
        Ok(())
    }
}

fn validate(body: &BoundaryDeclarationBody) -> Result<(), DeclarationError> {
    if body.enabled && body.enforcement.is_empty() {
        return Err(DeclarationError::EnabledWithoutEnforcement);
    }
    if body.version == 0 {
        return Err(DeclarationError::InvalidVersion);
    }
    if body.scope.id().trim().is_empty() {
        return Err(DeclarationError::EmptyIdentity("scope"));
    }
    if body.declared_by.0.trim().is_empty() {
        return Err(DeclarationError::EmptyIdentity("declared_by"));
    }
    if let Some(data_ref) = &body.data_ref {
        if data_ref.id.trim().is_empty() {
            return Err(DeclarationError::EmptyIdentity("data_ref"));
        }
    }
    Ok(())
}

/// The canonical encoding of a declaration body, used for the content
/// address. Serde's derived serialisation follows the fixed struct field
/// order, and `deny_unknown_fields` freezes the shape, so the encoding is
/// stable for a given body.
fn canonical_bytes(body: &BoundaryDeclarationBody) -> Vec<u8> {
    serde_json::to_vec(body).unwrap_or_default()
}

fn declaration_id(body: &BoundaryDeclarationBody) -> DeclarationId {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"id.boundary.declaration.v1\0");
    hasher.update(&canonical_bytes(body));
    DeclarationId(*hasher.finalize().as_bytes())
}

/// The intersection of the enabled declarations (§7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveBoundary {
    /// The capability composed. `None` when no declaration is enabled.
    pub capability: Option<CapabilityKind>,
    /// Surfaces every enabled participant enforces.
    pub enforcement: BTreeSet<EnforcementPoint>,
    /// The most adverse failure behaviour among the participants.
    pub failure: Option<FailureBehaviour>,
    /// The highest assurance bar among the participants.
    pub minimum_assurance: Option<AssuranceLevel>,
    /// Sources every participant with a source restriction accepts. Empty
    /// means no participant restricted the source.
    pub accepted_sources: BTreeSet<String>,
}

/// Why a composition was refused. The conflict is a deny, and it names the
/// denying scope (§7).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EffectiveConflict {
    /// An enabled declaration names no enforcement point. It is invalid by
    /// the declaration rules; composition refuses rather than ignoring it.
    #[error("scope {scope:?} is enabled without enforcement")]
    EnabledWithoutEnforcement {
        /// The scope that owns the invalid declaration.
        scope: ScopeRef,
    },
    /// A participant governs a different capability than the first, so the
    /// declarations are not one boundary and cannot be merged.
    #[error("scope {scope:?} declares a different capability and denies the composition")]
    CapabilityMismatch {
        /// The scope whose declaration does not compose.
        scope: ScopeRef,
    },
    /// The participants share no enforcement surface: the intersection is
    /// empty, so the composition denies at the scope that emptied it.
    #[error(
        "scope {denying:?} shares no enforcement surface with {wider:?}; denied at {denying:?}"
    )]
    NoCommonEnforcement {
        /// The scope whose enforcement set emptied the intersection.
        denying: ScopeRef,
        /// The scope that enforced `point`.
        wider: ScopeRef,
        /// A surface the wider scope enforces and the denying scope does not.
        point: EnforcementPoint,
    },
    /// The participants share no accepted evidence source, so no evidence
    /// could ever satisfy the composition.
    #[error(
        "scope {denying:?} shares no accepted evidence source with {wider:?}; denied at {denying:?}"
    )]
    NoCommonEvidenceSource {
        /// The scope whose source set emptied the intersection.
        denying: ScopeRef,
        /// The first scope with a source restriction.
        wider: ScopeRef,
    },
}

/// Compute the effective boundary of `server`, `world` and `user`
/// declarations: the intersection of the enabled ones (§7).
///
/// Only enabled declarations participate; a disabled declaration is off and
/// contributes nothing. Among participants the composition can only narrow:
/// enforcement points and accepted sources are intersected, the failure
/// behaviour is the most adverse, and the assurance bar is the highest. A
/// user preference therefore can never widen a server or world boundary,
/// and a world can never widen its server's. When the intersection is
/// empty the composition is a deny naming the denying scope
/// ([`EffectiveConflict`]), never a silent merge.
///
/// # Errors
///
/// Returns [`EffectiveConflict`] when an enabled participant is invalid,
/// when participants govern different capabilities, or when they share no
/// enforcement surface or accepted evidence source.
pub fn effective(
    server: Option<&BoundaryDeclaration>,
    world: Option<&BoundaryDeclaration>,
    user: Option<&BoundaryDeclaration>,
) -> Result<EffectiveBoundary, EffectiveConflict> {
    let mut participants: Vec<&BoundaryDeclaration> = Vec::new();
    for declaration in [server, world, user].into_iter().flatten() {
        if !declaration.is_enabled() {
            continue;
        }
        if declaration.body.enforcement.is_empty() {
            return Err(EffectiveConflict::EnabledWithoutEnforcement {
                scope: declaration.body.scope.clone(),
            });
        }
        participants.push(declaration);
    }

    let Some(first) = participants.first() else {
        return Ok(EffectiveBoundary {
            capability: None,
            enforcement: BTreeSet::new(),
            failure: None,
            minimum_assurance: None,
            accepted_sources: BTreeSet::new(),
        });
    };

    let mut enforcement = first.body.enforcement.clone();
    let mut failure = first.body.failure;
    let mut minimum_assurance = first.body.evidence_req.minimum_assurance;
    // A participant with no source restriction accepts any source, so only
    // restricted participants take part in the source intersection.
    let mut accepted_sources: Option<BTreeSet<String>> = None;
    let mut source_scope: Option<&ScopeRef> = None;
    if !first.body.evidence_req.accepted_sources.is_empty() {
        accepted_sources = Some(first.body.evidence_req.accepted_sources.clone());
        source_scope = Some(&first.body.scope);
    }

    for participant in &participants[1..] {
        if participant.body.capability != first.body.capability {
            return Err(EffectiveConflict::CapabilityMismatch {
                scope: participant.body.scope.clone(),
            });
        }
        let narrowed: BTreeSet<EnforcementPoint> = enforcement
            .intersection(&participant.body.enforcement)
            .copied()
            .collect();
        if narrowed.is_empty() {
            let point = *enforcement
                .first()
                .unwrap_or(&EnforcementPoint::AccountCreation);
            return Err(EffectiveConflict::NoCommonEnforcement {
                denying: participant.body.scope.clone(),
                wider: first.body.scope.clone(),
                point,
            });
        }
        enforcement = narrowed;
        if participant.body.failure.severity() > failure.severity() {
            failure = participant.body.failure;
        }
        if participant.body.evidence_req.minimum_assurance > minimum_assurance {
            minimum_assurance = participant.body.evidence_req.minimum_assurance;
        }
        if !participant.body.evidence_req.accepted_sources.is_empty() {
            match &accepted_sources {
                None => {
                    accepted_sources = Some(participant.body.evidence_req.accepted_sources.clone());
                    source_scope = Some(&participant.body.scope);
                }
                Some(current) => {
                    let narrowed_sources: BTreeSet<String> = current
                        .intersection(&participant.body.evidence_req.accepted_sources)
                        .cloned()
                        .collect();
                    if narrowed_sources.is_empty() {
                        return Err(EffectiveConflict::NoCommonEvidenceSource {
                            denying: participant.body.scope.clone(),
                            wider: source_scope
                                .cloned()
                                .unwrap_or_else(|| first.body.scope.clone()),
                        });
                    }
                    accepted_sources = Some(narrowed_sources);
                }
            }
        }
    }

    Ok(EffectiveBoundary {
        capability: Some(first.body.capability),
        enforcement,
        failure: Some(failure),
        minimum_assurance: Some(minimum_assurance),
        accepted_sources: accepted_sources.unwrap_or_default(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn body(
        scope: ScopeRef,
        enabled: bool,
        enforcement: &[EnforcementPoint],
    ) -> BoundaryDeclarationBody {
        BoundaryDeclarationBody {
            capability: CapabilityKind::Residency,
            scope,
            subject_class: SubjectClass::User,
            evidence_req: EvidenceRequirement {
                minimum_assurance: AssuranceLevel::SelfDeclared,
                accepted_sources: BTreeSet::new(),
            },
            enforcement: enforcement.iter().copied().collect(),
            failure: FailureBehaviour::Deny,
            data_ref: None,
            enabled,
            version: 1,
            declared_by: ActorId("admin".into()),
            declared_at: 1,
        }
    }

    fn server(scope: &str, enabled: bool, enforcement: &[EnforcementPoint]) -> BoundaryDeclaration {
        BoundaryDeclaration::new(body(ScopeRef::Server(scope.into()), enabled, enforcement))
            .expect("valid declaration")
    }

    #[test]
    fn enabled_without_enforcement_is_named_rejection() {
        // Oracle: design §2.3 — a record with enabled:true and empty
        // enforcement is rejected with a named error.
        let err =
            BoundaryDeclaration::new(body(ScopeRef::Server("s".into()), true, &[])).unwrap_err();
        assert_eq!(err, DeclarationError::EnabledWithoutEnforcement);
        assert_eq!(
            err.to_string(),
            "enabled boundary declaration names no enforcement point"
        );
    }

    #[test]
    fn disabled_without_enforcement_is_valid_and_default_is_off() {
        let d = server("s", false, &[]);
        assert!(!d.is_enabled());
        assert!(!BoundaryDeclaration::default().is_enabled());
        assert!(BoundaryDeclaration::default().body.enforcement.is_empty());
    }

    #[test]
    fn content_addressed_id_changes_with_any_field() {
        let base = server("s", true, &[EnforcementPoint::AccountCreation]);
        let mut changed = base.body.clone();
        changed.failure = FailureBehaviour::Warn;
        let other = BoundaryDeclaration::new(changed).expect("valid");
        assert_ne!(base.id, other.id);

        let mut renamed_body = base.body.clone();
        renamed_body.declared_by = ActorId("other".into());
        let renamed = BoundaryDeclaration::new(renamed_body).expect("valid");
        assert_ne!(base.id, renamed.id);

        let same = server("s", true, &[EnforcementPoint::AccountCreation]);
        assert_eq!(base.id, same.id);
    }

    #[test]
    fn versions_are_monotone() {
        let first = server("s", true, &[EnforcementPoint::AccountCreation]);
        let mut second_body = body(
            ScopeRef::Server("s".into()),
            true,
            &[EnforcementPoint::AccountCreation],
        );
        second_body.version = 2;
        let second = BoundaryDeclaration::new(second_body).expect("valid");
        second.ensure_supersedes(&first).expect("2 supersedes 1");

        let stale = server("s", true, &[EnforcementPoint::AccountCreation]);
        assert_eq!(
            stale.ensure_supersedes(&first).unwrap_err(),
            DeclarationError::InvalidVersion
        );

        let mut other_scope_body = body(
            ScopeRef::World("w".into()),
            true,
            &[EnforcementPoint::AccountCreation],
        );
        other_scope_body.version = 2;
        let other_scope = BoundaryDeclaration::new(other_scope_body).expect("valid");
        assert_eq!(
            other_scope.ensure_supersedes(&first).unwrap_err(),
            DeclarationError::NotASuccessor
        );
    }

    #[test]
    fn evidence_requirement_checks_level_and_source() {
        let req = EvidenceRequirement {
            minimum_assurance: AssuranceLevel::OrgAttested,
            accepted_sources: ["org-roster".to_string()].into_iter().collect(),
        };
        assert!(!req.accepts(AssuranceLevel::SelfDeclared, "org-roster"));
        assert!(!req.accepts(AssuranceLevel::OrgAttested, "self-form"));
        assert!(req.accepts(AssuranceLevel::OrgAttested, "org-roster"));
        assert!(req.accepts(AssuranceLevel::ProviderVerified, "org-roster"));
    }

    #[test]
    fn effective_intersects_and_takes_the_strongest_terms() {
        let mut server_body = body(
            ScopeRef::Server("server-a".into()),
            true,
            &[
                EnforcementPoint::AccountCreation,
                EnforcementPoint::WorldMembership,
            ],
        );
        server_body.failure = FailureBehaviour::Warn;
        let server_d = BoundaryDeclaration::new(server_body).expect("valid");
        let mut world_body = body(
            ScopeRef::World("world-b".into()),
            true,
            &[
                EnforcementPoint::AccountCreation,
                EnforcementPoint::WorldMembership,
                EnforcementPoint::BlobStorage,
            ],
        );
        world_body.failure = FailureBehaviour::Deny;
        world_body.evidence_req.minimum_assurance = AssuranceLevel::OrgAttested;
        let world_d = BoundaryDeclaration::new(world_body).expect("valid");

        let eff = effective(Some(&server_d), Some(&world_d), None).expect("composes");
        assert_eq!(
            eff.enforcement,
            [
                EnforcementPoint::AccountCreation,
                EnforcementPoint::WorldMembership
            ]
            .into_iter()
            .collect()
        );
        assert_eq!(eff.failure, Some(FailureBehaviour::Deny));
        assert_eq!(eff.minimum_assurance, Some(AssuranceLevel::OrgAttested));
    }

    #[test]
    fn conflict_yields_a_deny_naming_the_denying_scope() {
        // Oracle: design §7 — a conflict is a deny naming which scope denied.
        let server_d = server(
            "server-a",
            true,
            &[
                EnforcementPoint::AccountCreation,
                EnforcementPoint::WorldMembership,
            ],
        );
        let world_d = BoundaryDeclaration::new(body(
            ScopeRef::World("world-b".into()),
            true,
            &[EnforcementPoint::BlobStorage],
        ))
        .expect("valid");
        let err = effective(Some(&server_d), Some(&world_d), None).unwrap_err();
        assert_eq!(
            err,
            EffectiveConflict::NoCommonEnforcement {
                denying: ScopeRef::World("world-b".into()),
                wider: ScopeRef::Server("server-a".into()),
                point: EnforcementPoint::AccountCreation,
            }
        );
        assert!(err.to_string().contains("world-b"));
    }

    #[test]
    fn disabled_declarations_do_not_participate() {
        let server_d = server("server-a", true, &[EnforcementPoint::AccountCreation]);
        let world_d = BoundaryDeclaration::new(body(
            ScopeRef::World("world-b".into()),
            false,
            &[EnforcementPoint::BlobStorage],
        ))
        .expect("valid");
        let eff = effective(Some(&server_d), Some(&world_d), None).expect("composes");
        assert_eq!(
            eff.enforcement,
            [EnforcementPoint::AccountCreation].into_iter().collect()
        );
        let none = effective(None, None, None).expect("empty composition is off");
        assert_eq!(none.capability, None);
        assert!(none.enforcement.is_empty());
    }

    mod narrowing_property {
        use super::*;
        use proptest::prelude::*;

        fn enforcement_strategy() -> impl Strategy<Value = BTreeSet<EnforcementPoint>> {
            prop::collection::vec(
                prop::sample::select(vec![
                    EnforcementPoint::AccountCreation,
                    EnforcementPoint::Login,
                    EnforcementPoint::WorldMembership,
                    EnforcementPoint::BlobStorage,
                ]),
                1..=4,
            )
            .prop_map(|points| points.into_iter().collect())
        }

        fn sources_strategy() -> impl Strategy<Value = BTreeSet<String>> {
            prop::collection::vec(prop::sample::select(vec!["a", "b", "c"]), 0..=3)
                .prop_map(|sources| sources.into_iter().map(str::to_string).collect())
        }

        fn declaration_strategy(scope: ScopeRef) -> impl Strategy<Value = BoundaryDeclaration> {
            (
                enforcement_strategy(),
                prop::sample::select(vec![
                    FailureBehaviour::Deny,
                    FailureBehaviour::Warn,
                    FailureBehaviour::StepUp,
                    FailureBehaviour::Quarantine,
                ]),
                prop::sample::select(vec![
                    AssuranceLevel::SelfDeclared,
                    AssuranceLevel::OrgAttested,
                    AssuranceLevel::ProviderVerified,
                ]),
                sources_strategy(),
            )
                .prop_map(move |(enforcement, failure, assurance, sources)| {
                    let mut b = body(scope.clone(), true, &[]);
                    b.enforcement = enforcement;
                    b.failure = failure;
                    b.evidence_req.minimum_assurance = assurance;
                    b.evidence_req.accepted_sources = sources;
                    BoundaryDeclaration::new(b).expect("generated declaration is valid")
                })
        }

        proptest! {
            /// Oracle: design §7 — effective(server, world, user) is the
            /// intersection, so it is narrowing-only: the result enforces no
            /// surface any participant lacks, is at least as adverse and
            /// demands at least as much assurance as every participant, and
            /// accepts no source any restricted participant rejects. When the
            /// participants share an enforcement surface and a compatible
            /// source set the composition must succeed with exactly the
            /// common surfaces; otherwise it must refuse with a conflict.
            #[test]
            fn effective_is_narrowing_only(
                server_d in declaration_strategy(ScopeRef::Server("server-a".into())),
                world_d in declaration_strategy(ScopeRef::World("world-b".into())),
                user_d in declaration_strategy(ScopeRef::Account("user-c".into())),
            ) {
                let participants = [&server_d, &world_d, &user_d];
                let common: BTreeSet<EnforcementPoint> = participants
                    .iter()
                    .fold(server_d.body.enforcement.clone(), |acc, d| {
                        acc.intersection(&d.body.enforcement).copied().collect()
                    });
                let restricted: Vec<&BTreeSet<String>> = participants
                    .iter()
                    .map(|d| &d.body.evidence_req.accepted_sources)
                    .filter(|sources| !sources.is_empty())
                    .collect();
                let sources_compatible = restricted.is_empty()
                    || restricted
                        .iter()
                        .fold(restricted[0].clone(), |acc, sources| {
                            acc.intersection(sources).cloned().collect::<BTreeSet<String>>()
                        })
                        .len()
                        > 0;

                match effective(Some(&server_d), Some(&world_d), Some(&user_d)) {
                    Ok(eff) => {
                        prop_assert!(!common.is_empty());
                        prop_assert!(sources_compatible);
                        prop_assert_eq!(&eff.enforcement, &common);
                        for d in participants {
                            prop_assert!(eff.enforcement.is_subset(&d.body.enforcement));
                            let eff_failure = eff.failure.expect("participants exist");
                            prop_assert!(eff_failure.severity() >= d.body.failure.severity());
                            let eff_assurance =
                                eff.minimum_assurance.expect("participants exist");
                            prop_assert!(eff_assurance >= d.body.evidence_req.minimum_assurance);
                            if !d.body.evidence_req.accepted_sources.is_empty() {
                                prop_assert!(
                                    eff.accepted_sources
                                        .is_subset(&d.body.evidence_req.accepted_sources)
                                );
                            }
                        }
                    }
                    Err(conflict) => {
                        prop_assert!(common.is_empty() || !sources_compatible);
                        prop_assert!(
                            matches!(
                                conflict,
                                EffectiveConflict::NoCommonEnforcement { .. }
                                    | EffectiveConflict::NoCommonEvidenceSource { .. }
                            ),
                            "unexpected conflict: {conflict}"
                        );
                    }
                }
            }
        }
    }
}
