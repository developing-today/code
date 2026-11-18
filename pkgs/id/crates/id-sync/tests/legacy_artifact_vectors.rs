//! Legacy signed-artifact golden vectors (slice
//! A9113-legacy-hash-vectors, issue #10225; design A-9113.2 and
//! A-9114.3, S3.4–S3.10).
//!
//! Oracle: **golden** — the known source implementation
//! (`id_sync::artifact`) against fixed byte-level vectors. Every pinned
//! preimage, hash and signature below was computed outside this
//! codebase (Python `hashlib` SHA-256 and `cryptography` Ed25519 over a
//! hand-written compact-JSON preimage) and must match the production
//! writer byte for byte. No serializer or signature preimage is changed
//! by this slice: if one of these assertions ever fails, the fix is to
//! restore the old bytes, never to re-baseline the vector or re-sign old
//! artifacts.
//!
//! The legacy format, as the code actually writes it (see
//! `doc/.../part-a-addressing/legacy-artifact-compatibility.md`):
//!
//! - preimage = compact `serde_json::to_vec` of `{author, seq, prev,
//!   kind, body, after?}` in that field order, `after` omitted when
//!   empty, body object keys sorted;
//! - `hash` = lowercase hex SHA-256 of the preimage;
//! - `signature` = lowercase hex Ed25519 over the same preimage.
//!
//! There is no domain prefix and no length framing, although the doc
//! comment on `artifact::signed_bytes` says "length-prefixed and
//! domain-separated". That discrepancy is recorded, not repaired.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _};
use id_sync::artifact::{SignedArtifact, signed_bytes};
use sha2::{Digest as _, Sha256};

/// RFC 8032 section 7.1 TEST 1 secret key.
const SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
/// Its public key, the artifact `author`.
const AUTHOR: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

const PREIMAGE_1: &str = concat!(
    r#"{"author":"d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a","#,
    r#""seq":1,"prev":"","kind":"document","#,
    r#""body":{"content":{"tags":["a","b"],"text":"hello"},"media_type":"application/json"}}"#,
);
const HASH_1: &str = "afab18aed3d6447b6b33fb3441d43f9e1ea909c4d3ef0335674670bbe412bc37";
const SIGNATURE_1: &str = concat!(
    "c3f88bd41c1b6e429225a77516969678525bb153cb83dc290a2d906629c6e7af",
    "17bd385316162578a1de719fde90f96de61e7a4a08c3a267f0fa159d039cc20e",
);

const PREIMAGE_2: &str = concat!(
    r#"{"author":"d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a","#,
    r#""seq":2,"prev":"afab18aed3d6447b6b33fb3441d43f9e1ea909c4d3ef0335674670bbe412bc37","#,
    r#""kind":"tag","body":{"name":"author","#,
    r#""subject":"afab18aed3d6447b6b33fb3441d43f9e1ea909c4d3ef0335674670bbe412bc37","#,
    r#""value":"d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"}}"#,
);
const HASH_2: &str = "1932e31b0e5829b03621f21fc0bdb164f48d2058f5b872b7f65838b02b96a26d";
const SIGNATURE_2: &str = concat!(
    "e492ddb221ac680a310f953b89c0a79496fc2a031d5cd75999d81c2c5d51ebcc",
    "55570253b7923d3a270fe934fbada0557abe0aaf8998da319e3e55244838660d",
);

fn unhex<const N: usize>(text: &str) -> [u8; N] {
    let bytes: Vec<u8> = (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect();
    <[u8; N]>::try_from(bytes).unwrap()
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

fn key() -> SigningKey {
    SigningKey::from_bytes(&unhex::<32>(SEED))
}

fn document_body() -> serde_json::Value {
    serde_json::json!({
        "media_type": "application/json",
        "content": {"text": "hello", "tags": ["a", "b"]},
    })
}

fn tag_body() -> serde_json::Value {
    serde_json::json!({"subject": HASH_1, "name": "author", "value": AUTHOR})
}

#[test]
fn rfc8032_key_is_the_author() {
    assert_eq!(hex(key().verifying_key().as_bytes()), AUTHOR);
}

#[test]
fn production_writer_produces_the_pinned_preimages() {
    let first = signed_bytes(AUTHOR, 1, "", "document", &document_body(), &[]).unwrap();
    assert_eq!(String::from_utf8(first).unwrap(), PREIMAGE_1);
    let second = signed_bytes(AUTHOR, 2, HASH_1, "tag", &tag_body(), &[]).unwrap();
    assert_eq!(String::from_utf8(second).unwrap(), PREIMAGE_2);
}

#[test]
fn pinned_hashes_are_plain_sha256_of_the_preimage() {
    for (preimage, hash) in [(PREIMAGE_1, HASH_1), (PREIMAGE_2, HASH_2)] {
        assert_eq!(hex(&Sha256::digest(preimage.as_bytes())), hash);
        // Not BLAKE3, and not over any framed or prefixed form.
        assert_ne!(hex(blake3::hash(preimage.as_bytes()).as_bytes()), hash);
        assert!(preimage.starts_with("{\"author\":"));
    }
}

#[test]
fn pinned_signatures_are_ed25519_over_the_bare_preimage() {
    let verifying = key().verifying_key();
    for (preimage, signature) in [(PREIMAGE_1, SIGNATURE_1), (PREIMAGE_2, SIGNATURE_2)] {
        let signature = Signature::from_bytes(&unhex::<64>(signature));
        verifying.verify(preimage.as_bytes(), &signature).unwrap();
        // Ed25519 is deterministic: signing again gives the same bytes.
        assert_eq!(key().sign(preimage.as_bytes()), signature);
    }
}

#[test]
fn production_signer_produces_the_pinned_artifacts() {
    let first = SignedArtifact::sign(&key(), 1, "", "document", document_body()).unwrap();
    assert_eq!(first.author, AUTHOR);
    assert_eq!(first.hash, HASH_1);
    assert_eq!(first.signature, SIGNATURE_1);
    first.verify().unwrap();

    let second = SignedArtifact::sign(&key(), 2, HASH_1, "tag", tag_body()).unwrap();
    assert_eq!(second.prev, HASH_1);
    assert_eq!(second.hash, HASH_2);
    assert_eq!(second.signature, SIGNATURE_2);
    second.verify().unwrap();
}

#[test]
fn wire_form_carries_hash_and_signature_beside_the_signed_fields() {
    let first = SignedArtifact::sign(&key(), 1, "", "document", document_body()).unwrap();
    let wire = serde_json::to_string(&first).unwrap();
    let expected = format!(
        "{},\"hash\":\"{HASH_1}\",\"signature\":\"{SIGNATURE_1}\"}}",
        &PREIMAGE_1[..PREIMAGE_1.len() - 1]
    );
    assert_eq!(wire, expected);
    let read: SignedArtifact = serde_json::from_str(&expected).unwrap();
    assert_eq!(read, first);
    read.verify().unwrap();
}

#[test]
fn any_change_to_a_pinned_artifact_fails_verification() {
    let first = SignedArtifact::sign(&key(), 1, "", "document", document_body()).unwrap();
    let mut changed_body = first.clone();
    changed_body.body = serde_json::json!({
        "media_type": "application/json",
        "content": {"text": "hello!", "tags": ["a", "b"]},
    });
    assert!(changed_body.verify().is_err());
    let mut changed_hash = first.clone();
    changed_hash.hash = HASH_2.to_owned();
    assert!(changed_hash.verify().is_err());
    let mut changed_signature = first;
    changed_signature.signature = SIGNATURE_2.to_owned();
    assert!(changed_signature.verify().is_err());
}
