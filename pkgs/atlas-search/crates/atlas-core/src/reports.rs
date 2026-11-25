//! The Atlas research-reports connector (developing-today/code#8626,
//! slice 4 of the #8567 design record,
//! `pkgs/atlas-search/docs/memory-integration-design-8567.md`
//! sections 1 and 5).
//!
//! Slice 4 builds the remaining corpus connectors on the pattern
//! slice 1 established in [`crate::memory`]. This module is the
//! research-reports family: prior fleet research outputs — goal
//! files and briefs, held as Markdown documents in a tree the caller
//! names. It is the fifth and last of the design's five corpora to
//! gain a connector (agent memory in slice 1; fleet memory files,
//! zettelkasten notes and repo docs in the sibling slice-4 PRs).
//!
//! * [`collect_research_reports`] walks a report tree recursively
//!   and reads every `.md` document in it, in relative-path order.
//!   Each document's `updated_at` provenance is its file mtime; a
//!   document that cannot be read or is not UTF-8 is a loud error
//!   naming the path — never a silent skip, which would leave the
//!   corpus quietly incomplete.
//! * [`stage_research_report`] / [`stage_research_reports`] stage
//!   documents into the `research-reports` unit (already registered
//!   by [`crate::memory::register_memory_corpora`]) through the same
//!   `items` + `chunks_raw` write path every staged unit uses, with
//!   the design's full provenance set (corpus, agent, source_ref,
//!   updated_at, sensitivity, zone — `fleet`, per section 5's
//!   pre-ratification scope: fleet/own corpora only). The
//!   originating agent of a report is not recorded in the document,
//!   so `agent` is null — the design carries it "where known".
//! * Staging is replay-safe, as in slice 1: re-staging an unchanged
//!   document is a no-op that writes nothing, and editing one
//!   document re-stages exactly that document's chunks — the
//!   per-family incremental proof #8626's acceptance asks for.
//!
//! Promotion and search are the existing aggregator and fan-out,
//! unchanged; queryability through the slice-3 MCP surface (#8625)
//! lands with that slice, as for every other corpus.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{Error, Result};
use crate::shards::Tier;
use crate::stage::{check_stage_schema, chunk_artifact_text, open, sha256_hex};
use crate::trace::rfc3339_utc;

/// The staging-unit id of the research-reports corpus — the corpus
/// this connector populates (one of the five units
/// [`crate::memory::MEMORY_CORPORA`] registers).
pub const RESEARCH_REPORTS_UNIT: &str = "research-reports";

/// One research report document, as staged: its path relative to
/// the report tree it was collected from, its full text, and its
/// file mtime as the provenance `updated_at`.
#[derive(Debug, Clone, PartialEq)]
pub struct ResearchReport {
    /// The document's path relative to the tree root, with `/`
    /// separators (e.g. `goals/atlas/briefs/kickoff.md`).
    pub rel_path: String,
    /// The document's full text.
    pub text: String,
    /// The file's mtime as RFC 3339 UTC, where the filesystem
    /// records one.
    pub updated_at: Option<String>,
    /// The document's physical sensitivity tier (the corpus's
    /// internal tier; reports carry no per-file tier of their own).
    pub sensitivity: Tier,
}

impl ResearchReport {
    /// The report's source-of-truth pointer: the tree-relative path
    /// a reader opens to see the source, carried as the staged
    /// item's `source` and in its provenance as `source_ref`.
    pub fn source_ref(&self) -> String {
        format!("research-reports:{}", self.rel_path)
    }
}

fn collect_into(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| {
            Error::fatal(format!(
                "research-reports connector: reading report tree {}: {e}",
                dir.display()
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| {
            Error::fatal(format!(
                "research-reports connector: reading report tree {}: {e}",
                dir.display()
            ))
        })?
        .into_iter()
        .map(|entry| entry.path())
        .collect();
    entries.sort();
    for path in entries {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        let file_type = std::fs::symlink_metadata(&path)
            .map(|m| m.file_type())
            .map_err(|e| {
                Error::fatal(format!(
                    "research-reports connector: stat {}: {e}",
                    path.display()
                ))
            })?;
        if file_type.is_dir() {
            collect_into(&path, out)?;
        } else if file_type.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_case("md"))
        {
            out.push(path);
        }
    }
    Ok(())
}

/// Collect every research report under `root`: the tree's `.md`
/// documents (goal files, briefs, reports), recursively, in
/// relative-path order. Hidden files and directories are skipped —
/// they are editor/VCS state, not reports. A document that cannot
/// be read, or whose bytes are not UTF-8, is a loud error naming
/// the path: a partial ingest presented as complete is exactly
/// what the registry's provenance discipline exists to prevent.
pub fn collect_research_reports(root: &Path) -> Result<Vec<ResearchReport>> {
    if !root.is_dir() {
        return Err(Error::fatal(format!(
            "research-reports connector: report tree {} is not a directory",
            root.display()
        )));
    }
    let mut files = Vec::new();
    collect_into(root, &mut files)?;
    files.sort();
    let mut out = Vec::with_capacity(files.len());
    for path in files {
        let rel = path
            .strip_prefix(root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .map_err(|_| {
                Error::fatal(format!(
                    "research-reports connector: {} is outside its tree {}",
                    path.display(),
                    root.display()
                ))
            })?;
        let bytes = std::fs::read(&path).map_err(|e| {
            Error::fatal(format!(
                "research-reports connector: reading report {rel}: {e}"
            ))
        })?;
        let text = String::from_utf8(bytes).map_err(|_| {
            Error::fatal(format!(
                "research-reports connector: report {rel} is not UTF-8 text"
            ))
        })?;
        let updated_at = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| rfc3339_utc(d.as_millis() as u64));
        out.push(ResearchReport {
            rel_path: rel,
            text,
            updated_at,
            sensitivity: Tier::Internal,
        });
    }
    Ok(out)
}

/// Stage one research report into a v1 staging DB as one `items`
/// row plus its `chunks_raw` rows, on the same write path as every
/// staged unit: `kind` is `research-report`, `source` is the
/// report's source-of-truth pointer, and `meta` carries the
/// design's provenance set. Returns the item's id. Re-staging an
/// unchanged report returns its existing id and writes nothing;
/// re-staging an edited report replaces exactly its own chunks.
/// The DB must already be a v1 staging DB
/// ([`check_stage_schema`]); vectors are produced later, at
/// promotion, as for every staged item.
pub fn stage_research_report(
    conn: &Connection,
    unit_id: &str,
    report: &ResearchReport,
    now: &str,
) -> Result<i64> {
    stage_research_report_with_status(conn, unit_id, report, now).map(|(id, _)| id)
}

fn stage_research_report_with_status(
    conn: &Connection,
    unit_id: &str,
    report: &ResearchReport,
    now: &str,
) -> Result<(i64, bool)> {
    let tx = conn.unchecked_transaction().map_err(|e| {
        Error::backend(format!(
            "research-reports connector: starting stage transaction: {e}"
        ))
    })?;
    let result = stage_research_report_in_transaction(&tx, unit_id, report, now)?;
    tx.commit().map_err(|e| {
        Error::backend(format!(
            "research-reports connector: committing stage transaction: {e}"
        ))
    })?;
    Ok(result)
}

fn stage_research_report_in_transaction(
    conn: &Connection,
    unit_id: &str,
    report: &ResearchReport,
    now: &str,
) -> Result<(i64, bool)> {
    check_stage_schema(conn)?;
    let chunks = chunk_artifact_text(&report.text);
    let source = report.source_ref();
    let meta = serde_json::json!({
        "corpus": RESEARCH_REPORTS_UNIT,
        "agent": serde_json::Value::Null,
        "source_ref": source,
        "updated_at": report.updated_at,
        "sensitivity": report.sensitivity.name(),
        "zone": "fleet",
        "staged_at": now,
        "chunks": chunks.len(),
    })
    .to_string();
    let existing = conn
        .query_row(
            "SELECT id, text, sensitivity, meta FROM items \
             WHERE unit_id = ?1 AND kind = 'research-report' AND source = ?2 ORDER BY id DESC LIMIT 1",
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
                "research-reports connector: finding item for {}: {e}",
                report.rel_path
            ))
        })?;

    let (item_id, changed) = match existing {
        Some((id, text, sensitivity, old_meta)) => {
            let unchanged = text == report.text
                && sensitivity == report.sensitivity.name()
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
                    report.text,
                    report.sensitivity.name(),
                    report.updated_at.as_deref().unwrap_or(now),
                    meta,
                    id,
                ],
            )
            .map_err(|e| {
                Error::backend(format!(
                    "research-reports connector: updating item for {}: {e}",
                    report.rel_path
                ))
            })?;
            conn.execute("DELETE FROM chunks_raw WHERE item_id = ?1", [id])
                .map_err(|e| {
                    Error::backend(format!(
                        "research-reports connector: replacing chunks for {}: {e}",
                        report.rel_path
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
                    "research-report",
                    source,
                    report.text,
                    report.sensitivity.name(),
                    report.updated_at.as_deref().unwrap_or(now),
                    meta,
                ],
            )
            .map_err(|e| {
                Error::backend(format!(
                    "research-reports connector: inserting item for {}: {e}",
                    report.rel_path
                ))
            })?;
            (conn.last_insert_rowid(), true)
        }
    };
    if changed {
        for (idx, chunk) in chunks.iter().enumerate() {
            conn.execute(
                "INSERT INTO chunks_raw (item_id, idx, text, content_hash, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![item_id, idx as i64, chunk, sha256_hex(chunk), now],
            )
            .map_err(|e| {
                Error::backend(format!(
                    "research-reports connector: inserting chunk {idx} for {}: {e}",
                    report.rel_path
                ))
            })?;
        }
    }
    Ok((item_id, changed))
}

/// Stage a whole collected tree into a unit, in collection order.
/// Returns the ids of the items that were inserted or re-staged —
/// unchanged reports contribute no id, so a no-change re-scan of a
/// tree returns an empty vec (the incremental-ingest oracle).
pub fn stage_research_reports(
    conn: &Connection,
    unit_id: &str,
    reports: &[ResearchReport],
    now: &str,
) -> Result<Vec<i64>> {
    reports
        .iter()
        .map(|report| stage_research_report_with_status(conn, unit_id, report, now))
        .filter_map(|result| match result {
            Ok((id, true)) => Some(Ok(id)),
            Ok((_, false)) => None,
            Err(error) => Some(Err(error)),
        })
        .collect()
}

/// [`stage_research_reports`] against a staging DB file: the form
/// the `atlas-stage ingest-reports` tool calls, so the CLI needs no
/// DB handle of its own.
pub fn stage_research_reports_in_db(
    db: &Path,
    unit_id: &str,
    reports: &[ResearchReport],
    now: &str,
) -> Result<Vec<i64>> {
    let conn = open(db)?;
    stage_research_reports(&conn, unit_id, reports, now)
}
