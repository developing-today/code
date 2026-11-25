//! Public-world safety reports: the signed report fact and its scoped
//! intake (#14464; creator public-world safety v3 outline,
//! `doc/2026-11-24T16-12-00Z_design_id6310_creator_public_world_safety_v3_outline.md`,
//! sections 1, 2, 4 and 6 item 1).
//!
//! A report is a signed, timestamped fact naming the target, the
//! reason class and the reporter. It is **not a verdict**: filing one
//! changes nothing about the target. A reported program stays exactly
//! as published, a reported world keeps running; only a moderator
//! action through the role model ever changes anything (outline
//! section 2, "safety boundaries").
//!
//! Every operator runs the same intake over the targets *it* lists or
//! hosts. There is no central moderation authority and no
//! network-wide verdict: a peer with no hosted instance gets the same
//! report fact and the same intake as our hosted node (outline section
//! 3, #1240).
//!
//! # What may be reported
//!
//! [`ReportTarget`] names the three targets an operator can be
//! responsible for:
//!
//! - [`ReportTarget::ListedProgram`]: a program record this operator's
//!   gallery lists (#6277 / #6278 registry listing, by content hash);
//! - [`ReportTarget::HostedWorld`]: a public world this operator hosts;
//! - [`ReportTarget::HostedProgram`]: a program version a hosted world
//!   has recorded as **published** (the #13742 draft / published fact).
//!   A draft is not public, so it is not reportable here.
//!
//! # Intake
//!
//! [`intake`] checks, in order: the document is well-formed and within
//! bounds, the reporter's signature verifies, the timestamp is not
//! from the future beyond [`MAX_FUTURE_SKEW_MS`], and the target is in
//! this operator's [`OperatorScope`]. A target outside the scope is
//! refused with [`ReportError::OutOfScope`]: an operator only takes
//! reports about what it lists or hosts. Listing determines
//! *eligibility* for a report, never its outcome (outline section 2).
//!
//! # Storage
//!
//! Accepted reports are appended to a [`ReportLog`]: one JSON line per
//! report under the operator directory ([`REPORTS_FILE`]), each line
//! carrying the reporter's signed document unchanged, a local sequence
//! number and the local receipt time. The log is the source of truth;
//! the review queue (#14465) is a projection rebuilt from it. Reports
//! never expire and are never deleted here (outline section 4,
//! question 4): closing one is a moderator decision, logged by the
//! later slices. Reloading re-verifies every line's signature, so a
//! tampered log fails loudly instead of yielding a guessed report.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::program_bundle::{ProgramIdentity, check_identity, parse_hex};
use crate::program_gallery::Gallery;
use crate::program_publish_state::{self, ProgramPublishState};
use crate::world::Records;
use crate::world_session::encode_hex;

/// Domain separator for a reporter's signature.
const REPORT_DOMAIN: &[u8] = b"developing-today/id-safety-report/v1\0";

/// File, under the operator directory, holding accepted reports as one
/// JSON entry per line, appended in sequence order.
pub const REPORTS_FILE: &str = "reports.jsonl";

/// Longest free-text detail on one report, in bytes. The text is shown
/// to a moderator and never used for automated decisions.
pub const MAX_DETAIL_BYTES: usize = 2000;

/// Longest world identifier a report may name, in bytes.
pub const MAX_WORLD_ID_BYTES: usize = 256;

/// Largest signed report document accepted, in bytes.
pub const MAX_REPORT_BYTES: usize = 16 * 1024;

/// How far into the future a report's timestamp may be, relative to
/// the receiving node's clock, before intake refuses it (milliseconds).
pub const MAX_FUTURE_SKEW_MS: u64 = 5 * 60 * 1000;

/// Why a report was filed: a small fixed set (outline section 4,
/// question 2). The class drives review-queue ordering; the free-text
/// detail never does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportReason {
    /// Unsolicited or repetitive content.
    Spam,
    /// Harassment, abuse or otherwise harmful content.
    Abuse,
    /// The target claims to be someone or something it is not.
    Impersonation,
    /// Anything else; the detail says what.
    Other,
}

impl ReportReason {
    /// The wire name of the class.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Spam => "spam",
            Self::Abuse => "abuse",
            Self::Impersonation => "impersonation",
            Self::Other => "other",
        }
    }
}

impl std::fmt::Display for ReportReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ReportReason {
    type Err = ReportError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "spam" => Ok(Self::Spam),
            "abuse" => Ok(Self::Abuse),
            "impersonation" => Ok(Self::Impersonation),
            "other" => Ok(Self::Other),
            other => Err(ReportError::Malformed(format!(
                "unknown reason class {other:?}; expected spam, abuse, impersonation or other"
            ))),
        }
    }
}

/// What a report is about. Always something an operator can list or
/// host; never an author or a person.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReportTarget {
    /// A program record listed by the operator's gallery.
    ListedProgram {
        /// The record's content hash, lower-case hex.
        content_hash: String,
    },
    /// A public world the operator hosts.
    HostedWorld {
        /// The world's identifier.
        world_id: String,
    },
    /// A program version a hosted world has recorded as published.
    HostedProgram {
        /// The hosting world's identifier.
        world_id: String,
        /// The program version (D1 identity).
        program: ProgramIdentity,
    },
}

impl std::fmt::Display for ReportTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ListedProgram { content_hash } => write!(f, "listed program {content_hash}"),
            Self::HostedWorld { world_id } => write!(f, "hosted world {world_id:?}"),
            Self::HostedProgram { world_id, program } => write!(
                f,
                "program {:?} v{} ({}) in hosted world {world_id:?}",
                program.name, program.version, program.content_hash
            ),
        }
    }
}

/// One report, as the reporter states and signs it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    /// The reporter's Ed25519 public key, lower-case hex.
    pub reporter: String,
    /// What the report is about.
    pub target: ReportTarget,
    /// The reason class.
    pub reason: ReportReason,
    /// Free-text detail for the moderator; may be empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    /// When the reporter filed it, Unix milliseconds, as the reporter
    /// signed it.
    pub filed_at_ms: u64,
}

/// A report document with the reporter's signature over it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedReport {
    /// The report's exact JSON, as signed.
    pub report: String,
    /// The reporter's Ed25519 signature, lower-case hex.
    pub signature: String,
}

/// Why a report was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReportError {
    /// The document is malformed or out of bounds.
    #[error("report: {0}")]
    Malformed(String),
    /// The document is not signed by the reporter it names.
    #[error("report signature: not signed by the reporter it names")]
    Signature,
    /// The report claims a filing time too far ahead of this node's
    /// clock.
    #[error(
        "report filed at {filed_at_ms} ms is more than {MAX_FUTURE_SKEW_MS} ms ahead of now ({now_ms} ms)"
    )]
    FromTheFuture {
        /// The signed filing time.
        filed_at_ms: u64,
        /// This node's clock at intake.
        now_ms: u64,
    },
    /// The target is not something this operator lists or hosts.
    #[error(
        "{0} is not in this operator's scope; an operator only takes reports about what it lists or hosts"
    )]
    OutOfScope(String),
    /// Reading or writing the report log failed.
    #[error("report storage: {0}")]
    Storage(String),
}

impl SignedReport {
    /// Sign a report about `target` as `reporter`.
    ///
    /// # Errors
    ///
    /// The target or detail is out of bounds.
    pub fn sign(
        target: ReportTarget,
        reason: ReportReason,
        detail: &str,
        filed_at_ms: u64,
        reporter: &SigningKey,
    ) -> Result<Self, ReportError> {
        let report = Report {
            reporter: encode_hex(&reporter.verifying_key().to_bytes()),
            target,
            reason,
            detail: detail.to_owned(),
            filed_at_ms,
        };
        check_report(&report)?;
        let report =
            serde_json::to_string(&report).map_err(|e| ReportError::Malformed(e.to_string()))?;
        let signature = encode_hex(&reporter.sign(&report_message(&report)).to_bytes());
        Ok(Self { report, signature })
    }

    /// The report, once its bounds and signature verify. This is a
    /// pure offline check; it says nothing about scope.
    ///
    /// # Errors
    ///
    /// The document is malformed, out of bounds, or not signed by the
    /// reporter it names.
    pub fn verify(&self) -> Result<Report, ReportError> {
        if self.report.len() > MAX_REPORT_BYTES {
            return Err(ReportError::Malformed(format!(
                "the document is {} bytes; the limit is {MAX_REPORT_BYTES}",
                self.report.len()
            )));
        }
        let report: Report =
            id_core::json_unique::from_slice_without_duplicate_keys(self.report.as_bytes())
                .map_err(|e| ReportError::Malformed(e.to_string()))?;
        check_report(&report)?;
        let reporter = parse_hex::<32>(&report.reporter)
            .and_then(|bytes| VerifyingKey::from_bytes(&bytes).ok())
            .ok_or(ReportError::Signature)?;
        let signature = parse_hex::<64>(&self.signature)
            .map(|bytes| Signature::from_bytes(&bytes))
            .ok_or(ReportError::Signature)?;
        reporter
            .verify_strict(&report_message(&self.report), &signature)
            .map_err(|_| ReportError::Signature)?;
        Ok(report)
    }

    /// The report's content address: BLAKE3 of the signed document,
    /// lower-case hex. The same report filed twice has the same id.
    #[must_use]
    pub fn id(&self) -> String {
        encode_hex(blake3::hash(self.report.as_bytes()).as_bytes())
    }
}

fn report_message(report: &str) -> Vec<u8> {
    let mut message = REPORT_DOMAIN.to_vec();
    message.extend_from_slice(blake3::hash(report.as_bytes()).as_bytes());
    message
}

fn check_world_id(world_id: &str) -> Result<(), ReportError> {
    if world_id.is_empty()
        || world_id.len() > MAX_WORLD_ID_BYTES
        || world_id.chars().any(char::is_control)
    {
        return Err(ReportError::Malformed(format!(
            "the world id must be 1 to {MAX_WORLD_ID_BYTES} bytes with no control characters"
        )));
    }
    Ok(())
}

fn check_report(report: &Report) -> Result<(), ReportError> {
    match &report.target {
        ReportTarget::ListedProgram { content_hash } => {
            if parse_hex::<32>(content_hash).is_none() {
                return Err(ReportError::Malformed(
                    "the content hash is not 32 bytes of lower-case hex".to_owned(),
                ));
            }
        }
        ReportTarget::HostedWorld { world_id } => check_world_id(world_id)?,
        ReportTarget::HostedProgram { world_id, program } => {
            check_world_id(world_id)?;
            check_identity(program)
                .map_err(|reason| ReportError::Malformed(format!("program: {reason}")))?;
        }
    }
    if report.detail.len() > MAX_DETAIL_BYTES {
        return Err(ReportError::Malformed(format!(
            "the detail is {} bytes; the limit is {MAX_DETAIL_BYTES}",
            report.detail.len()
        )));
    }
    if report
        .detail
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(ReportError::Malformed(
            "the detail may not contain control characters other than newline and tab".to_owned(),
        ));
    }
    Ok(())
}

/// What one operator lists or hosts: the only targets its intake takes
/// reports about.
pub trait OperatorScope {
    /// Whether the operator's gallery lists the record with this
    /// content hash.
    fn lists_program(&self, content_hash: &str) -> bool;

    /// Whether the operator hosts this world.
    fn hosts_world(&self, world_id: &str) -> bool;

    /// Whether the operator hosts this world and the world has recorded
    /// this program version as published.
    fn hosts_published(&self, world_id: &str, program: &ProgramIdentity) -> bool;

    /// Whether `target` is in this scope.
    fn covers(&self, target: &ReportTarget) -> bool {
        match target {
            ReportTarget::ListedProgram { content_hash } => self.lists_program(content_hash),
            ReportTarget::HostedWorld { world_id } => self.hosts_world(world_id),
            ReportTarget::HostedProgram { world_id, program } => {
                self.hosts_published(world_id, program)
            }
        }
    }
}

/// A gallery's scope is the records it lists; it hosts no worlds.
impl OperatorScope for Gallery {
    fn lists_program(&self, content_hash: &str) -> bool {
        self.lists(content_hash)
    }

    fn hosts_world(&self, _world_id: &str) -> bool {
        false
    }

    fn hosts_published(&self, _world_id: &str, _program: &ProgramIdentity) -> bool {
        false
    }
}

/// An operator that both runs a gallery and hosts worlds: either side
/// may cover a target.
impl<A: OperatorScope, B: OperatorScope> OperatorScope for (A, B) {
    fn lists_program(&self, content_hash: &str) -> bool {
        self.0.lists_program(content_hash) || self.1.lists_program(content_hash)
    }

    fn hosts_world(&self, world_id: &str) -> bool {
        self.0.hosts_world(world_id) || self.1.hosts_world(world_id)
    }

    fn hosts_published(&self, world_id: &str, program: &ProgramIdentity) -> bool {
        self.0.hosts_published(world_id, program) || self.1.hosts_published(world_id, program)
    }
}

impl<S: OperatorScope + ?Sized> OperatorScope for &S {
    fn lists_program(&self, content_hash: &str) -> bool {
        (**self).lists_program(content_hash)
    }

    fn hosts_world(&self, world_id: &str) -> bool {
        (**self).hosts_world(world_id)
    }

    fn hosts_published(&self, world_id: &str, program: &ProgramIdentity) -> bool {
        (**self).hosts_published(world_id, program)
    }
}

/// The public worlds an operator hosts, with the program versions each
/// has recorded as published, read from the worlds' own records.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostedScope {
    /// World id to the record keys of its published program versions.
    worlds: BTreeMap<String, BTreeSet<String>>,
}

impl HostedScope {
    /// An empty scope: hosts nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a hosted world, taking its published program versions from
    /// its records (#13742 facts). A version with no fact is a draft
    /// and stays out of scope.
    ///
    /// # Errors
    ///
    /// The world id is out of bounds, or a publish-state record in the
    /// world is malformed or invalid (never read as a guessed state).
    pub fn host(&mut self, world_id: &str, records: &Records) -> Result<(), ReportError> {
        check_world_id(world_id)?;
        let facts = crate::world_records::program_publish_facts(records)
            .map_err(|e| ReportError::Malformed(format!("world {world_id:?}: {e:#}")))?;
        let published = facts
            .into_iter()
            .filter(|fact| fact.state == ProgramPublishState::Published)
            .map(|fact| program_publish_state::record_key(&fact.program))
            .collect();
        self.worlds.insert(world_id.to_owned(), published);
        Ok(())
    }

    /// Stop hosting a world; later reports about it are out of scope.
    pub fn unhost(&mut self, world_id: &str) {
        self.worlds.remove(world_id);
    }

    /// The hosted world ids, in order.
    pub fn worlds(&self) -> impl Iterator<Item = &str> {
        self.worlds.keys().map(String::as_str)
    }
}

impl OperatorScope for HostedScope {
    fn lists_program(&self, _content_hash: &str) -> bool {
        false
    }

    fn hosts_world(&self, world_id: &str) -> bool {
        self.worlds.contains_key(world_id)
    }

    fn hosts_published(&self, world_id: &str, program: &ProgramIdentity) -> bool {
        self.worlds.get(world_id).is_some_and(|published| {
            published.contains(&program_publish_state::record_key(program))
        })
    }
}

/// Check a signed report for intake against `scope` at `now_ms`
/// (Unix milliseconds on this node's clock).
///
/// # Errors
///
/// The document is malformed or out of bounds, the signature does not
/// verify, the filing time is too far in the future, or the target is
/// not in `scope`.
pub fn intake(
    signed: &SignedReport,
    scope: &impl OperatorScope,
    now_ms: u64,
) -> Result<Report, ReportError> {
    let report = signed.verify()?;
    if report.filed_at_ms > now_ms.saturating_add(MAX_FUTURE_SKEW_MS) {
        return Err(ReportError::FromTheFuture {
            filed_at_ms: report.filed_at_ms,
            now_ms,
        });
    }
    if !scope.covers(&report.target) {
        return Err(ReportError::OutOfScope(report.target.to_string()));
    }
    Ok(report)
}

/// One accepted report, as the operator's log keeps it: the reporter's
/// signed document unchanged, plus the operator's own sequence and
/// receipt time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportFact {
    /// Position in this operator's log, from 1.
    pub sequence: u64,
    /// The report's content address ([`SignedReport::id`]).
    pub id: String,
    /// When this node accepted it, Unix milliseconds.
    pub received_at_ms: u64,
    /// The report as signed.
    pub signed: SignedReport,
}

impl ReportFact {
    /// The verified report this fact carries.
    ///
    /// # Errors
    ///
    /// The carried document no longer verifies, or its id does not
    /// match its content.
    pub fn report(&self) -> Result<Report, ReportError> {
        if self.id != self.signed.id() {
            return Err(ReportError::Malformed(format!(
                "fact {} carries a report whose id is {}",
                self.sequence,
                self.signed.id()
            )));
        }
        self.signed.verify()
    }
}

/// The operator's append-only log of accepted reports.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReportLog {
    facts: Vec<ReportFact>,
    by_id: BTreeMap<String, usize>,
}

impl ReportLog {
    /// An empty log in memory.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Load the log under `operator_dir`; a missing file is an empty
    /// log. Every line is re-verified.
    ///
    /// # Errors
    ///
    /// The file cannot be read, a line is malformed, a signature or id
    /// does not verify, or the sequence is not 1, 2, 3 ...
    pub fn load(operator_dir: &Path) -> Result<Self, ReportError> {
        let path = operator_dir.join(REPORTS_FILE);
        let mut log = Self::default();
        if !path.is_file() {
            return Ok(log);
        }
        let text = std::fs::read_to_string(&path).map_err(|e| storage(&path, &e))?;
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            let fact: ReportFact =
                id_core::json_unique::from_slice_without_duplicate_keys(line.as_bytes())
                    .map_err(|e| ReportError::Malformed(format!("{}: {e}", path.display())))?;
            log.check_next(&fact)?;
            log.push(fact);
        }
        Ok(log)
    }

    /// Every accepted report, in sequence order.
    #[must_use]
    pub fn facts(&self) -> &[ReportFact] {
        &self.facts
    }

    /// The fact for the report with this id, if accepted.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&ReportFact> {
        self.by_id.get(id).and_then(|&index| self.facts.get(index))
    }

    /// How many reports the log holds.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.facts.len()
    }

    /// Whether the log is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.facts.is_empty()
    }

    /// Take a signed report through [`intake`] and append it, writing
    /// the line under `operator_dir` when given. Filing the same report
    /// again returns the existing fact and appends nothing.
    ///
    /// # Errors
    ///
    /// Intake refuses the report, or the line cannot be written (the
    /// log in memory is then unchanged).
    pub fn file(
        &mut self,
        signed: SignedReport,
        scope: &impl OperatorScope,
        now_ms: u64,
        operator_dir: Option<&Path>,
    ) -> Result<ReportFact, ReportError> {
        intake(&signed, scope, now_ms)?;
        let id = signed.id();
        if let Some(existing) = self.get(&id) {
            return Ok(existing.clone());
        }
        let fact = ReportFact {
            sequence: self.next_sequence(),
            id,
            received_at_ms: now_ms,
            signed,
        };
        if let Some(dir) = operator_dir {
            append_line(&dir.join(REPORTS_FILE), &fact)?;
        }
        self.push(fact.clone());
        Ok(fact)
    }

    fn next_sequence(&self) -> u64 {
        self.facts
            .last()
            .map_or(1, |fact| fact.sequence.saturating_add(1))
    }

    fn check_next(&self, fact: &ReportFact) -> Result<(), ReportError> {
        if fact.sequence != self.next_sequence() {
            return Err(ReportError::Malformed(format!(
                "report log sequence {} where {} was expected",
                fact.sequence,
                self.next_sequence()
            )));
        }
        fact.report()?;
        if self.by_id.contains_key(&fact.id) {
            return Err(ReportError::Malformed(format!(
                "report {} appears twice in the log",
                fact.id
            )));
        }
        Ok(())
    }

    fn push(&mut self, fact: ReportFact) {
        self.by_id.insert(fact.id.clone(), self.facts.len());
        self.facts.push(fact);
    }
}

fn append_line(path: &Path, fact: &ReportFact) -> Result<(), ReportError> {
    let dir = path
        .parent()
        .ok_or_else(|| ReportError::Storage(format!("{} has no parent", path.display())))?;
    std::fs::create_dir_all(dir).map_err(|e| storage(dir, &e))?;
    let mut line = serde_json::to_vec(fact).map_err(|e| ReportError::Storage(e.to_string()))?;
    line.push(b'\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| storage(path, &e))?;
    std::io::Write::write_all(&mut file, &line).map_err(|e| storage(path, &e))?;
    file.sync_all().map_err(|e| storage(path, &e))
}

fn storage(path: &Path, error: &std::io::Error) -> ReportError {
    ReportError::Storage(format!("{}: {error}", path.display()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use ed25519_dalek::SigningKey;

    use super::*;
    use crate::program_bundle::{ProgramBundle, ProgramSpec, export};
    use crate::program_publish_state::{ProgramPublishFact, record_fact};
    use crate::program_registry::{AdmitAll, Registry};

    const NOW: u64 = 1_795_000_000_000;

    fn reporter() -> SigningKey {
        SigningKey::from_bytes(&[3_u8; 32])
    }

    fn author() -> SigningKey {
        SigningKey::from_bytes(&[7_u8; 32])
    }

    fn bundle(name: &str, version: u64, payload: &[u8]) -> ProgramBundle {
        export(
            payload.to_vec(),
            ProgramSpec {
                name,
                version,
                licence: None,
                description: None,
                categories: &[],
            },
            &author(),
        )
        .unwrap()
    }

    fn gallery_listing(bundle: &ProgramBundle) -> Gallery {
        let mut gallery = Gallery::new(Registry::in_memory(AdmitAll));
        gallery.publish(&bundle.to_bytes()).unwrap();
        gallery
    }

    fn report(target: ReportTarget, reason: ReportReason) -> SignedReport {
        SignedReport::sign(
            target,
            reason,
            "seen in the lobby",
            NOW - 1_000,
            &reporter(),
        )
        .unwrap()
    }

    #[test]
    fn a_report_is_a_signed_timestamped_fact_naming_target_reason_and_reporter() {
        let target = ReportTarget::HostedWorld {
            world_id: "quiz-hall".to_owned(),
        };
        let signed = report(target.clone(), ReportReason::Spam);
        let report = signed.verify().unwrap();
        assert_eq!(report.target, target);
        assert_eq!(report.reason, ReportReason::Spam);
        assert_eq!(report.filed_at_ms, NOW - 1_000);
        assert_eq!(
            report.reporter,
            encode_hex(&reporter().verifying_key().to_bytes())
        );
        assert_eq!(report.detail, "seen in the lobby");
        // The id is the content address: stable for the same document.
        assert_eq!(signed.id(), signed.clone().id());
        assert_eq!(signed.id().len(), 64);
    }

    #[test]
    fn tampered_or_unsigned_reports_are_refused() {
        let signed = report(
            ReportTarget::HostedWorld {
                world_id: "quiz-hall".to_owned(),
            },
            ReportReason::Abuse,
        );
        let mut tampered = signed.clone();
        tampered.report = tampered.report.replace("abuse", "spam");
        assert_eq!(tampered.verify(), Err(ReportError::Signature));

        let mut forged = signed.clone();
        forged.signature = "00".repeat(64);
        assert_eq!(forged.verify(), Err(ReportError::Signature));

        let mut garbage = signed;
        garbage.report = "{\"reporter\":1}".to_owned();
        assert!(matches!(garbage.verify(), Err(ReportError::Malformed(_))));
    }

    #[test]
    fn out_of_bounds_reports_are_refused_at_signing() {
        let long = "x".repeat(MAX_DETAIL_BYTES + 1);
        let err = SignedReport::sign(
            ReportTarget::HostedWorld {
                world_id: "w".to_owned(),
            },
            ReportReason::Other,
            &long,
            NOW,
            &reporter(),
        )
        .unwrap_err();
        assert!(matches!(err, ReportError::Malformed(_)), "{err}");

        let err = SignedReport::sign(
            ReportTarget::HostedWorld {
                world_id: String::new(),
            },
            ReportReason::Other,
            "",
            NOW,
            &reporter(),
        )
        .unwrap_err();
        assert!(matches!(err, ReportError::Malformed(_)), "{err}");

        let err = SignedReport::sign(
            ReportTarget::ListedProgram {
                content_hash: "not-hex".to_owned(),
            },
            ReportReason::Spam,
            "",
            NOW,
            &reporter(),
        )
        .unwrap_err();
        assert!(matches!(err, ReportError::Malformed(_)), "{err}");
    }

    #[test]
    fn intake_accepts_a_program_the_gallery_lists() {
        let quiz = bundle("quiz", 1, b"quiz payload v1");
        let gallery = gallery_listing(&quiz);
        let signed = report(
            ReportTarget::ListedProgram {
                content_hash: quiz.identity().content_hash,
            },
            ReportReason::Impersonation,
        );
        let report = intake(&signed, &gallery, NOW).unwrap();
        assert_eq!(report.reason, ReportReason::Impersonation);
    }

    #[test]
    fn intake_refuses_a_program_the_gallery_does_not_list() {
        let quiz = bundle("quiz", 1, b"quiz payload v1");
        let other = bundle("other", 1, b"other payload");
        let gallery = gallery_listing(&quiz);
        let signed = report(
            ReportTarget::ListedProgram {
                content_hash: other.identity().content_hash,
            },
            ReportReason::Spam,
        );
        let err = intake(&signed, &gallery, NOW).unwrap_err();
        assert!(matches!(err, ReportError::OutOfScope(_)), "{err}");
    }

    #[test]
    fn intake_scopes_hosted_worlds_and_only_their_published_programs() {
        let published = bundle("quiz", 1, b"quiz payload v1");
        let draft = bundle("quiz", 2, b"quiz payload v2");
        let mut records = Records::new();
        record_fact(
            &mut records,
            &ProgramPublishFact::published(published.identity(), published.record()).unwrap(),
        )
        .unwrap();
        record_fact(&mut records, &ProgramPublishFact::draft(draft.identity())).unwrap();

        let mut hosted = HostedScope::new();
        hosted.host("quiz-hall", &records).unwrap();

        // The hosted world itself.
        let world = report(
            ReportTarget::HostedWorld {
                world_id: "quiz-hall".to_owned(),
            },
            ReportReason::Abuse,
        );
        intake(&world, &hosted, NOW).unwrap();

        // A published version in it.
        let program = report(
            ReportTarget::HostedProgram {
                world_id: "quiz-hall".to_owned(),
                program: published.identity(),
            },
            ReportReason::Spam,
        );
        intake(&program, &hosted, NOW).unwrap();

        // A draft is not public, so it is not reportable here.
        let draft_report = report(
            ReportTarget::HostedProgram {
                world_id: "quiz-hall".to_owned(),
                program: draft.identity(),
            },
            ReportReason::Spam,
        );
        assert!(matches!(
            intake(&draft_report, &hosted, NOW),
            Err(ReportError::OutOfScope(_))
        ));

        // A world this operator does not host.
        let elsewhere = report(
            ReportTarget::HostedWorld {
                world_id: "someone-elses".to_owned(),
            },
            ReportReason::Abuse,
        );
        assert!(matches!(
            intake(&elsewhere, &hosted, NOW),
            Err(ReportError::OutOfScope(_))
        ));

        // Unhosting takes the world out of scope.
        hosted.unhost("quiz-hall");
        assert!(matches!(
            intake(&world, &hosted, NOW),
            Err(ReportError::OutOfScope(_))
        ));
    }

    #[test]
    fn a_gallery_and_hosted_worlds_combine_into_one_scope() {
        let quiz = bundle("quiz", 1, b"quiz payload v1");
        let gallery = gallery_listing(&quiz);
        let mut hosted = HostedScope::new();
        hosted.host("quiz-hall", &Records::new()).unwrap();
        let scope = (&gallery, &hosted);

        intake(
            &report(
                ReportTarget::ListedProgram {
                    content_hash: quiz.identity().content_hash,
                },
                ReportReason::Spam,
            ),
            &scope,
            NOW,
        )
        .unwrap();
        intake(
            &report(
                ReportTarget::HostedWorld {
                    world_id: "quiz-hall".to_owned(),
                },
                ReportReason::Spam,
            ),
            &scope,
            NOW,
        )
        .unwrap();
    }

    #[test]
    fn intake_refuses_a_report_from_the_future() {
        let mut hosted = HostedScope::new();
        hosted.host("quiz-hall", &Records::new()).unwrap();
        let target = ReportTarget::HostedWorld {
            world_id: "quiz-hall".to_owned(),
        };
        let late = SignedReport::sign(
            target.clone(),
            ReportReason::Spam,
            "",
            NOW + MAX_FUTURE_SKEW_MS,
            &reporter(),
        )
        .unwrap();
        intake(&late, &hosted, NOW).unwrap();
        let future = SignedReport::sign(
            target,
            ReportReason::Spam,
            "",
            NOW + MAX_FUTURE_SKEW_MS + 1,
            &reporter(),
        )
        .unwrap();
        assert!(matches!(
            intake(&future, &hosted, NOW),
            Err(ReportError::FromTheFuture { .. })
        ));
    }

    #[test]
    fn reporting_changes_nothing_about_the_target() {
        let quiz = bundle("quiz", 1, b"quiz payload v1");
        let gallery = gallery_listing(&quiz);
        let hash = quiz.identity().content_hash;
        let before = gallery.fetch(&hash);
        let mut log = ReportLog::new();
        log.file(
            report(
                ReportTarget::ListedProgram {
                    content_hash: hash.clone(),
                },
                ReportReason::Abuse,
            ),
            &gallery,
            NOW,
            None,
        )
        .unwrap();
        assert_eq!(gallery.fetch(&hash), before);
        assert!(gallery.lists(&hash));
    }

    #[test]
    fn the_log_appends_dedupes_and_reloads_verified() {
        let dir = tempfile::tempdir().unwrap();
        let mut hosted = HostedScope::new();
        hosted.host("quiz-hall", &Records::new()).unwrap();
        let first = report(
            ReportTarget::HostedWorld {
                world_id: "quiz-hall".to_owned(),
            },
            ReportReason::Spam,
        );
        let second = SignedReport::sign(
            ReportTarget::HostedWorld {
                world_id: "quiz-hall".to_owned(),
            },
            ReportReason::Abuse,
            "",
            NOW,
            &SigningKey::from_bytes(&[9_u8; 32]),
        )
        .unwrap();

        let mut log = ReportLog::new();
        let a = log
            .file(first.clone(), &hosted, NOW, Some(dir.path()))
            .unwrap();
        let again = log
            .file(first.clone(), &hosted, NOW + 5, Some(dir.path()))
            .unwrap();
        assert_eq!(a, again, "filing the same report twice appends nothing");
        let b = log
            .file(second, &hosted, NOW + 10, Some(dir.path()))
            .unwrap();
        assert_eq!((a.sequence, b.sequence), (1, 2));
        assert_eq!(a.received_at_ms, NOW);

        // An out-of-scope report leaves the log untouched.
        let refused = report(
            ReportTarget::HostedWorld {
                world_id: "elsewhere".to_owned(),
            },
            ReportReason::Spam,
        );
        assert!(log.file(refused, &hosted, NOW, Some(dir.path())).is_err());
        assert_eq!(log.len(), 2);

        let reloaded = ReportLog::load(dir.path()).unwrap();
        assert_eq!(reloaded, log);
        assert_eq!(reloaded.get(&first.id()), Some(&a));
    }

    #[test]
    fn a_tampered_log_line_fails_to_load() {
        let dir = tempfile::tempdir().unwrap();
        let mut hosted = HostedScope::new();
        hosted.host("quiz-hall", &Records::new()).unwrap();
        let mut log = ReportLog::new();
        log.file(
            report(
                ReportTarget::HostedWorld {
                    world_id: "quiz-hall".to_owned(),
                },
                ReportReason::Spam,
            ),
            &hosted,
            NOW,
            Some(dir.path()),
        )
        .unwrap();
        let path = dir.path().join(REPORTS_FILE);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replace("spam", "abuse")).unwrap();
        assert!(ReportLog::load(dir.path()).is_err());
    }

    #[test]
    fn reason_classes_round_trip_by_name() {
        for reason in [
            ReportReason::Spam,
            ReportReason::Abuse,
            ReportReason::Impersonation,
            ReportReason::Other,
        ] {
            assert_eq!(reason.as_str().parse::<ReportReason>().unwrap(), reason);
        }
        assert!("takedown".parse::<ReportReason>().is_err());
    }
}
