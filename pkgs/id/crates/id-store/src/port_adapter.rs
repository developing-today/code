//! Capability profiles for the two committed record-store backends
//! (slice 9110-03, #10174; parent #9110). The profiles consume the
//! store contract owned by #9082 and are expressed against the port
//! contract of slice 9110-02 (#10160, `id_core::port`).
//!
//! Each profile is a *declaration*: the transaction scope, durability,
//! scan, retry and timeout-ambiguity semantics a caller may rely on,
//! stated before any operation runs. The declarations are deliberately
//! conservative — the settled design (crosscut versioned-design, V2)
//! requires adapters to advertise only explicit transaction APIs and
//! conditional durability, and an unknown remote outcome stays unknown
//! pending stable-ID readback. Deployment-pinned measurements belong
//! to the shared conformance fixtures (slice 9110-04); if a future
//! conformance run contradicts a declaration here, the declaration is
//! the bug.
//!
//! The two profiles intentionally do **not** claim identical
//! atomicity:
//!
//! - **redb** (native, embedded): one write transaction spans the
//!   whole database, and the commit call's return *is* the outcome —
//!   success means committed, an error means not committed. There is
//!   no response to lose, so outcomes are definitive.
//! - **Turso / libSQL over HTTP** (hosted): the default write path
//!   commits atomically within one namespace; a whole-database commit
//!   is available only under an explicit interactive transaction on a
//!   single session. A commit is a network round trip, so a lost
//!   response (timeout, dropped connection) leaves the outcome
//!   *unknown*: it stays unknown until resolved by readback under the
//!   operation's stable ID. Callers must not claim rollback, must not
//!   blindly replay, and must reauthorize before a retry.
//!
//! This module is not yet wired into this crate's `lib.rs`: public
//! module/export wiring is slice 9110-09's, per the summit sequencing
//! rules. Until then the conformance suite
//! (`pkgs/id/tests/store_adapter_profiles.rs`) includes this module
//! by direct path.

use id_core::port::{
    CapabilityId, PORT_CONTRACT_VERSION, Port, PortDescriptor, PortError, PortKind, PortLimits,
    RecordStorePort, SemanticGuarantees,
};

/// A commit spanning more than one record, within one namespace, is
/// atomic. Additive capability under contract version 2.
pub const TXN_MULTI_RECORD: CapabilityId = CapabilityId("store.txn.multi-record");
/// A commit spanning every namespace of one database is atomic.
/// Additive capability under contract version 2.
pub const TXN_WHOLE_DATABASE: CapabilityId = CapabilityId("store.txn.whole-database");

/// The breadth a single commit can span, narrowest first. A profile
/// declares the broadest scope it commits atomically; anything wider
/// is refused (or conditional) before any side effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AtomicScope {
    /// One record in one namespace.
    SingleRecord,
    /// Many records within one namespace.
    SingleNamespace,
    /// Many records across every namespace of one database.
    WholeDatabase,
}

impl AtomicScope {
    /// The capability an operation must be granted to rely on this
    /// scope, if any beyond the base record-store contract.
    pub fn required_capability(self) -> Option<CapabilityId> {
        match self {
            Self::SingleRecord => None,
            Self::SingleNamespace => Some(TXN_MULTI_RECORD),
            Self::WholeDatabase => Some(TXN_WHOLE_DATABASE),
        }
    }
}

/// What a lost commit response means for an adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitAmbiguity {
    /// The commit call's return decides the outcome: success means
    /// committed, an error means not committed. There is no third
    /// state and no response that can be lost in transit.
    Definitive,
    /// A lost response leaves the outcome unknown. It stays unknown
    /// until resolved by readback under the operation's stable ID;
    /// neither "committed" nor "rolled back" may be inferred.
    UnknownUntilReadback,
}

/// The outcome of a commit, as the caller is entitled to state it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitOutcome {
    /// The commit is known to have applied.
    Committed,
    /// The commit is known not to have applied.
    NotCommitted,
    /// The commit may or may not have applied; only stable-ID
    /// readback can resolve it.
    Unknown,
}

/// How a caller may retry after a commit did not report `Committed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryRule {
    /// A failed commit definitively did not apply, so the same
    /// writes may be retried as a new operation.
    RetryAsNewOperation,
    /// Resolve the original operation by stable-ID readback first;
    /// replay only the writes readback proves absent, under the
    /// original stable operation ID, after reauthorization.
    ReadbackBeforeRetry,
}

/// One backend's declared capability profile: the port descriptor it
/// advertises plus the store-specific semantics the descriptor has
/// no fields for (atomic scope, commit ambiguity, retry rule).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreProfile {
    descriptor: PortDescriptor,
    atomic_scope: AtomicScope,
    conditional_atomic_scope: Option<(AtomicScope, &'static str)>,
    commit_ambiguity: CommitAmbiguity,
    retry_rule: RetryRule,
}

impl StoreProfile {
    /// The broadest scope a single commit spans atomically on the
    /// default write path.
    pub fn atomic_scope(&self) -> AtomicScope {
        self.atomic_scope
    }

    /// A broader scope available only under the stated condition,
    /// when the backend has one.
    pub fn conditional_atomic_scope(&self) -> Option<(AtomicScope, &'static str)> {
        self.conditional_atomic_scope
    }

    /// What a lost commit response means for this backend.
    pub fn commit_ambiguity(&self) -> CommitAmbiguity {
        self.commit_ambiguity
    }

    /// How a caller may retry after a non-`Committed` report.
    pub fn retry_rule(&self) -> RetryRule {
        self.retry_rule
    }

    /// Whether this profile declares `scope`, given whether the
    /// caller met the condition attached to the conditional scope.
    pub fn declares_scope(&self, scope: AtomicScope, condition_met: bool) -> bool {
        scope <= self.atomic_scope
            || (condition_met
                && self
                    .conditional_atomic_scope
                    .is_some_and(|(conditional, _)| scope <= conditional))
    }

    /// Refuse a commit spanning more than the declared scope,
    /// before any side effect. A scope available only under an
    /// unmet condition is a typed `Conditional` refusal, not a
    /// silent narrowing of the commit.
    pub fn check_commit(&self, scope: AtomicScope, condition_met: bool) -> Result<(), PortError> {
        if scope <= self.atomic_scope {
            return Ok(());
        }
        if let Some((conditional, condition)) = self.conditional_atomic_scope {
            if scope <= conditional {
                if condition_met {
                    return Ok(());
                }
                return Err(PortError::Conditional {
                    capability: scope.required_capability().unwrap_or(TXN_MULTI_RECORD),
                    condition: condition.to_string(),
                });
            }
        }
        Err(PortError::UnsupportedCapability {
            capability: scope.required_capability().unwrap_or(TXN_MULTI_RECORD),
            port: self.descriptor.kind,
        })
    }

    /// Resolve a reported commit outcome. A definitive profile
    /// reports its outcome as-is and never reports `Unknown`. An
    /// ambiguous profile resolves `Unknown` only from stable-ID
    /// readback: `Some(true)` when readback found the operation's
    /// writes applied, `Some(false)` when it proved them absent,
    /// and `None` when no readback has happened — in which case the
    /// outcome stays `Unknown`.
    pub fn resolve_outcome(
        &self,
        reported: CommitOutcome,
        readback: Option<bool>,
    ) -> CommitOutcome {
        match (self.commit_ambiguity, reported) {
            (CommitAmbiguity::Definitive, outcome) => outcome,
            (CommitAmbiguity::UnknownUntilReadback, CommitOutcome::Unknown) => match readback {
                Some(true) => CommitOutcome::Committed,
                Some(false) => CommitOutcome::NotCommitted,
                None => CommitOutcome::Unknown,
            },
            (_, outcome) => outcome,
        }
    }
}

impl Port for StoreProfile {
    fn descriptor(&self) -> &PortDescriptor {
        &self.descriptor
    }
}

impl RecordStorePort for StoreProfile {}

/// The native embedded backend: redb, one database file.
///
/// A redb write transaction spans every table of the database and
/// is durable once it commits, so the profile declares the whole
/// database as its atomic scope, unconditionally, and definitive
/// commit outcomes.
pub fn redb_profile() -> StoreProfile {
    StoreProfile {
        descriptor: PortDescriptor {
            kind: PortKind::RecordStore,
            contract_version: PORT_CONTRACT_VERSION,
            resource_id: "redb".to_string(),
            scope: "local".to_string(),
            required_capabilities: Vec::new(),
            optional_capabilities: vec![CapabilityId::SCAN, TXN_MULTI_RECORD, TXN_WHOLE_DATABASE],
            guarantees: SemanticGuarantees {
                ordered: false,
                durable: true,
                atomic_commit: true,
                atomic_outbox: false,
                exactly_once: false,
            },
            limits: PortLimits::default(),
        },
        atomic_scope: AtomicScope::WholeDatabase,
        conditional_atomic_scope: None,
        commit_ambiguity: CommitAmbiguity::Definitive,
        retry_rule: RetryRule::RetryAsNewOperation,
    }
}

/// The hosted backend: Turso / libSQL over HTTP.
///
/// The default write path commits atomically within one namespace.
/// A whole-database commit exists only inside an explicit
/// interactive transaction on a single session against a single
/// database, so that scope is declared as conditional. Commit
/// outcomes are ambiguous on a lost response and resolve only by
/// stable-ID readback.
pub fn turso_profile() -> StoreProfile {
    StoreProfile {
        descriptor: PortDescriptor {
            kind: PortKind::RecordStore,
            contract_version: PORT_CONTRACT_VERSION,
            resource_id: "turso-libsql-http".to_string(),
            scope: "hosted".to_string(),
            required_capabilities: Vec::new(),
            optional_capabilities: vec![CapabilityId::SCAN, TXN_MULTI_RECORD, TXN_WHOLE_DATABASE],
            guarantees: SemanticGuarantees {
                ordered: false,
                durable: true,
                atomic_commit: true,
                atomic_outbox: false,
                exactly_once: false,
            },
            limits: PortLimits::default(),
        },
        atomic_scope: AtomicScope::SingleNamespace,
        conditional_atomic_scope: Some((
            AtomicScope::WholeDatabase,
            "explicit interactive transaction on a single session against a single database",
        )),
        commit_ambiguity: CommitAmbiguity::UnknownUntilReadback,
        retry_rule: RetryRule::ReadbackBeforeRetry,
    }
}
