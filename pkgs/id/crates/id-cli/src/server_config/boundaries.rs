//! Boundary declarations and the residency policy in the server config (#8851).
//!
//! The design record `doc/2026-11-17T21-55-00Z_design_id8851_boundary_inputs.md`
//! (§1, §4, §5) puts boundary declarations in the operator's server config, so
//! this module adds two optional parts to it and the loader that turns them into
//! what [`id_core::boundary_surfaces::check_surface`] takes.
//!
//! ```toml
//! [residency]                      # absent: no trust roots, so an enabled declaration admits nothing
//! trust_roots = ["<64 hex>"]       # issuers trusted for evidence and peer claims (one list)
//! required = ["transport", "storage"]   # optional: the planes a path must prove (default all)
//!
//! [[residency.planes]]             # an allow-list for one plane; an omitted plane permits nothing
//! plane = "storage"
//! jurisdictions = ["us"]
//! resources = ["account-store"]
//! evidence_sources = ["self-attested-hop"]
//!
//! [[boundaries]]                   # one immutable declaration; the file lists versions in order
//! capability = "residency"
//! scope = { kind = "server", id = "main" }   # server, world, or account (a user key)
//! subject_class = "user"
//! evidence_req = { minimum_assurance = "provider_verified", accepted_sources = [] }
//! enforcement = ["account_creation", "login"]
//! failure = "deny"
//! enabled = true
//! version = 1
//! declared_by = "operator"
//! declared_at = 1763424000
//! ```
//!
//! Every entry is checked by [`BoundaryDeclaration::new`], so an enabled entry
//! with no enforcement point is a load error, never a silently ignored entry.
//! Several entries for one scope and capability are that boundary's versions:
//! each must supersede the one before it ([`BoundaryDeclaration::ensure_supersedes`]),
//! and the last is in force.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::Deserialize;

use id_core::boundary::{
    BoundaryDeclaration, BoundaryDeclarationBody, CapabilityKind, EffectiveBoundary,
    EffectiveConflict, ScopeRef,
};
use id_core::residency::{
    EvidenceVerifier, Jurisdiction, Plane, PlanePolicy, Policy, ResourceId, Scope,
};

use super::ConfigError;

/// The `[residency]` table as written.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawResidency {
    #[serde(default)]
    trust_roots: Vec<String>,
    required: Option<Vec<String>>,
    #[serde(default)]
    planes: Vec<RawPlane>,
}

/// One `[[residency.planes]]` row as written.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPlane {
    plane: String,
    #[serde(default)]
    jurisdictions: Vec<String>,
    #[serde(default)]
    resources: Vec<String>,
    #[serde(default)]
    evidence_sources: Vec<String>,
}

/// The boundary declarations in force and the residency policy, checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Boundaries {
    /// The last declaration of each scope and capability: the one in force.
    in_force: BTreeMap<(ScopeRef, CapabilityKind), BoundaryDeclaration>,
    /// The residency policy, including the operator's trust roots.
    policy: Policy,
}

impl Default for Boundaries {
    /// No declarations and no trust roots: every check is the no-op allow.
    fn default() -> Self {
        Self {
            in_force: BTreeMap::new(),
            policy: Policy::new(Vec::new()),
        }
    }
}

/// The scopes in play at one call site (design §1). A call site with no world or
/// account in play passes `None` for it rather than inventing one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScopesInPlay<'a> {
    /// The server's own name.
    pub server: &'a str,
    /// The world the action touches, if any.
    pub world: Option<&'a str>,
    /// The account key the action is for, if any.
    pub account: Option<&'a str>,
}

/// What one surface check receives (design §4): everything
/// [`id_core::boundary_surfaces::check_surface`] takes except the action's own
/// point, actor, subject, claims and clock.
#[derive(Clone)]
pub struct BoundaryInputs {
    /// The intersection of the enabled declarations in play.
    pub effective: EffectiveBoundary,
    /// The version of the declaration the check is charged to. Zero when no
    /// declaration is enabled, which `check_surface` never reads.
    pub version: u64,
    /// The scope the check is charged to: the narrowest scope with an enabled
    /// declaration, or the narrowest scope in play when none is enabled.
    pub scope_ref: ScopeRef,
    /// The residency scope, carrying the operator's policy and trust roots.
    pub scope: Scope,
    /// The app's one evidence verifier.
    pub verifier: Arc<dyn EvidenceVerifier + Send + Sync>,
}

impl std::fmt::Debug for BoundaryInputs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoundaryInputs")
            .field("effective", &self.effective)
            .field("version", &self.version)
            .field("scope_ref", &self.scope_ref)
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl Boundaries {
    /// The residency policy the config sets.
    #[must_use]
    pub const fn policy(&self) -> &Policy {
        &self.policy
    }

    /// The operator's trust roots: one list for evidence issuers and for peers
    /// whose claims are trusted (design §1).
    #[must_use]
    pub const fn trust_roots(&self) -> &BTreeSet<String> {
        &self.policy.trust_roots
    }

    /// The declaration in force for one scope and capability, if the config
    /// declares one.
    #[must_use]
    pub fn declaration(
        &self,
        scope: &ScopeRef,
        capability: CapabilityKind,
    ) -> Option<&BoundaryDeclaration> {
        self.in_force.get(&(scope.clone(), capability))
    }

    /// The inputs for one check of `capability` with `scopes` in play: the
    /// effective boundary composed by [`id_core::boundary::effective`], so a
    /// narrower world or account declaration can never be widened by a broader
    /// one, and the version and scope it is charged to.
    ///
    /// # Errors
    ///
    /// Returns the composition's [`EffectiveConflict`] when the enabled
    /// declarations in play share no enforcement surface or evidence source.
    pub fn inputs(
        &self,
        capability: CapabilityKind,
        scopes: ScopesInPlay<'_>,
        verifier: Arc<dyn EvidenceVerifier + Send + Sync>,
    ) -> Result<BoundaryInputs, EffectiveConflict> {
        let server_ref = ScopeRef::Server(scopes.server.to_owned());
        let world_ref = scopes.world.map(|world| ScopeRef::World(world.to_owned()));
        let account_ref = scopes
            .account
            .map(|account| ScopeRef::Account(account.to_owned()));

        let server = self.declaration(&server_ref, capability);
        let world = world_ref
            .as_ref()
            .and_then(|scope| self.declaration(scope, capability));
        let account = account_ref
            .as_ref()
            .and_then(|scope| self.declaration(scope, capability));
        let effective = id_core::boundary::effective(server, world, account)?;

        let charged = [account, world, server]
            .into_iter()
            .flatten()
            .find(|declaration| declaration.is_enabled());
        let (scope_ref, version) = match charged {
            Some(declaration) => (declaration.body.scope.clone(), declaration.body.version),
            None => (account_ref.or(world_ref).unwrap_or(server_ref), 0),
        };
        Ok(BoundaryInputs {
            effective,
            version,
            scope_ref,
            scope: Scope::Scoped(self.policy.clone()),
            verifier,
        })
    }
}

/// Checks the `[[boundaries]]` entries and the `[residency]` table.
/// `account_known` says whether an account key names a user the config declares.
pub(super) fn parse(
    raw_boundaries: &[BoundaryDeclarationBody],
    raw_residency: Option<&RawResidency>,
    account_known: impl Fn(&str) -> bool,
) -> Result<Boundaries, ConfigError> {
    let mut in_force: BTreeMap<(ScopeRef, CapabilityKind), BoundaryDeclaration> = BTreeMap::new();
    for (index, body) in raw_boundaries.iter().enumerate() {
        let place = format!("[[boundaries]] entry {}", index + 1);
        if let ScopeRef::Account(key) = &body.scope
            && !account_known(key)
        {
            return Err(ConfigError::Boundary {
                place,
                reason: format!("account scope `{key}` is not a user key this config declares"),
            });
        }
        let declaration =
            BoundaryDeclaration::new(body.clone()).map_err(|error| ConfigError::Boundary {
                place: place.clone(),
                reason: error.to_string(),
            })?;
        let key = (declaration.body.scope.clone(), declaration.body.capability);
        if let Some(previous) = in_force.get(&key) {
            declaration
                .ensure_supersedes(previous)
                .map_err(|error| ConfigError::Boundary {
                    place,
                    reason: format!(
                        "{error}: version {} follows version {} for the same scope and capability",
                        declaration.body.version, previous.body.version
                    ),
                })?;
        }
        in_force.insert(key, declaration);
    }
    let policy = match raw_residency {
        None => Policy::new(Vec::new()),
        Some(raw) => residency_policy(raw)?,
    };
    Ok(Boundaries { in_force, policy })
}

fn residency_policy(raw: &RawResidency) -> Result<Policy, ConfigError> {
    let mut trust_roots = BTreeSet::new();
    for root in &raw.trust_roots {
        let normalized = root.trim().to_ascii_lowercase();
        if crate::boundary_evidence::issuer_key(&normalized).is_none() {
            return Err(ConfigError::Boundary {
                place: "[residency] trust_roots".to_owned(),
                reason: format!("`{root}` is not a 64-hex Ed25519 public key"),
            });
        }
        if !trust_roots.insert(normalized) {
            return Err(ConfigError::Boundary {
                place: "[residency] trust_roots".to_owned(),
                reason: format!("`{root}` is listed more than once"),
            });
        }
    }
    let mut policy = Policy::new(trust_roots);
    if let Some(required) = &raw.required {
        policy.required = required
            .iter()
            .map(|name| parse_plane(name, "[residency] required"))
            .collect::<Result<_, _>>()?;
        if policy.required.is_empty() {
            return Err(ConfigError::Boundary {
                place: "[residency] required".to_owned(),
                reason: "names no plane; omit it to require all four".to_owned(),
            });
        }
    }
    for row in &raw.planes {
        let plane = parse_plane(&row.plane, "a [[residency.planes]] row")?;
        let plane_policy = PlanePolicy {
            jurisdictions: row
                .jurisdictions
                .iter()
                .map(|j| Jurisdiction(j.clone()))
                .collect(),
            evidence_sources: row.evidence_sources.iter().cloned().collect(),
            resources: row
                .resources
                .iter()
                .map(|r| ResourceId(r.clone()))
                .collect(),
        };
        if policy.planes.insert(plane, plane_policy).is_some() {
            return Err(ConfigError::Boundary {
                place: "[[residency.planes]]".to_owned(),
                reason: format!("plane `{}` has more than one row", row.plane),
            });
        }
    }
    Ok(policy)
}

fn parse_plane(name: &str, place: &str) -> Result<Plane, ConfigError> {
    crate::boundary_evidence::plane_from_name(name).ok_or_else(|| ConfigError::Boundary {
        place: place.to_owned(),
        reason: format!("`{name}` is not a plane; use storage, processing, transport or metadata"),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::Arc;

    use ed25519_dalek::SigningKey;
    use id_core::boundary::{CapabilityKind, EnforcementPoint, ScopeRef};
    use id_core::boundary_surfaces::{FrameworkOutcome, check_surface};
    use id_core::residency::Plane;

    use super::ScopesInPlay;
    use crate::boundary_evidence::AppEvidenceVerifier;
    use crate::server_config::{ConfigError, ServerConfig};

    fn key_hex(seed: u8) -> String {
        crate::boundary_evidence::hex(
            SigningKey::from_bytes(&[seed; 32])
                .verifying_key()
                .as_bytes(),
        )
    }

    fn users() -> String {
        format!("[[users]]\nkey = \"ann\"\nid = \"{}\"\n\n", key_hex(1))
    }

    fn boundary(scope: &str, version: u64, enforcement: &str, enabled: bool) -> String {
        let (kind, id) = scope.split_once(':').unwrap();
        format!(
            "[[boundaries]]\ncapability = \"residency\"\nscope = {{ kind = \"{kind}\", id = \"{id}\" }}\n\
             subject_class = \"user\"\n\
             evidence_req = {{ minimum_assurance = \"provider_verified\", accepted_sources = [] }}\n\
             enforcement = [{enforcement}]\nfailure = \"deny\"\nenabled = {enabled}\n\
             version = {version}\ndeclared_by = \"operator\"\ndeclared_at = 1763424000\n\n"
        )
    }

    fn verifier() -> Arc<AppEvidenceVerifier> {
        Arc::new(AppEvidenceVerifier::new(|| 0))
    }

    const MAIN: ScopesInPlay<'static> = ScopesInPlay {
        server: "main",
        world: None,
        account: None,
    };

    fn boundary_error(text: &str) -> (String, String) {
        match ServerConfig::parse(text) {
            Err(ConfigError::Boundary { place, reason }) => (place, reason),
            other => panic!("expected a boundary error, got {other:?}"),
        }
    }

    #[test]
    fn absent_declaration_is_the_no_op_allow() {
        let config = ServerConfig::parse("").unwrap();
        let inputs = config
            .boundaries()
            .inputs(CapabilityKind::Residency, MAIN, verifier())
            .unwrap();
        assert_eq!(inputs.effective.capability, None);
        assert_eq!(inputs.version, 0);
        assert_eq!(inputs.scope_ref, ScopeRef::Server("main".to_owned()));
        let check = check_surface(
            EnforcementPoint::Login,
            &inputs.effective,
            inputs.version,
            &inputs.scope_ref,
            &id_core::boundary::ActorId("ann".to_owned()),
            "ann",
            &inputs.scope,
            &[],
            1,
            inputs.verifier.as_ref(),
        );
        assert_eq!(check.outcome, FrameworkOutcome::Allow);
        assert!(check.log_entry.is_none());
    }

    #[test]
    fn a_disabled_declaration_is_also_the_no_op_allow() {
        let text = boundary("server:main", 1, "", false);
        let config = ServerConfig::parse(&text).unwrap();
        let inputs = config
            .boundaries()
            .inputs(CapabilityKind::Residency, MAIN, verifier())
            .unwrap();
        assert_eq!(inputs.effective.capability, None);
        assert_eq!(inputs.version, 0);
    }

    #[test]
    fn declarations_compose_per_scope_and_charge_the_narrowest() {
        let text = format!(
            "{}{}{}{}",
            users(),
            boundary("server:main", 1, "\"account_creation\", \"login\"", true),
            boundary("world:w1", 3, "\"login\"", true),
            boundary("account:ann", 2, "\"login\", \"blob_storage\"", true),
        );
        let config = ServerConfig::parse(&text).unwrap();
        let boundaries = config.boundaries();

        let server_only = boundaries
            .inputs(CapabilityKind::Residency, MAIN, verifier())
            .unwrap();
        assert_eq!(
            server_only.effective.enforcement,
            [EnforcementPoint::AccountCreation, EnforcementPoint::Login]
                .into_iter()
                .collect()
        );
        assert_eq!(server_only.version, 1);
        assert_eq!(server_only.scope_ref, ScopeRef::Server("main".to_owned()));

        let all = boundaries
            .inputs(
                CapabilityKind::Residency,
                ScopesInPlay {
                    server: "main",
                    world: Some("w1"),
                    account: Some("ann"),
                },
                verifier(),
            )
            .unwrap();
        assert_eq!(
            all.effective.enforcement,
            std::iter::once(EnforcementPoint::Login).collect()
        );
        assert_eq!(all.version, 2);
        assert_eq!(all.scope_ref, ScopeRef::Account("ann".to_owned()));

        let other_world = boundaries
            .inputs(
                CapabilityKind::Residency,
                ScopesInPlay {
                    server: "main",
                    world: Some("w2"),
                    account: None,
                },
                verifier(),
            )
            .unwrap();
        assert_eq!(other_world.version, 1);
        assert_eq!(other_world.scope_ref, ScopeRef::Server("main".to_owned()));
    }

    #[test]
    fn the_last_version_is_in_force() {
        let text = format!(
            "{}{}",
            boundary("server:main", 1, "\"login\"", true),
            boundary("server:main", 2, "\"account_creation\"", true),
        );
        let config = ServerConfig::parse(&text).unwrap();
        let inputs = config
            .boundaries()
            .inputs(CapabilityKind::Residency, MAIN, verifier())
            .unwrap();
        assert_eq!(inputs.version, 2);
        assert_eq!(
            inputs.effective.enforcement,
            std::iter::once(EnforcementPoint::AccountCreation).collect()
        );
    }

    #[test]
    fn a_version_that_does_not_supersede_is_refused() {
        let text = format!(
            "{}{}",
            boundary("server:main", 2, "\"login\"", true),
            boundary("server:main", 2, "\"login\"", true),
        );
        let (place, reason) = boundary_error(&text);
        assert_eq!(place, "[[boundaries]] entry 2");
        assert!(reason.contains("version 2 follows version 2"), "{reason}");
    }

    #[test]
    fn enabled_without_enforcement_is_refused() {
        let (place, reason) = boundary_error(&boundary("server:main", 1, "", true));
        assert_eq!(place, "[[boundaries]] entry 1");
        assert!(reason.contains("no enforcement point"), "{reason}");
    }

    #[test]
    fn an_unknown_account_is_refused() {
        let (_, reason) = boundary_error(&boundary("account:bo", 1, "\"login\"", true));
        assert!(reason.contains("`bo`"), "{reason}");
    }

    #[test]
    fn an_unknown_field_is_refused() {
        let mut text = boundary("server:main", 1, "\"login\"", true);
        text.push_str("surprise = 1\n");
        assert!(matches!(
            ServerConfig::parse(&text),
            Err(ConfigError::Parse(_))
        ));
    }

    #[test]
    fn residency_table_is_parsed() {
        let root = key_hex(9).to_ascii_uppercase();
        let text = format!(
            "[residency]\ntrust_roots = [\"{root}\"]\nrequired = [\"transport\", \"storage\"]\n\n\
             [[residency.planes]]\nplane = \"storage\"\njurisdictions = [\"us\"]\n\
             resources = [\"r\"]\nevidence_sources = [\"attestor\"]\n"
        );
        let config = ServerConfig::parse(&text).unwrap();
        let policy = config.boundaries().policy();
        assert_eq!(
            config.boundaries().trust_roots(),
            &std::iter::once(key_hex(9)).collect()
        );
        assert_eq!(
            policy.required,
            [Plane::Transport, Plane::Storage].into_iter().collect()
        );
        let storage = &policy.planes[&Plane::Storage];
        assert!(storage.evidence_sources.contains("attestor"));
        assert!(!policy.planes.contains_key(&Plane::Transport));
    }

    #[test]
    fn bad_residency_tables_are_refused() {
        let (place, _) = boundary_error("[residency]\ntrust_roots = [\"nope\"]\n");
        assert_eq!(place, "[residency] trust_roots");

        let root = key_hex(9);
        let (_, reason) = boundary_error(&format!(
            "[residency]\ntrust_roots = [\"{root}\", \"{root}\"]\n"
        ));
        assert!(reason.contains("more than once"), "{reason}");

        let (_, reason) = boundary_error("[residency]\nrequired = [\"disk\"]\n");
        assert!(reason.contains("`disk` is not a plane"), "{reason}");

        let (place, _) = boundary_error("[residency]\nrequired = []\n");
        assert_eq!(place, "[residency] required");

        let (_, reason) = boundary_error(
            "[residency]\n[[residency.planes]]\nplane = \"storage\"\n\
             [[residency.planes]]\nplane = \"storage\"\n",
        );
        assert!(reason.contains("more than one row"), "{reason}");
    }
}
