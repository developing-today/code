//! Gallery admission and its public transparency list (#10005; v3 design
//! `doc/2026-11-14T13-33-00Z_design_id_program_ecosystem_v3/`, G6).
//!
//! Validity is protocol, admission is policy (D6). Whether a verified record
//! is listed by *this* gallery is the operator's decision, and it binds only
//! this endpoint: a removed record keeps verifying, keeps its bundle file
//! here, and stays listed on any other host. What this module adds is that
//! the decision is never silent. Every removal, every refusal and every
//! reinstatement is appended to a [`TransparencyList`] that the gallery
//! serves publicly: the record's identity, when, and one of four reason
//! classes ([`ReasonClass`]).
//!
//! The default posture and the abuse response times are operator rulings,
//! so both are configuration ([`AdmissionConfig`], stored as
//! `operator/admission.json`), never constants:
//!
//! - [`AdmissionPosture::Open`]: every verified record is listed until it is
//!   removed.
//! - [`AdmissionPosture::Reviewed`]: a record is listed once the operator
//!   approves it; until then it is pending, and the operator may refuse it
//!   (which is public, like a removal).
//!
//! Switching posture changes what is listed at once, because a record's
//! visibility is computed from the posture, the approvals and the list.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::program_bundle::{ProgramIdentity, check_identity};

/// Longest public note on one transparency entry, in bytes.
pub const MAX_NOTE_BYTES: usize = 500;

/// Most transparency entries one page of the list returns.
pub const MAX_TRANSPARENCY_PAGE: usize = 500;

/// File, under the operator directory, holding the admission configuration.
pub const ADMISSION_FILE: &str = "admission.json";

/// File, under the operator directory, holding the transparency list as one
/// JSON entry per line, appended in sequence order.
pub const TRANSPARENCY_FILE: &str = "transparency.jsonl";

/// File, under the operator directory, holding the approved content hashes.
pub const APPROVED_FILE: &str = "approved.json";

/// Why the operator stopped listing a record (G6). The classes are coarse
/// on purpose: the list is auditable without publishing the details of an
/// abuse report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonClass {
    /// The content is unlawful where the operator runs the endpoint.
    IllegalContent,
    /// The program is malicious.
    Malware,
    /// The record is spam.
    Spam,
    /// The record breaks the operator's published policy.
    Policy,
}

impl ReasonClass {
    /// Every class, in the documented order.
    pub const ALL: [Self; 4] = [
        Self::IllegalContent,
        Self::Malware,
        Self::Spam,
        Self::Policy,
    ];

    /// The class's wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IllegalContent => "illegal_content",
            Self::Malware => "malware",
            Self::Spam => "spam",
            Self::Policy => "policy",
        }
    }
}

impl std::fmt::Display for ReasonClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ReasonClass {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|class| class.as_str() == text.replace('-', "_"))
            .ok_or_else(|| {
                format!(
                    "reason class is one of illegal_content, malware, spam or policy, not {text:?}"
                )
            })
    }
}

/// Which records the gallery lists before the operator has looked at them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionPosture {
    /// Every verified record is listed until it is removed.
    #[default]
    Open,
    /// A record is listed once the operator approves it.
    Reviewed,
}

impl AdmissionPosture {
    /// The posture's wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Reviewed => "reviewed",
        }
    }
}

impl FromStr for AdmissionPosture {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "open" => Ok(Self::Open),
            "reviewed" => Ok(Self::Reviewed),
            _ => Err(format!("posture is open or reviewed, not {text:?}")),
        }
    }
}

/// The operator's admission configuration, published with the list.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionConfig {
    /// Whether records are listed before review.
    #[serde(default)]
    pub posture: AdmissionPosture,
    /// The operator's stated response time for each reason class, in hours.
    /// A class without an entry has no stated commitment.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub response_hours: BTreeMap<ReasonClass, u32>,
}

/// What an operator did to a record's listing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionAction {
    /// A listed record stopped being listed.
    Removed,
    /// A pending record was refused (reviewed posture).
    Refused,
    /// A removal or refusal was reversed, on appeal or otherwise.
    Reinstated,
}

/// One public admission decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransparencyEntry {
    /// Position in the list, counting from 1 with no gaps.
    pub sequence: u64,
    /// When the operator recorded it, in Unix seconds by the operator's
    /// clock.
    pub at: u64,
    /// What was done.
    pub action: AdmissionAction,
    /// The record it was done to, exactly as signed.
    pub record: ProgramIdentity,
    /// The reason class. A reinstatement carries the class of the decision
    /// it reverses.
    pub reason: ReasonClass,
    /// For a reinstatement, the sequence of the removal or refusal it
    /// reverses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverses: Option<u64>,
    /// The operator's public note, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl TransparencyEntry {
    /// Whether this entry, as a record's latest, keeps it unlisted.
    #[must_use]
    pub const fn hides(&self) -> bool {
        matches!(
            self.action,
            AdmissionAction::Removed | AdmissionAction::Refused
        )
    }
}

/// Why an admission step or admission data was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AdmissionError {
    /// No bundle with this content hash is held here.
    #[error("no bundle {0} is held here")]
    NotFound(String),
    /// The record is not listed, so there is nothing to remove.
    #[error("{0} is not listed")]
    NotListed(String),
    /// The record is not waiting for review.
    #[error("{0} is not pending review")]
    NotPending(String),
    /// The record is not removed or refused, so there is nothing to reverse.
    #[error("{0} is not removed or refused")]
    NotHidden(String),
    /// The note is longer than [`MAX_NOTE_BYTES`].
    #[error("a note is at most {MAX_NOTE_BYTES} bytes")]
    Note,
    /// The stored configuration, list or approvals are malformed.
    #[error("admission data: {0}")]
    Data(String),
    /// Reading or writing admission data failed.
    #[error("admission storage: {0}")]
    Storage(String),
}

/// The append-only public record of admission decisions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransparencyList {
    entries: Vec<TransparencyEntry>,
    latest: BTreeMap<String, usize>,
}

impl TransparencyList {
    /// Rebuild a list from its entries, checking that it is well formed:
    /// sequences count from 1 with no gaps, every identity is canonical,
    /// a removal follows a listing, a refusal follows nothing, and a
    /// reinstatement reverses its record's latest decision with that
    /// decision's reason class.
    ///
    /// # Errors
    ///
    /// The entries do not form such a list.
    pub fn from_entries(
        entries: impl IntoIterator<Item = TransparencyEntry>,
    ) -> Result<Self, AdmissionError> {
        let mut list = Self::default();
        for entry in entries {
            list.check_next(&entry)?;
            list.push(entry);
        }
        Ok(list)
    }

    /// Every entry, in sequence order.
    #[must_use]
    pub fn entries(&self) -> &[TransparencyEntry] {
        &self.entries
    }

    /// The entries after `sequence`, at most `limit` of them (and never
    /// more than [`MAX_TRANSPARENCY_PAGE`]).
    #[must_use]
    pub fn page(&self, after: u64, limit: usize) -> &[TransparencyEntry] {
        let start = usize::try_from(after)
            .unwrap_or(usize::MAX)
            .min(self.entries.len());
        let end = start
            .saturating_add(limit.min(MAX_TRANSPARENCY_PAGE))
            .min(self.entries.len());
        &self.entries[start..end]
    }

    /// The latest decision about the record with `content_hash`.
    #[must_use]
    pub fn latest(&self, content_hash: &str) -> Option<&TransparencyEntry> {
        self.latest
            .get(content_hash)
            .and_then(|&at| self.entries.get(at))
    }

    /// Whether the latest decision about this record keeps it unlisted.
    #[must_use]
    pub fn hides(&self, content_hash: &str) -> bool {
        self.latest(content_hash)
            .is_some_and(TransparencyEntry::hides)
    }

    /// How many entries the list holds.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the list is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn next_sequence(&self) -> u64 {
        u64::try_from(self.entries.len()).map_or(u64::MAX, |n| n.saturating_add(1))
    }

    fn check_next(&self, entry: &TransparencyEntry) -> Result<(), AdmissionError> {
        let data = |why: &str| AdmissionError::Data(format!("entry {}: {why}", entry.sequence));
        if entry.sequence != self.next_sequence() {
            return Err(data("is out of sequence"));
        }
        check_identity(&entry.record).map_err(|e| data(&e))?;
        if entry
            .note
            .as_ref()
            .is_some_and(|n| n.len() > MAX_NOTE_BYTES)
        {
            return Err(data("note is too long"));
        }
        let latest = self.latest(&entry.record.content_hash);
        match entry.action {
            AdmissionAction::Removed | AdmissionAction::Refused => {
                if entry.reverses.is_some() {
                    return Err(data("only a reinstatement reverses an entry"));
                }
                if latest.is_some_and(TransparencyEntry::hides) {
                    return Err(data("the record is already unlisted"));
                }
            }
            AdmissionAction::Reinstated => {
                let Some(latest) = latest.filter(|latest| latest.hides()) else {
                    return Err(data("nothing to reinstate"));
                };
                if entry.reverses != Some(latest.sequence) || entry.reason != latest.reason {
                    return Err(data(
                        "a reinstatement must reverse the record's latest decision",
                    ));
                }
                if entry.record != latest.record {
                    return Err(data("a reinstatement names the record it reverses"));
                }
            }
        }
        Ok(())
    }

    fn push(&mut self, entry: TransparencyEntry) {
        self.latest
            .insert(entry.record.content_hash.clone(), self.entries.len());
        self.entries.push(entry);
    }
}

/// A gallery's admission state: its configuration, its approvals and its
/// transparency list, kept under the operator directory when it has one.
#[derive(Clone, Debug, Default)]
pub struct Admission {
    config: AdmissionConfig,
    approved: BTreeSet<String>,
    list: TransparencyList,
}

impl Admission {
    /// Admission in memory with this configuration and no decisions.
    #[must_use]
    pub fn with_config(config: AdmissionConfig) -> Self {
        Self {
            config,
            ..Self::default()
        }
    }

    /// Load the admission data under `operator_dir`; a missing file means
    /// the default (open posture, nothing approved, an empty list).
    ///
    /// # Errors
    ///
    /// A file cannot be read or is malformed.
    pub fn load(operator_dir: &Path) -> Result<Self, AdmissionError> {
        let mut admission = Self::default();
        let config = operator_dir.join(ADMISSION_FILE);
        if config.is_file() {
            let bytes = std::fs::read(&config).map_err(|e| storage(&config, &e))?;
            admission.config = id_core::json_unique::from_slice_without_duplicate_keys(&bytes)
                .map_err(|e| AdmissionError::Data(format!("{}: {e}", config.display())))?;
        }
        let approved = operator_dir.join(APPROVED_FILE);
        if approved.is_file() {
            let bytes = std::fs::read(&approved).map_err(|e| storage(&approved, &e))?;
            let hashes: Vec<String> =
                id_core::json_unique::from_slice_without_duplicate_keys(&bytes)
                    .map_err(|e| AdmissionError::Data(format!("{}: {e}", approved.display())))?;
            admission.approved = hashes.into_iter().collect();
        }
        let list = operator_dir.join(TRANSPARENCY_FILE);
        if list.is_file() {
            let text = std::fs::read_to_string(&list).map_err(|e| storage(&list, &e))?;
            let entries = text
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| {
                    id_core::json_unique::from_slice_without_duplicate_keys(line.as_bytes())
                        .map_err(|e| AdmissionError::Data(format!("{}: {e}", list.display())))
                })
                .collect::<Result<Vec<TransparencyEntry>, _>>()?;
            admission.list = TransparencyList::from_entries(entries)?;
        }
        Ok(admission)
    }

    /// The configuration in force.
    #[must_use]
    pub const fn config(&self) -> &AdmissionConfig {
        &self.config
    }

    /// The public transparency list.
    #[must_use]
    pub const fn list(&self) -> &TransparencyList {
        &self.list
    }

    /// Whether the record with `content_hash` is listed by this gallery,
    /// given that it holds the bundle.
    #[must_use]
    pub fn lists(&self, content_hash: &str) -> bool {
        !self.list.hides(content_hash)
            && (self.config.posture == AdmissionPosture::Open
                || self.approved.contains(content_hash))
    }

    /// Whether the record is waiting for the operator (reviewed posture).
    #[must_use]
    pub fn is_pending(&self, content_hash: &str) -> bool {
        self.config.posture == AdmissionPosture::Reviewed
            && !self.approved.contains(content_hash)
            && !self.list.hides(content_hash)
    }

    /// Change the configuration, storing it under `operator_dir` if given.
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn set_config(
        &mut self,
        config: AdmissionConfig,
        operator_dir: Option<&Path>,
    ) -> Result<(), AdmissionError> {
        if let Some(dir) = operator_dir {
            write_json(&dir.join(ADMISSION_FILE), &config)?;
        }
        self.config = config;
        Ok(())
    }

    /// Approve a pending record so it is listed. Approval is not a public
    /// decision: listing is what the gallery does by default.
    ///
    /// # Errors
    ///
    /// The record is not pending, or the approvals cannot be stored.
    pub fn approve(
        &mut self,
        content_hash: &str,
        operator_dir: Option<&Path>,
    ) -> Result<(), AdmissionError> {
        if !self.is_pending(content_hash) {
            return Err(AdmissionError::NotPending(content_hash.to_owned()));
        }
        self.approve_unchecked(content_hash, operator_dir)
    }

    /// Record that a listed record is removed from this gallery.
    ///
    /// # Errors
    ///
    /// The record is not listed, the note is too long, or the list cannot
    /// be stored.
    pub fn remove(
        &mut self,
        record: ProgramIdentity,
        reason: ReasonClass,
        at: u64,
        note: Option<String>,
        operator_dir: Option<&Path>,
    ) -> Result<TransparencyEntry, AdmissionError> {
        if !self.lists(&record.content_hash) {
            return Err(AdmissionError::NotListed(record.content_hash));
        }
        self.decide(
            AdmissionAction::Removed,
            record,
            reason,
            at,
            note,
            operator_dir,
        )
    }

    /// Record that a pending record is refused.
    ///
    /// # Errors
    ///
    /// The record is not pending, the note is too long, or the list cannot
    /// be stored.
    pub fn refuse(
        &mut self,
        record: ProgramIdentity,
        reason: ReasonClass,
        at: u64,
        note: Option<String>,
        operator_dir: Option<&Path>,
    ) -> Result<TransparencyEntry, AdmissionError> {
        if !self.is_pending(&record.content_hash) {
            return Err(AdmissionError::NotPending(record.content_hash));
        }
        self.decide(
            AdmissionAction::Refused,
            record,
            reason,
            at,
            note,
            operator_dir,
        )
    }

    /// Reverse the record's removal or refusal, and approve it so it is
    /// listed under either posture.
    ///
    /// # Errors
    ///
    /// The record is not removed or refused, the note is too long, or the
    /// list cannot be stored.
    pub fn reinstate(
        &mut self,
        content_hash: &str,
        at: u64,
        note: Option<String>,
        operator_dir: Option<&Path>,
    ) -> Result<TransparencyEntry, AdmissionError> {
        let Some(latest) = self.list.latest(content_hash).filter(|e| e.hides()) else {
            return Err(AdmissionError::NotHidden(content_hash.to_owned()));
        };
        let (record, reason, reverses) = (latest.record.clone(), latest.reason, latest.sequence);
        let entry = self.append(
            TransparencyEntry {
                sequence: self.list.next_sequence(),
                at,
                action: AdmissionAction::Reinstated,
                record,
                reason,
                reverses: Some(reverses),
                note,
            },
            operator_dir,
        )?;
        self.approve_unchecked(content_hash, operator_dir)?;
        Ok(entry)
    }

    fn decide(
        &mut self,
        action: AdmissionAction,
        record: ProgramIdentity,
        reason: ReasonClass,
        at: u64,
        note: Option<String>,
        operator_dir: Option<&Path>,
    ) -> Result<TransparencyEntry, AdmissionError> {
        self.append(
            TransparencyEntry {
                sequence: self.list.next_sequence(),
                at,
                action,
                record,
                reason,
                reverses: None,
                note,
            },
            operator_dir,
        )
    }

    fn append(
        &mut self,
        entry: TransparencyEntry,
        operator_dir: Option<&Path>,
    ) -> Result<TransparencyEntry, AdmissionError> {
        if entry
            .note
            .as_ref()
            .is_some_and(|n| n.len() > MAX_NOTE_BYTES)
        {
            return Err(AdmissionError::Note);
        }
        self.list.check_next(&entry)?;
        if let Some(dir) = operator_dir {
            append_line(&dir.join(TRANSPARENCY_FILE), &entry)?;
        }
        self.list.push(entry.clone());
        Ok(entry)
    }

    fn approve_unchecked(
        &mut self,
        content_hash: &str,
        operator_dir: Option<&Path>,
    ) -> Result<(), AdmissionError> {
        let mut approved = self.approved.clone();
        if !approved.insert(content_hash.to_owned()) {
            return Ok(());
        }
        if let Some(dir) = operator_dir {
            write_json(&dir.join(APPROVED_FILE), &approved)?;
        }
        self.approved = approved;
        Ok(())
    }
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), AdmissionError> {
    let dir = path
        .parent()
        .ok_or_else(|| AdmissionError::Storage(format!("{} has no parent", path.display())))?;
    std::fs::create_dir_all(dir).map_err(|e| storage(dir, &e))?;
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|e| AdmissionError::Storage(e.to_string()))?;
    let mut staged = tempfile::NamedTempFile::new_in(dir).map_err(|e| storage(dir, &e))?;
    std::io::Write::write_all(&mut staged, &bytes).map_err(|e| storage(path, &e))?;
    staged.as_file().sync_all().map_err(|e| storage(path, &e))?;
    staged.persist(path).map_err(|e| storage(path, &e.error))?;
    Ok(())
}

fn append_line(path: &Path, entry: &TransparencyEntry) -> Result<(), AdmissionError> {
    let dir = path
        .parent()
        .ok_or_else(|| AdmissionError::Storage(format!("{} has no parent", path.display())))?;
    std::fs::create_dir_all(dir).map_err(|e| storage(dir, &e))?;
    let mut line = serde_json::to_vec(entry).map_err(|e| AdmissionError::Storage(e.to_string()))?;
    line.push(b'\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| storage(path, &e))?;
    std::io::Write::write_all(&mut file, &line).map_err(|e| storage(path, &e))?;
    file.sync_all().map_err(|e| storage(path, &e))
}

fn storage(path: &Path, error: &std::io::Error) -> AdmissionError {
    AdmissionError::Storage(format!("{}: {error}", path.display()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn identity(hash_byte: char) -> ProgramIdentity {
        ProgramIdentity {
            author: crate::world_session::encode_hex(
                ed25519_dalek::SigningKey::from_bytes(&[1; 32])
                    .verifying_key()
                    .as_bytes(),
            ),
            name: "chess".to_owned(),
            version: 1,
            content_hash: hash_byte.to_string().repeat(64),
        }
    }

    #[test]
    fn removal_and_reinstatement_are_public_and_reverse_each_other() {
        let root = tempfile::tempdir().unwrap();
        let dir = Some(root.path());
        let mut admission = Admission::default();
        let record = identity('a');
        let hash = record.content_hash.clone();
        assert!(admission.lists(&hash));

        let removed = admission
            .remove(record.clone(), ReasonClass::Malware, 10, None, dir)
            .unwrap();
        assert_eq!(
            (removed.sequence, removed.action),
            (1, AdmissionAction::Removed)
        );
        assert!(!admission.lists(&hash));
        assert_eq!(
            admission.remove(record, ReasonClass::Spam, 11, None, dir),
            Err(AdmissionError::NotListed(hash.clone()))
        );

        let back = admission
            .reinstate(&hash, 20, Some("appeal upheld".to_owned()), dir)
            .unwrap();
        assert_eq!(back.reverses, Some(1));
        assert_eq!(back.reason, ReasonClass::Malware);
        assert!(admission.lists(&hash));
        assert_eq!(
            admission.reinstate(&hash, 21, None, dir),
            Err(AdmissionError::NotHidden(hash.clone()))
        );

        let reloaded = Admission::load(root.path()).unwrap();
        assert_eq!(reloaded.list(), admission.list());
        assert!(reloaded.lists(&hash));
    }

    #[test]
    fn reviewed_posture_lists_only_approved_records_and_switches_at_once() {
        let root = tempfile::tempdir().unwrap();
        let dir = Some(root.path());
        let mut admission = Admission::with_config(AdmissionConfig {
            posture: AdmissionPosture::Reviewed,
            response_hours: BTreeMap::from([(ReasonClass::Malware, 4)]),
        });
        let (a, b) = (identity('a'), identity('b'));
        assert!(admission.is_pending(&a.content_hash));
        assert!(!admission.lists(&a.content_hash));

        admission.approve(&a.content_hash, dir).unwrap();
        assert!(admission.lists(&a.content_hash));
        assert_eq!(
            admission.approve(&a.content_hash, dir),
            Err(AdmissionError::NotPending(a.content_hash.clone()))
        );
        let refused = admission
            .refuse(b.clone(), ReasonClass::Spam, 5, None, dir)
            .unwrap();
        assert_eq!(refused.action, AdmissionAction::Refused);
        assert!(!admission.is_pending(&b.content_hash));

        let open = AdmissionConfig {
            posture: AdmissionPosture::Open,
            ..admission.config().clone()
        };
        admission.set_config(open, dir).unwrap();
        assert!(admission.lists(&identity('c').content_hash));
        assert!(!admission.lists(&b.content_hash), "a refusal still holds");

        let reloaded = Admission::load(root.path()).unwrap();
        assert_eq!(reloaded.config().posture, AdmissionPosture::Open);
        assert_eq!(reloaded.config().response_hours[&ReasonClass::Malware], 4);
        assert!(reloaded.lists(&a.content_hash));
        assert!(!reloaded.lists(&b.content_hash));
    }

    #[test]
    fn a_tampered_list_is_refused_on_load() {
        let mut admission = Admission::default();
        admission
            .remove(identity('a'), ReasonClass::Policy, 1, None, None)
            .unwrap();
        let mut entries = admission.list().entries().to_vec();

        let mut gap = entries.clone();
        gap[0].sequence = 2;
        assert!(TransparencyList::from_entries(gap).is_err());

        entries.push(TransparencyEntry {
            sequence: 2,
            at: 2,
            action: AdmissionAction::Reinstated,
            record: identity('a'),
            reason: ReasonClass::Spam,
            reverses: Some(1),
            note: None,
        });
        assert!(
            TransparencyList::from_entries(entries.clone()).is_err(),
            "a reinstatement keeps the reversed class"
        );
        entries[1].reason = ReasonClass::Policy;
        assert!(TransparencyList::from_entries(entries).is_ok());
    }

    #[test]
    fn notes_are_bounded_and_classes_parse() {
        let mut admission = Admission::default();
        assert_eq!(
            admission.remove(
                identity('a'),
                ReasonClass::Spam,
                1,
                Some("x".repeat(MAX_NOTE_BYTES + 1)),
                None
            ),
            Err(AdmissionError::Note)
        );
        assert!(admission.list().is_empty());
        assert_eq!("illegal-content".parse(), Ok(ReasonClass::IllegalContent));
        assert!("other".parse::<ReasonClass>().is_err());
        assert_eq!("reviewed".parse(), Ok(AdmissionPosture::Reviewed));
        let json = serde_json::to_string(&AdmissionConfig {
            posture: AdmissionPosture::Reviewed,
            response_hours: BTreeMap::from([(ReasonClass::IllegalContent, 24)]),
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"posture":"reviewed","response_hours":{"illegal_content":24}}"#
        );
    }

    #[test]
    fn pages_follow_the_sequence() {
        let mut admission = Admission::default();
        for (i, byte) in ['a', 'b', 'c'].into_iter().enumerate() {
            admission
                .remove(identity(byte), ReasonClass::Spam, i as u64, None, None)
                .unwrap();
        }
        let page = admission.list().page(1, 1);
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].sequence, 2);
        assert_eq!(admission.list().page(3, 10), []);
        assert_eq!(admission.list().page(u64::MAX, 10), []);
    }
}
