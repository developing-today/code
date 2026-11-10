use ed25519_dalek::{Signer, SigningKey};
use proptest::prelude::*;
use seitan_v2::accept::{self, Proof, ProofError};
use seitan_v2::{InviteId, Token, derive};
use serde::Serialize;

const TOKEN: &str = "4uywza+b3cga7rd6yc";
const INVITE_ID: &str = "6303ec43bd61d21edb95a433faf06227";
const CTIME: u64 = 1_800_000_000_000;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A msgpack fixstr (length below 32), written out so the expected bytes do not come from rmp-serde.
fn fixstr(s: &str) -> String {
    format!("{:02x}{}", 0xa0 | s.len() as u8, hex(s.as_bytes()))
}

fn public_key() -> [u8; 32] {
    derive::signing_key(&derive::stretch(&Token::parse(TOKEN).unwrap()))
        .verifying_key()
        .to_bytes()
}

/// An accept map with arbitrary field values, for building malformed messages.
#[derive(Serialize)]
struct Raw {
    stage: &'static str,
    uid: &'static str,
    eldest_seqno: u64,
    ctime: u64,
    invite_id: &'static str,
    version: u8,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct WithExtra {
    stage: &'static str,
    uid: &'static str,
    eldest_seqno: u64,
    ctime: u64,
    invite_id: &'static str,
    version: u8,
    extra: u8,
}

fn raw(stage: &'static str, invite_id: &'static str, version: u8) -> Vec<u8> {
    rmp_serde::to_vec_named(&Raw {
        stage,
        uid: "alice",
        eldest_seqno: 3,
        ctime: CTIME,
        invite_id,
        version,
    })
    .unwrap()
}

#[test]
fn accept_message_is_the_keybase_map_in_declaration_order() {
    // Golden bytes: fixmap(6), then stage, uid, eldest_seqno, ctime, invite_id, version.
    // ctime is a 64-bit unsigned value (0xcf). This pins rmp-serde's integer choice. Keybase's go-codec may differ.
    let id = InviteId::from_hex(INVITE_ID).unwrap();
    let got = hex(&accept::message("uid-1", 7, CTIME, &id));
    let expected = [
        "86".to_owned(),
        fixstr("stage"),
        fixstr("accept"),
        fixstr("uid"),
        fixstr("uid-1"),
        fixstr("eldest_seqno"),
        "07".to_owned(),
        fixstr("ctime"),
        format!("cf{CTIME:016x}"),
        fixstr("invite_id"),
        format!("d920{}", hex(INVITE_ID.as_bytes())),
        fixstr("version"),
        "02".to_owned(),
    ]
    .concat();
    assert_eq!(got, expected);
}

#[test]
fn a_signed_proof_verifies_under_the_invite_public_key() {
    let proof = accept::sign(&Token::parse(TOKEN).unwrap(), "alice", 3, CTIME);
    assert_eq!(accept::verify(&public_key(), &proof), Ok(()));
    let fields = accept::parse(&proof.message).unwrap();
    assert_eq!(
        (fields.uid.as_str(), fields.eldest_seqno, fields.ctime_ms),
        ("alice", 3, CTIME)
    );
    assert_eq!(fields.invite_id.to_hex(), INVITE_ID);
}

#[test]
fn a_proof_from_another_token_fails_under_this_key() {
    let other = accept::sign(&Token::generate().unwrap(), "alice", 3, CTIME);
    assert_eq!(
        accept::verify(&public_key(), &other),
        Err(ProofError::BadSignature)
    );
}

#[test]
fn parse_accepts_the_well_formed_message_and_rejects_the_rest() {
    assert!(accept::parse(&raw("accept", INVITE_ID, 2)).is_ok());
    assert_eq!(
        accept::parse(&raw("acceptx", INVITE_ID, 2)).unwrap_err(),
        ProofError::Malformed
    );
    assert_eq!(
        accept::parse(&raw("accept", INVITE_ID, 1)).unwrap_err(),
        ProofError::Malformed
    );
    assert_eq!(
        accept::parse(&raw("accept", "6303ec43", 2)).unwrap_err(),
        ProofError::Malformed
    );
    assert_eq!(
        accept::parse(&raw("accept", "+303ec43bd61d21edb95a433faf06227", 2)).unwrap_err(),
        ProofError::Malformed
    );
    assert_eq!(accept::parse(&[0xc1]).unwrap_err(), ProofError::Malformed);

    let extra = rmp_serde::to_vec_named(&WithExtra {
        stage: "accept",
        uid: "alice",
        eldest_seqno: 3,
        ctime: CTIME,
        invite_id: INVITE_ID,
        version: 2,
        extra: 1,
    })
    .unwrap();
    assert_eq!(accept::parse(&extra).unwrap_err(), ProofError::Malformed);
}

#[test]
fn signed_integer_families_decode_to_the_same_fields() {
    // Some encoders write non-negative integers in the signed msgpack families. The verifier reads the bytes
    // it was sent, so a message like this should still decode and verify. Built by hand, not with rmp-serde.
    let fixstr_bytes = |s: &str| [vec![0xa0 | s.len() as u8], s.as_bytes().to_vec()].concat();
    let mut message = vec![0x86];
    message.extend(fixstr_bytes("stage"));
    message.extend(fixstr_bytes("accept"));
    message.extend(fixstr_bytes("uid"));
    message.extend(fixstr_bytes("alice"));
    message.extend(fixstr_bytes("eldest_seqno"));
    message.extend([0xd0, 0x03]);
    message.extend(fixstr_bytes("ctime"));
    message.push(0xd3);
    message.extend_from_slice(&CTIME.to_be_bytes());
    message.extend(fixstr_bytes("invite_id"));
    message.extend([0xd9, 0x20]);
    message.extend_from_slice(INVITE_ID.as_bytes());
    message.extend(fixstr_bytes("version"));
    message.extend([0xd0, 0x02]);

    let key = derive::signing_key(&derive::stretch(&Token::parse(TOKEN).unwrap()));
    let proof = Proof {
        signature: key.sign(&message).to_bytes(),
        message,
    };
    let fields = accept::parse(&proof.message).unwrap();
    assert_eq!(
        (fields.uid.as_str(), fields.eldest_seqno, fields.ctime_ms),
        ("alice", 3, CTIME)
    );
    assert_eq!(accept::verify(&public_key(), &proof), Ok(()));
}

#[test]
fn the_signature_covers_the_exact_bytes_sent() {
    let proof = accept::sign(&Token::parse(TOKEN).unwrap(), "alice", 3, CTIME);
    let mut edited = proof.clone();
    let last = edited.message.len() - 1;
    edited.message[last] ^= 0x01;
    assert_eq!(
        accept::verify(&public_key(), &edited),
        Err(ProofError::BadSignature)
    );
}

#[test]
fn a_forged_message_signed_with_another_key_fails() {
    // An attacker can read the invite ID and the public key from the chain, but not the token.
    let id = InviteId::from_hex(INVITE_ID).unwrap();
    let message = accept::message("mallory", 1, CTIME, &id);
    let forged = SigningKey::from_bytes(&[9u8; 32]).sign(&message).to_bytes();
    let proof = Proof {
        message,
        signature: forged,
    };
    assert_eq!(
        accept::verify(&public_key(), &proof),
        Err(ProofError::BadSignature)
    );
}

proptest! {
    #[test]
    fn fields_survive_the_round_trip(
        uid in "[ -~]{0,64}",
        eldest in any::<u64>(),
        ctime in any::<u64>(),
        id in any::<[u8; 16]>(),
    ) {
        let invite_id = InviteId::from_hex(&hex(&id)).unwrap();
        let decoded = accept::parse(&accept::message(&uid, eldest, ctime, &invite_id)).unwrap();
        prop_assert_eq!(decoded.uid, uid);
        prop_assert_eq!(decoded.eldest_seqno, eldest);
        prop_assert_eq!(decoded.ctime_ms, ctime);
        prop_assert_eq!(decoded.invite_id, invite_id);
    }

    #[test]
    fn flipping_any_message_bit_breaks_the_signature(index in any::<prop::sample::Index>(), bit in 0u8..8) {
        let proof = accept::sign(&Token::parse(TOKEN).unwrap(), "alice", 3, CTIME);
        let mut edited = proof.clone();
        let at = index.index(edited.message.len());
        edited.message[at] ^= 1 << bit;
        prop_assert!(accept::verify(&public_key(), &edited).is_err());
    }
}
