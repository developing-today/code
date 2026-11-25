//! Agent-majority worlds and human visitors — v3 shared-agent-worlds
//! step 4 (#10052, parent #6302).
//!
//! Oracle: `doc/2026-11-14_design_id6302_shared_agent_worlds_v3_outline.md`
//! §2.5 and §4 decisions 5–6, over the shared work surfaces
//! ([`crate::work_surfaces`], #10049), the v2 role configuration
//! ([`crate::role_config`], #8299) and multi-agent presence
//! ([`crate::world_agents`], #8300).
//!
//! # What this module settles
//!
//! - **The declaration is world configuration.** A [`WorldProfile`] says
//!   whether the world's standing population is mostly agents, which
//!   role visitors arrive under, and the visitor defaults (presence
//!   emphasis, summary cadence, how prominent the review surface is).
//!   It is serde-shaped and versioned like a role definition; only the
//!   owner changes it, every change and refusal is audited, and any
//!   client may read it ([`ProfileStore::profile`],
//!   [`ProfileStore::to_json`]). It selects no transport and no client
//!   mode: a client that ignores it still gets the same authority.
//! - **A visitor has exactly its role's ceiling.** The visitor is a role
//!   like any other: [`Visitors::act`] maps each visitor act to the
//!   [`WorldScopes`] bit it needs and checks it against the ceiling of
//!   the role the visitor was granted, read from the [`RoleRegistry`] at
//!   the moment of the act — so a narrowed role applies at the next act.
//!   The world being agent-majority changes defaults, never authority.
//! - **No silent override of agent work.** A visitor act on an item an
//!   agent holds is refused, whatever the visitor's ceiling. Taking work
//!   back from an agent is an explicit owner act with a stated reason
//!   ([`Visitors::owner_override`]), which releases the claim through
//!   the work surfaces and is audited on both sides.
//! - **Presence honesty.** [`PresenceRoster`] renders who is here. An
//!   agent always renders as an agent, with its owner's name where a
//!   human's would be; a key that holds an agent grant, or that is
//!   present as an agent, cannot enter as a human; an agent with no
//!   owner name is refused.
//! - **"What changed since I was last here."** [`visitor_summary`]
//!   renders, from the surfaces' audit log alone — the record of truth —
//!   the delta on each shared surface since a visitor's last presence:
//!   what was done, by whom, what was refused, and which claims ran out.
//!   [`Visitors::arrive`] and [`Visitors::depart`] keep the
//!   last-presence mark per visitor key.
//!
//! Standing-population caps and per-role event-rate budgets (§2.5,
//! decision 5) are later slices of #10052; they extend this profile.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::agent_delegation::{AgentId, MIN_LIFETIME_SECS};
use crate::mcp_policy::Quota;
use crate::role_config::{QuotaClass, ReviewPolicy, RoleDefinition, RoleRegistry};
use crate::work_surfaces::{
    EVENT_CLAIM_EXPIRE, SurfaceActor, SurfaceError, SurfaceEvent, SurfaceId, WorkSurfaces,
};
use crate::world_agents::AgentPresence;
use crate::world_caps::MAX_NAME_BYTES;
use crate::world_scopes::WorldScopes;

/// Audit event kind: the world profile was set.
pub const EVENT_PROFILE_SET: &str = "world.profile.set";
/// Audit event kind: a visitor role was granted to a human key.
pub const EVENT_VISITOR_GRANT: &str = "visitor.grant";
/// Audit event kind: a visitor grant was revoked.
pub const EVENT_VISITOR_REVOKE: &str = "visitor.revoke";
/// Audit event kind: a visitor acted (or was refused).
pub const EVENT_VISITOR_ACT: &str = "visitor.act";
/// Audit event kind: the owner took work back from an agent.
pub const EVENT_OWNER_OVERRIDE: &str = "visitor.owner_override";
/// Audit event kind: someone entered the presence roster (or was
/// refused).
pub const EVENT_PRESENCE_ENTER: &str = "presence.enter";

/// The starter visitor role's name.
pub const VISITOR_ROLE: &str = "visitor";
/// The default summary cadence for visitors, seconds.
pub const DEFAULT_SUMMARY_CADENCE_SECS: u64 = 15 * 60;
/// The longest override reason, in bytes.
pub const MAX_REASON_BYTES: usize = 1024;

/// How prominently a visitor's client shows the review surface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewProminence {
    /// Behind a menu.
    Collapsed,
    /// Alongside the other surfaces.
    #[default]
    Normal,
    /// First thing a visitor sees.
    Prominent,
}

/// Defaults a visitor's client starts from. Defaults only: none of them
/// grants or withholds authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisitorDefaults {
    /// Lead with who is here (agents, roles, owners).
    #[serde(default)]
    pub presence_emphasis: bool,
    /// How often a visitor's client refreshes its change summary,
    /// seconds; positive.
    pub summary_cadence_secs: u64,
    /// How prominent the review surface is.
    #[serde(default)]
    pub review_prominence: ReviewProminence,
}

impl Default for VisitorDefaults {
    fn default() -> Self {
        Self {
            presence_emphasis: false,
            summary_cadence_secs: DEFAULT_SUMMARY_CADENCE_SECS,
            review_prominence: ReviewProminence::Normal,
        }
    }
}

/// A world's population profile: world configuration any client may read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldProfile {
    /// Whether the world's standing population is mostly agents.
    #[serde(default)]
    pub agent_majority: bool,
    /// The role human visitors arrive under; must name a role in the
    /// world's registry.
    pub visitor_role: String,
    /// Defaults for visitors' clients.
    #[serde(default)]
    pub visitor_defaults: VisitorDefaults,
}

impl WorldProfile {
    /// The profile of an ordinary world: not agent-majority, visitors
    /// under [`VISITOR_ROLE`], default visitor defaults.
    #[must_use]
    pub fn ordinary() -> Self {
        Self {
            agent_majority: false,
            visitor_role: VISITOR_ROLE.to_owned(),
            visitor_defaults: VisitorDefaults::default(),
        }
    }

    /// The starter agent-majority profile: presence first, the review
    /// surface prominent.
    #[must_use]
    pub fn agent_majority() -> Self {
        Self {
            agent_majority: true,
            visitor_role: VISITOR_ROLE.to_owned(),
            visitor_defaults: VisitorDefaults {
                presence_emphasis: true,
                summary_cadence_secs: DEFAULT_SUMMARY_CADENCE_SECS,
                review_prominence: ReviewProminence::Prominent,
            },
        }
    }

    fn validate(&self, registry: &RoleRegistry) -> Result<(), VisitorError> {
        if self.visitor_role.is_empty() || self.visitor_role.len() > MAX_NAME_BYTES {
            return Err(VisitorError::BadProfile);
        }
        if self.visitor_defaults.summary_cadence_secs == 0 {
            return Err(VisitorError::BadProfile);
        }
        if registry.record(&self.visitor_role).is_none() {
            return Err(VisitorError::UnknownRole);
        }
        Ok(())
    }
}

/// The starter visitor role: join and chat, nothing else — read-mostly,
/// as ordinary world configuration a world may copy and edit. Its
/// budget is the observer's: modest, never replenishing.
#[must_use]
pub fn starter_visitor_role() -> RoleDefinition {
    RoleDefinition {
        name: VISITOR_ROLE.to_owned(),
        ceiling: WorldScopes::JOIN.union(WorldScopes::CHAT),
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
    }
}

/// Why a profile, visitor or presence act was refused. Each maps to a
/// stable code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VisitorError {
    /// Only the owner may do this.
    NotOwner,
    /// The profile is malformed.
    BadProfile,
    /// The role is not in the world's registry.
    UnknownRole,
    /// The key holds no visitor grant here.
    NotAVisitor,
    /// The key already holds a visitor grant.
    AlreadyGranted,
    /// The act needs a scope the visitor's role ceiling lacks.
    OutsideCeiling {
        /// The scope the act needs.
        needed: WorldScopes,
    },
    /// An agent holds the item; only the owner may take it back.
    AgentWorkHeld {
        /// The agent holding it.
        holder: SurfaceActor,
    },
    /// The act is the owner's alone.
    OwnerOnly,
    /// The override reason is empty or too long.
    BadReason,
    /// The work surfaces refused the owner's release.
    Surface(SurfaceError),
    /// A key that is an agent tried to enter as a human.
    AgentAsHuman,
    /// An agent entered with no owner name.
    MissingOwner,
    /// A display name is empty or too long.
    BadName,
}

impl VisitorError {
    /// Stable machine-readable code, as journaled in an event outcome.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotOwner => "not_owner",
            Self::BadProfile => "bad_profile",
            Self::UnknownRole => "unknown_role",
            Self::NotAVisitor => "not_a_visitor",
            Self::AlreadyGranted => "already_granted",
            Self::OutsideCeiling { .. } => "outside_ceiling",
            Self::AgentWorkHeld { .. } => "agent_work_held",
            Self::OwnerOnly => "owner_only",
            Self::BadReason => "bad_reason",
            Self::Surface(_) => "surface_refused",
            Self::AgentAsHuman => "agent_as_human",
            Self::MissingOwner => "missing_owner",
            Self::BadName => "bad_name",
        }
    }
}

/// One audited profile, visitor or presence act.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisitorEvent {
    /// One of the `EVENT_*` kinds.
    pub kind: String,
    /// The world.
    pub world: String,
    /// Who acted: `owner:<label>`, `human:<key hex>` or `agent:<key hex>`.
    pub actor: String,
    /// What the act named (a role, a surface, an item), when it names one.
    pub subject: Option<String>,
    /// `executed`, or `refused:<code>`.
    pub outcome: String,
    /// A free-text note: the owner's override reason.
    pub note: Option<String>,
    /// When, unix seconds.
    pub at: u64,
}

fn outcome<T>(result: &Result<T, VisitorError>) -> String {
    match result {
        Ok(_) => "executed".to_owned(),
        Err(refusal) => format!("refused:{}", refusal.code()),
    }
}

fn hex(key: &[u8; 32]) -> String {
    AgentId(*key).to_hex()
}

/// The world's profile, versioned, with its audit log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileStore {
    world: String,
    owner: String,
    profile: WorldProfile,
    version: u64,
    events: Vec<VisitorEvent>,
}

impl ProfileStore {
    /// A world's store, starting from the [`WorldProfile::ordinary`]
    /// profile at version 0. `owner` is the owner's key hex or label.
    pub fn new(world: impl Into<String>, owner: impl Into<String>) -> Self {
        Self {
            world: world.into(),
            owner: owner.into(),
            profile: WorldProfile::ordinary(),
            version: 0,
            events: Vec::new(),
        }
    }

    /// The profile in force. Anyone may read it.
    #[must_use]
    pub const fn profile(&self) -> &WorldProfile {
        &self.profile
    }

    /// The profile's version: 0 until first set, +1 per change.
    #[must_use]
    pub const fn version(&self) -> u64 {
        self.version
    }

    /// The profile and its version as JSON, as any client reads it.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "world": self.world,
            "version": self.version,
            "profile": self.profile,
        })
    }

    /// Every profile change and refusal.
    #[must_use]
    pub fn events(&self) -> &[VisitorEvent] {
        &self.events
    }

    /// Set the profile. Only the owner may; the visitor role must be in
    /// the registry.
    ///
    /// # Errors
    ///
    /// The actor is not the owner, or the profile is malformed or names
    /// an unknown role. Refusals are audited too.
    pub fn set(
        &mut self,
        registry: &RoleRegistry,
        actor: &str,
        profile: WorldProfile,
        now: u64,
    ) -> Result<u64, VisitorError> {
        let result = if actor == self.owner {
            profile.validate(registry).map(|()| {
                self.profile = profile;
                self.version += 1;
                self.version
            })
        } else {
            Err(VisitorError::NotOwner)
        };
        self.events.push(VisitorEvent {
            kind: EVENT_PROFILE_SET.to_owned(),
            world: self.world.clone(),
            actor: format!("owner:{actor}"),
            subject: None,
            outcome: outcome(&result),
            note: None,
            at: now,
        });
        result
    }
}

/// What a visitor does to an item, for the agent-work check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemOp {
    /// Claim it.
    Claim,
    /// Release a claim on it.
    Release,
    /// Move it between states.
    Move,
}

/// One thing a visitor tries to do.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "act", rename_all = "snake_case")]
pub enum VisitorAct {
    /// Look at a shared surface.
    Observe {
        /// The surface.
        surface: SurfaceId,
    },
    /// Post a chat line.
    PostChat,
    /// Act on a board item. `holder` is the item's live claim holder as
    /// the host read it, if any.
    Item {
        /// The board.
        board: String,
        /// The item.
        item: u64,
        /// What the visitor does to it.
        op: ItemOp,
        /// Who holds it now.
        holder: Option<SurfaceActor>,
    },
    /// Review a shared document.
    Review {
        /// The document.
        document: String,
    },
    /// Take work back from an agent. Never a visitor's act.
    Override {
        /// The board.
        board: String,
        /// The item.
        item: u64,
    },
}

impl VisitorAct {
    /// The scope the act needs from the visitor's role ceiling, or
    /// `None` when no ceiling allows it.
    #[must_use]
    pub const fn needed_scope(&self) -> Option<WorldScopes> {
        match self {
            Self::Observe { .. } => Some(WorldScopes::JOIN),
            Self::PostChat => Some(WorldScopes::CHAT),
            Self::Item { .. } | Self::Review { .. } => Some(WorldScopes::INPUT),
            Self::Override { .. } => None,
        }
    }

    fn subject(&self) -> Option<String> {
        match self {
            Self::Observe { surface } => Some(match surface {
                SurfaceId::Board(name) => format!("board:{name}"),
                SurfaceId::Document(name) => format!("document:{name}"),
            }),
            Self::PostChat => None,
            Self::Item { board, item, .. } | Self::Override { board, item } => {
                Some(format!("board:{board}#{item}"))
            }
            Self::Review { document } => Some(format!("document:{document}")),
        }
    }
}

/// One human visitor's grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisitorGrant {
    /// The human's key.
    pub key: [u8; 32],
    /// The role the visitor holds.
    pub role: String,
    /// Who granted it.
    pub granted_by: String,
    /// When, unix seconds.
    pub granted_at: u64,
}

/// A world's human visitors: their grants, last-presence marks and the
/// audit log of everything they did or were refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Visitors {
    world: String,
    owner: String,
    grants: BTreeMap<[u8; 32], VisitorGrant>,
    last_seen: BTreeMap<[u8; 32], u64>,
    events: Vec<VisitorEvent>,
}

impl Visitors {
    /// No visitors yet. `owner` is the owner's key hex or label.
    pub fn new(world: impl Into<String>, owner: impl Into<String>) -> Self {
        Self {
            world: world.into(),
            owner: owner.into(),
            grants: BTreeMap::new(),
            last_seen: BTreeMap::new(),
            events: Vec::new(),
        }
    }

    /// Every visitor act, refusal and override.
    #[must_use]
    pub fn events(&self) -> &[VisitorEvent] {
        &self.events
    }

    /// The visitor grants in force, by key.
    #[must_use]
    pub fn grants(&self) -> Vec<&VisitorGrant> {
        self.grants.values().collect()
    }

    /// Whether `key` holds a visitor grant here.
    #[must_use]
    pub fn is_visitor(&self, key: &[u8; 32]) -> bool {
        self.grants.contains_key(key)
    }

    fn record(
        &mut self,
        kind: &str,
        actor: String,
        subject: Option<String>,
        outcome: String,
        note: Option<String>,
        at: u64,
    ) {
        self.events.push(VisitorEvent {
            kind: kind.to_owned(),
            world: self.world.clone(),
            actor,
            subject,
            outcome,
            note,
            at,
        });
    }

    /// Grant `role` to a human visitor. Only the owner may; the role must
    /// be in the registry; a key holding an agent grant is an agent and
    /// cannot be a human visitor.
    ///
    /// # Errors
    ///
    /// Not the owner, unknown role, the key is an agent, or it already
    /// holds a visitor grant. Refusals are audited.
    pub fn grant(
        &mut self,
        registry: &RoleRegistry,
        actor: &str,
        key: [u8; 32],
        role: &str,
        now: u64,
    ) -> Result<(), VisitorError> {
        let result = if actor != self.owner {
            Err(VisitorError::NotOwner)
        } else if registry.record(role).is_none() {
            Err(VisitorError::UnknownRole)
        } else if holds_agent_grant(registry, &key) {
            Err(VisitorError::AgentAsHuman)
        } else if self.grants.contains_key(&key) {
            Err(VisitorError::AlreadyGranted)
        } else {
            self.grants.insert(
                key,
                VisitorGrant {
                    key,
                    role: role.to_owned(),
                    granted_by: actor.to_owned(),
                    granted_at: now,
                },
            );
            Ok(())
        };
        self.record(
            EVENT_VISITOR_GRANT,
            format!("owner:{actor}"),
            Some(format!("human:{}/{role}", hex(&key))),
            outcome(&result),
            None,
            now,
        );
        result
    }

    /// Revoke a visitor's grant. Only the owner may.
    ///
    /// # Errors
    ///
    /// Not the owner, or the key holds no grant. Refusals are audited.
    pub fn revoke(&mut self, actor: &str, key: &[u8; 32], now: u64) -> Result<(), VisitorError> {
        let result = if actor != self.owner {
            Err(VisitorError::NotOwner)
        } else if self.grants.remove(key).is_some() {
            Ok(())
        } else {
            Err(VisitorError::NotAVisitor)
        };
        self.record(
            EVENT_VISITOR_REVOKE,
            format!("owner:{actor}"),
            Some(format!("human:{}", hex(key))),
            outcome(&result),
            None,
            now,
        );
        result
    }

    /// Decide one visitor act. The visitor gets exactly the ceiling of
    /// the role it holds, as the registry defines it now; the world's
    /// profile changes no part of the answer. An act on an item an agent
    /// holds is refused whatever the ceiling, and an override is never a
    /// visitor's act. Every decision is audited.
    ///
    /// Surface-level permissions (which roles a board lets claim) are
    /// still applied by [`WorkSurfaces`] when the host executes the act;
    /// this check is the ceiling every visitor act passes first.
    ///
    /// # Errors
    ///
    /// The key is not a visitor, its role is gone, the act needs a scope
    /// outside the ceiling, an agent holds the item, or the act is the
    /// owner's.
    pub fn act(
        &mut self,
        registry: &RoleRegistry,
        key: &[u8; 32],
        act: &VisitorAct,
        now: u64,
    ) -> Result<(), VisitorError> {
        let result = self.decide(registry, key, act);
        self.record(
            EVENT_VISITOR_ACT,
            format!("human:{}", hex(key)),
            act.subject(),
            outcome(&result),
            None,
            now,
        );
        result
    }

    fn decide(
        &self,
        registry: &RoleRegistry,
        key: &[u8; 32],
        act: &VisitorAct,
    ) -> Result<(), VisitorError> {
        let grant = self.grants.get(key).ok_or(VisitorError::NotAVisitor)?;
        let ceiling = registry
            .record(&grant.role)
            .ok_or(VisitorError::UnknownRole)?
            .definition
            .ceiling
            .implied();
        let Some(needed) = act.needed_scope() else {
            return Err(VisitorError::OwnerOnly);
        };
        if !ceiling.contains(needed) {
            return Err(VisitorError::OutsideCeiling { needed });
        }
        if let VisitorAct::Item {
            holder: Some(holder @ SurfaceActor::Agent { .. }),
            ..
        } = act
        {
            return Err(VisitorError::AgentWorkHeld {
                holder: holder.clone(),
            });
        }
        Ok(())
    }

    /// The owner takes an item back from the agent holding it: an
    /// explicit act with a stated reason, executed as an owner release on
    /// the work surfaces and audited here with the reason.
    ///
    /// # Errors
    ///
    /// The actor is not the owner, the reason is empty or too long, or
    /// the surfaces refused the release (no such item, no live claim).
    pub fn owner_override(
        &mut self,
        surfaces: &mut WorkSurfaces,
        registry: &RoleRegistry,
        actor: &str,
        board: &str,
        item: u64,
        reason: &str,
        now: u64,
    ) -> Result<(), VisitorError> {
        let result = if actor != self.owner {
            Err(VisitorError::NotOwner)
        } else if reason.trim().is_empty() || reason.len() > MAX_REASON_BYTES {
            Err(VisitorError::BadReason)
        } else {
            surfaces
                .release(registry, &SurfaceActor::owner(actor), board, item, now)
                .map_err(VisitorError::Surface)
        };
        self.record(
            EVENT_OWNER_OVERRIDE,
            format!("owner:{actor}"),
            Some(format!("board:{board}#{item}")),
            outcome(&result),
            Some(reason.to_owned()),
            now,
        );
        result
    }

    /// A visitor arrives: the summary of what changed on the surfaces
    /// since its last presence (everything, on a first visit).
    ///
    /// # Errors
    ///
    /// The key holds no visitor grant.
    pub fn arrive(
        &self,
        key: &[u8; 32],
        audit: &[SurfaceEvent],
        now: u64,
    ) -> Result<VisitorSummary, VisitorError> {
        if !self.grants.contains_key(key) {
            return Err(VisitorError::NotAVisitor);
        }
        Ok(visitor_summary(
            audit,
            self.last_seen.get(key).copied(),
            now,
        ))
    }

    /// A visitor leaves: its last-presence mark moves to `now`.
    pub fn depart(&mut self, key: &[u8; 32], now: u64) {
        if self.grants.contains_key(key) {
            self.last_seen.insert(*key, now);
        }
    }

    /// When the visitor was last present, if ever.
    #[must_use]
    pub fn last_seen(&self, key: &[u8; 32]) -> Option<u64> {
        self.last_seen.get(key).copied()
    }
}

fn holds_agent_grant(registry: &RoleRegistry, key: &[u8; 32]) -> bool {
    registry.list_grants().iter().any(|g| g.agent.0 == *key)
}

/// What happened on one surface in a visitor's window.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceDelta {
    /// Executed acts, by event kind, with counts.
    pub executed: BTreeMap<String, u64>,
    /// How many acts were refused.
    pub refused: u64,
    /// Who acted (executed acts only), as the audit log names them.
    pub actors: BTreeSet<String>,
    /// Items whose claim ran out in the window — work that stalled.
    pub stalled_items: BTreeSet<u64>,
}

/// "What changed since I was last here", rendered from the audit log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisitorSummary {
    /// The window's start (exclusive), or `None` on a first visit.
    pub since: Option<u64>,
    /// The window's end (inclusive).
    pub through: u64,
    /// Per surface, what happened.
    pub surfaces: BTreeMap<SurfaceId, SurfaceDelta>,
}

impl VisitorSummary {
    /// Whether nothing happened in the window.
    #[must_use]
    pub fn is_quiet(&self) -> bool {
        self.surfaces.is_empty()
    }

    /// One line per surface, for a plain-text client.
    #[must_use]
    pub fn to_text(&self) -> String {
        if self.is_quiet() {
            return "nothing changed since your last visit".to_owned();
        }
        let mut lines = Vec::new();
        for (surface, delta) in &self.surfaces {
            let name = match surface {
                SurfaceId::Board(name) => format!("board {name}"),
                SurfaceId::Document(name) => format!("document {name}"),
            };
            let done: Vec<String> = delta
                .executed
                .iter()
                .map(|(kind, count)| format!("{count}× {kind}"))
                .collect();
            let mut parts = vec![if done.is_empty() {
                "no changes".to_owned()
            } else {
                done.join(", ")
            }];
            if delta.refused > 0 {
                parts.push(format!("{} refused", delta.refused));
            }
            if !delta.stalled_items.is_empty() {
                let items: Vec<String> = delta
                    .stalled_items
                    .iter()
                    .map(|item| format!("#{item}"))
                    .collect();
                parts.push(format!("stalled {}", items.join(" ")));
            }
            let mut line = format!("{name}: {}", parts.join("; "));
            if !delta.actors.is_empty() {
                let actors: Vec<&str> = delta.actors.iter().map(String::as_str).collect();
                line = format!("{line} (by {})", actors.join(", "));
            }
            lines.push(line);
        }
        lines.join("\n")
    }
}

/// Render the delta on every shared surface from the audit log alone:
/// events after `since` (all of them when `None`) up to `through`.
#[must_use]
pub fn visitor_summary(audit: &[SurfaceEvent], since: Option<u64>, through: u64) -> VisitorSummary {
    let mut surfaces: BTreeMap<SurfaceId, SurfaceDelta> = BTreeMap::new();
    for event in audit {
        if since.is_some_and(|mark| event.at <= mark) || event.at > through {
            continue;
        }
        let delta = surfaces.entry(event.surface.clone()).or_default();
        if event.outcome == "executed" {
            *delta.executed.entry(event.kind.clone()).or_default() += 1;
            delta.actors.insert(event.actor.clone());
            if event.kind == EVENT_CLAIM_EXPIRE
                && let Some(item) = event.item
            {
                delta.stalled_items.insert(item);
            }
        } else {
            delta.refused += 1;
        }
    }
    VisitorSummary {
        since,
        through,
        surfaces,
    }
}

/// Someone entering the world's presence roster.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Entrant {
    /// A human, by key and chosen display name.
    Human {
        /// The human's key.
        key: [u8; 32],
        /// The display name.
        name: String,
        /// The role held here.
        role: String,
    },
    /// An agent, as the world's agent sessions present it.
    Agent(AgentPresence),
}

/// What a participant is, as presence renders it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParticipantKind {
    /// A human.
    Human,
    /// An agent.
    Agent,
}

/// One rendered presence line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceLine {
    /// Human or agent; never left for the client to guess.
    pub kind: ParticipantKind,
    /// The name shown.
    pub name: String,
    /// For an agent, its owner's name, shown wherever a human's name
    /// would be; `None` for a human.
    pub owner: Option<String>,
    /// The role held here.
    pub role: String,
    /// The text a plain client shows, which always says "agent" and the
    /// owner for an agent.
    pub display: String,
}

/// Who is here, rendered for anyone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceView {
    /// How many agents are present.
    pub agents: usize,
    /// How many humans are present.
    pub humans: usize,
    /// Every participant, humans first when the profile emphasises
    /// presence in an agent-majority world, else by name.
    pub lines: Vec<PresenceLine>,
    /// Agents grouped by owner name.
    pub agents_by_owner: BTreeMap<String, Vec<String>>,
}

/// The world's presence roster, honest by construction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresenceRoster {
    world: String,
    present: BTreeMap<[u8; 32], PresenceLine>,
    events: Vec<VisitorEvent>,
}

impl PresenceRoster {
    /// An empty roster.
    pub fn new(world: impl Into<String>) -> Self {
        Self {
            world: world.into(),
            present: BTreeMap::new(),
            events: Vec::new(),
        }
    }

    /// Every entry and refusal.
    #[must_use]
    pub fn events(&self) -> &[VisitorEvent] {
        &self.events
    }

    /// Enter the roster. A human entrant is refused if its key holds an
    /// agent grant or is present as an agent; an agent entrant is refused
    /// without an owner name. Every entry is audited.
    ///
    /// # Errors
    ///
    /// [`VisitorError::AgentAsHuman`], [`VisitorError::MissingOwner`] or
    /// [`VisitorError::BadName`].
    pub fn enter(
        &mut self,
        registry: &RoleRegistry,
        entrant: Entrant,
        now: u64,
    ) -> Result<(), VisitorError> {
        let (key, actor) = match &entrant {
            Entrant::Human { key, .. } => (*key, format!("human:{}", hex(key))),
            Entrant::Agent(agent) => (agent.agent_key, format!("agent:{}", hex(&agent.agent_key))),
        };
        let result = self.admit(registry, entrant);
        self.events.push(VisitorEvent {
            kind: EVENT_PRESENCE_ENTER.to_owned(),
            world: self.world.clone(),
            actor,
            subject: None,
            outcome: outcome(&result),
            note: None,
            at: now,
        });
        result.map(|line| {
            self.present.insert(key, line);
        })
    }

    fn admit(
        &self,
        registry: &RoleRegistry,
        entrant: Entrant,
    ) -> Result<PresenceLine, VisitorError> {
        match entrant {
            Entrant::Human { key, name, role } => {
                let name = name.trim().to_owned();
                if name.is_empty() || name.len() > MAX_NAME_BYTES {
                    return Err(VisitorError::BadName);
                }
                let present_as_agent = self
                    .present
                    .get(&key)
                    .is_some_and(|line| line.kind == ParticipantKind::Agent);
                if present_as_agent || holds_agent_grant(registry, &key) {
                    return Err(VisitorError::AgentAsHuman);
                }
                Ok(PresenceLine {
                    kind: ParticipantKind::Human,
                    display: format!("{name} ({role})"),
                    name,
                    owner: None,
                    role,
                })
            }
            Entrant::Agent(agent) => {
                let owner = agent.owner.trim().to_owned();
                if owner.is_empty() {
                    return Err(VisitorError::MissingOwner);
                }
                let name = agent.agent_label.trim().to_owned();
                if name.is_empty() || name.len() > MAX_NAME_BYTES {
                    return Err(VisitorError::BadName);
                }
                Ok(PresenceLine {
                    kind: ParticipantKind::Agent,
                    display: format!("{name} (agent of {owner}, {})", agent.role),
                    name,
                    owner: Some(owner),
                    role: agent.role,
                })
            }
        }
    }

    /// Someone leaves.
    pub fn leave(&mut self, key: &[u8; 32]) {
        self.present.remove(key);
    }

    /// Render who is here under `profile`'s presence defaults.
    #[must_use]
    pub fn view(&self, profile: &WorldProfile) -> PresenceView {
        let mut lines: Vec<PresenceLine> = self.present.values().cloned().collect();
        let humans_first = profile.agent_majority && profile.visitor_defaults.presence_emphasis;
        lines.sort_by(|a, b| {
            let rank = |line: &PresenceLine| match (humans_first, line.kind) {
                (true, ParticipantKind::Human) | (false, _) => 0,
                (true, ParticipantKind::Agent) => 1,
            };
            rank(a).cmp(&rank(b)).then_with(|| a.name.cmp(&b.name))
        });
        let mut agents_by_owner: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for line in &lines {
            if let Some(owner) = &line.owner {
                agents_by_owner
                    .entry(owner.clone())
                    .or_default()
                    .push(line.name.clone());
            }
        }
        let agents = lines
            .iter()
            .filter(|line| line.kind == ParticipantKind::Agent)
            .count();
        PresenceView {
            agents,
            humans: lines.len() - agents,
            lines,
            agents_by_owner,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::work_surfaces::{BoardDefinition, BoardPermissions, ItemDraft};

    const WORLD: &str = "studio";
    const OWNER: &str = "owner";
    const T0: u64 = 1_800_000_000;
    const HUMAN: [u8; 32] = [0xA0; 32];
    const HUMAN2: [u8; 32] = [0xA1; 32];

    fn agent(byte: u8) -> AgentId {
        AgentId([byte; 32])
    }

    /// Starters plus the visitor role and a "contributor" visitor role
    /// with input; agents 1 and 2 are participants.
    fn registry() -> RoleRegistry {
        let mut registry = RoleRegistry::new(WORLD);
        registry.install_starters(OWNER, T0);
        assert!(
            registry
                .upsert(starter_visitor_role(), OWNER, T0)
                .decision
                .is_ok()
        );
        let mut contributor = starter_visitor_role();
        contributor.name = "contributor".to_owned();
        contributor.ceiling = WorldScopes::GUEST;
        assert!(registry.upsert(contributor, OWNER, T0).decision.is_ok());
        for byte in [1, 2] {
            assert!(
                registry
                    .grant("participant", agent(byte), OWNER, T0)
                    .decision
                    .is_ok()
            );
        }
        registry
    }

    fn presence(byte: u8, label: &str, owner: &str) -> AgentPresence {
        AgentPresence {
            agent_key: [byte; 32],
            agent_label: label.to_owned(),
            owner: owner.to_owned(),
            role: "participant".to_owned(),
        }
    }

    /// Acceptance 1: the declaration is world configuration — owner-set,
    /// versioned, audited, readable by any client as plain JSON, with no
    /// transport field it would accept.
    #[test]
    fn profile_is_world_configuration() {
        let registry = registry();
        let mut store = ProfileStore::new(WORLD, OWNER);
        assert_eq!(store.version(), 0);
        assert!(!store.profile().agent_majority);

        assert_eq!(
            store.set(&registry, "mallory", WorldProfile::agent_majority(), T0),
            Err(VisitorError::NotOwner)
        );
        let mut unknown = WorldProfile::agent_majority();
        unknown.visitor_role = "nobody".to_owned();
        assert_eq!(
            store.set(&registry, OWNER, unknown, T0),
            Err(VisitorError::UnknownRole)
        );
        let mut zero = WorldProfile::agent_majority();
        zero.visitor_defaults.summary_cadence_secs = 0;
        assert_eq!(
            store.set(&registry, OWNER, zero, T0),
            Err(VisitorError::BadProfile)
        );
        assert_eq!(store.version(), 0);

        assert_eq!(
            store.set(&registry, OWNER, WorldProfile::agent_majority(), T0 + 1),
            Ok(1)
        );
        assert!(store.profile().agent_majority);
        let outcomes: Vec<&str> = store.events().iter().map(|e| e.outcome.as_str()).collect();
        assert_eq!(
            outcomes,
            [
                "refused:not_owner",
                "refused:unknown_role",
                "refused:bad_profile",
                "executed"
            ]
        );

        // Any client reads it; it round-trips; a transport mode is not
        // part of the shape.
        let json = store.to_json();
        assert_eq!(json["version"], 1);
        assert_eq!(json["profile"]["agent_majority"], true);
        let back: WorldProfile = serde_json::from_value(json["profile"].clone()).unwrap();
        assert_eq!(&back, store.profile());
        let mut with_transport = json["profile"].clone();
        with_transport["transport"] = serde_json::json!("relay");
        assert!(serde_json::from_value::<WorldProfile>(with_transport).is_err());
    }

    /// Acceptance 2 and 5: a visitor has exactly its role's ceiling, and
    /// the world being agent-majority changes no answer.
    #[test]
    fn visitor_gets_exactly_its_role_ceiling_in_any_world() {
        let registry = registry();
        let acts = [
            VisitorAct::Observe {
                surface: SurfaceId::Board("work".to_owned()),
            },
            VisitorAct::PostChat,
            VisitorAct::Item {
                board: "work".to_owned(),
                item: 1,
                op: ItemOp::Claim,
                holder: None,
            },
            VisitorAct::Review {
                document: "plan".to_owned(),
            },
            VisitorAct::Override {
                board: "work".to_owned(),
                item: 1,
            },
        ];
        for profile in [WorldProfile::ordinary(), WorldProfile::agent_majority()] {
            let mut store = ProfileStore::new(WORLD, OWNER);
            store.set(&registry, OWNER, profile, T0).unwrap();
            let mut visitors = Visitors::new(WORLD, OWNER);
            visitors
                .grant(&registry, OWNER, HUMAN, &store.profile().visitor_role, T0)
                .unwrap();
            visitors
                .grant(&registry, OWNER, HUMAN2, "contributor", T0)
                .unwrap();
            for act in &acts {
                for (key, role) in [(HUMAN, VISITOR_ROLE), (HUMAN2, "contributor")] {
                    let ceiling = registry.record(role).unwrap().definition.ceiling.implied();
                    let expected = match act.needed_scope() {
                        None => Err(VisitorError::OwnerOnly),
                        Some(needed) if ceiling.contains(needed) => Ok(()),
                        Some(needed) => Err(VisitorError::OutsideCeiling { needed }),
                    };
                    assert_eq!(
                        visitors.act(&registry, &key, act, T0),
                        expected,
                        "{act:?} {role}"
                    );
                }
            }
            // The starter visitor observes and chats; nothing else.
            let allowed: Vec<_> = visitors
                .events()
                .iter()
                .filter(|e| e.kind == EVENT_VISITOR_ACT && e.actor.contains(&hex(&HUMAN)))
                .map(|e| e.outcome == "executed")
                .collect();
            assert_eq!(allowed, [true, true, false, false, false]);
        }
    }

    /// The ceiling is read at the act: a revoked visitor, an unknown key
    /// and a narrowed role all apply at once.
    #[test]
    fn grants_and_role_edits_apply_at_the_next_act() {
        let mut registry = registry();
        let mut visitors = Visitors::new(WORLD, OWNER);
        assert_eq!(
            visitors.grant(&registry, "mallory", HUMAN, VISITOR_ROLE, T0),
            Err(VisitorError::NotOwner)
        );
        assert_eq!(
            visitors.grant(&registry, OWNER, agent(1).0, VISITOR_ROLE, T0),
            Err(VisitorError::AgentAsHuman)
        );
        visitors
            .grant(&registry, OWNER, HUMAN, VISITOR_ROLE, T0)
            .unwrap();
        assert_eq!(
            visitors.grant(&registry, OWNER, HUMAN, VISITOR_ROLE, T0),
            Err(VisitorError::AlreadyGranted)
        );
        assert_eq!(
            visitors.act(&registry, &HUMAN2, &VisitorAct::PostChat, T0),
            Err(VisitorError::NotAVisitor)
        );
        assert_eq!(
            visitors.act(&registry, &HUMAN, &VisitorAct::PostChat, T0),
            Ok(())
        );

        let mut narrowed = starter_visitor_role();
        narrowed.ceiling = WorldScopes::JOIN;
        assert!(registry.upsert(narrowed, OWNER, T0 + 1).decision.is_ok());
        assert_eq!(
            visitors.act(&registry, &HUMAN, &VisitorAct::PostChat, T0 + 1),
            Err(VisitorError::OutsideCeiling {
                needed: WorldScopes::CHAT
            })
        );

        visitors.revoke(OWNER, &HUMAN, T0 + 2).unwrap();
        assert_eq!(
            visitors.act(
                &registry,
                &HUMAN,
                &VisitorAct::Observe {
                    surface: SurfaceId::Document("plan".to_owned())
                },
                T0 + 2
            ),
            Err(VisitorError::NotAVisitor)
        );
        assert_eq!(
            visitors.revoke(OWNER, &HUMAN, T0 + 3),
            Err(VisitorError::NotAVisitor)
        );
    }

    fn surfaces_with_agent_claim(registry: &RoleRegistry) -> WorkSurfaces {
        let mut surfaces = WorkSurfaces::new(WORLD);
        let mut board = BoardDefinition::starter("work");
        board.roles.insert(
            "contributor".to_owned(),
            BoardPermissions {
                read: true,
                claim: true,
                ..BoardPermissions::default()
            },
        );
        surfaces
            .define_board(&SurfaceActor::owner(OWNER), board, T0)
            .unwrap();
        let worker = SurfaceActor::agent(agent(1), "participant");
        surfaces
            .create_item(
                registry,
                &worker,
                "work",
                ItemDraft {
                    title: "index the archive".to_owned(),
                    ..ItemDraft::default()
                },
                T0,
            )
            .unwrap();
        surfaces
            .claim(registry, &worker, "work", 1, 1, T0 + 1)
            .unwrap();
        surfaces
    }

    /// Acceptance 2: no silent override — a visitor with input still
    /// cannot touch an agent's held item; the owner's override is
    /// explicit, needs a reason, and is audited on both sides.
    #[test]
    fn no_silent_override_of_agent_work() {
        let registry = registry();
        let mut surfaces = surfaces_with_agent_claim(&registry);
        let mut visitors = Visitors::new(WORLD, OWNER);
        visitors
            .grant(&registry, OWNER, HUMAN2, "contributor", T0)
            .unwrap();
        let worker = SurfaceActor::agent(agent(1), "participant");

        for op in [ItemOp::Claim, ItemOp::Release, ItemOp::Move] {
            assert_eq!(
                visitors.act(
                    &registry,
                    &HUMAN2,
                    &VisitorAct::Item {
                        board: "work".to_owned(),
                        item: 1,
                        op,
                        holder: Some(worker.clone()),
                    },
                    T0 + 2,
                ),
                Err(VisitorError::AgentWorkHeld {
                    holder: worker.clone()
                })
            );
        }
        // An unheld item is fine for a contributor.
        assert_eq!(
            visitors.act(
                &registry,
                &HUMAN2,
                &VisitorAct::Item {
                    board: "work".to_owned(),
                    item: 2,
                    op: ItemOp::Claim,
                    holder: None,
                },
                T0 + 2,
            ),
            Ok(())
        );

        assert_eq!(
            visitors.owner_override(
                &mut surfaces,
                &registry,
                "mallory",
                "work",
                1,
                "stuck",
                T0 + 3
            ),
            Err(VisitorError::NotOwner)
        );
        assert_eq!(
            visitors.owner_override(&mut surfaces, &registry, OWNER, "work", 1, "  ", T0 + 3),
            Err(VisitorError::BadReason)
        );
        assert_eq!(
            visitors.owner_override(
                &mut surfaces,
                &registry,
                OWNER,
                "work",
                1,
                "agent stalled for a day",
                T0 + 3
            ),
            Ok(())
        );
        let board = surfaces
            .board(&registry, &SurfaceActor::owner(OWNER), "work", T0 + 3)
            .unwrap();
        assert!(board.items[0].claim.is_none());
        let last = visitors.events().last().unwrap();
        assert_eq!(last.kind, EVENT_OWNER_OVERRIDE);
        assert_eq!(last.note.as_deref(), Some("agent stalled for a day"));
        assert_eq!(last.outcome, "executed");
        // A second override has nothing to release.
        assert_eq!(
            visitors.owner_override(&mut surfaces, &registry, OWNER, "work", 1, "again", T0 + 4),
            Err(VisitorError::Surface(SurfaceError::NotHolder))
        );
    }

    /// Acceptance 3: presence honesty — agents render as agents with
    /// their owner, an agent key cannot enter as a human, and an agent
    /// without an owner is refused.
    #[test]
    fn presence_is_honest() {
        let registry = registry();
        let mut roster = PresenceRoster::new(WORLD);
        roster
            .enter(&registry, Entrant::Agent(presence(1, "indexer", "Ada")), T0)
            .unwrap();
        roster
            .enter(&registry, Entrant::Agent(presence(9, "scribe", "Ada")), T0)
            .unwrap();
        assert_eq!(
            roster.enter(&registry, Entrant::Agent(presence(7, "ghost", " ")), T0),
            Err(VisitorError::MissingOwner)
        );
        // Agent 1 holds a grant; agent 9 is present as an agent.
        for key in [[1; 32], [9; 32]] {
            assert_eq!(
                roster.enter(
                    &registry,
                    Entrant::Human {
                        key,
                        name: "Totally Human".to_owned(),
                        role: VISITOR_ROLE.to_owned(),
                    },
                    T0
                ),
                Err(VisitorError::AgentAsHuman)
            );
        }
        roster
            .enter(
                &registry,
                Entrant::Human {
                    key: HUMAN,
                    name: "Grace".to_owned(),
                    role: VISITOR_ROLE.to_owned(),
                },
                T0,
            )
            .unwrap();

        let view = roster.view(&WorldProfile::agent_majority());
        assert_eq!((view.agents, view.humans), (2, 1));
        assert_eq!(view.lines[0].kind, ParticipantKind::Human);
        assert_eq!(view.lines[0].display, "Grace (visitor)");
        for line in &view.lines[1..] {
            assert_eq!(line.kind, ParticipantKind::Agent);
            assert_eq!(line.owner.as_deref(), Some("Ada"));
            assert!(line.display.contains("agent of Ada"), "{}", line.display);
        }
        assert_eq!(
            view.agents_by_owner["Ada"],
            ["indexer".to_owned(), "scribe".to_owned()]
        );
        // An ordinary world lists by name; honesty is the same.
        let plain = roster.view(&WorldProfile::ordinary());
        let names: Vec<&str> = plain.lines.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Grace", "indexer", "scribe"]);

        let refused = roster
            .events()
            .iter()
            .filter(|e| e.outcome.starts_with("refused:"))
            .count();
        assert_eq!(refused, 3);
        roster.leave(&HUMAN);
        assert_eq!(roster.view(&WorldProfile::ordinary()).humans, 0);
    }

    /// Acceptance 4: the visitor summary is the delta since the last
    /// presence, rendered from the audit log alone.
    #[test]
    fn visitor_summary_renders_the_delta_from_the_audit_log() {
        let registry = registry();
        let mut surfaces = surfaces_with_agent_claim(&registry);
        let mut visitors = Visitors::new(WORLD, OWNER);
        visitors
            .grant(&registry, OWNER, HUMAN, VISITOR_ROLE, T0)
            .unwrap();
        assert_eq!(
            visitors.arrive(&HUMAN2, surfaces.events(), T0),
            Err(VisitorError::NotAVisitor)
        );

        let first = visitors.arrive(&HUMAN, surfaces.events(), T0 + 1).unwrap();
        assert_eq!(first.since, None);
        let work = &first.surfaces[&SurfaceId::Board("work".to_owned())];
        assert_eq!(work.executed.values().sum::<u64>(), 3);
        visitors.depart(&HUMAN, T0 + 1);
        assert_eq!(visitors.last_seen(&HUMAN), Some(T0 + 1));

        let quiet = visitors.arrive(&HUMAN, surfaces.events(), T0 + 2).unwrap();
        assert!(quiet.is_quiet());
        assert_eq!(quiet.to_text(), "nothing changed since your last visit");

        // The claim's lease runs out; an observer is refused a claim.
        let lease = 60 * 60;
        surfaces.expire_claims(T0 + 1 + lease);
        let stranger = SurfaceActor::agent(agent(2), "participant");
        assert!(
            surfaces
                .claim(&registry, &stranger, "work", 1, 99, T0 + 2 + lease)
                .is_err()
        );
        let later = visitors
            .arrive(&HUMAN, surfaces.events(), T0 + 3 + lease)
            .unwrap();
        assert_eq!(later.since, Some(T0 + 1));
        let work = &later.surfaces[&SurfaceId::Board("work".to_owned())];
        assert_eq!(work.executed.get(EVENT_CLAIM_EXPIRE), Some(&1));
        assert_eq!(work.refused, 1);
        assert_eq!(work.stalled_items, BTreeSet::from([1]));
        let text = later.to_text();
        assert!(
            text.starts_with("board work: 1× surface.item.claim_expire"),
            "{text}"
        );
        assert!(text.contains("1 refused"), "{text}");
        assert!(text.contains("stalled #1"), "{text}");
    }
}
