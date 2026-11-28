//! B02 key/peer revocation and quarantine — integration test (slice B02, #10204).
//!
//! Oracle (invariant, from the issue + design S1.22/S1.84/S1.86, wire/data
//! model): a locally signed emergency key/peer revocation or disable
//! record IMMEDIATELY blocks new acceptance and relay authority for that
//! key/peer; previously accepted affected data is quarantined/flagged by
//! policy — never silently rewritten (signed bytes and admission-time
//! evidence are preserved); historical validity is `indeterminate`
//! absent independently witnessed key-status/epoch evidence (issuer
//! wall-clock claims are stored but never trusted); unrelated keys/peers
//! and out-of-scope records are unaffected; key lineage stays intact and
//! rotation does not invalidate prior valid envelopes.
//!
//! Independence: the expected verdicts below come from the independent
//! reference state machine in this file (`reference_*`), written directly
//! from the design statements and sharing no code with the module under
//! test. Signatures are produced by ed25519-dalek directly and by a
//! hard-coded RFC 8032 golden vector — never by the module under test,
//! which only verifies. The federation roots deliberately do not declare
//! slice modules (slice 9110-02), so this test includes the slice file by
//! path; #9110-09 wires versioned public exports later.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/federation/slices/b02_key_revocation.rs"]
mod b02_key_revocation;

use std::collections::BTreeMap;

use b02_key_revocation::{
    AcceptedRecord, AffectCause, Decision, DenyReason, HistoricalValidity, RecordStatus,
    RevalidationEvidence, RevalidationMethod, RevocationError, RevocationReason, RevocationRecord,
    RevocationRegistry, RevocationSubject,
};
use ed25519_dalek::{Signer, SigningKey};

// ---------------------------------------------------------------------------
// Independent reference state machine (written from the design text).
// ---------------------------------------------------------------------------

/// Reference verdict for one previously accepted record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefVerdict {
    AcceptedValid,
    FlaggedIndeterminate,
    QuarantinedIndeterminate,
    RevalidatedValid,
}

impl RefVerdict {
    fn validity(self) -> HistoricalValidity {
        match self {
            Self::AcceptedValid | Self::RevalidatedValid => HistoricalValidity::Valid,
            Self::FlaggedIndeterminate | Self::QuarantinedIndeterminate => {
                HistoricalValidity::Indeterminate
            }
        }
    }

    fn releasable(self) -> bool {
        !matches!(self, Self::QuarantinedIndeterminate)
    }
}

/// Minimal independent model: revocation intervals per subject, admission
/// instants per record, and the sticky quarantine/flag transitions
/// (`AcceptedValid` records are flagged or quarantined; a later revocation
/// covering a flagged record's admission escalates it to quarantined;
/// quarantined and revalidated records never change on a revocation).
#[derive(Debug, Default)]
struct RefModel {
    /// key tag -> list of (from, until-or-None) intervals.
    key_intervals: BTreeMap<String, Vec<(u64, Option<u64>)>>,
    /// peer tag -> list of (from, until-or-None) intervals.
    peer_intervals: BTreeMap<String, Vec<(u64, Option<u64>)>>,
    /// record id -> (key tag, peer tag, admitted_at, verdict).
    records: BTreeMap<String, (String, String, u64, RefVerdict)>,
}

fn ref_key_tag(key: &[u8; 32]) -> String {
    let mut s = String::with_capacity(3 + 64);
    s.push_str("k:");
    for b in key {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn ref_peer_tag(peer: &str) -> String {
    format!("p:{peer}")
}

impl RefModel {
    fn sweep(&mut self, tag: &str, by_key: bool, from: u64, until: Option<u64>) {
        for (ktag, ptag, admitted_at, verdict) in self.records.values_mut() {
            let after_end = until.is_some_and(|u| *admitted_at >= u);
            let hit = !after_end && if by_key { ktag == tag } else { ptag == tag };
            let escalate = *verdict == RefVerdict::FlaggedIndeterminate && *admitted_at >= from;
            if hit && (*verdict == RefVerdict::AcceptedValid || escalate) {
                *verdict = if *admitted_at >= from {
                    RefVerdict::QuarantinedIndeterminate
                } else {
                    RefVerdict::FlaggedIndeterminate
                };
            }
        }
    }

    fn revoke_key(&mut self, key: &[u8; 32], from: u64, until: Option<u64>) {
        let tag = ref_key_tag(key);
        self.key_intervals
            .entry(tag.clone())
            .or_default()
            .push((from, until));
        self.sweep(&tag, true, from, until);
    }

    fn revoke_peer(&mut self, peer: &str, from: u64, until: Option<u64>) {
        let tag = ref_peer_tag(peer);
        self.peer_intervals
            .entry(tag.clone())
            .or_default()
            .push((from, until));
        self.sweep(&tag, false, from, until);
    }

    fn blocked(
        intervals: &BTreeMap<String, Vec<(u64, Option<u64>)>>,
        subject: &str,
        at: u64,
    ) -> bool {
        intervals.get(subject).is_some_and(|ivs| {
            ivs.iter()
                .any(|(from, until)| at >= *from && until.is_none_or(|u| at < u))
        })
    }

    fn admit(&mut self, id: &str, key: &[u8; 32], peer: &str, at: u64) -> Result<(), DenyReason> {
        if Self::blocked(&self.key_intervals, &ref_key_tag(key), at) {
            return Err(DenyReason::RevokedKey);
        }
        if Self::blocked(&self.peer_intervals, &ref_peer_tag(peer), at) {
            return Err(DenyReason::DisabledPeer);
        }
        self.records.insert(
            id.to_string(),
            (
                ref_key_tag(key),
                ref_peer_tag(peer),
                at,
                RefVerdict::AcceptedValid,
            ),
        );
        Ok(())
    }

    fn verdict_for(&self, id: &str) -> RefVerdict {
        self.records
            .get(id)
            .map(|(_, _, _, v)| *v)
            .unwrap_or(RefVerdict::AcceptedValid)
    }

    fn revalidate(&mut self, id: &str, witnessed_through: u64) -> bool {
        let admitted_at = match self.records.get(id) {
            Some((_, _, at, v))
                if matches!(
                    v,
                    RefVerdict::QuarantinedIndeterminate | RefVerdict::FlaggedIndeterminate
                ) =>
            {
                *at
            }
            _ => return false,
        };
        if witnessed_through < admitted_at {
            return false;
        }
        if let Some(entry) = self.records.get_mut(id) {
            entry.3 = RefVerdict::RevalidatedValid;
        }
        true
    }
}

// ---------------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------------

fn keypair(seed: [u8; 32]) -> (SigningKey, [u8; 32]) {
    let sk = SigningKey::from_bytes(&seed);
    let vk = sk.verifying_key().to_bytes();
    (sk, vk)
}

fn sign(sk: &SigningKey, msg: &[u8]) -> [u8; 64] {
    sk.sign(msg).to_bytes()
}

fn hex_bytes(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("test vector hex"))
        .collect()
}

/// RFC 8032 section 7.1, TEST 3: an independent golden vector
/// (verify-only: public key, message, signature).
const RFC_PUBLIC: &str = "fc51cd8e6218a1a38da47ed00230f0580816ed13ba3303ac5deb911548908025";
const RFC_MESSAGE: &str = "af82";
const RFC_SIGNATURE: &str = "6291d657deec24024827e69c3abe01a30ce548a284743a445e3680d7db5ac3ac18ff9b538d16f290ae67f760984dc6594a7c15e9716ed28dc027beceea1ec40a";

fn key_revocation(id: &str, key: [u8; 32], from: u64, at: u64) -> RevocationRecord {
    RevocationRecord {
        record_id: id.to_string(),
        subject: RevocationSubject::Key(key),
        effective_from: from,
        effective_until: None,
        reason: RevocationReason::KeyCompromise,
        recorded_at: at,
    }
}

fn peer_revocation(id: &str, peer: &str, from: u64, at: u64) -> RevocationRecord {
    RevocationRecord {
        record_id: id.to_string(),
        subject: RevocationSubject::Peer(peer.to_string()),
        effective_from: from,
        effective_until: None,
        reason: RevocationReason::PeerCompromise,
        recorded_at: at,
    }
}

fn admit(
    reg: &mut RevocationRegistry,
    id: &str,
    key: [u8; 32],
    peer: &str,
    at: u64,
    claimed: Option<u64>,
) {
    reg.admit(
        id.to_string(),
        key,
        peer.to_string(),
        at,
        7,
        claimed,
        format!("signed-bytes:{id}").into_bytes(),
    )
    .expect("admission in fixture must succeed");
}

fn status_of(reg: &RevocationRegistry, id: &str) -> RecordStatus {
    reg.find_record(id)
        .expect("record must exist")
        .status
        .clone()
}

fn check_against_reference(
    reg: &RevocationRegistry,
    model: &RefModel,
    ids: &[&str],
    key_of: &BTreeMap<String, [u8; 32]>,
    peer_of: &BTreeMap<String, String>,
    at: u64,
) {
    for id in ids {
        let expected = model.verdict_for(id);
        let rec: &AcceptedRecord = reg.find_record(id).expect("record must exist");
        assert_eq!(
            rec.status.historical_validity(),
            expected.validity(),
            "validity mismatch for {id}"
        );
        assert_eq!(
            rec.status.is_releasable(),
            expected.releasable(),
            "releasability mismatch for {id}"
        );
        let key = key_of[*id];
        let peer = peer_of[*id].as_str();
        let want = match model.admit_probe(&key, peer, at) {
            Ok(()) => Decision::Allow,
            Err(DenyReason::RevokedKey) => Decision::Deny(DenyReason::RevokedKey),
            Err(DenyReason::DisabledPeer) => Decision::Deny(DenyReason::DisabledPeer),
        };
        assert_eq!(reg.admission_decision(&key, peer, at), want);
        assert_eq!(reg.relay_decision(&key, peer, at), want);
    }
}

impl RefModel {
    /// Probe helper: pure admission check without storing.
    fn admit_probe(&self, key: &[u8; 32], peer: &str, at: u64) -> Result<(), DenyReason> {
        if Self::blocked(&self.key_intervals, &ref_key_tag(key), at) {
            return Err(DenyReason::RevokedKey);
        }
        if Self::blocked(&self.peer_intervals, &ref_peer_tag(peer), at) {
            return Err(DenyReason::DisabledPeer);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Targeted invariant tests.
// ---------------------------------------------------------------------------

#[test]
fn emergency_key_revocation_blocks_new_acceptance_and_relay_immediately() {
    let (_ska, key_a) = keypair([3u8; 32]);
    let (_skb, key_b) = keypair([9u8; 32]);
    let mut reg = RevocationRegistry::new();

    admit(&mut reg, "early-a", key_a, "peer-1", 100, None);
    assert_eq!(
        reg.admission_decision(&key_a, "peer-1", 150),
        Decision::Allow
    );

    // Emergency revocation lands at t=200, effective immediately.
    let affected = reg
        .apply_revocation(key_revocation("rev-1", key_a, 200, 200))
        .expect("revocation must apply");
    assert_eq!(affected, vec!["early-a".to_string()]);

    // New acceptance AND relay authority for the revoked key stop now.
    assert_eq!(
        reg.admission_decision(&key_a, "peer-1", 200),
        Decision::Deny(DenyReason::RevokedKey)
    );
    assert_eq!(
        reg.relay_decision(&key_a, "peer-1", 250),
        Decision::Deny(DenyReason::RevokedKey)
    );
    assert_eq!(
        reg.admit(
            "late-a".to_string(),
            key_a,
            "peer-1".to_string(),
            250,
            7,
            None,
            b"late".to_vec()
        ),
        Err(RevocationError::RevokedKey)
    );

    // Unrelated keys and peers are unaffected.
    assert_eq!(
        reg.admission_decision(&key_b, "peer-1", 250),
        Decision::Allow
    );
    assert_eq!(reg.relay_decision(&key_b, "peer-2", 250), Decision::Allow);
    admit(&mut reg, "other", key_b, "peer-2", 250, None);
    assert_eq!(status_of(&reg, "other"), RecordStatus::Accepted);
}

#[test]
fn peer_disable_blocks_acceptance_and_relay_with_peer_reason() {
    let (_ska, key_a) = keypair([3u8; 32]);
    let mut reg = RevocationRegistry::new();
    admit(&mut reg, "r1", key_a, "peer-9", 100, None);

    reg.apply_revocation(peer_revocation("rev-peer", "peer-9", 300, 300))
        .expect("peer revocation must apply");

    assert_eq!(
        reg.admission_decision(&key_a, "peer-9", 300),
        Decision::Deny(DenyReason::DisabledPeer)
    );
    assert_eq!(
        reg.relay_decision(&key_a, "peer-9", 400),
        Decision::Deny(DenyReason::DisabledPeer)
    );
    // Same key via a different peer still admits: disable is per-peer.
    assert_eq!(
        reg.admission_decision(&key_a, "peer-10", 400),
        Decision::Allow
    );
}

#[test]
fn sweep_quarantines_in_scope_and_flags_pre_revocation_without_rewriting() {
    let (_ska, key_a) = keypair([3u8; 32]);
    let (_skb, key_b) = keypair([9u8; 32]);
    let mut reg = RevocationRegistry::new();

    admit(&mut reg, "before", key_a, "peer-1", 100, Some(90));
    admit(&mut reg, "inside", key_a, "peer-1", 220, Some(95));
    admit(&mut reg, "unrelated", key_b, "peer-1", 220, None);

    let before_bytes = reg
        .find_record("before")
        .expect("record must exist")
        .signed_bytes
        .clone();
    let before_evidence = reg
        .find_record("before")
        .expect("record must exist")
        .admission_evidence
        .clone();

    reg.apply_revocation(key_revocation("rev-1", key_a, 200, 210))
        .expect("revocation must apply");

    // In-interval record: quarantined (withheld). Pre-interval record:
    // flagged (releasable but indeterminate). Unrelated key untouched.
    assert_eq!(
        status_of(&reg, "inside"),
        RecordStatus::Quarantined {
            quarantined_at: 210,
            cause: AffectCause::AdmittedUnderRevocation,
        }
    );
    assert_eq!(
        status_of(&reg, "before"),
        RecordStatus::Flagged {
            flagged_at: 210,
            cause: AffectCause::AdmittedBeforeRevocation,
        }
    );
    assert_eq!(status_of(&reg, "unrelated"), RecordStatus::Accepted);

    // Nothing was rewritten: bytes and admission evidence are identical.
    let after = reg.find_record("before").expect("record must exist");
    assert_eq!(after.signed_bytes, before_bytes);
    assert_eq!(after.admission_evidence, before_evidence);

    // The registry's own record view agrees: nothing was added or dropped.
    assert_eq!(reg.records().len(), 3);
    let ids: Vec<&str> = reg.records().iter().map(|r| r.record_id.as_str()).collect();
    assert_eq!(ids, vec!["before", "inside", "unrelated"]);

    // Releasability: quarantined withholds, flagged stays releasable.
    assert!(!reg.find_record("inside").expect("x").status.is_releasable());
    assert!(reg.find_record("before").expect("x").status.is_releasable());

    // Historical validity without a witness is indeterminate either way.
    assert_eq!(
        status_of(&reg, "inside").historical_validity(),
        HistoricalValidity::Indeterminate
    );
    assert_eq!(
        status_of(&reg, "before").historical_validity(),
        HistoricalValidity::Indeterminate
    );
    assert_eq!(
        status_of(&reg, "unrelated").historical_validity(),
        HistoricalValidity::Valid
    );
}

#[test]
fn issuer_wall_clock_claims_are_never_trusted() {
    let (_ska, key_a) = keypair([3u8; 32]);
    let mut reg = RevocationRegistry::new();

    // Backdated claim (claim 50 << effective 200) on a record admitted
    // after the effective start: still quarantined, not exonerated.
    admit(&mut reg, "backdated", key_a, "peer-1", 220, Some(50));
    // Claim inside the interval on a pre-interval admission: still only
    // flagged — the claim cannot escalate either.
    admit(&mut reg, "pre", key_a, "peer-1", 100, Some(230));

    reg.apply_revocation(key_revocation("rev-1", key_a, 200, 210))
        .expect("revocation must apply");

    assert!(matches!(
        status_of(&reg, "backdated"),
        RecordStatus::Quarantined { .. }
    ));
    assert!(matches!(
        status_of(&reg, "pre"),
        RecordStatus::Flagged { .. }
    ));
    // The claims are preserved for audit, byte-identical.
    assert_eq!(
        reg.find_record("backdated").expect("x").issuer_claimed_at,
        Some(50)
    );
    assert_eq!(
        reg.find_record("pre").expect("x").issuer_claimed_at,
        Some(230)
    );
}

#[test]
fn revalidation_clears_quarantine_only_on_explicit_evidence() {
    let (ska, key_a) = keypair([3u8; 32]);
    let (_skb, key_b) = keypair([9u8; 32]);
    let (skc, key_c) = keypair([42u8; 32]);
    let mut reg = RevocationRegistry::new();

    admit(&mut reg, "q", key_a, "peer-1", 220, None);
    admit(&mut reg, "f", key_a, "peer-1", 100, None);
    reg.apply_revocation(key_revocation("rev-1", key_a, 200, 210))
        .expect("revocation must apply");

    // A witness that does not cover admission is insufficient.
    assert_eq!(
        reg.revalidate(
            "q",
            &RevalidationEvidence::WitnessedGoodThrough { through: 219 },
            300
        ),
        Err(RevocationError::InsufficientWitness {
            through: 219,
            admitted_at: 220
        })
    );
    assert!(matches!(
        status_of(&reg, "q"),
        RecordStatus::Quarantined { .. }
    ));

    // Independent witness covering admission clears to Valid.
    assert_eq!(
        reg.revalidate(
            "q",
            &RevalidationEvidence::WitnessedGoodThrough { through: 220 },
            300
        ),
        Ok(RevalidationMethod::IndependentWitness)
    );
    assert_eq!(
        status_of(&reg, "q"),
        RecordStatus::Revalidated {
            revalidated_at: 300,
            method: RevalidationMethod::IndependentWitness,
        }
    );
    assert_eq!(
        status_of(&reg, "q").historical_validity(),
        HistoricalValidity::Valid
    );

    // Re-attestation under a currently good key re-anchors the bytes.
    let bytes = reg.find_record("f").expect("x").signed_bytes.clone();
    let sig = sign(&skc, &bytes);
    assert_eq!(
        reg.revalidate(
            "f",
            &RevalidationEvidence::Reattested {
                key: key_c,
                signature: sig
            },
            310
        ),
        Ok(RevalidationMethod::Reattestation)
    );
    // Bytes unchanged by re-attestation.
    assert_eq!(reg.find_record("f").expect("x").signed_bytes, bytes);

    // Bad attestation signature is rejected.
    admit(&mut reg, "q2", key_b, "peer-2", 230, None);
    reg.apply_revocation(key_revocation("rev-2", key_b, 200, 240))
        .expect("revocation must apply");
    let genuine = sign(&skc, b"q2-bytes");
    assert_eq!(
        reg.revalidate(
            "q2",
            &RevalidationEvidence::Reattested {
                key: key_c,
                signature: genuine
            },
            250
        ),
        Err(RevocationError::BadAttestationSignature)
    );

    // Attestation under a revoked key is rejected even if valid.
    let bytes2 = reg.find_record("q2").expect("x").signed_bytes.clone();
    let attacker_sig = sign(&ska, &bytes2);
    assert_eq!(
        reg.revalidate(
            "q2",
            &RevalidationEvidence::Reattested {
                key: key_a,
                signature: attacker_sig
            },
            250
        ),
        Err(RevocationError::AttestingKeyRevoked)
    );

    // Unknown records and already-clear records cannot revalidate.
    assert_eq!(
        reg.revalidate(
            "missing",
            &RevalidationEvidence::WitnessedGoodThrough { through: 999 },
            250
        ),
        Err(RevocationError::UnknownRecord("missing".to_string()))
    );
    assert_eq!(
        reg.revalidate(
            "q",
            &RevalidationEvidence::WitnessedGoodThrough { through: 999 },
            260
        ),
        Err(RevocationError::NotEligibleForRevalidation("q".to_string()))
    );
}

#[test]
fn rfc8032_golden_vector_reattestation() {
    let (_ska, key_a) = keypair([3u8; 32]);
    let public: [u8; 32] = hex_bytes(RFC_PUBLIC).try_into().expect("vector key");
    let message = hex_bytes(RFC_MESSAGE);
    let signature: [u8; 64] = hex_bytes(RFC_SIGNATURE).try_into().expect("vector sig");

    let mut reg = RevocationRegistry::new();
    // The golden-vector message is the stored signed bytes; admission is
    // under the test key so the record can be quarantined.
    reg.admit(
        "golden".to_string(),
        key_a,
        "peer-1".to_string(),
        220,
        7,
        None,
        message,
    )
    .expect("admission must succeed");
    reg.apply_revocation(key_revocation("rev-1", key_a, 200, 210))
        .expect("revocation must apply");
    assert!(matches!(
        status_of(&reg, "golden"),
        RecordStatus::Quarantined { .. }
    ));

    // Re-attestation verifies through the RFC 8032 TEST 3 vector alone —
    // the module never signed anything in this path.
    assert_eq!(
        reg.revalidate(
            "golden",
            &RevalidationEvidence::Reattested {
                key: public,
                signature
            },
            300
        ),
        Ok(RevalidationMethod::Reattestation)
    );
}

#[test]
fn bounded_interval_blocks_only_inside_and_quarantine_is_sticky() {
    let (_ska, key_a) = keypair([3u8; 32]);
    let mut reg = RevocationRegistry::new();
    admit(&mut reg, "old", key_a, "peer-1", 100, None);

    reg.apply_revocation(RevocationRecord {
        record_id: "suspension".to_string(),
        subject: RevocationSubject::Key(key_a),
        effective_from: 200,
        effective_until: Some(300),
        reason: RevocationReason::AdministrativeDisable,
        recorded_at: 200,
    })
    .expect("bounded revocation must apply");

    // Inside the interval: blocked. At/past the end: allowed again.
    assert_eq!(
        reg.admission_decision(&key_a, "peer-1", 250),
        Decision::Deny(DenyReason::RevokedKey)
    );
    assert_eq!(
        reg.admission_decision(&key_a, "peer-1", 300),
        Decision::Allow
    );
    admit(&mut reg, "after", key_a, "peer-1", 300, None);
    assert_eq!(status_of(&reg, "after"), RecordStatus::Accepted);

    // The sweep already applied is sticky: expiry does not un-quarantine.
    assert!(matches!(
        status_of(&reg, "old"),
        RecordStatus::Flagged { .. }
    ));
}

#[test]
fn late_bounded_revocation_sweeps_only_records_admitted_inside_it() {
    let (_ska, key_a) = keypair([3u8; 32]);
    let mut reg = RevocationRegistry::new();
    admit(&mut reg, "before", key_a, "peer-1", 100, None);
    admit(&mut reg, "inside", key_a, "peer-1", 250, None);
    admit(&mut reg, "at-end", key_a, "peer-1", 300, None);
    admit(&mut reg, "after", key_a, "peer-1", 400, None);

    // The suspension is learned only after every admission.
    let affected = reg
        .apply_revocation(RevocationRecord {
            record_id: "late-suspension".to_string(),
            subject: RevocationSubject::Key(key_a),
            effective_from: 200,
            effective_until: Some(300),
            reason: RevocationReason::AdministrativeDisable,
            recorded_at: 500,
        })
        .expect("bounded revocation must apply");
    assert_eq!(affected, vec!["before".to_string(), "inside".to_string()]);
    assert!(matches!(
        status_of(&reg, "before"),
        RecordStatus::Flagged { .. }
    ));
    assert!(matches!(
        status_of(&reg, "inside"),
        RecordStatus::Quarantined { .. }
    ));
    assert_eq!(status_of(&reg, "at-end"), RecordStatus::Accepted);
    assert_eq!(status_of(&reg, "after"), RecordStatus::Accepted);

    // A later unbounded revocation still reaches the post-interval
    // records, and the earlier quarantine stays.
    reg.apply_revocation(RevocationRecord {
        record_id: "compromise".to_string(),
        subject: RevocationSubject::Key(key_a),
        effective_from: 350,
        effective_until: None,
        reason: RevocationReason::KeyCompromise,
        recorded_at: 600,
    })
    .expect("unbounded revocation must apply");
    assert!(matches!(
        status_of(&reg, "inside"),
        RecordStatus::Quarantined { .. }
    ));
    assert!(matches!(
        status_of(&reg, "at-end"),
        RecordStatus::Flagged { .. }
    ));
    assert!(matches!(
        status_of(&reg, "after"),
        RecordStatus::Quarantined { .. }
    ));
}

#[test]
fn malformed_revocation_and_admission_inputs_are_rejected() {
    let (_ska, key_a) = keypair([3u8; 32]);
    let mut reg = RevocationRegistry::new();

    // Inverted and empty intervals are rejected; nothing is stored.
    assert_eq!(
        reg.apply_revocation(RevocationRecord {
            record_id: "bad".to_string(),
            subject: RevocationSubject::Key(key_a),
            effective_from: 300,
            effective_until: Some(200),
            reason: RevocationReason::KeyCompromise,
            recorded_at: 300,
        }),
        Err(RevocationError::InvertedInterval {
            from: 300,
            until: 200
        })
    );
    assert_eq!(
        reg.apply_revocation(RevocationRecord {
            record_id: "empty".to_string(),
            subject: RevocationSubject::Key(key_a),
            effective_from: 300,
            effective_until: Some(300),
            reason: RevocationReason::KeyCompromise,
            recorded_at: 300,
        }),
        Err(RevocationError::InvertedInterval {
            from: 300,
            until: 300
        })
    );
    assert!(reg.revocations().is_empty());

    // Duplicate revocation and record ids are rejected.
    reg.apply_revocation(key_revocation("rev-1", key_a, 200, 200))
        .expect("first apply must succeed");
    assert_eq!(
        reg.apply_revocation(key_revocation("rev-1", key_a, 250, 250)),
        Err(RevocationError::DuplicateRevocationId("rev-1".to_string()))
    );
    admit(&mut reg, "r", key_a, "peer-9", 100, None);
    assert_eq!(
        reg.admit(
            "r".to_string(),
            key_a,
            "peer-9".to_string(),
            100,
            7,
            None,
            b"x".to_vec()
        ),
        Err(RevocationError::DuplicateRecordId("r".to_string()))
    );

    // Key reason reports first when both key and peer are blocked.
    reg.apply_revocation(peer_revocation("rev-peer", "peer-1", 100, 100))
        .expect("peer revocation must apply");
    let (_skz, key_z) = keypair([77u8; 32]);
    reg.apply_revocation(key_revocation("rev-z", key_z, 100, 100))
        .expect("key revocation must apply");
    assert_eq!(
        reg.admission_decision(&key_z, "peer-1", 150),
        Decision::Deny(DenyReason::RevokedKey)
    );
}

// ---------------------------------------------------------------------------
// Differential test against the independent reference model.
// ---------------------------------------------------------------------------

fn lcg_next(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *state >> 33
}

#[test]
fn differential_against_reference_model() {
    let seeds: [[u8; 32]; 3] = [[11u8; 32], [22u8; 32], [33u8; 32]];
    let keys: Vec<[u8; 32]> = seeds.map(|s| keypair(s).1).to_vec();
    let peers = ["pa", "pb"];

    let mut rng_state: u64 = 0x243F_6A88_85A3_08D3;
    let mut next = move || lcg_next(&mut rng_state);

    for round in 0..25u64 {
        let mut reg = RevocationRegistry::new();
        let mut model = RefModel::default();
        let mut ids: Vec<String> = Vec::new();
        let mut key_of: BTreeMap<String, [u8; 32]> = BTreeMap::new();
        let mut peer_of: BTreeMap<String, String> = BTreeMap::new();

        // Interleave admissions and revocations on a small deterministic script.
        for step in 0..12u64 {
            let at = 50 + (next() % 400);
            let ki = (next() % 3) as usize;
            let pi = (next() % 2) as usize;
            let key = keys[ki];
            let peer = peers[pi];
            if next() % 3 == 0 {
                // Revoke: random subject, kind, and interval shape.
                let rev_id = format!("rev-{round}-{step}");
                let from = 50 + (next() % 400);
                if next() % 2 == 0 {
                    let rec = RevocationRecord {
                        record_id: rev_id.clone(),
                        subject: RevocationSubject::Key(key),
                        effective_from: from,
                        effective_until: None,
                        reason: RevocationReason::KeyCompromise,
                        recorded_at: from,
                    };
                    reg.apply_revocation(rec).expect("revocation must apply");
                    model.revoke_key(&key, from, None);
                } else {
                    let until = if next() % 2 == 0 {
                        Some(from + 1 + (next() % 200))
                    } else {
                        None
                    };
                    let rec = RevocationRecord {
                        record_id: rev_id.clone(),
                        subject: RevocationSubject::Peer(peer.to_string()),
                        effective_from: from,
                        effective_until: until,
                        reason: RevocationReason::PeerCompromise,
                        recorded_at: from,
                    };
                    reg.apply_revocation(rec).expect("revocation must apply");
                    model.revoke_peer(peer, from, until);
                }
            } else {
                // Admit: succeed in both or fail in both, identically.
                let id = format!("rec-{round}-{step}");
                let claimed = if next() % 2 == 0 {
                    Some(next() % 500)
                } else {
                    None
                };
                let got = reg.admit(
                    id.clone(),
                    key,
                    peer.to_string(),
                    at,
                    7,
                    claimed,
                    format!("bytes:{id}").into_bytes(),
                );
                let want = model.admit(&id, &key, peer, at);
                match (got, want) {
                    (Ok(()), Ok(())) => {
                        ids.push(id.clone());
                        key_of.insert(id.clone(), key);
                        peer_of.insert(id, peer.to_string());
                    }
                    (Err(_), Err(_)) => {}
                    (g, w) => panic!("admit divergence for {id}: got {g:?}, want {w:?}"),
                }
            }
        }

        // Revalidate a deterministic subset by witness and compare.
        for id in ids.clone() {
            if id.len() % 2 == 0 {
                let through = 50 + (next() % 500);
                let got = reg
                    .revalidate(
                        &id,
                        &RevalidationEvidence::WitnessedGoodThrough { through },
                        450,
                    )
                    .is_ok();
                let want = model.revalidate(&id, through);
                assert_eq!(got, want, "revalidate divergence for {id}");
            }
        }

        let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        for probe in [0u64, 150, 250, 350, 500] {
            check_against_reference(&reg, &model, &id_refs, &key_of, &peer_of, probe);
        }
    }
}

#[test]
fn a_wider_revocation_escalates_a_flagged_record_to_quarantine() {
    let (_ska, key_a) = keypair([3u8; 32]);
    let mut reg = RevocationRegistry::new();
    admit(&mut reg, "mid", key_a, "peer-1", 150, None);

    // Suspected from 200: the record predates it and is only flagged.
    reg.apply_revocation(key_revocation("narrow", key_a, 200, 210))
        .expect("revocation must apply");
    assert!(matches!(
        status_of(&reg, "mid"),
        RecordStatus::Flagged { .. }
    ));

    // Suspected from 100 after all: the record is now suspect.
    assert_eq!(
        reg.apply_revocation(key_revocation("wide", key_a, 100, 220)),
        Ok(vec!["mid".to_string()])
    );
    assert!(matches!(
        status_of(&reg, "mid"),
        RecordStatus::Quarantined { .. }
    ));

    // Nothing de-escalates.
    reg.apply_revocation(key_revocation("late", key_a, 300, 310))
        .expect("revocation must apply");
    assert!(matches!(
        status_of(&reg, "mid"),
        RecordStatus::Quarantined { .. }
    ));
}
