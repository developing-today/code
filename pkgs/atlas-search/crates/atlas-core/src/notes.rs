//! The Atlas memory-integration zettelkasten connector
//! (developing-today/code#8626 slice 4, the design record
//! `pkgs/atlas-search/docs/memory-integration-design-8567.md`
//! section 1: the notes-vault corpus).
//!
//! The design unifies the fleet's own corpora in the existing
//! stage/registry path: one staging unit per corpus, staged units
//! carrying the design's provenance set (corpus, agent, source_ref,
//! updated_at, sensitivity, zone=fleet), and incremental re-indexing
//! on source change (the issue's second acceptance criterion,
//! demonstrated per connector family). This module is the vault-notes
//! slice of that work, on the pattern slice 1 establishes
//! ([`crate::memory`], including the slice-1b replay-safe
//! discipline):
//!
//! * [`scan_note_files`] reads a Zettelkasten/Emanote notes vault:
//!   every `*.md` file under the vault root, in sorted
//!   tree-relative order so a scan is deterministic. Non-markdown
//!   files are not corpus content and are skipped; unreadable files
//!   and invalid UTF-8 are loud errors naming the path — a partial
//!   or silently defaulted ingest is exactly what the registry's
//!   provenance discipline exists to prevent.
//! * [`stage_note_files`] stages the scanned notes into the
//!   `zettelkasten` unit through the same `items` + `chunks_raw`
//!   write path every staged unit uses: `kind` is `note`, `source`
//!   is the note's source-of-truth pointer, and `meta` carries the
//!   design's provenance set with the file's mtime as `updated_at`.
//!   Re-scanning an unchanged vault stages zero; an edited note
//!   re-stages exactly its chunks (the note's item is updated and
//!   its chunks replaced; every other item is untouched).
//!   Promotion and search are the existing aggregator and fan-out,
//!   unchanged.
//!
//! Out of scope: per-note agent attribution from note frontmatter
//! (the vault records no author reliably, so `agent` is null — the
//! design carries it "where known"); backlink-context enrichment
//! (links ride along as note text; a backlink graph is follow-up
//! work); and the slice-3 query surface, which this slice stages
//! for but does not expose.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{Error, Result};
use crate::shards::Tier;
use crate::stage::{check_stage_schema, chunk_artifact_text, open, sha256_hex};

/// The staging-unit id of the zettelkasten corpus — the corpus this
/// connector populates (design section 1: the Zettelkasten/Emanote
/// notes vault).
pub const ZETTELKASTEN_UNIT: &str = "zettelkasten";

/// One vault note, as scanned: its path relative to the vault root
/// (the stable source identity, `/`-separated), its text, its
/// modification instant as RFC 3339 UTC where known, and its
/// physical sensitivity tier (the corpus default is the corpus's
/// internal tier — individual notes carry no sensitivity marking).
#[derive(Debug, Clone, PartialEq)]
pub struct NoteFile {
    /// The note's path relative to the scanned vault root,
    /// `/`-separated (e.g. `habits/sleep.md`).
    pub rel_path: String,
    /// The note's full text, staged as-is (wikilinks and tags ride
    /// along as text).
    pub text: String,
    /// The note's modification instant as RFC 3339 UTC, where the
    /// filesystem reports one.
    pub updated_at: Option<String>,
    /// The note's physical sensitivity tier.
    pub sensitivity: Tier,
}

impl NoteFile {
    /// The note's source-of-truth pointer: the corpus scheme plus
    /// its vault-relative path, carried as the staged item's
    /// `source` and in its provenance as `source_ref`.
    pub fn source_ref(&self) -> String {
        format!("note:{}", self.rel_path)
    }
}

/// Format a filesystem modification instant as RFC 3339 UTC
/// (second precision). Times before the unix epoch report `None`
/// and let the caller fall back to the staging instant.
fn system_time_rfc3339(mtime: std::time::SystemTime) -> Option<String> {
    let secs = mtime.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
    // Days-to-civil, shared with the `atlas-stage now_utc` helper:
    // RFC 3339 records when, not sub-second detail.
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    Some(format!("{year:04}-{month:02}-{d:02}T{h:02}:{m:02}:{s:02}Z"))
}

/// Scan a notes vault root for stageable notes: every `*.md` file
/// under `root`, in sorted tree-relative order so a scan is
/// deterministic. Non-markdown files are not corpus content and are
/// skipped; unreadable files and invalid UTF-8 are loud errors
/// naming the path.
pub fn scan_note_files(root: &Path) -> Result<Vec<NoteFile>> {
    if !root.is_dir() {
        return Err(Error::fatal(format!(
            "notes connector: notes vault {} is not a directory",
            root.display()
        )));
    }
    let mut rel_paths: Vec<String> = Vec::new();
    let mut stack: Vec<std::path::PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| {
            Error::backend(format!("notes connector: listing {}: {e}", dir.display()))
        })?;
        for entry in entries {
            let entry = entry.map_err(|e| {
                Error::backend(format!("notes connector: listing {}: {e}", dir.display()))
            })?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file()
                && path
                    .extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| x.eq_ignore_ascii_case("md"))
            {
                let rel = path.strip_prefix(root).map_err(|e| {
                    Error::backend(format!(
                        "notes connector: relativizing {}: {e}",
                        path.display()
                    ))
                })?;
                rel_paths.push(
                    rel.components()
                        .map(|c| c.as_os_str().to_string_lossy())
                        .collect::<Vec<_>>()
                        .join("/"),
                );
            }
        }
    }
    rel_paths.sort();
    rel_paths
        .iter()
        .map(|rel| {
            // Rebuild the filesystem path from the `/`-joined
            // relative form (components were split on the platform
            // separator, so this round-trips on any host).
            let mut path = root.to_path_buf();
            for comp in rel.split('/') {
                path.push(comp);
            }
            let text = std::fs::read_to_string(&path)
                .map_err(|e| Error::backend(format!("notes connector: reading note {rel}: {e}")))?;
            let updated_at = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(system_time_rfc3339);
            Ok(NoteFile {
                rel_path: rel.clone(),
                text,
                updated_at,
                sensitivity: Tier::Internal,
            })
        })
        .collect()
}

/// Stage one vault note into a v1 staging DB as one `items` row plus
/// its `chunks_raw` rows, on the same write path as every staged
/// unit: `kind` is `note`, `source` is the note's source-of-truth
/// pointer, and `meta` carries the design's provenance set
/// (corpus `zettelkasten`, agent `null` — the vault records no
/// author reliably — source_ref, updated_at from the file's mtime,
/// sensitivity, zone `fleet`). Replay-safe: an unchanged note
/// re-stages to its existing item with no write; an edited note
/// updates its item and replaces exactly its chunks. Returns the
/// item's id. The DB must already be a v1 staging DB
/// ([`check_stage_schema`]); vectors are produced later, at
/// promotion, as for every staged item.
pub fn stage_note_file(
    conn: &Connection,
    unit_id: &str,
    file: &NoteFile,
    now: &str,
) -> Result<i64> {
    stage_note_file_with_status(conn, unit_id, file, now).map(|(id, _)| id)
}

fn stage_note_file_with_status(
    conn: &Connection,
    unit_id: &str,
    file: &NoteFile,
    now: &str,
) -> Result<(i64, bool)> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| Error::backend(format!("notes connector: starting stage transaction: {e}")))?;
    let result = stage_note_file_in_transaction(&tx, unit_id, file, now)?;
    tx.commit().map_err(|e| {
        Error::backend(format!(
            "notes connector: committing stage transaction: {e}"
        ))
    })?;
    Ok(result)
}

fn stage_note_file_in_transaction(
    conn: &Connection,
    unit_id: &str,
    file: &NoteFile,
    now: &str,
) -> Result<(i64, bool)> {
    check_stage_schema(conn)?;
    let chunks = chunk_artifact_text(&file.text);
    let source = file.source_ref();
    let meta = serde_json::json!({
        "corpus": ZETTELKASTEN_UNIT,
        "agent": None::<String>,
        "source_ref": source,
        "updated_at": file.updated_at,
        "sensitivity": file.sensitivity.name(),
        "zone": "fleet",
        "rel_path": file.rel_path,
        "staged_at": now,
        "chunks": chunks.len(),
    })
    .to_string();
    let existing = conn
        .query_row(
            "SELECT id, text, sensitivity, meta FROM items \
             WHERE unit_id = ?1 AND kind = 'note' AND source = ?2 ORDER BY id DESC LIMIT 1",
            params![unit_id, source],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|e| {
            Error::backend(format!(
                "notes connector: finding item for {}: {e}",
                file.rel_path
            ))
        })?;

    let (item_id, changed) = match existing {
        Some((id, text, sensitivity, old_meta)) => {
            let unchanged = text == file.text
                && sensitivity == file.sensitivity.name()
                && old_meta
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                    .is_some_and(|old| {
                        let new: serde_json::Value =
                            serde_json::from_str(&meta).unwrap_or_default();
                        old.get("updated_at") == new.get("updated_at")
                            && old.get("sensitivity") == new.get("sensitivity")
                    });
            if unchanged {
                return Ok((id, false));
            }
            conn.execute(
                "UPDATE items SET text = ?1, sensitivity = ?2, created_at = ?3, meta = ?4 WHERE id = ?5",
                params![
                    file.text,
                    file.sensitivity.name(),
                    file.updated_at.as_deref().unwrap_or(now),
                    meta,
                    id,
                ],
            )
            .map_err(|e| Error::backend(format!("notes connector: updating item for {}: {e}", file.rel_path)))?;
            conn.execute("DELETE FROM chunks_raw WHERE item_id = ?1", [id])
                .map_err(|e| {
                    Error::backend(format!(
                        "notes connector: replacing chunks for {}: {e}",
                        file.rel_path
                    ))
                })?;
            (id, true)
        }
        None => {
            conn.execute(
                "INSERT INTO items (unit_id, kind, source, text, sensitivity, created_at, meta) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    unit_id,
                    "note",
                    source,
                    file.text,
                    file.sensitivity.name(),
                    file.updated_at.as_deref().unwrap_or(now),
                    meta,
                ],
            )
            .map_err(|e| {
                Error::backend(format!(
                    "notes connector: inserting item for {}: {e}",
                    file.rel_path
                ))
            })?;
            (conn.last_insert_rowid(), true)
        }
    };
    for (idx, chunk) in chunks.iter().enumerate() {
        conn.execute(
            "INSERT INTO chunks_raw (item_id, idx, text, content_hash, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![item_id, idx as i64, chunk, sha256_hex(chunk), now],
        )
        .map_err(|e| {
            Error::backend(format!(
                "notes connector: inserting chunk {idx} for {}: {e}",
                file.rel_path
            ))
        })?;
    }
    Ok((item_id, changed))
}

/// Stage scanned vault notes into a unit, in scan order. Returns the
/// ids of the notes that staged new or changed content — an
/// unchanged re-scan returns an empty vec, never duplicate items.
pub fn stage_note_files(
    conn: &Connection,
    unit_id: &str,
    files: &[NoteFile],
    now: &str,
) -> Result<Vec<i64>> {
    files
        .iter()
        .map(|file| stage_note_file_with_status(conn, unit_id, file, now))
        .filter_map(|result| match result {
            Ok((id, true)) => Some(Ok(id)),
            Ok((_, false)) => None,
            Err(error) => Some(Err(error)),
        })
        .collect()
}

/// [`scan_note_files`] + [`stage_note_files`] against a staging DB
/// file: the form the `atlas-stage ingest-notes` tool calls, so the
/// CLI needs no DB handle of its own.
pub fn scan_and_stage_note_files_in_db(
    db: &Path,
    unit_id: &str,
    vault: &Path,
    now: &str,
) -> Result<Vec<i64>> {
    let files = scan_note_files(vault)?;
    let conn = open(db)?;
    stage_note_files(&conn, unit_id, &files, now)
}
