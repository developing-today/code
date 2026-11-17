//! Integration test for slice B21 — short-circuit and deduplicate
//! in-flight/answered requests into coherent results (issue #10210;
//! statements S1.67–S1.68).
//!
//! Oracle: **model**. The expected canonical encodings and
//! coherent-set digests are built by an independent reference
//! construction in this file (written field by field from the
//! canonical form in the design, not by calling the module's own
//! encoder), and the coordinator's behaviour over generated op
//! sequences is checked against an independent reference state
//! machine below. The issue's oracle sentence — "Concurrent
//! duplicate request attaches once; stable result IDs deduplicate
//! while retaining each provenance proof" — is pinned by
//! `oracle_sentence_from_the_issue`.
//!
//! The module under test is included by path: slice 9110-02
//! (#10160) recorded the module in the federation registry without
//! exporting it, and slice 9110-09 wires the versioned public
//! exports after module outputs land, so this test must not edit
//! shared roots either.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/federation/slices/b21_result_dedup.rs"]
mod b21;

use std::collections::BTreeMap;

use b21::{
    DedupError, PeerKey, Registration, RequestCoordinator, RequestId, ReservationId, ResultId,
    ResultSet, ShortCircuit,
};
use sha2::{Digest, Sha256};

// ---------- independent reference construction ----------

fn key(byte: u8) -> [u8; 32] {
    [byte; 32]
}

fn reference_entry_bytes(
    result_id: &ResultId,
    body_digest: &[u8; 32],
    provenance: &BTreeMap<PeerKey, Vec<u8>>,
) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(result_id);
    v.extend_from_slice(body_digest);
    v.extend_from_slice(&(provenance.len() as u32).to_be_bytes());
    for (responder, proof) in provenance {
        v.extend_from_slice(responder);
        v.extend_from_slice(&(proof.len() as u32).to_be_bytes());
        v.extend_from_slice(proof);
    }
    v
}

fn reference_set_digest(entries: &[Vec<u8>]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"id-atlas/b21/result-set/v1");
    for e in entries {
        h.update(e);
    }
    h.finalize().into()
}

// ---------- independent reference state machine ----------

#[derive(Clone, Debug, PartialEq, Eq)]
enum RefState {
    InFlight {
        reservation: ReservationId,
        attachments: u32,
    },
    Done {
        reservation: ReservationId,
        results: Vec<(ResultId, [u8; 32], Vec<PeerKey>)>,
    },
    Evicted {
        reservation: ReservationId,
    },
}

/// Reference model: mirrors the design bullets directly (attach,
/// cache, evict, expire) with no shared code with the module.
#[derive(Default)]
struct RefModel {
    items: BTreeMap<RequestId, (RefState, u64)>,
}

#[derive(Debug, PartialEq, Eq)]
enum RefVerdict {
    New,
    Attached(ReservationId, u32),
    Cached(Vec<(ResultId, [u8; 32], Vec<PeerKey>)>),
    Evicted(ReservationId),
    Expired,
    LedgerFull,
}

impl RefModel {
    fn observe(
        &mut self,
        req: RequestId,
        res: ReservationId,
        expires_at: u64,
        now: u64,
        capacity: usize,
    ) -> RefVerdict {
        if let Some((state, exp)) = self.items.get_mut(&req) {
            if now >= *exp {
                return RefVerdict::Expired;
            }
            return match state {
                RefState::InFlight {
                    reservation,
                    attachments,
                } => {
                    *attachments += 1;
                    RefVerdict::Attached(*reservation, *attachments)
                }
                RefState::Done { results, .. } => RefVerdict::Cached(results.clone()),
                RefState::Evicted { reservation } => RefVerdict::Evicted(*reservation),
            };
        }
        if now >= expires_at {
            return RefVerdict::Expired;
        }
        // Expired items are pruned only when the ledger is full:
        // live state is never evicted and below capacity nothing is
        // dropped, so a stale entry can still answer `complete`.
        if self.items.len() >= capacity {
            self.items.retain(|_, (_, exp)| now < *exp);
            if self.items.len() >= capacity {
                return RefVerdict::LedgerFull;
            }
        }
        self.items.insert(
            req,
            (
                RefState::InFlight {
                    reservation: res,
                    attachments: 0,
                },
                expires_at,
            ),
        );
        RefVerdict::New
    }

    fn complete(
        &mut self,
        req: RequestId,
        res: ReservationId,
        results: Vec<(ResultId, [u8; 32], Vec<PeerKey>)>,
    ) -> Result<(), DedupError> {
        let Some((state, _)) = self.items.get_mut(&req) else {
            return Err(DedupError::UnknownWork);
        };
        let state_reservation = match state {
            RefState::InFlight { reservation, .. }
            | RefState::Done { reservation, .. }
            | RefState::Evicted { reservation } => *reservation,
        };
        if state_reservation != res {
            return Err(DedupError::ReservationMismatch);
        }
        match state {
            RefState::InFlight { .. } => {
                *state = RefState::Done {
                    reservation: res,
                    results,
                };
                Ok(())
            }
            RefState::Done { results: old, .. } if *old == results => Ok(()),
            _ => Err(DedupError::OutcomeConflict),
        }
    }

    fn evict(&mut self, req: &RequestId, now: u64) -> bool {
        match self.items.get_mut(req) {
            Some((state, exp)) if now < *exp && matches!(state, RefState::Done { .. }) => {
                let reservation = match state {
                    RefState::Done { reservation, .. } => *reservation,
                    _ => unreachable!(),
                };
                *state = RefState::Evicted { reservation };
                true
            }
            _ => false,
        }
    }
}

fn to_ref(results: &ResultSet) -> Vec<(ResultId, [u8; 32], Vec<PeerKey>)> {
    results
        .entries()
        .map(|e| {
            (
                e.result_id,
                e.body_digest,
                e.provenance.keys().copied().collect(),
            )
        })
        .collect()
}

// ---------- oracle sentence from the issue ----------

#[test]
fn oracle_sentence_from_the_issue() {
    // "Concurrent duplicate request attaches once; stable result
    // IDs deduplicate while retaining each provenance proof."
    let req = key(1);
    let res = key(2);
    let mut c = RequestCoordinator::new(8);

    assert_eq!(
        c.observe(req, res, 100, 0),
        ShortCircuit::NewWork { reservation: res }
    );
    // Three concurrent duplicates all attach to the one work item.
    for n in 1..=3 {
        assert_eq!(
            c.observe(req, key(9), 100, 0),
            ShortCircuit::Attached {
                reservation: res,
                attachments: n
            }
        );
    }

    // Two responders attest the same stable result ID: one entry,
    // both provenance proofs retained.
    let mut set = ResultSet::new();
    let id = key(42);
    let body = key(7);
    assert_eq!(
        set.register(id, body, key(10), b"proof-a".to_vec()),
        Registration::New
    );
    assert_eq!(
        set.register(id, body, key(11), b"proof-b".to_vec()),
        Registration::Merged
    );
    assert_eq!(set.len(), 1);
    assert_eq!(set.get(&id).unwrap().provenance.len(), 2);
    assert_eq!(set.get(&id).unwrap().provenance[&key(10)], b"proof-a");
    assert_eq!(set.get(&id).unwrap().provenance[&key(11)], b"proof-b");

    // A different body under the same stable ID contradicts; the
    // first body stands and nothing is added.
    assert_eq!(
        set.register(id, key(8), key(12), b"proof-c".to_vec()),
        Registration::ConflictingBody
    );
    assert_eq!(set.get(&id).unwrap().body_digest, body);
    assert!(!set.get(&id).unwrap().provenance.contains_key(&key(12)));

    c.complete(req, res, set.clone()).unwrap();
    assert_eq!(
        c.observe(req, key(9), 100, 1),
        ShortCircuit::Cached { results: set }
    );
}

// ---------- canonical form and digest against reference ----------

#[test]
fn canonical_bytes_and_digest_match_reference_construction() {
    let id1 = key(1);
    let id2 = key(2);
    let mut set = ResultSet::new();
    set.register(id2, key(20), key(30), vec![0xAA, 0xBB]);
    set.register(id2, key(20), key(31), vec![0xCC]);
    set.register(id1, key(10), key(30), vec![0x01, 0x02, 0x03]);

    let mut ref_entries = Vec::new();
    for (rid, body, prov) in [
        (id1, key(10), vec![(key(30), vec![1u8, 2, 3])]),
        (
            id2,
            key(20),
            vec![(key(30), vec![0xAA, 0xBB]), (key(31), vec![0xCC])],
        ),
    ] {
        let m: BTreeMap<PeerKey, Vec<u8>> = prov.into_iter().collect();
        ref_entries.push(reference_entry_bytes(&rid, &body, &m));
    }
    assert_eq!(set.digest(), reference_set_digest(&ref_entries));

    // Entries are exposed in ascending result-id order.
    let ids: Vec<ResultId> = set.entries().map(|e| e.result_id).collect();
    assert_eq!(ids, vec![id1, id2]);

    // Empty set digest is the domain separator alone.
    assert_eq!(ResultSet::new().digest(), reference_set_digest(&[]));
}

// ---------- eviction and expiry ----------

#[test]
fn evicted_completed_work_reports_eviction_and_never_reruns() {
    let req = key(3);
    let res = key(4);
    let mut c = RequestCoordinator::new(8);
    c.observe(req, res, 100, 0);
    let mut set = ResultSet::new();
    set.register(key(5), key(6), key(7), vec![1]);
    c.complete(req, res, set).unwrap();

    assert!(c.evict_results(&req, 10));
    // The same reservation is reported; not a fresh one, not rerun.
    assert_eq!(
        c.observe(req, key(99), 100, 20),
        ShortCircuit::ResultEvicted { reservation: res }
    );
    // After expiry the query must not rerun at all.
    assert_eq!(c.observe(req, res, 100, 100), ShortCircuit::Expired);
    // Completing an evicted item does not create a new outcome.
    assert_eq!(
        c.complete(req, res, ResultSet::new()),
        Err(DedupError::OutcomeConflict)
    );
    // Eviction is refused once the request has expired.
    assert!(!c.evict_results(&req, 150));
}

#[test]
fn completion_guards() {
    let req = key(8);
    let res = key(9);
    let mut c = RequestCoordinator::new(8);
    assert_eq!(
        c.complete(req, res, ResultSet::new()),
        Err(DedupError::UnknownWork)
    );
    c.observe(req, res, 100, 0);
    assert_eq!(
        c.complete(req, key(10), ResultSet::new()),
        Err(DedupError::ReservationMismatch)
    );
    let mut a = ResultSet::new();
    a.register(key(11), key(12), key(13), vec![]);
    let mut b = ResultSet::new();
    b.register(key(14), key(15), key(16), vec![]);
    c.complete(req, res, a.clone()).unwrap();
    // Same set again is idempotent; a different set conflicts.
    c.complete(req, res, a).unwrap();
    assert_eq!(c.complete(req, res, b), Err(DedupError::OutcomeConflict));
}

#[test]
fn ledger_is_bounded_and_never_evicts_live_items() {
    let mut c = RequestCoordinator::new(2);
    assert_eq!(
        c.observe(key(1), key(1), 100, 0),
        ShortCircuit::NewWork {
            reservation: key(1)
        }
    );
    assert_eq!(
        c.observe(key(2), key(2), 100, 0),
        ShortCircuit::NewWork {
            reservation: key(2)
        }
    );
    // Full of live items: refuse rather than evict live state.
    assert_eq!(c.observe(key(3), key(3), 100, 0), ShortCircuit::LedgerFull);
    // Once expired, pruning frees capacity.
    assert_eq!(c.prune(100), 2);
    assert!(c.is_empty());
    assert_eq!(
        c.observe(key(3), key(3), 200, 100),
        ShortCircuit::NewWork {
            reservation: key(3)
        }
    );
}

// ---------- reauthorization before release ----------

#[test]
fn cached_results_are_reauthorized_before_release() {
    let req = key(21);
    let res = key(22);
    let mut c = RequestCoordinator::new(4);
    c.observe(req, res, 100, 0);
    let mut set = ResultSet::new();
    set.register(key(23), key(24), key(25), vec![1]);
    set.register(key(23), key(24), key(26), vec![2]);
    c.complete(req, res, set).unwrap();

    let cached = c.observe(req, res, 100, 1);
    // All responders allowed: the cached set is released unchanged.
    assert!(matches!(
        cached.clone().reauthorized(|_| true),
        ShortCircuit::Cached { .. }
    ));
    // A revoked responder blocks disclosure of the cached result.
    assert_eq!(
        cached.reauthorized(|k| *k != key(26)),
        ShortCircuit::ReauthorizationDenied
    );
}

// ---------- generated op sequences against the reference model ----------

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[test]
fn generated_sequences_match_reference_state_machine() {
    let capacity = 3usize;
    for seed in 0..40u64 {
        let mut rng = Lcg(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xB21);
        let mut model = RefModel::default();
        let mut impl_ = RequestCoordinator::new(capacity);
        let mut now = 0u64;

        for _ in 0..60 {
            now += rng.below(6);
            let req = key((rng.below(4) + 1) as u8);
            let res = key((rng.below(3) + 10) as u8);
            let expires_at = 20 + rng.below(10);

            match rng.below(10) {
                0..=5 => {
                    let expected = model.observe(req, res, expires_at, now, capacity);
                    let actual = impl_.observe(req, res, expires_at, now);
                    match (&expected, &actual) {
                        (RefVerdict::New, ShortCircuit::NewWork { .. }) => {}
                        (
                            RefVerdict::Attached(er, ea),
                            ShortCircuit::Attached {
                                reservation,
                                attachments,
                            },
                        ) => {
                            assert_eq!((er, ea), (reservation, attachments), "seed {seed}");
                        }
                        (RefVerdict::Cached(er), ShortCircuit::Cached { results }) => {
                            assert_eq!(*er, to_ref(results), "seed {seed}");
                        }
                        (RefVerdict::Evicted(er), ShortCircuit::ResultEvicted { reservation }) => {
                            assert_eq!(er, reservation, "seed {seed}");
                        }
                        (RefVerdict::Expired, ShortCircuit::Expired) => {}
                        (RefVerdict::LedgerFull, ShortCircuit::LedgerFull) => {}
                        _ => panic!("seed {seed}: model {expected:?} != impl {actual:?}"),
                    }
                }
                6..=8 => {
                    let mut set = ResultSet::new();
                    let rid = key((rng.below(2) + 30) as u8);
                    let body = key((rng.below(2) + 40) as u8);
                    let responder = key((rng.below(2) + 50) as u8);
                    set.register(rid, body, responder, vec![1, 2]);
                    let ref_results: Vec<_> = vec![(rid, body, vec![responder])];
                    let expected = model.complete(req, res, ref_results);
                    let actual = impl_.complete(req, res, set);
                    assert_eq!(expected, actual, "seed {seed}");
                }
                _ => {
                    let expected = model.evict(&req, now);
                    let actual = impl_.evict_results(&req, now);
                    assert_eq!(expected, actual, "seed {seed}");
                }
            }
        }
    }
}
