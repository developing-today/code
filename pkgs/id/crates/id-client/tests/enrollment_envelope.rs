//! Slice-2 envelope regression tests (`#7938` child `#15842`).
//!
//! These tests pin the spike verdict, not Android behavior: the
//! per-API-level support line, the default-to-envelope decision, the
//! backup-exclusion list, and the secret zeroization guarantee. None
//! of them touches a Keystore, an emulator, or a real device.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use id_client::enrollment::{
    DOC_DERIVED_DIRECT_ED25519_API, SecretBuffer, StoragePosture, WrappingKeyLevel,
    direct_keystore_ed25519_supported, is_backup_excluded, recommended_posture,
};

#[test]
fn support_line_sits_at_doc_derived_api_33() {
    assert_eq!(DOC_DERIVED_DIRECT_ED25519_API, 33);
    assert!(!direct_keystore_ed25519_supported(21));
    assert!(!direct_keystore_ed25519_supported(32));
    assert!(direct_keystore_ed25519_supported(33));
    assert!(direct_keystore_ed25519_supported(34));
}

#[test]
fn default_posture_is_envelope_never_direct() {
    // Even where doc sources report direct Ed25519, the spike default
    // is the envelope until a real-device run promotes the device.
    assert_eq!(
        recommended_posture(33, true),
        StoragePosture::EnvelopedHardwareWrap(WrappingKeyLevel::StrongBox)
    );
    assert_eq!(
        recommended_posture(33, false),
        StoragePosture::EnvelopedHardwareWrap(WrappingKeyLevel::Tee)
    );
    assert_eq!(
        recommended_posture(21, false),
        StoragePosture::EnvelopedHardwareWrap(WrappingKeyLevel::Tee)
    );
}

#[test]
fn key_material_paths_are_backup_excluded() {
    for path in [
        "keys/id.keyenvelope",
        "keys/device.idkey",
        "backup/words.recoverycodes",
        "delegations/phone.delegation",
    ] {
        assert!(is_backup_excluded(path), "{path} must be backup-excluded");
    }
    assert!(!is_backup_excluded("journal/local.sqlite"));
    assert!(!is_backup_excluded("prefs/ui.json"));
}

#[test]
fn secret_buffer_zeroizes() {
    let mut buf = SecretBuffer::new(&[1_u8, 2, 3, 4]);
    assert_eq!(buf.expose(), &[1_u8, 2, 3, 4]);
    buf.zeroize();
    assert!(buf.is_zeroed());
}
