//! The author-scoped local index as a journal projection (#13741,
//! design #6309 §6 step 1).
//!
//! [`build`] reads each world journal the author holds once and returns
//! an [`AuthorIndex`]: one entry per world with the programs its journal
//! installed (module hashes), join counts and recorded timestamps, the
//! journal coverage the entry rests on, and the build time the entry is
//! stale against. The index is a projection: it is rebuilt from the
//! journals on demand, never stored as a second source of truth, and a
//! world whose journal cannot be read degrades to a labelled
//! directory-only entry instead of a guess. It never writes, never opens
//! a network connection and never loads a whole journal: it keeps one
//! line, a per-participant flag set bounded by
//! [`AuthorIndexLimits::max_participants`], and a per-world program map
//! bounded by [`AuthorIndexLimits::max_programs_per_world`].
//!
//! What the journal can and cannot answer, as of journal version
//! [`crate::world::MAX_JOURNAL_VERSION`]:
//!
//! - A program is a distinct module hash the journal installed: a
//!   [`crate::world::JournalEntry::BuilderInstalled`] audit record, a
//!   `ProgramInstalled` committed or system event, or a checkpoint's
//!   active module, which stands in for install events that did not
//!   survive compaction. The journal carries no program name or tag;
//!   those resolve against the program registry in a later slice, not
//!   here.
//! - A join is an `Issued` entry, as in the owner report. The index
//!   keeps the count and the earliest/latest recorded join times; joins
//!   from before times were recorded count as untimed, never as zero.
//! - Staleness is a comparison, not a stored flag: [`AuthorIndex::is_stale`]
//!   reports whether a journal's current modified time is newer than
//!   [`AuthorIndex::built_at_unix_ms`].

use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead};
use std::time::Instant;

use serde::Serialize;

use crate::world::{
    JournalEntry, MAX_JOURNAL_VERSION, WorldEventKind, journal_version_required_by,
};

/// Major version of the index's JSON shape. Consumers accept added
/// optional fields and refuse any other major version.
pub const AUTHOR_INDEX_SCHEMA_VERSION: u32 = 1;

/// Bounds on one index build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthorIndexLimits {
    /// Longest journal line read, newline included. A longer line is
    /// refused rather than buffered.
    pub max_line_bytes: usize,
    /// Most distinct participants tracked per world. Past it the join
    /// counts stop and [`JoinsSummary::complete`] is false.
    pub max_participants: usize,
    /// Most distinct programs tracked per world. Past it later installs
    /// are dropped and the program list is incomplete.
    pub max_programs_per_world: usize,
}

impl Default for AuthorIndexLimits {
    fn default() -> Self {
        Self {
            max_line_bytes: 64 * 1024 * 1024,
            max_participants: 1_000_000,
            max_programs_per_world: 10_000,
        }
    }
}

/// One world the author holds, and how to read its journal. The caller
/// owns enumeration — typically a walk of the local worlds directory —
/// and supplies one source per world.
pub trait WorldJournalSource {
    /// The world this source is for; the journal header must agree.
    fn world_id(&self) -> &str;
    /// Open the world's journal for streaming. `Err` with
    /// [`io::ErrorKind::NotFound`] means the world directory has no
    /// journal; any other error means it could not be read.
    fn open(&self) -> io::Result<Box<dyn BufRead + '_>>;
    /// When the journal file was last modified, in unix milliseconds,
    /// when the caller knows it. The index compares it against its
    /// build time to answer staleness.
    fn journal_modified_unix_ms(&self) -> Option<u64>;
}

/// Why a world has no journal-backed entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectoryOnlyReason {
    /// The world directory has no journal file.
    JournalMissing,
    /// The journal file could not be opened.
    JournalUnreadable,
    /// A journal line does not parse, repeats the header, or needs a
    /// newer version than its header declares.
    JournalMalformed,
    /// The journal's header names another world.
    WrongWorld,
    /// The journal's header version is outside what this binary reads.
    UnsupportedJournalVersion,
}

/// What an index entry rests on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexSource {
    /// The entry was projected from the world's journal.
    Journal,
    /// The world is known only from its directory; the reason says why
    /// its journal could not be projected. Programs and joins are
    /// unknown, not zero.
    DirectoryOnly {
        /// Why the journal could not be projected.
        reason: DirectoryOnlyReason,
    },
}

/// How much of the journal an entry rests on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct JournalSummary {
    /// Version in the journal's header.
    pub header_version: Option<u32>,
    /// Entries read, header included.
    pub entries: u64,
    /// Whether a compaction checkpoint stands in for earlier events.
    pub checkpointed: bool,
    /// Whether the journal ended in an unterminated line (a write cut
    /// short). The line is not counted and the entry is incomplete.
    pub torn_tail: bool,
}

/// One program the world's journal installed, by module hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProgramEntry {
    /// Hash of the installed module.
    pub module_hash: String,
    /// One-based journal line of the first install record for it.
    pub first_seen_entry: u64,
}

/// Durable join facts, summarized.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct JoinsSummary {
    /// Distinct participants ever issued a capability.
    pub count: u64,
    /// Earliest recorded join time, in unix milliseconds.
    pub first_issued_at_unix_ms: Option<u64>,
    /// Latest recorded join time, in unix milliseconds.
    pub last_issued_at_unix_ms: Option<u64>,
    /// Joins whose entry carries no time (issued before times were
    /// recorded).
    pub untimed: u64,
    /// Whether every join in the journal was counted.
    pub complete: bool,
}

/// One world's entry in the index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AuthorIndexWorld {
    /// The world.
    pub world_id: String,
    /// What the entry rests on.
    pub source: IndexSource,
    /// Journal coverage, when the journal was projected.
    pub journal: Option<JournalSummary>,
    /// Programs the journal installed, in first-seen order. `None`
    /// when the world is directory-only: unknown, not empty.
    pub programs: Option<Vec<ProgramEntry>>,
    /// Join facts. `None` when the world is directory-only.
    pub joins: Option<JoinsSummary>,
    /// When the journal file was last modified, when known.
    pub journal_modified_unix_ms: Option<u64>,
}

/// The author-scoped local index: one entry per world the author holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AuthorIndex {
    /// [`AUTHOR_INDEX_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// When the index was built, in unix milliseconds.
    pub built_at_unix_ms: u64,
    /// How long the build took, in milliseconds.
    pub build_duration_ms: u64,
    /// One entry per world, in the order the sources were supplied.
    pub worlds: Vec<AuthorIndexWorld>,
    /// Sources skipped because an earlier source named the same world.
    pub duplicates_skipped: u64,
}

impl AuthorIndex {
    /// Whether the world's journal has changed since this index was
    /// built: `Some(true)` when `journal_modified_unix_ms` is newer
    /// than the build time, `Some(false)` when it is not, `None` when
    /// the index holds no entry for the world.
    #[must_use]
    pub fn is_stale(&self, world_id: &str, journal_modified_unix_ms: u64) -> Option<bool> {
        self.worlds
            .iter()
            .find(|world| world.world_id == world_id)
            .map(|_| journal_modified_unix_ms > self.built_at_unix_ms)
    }
}

/// Refuse an index JSON whose major schema version this reader does not
/// know. Added fields are fine; a missing or other version is not.
///
/// # Errors
///
/// The version found (or `None` when absent or not a number).
pub fn check_schema_version(index: &serde_json::Value) -> Result<u32, Option<u64>> {
    let found = index
        .get("schema_version")
        .and_then(serde_json::Value::as_u64);
    match found {
        Some(v) if v == u64::from(AUTHOR_INDEX_SCHEMA_VERSION) => Ok(AUTHOR_INDEX_SCHEMA_VERSION),
        other => Err(other),
    }
}

/// Why one world's journal could not be projected.
#[derive(Debug)]
enum WorldFailure {
    /// A journal line does not parse, repeats the header, or needs a
    /// newer version than its header declares.
    Malformed {
        /// One-based line number.
        line: u64,
    },
    WrongWorld,
    UnsupportedVersion,
}

/// Read one line of at most `max` bytes. `Ok(None)` at end of input;
/// the flag says whether the line ended in a newline. Mirrors the
/// owner report's bounded read.
fn next_line<R: BufRead>(
    journal: &mut R,
    buf: &mut Vec<u8>,
    max: usize,
    line: u64,
) -> Result<Option<bool>, WorldFailure> {
    buf.clear();
    let cap = u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1);
    let read = <&mut R as io::Read>::take(journal, cap)
        .read_until(b'\n', buf)
        .map_err(|_| WorldFailure::Malformed { line })?;
    if read == 0 {
        return Ok(None);
    }
    let terminated = buf.last() == Some(&b'\n');
    if buf.len() > max || (!terminated && read as u64 == cap) {
        return Err(WorldFailure::Malformed { line });
    }
    Ok(Some(terminated))
}

/// Project one world's entry from its journal.
///
/// # Errors
///
/// Any [`WorldFailure`]: the caller degrades the world to
/// directory-only rather than reporting a partial entry as complete.
fn project_world<R: BufRead>(
    world_id: &str,
    mut journal: R,
    modified_unix_ms: Option<u64>,
    limits: AuthorIndexLimits,
) -> Result<AuthorIndexWorld, WorldFailure> {
    let mut summary = JournalSummary {
        header_version: None,
        entries: 0,
        checkpointed: false,
        torn_tail: false,
    };
    let mut programs: Vec<ProgramEntry> = Vec::new();
    let mut program_lines: HashMap<String, u64> = HashMap::new();
    let mut participants: HashSet<u64> = HashSet::new();
    let mut joins_complete = true;
    let mut first_issued_at: Option<u64> = None;
    let mut last_issued_at: Option<u64> = None;
    let mut untimed = 0_u64;
    let mut buf = Vec::new();
    let mut line = 0_u64;

    // Record a program install, keeping first-seen order.
    let record_program = |programs: &mut Vec<ProgramEntry>,
                          program_lines: &mut HashMap<String, u64>,
                          module_hash: &str,
                          line: u64| {
        if module_hash.is_empty() || program_lines.contains_key(module_hash) {
            return;
        }
        if programs.len() >= limits.max_programs_per_world {
            return;
        }
        program_lines.insert(module_hash.to_owned(), line);
        programs.push(ProgramEntry {
            module_hash: module_hash.to_owned(),
            first_seen_entry: line,
        });
    };

    loop {
        line += 1;
        let Some(terminated) = next_line(&mut journal, &mut buf, limits.max_line_bytes, line)?
        else {
            break;
        };
        if !terminated {
            summary.torn_tail = true;
            break;
        }
        let text = buf.strip_suffix(b"\n").unwrap_or(&buf);
        if text.is_empty() {
            // A blank line is skipped, as `FileJournal::open` skips it.
            continue;
        }
        let entry: JournalEntry =
            serde_json::from_slice(text).map_err(|_| WorldFailure::Malformed { line })?;
        summary.entries += 1;
        let Some(version) = summary.header_version else {
            let JournalEntry::Created {
                world_id: found,
                version,
            } = entry
            else {
                return Err(WorldFailure::Malformed { line });
            };
            if found != world_id {
                return Err(WorldFailure::WrongWorld);
            }
            if !(1..=MAX_JOURNAL_VERSION).contains(&version) {
                return Err(WorldFailure::UnsupportedVersion);
            }
            summary.header_version = Some(version);
            continue;
        };
        let required = journal_version_required_by(&entry);
        if required > version {
            return Err(WorldFailure::Malformed { line });
        }
        match entry {
            JournalEntry::Created { .. } => return Err(WorldFailure::Malformed { line }),
            JournalEntry::Issued {
                participant,
                issued_at,
                ..
            } => {
                if participants.contains(&participant.id) {
                    continue;
                }
                if participants.len() >= limits.max_participants {
                    joins_complete = false;
                    continue;
                }
                participants.insert(participant.id);
                match issued_at {
                    Some(at) => {
                        first_issued_at = Some(first_issued_at.map_or(at, |first| first.min(at)));
                        last_issued_at = Some(last_issued_at.map_or(at, |last| last.max(at)));
                    }
                    None => untimed += 1,
                }
            }
            JournalEntry::BuilderInstalled { module_hash, .. } => {
                record_program(&mut programs, &mut program_lines, &module_hash, line);
            }
            JournalEntry::Committed { event } | JournalEntry::System { event } => {
                if let WorldEventKind::ProgramInstalled { module_hash, .. } = &event.kind {
                    record_program(&mut programs, &mut program_lines, module_hash, line);
                }
            }
            JournalEntry::Checkpoint { module_hash, .. } => {
                summary.checkpointed = true;
                record_program(&mut programs, &mut program_lines, &module_hash, line);
            }
            _ => {}
        }
    }

    if summary.header_version.is_none() {
        return Err(WorldFailure::Malformed { line });
    }
    Ok(AuthorIndexWorld {
        world_id: world_id.to_owned(),
        source: IndexSource::Journal,
        journal: Some(summary),
        programs: Some(programs),
        joins: Some(JoinsSummary {
            count: participants.len() as u64,
            first_issued_at_unix_ms: first_issued_at,
            last_issued_at_unix_ms: last_issued_at,
            untimed,
            complete: joins_complete && !summary.torn_tail,
        }),
        journal_modified_unix_ms: modified_unix_ms,
    })
}

fn directory_only(
    world_id: &str,
    reason: DirectoryOnlyReason,
    modified_unix_ms: Option<u64>,
) -> AuthorIndexWorld {
    AuthorIndexWorld {
        world_id: world_id.to_owned(),
        source: IndexSource::DirectoryOnly { reason },
        journal: None,
        programs: None,
        joins: None,
        journal_modified_unix_ms: modified_unix_ms,
    }
}

/// Build the author-scoped index from the worlds the author holds.
///
/// A world whose journal cannot be opened or projected degrades to a
/// directory-only entry with the reason labelled; the other worlds
/// still index. The build never fails.
#[must_use]
pub fn build<S: WorldJournalSource>(
    sources: &[S],
    built_at_unix_ms: u64,
    limits: AuthorIndexLimits,
) -> AuthorIndex {
    let started = Instant::now();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut worlds: Vec<AuthorIndexWorld> = Vec::new();
    let mut duplicates_skipped = 0_u64;

    for source in sources {
        let world_id = source.world_id();
        if !seen.insert(world_id) {
            duplicates_skipped += 1;
            continue;
        }
        let modified = source.journal_modified_unix_ms();
        let world = match source.open() {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                directory_only(world_id, DirectoryOnlyReason::JournalMissing, modified)
            }
            Err(_) => {
                directory_only(world_id, DirectoryOnlyReason::JournalUnreadable, modified)
            }
            Ok(journal) => match project_world(world_id, journal, modified, limits) {
                Ok(world) => world,
                Err(WorldFailure::Malformed { .. }) => {
                    directory_only(world_id, DirectoryOnlyReason::JournalMalformed, modified)
                }
                Err(WorldFailure::WrongWorld) => {
                    directory_only(world_id, DirectoryOnlyReason::WrongWorld, modified)
                }
                Err(WorldFailure::UnsupportedVersion) => directory_only(
                    world_id,
                    DirectoryOnlyReason::UnsupportedJournalVersion,
                    modified,
                ),
            },
        };
        worlds.push(world);
    }

    AuthorIndex {
        schema_version: AUTHOR_INDEX_SCHEMA_VERSION,
        built_at_unix_ms,
        build_duration_ms: started.elapsed().as_millis() as u64,
        worlds,
        duplicates_skipped,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "world_author_index_tests.rs"]
mod tests;
