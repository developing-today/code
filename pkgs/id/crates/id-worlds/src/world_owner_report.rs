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
//! - Since journal version 12 ([`crate::world::JOURNAL_VERSION_JOIN_TIMES`],
//!   #11960) an `Issued` entry carries `issued_at` and a `Revoked` entry
//!   carries `revoked_at`. Joins and revokes from before that carry no
//!   time and are reported as such, never as zero. `available_since` is
//!   the earliest time from which every later join carries one; a window
//!   that starts before it is counted but marked incomplete. A journal
//!   with no timed join at all reports join times as
//!   [`UnavailableReason::JoinTimeNotRecorded`].
//! - Live membership and program activity come only from a running
//!   world: [`OwnerReport::with_live`] fills them from a [`LiveSnapshot`]
//!   (#11775), and without one they are unavailable. Program activity is
//!   a process-local guest call count for one program instance, never
//!   durable history: the journal records no runs.
//!
//! `null` plus a reason means unavailable; `0` means measured zero.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fmt;
use std::io::{self, BufRead, Read};

use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};

#[cfg(feature = "sandbox")]
use crate::sandbox::CallMetrics;
use crate::world::{
    JournalEntry, MAX_JOURNAL_VERSION, MetricsSnapshot, journal_version_required_by,
};
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
    /// Capabilities were revoked, and the journal records no time for any
    /// of those revokes.
    RevokeTimeNotRecorded,
    /// No one has joined, so there is no join time to start from.
    NoJoins,
    /// No live participant snapshot was supplied (the world is not
    /// running, or the caller did not ask it).
    LiveStateNotProvided,
    /// No program call counter was supplied; the journal records no runs.
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
            Self::RevokeTimeNotRecorded => "the journal does not record when anyone was revoked",
            Self::NoJoins => "no one has joined yet",
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
    /// When the capability was issued (Unix ms); `null` when the journal
    /// entry predates recorded times.
    pub issued_at_unix_ms: Option<u64>,
    /// When it was revoked (Unix ms); `null` while active or when no time
    /// was recorded.
    pub revoked_at_unix_ms: Option<u64>,
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
    /// Whether `in_window` covers the whole window: false when the window
    /// starts before `available_since`, where untimed joins may belong.
    pub in_window_complete: bool,
    /// Earliest time from which joins are answerable: every join after it
    /// in the journal carries a time.
    pub available_since: Measured<u64>,
    /// Joins whose entry carries no time (issued before times were
    /// recorded).
    pub untimed: u64,
    /// Revokes inside the requested window, counting those with a time.
    pub revoked_in_window: Measured<u64>,
    /// Revoked participants whose revoke carries no time.
    pub revoked_untimed: u64,
    /// Whether every join in the journal was counted.
    pub complete: bool,
    /// The most recent joins, newest first, at most
    /// [`ReportLimits::sample_limit`].
    pub recent: Vec<JoinSample>,
}

/// Live membership, from a [`LiveSnapshot`] when one is supplied.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MembersSection {
    /// Distinct participants connected at the snapshot.
    pub active_count: Measured<u64>,
    /// Live sessions at the snapshot; one participant may hold several.
    pub sessions: Measured<u64>,
    /// What "active" means.
    pub definition: &'static str,
    /// When the live snapshot was taken, when there is one.
    pub as_of_unix_ms: Option<u64>,
}

/// Program activity, only ever from a process-local counter.
///
/// The journal records no program runs, so the count covers one program
/// instance since it started in this process, resets on restart or
/// reinstall, and is never complete.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProgramRunsSection {
    /// Events the program processed (guest `update` calls) since
    /// [`Self::since_unix_ms`].
    pub count: Measured<u64>,
    /// Every guest call kind with its count, when supplied.
    pub by_call: Option<BTreeMap<&'static str, u64>>,
    /// What the counters cover.
    pub scope: ProgramRunScope,
    /// When the counted program instance started, when known.
    pub since_unix_ms: Option<u64>,
    /// Always false: counters are process-local and not durable history.
    pub complete: bool,
}

/// What a program-run count covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramRunScope {
    /// No counter was supplied.
    None,
    /// One program instance, in this process, since it started.
    ProgramInstanceInProcess,
}

/// Guest call counts from one running program instance (#11775): the
/// in-memory call metrics a sandboxed world instance keeps, never
/// journaled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramCallCounts {
    /// Calls per guest call kind.
    pub by_call: BTreeMap<&'static str, u64>,
    /// When the instance started, when known.
    pub since_unix_ms: Option<u64>,
}

#[cfg(feature = "sandbox")]
impl ProgramCallCounts {
    /// The counts from a program instance's call metrics.
    #[must_use]
    pub fn from_call_metrics(metrics: &CallMetrics, since_unix_ms: Option<u64>) -> Self {
        Self {
            by_call: metrics
                .iter()
                .map(|(call, stats)| (call.name(), stats.calls))
                .collect(),
            since_unix_ms,
        }
    }
}

/// What a caller holding the running world saw at one moment (#11775).
/// Nothing in it is durable; a report built without one says so.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveSnapshot {
    /// When it was taken, in unix milliseconds.
    pub as_of_unix_ms: u64,
    /// Distinct participants connected.
    pub connected_participants: u64,
    /// Live sessions.
    pub sessions: u64,
    /// Program call counts, when the caller can reach the instance.
    pub program_calls: Option<ProgramCallCounts>,
}

impl LiveSnapshot {
    /// The live counts in a running world's metrics snapshot (what
    /// `WorldHandle::metrics` returns). The actor exposes no program
    /// call counts, so `program_calls` is `None`.
    #[must_use]
    pub const fn from_metrics(metrics: &MetricsSnapshot, as_of_unix_ms: u64) -> Self {
        Self {
            as_of_unix_ms,
            connected_participants: metrics.participants,
            sessions: metrics.sessions,
            program_calls: None,
        }
    }
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
    issued_at: Option<u64>,
    revoked_at: Option<u64>,
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
    // Earliest join time after the last join without one (#11960).
    let mut timed_since: Option<u64> = None;
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
        if text.is_empty() {
            // A blank line is skipped, as `FileJournal::open` skips it.
            continue;
        }
        let entry: JournalEntry = serde_json::from_slice(text)
            .map_err(|_parse_error| OwnerReportError::Malformed { line })?;
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
                issued_at,
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
                        issued_at,
                        revoked_at: None,
                    },
                );
                timed_since = issued_at.map(|at| timed_since.map_or(at, |since| since.min(at)));
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
            JournalEntry::Revoked {
                participant_id,
                revoked_at,
                ..
            } => {
                if let Some(flags) = participants.get_mut(&participant_id) {
                    flags.revoked = true;
                    flags.revoked_at = revoked_at;
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
                issued_at_unix_ms: flags.issued_at,
                revoked_at_unix_ms: flags.revoked_at,
            }
        })
        .collect();
    let in_range = |at: Option<u64>| {
        at.is_some_and(|at| (window.from_unix_ms..window.to_unix_ms).contains(&at))
    };
    let untimed = participants
        .values()
        .filter(|f| f.issued_at.is_none())
        .count() as u64;
    let (in_window, in_window_complete, available_since) = match timed_since {
        _ if count == 0 => (
            Measured::Known(0),
            true,
            Measured::Unavailable(UnavailableReason::NoJoins),
        ),
        Some(since) => (
            Measured::Known(
                participants
                    .values()
                    .filter(|f| in_range(f.issued_at))
                    .count() as u64,
            ),
            window.from_unix_ms >= since,
            Measured::Known(since),
        ),
        None => (
            Measured::Unavailable(UnavailableReason::JoinTimeNotRecorded),
            false,
            Measured::Unavailable(UnavailableReason::JoinTimeNotRecorded),
        ),
    };
    let revoked_timed = participants
        .values()
        .filter(|f| f.revoked && f.revoked_at.is_some())
        .count() as u64;
    let revoked_in_window = if revoked > 0 && revoked_timed == 0 {
        Measured::Unavailable(UnavailableReason::RevokeTimeNotRecorded)
    } else {
        Measured::Known(
            participants
                .values()
                .filter(|f| f.revoked && in_range(f.revoked_at))
                .count() as u64,
        )
    };
    let joins = JoinsSection {
        count,
        host_issued: count - delegated,
        delegated,
        revoked,
        distinct_subjects: subjects.len() as u64,
        inherited_from_fork,
        in_window,
        in_window_complete,
        available_since,
        untimed,
        revoked_in_window,
        revoked_untimed: revoked - revoked_timed,
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
        members: members_section(None),
        program_runs: program_runs_section(None),
        storage: storage.map_or(
            Measured::Unavailable(UnavailableReason::StorageNotMeasured),
            Measured::Known,
        ),
    })
}

const ACTIVE_DEFINITION: &str = "connected at the snapshot";

const fn members_section(live: Option<&LiveSnapshot>) -> MembersSection {
    match live {
        Some(live) => MembersSection {
            active_count: Measured::Known(live.connected_participants),
            sessions: Measured::Known(live.sessions),
            definition: ACTIVE_DEFINITION,
            as_of_unix_ms: Some(live.as_of_unix_ms),
        },
        None => MembersSection {
            active_count: Measured::Unavailable(UnavailableReason::LiveStateNotProvided),
            sessions: Measured::Unavailable(UnavailableReason::LiveStateNotProvided),
            definition: ACTIVE_DEFINITION,
            as_of_unix_ms: None,
        },
    }
}

fn program_runs_section(calls: Option<&ProgramCallCounts>) -> ProgramRunsSection {
    match calls {
        Some(calls) => ProgramRunsSection {
            count: Measured::Known(calls.by_call.get("update").copied().unwrap_or(0)),
            by_call: Some(calls.by_call.clone()),
            scope: ProgramRunScope::ProgramInstanceInProcess,
            since_unix_ms: calls.since_unix_ms,
            complete: false,
        },
        None => ProgramRunsSection {
            count: Measured::Unavailable(UnavailableReason::ProgramRunsNotRecorded),
            by_call: None,
            scope: ProgramRunScope::None,
            since_unix_ms: None,
            complete: false,
        },
    }
}

impl OwnerReport {
    /// This report with the live sections taken from `live`, or marked
    /// unavailable when there is none. The durable sections are
    /// unchanged, so two hosts with the same journal still agree on them.
    #[must_use]
    pub fn with_live(mut self, live: Option<&LiveSnapshot>) -> Self {
        self.members = members_section(live);
        self.program_runs = program_runs_section(live.and_then(|live| live.program_calls.as_ref()));
        self
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "world_owner_report_tests.rs"]
mod tests;
