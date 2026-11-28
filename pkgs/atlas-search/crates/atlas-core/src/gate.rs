//! The staging-shard ingest gate's record and state machine
//! (developing-today/code#7709, follow-on 1 of #6057; design
//! `doc/2026-11-08T13-45-00Z_design_atlas_staging_shard_ingest_gate`).
//!
//! The gate judges one *batch* (a staging unit, or one increment of a
//! long-lived unit) through five ordered stages: write, classify, dedup,
//! provenance-check, publish. This module is the audit trail the gate
//! exists to produce and the rules that make every later stage checkable:
//!
//! * **Gate record.** One durable row per batch in the staging registry
//!   DB ([`Registry::save_gate_record`]): batch and unit ids, tier, state,
//!   stage reached, the verdict for each stage with its reason codes, row
//!   counts, flagged row ids and stage-input hash, strike counts, and the
//!   publish manifest id once published.
//! * **State machine.** `open → closing → gated → published | quarantined`,
//!   and `quarantined → gated` on retry. No other transition exists.
//! * **Fail-closed.** A stage with no recorded verdict has not passed.
//!   Stages are recorded strictly in order, so no stage can be judged
//!   before its predecessor passed. `skip` is allowed only where design
//!   section 4 says so (write on an empty batch, publish with zero
//!   survivors and no occurrence additions pending), always carries an
//!   enumerated reason, and never yields a publish manifest.
//! * **Bounded quarantine.** A batch quarantined on the same reason code
//!   three times is marked `needs_review` and refuses further automatic
//!   retries (design 5.5).
//! * **Resume at the failing stage.** A retry re-enters at the stage that
//!   failed, or at write for `write_invalid` / `write_bypass` (the
//!   producer must re-write). If the input identity of any earlier stage
//!   changed, it re-enters at the earliest such stage instead, and every
//!   verdict from there on is discarded: changed bytes never inherit a
//!   verdict obtained for different content (design 5.3).
//!
//! **Never content (design section 8).** Nothing in a record can carry
//! batch content: reason codes and skip reasons are closed enums, row and
//! batch ids must match a short identifier grammar, and input hashes must
//! be lowercase hex SHA-256. A secret-tier batch's record is therefore
//! readable without reproducing the material that caused a quarantine.
//!
//! Quarantine is a state, not a deletion: nothing here touches the
//! batch's staging DB. The five stage checks themselves (follow-ons 2 to
//! 5 of #6057) record their verdicts through [`GateRecord::record`].

use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::shards::Tier;
use crate::stage::Registry;

/// The gate record table's v1 DDL artifact
/// (`pkgs/atlas-search/schema/gate_records_v1.sql`).
pub const GATE_RECORDS_V1_DDL: &str = include_str!("../../../schema/gate_records_v1.sql");

/// Strikes on one reason code after which a batch stops auto-retrying.
pub const STRIKE_LIMIT: u32 = 3;

/// The five gate stages, in the order they run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateStage {
    Write,
    Classify,
    Dedup,
    ProvenanceCheck,
    Publish,
}

impl GateStage {
    pub const ALL: [GateStage; 5] = [
        GateStage::Write,
        GateStage::Classify,
        GateStage::Dedup,
        GateStage::ProvenanceCheck,
        GateStage::Publish,
    ];

    pub fn name(self) -> &'static str {
        match self {
            GateStage::Write => "write",
            GateStage::Classify => "classify",
            GateStage::Dedup => "dedup",
            GateStage::ProvenanceCheck => "provenance_check",
            GateStage::Publish => "publish",
        }
    }

    pub fn parse(s: &str) -> Option<GateStage> {
        GateStage::ALL.into_iter().find(|stage| stage.name() == s)
    }
}

impl std::fmt::Display for GateStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// A batch's lifecycle state (design section 9, follow-on 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchState {
    Open,
    Closing,
    Gated,
    Published,
    Quarantined,
}

impl BatchState {
    pub fn name(self) -> &'static str {
        match self {
            BatchState::Open => "open",
            BatchState::Closing => "closing",
            BatchState::Gated => "gated",
            BatchState::Published => "published",
            BatchState::Quarantined => "quarantined",
        }
    }

    pub fn parse(s: &str) -> Option<BatchState> {
        [
            BatchState::Open,
            BatchState::Closing,
            BatchState::Gated,
            BatchState::Published,
            BatchState::Quarantined,
        ]
        .into_iter()
        .find(|state| state.name() == s)
    }
}

impl std::fmt::Display for BatchState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// The enumerated failure classes of design section 4. Each belongs to
/// exactly one stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
    WriteInvalid,
    WriteBypass,
    ClassifyUnresolved,
    DedupUndispositioned,
    ProvenanceUnresolvable,
    PublishPartial,
}

impl ReasonCode {
    pub fn stage(self) -> GateStage {
        match self {
            ReasonCode::WriteInvalid | ReasonCode::WriteBypass => GateStage::Write,
            ReasonCode::ClassifyUnresolved => GateStage::Classify,
            ReasonCode::DedupUndispositioned => GateStage::Dedup,
            ReasonCode::ProvenanceUnresolvable => GateStage::ProvenanceCheck,
            ReasonCode::PublishPartial => GateStage::Publish,
        }
    }

    /// Whether a retry must start over at write (the producer re-writes).
    pub fn requires_rewrite(self) -> bool {
        matches!(self, ReasonCode::WriteInvalid | ReasonCode::WriteBypass)
    }
}

/// The only reasons a stage may be skipped (design section 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// Write: the batch has no rows.
    EmptyBatch,
    /// Publish: no chunk survived dedup suppression and no occurrence
    /// additions are pending. A unit whose chunks were all suppressed
    /// into occurrences is not skippable; it publishes.
    ZeroSurvivorsNoOccurrences,
}

impl SkipReason {
    pub fn stage(self) -> GateStage {
        match self {
            SkipReason::EmptyBatch => GateStage::Write,
            SkipReason::ZeroSurvivorsNoOccurrences => GateStage::Publish,
        }
    }
}

/// One stage's outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "verdict")]
pub enum Verdict {
    Pass,
    Fail { reasons: Vec<ReasonCode> },
    Skip { reason: SkipReason },
}

/// A stage's recorded judgement, bound to the identity of its input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageVerdict {
    #[serde(flatten)]
    pub verdict: Verdict,
    /// Lowercase hex SHA-256 of the stage's input (design 5.3).
    pub input_hash: String,
    pub rows_in: u64,
    pub rows_out: u64,
    /// Ids of the rows a failure names, never their content.
    pub row_ids: Vec<String>,
}

impl StageVerdict {
    fn passed(&self) -> bool {
        !matches!(self.verdict, Verdict::Fail { .. })
    }
}

/// The durable per-batch gate record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateRecord {
    pub batch_id: String,
    pub unit_id: String,
    pub tier: Tier,
    pub state: BatchState,
    pub verdicts: BTreeMap<GateStage, StageVerdict>,
    pub strikes: BTreeMap<ReasonCode, u32>,
    pub needs_review: bool,
    pub publish_manifest_id: Option<String>,
}

fn refuse(batch: &str, msg: impl std::fmt::Display) -> Error {
    Error::fatal(format!("gate: batch {batch:?}: {msg}"))
}

/// Ids in a gate record are identifiers, never free text.
fn is_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':' | b'/'))
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl GateRecord {
    /// A new record for a batch, in state `open`.
    pub fn new(batch_id: &str, unit_id: &str, tier: Tier) -> Result<GateRecord> {
        if !is_id(batch_id) || !is_id(unit_id) {
            return Err(refuse(batch_id, "batch and unit ids must be identifiers"));
        }
        Ok(GateRecord {
            batch_id: batch_id.to_owned(),
            unit_id: unit_id.to_owned(),
            tier,
            state: BatchState::Open,
            verdicts: BTreeMap::new(),
            strikes: BTreeMap::new(),
            needs_review: false,
            publish_manifest_id: None,
        })
    }

    fn transition(&mut self, from: BatchState, to: BatchState) -> Result<()> {
        if self.state != from {
            return Err(refuse(
                &self.batch_id,
                format!("cannot move to {to} from {} (needs {from})", self.state),
            ));
        }
        self.state = to;
        Ok(())
    }

    /// `open → closing`: the producer is done writing this batch.
    pub fn close(&mut self) -> Result<()> {
        self.transition(BatchState::Open, BatchState::Closing)
    }

    /// `closing → gated`: the gate starts judging the batch.
    pub fn begin_gate(&mut self) -> Result<()> {
        self.transition(BatchState::Closing, BatchState::Gated)
    }

    /// Whether `stage` has a recorded pass or allowed skip. A stage with
    /// no verdict has not passed (fail-closed).
    pub fn passed(&self, stage: GateStage) -> bool {
        self.verdicts.get(&stage).is_some_and(StageVerdict::passed)
    }

    /// The first stage without a passing verdict, or `None` when all five
    /// passed.
    pub fn next_stage(&self) -> Option<GateStage> {
        GateStage::ALL
            .into_iter()
            .find(|&stage| !self.passed(stage))
    }

    /// The furthest stage with any recorded verdict.
    pub fn stage_reached(&self) -> Option<GateStage> {
        self.verdicts.keys().next_back().copied()
    }

    /// The stage that failed, while quarantined.
    pub fn failing_stage(&self) -> Option<GateStage> {
        self.verdicts
            .iter()
            .find(|(_, v)| !v.passed())
            .map(|(&stage, _)| stage)
    }

    /// Record `stage`'s verdict. The batch must be `gated` and `stage`
    /// must be the next unpassed stage. A failure quarantines the batch
    /// and counts a strike against each of its reason codes.
    pub fn record(&mut self, stage: GateStage, verdict: StageVerdict) -> Result<()> {
        let batch = self.batch_id.clone();
        if self.state != BatchState::Gated {
            return Err(refuse(
                &batch,
                format!("cannot record a verdict while {}", self.state),
            ));
        }
        if self.next_stage() != Some(stage) {
            return Err(refuse(
                &batch,
                format!(
                    "{stage} is out of order; the next stage is {:?}",
                    self.next_stage().map(GateStage::name)
                ),
            ));
        }
        if !is_sha256_hex(&verdict.input_hash) {
            return Err(refuse(&batch, "input hash must be lowercase hex SHA-256"));
        }
        if let Some(bad) = verdict.row_ids.iter().find(|id| !is_id(id)) {
            return Err(refuse(
                &batch,
                format!("row id of {} bytes is not an identifier", bad.len()),
            ));
        }
        match &verdict.verdict {
            Verdict::Pass => {}
            Verdict::Skip { reason } => {
                if reason.stage() != stage {
                    return Err(refuse(
                        &batch,
                        format!("{stage} may not be skipped for {reason:?}"),
                    ));
                }
            }
            Verdict::Fail { reasons } => {
                if reasons.is_empty() {
                    return Err(refuse(&batch, "a failure needs at least one reason code"));
                }
                if let Some(code) = reasons.iter().find(|code| code.stage() != stage) {
                    return Err(refuse(&batch, format!("{code:?} is not a {stage} failure")));
                }
            }
        }
        let failed = match &verdict.verdict {
            Verdict::Fail { reasons } => Some(reasons.clone()),
            _ => None,
        };
        self.verdicts.insert(stage, verdict);
        if let Some(reasons) = failed {
            self.state = BatchState::Quarantined;
            let mut seen = reasons;
            seen.sort();
            seen.dedup();
            for code in seen {
                let strikes = self.strikes.entry(code).or_insert(0);
                *strikes += 1;
                if *strikes >= STRIKE_LIMIT {
                    self.needs_review = true;
                }
            }
        }
        Ok(())
    }

    /// `gated → published`. Every stage must have passed; a published
    /// publish stage names its manifest, a skipped one names none.
    pub fn publish(&mut self, manifest_id: Option<&str>) -> Result<()> {
        let batch = self.batch_id.clone();
        if self.state != BatchState::Gated {
            return Err(refuse(
                &batch,
                format!("cannot publish while {}", self.state),
            ));
        }
        if let Some(stage) = self.next_stage() {
            return Err(refuse(&batch, format!("{stage} has not passed")));
        }
        let skipped = matches!(
            self.verdicts.get(&GateStage::Publish).map(|v| &v.verdict),
            Some(Verdict::Skip { .. })
        );
        match (skipped, manifest_id) {
            (false, Some(id)) if is_id(id) => self.publish_manifest_id = Some(id.to_owned()),
            (false, _) => return Err(refuse(&batch, "a published batch needs its manifest id")),
            (true, None) => {}
            (true, Some(_)) => return Err(refuse(&batch, "a skipped publish has no manifest")),
        }
        self.state = BatchState::Published;
        Ok(())
    }

    /// `quarantined → gated`: retry a quarantined batch. `input_hashes`
    /// gives the current input identity of every stage up to the failing
    /// one. Returns the stage the retry re-enters at; every verdict from
    /// that stage on is discarded.
    pub fn retry(&mut self, input_hashes: &BTreeMap<GateStage, String>) -> Result<GateStage> {
        let batch = self.batch_id.clone();
        if self.state != BatchState::Quarantined {
            return Err(refuse(&batch, format!("cannot retry while {}", self.state)));
        }
        if self.needs_review {
            return Err(refuse(
                &batch,
                "needs review after repeated quarantine; not retrying",
            ));
        }
        let failing = self
            .failing_stage()
            .ok_or_else(|| refuse(&batch, "quarantined without a failing stage"))?;
        let rewrite = matches!(
            &self.verdicts[&failing].verdict,
            Verdict::Fail { reasons } if reasons.iter().any(|code| code.requires_rewrite())
        );
        let mut resume = if rewrite { GateStage::Write } else { failing };
        for (&stage, verdict) in self.verdicts.range(..=failing) {
            let current = input_hashes.get(&stage).ok_or_else(|| {
                refuse(
                    &batch,
                    format!("retry needs the current {stage} input hash"),
                )
            })?;
            if !is_sha256_hex(current) {
                return Err(refuse(&batch, "input hash must be lowercase hex SHA-256"));
            }
            if *current != verdict.input_hash && stage < resume {
                resume = stage;
            }
        }
        self.verdicts.retain(|&stage, _| stage < resume);
        self.state = BatchState::Gated;
        Ok(resume)
    }
}

fn ensure_gate_table(registry: &Registry) -> Result<()> {
    registry
        .conn()
        .execute_batch(GATE_RECORDS_V1_DDL)
        .map_err(|e| Error::backend(format!("gate: applying v1 DDL: {e}")))
}

fn to_json<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|e| Error::backend(format!("gate: encoding record: {e}")))
}

impl Registry {
    /// Write (insert or replace) a batch's gate record.
    pub fn save_gate_record(&self, record: &GateRecord, now: &str) -> Result<()> {
        ensure_gate_table(self)?;
        let verdicts: BTreeMap<&str, &StageVerdict> = record
            .verdicts
            .iter()
            .map(|(stage, v)| (stage.name(), v))
            .collect();
        self.conn()
            .execute(
                "INSERT INTO gate_records (batch_id, unit_id, tier, state, stage_reached, verdicts_json, strikes_json, needs_review, publish_manifest_id, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT (batch_id) DO UPDATE SET unit_id = excluded.unit_id, tier = excluded.tier, state = excluded.state,
                   stage_reached = excluded.stage_reached, verdicts_json = excluded.verdicts_json, strikes_json = excluded.strikes_json,
                   needs_review = excluded.needs_review, publish_manifest_id = excluded.publish_manifest_id, updated_at = excluded.updated_at",
                params![
                    record.batch_id,
                    record.unit_id,
                    record.tier.name(),
                    record.state.name(),
                    record.stage_reached().map(GateStage::name),
                    to_json(&verdicts)?,
                    to_json(&record.strikes.iter().collect::<Vec<_>>())?,
                    i64::from(record.needs_review),
                    record.publish_manifest_id,
                    now,
                ],
            )
            .map_err(|e| Error::backend(format!("gate: saving {}: {e}", record.batch_id)))?;
        Ok(())
    }

    /// Read a batch's gate record. `None` means no record, which the gate
    /// treats as not passed, never as success.
    pub fn load_gate_record(&self, batch_id: &str) -> Result<Option<GateRecord>> {
        ensure_gate_table(self)?;
        type Row = (String, String, String, String, String, i64, Option<String>);
        let row: Option<Row> = self
            .conn()
            .query_row(
                "SELECT unit_id, tier, state, verdicts_json, strikes_json, needs_review, publish_manifest_id FROM gate_records WHERE batch_id = ?1",
                params![batch_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
            )
            .optional()
            .map_err(|e| Error::backend(format!("gate: loading {batch_id}: {e}")))?;
        let Some((unit_id, tier, state, verdicts, strikes, needs_review, manifest)) = row else {
            return Ok(None);
        };
        let corrupt =
            |what: &str| Error::backend(format!("gate: record {batch_id:?} has a corrupt {what}"));
        let named: BTreeMap<String, StageVerdict> =
            serde_json::from_str(&verdicts).map_err(|_| corrupt("verdict set"))?;
        let mut parsed = BTreeMap::new();
        for (name, verdict) in named {
            parsed.insert(
                GateStage::parse(&name).ok_or_else(|| corrupt("stage name"))?,
                verdict,
            );
        }
        let strikes: Vec<(ReasonCode, u32)> =
            serde_json::from_str(&strikes).map_err(|_| corrupt("strike count"))?;
        Ok(Some(GateRecord {
            batch_id: batch_id.to_owned(),
            unit_id,
            tier: Tier::parse(&tier).ok_or_else(|| corrupt("tier"))?,
            state: BatchState::parse(&state).ok_or_else(|| corrupt("state"))?,
            verdicts: parsed,
            strikes: strikes.into_iter().collect(),
            needs_review: needs_review != 0,
            publish_manifest_id: manifest,
        }))
    }
}
