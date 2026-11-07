//! Probe (creator-a, 2026-11-05): does sign_claim refuse outcomes that
//! compute_state can never count? compute_state counts a claim only when
//! its outcome is "draw" or a seated player's lowercase 64-char hex id
//! (attestation.rs compute_state valid_outcome). A claim signed with any
//! other outcome string is ignored in every view.

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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn run_pair(outcome: &str) -> (bool, MatchState) {
    let x = key(0);
    let o = key(100);
    let agreement = GenesisAgreement {
        game_id: "tic-tac-toe".to_owned(),
        logic_hash: Hash::digest(b"logic"),
        players: [player_id(&x), player_id(&o)],
        witness: None,
        objection_window_secs: 3600,
    };
    let gx = sign_genesis(&x, 0, None, &agreement, vec![]).unwrap();
    let go = sign_genesis(&o, 0, None, &agreement, vec![]).unwrap();
    let hash = agreement.agreement_hash().unwrap();
    let mk = |outcome: &str| ResultClaim {
        agreement: hash,
        outcome: outcome.to_owned(),
        final_state_hash: Hash::digest(b"final"),
        frontiers: vec![gx.id, go.id],
    };
    let cx = sign_claim(&x, 1, Some(gx.id), &mk(outcome), vec![]);
    let co = sign_claim(&o, 1, Some(go.id), &mk(outcome), vec![]);
    let signed_ok = cx.is_ok() && co.is_ok();
    let genesis = vec![
        (author_id(&gx), agreement.clone()),
        (author_id(&go), agreement.clone()),
    ];
    let claims = vec![
        (player_id(&x), mk(outcome)),
        (player_id(&o), mk(outcome)),
    ];
    let state = compute_state(&agreement, &genesis, &claims);
    (signed_ok, state)
}

#[test]
fn probe_outcome_domain() {
    let x_hex = hex(&player_id(&key(0)));
    // Controls: outcomes inside compute_state's domain reach Agreed when
    // both players concur.
    let (ok, state) = run_pair("draw");
    println!("control draw: sign_ok={ok} state={state:?}");
    assert!(ok && state == MatchState::Agreed);
    let (ok, state) = run_pair(&x_hex);
    println!("control winner-hex: sign_ok={ok} state={state:?}");
    assert!(ok && state == MatchState::Agreed);

    // Malformed outcomes: outside the domain in shape alone (sign_claim
    // needs no agreement to know these can never count).
    let upper_hex = x_hex.to_uppercase();
    let short_hex = x_hex[..63].to_owned();
    for outcome in ["banana", "", "DRAW", "Draw", upper_hex.as_str(), short_hex.as_str(), "0x1234"] {
        let (ok, state) = run_pair(outcome);
        println!("malformed {outcome:?}: sign_ok={ok} state={state:?}");
        // The bug: signing succeeds, yet the claim is ignored in every
        // view — two concurring claims still compute Proposed.
        assert!(ok, "sign_claim refused {outcome:?}");
        assert_eq!(state, MatchState::Proposed, "outcome {outcome:?}");
    }
}
