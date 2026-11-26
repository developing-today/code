//! A world's journal on disk, for the host posture (design §3, second
//! posture; #13706).
//!
//! The file is JSON lines: a header line, then one [`PositionBatch`] per
//! line, positions 1, 2, 3 and so on with no gaps. An append writes the
//! whole line, newline included, and syncs it to disk before it returns,
//! so a position the store has reported is durable.
//!
//! Opening a store checks every line. A last line without its newline is
//! an append that never finished: nobody was told about it, so it is
//! dropped and the file cut back to the last whole position. Anything
//! else that is wrong (a bad header, a line that does not parse, a gap, a
//! position out of order, a delta stamped with another position) refuses
//! the open. Positions are never skipped, renumbered or reordered.

use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use id_bevy_bridge::{JournalDelta, PositionBatch};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The journal file format this store reads and writes.
pub const JOURNAL_FORMAT: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct Header {
    id_bevy_journal: u32,
}

/// Why a journal could not be created, opened or appended to.
#[derive(Debug, Error)]
pub enum StoreError {
    /// The file system failed.
    #[error("journal {path}: {source}")]
    Io {
        /// The journal file.
        path: PathBuf,
        /// The failure.
        source: io::Error,
    },
    /// The file is not a journal this store can read.
    #[error("journal {path}, line {line}: {reason}")]
    Corrupt {
        /// The journal file.
        path: PathBuf,
        /// The 1-based line that is wrong.
        line: usize,
        /// What is wrong with it.
        reason: String,
    },
    /// An earlier append failed and could not be undone, so the file may
    /// end in a partial line. Reopen the store to recover it.
    #[error("journal {0}: an earlier append failed; reopen the journal")]
    Broken(PathBuf),
}

/// A world's journal on disk.
#[derive(Debug)]
pub struct JournalStore {
    path: PathBuf,
    file: File,
    /// Bytes of whole lines in the file.
    len: u64,
    batches: Vec<PositionBatch>,
    broken: bool,
    /// Bytes a torn last line held when the store was opened.
    recovered: u64,
}

impl JournalStore {
    /// Create a new, empty journal at `path`. Fails if the file exists.
    ///
    /// # Errors
    ///
    /// The file exists or could not be written.
    pub fn create(path: &Path) -> Result<Self, StoreError> {
        let io_error = |source| StoreError::Io {
            path: path.to_owned(),
            source,
        };
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .create_new(true)
            .open(path)
            .map_err(io_error)?;
        let mut line = serde_json::to_string(&Header {
            id_bevy_journal: JOURNAL_FORMAT,
        })
        .map_err(|error| io_error(io::Error::other(error)))?;
        line.push('\n');
        file.write_all(line.as_bytes()).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        sync_parent(path).map_err(io_error)?;
        Ok(Self {
            path: path.to_owned(),
            file,
            len: u64::try_from(line.len()).unwrap_or(u64::MAX),
            batches: Vec::new(),
            broken: false,
            recovered: 0,
        })
    }

    /// Open the journal at `path` and check every position in it. A torn
    /// last line is cut off; see the module docs.
    ///
    /// # Errors
    ///
    /// The file could not be read, or it is not a whole, ordered journal.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let io_error = |source| StoreError::Io {
            path: path.to_owned(),
            source,
        };
        let corrupt = |line: usize, reason: String| StoreError::Corrupt {
            path: path.to_owned(),
            line,
            reason,
        };
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .open(path)
            .map_err(io_error)?;
        let total = file.metadata().map_err(io_error)?.len();
        let mut reader = BufReader::new(&mut file);
        let mut batches: Vec<PositionBatch> = Vec::new();
        let mut len: u64 = 0;
        let mut number = 0;
        let mut text = String::new();
        loop {
            text.clear();
            let read = reader.read_line(&mut text).map_err(io_error)?;
            if read == 0 {
                break;
            }
            number += 1;
            if !text.ends_with('\n') {
                // An append that never finished. Nothing was told of it.
                break;
            }
            if number == 1 {
                let header: Header = serde_json::from_str(&text)
                    .map_err(|error| corrupt(1, format!("not a journal header: {error}")))?;
                if header.id_bevy_journal != JOURNAL_FORMAT {
                    return Err(corrupt(
                        1,
                        format!("journal format {} is not supported", header.id_bevy_journal),
                    ));
                }
            } else {
                let batch: PositionBatch = serde_json::from_str(&text)
                    .map_err(|error| corrupt(number, format!("not a position: {error}")))?;
                let expected = batches.last().map_or(1, |last| last.position + 1);
                check_batch(&batch, expected).map_err(|reason| corrupt(number, reason))?;
                batches.push(batch);
            }
            len += u64::try_from(read).unwrap_or(u64::MAX);
        }
        drop(reader);
        if number == 0 {
            return Err(corrupt(1, "the file is empty".to_owned()));
        }
        if len == 0 {
            return Err(corrupt(1, "the header was never finished".to_owned()));
        }
        let recovered = total - len;
        if recovered > 0 {
            file.set_len(len).map_err(io_error)?;
            file.sync_all().map_err(io_error)?;
        }
        Ok(Self {
            path: path.to_owned(),
            file,
            len,
            batches,
            broken: false,
            recovered,
        })
    }

    /// Open the journal at `path`, or create it if there is none.
    ///
    /// # Errors
    ///
    /// As [`Self::open`] and [`Self::create`].
    pub fn open_or_create(path: &Path) -> Result<Self, StoreError> {
        if path.exists() {
            Self::open(path)
        } else {
            Self::create(path)
        }
    }

    /// The journal file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Every position, in order.
    #[must_use]
    pub fn batches(&self) -> &[PositionBatch] {
        &self.batches
    }

    /// The last position, if any.
    #[must_use]
    pub fn head(&self) -> Option<u64> {
        self.batches.last().map(|batch| batch.position)
    }

    /// Bytes of a torn last line that [`Self::open`] cut off; zero if
    /// the file ended on a whole position.
    #[must_use]
    pub const fn recovered_bytes(&self) -> u64 {
        self.recovered
    }

    /// Journal `deltas` as the next position and return it. Each delta is
    /// stamped with that position. The position is on disk before this
    /// returns.
    ///
    /// # Errors
    ///
    /// The write or sync failed. The file is cut back to the last whole
    /// position; if even that fails, the store refuses further appends.
    pub fn append(&mut self, deltas: Vec<JournalDelta>) -> Result<u64, StoreError> {
        if self.broken {
            return Err(StoreError::Broken(self.path.clone()));
        }
        let position = self.head().map_or(1, |head| head + 1);
        let batch = PositionBatch {
            position,
            deltas: deltas
                .into_iter()
                .map(|delta| restamp(delta, position))
                .collect(),
        };
        let io_error = |source| StoreError::Io {
            path: self.path.clone(),
            source,
        };
        let mut line =
            serde_json::to_string(&batch).map_err(|error| io_error(io::Error::other(error)))?;
        line.push('\n');
        let written = self
            .file
            .write_all(line.as_bytes())
            .and_then(|()| self.file.sync_data());
        if let Err(source) = written {
            let undone = self
                .file
                .set_len(self.len)
                .and_then(|()| self.file.seek(SeekFrom::End(0)).map(drop))
                .and_then(|()| self.file.sync_all());
            if undone.is_err() {
                self.broken = true;
            }
            return Err(io_error(source));
        }
        self.len += u64::try_from(line.len()).unwrap_or(u64::MAX);
        self.batches.push(batch);
        Ok(position)
    }

    /// Sync and close the journal.
    ///
    /// # Errors
    ///
    /// The final sync failed.
    pub fn close(self) -> Result<(), StoreError> {
        self.file.sync_all().map_err(|source| StoreError::Io {
            path: self.path.clone(),
            source,
        })
    }
}

const fn restamp(mut delta: JournalDelta, position: u64) -> JournalDelta {
    // Rewrite the position in place: destructuring by value would drop the
    // ignored fields, which a `const fn` cannot do.
    match &mut delta {
        JournalDelta::Upsert { position: at, .. } | JournalDelta::Retract { position: at, .. } => {
            *at = position;
        }
    }
    delta
}

fn check_batch(batch: &PositionBatch, expected: u64) -> Result<(), String> {
    if batch.position != expected {
        return Err(format!(
            "position {} where {expected} should be",
            batch.position
        ));
    }
    if let Some(stray) = batch
        .deltas
        .iter()
        .find(|delta| delta.position() != batch.position)
    {
        return Err(format!(
            "a delta stamped {} inside position {}",
            stray.position(),
            batch.position
        ));
    }
    Ok(())
}

/// Sync the directory holding `path`, so a new file's name is durable.
fn sync_parent(path: &Path) -> io::Result<()> {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    #[cfg(unix)]
    {
        File::open(parent)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = parent;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("id-bevy-journal-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("world.journal")
    }

    fn upsert(id: &str, x: f64) -> JournalDelta {
        JournalDelta::Upsert {
            position: 0,
            id: id.to_owned(),
            values: vec![("position".to_owned(), serde_json::json!([x, 0.0]))],
        }
    }

    #[test]
    fn create_refuses_an_existing_file() {
        let path = temp_path("exists");
        JournalStore::create(&path).unwrap().close().unwrap();
        assert!(matches!(
            JournalStore::create(&path),
            Err(StoreError::Io { .. })
        ));
    }

    #[test]
    fn reopening_resumes_from_the_last_position_without_loss_or_reordering() {
        let path = temp_path("resume");
        let mut store = JournalStore::create(&path).unwrap();
        for x in 0..5 {
            store.append(vec![upsert("a", f64::from(x))]).unwrap();
        }
        let before = store.batches().to_vec();
        store.close().unwrap();

        let mut store = JournalStore::open(&path).unwrap();
        assert_eq!(store.batches(), before.as_slice());
        assert_eq!(store.head(), Some(5));
        assert_eq!(store.recovered_bytes(), 0);
        assert_eq!(store.append(vec![upsert("a", 9.0)]).unwrap(), 6);
        store.close().unwrap();

        let store = JournalStore::open(&path).unwrap();
        let positions: Vec<u64> = store.batches().iter().map(|b| b.position).collect();
        assert_eq!(positions, vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(&store.batches()[..5], before.as_slice());
    }

    #[test]
    fn a_torn_last_line_is_cut_off_and_the_next_append_takes_its_place() {
        let path = temp_path("torn");
        let mut store = JournalStore::create(&path).unwrap();
        store.append(vec![upsert("a", 1.0)]).unwrap();
        store.append(vec![upsert("a", 2.0)]).unwrap();
        store.close().unwrap();
        let whole = std::fs::metadata(&path).unwrap().len();
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(br#"{"position":3,"deltas":[{"op":"ups"#)
            .unwrap();
        drop(file);

        let mut store = JournalStore::open(&path).unwrap();
        assert_eq!(store.head(), Some(2));
        assert!(store.recovered_bytes() > 0);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), whole);
        assert_eq!(store.append(vec![upsert("a", 3.0)]).unwrap(), 3);
        store.close().unwrap();
        assert_eq!(JournalStore::open(&path).unwrap().head(), Some(3));
    }

    fn write_lines(path: &Path, lines: &[&str]) {
        let mut text = String::new();
        for line in lines {
            text.push_str(line);
            text.push('\n');
        }
        std::fs::write(path, text).unwrap();
    }

    const HEADER: &str = r#"{"id_bevy_journal":1}"#;

    #[test]
    fn a_gap_or_a_position_out_of_order_refuses_the_open() {
        let path = temp_path("gap");
        write_lines(
            &path,
            &[
                HEADER,
                r#"{"position":1,"deltas":[]}"#,
                r#"{"position":3,"deltas":[]}"#,
            ],
        );
        let Err(StoreError::Corrupt { line, .. }) = JournalStore::open(&path) else {
            panic!("a gap was accepted");
        };
        assert_eq!(line, 3);

        write_lines(
            &path,
            &[
                HEADER,
                r#"{"position":2,"deltas":[]}"#,
                r#"{"position":1,"deltas":[]}"#,
            ],
        );
        assert!(matches!(
            JournalStore::open(&path),
            Err(StoreError::Corrupt { line: 2, .. })
        ));
    }

    #[test]
    fn a_misstamped_delta_or_a_bad_line_refuses_the_open() {
        let path = temp_path("bad");
        write_lines(
            &path,
            &[
                HEADER,
                r#"{"position":1,"deltas":[{"op":"retract","position":2,"id":"a"}]}"#,
            ],
        );
        assert!(matches!(
            JournalStore::open(&path),
            Err(StoreError::Corrupt { line: 2, .. })
        ));

        write_lines(&path, &[HEADER, "not json"]);
        assert!(matches!(
            JournalStore::open(&path),
            Err(StoreError::Corrupt { line: 2, .. })
        ));

        write_lines(&path, &[r#"{"id_bevy_journal":2}"#]);
        assert!(matches!(
            JournalStore::open(&path),
            Err(StoreError::Corrupt { line: 1, .. })
        ));

        std::fs::write(&path, "").unwrap();
        assert!(matches!(
            JournalStore::open(&path),
            Err(StoreError::Corrupt { line: 1, .. })
        ));
    }
}
