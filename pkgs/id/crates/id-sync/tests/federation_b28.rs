//! B28 invariant test: takeover and key-compromise response (#10232,
//! statements S1.83-S1.87).
//!
//! Oracle: **invariant** — the incident query identifies the affected
//! objects from key lineage and admission interval, and the cause may
//! remain indeterminate. The reference is an independent state machine
//! in this file: it tracks each trusted server's keys, every accepted
//! record, every incident and each record's status by the rules in the
//! slice contract, and predicts every admission, revalidation and blast
//! radius by brute force. Random streams admit records directly and
//! through a relay, rotate keys, declare incidents of each kind with a
//! suspected start in the past, and try revalidation with witnesses and
//! attestations. Signers' claimed times are random and never consulted.
//!
//! The slice modules are loaded by path: the federation roots record
//! them without declaring them (slice 9110-02), and B28 reaches B01 and
//! B02 through `super::`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/federation/slices/b01_key_lifecycle.rs"]
#[allow(dead_code)]
mod b01_key_lifecycle;

#[path = "../src/federation/slices/b02_key_revocation.rs"]
#[allow(dead_code)]
mod b02_key_revocation;

#[path = "../src/federation/slices/b28_takeover_response.rs"]
mod b28_takeover_response;

use std::collections::{BTreeMap, BTreeSet};

use b02_key_revocation::{
    HistoricalValidity, RecordStatus, RevalidationEvidence, RevalidationMethod, RevocationError,
};
use b28_takeover_response::{
    BlastRadius, Cause, Declaration, IncidentKind, KeyBytes, ObjectId, Refusal, TakeoverError,
    TakeoverResponder, Window,
};
use ed25519_dalek::{Signer, SigningKey};
use proptest::prelude::*;

const START: u64 = 1_000;
const SERVERS: usize = 3;
const RELAY: &str = "relay";
const OBJECTS: u8 = 5;
const AUDITOR: u8 = 200;

fn seed(server: usize, generation: usize) -> u8 {
    u8::try_from(1 + server * 16 + generation).unwrap()
}

fn signing(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn public(seed: u8) -> KeyBytes {
    signing(seed).verifying_key().to_bytes()
}

fn server(n: usize) -> String {
    format!("s{n}")
}

const fn object(n: u8) -> ObjectId {
    [0x28 ^ n; 32]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Accepted,
    Flagged,
    Quarantined,
    Revalidated,
}

struct RefRecord {
    object_id: ObjectId,
    key: KeyBytes,
    peer: String,
    admitted_at: u64,
    bytes: Vec<u8>,
    status: Status,
}

struct RefIncident {
    id: String,
    server: usize,
    cause: Cause,
    keys: BTreeSet<KeyBytes>,
    hostile: bool,
    from: u64,
}

impl RefIncident {
    fn touches(&self, key: &KeyBytes, peer: &str) -> bool {
        self.keys.contains(key) || (self.hostile && peer == server(self.server))
    }
}

#[derive(Default)]
struct Reference {
    /// Each server's key seeds, oldest first.
    lineages: Vec<Vec<u8>>,
    records: BTreeMap<String, RefRecord>,
    incidents: Vec<RefIncident>,
}

impl Reference {
    fn owner(&self, key: &KeyBytes) -> Option<usize> {
        self.lineages
            .iter()
            .position(|seeds| seeds.iter().any(|s| public(*s) == *key))
    }

    fn admission(&self, key: &KeyBytes, peer: &str) -> Result<(), Refusal> {
        if self.incidents.iter().any(|i| i.keys.contains(key)) {
            return Err(Refusal::RevokedKey);
        }
        if self
            .incidents
            .iter()
            .any(|i| i.hostile && peer == server(i.server))
        {
            return Err(Refusal::DisabledPeer);
        }
        if let Some(owner) = self.owner(key)
            && self
                .incidents
                .iter()
                .any(|i| i.hostile && i.server == owner)
        {
            return Err(Refusal::HostileLineage);
        }
        Ok(())
    }

    fn key_revoked(&self, key: &KeyBytes) -> bool {
        self.incidents.iter().any(|i| i.keys.contains(key))
    }
}

#[derive(Debug, Clone, Copy)]
enum Step {
    Admit {
        server: usize,
        generation: usize,
        via_relay: bool,
        object: u8,
        claimed: u16,
    },
    Rotate {
        server: usize,
    },
    Declare {
        server: usize,
        kind: u8,
        generation: usize,
        back: u64,
    },
    Revalidate {
        pick: usize,
        mode: u8,
    },
}

fn step_strategy() -> impl Strategy<Value = Step> {
    prop_oneof![
        6 => (0..SERVERS, 0..4_usize, any::<bool>(), 0..OBJECTS, any::<u16>()).prop_map(
            |(server, generation, via_relay, object, claimed)| Step::Admit {
                server,
                generation,
                via_relay,
                object,
                claimed,
            }
        ),
        1 => (0..SERVERS).prop_map(|server| Step::Rotate { server }),
        1 => (0..SERVERS, 0_u8..3, 0..4_usize, 0_u64..30).prop_map(
            |(server, kind, generation, back)| Step::Declare {
                server,
                kind,
                generation,
                back,
            }
        ),
        2 => (any::<usize>(), 0_u8..4).prop_map(|(pick, mode)| Step::Revalidate { pick, mode }),
    ]
}

const fn status_of(status: &RecordStatus) -> Status {
    match status {
        RecordStatus::Accepted => Status::Accepted,
        RecordStatus::Flagged { .. } => Status::Flagged,
        RecordStatus::Quarantined { .. } => Status::Quarantined,
        RecordStatus::Revalidated { .. } => Status::Revalidated,
    }
}

fn check(responder: &TakeoverResponder, reference: &Reference, now: u64) {
    // Every record's status follows the reference.
    for (id, record) in &reference.records {
        let got = responder.record(id).unwrap();
        assert_eq!(status_of(&got.status), record.status, "{id}");
        assert_eq!(got.signed_bytes, record.bytes);
        let expected = match record.status {
            Status::Accepted | Status::Revalidated => HistoricalValidity::Valid,
            Status::Flagged | Status::Quarantined => HistoricalValidity::Indeterminate,
        };
        assert_eq!(responder.historical_validity(id), Some(expected));
    }
    // The incident query names exactly the touched records and objects.
    for incident in &reference.incidents {
        let radius: BlastRadius = responder.blast_radius(&incident.id).unwrap();
        assert_eq!(radius.cause, incident.cause);
        let touched: BTreeMap<&String, &RefRecord> = reference
            .records
            .iter()
            .filter(|(_, r)| incident.touches(&r.key, &r.peer))
            .collect();
        assert_eq!(
            radius.records.keys().collect::<BTreeSet<_>>(),
            touched.keys().copied().collect::<BTreeSet<_>>()
        );
        for (id, record) in &touched {
            let exposure = &radius.records[*id];
            assert_eq!(exposure.object_id, record.object_id);
            assert_eq!(exposure.admitted_at, record.admitted_at);
            let window = if record.admitted_at >= incident.from {
                Window::Suspect
            } else {
                Window::Prior
            };
            assert_eq!(exposure.window, window);
            assert_eq!(status_of(&exposure.status), record.status);
            // Nothing suspect is released unless explicitly cleared.
            if window == Window::Suspect {
                assert!(matches!(
                    record.status,
                    Status::Quarantined | Status::Revalidated
                ));
            }
        }
        let objects: BTreeSet<ObjectId> = touched.values().map(|r| r.object_id).collect();
        assert_eq!(radius.objects, objects);
    }
    // Revocation is immediate: every incident's keys, and a hostile
    // server's whole lineage and peer, are refused now.
    for incident in &reference.incidents {
        for key in &incident.keys {
            assert_eq!(
                responder.decision(key, RELAY, now),
                Err(Refusal::RevokedKey)
            );
        }
        if incident.hostile {
            for s in &reference.lineages[incident.server] {
                assert!(responder.decision(&public(*s), RELAY, now).is_err());
            }
            assert!(
                responder
                    .decision(&public(AUDITOR), &server(incident.server), now)
                    .is_err()
            );
        }
    }
}

fn run(steps: &[Step]) {
    let mut responder = TakeoverResponder::new();
    let mut reference = Reference::default();
    for n in 0..SERVERS {
        responder
            .enroll_server(&server(n), public(seed(n, 0)), START)
            .unwrap();
        reference.lineages.push(vec![seed(n, 0)]);
    }
    let mut now = START;
    let mut counter = 0_u32;

    for step in steps {
        now += 1;
        match *step {
            Step::Admit {
                server: s,
                generation,
                via_relay,
                object: o,
                claimed,
            } => {
                let seeds = &reference.lineages[s];
                let key_seed = seeds[generation % seeds.len()];
                let key = public(key_seed);
                let peer = if via_relay {
                    RELAY.to_owned()
                } else {
                    server(s)
                };
                counter += 1;
                let id = format!("r{counter}");
                let bytes = format!("record {counter}").into_bytes();
                let expected = reference.admission(&key, &peer);
                // The signer's claimed time is random, and backdated half
                // the time; nothing may depend on it.
                let claimed_at = Some(u64::from(claimed));
                let got =
                    responder.admit(&id, object(o), key, &peer, now, claimed_at, bytes.clone());
                match expected {
                    Ok(()) => {
                        got.unwrap();
                        reference.records.insert(
                            id,
                            RefRecord {
                                object_id: object(o),
                                key,
                                peer,
                                admitted_at: now,
                                bytes,
                                status: Status::Accepted,
                            },
                        );
                    }
                    Err(refusal) => assert_eq!(got, Err(TakeoverError::Refused(refusal))),
                }
            }
            Step::Rotate { server: s } => {
                let seeds = &mut reference.lineages[s];
                if seeds.len() < 4 {
                    let next = seed(s, seeds.len());
                    responder.rotate(&server(s), public(next), now, 5).unwrap();
                    seeds.push(next);
                }
            }
            Step::Declare {
                server: s,
                kind,
                generation,
                back,
            } => {
                counter += 1;
                let id = format!("i{counter}");
                let from = now - back;
                let seeds = &reference.lineages[s];
                let leaked = public(seeds[generation % seeds.len()]);
                let (kind, cause, keys, hostile) = match kind {
                    0 => (
                        IncidentKind::KeyCompromise {
                            keys: BTreeSet::from([leaked]),
                        },
                        Cause::KeyCompromise,
                        BTreeSet::from([leaked]),
                        false,
                    ),
                    1 => (
                        IncidentKind::ServerTakeover,
                        Cause::ServerTakeover,
                        seeds.iter().map(|s| public(*s)).collect(),
                        true,
                    ),
                    _ => (
                        IncidentKind::Undetermined,
                        Cause::Indeterminate,
                        seeds.iter().map(|s| public(*s)).collect(),
                        true,
                    ),
                };
                let incident = RefIncident {
                    id: id.clone(),
                    server: s,
                    cause,
                    keys,
                    hostile,
                    from,
                };
                for record in reference.records.values_mut() {
                    // Accepted records are flagged or quarantined by
                    // window; a flagged one now suspect is quarantined.
                    let escalate = record.status == Status::Flagged && record.admitted_at >= from;
                    if (record.status == Status::Accepted || escalate)
                        && incident.touches(&record.key, &record.peer)
                    {
                        record.status = if record.admitted_at >= from {
                            Status::Quarantined
                        } else {
                            Status::Flagged
                        };
                    }
                }
                reference.incidents.push(incident);
                let radius = responder
                    .declare(
                        Declaration {
                            incident_id: id.clone(),
                            server: server(s),
                            kind,
                            suspected_from: from,
                        },
                        now,
                    )
                    .unwrap();
                assert_eq!(radius, responder.blast_radius(&id).unwrap());
            }
            Step::Revalidate { pick, mode } => {
                if reference.records.is_empty() {
                    continue;
                }
                let id = reference
                    .records
                    .keys()
                    .nth(pick % reference.records.len())
                    .unwrap()
                    .clone();
                let record = &reference.records[&id];
                let owner = reference.owner(&record.key).unwrap();
                let attesting_seed = match mode {
                    2 => Some(*reference.lineages[owner].last().unwrap()),
                    3 => Some(AUDITOR),
                    _ => None,
                };
                let evidence = match (mode, attesting_seed) {
                    (0, _) => RevalidationEvidence::WitnessedGoodThrough {
                        through: record.admitted_at,
                    },
                    (1, _) => RevalidationEvidence::WitnessedGoodThrough {
                        through: record.admitted_at - 1,
                    },
                    (_, Some(s)) => RevalidationEvidence::Reattested {
                        key: public(s),
                        signature: signing(s).sign(&record.bytes).to_bytes(),
                    },
                    _ => unreachable!(),
                };
                let hostile_attester = attesting_seed.is_some_and(|s| {
                    reference.incidents.iter().any(|i| {
                        i.hostile
                            && i.touches(&record.key, &record.peer)
                            && reference.lineages[i.server].contains(&s)
                    })
                });
                let eligible = matches!(record.status, Status::Flagged | Status::Quarantined);
                let expected = if hostile_attester {
                    Err(TakeoverError::AttestingKeyInHostileLineage)
                } else if !eligible {
                    Err(TakeoverError::Revocation(
                        RevocationError::NotEligibleForRevalidation(id.clone()),
                    ))
                } else if mode == 1 {
                    Err(TakeoverError::Revocation(
                        RevocationError::InsufficientWitness {
                            through: record.admitted_at - 1,
                            admitted_at: record.admitted_at,
                        },
                    ))
                } else if attesting_seed.is_some_and(|s| reference.key_revoked(&public(s))) {
                    Err(TakeoverError::Revocation(
                        RevocationError::AttestingKeyRevoked,
                    ))
                } else if mode == 0 {
                    Ok(RevalidationMethod::IndependentWitness)
                } else {
                    Ok(RevalidationMethod::Reattestation)
                };
                let got = responder.revalidate(&id, &evidence, now);
                assert_eq!(got, expected);
                if got.is_ok() {
                    reference.records.get_mut(&id).unwrap().status = Status::Revalidated;
                }
            }
        }
        check(&responder, &reference, now);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn the_incident_query_follows_key_lineage_and_admission_time(
        steps in prop::collection::vec(step_strategy(), 1..80),
    ) {
        run(&steps);
    }
}

fn responder_with_records() -> TakeoverResponder {
    let mut responder = TakeoverResponder::new();
    responder
        .enroll_server("s0", public(seed(0, 0)), START)
        .unwrap();
    responder
        .enroll_server("s1", public(seed(1, 0)), START)
        .unwrap();
    // Before the suspected start, and claiming to be later still.
    responder
        .admit(
            "early",
            object(0),
            public(seed(0, 0)),
            "s0",
            START + 10,
            Some(START + 500),
            b"early".to_vec(),
        )
        .unwrap();
    // After it, through a relay, and backdated by its signer.
    responder
        .admit(
            "late",
            object(1),
            public(seed(0, 0)),
            RELAY,
            START + 30,
            Some(START),
            b"late".to_vec(),
        )
        .unwrap();
    responder
        .admit(
            "other",
            object(2),
            public(seed(1, 0)),
            "s1",
            START + 30,
            None,
            b"other".to_vec(),
        )
        .unwrap();
    responder
}

#[test]
fn a_takeover_disables_the_server_and_quarantines_by_admission_time_not_claims() {
    let mut responder = responder_with_records();
    let radius = responder
        .declare(
            Declaration {
                incident_id: "takeover".to_owned(),
                server: "s0".to_owned(),
                kind: IncidentKind::ServerTakeover,
                suspected_from: START + 20,
            },
            START + 40,
        )
        .unwrap();
    assert_eq!(radius.cause, Cause::ServerTakeover);
    assert_eq!(
        radius.in_window(Window::Prior),
        BTreeSet::from(["early".to_owned()])
    );
    assert_eq!(
        radius.in_window(Window::Suspect),
        BTreeSet::from(["late".to_owned()])
    );
    assert_eq!(radius.objects, BTreeSet::from([object(0), object(1)]));
    assert!(matches!(
        responder.record("late").unwrap().status,
        RecordStatus::Quarantined { .. }
    ));
    assert!(matches!(
        responder.record("early").unwrap().status,
        RecordStatus::Flagged { .. }
    ));
    assert_eq!(
        responder.historical_validity("early"),
        Some(HistoricalValidity::Indeterminate)
    );
    // The other server is untouched.
    assert_eq!(
        responder.record("other").unwrap().status,
        RecordStatus::Accepted
    );

    // A key the server enrolls afterwards is refused, and cannot attest.
    responder
        .rotate("s0", public(seed(0, 1)), START + 41, 0)
        .unwrap();
    assert_eq!(
        responder.decision(&public(seed(0, 1)), RELAY, START + 42),
        Err(Refusal::HostileLineage)
    );
    let attest = RevalidationEvidence::Reattested {
        key: public(seed(0, 1)),
        signature: signing(seed(0, 1)).sign(b"late").to_bytes(),
    };
    assert_eq!(
        responder.revalidate("late", &attest, START + 42),
        Err(TakeoverError::AttestingKeyInHostileLineage)
    );
    // An independent auditor can.
    let audit = RevalidationEvidence::Reattested {
        key: public(AUDITOR),
        signature: signing(AUDITOR).sign(b"late").to_bytes(),
    };
    assert_eq!(
        responder.revalidate("late", &audit, START + 42),
        Ok(RevalidationMethod::Reattestation)
    );
}

#[test]
fn a_key_compromise_keeps_the_server_and_lets_its_new_key_attest() {
    let mut responder = responder_with_records();
    responder
        .rotate("s0", public(seed(0, 1)), START + 35, 0)
        .unwrap();
    let radius = responder
        .declare(
            Declaration {
                incident_id: "leak".to_owned(),
                server: "s0".to_owned(),
                kind: IncidentKind::KeyCompromise {
                    keys: BTreeSet::from([public(seed(0, 0))]),
                },
                suspected_from: START + 20,
            },
            START + 40,
        )
        .unwrap();
    assert_eq!(radius.cause, Cause::KeyCompromise);
    assert_eq!(radius.records.len(), 2);
    // The server still delivers data under its new key.
    assert_eq!(
        responder.decision(&public(seed(0, 1)), "s0", START + 41),
        Ok(())
    );
    assert_eq!(
        responder.decision(&public(seed(0, 0)), "s0", START + 41),
        Err(Refusal::RevokedKey)
    );
    let attest = RevalidationEvidence::Reattested {
        key: public(seed(0, 1)),
        signature: signing(seed(0, 1)).sign(b"late").to_bytes(),
    };
    assert_eq!(
        responder.revalidate("late", &attest, START + 41),
        Ok(RevalidationMethod::Reattestation)
    );
    // The leaked key itself cannot vouch for anything.
    let leaked = RevalidationEvidence::Reattested {
        key: public(seed(0, 0)),
        signature: signing(seed(0, 0)).sign(b"early").to_bytes(),
    };
    assert_eq!(
        responder.revalidate("early", &leaked, START + 41),
        Err(TakeoverError::Revocation(
            RevocationError::AttestingKeyRevoked
        ))
    );
}

#[test]
fn an_undetermined_incident_gets_the_takeover_response_with_an_indeterminate_cause() {
    let mut responder = responder_with_records();
    let radius = responder
        .declare(
            Declaration {
                incident_id: "unknown".to_owned(),
                server: "s0".to_owned(),
                kind: IncidentKind::Undetermined,
                suspected_from: START + 20,
            },
            START + 40,
        )
        .unwrap();
    assert_eq!(radius.cause, Cause::Indeterminate);
    assert_eq!(
        responder.decision(&public(AUDITOR), "s0", START + 41),
        Err(Refusal::DisabledPeer)
    );
    assert_eq!(responder.incidents()[0].keys.len(), 1);
    assert!(responder.incidents()[0].peer_disabled);
}

#[test]
fn declarations_are_checked() {
    let mut responder = responder_with_records();
    let declare = |id: &str, kind: IncidentKind, from: u64| Declaration {
        incident_id: id.to_owned(),
        server: "s0".to_owned(),
        kind,
        suspected_from: from,
    };
    assert_eq!(
        responder.declare(
            declare("a", IncidentKind::ServerTakeover, START + 50),
            START + 40
        ),
        Err(TakeoverError::SuspicionAfterDetection)
    );
    assert_eq!(
        responder.declare(
            declare(
                "b",
                IncidentKind::KeyCompromise {
                    keys: BTreeSet::new()
                },
                START
            ),
            START + 40
        ),
        Err(TakeoverError::NoKeys)
    );
    assert_eq!(
        responder.declare(
            declare(
                "c",
                IncidentKind::KeyCompromise {
                    keys: BTreeSet::from([public(seed(1, 0))])
                },
                START
            ),
            START + 40
        ),
        Err(TakeoverError::KeyNotInLineage(public(seed(1, 0))))
    );
    let mut unknown = declare("d", IncidentKind::ServerTakeover, START);
    unknown.server = "nobody".to_owned();
    assert_eq!(
        responder.declare(unknown, START + 40),
        Err(TakeoverError::UnknownServer("nobody".to_owned()))
    );
    responder
        .declare(
            declare("e", IncidentKind::ServerTakeover, START),
            START + 40,
        )
        .unwrap();
    assert_eq!(
        responder.declare(declare("e", IncidentKind::Undetermined, START), START + 41),
        Err(TakeoverError::DuplicateIncident("e".to_owned()))
    );
    assert_eq!(responder.incidents().len(), 1);
    assert_eq!(responder.incidents()[0].id, "e");
    // The registry holds a revocation per lineage key and one for the peer.
    let keys = responder.lineage("s0").unwrap().generations().len();
    assert_eq!(responder.registry().revocations().len(), keys + 1);
}
