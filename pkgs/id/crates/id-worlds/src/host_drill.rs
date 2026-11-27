//! One-shot hosted-world restore drills (#13744, packet Q2 slice 1 of the
//! hosted-worlds v2 outline on #6284).
//!
//! A drill proves a snapshot is a backup rather than a hope: export the
//! named world's latest signed snapshot ([`crate::world_export`]), restore
//! it into a fresh scratch world that is never adopted into the host file,
//! replay the scratch to the same journal head, and compare. The oracle is
//! the replay-equality check P6 built for hosted/local parity (#10698):
//! the restored journal must be byte-identical and the replay reports must
//! agree on sequence, entry count, replayed inputs, module and program
//! state.
//!
//! The harness lives here, over a data root with no host running, so a
//! drill never needs the serving host and never mutates the source world.
//! `id host drill` is the operator surface. Every attempt appends one
//! typed [`DrillRecord`] to [`DRILL_RECORD_FILE_NAME`] beside the data
//! root, readable locally first; scheduling cadence and the
//! missing-success alarm stay follow-ups on #13744, as does cross-host
//! adoption (a drill restores into scratch outside every data root, so the
//! import collision question does not arise here).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};

use crate::host::{HostFile, open_in_root};
use crate::world::WorldLimits;
use crate::world_export::{export_world_to_file, import_world_from_file};
use crate::world_limits::RuntimeLimits;
use crate::world_session::WorldService;
use crate::world_storage::JOURNAL_FILE;
use crate::world_store::{OpenReport, open_world};

/// File at the data-root top level holding one [`DrillRecord`] per drill
/// attempt as JSON, one object per line, oldest first. Append-only: a drill
/// never rewrites history, and a missing recent success is the alarm's
/// input (#13744).
pub const DRILL_RECORD_FILE_NAME: &str = ".drills.jsonl";

/// The typed result of one drill attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DrillOutcome {
    /// The scratch restore replayed to the same head as the source.
    Pass,
    /// Any stage failed, or the scratch diverged from the source.
    Fail,
}

/// The replay state a drill compares: the [`crate::world_store::OpenReport`]
/// fields that must agree for a restore to count as the same world.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayFingerprint {
    /// Last committed sequence after the replay.
    pub sequence: u64,
    /// Journal entries replayed, the `Created` header included.
    pub journal_entries: u64,
    /// Inputs replayed into the restored program.
    pub replayed_inputs: u64,
    /// Active module, if any.
    pub module_hash: Option<String>,
    /// Why the program could not be restored, if it could not.
    pub program_error: Option<String>,
}

impl ReplayFingerprint {
    /// The fingerprint of a completed replay.
    #[must_use]
    pub fn of(report: &OpenReport) -> Self {
        Self {
            sequence: report.sequence,
            journal_entries: u64::try_from(report.journal_entries).unwrap_or(u64::MAX),
            replayed_inputs: u64::try_from(report.replayed_inputs).unwrap_or(u64::MAX),
            module_hash: report.module_hash.clone(),
            program_error: report.program_error.clone(),
        }
    }

    /// The field names on which `self` and `other` disagree, sorted.
    #[must_use]
    pub fn diff(&self, other: &Self) -> Vec<String> {
        let mut out = Vec::new();
        if self.sequence != other.sequence {
            out.push("sequence".to_owned());
        }
        if self.journal_entries != other.journal_entries {
            out.push("journal_entries".to_owned());
        }
        if self.replayed_inputs != other.replayed_inputs {
            out.push("replayed_inputs".to_owned());
        }
        if self.module_hash != other.module_hash {
            out.push("module_hash".to_owned());
        }
        if self.program_error != other.program_error {
            out.push("program_error".to_owned());
        }
        out.sort();
        out
    }
}

/// One drill attempt, as appended to [`DRILL_RECORD_FILE_NAME`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrillRecord {
    /// Unix seconds (UTC) when the drill finished.
    pub at_secs: u64,
    /// The drilled world.
    pub world: String,
    /// Whether the scratch restore matched the source.
    pub outcome: DrillOutcome,
    /// The drill stage that decided the outcome: `compare` for a
    /// completed round trip, otherwise the stage that failed
    /// (`open-source`, `export`, `restore` or `reopen`).
    pub stage: String,
    /// The source world's replay state, when it replayed.
    pub source: Option<ReplayFingerprint>,
    /// The scratch world's replay state, when it replayed.
    pub restored: Option<ReplayFingerprint>,
    /// Whether the scratch journal bytes equal the source journal bytes.
    pub journal_identical: bool,
    /// Fingerprint fields that disagreed, sorted; empty on a pass.
    pub mismatches: Vec<String>,
    /// Journal entries in the signed export, when one was taken.
    pub export_entries: Option<u64>,
    /// Why the drill failed, when it did.
    pub error: Option<String>,
}

/// A completed drill: its record and where it was appended.
#[derive(Clone, Debug)]
pub struct DrillReport {
    /// The appended record.
    pub record: DrillRecord,
    /// The record file the record was appended to.
    pub record_path: PathBuf,
}

impl DrillReport {
    /// Whether the scratch restore matched the source.
    #[must_use]
    pub fn pass(&self) -> bool {
        self.record.outcome == DrillOutcome::Pass
    }
}

/// Unix seconds (UTC) now.
#[must_use]
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The record file for the data root `root`.
#[must_use]
pub fn drill_record_path(root: &Path) -> PathBuf {
    root.join(DRILL_RECORD_FILE_NAME)
}

/// Every drill record for the data root `root`, oldest first. A missing
/// record file is no records, not an error, so the alarm check
/// (`read_drill_records` empty) never fails on a fresh root.
///
/// # Errors
///
/// Fails when the record file exists but cannot be read or parsed.
pub fn read_drill_records(root: &Path) -> Result<Vec<DrillRecord>> {
    let path = drill_record_path(root);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("reading {}", path.display()));
        }
    };
    let text =
        String::from_utf8(bytes).with_context(|| format!("{} is not UTF-8", path.display()))?;
    let mut records = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let record: DrillRecord = serde_json::from_str(line)
            .with_context(|| format!("parsing {} line {}", path.display(), index + 1))?;
        records.push(record);
    }
    Ok(records)
}

/// Append `record` to the data root's drill record file.
///
/// # Errors
///
/// Fails on any filesystem error; the record is lost then.
fn append_drill_record(root: &Path, record: &DrillRecord) -> Result<PathBuf> {
    let path = drill_record_path(root);
    let mut line = serde_json::to_string(record).context("serializing drill record")?;
    line.push('\n');
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    use std::io::Write as _;
    (&file)
        .write_all(line.as_bytes())
        .with_context(|| format!("appending to {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("fsync {}", path.display()))?;
    Ok(path)
}

/// Run a one-shot restore drill for `world` in the data root `root`:
/// replay the source, export it signed by `signing_key`, restore the
/// export into a fresh scratch world outside every data root, replay the
/// scratch, and compare. The source world is never mutated and the scratch
/// is never adopted: only [`DRILL_RECORD_FILE_NAME`] changes.
///
/// A completed round trip returns its [`DrillReport`] with a typed
/// outcome; the scratch diverging from the source is
/// [`DrillOutcome::Fail`], not an error. A stage that cannot complete is
/// recorded as a failure and returned as an error.
///
/// # Errors
///
/// Fails for a world the host file does not record, and for any export,
/// restore or reopen failure (each recorded as a failed attempt first).
pub async fn run_restore_drill(
    root: &Path,
    world: &str,
    signing_key: &SigningKey,
    at_secs: u64,
) -> Result<DrillReport> {
    let file = HostFile::read_or_rebuild(root)
        .with_context(|| format!("reading data root {}", root.display()))?;
    if !file.worlds.contains_key(world) {
        bail!("no world {world:?} in data root {}", root.display());
    }
    match round_trip(root, world, signing_key).await {
        Ok(trip) => {
            let outcome = if trip.journal_identical && trip.mismatches.is_empty() {
                DrillOutcome::Pass
            } else {
                DrillOutcome::Fail
            };
            let record = DrillRecord {
                at_secs,
                world: world.to_owned(),
                outcome,
                stage: "compare".to_owned(),
                source: Some(trip.source),
                restored: Some(trip.restored),
                journal_identical: trip.journal_identical,
                mismatches: trip.mismatches,
                export_entries: Some(trip.export_entries),
                error: None,
            };
            let record_path = append_drill_record(root, &record)?;
            Ok(DrillReport {
                record,
                record_path,
            })
        }
        Err(stage) => {
            let record = DrillRecord {
                at_secs,
                world: world.to_owned(),
                outcome: DrillOutcome::Fail,
                stage: stage.stage.to_owned(),
                source: None,
                restored: None,
                journal_identical: false,
                mismatches: Vec::new(),
                export_entries: None,
                error: Some(format!("{:#}", stage.error)),
            };
            append_drill_record(root, &record)?;
            Err(stage.error).with_context(|| {
                format!(
                    "restore drill for world {world:?} failed at stage {}",
                    stage.stage
                )
            })
        }
    }
}

/// A completed export→restore→reopen round trip, before the verdict.
struct RoundTrip {
    source: ReplayFingerprint,
    restored: ReplayFingerprint,
    journal_identical: bool,
    mismatches: Vec<String>,
    export_entries: u64,
}

/// A drill stage that could not complete.
#[derive(Debug)]
struct StageError {
    stage: &'static str,
    error: anyhow::Error,
}

/// Tag an error with the drill stage that raised it.
fn stage_error<E>(stage: &'static str, error: E) -> StageError
where
    anyhow::Error: From<E>,
{
    StageError {
        stage,
        error: anyhow::Error::from(error),
    }
}

/// Replay the source, export it, restore the export into a fresh scratch
/// directory outside every data root, replay the scratch, and compare.
/// The source directory is only read; the scratch is dropped with the
/// temporary directory it lives in and is never adopted.
async fn round_trip(
    root: &Path,
    world: &str,
    signing_key: &SigningKey,
) -> Result<RoundTrip, StageError> {
    let scratch = tempfile::TempDir::new().map_err(|error| stage_error("scratch", error))?;
    let export_path = scratch.path().join("drill.idexport");
    let scratch_dir = scratch.path().join("scratch");

    let (service, source_report) = open_in_root(root, world)
        .await
        .map_err(|error| stage_error("open-source", error))?;
    let source = ReplayFingerprint::of(&source_report);
    export_world_to_file(&service, signing_key, &export_path)
        .await
        .map_err(|error| stage_error("export", error))?;
    service
        .world()
        .shutdown()
        .await
        .map_err(|error| stage_error("export", error))?;

    let pinned = *signing_key.verifying_key().as_bytes();
    let manifest = import_world_from_file(&export_path, &scratch_dir, pinned)
        .await
        .map_err(|error| stage_error("restore", error))?;
    if manifest.world != world {
        return Err(stage_error(
            "restore",
            anyhow::anyhow!(
                "the export restores world {:?}, not the drilled world {:?}",
                manifest.world,
                world
            ),
        ));
    }
    let (restored_service, restored_report) = open_world(
        &scratch_dir,
        &manifest.world,
        WorldLimits::default(),
        RuntimeLimits::default(),
        |handle| WorldService::new(handle, None),
    )
    .await
    .map_err(|error| stage_error("reopen", error))?;
    restored_service
        .world()
        .shutdown()
        .await
        .map_err(|error| stage_error("reopen", error))?;
    let restored = ReplayFingerprint::of(&restored_report);

    let source_journal = std::fs::read(root.join(world).join(JOURNAL_FILE))
        .map_err(|error| stage_error("reopen", error))?;
    let restored_journal = std::fs::read(scratch_dir.join(JOURNAL_FILE))
        .map_err(|error| stage_error("reopen", error))?;
    let journal_identical = source_journal == restored_journal;
    let mismatches = source.diff(&restored);
    Ok(RoundTrip {
        source,
        restored,
        journal_identical,
        mismatches,
        export_entries: manifest.entries,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::world_store::OpenReport;

    #[test]
    fn replay_fingerprints_name_every_divergence() {
        let base = OpenReport {
            sequence: 9,
            journal_entries: 4,
            module_hash: Some("ab".to_owned()),
            replayed_inputs: 2,
            program_error: None,
        };
        let same = ReplayFingerprint::of(&base);
        assert!(same.diff(&ReplayFingerprint::of(&base)).is_empty());

        let other = OpenReport {
            sequence: 10,
            journal_entries: 5,
            module_hash: None,
            replayed_inputs: 3,
            program_error: Some("no module".to_owned()),
        };
        assert_eq!(
            ReplayFingerprint::of(&base).diff(&ReplayFingerprint::of(&other)),
            vec![
                "journal_entries".to_owned(),
                "module_hash".to_owned(),
                "program_error".to_owned(),
                "replayed_inputs".to_owned(),
                "sequence".to_owned(),
            ]
        );
    }

    #[test]
    fn drill_records_append_and_read_back_oldest_first() {
        let root = tempfile::TempDir::new().unwrap();
        assert!(read_drill_records(root.path()).unwrap().is_empty());

        for (at_secs, outcome) in [(1, DrillOutcome::Pass), (2, DrillOutcome::Fail)] {
            let record = DrillRecord {
                at_secs,
                world: "w".to_owned(),
                outcome,
                stage: "compare".to_owned(),
                source: None,
                restored: None,
                journal_identical: outcome == DrillOutcome::Pass,
                mismatches: Vec::new(),
                export_entries: Some(3),
                error: None,
            };
            append_drill_record(root.path(), &record).unwrap();
        }
        let records = read_drill_records(root.path()).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].at_secs, 1);
        assert_eq!(records[0].outcome, DrillOutcome::Pass);
        assert_eq!(records[1].outcome, DrillOutcome::Fail);
    }
}
