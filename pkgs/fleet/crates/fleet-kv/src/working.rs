//! Streamed working files for values over the resident budget (#13314).
//!
//! A value larger than [`INLINE_LIMIT`] is never held in memory whole and
//! never rejected for size. It streams into a temporary file beside the
//! store, is hashed on the way (unkeyed BLAKE3-256, the #9120 content-id
//! profile), synced, and renamed to its content id. Only then does the
//! store commit a row that refers to it, so a crash leaves either no row
//! (and at worst an unreferenced file) or a row whose file is complete.
//! Files are content-addressed and never rewritten: two versions with the
//! same bytes share one file, and `revert` copies the reference.
//!
//! Reads stream the file back and check its length and hash; a missing or
//! altered file is a typed error, never a silent short read.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// Values up to this many bytes are stored inline in the database row;
/// larger ones become working files. This is the 1 MiB resident budget.
pub const INLINE_LIMIT: usize = 1 << 20;

const CHUNK: usize = 64 * 1024;

/// A committed row's reference to a working file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkingRef {
    /// Lower-case hex BLAKE3-256 of the bytes; also the file name.
    pub id: String,
    pub len: u64,
}

impl WorkingRef {
    /// The content id in its tagged form, as the backup manifest records it.
    pub fn hash(&self) -> String {
        format!("blake3-256:{}", self.id)
    }
}

/// Why a working file could not be read or copied.
#[derive(Debug)]
pub enum WorkingError {
    /// The file a row refers to is not there.
    Missing(String),
    /// The file's length or hash does not match the row.
    Corrupt(String),
    Io(io::Error),
}

impl From<io::Error> for WorkingError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Stream `prefix` followed by the rest of `reader` into a working file in
/// `dir` and return its reference. The file appears under its content id
/// only once it is complete and synced.
pub(crate) fn stage(dir: &Path, prefix: &[u8], reader: &mut dyn Read) -> io::Result<WorkingRef> {
    fs::create_dir_all(dir)?;
    let temporary = dir.join(format!(
        "tmp-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos())
    ));
    let result = (|| -> io::Result<WorkingRef> {
        let mut file = File::create(&temporary)?;
        let mut hasher = blake3::Hasher::new();
        file.write_all(prefix)?;
        hasher.update(prefix);
        let mut len = prefix.len() as u64;
        let mut chunk = vec![0; CHUNK];
        loop {
            let read = match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            file.write_all(&chunk[..read])?;
            hasher.update(&chunk[..read]);
            len += read as u64;
        }
        file.sync_all()?;
        let reference = WorkingRef {
            id: hasher.finalize().to_hex().to_string(),
            len,
        };
        let path = dir.join(&reference.id);
        if path.exists() {
            // Same content id, same bytes: keep the file already there.
            fs::remove_file(&temporary)?;
        } else {
            fs::rename(&temporary, &path)?;
            sync_dir(dir);
        }
        Ok(reference)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Stream a working file to `out`, checking its length first and its hash
/// at the end. On a hash mismatch the bytes have been written; the caller
/// must treat the read as failed.
pub(crate) fn stream(
    dir: &Path,
    reference: &WorkingRef,
    out: &mut dyn Write,
) -> Result<(), WorkingError> {
    let mut file = open_checked(dir, reference)?;
    let mut hasher = blake3::Hasher::new();
    let mut chunk = vec![0; CHUNK];
    loop {
        let read = match file.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        hasher.update(&chunk[..read]);
        out.write_all(&chunk[..read])?;
    }
    if hasher.finalize().to_hex().as_str() == reference.id {
        Ok(())
    } else {
        Err(WorkingError::Corrupt(reference.id.clone()))
    }
}

/// Copy a working file from `from` to `to` (both directories), verifying
/// length and hash before the copy is published. An identical file already
/// at the destination is kept.
pub(crate) fn copy_verified(
    from: &Path,
    to: &Path,
    reference: &WorkingRef,
) -> Result<(), WorkingError> {
    fs::create_dir_all(to)?;
    let destination = to.join(&reference.id);
    if destination.exists() {
        // Content-addressed: a file with this name must hold these bytes.
        // Verify it rather than trust the name.
        return verify(to, reference);
    }
    let partial = to.join(format!("{}.partial", reference.id));
    let result = (|| -> Result<(), WorkingError> {
        let mut target = File::create(&partial)?;
        stream(from, reference, &mut target)?;
        target.sync_all()?;
        fs::rename(&partial, &destination)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result
}

/// Check that a working file exists with the right length and hash.
pub(crate) fn verify(dir: &Path, reference: &WorkingRef) -> Result<(), WorkingError> {
    stream(dir, reference, &mut io::sink())
}

fn open_checked(dir: &Path, reference: &WorkingRef) -> Result<File, WorkingError> {
    // Only a 64-hex-digit id is ever a file name here, so a crafted row can
    // never point a read outside the working directory.
    if reference.id.len() != 64
        || !reference
            .id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(WorkingError::Corrupt(reference.id.clone()));
    }
    let path: PathBuf = dir.join(&reference.id);
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(WorkingError::Missing(reference.id.clone()));
        }
        Err(error) => return Err(error.into()),
    };
    if file.metadata()?.len() != reference.len {
        return Err(WorkingError::Corrupt(reference.id.clone()));
    }
    Ok(file)
}

fn sync_dir(dir: &Path) {
    // Best effort: not every platform can sync a directory handle.
    if let Ok(directory) = File::open(dir) {
        let _ = directory.sync_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Identity, KvError, LocalStore, backup};

    fn me() -> Identity<'static> {
        Identity::new("owner", "context", "ses_working")
    }

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let suffix = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "fleet-kv-working-{label}-{}-{suffix}",
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

    /// Deterministic bytes that differ along their whole length.
    fn bytes(len: usize, seed: u8) -> Vec<u8> {
        (0..len)
            .map(|index| (index as u8).wrapping_mul(31).wrapping_add(seed))
            .collect()
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = match fs::read_dir(dir) {
            Ok(entries) => entries
                .map(|entry| {
                    entry
                        .expect("entry")
                        .file_name()
                        .into_string()
                        .expect("name")
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        names.sort();
        names
    }

    /// A reader that hands out at most 4 KiB per call, like a pipe.
    struct Trickle<'a> {
        data: &'a [u8],
    }

    impl Read for Trickle<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let take = buf.len().min(4096).min(self.data.len());
            buf[..take].copy_from_slice(&self.data[..take]);
            self.data = &self.data[take..];
            Ok(take)
        }
    }

    #[test]
    fn values_over_the_budget_stream_to_a_content_addressed_file() {
        let host = Scratch::new("stream");
        let db = host.path("local.db");
        let store = LocalStore::open(&db).expect("open");
        let big = bytes(3 * INLINE_LIMIT + 17, 7);
        let version = store
            .set_reader(&me(), "notes", "big", &mut Trickle { data: &big })
            .expect("set big");
        assert_eq!(version, 1);
        let dir = crate::working_dir_for(&db);
        let id = blake3::hash(&big).to_hex().to_string();
        assert_eq!(files_in(&dir), [id.clone()]);
        let mut out = Vec::new();
        assert!(store.get_to(&me(), "notes", "big", &mut out).expect("get"));
        assert_eq!(out, big);
        let inline_len: i64 = store
            .connection
            .query_row(
                "SELECT length(value) FROM kv_local WHERE key='big'",
                [],
                |row| row.get(0),
            )
            .expect("row");
        assert_eq!(inline_len, 0, "the row holds a reference, not the bytes");

        // The same bytes again share the file; revert shares it too.
        store
            .set(&me(), "notes", "big", b"small")
            .expect("set small");
        store.set(&me(), "notes", "twin", &big).expect("set twin");
        assert_eq!(store.revert(&me(), "notes", "big", 1).expect("revert"), 3);
        assert_eq!(files_in(&dir), [id]);
        assert_eq!(
            store.get(&me(), "notes", "big").expect("get"),
            Some(big.clone())
        );
        assert_eq!(store.get(&me(), "notes", "twin").expect("get"), Some(big));
        drop(store);
        let reopened = LocalStore::open(&db).expect("reopen");
        assert!(reopened.get(&me(), "notes", "twin").expect("get").is_some());
    }

    #[test]
    fn the_budget_boundary_is_inclusive() {
        let host = Scratch::new("boundary");
        let db = host.path("local.db");
        let store = LocalStore::open(&db).expect("open");
        let at = bytes(INLINE_LIMIT, 1);
        store.set(&me(), "notes", "at", &at).expect("set at limit");
        assert!(files_in(&crate::working_dir_for(&db)).is_empty());
        let over = bytes(INLINE_LIMIT + 1, 2);
        store
            .set(&me(), "notes", "over", &over)
            .expect("set over limit");
        assert_eq!(files_in(&crate::working_dir_for(&db)).len(), 1);
        assert_eq!(store.get(&me(), "notes", "at").expect("get"), Some(at));
        assert_eq!(store.get(&me(), "notes", "over").expect("get"), Some(over));
    }

    #[test]
    fn an_in_memory_store_keeps_large_values_inline() {
        let store = LocalStore::in_memory().expect("store");
        let big = bytes(INLINE_LIMIT + 5, 3);
        store
            .set_reader(&me(), "notes", "k", &mut &big[..])
            .expect("set");
        assert_eq!(store.get(&me(), "notes", "k").expect("get"), Some(big));
    }

    #[test]
    fn a_missing_or_altered_working_file_is_a_typed_error() {
        let host = Scratch::new("damage");
        let db = host.path("local.db");
        let store = LocalStore::open(&db).expect("open");
        let big = bytes(INLINE_LIMIT + 100, 9);
        store.set(&me(), "notes", "k", &big).expect("set");
        let file = crate::working_dir_for(&db).join(blake3::hash(&big).to_hex().as_str());

        let mut altered = big.clone();
        altered[INLINE_LIMIT] ^= 0xff;
        fs::write(&file, &altered).expect("alter");
        assert!(matches!(
            store.get(&me(), "notes", "k"),
            Err(KvError::WorkingFileCorrupt(_))
        ));

        fs::write(&file, &big[..10]).expect("truncate");
        let mut out = Vec::new();
        assert!(matches!(
            store.get_to(&me(), "notes", "k", &mut out),
            Err(KvError::WorkingFileCorrupt(_))
        ));
        assert!(out.is_empty(), "a length mismatch writes nothing");

        fs::remove_file(&file).expect("remove");
        let error = store.get(&me(), "notes", "k").expect_err("missing");
        assert_eq!(error.code(), "working_file_missing");
    }

    #[test]
    fn snapshots_carry_and_restores_relink_pinned_working_files() {
        let source_host = Scratch::new("snap-source");
        let fresh_host = Scratch::new("snap-fresh");
        let db = source_host.path("local.db");
        let store = LocalStore::open(&db).expect("open");
        let first = bytes(INLINE_LIMIT + 1, 4);
        let second = bytes(2 * INLINE_LIMIT, 5);
        store.set(&me(), "notes", "k", &first).expect("set first");
        store.set(&me(), "notes", "k", &second).expect("set second");
        store.set(&me(), "notes", "small", b"s").expect("set small");

        let snapshot = source_host.path("snap.db");
        let taken = store.snapshot(&snapshot).expect("snapshot");
        assert_eq!(taken.pinned_files.len(), 2, "history is pinned too");
        for pin in &taken.pinned_files {
            assert_eq!(pin.hash, format!("blake3-256:{}", pin.content_id));
        }
        assert_eq!(
            files_in(&backup::files_dir(&snapshot)),
            files_in(&crate::working_dir_for(&db))
        );
        assert_eq!(
            store.latest_manifest().expect("manifest"),
            Some(taken.clone())
        );

        let target = fresh_host.path("local.db");
        assert_eq!(backup::restore(&snapshot, &target).expect("restore"), taken);
        let copy = LocalStore::open(&target).expect("open restored");
        assert_eq!(copy.get(&me(), "notes", "k").expect("get"), Some(second));
        assert_eq!(copy.revert(&me(), "notes", "k", 1).expect("revert"), 3);
        assert_eq!(copy.get(&me(), "notes", "k").expect("get"), Some(first));
    }

    #[test]
    fn a_restore_with_a_missing_pinned_file_writes_no_store() {
        let host = Scratch::new("incomplete");
        let store = LocalStore::open(host.path("local.db")).expect("open");
        store
            .set(&me(), "notes", "k", &bytes(INLINE_LIMIT + 1, 6))
            .expect("set");
        let snapshot = host.path("snap.db");
        store.snapshot(&snapshot).expect("snapshot");
        fs::remove_dir_all(backup::files_dir(&snapshot)).expect("lose the files");
        let target = host.path("fresh.db");
        let refused = backup::restore(&snapshot, &target).expect_err("incomplete");
        assert_eq!(refused.code(), "snapshot_incomplete");
        assert!(!target.exists());

        let again = store.snapshot(&snapshot).expect_err("dest exists");
        assert_eq!(again.code(), "backup_target_exists");
        let other = host.path("other.db");
        fs::create_dir_all(backup::files_dir(&other)).expect("stray files dir");
        let refused = store.snapshot(&other).expect_err("files dir exists");
        assert_eq!(refused.code(), "backup_target_exists");
    }
}
