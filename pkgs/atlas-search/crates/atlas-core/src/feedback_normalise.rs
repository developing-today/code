//! Feedback normalisation — `atlas-feedback-normalised/v1`
//! (issue #9977, follow-on 5 of the #6027 design, doc
//! `2026-11-10T20-41-01Z_design_atlas_retrieval_stats_feedback.md`
//! section 6.2).
//!
//! Raw feedback records land unmodified (invariant 4) and are
//! never rewritten by this pass: normalisation emits a DERIVED
//! record beside the raw one, and because the raw store is
//! preserved verbatim the derivatives can be regenerated whenever
//! the pass is amended (design section 6.2 point 3) —
//! `write_normalised` therefore rewrites its per-day file, never
//! appends to it.
//!
//! The pass implemented here is the deterministic seam of the
//! design's "agent normalisation pass": category assignment is a
//! documented phrase-rule classifier over the comment text (the
//! eight section 6.2 categories), the summary is the comment's
//! first sentence, and a comment that explicitly names a different
//! chunk ("chunk 77") sets `about_chunk_id` when it differs from
//! the voted chunk. Vote-only records skip the pass and aggregate
//! directly (section 6.2 point 2). The weights a derivative
//! carries into aggregation are the section 6.3 parameters, with
//! the section 6.4 self-feedback rule applied on top: an agent
//! voting on a result served to a query that same agent (same
//! session lineage, read from the trace's requester identity)
//! issued in work it is itself evaluating is recorded at weight 0
//! and flagged `self_feedback`. Agents judging OTHER sessions'
//! retrievals keep the ordinary 0.25 agent weight.
//!
//! Human spot-checks (section 6.2 point 3) sample these
//! derivatives weekly; systematic errors are fixed by amending
//! the rules here and regenerating, never by editing a stored
//! derivative.

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::{Map, Value as Json, json};

use crate::error::{Error, Result};
use crate::feedback::{FeedbackRecord, active_view};
use crate::trace_stats::requester_weight;

/// Schema of one normalised derivative record.
pub const NORMALISED_SCHEMA: &str = "atlas-feedback-normalised/v1";

/// The section 6.2 categories, in the design's order. A comment
/// may carry several; `other` is the fallback when no phrase
/// rule matches.
pub const FEEDBACK_CATEGORIES: [&str; 8] = [
    "wrong-content",
    "stale-content",
    "right-file-wrong-chunk",
    "ranking-too-low",
    "ranking-too-high",
    "missing-result",
    "duplicate-or-near-duplicate",
    "other",
];

/// Phrase rules per category, matched against the lowercased
/// comment. The rules are deliberately plain substrings: they
/// are the auditable contract a spot-check samples against, and
/// amending them regenerates every derivative from the raw
/// store.
fn category_rules() -> [(&'static str, &'static [&'static str]); 7] {
    [
        (
            "wrong-content",
            &[
                "wrong content",
                "does not match",
                "doesn't match",
                "not about",
                "unrelated to the query",
                "factually wrong",
                "incorrect content",
            ],
        ),
        (
            "stale-content",
            &[
                "stale",
                "outdated",
                "out of date",
                "old version",
                "no longer current",
                "no longer applies",
                "obsolete",
                "superseded version",
            ],
        ),
        (
            "right-file-wrong-chunk",
            &[
                "wrong chunk",
                "wrong section",
                "right file",
                "another chunk",
                "different chunk",
                "wrong part of the file",
                "wrong part of the document",
            ],
        ),
        (
            "ranking-too-low",
            &[
                "ranking too low",
                "ranked too low",
                "should rank higher",
                "should be ranked higher",
                "should be first",
                "should have been top",
                "under-ranked",
                "underranked",
                "buried",
            ],
        ),
        (
            "ranking-too-high",
            &[
                "ranking too high",
                "ranked too high",
                "should rank lower",
                "should be ranked lower",
                "over-ranked",
                "overranked",
            ],
        ),
        (
            "missing-result",
            &[
                "missing",
                "didn't find",
                "did not find",
                "couldn't find",
                "could not find",
                "no result for",
                "nothing about",
                "not indexed",
                "absent from the results",
                "omitted",
            ],
        ),
        (
            "duplicate-or-near-duplicate",
            &[
                "duplicate",
                "near-duplicate",
                "near duplicate",
                "same content twice",
                "repeated result",
            ],
        ),
    ]
}

/// Assign the section 6.2 categories for one comment: every
/// category whose phrase rules match, in design order, or
/// `other` when none do. Matching is on the lowercased text;
/// the raw comment is never altered.
pub fn classify_comment(comment: &str) -> Vec<&'static str> {
    let lower = comment.to_lowercase();
    let mut out: Vec<&'static str> = Vec::new();
    for (category, phrases) in category_rules() {
        if phrases.iter().any(|p| lower.contains(p)) {
            out.push(category);
        }
    }
    if out.is_empty() {
        out.push("other");
    }
    out
}

/// The derivative's one-paragraph summary: the comment's first
/// sentence (up to the first sentence end or newline), capped
/// at 280 characters.
pub fn summarise(comment: &str) -> String {
    let trimmed = comment.trim();
    let end = trimmed
        .find(['.', '!', '?'])
        .map(|i| i + 1)
        .unwrap_or(trimmed.len());
    let first = trimmed[..end].trim();
    if first.chars().count() <= 280 {
        return first.to_string();
    }
    let mut out: String = first.chars().take(277).collect();
    out.push_str("...");
    out
}

/// A chunk the comment explicitly names ("chunk 77",
/// "chunk_id 77", "chunk #77"), when it differs from the voted
/// chunk — the section 6.2 "chunk the comment is actually
/// about" field, for the cases a comment states it outright.
pub fn about_chunk(comment: &str, voted_chunk: Option<i64>) -> Option<i64> {
    let lower = comment.to_lowercase();
    for marker in ["chunk_id", "chunk"] {
        let mut rest = lower.as_str();
        while let Some(pos) = rest.find(marker) {
            let after = &rest[pos + marker.len()..];
            let digits: String = after
                .trim_start_matches([' ', '#', ':', '='])
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(n) = digits.parse::<i64>()
                && voted_chunk != Some(n)
            {
                return Some(n);
            }
            rest = &rest[pos + marker.len()..];
        }
    }
    None
}

/// Query requester identities read from the traces:
/// `query_id -> (requester kind, requester id)`.
pub type QueryRequesters = HashMap<String, (String, String)>;

/// The section 6.4 self-feedback test: the voter is an agent
/// and the query the vote judges was issued by that same agent
/// (the trace for the query names the same requester id, as an
/// agent). Human and anonymous votes are never self-feedback,
/// and neither is an agent judging another session's retrieval.
pub fn is_self_feedback(
    requester_kind: &str,
    requester_id: &str,
    query_id: &str,
    query_requesters: &QueryRequesters,
) -> bool {
    if requester_kind != "agent" || requester_id.is_empty() {
        return false;
    }
    query_requesters
        .get(query_id)
        .is_some_and(|(kind, id)| kind == "agent" && id == requester_id)
}

/// One normalised derivative of a raw feedback record. The raw
/// record is carried by reference (`feedback_id`) and its
/// comment verbatim — the derivative never replaces the raw
/// record (invariant 4).
#[derive(Debug, Clone, PartialEq)]
pub struct NormalisedFeedback {
    pub feedback_id: String,
    pub query_id: String,
    pub chunk_id: Option<i64>,
    pub vote: crate::feedback::Vote,
    pub categories: Vec<&'static str>,
    pub summary: String,
    /// The raw comment, verbatim.
    pub raw_comment: String,
    pub requester_kind: String,
    pub requester_id: String,
    pub self_feedback: bool,
    /// The section 6.3 aggregation weight, 0 for self-feedback.
    pub weight: f64,
    pub about_chunk_id: Option<i64>,
    pub ts: String,
}

impl NormalisedFeedback {
    pub fn to_json(&self) -> Json {
        let mut rec = Map::new();
        rec.insert("schema".into(), Json::String(NORMALISED_SCHEMA.to_string()));
        rec.insert("feedback_id".into(), Json::String(self.feedback_id.clone()));
        rec.insert("query_id".into(), Json::String(self.query_id.clone()));
        rec.insert(
            "chunk_id".into(),
            self.chunk_id.map(Json::from).unwrap_or(Json::Null),
        );
        rec.insert("vote".into(), Json::String(self.vote.as_str().to_string()));
        rec.insert(
            "categories".into(),
            Json::Array(
                self.categories
                    .iter()
                    .map(|c| Json::String(c.to_string()))
                    .collect(),
            ),
        );
        rec.insert("summary".into(), Json::String(self.summary.clone()));
        rec.insert("comment".into(), Json::String(self.raw_comment.clone()));
        rec.insert(
            "requester".into(),
            json!({"kind": self.requester_kind, "id": self.requester_id}),
        );
        rec.insert("self_feedback".into(), Json::Bool(self.self_feedback));
        rec.insert("weight".into(), json!(self.weight));
        rec.insert(
            "about_chunk_id".into(),
            self.about_chunk_id.map(Json::from).unwrap_or(Json::Null),
        );
        rec.insert("ts".into(), Json::String(self.ts.clone()));
        Json::Object(rec)
    }
}

/// Run the normalisation pass over raw records: the ACTIVE
/// records (section 6.4 supersede view) that carry a comment.
/// Vote-only records skip the pass, and superseded records keep
/// only their raw form — a re-vote's comment replaces the old
/// one in the derivatives exactly as it does in the aggregates.
pub fn normalise_records(
    records: &[FeedbackRecord],
    query_requesters: &QueryRequesters,
) -> Vec<NormalisedFeedback> {
    active_view(records)
        .into_iter()
        .filter(|v| v.active)
        .filter_map(|v| {
            let comment = v.record.comment.as_ref()?.trim();
            if comment.is_empty() {
                return None;
            }
            let raw = v.record.comment.clone()?;
            let self_feedback = is_self_feedback(
                &v.record.requester_kind,
                &v.record.requester_id,
                &v.record.query_id,
                query_requesters,
            );
            Some(NormalisedFeedback {
                feedback_id: v.record.feedback_id.clone(),
                query_id: v.record.query_id.clone(),
                chunk_id: v.record.chunk_id,
                vote: v.record.vote,
                categories: classify_comment(&raw),
                summary: summarise(&raw),
                about_chunk_id: about_chunk(&raw, v.record.chunk_id),
                raw_comment: raw,
                requester_kind: v.record.requester_kind.clone(),
                requester_id: v.record.requester_id.clone(),
                self_feedback,
                weight: if self_feedback {
                    0.0
                } else {
                    requester_weight(&v.record.requester_kind)
                },
                ts: v.record.ts.clone(),
            })
        })
        .collect()
}

/// The derivative store's file name for a day.
pub fn normalised_file_name(day: &str) -> String {
    format!("atlas-feedback-normalised-{day}.jsonl")
}

/// Land one day's derivatives: one JSONL record per normalised
/// record in `dir`. Re-running for the same day rewrites the
/// same file — derivatives are regenerated from the raw store,
/// never appended to (section 6.2 point 3).
pub fn write_normalised(
    dir: &std::path::Path,
    day: &str,
    normalised: &[NormalisedFeedback],
) -> Result<PathBuf> {
    std::fs::create_dir_all(dir).map_err(|e| {
        Error::backend(format!(
            "feedback normalise: cannot create {}: {e}",
            dir.display()
        ))
    })?;
    let path = dir.join(normalised_file_name(day));
    let mut body = String::new();
    for n in normalised {
        body.push_str(&n.to_json().to_string());
        body.push('\n');
    }
    std::fs::write(&path, body).map_err(|e| {
        Error::backend(format!(
            "feedback normalise: cannot write {}: {e}",
            path.display()
        ))
    })?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feedback::Vote;

    // Oracle for every test in this module: design section 6.2
    // (categories, raw preservation, regeneration), section 6.3
    // (weights), and section 6.4 (self-feedback) of the #6027
    // design doc.

    fn record(
        id: &str,
        query: &str,
        chunk: Option<i64>,
        kind: &str,
        requester: &str,
        comment: Option<&str>,
    ) -> FeedbackRecord {
        FeedbackRecord {
            feedback_id: id.to_string(),
            query_id: query.to_string(),
            chunk_id: chunk,
            vote: Vote::Down,
            comment: comment.map(str::to_string),
            requester_kind: kind.to_string(),
            requester_id: requester.to_string(),
            ts: "2026-11-14T10:00:00Z".to_string(),
            opened: true,
        }
    }

    #[test]
    fn sample_essays_normalise_into_the_design_categories() {
        assert_eq!(
            classify_comment(
                "This embed is stale — it describes the old version of the config and is outdated."
            ),
            vec!["stale-content"]
        );
        assert_eq!(
            classify_comment("This is wrong content: it does not match the query at all."),
            vec!["wrong-content"]
        );
        assert_eq!(
            classify_comment(
                "The right file but the wrong chunk — the answer sits in another chunk of it."
            ),
            vec!["right-file-wrong-chunk"]
        );
        assert_eq!(
            classify_comment("This should rank higher; it was buried under weaker results."),
            vec!["ranking-too-low"]
        );
        assert_eq!(
            classify_comment("The guide I needed is missing — the search didn't find it at all."),
            vec!["missing-result"]
        );
        assert_eq!(
            classify_comment("Same content twice: a duplicate of the chunk above it."),
            vec!["duplicate-or-near-duplicate"]
        );
        // No phrase rule hits: the fallback category, never none.
        assert_eq!(
            classify_comment("Useful context, thanks — noted for later."),
            vec!["other"]
        );
        // Multi-category comments carry every matching category.
        let multi = classify_comment("Stale and wrong content — outdated and does not match.");
        assert!(multi.contains(&"stale-content"), "{multi:?}");
        assert!(multi.contains(&"wrong-content"), "{multi:?}");
    }

    #[test]
    fn raw_comment_is_preserved_verbatim_alongside_the_derivative() {
        let raw = "  Stale — describes the OLD version.\nSecond line here.  ";
        let records = vec![record("f1", "q1", Some(42), "user", "u-1", Some(raw))];
        let out = normalise_records(&records, &QueryRequesters::new());
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].raw_comment, raw,
            "verbatim, whitespace and case intact"
        );
        assert_eq!(out[0].categories, vec!["stale-content"]);
        assert_eq!(out[0].summary, "Stale — describes the OLD version.");
        let j = out[0].to_json();
        assert_eq!(j["schema"], json!(NORMALISED_SCHEMA));
        assert_eq!(j["comment"], json!(raw));
    }

    #[test]
    fn self_feedback_is_weight_zero_and_flagged() {
        let mut requesters = QueryRequesters::new();
        requesters.insert(
            "q-self".to_string(),
            ("agent".to_string(), "sess-1".to_string()),
        );
        requesters.insert(
            "q-other".to_string(),
            ("agent".to_string(), "sess-9".to_string()),
        );
        let records = vec![
            // The evaluating agent votes on its own query's result.
            record(
                "f-self",
                "q-self",
                Some(1),
                "agent",
                "sess-1",
                Some("Stale content in my own result."),
            ),
            // Another agent judging that session's retrieval: ordinary signal.
            record(
                "f-peer",
                "q-self",
                Some(2),
                "agent",
                "sess-2",
                Some("Stale content here too."),
            ),
            // The same agent judging a different session's query.
            record(
                "f-cross",
                "q-other",
                Some(3),
                "agent",
                "sess-1",
                Some("Stale again."),
            ),
            // Drew's vote is never self-feedback.
            record(
                "f-drew",
                "q-self",
                Some(4),
                "drew",
                "drew",
                Some("Stale, please re-ingest."),
            ),
        ];
        let out = normalise_records(&records, &requesters);
        let by_id: HashMap<&str, &NormalisedFeedback> =
            out.iter().map(|n| (n.feedback_id.as_str(), n)).collect();
        assert!(by_id["f-self"].self_feedback);
        assert_eq!(by_id["f-self"].weight, 0.0);
        assert!(!by_id["f-peer"].self_feedback);
        assert_eq!(by_id["f-peer"].weight, 0.25);
        assert!(!by_id["f-cross"].self_feedback);
        assert_eq!(by_id["f-cross"].weight, 0.25);
        assert!(!by_id["f-drew"].self_feedback);
        assert_eq!(by_id["f-drew"].weight, 1.0);
    }

    #[test]
    fn vote_only_and_superseded_records_skip_the_pass() {
        let records = vec![
            record("f1", "q1", Some(1), "user", "u-1", None),
            record(
                "f2",
                "q1",
                Some(2),
                "user",
                "u-1",
                Some("Stale first take."),
            ),
            // Same (requester, query, chunk) as f2: f3 supersedes it.
            record(
                "f3",
                "q1",
                Some(2),
                "user",
                "u-1",
                Some("Actually it is fine."),
            ),
        ];
        let out = normalise_records(&records, &QueryRequesters::new());
        assert_eq!(out.len(), 1, "only the active commented record: {out:?}");
        assert_eq!(out[0].feedback_id, "f3");
        assert_eq!(out[0].categories, vec!["other"]);
    }

    #[test]
    fn about_chunk_names_a_different_chunk_only() {
        let records = vec![
            record(
                "f1",
                "q1",
                Some(42),
                "user",
                "u-1",
                Some("Wrong chunk — the answer is in chunk 77 of the same file."),
            ),
            record(
                "f2",
                "q1",
                Some(42),
                "user",
                "u-2",
                Some("Chunk 42 itself is the problem here."),
            ),
        ];
        let out = normalise_records(&records, &QueryRequesters::new());
        assert_eq!(out[0].about_chunk_id, Some(77));
        assert_eq!(out[1].about_chunk_id, None);
    }

    #[test]
    fn write_normalised_round_trip() {
        let dir = std::env::temp_dir().join(format!(
            "atlas-feedback-normalise-{}-write",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let records = vec![record(
            "f1",
            "q1",
            Some(1),
            "user",
            "u-1",
            Some("Stale content, please re-ingest."),
        )];
        let out = normalise_records(&records, &QueryRequesters::new());
        let path = write_normalised(&dir, "2026-11-14", &out).expect("write");
        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            "atlas-feedback-normalised-2026-11-14.jsonl"
        );
        let body = std::fs::read_to_string(&path).expect("read");
        let rec: Json = serde_json::from_str(body.trim()).expect("derivative parses");
        assert_eq!(rec["schema"], json!(NORMALISED_SCHEMA));
        assert_eq!(rec["categories"], json!(["stale-content"]));
        assert_eq!(rec["weight"], json!(1.0));
        // A re-run rewrites rather than appending (regeneration).
        write_normalised(&dir, "2026-11-14", &out).expect("rewrite");
        let body2 = std::fs::read_to_string(&path).expect("read again");
        assert_eq!(body2.lines().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
