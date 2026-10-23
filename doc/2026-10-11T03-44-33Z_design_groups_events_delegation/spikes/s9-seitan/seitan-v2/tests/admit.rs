//! The admin path: freshness, expiry, revocation, single use, and signature checks.

use ed25519_dalek::{Signer, SigningKey};
use proptest::prelude::*;
use seitan_v2::accept::{self, Proof};
use seitan_v2::admit::SKEW_MS;
use seitan_v2::{Admitted, Invite, Reject, Token};

const TOKEN: &str = "4uywza+b3cga7rd6yc";
const CREATED: u64 = 1_800_000_000_000;
const DAY: u64 = 86_400_000;
const MIN: u64 = 60_000;

fn token() -> Token {
    Token::parse(TOKEN).unwrap()
}

fn invite(created: u64, expires: u64) -> Invite {
    Invite::create(&token(), created, expires, 1)
}

fn proof(uid: &str, ctime: u64) -> Proof {
    accept::sign(&token(), uid, 1, ctime)
}

fn admitted(uid: &str) -> Admitted {
    Admitted {
        uid: uid.to_owned(),
        eldest_seqno: 1,
    }
}

#[test]
fn a_valid_accept_admits_once_and_the_invite_is_then_used_up() {
    let mut inv = invite(CREATED, CREATED + DAY);
    let now = CREATED + MIN;
    assert_eq!(inv.admit(&proof("alice", now), now), Ok(admitted("alice")));
    assert_eq!(inv.admit(&proof("bob", now), now), Err(Reject::Used));
    assert_eq!(
        inv.admit(&proof("alice", now), now),
        Err(Reject::Used),
        "single use, even for the same joiner"
    );
}

#[test]
fn multi_use_invites_still_block_the_same_joiner_twice() {
    let mut inv = Invite::create(&token(), CREATED, CREATED + DAY, 2);
    let now = CREATED + MIN;
    assert_eq!(inv.admit(&proof("alice", now), now), Ok(admitted("alice")));
    assert_eq!(
        inv.admit(&proof("alice", now), now),
        Err(Reject::AlreadyAdmitted)
    );
    assert_eq!(inv.admit(&proof("bob", now), now), Ok(admitted("bob")));
    assert_eq!(inv.admit(&proof("carol", now), now), Err(Reject::Used));
}

#[test]
fn a_rejected_attempt_does_not_use_up_the_invite() {
    let mut inv = invite(CREATED, CREATED + DAY);
    let now = CREATED + MIN;
    let id = inv.id();
    let forged = {
        let message = accept::message("mallory", 1, now, &id);
        let signature = SigningKey::from_bytes(&[9u8; 32]).sign(&message).to_bytes();
        Proof { message, signature }
    };
    assert_eq!(inv.admit(&forged, now), Err(Reject::BadSignature));
    assert_eq!(inv.admit(&proof("alice", now), now), Ok(admitted("alice")));
}

#[test]
fn a_revoked_invite_admits_no_one() {
    let mut inv = invite(CREATED, CREATED + DAY);
    inv.revoke();
    let now = CREATED + MIN;
    assert_eq!(inv.admit(&proof("alice", now), now), Err(Reject::Revoked));
}

#[test]
fn an_expired_invite_rejects_at_processing_time() {
    let expires = CREATED + DAY;
    let mut inv = invite(CREATED, expires);
    assert_eq!(
        inv.admit(&proof("alice", expires), expires),
        Ok(admitted("alice")),
        "still valid at the last instant"
    );
    let mut inv = invite(CREATED, expires);
    assert_eq!(
        inv.admit(&proof("alice", expires), expires + 1),
        Err(Reject::Expired)
    );
}

#[test]
fn an_accept_signed_after_expiry_is_expired_even_if_processed_in_time() {
    let expires = CREATED + DAY;
    let mut inv = invite(CREATED, expires);
    assert_eq!(
        inv.admit(&proof("alice", expires + 1), expires),
        Err(Reject::Expired)
    );
}

#[test]
fn an_accept_from_before_the_invite_existed_is_stale() {
    let mut inv = invite(CREATED, CREATED + DAY);
    let stale = CREATED - SKEW_MS - 1;
    assert_eq!(
        inv.admit(&proof("alice", stale), CREATED + MIN),
        Err(Reject::Stale)
    );

    let mut inv = invite(CREATED, CREATED + DAY);
    assert_eq!(
        inv.admit(&proof("alice", CREATED - SKEW_MS), CREATED + MIN),
        Ok(admitted("alice")),
        "exactly one skew window before creation is still fresh"
    );
}

#[test]
fn an_accept_for_an_earlier_invite_with_the_same_token_is_stale() {
    // The invite ID comes from the token, so a re-issued invite with the same token has the same ID.
    // Without the freshness check, an old accept would work against the new invite.
    let mut old = invite(CREATED, CREATED + DAY);
    let old_proof = proof("alice", CREATED + 10 * MIN);
    assert_eq!(
        old.admit(&old_proof, CREATED + 11 * MIN),
        Ok(admitted("alice"))
    );

    let reissued_at = CREATED + 60 * MIN;
    let mut reissued = invite(reissued_at, reissued_at + DAY);
    assert_eq!(reissued.id(), old.id(), "same token, same invite ID");
    assert_eq!(
        reissued.admit(&old_proof, reissued_at + MIN),
        Err(Reject::Stale)
    );
    assert_eq!(
        reissued.admit(&proof("bob", reissued_at + MIN), reissued_at + MIN),
        Ok(admitted("bob"))
    );
}

#[test]
fn an_accept_from_the_future_beyond_the_skew_window_is_rejected() {
    let now = CREATED + MIN;
    let mut inv = invite(CREATED, CREATED + DAY);
    assert_eq!(
        inv.admit(&proof("alice", now + SKEW_MS + 1), now),
        Err(Reject::FromFuture)
    );

    let mut inv = invite(CREATED, CREATED + DAY);
    assert_eq!(
        inv.admit(&proof("alice", now + SKEW_MS), now),
        Ok(admitted("alice"))
    );
}

#[test]
fn an_accept_for_a_different_token_names_no_known_invite() {
    let mut inv = invite(CREATED, CREATED + DAY);
    let other = accept::sign(&Token::generate().unwrap(), "alice", 1, CREATED + MIN);
    assert_eq!(inv.admit(&other, CREATED + MIN), Err(Reject::UnknownInvite));
}

#[test]
fn a_forged_message_with_the_public_invite_id_fails_the_signature() {
    let mut inv = invite(CREATED, CREATED + DAY);
    let message = accept::message("mallory", 1, CREATED + MIN, &inv.id());
    let signature = SigningKey::from_bytes(&[9u8; 32]).sign(&message).to_bytes();
    assert_eq!(
        inv.admit(&Proof { message, signature }, CREATED + MIN),
        Err(Reject::BadSignature)
    );
}

#[test]
fn changing_any_signed_field_breaks_the_signature() {
    let now = CREATED + MIN;
    let honest = proof("alice", now);
    let id = accept::parse(&honest.message).unwrap().invite_id;

    let swapped_uid = Proof {
        message: accept::message("mallory", 1, now, &id),
        signature: honest.signature,
    };
    let swapped_eldest = Proof {
        message: accept::message("alice", 2, now, &id),
        signature: honest.signature,
    };
    let swapped_ctime = Proof {
        message: accept::message("alice", 1, now - 1, &id),
        signature: honest.signature,
    };
    for tampered in [swapped_uid, swapped_eldest, swapped_ctime] {
        let mut inv = invite(CREATED, CREATED + DAY);
        assert_eq!(inv.admit(&tampered, now), Err(Reject::BadSignature));
    }
    let mut inv = invite(CREATED, CREATED + DAY);
    assert_eq!(
        inv.admit(&honest, now),
        Ok(admitted("alice")),
        "the untampered proof still works"
    );
}

#[test]
fn a_flipped_signature_bit_is_rejected() {
    let now = CREATED + MIN;
    let mut bad = proof("alice", now);
    bad.signature[0] ^= 0x80;
    let mut inv = invite(CREATED, CREATED + DAY);
    assert_eq!(inv.admit(&bad, now), Err(Reject::BadSignature));
}

#[test]
fn garbage_is_malformed() {
    let mut inv = invite(CREATED, CREATED + DAY);
    let garbage = Proof {
        message: vec![0xc1, 0x00],
        signature: [0u8; 64],
    };
    assert_eq!(inv.admit(&garbage, CREATED + MIN), Err(Reject::Malformed));
}

proptest! {
    /// The fresh window is exactly [created - SKEW, min(now + SKEW, expires)], with expiry checked first.
    #[test]
    fn a_ctime_is_admitted_exactly_inside_the_window(
        age in 0u64..DAY,
        lead in 0u64..DAY,
        offset in -(DAY as i64)..(DAY as i64),
    ) {
        let now = CREATED + age;
        let expires = now + lead;
        let ctime = (CREATED as i64 + offset) as u64;
        let mut inv = invite(CREATED, expires);
        let got = inv.admit(&proof("joiner", ctime), now);
        let inside = ctime + SKEW_MS >= CREATED && ctime <= expires && ctime <= now + SKEW_MS;
        prop_assert_eq!(got.is_ok(), inside, "ctime {} now {} expires {} got {:?}", ctime, now, expires, got);
    }
}
