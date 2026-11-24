//! `id world journal check|backup` and `id world provenance`: reading a
//! world directory's journal for format audit, backup and provenance
//! (F11.12). Split out of `commands/world.rs` to keep that file under
//! the production-line limit (#1411).

use anyhow::{Context, Result, bail};

/// The outcome of checking a world directory's journal (F11.12): what
/// the `Created` header declares, what the entries require, and every
/// format problem found. The journal is valid only when `problems` is
/// empty.
#[derive(Debug)]
pub(super) struct JournalCheck {
    world_id: Option<String>,
    header_version: Option<u32>,
    entries: usize,
    required_version: u32,
    problems: Vec<String>,
}

/// Audit a world directory's journal bytes: the header version against
/// the newest this binary can read, every entry's parse, the version
/// the entries require against the header that declares less, and the
/// file ending at an entry boundary. Unlike [`read_world_journal`] —
/// an inspection read that skips a torn tail — a torn tail is a problem
/// here: `check` is the format audit, and a journal that does not end
/// at an entry boundary is not a complete record.
pub(super) fn check_journal_bytes(bytes: &[u8]) -> JournalCheck {
    use id_worlds::world::{JournalEntry, MAX_JOURNAL_VERSION, journal_version_required_by};

    let mut problems = Vec::new();
    if bytes.is_empty() {
        problems.push("the journal is empty: it has no Created header".to_owned());
        return JournalCheck {
            world_id: None,
            header_version: None,
            entries: 0,
            required_version: 0,
            problems,
        };
    }
    let torn = bytes.last() != Some(&b'\n');
    let mut lines: Vec<&[u8]> = bytes.split(|byte| *byte == b'\n').collect();
    if torn {
        lines.pop();
        problems.push(
            "the final entry is unterminated (torn tail): the journal does not end at an              entry boundary"
                .to_owned(),
        );
    }
    let mut world_id = None;
    let mut header_version = None;
    let mut required_version = 0;
    let mut entries = 0;
    for (index, line) in lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let entry: JournalEntry = match serde_json::from_slice(line) {
            Ok(entry) => entry,
            Err(err) => {
                problems.push(format!("line {} does not parse: {err}", index + 1));
                break;
            }
        };
        if entries == 0 {
            match &entry {
                JournalEntry::Created {
                    world_id: id,
                    version,
                } => {
                    world_id = Some(id.clone());
                    header_version = Some(*version);
                }
                _ => problems.push("the journal does not start with a Created header".to_owned()),
            }
        }
        entries += 1;
        required_version = required_version.max(journal_version_required_by(&entry));
    }
    if entries == 0 && problems.is_empty() {
        problems.push("the journal has no entries".to_owned());
    }
    if let Some(version) = header_version {
        if version == 0 || version > MAX_JOURNAL_VERSION {
            // The refusal wording of the restore path's header check
            // (`check_journal_version` in id-worlds), so the CLI and the
            // writer refuse a newer format in the same words.
            problems.push(format!("unsupported world journal version {version}"));
        }
        if required_version > version {
            problems.push(format!(
                "entries require journal version {required_version} but the header declares                  {version}"
            ));
        }
    }
    JournalCheck {
        world_id,
        header_version,
        entries,
        required_version,
        problems,
    }
}

/// Check the journal in `dir` (its `journal.jsonl`).
pub(super) fn check_world_journal(dir: &std::path::Path) -> Result<JournalCheck> {
    let path = dir.join("journal.jsonl");
    let bytes = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    Ok(check_journal_bytes(&bytes))
}

/// `id world journal check`: print the journal's format version and
/// validity, and exit nonzero when it is not valid — a newer-format
/// journal is flagged, never silently accepted.
pub(super) fn print_journal_check(dir: &std::path::Path, json: bool) -> Result<()> {
    let check = check_world_journal(dir)?;
    let valid = check.problems.is_empty();
    if json {
        println!(
            "{}",
            serde_json::json!({
                "world_id": check.world_id,
                "journal_version": check.header_version,
                "max_supported_version": id_worlds::world::MAX_JOURNAL_VERSION,
                "required_version": check.required_version,
                "entries": check.entries,
                "valid": valid,
                "problems": check.problems,
            })
        );
    } else {
        println!(
            "world: {}",
            check.world_id.as_deref().unwrap_or("(no Created header)")
        );
        println!(
            "journal version: {} (this binary reads up to {})",
            check
                .header_version
                .map_or_else(|| "unknown".to_owned(), |version| version.to_string()),
            id_worlds::world::MAX_JOURNAL_VERSION
        );
        println!("entries: {}", check.entries);
        println!("required version: {}", check.required_version);
        if valid {
            println!("status: valid");
        } else {
            println!("status: INVALID");
            for problem in &check.problems {
                println!("problem: {problem}");
            }
        }
    }
    if valid {
        Ok(())
    } else {
        bail!("journal check failed: {}", check.problems.join("; "))
    }
}

/// `id world journal backup`: copy the journal in `dir` to `out`, byte
/// for byte, so the copy restores the world exactly (F11.12's exit:
/// "backup restores byte for byte"). The journal must check valid
/// first — a backup of a corrupt or newer-format journal would not
/// restore here — and the output must not already exist, so a backup
/// never overwrites an earlier one by accident. The written file is
/// read back and compared before success is reported.
pub(super) fn journal_backup(dir: &std::path::Path, out: &std::path::Path) -> Result<()> {
    let path = dir.join("journal.jsonl");
    let bytes = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    let check = check_journal_bytes(&bytes);
    if !check.problems.is_empty() {
        bail!(
            "refusing to back up an invalid journal: {}",
            check.problems.join("; ")
        );
    }
    if out.exists() {
        bail!("backup output {} already exists", out.display());
    }
    write_backup(out, &bytes, &path, |path, contents| {
        std::fs::write(path, contents)
    })?;
    println!(
        "backed up {} ({} bytes, journal version {}) to {}",
        path.display(),
        bytes.len(),
        check.header_version.unwrap_or(0),
        out.display()
    );
    Ok(())
}

/// Write the backup bytes to `out` and read them back for
/// verification. Past the exists-guard in `journal_backup`, the file
/// at `out` belongs to this invocation, so any failure here — a
/// partial write, a read-back mismatch — removes it before the error
/// returns: left behind, the partial file would trip the guard on
/// every retry and strand the corrupt copy the guard exists to
/// protect against (#12786). A removal failure is appended to the
/// original error, never reported as success; a file that was never
/// created needs no removal. `write` is a seam so tests can inject
/// write failures.
fn write_backup(
    out: &std::path::Path,
    bytes: &[u8],
    journal_path: &std::path::Path,
    write: impl FnOnce(&std::path::Path, &[u8]) -> std::io::Result<()>,
) -> Result<()> {
    let attempt = (|| -> Result<()> {
        write(out, bytes).with_context(|| format!("write {}", out.display()))?;
        let written = std::fs::read(out).with_context(|| format!("read back {}", out.display()))?;
        if written != bytes {
            bail!(
                "backup verification failed: {} does not match {} byte for byte",
                out.display(),
                journal_path.display()
            );
        }
        Ok(())
    })();
    match attempt {
        Ok(()) => Ok(()),
        Err(err) => match std::fs::remove_file(out) {
            Ok(()) => Err(err),
            Err(remove_err) if remove_err.kind() == std::io::ErrorKind::NotFound => Err(err),
            Err(remove_err) => Err(err.context(format!(
                "additionally, removing the partial backup output {} failed: {remove_err}",
                out.display()
            ))),
        },
    }
}

/// Read a world directory's journal entries for inspection, without
/// opening the journal for append. A torn final fragment (a crash
/// mid-append, never acknowledged) is skipped, mirroring
/// `FileJournal::open`; any other unparseable line is corruption.
pub(super) fn read_world_journal(
    dir: &std::path::Path,
) -> Result<Vec<id_worlds::world::JournalEntry>> {
    let path = dir.join("journal.jsonl");
    let bytes = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    let mut lines: Vec<&[u8]> = bytes.split(|byte| *byte == b'\n').collect();
    if bytes.last() != Some(&b'\n') {
        // The final chunk is an unterminated fragment (or the empty tail
        // of an empty file): it was never durably acknowledged, so it is
        // dropped even when it happens to parse — the same rule
        // `FileJournal::open` and the store-level fork reader apply.
        lines.pop();
    }
    let mut entries = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let entry: id_worlds::world::JournalEntry = serde_json::from_slice(line)
            .with_context(|| format!("{} is corrupt", path.display()))?;
        entries.push(entry);
    }
    Ok(entries)
}

/// A world directory's provenance, read from its journal: the `Created`
/// header's id plus the `Forked` entry a fork appends, if any.
pub(super) struct WorldProvenance {
    pub(super) world_id: String,
    pub(super) forked: Option<(String, String, bool, u64)>,
}

pub(super) fn world_provenance(dir: &std::path::Path) -> Result<WorldProvenance> {
    let entries = read_world_journal(dir)?;
    let Some(id_worlds::world::JournalEntry::Created { world_id, .. }) = entries.first() else {
        bail!(
            "{} has no Created header",
            dir.join("journal.jsonl").display()
        );
    };
    let world_id = world_id.clone();
    let forked = entries.iter().rev().find_map(|entry| match entry {
        id_worlds::world::JournalEntry::Forked {
            from_world_id,
            from_module_hash,
            template,
            at_sequence,
        } => Some((
            from_world_id.clone(),
            from_module_hash.clone(),
            *template,
            *at_sequence,
        )),
        _ => None,
    });
    Ok(WorldProvenance { world_id, forked })
}

/// `id world provenance`: print a world directory's provenance, so a
/// template fork is identifiable without opening the world.
pub(super) fn print_world_provenance(dir: &std::path::Path, json: bool) -> Result<()> {
    let provenance = world_provenance(dir)?;
    if json {
        let (from_world_id, from_module_hash, template, at_sequence) = match &provenance.forked {
            Some((from, hash, template, at)) => {
                (Some(from.clone()), Some(hash.clone()), *template, Some(*at))
            }
            None => (None, None, false, None),
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "world_id": provenance.world_id,
                "dir": dir.display().to_string(),
                "forked_from": from_world_id,
                "from_module_hash": from_module_hash,
                "template": template,
                "at_sequence": at_sequence,
            }))?
        );
    } else {
        println!("world: {}", provenance.world_id);
        match &provenance.forked {
            Some((from, hash, template, at)) => {
                println!("forked_from: {from}");
                println!("from_module_hash: {hash}");
                println!("template: {template}");
                println!("at_sequence: {at}");
            }
            None => println!("forked_from: none"),
        }
    }
    Ok(())
}

/// Tests for `journal check` and `journal backup` (F11.12).
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A journal file's bytes for a fixture world: a `Created` header at
    /// `version`, then a `Forked` entry (a kind every version reads).
    fn journal_fixture(version: u32) -> Vec<u8> {
        use id_worlds::world::JournalEntry;
        let entries = [
            JournalEntry::Created {
                world_id: "world-1".to_owned(),
                version,
            },
            JournalEntry::Forked {
                from_world_id: "world-0".to_owned(),
                from_module_hash: "abc123".to_owned(),
                template: false,
                at_sequence: 3,
            },
        ];
        let mut bytes = Vec::new();
        for entry in &entries {
            bytes.extend_from_slice(&serde_json::to_vec(entry).unwrap());
            bytes.push(b'\n');
        }
        bytes
    }

    fn write_journal(dir: &std::path::Path, bytes: &[u8]) {
        std::fs::write(dir.join("journal.jsonl"), bytes).unwrap();
    }

    /// Oracle: F11.12's format contract — `journal check` reports the
    /// header version and validity of a well-formed journal.
    #[test]
    fn journal_check_reports_a_valid_journal() {
        let dir = tempfile::tempdir().unwrap();
        write_journal(dir.path(), &journal_fixture(1));
        let check = check_journal_bytes(&journal_fixture(1));
        assert!(check.problems.is_empty(), "{check:?}");
        assert_eq!(check.world_id.as_deref(), Some("world-1"));
        assert_eq!(check.header_version, Some(1));
        assert_eq!(check.entries, 2);
        assert!(print_journal_check(dir.path(), false).is_ok());
        assert!(print_journal_check(dir.path(), true).is_ok());
    }

    /// Oracle: the restore path's refusal discipline — a journal from a
    /// newer format is flagged with the same "unsupported world journal
    /// version" wording, and the check exits nonzero.
    #[test]
    fn journal_check_flags_a_newer_format_journal() {
        let newer = id_worlds::world::MAX_JOURNAL_VERSION + 1;
        let dir = tempfile::tempdir().unwrap();
        write_journal(dir.path(), &journal_fixture(newer));
        let check = check_journal_bytes(&journal_fixture(newer));
        assert!(
            check
                .problems
                .iter()
                .any(|problem| problem == &format!("unsupported world journal version {newer}")),
            "{check:?}"
        );
        let err = print_journal_check(dir.path(), false).unwrap_err();
        assert!(
            format!("{err}").contains("unsupported world journal version"),
            "{err}"
        );
    }

    /// Oracle: F11.12's exit — a backup restores byte for byte. The
    /// backup file's bytes equal the source journal's, and a world
    /// directory holding only the backup as its journal checks valid
    /// and reads back the same provenance.
    #[test]
    fn journal_backup_is_byte_for_byte_and_restores() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = journal_fixture(1);
        write_journal(dir.path(), &bytes);
        let out = dir.path().join("backup.jsonl");
        journal_backup(dir.path(), &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), bytes);

        let restored = tempfile::tempdir().unwrap();
        std::fs::copy(&out, restored.path().join("journal.jsonl")).unwrap();
        assert!(print_journal_check(restored.path(), false).is_ok());
        assert_eq!(
            world_provenance(restored.path()).unwrap().world_id,
            "world-1"
        );
    }

    /// A backup never overwrites an existing file, and never copies a
    /// journal that does not check valid.
    #[test]
    fn journal_backup_refuses_existing_output_and_invalid_journal() {
        let dir = tempfile::tempdir().unwrap();
        write_journal(dir.path(), &journal_fixture(1));
        let out = dir.path().join("backup.jsonl");
        std::fs::write(&out, b"earlier backup").unwrap();
        let err = journal_backup(dir.path(), &out).unwrap_err();
        assert!(format!("{err}").contains("already exists"), "{err}");

        let corrupt = tempfile::tempdir().unwrap();
        write_journal(corrupt.path(), b"not a journal\n");
        let err = journal_backup(corrupt.path(), &corrupt.path().join("b.jsonl")).unwrap_err();
        assert!(format!("{err}").contains("invalid journal"), "{err}");
    }

    /// Oracle: #12786 — a backup that fails after creating its output
    /// leaves no file behind, so the same path can be retried instead
    /// of refusing "already exists" forever. The injected writers
    /// stand in for a torn write (partial bytes, then an error) and
    /// for bytes that fail the read-back verification.
    #[test]
    fn journal_backup_removes_partial_output_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = journal_fixture(1);
        write_journal(dir.path(), &bytes);
        let journal_path = dir.path().join("journal.jsonl");

        let out = dir.path().join("partial.jsonl");
        let err = write_backup(&out, &bytes, &journal_path, |path, contents| {
            std::fs::write(path, &contents[..4])?;
            Err(std::io::Error::other("injected write failure"))
        })
        .unwrap_err();
        assert!(format!("{err}").contains("write"), "{err}");
        assert!(!out.exists(), "partial output must be removed");

        let out = dir.path().join("mismatch.jsonl");
        let err = write_backup(&out, &bytes, &journal_path, |path, _contents| {
            std::fs::write(path, b"torn write")
        })
        .unwrap_err();
        assert!(format!("{err}").contains("verification failed"), "{err}");
        assert!(!out.exists(), "unverified output must be removed");

        // The retry the guard used to refuse: a real backup to the
        // first path now succeeds.
        journal_backup(dir.path(), &dir.path().join("partial.jsonl")).unwrap();
    }
}
