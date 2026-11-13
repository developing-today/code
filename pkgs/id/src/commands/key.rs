//! Local export and import commands for identity keys.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use zeroize::Zeroizing;

use crate::cli::KeyCommand;
use id_core::key_envelope;
use id_core::seed_phrase;
use crate::store::load_or_create_keypair;
use crate::{CLIENT_KEY_FILE, KEY_FILE};

const MIN_PASSPHRASE_BYTES: usize = 8;

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
    if replace {
        replace_private_file(&target, &key_bytes[..])?;
    } else {
        write_new_private_file(&target, &key_bytes[..]).with_context(|| {
            format!(
                "could not create identity key {}; use --replace to overwrite it",
                target.display()
            )
        })?;
    }
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
    if replace {
        replace_private_file(&target, &key_bytes[..])?;
    } else {
        write_new_private_file(&target, &key_bytes[..]).with_context(|| {
            format!(
                "could not create identity key {}; use --replace to overwrite it",
                target.display()
            )
        })?;
    }
    println!(
        "Imported identity key from seed phrase to {}",
        target.display()
    );
    Ok(())
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
}
