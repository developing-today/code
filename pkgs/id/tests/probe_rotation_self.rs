// Probe (creator-b, this run): does sign_rotation refuse a routine
// rotation whose successor is the signing key itself? The fold can
// never count it: a routine transition counts only when its author
// is the current generation key (fold filter 2), and a transition
// whose successor is the key it replaces never counts (fold filter
// 3, issuer_rotation.rs fold_rotation_chain). successor == author
// makes both true at once, in every chain state -- yet neither
// check_rotation (no author context) nor validate (no such rule)
// nor sign_rotation (has the key, never checks) refuses it.
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
fn probe_sign_rotation_self_successor() {
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
    assert!(self_rot.validate(&issuer, None).is_ok(), "validate passes the self-rotation");
    let signed = sign_rotation(&gen0, 0, None, &self_rot, vec![]);
    println!("sign_rotation(self-successor routine): ok={}", signed.is_ok());
    let ev = signed.unwrap();
    let folded = fold_rotation_chain(
        &issuer,
        &currency(),
        None,
        &[(author_id(&ev), ev.id, self_rot.clone())],
    );
    println!(
        "self-rotation fold: keys={} transitions={} active_generation={}",
        folded.keys.len(),
        folded.transitions.len(),
        folded.active_generation()
    );
    // Dead on arrival: the fold never advances past generation 0.
    assert_eq!(folded.keys.len(), 1);
    assert!(folded.transitions.is_empty());
}
