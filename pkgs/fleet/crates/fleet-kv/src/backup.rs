//! Host-local snapshot and restore for the `fleet kv` store (#13314,
//! acceptance 6).
//!
//! A snapshot is one self-contained SQLite file written with `VACUUM INTO`:
//! it holds every committed row, including rows that still live only in the
//! write-ahead log, so the live database file and its WAL never have to be
//! copied by hand. Before the copy is taken the store records a manifest
//! row (`kv_backup_manifest`) that travels inside the snapshot and pins every
//! live working file (`kv_backup_pins`: content id, length, hash). Phase 1
//! keeps every value inline, so the pin list is empty until streamed working
//! files land; the table exists now so that a restore always knows what it
//! must re-link.
//!
//! The snapshot file appears atomically: it is written under a `.partial`
//! name, synced, then renamed. A restore never overwrites an existing store.

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, params};

use crate::{KvError, LocalStore, SCHEMA_VERSION, sql, unsigned};

/// A working file pinned by a manifest row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinnedFile {
    pub content_id: String,
    pub length: u64,
    pub hash: String,
}

/// What a snapshot or a restore recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    /// Monotone id of the manifest row inside the store.
    pub id: u64,
    pub taken_at_ms: i64,
    pub schema_version: i64,
    /// Overlay rows (live heads and masking tombstones) at snapshot time.
    pub overlay_rows: u64,
    pub pinned_files: Vec<PinnedFile>,
}

#[derive(Debug)]
pub enum BackupError {
    /// The snapshot or restore target already exists; nothing was written.
    TargetExists(PathBuf),
    /// The file is not a fleet-kv snapshot (no manifest row).
    NotASnapshot(PathBuf),
    /// The snapshot was written by a newer schema than this binary knows.
    UnknownSchemaVersion(i64),
    /// `PRAGMA integrity_check` did not report `ok`.
    Corrupt(String),
    Io(io::Error),
    Store(KvError),
}

impl BackupError {
    /// Stable typed refusal code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::TargetExists(_) => "backup_target_exists",
            Self::NotASnapshot(_) => "not_a_snapshot",
            Self::UnknownSchemaVersion(_) => "unknown_schema_version",
            Self::Corrupt(_) => "snapshot_corrupt",
            Self::Io(_) => "io_error",
            Self::Store(error) => error.code(),
        }
    }
}

impl std::fmt::Display for BackupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TargetExists(path) => {
                write!(f, "{} already exists; nothing was written", path.display())
            }
            Self::NotASnapshot(path) => {
                write!(f, "{} has no fleet-kv backup manifest", path.display())
            }
            Self::UnknownSchemaVersion(version) => write!(
                f,
                "snapshot schema version {version} is newer than this binary ({SCHEMA_VERSION})"
            ),
            Self::Corrupt(detail) => write!(f, "snapshot failed its integrity check: {detail}"),
            Self::Io(error) => write!(f, "{error}"),
            Self::Store(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for BackupError {}

impl From<KvError> for BackupError {
    fn from(error: KvError) -> Self {
        Self::Store(error)
    }
}

impl From<io::Error> for BackupError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub(crate) fn migrate_to_v3(tx: &Transaction<'_>) -> Result<(), KvError> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS kv_backup_manifest (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             taken_at_ms INTEGER NOT NULL,
             schema_version INTEGER NOT NULL,
             overlay_rows INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS kv_backup_pins (
             manifest_id INTEGER NOT NULL REFERENCES kv_backup_manifest(id),
             content_id TEXT NOT NULL,
             length INTEGER NOT NULL,
             hash TEXT NOT NULL,
             PRIMARY KEY(manifest_id, content_id)
         ) WITHOUT ROWID;",
    )
    .map_err(sql)
}

impl LocalStore {
    /// Write a consistent snapshot of the whole host store to `dest`.
    ///
    /// Records a manifest row first, so the snapshot carries the manifest
    /// that describes it, then copies every committed row (WAL included)
    /// into `dest` with `VACUUM INTO`. Refuses to overwrite `dest`.
    pub fn snapshot(&self, dest: &Path) -> Result<Manifest, BackupError> {
        if dest.exists() {
            return Err(BackupError::TargetExists(dest.to_path_buf()));
        }
        let manifest = self.record_manifest()?;
        let partial = partial_path(dest);
        remove_if_present(&partial)?;
        self.connection
            .execute(
                "VACUUM INTO ?1",
                params![partial.to_string_lossy().as_ref()],
            )
            .map_err(sql)?;
        // A snapshot is one file: rollback-journal mode, so it can be read
        // (and inspected read-only) without a WAL or shared-memory sidecar.
        // The restored store switches back to WAL when it is opened.
        Connection::open(&partial)
            .and_then(|copy| copy.pragma_update(None, "journal_mode", "DELETE"))
            .map_err(sql)?;
        publish(&partial, dest)?;
        Ok(manifest)
    }

    /// The newest manifest row in this store, if a snapshot was ever taken.
    pub fn latest_manifest(&self) -> Result<Option<Manifest>, BackupError> {
        read_latest_manifest(&self.connection)
    }

    fn record_manifest(&self) -> Result<Manifest, BackupError> {
        let tx = self.connection.unchecked_transaction().map_err(sql)?;
        let overlay_rows: i64 = tx
            .query_row("SELECT COUNT(*) FROM kv_local", [], |row| row.get(0))
            .map_err(sql)?;
        let taken_at_ms = now_ms();
        tx.execute(
            "INSERT INTO kv_backup_manifest(taken_at_ms, schema_version, overlay_rows)
             VALUES (?1, ?2, ?3)",
            params![taken_at_ms, SCHEMA_VERSION, overlay_rows],
        )
        .map_err(sql)?;
        // Phase 1 stores every value inline: there are no working files to
        // pin yet. Streamed working files add their rows here.
        tx.commit().map_err(sql)?;
        read_latest_manifest(&self.connection)?
            .ok_or_else(|| BackupError::Store(KvError::Sql(rusqlite::Error::QueryReturnedNoRows)))
    }
}

/// Restore `snapshot` to `target`, the store path of a fresh host.
///
/// Checks that the file is a fleet-kv snapshot this binary understands and
/// that it passes `PRAGMA integrity_check`, then copies it into place
/// atomically. Refuses to overwrite an existing store (or its WAL).
pub fn restore(snapshot: &Path, target: &Path) -> Result<Manifest, BackupError> {
    for path in [target.to_path_buf(), sidecar(target, "-wal")] {
        if path.exists() {
            return Err(BackupError::TargetExists(path));
        }
    }
    let manifest = inspect(snapshot)?;
    let partial = partial_path(target);
    remove_if_present(&partial)?;
    fs::copy(snapshot, &partial)?;
    publish(&partial, target)?;
    Ok(manifest)
}

/// Validate a snapshot file without changing anything and return the
/// manifest it carries.
pub fn inspect(snapshot: &Path) -> Result<Manifest, BackupError> {
    let connection = Connection::open_with_flags(
        snapshot,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| BackupError::NotASnapshot(snapshot.to_path_buf()))?;
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|_| BackupError::NotASnapshot(snapshot.to_path_buf()))?;
    if version > SCHEMA_VERSION {
        return Err(BackupError::UnknownSchemaVersion(version));
    }
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|error| BackupError::Corrupt(error.to_string()))?;
    if integrity != "ok" {
        return Err(BackupError::Corrupt(integrity));
    }
    read_latest_manifest(&connection)
        .map_err(|_| BackupError::NotASnapshot(snapshot.to_path_buf()))?
        .ok_or_else(|| BackupError::NotASnapshot(snapshot.to_path_buf()))
}

fn read_latest_manifest(connection: &Connection) -> Result<Option<Manifest>, BackupError> {
    let row = connection
        .query_row(
            "SELECT id, taken_at_ms, schema_version, overlay_rows
             FROM kv_backup_manifest ORDER BY id DESC LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()
        .map_err(sql)?;
    let Some((id, taken_at_ms, schema_version, overlay_rows)) = row else {
        return Ok(None);
    };
    let mut statement = connection
        .prepare(
            "SELECT content_id, length, hash FROM kv_backup_pins
             WHERE manifest_id=?1 ORDER BY content_id",
        )
        .map_err(sql)?;
    let pinned_files = statement
        .query_map(params![id], |row| {
            Ok(PinnedFile {
                content_id: row.get(0)?,
                length: unsigned(row.get(1)?),
                hash: row.get(2)?,
            })
        })
        .map_err(sql)?
        .collect::<Result<_, _>>()
        .map_err(sql)?;
    Ok(Some(Manifest {
        id: unsigned(id),
        taken_at_ms,
        schema_version,
        overlay_rows: unsigned(overlay_rows),
        pinned_files,
    }))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn partial_path(path: &Path) -> PathBuf {
    sidecar(path, ".partial")
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Sync `partial`, rename it to `dest` and sync the directory, so `dest`
/// is either absent or complete after a crash.
fn publish(partial: &Path, dest: &Path) -> io::Result<()> {
    File::open(partial)?.sync_all()?;
    fs::rename(partial, dest)?;
    if let Some(parent) = dest
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        // Directory fsync is best effort: not every platform allows opening
        // a directory for sync, and the rename has already happened.
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Identity;

    fn me() -> Identity<'static> {
        Identity::new("owner", "context", "ses_backup")
    }

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let suffix = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "fleet-kv-backup-{label}-{}-{suffix}",
                std::process::id()
            ));
            fs::create_dir_all(&dir).expect("scratch dir");
            Self(dir)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_snapshot_restores_on_a_fresh_host_to_the_same_reads() {
        let source_host = Scratch::new("source");
        let fresh_host = Scratch::new("fresh");
        let store = LocalStore::open(source_host.path("local.db")).expect("open");
        store.set(&me(), "notes", "a", b"one").expect("set a");
        store.set(&me(), "notes", "a", b"two").expect("set a again");
        store.set(&me(), "notes", "b", b"bee").expect("set b");
        store.delete(&me(), "notes", "b").expect("delete b");
        store
            .set(&me(), "other", "c/d", b"\x00\xff")
            .expect("set c/d");
        // These rows are still in the WAL: the live file alone would miss them.
        let snapshot = source_host.path("snap.db");
        let taken = store.snapshot(&snapshot).expect("snapshot");
        assert_eq!(taken.overlay_rows, 3);
        assert!(taken.pinned_files.is_empty());
        assert_eq!(taken.schema_version, SCHEMA_VERSION);
        assert!(!partial_path(&snapshot).exists());

        let target = fresh_host.path("local.db");
        let restored = restore(&snapshot, &target).expect("restore");
        assert_eq!(restored, taken);
        let copy = LocalStore::open(&target).expect("open restored");
        assert_eq!(
            copy.get(&me(), "notes", "a").expect("get a"),
            Some(b"two".to_vec())
        );
        assert_eq!(copy.get(&me(), "notes", "b").expect("get b"), None);
        assert_eq!(
            copy.get(&me(), "other", "c/d").expect("get c/d"),
            Some(b"\x00\xff".to_vec())
        );
        assert_eq!(
            copy.list(&me(), "notes", true).expect("list"),
            store.list(&me(), "notes", true).expect("list source")
        );
        assert_eq!(
            copy.outbox(&me(), "notes", "a").expect("outbox"),
            store.outbox(&me(), "notes", "a").expect("outbox source")
        );
        assert_eq!(copy.latest_manifest().expect("manifest"), Some(taken));
        // History restores too: the restored host can revert to version 1.
        assert_eq!(copy.revert(&me(), "notes", "a", 1).expect("revert"), 3);
        assert_eq!(
            copy.get(&me(), "notes", "a").expect("get a"),
            Some(b"one".to_vec())
        );
    }

    #[test]
    fn snapshot_and_restore_refuse_to_overwrite() {
        let host = Scratch::new("overwrite");
        let store = LocalStore::open(host.path("local.db")).expect("open");
        store.set(&me(), "notes", "k", b"v").expect("set");
        let snapshot = host.path("snap.db");
        fs::write(&snapshot, b"keep me").expect("occupy");
        let refused = store.snapshot(&snapshot).expect_err("must refuse");
        assert_eq!(refused.code(), "backup_target_exists");
        assert_eq!(fs::read(&snapshot).expect("read"), b"keep me");
        assert!(store.latest_manifest().expect("manifest").is_none());

        fs::remove_file(&snapshot).expect("clear");
        store.snapshot(&snapshot).expect("snapshot");
        let refused = restore(&snapshot, &host.path("local.db")).expect_err("must refuse");
        assert_eq!(refused.code(), "backup_target_exists");
        let wal_only = host.path("other.db");
        fs::write(sidecar(&wal_only, "-wal"), b"stale wal").expect("occupy wal");
        let refused = restore(&snapshot, &wal_only).expect_err("must refuse a stray WAL");
        assert_eq!(refused.code(), "backup_target_exists");
        assert!(!wal_only.exists());
    }

    #[test]
    fn restore_refuses_files_that_are_not_snapshots() {
        let host = Scratch::new("refuse");
        let plain = host.path("plain.db");
        LocalStore::open(&plain)
            .expect("open")
            .set(&me(), "notes", "k", b"v")
            .expect("set");
        let refused = restore(&plain, &host.path("target.db")).expect_err("no manifest");
        assert_eq!(refused.code(), "not_a_snapshot");

        let junk = host.path("junk.db");
        fs::write(&junk, b"definitely not sqlite").expect("junk");
        let refused = restore(&junk, &host.path("target2.db")).expect_err("junk");
        assert!(
            matches!(refused.code(), "not_a_snapshot" | "snapshot_corrupt"),
            "{refused:?}"
        );
        assert!(!host.path("target.db").exists());
        assert!(!host.path("target2.db").exists());
    }

    #[test]
    fn restore_refuses_a_newer_schema() {
        let host = Scratch::new("newer");
        let store = LocalStore::open(host.path("local.db")).expect("open");
        let snapshot = host.path("snap.db");
        store.snapshot(&snapshot).expect("snapshot");
        Connection::open(&snapshot)
            .expect("open snapshot")
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .expect("bump");
        let refused = restore(&snapshot, &host.path("target.db")).expect_err("newer");
        assert_eq!(refused.code(), "unknown_schema_version");
    }

    #[test]
    fn version_two_stores_gain_the_manifest_tables_in_place() {
        let host = Scratch::new("v2");
        let path = host.path("local.db");
        LocalStore::open(&path)
            .expect("open")
            .set(&me(), "notes", "k", b"kept")
            .expect("set");
        Connection::open(&path)
            .expect("raw")
            .execute_batch(
                "DROP TABLE kv_backup_pins; DROP TABLE kv_backup_manifest;
                 PRAGMA user_version=2;",
            )
            .expect("downgrade to v2");
        let store = LocalStore::open(&path).expect("migrate");
        assert_eq!(
            store.get(&me(), "notes", "k").expect("get"),
            Some(b"kept".to_vec())
        );
        assert_eq!(store.outbox(&me(), "notes", "k").expect("outbox").len(), 1);
        let taken = store.snapshot(&host.path("snap.db")).expect("snapshot");
        assert_eq!(taken.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn each_snapshot_records_its_own_manifest() {
        let host = Scratch::new("twice");
        let store = LocalStore::open(host.path("local.db")).expect("open");
        store.set(&me(), "notes", "k", b"v").expect("set");
        let first = store.snapshot(&host.path("one.db")).expect("first");
        store.set(&me(), "notes", "j", b"w").expect("set j");
        let second = store.snapshot(&host.path("two.db")).expect("second");
        assert!(second.id > first.id);
        assert_eq!((first.overlay_rows, second.overlay_rows), (1, 2));
        assert_eq!(inspect(&host.path("one.db")).expect("inspect one"), first);
        assert_eq!(inspect(&host.path("two.db")).expect("inspect two"), second);
    }
}
