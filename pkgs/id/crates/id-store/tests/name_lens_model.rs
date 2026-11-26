//! Explicit name lenses and voluntary authority-group filters
//! (slice A9121-lenses-groups, issue #10281; statements S4.53–S4.63).
//!
//! Oracle: **model** — the reference model in this file implements
//! A-9121.2 directly from the settled design text: lenses are named
//! projection functions over the authorized candidate/evidence
//! model; the candidate set is stable under the ordering lenses;
//! the authority-group lens filters to the group's curated subset;
//! ordering tie-breaks by declared score, then signer ID, then
//! candidate ID. Every generated candidate set, evidence assignment
//! and lens selection is projected by both the module under test
//! and the model, and every resulting entry — order, score and
//! presence classification — is compared.
//!
//! Presence evidence is admitted into the model only through the
//! real B19 binding (the `existence_receipt` module of
//! A9115-existence-receipt, #10256): a `verified_present` ack must
//! verify and presence-check against the candidate's canonical
//! bytes, and the model confirms the ack's subject digest is the
//! SHA-256 of those bytes with its own `sha2` computation. A
//! `stored_unverified` observation is admitted as inventory only
//! and must never prove presence, under any lens.
//!
//! The crate-root export belongs to the wiring slice, so this test
//! includes the leaf modules by path, as the neighbouring ID
//! slices do.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

#[path = "../../id-core/src/existence_receipt.rs"]
mod existence_receipt;
#[path = "../src/name_candidates.rs"]
mod name_candidates;
#[path = "../src/name_lens.rs"]
mod name_lens;

use std::collections::{BTreeMap, BTreeSet};

use ed25519_dalek::SigningKey;
use proptest::prelude::*;
use sha2::{Digest, Sha256};

use existence_receipt::{AckContext, Responder};
use name_candidates::{ComparisonProfile, NameCandidate, NamePath};
use name_lens::{
    AuthorityGroup, CandidateDigest, CandidateEvidence, GroupCuration, InteractionAttestation,
    Lens, LensEvidence, LensPolicy, NameLensError, PresenceEvidence, StandingClaim,
};

// ---------------------------------------------------------------------------
// Reference model: A-9121.2 written from the design text, independent
// of the module under test.
// ---------------------------------------------------------------------------

fn model_sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

fn model_digest(candidate: &NameCandidate) -> CandidateDigest {
    *candidate.candidate_id().expect("candidate id").digest()
}

/// The model's declared score for one candidate under one lens, or
/// `None` when the lens filters the candidate out.
fn model_score(
    lens: &Lens,
    policy: &LensPolicy,
    candidate: &NameCandidate,
    evidence: Option<&CandidateEvidence>,
    curations: &[GroupCuration],
    groups: &[AuthorityGroup],
) -> Option<u64> {
    match lens {
        Lens::LocalOverrides => {
            if policy.local_signers.contains(candidate.signer()) {
                Some(1)
            } else {
                Some(0)
            }
        }
        Lens::TotalPopularity => {
            let mut total = 0u64;
            if let Some(evidence) = evidence {
                for attestation in &evidence.interactions {
                    total = total.saturating_add(attestation.interactions);
                }
            }
            Some(total)
        }
        Lens::InteractionCohort { cohort } => {
            let mut total = 0u64;
            if let Some(evidence) = evidence {
                for attestation in &evidence.interactions {
                    if cohort.contains(&attestation.observer) {
                        total = total.saturating_add(attestation.interactions);
                    }
                }
            }
            Some(total)
        }
        Lens::StandingCohort { accepted_issuers } => {
            let mut member = false;
            if let Some(evidence) = evidence {
                for claim in &evidence.standing {
                    if accepted_issuers.contains(&claim.issuer) {
                        member = true;
                    }
                }
            }
            Some(u64::from(member))
        }
        Lens::AuthorityGroup { group_id } => {
            let group = groups
                .iter()
                .find(|group| &group.group_id == group_id)
                .expect("generated scenarios register the selected group");
            let digest = model_digest(candidate);
            let mut best: Option<u64> = None;
            for curation in curations {
                if curation.group_id() == group_id
                    && curation.signer() == &group.group_key
                    && curation.candidate() == &digest
                {
                    best = Some(best.map_or(curation.declared_rank(), |rank: u64| {
                        rank.max(curation.declared_rank())
                    }));
                }
            }
            best
        }
    }
}

/// The model's full projection: `(digest, score)` pairs in
/// projection order — score descending, then signer ascending, then
/// candidate ID ascending.
fn model_project(
    lens: &Lens,
    policy: &LensPolicy,
    candidates: &[NameCandidate],
    evidence: &LensEvidence,
    curations: &[GroupCuration],
    groups: &[AuthorityGroup],
) -> Vec<(CandidateDigest, u64, [u8; 32])> {
    let mut rows = Vec::new();
    for candidate in candidates {
        let digest = model_digest(candidate);
        if let Some(score) = model_score(
            lens,
            policy,
            candidate,
            evidence.get(&digest),
            curations,
            groups,
        ) {
            rows.push((digest, score, *candidate.signer()));
        }
    }
    rows.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| left.2.cmp(&right.2))
            .then_with(|| left.0.cmp(&right.0))
    });
    rows
}

/// Admit a signed ack as presence evidence for `content` through
/// the B19 binding, the way the existence gate does: the ack must
/// verify at `now`, its presence response must check against the
/// content, and its subject digest must be the content's SHA-256.
/// Anything else is not `verified_present` evidence for this
/// candidate.
fn model_admit_verified(
    signed: &existence_receipt::SignedAck,
    content: &[u8],
    now: u64,
) -> Option<PresenceEvidence> {
    existence_receipt::verify(signed, now).ok()?;
    existence_receipt::check_presence(&signed.body, content).ok()?;
    if model_sha256(content) != signed.body.subject_digest {
        return None;
    }
    Some(PresenceEvidence::VerifiedPresent {
        subject_digest: signed.body.subject_digest,
        responder: signed.body.responder.clone(),
        responder_key: signed.body.responder_key,
        scope: signed.body.scope.clone(),
        epoch: signed.body.epoch,
        observed_at: signed.body.observed_at,
    })
}

/// Build a real B19 ack over `content` with a fixed responder.
fn model_ack(content: &[u8], scope: &str, valid_until: u64) -> existence_receipt::SignedAck {
    let responder = Responder::new(
        "responder-model",
        SigningKey::from_bytes(&[99u8; 32]),
        [7u8; 32],
    );
    let ctx = AckContext {
        receipt_id: [1u8; 32],
        recipient_key: [2u8; 32],
        observed_at: 10,
        epoch: 3,
        valid_from: 0,
        valid_until,
    };
    responder
        .answer_challenge(content, [5u8; 32], scope, ctx)
        .expect("ack answers")
}

// ---------------------------------------------------------------------------
// Scenario helpers.
// ---------------------------------------------------------------------------

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn candidate(seed: u8, target: &[u8], freshness: u64) -> NameCandidate {
    let name = NamePath::new(["names", "Shared"], ComparisonProfile::Exact).expect("name path");
    NameCandidate::sign(
        name,
        target.to_vec(),
        "model-source",
        "tenant/model",
        freshness,
        &key(seed),
    )
    .expect("candidate signs")
}

fn evidence_with(
    presence: PresenceEvidence,
    interactions: Vec<InteractionAttestation>,
    standing: Vec<StandingClaim>,
) -> CandidateEvidence {
    CandidateEvidence {
        presence,
        interactions,
        standing,
    }
}

fn empty_policy() -> LensPolicy {
    LensPolicy {
        version: 1,
        local_signers: BTreeSet::new(),
    }
}

// ---------------------------------------------------------------------------
// Deterministic scenarios.
// ---------------------------------------------------------------------------

#[test]
fn local_overrides_rank_local_signers_first() {
    let local = candidate(11, b"target-local", 1);
    let remote_a = candidate(12, b"target-a", 2);
    let remote_b = candidate(13, b"target-b", 3);
    let candidates = vec![remote_a.clone(), local.clone(), remote_b.clone()];

    let mut policy = empty_policy();
    policy.local_signers.insert(*local.signer());

    let projection = name_lens::project(
        &candidates,
        &LensEvidence::new(),
        &[],
        &[],
        &Lens::LocalOverrides,
        &policy,
    )
    .expect("projection");

    assert_eq!(projection.entries.len(), 3, "candidate set is stable");
    assert_eq!(projection.entries[0].candidate_digest, model_digest(&local));
    assert_eq!(projection.entries[0].score, 1);
    assert!(projection.entries[1..].iter().all(|entry| entry.score == 0));
    assert_eq!(projection.lens_version, name_lens::LENS_VERSION);
    assert_eq!(projection.policy_version, 1);
}

#[test]
fn popularity_sums_all_observers_and_cohort_restricts() {
    let popular = candidate(21, b"target-popular", 1);
    let quiet = candidate(22, b"target-quiet", 1);
    let popular_digest = model_digest(&popular);
    let quiet_digest = model_digest(&quiet);

    let mut evidence = LensEvidence::new();
    evidence.insert(
        popular_digest,
        evidence_with(
            PresenceEvidence::None,
            vec![
                InteractionAttestation {
                    observer: [31u8; 32],
                    interactions: 5,
                },
                InteractionAttestation {
                    observer: [32u8; 32],
                    interactions: 7,
                },
                // Repeated attestations from one observer accumulate.
                InteractionAttestation {
                    observer: [31u8; 32],
                    interactions: 3,
                },
            ],
            vec![],
        ),
    );
    evidence.insert(
        quiet_digest,
        evidence_with(
            PresenceEvidence::None,
            vec![InteractionAttestation {
                observer: [32u8; 32],
                interactions: 9,
            }],
            vec![],
        ),
    );
    let candidates = vec![quiet.clone(), popular.clone()];

    let total = name_lens::project(
        &candidates,
        &evidence,
        &[],
        &[],
        &Lens::TotalPopularity,
        &empty_policy(),
    )
    .expect("total projection");
    assert_eq!(total.entries[0].candidate_digest, popular_digest);
    assert_eq!(total.entries[0].score, 15);
    assert_eq!(total.entries[1].score, 9);

    let mut cohort = BTreeSet::new();
    cohort.insert([32u8; 32]);
    let cohort_projection = name_lens::project(
        &candidates,
        &evidence,
        &[],
        &[],
        &Lens::InteractionCohort { cohort },
        &empty_policy(),
    )
    .expect("cohort projection");
    assert_eq!(cohort_projection.entries[0].candidate_digest, quiet_digest);
    assert_eq!(cohort_projection.entries[0].score, 9);
    assert_eq!(cohort_projection.entries[1].score, 7);
}

#[test]
fn standing_is_binary_and_issuer_scoped() {
    let held = candidate(41, b"target-held", 1);
    let unheld = candidate(42, b"target-unheld", 1);
    let held_digest = model_digest(&held);
    let unheld_digest = model_digest(&unheld);

    let mut evidence = LensEvidence::new();
    // Two claims from accepted issuers, with unrelated opaque
    // references: membership is still binary, never a trust sum.
    evidence.insert(
        held_digest,
        evidence_with(
            PresenceEvidence::None,
            vec![],
            vec![
                StandingClaim {
                    issuer: [51u8; 32],
                    reference: "claim/opaque-1".to_owned(),
                },
                StandingClaim {
                    issuer: [52u8; 32],
                    reference: "claim/opaque-2".to_owned(),
                },
            ],
        ),
    );
    // A claim from an issuer the policy does not accept never counts.
    evidence.insert(
        unheld_digest,
        evidence_with(
            PresenceEvidence::None,
            vec![],
            vec![StandingClaim {
                issuer: [53u8; 32],
                reference: "claim/opaque-3".to_owned(),
            }],
        ),
    );

    let mut accepted = BTreeSet::new();
    accepted.insert([51u8; 32]);
    accepted.insert([52u8; 32]);
    let projection = name_lens::project(
        &[unheld, held],
        &evidence,
        &[],
        &[],
        &Lens::StandingCohort {
            accepted_issuers: accepted,
        },
        &empty_policy(),
    )
    .expect("standing projection");
    assert_eq!(projection.entries[0].candidate_digest, held_digest);
    assert_eq!(projection.entries[0].score, 1);
    assert_eq!(projection.entries[1].score, 0);
}

#[test]
fn authority_group_filters_to_curated_and_ranks() {
    let first = candidate(61, b"target-first", 1);
    let second = candidate(62, b"target-second", 1);
    let uncurated = candidate(63, b"target-uncurated", 1);
    let first_digest = model_digest(&first);
    let second_digest = model_digest(&second);

    let group_key = key(71);
    let other_key = key(72);
    let groups = vec![
        AuthorityGroup {
            group_id: "curators".to_owned(),
            group_key: group_key.verifying_key().to_bytes(),
        },
        AuthorityGroup {
            group_id: "others".to_owned(),
            group_key: other_key.verifying_key().to_bytes(),
        },
    ];
    let curations = vec![
        GroupCuration::sign("curators", first_digest, 10, &group_key).expect("curation"),
        GroupCuration::sign("curators", second_digest, 30, &group_key).expect("curation"),
        // The other group curating a candidate does not pull it
        // into this group's filter.
        GroupCuration::sign("others", model_digest(&uncurated), 99, &other_key).expect("curation"),
    ];

    let projection = name_lens::project(
        &[first, second, uncurated],
        &LensEvidence::new(),
        &curations,
        &groups,
        &Lens::AuthorityGroup {
            group_id: "curators".to_owned(),
        },
        &empty_policy(),
    )
    .expect("group projection");
    assert_eq!(projection.entries.len(), 2, "filter keeps curated only");
    assert_eq!(projection.entries[0].candidate_digest, second_digest);
    assert_eq!(projection.entries[0].score, 30);
    assert_eq!(projection.entries[1].candidate_digest, first_digest);
    assert_eq!(projection.entries[1].score, 10);
}

#[test]
fn authority_group_rejects_unknown_mismatched_and_conflicting() {
    let only = candidate(81, b"target-only", 1);
    let only_digest = model_digest(&only);
    let group_key = key(82);
    let groups = vec![AuthorityGroup {
        group_id: "curators".to_owned(),
        group_key: group_key.verifying_key().to_bytes(),
    }];

    // Unregistered group.
    let unknown = name_lens::project(
        &[only.clone()],
        &LensEvidence::new(),
        &[],
        &groups,
        &Lens::AuthorityGroup {
            group_id: "missing".to_owned(),
        },
        &empty_policy(),
    );
    assert_eq!(unknown.unwrap_err(), NameLensError::UnknownGroup);

    // A curation naming the group but signed by another key.
    let forged = GroupCuration::sign("curators", only_digest, 5, &key(83)).expect("curation");
    let mismatched = name_lens::project(
        &[only.clone()],
        &LensEvidence::new(),
        &[forged],
        &groups,
        &Lens::AuthorityGroup {
            group_id: "curators".to_owned(),
        },
        &empty_policy(),
    );
    assert_eq!(
        mismatched.unwrap_err(),
        NameLensError::CurationSignerMismatch
    );

    // The same group id registered under two different keys.
    let conflicting = vec![
        AuthorityGroup {
            group_id: "curators".to_owned(),
            group_key: group_key.verifying_key().to_bytes(),
        },
        AuthorityGroup {
            group_id: "curators".to_owned(),
            group_key: key(84).verifying_key().to_bytes(),
        },
    ];
    let conflict = name_lens::project(
        &[only],
        &LensEvidence::new(),
        &[],
        &conflicting,
        &Lens::AuthorityGroup {
            group_id: "curators".to_owned(),
        },
        &empty_policy(),
    );
    assert!(matches!(conflict.unwrap_err(), NameLensError::Invalid(_)));
}

#[test]
fn duplicate_curation_takes_highest_declared_rank() {
    let only = candidate(85, b"target-dup", 1);
    let digest = model_digest(&only);
    let group_key = key(86);
    let groups = vec![AuthorityGroup {
        group_id: "curators".to_owned(),
        group_key: group_key.verifying_key().to_bytes(),
    }];
    let curations = vec![
        GroupCuration::sign("curators", digest, 4, &group_key).expect("curation"),
        GroupCuration::sign("curators", digest, 17, &group_key).expect("curation"),
        GroupCuration::sign("curators", digest, 9, &group_key).expect("curation"),
    ];
    let projection = name_lens::project(
        &[only],
        &LensEvidence::new(),
        &curations,
        &groups,
        &Lens::AuthorityGroup {
            group_id: "curators".to_owned(),
        },
        &empty_policy(),
    )
    .expect("projection");
    assert_eq!(projection.entries.len(), 1);
    assert_eq!(projection.entries[0].score, 17);
}

#[test]
fn lens_names_are_the_design_tokens() {
    assert_eq!(Lens::LocalOverrides.name(), "local_overrides");
    assert_eq!(Lens::TotalPopularity.name(), "total_popularity");
    assert_eq!(
        Lens::InteractionCohort {
            cohort: BTreeSet::new()
        }
        .name(),
        "interaction_cohort"
    );
    assert_eq!(
        Lens::StandingCohort {
            accepted_issuers: BTreeSet::new()
        }
        .name(),
        "standing_cohort"
    );
    assert_eq!(
        Lens::AuthorityGroup {
            group_id: "curators".to_owned()
        }
        .name(),
        "authority_group"
    );
}

#[test]
fn curation_canonical_bytes_are_deterministic() {
    let digest = model_digest(&candidate(89, b"target-canon", 1));
    let first = GroupCuration::sign("curators", digest, 7, &key(90)).expect("curation");
    let again = GroupCuration::sign("curators", digest, 7, &key(90)).expect("curation");
    let other_rank = GroupCuration::sign("curators", digest, 8, &key(90)).expect("curation");
    assert_eq!(
        first.canonical_bytes().expect("canonical"),
        again.canonical_bytes().expect("canonical"),
        "same assertion, same bytes"
    );
    assert_ne!(
        first.canonical_bytes().expect("canonical"),
        other_rank.canonical_bytes().expect("canonical"),
        "the declared rank is inside the signed bytes"
    );
    first.verify().expect("curation verifies");
}

#[test]
fn curation_sign_rejects_invisible_group_id() {
    let digest = model_digest(&candidate(87, b"target-x", 1));
    let rejected = GroupCuration::sign("", digest, 1, &key(88));
    assert!(matches!(rejected.unwrap_err(), NameLensError::Invalid(_)));
    let rejected = GroupCuration::sign("bad\nid", digest, 1, &key(88));
    assert!(matches!(rejected.unwrap_err(), NameLensError::Invalid(_)));
}

#[test]
fn presence_binding_follows_the_b19_gate() {
    let backed = candidate(91, b"target-backed", 1);
    let digest = model_digest(&backed);
    let canonical = backed.canonical_bytes().expect("canonical bytes");

    // An ack over the candidate's canonical bytes is admitted and
    // proves presence.
    let ack = model_ack(&canonical, "tenant/model", 1_000);
    let admitted = model_admit_verified(&ack, &canonical, 500).expect("ack admits");
    assert!(admitted.proves_present());
    assert_eq!(
        admitted.subject_digest(),
        Some(&model_sha256(&canonical)),
        "admitted evidence is bound to the candidate's digest"
    );
    assert_eq!(PresenceEvidence::None.subject_digest(), None);
    if let PresenceEvidence::VerifiedPresent {
        subject_digest,
        epoch,
        ..
    } = &admitted
    {
        assert_eq!(*subject_digest, model_sha256(&canonical));
        assert_eq!(*epoch, 3);
    } else {
        panic!("admitted evidence is verified_present");
    }

    // An ack over other bytes never admits for this candidate.
    let other = model_ack(b"different content", "tenant/model", 1_000);
    assert!(model_admit_verified(&other, &canonical, 500).is_none());

    // An expired ack never admits.
    let expired = model_ack(&canonical, "tenant/model", 100);
    assert!(model_admit_verified(&expired, &canonical, 500).is_none());

    // A stored_unverified observation of the very same bytes is
    // inventory only: it does not prove presence.
    let unverified = PresenceEvidence::StoredUnverified {
        subject_digest: model_sha256(&canonical),
        observed_at: 10,
        location: "model-store".to_owned(),
    };
    assert!(!unverified.proves_present());

    // The classification survives projection unchanged, and
    // presence never reorders: the unverified candidate outranks on
    // score yet still does not prove presence.
    let rival = candidate(92, b"target-rival", 1);
    let rival_digest = model_digest(&rival);
    let mut evidence = LensEvidence::new();
    evidence.insert(digest, evidence_with(admitted, vec![], vec![]));
    evidence.insert(
        rival_digest,
        evidence_with(
            unverified,
            vec![InteractionAttestation {
                observer: [31u8; 32],
                interactions: 50,
            }],
            vec![],
        ),
    );
    let projection = name_lens::project(
        &[backed, rival],
        &evidence,
        &[],
        &[],
        &Lens::TotalPopularity,
        &empty_policy(),
    )
    .expect("projection");
    assert_eq!(projection.entries[0].candidate_digest, rival_digest);
    assert!(!projection.entries[0].proves_present());
    assert_eq!(projection.entries[1].candidate_digest, digest);
    assert!(projection.entries[1].proves_present());
}

// ---------------------------------------------------------------------------
// Generated scenarios: the module against the model.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Scenario {
    candidates: Vec<NameCandidate>,
    evidence: LensEvidence,
    curations: Vec<GroupCuration>,
    groups: Vec<AuthorityGroup>,
    lens: Lens,
    policy: LensPolicy,
}

fn scenario_strategy() -> impl Strategy<Value = Scenario> {
    prop::collection::vec(1u8..=4, 1..=6).prop_flat_map(|seeds| {
        let group_seeds = (71u8, 72u8);
        (
            Just(seeds),
            prop::collection::vec(0u64..50, 64),
            prop::collection::vec(0usize..5, 1),
            0usize..5,
            0u64..3,
        )
            .prop_map(
                move |(seeds, counts, lens_pick, group_pick, policy_version)| {
                    let candidates: Vec<NameCandidate> = seeds
                        .iter()
                        .enumerate()
                        .map(|(index, seed)| {
                            candidate(*seed, format!("target-{index}").as_bytes(), index as u64)
                        })
                        .collect();
                    let mut evidence = LensEvidence::new();
                    let mut cursor = 0usize;
                    let mut take = |counts: &[u64]| {
                        let value = counts.get(cursor).copied().unwrap_or(0);
                        cursor += 1;
                        value
                    };
                    for candidate in &candidates {
                        let digest = model_digest(candidate);
                        let mut interactions = Vec::new();
                        for observer in [31u8, 32u8] {
                            let count = take(&counts);
                            if count > 0 {
                                interactions.push(InteractionAttestation {
                                    observer: [observer; 32],
                                    interactions: count,
                                });
                            }
                        }
                        let mut standing = Vec::new();
                        for issuer in [51u8, 52u8, 53u8] {
                            if take(&counts) % 3 == 0 {
                                standing.push(StandingClaim {
                                    issuer: [issuer; 32],
                                    reference: format!("claim/{issuer}"),
                                });
                            }
                        }
                        let presence = match take(&counts) % 3 {
                            0 => PresenceEvidence::None,
                            1 => PresenceEvidence::StoredUnverified {
                                subject_digest: model_sha256(
                                    &candidate.canonical_bytes().expect("canonical"),
                                ),
                                observed_at: 10,
                                location: "generated-store".to_owned(),
                            },
                            _ => {
                                let canonical = candidate.canonical_bytes().expect("canonical");
                                let ack = model_ack(&canonical, "tenant/model", 1_000);
                                model_admit_verified(&ack, &canonical, 500)
                                    .expect("generated ack admits")
                            }
                        };
                        evidence.insert(
                            digest,
                            CandidateEvidence {
                                presence,
                                interactions,
                                standing,
                            },
                        );
                    }
                    let groups = vec![
                        AuthorityGroup {
                            group_id: "curators".to_owned(),
                            group_key: key(group_seeds.0).verifying_key().to_bytes(),
                        },
                        AuthorityGroup {
                            group_id: "others".to_owned(),
                            group_key: key(group_seeds.1).verifying_key().to_bytes(),
                        },
                    ];
                    let mut curations = Vec::new();
                    for candidate in &candidates {
                        let digest = model_digest(candidate);
                        for (group_id, group_seed) in
                            [("curators", group_seeds.0), ("others", group_seeds.1)]
                        {
                            let rank = take(&counts);
                            if rank % 2 == 0 {
                                curations.push(
                                    GroupCuration::sign(group_id, digest, rank, &key(group_seed))
                                        .expect("curation signs"),
                                );
                            }
                        }
                    }
                    let lens = match lens_pick[0] {
                        0 => Lens::LocalOverrides,
                        1 => Lens::TotalPopularity,
                        2 => Lens::InteractionCohort {
                            cohort: [[32u8; 32]].into_iter().collect(),
                        },
                        3 => Lens::StandingCohort {
                            accepted_issuers: [[51u8; 32], [53u8; 32]].into_iter().collect(),
                        },
                        _ => Lens::AuthorityGroup {
                            group_id: if group_pick == 0 {
                                "curators".to_owned()
                            } else {
                                "others".to_owned()
                            },
                        },
                    };
                    let mut policy = empty_policy();
                    policy.version = policy_version;
                    policy.local_signers = candidates
                        .iter()
                        .take(1)
                        .map(|candidate| *candidate.signer())
                        .collect();
                    Scenario {
                        candidates,
                        evidence,
                        curations,
                        groups,
                        lens,
                        policy,
                    }
                },
            )
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn projection_matches_model(scenario in scenario_strategy()) {
        let projection = name_lens::project(
            &scenario.candidates,
            &scenario.evidence,
            &scenario.curations,
            &scenario.groups,
            &scenario.lens,
            &scenario.policy,
        )
        .expect("projection succeeds");
        let model = model_project(
            &scenario.lens,
            &scenario.policy,
            &scenario.candidates,
            &scenario.evidence,
            &scenario.curations,
            &scenario.groups,
        );
        prop_assert_eq!(projection.entries.len(), model.len());
        for (entry, (digest, score, _signer)) in projection.entries.iter().zip(model.iter()) {
            prop_assert_eq!(&entry.candidate_digest, digest);
            prop_assert_eq!(entry.score, *score);
            let expected_presence = scenario
                .evidence
                .get(digest)
                .map_or(PresenceEvidence::None, |evidence| evidence.presence.clone());
            prop_assert_eq!(&entry.presence, &expected_presence);
        }
        // Under the ordering lenses the candidate set is stable.
        if !matches!(scenario.lens, Lens::AuthorityGroup { .. }) {
            let mut projected: Vec<CandidateDigest> =
                projection.entries.iter().map(|entry| entry.candidate_digest).collect();
            let mut input: Vec<CandidateDigest> =
                scenario.candidates.iter().map(model_digest).collect();
            projected.sort_unstable();
            input.sort_unstable();
            prop_assert_eq!(projected, input);
        }
    }

    #[test]
    fn projection_is_permutation_invariant(scenario in scenario_strategy()) {
        let baseline = name_lens::project(
            &scenario.candidates,
            &scenario.evidence,
            &scenario.curations,
            &scenario.groups,
            &scenario.lens,
            &scenario.policy,
        )
        .expect("baseline projection");
        let mut candidates = scenario.candidates.clone();
        candidates.reverse();
        let mut curations = scenario.curations.clone();
        curations.reverse();
        let shuffled = name_lens::project(
            &candidates,
            &scenario.evidence,
            &curations,
            &scenario.groups,
            &scenario.lens,
            &scenario.policy,
        )
        .expect("shuffled projection");
        // Exact duplicate candidates would make input order the only
        // tiebreak left; generated candidates are distinct, so the
        // projections must be identical.
        let digests: Vec<CandidateDigest> = scenario.candidates.iter().map(model_digest).collect();
        let mut unique = digests.clone();
        unique.sort_unstable();
        unique.dedup();
        prop_assert_eq!(digests.len(), unique.len(), "scenario assumes distinct candidates");
        prop_assert_eq!(baseline.entries, shuffled.entries);
    }
}

#[test]
fn evidence_map_is_keyed_by_candidate_digest() {
    // Guard the scenario plumbing itself: evidence addressed to one
    // candidate never leaks onto another.
    let first = candidate(95, b"target-one", 1);
    let second = candidate(96, b"target-two", 1);
    let mut evidence = LensEvidence::new();
    evidence.insert(
        model_digest(&first),
        evidence_with(
            PresenceEvidence::None,
            vec![InteractionAttestation {
                observer: [31u8; 32],
                interactions: 4,
            }],
            vec![],
        ),
    );
    let mut by_digest = BTreeMap::new();
    let projection = name_lens::project(
        &[first, second],
        &evidence,
        &[],
        &[],
        &Lens::TotalPopularity,
        &empty_policy(),
    )
    .expect("projection");
    for entry in &projection.entries {
        by_digest.insert(entry.candidate_digest, entry.score);
    }
    assert_eq!(
        by_digest[&model_digest(&candidate(95, b"target-one", 1))],
        4
    );
    assert_eq!(
        by_digest[&model_digest(&candidate(96, b"target-two", 1))],
        0
    );
}
