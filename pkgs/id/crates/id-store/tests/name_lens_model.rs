//! Model oracle for A9121-lenses-groups.
//!
//! The reference projection below deliberately uses simple fixture data:
//! authorized candidates remain a stable set, each lens has its own ranking
//! or filter, and every tie is resolved by signer then candidate identity.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/name_candidates.rs"]
mod name_candidates;
#[path = "../src/name_lens.rs"]
mod name_lens;

use std::collections::BTreeSet;

use ed25519_dalek::SigningKey;
use name_candidates::{ComparisonProfile, NameCandidate, NamePath};
use name_lens::{CandidateEvidence, Lens, LensContext, LensEvidence, project};

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn candidate(seed: u8, target: &[u8]) -> NameCandidate {
    NameCandidate::sign(
        NamePath::new(["docs"], ComparisonProfile::Exact).unwrap(),
        target.to_vec(),
        "test",
        "tenant-a",
        1,
        &key(seed),
    )
    .unwrap()
}

fn evidence(candidate: &NameCandidate, popularity: u64, groups: &[&str]) -> CandidateEvidence {
    CandidateEvidence {
        candidate_id: candidate.candidate_id().unwrap(),
        issuer: [7; 32],
        popularity,
        interacted: false,
        local_override: None,
        standing: None,
        authority_groups: groups.iter().map(|group| (*group).to_owned()).collect(),
    }
}

fn context(candidates: &[NameCandidate]) -> LensContext {
    LensContext {
        scope: "tenant-a".to_owned(),
        now: 50,
        authorized_signers: candidates
            .iter()
            .map(|candidate| *candidate.signer())
            .collect(),
        authorized_evidence_issuers: BTreeSet::from([[7; 32]]),
    }
}

#[test]
fn popularity_is_deterministic_and_group_filter_does_not_grant_authority() {
    let a = candidate(1, b"a");
    let b = candidate(2, b"b");
    let outsider = candidate(3, b"outsider");
    let mut ctx = context(&[a.clone(), b.clone()]);
    let records = vec![
        evidence(&a, 4, &["curated"]),
        evidence(&b, 9, &[]),
        evidence(&outsider, 100, &["curated"]),
    ];

    let ranked = project(&[a.clone(), b.clone(), outsider.clone()], &records, &ctx, &Lens::TotalPopularity).unwrap();
    assert_eq!(ranked.iter().map(|item| item.target()).collect::<Vec<_>>(), vec![b"b".as_slice(), b"a"]);

    let filtered = project(&[a.clone(), b.clone(), outsider], &records, &ctx, &Lens::AuthorityGroup("curated".to_owned())).unwrap();
    assert_eq!(filtered.iter().map(|item| item.target()).collect::<Vec<_>>(), vec![b"a".as_slice()]);

    ctx.authorized_signers.insert(*b.signer());
    let tied = project(&[a.clone(), b.clone()], &[evidence(&a, 9, &[]), evidence(&b, 9, &[])], &ctx, &Lens::TotalPopularity).unwrap();
    let mut expected = vec![a, b];
    expected.sort_by_key(|candidate| (*candidate.signer(), *candidate.candidate_id().unwrap().digest()));
    assert_eq!(tied.iter().map(|item| item.target()).collect::<Vec<_>>(), expected.iter().map(NameCandidate::target).collect::<Vec<_>>());
}

#[test]
fn standing_is_issuer_scope_and_expiry_bound_and_unverified_is_not_standing() {
    let a = candidate(1, b"a");
    let b = candidate(2, b"b");
    let mut ctx = context(&[a.clone(), b.clone()]);
    let records = vec![
        evidence(&a, 0, &[]).with_standing([7; 32], LensEvidence::Standing {
            scope: "tenant-a".to_owned(), expires_at: 51,
        }),
        evidence(&b, 100, &[]).with_standing([7; 32], LensEvidence::StoredUnverified),
    ];
    let ranked = project(&[a.clone(), b.clone()], &records, &ctx, &Lens::StandingCohort).unwrap();
    assert_eq!(ranked.iter().map(|item| item.target()).collect::<Vec<_>>(), vec![b"a".as_slice(), b"b"]);
    ctx.now = 52;
    let expired = project(&[a.clone(), b.clone()], &records, &ctx, &Lens::StandingCohort).unwrap();
    assert_eq!(expired.iter().map(|item| item.target()).collect::<Vec<_>>(), vec![b"a".as_slice(), b"b"]);

    let verified = LensEvidence::VerifiedPresence {
        subject_digest: [9; 32],
        scope: "tenant-a".to_owned(),
        observed_at: 40,
        valid_until: 51,
    };
    assert!(verified.proves_present_in("tenant-a", 50));
    assert!(!verified.proves_present_in("tenant-b", 50));
    assert!(!verified.proves_present_in("tenant-a", 52));
    assert!(!LensEvidence::StoredUnverified.proves_present_in("tenant-a", 50));
}

#[test]
fn local_override_and_interaction_lenses_are_projections_over_the_authorized_set() {
    let a = candidate(1, b"a");
    let b = candidate(2, b"b");
    let records = vec![
        evidence(&a, 100, &[]).with_local_override(Some(2)).with_interacted(true),
        evidence(&b, 0, &[]).with_local_override(Some(1)),
    ];
    let ctx = context(&[a.clone(), b.clone()]);
    let local = project(&[a.clone(), b.clone()], &records, &ctx, &Lens::LocalOverrides).unwrap();
    assert_eq!(local[0].target(), b"a");
    let cohort = project(&[a.clone(), b.clone()], &records, &ctx, &Lens::InteractionCohort).unwrap();
    assert_eq!(cohort.iter().map(|item| item.target()).collect::<Vec<_>>(), vec![b"a".as_slice()]);
}
