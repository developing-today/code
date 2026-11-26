//! B15 invariant test: prioritized remote selection (#10229, statements
//! S1.47-S1.53).
//!
//! Oracle: **invariant** — equal inputs produce deterministic remote
//! ordering, and one remote's negative is never read as global absence.
//! The reference is written here, independently of the slice: it
//! filters the configured list by the same eligibility rules, then does
//! a stable sort on `(reported down, priority)` so list position breaks
//! ties, and it replays every search against a model that walks the
//! reference order, counting distinct result ids. Random configurations
//! mix classes, expiry windows, disabled peers, duplicates and opt-in
//! routing advice; random searches mix found, not-found and failed
//! answers with random limits and deadlines.
//!
//! The slice modules are loaded by path: the federation roots record
//! them without declaring them (slice 9110-02), and B15 reaches B20 and
//! B31 through `super::`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/federation/slices/b20_request_identity.rs"]
#[allow(dead_code)]
mod b20_request_identity;

#[path = "../src/federation/slices/b31_peer_classes.rs"]
#[allow(dead_code)]
mod b31_peer_classes;

#[path = "../src/federation/slices/b15_peer_priority.rs"]
mod b15_peer_priority;

use std::collections::{BTreeMap, BTreeSet};

use b15_peer_priority::{
    Answer, Next, Plan, RemoteEntry, ResultId, RoutingAdvice, Search, SearchError, SkipReason,
    Source, StopReason, plan,
};
use b20_request_identity::RequestId;
use b31_peer_classes::{Operation, PeerClass, PeerPolicy};
use proptest::prelude::*;

const NOW: u64 = 1_000;

const fn rid() -> RequestId {
    RequestId([7; 32])
}

fn policy(peer: &str, class: PeerClass) -> PeerPolicy {
    PeerPolicy {
        peer_id: peer.to_owned(),
        class,
        operations: BTreeSet::from([Operation::QueryRelay]),
        relay_granted: false,
        scope: "s".to_owned(),
        policy_version: 1,
        valid_from: None,
        expires_at: None,
    }
}

fn entry(peer: &str, priority: u32) -> RemoteEntry {
    RemoteEntry {
        peer_id: peer.to_owned(),
        priority,
    }
}

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

const fn id(n: u8) -> ResultId {
    [n; 32]
}

fn ids(v: &[u8]) -> BTreeSet<ResultId> {
    v.iter().map(|n| id(*n)).collect()
}

fn trusted(peers: &[&str]) -> BTreeMap<String, PeerPolicy> {
    peers
        .iter()
        .map(|p| ((*p).to_owned(), policy(p, PeerClass::FullyTrusted)))
        .collect()
}

#[test]
fn orders_by_priority_then_list_position() {
    let remotes = [entry("c", 2), entry("a", 1), entry("d", 1), entry("b", 0)];
    let p = plan(
        rid(),
        &remotes,
        &trusted(&["a", "b", "c", "d"]),
        &BTreeSet::new(),
        None,
        NOW,
    );
    assert_eq!(p.order, names(&["b", "a", "d", "c"]));
    assert_eq!(p.skipped, [] as [(PeerId, SkipReason); 0]);
    assert_eq!(p.request_id, rid());
}

#[test]
fn eligibility_is_a_gate_with_reasons() {
    let mut policies = trusted(&["ok", "off"]);
    policies.insert(
        "signed".into(),
        policy("signed", PeerClass::AcceptSignedFrom),
    );
    policies.insert("send".into(), policy("send", PeerClass::SendOnly));
    policies.insert("unknown".into(), policy("unknown", PeerClass::Unknown));
    let mut expired = policy("expired", PeerClass::FullyTrusted);
    expired.expires_at = Some(NOW);
    policies.insert("expired".into(), expired);
    let mut early = policy("early", PeerClass::FullyTrusted);
    early.valid_from = Some(NOW + 1);
    policies.insert("early".into(), early);
    let remotes = [
        entry("send", 0),
        entry("ok", 5),
        entry("none", 0),
        entry("unknown", 0),
        entry("expired", 0),
        entry("early", 0),
        entry("off", 0),
        entry("signed", 9),
        entry("ok", 0),
    ];
    let disabled = BTreeSet::from(["off".to_owned()]);
    let p = plan(rid(), &remotes, &policies, &disabled, None, NOW);
    assert_eq!(p.order, names(&["ok", "signed"]));
    assert_eq!(
        p.skipped,
        vec![
            ("send".into(), SkipReason::ClassForbidsQuery),
            ("none".into(), SkipReason::NoPolicy),
            ("unknown".into(), SkipReason::ClassForbidsQuery),
            ("expired".into(), SkipReason::PolicyNotInForce),
            ("early".into(), SkipReason::PolicyNotInForce),
            ("off".into(), SkipReason::Disabled),
            ("ok".into(), SkipReason::DuplicateEntry),
        ]
    );
}

#[test]
fn a_duplicate_keeps_the_first_entry_even_when_the_first_is_ineligible() {
    let remotes = [entry("a", 3), entry("a", 0), entry("b", 1)];
    let p = plan(
        rid(),
        &remotes,
        &trusted(&["a", "b"]),
        &BTreeSet::new(),
        None,
        NOW,
    );
    assert_eq!(p.order, names(&["b", "a"]));
    let disabled = BTreeSet::from(["a".to_owned()]);
    let p = plan(rid(), &remotes, &trusted(&["a", "b"]), &disabled, None, NOW);
    assert_eq!(p.order, names(&["b"]));
    assert_eq!(
        p.skipped,
        vec![
            ("a".into(), SkipReason::Disabled),
            ("a".into(), SkipReason::DuplicateEntry)
        ]
    );
}

#[test]
fn advice_reorders_only_when_given_and_never_removes_or_admits() {
    let remotes = [entry("a", 0), entry("b", 1), entry("c", 2), entry("x", 0)];
    let policies = trusted(&["a", "b", "c"]);
    let none = plan(rid(), &remotes, &policies, &BTreeSet::new(), None, NOW);
    assert_eq!(none.order, names(&["a", "b", "c"]));
    let advice = RoutingAdvice {
        down: BTreeSet::from(["a".to_owned(), "b".to_owned(), "zz".to_owned()]),
    };
    let with = plan(
        rid(),
        &remotes,
        &policies,
        &BTreeSet::new(),
        Some(&advice),
        NOW,
    );
    assert_eq!(with.order, names(&["c", "a", "b"]));
    assert_eq!(with.skipped, vec![("x".into(), SkipReason::NoPolicy)]);
    let empty = RoutingAdvice::default();
    let same = plan(
        rid(),
        &remotes,
        &policies,
        &BTreeSet::new(),
        Some(&empty),
        NOW,
    );
    assert_eq!(same, none);
}

fn search_over(order: &[&str], limit: usize, stop_at_limit: bool, deadline: u64) -> Search {
    let remotes: Vec<_> = order.iter().map(|p| entry(p, 0)).collect();
    let p = plan(
        rid(),
        &remotes,
        &trusted(order),
        &BTreeSet::new(),
        None,
        NOW,
    );
    Search::new(p, limit, stop_at_limit, deadline)
}

#[test]
fn local_answers_first_and_once() {
    let mut s = search_over(&["a"], 1, true, 100);
    assert_eq!(s.next(0), Err(SearchError::LocalNotFirst));
    assert_eq!(
        s.record_remote("a", Answer::NotFound),
        Err(SearchError::NotAsked)
    );
    s.record_local(Answer::NotFound).unwrap();
    assert_eq!(
        s.record_local(Answer::Found(ids(&[1]))),
        Err(SearchError::LocalAlreadyAnswered)
    );
    assert!(s.outcome().results.is_empty());
    assert!(s.outcome().local_not_found);
}

#[test]
fn local_results_alone_can_meet_the_limit() {
    let mut s = search_over(&["a", "b"], 2, true, 100);
    s.record_local(Answer::Found(ids(&[1, 2]))).unwrap();
    assert_eq!(s.next(0), Ok(Next::Stop(StopReason::LimitReached)));
    let o = s.outcome();
    assert_eq!(o.not_asked, names(&["a", "b"]));
    assert!(!o.local_not_found);
    assert_eq!(o.results[&id(1)], BTreeSet::from([Source::Local]));
}

#[test]
fn a_remote_negative_is_absence_at_that_peer_only() {
    let mut s = search_over(&["a", "b", "c"], 1, true, 100);
    s.record_local(Answer::NotFound).unwrap();
    assert_eq!(s.next(0), Ok(Next::Ask("a".into())));
    s.record_remote("a", Answer::NotFound).unwrap();
    assert_eq!(s.next(0), Ok(Next::Ask("b".into())));
    assert_eq!(s.outcome().absent_at, names(&["a"]));
    assert_eq!(s.outcome().stopped, None);
    s.record_remote("b", Answer::Found(ids(&[9]))).unwrap();
    assert_eq!(s.next(0), Ok(Next::Stop(StopReason::LimitReached)));
    let o = s.outcome();
    assert_eq!(
        o.results[&id(9)],
        BTreeSet::from([Source::Remote("b".into())])
    );
    assert_eq!(o.absent_at, names(&["a"]));
    assert_eq!(o.not_asked, names(&["c"]));
}

#[test]
fn all_negatives_still_name_each_peer_and_stop_as_exhausted() {
    let mut s = search_over(&["a", "b"], 1, true, 100);
    s.record_local(Answer::NotFound).unwrap();
    for peer in ["a", "b"] {
        assert_eq!(s.next(0), Ok(Next::Ask(peer.into())));
        let answer = if peer == "a" {
            Answer::NotFound
        } else {
            Answer::Failed
        };
        s.record_remote(peer, answer).unwrap();
    }
    assert_eq!(s.next(0), Ok(Next::Stop(StopReason::Exhausted)));
    assert_eq!(s.next(0), Ok(Next::Stop(StopReason::Exhausted)));
    let o = s.outcome();
    assert!(o.results.is_empty());
    assert_eq!(o.absent_at, names(&["a"]));
    assert_eq!(o.failed_at, names(&["b"]));
    assert_eq!(o.not_asked, [] as [String; 0]);
    assert_eq!(o.pending, None);
}

#[test]
fn duplicates_across_peers_count_once() {
    let mut s = search_over(&["a", "b"], 2, true, 100);
    s.record_local(Answer::Found(ids(&[1]))).unwrap();
    assert_eq!(s.next(0), Ok(Next::Ask("a".into())));
    s.record_remote("a", Answer::Found(ids(&[1]))).unwrap();
    assert_eq!(s.next(0), Ok(Next::Ask("b".into())));
    s.record_remote("b", Answer::Found(ids(&[2]))).unwrap();
    assert_eq!(s.next(0), Ok(Next::Stop(StopReason::LimitReached)));
    let o = s.outcome();
    assert_eq!(
        o.results[&id(1)],
        BTreeSet::from([Source::Local, Source::Remote("a".into())])
    );
    assert_eq!(o.results.len(), 2);
}

#[test]
fn without_stop_at_limit_every_remote_is_asked() {
    let mut s = search_over(&["a", "b"], 1, false, 100);
    s.record_local(Answer::Found(ids(&[1, 2]))).unwrap();
    assert_eq!(s.next(0), Ok(Next::Ask("a".into())));
    s.record_remote("a", Answer::Found(ids(&[3]))).unwrap();
    assert_eq!(s.next(0), Ok(Next::Ask("b".into())));
    s.record_remote("b", Answer::NotFound).unwrap();
    assert_eq!(s.next(0), Ok(Next::Stop(StopReason::Exhausted)));
}

#[test]
fn deadline_stops_and_marks_the_peer_in_flight_failed() {
    let mut s = search_over(&["a", "b"], 5, true, 10);
    s.record_local(Answer::NotFound).unwrap();
    assert_eq!(s.next(9), Ok(Next::Ask("a".into())));
    assert_eq!(s.next(9), Ok(Next::Ask("a".into())));
    assert_eq!(s.outcome().pending, Some("a".into()));
    assert_eq!(s.outcome().not_asked, names(&["b"]));
    assert_eq!(
        s.record_remote("b", Answer::NotFound),
        Err(SearchError::NotAsked)
    );
    assert_eq!(s.next(10), Ok(Next::Stop(StopReason::Deadline)));
    assert_eq!(
        s.record_remote("a", Answer::Found(ids(&[1]))),
        Err(SearchError::NotAsked)
    );
    let o = s.outcome();
    assert_eq!(o.failed_at, names(&["a"]));
    assert_eq!(o.pending, None);
    assert_eq!(o.not_asked, names(&["b"]));
    assert!(o.results.is_empty());
    assert_eq!(o.absent_at, [] as [String; 0]);
}

#[test]
fn a_peer_answers_at_most_once() {
    let mut s = search_over(&["a", "b"], 5, true, 100);
    s.record_local(Answer::NotFound).unwrap();
    assert_eq!(s.next(0), Ok(Next::Ask("a".into())));
    s.record_remote("a", Answer::NotFound).unwrap();
    assert_eq!(
        s.record_remote("a", Answer::Found(ids(&[1]))),
        Err(SearchError::NotAsked)
    );
    assert!(s.outcome().results.is_empty());
}

// ---- reference model ------------------------------------------------

const PEERS: [&str; 6] = ["p0", "p1", "p2", "p3", "p4", "p5"];

#[derive(Debug, Clone, Copy)]
enum Cfg {
    Missing,
    Class(u8),
    Expired,
    NotYet,
}

const fn class_of(n: u8) -> PeerClass {
    match n % 4 {
        0 => PeerClass::FullyTrusted,
        1 => PeerClass::SendOnly,
        2 => PeerClass::AcceptSignedFrom,
        _ => PeerClass::Unknown,
    }
}

fn reference_plan(
    remotes: &[RemoteEntry],
    cfg: &BTreeMap<String, Cfg>,
    disabled: &BTreeSet<String>,
    advice: Option<&BTreeSet<String>>,
) -> (Vec<String>, Vec<(String, SkipReason)>) {
    let mut eligible: Vec<&RemoteEntry> = Vec::new();
    let mut skipped = Vec::new();
    let mut first = BTreeSet::new();
    for e in remotes {
        let reason = if first.contains(&e.peer_id) {
            Some(SkipReason::DuplicateEntry)
        } else {
            first.insert(e.peer_id.clone());
            match cfg.get(&e.peer_id).copied().unwrap_or(Cfg::Missing) {
                Cfg::Missing => Some(SkipReason::NoPolicy),
                Cfg::Expired | Cfg::NotYet => Some(SkipReason::PolicyNotInForce),
                Cfg::Class(n) if n % 2 == 1 => Some(SkipReason::ClassForbidsQuery),
                Cfg::Class(_) if disabled.contains(&e.peer_id) => Some(SkipReason::Disabled),
                Cfg::Class(_) => None,
            }
        };
        match reason {
            Some(r) => skipped.push((e.peer_id.clone(), r)),
            None => eligible.push(e),
        }
    }
    let is_down = |p: &String| advice.is_some_and(|d| d.contains(p));
    eligible.sort_by_key(|e| (is_down(&e.peer_id), e.priority));
    (
        eligible.into_iter().map(|e| e.peer_id.clone()).collect(),
        skipped,
    )
}

fn policies_from(cfg: &BTreeMap<String, Cfg>) -> BTreeMap<String, PeerPolicy> {
    let mut out = BTreeMap::new();
    for (peer, c) in cfg {
        let p = match c {
            Cfg::Missing => continue,
            Cfg::Class(n) => policy(peer, class_of(*n)),
            Cfg::Expired => {
                let mut p = policy(peer, PeerClass::FullyTrusted);
                p.valid_from = Some(NOW - 10);
                p.expires_at = Some(NOW);
                p
            }
            Cfg::NotYet => {
                let mut p = policy(peer, PeerClass::AcceptSignedFrom);
                p.valid_from = Some(NOW + 1);
                p
            }
        };
        out.insert(peer.clone(), p);
    }
    out
}

fn arb_cfg() -> impl Strategy<Value = Cfg> {
    prop_oneof![
        Just(Cfg::Missing),
        Just(Cfg::Expired),
        Just(Cfg::NotYet),
        (0u8..4).prop_map(Cfg::Class),
        Just(Cfg::Class(0)),
        Just(Cfg::Class(2)),
    ]
}

#[derive(Debug, Clone)]
struct Setup {
    remotes: Vec<RemoteEntry>,
    cfg: BTreeMap<String, Cfg>,
    disabled: BTreeSet<String>,
    advice: Option<BTreeSet<String>>,
}

fn arb_setup() -> impl Strategy<Value = Setup> {
    (
        prop::collection::vec((0usize..PEERS.len(), 0u32..4), 0..10),
        prop::collection::vec(arb_cfg(), PEERS.len()),
        prop::collection::vec(any::<bool>(), PEERS.len()),
        prop::option::of(prop::collection::vec(any::<bool>(), PEERS.len())),
    )
        .prop_map(|(remotes, cfgs, disabled, down)| Setup {
            remotes: remotes
                .into_iter()
                .map(|(i, prio)| entry(PEERS[i], prio))
                .collect(),
            cfg: PEERS
                .iter()
                .zip(cfgs)
                .map(|(p, c)| ((*p).to_owned(), c))
                .collect(),
            disabled: PEERS
                .iter()
                .zip(disabled)
                .filter(|(_, d)| *d)
                .map(|(p, _)| (*p).to_owned())
                .collect(),
            advice: down.map(|d| {
                PEERS
                    .iter()
                    .zip(d)
                    .filter(|(_, d)| *d)
                    .map(|(p, _)| (*p).to_owned())
                    .collect()
            }),
        })
}

fn run_plan(s: &Setup) -> Plan {
    let advice = s.advice.clone().map(|down| RoutingAdvice { down });
    plan(
        rid(),
        &s.remotes,
        &policies_from(&s.cfg),
        &s.disabled,
        advice.as_ref(),
        NOW,
    )
}

#[derive(Debug, Clone)]
enum Reply {
    Found(Vec<u8>),
    NotFound,
    Failed,
}

fn arb_reply() -> impl Strategy<Value = Reply> {
    prop_oneof![
        prop::collection::vec(0u8..6, 0..3).prop_map(Reply::Found),
        Just(Reply::NotFound),
        Just(Reply::Failed),
    ]
}

fn to_answer(r: &Reply) -> Answer {
    match r {
        Reply::Found(v) => Answer::Found(ids(v)),
        Reply::NotFound => Answer::NotFound,
        Reply::Failed => Answer::Failed,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn plan_matches_the_reference_and_is_deterministic(s in arb_setup()) {
        let p = run_plan(&s);
        let (order, skipped) =
            reference_plan(&s.remotes, &s.cfg, &s.disabled, s.advice.as_ref());
        prop_assert_eq!(&p.order, &order);
        prop_assert_eq!(&p.skipped, &skipped);
        prop_assert_eq!(&run_plan(&s), &p);
        // Advice never changes which remotes are asked.
        let mut plain = s.clone();
        plain.advice = None;
        let q = run_plan(&plain);
        let a: BTreeSet<_> = p.order.iter().collect();
        let b: BTreeSet<_> = q.order.iter().collect();
        prop_assert_eq!(a, b);
        prop_assert_eq!(&p.skipped, &q.skipped);
        // Every configured remote is either ordered once or skipped.
        prop_assert_eq!(p.order.len() + p.skipped.len(), s.remotes.len());
    }

    #[test]
    fn distinct_priorities_make_list_order_irrelevant(
        prios in prop::collection::btree_set(0u32..100, 1..6),
        seed in any::<u64>(),
    ) {
        let remotes: Vec<_> = prios
            .iter()
            .enumerate()
            .map(|(i, prio)| entry(PEERS[i], *prio))
            .collect();
        let mut shuffled = remotes.clone();
        let mut x = seed | 1;
        for i in (1..shuffled.len()).rev() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            shuffled.swap(i, usize::try_from(x % (i as u64 + 1)).unwrap());
        }
        let policies = trusted(&PEERS);
        let a = plan(rid(), &remotes, &policies, &BTreeSet::new(), None, NOW);
        let b = plan(rid(), &shuffled, &policies, &BTreeSet::new(), None, NOW);
        prop_assert_eq!(&a.order, &b.order);
        let expected: Vec<_> = remotes.iter().map(|e| e.peer_id.clone()).collect();
        prop_assert_eq!(a.order, expected);
    }

    #[test]
    fn search_matches_the_reference(
        s in arb_setup(),
        local in arb_reply(),
        replies in prop::collection::vec(arb_reply(), PEERS.len()),
        limit in 0usize..5,
        stop_at_limit in any::<bool>(),
        deadline in 0u64..8,
    ) {
        let p = run_plan(&s);
        let order = p.order.clone();
        let reply_of = |peer: &str| {
            replies[PEERS.iter().position(|q| *q == peer).unwrap()].clone()
        };
        let mut search = Search::new(p, limit, stop_at_limit, deadline);
        search.record_local(to_answer(&local)).unwrap();

        // Reference walk: one tick per remote asked.
        let mut found: BTreeMap<ResultId, BTreeSet<Source>> = BTreeMap::new();
        if let Reply::Found(v) = &local {
            for n in v {
                found.entry(id(*n)).or_default().insert(Source::Local);
            }
        }
        let mut absent = Vec::new();
        let mut failed = Vec::new();
        let mut asked = 0usize;
        let mut now = 0u64;
        let stop = loop {
            if stop_at_limit && found.len() >= limit {
                break StopReason::LimitReached;
            }
            if now >= deadline {
                break StopReason::Deadline;
            }
            let Some(peer) = order.get(asked) else { break StopReason::Exhausted };
            prop_assert_eq!(search.next(now).unwrap(), Next::Ask(peer.clone()));
            let r = reply_of(peer);
            match &r {
                Reply::Found(v) => {
                    for n in v {
                        found.entry(id(*n)).or_default().insert(Source::Remote(peer.clone()));
                    }
                }
                Reply::NotFound => absent.push(peer.clone()),
                Reply::Failed => failed.push(peer.clone()),
            }
            search.record_remote(peer, to_answer(&r)).unwrap();
            asked += 1;
            now += 1;
        };
        prop_assert_eq!(search.next(now).unwrap(), Next::Stop(stop));
        let o = search.outcome();
        prop_assert_eq!(&o.results, &found);
        prop_assert_eq!(&o.absent_at, &absent);
        prop_assert_eq!(&o.failed_at, &failed);
        prop_assert_eq!(&o.not_asked, &order[asked..].to_vec());
        prop_assert_eq!(o.stopped, Some(stop));
        prop_assert_eq!(o.local_not_found, matches!(local, Reply::NotFound));
        // A negative is never global: every id found anywhere is kept,
        // even if other peers said "not found".
        for (peer, r) in order[..asked].iter().map(|p| (p, reply_of(p))) {
            if let Reply::Found(v) = r {
                for n in v {
                    prop_assert!(o.results[&id(n)].contains(&Source::Remote(peer.clone())));
                }
            }
        }
    }
}
