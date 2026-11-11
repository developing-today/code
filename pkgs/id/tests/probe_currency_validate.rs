//! Probe (creator-a, 2026-11-05): does fold_authorisation accept authorisations
//! whose terms the conversion layer can never honour?
//!
//! Flipped 2026-11-11 (task 14777, PR #5181, issue #4071): the probe was
//! salvaged onto main documenting the bug — every shape below folded Live
//! and validate() returned Ok. RateAuthorisation::validate now rejects
//! each shape with a named ValidateError and the fold filters it out, so
//! the probe asserts the fixed behaviour and stands as the regression
//! pin for #4071.
use id::conversion::{ConvertError, convert_amount, fee_amount};
use id::currency::{
    AuthorisationState, AuthorisedCallers, RateAuthorisation, ValidateError, fold_authorisation,
    issuer_id,
};
use p2panda_core::SigningKey;

fn lp_key() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}

fn base() -> RateAuthorisation {
    RateAuthorisation {
        authorisation_id: 7,
        version: 1,
        issuer_id: issuer_id(&lp_key()),
        source_ledger: "ABC".to_owned(),
        dest_ledger: "123".to_owned(),
        rate_num: 8123,
        rate_den: 10_000,
        min_amount: 1,
        max_amount: 10_000,
        total_cap: 1_000_000,
        authorised_callers: AuthorisedCallers::Any,
        not_before: 0,
        expires_at: 4_102_444_800,
        reversal_window_secs: None,
        fee_bps: None,
    }
}

fn folds_live(auth: &RateAuthorisation) -> bool {
    let author = issuer_id(&lp_key());
    matches!(
        fold_authorisation(7, &author, &[(author, auth.clone())], &[]),
        Some(AuthorisationState::Live(_))
    )
}

#[test]
fn probe_fee_bps_over_10000_is_rejected_and_conversion_refuses() {
    let mut auth = base();
    auth.fee_bps = Some(10_001);
    assert!(
        !folds_live(&auth),
        "authorisation with fee_bps=10001 must not fold Live"
    );
    assert_eq!(
        fee_amount(1_000, 10_001),
        Err(ConvertError::InvalidFeeBps { fee_bps: 10_001 }),
        "conversion layer refuses the very fee the authorisation carries"
    );
    assert_eq!(
        auth.validate(&issuer_id(&lp_key())),
        Err(ValidateError::FeeBpsTooHigh)
    );
}

#[test]
fn probe_min_above_max_is_rejected() {
    let mut auth = base();
    auth.min_amount = 10_000;
    auth.max_amount = 1;
    assert!(!folds_live(&auth), "min>max authorisation must not fold Live");
    assert_eq!(
        auth.validate(&issuer_id(&lp_key())),
        Err(ValidateError::MinAmountExceedsMaxAmount)
    );
}

#[test]
fn probe_empty_time_window_is_rejected() {
    let mut auth = base();
    auth.not_before = 2_000;
    auth.expires_at = 2_000; // expires_at is exclusive: no instant is valid
    assert!(
        !folds_live(&auth),
        "not_before==expires_at must not fold Live"
    );
    assert_eq!(
        auth.validate(&issuer_id(&lp_key())),
        Err(ValidateError::InvalidTimeWindow)
    );
}

#[test]
fn probe_zero_rate_num_is_rejected_and_every_conversion_is_zero_output() {
    let mut auth = base();
    auth.rate_num = 0;
    assert!(!folds_live(&auth), "rate_num=0 authorisation must not fold Live");
    assert_eq!(
        auth.validate(&issuer_id(&lp_key())),
        Err(ValidateError::ZeroRateNumerator)
    );
    assert_eq!(
        convert_amount(1_000, 0, 10_000, 0, 0),
        Err(ConvertError::ZeroOutput),
        "every conversion under a zero rate rounds to zero and is refused"
    );
}

#[test]
fn probe_same_ledger_and_zero_max_are_rejected() {
    let mut auth = base();
    auth.dest_ledger = auth.source_ledger.clone();
    auth.max_amount = 0;
    assert!(
        !folds_live(&auth),
        "same-ledger, max=0 authorisation must not fold Live"
    );
    // min_amount (1) > max_amount (0) is the first check that fires.
    assert_eq!(
        auth.validate(&issuer_id(&lp_key())),
        Err(ValidateError::MinAmountExceedsMaxAmount)
    );

    let mut same_ledger = base();
    same_ledger.dest_ledger = same_ledger.source_ledger.clone();
    assert!(!folds_live(&same_ledger));
    assert_eq!(
        same_ledger.validate(&issuer_id(&lp_key())),
        Err(ValidateError::SameSourceAndDestinationLedger)
    );
}
