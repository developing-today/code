//! How subject lookups map to tuple keys. Uses the real `pkgs/id/src/tuple.rs`, included
//! read-only with `#[path]` and not modified.
//!
//! Keys: alpha = (subject, name, value|null, claim) and omega = (value|null, name, subject, claim).
//! The subject is always 32 bytes, so a subject-only prefix is exact. A name or value prefix is
//! not: the current encoder ends a field with one 0x00, and an escaped NUL is 0x00 0xFF.

#[allow(dead_code, clippy::all, clippy::pedantic)]
#[path = "/home/user/code/pkgs/id/src/tuple.rs"]
mod tuple;

use std::collections::BTreeSet;

use tuple::TupleEncoder;

const SUBJECT: [u8; 32] = [0, 1, 0, 2, 0, 0, 3, 4, 0, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27];
const OTHER: [u8; 32] = [0, 1, 0, 2, 0, 0, 3, 4, 0, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 99];
const CLAIM_1: [u8; 32] = [1; 32];
const CLAIM_2: [u8; 32] = [2; 32];

/// alpha key with the current `tuple.rs` encoder.
fn alpha_key(subject: &[u8], name: &str, value: Option<&str>, claim: &[u8]) -> Vec<u8> {
    let mut enc = TupleEncoder::new();
    enc.bytes(subject).string(name);
    match value {
        Some(v) => enc.string(v),
        None => enc.null(),
    };
    enc.bytes(claim).build()
}

/// Field with the task-01 terminator: a tag, then the bytes with 0x00 escaped as 0x00 0xFF,
/// then 0x00 0x00. Escaped data never holds 0x00 0x00, so the end of a field is unambiguous.
fn fixed_field(out: &mut Vec<u8>, tag: u8, bytes: &[u8]) {
    out.push(tag);
    for &b in bytes {
        out.push(b);
        if b == 0 {
            out.push(0xFF);
        }
    }
    out.extend_from_slice(&[0, 0]);
}

fn fixed_alpha_key(subject: &[u8], name: &str, value: Option<&str>, claim: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    fixed_field(&mut out, 0x01, subject);
    fixed_field(&mut out, 0x02, name.as_bytes());
    match value {
        Some(v) => fixed_field(&mut out, 0x02, v.as_bytes()),
        None => out.push(0x00),
    }
    fixed_field(&mut out, 0x01, claim);
    out
}

#[test]
fn subject_prefix_is_exact_for_fixed_32_byte_subjects() {
    let keys = [
        alpha_key(&SUBJECT, "a", None, &CLAIM_1),
        alpha_key(&OTHER, "a", None, &CLAIM_2),
    ];
    let prefix = TupleEncoder::new().bytes(&SUBJECT).build();
    let hits: Vec<&Vec<u8>> = keys.iter().filter(|k| k.starts_with(&prefix)).collect();
    assert_eq!(hits, vec![&keys[0]]);
}

#[test]
fn current_name_prefix_matches_a_longer_name_containing_nul() {
    // Characterisation of the bug in task 01. This assertion should flip once the terminator is fixed.
    let keys = [
        alpha_key(&SUBJECT, "a", None, &CLAIM_1),
        alpha_key(&SUBJECT, "a\0b", None, &CLAIM_2),
    ];
    let prefix = TupleEncoder::new().bytes(&SUBJECT).string("a").build();
    let hits = keys.iter().filter(|k| k.starts_with(&prefix)).count();
    assert_eq!(hits, 2, "tuple.rs: name \"a\" also matches \"a\\0b\"");
}

#[test]
fn fixed_terminator_prefix_is_exact() {
    let keys = [
        fixed_alpha_key(&SUBJECT, "a", None, &CLAIM_1),
        fixed_alpha_key(&SUBJECT, "a\0b", None, &CLAIM_2),
    ];
    let mut prefix = Vec::new();
    fixed_field(&mut prefix, 0x01, &SUBJECT);
    fixed_field(&mut prefix, 0x02, b"a");
    let hits: Vec<&Vec<u8>> = keys.iter().filter(|k| k.starts_with(&prefix)).collect();
    assert_eq!(hits, vec![&keys[0]]);
}

#[test]
fn sorted_range_scans_on_the_current_encoder() {
    // The index is a sorted byte range. S6 found that a successor bound fails and that `prefix ++ 0xFF` works.
    let index: BTreeSet<Vec<u8>> = [
        alpha_key(&SUBJECT, "a", None, &CLAIM_1),
        alpha_key(&SUBJECT, "a\0b", None, &CLAIM_2),
    ]
    .into_iter()
    .collect();
    let prefix = TupleEncoder::new().bytes(&SUBJECT).string("a").build();
    let mut successor = prefix.clone();
    *successor.last_mut().expect("non-empty") += 1;
    let mut bounded = prefix.clone();
    bounded.push(0xFF);

    let by_successor = index.range(prefix.clone()..successor).count();
    let by_ff = index.range(prefix..bounded).count();
    assert_eq!(by_successor, 2, "successor bound includes the NUL name");
    assert_eq!(by_ff, 1, "prefix ++ 0xFF bound excludes it");
}

#[test]
fn fixed_terminator_keeps_string_order() {
    let names = ["", "a", "a\0", "a\0b", "ab", "b"];
    let mut encoded: Vec<Vec<u8>> = names
        .iter()
        .map(|n| {
            let mut out = Vec::new();
            fixed_field(&mut out, 0x02, n.as_bytes());
            out
        })
        .collect();
    let in_order = encoded.clone();
    encoded.reverse();
    encoded.sort();
    assert_eq!(encoded, in_order);
}
