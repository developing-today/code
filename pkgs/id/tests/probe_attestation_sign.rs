//! Regression probe: signing an attestation requires the signer to be one
//! of the agreement's two players; valid player signatures remain accepted.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use id::attestation::{
    GenesisAgreement, ResultClaim, SignError, player_id, sign_claim, sign_genesis,
};
use p2panda_core::{Hash, SigningKey};

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&std::array::from_fn(|i| {
        seed.wrapping_add(u8::try_from(i).unwrap_or(0))
    }))
}

#[test]
fn probe_nonplayer_signers_are_refused() {
    let x = key(0);
    let o = key(100);
    let stranger = key(200);
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

    let stranger_genesis = sign_genesis(&stranger, 0, None, &agreement, vec![]);
    assert!(
        matches!(stranger_genesis, Err(SignError::NotAPlayer)),
        "expected NotAPlayer, got {stranger_genesis:?}"
    );
    let stranger_claim = sign_claim(&stranger, 0, None, &agreement, &claim, vec![]);
    assert!(
        matches!(stranger_claim, Err(SignError::NotAPlayer)),
        "expected NotAPlayer, got {stranger_claim:?}"
    );

    // Controls: valid player signatures work, and duplicate seats are refused.
    assert!(sign_genesis(&x, 0, None, &agreement, vec![]).is_ok());
    assert!(sign_claim(&x, 0, None, &agreement, &claim, vec![]).is_ok());
    let duplicate_seat = GenesisAgreement {
        players: [player_id(&x), player_id(&x)],
        ..agreement
    };
    assert!(sign_genesis(&x, 0, None, &duplicate_seat, vec![]).is_err());
}
