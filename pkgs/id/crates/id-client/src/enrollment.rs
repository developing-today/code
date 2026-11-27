//! Slice-2 enrollment spike types (`#7938` child `#15842`).
//!
//! The `#5780` design (§2.1) forbids assuming every Android
//! Keystore/StrongBox combination can generate and use the ID Ed25519
//! identity directly. This module records the spike's storage decision
//! as checkable Rust: which API levels may attempt direct hardware
//! Ed25519, what the encrypted-key-envelope fallback looks like, which
//! paths must stay out of Android backup, and how secret bytes are
//! lifetime-bounded. It performs no Android API calls and handles no
//! real user keys — all secret material here is test-only.
//!
//! Provenance rule for this file: every API-level claim is
//! **doc-derived** (AOSP Keystore documentation and contemporaneous
//! reports place Keystore Ed25519 generation at API 33 / Android 13)
//! until a real-device or emulator run confirms it. Nothing here is
//! labelled emulator-proven or device-proven; see
//! `pkgs/id/android/SPIKE-02-identity-enrollment.md` for the matrix and
//! the outstanding on-device confirmation.

/// First API level where doc sources report direct Ed25519 generation
/// in Android Keystore (Android 13). Doc-derived, not device-proven:
/// the on-device confirmation is still owed (see the spike record).
pub const DOC_DERIVED_DIRECT_ED25519_API: u32 = 33;

/// Whether doc sources report direct Keystore Ed25519 generation at
/// `api_level`. This is the spike's support verdict in executable form;
/// on-device confirmation may narrow it (e.g. StrongBox absence on
/// specific devices) but must never widen it silently.
#[must_use]
pub const fn direct_keystore_ed25519_supported(api_level: u32) -> bool {
    api_level >= DOC_DERIVED_DIRECT_ED25519_API
}

/// Hardware level of the Keystore wrapping key used by the envelope
/// fallback. The level is always reported honestly: software backing
/// is never labelled StrongBox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrappingKeyLevel {
    /// StrongBox (dedicated secure element) generated the AES wrap key.
    StrongBox,
    /// Platform Keystore (TEE) generated the AES wrap key.
    Tee,
    /// No hardware backing was available; the wrap key is
    /// software-backed. An explicit setup choice, never a silent
    /// downgrade.
    Software,
}

/// Where the ID identity key lives on this device, per the §2.1
/// decision ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoragePosture {
    /// Direct hardware-backed non-exportable Ed25519 signing key —
    /// only where a real-device spike has verified algorithm + API
    /// level. Not selectable on doc evidence alone.
    DirectHardwareEd25519,
    /// Encrypted key envelope in app-private storage, AES wrap key
    /// from hardware (StrongBox preferred, TEE otherwise).
    EnvelopedHardwareWrap(WrappingKeyLevel),
    /// Envelope fallback with a software-backed wrap key, chosen
    /// explicitly at setup with the posture disclosed.
    EnvelopedSoftwareWrap,
}

/// Spike decision ladder (§2.1, rules 1–3) in executable form.
///
/// `strongbox_present` reports whether this device exposes StrongBox.
/// Levels below [`DOC_DERIVED_DIRECT_ED25519_API`] always take the
/// envelope fallback; levels at or above it take the envelope fallback
/// too until a real-device run promotes them to
/// [`StoragePosture::DirectHardwareEd25519`].
#[must_use]
pub const fn recommended_posture(api_level: u32, strongbox_present: bool) -> StoragePosture {
    let _ = api_level;
    if strongbox_present {
        StoragePosture::EnvelopedHardwareWrap(WrappingKeyLevel::StrongBox)
    } else {
        StoragePosture::EnvelopedHardwareWrap(WrappingKeyLevel::Tee)
    }
}

/// File-name suffixes that must be excluded from Android cloud,
/// device-to-device, and auto backup. Key envelopes, recovery
/// secrets, and delegation signing material are never backed up;
/// the local journal's backup/export choice is separate and never
/// implies key recovery.
pub const BACKUP_EXCLUDED_SUFFIXES: &[&str] =
    &[".keyenvelope", ".idkey", ".recoverycodes", ".delegation"];

/// True when `path` names backup-excluded key material.
#[must_use]
pub fn is_backup_excluded(path: &str) -> bool {
    BACKUP_EXCLUDED_SUFFIXES
        .iter()
        .any(|suffix| path.ends_with(suffix))
}

/// A short-lived plaintext secret buffer: zeroized on drop and on
/// explicit [`SecretBuffer::zeroize`].
///
/// Spike prototype for the §2.1 rule "decrypt only for signing, keep
/// plaintext lifetime short, and zeroize Rust-side secret buffers
/// where possible". The platform boundary exposes sign operations or
/// a short-lived handle to Rust — never a long-lived plaintext seed.
#[derive(Debug)]
pub struct SecretBuffer {
    bytes: Vec<u8>,
}

impl SecretBuffer {
    /// Copy `secret` into a lifetime-bounded buffer.
    #[must_use]
    pub fn new(secret: &[u8]) -> Self {
        Self {
            bytes: secret.to_vec(),
        }
    }

    /// Borrow the plaintext for one signing operation.
    #[must_use]
    pub fn expose(&self) -> &[u8] {
        &self.bytes
    }

    /// Zeroize in place; the buffer stays usable afterwards.
    pub fn zeroize(&mut self) {
        for byte in self.bytes.iter_mut() {
            *byte = 0;
        }
    }

    /// True when every byte is zero (test hook for the zeroization
    /// guarantee; not a security proof against compiler retention).
    #[must_use]
    pub fn is_zeroed(&self) -> bool {
        self.bytes.iter().all(|byte| *byte == 0)
    }
}

impl Drop for SecretBuffer {
    fn drop(&mut self) {
        self.zeroize();
    }
}
