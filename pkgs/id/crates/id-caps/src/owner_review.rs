//! Owner review tooling — v2 agents-as-citizens step 4 (#8302,
//! parent #6301).
//!
//! Oracle: `doc/2026-11-10_design_id6301_agents_as_citizens_v2_outline.md`
//! §2.5 (owner review tooling) and §4 decision 4 (review-queue
//! mechanics). V1 gave owners a signed audit log and a periodic
//! summary ([`crate::mcp_policy::OwnerSummary`]); v2 makes review a
//! working surface. Three pieces, all read from the audit log, which
//! stays the record of truth:
//!
//! - **Per-agent review view** ([`WorldReview`]): one place per world
//!   listing each agent, its role, its quota state, and its recent
//!   actions and refusals — rendered from audit events alone, never
//!   from live session state.
//! - **One-shot approval queue** ([`ReviewQueue`]): when the role's
//!   [`ReviewPolicy::queue_above_ceiling`] is on, an action the
//!   ceiling refused can be held as a single-action request. The owner
//!   approves or denies it once; a held request expires on its own.
//!   Approval executes exactly the refused action under the owner's
//!   authority — the audit event carries
//!   [`crate::mcp_policy::AuditEvent::approved_by`] — and never edits
//!   the role: the next identical call is refused again. It is not a
//!   delegation, and standing approvals for an action class are out
//!   of scope (open question 3 on #6301).
//! - **Refusal digests** ([`refusal_digests`]): repeated refusals of
//!   the same shape surface as one line naming the [`RoleEdit`] that
//!   would fix them, so a misconfigured role is fixable in one edit,
//!   never through an in-band exception. Refusals no role edit can fix
//!   (a stale decision, a bridge refusal) say so instead.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::mcp_policy::{
    AuditEvent, AuditOutcome, CeilingLeg, OwnerApproval, PolicyOutcome, PolicySession, Quota,
    audit_object,
};
use crate::role_config::ReviewPolicy;

/// How long a held request waits for the owner before it expires, by
/// default: one day.
pub const DEFAULT_REVIEW_TTL_SECS: u64 = 24 * 60 * 60;

/// The four-byte hex prefix of an agent key, for owner-facing text.
#[must_use]
pub fn short_key(agent_key: &[u8; 32]) -> String {
    agent_key[..4].iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

// ---------------------------------------------------------------------------
// Review view
// ---------------------------------------------------------------------------

/// What happened to one action, in the review view's terms.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum ReviewVerdict {
    /// Executed under the agent's own authority.
    Executed,
    /// Executed as the one action of a one-shot owner approval.
    OwnerApproved {
        /// The review request it answered.
        request_id: u64,
        /// The owner who approved it.
        approver: String,
    },
    /// Refused, with its stable refusal code
    /// ([`AuditOutcome::refusal_code`]) and detail.
    Refused {
        /// The refusal code.
        code: String,
        /// Human-readable detail.
        detail: String,
    },
}

/// One recent action in an agent's review.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewLine {
    /// The event's position in its session's audit chain.
    pub seq: u64,
    /// The tool the agent called.
    pub tool: String,
    /// The object the action named, if any.
    pub object: Option<String>,
    /// What happened.
    pub verdict: ReviewVerdict,
}

/// One agent's review, rendered from its audit events.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentReview {
    /// The agent's public key.
    pub agent_key: [u8; 32],
    /// The agent's owner.
    pub owner: String,
    /// The role named on the agent's latest event.
    pub role: String,
    /// The capability proof reference on the agent's latest event.
    pub capability_ref: String,
    /// Every action, executed or refused.
    pub actions: u64,
    /// Actions executed, owner-approved ones included.
    pub executed: u64,
    /// Actions executed under a one-shot owner approval.
    pub owner_approved: u64,
    /// Actions refused, for any reason.
    pub refusals: u64,
    /// Refusals by stable refusal code.
    pub refusals_by_code: BTreeMap<String, u64>,
    /// Quota charged across every event the log holds for the agent
    /// (before any replenishment: the ledger, not the log, is the
    /// budget's authority).
    pub charged: Quota,
    /// Whether the log's latest word on the agent is a park: a quota
    /// refusal, or a parked refusal, with no executed action since.
    pub parked: bool,
    /// The budget axis whose exhaustion parked the agent, when parked.
    pub parked_axis: Option<String>,
    /// The most recent actions, oldest first.
    pub recent: Vec<ReviewLine>,
}

impl AgentReview {
    fn new(event: &AuditEvent) -> Self {
        Self {
            agent_key: event.agent_key,
            owner: event.owner.clone(),
            role: event.role.clone(),
            capability_ref: event.capability_ref.clone(),
            actions: 0,
            executed: 0,
            owner_approved: 0,
            refusals: 0,
            refusals_by_code: BTreeMap::new(),
            charged: Quota::zero(),
            parked: false,
            parked_axis: None,
            recent: Vec::new(),
        }
    }

    fn record(&mut self, event: &AuditEvent, recent: usize) {
        if !event.role.is_empty() {
            self.role.clone_from(&event.role);
        }
        self.capability_ref.clone_from(&event.capability_ref);
        self.actions += 1;
        self.charged.add(event.quota_charged);
        let verdict = match (&event.outcome, &event.approved_by) {
            (AuditOutcome::Executed, Some(approval)) => {
                self.executed += 1;
                self.owner_approved += 1;
                ReviewVerdict::OwnerApproved {
                    request_id: approval.request_id,
                    approver: approval.approver.clone(),
                }
            }
            (AuditOutcome::Executed, None) => {
                self.executed += 1;
                ReviewVerdict::Executed
            }
            (outcome, _) => {
                self.refusals += 1;
                let code = outcome.refusal_code().unwrap_or("refused").to_owned();
                *self.refusals_by_code.entry(code.clone()).or_default() += 1;
                ReviewVerdict::Refused {
                    code,
                    detail: refusal_detail(outcome),
                }
            }
        };
        match &event.outcome {
            AuditOutcome::Executed => {
                self.parked = false;
                self.parked_axis = None;
            }
            AuditOutcome::RefusedQuota { axis } => {
                self.parked = true;
                self.parked_axis = Some(axis.clone());
            }
            _ => {}
        }
        if recent > 0 {
            if self.recent.len() == recent {
                self.recent.remove(0);
            }
            self.recent.push(ReviewLine {
                seq: event.seq,
                tool: event.tool.clone(),
                object: event.object.clone(),
                verdict,
            });
        }
    }

    /// Render this agent's review as owner-facing text: a headline
    /// with role, quota state and counts, then one line per recent
    /// action.
    #[must_use]
    pub fn render(&self) -> String {
        let mut text = format!(
            "agent {} (role {}): {} actions, {} executed ({} owner-approved), {} refused; charged {} frames / {} tool calls / {} program invocations",
            short_key(&self.agent_key),
            if self.role.is_empty() {
                "unknown"
            } else {
                &self.role
            },
            self.actions,
            self.executed,
            self.owner_approved,
            self.refusals,
            self.charged.frames,
            self.charged.tool_calls,
            self.charged.program_invocations,
        );
        if self.parked {
            let _ = write!(
                text,
                "; parked, quota exhausted on the {} axis",
                self.parked_axis.as_deref().unwrap_or("unknown")
            );
        }
        if !self.refusals_by_code.is_empty() {
            let codes: Vec<String> = self
                .refusals_by_code
                .iter()
                .map(|(code, count)| format!("{code} {count}"))
                .collect();
            let _ = write!(text, " (refusals: {})", codes.join(", "));
        }
        for line in &self.recent {
            let object = line
                .object
                .as_deref()
                .map(|object| format!(" {object}"))
                .unwrap_or_default();
            let verdict = match &line.verdict {
                ReviewVerdict::Executed => "executed".to_owned(),
                ReviewVerdict::OwnerApproved {
                    request_id,
                    approver,
                } => format!("executed, approved once by {approver} (request {request_id})"),
                ReviewVerdict::Refused { code, detail } => format!("refused ({code}): {detail}"),
            };
            let _ = write!(
                text,
                "\n  #{} {}{}: {}",
                line.seq, line.tool, object, verdict
            );
        }
        text
    }
}

/// The owner's per-world review view (§2.5): every agent the log
/// names in the world, with role, quota state, and recent actions and
/// refusals — built from audit events alone.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorldReview {
    /// The world reviewed.
    pub world: String,
    /// One review per agent, in agent-key order.
    pub agents: Vec<AgentReview>,
}

impl WorldReview {
    /// Build the review from audit logs — one slice per session, in
    /// chain order; an agent that left and rejoined contributes one
    /// slice per session, oldest first. Events from other worlds are
    /// ignored. `recent` caps the recent actions listed per agent.
    #[must_use]
    pub fn from_audit<'a>(
        world: &str,
        logs: impl IntoIterator<Item = &'a [AuditEvent]>,
        recent: usize,
    ) -> Self {
        let mut agents: BTreeMap<[u8; 32], AgentReview> = BTreeMap::new();
        for log in logs {
            for event in log.iter().filter(|event| event.world == world) {
                agents
                    .entry(event.agent_key)
                    .or_insert_with(|| AgentReview::new(event))
                    .record(event, recent);
            }
        }
        Self {
            world: world.to_owned(),
            agents: agents.into_values().collect(),
        }
    }

    /// The review of one agent, if the log names it.
    #[must_use]
    pub fn agent(&self, agent_key: &[u8; 32]) -> Option<&AgentReview> {
        self.agents
            .iter()
            .find(|review| &review.agent_key == agent_key)
    }

    /// Render the whole view: a header, then each agent's review.
    #[must_use]
    pub fn render(&self) -> String {
        let mut text = format!("world {}: {} agents", self.world, self.agents.len());
        for agent in &self.agents {
            text.push('\n');
            text.push_str(&agent.render());
        }
        text
    }
}

/// The human-readable detail of a refusal outcome.
fn refusal_detail(outcome: &AuditOutcome) -> String {
    match outcome {
        AuditOutcome::Executed => String::new(),
        AuditOutcome::RefusedCeiling { detail } | AuditOutcome::RefusedBridge { detail } => {
            detail.clone()
        }
        AuditOutcome::RefusedQuota { axis } => format!("quota exhausted on the {axis} axis"),
        AuditOutcome::RefusedParked => "the agent is parked".to_owned(),
        AuditOutcome::RefusedStaleInput { token, current } => {
            format!("acted on revision {token}, current revision is {current}")
        }
        AuditOutcome::RefusedRateLimit {
            tool,
            limit,
            window_secs,
        } => format!("{tool} is limited to {limit} calls per {window_secs}s"),
    }
}

// ---------------------------------------------------------------------------
// One-shot approval queue
// ---------------------------------------------------------------------------

/// One above-ceiling action held for the owner's one-shot decision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewRequest {
    /// The request's id in its queue.
    pub id: u64,
    /// The agent whose action the ceiling refused.
    pub agent_key: [u8; 32],
    /// The agent's owner: the only one who may decide.
    pub owner: String,
    /// The world the action was in.
    pub world: String,
    /// The role the agent held.
    pub role: String,
    /// The capability proof reference of the refusing session.
    pub capability_ref: String,
    /// The refused tool.
    pub tool: String,
    /// The refused call's arguments, exactly as the agent sent them:
    /// approval executes these and nothing else.
    pub arguments: serde_json::Value,
    /// The object the action named, if any.
    pub object: Option<String>,
    /// The ceiling refusal's detail.
    pub detail: String,
    /// The refusal's audit event: its position and hash.
    pub refused_seq: u64,
    /// The refusal event's hash, so one refusal is held at most once.
    pub refused_hash: [u8; 32],
    /// When the request was held, unix seconds.
    pub held_at: u64,
    /// When it expires unanswered, unix seconds.
    pub expires_at: u64,
}

/// How a held request ended.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum ReviewDecision {
    /// The owner approved it; the action ran once.
    Approved {
        /// Whether the action executed (the quota, rate limit, parking
        /// and stale-decision checks still apply to it).
        executed: bool,
    },
    /// The owner denied it.
    Denied,
    /// Nobody decided before it expired.
    Expired,
}

/// The record of one request's end: the queue's own audit of
/// decisions, alongside the signed chain the approved action lands in.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewResolution {
    /// The request.
    pub request_id: u64,
    /// The agent it was for.
    pub agent_key: [u8; 32],
    /// The tool it named.
    pub tool: String,
    /// How it ended.
    pub decision: ReviewDecision,
    /// Who decided; `None` for an expiry.
    pub by: Option<String>,
    /// When it ended, unix seconds.
    pub at: u64,
}

/// Why an action could not be held for review.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OfferRefusal {
    /// The role does not queue above-ceiling requests.
    QueueingOff,
    /// The session's latest audit event is not a ceiling refusal of
    /// this call: only an action the ceiling actually refused can be
    /// held.
    NotACeilingRefusal,
    /// This refusal is already held, or was already decided.
    AlreadyHeld {
        /// The request that holds it.
        request_id: u64,
    },
}

/// Why a decision on a request was refused.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReviewError {
    /// No such request is pending: unknown, or already decided.
    NotPending {
        /// The request named.
        request_id: u64,
    },
    /// The request expired before the decision; it is now resolved as
    /// expired.
    Expired {
        /// The request named.
        request_id: u64,
    },
    /// Only the agent's owner decides its requests.
    NotOwner {
        /// The request named.
        request_id: u64,
        /// The owner who may decide.
        owner: String,
    },
    /// The session offered is not the one the action was refused in.
    WrongSession {
        /// The request named.
        request_id: u64,
    },
    /// The agent holds no session in the world.
    AgentNotPresent {
        /// The request named.
        request_id: u64,
    },
}

impl std::fmt::Display for ReviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotPending { request_id } => write!(f, "request {request_id} is not pending"),
            Self::Expired { request_id } => write!(f, "request {request_id} expired"),
            Self::NotOwner { request_id, owner } => {
                write!(f, "only {owner} may decide request {request_id}")
            }
            Self::WrongSession { request_id } => write!(
                f,
                "request {request_id} was refused in another session of the agent"
            ),
            Self::AgentNotPresent { request_id } => write!(
                f,
                "the agent of request {request_id} holds no session in the world"
            ),
        }
    }
}

impl std::error::Error for ReviewError {}

/// The owner's one-shot approval queue for one world (§2.5, §4
/// decision 4). A request is held from a ceiling refusal, decided at
/// most once, and expires unanswered after the queue's TTL.
#[derive(Clone, Debug)]
pub struct ReviewQueue {
    ttl_secs: u64,
    next_id: u64,
    pending: BTreeMap<u64, ReviewRequest>,
    held: BTreeMap<[u8; 32], u64>,
    resolutions: Vec<ReviewResolution>,
}

impl Default for ReviewQueue {
    fn default() -> Self {
        Self::new(DEFAULT_REVIEW_TTL_SECS)
    }
}

impl ReviewQueue {
    /// An empty queue whose requests expire `ttl_secs` after they are
    /// held.
    #[must_use]
    pub const fn new(ttl_secs: u64) -> Self {
        Self {
            ttl_secs,
            next_id: 1,
            pending: BTreeMap::new(),
            held: BTreeMap::new(),
            resolutions: Vec::new(),
        }
    }

    /// Hold the call the session's ceiling just refused for the
    /// owner's one-shot decision. The host calls this right after
    /// [`PolicySession::act`] returned a ceiling refusal, passing the
    /// same call; the queue checks the session's latest audit event is
    /// that refusal, so nothing the ceiling did not refuse can be held.
    ///
    /// # Errors
    ///
    /// [`OfferRefusal`] when the role does not queue above-ceiling
    /// requests, the latest event is not this call's ceiling refusal,
    /// or the refusal is already held.
    pub fn offer(
        &mut self,
        session: &PolicySession,
        policy: &ReviewPolicy,
        name: &str,
        arguments: &serde_json::Value,
        now: u64,
    ) -> Result<u64, OfferRefusal> {
        if !policy.queue_above_ceiling {
            return Err(OfferRefusal::QueueingOff);
        }
        let event = session
            .audit_events()
            .last()
            .ok_or(OfferRefusal::NotACeilingRefusal)?;
        let AuditOutcome::RefusedCeiling { detail } = &event.outcome else {
            return Err(OfferRefusal::NotACeilingRefusal);
        };
        if event.tool != name || event.object != audit_object(name, arguments) {
            return Err(OfferRefusal::NotACeilingRefusal);
        }
        if let Some(request_id) = self.held.get(&event.hash) {
            return Err(OfferRefusal::AlreadyHeld {
                request_id: *request_id,
            });
        }
        let id = self.next_id;
        self.next_id += 1;
        self.held.insert(event.hash, id);
        self.pending.insert(
            id,
            ReviewRequest {
                id,
                agent_key: event.agent_key,
                owner: event.owner.clone(),
                world: event.world.clone(),
                role: event.role.clone(),
                capability_ref: event.capability_ref.clone(),
                tool: event.tool.clone(),
                arguments: arguments.clone(),
                object: event.object.clone(),
                detail: detail.clone(),
                refused_seq: event.seq,
                refused_hash: event.hash,
                held_at: now,
                expires_at: now.saturating_add(self.ttl_secs),
            },
        );
        Ok(id)
    }

    /// The requests waiting for a decision, in id order.
    #[must_use]
    pub fn pending(&self) -> Vec<&ReviewRequest> {
        self.pending.values().collect()
    }

    /// One pending request.
    #[must_use]
    pub fn request(&self, request_id: u64) -> Option<&ReviewRequest> {
        self.pending.get(&request_id)
    }

    /// Every decided or expired request, in the order it ended.
    #[must_use]
    pub fn resolutions(&self) -> &[ReviewResolution] {
        &self.resolutions
    }

    /// Resolve every pending request whose time is up as expired, and
    /// return their ids.
    pub fn expire(&mut self, now: u64) -> Vec<u64> {
        let due: Vec<u64> = self
            .pending
            .values()
            .filter(|request| now >= request.expires_at)
            .map(|request| request.id)
            .collect();
        for id in &due {
            self.resolve(*id, ReviewDecision::Expired, None, now);
        }
        due
    }

    /// Approve request `request_id` once: execute exactly the refused
    /// call in `session` under `approver`'s authority. The ceiling is
    /// lifted for that one call only; every other check still runs,
    /// the audit event carries the approval, and the role is never
    /// edited.
    ///
    /// # Errors
    ///
    /// [`ReviewError`] when the request is not pending, has expired
    /// (it is resolved as expired), `approver` is not the agent's
    /// owner, or `session` is not the session the call was refused in.
    pub fn approve(
        &mut self,
        request_id: u64,
        approver: &str,
        session: &mut PolicySession,
        now: u64,
    ) -> Result<PolicyOutcome, ReviewError> {
        let request = self.decidable(request_id, approver, now)?;
        let grant = session.grant();
        let same_session = grant.agent_key == request.agent_key
            && grant.world == request.world
            && grant.capability_ref == request.capability_ref
            && grant.owner == request.owner
            && session
                .audit_events()
                .get(usize::try_from(request.refused_seq).unwrap_or(usize::MAX))
                .is_some_and(|event| event.hash == request.refused_hash);
        if !same_session {
            return Err(ReviewError::WrongSession { request_id });
        }
        let tool = request.tool.clone();
        let arguments = request.arguments.clone();
        let outcome = session.act_owner_approved(
            OwnerApproval {
                request_id,
                approver: approver.to_owned(),
            },
            &tool,
            &arguments,
            now,
        );
        let executed = matches!(outcome, PolicyOutcome::Executed(_));
        self.resolve(
            request_id,
            ReviewDecision::Approved { executed },
            Some(approver.to_owned()),
            now,
        );
        Ok(outcome)
    }

    /// Deny request `request_id`: nothing runs.
    ///
    /// # Errors
    ///
    /// [`ReviewError`] when the request is not pending, has expired,
    /// or `approver` is not the agent's owner.
    pub fn deny(&mut self, request_id: u64, approver: &str, now: u64) -> Result<(), ReviewError> {
        self.decidable(request_id, approver, now)?;
        self.resolve(
            request_id,
            ReviewDecision::Denied,
            Some(approver.to_owned()),
            now,
        );
        Ok(())
    }

    /// The pending request `request_id`, if `approver` may decide it
    /// at `now`. An expired request is resolved as expired.
    fn decidable(
        &mut self,
        request_id: u64,
        approver: &str,
        now: u64,
    ) -> Result<&ReviewRequest, ReviewError> {
        let (expired, owner) = match self.pending.get(&request_id) {
            None => return Err(ReviewError::NotPending { request_id }),
            Some(request) => (now >= request.expires_at, request.owner.clone()),
        };
        if expired {
            self.resolve(request_id, ReviewDecision::Expired, None, now);
            return Err(ReviewError::Expired { request_id });
        }
        if owner != approver {
            return Err(ReviewError::NotOwner { request_id, owner });
        }
        self.pending
            .get(&request_id)
            .ok_or(ReviewError::NotPending { request_id })
    }

    fn resolve(&mut self, request_id: u64, decision: ReviewDecision, by: Option<String>, at: u64) {
        if let Some(request) = self.pending.remove(&request_id) {
            self.resolutions.push(ReviewResolution {
                request_id,
                agent_key: request.agent_key,
                tool: request.tool,
                decision,
                by,
                at,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Refusal digests
// ---------------------------------------------------------------------------

/// The role edit that would fix a pattern of refusals (§2.5: the fix
/// is a role edit, §2.1, never an in-band exception).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "edit", rename_all = "snake_case")]
pub enum RoleEdit {
    /// Add a tool to the role's capability ceiling.
    AddTool {
        /// The role.
        role: String,
        /// The tool.
        tool: String,
    },
    /// Add a world to the role's capability ceiling.
    AddWorld {
        /// The role.
        role: String,
        /// The world.
        world: String,
    },
    /// Add an object class to the role's capability ceiling.
    AddObjectClass {
        /// The role.
        role: String,
        /// The object class.
        class: String,
    },
    /// Add a program to the role's program-invoke allowlist.
    AddProgram {
        /// The role.
        role: String,
        /// The program.
        program: String,
    },
    /// Raise the role's quota-class budget on one axis.
    RaiseBudget {
        /// The role.
        role: String,
        /// The exhausted axis.
        axis: String,
    },
    /// Raise the role's rate limit for one tool.
    RaiseRateLimit {
        /// The role.
        role: String,
        /// The rate-limited tool.
        tool: String,
        /// The current limit, calls per window.
        limit: u64,
        /// The current window, seconds.
        window_secs: u64,
    },
}

impl RoleEdit {
    /// The edit, as an owner reads it.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::AddTool { role, tool } => {
                format!("add tool {tool} to role {role}'s capability ceiling")
            }
            Self::AddWorld { role, world } => {
                format!("add world {world} to role {role}'s capability ceiling")
            }
            Self::AddObjectClass { role, class } => {
                format!("add object class {class} to role {role}'s capability ceiling")
            }
            Self::AddProgram { role, program } => {
                format!("add program {program} to role {role}'s program-invoke allowlist")
            }
            Self::RaiseBudget { role, axis } => {
                format!("raise role {role}'s quota-class budget on the {axis} axis")
            }
            Self::RaiseRateLimit {
                role,
                tool,
                limit,
                window_secs,
            } => format!(
                "raise role {role}'s rate limit for {tool} above {limit} calls per {window_secs}s"
            ),
        }
    }
}

/// Repeated refusals of one shape, surfaced as one pattern.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RefusalDigest {
    /// The refused agent.
    pub agent_key: [u8; 32],
    /// The role it held.
    pub role: String,
    /// The refused tool.
    pub tool: String,
    /// The stable refusal code.
    pub code: String,
    /// What the refusals share, in words.
    pub shape: String,
    /// How many refusals had this shape.
    pub count: u64,
    /// The role edit that would fix them, when one would.
    pub role_edit: Option<RoleEdit>,
    /// Why no role edit fixes them, when none would.
    pub no_edit_reason: Option<String>,
}

impl RefusalDigest {
    /// The digest line, e.g. "role archivist (agent 03030303) was
    /// refused object.export 12 times (ceiling: tool object.export is
    /// outside the capability ceiling); fix: add tool object.export to
    /// role archivist's capability ceiling".
    #[must_use]
    pub fn render(&self) -> String {
        let fix = match (&self.role_edit, &self.no_edit_reason) {
            (Some(edit), _) => format!("fix: {}", edit.render()),
            (None, Some(reason)) => reason.clone(),
            (None, None) => "no role edit fixes this".to_owned(),
        };
        format!(
            "role {} (agent {}) was refused {} {} times ({}: {}); {}",
            self.role,
            short_key(&self.agent_key),
            self.tool,
            self.count,
            self.code,
            self.shape,
            fix,
        )
    }
}

type DigestKey = ([u8; 32], String, String, String, String);

/// Digest repeated same-shape refusals in the audit logs (§2.5).
///
/// Refusals are grouped by agent, role, tool, refusal code and shape;
/// a group of at least `digest_every(role)` refusals becomes one
/// digest — the role's [`ReviewPolicy::digest_every_refusals`], where
/// `0` never digests. Digests come most-refused first.
#[must_use]
pub fn refusal_digests<'a>(
    logs: impl IntoIterator<Item = &'a [AuditEvent]>,
    digest_every: impl Fn(&str) -> u64,
) -> Vec<RefusalDigest> {
    let mut groups: BTreeMap<DigestKey, RefusalDigest> = BTreeMap::new();
    let mut last_quota_axis: BTreeMap<[u8; 32], String> = BTreeMap::new();
    for log in logs {
        for event in log {
            let Some(code) = event.outcome.refusal_code() else {
                continue;
            };
            let (shape, role_edit, no_edit_reason) =
                digest_shape(event, last_quota_axis.get(&event.agent_key));
            if let AuditOutcome::RefusedQuota { axis } = &event.outcome {
                last_quota_axis.insert(event.agent_key, axis.clone());
            }
            let key = (
                event.agent_key,
                event.role.clone(),
                event.tool.clone(),
                code.to_owned(),
                shape.clone(),
            );
            groups
                .entry(key)
                .or_insert_with(|| RefusalDigest {
                    agent_key: event.agent_key,
                    role: event.role.clone(),
                    tool: event.tool.clone(),
                    code: code.to_owned(),
                    shape,
                    count: 0,
                    role_edit,
                    no_edit_reason,
                })
                .count += 1;
        }
    }
    let mut digests: Vec<RefusalDigest> = groups
        .into_values()
        .filter(|digest| {
            let every = digest_every(&digest.role);
            every > 0 && digest.count >= every
        })
        .collect();
    digests.sort_by_key(|digest| std::cmp::Reverse(digest.count));
    digests
}

/// The shape of one refusal and the role edit that fixes it, or why
/// none does. `parked_axis` is the axis of the agent's latest quota
/// refusal, which a parked refusal follows from.
fn digest_shape(
    event: &AuditEvent,
    parked_axis: Option<&String>,
) -> (String, Option<RoleEdit>, Option<String>) {
    let role = event.role.clone();
    match &event.outcome {
        AuditOutcome::RefusedCeiling { detail } => {
            let edit = match CeilingLeg::from_detail(detail) {
                Some(CeilingLeg::Tool(tool)) => Some(RoleEdit::AddTool { role, tool }),
                Some(CeilingLeg::World(world)) => Some(RoleEdit::AddWorld { role, world }),
                Some(CeilingLeg::ObjectClass(class)) => {
                    Some(RoleEdit::AddObjectClass { role, class })
                }
                Some(CeilingLeg::Program(program)) => Some(RoleEdit::AddProgram { role, program }),
                Some(CeilingLeg::ClassNotEstablished | CeilingLeg::NoProgram) | None => None,
            };
            let reason = edit.is_none().then(|| {
                "the call did not name what it acts on; no role edit fixes this".to_owned()
            });
            (detail.clone(), edit, reason)
        }
        AuditOutcome::RefusedQuota { axis } => (
            format!("quota exhausted on the {axis} axis"),
            Some(RoleEdit::RaiseBudget {
                role,
                axis: axis.clone(),
            }),
            None,
        ),
        AuditOutcome::RefusedParked => match parked_axis {
            Some(axis) => (
                format!("parked after the {axis} axis ran out"),
                Some(RoleEdit::RaiseBudget {
                    role,
                    axis: axis.clone(),
                }),
                None,
            ),
            None => (
                "parked".to_owned(),
                None,
                Some(
                    "the park's exhausted axis is not in the log; no role edit is named".to_owned(),
                ),
            ),
        },
        AuditOutcome::RefusedRateLimit {
            tool,
            limit,
            window_secs,
        } => (
            format!("limited to {limit} calls per {window_secs}s"),
            Some(RoleEdit::RaiseRateLimit {
                role,
                tool: tool.clone(),
                limit: *limit,
                window_secs: *window_secs,
            }),
            None,
        ),
        AuditOutcome::RefusedStaleInput { .. } => (
            "acted on an outdated frame".to_owned(),
            None,
            Some("the agent acted on an outdated frame; no role edit fixes this".to_owned()),
        ),
        AuditOutcome::RefusedBridge { detail } => (
            detail.clone(),
            None,
            Some("the bridge refused the call; no role edit fixes this".to_owned()),
        ),
        AuditOutcome::Executed => (String::new(), None, None),
    }
}

/// The set of roles a review view names, for looking up their review
/// policies.
#[must_use]
pub fn reviewed_roles(review: &WorldReview) -> BTreeSet<String> {
    review
        .agents
        .iter()
        .map(|agent| agent.role.clone())
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::collections::BTreeSet;

    use ed25519_dalek::SigningKey;
    use serde_json::json;

    use super::*;
    use crate::mcp_bridge::{BridgeSession, ToolAction, v1_tool_definitions};
    use crate::mcp_policy::{CapabilityCeiling, RateLimit, RefusalKind, RoleGrant};
    use crate::role_config::QuotaClass;

    const NOW: u64 = 1_700_000_000;

    fn grant(agent: u8, role: &str) -> RoleGrant {
        let tools: BTreeSet<String> = ["world.join", "chat.post", "world.leave"]
            .into_iter()
            .map(ToOwned::to_owned)
            .collect();
        RoleGrant {
            agent_key: [agent; 32],
            agent_label: role.to_owned(),
            owner: "drew".to_owned(),
            world: "atlas".to_owned(),
            capability_ref: format!("grant-{agent}"),
            ceiling: CapabilityCeiling {
                tools,
                worlds: std::iter::once("atlas".to_owned()).collect(),
                object_classes: BTreeSet::new(),
                program_allowlist: BTreeSet::new(),
            },
            role: role.to_owned(),
            quota_class: QuotaClass {
                budget: Quota {
                    frames: 100,
                    tool_calls: 100,
                    program_invocations: 10,
                },
                replenishment: None,
            },
            rate_limits: vec![RateLimit {
                tool: "chat.post".to_owned(),
                max_calls: 2,
                window_secs: 60,
            }],
        }
    }

    fn joined(grant: RoleGrant) -> PolicySession {
        let mut session = PolicySession::new(
            grant,
            BridgeSession::start(&v1_tool_definitions()).unwrap(),
            SigningKey::from_bytes(&[9; 32]),
            NOW,
        );
        assert!(matches!(
            session.act("world.join", &json!({ "capability": "cap-1" }), NOW),
            PolicyOutcome::Executed(_)
        ));
        session
    }

    const fn queueing() -> ReviewPolicy {
        ReviewPolicy {
            queue_above_ceiling: true,
            digest_every_refusals: 3,
        }
    }

    fn subscribe() -> serde_json::Value {
        json!({ "event_classes": ["chat_posted"] })
    }

    /// Hold a ceiling refusal of `screen.subscribe` (outside the test
    /// ceiling) and return its request id.
    fn refuse_and_hold(session: &mut PolicySession, queue: &mut ReviewQueue, now: u64) -> u64 {
        let refused = session.act("screen.subscribe", &subscribe(), now);
        assert!(
            matches!(&refused, PolicyOutcome::Refused(refusal) if refusal.kind == RefusalKind::Ceiling),
            "{refused:?}"
        );
        queue
            .offer(session, &queueing(), "screen.subscribe", &subscribe(), now)
            .unwrap()
    }

    /// Acceptance 2: approval executes the one refused action, is
    /// audited as owner authority, and leaves the role untouched.
    #[test]
    fn one_shot_approval_executes_once_under_owner_authority_and_never_edits_the_role() {
        let mut session = joined(grant(1, "archivist"));
        let role_before = session.grant().clone();
        let mut queue = ReviewQueue::default();
        let id = refuse_and_hold(&mut session, &mut queue, NOW + 1);
        assert_eq!(queue.pending().len(), 1);
        assert_eq!(queue.request(id).unwrap().tool, "screen.subscribe");

        let outcome = queue.approve(id, "drew", &mut session, NOW + 2).unwrap();
        assert!(
            matches!(outcome, PolicyOutcome::Executed(ToolAction::SendFrame(_))),
            "{outcome:?}"
        );

        // Audited as owner authority, on exactly that event.
        let events = session.audit_events();
        let approved = events.last().unwrap();
        assert_eq!(approved.tool, "screen.subscribe");
        assert_eq!(approved.outcome, AuditOutcome::Executed);
        assert_eq!(
            approved.approved_by,
            Some(OwnerApproval {
                request_id: id,
                approver: "drew".to_owned(),
            })
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event.approved_by.is_some())
                .count(),
            1
        );
        session.verify_audit().unwrap();

        // The role is untouched and the next identical call is refused.
        assert_eq!(session.grant(), &role_before);
        let again = session.act("screen.subscribe", &subscribe(), NOW + 3);
        assert!(
            matches!(&again, PolicyOutcome::Refused(refusal) if refusal.kind == RefusalKind::Ceiling),
            "{again:?}"
        );
        assert_eq!(session.audit_events().last().unwrap().approved_by, None);

        // One-shot: the request is gone and cannot be approved twice.
        assert_eq!(
            queue
                .approve(id, "drew", &mut session, NOW + 4)
                .unwrap_err(),
            ReviewError::NotPending { request_id: id }
        );
        assert_eq!(
            queue.resolutions(),
            &[ReviewResolution {
                request_id: id,
                agent_key: [1; 32],
                tool: "screen.subscribe".to_owned(),
                decision: ReviewDecision::Approved { executed: true },
                by: Some("drew".to_owned()),
                at: NOW + 2,
            }]
        );
    }

    #[test]
    fn only_ceiling_refusals_are_held_and_only_once() {
        let mut session = joined(grant(1, "archivist"));
        let mut queue = ReviewQueue::default();
        // Queueing off: refused.
        session.act("screen.subscribe", &subscribe(), NOW);
        assert_eq!(
            queue.offer(
                &session,
                &ReviewPolicy::default(),
                "screen.subscribe",
                &subscribe(),
                NOW
            ),
            Err(OfferRefusal::QueueingOff)
        );
        // A different call than the one refused: refused.
        assert_eq!(
            queue.offer(
                &session,
                &queueing(),
                "chat.post",
                &json!({"text": "hi"}),
                NOW
            ),
            Err(OfferRefusal::NotACeilingRefusal)
        );
        let id = queue
            .offer(&session, &queueing(), "screen.subscribe", &subscribe(), NOW)
            .unwrap();
        assert_eq!(
            queue.offer(&session, &queueing(), "screen.subscribe", &subscribe(), NOW),
            Err(OfferRefusal::AlreadyHeld { request_id: id })
        );
        // An executed action is not a refusal.
        session.act("chat.post", &json!({"text": "hi"}), NOW);
        assert_eq!(
            queue.offer(
                &session,
                &queueing(),
                "chat.post",
                &json!({"text": "hi"}),
                NOW
            ),
            Err(OfferRefusal::NotACeilingRefusal)
        );
    }

    #[test]
    fn deny_expiry_owner_and_session_checks() {
        let mut session = joined(grant(1, "archivist"));
        let mut other = joined(grant(2, "archivist"));
        let mut queue = ReviewQueue::new(100);

        let id = refuse_and_hold(&mut session, &mut queue, NOW);
        // Not the owner.
        assert_eq!(
            queue.approve(id, "mallory", &mut session, NOW + 1),
            Err(ReviewError::NotOwner {
                request_id: id,
                owner: "drew".to_owned(),
            })
        );
        // Another agent's session.
        assert_eq!(
            queue.approve(id, "drew", &mut other, NOW + 1),
            Err(ReviewError::WrongSession { request_id: id })
        );
        assert_eq!(other.audit_events().len(), 1);
        // Denied: nothing runs.
        let before = session.audit_events().len();
        queue.deny(id, "drew", NOW + 2).unwrap();
        assert_eq!(session.audit_events().len(), before);
        assert_eq!(
            queue.resolutions().last().unwrap().decision,
            ReviewDecision::Denied
        );

        // Expiry on approval, and by sweep.
        let late = refuse_and_hold(&mut session, &mut queue, NOW + 10);
        assert_eq!(
            queue.approve(late, "drew", &mut session, NOW + 110),
            Err(ReviewError::Expired { request_id: late })
        );
        let swept = refuse_and_hold(&mut session, &mut queue, NOW + 200);
        assert_eq!(queue.expire(NOW + 299), Vec::<u64>::new());
        assert_eq!(queue.expire(NOW + 300), vec![swept]);
        assert_eq!(queue.pending(), Vec::<&ReviewRequest>::new());
        let expired: Vec<_> = queue
            .resolutions()
            .iter()
            .filter(|resolution| resolution.decision == ReviewDecision::Expired)
            .map(|resolution| (resolution.request_id, resolution.by.clone()))
            .collect();
        assert_eq!(expired, vec![(late, None), (swept, None)]);
    }

    #[test]
    fn approval_still_charges_quota_and_respects_parking() {
        let mut tight = grant(1, "archivist");
        tight.quota_class.budget = Quota {
            frames: 1,
            tool_calls: 10,
            program_invocations: 0,
        };
        let mut session = joined(tight);
        let mut queue = ReviewQueue::default();
        let id = refuse_and_hold(&mut session, &mut queue, NOW + 1);
        // The join spent the only frame: approval runs the quota check
        // and parks rather than minting budget.
        let outcome = queue.approve(id, "drew", &mut session, NOW + 2).unwrap();
        assert!(
            matches!(&outcome, PolicyOutcome::Refused(refusal) if refusal.kind == RefusalKind::Quota),
            "{outcome:?}"
        );
        assert!(session.is_parked());
        let last = session.audit_events().last().unwrap();
        assert_eq!(
            last.outcome,
            AuditOutcome::RefusedQuota {
                axis: "frames".to_owned()
            }
        );
        assert_eq!(last.approved_by.as_ref().unwrap().request_id, id);
        assert_eq!(
            queue.resolutions()[0].decision,
            ReviewDecision::Approved { executed: false }
        );
    }

    /// Acceptance 1: the review view renders per-agent role, quota,
    /// actions and refusals from the audit log alone.
    #[test]
    fn review_view_renders_each_agent_from_the_audit_log_alone() {
        let mut archivist = joined(grant(1, "archivist"));
        let mut greeter = joined(grant(2, "greeter"));
        let mut queue = ReviewQueue::default();
        archivist.act("chat.post", &json!({"text": "one"}), NOW + 1);
        let id = refuse_and_hold(&mut archivist, &mut queue, NOW + 2);
        queue.approve(id, "drew", &mut archivist, NOW + 3).unwrap();
        for at in 0..3 {
            greeter.act("chat.post", &json!({"text": "spam"}), NOW + 10 + at);
        }

        // Serialize and reload the logs: the view needs nothing else.
        let archivist_log: Vec<AuditEvent> =
            serde_json::from_str(&serde_json::to_string(archivist.audit_events()).unwrap())
                .unwrap();
        let greeter_log: Vec<AuditEvent> =
            serde_json::from_str(&serde_json::to_string(greeter.audit_events()).unwrap()).unwrap();
        let review = WorldReview::from_audit(
            "atlas",
            [archivist_log.as_slice(), greeter_log.as_slice()],
            10,
        );
        assert_eq!(review.agents.len(), 2);

        let first = review.agent(&[1; 32]).unwrap();
        assert_eq!(first.role, "archivist");
        assert_eq!(first.actions, 4);
        assert_eq!(first.executed, 3);
        assert_eq!(first.owner_approved, 1);
        assert_eq!(first.refusals, 1);
        assert_eq!(first.refusals_by_code.get("ceiling"), Some(&1));
        assert_eq!(first.charged, archivist.spend(NOW + 3));
        assert!(!first.parked);
        assert!(matches!(
            first.recent.last().unwrap().verdict,
            ReviewVerdict::OwnerApproved { request_id, .. } if request_id == id
        ));

        let second = review.agent(&[2; 32]).unwrap();
        assert_eq!(second.role, "greeter");
        assert_eq!(second.executed, 3);
        assert_eq!(second.refusals_by_code.get("rate_limited"), Some(&1));

        let text = review.render();
        assert!(text.starts_with("world atlas: 2 agents"), "{text}");
        assert!(text.contains("agent 01010101 (role archivist)"), "{text}");
        assert!(text.contains("approved once by drew"), "{text}");
        assert!(text.contains("refused (rate_limited)"), "{text}");

        // The recent list is capped and keeps the newest.
        let capped = WorldReview::from_audit("atlas", [archivist_log.as_slice()], 2);
        let recent = &capped.agents[0].recent;
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[1].seq, archivist_log.last().unwrap().seq);
        // Other worlds are not in this world's view.
        assert_eq!(
            WorldReview::from_audit("elsewhere", [archivist_log.as_slice()], 2).agents,
            Vec::new()
        );
    }

    #[test]
    fn review_view_tracks_parking_from_the_log() {
        let mut tight = grant(1, "archivist");
        tight.quota_class.budget = Quota {
            frames: 1,
            tool_calls: 10,
            program_invocations: 0,
        };
        let mut session = joined(tight);
        session.act("chat.post", &json!({"text": "x"}), NOW + 1);
        session.act("chat.post", &json!({"text": "y"}), NOW + 2);
        let review = WorldReview::from_audit("atlas", [session.audit_events()], 5);
        let agent = &review.agents[0];
        assert!(agent.parked);
        assert_eq!(agent.parked_axis.as_deref(), Some("frames"));
        assert!(
            agent
                .render()
                .contains("parked, quota exhausted on the frames axis")
        );
    }

    /// Acceptance 3: a repeated-refusal pattern produces a digest
    /// naming the role edit that would fix it.
    #[test]
    fn repeated_refusals_digest_into_the_role_edit_that_fixes_them() {
        let mut session = joined(grant(1, "archivist"));
        for at in 0..4 {
            session.act("screen.subscribe", &subscribe(), NOW + at);
        }
        // Rate limit: two admitted, then refused three times.
        for at in 0..5 {
            session.act("chat.post", &json!({"text": "x"}), NOW + 10 + at);
        }
        // A one-off ceiling refusal on another tool stays below the bar.
        session.act(
            "object.inspect",
            &json!({"entity_ref": "scroll:1"}),
            NOW + 20,
        );

        let digests = refusal_digests([session.audit_events()], |role| {
            assert_eq!(role, "archivist");
            3
        });
        assert_eq!(digests.len(), 2, "{digests:?}");
        let ceiling = &digests[0];
        assert_eq!(ceiling.count, 4);
        assert_eq!(ceiling.code, "ceiling");
        assert_eq!(
            ceiling.role_edit,
            Some(RoleEdit::AddTool {
                role: "archivist".to_owned(),
                tool: "screen.subscribe".to_owned(),
            })
        );
        assert_eq!(
            ceiling.render(),
            "role archivist (agent 01010101) was refused screen.subscribe 4 times (ceiling: tool screen.subscribe is outside the capability ceiling); fix: add tool screen.subscribe to role archivist's capability ceiling"
        );
        let rate = &digests[1];
        assert_eq!(rate.count, 3);
        assert_eq!(
            rate.role_edit,
            Some(RoleEdit::RaiseRateLimit {
                role: "archivist".to_owned(),
                tool: "chat.post".to_owned(),
                limit: 2,
                window_secs: 60,
            })
        );
        assert!(rate.render().ends_with(
            "fix: raise role archivist's rate limit for chat.post above 2 calls per 60s"
        ));

        // A threshold of 0 never digests.
        assert_eq!(refusal_digests([session.audit_events()], |_| 0), Vec::new());
    }

    #[test]
    fn digests_name_the_budget_edit_for_quota_and_parked_refusals() {
        let mut tight = grant(1, "archivist");
        tight.quota_class.budget = Quota {
            frames: 1,
            tool_calls: 10,
            program_invocations: 0,
        };
        let mut session = joined(tight);
        for at in 0..4 {
            session.act("chat.post", &json!({"text": "x"}), NOW + 1 + at);
        }
        let digests = refusal_digests([session.audit_events()], |_| 2);
        assert_eq!(digests.len(), 1, "{digests:?}");
        assert_eq!(digests[0].code, "parked");
        assert_eq!(digests[0].count, 3);
        assert_eq!(
            digests[0].role_edit,
            Some(RoleEdit::RaiseBudget {
                role: "archivist".to_owned(),
                axis: "frames".to_owned(),
            })
        );
    }

    #[test]
    fn stale_refusals_digest_without_a_role_edit() {
        let mut session = joined(grant(1, "archivist"));
        for at in 0..3 {
            session.act(
                "chat.post",
                &json!({"text": "x", "screen_revision": 99}),
                NOW + 100 * at,
            );
        }
        let digests = refusal_digests([session.audit_events()], |_| 3);
        assert_eq!(digests.len(), 1);
        assert_eq!(digests[0].role_edit, None);
        assert!(
            digests[0]
                .render()
                .ends_with("the agent acted on an outdated frame; no role edit fixes this")
        );
    }

    #[test]
    fn ceiling_leg_details_round_trip() {
        for leg in [
            CeilingLeg::Tool("object.export".to_owned()),
            CeilingLeg::World("atlas".to_owned()),
            CeilingLeg::ObjectClass("scroll".to_owned()),
            CeilingLeg::ClassNotEstablished,
            CeilingLeg::Program("tally".to_owned()),
            CeilingLeg::NoProgram,
        ] {
            assert_eq!(CeilingLeg::from_detail(&leg.to_string()), Some(leg));
        }
        assert_eq!(CeilingLeg::from_detail("something else"), None);
    }
}
