//! Trace statistics aggregation + weekly report — statistics are
//! computed from traces (issue #9975, follow-on 3 of the #6027
//! design, doc 2026-11-10T20-41-01Z_design_atlas_retrieval_stats_
//! feedback.md sections 5 and 7).
//!
//! Section 5 has four aggregates, and this module is where each
//! lands — all of them computed by a batch pass over stored
//! `atlas-query-trace/v1` records, never from a parallel
//! instrumentation path (invariant 5):
//!
//! 1. Shard-stage survival, per `(source_file_id, embed_id)` and
//!    split by branch (FTS vs vector): pool entries, shard top-K
//!    entries (K is the trace's own `parameters.top_k`), and
//!    fusion survivors, with the (b/a) and (c/a) survival rates.
//! 2. Final-rank distribution over served entries: rank 1, rank 2,
//!    rank 3, ranks 1-5, and served-below-5 — the distribution,
//!    never a mean rank.
//! 3. Exposure: entries that entered a pool but were never served
//!    (the trace's `final.served: false` population).
//! 4. Feedback aggregates, joined from feedback records on
//!    `chunk_id` and weighted per design section 6.3 (drew 1.0,
//!    other users 1.0, agents 0.25, anonymous 0.1). They are
//!    reported alongside 1-3, never merged into them. Feedback
//!    capture itself is follow-on 4, so in the common case today
//!    this section is empty — the join, the weights, and the
//!    section 6.4 unopened-down-vote split (snippet signal, not
//!    content quality) are implemented here so the report shape
//!    does not change when capture lands.
//!
//! The weekly report (section 7) is itself an artifact in the
//! "statistics and reports" source class: `write_report` lands
//! one `atlas-weekly-statistics-report/v1` JSONL record in the
//! traces directory under the report source registration, and the
//! CLI renders the same record as markdown. Every aggregate the
//! report carries names the section 7 decision it feeds
//! (invariant 6); the report's `unconsumed_aggregates` list is
//! where an aggregate with no consumer would be named before
//! removal — by construction it is empty today, and the report
//! says so rather than omitting the field.
//!
//! Nominations are nominations only (the issue's scope excludes
//! ranking behaviour changes and automated deletion or weight
//! changes): protected embeds (sustained rank 1-3 presence),
//! retention candidates (sustained exposure, never served),
//! ranking-change candidates (sustained served-below-5 share),
//! and material branch-survival skew shifts between the 7-day
//! and 28-day windows. Thresholds are named constants below,
//! recorded in each nomination, not buried magic numbers.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use serde_json::{Value as Json, json};

use crate::error::{Error, Result};
use crate::trace_ingest::{
    HOT_RETENTION_DAYS, TRACE_SOURCE_CLASS, TraceSourceRegistration, days_from_civil,
    record_included, trace_file_day, valid_civil_date,
};

/// Schema of one window aggregation record.
pub const STATS_SCHEMA: &str = "atlas-trace-statistics/v1";

/// Schema of the weekly statistics report record.
pub const REPORT_SCHEMA: &str = "atlas-weekly-statistics-report/v1";

/// Stable id of the statistics-reports source in the ingestion
/// source set (same source class as the traces source, design
/// section 4/7: the report is an artifact of that class).
pub const REPORT_SOURCE_ID: &str = "atlas-statistics-reports";

/// Rolling windows the daily batch computes (design section 5).
pub const DEFAULT_WINDOWS: [i64; 2] = [7, 28];

/// Fallback shard top-K when a trace record carries no usable
/// `parameters.top_k` (the trace writer always writes one; this
/// only guards hand-built or future records).
pub const DEFAULT_TOP_K: u64 = 10;

/// A branch survival-rate shift at or above this between the
/// 7-day and 28-day windows is "material" for section 7.
pub const SKEW_SHIFT_THRESHOLD: f64 = 0.20;

/// Protected-embed nomination: at least this many served entries
/// in the 7-day window, with at least this share at ranks 1-3.
pub const PROTECTED_MIN_SERVED: u64 = 3;
pub const PROTECTED_TOP3_SHARE: f64 = 0.5;

/// Retention-candidate nomination: at least this many pool-but-
/// never-served entries across the 28-day window and no serves.
pub const RETENTION_MIN_EXPOSURE: u64 = 3;

/// Ranking-change-candidate nomination: at least this many served
/// entries in the 7-day window, with at least this share below 5.
pub const RANKING_MIN_SERVED: u64 = 3;
pub const RANKING_BELOW5_SHARE: f64 = 0.5;

/// The statistics-reports source registration (design sections
/// 4 and 7): the weekly report's registry row, mirroring the
/// traces source registration in `trace_ingest`.
pub fn report_source_registration() -> TraceSourceRegistration {
    TraceSourceRegistration {
        source_id: REPORT_SOURCE_ID,
        source_class: TRACE_SOURCE_CLASS,
        record_schema: REPORT_SCHEMA,
        file_pattern: "atlas-statistics-report-YYYY-MM-DD.jsonl",
        hot_retention_days: HOT_RETENTION_DAYS,
    }
}

/// Inverse of `days_from_civil` (Howard Hinnant's algorithm):
/// a day count since 1970-01-01 back to a civil `(y, m, d)`.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Format a civil date as `YYYY-MM-DD`.
pub fn format_day(y: i64, m: u32, d: u32) -> String {
    format!("{y:04}-{m:02}-{d:02}")
}

fn parse_day(s: &str) -> Option<(i64, u32, u32)> {
    let mut parts = s.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !valid_civil_date(y, m, d) {
        return None;
    }
    Some((y, m, d))
}

fn rate(num: u64, den: u64) -> f64 {
    if den == 0 {
        0.0
    } else {
        num as f64 / den as f64
    }
}

/// Per-embed aggregates for one window (design section 5.1-5.3).
#[derive(Debug, Clone, Default)]
pub struct EmbedStats {
    pub source_file_id: String,
    pub embed_id: String,
    /// The embed content hash first seen in the window: statistics
    /// describe the embed state that was actually scored.
    pub embed_content_hash: String,
    pub pool_entries: u64,
    pub shard_top_k: u64,
    pub survived_fusion: u64,
    pub served: u64,
    pub exposure_entries: u64,
    pub rank1: u64,
    pub rank2: u64,
    pub rank3: u64,
    pub rank_top5: u64,
    pub rank_below5: u64,
    pub pool_queries: BTreeSet<String>,
    pub served_queries: BTreeSet<String>,
}

impl EmbedStats {
    fn to_json(&self) -> Json {
        json!({
            "source_file_id": self.source_file_id,
            "embed_id": self.embed_id,
            "embed_content_hash": self.embed_content_hash,
            "pool_entries": self.pool_entries,
            "shard_top_k": self.shard_top_k,
            "shard_top_k_rate": rate(self.shard_top_k, self.pool_entries),
            "survived_fusion": self.survived_fusion,
            "fusion_survival_rate": rate(self.survived_fusion, self.pool_entries),
            "served": self.served,
            "exposure_entries": self.exposure_entries,
            "final_rank": {
                "rank1": self.rank1,
                "rank2": self.rank2,
                "rank3": self.rank3,
                "rank_top5": self.rank_top5,
                "rank_below5": self.rank_below5,
            },
            "pool_queries": self.pool_queries.len(),
            "served_queries": self.served_queries.len(),
        })
    }
}

/// Per-branch survival for one window (design section 5.1 split).
#[derive(Debug, Clone, Default)]
pub struct BranchStats {
    pub branch: String,
    pub pool_entries: u64,
    pub shard_top_k: u64,
    pub survived_fusion: u64,
}

impl BranchStats {
    fn survival_rate(&self) -> f64 {
        rate(self.survived_fusion, self.pool_entries)
    }
    fn to_json(&self) -> Json {
        json!({
            "branch": self.branch,
            "pool_entries": self.pool_entries,
            "shard_top_k": self.shard_top_k,
            "survived_fusion": self.survived_fusion,
            "fusion_survival_rate": self.survival_rate(),
        })
    }
}

/// Per-embed feedback aggregates (design section 5.4 + 6.3/6.4),
/// reported alongside — never merged into — retrieval statistics.
#[derive(Debug, Clone, Default)]
pub struct FeedbackStats {
    pub source_file_id: String,
    pub embed_id: String,
    pub up_votes: u64,
    pub down_votes: u64,
    pub weighted_up: f64,
    pub weighted_down: f64,
    /// Down-votes on unopened results: snippet signal, excluded
    /// from the content-quality counts above (section 6.4).
    pub snippet_down_votes: u64,
    pub unique_requesters: BTreeSet<String>,
}

impl FeedbackStats {
    fn to_json(&self) -> Json {
        json!({
            "source_file_id": self.source_file_id,
            "embed_id": self.embed_id,
            "up_votes": self.up_votes,
            "down_votes": self.down_votes,
            "weighted_up": self.weighted_up,
            "weighted_down": self.weighted_down,
            "weighted_thumbs_ratio": if self.weighted_up + self.weighted_down > 0.0 {
                self.weighted_up / (self.weighted_up + self.weighted_down)
            } else { 0.0 },
            "snippet_down_votes": self.snippet_down_votes,
            "unique_requesters": self.unique_requesters.len(),
        })
    }
}

/// One feedback record, at the seam follow-on 4's capture will
/// produce: a vote on `(query_id, chunk_id)`.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedbackVote {
    pub query_id: String,
    pub chunk_id: i64,
    pub up: bool,
    pub opened: bool,
    pub requester_kind: String,
    pub requester_id: String,
}

/// Aggregation weight for a requester kind (design section 6.3):
/// Drew and other human users 1.0, agents 0.25, anonymous 0.1.
pub fn requester_weight(kind: &str) -> f64 {
    match kind {
        "drew" | "user" => 1.0,
        "agent" => 0.25,
        _ => 0.1,
    }
}

/// Load feedback votes from JSONL files of feedback-shaped
/// records (the section 6.1 record shape): `query_id`,
/// `chunk_id`, `vote: "up"|"down"`, optional `opened`, and a
/// `requester {kind, id}` object. Malformed lines are ignored —
/// a feedback file must never crash the statistics batch, and an
/// unparseable line votes on nothing.
pub fn load_feedback_votes(paths: &[PathBuf]) -> Result<Vec<FeedbackVote>> {
    let mut votes = Vec::new();
    for path in paths {
        let body = std::fs::read_to_string(path).map_err(|e| {
            Error::backend(format!(
                "trace stats: cannot read feedback file {}: {e}",
                path.display()
            ))
        })?;
        for line in body.lines() {
            let Ok(rec) = serde_json::from_str::<Json>(line.trim()) else {
                continue;
            };
            let (Some(query_id), Some(chunk_id), Some(vote)) = (
                rec.get("query_id").and_then(Json::as_str),
                rec.get("chunk_id").and_then(Json::as_i64),
                rec.get("vote").and_then(Json::as_str),
            ) else {
                continue;
            };
            let up = match vote {
                "up" => true,
                "down" => false,
                _ => continue,
            };
            let (kind, id) = match rec.get("requester") {
                Some(r) => (
                    r.get("kind")
                        .and_then(Json::as_str)
                        .unwrap_or("")
                        .to_string(),
                    r.get("id").and_then(Json::as_str).unwrap_or("").to_string(),
                ),
                None => (String::new(), String::new()),
            };
            votes.push(FeedbackVote {
                query_id: query_id.to_string(),
                chunk_id,
                up,
                opened: rec.get("opened").and_then(Json::as_bool).unwrap_or(true),
                requester_kind: kind,
                requester_id: id,
            });
        }
    }
    Ok(votes)
}

/// The aggregates for one rolling window (design section 5).
#[derive(Debug, Clone)]
pub struct WindowStats {
    pub window_days: i64,
    pub start_day: String,
    pub end_day: String,
    pub traces: u64,
    pub skipped_lines: usize,
    pub embeds: Vec<EmbedStats>,
    pub branches: Vec<BranchStats>,
    pub feedback: Vec<FeedbackStats>,
    /// Feedback votes that named a `(query_id, chunk_id)` pair no
    /// trace in the window resolves to an embed.
    pub feedback_unjoined: u64,
}

impl WindowStats {
    pub fn to_json(&self) -> Json {
        json!({
            "schema": STATS_SCHEMA,
            "window_days": self.window_days,
            "start_day": self.start_day,
            "end_day": self.end_day,
            "traces": self.traces,
            "skipped_lines": self.skipped_lines,
            "embeds": self.embeds.iter().map(EmbedStats::to_json).collect::<Vec<_>>(),
            "branches": self.branches.iter().map(BranchStats::to_json).collect::<Vec<_>>(),
            "feedback": self.feedback.iter().map(FeedbackStats::to_json).collect::<Vec<_>>(),
            "feedback_unjoined": self.feedback_unjoined,
        })
    }
}

fn hot_trace_files(dir: &Path) -> Result<Vec<(PathBuf, (i64, u32, u32))>> {
    let mut files = Vec::new();
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(files),
        Err(e) => {
            return Err(Error::backend(format!(
                "trace stats: cannot list {}: {e}",
                dir.display()
            )));
        }
    };
    for ent in rd.flatten() {
        let name = ent.file_name().to_string_lossy().into_owned();
        if let Some(day) = trace_file_day(&name) {
            files.push((ent.path(), day));
        }
    }
    files.sort();
    Ok(files)
}

/// Run the daily batch aggregation over the stored traces in
/// `dir` for the rolling `window_days` window ending at `today`
/// (inclusive). Recomputed from raw traces on every run —
/// replayable by construction, never patched in place (section
/// 5). Cold-tier files are not read: retention already compacted
/// them, and the hot window never reaches past the 180-day hot
/// retention for the 7/28-day windows this batch computes.
pub fn aggregate_traces(
    dir: &Path,
    today: (i64, u32, u32),
    window_days: i64,
    feedback: &[FeedbackVote],
) -> Result<WindowStats> {
    if window_days < 1 {
        return Err(Error::backend(format!(
            "trace stats: window_days must be >= 1, got {window_days}"
        )));
    }
    let end = days_from_civil(today.0, today.1, today.2);
    let start = end - (window_days - 1);
    let (sy, sm, sd) = civil_from_days(start);
    let mut embeds: BTreeMap<(String, String), EmbedStats> = BTreeMap::new();
    let mut branches: BTreeMap<String, BranchStats> = BTreeMap::new();
    // (query_id, chunk_id) -> embed key, for the feedback join.
    let mut chunk_embed: HashMap<(String, i64), (String, String)> = HashMap::new();
    let mut traces = 0u64;
    let mut skipped_lines = 0usize;
    for (path, file_day) in hot_trace_files(dir)? {
        let file_days = days_from_civil(file_day.0, file_day.1, file_day.2);
        if file_days < start || file_days > end {
            continue;
        }
        let body = std::fs::read_to_string(&path).map_err(|e| {
            Error::backend(format!("trace stats: cannot read {}: {e}", path.display()))
        })?;
        for line in body.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(rec) = serde_json::from_str::<Json>(line) else {
                skipped_lines += 1;
                continue;
            };
            if !record_included(&rec) {
                skipped_lines += 1;
                continue;
            }
            // Window membership is by the record's own day; the
            // file day is the fallback for a record without a
            // parseable timestamp.
            let rec_day = rec
                .get("ts")
                .and_then(Json::as_str)
                .and_then(|ts| parse_day(&ts[..ts.len().min(10)]))
                .unwrap_or(file_day);
            let rec_days = days_from_civil(rec_day.0, rec_day.1, rec_day.2);
            if rec_days < start || rec_days > end {
                continue;
            }
            traces += 1;
            let query_id = rec
                .get("query_id")
                .and_then(Json::as_str)
                .unwrap_or("")
                .to_string();
            let top_k = rec
                .get("parameters")
                .and_then(|p| p.get("top_k"))
                .and_then(Json::as_u64)
                .filter(|k| *k >= 1)
                .unwrap_or(DEFAULT_TOP_K);
            let Some(results) = rec.get("results").and_then(Json::as_array) else {
                continue;
            };
            for r in results {
                let source_file_id = r
                    .get("source_file_id")
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_string();
                let embed_id = r
                    .get("embed_id")
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_string();
                if let Some(chunk_id) = r.get("chunk_id").and_then(Json::as_i64) {
                    chunk_embed.insert(
                        (query_id.clone(), chunk_id),
                        (source_file_id.clone(), embed_id.clone()),
                    );
                }
                let key = (source_file_id.clone(), embed_id.clone());
                let e = embeds.entry(key).or_insert_with(|| EmbedStats {
                    source_file_id: source_file_id.clone(),
                    embed_id: embed_id.clone(),
                    ..Default::default()
                });
                if e.embed_content_hash.is_empty() {
                    if let Some(h) = r.get("embed_content_hash").and_then(Json::as_str) {
                        e.embed_content_hash = h.to_string();
                    }
                }
                e.pool_entries += 1;
                e.pool_queries.insert(query_id.clone());
                let stages = r.get("stages");
                let shard = stages.and_then(|s| s.get("shard"));
                let branch = shard
                    .and_then(|s| s.get("branch"))
                    .and_then(Json::as_str)
                    .unwrap_or("unknown")
                    .to_string();
                let in_top_k = shard
                    .and_then(|s| s.get("rank_in_shard"))
                    .and_then(Json::as_u64)
                    .is_some_and(|rank| rank >= 1 && rank <= top_k);
                if in_top_k {
                    e.shard_top_k += 1;
                }
                let survived = stages
                    .and_then(|s| s.get("merge"))
                    .and_then(|m| m.get("rank_after_fusion"))
                    .is_some_and(|v| !v.is_null());
                if survived {
                    e.survived_fusion += 1;
                }
                let b = branches
                    .entry(branch.clone())
                    .or_insert_with(|| BranchStats {
                        branch: branch.clone(),
                        ..Default::default()
                    });
                b.pool_entries += 1;
                if in_top_k {
                    b.shard_top_k += 1;
                }
                if survived {
                    b.survived_fusion += 1;
                }
                let fin = stages.and_then(|s| s.get("final"));
                let served = fin
                    .and_then(|f| f.get("served"))
                    .and_then(Json::as_bool)
                    .unwrap_or(false);
                if served {
                    e.served += 1;
                    e.served_queries.insert(query_id.clone());
                    if let Some(rank) = fin.and_then(|f| f.get("rank")).and_then(Json::as_u64) {
                        match rank {
                            1 => e.rank1 += 1,
                            2 => e.rank2 += 1,
                            3 => e.rank3 += 1,
                            _ => {}
                        }
                        if (1..=5).contains(&rank) {
                            e.rank_top5 += 1;
                        } else if rank > 5 {
                            e.rank_below5 += 1;
                        }
                    }
                } else {
                    e.exposure_entries += 1;
                }
            }
        }
    }
    // Section 5.4: feedback joined on (query_id, chunk_id),
    // weighted per section 6.3, alongside — never merged.
    let mut fb: BTreeMap<(String, String), FeedbackStats> = BTreeMap::new();
    let mut feedback_unjoined = 0u64;
    // One active vote per (requester, query_id, chunk_id)
    // (section 6.4 duplicates rule): re-voting supersedes, so
    // only the last occurrence of each key is counted. Anonymous
    // votes carry no requester id and cannot be deduplicated —
    // each is counted (at the 0.1 anonymous weight).
    let mut last_vote: HashMap<(String, String, i64), usize> = HashMap::new();
    for (i, v) in feedback.iter().enumerate() {
        if !v.requester_id.is_empty() {
            last_vote.insert((v.requester_id.clone(), v.query_id.clone(), v.chunk_id), i);
        }
    }
    for (i, v) in feedback.iter().enumerate() {
        if !v.requester_id.is_empty()
            && last_vote.get(&(v.requester_id.clone(), v.query_id.clone(), v.chunk_id)) != Some(&i)
        {
            continue; // superseded by a later re-vote
        }
        let Some(key) = chunk_embed.get(&(v.query_id.clone(), v.chunk_id)) else {
            feedback_unjoined += 1;
            continue;
        };
        let f = fb.entry(key.clone()).or_insert_with(|| FeedbackStats {
            source_file_id: key.0.clone(),
            embed_id: key.1.clone(),
            ..Default::default()
        });
        let w = requester_weight(&v.requester_kind);
        if v.up {
            f.up_votes += 1;
            f.weighted_up += w;
        } else if !v.opened {
            f.snippet_down_votes += 1;
        } else {
            f.down_votes += 1;
            f.weighted_down += w;
        }
        if !v.requester_id.is_empty() {
            f.unique_requesters.insert(v.requester_id.clone());
        }
    }
    Ok(WindowStats {
        window_days,
        start_day: format_day(sy, sm, sd),
        end_day: format_day(today.0, today.1, today.2),
        traces,
        skipped_lines,
        embeds: embeds.into_values().collect(),
        branches: branches.into_values().collect(),
        feedback: fb.into_values().collect(),
        feedback_unjoined,
    })
}

/// The aggregates this fleet produces and the section 7 decision
/// each one feeds (invariant 6: no signal without a decision).
pub fn aggregate_consumers() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "shard_stage_survival",
            "search patterns: pool sizes, fusion weights, expansion policy (eval-gated)",
        ),
        (
            "final_rank_distribution",
            "retention: protected embeds; ranking: change candidates (eval-gated)",
        ),
        ("exposure", "retention review and ranking investigations"),
        (
            "feedback_aggregates",
            "retention + re-ingestion nominations (decision wiring is follow-on 5)",
        ),
        (
            "branch_survival_skew",
            "search patterns: pool sizes, fusion weights (eval-gated)",
        ),
    ]
}

/// One section 7 nomination in the weekly report.
#[derive(Debug, Clone)]
pub struct Nomination {
    pub kind: &'static str,
    pub subject: String,
    pub aggregate: &'static str,
    pub window: String,
    pub detail: String,
}

impl Nomination {
    fn to_json(&self) -> Json {
        json!({
            "kind": self.kind,
            "subject": self.subject,
            "aggregate": self.aggregate,
            "window": self.window,
            "detail": self.detail,
        })
    }
}

/// The weekly statistics report (design section 7): the 7-day
/// and 28-day aggregates plus the nominations they carry.
#[derive(Debug, Clone)]
pub struct WeeklyReport {
    pub report_day: String,
    pub windows: Vec<WindowStats>,
    pub nominations: Vec<Nomination>,
}

impl WeeklyReport {
    pub fn to_json(&self) -> Json {
        let consumers: Vec<Json> = aggregate_consumers()
            .into_iter()
            .map(|(aggregate, decision)| json!({"aggregate": aggregate, "decision": decision}))
            .collect();
        json!({
            "schema": REPORT_SCHEMA,
            "source_id": REPORT_SOURCE_ID,
            "source_class": TRACE_SOURCE_CLASS,
            "report_day": self.report_day,
            "windows": self.windows.iter().map(WindowStats::to_json).collect::<Vec<_>>(),
            "nominations": self.nominations.iter().map(Nomination::to_json).collect::<Vec<_>>(),
            "aggregate_consumers": consumers,
            // Invariant 6: an aggregate with no consumer after two
            // reporting cycles is named here before removal. Every
            // aggregate above names its decision, so this is empty
            // — stated, not omitted.
            "unconsumed_aggregates": [],
            "notes": [
                "Nominations only: no ranking behaviour change and no automated deletion or weight change is made by this report (issue #9975 scope).",
                "Feedback aggregates are reported alongside retrieval statistics, never merged into them (design section 5.4); feedback capture is follow-on 4, so an empty feedback section means no votes were supplied, not zero signal quality.",
            ],
        })
    }

    /// Human rendering of the same record the JSONL artifact
    /// carries — one report, two renderings, no divergent numbers.
    pub fn render_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "# Atlas weekly statistics report — {}\n\n",
            self.report_day
        ));
        for w in &self.windows {
            out.push_str(&format!(
                "## Window: {} days ({} to {})\n\nTraces: {} ({} skipped lines)\n\n",
                w.window_days, w.start_day, w.end_day, w.traces, w.skipped_lines
            ));
            out.push_str("| embed | pool | top-K rate | fusion survival | served | exposure | rank 1 | rank 2 | rank 3 | top 1-5 | below 5 |\n|---|---|---|---|---|---|---|---|---|---|---|\n");
            for e in &w.embeds {
                out.push_str(&format!(
                    "| {} / {} | {} | {:.2} | {:.2} | {} | {} | {} | {} | {} | {} | {} |\n",
                    e.source_file_id,
                    e.embed_id,
                    e.pool_entries,
                    rate(e.shard_top_k, e.pool_entries),
                    rate(e.survived_fusion, e.pool_entries),
                    e.served,
                    e.exposure_entries,
                    e.rank1,
                    e.rank2,
                    e.rank3,
                    e.rank_top5,
                    e.rank_below5,
                ));
            }
            out.push_str("\n| branch | pool | fusion survival rate |\n|---|---|---|\n");
            for b in &w.branches {
                out.push_str(&format!(
                    "| {} | {} | {:.2} |\n",
                    b.branch,
                    b.pool_entries,
                    b.survival_rate()
                ));
            }
            out.push('\n');
        }
        out.push_str("## Nominations (section 7)\n\n");
        if self.nominations.is_empty() {
            out.push_str("None this cycle.\n");
        }
        for n in &self.nominations {
            out.push_str(&format!(
                "- **{}** — {} (aggregate `{}`, window {}): {}\n",
                n.kind, n.subject, n.aggregate, n.window, n.detail
            ));
        }
        out.push_str("\n## Aggregate consumers (invariant 6)\n\n");
        for (aggregate, decision) in aggregate_consumers() {
            out.push_str(&format!("- `{aggregate}` → {decision}\n"));
        }
        out.push_str("\nUnconsumed aggregates: none.\n");
        out
    }
}

fn subject(e: &EmbedStats) -> String {
    format!("{}/{}", e.source_file_id, e.embed_id)
}

/// Build the weekly report for `today`: both rolling windows
/// plus the section 7 nominations derived from them.
pub fn weekly_report(
    dir: &Path,
    today: (i64, u32, u32),
    feedback: &[FeedbackVote],
) -> Result<WeeklyReport> {
    let w7 = aggregate_traces(dir, today, DEFAULT_WINDOWS[0], feedback)?;
    let w28 = aggregate_traces(dir, today, DEFAULT_WINDOWS[1], feedback)?;
    let mut nominations = Vec::new();
    for e in &w7.embeds {
        let top3 = e.rank1 + e.rank2 + e.rank3;
        if e.served >= PROTECTED_MIN_SERVED && rate(top3, e.served) >= PROTECTED_TOP3_SHARE {
            nominations.push(Nomination {
                kind: "protected_embed",
                subject: subject(e),
                aggregate: "final_rank_distribution",
                window: format!("{}..{}", w7.start_day, w7.end_day),
                detail: format!(
                    "served {} times with {} at ranks 1-3 (share {:.2} >= {PROTECTED_TOP3_SHARE}); protected from demotion triggers — demotion requires contradicting feedback evidence, not statistics alone",
                    e.served,
                    top3,
                    rate(top3, e.served)
                ),
            });
        }
        if e.served >= RANKING_MIN_SERVED && rate(e.rank_below5, e.served) >= RANKING_BELOW5_SHARE {
            nominations.push(Nomination {
                kind: "ranking_change_candidate",
                subject: subject(e),
                aggregate: "final_rank_distribution",
                window: format!("{}..{}", w7.start_day, w7.end_day),
                detail: format!(
                    "served {} times with {} below rank 5 (share {:.2} >= {RANKING_BELOW5_SHARE}); ranking-change candidate — production statistics nominate, the offline eval harness disposes",
                    e.served,
                    e.rank_below5,
                    rate(e.rank_below5, e.served)
                ),
            });
        }
    }
    for e in &w28.embeds {
        if e.served == 0 && e.exposure_entries >= RETENTION_MIN_EXPOSURE {
            nominations.push(Nomination {
                kind: "retention_candidate",
                subject: subject(e),
                aggregate: "exposure",
                window: format!("{}..{}", w28.start_day, w28.end_day),
                detail: format!(
                    "entered a pool {} times over 28 days and was never served; retention review candidate (no automated deletion — nomination only)",
                    e.exposure_entries
                ),
            });
        }
    }
    let b7: BTreeMap<&str, &BranchStats> =
        w7.branches.iter().map(|b| (b.branch.as_str(), b)).collect();
    for b28 in &w28.branches {
        if let Some(b7) = b7.get(b28.branch.as_str()) {
            let shift = (b7.survival_rate() - b28.survival_rate()).abs();
            if shift >= SKEW_SHIFT_THRESHOLD && b28.pool_entries > 0 && b7.pool_entries > 0 {
                nominations.push(Nomination {
                    kind: "branch_skew_shift",
                    subject: format!("branch {}", b28.branch),
                    aggregate: "branch_survival_skew",
                    window: format!(
                        "7d {}..{} vs 28d {}..{}",
                        w7.start_day, w7.end_day, w28.start_day, w28.end_day
                    ),
                    detail: format!(
                        "fusion survival {:.2} (7d) vs {:.2} (28d), shift {:.2} >= {SKEW_SHIFT_THRESHOLD}; search-patterns candidate (pool sizes, fusion weights) via the eval-gated path",
                        b7.survival_rate(),
                        b28.survival_rate(),
                        shift
                    ),
                });
            }
        }
    }
    Ok(WeeklyReport {
        report_day: format_day(today.0, today.1, today.2),
        windows: vec![w7, w28],
        nominations,
    })
}

/// The report artifact's file name for a report day.
pub fn report_file_name(report_day: &str) -> String {
    format!("atlas-statistics-report-{report_day}.jsonl")
}

/// Land the weekly report in the statistics-and-reports source
/// set: one JSONL record in the traces directory, beside the
/// traces it was computed from. Re-running for the same day
/// rewrites the same file — a report is recomputed from raw
/// traces, never appended to.
pub fn write_report(dir: &Path, report: &WeeklyReport) -> Result<PathBuf> {
    std::fs::create_dir_all(dir).map_err(|e| {
        Error::backend(format!("trace stats: cannot create {}: {e}", dir.display()))
    })?;
    let path = dir.join(report_file_name(&report.report_day));
    let body = format!("{}\n", report.to_json());
    std::fs::write(&path, body).map_err(|e| {
        Error::backend(format!("trace stats: cannot write {}: {e}", path.display()))
    })?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("atlas-trace-stats-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn result(
        chunk_id: i64,
        file: &str,
        embed: &str,
        branch: &str,
        shard_rank: u64,
        fusion_rank: Option<u64>,
        final_rank: Option<u64>,
    ) -> Json {
        json!({
            "chunk_id": chunk_id,
            "source_file_id": file,
            "embed_id": embed,
            "embed_content_hash": format!("hash-{embed}"),
            "stages": {
                "shard": {"shard_id": "s1", "branch": branch, "rank_in_shard": shard_rank, "score": 1.0},
                "merge": {"rank_after_fusion": fusion_rank, "fusion_score": 0.5},
                "rescore": {"applied": false, "kind": "none", "score": 0.0},
                "final": {"rank": final_rank, "served": final_rank.is_some()},
            },
        })
    }

    fn trace_line(query_id: &str, ts: &str, results: Vec<Json>) -> String {
        json!({
            "schema": "atlas-query-trace/v1",
            "query_id": query_id,
            "ts": ts,
            "duration_ms": 5,
            "surface": "cli",
            "requester": {"kind": "agent", "id": "test"},
            "query_text_hash": "h",
            "query_text": null,
            "parameters": {"pool": 20, "top_k": 2, "weights": {"fts": 1.0, "vector": 1.0}},
            "results": results,
            "pool_counts": {"merged": 2, "rescored": 2, "served": 1},
        })
        .to_string()
    }

    fn write_file(dir: &Path, name: &str, lines: &[String]) {
        std::fs::write(dir.join(name), lines.join("\n") + "\n").expect("write traces");
    }

    /// Fixture: q1 (2026-11-14) serves embed A at rank 1 via fts
    /// and leaves embed B unserved via vector; q2 (2026-11-13)
    /// serves A at rank 6 via vector.
    fn fixture(dir: &Path) {
        write_file(
            dir,
            "atlas-traces-2026-11-14.jsonl",
            &[trace_line(
                "q1",
                "2026-11-14T10:00:00Z",
                vec![
                    result(1, "f1", "A", "fts", 1, Some(1), Some(1)),
                    result(2, "f2", "B", "vector", 3, Some(2), None),
                ],
            )],
        );
        write_file(
            dir,
            "atlas-traces-2026-11-13.jsonl",
            &[trace_line(
                "q2",
                "2026-11-13T10:00:00Z",
                vec![result(1, "f1", "A", "vector", 1, Some(1), Some(6))],
            )],
        );
    }

    #[test]
    fn registration_is_statistics_and_reports() {
        let reg = report_source_registration();
        assert_eq!(reg.source_class, "statistics and reports");
        assert_eq!(reg.source_id, "atlas-statistics-reports");
        assert_eq!(reg.record_schema, REPORT_SCHEMA);
    }

    #[test]
    fn civil_round_trip() {
        for (y, m, d) in [(1970, 1, 1), (2026, 11, 14), (2024, 2, 29), (2000, 3, 1)] {
            assert_eq!(civil_from_days(days_from_civil(y, m, d)), (y, m, d));
        }
    }

    #[test]
    fn aggregation_counts_survival_ranks_and_exposure() {
        let dir = temp_dir("agg");
        fixture(&dir);
        let w = aggregate_traces(&dir, (2026, 11, 14), 7, &[]).expect("aggregate");
        assert_eq!(w.traces, 2);
        assert_eq!(w.start_day, "2026-11-08");
        assert_eq!(w.end_day, "2026-11-14");
        let a = w
            .embeds
            .iter()
            .find(|e| e.embed_id == "A")
            .expect("embed A");
        assert_eq!(a.pool_entries, 2);
        assert_eq!(a.shard_top_k, 2); // both shard ranks 1 <= top_k 2
        assert_eq!(a.survived_fusion, 2);
        assert_eq!(a.served, 2);
        assert_eq!(a.rank1, 1);
        assert_eq!(a.rank_top5, 1);
        assert_eq!(a.rank_below5, 1);
        assert_eq!(a.exposure_entries, 0);
        let b = w
            .embeds
            .iter()
            .find(|e| e.embed_id == "B")
            .expect("embed B");
        assert_eq!(b.pool_entries, 1);
        assert_eq!(b.shard_top_k, 0); // shard rank 3 > top_k 2
        assert_eq!(b.survived_fusion, 1);
        assert_eq!(b.served, 0);
        assert_eq!(b.exposure_entries, 1);
        let fts = w.branches.iter().find(|x| x.branch == "fts").expect("fts");
        assert_eq!(fts.pool_entries, 1);
        assert_eq!(fts.survival_rate(), 1.0);
        let vec = w
            .branches
            .iter()
            .find(|x| x.branch == "vector")
            .expect("vector");
        assert_eq!(vec.pool_entries, 2);
        assert_eq!(vec.survived_fusion, 2);
    }

    #[test]
    fn spot_check_rank1_derivable_from_raw_traces() {
        // Acceptance spot-check: recount rank-1 serves by scanning
        // the raw lines with no aggregation code, and compare.
        let dir = temp_dir("spot");
        fixture(&dir);
        let mut raw_rank1 = 0u64;
        for name in [
            "atlas-traces-2026-11-13.jsonl",
            "atlas-traces-2026-11-14.jsonl",
        ] {
            let body = std::fs::read_to_string(dir.join(name)).expect("read");
            for line in body.lines() {
                let rec: Json = serde_json::from_str(line).expect("parse");
                for r in rec["results"].as_array().expect("results") {
                    if r["stages"]["final"]["rank"] == json!(1) {
                        raw_rank1 += 1;
                    }
                }
            }
        }
        let w = aggregate_traces(&dir, (2026, 11, 14), 7, &[]).expect("aggregate");
        let agg_rank1: u64 = w.embeds.iter().map(|e| e.rank1).sum();
        assert_eq!(agg_rank1, raw_rank1);
        assert_eq!(agg_rank1, 1);
    }

    #[test]
    fn window_excludes_older_traces() {
        let dir = temp_dir("window");
        fixture(&dir);
        write_file(
            &dir,
            "atlas-traces-2026-10-01.jsonl",
            &[trace_line(
                "qold",
                "2026-10-01T10:00:00Z",
                vec![result(9, "f9", "OLD", "fts", 1, Some(1), Some(1))],
            )],
        );
        let w = aggregate_traces(&dir, (2026, 11, 14), 7, &[]).expect("aggregate");
        assert_eq!(w.traces, 2);
        assert!(w.embeds.iter().all(|e| e.embed_id != "OLD"));
        let w28 = aggregate_traces(&dir, (2026, 11, 14), 28, &[]).expect("aggregate 28");
        assert_eq!(w28.traces, 2); // 2026-10-01 is 44 days out, still excluded
    }

    #[test]
    fn feedback_weighting_and_unopened_split() {
        let dir = temp_dir("fb");
        fixture(&dir);
        let votes = vec![
            FeedbackVote {
                query_id: "q1".into(),
                chunk_id: 1,
                up: true,
                opened: true,
                requester_kind: "drew".into(),
                requester_id: "drew".into(),
            },
            FeedbackVote {
                query_id: "q1".into(),
                chunk_id: 1,
                up: false,
                opened: true,
                requester_kind: "agent".into(),
                requester_id: "sess-1".into(),
            },
            FeedbackVote {
                query_id: "q1".into(),
                chunk_id: 1,
                up: false,
                opened: false,
                requester_kind: "user".into(),
                requester_id: "u-2".into(),
            },
            FeedbackVote {
                query_id: "nope".into(),
                chunk_id: 99,
                up: true,
                opened: true,
                requester_kind: "user".into(),
                requester_id: "u-3".into(),
            },
        ];
        let w = aggregate_traces(&dir, (2026, 11, 14), 7, &votes).expect("aggregate");
        let f = w.feedback.iter().find(|x| x.embed_id == "A").expect("fb A");
        assert_eq!(f.up_votes, 1);
        assert_eq!(f.weighted_up, 1.0);
        assert_eq!(f.down_votes, 1);
        assert_eq!(f.weighted_down, 0.25);
        assert_eq!(f.snippet_down_votes, 1); // unopened: not content quality
        assert_eq!(f.unique_requesters.len(), 3);
        assert_eq!(w.feedback_unjoined, 1);
        // Feedback never merges into retrieval statistics.
        let a = w
            .embeds
            .iter()
            .find(|e| e.embed_id == "A")
            .expect("embed A");
        assert_eq!(a.served, 2);
    }

    #[test]
    fn weekly_report_nominations_and_artifact() {
        let dir = temp_dir("report");
        // Embed P served at rank 1 three times in 7d (protected);
        // embed R pooled-but-unserved three times in 28d (retention).
        let mut lines14 = Vec::new();
        for i in 0..3 {
            lines14.push(trace_line(
                &format!("qp{i}"),
                "2026-11-14T10:00:00Z",
                vec![result(1, "fp", "P", "fts", 1, Some(1), Some(1))],
            ));
            lines14.push(trace_line(
                &format!("qr{i}"),
                "2026-11-14T11:00:00Z",
                vec![result(2, "fr", "R", "vector", 2, Some(1), None)],
            ));
        }
        write_file(&dir, "atlas-traces-2026-11-14.jsonl", &lines14);
        let report = weekly_report(&dir, (2026, 11, 14), &[]).expect("report");
        let kinds: Vec<&str> = report.nominations.iter().map(|n| n.kind).collect();
        assert!(kinds.contains(&"protected_embed"), "kinds: {kinds:?}");
        assert!(kinds.contains(&"retention_candidate"), "kinds: {kinds:?}");
        let j = report.to_json();
        assert_eq!(j["schema"], json!(REPORT_SCHEMA));
        assert_eq!(j["source_class"], json!("statistics and reports"));
        assert_eq!(j["unconsumed_aggregates"], json!([]));
        let path = write_report(&dir, &report).expect("write report");
        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            "atlas-statistics-report-2026-11-14.jsonl"
        );
        let body = std::fs::read_to_string(&path).expect("read report");
        let rec: Json = serde_json::from_str(body.trim()).expect("report record parses");
        assert_eq!(rec["schema"], json!(REPORT_SCHEMA));
        let md = report.render_markdown();
        assert!(md.contains("protected_embed"));
        assert!(md.contains("Atlas weekly statistics report"));
    }
}
