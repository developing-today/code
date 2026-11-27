//! The classification operating procedure (#12405): the
//! unclassified-remainder sweep and the schema-change review from
//! the classification design
//! (`doc/2026-11-21T21-32-00Z_design_atlas_classification_model_v1.md`,
//! §5.1/§5.2), built on the per-embed decision records of #12402
//! ([`crate::classification`]).
//!
//! The sweep enumerates existing embeds whose classification is
//! absent, still at the locked `secret-bearing` default, or
//! previously left unresolved, and writes versioned, attributable
//! [`ClassificationRecord`] decisions for them. It never rebuilds
//! embeddings: chunk rows and vector rows are read but never
//! written, and every run proves that with before/after content
//! hashes recorded in its manifest. Already-classified embeds are
//! never touched. A run is deterministic (candidates in ascending
//! embed order, all timestamps and identities supplied by the run
//! config), resumable (decisions are written one by one; re-running
//! skips what a previous run decided and re-derives the rest
//! identically), and idempotent (a re-run over a fully decided
//! corpus writes nothing).
//!
//! Decisions are made by the pure [`decide`] function over an
//! assembled [`CandidateContext`]: embed kind, capture date, source
//! session, session-label sensitivity/confidence, topics/projects,
//! associated issue/PR/commit/file/design refs and link chains from
//! the generic label store, structure-backed detector hits over the
//! embed text, and the reuse floor (the most restrictive decided
//! classification of byte-identical content anywhere in the
//! corpus — design decision 1: reuse never lowers). Absent or
//! contradictory context leaves the embed at the locked default
//! with an explicit unresolved reason recorded on the decision;
//! the sweep never infers `shareable` from a filename, an age, or
//! a missing detector hit, and it never writes a `shareable`
//! decision at all: session/legacy evidence is a ceiling input,
//! not per-embed disclosure intent (design §2), so `shareable`
//! proposals are capped at `personal` and a per-embed reviewer
//! decision remains the only path to `shareable`.
//!
//! The schema-change review ([`review_schema_change`]) compares
//! two schema documents section by section, examines changed
//! sections only by default (a declared breaking-change event
//! examines everything, design §5.2's exception), and produces
//! proposals only. [`apply_schema_review`] writes a proposal only
//! with a reviewed disposition naming a reviewer and rationale —
//! a lowering without one is refused, never applied silently.
//!
//! Scope: this operates on a local corpus SQLite file (publish
//! copies and shards). The records table it maintains is
//! `classification_records` in that file; the JSON record is
//! authoritative and the denormalized columns are write-time
//! indexes derived from it.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use rusqlite::{Connection, OpenFlags, params};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json, json};
use sha2::{Digest, Sha256};

use crate::classification::{ClassificationDisposition, ClassificationRecord};
use crate::error::{Error, Result};
use crate::filters::Sensitivity;
use crate::types::DEFAULT_VEC_TABLE;

/// The run manifest's format tag.
pub const RUN_FORMAT: &str = "atlas-classify-run/1";
/// The schema-review manifest's format tag.
pub const REVIEW_FORMAT: &str = "atlas-classify-schema-review/1";
/// The schema-apply report's format tag.
pub const APPLY_FORMAT: &str = "atlas-classify-schema-apply/1";

/// The records table the sweep maintains in the corpus database.
pub const RECORDS_TABLE: &str = "classification_records";

/// DDL for the records table. `record_json` is authoritative; the
/// other columns are indexes derived from the parsed record at
/// write time.
pub const RECORDS_DDL: &str = "CREATE TABLE IF NOT EXISTS classification_records (\
    embed_id TEXT PRIMARY KEY, \
    record_json TEXT NOT NULL, \
    sensitivity TEXT NOT NULL, \
    schema_version INTEGER NOT NULL, \
    decided_at TEXT NOT NULL)";

/// Provenance `method` for sweep decisions.
pub const METHOD_SWEEP: &str = "remainder-sweep";
/// Provenance `method` for schema-review-applied decisions.
pub const METHOD_SCHEMA_REVIEW: &str = "schema-review";
/// The method version both methods currently write.
pub const METHOD_VERSION: &str = "1";
/// The classification schema version decisions are written under
/// unless a run overrides it.
pub const CURRENT_SCHEMA_VERSION: u64 = 1;

/// The default relatedness window in days (design §5.1).
pub const DEFAULT_WINDOW_DAYS: i64 = 90;

/// The corpus-chunk embed-id form: `chunk:<chunks.id>`.
pub const EMBED_ID_PREFIX: &str = "chunk:";

/// The current classification schema (v1), in the schema-document
/// shape [`review_schema_change`] consumes: a `version` plus named
/// `sections`. It mirrors [`crate::classification`]'s strict value
/// set and the design's §2 semantics; #9095 owns the doctrine and
/// may publish its own artifact, which operators can render into
/// this shape for review.
pub const CURRENT_SCHEMA_JSON: &str = r#"{
  "version": 1,
  "sections": {
    "sensitivity_values": {
      "values": ["shareable", "personal", "local-only", "secret-bearing"],
      "order": "shareable < personal < local-only < secret-bearing",
      "missing_or_invalid": "secret-bearing"
    },
    "dispositions": {
      "values": ["included", "excluded_personal_life"]
    },
    "provenance_fields": {
      "required": ["actor", "decided_at", "method", "method_version", "config_version", "evidence_refs"],
      "optional": ["previous_decision_id"]
    },
    "record_fields": {
      "required": ["schema_version", "embed_id", "sensitivity", "provenance"],
      "optional": ["disposition"],
      "unknown_fields": "preserved in extras"
    }
  }
}"#;

/// SHA-256 hex digest, in the style of `manifest::Manifest`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Days since the civil epoch for a `YYYY-MM-DD` date (Howard
/// Hinnant's algorithm), for window arithmetic over the corpus's
/// ISO capture timestamps without a calendar dependency.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// The inverse of [`days_from_civil`]: `(year, month, day)`.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Parse the leading `YYYY-MM-DD` of an ISO timestamp into a day
/// number. Anything else reads as `None` (unknown capture date).
fn parse_day(ts: &str) -> Option<i64> {
    let b = ts.as_bytes();
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let y: i64 = ts.get(0..4)?.parse().ok()?;
    let m: i64 = ts.get(5..7)?.parse().ok()?;
    let d: i64 = ts.get(8..10)?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(days_from_civil(y, m, d))
}

/// Render epoch seconds as an ISO-8601 UTC timestamp (the CLI's
/// default `now`).
pub fn iso_now(epoch_secs: u64) -> String {
    let days = (epoch_secs / 86_400) as i64;
    let secs = epoch_secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// Structure-backed secret patterns (design §5.3: checksum/
/// structure-backed findings are high-confidence candidates).
/// Returns pattern names only — matched text is never copied into
/// decisions, manifests, or logs.
pub fn detect_secret_patterns(text: &str) -> Vec<&'static str> {
    let mut hits: Vec<&'static str> = Vec::new();
    let mut push = |name: &'static str| {
        if !hits.contains(&name) {
            hits.push(name);
        }
    };
    if text.contains("-----BEGIN ") && text.contains(" PRIVATE KEY-----") {
        push("private-key-pem");
    }
    for (prefix, name) in [
        ("github_pat_", "github-token"),
        ("ghp_", "github-token"),
        ("gho_", "github-token"),
        ("ghu_", "github-token"),
        ("ghs_", "github-token"),
        ("ghr_", "github-token"),
        ("xoxb-", "slack-token"),
        ("xoxa-", "slack-token"),
        ("xoxp-", "slack-token"),
        ("xoxr-", "slack-token"),
        ("xoxs-", "slack-token"),
        ("sk-ant-", "anthropic-key"),
        ("sk-proj-", "openai-key"),
    ] {
        if has_token_shape(text, prefix, 16) {
            push(name);
        }
    }
    if has_aws_access_key_id(text) {
        push("aws-access-key-id");
    }
    if has_token_shape(text, "AIza", 35) {
        push("google-api-key");
    }
    hits
}

/// A prefix followed by at least `min` token characters
/// (`[A-Za-z0-9_-]`), the structural shape of an issued token.
fn has_token_shape(text: &str, prefix: &str, min: usize) -> bool {
    text.match_indices(prefix).any(|(i, _)| {
        let rest = &text[i + prefix.len()..];
        rest.chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .count()
            >= min
    })
}

/// `AKIA`/`ASIA` plus exactly 16 `[A-Z0-9]` characters, the AWS
/// access-key-id shape.
fn has_aws_access_key_id(text: &str) -> bool {
    ["AKIA", "ASIA"].iter().any(|prefix| {
        text.match_indices(prefix).any(|(i, _)| {
            let rest = &text[i + prefix.len()..];
            rest.chars()
                .take(16)
                .filter(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                .count()
                == 16
        })
    })
}

/// Why a candidate was enumerated (its state before this run).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PriorState {
    /// No classification record and no legacy sensitivity: the
    /// embed has never been classified.
    Absent,
    /// No classification record, but legacy sensitivity columns
    /// exist: the embed rides the locked default / session-label
    /// path with no per-embed provenance.
    DefaultState,
    /// A record exists but was left unresolved by an earlier run.
    Unresolved,
}

impl PriorState {
    pub fn name(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::DefaultState => "default_state",
            Self::Unresolved => "unresolved",
        }
    }
}

/// What the sweep decided for one candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionOutcome {
    /// A structure-backed detector hit confirms secret content.
    ConfirmedSecret,
    /// Positive personal-life evidence: excluded disposition.
    ExcludedPersonalLife,
    /// Classified from label/legacy evidence (possibly floored).
    Classified,
    /// The reuse floor, not the local evidence, set the result.
    HeldByReuse,
    /// Evidence absent or contradictory: stays at the locked
    /// default with an explicit unresolved reason.
    Unresolved,
}

impl DecisionOutcome {
    pub fn name(self) -> &'static str {
        match self {
            Self::ConfirmedSecret => "confirmed_secret",
            Self::ExcludedPersonalLife => "excluded_personal_life",
            Self::Classified => "classified",
            Self::HeldByReuse => "held_by_reuse",
            Self::Unresolved => "unresolved",
        }
    }
}

/// Unresolved reason: no label, associations, or other evidence.
pub const REASON_NO_CONTEXT: &str = "no_classification_context";
/// Unresolved reason: a present safety value cannot be read.
pub const REASON_CONTRADICTORY: &str = "contradictory_context";
/// Unresolved reason: label evidence exists but is too weak.
pub const REASON_WEAK_EVIDENCE: &str = "weak_evidence";

/// The assembled context for one candidate embed (design §5.1:
/// only available context; links are evidence, never copied
/// content). Assembled by [`run_classification`]; [`decide`] is
/// pure over it.
#[derive(Debug, Clone, Default)]
pub struct CandidateContext {
    pub embed_id: String,
    /// The embed's own legacy sensitivity column, verbatim.
    pub chunk_legacy_sensitivity: Option<String>,
    /// The session label's sensitivity, verbatim.
    pub label_sensitivity: Option<String>,
    /// The session label's confidence, verbatim.
    pub label_confidence: Option<String>,
    /// The session label's kind.
    pub kind: Option<String>,
    pub topics: Vec<String>,
    pub projects: Vec<String>,
    /// Evidence refs from the generic label store: associated
    /// issues, PRs, commits, files, designs, and link chains.
    pub associations: Vec<String>,
    /// Structure-backed detector pattern names over the text.
    pub detector_hits: Vec<&'static str>,
    /// The most restrictive decided classification of
    /// byte-identical content in this corpus, if any.
    pub reuse_floor: Option<Sensitivity>,
    /// The embed id that established the reuse floor.
    pub reuse_floor_ref: Option<String>,
}

/// One sweep decision: the record fields to write, plus the
/// outcome accounting the manifest reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub sensitivity: Sensitivity,
    pub disposition: Option<ClassificationDisposition>,
    pub outcome: DecisionOutcome,
    pub unresolved_reason: Option<&'static str>,
    pub evidence_refs: Vec<String>,
}

/// The classifier: a deterministic rule pass over the assembled
/// context. Oracles are the design's §8 guarantees 2, 4, and 6 —
/// no evidence keeps the locked default, reuse never lowers, and
/// `shareable` is never produced from session-level evidence.
pub fn decide(ctx: &CandidateContext) -> Decision {
    // `finish` assembles a decision from the evidence accumulated
    // so far (`base`) plus outcome-specific refs (`extra`).
    let finish = |sensitivity: Sensitivity,
                  disposition: Option<ClassificationDisposition>,
                  outcome: DecisionOutcome,
                  unresolved_reason: Option<&'static str>,
                  base: Vec<String>,
                  mut extra: Vec<String>| {
        let mut evidence_refs = base;
        evidence_refs.append(&mut extra);
        Decision {
            sensitivity,
            disposition,
            outcome,
            unresolved_reason,
            evidence_refs,
        }
    };
    let no_evidence: Vec<String> = Vec::new();

    // 1. A structure-backed detector hit is decisive: the embed
    //    is confirmed secret-bearing regardless of label claims.
    if !ctx.detector_hits.is_empty() {
        return finish(
            Sensitivity::SecretBearing,
            None,
            DecisionOutcome::ConfirmedSecret,
            None,
            no_evidence,
            ctx.detector_hits
                .iter()
                .map(|h| format!("detector:{h}"))
                .collect(),
        );
    }

    // 2. A present-but-unreadable safety value (chunk or label)
    //    is contradictory evidence: fail closed, unresolved.
    //    Published labels dual-read canonical spellings (#13506 stage 2).
    let chunk_legacy = ctx
        .chunk_legacy_sensitivity
        .as_deref()
        .filter(|s| !s.trim().is_empty());
    let label_sens = ctx
        .label_sensitivity
        .as_deref()
        .filter(|s| !s.trim().is_empty());
    if chunk_legacy.is_some_and(|s| Sensitivity::from_published_name(s).is_none())
        || label_sens.is_some_and(|s| Sensitivity::from_published_name(s).is_none())
    {
        return finish(
            Sensitivity::SecretBearing,
            None,
            DecisionOutcome::Unresolved,
            Some(REASON_CONTRADICTORY),
            no_evidence,
            Vec::new(),
        );
    }
    let chunk_legacy = chunk_legacy.and_then(Sensitivity::from_published_name);
    let label_sens = label_sens.and_then(Sensitivity::from_published_name);

    // 3. Positive personal-life evidence excludes the embed.
    //    Sensitivity stays at the locked default; the exclusion
    //    disposition is the decision (design §2).
    let personal_life = ctx
        .kind
        .as_deref()
        .is_some_and(|k| k == "personal-life" || k == "personal_life")
        || ctx
            .topics
            .iter()
            .any(|t| t == "personal-life" || t == "personal_life");
    if personal_life {
        return finish(
            Sensitivity::SecretBearing,
            Some(ClassificationDisposition::ExcludedPersonalLife),
            DecisionOutcome::ExcludedPersonalLife,
            None,
            no_evidence,
            vec!["label:personal-life".to_string()],
        );
    }

    // 4. Evidence proposal: the session label is a ceiling input;
    //    the embed's own legacy value floors it (it is the more
    //    specific recorded tier). `shareable` is capped at
    //    `personal`: session/legacy evidence never proves
    //    per-embed disclosure intent.
    let mut proposal: Option<Sensitivity> = None;
    let mut evidence: Vec<String> = Vec::new();
    if let Some(label) = label_sens {
        let weak = ctx
            .label_confidence
            .as_deref()
            .is_some_and(|c| !c.eq_ignore_ascii_case("high"));
        if weak {
            return finish(
                Sensitivity::SecretBearing,
                None,
                DecisionOutcome::Unresolved,
                Some(REASON_WEAK_EVIDENCE),
                no_evidence,
                Vec::new(),
            );
        }
        proposal = Some(label);
        evidence.push("label:session_sensitivity".to_string());
    }
    if let Some(chunk) = chunk_legacy {
        proposal = Some(match proposal {
            Some(p) => p.max(chunk),
            None => chunk,
        });
        evidence.push("legacy:chunk_sensitivity".to_string());
    }
    let Some(mut proposed) = proposal else {
        // 5. No classification evidence at all: the locked
        //    default stands, with the reason on record.
        return finish(
            Sensitivity::SecretBearing,
            None,
            DecisionOutcome::Unresolved,
            Some(REASON_NO_CONTEXT),
            no_evidence,
            Vec::new(),
        );
    };
    if proposed == Sensitivity::Shareable {
        proposed = Sensitivity::Personal;
        evidence.push("cap:shareable_requires_per_embed_review".to_string());
    }
    evidence.extend(ctx.associations.iter().cloned());

    // 6. The reuse floor: byte-identical content keeps the most
    //    restrictive classification ever established for it.
    if let Some(floor) = ctx.reuse_floor
        && floor > proposed
    {
        let mut refs = vec![];
        if let Some(r) = &ctx.reuse_floor_ref {
            refs.push(format!("reuse_floor:{r}"));
        }
        return finish(
            floor,
            None,
            DecisionOutcome::HeldByReuse,
            None,
            evidence,
            refs,
        );
    }
    finish(
        proposed,
        None,
        DecisionOutcome::Classified,
        None,
        evidence,
        Vec::new(),
    )
}

/// A sweep run's configuration. Every field lands in the run
/// manifest and in each decision's provenance, so a run is fully
/// attributable and reproducible.
#[derive(Debug, Clone)]
pub struct RunConfig {
    /// Provenance actor for written decisions.
    pub actor: String,
    /// Decision timestamp (`decided_at`) for this run.
    pub now: String,
    /// Relatedness window in days (design §5.1 default: 90).
    pub window_days: i64,
    /// Schema version written decisions carry.
    pub schema_version: u64,
    /// Cap on decisions written this run (interruption/resume
    /// control); `None` runs to completion.
    pub max_decisions: Option<usize>,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            actor: "atlas-classify/remainder-sweep".to_string(),
            now: String::new(),
            window_days: DEFAULT_WINDOW_DAYS,
            schema_version: CURRENT_SCHEMA_VERSION,
            max_decisions: None,
        }
    }
}

impl RunConfig {
    /// SHA-256 over the canonical config serialization — the
    /// `config_version` provenance decisions carry, in the
    /// manifest module's `sha256:` pattern.
    pub fn config_hash(&self) -> String {
        let canonical = format!(
            "atlas-classify/1\nactor={}\nwindow_days={}\nschema_version={}\nmethod={}@{}\n",
            self.actor, self.window_days, self.schema_version, METHOD_SWEEP, METHOD_VERSION,
        );
        format!("sha256:{}", sha256_hex(canonical.as_bytes()))
    }
}

/// One decision as the run manifest reports it. Carries no embed
/// text and no secret material — ids, refs, and outcomes only.
#[derive(Debug, Clone, Serialize)]
pub struct DecisionEntry {
    pub embed_id: String,
    pub prior_state: PriorState,
    pub outcome: DecisionOutcome,
    pub sensitivity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disposition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unresolved_reason: Option<String>,
    pub evidence_refs: Vec<String>,
    /// Sessions related by shared project inside the window.
    pub related_sessions: usize,
    /// Most restrictive decided classification among related
    /// sessions' embeds inside the window, if any (context only;
    /// never a floor — only byte-identical reuse binds).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related_classified_max: Option<String>,
    pub written: bool,
}

/// The run manifest: counts by prior state and outcome, the
/// declared window, vector/chunk preservation hashes, and every
/// decision. This is the acceptance artifact of a run.
#[derive(Debug, Clone, Serialize)]
pub struct RunManifest {
    pub format: &'static str,
    pub run_id: String,
    pub mode: &'static str,
    pub corpus: String,
    pub actor: String,
    pub decided_at: String,
    pub method: &'static str,
    pub method_version: &'static str,
    pub config_version: String,
    pub schema_version: u64,
    /// The declared relatedness window (design §5.1).
    pub window: WindowRecord,
    pub counts: RunCounts,
    /// Hash of the chunk rows before the run.
    pub chunks_hash_before: String,
    /// Hash of the chunk rows after the run (commit mode).
    pub chunks_hash_after: String,
    /// Hash of the vector rows before the run.
    pub vectors_hash_before: String,
    /// Hash of the vector rows after the run (commit mode).
    pub vectors_hash_after: String,
    /// True when both preservation hashes are unchanged.
    pub vectors_preserved: bool,
    pub decisions: Vec<DecisionEntry>,
}

/// The declared relatedness window, recorded on every run.
#[derive(Debug, Clone, Serialize)]
pub struct WindowRecord {
    pub days: i64,
    pub scope: &'static str,
    pub basis: &'static str,
}

/// Result counts: by prior state, by outcome, and write totals.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RunCounts {
    pub total_embeds: usize,
    pub already_classified: usize,
    pub candidates: usize,
    pub by_prior_state: BTreeMap<String, usize>,
    pub by_outcome: BTreeMap<String, usize>,
    pub written: usize,
}

/// One corpus chunk row, with its session context joined.
#[derive(Clone)]
struct ChunkRow {
    id: i64,
    text: String,
    content_hash: String,
    legacy_sensitivity: Option<String>,
    session_id: String,
    capture_ts: Option<String>,
}

/// One session label row (the fields the sweep reads).
#[derive(Default, Clone)]
struct LabelRow {
    kind: Option<String>,
    topics: Vec<String>,
    projects: Vec<String>,
    sensitivity: Option<String>,
    confidence: Option<String>,
}

/// Parse a label list column tolerantly: JSON list first (the
/// publish fixture shape), comma-split second (the Python's CSV
/// shape). Non-list content reads as no entries.
fn parse_label_list(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw else { return Vec::new() };
    let raw = raw.trim();
    if raw.is_empty() {
        return Vec::new();
    }
    if let Ok(Json::Array(items)) = serde_json::from_str::<Json>(raw) {
        return items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
    }
    crate::filters::split_csv(Some(raw))
}

/// Generic-label dimensions that carry candidate associations
/// (design §5.1: issues, PRs, commits, files, designs, link
/// chains). Values are taken verbatim as evidence refs.
const ASSOCIATION_DIMENSIONS: [&str; 6] = ["issues", "prs", "commits", "files", "designs", "links"];

fn table_exists(con: &Connection, name: &str) -> Result<bool> {
    let n: i64 = con
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            params![name],
            |r| r.get(0),
        )
        .map_err(|e| Error::backend(format!("classify: table probe: {e}")))?;
    Ok(n > 0)
}

fn column_exists(con: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = con
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| Error::backend(format!("classify: pragma {table}: {e}")))?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(|e| Error::backend(format!("classify: pragma {table}: {e}")))?;
    for row in rows {
        if row.map_err(|e| Error::backend(e.to_string()))? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// A loaded classification record plus its stored JSON (the
/// digest source for `previous_decision_id` chains).
struct StoredRecord {
    record: ClassificationRecord,
    json_text: String,
}

fn load_records(con: &Connection) -> Result<BTreeMap<String, StoredRecord>> {
    let mut out = BTreeMap::new();
    if !table_exists(con, RECORDS_TABLE)? {
        return Ok(out);
    }
    let mut stmt = con
        .prepare(&format!(
            "SELECT embed_id, record_json FROM {RECORDS_TABLE} ORDER BY embed_id"
        ))
        .map_err(|e| Error::backend(format!("classify: read records: {e}")))?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| Error::backend(format!("classify: read records: {e}")))?;
    for row in rows {
        let (embed_id, json_text) =
            row.map_err(|e| Error::backend(format!("classify: read records: {e}")))?;
        let value: Json = serde_json::from_str(&json_text).map_err(|e| {
            Error::fatal(format!(
                "classify: stored record for {embed_id} is not valid JSON: {e}"
            ))
        })?;
        let record = ClassificationRecord::parse(value).map_err(|e| {
            Error::fatal(format!(
                "classify: stored record for {embed_id} fails strict parsing: {e}"
            ))
        })?;
        out.insert(embed_id, StoredRecord { record, json_text });
    }
    Ok(out)
}

/// Content hash over chunk rows (id, content hash, legacy
/// sensitivity, text digest) — proof the sweep left the corpus
/// content untouched.
fn chunks_hash(con: &Connection) -> Result<String> {
    let mut stmt = con
        .prepare("SELECT id, content_hash, sensitivity, text FROM chunks ORDER BY id")
        .map_err(|e| Error::backend(format!("classify: chunk hash: {e}")))?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| Error::backend(format!("classify: chunk hash: {e}")))?;
    let mut h = Sha256::new();
    for row in rows {
        let (id, content_hash, sens, text) =
            row.map_err(|e| Error::backend(format!("classify: chunk hash: {e}")))?;
        h.update(
            format!(
                "{id}|{}|{}|{}\n",
                content_hash.as_deref().unwrap_or(""),
                sens.as_deref().unwrap_or(""),
                sha256_hex(text.as_bytes())
            )
            .as_bytes(),
        );
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Content hash over the vector table (or a fixed sentinel when
/// the corpus carries none).
fn vectors_hash(con: &Connection) -> Result<String> {
    if !table_exists(con, DEFAULT_VEC_TABLE)? {
        return Ok("absent".to_string());
    }
    let mut stmt = con
        .prepare(&format!(
            "SELECT chunk_id, vec FROM {DEFAULT_VEC_TABLE} ORDER BY chunk_id"
        ))
        .map_err(|e| Error::backend(format!("classify: vector hash: {e}")))?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))
        .map_err(|e| Error::backend(format!("classify: vector hash: {e}")))?;
    let mut h = Sha256::new();
    for row in rows {
        let (id, vec) = row.map_err(|e| Error::backend(format!("classify: vector hash: {e}")))?;
        h.update(format!("{id}|{}|", vec.len()).as_bytes());
        h.update(&vec);
        h.update(b"\n");
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// The record's digest, the `previous_decision_id` chain link.
fn record_digest(json_text: &str) -> String {
    format!("sha256:{}", sha256_hex(json_text.as_bytes()))
}

/// Is this stored record an unresolved sweep outcome (a candidate
/// again on the next run)?
fn is_unresolved(record: &ClassificationRecord) -> bool {
    record
        .extras
        .get("unresolved_reason")
        .is_some_and(|v| v.as_str().is_some_and(|s| !s.is_empty()))
}

/// Run the remainder sweep over the corpus at `db_path`.
///
/// Dry-run (`commit == false`) opens the corpus read-only and
/// writes nothing anywhere. Commit mode writes one decision
/// record per candidate (bounded by `config.max_decisions`) into
/// `classification_records`, in ascending embed order, each in
/// its own transaction — an interrupted run leaves a decided
/// prefix, and re-running resumes exactly there.
pub fn run_classification(db_path: &Path, commit: bool, config: &RunConfig) -> Result<RunManifest> {
    if !db_path.exists() {
        return Err(Error::fatal(format!(
            "classify: corpus DB not found: {}",
            db_path.display()
        )));
    }
    let con = if commit {
        Connection::open(db_path)
            .map_err(|e| Error::backend(format!("classify: open {}: {e}", db_path.display())))?
    } else {
        Connection::open_with_flags(
            format!("file:{}?mode=ro", db_path.display()),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )
        .map_err(|e| Error::backend(format!("classify: open {}: {e}", db_path.display())))?
    };
    if commit {
        con.execute_batch(RECORDS_DDL)
            .map_err(|e| Error::backend(format!("classify: create records table: {e}")))?;
    }

    let chunks_before = chunks_hash(&con)?;
    let vectors_before = vectors_hash(&con)?;
    let records = load_records(&con)?;

    // --- enumerate embeds with their session context ----------
    let has_chunk_sens = column_exists(&con, "chunks", "sensitivity")?;
    let sens_col = if has_chunk_sens {
        "c.sensitivity"
    } else {
        "NULL"
    };
    let chunks: Vec<ChunkRow> = {
        let mut stmt = con
            .prepare(&format!(
                "SELECT c.id, c.text, c.content_hash, {sens_col}, m.session_id, s.created_at \
                 FROM chunks c \
                 JOIN messages m ON m.id = c.message_id \
                 JOIN sessions s ON s.id = m.session_id \
                 ORDER BY c.id"
            ))
            .map_err(|e| Error::backend(format!("classify: enumerate chunks: {e}")))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ChunkRow {
                    id: r.get(0)?,
                    text: r.get(1)?,
                    content_hash: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    legacy_sensitivity: r.get(3)?,
                    session_id: r.get(4)?,
                    capture_ts: r.get(5)?,
                })
            })
            .map_err(|e| Error::backend(format!("classify: enumerate chunks: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| Error::backend(format!("classify: enumerate: {e}")))?);
        }
        out
    };

    // --- session labels ----------------------------------------
    let mut labels: HashMap<String, LabelRow> = HashMap::new();
    if table_exists(&con, "session_labels")? {
        let mut stmt = con
            .prepare(
                "SELECT session_id, kind, topics, projects, sensitivity, confidence \
                 FROM session_labels",
            )
            .map_err(|e| Error::backend(format!("classify: read labels: {e}")))?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    LabelRow {
                        kind: r.get(1)?,
                        topics: parse_label_list(r.get::<_, Option<String>>(2)?.as_deref()),
                        projects: parse_label_list(r.get::<_, Option<String>>(3)?.as_deref()),
                        sensitivity: r.get(4)?,
                        confidence: r.get(5)?,
                    },
                ))
            })
            .map_err(|e| Error::backend(format!("classify: read labels: {e}")))?;
        for row in rows {
            let (sid, label) = row.map_err(|e| Error::backend(e.to_string()))?;
            labels.insert(sid, label);
        }
    }

    // --- generic-label associations ----------------------------
    let mut associations: HashMap<String, Vec<String>> = HashMap::new();
    if table_exists(&con, crate::manifest::GENERIC_LABELS_TABLE)? {
        let mut stmt = con
            .prepare(&format!(
                "SELECT session_id, dimension, values_json FROM {}",
                crate::manifest::GENERIC_LABELS_TABLE
            ))
            .map_err(|e| Error::backend(format!("classify: read associations: {e}")))?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| Error::backend(format!("classify: read associations: {e}")))?;
        for row in rows {
            let (sid, dimension, values_json) = row.map_err(|e| Error::backend(e.to_string()))?;
            if !ASSOCIATION_DIMENSIONS.contains(&dimension.as_str()) {
                continue;
            }
            if let Ok(Json::Array(items)) = serde_json::from_str::<Json>(&values_json) {
                let refs: Vec<String> = items
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect();
                if !refs.is_empty() {
                    associations.entry(sid).or_default().extend(refs);
                }
            }
        }
    }

    // --- reuse floors: decided records per content hash --------
    let chunk_by_id: HashMap<i64, &ChunkRow> = chunks.iter().map(|c| (c.id, c)).collect();
    let mut floor_by_hash: HashMap<String, (Sensitivity, String)> = HashMap::new();
    for (embed_id, stored) in &records {
        if is_unresolved(&stored.record) {
            continue;
        }
        let Some(id_str) = embed_id.strip_prefix(EMBED_ID_PREFIX) else {
            continue;
        };
        let Ok(id) = id_str.parse::<i64>() else {
            continue;
        };
        let Some(chunk) = chunk_by_id.get(&id) else {
            continue;
        };
        if chunk.content_hash.is_empty() {
            continue;
        }
        let entry = floor_by_hash
            .entry(chunk.content_hash.clone())
            .or_insert((stored.record.sensitivity, embed_id.clone()));
        if stored.record.sensitivity > entry.0 {
            *entry = (stored.record.sensitivity, embed_id.clone());
        }
    }

    // --- relatedness: sessions by shared project ---------------
    let session_projects: HashMap<String, Vec<String>> = labels
        .iter()
        .map(|(sid, l)| (sid.clone(), l.projects.clone()))
        .collect();
    let session_day: HashMap<String, Option<i64>> = chunks
        .iter()
        .map(|c| {
            (
                c.session_id.clone(),
                c.capture_ts.as_deref().and_then(parse_day),
            )
        })
        .collect();
    let decided_sensitivity: HashMap<String, Sensitivity> = records
        .iter()
        .filter(|(_, s)| !is_unresolved(&s.record))
        .map(|(id, s)| (id.clone(), s.record.sensitivity))
        .collect();
    let chunks_by_session: HashMap<String, Vec<i64>> = {
        let mut m: HashMap<String, Vec<i64>> = HashMap::new();
        for c in &chunks {
            m.entry(c.session_id.clone()).or_default().push(c.id);
        }
        m
    };
    let related_stats = |chunk: &ChunkRow| -> (usize, Option<Sensitivity>) {
        let my_projects = session_projects
            .get(&chunk.session_id)
            .cloned()
            .unwrap_or_default();
        let my_day = session_day.get(&chunk.session_id).copied().flatten();
        if my_projects.is_empty() || my_day.is_none() {
            return (0, None);
        }
        let my_day = my_day.unwrap();
        let mut count = 0usize;
        let mut max_sens: Option<Sensitivity> = None;
        for (sid, projects) in &session_projects {
            if sid == &chunk.session_id || projects.iter().all(|p| !my_projects.contains(p)) {
                continue;
            }
            let Some(day) = session_day.get(sid).copied().flatten() else {
                continue;
            };
            if (day - my_day).abs() > config.window_days {
                continue;
            }
            count += 1;
            if let Some(ids) = chunks_by_session.get(sid) {
                for id in ids {
                    let embed_id = format!("{EMBED_ID_PREFIX}{id}");
                    if let Some(s) = decided_sensitivity.get(&embed_id) {
                        max_sens = Some(match max_sens {
                            Some(m) => m.max(*s),
                            None => *s,
                        });
                    }
                }
            }
        }
        (count, max_sens)
    };

    // --- decide -------------------------------------------------
    let config_version = config.config_hash();
    let mut counts = RunCounts {
        total_embeds: chunks.len(),
        ..RunCounts::default()
    };
    let mut entries: Vec<DecisionEntry> = Vec::new();
    let mut to_write: Vec<(ChunkRow, Decision, Vec<String>, Option<String>)> = Vec::new();
    // The run digest binds the manifest to the exact candidate
    // set and prior states it decided over.
    let mut run_digest = Sha256::new();
    for chunk in &chunks {
        let embed_id = format!("{EMBED_ID_PREFIX}{}", chunk.id);
        let prior = match records.get(&embed_id) {
            Some(stored) if !is_unresolved(&stored.record) => {
                counts.already_classified += 1;
                continue;
            }
            Some(_) => PriorState::Unresolved,
            None => {
                let label = labels.get(&chunk.session_id);
                if chunk.legacy_sensitivity.is_some()
                    || label.is_some_and(|l| l.sensitivity.is_some())
                {
                    PriorState::DefaultState
                } else {
                    PriorState::Absent
                }
            }
        };
        counts.candidates += 1;
        *counts
            .by_prior_state
            .entry(prior.name().to_string())
            .or_default() += 1;
        run_digest.update(format!("{embed_id}|{}\n", prior.name()).as_bytes());

        let label = labels.get(&chunk.session_id);
        let (floor, floor_ref) = if chunk.content_hash.is_empty() {
            (None, None)
        } else {
            match floor_by_hash.get(&chunk.content_hash) {
                Some((s, r)) if *r != embed_id => (Some(*s), Some(r.clone())),
                _ => (None, None),
            }
        };
        let ctx = CandidateContext {
            embed_id: embed_id.clone(),
            chunk_legacy_sensitivity: chunk.legacy_sensitivity.clone(),
            label_sensitivity: label.and_then(|l| l.sensitivity.clone()),
            label_confidence: label.and_then(|l| l.confidence.clone()),
            kind: label.and_then(|l| l.kind.clone()),
            topics: label.map(|l| l.topics.clone()).unwrap_or_default(),
            projects: label.map(|l| l.projects.clone()).unwrap_or_default(),
            associations: associations
                .get(&chunk.session_id)
                .cloned()
                .unwrap_or_default(),
            detector_hits: detect_secret_patterns(&chunk.text),
            reuse_floor: floor,
            reuse_floor_ref: floor_ref,
        };
        let decision = decide(&ctx);
        let (related_sessions, related_max) = related_stats(chunk);
        let mut evidence_refs = vec![format!("session:{}", chunk.session_id)];
        evidence_refs.extend(decision.evidence_refs.iter().cloned());
        let previous = records.get(&embed_id).map(|s| record_digest(&s.json_text));
        // Idempotency: an unresolved record with the same reason
        // and sensitivity is already the run's outcome — rewriting
        // it would only churn provenance.
        let unchanged = match (records.get(&embed_id), decision.unresolved_reason) {
            (Some(stored), Some(reason)) => {
                is_unresolved(&stored.record)
                    && decision.outcome == DecisionOutcome::Unresolved
                    && stored.record.sensitivity == decision.sensitivity
                    && stored.record.extras.get("unresolved_reason") == Some(&Json::from(reason))
            }
            _ => false,
        };
        let capped = config
            .max_decisions
            .is_some_and(|max| to_write.len() >= max);
        let written = commit && !unchanged && !capped;
        if written {
            to_write.push((
                chunk.clone(),
                decision.clone(),
                evidence_refs.clone(),
                previous,
            ));
        }
        *counts
            .by_outcome
            .entry(decision.outcome.name().to_string())
            .or_default() += 1;
        entries.push(DecisionEntry {
            embed_id,
            prior_state: prior,
            outcome: decision.outcome,
            sensitivity: decision.sensitivity.name().to_string(),
            disposition: decision.disposition.map(|d| match d {
                ClassificationDisposition::Included => "included".to_string(),
                ClassificationDisposition::ExcludedPersonalLife => {
                    "excluded_personal_life".to_string()
                }
            }),
            unresolved_reason: decision.unresolved_reason.map(str::to_string),
            evidence_refs,
            related_sessions,
            related_classified_max: related_max.map(|s| s.name().to_string()),
            written,
        });
    }
    let run_id = format!(
        "sha256:{}",
        sha256_hex(
            format!(
                "{RUN_FORMAT}\n{}\n{}\n{}\n{}\n",
                db_path.display(),
                config.now,
                config_version,
                hex_digest(run_digest)
            )
            .as_bytes()
        )
    );

    // --- write --------------------------------------------------
    if commit {
        for (chunk, decision, evidence_refs, previous) in &to_write {
            let embed_id = format!("{EMBED_ID_PREFIX}{}", chunk.id);
            let mut extras = Map::new();
            if let Some(reason) = decision.unresolved_reason {
                extras.insert("unresolved_reason".into(), Json::from(reason));
            }
            let mut provenance_extras = Map::new();
            provenance_extras.insert("run_id".into(), Json::from(run_id.clone()));
            let record = ClassificationRecord {
                schema_version: config.schema_version,
                embed_id: embed_id.clone(),
                sensitivity: decision.sensitivity,
                disposition: decision.disposition,
                provenance: crate::classification::ClassificationProvenance {
                    actor: config.actor.clone(),
                    decided_at: config.now.clone(),
                    method: METHOD_SWEEP.to_string(),
                    method_version: METHOD_VERSION.to_string(),
                    config_version: config_version.clone(),
                    evidence_refs: evidence_refs.clone(),
                    previous_decision_id: previous.clone(),
                    extras: provenance_extras,
                },
                extras,
            };
            let json_text = record.to_json().to_string();
            con.execute(
                &format!(
                    "INSERT INTO {RECORDS_TABLE} (embed_id, record_json, sensitivity, schema_version, decided_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5) \
                     ON CONFLICT(embed_id) DO UPDATE SET record_json=excluded.record_json, \
                     sensitivity=excluded.sensitivity, schema_version=excluded.schema_version, \
                     decided_at=excluded.decided_at"
                ),
                params![
                    embed_id,
                    json_text,
                    decision.sensitivity.name(),
                    config.schema_version as i64,
                    config.now,
                ],
            )
            .map_err(|e| Error::backend(format!("classify: write {embed_id}: {e}")))?;
            counts.written += 1;
        }
    }

    let chunks_after = chunks_hash(&con)?;
    let vectors_after = vectors_hash(&con)?;
    if commit && (chunks_after != chunks_before || vectors_after != vectors_before) {
        return Err(Error::fatal(
            "classify: corpus content changed during the run — the sweep must never alter chunks or vectors",
        ));
    }

    Ok(RunManifest {
        format: RUN_FORMAT,
        run_id,
        mode: if commit { "commit" } else { "dry-run" },
        corpus: db_path.display().to_string(),
        actor: config.actor.clone(),
        decided_at: config.now.clone(),
        method: METHOD_SWEEP,
        method_version: METHOD_VERSION,
        config_version,
        schema_version: config.schema_version,
        window: WindowRecord {
            days: config.window_days,
            scope: "same-project",
            basis: "session.created_at",
        },
        counts,
        chunks_hash_before: chunks_before,
        chunks_hash_after: chunks_after,
        vectors_hash_before: vectors_before,
        vectors_hash_after: vectors_after,
        vectors_preserved: true,
        decisions: entries,
    })
}

/// Finalize a `Sha256` into lowercase hex (the crate's digest
/// rendering).
fn hex_digest(h: Sha256) -> String {
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// A schema document in the shape [`review_schema_change`]
/// consumes: `{"version": N, "breaking_change_event": bool?,
/// "sections": {name: definition}}`. Section definitions are
/// arbitrary JSON; the review understands `sensitivity_values`
/// and `dispositions` (their `values` lists and `renames` maps)
/// and treats every other section as record-wide.
#[derive(Debug, Clone)]
pub struct SchemaDoc {
    pub version: u64,
    pub breaking_change_event: bool,
    pub sections: Map<String, Json>,
}

impl SchemaDoc {
    pub fn parse(value: &Json) -> Result<Self> {
        let obj = value
            .as_object()
            .ok_or_else(|| Error::fatal("schema document must be a JSON object"))?;
        let version = obj
            .get("version")
            .and_then(Json::as_u64)
            .ok_or_else(|| Error::fatal("schema document: version must be an unsigned integer"))?;
        let breaking_change_event = obj
            .get("breaking_change_event")
            .and_then(Json::as_bool)
            .unwrap_or(false);
        let sections = obj
            .get("sections")
            .and_then(Json::as_object)
            .ok_or_else(|| Error::fatal("schema document: sections must be an object"))?
            .clone();
        Ok(Self {
            version,
            breaking_change_event,
            sections,
        })
    }

    /// SHA-256 over the canonical document (keys sorted
    /// recursively), the schema provenance hash reviews record.
    pub fn content_hash(&self) -> String {
        let canonical = canonical_json(&json!({
            "version": self.version,
            "sections": self.sections,
        }));
        format!("sha256:{}", sha256_hex(canonical.as_bytes()))
    }
}

/// Canonical JSON: object keys sorted at every depth, so hashes
/// and section comparisons are order-insensitive.
pub fn canonical_json(value: &Json) -> String {
    match value {
        Json::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .iter()
                .map(|k| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).expect("string serializes"),
                        canonical_json(&map[*k])
                    )
                })
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Json::Array(items) => {
            let parts: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", parts.join(","))
        }
        other => other.to_string(),
    }
}

/// Reviewer identity for a schema review (the design's low-cost
/// local reviewer model, as operator-supplied provenance).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewerIdentity {
    pub model: String,
    pub model_version: String,
    pub config_version: String,
}

/// One proposed record change from a schema review. Proposals
/// never write anything; [`apply_schema_review`] is the only
/// path from proposal to record, and it requires a disposition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordProposal {
    pub embed_id: String,
    pub section: String,
    /// `revalidate` (version bump only), `change` (value change),
    /// or `invalid` (cannot be applied as proposed).
    pub action: String,
    pub current_sensitivity: String,
    pub proposed_sensitivity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_disposition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed_disposition: Option<String>,
    /// True when applying would lower the record's effective
    /// restriction — the case that must never happen without a
    /// reviewed disposition.
    pub lowering: bool,
    pub reason: String,
}

/// One section's review outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionReview {
    pub name: String,
    /// added | removed | changed | unchanged
    pub change: String,
    pub examined: bool,
    pub proposals: Vec<RecordProposal>,
}

/// The schema-review manifest: provenance, section scope, and
/// proposals. Applying it is a separate, disposition-gated step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaReview {
    pub format: String,
    pub old_version: u64,
    pub new_version: u64,
    pub old_schema_hash: String,
    pub new_schema_hash: String,
    pub breaking_change_event: bool,
    /// Why the examined scope is what it is.
    pub scope_reason: String,
    pub reviewer: ReviewerIdentity,
    pub sections_examined: Vec<String>,
    pub sections_skipped: Vec<String>,
    pub sections: Vec<SectionReview>,
    /// Records at a schema version other than the old one: the
    /// review does not propose for them.
    pub records_at_other_versions: usize,
    pub proposals: usize,
    pub lowering_proposals: usize,
}

fn disposition_name(d: Option<ClassificationDisposition>) -> Option<String> {
    d.map(|d| match d {
        ClassificationDisposition::Included => "included".to_string(),
        ClassificationDisposition::ExcludedPersonalLife => "excluded_personal_life".to_string(),
    })
}

/// The string list at `key` in a section definition.
fn section_values(section: &Json, key: &str) -> Vec<String> {
    section
        .get(key)
        .and_then(Json::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// The rename map (`old -> new`) in a section definition.
fn section_renames(section: &Json) -> BTreeMap<String, String> {
    section
        .get("renames")
        .and_then(Json::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// Compare two schema documents against the corpus's current
/// records and produce the review batch (design §5.2). Changed
/// sections are examined by default; unchanged sections are
/// listed as skipped and produce nothing. A declared
/// breaking-change event on the new document examines every
/// section and proposes revalidation for all old-version
/// records, per the design's exception.
pub fn review_schema_change(
    db_path: &Path,
    old: &SchemaDoc,
    new: &SchemaDoc,
    reviewer: ReviewerIdentity,
) -> Result<SchemaReview> {
    if !db_path.exists() {
        return Err(Error::fatal(format!(
            "classify: corpus DB not found: {}",
            db_path.display()
        )));
    }
    let con = Connection::open_with_flags(
        format!("file:{}?mode=ro", db_path.display()),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|e| Error::backend(format!("classify: open {}: {e}", db_path.display())))?;
    let records = load_records(&con)?;
    let current: Vec<&StoredRecord> = records
        .values()
        .filter(|s| s.record.schema_version == old.version)
        .collect();
    let other_versions = records.len() - current.len();

    let breaking = new.breaking_change_event;
    let mut names: BTreeSet<String> = BTreeSet::new();
    names.extend(old.sections.keys().cloned());
    names.extend(new.sections.keys().cloned());

    let mut sections: Vec<SectionReview> = Vec::new();
    let mut examined_names: Vec<String> = Vec::new();
    let mut skipped_names: Vec<String> = Vec::new();
    let mut total_proposals = 0usize;
    let mut total_lowering = 0usize;

    for name in &names {
        let old_def = old.sections.get(name);
        let new_def = new.sections.get(name);
        let change = match (old_def, new_def) {
            (None, Some(_)) => "added",
            (Some(_), None) => "removed",
            (Some(a), Some(b)) => {
                if canonical_json(a) == canonical_json(b) {
                    "unchanged"
                } else {
                    "changed"
                }
            }
            (None, None) => unreachable!(),
        };
        let examined = breaking || change != "unchanged";
        if examined {
            examined_names.push(name.clone());
        } else {
            skipped_names.push(name.clone());
        }
        let mut proposals: Vec<RecordProposal> = Vec::new();
        if examined {
            for stored in &current {
                let record = &stored.record;
                let cur_sens = record.sensitivity.name().to_string();
                let cur_disp = disposition_name(record.disposition);
                let base = RecordProposal {
                    embed_id: record.embed_id.clone(),
                    section: name.clone(),
                    action: "revalidate".to_string(),
                    current_sensitivity: cur_sens.clone(),
                    proposed_sensitivity: cur_sens.clone(),
                    current_disposition: cur_disp.clone(),
                    proposed_disposition: cur_disp.clone(),
                    lowering: false,
                    reason: format!(
                        "section {name} {change}: record revalidates under schema v{}",
                        new.version
                    ),
                };
                match name.as_str() {
                    "sensitivity_values" => {
                        let new_values = new_def
                            .map(|d| section_values(d, "values"))
                            .unwrap_or_default();
                        let renames = new_def.map(section_renames).unwrap_or_default();
                        if new_values.contains(&cur_sens) {
                            proposals.push(base);
                        } else if let Some(target) = renames.get(&cur_sens) {
                            match Sensitivity::from_published_name(target) {
                                Some(t) => proposals.push(RecordProposal {
                                    action: "change".to_string(),
                                    proposed_sensitivity: target.clone(),
                                    lowering: t < record.sensitivity,
                                    reason: format!(
                                        "sensitivity value {cur_sens} renamed to {target} by schema v{}",
                                        new.version
                                    ),
                                    ..base
                                }),
                                None => proposals.push(RecordProposal {
                                    action: "invalid".to_string(),
                                    proposed_sensitivity: target.clone(),
                                    reason: format!(
                                        "rename target {target} is not a strict sensitivity value; cannot apply"
                                    ),
                                    ..base
                                }),
                            }
                        } else {
                            proposals.push(RecordProposal {
                                action: "change".to_string(),
                                proposed_sensitivity: Sensitivity::SecretBearing
                                    .name()
                                    .to_string(),
                                lowering: false,
                                reason: format!(
                                    "sensitivity value {cur_sens} removed by schema v{} without a rename; fail closed to the locked default",
                                    new.version
                                ),
                                ..base
                            });
                        }
                    }
                    "dispositions" => {
                        let Some(disp) = &cur_disp else {
                            proposals.push(base);
                            continue;
                        };
                        let new_values = new_def
                            .map(|d| section_values(d, "values"))
                            .unwrap_or_default();
                        let renames = new_def.map(section_renames).unwrap_or_default();
                        if new_values.contains(disp) {
                            proposals.push(base);
                        } else if let Some(target) = renames.get(disp) {
                            let lowering = disp.as_str() == "excluded_personal_life"
                                && target != "excluded_personal_life";
                            proposals.push(RecordProposal {
                                action: "change".to_string(),
                                proposed_disposition: Some(target.clone()),
                                lowering,
                                reason: format!(
                                    "disposition {disp} renamed to {target} by schema v{}",
                                    new.version
                                ),
                                ..base
                            });
                        } else {
                            proposals.push(RecordProposal {
                                action: "change".to_string(),
                                proposed_disposition: None,
                                lowering: disp.as_str() == "excluded_personal_life",
                                reason: format!(
                                    "disposition {disp} removed by schema v{}; dropping it {}",
                                    new.version,
                                    if disp.as_str() == "excluded_personal_life" {
                                        "un-excludes the embed and lowers its handling"
                                    } else {
                                        "does not change effective handling"
                                    }
                                ),
                                ..base
                            });
                        }
                    }
                    _ => proposals.push(base),
                }
            }
        }
        total_proposals += proposals.len();
        total_lowering += proposals.iter().filter(|p| p.lowering).count();
        sections.push(SectionReview {
            name: name.clone(),
            change: change.to_string(),
            examined,
            proposals,
        });
    }

    Ok(SchemaReview {
        format: REVIEW_FORMAT.to_string(),
        old_version: old.version,
        new_version: new.version,
        old_schema_hash: old.content_hash(),
        new_schema_hash: new.content_hash(),
        breaking_change_event: breaking,
        scope_reason: if breaking {
            "breaking_change_event: all sections examined (design §5.2 exception)".to_string()
        } else {
            "changed sections only (design §5.2 default)".to_string()
        },
        reviewer,
        sections_examined: examined_names,
        sections_skipped: skipped_names,
        sections,
        records_at_other_versions: other_versions,
        proposals: total_proposals,
        lowering_proposals: total_lowering,
    })
}

/// A reviewed disposition for one record: who decided, and why.
/// Applying any proposal requires one (design §5.2).
#[derive(Debug, Clone)]
pub struct Disposition {
    pub reviewer: String,
    pub rationale: String,
    pub decided_at: Option<String>,
}

/// Parse the dispositions document:
/// `{"dispositions": {"<embed_id>": {"reviewer": ..., "rationale": ..., "decided_at": ...?}}}`.
pub fn parse_dispositions(value: &Json) -> Result<BTreeMap<String, Disposition>> {
    let obj = value
        .get("dispositions")
        .and_then(Json::as_object)
        .ok_or_else(|| Error::fatal("dispositions document: 'dispositions' object is required"))?;
    let mut out = BTreeMap::new();
    for (embed_id, entry) in obj {
        let reviewer = entry
            .get("reviewer")
            .and_then(Json::as_str)
            .ok_or_else(|| {
                Error::fatal(format!(
                    "dispositions document: {embed_id} is missing reviewer"
                ))
            })?;
        let rationale = entry
            .get("rationale")
            .and_then(Json::as_str)
            .ok_or_else(|| {
                Error::fatal(format!(
                    "dispositions document: {embed_id} is missing rationale"
                ))
            })?;
        out.insert(
            embed_id.clone(),
            Disposition {
                reviewer: reviewer.to_string(),
                rationale: rationale.to_string(),
                decided_at: entry
                    .get("decided_at")
                    .and_then(Json::as_str)
                    .map(str::to_owned),
            },
        );
    }
    Ok(out)
}

/// The outcome of applying one proposal.
#[derive(Debug, Clone, Serialize)]
pub struct ApplyEntry {
    pub embed_id: String,
    pub section: String,
    /// applied | skipped_no_disposition | skipped_stale |
    /// skipped_invalid
    pub result: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// The schema-apply report.
#[derive(Debug, Clone, Serialize)]
pub struct ApplyReport {
    pub format: &'static str,
    pub corpus: String,
    pub new_schema_version: u64,
    pub applied: usize,
    pub skipped: usize,
    pub entries: Vec<ApplyEntry>,
}

/// Apply a schema review's proposals to the corpus's records.
/// Every applied change requires a reviewed disposition for its
/// record; lowering proposals without one are refused (listed,
/// never written). Proposals whose record has moved since the
/// review are skipped as stale. Applied decisions chain to the
/// record they replace via `previous_decision_id`.
pub fn apply_schema_review(
    db_path: &Path,
    review: &SchemaReview,
    dispositions: &BTreeMap<String, Disposition>,
    now: &str,
) -> Result<ApplyReport> {
    if !db_path.exists() {
        return Err(Error::fatal(format!(
            "classify: corpus DB not found: {}",
            db_path.display()
        )));
    }
    let con = Connection::open(db_path)
        .map_err(|e| Error::backend(format!("classify: open {}: {e}", db_path.display())))?;
    con.execute_batch(RECORDS_DDL)
        .map_err(|e| Error::backend(format!("classify: create records table: {e}")))?;

    let mut entries: Vec<ApplyEntry> = Vec::new();
    let mut applied = 0usize;
    let mut skipped = 0usize;
    // Records this run has already advanced to the new version:
    // a second section's proposal for the same record is not
    // stale merely because the first proposal bumped its version
    // (its reviewed value state is still checked below).
    let mut advanced: std::collections::HashSet<String> = std::collections::HashSet::new();
    for section in &review.sections {
        if !section.examined {
            continue;
        }
        for proposal in &section.proposals {
            let skip = |result: &str, detail: String| ApplyEntry {
                embed_id: proposal.embed_id.clone(),
                section: proposal.section.clone(),
                result: result.to_string(),
                detail: Some(detail),
            };
            if proposal.action == "invalid" {
                skipped += 1;
                entries.push(skip("skipped_invalid", proposal.reason.clone()));
                continue;
            }
            let Some(disposition) = dispositions.get(&proposal.embed_id) else {
                skipped += 1;
                entries.push(skip(
                    "skipped_no_disposition",
                    if proposal.lowering {
                        "lowering proposal without a reviewed disposition — refused".to_string()
                    } else {
                        "no reviewed disposition for this record".to_string()
                    },
                ));
                continue;
            };
            // Re-read the record: a proposal applies only to the
            // state it was reviewed against.
            let current_json: Option<String> = con
                .query_row(
                    &format!("SELECT record_json FROM {RECORDS_TABLE} WHERE embed_id=?1"),
                    params![proposal.embed_id],
                    |r| r.get(0),
                )
                .ok();
            let Some(current_json) = current_json else {
                skipped += 1;
                entries.push(skip("skipped_stale", "record no longer exists".to_string()));
                continue;
            };
            let current =
                ClassificationRecord::parse(serde_json::from_str(&current_json).map_err(|e| {
                    Error::fatal(format!(
                        "classify: stored record for {} is not valid JSON: {e}",
                        proposal.embed_id
                    ))
                })?)
                .map_err(|e| Error::fatal(format!("classify: stored record: {e}")))?;
            let version_ok = current.schema_version == review.old_version
                || (advanced.contains(&proposal.embed_id)
                    && current.schema_version == review.new_version);
            if current.sensitivity.name() != proposal.current_sensitivity
                || disposition_name(current.disposition) != proposal.current_disposition
                || !version_ok
            {
                skipped += 1;
                entries.push(skip(
                    "skipped_stale",
                    "record changed since the review was produced".to_string(),
                ));
                continue;
            }
            let proposed_disposition = match &proposal.proposed_disposition {
                Some(name) => Some(match name.as_str() {
                    "included" => ClassificationDisposition::Included,
                    "excluded_personal_life" => ClassificationDisposition::ExcludedPersonalLife,
                    other => {
                        skipped += 1;
                        entries.push(skip(
                            "skipped_invalid",
                            format!("proposed disposition {other} is not a strict value"),
                        ));
                        continue;
                    }
                }),
                None => None,
            };
            let decided_at = disposition
                .decided_at
                .clone()
                .unwrap_or_else(|| now.to_string());
            let mut extras = current.extras.clone();
            // An applied review decision is a decided record: a
            // stale unresolved marker must not survive it, or the
            // next sweep would treat the record as a candidate
            // again.
            extras.remove("unresolved_reason");
            extras.insert(
                "schema_review".into(),
                json!({
                    "section": proposal.section,
                    "rationale": disposition.rationale,
                    "old_schema_version": review.old_version,
                    "new_schema_version": review.new_version,
                }),
            );
            let record = ClassificationRecord {
                schema_version: review.new_version,
                embed_id: proposal.embed_id.clone(),
                sensitivity: Sensitivity::from_published_name(&proposal.proposed_sensitivity)
                    .unwrap_or(current.sensitivity),
                disposition: proposed_disposition,
                provenance: crate::classification::ClassificationProvenance {
                    actor: disposition.reviewer.clone(),
                    decided_at: decided_at.clone(),
                    method: METHOD_SCHEMA_REVIEW.to_string(),
                    method_version: METHOD_VERSION.to_string(),
                    config_version: review.reviewer.config_version.clone(),
                    evidence_refs: vec![
                        format!(
                            "schema-review:v{}->v{}",
                            review.old_version, review.new_version
                        ),
                        format!("section:{}", proposal.section),
                    ],
                    previous_decision_id: Some(record_digest(&current_json)),
                    extras: Map::new(),
                },
                extras,
            };
            let json_text = record.to_json().to_string();
            con.execute(
                &format!(
                    "INSERT INTO {RECORDS_TABLE} (embed_id, record_json, sensitivity, schema_version, decided_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5) \
                     ON CONFLICT(embed_id) DO UPDATE SET record_json=excluded.record_json, \
                     sensitivity=excluded.sensitivity, schema_version=excluded.schema_version, \
                     decided_at=excluded.decided_at"
                ),
                params![
                    proposal.embed_id,
                    json_text,
                    record.sensitivity.name(),
                    review.new_version as i64,
                    decided_at,
                ],
            )
            .map_err(|e| Error::backend(format!("classify: apply {}: {e}", proposal.embed_id)))?;
            advanced.insert(proposal.embed_id.clone());
            applied += 1;
            entries.push(ApplyEntry {
                embed_id: proposal.embed_id.clone(),
                section: proposal.section.clone(),
                result: "applied".to_string(),
                detail: None,
            });
        }
    }
    Ok(ApplyReport {
        format: APPLY_FORMAT,
        corpus: db_path.display().to_string(),
        new_schema_version: review.new_version,
        applied,
        skipped,
        entries,
    })
}
