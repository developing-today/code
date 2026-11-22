//! The restore flag on an identity key (#2381, "restore forces
//! rotation").
//!
//! A key brought back from a backup (an encrypted envelope or a seed
//! phrase) may be one a thief also holds, since the backup is exactly
//! what a thief would take. So `id key import` and `id key import-seed`
//! leave a marker naming the restored key's public key, and the key
//! stays flagged until a keyset rotation retires it: until the
//! identity's keyset no longer lists it as an active admin key. The
//! flag warns; it does not stop the key from working, because a
//! restore is also how an owner gets back in.

use std::io::ErrorKind;
use std::path::Path;

use anyhow::{Context, Result};

use crate::keyset::{AdminKeyState, KeysetState};
use crate::keyset_file::KeysetFile;

/// The server identity's restore marker, beside [`id_core::KEY_FILE`].
pub const RESTORED_KEY_FILE: &str = ".id-key-restored";
/// The client identity's restore marker, beside
/// [`id_core::CLIENT_KEY_FILE`].
pub const CLIENT_RESTORED_KEY_FILE: &str = ".id-key-client-restored";

/// Where a restored key stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestoreStatus {
    /// No restore marker: the key was not restored from a backup here.
    NotRestored,
    /// The key with this public key (hex) was restored and is still an
    /// active admin key, so it still needs rotating.
    RotationPending(String),
    /// The restored key (hex) has been rotated away or revoked; the
    /// marker is gone.
    Retired(String),
}

/// Records that the key with public key `public_key` (hex) was restored
/// from a backup, replacing any earlier marker.
///
/// The marker is written durably (#12420): the new contents go to a
/// sibling temp file that is synced before a rename replaces the
/// marker, and the directory is synced after. A crash or a failed
/// write leaves the earlier complete marker (or none), never an empty
/// or partial one that would silence the rotation warning.
///
/// # Errors
///
/// Fails if the marker cannot be written.
pub fn mark_restored(marker: impl AsRef<Path>, public_key: &str) -> Result<()> {
    let marker = marker.as_ref();
    let tmp = marker_tmp(marker);
    let result = id_core::durable_write::write_durable(
        marker,
        &tmp,
        format!("{public_key}\n").as_bytes(),
    );
    if let Err(error) = &result {
        if error.stage() == id_core::durable_write::WriteStage::BeforeRename {
            let _ = std::fs::remove_file(&tmp);
        }
    }
    result.with_context(|| format!("could not write restore marker {}", marker.display()))
}

/// The sibling temp file a marker is staged in before it replaces the
/// marker.
fn marker_tmp(marker: &Path) -> std::path::PathBuf {
    let mut name = marker.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    marker.with_file_name(name)
}

/// The public key (hex) a restore marker names, if there is one.
///
/// # Errors
///
/// Fails if the marker exists but cannot be read.
pub fn restored_key(marker: impl AsRef<Path>) -> Result<Option<String>> {
    let marker = marker.as_ref();
    match std::fs::read_to_string(marker) {
        Ok(text) => Ok(Some(text.trim().to_owned()).filter(|key| !key.is_empty())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error)
            .with_context(|| format!("could not read restore marker {}", marker.display())),
    }
}

/// Whether `restored` still needs rotating under `keyset`: with no
/// keyset the key is the identity's only admin key, so it does; with
/// one, it does while the fold lists it as an active admin key.
#[must_use]
pub fn rotation_pending(keyset: Option<&KeysetState>, restored: &str) -> bool {
    keyset.is_none_or(|state| matches!(state.admin_keys.get(restored), Some(AdminKeyState::Active)))
}

/// Settles the restore flag against `keyset`, removing the marker once
/// a rotation has retired the restored key.
///
/// # Errors
///
/// Fails if the marker cannot be read or removed.
pub fn settle(marker: impl AsRef<Path>, keyset: Option<&KeysetState>) -> Result<RestoreStatus> {
    let marker = marker.as_ref();
    let Some(restored) = restored_key(marker)? else {
        return Ok(RestoreStatus::NotRestored);
    };
    if rotation_pending(keyset, &restored) {
        return Ok(RestoreStatus::RotationPending(restored));
    }
    std::fs::remove_file(marker)
        .with_context(|| format!("could not remove restore marker {}", marker.display()))?;
    Ok(RestoreStatus::Retired(restored))
}

/// Reads the restore flag for a node without changing anything: the
/// restored key (hex) when it is still pending under the keyset log at
/// `keyset_path`, which may not exist yet.
///
/// # Errors
///
/// Fails if the marker or the keyset log cannot be read, or the log
/// holds a record that does not verify.
pub fn pending(marker: impl AsRef<Path>, keyset_path: impl AsRef<Path>) -> Result<Option<String>> {
    let Some(restored) = restored_key(marker)? else {
        return Ok(None);
    };
    let state = match KeysetFile::open_existing(keyset_path.as_ref(), &restored)? {
        Some(file) => Some(file.state()?),
        None => None,
    };
    Ok(rotation_pending(state.as_ref(), &restored).then_some(restored))
}

/// The warning a flagged key prints.
#[must_use]
pub fn warning(public_key: &str) -> String {
    format!(
        "This identity key ({public_key}) was restored from a backup, and a restore may \
         follow theft. Rotate it with `id keyset rotate <new-admin-key-file>`; it stays \
         flagged until then."
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::keyset::KeysetEntry;
    use ed25519_dalek::SigningKey;
    use id_core::hex::hex_encode;

    fn public_hex(key: &SigningKey) -> String {
        hex_encode(&key.verifying_key().to_bytes())
    }

    #[test]
    fn marking_again_replaces_the_marker_whole() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join(RESTORED_KEY_FILE);
        let first = public_hex(&SigningKey::from_bytes(&[7; 32]));
        let second = public_hex(&SigningKey::from_bytes(&[8; 32]));
        mark_restored(&marker, &first).unwrap();
        mark_restored(&marker, &second).unwrap();
        assert_eq!(restored_key(&marker).unwrap(), Some(second));
        assert!(!marker_tmp(&marker).exists(), "no temp file left behind");
    }

    #[test]
    fn a_failed_marker_write_keeps_the_prior_marker() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join(CLIENT_RESTORED_KEY_FILE);
        let prior = public_hex(&SigningKey::from_bytes(&[9; 32]));
        mark_restored(&marker, &prior).unwrap();
        // A directory where the temp file goes makes the write fail
        // before the rename.
        std::fs::create_dir(marker_tmp(&marker)).unwrap();
        let next = public_hex(&SigningKey::from_bytes(&[10; 32]));
        let error = mark_restored(&marker, &next).unwrap_err();
        assert!(
            format!("{error:#}").contains(&marker.display().to_string()),
            "error names the marker: {error:#}"
        );
        assert_eq!(restored_key(&marker).unwrap(), Some(prior));
    }

    #[test]
    fn a_failed_first_write_leaves_no_marker() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join(RESTORED_KEY_FILE);
        std::fs::create_dir(marker_tmp(&marker)).unwrap();
        let key = public_hex(&SigningKey::from_bytes(&[11; 32]));
        assert!(mark_restored(&marker, &key).is_err());
        assert!(!marker.exists());
        assert_eq!(restored_key(&marker).unwrap(), None);
    }

    #[test]
    fn no_marker_means_not_restored() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join(RESTORED_KEY_FILE);
        assert_eq!(settle(&marker, None).unwrap(), RestoreStatus::NotRestored);
        assert_eq!(
            pending(&marker, dir.path().join("keyset.jsonl")).unwrap(),
            None
        );
    }

    #[test]
    fn a_restored_key_stays_flagged_until_a_rotation_retires_it() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join(RESTORED_KEY_FILE);
        let keyset_path = dir.path().join("keyset.jsonl");
        let restored = SigningKey::from_bytes(&[5; 32]);
        let fresh = SigningKey::from_bytes(&[6; 32]);
        mark_restored(&marker, &public_hex(&restored)).unwrap();

        // No keyset yet: the restored key is the only admin key.
        assert_eq!(
            pending(&marker, &keyset_path).unwrap(),
            Some(public_hex(&restored))
        );

        // Migrating alone retires nothing.
        let (mut file, _) = KeysetFile::open_or_migrate(&keyset_path, &restored).unwrap();
        assert_eq!(
            settle(&marker, Some(&file.state().unwrap())).unwrap(),
            RestoreStatus::RotationPending(public_hex(&restored))
        );
        assert!(marker.exists());

        file.append(
            &restored,
            KeysetEntry::AdminKeyRotated {
                new_admin_key: public_hex(&fresh),
            },
            Some(&fresh),
        )
        .unwrap();
        assert_eq!(pending(&marker, &keyset_path).unwrap(), None);
        assert_eq!(
            settle(&marker, Some(&file.state().unwrap())).unwrap(),
            RestoreStatus::Retired(public_hex(&restored))
        );
        assert!(!marker.exists());
        assert_eq!(settle(&marker, None).unwrap(), RestoreStatus::NotRestored);
    }

    #[test]
    fn a_keyset_for_another_identity_does_not_clear_the_flag() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join(RESTORED_KEY_FILE);
        let keyset_path = dir.path().join("keyset.jsonl");
        let restored = SigningKey::from_bytes(&[7; 32]);
        let other = SigningKey::from_bytes(&[8; 32]);
        KeysetFile::open_or_migrate(&keyset_path, &other).unwrap();
        mark_restored(&marker, &public_hex(&restored)).unwrap();
        assert_eq!(
            pending(&marker, &keyset_path).unwrap(),
            Some(public_hex(&restored))
        );
    }

    #[test]
    fn open_existing_never_writes() {
        let dir = tempfile::tempdir().unwrap();
        let keyset_path = dir.path().join("keyset.jsonl");
        let key = SigningKey::from_bytes(&[9; 32]);
        assert!(
            KeysetFile::open_existing(&keyset_path, &public_hex(&key))
                .unwrap()
                .is_none()
        );
        assert!(!keyset_path.exists());
    }
}
