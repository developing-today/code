// Regression test (salvaged from closed archive PR #7534, task 14212):
// a routine rotation whose successor is the signing key itself can
// never count in the fold (fold filters in issuer_rotation.rs require
// the author to be the current generation key and the successor to
// differ from the key it replaces). The probe in #7534 documented the
// gap when neither validate nor sign_rotation refused it; #6451 fixed
// both. These tests pin the fixed behavior: validate refuses the
// self-rotation and sign_rotation returns
// RotationError::RoutineSuccessorIsAuthor, while a normal rotation
// still signs and folds.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use id::currency::{author_id, issuer_id};
use id::issuer_rotation::{IssuerRotation, RotationMode, fold_rotation_chain, sign_rotation};
use p2panda_core::{Hash, SigningKey};

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn currency() -> Hash {
    Hash::digest([42])
}

#[test]
fn sign_rotation_refuses_self_successor() {
    let gen0 = key(7);
    let gen1 = key(9);
    let issuer = issuer_id(&gen0);

    // Control: routine rotation gen0 -> gen1, proven by gen1.
    let mut control = IssuerRotation::unsigned(
        RotationMode::Routine,
        issuer,
        currency(),
        0,
        None,
        issuer_id(&gen1),
        None,
    );
    control.prove_successor(&gen1).unwrap();
    let ev = sign_rotation(&gen0, 0, None, &control, vec![]).unwrap();
    let folded = fold_rotation_chain(
        &issuer,
        &currency(),
        None,
        &[(author_id(&ev), ev.id, control.clone())],
    );
    println!("control: keys={} transitions={}", folded.keys.len(), folded.transitions.len());
    assert_eq!(folded.keys.len(), 2);
    assert_eq!(folded.transitions.len(), 1);

    // Case: routine rotation gen0 -> gen0 (successor is the signer
    // itself), proven by gen0 -- every body-local check passes.
    let mut self_rot = IssuerRotation::unsigned(
        RotationMode::Routine,
        issuer,
        currency(),
        0,
        None,
        issuer_id(&gen0),
        None,
    );
    self_rot.prove_successor(&gen0).unwrap();
    assert!(
        self_rot.validate(&issuer, None).is_err(),
        "validate must refuse the self-rotation"
    );
    let signed = sign_rotation(&gen0, 0, None, &self_rot, vec![]);
    assert!(signed.is_err(), "sign_rotation must refuse the self-rotation");
}
