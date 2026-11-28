//! Property tests for the #6424 signer-membership invariants of
//! `sign_genesis` / `sign_claim` (Codex P1 on PR #6559: the fixed
//! examples in attestation_signer_player.rs use three predetermined
//! keys; the testing doctrine requires property coverage for
//! meaningful crypto invariants).
//!
//! Oracle: the membership rule stated on the functions themselves —
//! a genesis or claim counts only when its author is one of the
//! agreement's two players (compute_state ignores every other
//! author), and a claim counts only against the agreement it names.
//! For ALL generated seat/outsider key triples and agreement
//! variants: either seated signer is accepted; an outsider is
//! refused with SignError::NotAPlayer; a claim answering a different
//! agreement is refused with SignError::AgreementMismatch even from
//! a seated signer.
//!
//! The properties use the default case count, which `PROPTEST_CASES`
//! raises. Failures are not written to `proptest-regressions/`
//! (failure_persistence is None, as in chain_properties.rs).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use p2panda_core::{Hash, SigningKey};

use id::attestation::{
    GenesisAgreement, ResultClaim, SignError, player_id, sign_claim, sign_genesis,
};
use proptest::prelude::*;

fn config() -> ProptestConfig {
    ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

/// The fixed-example key derivation: seed byte followed by seed+1.. —
/// distinct seeds give distinct keys (the first byte differs).
fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&std::array::from_fn(|i| {
        seed.wrapping_add(u8::try_from(i).unwrap_or(0))
    }))
}

fn agreement_for(x: &SigningKey, o: &SigningKey, variant: u8, window: u64) -> GenesisAgreement {
    GenesisAgreement {
        game_id: format!("game-{variant}"),
        logic_hash: Hash::digest(format!("logic-{variant}").as_bytes()),
        players: [player_id(x), player_id(o)],
        witness: None,
        objection_window_secs: window,
    }
}

fn claim_for(agreement: &GenesisAgreement, outcome_seed: u8) -> ResultClaim {
    ResultClaim {
        agreement: agreement.agreement_hash().unwrap(),
        outcome: "draw".to_owned(),
        final_state_hash: Hash::digest(&[outcome_seed]),
        frontiers: vec![],
    }
}

/// Three pairwise-distinct seed bytes.
fn distinct_seeds() -> impl Strategy<Value = (u8, u8, u8)> {
    (any::<u8>(), any::<u8>(), any::<u8>())
        .prop_filter("seeds must be pairwise distinct", |(a, b, c)| {
            a != b && b != c && a != c
        })
}

proptest! {
    #![proptest_config(config())]

    /// Either seated signer may sign the genesis; an outsider may not.
    #[test]
    fn genesis_signer_membership(
        (seat_a, seat_b, outsider) in distinct_seeds(),
        variant in any::<u8>(),
        window in any::<u64>(),
    ) {
        let x = key(seat_a);
        let o = key(seat_b);
        let stranger = key(outsider);
        // Distinct seeds give distinct player ids under this derivation;
        // assume it rather than assert, so a derivation change that
        // broke it would shrink to a visible counterexample seed set.
        prop_assume!(player_id(&x) != player_id(&o));
        prop_assume!(player_id(&stranger) != player_id(&x) && player_id(&stranger) != player_id(&o));
        let agreement = agreement_for(&x, &o, variant, window);

        prop_assert!(sign_genesis(&x, 0, None, &agreement, vec![]).is_ok());
        prop_assert!(sign_genesis(&o, 0, None, &agreement, vec![]).is_ok());
        let refused = sign_genesis(&stranger, 0, None, &agreement, vec![]);
        prop_assert!(
            matches!(refused, Err(SignError::NotAPlayer)),
            "outsider genesis must be NotAPlayer, got {refused:?}"
        );
    }

    /// Either seated signer may sign a claim answering the agreement;
    /// an outsider may not.
    #[test]
    fn claim_signer_membership(
        (seat_a, seat_b, outsider) in distinct_seeds(),
        variant in any::<u8>(),
        window in any::<u64>(),
        outcome_seed in any::<u8>(),
    ) {
        let x = key(seat_a);
        let o = key(seat_b);
        let stranger = key(outsider);
        prop_assume!(player_id(&x) != player_id(&o));
        prop_assume!(player_id(&stranger) != player_id(&x) && player_id(&stranger) != player_id(&o));
        let agreement = agreement_for(&x, &o, variant, window);
        let claim = claim_for(&agreement, outcome_seed);
        // Chain validity is not the invariant under test, but seq 1
        // needs a backlink: sign the genesis first, as the fixed
        // examples do, and link claims to it.
        let gx = sign_genesis(&x, 0, None, &agreement, vec![]).unwrap();

        prop_assert!(sign_claim(&x, 1, Some(gx.id), &agreement, &claim, vec![]).is_ok());
        prop_assert!(sign_claim(&o, 1, Some(gx.id), &agreement, &claim, vec![]).is_ok());
        let refused = sign_claim(&stranger, 1, Some(gx.id), &agreement, &claim, vec![]);
        prop_assert!(
            matches!(refused, Err(SignError::NotAPlayer)),
            "outsider claim must be NotAPlayer, got {refused:?}"
        );
    }

    /// A claim answering a DIFFERENT agreement is refused with
    /// AgreementMismatch even when a seated player signs it.
    #[test]
    fn claim_must_answer_the_given_agreement(
        (seat_a, seat_b, _outsider) in distinct_seeds(),
        variant in any::<u8>(),
        other_variant in any::<u8>(),
        window in any::<u64>(),
        outcome_seed in any::<u8>(),
    ) {
        prop_assume!(variant != other_variant);
        let x = key(seat_a);
        let o = key(seat_b);
        prop_assume!(player_id(&x) != player_id(&o));
        let agreement = agreement_for(&x, &o, variant, window);
        let other = agreement_for(&x, &o, other_variant, window);
        // The mismatch is in the agreement hash, not the seats.
        prop_assume!(agreement.agreement_hash().unwrap() != other.agreement_hash().unwrap());
        let foreign_claim = claim_for(&other, outcome_seed);
        let gx = sign_genesis(&x, 0, None, &agreement, vec![]).unwrap();

        for signer in [&x, &o] {
            let refused = sign_claim(signer, 1, Some(gx.id), &agreement, &foreign_claim, vec![]);
            prop_assert!(
                matches!(refused, Err(SignError::AgreementMismatch)),
                "foreign claim must be AgreementMismatch, got {refused:?}"
            );
        }
        // Control: the same claim signs fine against its own agreement.
        prop_assert!(sign_claim(&x, 1, Some(gx.id), &other, &foreign_claim, vec![]).is_ok());
    }
}
