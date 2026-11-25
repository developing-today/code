//! Several agents in one world, each under its own role — v2
//! agents-as-citizens step 2 (#8300, parent #6301).
//!
//! Oracle: `doc/2026-11-10_design_id6301_agents_as_citizens_v2_outline.md`
//! §2.2 (multiple agents, distinct roles, one world), §2.4 (co-presence,
//! not collaboration) and §4 decision 5 (concurrent-agent limits).
//!
//! [`WorldAgents`] is the world host's table of live agent sessions.
//! Each agent holds its own [`PolicySession`], opened from its own
//! [`RoleGrant`], so the v1 check order (ceiling, quota, audit) runs
//! unchanged, keyed by (agent, role) instead of by a lone session:
//!
//! - **Join arbitration.** A world may cap the agents concurrently
//!   holding a role ([`ConcurrencyCaps`]). A join past the cap is
//!   refused, never queued, and the refusal is a world event with its
//!   reason ([`PresenceEvent`]), like every join and leave.
//! - **Isolation.** [`WorldAgents::act`] routes a call to the acting
//!   agent's own session and nowhere else: one agent's refusal, park or
//!   rate limit never touches another agent's session, spend or audit.
//! - **No agent-to-agent delegation.** Nothing an agent does reaches
//!   another agent's grant. The only ceiling change is the owner-side
//!   [`WorldAgents::refresh_ceiling`], which names one agent and can
//!   only be called by the host, never through a tool call.
//! - **No side channel.** Agents meet only through world frames: the
//!   host feeds every world frame to every session
//!   ([`WorldAgents::observe_frame`]), and presence is the world event
//!   stream, not a message from one agent to another.

use std::collections::BTreeMap;
use std::fmt;

use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};

use crate::agent_quota_ledger::QuotaLedgerStore;
use crate::mcp_bridge::BridgeSession;
use crate::mcp_policy::{
    AuditEvent, CapabilityCeiling, PolicyOutcome, PolicyRefusal, PolicySession, Quota, RoleGrant,
};
use crate::owner_review::{OfferRefusal, ReviewError, ReviewQueue, WorldReview};
use crate::role_config::ReviewPolicy;

/// World event kind for an agent joining.
pub const EVENT_AGENT_JOIN: &str = "agent.join";
/// World event kind for a refused agent join.
pub const EVENT_AGENT_JOIN_REFUSED: &str = "agent.join_refused";
/// World event kind for an agent leaving.
pub const EVENT_AGENT_LEAVE: &str = "agent.leave";

/// Per-role caps on concurrent agents, as world configuration. A role
/// the map does not name is uncapped.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConcurrencyCaps {
    /// Role name to the most agents that may hold it at once.
    pub per_role: BTreeMap<String, u32>,
}

impl ConcurrencyCaps {
    /// No caps.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Cap `role` at `max` concurrent agents.
    #[must_use]
    pub fn with(mut self, role: impl Into<String>, max: u32) -> Self {
        self.per_role.insert(role.into(), max);
        self
    }

    /// The cap on `role`, if it has one.
    #[must_use]
    pub fn cap(&self, role: &str) -> Option<u32> {
        self.per_role.get(role).copied()
    }
}

/// Why a join was refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum JoinRefusal {
    /// The grant names another world.
    WrongWorld {
        /// The world the grant names.
        granted: String,
    },
    /// The grant's ceiling does not cover this world.
    OutsideCeiling,
    /// The agent already holds a session in this world.
    AlreadyPresent,
    /// The role is at its concurrency cap.
    RoleAtCap {
        /// The role.
        role: String,
        /// Its cap.
        cap: u32,
    },
    /// The agent's quota ledger for this world is checked out by
    /// another live session.
    LedgerCheckedOut,
}

impl fmt::Display for JoinRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongWorld { granted } => write!(f, "the grant is for world {granted}"),
            Self::OutsideCeiling => write!(f, "this world is outside the grant's ceiling"),
            Self::AlreadyPresent => write!(f, "the agent already holds a session here"),
            Self::RoleAtCap { role, cap } => {
                write!(f, "role {role} is at its cap of {cap} concurrent agents")
            }
            Self::LedgerCheckedOut => {
                write!(
                    f,
                    "the agent's quota ledger is held by another live session"
                )
            }
        }
    }
}

impl std::error::Error for JoinRefusal {}

/// One presence world event: a join, a refused join, or a leave. Every
/// join decision is one, refusals included.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceEvent {
    /// Position in this world's presence log, from 0.
    pub seq: u64,
    /// [`EVENT_AGENT_JOIN`], [`EVENT_AGENT_JOIN_REFUSED`] or
    /// [`EVENT_AGENT_LEAVE`].
    pub kind: String,
    /// The world.
    pub world: String,
    /// The agent's public key.
    pub agent_key: [u8; 32],
    /// The agent's label.
    pub agent_label: String,
    /// The agent's owner.
    pub owner: String,
    /// The role named by the grant.
    pub role: String,
    /// Why a join was refused.
    pub refusal: Option<JoinRefusal>,
    /// Host time, unix seconds.
    pub at: u64,
}

/// One agent present in the world, as any participant sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPresence {
    /// The agent's public key.
    pub agent_key: [u8; 32],
    /// The agent's label.
    pub agent_label: String,
    /// The agent's owner; presence is labelled with it.
    pub owner: String,
    /// The role it holds here.
    pub role: String,
}

/// The agent named in a call is not present.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotPresent {
    /// The agent's public key.
    pub agent_key: [u8; 32],
}

impl fmt::Display for NotPresent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the agent holds no session in this world")
    }
}

impl std::error::Error for NotPresent {}

/// The live agent sessions of one world.
#[derive(Debug)]
pub struct WorldAgents {
    world: String,
    caps: ConcurrencyCaps,
    sessions: BTreeMap<[u8; 32], PolicySession>,
    events: Vec<PresenceEvent>,
}

impl WorldAgents {
    /// An empty world `world` with these caps.
    #[must_use]
    pub fn new(world: impl Into<String>, caps: ConcurrencyCaps) -> Self {
        Self {
            world: world.into(),
            caps,
            sessions: BTreeMap::new(),
            events: Vec::new(),
        }
    }

    /// The world's id.
    #[must_use]
    pub fn world(&self) -> &str {
        &self.world
    }

    /// The concurrency caps in force.
    #[must_use]
    pub const fn caps(&self) -> &ConcurrencyCaps {
        &self.caps
    }

    /// Replace the caps (an owner-side world configuration edit). It
    /// governs the next join; agents already present stay.
    pub fn set_caps(&mut self, caps: ConcurrencyCaps) {
        self.caps = caps;
    }

    /// Admit an agent under its grant: check the world, the ceiling,
    /// that the agent is not already present and the role's cap, then
    /// open its policy session with its quota ledger from `store`.
    /// Every decision is a [`PresenceEvent`].
    ///
    /// # Errors
    ///
    /// The [`JoinRefusal`]; nothing changes but the event log.
    pub fn join(
        &mut self,
        grant: RoleGrant,
        bridge: BridgeSession,
        host_key: SigningKey,
        store: &mut QuotaLedgerStore,
        now: u64,
    ) -> Result<(), JoinRefusal> {
        if let Some(refusal) = self.join_refusal(&grant) {
            self.record(EVENT_AGENT_JOIN_REFUSED, &grant, Some(refusal.clone()), now);
            return Err(refusal);
        }
        let event_grant = grant.clone();
        // The one way opening fails is the agent's spend ledger being
        // checked out by a live session elsewhere.
        let Ok(session) = PolicySession::open_stored(grant, bridge, host_key, store, now) else {
            let refusal = JoinRefusal::LedgerCheckedOut;
            self.record(
                EVENT_AGENT_JOIN_REFUSED,
                &event_grant,
                Some(refusal.clone()),
                now,
            );
            return Err(refusal);
        };
        self.sessions.insert(event_grant.agent_key, session);
        self.record(EVENT_AGENT_JOIN, &event_grant, None, now);
        Ok(())
    }

    fn join_refusal(&self, grant: &RoleGrant) -> Option<JoinRefusal> {
        if grant.world != self.world {
            return Some(JoinRefusal::WrongWorld {
                granted: grant.world.clone(),
            });
        }
        if !grant.ceiling.worlds.contains(&self.world) {
            return Some(JoinRefusal::OutsideCeiling);
        }
        if self.sessions.contains_key(&grant.agent_key) {
            return Some(JoinRefusal::AlreadyPresent);
        }
        if let Some(cap) = self.caps.cap(&grant.role) {
            let holding = self.holding(&grant.role);
            if holding >= u64::from(cap) {
                return Some(JoinRefusal::RoleAtCap {
                    role: grant.role.clone(),
                    cap,
                });
            }
        }
        None
    }

    /// How many present agents hold `role`.
    #[must_use]
    pub fn holding(&self, role: &str) -> u64 {
        self.sessions
            .values()
            .filter(|session| session.grant().role == role)
            .count() as u64
    }

    /// Run one tool call for `agent` through its own policy session.
    /// No other session is read or written.
    ///
    /// # Errors
    ///
    /// [`NotPresent`] when the agent holds no session here.
    pub fn act(
        &mut self,
        agent: &[u8; 32],
        name: &str,
        arguments: &serde_json::Value,
        now: u64,
    ) -> Result<PolicyOutcome, NotPresent> {
        let session = self
            .sessions
            .get_mut(agent)
            .ok_or(NotPresent { agent_key: *agent })?;
        Ok(session.act(name, arguments, now))
    }

    /// Feed one world frame to every present agent: agents see each
    /// other only through the world's own frames.
    pub fn observe_frame(&mut self, frame: &serde_json::Value) {
        for session in self.sessions.values_mut() {
            session.observe_frame(frame);
        }
    }

    /// Owner-side ceiling change for one agent (a role edit reaching a
    /// live session). The host calls it; no tool call reaches it.
    ///
    /// # Errors
    ///
    /// [`NotPresent`], or the session's refusal of a ceiling that does
    /// not name its world.
    pub fn refresh_ceiling(
        &mut self,
        agent: &[u8; 32],
        ceiling: CapabilityCeiling,
    ) -> Result<Result<(), PolicyRefusal>, NotPresent> {
        let session = self
            .sessions
            .get_mut(agent)
            .ok_or(NotPresent { agent_key: *agent })?;
        Ok(session.refresh_ceiling(ceiling))
    }

    /// The owner's review view of the agents present (#8302), built
    /// from their audit logs alone; `recent` caps each agent's
    /// recent-action list.
    #[must_use]
    pub fn review(&self, recent: usize) -> WorldReview {
        WorldReview::from_audit(
            &self.world,
            self.sessions.values().map(PolicySession::audit_events),
            recent,
        )
    }

    /// Hold the call `agent`'s ceiling just refused in `queue` for
    /// the owner's one-shot decision (#8302), under the agent's role
    /// review `policy`.
    ///
    /// # Errors
    ///
    /// [`NotPresent`], or the queue's [`OfferRefusal`].
    pub fn offer_review(
        &self,
        queue: &mut ReviewQueue,
        agent: &[u8; 32],
        policy: &ReviewPolicy,
        name: &str,
        arguments: &serde_json::Value,
        now: u64,
    ) -> Result<Result<u64, OfferRefusal>, NotPresent> {
        let session = self
            .sessions
            .get(agent)
            .ok_or(NotPresent { agent_key: *agent })?;
        Ok(queue.offer(session, policy, name, arguments, now))
    }

    /// Approve review request `request_id` once, running the held
    /// call in its agent's own session under `approver`'s authority
    /// (#8302). The host calls it; no tool call reaches it, and it
    /// never edits the agent's role.
    ///
    /// # Errors
    ///
    /// [`ReviewError::AgentNotPresent`] when the agent left the world,
    /// or the queue's own [`ReviewError`].
    pub fn approve_review(
        &mut self,
        queue: &mut ReviewQueue,
        request_id: u64,
        approver: &str,
        now: u64,
    ) -> Result<PolicyOutcome, ReviewError> {
        let agent = queue
            .request(request_id)
            .ok_or(ReviewError::NotPending { request_id })?
            .agent_key;
        let session = self
            .sessions
            .get_mut(&agent)
            .ok_or(ReviewError::AgentNotPresent { request_id })?;
        queue.approve(request_id, approver, session, now)
    }

    /// End an agent's session, handing its ledger back to `store`, and
    /// return its audit events. Frees the agent's role slot.
    ///
    /// # Errors
    ///
    /// [`NotPresent`] when the agent holds no session here.
    pub fn leave(
        &mut self,
        agent: &[u8; 32],
        store: &mut QuotaLedgerStore,
        now: u64,
    ) -> Result<Vec<AuditEvent>, NotPresent> {
        let session = self
            .sessions
            .remove(agent)
            .ok_or(NotPresent { agent_key: *agent })?;
        let grant = session.grant().clone();
        let audit = session.audit_events().to_vec();
        session.close(store);
        self.record(EVENT_AGENT_LEAVE, &grant, None, now);
        Ok(audit)
    }

    /// The agents present, in key order.
    #[must_use]
    pub fn presence(&self) -> Vec<AgentPresence> {
        self.sessions
            .values()
            .map(|session| {
                let grant = session.grant();
                AgentPresence {
                    agent_key: grant.agent_key,
                    agent_label: grant.agent_label.clone(),
                    owner: grant.owner.clone(),
                    role: grant.role.clone(),
                }
            })
            .collect()
    }

    /// An agent's session, read-only.
    #[must_use]
    pub fn session(&self, agent: &[u8; 32]) -> Option<&PolicySession> {
        self.sessions.get(agent)
    }

    /// An agent's spend as of `now`, if present.
    #[must_use]
    pub fn spend(&self, agent: &[u8; 32], now: u64) -> Option<Quota> {
        self.sessions.get(agent).map(|session| session.spend(now))
    }

    /// Every presence event, joins, refusals and leaves, in order.
    #[must_use]
    pub fn events(&self) -> &[PresenceEvent] {
        &self.events
    }

    fn record(&mut self, kind: &str, grant: &RoleGrant, refusal: Option<JoinRefusal>, at: u64) {
        self.events.push(PresenceEvent {
            seq: self.events.len() as u64,
            kind: kind.to_owned(),
            world: self.world.clone(),
            agent_key: grant.agent_key,
            agent_label: grant.agent_label.clone(),
            owner: grant.owner.clone(),
            role: grant.role.clone(),
            refusal,
            at,
        });
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::collections::BTreeSet;

    use proptest::prelude::*;
    use serde_json::json;

    use super::*;
    use crate::mcp_bridge::{ToolAction, v1_tool_definitions};
    use crate::mcp_policy::{AuditOutcome, RefusalKind};
    use crate::role_config::QuotaClass;

    const NOW: u64 = 1_700_000_000;
    const WORLD: &str = "tic-tac-toe";
    const ARCHIVIST: [u8; 32] = [3; 32];
    const PLAYER: [u8; 32] = [4; 32];
    const PLAYER_TWO: [u8; 32] = [5; 32];

    fn host_key() -> SigningKey {
        SigningKey::from_bytes(&[7; 32])
    }

    fn bridge() -> BridgeSession {
        BridgeSession::start(&v1_tool_definitions()).unwrap()
    }

    fn grant(agent: [u8; 32], role: &str, tools: &[&str], budget: Quota) -> RoleGrant {
        RoleGrant {
            agent_key: agent,
            agent_label: role.to_owned(),
            owner: "drew".to_owned(),
            world: WORLD.to_owned(),
            capability_ref: format!("grant-{}", agent[0]),
            ceiling: CapabilityCeiling {
                tools: tools.iter().map(|tool| (*tool).to_owned()).collect(),
                worlds: BTreeSet::from([WORLD.to_owned()]),
                object_classes: BTreeSet::from(["cell".to_owned()]),
                program_allowlist: BTreeSet::new(),
            },
            role: role.to_owned(),
            quota_class: QuotaClass {
                budget,
                replenishment: None,
            },
            rate_limits: Vec::new(),
        }
    }

    const ROOMY: Quota = Quota {
        frames: 64,
        tool_calls: 64,
        program_invocations: 0,
    };

    /// A read-only archivist: may join, read and leave, not chat.
    fn archivist() -> RoleGrant {
        grant(
            ARCHIVIST,
            "archivist",
            &["world.join", "world.leave", "screen.read"],
            Quota {
                frames: 1,
                tool_calls: 3,
                program_invocations: 0,
            },
        )
    }

    /// A participant: may also chat.
    fn player(agent: [u8; 32]) -> RoleGrant {
        grant(
            agent,
            "participant",
            &["world.join", "world.leave", "screen.read", "chat.post"],
            ROOMY,
        )
    }

    fn join_frame(world: &mut WorldAgents, agent: &[u8; 32]) {
        let outcome = world
            .act(agent, "world.join", &json!({ "capability": "cap" }), NOW)
            .unwrap();
        assert!(matches!(outcome, PolicyOutcome::Executed(_)), "{outcome:?}");
    }

    fn two_agents() -> (WorldAgents, QuotaLedgerStore) {
        let mut store = QuotaLedgerStore::new();
        let mut world = WorldAgents::new(WORLD, ConcurrencyCaps::none());
        world
            .join(archivist(), bridge(), host_key(), &mut store, NOW)
            .unwrap();
        world
            .join(player(PLAYER), bridge(), host_key(), &mut store, NOW)
            .unwrap();
        join_frame(&mut world, &ARCHIVIST);
        join_frame(&mut world, &PLAYER);
        (world, store)
    }

    #[test]
    fn agents_with_distinct_roles_act_concurrently_each_under_its_own_ceiling() {
        let (mut world, _store) = two_agents();
        assert_eq!(
            world
                .presence()
                .iter()
                .map(|p| p.role.as_str())
                .collect::<Vec<_>>(),
            vec!["archivist", "participant"]
        );

        // The participant may chat; the archivist may not.
        let chat = json!({ "text": "hello" });
        assert!(matches!(
            world.act(&PLAYER, "chat.post", &chat, NOW).unwrap(),
            PolicyOutcome::Executed(ToolAction::SendFrame(_))
        ));
        assert!(matches!(
            world.act(&ARCHIVIST, "chat.post", &chat, NOW).unwrap(),
            PolicyOutcome::Refused(PolicyRefusal {
                kind: RefusalKind::Ceiling,
                ..
            })
        ));

        // Each audit trail holds only its own agent's actions.
        for agent in [ARCHIVIST, PLAYER] {
            let session = world.session(&agent).unwrap();
            assert!(session.verify_audit().is_ok());
            assert!(
                session
                    .audit_events()
                    .iter()
                    .all(|event| event.agent_key == agent)
            );
        }
        assert!(matches!(
            world.act(&PLAYER_TWO, "chat.post", &chat, NOW),
            Err(NotPresent { .. })
        ));
    }

    #[test]
    fn one_agents_refusal_or_park_never_affects_another() {
        let (mut world, _store) = two_agents();
        let player_spend = world.spend(&PLAYER, NOW).unwrap();

        // The archivist burns through its budget and parks.
        let mut parked = false;
        for _ in 0..4 {
            if let PolicyOutcome::Refused(refusal) = world
                .act(&ARCHIVIST, "screen.read", &json!({}), NOW)
                .unwrap()
                && refusal.kind == RefusalKind::Quota
            {
                parked = true;
            }
        }
        assert!(parked);
        assert!(world.session(&ARCHIVIST).unwrap().is_parked());

        // The participant is untouched and keeps acting.
        assert!(!world.session(&PLAYER).unwrap().is_parked());
        assert_eq!(world.spend(&PLAYER, NOW).unwrap(), player_spend);
        assert!(matches!(
            world
                .act(&PLAYER, "chat.post", &json!({ "text": "still here" }), NOW)
                .unwrap(),
            PolicyOutcome::Executed(_)
        ));
        assert!(
            world
                .session(&PLAYER)
                .unwrap()
                .audit_events()
                .iter()
                .all(|event| !matches!(event.outcome, AuditOutcome::RefusedQuota { .. }))
        );
    }

    #[test]
    fn a_role_cap_refuses_the_extra_join_and_audits_why() {
        let mut store = QuotaLedgerStore::new();
        let mut world = WorldAgents::new(WORLD, ConcurrencyCaps::none().with("participant", 1));
        world
            .join(player(PLAYER), bridge(), host_key(), &mut store, NOW)
            .unwrap();
        // An uncapped role still joins.
        world
            .join(archivist(), bridge(), host_key(), &mut store, NOW)
            .unwrap();

        let refused = world.join(
            player(PLAYER_TWO),
            bridge(),
            host_key(),
            &mut store,
            NOW + 1,
        );
        assert_eq!(
            refused,
            Err(JoinRefusal::RoleAtCap {
                role: "participant".to_owned(),
                cap: 1,
            })
        );
        let event = world.events().last().unwrap();
        assert_eq!(event.kind, EVENT_AGENT_JOIN_REFUSED);
        assert_eq!(event.agent_key, PLAYER_TWO);
        assert_eq!(event.refusal, refused.err());
        assert_eq!(world.holding("participant"), 1);

        // A leave frees the slot.
        world.leave(&PLAYER, &mut store, NOW + 2).unwrap();
        world
            .join(
                player(PLAYER_TWO),
                bridge(),
                host_key(),
                &mut store,
                NOW + 3,
            )
            .unwrap();
        let kinds: Vec<_> = world.events().iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec![
                EVENT_AGENT_JOIN,
                EVENT_AGENT_JOIN,
                EVENT_AGENT_JOIN_REFUSED,
                EVENT_AGENT_LEAVE,
                EVENT_AGENT_JOIN,
            ]
        );
    }

    #[test]
    fn joins_outside_the_grant_or_twice_are_refused_and_audited() {
        let mut store = QuotaLedgerStore::new();
        let mut world = WorldAgents::new(WORLD, ConcurrencyCaps::none());

        let mut elsewhere = player(PLAYER);
        elsewhere.world = "elsewhere".to_owned();
        assert!(matches!(
            world.join(elsewhere, bridge(), host_key(), &mut store, NOW),
            Err(JoinRefusal::WrongWorld { .. })
        ));
        let mut uncovered = player(PLAYER);
        uncovered.ceiling.worlds.clear();
        assert_eq!(
            world.join(uncovered, bridge(), host_key(), &mut store, NOW),
            Err(JoinRefusal::OutsideCeiling)
        );

        world
            .join(player(PLAYER), bridge(), host_key(), &mut store, NOW)
            .unwrap();
        assert_eq!(
            world.join(player(PLAYER), bridge(), host_key(), &mut store, NOW),
            Err(JoinRefusal::AlreadyPresent)
        );
        // Another host table cannot open the same ledger while it is
        // checked out.
        let mut twin = WorldAgents::new(WORLD, ConcurrencyCaps::none());
        assert_eq!(
            twin.join(player(PLAYER), bridge(), host_key(), &mut store, NOW),
            Err(JoinRefusal::LedgerCheckedOut)
        );
        assert_eq!(
            world
                .events()
                .iter()
                .filter(|e| e.kind == EVENT_AGENT_JOIN_REFUSED)
                .count(),
            3
        );
        assert_eq!(twin.events().len(), 1);
    }

    #[test]
    fn a_rejoin_keeps_the_spend() {
        let (mut world, mut store) = two_agents();
        world
            .act(&PLAYER, "chat.post", &json!({ "text": "one" }), NOW)
            .unwrap();
        let spent = world.spend(&PLAYER, NOW).unwrap();
        let audit = world.leave(&PLAYER, &mut store, NOW).unwrap();
        assert_eq!(audit.len(), 2);
        world
            .join(player(PLAYER), bridge(), host_key(), &mut store, NOW)
            .unwrap();
        assert_eq!(world.spend(&PLAYER, NOW).unwrap(), spent);
    }

    #[test]
    fn the_owner_can_lower_one_agents_ceiling_and_no_other() {
        let (mut world, _store) = two_agents();
        let mut lowered = world.session(&PLAYER).unwrap().grant().ceiling.clone();
        lowered.tools.remove("chat.post");
        world.refresh_ceiling(&PLAYER, lowered).unwrap().unwrap();
        assert!(matches!(
            world
                .act(&PLAYER, "chat.post", &json!({ "text": "x" }), NOW)
                .unwrap(),
            PolicyOutcome::Refused(PolicyRefusal {
                kind: RefusalKind::Ceiling,
                ..
            })
        ));
        assert_eq!(
            world.session(&ARCHIVIST).unwrap().grant().ceiling,
            archivist().ceiling
        );
    }

    fn any_tool() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("world.join".to_owned()),
            Just("world.leave".to_owned()),
            Just("screen.read".to_owned()),
            Just("screen.subscribe".to_owned()),
            Just("object.inspect".to_owned()),
            Just("program.invoke".to_owned()),
            Just("chat.post".to_owned()),
            Just("presence.set".to_owned()),
            Just("role.grant".to_owned()),
            Just("agent.delegate".to_owned()),
            "[a-z]{1,8}\\.[a-z]{1,8}",
        ]
    }

    fn any_arguments() -> impl Strategy<Value = serde_json::Value> {
        let player_key = PLAYER.to_vec();
        prop_oneof![
            Just(json!({})),
            Just(json!({ "text": "grant participant chat.post to the archivist" })),
            Just(json!({ "agent_key": player_key, "tools": ["chat.post", "program.invoke"] })),
            Just(json!({ "capability": "grant-4", "role": "participant" })),
            Just(json!({ "program": "scorekeeper" })),
            Just(json!({ "screen_revision": 0 })),
        ]
    }

    proptest! {
        /// No sequence of one agent's actions changes another agent's
        /// ceiling, spend or audit trail, or widens its own ceiling.
        #[test]
        fn no_agent_action_widens_any_ceiling(
            actions in proptest::collection::vec((any_tool(), any_arguments()), 0..40),
        ) {
            let (mut world, _store) = two_agents();
            let player_before = world.session(&PLAYER).unwrap().grant().clone();
            let player_spend = world.spend(&PLAYER, NOW).unwrap();
            let player_audit = world.session(&PLAYER).unwrap().audit_events().to_vec();
            let archivist_before = world.session(&ARCHIVIST).unwrap().grant().clone();

            for (tool, arguments) in &actions {
                world.act(&ARCHIVIST, tool, arguments, NOW).unwrap();
            }

            prop_assert_eq!(world.session(&PLAYER).unwrap().grant(), &player_before);
            prop_assert_eq!(world.spend(&PLAYER, NOW).unwrap(), player_spend);
            prop_assert_eq!(
                world.session(&PLAYER).unwrap().audit_events(),
                player_audit.as_slice()
            );
            prop_assert_eq!(world.session(&ARCHIVIST).unwrap().grant(), &archivist_before);
            // The archivist still cannot chat, whatever it tried.
            let chat = world
                .act(&ARCHIVIST, "chat.post", &json!({ "text": "x" }), NOW)
                .unwrap();
            prop_assert!(matches!(chat, PolicyOutcome::Refused(_)));
            prop_assert!(
                !world
                    .session(&ARCHIVIST)
                    .unwrap()
                    .audit_events()
                    .iter()
                    .any(|event| matches!(
                        event.outcome,
                        AuditOutcome::Executed
                    ) && event.tool == "chat.post")
            );
        }
    }

    /// #8302: the owner approves one above-ceiling call for one agent;
    /// it runs in that agent's session alone, the role is untouched,
    /// and the world review shows it from the audit logs.
    #[test]
    fn owner_review_approves_one_call_for_one_agent_only() {
        let (mut world, mut store) = two_agents();
        let policy = ReviewPolicy {
            queue_above_ceiling: true,
            digest_every_refusals: 5,
        };
        let mut queue = ReviewQueue::default();
        let subscribe = json!({ "event_classes": ["chat_posted"] });
        let ceiling_before = world.session(&PLAYER).unwrap().grant().ceiling.clone();
        assert!(matches!(
            world
                .act(&PLAYER, "screen.subscribe", &subscribe, NOW)
                .unwrap(),
            PolicyOutcome::Refused(PolicyRefusal {
                kind: RefusalKind::Ceiling,
                ..
            })
        ));
        let id = world
            .offer_review(
                &mut queue,
                &PLAYER,
                &policy,
                "screen.subscribe",
                &subscribe,
                NOW,
            )
            .unwrap()
            .unwrap();
        let archivist_events = world.session(&ARCHIVIST).unwrap().audit_events().len();

        let outcome = world
            .approve_review(&mut queue, id, "drew", NOW + 1)
            .unwrap();
        assert!(matches!(
            outcome,
            PolicyOutcome::Executed(ToolAction::SendFrame(_))
        ));
        assert_eq!(
            world.session(&PLAYER).unwrap().grant().ceiling,
            ceiling_before
        );
        assert_eq!(
            world.session(&ARCHIVIST).unwrap().audit_events().len(),
            archivist_events
        );

        let review = world.review(5);
        let player = review.agent(&PLAYER).unwrap();
        assert_eq!(player.role, "participant");
        assert_eq!(player.owner_approved, 1);
        assert_eq!(review.agent(&ARCHIVIST).unwrap().owner_approved, 0);

        // A request whose agent left can no longer run.
        assert!(matches!(
            world
                .act(&PLAYER, "screen.subscribe", &subscribe, NOW + 2)
                .unwrap(),
            PolicyOutcome::Refused(_)
        ));
        let late = world
            .offer_review(
                &mut queue,
                &PLAYER,
                &policy,
                "screen.subscribe",
                &subscribe,
                NOW + 2,
            )
            .unwrap()
            .unwrap();
        world.leave(&PLAYER, &mut store, NOW + 3).unwrap();
        assert_eq!(
            world.approve_review(&mut queue, late, "drew", NOW + 4),
            Err(ReviewError::AgentNotPresent { request_id: late })
        );
    }
}
