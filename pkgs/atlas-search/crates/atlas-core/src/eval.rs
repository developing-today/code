//! Deterministic offline retrieval evaluation for Atlas (#5741).
//!
//! The eval harness is the prerequisite the #5741 test plan calls for
//! before any signal (SPLADE included) earns a place in the fused
//! ranking: saved rankings from arbitrary signals — dense, FTS5/BM25,
//! SPLADE, or any fusion of them — are scored offline against judged
//! qrels. This module never runs a model, a database, or a search; it
//! only reads the versioned input format below and computes metrics.
//!
//! ## Input format `atlas-eval/v1`
//!
//! ```json
//! {
//!   "format": "atlas-eval/v1",
//!   "queries": [
//!     {"id": "q1", "class": "code", "split": "dev", "text": "..."}
//!   ],
//!   "qrels": [
//!     {"query": "q1", "doc": "chunk-1", "grade": 2}
//!   ],
//!   "runs": [
//!     {"signal": "dense", "results": {"q1": ["chunk-2", "chunk-1"]}}
//!   ]
//! }
//! ```
//!
//! - `class` is the query class (`code`, `documentation`, `error`,
//!   `concept`, …) used for the per-class summaries the #5741 ablation
//!   matrix is reported in. Free-form string; classes sort by name.
//! - `split` is `dev` or `test`. The held-out test split is scored
//!   only by [`evaluate`]; [`select_best_signal`] refuses test-split
//!   reports, so parameter selection cannot silently tune on held-out
//!   qrels (#5741 acceptance).
//! - Grades are unsigned integers in `0..=MAX_GRADE` (30); a doc is
//!   *relevant* iff grade ≥ 1. Unjudged docs in a ranking score as
//!   grade 0. The cap keeps the `2^grade - 1` gain finite and exact:
//!   an unbounded `u32` grade could overflow the gain to infinity
//!   (grade 1024) and turn nDCG into NaN, so validation rejects
//!   larger grades instead of scoring them.
//! - A run that has no entry for a query scores that query as an
//!   empty ranking (all metrics 0) — an incomplete run is penalized,
//!   never silently skipped.
//! - A ranking may not repeat a doc, a qrel may not repeat a
//!   (query, doc) pair, a run's `results` may not repeat a query key
//!   (serde_json would otherwise keep only the last ranking), and
//!   qrels must name known queries; violations are rejected at
//!   parse/validation time.
//!
//! ## Metrics and reporting
//!
//! Per query: nDCG@10 (graded gains `2^grade − 1`, log2 discount),
//! Recall@100, Recall@1000, MRR@10. Aggregates are means over the
//! queries that have at least one relevant doc (`queries_scored`);
//! queries with no relevant doc are counted in `queries_total` and
//! listed per-query with zero metrics, but excluded from the means
//! (the standard convention: their recall/MRR are undefined, not 0).
//! Each per-query record preserves the run's top-1000 doc ids and
//! the query's judged grades, so any reported metric (nDCG@10,
//! Recall@100/1000, MRR@10) can be recomputed from the report alone.
//!
//! Reports are deterministic: signals, queries, and classes are
//! processed in sorted order, and every reported float passes through
//! the crate's Python-parity `round6`, so the same input serializes
//! to byte-identical JSON on every run.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::types::round6;

/// The only input-format version this harness reads.
pub const EVAL_FORMAT: &str = "atlas-eval/v1";
/// The report format version emitted by [`evaluate`].
pub const REPORT_FORMAT: &str = "atlas-eval-report/v1";

/// The largest relevance grade the format accepts. Gains are
/// `2^grade - 1` in `f64`; the cap keeps every gain finite and the
/// `grade as i32` exponent cast exact for any accepted input, with
/// wide headroom over real graded-relevance schemes (typically 0-4).
pub const MAX_GRADE: u32 = 30;

/// Which split of the eval set a query belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Split {
    /// Development split: tuning and parameter selection are allowed.
    Dev,
    /// Held-out test split: scoring only, never parameter selection.
    Test,
}

/// One query in the eval set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryRec {
    /// Unique query id.
    pub id: String,
    /// Query class (`code`, `documentation`, `error`, `concept`, …).
    #[serde(rename = "class")]
    pub class: String,
    /// The split this query belongs to.
    pub split: Split,
    /// Optional query text; carried for humans, never scored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// One judged (query, doc) relevance grade.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Qrel {
    /// The query id (must exist in `queries`).
    pub query: String,
    /// The judged doc/chunk id.
    pub doc: String,
    /// Relevance grade (`0..=MAX_GRADE`); relevant iff ≥ 1.
    pub grade: u32,
}

/// One signal's saved rankings: query id → doc ids in rank order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Run {
    /// Signal name (`dense`, `bm25`, `splade`, `fused:dense+bm25`, …).
    pub signal: String,
    /// Ranked doc ids per query (best first). Rank order is the
    /// contract; raw scores are not needed by any metric here.
    pub results: BTreeMap<String, Vec<String>>,
}

// Manual `Deserialize` for `Run` (#6390): serde_json collapses a
// repeated object key into a map silently, so the derived impl dropped
// the first ranking for a repeated query key in `results` before
// `validate()` could ever see it. This impl deserializes `results`
// through a visitor that records a repeated key, then rejects the run
// naming both the signal and the query. Serialization stays derived,
// and struct-field duplicates keep serde's derived rejection.
impl<'de> Deserialize<'de> for Run {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "lowercase")]
        enum Field {
            Signal,
            Results,
            #[serde(other)]
            Other,
        }

        /// `results` plus the first repeated query key, if any.
        struct DupCheckedResults(BTreeMap<String, Vec<String>>, Option<String>);

        impl<'de> Deserialize<'de> for DupCheckedResults {
            fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct ResultsVisitor;

                impl<'de> serde::de::Visitor<'de> for ResultsVisitor {
                    type Value = DupCheckedResults;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                        formatter.write_str("a map of query id to ranked doc ids")
                    }

                    fn visit_map<A>(
                        self,
                        mut access: A,
                    ) -> std::result::Result<Self::Value, A::Error>
                    where
                        A: serde::de::MapAccess<'de>,
                    {
                        let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
                        let mut duplicate: Option<String> = None;
                        while let Some((key, value)) = access.next_entry::<String, Vec<String>>()? {
                            if map.contains_key(&key) && duplicate.is_none() {
                                duplicate = Some(key.clone());
                            }
                            map.entry(key).or_insert(value);
                        }
                        Ok(DupCheckedResults(map, duplicate))
                    }
                }

                deserializer.deserialize_map(ResultsVisitor)
            }
        }

        struct RunVisitor;

        impl<'de> serde::de::Visitor<'de> for RunVisitor {
            type Value = Run;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("an atlas-eval run (signal + results)")
            }

            fn visit_map<A>(self, mut access: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut signal: Option<String> = None;
                let mut results: Option<BTreeMap<String, Vec<String>>> = None;
                let mut duplicate_query: Option<String> = None;
                while let Some(key) = access.next_key::<Field>()? {
                    match key {
                        Field::Signal => {
                            if signal.is_some() {
                                return Err(serde::de::Error::duplicate_field("signal"));
                            }
                            signal = Some(access.next_value()?);
                        }
                        Field::Results => {
                            if results.is_some() {
                                return Err(serde::de::Error::duplicate_field("results"));
                            }
                            let checked = access.next_value::<DupCheckedResults>()?;
                            duplicate_query = checked.1;
                            results = Some(checked.0);
                        }
                        Field::Other => {
                            let _ = access.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }
                let signal = signal.ok_or_else(|| serde::de::Error::missing_field("signal"))?;
                let results = results.ok_or_else(|| serde::de::Error::missing_field("results"))?;
                if let Some(query) = duplicate_query {
                    return Err(serde::de::Error::custom(format!(
                        "duplicate results key: run {signal:?} repeats query {query:?} \
                         (a repeated query key would silently replace that query's ranking)"
                    )));
                }
                Ok(Run { signal, results })
            }
        }

        deserializer.deserialize_struct("Run", &["signal", "results"], RunVisitor)
    }
}

/// A complete `atlas-eval/v1` input document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalData {
    /// Must equal [`EVAL_FORMAT`].
    pub format: String,
    /// The eval queries (either split).
    pub queries: Vec<QueryRec>,
    /// The relevance judgments.
    pub qrels: Vec<Qrel>,
    /// The saved rankings to score.
    pub runs: Vec<Run>,
}

impl EvalData {
    /// Parse and validate an `atlas-eval/v1` JSON document.
    ///
    /// # Errors
    ///
    /// Fails on malformed JSON or any validation rule in the module
    /// docs (wrong format version, duplicate query ids, qrels naming
    /// unknown queries, duplicate qrels, grades above `MAX_GRADE`,
    /// duplicate docs in a ranking, duplicate results keys, duplicate
    /// signal names).
    pub fn from_json(text: &str) -> Result<Self> {
        let data: EvalData = serde_json::from_str(text)
            .map_err(|e| Error::fatal(format!("eval input is not valid atlas-eval JSON: {e}")))?;
        data.validate()?;
        Ok(data)
    }

    /// Check the structural rules of the format.
    ///
    /// # Errors
    ///
    /// Fails with a user-facing message naming the violated rule.
    pub fn validate(&self) -> Result<()> {
        if self.format != EVAL_FORMAT {
            return Err(Error::fatal(format!(
                "eval input format {:?} is not {EVAL_FORMAT:?}",
                self.format
            )));
        }
        let mut query_ids: HashSet<&str> = HashSet::new();
        for q in &self.queries {
            if !query_ids.insert(q.id.as_str()) {
                return Err(Error::fatal(format!("duplicate query id {:?}", q.id)));
            }
        }
        let mut qrel_keys: HashSet<(&str, &str)> = HashSet::new();
        for qrel in &self.qrels {
            if !query_ids.contains(qrel.query.as_str()) {
                return Err(Error::fatal(format!(
                    "qrel names unknown query {:?}",
                    qrel.query
                )));
            }
            if qrel.grade > MAX_GRADE {
                return Err(Error::fatal(format!(
                    "qrel grade {} for query {:?} doc {:?} exceeds the supported \
                     maximum {MAX_GRADE} (larger grades overflow the 2^grade \
                     gain calculation)",
                    qrel.grade, qrel.query, qrel.doc
                )));
            }
            if !qrel_keys.insert((qrel.query.as_str(), qrel.doc.as_str())) {
                return Err(Error::fatal(format!(
                    "duplicate qrel for query {:?} doc {:?}",
                    qrel.query, qrel.doc
                )));
            }
        }
        let mut signals: HashSet<&str> = HashSet::new();
        for run in &self.runs {
            if !signals.insert(run.signal.as_str()) {
                return Err(Error::fatal(format!(
                    "duplicate run for signal {:?}",
                    run.signal
                )));
            }
            for (query, ranking) in &run.results {
                if !query_ids.contains(query.as_str()) {
                    return Err(Error::fatal(format!(
                        "run {:?} names unknown query {:?}",
                        run.signal, query
                    )));
                }
                let mut seen: HashSet<&str> = HashSet::new();
                for doc in ranking {
                    if !seen.insert(doc.as_str()) {
                        return Err(Error::fatal(format!(
                            "run {:?} repeats doc {:?} for query {:?}",
                            run.signal, doc, query
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    fn grades_for(&self, query: &str) -> BTreeMap<String, u32> {
        self.qrels
            .iter()
            .filter(|q| q.query == query)
            .map(|q| (q.doc.clone(), q.grade))
            .collect()
    }
}

/// The four headline metrics, as means over scored queries.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MetricMeans {
    /// Mean nDCG@10.
    pub ndcg10: f64,
    /// Mean Recall@100.
    pub recall100: f64,
    /// Mean Recall@1000.
    pub recall1000: f64,
    /// Mean MRR@10.
    pub mrr10: f64,
}

/// One metric a caller might select parameters by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    /// nDCG@10.
    Ndcg10,
    /// Recall@100.
    Recall100,
    /// Recall@1000.
    Recall1000,
    /// MRR@10.
    Mrr10,
}

impl Metric {
    fn of(self, means: &MetricMeans) -> f64 {
        match self {
            Metric::Ndcg10 => means.ndcg10,
            Metric::Recall100 => means.recall100,
            Metric::Recall1000 => means.recall1000,
            Metric::Mrr10 => means.mrr10,
        }
    }
}

/// Per-query metric detail, with the evidence needed to recompute
/// the metrics from the report alone preserved alongside them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerQuery {
    /// Query id.
    pub query: String,
    /// Query class.
    #[serde(rename = "class")]
    pub class: String,
    /// Number of relevant (grade ≥ 1) docs for this query.
    pub relevant: usize,
    /// nDCG@10 for this query.
    pub ndcg10: f64,
    /// Recall@100 for this query.
    pub recall100: f64,
    /// Recall@1000 for this query.
    pub recall1000: f64,
    /// Reciprocal rank of the first relevant doc within the top 10.
    pub mrr10: f64,
    /// The run's top-1000 doc ids for this query, preserved so the
    /// metrics above (including Recall@1000) can be recomputed from
    /// the report alone.
    pub top1000: Vec<String>,
    /// The query's judged grades (doc id -> grade), preserved for
    /// the same recomputation: without them nDCG and the recall hits
    /// cannot be rechecked from the report alone.
    pub grades: BTreeMap<String, u32>,
}

/// One signal's scores within a split.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignalReport {
    /// Signal name.
    pub signal: String,
    /// Queries in the split.
    pub queries_total: usize,
    /// Queries with at least one relevant doc (the means' denominator).
    pub queries_scored: usize,
    /// Means over all scored queries in the split.
    pub overall: MetricMeans,
    /// Means per query class (classes sorted by name).
    pub per_class: BTreeMap<String, MetricMeans>,
    /// Per-query detail, sorted by query id.
    pub per_query: Vec<PerQuery>,
}

/// A complete evaluation report for one split.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalReport {
    /// Always [`REPORT_FORMAT`].
    pub format: String,
    /// The split this report scored.
    pub split: Split,
    /// One entry per signal, sorted by signal name.
    pub signals: Vec<SignalReport>,
}

fn dcg(gains_in_rank_order: &[f64], k: usize) -> f64 {
    gains_in_rank_order
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, gain)| gain / ((i + 2) as f64).log2())
        .sum()
}

struct RawMetrics {
    ndcg10: f64,
    recall100: f64,
    recall1000: f64,
    mrr10: f64,
}

fn query_metrics(ranking: &[String], grades: &BTreeMap<String, u32>) -> (usize, RawMetrics) {
    let relevant: Vec<&String> = grades
        .iter()
        .filter(|(_, grade)| **grade >= 1)
        .map(|(doc, _)| doc)
        .collect();
    let n_relevant = relevant.len();
    if n_relevant == 0 {
        return (
            0,
            RawMetrics {
                ndcg10: 0.0,
                recall100: 0.0,
                recall1000: 0.0,
                mrr10: 0.0,
            },
        );
    }
    let gains: Vec<f64> = ranking
        .iter()
        .map(|doc| {
            let grade = grades.get(doc).copied().unwrap_or(0);
            (2f64.powi(grade as i32) - 1.0).max(0.0)
        })
        .collect();
    let mut ideal: Vec<f64> = grades
        .values()
        .map(|grade| (2f64.powi(*grade as i32) - 1.0).max(0.0))
        .collect();
    ideal.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let idcg = dcg(&ideal, 10);
    let ndcg10 = if idcg > 0.0 {
        dcg(&gains, 10) / idcg
    } else {
        0.0
    };
    let recall_at = |k: usize| {
        let hits = ranking
            .iter()
            .take(k)
            .filter(|doc| grades.get(*doc).copied().unwrap_or(0) >= 1)
            .count();
        hits as f64 / n_relevant as f64
    };
    let mrr10 = ranking
        .iter()
        .take(10)
        .position(|doc| grades.get(doc).copied().unwrap_or(0) >= 1)
        .map_or(0.0, |i| 1.0 / (i + 1) as f64);
    (
        n_relevant,
        RawMetrics {
            ndcg10,
            recall100: recall_at(100),
            recall1000: recall_at(1000),
            mrr10,
        },
    )
}

#[derive(Default)]
struct Accum {
    n: usize,
    ndcg10: f64,
    recall100: f64,
    recall1000: f64,
    mrr10: f64,
}

impl Accum {
    fn add(&mut self, m: &RawMetrics) {
        self.n += 1;
        self.ndcg10 += m.ndcg10;
        self.recall100 += m.recall100;
        self.recall1000 += m.recall1000;
        self.mrr10 += m.mrr10;
    }

    fn means(&self) -> MetricMeans {
        if self.n == 0 {
            return MetricMeans {
                ndcg10: 0.0,
                recall100: 0.0,
                recall1000: 0.0,
                mrr10: 0.0,
            };
        }
        let n = self.n as f64;
        MetricMeans {
            ndcg10: round6(self.ndcg10 / n),
            recall100: round6(self.recall100 / n),
            recall1000: round6(self.recall1000 / n),
            mrr10: round6(self.mrr10 / n),
        }
    }
}

/// Score every run in `data` over the queries of one split.
///
/// # Errors
///
/// Fails if the data does not validate, or the split has no queries.
pub fn evaluate(data: &EvalData, split: Split) -> Result<EvalReport> {
    data.validate()?;
    let mut queries: Vec<&QueryRec> = data.queries.iter().filter(|q| q.split == split).collect();
    queries.sort_by(|a, b| a.id.cmp(&b.id));
    if queries.is_empty() {
        return Err(Error::fatal(format!(
            "eval input has no queries in the {split:?} split"
        )));
    }
    let mut runs: Vec<&Run> = data.runs.iter().collect();
    runs.sort_by(|a, b| a.signal.cmp(&b.signal));
    let mut signals = Vec::with_capacity(runs.len());
    for run in runs {
        let mut overall = Accum::default();
        let mut per_class: BTreeMap<String, Accum> = BTreeMap::new();
        let mut per_query = Vec::with_capacity(queries.len());
        for query in &queries {
            let grades = data.grades_for(&query.id);
            let empty: Vec<String> = Vec::new();
            let ranking = run.results.get(&query.id).unwrap_or(&empty);
            let (relevant, raw) = query_metrics(ranking, &grades);
            if relevant > 0 {
                overall.add(&raw);
                per_class.entry(query.class.clone()).or_default().add(&raw);
            }
            per_query.push(PerQuery {
                query: query.id.clone(),
                class: query.class.clone(),
                relevant,
                ndcg10: round6(raw.ndcg10),
                recall100: round6(raw.recall100),
                recall1000: round6(raw.recall1000),
                mrr10: round6(raw.mrr10),
                top1000: ranking.iter().take(1000).cloned().collect(),
                grades: grades.clone(),
            });
        }
        signals.push(SignalReport {
            signal: run.signal.clone(),
            queries_total: queries.len(),
            queries_scored: overall.n,
            overall: overall.means(),
            per_class: per_class
                .iter()
                .map(|(class, acc)| (class.clone(), acc.means()))
                .collect(),
            per_query,
        });
    }
    Ok(EvalReport {
        format: REPORT_FORMAT.to_owned(),
        split,
        signals,
    })
}

/// Pick the best signal in a report by one metric — the harness's
/// parameter-selection helper. Deterministic: ties go to the
/// lexicographically smallest signal name.
///
/// # Errors
///
/// Fails for a held-out **test**-split report (#5741: the test split
/// is for final scoring only; tuning on it would invalidate it), and
/// for a report with no signals.
pub fn select_best_signal(report: &EvalReport, metric: Metric) -> Result<String> {
    if report.split == Split::Test {
        return Err(Error::fatal(
            "held-out test results must not be used for parameter selection (#5741); \
             select on the dev split and score the test split once",
        ));
    }
    let mut best: Option<(&SignalReport, f64)> = None;
    for signal in &report.signals {
        let value = metric.of(&signal.overall);
        if best.is_none_or(|(_, best_value)| value > best_value) {
            best = Some((signal, value));
        }
    }
    best.map(|(signal, _)| signal.signal.clone())
        .ok_or_else(|| Error::fatal("report has no signals to select from"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(json: serde_json::Value) -> EvalData {
        EvalData::from_json(&json.to_string()).expect("valid eval data")
    }

    fn grades(pairs: &[(&str, u32)]) -> BTreeMap<String, u32> {
        pairs
            .iter()
            .map(|(doc, grade)| (doc.to_string(), *grade))
            .collect()
    }

    fn ranking(docs: &[&str]) -> Vec<String> {
        docs.iter().map(ToString::to_string).collect()
    }

    // Oracle: hand-computed from the metric definitions. Gains are
    // 2^grade − 1: d1 (grade 2) → 3, d2 (grade 1) → 1. Ranking
    // [d2, d1, d4, d3] has DCG@10 = 1/log2(2) + 3/log2(3)
    // = 1 + 1.892789 = 2.892789; the ideal ranking [d1, d2, …] has
    // IDCG = 3/log2(2) + 1/log2(3) = 3.630930; nDCG = 0.7967076, reported as 0.796708.
    #[test]
    fn ndcg_golden_hand_computed() {
        let g = grades(&[("d1", 2), ("d2", 1), ("d3", 0)]);
        let (_, m) = query_metrics(&ranking(&["d2", "d1", "d4", "d3"]), &g);
        assert!((m.ndcg10 - 0.7967076).abs() < 1e-6, "ndcg10 = {}", m.ndcg10);
    }

    // Oracle: hand-computed. The ideal ordering of the judged grades
    // scores nDCG exactly 1, MRR exactly 1, recall exactly 1 at any k.
    #[test]
    fn perfect_ranking_scores_one() {
        let g = grades(&[("d1", 2), ("d2", 1), ("d3", 1)]);
        let (_, m) = query_metrics(&ranking(&["d1", "d2", "d3"]), &g);
        assert!((m.ndcg10 - 1.0).abs() < 1e-12);
        assert!((m.mrr10 - 1.0).abs() < 1e-12);
        assert!((m.recall100 - 1.0).abs() < 1e-12);
    }

    // Oracle: hand-computed. First relevant doc at rank 3 → MRR@10
    // = 1/3; no relevant doc in the top 10 → 0.
    #[test]
    fn mrr_golden() {
        let g = grades(&[("d1", 1)]);
        let (_, m) = query_metrics(&ranking(&["x", "y", "d1"]), &g);
        assert!((m.mrr10 - 1.0 / 3.0).abs() < 1e-12);
        let (_, m) = query_metrics(&ranking(&["x", "y", "z"]), &g);
        assert_eq!(m.mrr10, 0.0);
    }

    // Oracle: hand-computed. Two relevant docs at ranks 50 and 120 of
    // a 150-doc ranking: Recall@100 = 1/2, Recall@1000 = 2/2 = 1.
    #[test]
    fn recall_golden_across_the_100_boundary() {
        let mut docs: Vec<String> = (0..150).map(|i| format!("x{i}")).collect();
        docs[49] = "rel-a".to_owned();
        docs[119] = "rel-b".to_owned();
        let g = grades(&[("rel-a", 1), ("rel-b", 1)]);
        let (_, m) = query_metrics(&docs, &g);
        assert!((m.recall100 - 0.5).abs() < 1e-12);
        assert!((m.recall1000 - 1.0).abs() < 1e-12);
    }

    // Oracle: the input itself, scored by hand. q1 (code): dense
    // ranking [d2, d1] against grades d1=2, d2=1 is the nDCG golden
    // above (0.796708). q2 (documentation): ranking [d1] against
    // d1=1 is perfect (1.0). Overall mean = 0.898354 (rounded);
    // per-class means keep the two values separate.
    #[test]
    fn overall_and_per_class_means_golden() {
        let d = data(serde_json::json!({
            "format": "atlas-eval/v1",
            "queries": [
                {"id": "q1", "class": "code", "split": "dev"},
                {"id": "q2", "class": "documentation", "split": "dev"}
            ],
            "qrels": [
                {"query": "q1", "doc": "d1", "grade": 2},
                {"query": "q1", "doc": "d2", "grade": 1},
                {"query": "q2", "doc": "d1", "grade": 1}
            ],
            "runs": [
                {"signal": "dense", "results": {"q1": ["d2", "d1"], "q2": ["d1"]}}
            ]
        }));
        let report = evaluate(&d, Split::Dev).unwrap();
        let signal = &report.signals[0];
        assert_eq!(signal.queries_total, 2);
        assert_eq!(signal.queries_scored, 2);
        assert_eq!(signal.overall.ndcg10, 0.898354);
        assert_eq!(signal.per_class["code"].ndcg10, 0.796708);
        assert_eq!(signal.per_class["documentation"].ndcg10, 1.0);
        // The per-query top-1000 is preserved for recomputation.
        assert_eq!(signal.per_query[0].top1000, ranking(&["d2", "d1"]));
    }

    // Oracle: the split labels in the input. Dev evaluation must not
    // score (or even list) test queries, and vice versa.
    #[test]
    fn splits_are_scored_in_isolation() {
        let d = data(serde_json::json!({
            "format": "atlas-eval/v1",
            "queries": [
                {"id": "qd", "class": "code", "split": "dev"},
                {"id": "qt", "class": "code", "split": "test"}
            ],
            "qrels": [
                {"query": "qd", "doc": "d1", "grade": 1},
                {"query": "qt", "doc": "d1", "grade": 1}
            ],
            "runs": [{"signal": "dense", "results": {"qd": ["d1"], "qt": ["x"]}}]
        }));
        let dev = evaluate(&d, Split::Dev).unwrap();
        assert_eq!(dev.signals[0].per_query.len(), 1);
        assert_eq!(dev.signals[0].per_query[0].query, "qd");
        assert_eq!(dev.signals[0].overall.ndcg10, 1.0);
        let test = evaluate(&d, Split::Test).unwrap();
        assert_eq!(test.signals[0].per_query[0].query, "qt");
        assert_eq!(test.signals[0].overall.ndcg10, 0.0);
    }

    // Oracle: the #5741 rule itself — the held-out test split is
    // scored once, never tuned on. Parameter selection must refuse a
    // test report and accept the same data's dev report.
    #[test]
    fn held_out_test_split_is_refused_for_parameter_selection() {
        let d = data(serde_json::json!({
            "format": "atlas-eval/v1",
            "queries": [
                {"id": "qd", "class": "code", "split": "dev"},
                {"id": "qt", "class": "code", "split": "test"}
            ],
            "qrels": [
                {"query": "qd", "doc": "d1", "grade": 1},
                {"query": "qt", "doc": "d1", "grade": 1}
            ],
            "runs": [{"signal": "dense", "results": {"qd": ["d1"], "qt": ["d1"]}}]
        }));
        let test = evaluate(&d, Split::Test).unwrap();
        assert!(select_best_signal(&test, Metric::Ndcg10).is_err());
        let dev = evaluate(&d, Split::Dev).unwrap();
        assert_eq!(
            select_best_signal(&dev, Metric::Ndcg10).unwrap(),
            "dense".to_owned()
        );
    }

    // Oracle: the format rules in the module docs, one violation each.
    #[test]
    fn validation_rejects_format_violations() {
        let base = serde_json::json!({
            "format": "atlas-eval/v1",
            "queries": [{"id": "q1", "class": "code", "split": "dev"}],
            "qrels": [{"query": "q1", "doc": "d1", "grade": 1}],
            "runs": [{"signal": "dense", "results": {"q1": ["d1"]}}]
        });
        assert!(EvalData::from_json(&base.to_string()).is_ok());

        let mut wrong_format = base.clone();
        wrong_format["format"] = serde_json::json!("atlas-eval/v99");
        assert!(EvalData::from_json(&wrong_format.to_string()).is_err());

        let mut dup_ranking = base.clone();
        dup_ranking["runs"][0]["results"]["q1"] = serde_json::json!(["d1", "d1"]);
        assert!(EvalData::from_json(&dup_ranking.to_string()).is_err());

        let mut unknown_qrel = base.clone();
        unknown_qrel["qrels"][0]["query"] = serde_json::json!("nope");
        assert!(EvalData::from_json(&unknown_qrel.to_string()).is_err());

        let mut huge_grade = base.clone();
        huge_grade["qrels"][0]["grade"] = serde_json::json!(1024);
        assert!(EvalData::from_json(&huge_grade.to_string()).is_err());

        let mut dup_qrel = base.clone();
        dup_qrel["qrels"] = serde_json::json!([
            {"query": "q1", "doc": "d1", "grade": 1},
            {"query": "q1", "doc": "d1", "grade": 2}
        ]);
        assert!(EvalData::from_json(&dup_qrel.to_string()).is_err());
    }

    // Oracle: issue #6390 — the raw document level of the format. A
    // repeated query key inside a run's `results` object must be
    // rejected naming the signal and the query; serde_json's map
    // collapse must not get to silently keep the last ranking. Raw
    // text is required: `serde_json::json!` cannot express duplicate
    // object keys.
    #[test]
    fn duplicate_results_key_is_rejected_from_raw_text() {
        let raw = r#"{
            "format": "atlas-eval/v1",
            "queries": [{"id": "q1", "class": "code", "split": "dev"}],
            "qrels": [{"query": "q1", "doc": "d1", "grade": 1}],
            "runs": [{"signal": "dense", "results": {"q1": ["d1"], "q1": ["d2"]}}]
        }"#;
        let err = EvalData::from_json(raw).expect_err("duplicate results key must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("duplicate results key"),
            "error names the violation: {msg}"
        );
        assert!(msg.contains("dense"), "error names the signal: {msg}");
        assert!(msg.contains("q1"), "error names the query: {msg}");

        // The same document with the duplicate removed parses and
        // scores exactly as before.
        let clean = raw.replace(r#", "q1": ["d2"]"#, "");
        let d = EvalData::from_json(&clean).expect("clean document parses");
        let report = evaluate(&d, Split::Dev).unwrap();
        assert_eq!(report.signals[0].overall.ndcg10, 1.0);
    }

    // Oracle: the documented missing-run rule — a signal with no
    // entry for a query scores that query as an empty ranking.
    #[test]
    fn missing_query_in_run_scores_as_empty() {
        let d = data(serde_json::json!({
            "format": "atlas-eval/v1",
            "queries": [{"id": "q1", "class": "code", "split": "dev"}],
            "qrels": [{"query": "q1", "doc": "d1", "grade": 1}],
            "runs": [{"signal": "sparse", "results": {}}]
        }));
        let report = evaluate(&d, Split::Dev).unwrap();
        let signal = &report.signals[0];
        assert_eq!(signal.queries_scored, 1);
        assert_eq!(signal.overall.ndcg10, 0.0);
        assert_eq!(signal.overall.recall1000, 0.0);
        assert!(signal.per_query[0].top1000.is_empty());
    }

    // Oracle: the report's own preserved evidence. A relevant doc
    // at rank 120 must survive in the per-query record (top-1000,
    // not top-100) together with the judged grades, and recomputing
    // the metrics from that record alone must reproduce the reported
    // values (Codex finding on PR #6262: report-only recomputation
    // was impossible when only the top 100 was kept and the qrels
    // were dropped).
    #[test]
    fn report_preserves_evidence_for_recomputation() {
        let mut docs: Vec<String> = (0..150).map(|i| format!("x{i}")).collect();
        docs[49] = "rel-a".to_owned();
        docs[119] = "rel-b".to_owned();
        let results = serde_json::Value::Array(docs.iter().map(|d| serde_json::json!(d)).collect());
        let d = data(serde_json::json!({
            "format": "atlas-eval/v1",
            "queries": [{"id": "q1", "class": "code", "split": "dev"}],
            "qrels": [
                {"query": "q1", "doc": "rel-a", "grade": 2},
                {"query": "q1", "doc": "rel-b", "grade": 1}
            ],
            "runs": [{"signal": "dense", "results": {"q1": results}}]
        }));
        let report = evaluate(&d, Split::Dev).unwrap();
        let pq = &report.signals[0].per_query[0];
        assert_eq!(pq.top1000.len(), 150);
        assert_eq!(pq.grades.len(), 2);
        assert_eq!(pq.recall100, 0.5);
        assert_eq!(pq.recall1000, 1.0);
        // Recompute from the preserved record alone.
        let (_, m) = query_metrics(&pq.top1000, &pq.grades);
        assert!((m.ndcg10 - pq.ndcg10).abs() < 1e-6, "ndcg recompute");
        assert!((m.recall100 - pq.recall100).abs() < 1e-12);
        assert!((m.recall1000 - pq.recall1000).abs() < 1e-12);
        assert!((m.mrr10 - pq.mrr10).abs() < 1e-12);
    }

    // Deterministic xorshift for the invariant sweep (no new dev
    // dependency; the generator is part of the test, seeded fixed).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    // Oracle: the metric definitions as invariants, checked over
    // 2000 generated (qrels, ranking) pairs — every metric stays in
    // [0, 1], Recall@100 ≤ Recall@1000 (monotone in k), and the
    // ideal ordering of the judged grades always scores nDCG 1.
    #[test]
    fn metric_invariants_hold_over_generated_inputs() {
        let mut rng = Rng(0x5741_2026_1105);
        for case in 0..2000 {
            let n_docs = 1 + rng.below(24);
            let docs: Vec<String> = (0..n_docs).map(|i| format!("d{i}")).collect();
            let mut g: BTreeMap<String, u32> = BTreeMap::new();
            for doc in &docs {
                if rng.below(2) == 0 {
                    g.insert(doc.clone(), rng.below(4) as u32);
                }
            }
            let mut ranking = docs.clone();
            for i in (1..ranking.len()).rev() {
                let j = rng.below(i + 1);
                ranking.swap(i, j);
            }
            let (relevant, m) = query_metrics(&ranking, &g);
            for (name, value) in [
                ("ndcg10", m.ndcg10),
                ("recall100", m.recall100),
                ("recall1000", m.recall1000),
                ("mrr10", m.mrr10),
            ] {
                assert!(
                    (0.0..=1.0).contains(&value),
                    "case {case}: {name} = {value} outside [0, 1]"
                );
            }
            assert!(
                m.recall100 <= m.recall1000 + 1e-12,
                "case {case}: recall not monotone in k"
            );
            if relevant > 0 {
                let mut ideal: Vec<(&String, &u32)> = g.iter().collect();
                ideal.sort_by(|a, b| b.1.cmp(a.1));
                let ideal_ranking: Vec<String> =
                    ideal.iter().map(|(doc, _)| (*doc).clone()).collect();
                let (_, ideal_metrics) = query_metrics(&ideal_ranking, &g);
                assert!(
                    (ideal_metrics.ndcg10 - 1.0).abs() < 1e-9,
                    "case {case}: ideal ranking ndcg = {}",
                    ideal_metrics.ndcg10
                );
            }
        }
    }

    // Oracle: determinism + the fixture's own hand-checkable shape.
    // The checked-in smoke fixture (synthetic, NOT evaluation
    // evidence) must serialize byte-identically across repeated
    // evaluations and across input-order permutations, and its dev
    // split must select dense by nDCG (dense dev per-query nDCG:
    // 0.796708 for q-code-1 and 1.0 for q-docs-1; bm25: 1.0 and
    // 0.693426 — means 0.898354 vs 0.846713).
    #[test]
    fn fixture_smoke_is_deterministic_and_selects_dense_on_dev() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/eval-smoke.json"
        );
        let text = std::fs::read_to_string(path).expect("smoke fixture exists");
        let d = EvalData::from_json(&text).unwrap();
        let first = serde_json::to_string(&evaluate(&d, Split::Dev).unwrap()).unwrap();
        let second = serde_json::to_string(&evaluate(&d, Split::Dev).unwrap()).unwrap();
        assert_eq!(first, second, "same input must serialize identically");

        let mut shuffled = d.clone();
        shuffled.runs.reverse();
        shuffled.qrels.reverse();
        shuffled.queries.reverse();
        let third = serde_json::to_string(&evaluate(&shuffled, Split::Dev).unwrap()).unwrap();
        assert_eq!(first, third, "input order must not change the report");

        let dev = evaluate(&d, Split::Dev).unwrap();
        assert_eq!(
            select_best_signal(&dev, Metric::Ndcg10).unwrap(),
            "dense".to_owned()
        );
        let test = evaluate(&d, Split::Test).unwrap();
        assert_eq!(test.signals[0].queries_total, 2);
        assert!(select_best_signal(&test, Metric::Ndcg10).is_err());
    }
}
