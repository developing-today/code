//! `id keyset`: show and change an identity's keyset (#2380 packet 4).
//!
//! Every subcommand opens the identity's keyset log first, which
//! migrates a single-key account on its first use (keyset ADR
//! Decision 5): a genesis signed by the existing key, with that key as
//! the admin key and the first device. The identity ID stays the key's
//! public key.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result, bail};
use ed25519_dalek::SigningKey;
use zeroize::Zeroizing;

use crate::cli::KeysetCommand;
use crate::{CLIENT_KEY_FILE, KEY_FILE};
use id_core::hex::hex_encode;
use id_directory::keyset::{AdminKeyState, KeysetEntry, KeysetState};
use id_directory::keyset_file::{CLIENT_KEYSET_FILE, KEYSET_FILE, KeysetFile};
use id_directory::restored_key::{
    self, CLIENT_RESTORED_KEY_FILE, RESTORED_KEY_FILE, RestoreStatus,
};
use id_store::store::load_or_create_keypair;

/// Run an `id keyset` subcommand against the local identity.
///
/// # Errors
///
/// Fails if the identity key or keyset log cannot be read or written,
/// or the keyset refuses the change.
pub async fn cmd_keyset(command: KeysetCommand) -> Result<()> {
    let client = match &command {
        KeysetCommand::Show { client }
        | KeysetCommand::Enrol { client, .. }
        | KeysetCommand::Revoke { client, .. }
        | KeysetCommand::Rotate { client, .. } => *client,
    };
    let key_path = if client { CLIENT_KEY_FILE } else { KEY_FILE };
    let keyset_path = if client {
        CLIENT_KEYSET_FILE
    } else {
        KEYSET_FILE
    };
    let secret = load_or_create_keypair(key_path).await?;
    let key = SigningKey::from_bytes(&secret.to_bytes());
    let (mut file, migrated) = KeysetFile::open_or_migrate(keyset_path, &key)?;
    if migrated {
        println!(
            "Created the keyset for this identity in {keyset_path} (its key is the admin key and first device)."
        );
    }
    match command {
        KeysetCommand::Show { .. } => {}
        KeysetCommand::Enrol {
            device_key_file,
            label,
            ..
        } => {
            let device = read_key_file(&device_key_file)?;
            let device_key = public_hex(&device);
            file.append(
                &key,
                KeysetEntry::DeviceEnrolled {
                    device_key: device_key.clone(),
                    label,
                },
                Some(&device),
            )?;
            println!("Enrolled device {device_key}.");
        }
        KeysetCommand::Revoke {
            device_key, reason, ..
        } => {
            file.append(
                &key,
                KeysetEntry::DeviceRevoked {
                    device_key: device_key.clone(),
                    reason,
                },
                None,
            )?;
            println!("Revoked device {device_key}.");
        }
        KeysetCommand::Rotate {
            new_admin_key_file, ..
        } => {
            let new_admin = read_key_file(&new_admin_key_file)?;
            let new_admin_key = public_hex(&new_admin);
            file.append(
                &key,
                KeysetEntry::AdminKeyRotated {
                    new_admin_key: new_admin_key.clone(),
                },
                Some(&new_admin),
            )?;
            println!("Rotated the admin key to {new_admin_key}.");
        }
    }
    let state = file.state()?;
    print!("{}", render_state(&state));
    let marker = if client {
        CLIENT_RESTORED_KEY_FILE
    } else {
        RESTORED_KEY_FILE
    };
    match restored_key::settle(marker, Some(&state))? {
        RestoreStatus::NotRestored => {}
        RestoreStatus::RotationPending(public) => println!("{}", restored_key::warning(&public)),
        RestoreStatus::Retired(public) => {
            println!(
                "The restored key {public} is no longer an active admin key; its restore flag is cleared."
            );
        }
    }
    Ok(())
}

fn public_hex(key: &SigningKey) -> String {
    hex_encode(&key.verifying_key().to_bytes())
}

/// Reads a raw 32-byte key file, the format `id` writes for its own keys.
fn read_key_file(path: &Path) -> Result<SigningKey> {
    let bytes = Zeroizing::new(
        std::fs::read(path)
            .with_context(|| format!("could not read key file {}", path.display()))?,
    );
    let Ok(bytes) = <[u8; 32]>::try_from(bytes.as_slice()) else {
        bail!("{} is not a 32-byte key file", path.display());
    };
    Ok(SigningKey::from_bytes(&bytes))
}

/// The keyset as `id keyset` prints it.
#[must_use]
pub fn render_state(state: &KeysetState) -> String {
    let mut out = format!("identity: {}\nadmin keys:\n", state.identity_id);
    for (key, admin) in &state.admin_keys {
        let status = match admin {
            AdminKeyState::Active => "active",
            AdminKeyState::Rotated => "rotated away",
            AdminKeyState::Revoked => "revoked",
        };
        let _ = writeln!(out, "  {key}  {status}");
    }
    out.push_str("devices:\n");
    for (key, device) in &state.devices {
        let status = if device.revoked { "revoked" } else { "active" };
        let _ = writeln!(out, "  {key}  {status}  {}", device.label);
    }
    let _ = writeln!(out, "records: {}", state.records_folded);
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn render_lists_admins_and_devices_with_status() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(KEYSET_FILE);
        let owner = SigningKey::from_bytes(&[1; 32]);
        let laptop = SigningKey::from_bytes(&[2; 32]);
        let (mut file, _) = KeysetFile::open_or_migrate(&path, &owner).unwrap();
        file.append(
            &owner,
            KeysetEntry::DeviceEnrolled {
                device_key: public_hex(&laptop),
                label: "laptop".to_owned(),
            },
            Some(&laptop),
        )
        .unwrap();
        file.append(
            &owner,
            KeysetEntry::DeviceRevoked {
                device_key: public_hex(&laptop),
                reason: "lost".to_owned(),
            },
            None,
        )
        .unwrap();
        let text = render_state(&file.state().unwrap());
        assert!(text.starts_with(&format!("identity: {}\n", public_hex(&owner))));
        assert!(text.contains(&format!("  {}  active\n", public_hex(&owner))));
        assert!(text.contains(&format!("  {}  revoked  laptop\n", public_hex(&laptop))));
        assert!(text.contains("original device"));
        assert!(text.ends_with("records: 3\n"));
    }

    #[test]
    fn key_files_must_be_32_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let short = dir.path().join("short");
        std::fs::write(&short, [7u8; 31]).unwrap();
        assert!(read_key_file(&short).is_err());
        let good = dir.path().join("good");
        std::fs::write(&good, [7u8; 32]).unwrap();
        assert_eq!(
            read_key_file(&good).unwrap().to_bytes(),
            SigningKey::from_bytes(&[7; 32]).to_bytes()
        );
    }
}
