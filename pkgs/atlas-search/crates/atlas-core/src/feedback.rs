//! Feedback capture — `atlas-feedback/v1` (issue #9976,
//! follow-on 4 of the #6027 design, doc
//! `2026-11-10T20-41-01Z_design_atlas_retrieval_stats_feedback.md`
//! section 6).
//!
//! Every surface that returns results can record a thumbs vote
//! against the served result it came from: feedback attaches to
//! the `(query_id, chunk_id)` result identity (design invariant
//! 3), never to a rank position, and a per-query vote is the same
//! record with `chunk_id: null`.
//!
//! Raw records are append-only and never mutated (invariant 4):
//! they land verbatim in one JSONL file per day under the Atlas
//! state root (`feedback/atlas-feedback-YYYY-MM-DD.jsonl`), in
//! exactly the shape the statistics loader (`load_feedback_votes`)
//! and the retention loader (`load_feedback_refs`) already read.
//! Supersede (§6.4) is a DERIVED view over the raw store: one
//! ACTIVE vote per `(requester, query_id, chunk_id)` — the latest
//! record for the key — while every superseded record stays in
//! the raw store and is reported as superseded, never rewritten.
//!
//! UI affordance contract (§6.1, the UI surface of this
//! follow-on): a UI renders one thumbs-up and one thumbs-down
//! control WITH each result; activating one records the vote in
//! that single action — no navigation and no form before the
//! vote is recorded; the optional comment is offered only after
//! (or alongside) the recorded vote; the surface reports
//! `opened` = whether the result was opened/expanded before the
//! vote. A UI that cannot carry `(query_id, chunk_id)` does not
//! serve results (invariant 1), so it always has the pair a vote
//! needs.

use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value as Json, json};

use crate::error::{Error, Result};
use crate::trace::{new_query_id, rfc3339_utc};

pub const FEEDBACK_SCHEMA: &str = "atlas-feedback/v1";

/// Comment size ceiling: 256 KiB. The design (§6.1) requires the
/// capture field to support at least 64 KiB ("minimum supported
/// 64 KiB"); this accepts four times that and still bounds a
/// single record well inside the service's request-body cap.
pub const MAX_COMMENT_BYTES: usize = 256 * 1024;

/// Requester kinds the aggregation weights know (§6.3), plus
/// `anonymous` (accepted, weighted 0.1 by `requester_weight`).
pub const REQUESTER_KINDS: [&str; 4] = ["drew", "agent", "user", "anonymous"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vote {
    Up,
    Down,
}

impl Vote {
    pub fn as_str(self) -> &'static str {
        match self {
            Vote::Up => "up",
            Vote::Down => "down",
        }
    }

    pub fn parse(s: &str) -> Result<Vote> {
        match s {
            "up" => Ok(Vote::Up),
            "down" => Ok(Vote::Down),
            _ => Err(Error::fatal(format!(
                "feedback vote must be \"up\" or \"down\", got {s:?}"
            ))),
        }
    }
}

/// One vote as a surface submits it (before the store stamps the
/// `feedback_id` and `ts`).
#[derive(Debug, Clone, PartialEq)]
pub struct FeedbackInput {
    pub query_id: String,
    /// `None` = a per-query vote (§6.1).
    pub chunk_id: Option<i64>,
    pub vote: Vote,
    /// Verbatim, never normalised or truncated by capture (§6.2
    /// normalisation is a separate derived pass, follow-on 5).
    pub comment: Option<String>,
    /// One of [`REQUESTER_KINDS`]; empty means `anonymous`.
    pub requester_kind: String,
    /// Stable requester id; may be empty for anonymous.
    pub requester_id: String,
    /// Whether the result was opened/expanded before the vote
    /// (§6.4). Per-query votes report the set-level analogue the
    /// surface knows; the field is stored as given.
    pub opened: bool,
}

impl FeedbackInput {
    pub fn new(query_id: impl Into<String>, vote: Vote) -> FeedbackInput {
        FeedbackInput {
            query_id: query_id.into(),
            chunk_id: None,
            vote,
            comment: None,
            requester_kind: "anonymous".to_string(),
            requester_id: String::new(),
            opened: false,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.query_id.trim().is_empty() {
            return Err(Error::fatal("feedback query_id must not be empty"));
        }
        if let Some(c) = &self.comment
            && c.len() > MAX_COMMENT_BYTES
        {
            return Err(Error::fatal(format!(
                "feedback comment is {} bytes; the capture ceiling is {MAX_COMMENT_BYTES} bytes (design section 6.1 supports at least 64 KiB)",
                c.len()
            )));
        }
        if !self.requester_kind.is_empty()
            && !REQUESTER_KINDS.contains(&self.requester_kind.as_str())
        {
            return Err(Error::fatal(format!(
                "feedback requester kind must be one of {}, got {:?}",
                REQUESTER_KINDS.join(" | "),
                self.requester_kind
            )));
        }
        Ok(())
    }

    fn requester_kind_or_anonymous(&self) -> &str {
        if self.requester_kind.is_empty() {
            "anonymous"
        } else {
            &self.requester_kind
        }
    }
}

/// One raw feedback record, exactly as stored. Records are never
/// mutated after the write; supersede state is derived by
/// [`active_view`], not stored.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedbackRecord {
    pub feedback_id: String,
    pub query_id: String,
    pub chunk_id: Option<i64>,
    pub vote: Vote,
    pub comment: Option<String>,
    pub requester_kind: String,
    pub requester_id: String,
    pub ts: String,
    pub opened: bool,
}

impl FeedbackRecord {
    /// The §6.4 active-vote key: one active vote per
    /// `(requester, query_id, chunk_id)`.
    fn active_key(&self) -> (String, String, String, Option<i64>) {
        (
            self.requester_kind.clone(),
            self.requester_id.clone(),
            self.query_id.clone(),
            self.chunk_id,
        )
    }

    pub fn to_json(&self) -> Json {
        let mut rec = Map::new();
        rec.insert("schema".into(), Json::String(FEEDBACK_SCHEMA.to_string()));
        rec.insert("feedback_id".into(), Json::String(self.feedback_id.clone()));
        rec.insert("query_id".into(), Json::String(self.query_id.clone()));
        rec.insert(
            "chunk_id".into(),
            self.chunk_id.map(Json::from).unwrap_or(Json::Null),
        );
        rec.insert("vote".into(), Json::String(self.vote.as_str().to_string()));
        rec.insert(
            "comment".into(),
            self.comment
                .as_ref()
                .map_or(Json::Null, |c| Json::String(c.clone())),
        );
        rec.insert(
            "requester".into(),
            json!({"kind": self.requester_kind, "id": self.requester_id}),
        );
        rec.insert("ts".into(), Json::String(self.ts.clone()));
        rec.insert("opened".into(), Json::Bool(self.opened));
        Json::Object(rec)
    }

    /// Parse one raw-store line. `None` for a line that is not a
    /// feedback record — loaders skip malformed lines rather than
    /// failing the whole store, the same philosophy as
    /// `load_feedback_votes`.
    pub fn from_json(rec: &Json) -> Option<FeedbackRecord> {
        let query_id = rec.get("query_id").and_then(Json::as_str)?;
        let vote = Vote::parse(rec.get("vote").and_then(Json::as_str)?).ok()?;
        let (kind, id) = match rec.get("requester") {
            Some(r) => (
                r.get("kind")
                    .and_then(Json::as_str)
                    .unwrap_or("anonymous")
                    .to_string(),
                r.get("id").and_then(Json::as_str).unwrap_or("").to_string(),
            ),
            None => ("anonymous".to_string(), String::new()),
        };
        Some(FeedbackRecord {
            feedback_id: rec
                .get("feedback_id")
                .and_then(Json::as_str)
                .unwrap_or("")
                .to_string(),
            query_id: query_id.to_string(),
            chunk_id: rec.get("chunk_id").and_then(Json::as_i64),
            vote,
            comment: rec
                .get("comment")
                .and_then(Json::as_str)
                .map(str::to_string),
            requester_kind: kind,
            requester_id: id,
            ts: rec
                .get("ts")
                .and_then(Json::as_str)
                .unwrap_or("")
                .to_string(),
            opened: rec.get("opened").and_then(Json::as_bool).unwrap_or(false),
        })
    }
}

/// The feedback persistence seam. Implementations must return
/// Err on any failure — a vote that could not be stored was not
/// cast, and the surface must say so rather than pretend.
pub trait FeedbackSink {
    fn write_feedback(&self, record: &FeedbackRecord) -> Result<()>;
}

/// The production sink: one JSONL file per day under the Atlas
/// state root (`feedback/atlas-feedback-YYYY-MM-DD.jsonl`),
/// appended one record per line, beside the trace store.
#[derive(Debug, Clone)]
pub struct JsonlFeedbackSink {
    dir: PathBuf,
}

impl JsonlFeedbackSink {
    pub fn new(dir: PathBuf) -> JsonlFeedbackSink {
        JsonlFeedbackSink { dir }
    }

    pub fn dir(&self) -> &PathBuf {
        &self.dir
    }

    /// `~/.local/state/atlas-search/feedback` — the same state
    /// root as the trace sink and the service config.
    pub fn default_dir() -> PathBuf {
        let home = std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        home.join(".local/state/atlas-search/feedback")
    }
}

impl FeedbackSink for JsonlFeedbackSink {
    fn write_feedback(&self, record: &FeedbackRecord) -> Result<()> {
        std::fs::create_dir_all(&self.dir).map_err(|e| {
            Error::backend(format!(
                "feedback sink: cannot create {}: {e}",
                self.dir.display()
            ))
        })?;
        // File per day, from the record's own ts (its date part).
        let day = record.ts.get(..10).unwrap_or("unknown");
        let path = self.dir.join(format!("atlas-feedback-{day}.jsonl"));
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| {
                Error::backend(format!(
                    "feedback sink: cannot open {}: {e}",
                    path.display()
                ))
            })?;
        let line = serde_json::to_string(&record.to_json())
            .map_err(|e| Error::backend(format!("feedback sink: cannot serialise record: {e}")))?;
        f.write_all(line.as_bytes())
            .and_then(|()| f.write_all(b"\n"))
            .map_err(|e| {
                Error::backend(format!(
                    "feedback sink: cannot write {}: {e}",
                    path.display()
                ))
            })?;
        Ok(())
    }
}

/// Validate + stamp + store one vote. The returned record is the
/// raw record as written (its `feedback_id` is the handle a
/// surface echoes back to the voter).
pub fn record_feedback(sink: &dyn FeedbackSink, input: &FeedbackInput) -> Result<FeedbackRecord> {
    input.validate()?;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let record = FeedbackRecord {
        feedback_id: new_query_id(),
        query_id: input.query_id.clone(),
        chunk_id: input.chunk_id,
        vote: input.vote,
        comment: input.comment.clone(),
        requester_kind: input.requester_kind_or_anonymous().to_string(),
        requester_id: input.requester_id.clone(),
        ts: rfc3339_utc(now_ms),
        opened: input.opened,
    };
    sink.write_feedback(&record)?;
    Ok(record)
}

/// Load raw records from JSONL feedback files, in file order.
/// Malformed lines are skipped (they vote on nothing and refer
/// to nothing); a missing/unreadable FILE is an error, matching
/// `load_feedback_votes`.
pub fn load_feedback_records(paths: &[PathBuf]) -> Result<Vec<FeedbackRecord>> {
    let mut records = Vec::new();
    for path in paths {
        let body = std::fs::read_to_string(path).map_err(|e| {
            Error::backend(format!(
                "feedback store: cannot read feedback file {}: {e}",
                path.display()
            ))
        })?;
        for line in body.lines() {
            let Ok(rec) = serde_json::from_str::<Json>(line.trim()) else {
                continue;
            };
            if let Some(r) = FeedbackRecord::from_json(&rec) {
                records.push(r);
            }
        }
    }
    Ok(records)
}

/// Every feedback file in a store directory, in name order
/// (which is day order for the sink's file names).
pub fn feedback_files(dir: &PathBuf) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("atlas-feedback-") && n.ends_with(".jsonl"))
        })
        .collect();
    files.sort();
    files
}

/// Load the whole raw store in a directory (empty when the
/// directory does not exist yet — no votes cast is a state, not
/// an error).
pub fn load_feedback_dir(dir: &PathBuf) -> Result<Vec<FeedbackRecord>> {
    load_feedback_records(&feedback_files(dir))
}

/// One record with its derived §6.4 supersede state.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedbackView {
    pub record: FeedbackRecord,
    /// True when this is the latest record for its
    /// `(requester, query_id, chunk_id)` key.
    pub active: bool,
    /// The `feedback_id` of the record that superseded this one,
    /// when it is not active.
    pub superseded_by: Option<String>,
}

impl FeedbackView {
    pub fn to_json(&self) -> Json {
        let mut out = self.record.to_json();
        if let Json::Object(map) = &mut out {
            map.insert("active".into(), Json::Bool(self.active));
            map.insert(
                "superseded_by".into(),
                self.superseded_by
                    .as_ref()
                    .map_or(Json::Null, |id| Json::String(id.clone())),
            );
        }
        out
    }
}

/// Derive the active/superseded view over raw records (§6.4):
/// for each active key the LAST record in store order is the
/// active vote; every earlier record for the key is superseded
/// by that active record. Raw records are returned unmodified
/// and in store order.
pub fn active_view(records: &[FeedbackRecord]) -> Vec<FeedbackView> {
    let mut latest: std::collections::HashMap<(String, String, String, Option<i64>), usize> =
        std::collections::HashMap::new();
    for (i, r) in records.iter().enumerate() {
        latest.insert(r.active_key(), i);
    }
    records
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let winner = latest[&r.active_key()];
            if winner == i {
                FeedbackView {
                    record: r.clone(),
                    active: true,
                    superseded_by: None,
                }
            } else {
                FeedbackView {
                    record: r.clone(),
                    active: false,
                    superseded_by: Some(records[winner].feedback_id.clone()),
                }
            }
        })
        .collect()
}

/// Retrieve the feedback for one query, optionally narrowed to
/// one result: every raw record for `(query_id, chunk_id)` with
/// its supersede state — the acceptance retrieval surface. A
/// `chunk_id` of `None` returns the query's whole set, including
/// its per-query (`chunk_id: null`) records.
pub fn feedback_for(
    records: &[FeedbackRecord],
    query_id: &str,
    chunk_id: Option<i64>,
) -> Vec<FeedbackView> {
    active_view(records)
        .into_iter()
        .filter(|v| v.record.query_id == query_id)
        .filter(|v| chunk_id.is_none() || v.record.chunk_id == chunk_id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Oracle for every test in this module: design section 6
    // (capture §6.1, identity §6.3, anti-gaming §6.4) and
    // invariants 3-4 of the #6027 design doc.

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("atlas-feedback-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn input(query: &str, chunk: Option<i64>, vote: Vote) -> FeedbackInput {
        FeedbackInput {
            query_id: query.to_string(),
            chunk_id: chunk,
            vote,
            comment: None,
            requester_kind: "user".to_string(),
            requester_id: "tester".to_string(),
            opened: true,
        }
    }

    #[test]
    fn cast_and_retrieve_by_query_and_result_ids() {
        let dir = temp_dir("cast");
        let sink = JsonlFeedbackSink::new(dir.clone());
        let mut cast = input("q-1", Some(42), Vote::Up);
        cast.comment = Some("exactly the chunk I needed".to_string());
        let rec = record_feedback(&sink, &cast).expect("cast");
        assert!(!rec.feedback_id.is_empty());
        assert_eq!(rec.vote, Vote::Up);
        assert_eq!(rec.comment.as_deref(), Some("exactly the chunk I needed"));

        let records = load_feedback_dir(&dir).expect("load");
        assert_eq!(records.len(), 1);
        let found = feedback_for(&records, "q-1", Some(42));
        assert_eq!(found.len(), 1);
        assert!(found[0].active);
        assert_eq!(found[0].record.feedback_id, rec.feedback_id);
        // Retrieval by query alone finds it too; another query does not.
        assert_eq!(feedback_for(&records, "q-1", None).len(), 1);
        assert!(feedback_for(&records, "q-other", None).is_empty());
        // The stored shape is the one the stats loader reads.
        let votes =
            crate::trace_stats::load_feedback_votes(&feedback_files(&dir)).expect("stats loader");
        assert_eq!(votes.len(), 1);
        assert!(votes[0].up);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unopened_down_vote_is_stored_with_opened_false() {
        let dir = temp_dir("unopened");
        let sink = JsonlFeedbackSink::new(dir.clone());
        let mut cast = input("q-2", Some(7), Vote::Down);
        cast.opened = false;
        record_feedback(&sink, &cast).expect("cast");
        let records = load_feedback_dir(&dir).expect("load");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].vote, Vote::Down);
        assert!(!records[0].opened, "unopened down-vote keeps opened=false");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn revote_supersedes_and_the_raw_record_is_preserved() {
        let dir = temp_dir("revote");
        let sink = JsonlFeedbackSink::new(dir.clone());
        let first = record_feedback(&sink, &input("q-3", Some(9), Vote::Down)).expect("first");
        let second = record_feedback(&sink, &input("q-3", Some(9), Vote::Up)).expect("second");
        // A different requester on the same result is a different key.
        let mut other = input("q-3", Some(9), Vote::Down);
        other.requester_id = "someone-else".to_string();
        let third = record_feedback(&sink, &other).expect("third");

        let records = load_feedback_dir(&dir).expect("load");
        assert_eq!(records.len(), 3, "raw store keeps every record verbatim");
        let view = feedback_for(&records, "q-3", Some(9));
        let by_id: std::collections::HashMap<&str, &FeedbackView> = view
            .iter()
            .map(|v| (v.record.feedback_id.as_str(), v))
            .collect();
        assert!(!by_id[first.feedback_id.as_str()].active);
        assert_eq!(
            by_id[first.feedback_id.as_str()].superseded_by.as_deref(),
            Some(second.feedback_id.as_str())
        );
        assert!(by_id[second.feedback_id.as_str()].active);
        assert!(by_id[third.feedback_id.as_str()].active);
        // The active set is exactly one vote per (requester, query, chunk).
        assert_eq!(view.iter().filter(|v| v.active).count(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn per_query_vote_stores_null_chunk() {
        let dir = temp_dir("perquery");
        let sink = JsonlFeedbackSink::new(dir.clone());
        record_feedback(&sink, &input("q-4", None, Vote::Down)).expect("cast");
        let records = load_feedback_dir(&dir).expect("load");
        assert_eq!(records[0].chunk_id, None);
        assert_eq!(feedback_for(&records, "q-4", None).len(), 1);
        assert!(feedback_for(&records, "q-4", Some(1)).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn comment_up_to_and_beyond_the_design_minimum() {
        let dir = temp_dir("comment");
        let sink = JsonlFeedbackSink::new(dir.clone());
        // 64 KiB — the design's minimum supported essay size — is accepted.
        let mut cast = input("q-5", Some(1), Vote::Up);
        cast.comment = Some("x".repeat(64 * 1024));
        record_feedback(&sink, &cast).expect("64 KiB comment");
        // Beyond the capture ceiling is a validation error, not a truncation.
        let mut big = input("q-5", Some(1), Vote::Up);
        big.comment = Some("x".repeat(MAX_COMMENT_BYTES + 1));
        assert!(record_feedback(&sink, &big).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validation_rejects_empty_query_and_unknown_requester_kind() {
        let dir = temp_dir("validation");
        let sink = JsonlFeedbackSink::new(dir.clone());
        assert!(record_feedback(&sink, &input("", Some(1), Vote::Up)).is_err());
        let mut bad = input("q-6", Some(1), Vote::Up);
        bad.requester_kind = "robot".to_string();
        assert!(record_feedback(&sink, &bad).is_err());
        assert!(Vote::parse("sideways").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
