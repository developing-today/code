//! Ordered lifecycle hooks and the fixed acceptance gate
//! (issue #10192, slice 9110-06).
//!
//! Oracle: **model** — the expected admission trace is
//! `gate → prepare (deterministic order) → reauthorize → commit →
//! outbox`, evaluated independently of the pipeline's internals:
//! these tests drive [`id::admission::AdmissionPipeline`] with
//! recording hooks and a counting commit step, and assert the
//! recorded trace and the committed set against the design's
//! invariants:
//!
//! - No hook or local path accepts an unsigned, invalid,
//!   replayed, or unauthorized retained object: gate refusals
//!   (forged signature, tampered payload, unknown signer,
//!   over-budget payload, wrong tenant) happen before any hook
//!   runs, and no commit follows them.
//! - A committed operation re-presented resolves as already
//!   committed — never a second commit, never a second hook
//!   trace — and its operation ID cannot be rebound to a
//!   different envelope.
//! - Revoked policy blocks commit, and blocks the retry for as
//!   long as the revocation stands.
//! - Prepare annotations are non-authoritative data flowing in
//!   hook order to the commit step; they cannot change any of
//!   the outcomes above (the veto/contract mechanics are pinned
//!   by the unit tests in `id_store::admission`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use ed25519_dalek::SigningKey;
use id::admission::{
    AdmissionPipeline, Annotation, CommitInput, HookId, LifecycleRejection, MemoryOperationLedger,
    MemoryOutboxJournal, OperationId, OperationLedger, OutboxEvent, OutboxHook, PipelineOutcome,
    PrepareDecision, PrepareHook, PrepareInput, Reauthorizer,
};
use id_core::admission_gate::{
    AdmissionContext, AdmissionGate, AdmissionReceipt, AdmissionRejection, GatePolicy,
    ScopeBindingAuthorizer, StaticSignerDirectory,
};
use id_core::id_shape::HashProfile;
use id_core::signed_envelope::{Body, Envelope, PayloadId, SUITE_ED25519};

const RAW: HashProfile = HashProfile::RawBlake3_256;
const DOMAIN: &str = "id/atlas";
const TENANT_A: &str = "tenant-a";
const TENANT_B: &str = "tenant-b";
const MAX_PAYLOAD: u64 = 64;

fn signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn make_envelope(signer_id: &str, seed: u8, tenant: &str, payload: Vec<u8>) -> Envelope {
    let key = signing_key(seed);
    let body = Body {
        domain: DOMAIN.to_string(),
        context: tenant.to_string(),
        subject: None,
        graph: None,
        predecessor: None,
        signer_id: signer_id.to_string(),
        key: key.verifying_key().to_bytes(),
        suite: SUITE_ED25519.to_string(),
        payload_id: PayloadId::of(RAW, &payload).unwrap(),
        payload_length: payload.len() as u64,
    };
    Envelope::sign(body, payload, &key).unwrap()
}

fn alice_envelope(payload: Vec<u8>) -> Envelope {
    make_envelope("alice", 11, TENANT_A, payload)
}

fn context(principal: &str, tenant: &str) -> AdmissionContext {
    AdmissionContext::new(principal, tenant, "admission:write").unwrap()
}

fn gate() -> AdmissionGate {
    let mut directory = StaticSignerDirectory::new();
    directory.register("alice", signing_key(11).verifying_key().to_bytes());
    directory.register("bob", signing_key(22).verifying_key().to_bytes());
    AdmissionGate::new(
        GatePolicy::admitting([DOMAIN.to_string()]).with_max_payload_bytes(MAX_PAYLOAD),
        directory,
        ScopeBindingAuthorizer,
    )
}

/// A reauthorizer that records its place in the trace and can be
/// revoked mid-test.
struct TraceReauthorizer {
    trace: Arc<Mutex<Vec<String>>>,
    revoked: Mutex<bool>,
}

impl Reauthorizer for TraceReauthorizer {
    fn policy_epoch(&self) -> u64 {
        3
    }

    fn reauthorize(
        &self,
        _receipt: &AdmissionReceipt,
        _context: &AdmissionContext,
    ) -> Result<(), AdmissionRejection> {
        self.trace.lock().unwrap().push("reauthorize".to_string());
        if *self.revoked.lock().unwrap() {
            Err(AdmissionRejection::Policy {
                problem: "signer revoked at current policy epoch".to_string(),
            })
        } else {
            Ok(())
        }
    }
}

struct TracePrepareHook {
    id: HookId,
    priority: i32,
    trace: Arc<Mutex<Vec<String>>>,
    annotate: bool,
    saw_annotations: Arc<Mutex<Vec<String>>>,
}

impl PrepareHook for TracePrepareHook {
    fn id(&self) -> &HookId {
        &self.id
    }

    fn namespace(&self) -> &str {
        // One namespace per hook, derived from its ID.
        match self.id.as_str() {
            "a" => "ns/a",
            "b" => "ns/b",
            _ => "ns/trace",
        }
    }

    fn priority(&self) -> i32 {
        self.priority
    }

    fn prepare(&self, input: &PrepareInput<'_>) -> PrepareDecision {
        self.trace
            .lock()
            .unwrap()
            .push(format!("prepare:{}", self.id.as_str()));
        for annotation in input.annotations {
            self.saw_annotations.lock().unwrap().push(format!(
                "{}:{}",
                annotation.namespace(),
                annotation.key()
            ));
        }
        if self.annotate {
            let namespace = match self.id.as_str() {
                "a" => "ns/a",
                _ => "ns/b",
            };
            PrepareDecision::Annotate(vec![
                Annotation::new(namespace, format!("from-{}", self.id.as_str()), vec![1]).unwrap(),
            ])
        } else {
            PrepareDecision::Proceed
        }
    }
}

struct TraceOutboxHook {
    id: HookId,
    trace: Arc<Mutex<Vec<String>>>,
}

impl OutboxHook for TraceOutboxHook {
    fn id(&self) -> &HookId {
        &self.id
    }

    fn deliver(&self, _event: &OutboxEvent) -> Result<(), String> {
        self.trace
            .lock()
            .unwrap()
            .push("outbox:deliver".to_string());
        Ok(())
    }
}

struct Rig {
    pipeline: AdmissionPipeline,
    reauthorizer: Arc<TraceReauthorizer>,
    ledger: Arc<MemoryOperationLedger>,
    trace: Arc<Mutex<Vec<String>>>,
    commits: Arc<Mutex<Vec<Vec<String>>>>,
}

fn rig() -> Rig {
    let trace: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let reauthorizer = Arc::new(TraceReauthorizer {
        trace: trace.clone(),
        revoked: Mutex::new(false),
    });
    let ledger = Arc::new(MemoryOperationLedger::new());
    let pipeline = AdmissionPipeline::new(
        gate(),
        reauthorizer.clone(),
        ledger.clone(),
        Arc::new(MemoryOutboxJournal::new()),
    );
    Rig {
        pipeline,
        reauthorizer,
        ledger,
        trace,
        commits: Arc::new(Mutex::new(Vec::new())),
    }
}

impl Rig {
    fn commit_step(&self) -> impl Fn(&CommitInput<'_>) -> Result<(), String> + use<> {
        let trace = self.trace.clone();
        let commits = self.commits.clone();
        move |input: &CommitInput<'_>| {
            trace.lock().unwrap().push("commit".to_string());
            commits.lock().unwrap().push(
                input
                    .annotations
                    .iter()
                    .map(|a| format!("{}:{}", a.namespace(), a.key()))
                    .collect(),
            );
            Ok(())
        }
    }

    fn add_trace_hook(
        &mut self,
        id: &str,
        priority: i32,
        annotate: bool,
    ) -> Arc<Mutex<Vec<String>>> {
        let saw = Arc::new(Mutex::new(Vec::new()));
        self.pipeline.register_prepare(Arc::new(TracePrepareHook {
            id: HookId::new(id).unwrap(),
            priority,
            trace: self.trace.clone(),
            annotate,
            saw_annotations: saw.clone(),
        }));
        saw
    }
}

fn op(id: &str) -> OperationId {
    OperationId::new(id).unwrap()
}

/// Every gate refusal class reaches the pipeline as a rejection
/// at the gate stage, with no hook trace, no commit, and no
/// ledger entry: there is no hook or local path around the gate.
#[test]
fn gate_refusals_never_reach_hooks_or_commit() {
    let mut forged = alice_envelope(vec![1; 8]);
    forged.signature[0] ^= 0xff;

    let mut tampered = alice_envelope(vec![2; 8]);
    tampered.payload = vec![9; 8];

    let unknown_signer = make_envelope("mallory", 33, TENANT_A, vec![3; 8]);
    let oversized = alice_envelope(vec![7; (MAX_PAYLOAD + 1) as usize]);

    let cases: [(&str, Envelope, &'static str); 4] = [
        ("forged signature", forged, "bad_signature"),
        ("tampered payload", tampered, "payload"),
        ("unknown signer", unknown_signer, "signer_binding"),
        ("oversized payload", oversized, "policy"),
    ];

    for (name, envelope, expected_class) in cases {
        let mut rig = rig();
        rig.add_trace_hook("a", 1, false);
        let err = rig
            .pipeline
            .run(
                op("op-refused"),
                &envelope,
                &context("alice", TENANT_A),
                rig.commit_step(),
            )
            .unwrap_err();
        assert_eq!(err.stage(), "gate", "{name}");
        assert_eq!(err.class(), expected_class, "{name}");
        assert!(rig.trace.lock().unwrap().is_empty(), "{name}: no hook ran");
        assert!(
            rig.commits.lock().unwrap().is_empty(),
            "{name}: no commit ran"
        );
        assert!(
            rig.ledger.entry(&op("op-refused")).is_none(),
            "{name}: a gate refusal leaves no ledger entry"
        );
    }
}

/// A validly signed envelope for another tenant, presented under
/// this tenant's authenticated context, is refused by the gate's
/// tenant authorization — hooks never see it.
#[test]
fn unauthorized_tenant_never_reaches_hooks() {
    let mut rig = rig();
    rig.add_trace_hook("a", 1, false);
    let envelope = make_envelope("alice", 11, TENANT_B, vec![4; 8]);
    let err = rig
        .pipeline
        .run(
            op("op-tenant"),
            &envelope,
            &context("alice", TENANT_A),
            rig.commit_step(),
        )
        .unwrap_err();
    assert_eq!(err.stage(), "gate");
    assert_eq!(err.class(), "unauthorized");
    assert!(rig.trace.lock().unwrap().is_empty());
    assert!(rig.commits.lock().unwrap().is_empty());
}

/// A replayed operation never commits twice and never re-runs
/// the hook trace; rebinding its ID to another envelope is a
/// replay rejection.
#[test]
fn replay_never_double_commits() {
    let mut rig = rig();
    rig.add_trace_hook("a", 1, false);
    let envelope = alice_envelope(vec![5; 8]);
    let first = rig
        .pipeline
        .run(
            op("op-replay"),
            &envelope,
            &context("alice", TENANT_A),
            rig.commit_step(),
        )
        .unwrap();
    assert!(matches!(first, PipelineOutcome::Committed { .. }));
    let trace_after_first = rig.trace.lock().unwrap().clone();

    let second = rig
        .pipeline
        .run(
            op("op-replay"),
            &envelope,
            &context("alice", TENANT_A),
            rig.commit_step(),
        )
        .unwrap();
    assert!(matches!(second, PipelineOutcome::AlreadyCommitted { .. }));
    assert_eq!(
        rig.trace.lock().unwrap().clone(),
        trace_after_first,
        "a replay produces no new trace"
    );
    assert_eq!(rig.commits.lock().unwrap().len(), 1, "exactly one commit");

    let err = rig
        .pipeline
        .run(
            op("op-replay"),
            &alice_envelope(vec![6; 8]),
            &context("alice", TENANT_A),
            rig.commit_step(),
        )
        .unwrap_err();
    assert_eq!(err.class(), "replay");
}

/// Revoked policy blocks the commit — and the retry, for as long
/// as the revocation stands; clearing it lets the same operation
/// through the full lifecycle.
#[test]
fn revoked_policy_blocks_commit_and_retry() {
    let mut rig = rig();
    rig.add_trace_hook("a", 1, false);
    *rig.reauthorizer.revoked.lock().unwrap() = true;
    let envelope = alice_envelope(vec![7; 8]);
    for attempt in 0..2 {
        let err = rig
            .pipeline
            .run(
                op("op-revoked"),
                &envelope,
                &context("alice", TENANT_A),
                rig.commit_step(),
            )
            .unwrap_err();
        assert_eq!(err.stage(), "reauthorize", "attempt {attempt}");
        assert_eq!(err.class(), "policy", "attempt {attempt}");
    }
    assert!(
        rig.commits.lock().unwrap().is_empty(),
        "no commit while policy is revoked"
    );

    *rig.reauthorizer.revoked.lock().unwrap() = false;
    let outcome = rig
        .pipeline
        .run(
            op("op-revoked"),
            &envelope,
            &context("alice", TENANT_A),
            rig.commit_step(),
        )
        .unwrap();
    assert!(matches!(outcome, PipelineOutcome::Committed { .. }));
    assert_eq!(rig.commits.lock().unwrap().len(), 1);
}

/// The happy path produces exactly the design's trace, and the
/// annotations flow in hook order: hook b sees hook a's
/// annotation, and the commit step receives both, in order.
#[test]
fn admission_trace_matches_the_design_order() {
    let mut rig = rig();
    // Registered out of order on purpose: execution order is
    // (priority, registration ID), never registration order.
    let saw_by_b = rig.add_trace_hook("b", 2, true);
    let _saw_by_a = rig.add_trace_hook("a", 1, true);
    rig.pipeline.register_outbox(Arc::new(TraceOutboxHook {
        id: HookId::new("audit").unwrap(),
        trace: rig.trace.clone(),
    }));

    let outcome = rig
        .pipeline
        .run(
            op("op-trace"),
            &alice_envelope(vec![8; 8]),
            &context("alice", TENANT_A),
            rig.commit_step(),
        )
        .unwrap();
    let PipelineOutcome::Committed {
        annotations,
        pending_outbox,
        policy_epoch,
        ..
    } = outcome
    else {
        panic!("the operation commits");
    };
    assert_eq!(
        *rig.trace.lock().unwrap(),
        [
            "prepare:a",
            "prepare:b",
            "reauthorize",
            "commit",
            "outbox:deliver"
        ],
        "the trace is the design's mandatory order"
    );
    assert_eq!(
        *saw_by_b.lock().unwrap(),
        ["ns/a:from-a"],
        "a later hook sees earlier hooks' annotations, read-only"
    );
    assert_eq!(
        rig.commits.lock().unwrap()[0],
        ["ns/a:from-a", "ns/b:from-b"],
        "the commit step receives the annotations in hook order"
    );
    assert_eq!(annotations.len(), 2);
    assert_eq!(pending_outbox, [] as [HookId; 0]);
    assert_eq!(policy_epoch, 3);
}

/// A rejection's stage and class vocabulary is stable for the
/// oracle: lifecycle failures do not borrow gate classes they did
/// not come from.
#[test]
fn rejection_vocabulary_is_stable() {
    let replay = LifecycleRejection::Replay {
        operation_id: op("op-x"),
        problem: "bound to a different envelope".to_string(),
    };
    assert_eq!(replay.stage(), "replay");
    assert_eq!(replay.class(), "replay");

    let gate = LifecycleRejection::Gate(AdmissionRejection::BadSignature);
    assert_eq!(gate.stage(), "gate");
    assert_eq!(gate.class(), "bad_signature");
}
