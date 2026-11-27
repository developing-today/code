//! Ordered lifecycle hooks around the fixed admission gate
//! (issue #10192, slice 9110-06; design:
//! `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/crosscut/versioned-design.md`,
//! "Operation pipeline (mandatory ordering)").
//!
//! The fixed gate itself lives in `id_core::admission_gate`: every
//! retained-data admission — local or remote — passes its signature,
//! validation, signer-binding, tenant-authorization and policy
//! checks, and the only artifact of acceptance is an
//! [`AdmissionReceipt`]. This module is the lifecycle the design
//! wraps around that gate, in the design's mandatory order:
//!
//! 1. **Gate** — [`AdmissionGate::admit`] runs first; no hook runs
//!    for an envelope the gate refused, and no hook can weaken or
//!    skip a gate check.
//! 2. **Replay / idempotency** — an [`OperationLedger`] binds each
//!    operation ID to exactly one envelope. Re-presenting a
//!    committed operation resolves as
//!    [`PipelineOutcome::AlreadyCommitted`] (the caller resolves by
//!    readback, per the design); re-presenting the ID with a
//!    different envelope is a replay rejection.
//! 3. **Prepare hooks** — deterministically ordered (phase order,
//!    then configured priority, then stable registration ID). A
//!    prepare hook sees only the validated, immutable input (the
//!    receipt, the envelope, the authenticated context and the
//!    annotations earlier hooks returned) and may proceed, veto, or
//!    return namespaced, non-authoritative annotations bounded by
//!    the hook budget. It cannot mutate signed bytes or authority:
//!    the input is shared references all the way down.
//! 4. **Commit-time reauthorization** — immediately before the
//!    commit step, the injected [`Reauthorizer`] judges the receipt
//!    against the *current* policy revision. Revocation that landed
//!    while prepare hooks ran rejects the commit.
//! 5. **Commit** — the caller's commit closure runs inside the
//!    declared ordering scope. A commit failure leaves the ledger
//!    entry `Admitted` — the outcome is unknown, the operation is
//!    resumable, and a retry re-enters at the prepare cursor with
//!    reauthorization first. Nothing claims a rollback.
//! 6. **Outbox hooks** — post-commit delivery is at least once,
//!    keyed by a stable event ID (envelope digest + hook ID).
//!    Failures are recorded in the [`OutboxJournal`] as pending and
//!    retried by stable ID via [`AdmissionPipeline::retry_outbox`];
//!    a delivery failure never reports committed state as rolled
//!    back.
//!
//! Same-operation reentrancy is rejected: a hook that re-enters the
//! pipeline for the operation currently in flight gets
//! [`LifecycleRejection::ReentrantOperation`]. A hook instead
//! requests a separate queued operation — with a fresh operation
//! ID — through the [`FollowUpSink`] in its input; the requests are
//! returned in the outcome for the caller to submit as new runs.
//!
//! The per-crate ingress wiring (routing `id-events` and `id-sync`
//! retained-data paths through this pipeline) and the live policy
//! epoch source are the follow-on slices of #10192; this module is
//! the engine they share, placed in `id-store` because both crates
//! already depend on it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, Mutex};

use id_core::admission_gate::{
    AdmissionContext, AdmissionGate, AdmissionReceipt, AdmissionRejection, MAX_CONTEXT_BYTES,
};
use id_core::signed_envelope::{Envelope, EnvelopeId};

/// Maximum annotations one prepare hook may return per operation.
pub const MAX_ANNOTATIONS_PER_HOOK: usize = 8;

/// Maximum total annotation value bytes one prepare hook may
/// return per operation.
pub const MAX_ANNOTATION_BYTES_PER_HOOK: usize = 1024;

/// Maximum annotations accumulated across all prepare hooks for
/// one operation.
pub const MAX_ANNOTATIONS_TOTAL: usize = 64;

fn checked_label(field: &'static str, text: &str) -> Result<(), LifecycleRejection> {
    if !text.is_empty()
        && text.len() <= MAX_CONTEXT_BYTES
        && text.bytes().all(|byte| byte.is_ascii_graphic())
    {
        Ok(())
    } else {
        Err(LifecycleRejection::InvalidLabel {
            field,
            problem: "is not 1 to 255 visible ASCII characters",
        })
    }
}

/// A stable operation identity, bound by the caller before
/// admission (design step 2: caller, tenant, peer, operation ID and
/// the immutable signed envelope are bound together). The ledger
/// binds an operation ID to exactly one envelope ID for its whole
/// life.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationId(String);

impl OperationId {
    /// A validated operation ID.
    ///
    /// # Errors
    ///
    /// [`LifecycleRejection::InvalidLabel`] when the ID is empty,
    /// over [`MAX_CONTEXT_BYTES`], or not visible ASCII.
    pub fn new(id: impl Into<String>) -> Result<Self, LifecycleRejection> {
        let id = id.into();
        checked_label("operation_id", &id)?;
        Ok(Self(id))
    }

    /// The ID text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OperationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A hook's stable registration ID. Hook order within a phase is
/// (priority, registration ID), so IDs are part of the
/// deterministic trace.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HookId(String);

impl HookId {
    /// A validated hook ID.
    ///
    /// # Errors
    ///
    /// [`LifecycleRejection::InvalidLabel`] when the ID is empty,
    /// over [`MAX_CONTEXT_BYTES`], or not visible ASCII.
    pub fn new(id: impl Into<String>) -> Result<Self, LifecycleRejection> {
        let id = id.into();
        checked_label("hook_id", &id)?;
        Ok(Self(id))
    }

    /// The ID text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for HookId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A namespaced, non-authoritative annotation returned by a
/// prepare hook. Annotations never change acceptance: the pipeline
/// validates that the namespace is the hook's own registered
/// namespace and that the hook stayed inside its budget, and the
/// commit step receives them as data, not authority. Persisted
/// enrichment is a distinct derived object through a new full
/// admission (design step 4); an annotation is not that object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotation {
    namespace: String,
    key: String,
    value: Vec<u8>,
}

impl Annotation {
    /// An annotation under `namespace` (which must be the
    /// returning hook's registered namespace; the pipeline checks).
    ///
    /// # Errors
    ///
    /// [`LifecycleRejection::InvalidLabel`] when the namespace or
    /// key is empty, over [`MAX_CONTEXT_BYTES`], or not visible
    /// ASCII.
    pub fn new(
        namespace: impl Into<String>,
        key: impl Into<String>,
        value: Vec<u8>,
    ) -> Result<Self, LifecycleRejection> {
        let namespace = namespace.into();
        let key = key.into();
        checked_label("annotation namespace", &namespace)?;
        checked_label("annotation key", &key)?;
        Ok(Self {
            namespace,
            key,
            value,
        })
    }

    /// The annotation's namespace.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// The annotation's key within its namespace.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The annotation's value bytes.
    #[must_use]
    pub fn value(&self) -> &[u8] {
        &self.value
    }
}

/// What a prepare hook decided. A veto is final for this attempt:
/// no commit runs, and the ledger records the operation rejected
/// so a later retry starts the lifecycle from the top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrepareDecision {
    /// Continue to the next hook (or to reauthorization).
    Proceed,
    /// Continue, appending these annotations (validated against
    /// the hook's namespace and budget by the pipeline).
    Annotate(Vec<Annotation>),
    /// Refuse the operation before commit.
    Veto {
        /// Why the hook refused, recorded in the rejection.
        reason: String,
    },
}

/// A follow-up operation a hook requested: a separate queued
/// operation with a fresh operation ID, submitted by the caller
/// as a new pipeline run (design: "a callback can request a
/// separate queued operation with a fresh ID").
#[derive(Debug, Clone)]
pub struct QueuedOperation {
    /// The fresh operation ID for the follow-up.
    pub operation_id: OperationId,
    /// The follow-up's signed envelope (admitted fresh, through
    /// the full gate, when submitted).
    pub envelope: Envelope,
}

/// The sink prepare hooks request follow-up operations through.
/// Requests accumulate for the duration of one run and are handed
/// to the caller in the [`PipelineOutcome`]; the pipeline never
/// executes a follow-up inline, so hook execution cannot recurse
/// into admission.
#[derive(Debug, Default)]
pub struct FollowUpSink {
    queued: Mutex<Vec<QueuedOperation>>,
}

impl FollowUpSink {
    /// Requests a follow-up operation. The operation ID must be
    /// fresh — reusing the in-flight operation's ID is rejected
    /// when the caller submits it, by the reentrancy guard and
    /// the ledger binding.
    pub fn request(&self, operation_id: OperationId, envelope: Envelope) {
        self.queued
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(QueuedOperation {
                operation_id,
                envelope,
            });
    }

    fn drain(&self) -> Vec<QueuedOperation> {
        std::mem::take(
            &mut *self
                .queued
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }

    fn snapshot(&self) -> Vec<QueuedOperation> {
        self.queued
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn from_queued(queued: Vec<QueuedOperation>) -> Self {
        Self {
            queued: Mutex::new(queued),
        }
    }
}

/// The immutable input a prepare hook sees: the gate's receipt
/// (proof the fixed checks passed), the admitted envelope, the
/// authenticated context, and the annotations earlier hooks in
/// this run returned. Everything is a shared reference — a hook
/// cannot mutate signed bytes, the receipt, or another hook's
/// annotations.
pub struct PrepareInput<'a> {
    /// The operation being admitted.
    pub operation_id: &'a OperationId,
    /// Proof the fixed gate admitted this envelope.
    pub receipt: &'a AdmissionReceipt,
    /// The admitted envelope (immutable).
    pub envelope: &'a Envelope,
    /// The authenticated tenant/scope context.
    pub context: &'a AdmissionContext,
    /// Annotations returned by earlier prepare hooks, in hook
    /// order (read-only).
    pub annotations: &'a [Annotation],
    /// Where follow-up operations are requested.
    pub follow_ups: &'a FollowUpSink,
}

/// A prepare hook (design step 4). Implementations must be
/// deterministic for a given input: the same validated input in
/// the same order must produce the same decision, because the
/// admission trace is replayed by the model oracle.
pub trait PrepareHook: Send + Sync {
    /// The hook's stable registration ID.
    fn id(&self) -> &HookId;
    /// The hook's registered namespace; annotations outside it
    /// are a contract violation.
    fn namespace(&self) -> &str;
    /// Configured priority; lower runs earlier. Ties break on
    /// the registration ID.
    fn priority(&self) -> i32 {
        0
    }
    /// Judges the validated input.
    fn prepare(&self, input: &PrepareInput<'_>) -> PrepareDecision;
}

/// The post-commit delivery event handed to outbox hooks. The
/// `event_id` is stable for the (envelope, hook) pair, so
/// at-least-once redelivery is deduplicated by the receiver on
/// that ID.
#[derive(Debug, Clone)]
pub struct OutboxEvent {
    /// Stable event ID: `<envelope digest hex>:<hook id>`.
    pub event_id: String,
    /// The committed operation.
    pub operation_id: OperationId,
    /// The committed envelope's identity.
    pub envelope_id: EnvelopeId,
    /// The gate receipt for the committed admission.
    pub receipt: AdmissionReceipt,
    /// The annotations the prepare phase produced.
    pub annotations: Vec<Annotation>,
}

/// An outbox hook (design step 7): post-commit delivery, retried
/// by stable event ID. Returning `Err` records the delivery as
/// pending in the journal; it never un-commits the operation.
pub trait OutboxHook: Send + Sync {
    /// The hook's stable registration ID.
    fn id(&self) -> &HookId;
    /// Configured priority; lower delivers earlier. Ties break
    /// on the registration ID.
    fn priority(&self) -> i32 {
        0
    }
    /// Delivers the event.
    ///
    /// # Errors
    ///
    /// A human-readable failure reason; the delivery is recorded
    /// pending and retried by [`AdmissionPipeline::retry_outbox`].
    fn deliver(&self, event: &OutboxEvent) -> Result<(), String>;
}

/// Commit-time reauthorization seam (design step 5). The pipeline
/// calls [`Self::reauthorize`] immediately before the commit step,
/// so policy that was revoked while prepare hooks ran rejects the
/// commit. [`Self::policy_epoch`] is the current policy
/// revision/epoch, recorded in the outcome for audit.
pub trait Reauthorizer: Send + Sync {
    /// The current policy revision/epoch.
    fn policy_epoch(&self) -> u64;
    /// Judges the admitted receipt against current policy.
    ///
    /// # Errors
    ///
    /// [`AdmissionRejection`] when current policy no longer
    /// authorizes this admission (revoked signer, narrowed tenant
    /// scope, superseded acceptance).
    fn reauthorize(
        &self,
        receipt: &AdmissionReceipt,
        context: &AdmissionContext,
    ) -> Result<(), AdmissionRejection>;
}

/// Where an operation sits in its lifecycle, as the ledger
/// records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpState {
    /// The gate admitted the envelope; commit has not completed.
    /// An entry resting here means the outcome is unknown
    /// (crash or commit failure) and the operation is resumable.
    Admitted,
    /// The commit step completed. Terminal.
    Committed,
    /// A prepare hook vetoed or reauthorization refused. The
    /// operation was never committed; a retry starts fresh.
    Rejected,
}

/// One operation's ledger record: the envelope binding, the
/// admission receipt, the annotations so far, and how far the
/// prepare phase progressed (so a resumed operation continues
/// the trace instead of restarting it).
#[derive(Debug, Clone)]
pub struct LedgerEntry {
    /// The envelope this operation ID is bound to.
    pub envelope_id: EnvelopeId,
    /// The gate receipt from admission.
    pub receipt: AdmissionReceipt,
    /// Annotations accumulated by completed prepare hooks.
    pub annotations: Vec<Annotation>,
    /// Index of the next prepare hook to run.
    pub prepare_cursor: usize,
    /// Follow-up operations requested by completed prepare hooks.
    /// Retained while admitted so commit retries do not lose them.
    pub follow_ups: Vec<QueuedOperation>,
    /// The lifecycle state.
    pub state: OpState,
}

/// The operation ledger: replay/idempotency state for the
/// pipeline (design steps 3 and 6). Implementations must make
/// the binding durable before the pipeline's first use in
/// production wiring; the shipped in-memory implementation is
/// for tests and single-process adapters.
pub trait OperationLedger: Send + Sync {
    /// The entry recorded for `operation_id`, if any.
    fn entry(&self, operation_id: &OperationId) -> Option<LedgerEntry>;
    /// Records (insert or replace) the entry for `operation_id`.
    fn record(&self, operation_id: OperationId, entry: LedgerEntry);
}

/// In-memory [`OperationLedger`].
#[derive(Debug, Default)]
pub struct MemoryOperationLedger {
    entries: Mutex<BTreeMap<OperationId, LedgerEntry>>,
}

impl MemoryOperationLedger {
    /// An empty ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl OperationLedger for MemoryOperationLedger {
    fn entry(&self, operation_id: &OperationId) -> Option<LedgerEntry> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(operation_id)
            .cloned()
    }

    fn record(&self, operation_id: OperationId, entry: LedgerEntry) {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(operation_id, entry);
    }
}

/// A post-commit delivery that has not succeeded yet, recorded
/// in the [`OutboxJournal`] for retry by stable event ID.
#[derive(Debug, Clone)]
pub struct PendingDelivery {
    /// The stable event ID.
    pub event_id: String,
    /// The hook that has not accepted delivery.
    pub hook_id: HookId,
    /// The event to redeliver.
    pub event: OutboxEvent,
    /// The most recent failure reason.
    pub last_error: String,
    /// How many delivery attempts have failed.
    pub attempts: u32,
}

/// The outbox journal: pending post-commit deliveries (design
/// step 7 — record pending/quarantined failures, retry by stable
/// event ID, never report committed state as rolled back).
pub trait OutboxJournal: Send + Sync {
    /// Records or updates the pending delivery for
    /// (`event_id`, `hook_id`).
    fn record_pending(&self, pending: PendingDelivery);
    /// Every pending delivery, in stable (event ID, hook ID)
    /// order.
    fn pending(&self) -> Vec<PendingDelivery>;
    /// Drops the pending record after a successful delivery.
    fn clear(&self, event_id: &str, hook_id: &HookId);
}

/// In-memory [`OutboxJournal`].
#[derive(Debug, Default)]
pub struct MemoryOutboxJournal {
    pending: Mutex<BTreeMap<(String, HookId), PendingDelivery>>,
}

impl MemoryOutboxJournal {
    /// An empty journal.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl OutboxJournal for MemoryOutboxJournal {
    fn record_pending(&self, pending: PendingDelivery) {
        let key = (pending.event_id.clone(), pending.hook_id.clone());
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key, pending);
    }

    fn pending(&self) -> Vec<PendingDelivery> {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .cloned()
            .collect()
    }

    fn clear(&self, event_id: &str, hook_id: &HookId) {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&(event_id.to_owned(), hook_id.clone()));
    }
}

/// Why the lifecycle refused or failed an operation. Each variant
/// is one pipeline stage; [`Self::stage`] names the stage and
/// [`Self::class`] the stable failure class the model oracle
/// compares (gate-originated failures delegate to
/// [`AdmissionRejection::class`], keeping one vocabulary).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleRejection {
    /// An operation ID, hook ID, or annotation label failed
    /// validation at construction.
    InvalidLabel {
        /// The field name.
        field: &'static str,
        /// What was wrong.
        problem: &'static str,
    },
    /// The fixed gate refused the envelope.
    Gate(AdmissionRejection),
    /// The operation ID is already bound to a different envelope,
    /// or a committed operation was re-presented as new work.
    Replay {
        /// The operation ID.
        operation_id: OperationId,
        /// What was refused.
        problem: String,
    },
    /// A hook re-entered the pipeline for the operation already
    /// in flight.
    ReentrantOperation {
        /// The operation ID.
        operation_id: OperationId,
    },
    /// A prepare hook vetoed the operation.
    Vetoed {
        /// The vetoing hook.
        hook: HookId,
        /// The hook's reason.
        reason: String,
    },
    /// A prepare hook broke its contract (foreign namespace,
    /// over-budget annotations).
    HookContract {
        /// The offending hook.
        hook: HookId,
        /// What the hook did wrong.
        problem: String,
    },
    /// Commit-time reauthorization refused the operation.
    Reauthorization(AdmissionRejection),
    /// The caller's commit step failed; the outcome is unknown
    /// and the operation stays resumable in the ledger.
    CommitFailed {
        /// The commit step's failure description.
        problem: String,
    },
}

impl LifecycleRejection {
    /// The pipeline stage that produced this rejection: `input`,
    /// `gate`, `replay`, `prepare`, `reauthorize`, or `commit`.
    #[must_use]
    pub const fn stage(&self) -> &'static str {
        match self {
            Self::InvalidLabel { .. } => "input",
            Self::Gate(_) => "gate",
            Self::Replay { .. } | Self::ReentrantOperation { .. } => "replay",
            Self::Vetoed { .. } | Self::HookContract { .. } => "prepare",
            Self::Reauthorization(_) => "reauthorize",
            Self::CommitFailed { .. } => "commit",
        }
    }

    /// A stable failure class. Gate and reauthorization failures
    /// delegate to the gate's vocabulary (`bad_signature`,
    /// `unauthorized`, `policy`, …); lifecycle failures use
    /// `replay`, `reentrant`, `vetoed`, `hook_contract`, or
    /// `commit_failed`.
    #[must_use]
    pub fn class(&self) -> &'static str {
        match self {
            Self::InvalidLabel { .. } => "malformed",
            Self::Gate(rejection) | Self::Reauthorization(rejection) => rejection.class(),
            Self::Replay { .. } => "replay",
            Self::ReentrantOperation { .. } => "reentrant",
            Self::Vetoed { .. } => "vetoed",
            Self::HookContract { .. } => "hook_contract",
            Self::CommitFailed { .. } => "commit_failed",
        }
    }
}

impl fmt::Display for LifecycleRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLabel { field, problem } => {
                write!(f, "{field}: {problem}")
            }
            Self::Gate(rejection) => write!(f, "admission gate: {rejection}"),
            Self::Replay {
                operation_id,
                problem,
            } => write!(f, "operation {operation_id}: {problem}"),
            Self::ReentrantOperation { operation_id } => {
                write!(f, "operation {operation_id} is already in flight")
            }
            Self::Vetoed { hook, reason } => {
                write!(f, "prepare hook {hook} vetoed: {reason}")
            }
            Self::HookContract { hook, problem } => {
                write!(f, "prepare hook {hook} broke its contract: {problem}")
            }
            Self::Reauthorization(rejection) => {
                write!(f, "commit-time reauthorization: {rejection}")
            }
            Self::CommitFailed { problem } => {
                write!(f, "commit step failed (outcome unknown): {problem}")
            }
        }
    }
}

impl std::error::Error for LifecycleRejection {}

/// What the caller's commit step receives: the receipt, the
/// authenticated context, and the prepare phase's annotations.
/// The commit step performs the state transition inside its
/// declared transaction/ordering scope; it gets no path back to
/// the gate, the hooks, or the ledger.
pub struct CommitInput<'a> {
    /// The operation being committed.
    pub operation_id: &'a OperationId,
    /// Proof the gate admitted this envelope and reauthorization
    /// just passed.
    pub receipt: &'a AdmissionReceipt,
    /// The authenticated context.
    pub context: &'a AdmissionContext,
    /// The prepare phase's annotations, in hook order.
    pub annotations: &'a [Annotation],
}

/// The result of a pipeline run.
#[derive(Debug)]
pub enum PipelineOutcome {
    /// The operation committed in this run.
    Committed {
        /// The gate receipt.
        receipt: AdmissionReceipt,
        /// The prepare phase's annotations, in hook order.
        annotations: Vec<Annotation>,
        /// Outbox hooks whose delivery is pending (recorded in
        /// the journal for retry; the commit stands).
        pending_outbox: Vec<HookId>,
        /// Follow-up operations hooks requested during this run.
        follow_ups: Vec<QueuedOperation>,
        /// The policy epoch reauthorization judged at commit.
        policy_epoch: u64,
    },
    /// The operation was already committed by an earlier run;
    /// the caller resolves by readback (design step 6). No hook
    /// ran and nothing was re-committed.
    AlreadyCommitted {
        /// The original admission's receipt.
        receipt: AdmissionReceipt,
    },
}

struct InFlightGuard<'a> {
    set: &'a Mutex<BTreeSet<OperationId>>,
    operation_id: OperationId,
}

impl Drop for InFlightGuard<'_> {
    fn drop(&mut self) {
        self.set
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.operation_id);
    }
}

fn event_id_for(envelope_id: &EnvelopeId, hook_id: &HookId) -> String {
    let digest = envelope_id.content_id().digest();
    let mut hex = String::with_capacity(digest.len() * 2 + 1 + hook_id.as_str().len());
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex.push(':');
    hex.push_str(hook_id.as_str());
    hex
}

/// The admission lifecycle pipeline: fixed gate, replay ledger,
/// ordered prepare hooks, commit-time reauthorization, the
/// caller's commit step, and at-least-once outbox delivery — in
/// the design's mandatory order, with no stage skippable.
pub struct AdmissionPipeline {
    gate: AdmissionGate,
    reauthorizer: Arc<dyn Reauthorizer>,
    ledger: Arc<dyn OperationLedger>,
    journal: Arc<dyn OutboxJournal>,
    prepare_hooks: Vec<Arc<dyn PrepareHook>>,
    outbox_hooks: Vec<Arc<dyn OutboxHook>>,
    in_flight: Mutex<BTreeSet<OperationId>>,
}

impl AdmissionPipeline {
    /// A pipeline over the given gate, reauthorizer, ledger and
    /// journal, with no hooks registered.
    #[must_use]
    pub fn new(
        gate: AdmissionGate,
        reauthorizer: Arc<dyn Reauthorizer>,
        ledger: Arc<dyn OperationLedger>,
        journal: Arc<dyn OutboxJournal>,
    ) -> Self {
        Self {
            gate,
            reauthorizer,
            ledger,
            journal,
            prepare_hooks: Vec::new(),
            outbox_hooks: Vec::new(),
            in_flight: Mutex::new(BTreeSet::new()),
        }
    }

    /// Registers a prepare hook. Registration order does not
    /// matter: execution order is (priority, registration ID).
    pub fn register_prepare(&mut self, hook: Arc<dyn PrepareHook>) {
        self.prepare_hooks.push(hook);
        self.prepare_hooks
            .sort_by(|a, b| (a.priority(), a.id()).cmp(&(b.priority(), b.id())));
    }

    /// Registers an outbox hook. Delivery order is (priority,
    /// registration ID).
    pub fn register_outbox(&mut self, hook: Arc<dyn OutboxHook>) {
        self.outbox_hooks.push(hook);
        self.outbox_hooks
            .sort_by(|a, b| (a.priority(), a.id()).cmp(&(b.priority(), b.id())));
    }

    /// Runs one operation through the full lifecycle.
    ///
    /// `commit` is the caller's state transition; it runs exactly
    /// once per successful pass, after reauthorization, and only
    /// for an operation the ledger has not already committed.
    ///
    /// # Errors
    ///
    /// [`LifecycleRejection`] naming the stage that refused:
    /// gate, replay/reentrancy, prepare (veto or contract),
    /// reauthorization, or the commit step itself.
    pub fn run(
        &self,
        operation_id: OperationId,
        envelope: &Envelope,
        context: &AdmissionContext,
        commit: impl FnOnce(&CommitInput<'_>) -> Result<(), String>,
    ) -> Result<PipelineOutcome, LifecycleRejection> {
        {
            let mut in_flight = self
                .in_flight
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !in_flight.insert(operation_id.clone()) {
                return Err(LifecycleRejection::ReentrantOperation { operation_id });
            }
        }
        let _guard = InFlightGuard {
            set: &self.in_flight,
            operation_id: operation_id.clone(),
        };

        // Stage 1 — the fixed gate. Nothing below runs for an
        // envelope the gate refused.
        let receipt = self
            .gate
            .admit(envelope, context)
            .map_err(LifecycleRejection::Gate)?;

        // Stage 2 — replay / idempotency against the ledger.
        let mut entry = match self.ledger.entry(&operation_id) {
            Some(entry) => {
                if entry.envelope_id != *receipt.envelope_id() {
                    return Err(LifecycleRejection::Replay {
                        operation_id,
                        problem: "operation ID is already bound to a different envelope".to_owned(),
                    });
                }
                match entry.state {
                    OpState::Committed => {
                        return Ok(PipelineOutcome::AlreadyCommitted {
                            receipt: entry.receipt,
                        });
                    }
                    OpState::Admitted => entry,
                    OpState::Rejected => LedgerEntry {
                        envelope_id: receipt.envelope_id().clone(),
                        receipt: receipt.clone(),
                        annotations: Vec::new(),
                        prepare_cursor: 0,
                        follow_ups: Vec::new(),
                        state: OpState::Admitted,
                    },
                }
            }
            None => LedgerEntry {
                envelope_id: receipt.envelope_id().clone(),
                receipt: receipt.clone(),
                annotations: Vec::new(),
                prepare_cursor: 0,
                follow_ups: Vec::new(),
                state: OpState::Admitted,
            },
        };
        self.ledger.record(operation_id.clone(), entry.clone());

        // Stage 3 — prepare hooks, in deterministic order,
        // resuming at the recorded cursor. Persist requests alongside
        // the cursor so a commit retry can deliver them without rerunning hooks.
        let follow_ups = FollowUpSink::from_queued(entry.follow_ups.clone());
        while entry.prepare_cursor < self.prepare_hooks.len() {
            let hook = &self.prepare_hooks[entry.prepare_cursor];
            let input = PrepareInput {
                operation_id: &operation_id,
                receipt: &receipt,
                envelope,
                context,
                annotations: &entry.annotations,
                follow_ups: &follow_ups,
            };
            match hook.prepare(&input) {
                PrepareDecision::Proceed => {}
                PrepareDecision::Annotate(annotations) => {
                    self.check_annotations(hook, &annotations, entry.annotations.len())?;
                    entry.annotations.extend(annotations);
                }
                PrepareDecision::Veto { reason } => {
                    entry.state = OpState::Rejected;
                    self.ledger.record(operation_id.clone(), entry);
                    return Err(LifecycleRejection::Vetoed {
                        hook: hook.id().clone(),
                        reason,
                    });
                }
            }
            entry.prepare_cursor += 1;
            entry.follow_ups = follow_ups.snapshot();
            self.ledger.record(operation_id.clone(), entry.clone());
        }

        // Stage 4 — commit-time reauthorization against current
        // policy. Revocation during the prepare phase lands here.
        if let Err(rejection) = self.reauthorizer.reauthorize(&receipt, context) {
            entry.state = OpState::Rejected;
            self.ledger.record(operation_id.clone(), entry);
            return Err(LifecycleRejection::Reauthorization(rejection));
        }
        let policy_epoch = self.reauthorizer.policy_epoch();

        // Stage 5 — the caller's commit step. A failure leaves
        // the entry Admitted: outcome unknown, resumable, never
        // reported as rolled back.
        let commit_input = CommitInput {
            operation_id: &operation_id,
            receipt: &receipt,
            context,
            annotations: &entry.annotations,
        };
        if let Err(problem) = commit(&commit_input) {
            return Err(LifecycleRejection::CommitFailed { problem });
        }
        entry.state = OpState::Committed;
        self.ledger.record(operation_id.clone(), entry.clone());

        // Stage 6 — outbox delivery, at least once, failures
        // recorded pending. The commit stands regardless.
        let mut pending_outbox = Vec::new();
        for hook in &self.outbox_hooks {
            let event = OutboxEvent {
                event_id: event_id_for(receipt.envelope_id(), hook.id()),
                operation_id: operation_id.clone(),
                envelope_id: receipt.envelope_id().clone(),
                receipt: receipt.clone(),
                annotations: entry.annotations.clone(),
            };
            if let Err(reason) = hook.deliver(&event) {
                self.journal.record_pending(PendingDelivery {
                    event_id: event.event_id.clone(),
                    hook_id: hook.id().clone(),
                    event,
                    last_error: reason,
                    attempts: 1,
                });
                pending_outbox.push(hook.id().clone());
            }
        }

        Ok(PipelineOutcome::Committed {
            receipt,
            annotations: entry.annotations,
            pending_outbox,
            follow_ups: follow_ups.drain(),
            policy_epoch,
        })
    }

    fn check_annotations(
        &self,
        hook: &Arc<dyn PrepareHook>,
        annotations: &[Annotation],
        already: usize,
    ) -> Result<(), LifecycleRejection> {
        let contract = |problem: String| LifecycleRejection::HookContract {
            hook: hook.id().clone(),
            problem,
        };
        if annotations.len() > MAX_ANNOTATIONS_PER_HOOK {
            return Err(contract(format!(
                "returned {} annotations, over the per-hook budget of {MAX_ANNOTATIONS_PER_HOOK}",
                annotations.len()
            )));
        }
        let bytes: usize = annotations.iter().map(|a| a.value.len()).sum();
        if bytes > MAX_ANNOTATION_BYTES_PER_HOOK {
            return Err(contract(format!(
                "returned {bytes} annotation value bytes, over the per-hook budget of {MAX_ANNOTATION_BYTES_PER_HOOK}"
            )));
        }
        if already + annotations.len() > MAX_ANNOTATIONS_TOTAL {
            return Err(contract(format!(
                "operation would exceed the total annotation budget of {MAX_ANNOTATIONS_TOTAL}"
            )));
        }
        for annotation in annotations {
            if annotation.namespace() != hook.namespace() {
                return Err(contract(format!(
                    "annotation namespace {:?} is not the hook's registered namespace {:?}",
                    annotation.namespace(),
                    hook.namespace()
                )));
            }
        }
        Ok(())
    }

    /// Retries every pending outbox delivery by stable event ID.
    /// Returns the hook IDs whose deliveries are still pending
    /// afterwards. Committed state is never touched here.
    pub fn retry_outbox(&self) -> Vec<HookId> {
        let mut still_pending = Vec::new();
        for pending in self.journal.pending() {
            let Some(hook) = self
                .outbox_hooks
                .iter()
                .find(|hook| hook.id() == &pending.hook_id)
            else {
                still_pending.push(pending.hook_id.clone());
                continue;
            };
            match hook.deliver(&pending.event) {
                Ok(()) => self.journal.clear(&pending.event_id, &pending.hook_id),
                Err(reason) => {
                    self.journal.record_pending(PendingDelivery {
                        attempts: pending.attempts + 1,
                        last_error: reason,
                        ..pending.clone()
                    });
                    still_pending.push(pending.hook_id.clone());
                }
            }
        }
        still_pending
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use ed25519_dalek::SigningKey;
    use id_core::admission_gate::{GatePolicy, ScopeBindingAuthorizer, StaticSignerDirectory};
    use id_core::id_shape::HashProfile;
    use id_core::signed_envelope::{Body, Envelope, PayloadId, SUITE_ED25519};
    use std::sync::OnceLock;

    const RAW: HashProfile = HashProfile::RawBlake3_256;
    const DOMAIN: &str = "id/atlas";
    const TENANT: &str = "tenant-a";

    fn signing_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn envelope(payload: Vec<u8>) -> Envelope {
        let key = signing_key(11);
        let body = Body {
            domain: DOMAIN.to_string(),
            context: TENANT.to_string(),
            subject: None,
            graph: None,
            predecessor: None,
            signer_id: "alice".to_string(),
            key: key.verifying_key().to_bytes(),
            suite: SUITE_ED25519.to_string(),
            payload_id: PayloadId::of(RAW, &payload).unwrap(),
            payload_length: payload.len() as u64,
        };
        Envelope::sign(body, payload, &key).unwrap()
    }

    fn context() -> AdmissionContext {
        AdmissionContext::new("alice", TENANT, "admission:write").unwrap()
    }

    fn gate() -> AdmissionGate {
        let mut directory = StaticSignerDirectory::new();
        directory.register("alice", signing_key(11).verifying_key().to_bytes());
        AdmissionGate::new(
            GatePolicy::admitting([DOMAIN.to_string()]),
            directory,
            ScopeBindingAuthorizer,
        )
    }

    #[derive(Default)]
    struct StubReauthorizer {
        revoked: Mutex<bool>,
    }

    impl Reauthorizer for StubReauthorizer {
        fn policy_epoch(&self) -> u64 {
            7
        }

        fn reauthorize(
            &self,
            _receipt: &AdmissionReceipt,
            _context: &AdmissionContext,
        ) -> Result<(), AdmissionRejection> {
            if *self.revoked.lock().unwrap() {
                Err(AdmissionRejection::Policy {
                    problem: "signer revoked at current policy epoch".to_string(),
                })
            } else {
                Ok(())
            }
        }
    }

    struct FnPrepareHook {
        id: HookId,
        priority: i32,
        namespace: String,
        f: Box<dyn Fn(&PrepareInput<'_>) -> PrepareDecision + Send + Sync>,
    }

    impl PrepareHook for FnPrepareHook {
        fn id(&self) -> &HookId {
            &self.id
        }

        fn namespace(&self) -> &str {
            &self.namespace
        }

        fn priority(&self) -> i32 {
            self.priority
        }

        fn prepare(&self, input: &PrepareInput<'_>) -> PrepareDecision {
            (self.f)(input)
        }
    }

    fn prepare_hook(
        id: &str,
        priority: i32,
        f: impl Fn(&PrepareInput<'_>) -> PrepareDecision + Send + Sync + 'static,
    ) -> Arc<dyn PrepareHook> {
        Arc::new(FnPrepareHook {
            id: HookId::new(id).unwrap(),
            priority,
            namespace: format!("ns/{id}"),
            f: Box::new(f),
        })
    }

    struct FnOutboxHook {
        id: HookId,
        priority: i32,
        f: Mutex<Box<dyn FnMut(&OutboxEvent) -> Result<(), String> + Send>>,
    }

    impl OutboxHook for FnOutboxHook {
        fn id(&self) -> &HookId {
            &self.id
        }

        fn priority(&self) -> i32 {
            self.priority
        }

        fn deliver(&self, event: &OutboxEvent) -> Result<(), String> {
            (self.f.lock().unwrap())(event)
        }
    }

    struct Rig {
        pipeline: AdmissionPipeline,
        reauthorizer: Arc<StubReauthorizer>,
        ledger: Arc<MemoryOperationLedger>,
        journal: Arc<MemoryOutboxJournal>,
    }

    fn rig() -> Rig {
        let reauthorizer = Arc::new(StubReauthorizer::default());
        let ledger = Arc::new(MemoryOperationLedger::new());
        let journal = Arc::new(MemoryOutboxJournal::new());
        let pipeline = AdmissionPipeline::new(
            gate(),
            reauthorizer.clone(),
            ledger.clone(),
            journal.clone(),
        );
        Rig {
            pipeline,
            reauthorizer,
            ledger,
            journal,
        }
    }

    fn op(id: &str) -> OperationId {
        OperationId::new(id).unwrap()
    }

    fn ok_commit(_: &CommitInput<'_>) -> Result<(), String> {
        Ok(())
    }

    #[test]
    fn prepare_hooks_run_in_priority_then_id_order() {
        let mut rig = rig();
        let trace: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        for (id, priority) in [("zeta", 5), ("alpha", 5), ("mid", 1), ("beta", 5)] {
            let trace = trace.clone();
            rig.pipeline
                .register_prepare(prepare_hook(id, priority, move |_| {
                    trace.lock().unwrap().push(id.to_string());
                    PrepareDecision::Proceed
                }));
        }
        let outcome = rig
            .pipeline
            .run(op("op-order"), &envelope(vec![1; 8]), &context(), ok_commit)
            .unwrap();
        assert!(matches!(outcome, PipelineOutcome::Committed { .. }));
        assert_eq!(
            *trace.lock().unwrap(),
            ["mid", "alpha", "beta", "zeta"],
            "phase order is (priority, registration ID), not registration order"
        );
    }

    #[test]
    fn veto_blocks_commit_and_retry_starts_fresh() {
        let mut rig = self::rig();
        let veto = Arc::new(Mutex::new(true));
        let veto_in_hook = veto.clone();
        rig.pipeline
            .register_prepare(prepare_hook("gatekeeper", 0, move |_| {
                if *veto_in_hook.lock().unwrap() {
                    PrepareDecision::Veto {
                        reason: "not yet".to_string(),
                    }
                } else {
                    PrepareDecision::Proceed
                }
            }));
        let committed = Arc::new(Mutex::new(false));
        let committed_in_commit = committed.clone();
        let commit = move |_: &CommitInput<'_>| {
            *committed_in_commit.lock().unwrap() = true;
            Ok(())
        };
        let err = rig
            .pipeline
            .run(op("op-veto"), &envelope(vec![2; 8]), &context(), commit)
            .unwrap_err();
        assert_eq!(err.class(), "vetoed");
        assert_eq!(err.stage(), "prepare");
        assert!(
            !*committed.lock().unwrap(),
            "a vetoed operation never commits"
        );
        assert_eq!(
            rig.ledger.entry(&op("op-veto")).unwrap().state,
            OpState::Rejected
        );

        *veto.lock().unwrap() = false;
        let outcome = rig
            .pipeline
            .run(op("op-veto"), &envelope(vec![2; 8]), &context(), ok_commit)
            .unwrap();
        assert!(matches!(outcome, PipelineOutcome::Committed { .. }));
    }

    #[test]
    fn revocation_during_prepare_blocks_commit() {
        let mut rig = rig();
        let reauthorizer = rig.reauthorizer.clone();
        rig.pipeline
            .register_prepare(prepare_hook("revoker", 0, move |_| {
                *reauthorizer.revoked.lock().unwrap() = true;
                PrepareDecision::Proceed
            }));
        let committed = Arc::new(Mutex::new(false));
        let committed_in_commit = committed.clone();
        let err = rig
            .pipeline
            .run(
                op("op-revoked"),
                &envelope(vec![3; 8]),
                &context(),
                move |_: &CommitInput<'_>| {
                    *committed_in_commit.lock().unwrap() = true;
                    Ok(())
                },
            )
            .unwrap_err();
        assert_eq!(err.stage(), "reauthorize");
        assert_eq!(err.class(), "policy");
        assert!(!*committed.lock().unwrap());
    }

    #[test]
    fn committed_operation_replay_resolves_already_committed() {
        let rig = rig();
        let env = envelope(vec![4; 8]);
        let first = rig
            .pipeline
            .run(op("op-once"), &env, &context(), ok_commit)
            .unwrap();
        let PipelineOutcome::Committed { receipt, .. } = first else {
            panic!("first run must commit");
        };
        let second = rig
            .pipeline
            .run(op("op-once"), &env, &context(), |_| {
                panic!("an already-committed operation must not re-run the commit step")
            })
            .unwrap();
        let PipelineOutcome::AlreadyCommitted { receipt: original } = second else {
            panic!("second run must resolve as already committed");
        };
        assert_eq!(original, receipt);
    }

    #[test]
    fn operation_id_reuse_with_a_different_envelope_is_a_replay() {
        let rig = rig();
        rig.pipeline
            .run(op("op-bound"), &envelope(vec![5; 8]), &context(), ok_commit)
            .unwrap();
        let err = rig
            .pipeline
            .run(op("op-bound"), &envelope(vec![6; 8]), &context(), ok_commit)
            .unwrap_err();
        assert_eq!(err.class(), "replay");
        assert_eq!(err.stage(), "replay");
    }

    #[test]
    fn reentrant_same_operation_is_rejected() {
        let cell: Arc<OnceLock<Arc<AdmissionPipeline>>> = Arc::new(OnceLock::new());
        let mut rig = rig();
        let cell_in_hook = cell.clone();
        let inner_class: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let inner_in_hook = inner_class.clone();
        rig.pipeline
            .register_prepare(prepare_hook("loopy", 0, move |input| {
                let pipeline = cell_in_hook.get().expect("pipeline cell set").clone();
                let inner = pipeline.run(
                    input.operation_id.clone(),
                    input.envelope,
                    input.context,
                    ok_commit,
                );
                *inner_in_hook.lock().unwrap() = Some(match inner {
                    Ok(_) => "ok".to_string(),
                    Err(rejection) => rejection.class().to_string(),
                });
                PrepareDecision::Proceed
            }));
        let pipeline = Arc::new(rig.pipeline);
        assert!(cell.set(pipeline.clone()).is_ok());
        let outcome = pipeline
            .run(op("op-loop"), &envelope(vec![7; 8]), &context(), ok_commit)
            .unwrap();
        assert!(matches!(outcome, PipelineOutcome::Committed { .. }));
        assert_eq!(inner_class.lock().unwrap().as_deref(), Some("reentrant"));
    }

    #[test]
    fn annotation_namespace_and_budget_are_enforced() {
        let mut rig = rig();
        rig.pipeline
            .register_prepare(prepare_hook("foreign", 0, |_| {
                PrepareDecision::Annotate(vec![Annotation::new("ns/other", "k", vec![1]).unwrap()])
            }));
        let err = rig
            .pipeline
            .run(
                op("op-foreign"),
                &envelope(vec![8; 8]),
                &context(),
                ok_commit,
            )
            .unwrap_err();
        assert_eq!(err.class(), "hook_contract");

        let mut rig = self::rig();
        rig.pipeline
            .register_prepare(prepare_hook("chatty", 0, |input| {
                let annotations = (0..=MAX_ANNOTATIONS_PER_HOOK)
                    .map(|i| Annotation::new("ns/chatty", format!("k{i}"), vec![1]).unwrap())
                    .collect();
                let _ = input;
                PrepareDecision::Annotate(annotations)
            }));
        let err = rig
            .pipeline
            .run(
                op("op-chatty"),
                &envelope(vec![9; 8]),
                &context(),
                ok_commit,
            )
            .unwrap_err();
        assert_eq!(err.class(), "hook_contract");
    }

    #[test]
    fn commit_failure_leaves_the_operation_resumable() {
        let mut rig = rig();
        let prepare_runs = Arc::new(Mutex::new(0_u32));
        let runs_in_hook = prepare_runs.clone();
        rig.pipeline
            .register_prepare(prepare_hook("counter", 0, move |_| {
                *runs_in_hook.lock().unwrap() += 1;
                PrepareDecision::Proceed
            }));
        let env = envelope(vec![10; 8]);
        let err = rig
            .pipeline
            .run(op("op-flaky"), &env, &context(), |_| {
                Err("store unavailable".to_string())
            })
            .unwrap_err();
        assert_eq!(err.class(), "commit_failed");
        assert_eq!(
            rig.ledger.entry(&op("op-flaky")).unwrap().state,
            OpState::Admitted,
            "an unknown commit outcome stays resumable, never rejected or rolled back"
        );

        let outcome = rig
            .pipeline
            .run(op("op-flaky"), &env, &context(), ok_commit)
            .unwrap();
        assert!(matches!(outcome, PipelineOutcome::Committed { .. }));
        assert_eq!(
            *prepare_runs.lock().unwrap(),
            1,
            "a resumed operation continues the prepare trace at its cursor"
        );
    }

    #[test]
    fn outbox_failure_is_pending_and_retries_by_stable_event_id() {
        let mut rig = rig();
        let attempts: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let attempts_in_hook = attempts.clone();
        rig.pipeline.register_outbox(Arc::new(FnOutboxHook {
            id: HookId::new("mailer").unwrap(),
            priority: 0,
            f: Mutex::new(Box::new(move |event: &OutboxEvent| {
                let mut attempts = attempts_in_hook.lock().unwrap();
                attempts.push(event.event_id.clone());
                if attempts.len() < 2 {
                    Err("smtp down".to_string())
                } else {
                    Ok(())
                }
            })),
        }));
        let outcome = rig
            .pipeline
            .run(op("op-mail"), &envelope(vec![11; 8]), &context(), ok_commit)
            .unwrap();
        let PipelineOutcome::Committed { pending_outbox, .. } = outcome else {
            panic!("the commit stands even when delivery fails");
        };
        assert_eq!(pending_outbox, [HookId::new("mailer").unwrap()]);
        assert_eq!(rig.journal.pending().len(), 1);
        assert_eq!(rig.journal.pending()[0].attempts, 1);

        let still = rig.pipeline.retry_outbox();
        assert_eq!(still, [] as [HookId; 0]);
        assert!(rig.journal.pending().is_empty());
        let attempts = attempts.lock().unwrap();
        assert_eq!(attempts.len(), 2);
        assert_eq!(
            attempts[0], attempts[1],
            "redelivery uses the stable event ID"
        );
        assert!(attempts[0].ends_with(":mailer"));
    }

    #[test]
    fn follow_up_requests_are_returned_not_executed() {
        let mut rig = rig();
        rig.pipeline
            .register_prepare(prepare_hook("spawner", 0, |input| {
                input.follow_ups.request(
                    OperationId::new("op-child").unwrap(),
                    input.envelope.clone(),
                );
                PrepareDecision::Proceed
            }));
        let outcome = rig
            .pipeline
            .run(
                op("op-parent"),
                &envelope(vec![12; 8]),
                &context(),
                ok_commit,
            )
            .unwrap();
        let PipelineOutcome::Committed { follow_ups, .. } = outcome else {
            panic!("parent commits");
        };
        assert_eq!(follow_ups.len(), 1);
        assert_eq!(follow_ups[0].operation_id, op("op-child"));
        assert!(
            rig.ledger.entry(&op("op-child")).is_none(),
            "a follow-up is a request for a new run, never an inline execution"
        );
    }
}
