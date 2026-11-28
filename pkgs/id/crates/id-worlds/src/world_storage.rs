//! One world's on-disk storage, inspected read-only (#11776, design
//! #6308 §§2–3, 7–9).
//!
//! [`inventory`] walks one selected world directory and reports logical
//! file bytes split three ways, by the layout [`crate::world_store`]
//! writes:
//!
//! ```text
//! <dir>/journal.jsonl            journal, with its siblings
//! <dir>/journal.jsonl.*          (lock, compaction and install files)
//! <dir>/modules/<blake3>.wasm    content-addressed modules
//! everything else                other
//! ```
//!
//! The walk is a point-in-time filesystem observation, never an exact
//! figure: a file written, grown or removed while it runs is counted as
//! it was seen, and a file that disappears between listing and reading
//! is counted in [`StorageInventory::vanished`], not as an error. It
//! never follows a symlink (each one is counted and skipped), never
//! enters a nested directory that holds its own `journal.jsonl` (that is
//! another world), and stops at [`WalkLimits`], saying so in
//! [`StorageInventory::truncated`]. It opens no file for writing and
//! changes nothing.
//!
//! Filesystem access goes through [`FsRead`], so tests can stage races
//! the real filesystem cannot stage on demand.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

/// The journal file at a world directory's root.
pub const JOURNAL_FILE: &str = "journal.jsonl";
/// The content-addressed module directory at a world directory's root.
pub const MODULES_DIR: &str = "modules";

/// What one path is, as the walk sees it without following links.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    /// A regular file of this many logical bytes.
    File(u64),
    /// A directory.
    Dir,
    /// A symbolic link (never followed).
    Symlink,
    /// Anything else (socket, fifo, device): counted, not sized.
    Other,
}

/// Read-only filesystem access for the walk.
pub trait FsRead {
    /// What `path` is, without following a symlink at `path`.
    ///
    /// # Errors
    ///
    /// The underlying I/O error; `NotFound` when the path vanished.
    fn kind(&self, path: &Path) -> io::Result<EntryKind>;

    /// The entries of directory `path`, as full paths.
    ///
    /// # Errors
    ///
    /// The underlying I/O error; `NotFound` when the directory vanished.
    fn list(&self, path: &Path) -> io::Result<Vec<PathBuf>>;
}

/// The real filesystem.
#[derive(Clone, Copy, Debug, Default)]
pub struct RealFs;

impl FsRead for RealFs {
    // Sockets, fifos and devices are told apart from regular files on
    // purpose: they are counted, not sized.
    #[allow(clippy::filetype_is_file)]
    fn kind(&self, path: &Path) -> io::Result<EntryKind> {
        let meta = std::fs::symlink_metadata(path)?;
        let file_type = meta.file_type();
        Ok(if file_type.is_symlink() {
            EntryKind::Symlink
        } else if file_type.is_dir() {
            EntryKind::Dir
        } else if file_type.is_file() {
            EntryKind::File(meta.len())
        } else {
            EntryKind::Other
        })
    }

    fn list(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        std::fs::read_dir(path)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect()
    }
}

/// Bounds on one walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WalkLimits {
    /// Most entries (files, directories, links) the walk examines.
    pub max_entries: u64,
    /// Deepest directory level the walk enters below the root (the root
    /// is depth 0).
    pub max_depth: usize,
}

impl Default for WalkLimits {
    fn default() -> Self {
        Self {
            max_entries: 1_000_000,
            max_depth: 32,
        }
    }
}

/// Bytes and file count for one storage class.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ByteCount {
    /// Logical bytes (file length), not allocated blocks.
    pub bytes: u64,
    /// Regular files counted.
    pub files: u64,
}

impl ByteCount {
    const fn add(&mut self, bytes: u64) {
        self.bytes = self.bytes.saturating_add(bytes);
        self.files = self.files.saturating_add(1);
    }
}

/// One world directory's storage at one moment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StorageInventory {
    /// `journal.jsonl` and its `journal.jsonl.*` siblings at the root.
    pub journal: ByteCount,
    /// `modules/<64 hex>.wasm`.
    pub modules: ByteCount,
    /// Every other regular file under the root.
    pub other: ByteCount,
    /// Sum of the three classes.
    pub total_bytes: u64,
    /// Always false: a walk is a point-in-time observation that may race
    /// with writes (design #6308 §3).
    pub exact: bool,
    /// Symlinks seen and not followed.
    pub symlinks_skipped: u64,
    /// Nested directories holding their own journal (another world),
    /// not entered.
    pub nested_worlds_skipped: u64,
    /// Entries that were listed but gone by the time they were read.
    pub vanished: u64,
    /// Non-file, non-directory entries (sockets, fifos), not sized.
    pub special_files: u64,
    /// The walk stopped at a [`WalkLimits`] bound, so the totals cover
    /// only part of the directory.
    pub truncated: bool,
    /// When the walk began, as Unix milliseconds (UTC).
    pub as_of_unix_ms: u64,
}

/// Why a world directory could not be inventoried at all.
#[derive(Debug)]
pub enum StorageError {
    /// The world directory does not exist.
    NotFound(PathBuf),
    /// The path is not a directory.
    NotADirectory(PathBuf),
    /// The world directory itself is a symlink, which the walk never
    /// follows.
    RootIsSymlink(PathBuf),
    /// A filesystem read failed for a reason other than a vanished entry.
    Io {
        /// The path being read.
        path: PathBuf,
        /// The failure.
        source: io::Error,
    },
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(path) => write!(f, "no world directory at {}", path.display()),
            Self::NotADirectory(path) => write!(f, "{} is not a directory", path.display()),
            Self::RootIsSymlink(path) => write!(
                f,
                "{} is a symlink; the storage walk does not follow links",
                path.display()
            ),
            Self::Io { path, source } => write!(f, "read {}: {source}", path.display()),
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Which class a regular file at `relative` (to the world root) is in.
fn classify(relative: &Path) -> Class {
    let mut parts = relative.components().map(|c| c.as_os_str().to_str());
    match (parts.next(), parts.next(), parts.next()) {
        (Some(Some(name)), None, None)
            if name == JOURNAL_FILE
                || name
                    .strip_prefix(JOURNAL_FILE)
                    .is_some_and(|rest| rest.starts_with('.')) =>
        {
            Class::Journal
        }
        (Some(Some(MODULES_DIR)), Some(Some(name)), None)
            if name.strip_suffix(".wasm").is_some_and(|hash| {
                hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
            }) =>
        {
            Class::Module
        }
        _ => Class::Other,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Journal,
    Module,
    Other,
}

/// Inventory the real world directory `root`.
///
/// # Errors
///
/// See [`inventory_with`].
pub fn inventory(root: &Path, limits: WalkLimits) -> Result<StorageInventory, StorageError> {
    inventory_with(&RealFs, root, limits)
}

/// Inventory world directory `root` through `fs`.
///
/// # Errors
///
/// [`StorageError`] when the root is missing, not a directory or a
/// symlink, or a read fails for a reason other than a vanished entry.
pub fn inventory_with(
    fs: &impl FsRead,
    root: &Path,
    limits: WalkLimits,
) -> Result<StorageInventory, StorageError> {
    let as_of_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    match fs.kind(root) {
        Ok(EntryKind::Dir) => {}
        Ok(EntryKind::Symlink) => return Err(StorageError::RootIsSymlink(root.to_owned())),
        Ok(_) => return Err(StorageError::NotADirectory(root.to_owned())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(StorageError::NotFound(root.to_owned()));
        }
        Err(source) => {
            return Err(StorageError::Io {
                path: root.to_owned(),
                source,
            });
        }
    }
    let mut report = StorageInventory {
        journal: ByteCount::default(),
        modules: ByteCount::default(),
        other: ByteCount::default(),
        total_bytes: 0,
        exact: false,
        symlinks_skipped: 0,
        nested_worlds_skipped: 0,
        vanished: 0,
        special_files: 0,
        truncated: false,
        as_of_unix_ms,
    };
    let mut seen: u64 = 0;
    // Depth-first, one directory listing held per level: memory is bounded
    // by depth times the widest directory, not by the tree.
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_owned(), 0)];
    'walk: while let Some((dir, depth)) = stack.pop() {
        let mut entries = match fs.list(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound && dir != root => {
                report.vanished += 1;
                continue;
            }
            Err(source) => return Err(StorageError::Io { path: dir, source }),
        };
        // A stable order makes a truncated walk deterministic.
        entries.sort();
        for path in entries {
            if seen >= limits.max_entries {
                report.truncated = true;
                break 'walk;
            }
            seen += 1;
            let kind = match fs.kind(&path) {
                Ok(kind) => kind,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    report.vanished += 1;
                    continue;
                }
                Err(source) => return Err(StorageError::Io { path, source }),
            };
            match kind {
                EntryKind::Symlink => report.symlinks_skipped += 1,
                EntryKind::Other => report.special_files += 1,
                EntryKind::File(bytes) => {
                    let relative = path.strip_prefix(root).unwrap_or(&path);
                    match classify(relative) {
                        Class::Journal => report.journal.add(bytes),
                        Class::Module => report.modules.add(bytes),
                        Class::Other => report.other.add(bytes),
                    }
                }
                EntryKind::Dir => {
                    if depth + 1 > limits.max_depth {
                        report.truncated = true;
                        continue;
                    }
                    match fs.kind(&path.join(JOURNAL_FILE)) {
                        Ok(EntryKind::File(_) | EntryKind::Symlink) => {
                            report.nested_worlds_skipped += 1;
                        }
                        _ => stack.push((path, depth + 1)),
                    }
                }
            }
        }
    }
    report.total_bytes = report
        .journal
        .bytes
        .saturating_add(report.modules.bytes)
        .saturating_add(report.other.bytes);
    Ok(report)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::fs;

    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn write(root: &Path, rel: &str, bytes: usize) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    #[test]
    fn known_bytes_split_into_journal_modules_and_other() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "journal.jsonl", 100);
        write(root, "journal.jsonl.lock", 0);
        write(root, "journal.jsonl.tmp", 7);
        write(root, &format!("modules/{HASH}.wasm"), 40);
        write(root, &format!("modules/.{HASH}.tmp"), 3);
        write(root, "modules/not-a-hash.wasm", 5);
        write(root, "notes/journal.jsonl.bak", 11);
        write(root, "journal.jsonlx", 2);

        let report = inventory(root, WalkLimits::default()).unwrap();
        assert_eq!(
            report.journal,
            ByteCount {
                bytes: 107,
                files: 3
            }
        );
        assert_eq!(
            report.modules,
            ByteCount {
                bytes: 40,
                files: 1
            }
        );
        assert_eq!(
            report.other,
            ByteCount {
                bytes: 21,
                files: 4
            }
        );
        assert_eq!(report.total_bytes, 168);
        assert!(!report.exact);
        assert!(!report.truncated);
        assert_eq!(report.vanished, 0);
    }

    #[test]
    fn an_empty_world_is_zero_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let report = inventory(dir.path(), WalkLimits::default()).unwrap();
        assert_eq!(report.total_bytes, 0);
        assert_eq!(
            report.journal.files + report.modules.files + report.other.files,
            0
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_counted_and_never_followed() {
        let outside = tempfile::tempdir().unwrap();
        write(outside.path(), "secret.bin", 1000);
        write(outside.path(), "deep/more.bin", 1000);
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "journal.jsonl", 10);
        std::os::unix::fs::symlink(outside.path().join("secret.bin"), root.join("file-link"))
            .unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("dir-link")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join(MODULES_DIR)).unwrap();

        let report = inventory(root, WalkLimits::default()).unwrap();
        assert_eq!(report.total_bytes, 10);
        assert_eq!(report.symlinks_skipped, 3);

        let link = tempfile::tempdir().unwrap();
        let root_link = link.path().join("world");
        std::os::unix::fs::symlink(root, &root_link).unwrap();
        assert!(matches!(
            inventory(&root_link, WalkLimits::default()),
            Err(StorageError::RootIsSymlink(_))
        ));
    }

    #[test]
    fn a_nested_world_is_not_traversed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "journal.jsonl", 10);
        write(root, "forks/other/journal.jsonl", 500);
        write(root, &format!("forks/other/modules/{HASH}.wasm"), 500);
        write(root, "forks/readme.txt", 4);

        let report = inventory(root, WalkLimits::default()).unwrap();
        assert_eq!(report.total_bytes, 14);
        assert_eq!(report.nested_worlds_skipped, 1);
    }

    #[test]
    fn missing_and_non_directory_roots_are_typed() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            inventory(&dir.path().join("absent"), WalkLimits::default()),
            Err(StorageError::NotFound(_))
        ));
        write(dir.path(), "file", 1);
        assert!(matches!(
            inventory(&dir.path().join("file"), WalkLimits::default()),
            Err(StorageError::NotADirectory(_))
        ));
    }

    #[test]
    fn limits_bound_the_walk_and_say_so() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for i in 0..10 {
            write(root, &format!("f{i}"), 1);
        }
        write(root, "a/b/c/deep.bin", 9);

        let capped = inventory(
            root,
            WalkLimits {
                max_entries: 4,
                max_depth: 32,
            },
        )
        .unwrap();
        assert!(capped.truncated);
        assert!(capped.other.files <= 4);

        let shallow = inventory(
            root,
            WalkLimits {
                max_entries: 1_000,
                max_depth: 1,
            },
        )
        .unwrap();
        assert!(shallow.truncated);
        assert_eq!(shallow.other.bytes, 10);

        let full = inventory(root, WalkLimits::default()).unwrap();
        assert!(!full.truncated);
        assert_eq!(full.other.bytes, 19);
    }

    /// A filesystem whose listings name entries that are gone when read.
    struct Racy {
        kinds: BTreeMap<PathBuf, EntryKind>,
        listings: BTreeMap<PathBuf, Vec<PathBuf>>,
    }

    impl FsRead for Racy {
        fn kind(&self, path: &Path) -> io::Result<EntryKind> {
            self.kinds
                .get(path)
                .copied()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }

        fn list(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
            self.listings
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
    }

    #[test]
    fn a_file_or_directory_that_vanishes_mid_walk_is_counted_not_fatal() {
        let root = PathBuf::from("/w");
        let kinds = BTreeMap::from([
            (root.clone(), EntryKind::Dir),
            (root.join(JOURNAL_FILE), EntryKind::File(10)),
            (root.join("gone-dir"), EntryKind::Dir),
        ]);
        let listings = BTreeMap::from([(
            root.clone(),
            vec![
                root.join(JOURNAL_FILE),
                root.join("gone-file"),
                root.join("gone-dir"),
            ],
        )]);
        let report =
            inventory_with(&Racy { kinds, listings }, &root, WalkLimits::default()).unwrap();
        assert_eq!(report.total_bytes, 10);
        assert_eq!(report.vanished, 2);
        assert!(!report.exact);
    }

    #[test]
    fn other_read_failures_are_typed_with_their_path() {
        struct Denied;
        impl FsRead for Denied {
            fn kind(&self, _: &Path) -> io::Result<EntryKind> {
                Ok(EntryKind::Dir)
            }
            fn list(&self, _: &Path) -> io::Result<Vec<PathBuf>> {
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            }
        }
        let err = inventory_with(&Denied, Path::new("/w"), WalkLimits::default()).unwrap_err();
        assert!(matches!(&err, StorageError::Io { path, .. } if path == Path::new("/w")));
        assert!(err.to_string().contains("/w"));
    }

    #[test]
    fn the_report_serializes_with_stable_field_names() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "journal.jsonl", 3);
        let value =
            serde_json::to_value(inventory(dir.path(), WalkLimits::default()).unwrap()).unwrap();
        for key in [
            "journal",
            "modules",
            "other",
            "total_bytes",
            "exact",
            "symlinks_skipped",
            "nested_worlds_skipped",
            "vanished",
            "special_files",
            "truncated",
            "as_of_unix_ms",
        ] {
            assert!(value.get(key).is_some(), "{key}");
        }
        assert_eq!(value["journal"]["bytes"], 3);
        assert_eq!(value["exact"], false);
    }
}
