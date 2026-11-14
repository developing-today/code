//! #6424: sign_genesis / sign_claim refuse signers that are not players
//! of the agreement, since compute_state never counts their objects.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use p2panda_core::{Hash, SigningKey};

use id::attestation::{
    GenesisAgreement, MatchState, ResultClaim, SignError, author_id, compute_state, player_id,
    sign_claim, sign_genesis,
};

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&std::array::from_fn(|i| {
        seed.wrapping_add(u8::try_from(i).unwrap_or(0))
    }))
}

fn setup() -> (SigningKey, SigningKey, GenesisAgreement, ResultClaim) {
    let x = key(0);
    let o = key(100);
    let agreement = GenesisAgreement {
        game_id: "tic-tac-toe".to_owned(),
        logic_hash: Hash::digest(b"logic"),
        players: [player_id(&x), player_id(&o)],
        witness: None,
        objection_window_secs: 3600,
    };
    let claim = ResultClaim {
        agreement: agreement.agreement_hash().unwrap(),
        outcome: "draw".to_owned(),
        final_state_hash: Hash::digest(b"final"),
        frontiers: vec![],
    };
    (x, o, agreement, claim)
}

#[test]
fn sign_genesis_refuses_non_player() {
    let (_x, _o, agreement, _claim) = setup();
    let stranger = key(200);
    let result = sign_genesis(&stranger, 0, None, &agreement, vec![]);
    assert!(
        matches!(result, Err(SignError::NotAPlayer)),
        "expected NotAPlayer, got {result:?}"
    );
}

#[test]
fn sign_claim_refuses_non_player() {
    let (_x, _o, agreement, claim) = setup();
    let stranger = key(200);
    let result = sign_claim(&stranger, 1, None, &agreement, &claim, vec![]);
    assert!(
        matches!(result, Err(SignError::NotAPlayer)),
        "expected NotAPlayer, got {result:?}"
    );
}

#[test]
fn sign_claim_refuses_claim_for_another_agreement() {
    let (x, _o, agreement, mut claim) = setup();
    claim.agreement = Hash::digest(b"some other agreement");
    let result = sign_claim(&x, 1, None, &agreement, &claim, vec![]);
    assert!(
        matches!(result, Err(SignError::AgreementMismatch)),
        "expected AgreementMismatch, got {result:?}"
    );
}

#[test]
fn both_players_still_sign_and_compute_agreed() {
    let (x, o, agreement, claim) = setup();
    let gx = sign_genesis(&x, 0, None, &agreement, vec![]).unwrap();
    let go = sign_genesis(&o, 0, None, &agreement, vec![]).unwrap();
    sign_claim(&x, 1, Some(gx.id), &agreement, &claim, vec![gx.id, go.id]).unwrap();
    sign_claim(&o, 1, Some(go.id), &agreement, &claim, vec![gx.id, go.id]).unwrap();
    let genesis = vec![
        (author_id(&gx), agreement.clone()),
        (author_id(&go), agreement.clone()),
    ];
    let claims = vec![(player_id(&x), claim.clone()), (player_id(&o), claim)];
    assert_eq!(
        compute_state(&agreement, &genesis, &claims),
        MatchState::Agreed
    );
}
