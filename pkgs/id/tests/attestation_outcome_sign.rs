//! #6441: sign_claim refuses outcomes compute_state can never count.
//!
//! compute_state counts a claim only when its outcome is "draw" or a
//! seated player's lowercase 64-char hex id. The shape half of that
//! domain is decidable from the claim alone, so sign_claim must refuse
//! every other outcome with SignError::InvalidOutcome before signing.

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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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

fn sign_with_outcome(outcome: &str) -> Result<(), SignError> {
    let (x, _o, agreement, mut claim) = setup();
    let genesis = sign_genesis(&x, 0, None, &agreement, vec![])?;
    claim.outcome = outcome.to_owned();
    claim.frontiers = vec![genesis.id];
    sign_claim(&x, 1, Some(genesis.id), &agreement, &claim, vec![]).map(|_| ())
}

#[test]
fn sign_claim_refuses_malformed_outcomes() {
    let x_hex = hex(&player_id(&key(0)));
    let upper_hex = x_hex.to_uppercase();
    let short_hex = x_hex[..63].to_owned();
    let long_hex = format!("{x_hex}0");
    for outcome in [
        "banana",
        "",
        "DRAW",
        "Draw",
        "draw ",
        upper_hex.as_str(),
        short_hex.as_str(),
        long_hex.as_str(),
        "0x1234",
    ] {
        let result = sign_with_outcome(outcome);
        assert!(
            matches!(result, Err(SignError::InvalidOutcome)),
            "outcome {outcome:?} must be refused with InvalidOutcome, got {result:?}"
        );
    }
}

#[test]
fn sign_claim_accepts_well_formed_outcomes() {
    let x_hex = hex(&player_id(&key(0)));
    // A third party's well-formed hex also signs: seated-ness is
    // compute_state's call, not sign_claim's (#6441 acceptance).
    let third_party_hex = hex(&player_id(&key(200)));
    for outcome in ["draw", x_hex.as_str(), third_party_hex.as_str()] {
        assert!(
            sign_with_outcome(outcome).is_ok(),
            "outcome {outcome:?} must sign Ok"
        );
    }
}

#[test]
fn controls_sign_ok_and_compute_agreed_end_to_end() {
    let (x, o, agreement, base) = setup();
    let gx = sign_genesis(&x, 0, None, &agreement, vec![]).unwrap();
    let go = sign_genesis(&o, 0, None, &agreement, vec![]).unwrap();
    let x_hex = hex(&player_id(&x));
    for outcome in ["draw", x_hex.as_str()] {
        let mut claim = base.clone();
        claim.outcome = outcome.to_owned();
        claim.frontiers = vec![gx.id, go.id];
        let cx = sign_claim(&x, 1, Some(gx.id), &agreement, &claim, vec![]).unwrap();
        let co = sign_claim(&o, 1, Some(go.id), &agreement, &claim, vec![]).unwrap();
        let _ = (cx, co);
        let genesis = vec![
            (author_id(&gx), agreement.clone()),
            (author_id(&go), agreement.clone()),
        ];
        let claims = vec![(player_id(&x), claim.clone()), (player_id(&o), claim)];
        assert_eq!(
            compute_state(&agreement, &genesis, &claims),
            MatchState::Agreed,
            "outcome {outcome:?}"
        );
    }
}
