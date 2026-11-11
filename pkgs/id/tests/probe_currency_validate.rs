//! Probe (creator-a, 2026-11-05): does fold_authorisation accept authorisations
//! whose terms the conversion layer can never honour?
use id::conversion::{ConvertError, convert_amount, fee_amount};
use id::currency::{
    AuthorisationState, AuthorisedCallers, RateAuthorisation, fold_authorisation, issuer_id,
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
fn probe_fee_bps_over_10000_folds_live_but_conversion_refuses() {
    let mut auth = base();
    auth.fee_bps = Some(10_001);
    assert!(
        folds_live(&auth),
        "authorisation with fee_bps=10001 folds Live"
    );
    assert_eq!(
        fee_amount(1_000, 10_001),
        Err(ConvertError::InvalidFeeBps { fee_bps: 10_001 }),
        "conversion layer refuses the very fee the folded authorisation carries"
    );
    // And the authorisation's own validate() accepts it too.
    assert!(auth.validate(&issuer_id(&lp_key())).is_ok());
}

#[test]
fn probe_min_above_max_folds_live_but_no_amount_is_convertible() {
    let mut auth = base();
    auth.min_amount = 10_000;
    auth.max_amount = 1;
    assert!(folds_live(&auth), "min>max authorisation folds Live");
    assert!(auth.validate(&issuer_id(&lp_key())).is_ok());
}

#[test]
fn probe_empty_time_window_folds_live() {
    let mut auth = base();
    auth.not_before = 2_000;
    auth.expires_at = 2_000; // expires_at is exclusive: no instant is valid
    assert!(folds_live(&auth), "not_before==expires_at folds Live");
    assert!(auth.validate(&issuer_id(&lp_key())).is_ok());
}

#[test]
fn probe_zero_rate_num_folds_live_but_every_conversion_is_zero_output() {
    let mut auth = base();
    auth.rate_num = 0;
    assert!(folds_live(&auth), "rate_num=0 authorisation folds Live");
    assert_eq!(
        convert_amount(1_000, 0, 10_000, 0, 0),
        Err(ConvertError::ZeroOutput),
        "every conversion under a zero rate rounds to zero and is refused"
    );
}

#[test]
fn probe_same_ledger_and_zero_max_also_fold_live() {
    let mut auth = base();
    auth.dest_ledger = auth.source_ledger.clone();
    auth.max_amount = 0;
    assert!(
        folds_live(&auth),
        "same-ledger, max=0 authorisation folds Live"
    );
}
