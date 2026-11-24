//! The serving app's boundary gate for the signup/login surface (#7787).
//!
//! Design: `doc/2026-11-08_design_id5683.md` §8, with the input decisions
//! in `doc/2026-11-17T21-55-00Z_design_id8851_boundary_inputs.md`. The gate
//! is the [`BoundaryGate`] a served world's [`Directory`] consults before
//! account creation and credential login. It owns no evaluation of its
//! own: it assembles the inputs the server config's [`Boundaries`] loader
//! produces, offers the claims the app can make about the subject, and
//! renders the one shared adapter's outcome
//! ([`id_core::boundary_surfaces::check_surface`]).
//!
//! Every non-allow outcome is recorded before it is returned: the
//! adapter's decision-log entry (scope, boundary version, evidence
//! *classes*, outcome — never raw evidence values, §8) is emitted on the
//! node's log under the `boundary_decision` target. The signed per-scope
//! decision log is #5883's own piece; until it lands, no adverse decision
//! goes unrecorded.
//!
//! [`Directory`]: id_directory::directory::Directory

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use id_core::boundary::{ActorId, CapabilityKind, EnforcementPoint};
use id_core::boundary_surfaces::{FrameworkOutcome, check_surface};
use id_core::residency::{EvidenceVerifier, SubjectEvidence};
use id_directory::directory::BoundaryGate;

use crate::server_config::{Boundaries, ScopesInPlay};

/// Where a surface check's claims come from.
///
/// Given the subject, the claims the app can make about it (design
/// §2–§3), built with [`crate::boundary_evidence::ClaimsBuilder`] so
/// peer claims are graded by the trust roots. Hop-evidence signing and
/// key custody stay with the caller (design §8 open point); a surface
/// with no hop evidence yet passes a factory that returns none, and an
/// enabled boundary then fails closed, as the framework requires.
pub type ClaimsFactory = Arc<dyn Fn(&str) -> Vec<SubjectEvidence> + Send + Sync>;

/// The boundary gate the serve path installs on a world's directory:
/// the server config's declarations, composed per check for the scopes
/// in play (this server, the gate's world, the subject's account).
pub struct ServerBoundaryGate {
    boundaries: Boundaries,
    server: String,
    world: Option<String>,
    verifier: Arc<dyn EvidenceVerifier + Send + Sync>,
    claims: ClaimsFactory,
}

impl ServerBoundaryGate {
    /// A gate for one served world (`None` for a directory that is not
    /// a world's), checking the residency capability's declarations.
    #[must_use]
    pub fn new(
        boundaries: Boundaries,
        server: String,
        world: Option<String>,
        verifier: Arc<dyn EvidenceVerifier + Send + Sync>,
        claims: ClaimsFactory,
    ) -> Self {
        Self {
            boundaries,
            server,
            world,
            verifier,
            claims,
        }
    }

    /// A gate with no hop evidence to offer (see [`ClaimsFactory`]).
    #[must_use]
    pub fn without_hop_evidence(
        boundaries: Boundaries,
        server: String,
        world: Option<String>,
        verifier: Arc<dyn EvidenceVerifier + Send + Sync>,
    ) -> Self {
        Self::new(
            boundaries,
            server,
            world,
            verifier,
            Arc::new(|_| Vec::new()),
        )
    }
}

impl std::fmt::Debug for ServerBoundaryGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerBoundaryGate")
            .field("boundaries", &self.boundaries)
            .field("server", &self.server)
            .field("world", &self.world)
            .finish_non_exhaustive()
    }
}

impl BoundaryGate for ServerBoundaryGate {
    fn check(&self, point: EnforcementPoint, subject: &str) -> FrameworkOutcome {
        let scopes = ScopesInPlay {
            server: &self.server,
            world: self.world.as_deref(),
            account: Some(subject),
        };
        let inputs = match self.boundaries.inputs(
            CapabilityKind::Residency,
            scopes,
            Arc::clone(&self.verifier),
        ) {
            Ok(inputs) => inputs,
            Err(conflict) => {
                // Enabled declarations in play share no enforcement
                // surface or evidence source. An enabled boundary with
                // missing choices is invalid, never an implicit allow
                // (§2.3): fail closed, and say so on the log.
                tracing::error!(
                    ?conflict,
                    "boundary declarations in play do not compose; refusing the action"
                );
                return FrameworkOutcome::Deny {
                    placement: Vec::new(),
                    assurance: Vec::new(),
                };
            }
        };
        let claims = (self.claims)(subject);
        let now = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => duration.as_secs(),
            Err(_) => 0,
        };
        let check = check_surface(
            point,
            &inputs.effective,
            inputs.version,
            &inputs.scope_ref,
            &ActorId(subject.to_owned()),
            subject,
            &inputs.scope,
            &claims,
            now,
            &*inputs.verifier,
        );
        if let Some(entry) = &check.log_entry {
            tracing::info!(
                target: "boundary_decision",
                surface = %entry.surface,
                scope = ?entry.scope,
                boundary_version = entry.boundary_version,
                outcome = %entry.outcome,
                evidence_classes = ?entry.evidence_classes,
                minimum_assurance = ?entry.minimum_assurance,
                reasons = ?entry.reasons,
                actor = %entry.actor.0,
                at = entry.at,
                "boundary decision"
            );
        }
        check.outcome
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::Arc;

    use id_core::boundary::EnforcementPoint;
    use id_core::boundary_surfaces::FrameworkOutcome;
    use id_core::residency::EvidenceVerifier;
    use id_directory::directory::BoundaryGate;

    use super::ServerBoundaryGate;
    use crate::boundary_evidence::AppEvidenceVerifier;
    use crate::server_config::ServerConfig;

    fn verifier() -> Arc<dyn EvidenceVerifier + Send + Sync> {
        Arc::new(AppEvidenceVerifier::new(|| 0))
    }

    fn gate(config_text: &str) -> ServerBoundaryGate {
        let config = ServerConfig::parse(config_text).unwrap();
        ServerBoundaryGate::without_hop_evidence(
            config.boundaries().clone(),
            "main".to_owned(),
            None,
            verifier(),
        )
    }

    fn boundary(enforcement: &str) -> String {
        format!(
            "[[boundaries]]\ncapability = \"residency\"\nscope = {{ kind = \"server\", id = \"main\" }}\n\
             subject_class = \"user\"\n\
             evidence_req = {{ minimum_assurance = \"provider_verified\", accepted_sources = [] }}\n\
             enforcement = [{enforcement}]\nfailure = \"deny\"\nenabled = true\n\
             version = 1\ndeclared_by = \"operator\"\ndeclared_at = 1763424000\n"
        )
    }

    #[test]
    fn no_declarations_allow_every_point() {
        // Oracle (#7787): with nothing enabled, the gate is the no-op
        // allow — off is the only implicit default.
        let gate = gate("");
        assert_eq!(
            gate.check(EnforcementPoint::AccountCreation, "ann"),
            FrameworkOutcome::Allow
        );
        assert_eq!(
            gate.check(EnforcementPoint::Login, "ann"),
            FrameworkOutcome::Allow
        );
    }

    #[test]
    fn enabled_creation_boundary_denies_creation_and_not_login() {
        // Oracle (#7787): an enabled deny boundary naming only account
        // creation refuses creation through the shared evaluator (the
        // gate offers no hop evidence, so the check fails closed) and
        // leaves login outside its coverage.
        let gate = gate(&boundary("\"account_creation\""));
        let outcome = gate.check(EnforcementPoint::AccountCreation, "ann");
        assert!(
            matches!(outcome, FrameworkOutcome::Deny { .. }),
            "{outcome:?}"
        );
        assert_eq!(
            gate.check(EnforcementPoint::Login, "ann"),
            FrameworkOutcome::Allow
        );
    }

    #[test]
    fn enabled_login_boundary_denies_login_and_not_creation() {
        // Oracle (#7787): coverage is per enforcement point, in the
        // other direction as well.
        let gate = gate(&boundary("\"login\""));
        assert_eq!(
            gate.check(EnforcementPoint::AccountCreation, "ann"),
            FrameworkOutcome::Allow
        );
        let outcome = gate.check(EnforcementPoint::Login, "ann");
        assert!(
            matches!(outcome, FrameworkOutcome::Deny { .. }),
            "{outcome:?}"
        );
    }
}
