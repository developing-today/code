//! Reference oracle for A9121-name-candidates.
//!
//! The candidate multiset is the oracle: signatures must verify, authorized
//! signers are the only visible claims, original spelling survives lookup,
//! and comparison-profile collisions return every matching candidate.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/name_candidates.rs"]
mod name_candidates;

use std::collections::BTreeSet;

use ed25519_dalek::SigningKey;
use name_candidates::{ComparisonProfile, NameCandidate, NameCandidateStore, NamePath};

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

#[test]
fn nfkc_casefold_lookup_preserves_colliding_original_names() {
    let alice = key(1);
    let bob = key(2);
    let profile = ComparisonProfile::NfkcCaseFold;
    let mut store = NameCandidateStore::default();

    let first = NameCandidate::sign(
        NamePath::new(["Docs", "Readme"], profile).unwrap(),
        b"target-a".to_vec(),
        "import-a",
        "tenant-a",
        10,
        &alice,
    )
    .unwrap();
    let second = NameCandidate::sign(
        NamePath::new(["docs", "README"], profile).unwrap(),
        b"target-b".to_vec(),
        "import-b",
        "tenant-a",
        11,
        &alice,
    )
    .unwrap();
    let unauthorized = NameCandidate::sign(
        NamePath::new(["DOCS", "readme"], profile).unwrap(),
        b"private-target".to_vec(),
        "private",
        "tenant-a",
        12,
        &bob,
    )
    .unwrap();

    store.insert(first).unwrap();
    store.insert(second).unwrap();
    store.insert(unauthorized).unwrap();

    let mut authorized = BTreeSet::new();
    authorized.insert(alice.verifying_key().to_bytes());
    let results = store
        .query(
            &NamePath::new(["dÓcs", "readme"], profile).unwrap(),
            &authorized,
        )
        .unwrap();

    assert_eq!(results.len(), 0, "accented input is not the same key");

    let results = store
        .query(
            &NamePath::new(["DOCS", "README"], profile).unwrap(),
            &authorized,
        )
        .unwrap();
    assert_eq!(results.len(), 2, "collision candidates are not collapsed");
    assert_eq!(
        results
            .iter()
            .map(|candidate| candidate.target())
            .collect::<Vec<_>>(),
        vec![b"target-a".as_slice(), b"target-b".as_slice()]
    );
    assert_eq!(results[0].name().components(), &["Docs", "Readme"]);
    assert_eq!(results[1].name().components(), &["docs", "README"]);
    assert_eq!(results[0].source(), "import-a");
    assert_eq!(results[0].scope(), "tenant-a");
}

#[test]
fn exact_profile_does_not_fold_names() {
    let signer = key(3);
    let exact = ComparisonProfile::Exact;
    let mut store = NameCandidateStore::default();
    let candidate = NameCandidate::sign(
        NamePath::new(["Docs"], exact).unwrap(),
        b"target".to_vec(),
        "source",
        "tenant-a",
        1,
        &signer,
    )
    .unwrap();
    store.insert(candidate).unwrap();

    let mut authorized = BTreeSet::new();
    authorized.insert(signer.verifying_key().to_bytes());
    assert_eq!(
        store
            .query(&NamePath::new(["docs"], exact).unwrap(), &authorized)
            .unwrap(),
        [] as [NameCandidate; 0]
    );
}
