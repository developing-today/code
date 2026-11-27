//! Scheduled snapshots and retention for hosted worlds (#13739, packet Q1
//! of the hosted-worlds v2 outline on #6284, on top of the P3 signed
//! export of #10695).
//!
//! A snapshot set is one P3 export file ([`crate::world_export`]): a
//! signed, self-describing copy of one world's journal, directory facts
//! and active module. This module adds only the schedule and the policy:
//!
//! - [`SnapshotPolicy`]: a cadence (`every_secs`, `0` is off), a set count
//!   to keep, and an optional maximum age. The host file carries one
//!   host-wide default and an optional per-world override
//!   ([`crate::host::HostFile::snapshot_policy`]).
//! - Where sets live: `<snapshot root>/<world>/`, the snapshot root being
//!   `<data root>/.snapshots` unless the host file names another
//!   directory (off-host storage, for example a mounted backup disk).
//! - [`is_due`] and [`prune_plan`]: pure decisions over the sets on disk,
//!   so the schedule survives restarts with no state of its own. File
//!   names carry the time and the journal entry count, and the directory
//!   listing is the whole record.
//! - Pruning removes whole set files and nothing else. A journal is never
//!   opened for writing here: the append-only invariant holds by
//!   construction.
//! - [`usage`]: per-world set counts and bytes, which the host census adds
//!   to each lifecycle state's capacity.
//!
//! Only live worlds are snapshotted. An asleep or archived world cannot
//! change, so its newest set stays current until it wakes. A due world
//! whose journal has not grown since its newest set gets a fresh set that
//! replaces that one, so an idle world never fills its retention with
//! copies of the same journal. Encryption at rest stays a deployment
//! property: sets are the P3 format, unchanged.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// Directory under the data root that holds snapshot sets when the host
/// file names no other place. Reserved: no world may take this name.
pub const SNAPSHOT_DIR_NAME: &str = ".snapshots";

/// File extension of a snapshot set.
pub const SNAPSHOT_EXTENSION: &str = "idexport";

/// The shortest cadence a policy may ask for.
pub const MIN_SNAPSHOT_INTERVAL_SECS: u64 = 60;

/// How often a running host checks which worlds are due.
pub const SNAPSHOT_TICK_SECS: u64 = 60;

/// Cadence and retention for one world, or the host-wide default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotPolicy {
    /// Seconds between sets while the world is live. `0` turns scheduled
    /// snapshots off.
    pub every_secs: u64,
    /// Newest sets kept; older ones are pruned. At least 1 when on.
    pub keep: u32,
    /// Sets older than this many seconds are pruned too. The newest set is
    /// always kept, however old.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age_secs: Option<u64>,
}

impl SnapshotPolicy {
    /// Scheduled snapshots off.
    pub const OFF: SnapshotPolicy = SnapshotPolicy {
        every_secs: 0,
        keep: 0,
        max_age_secs: None,
    };

    /// Whether this policy takes sets.
    #[must_use]
    pub fn is_on(&self) -> bool {
        self.every_secs > 0
    }

    /// Check the policy is one a host can follow.
    ///
    /// # Errors
    ///
    /// Refuses an on policy with a cadence under
    /// [`MIN_SNAPSHOT_INTERVAL_SECS`], a keep count of 0, or a maximum age
    /// of 0.
    pub fn validate(&self) -> Result<()> {
        if !self.is_on() {
            return Ok(());
        }
        if self.every_secs < MIN_SNAPSHOT_INTERVAL_SECS {
            bail!(
                "a snapshot cadence must be at least {MIN_SNAPSHOT_INTERVAL_SECS} seconds, not {}",
                self.every_secs
            );
        }
        if self.keep == 0 {
            bail!("a snapshot policy must keep at least one set");
        }
        if self.max_age_secs == Some(0) {
            bail!("a snapshot maximum age must be more than 0 seconds");
        }
        Ok(())
    }
}

/// One snapshot set on disk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SnapshotSet {
    /// The world the set belongs to.
    pub world: String,
    /// The set file.
    pub path: PathBuf,
    /// Unix seconds (UTC) when the set was taken.
    pub at_secs: u64,
    /// Journal entries in the set, the `Created` header included.
    pub entries: u64,
    /// File size.
    pub bytes: u64,
}

/// Snapshot set count and bytes for one world.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SnapshotUsage {
    /// Sets on disk.
    pub sets: u64,
    /// Their total size.
    pub bytes: u64,
}

/// The file name of a set taken at `at_secs` holding `entries` journal
/// entries. Zero-padded, so names sort in time order.
#[must_use]
pub fn set_file_name(at_secs: u64, entries: u64) -> String {
    format!("{at_secs:020}-{entries:020}.{SNAPSHOT_EXTENSION}")
}

/// The time and entry count a set file name carries, or `None` for any
/// other file (an export still being written, an operator's own file).
#[must_use]
pub fn parse_set_name(name: &str) -> Option<(u64, u64)> {
    let stem = name.strip_suffix(&format!(".{SNAPSHOT_EXTENSION}"))?;
    let (at, entries) = stem.split_once('-')?;
    if at.len() != 20 || entries.len() != 20 {
        return None;
    }
    if !at
        .bytes()
        .chain(entries.bytes())
        .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    Some((at.parse().ok()?, entries.parse().ok()?))
}

/// The directory holding `world`'s sets under `snapshot_root`.
#[must_use]
pub fn world_snapshot_dir(snapshot_root: &Path, world: &str) -> PathBuf {
    snapshot_root.join(world)
}

/// `world`'s sets under `snapshot_root`, oldest first. A missing
/// directory is no sets, not an error; files that are not sets are
/// ignored.
///
/// # Errors
///
/// Fails when the directory exists but cannot be listed.
pub fn list_sets(snapshot_root: &Path, world: &str) -> Result<Vec<SnapshotSet>> {
    let dir = world_snapshot_dir(snapshot_root, world);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("listing snapshots in {}", dir.display()));
        }
    };
    let mut sets = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("listing snapshots in {}", dir.display()))?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some((at_secs, set_entries)) = parse_set_name(&name) else {
            continue;
        };
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        sets.push(SnapshotSet {
            world: world.to_owned(),
            path: entry.path(),
            at_secs,
            entries: set_entries,
            bytes: meta.len(),
        });
    }
    sets.sort_by(|a, b| (a.at_secs, a.entries).cmp(&(b.at_secs, b.entries)));
    Ok(sets)
}

/// Set count and bytes for `world`. An unreadable directory counts as
/// none: the census never fails on snapshots.
#[must_use]
pub fn usage(snapshot_root: &Path, world: &str) -> SnapshotUsage {
    list_sets(snapshot_root, world)
        .unwrap_or_default()
        .iter()
        .fold(SnapshotUsage::default(), |mut total, set| {
            total.sets += 1;
            total.bytes = total.bytes.saturating_add(set.bytes);
            total
        })
}

/// Whether a world whose newest set is `newest` is due a new one at
/// `now_secs` under `policy`. A world with no set is due at once.
#[must_use]
pub fn is_due(policy: &SnapshotPolicy, newest: Option<&SnapshotSet>, now_secs: u64) -> bool {
    if !policy.is_on() {
        return false;
    }
    newest.is_none_or(|set| now_secs.saturating_sub(set.at_secs) >= policy.every_secs)
}

/// The sets `policy` no longer keeps at `now_secs`, given `sets` oldest
/// first: everything beyond the newest `keep`, and everything older than
/// `max_age_secs`. The newest set is never in the plan. An off policy
/// prunes nothing: turning snapshots off keeps the sets already taken.
#[must_use]
pub fn prune_plan(
    sets: &[SnapshotSet],
    policy: &SnapshotPolicy,
    now_secs: u64,
) -> Vec<SnapshotSet> {
    if !policy.is_on() || sets.len() <= 1 {
        return Vec::new();
    }
    let keep = usize::try_from(policy.keep.max(1)).unwrap_or(usize::MAX);
    let newest = sets.len() - 1;
    sets.iter()
        .enumerate()
        .filter(|(index, set)| {
            if *index == newest {
                return false;
            }
            let beyond_count = sets.len() - index > keep;
            let too_old = policy
                .max_age_secs
                .is_some_and(|max| now_secs.saturating_sub(set.at_secs) > max);
            beyond_count || too_old
        })
        .map(|(_, set)| set.clone())
        .collect()
}

/// Remove the set files in `plan`. A set already gone counts as removed.
///
/// # Errors
///
/// Fails on the first file that exists and cannot be removed; the sets
/// before it are gone, the rest stay.
pub fn remove_sets(plan: &[SnapshotSet]) -> Result<()> {
    for set in plan {
        match std::fs::remove_file(&set.path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("removing snapshot {}", set.path.display()));
            }
        }
    }
    Ok(())
}

/// What one pass over the live worlds did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SnapshotRun {
    /// New sets taken.
    pub taken: Vec<SnapshotSet>,
    /// Sets removed: replaced by a newer set of the same journal, or
    /// beyond the policy's retention.
    pub removed: Vec<SnapshotSet>,
    /// Worlds whose pass failed, with why. One world failing never stops
    /// the others.
    pub failed: Vec<(String, String)>,
}

impl SnapshotRun {
    /// Whether the pass did nothing at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.taken.is_empty() && self.removed.is_empty() && self.failed.is_empty()
    }
}

/// Parse a duration such as `90`, `90s`, `15m`, `6h` or `7d` into seconds.
///
/// # Errors
///
/// Refuses anything else, and values that overflow.
pub fn parse_duration_secs(text: &str) -> Result<u64> {
    let text = text.trim();
    let (digits, unit) = match text.char_indices().find(|(_, c)| !c.is_ascii_digit()) {
        Some((at, _)) => text.split_at(at),
        None => (text, ""),
    };
    let value: u64 = digits
        .parse()
        .with_context(|| format!("{text:?} is not a duration like 90s, 15m, 6h or 7d"))?;
    let scale = match unit {
        "" | "s" => 1,
        "m" => 60,
        "h" => 3_600,
        "d" => 86_400,
        _ => bail!("{text:?} is not a duration like 90s, 15m, 6h or 7d"),
    };
    value
        .checked_mul(scale)
        .with_context(|| format!("{text:?} is too long a duration"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn set(at_secs: u64, entries: u64) -> SnapshotSet {
        SnapshotSet {
            world: "w".to_owned(),
            path: PathBuf::from(set_file_name(at_secs, entries)),
            at_secs,
            entries,
            bytes: 10,
        }
    }

    fn policy(every_secs: u64, keep: u32, max_age_secs: Option<u64>) -> SnapshotPolicy {
        SnapshotPolicy {
            every_secs,
            keep,
            max_age_secs,
        }
    }

    #[test]
    fn set_names_round_trip_and_sort_in_time_order() {
        let name = set_file_name(1_700_000_000, 42);
        assert_eq!(parse_set_name(&name), Some((1_700_000_000, 42)));
        assert!(set_file_name(9, 1) < set_file_name(10, 1));
        for other in [
            "notes.txt",
            "1-2.idexport",
            ".00000000000000000001-00000000000000000002.idexport.partial",
            "0000000000000000000a-00000000000000000002.idexport",
            "00000000000000000001-00000000000000000002.export",
        ] {
            assert_eq!(parse_set_name(other), None, "{other}");
        }
    }

    #[test]
    fn policies_below_the_floor_or_keeping_nothing_are_refused() {
        assert!(SnapshotPolicy::OFF.validate().is_ok());
        assert!(policy(3_600, 3, None).validate().is_ok());
        assert!(policy(59, 3, None).validate().is_err());
        assert!(policy(3_600, 0, None).validate().is_err());
        assert!(policy(3_600, 1, Some(0)).validate().is_err());
    }

    #[test]
    fn a_world_is_due_when_it_has_no_set_or_its_newest_is_a_cadence_old() {
        let hourly = policy(3_600, 3, None);
        assert!(is_due(&hourly, None, 0));
        assert!(!is_due(&hourly, Some(&set(1_000, 1)), 4_599));
        assert!(is_due(&hourly, Some(&set(1_000, 1)), 4_600));
        assert!(!is_due(&SnapshotPolicy::OFF, None, 99));
        // A clock that went backwards waits rather than snapshotting.
        assert!(!is_due(&hourly, Some(&set(5_000, 1)), 4_000));
    }

    #[test]
    fn prune_keeps_the_newest_count_and_drops_whole_old_sets() {
        let sets: Vec<_> = (1..=5).map(|i| set(i * 100, i)).collect();
        let plan = prune_plan(&sets, &policy(60, 2, None), 600);
        let gone: Vec<u64> = plan.iter().map(|s| s.at_secs).collect();
        assert_eq!(gone, vec![100, 200, 300]);
        // Age prunes inside the count, but never the newest set.
        let plan = prune_plan(&sets, &policy(60, 5, Some(150)), 600);
        let gone: Vec<u64> = plan.iter().map(|s| s.at_secs).collect();
        assert_eq!(gone, vec![100, 200, 300, 400]);
        let plan = prune_plan(&sets, &policy(60, 5, Some(1)), 10_000);
        assert_eq!(plan.len(), 4);
        assert!(plan.iter().all(|s| s.at_secs != 500));
        // Off keeps what was taken.
        assert!(prune_plan(&sets, &SnapshotPolicy::OFF, 10_000).is_empty());
    }

    #[test]
    fn listing_reads_sets_oldest_first_and_ignores_other_files() {
        let root = tempfile::TempDir::new().unwrap();
        let dir = world_snapshot_dir(root.path(), "alpha");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(set_file_name(20, 3)), b"abc").unwrap();
        std::fs::write(dir.join(set_file_name(10, 2)), b"ab").unwrap();
        std::fs::write(dir.join("readme.txt"), b"not a set").unwrap();
        std::fs::write(dir.join(format!(".{}.partial", set_file_name(30, 4))), b"x").unwrap();
        let sets = list_sets(root.path(), "alpha").unwrap();
        assert_eq!(
            sets.iter()
                .map(|s| (s.at_secs, s.entries, s.bytes))
                .collect::<Vec<_>>(),
            vec![(10, 2, 2), (20, 3, 3)]
        );
        assert_eq!(
            usage(root.path(), "alpha"),
            SnapshotUsage { sets: 2, bytes: 5 }
        );
        assert!(list_sets(root.path(), "nobody").unwrap().is_empty());
        remove_sets(&sets[..1]).unwrap();
        remove_sets(&sets[..1]).unwrap();
        assert_eq!(usage(root.path(), "alpha").sets, 1);
    }

    #[test]
    fn durations_take_seconds_minutes_hours_and_days() {
        assert_eq!(parse_duration_secs("90").unwrap(), 90);
        assert_eq!(parse_duration_secs("90s").unwrap(), 90);
        assert_eq!(parse_duration_secs("15m").unwrap(), 900);
        assert_eq!(parse_duration_secs("6h").unwrap(), 21_600);
        assert_eq!(parse_duration_secs("7d").unwrap(), 604_800);
        for bad in ["", "h", "6w", "-1", "1.5h"] {
            assert!(parse_duration_secs(bad).is_err(), "{bad}");
        }
    }
}
