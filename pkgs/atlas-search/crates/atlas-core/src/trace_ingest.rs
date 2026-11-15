//! Trace ingestion + retention — traces are corpus (issues #9974
//! and #8246, follow-on 2 of the #6027 design, doc
//! 2026-11-10T20-41-01Z_design_atlas_retrieval_stats_feedback.md
//! section 4).
//!
//! Section 4 has three rules, and this module is where each lands:
//!
//! 1. Trace JSONL files are registered in the ingestion source set
//!    as source class "statistics and reports", so traces are
//!    themselves searchable through Atlas rather than living in a
//!    side store nobody queries. No source-set registry exists
//!    anywhere else in this repo (the ingestion-operations design's
//!    sweep was never implemented), so the registration lives here,
//!    beside the trace writer: `trace_source_registration` is the
//!    registry row, `record_included` is its inclusion test, and
//!    `ingest_traces` turns a traces directory into a `TraceCorpus`
//!    of searchable documents.
//! 2. Raw traces are retained 180 days hot; after that the raw
//!    record moves to the cold tier and a compaction aggregate is
//!    emitted for it. The aggregate tables of design section 5 are
//!    follow-on 3 and do not exist yet, so the aggregate lands as a
//!    per-file `atlas-trace-compaction/v1` record beside the cold
//!    file — the hook follow-on 3 will consume — and the per-result
//!    stage detail survives only in the cold-tier raw copy, exactly
//!    as section 4 prescribes (compaction summarises; the cold tier
//!    keeps the raw record under cold-corpus retention).
//! 3. A trace referenced by any feedback record is retained raw
//!    for as long as the feedback record exists. Feedback records
//!    are follow-ons 4/5 and do not exist yet either, so the
//!    exception is driven by `load_feedback_refs`: any JSONL file
//!    of feedback-shaped records naming query ids (a `query_id` or
//!    `trace_query_id` string field, or a `query_ids` array).
//!
//! Deletion interplay (section 4): this module never deletes a
//! trace because a source file was deleted — retention here is a
//! function of trace age and feedback references only. Trace
//! records carry hashes and state, never embed content, so a
//! retained trace cannot resurrect deleted content.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::{Value as Json, json};

use crate::error::{Error, Result};
use crate::trace::TRACE_SCHEMA;

/// The ingestion source class design section 4 assigns to traces.
pub const TRACE_SOURCE_CLASS: &str = "statistics and reports";

/// Stable id of the traces source in the ingestion source set.
pub const TRACE_SOURCE_ID: &str = "atlas-query-traces";

/// Raw traces stay in the hot tier this many days (design section 4).
pub const HOT_RETENTION_DAYS: i64 = 180;

/// Schema of the per-file compaction aggregate record emitted when
/// raw traces move to the cold tier (the section 5 hook).
pub const COMPACTION_SCHEMA: &str = "atlas-trace-compaction/v1";

/// Name of the cold-tier subdirectory inside the traces directory.
pub const COLD_DIR_NAME: &str = "cold";

/// One row of the ingestion source set: the traces source.
/// `file_pattern` is the JsonlTraceSink's per-day file name shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceSourceRegistration {
    pub source_id: &'static str,
    pub source_class: &'static str,
    pub record_schema: &'static str,
    pub file_pattern: &'static str,
    pub hot_retention_days: i64,
}

/// The traces source registration (design section 4, rule 1).
pub fn trace_source_registration() -> TraceSourceRegistration {
    TraceSourceRegistration {
        source_id: TRACE_SOURCE_ID,
        source_class: TRACE_SOURCE_CLASS,
        record_schema: TRACE_SCHEMA,
        file_pattern: "atlas-traces-YYYY-MM-DD.jsonl",
        hot_retention_days: HOT_RETENTION_DAYS,
    }
}

/// The registration's inclusion test: a record belongs to the
/// traces source exactly when it is an `atlas-query-trace/v1`
/// record. Anything else found in a trace file is skipped by
/// ingestion and never silently reclassified.
pub fn record_included(record: &Json) -> bool {
    record.get("schema").and_then(Json::as_str) == Some(TRACE_SCHEMA)
}

/// One ingested trace as a searchable corpus document. `text` is
/// the document body the corpus search matches over: the retained
/// query text when the surface stored it, otherwise the query text
/// hash (statistics still group on it), plus the identifying
/// fields a report would filter by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceDocument {
    pub query_id: String,
    pub ts: String,
    pub day: String,
    pub surface: String,
    pub requester_kind: String,
    pub requester_id: String,
    pub query_text: Option<String>,
    pub query_text_hash: String,
    pub served: u64,
    pub merged: u64,
    pub text: String,
}

fn document_from_record(record: &Json) -> Option<TraceDocument> {
    if !record_included(record) {
        return None;
    }
    let query_id = record.get("query_id")?.as_str()?.to_string();
    let ts = record
        .get("ts")
        .and_then(Json::as_str)
        .unwrap_or("")
        .to_string();
    let day = ts.get(..10).unwrap_or("").to_string();
    let surface = record
        .get("surface")
        .and_then(Json::as_str)
        .unwrap_or("")
        .to_string();
    let (requester_kind, requester_id) = match record.get("requester") {
        Some(r) => (
            r.get("kind")
                .and_then(Json::as_str)
                .unwrap_or("")
                .to_string(),
            r.get("id").and_then(Json::as_str).unwrap_or("").to_string(),
        ),
        None => (String::new(), String::new()),
    };
    let query_text = record
        .get("query_text")
        .and_then(Json::as_str)
        .map(str::to_string);
    let query_text_hash = record
        .get("query_text_hash")
        .and_then(Json::as_str)
        .unwrap_or("")
        .to_string();
    let pool_counts = record.get("pool_counts");
    let served = pool_counts
        .and_then(|p| p.get("served"))
        .and_then(Json::as_u64)
        .unwrap_or(0);
    let merged = pool_counts
        .and_then(|p| p.get("merged"))
        .and_then(Json::as_u64)
        .unwrap_or(0);
    let mut text_parts: Vec<String> = vec![
        query_id.clone(),
        ts.clone(),
        surface.clone(),
        requester_kind.clone(),
        requester_id.clone(),
        query_text_hash.clone(),
    ];
    if let Some(q) = &query_text {
        text_parts.push(q.clone());
    }
    if let Some(results) = record.get("results").and_then(Json::as_array) {
        for r in results {
            if let Some(cid) = r.get("chunk_id").and_then(Json::as_i64) {
                text_parts.push(cid.to_string());
            }
        }
    }
    Some(TraceDocument {
        query_id,
        ts,
        day,
        surface,
        requester_kind,
        requester_id,
        query_text,
        query_text_hash,
        served,
        merged,
        text: text_parts.join(" "),
    })
}

/// The ingested traces corpus: the registration the documents were
/// ingested under, the documents themselves, and the ingestion
/// accounting (files read, lines skipped as non-trace or malformed).
#[derive(Debug, Clone)]
pub struct TraceCorpus {
    pub registration: TraceSourceRegistration,
    pub documents: Vec<TraceDocument>,
    pub files: usize,
    pub skipped_lines: usize,
}

/// The per-day trace file name shape the sink writes and this
/// module reads: `atlas-traces-YYYY-MM-DD.jsonl`.
pub fn trace_file_day(file_name: &str) -> Option<(i64, u32, u32)> {
    let stem = file_name
        .strip_prefix("atlas-traces-")?
        .strip_suffix(".jsonl")?;
    let mut parts = stem.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !valid_civil_date(y, m, d) {
        return None;
    }
    Some((y, m, d))
}

fn trace_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = Vec::new();
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(files),
        Err(e) => {
            return Err(Error::backend(format!(
                "trace ingest: cannot list {}: {e}",
                dir.display()
            )));
        }
    };
    for ent in rd.flatten() {
        let name = ent.file_name().to_string_lossy().into_owned();
        if trace_file_day(&name).is_some() {
            files.push(ent.path());
        }
    }
    files.sort();
    Ok(files)
}

/// Ingest every hot trace file in `dir` into a searchable corpus.
/// Cold-tier files (under `dir/cold`) are not re-ingested: they
/// left the hot source set when retention moved them.
pub fn ingest_traces(dir: &Path) -> Result<TraceCorpus> {
    let files = trace_files(dir)?;
    let mut documents = Vec::new();
    let mut skipped_lines = 0usize;
    for path in &files {
        let body = std::fs::read_to_string(path).map_err(|e| {
            Error::backend(format!("trace ingest: cannot read {}: {e}", path.display()))
        })?;
        for line in body.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<Json>(line) {
                Ok(rec) => match document_from_record(&rec) {
                    Some(doc) => documents.push(doc),
                    None => skipped_lines += 1,
                },
                Err(_) => skipped_lines += 1,
            }
        }
    }
    Ok(TraceCorpus {
        registration: trace_source_registration(),
        documents,
        files: files.len(),
        skipped_lines,
    })
}

/// Corpus search over ingested traces: every query token must
/// appear in the document text (case-insensitive); results rank by
/// total token occurrences, ties by query_id for determinism. This
/// is the section 4 "searchable through Atlas" proof at the
/// library seam — the atlas-search CLI's `traces ingest --query`
/// runs exactly this over the ingested corpus.
pub fn search_traces<'a>(corpus: &'a TraceCorpus, query: &str) -> Vec<&'a TraceDocument> {
    let tokens: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric() && c != '-')
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect();
    if tokens.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(usize, &TraceDocument)> = corpus
        .documents
        .iter()
        .filter_map(|doc| {
            let text = doc.text.to_lowercase();
            let mut total = 0usize;
            for tok in &tokens {
                let n = text.matches(tok.as_str()).count();
                if n == 0 {
                    return None;
                }
                total += n;
            }
            Some((total, doc))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.query_id.cmp(&b.1.query_id)));
    scored.into_iter().map(|(_, d)| d).collect()
}

/// Load the set of query ids referenced by feedback records, from
/// JSONL files of feedback-shaped records (design section 6's
/// record shape is follow-on 4; the reference field is the
/// contract here): a `query_id` or `trace_query_id` string field,
/// or a `query_ids` array of strings, per line. Malformed lines
/// are ignored — a feedback file must never be able to crash
/// retention, and an unparseable line references nothing.
pub fn load_feedback_refs(paths: &[PathBuf]) -> Result<HashSet<String>> {
    let mut refs = HashSet::new();
    for path in paths {
        let body = std::fs::read_to_string(path).map_err(|e| {
            Error::backend(format!(
                "trace retention: cannot read feedback file {}: {e}",
                path.display()
            ))
        })?;
        for line in body.lines() {
            let Ok(rec) = serde_json::from_str::<Json>(line.trim()) else {
                continue;
            };
            for key in ["query_id", "trace_query_id"] {
                if let Some(id) = rec.get(key).and_then(Json::as_str) {
                    refs.insert(id.to_string());
                }
            }
            if let Some(ids) = rec.get("query_ids").and_then(Json::as_array) {
                for id in ids {
                    if let Some(id) = id.as_str() {
                        refs.insert(id.to_string());
                    }
                }
            }
        }
    }
    Ok(refs)
}

/// What one retention pass did. Counts are records unless the name
/// says files.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetentionReport {
    pub kept_files: usize,
    pub moved_files: usize,
    pub compacted_records: usize,
    pub exempt_records: usize,
    pub cold_files: Vec<String>,
}

/// Howard Hinnant's days-from-civil: days since 1970-01-01 for a
/// proleptic Gregorian date. The inverse of trace.rs's
/// civil_from_days; kept local so retention ages are computed from
/// the file name's day stamp with no clock or calendar dependency.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Whether `(y, m, d)` is a real proleptic Gregorian calendar
/// date: month in 1..=12 and day within that month, leap years
/// included. `days_from_civil` silently normalises out-of-range
/// components (month 99 advances the year count), so callers that
/// parse a date from text must gate on this first — an invalid
/// `today` would otherwise age every trace file by years. The
/// year is bounded to the four-digit YYYY range (0..=9999) the
/// trace file names and the `--today` format carry; the bound
/// also keeps `days_from_civil`'s era arithmetic far from i64
/// overflow (an unbounded year such as i64::MAX would overflow
/// `era * 146_097`).
pub fn valid_civil_date(y: i64, m: u32, d: u32) -> bool {
    if !(0..=9999).contains(&y) || !(1..=12).contains(&m) || d == 0 {
        return false;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days_in_month = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if leap {
                29
            } else {
                28
            }
        }
    };
    d <= days_in_month
}

/// Identity of one trace line for cold-tier deduplication: the
/// record's `query_id` when it parses and carries one, else the
/// line itself. Retention retries key on this so a record already
/// archived is never archived twice.
fn trace_line_key(line: &str) -> String {
    match serde_json::from_str::<Json>(line) {
        Ok(rec) => match rec.get("query_id").and_then(Json::as_str) {
            Some(qid) => format!("qid:{qid}"),
            None => format!("raw:{line}"),
        },
        Err(_) => format!("raw:{line}"),
    }
}

/// Accumulate one cold record's contribution to the compaction
/// aggregate (served / merged totals, per-surface count, query
/// text hash). Unparseable lines contribute nothing but the
/// record count, which the caller tracks separately.
fn accumulate_aggregate(
    line: &str,
    served_total: &mut u64,
    merged_total: &mut u64,
    surfaces: &mut BTreeMap<String, u64>,
    hashes: &mut Vec<String>,
) {
    let Some(rec) = serde_json::from_str::<Json>(line).ok() else {
        return;
    };
    if let Some(pc) = rec.get("pool_counts") {
        *served_total += pc.get("served").and_then(Json::as_u64).unwrap_or(0);
        *merged_total += pc.get("merged").and_then(Json::as_u64).unwrap_or(0);
    }
    let surface = rec
        .get("surface")
        .and_then(Json::as_str)
        .unwrap_or("")
        .to_string();
    *surfaces.entry(surface).or_default() += 1;
    if let Some(h) = rec.get("query_text_hash").and_then(Json::as_str) {
        hashes.push(h.to_string());
    }
}

/// Monotonic counter for unique temp-file names within a process,
/// so concurrent writers never share a path.
static ATOMIC_TMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Replace `path` with `contents` atomically: write the full
/// contents to a unique, exclusively created temporary file in the
/// same directory, sync it, then rename over the destination. A
/// failure at any step before the rename leaves the original file
/// intact — a truncating write could destroy the only complete
/// copy of a feedback-exempt or previously archived record. The
/// temporary path is unique per write (pid + counter + timestamp)
/// and created with `create_new`, so concurrent writers cannot
/// truncate each other's in-progress file before either rename.
fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    use std::io::{ErrorKind, Write as _};
    let parent = path.parent().ok_or_else(|| {
        Error::backend(format!("trace retention: no parent for {}", path.display()))
    })?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let mut last_err: Option<std::io::Error> = None;
    let mut tmp = None;
    let mut file = None;
    for _ in 0..32 {
        let n = ATOMIC_TMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let candidate = parent.join(format!(".{name}.tmp-{pid}-{n}-{nanos}"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(f) => {
                tmp = Some(candidate);
                file = Some(f);
                break;
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                last_err = Some(e);
                continue;
            }
            Err(e) => {
                last_err = Some(e);
                break;
            }
        }
    }
    let (tmp_path, mut f) = match (tmp, file) {
        (Some(p), Some(f)) => (p, f),
        _ => {
            let e = last_err
                .map(|e| e.to_string())
                .unwrap_or_else(|| "could not allocate unique temp file".to_string());
            return Err(Error::backend(format!(
                "trace retention: cannot write {}: {e}",
                path.display()
            )));
        }
    };
    let write = (|| -> std::io::Result<()> {
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
        Ok(())
    })();
    if let Err(e) = write {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(Error::backend(format!(
            "trace retention: cannot write {}: {e}",
            path.display()
        )));
    }
    drop(f);
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(Error::backend(format!(
            "trace retention: cannot write {}: {e}",
            path.display()
        )));
    }
    Ok(())
}

/// Rebuild one day's `atlas-trace-compaction/v1` aggregate from
/// the COMPLETE resulting cold file and write it atomically.
/// Building from the full cold contents — never from one pass's
/// lines alone — keeps the aggregate describing every archived
/// record after any number of passes, including a repair pass
/// that moves nothing.
fn write_aggregate(
    cold_dir: &Path,
    day: (i64, u32, u32),
    result_lines: &[String],
    exempt_records: usize,
) -> Result<()> {
    let mut served_total: u64 = 0;
    let mut merged_total: u64 = 0;
    let mut surfaces: BTreeMap<String, u64> = BTreeMap::new();
    let mut hashes: Vec<String> = Vec::new();
    for line in result_lines {
        accumulate_aggregate(
            line,
            &mut served_total,
            &mut merged_total,
            &mut surfaces,
            &mut hashes,
        );
    }
    let aggregate = json!({
        "schema": COMPACTION_SCHEMA,
        "source_id": TRACE_SOURCE_ID,
        "source_class": TRACE_SOURCE_CLASS,
        "day": format!("{:04}-{:02}-{:02}", day.0, day.1, day.2),
        "compacted_records": result_lines.len(),
        "exempt_records": exempt_records,
        "served_total": served_total,
        "merged_total": merged_total,
        "surfaces": surfaces,
        "query_text_hashes": hashes,
    });
    let aggregate_path = cold_dir.join(format!(
        "atlas-traces-{:04}-{:02}-{:02}.aggregate.json",
        day.0, day.1, day.2
    ));
    write_atomic(
        &aggregate_path,
        &serde_json::to_string_pretty(&aggregate).unwrap_or_default(),
    )
}

/// Cross-process lock for the retention transaction, held for the
/// full read-merge-write-cleanup pass so concurrent processes cannot
/// apply updates from stale snapshots (e.g. overwrite the archive
/// with a snapshot that predates another process's move and then
/// drop the hot file). Uses an advisory directory lock in the
/// traces dir (mkdir is atomic across processes) and removes it on
/// drop.
struct RetentionLock(PathBuf);

impl RetentionLock {
    fn acquire(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir).map_err(|e| {
            Error::backend(format!(
                "trace retention: cannot create {}: {e}",
                dir.display()
            ))
        })?;
        let lock = dir.join(".retention.lock");
        match std::fs::create_dir(&lock) {
            Ok(()) => Ok(Self(lock)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(Error::backend(format!(
                    "trace retention: another retention run is in progress (lock {} exists); if no run is active, a previous run was interrupted — remove the lock dir and retry",
                    lock.display()
                )))
            }
            Err(e) => Err(Error::backend(format!(
                "trace retention: cannot take lock {}: {e}",
                lock.display()
            ))),
        }
    }
}

impl Drop for RetentionLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}

/// Run the section 4 retention pass over the hot trace files in
/// `dir`, ageing each file by its name's day stamp against
/// `today` (a (year, month, day) triple, injected so the pass is
/// deterministic under test):
///
/// - age <= 180 days: the file stays hot, untouched.
/// - age > 180 days: records whose query_id is in `feedback_refs`
///   stay raw in the hot file (the feedback exception); every
///   other record is appended verbatim to
///   `dir/cold/atlas-traces-<day>.jsonl` and summarised in a
///   `dir/cold/atlas-traces-<day>.aggregate.json` compaction
///   record (schema `atlas-trace-compaction/v1`: counts, served /
///   merged totals, per-surface counts, query text hashes — the
///   aggregate hook follow-on 3's section 5 tables will consume).
///   A file whose records all move is removed from the hot tier.
///
/// The move is safe to retry: the cold file is merged
/// idempotently (records keyed by query_id, archived exactly
/// once), the aggregate is rebuilt from the complete resulting
/// cold file, every replacement is written atomically (a unique,
/// exclusively created temp file + sync + rename in the same
/// directory), and the hot cleanup happens only after the cold
/// file and its aggregate are in place — a failure mid-pass
/// leaves the hot file intact for a retry that cannot duplicate
/// or lose records. The complete read-merge-write-cleanup pass is
/// serialized by a cross-process directory lock so concurrent
/// processes cannot apply updates from stale snapshots.
///
/// Lines that do not parse as trace records are treated as
/// compactable (they carry no query_id a feedback record could
/// reference) and are preserved verbatim in the cold file.
pub fn run_retention(
    dir: &Path,
    today: (i64, u32, u32),
    feedback_refs: &HashSet<String>,
) -> Result<RetentionReport> {
    let mut report = RetentionReport::default();
    if !valid_civil_date(today.0, today.1, today.2) {
        return Err(Error::backend(format!(
            "trace retention: invalid today date {:04}-{:02}-{:02}",
            today.0, today.1, today.2
        )));
    }
    let _lock = RetentionLock::acquire(dir)?;
    let today_days = days_from_civil(today.0, today.1, today.2);
    let cold_dir = dir.join(COLD_DIR_NAME);
    for path in trace_files(dir)? {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Some(day) = trace_file_day(&name) else {
            continue;
        };
        let age = today_days - days_from_civil(day.0, day.1, day.2);
        if age <= HOT_RETENTION_DAYS {
            report.kept_files += 1;
            continue;
        }
        let body = std::fs::read_to_string(&path).map_err(|e| {
            Error::backend(format!(
                "trace retention: cannot read {}: {e}",
                path.display()
            ))
        })?;
        let mut exempt_lines: Vec<&str> = Vec::new();
        let mut cold_lines: Vec<&str> = Vec::new();
        for line in body.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let parsed = serde_json::from_str::<Json>(line).ok();
            let qid = parsed
                .as_ref()
                .and_then(|r| r.get("query_id"))
                .and_then(Json::as_str);
            if qid.is_some_and(|id| feedback_refs.contains(id)) {
                exempt_lines.push(line);
                report.exempt_records += 1;
                continue;
            }
            cold_lines.push(line);
        }
        let cold_path = cold_dir.join(&name);
        // Idempotent archive merge: records already in the cold
        // file (from an earlier pass, or from a pass that failed
        // after the cold write but before the hot cleanup) are
        // kept exactly once, keyed by query_id. A retry therefore
        // cannot duplicate raw traces, and the aggregate below is
        // rebuilt from the complete resulting cold file, so it
        // describes every archived record — not just this pass's.
        let mut result_lines: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        if cold_path.exists() {
            let existing = std::fs::read_to_string(&cold_path).map_err(|e| {
                Error::backend(format!(
                    "trace retention: cannot read {}: {e}",
                    cold_path.display()
                ))
            })?;
            for line in existing.lines() {
                let line = line.trim();
                if !line.is_empty() && seen.insert(trace_line_key(line)) {
                    result_lines.push(line.to_string());
                }
            }
        }
        if cold_lines.is_empty() {
            // Everything left in the file is feedback-referenced:
            // the file stays hot exactly as written. But an
            // earlier pass may have archived records into the
            // cold file and then failed before writing the
            // aggregate — repair that here: rewrite the cold file
            // deduplicated and rebuild its aggregate, so the
            // archive is never left without one (or with a stale
            // one) just because this pass moves nothing.
            if cold_path.exists() {
                let mut out = String::new();
                for line in &result_lines {
                    out.push_str(line);
                    out.push('\n');
                }
                write_atomic(&cold_path, &out)?;
                write_aggregate(&cold_dir, day, &result_lines, exempt_lines.len())?;
            }
            report.kept_files += 1;
            continue;
        }
        std::fs::create_dir_all(&cold_dir).map_err(|e| {
            Error::backend(format!(
                "trace retention: cannot create {}: {e}",
                cold_dir.display()
            ))
        })?;
        for line in &cold_lines {
            if seen.insert(trace_line_key(line)) {
                result_lines.push((*line).to_string());
            }
        }
        let mut out = String::new();
        for line in &result_lines {
            out.push_str(line);
            out.push('\n');
        }
        // Commit order: cold file first, then its aggregate, and
        // only then the hot cleanup. Every write is atomic, and
        // any failure before the hot cleanup leaves the hot file
        // intact so the pass can be retried — the merge above
        // makes that retry a no-op for records already archived.
        write_atomic(&cold_path, &out)?;
        write_aggregate(&cold_dir, day, &result_lines, exempt_lines.len())?;
        report.compacted_records += cold_lines.len();
        report.moved_files += 1;
        report
            .cold_files
            .push(cold_path.to_string_lossy().into_owned());
        if exempt_lines.is_empty() {
            std::fs::remove_file(&path).map_err(|e| {
                Error::backend(format!(
                    "trace retention: cannot remove {}: {e}",
                    path.display()
                ))
            })?;
        } else {
            let mut out = String::new();
            for line in &exempt_lines {
                out.push_str(line);
                out.push('\n');
            }
            write_atomic(&path, &out)?;
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("atlas-trace-ingest-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn trace_line(query_id: &str, ts: &str, query_text: &str, hash: &str) -> String {
        json!({
            "schema": TRACE_SCHEMA,
            "query_id": query_id,
            "ts": ts,
            "duration_ms": 12,
            "surface": "cli",
            "requester": {"kind": "agent", "id": "test"},
            "query_text_hash": hash,
            "query_text": query_text,
            "results": [{"chunk_id": 42}, {"chunk_id": 77}],
            "pool_counts": {"served": 2, "merged": 5},
        })
        .to_string()
    }

    fn write_file(dir: &Path, name: &str, lines: &[String]) -> PathBuf {
        let path = dir.join(name);
        let mut body = lines.join("\n");
        if !body.is_empty() {
            body.push('\n');
        }
        std::fs::write(&path, body).expect("write trace file");
        path
    }

    #[test]
    fn registration_is_statistics_and_reports() {
        let reg = trace_source_registration();
        assert_eq!(reg.source_class, "statistics and reports");
        assert_eq!(reg.record_schema, TRACE_SCHEMA);
        assert_eq!(reg.hot_retention_days, 180);
        assert!(record_included(&json!({"schema": TRACE_SCHEMA})));
        assert!(!record_included(&json!({"schema": "other/v9"})));
        assert!(!record_included(&json!({"no_schema": true})));
    }

    #[test]
    fn days_from_civil_known_values() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(1970, 1, 2), 1);
        // 2026-05-18 is exactly 180 days before 2026-11-14.
        assert_eq!(
            days_from_civil(2026, 11, 14) - days_from_civil(2026, 5, 18),
            180
        );
        assert_eq!(
            days_from_civil(2026, 11, 14) - days_from_civil(2026, 5, 17),
            181
        );
    }

    #[test]
    fn ingest_and_search_finds_traces() {
        let dir = temp_dir("ingest");
        write_file(
            &dir,
            "atlas-traces-2026-11-13.jsonl",
            &[
                trace_line(
                    "019abcde-0000-7000-8000-000000000001",
                    "2026-11-13T10:00:00Z",
                    "how do we back up the cluster",
                    "hashone",
                ),
                trace_line(
                    "019abcde-0000-7000-8000-000000000002",
                    "2026-11-13T11:00:00Z",
                    "restart the laptop agent",
                    "hashtwo",
                ),
                "{\"schema\":\"not-a-trace\"}".to_string(),
                "not json at all".to_string(),
            ],
        );
        // A cold-tier file must not be re-ingested.
        let cold = dir.join(COLD_DIR_NAME);
        std::fs::create_dir_all(&cold).expect("cold dir");
        write_file(
            &cold,
            "atlas-traces-2025-01-01.jsonl",
            &[trace_line(
                "019abcde-0000-7000-8000-000000000099",
                "2025-01-01T10:00:00Z",
                "ancient cold query",
                "hashcold",
            )],
        );
        let corpus = ingest_traces(&dir).expect("ingest");
        assert_eq!(corpus.registration.source_class, TRACE_SOURCE_CLASS);
        assert_eq!(corpus.files, 1);
        assert_eq!(corpus.documents.len(), 2);
        assert_eq!(corpus.skipped_lines, 2);
        let hits = search_traces(&corpus, "back up cluster");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].query_id, "019abcde-0000-7000-8000-000000000001");
        assert_eq!(hits[0].served, 2);
        // Searchable by query_id and by chunk id too.
        assert_eq!(search_traces(&corpus, "000000000002").len(), 1);
        assert_eq!(search_traces(&corpus, "77").len(), 2);
        assert!(search_traces(&corpus, "nonexistenttoken").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn retention_moves_aged_and_keeps_recent() {
        let dir = temp_dir("retention");
        let aged = write_file(
            &dir,
            "atlas-traces-2026-05-17.jsonl",
            &[trace_line(
                "019abcde-0000-7000-8000-000000000010",
                "2026-05-17T09:00:00Z",
                "aged query",
                "hashaged",
            )],
        );
        let boundary = write_file(
            &dir,
            "atlas-traces-2026-05-18.jsonl",
            &[trace_line(
                "019abcde-0000-7000-8000-000000000011",
                "2026-05-18T09:00:00Z",
                "boundary query",
                "hashboundary",
            )],
        );
        let report = run_retention(&dir, (2026, 11, 14), &HashSet::new()).expect("retention");
        assert_eq!(report.moved_files, 1);
        assert_eq!(report.kept_files, 1);
        assert_eq!(report.compacted_records, 1);
        assert!(!aged.exists(), "aged hot file removed");
        assert!(boundary.exists(), "exactly-180-day file stays hot");
        let cold = dir
            .join(COLD_DIR_NAME)
            .join("atlas-traces-2026-05-17.jsonl");
        assert!(cold.exists(), "cold copy exists");
        let aggregate: Json = serde_json::from_str(
            &std::fs::read_to_string(
                dir.join(COLD_DIR_NAME)
                    .join("atlas-traces-2026-05-17.aggregate.json"),
            )
            .expect("aggregate exists"),
        )
        .expect("aggregate parses");
        assert_eq!(aggregate["schema"], COMPACTION_SCHEMA);
        assert_eq!(aggregate["compacted_records"], 1);
        assert_eq!(aggregate["served_total"], 2);
        assert_eq!(aggregate["query_text_hashes"][0], "hashaged");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn retention_feedback_referenced_traces_survive() {
        let dir = temp_dir("feedback");
        let aged = write_file(
            &dir,
            "atlas-traces-2026-01-01.jsonl",
            &[
                trace_line(
                    "019abcde-0000-7000-8000-000000000020",
                    "2026-01-01T09:00:00Z",
                    "referenced query",
                    "hashref",
                ),
                trace_line(
                    "019abcde-0000-7000-8000-000000000021",
                    "2026-01-01T10:00:00Z",
                    "unreferenced query",
                    "hashunref",
                ),
            ],
        );
        let feedback = write_file(
            &dir,
            "feedback.jsonl",
            &[
                json!({"query_id": "019abcde-0000-7000-8000-000000000020", "verdict": "wrong"})
                    .to_string(),
            ],
        );
        let refs = load_feedback_refs(&[feedback]).expect("feedback refs");
        assert!(refs.contains("019abcde-0000-7000-8000-000000000020"));
        let report = run_retention(&dir, (2026, 11, 14), &refs).expect("retention");
        assert_eq!(report.exempt_records, 1);
        assert_eq!(report.compacted_records, 1);
        // The hot file survives holding only the referenced record.
        let hot = std::fs::read_to_string(&aged).expect("hot file remains");
        assert!(hot.contains("000000000020"));
        assert!(!hot.contains("000000000021"));
        // The cold file holds only the unreferenced record.
        let cold = std::fs::read_to_string(
            dir.join(COLD_DIR_NAME)
                .join("atlas-traces-2026-01-01.jsonl"),
        )
        .expect("cold file");
        assert!(cold.contains("000000000021"));
        assert!(!cold.contains("000000000020"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn valid_civil_date_rejects_out_of_range() {
        assert!(valid_civil_date(2026, 11, 14));
        assert!(valid_civil_date(2024, 2, 29), "leap year Feb 29");
        assert!(!valid_civil_date(2026, 2, 29), "non-leap Feb 29");
        assert!(!valid_civil_date(2026, 99, 14));
        assert!(!valid_civil_date(2026, 0, 10));
        assert!(!valid_civil_date(2026, 4, 31));
        assert!(!valid_civil_date(2026, 1, 0));
        assert!(!valid_civil_date(i64::MAX, 1, 1), "oversized year");
        assert!(!valid_civil_date(10_000, 1, 1), "five-digit year");
        assert!(valid_civil_date(9999, 12, 31));
        // An invalid today is a usage error, never a silent
        // normalisation that ages recent traces by years.
        let dir = temp_dir("bad-today");
        assert!(run_retention(&dir, (2026, 99, 14), &HashSet::new()).is_err());
        assert!(run_retention(&dir, (2026, 2, 30), &HashSet::new()).is_err());
        assert!(run_retention(&dir, (i64::MAX, 1, 1), &HashSet::new()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn retention_retry_after_cold_write_does_not_duplicate() {
        // Simulate a pass that wrote the cold file and then failed
        // before the hot cleanup: the record sits in BOTH tiers.
        // The retry must archive it exactly once.
        let dir = temp_dir("retry");
        let line = trace_line(
            "019abcde-0000-7000-8000-000000000030",
            "2026-01-01T09:00:00Z",
            "retry query",
            "hashretry",
        );
        let aged = write_file(&dir, "atlas-traces-2026-01-01.jsonl", &[line.clone()]);
        let cold_dir = dir.join(COLD_DIR_NAME);
        std::fs::create_dir_all(&cold_dir).expect("cold dir");
        write_file(&cold_dir, "atlas-traces-2026-01-01.jsonl", &[line]);
        let report = run_retention(&dir, (2026, 11, 14), &HashSet::new()).expect("retention");
        assert_eq!(report.moved_files, 1);
        assert!(!aged.exists(), "hot file cleaned up on retry");
        let cold = std::fs::read_to_string(cold_dir.join("atlas-traces-2026-01-01.jsonl"))
            .expect("cold file");
        assert_eq!(
            cold.matches("000000000030").count(),
            1,
            "archived exactly once"
        );
        let aggregate: Json = serde_json::from_str(
            &std::fs::read_to_string(cold_dir.join("atlas-traces-2026-01-01.aggregate.json"))
                .expect("aggregate exists"),
        )
        .expect("aggregate parses");
        assert_eq!(aggregate["compacted_records"], 1);
        assert_eq!(aggregate["served_total"], 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn retention_rebuilds_missing_aggregate_when_all_remaining_exempt() {
        // A pass archived record A into the cold file, then failed
        // before writing the aggregate, leaving the hot file in
        // place. Feedback then references every record left hot,
        // so the retry moves nothing — it must still rebuild the
        // missing aggregate from the existing cold file.
        let dir = temp_dir("aggregate-repair");
        let line_a = trace_line(
            "019abcde-0000-7000-8000-000000000050",
            "2026-01-01T09:00:00Z",
            "already archived",
            "hasharchived",
        );
        let line_b = trace_line(
            "019abcde-0000-7000-8000-000000000051",
            "2026-01-01T10:00:00Z",
            "still exempt",
            "hashexempt",
        );
        let aged = write_file(
            &dir,
            "atlas-traces-2026-01-01.jsonl",
            &[line_a.clone(), line_b],
        );
        let cold_dir = dir.join(COLD_DIR_NAME);
        std::fs::create_dir_all(&cold_dir).expect("cold dir");
        write_file(&cold_dir, "atlas-traces-2026-01-01.jsonl", &[line_a]);
        // No aggregate file: the earlier pass failed before writing it.
        let mut refs = HashSet::new();
        refs.insert("019abcde-0000-7000-8000-000000000050".to_string());
        refs.insert("019abcde-0000-7000-8000-000000000051".to_string());
        let report = run_retention(&dir, (2026, 11, 14), &refs).expect("retention");
        assert_eq!(report.moved_files, 0);
        assert_eq!(report.kept_files, 1);
        assert!(aged.exists(), "all-exempt hot file stays hot");
        let aggregate: Json = serde_json::from_str(
            &std::fs::read_to_string(cold_dir.join("atlas-traces-2026-01-01.aggregate.json"))
                .expect("aggregate rebuilt"),
        )
        .expect("aggregate parses");
        assert_eq!(aggregate["compacted_records"], 1);
        assert_eq!(aggregate["served_total"], 2);
        assert_eq!(aggregate["query_text_hashes"][0], "hasharchived");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn retention_second_pass_preserves_aggregate_history() {
        // A is archived while B is feedback-exempt; when B's
        // exemption later lapses, the aggregate must still
        // describe A as well as B.
        let dir = temp_dir("aggregate-history");
        write_file(
            &dir,
            "atlas-traces-2026-01-01.jsonl",
            &[
                trace_line(
                    "019abcde-0000-7000-8000-000000000040",
                    "2026-01-01T09:00:00Z",
                    "first archived",
                    "hashfirst",
                ),
                trace_line(
                    "019abcde-0000-7000-8000-000000000041",
                    "2026-01-01T10:00:00Z",
                    "later archived",
                    "hashlater",
                ),
            ],
        );
        let mut refs = HashSet::new();
        refs.insert("019abcde-0000-7000-8000-000000000041".to_string());
        run_retention(&dir, (2026, 11, 14), &refs).expect("first pass");
        run_retention(&dir, (2026, 11, 14), &HashSet::new()).expect("second pass");
        let aggregate: Json = serde_json::from_str(
            &std::fs::read_to_string(
                dir.join(COLD_DIR_NAME)
                    .join("atlas-traces-2026-01-01.aggregate.json"),
            )
            .expect("aggregate exists"),
        )
        .expect("aggregate parses");
        assert_eq!(aggregate["compacted_records"], 2);
        assert_eq!(aggregate["served_total"], 4);
        let hashes: Vec<&str> = aggregate["query_text_hashes"]
            .as_array()
            .expect("hashes array")
            .iter()
            .filter_map(Json::as_str)
            .collect();
        assert!(
            hashes.contains(&"hashfirst"),
            "first pass history kept: {hashes:?}"
        );
        assert!(
            hashes.contains(&"hashlater"),
            "second pass included: {hashes:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
