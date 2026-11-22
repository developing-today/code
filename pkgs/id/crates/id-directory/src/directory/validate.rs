//! Validation and normalization helpers for the directory, split out of
//! directory.rs for the F11.10 production-line guard (#8591). The public
//! helpers are re-exported from the directory module at their historical paths.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use ed25519_dalek::{SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};

use id_caps::world_scopes::WorldScopes;
use id_sync::artifact;

use super::{Actor, KEY_DOMAIN, MAX_DESCRIPTION_CHARS, MAX_NAME_CHARS};

pub(super) fn artifact_path(journal: &Path) -> PathBuf {
    journal.with_file_name(artifact::FILE)
}

pub(super) fn signing_key(secret: &[u8; 32]) -> SigningKey {
    let mut hasher = Sha256::new();
    hasher.update(KEY_DOMAIN);
    hasher.update(secret);
    SigningKey::from_bytes(&hasher.finalize().into())
}

pub(super) fn may_act_for(actor: Actor<'_>, account: &str) -> bool {
    match actor {
        Actor::Server => true,
        Actor::Account(me) => me == account,
    }
}

/// A public key as lower-case hex, if it is an Ed25519 public key.
///
/// # Errors
///
/// Fails if the text is not 64 hex characters or not a valid key.
pub fn normalize_key(key: &str) -> Result<String> {
    let key = key.trim().to_ascii_lowercase();
    let bytes = hex_to_array::<32>(&key).context("a public key is 64 hex characters")?;
    VerifyingKey::from_bytes(&bytes).context("not an Ed25519 public key")?;
    Ok(key)
}

/// An email address in lower case, checked for shape but not deliverability.
///
/// # Errors
///
/// Fails if the text does not look like `local@domain.tld`.
pub fn normalize_email(address: &str) -> Result<String> {
    let address = address.trim().to_ascii_lowercase();
    ensure!(address.len() <= 254, "email address is too long");
    let (local, domain) = address
        .split_once('@')
        .context("an email address has one @")?;
    let local_ok = !local.is_empty()
        && local.len() <= 64
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+-/=?^_`{|}~.".contains(c));
    let domain_ok = domain.contains('.')
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    ensure!(local_ok && domain_ok, "not an email address");
    Ok(address)
}

pub(super) fn pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_owned(), b.to_owned())
    } else {
        (b.to_owned(), a.to_owned())
    }
}

pub(super) fn ensure_account_id(id: &str) -> Result<()> {
    ensure!(
        hex_to_array::<32>(id).is_some(),
        "account ID must be 64 hex characters"
    );
    Ok(())
}

pub(super) fn permissions(bits: u8) -> Result<WorldScopes> {
    WorldScopes::from_bits(bits).context("unknown permission bits")
}

pub(super) fn clean_name(name: &str) -> Result<String> {
    clean_name_mode(name, true)
}

pub(super) fn clean_name_mode(name: &str, refuse_format: bool) -> Result<String> {
    let name = name.trim();
    ensure!(!name.is_empty(), "name must not be empty");
    ensure!(
        name.chars().count() <= MAX_NAME_CHARS,
        "name must be at most {MAX_NAME_CHARS} characters"
    );
    ensure!(
        !name.chars().any(char::is_control),
        "name must not contain control characters"
    );
    if refuse_format {
        ensure!(
            !name.chars().any(id_core::petname::is_format_char),
            "name must not contain format characters"
        );
    }
    Ok(name.to_owned())
}

pub(super) fn clean_description(description: &str) -> Result<String> {
    clean_description_mode(description, true)
}

pub(super) fn clean_description_mode(description: &str, refuse_format: bool) -> Result<String> {
    let description = description.trim();
    ensure!(
        description.chars().count() <= MAX_DESCRIPTION_CHARS,
        "description must be at most {MAX_DESCRIPTION_CHARS} characters"
    );
    ensure!(
        !description.chars().any(|c| c.is_control() && c != '\n'),
        "description must not contain control characters"
    );
    if refuse_format {
        ensure!(
            !description.chars().any(id_core::petname::is_format_char),
            "description must not contain format characters"
        );
    }
    Ok(description.to_owned())
}

/// Bytes decoded from a hex string of exactly twice N hex digits, if it is one.
pub fn hex_to_array<const N: usize>(text: &str) -> Option<[u8; N]> {
    // Every byte must be an ASCII hex digit (#2517): u8::from_str_radix
    // also accepts a leading '+', so without this gate a byte 0x00-0x0f
    // has two accepted spellings ('0x' and '+x') and key strings stop
    // being canonical.
    if text.len() != N * 2 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0_u8; N];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(out)
}
