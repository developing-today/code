//! The local keyset log file and the lazy migration of single-key
//! accounts (#2380 packet 4, keyset ADR Decision 5).
//!
//! The file holds one [`KeysetRecord`] per line as JSON. Every line is
//! verified on load, so the in-memory [`Log`] only ever holds records
//! that verify; a complete line that does not parse or verify is an
//! error, never a silent drop. An unterminated final line is a torn
//! append and is ignored, as in the group log.
//!
//! Migration is lazy and idempotent: the first keyset-aware use of an
//! identity key with no genesis in its file appends a `Genesis` record
//! signed by that key, enrolling it as both the admin key and the first
//! device key. The identity identifier stays that key's public key
//! (Decision 1), so invites, the directory and group membership see no
//! change. The trusted genesis that anchors every fold is the root
//! record that key authored, chosen outside the mergeable log as
//! [`fold`] requires.

use std::fs::OpenOptions;
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use ed25519_dalek::SigningKey;
use id_core::hex::hex_encode;

use crate::keyset::{KeysetEntry, KeysetRecord, KeysetState, Log, fold};

/// The server identity's keyset log, beside [`id_core::KEY_FILE`].
pub const KEYSET_FILE: &str = ".id-keyset.jsonl";
/// The client identity's keyset log, beside [`id_core::CLIENT_KEY_FILE`].
pub const CLIENT_KEYSET_FILE: &str = ".id-keyset-client.jsonl";

/// The label migration gives the identity's first device.
pub const MIGRATED_DEVICE_LABEL: &str = "original device";

/// One identity's keyset log on disk, anchored at its trusted genesis.
#[derive(Debug)]
pub struct KeysetFile {
    path: PathBuf,
    log: Log,
    genesis_hash: String,
}

impl KeysetFile {
    /// Opens the keyset log at `path` for the identity `key` names,
    /// appending a genesis signed by `key` when the file holds none for
    /// it (the lazy migration of Decision 5). Returns the file and
    /// whether this call migrated the account.
    ///
    /// # Errors
    ///
    /// Fails if the file cannot be read or written, holds a record that
    /// does not verify, or holds records for another identity only.
    pub fn open_or_migrate(path: impl Into<PathBuf>, key: &SigningKey) -> Result<(Self, bool)> {
        let path = path.into();
        let mut log = read_log(&path)?;
        let identity = hex_encode(&key.verifying_key().to_bytes());
        if let Some(genesis_hash) = trusted_genesis(&log, &identity) {
            return Ok((
                Self {
                    path,
                    log,
                    genesis_hash,
                },
                false,
            ));
        }
        ensure!(
            log.records().next().is_none(),
            "keyset log {} belongs to another identity",
            path.display()
        );
        let genesis = KeysetRecord::sign(
            key,
            0,
            "",
            KeysetEntry::Genesis {
                admin_key: identity.clone(),
                device_key: identity,
                label: MIGRATED_DEVICE_LABEL.to_owned(),
            },
            None,
        )?;
        append_line(&path, &genesis)?;
        let genesis_hash = genesis.hash.clone();
        log.insert(genesis)?;
        Ok((
            Self {
                path,
                log,
                genesis_hash,
            },
            true,
        ))
    }

    /// Opens the keyset log at `path` for the identity `identity` (its
    /// public key, hex) without writing anything: `None` when the file
    /// holds no genesis for that identity, missing file included.
    ///
    /// # Errors
    ///
    /// Fails if the file cannot be read or holds a record that does not
    /// verify.
    pub fn open_existing(path: impl Into<PathBuf>, identity: &str) -> Result<Option<Self>> {
        let path = path.into();
        let log = read_log(&path)?;
        Ok(trusted_genesis(&log, identity).map(|genesis_hash| Self {
            path,
            log,
            genesis_hash,
        }))
    }

    /// The folded keyset state.
    ///
    /// # Errors
    ///
    /// Fails if the trusted genesis is missing, which a file opened
    /// here cannot be.
    pub fn state(&self) -> Result<KeysetState> {
        fold(&self.log, &self.genesis_hash)
    }

    /// The hash of the genesis record anchoring this identity's fold.
    #[must_use]
    pub fn genesis_hash(&self) -> &str {
        &self.genesis_hash
    }

    /// The verified records, in merge-key order.
    pub fn records(&self) -> impl Iterator<Item = &KeysetRecord> {
        self.log.records()
    }

    /// Signs `entry` with `key` at the chain head and appends it. The
    /// record is written only when it folds: an entry the fold would
    /// refuse (a signer that is not an active admin, an unknown or
    /// already revoked device, a rotation to the same key) leaves the
    /// file and the log unchanged.
    ///
    /// # Errors
    ///
    /// Fails if the entry is refused, signing fails, or the file cannot
    /// be written.
    pub fn append(
        &mut self,
        key: &SigningKey,
        entry: KeysetEntry,
        countersigner: Option<&SigningKey>,
    ) -> Result<KeysetRecord> {
        let before = self.state()?;
        let lamport = self
            .log
            .records()
            .map(|record| record.lamport)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .context("keyset lamport overflow")?;
        let record =
            KeysetRecord::sign(key, lamport, before.head_hash.clone(), entry, countersigner)?;
        let mut next = self.log.clone();
        next.insert(record.clone())?;
        let after = fold(&next, &self.genesis_hash)?;
        if after.head_hash != record.hash {
            let author = &record.author;
            if before.is_admin(author) {
                bail!("the keyset refused this change");
            }
            bail!("{author} is not an active admin key of this identity");
        }
        append_line(&self.path, &record)?;
        self.log = next;
        Ok(record)
    }
}

/// The trusted genesis for `identity`: the root record that key
/// authored as its own admin key. Should a file ever hold two, the
/// smaller merge key wins, as at any fork.
fn trusted_genesis(log: &Log, identity: &str) -> Option<String> {
    log.records()
        .find(|record| {
            record.prev_hash.is_empty()
                && record.author == identity
                && matches!(&record.entry, KeysetEntry::Genesis { admin_key, .. } if admin_key == identity)
        })
        .map(|record| record.hash.clone())
}

fn read_log(path: &Path) -> Result<Log> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Log::default()),
        Err(error) => {
            return Err(error).with_context(|| format!("could not read {}", path.display()));
        }
    };
    let complete = match text.rfind('\n') {
        Some(end) => &text[..=end],
        None => "",
    };
    let mut log = Log::default();
    for (index, line) in complete.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let record: KeysetRecord = serde_json::from_str(line).with_context(|| {
            format!("{} line {}: not a keyset record", path.display(), index + 1)
        })?;
        log.insert(record).with_context(|| {
            format!(
                "{} line {}: record does not verify",
                path.display(),
                index + 1
            )
        })?;
    }
    Ok(log)
}

fn append_line(path: &Path, record: &KeysetRecord) -> Result<()> {
    drop_torn_tail(path)?;
    let mut line = serde_json::to_vec(record)?;
    line.push(b'\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("could not open {}", path.display()))?;
    file.write_all(&line)
        .and_then(|()| file.sync_all())
        .with_context(|| format!("could not write {}", path.display()))
}

/// Cuts an unterminated final line (a torn append) so the next record
/// starts on a line of its own instead of being glued onto it.
fn drop_torn_tail(path: &Path) -> Result<()> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("could not read {}", path.display()));
        }
    };
    if bytes.last().is_none_or(|last| *last == b'\n') {
        return Ok(());
    }
    let keep = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |end| end + 1);
    OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|file| file.set_len(u64::try_from(keep).unwrap_or(u64::MAX)))
        .with_context(|| format!("could not repair {}", path.display()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::keyset::AdminKeyState;

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn public(seed: u8) -> String {
        hex_encode(&key(seed).verifying_key().to_bytes())
    }

    #[test]
    fn first_use_migrates_once_and_keeps_the_identity_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(KEYSET_FILE);
        let (file, migrated) = KeysetFile::open_or_migrate(&path, &key(1)).unwrap();
        assert!(migrated);
        let state = file.state().unwrap();
        assert_eq!(state.identity_id, public(1));
        assert!(state.is_admin(&public(1)));
        assert!(state.is_device(&public(1)));
        assert_eq!(state.records_folded, 1);

        let (again, migrated) = KeysetFile::open_or_migrate(&path, &key(1)).unwrap();
        assert!(!migrated, "a second open must not mint a second genesis");
        assert_eq!(again.genesis_hash(), file.genesis_hash());
        assert_eq!(again.records().count(), 1);
    }

    #[test]
    fn enrol_revoke_and_rotate_survive_a_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(KEYSET_FILE);
        let (mut file, _) = KeysetFile::open_or_migrate(&path, &key(1)).unwrap();
        file.append(
            &key(1),
            KeysetEntry::DeviceEnrolled {
                device_key: public(2),
                label: "laptop".to_owned(),
            },
            Some(&key(2)),
        )
        .unwrap();
        file.append(
            &key(1),
            KeysetEntry::DeviceRevoked {
                device_key: public(2),
                reason: "lost".to_owned(),
            },
            None,
        )
        .unwrap();
        file.append(
            &key(1),
            KeysetEntry::AdminKeyRotated {
                new_admin_key: public(3),
            },
            Some(&key(3)),
        )
        .unwrap();

        let (reopened, migrated) = KeysetFile::open_or_migrate(&path, &key(1)).unwrap();
        assert!(!migrated);
        let state = reopened.state().unwrap();
        assert_eq!(
            state.identity_id,
            public(1),
            "rotation keeps the identity id"
        );
        assert!(!state.is_device(&public(2)));
        assert_eq!(
            state.admin_keys.get(&public(1)),
            Some(&AdminKeyState::Rotated)
        );
        assert!(state.is_admin(&public(3)));
        assert_eq!(state.records_folded, 4);
    }

    #[test]
    fn a_refused_change_is_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(KEYSET_FILE);
        let (mut file, _) = KeysetFile::open_or_migrate(&path, &key(1)).unwrap();
        // Key 2 is not an admin of this identity.
        let error = file
            .append(
                &key(2),
                KeysetEntry::DeviceRevoked {
                    device_key: public(1),
                    reason: "takeover".to_owned(),
                },
                None,
            )
            .unwrap_err();
        assert!(error.to_string().contains("not an active admin"));
        // Revoking a device the keyset never enrolled folds nowhere.
        assert!(
            file.append(
                &key(1),
                KeysetEntry::DeviceRevoked {
                    device_key: public(9),
                    reason: "unknown".to_owned(),
                },
                None,
            )
            .is_err()
        );
        let (reopened, _) = KeysetFile::open_or_migrate(&path, &key(1)).unwrap();
        assert_eq!(reopened.records().count(), 1);
        assert_eq!(reopened.state().unwrap().records_folded, 1);
    }

    #[test]
    fn another_identitys_log_is_refused_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(KEYSET_FILE);
        KeysetFile::open_or_migrate(&path, &key(1)).unwrap();
        let error = KeysetFile::open_or_migrate(&path, &key(5)).unwrap_err();
        assert!(error.to_string().contains("another identity"));
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 1);
    }

    #[test]
    fn a_torn_tail_is_ignored_and_a_bad_line_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(KEYSET_FILE);
        KeysetFile::open_or_migrate(&path, &key(1)).unwrap();
        let good = std::fs::read_to_string(&path).unwrap();

        std::fs::write(&path, format!("{good}{{\"author\":\"ab")).unwrap();
        let (mut file, migrated) = KeysetFile::open_or_migrate(&path, &key(1)).unwrap();
        assert!(!migrated);
        assert_eq!(file.records().count(), 1);
        // The next append starts a fresh line instead of gluing onto
        // the torn fragment.
        file.append(
            &key(1),
            KeysetEntry::IdpBindingBound {
                binding_id: "b1".to_owned(),
            },
            None,
        )
        .unwrap();
        let (reopened, _) = KeysetFile::open_or_migrate(&path, &key(1)).unwrap();
        assert_eq!(reopened.state().unwrap().records_folded, 2);

        std::fs::write(&path, format!("{good}not json\n")).unwrap();
        assert!(KeysetFile::open_or_migrate(&path, &key(1)).is_err());
    }
}
