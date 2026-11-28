//! B16 model test: fan-out budgets, max-hop propagation and breadth ×
//! depth bounds (#10263, statements S1.54-S1.57 and S1.61).
//!
//! Oracle: **model** — at the per-query, requester, supplying-peer,
//! tenant and node scopes, spent never decreases and held + spent stays
//! within cap. Duplicate scopes, over-cap reservations and use outside
//! the reserved dimensions are refused with nothing changed. Closing
//! (cancel or expiry) refunds only unused held amounts, and a retry of
//! the same reservation never mints. The reference ledger here follows
//! `ScopedLedger` in `part-b/federation_model.py`, written out again in
//! Rust with per-scope arithmetic recomputed from the reservations it
//! keeps, so it shares no state layout with the slice. A second
//! reference checks child allocation, and a random tree simulation
//! checks that a whole relay tree never contacts more peers than the
//! origin's peer slots or goes deeper than its hops.
//!
//! The slice modules are loaded by path: the federation roots record
//! them without declaring them (slice 9110-02), and B16 reaches B20
//! through `super::`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/federation/slices/b20_request_identity.rs"]
#[allow(dead_code)]
mod b20_request_identity;

#[path = "../src/federation/slices/b16_fanout_budget.rs"]
mod b16_fanout_budget;

use std::collections::BTreeMap;

use b16_fanout_budget::{
    AllocError, BudgetError, ChildAllocator, Dimension, Grant, ReservationId, Scope, ScopedLedger,
    Vector,
};
use b20_request_identity::RequestId;
use proptest::prelude::*;

use Dimension::{Bytes, Hops, PeerSlots, Work};

const DIMS: [Dimension; 4] = [Hops, PeerSlots, Work, Bytes];

fn v(pairs: &[(Dimension, u64)]) -> Vector {
    pairs.iter().copied().collect()
}

const fn rid(n: u8) -> ReservationId {
    [n; 32]
}

fn scopes_all() -> Vec<Scope> {
    vec![
        Scope::Query(RequestId([1; 32])),
        Scope::Requester("alice".into()),
        Scope::SupplyingPeer("peer-a".into()),
        Scope::Tenant("t1".into()),
        Scope::Node,
    ]
}

fn limits(scopes: &[Scope], cap: u64) -> BTreeMap<(Scope, Dimension), u64> {
    scopes
        .iter()
        .flat_map(|s| DIMS.iter().map(move |d| ((s.clone(), *d), cap)))
        .collect()
}

#[test]
fn reserve_use_close_moves_held_to_spent_and_refunds_the_rest() {
    let scopes = scopes_all();
    let mut l = ScopedLedger::new(limits(&scopes, 100), 10);
    let want = v(&[(Work, 40), (Bytes, 30)]);
    assert_eq!(
        l.reserve(rid(1), want.clone(), scopes.clone(), 50, 0),
        Ok(want)
    );
    for s in &scopes {
        assert_eq!(l.held(s, Work), 40);
        assert_eq!(l.spent(s, Work), 0);
    }
    l.record_use(&rid(1), &v(&[(Work, 25)]), 1).unwrap();
    l.record_use(&rid(1), &v(&[(Work, 5), (Bytes, 30)]), 2)
        .unwrap();
    assert_eq!(
        l.record_use(&rid(1), &v(&[(Work, 11)]), 3),
        Err(BudgetError::OverReservation(Work))
    );
    assert_eq!(
        l.record_use(&rid(1), &v(&[(PeerSlots, 1)]), 3),
        Err(BudgetError::OutsideReservation(PeerSlots))
    );
    assert_eq!(l.close(&rid(1)), Ok(v(&[(Work, 10), (Bytes, 0)])));
    for s in &scopes {
        assert_eq!(l.held(s, Work), 0);
        assert_eq!(l.spent(s, Work), 30);
        assert_eq!(l.spent(s, Bytes), 30);
    }
    // Closing again refunds nothing; a closed reservation takes no use.
    assert_eq!(l.close(&rid(1)), Ok(Vector::new()));
    assert_eq!(
        l.record_use(&rid(1), &v(&[(Work, 1)]), 4),
        Err(BudgetError::Closed)
    );
}

#[test]
fn refusals_are_atomic() {
    let scopes = scopes_all();
    let mut lim = limits(&scopes, 100);
    lim.insert((Scope::Tenant("t1".into()), Work), 10);
    let mut l = ScopedLedger::new(lim, 10);
    // Over the tenant cap: nothing is held anywhere.
    assert_eq!(
        l.reserve(rid(1), v(&[(Work, 11)]), scopes.clone(), 50, 0),
        Err(BudgetError::OverCap(Scope::Tenant("t1".into()), Work))
    );
    for s in &scopes {
        assert_eq!(l.held(s, Work), 0);
    }
    let mut dup = scopes.clone();
    dup.push(Scope::Requester("alice".into()));
    assert_eq!(
        l.reserve(rid(1), v(&[(Work, 1)]), dup, 50, 0),
        Err(BudgetError::DuplicateScope(Scope::Requester(
            "alice".into()
        )))
    );
    let no_node: Vec<Scope> = scopes[..4].to_vec();
    assert_eq!(
        l.reserve(rid(1), v(&[(Work, 1)]), no_node, 50, 0),
        Err(BudgetError::MissingNodeScope)
    );
    let stranger = vec![Scope::Tenant("t2".into()), Scope::Node];
    assert_eq!(
        l.reserve(rid(1), v(&[(Work, 1)]), stranger, 50, 0),
        Err(BudgetError::NoLimit(Scope::Tenant("t2".into()), Work))
    );
    assert_eq!(l.held(&Scope::Node, Work), 0);
    assert_eq!(
        l.reserve(rid(1), v(&[(Work, u64::MAX)]), scopes.clone(), 50, 0),
        Err(BudgetError::OverCap(scopes[0].clone(), Work))
    );
    assert!(!l.is_open(&rid(1)));
}

#[test]
fn spent_work_counts_against_the_cap_after_close() {
    let scopes = vec![Scope::Node];
    let mut l = ScopedLedger::new(limits(&scopes, 10), 0);
    l.reserve(rid(1), v(&[(Work, 10)]), scopes.clone(), 50, 0)
        .unwrap();
    l.record_use(&rid(1), &v(&[(Work, 8)]), 1).unwrap();
    assert_eq!(l.close(&rid(1)), Ok(v(&[(Work, 2)])));
    assert_eq!(
        l.reserve(rid(2), v(&[(Work, 3)]), scopes.clone(), 50, 2),
        Err(BudgetError::OverCap(Scope::Node, Work))
    );
    l.reserve(rid(2), v(&[(Work, 2)]), scopes, 50, 2).unwrap();
    assert_eq!(l.spent(&Scope::Node, Work), 8);
    assert_eq!(l.held(&Scope::Node, Work), 2);
}

#[test]
fn one_requester_cannot_take_anothers_allowance() {
    let alice = vec![Scope::Requester("alice".into()), Scope::Node];
    let bob = vec![Scope::Requester("bob".into()), Scope::Node];
    let mut lim = limits(&alice, 10);
    lim.extend(limits(&bob, 10));
    lim.insert((Scope::Node, Work), 15);
    let mut l = ScopedLedger::new(lim, 0);
    l.reserve(rid(1), v(&[(Work, 10)]), alice.clone(), 50, 0)
        .unwrap();
    assert_eq!(
        l.reserve(rid(2), v(&[(Work, 1)]), alice, 50, 0),
        Err(BudgetError::OverCap(Scope::Requester("alice".into()), Work))
    );
    assert_eq!(
        l.reserve(rid(3), v(&[(Work, 6)]), bob.clone(), 50, 0),
        Err(BudgetError::OverCap(Scope::Node, Work))
    );
    l.reserve(rid(3), v(&[(Work, 5)]), bob, 50, 0).unwrap();
}

#[test]
fn retries_reuse_the_exact_reservation_and_never_mint() {
    let scopes = scopes_all();
    let mut l = ScopedLedger::new(limits(&scopes, 100), 10);
    let want = v(&[(Work, 60)]);
    l.reserve(rid(1), want.clone(), scopes.clone(), 50, 0)
        .unwrap();
    assert_eq!(
        l.reserve(rid(1), want.clone(), scopes.clone(), 50, 5),
        Ok(want.clone())
    );
    assert_eq!(l.held(&Scope::Node, Work), 60);
    assert_eq!(
        l.reserve(rid(1), v(&[(Work, 61)]), scopes.clone(), 50, 5),
        Err(BudgetError::BindingConflict)
    );
    assert_eq!(
        l.reserve(rid(1), want.clone(), scopes.clone(), 51, 5),
        Err(BudgetError::BindingConflict)
    );
    let mut reordered = scopes.clone();
    reordered.reverse();
    assert_eq!(
        l.reserve(rid(1), want.clone(), reordered, 50, 5),
        Err(BudgetError::BindingConflict)
    );
    assert_eq!(
        l.reserve(rid(1), want.clone(), scopes.clone(), 50, 50),
        Err(BudgetError::Expired)
    );
    l.close(&rid(1)).unwrap();
    assert_eq!(
        l.reserve(rid(1), want, scopes, 50, 5),
        Err(BudgetError::Closed)
    );
    assert_eq!(l.held(&Scope::Node, Work), 0);
}

#[test]
fn over_cap_reserve_reclaims_expired_opens_before_refusing() {
    let scopes = scopes_all();
    let mut l = ScopedLedger::new(limits(&scopes, 100), 10);
    l.reserve(rid(1), v(&[(Work, 90)]), scopes.clone(), 20, 0)
        .unwrap();
    assert_eq!(
        l.reserve(rid(2), v(&[(Work, 50)]), scopes.clone(), 100, 10),
        Err(BudgetError::OverCap(scopes[0].clone(), Work))
    );
    assert!(l.is_open(&rid(1)));
    assert_eq!(l.held(&Scope::Node, Work), 90);
    l.reserve(rid(2), v(&[(Work, 50)]), scopes.clone(), 100, 20)
        .unwrap();
    assert!(!l.is_open(&rid(1)));
    assert!(l.is_open(&rid(2)));
    assert_eq!(l.held(&Scope::Node, Work), 50);
    assert_eq!(l.spent(&Scope::Node, Work), 0);
}

#[test]
fn expiry_closes_refunds_unused_and_tombstones_until_grace() {
    let scopes = scopes_all();
    let mut l = ScopedLedger::new(limits(&scopes, 100), 10);
    l.reserve(rid(1), v(&[(Work, 50)]), scopes.clone(), 20, 0)
        .unwrap();
    l.reserve(rid(2), v(&[(Work, 10)]), scopes, 40, 0).unwrap();
    l.record_use(&rid(1), &v(&[(Work, 7)]), 19).unwrap();
    assert_eq!(
        l.record_use(&rid(1), &v(&[(Work, 1)]), 20),
        Err(BudgetError::Expired)
    );
    assert_eq!(l.close_expired(20), vec![rid(1)]);
    assert_eq!(l.held(&Scope::Node, Work), 10);
    assert_eq!(l.spent(&Scope::Node, Work), 7);
    assert_eq!(l.prune(29), 0);
    assert_eq!(l.prune(30), 1);
    // Open reservations are never pruned, even well past expiry.
    assert_eq!(l.prune(1_000), 0);
    assert!(l.is_open(&rid(2)));
    assert_eq!(
        l.record_use(&rid(9), &Vector::new(), 0),
        Err(BudgetError::Unknown)
    );
    assert_eq!(l.close(&rid(9)), Err(BudgetError::Unknown));
}

fn grant(pairs: &[(Dimension, u64)], deadline: u64) -> Grant {
    Grant {
        request_id: RequestId([1; 32]),
        vector: v(pairs),
        deadline,
    }
}

#[test]
fn children_get_fewer_hops_and_a_partition_of_the_rest() {
    let mut a = ChildAllocator::new(grant(&[(Hops, 3), (PeerSlots, 5), (Work, 100)], 50));
    let g = a
        .allocate("b", v(&[(Hops, 2), (PeerSlots, 2), (Work, 40)]), 0)
        .unwrap();
    assert_eq!(g.deadline, 50);
    assert_eq!(g.request_id, RequestId([1; 32]));
    assert_eq!(a.remaining(), v(&[(PeerSlots, 2), (Work, 60)]));
    assert_eq!(
        a.allocate("c", v(&[(Hops, 3)]), 0),
        Err(AllocError::TooDeep)
    );
    assert_eq!(
        a.allocate("c", v(&[(Hops, 1), (PeerSlots, 2)]), 0),
        Err(AllocError::Exhausted(PeerSlots))
    );
    assert_eq!(
        a.allocate("c", v(&[(Bytes, 1)]), 0),
        Err(AllocError::OutsideGrant(Bytes))
    );
    a.consume(&v(&[(Work, 60)]), 1).unwrap();
    assert_eq!(
        a.allocate("c", v(&[(Work, 1)]), 1),
        Err(AllocError::Exhausted(Work))
    );
    // Retry returns the exact allocation; a different one conflicts.
    assert_eq!(
        a.allocate("b", v(&[(Hops, 2), (PeerSlots, 2), (Work, 40)]), 2),
        Ok(g)
    );
    assert_eq!(
        a.allocate("b", v(&[(Hops, 2), (PeerSlots, 1), (Work, 40)]), 2),
        Err(AllocError::ChildConflict)
    );
    assert_eq!(
        a.allocate("c", v(&[(Hops, 0)]), 2).unwrap().vector,
        v(&[(Hops, 0)])
    );
    assert_eq!(a.remaining(), v(&[(PeerSlots, 1), (Work, 0)]));
    assert_eq!(a.children().len(), 2);
    assert_eq!(a.allocate("d", Vector::new(), 50), Err(AllocError::Expired));
    assert_eq!(a.consume(&Vector::new(), 50), Err(AllocError::Expired));
    assert_eq!(
        a.consume(&v(&[(Hops, 1)]), 3),
        Err(AllocError::OutsideGrant(Hops))
    );
}

#[test]
fn a_grant_with_no_hops_left_cannot_forward() {
    let mut a = ChildAllocator::new(grant(&[(Hops, 0), (PeerSlots, 9)], 50));
    assert_eq!(
        a.allocate("b", Vector::new(), 0),
        Err(AllocError::NoHopsLeft)
    );
    let mut b = ChildAllocator::new(grant(&[(PeerSlots, 9)], 50));
    assert_eq!(
        b.allocate("b", Vector::new(), 0),
        Err(AllocError::NoHopsLeft)
    );
    let mut c = ChildAllocator::new(grant(&[(Hops, 1)], 50));
    assert_eq!(
        c.allocate("b", Vector::new(), 0),
        Err(AllocError::OutsideGrant(PeerSlots))
    );
}

// ---- reference ledger ------------------------------------------------

#[derive(Debug, Clone)]
struct RefRes {
    vector: Vector,
    scopes: Vec<Scope>,
    expires: u64,
    used: Vector,
    closed: bool,
}

#[derive(Debug, Clone)]
struct RefLedger {
    limits: BTreeMap<(Scope, Dimension), u64>,
    res: BTreeMap<ReservationId, RefRes>,
    pruned: Vec<RefRes>,
    grace: u64,
}

impl RefLedger {
    // Held and spent are recomputed from the reservations every time.
    fn held(&self, s: &Scope, d: Dimension) -> u64 {
        self.res
            .values()
            .filter(|r| !r.closed && r.scopes.contains(s))
            .map(|r| r.vector.get(&d).copied().unwrap_or(0) - r.used.get(&d).copied().unwrap_or(0))
            .sum()
    }

    fn spent(&self, s: &Scope, d: Dimension) -> u64 {
        self.res
            .values()
            .filter(|r| r.scopes.contains(s))
            .map(|r| r.used.get(&d).copied().unwrap_or(0))
            .sum::<u64>()
            + self.pruned_spent(s, d)
    }

    fn pruned_spent(&self, s: &Scope, d: Dimension) -> u64 {
        self.pruned
            .iter()
            .filter(|r| r.scopes.contains(s))
            .map(|r| r.used.get(&d).copied().unwrap_or(0))
            .sum()
    }

    fn reserve(
        &mut self,
        id: ReservationId,
        vector: &Vector,
        scopes: &[Scope],
        expires: u64,
        now: u64,
    ) -> Result<Vector, BudgetError> {
        for (i, s) in scopes.iter().enumerate() {
            if scopes[..i].contains(s) {
                return Err(BudgetError::DuplicateScope(s.clone()));
            }
        }
        if !scopes.contains(&Scope::Node) {
            return Err(BudgetError::MissingNodeScope);
        }
        if let Some(r) = self.res.get(&id) {
            if (&r.vector, r.scopes.as_slice(), r.expires) != (vector, scopes, expires) {
                return Err(BudgetError::BindingConflict);
            }
            if r.closed {
                return Err(BudgetError::Closed);
            }
            return if now < expires {
                Ok(vector.clone())
            } else {
                Err(BudgetError::Expired)
            };
        }
        if now >= expires {
            return Err(BudgetError::Expired);
        }
        let over_cap = |this: &Self| -> Result<Option<BudgetError>, BudgetError> {
            for s in scopes {
                for (d, amount) in vector {
                    let Some(limit) = this.limits.get(&(s.clone(), *d)) else {
                        return Err(BudgetError::NoLimit(s.clone(), *d));
                    };
                    let used = u128::from(this.held(s, *d)) + u128::from(this.spent(s, *d));
                    if used + u128::from(*amount) > u128::from(*limit) {
                        return Ok(Some(BudgetError::OverCap(s.clone(), *d)));
                    }
                }
            }
            Ok(None)
        };
        if over_cap(self)?.is_some() {
            let due: Vec<ReservationId> = self
                .res
                .iter()
                .filter(|(_, x)| !x.closed && now >= x.expires)
                .map(|(id, _)| *id)
                .collect();
            for id in &due {
                self.close(id)?;
            }
            if let Some(err) = over_cap(self)? {
                return Err(err);
            }
        }
        self.res.insert(
            id,
            RefRes {
                vector: vector.clone(),
                scopes: scopes.to_vec(),
                expires,
                used: Vector::new(),
                closed: false,
            },
        );
        Ok(vector.clone())
    }

    fn record_use(
        &mut self,
        id: &ReservationId,
        delta: &Vector,
        now: u64,
    ) -> Result<(), BudgetError> {
        let r = self.res.get_mut(id).ok_or(BudgetError::Unknown)?;
        if r.closed {
            return Err(BudgetError::Closed);
        }
        if now >= r.expires {
            return Err(BudgetError::Expired);
        }
        for (d, more) in delta {
            let Some(reserved) = r.vector.get(d) else {
                return Err(BudgetError::OutsideReservation(*d));
            };
            let used = r.used.get(d).copied().unwrap_or(0);
            if u128::from(used) + u128::from(*more) > u128::from(*reserved) {
                return Err(BudgetError::OverReservation(*d));
            }
        }
        for (d, more) in delta {
            *r.used.entry(*d).or_default() += more;
        }
        Ok(())
    }

    fn close(&mut self, id: &ReservationId) -> Result<Vector, BudgetError> {
        let r = self.res.get_mut(id).ok_or(BudgetError::Unknown)?;
        if r.closed {
            return Ok(Vector::new());
        }
        r.closed = true;
        Ok(r.vector
            .iter()
            .map(|(d, a)| (*d, a - r.used.get(d).copied().unwrap_or(0)))
            .collect())
    }
}

#[derive(Debug, Clone)]
enum Op {
    Reserve {
        id: u8,
        vector: Vec<(u8, u64)>,
        scopes: Vec<u8>,
        expires: u64,
    },
    Use {
        id: u8,
        delta: Vec<(u8, u64)>,
    },
    Close {
        id: u8,
    },
    CloseExpired,
    Prune,
    Tick(u64),
}

fn scope_of(n: u8) -> Scope {
    match n % 7 {
        0 => Scope::Query(RequestId([1; 32])),
        1 => Scope::Query(RequestId([2; 32])),
        2 => Scope::Requester("alice".into()),
        3 => Scope::Requester("bob".into()),
        4 => Scope::SupplyingPeer("peer-a".into()),
        5 => Scope::Tenant("t1".into()),
        _ => Scope::Node,
    }
}

fn vector_of(pairs: &[(u8, u64)]) -> Vector {
    let mut out = Vector::new();
    for (d, a) in pairs {
        out.insert(DIMS[usize::from(*d) % 4], *a);
    }
    out
}

fn arb_op() -> impl Strategy<Value = Op> {
    let pairs = || prop::collection::vec((0u8..4, 0u64..12), 0..4);
    prop_oneof![
        4 => (0u8..5, pairs(), prop::collection::vec(0u8..8, 0..5), 0u64..30).prop_map(
            |(id, vector, mut scopes, expires)| {
                scopes.push(6);
                Op::Reserve { id, vector, scopes, expires }
            }
        ),
        1 => (0u8..5, pairs(), prop::collection::vec(0u8..8, 0..4), 0u64..30).prop_map(
            |(id, vector, scopes, expires)| Op::Reserve { id, vector, scopes, expires }
        ),
        4 => (0u8..5, pairs()).prop_map(|(id, delta)| Op::Use { id, delta }),
        2 => (0u8..5).prop_map(|id| Op::Close { id }),
        1 => Just(Op::CloseExpired),
        1 => Just(Op::Prune),
        2 => (0u64..6).prop_map(Op::Tick),
    ]
}

fn all_keys() -> Vec<(Scope, Dimension)> {
    (0u8..7)
        .flat_map(|s| DIMS.iter().map(move |d| (scope_of(s), *d)))
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn ledger_matches_the_reference_and_keeps_its_invariants(
        caps in prop::collection::vec(0u64..30, 28),
        ops in prop::collection::vec(arb_op(), 1..60),
    ) {
        // Every scope and dimension gets a cap, except Bytes for bob.
        let mut lim = BTreeMap::new();
        for ((s, d), cap) in all_keys().into_iter().zip(caps) {
            if !(s == Scope::Requester("bob".into()) && d == Bytes) {
                lim.insert((s, d), cap);
            }
        }
        let grace = 5;
        let mut l = ScopedLedger::new(lim.clone(), grace);
        let mut r = RefLedger { limits: lim.clone(), res: BTreeMap::new(), pruned: Vec::new(), grace };
        let mut now = 0u64;
        let mut spent_floor: BTreeMap<(Scope, Dimension), u64> = BTreeMap::new();
        for op in ops {
            match op {
                Op::Reserve { id, vector, scopes, expires } => {
                    let vector = vector_of(&vector);
                    let scopes: Vec<Scope> = scopes.into_iter().map(scope_of).collect();
                    let want = r.reserve(rid(id), &vector, &scopes, expires, now);
                    prop_assert_eq!(l.reserve(rid(id), vector, scopes, expires, now), want);
                }
                Op::Use { id, delta } => {
                    let delta = vector_of(&delta);
                    let want = r.record_use(&rid(id), &delta, now);
                    prop_assert_eq!(l.record_use(&rid(id), &delta, now), want);
                }
                Op::Close { id } => {
                    prop_assert_eq!(l.close(&rid(id)), r.close(&rid(id)));
                }
                Op::CloseExpired => {
                    let due: Vec<ReservationId> = r
                        .res
                        .iter()
                        .filter(|(_, x)| !x.closed && now >= x.expires)
                        .map(|(id, _)| *id)
                        .collect();
                    for id in &due {
                        r.close(id).unwrap();
                    }
                    prop_assert_eq!(l.close_expired(now), due);
                }
                Op::Prune => {
                    // A pruned tombstone is forgotten, but what it spent
                    // stays charged.
                    let stale: Vec<ReservationId> = r
                        .res
                        .iter()
                        .filter(|(_, x)| x.closed && now >= x.expires + r.grace)
                        .map(|(id, _)| *id)
                        .collect();
                    for id in &stale {
                        let gone = r.res.remove(id).unwrap();
                        r.pruned.push(gone);
                    }
                    prop_assert_eq!(l.prune(now), stale.len());
                }
                Op::Tick(dt) => now += dt,
            }
            for (key, limit) in &lim {
                let (s, d) = key;
                let held = l.held(s, *d);
                let spent = l.spent(s, *d);
                prop_assert_eq!(held, r.held(s, *d));
                prop_assert_eq!(spent, r.spent(s, *d));
                prop_assert!(held + spent <= *limit);
                let floor = spent_floor.entry(key.clone()).or_default();
                prop_assert!(spent >= *floor);
                *floor = spent;
            }
        }
    }
}

// ---- relay tree ------------------------------------------------------

#[derive(Debug, Clone)]
struct Node {
    // How each child's share is chosen: fractions of what is left.
    fanout: u8,
    share: u8,
    local: u8,
}

fn arb_nodes() -> impl Strategy<Value = Vec<Node>> {
    prop::collection::vec(
        (0u8..5, 0u8..101, 0u8..101).prop_map(|(fanout, share, local)| Node {
            fanout,
            share,
            local,
        }),
        64,
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    // Every node greedily forwards to `fanout` children with a share of
    // what is left and asks for a random depth; however the tree grows,
    // it contacts at most the origin's peer slots and never goes deeper
    // than its hops, and work charged never exceeds the origin's work.
    #[test]
    fn a_relay_tree_stays_within_the_origin_grant(
        hops in 0u64..6,
        slots in 0u64..40,
        work in 0u64..200,
        nodes in arb_nodes(),
        depth_pick in prop::collection::vec(0u64..6, 64),
    ) {
        let origin = grant(&[(Hops, hops), (PeerSlots, slots), (Work, work)], 100);
        let mut queue = vec![(origin, 0u64)];
        let mut contacted = 0u64;
        let mut max_depth = 0u64;
        let mut work_used = 0u64;
        let mut next = 0usize;
        while let Some((g, depth)) = queue.pop() {
            max_depth = max_depth.max(depth);
            let node = nodes[next % nodes.len()].clone();
            let pick = depth_pick[next % depth_pick.len()];
            next += 1;
            let mut a = ChildAllocator::new(g.clone());
            for c in 0..node.fanout {
                let left = a.remaining();
                let slots_left = left.get(&PeerSlots).copied().unwrap_or(0);
                let work_left = left.get(&Work).copied().unwrap_or(0);
                let child = v(&[
                    (Hops, pick),
                    (PeerSlots, slots_left.saturating_sub(1) * u64::from(node.share) / 100),
                    (Work, work_left * u64::from(node.share) / 100),
                ]);
                match a.allocate(&format!("n{next}-{c}"), child, 0) {
                    Ok(cg) => {
                        prop_assert!(cg.vector[&Hops] < g.vector[&Hops]);
                        contacted += 1;
                        queue.push((cg, depth + 1));
                    }
                    Err(AllocError::NoHopsLeft) => prop_assert_eq!(g.vector[&Hops], 0),
                    Err(AllocError::TooDeep) => prop_assert!(pick >= g.vector[&Hops]),
                    Err(AllocError::Exhausted(PeerSlots)) => prop_assert_eq!(slots_left, 0),
                    Err(e) => prop_assert!(false, "unexpected {:?}", e),
                }
            }
            let left = a.remaining().get(&Work).copied().unwrap_or(0);
            let used = left * u64::from(node.local) / 100;
            a.consume(&v(&[(Work, used)]), 0).unwrap();
            work_used += used;
            prop_assert!(a.consume(&v(&[(Work, left - used + 1)]), 0).is_err());
        }
        prop_assert!(contacted <= slots);
        prop_assert!(max_depth <= hops);
        prop_assert!(work_used <= work);
    }
}
