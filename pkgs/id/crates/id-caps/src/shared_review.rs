//! Review and quota for shared surfaces — v3 shared-agent-worlds step 5
//! (#10053, parent #6302).
//!
//! Oracle: `doc/2026-11-14_design_id6302_shared_agent_worlds_v3_outline.md`
//! §2.3, §2.4 and §6, carrying the v2 owner review tooling
//! ([`crate::owner_review`], #8302) over to the shared work surfaces
//! ([`crate::work_surfaces`], #10049/#10050). Every piece here reads the
//! surface audit log, which stays the record of truth.
//!
//! - **Delta summaries** ([`surface_delta`]): "what changed since I was
//!   last here" over shared surfaces, for the owner or a human visitor,
//!   from audit events alone. It is the same summary a visitor gets on
//!   arrival ([`crate::agent_majority::visitor_summary`]).
//! - **Above-ceiling requests on shared edits** ([`SharedEditQueue`]):
//!   when an agent's role may not make a shared edit and the role's
//!   [`crate::role_config::ReviewPolicy::queue_above_ceiling`] is on,
//!   the refused edit can be held as one request. The owner approves or
//!   denies it once, and an unanswered request expires. Approval
//!   executes exactly the held edit, once, under the owner's authority:
//!   the surface audit event names the owner, and the queue's
//!   [`SharedEditResolution`] names the agent it was for. It never
//!   edits the role or the surface's permissions — the queue only reads
//!   the [`RoleRegistry`] — so the next identical edit is refused again.
//!   Claims are not holdable: approving a claim under the owner's
//!   authority would make the owner the holder, which is a reassignment
//!   ([`crate::task_delegation`]), not an approval.
//! - **Refusal digests** ([`surface_refusal_digests`]): repeated refusals
//!   of one shape on one surface surface as one line, after the role's
//!   `digest_every_refusals`, naming the surface permission that would
//!   fix them — or saying that no role edit would.
//! - **Quota attribution** ([`SurfaceSpend`], [`SpendCharger`]): every
//!   executed surface act is attributed to the agent that acted, under
//!   the role it acted as; an approved above-ceiling edit is attributed
//!   to the agent it was for, never to the owner. Per-role aggregate
//!   spend is kept alongside per-agent spend. Attribution is keyed by
//!   audit event, so each act is charged at most once: a handoff
//!   (one agent releases, another claims) charges each agent for its
//!   own act and nothing for the other's.
//!
//! Task-carried budgets (a budget riding on a board item) are not
//! adopted in this step: the outline leaves them optional, and the
//! acting agent's own ledger already bounds every act.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::agent_majority::{VisitorSummary, visitor_summary};
use crate::agent_quota_ledger::AgentQuotaLedger;
use crate::mcp_policy::Quota;
use crate::owner_review::DEFAULT_REVIEW_TTL_SECS;
use crate::role_config::RoleRegistry;
use crate::work_surfaces::{
    EVENT_DOCUMENT_EDIT, EVENT_DOCUMENT_REVIEW, EVENT_ITEM_CLAIM, EVENT_ITEM_CREATE,
    EVENT_ITEM_MOVE, EVENT_ITEM_RELEASE, EVENT_SURFACE_READ, EditOp, ItemDraft, ReviewState,
    SurfaceActor, SurfaceError, SurfaceEvent, SurfaceId, WorkSurfaces,
};

/// The outcome a surface audit event carries when the actor's role may
/// not act there: the shape an above-ceiling request is held from.
pub const NOT_PERMITTED: &str = "refused:not_permitted";

/// What one executed surface act costs by default: one tool call.
pub const SURFACE_ACT_CHARGE: Quota = Quota {
    frames: 0,
    tool_calls: 1,
    program_invocations: 0,
};

// ---------------------------------------------------------------------------
// Delta summaries
// ---------------------------------------------------------------------------

/// "What changed since I was last here" over every shared surface in
/// the world, from the audit log: executed acts by kind, refusals, who
/// acted and which claims ran out, for the window after `since`
/// (exclusive; `None` for everything) through `through`. The owner and
/// a human visitor get the same view.
#[must_use]
pub fn surface_delta(surfaces: &WorkSurfaces, since: Option<u64>, through: u64) -> VisitorSummary {
    visitor_summary(surfaces.events(), since, through)
}

// ---------------------------------------------------------------------------
// Above-ceiling requests on shared edits
// ---------------------------------------------------------------------------

/// A shared edit an agent's role may not make, as it was attempted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SharedOp {
    /// Create an item on a board.
    Create {
        /// The board.
        board: String,
        /// The item.
        draft: ItemDraft,
    },
    /// Move an item to another state, closing states included.
    Move {
        /// The board.
        board: String,
        /// The item.
        item: u64,
        /// The target state.
        to: String,
        /// The item version the agent saw.
        seen_version: u64,
    },
    /// Edit a document.
    Edit {
        /// The document.
        document: String,
        /// The edit.
        edit: EditOp,
        /// The revision the agent decided against.
        decided_against: u64,
    },
    /// Change a document's review state.
    Review {
        /// The document.
        document: String,
        /// The new review state.
        review: ReviewState,
        /// The revision the agent reviewed.
        reviewed_revision: u64,
    },
}

impl SharedOp {
    /// The surface audit event kind this edit is recorded under.
    #[must_use]
    pub const fn event_kind(&self) -> &'static str {
        match self {
            Self::Create { .. } => EVENT_ITEM_CREATE,
            Self::Move { .. } => EVENT_ITEM_MOVE,
            Self::Edit { .. } => EVENT_DOCUMENT_EDIT,
            Self::Review { .. } => EVENT_DOCUMENT_REVIEW,
        }
    }

    /// The surface the edit names.
    #[must_use]
    pub fn surface(&self) -> SurfaceId {
        match self {
            Self::Create { board, .. } | Self::Move { board, .. } => {
                SurfaceId::Board(board.clone())
            }
            Self::Edit { document, .. } | Self::Review { document, .. } => {
                SurfaceId::Document(document.clone())
            }
        }
    }

    /// The item the edit names, when it names one.
    #[must_use]
    pub const fn item(&self) -> Option<u64> {
        match self {
            Self::Move { item, .. } => Some(*item),
            _ => None,
        }
    }

    /// Run the edit on `surfaces` as `actor`, returning the new item id,
    /// item version or document revision (`0` for a review).
    fn run(
        &self,
        surfaces: &mut WorkSurfaces,
        registry: &RoleRegistry,
        actor: &SurfaceActor,
        now: u64,
    ) -> Result<u64, SurfaceError> {
        match self {
            Self::Create { board, draft } => {
                surfaces.create_item(registry, actor, board, draft.clone(), now)
            }
            Self::Move {
                board,
                item,
                to,
                seen_version,
            } => surfaces.move_item(registry, actor, board, *item, to, *seen_version, now),
            Self::Edit {
                document,
                edit,
                decided_against,
            } => surfaces.edit_document(
                registry,
                actor,
                document,
                edit.clone(),
                *decided_against,
                now,
            ),
            Self::Review {
                document,
                review,
                reviewed_revision,
            } => surfaces
                .review_document(registry, actor, document, *review, *reviewed_revision, now)
                .map(|()| 0),
        }
    }
}

/// One refused shared edit held for the owner's one-shot decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedEditRequest {
    /// The request's id in its queue.
    pub id: u64,
    /// The agent the edit was refused for, lower-case hex.
    pub agent: String,
    /// The role it acted under.
    pub role: String,
    /// The world owner: the only one who may decide.
    pub owner: String,
    /// The edit, exactly as attempted: approval runs this and nothing
    /// else.
    pub op: SharedOp,
    /// The refusal's position in the surface audit log.
    pub refused_index: usize,
    /// When the request was held, unix seconds.
    pub held_at: u64,
    /// When it expires unanswered, unix seconds.
    pub expires_at: u64,
}

/// How a held shared edit ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum SharedDecision {
    /// The owner approved it and it ran once.
    Approved {
        /// Whether the edit executed: the surface's own checks (a stale
        /// version, a live claim) still apply to it.
        executed: bool,
        /// `executed`, or `refused:<code>`.
        outcome: String,
    },
    /// The owner denied it; nothing ran.
    Denied,
    /// Nobody decided before it expired; nothing ran.
    Expired,
}

/// The queue's record of one request's end.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedEditResolution {
    /// The request.
    pub request_id: u64,
    /// The agent it was for, lower-case hex.
    pub agent: String,
    /// The role the agent acted under.
    pub role: String,
    /// The edit's surface audit event kind.
    pub kind: String,
    /// The surface it named.
    pub surface: SurfaceId,
    /// How it ended.
    pub decision: SharedDecision,
    /// Who decided; `None` for an expiry.
    pub by: Option<String>,
    /// When it ended, unix seconds.
    pub at: u64,
    /// The position in the surface audit log of the owner-authority
    /// event the approval produced, when it executed.
    pub executed_index: Option<usize>,
}

/// Why a refused edit could not be held.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OfferRefusal {
    /// Only an agent's edit can be held.
    NotAnAgent,
    /// The agent's role does not queue above-ceiling requests.
    QueueingOff,
    /// The latest surface audit event is not this agent's
    /// `not_permitted` refusal of this edit: only an edit the role's
    /// surface permissions actually refused can be held.
    NotACeilingRefusal,
    /// This refusal is already held, or was already decided.
    AlreadyHeld {
        /// The request that holds it.
        request_id: u64,
    },
}

impl OfferRefusal {
    /// Stable machine-readable code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotAnAgent => "not_an_agent",
            Self::QueueingOff => "queueing_off",
            Self::NotACeilingRefusal => "not_a_ceiling_refusal",
            Self::AlreadyHeld { .. } => "already_held",
        }
    }
}

/// Why a decision on a request was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecideError {
    /// No such request is pending: unknown, or already decided.
    NotPending,
    /// The request expired before the decision; it is now resolved as
    /// expired.
    Expired,
    /// Only the world owner decides.
    NotOwner {
        /// The owner who may decide.
        owner: String,
    },
}

impl DecideError {
    /// Stable machine-readable code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotPending => "not_pending",
            Self::Expired => "expired",
            Self::NotOwner { .. } => "not_owner",
        }
    }
}

/// The owner's one-shot approval queue for refused shared edits in one
/// world. A request is held from a `not_permitted` refusal, decided at
/// most once, and expires unanswered after the queue's TTL.
#[derive(Clone, Debug)]
pub struct SharedEditQueue {
    ttl_secs: u64,
    next_id: u64,
    pending: BTreeMap<u64, SharedEditRequest>,
    held: BTreeMap<usize, u64>,
    resolutions: Vec<SharedEditResolution>,
}

impl Default for SharedEditQueue {
    fn default() -> Self {
        Self::new(DEFAULT_REVIEW_TTL_SECS)
    }
}

impl SharedEditQueue {
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

    /// Hold the edit `actor` just attempted and the surfaces refused as
    /// `not_permitted`, for `owner`'s one-shot decision. The host calls
    /// this right after the refusal, passing the same edit; the queue
    /// checks that the latest surface audit event is that refusal.
    ///
    /// # Errors
    ///
    /// [`OfferRefusal`] when the actor is not an agent, its role does
    /// not queue above-ceiling requests, the latest event is not this
    /// edit's `not_permitted` refusal, or the refusal is already held.
    pub fn offer(
        &mut self,
        surfaces: &WorkSurfaces,
        registry: &RoleRegistry,
        owner: &str,
        actor: &SurfaceActor,
        op: SharedOp,
        now: u64,
    ) -> Result<u64, OfferRefusal> {
        let SurfaceActor::Agent { agent, role } = actor else {
            return Err(OfferRefusal::NotAnAgent);
        };
        let queues = registry
            .record(role)
            .is_some_and(|record| record.definition.review.queue_above_ceiling);
        if !queues {
            return Err(OfferRefusal::QueueingOff);
        }
        let events = surfaces.events();
        let index = events
            .len()
            .checked_sub(1)
            .ok_or(OfferRefusal::NotACeilingRefusal)?;
        let event = &events[index];
        let matches = event.kind == op.event_kind()
            && event.surface == op.surface()
            && event.item == op.item()
            && event.actor == agent_label(agent, role)
            && event.outcome == NOT_PERMITTED;
        if !matches {
            return Err(OfferRefusal::NotACeilingRefusal);
        }
        if let Some(request_id) = self.held.get(&index) {
            return Err(OfferRefusal::AlreadyHeld {
                request_id: *request_id,
            });
        }
        let id = self.next_id;
        self.next_id += 1;
        self.held.insert(index, id);
        self.pending.insert(
            id,
            SharedEditRequest {
                id,
                agent: agent.clone(),
                role: role.clone(),
                owner: owner.to_owned(),
                op,
                refused_index: index,
                held_at: now,
                expires_at: now.saturating_add(self.ttl_secs),
            },
        );
        Ok(id)
    }

    /// The requests waiting for a decision, in id order.
    #[must_use]
    pub fn pending(&self) -> Vec<&SharedEditRequest> {
        self.pending.values().collect()
    }

    /// Every decided or expired request, in the order it ended.
    #[must_use]
    pub fn resolutions(&self) -> &[SharedEditResolution] {
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
            self.resolve(*id, SharedDecision::Expired, None, now, None);
        }
        due
    }

    /// Approve request `request_id` once: run exactly the held edit on
    /// `surfaces` as the owner. The surface checks still run; the role
    /// and the surface's permissions are untouched.
    ///
    /// # Errors
    ///
    /// [`DecideError`] when the request is not pending, has expired (it
    /// is resolved as expired), or `approver` is not the owner. The
    /// inner result is the edit's own outcome.
    pub fn approve(
        &mut self,
        request_id: u64,
        approver: &str,
        surfaces: &mut WorkSurfaces,
        registry: &RoleRegistry,
        now: u64,
    ) -> Result<Result<u64, SurfaceError>, DecideError> {
        let request = self.decidable(request_id, approver, now)?.clone();
        let owner = SurfaceActor::owner(approver);
        let result = request.op.run(surfaces, registry, &owner, now);
        let outcome = match &result {
            Ok(_) => "executed".to_owned(),
            Err(error) => format!("refused:{}", error.code()),
        };
        let executed_index = if result.is_ok() {
            surfaces.events().len().checked_sub(1)
        } else {
            None
        };
        self.resolve(
            request_id,
            SharedDecision::Approved {
                executed: result.is_ok(),
                outcome,
            },
            Some(approver.to_owned()),
            now,
            executed_index,
        );
        Ok(result)
    }

    /// Deny request `request_id`: nothing runs.
    ///
    /// # Errors
    ///
    /// [`DecideError`] when the request is not pending, has expired, or
    /// `approver` is not the owner.
    pub fn deny(&mut self, request_id: u64, approver: &str, now: u64) -> Result<(), DecideError> {
        self.decidable(request_id, approver, now)?;
        self.resolve(
            request_id,
            SharedDecision::Denied,
            Some(approver.to_owned()),
            now,
            None,
        );
        Ok(())
    }

    fn decidable(
        &mut self,
        request_id: u64,
        approver: &str,
        now: u64,
    ) -> Result<&SharedEditRequest, DecideError> {
        let (expired, owner) = match self.pending.get(&request_id) {
            None => return Err(DecideError::NotPending),
            Some(request) => (now >= request.expires_at, request.owner.clone()),
        };
        if expired {
            self.resolve(request_id, SharedDecision::Expired, None, now, None);
            return Err(DecideError::Expired);
        }
        if owner != approver {
            return Err(DecideError::NotOwner { owner });
        }
        self.pending.get(&request_id).ok_or(DecideError::NotPending)
    }

    fn resolve(
        &mut self,
        request_id: u64,
        decision: SharedDecision,
        by: Option<String>,
        at: u64,
        executed_index: Option<usize>,
    ) {
        if let Some(request) = self.pending.remove(&request_id) {
            self.resolutions.push(SharedEditResolution {
                request_id,
                kind: request.op.event_kind().to_owned(),
                surface: request.op.surface(),
                agent: request.agent,
                role: request.role,
                decision,
                by,
                at,
                executed_index,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Refusal digests
// ---------------------------------------------------------------------------

/// The surface permission edit that would stop a pattern of refusals.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfacePermissionEdit {
    /// The surface whose configuration to edit.
    pub surface: SurfaceId,
    /// The role to give the permission to.
    pub role: String,
    /// The permission, by its configuration field name.
    pub permission: String,
}

/// Repeated refusals of one shape on one surface, as one line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceRefusalDigest {
    /// The agent, lower-case hex.
    pub agent: String,
    /// The role it acted under.
    pub role: String,
    /// The refused act's event kind.
    pub kind: String,
    /// The surface.
    pub surface: SurfaceId,
    /// The refusal code.
    pub code: String,
    /// How many times.
    pub count: u64,
    /// The first and last refusal, unix seconds.
    pub first_at: u64,
    /// See `first_at`.
    pub last_at: u64,
    /// The configuration edit that would fix it, when one would.
    pub fix: Option<SurfacePermissionEdit>,
}

impl SurfaceRefusalDigest {
    /// One owner-facing line.
    #[must_use]
    pub fn render(&self) -> String {
        let short: String = self.agent.chars().take(8).collect();
        let head = format!(
            "agent {short} ({}) was refused {} on {} {} times ({})",
            self.role,
            self.kind,
            surface_text(&self.surface),
            self.count,
            self.code
        );
        self.fix.as_ref().map_or_else(
            || format!("{head}; no role edit fixes this"),
            |fix| {
                format!(
                    "{head}; a role edit fixes it: give {} {} on {}",
                    fix.role,
                    fix.permission,
                    surface_text(&fix.surface)
                )
            },
        )
    }
}

/// The repeated refusal shapes on shared surfaces at or after `since`:
/// agent, role, act, surface and code, once their count reaches the
/// role's `digest_every_refusals` (a role with `0`, or one the registry
/// no longer holds, is never digested). Ordered by count, highest
/// first.
#[must_use]
pub fn surface_refusal_digests(
    events: &[SurfaceEvent],
    registry: &RoleRegistry,
    since: u64,
) -> Vec<SurfaceRefusalDigest> {
    type Shape = (String, String, String, SurfaceId, String);
    let mut shapes: BTreeMap<Shape, (u64, u64, u64)> = BTreeMap::new();
    for event in events.iter().filter(|e| e.at >= since) {
        let Some(code) = event.outcome.strip_prefix("refused:") else {
            continue;
        };
        let Some((agent, role)) = parse_agent(&event.actor) else {
            continue;
        };
        let entry = shapes
            .entry((
                agent.to_owned(),
                role.to_owned(),
                event.kind.clone(),
                event.surface.clone(),
                code.to_owned(),
            ))
            .or_insert((0, event.at, event.at));
        entry.0 += 1;
        entry.1 = entry.1.min(event.at);
        entry.2 = entry.2.max(event.at);
    }
    let mut digests: Vec<SurfaceRefusalDigest> = shapes
        .into_iter()
        .filter_map(
            |((agent, role, kind, surface, code), (count, first, last))| {
                let every = registry
                    .record(&role)?
                    .definition
                    .review
                    .digest_every_refusals;
                if every == 0 || count < every {
                    return None;
                }
                let fix = permission_for(&kind)
                    .filter(|_| code == "not_permitted")
                    .map(|permission| SurfacePermissionEdit {
                        surface: surface.clone(),
                        role: role.clone(),
                        permission: permission.to_owned(),
                    });
                Some(SurfaceRefusalDigest {
                    agent,
                    role,
                    kind,
                    surface,
                    code,
                    count,
                    first_at: first,
                    last_at: last,
                    fix,
                })
            },
        )
        .collect();
    digests.sort_by(|a, b| b.count.cmp(&a.count));
    digests
}

fn permission_for(kind: &str) -> Option<&'static str> {
    match kind {
        EVENT_SURFACE_READ => Some("read"),
        EVENT_ITEM_CREATE => Some("create"),
        EVENT_ITEM_CLAIM | EVENT_ITEM_RELEASE => Some("claim"),
        EVENT_ITEM_MOVE => Some("move_items (or close, for a closing state)"),
        EVENT_DOCUMENT_EDIT => Some("edit"),
        EVENT_DOCUMENT_REVIEW => Some("review"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Quota attribution
// ---------------------------------------------------------------------------

/// Spend on shared surfaces, attributed to the acting agent and
/// aggregated per role, from the audit log.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceSpend {
    /// Per agent (lower-case hex).
    pub by_agent: BTreeMap<String, Quota>,
    /// Per role: the first-class aggregate.
    pub by_role: BTreeMap<String, Quota>,
    /// Each charged audit event, by position, with the agent and role
    /// charged: an event appears at most once.
    pub charged: BTreeMap<usize, (String, String)>,
}

impl SurfaceSpend {
    /// Attribute every executed surface act in `events` at `per_act`:
    /// an agent's act to that agent and role; an owner act that
    /// executed an approved request to the agent the request was for;
    /// any other owner or host act to nobody.
    #[must_use]
    pub fn from_audit(
        events: &[SurfaceEvent],
        resolutions: &[SharedEditResolution],
        per_act: Quota,
    ) -> Self {
        let approved = approved_for(resolutions);
        let mut spend = Self::default();
        for (index, event) in events.iter().enumerate() {
            if let Some((agent, role)) = payer(index, event, &approved) {
                spend
                    .by_agent
                    .entry(agent.clone())
                    .or_default()
                    .add(per_act);
                spend.by_role.entry(role.clone()).or_default().add(per_act);
                spend.charged.insert(index, (agent, role));
            }
        }
        spend
    }

    /// One agent's spend.
    #[must_use]
    pub fn agent(&self, agent: &str) -> Quota {
        self.by_agent.get(agent).copied().unwrap_or_default()
    }

    /// One role's aggregate spend.
    #[must_use]
    pub fn role(&self, role: &str) -> Quota {
        self.by_role.get(role).copied().unwrap_or_default()
    }
}

/// A charge a ledger would not take.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnpaidCharge {
    /// The audit event's position.
    pub index: usize,
    /// The agent, lower-case hex.
    pub agent: String,
    /// `no_ledger`, or the exhausted budget axis.
    pub reason: String,
}

/// Charges the acting agent's ledger for each executed surface act,
/// once: it walks the audit log from where it last stopped, so calling
/// it again never charges an act twice.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpendCharger {
    cursor: usize,
}

impl SpendCharger {
    /// A charger that has charged nothing yet.
    #[must_use]
    pub const fn new() -> Self {
        Self { cursor: 0 }
    }

    /// The audit position the next call starts from.
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    /// Charge `per_act` for every executed act after the cursor to the
    /// paying agent's ledger in `ledgers` (keyed by agent hex). An act
    /// whose agent has no ledger in this world, or whose ledger is
    /// exhausted, is returned as unpaid — it already happened, so it is
    /// reported, never retried.
    pub fn charge(
        &mut self,
        surfaces: &WorkSurfaces,
        resolutions: &[SharedEditResolution],
        ledgers: &mut BTreeMap<String, AgentQuotaLedger>,
        per_act: Quota,
        now: u64,
    ) -> Vec<UnpaidCharge> {
        let approved = approved_for(resolutions);
        let events = surfaces.events();
        let mut unpaid = Vec::new();
        for (index, event) in events.iter().enumerate().skip(self.cursor) {
            let Some((agent, _role)) = payer(index, event, &approved) else {
                continue;
            };
            let reason = match ledgers.get_mut(&agent) {
                Some(ledger) if ledger.world() == surfaces.world() => ledger
                    .charge(now, per_act)
                    .err()
                    .map(|exhausted| exhausted.axis),
                _ => Some("no_ledger".to_owned()),
            };
            if let Some(reason) = reason {
                unpaid.push(UnpaidCharge {
                    index,
                    agent,
                    reason,
                });
            }
        }
        self.cursor = events.len();
        unpaid
    }
}

fn approved_for(resolutions: &[SharedEditResolution]) -> BTreeMap<usize, (String, String)> {
    resolutions
        .iter()
        .filter_map(|r| {
            r.executed_index
                .map(|index| (index, (r.agent.clone(), r.role.clone())))
        })
        .collect()
}

fn payer(
    index: usize,
    event: &SurfaceEvent,
    approved: &BTreeMap<usize, (String, String)>,
) -> Option<(String, String)> {
    if event.outcome != "executed" {
        return None;
    }
    if let Some((agent, role)) = parse_agent(&event.actor) {
        return Some((agent.to_owned(), role.to_owned()));
    }
    approved.get(&index).cloned()
}

fn agent_label(agent: &str, role: &str) -> String {
    format!("agent:{agent}/{role}")
}

fn parse_agent(actor: &str) -> Option<(&str, &str)> {
    actor.strip_prefix("agent:")?.split_once('/')
}

fn surface_text(surface: &SurfaceId) -> String {
    match surface {
        SurfaceId::Board(name) => format!("board {name}"),
        SurfaceId::Document(name) => format!("document {name}"),
    }
}

/// The agents a set of resolutions names, for owner-facing lists.
#[must_use]
pub fn requesting_agents(resolutions: &[SharedEditResolution]) -> BTreeSet<String> {
    resolutions.iter().map(|r| r.agent.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_delegation::AgentId;
    use crate::role_config::QuotaClass;
    use crate::work_surfaces::{BoardDefinition, ConflictModel, DocumentDefinition};

    const WORLD: &str = "studio";
    const T0: u64 = 1_800_000_000;
    const BOARD: &str = "work";
    const DOC: &str = "notes";

    fn agent(byte: u8) -> AgentId {
        AgentId([byte; 32])
    }

    fn hex(byte: u8) -> String {
        agent(byte).to_hex()
    }

    /// Starter roles. Agents 1–2 are participants, 3 an
    /// archivist-editor (queues above-ceiling requests, digests every
    /// 5), 5 an observer (no queue, digests every 10).
    fn registry() -> RoleRegistry {
        let mut registry = RoleRegistry::new(WORLD);
        registry.install_starters("owner", T0);
        for (byte, role) in [
            (1, "participant"),
            (2, "participant"),
            (3, "archivist-editor"),
            (5, "observer"),
        ] {
            assert!(
                registry
                    .grant(role, agent(byte), "owner", T0)
                    .decision
                    .is_ok()
            );
        }
        registry
    }

    fn actor(byte: u8, role: &str) -> SurfaceActor {
        SurfaceActor::agent(agent(byte), role)
    }

    fn owner() -> SurfaceActor {
        SurfaceActor::owner("owner")
    }

    fn draft(title: &str) -> ItemDraft {
        ItemDraft {
            title: title.to_owned(),
            ..ItemDraft::default()
        }
    }

    fn setup() -> (RoleRegistry, WorkSurfaces) {
        let registry = registry();
        let mut surfaces = WorkSurfaces::new(WORLD);
        surfaces
            .define_board(&owner(), BoardDefinition::starter(BOARD), T0)
            .unwrap();
        surfaces
            .define_document(
                &owner(),
                DocumentDefinition {
                    name: DOC.to_owned(),
                    conflict_model: ConflictModel::AppendOnly,
                    roles: BTreeMap::new(),
                },
                T0,
            )
            .unwrap();
        (registry, surfaces)
    }

    fn ledger(byte: u8, role: &str, tool_calls: u64) -> AgentQuotaLedger {
        AgentQuotaLedger::open(
            [byte; 32],
            WORLD,
            role,
            QuotaClass {
                budget: Quota {
                    frames: 0,
                    tool_calls,
                    program_invocations: 0,
                },
                replenishment: None,
            },
            T0,
        )
    }

    fn item_version(surfaces: &mut WorkSurfaces, registry: &RoleRegistry, item: u64) -> u64 {
        surfaces
            .board(registry, &owner(), BOARD, T0)
            .unwrap()
            .items
            .iter()
            .find(|i| i.id == item)
            .unwrap()
            .version
    }

    #[test]
    fn owner_and_visitor_see_the_delta_from_the_audit_log() {
        let (registry, mut surfaces) = setup();
        let item = surfaces
            .create_item(
                &registry,
                &actor(1, "participant"),
                BOARD,
                draft("a"),
                T0 + 10,
            )
            .unwrap();
        let seen = item_version(&mut surfaces, &registry, item);
        surfaces
            .claim(
                &registry,
                &actor(1, "participant"),
                BOARD,
                item,
                seen,
                T0 + 11,
            )
            .unwrap();
        let _ = surfaces.create_item(&registry, &actor(5, "observer"), BOARD, draft("b"), T0 + 12);
        let delta = surface_delta(&surfaces, Some(T0 + 5), T0 + 20);
        let board = &delta.surfaces[&SurfaceId::Board(BOARD.to_owned())];
        assert_eq!(board.executed.get(EVENT_ITEM_CREATE), Some(&1));
        assert_eq!(board.executed.get(EVENT_ITEM_CLAIM), Some(&1));
        assert_eq!(board.refused, 1);
        assert!(board.actors.contains(&agent_label(&hex(1), "participant")));
        // The window is the audit log's: nothing before `since`.
        let quiet = surface_delta(&surfaces, Some(T0 + 20), T0 + 30);
        assert!(quiet.is_quiet());
        // The visitor's arrival summary is the same view.
        assert_eq!(
            delta,
            visitor_summary(surfaces.events(), Some(T0 + 5), T0 + 20)
        );
    }

    #[test]
    fn above_ceiling_edit_is_held_expires_and_executes_once_under_the_owner() {
        let (registry, mut surfaces) = setup();
        let mut queue = SharedEditQueue::new(600);
        let editor = actor(3, "archivist-editor");
        let op = SharedOp::Create {
            board: BOARD.to_owned(),
            draft: draft("needs the owner"),
        };
        // The board gives archivist-editor nothing: refused.
        assert_eq!(
            surfaces.create_item(&registry, &editor, BOARD, draft("needs the owner"), T0 + 1),
            Err(SurfaceError::NotPermitted)
        );
        // Only the agent's own not_permitted refusal of this edit holds.
        assert_eq!(
            queue.offer(&surfaces, &registry, "owner", &owner(), op.clone(), T0 + 1),
            Err(OfferRefusal::NotAnAgent)
        );
        let other = SharedOp::Create {
            board: "elsewhere".to_owned(),
            draft: draft("x"),
        };
        assert_eq!(
            queue.offer(&surfaces, &registry, "owner", &editor, other, T0 + 1),
            Err(OfferRefusal::NotACeilingRefusal)
        );
        let id = queue
            .offer(&surfaces, &registry, "owner", &editor, op.clone(), T0 + 1)
            .unwrap();
        assert_eq!(
            queue.offer(&surfaces, &registry, "owner", &editor, op.clone(), T0 + 1),
            Err(OfferRefusal::AlreadyHeld { request_id: id })
        );
        // An observer's role does not queue.
        let observer = actor(5, "observer");
        let _ = surfaces.create_item(&registry, &observer, BOARD, draft("x"), T0 + 2);
        assert_eq!(
            queue.offer(
                &surfaces,
                &registry,
                "owner",
                &observer,
                SharedOp::Create {
                    board: BOARD.to_owned(),
                    draft: draft("x"),
                },
                T0 + 2,
            ),
            Err(OfferRefusal::QueueingOff)
        );
        // Only the owner decides.
        assert_eq!(
            queue.approve(id, "someone", &mut surfaces, &registry, T0 + 3),
            Err(DecideError::NotOwner {
                owner: "owner".to_owned()
            })
        );
        let item = queue
            .approve(id, "owner", &mut surfaces, &registry, T0 + 3)
            .unwrap()
            .unwrap();
        let executed = surfaces.events().last().unwrap();
        assert_eq!(executed.kind, EVENT_ITEM_CREATE);
        assert_eq!(executed.actor, "owner:owner");
        assert_eq!(executed.outcome, "executed");
        assert_eq!(executed.item, Some(item));
        let resolution = &queue.resolutions()[0];
        assert_eq!(resolution.agent, hex(3));
        assert_eq!(resolution.by.as_deref(), Some("owner"));
        assert_eq!(
            resolution.decision,
            SharedDecision::Approved {
                executed: true,
                outcome: "executed".to_owned()
            }
        );
        assert_eq!(resolution.executed_index, Some(surfaces.events().len() - 1));
        // Once: the request is gone.
        assert_eq!(
            queue.approve(id, "owner", &mut surfaces, &registry, T0 + 4),
            Err(DecideError::NotPending)
        );
        // An unanswered request expires; nothing runs.
        let _ = surfaces.create_item(&registry, &editor, BOARD, draft("late"), T0 + 5);
        let late = queue
            .offer(
                &surfaces,
                &registry,
                "owner",
                &editor,
                SharedOp::Create {
                    board: BOARD.to_owned(),
                    draft: draft("late"),
                },
                T0 + 5,
            )
            .unwrap();
        let before = surfaces.events().len();
        assert_eq!(
            queue.approve(late, "owner", &mut surfaces, &registry, T0 + 5 + 600),
            Err(DecideError::Expired)
        );
        assert_eq!(surfaces.events().len(), before);
        assert_eq!(
            queue.resolutions().last().unwrap().decision,
            SharedDecision::Expired
        );
        // A denial runs nothing either.
        let _ = surfaces.create_item(&registry, &editor, BOARD, draft("no"), T0 + 700);
        let denied = queue
            .offer(
                &surfaces,
                &registry,
                "owner",
                &editor,
                SharedOp::Create {
                    board: BOARD.to_owned(),
                    draft: draft("no"),
                },
                T0 + 700,
            )
            .unwrap();
        queue.deny(denied, "owner", T0 + 701).unwrap();
        assert_eq!(queue.pending().len(), 0);
        assert_eq!(
            requesting_agents(queue.resolutions()),
            BTreeSet::from([hex(3)])
        );
    }

    #[test]
    fn approval_runs_the_surface_checks_and_holds_document_edits() {
        let (registry, mut surfaces) = setup();
        let mut queue = SharedEditQueue::default();
        let editor = actor(3, "archivist-editor");
        let edit = EditOp::Append {
            text: "a line".to_owned(),
        };
        assert_eq!(
            surfaces.edit_document(&registry, &editor, DOC, edit.clone(), 0, T0 + 1),
            Err(SurfaceError::NotPermitted)
        );
        let id = queue
            .offer(
                &surfaces,
                &registry,
                "owner",
                &editor,
                SharedOp::Edit {
                    document: DOC.to_owned(),
                    edit,
                    decided_against: 0,
                },
                T0 + 1,
            )
            .unwrap();
        // The document moves on before the owner decides: the approved
        // edit is refused as stale, never merged.
        surfaces
            .edit_document(
                &registry,
                &owner(),
                DOC,
                EditOp::Append {
                    text: "first".to_owned(),
                },
                0,
                T0 + 2,
            )
            .unwrap();
        let outcome = queue
            .approve(id, "owner", &mut surfaces, &registry, T0 + 3)
            .unwrap();
        assert!(matches!(outcome, Err(SurfaceError::StaleDecision(_))));
        let resolution = &queue.resolutions()[0];
        assert_eq!(
            resolution.decision,
            SharedDecision::Approved {
                executed: false,
                outcome: "refused:stale_decision".to_owned()
            }
        );
        assert_eq!(resolution.executed_index, None);
    }

    #[test]
    fn refusal_digests_surface_repeated_shapes() {
        let (registry, mut surfaces) = setup();
        for i in 0..6 {
            let _ = surfaces.create_item(
                &registry,
                &actor(3, "archivist-editor"),
                BOARD,
                draft("x"),
                T0 + i,
            );
        }
        for i in 0..3 {
            let _ =
                surfaces.create_item(&registry, &actor(5, "observer"), BOARD, draft("y"), T0 + i);
        }
        let digests = surface_refusal_digests(surfaces.events(), &registry, T0);
        assert_eq!(digests.len(), 1, "the observer is under its threshold");
        let digest = &digests[0];
        assert_eq!(digest.agent, hex(3));
        assert_eq!(digest.count, 6);
        assert_eq!(digest.code, "not_permitted");
        assert_eq!((digest.first_at, digest.last_at), (T0, T0 + 5));
        assert_eq!(
            digest.fix,
            Some(SurfacePermissionEdit {
                surface: SurfaceId::Board(BOARD.to_owned()),
                role: "archivist-editor".to_owned(),
                permission: "create".to_owned(),
            })
        );
        let line = digest.render();
        assert!(line.contains("refused surface.item.create on board work 6 times"));
        assert!(line.contains("give archivist-editor create on board work"));
        // A refusal no role edit fixes says so.
        let item = surfaces
            .create_item(
                &registry,
                &actor(1, "participant"),
                BOARD,
                draft("z"),
                T0 + 10,
            )
            .unwrap();
        let seen = item_version(&mut surfaces, &registry, item);
        surfaces
            .claim(
                &registry,
                &actor(1, "participant"),
                BOARD,
                item,
                seen,
                T0 + 11,
            )
            .unwrap();
        for i in 0..10 {
            let _ = surfaces.claim(
                &registry,
                &actor(2, "participant"),
                BOARD,
                item,
                seen + 1,
                T0 + 12 + i,
            );
        }
        let digests = surface_refusal_digests(surfaces.events(), &registry, T0 + 10);
        assert_eq!(digests.len(), 1);
        assert_eq!(digests[0].fix, None);
        assert!(digests[0].render().ends_with("no role edit fixes this"));
    }

    #[test]
    fn quota_charges_the_acting_agent_once_per_act() {
        let (registry, mut surfaces) = setup();
        let mut queue = SharedEditQueue::default();
        let a = actor(1, "participant");
        let b = actor(2, "participant");
        // Handoff: A creates and claims, releases; B claims and moves.
        let item = surfaces
            .create_item(&registry, &a, BOARD, draft("handoff"), T0 + 1)
            .unwrap();
        let seen = item_version(&mut surfaces, &registry, item);
        surfaces
            .claim(&registry, &a, BOARD, item, seen, T0 + 2)
            .unwrap();
        surfaces
            .release(&registry, &a, BOARD, item, T0 + 3)
            .unwrap();
        let seen = item_version(&mut surfaces, &registry, item);
        surfaces
            .claim(&registry, &b, BOARD, item, seen, T0 + 4)
            .unwrap();
        let seen = item_version(&mut surfaces, &registry, item);
        surfaces
            .move_item(&registry, &b, BOARD, item, "doing", seen, T0 + 5)
            .unwrap();
        // An above-ceiling create, refused then approved for agent 3.
        let editor = actor(3, "archivist-editor");
        let _ = surfaces.create_item(&registry, &editor, BOARD, draft("held"), T0 + 6);
        let id = queue
            .offer(
                &surfaces,
                &registry,
                "owner",
                &editor,
                SharedOp::Create {
                    board: BOARD.to_owned(),
                    draft: draft("held"),
                },
                T0 + 6,
            )
            .unwrap();
        queue
            .approve(id, "owner", &mut surfaces, &registry, T0 + 7)
            .unwrap()
            .unwrap();
        // An owner act of its own is nobody's spend.
        surfaces
            .create_item(&registry, &owner(), BOARD, draft("owner's"), T0 + 8)
            .unwrap();

        let spend =
            SurfaceSpend::from_audit(surfaces.events(), queue.resolutions(), SURFACE_ACT_CHARGE);
        assert_eq!(spend.agent(&hex(1)).tool_calls, 3);
        assert_eq!(spend.agent(&hex(2)).tool_calls, 2);
        assert_eq!(
            spend.agent(&hex(3)).tool_calls,
            1,
            "approved act, not the refusal"
        );
        assert_eq!(spend.role("participant").tool_calls, 5);
        assert_eq!(spend.role("archivist-editor").tool_calls, 1);
        assert_eq!(spend.charged.len(), 6);

        let mut ledgers = BTreeMap::from([
            (hex(1), ledger(1, "participant", 10)),
            (hex(2), ledger(2, "participant", 1)),
        ]);
        let mut charger = SpendCharger::new();
        let unpaid = charger.charge(
            &surfaces,
            queue.resolutions(),
            &mut ledgers,
            SURFACE_ACT_CHARGE,
            T0 + 9,
        );
        assert_eq!(ledgers[&hex(1)].spend_at(T0 + 9).tool_calls, 3);
        assert_eq!(ledgers[&hex(2)].spend_at(T0 + 9).tool_calls, 1);
        let reasons: Vec<(&str, &str)> = unpaid
            .iter()
            .map(|u| (u.agent.as_str(), u.reason.as_str()))
            .collect();
        assert_eq!(reasons.len(), 2);
        assert!(reasons.contains(&(hex(2).as_str(), "tool_calls")));
        assert!(reasons.contains(&(hex(3).as_str(), "no_ledger")));
        // Charging again charges nothing twice.
        assert_eq!(charger.cursor(), surfaces.events().len());
        assert!(
            charger
                .charge(
                    &surfaces,
                    queue.resolutions(),
                    &mut ledgers,
                    SURFACE_ACT_CHARGE,
                    T0 + 10
                )
                .is_empty()
        );
        assert_eq!(ledgers[&hex(1)].spend_at(T0 + 10).tool_calls, 3);
    }

    #[test]
    fn an_approved_above_ceiling_edit_never_persists_as_a_ceiling_change() {
        let (registry, mut surfaces) = setup();
        let mut queue = SharedEditQueue::default();
        let editor = actor(3, "archivist-editor");
        let role_before = registry.record("archivist-editor").unwrap().clone();
        let board_before = surfaces
            .board(&registry, &owner(), BOARD, T0)
            .unwrap()
            .definition;
        let _ = surfaces.create_item(&registry, &editor, BOARD, draft("once"), T0 + 1);
        let id = queue
            .offer(
                &surfaces,
                &registry,
                "owner",
                &editor,
                SharedOp::Create {
                    board: BOARD.to_owned(),
                    draft: draft("once"),
                },
                T0 + 1,
            )
            .unwrap();
        queue
            .approve(id, "owner", &mut surfaces, &registry, T0 + 2)
            .unwrap()
            .unwrap();
        // The same edit again is refused again.
        assert_eq!(
            surfaces.create_item(&registry, &editor, BOARD, draft("once"), T0 + 3),
            Err(SurfaceError::NotPermitted)
        );
        assert_eq!(registry.record("archivist-editor").unwrap(), &role_before);
        assert_eq!(
            surfaces
                .board(&registry, &owner(), BOARD, T0 + 3)
                .unwrap()
                .definition,
            board_before
        );
    }
}
