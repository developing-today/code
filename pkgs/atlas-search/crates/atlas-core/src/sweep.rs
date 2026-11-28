//! Atlas ingestion sweep runner core (#7969) — the deterministic heart
//! of the recurring ingestion sweep designed in
//! `doc/2026-11-09T05-12-00Z_design_atlas_ingestion_operations/`
//! (parent design #6019, program #6018).
//!
//! This module implements the design's testable core, with the seams
//! the design fixes kept explicit:
//!
//! - §2: the source-class table and its usefulness filter. Every
//!   candidate outcome is a manifest-shaped record; rejections carry a
//!   reason from the closed code set, and secret-tier items are HELD
//!   (never staged, never sent to a hosted embedder) until the #6059
//!   local route lands.
//! - §4: guardrail evaluation over a run's measurement points, with
//!   the design's initial-slot thresholds and named alerts.
//! - §5: plan-headroom accounting. The envelope is an INPUT (the plan
//!   is a human-set constant between plan changes; this code invents
//!   no quota numbers) and the alert rules are the design's complete
//!   set: 70% projection surfacing, 80% alert, 95%/30-day ceiling
//!   stop, spend anomaly.
//! - §6: one lock per (source class, host) with overlap-skip, cursor
//!   idempotence by content address, per-item failure quarantine with
//!   a 3-strike dead-letter list, and the operator actions
//!   (`pause-class`, `resume-class`, `run-once`, `dead-letter`,
//!   `report`) as functions over a state directory.
//!
//! What this module deliberately does NOT do: discover from live
//! sources (adapters are the follow-on), land into the staging shard
//! (accepted items are counted and their content addresses recorded;
//! the #6057 gate owns promotion and the sweep never promotes or
//! deletes), read live Turso usage (§8 open question (a) — callers
//! supply the deltas), or create/enable any schedule (Drew-gated on
//! #7969).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// §4 per-item slot: a single source object over 256 MiB is an
/// `oversize` reject, never partially read.
pub const OVERSIZE_BYTES: u64 = 256 * 1024 * 1024;
/// §6: a held lock older than this is stale and may be taken over
/// (a crashed run's lock expires; staged partial work is inert).
pub const LOCK_TTL_SECONDS: u64 = 2 * 3600;
/// §6: failures of the same item across this many runs dead-letter it.
pub const DEAD_LETTER_STRIKES: u32 = 3;
/// §5 cycle bookkeeping: report rows retained per state directory.
pub const MAX_REPORT_ROWS: usize = 100;

/// The §2 source classes. `code-repo` is the pre-existing git class,
/// listed for completeness; its ingestion is unchanged by the design.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceClass {
    BootRecords,
    SessionLogs,
    ChatLogs,
    Statistics,
    Reports,
    CodeRepo,
}

/// How a class is triggered (§2 / §6 cadence shape).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Trigger {
    /// Drained by the frequent lightweight sweep on finalization.
    EventDriven,
    /// Runs in its class slot.
    Scheduled,
    /// Pre-existing ingestion, unchanged.
    Existing,
}

impl SourceClass {
    /// All classes, in design-table order.
    pub const ALL: [SourceClass; 6] = [
        SourceClass::BootRecords,
        SourceClass::SessionLogs,
        SourceClass::ChatLogs,
        SourceClass::Statistics,
        SourceClass::Reports,
        SourceClass::CodeRepo,
    ];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            SourceClass::BootRecords => "boot-records",
            SourceClass::SessionLogs => "session-logs",
            SourceClass::ChatLogs => "chat-logs",
            SourceClass::Statistics => "statistics",
            SourceClass::Reports => "reports",
            SourceClass::CodeRepo => "code-repo",
        }
    }

    /// Parse a class name as produced by [`SourceClass::name`].
    #[must_use]
    pub fn from_name(name: &str) -> Option<SourceClass> {
        SourceClass::ALL
            .into_iter()
            .find(|class| class.name() == name)
    }

    #[must_use]
    pub fn trigger(self) -> Trigger {
        match self {
            SourceClass::BootRecords | SourceClass::SessionLogs | SourceClass::Reports => {
                Trigger::EventDriven
            }
            SourceClass::ChatLogs | SourceClass::Statistics => Trigger::Scheduled,
            SourceClass::CodeRepo => Trigger::Existing,
        }
    }

    /// The §2 inclusion test, stated as the question the class answers.
    #[must_use]
    pub fn question(self) -> &'static str {
        match self {
            SourceClass::BootRecords => "did this host come up clean, and what ran?",
            SourceClass::SessionLogs => "what was decided/done, and what evidence supports it?",
            SourceClass::ChatLogs => "what did the fleet agree, announce, or escalate?",
            SourceClass::Statistics => {
                "what did the fleet consume/produce in this period, at what cost?"
            }
            SourceClass::Reports => "what did this run conclude?",
            SourceClass::CodeRepo => "per #6028/#6058 (existing git ingestion)",
        }
    }
}

/// The closed §2 reason-code set. Rejections and secret-tier holds are
/// logged with one of these; the set grows only by design amendment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReasonCode {
    NoQuestion,
    DuplicateContent,
    Oversize,
    SecretTierHeld,
    SupersededVersion,
}

impl ReasonCode {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            ReasonCode::NoQuestion => "no-question",
            ReasonCode::DuplicateContent => "duplicate-content",
            ReasonCode::Oversize => "oversize",
            ReasonCode::SecretTierHeld => "secret-tier-held",
            ReasonCode::SupersededVersion => "superseded-version",
        }
    }
}

/// The facts about one discovered candidate that the §2 class tests
/// read. Adapters (the follow-on) produce these from live sources;
/// keeping them a plain struct is what makes the filter testable
/// without any source system.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct CandidateFacts {
    /// Stable source reference (path, id, or URI) for the manifest.
    pub source_ref: String,
    /// Item size in bytes (checked against the §4 per-item slot).
    pub bytes: u64,
    /// Content address, when the adapter computed one (§3 landing key).
    pub content_address: Option<String>,
    /// Classified secret-tier (§2 secret-tier rule).
    pub secret_tier: bool,
    /// The item reached a terminal/final state (boot segment closed,
    /// session finalized).
    pub terminal: bool,
    /// The item carries substance per its class test (a non-synthesized
    /// exchange in a session; a non-empty boot record).
    pub has_substance: bool,
    /// Chat only: the item is a bot or heartbeat post.
    pub bot_or_heartbeat: bool,
    /// Chat only: the channel is on the class allowlist. Off-allowlist
    /// channels are not read at all — absence, not rejection.
    pub allowlisted: bool,
    /// Statistics only: this is the final version for its period.
    pub final_version: bool,
    /// Reports only: the file is a partial/`.tmp` write still in flight.
    pub partial_write: bool,
}

/// What the §2 filter decides for one candidate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilterOutcome {
    Accept,
    Reject(ReasonCode),
    /// Held, not rejected: re-offered automatically when its condition
    /// clears (secret-tier route lands; segment/session finalizes).
    /// `None` = held for finalization, with no reason code in the
    /// closed set applying.
    Hold(Option<ReasonCode>),
    /// Not a candidate at all (off-allowlist chat): no manifest record.
    Skip,
}

/// The §2 usefulness filter. Order matters and follows the design:
/// oversize is never partially read, secret-tier is held before any
/// staging question, duplicates reject against the landed set, and
/// only then does the per-class question apply.
#[must_use]
pub fn filter_candidate(
    class: SourceClass,
    facts: &CandidateFacts,
    landed: &HashSet<String>,
) -> FilterOutcome {
    if facts.bytes > OVERSIZE_BYTES {
        return FilterOutcome::Reject(ReasonCode::Oversize);
    }
    if facts.secret_tier {
        return FilterOutcome::Hold(Some(ReasonCode::SecretTierHeld));
    }
    if let Some(address) = &facts.content_address
        && landed.contains(address)
    {
        return FilterOutcome::Reject(ReasonCode::DuplicateContent);
    }
    match class {
        SourceClass::BootRecords => {
            if facts.terminal {
                FilterOutcome::Accept
            } else {
                // Partial/still-open segments are held, not rejected.
                FilterOutcome::Hold(None)
            }
        }
        SourceClass::SessionLogs => {
            if !facts.terminal {
                FilterOutcome::Hold(None)
            } else if facts.has_substance {
                FilterOutcome::Accept
            } else {
                FilterOutcome::Reject(ReasonCode::NoQuestion)
            }
        }
        SourceClass::ChatLogs => {
            if !facts.allowlisted {
                FilterOutcome::Skip
            } else if facts.bot_or_heartbeat {
                FilterOutcome::Reject(ReasonCode::NoQuestion)
            } else {
                FilterOutcome::Accept
            }
        }
        SourceClass::Statistics => {
            if facts.final_version {
                FilterOutcome::Accept
            } else {
                FilterOutcome::Reject(ReasonCode::SupersededVersion)
            }
        }
        SourceClass::Reports => {
            if facts.partial_write {
                FilterOutcome::Hold(None)
            } else {
                FilterOutcome::Accept
            }
        }
        SourceClass::CodeRepo => FilterOutcome::Accept,
    }
}

/// One §2 manifest record — the same shape for acceptances, rejections,
/// and holds (rejection is data; the unassociated-log principle).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ManifestRecord {
    pub batch_id: String,
    pub source_class: String,
    pub source_ref: String,
    /// "accepted" | "rejected" | "held"
    pub outcome: String,
    pub reason_code: Option<String>,
    pub bytes: u64,
    pub at: String,
}

/// §4 measurement points for one run (all of them land in the §5
/// report; the adapters/host probes that produce them are follow-on).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Measurements {
    pub src_free_bytes: u64,
    pub src_volume_bytes: u64,
    pub src_io_wait_pct: f64,
    pub stg_free_bytes: u64,
    pub projected_batch_bytes: u64,
    pub tgt_read_rows: u64,
    pub tgt_write_rows: u64,
    /// Trailing 7-day per-run median write rows for this class.
    pub class_median_write_rows: Option<u64>,
}

/// What a guardrail does to the run (§4 behaviour column).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuardrailAction {
    Pause,
    Abort,
    Throttle,
    PauseClass,
}

/// One §4 guardrail firing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuardrailEvent {
    /// Named alert, when the design gives the firing one.
    pub alert: Option<String>,
    pub action: GuardrailAction,
    pub detail: String,
}

const GIB: u64 = 1024 * 1024 * 1024;

/// §4 guardrail evaluation, thresholds exactly as the design's
/// initial slots. `previous_throttled` carries the §4 rule that the
/// `guardrail/src-io` alert fires on the SECOND consecutive throttled
/// run for a source.
#[must_use]
pub fn evaluate_guardrails(m: &Measurements, previous_throttled: bool) -> Vec<GuardrailEvent> {
    let mut events = Vec::new();
    let src_low_threshold = (m.src_volume_bytes / 10).max(5 * GIB);
    let src_critical_threshold = (m.src_volume_bytes / 20).max(2 * GIB);
    if m.src_free_bytes < src_critical_threshold {
        events.push(GuardrailEvent {
            alert: Some("guardrail/src-space-critical".to_string()),
            action: GuardrailAction::Abort,
            detail: format!(
                "src_free_bytes {} below critical threshold {}",
                m.src_free_bytes, src_critical_threshold
            ),
        });
    } else if m.src_free_bytes < src_low_threshold {
        events.push(GuardrailEvent {
            alert: Some("guardrail/src-space".to_string()),
            action: GuardrailAction::Pause,
            detail: format!(
                "src_free_bytes {} below threshold {}",
                m.src_free_bytes, src_low_threshold
            ),
        });
    }
    if m.src_io_wait_pct > 25.0 {
        events.push(GuardrailEvent {
            alert: previous_throttled.then(|| "guardrail/src-io".to_string()),
            action: GuardrailAction::Throttle,
            detail: format!("src_io_wait_pct {} above 25", m.src_io_wait_pct),
        });
    }
    let stg_threshold = m.projected_batch_bytes.saturating_mul(3).max(10 * GIB);
    if m.stg_free_bytes < stg_threshold {
        events.push(GuardrailEvent {
            alert: Some("guardrail/stg-space".to_string()),
            action: GuardrailAction::Pause,
            detail: format!(
                "stg_free_bytes {} below threshold {}",
                m.stg_free_bytes, stg_threshold
            ),
        });
    }
    if let Some(median) = m.class_median_write_rows
        && median > 0
        && m.tgt_write_rows > median.saturating_mul(2)
    {
        events.push(GuardrailEvent {
            alert: Some("guardrail/tgt-burst".to_string()),
            action: GuardrailAction::PauseClass,
            detail: format!(
                "tgt_write_rows {} above 2x trailing median {}",
                m.tgt_write_rows, median
            ),
        });
    }
    events
}

/// One §5 envelope line, as supplied by the caller: current position
/// plus the #6061 growth model's projections. No quota numbers live
/// in this code — the plan envelope is a human-set constant.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EnvelopeLine {
    pub name: String,
    /// Current position against the envelope, percent (0–100+).
    pub pct: f64,
    /// #6061 projection: days until this line crosses 70%.
    pub days_to_70_pct: Option<f64>,
    /// #6061 projection: days until this line reaches its ceiling.
    pub days_to_ceiling: Option<f64>,
}

/// A §5 headroom firing.
#[derive(Clone, Debug, PartialEq)]
pub struct HeadroomEvent {
    /// "headroom-warning" (surfaced in the cycle report),
    /// "plan/headroom-80", or "plan/ceiling-stop".
    pub alert: String,
    pub line: String,
}

/// §5's complete alert set, evaluated per envelope line; the highest
/// severity per line wins.
#[must_use]
pub fn evaluate_headroom(lines: &[EnvelopeLine]) -> Vec<HeadroomEvent> {
    let mut events = Vec::new();
    for line in lines {
        if line.pct >= 95.0 || line.days_to_ceiling.is_some_and(|days| days <= 30.0) {
            events.push(HeadroomEvent {
                alert: "plan/ceiling-stop".to_string(),
                line: line.name.clone(),
            });
        } else if line.pct >= 80.0 {
            events.push(HeadroomEvent {
                alert: "plan/headroom-80".to_string(),
                line: line.name.clone(),
            });
        } else if line.days_to_70_pct.is_some_and(|days| days <= 180.0) {
            events.push(HeadroomEvent {
                alert: "headroom-warning".to_string(),
                line: line.name.clone(),
            });
        }
    }
    events
}

/// §5 spend anomaly: a run value above 2× its class's trailing
/// 7-day median (embedding spend reports as `plan/spend-anomaly`;
/// write rows are covered by `guardrail/tgt-burst` instead).
#[must_use]
pub fn spend_anomaly(run_value: f64, trailing_median: Option<f64>) -> bool {
    trailing_median.is_some_and(|median| median > 0.0 && run_value > 2.0 * median)
}

/// The §5 report row: one per source class per run (the cycle report
/// rolls these up per source).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct RunReportRow {
    pub run_id: String,
    pub batch_id: String,
    pub source_class: String,
    pub host: String,
    pub started_at: String,
    pub finished_at: String,
    pub items_seen: u64,
    pub items_accepted: u64,
    pub items_rejected: u64,
    pub items_held: u64,
    pub bytes_read: u64,
    pub bytes_staged: u64,
    pub storage_delta_bytes: i64,
    pub chunks_new: u64,
    pub chunks_reused: u64,
    pub tokens_embedded: u64,
    pub embed_model: Option<String>,
    pub embed_usd: f64,
    pub tgt_read_rows: u64,
    pub tgt_write_rows: u64,
    pub db_count_after: u64,
    pub envelope_storage_pct: f64,
    pub envelope_reads_pct: f64,
    pub envelope_writes_pct: f64,
    pub envelope_db_pct: f64,
    pub projected_days_to_ceiling: Option<f64>,
    pub guardrail_events: Vec<String>,
    pub pause_state: String,
}

/// Persistent runner state in the state directory. Pause state is per
/// (source class, host) and is itself reported, never silent (§4).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SweepState {
    /// Paused `class@host` keys.
    pub paused: BTreeSet<String>,
    /// Content addresses landed by completed runs (idempotence, §6).
    pub landed: BTreeSet<String>,
    /// Per-item failure counts across runs (§6 quarantine).
    pub failures: BTreeMap<String, u32>,
    /// Dead-lettered source refs, in the order they dead-lettered.
    pub dead_letters: Vec<String>,
    /// `class@host` keys whose previous run throttled (§4 src-io rule).
    pub throttled_last: BTreeSet<String>,
    /// Recent report rows, oldest first, capped at MAX_REPORT_ROWS.
    pub last_reports: Vec<RunReportRow>,
}

/// The (source class, host) key used for locks and pause state.
#[must_use]
pub fn class_host_key(class: SourceClass, host: &str) -> String {
    format!("{}@{}", class.name(), host)
}

fn state_path(state_dir: &Path) -> PathBuf {
    state_dir.join("sweep-state.json")
}

/// Load the state file (an absent file is a fresh state).
pub fn load_state(state_dir: &Path) -> Result<SweepState> {
    let path = state_path(state_dir);
    if !path.exists() {
        return Ok(SweepState::default());
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| Error::backend(format!("read {}: {e}", path.display())))?;
    serde_json::from_str(&text)
        .map_err(|e| Error::backend(format!("parse {}: {e}", path.display())))
}

/// Persist the state file (written via a temp file + rename so a
/// crash mid-write cannot corrupt the previous state).
pub fn save_state(state_dir: &Path, state: &SweepState) -> Result<()> {
    std::fs::create_dir_all(state_dir)
        .map_err(|e| Error::backend(format!("create {}: {e}", state_dir.display())))?;
    let path = state_path(state_dir);
    let tmp = state_dir.join("sweep-state.json.tmp");
    let text = serde_json::to_string_pretty(state)
        .map_err(|e| Error::backend(format!("serialize state: {e}")))?;
    std::fs::write(&tmp, text)
        .map_err(|e| Error::backend(format!("write {}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, &path)
        .map_err(|e| Error::backend(format!("rename to {}: {e}", path.display())))
}

fn lock_path(state_dir: &Path, class: SourceClass, host: &str) -> PathBuf {
    let host_part: String = host
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    state_dir.join(format!("lock-{}-{}.json", class.name(), host_part))
}

#[derive(Deserialize, Serialize)]
struct LockFile {
    run_id: String,
    at_unix: u64,
}

/// The §6 per-(source class, host) lock. Dropping releases it; a lock
/// older than LOCK_TTL_SECONDS is stale and taken over (host failure
/// mid-run leaves inert staged work, never a stuck fleet).
pub struct SweepLock {
    path: PathBuf,
    run_id: String,
}

impl SweepLock {
    /// Acquire the lock, or `None` when a live run holds it
    /// (the caller logs `overlap-skip` and exits, per §6).
    pub fn acquire(
        state_dir: &Path,
        class: SourceClass,
        host: &str,
        run_id: &str,
        now_unix: u64,
    ) -> Result<Option<SweepLock>> {
        std::fs::create_dir_all(state_dir)
            .map_err(|e| Error::backend(format!("create {}: {e}", state_dir.display())))?;
        let path = lock_path(state_dir, class, host);
        if let Ok(text) = std::fs::read_to_string(&path)
            && let Ok(existing) = serde_json::from_str::<LockFile>(&text)
            && now_unix.saturating_sub(existing.at_unix) < LOCK_TTL_SECONDS
        {
            return Ok(None);
        }
        let text = serde_json::to_string(&LockFile {
            run_id: run_id.to_string(),
            at_unix: now_unix,
        })
        .map_err(|e| Error::backend(format!("serialize lock: {e}")))?;
        std::fs::write(&path, text)
            .map_err(|e| Error::backend(format!("write {}: {e}", path.display())))?;
        Ok(Some(SweepLock {
            path,
            run_id: run_id.to_string(),
        }))
    }
}

impl Drop for SweepLock {
    fn drop(&mut self) {
        if let Ok(text) = std::fs::read_to_string(&self.path)
            && let Ok(existing) = serde_json::from_str::<LockFile>(&text)
            && existing.run_id == self.run_id
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// One run-once request (§6 operator action shape): a single source
/// class on a single host, candidates supplied by the caller's
/// discovery adapter.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunRequest {
    pub run_id: String,
    pub batch_id: String,
    pub source_class: SourceClass,
    pub host: String,
    /// ISO-8601 run timestamp, used for manifest records and the row.
    pub at: String,
    /// Unix seconds, for lock freshness.
    pub now_unix: u64,
    pub candidates: Vec<CandidateFacts>,
    /// Source refs that failed (unreadable/malformed) in this run;
    /// they are quarantined in the manifest accounting and counted
    /// toward the dead-letter strikes.
    #[serde(default)]
    pub item_failures: Vec<String>,
    pub measurements: Measurements,
    pub envelope: Vec<EnvelopeLine>,
    #[serde(default)]
    pub embed_usd: f64,
    pub embed_usd_median: Option<f64>,
}

/// How a run ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunStatus {
    Completed,
    /// A live lock for the same (class, host) exists (§6).
    OverlapSkip,
    /// The class is paused (operator, guardrail, or ceiling stop).
    PausedClass,
    /// A critical guardrail aborted the run; staged work stays staged
    /// and nothing is promoted from a partial batch (§4).
    Aborted,
    /// A pause guardrail stopped the run before landing (§4).
    GuardrailPaused,
}

/// The result of one run: manifest records, the §5 report row (when a
/// row is owed), and every alert name the run raised.
#[derive(Clone, Debug)]
pub struct RunOutcome {
    pub status: RunStatus,
    pub records: Vec<ManifestRecord>,
    pub report: Option<RunReportRow>,
    pub alerts: Vec<String>,
}

/// Run one sweep over the supplied candidates, per §§2–6. The run
/// never promotes and never deletes; accepted items are counted and
/// their content addresses recorded as landed (the staging write and
/// the #6057 promotion are downstream of this core).
pub fn run_once(state_dir: &Path, req: &RunRequest) -> Result<RunOutcome> {
    let mut state = load_state(state_dir)?;
    let key = class_host_key(req.source_class, &req.host);
    let mut report = RunReportRow {
        run_id: req.run_id.clone(),
        batch_id: req.batch_id.clone(),
        source_class: req.source_class.name().to_string(),
        host: req.host.clone(),
        started_at: req.at.clone(),
        finished_at: req.at.clone(),
        tgt_read_rows: req.measurements.tgt_read_rows,
        tgt_write_rows: req.measurements.tgt_write_rows,
        embed_usd: req.embed_usd,
        ..RunReportRow::default()
    };
    let finish = |state: &SweepState,
                  report: RunReportRow,
                  status: RunStatus,
                  records: Vec<ManifestRecord>,
                  alerts: Vec<String>|
     -> Result<RunOutcome> {
        let mut state = state.clone();
        state.last_reports.push(report.clone());
        if state.last_reports.len() > MAX_REPORT_ROWS {
            let excess = state.last_reports.len() - MAX_REPORT_ROWS;
            state.last_reports.drain(..excess);
        }
        save_state(state_dir, &state)?;
        Ok(RunOutcome {
            status,
            records,
            report: Some(report),
            alerts,
        })
    };

    if state.paused.contains(&key) {
        report.pause_state = "paused".to_string();
        return finish(
            &state,
            report,
            RunStatus::PausedClass,
            Vec::new(),
            Vec::new(),
        );
    }
    let Some(_lock) = SweepLock::acquire(
        state_dir,
        req.source_class,
        &req.host,
        &req.run_id,
        req.now_unix,
    )?
    else {
        // Overlap: the other runner owns this pair; §6 logs
        // `overlap-skip` (the CLI prints it) and no row is owed.
        return Ok(RunOutcome {
            status: RunStatus::OverlapSkip,
            records: Vec::new(),
            report: None,
            alerts: Vec::new(),
        });
    };

    let previous_throttled = state.throttled_last.contains(&key);
    let guardrails = evaluate_guardrails(&req.measurements, previous_throttled);
    let mut alerts: Vec<String> = guardrails
        .iter()
        .filter_map(|event| event.alert.clone())
        .collect();
    report.guardrail_events = guardrails
        .iter()
        .map(|event| {
            event
                .alert
                .clone()
                .unwrap_or_else(|| format!("guardrail/{:?}", event.action).to_lowercase())
        })
        .collect();
    if guardrails
        .iter()
        .any(|event| event.action == GuardrailAction::Throttle)
    {
        state.throttled_last.insert(key.clone());
    } else {
        state.throttled_last.remove(&key);
    }
    if let Some(event) = guardrails
        .iter()
        .find(|event| event.action == GuardrailAction::Abort)
    {
        report.pause_state = "aborted".to_string();
        let _ = event;
        return finish(&state, report, RunStatus::Aborted, Vec::new(), alerts);
    }
    if guardrails
        .iter()
        .any(|event| event.action == GuardrailAction::Pause)
    {
        report.pause_state = "guardrail-paused".to_string();
        return finish(
            &state,
            report,
            RunStatus::GuardrailPaused,
            Vec::new(),
            alerts,
        );
    }

    // §2 filter pass. Dead-lettered refs are no longer re-offered.
    let landed: HashSet<String> = state.landed.iter().cloned().collect();
    let mut records = Vec::new();
    for facts in &req.candidates {
        report.items_seen += 1;
        report.bytes_read = report.bytes_read.saturating_add(facts.bytes);
        if state.dead_letters.contains(&facts.source_ref) {
            continue;
        }
        let outcome = filter_candidate(req.source_class, facts, &landed);
        let (outcome_name, reason) = match outcome {
            FilterOutcome::Accept => {
                report.items_accepted += 1;
                report.bytes_staged = report.bytes_staged.saturating_add(facts.bytes);
                if let Some(address) = &facts.content_address {
                    state.landed.insert(address.clone());
                }
                ("accepted", None)
            }
            FilterOutcome::Reject(code) => {
                report.items_rejected += 1;
                ("rejected", Some(code.name().to_string()))
            }
            FilterOutcome::Hold(code) => {
                report.items_held += 1;
                ("held", code.map(|c| c.name().to_string()))
            }
            FilterOutcome::Skip => continue,
        };
        records.push(ManifestRecord {
            batch_id: req.batch_id.clone(),
            source_class: req.source_class.name().to_string(),
            source_ref: facts.source_ref.clone(),
            outcome: outcome_name.to_string(),
            reason_code: reason,
            bytes: facts.bytes,
            at: req.at.clone(),
        });
    }

    // §6 item-failure quarantine: 3 strikes across runs dead-letters.
    for source_ref in &req.item_failures {
        let count = state.failures.entry(source_ref.clone()).or_insert(0);
        *count += 1;
        if *count >= DEAD_LETTER_STRIKES && !state.dead_letters.contains(source_ref) {
            state.dead_letters.push(source_ref.clone());
        }
    }

    // §4 burst + §5 headroom/spend rules that act after the run.
    if guardrails
        .iter()
        .any(|event| event.action == GuardrailAction::PauseClass)
    {
        state.paused.insert(key.clone());
        report.pause_state = "paused".to_string();
    }
    for line in &req.envelope {
        match line.name.as_str() {
            "storage" => report.envelope_storage_pct = line.pct,
            "reads" => report.envelope_reads_pct = line.pct,
            "writes" => report.envelope_writes_pct = line.pct,
            "databases" => report.envelope_db_pct = line.pct,
            _ => {}
        }
    }
    report.projected_days_to_ceiling = req
        .envelope
        .iter()
        .filter_map(|line| line.days_to_ceiling)
        .reduce(f64::min);
    let mut ceiling_stop = false;
    for event in evaluate_headroom(&req.envelope) {
        if event.alert == "plan/ceiling-stop" {
            ceiling_stop = true;
        }
        alerts.push(event.alert);
    }
    if ceiling_stop {
        // §5 rule 3: automated sweeps pause for the affected classes;
        // this runner pauses its own class and reports — resuming is
        // a human decision.
        state.paused.insert(key.clone());
        report.pause_state = "paused".to_string();
    }
    if spend_anomaly(req.embed_usd, req.embed_usd_median) {
        alerts.push("plan/spend-anomaly".to_string());
    }
    if report.pause_state.is_empty() {
        report.pause_state = "running".to_string();
    }
    finish(&state, report, RunStatus::Completed, records, alerts)
}

/// §6 operator action: pause a class on a host.
pub fn pause_class(state_dir: &Path, class: SourceClass, host: &str) -> Result<()> {
    let mut state = load_state(state_dir)?;
    state.paused.insert(class_host_key(class, host));
    save_state(state_dir, &state)
}

/// §6 operator action: resume a class on a host.
pub fn resume_class(state_dir: &Path, class: SourceClass, host: &str) -> Result<()> {
    let mut state = load_state(state_dir)?;
    state.paused.remove(&class_host_key(class, host));
    save_state(state_dir, &state)
}

/// §6 operator action: list the dead-letter queue.
pub fn dead_letter_list(state_dir: &Path) -> Result<Vec<String>> {
    Ok(load_state(state_dir)?.dead_letters)
}

/// §6 operator action: retry a dead-lettered item (clears its strikes).
pub fn dead_letter_retry(state_dir: &Path, source_ref: &str) -> Result<()> {
    let mut state = load_state(state_dir)?;
    state.dead_letters.retain(|r| r != source_ref);
    state.failures.remove(source_ref);
    save_state(state_dir, &state)
}

/// §6 operator action: drop a dead-lettered item after human review
/// (it stays out of the sweep's way via its landed/superseded state;
/// the drop only clears the review queue entry and its strikes).
pub fn dead_letter_drop(state_dir: &Path, source_ref: &str) -> Result<()> {
    dead_letter_retry(state_dir, source_ref)
}

/// §6 operator action: the cycle report — recent run rows, oldest first.
pub fn cycle_report(state_dir: &Path) -> Result<Vec<RunReportRow>> {
    Ok(load_state(state_dir)?.last_reports)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn facts(source_ref: &str) -> CandidateFacts {
        CandidateFacts {
            source_ref: source_ref.to_string(),
            bytes: 1024,
            ..CandidateFacts::default()
        }
    }

    fn temp_state_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("atlas-sweep-test-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp state dir");
        dir
    }

    #[test]
    fn filter_applies_the_design_order_and_closed_reason_set() {
        let landed: HashSet<String> = HashSet::new();
        // Oversize beats secret-tier: never partially read.
        let mut f = facts("big-secret");
        f.bytes = OVERSIZE_BYTES + 1;
        f.secret_tier = true;
        assert_eq!(
            filter_candidate(SourceClass::Reports, &f, &landed),
            FilterOutcome::Reject(ReasonCode::Oversize)
        );
        // Secret-tier is held, never rejected or staged.
        let mut f = facts("secret-session");
        f.secret_tier = true;
        f.terminal = true;
        f.has_substance = true;
        assert_eq!(
            filter_candidate(SourceClass::SessionLogs, &f, &landed),
            FilterOutcome::Hold(Some(ReasonCode::SecretTierHeld))
        );
        // Duplicate content rejects against the landed set.
        let mut f = facts("dupe");
        f.content_address = Some("addr-1".to_string());
        let landed: HashSet<String> = ["addr-1".to_string()].into_iter().collect();
        assert_eq!(
            filter_candidate(SourceClass::Reports, &f, &landed),
            FilterOutcome::Reject(ReasonCode::DuplicateContent)
        );
    }

    #[test]
    fn filter_class_tests_match_the_design_table() {
        let landed = HashSet::new();
        // Boot: partial segments held, terminal accepted.
        let mut f = facts("boot-1");
        assert_eq!(
            filter_candidate(SourceClass::BootRecords, &f, &landed),
            FilterOutcome::Hold(None)
        );
        f.terminal = true;
        assert_eq!(
            filter_candidate(SourceClass::BootRecords, &f, &landed),
            FilterOutcome::Accept
        );
        // Sessions: finalized-but-empty is a no-question reject.
        let mut f = facts("sess-1");
        f.terminal = true;
        assert_eq!(
            filter_candidate(SourceClass::SessionLogs, &f, &landed),
            FilterOutcome::Reject(ReasonCode::NoQuestion)
        );
        f.has_substance = true;
        assert_eq!(
            filter_candidate(SourceClass::SessionLogs, &f, &landed),
            FilterOutcome::Accept
        );
        // Chat: off-allowlist is absence (skip, no record); bots reject.
        let mut f = facts("chat-1");
        assert_eq!(
            filter_candidate(SourceClass::ChatLogs, &f, &landed),
            FilterOutcome::Skip
        );
        f.allowlisted = true;
        f.bot_or_heartbeat = true;
        assert_eq!(
            filter_candidate(SourceClass::ChatLogs, &f, &landed),
            FilterOutcome::Reject(ReasonCode::NoQuestion)
        );
        f.bot_or_heartbeat = false;
        assert_eq!(
            filter_candidate(SourceClass::ChatLogs, &f, &landed),
            FilterOutcome::Accept
        );
        // Statistics: drafts are superseded-version rejects.
        let mut f = facts("stat-1");
        assert_eq!(
            filter_candidate(SourceClass::Statistics, &f, &landed),
            FilterOutcome::Reject(ReasonCode::SupersededVersion)
        );
        f.final_version = true;
        assert_eq!(
            filter_candidate(SourceClass::Statistics, &f, &landed),
            FilterOutcome::Accept
        );
        // Reports: partial writes held.
        let mut f = facts("rep-1");
        f.partial_write = true;
        assert_eq!(
            filter_candidate(SourceClass::Reports, &f, &landed),
            FilterOutcome::Hold(None)
        );
    }

    #[test]
    fn guardrails_fire_at_the_design_thresholds() {
        let base = Measurements {
            src_free_bytes: 500 * GIB,
            src_volume_bytes: 1000 * GIB,
            stg_free_bytes: 500 * GIB,
            projected_batch_bytes: GIB,
            ..Measurements::default()
        };
        assert!(evaluate_guardrails(&base, false).is_empty());
        // Source space low → pause; critical → abort (and not both).
        let mut m = base.clone();
        m.src_free_bytes = 90 * GIB; // below max(10% of 1000G, 5G) = 100G
        let events = evaluate_guardrails(&m, false);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, GuardrailAction::Pause);
        assert_eq!(events[0].alert.as_deref(), Some("guardrail/src-space"));
        m.src_free_bytes = 40 * GIB; // below max(5%, 2G) = 50G
        let events = evaluate_guardrails(&m, false);
        assert_eq!(events[0].action, GuardrailAction::Abort);
        assert_eq!(
            events[0].alert.as_deref(),
            Some("guardrail/src-space-critical")
        );
        // I/O throttle alerts only on the second consecutive run.
        let mut m = base.clone();
        m.src_io_wait_pct = 30.0;
        let events = evaluate_guardrails(&m, false);
        assert_eq!(events[0].action, GuardrailAction::Throttle);
        assert_eq!(events[0].alert, None);
        let events = evaluate_guardrails(&m, true);
        assert_eq!(events[0].alert.as_deref(), Some("guardrail/src-io"));
        // Staging space and write burst.
        let mut m = base.clone();
        m.stg_free_bytes = 9 * GIB;
        let events = evaluate_guardrails(&m, false);
        assert!(
            events
                .iter()
                .any(|e| e.alert.as_deref() == Some("guardrail/stg-space"))
        );
        let mut m = base.clone();
        m.class_median_write_rows = Some(100);
        m.tgt_write_rows = 201;
        let events = evaluate_guardrails(&m, false);
        assert!(
            events
                .iter()
                .any(|e| e.action == GuardrailAction::PauseClass
                    && e.alert.as_deref() == Some("guardrail/tgt-burst"))
        );
    }

    #[test]
    fn headroom_rules_match_the_design_alert_set() {
        let line = |pct: f64, d70: Option<f64>, d100: Option<f64>| EnvelopeLine {
            name: "storage".to_string(),
            pct,
            days_to_70_pct: d70,
            days_to_ceiling: d100,
        };
        assert!(evaluate_headroom(&[line(50.0, Some(200.0), Some(400.0))]).is_empty());
        assert_eq!(
            evaluate_headroom(&[line(50.0, Some(90.0), Some(400.0))])[0].alert,
            "headroom-warning"
        );
        assert_eq!(
            evaluate_headroom(&[line(80.0, None, Some(400.0))])[0].alert,
            "plan/headroom-80"
        );
        assert_eq!(
            evaluate_headroom(&[line(95.0, None, None)])[0].alert,
            "plan/ceiling-stop"
        );
        assert_eq!(
            evaluate_headroom(&[line(60.0, None, Some(20.0))])[0].alert,
            "plan/ceiling-stop"
        );
        assert!(spend_anomaly(201.0, Some(100.0)));
        assert!(!spend_anomaly(200.0, Some(100.0)));
        assert!(!spend_anomaly(201.0, None));
    }

    fn request(class: SourceClass, candidates: Vec<CandidateFacts>) -> RunRequest {
        RunRequest {
            run_id: "run-1".to_string(),
            batch_id: "batch-1".to_string(),
            source_class: class,
            host: "host-a".to_string(),
            at: "2026-11-17T00:00:00Z".to_string(),
            now_unix: 1_000_000,
            candidates,
            item_failures: Vec::new(),
            measurements: Measurements {
                src_free_bytes: 500 * GIB,
                src_volume_bytes: 1000 * GIB,
                stg_free_bytes: 500 * GIB,
                projected_batch_bytes: GIB,
                ..Measurements::default()
            },
            envelope: Vec::new(),
            embed_usd: 0.0,
            embed_usd_median: None,
        }
    }

    #[test]
    fn run_once_filters_records_and_reports() {
        let dir = temp_state_dir("run-once");
        let mut good = facts("sess-good");
        good.terminal = true;
        good.has_substance = true;
        good.content_address = Some("addr-good".to_string());
        let mut empty = facts("sess-empty");
        empty.terminal = true;
        let outcome =
            run_once(&dir, &request(SourceClass::SessionLogs, vec![good, empty])).expect("run");
        assert_eq!(outcome.status, RunStatus::Completed);
        assert_eq!(outcome.records.len(), 2);
        assert_eq!(outcome.records[0].outcome, "accepted");
        assert_eq!(
            outcome.records[1].reason_code.as_deref(),
            Some("no-question")
        );
        let report = outcome.report.expect("report row");
        assert_eq!(report.items_seen, 2);
        assert_eq!(report.items_accepted, 1);
        assert_eq!(report.items_rejected, 1);
        assert_eq!(report.pause_state, "running");
        // Re-running the same batch: the landed address now duplicates.
        let mut again = facts("sess-good");
        again.terminal = true;
        again.has_substance = true;
        again.content_address = Some("addr-good".to_string());
        let outcome =
            run_once(&dir, &request(SourceClass::SessionLogs, vec![again])).expect("rerun");
        assert_eq!(
            outcome.records[0].reason_code.as_deref(),
            Some("duplicate-content")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_once_honours_pause_overlap_and_dead_letter() {
        let dir = temp_state_dir("ops");
        // Operator pause: the run reports PausedClass and stages nothing.
        pause_class(&dir, SourceClass::Reports, "host-a").expect("pause");
        let outcome =
            run_once(&dir, &request(SourceClass::Reports, vec![facts("r1")])).expect("paused run");
        assert_eq!(outcome.status, RunStatus::PausedClass);
        resume_class(&dir, SourceClass::Reports, "host-a").expect("resume");

        // A live lock makes the run an overlap-skip.
        let _held =
            SweepLock::acquire(&dir, SourceClass::Reports, "host-a", "other-run", 1_000_000)
                .expect("lock")
                .expect("acquired");
        let outcome =
            run_once(&dir, &request(SourceClass::Reports, vec![facts("r1")])).expect("overlap run");
        assert_eq!(outcome.status, RunStatus::OverlapSkip);
        drop(_held);

        // Three failure strikes dead-letter the item; retry clears it.
        for _ in 0..DEAD_LETTER_STRIKES {
            let mut req = request(SourceClass::Reports, vec![]);
            req.item_failures = vec!["bad-1".to_string()];
            run_once(&dir, &req).expect("failure run");
        }
        assert_eq!(dead_letter_list(&dir).expect("list"), vec!["bad-1"]);
        dead_letter_retry(&dir, "bad-1").expect("retry");
        assert!(dead_letter_list(&dir).expect("list").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_once_ceiling_stop_pauses_the_class() {
        let dir = temp_state_dir("ceiling");
        let mut req = request(SourceClass::Statistics, vec![]);
        req.envelope = vec![EnvelopeLine {
            name: "storage".to_string(),
            pct: 96.0,
            days_to_70_pct: None,
            days_to_ceiling: Some(10.0),
        }];
        let outcome = run_once(&dir, &req).expect("run");
        assert!(outcome.alerts.iter().any(|a| a == "plan/ceiling-stop"));
        let state = load_state(&dir).expect("state");
        assert!(
            state
                .paused
                .contains(&class_host_key(SourceClass::Statistics, "host-a"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lock_staleness_allows_takeover_after_ttl() {
        let dir = temp_state_dir("lock");
        let first = SweepLock::acquire(&dir, SourceClass::ChatLogs, "h", "r1", 1_000)
            .expect("acquire")
            .expect("first");
        assert!(
            SweepLock::acquire(&dir, SourceClass::ChatLogs, "h", "r2", 1_100)
                .expect("acquire")
                .is_none()
        );
        drop(first);
        // Even without a drop, a lock older than the TTL is stale.
        let _stale = SweepLock::acquire(&dir, SourceClass::ChatLogs, "h", "r3", 2_000)
            .expect("acquire")
            .expect("third");
        assert!(
            SweepLock::acquire(
                &dir,
                SourceClass::ChatLogs,
                "h",
                "r4",
                2_000 + LOCK_TTL_SECONDS + 1
            )
            .expect("acquire")
            .is_some()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
