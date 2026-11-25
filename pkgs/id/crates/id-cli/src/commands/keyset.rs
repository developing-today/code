//! `id keyset`: show and change an identity's keyset (#2380 packet 4,
//! plus the packet-2 pairing flow).
//!
//! Every subcommand except `enrol-accept` opens the identity's keyset
//! log first, which migrates a single-key account on its first use
//! (keyset ADR Decision 5): a genesis signed by the existing key, with
//! that key as the admin key and the first device. The identity ID
//! stays the key's public key. `enrol-accept` runs on the joining
//! device, which has no keyset yet, so it only reads the scanned offer
//! and the device's own key file.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{bail, Context, Result};
use ed25519_dalek::SigningKey;
use zeroize::Zeroizing;

use crate::cli::KeysetCommand;
use crate::{CLIENT_KEY_FILE, KEY_FILE};
use id_core::hex::hex_encode;
use id_directory::keyset::{AdminKeyState, KeysetEntry, KeysetState};
use id_directory::keyset_file::{KeysetFile, CLIENT_KEYSET_FILE, KEYSET_FILE};
use id_directory::keyset_pairing::{DeviceQr, EnrolAccept, EnrolOffer};
use id_directory::restored_key::{
    self, RestoreStatus, CLIENT_RESTORED_KEY_FILE, RESTORED_KEY_FILE,
};
use id_store::store::load_or_create_keypair;

/// Run an `id keyset` subcommand against the local identity.
///
/// # Errors
///
/// Fails if the identity key or keyset log cannot be read or written,
/// or the keyset refuses the change.
pub async fn cmd_keyset(command: KeysetCommand) -> Result<()> {
    let command = match command {
        KeysetCommand::Recovery { command } => {
            return super::keyset_recovery::cmd_recovery(command).await;
        }
        // The joining device has no keyset yet: answering needs only
        // the scanned offer and its own key file.
        KeysetCommand::EnrolAccept {
            offer,
            device_key_file,
        } => {
            let offer = EnrolOffer::parse(&offer)?;
            let device = read_key_file(&device_key_file)?;
            let accept = offer.accept(&device)?;
            println!("{}", accept.qr_payload()?);
            println!("Scan this accept on the admin device with `id keyset enrol-complete`.");
            return Ok(());
        }
        other => other,
    };
    let client = match &command {
        // Handled above; recovery opens keys and keysets as it needs.
        KeysetCommand::Recovery { .. } => false,
        KeysetCommand::Show { client }
        | KeysetCommand::Enrol { client, .. }
        | KeysetCommand::EnrolOffer { client, .. }
        | KeysetCommand::EnrolComplete { client, .. }
        | KeysetCommand::DeviceQr { client }
        | KeysetCommand::Revoke { client, .. }
        | KeysetCommand::Rotate { client, .. }
        | KeysetCommand::Guardians { client, .. }
        | KeysetCommand::Checkpoint { client } => *client,
        // Answered above, before any keyset is opened.
        KeysetCommand::EnrolAccept { .. } => false,
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
        KeysetCommand::EnrolOffer { label, .. } => {
            let offer = file.enrol_offer(&key, &label)?;
            let payload = offer.qr_payload()?;
            println!("{payload}");
            println!(
                "Show this as a QR code for the new device to scan; answer it there with `id keyset enrol-accept`."
            );
        }
        KeysetCommand::EnrolComplete { accept, .. } => {
            let accept = EnrolAccept::parse(&accept)?;
            let record = file.complete_enrol(&key, &accept)?;
            println!(
                "Enrolled device {} as {} (record {}).",
                accept.device_key, accept.offer.label, record.hash
            );
        }
        KeysetCommand::DeviceQr { .. } => {
            println!("{}", DeviceQr::for_key(&key).qr_payload());
            println!("Show this to an admin device to scan with `id keyset revoke`.");
        }
        KeysetCommand::Revoke {
            device_key, reason, ..
        } => {
            let device = parse_revoke_device(&device_key)?;
            let revoked_key = device.device_key.clone();
            file.revoke_scanned(&key, &device, &reason)?;
            println!("Revoked device {revoked_key}.");
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
        KeysetCommand::Guardians {
            mut guardians,
            threshold,
            ..
        } => {
            // The record wants them unique and sorted; the user need not.
            for guardian in &mut guardians {
                guardian.make_ascii_lowercase();
            }
            guardians.sort();
            guardians.dedup();
            file.append(
                &key,
                KeysetEntry::GuardianSetUpdated {
                    guardians,
                    threshold,
                },
                None,
            )?;
            println!("Updated the guardian set.");
        }
        // Answered before any keyset was opened; nothing more to do.
        KeysetCommand::Show { .. }
        | KeysetCommand::EnrolAccept { .. }
        | KeysetCommand::Recovery { .. } => {}
        KeysetCommand::Checkpoint { .. } => {
            let head = file.checkpoint(&key)?;
            println!("Checkpointed head {head}; it is final here.");
        }
    }
    let state = file.state()?;
    print!("{}", render_state(&state));
    println!("{}", finality_line(&file, &state.head_hash));
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

fn parse_revoke_device(text: &str) -> Result<DeviceQr> {
    if text.trim().starts_with("id:keyset-device?") {
        DeviceQr::parse(text)
    } else {
        Ok(DeviceQr {
            device_key: text.to_owned(),
        })
    }
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
    match &state.guardian_set {
        Some(set) => {
            let _ = writeln!(
                out,
                "guardians: {} of {} must approve a recovery",
                set.threshold,
                set.guardians.len()
            );
            for guardian in &set.guardians {
                let _ = writeln!(out, "  {guardian}");
            }
        }
        None => out.push_str("guardians: none\n"),
    }
    if let Some(pending) = &state.pending_guardian_set {
        let _ = writeln!(
            out,
            "pending guardian change: {} of {}, in force once the recovery delay has passed",
            pending.set.threshold,
            pending.set.guardians.len()
        );
        for guardian in &pending.set.guardians {
            let _ = writeln!(out, "  {guardian}");
        }
    }
    let _ = writeln!(out, "records: {}", state.records_folded);
    out
}

/// One line on how far the head is protected by checkpoint finality.
fn finality_line(file: &KeysetFile, head: &str) -> String {
    if file.final_heads().contains(head) {
        "head: final (checkpointed)".to_owned()
    } else if file.final_heads().is_empty() {
        "head: provisional (no checkpoint yet; `id keyset checkpoint` makes it final)".to_owned()
    } else {
        "head: provisional (an earlier head is final)".to_owned()
    }
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
        assert!(text.contains("guardians: none\n"));
        assert!(text.ends_with("records: 3\n"));
    }

    #[test]
    fn render_shows_guardians_and_a_pending_removal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(KEYSET_FILE);
        let owner = SigningKey::from_bytes(&[1; 32]);
        let mut guardians: Vec<String> = [11u8, 12, 13]
            .iter()
            .map(|seed| public_hex(&SigningKey::from_bytes(&[*seed; 32])))
            .collect();
        guardians.sort();
        let (mut file, _) = KeysetFile::open_or_migrate(&path, &owner).unwrap();
        file.append(
            &owner,
            KeysetEntry::GuardianSetUpdated {
                guardians: guardians.clone(),
                threshold: 2,
            },
            None,
        )
        .unwrap();
        let text = render_state(&file.state().unwrap());
        assert!(text.contains("guardians: 2 of 3 must approve a recovery\n"));
        assert!(text.contains(&format!("  {}\n", guardians[0])));
        assert!(!text.contains("pending guardian change"));
        // Dropping a guardian waits out the delay: the old set stays.
        file.append(
            &owner,
            KeysetEntry::GuardianSetUpdated {
                guardians: guardians[1..].to_vec(),
                threshold: 2,
            },
            None,
        )
        .unwrap();
        let text = render_state(&file.state().unwrap());
        assert!(text.contains("guardians: 2 of 3 must approve a recovery\n"));
        assert!(text.contains("pending guardian change: 2 of 2"));
    }

    #[test]
    fn the_head_reads_final_once_checkpointed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(KEYSET_FILE);
        let owner = SigningKey::from_bytes(&[1; 32]);
        let (mut file, _) = KeysetFile::open_or_migrate(&path, &owner).unwrap();
        let head = file.state().unwrap().head_hash;
        assert!(finality_line(&file, &head).contains("no checkpoint yet"));
        file.checkpoint(&owner).unwrap();
        assert_eq!(finality_line(&file, &head), "head: final (checkpointed)");
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

    #[test]
    fn revoke_accepts_a_device_qr_or_the_existing_hex_argument() {
        let device = SigningKey::from_bytes(&[7; 32]);
        let qr = DeviceQr::for_key(&device).qr_payload();
        assert_eq!(
            parse_revoke_device(&qr).unwrap(),
            DeviceQr::for_key(&device)
        );
        assert_eq!(
            parse_revoke_device(&public_hex(&device)).unwrap(),
            DeviceQr::for_key(&device)
        );
        assert!(parse_revoke_device("id:keyset-device?device=invalid").is_err());
    }
}
