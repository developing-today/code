//! `id world owner <dir> [--since <duration>] [--json]`: one durable
//! world's owner report, read locally (#11777, design #6308 §§2, 4, 6–8).
//!
//! The command is an adapter: it checks that the caller owns the world
//! directory, measures its storage with
//! [`id_worlds::world_storage::inventory`], streams its journal through
//! [`id_worlds::world_owner_report::project`] and prints the result. It
//! owns no aggregation. It contacts no host and never wakes a world, so
//! the live-only sections (connected members, program runs) are always
//! unavailable here, and it writes nothing.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use id_worlds::world::JournalEntry;
use id_worlds::world_owner_report::{
    Measured, OwnerReport, ReportLimits, ReportRequest, ReportWindow, project,
};
use id_worlds::world_storage::{JOURNAL_FILE, StorageInventory, WalkLimits, inventory};

/// Refuse a world directory the caller does not own. A world's journal
/// holds its participant list, so the local owner (the user the world
/// directory belongs to) or root may read the report; anyone else is
/// refused even when the filesystem would let them read the files.
#[cfg(unix)]
fn require_local_owner(dir: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = std::fs::metadata(dir).with_context(|| format!("read {}", dir.display()))?;
    // SAFETY: geteuid has no preconditions and cannot fail.
    #[allow(unsafe_code)]
    let euid = unsafe { libc::geteuid() };
    check_owner(metadata.uid(), euid, dir)
}

#[cfg(not(unix))]
fn require_local_owner(dir: &Path) -> Result<()> {
    std::fs::metadata(dir).with_context(|| format!("read {}", dir.display()))?;
    Ok(())
}

/// The ownership rule itself, apart from the system calls, so it can be
/// tested for any pair of ids.
#[cfg_attr(not(unix), allow(dead_code))]
fn check_owner(dir_uid: u32, euid: u32, dir: &Path) -> Result<()> {
    if euid == 0 || euid == dir_uid {
        return Ok(());
    }
    bail!(
        "permission denied: {} belongs to another user; only the world's owner or root may read its owner report",
        dir.display()
    )
}

/// The world id from the journal's first line, read alone so the
/// projection can check every later line against it.
fn journal_world_id(path: &Path) -> Result<String> {
    let file = File::open(path).with_context(|| format!("read {}", path.display()))?;
    let mut first = Vec::new();
    BufReader::new(file)
        .take(64 * 1024)
        .read_until(b'\n', &mut first)
        .with_context(|| format!("read {}", path.display()))?;
    if !first.ends_with(b"\n") {
        bail!(
            "{} has no complete header line; it is not a world journal",
            path.display()
        );
    }
    match serde_json::from_slice::<JournalEntry>(&first) {
        Ok(JournalEntry::Created { world_id, .. }) => Ok(world_id),
        _ => bail!(
            "{} does not start with a world journal header",
            path.display()
        ),
    }
}

/// Build the owner report for the world in `dir` as of `now_unix_ms`,
/// over the `since_secs` before it.
pub(super) fn owner_report(dir: &Path, since_secs: u64, now_unix_ms: u64) -> Result<OwnerReport> {
    require_local_owner(dir)?;
    let journal = dir.join(JOURNAL_FILE);
    let world_id = journal_world_id(&journal)?;
    let storage: StorageInventory = inventory(dir, WalkLimits::default())
        .with_context(|| format!("measure storage of {}", dir.display()))?;
    let request = ReportRequest {
        world_id,
        generated_at_unix_ms: now_unix_ms,
        window: ReportWindow {
            from_unix_ms: now_unix_ms.saturating_sub(since_secs.saturating_mul(1_000)),
            to_unix_ms: now_unix_ms,
        },
        limits: ReportLimits::default(),
    };
    let file = File::open(&journal).with_context(|| format!("read {}", journal.display()))?;
    let report = project(BufReader::new(file), &request, Some(storage))
        .with_context(|| format!("{} cannot be reported on", journal.display()))?;
    Ok(report.with_live(None))
}

fn measured(value: &Measured<u64>) -> String {
    match value {
        Measured::Known(value) => value.to_string(),
        Measured::Unavailable(reason) => format!("unavailable ({})", reason.describe()),
    }
}

/// The text form: the same values and labels as the JSON form.
pub(super) fn render_text(report: &OwnerReport, since_secs: u64) -> String {
    let joins = &report.joins;
    let mut out = Vec::new();
    out.push(format!("world: {}", report.world_id));
    out.push(format!(
        "window: the {since_secs}s before {} (unix ms)",
        report.generated_at_unix_ms
    ));
    out.push(format!(
        "joins: {} ({} host-issued, {} delegated, {} revoked, {} accounts){}",
        joins.count,
        joins.host_issued,
        joins.delegated,
        joins.revoked,
        joins.distinct_subjects,
        if joins.complete { "" } else { ", incomplete" }
    ));
    if let Some(from) = &report.journal.forked_from {
        let inherited = joins
            .inherited_from_fork
            .map_or_else(|| "unknown".to_owned(), |n| n.to_string());
        out.push(format!("forked from: {from} ({inherited} joins inherited)"));
    }
    out.push(format!(
        "joins in window: {}{}",
        measured(&joins.in_window),
        if joins.in_window.value().is_some() && !joins.in_window_complete {
            ", incomplete: the window starts before join times were recorded"
        } else {
            ""
        }
    ));
    out.push(format!(
        "join times recorded since: {}{}",
        measured(&joins.available_since),
        if joins.untimed > 0 {
            format!(" ({} joins have no time)", joins.untimed)
        } else {
            String::new()
        }
    ));
    out.push(format!(
        "revokes in window: {}{}",
        measured(&joins.revoked_in_window),
        if joins.revoked_untimed > 0 && joins.revoked_in_window.value().is_some() {
            format!(" ({} revokes have no time)", joins.revoked_untimed)
        } else {
            String::new()
        }
    ));
    out.push(format!(
        "active members: {}",
        measured(&report.members.active_count)
    ));
    out.push(format!(
        "program runs: {}",
        measured(&report.program_runs.count)
    ));
    match &report.storage {
        Measured::Known(storage) => out.push(format!(
            "storage: {} bytes (journal {}, modules {}, other {}), best effort{}",
            storage.total_bytes,
            storage.journal.bytes,
            storage.modules.bytes,
            storage.other.bytes,
            if storage.truncated {
                ", walk stopped early"
            } else {
                ""
            }
        )),
        Measured::Unavailable(reason) => {
            out.push(format!("storage: unavailable ({})", reason.describe()));
        }
    }
    if !joins.recent.is_empty() {
        let recent: Vec<String> = joins
            .recent
            .iter()
            .map(|join| {
                let mut tags = Vec::new();
                if join.delegated {
                    tags.push("delegated");
                }
                if join.revoked {
                    tags.push("revoked");
                }
                let mut label = if tags.is_empty() {
                    join.participant_id.to_string()
                } else {
                    format!("{} ({})", join.participant_id, tags.join(", "))
                };
                if let Some(at) = join.issued_at_unix_ms {
                    let _ = write!(label, " joined {at}");
                }
                if let Some(at) = join.revoked_at_unix_ms {
                    let _ = write!(label, " revoked {at}");
                }
                label
            })
            .collect();
        out.push(format!("recent joins, newest first: {}", recent.join("; ")));
    }
    if report.journal.torn_tail {
        out.push("note: the journal ends in a cut-short line, which was not counted".to_owned());
    }
    out.join("\n")
}

/// `id world owner`.
pub(super) fn print_world_owner(dir: &Path, since_secs: u64, json: bool) -> Result<()> {
    let now_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        });
    let report = owner_report(dir, since_secs, now_unix_ms)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("{}", render_text(&report, since_secs));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::io::Write as _;

    use id_worlds::world::{JournalEntry, Participant};
    use id_worlds::world_owner_report::{UnavailableReason, check_schema_version};

    use super::*;

    fn world_dir(entries: &[JournalEntry]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let mut file = File::create(dir.path().join(JOURNAL_FILE)).unwrap();
        for entry in entries {
            serde_json::to_writer(&mut file, entry).unwrap();
            file.write_all(b"\n").unwrap();
        }
        std::fs::create_dir_all(dir.path().join("modules")).unwrap();
        std::fs::write(
            dir.path()
                .join("modules")
                .join(format!("{}.wasm", "a".repeat(64))),
            b"wasm",
        )
        .unwrap();
        dir
    }

    fn created() -> JournalEntry {
        JournalEntry::Created {
            world_id: "w-cli".to_owned(),
            version: 1,
        }
    }

    fn issued(id: u64, parent: Option<u64>) -> JournalEntry {
        JournalEntry::Issued {
            participant: Participant {
                id,
                display_name: format!("player-{id}"),
            },
            digest: format!("{id:064x}"),
            scopes: 1,
            parent,
            expires_at: None,
            uses: None,
            used: 0,
            subject: None,
            issued_at: None,
        }
    }

    fn issued_at(id: u64, at: u64) -> JournalEntry {
        let JournalEntry::Issued {
            participant,
            digest,
            scopes,
            parent,
            expires_at,
            uses,
            used,
            subject,
            ..
        } = issued(id, None)
        else {
            unreachable!()
        };
        JournalEntry::Issued {
            participant,
            digest,
            scopes,
            parent,
            expires_at,
            uses,
            used,
            subject,
            issued_at: Some(at),
        }
    }

    #[test]
    fn text_form_shows_join_and_revoke_times() {
        let v = id_worlds::world::JOURNAL_VERSION_JOIN_TIMES;
        let dir = world_dir(&[
            JournalEntry::Created {
                world_id: "w-cli".to_owned(),
                version: v,
            },
            issued(1, None),
            issued_at(2, 4_000),
            JournalEntry::Revoked {
                participant_id: 2,
                by: None,
                revoked_at: Some(4_500),
            },
        ]);
        let report = owner_report(dir.path(), 3, 5_000).unwrap();
        let text = render_text(&report, 3);
        assert!(text.contains("joins in window: 1, incomplete"), "{text}");
        assert!(
            text.contains("join times recorded since: 4000 (1 joins have no time)"),
            "{text}"
        );
        assert!(text.contains("revokes in window: 1"), "{text}");
        assert!(
            text.contains("recent joins, newest first: 2 (revoked) joined 4000 revoked 4500; 1"),
            "{text}"
        );
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["joins"]["available_since"]["value"], 4_000);
        assert_eq!(
            json["joins"]["recent"][1]["issued_at_unix_ms"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn reports_a_local_world_offline() {
        let dir = world_dir(&[created(), issued(1, None), issued(2, Some(1))]);
        let report = owner_report(dir.path(), 86_400, 100_000_000).unwrap();
        assert_eq!(report.world_id, "w-cli");
        assert_eq!(report.joins.count, 2);
        assert_eq!(report.window.from_unix_ms, 100_000_000 - 86_400_000);
        assert_eq!(report.window.to_unix_ms, 100_000_000);
        // Offline: live sections are unavailable, never zero.
        assert_eq!(
            report.members.active_count.reason(),
            Some(UnavailableReason::LiveStateNotProvided)
        );
        assert_eq!(
            report.program_runs.count.reason(),
            Some(UnavailableReason::ProgramRunsNotRecorded)
        );
        let Measured::Known(storage) = &report.storage else {
            panic!("storage was measured");
        };
        assert_eq!(storage.modules.bytes, 4);
        assert_eq!(storage.modules.files, 1);
        assert!(storage.journal.bytes > 0);
    }

    #[test]
    fn json_form_carries_the_supported_schema() {
        let dir = world_dir(&[created(), issued(1, None)]);
        let report = owner_report(dir.path(), 60, 5_000).unwrap();
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(check_schema_version(&json), Ok(1));
        assert_eq!(
            json["joins"]["in_window"]["reason"],
            "join_time_not_recorded"
        );
        // Repeating the report changes nothing durable.
        let again = serde_json::to_value(owner_report(dir.path(), 60, 5_000).unwrap()).unwrap();
        assert_eq!(json["joins"], again["joins"]);
        assert_eq!(json["journal"], again["journal"]);
    }

    #[test]
    fn text_form_uses_the_same_values_and_labels() {
        let dir = world_dir(&[created(), issued(1, None), issued(2, Some(1))]);
        let report = owner_report(dir.path(), 3_600, 5_000).unwrap();
        let text = render_text(&report, 3_600);
        assert!(text.contains("world: w-cli"), "{text}");
        assert!(
            text.contains("joins: 2 (1 host-issued, 1 delegated, 0 revoked, 0 accounts)"),
            "{text}"
        );
        assert!(
            text.contains(
                "joins in window: unavailable (the journal does not record when anyone joined)"
            ),
            "{text}"
        );
        assert!(text.contains("active members: unavailable"), "{text}");
        assert!(
            text.contains("recent joins, newest first: 2 (delegated); 1"),
            "{text}"
        );
        // Participant ids only: no display names.
        assert!(!text.contains("player-"), "{text}");
    }

    #[test]
    fn refuses_what_is_not_a_world() {
        let empty = tempfile::tempdir().unwrap();
        assert!(owner_report(empty.path(), 60, 0).is_err());
        let headless = world_dir(&[issued(1, None)]);
        let error = owner_report(headless.path(), 60, 0).unwrap_err();
        assert!(error.to_string().contains("header"), "{error:#}");
        let newer = world_dir(&[JournalEntry::Created {
            world_id: "w-cli".to_owned(),
            version: id_worlds::world::MAX_JOURNAL_VERSION + 1,
        }]);
        let error = owner_report(newer.path(), 60, 0).unwrap_err();
        assert!(
            format!("{error:#}").contains("unsupported world journal version"),
            "{error:#}"
        );
    }

    #[test]
    fn only_the_owner_or_root_may_read() {
        let dir = Path::new("/worlds/w");
        assert!(check_owner(1000, 1000, dir).is_ok());
        assert!(check_owner(1000, 0, dir).is_ok());
        let error = check_owner(1000, 1001, dir).unwrap_err();
        assert!(
            error.to_string().starts_with("permission denied"),
            "{error}"
        );
    }
}
