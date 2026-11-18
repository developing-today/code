//! B09 model test: divergence detection and anti-entropy (#10262,
//! statements S1.32-S1.40).
//!
//! Oracle: **model** — a reference inventory delta detects divergence,
//! every resolution follows a conflict report, and the order of
//! reconciliation does not silently merge. The reference is written
//! independently in this file: it records every version a writer
//! signed, computes each replica's expected version sets, the expected
//! divergence and digests from the documented inventory layout, and
//! the heads of each object by brute force. Random streams interleave
//! writes on random replicas with reconciliation between random pairs;
//! after every step each replica must hold only signed versions, keep
//! an open report exactly while an object has more than one head, and
//! show every merge after a report on its object. A final full sync
//! must leave every replica with the reference union and its heads,
//! whatever order the rounds ran in, while a private-scope object never
//! leaves its replica.
//!
//! Every slice module is loaded by path: the federation roots record
//! them without declaring them (slice 9110-02), and the slices reach
//! each other through `super::`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/federation/slices/b32_ingest_gate.rs"]
mod b32_ingest_gate;

#[path = "../src/federation/slices/b31_peer_classes.rs"]
#[allow(dead_code)]
mod b31_peer_classes;

#[path = "../src/federation/slices/b08_sync_scope.rs"]
#[allow(dead_code)]
mod b08_sync_scope;

#[path = "../src/federation/slices/b07_conflict_report.rs"]
#[allow(dead_code)]
mod b07_conflict_report;

#[path = "../src/federation/slices/b09_anti_entropy.rs"]
mod b09_anti_entropy;

use std::collections::{BTreeMap, BTreeSet};

use b07_conflict_report::{
    AuditEntry, ConflictStore, IngestError, IngestOutcome, ObjectId, RetentionPolicy,
};
use b08_sync_scope::{Exclusion, SyncScopeConfig};
use b09_anti_entropy::{Inventory, ObjectDivergence, Refusal, Replica, divergence, reconcile};
use b32_ingest_gate::{
    Envelope, EventId, FreshnessWindow, RejectReason, SignerAuthorization, sign_envelope,
};
use ed25519_dalek::SigningKey;
use proptest::prelude::*;
use sha2::{Digest, Sha256};

const START: u64 = 10_000;
const WIDE: FreshnessWindow = FreshnessWindow {
    max_age: 1_000_000,
    max_future_skew: 30,
};
const RETENTION: RetentionPolicy = RetentionPolicy { retain_for: 3_600 };
const SIGNERS: [(&str, u8); 3] = [("alice", 7), ("bob", 9), ("carol", 11)];
const REPLICAS: usize = 4;
const SHARED: &str = "sync";
const PRIVATE: &str = "private";
const DOMAIN: &str = "fleet";
/// Objects 0..3 are in the shared scope; object 3 is private.
const OBJECTS: u8 = 4;

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn signers() -> Vec<SignerAuthorization> {
    SIGNERS
        .iter()
        .map(|(identity, seed)| SignerAuthorization {
            identity: (*identity).to_owned(),
            verifying_key: key(*seed).verifying_key().to_bytes(),
            scopes: [SHARED, PRIVATE].iter().map(|s| (*s).to_owned()).collect(),
        })
        .collect()
}

fn replica(n: usize, freshness: FreshnessWindow) -> Replica {
    Replica::new(
        format!("r{n}"),
        DOMAIN,
        ConflictStore::new(signers(), freshness, RETENTION),
    )
}

fn config() -> SyncScopeConfig {
    SyncScopeConfig::new(SHARED, DOMAIN)
}

fn sha(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

const fn object(n: u8) -> ObjectId {
    [0xB0 | n; 32]
}

const fn scope_of(n: u8) -> &'static str {
    if n == OBJECTS - 1 { PRIVATE } else { SHARED }
}

fn event(n: u32) -> EventId {
    let mut id = [0_u8; 32];
    // Spread ids so ascending order is not production order.
    id[..4].copy_from_slice(&n.wrapping_mul(0x9E37_79B9).to_be_bytes());
    id[4..8].copy_from_slice(&n.to_be_bytes());
    id[31] = 0x09;
    id
}

/// The B07 wire layout, written independently.
fn payload(
    object_id: ObjectId,
    resolves: Option<[u8; 32]>,
    parents: &[EventId],
    body: &[u8],
) -> Vec<u8> {
    let mut sorted = parents.to_vec();
    sorted.sort_unstable();
    let mut out = b"id-sync/b07-version/v1".to_vec();
    out.extend_from_slice(&object_id);
    match resolves {
        None => out.push(0),
        Some(id) => {
            out.push(1);
            out.extend_from_slice(&id);
        }
    }
    out.extend_from_slice(&u32::try_from(sorted.len()).unwrap().to_be_bytes());
    for parent in sorted {
        out.extend_from_slice(&parent);
    }
    out.extend_from_slice(body);
    out
}

/// The B07 conflict key, written independently.
fn key_of(ids: &[EventId]) -> [u8; 32] {
    let distinct: BTreeSet<&EventId> = ids.iter().collect();
    let mut out = b"id-sync/b07-conflict-key/v1".to_vec();
    out.extend_from_slice(&u64::try_from(distinct.len()).unwrap().to_be_bytes());
    for id in distinct {
        out.extend_from_slice(id);
    }
    sha(&out)
}

/// The inventory digest from the slice contract, written independently.
fn digest_of(object_id: &ObjectId, ids: &BTreeSet<EventId>) -> [u8; 32] {
    let mut out = b"id-sync/b09-inventory/v1".to_vec();
    out.extend_from_slice(object_id);
    out.extend_from_slice(&u64::try_from(ids.len()).unwrap().to_be_bytes());
    for id in ids {
        out.extend_from_slice(id);
    }
    sha(&out)
}

fn version(
    signer: usize,
    event_id: EventId,
    object_n: u8,
    parents: &[EventId],
    body: &[u8],
    issued_at: u64,
) -> Envelope {
    let (identity, seed) = SIGNERS[signer];
    let resolves = (parents.len() >= 2).then(|| key_of(parents));
    sign_envelope(
        &key(seed),
        identity,
        scope_of(object_n),
        &payload(object(object_n), resolves, parents, body),
        event_id,
        issued_at,
    )
}

/// What a writer signed: the object, its parents and the replica it
/// was produced on.
struct Signed {
    object_id: ObjectId,
    parents: Vec<EventId>,
    origin: usize,
}

/// The reference: every signed version.
#[derive(Default)]
struct Reference {
    signed: BTreeMap<EventId, Signed>,
}

impl Reference {
    /// The heads of a version set, by brute force.
    fn heads(&self, ids: &BTreeSet<EventId>) -> BTreeSet<EventId> {
        let named: BTreeSet<EventId> = ids
            .iter()
            .flat_map(|id| self.signed[id].parents.iter().copied())
            .collect();
        ids.difference(&named).copied().collect()
    }

    /// Every signed version of an object.
    fn all(&self, object_id: &ObjectId) -> BTreeSet<EventId> {
        self.signed
            .iter()
            .filter(|(_, s)| s.object_id == *object_id)
            .map(|(id, _)| *id)
            .collect()
    }
}

/// What a replica holds of one object, read from its store directly.
fn held_by(replica: &Replica, reference: &Reference, object_id: &ObjectId) -> BTreeSet<EventId> {
    reference
        .signed
        .iter()
        .filter(|(id, s)| {
            s.object_id == *object_id
                && (replica.store().version(id).is_some() || replica.store().is_held(id))
        })
        .map(|(id, _)| *id)
        .collect()
}

fn stored_by(replica: &Replica, reference: &Reference, object_id: &ObjectId) -> BTreeSet<EventId> {
    reference
        .all(object_id)
        .into_iter()
        .filter(|id| replica.store().version(id).is_some())
        .collect()
}

/// The checks that hold on every replica after every step.
fn check_replica(replica: &Replica, reference: &Reference, n: usize) {
    let store = replica.store();
    let inventory = replica.inventory();
    for object_id in inventory.objects() {
        // Reconciliation never creates a version: everything held was
        // signed by a writer.
        for id in inventory.versions(object_id).unwrap() {
            let signed = reference.signed.get(id).expect("a version nobody signed");
            assert_eq!(signed.object_id, *object_id);
        }
    }
    for o in 0..OBJECTS {
        let object_id = object(o);
        let held = held_by(replica, reference, &object_id);
        assert_eq!(
            inventory.versions(&object_id).cloned().unwrap_or_default(),
            held
        );
        // The private object never leaves its origin.
        if scope_of(o) == PRIVATE {
            assert!(held.iter().all(|id| reference.signed[id].origin == n));
        }
        // Heads are exactly the stored versions nobody names, and an
        // object has an open report exactly while it has two or more.
        let stored = stored_by(replica, reference, &object_id);
        let heads: BTreeSet<EventId> = store.heads(&object_id).into_iter().collect();
        assert_eq!(heads, reference.heads(&stored));
        assert_eq!(store.open_report(&object_id).is_some(), heads.len() > 1);
    }
    // Every merge comes after a report on its object.
    let audit = store.audit();
    for (i, entry) in audit.iter().enumerate() {
        if let AuditEntry::Recorded {
            object_id,
            event_id,
        } = entry
        {
            let version = store.version(event_id).unwrap();
            if version.resolves.is_some() {
                assert!(
                    audit[..i].iter().any(|e| matches!(
                        e,
                        AuditEntry::ReportCreated { object_id: o, .. } if o == object_id
                    )),
                    "merge recorded before any report on its object"
                );
            }
        }
    }
    assert!(replica.refused().is_empty(), "{:?}", replica.refused());
}

/// The divergence the reference predicts between two replicas.
fn expected_divergence(a: &Replica, b: &Replica, reference: &Reference) -> Vec<ObjectDivergence> {
    (0..OBJECTS)
        .filter_map(|o| {
            let object_id = object(o);
            let ours = held_by(a, reference, &object_id);
            let theirs = held_by(b, reference, &object_id);
            (ours != theirs).then(|| ObjectDivergence {
                object_id,
                missing_local: theirs.difference(&ours).copied().collect(),
                missing_remote: ours.difference(&theirs).copied().collect(),
            })
        })
        .collect()
}

fn check_summary(inventory: &Inventory) {
    for (object_id, digest) in inventory.summary() {
        assert_eq!(
            digest,
            digest_of(&object_id, inventory.versions(&object_id).unwrap())
        );
    }
}

#[derive(Debug, Clone, Copy)]
enum Step {
    /// A writer on `replica` signs a version of `object`. `mode` picks
    /// the parents from that replica's heads: 0 merges them all, 1 edits
    /// the first, 2 starts a new root.
    Write {
        replica: usize,
        signer: usize,
        object: u8,
        mode: u8,
        body: u8,
    },
    /// Reconcile two replicas.
    Sync { first: usize, second: usize },
}

fn step_strategy() -> impl Strategy<Value = Step> {
    prop_oneof![
        (
            0..REPLICAS,
            0..SIGNERS.len(),
            0..OBJECTS,
            prop_oneof![3 => Just(0_u8), 3 => Just(1_u8), 1 => Just(2_u8)],
            any::<u8>(),
        )
            .prop_map(|(replica, signer, object, mode, body)| Step::Write {
                replica,
                signer,
                object,
                mode,
                body,
            }),
        (0..REPLICAS, 0..REPLICAS)
            .prop_filter("two replicas", |(a, b)| a != b)
            .prop_map(|(first, second)| Step::Sync { first, second }),
    ]
}

const fn pair(replicas: &mut [Replica], a: usize, b: usize) -> (&mut Replica, &mut Replica) {
    if a < b {
        let (left, right) = replicas.split_at_mut(b);
        (&mut left[a], &mut right[0])
    } else {
        let (left, right) = replicas.split_at_mut(a);
        (&mut right[0], &mut left[b])
    }
}

fn run(steps: &[Step], final_order: &[(usize, usize)]) {
    let mut replicas: Vec<Replica> = (0..REPLICAS).map(|n| replica(n, WIDE)).collect();
    let mut reference = Reference::default();
    let mut clock = START;
    let mut counter = 0_u32;

    for step in steps {
        clock += 1;
        match *step {
            Step::Write {
                replica,
                signer,
                object: o,
                mode,
                body,
            } => {
                let object_id = object(o);
                let heads = replicas[replica].store().heads(&object_id);
                let parents: Vec<EventId> = match mode {
                    0 => heads,
                    1 => heads.into_iter().take(1).collect(),
                    _ => Vec::new(),
                };
                counter += 1;
                let event_id = event(counter);
                let envelope = version(signer, event_id, o, &parents, &[body], clock);
                reference.signed.insert(
                    event_id,
                    Signed {
                        object_id,
                        parents: parents.clone(),
                        origin: replica,
                    },
                );
                let outcome = replicas[replica].produce(&envelope, clock).unwrap();
                if parents.len() >= 2 {
                    // A writer's merge of every head closes the report.
                    assert!(matches!(outcome, IngestOutcome::Resolved(_)));
                }
            }
            Step::Sync { first, second } => {
                let expected = expected_divergence(&replicas[first], &replicas[second], &reference);
                let (a, b) = pair(&mut replicas, first, second);
                check_summary(&a.inventory());
                assert_eq!(divergence(&a.inventory(), &b.inventory()), expected);
                let exchange = reconcile(a, b, &config(), clock);
                assert_eq!(exchange.divergence, expected);

                // Each side sends exactly the shared-scope versions the
                // other lacks, ascending; private ones are withheld.
                let to_second: Vec<EventId> = expected
                    .iter()
                    .flat_map(|d| d.missing_remote.iter().copied())
                    .collect();
                let to_first: Vec<EventId> = expected
                    .iter()
                    .flat_map(|d| d.missing_local.iter().copied())
                    .collect();
                for (transfer, missing) in [
                    (&exchange.forward, to_second),
                    (&exchange.backward, to_first),
                ] {
                    let mut send: Vec<EventId> = missing
                        .iter()
                        .filter(|id| reference.signed[*id].object_id != object(OBJECTS - 1))
                        .copied()
                        .collect();
                    send.sort_unstable();
                    let mut withheld: Vec<(EventId, Exclusion)> = missing
                        .iter()
                        .filter(|id| reference.signed[*id].object_id == object(OBJECTS - 1))
                        .map(|id| (*id, Exclusion::OutsideScope))
                        .collect();
                    withheld.sort_unstable();
                    assert_eq!(transfer.offer.send, send);
                    assert_eq!(transfer.offer.withheld, withheld);
                    assert!(transfer.offer.refused_by_remote.is_empty());
                    assert_eq!(transfer.outcomes.len(), send.len());
                    assert!(transfer.outcomes.iter().all(|(_, r)| r.is_ok()));
                }
            }
        }
        for (n, r) in replicas.iter().enumerate() {
            check_replica(r, &reference, n);
        }
    }

    // Full sync, in the order given, until nothing moves.
    let mut rounds = 0;
    loop {
        clock += 1;
        let mut quiet = true;
        for &(first, second) in final_order {
            let (a, b) = pair(&mut replicas, first, second);
            quiet &= reconcile(a, b, &config(), clock).is_quiet();
        }
        rounds += 1;
        if quiet {
            break;
        }
        assert!(rounds < 2 * REPLICAS, "full sync did not settle");
    }

    for (n, r) in replicas.iter().enumerate() {
        check_replica(r, &reference, n);
        for o in 0..OBJECTS {
            let object_id = object(o);
            let expected: BTreeSet<EventId> = if scope_of(o) == PRIVATE {
                reference
                    .all(&object_id)
                    .into_iter()
                    .filter(|id| reference.signed[id].origin == n)
                    .collect()
            } else {
                reference.all(&object_id)
            };
            // Shared objects converge to the union, whatever the order
            // of rounds was, with no version held and the same heads.
            if scope_of(o) == SHARED {
                assert_eq!(stored_by(r, &reference, &object_id), expected);
                let heads: BTreeSet<EventId> = r.store().heads(&object_id).into_iter().collect();
                assert_eq!(heads, reference.heads(&expected));
            } else {
                assert_eq!(held_by(r, &reference, &object_id), expected);
            }
        }
    }
    // Every pair now agrees on every shared object.
    for a in 0..REPLICAS {
        for b in 0..REPLICAS {
            if a != b {
                let d = divergence(&replicas[a].inventory(), &replicas[b].inventory());
                assert!(d.iter().all(|d| d.object_id == object(OBJECTS - 1)));
            }
        }
    }
}

fn order_strategy() -> impl Strategy<Value = Vec<(usize, usize)>> {
    let pairs: Vec<(usize, usize)> = (0..REPLICAS)
        .flat_map(|a| (0..REPLICAS).filter(move |b| *b != a).map(move |b| (a, b)))
        .collect();
    Just(pairs).prop_shuffle()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn reconciliation_detects_divergence_reports_first_and_converges(
        steps in prop::collection::vec(step_strategy(), 1..60),
        final_order in order_strategy(),
    ) {
        run(&steps, &final_order);
    }
}

#[test]
fn a_concurrent_edit_is_reported_on_both_sides_and_other_objects_keep_syncing() {
    let mut a = replica(0, WIDE);
    let mut b = replica(1, WIDE);
    let doc = 0;
    let other = 1;
    let base = event(1);
    a.produce(&version(0, base, doc, &[], b"base", START), START)
        .unwrap();
    let first = reconcile(&mut a, &mut b, &config(), START);
    assert_eq!(first.forward.offer.send, vec![base]);

    // Both sides edit the same base; one side also edits another object.
    a.produce(&version(0, event(2), doc, &[base], b"a", START), START)
        .unwrap();
    b.produce(&version(1, event(3), doc, &[base], b"b", START), START)
        .unwrap();
    a.produce(&version(0, event(4), other, &[], b"x", START), START)
        .unwrap();

    let exchange = reconcile(&mut a, &mut b, &config(), START + 1);
    assert_eq!(
        exchange.divergence,
        vec![
            ObjectDivergence {
                object_id: object(doc),
                missing_local: vec![event(3)],
                missing_remote: vec![event(2)],
            },
            ObjectDivergence {
                object_id: object(other),
                missing_local: vec![],
                missing_remote: vec![event(4)],
            },
        ]
    );
    // The conflict is a report on both sides, naming the same versions,
    // and the unrelated object synced in the same exchange.
    let heads = vec![event(2), event(3)];
    for side in [&a, &b] {
        let report = side.store().open_report(&object(doc)).unwrap();
        let stored = side.store().report(&report).unwrap();
        assert_eq!(stored.report.conflict_key(), key_of(&heads));
        assert_eq!(side.store().heads(&object(other)), vec![event(4)]);
        assert!(side.store().open_report(&object(other)).is_none());
    }
    // Nothing was merged by reconciling.
    assert_eq!(a.store().heads(&object(doc)).len(), 2);

    // A writer on b merges; the next exchange resolves a's report too.
    b.produce(&version(1, event(5), doc, &heads, b"merged", START), START)
        .unwrap();
    let report_a = a.store().open_report(&object(doc)).unwrap();
    let exchange = reconcile(&mut a, &mut b, &config(), START + 2);
    assert_eq!(
        exchange.backward.outcomes,
        vec![(event(5), Ok(IngestOutcome::Resolved(report_a)))]
    );
    assert_eq!(a.store().heads(&object(doc)), vec![event(5)]);
    assert!(reconcile(&mut a, &mut b, &config(), START + 3).is_quiet());
}

#[test]
fn a_refused_version_is_never_offered_again_and_its_children_wait() {
    let short = FreshnessWindow {
        max_age: 10,
        max_future_skew: 30,
    };
    let mut a = replica(0, short);
    let mut b = replica(1, short);
    let doc = 0;
    a.produce(&version(0, event(1), doc, &[], b"old", START), START)
        .unwrap();
    // The child is fresh, the parent is stale by the time they meet.
    let later = START + 20;
    a.produce(
        &version(0, event(2), doc, &[event(1)], b"new", later),
        later,
    )
    .unwrap();
    let exchange = reconcile(&mut a, &mut b, &config(), later);
    let mut sent = vec![event(1), event(2)];
    sent.sort_unstable();
    assert_eq!(exchange.forward.offer.send, sent);
    assert!(
        exchange
            .forward
            .outcomes
            .contains(&(event(1), Err(IngestError::Rejected(RejectReason::Stale))))
    );
    assert!(b.store().is_held(&event(2)));
    assert_eq!(
        b.refused().get(&event(1)),
        Some(&Refusal::Ingest(IngestError::Rejected(RejectReason::Stale)))
    );

    // The divergence stays visible, but nothing is sent again.
    let again = reconcile(&mut a, &mut b, &config(), later + 1);
    assert!(again.is_quiet());
    assert_eq!(again.forward.offer.refused_by_remote, vec![event(1)]);
    assert_eq!(
        again.divergence,
        vec![ObjectDivergence {
            object_id: object(doc),
            missing_local: vec![],
            missing_remote: vec![event(1)],
        }]
    );
}

#[test]
fn out_of_bounds_versions_are_withheld_with_the_b08_reason() {
    let mut a = replica(0, WIDE);
    let mut b = replica(1, WIDE);
    let private = OBJECTS - 1;
    a.produce(&version(0, event(1), private, &[], b"p", START), START)
        .unwrap();
    a.produce(&version(0, event(2), 0, &[], b"s", START), START)
        .unwrap();

    // Another scope never crosses; a version past retention neither.
    let exchange = reconcile(&mut a, &mut b, &config().with_retention(5), START + 10);
    assert_eq!(exchange.forward.offer.withheld, {
        let mut w = vec![
            (event(1), Exclusion::OutsideScope),
            (event(2), Exclusion::ExpiredRetention),
        ];
        w.sort_unstable();
        w
    });
    assert!(exchange.forward.offer.send.is_empty());
    assert!(b.inventory().objects().next().is_none());

    // Within retention the shared version crosses; the private one never.
    let exchange = reconcile(&mut a, &mut b, &config(), START + 10);
    assert_eq!(exchange.forward.offer.send, vec![event(2)]);
    assert_eq!(
        exchange.forward.offer.withheld,
        vec![(event(1), Exclusion::OutsideScope)]
    );
}

#[test]
fn equal_sets_have_equal_digests_and_report_no_divergence() {
    let mut a = replica(0, WIDE);
    let mut b = replica(1, WIDE);
    let envelope = version(0, event(1), 0, &[], b"v", START);
    a.produce(&envelope, START).unwrap();
    b.produce(&envelope, START).unwrap();
    assert_eq!(a.inventory().summary(), b.inventory().summary());
    assert!(divergence(&a.inventory(), &b.inventory()).is_empty());
    assert!(reconcile(&mut a, &mut b, &config(), START).is_quiet());
    let ids = BTreeSet::from([event(1)]);
    assert_eq!(
        a.inventory().summary()[&object(0)],
        digest_of(&object(0), &ids)
    );
    assert_eq!(a.envelope(&event(1)), Some(&envelope));
    assert!(a.inventory().contains(&event(1)));
}
