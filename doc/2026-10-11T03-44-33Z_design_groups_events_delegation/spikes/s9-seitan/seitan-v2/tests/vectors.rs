//! Keybase Seitan V2 vectors, from keybase/client `go/teams/seitan_v2_test.go` (`TestSeitanV2KnownSamples`).

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use crypto_secretbox::{KeyInit, Nonce, XSalsa20Poly1305, aead::Aead};
use seitan_v2::{Token, derive};
use serde::Deserialize;

const IKEY: &str = "4uywza+b3cga7rd6yc";
const SIKEY_B64: &str = "Il9ZFgI1yP2b6Hvt53jWIoo8sDre3puyNH8b2es9TTQ=";
const INVITE_ID: &str = "6303ec43bd61d21edb95a433faf06227";
/// The vector's team key, which boxes the pkey below.
const SECRET_KEY_B64: &str = "GBsy8q2vgQ6jEHmQZiNJcxvgxVNlG4IsxKf/zcxtJIA=";
const PKEY_B64: &str = "lAIBxBgxk+0rwtlCacCIzNK8apyoiiN69+tTU1HEgze4fphJmImUv7wFm54ioO9dB876yLUciHsuUItYXH1cSq6cShz2HrjVCSUQCJVxDNQwb3A6x2zv6/mrbUselphhjzxrJFGb6mS7N0cA3cYfdk+WByNEUOVqi6qwzgvAYuwEqM1sAYYb+NgrLEH5+4Tlr5mcWfAtLynLngX3Z4Ef4Mf1";

/// Keybase's `SeitanPKey` is a msgpack array: [version, team key generation, nonce, boxed].
#[derive(Deserialize)]
struct PKey(u8, u8, serde_bytes::ByteBuf, serde_bytes::ByteBuf);

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn token() -> Token {
    Token::parse(IKEY).unwrap()
}

#[test]
fn stretch_matches_the_vector_sikey() {
    let sikey = derive::stretch(&token());
    assert_eq!(sikey.as_bytes(), B64.decode(SIKEY_B64).unwrap().as_slice());
}

#[test]
fn invite_id_matches_the_vector() {
    let sikey = derive::stretch(&token());
    assert_eq!(derive::invite_id(&sikey).to_hex(), INVITE_ID);
}

#[test]
fn ed25519_public_key_is_the_one_boxed_in_the_vector_pkey() {
    // The pkey boxes {K: the derived public key, L: label} under the team key. Opening the box and
    // finding our public key in the plaintext checks the Ed25519 seed derivation against Keybase's data.
    let sikey = derive::stretch(&token());
    let public = derive::signing_key(&sikey).verifying_key().to_bytes();

    let pkey: PKey = rmp_serde::from_slice(&B64.decode(PKEY_B64).unwrap()).unwrap();
    assert_eq!((pkey.0, pkey.1), (2, 1), "version 2, team key generation 1");
    assert_eq!(pkey.2.len(), 24, "nonce is 24 bytes");

    let key = B64.decode(SECRET_KEY_B64).unwrap();
    let cipher = XSalsa20Poly1305::new_from_slice(&key).unwrap();
    let plain = cipher
        .decrypt(Nonce::from_slice(&pkey.2), pkey.3.as_slice())
        .expect("the vector's box opens with the vector's team key");

    assert!(
        contains(&plain, b"Alice"),
        "the label is in the plaintext, so the box opened to the right data"
    );
    let public_hex = hex(&public);
    assert!(
        contains(&plain, public_hex.as_bytes()),
        "the plaintext names our derived public key {public_hex}"
    );
}

#[test]
fn a_wrong_team_key_does_not_open_the_vector_pkey() {
    let pkey: PKey = rmp_serde::from_slice(&B64.decode(PKEY_B64).unwrap()).unwrap();
    let cipher = XSalsa20Poly1305::new_from_slice(&[0u8; 32]).unwrap();
    assert!(
        cipher
            .decrypt(Nonce::from_slice(&pkey.2), pkey.3.as_slice())
            .is_err()
    );
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}
