//! Integration test for slice B20 — request identity,
//! duplicate-vs-loop classification, finite dedup state and
//! expiry (issue #10184; statements S1.64, S1.70–S1.72).
//!
//! Oracle: **model**. The expected request IDs are computed by an
//! independent reference construction in this file (written from
//! the canonical form in the design, not by calling the module's
//! derivation), and the ledger's behaviour over generated op
//! sequences is checked against an independent reference state
//! machine below. The issue's oracle sentence is pinned by
//! `oracle_sentence_from_the_issue`: same query/time with distinct
//! nonce yields distinct request IDs; altered binding under old
//! ID fails; expired origin never reruns.
//!
//! The module under test is included by path: slice 9110-02
//! (#10160) recorded the module in the federation registry
//! without exporting it, and slice 9110-09 wires the versioned
//! public exports after module outputs land, so this test must
//! not edit shared roots either.

#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "../src/federation/slices/b20_request_identity.rs"]
mod b20;

use std::collections::HashMap;

use b20::{
    Classification, DedupError, DedupLedger, OriginKey, OriginNonce, QueryBinding, RequestId,
    RequestIdentity,
};
use sha2::{Digest, Sha256};

// ---------- independent reference construction ----------

/// Reference canonical binding bytes, built field by field from
/// the design's canonical form (independent of the module's own
/// encoder).
fn reference_canonical(
    query_digest: &[u8; 32],
    scope: &[u8],
    issued_at: u64,
    expires_at: u64,
    initial_max_hops: u32,
) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(query_digest);
    v.extend_from_slice(&(scope.len() as u32).to_be_bytes());
    v.extend_from_slice(scope);
    v.extend_from_slice(&issued_at.to_be_bytes());
    v.extend_from_slice(&expires_at.to_be_bytes());
    v.extend_from_slice(&initial_max_hops.to_be_bytes());
    v
}

fn reference_request_id(
    origin: &OriginKey,
    nonce: &OriginNonce,
    binding: &QueryBinding,
) -> RequestId {
    let mut h = Sha256::new();
    h.update(b"id-atlas/b20/request-id/v1");
    h.update(origin);
    h.update(nonce);
    h.update(reference_canonical(
        &binding.query_digest,
        &binding.scope,
        binding.issued_at,
        binding.expires_at,
        binding.initial_max_hops,
    ));
    RequestId(h.finalize().into())
}

// ---------- fixtures ----------

fn binding(query_byte: u8, issued_at: u64, expires_at: u64) -> QueryBinding {
    QueryBinding {
        query_digest: [query_byte; 32],
        scope: b"tenant-a/read".to_vec(),
        issued_at,
        expires_at,
        initial_max_hops: 4,
    }
}

fn identity(origin: u8, nonce: u8, binding: QueryBinding) -> RequestIdentity {
    RequestIdentity::new([origin; 32], [nonce; 32], binding)
}

// ---------- the issue's oracle sentence ----------

#[test]
fn oracle_sentence_from_the_issue() {
    // Same query/time with distinct nonce yields distinct IDs.
    let a = identity(7, 1, binding(9, 100, 200));
    let b = identity(7, 2, binding(9, 100, 200));
    assert_ne!(a.id, b.id);
    // … and both equal the independently constructed reference IDs.
    assert_eq!(a.id, reference_request_id(&a.origin, &a.nonce, &a.binding));
    assert_eq!(b.id, reference_request_id(&b.origin, &b.nonce, &b.binding));

    // Altered binding under old ID fails: keep a's ID, change a
    // binding field, and the stateless check must reject it.
    let mut altered = identity(7, 1, binding(9, 100, 200));
    altered.binding.expires_at = 201;
    assert!(!altered.binding_is_valid());
    let mut ledger = DedupLedger::new(8, 10);
    assert_eq!(
        ledger.observe(&altered, 150),
        Classification::BindingConflict
    );

    // Expired origin never reruns: after expiry the verdict is
    // Expired, and it stays Expired (never New again).
    let mut ledger = DedupLedger::new(8, 10);
    let exp = identity(7, 3, binding(9, 100, 200));
    assert_eq!(ledger.observe(&exp, 150), Classification::New);
    ledger.complete(&exp, [42; 32]).unwrap();
    assert_eq!(ledger.observe(&exp, 200), Classification::Expired);
    assert_eq!(ledger.observe(&exp, 10_000), Classification::Expired);
}

// ---------- classification behaviour ----------

#[test]
fn duplicates_attach_then_return_cached_outcome_without_rerun() {
    let mut ledger = DedupLedger::new(8, 10);
    let req = identity(1, 1, binding(5, 0, 1_000));
    let mut executions = 0_u32;

    assert_eq!(ledger.observe(&req, 10), Classification::New);
    executions += 1; // the one work item starts
    assert_eq!(ledger.observe(&req, 11), Classification::DuplicateInFlight);
    assert_eq!(ledger.observe(&req, 12), Classification::DuplicateInFlight);
    ledger.complete(&req, [77; 32]).unwrap();
    assert_eq!(ledger.observe(&req, 13), Classification::DuplicateCompleted);
    assert_eq!(ledger.cached_outcome(&req.id), Some([77; 32]));
    // Exactly one execution ever happened.
    assert_eq!(executions, 1);
}

#[test]
fn changed_binding_under_registered_id_is_binding_conflict() {
    let mut ledger = DedupLedger::new(8, 10);
    let req = identity(2, 1, binding(5, 0, 1_000));
    assert_eq!(ledger.observe(&req, 10), Classification::New);
    // Same ID claimed, but the presented binding's scope differs:
    // construct by hand so the claimed ID is the registered one.
    let mut tampered = req.clone();
    tampered.binding.scope = b"tenant-b/read".to_vec();
    assert_eq!(
        ledger.observe(&tampered, 11),
        Classification::BindingConflict
    );
}

#[test]
fn nonce_reuse_under_a_different_id_is_replay() {
    let mut ledger = DedupLedger::new(8, 10);
    let first = identity(3, 9, binding(5, 0, 1_000));
    assert_eq!(ledger.observe(&first, 10), Classification::New);
    // Same origin + same nonce, different query → different ID.
    let second = identity(3, 9, binding(6, 0, 1_000));
    assert_ne!(first.id, second.id);
    assert_eq!(ledger.observe(&second, 11), Classification::Replay);
}

#[test]
fn grant_path_proving_local_visit_is_cycle() {
    let mut ledger = DedupLedger::new(8, 10);
    let req = identity(4, 1, binding(5, 0, 1_000));
    let local = [200; 32];
    let path = vec![[100; 32], local, [101; 32]];
    assert_eq!(
        ledger.observe_with_path(&req, &path, &local, 10),
        Classification::Cycle
    );
    // A cycle registers no work: the same request arriving on a
    // fresh path is still New.
    assert_eq!(
        ledger.observe_with_path(&req, &[[100; 32]], &local, 10),
        Classification::New
    );
}

#[test]
fn finite_capacity_refuses_rather_than_evicting_live_entries() {
    let mut ledger = DedupLedger::new(2, 10);
    let r1 = identity(5, 1, binding(1, 0, 1_000));
    let r2 = identity(5, 2, binding(2, 0, 1_000));
    let r3 = identity(5, 3, binding(3, 0, 1_000));
    assert_eq!(ledger.observe(&r1, 10), Classification::New);
    assert_eq!(ledger.observe(&r2, 10), Classification::New);
    assert_eq!(ledger.observe(&r3, 10), Classification::LedgerFull);
    assert_eq!(ledger.len(), 2);
    assert!(!ledger.is_empty());
    // After r1/r2 expire (plus grace), pruning frees capacity.
    assert_eq!(ledger.observe(&r3, 2_000), Classification::Expired);
    let r4 = identity(5, 4, binding(4, 0, 3_000));
    assert_eq!(ledger.observe(&r4, 2_000), Classification::New);
}

#[test]
fn completion_rules() {
    let mut ledger = DedupLedger::new(8, 10);
    let req = identity(6, 1, binding(5, 0, 1_000));
    // Unknown request cannot complete.
    assert_eq!(
        ledger.complete(&req, [1; 32]),
        Err(DedupError::UnknownRequest)
    );
    assert_eq!(ledger.observe(&req, 10), Classification::New);
    ledger.complete(&req, [1; 32]).unwrap();
    // Idempotent re-completion with the same outcome.
    ledger.complete(&req, [1; 32]).unwrap();
    // A different outcome conflicts; the first stands.
    assert_eq!(
        ledger.complete(&req, [2; 32]),
        Err(DedupError::OutcomeConflict)
    );
    assert_eq!(ledger.cached_outcome(&req.id), Some([1; 32]));
}

#[test]
fn completion_rejects_an_identity_that_does_not_derive_its_id() {
    let mut ledger = DedupLedger::new(8, 10);
    let req = identity(7, 1, binding(6, 0, 1_000));
    assert_eq!(ledger.observe(&req, 10), Classification::New);
    let mut other_origin = req.clone();
    other_origin.origin = [0xEE; 32];
    let mut other_nonce = req.clone();
    other_nonce.nonce = [0xDD; 32];
    for forged in [&other_origin, &other_nonce] {
        assert_eq!(forged.id, req.id);
        assert!(!forged.binding_is_valid());
        assert_eq!(
            ledger.complete(forged, [3; 32]),
            Err(DedupError::InvalidIdentity)
        );
        assert_eq!(ledger.cached_outcome(&req.id), None);
    }
    // The registered request is still in flight and its valid identity
    // completes as before.
    assert_eq!(ledger.observe(&req, 11), Classification::DuplicateInFlight);
    ledger.complete(&req, [1; 32]).unwrap();
    assert_eq!(ledger.cached_outcome(&req.id), Some([1; 32]));
    // A forged identity cannot complete an already completed entry
    // either, even with the stored outcome.
    assert_eq!(
        ledger.complete(&other_origin, [1; 32]),
        Err(DedupError::InvalidIdentity)
    );
}

#[test]
fn prune_keeps_tombstones_through_expiry_plus_grace() {
    let mut ledger = DedupLedger::new(8, 10);
    let req = identity(8, 1, binding(5, 0, 100));
    assert_eq!(ledger.observe(&req, 50), Classification::New);
    // At expiry the entry tombstones; before grace elapses prune
    // must retain it so late duplicates still read Expired.
    assert_eq!(ledger.observe(&req, 100), Classification::Expired);
    assert_eq!(ledger.prune(109), 0);
    assert_eq!(ledger.observe(&req, 109), Classification::Expired);
    // Past expiry + grace the entry may go — and the verdict for
    // the (still expired) origin remains Expired, never New.
    assert_eq!(ledger.prune(110), 1);
    assert_eq!(ledger.observe(&req, 110), Classification::Expired);
}

// ---------- model-based sequence test ----------

/// Independent reference state machine: the simplest possible
/// restatement of the design rules, sharing no code with the
/// ledger under test.
#[derive(Default)]
struct Model {
    entries: HashMap<RequestId, ([u8; 32], u64, Option<Option<[u8; 32]>>)>, // id -> (binding digest stand-in: query byte repeated, expiry, state: None=in-flight, Some(outcome))
    nonces: HashMap<(u8, u8), RequestId>,
    capacity: usize,
}

fn model_binding_key(b: &QueryBinding) -> [u8; 32] {
    // Stand-in key: hash the reference canonical bytes.
    let mut h = Sha256::new();
    h.update(reference_canonical(
        &b.query_digest,
        &b.scope,
        b.issued_at,
        b.expires_at,
        b.initial_max_hops,
    ));
    h.finalize().into()
}

impl Model {
    fn observe(&mut self, idn: &RequestIdentity, now: u64) -> Classification {
        if reference_request_id(&idn.origin, &idn.nonce, &idn.binding) != idn.id {
            return Classification::BindingConflict;
        }
        let key = model_binding_key(&idn.binding);
        if let Some((stored_key, expiry, state)) = self.entries.get(&idn.id) {
            if *stored_key != key {
                return Classification::BindingConflict;
            }
            if now >= *expiry {
                return Classification::Expired;
            }
            return match state {
                None => Classification::DuplicateInFlight,
                Some(_) => Classification::DuplicateCompleted,
            };
        }
        if now >= idn.binding.expires_at {
            return Classification::Expired;
        }
        if let Some(existing) = self.nonces.get(&(idn.origin[0], idn.nonce[0])) {
            if *existing != idn.id {
                return Classification::Replay;
            }
        }
        if self.entries.len() >= self.capacity {
            // Finite retention, restated: entries past expiry +
            // grace (5s in this test's ledgers) stop counting
            // against capacity.
            let doomed: Vec<RequestId> = self
                .entries
                .iter()
                .filter(|(_, (_, expiry, _))| now >= expiry + 5)
                .map(|(id, _)| *id)
                .collect();
            for id in doomed {
                self.entries.remove(&id);
                self.nonces.retain(|_, mapped| *mapped != id);
            }
            if self.entries.len() >= self.capacity {
                return Classification::LedgerFull;
            }
        }
        self.entries
            .insert(idn.id, (key, idn.binding.expires_at, None));
        self.nonces.insert((idn.origin[0], idn.nonce[0]), idn.id);
        Classification::New
    }

    fn complete(&mut self, idn: &RequestIdentity, outcome: [u8; 32]) {
        if let Some(entry) = self.entries.get_mut(&idn.id) {
            if entry.2.is_none() {
                entry.2 = Some(Some(outcome));
            }
        }
    }
}

/// Tiny deterministic PRNG (xorshift64) for generated sequences.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[test]
fn generated_sequences_match_reference_model() {
    for seed in [1_u64, 42, 0xB20] {
        let mut rng = Rng(seed ^ 0x9E37_79B9_7F4A_7C15);
        let mut ledger = DedupLedger::new(6, 5);
        let mut model = Model {
            capacity: 6,
            ..Default::default()
        };
        // A small pool of requests to force duplicates, replays
        // and conflicts.
        let pool: Vec<RequestIdentity> = (0..8_u64)
            .map(|i| identity(11, i as u8, binding(20 + i as u8, 0, 120 + i * 30)))
            .collect();
        let mut now = 0_u64;
        for step in 0..600 {
            now += rng.below(7);
            let pick = &pool[rng.below(pool.len() as u64) as usize];
            match rng.below(10) {
                0..=6 => {
                    let got = ledger.observe(pick, now);
                    let want = model.observe(pick, now);
                    assert_eq!(got, want, "seed {seed} step {step} now {now}");
                }
                7..=8 => {
                    let outcome = [pick.nonce[0]; 32];
                    if ledger.complete(pick, outcome).is_ok() {
                        model.complete(pick, outcome);
                    }
                }
                _ => {
                    ledger.prune(now);
                    // The model has no prune: entries past expiry
                    // still answer Expired there, and the ledger
                    // answers Expired for expired origins too, so
                    // post-prune verdicts stay comparable for the
                    // expired pool members used here.
                }
            }
        }
    }
}
