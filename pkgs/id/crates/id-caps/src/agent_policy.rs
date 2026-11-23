//! The host-side policy point for agent frames (spike 4/5 for #6300,
//! issue #7916).
//!
//! Oracle: `doc/2026-11-09_design_id6300_agents_in_worlds_v1.md` §6
//! (guardrails: capability ceiling, quota, behavioural limits), §7
//! (audit log) and §9.3 (the spike's quota is a local, per-session
//! budget in the event shape F12.10 will consume — it does not wait
//! for #1067 to land).
//!
//! The policy point sits in the world host, outside the agent's
//! execution environment (§3): every frame the agent causes is checked
//! against (1) the capability ceiling set at role grant, (2) the
//! per-session quota ledger — charged *before* execution — and only
//! then executed and (3) written to the audit log, refusals included.
//! Refusals are the highest-signal events and are never sampled away
//! (§7). The agent is the audit subject, never its editor: this module
//! exposes no operation that mutates the log or the ledger from the
//! agent side, and the log is hash-chained and host-signed so a
//! hosted operator cannot quietly edit an agent's history either.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use serde::Serialize;
use sha2::{Digest as _, Sha256};

use crate::agent_delegation::{AgentId, JoinGrant};
use crate::world_scopes::WorldScopes;

/// Ceiling key for the presence tools (`world.join`, `world.leave`,
/// `presence.set`), per the §4.4 tool table.
pub const CEILING_PRESENCE: &str = "presence";
/// Ceiling key for the screen tools (`screen.read`,
/// `screen.subscribe`, `object.inspect`), per §4.4.
pub const CEILING_SCREEN_READ: &str = "screen.read";
/// Ceiling key for `chat.post`, per §4.4.
pub const CEILING_CHAT_POST: &str = "chat.post";
/// Base ceiling key for the one bounded program interaction (§4.6);
/// the full key is `program.invoke:<program-id>`.
pub const CEILING_PROGRAM_INVOKE: &str = "program.invoke";

/// One agent action presented to the policy point: a single frame the
/// agent causes, described in the terms the ceiling and quota check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentAction {
    /// MCP tool name, e.g. `chat.post` (the §4.4 surface).
    pub tool: String,
    /// Ceiling key from the pinned tool definition (§4.4):
    /// `presence`, `screen.read`, `chat.post`, or
    /// `program.invoke:<program-id>`.
    pub ceiling_key: String,
    /// Object/entity reference the action acts on, when it acts on
    /// one. Recorded in the audit event (§7).
    pub object: Option<String>,
    /// Class of the object acted on, when known (§6.1 names object
    /// classes as part of the ceiling).
    pub object_class: Option<String>,
    /// Screen revision the action was decided against (§4.3
    /// stale-decision token), when the frame carries one.
    pub decided_revision: Option<u64>,
}

impl AgentAction {
    /// An action for `tool` under `ceiling_key`, acting on no object
    /// and carrying no stale-decision token.
    #[must_use]
    pub fn new(tool: impl Into<String>, ceiling_key: impl Into<String>) -> Self {
        Self {
            tool: tool.into(),
            ceiling_key: ceiling_key.into(),
            object: None,
            object_class: None,
            decided_revision: None,
        }
    }

    /// This action acting on `object` of class `object_class`.
    #[must_use]
    pub fn with_object(
        mut self,
        object: impl Into<String>,
        object_class: impl Into<String>,
    ) -> Self {
        self.object = Some(object.into());
        self.object_class = Some(object_class.into());
        self
    }

    /// This action carrying the §4.3 stale-decision token: it was
    /// decided against screen revision `revision`.
    #[must_use]
    pub const fn with_decided_revision(mut self, revision: u64) -> Self {
        self.decided_revision = Some(revision);
        self
    }

    /// Whether this action invokes a program (§4.6), and so charges
    /// the program-invocation quota axis.
    #[must_use]
    pub fn is_program_invoke(&self) -> bool {
        self.ceiling_key == CEILING_PROGRAM_INVOKE
            || self.ceiling_key.starts_with("program.invoke:")
    }
}

/// The capability ceiling (§6.1).
///
/// Set at role grant, enforced at the policy point on every frame,
/// never by the model. The ceiling names tools, worlds, object
/// classes, and the program-invoke allowlist. It can only be changed
/// owner-side, out-of-band — nothing in this module widens one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ceiling {
    /// Base ceiling keys the agent may use at all (§4.4):
    /// `presence`, `screen.read`, `chat.post`, `program.invoke`.
    tools: BTreeSet<String>,
    /// Worlds the ceiling applies to.
    worlds: BTreeSet<String>,
    /// Object classes the agent may act on. An action naming a class
    /// outside this set is refused; an empty set refuses every
    /// class-named action (deny by default).
    object_classes: BTreeSet<String>,
    /// Program ids the agent may invoke (§4.6). A `program.invoke`
    /// action for a program outside this list is refused even when
    /// the base `program.invoke` key is in `tools`.
    programs: BTreeSet<String>,
}

impl Ceiling {
    /// A ceiling naming exactly the given tools, worlds, object
    /// classes, and program allowlist.
    #[must_use]
    pub fn new(
        tools: Vec<String>,
        worlds: Vec<String>,
        object_classes: Vec<String>,
        programs: Vec<String>,
    ) -> Self {
        Self {
            tools: tools.into_iter().collect(),
            worlds: worlds.into_iter().collect(),
            object_classes: object_classes.into_iter().collect(),
            programs: programs.into_iter().collect(),
        }
    }

    /// The ceiling a join grant carries (§5.2, §6.1): the tool keys
    /// derived from the grant's scopes, in the grant's one world,
    /// with no object classes and an empty program allowlist — the
    /// owner widens those two out-of-band, never the agent.
    ///
    /// Scope mapping (§4.4 against [`WorldScopes`]): `JOIN` carries
    /// `presence`; chat-reading scopes carry `screen.read`; `CHAT`
    /// carries `chat.post`; `INPUT` carries the base `program.invoke`
    /// key (each invocation still needs its program on the allowlist).
    #[must_use]
    pub fn from_grant(grant: &JoinGrant) -> Self {
        let mut tools = BTreeSet::new();
        if grant.scopes.contains(WorldScopes::JOIN) {
            tools.insert(CEILING_PRESENCE.to_owned());
        }
        if grant.scopes.reads_chat() {
            tools.insert(CEILING_SCREEN_READ.to_owned());
        }
        if grant.scopes.contains(WorldScopes::CHAT) {
            tools.insert(CEILING_CHAT_POST.to_owned());
        }
        if grant.scopes.contains(WorldScopes::INPUT) {
            tools.insert(CEILING_PROGRAM_INVOKE.to_owned());
        }
        Self {
            tools,
            worlds: BTreeSet::from([grant.world.clone()]),
            object_classes: BTreeSet::new(),
            programs: BTreeSet::new(),
        }
    }

    /// This ceiling with the object classes the owner allows,
    /// replacing any previously set (an owner-side, out-of-band act).
    #[must_use]
    pub fn with_object_classes(mut self, object_classes: Vec<String>) -> Self {
        self.object_classes = object_classes.into_iter().collect();
        self
    }

    /// This ceiling with the program allowlist the owner allows,
    /// replacing any previously set (an owner-side, out-of-band act).
    #[must_use]
    pub fn with_programs(mut self, programs: Vec<String>) -> Self {
        self.programs = programs.into_iter().collect();
        self
    }

    /// Base ceiling keys in force.
    #[must_use]
    pub const fn tools(&self) -> &BTreeSet<String> {
        &self.tools
    }

    /// Worlds in force.
    #[must_use]
    pub const fn worlds(&self) -> &BTreeSet<String> {
        &self.worlds
    }

    /// Object classes in force.
    #[must_use]
    pub const fn object_classes(&self) -> &BTreeSet<String> {
        &self.object_classes
    }

    /// Program allowlist in force.
    #[must_use]
    pub const fn programs(&self) -> &BTreeSet<String> {
        &self.programs
    }

    /// The ceiling stage of the policy point (§6.1): anything outside
    /// the ceiling is refused (and audited by the caller).
    ///
    /// # Errors
    ///
    /// Returns the specific [`PolicyRefusal`] for the first failed
    /// check: program allowlist, tool key, then object class.
    pub fn check(&self, action: &AgentAction) -> Result<(), PolicyRefusal> {
        if let Some(program) = action.ceiling_key.strip_prefix("program.invoke:") {
            if !self.tools.contains(CEILING_PROGRAM_INVOKE) || !self.programs.contains(program) {
                return Err(PolicyRefusal::CeilingProgram);
            }
        } else if !self.tools.contains(&action.ceiling_key) {
            return Err(PolicyRefusal::CeilingTool);
        }
        if let Some(class) = &action.object_class
            && !self.object_classes.contains(class)
        {
            return Err(PolicyRefusal::CeilingObjectClass);
        }
        Ok(())
    }
}

/// Quota amounts over the §6.2 axes: frames requested, tool calls,
/// program invocations. One shape serves as the session budget, the
/// running spend, and the per-event charge recorded in the audit log
/// (§7).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Quota {
    /// Frames requested.
    pub frames: u64,
    /// Tool calls made.
    pub tool_calls: u64,
    /// Program invocations made.
    pub program_invocations: u64,
}

impl Quota {
    /// No charge on any axis.
    pub const ZERO: Self = Self {
        frames: 0,
        tool_calls: 0,
        program_invocations: 0,
    };

    /// The charge for one admitted action: one frame and one tool
    /// call, plus one program invocation for a §4.6 program action.
    #[must_use]
    pub fn charge_for(action: &AgentAction) -> Self {
        Self {
            frames: 1,
            tool_calls: 1,
            program_invocations: u64::from(action.is_program_invoke()),
        }
    }

    /// This amount plus `other`, saturating per axis.
    #[must_use]
    pub const fn plus(self, other: Self) -> Self {
        Self {
            frames: self.frames.saturating_add(other.frames),
            tool_calls: self.tool_calls.saturating_add(other.tool_calls),
            program_invocations: self
                .program_invocations
                .saturating_add(other.program_invocations),
        }
    }

    /// Whether this amount stays within `budget` on every axis.
    #[must_use]
    pub const fn within(self, budget: Self) -> bool {
        self.frames <= budget.frames
            && self.tool_calls <= budget.tool_calls
            && self.program_invocations <= budget.program_invocations
    }
}

/// A per-tool rate limit (§6.3).
///
/// At most `max_calls` admitted calls to `tool` within any
/// `window_secs` window. Rate limits apply in addition to quota;
/// `chat.post` is the spam-prone surface and its limit is part of
/// the role.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RateLimit {
    /// Tool the limit applies to, e.g. `chat.post`.
    pub tool: String,
    /// Admitted calls allowed inside the window.
    pub max_calls: u64,
    /// Window length, seconds.
    pub window_secs: u64,
}

/// Why the policy point refused an action. Each variant maps to a
/// stable machine-readable [`PolicyRefusal::code`], journaled in the
/// audit event's outcome as `refused:<code>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyRefusal {
    /// The session is parked (§6.2): quota exhaustion ended it, and
    /// every later action is refused until the owner intervenes.
    SessionParked,
    /// The delegation behind the session has expired (§5.3).
    DelegationExpired,
    /// The ceiling does not cover the session's world (§6.1).
    CeilingWorld,
    /// The action's ceiling key is not in the ceiling (§6.1).
    CeilingTool,
    /// The action names an object class outside the ceiling (§6.1).
    CeilingObjectClass,
    /// The action invokes a program outside the allowlist (§4.6,
    /// §6.1).
    CeilingProgram,
    /// The action was decided against a screen revision that is no
    /// longer current (§4.3): the agent may not act on unseen state.
    StaleInput,
    /// The action exceeds the tool's per-role rate limit (§6.3).
    RateLimited,
    /// The action's charge would exceed the session budget (§6.2).
    /// This refusal parks the session.
    QuotaExhausted,
}

impl PolicyRefusal {
    /// Stable machine-readable refusal code, as journaled in §7
    /// events.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::SessionParked => "session_parked",
            Self::DelegationExpired => "delegation_expired",
            Self::CeilingWorld => "ceiling_world",
            Self::CeilingTool => "ceiling_tool",
            Self::CeilingObjectClass => "ceiling_object_class",
            Self::CeilingProgram => "ceiling_program",
            Self::StaleInput => "stale_input",
            Self::RateLimited => "rate_limited",
            Self::QuotaExhausted => "quota_exhausted",
        }
    }
}

/// One §7 audit event for an agent action.
///
/// Agent key, owner, world, tool, object, capability proof
/// reference, quota charged, outcome. Refusals and stale-input
/// rejections are events exactly like executions — one event per
/// action, never sampled away.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyAuditEvent {
    /// Agent key (hex).
    pub agent: String,
    /// Owner key the authority derives from (hex).
    pub owner: String,
    /// World the session is in.
    pub world: String,
    /// Tool surface, e.g. `chat.post`; the join event uses
    /// `world.join`.
    pub tool: String,
    /// Object acted on, when the action names one.
    pub object: Option<String>,
    /// §7 capability proof reference of the session's delegation.
    pub proof_ref: Option<String>,
    /// Quota charged by this action (zero for a refusal: the charge
    /// is refused atomically, never partially applied).
    pub quota_charged: Quota,
    /// `granted` (the join), `executed`, or `refused:<code>`.
    pub outcome: String,
    /// Host time of the event, unix seconds.
    pub at: u64,
}

impl PolicyAuditEvent {
    /// Canonical bytes for the hash chain: length-prefixed fields in
    /// a fixed order under a domain tag.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        push_field(&mut out, b"agent-audit/v1");
        push_field(&mut out, self.agent.as_bytes());
        push_field(&mut out, self.owner.as_bytes());
        push_field(&mut out, self.world.as_bytes());
        push_field(&mut out, self.tool.as_bytes());
        push_field(&mut out, &option_field(self.object.as_deref()));
        push_field(&mut out, &option_field(self.proof_ref.as_deref()));
        push_field(&mut out, &self.quota_charged.frames.to_be_bytes());
        push_field(&mut out, &self.quota_charged.tool_calls.to_be_bytes());
        push_field(
            &mut out,
            &self.quota_charged.program_invocations.to_be_bytes(),
        );
        push_field(&mut out, self.outcome.as_bytes());
        push_field(&mut out, &self.at.to_be_bytes());
        out
    }
}

/// One audit event sealed into the hash chain (§7).
///
/// Its sequence number, the previous event's hash, its own hash over
/// the canonical event bytes, and the host's signature over that
/// hash — so neither the agent nor a hosted operator can quietly
/// edit history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainedAuditEvent {
    /// Position in the chain, from 0 (the join event).
    pub seq: u64,
    /// Hash of the previous event; all-zero for the first.
    pub prev_hash: [u8; 32],
    /// The event itself.
    pub event: PolicyAuditEvent,
    /// `Sha256(prev_hash || seq || canonical event bytes)`.
    pub hash: [u8; 32],
    /// Host Ed25519 signature over `hash`.
    pub signature: [u8; 64],
}

/// Why an audit chain failed verification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditChainError {
    /// An event's sequence number is not its position in the chain.
    SequenceGap,
    /// An event's `prev_hash` does not match its predecessor's hash.
    PrevHashMismatch,
    /// An event's recorded hash does not match its contents.
    HashMismatch,
    /// An event's host signature does not verify.
    BadSignature,
}

/// The session audit log (§7): append-only, hash-chained,
/// host-signed. Appending happens only inside the policy point; the
/// public surface is reading and verifying.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuditLog {
    events: Vec<ChainedAuditEvent>,
}

impl AuditLog {
    /// An empty log.
    #[must_use]
    pub const fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// Every event in order, join first. One event exists per action
    /// the policy point has seen — refusals included, never sampled.
    #[must_use]
    pub fn events(&self) -> &[ChainedAuditEvent] {
        &self.events
    }

    /// Number of events recorded.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether no event has been recorded yet.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Verify this log's chain end-to-end against the host key.
    ///
    /// # Errors
    ///
    /// Returns the first [`AuditChainError`] found.
    pub fn verify(&self, host: &VerifyingKey) -> Result<(), AuditChainError> {
        Self::verify_events(&self.events, host)
    }

    /// Verify an exported chain end-to-end: sequence, hash links,
    /// event hashes, and host signatures are all re-derived from the
    /// events themselves, so any edit to any event is detected.
    ///
    /// # Errors
    ///
    /// Returns the first [`AuditChainError`] found.
    pub fn verify_events(
        events: &[ChainedAuditEvent],
        host: &VerifyingKey,
    ) -> Result<(), AuditChainError> {
        let mut prev_hash = [0u8; 32];
        for (index, chained) in events.iter().enumerate() {
            if chained.seq != index as u64 {
                return Err(AuditChainError::SequenceGap);
            }
            if chained.prev_hash != prev_hash {
                return Err(AuditChainError::PrevHashMismatch);
            }
            if chain_hash(&chained.prev_hash, chained.seq, &chained.event) != chained.hash {
                return Err(AuditChainError::HashMismatch);
            }
            host.verify(&chained.hash, &Signature::from_bytes(&chained.signature))
                .map_err(|_err| AuditChainError::BadSignature)?;
            prev_hash = chained.hash;
        }
        Ok(())
    }

    /// Append one event, sealing it into the chain under the host
    /// key. Policy-point internal: there is deliberately no public
    /// append, because the agent must never write its own history.
    fn append(&mut self, host_key: &SigningKey, event: PolicyAuditEvent) -> ChainedAuditEvent {
        let seq = self.events.len() as u64;
        let prev_hash = self.events.last().map_or([0u8; 32], |last| last.hash);
        let hash = chain_hash(&prev_hash, seq, &event);
        let signature = host_key.sign(&hash).to_bytes();
        let chained = ChainedAuditEvent {
            seq,
            prev_hash,
            event,
            hash,
            signature,
        };
        self.events.push(chained.clone());
        chained
    }
}

/// The park world event (§6.2).
///
/// Emitted when quota exhaustion parks the agent: "agent parked,
/// quota exhausted" is a first-class world event the owner sees,
/// never a silent failure. The shape is the §9.3 local budget event
/// F12.10 will consume.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ParkEvent {
    /// Event kind: always `agent_parked`.
    pub kind: String,
    /// Machine-readable reason: always `quota_exhausted`.
    pub reason: String,
    /// Owner-facing message: `agent parked, quota exhausted`.
    pub message: String,
    /// Agent key (hex).
    pub agent: String,
    /// World the session is in.
    pub world: String,
    /// Host time of the park, unix seconds.
    pub at: u64,
    /// Quota spent when the park happened.
    pub quota_spent: Quota,
    /// The session budget that was exhausted.
    pub quota_budget: Quota,
}

/// The owner-readable periodic summary (§7): the raw log is for
/// machines; this is the review surface an owner actually reads —
/// actions, spend, and refusals for one session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerSummary {
    /// Agent key (hex).
    pub agent: String,
    /// Owner key (hex).
    pub owner: String,
    /// World the session is in.
    pub world: String,
    /// Actions executed.
    pub executed: u64,
    /// Actions refused.
    pub refused: u64,
    /// Refusals by stable refusal code.
    pub refusals: BTreeMap<String, u64>,
    /// Total quota charged across the session.
    pub quota_spent: Quota,
    /// The session budget.
    pub quota_budget: Quota,
    /// Whether the session is parked (§6.2).
    pub parked: bool,
}

impl OwnerSummary {
    /// Render the summary as one owner-facing sentence set, in the
    /// §7 shape ("your archivist agent edited 14 objects, spent X,
    /// was refused twice").
    #[must_use]
    pub fn render(&self) -> String {
        let refusal_detail = if self.refusals.is_empty() {
            "no refusals".to_owned()
        } else {
            let parts: Vec<String> = self
                .refusals
                .iter()
                .map(|(code, count)| format!("{code} x{count}"))
                .collect();
            format!("refusals: {}", parts.join(", "))
        };
        let parked = if self.parked {
            " The agent is parked: quota exhausted."
        } else {
            ""
        };
        format!(
            "your agent {} in world {}: {} actions executed, {} refused ({}); spent {} frames, {} tool calls, {} program invocations of a budget of {} / {} / {}.{}",
            self.agent,
            self.world,
            self.executed,
            self.refused,
            refusal_detail,
            self.quota_spent.frames,
            self.quota_spent.tool_calls,
            self.quota_spent.program_invocations,
            self.quota_budget.frames,
            self.quota_budget.tool_calls,
            self.quota_budget.program_invocations,
            parked,
        )
    }
}

/// The result of presenting one action to the policy point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Admission {
    /// `Ok(charge)` when the action is admitted — the charge has
    /// already been applied to the ledger, before execution — or the
    /// refusal that stopped it. Execution happens only on `Ok`.
    pub decision: Result<Quota, PolicyRefusal>,
    /// The audit event recorded for this action, either way (§7).
    pub event: ChainedAuditEvent,
    /// The park world event, present exactly when this action's
    /// refusal parked the session (§6.2).
    pub park_event: Option<ParkEvent>,
}

/// The policy point for one agent session (§3, §6, §7).
///
/// Owns the ceiling, the quota ledger, the rate-limit windows, and
/// the audit log for a session opened by a verified join. Every
/// action goes through [`PolicyPoint::admit`], in the design's
/// order — ceiling, quota, audit — and no other path executes an
/// agent action.
#[derive(Debug)]
pub struct PolicyPoint {
    agent: String,
    owner: String,
    world: String,
    proof_ref: String,
    expires_at: u64,
    ceiling: Ceiling,
    budget: Quota,
    spent: Quota,
    rate_limits: Vec<RateLimit>,
    admitted_at: BTreeMap<String, VecDeque<u64>>,
    parked: bool,
    log: AuditLog,
}

impl PolicyPoint {
    /// Open the policy point for a session whose join was verified
    /// (spike step 3, [`crate::agent_delegation::verify_join`]). The
    /// join itself is audit event #1 of the session (§4.1), recorded
    /// here under the host key.
    ///
    /// # Errors
    ///
    /// Returns [`PolicyRefusal::CeilingWorld`] if the ceiling does
    /// not cover the grant's world: a session never opens under a
    /// ceiling that does not name its world.
    pub fn open(
        grant: &JoinGrant,
        ceiling: Ceiling,
        budget: Quota,
        rate_limits: Vec<RateLimit>,
        host_key: &SigningKey,
        now: u64,
    ) -> Result<Self, PolicyRefusal> {
        if !ceiling.worlds().contains(&grant.world) {
            return Err(PolicyRefusal::CeilingWorld);
        }
        let mut point = Self {
            agent: grant.agent.to_hex(),
            owner: AgentId(grant.owner).to_hex(),
            world: grant.world.clone(),
            proof_ref: grant.proof_ref.clone(),
            expires_at: grant.expires_at,
            ceiling,
            budget,
            spent: Quota::ZERO,
            rate_limits,
            admitted_at: BTreeMap::new(),
            parked: false,
            log: AuditLog::new(),
        };
        point.log.append(
            host_key,
            PolicyAuditEvent {
                agent: point.agent.clone(),
                owner: point.owner.clone(),
                world: point.world.clone(),
                tool: "world.join".to_owned(),
                object: None,
                proof_ref: Some(point.proof_ref.clone()),
                quota_charged: Quota::ZERO,
                outcome: "granted".to_owned(),
                at: now,
            },
        );
        Ok(point)
    }

    /// Present one agent action, in the §3 order: parked and expiry
    /// checks, the ceiling (§6.1), the stale-decision rule (§4.3),
    /// per-tool rate limits (§6.3), then the quota charge (§6.2) —
    /// applied before execution, refused atomically when it would
    /// exceed the budget, parking the session on exhaustion. Exactly
    /// one audit event is appended per action, whatever the outcome
    /// (§7).
    pub fn admit(
        &mut self,
        host_key: &SigningKey,
        action: &AgentAction,
        current_revision: u64,
        now: u64,
    ) -> Admission {
        let mut park_event = None;
        let refusal = if self.parked {
            Some(PolicyRefusal::SessionParked)
        } else if now >= self.expires_at {
            Some(PolicyRefusal::DelegationExpired)
        } else if let Err(refusal) = self.ceiling.check(action) {
            Some(refusal)
        } else if action
            .decided_revision
            .is_some_and(|decided| decided != current_revision)
        {
            Some(PolicyRefusal::StaleInput)
        } else if self.rate_limited(action, now) {
            Some(PolicyRefusal::RateLimited)
        } else {
            None
        };
        let (decision, charged) = if let Some(refusal) = refusal {
            (Err(refusal), Quota::ZERO)
        } else {
            let charge = Quota::charge_for(action);
            let prospective = self.spent.plus(charge);
            if prospective.within(self.budget) {
                self.spent = prospective;
                self.admitted_at
                    .entry(action.tool.clone())
                    .or_default()
                    .push_back(now);
                (Ok(charge), charge)
            } else {
                self.parked = true;
                park_event = Some(ParkEvent {
                    kind: "agent_parked".to_owned(),
                    reason: PolicyRefusal::QuotaExhausted.code().to_owned(),
                    message: "agent parked, quota exhausted".to_owned(),
                    agent: self.agent.clone(),
                    world: self.world.clone(),
                    at: now,
                    quota_spent: self.spent,
                    quota_budget: self.budget,
                });
                (Err(PolicyRefusal::QuotaExhausted), Quota::ZERO)
            }
        };
        let outcome = match &decision {
            Ok(_) => "executed".to_owned(),
            Err(refusal) => {
                let code = refusal.code();
                format!("refused:{code}")
            }
        };
        let event = self.log.append(
            host_key,
            PolicyAuditEvent {
                agent: self.agent.clone(),
                owner: self.owner.clone(),
                world: self.world.clone(),
                tool: action.tool.clone(),
                object: action.object.clone(),
                proof_ref: Some(self.proof_ref.clone()),
                quota_charged: charged,
                outcome,
                at: now,
            },
        );
        Admission {
            decision,
            event,
            park_event,
        }
    }

    /// Whether this action's tool is at its §6.3 rate limit at
    /// `now`, counting admitted calls inside the trailing window.
    fn rate_limited(&mut self, action: &AgentAction, now: u64) -> bool {
        let Some(limit) = self
            .rate_limits
            .iter()
            .find(|limit| limit.tool == action.tool)
            .cloned()
        else {
            return false;
        };
        let window = self.admitted_at.entry(action.tool.clone()).or_default();
        while window
            .front()
            .is_some_and(|at| now.saturating_sub(*at) >= limit.window_secs)
        {
            window.pop_front();
        }
        window.len() as u64 >= limit.max_calls
    }

    /// Whether quota exhaustion has parked this session (§6.2).
    #[must_use]
    pub const fn is_parked(&self) -> bool {
        self.parked
    }

    /// Quota charged so far this session.
    #[must_use]
    pub const fn spent(&self) -> Quota {
        self.spent
    }

    /// The session budget.
    #[must_use]
    pub const fn budget(&self) -> Quota {
        self.budget
    }

    /// The ceiling in force.
    #[must_use]
    pub const fn ceiling(&self) -> &Ceiling {
        &self.ceiling
    }

    /// The session audit log (§7): read and verify, never edit.
    #[must_use]
    pub const fn log(&self) -> &AuditLog {
        &self.log
    }

    /// The owner summary for this session (§7), derived from the
    /// audit log plus the live ledger.
    #[must_use]
    pub fn summary(&self) -> OwnerSummary {
        let mut executed = 0u64;
        let mut refused = 0u64;
        let mut refusals: BTreeMap<String, u64> = BTreeMap::new();
        for chained in self.log.events() {
            if chained.event.outcome == "executed" {
                executed += 1;
            } else if let Some(code) = chained.event.outcome.strip_prefix("refused:") {
                refused += 1;
                *refusals.entry(code.to_owned()).or_default() += 1;
            }
        }
        OwnerSummary {
            agent: self.agent.clone(),
            owner: self.owner.clone(),
            world: self.world.clone(),
            executed,
            refused,
            refusals,
            quota_spent: self.spent,
            quota_budget: self.budget,
            parked: self.parked,
        }
    }
}

/// The chain hash of one event: `Sha256(prev_hash || seq ||
/// canonical event bytes)`.
fn chain_hash(prev_hash: &[u8; 32], seq: u64, event: &PolicyAuditEvent) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(prev_hash);
    hasher.update(seq.to_be_bytes());
    hasher.update(event.canonical_bytes());
    hasher.finalize().into()
}

/// Length-prefixed field encoding for canonical bytes (the same
/// shape `agent_delegation` signs).
fn push_field(out: &mut Vec<u8>, field: &[u8]) {
    out.extend_from_slice(&(field.len() as u64).to_be_bytes());
    out.extend_from_slice(field);
}

/// An optional string as canonical field bytes: a presence byte
/// followed by the value, so `None` and `Some("")` never collide.
fn option_field(value: Option<&str>) -> Vec<u8> {
    match value {
        None => vec![0],
        Some(text) => {
            let mut out = vec![1];
            out.extend_from_slice(text.as_bytes());
            out
        }
    }
}
