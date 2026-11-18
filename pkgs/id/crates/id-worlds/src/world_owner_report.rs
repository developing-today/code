//! The owner report projection for one world, read-only (#11774, design
//! #6308 §§2–4, 8–9).
//!
//! [`project`] streams one world's journal once and returns an
//! [`OwnerReport`]: durable join facts from the journal, the storage
//! inventory the caller measured ([`crate::world_storage`]), and the
//! sections it has no source for yet marked unavailable with a reason.
//! It never writes, never opens a network connection and never loads the
//! whole journal: it keeps one line, a per-participant flag map bounded
//! by [`ReportLimits::max_participants`], and a bounded sample.
//!
//! What the journal can and cannot answer, as of journal version
//! [`crate::world::MAX_JOURNAL_VERSION`]:
//!
//! - A join is an `Issued` entry: a capability the world granted, host
//!   issued or delegated. Compaction rewrites every capability ever
//!   issued (revoked ones with their `Revoked` entry), so the count is
//!   complete across checkpoints.
//! - No journal entry records *when* a capability was issued, so the
//!   join time, the earliest answerable time and any count inside a time
//!   window are [`UnavailableReason::JoinTimeNotRecorded`], never zero.
//!   Recording them is a journal-format change, out of this scope.
//! - Live membership and program runs come from a running world
//!   (#11775); this projection reports them unavailable until a caller
//!   supplies them.
//!
//! `null` plus a reason means unavailable; `0` means measured zero.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt;
use std::io::{self, BufRead, Read};

use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};

use crate::world::{JournalEntry, MAX_JOURNAL_VERSION, journal_version_required_by};
use crate::world_storage::StorageInventory;

/// Major version of the report's JSON shape. Consumers accept added
/// optional fields and refuse any other major version.
pub const OWNER_REPORT_SCHEMA_VERSION: u32 = 1;

/// Why a report value is unavailable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    /// The journal records no time for a capability's issue.
    JoinTimeNotRecorded,
    /// No live participant snapshot was supplied (the world is not
    /// running, or the caller did not ask it).
    LiveStateNotProvided,
    /// No program-run source was supplied; the journal records no runs.
    ProgramRunsNotRecorded,
    /// No storage inventory was supplied.
    StorageNotMeasured,
}

impl UnavailableReason {
    /// The human-readable form the text report prints.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Self::JoinTimeNotRecorded => "the journal does not record when anyone joined",
            Self::LiveStateNotProvided => "no live snapshot (the world is not running here)",
            Self::ProgramRunsNotRecorded => "program runs are not recorded by this journal",
            Self::StorageNotMeasured => "storage was not measured",
        }
    }
}

/// A value that is either measured or unavailable for a stated reason.
/// Serializes as `{"value": v}` or `{"value": null, "reason": "…"}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Measured<T> {
    /// The value, measured.
    Known(T),
    /// No value, and why.
    Unavailable(UnavailableReason),
}

impl<T> Measured<T> {
    /// The value when measured.
    #[must_use]
    pub const fn value(&self) -> Option<&T> {
        match self {
            Self::Known(value) => Some(value),
            Self::Unavailable(_) => None,
        }
    }

    /// Why the value is unavailable, when it is.
    #[must_use]
    pub const fn reason(&self) -> Option<UnavailableReason> {
        match self {
            Self::Known(_) => None,
            Self::Unavailable(reason) => Some(*reason),
        }
    }
}

impl<T: Serialize> Serialize for Measured<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Known(value) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("value", value)?;
                map.end()
            }
            Self::Unavailable(reason) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("value", &None::<()>)?;
                map.serialize_entry("reason", reason)?;
                map.end()
            }
        }
    }
}

/// The requested time window, in unix milliseconds, `from` inclusive and
/// `to` exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ReportWindow {
    /// Start, inclusive.
    pub from_unix_ms: u64,
    /// End, exclusive.
    pub to_unix_ms: u64,
}

/// Bounds on one projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReportLimits {
    /// Longest journal line read, newline included. A longer line is
    /// refused rather than buffered.
    pub max_line_bytes: usize,
    /// Most distinct participants tracked. Past it the counts stop and
    /// [`JoinsSection::complete`] is false.
    pub max_participants: usize,
    /// Most recent joins listed in [`JoinsSection::recent`].
    pub sample_limit: usize,
}

impl Default for ReportLimits {
    fn default() -> Self {
        Self {
            max_line_bytes: 64 * 1024 * 1024,
            max_participants: 1_000_000,
            sample_limit: 20,
        }
    }
}

/// What a projection was asked for. Every input that varies between
/// calls is here, so the same journal and request give the same report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportRequest {
    /// The world the journal must belong to.
    pub world_id: String,
    /// The report's snapshot time, in unix milliseconds.
    pub generated_at_unix_ms: u64,
    /// The requested window.
    pub window: ReportWindow,
    /// Bounds.
    pub limits: ReportLimits,
}

/// How much of the journal the report rests on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct JournalCoverage {
    /// Version in the journal's header (`None` for an empty journal).
    pub header_version: Option<u32>,
    /// Entries read, header included.
    pub entries: u64,
    /// Whether a compaction checkpoint stands in for earlier events.
    pub checkpointed: bool,
    /// The world this one was forked from, when the fork record survives.
    pub forked_from: Option<String>,
    /// Whether the journal ended in an unterminated line (a write cut
    /// short). The line is not counted and the report is incomplete.
    pub torn_tail: bool,
}

/// One join in the recent sample: a participant id and its state, never
/// a display name or capability material.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct JoinSample {
    /// Server-assigned participant id.
    pub participant_id: u64,
    /// Whether the capability was delegated from another participant's.
    pub delegated: bool,
    /// Whether the capability has since been revoked.
    pub revoked: bool,
}

/// Durable join facts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct JoinsSection {
    /// Distinct participants ever issued a capability.
    pub count: u64,
    /// Of those, issued by the host (no parent capability).
    pub host_issued: u64,
    /// Of those, delegated from another participant's capability.
    pub delegated: u64,
    /// Of those, revoked since.
    pub revoked: u64,
    /// Distinct accounts the capabilities act for; a rejoin under the
    /// same account is one subject and several participants.
    pub distinct_subjects: u64,
    /// Participants the journal had before its fork record, when the
    /// world is a fork and the record survives.
    pub inherited_from_fork: Option<u64>,
    /// Joins inside the requested window.
    pub in_window: Measured<u64>,
    /// Earliest time from which joins are answerable.
    pub available_since: Measured<u64>,
    /// Whether every join in the journal was counted.
    pub complete: bool,
    /// The most recent joins, newest first, at most
    /// [`ReportLimits::sample_limit`].
    pub recent: Vec<JoinSample>,
}

/// Live membership (#11775 supplies it).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MembersSection {
    /// Participants connected at the snapshot.
    pub active_count: Measured<u64>,
    /// What "active" means.
    pub definition: &'static str,
}

/// Program runs (#11775 supplies process-local counters).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProgramRunsSection {
    /// Runs counted.
    pub count: Measured<u64>,
}

/// The owner report for one world.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OwnerReport {
    /// [`OWNER_REPORT_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The world reported on.
    pub world_id: String,
    /// Snapshot time, in unix milliseconds.
    pub generated_at_unix_ms: u64,
    /// The requested window.
    pub window: ReportWindow,
    /// What the journal covered.
    pub journal: JournalCoverage,
    /// Durable join facts.
    pub joins: JoinsSection,
    /// Live membership.
    pub members: MembersSection,
    /// Program runs.
    pub program_runs: ProgramRunsSection,
    /// On-disk storage, as measured by the caller.
    pub storage: Measured<StorageInventory>,
}

/// Why a projection could not produce a report. Each is distinct, so a
/// caller never shows a refusal as an empty report.
#[derive(Debug)]
pub enum OwnerReportError {
    /// The window ends before it starts.
    InvalidWindow,
    /// The journal's first entry is not its header.
    MissingHeader,
    /// The journal belongs to another world.
    WrongWorld {
        /// The world named in the header.
        found: String,
    },
    /// The journal's header version is outside what this binary reads.
    UnsupportedJournalVersion {
        /// The header version.
        version: u32,
    },
    /// A line does not parse as a journal entry, or repeats the header.
    Malformed {
        /// One-based line number.
        line: u64,
    },
    /// An entry needs a newer journal version than its header declares.
    EntryNewerThanHeader {
        /// One-based line number.
        line: u64,
        /// The version the entry needs.
        required: u32,
    },
    /// A line is longer than [`ReportLimits::max_line_bytes`].
    OversizedLine {
        /// One-based line number.
        line: u64,
    },
    /// Reading the journal failed.
    Io(io::Error),
}

impl fmt::Display for OwnerReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidWindow => write!(f, "report window ends before it starts"),
            Self::MissingHeader => write!(f, "world journal does not start with a header"),
            Self::WrongWorld { found } => write!(f, "journal belongs to world {found:?}"),
            Self::UnsupportedJournalVersion { version } => {
                write!(f, "unsupported world journal version {version}")
            }
            Self::Malformed { line } => write!(f, "journal line {line} is not a journal entry"),
            Self::EntryNewerThanHeader { line, required } => write!(
                f,
                "journal line {line} needs journal version {required}, newer than its header"
            ),
            Self::OversizedLine { line } => write!(f, "journal line {line} exceeds the line limit"),
            Self::Io(error) => write!(f, "read journal: {error}"),
        }
    }
}

impl std::error::Error for OwnerReportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

/// Refuse a report JSON whose major schema version this reader does not
/// know. Added fields are fine; a missing or other version is not.
///
/// # Errors
///
/// The version found (or `None` when absent or not a number).
pub fn check_schema_version(report: &serde_json::Value) -> Result<u32, Option<u64>> {
    let found = report
        .get("schema_version")
        .and_then(serde_json::Value::as_u64);
    match found {
        Some(v) if v == u64::from(OWNER_REPORT_SCHEMA_VERSION) => Ok(OWNER_REPORT_SCHEMA_VERSION),
        other => Err(other),
    }
}

#[derive(Clone, Copy, Default)]
struct ParticipantFlags {
    delegated: bool,
    revoked: bool,
}

/// Read one line of at most `max` bytes. `Ok(None)` at end of input;
/// the flag says whether the line ended in a newline.
fn next_line<R: BufRead>(
    journal: &mut R,
    buf: &mut Vec<u8>,
    max: usize,
    line: u64,
) -> Result<Option<bool>, OwnerReportError> {
    buf.clear();
    let cap = u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1);
    let read = journal
        .by_ref()
        .take(cap)
        .read_until(b'\n', buf)
        .map_err(OwnerReportError::Io)?;
    if read == 0 {
        return Ok(None);
    }
    let terminated = buf.last() == Some(&b'\n');
    if buf.len() > max || (!terminated && read as u64 == cap) {
        return Err(OwnerReportError::OversizedLine { line });
    }
    Ok(Some(terminated))
}

/// Project the owner report from one world's journal.
///
/// `storage` is the caller's [`crate::world_storage::inventory`] of the
/// same world, or `None` when it was not measured.
///
/// # Errors
///
/// Any [`OwnerReportError`]: the journal is refused whole rather than
/// reported in part as if complete.
pub fn project<R: BufRead>(
    mut journal: R,
    request: &ReportRequest,
    storage: Option<StorageInventory>,
) -> Result<OwnerReport, OwnerReportError> {
    let window = request.window;
    if window.to_unix_ms < window.from_unix_ms {
        return Err(OwnerReportError::InvalidWindow);
    }
    let limits = request.limits;
    let mut coverage = JournalCoverage {
        header_version: None,
        entries: 0,
        checkpointed: false,
        forked_from: None,
        torn_tail: false,
    };
    let mut participants: HashMap<u64, ParticipantFlags> = HashMap::new();
    let mut subjects: BTreeSet<String> = BTreeSet::new();
    let mut recent: VecDeque<u64> = VecDeque::with_capacity(limits.sample_limit);
    let mut inherited_from_fork = None;
    let mut over_limit = false;
    let mut buf = Vec::new();
    let mut line = 0_u64;

    loop {
        line += 1;
        let Some(terminated) = next_line(&mut journal, &mut buf, limits.max_line_bytes, line)?
        else {
            break;
        };
        if !terminated {
            coverage.torn_tail = true;
            break;
        }
        let text = buf.strip_suffix(b"\n").unwrap_or(&buf);
        let entry: JournalEntry =
            serde_json::from_slice(text).map_err(|_parse_error| OwnerReportError::Malformed { line })?;
        coverage.entries += 1;
        let Some(version) = coverage.header_version else {
            let JournalEntry::Created { world_id, version } = entry else {
                return Err(OwnerReportError::MissingHeader);
            };
            if world_id != request.world_id {
                return Err(OwnerReportError::WrongWorld { found: world_id });
            }
            if !(1..=MAX_JOURNAL_VERSION).contains(&version) {
                return Err(OwnerReportError::UnsupportedJournalVersion { version });
            }
            coverage.header_version = Some(version);
            continue;
        };
        let required = journal_version_required_by(&entry);
        if required > version {
            return Err(OwnerReportError::EntryNewerThanHeader { line, required });
        }
        match entry {
            JournalEntry::Created { .. } => return Err(OwnerReportError::Malformed { line }),
            JournalEntry::Issued {
                participant,
                parent,
                subject,
                ..
            } => {
                if participants.contains_key(&participant.id) {
                    continue;
                }
                if participants.len() >= limits.max_participants {
                    over_limit = true;
                    continue;
                }
                participants.insert(
                    participant.id,
                    ParticipantFlags {
                        delegated: parent.is_some(),
                        revoked: false,
                    },
                );
                if let Some(subject) = subject {
                    subjects.insert(subject);
                }
                if limits.sample_limit > 0 {
                    if recent.len() == limits.sample_limit {
                        recent.pop_front();
                    }
                    recent.push_back(participant.id);
                }
            }
            JournalEntry::Revoked { participant_id, .. } => {
                if let Some(flags) = participants.get_mut(&participant_id) {
                    flags.revoked = true;
                }
            }
            JournalEntry::Forked { from_world_id, .. } => {
                coverage.forked_from = Some(from_world_id);
                inherited_from_fork = Some(participants.len() as u64);
            }
            JournalEntry::Checkpoint { .. } => coverage.checkpointed = true,
            _ => {}
        }
    }

    let count = participants.len() as u64;
    let delegated = participants.values().filter(|f| f.delegated).count() as u64;
    let revoked = participants.values().filter(|f| f.revoked).count() as u64;
    let recent = recent
        .iter()
        .rev()
        .map(|id| {
            let flags = participants.get(id).copied().unwrap_or_default();
            JoinSample {
                participant_id: *id,
                delegated: flags.delegated,
                revoked: flags.revoked,
            }
        })
        .collect();
    let joins = JoinsSection {
        count,
        host_issued: count - delegated,
        delegated,
        revoked,
        distinct_subjects: subjects.len() as u64,
        inherited_from_fork,
        in_window: Measured::Unavailable(UnavailableReason::JoinTimeNotRecorded),
        available_since: Measured::Unavailable(UnavailableReason::JoinTimeNotRecorded),
        complete: !over_limit && !coverage.torn_tail,
        recent,
    };
    Ok(OwnerReport {
        schema_version: OWNER_REPORT_SCHEMA_VERSION,
        world_id: request.world_id.clone(),
        generated_at_unix_ms: request.generated_at_unix_ms,
        window,
        journal: coverage,
        joins,
        members: MembersSection {
            active_count: Measured::Unavailable(UnavailableReason::LiveStateNotProvided),
            definition: "connected at the snapshot",
        },
        program_runs: ProgramRunsSection {
            count: Measured::Unavailable(UnavailableReason::ProgramRunsNotRecorded),
        },
        storage: storage.map_or(
            Measured::Unavailable(UnavailableReason::StorageNotMeasured),
            Measured::Known,
        ),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "world_owner_report_tests.rs"]
mod tests;
