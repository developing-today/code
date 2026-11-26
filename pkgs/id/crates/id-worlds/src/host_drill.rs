//! One-shot restore drills for a hosted world (#13744, slice 1).
//!
//! A drill proves that a world's signed export restores. It copies the
//! world's directory out of the data root into scratch space, replays
//! the copy, exports it signed with the host key, restores the export
//! into a fresh scratch world and replays that. The source world in the
//! data root is only read, never opened, so a drill is safe to run while
//! a host serves the world.
//!
//! The oracle is the one the host export tests assert: the restored
//! journal is byte-identical to the exported one, and both replays land
//! on the same head (sequence, journal entries, active module, replayed
//! inputs, program state). Every attempt, passing or not, is appended to
//! [`DRILL_RECORD_FILE`] in the data root.
//!
//! Scheduling a cadence, alarming on a missing success and cross-host
//! import policy are later work on #13744.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};

use crate::host::{open_in_root, validate_name};
use crate::world_export::{export_world_to_file, import_world_from_file};
use crate::world_session::encode_hex;
use crate::world_storage::JOURNAL_FILE;
use crate::world_store::OpenReport;

/// The append-only drill record inside a data root: one JSON line per
/// attempt.
pub const DRILL_RECORD_FILE: &str = "restore-drills.jsonl";

/// Whether a drill's restore matched its source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DrillOutcome {
    /// The restore replayed to the same head, byte for byte.
    Pass,
    /// A step failed or the restore differs; see [`DrillRecord::reason`].
    Fail,
}

/// Where one replay landed: the parts of [`OpenReport`] the oracle
/// compares.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrillHead {
    /// Last committed sequence.
    pub sequence: u64,
    /// Participants known to the journal.
    pub journal_entries: usize,
    /// Active module, if any.
    pub module_hash: Option<String>,
    /// Inputs replayed into the restored program.
    pub replayed_inputs: usize,
    /// Why the program could not be restored, if it could not.
    pub program_error: Option<String>,
}

impl From<&OpenReport> for DrillHead {
    fn from(report: &OpenReport) -> Self {
        Self {
            sequence: report.sequence,
            journal_entries: report.journal_entries,
            module_hash: report.module_hash.clone(),
            replayed_inputs: report.replayed_inputs,
            program_error: report.program_error.clone(),
        }
    }
}

/// One drill attempt, as [`DRILL_RECORD_FILE`] keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrillRecord {
    /// Unix seconds when the drill started.
    pub at: u64,
    /// The world drilled.
    pub world: String,
    /// The outcome.
    pub outcome: DrillOutcome,
    /// Why the drill failed; `None` on a pass.
    pub reason: Option<String>,
    /// Lowercase hex of the key that signed the export.
    pub signed_by: String,
    /// Journal lines in the export, the `Created` header included.
    pub export_entries: Option<u64>,
    /// Where the source replay landed.
    pub source_head: Option<DrillHead>,
    /// Where the restored replay landed.
    pub restored_head: Option<DrillHead>,
}

/// What the steps of a drill produced before the oracle compares them.
#[derive(Default)]
struct Seen {
    export_entries: Option<u64>,
    source_head: Option<DrillHead>,
    source_journal: Option<Vec<u8>>,
    restored_head: Option<DrillHead>,
    restored_journal: Option<Vec<u8>>,
}

/// The oracle: the restored world must hold the exported journal byte
/// for byte and replay to the same head.
///
/// # Errors
///
/// Names the first difference.
fn replay_equality(
    source_journal: &[u8],
    source: &DrillHead,
    restored_journal: &[u8],
    restored: &DrillHead,
) -> Result<()> {
    if source_journal != restored_journal {
        bail!(
            "replay equality: the restored journal differs from the exported one ({} bytes, {} restored)",
            source_journal.len(),
            restored_journal.len()
        );
    }
    if source != restored {
        bail!(
            "replay equality: the restored world replays to {restored:?}, the source to {source:?}"
        );
    }
    Ok(())
}

/// Run one restore drill of the world `name` in the data root `root`,
/// signing the export with `signing_key`, working in `scratch` (a
/// directory that must not exist yet; the caller removes it).
///
/// A failing step does not return an error: it yields a
/// [`DrillOutcome::Fail`] record with the reason, so every attempt can be
/// recorded. Nothing in `root` is written.
pub async fn run_drill(
    root: &Path,
    name: &str,
    signing_key: &SigningKey,
    scratch: &Path,
) -> DrillRecord {
    let mut seen = Seen::default();
    let result = drill_steps(root, name, signing_key, scratch, &mut seen).await;
    let reason = match result {
        Ok(()) => None,
        Err(error) => Some(format!("{error:#}")),
    };
    DrillRecord {
        at: now_secs(),
        world: name.to_owned(),
        outcome: if reason.is_none() {
            DrillOutcome::Pass
        } else {
            DrillOutcome::Fail
        },
        reason,
        signed_by: encode_hex(signing_key.verifying_key().as_bytes()),
        export_entries: seen.export_entries,
        source_head: seen.source_head,
        restored_head: seen.restored_head,
    }
}

async fn drill_steps(
    root: &Path,
    name: &str,
    signing_key: &SigningKey,
    scratch: &Path,
    seen: &mut Seen,
) -> Result<()> {
    validate_name(name)?;
    let source_dir = root.join(name);
    if !source_dir.join(JOURNAL_FILE).is_file() {
        bail!("no world named {name:?} in data root {}", root.display());
    }
    if std::fs::symlink_metadata(scratch).is_ok() {
        bail!(
            "scratch {} already exists: drill into a new directory",
            scratch.display()
        );
    }
    let copy_root = scratch.join("source");
    let restore_root = scratch.join("restored");
    let export = scratch.join(format!("{name}.export"));
    std::fs::create_dir_all(&restore_root)
        .with_context(|| format!("creating {}", restore_root.display()))?;
    copy_world(&source_dir, &copy_root.join(name))?;

    let (service, report) = open_in_root(&copy_root, name)
        .await
        .context("replaying the source copy")?;
    seen.source_head = Some(DrillHead::from(&report));
    let exported = export_world_to_file(&service, signing_key, &export).await;
    service
        .world()
        .shutdown()
        .await
        .context("closing the source copy")?;
    let manifest = exported.context("exporting the source copy")?;
    seen.export_entries = Some(manifest.entries);
    seen.source_journal = Some(
        std::fs::read(copy_root.join(name).join(JOURNAL_FILE))
            .context("reading the exported journal")?,
    );

    let source = *signing_key.verifying_key().as_bytes();
    let restored_dir = restore_root.join(&manifest.world);
    import_world_from_file(&export, &restored_dir, source)
        .await
        .context("restoring the export")?;
    seen.restored_journal = Some(
        std::fs::read(restored_dir.join(JOURNAL_FILE)).context("reading the restored journal")?,
    );
    let (restored, report) = open_in_root(&restore_root, &manifest.world)
        .await
        .context("replaying the restored world")?;
    restored
        .world()
        .shutdown()
        .await
        .context("closing the restored world")?;
    seen.restored_head = Some(DrillHead::from(&report));

    match (
        &seen.source_journal,
        &seen.source_head,
        &seen.restored_journal,
        &seen.restored_head,
    ) {
        (Some(source_journal), Some(source), Some(restored_journal), Some(restored)) => {
            replay_equality(source_journal, source, restored_journal, restored)
        }
        _ => bail!("the drill ended before both replays"),
    }
}

/// Copy a world directory, leaving out the journal lock a serving host
/// holds.
fn copy_world(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to).with_context(|| format!("creating {}", to.display()))?;
    for entry in std::fs::read_dir(from).with_context(|| format!("listing {}", from.display()))? {
        let entry = entry.with_context(|| format!("listing {}", from.display()))?;
        let file_name = entry.file_name();
        if file_name.to_string_lossy().ends_with(".lock") {
            continue;
        }
        let target = to.join(&file_name);
        let kind = entry
            .file_type()
            .with_context(|| format!("typing {}", entry.path().display()))?;
        if kind.is_dir() {
            copy_world(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)
                .with_context(|| format!("copying {}", entry.path().display()))?;
        }
    }
    Ok(())
}

/// The drill record file of the data root `root`.
#[must_use]
pub fn drill_record_path(root: &Path) -> PathBuf {
    root.join(DRILL_RECORD_FILE)
}

/// Append `record` to the data root's drill record as one JSON line,
/// synced before returning.
///
/// # Errors
///
/// Fails when the record file cannot be opened, written or synced.
pub fn append_drill_record(root: &Path, record: &DrillRecord) -> Result<()> {
    let path = drill_record_path(root);
    let mut line = serde_json::to_vec(record)?;
    line.push(b'\n');
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    file.write_all(&line)
        .with_context(|| format!("writing {}", path.display()))?;
    file.sync_data()
        .with_context(|| format!("syncing {}", path.display()))
}

/// Every drill record of the data root `root`, oldest first; none when
/// no drill has run.
///
/// # Errors
///
/// Fails when the record file exists but cannot be read or a line does
/// not parse.
pub fn read_drill_records(root: &Path) -> Result<Vec<DrillRecord>> {
    let path = drill_record_path(root);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line).with_context(|| format!("parsing {}", path.display()))
        })
        .collect()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::world::{WorldLimits, WorldScopes};
    use crate::world_limits::RuntimeLimits;
    use crate::world_session::WorldService;
    use crate::world_store::open_world;
    use tempfile::TempDir;

    async fn busy_world(dir: &Path, name: &str) {
        let (service, _) = open_world(
            dir,
            name,
            WorldLimits::default(),
            RuntimeLimits::default(),
            |world| WorldService::new(world, None),
        )
        .await
        .unwrap();
        let (_, ann) = service
            .world()
            .issue("ann", WorldScopes::GUEST)
            .await
            .unwrap();
        service.world().chat(ann.clone(), "hello").await.unwrap();
        service.world().chat(ann, "again").await.unwrap();
        service.world().shutdown().await.unwrap();
    }

    /// Every file under `dir`, with its bytes.
    fn snapshot_tree(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(at) = stack.pop() {
            for entry in std::fs::read_dir(&at).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    stack.push(entry.path());
                } else {
                    out.push((entry.path(), std::fs::read(entry.path()).unwrap()));
                }
            }
        }
        out.sort();
        out
    }

    fn head(sequence: u64) -> DrillHead {
        DrillHead {
            sequence,
            journal_entries: 1,
            module_hash: None,
            replayed_inputs: 0,
            program_error: None,
        }
    }

    #[test]
    fn replay_equality_names_a_differing_journal_or_head() {
        assert!(replay_equality(b"a\n", &head(2), b"a\n", &head(2)).is_ok());
        let journal = replay_equality(b"a\n", &head(2), b"b\n", &head(2))
            .unwrap_err()
            .to_string();
        assert!(journal.contains("replay equality"), "{journal}");
        assert!(journal.contains("journal"), "{journal}");
        let head_differs = replay_equality(b"a\n", &head(2), b"a\n", &head(3))
            .unwrap_err()
            .to_string();
        assert!(head_differs.contains("replay equality"), "{head_differs}");
    }

    #[tokio::test]
    async fn restore_drill_passes_and_leaves_the_source_untouched() {
        let root = TempDir::new().unwrap();
        busy_world(&root.path().join("w"), "w").await;
        let before = snapshot_tree(root.path());
        let key = SigningKey::from_bytes(&[9; 32]);
        let scratch = TempDir::new().unwrap();

        let record = run_drill(root.path(), "w", &key, &scratch.path().join("drill")).await;
        assert_eq!(record.outcome, DrillOutcome::Pass, "{record:?}");
        assert_eq!(record.reason, None);
        assert_eq!(record.world, "w");
        assert_eq!(record.signed_by, encode_hex(key.verifying_key().as_bytes()));
        assert!(record.export_entries.unwrap() >= 3, "{record:?}");
        let source = record.source_head.clone().unwrap();
        assert!(source.sequence >= 2, "{record:?}");
        assert_eq!(record.restored_head, Some(source));
        assert_eq!(snapshot_tree(root.path()), before);

        append_drill_record(root.path(), &record).unwrap();
        append_drill_record(root.path(), &record).unwrap();
        assert_eq!(
            read_drill_records(root.path()).unwrap(),
            vec![record.clone(), record]
        );
    }

    #[tokio::test]
    async fn restore_drill_fails_on_a_damaged_world_and_leaves_it_untouched() {
        let root = TempDir::new().unwrap();
        busy_world(&root.path().join("w"), "w").await;
        let journal = root.path().join("w").join(JOURNAL_FILE);
        let mut bytes = std::fs::read(&journal).unwrap();
        bytes.splice(0..0, b"not json\n".iter().copied());
        std::fs::write(&journal, &bytes).unwrap();
        let before = snapshot_tree(root.path());
        let key = SigningKey::from_bytes(&[9; 32]);
        let scratch = TempDir::new().unwrap();

        let record = run_drill(root.path(), "w", &key, &scratch.path().join("drill")).await;
        assert_eq!(record.outcome, DrillOutcome::Fail, "{record:?}");
        assert!(record.reason.is_some());
        assert_eq!(record.restored_head, None);
        assert_eq!(snapshot_tree(root.path()), before);

        append_drill_record(root.path(), &record).unwrap();
        assert_eq!(read_drill_records(root.path()).unwrap(), vec![record]);
    }

    #[tokio::test]
    async fn restore_drill_refuses_an_unknown_world_or_a_used_scratch() {
        let root = TempDir::new().unwrap();
        busy_world(&root.path().join("w"), "w").await;
        let key = SigningKey::from_bytes(&[9; 32]);
        let scratch = TempDir::new().unwrap();

        let missing = run_drill(root.path(), "nope", &key, &scratch.path().join("a")).await;
        assert_eq!(missing.outcome, DrillOutcome::Fail);
        assert!(missing.reason.unwrap().contains("no world"));

        let used = run_drill(root.path(), "w", &key, scratch.path()).await;
        assert_eq!(used.outcome, DrillOutcome::Fail);
        assert!(used.reason.unwrap().contains("already exists"));
        assert_eq!(read_drill_records(root.path()).unwrap(), Vec::new());
    }
}
