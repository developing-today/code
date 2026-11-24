//! Store adapter profile conformance tests (slice 9110-03, #10174)
//! — evidence first, per the issue.
//!
//! Oracle (model, from the issue): redb and Turso profiles report
//! only declared transaction scopes; an injected lost response
//! remains unknown until stable-ID readback. The verifier is an
//! abstract store state machine plus a fault injector in this
//! file, independent of either adapter: the simulated adapters
//! below are driven by the *declared profiles* and checked against
//! the reference state machine's ground truth, never against their
//! own bookkeeping.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../crates/id-store/src/port_adapter.rs"]
mod port_adapter;

use std::collections::{BTreeMap, HashSet};

use id_core::port::{CapabilityId, OperationRequest, PortError, PortKind, negotiate};
use port_adapter::{
    AtomicScope, CommitAmbiguity, CommitOutcome, RetryRule, StoreProfile, TXN_WHOLE_DATABASE,
    redb_profile, turso_profile,
};

/// One record write inside a commit.
#[derive(Debug, Clone)]
struct Write {
    namespace: String,
    id: String,
    value: Vec<u8>,
}

fn w(namespace: &str, id: &str, value: &[u8]) -> Write {
    Write {
        namespace: namespace.to_string(),
        id: id.to_string(),
        value: value.to_vec(),
    }
}

/// The scope a set of writes spans, by the profile's own terms:
/// more than one namespace is a whole-database commit; more than
/// one record in one namespace is a single-namespace commit.
fn scope_of(writes: &[Write]) -> AtomicScope {
    let namespaces: HashSet<&str> = writes
        .iter()
        .map(|write| write.namespace.as_str())
        .collect();
    if namespaces.len() > 1 {
        AtomicScope::WholeDatabase
    } else if writes.len() > 1 {
        AtomicScope::SingleNamespace
    } else {
        AtomicScope::SingleRecord
    }
}

/// The abstract store state machine: ground truth about what has
/// actually been applied, keyed by stable record ID, with commits
/// deduplicated by stable operation ID.
#[derive(Default)]
struct ReferenceStore {
    records: BTreeMap<(String, String), Vec<u8>>,
    applied_ops: HashSet<String>,
}

impl ReferenceStore {
    fn apply(&mut self, op_id: &str, writes: &[Write]) {
        if self.applied_ops.contains(op_id) {
            return;
        }
        for write in writes {
            self.records.insert(
                (write.namespace.clone(), write.id.clone()),
                write.value.clone(),
            );
        }
        self.applied_ops.insert(op_id.to_string());
    }

    fn get(&self, namespace: &str, id: &str) -> Option<Vec<u8>> {
        self.records
            .get(&(namespace.to_string(), id.to_string()))
            .cloned()
    }

    fn op_applied(&self, op_id: &str) -> bool {
        self.applied_ops.contains(op_id)
    }

    fn is_empty(&self) -> bool {
        self.records.is_empty() && self.applied_ops.is_empty()
    }
}

/// Faults the injector can arm for the next commit.
#[derive(Default, Clone, Copy)]
struct Faults {
    /// The server applies the commit but the response is lost.
    drop_response_after_apply: bool,
    /// The request is lost before the server applies anything.
    drop_request_before_apply: bool,
}

/// A simulated adapter: a declared profile in front of the
/// reference store, with the fault injector between caller and
/// server. It knows nothing about outcomes except through the
/// profile's declared ambiguity rule.
struct SimulatedAdapter {
    profile: StoreProfile,
    server: ReferenceStore,
    faults: Faults,
    /// Whether the caller met the profile's conditional-scope
    /// condition (an explicit interactive transaction).
    interactive: bool,
}

impl SimulatedAdapter {
    fn new(profile: StoreProfile) -> Self {
        Self {
            profile,
            server: ReferenceStore::default(),
            faults: Faults::default(),
            interactive: false,
        }
    }

    fn commit(&mut self, op_id: &str, writes: &[Write]) -> Result<CommitOutcome, PortError> {
        // Declared-scope check first: refusal happens before any
        // side effect, per the port contract.
        self.profile
            .check_commit(scope_of(writes), self.interactive)?;
        // The injector models a network hop between caller and
        // server. A definitive (local) adapter has no such hop —
        // the commit call's return is the outcome — so injected
        // network faults cannot manufacture an unknown outcome
        // for it, and the same fault script is inert there.
        let remote = self.profile.commit_ambiguity() == CommitAmbiguity::UnknownUntilReadback;
        if remote && self.faults.drop_request_before_apply {
            self.faults.drop_request_before_apply = false;
            return Ok(CommitOutcome::Unknown);
        }
        self.server.apply(op_id, writes);
        if remote && self.faults.drop_response_after_apply {
            self.faults.drop_response_after_apply = false;
            return Ok(CommitOutcome::Unknown);
        }
        Ok(CommitOutcome::Committed)
    }

    /// Resolve a reported outcome the only way the profile allows:
    /// stable-ID readback against ground truth.
    fn resolve(&self, op_id: &str, reported: CommitOutcome) -> CommitOutcome {
        let readback = match reported {
            CommitOutcome::Unknown => Some(self.server.op_applied(op_id)),
            _ => None,
        };
        self.profile.resolve_outcome(reported, readback)
    }

    fn read(&self, namespace: &str, id: &str) -> Option<Vec<u8>> {
        self.server.get(namespace, id)
    }
}

fn operation(required: Vec<CapabilityId>) -> OperationRequest {
    OperationRequest {
        required_capabilities: required,
        protocol_version: id_core::port::PORT_CONTRACT_VERSION,
    }
}

#[test]
fn redb_profile_declares_whole_database_scope_unconditionally() {
    let profile = redb_profile();
    let descriptor = id_core::port::Port::descriptor(&profile);
    assert_eq!(descriptor.kind, PortKind::RecordStore);
    assert_eq!(
        descriptor.contract_version,
        id_core::port::PORT_CONTRACT_VERSION
    );
    assert!(descriptor.guarantees.durable);
    assert!(descriptor.guarantees.atomic_commit);
    assert!(!descriptor.guarantees.exactly_once);
    assert_eq!(profile.atomic_scope(), AtomicScope::WholeDatabase);
    assert_eq!(profile.conditional_atomic_scope(), None);
    assert_eq!(profile.commit_ambiguity(), CommitAmbiguity::Definitive);
    assert_eq!(profile.retry_rule(), RetryRule::RetryAsNewOperation);
    // The whole-database scope needs no condition.
    assert!(profile.declares_scope(AtomicScope::WholeDatabase, false));
    assert_eq!(
        profile.check_commit(AtomicScope::WholeDatabase, false),
        Ok(())
    );
}

#[test]
fn turso_profile_declares_conditional_scope_and_unknown_outcome() {
    let profile = turso_profile();
    let descriptor = id_core::port::Port::descriptor(&profile);
    assert_eq!(descriptor.kind, PortKind::RecordStore);
    assert!(descriptor.guarantees.atomic_commit);
    assert!(!descriptor.guarantees.exactly_once);
    assert_eq!(profile.atomic_scope(), AtomicScope::SingleNamespace);
    let (scope, condition) = profile
        .conditional_atomic_scope()
        .expect("conditional scope");
    assert_eq!(scope, AtomicScope::WholeDatabase);
    assert!(condition.contains("interactive transaction"));
    assert_eq!(
        profile.commit_ambiguity(),
        CommitAmbiguity::UnknownUntilReadback
    );
    assert_eq!(profile.retry_rule(), RetryRule::ReadbackBeforeRetry);
    // Whole-database is declared only when the condition is met,
    // and the refusal is typed Conditional, never a silent narrowing.
    assert!(!profile.declares_scope(AtomicScope::WholeDatabase, false));
    assert!(profile.declares_scope(AtomicScope::WholeDatabase, true));
    match profile.check_commit(AtomicScope::WholeDatabase, false) {
        Err(PortError::Conditional { capability, .. }) => {
            assert_eq!(capability, TXN_WHOLE_DATABASE);
        }
        other => panic!("expected Conditional refusal, got {other:?}"),
    }
    assert_eq!(
        profile.check_commit(AtomicScope::WholeDatabase, true),
        Ok(())
    );
}

#[test]
fn profiles_do_not_claim_identical_atomicity() {
    let (redb, turso) = (redb_profile(), turso_profile());
    assert_ne!(redb.atomic_scope(), turso.atomic_scope());
    assert_ne!(redb.commit_ambiguity(), turso.commit_ambiguity());
    assert_ne!(redb.retry_rule(), turso.retry_rule());
}

#[test]
fn profiles_negotiate_only_what_they_advertise() {
    for profile in [redb_profile(), turso_profile()] {
        let descriptor = id_core::port::Port::descriptor(&profile);
        let granted = negotiate(descriptor, &operation(vec![TXN_WHOLE_DATABASE])).unwrap();
        assert!(granted.grants(TXN_WHOLE_DATABASE));
        let granted = negotiate(descriptor, &operation(vec![CapabilityId::SCAN])).unwrap();
        assert!(granted.grants(CapabilityId::SCAN));
        // A capability neither backend advertises is refused.
        let refused = negotiate(descriptor, &operation(vec![CapabilityId::SIGNED_RECEIPTS]));
        assert!(matches!(
            refused,
            Err(PortError::UnsupportedCapability { .. })
        ));
    }
}

#[test]
fn commits_report_only_declared_scopes() {
    let cross_namespace = vec![w("groups", "g1", b"a"), w("tags", "t1", b"b")];
    let one_namespace = vec![w("groups", "g1", b"a"), w("groups", "g2", b"b")];

    // redb: a whole-database commit is in scope unconditionally.
    let mut redb = SimulatedAdapter::new(redb_profile());
    assert_eq!(
        redb.commit("op-redb-1", &cross_namespace),
        Ok(CommitOutcome::Committed)
    );
    assert_eq!(redb.read("tags", "t1"), Some(b"b".to_vec()));

    // Turso without the interactive-transaction condition: the
    // same commit is refused before any side effect.
    let mut turso = SimulatedAdapter::new(turso_profile());
    let refused = turso.commit("op-turso-1", &cross_namespace);
    assert!(matches!(refused, Err(PortError::Conditional { .. })));
    assert!(turso.server.is_empty(), "refusal must precede side effects");
    assert!(!turso.server.op_applied("op-turso-1"));

    // Turso with the condition met: the commit is in scope.
    turso.interactive = true;
    assert_eq!(
        turso.commit("op-turso-1", &cross_namespace),
        Ok(CommitOutcome::Committed)
    );
    assert_eq!(turso.read("tags", "t1"), Some(b"b".to_vec()));

    // Both backends commit a single-namespace batch on the
    // default path.
    let mut turso = SimulatedAdapter::new(turso_profile());
    assert_eq!(
        turso.commit("op-turso-2", &one_namespace),
        Ok(CommitOutcome::Committed)
    );
    let mut redb = SimulatedAdapter::new(redb_profile());
    assert_eq!(
        redb.commit("op-redb-2", &one_namespace),
        Ok(CommitOutcome::Committed)
    );
}

#[test]
fn injected_lost_response_remains_unknown_until_stable_id_readback() {
    let mut turso = SimulatedAdapter::new(turso_profile());
    turso.faults.drop_response_after_apply = true;
    let writes = vec![w("groups", "g1", b"a")];
    let reported = turso.commit("op-lost-response", &writes).unwrap();
    // The response was lost after the server applied the commit:
    // the caller may state only Unknown — not Committed, and not
    // rolled back.
    assert_eq!(reported, CommitOutcome::Unknown);
    // Without readback it stays unknown.
    assert_eq!(
        turso.profile.resolve_outcome(reported, None),
        CommitOutcome::Unknown
    );
    // Stable-ID readback resolves it to ground truth.
    assert_eq!(
        turso.resolve("op-lost-response", reported),
        CommitOutcome::Committed
    );
    assert_eq!(turso.read("groups", "g1"), Some(b"a".to_vec()));
}

#[test]
fn injected_lost_request_resolves_not_committed_and_retries_by_readback() {
    let mut turso = SimulatedAdapter::new(turso_profile());
    turso.faults.drop_request_before_apply = true;
    let writes = vec![w("groups", "g1", b"a")];
    let reported = turso.commit("op-lost-request", &writes).unwrap();
    assert_eq!(reported, CommitOutcome::Unknown);
    // Readback proves the operation absent: NotCommitted, and the
    // profile's rule is readback before any retry.
    assert_eq!(
        turso.resolve("op-lost-request", reported),
        CommitOutcome::NotCommitted
    );
    assert_eq!(turso.profile.retry_rule(), RetryRule::ReadbackBeforeRetry);
    // Retry under the same stable operation ID applies exactly once.
    assert_eq!(
        turso.commit("op-lost-request", &writes),
        Ok(CommitOutcome::Committed)
    );
    assert_eq!(
        turso.commit("op-lost-request", &writes),
        Ok(CommitOutcome::Committed)
    );
    assert_eq!(turso.read("groups", "g1"), Some(b"a".to_vec()));
    assert!(turso.server.op_applied("op-lost-request"));
}

#[test]
fn redb_outcomes_are_definitive_under_the_same_fault_script() {
    let mut redb = SimulatedAdapter::new(redb_profile());
    redb.faults = Faults {
        drop_response_after_apply: true,
        drop_request_before_apply: true,
    };
    let writes = vec![w("groups", "g1", b"a"), w("tags", "t1", b"b")];
    // A local commit has no response to lose: the outcome is
    // decided by the call itself and is never Unknown.
    let reported = redb.commit("op-redb-faults", &writes).unwrap();
    assert_eq!(reported, CommitOutcome::Committed);
    assert_eq!(
        redb.resolve("op-redb-faults", reported),
        CommitOutcome::Committed
    );
    assert_eq!(redb.read("tags", "t1"), Some(b"b".to_vec()));
}
