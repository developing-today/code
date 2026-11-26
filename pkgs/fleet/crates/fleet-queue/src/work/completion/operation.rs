//! Completion operation and effect state machines — REL-07
//! (developing-today/code#9517; design parent #9209, program #9196).
//!
//! Design record:
//! `doc/2026-11-25T22-26-00Z_design_rel07_completion_state_machine/`.
//!
//! When an agent finishes, one completion should merge the named PR,
//! close the named issues and close the queue item, and say exactly
//! which of those happened. If an answer is lost anywhere in between,
//! running the same completion again must read back what already
//! happened and finish from there, without merging twice, closing
//! the wrong thing, or waiting for a webhook.
//!
//! This is the pure model of that record (§3, §4, §5, §6), like its
//! `work` siblings: it holds operations in memory and does no I/O.
//! The remote side (GitHub) is reached only by a caller that feeds
//! answers and readbacks in as values; the persisted operation store
//! (REL-03/REL-04), the relay (REL-05), the readback calls (REL-06)
//! and the stored `completing` queue status are not part of this
//! slice and stay with their owners.
//!
//! Rules the model enforces:
//!
//! * **Explicit, linked targets only.** Every remote target is named
//!   in the request and must also be an active link of the work with
//!   a matching kind (`pr_for` for a merge, `implements` for a
//!   close), unless an owner-confirmed override names it. Nothing is
//!   added by inference.
//! * **Effect status moves only along the §4 table.** `confirmed`
//!   never moves again; `conflict` and `failed_terminal` move only by
//!   a new operation.
//! * **Operation status is derived**, never stored or set by hand.
//! * **Readback before resend.** Recovery reads back every effect
//!   whose outcome is not proven, and never plans a resend of a
//!   merge or close whose outcome is not proven absent.
//! * **Finalize** needs every required effect confirmed, an allowed
//!   final refresh, the work still completing under this operation,
//!   no newer cancel, and the existing `/complete` gates. It does not
//!   depend on the initiating worker's lease.

use std::collections::{BTreeMap, BTreeSet};

use super::{ActivityClass, CompletionReceipt};

/// How a PR is merged (§5.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MergeMethod {
    /// A merge commit.
    Merge,
    /// A squash commit.
    Squash,
    /// Rebased commits.
    Rebase,
}

impl MergeMethod {
    /// Stable spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            MergeMethod::Merge => "merge",
            MergeMethod::Squash => "squash",
            MergeMethod::Rebase => "rebase",
        }
    }
}

/// The link kinds a completion target must match (CLI-12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LinkKind {
    /// The PR that carries the work.
    PrFor,
    /// An issue the work implements.
    Implements,
    /// Any other relation; never authorizes a completion effect.
    Other,
}

/// An active link of the work being completed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveLink {
    /// The link's kind.
    pub kind: LinkKind,
    /// `owner/repo` of the linked item.
    pub repo: String,
    /// PR or issue number.
    pub number: u64,
}

/// A remote effect target named in a completion request (§3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectTarget {
    /// Merge a PR at an expected head with a method.
    MergePr {
        /// `owner/repo`.
        repo: String,
        /// PR number.
        number: u64,
        /// The head SHA the merge must apply to.
        expected_head: String,
        /// The merge method.
        method: MergeMethod,
    },
    /// Close an issue.
    CloseIssue {
        /// `owner/repo`.
        repo: String,
        /// Issue number.
        number: u64,
    },
}

impl EffectTarget {
    /// The link kind this target must match.
    pub fn required_link(&self) -> LinkKind {
        match self {
            EffectTarget::MergePr { .. } => LinkKind::PrFor,
            EffectTarget::CloseIssue { .. } => LinkKind::Implements,
        }
    }

    /// `(repo, number)` of the target.
    pub fn reference(&self) -> (&str, u64) {
        match self {
            EffectTarget::MergePr { repo, number, .. }
            | EffectTarget::CloseIssue { repo, number } => (repo.as_str(), *number),
        }
    }

    /// `owner/repo#N`, for messages and overrides.
    pub fn display_ref(&self) -> String {
        let (repo, number) = self.reference();
        format!("{repo}#{number}")
    }

    fn is_merge(&self) -> bool {
        matches!(self, EffectTarget::MergePr { .. })
    }
}

/// Status of one effect (§4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EffectStatus {
    /// Written at prepare; nothing sent.
    Planned,
    /// The relay took it; the request may have left.
    Sending,
    /// Accepted asynchronously (202 or merge queue); not yet applied.
    Pending,
    /// The request may have applied; readback decides.
    Unknown,
    /// Readback shows the intended result. Never moves again.
    Confirmed,
    /// A precondition or another result stopped it.
    Conflict,
    /// A terminal refusal (401/403/404/422).
    FailedTerminal,
    /// An optional effect that is no longer wanted.
    Skipped,
}

impl EffectStatus {
    /// Stable spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            EffectStatus::Planned => "planned",
            EffectStatus::Sending => "sending",
            EffectStatus::Pending => "pending",
            EffectStatus::Unknown => "unknown",
            EffectStatus::Confirmed => "confirmed",
            EffectStatus::Conflict => "conflict",
            EffectStatus::FailedTerminal => "failed_terminal",
            EffectStatus::Skipped => "skipped",
        }
    }

    /// Outcome not yet proven: recovery must read it back first.
    pub fn needs_readback(self) -> bool {
        matches!(
            self,
            EffectStatus::Sending | EffectStatus::Pending | EffectStatus::Unknown
        )
    }

    fn is_failure(self) -> bool {
        matches!(self, EffectStatus::Conflict | EffectStatus::FailedTerminal)
    }
}

/// What a send returned (§4, §5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendAnswer {
    /// Applied, and the readback that follows matched.
    AppliedAndReadBack,
    /// 202 accepted or merge queued.
    Accepted,
    /// Timeout, reset or 5xx after the request left.
    Lost,
    /// 409 head mismatch, or a review or check gate failed.
    Conflict,
    /// 401/403/404/422.
    Terminal,
    /// Refused before sending (rate limit); stays in the operation.
    RefusedBeforeSend,
}

/// What a readback showed, against the effect's intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readback {
    /// The intended result is in place.
    Matches,
    /// Proven not applied.
    ProvenAbsent,
    /// A different result is in place.
    Different,
    /// Still queued (merge queue).
    StillPending,
}

/// An input to the effect state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectEvent {
    /// The relay takes the effect.
    Take,
    /// An optional effect is no longer wanted.
    SkipOptional,
    /// The send returned.
    Answer(SendAnswer),
    /// A readback was taken.
    Readback(Readback),
}

/// A refused completion step, with a stable code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionError {
    /// The principal is not authenticated.
    Unauthenticated,
    /// The worker's fence is older than the work's current fence.
    StaleFence {
        /// The worker's fence.
        worker: u64,
        /// The work's current fence.
        current: u64,
    },
    /// An open hard blocker stops prepare.
    HardBlockerOpen,
    /// A request named no effect.
    NoEffects,
    /// A named target is not an active link of matching kind.
    RefNotLinked(String),
    /// The same key arrived with a different request digest.
    KeyConflict(String),
    /// The work already has an open operation under another key.
    OperationOpen(String),
    /// No operation with that key.
    UnknownOperation(String),
    /// No effect at that index.
    UnknownEffect(usize),
    /// The operation is past prepare; effects can no longer be added.
    OperationSealed,
    /// A settled effect cannot move.
    EffectSettled(EffectStatus),
    /// The event is not a §4 transition from this status.
    TransitionRefused(EffectStatus, EffectEvent),
    /// A required effect cannot be skipped.
    SkipRequired,
    /// The final refresh held the merge (REL-09).
    RefreshHeld(Vec<ActivityClass>),
    /// Finalize conditions (§5.6) are not met.
    FinalizeRefused(String),
}

impl CompletionError {
    /// Stable machine code.
    pub fn code(&self) -> &'static str {
        match self {
            CompletionError::Unauthenticated => "completion_unauthenticated",
            CompletionError::StaleFence { .. } => "completion_stale_fence",
            CompletionError::HardBlockerOpen => "completion_hard_blocker_open",
            CompletionError::NoEffects => "completion_no_effects",
            CompletionError::RefNotLinked(_) => "completion_ref_not_linked",
            CompletionError::KeyConflict(_) => "completion_key_conflict",
            CompletionError::OperationOpen(_) => "completion_operation_open",
            CompletionError::UnknownOperation(_) => "completion_unknown_operation",
            CompletionError::UnknownEffect(_) => "completion_unknown_effect",
            CompletionError::OperationSealed => "completion_operation_sealed",
            CompletionError::EffectSettled(_) => "completion_effect_settled",
            CompletionError::TransitionRefused(..) => "completion_transition_refused",
            CompletionError::SkipRequired => "completion_skip_required",
            CompletionError::RefreshHeld(_) => "completion_refresh_held",
            CompletionError::FinalizeRefused(_) => "completion_finalize_refused",
        }
    }
}

impl std::fmt::Display for CompletionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let detail = match self {
            CompletionError::Unauthenticated => "the principal is not authenticated".to_string(),
            CompletionError::StaleFence { worker, current } => {
                format!("worker fence {worker} is older than the current fence {current}")
            }
            CompletionError::HardBlockerOpen => "an open hard blocker stops completion".to_string(),
            CompletionError::NoEffects => "a completion must name at least one effect".to_string(),
            CompletionError::RefNotLinked(target) => {
                format!("{target} is not an active link of matching kind")
            }
            CompletionError::KeyConflict(key) => {
                format!("key {key} was used with a different request")
            }
            CompletionError::OperationOpen(key) => {
                format!("the work already has open operation {key}")
            }
            CompletionError::UnknownOperation(key) => format!("no completion operation {key}"),
            CompletionError::UnknownEffect(index) => format!("no effect at index {index}"),
            CompletionError::OperationSealed => "effects are fixed at prepare".to_string(),
            CompletionError::EffectSettled(status) => {
                format!("a {} effect never moves", status.as_str())
            }
            CompletionError::TransitionRefused(status, event) => {
                format!("{event:?} is not a transition from {}", status.as_str())
            }
            CompletionError::SkipRequired => "a required effect cannot be skipped".to_string(),
            CompletionError::RefreshHeld(classes) => {
                format!("late activity holds the merge: {classes:?}")
            }
            CompletionError::FinalizeRefused(reason) => reason.clone(),
        };
        write!(f, "{}: {detail}", self.code())
    }
}

impl std::error::Error for CompletionError {}

/// The §4 effect transition table.
pub fn effect_transition(
    from: EffectStatus,
    event: EffectEvent,
) -> Result<EffectStatus, CompletionError> {
    use EffectEvent as E;
    use EffectStatus as S;
    let to = match (from, event) {
        (S::Planned, E::Take) => S::Sending,
        (S::Planned, E::SkipOptional) => S::Skipped,
        (S::Sending, E::Answer(answer)) => match answer {
            SendAnswer::AppliedAndReadBack => S::Confirmed,
            SendAnswer::Accepted => S::Pending,
            SendAnswer::Lost => S::Unknown,
            SendAnswer::Conflict => S::Conflict,
            SendAnswer::Terminal => S::FailedTerminal,
            SendAnswer::RefusedBeforeSend => S::Planned,
        },
        // Recovery after a crash mid-send reads back first (§6.2), so
        // a sending or unknown effect resolves the same way.
        (S::Sending | S::Unknown, E::Readback(readback)) => match readback {
            Readback::Matches => S::Confirmed,
            Readback::ProvenAbsent => S::Planned,
            Readback::Different => S::Conflict,
            // The request reached a merge queue: wait on it.
            Readback::StillPending => S::Pending,
        },
        (S::Pending, E::Readback(readback)) => match readback {
            Readback::Matches => S::Confirmed,
            Readback::StillPending => S::Pending,
            // A queued merge that dropped out or closed unmerged.
            Readback::ProvenAbsent | Readback::Different => S::FailedTerminal,
        },
        // A duplicate webhook or a second readback of a confirmed
        // effect changes nothing.
        (S::Confirmed, E::Readback(Readback::Matches)) => S::Confirmed,
        (S::Confirmed | S::Conflict | S::FailedTerminal | S::Skipped, _) => {
            return Err(CompletionError::EffectSettled(from));
        }
        _ => return Err(CompletionError::TransitionRefused(from, event)),
    };
    Ok(to)
}

/// A PR as read back from the provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrReadback {
    /// The provider's `merged` record.
    pub merged: bool,
    /// The PR is closed (merged or not).
    pub closed: bool,
    /// Whether the PR sits in a merge queue.
    pub queued: bool,
    /// The PR's head SHA.
    pub head_sha: String,
    /// The commit the merge produced, when merged.
    pub merge_commit_sha: Option<String>,
}

/// Reads back a merge (§5.4). Only the provider's `merged` record
/// plus the returned commit confirms, for every method: a squash or
/// rebase is verified from the PR record, never from ancestry, and a
/// closed PR without `merged` is never merged.
pub fn read_back_merge(expected_head: &str, pr: &PrReadback) -> Readback {
    if pr.merged {
        if pr.head_sha == expected_head && pr.merge_commit_sha.is_some() {
            Readback::Matches
        } else {
            Readback::Different
        }
    } else if pr.queued {
        Readback::StillPending
    } else if pr.closed {
        Readback::Different
    } else {
        Readback::ProvenAbsent
    }
}

/// Reads back an issue close.
pub fn read_back_close(issue_closed: bool) -> Readback {
    if issue_closed {
        Readback::Matches
    } else {
        Readback::ProvenAbsent
    }
}

/// One effect of a request: its target and whether policy requires it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestedEffect {
    /// The named target.
    pub target: EffectTarget,
    /// Required effects must confirm before finalize.
    pub required: bool,
}

/// A completion request (§3, §5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionRequest {
    /// Principal-scoped idempotency key (REL-03).
    pub key: String,
    /// Digest of the request body (REL-03).
    pub digest: String,
    /// The work being completed.
    pub work: String,
    /// The named effects, in request order.
    pub effects: Vec<RequestedEffect>,
}

/// What prepare checks against (§5.1).
#[derive(Debug, Clone, Default)]
pub struct PrepareContext {
    /// The principal is authenticated.
    pub authenticated: bool,
    /// The fence the caller holds.
    pub worker_fence: u64,
    /// The work's current fence.
    pub current_fence: u64,
    /// An open hard blocker exists.
    pub hard_blocker_open: bool,
    /// Active links of the work.
    pub links: Vec<ActiveLink>,
    /// `owner/repo#N` targets an owner confirmed without a link.
    pub overrides: Vec<String>,
}

/// One effect of an operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effect {
    /// The named target.
    pub target: EffectTarget,
    /// Required by policy.
    pub required: bool,
    /// Current status.
    pub status: EffectStatus,
    /// Every status the effect has held, oldest first.
    pub history: Vec<EffectStatus>,
}

/// Operation status (§4), derived from its effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationStatus {
    /// Prepared; nothing sent.
    Prepared,
    /// Effects in flight or still to send.
    InProgress,
    /// A required effect is pending or unknown; none failed.
    Pending,
    /// Some required effect confirmed and some failed or conflicted.
    Partial,
    /// Stopped before any required effect landed.
    Conflict,
    /// Finalize committed.
    Completed,
}

impl OperationStatus {
    /// Stable spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            OperationStatus::Prepared => "prepared",
            OperationStatus::InProgress => "in_progress",
            OperationStatus::Pending => "pending",
            OperationStatus::Partial => "partial",
            OperationStatus::Conflict => "conflict",
            OperationStatus::Completed => "completed",
        }
    }
}

/// The final refresh's verdict, kept on the operation (§5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshState {
    /// Not run yet.
    NotRun,
    /// REL-09 allowed the merge.
    Allowed,
    /// REL-09 held the merge; late invalidating classes.
    Held(Vec<ActivityClass>),
}

/// A completion operation: one per work while it is completing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionOperation {
    /// The request key.
    pub key: String,
    /// The request digest.
    pub digest: String,
    /// The work.
    pub work: String,
    /// The fence held at prepare.
    pub fence: u64,
    /// Effects in request order.
    pub effects: Vec<Effect>,
    /// The final refresh.
    pub refresh: RefreshState,
    /// Finalize committed.
    pub finalized: bool,
    /// Late answers from a stale worker, kept as evidence only.
    pub evidence: Vec<String>,
}

impl CompletionOperation {
    /// The derived operation status (§4).
    pub fn status(&self) -> OperationStatus {
        if self.finalized {
            return OperationStatus::Completed;
        }
        let required: Vec<EffectStatus> = self
            .effects
            .iter()
            .filter(|e| e.required)
            .map(|e| e.status)
            .collect();
        let any_confirmed = required.iter().any(|s| *s == EffectStatus::Confirmed);
        let any_failed = required.iter().any(|s| s.is_failure());
        if matches!(self.refresh, RefreshState::Held(_)) && !any_confirmed {
            return OperationStatus::Conflict;
        }
        if any_confirmed && any_failed {
            return OperationStatus::Partial;
        }
        if self
            .effects
            .iter()
            .any(|e| e.status == EffectStatus::Sending)
        {
            return OperationStatus::InProgress;
        }
        if any_failed {
            return OperationStatus::Conflict;
        }
        if required
            .iter()
            .any(|s| matches!(s, EffectStatus::Pending | EffectStatus::Unknown))
        {
            return OperationStatus::Pending;
        }
        if self
            .effects
            .iter()
            .all(|e| matches!(e.status, EffectStatus::Planned | EffectStatus::Skipped))
        {
            return OperationStatus::Prepared;
        }
        OperationStatus::InProgress
    }

    /// The next effect to send, in §3 order: merges first, then
    /// closes once every merge is confirmed. None when the refresh has
    /// not allowed the merge, an effect still needs readback, or
    /// nothing is left to send.
    pub fn next_to_send(&self) -> Option<usize> {
        if self.refresh != RefreshState::Allowed {
            return None;
        }
        if self.effects.iter().any(|e| e.status.needs_readback()) {
            return None;
        }
        let merges_done = self
            .effects
            .iter()
            .filter(|e| e.target.is_merge())
            .all(|e| matches!(e.status, EffectStatus::Confirmed | EffectStatus::Skipped));
        self.effects
            .iter()
            .position(|e| e.status == EffectStatus::Planned && (e.target.is_merge() || merges_done))
    }

    /// Recovery (§6.2–§6.3): the effects to read back before anything
    /// else is sent. A resend is never planned here; an effect returns
    /// to `planned` only when its readback proves it absent.
    pub fn recovery_readbacks(&self) -> Vec<usize> {
        self.effects
            .iter()
            .enumerate()
            .filter(|(_, e)| e.status.needs_readback())
            .map(|(i, _)| i)
            .collect()
    }
}

/// The result of a prepare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareOutcome {
    /// The operation key.
    pub key: String,
    /// True when an earlier prepare with the same key and digest was
    /// returned instead of a new operation.
    pub replayed: bool,
}

/// What finalize checks besides the effects (§5.6).
#[derive(Debug, Clone)]
pub struct FinalizeContext {
    /// The work is still `completing` under this operation.
    pub still_completing: bool,
    /// A newer cancel, hold or revocation exists.
    pub newer_cancel: bool,
    /// The existing `/complete` gates (`gate_completion`, and
    /// `disposition::require_verified` when given): `Err` with the
    /// gate's reason when they would refuse today.
    pub existing_gates: Result<(), String>,
}

/// In-memory completion operations, keyed by request key.
#[derive(Debug, Default)]
pub struct CompletionBook {
    operations: BTreeMap<String, CompletionOperation>,
    open_by_work: BTreeMap<String, String>,
    webhook_deliveries: BTreeSet<String>,
}

impl CompletionBook {
    /// An empty book.
    pub fn new() -> Self {
        Self::default()
    }

    /// The operation under `key`.
    pub fn operation(&self, key: &str) -> Option<&CompletionOperation> {
        self.operations.get(key)
    }

    fn operation_mut(&mut self, key: &str) -> Result<&mut CompletionOperation, CompletionError> {
        self.operations
            .get_mut(key)
            .ok_or_else(|| CompletionError::UnknownOperation(key.to_string()))
    }

    /// Prepare (§5.1): one operation per work, effects fixed here.
    /// The same key with the same digest returns the stored operation.
    pub fn prepare(
        &mut self,
        request: CompletionRequest,
        ctx: &PrepareContext,
    ) -> Result<PrepareOutcome, CompletionError> {
        if let Some(existing) = self.operations.get(&request.key) {
            if existing.digest == request.digest && existing.work == request.work {
                return Ok(PrepareOutcome {
                    key: request.key,
                    replayed: true,
                });
            }
            return Err(CompletionError::KeyConflict(request.key));
        }
        if !ctx.authenticated {
            return Err(CompletionError::Unauthenticated);
        }
        if ctx.worker_fence < ctx.current_fence {
            return Err(CompletionError::StaleFence {
                worker: ctx.worker_fence,
                current: ctx.current_fence,
            });
        }
        if let Some(open) = self.open_by_work.get(&request.work) {
            return Err(CompletionError::OperationOpen(open.clone()));
        }
        if ctx.hard_blocker_open {
            return Err(CompletionError::HardBlockerOpen);
        }
        if request.effects.is_empty() {
            return Err(CompletionError::NoEffects);
        }
        for effect in &request.effects {
            check_linked(&effect.target, ctx)?;
        }
        let operation = CompletionOperation {
            key: request.key.clone(),
            digest: request.digest,
            work: request.work.clone(),
            fence: ctx.worker_fence,
            effects: request
                .effects
                .into_iter()
                .map(|e| Effect {
                    target: e.target,
                    required: e.required,
                    status: EffectStatus::Planned,
                    history: vec![EffectStatus::Planned],
                })
                .collect(),
            refresh: RefreshState::NotRun,
            finalized: false,
            evidence: Vec::new(),
        };
        self.open_by_work.insert(request.work, request.key.clone());
        self.operations.insert(request.key.clone(), operation);
        Ok(PrepareOutcome {
            key: request.key,
            replayed: false,
        })
    }

    /// Effects are fixed at prepare. A stale worker's attempt is kept
    /// as evidence only; any other attempt is refused as sealed.
    pub fn add_effect(
        &mut self,
        key: &str,
        worker_fence: u64,
        current_fence: u64,
        target: EffectTarget,
    ) -> Result<(), CompletionError> {
        let operation = self.operation_mut(key)?;
        if worker_fence < current_fence {
            operation.evidence.push(format!(
                "stale worker (fence {worker_fence}) tried to add {}",
                target.display_ref()
            ));
            return Err(CompletionError::StaleFence {
                worker: worker_fence,
                current: current_fence,
            });
        }
        Err(CompletionError::OperationSealed)
    }

    /// Records a stale worker's late answer as evidence only (§6).
    pub fn record_stale_answer(&mut self, key: &str, note: &str) -> Result<(), CompletionError> {
        self.operation_mut(key)?.evidence.push(note.to_string());
        Ok(())
    }

    /// The final refresh (§5.2): REL-09's receipt for the pinned read.
    pub fn apply_refresh(
        &mut self,
        key: &str,
        receipt: &CompletionReceipt,
    ) -> Result<(), CompletionError> {
        let operation = self.operation_mut(key)?;
        if receipt.merge_allowed {
            operation.refresh = RefreshState::Allowed;
            return Ok(());
        }
        let mut late: Vec<ActivityClass> = receipt
            .late_activity
            .iter()
            .map(|a| a.class)
            .filter(|c| c.invalidates_completion())
            .collect();
        late.sort();
        late.dedup();
        operation.refresh = RefreshState::Held(late.clone());
        Err(CompletionError::RefreshHeld(late))
    }

    /// Moves one effect along the §4 table. A send (`Take`) is refused
    /// while the refresh has not allowed the merge.
    pub fn apply(
        &mut self,
        key: &str,
        index: usize,
        event: EffectEvent,
    ) -> Result<EffectStatus, CompletionError> {
        let operation = self.operation_mut(key)?;
        if event == EffectEvent::Take {
            if let RefreshState::Held(late) = &operation.refresh {
                return Err(CompletionError::RefreshHeld(late.clone()));
            }
            if operation.refresh == RefreshState::NotRun {
                return Err(CompletionError::TransitionRefused(
                    EffectStatus::Planned,
                    event,
                ));
            }
        }
        let effect = operation
            .effects
            .get_mut(index)
            .ok_or(CompletionError::UnknownEffect(index))?;
        if event == EffectEvent::SkipOptional && effect.required {
            return Err(CompletionError::SkipRequired);
        }
        let to = effect_transition(effect.status, event)?;
        if to != effect.status {
            effect.status = to;
            effect.history.push(to);
        }
        Ok(to)
    }

    /// A webhook hint (§6): deduplicated by delivery id, and it moves
    /// an effect only through a readback, never straight to completed.
    /// Returns false for a duplicate delivery, which changes nothing.
    pub fn apply_webhook(
        &mut self,
        key: &str,
        delivery: &str,
        index: usize,
        readback: Readback,
    ) -> Result<bool, CompletionError> {
        if !self.webhook_deliveries.insert(delivery.to_string()) {
            return Ok(false);
        }
        self.apply(key, index, EffectEvent::Readback(readback))?;
        Ok(true)
    }

    /// Finalize (§5.6). Scoped to the operation: it does not depend
    /// on the initiating worker's lease.
    pub fn finalize(&mut self, key: &str, ctx: &FinalizeContext) -> Result<(), CompletionError> {
        let operation = self.operation_mut(key)?;
        if operation.finalized {
            return Ok(());
        }
        if operation.refresh != RefreshState::Allowed {
            return Err(CompletionError::FinalizeRefused(
                "the final refresh has not allowed the merge".to_string(),
            ));
        }
        if let Some(effect) = operation
            .effects
            .iter()
            .find(|e| e.required && e.status != EffectStatus::Confirmed)
        {
            return Err(CompletionError::FinalizeRefused(format!(
                "required effect {} is {}",
                effect.target.display_ref(),
                effect.status.as_str()
            )));
        }
        if !ctx.still_completing {
            return Err(CompletionError::FinalizeRefused(
                "the work is no longer completing under this operation".to_string(),
            ));
        }
        if ctx.newer_cancel {
            return Err(CompletionError::FinalizeRefused(
                "a newer cancel, hold or revocation exists".to_string(),
            ));
        }
        if let Err(reason) = &ctx.existing_gates {
            return Err(CompletionError::FinalizeRefused(format!(
                "existing completion gate: {reason}"
            )));
        }
        operation.finalized = true;
        let work = operation.work.clone();
        self.open_by_work.remove(&work);
        Ok(())
    }
}

fn check_linked(target: &EffectTarget, ctx: &PrepareContext) -> Result<(), CompletionError> {
    let (repo, number) = target.reference();
    let kind = target.required_link();
    let linked = ctx
        .links
        .iter()
        .any(|l| l.kind == kind && l.repo == repo && l.number == number);
    if linked || ctx.overrides.iter().any(|o| *o == target.display_ref()) {
        Ok(())
    } else {
        Err(CompletionError::RefNotLinked(target.display_ref()))
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Activity, ActivityWatermark, evaluate_completion};
    use super::*;

    const REPO: &str = "developing-today/code";
    const HEAD: &str = "aaaa111";

    /// A fake provider with an exact request ledger (§7): it can apply
    /// a request and then lose the answer, answer 202, refuse with 409,
    /// and close issues one by one.
    #[derive(Default)]
    struct FakeGitHub {
        ledger: Vec<String>,
        merged: BTreeMap<u64, (String, String)>,
        queued: BTreeSet<u64>,
        closed_unmerged: BTreeSet<u64>,
        closed_issues: BTreeSet<u64>,
        refuse_close: BTreeSet<u64>,
        head: BTreeMap<u64, String>,
    }

    impl FakeGitHub {
        fn with_pr(number: u64, head: &str) -> Self {
            let mut fake = FakeGitHub::default();
            fake.head.insert(number, head.to_string());
            fake
        }

        fn merge(&mut self, number: u64, expected_head: &str, queue: bool) -> SendAnswer {
            self.ledger.push(format!("merge {number} {expected_head}"));
            if self.head.get(&number).map(String::as_str) != Some(expected_head) {
                return SendAnswer::Conflict;
            }
            if queue {
                self.queued.insert(number);
                return SendAnswer::Accepted;
            }
            self.merged
                .insert(number, (expected_head.to_string(), format!("sq{number}")));
            SendAnswer::AppliedAndReadBack
        }

        fn land_queued(&mut self, number: u64) {
            self.queued.remove(&number);
            let head = self.head[&number].clone();
            self.merged.insert(number, (head, format!("sq{number}")));
        }

        fn close(&mut self, number: u64) -> SendAnswer {
            self.ledger.push(format!("close {number}"));
            if self.refuse_close.contains(&number) {
                return SendAnswer::Terminal;
            }
            self.closed_issues.insert(number);
            SendAnswer::AppliedAndReadBack
        }

        fn pr(&self, number: u64) -> PrReadback {
            match self.merged.get(&number) {
                Some((head, sha)) => PrReadback {
                    merged: true,
                    closed: true,
                    queued: false,
                    head_sha: head.clone(),
                    merge_commit_sha: Some(sha.clone()),
                },
                None => PrReadback {
                    merged: false,
                    closed: self.closed_unmerged.contains(&number),
                    queued: self.queued.contains(&number),
                    head_sha: self.head.get(&number).cloned().unwrap_or_default(),
                    merge_commit_sha: None,
                },
            }
        }

        fn merges_sent(&self) -> usize {
            self.ledger
                .iter()
                .filter(|l| l.starts_with("merge "))
                .count()
        }
    }

    fn merge_target(number: u64, method: MergeMethod) -> EffectTarget {
        EffectTarget::MergePr {
            repo: REPO.to_string(),
            number,
            expected_head: HEAD.to_string(),
            method,
        }
    }

    fn close_target(number: u64) -> EffectTarget {
        EffectTarget::CloseIssue {
            repo: REPO.to_string(),
            number,
        }
    }

    fn request(key: &str, effects: Vec<EffectTarget>) -> CompletionRequest {
        CompletionRequest {
            key: key.to_string(),
            digest: format!("digest-{key}"),
            work: "task:42".to_string(),
            effects: effects
                .into_iter()
                .map(|target| RequestedEffect {
                    target,
                    required: true,
                })
                .collect(),
        }
    }

    fn context(links: &[(LinkKind, u64)]) -> PrepareContext {
        PrepareContext {
            authenticated: true,
            worker_fence: 3,
            current_fence: 3,
            hard_blocker_open: false,
            links: links
                .iter()
                .map(|(kind, number)| ActiveLink {
                    kind: *kind,
                    repo: REPO.to_string(),
                    number: *number,
                })
                .collect(),
            overrides: Vec::new(),
        }
    }

    fn allowed() -> CompletionReceipt {
        evaluate_completion(ActivityWatermark::at(10), &[])
    }

    fn finalize_ok() -> FinalizeContext {
        FinalizeContext {
            still_completing: true,
            newer_cancel: false,
            existing_gates: Ok(()),
        }
    }

    /// Prepares merge #7 + close #70 and runs the allowed refresh.
    fn prepared_merge_and_close(book: &mut CompletionBook) -> String {
        let key = "k1".to_string();
        book.prepare(
            request(
                &key,
                vec![merge_target(7, MergeMethod::Squash), close_target(70)],
            ),
            &context(&[(LinkKind::PrFor, 7), (LinkKind::Implements, 70)]),
        )
        .unwrap();
        book.apply_refresh(&key, &allowed()).unwrap();
        key
    }

    #[test]
    fn pmo_rel_completion_recovers_merge_after_lost_ack_without_webhook() {
        let mut gh = FakeGitHub::with_pr(7, HEAD);
        let mut book = CompletionBook::new();
        let key = prepared_merge_and_close(&mut book);

        // The merge applies remotely, then the process dies before the
        // answer is recorded: the effect is left `sending`.
        assert_eq!(book.operation(&key).unwrap().next_to_send(), Some(0));
        book.apply(&key, 0, EffectEvent::Take).unwrap();
        let _lost = gh.merge(7, HEAD, false);

        // Retry with the same key: same operation, readback first.
        let again = book
            .prepare(
                request(
                    &key,
                    vec![merge_target(7, MergeMethod::Squash), close_target(70)],
                ),
                &context(&[(LinkKind::PrFor, 7), (LinkKind::Implements, 70)]),
            )
            .unwrap();
        assert!(again.replayed);
        let operation = book.operation(&key).unwrap();
        assert_eq!(operation.recovery_readbacks(), vec![0]);
        assert_eq!(
            operation.next_to_send(),
            None,
            "no send before the readback"
        );
        let readback = read_back_merge(HEAD, &gh.pr(7));
        assert_eq!(
            book.apply(&key, 0, EffectEvent::Readback(readback))
                .unwrap(),
            EffectStatus::Confirmed
        );

        // Continue from the first effect that is not confirmed.
        assert_eq!(book.operation(&key).unwrap().next_to_send(), Some(1));
        book.apply(&key, 1, EffectEvent::Take).unwrap();
        let answer = gh.close(70);
        book.apply(&key, 1, EffectEvent::Answer(answer)).unwrap();
        book.finalize(&key, &finalize_ok()).unwrap();

        assert_eq!(
            book.operation(&key).unwrap().status(),
            OperationStatus::Completed
        );
        assert_eq!(gh.merges_sent(), 1, "merged exactly once");
        assert_eq!(
            gh.ledger,
            vec!["merge 7 aaaa111".to_string(), "close 70".to_string()]
        );
    }

    #[test]
    fn pmo_rel_completion_partial_issue_close_stays_completing() {
        let mut gh = FakeGitHub::with_pr(7, HEAD);
        gh.refuse_close.insert(71);
        let mut book = CompletionBook::new();
        let key = "k-partial";
        book.prepare(
            request(
                key,
                vec![
                    merge_target(7, MergeMethod::Merge),
                    close_target(70),
                    close_target(71),
                ],
            ),
            &context(&[
                (LinkKind::PrFor, 7),
                (LinkKind::Implements, 70),
                (LinkKind::Implements, 71),
            ]),
        )
        .unwrap();
        book.apply_refresh(key, &allowed()).unwrap();
        while let Some(index) = book.operation(key).unwrap().next_to_send() {
            book.apply(key, index, EffectEvent::Take).unwrap();
            let answer = match &book.operation(key).unwrap().effects[index].target {
                EffectTarget::MergePr { number, .. } => gh.merge(*number, HEAD, false),
                EffectTarget::CloseIssue { number, .. } => gh.close(*number),
            };
            book.apply(key, index, EffectEvent::Answer(answer)).unwrap();
        }
        let operation = book.operation(key).unwrap();
        assert_eq!(operation.status(), OperationStatus::Partial);
        assert_eq!(
            operation.effects[0].status,
            EffectStatus::Confirmed,
            "merge receipt kept"
        );
        assert_eq!(operation.effects[1].status, EffectStatus::Confirmed);
        assert_eq!(operation.effects[2].status, EffectStatus::FailedTerminal);
        let refused = book.finalize(key, &finalize_ok()).unwrap_err();
        assert_eq!(refused.code(), "completion_finalize_refused");
        assert!(
            !gh.ledger.iter().any(|l| l.starts_with("reopen")),
            "nothing is reopened"
        );
        // The failed effect moves only by a new operation.
        assert_eq!(
            book.apply(key, 2, EffectEvent::Take).unwrap_err(),
            CompletionError::EffectSettled(EffectStatus::FailedTerminal)
        );
    }

    #[test]
    fn pmo_rel_completion_async_merge_pending_not_completed() {
        let mut gh = FakeGitHub::with_pr(7, HEAD);
        let mut book = CompletionBook::new();
        let key = "k-async";
        book.prepare(
            request(key, vec![merge_target(7, MergeMethod::Squash)]),
            &context(&[(LinkKind::PrFor, 7)]),
        )
        .unwrap();
        book.apply_refresh(key, &allowed()).unwrap();
        book.apply(key, 0, EffectEvent::Take).unwrap();
        let answer = gh.merge(7, HEAD, true);
        assert_eq!(answer, SendAnswer::Accepted);
        book.apply(key, 0, EffectEvent::Answer(answer)).unwrap();
        assert_eq!(
            book.operation(key).unwrap().status(),
            OperationStatus::Pending
        );
        assert!(
            book.finalize(key, &finalize_ok()).is_err(),
            "acceptance is never reported as merged"
        );

        // Still queued: no change. Then the queue lands it.
        let still = read_back_merge(HEAD, &gh.pr(7));
        assert_eq!(still, Readback::StillPending);
        assert_eq!(
            book.apply(key, 0, EffectEvent::Readback(still)).unwrap(),
            EffectStatus::Pending
        );
        gh.land_queued(7);
        let landed = read_back_merge(HEAD, &gh.pr(7));
        book.apply(key, 0, EffectEvent::Readback(landed)).unwrap();
        book.finalize(key, &finalize_ok()).unwrap();
        assert_eq!(
            book.operation(key).unwrap().status(),
            OperationStatus::Completed
        );
    }

    #[test]
    fn pmo_rel_completion_head_mismatch_conflict() {
        let mut gh = FakeGitHub::with_pr(7, "bbbb222");
        let mut book = CompletionBook::new();
        let key = "k-409";
        book.prepare(
            request(key, vec![merge_target(7, MergeMethod::Merge)]),
            &context(&[(LinkKind::PrFor, 7)]),
        )
        .unwrap();
        book.apply_refresh(key, &allowed()).unwrap();
        book.apply(key, 0, EffectEvent::Take).unwrap();
        let answer = gh.merge(7, HEAD, false);
        assert_eq!(answer, SendAnswer::Conflict);
        book.apply(key, 0, EffectEvent::Answer(answer)).unwrap();
        let operation = book.operation(key).unwrap();
        assert_eq!(operation.effects[0].status, EffectStatus::Conflict);
        assert_eq!(operation.status(), OperationStatus::Conflict);
        assert!(gh.merged.is_empty(), "the new head is never merged");
        assert_eq!(operation.next_to_send(), None);
    }

    #[test]
    fn pmo_rel_completion_closed_unmerged_is_not_merged() {
        let mut gh = FakeGitHub::with_pr(7, HEAD);
        gh.closed_unmerged.insert(7);
        let readback = read_back_merge(HEAD, &gh.pr(7));
        assert_eq!(readback, Readback::Different);
        // From unknown it is a conflict, from pending a failure; never confirmed.
        assert_eq!(
            effect_transition(EffectStatus::Unknown, EffectEvent::Readback(readback)).unwrap(),
            EffectStatus::Conflict
        );
        assert_eq!(
            effect_transition(EffectStatus::Pending, EffectEvent::Readback(readback)).unwrap(),
            EffectStatus::FailedTerminal
        );
        // A merged record with another head is not this merge either.
        let other = PrReadback {
            merged: true,
            closed: true,
            queued: false,
            head_sha: "cccc333".to_string(),
            merge_commit_sha: Some("x".to_string()),
        };
        assert_eq!(read_back_merge(HEAD, &other), Readback::Different);
    }

    #[test]
    fn pmo_rel_completion_squash_verified_without_ancestry() {
        // A squash commit is not a descendant of the PR head; the PR
        // record plus the returned commit is the whole proof.
        for method in [MergeMethod::Squash, MergeMethod::Rebase, MergeMethod::Merge] {
            let pr = PrReadback {
                merged: true,
                closed: true,
                queued: false,
                head_sha: HEAD.to_string(),
                merge_commit_sha: Some("unrelated-squash-sha".to_string()),
            };
            assert_eq!(
                read_back_merge(HEAD, &pr),
                Readback::Matches,
                "{}",
                method.as_str()
            );
        }
        let missing_commit = PrReadback {
            merged: true,
            closed: true,
            queued: false,
            head_sha: HEAD.to_string(),
            merge_commit_sha: None,
        };
        assert_eq!(read_back_merge(HEAD, &missing_commit), Readback::Different);
    }

    #[test]
    fn pmo_rel_completion_only_named_linked_refs() {
        let mut book = CompletionBook::new();
        // Named but unlinked: refused before anything is written.
        let refused = book
            .prepare(
                request(
                    "k-unlinked",
                    vec![merge_target(7, MergeMethod::Squash), close_target(9999)],
                ),
                &context(&[(LinkKind::PrFor, 7), (LinkKind::Other, 9999)]),
            )
            .unwrap_err();
        assert_eq!(
            refused,
            CompletionError::RefNotLinked(format!("{REPO}#9999"))
        );
        assert_eq!(refused.code(), "completion_ref_not_linked");
        assert!(book.operation("k-unlinked").is_none());

        // An owner-confirmed override admits that one target.
        let mut ctx = context(&[(LinkKind::PrFor, 7)]);
        ctx.overrides.push(format!("{REPO}#9999"));
        book.prepare(
            request(
                "k-override",
                vec![merge_target(7, MergeMethod::Squash), close_target(9999)],
            ),
            &ctx,
        )
        .unwrap();

        // A linked epic that was not named is never added.
        let mut other = CompletionBook::new();
        other
            .prepare(
                request("k-named", vec![merge_target(7, MergeMethod::Squash)]),
                &context(&[(LinkKind::PrFor, 7), (LinkKind::Implements, 9196)]),
            )
            .unwrap();
        let operation = other.operation("k-named").unwrap();
        assert_eq!(operation.effects.len(), 1);
        assert!(
            operation
                .effects
                .iter()
                .all(|e| e.target.reference().1 != 9196)
        );
    }

    #[test]
    fn pmo_rel_completion_stale_worker_cannot_add_effects() {
        let mut book = CompletionBook::new();
        let key = prepared_merge_and_close(&mut book);
        // The lease moved on: the fence is now 4 and the old worker holds 3.
        let refused = book.add_effect(&key, 3, 4, close_target(72)).unwrap_err();
        assert_eq!(refused.code(), "completion_stale_fence");
        assert_eq!(book.operation(&key).unwrap().effects.len(), 2);
        assert_eq!(
            book.operation(&key).unwrap().evidence.len(),
            1,
            "kept as evidence only"
        );

        let mut stale = context(&[(LinkKind::PrFor, 8)]);
        stale.worker_fence = 3;
        stale.current_fence = 4;
        let mut fresh_book = CompletionBook::new();
        let no_new = fresh_book
            .prepare(
                request("k-stale", vec![merge_target(8, MergeMethod::Squash)]),
                &stale,
            )
            .unwrap_err();
        assert_eq!(no_new.code(), "completion_stale_fence");

        book.record_stale_answer(&key, "late 200 from the old worker")
            .unwrap();
        assert_eq!(book.operation(&key).unwrap().evidence.len(), 2);
        assert_eq!(
            book.operation(&key).unwrap().status(),
            OperationStatus::Prepared
        );
    }

    #[test]
    fn pmo_rel_completion_finalize_after_lease_expiry_scoped() {
        let mut gh = FakeGitHub::with_pr(7, HEAD);
        let mut book = CompletionBook::new();
        let key = prepared_merge_and_close(&mut book);
        for index in [0, 1] {
            book.apply(&key, index, EffectEvent::Take).unwrap();
            let answer = if index == 0 {
                gh.merge(7, HEAD, false)
            } else {
                gh.close(70)
            };
            book.apply(&key, index, EffectEvent::Answer(answer))
                .unwrap();
        }
        // The initiating lease has expired; finalize is scoped to the
        // operation, so a newer cancel is what blocks it, not the lease.
        let cancelled = FinalizeContext {
            newer_cancel: true,
            ..finalize_ok()
        };
        assert_eq!(
            book.finalize(&key, &cancelled).unwrap_err().code(),
            "completion_finalize_refused"
        );
        book.finalize(&key, &finalize_ok()).unwrap();
        assert_eq!(
            book.operation(&key).unwrap().status(),
            OperationStatus::Completed
        );
    }

    #[test]
    fn pmo_rel_completion_late_activity_holds_before_merge() {
        let gh = FakeGitHub::with_pr(7, HEAD);
        let mut book = CompletionBook::new();
        let key = "k-late";
        book.prepare(
            request(key, vec![merge_target(7, MergeMethod::Squash)]),
            &context(&[(LinkKind::PrFor, 7)]),
        )
        .unwrap();
        let receipt = evaluate_completion(
            ActivityWatermark::at(10),
            &[Activity::new(
                11,
                ActivityClass::Review,
                "changes requested",
            )],
        );
        assert!(!receipt.merge_allowed);
        let held = book.apply_refresh(key, &receipt).unwrap_err();
        assert_eq!(
            held,
            CompletionError::RefreshHeld(vec![ActivityClass::Review])
        );
        assert_eq!(
            book.operation(key).unwrap().status(),
            OperationStatus::Conflict
        );
        assert_eq!(
            book.apply(key, 0, EffectEvent::Take).unwrap_err().code(),
            "completion_refresh_held"
        );
        assert_eq!(book.operation(key).unwrap().next_to_send(), None);
        assert!(gh.ledger.is_empty(), "nothing was sent");
    }

    #[test]
    fn pmo_rel_completion_duplicate_webhook_no_double_transition() {
        let mut gh = FakeGitHub::with_pr(7, HEAD);
        let mut book = CompletionBook::new();
        let key = "k-hook";
        book.prepare(
            request(key, vec![merge_target(7, MergeMethod::Squash)]),
            &context(&[(LinkKind::PrFor, 7)]),
        )
        .unwrap();
        book.apply_refresh(key, &allowed()).unwrap();
        book.apply(key, 0, EffectEvent::Take).unwrap();
        gh.merge(7, HEAD, false);
        book.apply(key, 0, EffectEvent::Answer(SendAnswer::Lost))
            .unwrap();

        let readback = read_back_merge(HEAD, &gh.pr(7));
        assert!(book.apply_webhook(key, "delivery-1", 0, readback).unwrap());
        let history = book.operation(key).unwrap().effects[0].history.clone();
        assert!(
            !book.apply_webhook(key, "delivery-1", 0, readback).unwrap(),
            "duplicate delivery"
        );
        // A second distinct delivery for a confirmed effect changes nothing either.
        assert!(book.apply_webhook(key, "delivery-2", 0, readback).unwrap());
        assert_eq!(book.operation(key).unwrap().effects[0].history, history);
        assert_eq!(
            book.operation(key).unwrap().effects[0].status,
            EffectStatus::Confirmed
        );
        assert_ne!(
            book.operation(key).unwrap().status(),
            OperationStatus::Completed,
            "a webhook never finalizes"
        );
    }

    #[test]
    fn pmo_rel_completion_existing_gate_still_applies() {
        let mut gh = FakeGitHub::with_pr(7, HEAD);
        let mut book = CompletionBook::new();
        let key = "k-gate";
        book.prepare(
            request(key, vec![merge_target(7, MergeMethod::Squash)]),
            &context(&[(LinkKind::PrFor, 7)]),
        )
        .unwrap();
        book.apply_refresh(key, &allowed()).unwrap();
        book.apply(key, 0, EffectEvent::Take).unwrap();
        let answer = gh.merge(7, HEAD, false);
        book.apply(key, 0, EffectEvent::Answer(answer)).unwrap();
        let gated = FinalizeContext {
            existing_gates: Err("disposition not verified".to_string()),
            ..finalize_ok()
        };
        let refused = book.finalize(key, &gated).unwrap_err();
        assert_eq!(refused.code(), "completion_finalize_refused");
        assert!(refused.to_string().contains("disposition not verified"));
        assert_ne!(
            book.operation(key).unwrap().status(),
            OperationStatus::Completed
        );
    }

    #[test]
    fn pmo_rel_completion_same_key_one_operation() {
        let mut book = CompletionBook::new();
        let ctx = context(&[(LinkKind::PrFor, 7)]);
        let first = book
            .prepare(
                request("k-same", vec![merge_target(7, MergeMethod::Squash)]),
                &ctx,
            )
            .unwrap();
        let second = book
            .prepare(
                request("k-same", vec![merge_target(7, MergeMethod::Squash)]),
                &ctx,
            )
            .unwrap();
        assert!(!first.replayed);
        assert!(second.replayed);
        // Same key, different body: refused, never a second operation.
        let mut changed = request("k-same", vec![merge_target(7, MergeMethod::Merge)]);
        changed.digest = "other".to_string();
        assert_eq!(
            book.prepare(changed, &ctx).unwrap_err().code(),
            "completion_key_conflict"
        );
        // Another key for the same work while one is open: refused.
        let other = book
            .prepare(
                request("k-other", vec![merge_target(7, MergeMethod::Squash)]),
                &ctx,
            )
            .unwrap_err();
        assert_eq!(other.code(), "completion_operation_open");
    }

    #[test]
    fn pmo_rel_completion_transition_table_is_closed() {
        use EffectStatus as S;
        // Confirmed never moves (except an idempotent matching readback).
        for event in [
            EffectEvent::Take,
            EffectEvent::SkipOptional,
            EffectEvent::Answer(SendAnswer::Lost),
            EffectEvent::Readback(Readback::ProvenAbsent),
        ] {
            assert_eq!(
                effect_transition(S::Confirmed, event),
                Err(CompletionError::EffectSettled(S::Confirmed))
            );
        }
        // An unknown effect is never resent without a readback.
        assert_eq!(
            effect_transition(S::Unknown, EffectEvent::Take),
            Err(CompletionError::TransitionRefused(
                S::Unknown,
                EffectEvent::Take
            ))
        );
        assert_eq!(
            effect_transition(S::Unknown, EffectEvent::Readback(Readback::ProvenAbsent)),
            Ok(S::Planned)
        );
        assert_eq!(
            effect_transition(
                S::Sending,
                EffectEvent::Answer(SendAnswer::RefusedBeforeSend)
            ),
            Ok(S::Planned)
        );
        // A required effect cannot be skipped.
        let mut book = CompletionBook::new();
        let key = prepared_merge_and_close(&mut book);
        assert_eq!(
            book.apply(&key, 1, EffectEvent::SkipOptional)
                .unwrap_err()
                .code(),
            "completion_skip_required"
        );
    }
}
