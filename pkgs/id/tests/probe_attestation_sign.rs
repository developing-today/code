// Probe (creator-b, this run): does sign_genesis / sign_claim refuse a
// signer who is not one of the agreement's two players? compute_state
// ignores non-player genesis/claims entirely, so such events are dead
// on arrival; sign_genesis already refuses the duplicate-seat viewer
// invariant at sign time (#2863 precedent).
#![allow(clippy::unwrap_used, clippy::expect_used)]
use p2panda_core::{Hash, SigningKey};
use id::attestation::{
    GenesisAgreement, MatchState, ResultClaim, author_id, compute_state, player_id, sign_claim,
    sign_genesis,
};
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
    println!("sign_genesis by non-player: {:?}", g_stranger.is_ok());
    // 2. A non-player signs a result claim answering the agreement.
    let claim = ResultClaim {
        agreement: agreement.agreement_hash().unwrap(),
        outcome: "draw".to_owned(),
        final_state_hash: Hash::digest(b"final"),
        frontiers: vec![],
    };
    let c_stranger = sign_claim(&stranger, 0, None, &claim, vec![]);
    println!("sign_claim by non-player: {:?}", c_stranger.is_ok());
    // 3. Viewer side: even with both players' genuine genesis events,
    //    the stranger's claim cannot move the state (still Proposed,
    //    since only one player -- none -- has claimed; the stranger
    //    claim is ignored). And a state built ONLY from stranger
    //    objects is Proposed.
    let gx = sign_genesis(&x, 0, None, &agreement, vec![]).unwrap();
    let go = sign_genesis(&o, 0, None, &agreement, vec![]).unwrap();
    let genesis = vec![
        (author_id(&gx), agreement.clone()),
        (author_id(&go), agreement.clone()),
        (author_id(&g_stranger.unwrap()), agreement.clone()),
    ];
    let claims = vec![(author_id(&c_stranger.unwrap()), claim.clone())];
    let state = compute_state(&agreement, &genesis, &claims);
    println!("compute_state with only a stranger claim: {:?}", state);
    assert_eq!(state, MatchState::Proposed);
    // Controls: player-signed genesis/claim work as documented.
    assert!(sign_genesis(&x, 0, None, &agreement, vec![]).is_ok());
    assert!(sign_claim(&x, 0, None, &claim, vec![]).is_ok());
    // And the duplicate-seat refusal (the existing sign-time check):
    let dup = GenesisAgreement { players: [player_id(&x), player_id(&x)], ..agreement.clone() };
    assert!(sign_genesis(&x, 0, None, &dup, vec![]).is_err());
}
