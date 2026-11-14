//! Probe (creator-b, 2026-11-04): does `sign_genesis` / `sign_claim` refuse a
//! signer who is not one of the agreement's two players?
//!
//! Original finding (2026-11-04): neither sign function checked seat
//! membership, so a stranger's events signed cleanly; `compute_state`
//! ignored non-player genesis and claims entirely, so such events were
//! dead on arrival.
//!
//! Updated after #6424 (PR #6559): both sign functions now refuse a
//! non-player signer with [`SignError::NotAPlayer`], and `sign_claim`
//! takes the agreement explicitly so it can check the claim against it.
//! This probe now pins the new behaviour; the dedicated refusal suite
//! lives in `tests/attestation_signer_player.rs`.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use id::attestation::{
    GenesisAgreement, MatchState, PlayerId, ResultClaim, SignError, author_id, compute_state,
    player_id, sign_claim, sign_genesis,
};
use p2panda_core::{Hash, SigningKey};

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&std::array::from_fn(|i| {
        seed.wrapping_add(u8::try_from(i).unwrap_or(0))
    }))
}

#[test]
fn probe_nonplayer_signers() {
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
    // 1. A non-player signs the genesis agreement: refused (#6424).
    let g_stranger = sign_genesis(&stranger, 0, None, &agreement, vec![]);
    assert!(
        matches!(g_stranger, Err(SignError::NotAPlayer)),
        "sign_genesis refuses a non-player signer, got {g_stranger:?}"
    );
    // 2. A non-player signs a result claim answering the agreement: refused.
    let claim = ResultClaim {
        agreement: agreement.agreement_hash().unwrap(),
        outcome: "draw".to_owned(),
        final_state_hash: Hash::digest(b"final"),
        frontiers: vec![],
    };
    let c_stranger = sign_claim(&stranger, 0, None, &agreement, &claim, vec![]);
    assert!(
        matches!(c_stranger, Err(SignError::NotAPlayer)),
        "sign_claim refuses a non-player signer, got {c_stranger:?}"
    );
    // 3. Viewer side: with both players' genuine genesis events and no
    //    claims, the state stays Proposed. A stranger can no longer mint
    //    a signed claim through the public API at all (step 2), so there
    //    is nothing for `compute_state` to ignore.
    let gx = sign_genesis(&x, 0, None, &agreement, vec![]).unwrap();
    let go = sign_genesis(&o, 0, None, &agreement, vec![]).unwrap();
    let genesis = vec![
        (author_id(&gx), agreement.clone()),
        (author_id(&go), agreement.clone()),
    ];
    let claims: Vec<(PlayerId, ResultClaim)> = vec![];
    let state = compute_state(&agreement, &genesis, &claims);
    assert_eq!(
        state,
        MatchState::Proposed,
        "compute_state stays Proposed with no player claims"
    );
    // Controls: player-signed genesis/claim work as documented.
    assert!(sign_genesis(&x, 0, None, &agreement, vec![]).is_ok());
    assert!(sign_claim(&x, 0, None, &agreement, &claim, vec![]).is_ok());
    // And the duplicate-seat refusal (the pre-existing sign-time check):
    let dup = GenesisAgreement {
        players: [player_id(&x), player_id(&x)],
        ..agreement.clone()
    };
    assert!(sign_genesis(&x, 0, None, &dup, vec![]).is_err());
}
