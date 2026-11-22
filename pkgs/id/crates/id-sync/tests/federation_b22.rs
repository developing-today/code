//! B22 invariant test: duplicate notices and loop reports (#10230,
//! statements S1.65, S1.66 and S1.69).
//!
//! Oracle: **invariant** — the requester and the delivering peer are
//! informed of a duplicate, and a loop is reported only with a retained
//! signed path showing the reporting peer already visited. The
//! reference is a state machine in this file: for each peer it tracks
//! which requests it has run and with what outcome, and it predicts
//! every arrival from how the test built the delivery (route of relays,
//! intended and actual destination, tampering, hop bound), never from
//! the slice. Random streams sign queries, deliver them directly and
//! through random relay routes that may revisit peers, misdirect or
//! tamper with chains, complete requests and toggle duplicate notices.
//! Every notice must verify for its recipients, and a hand-built
//! encoder checks the notice preimage layout.
//!
//! The slice modules are loaded by path: the federation roots record
//! them without declaring them (slice 9110-02), and B22 reaches B17 and
//! B20 through `super::`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/federation/slices/b20_request_identity.rs"]
#[allow(dead_code)]
mod b20_request_identity;

#[path = "../src/federation/slices/b17_query_grants.rs"]
#[allow(dead_code)]
mod b17_query_grants;

#[path = "../src/federation/slices/b22_loop_inform.rs"]
mod b22_loop_inform;

use std::collections::BTreeMap;

use b17_query_grants::{ChainError, OriginQuery, QueryError, RelayGrant};
use b20_request_identity::{DedupError, OutcomeId, PeerKey, QueryBinding};
use b22_loop_inform::{
    Arrival, LoopInformer, Notice, NoticeError, NoticeKind, Refusal, verify_notice,
};
use ed25519_dalek::{Signature, SigningKey, VerifyingKey};
use proptest::prelude::*;

const NOW: u64 = 500;
const PEERS: std::ops::RangeInclusive<u8> = 2..=7;

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn id(seed: u8) -> PeerKey {
    key(seed).verifying_key().to_bytes()
}

fn query(nonce: u8, hops: u32) -> OriginQuery {
    OriginQuery::sign(
        &key(1),
        [nonce; 32],
        QueryBinding {
            query_digest: [9; 32],
            scope: b"public".to_vec(),
            issued_at: 100,
            expires_at: 1000,
            initial_max_hops: hops,
        },
    )
}

/// Grants by each relay in `route` in turn, the last one granting `child`.
fn chain(query: &OriginQuery, route: &[u8], child: u8) -> Vec<RelayGrant> {
    let mut grants: Vec<RelayGrant> = Vec::new();
    for (i, relay) in route.iter().enumerate() {
        let next = route.get(i + 1).copied().unwrap_or(child);
        let grant = RelayGrant::issue(&key(*relay), query, grants.last(), id(next), 900);
        grants.push(grant);
    }
    grants
}

fn informer(seed: u8) -> LoopInformer {
    LoopInformer::new(key(seed), 64, 60)
}

// ----- notice layout --------------------------------------------------

fn expected_preimage(n: &Notice) -> Vec<u8> {
    let mut out = b"id-sync/b22-notice/v1".to_vec();
    out.extend_from_slice(&n.request_id.0);
    out.extend_from_slice(&n.origin);
    out.extend_from_slice(&n.reporter);
    if let Some(sender) = n.sender {
        out.push(1);
        out.extend_from_slice(&sender);
    } else {
        out.push(0);
    }
    match n.kind {
        NoticeKind::DuplicateInFlight => out.push(1),
        NoticeKind::DuplicateCompleted { outcome } => {
            out.push(2);
            out.extend_from_slice(&outcome);
        }
        NoticeKind::Loop => out.push(3),
    }
    out.extend_from_slice(&n.observed_at.to_be_bytes());
    out.extend_from_slice(&u32::try_from(n.evidence.len()).unwrap().to_be_bytes());
    for grant in &n.evidence {
        out.extend_from_slice(&grant.digest());
    }
    out
}

fn check_layout(n: &Notice) {
    let pre = expected_preimage(n);
    assert_eq!(n.preimage(), pre);
    assert!(
        VerifyingKey::from_bytes(&n.reporter)
            .unwrap()
            .verify_strict(&pre, &Signature::from_bytes(&n.signature))
            .is_ok()
    );
}

// ----- explicit cases -------------------------------------------------

#[test]
fn the_same_request_direct_and_relayed_is_a_duplicate_not_a_loop() {
    let q = query(1, 4);
    let mut peer = informer(5);
    assert_eq!(peer.arrive(&q, &[], true, NOW), Arrival::Run);
    let relayed = chain(&q, &[2, 3], 5);
    let Arrival::Duplicate {
        completed: None,
        notice: Some(notice),
    } = peer.arrive(&q, &relayed, false, NOW)
    else {
        panic!("expected an in-flight duplicate notice");
    };
    assert_eq!(notice.kind, NoticeKind::DuplicateInFlight);
    assert_eq!(notice.sender, Some(id(3)));
    assert_eq!(notice.recipients(), vec![q.origin, id(3)]);
    assert!(notice.evidence.is_empty());
    check_layout(&notice);
    assert_eq!(verify_notice(&q, &notice), Ok(()));
}

#[test]
fn an_answered_duplicate_names_its_cached_outcome() {
    let q = query(1, 4);
    let mut peer = informer(5);
    assert_eq!(
        peer.arrive(&q, &chain(&q, &[2], 5), false, NOW),
        Arrival::Run
    );
    assert_eq!(peer.complete(&q, [8; 32]), Ok(()));
    assert_eq!(peer.complete(&q, [9; 32]), Err(DedupError::OutcomeConflict));
    let Arrival::Duplicate {
        completed: Some(outcome),
        notice: Some(notice),
    } = peer.arrive(&q, &[], true, NOW)
    else {
        panic!("expected a completed duplicate notice");
    };
    assert_eq!(outcome, [8; 32]);
    assert_eq!(notice.kind, NoticeKind::DuplicateCompleted { outcome });
    // A direct duplicate goes to the origin only.
    assert_eq!(notice.sender, None);
    assert_eq!(notice.recipients(), vec![q.origin]);
    check_layout(&notice);
    assert_eq!(verify_notice(&q, &notice), Ok(()));
    assert_eq!(
        informer(6).complete(&q, [8; 32]),
        Err(DedupError::UnknownRequest)
    );
}

#[test]
fn a_loop_is_reported_with_the_chain_that_proves_it() {
    let q = query(1, 6);
    let mut peer = informer(2);
    // 2 relays to 3, 3 to 4, and 4 hands it back to 2.
    let grants = chain(&q, &[2, 3, 4], 2);
    let Arrival::Loop { notice } = peer.arrive(&q, &grants, false, NOW) else {
        panic!("expected a loop");
    };
    assert_eq!(notice.kind, NoticeKind::Loop);
    assert_eq!(notice.evidence, grants);
    assert_eq!(notice.sender, Some(id(4)));
    assert_eq!(notice.recipients(), vec![q.origin, id(4)]);
    check_layout(&notice);
    assert_eq!(verify_notice(&q, &notice), Ok(()));
    // A loop is not registered as work: the first real arrival still runs.
    assert_eq!(peer.arrive(&q, &[], true, NOW), Arrival::Run);
}

#[test]
fn the_origin_relaying_itself_is_told_once() {
    let q = query(1, 4);
    let mut peer = informer(5);
    assert_eq!(peer.arrive(&q, &[], true, NOW), Arrival::Run);
    // The origin's key also signs as the first relay.
    let Arrival::Duplicate {
        notice: Some(notice),
        ..
    } = peer.arrive(&q, &chain(&q, &[1], 5), false, NOW)
    else {
        panic!("expected a duplicate notice");
    };
    assert_eq!(notice.sender, Some(q.origin));
    assert_eq!(notice.recipients(), vec![q.origin]);
}

#[test]
fn a_path_where_another_peer_repeats_is_not_a_loop_here() {
    let q = query(1, 6);
    let mut peer = informer(5);
    let grants = chain(&q, &[2, 3, 2], 5);
    assert_eq!(peer.arrive(&q, &grants, false, NOW), Arrival::Run);
}

#[test]
fn a_loop_notice_outlives_the_grants_it_carries() {
    let q = query(1, 6);
    let mut peer = informer(2);
    let Arrival::Loop { notice } = peer.arrive(&q, &chain(&q, &[2, 3], 2), false, NOW) else {
        panic!("expected a loop");
    };
    // The grants expire at 900 and the query at 1000; the notice is
    // checked as of when it was observed.
    assert_eq!(verify_notice(&q, &notice), Ok(()));
    assert!(notice.observed_at < 900);
}

#[test]
fn unverified_or_misdirected_arrivals_report_nothing() {
    let q = query(1, 4);
    let mut peer = informer(5);

    let mut forged = chain(&q, &[2, 5, 3], 5);
    forged[1].signature[0] ^= 1;
    assert_eq!(
        peer.arrive(&q, &forged, false, NOW),
        Arrival::Refused(Refusal::Chain(ChainError::BadSignature { index: 1 }))
    );
    assert_eq!(
        peer.arrive(&q, &chain(&q, &[2, 3], 6), false, NOW),
        Arrival::Refused(Refusal::NotAddressed)
    );
    assert_eq!(
        peer.arrive(&q, &[], false, NOW),
        Arrival::Refused(Refusal::NotAddressed)
    );
    // A peer that grants itself does not get to call that a loop.
    assert_eq!(
        peer.arrive(&q, &chain(&q, &[5], 5), false, NOW),
        Arrival::Refused(Refusal::SelfGrant)
    );
    assert_eq!(
        peer.arrive(&q, &chain(&q, &[2, 3, 4, 6], 5), false, NOW),
        Arrival::Refused(Refusal::Chain(ChainError::HopBoundExceeded { index: 3 }))
    );
    assert_eq!(
        peer.arrive(&q, &[], true, 1000),
        Arrival::Refused(Refusal::Chain(ChainError::Query(QueryError::Expired)))
    );
    // None of that registered the request.
    assert_eq!(peer.arrive(&q, &[], true, NOW), Arrival::Run);
}

#[test]
fn b20_refusals_pass_through_without_a_notice() {
    let mut peer = informer(5);
    let first = query(1, 4);
    assert_eq!(peer.arrive(&first, &[], true, NOW), Arrival::Run);
    // The same origin nonce under another binding is a replay.
    let replay = query(1, 5);
    assert_eq!(
        peer.arrive(&replay, &[], true, NOW),
        Arrival::Refused(Refusal::Replay)
    );

    let mut small = LoopInformer::new(key(5), 1, 0);
    assert_eq!(small.arrive(&query(2, 4), &[], true, NOW), Arrival::Run);
    assert_eq!(
        small.arrive(&query(3, 4), &[], true, NOW),
        Arrival::Refused(Refusal::LedgerFull)
    );
}

#[test]
fn turning_duplicate_notices_off_keeps_loops_and_dedup() {
    let q = query(1, 6);
    let mut peer = informer(3);
    peer.set_inform_duplicates(false);
    assert_eq!(peer.arrive(&q, &[], true, NOW), Arrival::Run);
    assert_eq!(
        peer.arrive(&q, &chain(&q, &[2], 3), false, NOW),
        Arrival::Duplicate {
            completed: None,
            notice: None
        }
    );
    assert!(matches!(
        peer.arrive(&q, &chain(&q, &[3, 4], 3), false, NOW),
        Arrival::Loop { .. }
    ));
    peer.set_inform_duplicates(true);
    assert!(matches!(
        peer.arrive(&q, &[], true, NOW),
        Arrival::Duplicate {
            notice: Some(_),
            ..
        }
    ));
}

#[test]
fn forged_loop_reports_are_refused() {
    let q = query(1, 6);
    let reporter = key(5);
    let honest_loop = {
        let mut peer = LoopInformer::new(reporter.clone(), 8, 0);
        let Arrival::Loop { notice } = peer.arrive(&q, &chain(&q, &[5, 3], 5), false, NOW) else {
            panic!("expected a loop");
        };
        notice
    };
    assert_eq!(verify_notice(&q, &honest_loop), Ok(()));
    let resign = |mut n: Notice| {
        n.sign(&reporter);
        n
    };

    // A path that never visited the reporter.
    let mut no_visit = honest_loop.clone();
    no_visit.evidence = chain(&q, &[2, 3], 5);
    assert_eq!(
        verify_notice(&q, &resign(no_visit)),
        Err(NoticeError::NoSelfVisit)
    );

    // Someone else's loop, claimed by the reporter.
    let mut theirs = honest_loop.clone();
    theirs.evidence = chain(&q, &[4, 3], 4);
    assert_eq!(
        verify_notice(&q, &resign(theirs)),
        Err(NoticeError::NotTheReporter)
    );

    // The reporter's own self-grant as "evidence".
    let mut own = honest_loop.clone();
    own.evidence = chain(&q, &[5], 5);
    own.sender = Some(id(5));
    assert_eq!(verify_notice(&q, &resign(own)), Err(NoticeError::SelfGrant));

    // A sender other than the relay that delivered it.
    let mut wrong_sender = honest_loop.clone();
    wrong_sender.sender = Some(id(4));
    assert_eq!(
        verify_notice(&q, &resign(wrong_sender)),
        Err(NoticeError::NotTheReporter)
    );

    // An evidence grant changed after signing.
    let mut tampered = honest_loop.clone();
    tampered.evidence[0].expires_at = 899;
    assert!(matches!(
        verify_notice(&q, &resign(tampered)),
        Err(NoticeError::Evidence(ChainError::BadSignature { index: 0 }))
    ));

    // Observed after the grants expired.
    let mut late = honest_loop.clone();
    late.observed_at = 950;
    assert!(matches!(
        verify_notice(&q, &resign(late)),
        Err(NoticeError::Evidence(ChainError::Expired { .. }))
    ));

    // A loop with no evidence, and a duplicate with some.
    let mut bare = honest_loop.clone();
    bare.evidence.clear();
    assert_eq!(
        verify_notice(&q, &resign(bare)),
        Err(NoticeError::EvidenceShape)
    );
    let mut padded = honest_loop.clone();
    padded.kind = NoticeKind::DuplicateInFlight;
    assert_eq!(
        verify_notice(&q, &resign(padded)),
        Err(NoticeError::EvidenceShape)
    );

    // Signed by another peer in the reporter's name.
    let mut impostor = honest_loop.clone();
    impostor.sign(&key(6));
    assert_eq!(verify_notice(&q, &impostor), Err(NoticeError::BadSignature));

    // Naming another origin under the same request id.
    let mut other_origin = honest_loop.clone();
    other_origin.origin = id(6);
    assert_eq!(
        verify_notice(&q, &resign(other_origin)),
        Err(NoticeError::WrongRequest)
    );

    // About another request.
    assert_eq!(
        verify_notice(&query(2, 6), &honest_loop),
        Err(NoticeError::WrongRequest)
    );
}

// ----- reference model -------------------------------------------------

#[derive(Debug, Clone)]
enum Action {
    Sign {
        hops: u32,
    },
    Deliver {
        query: usize,
        route: Vec<u8>,
        intended: u8,
        actual: u8,
        addressed: bool,
        tamper: bool,
    },
    Complete {
        query: usize,
        peer: u8,
        outcome: u8,
    },
    Inform {
        peer: u8,
        on: bool,
    },
}

fn peer_seed() -> impl Strategy<Value = u8> {
    PEERS
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        1 => (1u32..=8).prop_map(|hops| Action::Sign { hops }),
        6 => (
            any::<usize>(),
            prop::collection::vec(peer_seed(), 0..6),
            peer_seed(),
            prop::option::weighted(0.15, peer_seed()),
            prop::bool::weighted(0.85),
            prop::bool::weighted(0.1),
        )
            .prop_map(|(query, route, intended, misdirect, addressed, tamper)| {
                Action::Deliver {
                    query,
                    route,
                    intended,
                    actual: misdirect.unwrap_or(intended),
                    addressed,
                    tamper,
                }
            }),
        2 => (any::<usize>(), peer_seed(), 0u8..3).prop_map(|(query, peer, outcome)| {
            Action::Complete { query, peer, outcome }
        }),
        1 => (peer_seed(), any::<bool>()).prop_map(|(peer, on)| Action::Inform { peer, on }),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seen {
    Running,
    Answered(OutcomeId),
}

#[derive(Debug, PartialEq, Eq)]
enum Expect {
    Run,
    Duplicate(Option<OutcomeId>),
    Loop,
    ChainRefused,
    NotAddressed,
    SelfGrant,
}

struct Model {
    queries: Vec<OriginQuery>,
    peers: BTreeMap<u8, LoopInformer>,
    seen: BTreeMap<(usize, u8), Seen>,
    inform: BTreeMap<u8, bool>,
}

impl Model {
    fn new() -> Self {
        Self {
            queries: vec![query(0, 4)],
            peers: PEERS.map(|p| (p, informer(p))).collect(),
            seen: BTreeMap::new(),
            inform: PEERS.map(|p| (p, true)).collect(),
        }
    }

    fn expect(
        &self,
        q: usize,
        route: &[u8],
        intended: u8,
        actual: u8,
        addressed: bool,
        tamper: bool,
    ) -> Expect {
        let hops = self.queries[q].binding.initial_max_hops as usize;
        if route.len() >= hops || (tamper && !route.is_empty()) {
            return Expect::ChainRefused;
        }
        if (route.is_empty() && !addressed) || (!route.is_empty() && intended != actual) {
            return Expect::NotAddressed;
        }
        if route.last() == Some(&actual) {
            return Expect::SelfGrant;
        }
        if route.contains(&actual) {
            return Expect::Loop;
        }
        match self.seen.get(&(q, actual)) {
            None => Expect::Run,
            Some(Seen::Running) => Expect::Duplicate(None),
            Some(Seen::Answered(outcome)) => Expect::Duplicate(Some(*outcome)),
        }
    }

    fn step(&mut self, action: &Action) {
        match action {
            Action::Sign { hops } => {
                let nonce = u8::try_from(self.queries.len()).unwrap();
                self.queries.push(query(nonce, *hops));
            }
            Action::Inform { peer, on } => {
                self.peers.get_mut(peer).unwrap().set_inform_duplicates(*on);
                self.inform.insert(*peer, *on);
            }
            Action::Complete {
                query,
                peer,
                outcome,
            } => {
                let q = query % self.queries.len();
                let outcome = [*outcome; 32];
                let got = self
                    .peers
                    .get_mut(peer)
                    .unwrap()
                    .complete(&self.queries[q], outcome);
                let want = match self.seen.get(&(q, *peer)) {
                    None => Err(DedupError::UnknownRequest),
                    Some(Seen::Running) => Ok(()),
                    Some(Seen::Answered(o)) if *o == outcome => Ok(()),
                    Some(Seen::Answered(_)) => Err(DedupError::OutcomeConflict),
                };
                assert_eq!(got, want, "{action:?}");
                if want.is_ok() {
                    self.seen.insert((q, *peer), Seen::Answered(outcome));
                }
            }
            Action::Deliver {
                query,
                route,
                intended,
                actual,
                addressed,
                tamper,
            } => {
                let q = query % self.queries.len();
                let origin_query = self.queries[q].clone();
                let mut grants = chain(&origin_query, route, *intended);
                if *tamper && let Some(g) = grants.last_mut() {
                    g.signature[5] ^= 1;
                }
                let want = self.expect(q, route, *intended, *actual, *addressed, *tamper);
                let got = self.peers.get_mut(actual).unwrap().arrive(
                    &origin_query,
                    &grants,
                    *addressed,
                    NOW,
                );
                let sender = route.last().map(|r| id(*r));
                let check = |notice: &Notice, kind: NoticeKind, evidence: &[RelayGrant]| {
                    assert_eq!(notice.kind, kind, "{action:?}");
                    assert_eq!(notice.reporter, id(*actual));
                    assert_eq!(notice.sender, sender);
                    assert_eq!(notice.evidence, evidence);
                    let mut recipients = vec![origin_query.origin];
                    recipients.extend(sender);
                    assert_eq!(notice.recipients(), recipients);
                    check_layout(notice);
                    assert_eq!(verify_notice(&origin_query, notice), Ok(()), "{action:?}");
                };
                match (&want, got) {
                    (Expect::Run, Arrival::Run) => {
                        self.seen.insert((q, *actual), Seen::Running);
                    }
                    (Expect::Duplicate(cached), Arrival::Duplicate { completed, notice }) => {
                        assert_eq!(completed, *cached, "{action:?}");
                        assert_eq!(notice.is_some(), self.inform[actual], "{action:?}");
                        if let Some(notice) = notice {
                            let kind = cached.map_or(NoticeKind::DuplicateInFlight, |outcome| {
                                NoticeKind::DuplicateCompleted { outcome }
                            });
                            check(&notice, kind, &[]);
                        }
                    }
                    (Expect::Loop, Arrival::Loop { notice }) => {
                        assert!(notice.evidence.iter().any(|g| g.relay_id == id(*actual)));
                        check(&notice, NoticeKind::Loop, &grants);
                    }
                    (Expect::ChainRefused, Arrival::Refused(Refusal::Chain(_)))
                    | (Expect::NotAddressed, Arrival::Refused(Refusal::NotAddressed))
                    | (Expect::SelfGrant, Arrival::Refused(Refusal::SelfGrant)) => {}
                    (want, got) => panic!("{action:?}: expected {want:?}, got {got:?}"),
                }
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    #[test]
    fn arrivals_match_the_reference(actions in prop::collection::vec(action(), 1..40)) {
        let mut model = Model::new();
        for action in &actions {
            model.step(action);
        }
    }
}
