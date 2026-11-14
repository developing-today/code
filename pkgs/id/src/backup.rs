//! Server-side identity backup, implementing the #5033 design decision
//! for issue #7360.
//!
//! Format (v1, #5033 items 1–4):
//! - The backup blob is the [`id_core::key_envelope`] v1 envelope
//!   byte-for-byte: `IDKEY` magic, version byte 1, a 60-byte header,
//!   the 32-byte identity key wrapped with Argon2id + XChaCha20-Poly1305,
//!   108 bytes total. The server never sees a passphrase and cannot
//!   decrypt the blob; decryption stays client-side through
//!   `key_envelope::decrypt`.
//! - A backup record is keyed by `H(pub) = SHA-256(identity public
//!   key)` (hex in diagnostics) and holds ONLY the #5033 item-2 field
//!   set: envelope bytes, envelope SHA-256, blob size, upload/download
//!   timestamps, the monotone upload epoch, throttle counters,
//!   `restore_epoch`, and a deletion tombstone (`H(pub)` + final
//!   epoch). [`BackupRecord::stored_field_names`] is the authoritative
//!   list; a test pins it to the decision so a new stored field fails
//!   CI until the decision is amended.
//! - Uploads are epoch-guarded: the client presents
//!   `Sign_identity(H(pub) || envelope_sha256 || epoch_be)` and the
//!   server verifies it against the named public key. Non-increasing
//!   epochs are rejected; only a valid higher-epoch upload overwrites.
//! - Downloads are budgeted per `H(pub)`: 10/hour and 50/day, counted
//!   with weighted fixed windows in the shape of
//!   [`crate::abuse`]'s `WindowCounter` (current window in full plus
//!   the previous window weighted by the fraction still inside the
//!   rolling span). Refusals carry a retry-after. Per-IP and global
//!   limits stay with the endpoint layer's `abuse::AbuseEvaluator`;
//!   this module owns only the per-identity budget from the decision.
//! - Deletion is signed (`Sign_identity(H(pub) || "delete" ||
//!   epoch_be)` over the record's current epoch): the ciphertext and
//!   counters are dropped and only the tombstone remains, so replaying
//!   the pre-delete signed upload cannot resurrect the backup. Account
//!   deletion purges the tombstone itself.
//! - Restore bookkeeping (#5033 item 5, interim rule): a
//!   download-for-restore increments `restore_epoch` and emits a
//!   notification for every bound device; the restored key is not
//!   authoritative for new group epochs until the notifications are
//!   confirmed via [`BackupStore::confirm_restore_notifications`].
//!   Confirmations name a [`RestoreToken`] — the restore epoch plus
//!   the backup generation (a store-level counter bumped every time
//!   a backup record is created for an identity, never reset by
//!   deletion) — so a delayed confirmation from a deleted backup
//!   cannot clear a recreated backup's pending notifications even
//!   when the recreated backup reuses the same restore epoch.
//!   No rotation flag is introduced here — that representation belongs
//!   to #2380.

use std::collections::HashMap;
use std::fmt;

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use sha2::{Digest as _, Sha256};

use id_core::key_envelope;

/// Total v1 envelope length: 60-byte header + 32-byte wrapped key +
/// 16-byte AEAD tag, matching `key_envelope`'s layout.
pub const BACKUP_ENVELOPE_LEN: usize = 108;
/// Envelope magic, mirroring `key_envelope`'s `IDKEY` header.
const ENVELOPE_MAGIC: &[u8; 5] = b"IDKEY";
/// Envelope version byte offset and expected v1 value.
const ENVELOPE_VERSION_OFFSET: usize = 5;
const ENVELOPE_VERSION_V1: u8 = 1;
/// Envelope KDF/cipher byte offsets and expected v1 values, mirroring
/// `key_envelope`'s header layout.
const ENVELOPE_KDF_OFFSET: usize = 6;
const ENVELOPE_KDF_ARGON2ID: u8 = 1;
const ENVELOPE_CIPHER_OFFSET: usize = 7;
const ENVELOPE_CIPHER_XCHACHA20_POLY1305: u8 = 1;
/// KDF parameter bounds, mirroring `key_envelope`'s decrypt-path checks.
const MIN_MEMORY_KIB: u32 = 19_456;
const MAX_MEMORY_KIB: u32 = 262_144;
const MIN_ITERATIONS: u32 = 2;
const MAX_ITERATIONS: u32 = 10;
const MIN_LANES: u32 = 1;
const MAX_LANES: u32 = 4;

/// Per-identity download budget from the #5033 decision: 10 per hour,
/// 50 per day, per `H(pub)`.
pub const DOWNLOAD_HOUR_LIMIT: u64 = 10;
pub const DOWNLOAD_HOUR_WINDOW_MS: u64 = 3_600_000;
pub const DOWNLOAD_DAY_LIMIT: u64 = 50;
pub const DOWNLOAD_DAY_WINDOW_MS: u64 = 86_400_000;

/// `H(pub)`: SHA-256 of the identity public key. The only identity a
/// backup record is ever stored under.
pub fn key_hash(public_key: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(public_key);
    hasher.finalize().into()
}

/// Lowercase hex rendering of a [`key_hash`], for logs and diagnostics.
pub fn key_hash_hex(public_key: &[u8; 32]) -> String {
    key_hash(public_key)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn envelope_sha256(envelope: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(envelope);
    hasher.finalize().into()
}

/// The exact byte string an upload signature covers (#5033 item 4):
/// `H(pub) || envelope_sha256 || epoch` with the epoch big-endian.
pub fn upload_message(key_hash: &[u8; 32], envelope_sha256: &[u8; 32], epoch: u64) -> Vec<u8> {
    let mut msg = Vec::with_capacity(72);
    msg.extend_from_slice(key_hash);
    msg.extend_from_slice(envelope_sha256);
    msg.extend_from_slice(&epoch.to_be_bytes());
    msg
}

/// The exact byte string a delete signature covers:
/// `H(pub) || "delete" || epoch` over the record's current epoch.
pub fn delete_message(key_hash: &[u8; 32], epoch: u64) -> Vec<u8> {
    let mut msg = Vec::with_capacity(46);
    msg.extend_from_slice(key_hash);
    msg.extend_from_slice(b"delete");
    msg.extend_from_slice(&epoch.to_be_bytes());
    msg
}

/// Client side: sign an upload for `signing_key`'s identity.
pub fn sign_upload(signing_key: &SigningKey, envelope: &[u8], epoch: u64) -> [u8; 64] {
    let public = signing_key.verifying_key().to_bytes();
    let msg = upload_message(&key_hash(&public), &envelope_sha256(envelope), epoch);
    signing_key.sign(&msg).to_bytes()
}

/// Client side: sign a delete for `signing_key`'s identity at the
/// record's current `epoch`.
pub fn sign_delete(signing_key: &SigningKey, epoch: u64) -> [u8; 64] {
    let public = signing_key.verifying_key().to_bytes();
    let msg = delete_message(&key_hash(&public), epoch);
    signing_key.sign(&msg).to_bytes()
}

/// Named refusal reasons for every backup operation. A caller (the
/// endpoint layer) maps these onto wire errors; the names are the
/// contract tests assert against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackupError {
    /// The envelope is not a well-formed v1 key envelope.
    MalformedEnvelope(&'static str),
    /// The presented signature does not verify against the named
    /// public key (or the key/signature bytes are not valid Ed25519).
    BadSignature,
    /// A live record exists at an epoch >= the presented one.
    StaleEpoch { current: u64, presented: u64 },
    /// No backup (live or tombstoned) exists for this identity.
    NotFound,
    /// The backup was deleted and this upload's epoch does not exceed
    /// the tombstone's final epoch — a replay, not a new backup.
    Tombstoned { final_epoch: u64 },
    /// The per-identity download budget is exhausted; retry after the
    /// given number of milliseconds.
    DownloadBudgetExceeded { retry_after_ms: u64 },
}

impl fmt::Display for BackupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedEnvelope(why) => write!(f, "malformed backup envelope: {why}"),
            Self::BadSignature => write!(f, "backup signature does not verify"),
            Self::StaleEpoch { current, presented } => write!(
                f,
                "stale backup epoch: record is at epoch {current}, upload presented {presented}"
            ),
            Self::NotFound => write!(f, "no backup stored for this identity"),
            Self::Tombstoned { final_epoch } => write!(
                f,
                "backup was deleted at epoch {final_epoch}; replayed uploads cannot resurrect it"
            ),
            Self::DownloadBudgetExceeded { retry_after_ms } => write!(
                f,
                "download budget exceeded; retry after {retry_after_ms} ms"
            ),
        }
    }
}

impl std::error::Error for BackupError {}

/// One weighted fixed window, mirroring the estimator shape of
/// `abuse::WindowCounter`: the current fixed window counts in full,
/// the previous window counts weighted by the fraction of it still
/// inside the rolling span.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WeightedWindow {
    window_ms: u64,
    cur_start_ms: u64,
    cur: u64,
    prev: u64,
}

impl WeightedWindow {
    fn new(window_ms: u64, now_ms: u64) -> Self {
        Self {
            window_ms: window_ms.max(1),
            cur_start_ms: now_ms,
            cur: 0,
            prev: 0,
        }
    }

    fn roll(&mut self, now_ms: u64) {
        let elapsed = now_ms.saturating_sub(self.cur_start_ms);
        if elapsed >= self.window_ms {
            if elapsed >= self.window_ms.saturating_mul(2) {
                self.prev = 0;
                self.cur_start_ms = now_ms;
            } else {
                self.prev = self.cur;
                self.cur_start_ms += self.window_ms;
            }
            self.cur = 0;
        }
    }

    fn rolled(&self, now_ms: u64) -> Self {
        let mut rolled = *self;
        rolled.roll(now_ms);
        rolled
    }

    /// Weighted estimate of the count inside the rolling span at
    /// `now_ms`, without mutating.
    fn estimate(&self, now_ms: u64) -> u64 {
        let rolled = self.rolled(now_ms);
        let elapsed = now_ms.saturating_sub(rolled.cur_start_ms);
        let remaining = rolled.window_ms.saturating_sub(elapsed);
        let weighted =
            u128::from(rolled.prev) * u128::from(remaining) / u128::from(rolled.window_ms);
        rolled
            .cur
            .saturating_add(u64::try_from(weighted).unwrap_or(u64::MAX))
    }

    /// Earliest delay after `now_ms` at which the weighted estimate
    /// falls below `limit` — the next admissible instant, per the
    /// Codex finding on PR #7658 (4506243710). The old value (next
    /// fixed-window boundary) is wrong for a weighted estimator: at
    /// the boundary the previous window still contributes its full
    /// count, so a client retrying exactly then is refused again and
    /// advertised another full window, even though the estimate
    /// permits admission ~1 ms later. The estimate is monotonically
    /// non-increasing in time (the previous window's weight decays
    /// linearly, a roll re-weights the old current count to at most
    /// its end-of-window value, and two windows out the estimate is
    /// 0), so binary search over `estimate` finds the minimum.
    /// Oracle: the estimator itself — retrying at `now_ms + result`
    /// must be admissible, and at any earlier instant must not.
    fn retry_after_ms(&self, now_ms: u64, limit: u64) -> u64 {
        let mut lo: u64 = 1;
        let mut hi: u64 = self.window_ms.saturating_mul(2).max(1);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.estimate(now_ms.saturating_add(mid)) < limit {
                hi = mid;
            } else {
                lo = mid.saturating_add(1);
            }
        }
        lo.max(1)
    }

    fn record(&mut self, now_ms: u64) {
        self.roll(now_ms);
        self.cur = self.cur.saturating_add(1);
    }
}

/// Throttle counters stored on a backup record: the per-identity
/// download budget (hour and day windows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DownloadBudget {
    hour: WeightedWindow,
    day: WeightedWindow,
}

impl DownloadBudget {
    fn new(now_ms: u64) -> Self {
        Self {
            hour: WeightedWindow::new(DOWNLOAD_HOUR_WINDOW_MS, now_ms),
            day: WeightedWindow::new(DOWNLOAD_DAY_WINDOW_MS, now_ms),
        }
    }

    /// Admit one download at `now_ms`, charging both windows, or
    /// refuse with the retry-after of the refusing window (the later
    /// of the two when both refuse).
    fn try_admit(&mut self, now_ms: u64) -> Result<(), BackupError> {
        let hour_full = self.hour.estimate(now_ms) >= DOWNLOAD_HOUR_LIMIT;
        let day_full = self.day.estimate(now_ms) >= DOWNLOAD_DAY_LIMIT;
        if hour_full || day_full {
            let mut retry = 0;
            if hour_full {
                retry = retry.max(self.hour.retry_after_ms(now_ms, DOWNLOAD_HOUR_LIMIT));
            }
            if day_full {
                retry = retry.max(self.day.retry_after_ms(now_ms, DOWNLOAD_DAY_LIMIT));
            }
            return Err(BackupError::DownloadBudgetExceeded {
                retry_after_ms: retry,
            });
        }
        self.hour.record(now_ms);
        self.day.record(now_ms);
        Ok(())
    }

    /// Downloads admitted inside the current fixed hour window.
    /// Exposed for tests and endpoint diagnostics.
    pub fn hour_count(&self, now_ms: u64) -> u64 {
        self.hour.rolled(now_ms).cur
    }

    /// Downloads admitted inside the current fixed day window.
    pub fn day_count(&self, now_ms: u64) -> u64 {
        self.day.rolled(now_ms).cur
    }
}

impl Default for DownloadBudget {
    fn default() -> Self {
        Self::new(0)
    }
}

/// The deletion tombstone that survives a signed delete:
/// `H(pub)` + the record's final epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tombstone {
    /// `H(pub)` of the deleted backup.
    pub key_hash: [u8; 32],
    /// The upload epoch the record had reached when deleted.
    pub final_epoch: u64,
}

/// One live backup record. The field set is exactly the #5033 item-2
/// list — see the module docs. There is deliberately no rotation
/// flag: that representation belongs to #2380.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupRecord {
    envelope: Vec<u8>,
    envelope_sha256: [u8; 32],
    blob_size: u64,
    uploaded_at_ms: u64,
    last_downloaded_at_ms: Option<u64>,
    epoch: u64,
    throttle_counters: DownloadBudget,
    restore_epoch: u64,
    tombstone: Option<Tombstone>,
}

impl BackupRecord {
    /// The stored field names, in declaration order — the #5033
    /// item-2 list. A server data dump of a record contains exactly
    /// these fields and no others.
    pub fn stored_field_names() -> &'static [&'static str] {
        &[
            "envelope",
            "envelope_sha256",
            "blob_size",
            "uploaded_at_ms",
            "last_downloaded_at_ms",
            "epoch",
            "throttle_counters",
            "restore_epoch",
            "tombstone",
        ]
    }

    /// The stored envelope bytes (ciphertext only).
    pub fn envelope(&self) -> &[u8] {
        &self.envelope
    }

    /// SHA-256 of the stored envelope bytes.
    pub fn envelope_sha256(&self) -> &[u8; 32] {
        &self.envelope_sha256
    }

    /// The record's current monotone upload epoch.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// How many restore downloads have completed.
    pub fn restore_epoch(&self) -> u64 {
        self.restore_epoch
    }
}

enum BackupEntry {
    Live(BackupRecord),
    Deleted(Tombstone),
}

/// A restore notification owed to one bound device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceNotification {
    /// `H(pub)` of the restored identity.
    pub key_hash: [u8; 32],
    /// The bound device's public key to notify.
    pub device_public_key: [u8; 32],
    /// The backup generation the notification belongs to.
    pub generation: u64,
    /// The restore epoch the notification announces.
    pub restore_epoch: u64,
}

/// The confirmation token for one restore's notifications: the
/// backup generation plus the restore epoch. Both halves must match
/// the pending restore exactly — a token from an earlier backup of
/// the same identity (before a delete + recreate) never confirms a
/// later backup's restore, even at the same restore epoch (Codex
/// review on PR #7658).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RestoreToken {
    /// Backup generation: bumped on every record creation for the
    /// identity, never reset by deletion or tombstone purge.
    pub generation: u64,
    /// The restore epoch within that generation.
    pub restore_epoch: u64,
}

/// What a successful upload stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UploadReceipt {
    /// The record key, `H(pub)`.
    pub key_hash: [u8; 32],
    /// The epoch now stored.
    pub epoch: u64,
    /// SHA-256 of the stored envelope.
    pub envelope_sha256: [u8; 32],
}

/// What a download-for-restore returns: the envelope plus the new
/// restore epoch. The envelope still needs the client-side passphrase
/// decrypt before the key exists anywhere.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestoreReceipt {
    /// The stored envelope bytes.
    pub envelope: Vec<u8>,
    /// `restore_epoch` after this restore's increment.
    pub restore_epoch: u64,
    /// The token that confirms this restore's notifications.
    pub token: RestoreToken,
    /// Notifications emitted to bound devices by this restore.
    pub notifications: Vec<DeviceNotification>,
}

/// The server-side backup store: one entry per identity, keyed by
/// `H(pub)`. In-memory here; the endpoint layer persists the same
/// record shape. Bound-device registrations and restore-notification
/// state are server registry data, NOT part of a backup record's
/// stored field set.
#[derive(Default)]
pub struct BackupStore {
    entries: HashMap<[u8; 32], BackupEntry>,
    bound_devices: HashMap<[u8; 32], Vec<[u8; 32]>>,
    restore_pending: HashMap<[u8; 32], RestoreToken>,
    /// Per-identity backup generation: bumped on every record
    /// creation, never reset (deletion and tombstone purge keep it),
    /// so restore tokens stay unique across delete + recreate.
    generations: HashMap<[u8; 32], u64>,
}

fn validate_envelope(envelope: &[u8]) -> Result<(), BackupError> {
    if envelope.len() != BACKUP_ENVELOPE_LEN {
        return Err(BackupError::MalformedEnvelope(
            "length is not the v1 envelope length",
        ));
    }
    if &envelope[..ENVELOPE_MAGIC.len()] != ENVELOPE_MAGIC {
        return Err(BackupError::MalformedEnvelope("bad envelope magic"));
    }
    if envelope[ENVELOPE_VERSION_OFFSET] != ENVELOPE_VERSION_V1 {
        return Err(BackupError::MalformedEnvelope(
            "unsupported envelope version",
        ));
    }
    // Mirror the remaining structural checks in key_envelope::decrypt:
    // an upload the decrypt path would reject for its header must not
    // be stored, or it would replace a working backup with one no
    // passphrase can restore (Codex review on PR #7658).
    if envelope[ENVELOPE_KDF_OFFSET] != ENVELOPE_KDF_ARGON2ID {
        return Err(BackupError::MalformedEnvelope("unsupported envelope KDF"));
    }
    if envelope[ENVELOPE_CIPHER_OFFSET] != ENVELOPE_CIPHER_XCHACHA20_POLY1305 {
        return Err(BackupError::MalformedEnvelope(
            "unsupported envelope cipher",
        ));
    }
    let memory_kib = u32::from_le_bytes(
        envelope[8..12]
            .try_into()
            .map_err(|_| BackupError::MalformedEnvelope("truncated KDF memory parameter"))?,
    );
    let iterations = u32::from_le_bytes(
        envelope[12..16]
            .try_into()
            .map_err(|_| BackupError::MalformedEnvelope("truncated KDF iteration parameter"))?,
    );
    let lanes = u32::from_le_bytes(
        envelope[16..20]
            .try_into()
            .map_err(|_| BackupError::MalformedEnvelope("truncated KDF lane parameter"))?,
    );
    if !(MIN_MEMORY_KIB..=MAX_MEMORY_KIB).contains(&memory_kib)
        || !(MIN_ITERATIONS..=MAX_ITERATIONS).contains(&iterations)
        || !(MIN_LANES..=MAX_LANES).contains(&lanes)
        || memory_kib < lanes.saturating_mul(8)
    {
        return Err(BackupError::MalformedEnvelope(
            "envelope KDF parameters are outside the supported range",
        ));
    }
    Ok(())
}

fn verify(public_key: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> Result<(), BackupError> {
    let verifying = VerifyingKey::from_bytes(public_key).map_err(|_| BackupError::BadSignature)?;
    verifying
        .verify_strict(message, &Signature::from_bytes(signature))
        .map_err(|_| BackupError::BadSignature)
}

impl BackupStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Bump and return the backup generation for `hash`. Called on
    /// every record creation (first upload, or recreation after a
    /// delete) — never on an overwrite of a live record.
    fn bump_generation(&mut self, hash: &[u8; 32]) -> u64 {
        let generation = self.generations.entry(*hash).or_default();
        *generation = generation.saturating_add(1);
        *generation
    }

    /// The current backup generation for `hash` (0 = never created).
    fn generation(&self, hash: &[u8; 32]) -> u64 {
        self.generations.get(hash).copied().unwrap_or(0)
    }

    /// Upload endpoint (#5033 item 4): verify the signature against
    /// the named public key, reject non-increasing epochs, overwrite
    /// only on a valid higher-epoch upload.
    pub fn upload(
        &mut self,
        public_key: &[u8; 32],
        envelope: &[u8],
        epoch: u64,
        signature: &[u8; 64],
        now_ms: u64,
    ) -> Result<UploadReceipt, BackupError> {
        validate_envelope(envelope)?;
        let hash = key_hash(public_key);
        let env_hash = envelope_sha256(envelope);
        verify(
            public_key,
            &upload_message(&hash, &env_hash, epoch),
            signature,
        )?;
        match self.entries.get_mut(&hash) {
            Some(BackupEntry::Deleted(tombstone)) => {
                if epoch <= tombstone.final_epoch {
                    return Err(BackupError::Tombstoned {
                        final_epoch: tombstone.final_epoch,
                    });
                }
                // A fresh, higher-epoch signed upload by the key
                // holder is a new backup, not a replay.
                self.bump_generation(&hash);
                self.entries.insert(
                    hash,
                    BackupEntry::Live(BackupRecord {
                        envelope: envelope.to_vec(),
                        envelope_sha256: env_hash,
                        blob_size: envelope.len() as u64,
                        uploaded_at_ms: now_ms,
                        last_downloaded_at_ms: None,
                        epoch,
                        throttle_counters: DownloadBudget::new(now_ms),
                        restore_epoch: 0,
                        tombstone: None,
                    }),
                );
            }
            Some(BackupEntry::Live(record)) => {
                if epoch <= record.epoch {
                    return Err(BackupError::StaleEpoch {
                        current: record.epoch,
                        presented: epoch,
                    });
                }
                record.envelope = envelope.to_vec();
                record.envelope_sha256 = env_hash;
                record.blob_size = envelope.len() as u64;
                record.uploaded_at_ms = now_ms;
                record.epoch = epoch;
            }
            None => {
                self.bump_generation(&hash);
                self.entries.insert(
                    hash,
                    BackupEntry::Live(BackupRecord {
                        envelope: envelope.to_vec(),
                        envelope_sha256: env_hash,
                        blob_size: envelope.len() as u64,
                        uploaded_at_ms: now_ms,
                        last_downloaded_at_ms: None,
                        epoch,
                        throttle_counters: DownloadBudget::new(now_ms),
                        restore_epoch: 0,
                        tombstone: None,
                    }),
                );
            }
        }
        Ok(UploadReceipt {
            key_hash: hash,
            epoch,
            envelope_sha256: env_hash,
        })
    }

    fn live_mut(&mut self, hash: &[u8; 32]) -> Result<&mut BackupRecord, BackupError> {
        match self.entries.get_mut(hash) {
            Some(BackupEntry::Live(record)) => Ok(record),
            Some(BackupEntry::Deleted(tombstone)) => Err(BackupError::Tombstoned {
                final_epoch: tombstone.final_epoch,
            }),
            None => Err(BackupError::NotFound),
        }
    }

    /// Download endpoint: enforce the per-identity download budget,
    /// then return the envelope bytes. Decryption is client-side.
    pub fn download(&mut self, public_key: &[u8; 32], now_ms: u64) -> Result<Vec<u8>, BackupError> {
        let hash = key_hash(public_key);
        let record = self.live_mut(&hash)?;
        record.throttle_counters.try_admit(now_ms)?;
        record.last_downloaded_at_ms = Some(now_ms);
        Ok(record.envelope.clone())
    }

    /// Download-for-restore: like [`BackupStore::download`], plus the
    /// item-5 bookkeeping — `restore_epoch` increments and every
    /// bound device is notified before the restored key may be
    /// treated as authoritative for new group epochs (see
    /// [`BackupStore::is_authoritative`]).
    pub fn download_for_restore(
        &mut self,
        public_key: &[u8; 32],
        now_ms: u64,
    ) -> Result<RestoreReceipt, BackupError> {
        let hash = key_hash(public_key);
        let devices = self.bound_devices.get(&hash).cloned().unwrap_or_default();
        let record = self.live_mut(&hash)?;
        record.throttle_counters.try_admit(now_ms)?;
        record.last_downloaded_at_ms = Some(now_ms);
        record.restore_epoch = record.restore_epoch.saturating_add(1);
        let restore_epoch = record.restore_epoch;
        let envelope = record.envelope.clone();
        let token = RestoreToken {
            generation: self.generation(&hash),
            restore_epoch,
        };
        let notifications: Vec<DeviceNotification> = devices
            .iter()
            .map(|device| DeviceNotification {
                key_hash: hash,
                device_public_key: *device,
                generation: token.generation,
                restore_epoch,
            })
            .collect();
        if !notifications.is_empty() {
            self.restore_pending.insert(hash, token);
        }
        Ok(RestoreReceipt {
            envelope,
            restore_epoch,
            token,
            notifications,
        })
    }

    /// Delete endpoint: signed by the identity key at the record's
    /// current epoch. Drops the ciphertext and counters and leaves
    /// only the tombstone.
    pub fn delete(
        &mut self,
        public_key: &[u8; 32],
        signature: &[u8; 64],
    ) -> Result<Tombstone, BackupError> {
        let hash = key_hash(public_key);
        let current_epoch = match self.entries.get(&hash) {
            Some(BackupEntry::Live(record)) => record.epoch,
            Some(BackupEntry::Deleted(tombstone)) => {
                return Err(BackupError::Tombstoned {
                    final_epoch: tombstone.final_epoch,
                });
            }
            None => return Err(BackupError::NotFound),
        };
        verify(public_key, &delete_message(&hash, current_epoch), signature)?;
        let tombstone = Tombstone {
            key_hash: hash,
            final_epoch: current_epoch,
        };
        self.entries.insert(hash, BackupEntry::Deleted(tombstone));
        self.restore_pending.remove(&hash);
        Ok(tombstone)
    }

    /// Account deletion: purge the tombstone itself.
    pub fn purge_tombstone(&mut self, public_key: &[u8; 32]) -> bool {
        let hash = key_hash(public_key);
        if matches!(self.entries.get(&hash), Some(BackupEntry::Deleted(_))) {
            self.entries.remove(&hash);
            self.bound_devices.remove(&hash);
            self.restore_pending.remove(&hash);
            true
        } else {
            false
        }
    }

    /// Register a bound device for restore notifications.
    pub fn bind_device(&mut self, public_key: &[u8; 32], device_public_key: [u8; 32]) {
        let hash = key_hash(public_key);
        let devices = self.bound_devices.entry(hash).or_default();
        if !devices.contains(&device_public_key) {
            devices.push(device_public_key);
        }
    }

    /// Confirm the restore notifications for the restore named by
    /// `token` were delivered; the restored key becomes
    /// authoritative. A confirmation naming an older restore does NOT
    /// clear a newer restore's pending notifications (Codex review
    /// on PR #7658): the caller must present the exact token its
    /// notifications belong to, as carried on the RestoreReceipt and
    /// each DeviceNotification. The generation half keeps tokens
    /// unique across a delete + recreate, where the recreated
    /// backup's restore_epoch restarts at 1.
    pub fn confirm_restore_notifications(
        &mut self,
        public_key: &[u8; 32],
        token: RestoreToken,
    ) -> bool {
        let hash = key_hash(public_key);
        if self.restore_pending.get(&hash) == Some(&token) {
            self.restore_pending.remove(&hash);
            true
        } else {
            false
        }
    }

    /// Whether the identity's key may be treated as authoritative
    /// for new group epochs: a live backup exists and no restore
    /// notification is still pending.
    pub fn is_authoritative(&self, public_key: &[u8; 32]) -> bool {
        let hash = key_hash(public_key);
        matches!(self.entries.get(&hash), Some(BackupEntry::Live(_)))
            && !self.restore_pending.contains_key(&hash)
    }

    /// The live record for an identity, if one exists.
    pub fn record(&self, public_key: &[u8; 32]) -> Option<&BackupRecord> {
        match self.entries.get(&key_hash(public_key)) {
            Some(BackupEntry::Live(record)) => Some(record),
            _ => None,
        }
    }

    /// The tombstone for an identity, if it was deleted.
    pub fn tombstone(&self, public_key: &[u8; 32]) -> Option<Tombstone> {
        match self.entries.get(&key_hash(public_key)) {
            Some(BackupEntry::Deleted(tombstone)) => Some(*tombstone),
            _ => None,
        }
    }
}

/// A freshly created client-side backup: the v1 envelope to upload,
/// plus the weak-passphrase warning when the creation passphrase is
/// weak (#5033 item 3's guessing-cost analysis: the Argon2id floor
/// and the server download budget are the only barriers, so the
/// passphrase itself must carry the entropy).
pub struct CreatedBackup {
    /// The v1 key envelope bytes, ready to upload.
    pub envelope: Vec<u8>,
    /// Set when the passphrase is weak; the client surfaces this at
    /// creation time.
    pub passphrase_warning: Option<&'static str>,
}

/// The weak-passphrase warning text shown at backup creation.
pub const WEAK_PASSPHRASE_WARNING: &str =
    "weak passphrase: a backup can be attacked offline, so use a long, unique passphrase";

/// Assess a creation passphrase. Weak means short, or built from a
/// single character class — the cases where the Argon2id
/// guessing-cost floor is the only real barrier.
pub fn passphrase_warning(passphrase: &[u8]) -> Option<&'static str> {
    let text = String::from_utf8_lossy(passphrase);
    let classes = [
        text.chars().any(|c| c.is_lowercase()),
        text.chars().any(|c| c.is_uppercase()),
        text.chars().any(|c| c.is_ascii_digit()),
        text.chars().any(|c| !c.is_alphanumeric()),
    ]
    .iter()
    .filter(|present| **present)
    .count();
    if text.chars().count() < 12 || (classes < 2 && text.chars().count() < 20) {
        Some(WEAK_PASSPHRASE_WARNING)
    } else {
        None
    }
}

/// Client create flow: wrap the identity key with
/// `key_envelope::encrypt` exactly — no second format — and report
/// the weak-passphrase warning alongside.
pub fn create_backup(passphrase: &[u8], identity_key: &[u8; 32]) -> anyhow::Result<CreatedBackup> {
    Ok(CreatedBackup {
        envelope: key_envelope::encrypt(passphrase, identity_key)?,
        passphrase_warning: passphrase_warning(passphrase),
    })
}

/// Client restore flow: recover the identity key from the passphrase
/// and the downloaded envelope, using only the existing
/// `key_envelope::decrypt` path. A wrong passphrase fails here,
/// locally — the server is never signalled.
pub fn restore_identity_key(passphrase: &[u8], envelope: &[u8]) -> anyhow::Result<[u8; 32]> {
    key_envelope::decrypt(passphrase, envelope)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const PASSPHRASE: &[u8] = b"correct horse battery staple";

    fn identity(seed: u8) -> (SigningKey, [u8; 32], [u8; 32]) {
        let signing = SigningKey::from_bytes(&[seed; 32]);
        let public = signing.verifying_key().to_bytes();
        (signing, public, [seed; 32])
    }

    fn envelope_for(secret: &[u8; 32]) -> Vec<u8> {
        key_envelope::encrypt(PASSPHRASE, secret).unwrap()
    }

    fn uploaded_store() -> (BackupStore, SigningKey, [u8; 32], Vec<u8>) {
        let (signing, public, secret) = identity(7);
        let envelope = envelope_for(&secret);
        let mut store = BackupStore::new();
        let sig = sign_upload(&signing, &envelope, 1);
        store.upload(&public, &envelope, 1, &sig, 1_000).unwrap();
        (store, signing, public, envelope)
    }

    /// Oracle: #5033 item 2's field list, transcribed from the
    /// decision. The stored field set must equal it exactly.
    #[test]
    fn stored_field_set_matches_the_5033_item_2_list() {
        assert_eq!(
            BackupRecord::stored_field_names(),
            &[
                "envelope",
                "envelope_sha256",
                "blob_size",
                "uploaded_at_ms",
                "last_downloaded_at_ms",
                "epoch",
                "throttle_counters",
                "restore_epoch",
                "tombstone",
            ]
        );
        // No rotation flag is introduced (#2380 owns that
        // representation).
        assert!(
            BackupRecord::stored_field_names()
                .iter()
                .all(|name| !name.contains("rotation"))
        );
    }

    /// Oracle: #5033 item 4 — bad signatures and non-increasing
    /// epochs are rejected; a higher-epoch signed upload replaces.
    #[test]
    fn upload_rejects_bad_signature_and_stale_epochs_and_replaces_on_higher() {
        let (signing, public, secret) = identity(3);
        let envelope = envelope_for(&secret);
        let mut store = BackupStore::new();

        let bad_sig = sign_upload(&SigningKey::from_bytes(&[9; 32]), &envelope, 1);
        assert_eq!(
            store.upload(&public, &envelope, 1, &bad_sig, 0),
            Err(BackupError::BadSignature)
        );

        let sig = sign_upload(&signing, &envelope, 1);
        store.upload(&public, &envelope, 1, &sig, 0).unwrap();

        // Equal and lower epochs are both non-increasing.
        let sig_equal = sign_upload(&signing, &envelope, 1);
        assert_eq!(
            store.upload(&public, &envelope, 1, &sig_equal, 10),
            Err(BackupError::StaleEpoch {
                current: 1,
                presented: 1
            })
        );
        let other = envelope_for(&[4; 32]);
        let sig_higher = sign_upload(&signing, &other, 2);
        let receipt = store.upload(&public, &other, 2, &sig_higher, 20).unwrap();
        assert_eq!(receipt.epoch, 2);
        assert_eq!(store.record(&public).unwrap().envelope(), other.as_slice());
    }

    /// Oracle: the acceptance list — malformed blobs (wrong length,
    /// bad magic, wrong version) are refused with the named error.
    #[test]
    fn upload_rejects_malformed_envelopes() {
        let (signing, public, secret) = identity(5);
        let envelope = envelope_for(&secret);
        let mut store = BackupStore::new();
        let sig = sign_upload(&signing, &envelope, 1);

        assert!(matches!(
            store.upload(&public, &envelope[..100], 1, &sig, 0),
            Err(BackupError::MalformedEnvelope(_))
        ));
        let mut bad_magic = envelope.clone();
        bad_magic[0] = b'X';
        assert!(matches!(
            store.upload(&public, &bad_magic, 1, &sig, 0),
            Err(BackupError::MalformedEnvelope(_))
        ));
        let mut bad_version = envelope;
        bad_version[ENVELOPE_VERSION_OFFSET] = 2;
        assert!(matches!(
            store.upload(&public, &bad_version, 1, &sig, 0),
            Err(BackupError::MalformedEnvelope(_))
        ));
    }

    /// Oracle: #5033 item 3 — the 11th download within an hour and
    /// the 51st within a day are refused, with a retry-after.
    #[test]
    fn download_budget_refuses_the_11th_hourly_and_51st_daily() {
        let (mut store, _signing, public, _envelope) = uploaded_store();
        let t0 = 10_000_000;
        for i in 0..DOWNLOAD_HOUR_LIMIT {
            store.download(&public, t0 + i).unwrap();
        }
        match store.download(&public, t0 + 100) {
            Err(BackupError::DownloadBudgetExceeded { retry_after_ms }) => {
                assert!(retry_after_ms > 0);
            }
            other => panic!("11th hourly download must be refused, got {other:?}"),
        }

        // Fresh store: five batches of 10, each two hours apart so
        // the hour window fully ages out, all inside one day.
        let (mut store, _s, public, _e) = uploaded_store();
        for batch in 0..5u64 {
            let base = t0 + batch * 2 * DOWNLOAD_HOUR_WINDOW_MS;
            for i in 0..DOWNLOAD_HOUR_LIMIT {
                store.download(&public, base + i).unwrap();
            }
        }
        // Two hours past the last batch: the hour budget has room
        // again, the day budget (50 admitted) does not.
        let t51 = t0 + 10 * DOWNLOAD_HOUR_WINDOW_MS;
        match store.download(&public, t51) {
            Err(BackupError::DownloadBudgetExceeded { retry_after_ms }) => {
                assert!(retry_after_ms > 0);
            }
            other => panic!("51st daily download must be refused, got {other:?}"),
        }
    }

    /// Regression test for the Codex finding on PR #7658
    /// (4506243710), oracle = the weighted estimator itself: ten
    /// downloads at t0 exhaust the hour budget; a refusal at
    /// t0 + 1_000 must advertise the next admissible instant
    /// (one millisecond past the window boundary, where the
    /// previous window's weighted contribution drops to 9), not
    /// the fixed-window boundary — and a download at the advertised
    /// instant must actually be admitted.
    #[test]
    fn download_budget_retry_after_is_the_next_admissible_instant() {
        let (mut store, _signing, public, _envelope) = uploaded_store();
        let t0 = 20_000_000;
        for _ in 0..DOWNLOAD_HOUR_LIMIT {
            store.download(&public, t0).unwrap();
        }
        let refused_at = t0 + 1_000;
        match store.download(&public, refused_at) {
            Err(BackupError::DownloadBudgetExceeded { retry_after_ms }) => {
                assert_eq!(retry_after_ms, DOWNLOAD_HOUR_WINDOW_MS - 1_000 + 1);
                store
                    .download(&public, refused_at + retry_after_ms)
                    .unwrap();
            }
            other => panic!("11th hourly download must be refused, got {other:?}"),
        }
    }

    proptest! {
        /// Property test, oracle = the budget invariant: the
        /// weighted estimator never under-counts the current fixed
        /// window, so the number admitted inside the budget's first
        /// fixed hour window (anchored at the first admitted
        /// download, where the window rolls to) never exceeds the
        /// hour limit, and inside the first fixed day window never
        /// exceeds the day limit — for arbitrary interleavings of
        /// monotone attempt times.
        #[test]
        fn download_budget_never_admits_past_the_estimator(
            offsets in proptest::collection::vec(0u64..3_600_000, 0..80),
        ) {
            let (mut store, _s, public, _e) = uploaded_store();
            let mut now = 50_000_000u64;
            let mut anchor: Option<u64> = None;
            let mut admitted_in_first_hour = 0u64;
            let mut admitted_in_first_day = 0u64;
            // The day window is anchored at upload time (1_000 in
            // uploaded_store) and does not roll before the attempts
            // start; the hour window ages out long before, so its
            // anchor is the first admitted download.
            let day_window_end = 1_000 + DOWNLOAD_DAY_WINDOW_MS;
            for off in offsets {
                now += off;
                if store.download(&public, now).is_ok() {
                    let start = *anchor.get_or_insert(now);
                    if now < start + DOWNLOAD_HOUR_WINDOW_MS {
                        admitted_in_first_hour += 1;
                    }
                    if now < day_window_end {
                        admitted_in_first_day += 1;
                    }
                }
            }
            prop_assert!(admitted_in_first_hour <= DOWNLOAD_HOUR_LIMIT);
            prop_assert!(admitted_in_first_day <= DOWNLOAD_DAY_LIMIT);
        }
    }

    /// Oracle: the restore acceptance item — passphrase + downloaded
    /// envelope recovers the key through the existing decrypt path;
    /// a wrong passphrase fails locally.
    #[test]
    fn restore_round_trip_recovers_the_key_and_wrong_passphrase_fails_locally() {
        let (mut store, _s, public, _e) = uploaded_store();
        let downloaded = store.download(&public, 5_000).unwrap();
        let created = create_backup(PASSPHRASE, &[7; 32]).unwrap();
        assert_eq!(created.envelope.len(), BACKUP_ENVELOPE_LEN);
        let recovered = restore_identity_key(PASSPHRASE, &downloaded).unwrap();
        assert_eq!(recovered, [7; 32]);
        assert!(restore_identity_key(b"wrong passphrase", &downloaded).is_err());
    }

    /// Oracle: #5033 item 5 — restore increments `restore_epoch`,
    /// notifies bound devices, and the key is not authoritative
    /// until the notifications are confirmed.
    #[test]
    fn restore_increments_epoch_and_notifies_bound_devices() {
        let (mut store, _s, public, _e) = uploaded_store();
        store.bind_device(&public, [42; 32]);
        assert!(store.is_authoritative(&public));
        let receipt = store.download_for_restore(&public, 5_000).unwrap();
        assert_eq!(receipt.restore_epoch, 1);
        assert_eq!(receipt.notifications.len(), 1);
        assert_eq!(receipt.notifications[0].device_public_key, [42; 32]);
        assert_eq!(receipt.notifications[0].restore_epoch, 1);
        assert_eq!(receipt.notifications[0].generation, 1);
        assert_eq!(receipt.token.generation, 1);
        assert!(!store.is_authoritative(&public));
        assert!(store.confirm_restore_notifications(&public, receipt.token));
        assert!(store.is_authoritative(&public));
        assert_eq!(store.record(&public).unwrap().restore_epoch(), 1);
    }

    /// Oracle: Codex review on PR #7658 — a stale confirmation for
    /// restore epoch 1 must not clear restore epoch 2's pending
    /// notifications; only the current epoch's confirmation restores
    /// authority.
    #[test]
    fn stale_restore_confirmation_does_not_clear_newer_restore() {
        let (mut store, _s, public, _e) = uploaded_store();
        store.bind_device(&public, [42; 32]);
        let first = store.download_for_restore(&public, 5_000).unwrap();
        assert_eq!(first.restore_epoch, 1);
        let second = store.download_for_restore(&public, 6_000).unwrap();
        assert_eq!(second.restore_epoch, 2);
        assert!(!store.is_authoritative(&public));
        assert!(!store.confirm_restore_notifications(&public, first.token));
        assert!(!store.is_authoritative(&public));
        assert!(store.confirm_restore_notifications(&public, second.token));
        assert!(store.is_authoritative(&public));
    }

    /// Oracle: Codex review on PR #7658 — a delayed confirmation
    /// from a deleted backup must not clear a recreated backup's
    /// pending notifications: recreation restarts `restore_epoch`
    /// at 1, so the token's generation half is what keeps the old
    /// confirmation (generation 1, epoch 1) from confirming the new
    /// backup's restore (generation 2, epoch 1).
    #[test]
    fn stale_restore_token_does_not_confirm_recreated_backup() {
        let (mut store, signing, public, envelope) = uploaded_store();
        store.bind_device(&public, [42; 32]);
        let first = store.download_for_restore(&public, 5_000).unwrap();
        assert_eq!(
            first.token,
            RestoreToken {
                generation: 1,
                restore_epoch: 1
            }
        );
        // Delete while the first restore's notifications are still
        // delivering, then recreate at a higher upload epoch.
        let delete_sig = sign_delete(&signing, 1);
        store.delete(&public, &delete_sig).unwrap();
        let recreate_sig = sign_upload(&signing, &envelope, 2);
        store
            .upload(&public, &envelope, 2, &recreate_sig, 6_000)
            .unwrap();
        let second = store.download_for_restore(&public, 7_000).unwrap();
        assert_eq!(second.restore_epoch, 1);
        assert_eq!(
            second.token,
            RestoreToken {
                generation: 2,
                restore_epoch: 1
            }
        );
        assert!(!store.is_authoritative(&public));
        // The delayed confirmation from the deleted backup arrives.
        assert!(!store.confirm_restore_notifications(&public, first.token));
        assert!(!store.is_authoritative(&public));
        assert!(store.confirm_restore_notifications(&public, second.token));
        assert!(store.is_authoritative(&public));
    }

    /// Oracle: Codex review on PR #7658 — a correctly signed envelope
    /// whose header the decrypt path would reject (bad KDF/cipher id,
    /// out-of-range KDF parameters) must be refused at upload, so it
    /// cannot replace a working backup with an unrestorable one.
    #[test]
    fn upload_rejects_envelope_headers_decrypt_would_reject() {
        let (signing, public, secret) = identity(11);
        let envelope = envelope_for(&secret);
        let mut store = BackupStore::new();
        for mutate in [
            |e: &mut Vec<u8>| e[6] = 9,                                      // KDF id
            |e: &mut Vec<u8>| e[7] = 9,                                      // cipher id
            |e: &mut Vec<u8>| e[8..12].copy_from_slice(&1u32.to_le_bytes()), // memory too low
            |e: &mut Vec<u8>| e[12..16].copy_from_slice(&99u32.to_le_bytes()), // iterations too high
            |e: &mut Vec<u8>| e[16..20].copy_from_slice(&0u32.to_le_bytes()),  // lanes too low
        ] {
            let mut bad = envelope.clone();
            mutate(&mut bad);
            let sig = sign_upload(&signing, &bad, 1);
            assert!(matches!(
                store.upload(&public, &bad, 1, &sig, 0),
                Err(BackupError::MalformedEnvelope(_))
            ));
        }
        assert!(store.record(&public).is_none());
    }

    /// Oracle: the delete acceptance item — a signed delete removes
    /// the ciphertext and replaying the pre-delete signed upload
    /// does not resurrect it.
    #[test]
    fn signed_delete_removes_ciphertext_and_replay_does_not_resurrect() {
        let (mut store, signing, public, envelope) = uploaded_store();
        // An unsigned/wrongly-signed delete is refused.
        let bad = sign_delete(&SigningKey::from_bytes(&[8; 32]), 1);
        assert_eq!(store.delete(&public, &bad), Err(BackupError::BadSignature));
        assert!(store.record(&public).is_some());

        let sig = sign_delete(&signing, 1);
        let tombstone = store.delete(&public, &sig).unwrap();
        assert_eq!(tombstone.final_epoch, 1);
        assert!(store.record(&public).is_none());
        assert_eq!(store.tombstone(&public), Some(tombstone));

        // Replay: the exact pre-delete upload, signature and all.
        let replay_sig = sign_upload(&signing, &envelope, 1);
        assert_eq!(
            store.upload(&public, &envelope, 1, &replay_sig, 9_999),
            Err(BackupError::Tombstoned { final_epoch: 1 })
        );
        assert!(store.record(&public).is_none());
        // Downloads of a deleted backup name the tombstone too.
        assert_eq!(
            store.download(&public, 10_000),
            Err(BackupError::Tombstoned { final_epoch: 1 })
        );
        // Account deletion purges the tombstone.
        assert!(store.purge_tombstone(&public));
        assert_eq!(store.download(&public, 10_000), Err(BackupError::NotFound));
    }

    #[test]
    fn weak_passphrases_are_warned_at_creation() {
        assert_eq!(
            create_backup(b"short", &[1; 32])
                .unwrap()
                .passphrase_warning,
            Some(WEAK_PASSPHRASE_WARNING)
        );
        assert_eq!(
            create_backup(PASSPHRASE, &[1; 32])
                .unwrap()
                .passphrase_warning,
            None
        );
    }

    #[test]
    fn unknown_identity_is_not_found() {
        let mut store = BackupStore::new();
        let (_s, public, _secret) = identity(11);
        assert_eq!(store.download(&public, 0), Err(BackupError::NotFound));
        assert_eq!(key_hash_hex(&public).len(), 64);
    }
}
