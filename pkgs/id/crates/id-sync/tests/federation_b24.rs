//! B24 invariant test: nonce-bound topology probes (#10265, statements
//! S1.77 and S1.78).
//!
//! Oracle: **invariant** — a probe response binds the nonce and the
//! peer, and stale, vanished and appeared edges update only this node's
//! local scoped routing state. Golden vectors for both signed preimages
//! were computed independently with Python `cryptography` (Ed25519) and
//! `hashlib`. A reference state machine in this file tracks, per peer,
//! when the next probe is due, the probe in flight and the current
//! evidence; random streams start probes, deliver honest and tampered
//! responses (another nonce, scope, prober or request, a bad signature,
//! an impossible time, an unknown responder, a late answer), expire
//! probes, disable and re-enable, and compare every outcome and the
//! resulting observation with the slice.
//!
//! The slice modules are loaded by path: the federation roots record
//! them without declaring them (slice 9110-02), and B24 reaches B23
//! through `super::`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/federation/slices/b32_ingest_gate.rs"]
#[allow(dead_code)]
mod b32_ingest_gate;

#[path = "../src/federation/slices/b23_routing_state.rs"]
#[allow(dead_code)]
mod b23_routing_state;

#[path = "../src/federation/slices/b24_topology_probe.rs"]
mod b24_topology_probe;

use std::collections::BTreeMap;
use std::fmt::Write as _;

use b23_routing_state::{LinkStatus, RouteObservation};
use b24_topology_probe::{
    ConfigError, Evidence, Nonce, ProbeConfig, ProbeError, ProbeRequest, ProbeResponse, Prober,
    RespondError, respond,
};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use proptest::prelude::*;

const REQ_PRE: &str = "69642d73796e632f6232342d70726f62652d726571756573742f76310000000161000000016200000001730707070707070707070707070707070707070707070707070707070707070707000000000000000a000000000000000f";
const REQ_SIG: &str = "61e040ee2fdf519cbef3c587034178d83c88d169c83456406e85a0896a773de8a5ef809d2b918de25566bf54f16c2ff9e3c3eb3e33421c65b84bdaf068e23309";
const REQ_DIG: &str = "55b50ac05166c58357e5ae1ef5636a2313eca460de2644ac125c799350dc9226";
const RESP_PRE: &str = "69642d73796e632f6232342d70726f62652d726573706f6e73652f7631000000016200000001610000000173070707070707070707070707070707070707070707070707070707070707070755b50ac05166c58357e5ae1ef5636a2313eca460de2644ac125c799350dc9226000000000000000c";
const RESP_SIG: &str = "20825285e52a01a6020b8d94409322618a75da17817274b397e716019cd3f432d8dcff453922735e9624ae2fd1880c4e8c3d77405667cc3f1fe87c482fe64001";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn pk(seed: u8) -> VerifyingKey {
    key(seed).verifying_key()
}

const CONFIG: ProbeConfig = ProbeConfig {
    interval: 10,
    jitter: 3,
    timeout: 5,
    ttl: 20,
};

const PEERS: [(&str, u8); 3] = [("b", 2), ("c", 3), ("d", 4)];

fn peers() -> BTreeMap<String, VerifyingKey> {
    PEERS
        .iter()
        .map(|(p, s)| ((*p).to_owned(), pk(*s)))
        .collect()
}

fn prober(now: u64) -> Prober {
    Prober::new("a".into(), key(1), "s".into(), CONFIG, peers(), now).unwrap()
}

fn seed_of(peer: &str) -> u8 {
    PEERS
        .iter()
        .find(|(p, _)| *p == peer)
        .map_or(9, |(_, s)| *s)
}

#[test]
fn golden_vectors() {
    let mut p = Prober::new(
        "a".into(),
        key(1),
        "s".into(),
        ProbeConfig {
            timeout: 5,
            ..CONFIG
        },
        peers(),
        10,
    )
    .unwrap();
    let req = p.start("b", [7; 32], 10).unwrap();
    assert_eq!(hex(&req.preimage()), REQ_PRE);
    assert_eq!(hex(&req.signature), REQ_SIG);
    assert_eq!(hex(&req.digest()), REQ_DIG);
    let resp = respond(&key(2), "b", &req, &pk(1), 12).unwrap();
    assert_eq!(hex(&resp.preimage()), RESP_PRE);
    assert_eq!(hex(&resp.signature), RESP_SIG);
    p.receive(&resp, 13).unwrap();
    assert_eq!(
        p.evidence("b", 13),
        Some(Evidence {
            status: LinkStatus::Reachable,
            observed_at: 13,
            expires_at: 33
        })
    );
}

#[test]
fn configuration_is_checked() {
    let mut with_self = peers();
    with_self.insert("a".into(), pk(1));
    assert_eq!(
        Prober::new("a".into(), key(1), "s".into(), CONFIG, with_self, 0).err(),
        Some(ConfigError::SelfProbe)
    );
    for bad in [
        ProbeConfig {
            timeout: 0,
            ..CONFIG
        },
        ProbeConfig { ttl: 0, ..CONFIG },
    ] {
        assert_eq!(
            Prober::new("a".into(), key(1), "s".into(), bad, peers(), 0).err(),
            Some(ConfigError::ZeroDuration)
        );
    }
}

#[test]
fn responder_answers_only_valid_probes_for_itself_in_time() {
    let mut p = prober(0);
    let req = p.start("b", [1; 32], 0).unwrap();
    assert_eq!(
        respond(&key(3), "c", &req, &pk(1), 1),
        Err(RespondError::NotForMe)
    );
    assert_eq!(
        respond(&key(2), "b", &req, &pk(4), 1),
        Err(RespondError::BadSignature)
    );
    let mut forged = req.clone();
    forged.scope = "other".into();
    assert_eq!(
        respond(&key(2), "b", &forged, &pk(1), 1),
        Err(RespondError::BadSignature)
    );
    assert_eq!(
        respond(&key(2), "b", &req, &pk(1), 5),
        Err(RespondError::Expired)
    );
    assert!(respond(&key(2), "b", &req, &pk(1), 4).is_ok());
}

#[test]
fn a_response_binds_nonce_and_peer() {
    let mut p = prober(0);
    let rb = p.start("b", [1; 32], 0).unwrap();
    let rc = p.start("c", [2; 32], 0).unwrap();
    let ok_b = respond(&key(2), "b", &rb, &pk(1), 1).unwrap();
    let ok_c = respond(&key(3), "c", &rc, &pk(1), 1).unwrap();

    // c answers b's probe under its own key: the response names c, so it
    // must match c's probe, whose nonce differs.
    let cross = respond(
        &key(3),
        "c",
        &ProbeRequest {
            target: "c".into(),
            ..rb.clone()
        },
        &pk(1),
        1,
    );
    assert_eq!(cross, Err(RespondError::BadSignature));
    let mut relabelled = ok_b.clone();
    relabelled.responder = "c".into();
    assert_eq!(p.receive(&relabelled, 2), Err(ProbeError::WrongNonce));
    // b's answer re-signed by c's key under b's name fails the signature.
    let mut stolen = ok_b.clone();
    stolen.signature = key(3).sign(&stolen.preimage()).to_bytes();
    assert_eq!(p.receive(&stolen, 2), Err(ProbeError::BadSignature));
    // Answers to the wrong request or nonce are refused.
    let mut other_nonce = ok_c.clone();
    other_nonce.nonce = [9; 32];
    assert_eq!(p.receive(&other_nonce, 2), Err(ProbeError::WrongNonce));
    let mut other_req = ok_c.clone();
    other_req.request_digest = rb.digest();
    assert_eq!(p.receive(&other_req, 2), Err(ProbeError::WrongRequest));
    assert!(p.in_flight("b") && p.in_flight("c"));
    assert_eq!(p.evidence("b", 2), None);

    p.receive(&ok_b, 2).unwrap();
    // The same answer again is unsolicited: the probe is closed.
    assert_eq!(p.receive(&ok_b, 2), Err(ProbeError::Unsolicited));
    p.receive(&ok_c, 3).unwrap();
}

#[test]
fn a_response_must_fall_between_send_and_receipt() {
    let mut p = prober(0);
    let rb = p.start("b", [1; 32], 3).unwrap();
    let resign = |r: &mut ProbeResponse| r.signature = key(2).sign(&r.preimage()).to_bytes();
    let mut early = respond(&key(2), "b", &rb, &pk(1), 3).unwrap();
    early.responded_at = 2;
    resign(&mut early);
    assert_eq!(p.receive(&early, 4), Err(ProbeError::BadTime));
    let mut future = respond(&key(2), "b", &rb, &pk(1), 3).unwrap();
    future.responded_at = 5;
    resign(&mut future);
    assert_eq!(p.receive(&future, 4), Err(ProbeError::BadTime));
    assert!(p.in_flight("b"));
    let at_send = respond(&key(2), "b", &rb, &pk(1), 3).unwrap();
    p.receive(&at_send, 3).unwrap();
}

#[test]
fn timeouts_mark_unreachable_and_late_answers_change_nothing() {
    let mut p = prober(0);
    let rb = p.start("b", [1; 32], 0).unwrap();
    let ok = respond(&key(2), "b", &rb, &pk(1), 4).unwrap();
    assert_eq!(p.receive(&ok, 5), Err(ProbeError::Late));
    assert!(p.in_flight("b"));
    assert_eq!(p.expire(4), Vec::<String>::new());
    assert_eq!(p.expire(5), vec!["b".to_owned()]);
    assert_eq!(p.receive(&ok, 5), Err(ProbeError::Unsolicited));
    assert_eq!(
        p.evidence("b", 6).map(|e| e.status),
        Some(LinkStatus::Unreachable)
    );
    assert_eq!(p.evidence("b", 25), None);
}

#[test]
fn probes_are_due_on_interval_plus_nonce_jitter() {
    let mut p = prober(0);
    assert_eq!(p.due(0), vec!["b".to_owned(), "c".into(), "d".into()]);
    let mut nonce = [0; 32];
    nonce[7] = 6; // 6 mod (jitter 3 + 1) = 2
    p.start("b", nonce, 0).unwrap();
    assert_eq!(p.due(0), vec!["c".to_owned(), "d".into()]);
    assert_eq!(p.start("b", nonce, 0), Err(ProbeError::InFlight));
    assert_eq!(p.start("z", nonce, 0), Err(ProbeError::UnknownPeer));
    p.expire(5);
    assert!(!p.due(11).contains(&"b".to_owned()));
    assert!(p.due(12).contains(&"b".to_owned()));
}

#[test]
fn observation_is_local_partial_and_expires_with_its_evidence() {
    let mut p = prober(0);
    assert_eq!(p.observation(0), None);
    let rb = p.start("b", [1; 32], 0).unwrap();
    p.start("c", [2; 32], 0).unwrap();
    p.receive(&respond(&key(2), "b", &rb, &pk(1), 1).unwrap(), 1)
        .unwrap();
    p.expire(5);
    let obs = p.observation(6).unwrap();
    assert_eq!(obs.observer, "a");
    assert_eq!(obs.scope, "s");
    assert_eq!(obs.observed_at, 6);
    assert_eq!(obs.expires_at, 21);
    assert_eq!(
        obs.links,
        BTreeMap::from([
            ("b".to_owned(), LinkStatus::Reachable),
            ("c".to_owned(), LinkStatus::Unreachable)
        ])
    );
    assert_eq!(RouteObservation::decode(&obs.encode()), Some(obs));
    // b's evidence has expired; c's remains until 25.
    let later = p.observation(21).unwrap();
    assert_eq!(
        later.links,
        BTreeMap::from([("c".to_owned(), LinkStatus::Unreachable)])
    );
    assert_eq!(p.observation(25), None);
}

#[test]
fn disable_drops_probe_state_and_keeps_peers() {
    let mut p = prober(0);
    let rb = p.start("b", [1; 32], 0).unwrap();
    let ok = respond(&key(2), "b", &rb, &pk(1), 1).unwrap();
    p.disable();
    assert_eq!(p.due(100), Vec::<String>::new());
    assert_eq!(p.start("c", [2; 32], 1), Err(ProbeError::Disabled));
    assert_eq!(p.receive(&ok, 1), Err(ProbeError::Unsolicited));
    assert_eq!(p.observation(1), None);
    p.enable(50);
    assert_eq!(p.due(50).len(), 3);
    let rc = p.start("c", [2; 32], 50).unwrap();
    p.receive(&respond(&key(3), "c", &rc, &pk(1), 51).unwrap(), 51)
        .unwrap();
}

// ---- reference state machine ----------------------------------------

#[derive(Debug, Clone, Default)]
struct RefPeer {
    due: u64,
    flight: Option<(Nonce, u64, u64)>,
    evidence: Option<(LinkStatus, u64, u64)>,
}

#[derive(Debug, Clone)]
struct RefProber {
    enabled: bool,
    peers: BTreeMap<String, RefPeer>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tamper {
    None,
    Prober,
    Scope,
    Nonce,
    Request,
    Time,
    Signature,
    Unknown,
}

#[derive(Debug, Clone)]
enum Op {
    Start(usize, [u8; 8]),
    Answer(usize, Tamper),
    Expire,
    Tick(u64),
    Disable,
    Enable,
}

fn arb_op() -> impl Strategy<Value = Op> {
    let tamper = prop_oneof![
        6 => Just(Tamper::None),
        1 => Just(Tamper::Prober),
        1 => Just(Tamper::Scope),
        1 => Just(Tamper::Nonce),
        1 => Just(Tamper::Request),
        1 => Just(Tamper::Time),
        1 => Just(Tamper::Signature),
        1 => Just(Tamper::Unknown),
    ];
    prop_oneof![
        4 => (0usize..3, any::<[u8; 8]>()).prop_map(|(p, n)| Op::Start(p, n)),
        5 => (0usize..3, tamper).prop_map(|(p, t)| Op::Answer(p, t)),
        2 => Just(Op::Expire),
        3 => (0u64..8).prop_map(Op::Tick),
        1 => Just(Op::Disable),
        1 => Just(Op::Enable),
    ]
}

fn nonce_from(head: [u8; 8], salt: u64) -> Nonce {
    let mut n = [0; 32];
    n[..8].copy_from_slice(&head);
    n[8..16].copy_from_slice(&salt.to_be_bytes());
    n
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn prober_matches_the_reference(ops in prop::collection::vec(arb_op(), 1..60)) {
        let mut p = prober(0);
        let mut r = RefProber {
            enabled: true,
            peers: PEERS.iter().map(|(n, _)| ((*n).to_owned(), RefPeer::default())).collect(),
        };
        let mut requests: BTreeMap<String, ProbeRequest> = BTreeMap::new();
        let mut now = 0u64;
        for (step, op) in ops.into_iter().enumerate() {
            match op {
                Op::Start(i, head) => {
                    let peer = PEERS[i].0;
                    let nonce = nonce_from(head, step as u64);
                    let rp = r.peers.get_mut(peer).unwrap();
                    let want = if !r.enabled {
                        Err(ProbeError::Disabled)
                    } else if rp.flight.is_some() {
                        Err(ProbeError::InFlight)
                    } else {
                        rp.flight = Some((nonce, now, now + CONFIG.timeout));
                        rp.due = now + CONFIG.interval + u64::from_be_bytes(head) % (CONFIG.jitter + 1);
                        Ok(())
                    };
                    let got = p.start(peer, nonce, now);
                    prop_assert_eq!(got.as_ref().map(|_| ()).map_err(|e| *e), want);
                    if let Ok(req) = got {
                        prop_assert_eq!(&req.target, peer);
                        prop_assert_eq!(req.nonce, nonce);
                        requests.insert(peer.to_owned(), req);
                    }
                }
                Op::Answer(i, tamper) => {
                    let peer = PEERS[i].0;
                    let Some(req) = requests.get(peer).cloned() else { continue };
                    // The responder answers at the probe's send time; the
                    // answer is delivered now.
                    let mut resp: ProbeResponse =
                        respond(&key(seed_of(peer)), peer, &req, &pk(1), req.sent_at).unwrap();
                    let resign = |resp: &mut ProbeResponse, seed: u8| {
                        resp.signature = key(seed).sign(&resp.preimage()).to_bytes();
                    };
                    match tamper {
                        Tamper::None => {}
                        Tamper::Prober => { resp.prober = "x".into(); resign(&mut resp, seed_of(peer)); }
                        Tamper::Scope => { resp.scope = "t".into(); resign(&mut resp, seed_of(peer)); }
                        Tamper::Nonce => { resp.nonce[31] ^= 1; resign(&mut resp, seed_of(peer)); }
                        Tamper::Request => { resp.request_digest[0] ^= 1; resign(&mut resp, seed_of(peer)); }
                        Tamper::Time => { resp.responded_at = now + 1; resign(&mut resp, seed_of(peer)); }
                        Tamper::Signature => resign(&mut resp, 9),
                        Tamper::Unknown => { resp.responder = "z".into(); resign(&mut resp, 9); }
                    }
                    let rp = r.peers.get_mut(peer).unwrap();
                    let live = rp.flight.filter(|(n, _, _)| *n == req.nonce);
                    let want = match (tamper, live) {
                        (Tamper::Unknown, _) | (_, None) => Err(ProbeError::Unsolicited),
                        (Tamper::Prober, _) => Err(ProbeError::WrongProber),
                        (Tamper::Scope, _) => Err(ProbeError::WrongScope),
                        (Tamper::Nonce, _) => Err(ProbeError::WrongNonce),
                        (Tamper::Request, _) => Err(ProbeError::WrongRequest),
                        (_, Some((_, _, exp))) if now >= exp => Err(ProbeError::Late),
                        (Tamper::Time, _) => Err(ProbeError::BadTime),
                        (Tamper::Signature, _) => Err(ProbeError::BadSignature),
                        (Tamper::None, Some(_)) => {
                            rp.flight = None;
                            rp.evidence = Some((LinkStatus::Reachable, now, now + CONFIG.ttl));
                            Ok(())
                        }
                    };
                    prop_assert_eq!(p.receive(&resp, now), want);
                }
                Op::Expire => {
                    let mut want = Vec::new();
                    for (name, rp) in &mut r.peers {
                        if let Some((_, _, exp)) = rp.flight
                            && now >= exp
                        {
                            rp.flight = None;
                            rp.evidence = Some((LinkStatus::Unreachable, exp, exp + CONFIG.ttl));
                            want.push(name.clone());
                        }
                    }
                    prop_assert_eq!(p.expire(now), want);
                }
                Op::Tick(dt) => now += dt,
                Op::Disable => {
                    r.enabled = false;
                    for rp in r.peers.values_mut() {
                        rp.flight = None;
                        rp.evidence = None;
                    }
                    p.disable();
                }
                Op::Enable => {
                    r.enabled = true;
                    for rp in r.peers.values_mut() {
                        rp.due = now;
                    }
                    p.enable(now);
                }
            }
            // Due peers, in-flight state, evidence and the observation.
            let due: Vec<String> = if r.enabled {
                r.peers
                    .iter()
                    .filter(|(_, rp)| rp.due <= now && rp.flight.is_none())
                    .map(|(n, _)| n.clone())
                    .collect()
            } else {
                Vec::new()
            };
            prop_assert_eq!(p.due(now), due);
            let mut links = BTreeMap::new();
            let mut exp_min = None::<u64>;
            for (name, rp) in &r.peers {
                prop_assert_eq!(p.in_flight(name), rp.flight.is_some());
                let ev = rp.evidence.filter(|(_, _, exp)| now < *exp);
                prop_assert_eq!(
                    p.evidence(name, now),
                    ev.map(|(status, observed_at, expires_at)| Evidence { status, observed_at, expires_at })
                );
                if let Some((status, _, exp)) = ev {
                    links.insert(name.clone(), status);
                    exp_min = Some(exp_min.map_or(exp, |m| m.min(exp)));
                }
            }
            let obs = p.observation(now);
            match exp_min {
                None => prop_assert_eq!(obs, None),
                Some(exp) => {
                    let obs = obs.unwrap();
                    prop_assert_eq!(&obs.observer, "a");
                    prop_assert_eq!(&obs.scope, "s");
                    prop_assert_eq!(obs.observed_at, now);
                    prop_assert_eq!(obs.expires_at, exp);
                    prop_assert!(!obs.links.contains_key("a"));
                    prop_assert_eq!(&obs.links, &links);
                    prop_assert_eq!(RouteObservation::decode(&obs.encode()), Some(obs.clone()));
                }
            }
        }
    }
}
