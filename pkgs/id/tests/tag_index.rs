//! The tag index on the tuple encoding (Phase 5, P5.2; design §9.2).
//!
//! Every prefix scan returns exactly the claims that match its query, in both directions. The
//! property draws claims whose names and values include NUL bytes, so a prefix that is only a
//! prefix of a longer field would show up. The regressions pin the `"a"` versus `"a\0b"` case.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;

use id::tag_claims::Claim;
use id::tag_index::{Entry, TagIndex};
use p2panda_core::Hash;
use proptest::prelude::*;

const SUBJECTS: [&[u8]; 3] = [b"subject-0", b"subject-1", b"subject-2"];
const NAMES: [&str; 6] = ["a", "a\0b", "ab", "b", "a\0", "\0"];
const VALUES: [Option<&str>; 7] = [
    None,
    Some("a"),
    Some("a\0b"),
    Some("ab"),
    Some(""),
    Some("\0"),
    Some("a\0"),
];

fn subject(index: usize) -> Hash {
    Hash::digest(SUBJECTS[index])
}

/// A claim with a distinct ID per `n`, so claims never share an ID.
fn claim(n: usize, subject: Hash, name: &str, value: Option<&str>) -> Claim {
    Claim {
        id: Hash::digest(n.to_be_bytes()),
        author: [7; 32],
        subject,
        name: name.to_owned(),
        value: value.map(str::to_owned),
    }
}

/// The IDs of the entries, checked for duplicates and for agreement with their claims.
fn ids_of(entries: &[Entry], claims: &[Claim]) -> BTreeSet<[u8; 32]> {
    let ids: BTreeSet<[u8; 32]> = entries
        .iter()
        .map(|entry| *entry.claim.as_bytes())
        .collect();
    assert_eq!(ids.len(), entries.len(), "a scan returned a claim twice");
    for entry in entries {
        let found = claims
            .iter()
            .find(|claim| claim.id == entry.claim)
            .expect("a scan returned a claim that is not indexed");
        assert_eq!(found.subject, entry.subject);
        assert_eq!(found.name, entry.name);
        assert_eq!(found.value, entry.value);
    }
    ids
}

/// The IDs of the claims that satisfy `matches`.
fn expected(claims: &[Claim], matches: impl Fn(&Claim) -> bool) -> BTreeSet<[u8; 32]> {
    claims
        .iter()
        .filter(|claim| matches(claim))
        .map(|claim| *claim.id.as_bytes())
        .collect()
}

fn claim_strategy() -> impl Strategy<Value = (usize, usize, usize)> {
    (0..SUBJECTS.len(), 0..NAMES.len(), 0..VALUES.len())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// Each scan, over random claims and random queries, returns exactly the matching claims.
    #[test]
    fn test_every_scan_returns_exactly_the_matching_claims(
        drawn in prop::collection::vec(claim_strategy(), 0..40),
        query in claim_strategy(),
    ) {
        let claims: Vec<Claim> = drawn
            .iter()
            .enumerate()
            .map(|(n, &(s, name, value))| claim(n, subject(s), NAMES[name], VALUES[value]))
            .collect();
        let index = TagIndex::new(&claims);
        prop_assert_eq!(index.len(), claims.len());

        let (qs, qn, qv) = query;
        let (s, name, value) = (subject(qs), NAMES[qn], VALUES[qv]);

        let got = ids_of(&index.subject(&s), &claims);
        prop_assert_eq!(got, expected(&claims, |c| c.subject == s));

        let got = ids_of(&index.subject_name(&s, name), &claims);
        prop_assert_eq!(got, expected(&claims, |c| c.subject == s && c.name == name));

        let got = ids_of(&index.subject_name_value(&s, name, value), &claims);
        prop_assert_eq!(
            got,
            expected(&claims, |c| c.subject == s && c.name == name && c.value.as_deref() == value)
        );

        let got = ids_of(&index.value(value), &claims);
        prop_assert_eq!(got, expected(&claims, |c| c.value.as_deref() == value));

        let got = ids_of(&index.value_name(value, name), &claims);
        prop_assert_eq!(
            got,
            expected(&claims, |c| c.value.as_deref() == value && c.name == name)
        );

        let got = ids_of(&index.value_name_subject(value, name, &s), &claims);
        prop_assert_eq!(
            got,
            expected(&claims, |c| c.value.as_deref() == value && c.name == name && c.subject == s)
        );
    }
}

#[test]
fn test_the_name_a_does_not_match_the_longer_name_a_nul_b() {
    let s = subject(0);
    let claims = [
        claim(0, s, "a", None),
        claim(1, s, "a\0b", None),
        claim(2, s, "a\0", None),
    ];
    let index = TagIndex::new(&claims);
    let got = ids_of(&index.subject_name(&s, "a"), &claims);
    assert_eq!(got, BTreeSet::from([*claims[0].id.as_bytes()]));
}

#[test]
fn test_the_value_a_does_not_match_the_longer_value_a_nul_b_in_either_direction() {
    let (s, t) = (subject(0), subject(1));
    let claims = [
        claim(0, s, "city", Some("a")),
        claim(1, t, "city", Some("a\0b")),
        claim(2, t, "city", Some("a\0")),
    ];
    let index = TagIndex::new(&claims);
    // Alpha: a subject's name and value.
    let got = ids_of(&index.subject_name_value(&t, "city", Some("a")), &claims);
    assert!(got.is_empty(), "a exact value on t matched a longer one");
    // Omega: the subjects with a value.
    let got = ids_of(&index.value(Some("a")), &claims);
    assert_eq!(got, BTreeSet::from([*claims[0].id.as_bytes()]));
    let got = ids_of(&index.value_name(Some("a"), "city"), &claims);
    assert_eq!(got, BTreeSet::from([*claims[0].id.as_bytes()]));
}

#[test]
fn test_a_bare_tag_and_an_empty_value_are_different_queries() {
    let s = subject(0);
    let claims = [claim(0, s, "flag", None), claim(1, s, "flag", Some(""))];
    let index = TagIndex::new(&claims);
    let got = ids_of(&index.subject_name_value(&s, "flag", None), &claims);
    assert_eq!(got, BTreeSet::from([*claims[0].id.as_bytes()]));
    let got = ids_of(&index.subject_name_value(&s, "flag", Some("")), &claims);
    assert_eq!(got, BTreeSet::from([*claims[1].id.as_bytes()]));
}
