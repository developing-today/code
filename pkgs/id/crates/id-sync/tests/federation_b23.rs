//! B23 golden and model test: routing-state freshness, periodic
//! validation and change detection (#10251, statements S1.73-S1.76).
//!
//! Oracle: **golden** — an expired or poisoned route observation is
//! never treated as current trusted topology, and the partial graph
//! stays explicitly partial. The observation encoding is pinned to hex
//! computed independently (Python, from the documented layout), and a
//! fixed fixture of signed observations is replayed step by step against
//! expected outcomes and topologies written out by hand. A reference
//! model then replays random streams of honest, poisoned, replayed,
//! reordered and expiring observations and predicts every admission and
//! every topology by brute force from the latest valid statement of each
//! observer.
//!
//! The slice modules are loaded by path: the federation roots record
//! them without declaring them (slice 9110-02), and B23 reaches B32
//! through `super::`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/federation/slices/b32_ingest_gate.rs"]
#[allow(dead_code)]
mod b32_ingest_gate;

#[path = "../src/federation/slices/b23_routing_state.rs"]
mod b23_routing_state;

use std::collections::{BTreeMap, BTreeSet};

use b23_routing_state::{
    LinkStatus, PartialTopology, PeerId, Refusal, RouteObservation, RoutingMode, RoutingPolicy,
    RoutingState, TopologyChange, changes,
};
use b32_ingest_gate::{
    Envelope, FreshnessWindow, IngestGate, IngressPath, RejectReason, SignerAuthorization,
    sign_envelope,
};
use ed25519_dalek::SigningKey;
use proptest::prelude::*;

const SCOPE: &str = "routing/public";
const NAMES: [&str; 4] = ["alice", "bob", "carol", "dave"];

fn key(name: &str) -> SigningKey {
    let seed = NAMES
        .iter()
        .position(|n| *n == name)
        .map_or(99, |i| u8::try_from(i).unwrap() + 1);
    SigningKey::from_bytes(&[seed; 32])
}

fn gate() -> IngestGate {
    let signer = |name: &str, scope: &str| SignerAuthorization {
        identity: name.to_owned(),
        verifying_key: key(name).verifying_key().to_bytes(),
        scopes: BTreeSet::from([scope.to_owned()]),
    };
    IngestGate::new(
        vec![
            signer("alice", SCOPE),
            signer("bob", SCOPE),
            signer("carol", SCOPE),
            signer("dave", "routing/other"),
        ],
        FreshnessWindow {
            max_age: 1000,
            max_future_skew: 5,
        },
    )
}

fn policy() -> RoutingPolicy {
    RoutingPolicy {
        mode: RoutingMode::Dynamic,
        max_ttl: 100,
        revalidate_after: 40,
        configured_peers: ["alice", "bob", "carol", "erin"]
            .into_iter()
            .map(String::from)
            .collect(),
        static_links: BTreeSet::from([("alice".to_owned(), "erin".to_owned())]),
    }
}

fn state() -> RoutingState {
    RoutingState::new(SCOPE.to_owned(), gate(), policy())
}

fn obs(observer: &str, at: u64, ttl: u64, links: &[(&str, LinkStatus)]) -> RouteObservation {
    RouteObservation {
        observer: observer.to_owned(),
        scope: SCOPE.to_owned(),
        observed_at: at,
        expires_at: at + ttl,
        links: links
            .iter()
            .map(|(peer, status)| ((*peer).to_owned(), *status))
            .collect(),
    }
}

fn envelope(signer: &str, o: &RouteObservation, event: u8) -> Envelope {
    sign_envelope(
        &key(signer),
        signer,
        &o.scope,
        &o.encode(),
        [event; 32],
        o.observed_at,
    )
}

fn set(items: &[&str]) -> BTreeSet<PeerId> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

fn links(items: &[(&str, &str)]) -> BTreeSet<(PeerId, PeerId)> {
    items
        .iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect()
}

use LinkStatus::{Reachable as Up, Unreachable as Down};

// ----- encoding --------------------------------------------------------

#[test]
fn the_payload_encoding_is_pinned() {
    let o = obs("alice", 10, 50, &[("bob", Up), ("carol", Down)]);
    assert_eq!(hex(&o.encode()), GOLDEN_PAYLOAD);
    assert_eq!(RouteObservation::decode(&o.encode()), Some(o));
}

const GOLDEN_PAYLOAD: &str = "69642d73796e632f6232332d726f7574652d6f62736572766174696f6e2f763100000005616c6963650000000e726f7574696e672f7075626c6963000000000000000a000000000000003c0000000200000003626f6201000000056361726f6c02";

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        write!(out, "{b:02x}").unwrap();
        out
    })
}

#[test]
fn only_canonical_payloads_decode() {
    let good = obs("alice", 10, 50, &[("bob", Up), ("carol", Down)]).encode();
    let decode = RouteObservation::decode;
    assert!(decode(&good).is_some());

    let mut trailing = good.clone();
    trailing.push(0);
    assert_eq!(decode(&trailing), None);
    assert_eq!(decode(&good[..good.len() - 1]), None);

    let mut bad_status = good.clone();
    *bad_status.last_mut().unwrap() = 3;
    assert_eq!(decode(&bad_status), None);

    let mut domain = good;
    domain[3] ^= 1;
    assert_eq!(decode(&domain), None);

    // Hand-built: links out of order, repeated, to the observer itself,
    // and an empty observer.
    let raw = |observer: &str, peers: &[&str]| {
        let mut out = b"id-sync/b23-route-observation/v1".to_vec();
        for s in [observer, SCOPE] {
            out.extend_from_slice(&u32::try_from(s.len()).unwrap().to_be_bytes());
            out.extend_from_slice(s.as_bytes());
        }
        out.extend_from_slice(&10u64.to_be_bytes());
        out.extend_from_slice(&60u64.to_be_bytes());
        out.extend_from_slice(&u32::try_from(peers.len()).unwrap().to_be_bytes());
        for p in peers {
            out.extend_from_slice(&u32::try_from(p.len()).unwrap().to_be_bytes());
            out.extend_from_slice(p.as_bytes());
            out.push(1);
        }
        out
    };
    assert!(decode(&raw("alice", &["bob", "carol"])).is_some());
    assert_eq!(decode(&raw("alice", &["carol", "bob"])), None);
    assert_eq!(decode(&raw("alice", &["bob", "bob"])), None);
    assert_eq!(decode(&raw("alice", &["alice"])), None);
    assert_eq!(decode(&raw("", &["bob"])), None);
    // A count larger than the links present.
    let mut short = raw("alice", &["bob"]);
    let at = short.len() - (4 + 3 + 1) - 4;
    short[at + 3] = 2;
    assert_eq!(decode(&short), None);
}

// ----- golden fixture ----------------------------------------------------

#[test]
fn the_fixture_replays_to_the_expected_topologies() {
    let mut s = state();
    let path = IngressPath::Remote;

    // Nothing observed: everything configured is unobserved and due.
    assert_eq!(
        s.topology(0),
        PartialTopology {
            unobserved: set(&["alice", "bob", "carol", "erin"]),
            ..PartialTopology::default()
        }
    );
    assert_eq!(s.validation_due(0), set(&["alice", "bob", "carol", "erin"]));

    let a1 = obs("alice", 10, 50, &[("bob", Up), ("carol", Down)]);
    assert_eq!(s.ingest(path, &envelope("alice", &a1, 1), 10), Ok(()));
    let b1 = obs("bob", 12, 30, &[("alice", Up), ("dave", Up)]);
    assert_eq!(s.ingest(path, &envelope("bob", &b1, 2), 12), Ok(()));
    let t20 = s.topology(20);
    assert_eq!(
        t20,
        PartialTopology {
            links: links(&[("alice", "bob"), ("bob", "alice"), ("bob", "dave")]),
            down: links(&[("alice", "carol")]),
            current_observers: set(&["alice", "bob"]),
            stale_observers: set(&[]),
            unobserved: set(&["erin"]),
        }
    );
    assert_eq!(t20.reachable(), set(&["alice", "bob", "dave"]));
    assert_eq!(s.validation_due(20), set(&["carol", "erin"]));

    // Bob's statement expires at 42: it no longer vouches for anything.
    let t45 = s.topology(45);
    assert_eq!(
        t45,
        PartialTopology {
            links: links(&[("alice", "bob")]),
            down: links(&[("alice", "carol")]),
            current_observers: set(&["alice"]),
            stale_observers: set(&["bob"]),
            unobserved: set(&["erin"]),
        }
    );
    assert_eq!(
        changes(&t20, &t45),
        TopologyChange {
            lost_sight: set(&["dave"]),
            went_stale: set(&["bob"]),
            links_lost: links(&[("bob", "alice"), ("bob", "dave")]),
            ..TopologyChange::default()
        }
    );
    // Alice is 35 old at 45: not yet due; at 50 she is.
    assert_eq!(s.validation_due(45), set(&["bob", "carol", "erin"]));
    assert_eq!(
        s.validation_due(50),
        set(&["alice", "bob", "carol", "erin"])
    );

    // Alice re-probes: bob is now down, carol is back.
    let a2 = obs("alice", 50, 50, &[("bob", Down), ("carol", Up)]);
    assert_eq!(s.ingest(path, &envelope("alice", &a2, 3), 50), Ok(()));
    let t55 = s.topology(55);
    assert_eq!(
        changes(&t45, &t55),
        TopologyChange {
            appeared: set(&["carol"]),
            vanished: set(&["bob"]),
            links_added: links(&[("alice", "carol")]),
            links_lost: links(&[("alice", "bob")]),
            ..TopologyChange::default()
        }
    );
    assert!(changes(&t55, &t55).is_empty());
    for only in [
        TopologyChange {
            appeared: set(&["x"]),
            ..TopologyChange::default()
        },
        TopologyChange {
            vanished: set(&["x"]),
            ..TopologyChange::default()
        },
        TopologyChange {
            lost_sight: set(&["x"]),
            ..TopologyChange::default()
        },
        TopologyChange {
            went_stale: set(&["x"]),
            ..TopologyChange::default()
        },
        TopologyChange {
            links_added: links(&[("x", "y")]),
            ..TopologyChange::default()
        },
        TopologyChange {
            links_lost: links(&[("x", "y")]),
            ..TopologyChange::default()
        },
    ] {
        assert!(!only.is_empty(), "{only:?}");
    }

    // The older statement arriving late changes nothing.
    assert_eq!(
        s.ingest(path, &envelope("alice", &a1, 4), 55),
        Err(Refusal::Superseded)
    );
    let mut same_time = a2;
    same_time.links.insert("dave".to_owned(), Up);
    assert_eq!(
        s.ingest(path, &envelope("alice", &same_time, 5), 55),
        Err(Refusal::Conflict)
    );
    assert_eq!(s.topology(55), t55);

    // Everything expires: no current link survives, and pruning drops it.
    let t200 = s.topology(200);
    assert!(t200.links.is_empty() && t200.down.is_empty());
    assert_eq!(t200.stale_observers, set(&["alice", "bob"]));
    assert_eq!(t200.unobserved, set(&["alice", "bob", "carol", "erin"]));
    assert_eq!(s.prune(200), 2);
    assert_eq!(s.topology(200).stale_observers, set(&[]));
}

#[test]
fn poisoned_observations_never_enter_the_topology() {
    let mut s = state();
    let path = IngressPath::Remote;
    let honest = obs("alice", 10, 50, &[("bob", Up)]);
    let try_it = |s: &mut RoutingState, e: &Envelope| s.ingest(path, e, 12);

    let mut unsigned = envelope("alice", &honest, 1);
    unsigned.signature = None;
    assert_eq!(
        try_it(&mut s, &unsigned),
        Err(Refusal::Gate(RejectReason::Unsigned))
    );

    // Bob's key claiming to be alice.
    let impostor = sign_envelope(&key("bob"), "alice", SCOPE, &honest.encode(), [2; 32], 10);
    assert!(matches!(
        try_it(&mut s, &impostor),
        Err(Refusal::Gate(RejectReason::SignerMismatch { .. }))
    ));

    // Bob signing a statement about alice's links.
    assert_eq!(
        try_it(&mut s, &envelope("bob", &honest, 3)),
        Err(Refusal::ObserverMismatch)
    );

    // Bytes changed in transit.
    let mut tampered = envelope("alice", &honest, 4);
    tampered.payload[40] ^= 1;
    assert_eq!(
        try_it(&mut s, &tampered),
        Err(Refusal::Gate(RejectReason::SemanticInvalid))
    );

    // Signed garbage.
    let garbage = sign_envelope(&key("alice"), "alice", SCOPE, b"not a route", [5; 32], 10);
    assert_eq!(try_it(&mut s, &garbage), Err(Refusal::Malformed));

    // Payload scope differs from the envelope's.
    let mut other_scope = honest.clone();
    other_scope.scope = "routing/other".to_owned();
    let e = sign_envelope(
        &key("alice"),
        "alice",
        SCOPE,
        &other_scope.encode(),
        [6; 32],
        10,
    );
    assert_eq!(try_it(&mut s, &e), Err(Refusal::ScopeMismatch));

    // Dave may publish only for another scope.
    let mut daves = obs("dave", 10, 50, &[("bob", Up)]);
    assert!(matches!(
        try_it(&mut s, &envelope("dave", &daves, 7)),
        Err(Refusal::Gate(RejectReason::NotAuthorized { .. }))
    ));
    daves.scope = "routing/other".to_owned();
    assert_eq!(
        try_it(&mut s, &envelope("dave", &daves, 8)),
        Err(Refusal::ScopeMismatch)
    );

    // Time and expiry.
    let e = sign_envelope(&key("alice"), "alice", SCOPE, &honest.encode(), [9; 32], 11);
    assert_eq!(try_it(&mut s, &e), Err(Refusal::TimeMismatch));
    assert_eq!(
        try_it(&mut s, &envelope("alice", &obs("alice", 10, 0, &[]), 10)),
        Err(Refusal::BadExpiry)
    );
    assert_eq!(
        try_it(&mut s, &envelope("alice", &obs("alice", 10, 101, &[]), 11)),
        Err(Refusal::TtlTooLong)
    );

    // Too old for the gate, and dated in the future.
    assert_eq!(
        s.ingest(path, &envelope("alice", &honest, 12), 1011),
        Err(Refusal::Gate(RejectReason::Stale))
    );
    assert_eq!(
        s.ingest(path, &envelope("alice", &obs("alice", 20, 50, &[]), 13), 14),
        Err(Refusal::Gate(RejectReason::FutureSkew))
    );

    assert_eq!(s.topology(12), state().topology(12));

    // The honest one is accepted once; its replay is refused by the gate.
    let e = envelope("alice", &honest, 14);
    assert_eq!(try_it(&mut s, &e), Ok(()));
    assert_eq!(try_it(&mut s, &e), Err(Refusal::Gate(RejectReason::Replay)));
    assert_eq!(s.topology(12).links, links(&[("alice", "bob")]));
}

#[test]
fn static_mode_ignores_observations_and_validates_nothing() {
    let mut s = state();
    let a = obs("alice", 10, 50, &[("bob", Up)]);
    s.ingest(IngressPath::Local, &envelope("alice", &a, 1), 10)
        .unwrap();
    s.set_mode(RoutingMode::Static);
    assert_eq!(
        s.topology(20),
        PartialTopology {
            links: links(&[("alice", "erin")]),
            unobserved: set(&["bob", "carol"]),
            ..PartialTopology::default()
        }
    );
    assert!(s.validation_due(20).is_empty());
    // Dynamic state still expires underneath.
    assert_eq!(s.prune(60), 1);
    s.set_mode(RoutingMode::Dynamic);
    assert!(s.topology(20).links.is_empty());
    assert_eq!(s.observation("alice"), None);
}

#[test]
fn every_ingress_path_faces_the_same_checks() {
    for (i, path) in IngressPath::ALL.into_iter().enumerate() {
        let mut s = state();
        let event = u8::try_from(i).unwrap();
        let honest = obs("alice", 10, 50, &[("bob", Up)]);
        assert_eq!(
            s.ingest(path, &envelope("bob", &honest, event), 12),
            Err(Refusal::ObserverMismatch)
        );
        assert_eq!(
            s.ingest(path, &envelope("alice", &honest, event + 10), 12),
            Ok(())
        );
    }
}

// ----- reference model -----------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Poison {
    None,
    Unsigned,
    ObserverMismatch,
    Tampered,
    TimeMismatch,
    TtlTooLong,
    Replay,
}

#[derive(Debug, Clone)]
struct Step {
    advance: u64,
    observer: usize,
    age: u64,
    ttl: u64,
    links: Vec<(usize, bool)>,
    poison: Poison,
}

fn step() -> impl Strategy<Value = Step> {
    (
        0u64..20,
        0usize..3,
        0u64..15,
        1u64..60,
        prop::collection::vec((0usize..5, any::<bool>()), 0..4),
        prop_oneof![
            8 => Just(Poison::None),
            1 => Just(Poison::Unsigned),
            1 => Just(Poison::ObserverMismatch),
            1 => Just(Poison::Tampered),
            1 => Just(Poison::TimeMismatch),
            1 => Just(Poison::TtlTooLong),
            1 => Just(Poison::Replay),
        ],
    )
        .prop_map(|(advance, observer, age, ttl, links, poison)| Step {
            advance,
            observer,
            age,
            ttl,
            links,
            poison,
        })
}

const OBSERVERS: [&str; 3] = ["alice", "bob", "carol"];
const TARGETS: [&str; 5] = ["alice", "bob", "carol", "dave", "erin"];

fn expected_topology(held: &BTreeMap<String, RouteObservation>, now: u64) -> PartialTopology {
    let mut t = PartialTopology::default();
    for o in held.values() {
        if now >= o.expires_at {
            t.stale_observers.insert(o.observer.clone());
            continue;
        }
        t.current_observers.insert(o.observer.clone());
        for (peer, status) in &o.links {
            let link = (o.observer.clone(), peer.clone());
            if *status == Up {
                t.links.insert(link);
            } else {
                t.down.insert(link);
            }
        }
    }
    for peer in &policy().configured_peers {
        let seen = t.current_observers.contains(peer)
            || t.links
                .iter()
                .chain(&t.down)
                .any(|(a, b)| a == peer || b == peer);
        if !seen {
            t.unobserved.insert(peer.clone());
        }
    }
    t
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 96, ..ProptestConfig::default() })]

    #[test]
    fn admissions_and_topologies_match_the_reference(steps in prop::collection::vec(step(), 1..40)) {
        let mut s = state();
        let mut held: BTreeMap<String, RouteObservation> = BTreeMap::new();
        let mut now = 20u64;
        let mut last: Option<Envelope> = None;
        let mut previous = s.topology(now);
        for (i, st) in steps.iter().enumerate() {
            now += st.advance;
            let observer = OBSERVERS[st.observer];
            let mut o = RouteObservation {
                observer: observer.to_owned(),
                scope: SCOPE.to_owned(),
                observed_at: now - st.age,
                expires_at: now - st.age + st.ttl,
                links: BTreeMap::new(),
            };
            for (target, up) in &st.links {
                if TARGETS[*target] != observer {
                    o.links.insert(TARGETS[*target].to_owned(), if *up { Up } else { Down });
                }
            }
            let event = u8::try_from(i).unwrap();
            let (env, want): (Envelope, Result<(), Refusal>) = match st.poison {
                Poison::Replay if last.is_some() => {
                    (last.clone().unwrap(), Err(Refusal::Gate(RejectReason::Replay)))
                }
                Poison::None | Poison::Replay => {
                    let want = match held.get(observer) {
                        Some(h) if h.observed_at > o.observed_at || *h == o => Err(Refusal::Superseded),
                        Some(h) if h.observed_at == o.observed_at => Err(Refusal::Conflict),
                        _ => Ok(()),
                    };
                    (envelope(observer, &o, event), want)
                }
                Poison::Unsigned => {
                    let mut e = envelope(observer, &o, event);
                    e.signature = None;
                    (e, Err(Refusal::Gate(RejectReason::Unsigned)))
                }
                Poison::ObserverMismatch => {
                    let other = OBSERVERS[(st.observer + 1) % 3];
                    (envelope(other, &o, event), Err(Refusal::ObserverMismatch))
                }
                Poison::Tampered => {
                    let mut e = envelope(observer, &o, event);
                    let n = e.payload.len();
                    e.payload[n - 1] ^= 1;
                    (e, Err(Refusal::Gate(RejectReason::SemanticInvalid)))
                }
                Poison::TimeMismatch => {
                    let e = sign_envelope(&key(observer), observer, SCOPE, &o.encode(), [event; 32], o.observed_at + 1);
                    (e, Err(Refusal::TimeMismatch))
                }
                Poison::TtlTooLong => {
                    o.expires_at = o.observed_at + 101;
                    (envelope(observer, &o, event), Err(Refusal::TtlTooLong))
                }
            };
            let got = s.ingest(IngressPath::Remote, &env, now);
            prop_assert_eq!(&got, &want, "step {} {:?}", i, st);
            if got.is_ok() {
                held.insert(observer.to_owned(), o);
                last = Some(env);
            }
            let topology = s.topology(now);
            prop_assert_eq!(&topology, &expected_topology(&held, now));
            // Nothing expired or refused contributes a link.
            for (a, _) in topology.links.iter().chain(&topology.down) {
                prop_assert!(now < held[a].expires_at);
            }
            let change = changes(&previous, &topology);
            let gone: BTreeSet<PeerId> =
                previous.reachable().difference(&topology.reachable()).cloned().collect();
            let mut split = change.vanished.clone();
            split.extend(change.lost_sight.iter().cloned());
            prop_assert_eq!(split, gone);
            prop_assert!(change.vanished.is_disjoint(&change.lost_sight));
            prop_assert!(change.appeared.is_disjoint(&previous.reachable()));
            prop_assert!(changes(&topology, &topology).is_empty());
            let due = s.validation_due(now);
            for peer in &policy().configured_peers {
                let fresh = held.get(peer).is_some_and(|h| now < h.expires_at && now < h.observed_at + 40);
                prop_assert_eq!(due.contains(peer), !fresh);
            }
            previous = topology;
        }
    }
}
