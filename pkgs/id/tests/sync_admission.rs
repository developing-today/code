//! id-sync retained-data ingress through the fixed admission pipeline
//! (issue #10192, slice 9110-06).
//!
//! Oracle: **model** — the expected admission trace for the
//! id-sync ingress path is `carrier gate (sync domain + chain
//! decode + author binding + B02 revocation consult) → gate →
//! prepare (deterministic order) → reauthorize → commit
//! (receiving-store ingest) → outbox`, evaluated independently of
//! the adapter's internals: these tests drive [`SyncAdmission`]
//! over a real receiving [`EventStore`] and assert the recorded
//! trace and the stored state against the design's invariants:
//!
//! - No hook or local path accepts an unsigned, invalid,
//!   replayed, revoked, or unauthorized retained object: carrier
//!   and gate refusals (forged signature, tampered payload, unknown
//!   signer, wrong tenant, wrong carrier domain, envelope signer who
//!   is not the event author, undecodable payload, revoked key,
//!   disabled peer) happen before any hook runs, and the store holds
//!   nothing for them.
//! - A committed operation re-presented resolves as already
//!   committed — never a second ingest, never a second hook
//!   trace — and its operation ID cannot be rebound to a
//!   different envelope.
//! - Revoked policy blocks commit, and blocks the retry for as
//!   long as the revocation stands.
//! - The store's decided outcomes (accepted, waiting, duplicate)
//!   are recorded outcomes of a committed admission, not commit
//!   failures: a re-presentation resolves by readback.
//!
//! [`SyncAdmission`]: id::sync_admission::SyncAdmission
//! [`EventStore`]: id::event_store::EventStore

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::{Arc, Mutex};

use common::ingest::MemStorage;
use ed25519_dalek::SigningKey;
use id::admission::{
    AdmissionPipeline, HookId, MemoryOperationLedger, MemoryOutboxJournal, OperationId,
    OperationLedger, OutboxEvent, OutboxHook, PrepareDecision, PrepareHook, PrepareInput,
    Reauthorizer,
};
use id::admission_gate::{
    AdmissionContext, AdmissionGate, AdmissionReceipt, AdmissionRejection, GatePolicy,
    ScopeBindingAuthorizer, StaticSignerDirectory,
};
use id::chain::{ChainExt, Event, sign};
use id::event_store::{EventStore, Ingest};
use id::sync_admission::{SYNC_DOMAIN, SyncAdmission, SyncAdmissionOutcome};
use id::waiting::WaitingLimits;
use id_core::id_shape::HashProfile;
use id_core::signed_envelope::{Body, Envelope, PayloadId, SUITE_ED25519};
use id_sync::federation::slices::b02_key_revocation::{
    RevocationReason, RevocationRecord, RevocationRegistry, RevocationSubject,
};
use p2panda_core::{Hash, SeqNum, SigningKey as ChainSigningKey};

const RAW: HashProfile = HashProfile::RawBlake3_256;
const TENANT_A: &str = "tenant-a";
const TENANT_B: &str = "tenant-b";

fn signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn chain_key(seed: u8) -> ChainSigningKey {
    ChainSigningKey::from_bytes(&[seed; 32])
}

fn signed_event(seed: u8, seq: SeqNum, backlink: Option<Hash>) -> Event {
    let key = chain_key(seed);
    let ext = ChainExt {
        kind: "message".to_owned(),
        group: None,
        after: Vec::new(),
        tag: None,
        document: None,
        removes: None,
        note: None,
        link: None,
        checkpoint: None,
        rotation: None,
        chat: None,
        anchor: None,
        thread: None,
    };
    sign(&key, seq, backlink, None, ext).unwrap()
}

fn carrier_envelope(signer_id: &str, seed: u8, tenant: &str, payload: Vec<u8>) -> Envelope {
    let key = signing_key(seed);
    let body = Body {
        domain: SYNC_DOMAIN.to_string(),
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

fn alice_envelope(event: &Event) -> Envelope {
    carrier_envelope("alice", 11, TENANT_A, event.bytes.clone())
}

fn context(principal: &str, tenant: &str) -> AdmissionContext {
    AdmissionContext::new(principal, tenant, "admission:write").unwrap()
}

fn gate() -> AdmissionGate {
    let mut directory = StaticSignerDirectory::new();
    directory.register("alice", signing_key(11).verifying_key().to_bytes());
    directory.register("bob", signing_key(22).verifying_key().to_bytes());
    AdmissionGate::new(
        GatePolicy::admitting([SYNC_DOMAIN.to_string()]),
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
    trace: Arc<Mutex<Vec<String>>>,
}

impl PrepareHook for TracePrepareHook {
    fn id(&self) -> &HookId {
        &self.id
    }

    fn namespace(&self) -> &str {
        "ns/trace"
    }

    fn prepare(&self, input: &PrepareInput<'_>) -> PrepareDecision {
        self.trace
            .lock()
            .unwrap()
            .push(format!("prepare:{}", self.id.as_str()));
        let _ = input;
        PrepareDecision::Proceed
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
    admission: SyncAdmission,
    reauthorizer: Arc<TraceReauthorizer>,
    ledger: Arc<MemoryOperationLedger>,
    revocations: Arc<Mutex<RevocationRegistry>>,
    trace: Arc<Mutex<Vec<String>>>,
    store: EventStore,
    _storage: MemStorage,
}

fn make_rig() -> Rig {
    let trace: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let reauthorizer = Arc::new(TraceReauthorizer {
        trace: trace.clone(),
        revoked: Mutex::new(false),
    });
    let ledger = Arc::new(MemoryOperationLedger::new());
    let revocations = Arc::new(Mutex::new(RevocationRegistry::new()));
    let mut pipeline = AdmissionPipeline::new(
        gate(),
        reauthorizer.clone(),
        ledger.clone(),
        Arc::new(MemoryOutboxJournal::new()),
    );
    pipeline.register_prepare(Arc::new(TracePrepareHook {
        id: HookId::new("a").unwrap(),
        trace: trace.clone(),
    }));
    pipeline.register_outbox(Arc::new(TraceOutboxHook {
        id: HookId::new("audit").unwrap(),
        trace: trace.clone(),
    }));
    let storage = MemStorage::default();
    let store = EventStore::open_with(&storage, WaitingLimits::default(), 0).unwrap();
    let admission = SyncAdmission::new(pipeline, revocations.clone());
    Rig {
        admission,
        reauthorizer,
        ledger,
        revocations,
        trace,
        store,
        _storage: storage,
    }
}

fn op(id: &str) -> OperationId {
    OperationId::new(id).unwrap()
}

fn revoke_key(registry: &Mutex<RevocationRegistry>, key: [u8; 32]) {
    registry
        .lock()
        .unwrap()
        .apply_revocation(RevocationRecord {
            record_id: "rev-key-1".to_owned(),
            subject: RevocationSubject::Key(key),
            effective_from: 0,
            effective_until: None,
            reason: RevocationReason::KeyCompromise,
            recorded_at: 0,
        })
        .unwrap();
}

fn disable_peer(registry: &Mutex<RevocationRegistry>, peer: &str) {
    registry
        .lock()
        .unwrap()
        .apply_revocation(RevocationRecord {
            record_id: "rev-peer-1".to_owned(),
            subject: RevocationSubject::Peer(peer.to_owned()),
            effective_from: 0,
            effective_until: None,
            reason: RevocationReason::PeerCompromise,
            recorded_at: 0,
        })
        .unwrap();
}

/// Every carrier and gate refusal class reaches the adapter as a
/// rejection at the gate stage, with no hook trace and nothing
/// stored: there is no hook or local path around the gate into the
/// receiving store.
#[test]
fn gate_refusals_never_reach_the_store() {
    let event = signed_event(11, 0, None);
    let mut forged = alice_envelope(&event);
    forged.signature[0] ^= 0xff;

    // Same author, different event bytes, but the envelope still
    // commits to the first event's payload: the gate sees a
    // tampered payload.
    let rival = {
        let ext = ChainExt {
            kind: "message".to_owned(),
            group: None,
            after: Vec::new(),
            tag: None,
            document: None,
            removes: None,
            note: None,
            link: None,
            checkpoint: None,
            rotation: None,
            chat: None,
            anchor: None,
            thread: None,
        };
        sign(&chain_key(11), 0, None, Some(b"rival"), ext).unwrap()
    };
    let mut tampered = alice_envelope(&event);
    tampered.payload = rival.bytes.clone();

    let mallory_event = signed_event(33, 0, None);
    let unknown_signer = carrier_envelope("mallory", 33, TENANT_A, mallory_event.bytes.clone());
    let wrong_tenant = carrier_envelope("alice", 11, TENANT_B, event.bytes.clone());

    let wrong_domain = {
        let key = signing_key(11);
        let body = Body {
            domain: "id/other".to_string(),
            context: TENANT_A.to_string(),
            subject: None,
            graph: None,
            predecessor: None,
            signer_id: "alice".to_string(),
            key: key.verifying_key().to_bytes(),
            suite: SUITE_ED25519.to_string(),
            payload_id: PayloadId::of(RAW, &event.bytes).unwrap(),
            payload_length: event.bytes.len() as u64,
        };
        Envelope::sign(body, event.bytes.clone(), &key).unwrap()
    };

    // The envelope signer is bob, the event author is alice.
    let author_mismatch = carrier_envelope("alice", 22, TENANT_A, event.bytes.clone());

    let undecodable = carrier_envelope("alice", 11, TENANT_A, vec![9; 8]);

    let cases: [(&str, Envelope, &'static str, Option<Event>); 7] = [
        (
            "forged signature",
            forged,
            "bad_signature",
            Some(event.clone()),
        ),
        ("tampered payload", tampered, "payload", Some(event.clone())),
        (
            "unknown signer",
            unknown_signer,
            "signer_binding",
            Some(mallory_event),
        ),
        (
            "wrong tenant",
            wrong_tenant,
            "unauthorized",
            Some(event.clone()),
        ),
        (
            "wrong carrier domain",
            wrong_domain,
            "malformed",
            Some(event.clone()),
        ),
        (
            "envelope signer is not the event author",
            author_mismatch,
            "signer_binding",
            Some(event.clone()),
        ),
        ("undecodable payload", undecodable, "malformed", None),
    ];

    for (name, envelope, expected_class, event) in cases {
        let mut rig = make_rig();
        let err = rig
            .admission
            .admit(
                &mut rig.store,
                op("op-refused"),
                &envelope,
                &context("alice", TENANT_A),
                0,
            )
            .unwrap_err();
        assert_eq!(err.stage(), "gate", "{name}");
        assert_eq!(err.class(), expected_class, "{name}");
        assert!(rig.trace.lock().unwrap().is_empty(), "{name}: no hook ran");
        assert_eq!(rig.store.waiting_len(), 0, "{name}: nothing held");
        if let Some(event) = event {
            assert!(
                rig.store.event_by_id(&event.id).unwrap().is_none(),
                "{name}: the event never reached the store"
            );
        }
        assert!(
            rig.ledger.entry(&op("op-refused")).is_none(),
            "{name}: a gate refusal leaves no ledger entry"
        );
    }
}

/// A key the B02 registry revokes, or a peer it disables, is a gate
/// refusal: the carrier consult fails before any hook runs, with no
/// ledger entry and nothing stored.
#[test]
fn revoked_key_and_disabled_peer_are_gate_refusals() {
    let event = signed_event(11, 0, None);
    let envelope = alice_envelope(&event);
    let alice_key = signing_key(11).verifying_key().to_bytes();

    let mut rig = make_rig();
    revoke_key(&rig.revocations, alice_key);
    let err = rig
        .admission
        .admit(
            &mut rig.store,
            op("op-revoked-key"),
            &envelope,
            &context("alice", TENANT_A),
            0,
        )
        .unwrap_err();
    assert_eq!(err.stage(), "gate");
    assert_eq!(err.class(), "policy");
    assert!(rig.trace.lock().unwrap().is_empty(), "no hook ran");
    assert!(
        rig.store.event_by_id(&event.id).unwrap().is_none(),
        "the event never reached the store"
    );
    assert!(
        rig.ledger.entry(&op("op-revoked-key")).is_none(),
        "a revoked key leaves no ledger entry"
    );

    let mut rig = make_rig();
    disable_peer(&rig.revocations, "alice");
    let err = rig
        .admission
        .admit(
            &mut rig.store,
            op("op-disabled-peer"),
            &envelope,
            &context("alice", TENANT_A),
            0,
        )
        .unwrap_err();
    assert_eq!(err.stage(), "gate");
    assert_eq!(err.class(), "policy");
    assert!(rig.trace.lock().unwrap().is_empty(), "no hook ran");
    assert!(
        rig.store.event_by_id(&event.id).unwrap().is_none(),
        "the event never reached the store"
    );
    assert!(
        rig.ledger.entry(&op("op-disabled-peer")).is_none(),
        "a disabled peer leaves no ledger entry"
    );
}

/// The happy path produces the design's trace, and the event is
/// stored and applied by the commit step.
#[test]
fn happy_path_stores_the_event_through_the_pipeline() {
    let mut rig = make_rig();
    let event = signed_event(11, 0, None);
    let envelope = alice_envelope(&event);
    let outcome = rig
        .admission
        .admit(
            &mut rig.store,
            op("op-happy"),
            &envelope,
            &context("alice", TENANT_A),
            0,
        )
        .unwrap();
    let SyncAdmissionOutcome::Committed {
        ingest,
        policy_epoch,
        pending_outbox,
        ..
    } = outcome
    else {
        panic!("the operation commits");
    };
    assert!(
        matches!(ingest, Ingest::Accepted { id, .. } if id == event.id),
        "the store accepted the admitted event: {ingest:?}"
    );
    assert_eq!(policy_epoch, 3);
    assert!(pending_outbox.is_empty());
    assert_eq!(
        *rig.trace.lock().unwrap(),
        ["prepare:a", "reauthorize", "outbox:deliver"],
        "the trace is the design's mandatory order (commit is the receiving-store ingest)"
    );
    assert!(rig.store.event_applied(&event.id).unwrap());
    assert!(rig.store.event_by_id(&event.id).unwrap().is_some());
}

/// A replayed operation never ingests twice and never re-runs
/// the hook trace; rebinding its ID to another envelope is a
/// replay rejection.
#[test]
fn replay_never_double_ingests() {
    let mut rig = make_rig();
    let event = signed_event(11, 0, None);
    let envelope = alice_envelope(&event);
    let first = rig
        .admission
        .admit(
            &mut rig.store,
            op("op-replay"),
            &envelope,
            &context("alice", TENANT_A),
            0,
        )
        .unwrap();
    assert!(matches!(first, SyncAdmissionOutcome::Committed { .. }));
    let trace_after_first = rig.trace.lock().unwrap().clone();

    let second = rig
        .admission
        .admit(
            &mut rig.store,
            op("op-replay"),
            &envelope,
            &context("alice", TENANT_A),
            0,
        )
        .unwrap();
    assert!(matches!(
        second,
        SyncAdmissionOutcome::AlreadyCommitted { .. }
    ));
    assert_eq!(
        rig.trace.lock().unwrap().clone(),
        trace_after_first,
        "a replay produces no new trace"
    );
    assert!(rig.store.event_applied(&event.id).unwrap());

    let other_event = signed_event(11, 1, Some(event.id));
    let other_envelope = alice_envelope(&other_event);
    let err = rig
        .admission
        .admit(
            &mut rig.store,
            op("op-replay"),
            &other_envelope,
            &context("alice", TENANT_A),
            0,
        )
        .unwrap_err();
    assert_eq!(err.class(), "replay");
}

/// Revoked policy blocks the commit — and the retry, for as long
/// as the revocation stands; clearing it lets the same operation
/// through the full lifecycle.
#[test]
fn revoked_policy_blocks_commit_and_retry() {
    let mut rig = make_rig();
    *rig.reauthorizer.revoked.lock().unwrap() = true;
    let event = signed_event(11, 0, None);
    let envelope = alice_envelope(&event);
    for attempt in 0..2 {
        let err = rig
            .admission
            .admit(
                &mut rig.store,
                op("op-revoked"),
                &envelope,
                &context("alice", TENANT_A),
                0,
            )
            .unwrap_err();
        assert_eq!(err.stage(), "reauthorize", "attempt {attempt}");
        assert_eq!(err.class(), "policy", "attempt {attempt}");
    }
    assert!(rig.store.event_by_id(&event.id).unwrap().is_none());

    *rig.reauthorizer.revoked.lock().unwrap() = false;
    let outcome = rig
        .admission
        .admit(
            &mut rig.store,
            op("op-revoked"),
            &envelope,
            &context("alice", TENANT_A),
            0,
        )
        .unwrap();
    let SyncAdmissionOutcome::Committed { ingest, .. } = outcome else {
        panic!("the operation commits once policy is restored");
    };
    assert!(matches!(ingest, Ingest::Accepted { .. }));
}

/// The store's decided outcomes are recorded outcomes of a
/// committed admission, not commit failures: an event that waits
/// for a missing backlink commits as waiting, and the same event
/// under a fresh operation ID commits as a duplicate.
#[test]
fn decided_store_outcomes_are_recorded_not_failed() {
    let mut rig = make_rig();
    let missing = Hash::from([7; 32]);
    let event = signed_event(11, 1, Some(missing));
    let envelope = alice_envelope(&event);
    let outcome = rig
        .admission
        .admit(
            &mut rig.store,
            op("op-waiting"),
            &envelope,
            &context("alice", TENANT_A),
            0,
        )
        .unwrap();
    let SyncAdmissionOutcome::Committed { ingest, .. } = outcome else {
        panic!("a waiting event still commits its admission");
    };
    assert!(
        matches!(ingest, Ingest::Waiting { id, .. } if id == event.id),
        "the store held the event for its missing backlink: {ingest:?}"
    );
    assert!(rig.store.is_waiting(&event.id));

    let first = signed_event(11, 0, None);
    let first_envelope = alice_envelope(&first);
    let outcome = rig
        .admission
        .admit(
            &mut rig.store,
            op("op-first"),
            &first_envelope,
            &context("alice", TENANT_A),
            0,
        )
        .unwrap();
    assert!(matches!(
        outcome,
        SyncAdmissionOutcome::Committed {
            ingest: Ingest::Accepted { .. },
            ..
        }
    ));
    let outcome = rig
        .admission
        .admit(
            &mut rig.store,
            op("op-second"),
            &first_envelope,
            &context("alice", TENANT_A),
            0,
        )
        .unwrap();
    let SyncAdmissionOutcome::Committed { ingest, .. } = outcome else {
        panic!("a duplicate event still commits its admission");
    };
    assert!(
        matches!(ingest, Ingest::Duplicate { id, .. } if id == first.id),
        "the store reported the duplicate: {ingest:?}"
    );
    assert!(rig.store.event_applied(&first.id).unwrap());
}
