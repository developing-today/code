//! Agent-to-agent task delegation — v3 shared-agent-worlds step 3
//! (#10051, parent #6302).
//!
//! Oracle: `doc/2026-11-14_design_id6302_shared_agent_worlds_v3_outline.md`
//! §2.2–§2.4, over the shared work surfaces ([`crate::work_surfaces`],
//! #10049/#10050), the v2 role configuration ([`crate::role_config`],
//! #8299) and the per-agent quota ledger ([`crate::agent_quota_ledger`],
//! #8301).
//!
//! # Handing work, not authority
//!
//! - **Assignment is world state.** An agent creates a task on a shared
//!   board and assigns it to a role — "any agent holding role X" — or to
//!   one agent under one of its roles ([`Assignee`]). The assignment is a
//!   record next to the board item. It mints nothing, delegates nothing,
//!   and changes no grant or ceiling: the v2 §2.4 authority ban stands.
//! - **The assignee acts under its own role.** Only an agent the
//!   assignment names may claim, and it claims as itself under its own
//!   role, so the board's permissions for that role, its ceiling and its
//!   quota apply — never the assigner's.
//! - **Claims are atomic at the host.** A claim goes through
//!   [`WorkSurfaces::claim`], which checks the item version the claimer
//!   saw and any live claim; concurrent claims resolve to one claimant
//!   and every other one is refused and audited.
//! - **The acting agent pays.** Every agent act through this module
//!   charges [`TASK_ACT_CHARGE`] to the acting agent's own ledger, which
//!   must be that agent's ledger in this world. A refused act charges
//!   nothing; an exhausted ledger refuses the act.
//! - **Nothing is released silently.** A claimant that parks or runs out
//!   of quota keeps its claim until its lease runs out
//!   ([`TaskDelegation::expire`]) or the owner releases it with a reason
//!   ([`TaskDelegation::owner_release`]); both are audited naming the
//!   assigner, the claimant and the task.
//! - **Every act names both agents and the task.** A [`DelegationEvent`]
//!   carries the actor, the assigner and the assignee alongside the
//!   board and item.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::agent_quota_ledger::{AgentQuotaLedger, LedgerExhaustion};
use crate::mcp_policy::Quota;
use crate::role_config::RoleRegistry;
use crate::work_surfaces::{
    BoardDefinition, BoardPermissions, ItemDraft, SurfaceActor, SurfaceError, WorkSurfaces,
};

/// Audit event kind: a task was created and assigned.
pub const EVENT_TASK_CREATE: &str = "task.create";
/// Audit event kind: a task was (re)assigned.
pub const EVENT_TASK_ASSIGN: &str = "task.assign";
/// Audit event kind: an assignee claimed a task.
pub const EVENT_TASK_CLAIM: &str = "task.claim";
/// Audit event kind: a task moved between states.
pub const EVENT_TASK_TRANSITION: &str = "task.transition";
/// Audit event kind: the owner released a task's claim.
pub const EVENT_TASK_OWNER_RELEASE: &str = "task.owner_release";
/// Audit event kind: a task's claim ran out.
pub const EVENT_TASK_EXPIRE: &str = "task.expire";

/// What one agent act through this module costs the acting agent.
pub const TASK_ACT_CHARGE: Quota = Quota {
    frames: 0,
    tool_calls: 1,
    program_invocations: 0,
};

/// The longest owner-release reason, in bytes.
pub const MAX_REASON_BYTES: usize = 1024;

/// The starter delegation board's states, in order: open, in progress,
/// review, done (closing) and blocked. "Claimed" is the claim itself,
/// not a state.
pub const DELEGATION_STATES: [&str; 5] = ["open", "in-progress", "review", "done", "blocked"];

/// A starter delegation board: participants create, claim and move;
/// reviewers also close; observers read.
#[must_use]
pub fn delegation_board(name: &str) -> BoardDefinition {
    let mut board = BoardDefinition::starter(name);
    board.states = DELEGATION_STATES.map(str::to_owned).to_vec();
    board.closing_states = vec!["done".to_owned()];
    board.roles.insert(
        "reviewer".to_owned(),
        BoardPermissions {
            read: true,
            create: false,
            claim: true,
            move_items: true,
            close: true,
        },
    );
    board
}

/// Who a task is assigned to.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Assignee {
    /// Any agent holding this role.
    Role {
        /// The role.
        role: String,
    },
    /// One agent, acting under one of its roles.
    Agent {
        /// The agent key, lower-case hex.
        agent: String,
        /// The role it acts under.
        role: String,
    },
}

impl Assignee {
    /// Any agent holding `role`.
    #[must_use]
    pub fn role(role: &str) -> Self {
        Self::Role {
            role: role.to_owned(),
        }
    }

    /// The role an assignee acts under.
    #[must_use]
    pub fn acting_role(&self) -> &str {
        match self {
            Self::Role { role } | Self::Agent { role, .. } => role,
        }
    }

    fn admits(&self, actor: &SurfaceActor) -> bool {
        match (self, actor) {
            (Self::Role { role }, SurfaceActor::Agent { role: acting, .. }) => role == acting,
            (
                Self::Agent { agent, role },
                SurfaceActor::Agent {
                    agent: acting_agent,
                    role: acting,
                },
            ) => agent == acting_agent && role == acting,
            _ => false,
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Role { role } => format!("role:{role}"),
            Self::Agent { agent, role } => format!("agent:{agent}/{role}"),
        }
    }
}

/// One task's assignment: world state next to the board item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assignment {
    /// Who it is assigned to.
    pub assignee: Assignee,
    /// Who assigned it.
    pub assigned_by: SurfaceActor,
    /// When, unix seconds.
    pub assigned_at: u64,
    /// The agent that claimed it under this assignment, if any.
    pub claimant: Option<SurfaceActor>,
}

/// Why a delegation act was refused. Each maps to a stable code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DelegationError {
    /// The work surfaces refused it.
    Surface(SurfaceError),
    /// The assignee's role is not in the world's registry.
    UnknownRole,
    /// The board gives the assignee's role no claim permission, so the
    /// task could never be picked up.
    AssigneeCannotClaim,
    /// The task has no assignment.
    Unassigned,
    /// The actor is not who the task is assigned to.
    NotAssignee,
    /// The actor may not assign on this board.
    NotPermitted,
    /// The ledger offered is not the acting agent's in this world.
    LedgerMismatch,
    /// The acting agent's quota is exhausted.
    QuotaExhausted(LedgerExhaustion),
    /// Only the owner may do this.
    NotOwner,
    /// The reason is empty or too long.
    BadReason,
}

impl DelegationError {
    /// Stable machine-readable code, as journaled in an event outcome.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Surface(inner) => inner.code(),
            Self::UnknownRole => "unknown_role",
            Self::AssigneeCannotClaim => "assignee_cannot_claim",
            Self::Unassigned => "unassigned",
            Self::NotAssignee => "not_assignee",
            Self::NotPermitted => "not_permitted",
            Self::LedgerMismatch => "ledger_mismatch",
            Self::QuotaExhausted(_) => "quota_exhausted",
            Self::NotOwner => "not_owner",
            Self::BadReason => "bad_reason",
        }
    }
}

impl From<SurfaceError> for DelegationError {
    fn from(error: SurfaceError) -> Self {
        Self::Surface(error)
    }
}

/// One audited delegation act: it names the actor, the assigner, the
/// assignee and the task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DelegationEvent {
    /// One of the `EVENT_TASK_*` kinds.
    pub kind: String,
    /// The world.
    pub world: String,
    /// The board.
    pub board: String,
    /// The item, when it exists.
    pub item: Option<u64>,
    /// Who acted.
    pub actor: String,
    /// Who assigned the task, when it is assigned.
    pub assigner: Option<String>,
    /// Who it is assigned to, when it is assigned.
    pub assignee: Option<String>,
    /// Who holds (or held) the claim, when anyone does.
    pub claimant: Option<String>,
    /// What the actor was charged.
    pub charged: Quota,
    /// `executed`, or `refused:<code>`.
    pub outcome: String,
    /// The owner's reason, for an owner release.
    pub note: Option<String>,
    /// When, unix seconds.
    pub at: u64,
}

/// The acting agent's ledger, or none for the owner and the host.
pub type ActorLedger<'a> = Option<&'a mut AgentQuotaLedger>;

fn label(actor: &SurfaceActor) -> String {
    match actor {
        SurfaceActor::Owner { label } => format!("owner:{label}"),
        SurfaceActor::Agent { agent, role } => format!("agent:{agent}/{role}"),
        SurfaceActor::Host => "host".to_owned(),
    }
}

/// Check the ledger belongs to the acting agent in this world, and that
/// it can pay. Owners and the host pay nothing.
fn admit(
    world: &str,
    actor: &SurfaceActor,
    ledger: Option<&AgentQuotaLedger>,
    now: u64,
) -> Result<Quota, DelegationError> {
    let SurfaceActor::Agent { agent, .. } = actor else {
        return Ok(Quota::zero());
    };
    let ledger = ledger.ok_or(DelegationError::LedgerMismatch)?;
    if ledger.world() != world
        || crate::agent_delegation::AgentId(*ledger.agent_key()).to_hex() != *agent
    {
        return Err(DelegationError::LedgerMismatch);
    }
    let mut probe = ledger.clone();
    probe
        .charge(now, TASK_ACT_CHARGE)
        .map_err(DelegationError::QuotaExhausted)?;
    Ok(TASK_ACT_CHARGE)
}

/// Charge an admitted act. The charge was proven to fit by [`admit`] at
/// the same `now`, so it cannot fail here.
fn pay(ledger: ActorLedger<'_>, charge: Quota, now: u64) {
    if charge == Quota::zero() {
        return;
    }
    if let Some(ledger) = ledger {
        // Admission charged a copy at the same instant; this charge fits.
        let _ = ledger.charge(now, charge);
    }
}

/// One world's task assignments and their audit log.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskDelegation {
    world: String,
    assignments: BTreeMap<(String, u64), Assignment>,
    events: Vec<DelegationEvent>,
}

impl TaskDelegation {
    /// No assignments yet.
    pub fn new(world: impl Into<String>) -> Self {
        Self {
            world: world.into(),
            assignments: BTreeMap::new(),
            events: Vec::new(),
        }
    }

    /// Every delegation act and refusal.
    #[must_use]
    pub fn events(&self) -> &[DelegationEvent] {
        &self.events
    }

    /// A task's assignment, if any.
    #[must_use]
    pub fn assignment(&self, board: &str, item: u64) -> Option<&Assignment> {
        self.assignments.get(&(board.to_owned(), item))
    }

    #[allow(clippy::too_many_arguments)]
    fn record(
        &mut self,
        kind: &str,
        board: &str,
        item: Option<u64>,
        actor: &SurfaceActor,
        charged: Quota,
        outcome: String,
        note: Option<String>,
        at: u64,
    ) {
        let assignment = item.and_then(|item| self.assignments.get(&(board.to_owned(), item)));
        self.events.push(DelegationEvent {
            kind: kind.to_owned(),
            world: self.world.clone(),
            board: board.to_owned(),
            item,
            actor: label(actor),
            assigner: assignment.map(|a| label(&a.assigned_by)),
            assignee: assignment.map(|a| a.assignee.label()),
            claimant: assignment.and_then(|a| a.claimant.as_ref().map(label)),
            charged,
            outcome,
            note,
            at,
        });
    }

    /// Check an assignment fits the board: the role exists and the board
    /// lets it claim. The actor reads the board to check, so it must be
    /// able to.
    fn check_assignee(
        surfaces: &mut WorkSurfaces,
        registry: &RoleRegistry,
        actor: &SurfaceActor,
        board: &str,
        assignee: &Assignee,
        now: u64,
    ) -> Result<(), DelegationError> {
        if registry.record(assignee.acting_role()).is_none() {
            return Err(DelegationError::UnknownRole);
        }
        let view = surfaces.board(registry, actor, board, now)?;
        let claims = view
            .definition
            .roles
            .get(assignee.acting_role())
            .is_some_and(|p| p.claim);
        if !claims {
            return Err(DelegationError::AssigneeCannotClaim);
        }
        Ok(())
    }

    /// Create a task on `board` and assign it. The item is created by the
    /// actor under its own role; the assignment is a record, not a grant.
    ///
    /// # Errors
    ///
    /// The surfaces refused the create or the read, the assignee's role
    /// is unknown or cannot claim on the board, or the actor's ledger is
    /// not its own or is exhausted. Refusals are audited and charge
    /// nothing.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &mut self,
        surfaces: &mut WorkSurfaces,
        registry: &RoleRegistry,
        actor: &SurfaceActor,
        ledger: ActorLedger<'_>,
        board: &str,
        draft: ItemDraft,
        assignee: Assignee,
        now: u64,
    ) -> Result<u64, DelegationError> {
        let result = admit(&self.world, actor, ledger.as_deref(), now).and_then(|charge| {
            Self::check_assignee(surfaces, registry, actor, board, &assignee, now)?;
            let item = surfaces.create_item(registry, actor, board, draft, now)?;
            Ok((item, charge))
        });
        match result {
            Ok((item, charge)) => {
                pay(ledger, charge, now);
                self.assignments.insert(
                    (board.to_owned(), item),
                    Assignment {
                        assignee,
                        assigned_by: actor.clone(),
                        assigned_at: now,
                        claimant: None,
                    },
                );
                self.record(
                    EVENT_TASK_CREATE,
                    board,
                    Some(item),
                    actor,
                    charge,
                    "executed".to_owned(),
                    None,
                    now,
                );
                Ok(item)
            }
            Err(error) => {
                self.record(
                    EVENT_TASK_CREATE,
                    board,
                    None,
                    actor,
                    Quota::zero(),
                    format!("refused:{}", error.code()),
                    None,
                    now,
                );
                Err(error)
            }
        }
    }

    /// Reassign an existing, unclaimed task. The owner, or an agent whose
    /// role may create on the board, may assign.
    ///
    /// # Errors
    ///
    /// As [`Self::create`], plus: the actor may not assign here, or the
    /// item is held by a live claim (the claimant keeps it until it
    /// releases, its lease runs out or the owner releases it).
    #[allow(clippy::too_many_arguments)]
    pub fn assign(
        &mut self,
        surfaces: &mut WorkSurfaces,
        registry: &RoleRegistry,
        actor: &SurfaceActor,
        ledger: ActorLedger<'_>,
        board: &str,
        item: u64,
        assignee: Assignee,
        now: u64,
    ) -> Result<(), DelegationError> {
        let result = admit(&self.world, actor, ledger.as_deref(), now).and_then(|charge| {
            Self::check_assignee(surfaces, registry, actor, board, &assignee, now)?;
            let view = surfaces.board(registry, actor, board, now)?;
            if let SurfaceActor::Agent { role, .. } = actor
                && !view.definition.roles.get(role).is_some_and(|p| p.create)
            {
                return Err(DelegationError::NotPermitted);
            }
            let entry = view
                .items
                .iter()
                .find(|i| i.id == item)
                .ok_or(DelegationError::Surface(SurfaceError::UnknownItem))?;
            if let Some(claim) = entry.claim.as_ref().filter(|c| c.expires_at > now) {
                return Err(DelegationError::Surface(SurfaceError::Contended {
                    holder: claim.holder.clone(),
                }));
            }
            Ok(charge)
        });
        let outcome = match &result {
            Ok(charge) => {
                pay(ledger, *charge, now);
                self.assignments.insert(
                    (board.to_owned(), item),
                    Assignment {
                        assignee,
                        assigned_by: actor.clone(),
                        assigned_at: now,
                        claimant: None,
                    },
                );
                "executed".to_owned()
            }
            Err(error) => format!("refused:{}", error.code()),
        };
        let charged = result.as_ref().copied().unwrap_or_else(|_| Quota::zero());
        self.record(
            EVENT_TASK_ASSIGN,
            board,
            Some(item),
            actor,
            charged,
            outcome,
            None,
            now,
        );
        result.map(|_| ())
    }

    /// The assignee claims the task, atomically at the host, naming the
    /// item version it saw. Only an agent the assignment names may claim,
    /// as itself under its own role.
    ///
    /// # Errors
    ///
    /// The task is unassigned or not assigned to the actor, the ledger is
    /// not the actor's or is exhausted, or the surfaces refused the claim
    /// (stale, contended, closed, not permitted). Refusals are audited
    /// and charge nothing.
    #[allow(clippy::too_many_arguments)]
    pub fn claim(
        &mut self,
        surfaces: &mut WorkSurfaces,
        registry: &RoleRegistry,
        actor: &SurfaceActor,
        ledger: ActorLedger<'_>,
        board: &str,
        item: u64,
        seen_version: u64,
        now: u64,
    ) -> Result<(), DelegationError> {
        let key = (board.to_owned(), item);
        let result = self
            .assignments
            .get(&key)
            .ok_or(DelegationError::Unassigned)
            .and_then(|a| {
                if a.assignee.admits(actor) {
                    Ok(())
                } else {
                    Err(DelegationError::NotAssignee)
                }
            })
            .and_then(|()| admit(&self.world, actor, ledger.as_deref(), now))
            .and_then(|charge| {
                surfaces.claim(registry, actor, board, item, seen_version, now)?;
                Ok(charge)
            });
        let charged = result.as_ref().copied().unwrap_or_else(|_| Quota::zero());
        if let Ok(charge) = &result {
            pay(ledger, *charge, now);
            if let Some(assignment) = self.assignments.get_mut(&key) {
                assignment.claimant = Some(actor.clone());
            }
        }
        let outcome = match &result {
            Ok(_) => "executed".to_owned(),
            Err(error) => format!("refused:{}", error.code()),
        };
        self.record(
            EVENT_TASK_CLAIM,
            board,
            Some(item),
            actor,
            charged,
            outcome,
            None,
            now,
        );
        result.map(|_| ())
    }

    /// Move a task between states (open, in progress, review, done,
    /// blocked) under the board's rules: while a claim holds it, only the
    /// claimant or the owner may move it. The actor pays.
    ///
    /// # Errors
    ///
    /// The ledger is not the actor's or is exhausted, or the surfaces
    /// refused the move. Refusals are audited and charge nothing.
    #[allow(clippy::too_many_arguments)]
    pub fn transition(
        &mut self,
        surfaces: &mut WorkSurfaces,
        registry: &RoleRegistry,
        actor: &SurfaceActor,
        ledger: ActorLedger<'_>,
        board: &str,
        item: u64,
        to: &str,
        seen_version: u64,
        now: u64,
    ) -> Result<u64, DelegationError> {
        let result = admit(&self.world, actor, ledger.as_deref(), now).and_then(|charge| {
            let version =
                surfaces.move_item(registry, actor, board, item, to, seen_version, now)?;
            Ok((version, charge))
        });
        let charged = result.as_ref().map_or_else(|_| Quota::zero(), |(_, c)| *c);
        if let Ok((_, charge)) = &result {
            pay(ledger, *charge, now);
        }
        let outcome = match &result {
            Ok(_) => "executed".to_owned(),
            Err(error) => format!("refused:{}", error.code()),
        };
        self.record(
            EVENT_TASK_TRANSITION,
            board,
            Some(item),
            actor,
            charged,
            outcome,
            Some(to.to_owned()),
            now,
        );
        result.map(|(version, _)| version)
    }

    /// The owner releases a parked or quota-exhausted claimant's task,
    /// with a reason. The assignment stays; the claimant is cleared.
    ///
    /// # Errors
    ///
    /// The actor is not the owner, the reason is empty or too long, or
    /// the surfaces refused (no live claim).
    pub fn owner_release(
        &mut self,
        surfaces: &mut WorkSurfaces,
        registry: &RoleRegistry,
        owner: &SurfaceActor,
        board: &str,
        item: u64,
        reason: &str,
        now: u64,
    ) -> Result<(), DelegationError> {
        let result = if !matches!(owner, SurfaceActor::Owner { .. }) {
            Err(DelegationError::NotOwner)
        } else if reason.trim().is_empty() || reason.len() > MAX_REASON_BYTES {
            Err(DelegationError::BadReason)
        } else {
            surfaces
                .release(registry, owner, board, item, now)
                .map_err(DelegationError::Surface)
        };
        let outcome = match &result {
            Ok(()) => "executed".to_owned(),
            Err(error) => format!("refused:{}", error.code()),
        };
        // Record before clearing, so the event names the claimant.
        self.record(
            EVENT_TASK_OWNER_RELEASE,
            board,
            Some(item),
            owner,
            Quota::zero(),
            outcome,
            Some(reason.to_owned()),
            now,
        );
        if result.is_ok()
            && let Some(assignment) = self.assignments.get_mut(&(board.to_owned(), item))
        {
            assignment.claimant = None;
        }
        result
    }

    /// Run the surfaces' lease expiry and record, for each assigned task
    /// whose claim ran out, an event naming the assigner, the assignee and
    /// the claimant that lost it. Returns the (board, item) pairs expired.
    pub fn expire(&mut self, surfaces: &mut WorkSurfaces, now: u64) -> Vec<(String, u64)> {
        let expired = surfaces.expire_claims(now);
        for (board, item) in &expired {
            if self.assignments.contains_key(&(board.clone(), *item)) {
                self.record(
                    EVENT_TASK_EXPIRE,
                    board,
                    Some(*item),
                    &SurfaceActor::Host,
                    Quota::zero(),
                    "executed".to_owned(),
                    None,
                    now,
                );
                if let Some(assignment) = self.assignments.get_mut(&(board.clone(), *item)) {
                    assignment.claimant = None;
                }
            }
        }
        expired
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::{Arc, Barrier, Mutex};

    use super::*;
    use crate::agent_delegation::AgentId;
    use crate::role_config::QuotaClass;

    const WORLD: &str = "studio";
    const T0: u64 = 1_800_000_000;
    const BOARD: &str = "work";

    fn agent(byte: u8) -> AgentId {
        AgentId([byte; 32])
    }

    /// Starters plus "reviewer" (a participant copy). Agents 1–3 are
    /// participants, 4 a reviewer, 5 an observer.
    fn registry() -> RoleRegistry {
        let mut registry = RoleRegistry::new(WORLD);
        registry.install_starters("owner", T0);
        let mut reviewer = registry.record("participant").unwrap().definition.clone();
        reviewer.name = "reviewer".to_owned();
        assert!(registry.upsert(reviewer, "owner", T0).decision.is_ok());
        for (byte, role) in [
            (1, "participant"),
            (2, "participant"),
            (3, "participant"),
            (4, "reviewer"),
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

    fn setup() -> (RoleRegistry, WorkSurfaces, TaskDelegation) {
        let registry = registry();
        let mut surfaces = WorkSurfaces::new(WORLD);
        surfaces
            .define_board(&owner(), delegation_board(BOARD), T0)
            .unwrap();
        (registry, surfaces, TaskDelegation::new(WORLD))
    }

    fn draft(title: &str) -> ItemDraft {
        ItemDraft {
            title: title.to_owned(),
            ..ItemDraft::default()
        }
    }

    fn version(surfaces: &mut WorkSurfaces, registry: &RoleRegistry, item: u64) -> u64 {
        surfaces
            .board(registry, &owner(), BOARD, T0)
            .unwrap()
            .items
            .iter()
            .find(|i| i.id == item)
            .unwrap()
            .version
    }

    /// Acceptance 1: an agent creates a task and assigns it to a role;
    /// assignment is world state and transfers no capability.
    #[test]
    fn create_assigns_to_a_role_as_world_state() {
        let (registry, mut surfaces, mut tasks) = setup();
        let grants_before: Vec<_> = registry.list_grants().into_iter().cloned().collect();
        let mut l1 = ledger(1, "participant", 10);

        let item = tasks
            .create(
                &mut surfaces,
                &registry,
                &actor(1, "participant"),
                Some(&mut l1),
                BOARD,
                draft("index the archive"),
                Assignee::role("reviewer"),
                T0,
            )
            .unwrap();
        let assignment = tasks.assignment(BOARD, item).unwrap();
        assert_eq!(assignment.assignee, Assignee::role("reviewer"));
        assert_eq!(assignment.assigned_by, actor(1, "participant"));
        assert_eq!(l1.spend_at(T0), TASK_ACT_CHARGE);

        // The assigner gains nothing: it cannot claim as the reviewer
        // role it does not hold, nor as its own role.
        let v = version(&mut surfaces, &registry, item);
        assert_eq!(
            tasks.claim(
                &mut surfaces,
                &registry,
                &actor(1, "participant"),
                Some(&mut l1),
                BOARD,
                item,
                v,
                T0 + 1
            ),
            Err(DelegationError::NotAssignee)
        );
        assert_eq!(
            tasks.claim(
                &mut surfaces,
                &registry,
                &actor(1, "reviewer"),
                Some(&mut l1),
                BOARD,
                item,
                v,
                T0 + 1
            ),
            Err(DelegationError::Surface(SurfaceError::UngrantedRole))
        );
        let grants_after: Vec<_> = registry.list_grants().into_iter().cloned().collect();
        assert_eq!(grants_before, grants_after);

        // A role that cannot claim on the board is refused up front.
        assert_eq!(
            tasks.create(
                &mut surfaces,
                &registry,
                &actor(1, "participant"),
                Some(&mut l1),
                BOARD,
                draft("stranded"),
                Assignee::role("observer"),
                T0 + 2,
            ),
            Err(DelegationError::AssigneeCannotClaim)
        );
        assert_eq!(
            tasks.create(
                &mut surfaces,
                &registry,
                &actor(1, "participant"),
                Some(&mut l1),
                BOARD,
                draft("nobody"),
                Assignee::role("ghost"),
                T0 + 2,
            ),
            Err(DelegationError::UnknownRole)
        );
        // Refusals charged nothing.
        assert_eq!(l1.spend_at(T0 + 2), TASK_ACT_CHARGE);
        let create = &tasks.events()[0];
        assert_eq!(
            create.assigner.as_deref(),
            Some(label(&actor(1, "participant")).as_str())
        );
        assert_eq!(create.assignee.as_deref(), Some("role:reviewer"));
    }

    /// Acceptance 2: concurrent claims resolve to one claimant; every
    /// other is refused and audited.
    #[test]
    fn concurrent_claims_resolve_to_one_claimant() {
        let (registry, mut surfaces, mut tasks) = setup();
        let item = tasks
            .create(
                &mut surfaces,
                &registry,
                &owner(),
                None,
                BOARD,
                draft("one winner"),
                Assignee::role("participant"),
                T0,
            )
            .unwrap();
        let seen = version(&mut surfaces, &registry, item);
        let shared = Arc::new(Mutex::new((surfaces, tasks)));
        let registry = Arc::new(registry);
        let barrier = Arc::new(Barrier::new(3));
        let handles: Vec<_> = (1..=3)
            .map(|byte| {
                let shared = Arc::clone(&shared);
                let registry = Arc::clone(&registry);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let mut own = ledger(byte, "participant", 10);
                    barrier.wait();
                    let mut guard = shared.lock().unwrap();
                    let (surfaces, tasks) = &mut *guard;
                    tasks
                        .claim(
                            surfaces,
                            &registry,
                            &actor(byte, "participant"),
                            Some(&mut own),
                            BOARD,
                            item,
                            seen,
                            T0 + 1,
                        )
                        .is_ok()
                })
            })
            .collect();
        let wins = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|won| *won)
            .count();
        assert_eq!(wins, 1);
        let guard = shared.lock().unwrap();
        let claims: Vec<&DelegationEvent> = guard
            .1
            .events()
            .iter()
            .filter(|e| e.kind == EVENT_TASK_CLAIM)
            .collect();
        assert_eq!(claims.len(), 3);
        assert_eq!(claims.iter().filter(|e| e.outcome == "executed").count(), 1);
        for refused in claims.iter().filter(|e| e.outcome != "executed") {
            assert_eq!(refused.charged, Quota::zero());
            assert!(refused.outcome.starts_with("refused:"));
        }
        assert!(guard.1.assignment(BOARD, item).unwrap().claimant.is_some());
    }

    /// Acceptance 3 and 4: an exhausted claimant cannot act, and its task
    /// is released by the owner or by timeout, never silently; the acting
    /// agent is the one charged.
    #[test]
    fn exhausted_claimant_is_released_by_owner_or_timeout() {
        let (registry, mut surfaces, mut tasks) = setup();
        let mut l2 = ledger(2, "participant", 2);
        let mut l3 = ledger(3, "participant", 10);
        let a = tasks
            .create(
                &mut surfaces,
                &registry,
                &owner(),
                None,
                BOARD,
                draft("first"),
                Assignee::Agent {
                    agent: agent(2).to_hex(),
                    role: "participant".to_owned(),
                },
                T0,
            )
            .unwrap();
        let b = tasks
            .create(
                &mut surfaces,
                &registry,
                &owner(),
                None,
                BOARD,
                draft("second"),
                Assignee::role("participant"),
                T0,
            )
            .unwrap();

        // Agent 3 is not the named assignee of task a.
        let va = version(&mut surfaces, &registry, a);
        assert_eq!(
            tasks.claim(
                &mut surfaces,
                &registry,
                &actor(3, "participant"),
                Some(&mut l3),
                BOARD,
                a,
                va,
                T0
            ),
            Err(DelegationError::NotAssignee)
        );
        // Agent 2 may not pay with agent 3's ledger.
        assert_eq!(
            tasks.claim(
                &mut surfaces,
                &registry,
                &actor(2, "participant"),
                Some(&mut l3),
                BOARD,
                a,
                va,
                T0
            ),
            Err(DelegationError::LedgerMismatch)
        );
        assert_eq!(l3.spend_at(T0), Quota::zero());

        // Agent 2 claims both tasks and runs dry.
        for item in [a, b] {
            let v = version(&mut surfaces, &registry, item);
            tasks
                .claim(
                    &mut surfaces,
                    &registry,
                    &actor(2, "participant"),
                    Some(&mut l2),
                    BOARD,
                    item,
                    v,
                    T0 + 1,
                )
                .unwrap();
        }
        assert_eq!(l2.spend_at(T0 + 1).tool_calls, 2);
        let vb = version(&mut surfaces, &registry, b);
        assert!(matches!(
            tasks.transition(
                &mut surfaces,
                &registry,
                &actor(2, "participant"),
                Some(&mut l2),
                BOARD,
                b,
                "in-progress",
                vb,
                T0 + 2
            ),
            Err(DelegationError::QuotaExhausted(_))
        ));
        // Nobody else may move it while the claim holds.
        assert!(
            tasks
                .transition(
                    &mut surfaces,
                    &registry,
                    &actor(3, "participant"),
                    Some(&mut l3),
                    BOARD,
                    b,
                    "in-progress",
                    vb,
                    T0 + 2
                )
                .is_err()
        );

        // The owner releases task a with a reason; a reason is required.
        assert_eq!(
            tasks.owner_release(&mut surfaces, &registry, &owner(), BOARD, a, " ", T0 + 3),
            Err(DelegationError::BadReason)
        );
        assert_eq!(
            tasks.owner_release(
                &mut surfaces,
                &registry,
                &actor(3, "participant"),
                BOARD,
                a,
                "mine now",
                T0 + 3
            ),
            Err(DelegationError::NotOwner)
        );
        tasks
            .owner_release(
                &mut surfaces,
                &registry,
                &owner(),
                BOARD,
                a,
                "claimant out of quota",
                T0 + 3,
            )
            .unwrap();
        let release = tasks.events().last().unwrap();
        assert_eq!(release.kind, EVENT_TASK_OWNER_RELEASE);
        assert_eq!(
            release.claimant.as_deref(),
            Some(label(&actor(2, "participant")).as_str())
        );
        assert_eq!(release.note.as_deref(), Some("claimant out of quota"));
        assert!(tasks.assignment(BOARD, a).unwrap().claimant.is_none());

        // Task b times out.
        let lease = delegation_board(BOARD).claim_lease_secs;
        let expired = tasks.expire(&mut surfaces, T0 + 1 + lease);
        assert_eq!(expired, [(BOARD.to_owned(), b)]);
        let expiry = tasks.events().last().unwrap();
        assert_eq!(expiry.kind, EVENT_TASK_EXPIRE);
        assert_eq!(
            expiry.claimant.as_deref(),
            Some(label(&actor(2, "participant")).as_str())
        );
        assert_eq!(expiry.assignee.as_deref(), Some("role:participant"));

        // Agent 3 picks task b up and pays for it itself.
        let vb = version(&mut surfaces, &registry, b);
        tasks
            .claim(
                &mut surfaces,
                &registry,
                &actor(3, "participant"),
                Some(&mut l3),
                BOARD,
                b,
                vb,
                T0 + 2 + lease,
            )
            .unwrap();
        assert_eq!(l3.spend_at(T0 + 2 + lease), TASK_ACT_CHARGE);
        assert_eq!(l2.spend_at(T0 + 2 + lease).tool_calls, 2);
    }

    /// Conformance: a full lifecycle (open, claimed, in progress, review,
    /// done) under assignment widens nobody's ceiling, and every event
    /// names both agents and the task.
    #[test]
    fn lifecycle_widens_no_ceiling_and_names_both_agents() {
        let (registry, mut surfaces, mut tasks) = setup();
        let ceilings_before: Vec<_> = registry
            .records()
            .into_iter()
            .map(|r| (r.definition.name.clone(), r.definition.ceiling))
            .collect();
        let mut l1 = ledger(1, "participant", 50);
        let mut l2 = ledger(2, "participant", 50);
        let mut l4 = ledger(4, "reviewer", 50);
        let worker = actor(2, "participant");
        let reviewer = actor(4, "reviewer");

        let item = tasks
            .create(
                &mut surfaces,
                &registry,
                &actor(1, "participant"),
                Some(&mut l1),
                BOARD,
                draft("write the summary"),
                Assignee::role("participant"),
                T0,
            )
            .unwrap();
        let v = version(&mut surfaces, &registry, item);
        tasks
            .claim(
                &mut surfaces,
                &registry,
                &worker,
                Some(&mut l2),
                BOARD,
                item,
                v,
                T0 + 1,
            )
            .unwrap();
        let v = version(&mut surfaces, &registry, item);
        let v = tasks
            .transition(
                &mut surfaces,
                &registry,
                &worker,
                Some(&mut l2),
                BOARD,
                item,
                "in-progress",
                v,
                T0 + 2,
            )
            .unwrap();
        let v = tasks
            .transition(
                &mut surfaces,
                &registry,
                &worker,
                Some(&mut l2),
                BOARD,
                item,
                "review",
                v,
                T0 + 3,
            )
            .unwrap();
        // The worker may not close it; the reviewer's handoff is a
        // reassignment, not a grant.
        assert!(
            tasks
                .transition(
                    &mut surfaces,
                    &registry,
                    &worker,
                    Some(&mut l2),
                    BOARD,
                    item,
                    "done",
                    v,
                    T0 + 4
                )
                .is_err()
        );
        tasks
            .owner_release(
                &mut surfaces,
                &registry,
                &owner(),
                BOARD,
                item,
                "hand to review",
                T0 + 4,
            )
            .unwrap();
        tasks
            .assign(
                &mut surfaces,
                &registry,
                &actor(1, "participant"),
                Some(&mut l1),
                BOARD,
                item,
                Assignee::role("reviewer"),
                T0 + 5,
            )
            .unwrap();
        let v = version(&mut surfaces, &registry, item);
        tasks
            .claim(
                &mut surfaces,
                &registry,
                &reviewer,
                Some(&mut l4),
                BOARD,
                item,
                v,
                T0 + 6,
            )
            .unwrap();
        let v = version(&mut surfaces, &registry, item);
        tasks
            .transition(
                &mut surfaces,
                &registry,
                &reviewer,
                Some(&mut l4),
                BOARD,
                item,
                "done",
                v,
                T0 + 7,
            )
            .unwrap();

        let ceilings_after: Vec<_> = registry
            .records()
            .into_iter()
            .map(|r| (r.definition.name.clone(), r.definition.ceiling))
            .collect();
        assert_eq!(ceilings_before, ceilings_after);
        // Each actor paid for its own acts only.
        assert_eq!(l1.spend_at(T0 + 7).tool_calls, 2);
        assert_eq!(l2.spend_at(T0 + 7).tool_calls, 3);
        assert_eq!(l4.spend_at(T0 + 7).tool_calls, 2);
        for event in tasks.events() {
            assert_eq!(event.item, Some(item), "{event:?}");
            assert!(event.assigner.is_some(), "{event:?}");
            assert!(event.assignee.is_some(), "{event:?}");
        }
        let done = tasks.events().last().unwrap();
        assert_eq!(done.actor, label(&reviewer));
        assert_eq!(
            done.assigner.as_deref(),
            Some(label(&actor(1, "participant")).as_str())
        );
        assert_eq!(done.claimant.as_deref(), Some(label(&reviewer).as_str()));
    }
}
