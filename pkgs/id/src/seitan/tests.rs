//! Seitan V2 tests. The known answers are Keybase's V2 vector (`go/teams/seitan_v2_test.go`, `TestSeitanV2KnownSamples`).
//! The V1 scrypt check is Keybase's V1 vector (`go/teams/seitan_test.go`, `TestSeitanKnownSamples`).

use std::collections::HashSet;
use std::fmt::Write as _;
use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use crypto_secretbox::{KeyInit, Nonce, XSalsa20Poly1305, aead::Aead};
use ed25519_dalek::Signer;
use serde::{Deserialize, Serialize};

use super::accept::{self, Proof, ProofError};
use super::admit::{Admitted, Invite, Reject};
use super::derive;
use super::freshness::{DEFAULT_WINDOW, Freshness};
use super::token::{ALPHABET, Token, TokenError};

const VECTOR_TOKEN: &str = "4uywza+b3cga7rd6yc";
const VECTOR_INVITE_ID: &str = "6303ec43bd61d21edb95a433faf06227";
const VECTOR_SIKEY_B64: &str = "Il9ZFgI1yP2b6Hvt53jWIoo8sDre3puyNH8b2es9TTQ=";
/// The vector's team key, which boxes the pkey below.
const VECTOR_TEAM_KEY_B64: &str = "GBsy8q2vgQ6jEHmQZiNJcxvgxVNlG4IsxKf/zcxtJIA=";
const VECTOR_PKEY_B64: &str = "lAIBxBgxk+0rwtlCacCIzNK8apyoiiN69+tTU1HEgze4fphJmImUv7wFm54ioO9dB876yLUciHsuUItYXH1cSq6cShz2HrjVCSUQCJVxDNQwb3A6x2zv6/mrbUselphhjzxrJFGb6mS7N0cA3cYfdk+WByNEUOVqi6qwzgvAYuwEqM1sAYYb+NgrLEH5+4Tlr5mcWfAtLynLngX3Z4Ef4Mf1";
/// The V1 token, which has its '+' at index 5. It is used for scrypt only.
const V1_TOKEN: &[u8] = b"raw2ewqp249dyod4";
const V1_SIKEY_B64: &str = "Yqbj8NgHkIG03wfZX/dxpBpqFoXPXNXyQr+MnvCMbS4=";

const MINUTE: u64 = 60 * 1000;
const WEEK: u64 = 7 * 24 * 60 * MINUTE;
/// The admin's clock when an invite is created, in milliseconds since the Unix epoch.
const CREATED: u64 = 1_800_000_000_000;

fn hex(bytes: &[u8]) -> String {
    let mut out = String::new();
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// A msgpack fixstr (length below 32), written out so the expected bytes do not come from rmp-serde.
fn fixstr(s: &str) -> String {
    let len = u8::try_from(s.len()).expect("a fixstr is short");
    format!("{:02x}{}", 0xa0 | len, hex(s.as_bytes()))
}

fn vector_token() -> Token {
    Token::parse(VECTOR_TOKEN).expect("the vector token parses")
}

/// Keybase's vector invite: created at `CREATED`, expiring a week later.
fn vector_invite() -> Invite {
    Invite::create(&vector_token(), CREATED, CREATED + WEEK).expect("the invite is created")
}

fn accept_from(token: &Token, uid: &str, eldest_seqno: u64, ctime: u64) -> Proof {
    accept::sign(token, uid, eldest_seqno, ctime).expect("the proof is signed")
}

// ---------------------------------------------------------------------------
// Tokens
// ---------------------------------------------------------------------------

#[test]
fn the_vector_token_parses_and_keeps_its_form() {
    assert_eq!(vector_token().as_str(), VECTOR_TOKEN);
}

#[test]
fn parse_ignores_case_and_surrounding_space() {
    let token = Token::parse("  4UYWZA+B3CGA7RD6YC \n").expect("the token parses");
    assert_eq!(token.as_str(), "4uywza+b3cga7rd6yc");
}

#[test]
fn parse_rejects_bad_length_plus_position_symbols_and_non_ascii() {
    assert_eq!(
        Token::parse("4uywza+b3cga7rd6y"),
        Err(TokenError::Length(17))
    );
    assert_eq!(
        Token::parse("4uywzab+3cga7rd6yc"),
        Err(TokenError::PlusPosition)
    );
    // The alphabet leaves out 'o', '0', '1', 'i' and 'l', so a typo of those fails here with the symbol named.
    assert_eq!(
        Token::parse("4uywzo+b3cga7rd6yc"),
        Err(TokenError::Symbol('o'))
    );
    assert_eq!(
        Token::parse("0uywza+b3cga7rd6yc"),
        Err(TokenError::Symbol('0'))
    );
    assert_eq!(
        Token::parse("4uywza+b3cga7rd6yé"),
        Err(TokenError::NotAscii)
    );
}

#[test]
fn generated_tokens_have_the_token_shape() {
    for _ in 0..200 {
        let token = Token::generate();
        let s = token.as_str();
        assert_eq!(s.len(), 18);
        assert_eq!(s.as_bytes()[6], b'+');
        assert!(
            s.bytes()
                .enumerate()
                .all(|(i, c)| i == 6 || ALPHABET.contains(&c)),
            "every symbol but the '+' is in the alphabet: {s}"
        );
        assert_eq!(
            Token::parse(s).map(|t| t.as_str().to_owned()),
            Ok(s.to_owned())
        );
    }
}

#[test]
fn generated_tokens_do_not_repeat() {
    let tokens: HashSet<String> = (0..200)
        .map(|_| Token::generate().as_str().to_owned())
        .collect();
    assert_eq!(tokens.len(), 200);
}

#[test]
fn token_debug_output_hides_the_secret() {
    assert!(!format!("{:?}", vector_token()).contains(VECTOR_TOKEN));
}

// ---------------------------------------------------------------------------
// Derivations (known answers)
// ---------------------------------------------------------------------------

#[test]
fn stretch_matches_the_keybase_v2_vector() {
    let sikey = derive::stretch(&vector_token()).expect("scrypt runs");
    assert_eq!(B64.encode(sikey.as_bytes()), VECTOR_SIKEY_B64);
}

#[test]
fn the_scrypt_parameters_match_the_keybase_v1_vector() {
    // V1 and V2 share the stretch. A second vector confirms the scrypt parameters.
    let sikey = derive::stretch_bytes(V1_TOKEN).expect("scrypt runs");
    assert_eq!(B64.encode(sikey.as_bytes()), V1_SIKEY_B64);
}

#[test]
fn invite_id_matches_the_keybase_v2_vector() {
    let sikey = derive::stretch(&vector_token()).expect("scrypt runs");
    let id = derive::invite_id(&sikey).expect("the invite ID derives");
    assert_eq!(id.to_hex(), VECTOR_INVITE_ID);
}

#[test]
fn invite_id_hex_round_trips_and_rejects_other_forms() {
    let id = derive::InviteId::from_hex(VECTOR_INVITE_ID).expect("32 hex characters parse");
    assert_eq!(id.to_hex(), VECTOR_INVITE_ID);
    assert!(derive::InviteId::from_hex("6303ec43bd61d21edb95a433faf0622").is_none());
    assert!(derive::InviteId::from_hex("+303ec43bd61d21edb95a433faf06227").is_none());
    assert!(derive::InviteId::from_hex("6303ec43bd61d21edb95a433faf0622g").is_none());
}

#[test]
fn stage_payloads_are_the_keybase_maps_byte_for_byte() {
    // Encoded by hand from the msgpack spec: fixmap(2), then the keys and values in declaration order.
    assert_eq!(
        hex(&derive::stage_payload("invite_id").expect("encodes")),
        "82a57374616765a9696e766974655f6964a776657273696f6e02"
    );
    assert_eq!(
        hex(&derive::stage_payload("eddsa").expect("encodes")),
        "82a57374616765a56564647361a776657273696f6e02"
    );
}

#[derive(Deserialize)]
struct PKey(u8, u8, serde_bytes::ByteBuf, serde_bytes::ByteBuf);

#[test]
fn ed25519_public_key_is_the_one_boxed_in_the_keybase_vector_pkey() {
    // The pkey boxes {K: the derived public key, L: label} under the team key. Opening the box and finding our
    // public key in the plaintext checks the Ed25519 seed derivation against Keybase's data.
    let sikey = derive::stretch(&vector_token()).expect("scrypt runs");
    let public = derive::signing_key(&sikey)
        .expect("the signing key derives")
        .verifying_key()
        .to_bytes();

    let pkey: PKey =
        rmp_serde::from_slice(&B64.decode(VECTOR_PKEY_B64).expect("base64")).expect("msgpack");
    assert_eq!((pkey.0, pkey.1), (2, 1), "version 2, team key generation 1");
    assert_eq!(pkey.2.len(), 24, "the nonce is 24 bytes");

    let key = B64.decode(VECTOR_TEAM_KEY_B64).expect("base64");
    let cipher = XSalsa20Poly1305::new_from_slice(&key).expect("a 32-byte key");
    let plain = cipher
        .decrypt(Nonce::from_slice(&pkey.2), pkey.3.as_slice())
        .expect("the vector's box opens with the vector's team key");

    assert!(contains(&plain, b"Alice"), "the label is in the plaintext");
    let public_hex = hex(&public);
    assert!(
        contains(&plain, public_hex.as_bytes()),
        "the plaintext names our derived public key {public_hex}"
    );
}

#[test]
fn a_wrong_team_key_does_not_open_the_vector_pkey() {
    let pkey: PKey =
        rmp_serde::from_slice(&B64.decode(VECTOR_PKEY_B64).expect("base64")).expect("msgpack");
    let cipher = XSalsa20Poly1305::new_from_slice(&[0u8; 32]).expect("a 32-byte key");
    assert!(
        cipher
            .decrypt(Nonce::from_slice(&pkey.2), pkey.3.as_slice())
            .is_err()
    );
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

// ---------------------------------------------------------------------------
// Accept proofs
// ---------------------------------------------------------------------------

#[test]
fn accept_message_is_the_keybase_map_in_declaration_order() {
    // Golden bytes: fixmap(6), then stage, uid, eldest_seqno, ctime, invite_id, version. ctime is a 64-bit
    // unsigned value (0xcf). Keybase's go-codec writes the smallest integer form, which rmp-serde matches here.
    let id = derive::InviteId::from_hex(VECTOR_INVITE_ID).expect("parses");
    let got = hex(&accept::message("uid-1", 7, CREATED, &id).expect("encodes"));
    let expected = [
        "86".to_owned(),
        fixstr("stage"),
        fixstr("accept"),
        fixstr("uid"),
        fixstr("uid-1"),
        fixstr("eldest_seqno"),
        "07".to_owned(),
        fixstr("ctime"),
        format!("cf{CREATED:016x}"),
        fixstr("invite_id"),
        format!("d920{}", hex(VECTOR_INVITE_ID.as_bytes())),
        fixstr("version"),
        "02".to_owned(),
    ]
    .concat();
    assert_eq!(got, expected);
}

#[test]
fn a_signed_proof_verifies_under_the_invite_public_key() {
    let invite = vector_invite();
    let proof = accept_from(&vector_token(), "alice", 3, CREATED);
    assert_eq!(accept::verify(&invite.public_key(), &proof), Ok(()));
    let fields = accept::parse(&proof.message).expect("the message parses");
    assert_eq!(
        (fields.uid.as_str(), fields.eldest_seqno, fields.ctime_ms),
        ("alice", 3, CREATED)
    );
    assert_eq!(fields.invite_id.to_hex(), VECTOR_INVITE_ID);
}

#[test]
fn a_proof_from_another_token_fails_under_this_key() {
    let invite = vector_invite();
    let other = accept_from(&Token::generate(), "alice", 3, CREATED);
    assert_eq!(
        accept::verify(&invite.public_key(), &other),
        Err(ProofError::BadSignature)
    );
}

#[test]
fn changing_any_byte_of_the_signed_message_fails() {
    let invite = vector_invite();
    let proof = accept_from(&vector_token(), "alice", 3, CREATED);
    for i in 0..proof.message.len() {
        let mut tampered = proof.clone();
        tampered.message[i] ^= 0x01;
        assert_eq!(
            accept::verify(&invite.public_key(), &tampered),
            Err(ProofError::BadSignature),
            "flipping byte {i} must fail"
        );
    }
}

#[derive(Serialize)]
struct Raw {
    stage: &'static str,
    uid: &'static str,
    eldest_seqno: u64,
    ctime: u64,
    invite_id: &'static str,
    version: u8,
}

fn raw(stage: &'static str, invite_id: &'static str, version: u8) -> Vec<u8> {
    rmp_serde::to_vec_named(&Raw {
        stage,
        uid: "alice",
        eldest_seqno: 3,
        ctime: CREATED,
        invite_id,
        version,
    })
    .expect("encodes")
}

#[test]
fn parse_accepts_the_well_formed_message_and_rejects_the_rest() {
    assert!(accept::parse(&raw("accept", VECTOR_INVITE_ID, 2)).is_ok());
    assert_eq!(
        accept::parse(&raw("acceptx", VECTOR_INVITE_ID, 2)),
        Err(ProofError::Malformed)
    );
    assert_eq!(
        accept::parse(&raw("accept", VECTOR_INVITE_ID, 1)),
        Err(ProofError::Malformed)
    );
    assert_eq!(
        accept::parse(&raw("accept", "6303ec43", 2)),
        Err(ProofError::Malformed)
    );
    assert_eq!(accept::parse(b"\xc1"), Err(ProofError::Malformed));
}

// ---------------------------------------------------------------------------
// The admin path
// ---------------------------------------------------------------------------

#[test]
fn a_valid_accept_admits_the_joiner_once() {
    let mut invite = vector_invite();
    let now = CREATED + MINUTE;
    let alice = accept_from(&vector_token(), "alice", 3, now);
    assert_eq!(
        invite.admit(&alice, now, DEFAULT_WINDOW),
        Ok(Admitted {
            uid: "alice".to_owned(),
            eldest_seqno: 3,
        })
    );
    let bob = accept_from(&vector_token(), "bob", 1, now);
    assert_eq!(invite.admit(&bob, now, DEFAULT_WINDOW), Err(Reject::Used));
}

#[test]
fn a_token_from_another_world_is_refused() {
    // Each world holds its own invite, made from its own token. A joiner with world A's token gets nothing from
    // world B, and world B's invite stays unused.
    let token_a = Token::generate();
    let token_b = Token::generate();
    let mut world_b = Invite::create(&token_b, CREATED, CREATED + WEEK).expect("created");
    let mut world_a = Invite::create(&token_a, CREATED, CREATED + WEEK).expect("created");
    let now = CREATED + MINUTE;
    let from_a = accept_from(&token_a, "alice", 3, now);

    assert_eq!(
        world_b.admit(&from_a, now, DEFAULT_WINDOW),
        Err(Reject::UnknownInvite)
    );
    assert_eq!(world_b.admitted(), None);
    assert!(
        world_a.admit(&from_a, now, DEFAULT_WINDOW).is_ok(),
        "world A admits its own joiner"
    );
}

#[test]
fn a_proof_signed_with_another_key_for_this_invite_is_refused() {
    use ed25519_dalek::SigningKey;
    let mut invite = vector_invite();
    let now = CREATED + MINUTE;
    let message = accept::message("mallory", 1, now, &invite.id()).expect("encodes");
    let forged_key: SigningKey =
        derive::signing_key(&derive::stretch(&Token::generate()).expect("scrypt runs"))
            .expect("the key derives");
    let signature = forged_key.sign(&message).to_bytes();
    let proof = Proof { message, signature };
    assert_eq!(
        invite.admit(&proof, now, DEFAULT_WINDOW),
        Err(Reject::BadSignature)
    );
}

#[test]
fn a_revoked_invite_admits_no_one() {
    let mut invite = vector_invite();
    invite.revoke();
    let now = CREATED + MINUTE;
    let alice = accept_from(&vector_token(), "alice", 3, now);
    assert_eq!(
        invite.admit(&alice, now, DEFAULT_WINDOW),
        Err(Reject::Revoked)
    );
}

#[test]
fn an_accept_after_the_expiry_is_refused() {
    let mut invite = vector_invite();
    let now = CREATED + WEEK + 1;
    let alice = accept_from(&vector_token(), "alice", 3, now);
    assert_eq!(
        invite.admit(&alice, now, DEFAULT_WINDOW),
        Err(Reject::Expired)
    );
}

#[test]
fn an_accept_signed_after_the_expiry_is_refused() {
    let mut invite = vector_invite();
    let signed_late = accept_from(&vector_token(), "alice", 3, CREATED + WEEK + 1);
    assert_eq!(
        invite.admit(&signed_late, CREATED + WEEK, DEFAULT_WINDOW),
        Err(Reject::Expired)
    );
}

#[test]
fn a_refused_attempt_does_not_use_up_the_invite() {
    let mut invite = vector_invite();
    let now = CREATED + MINUTE;
    let forged = Proof {
        message: accept::message("mallory", 1, now, &invite.id()).expect("encodes"),
        signature: [0u8; 64],
    };
    assert_eq!(
        invite.admit(&forged, now, DEFAULT_WINDOW),
        Err(Reject::BadSignature)
    );
    let alice = accept_from(&vector_token(), "alice", 3, now);
    assert!(invite.admit(&alice, now, DEFAULT_WINDOW).is_ok());
}

// ---------------------------------------------------------------------------
// The freshness window (N23)
// ---------------------------------------------------------------------------

const SECOND: u64 = 1000;
/// The default window in milliseconds. It is 5:00.
const FIVE_MINUTES: u64 = 5 * MINUTE;

fn fresh_invite() -> Invite {
    Invite::create(&vector_token(), CREATED, CREATED + WEEK).expect("the invite is created")
}

#[test]
fn an_accept_signed_4_59_before_the_invite_is_admitted() {
    let mut invite = fresh_invite();
    let now = CREATED + MINUTE;
    let ctime = CREATED - (FIVE_MINUTES - 59 * SECOND);
    let proof = accept_from(&vector_token(), "alice", 3, ctime);
    assert!(invite.admit(&proof, now, DEFAULT_WINDOW).is_ok());
}

#[test]
fn an_accept_signed_exactly_5_00_before_the_invite_is_admitted() {
    // The window is inclusive: an accept at the edge is not older than the window.
    let mut invite = fresh_invite();
    let now = CREATED + MINUTE;
    let ctime = CREATED - FIVE_MINUTES;
    let proof = accept_from(&vector_token(), "alice", 3, ctime);
    assert!(invite.admit(&proof, now, DEFAULT_WINDOW).is_ok());
}

#[test]
fn an_accept_signed_5_01_before_the_invite_is_refused_with_the_window_named() {
    let mut invite = fresh_invite();
    let now = CREATED + MINUTE;
    let ctime = CREATED - (FIVE_MINUTES + SECOND);
    let proof = accept_from(&vector_token(), "alice", 3, ctime);
    let refused = invite.admit(&proof, now, DEFAULT_WINDOW);
    assert_eq!(refused, Err(Reject::Stale(DEFAULT_WINDOW)));
    let message = refused.expect_err("refused").to_string();
    assert!(
        message.contains("older than the 300 s freshness window"),
        "the error names the window: {message}"
    );
    assert_eq!(invite.admitted(), None, "a stale accept uses nothing up");
}

#[test]
fn the_window_also_edges_accepts_signed_from_the_future() {
    let now = CREATED + MINUTE;
    let mut at_edge = fresh_invite();
    let on_edge = accept_from(&vector_token(), "alice", 3, now + FIVE_MINUTES);
    assert!(at_edge.admit(&on_edge, now, DEFAULT_WINDOW).is_ok());

    let mut past_edge = fresh_invite();
    let past = accept_from(&vector_token(), "alice", 3, now + FIVE_MINUTES + SECOND);
    assert_eq!(
        past_edge.admit(&past, now, DEFAULT_WINDOW),
        Err(Reject::FromFuture(DEFAULT_WINDOW))
    );
}

#[test]
fn the_window_resolves_inviter_then_world_then_server_then_default() {
    let two = Duration::from_secs(120);
    let ten = Duration::from_secs(600);
    let one = Duration::from_secs(60);
    assert_eq!(DEFAULT_WINDOW, Duration::from_secs(300));
    assert_eq!(Freshness::default().window(), DEFAULT_WINDOW);
    let server = Freshness {
        server: Some(two),
        ..Freshness::default()
    };
    assert_eq!(server.window(), two);
    let world = Freshness {
        server: Some(two),
        world: Some(ten),
        ..Freshness::default()
    };
    assert_eq!(world.window(), ten, "the world overrides the server");
    let inviter = Freshness {
        server: Some(two),
        world: Some(ten),
        inviter: Some(one),
    };
    assert_eq!(inviter.window(), one, "the inviter overrides the world");
}

#[test]
fn a_configured_window_overrides_the_default_in_both_directions() {
    let now = CREATED + MINUTE;
    let strict = Freshness {
        server: Some(Duration::from_secs(120)),
        ..Freshness::default()
    }
    .window();
    let wide = Freshness {
        world: Some(Duration::from_secs(600)),
        ..Freshness::default()
    }
    .window();

    // Three minutes before the invite: the default admits it, the two-minute window refuses it.
    let three_minutes_old = accept_from(&vector_token(), "alice", 3, CREATED - 3 * MINUTE);
    assert!(
        fresh_invite()
            .admit(&three_minutes_old, now, DEFAULT_WINDOW)
            .is_ok()
    );
    assert_eq!(
        fresh_invite().admit(&three_minutes_old, now, strict),
        Err(Reject::Stale(strict))
    );

    // Seven minutes before the invite: the default refuses it, the ten-minute window admits it.
    let seven_minutes_old = accept_from(&vector_token(), "bob", 1, CREATED - 7 * MINUTE);
    assert_eq!(
        fresh_invite().admit(&seven_minutes_old, now, DEFAULT_WINDOW),
        Err(Reject::Stale(DEFAULT_WINDOW))
    );
    assert!(fresh_invite().admit(&seven_minutes_old, now, wide).is_ok());
}
