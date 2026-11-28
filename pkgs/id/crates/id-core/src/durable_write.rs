//! Crash-safe replacement of a small state file: the bytes reach disk before the
//! rename makes them visible at `path`, so a power loss never leaves an empty or
//! torn file at the real path.
use std::fs::File;
use std::io::Write as _;
use std::path::Path;
use std::{error::Error, fmt, io};

/// Which stage failed while replacing a durable state file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteStage {
    /// The old target is still in place (or no target was installed).
    BeforeRename,
    /// The rename installed the new target, but syncing its directory failed.
    AfterRename,
}

/// An I/O failure while replacing a durable state file.
#[derive(Debug)]
pub struct DurableWriteError {
    stage: WriteStage,
    source: io::Error,
}

impl DurableWriteError {
    /// The failure stage, allowing callers to reconcile post-rename state.
    #[must_use]
    pub fn stage(&self) -> WriteStage {
        self.stage
    }
}

impl fmt::Display for DurableWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "durable write failed {:?}: {}", self.stage, self.source)
    }
}

impl Error for DurableWriteError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

/// Writes `bytes` to `tmp`, syncs it, renames it over `path`, then syncs the parent directory.
pub fn write_durable(path: &Path, tmp: &Path, bytes: &[u8]) -> Result<(), DurableWriteError> {
    write_durable_with_parent_sync(path, tmp, bytes, |dir| File::open(dir)?.sync_all())
}

/// [`write_durable`] with the parent-directory sync step supplied by
/// the caller, so a caller can observe — and a test can inject — the
/// post-rename sync failure that [`WriteStage::AfterRename`] reports.
pub fn write_durable_with_parent_sync(
    path: &Path,
    tmp: &Path,
    bytes: &[u8],
    sync_parent: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<(), DurableWriteError> {
    {
        let mut file = File::create(tmp).map_err(before_rename)?;
        file.write_all(bytes).map_err(before_rename)?;
        file.sync_all().map_err(before_rename)?;
    }
    std::fs::rename(tmp, path).map_err(before_rename)?;
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    sync_parent(dir).map_err(|source| DurableWriteError {
        stage: WriteStage::AfterRename,
        source,
    })
}

fn before_rename(source: io::Error) -> DurableWriteError {
    DurableWriteError {
        stage: WriteStage::BeforeRename,
        source,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{WriteStage, write_durable, write_durable_with_parent_sync};

    #[test]
    fn replaces_file_and_leaves_no_temp() {
        let dir = std::env::temp_dir().join(format!("durable-write-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create dir");
        let path = dir.join("state.json");
        let tmp = dir.join("state.json.tmp");

        write_durable(&path, &tmp, b"first").expect("first write");
        write_durable(&path, &tmp, b"second").expect("second write");

        assert_eq!(std::fs::read(&path).expect("read"), b"second");
        assert!(!tmp.exists());
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn parent_sync_failure_is_reported_after_installing_new_file() {
        let dir = std::env::temp_dir().join(format!("durable-write-sync-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create dir");
        let path = dir.join("state.json");
        let tmp = dir.join("state.json.tmp");

        let error = write_durable_with_parent_sync(&path, &tmp, b"new", |_| {
            Err(std::io::Error::other("injected parent sync failure"))
        })
        .expect_err("parent sync failure is not success");

        assert_eq!(error.stage(), WriteStage::AfterRename);
        assert_eq!(std::fs::read(&path).expect("read installed file"), b"new");
        assert!(!tmp.exists());
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn rename_failure_is_reported_before_installing_new_file() {
        let dir = std::env::temp_dir().join(format!("durable-write-rename-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create dir");
        let path = dir.join("missing").join("state.json");
        let tmp = dir.join("state.json.tmp");

        let error = write_durable(&path, &tmp, b"new").expect_err("rename failure");

        assert_eq!(error.stage(), WriteStage::BeforeRename);
        assert!(!path.exists());
        std::fs::remove_file(tmp).expect("remove temp");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
