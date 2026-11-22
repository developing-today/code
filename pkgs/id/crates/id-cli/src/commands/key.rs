//! Local export and import commands for identity keys.

use std::fs::File;
use std::io::Write as _;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use zeroize::Zeroizing;

use crate::cli::KeyCommand;
use crate::{CLIENT_KEY_FILE, KEY_FILE};
use id_core::hex::hex_encode;
use id_core::key_envelope;
use id_core::key_envelope::MIN_PASSPHRASE_BYTES;
use id_core::seed_phrase;
use id_directory::restored_key::{self, CLIENT_RESTORED_KEY_FILE, RESTORED_KEY_FILE};
use id_store::store::load_or_create_keypair;

/// Run a local-only identity-key export or import operation.
pub async fn cmd_key(command: KeyCommand) -> Result<()> {
    match command {
        KeyCommand::Export { output, client } => export_key(&output, client).await,
        KeyCommand::Import {
            input,
            client,
            replace,
        } => import_key(&input, client, replace).await,
        KeyCommand::ExportSeed { client } => export_seed(client).await,
        KeyCommand::ImportSeed { client, replace } => import_seed(client, replace).await,
    }
}

async fn export_key(output: &Path, client: bool) -> Result<()> {
    let key_path = if client { CLIENT_KEY_FILE } else { KEY_FILE };
    let key = load_or_create_keypair(key_path).await?;
    let key_bytes = Zeroizing::new(key.to_bytes());

    let passphrase = Zeroizing::new(rpassword::prompt_password("Export passphrase: ")?);
    if passphrase.len() < MIN_PASSPHRASE_BYTES {
        bail!("passphrase must be at least {MIN_PASSPHRASE_BYTES} bytes");
    }
    let confirmation = Zeroizing::new(rpassword::prompt_password("Confirm passphrase: ")?);
    if passphrase.as_bytes() != confirmation.as_bytes() {
        bail!("passphrases do not match");
    }

    let envelope = key_envelope::encrypt(passphrase.as_bytes(), &key_bytes)?;
    write_new_private_file(output, &envelope)
        .with_context(|| format!("could not create key envelope {}", output.display()))?;
    println!("Exported identity key envelope to {}", output.display());
    Ok(())
}

async fn import_key(input: &Path, client: bool, replace: bool) -> Result<()> {
    let envelope = std::fs::read(input)
        .with_context(|| format!("could not read key envelope {}", input.display()))?;
    let passphrase = Zeroizing::new(rpassword::prompt_password("Import passphrase: ")?);
    let key_bytes = Zeroizing::new(key_envelope::decrypt(passphrase.as_bytes(), &envelope)?);

    let target = PathBuf::from(if client { CLIENT_KEY_FILE } else { KEY_FILE });
    let marker = if client {
        CLIENT_RESTORED_KEY_FILE
    } else {
        RESTORED_KEY_FILE
    };
    import_restored_key(&target, marker, &key_bytes, replace)?;
    println!("Imported identity key to {}", target.display());
    Ok(())
}

async fn export_seed(client: bool) -> Result<()> {
    let key_path = if client { CLIENT_KEY_FILE } else { KEY_FILE };
    let key = load_or_create_keypair(key_path).await?;
    let key_bytes = Zeroizing::new(key.to_bytes());
    let phrase = seed_phrase::encode_key(&key_bytes)?;
    println!("Seed phrase for this identity — write it down and keep it offline:");
    println!();
    println!("{phrase}");
    println!();
    println!("Anyone with this phrase can restore (and impersonate) this identity.");
    println!("It is shown once and is not stored anywhere by this command.");
    Ok(())
}

async fn import_seed(client: bool, replace: bool) -> Result<()> {
    let phrase = Zeroizing::new(rpassword::prompt_password("Seed phrase: ")?);
    let key_bytes = seed_phrase::decode_key(&phrase)?;

    let target = PathBuf::from(if client { CLIENT_KEY_FILE } else { KEY_FILE });
    let marker = if client {
        CLIENT_RESTORED_KEY_FILE
    } else {
        RESTORED_KEY_FILE
    };
    import_restored_key(&target, marker, &key_bytes, replace)?;
    println!(
        "Imported identity key from seed phrase to {}",
        target.display()
    );
    Ok(())
}

/// Records the restore before installing the key, so a failed marker write
/// cannot leave an installed key without rotation-warning state.
fn import_restored_key(
    target: &Path,
    marker: impl AsRef<Path>,
    key_bytes: &[u8; 32],
    replace: bool,
) -> Result<()> {
    import_restored_key_with_hook(target, marker, key_bytes, replace, || {})
}

fn import_restored_key_with_hook(
    target: &Path,
    marker: impl AsRef<Path>,
    key_bytes: &[u8; 32],
    replace: bool,
    after_marker_snapshot: impl FnOnce(),
) -> Result<()> {
    let _lock = lock_restored_key(marker.as_ref())?;
    let public = hex_encode(
        &ed25519_dalek::SigningKey::from_bytes(key_bytes)
            .verifying_key()
            .to_bytes(),
    );
    let previous_marker = match std::fs::read(&marker) {
        Ok(contents) => Some(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "could not read restore marker {}",
                    marker.as_ref().display()
                )
            });
        }
    };
    after_marker_snapshot();
    restored_key::mark_restored(&marker, &public)?;
    let install = if replace {
        replace_private_file(target, key_bytes)
    } else {
        write_new_private_file(target, key_bytes).with_context(|| {
            format!(
                "could not create identity key {}; use --replace to overwrite it",
                target.display()
            )
        })
    };
    if let Err(error) = install {
        let rollback = match previous_marker {
            Some(contents) => {
                let previous = String::from_utf8_lossy(&contents);
                restored_key::mark_restored(&marker, previous.trim())
            }
            None => match std::fs::remove_file(&marker) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error).with_context(|| {
                    format!(
                        "could not remove restore marker {}",
                        marker.as_ref().display()
                    )
                }),
            },
        };
        if let Err(rollback_error) = rollback {
            return Err(error).context(format!(
                "could not restore previous restore marker: {rollback_error:#}"
            ));
        }
        return Err(error);
    }
    println!("{}", restored_key::warning(&public));
    Ok(())
}

/// Locks one identity's restore transaction across processes. The lock file is
/// intentionally persistent: unlinking it could let waiters lock different inodes.
fn lock_restored_key(marker: &Path) -> Result<File> {
    let mut lock_name = marker.file_name().unwrap_or_default().to_os_string();
    lock_name.push(".lock");
    let lock_path = marker.with_file_name(lock_name);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("could not open restore lock {}", lock_path.display()))?;
    // SAFETY: flock only uses this valid, open file descriptor; the File remains
    // alive for the full duration of the guarded import transaction.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    if result == -1 {
        return Err(std::io::Error::last_os_error()).with_context(|| {
            format!("could not lock restore transaction {}", lock_path.display())
        });
    }
    Ok(file)
}

fn write_new_private_file(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::Builder::new()
        .prefix(".id-key-write-")
        .tempfile_in(parent)
        .with_context(|| format!("could not prepare private file in {}", parent.display()))?;
    temporary.write_all(contents)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(path)
        .map_err(|error| anyhow::anyhow!("could not create private file: {}", error.error))?;
    Ok(())
}

fn replace_private_file(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::Builder::new()
        .prefix(".id-key-import-")
        .tempfile_in(parent)
        .with_context(|| format!("could not prepare identity key in {}", parent.display()))?;
    temporary.write_all(contents)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| {
        anyhow::anyhow!("could not atomically replace identity key: {}", error.error)
    })?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use iroh_base::SecretKey;
    use tempfile::TempDir;

    #[test]
    fn concurrent_no_clobber_imports_serialize_marker_snapshot_and_install() {
        use std::sync::mpsc;
        use std::thread;
        use std::time::Duration;

        let temp = TempDir::new().unwrap();
        let target = temp.path().join(".iroh-key");
        let marker = temp.path().join(RESTORED_KEY_FILE);
        let keys = [
            SecretKey::from([11; 32]).to_bytes(),
            SecretKey::from([12; 32]).to_bytes(),
        ];
        let (snapshot_tx, snapshot_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let first_key = keys[0];
        let first_target = target.clone();
        let first_marker = marker.clone();
        let first = thread::spawn(move || {
            import_restored_key_with_hook(&first_target, &first_marker, &first_key, false, || {
                snapshot_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            })
        });
        snapshot_rx.recv().unwrap();

        let (second_started_tx, second_started_rx) = mpsc::channel();
        let (second_snapshot_tx, second_snapshot_rx) = mpsc::channel();
        let second_target = target.clone();
        let second_marker = marker.clone();
        let second_key = keys[1];
        let second = thread::spawn(move || {
            second_started_tx.send(()).unwrap();
            import_restored_key_with_hook(
                &second_target,
                &second_marker,
                &second_key,
                false,
                || second_snapshot_tx.send(()).unwrap(),
            )
        });
        second_started_rx.recv().unwrap();

        // The first transaction has snapshotted the absent marker and is held
        // there. The second must not pass its own snapshot until the first
        // publishes the marker and installs its key.
        let second_passed_snapshot = second_snapshot_rx
            .recv_timeout(Duration::from_millis(100))
            .is_ok();
        release_tx.send(()).unwrap();
        first.join().unwrap().unwrap();
        assert!(second.join().unwrap().is_err());
        assert!(
            !second_passed_snapshot,
            "second import bypassed the transaction lock"
        );

        let installed = std::fs::read(&target).unwrap();
        let installed: [u8; 32] = installed.try_into().unwrap();
        let expected_public = hex_encode(
            &ed25519_dalek::SigningKey::from_bytes(&installed)
                .verifying_key()
                .to_bytes(),
        );
        assert_eq!(
            restored_key::restored_key(&marker).unwrap().as_deref(),
            Some(expected_public.as_str())
        );
    }

    #[test]
    fn imports_without_clobbering_and_replaces_only_when_requested() {
        let temp = TempDir::new().unwrap();
        let target = temp.path().join(".iroh-key");
        let first = SecretKey::from([1; 32]).to_bytes();
        let second = SecretKey::from([2; 32]).to_bytes();

        write_new_private_file(&target, &first).unwrap();
        assert!(write_new_private_file(&target, &second).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), first);

        replace_private_file(&target, &second).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), second);
    }

    #[test]
    fn failed_restore_marker_write_does_not_install_the_imported_key() {
        let temp = TempDir::new().unwrap();
        let marker = temp.path().join(RESTORED_KEY_FILE);
        std::fs::create_dir(marker.with_file_name(format!("{RESTORED_KEY_FILE}.tmp"))).unwrap();

        let existing_path = temp.path().join("existing-key");
        let existing = SecretKey::from([1; 32]).to_bytes();
        let imported = SecretKey::from([2; 32]).to_bytes();
        std::fs::write(&existing_path, existing).unwrap();
        assert!(import_restored_key(&existing_path, &marker, &imported, true).is_err());
        assert_eq!(std::fs::read(&existing_path).unwrap(), existing);

        let new_path = temp.path().join("new-key");
        assert!(import_restored_key(&new_path, &marker, &imported, false).is_err());
        assert!(!new_path.exists());
    }

    #[test]
    fn failed_key_install_preserves_the_previous_restore_marker() {
        let temp = TempDir::new().unwrap();
        let target = temp.path().join("existing-key");
        let marker = temp.path().join(RESTORED_KEY_FILE);
        let existing = SecretKey::from([1; 32]).to_bytes();
        let imported = SecretKey::from([2; 32]).to_bytes();
        let existing_public = hex_encode(
            &ed25519_dalek::SigningKey::from_bytes(&existing)
                .verifying_key()
                .to_bytes(),
        );
        std::fs::write(&target, existing).unwrap();
        restored_key::mark_restored(&marker, &existing_public).unwrap();

        assert!(import_restored_key(&target, &marker, &imported, false).is_err());

        assert_eq!(std::fs::read(&target).unwrap(), existing);
        assert_eq!(
            restored_key::restored_key(&marker).unwrap().as_deref(),
            Some(existing_public.as_str())
        );

        std::fs::remove_file(&marker).unwrap();
        assert!(import_restored_key(&target, &marker, &imported, false).is_err());
        assert!(!marker.exists());
    }
}
