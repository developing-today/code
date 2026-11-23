//! Role definitions as first-class, versioned world configuration —
//! v2 agents-as-citizens step 1 (#8299, parent #6301).
//!
//! Oracle: `doc/2026-11-10_design_id6301_agents_as_citizens_v2_outline.md`
//! §2.1 (roles as world-owned configuration objects) and §4 decisions 1–2
//! (role configuration format and versioning; grant/revoke surface),
//! extending the v1 surfaces of [`crate::agent_delegation`] (role
//! binding at join, v1 §5) and [`crate::agent_policy`] (the policy
//! point, v1 §6). In v1 a role is only a binding the host checks at
//! join; in v2 the role becomes a world-owned configuration object:
//!
//! - A [`RoleDefinition`] — capability ceiling, quota class,
//!   delegation-lifetime bounds, review policy — is serde-shaped so it
//!   can sit in world configuration like every other world setting,
//!   and is versioned by the [`RoleRegistry`]: an edit replaces the
//!   definition and bumps its version.
//! - Grants are explicit world state — *this agent key holds this role
//!   in this world, granted by this owner, at this time* — enumerable
//!   by the owner ([`RoleRegistry::list_grants`]) and revocable
//!   independently of delegation expiry: revocation ends the grant,
//!   expiry only ends a session's proof of it.
//! - Every create, edit, grant and revoke is an audited world event
//!   ([`RoleConfigEvent`]) — refusals included, never sampled away,
//!   in the v1 §7 spirit.
//! - Starter roles ([`starter_roles`]) ship as ordinary definitions a
//!   world can copy and edit — defaults, not a privileged tier.
//! - A changed ceiling applies at the next frame check, not the next
//!   join: the host applies it to a live session through
//!   [`crate::agent_policy::PolicyPoint::refresh_ceiling`].

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use serde::{Deserialize, Serialize};

use crate::agent_delegation::{AgentId, MAX_LIFETIME_SECS, MIN_LIFETIME_SECS};
use crate::agent_policy::Quota;
use crate::world_caps::MAX_NAME_BYTES;
use crate::world_scopes::WorldScopes;

/// Audit event kind for creating a role definition.
pub const EVENT_ROLE_CREATE: &str = "role.create";
/// Audit event kind for editing a role definition (a versioned
/// world-config change).
pub const EVENT_ROLE_EDIT: &str = "role.edit";
/// Audit event kind for granting a role to an agent.
pub const EVENT_ROLE_GRANT: &str = "role.grant";
/// Audit event kind for revoking a role from an agent.
pub const EVENT_ROLE_REVOKE: &str = "role.revoke";

/// How a role's budget replenishes (§2.3): every `every_secs`, the
/// per-(agent, world) ledger regains `amount`. `None` in
/// [`QuotaClass::replenishment`] means the budget never replenishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Replenishment {
    /// Seconds between replenishments; must be positive.
    pub every_secs: u64,
    /// Amount regained per replenishment, per axis.
    pub amount: Quota,
}

/// A role's quota class (§2.1, §2.3): the session budget granted with
/// the role and how it replenishes.
///
/// The per-(agent, world) ledger that
/// consumes this class is the v2 step-3 subject; the class itself is
/// part of the role definition so an edit can move an agent's budget.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct QuotaClass {
    /// Budget granted with the role, per axis.
    pub budget: Quota,
    /// Replenishment cadence, or `None` for a never-replenishing
    /// budget.
    pub replenishment: Option<Replenishment>,
}

/// A role's review policy (§2.5): whether above-ceiling requests queue
/// for one-shot owner approval, and digest cadence.
///
/// How many repeated refusals of the
/// same shape surface as a digest. Both are hooks the step-4 review
/// tooling consumes; the policy point itself only reads the ceiling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ReviewPolicy {
    /// Whether an above-ceiling request queues for one-shot owner
    /// approval instead of being only refused.
    pub queue_above_ceiling: bool,
    /// Refusals of one shape before the review view digests them into
    /// one line; `0` never digests.
    pub digest_every_refusals: u64,
}

/// A role definition (§2.1): the versioned world-configuration object
/// a grant refers to by name.
///
/// Serde-shaped so it lives in world
/// configuration; [`RoleDefinition::validate`] is the authority on
/// whether a loaded definition is acceptable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct RoleDefinition {
    /// Stable role name, as carried in hosted-edge scope strings
    /// (v1 §9.4). Non-empty, at most [`MAX_NAME_BYTES`] bytes.
    pub name: String,
    /// The most any delegation under this role may ever carry (v1
    /// §5.2). Only recognized scope bits.
    pub ceiling: WorldScopes,
    /// Shortest delegation lifetime a grant under this role may set,
    /// within the v1 §9.2 range.
    pub lifetime_min_secs: u64,
    /// Longest delegation lifetime a grant under this role may set,
    /// within the v1 §9.2 range.
    pub lifetime_max_secs: u64,
    /// Quota class granted with the role.
    pub quota_class: QuotaClass,
    /// Review policy for agents holding the role.
    pub review: ReviewPolicy,
}

impl RoleDefinition {
    /// Validate this definition as world configuration (§2.1): the
    /// name is non-empty and bounded, the ceiling names only
    /// recognized scope bits, and the lifetime bounds lie inside the
    /// v1 §9.2 range with min ≤ max.
    ///
    /// # Errors
    ///
    /// Returns the specific [`RoleConfigError`] for the first failed
    /// check.
    pub fn validate(&self) -> Result<(), RoleConfigError> {
        if self.name.is_empty() || self.name.len() > MAX_NAME_BYTES {
            return Err(RoleConfigError::BadName);
        }
        if WorldScopes::from_bits(self.ceiling.bits()).is_none() {
            return Err(RoleConfigError::BadCeilingBits);
        }
        if !(MIN_LIFETIME_SECS..=MAX_LIFETIME_SECS).contains(&self.lifetime_min_secs)
            || !(MIN_LIFETIME_SECS..=MAX_LIFETIME_SECS).contains(&self.lifetime_max_secs)
            || self.lifetime_min_secs > self.lifetime_max_secs
        {
            return Err(RoleConfigError::BadLifetimeBounds);
        }
        if let Some(replenishment) = self.quota_class.replenishment
            && replenishment.every_secs == 0
        {
            return Err(RoleConfigError::BadReplenishment);
        }
        Ok(())
    }

    /// Whether a delegation lifetime of `secs` is inside this role's
    /// bounds.
    #[must_use]
    pub const fn lifetime_within_bounds(&self, secs: u64) -> bool {
        self.lifetime_min_secs <= secs && secs <= self.lifetime_max_secs
    }
}

/// The starter roles (§2.1): observer, participant, archivist-class
/// editor.
///
/// Ordinary definitions a world can copy and edit through
/// the same [`RoleRegistry::upsert`] path as any hand-written
/// definition — defaults, not a privileged tier. Object-class
/// authority stays the v1 §6.1 policy-point layer the owner widens
/// out-of-band; a role's `ceiling` here is the delegation ceiling.
#[must_use]
pub fn starter_roles() -> Vec<RoleDefinition> {
    vec![
        RoleDefinition {
            name: "observer".to_owned(),
            ceiling: WorldScopes::JOIN.union(WorldScopes::HISTORY),
            lifetime_min_secs: MIN_LIFETIME_SECS,
            lifetime_max_secs: 3_600,
            quota_class: QuotaClass {
                budget: Quota {
                    frames: 500,
                    tool_calls: 500,
                    program_invocations: 0,
                },
                replenishment: None,
            },
            review: ReviewPolicy {
                queue_above_ceiling: false,
                digest_every_refusals: 10,
            },
        },
        RoleDefinition {
            name: "participant".to_owned(),
            ceiling: WorldScopes::GUEST,
            lifetime_min_secs: MIN_LIFETIME_SECS,
            lifetime_max_secs: MAX_LIFETIME_SECS,
            quota_class: QuotaClass {
                budget: Quota {
                    frames: 10_000,
                    tool_calls: 10_000,
                    program_invocations: 1_000,
                },
                replenishment: Some(Replenishment {
                    every_secs: 3_600,
                    amount: Quota {
                        frames: 10_000,
                        tool_calls: 10_000,
                        program_invocations: 1_000,
                    },
                }),
            },
            review: ReviewPolicy {
                queue_above_ceiling: false,
                digest_every_refusals: 10,
            },
        },
        RoleDefinition {
            name: "archivist-editor".to_owned(),
            ceiling: WorldScopes::GUEST,
            lifetime_min_secs: MIN_LIFETIME_SECS,
            lifetime_max_secs: MAX_LIFETIME_SECS,
            quota_class: QuotaClass {
                budget: Quota {
                    frames: 50_000,
                    tool_calls: 50_000,
                    program_invocations: 10_000,
                },
                replenishment: Some(Replenishment {
                    every_secs: 3_600,
                    amount: Quota {
                        frames: 50_000,
                        tool_calls: 50_000,
                        program_invocations: 10_000,
                    },
                }),
            },
            review: ReviewPolicy {
                queue_above_ceiling: true,
                digest_every_refusals: 5,
            },
        },
    ]
}

/// A role definition in force, with its version (§4 decision 1): an
/// edit replaces the definition and bumps the version; the grant and
/// audit surfaces refer to the name and observe the current record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleRecord {
    /// The definition in force.
    pub definition: RoleDefinition,
    /// Version of this definition: 1 at create, +1 per edit.
    pub version: u64,
}

/// One agent's grant of a role in this world (§2.1): explicit,
/// owner-enumerable world state, independent of delegation expiry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleGrant {
    /// Role name (see [`RoleDefinition::name`]).
    pub role: String,
    /// The agent key holding the role.
    pub agent: AgentId,
    /// Who granted it (owner key hex or label, as supplied).
    pub granted_by: String,
    /// When it was granted, unix seconds.
    pub granted_at: u64,
}

/// One audited world event for a role-configuration operation (§2.1,
/// §4 decision 2). Every create, edit, grant and revoke produces
/// exactly one event — refusals included, never sampled away.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleConfigEvent {
    /// Event kind: one of [`EVENT_ROLE_CREATE`],
    /// [`EVENT_ROLE_EDIT`], [`EVENT_ROLE_GRANT`],
    /// [`EVENT_ROLE_REVOKE`].
    pub kind: &'static str,
    /// Who performed the operation (owner key hex or label, as
    /// supplied).
    pub actor: String,
    /// World whose configuration changed.
    pub world: String,
    /// Role the operation names.
    pub role: String,
    /// Agent key the operation names (hex), when it names one.
    pub agent: Option<String>,
    /// `executed`, or `refused:<code>` with the stable
    /// [`RoleConfigError::code`].
    pub outcome: String,
    /// Host time of the event, unix seconds.
    pub at: u64,
}

/// The result of one role-configuration operation: the decision, plus
/// the audit event recorded for it either way — the shape callers
/// journal as a world event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleConfigOutcome {
    /// `Ok(())` when the operation changed the registry, or the
    /// specific refusal.
    pub decision: Result<(), RoleConfigError>,
    /// The audit event recorded for this operation, whatever the
    /// outcome.
    pub event: RoleConfigEvent,
}

/// Why a role-configuration operation was refused. Each variant maps
/// to a stable machine-readable [`RoleConfigError::code`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoleConfigError {
    /// The role name is empty or over the [`MAX_NAME_BYTES`] bound.
    BadName,
    /// The ceiling names scope bits the vocabulary does not
    /// recognize.
    BadCeilingBits,
    /// The lifetime bounds fall outside the v1 §9.2 range, or min
    /// exceeds max.
    BadLifetimeBounds,
    /// A replenishment cadence of zero seconds.
    BadReplenishment,
    /// The named role definition does not exist in this world's
    /// configuration.
    UnknownRole,
    /// No grant of that role to that agent exists in this world.
    UnknownGrant,
    /// The agent already holds that role in this world.
    AlreadyGranted,
}

impl RoleConfigError {
    /// Stable machine-readable refusal code, as journaled in the
    /// event's outcome.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::BadName => "bad_name",
            Self::BadCeilingBits => "bad_ceiling_bits",
            Self::BadLifetimeBounds => "bad_lifetime_bounds",
            Self::BadReplenishment => "bad_replenishment",
            Self::UnknownRole => "unknown_role",
            Self::UnknownGrant => "unknown_grant",
            Self::AlreadyGranted => "already_granted",
        }
    }
}

/// One world's role configuration (§2.1): the versioned role
/// definitions plus the explicit grant list.
///
/// An audit event for every mutation — refusal included. Role definitions are
/// world-owned; one registry serves one world.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleRegistry {
    world: String,
    roles: BTreeMap<String, RoleRecord>,
    grants: BTreeMap<(String, String), RoleGrant>,
    events: Vec<RoleConfigEvent>,
}

impl RoleRegistry {
    /// An empty registry for `world`.
    #[must_use]
    pub fn new(world: impl Into<String>) -> Self {
        Self {
            world: world.into(),
            roles: BTreeMap::new(),
            grants: BTreeMap::new(),
            events: Vec::new(),
        }
    }

    /// The world this registry configures.
    #[must_use]
    pub fn world(&self) -> &str {
        &self.world
    }

    /// Install the starter roles as ordinary definitions (§2.1):
    /// each goes through the same [`Self::upsert`] path as a
    /// hand-written definition, so a world can copy and edit them
    /// afterwards with no privileged tier.
    pub fn install_starters(&mut self, actor: &str, now: u64) -> Vec<RoleConfigOutcome> {
        starter_roles()
            .into_iter()
            .map(|definition| self.upsert(definition, actor, now))
            .collect()
    }

    /// Create or edit a role definition (§4 decision 1). Creating
    /// records [`EVENT_ROLE_CREATE`] at version 1; editing an existing
    /// definition replaces it and bumps its version, recording
    /// [`EVENT_ROLE_EDIT`]. An invalid definition changes nothing and
    /// is still audited.
    pub fn upsert(
        &mut self,
        definition: RoleDefinition,
        actor: &str,
        now: u64,
    ) -> RoleConfigOutcome {
        let kind = if self.roles.contains_key(&definition.name) {
            EVENT_ROLE_EDIT
        } else {
            EVENT_ROLE_CREATE
        };
        let name = definition.name.clone();
        if let Err(refusal) = definition.validate() {
            let recorded = RoleConfigEvent {
                kind,
                actor: actor.to_owned(),
                world: self.world.clone(),
                role: name,
                agent: None,
                outcome: format!("refused:{}", refusal.code()),
                at: now,
            };
            self.events.push(recorded.clone());
            return RoleConfigOutcome {
                decision: Err(refusal),
                event: recorded,
            };
        }
        let version = self
            .roles
            .get(&name)
            .map_or(1, |record| record.version.saturating_add(1));
        self.roles.insert(
            name.clone(),
            RoleRecord {
                definition,
                version,
            },
        );
        let recorded = RoleConfigEvent {
            kind,
            actor: actor.to_owned(),
            world: self.world.clone(),
            role: name,
            agent: None,
            outcome: "executed".to_owned(),
            at: now,
        };
        self.events.push(recorded.clone());
        RoleConfigOutcome {
            decision: Ok(()),
            event: recorded,
        }
    }

    /// Grant `role` to `agent` in this world (§2.1): explicit,
    /// owner-enumerable state, recorded by `actor` at `now`.
    pub fn grant(
        &mut self,
        role: &str,
        agent: AgentId,
        actor: &str,
        now: u64,
    ) -> RoleConfigOutcome {
        let decision = if self.roles.contains_key(role) {
            match self.grants.entry((role.to_owned(), agent.to_hex())) {
                Entry::Occupied(_) => Err(RoleConfigError::AlreadyGranted),
                Entry::Vacant(vacant) => {
                    vacant.insert(RoleGrant {
                        role: role.to_owned(),
                        agent,
                        granted_by: actor.to_owned(),
                        granted_at: now,
                    });
                    Ok(())
                }
            }
        } else {
            Err(RoleConfigError::UnknownRole)
        };
        let event = self.record_event(EVENT_ROLE_GRANT, role, Some(agent), decision, actor, now);
        RoleConfigOutcome { decision, event }
    }

    /// Revoke `role` from `agent` in this world (§2.1): revocation ends
    /// the grant; delegation expiry only ends a session's proof of it.
    pub fn revoke(
        &mut self,
        role: &str,
        agent: AgentId,
        actor: &str,
        now: u64,
    ) -> RoleConfigOutcome {
        let decision = if self
            .grants
            .remove(&(role.to_owned(), agent.to_hex()))
            .is_none()
        {
            Err(RoleConfigError::UnknownGrant)
        } else {
            Ok(())
        };
        let event = self.record_event(EVENT_ROLE_REVOKE, role, Some(agent), decision, actor, now);
        RoleConfigOutcome { decision, event }
    }

    /// Every grant in this world, ordered by role then agent key —
    /// the owner's enumerable answer to "who holds what here" (§2.1).
    #[must_use]
    pub fn list_grants(&self) -> Vec<&RoleGrant> {
        self.grants.values().collect()
    }

    /// The definition in force for `role`, with its version.
    #[must_use]
    pub fn record(&self, role: &str) -> Option<&RoleRecord> {
        self.roles.get(role)
    }

    /// Every role definition in force, ordered by name.
    #[must_use]
    pub fn records(&self) -> Vec<&RoleRecord> {
        self.roles.values().collect()
    }

    /// Every audit event in order, refusals included.
    #[must_use]
    pub fn events(&self) -> &[RoleConfigEvent] {
        &self.events
    }

    /// Record the audit event for one operation, whatever the
    /// outcome: append it to the registry log and return it.
    fn record_event(
        &mut self,
        kind: &'static str,
        role: &str,
        agent: Option<AgentId>,
        decision: Result<(), RoleConfigError>,
        actor: &str,
        now: u64,
    ) -> RoleConfigEvent {
        let outcome = match decision {
            Ok(()) => "executed".to_owned(),
            Err(refusal) => format!("refused:{}", refusal.code()),
        };
        let recorded = RoleConfigEvent {
            kind,
            actor: actor.to_owned(),
            world: self.world.clone(),
            role: role.to_owned(),
            agent: agent.map(|id| id.to_hex()),
            outcome,
            at: now,
        };
        self.events.push(recorded.clone());
        recorded
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::agent_delegation::JoinGrant;
    use crate::agent_policy::{
        AgentAction, CEILING_CHAT_POST, CEILING_PRESENCE, Ceiling, PolicyPoint, PolicyRefusal,
    };
    use ed25519_dalek::SigningKey;

    const NOW: u64 = 1_700_000_000;
    const OWNER: &str = "owner";

    fn agent(seed: u8) -> AgentId {
        AgentId([seed; 32])
    }

    fn participant() -> RoleDefinition {
        starter_roles()
            .into_iter()
            .find(|definition| definition.name == "participant")
            .expect("starter participant exists")
    }

    /// Oracle §2.1 + §4 decision 1: the definition shape validates and
    /// is versioned — a bad name, bad ceiling bits or out-of-range
    /// lifetime bounds are refused (and audited); a create lands at
    /// version 1 and an edit bumps to version 2 as an audited
    /// role edit.
    #[test]
    fn definition_shape_validates_and_versions() {
        let mut registry = RoleRegistry::new("atlas");
        let good = participant();
        assert_eq!(good.validate(), Ok(()));

        // Bad shapes are refused before any mutation.
        let mut bad_name = good.clone();
        bad_name.name = String::new();
        assert_eq!(bad_name.validate(), Err(RoleConfigError::BadName));

        let mut bad_bounds = good.clone();
        bad_bounds.lifetime_min_secs = MIN_LIFETIME_SECS - 1;
        assert_eq!(
            bad_bounds.validate(),
            Err(RoleConfigError::BadLifetimeBounds)
        );

        let mut inverted = good.clone();
        inverted.lifetime_min_secs = 2 * MIN_LIFETIME_SECS;
        inverted.lifetime_max_secs = MIN_LIFETIME_SECS;
        assert_eq!(inverted.validate(), Err(RoleConfigError::BadLifetimeBounds));

        let mut bad_replenishment = good.clone();
        bad_replenishment.quota_class.replenishment = Some(Replenishment {
            every_secs: 0,
            amount: Quota::ZERO,
        });
        assert_eq!(
            bad_replenishment.validate(),
            Err(RoleConfigError::BadReplenishment)
        );

        // Create, then edit: versions 1 and 2, distinct audit kinds.
        let created = registry.upsert(good.clone(), OWNER, NOW);
        assert_eq!(created.decision, Ok(()));
        assert_eq!(created.event.kind, EVENT_ROLE_CREATE);
        assert_eq!(created.event.outcome, "executed");
        assert_eq!(registry.record("participant").unwrap().version, 1);

        let mut edited = good;
        edited.review.digest_every_refusals = 3;
        let updated = registry.upsert(edited, OWNER, NOW + 1);
        assert_eq!(updated.decision, Ok(()));
        assert_eq!(updated.event.kind, EVENT_ROLE_EDIT);
        assert_eq!(registry.record("participant").unwrap().version, 2);
        assert_eq!(
            registry
                .record("participant")
                .unwrap()
                .definition
                .review
                .digest_every_refusals,
            3
        );

        // A refused upsert is still audited and changes nothing.
        let before = registry.record("participant").unwrap().clone();
        let refused = registry.upsert(bad_name, OWNER, NOW + 2);
        assert_eq!(refused.decision, Err(RoleConfigError::BadName));
        assert_eq!(refused.event.outcome, "refused:bad_name");
        assert_eq!(&registry.record("participant").unwrap().clone(), &before);
    }

    /// Oracle §2.1 + §4 decision 2: grant, list and revoke exist; every
    /// grant/revoke — refusal included — is an audited world event.
    #[test]
    fn grant_list_revoke_are_audited() {
        let mut registry = RoleRegistry::new("atlas");
        registry.upsert(participant(), OWNER, NOW);

        // Granting an unknown role is refused and audited.
        let unknown = registry.grant("ghost", agent(9), OWNER, NOW);
        assert_eq!(unknown.decision, Err(RoleConfigError::UnknownRole));
        assert_eq!(unknown.event.outcome, "refused:unknown_role");

        // The happy path: grant, list, revoke.
        let granted = registry.grant("participant", agent(9), OWNER, NOW + 1);
        assert_eq!(granted.decision, Ok(()));
        assert_eq!(granted.event.kind, EVENT_ROLE_GRANT);
        let hex = agent(9).to_hex();
        assert_eq!(granted.event.agent.as_deref(), Some(hex.as_str()));

        let grants = registry.list_grants();
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].role, "participant");
        assert_eq!(grants[0].agent, agent(9));
        assert_eq!(grants[0].granted_by, OWNER);
        assert_eq!(grants[0].granted_at, NOW + 1);

        // Re-granting is refused; the grant stands.
        let again = registry.grant("participant", agent(9), OWNER, NOW + 2);
        assert_eq!(again.decision, Err(RoleConfigError::AlreadyGranted));

        let revoked = registry.revoke("participant", agent(9), OWNER, NOW + 3);
        assert_eq!(revoked.decision, Ok(()));
        assert_eq!(revoked.event.kind, EVENT_ROLE_REVOKE);
        assert!(registry.list_grants().is_empty());

        // Revoking a grant that is not held is refused and audited.
        let missing = registry.revoke("participant", agent(9), OWNER, NOW + 4);
        assert_eq!(missing.decision, Err(RoleConfigError::UnknownGrant));
        assert_eq!(missing.event.outcome, "refused:unknown_grant");

        // Two agents, one role: list is enumerable and ordered.
        registry
            .grant("participant", agent(1), OWNER, NOW + 5)
            .decision
            .expect("first agent granted");
        registry
            .grant("participant", agent(2), OWNER, NOW + 6)
            .decision
            .expect("second agent granted");
        let names: Vec<String> = registry
            .list_grants()
            .iter()
            .map(|grant| grant.agent.to_hex())
            .collect();
        assert_eq!(names, vec![agent(1).to_hex(), agent(2).to_hex()]);
        assert_eq!(registry.list_grants().len(), 2);

        // The registry log itself carries every operation above —
        // grants and revokes, refusals included — in order.
        let logged: Vec<(&str, &str)> = registry
            .events()
            .iter()
            .map(|event| (event.kind, event.outcome.as_str()))
            .collect();
        assert_eq!(
            logged,
            vec![
                (EVENT_ROLE_CREATE, "executed"),
                (EVENT_ROLE_GRANT, "refused:unknown_role"),
                (EVENT_ROLE_GRANT, "executed"),
                (EVENT_ROLE_GRANT, "refused:already_granted"),
                (EVENT_ROLE_REVOKE, "executed"),
                (EVENT_ROLE_REVOKE, "refused:unknown_grant"),
                (EVENT_ROLE_GRANT, "executed"),
                (EVENT_ROLE_GRANT, "executed"),
            ]
        );
    }

    /// Oracle §2.1: starter roles are plain definitions a world can
    /// copy and edit — they install through the ordinary upsert path
    /// at version 1, and an edited copy bumps the version like any
    /// other definition. No privileged tier exists.
    #[test]
    fn starter_roles_are_plain_editable_definitions() {
        for definition in starter_roles() {
            assert_eq!(definition.validate(), Ok(()));
        }
        let names: Vec<String> = starter_roles()
            .iter()
            .map(|definition| definition.name.clone())
            .collect();
        assert_eq!(names, vec!["observer", "participant", "archivist-editor"]);

        let mut registry = RoleRegistry::new("atlas");
        let outcomes = registry.install_starters(OWNER, NOW);
        assert_eq!(outcomes.len(), 3);
        assert!(outcomes.iter().all(|outcome| outcome.decision.is_ok()));
        assert!(registry.records().iter().all(|record| record.version == 1));

        // Copy the participant starter and edit its ceiling: the same
        // upsert path, an ordinary version bump, no special case.
        let mut edited = participant();
        edited.ceiling = WorldScopes::JOIN.union(WorldScopes::HISTORY);
        edited.name = "quiet-participant".to_owned();
        let outcome = registry.upsert(edited, OWNER, NOW + 1);
        assert_eq!(outcome.decision, Ok(()));
        assert_eq!(
            registry
                .record("quiet-participant")
                .unwrap()
                .definition
                .ceiling,
            WorldScopes::JOIN.union(WorldScopes::HISTORY)
        );
        assert_eq!(registry.record("quiet-participant").unwrap().version, 1);
    }

    /// Oracle §2.1 conformance: an agent whose role's ceiling is
    /// lowered mid-session is refused the newly out-of-ceiling action
    /// on its **next frame** — the changed ceiling applies at the
    /// next frame check, not the next join
    /// ([`PolicyPoint::refresh_ceiling`]).
    #[test]
    fn lowered_ceiling_refuses_the_next_frame() {
        let host = SigningKey::from_bytes(&[3u8; 32]);
        let grant = JoinGrant {
            agent: agent(9),
            owner: [2u8; 32],
            world: "atlas".to_owned(),
            role: "participant".to_owned(),
            scopes: WorldScopes::GUEST,
            expires_at: NOW + 3_600,
            proof_ref: "proof".to_owned(),
        };
        let ceiling = Ceiling::from_grant(&grant);
        let budget = Quota {
            frames: 10,
            tool_calls: 10,
            program_invocations: 0,
        };
        let mut point = PolicyPoint::open(&grant, ceiling, budget, Vec::new(), &host, NOW)
            .expect("session opens in its world");

        // Before the edit: chat.post is inside the ceiling.
        let chat = AgentAction::new("chat.post", CEILING_CHAT_POST);
        let admitted = point.admit(&host, &chat, 0, NOW + 1);
        assert_eq!(
            admitted.decision,
            Ok(Quota::default().plus(Quota::charge_for(&chat)))
        );

        // The owner lowers the role's ceiling mid-session: an
        // owner-side world-config edit, applied to the live session.
        let lowered = Ceiling::new(
            vec![CEILING_PRESENCE.to_owned()],
            vec!["atlas".to_owned()],
            Vec::new(),
            Vec::new(),
        );
        assert_eq!(point.refresh_ceiling(lowered), Ok(()));

        // The very next frame is refused for the newly out-of-ceiling
        // action, and the refusal is audited in the session log.
        let refused = point.admit(&host, &chat, 0, NOW + 2);
        assert_eq!(refused.decision, Err(PolicyRefusal::CeilingTool));
        let last = point
            .log()
            .events()
            .last()
            .expect("refusal is an audit event");
        assert_eq!(last.event.outcome, "refused:ceiling_tool");
        assert!(point.log().verify(&host.verifying_key()).is_ok());

        // A refresh that does not name the session's world never
        // applies: a session never runs under a foreign ceiling.
        let foreign = Ceiling::new(
            vec![CEILING_PRESENCE.to_owned()],
            vec!["elsewhere".to_owned()],
            Vec::new(),
            Vec::new(),
        );
        assert_eq!(
            point.refresh_ceiling(foreign),
            Err(PolicyRefusal::CeilingWorld)
        );
    }
}
