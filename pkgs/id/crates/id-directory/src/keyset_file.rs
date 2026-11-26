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

use std::collections::BTreeSet;

use crate::keyset::{
    KeysetEntry, KeysetRecord, KeysetState, Log, checkpoint_final_heads, fold_with_finality,
};
use crate::keyset_pairing::{DeviceQr, EnrolAccept, EnrolOffer};

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
    /// Heads this replica verified a checkpoint against (ADR Decision 4
    /// rule 5), kept beside the log in [`final_heads_path`]: they are
    /// this replica's own record, never merged from another.
    final_heads: BTreeSet<String>,
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
            return Ok((Self::loaded(path, log, genesis_hash)?, false));
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
        Ok((Self::loaded(path, log, genesis_hash)?, true))
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
        trusted_genesis(&log, identity)
            .map(|genesis_hash| Self::loaded(path, log, genesis_hash))
            .transpose()
    }

    /// Reads the persisted final heads and records any head a
    /// checkpoint in the log now covers.
    fn loaded(path: PathBuf, log: Log, genesis_hash: String) -> Result<Self> {
        let mut file = Self {
            final_heads: read_final_heads(&final_heads_path(&path))?,
            path,
            log,
            genesis_hash,
        };
        file.finalize()?;
        Ok(file)
    }

    /// Adds every head a verified checkpoint covers to the final heads,
    /// persisting the new ones. Returns how many were added.
    fn finalize(&mut self) -> Result<usize> {
        let covered = checkpoint_final_heads(&self.log, &self.genesis_hash, &self.final_heads)?;
        let fresh: Vec<String> = covered.difference(&self.final_heads).cloned().collect();
        if !fresh.is_empty() {
            append_final_heads(&final_heads_path(&self.path), &fresh)?;
            self.final_heads.extend(fresh.iter().cloned());
        }
        Ok(fresh.len())
    }

    /// The checkpoint-final heads this replica has verified.
    #[must_use]
    pub const fn final_heads(&self) -> &BTreeSet<String> {
        &self.final_heads
    }

    /// Signs a `Checkpoint` of the current head with `key`, which must
    /// be an active device key, appends it, and records the head as
    /// final. Returns the head hash it covers.
    ///
    /// # Errors
    ///
    /// Fails if `key` is not an active device key, signing fails, or a
    /// file cannot be written.
    pub fn checkpoint(&mut self, key: &SigningKey) -> Result<String> {
        let state = self.state()?;
        let record = KeysetRecord::sign(
            key,
            self.next_lamport()?,
            state.head_hash.clone(),
            KeysetEntry::Checkpoint {
                state_hash: state.state_hash()?,
                record_count: state.records_folded,
            },
            None,
        )?;
        let mut next = self.log.clone();
        next.insert(record.clone())?;
        let covered = checkpoint_final_heads(&next, &self.genesis_hash, &self.final_heads)?;
        ensure!(
            covered.contains(&state.head_hash),
            "{} is not an active device key of this identity",
            record.author
        );
        append_line(&self.path, &record)?;
        self.log = next;
        self.finalize()?;
        Ok(state.head_hash)
    }

    /// Merges records from another replica's copy of this log. Each one
    /// must verify; duplicates are skipped. Records that do not fold
    /// are kept as evidence, as in any fork. Returns how many were new.
    ///
    /// # Errors
    ///
    /// Fails if a record does not verify or the file cannot be written;
    /// records before the failing one stay merged.
    pub fn merge_records(
        &mut self,
        records: impl IntoIterator<Item = KeysetRecord>,
    ) -> Result<usize> {
        let mut added = 0;
        for record in records {
            if self.log.insert(record.clone())? {
                append_line(&self.path, &record)?;
                added += 1;
            }
        }
        self.finalize()?;
        Ok(added)
    }

    /// The offer an admin shows to enrol a new device by scan
    /// (#2380 packet 2): the current head, its position, and a label.
    ///
    /// # Errors
    ///
    /// Fails if `key` is not an active admin key.
    pub fn enrol_offer(&self, key: &SigningKey, label: &str) -> Result<EnrolOffer> {
        let state = self.state()?;
        let author = hex_encode(&key.verifying_key().to_bytes());
        ensure!(
            state.is_admin(&author),
            "{author} is not an active admin key of this identity"
        );
        EnrolOffer::mint(
            &state.identity_id,
            &author,
            &state.head_hash,
            self.next_lamport()?,
            label,
        )
    }

    /// Files a scanned [`EnrolAccept`] as a `DeviceEnrolled` record,
    /// countersigned by the new device. Anything stale is refused: the
    /// accept must address this admin, this identity, and the current
    /// head, and its countersignature must verify.
    ///
    /// # Errors
    ///
    /// Fails if the accept is stale, addresses another admin or
    /// identity, fails verification, the entry does not fold, or the
    /// file cannot be written.
    pub fn complete_enrol(
        &mut self,
        admin: &SigningKey,
        accept: &EnrolAccept,
    ) -> Result<KeysetRecord> {
        let state = self.state()?;
        ensure!(
            accept.offer.identity_id == state.identity_id,
            "this enrolment offer names another identity"
        );
        ensure!(
            accept.offer.prev_hash == state.head_hash,
            "this enrolment offer is stale: the keyset head moved on"
        );
        let author = hex_encode(&admin.verifying_key().to_bytes());
        ensure!(
            accept.offer.author == author,
            "this enrolment accept is addressed to another admin key"
        );
        ensure!(
            state.is_admin(&author),
            "{author} is not an active admin key of this identity"
        );
        let record = accept.record(admin)?;
        let mut next = self.log.clone();
        next.insert(record.clone())?;
        let after = fold_with_finality(&next, &self.genesis_hash, &self.final_heads)?;
        ensure!(
            after.head_hash == record.hash,
            "the keyset refused this enrolment"
        );
        append_line(&self.path, &record)?;
        self.log = next;
        self.finalize()?;
        Ok(record)
    }

    /// Revokes the device a scan named, with `reason`.
    ///
    /// # Errors
    ///
    /// Fails if `key` is not an active admin key, the scanned device is
    /// unknown or already revoked, or the file cannot be written.
    pub fn revoke_scanned(
        &mut self,
        key: &SigningKey,
        device: &DeviceQr,
        reason: &str,
    ) -> Result<KeysetRecord> {
        self.append(
            key,
            KeysetEntry::DeviceRevoked {
                device_key: device.device_key.clone(),
                reason: reason.to_owned(),
            },
            None,
        )
    }

    fn next_lamport(&self) -> Result<u64> {
        self.log
            .records()
            .map(|record| record.lamport)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .context("keyset lamport overflow")
    }

    /// The folded keyset state.
    ///
    /// # Errors
    ///
    /// Fails if the trusted genesis is missing, which a file opened
    /// here cannot be.
    pub fn state(&self) -> Result<KeysetState> {
        fold_with_finality(&self.log, &self.genesis_hash, &self.final_heads)
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
        let lamport = self.next_lamport()?;
        let record =
            KeysetRecord::sign(key, lamport, before.head_hash.clone(), entry, countersigner)?;
        let mut next = self.log.clone();
        next.insert(record.clone())?;
        let after = fold_with_finality(&next, &self.genesis_hash, &self.final_heads)?;
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

/// Where a keyset log's final heads live: beside it, one hash a line.
#[must_use]
pub fn final_heads_path(log_path: &Path) -> PathBuf {
    let mut name = log_path.as_os_str().to_owned();
    name.push(".final");
    PathBuf::from(name)
}

fn read_final_heads(path: &Path) -> Result<BTreeSet<String>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("could not read {}", path.display()));
        }
    };
    let complete = match text.rfind('\n') {
        Some(end) => &text[..=end],
        None => "",
    };
    let mut heads = BTreeSet::new();
    for (index, line) in complete.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        ensure!(
            line.len() == 64
                && line
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')),
            "{} line {}: not a record hash",
            path.display(),
            index + 1
        );
        heads.insert(line.to_owned());
    }
    Ok(heads)
}

fn append_final_heads(path: &Path, heads: &[String]) -> Result<()> {
    drop_torn_tail(path)?;
    let mut text = String::new();
    for head in heads {
        text.push_str(head);
        text.push('\n');
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("could not open {}", path.display()))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .with_context(|| format!("could not write {}", path.display()))
}

/// The complete lines of a JSONL file: empty when it does not exist,
/// and without a torn final line.
///
/// # Errors
///
/// Fails if the file exists but cannot be read.
pub(crate) fn read_complete(path: &Path) -> Result<String> {
    let mut text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(String::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("could not read {}", path.display()));
        }
    };
    text.truncate(text.rfind('\n').map_or(0, |end| end + 1));
    Ok(text)
}

fn read_log(path: &Path) -> Result<Log> {
    let complete = read_complete(path)?;
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

/// Appends `record` as one JSON line and syncs, first cutting any torn
/// final line.
///
/// # Errors
///
/// Fails if the record cannot be encoded or the file written.
pub(crate) fn append_line<T: serde::Serialize>(path: &Path, record: &T) -> Result<()> {
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

    #[test]
    fn a_checkpoint_pins_the_head_across_a_merge_and_a_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(KEYSET_FILE);
        let owner = key(1);
        let (mut file, _) = KeysetFile::open_or_migrate(&path, &owner).unwrap();
        let genesis = file.genesis_hash().to_owned();
        file.append(
            &owner,
            KeysetEntry::IdpBindingBound {
                binding_id: "mine".to_owned(),
            },
            None,
        )
        .unwrap();
        let head = file.state().unwrap().head_hash;
        assert_eq!(file.checkpoint(&owner).unwrap(), head);
        assert!(file.final_heads().contains(&head));
        // Another replica's sibling of the bind, sorting first.
        let sibling = KeysetRecord::sign(
            &owner,
            1,
            genesis,
            KeysetEntry::IdpBindingBound {
                binding_id: "theirs".to_owned(),
            },
            None,
        )
        .unwrap();
        assert_eq!(file.merge_records([sibling.clone(), sibling]).unwrap(), 1);
        assert_eq!(file.state().unwrap().head_hash, head);
        let (again, _) = KeysetFile::open_or_migrate(&path, &owner).unwrap();
        assert_eq!(again.state().unwrap().head_hash, head);
        assert!(final_heads_path(&path).exists());
    }

    #[test]
    fn only_a_device_key_can_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(KEYSET_FILE);
        let (mut file, _) = KeysetFile::open_or_migrate(&path, &key(1)).unwrap();
        assert!(file.checkpoint(&key(5)).is_err());
        assert!(file.final_heads().is_empty());
        assert_eq!(file.records().count(), 1);
    }
}
