//! Tests for slice 9111-04 (#10195): capability negotiation and
//! downgrade rejection.
//!
//! Oracle (from the slice manifest): **model** — "Negotiation selects
//! the highest allowed common capability set and rejects
//! security-weakening downgrade offers." Verifier: an adversarial
//! protocol-negotiation state machine and signed transcript vectors.
//!
//! How the independence works here: the module under test is included
//! by path (its crate-root export belongs to slice 9110-02), and the
//! reference model below is a second, deliberately plain
//! implementation of the design rules in
//! `crosscut/versioned-design.md` ("Failure, recovery, and version
//! rules"), written over raw strings and sets. The generated test
//! drives both state machines through the same adversarial offer
//! sequences — downgrade attempts, replays, identity swaps, malformed
//! versions, mid-session weakening — and compares the outcome class
//! and the settled state after every single offer. The fixed tests
//! pin the headline behaviors and the golden transcript bytes a
//! signature must cover.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../crates/id-config/src/negotiation.rs"]
mod negotiation;

use std::collections::{BTreeMap, BTreeSet};

use id_config::{ConfigDocument, FIXED_GATE_IDS};
use negotiation::{
    CapabilityClass, Negotiation, NegotiationError, NegotiationPolicy, PeerOffer, ProtocolVersion,
};
use proptest::prelude::*;

const LOCAL: &str = "node-a";
const PEER: &str = "node-b";
const OPTIONAL_CAPS: [&str; 2] = ["bulk-transfer", "delta-sync"];

fn version(raw: &str) -> ProtocolVersion {
    ProtocolVersion::parse(raw).unwrap()
}

fn gates() -> BTreeSet<String> {
    FIXED_GATE_IDS
        .iter()
        .map(|gate| (*gate).to_owned())
        .collect()
}

fn all_caps() -> BTreeSet<String> {
    let mut caps = gates();
    caps.extend(OPTIONAL_CAPS.iter().map(|cap| (*cap).to_owned()));
    caps
}

/// The standard local policy: four allowlisted versions, floor at
/// `id/2.0`, the five fixed gates required, two optional extras.
fn policy() -> NegotiationPolicy {
    let mut capabilities: BTreeMap<String, CapabilityClass> = BTreeMap::new();
    for gate in FIXED_GATE_IDS {
        capabilities.insert(gate.to_owned(), CapabilityClass::Required);
    }
    for cap in OPTIONAL_CAPS {
        capabilities.insert(cap.to_owned(), CapabilityClass::Optional);
    }
    NegotiationPolicy::new(
        LOCAL,
        ["id/1.0", "id/2.0", "id/2.1", "id/3.0"]
            .iter()
            .map(|raw| version(raw))
            .collect(),
        version("id/2.0"),
        capabilities,
    )
    .unwrap()
}

fn offer(round: u64, versions: &[&str], capabilities: BTreeSet<String>) -> PeerOffer {
    PeerOffer {
        peer_identity: PEER.to_owned(),
        versions: versions.iter().map(|raw| (*raw).to_owned()).collect(),
        capabilities,
        round,
    }
}

fn session() -> Negotiation {
    Negotiation::new(policy(), PEER).unwrap()
}

// ---------------------------------------------------------------------
// Fixed behavior tests
// ---------------------------------------------------------------------

#[test]
fn selects_the_highest_common_allowlisted_version() {
    let mut negotiation = session();
    // id/3.0 is allowlisted and offered; id/9.9 is offered but not
    // allowlisted and must not win.
    let established = negotiation
        .offer(offer(0, &["id/2.0", "id/3.0", "id/9.9"], all_caps()))
        .unwrap();
    assert_eq!(established.version, version("id/3.0"));
    assert_eq!(established.capabilities, all_caps());
}

#[test]
fn version_ordering_is_numeric_not_lexicographic() {
    let mut capabilities: BTreeMap<String, CapabilityClass> = BTreeMap::new();
    for gate in FIXED_GATE_IDS {
        capabilities.insert(gate.to_owned(), CapabilityClass::Required);
    }
    let numeric_policy = NegotiationPolicy::new(
        LOCAL,
        ["id/2.9", "id/2.10"]
            .iter()
            .map(|raw| version(raw))
            .collect(),
        version("id/2.9"),
        capabilities,
    )
    .unwrap();
    let mut negotiation = Negotiation::new(numeric_policy, PEER).unwrap();
    let established = negotiation
        .offer(offer(0, &["id/2.9", "id/2.10"], gates()))
        .unwrap();
    assert_eq!(established.version.canonical(), "id/2.10");
}

#[test]
fn protocol_version_accessors_and_canonical_form() {
    let parsed = version("id/2.10");
    assert_eq!(parsed.family(), "id");
    assert_eq!(parsed.major(), 2);
    assert_eq!(parsed.minor(), 10);
    // The canonical form always carries both components, so "id/2"
    // and "id/2.0" are the same version, not two.
    assert_eq!(version("id/2").canonical(), "id/2.0");
    assert_eq!(version("id/2"), version("id/2.0"));
}

#[test]
fn downgrade_below_the_floor_is_rejected_and_closes_the_session() {
    let mut negotiation = session();
    // Best common version is id/1.0, below the id/2.0 floor.
    let error = negotiation
        .offer(offer(0, &["id/1.0"], all_caps()))
        .unwrap_err();
    assert_eq!(
        error,
        NegotiationError::VersionBelowFloor {
            best: "id/1.0".to_owned(),
            floor: "id/2.0".to_owned(),
        }
    );
    assert!(negotiation.is_closed());
    // Never fall back silently: a later, perfectly good offer is not
    // resumed in this session.
    let error = negotiation
        .offer(offer(1, &["id/2.1"], all_caps()))
        .unwrap_err();
    assert_eq!(error, NegotiationError::SessionClosed);
}

#[test]
fn an_offer_missing_a_required_capability_is_a_downgrade() {
    let mut negotiation = session();
    let mut caps = all_caps();
    caps.remove("authorization");
    let error = negotiation.offer(offer(0, &["id/2.1"], caps)).unwrap_err();
    assert_eq!(
        error,
        NegotiationError::RequiredCapabilityMissing("authorization".to_owned())
    );
    assert!(negotiation.is_closed());
}

#[test]
fn no_common_version_is_rejected() {
    let mut negotiation = session();
    let error = negotiation
        .offer(offer(0, &["atlas/1.0", "id/4.0"], all_caps()))
        .unwrap_err();
    assert_eq!(error, NegotiationError::NoCommonVersion);
}

#[test]
fn unsupported_peer_capabilities_are_not_selected() {
    let mut negotiation = session();
    let mut caps = all_caps();
    caps.insert("telepathy".to_owned());
    let established = negotiation.offer(offer(0, &["id/2.1"], caps)).unwrap();
    assert_eq!(established.capabilities, all_caps());
}

#[test]
fn renegotiation_downgrade_is_rejected_and_the_session_survives() {
    let mut negotiation = session();
    let established = negotiation
        .offer(offer(0, &["id/2.1"], all_caps()))
        .unwrap();
    assert_eq!(established.version, version("id/2.1"));

    // A weaker renegotiation: lower version.
    let error = negotiation
        .offer(offer(1, &["id/2.0"], all_caps()))
        .unwrap_err();
    assert!(matches!(error, NegotiationError::SessionDowngrade(_)));
    // The established session is untouched (rollback rule: continue
    // only with the previously allowed version).
    assert_eq!(negotiation.established(), Some(&established));

    // A weaker renegotiation the other way: same version, an
    // established capability dropped.
    let mut fewer = all_caps();
    fewer.remove("delta-sync");
    let error = negotiation.offer(offer(2, &["id/2.1"], fewer)).unwrap_err();
    assert!(matches!(error, NegotiationError::SessionDowngrade(_)));
    assert_eq!(negotiation.established(), Some(&established));

    // An upgrade is accepted and becomes the new baseline. Note the
    // rounds: both rejected offers were evaluated, so they consumed
    // rounds 1 and 2.
    let upgraded = negotiation
        .offer(offer(3, &["id/3.0"], all_caps()))
        .unwrap();
    assert_eq!(upgraded.version, version("id/3.0"));
    assert_eq!(negotiation.established(), Some(&upgraded));
}

#[test]
fn renegotiation_may_add_an_optional_capability_at_the_same_version() {
    let mut negotiation = session();
    negotiation.offer(offer(0, &["id/2.1"], gates())).unwrap();
    // Additive optional capability: minor-compatible, not a downgrade.
    let established = negotiation
        .offer(offer(1, &["id/2.1"], all_caps()))
        .unwrap();
    assert_eq!(established.capabilities, all_caps());
}

#[test]
fn an_offer_from_another_peer_is_rejected_without_consuming_the_round() {
    let mut negotiation = session();
    let mut wrong_peer = offer(0, &["id/2.1"], all_caps());
    wrong_peer.peer_identity = "mallory".to_owned();
    let error = negotiation.offer(wrong_peer).unwrap_err();
    assert_eq!(
        error,
        NegotiationError::IdentityMismatch {
            expected: PEER.to_owned(),
            found: "mallory".to_owned(),
        }
    );
    assert_eq!(negotiation.next_round(), 0);
    negotiation
        .offer(offer(0, &["id/2.1"], all_caps()))
        .unwrap();
}

#[test]
fn a_replayed_offer_is_rejected_as_stale() {
    let mut negotiation = session();
    let first = offer(0, &["id/2.1"], all_caps());
    negotiation.offer(first.clone()).unwrap();
    // The identical offer again: its round was consumed.
    let error = negotiation.offer(first).unwrap_err();
    assert_eq!(
        error,
        NegotiationError::StaleRound {
            expected: 1,
            found: 0
        }
    );
}

#[test]
fn malformed_versions_are_rejected_without_consuming_the_round() {
    for bogus in ["bogus", "id/", "/2.1", "id/x.y", "id/2.1.3", "id/ 2.1", ""] {
        let mut negotiation = session();
        let error = negotiation
            .offer(offer(0, &["id/2.1", bogus], all_caps()))
            .unwrap_err();
        assert_eq!(
            error,
            NegotiationError::MalformedVersion(bogus.to_owned()),
            "input {bogus:?}"
        );
        assert_eq!(negotiation.next_round(), 0, "input {bogus:?}");
        negotiation
            .offer(offer(0, &["id/2.1"], all_caps()))
            .unwrap();
    }
}

#[test]
fn policy_validation_rejects_a_floor_outside_the_allowlist() {
    let mut capabilities: BTreeMap<String, CapabilityClass> = BTreeMap::new();
    capabilities.insert("authorization".to_owned(), CapabilityClass::Required);
    let error = NegotiationPolicy::new(
        LOCAL,
        ["id/2.0"].iter().map(|raw| version(raw)).collect(),
        version("id/2.1"),
        capabilities,
    )
    .unwrap_err();
    assert_eq!(
        error,
        NegotiationError::FloorNotAllowlisted {
            floor: "id/2.1".to_owned()
        }
    );
}

// ---------------------------------------------------------------------
// Transcript vectors
// ---------------------------------------------------------------------

fn golden_scenario() -> (Negotiation, PeerOffer) {
    let negotiation = session();
    let the_offer = offer(0, &["id/2.1", "id/2.0"], {
        let mut caps = gates();
        caps.insert("delta-sync".to_owned());
        caps
    });
    (negotiation, the_offer)
}

#[test]
fn transcript_golden_vector() {
    let (mut negotiation, the_offer) = golden_scenario();
    let established = negotiation.offer(the_offer).unwrap();
    let expected = "id-config/negotiation-transcript/v1\n\
        {\"local_identity\":\"node-a\",\"peer_identity\":\"node-b\",\"round\":0,\
         \"offered_versions\":[\"id/2.0\",\"id/2.1\"],\
         \"offered_capabilities\":[\"authorization\",\"delta-sync\",\"replay-protection\",\
         \"signature-verification\",\"tenant-scope\",\"validation\"],\
         \"selected_version\":\"id/2.1\",\
         \"selected_capabilities\":[\"authorization\",\"delta-sync\",\"replay-protection\",\
         \"signature-verification\",\"tenant-scope\",\"validation\"]}";
    assert_eq!(
        String::from_utf8(established.transcript.canonical_bytes()).unwrap(),
        expected
    );
}

#[test]
fn a_tampered_transcript_no_longer_binds() {
    let (mut negotiation, the_offer) = golden_scenario();
    let established = negotiation.offer(the_offer.clone()).unwrap();
    assert!(established.transcript.binds(LOCAL, &the_offer));

    // Rewrite history: claim the peer offered less than it did.
    let mut tampered = established.transcript.clone();
    tampered
        .offered_capabilities
        .retain(|cap| cap != "authorization");
    assert!(!tampered.binds(LOCAL, &the_offer));
    assert_ne!(
        tampered.canonical_bytes(),
        established.transcript.canonical_bytes()
    );

    // Or claim a different peer entirely.
    let mut swapped = established.transcript.clone();
    swapped.peer_identity = "mallory".to_owned();
    assert!(!swapped.binds(LOCAL, &the_offer));
}

// ---------------------------------------------------------------------
// Policy derivation from the 9111-01 config contract
// ---------------------------------------------------------------------

#[test]
fn policy_derives_from_a_resolved_config() {
    let document = ConfigDocument::parse(
        r#"{
            "schema_version": 1,
            "config_id": "cfg-home-node",
            "revision": 7,
            "mode": "federated-relay",
            "protocol_versions": ["id/2.1", "id/2.0"],
            "ceilings": {
                "max_hops": 5,
                "max_bytes": 1048576,
                "max_time_ms": 30000,
                "max_fan_out": 16
            }
        }"#,
    )
    .unwrap();
    let resolved = document.resolve().unwrap();
    let derived =
        NegotiationPolicy::from_resolved_config(&resolved, LOCAL, "id/2.0", &["delta-sync"])
            .unwrap();
    assert_eq!(derived.required_capabilities(), gates());
    assert_eq!(
        derived.allowlist,
        ["id/2.0", "id/2.1"]
            .iter()
            .map(|raw| version(raw))
            .collect()
    );
    assert_eq!(
        derived.capabilities.get("delta-sync"),
        Some(&CapabilityClass::Optional)
    );

    // A fixed gate can never be relabeled optional.
    let error =
        NegotiationPolicy::from_resolved_config(&resolved, LOCAL, "id/2.0", &["authorization"])
            .unwrap_err();
    assert_eq!(
        error,
        NegotiationError::CapabilityClassConflict("authorization".to_owned())
    );
}

// ---------------------------------------------------------------------
// The adversarial state-machine model
// ---------------------------------------------------------------------

/// The reference model: the design rules restated as plainly as
/// possible over raw strings. `version_key` is the model's own parse:
/// `(family, major, minor)`, or `None` for malformed input.
fn version_key(raw: &str) -> Option<(String, u32, u32)> {
    if raw.is_empty() || raw.chars().any(char::is_whitespace) {
        return None;
    }
    let (family, rest) = raw.split_once('/')?;
    if family.is_empty() || rest.contains('/') {
        return None;
    }
    let mut parts = rest.split('.');
    let major = parts.next()?;
    let minor = parts.next().unwrap_or("0");
    if parts.next().is_some() {
        return None;
    }
    let numeric = |text: &str| {
        if text.is_empty() || !text.chars().all(|c| c.is_ascii_digit()) {
            None
        } else {
            text.parse::<u32>().ok()
        }
    };
    Some((family.to_owned(), numeric(major)?, numeric(minor)?))
}

#[derive(Debug, Clone, PartialEq)]
enum ModelState {
    Awaiting,
    Established {
        version: (String, u32, u32),
        caps: BTreeSet<String>,
    },
    Rejected,
}

#[derive(Debug, Clone)]
struct Model {
    allowlist: BTreeSet<(String, u32, u32)>,
    floor: (String, u32, u32),
    required: BTreeSet<String>,
    supported: BTreeSet<String>,
    expected_peer: String,
    state: ModelState,
    next_round: u64,
}

impl Model {
    fn new() -> Self {
        Self {
            allowlist: ["id/1.0", "id/2.0", "id/2.1", "id/3.0"]
                .iter()
                .map(|raw| version_key(raw).unwrap())
                .collect(),
            floor: version_key("id/2.0").unwrap(),
            required: gates(),
            supported: all_caps(),
            expected_peer: PEER.to_owned(),
            state: ModelState::Awaiting,
            next_round: 0,
        }
    }

    /// One model step; returns the outcome class and, on acceptance,
    /// the settled version key and capability set.
    fn step(
        &mut self,
        offer: &PeerOffer,
    ) -> (&'static str, Option<((String, u32, u32), BTreeSet<String>)>) {
        if self.state == ModelState::Rejected {
            return ("closed", None);
        }
        if offer.peer_identity != self.expected_peer {
            return ("identity", None);
        }
        if offer.round != self.next_round {
            return ("stale-round", None);
        }
        let mut offered = BTreeSet::new();
        for raw in &offer.versions {
            match version_key(raw) {
                Some(key) => {
                    offered.insert(key);
                }
                None => return ("malformed", None),
            }
        }
        self.next_round += 1;

        let common: Vec<&(String, u32, u32)> = offered.intersection(&self.allowlist).collect();
        let Some(best) = common.into_iter().next_back() else {
            return self.reject("no-common");
        };
        if *best < self.floor {
            return self.reject("below-floor");
        }
        for required in &self.required {
            if !offer.capabilities.contains(required) {
                return self.reject("missing-required");
            }
        }
        let selected: BTreeSet<String> = self
            .supported
            .intersection(&offer.capabilities)
            .cloned()
            .collect();
        if let ModelState::Established { version, caps } = &self.state {
            if *best < *version || !caps.is_subset(&selected) {
                return ("session-downgrade", None);
            }
        }
        let settled = (best.clone(), selected);
        self.state = ModelState::Established {
            version: settled.0.clone(),
            caps: settled.1.clone(),
        };
        ("ok", Some(settled))
    }

    fn reject(
        &mut self,
        class: &'static str,
    ) -> (&'static str, Option<((String, u32, u32), BTreeSet<String>)>) {
        if self.state == ModelState::Awaiting {
            self.state = ModelState::Rejected;
        }
        (class, None)
    }
}

fn error_class(error: &NegotiationError) -> &'static str {
    match error {
        NegotiationError::SessionClosed => "closed",
        NegotiationError::IdentityMismatch { .. } => "identity",
        NegotiationError::StaleRound { .. } => "stale-round",
        NegotiationError::MalformedVersion(_) => "malformed",
        NegotiationError::NoCommonVersion => "no-common",
        NegotiationError::VersionBelowFloor { .. } => "below-floor",
        NegotiationError::RequiredCapabilityMissing(_) => "missing-required",
        NegotiationError::SessionDowngrade(_) => "session-downgrade",
        _ => "other",
    }
}

fn version_pool() -> Vec<String> {
    [
        "id/1.0",
        "id/2.0",
        "id/2.1",
        "id/3.0",
        "id/4.0",
        "atlas/1.0",
        "bogus",
        "id/2.x",
    ]
    .iter()
    .map(|raw| (*raw).to_owned())
    .collect()
}

fn cap_pool() -> Vec<String> {
    let mut caps = all_caps();
    caps.insert("telepathy".to_owned());
    caps.into_iter().collect()
}

prop_compose! {
    fn arb_offer(round: u64)(
        versions in proptest::collection::vec(proptest::sample::select(version_pool()), 0..=4),
        caps in proptest::collection::btree_set(proptest::sample::select(cap_pool()), 0..=8),
        wrong_peer in proptest::bool::weighted(0.15),
    ) -> (u64, PeerOffer) {
        (
            round,
            PeerOffer {
                peer_identity: if wrong_peer { "mallory".to_owned() } else { PEER.to_owned() },
                versions,
                capabilities: caps,
                round,
            },
        )
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn generated_negotiations_match_the_model(
        // Per step: a round choice (0 = the expected round, 1 = a
        // replay of round 0, 2 = a future round) and the offer to
        // evaluate at the chosen round.
        steps in proptest::collection::vec(
            (0u8..3).prop_flat_map(|choice| {
                // Placeholder round; the harness resolves the choice
                // against the live session before evaluating.
                let placeholder = if choice == 2 { u64::MAX } else { 0 };
                arb_offer(placeholder).prop_map(move |(_, offer)| (choice, offer))
            }),
            1..=10,
        )
    ) {
        let mut negotiation = session();
        let mut model = Model::new();
        for (choice, mut step_offer) in steps {
            // The harness resolves the round choice against the live
            // session: both machines must agree on the expected round
            // before the offer is even built.
            prop_assert_eq!(negotiation.next_round(), model.next_round);
            match choice {
                0 => {
                    step_offer.round = model.next_round;
                }
                1 => {
                    step_offer.round = 0;
                }
                _ => {
                    step_offer.round = model.next_round + 5;
                }
            }
            let (expected_class, expected_settled) = model.step(&step_offer);
            match negotiation.offer(step_offer) {
                Ok(established) => {
                    prop_assert_eq!(expected_class, "ok");
                    let (want_version, want_caps) = expected_settled.unwrap();
                    prop_assert_eq!(
                        established.version.canonical(),
                        format!("{}/{}.{}", want_version.0, want_version.1, want_version.2)
                    );
                    prop_assert_eq!(established.capabilities, want_caps);
                }
                Err(error) => {
                    prop_assert_eq!(error_class(&error), expected_class);
                }
            }
            prop_assert_eq!(negotiation.next_round(), model.next_round);
            prop_assert_eq!(negotiation.is_closed(), model.state == ModelState::Rejected);
        }
    }
}
