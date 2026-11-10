//! Probe (creator-b, 2026-11-04): does `sign_genesis` / `sign_claim` refuse a
//! signer who is not one of the agreement's two players?
//!
//! Pins the observed behaviour: neither sign function checks seat membership,
//! so a stranger's events sign cleanly. `compute_state` ignores non-player
//! genesis and claims entirely, so such events are dead on arrival; the
//! sign-time refusal that does exist is the duplicate-seat viewer invariant
//! (#2863 precedent).
#![allow(clippy::unwrap_used, clippy::expect_used)]
use id::attestation::{
    GenesisAgreement, MatchState, ResultClaim, author_id, compute_state, player_id, sign_claim,
    sign_genesis,
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
    // 1. A non-player signs the genesis agreement.
    let g_stranger = sign_genesis(&stranger, 0, None, &agreement, vec![]);
    assert!(
        g_stranger.is_ok(),
        "sign_genesis does not refuse a non-player signer"
    );
    // 2. A non-player signs a result claim answering the agreement.
    let claim = ResultClaim {
        agreement: agreement.agreement_hash().unwrap(),
        outcome: "draw".to_owned(),
        final_state_hash: Hash::digest(b"final"),
        frontiers: vec![],
    };
    let c_stranger = sign_claim(&stranger, 0, None, &claim, vec![]);
    assert!(
        c_stranger.is_ok(),
        "sign_claim does not refuse a non-player signer"
    );
    // 3. Viewer side: even with both players' genuine genesis events,
    //    the stranger's claim cannot move the state (still Proposed,
    //    since only one player -- none -- has claimed; the stranger
    //    claim is ignored).
    let gx = sign_genesis(&x, 0, None, &agreement, vec![]).unwrap();
    let go = sign_genesis(&o, 0, None, &agreement, vec![]).unwrap();
    let genesis = vec![
        (author_id(&gx), agreement.clone()),
        (author_id(&go), agreement.clone()),
        (author_id(&g_stranger.unwrap()), agreement.clone()),
    ];
    let claims = vec![(author_id(&c_stranger.unwrap()), claim.clone())];
    let state = compute_state(&agreement, &genesis, &claims);
    assert_eq!(
        state,
        MatchState::Proposed,
        "compute_state ignores a stranger's claim"
    );
    // Controls: player-signed genesis/claim work as documented.
    assert!(sign_genesis(&x, 0, None, &agreement, vec![]).is_ok());
    assert!(sign_claim(&x, 0, None, &claim, vec![]).is_ok());
    // And the duplicate-seat refusal (the existing sign-time check):
    let dup = GenesisAgreement {
        players: [player_id(&x), player_id(&x)],
        ..agreement.clone()
    };
    assert!(sign_genesis(&x, 0, None, &dup, vec![]).is_err());
}
