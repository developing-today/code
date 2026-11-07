#![allow(clippy::unwrap_used)]
use p2panda_core::{Hash, SigningKey};
use id::attestation::{GenesisAgreement, MatchState, ResultClaim, compute_state, player_id};

fn key(seed: u8) -> SigningKey { SigningKey::from_bytes(&[seed; 32]) }
fn hex(bytes: &[u8]) -> String { bytes.iter().map(|b| format!("{b:02x}")).collect() }

#[test]
fn duplicate_claim_differing_only_in_frontiers() {
    let x = key(7); let o = key(9);
    let agreement = GenesisAgreement { game_id: "tic-tac-toe".to_owned(), logic_hash: Hash::digest(b"logic"), players: [player_id(&x), player_id(&o)], witness: None, objection_window_secs: 3600 };
    let genesis = vec![(player_id(&x), agreement.clone()), (player_id(&o), agreement.clone())];
    let ah = agreement.agreement_hash().unwrap();
    let fsh = Hash::digest(b"final");
    let cx = ResultClaim { agreement: ah, outcome: hex(&player_id(&x)), final_state_hash: fsh, frontiers: vec![Hash::digest(b"g1")] };
    let mut cx2 = cx.clone(); cx2.frontiers = vec![Hash::digest(b"g1"), Hash::digest(b"g2")];
    let co = ResultClaim { agreement: ah, outcome: hex(&player_id(&x)), final_state_hash: fsh, frontiers: vec![Hash::digest(b"g2")] };
    // X claims twice, identical outcome+state, frontiers differ (second claim after seeing more); O concurs.
    let claims = vec![(player_id(&x), cx), (player_id(&x), cx2), (player_id(&o), co)];
    let state = compute_state(&agreement, &genesis, &claims);
    println!("STATE={state:?}");
    assert_eq!(state, MatchState::Agreed, "doc rule: claims concur on outcome and final-state hash");
}
