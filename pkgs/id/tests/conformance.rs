//! Shared store conformance fixtures (slice 9110-04, #10191) —
//! evidence first, per the issue.
//!
//! Oracle (model, from the issue): adapter observations match
//! reference CRUD, scan, transaction, retry, and
//! capability-negotiation transitions. The verifier is a
//! deterministic abstract state machine (the reference model below)
//! that emits expected state and the capability report; the
//! portable script and the machine-readable capability reports
//! live in `tests/fixtures/port-v1.json`, shared with any future
//! runner (a browser runner for the IndexedDB profile reads the
//! same file).
//!
//! The fixture pins the declarations of slice 9110-03 (#10174): if
//! a conformance run contradicts a declaration, the declaration is
//! the bug (see `port_adapter.rs`). Every step's observation must
//! agree three ways — the simulated adapter driven by the declared
//! profile, the fixture's recorded expectation, and the reference
//! model's independently computed state. The script runs twice per
//! profile and the traces must be identical (determinism).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../crates/id-store/src/port_adapter.rs"]
mod port_adapter;

use std::collections::{BTreeMap, HashSet};

use id_core::port::{CapabilityId, OperationRequest, Port, PortError, PortKind, negotiate};
use port_adapter::{
    AtomicScope, CommitAmbiguity, CommitOutcome, StoreProfile, TXN_MULTI_RECORD,
    TXN_WHOLE_DATABASE, redb_profile, turso_profile,
};
use serde_json::{Value, json};

const FIXTURE: &str = include_str!("fixtures/port-v1.json");

fn fixture() -> Value {
    serde_json::from_str(FIXTURE).expect("port-v1.json parses")
}

/// The capability a fixture string names. The fixture is ours, so
/// an unknown name is a fixture bug and fails loudly.
fn capability(name: &str) -> CapabilityId {
    match name {
        "store.scan" => CapabilityId::SCAN,
        "store.txn.multi-record" => TXN_MULTI_RECORD,
        "store.txn.whole-database" => TXN_WHOLE_DATABASE,
        "transport.signed-receipts" => CapabilityId::SIGNED_RECEIPTS,
        other => panic!("fixture names unknown capability {other}"),
    }
}

fn kind_name(kind: PortKind) -> &'static str {
    match kind {
        PortKind::RecordStore => "record-store",
        PortKind::BlobStore => "blob-store",
        PortKind::EventLog => "event-log",
        PortKind::WorldStore => "world-store",
        PortKind::ObjectBucket => "object-bucket",
        PortKind::MessageStream => "message-stream",
        PortKind::RequestTransport => "request-transport",
    }
}

fn scope_name(scope: AtomicScope) -> &'static str {
    match scope {
        AtomicScope::SingleRecord => "single-record",
        AtomicScope::SingleNamespace => "single-namespace",
        AtomicScope::WholeDatabase => "whole-database",
    }
}

/// The machine-readable capability report the state machine emits
/// for one declared profile, in the fixture's schema.
fn capability_report(profile: &StoreProfile) -> Value {
    let descriptor = profile.descriptor();
    let caps =
        |caps: &[CapabilityId]| -> Vec<String> { caps.iter().map(ToString::to_string).collect() };
    json!({
        "resource_id": descriptor.resource_id,
        "kind": kind_name(descriptor.kind),
        "contract_version": descriptor.contract_version,
        "scope": descriptor.scope,
        "status": "active",
        "required_capabilities": caps(&descriptor.required_capabilities),
        "optional_capabilities": caps(&descriptor.optional_capabilities),
        "guarantees": {
            "ordered": descriptor.guarantees.ordered,
            "durable": descriptor.guarantees.durable,
            "atomic_commit": descriptor.guarantees.atomic_commit,
            "atomic_outbox": descriptor.guarantees.atomic_outbox,
            "exactly_once": descriptor.guarantees.exactly_once,
        },
        "limits": {
            "max_record_bytes": descriptor.limits.max_record_bytes,
            "max_batch_records": descriptor.limits.max_batch_records,
            "max_message_bytes": descriptor.limits.max_message_bytes,
        },
        "atomic_scope": scope_name(profile.atomic_scope()),
        "conditional_atomic_scope": profile.conditional_atomic_scope().map(|(scope, condition)| {
            json!({ "scope": scope_name(scope), "condition": condition })
        }),
        "commit_ambiguity": match profile.commit_ambiguity() {
            CommitAmbiguity::Definitive => "definitive",
            CommitAmbiguity::UnknownUntilReadback => "unknown-until-readback",
        },
        "retry_rule": match profile.retry_rule() {
            port_adapter::RetryRule::RetryAsNewOperation => "retry-as-new-operation",
            port_adapter::RetryRule::ReadbackBeforeRetry => "readback-before-retry",
        },
    })
}

/// One record write inside a commit; `value: None` is a delete.
#[derive(Debug, Clone)]
struct Write {
    namespace: String,
    id: String,
    value: Option<Vec<u8>>,
}

/// The scope a set of writes spans, by the profiles' own terms
/// (same rule as slice 9110-03's suite).
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

/// The deterministic abstract state machine: ground truth about
/// what has actually been applied, keyed by stable record ID, with
/// commits deduplicated by stable operation ID. Independent of
/// either adapter — it follows only the fixture's declared
/// outcomes, never the adapter's bookkeeping.
#[derive(Default)]
struct ReferenceModel {
    records: BTreeMap<(String, String), Vec<u8>>,
    applied_ops: HashSet<String>,
}

impl ReferenceModel {
    fn apply(&mut self, op_id: &str, writes: &[Write]) {
        if self.applied_ops.contains(op_id) {
            return;
        }
        for write in writes {
            let key = (write.namespace.clone(), write.id.clone());
            match &write.value {
                Some(value) => {
                    self.records.insert(key, value.clone());
                }
                None => {
                    self.records.remove(&key);
                }
            }
        }
        self.applied_ops.insert(op_id.to_string());
    }

    fn get(&self, namespace: &str, id: &str) -> Option<Vec<u8>> {
        self.records
            .get(&(namespace.to_string(), id.to_string()))
            .cloned()
    }

    fn scan(&self, namespace: &str) -> Vec<(String, Vec<u8>)> {
        self.records
            .iter()
            .filter(|((ns, _), _)| ns == namespace)
            .map(|((_, id), value)| (id.clone(), value.clone()))
            .collect()
    }
}

/// Faults the injector can arm for the next commit.
#[derive(Default, Clone, Copy)]
struct Faults {
    drop_response_after_apply: bool,
    drop_request_before_apply: bool,
}

/// A simulated adapter: a declared profile in front of its own
/// server state, with the fault injector between caller and
/// server. Network faults are inert for a definitive (local)
/// profile — the commit call's return is the outcome there.
struct SimulatedAdapter {
    profile: StoreProfile,
    server: ReferenceModel,
    faults: Faults,
}

impl SimulatedAdapter {
    fn new(profile: StoreProfile) -> Self {
        Self {
            profile,
            server: ReferenceModel::default(),
            faults: Faults::default(),
        }
    }

    fn commit(
        &mut self,
        op_id: &str,
        writes: &[Write],
        interactive: bool,
    ) -> Result<CommitOutcome, PortError> {
        self.profile.check_commit(scope_of(writes), interactive)?;
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

    fn resolve(&self, op_id: &str, reported: CommitOutcome) -> CommitOutcome {
        let readback = match reported {
            CommitOutcome::Unknown => Some(self.server.applied_ops.contains(op_id)),
            _ => None,
        };
        self.profile.resolve_outcome(reported, readback)
    }

    fn get(&self, namespace: &str, id: &str) -> Option<Vec<u8>> {
        self.server.get(namespace, id)
    }

    fn scan(&self, namespace: &str) -> Vec<(String, Vec<u8>)> {
        self.server.scan(namespace)
    }
}

fn error_kind(error: &PortError) -> &'static str {
    match error {
        PortError::UnsupportedCapability { .. } => "unsupported-capability",
        PortError::Conditional { .. } => "conditional",
        PortError::LimitExceeded { .. } => "limit-exceeded",
        PortError::VersionMismatch { .. } => "version-mismatch",
    }
}

fn outcome_name(outcome: CommitOutcome) -> &'static str {
    match outcome {
        CommitOutcome::Committed => "committed",
        CommitOutcome::NotCommitted => "not-committed",
        CommitOutcome::Unknown => "unknown",
    }
}

fn commit_observation(result: Result<CommitOutcome, PortError>) -> String {
    match result {
        Ok(outcome) => outcome_name(outcome).to_string(),
        Err(error) => format!("refused:{}", error_kind(&error)),
    }
}

fn get_observation(value: Option<Vec<u8>>) -> String {
    match value {
        Some(value) => format!("value:{}", String::from_utf8(value).unwrap()),
        None => "absent".to_string(),
    }
}

fn scan_observation(entries: Vec<(String, Vec<u8>)>) -> String {
    let parts: Vec<String> = entries
        .into_iter()
        .map(|(id, value)| format!("{id}={}", String::from_utf8(value).unwrap()))
        .collect();
    format!("entries:{}", parts.join(","))
}

/// The expectation a step declares for one profile: a string is
/// shared by every profile; an object keys it by resource_id.
fn expect_for(step: &Value, resource_id: &str) -> String {
    match &step["expect"] {
        Value::String(shared) => shared.clone(),
        Value::Object(by_profile) => by_profile
            .get(resource_id)
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("step {step} has no expect for {resource_id}"))
            .to_string(),
        other => panic!("step expect must be a string or object, got {other}"),
    }
}

fn parse_writes(step: &Value) -> Vec<Write> {
    if let Some(writes) = step["writes"].as_array() {
        return writes
            .iter()
            .map(|write| Write {
                namespace: write["namespace"].as_str().unwrap().to_string(),
                id: write["id"].as_str().unwrap().to_string(),
                value: write["value"].as_str().map(|v| v.as_bytes().to_vec()),
            })
            .collect();
    }
    vec![Write {
        namespace: step["namespace"].as_str().unwrap().to_string(),
        id: step["id"].as_str().unwrap().to_string(),
        value: match step["op"].as_str().unwrap() {
            "put" => Some(step["value"].as_str().unwrap().as_bytes().to_vec()),
            "delete" => None,
            other => panic!("no writes for op {other}"),
        },
    }]
}

/// Run the fixture's vector script against one profile and return
/// the observation trace. Every observation must match the
/// fixture's expectation for this profile, and every read must
/// match the reference model, which follows only the fixture's
/// declared outcomes.
fn run_script(profile: StoreProfile, vectors: &[Value]) -> Vec<String> {
    let resource_id = profile.descriptor().resource_id.clone();
    let mut adapter = SimulatedAdapter::new(profile);
    let mut model = ReferenceModel::default();
    let mut trace = Vec::new();

    for (index, step) in vectors.iter().enumerate() {
        let expect = expect_for(step, &resource_id);
        let observation = match step["op"].as_str().unwrap() {
            "put" | "delete" => {
                let writes = parse_writes(step);
                let op_id = step["op_id"].as_str().unwrap();
                let observation = commit_observation(adapter.commit(op_id, &writes, false));
                if expect == "committed" {
                    model.apply(op_id, &writes);
                }
                observation
            }
            "commit_batch" => {
                let writes = parse_writes(step);
                let op_id = step["op_id"].as_str().unwrap();
                let interactive = step["interactive"].as_bool().unwrap_or(false);
                let observation = commit_observation(adapter.commit(op_id, &writes, interactive));
                if expect == "committed" {
                    model.apply(op_id, &writes);
                }
                observation
            }
            "commit_fault" => {
                let writes = parse_writes(step);
                let op_id = step["op_id"].as_str().unwrap();
                adapter.faults = match step["fault"].as_str().unwrap() {
                    "drop-response" => Faults {
                        drop_response_after_apply: true,
                        ..Faults::default()
                    },
                    "drop-request" => Faults {
                        drop_request_before_apply: true,
                        ..Faults::default()
                    },
                    other => panic!("unknown fault {other}"),
                };
                let reported = adapter
                    .commit(op_id, &writes, false)
                    .expect("fault steps stay within declared scope");
                let resolved = adapter.resolve(op_id, reported);
                if resolved == CommitOutcome::Committed {
                    model.apply(op_id, &writes);
                }
                format!(
                    "reported:{},resolved:{}",
                    outcome_name(reported),
                    outcome_name(resolved)
                )
            }
            "get" => {
                let namespace = step["namespace"].as_str().unwrap();
                let id = step["id"].as_str().unwrap();
                let observation = get_observation(adapter.get(namespace, id));
                assert_eq!(
                    observation,
                    get_observation(model.get(namespace, id)),
                    "step {index}: adapter read disagrees with the reference model"
                );
                observation
            }
            "scan" => {
                let namespace = step["namespace"].as_str().unwrap();
                let observation = scan_observation(adapter.scan(namespace));
                assert_eq!(
                    observation,
                    scan_observation(model.scan(namespace)),
                    "step {index}: adapter scan disagrees with the reference model"
                );
                observation
            }
            "negotiate" => {
                let required: Vec<CapabilityId> = step["required"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|name| capability(name.as_str().unwrap()))
                    .collect();
                let request = OperationRequest {
                    required_capabilities: required,
                    protocol_version: step["protocol_version"]
                        .as_u64()
                        .map(|v| v as u32)
                        .unwrap_or(id_core::port::PORT_CONTRACT_VERSION),
                };
                match negotiate(adapter.profile.descriptor(), &request) {
                    Ok(_) => "granted".to_string(),
                    Err(error) => format!("refused:{}", error_kind(&error)),
                }
            }
            other => panic!("unknown op {other}"),
        };
        assert_eq!(
            observation, expect,
            "step {index} ({step}) diverged for profile {resource_id}"
        );
        trace.push(observation);
    }

    // Final state: the adapter's visible state equals the model's
    // in every namespace the script touched.
    let mut namespaces: Vec<String> = vectors
        .iter()
        .filter_map(|step| step["namespace"].as_str().map(str::to_string))
        .collect();
    namespaces.sort();
    namespaces.dedup();
    for namespace in namespaces {
        assert_eq!(
            scan_observation(adapter.scan(&namespace)),
            scan_observation(model.scan(&namespace)),
            "final state diverged in namespace {namespace} for profile {resource_id}"
        );
    }
    trace
}

#[test]
fn capability_reports_match_the_declared_profiles() {
    let fixture = fixture();
    assert_eq!(fixture["fixture_version"], json!(1));
    assert_eq!(
        fixture["contract_version"].as_u64().unwrap(),
        u64::from(id_core::port::PORT_CONTRACT_VERSION)
    );
    let reports = fixture["capability_reports"].as_array().unwrap();
    let profiles = [redb_profile(), turso_profile()];
    let mut matched: HashSet<&str> = HashSet::new();
    for report in reports {
        if report["resource_id"] == "indexeddb" {
            // The TypeScript suite compares this exact report to the actual browser adapter.
            continue;
        }
        assert_eq!(report["status"], "active");
        let resource_id = report["resource_id"].as_str().unwrap();
        let profile = profiles
            .iter()
            .find(|profile| profile.descriptor().resource_id == resource_id)
            .unwrap_or_else(|| panic!("fixture report {resource_id} has no live profile"));
        assert_eq!(
            capability_report(profile),
            *report,
            "capability report for {resource_id} diverged from the declared profile"
        );
        matched.insert(resource_id);
    }
    // Every live profile is pinned by the fixture, and vice versa.
    for profile in &profiles {
        assert!(
            matched.contains(profile.descriptor().resource_id.as_str()),
            "profile {} is not pinned by the fixture",
            profile.descriptor().resource_id
        );
    }
}

#[test]
fn indexeddb_has_an_active_report_and_explicit_vector_expectations() {
    let fixture = fixture();
    let browser_reports: Vec<_> = fixture["capability_reports"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|report| report["resource_id"] == "indexeddb")
        .collect();
    assert_eq!(browser_reports.len(), 1);
    assert_eq!(browser_reports[0]["status"], "active");
    assert_eq!(browser_reports[0]["scope"], "browser");
    for step in fixture["vectors"].as_array().unwrap() {
        if let Some(by_profile) = step["expect"].as_object() {
            assert!(
                by_profile.get("indexeddb").is_some_and(Value::is_string),
                "missing browser expectation for {step}"
            );
        }
    }
}

#[test]
fn declared_scope_agrees_with_commit_gate() {
    // The profile's two scope surfaces — the boolean declaration
    // and the typed commit gate — must never disagree: a scope
    // declared under a condition is exactly the scope the gate
    // admits under that condition.
    for profile in [redb_profile(), turso_profile()] {
        for scope in [
            AtomicScope::SingleRecord,
            AtomicScope::SingleNamespace,
            AtomicScope::WholeDatabase,
        ] {
            for condition_met in [false, true] {
                assert_eq!(
                    profile.declares_scope(scope, condition_met),
                    profile.check_commit(scope, condition_met).is_ok(),
                    "scope surfaces diverged for {} at {scope:?} (condition_met={condition_met})",
                    profile.descriptor().resource_id
                );
            }
        }
    }
}

#[test]
fn conformance_vectors_match_reference_model() {
    let fixture = fixture();
    let vectors = fixture["vectors"].as_array().unwrap();
    assert_ne!(*vectors, [] as [serde_json::Value; 0]);
    for profile in [redb_profile(), turso_profile()] {
        let resource_id = profile.descriptor().resource_id.clone();
        // Determinism: the same script against a fresh adapter and
        // a fresh model emits the identical trace.
        let first = run_script(profile, vectors);
        let profile = match resource_id.as_str() {
            "redb" => redb_profile(),
            "turso-libsql-http" => turso_profile(),
            other => panic!("unknown profile {other}"),
        };
        let second = run_script(profile, vectors);
        assert_eq!(
            first, second,
            "trace for {resource_id} is not deterministic"
        );
        assert_eq!(first.len(), vectors.len());
    }
}
