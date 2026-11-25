//! The operator's safety review queue: a local, rebuildable,
//! policy-ordered projection over report facts, where a report closes
//! only by a moderator's decision and every closure is logged
//! (#14465; creator public-world safety v3 outline,
//! `doc/2026-11-24T16-12-00Z_design_id6310_creator_public_world_safety_v3_outline.md`,
//! sections 1, 2, 4 and 6 item 2).
//!
//! The queue is never a second source of truth. It is rebuilt from two
//! append-only logs the operator already holds:
//!
//! - the [`ReportLog`](crate::safety_report::ReportLog) of accepted
//!   reports (#14464), and
//! - the [`ClosureLog`] of moderator decisions, one JSON line per
//!   closure under the operator directory ([`CLOSURES_FILE`]).
//!
//! [`ReviewQueue::rebuild`] joins the two and orders the open reports by
//! the operator's own [`QueuePolicy`]: reason class first (in the order
//! the policy states), then the reporter's filing time, then the local
//! sequence. Ordering uses local data only, and the free-text detail is
//! never an input (outline section 4, question 2).
//!
//! # Closing a report
//!
//! Reports never expire and never close themselves (outline section 4,
//! question 4). The only way out of the queue is a [`SignedClosure`]: a
//! decision signed by a moderator key that the operator's
//! [`ModeratorAuthority`] recognises, naming the report it closes and
//! the outcome. [`ClosureLog::close`] checks the signature, the
//! authority, that the report exists and that it is still open, then
//! appends the closure. A refused closure leaves both logs unchanged.
//! Closing a report records a decision; it changes nothing about the
//! target itself — any action on the target goes through the role
//! model's own action path.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::program_bundle::parse_hex;
use crate::safety_report::{Report, ReportFact, ReportLog, ReportReason};
use crate::world_session::encode_hex;

/// Domain separator for a moderator's closure signature.
const CLOSURE_DOMAIN: &[u8] = b"developing-today/id-safety-closure/v1\0";

/// File, under the operator directory, holding closures as one JSON
/// entry per line, appended in sequence order.
pub const CLOSURES_FILE: &str = "closures.jsonl";

/// Longest note on one closure, in bytes.
pub const MAX_CLOSURE_NOTE_BYTES: usize = 2000;

/// Largest signed closure document accepted, in bytes.
pub const MAX_CLOSURE_BYTES: usize = 8 * 1024;

/// What the moderator decided about a report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClosureDecision {
    /// The report was acted on through the role model's action path.
    Actioned,
    /// The moderator reviewed the report and took no action.
    NoAction,
    /// The report repeats another report already handled.
    Duplicate,
}

/// One moderator decision closing one report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Closure {
    /// The moderator's Ed25519 public key, lower-case hex.
    pub moderator: String,
    /// The id of the report this closes ([`crate::safety_report::SignedReport::id`]).
    pub report_id: String,
    /// The decision.
    pub decision: ClosureDecision,
    /// The moderator's note; may be empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// When the moderator decided, Unix milliseconds, as signed.
    pub decided_at_ms: u64,
}

/// A closure document with the moderator's signature over it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedClosure {
    /// The closure's exact JSON, as signed.
    pub closure: String,
    /// The moderator's Ed25519 signature, lower-case hex.
    pub signature: String,
}

/// Why a closure was refused or the queue could not be built.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum QueueError {
    /// The closure document is malformed or out of bounds.
    #[error("closure: {0}")]
    Malformed(String),
    /// The closure is not signed by the moderator it names.
    #[error("closure signature: not signed by the moderator it names")]
    Signature,
    /// The signing key is not one of this operator's moderators.
    #[error("{0} is not a moderator here; only a moderator's decision closes a report")]
    NotModerator(String),
    /// The closure names a report this operator never accepted.
    #[error("no report {0} in this operator's log")]
    UnknownReport(String),
    /// The report was already closed.
    #[error("report {0} is already closed")]
    AlreadyClosed(String),
    /// A report in the log no longer verifies.
    #[error("report log: {0}")]
    Report(String),
    /// Reading or writing the closure log failed.
    #[error("closure storage: {0}")]
    Storage(String),
}

impl SignedClosure {
    /// Sign a decision on report `report_id` as `moderator`.
    ///
    /// # Errors
    ///
    /// The report id or note is out of bounds.
    pub fn sign(
        report_id: &str,
        decision: ClosureDecision,
        note: &str,
        decided_at_ms: u64,
        moderator: &SigningKey,
    ) -> Result<Self, QueueError> {
        let closure = Closure {
            moderator: encode_hex(&moderator.verifying_key().to_bytes()),
            report_id: report_id.to_owned(),
            decision,
            note: note.to_owned(),
            decided_at_ms,
        };
        check_closure(&closure)?;
        let closure =
            serde_json::to_string(&closure).map_err(|e| QueueError::Malformed(e.to_string()))?;
        let signature = encode_hex(&moderator.sign(&closure_message(&closure)).to_bytes());
        Ok(Self { closure, signature })
    }

    /// The closure, once its bounds and signature verify. This says
    /// nothing about whether the key is a moderator here.
    ///
    /// # Errors
    ///
    /// The document is malformed, out of bounds, or not signed by the
    /// moderator it names.
    pub fn verify(&self) -> Result<Closure, QueueError> {
        if self.closure.len() > MAX_CLOSURE_BYTES {
            return Err(QueueError::Malformed(format!(
                "the document is {} bytes; the limit is {MAX_CLOSURE_BYTES}",
                self.closure.len()
            )));
        }
        let closure: Closure =
            id_core::json_unique::from_slice_without_duplicate_keys(self.closure.as_bytes())
                .map_err(|e| QueueError::Malformed(e.to_string()))?;
        check_closure(&closure)?;
        let moderator = parse_hex::<32>(&closure.moderator)
            .and_then(|bytes| VerifyingKey::from_bytes(&bytes).ok())
            .ok_or(QueueError::Signature)?;
        let signature = parse_hex::<64>(&self.signature)
            .map(|bytes| Signature::from_bytes(&bytes))
            .ok_or(QueueError::Signature)?;
        moderator
            .verify_strict(&closure_message(&self.closure), &signature)
            .map_err(|_| QueueError::Signature)?;
        Ok(closure)
    }
}

fn closure_message(closure: &str) -> Vec<u8> {
    let mut message = CLOSURE_DOMAIN.to_vec();
    message.extend_from_slice(blake3::hash(closure.as_bytes()).as_bytes());
    message
}

fn check_closure(closure: &Closure) -> Result<(), QueueError> {
    if parse_hex::<32>(&closure.report_id).is_none() {
        return Err(QueueError::Malformed(
            "the report id is not 32 bytes of lower-case hex".to_owned(),
        ));
    }
    if closure.note.len() > MAX_CLOSURE_NOTE_BYTES {
        return Err(QueueError::Malformed(format!(
            "the note is {} bytes; the limit is {MAX_CLOSURE_NOTE_BYTES}",
            closure.note.len()
        )));
    }
    if closure
        .note
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(QueueError::Malformed(
            "the note may not contain control characters other than newline and tab".to_owned(),
        ));
    }
    Ok(())
}

/// Who may close reports here. The binding to the world role model is
/// the caller's: this module only asks.
pub trait ModeratorAuthority {
    /// Whether the key (lower-case hex) is one of this operator's
    /// moderators.
    fn is_moderator(&self, key: &str) -> bool;
}

/// A fixed set of moderator keys, as the operator configures them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Moderators {
    keys: BTreeSet<String>,
}

impl Moderators {
    /// The moderators with these public keys.
    #[must_use]
    pub fn new(keys: impl IntoIterator<Item = VerifyingKey>) -> Self {
        Self {
            keys: keys
                .into_iter()
                .map(|key| encode_hex(&key.to_bytes()))
                .collect(),
        }
    }
}

impl ModeratorAuthority for Moderators {
    fn is_moderator(&self, key: &str) -> bool {
        self.keys.contains(key)
    }
}

/// One logged closure: the moderator's signed decision unchanged, plus
/// the operator's sequence and receipt time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClosureFact {
    /// Position in this operator's closure log, from 1.
    pub sequence: u64,
    /// When this node logged it, Unix milliseconds.
    pub logged_at_ms: u64,
    /// The decision as signed.
    pub signed: SignedClosure,
}

/// The operator's append-only log of closures.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClosureLog {
    facts: Vec<ClosureFact>,
    by_report: BTreeMap<String, usize>,
}

impl ClosureLog {
    /// An empty log in memory.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Load the log under `operator_dir`; a missing file is an empty
    /// log. Every line's signature is re-verified, and no report may be
    /// closed twice. Whether each signer was a moderator is checked at
    /// [`ClosureLog::close`] time and is not re-judged on load, so
    /// removing a moderator later does not rewrite history.
    ///
    /// # Errors
    ///
    /// The file cannot be read, a line is malformed or does not verify,
    /// a report is closed twice, or the sequence is not 1, 2, 3 ...
    pub fn load(operator_dir: &Path) -> Result<Self, QueueError> {
        let path = operator_dir.join(CLOSURES_FILE);
        let mut log = Self::default();
        if !path.is_file() {
            return Ok(log);
        }
        let text = std::fs::read_to_string(&path).map_err(|e| storage(&path, &e))?;
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            let fact: ClosureFact =
                id_core::json_unique::from_slice_without_duplicate_keys(line.as_bytes())
                    .map_err(|e| QueueError::Malformed(format!("{}: {e}", path.display())))?;
            if fact.sequence != log.next_sequence() {
                return Err(QueueError::Malformed(format!(
                    "closure log sequence {} where {} was expected",
                    fact.sequence,
                    log.next_sequence()
                )));
            }
            let closure = fact.signed.verify()?;
            if log.by_report.contains_key(&closure.report_id) {
                return Err(QueueError::AlreadyClosed(closure.report_id));
            }
            log.push(closure.report_id, fact);
        }
        Ok(log)
    }

    /// Every closure, in sequence order.
    #[must_use]
    pub fn facts(&self) -> &[ClosureFact] {
        &self.facts
    }

    /// The closure of the report with this id, if it is closed.
    #[must_use]
    pub fn closure_of(&self, report_id: &str) -> Option<&ClosureFact> {
        self.by_report
            .get(report_id)
            .and_then(|&index| self.facts.get(index))
    }

    /// How many closures the log holds.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.facts.len()
    }

    /// Whether the log is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.facts.is_empty()
    }

    /// Close a report: check the moderator's signed decision against
    /// `authority` and `reports`, then append it, writing the line under
    /// `operator_dir` when given.
    ///
    /// # Errors
    ///
    /// The closure does not verify, its signer is not a moderator, the
    /// report is unknown or already closed, or the line cannot be
    /// written (the log in memory is then unchanged).
    pub fn close(
        &mut self,
        signed: SignedClosure,
        authority: &impl ModeratorAuthority,
        reports: &ReportLog,
        now_ms: u64,
        operator_dir: Option<&Path>,
    ) -> Result<ClosureFact, QueueError> {
        let closure = signed.verify()?;
        if !authority.is_moderator(&closure.moderator) {
            return Err(QueueError::NotModerator(closure.moderator));
        }
        if reports.get(&closure.report_id).is_none() {
            return Err(QueueError::UnknownReport(closure.report_id));
        }
        if self.by_report.contains_key(&closure.report_id) {
            return Err(QueueError::AlreadyClosed(closure.report_id));
        }
        let fact = ClosureFact {
            sequence: self.next_sequence(),
            logged_at_ms: now_ms,
            signed,
        };
        if let Some(dir) = operator_dir {
            append_line(&dir.join(CLOSURES_FILE), &fact)?;
        }
        self.push(closure.report_id, fact.clone());
        Ok(fact)
    }

    fn next_sequence(&self) -> u64 {
        self.facts
            .last()
            .map_or(1, |fact| fact.sequence.saturating_add(1))
    }

    fn push(&mut self, report_id: String, fact: ClosureFact) {
        self.by_report.insert(report_id, self.facts.len());
        self.facts.push(fact);
    }
}

/// The operator's ordering policy for open reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueuePolicy {
    /// Reason classes, most urgent first. A class the list leaves out
    /// sorts after every listed class.
    pub reason_order: Vec<ReportReason>,
}

impl Default for QueuePolicy {
    /// Severity first: abuse, impersonation, spam, other.
    fn default() -> Self {
        Self {
            reason_order: vec![
                ReportReason::Abuse,
                ReportReason::Impersonation,
                ReportReason::Spam,
                ReportReason::Other,
            ],
        }
    }
}

impl QueuePolicy {
    fn rank(&self, reason: ReportReason) -> usize {
        self.reason_order
            .iter()
            .position(|listed| *listed == reason)
            .unwrap_or(self.reason_order.len())
    }
}

/// One report as the queue shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueItem {
    /// The accepted report fact.
    pub fact: ReportFact,
    /// The verified report it carries.
    pub report: Report,
    /// Its closure, when closed.
    pub closure: Option<Closure>,
}

/// The review queue: open reports in policy order, closed reports in
/// closure order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReviewQueue {
    open: Vec<QueueItem>,
    closed: Vec<QueueItem>,
}

impl ReviewQueue {
    /// Rebuild the queue from the operator's logs. Pure: the same logs
    /// and policy always yield the same queue.
    ///
    /// # Errors
    ///
    /// A report or closure in the logs no longer verifies.
    pub fn rebuild(
        reports: &ReportLog,
        closures: &ClosureLog,
        policy: &QueuePolicy,
    ) -> Result<Self, QueueError> {
        let mut open = Vec::new();
        let mut closed = Vec::new();
        for fact in reports.facts() {
            let report = fact
                .report()
                .map_err(|e| QueueError::Report(e.to_string()))?;
            match closures.closure_of(&fact.id) {
                Some(closure) => {
                    let decision = closure.signed.verify()?;
                    closed.push((
                        closure.sequence,
                        QueueItem {
                            fact: fact.clone(),
                            report,
                            closure: Some(decision),
                        },
                    ));
                }
                None => open.push(QueueItem {
                    fact: fact.clone(),
                    report,
                    closure: None,
                }),
            }
        }
        open.sort_by(|a, b| {
            policy
                .rank(a.report.reason)
                .cmp(&policy.rank(b.report.reason))
                .then(a.report.filed_at_ms.cmp(&b.report.filed_at_ms))
                .then(a.fact.sequence.cmp(&b.fact.sequence))
        });
        closed.sort_by_key(|(sequence, _)| *sequence);
        Ok(Self {
            open,
            closed: closed.into_iter().map(|(_, item)| item).collect(),
        })
    }

    /// Open reports, most urgent first.
    #[must_use]
    pub fn open(&self) -> &[QueueItem] {
        &self.open
    }

    /// Closed reports, in the order they were closed.
    #[must_use]
    pub fn closed(&self) -> &[QueueItem] {
        &self.closed
    }
}

fn append_line(path: &Path, fact: &ClosureFact) -> Result<(), QueueError> {
    let dir = path
        .parent()
        .ok_or_else(|| QueueError::Storage(format!("{} has no parent", path.display())))?;
    std::fs::create_dir_all(dir).map_err(|e| storage(dir, &e))?;
    let mut line = serde_json::to_vec(fact).map_err(|e| QueueError::Storage(e.to_string()))?;
    line.push(b'\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| storage(path, &e))?;
    std::io::Write::write_all(&mut file, &line).map_err(|e| storage(path, &e))?;
    file.sync_all().map_err(|e| storage(path, &e))
}

fn storage(path: &Path, error: &std::io::Error) -> QueueError {
    QueueError::Storage(format!("{}: {error}", path.display()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use ed25519_dalek::SigningKey;

    use super::*;
    use crate::safety_report::{HostedScope, ReportTarget, SignedReport};
    use crate::world::Records;

    const NOW: u64 = 1_795_000_000_000;

    fn key(byte: u8) -> SigningKey {
        SigningKey::from_bytes(&[byte; 32])
    }

    fn moderator() -> SigningKey {
        key(40)
    }

    fn moderators() -> Moderators {
        Moderators::new([moderator().verifying_key()])
    }

    fn hosted() -> HostedScope {
        let mut scope = HostedScope::new();
        scope.host("quiz-hall", &Records::new()).unwrap();
        scope.host("chat-den", &Records::new()).unwrap();
        scope
    }

    fn file(
        log: &mut ReportLog,
        reporter: u8,
        world: &str,
        reason: ReportReason,
        filed_at_ms: u64,
    ) -> ReportFact {
        let signed = SignedReport::sign(
            ReportTarget::HostedWorld {
                world_id: world.to_owned(),
            },
            reason,
            "",
            filed_at_ms,
            &key(reporter),
        )
        .unwrap();
        log.file(signed, &hosted(), NOW, None).unwrap()
    }

    fn reasons(items: &[QueueItem]) -> Vec<(ReportReason, u64)> {
        items
            .iter()
            .map(|item| (item.report.reason, item.report.filed_at_ms))
            .collect()
    }

    #[test]
    fn open_reports_order_by_policy_then_age() {
        let mut log = ReportLog::new();
        file(&mut log, 1, "quiz-hall", ReportReason::Spam, NOW - 10);
        file(&mut log, 2, "quiz-hall", ReportReason::Abuse, NOW - 5);
        file(&mut log, 3, "chat-den", ReportReason::Other, NOW - 50);
        file(&mut log, 4, "chat-den", ReportReason::Abuse, NOW - 20);

        let queue =
            ReviewQueue::rebuild(&log, &ClosureLog::new(), &QueuePolicy::default()).unwrap();
        assert_eq!(
            reasons(queue.open()),
            vec![
                (ReportReason::Abuse, NOW - 20),
                (ReportReason::Abuse, NOW - 5),
                (ReportReason::Spam, NOW - 10),
                (ReportReason::Other, NOW - 50),
            ]
        );
        assert!(queue.closed().is_empty());

        // The operator's own policy reorders the same facts.
        let spam_first = QueuePolicy {
            reason_order: vec![ReportReason::Spam],
        };
        let queue = ReviewQueue::rebuild(&log, &ClosureLog::new(), &spam_first).unwrap();
        assert_eq!(
            reasons(queue.open()),
            vec![
                (ReportReason::Spam, NOW - 10),
                (ReportReason::Other, NOW - 50),
                (ReportReason::Abuse, NOW - 20),
                (ReportReason::Abuse, NOW - 5),
            ]
        );
    }

    #[test]
    fn only_a_moderator_decision_closes_a_report_and_it_is_logged() {
        let mut log = ReportLog::new();
        let spam = file(&mut log, 1, "quiz-hall", ReportReason::Spam, NOW - 10);
        let abuse = file(&mut log, 2, "quiz-hall", ReportReason::Abuse, NOW - 5);
        let mut closures = ClosureLog::new();

        // A key that is not a moderator cannot close it.
        let stranger =
            SignedClosure::sign(&spam.id, ClosureDecision::NoAction, "", NOW, &key(9)).unwrap();
        assert!(matches!(
            closures.close(stranger, &moderators(), &log, NOW, None),
            Err(QueueError::NotModerator(_))
        ));
        // Nor can a forged signature.
        let mut forged =
            SignedClosure::sign(&spam.id, ClosureDecision::NoAction, "", NOW, &moderator())
                .unwrap();
        forged.signature = "00".repeat(64);
        assert_eq!(
            closures.close(forged, &moderators(), &log, NOW, None),
            Err(QueueError::Signature)
        );
        assert!(closures.is_empty());

        let decision = SignedClosure::sign(
            &spam.id,
            ClosureDecision::NoAction,
            "not spam",
            NOW,
            &moderator(),
        )
        .unwrap();
        let logged = closures
            .close(decision, &moderators(), &log, NOW + 1, None)
            .unwrap();
        assert_eq!(logged.sequence, 1);

        let queue = ReviewQueue::rebuild(&log, &closures, &QueuePolicy::default()).unwrap();
        assert_eq!(queue.open().len(), 1);
        assert_eq!(queue.open()[0].fact.id, abuse.id);
        assert_eq!(queue.closed().len(), 1);
        let closure = queue.closed()[0].closure.as_ref().unwrap();
        assert_eq!(closure.decision, ClosureDecision::NoAction);
        assert_eq!(
            closure.moderator,
            encode_hex(&moderator().verifying_key().to_bytes())
        );
    }

    #[test]
    fn a_report_closes_once_and_unknown_reports_cannot_close() {
        let mut log = ReportLog::new();
        let spam = file(&mut log, 1, "quiz-hall", ReportReason::Spam, NOW - 10);
        let mut closures = ClosureLog::new();
        let first = SignedClosure::sign(&spam.id, ClosureDecision::Actioned, "", NOW, &moderator())
            .unwrap();
        closures
            .close(first, &moderators(), &log, NOW, None)
            .unwrap();
        let again = SignedClosure::sign(
            &spam.id,
            ClosureDecision::NoAction,
            "",
            NOW + 1,
            &moderator(),
        )
        .unwrap();
        assert!(matches!(
            closures.close(again, &moderators(), &log, NOW, None),
            Err(QueueError::AlreadyClosed(_))
        ));
        let unknown = SignedClosure::sign(
            &"ab".repeat(32),
            ClosureDecision::NoAction,
            "",
            NOW,
            &moderator(),
        )
        .unwrap();
        assert!(matches!(
            closures.close(unknown, &moderators(), &log, NOW, None),
            Err(QueueError::UnknownReport(_))
        ));
        assert_eq!(closures.len(), 1);
    }

    #[test]
    fn the_queue_rebuilds_identically_from_the_logs_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = ReportLog::new();
        for (reporter, reason, at) in [
            (1, ReportReason::Spam, NOW - 30),
            (2, ReportReason::Abuse, NOW - 20),
            (3, ReportReason::Impersonation, NOW - 10),
        ] {
            let signed = SignedReport::sign(
                ReportTarget::HostedWorld {
                    world_id: "quiz-hall".to_owned(),
                },
                reason,
                "",
                at,
                &key(reporter),
            )
            .unwrap();
            log.file(signed, &hosted(), NOW, Some(dir.path())).unwrap();
        }
        let mut closures = ClosureLog::new();
        let target = log.facts()[1].id.clone();
        closures
            .close(
                SignedClosure::sign(&target, ClosureDecision::Actioned, "", NOW, &moderator())
                    .unwrap(),
                &moderators(),
                &log,
                NOW,
                Some(dir.path()),
            )
            .unwrap();

        let policy = QueuePolicy::default();
        let live = ReviewQueue::rebuild(&log, &closures, &policy).unwrap();
        let reports = ReportLog::load(dir.path()).unwrap();
        let reloaded_closures = ClosureLog::load(dir.path()).unwrap();
        assert_eq!(reloaded_closures, closures);
        let rebuilt = ReviewQueue::rebuild(&reports, &reloaded_closures, &policy).unwrap();
        assert_eq!(rebuilt, live);
        assert_eq!(
            reasons(rebuilt.open()),
            vec![
                (ReportReason::Impersonation, NOW - 10),
                (ReportReason::Spam, NOW - 30),
            ]
        );
    }

    #[test]
    fn a_tampered_closure_log_fails_to_load() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = ReportLog::new();
        let spam = file(&mut log, 1, "quiz-hall", ReportReason::Spam, NOW - 10);
        let mut closures = ClosureLog::new();
        closures
            .close(
                SignedClosure::sign(&spam.id, ClosureDecision::NoAction, "", NOW, &moderator())
                    .unwrap(),
                &moderators(),
                &log,
                NOW,
                Some(dir.path()),
            )
            .unwrap();
        let path = dir.path().join(CLOSURES_FILE);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replace("no_action", "actioned")).unwrap();
        assert!(ClosureLog::load(dir.path()).is_err());
    }
}
