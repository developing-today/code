//! The directory's relay duty for guardian recovery (#2382).
//!
//! The server role is relay and timestamp only. A [`RecoveryRelay`]
//! keeps every recovery annex record and every keyset record it is
//! handed, and attests the time it first saw each one. It decides
//! nothing: [`crate::recovery::RecoveryEvidence`] weighs what it
//! relays, so the relay's only power is time.
//!
//! That power is bounded in the safe direction. Every record must
//! verify before it is kept, so the relay cannot forge an approval or a
//! request. A first-seen time is written once and never moves, so
//! sending a record again cannot shorten a delay. A request is never
//! attested before its own `created_at`, matching the verifier's
//! `max(created_at, first_seen)` rule. A relay that withholds a record
//! can only lengthen the delay. Records persist as JSON lines in one
//! append-only file, so the attested times survive a restart.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::keyset::KeysetRecord;
use crate::keyset_file::{append_line, read_complete};
use crate::recovery::{AnnexEntry, AnnexRecord, Seen};

/// A record the relay carries: a recovery annex record or a keyset
/// chain record (such as a guardian-set change or a recovery rotation).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "record", rename_all = "snake_case")]
pub enum RelayedRecord {
    /// A request, approval or cancellation.
    Annex(AnnexRecord),
    /// A keyset chain record.
    Keyset(KeysetRecord),
}

impl RelayedRecord {
    /// The record's content hash.
    #[must_use]
    pub fn hash(&self) -> &str {
        match self {
            Self::Annex(record) => &record.hash,
            Self::Keyset(record) => &record.hash,
        }
    }

    /// Verifies the record's hash and signature.
    ///
    /// # Errors
    ///
    /// Fails when the record does not verify.
    pub fn verify(&self) -> Result<()> {
        match self {
            Self::Annex(record) => record.verify(),
            Self::Keyset(record) => record.verify(),
        }
    }

    /// The earliest time an honest relay can attest: a request's own
    /// `created_at`, and no floor for anything else.
    const fn floor(&self) -> u64 {
        match self {
            Self::Annex(AnnexRecord {
                entry: AnnexEntry::RecoveryRequest { created_at, .. },
                ..
            }) => *created_at,
            _ => 0,
        }
    }
}

/// A relayed record with the time the relay first saw it (unix seconds).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relayed {
    /// When the relay first saw the record.
    pub first_seen: u64,
    /// The record.
    pub record: RelayedRecord,
}

/// What relaying one record did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Relay {
    /// The attested first-seen time, unchanged if the record was known.
    pub first_seen: u64,
    /// Whether the record was new to this relay, so its notifications
    /// are still owed.
    pub new: bool,
}

/// The relay's store: every record it has seen, in arrival order.
#[derive(Debug)]
pub struct RecoveryRelay {
    path: PathBuf,
    relayed: Vec<Relayed>,
    index: BTreeMap<String, usize>,
}

impl RecoveryRelay {
    /// Opens the relay store at `path`, which may not exist yet.
    ///
    /// # Errors
    ///
    /// Fails if the file cannot be read or holds a line that is not a
    /// verifying record.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let mut relay = Self {
            relayed: Vec::new(),
            index: BTreeMap::new(),
            path,
        };
        let text = read_complete(&relay.path)?;
        for (index, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let relayed: Relayed = serde_json::from_str(line).with_context(|| {
                format!(
                    "{} line {}: not a relayed record",
                    relay.path.display(),
                    index + 1
                )
            })?;
            relayed.record.verify().with_context(|| {
                format!(
                    "{} line {}: record does not verify",
                    relay.path.display(),
                    index + 1
                )
            })?;
            relay.remember(relayed);
        }
        Ok(relay)
    }

    /// Relays `record`, first seen at `now` unless the relay already
    /// holds it. A request is never attested before its `created_at`.
    ///
    /// # Errors
    ///
    /// Fails if the record does not verify or cannot be written.
    pub fn relay(&mut self, record: RelayedRecord, now: u64) -> Result<Relay> {
        record
            .verify()
            .context("refusing to relay a record that does not verify")?;
        if let Some(first_seen) = self.first_seen(record.hash()) {
            return Ok(Relay {
                first_seen,
                new: false,
            });
        }
        let relayed = Relayed {
            first_seen: now.max(record.floor()),
            record,
        };
        append_line(&self.path, &relayed)?;
        let first_seen = relayed.first_seen;
        self.remember(relayed);
        Ok(Relay {
            first_seen,
            new: true,
        })
    }

    /// When the relay first saw the record with this hash.
    #[must_use]
    pub fn first_seen(&self, hash: &str) -> Option<u64> {
        self.index.get(hash).map(|at| self.relayed[*at].first_seen)
    }

    /// Every annex record for `identity`, with its first-seen time.
    #[must_use]
    pub fn annex(&self, identity: &str) -> Vec<Seen<'_>> {
        self.relayed
            .iter()
            .filter_map(|relayed| match &relayed.record {
                RelayedRecord::Annex(record) if record.identity_id() == identity => Some(Seen {
                    record,
                    first_seen: relayed.first_seen,
                }),
                _ => None,
            })
            .collect()
    }

    /// The first-seen time of every keyset record, by hash.
    #[must_use]
    pub fn keyset_first_seen(&self) -> BTreeMap<String, u64> {
        self.relayed
            .iter()
            .filter(|relayed| matches!(relayed.record, RelayedRecord::Keyset(_)))
            .map(|relayed| (relayed.record.hash().to_owned(), relayed.first_seen))
            .collect()
    }

    /// Every keyset record the relay holds, in arrival order.
    pub fn keyset_records(&self) -> impl Iterator<Item = &KeysetRecord> {
        self.relayed
            .iter()
            .filter_map(|relayed| match &relayed.record {
                RelayedRecord::Keyset(record) => Some(record),
                RelayedRecord::Annex(_) => None,
            })
    }

    /// Keeps the first sighting of each hash.
    fn remember(&mut self, relayed: Relayed) {
        if self.index.contains_key(relayed.record.hash()) {
            return;
        }
        self.index
            .insert(relayed.record.hash().to_owned(), self.relayed.len());
        self.relayed.push(relayed);
    }
}
