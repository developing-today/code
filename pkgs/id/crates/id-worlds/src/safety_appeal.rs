//! Appeals against logged moderation actions (#14467).
//!
//! Design: creator public-world safety v3 outline,
//! `doc/2026-11-24T16-12-00Z_design_id6310_creator_public_world_safety_v3_outline.md`,
//! sections 1, 2, 4 question 1 and 6 item 4.
//!
//! An affected party contests one action in the operator's
//! transparency log ([`crate::safety_transparency`], #14466). The appeal
//! is a signed, timestamped fact naming the action by its
//! [`action_hash`](crate::safety_transparency::TransparencyEntry::action_hash),
//! the content hash the outline recommends for open question 1, so the
//! link does not depend on log positions.
//!
//! - **Filing.** [`AppealLog::file`] checks the signature, the time,
//!   that the action is in the operator's view, and that the caller's
//!   [`AppealStanding`] says the appellant is affected by it. One
//!   appellant has at most one open appeal per action.
//! - **The same queue.** An accepted appeal is its own item in the
//!   review queue
//!   ([`ReviewQueue::rebuild_with_appeals`](crate::safety_review_queue::ReviewQueue::rebuild_with_appeals)).
//! - **The same decision path.** A moderator decides it with the same
//!   [`SignedClosure`](crate::safety_review_queue::SignedClosure)
//!   through the same
//!   [`ClosureLog::close`](crate::safety_review_queue::ClosureLog::close),
//!   so the decision is appended to the same closure log and shows in
//!   the transparency view as an appeal decision. `Actioned` upholds the
//!   appeal, `NoAction` denies it.
//! - **Never automatic.** Neither filing nor deciding an appeal reverses
//!   anything. A reversal is a separate moderator action through the
//!   world role model, logged in its own right.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::program_bundle::parse_hex;
use crate::safety_report::MAX_FUTURE_SKEW_MS;
use crate::safety_review_queue::{Closure, QueueItems};
use crate::safety_transparency::{TransparencyEntry, TransparencyView};
use crate::world_session::encode_hex;

/// Domain separator for an appeal signature.
pub const APPEAL_DOMAIN: &[u8] = b"developing-today/id-safety-appeal/v1\0";

/// File name of the appeal log under the operator directory.
pub const APPEALS_FILE: &str = "appeals.jsonl";

/// Largest free-text detail on an appeal, in bytes.
pub const MAX_APPEAL_DETAIL_BYTES: usize = 2000;

/// Largest signed appeal document accepted, in bytes.
pub const MAX_APPEAL_BYTES: usize = 8 * 1024;

/// One appeal against one logged action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Appeal {
    /// The appellant's public key, lower-case hex.
    pub appellant: String,
    /// The contested action's hash in the transparency view.
    pub action_hash: String,
    /// Why, in the appellant's words. Never an input to any automatic
    /// decision, and never shown in the transparency view.
    pub detail: String,
    /// When the appellant filed it, Unix milliseconds.
    pub filed_at_ms: u64,
}

/// An appeal document with the appellant's signature over it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedAppeal {
    /// The appeal as the exact JSON that was signed.
    pub appeal: String,
    /// Ed25519 signature, lower-case hex.
    pub signature: String,
}

/// Why an appeal was refused or the log could not be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AppealError {
    /// The document is malformed or out of bounds.
    #[error("appeal: {0}")]
    Malformed(String),
    /// The appeal is not signed by the appellant it names.
    #[error("appeal signature: not signed by the appellant it names")]
    Signature,
    /// The appeal was filed later than this node's clock allows.
    #[error("appeal filed at {filed_at_ms}, after this node's time {now_ms}")]
    FromTheFuture {
        /// The appeal's filing time.
        filed_at_ms: u64,
        /// This node's time at intake.
        now_ms: u64,
    },
    /// No action with this hash is in the operator's view.
    #[error("no logged action {0} in this operator's view")]
    UnknownAction(String),
    /// The appellant is not affected by the action.
    #[error("{0} is not affected by this action")]
    NoStanding(String),
    /// The appellant already has an open appeal on this action.
    #[error("an appeal on {0} by this appellant is already open")]
    AlreadyOpen(String),
    /// Reading or writing the appeal log failed.
    #[error("appeal storage: {0}")]
    Storage(String),
}

impl SignedAppeal {
    /// Sign an appeal against the action with `action_hash`.
    ///
    /// # Errors
    ///
    /// The hash or detail is out of bounds.
    pub fn sign(
        action_hash: &str,
        detail: &str,
        filed_at_ms: u64,
        appellant: &SigningKey,
    ) -> Result<Self, AppealError> {
        let appeal = Appeal {
            appellant: encode_hex(&appellant.verifying_key().to_bytes()),
            action_hash: action_hash.to_owned(),
            detail: detail.to_owned(),
            filed_at_ms,
        };
        check_appeal(&appeal)?;
        let appeal =
            serde_json::to_string(&appeal).map_err(|e| AppealError::Malformed(e.to_string()))?;
        let signature = encode_hex(&appellant.sign(&appeal_message(&appeal)).to_bytes());
        Ok(Self { appeal, signature })
    }

    /// The appeal, once its bounds and signature verify.
    ///
    /// # Errors
    ///
    /// The document is malformed, out of bounds, or not signed by the
    /// appellant it names.
    pub fn verify(&self) -> Result<Appeal, AppealError> {
        if self.appeal.len() > MAX_APPEAL_BYTES {
            return Err(AppealError::Malformed(format!(
                "the document is {} bytes; the limit is {MAX_APPEAL_BYTES}",
                self.appeal.len()
            )));
        }
        let appeal: Appeal =
            id_core::json_unique::from_slice_without_duplicate_keys(self.appeal.as_bytes())
                .map_err(|e| AppealError::Malformed(e.to_string()))?;
        check_appeal(&appeal)?;
        let appellant = parse_hex::<32>(&appeal.appellant)
            .and_then(|bytes| VerifyingKey::from_bytes(&bytes).ok())
            .ok_or(AppealError::Signature)?;
        let signature = parse_hex::<64>(&self.signature)
            .map(|bytes| Signature::from_bytes(&bytes))
            .ok_or(AppealError::Signature)?;
        appellant
            .verify_strict(&appeal_message(&self.appeal), &signature)
            .map_err(|_signature_error| AppealError::Signature)?;
        Ok(appeal)
    }

    /// The appeal's content address: BLAKE3 of the signed document,
    /// lower-case hex. This is the id a closure names.
    #[must_use]
    pub fn id(&self) -> String {
        encode_hex(blake3::hash(self.appeal.as_bytes()).as_bytes())
    }
}

fn appeal_message(appeal: &str) -> Vec<u8> {
    let mut message = APPEAL_DOMAIN.to_vec();
    message.extend_from_slice(blake3::hash(appeal.as_bytes()).as_bytes());
    message
}

fn check_appeal(appeal: &Appeal) -> Result<(), AppealError> {
    if parse_hex::<32>(&appeal.action_hash).is_none() {
        return Err(AppealError::Malformed(
            "the action hash is not 32 bytes of lower-case hex".to_owned(),
        ));
    }
    if appeal.detail.len() > MAX_APPEAL_DETAIL_BYTES {
        return Err(AppealError::Malformed(format!(
            "the detail is {} bytes; the limit is {MAX_APPEAL_DETAIL_BYTES}",
            appeal.detail.len()
        )));
    }
    if appeal
        .detail
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(AppealError::Malformed(
            "the detail may not contain control characters other than newline and tab".to_owned(),
        ));
    }
    Ok(())
}

/// Who may appeal which action. The binding to identities (which key
/// is the revoked participant, or the reported world's owner) is the
/// caller's; this module only asks.
pub trait AppealStanding {
    /// Whether the appellant (public key, lower-case hex) is affected by
    /// this logged action.
    fn may_appeal(&self, appellant: &str, entry: &TransparencyEntry) -> bool;
}

/// One accepted appeal: the signed document unchanged, plus the
/// operator's sequence, id and receipt time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppealFact {
    /// Position in this operator's appeal log, from 1.
    pub sequence: u64,
    /// The appeal's id ([`SignedAppeal::id`]).
    pub id: String,
    /// When this node accepted it, Unix milliseconds.
    pub received_at_ms: u64,
    /// The appeal as signed.
    pub signed: SignedAppeal,
}

impl AppealFact {
    /// The appeal, verified again.
    ///
    /// # Errors
    ///
    /// The fact's id does not match its document, or the document no
    /// longer verifies.
    pub fn appeal(&self) -> Result<Appeal, AppealError> {
        if self.id != self.signed.id() {
            return Err(AppealError::Malformed(format!(
                "fact {} carries an appeal whose id is {}",
                self.id,
                self.signed.id()
            )));
        }
        self.signed.verify()
    }
}

/// An appeal as a review-queue item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppealItem {
    /// The logged appeal.
    pub fact: AppealFact,
    /// Its verified content.
    pub appeal: Appeal,
    /// The moderator's decision, once there is one.
    pub closure: Option<Closure>,
}

/// The operator's append-only log of accepted appeals.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppealLog {
    facts: Vec<AppealFact>,
    by_id: BTreeMap<String, usize>,
}

impl AppealLog {
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
    /// The file cannot be read, a line is malformed or does not verify,
    /// or the sequence is not 1, 2, 3 ...
    pub fn load(operator_dir: &Path) -> Result<Self, AppealError> {
        let path = operator_dir.join(APPEALS_FILE);
        let mut log = Self::default();
        if !path.is_file() {
            return Ok(log);
        }
        let text = std::fs::read_to_string(&path).map_err(|e| storage(&path, &e))?;
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            let fact: AppealFact =
                id_core::json_unique::from_slice_without_duplicate_keys(line.as_bytes())
                    .map_err(|e| AppealError::Malformed(format!("{}: {e}", path.display())))?;
            if fact.sequence != log.next_sequence() {
                return Err(AppealError::Malformed(format!(
                    "appeal log sequence {} where {} was expected",
                    fact.sequence,
                    log.next_sequence()
                )));
            }
            fact.appeal()?;
            if log.by_id.contains_key(&fact.id) {
                return Err(AppealError::Malformed(format!(
                    "appeal {} is logged twice",
                    fact.id
                )));
            }
            log.push(fact);
        }
        Ok(log)
    }

    /// Every appeal, in sequence order.
    #[must_use]
    pub fn facts(&self) -> &[AppealFact] {
        &self.facts
    }

    /// The appeal with this id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&AppealFact> {
        self.by_id.get(id).and_then(|&index| self.facts.get(index))
    }

    /// How many appeals the log holds.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.facts.len()
    }

    /// Whether the log is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.facts.is_empty()
    }

    /// File an appeal: check it against the operator's `view` and the
    /// caller's `standing`, then append it, writing the line under
    /// `operator_dir` when given. Filing the same signed appeal twice
    /// returns the fact already logged. `open` lists the ids of appeals
    /// already decided (pass the closure log), so one appellant has at
    /// most one open appeal per action.
    ///
    /// # Errors
    ///
    /// Any [`AppealError`]; a refused appeal leaves the log unchanged.
    pub fn file(
        &mut self,
        signed: SignedAppeal,
        view: &TransparencyView,
        standing: &impl AppealStanding,
        decided: &impl QueueDecided,
        now_ms: u64,
        operator_dir: Option<&Path>,
    ) -> Result<AppealFact, AppealError> {
        let appeal = signed.verify()?;
        if appeal.filed_at_ms > now_ms.saturating_add(MAX_FUTURE_SKEW_MS) {
            return Err(AppealError::FromTheFuture {
                filed_at_ms: appeal.filed_at_ms,
                now_ms,
            });
        }
        let id = signed.id();
        if let Some(existing) = self.get(&id) {
            return Ok(existing.clone());
        }
        let entry = view
            .find(&appeal.action_hash)
            .ok_or_else(|| AppealError::UnknownAction(appeal.action_hash.clone()))?;
        if !standing.may_appeal(&appeal.appellant, entry) {
            return Err(AppealError::NoStanding(appeal.appellant));
        }
        if self
            .open_by(&appeal.appellant, decided)
            .contains(&appeal.action_hash)
        {
            return Err(AppealError::AlreadyOpen(appeal.action_hash));
        }
        let fact = AppealFact {
            sequence: self.next_sequence(),
            id,
            received_at_ms: now_ms,
            signed,
        };
        if let Some(dir) = operator_dir {
            append_line(&dir.join(APPEALS_FILE), &fact)?;
        }
        self.push(fact.clone());
        Ok(fact)
    }

    /// The action hashes this appellant has an undecided appeal on.
    fn open_by(&self, appellant: &str, decided: &impl QueueDecided) -> BTreeSet<String> {
        self.facts
            .iter()
            .filter(|fact| !decided.decided(&fact.id))
            .filter_map(|fact| fact.signed.verify().ok())
            .filter(|appeal| appeal.appellant == appellant)
            .map(|appeal| appeal.action_hash)
            .collect()
    }

    fn next_sequence(&self) -> u64 {
        self.facts
            .last()
            .map_or(1, |fact| fact.sequence.saturating_add(1))
    }

    fn push(&mut self, fact: AppealFact) {
        self.by_id.insert(fact.id.clone(), self.facts.len());
        self.facts.push(fact);
    }
}

impl QueueItems for AppealLog {
    fn holds(&self, id: &str) -> bool {
        self.get(id).is_some()
    }
}

/// Which queue items have a moderator's decision.
pub trait QueueDecided {
    /// Whether the item with this id has been decided.
    fn decided(&self, id: &str) -> bool;
}

impl QueueDecided for crate::safety_review_queue::ClosureLog {
    fn decided(&self, id: &str) -> bool {
        self.closure_of(id).is_some()
    }
}

fn append_line(path: &Path, fact: &AppealFact) -> Result<(), AppealError> {
    let dir = path
        .parent()
        .ok_or_else(|| AppealError::Storage(format!("{} has no parent", path.display())))?;
    std::fs::create_dir_all(dir).map_err(|e| storage(dir, &e))?;
    let mut line = serde_json::to_vec(fact).map_err(|e| AppealError::Storage(e.to_string()))?;
    line.push(b'\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| storage(path, &e))?;
    std::io::Write::write_all(&mut file, &line).map_err(|e| storage(path, &e))?;
    file.sync_all().map_err(|e| storage(path, &e))
}

fn storage(path: &Path, error: &std::io::Error) -> AppealError {
    AppealError::Storage(format!("{}: {error}", path.display()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use ed25519_dalek::SigningKey;

    use super::*;
    use crate::safety_report::ReportLog;
    use crate::safety_review_queue::{
        ClosureDecision, ClosureLog, Moderators, QueueError, QueuePolicy, ReviewQueue,
        SignedClosure,
    };
    use crate::safety_transparency::{
        ReadAuthority, TransparencyAction, ViewLimits, ViewRequest, replay_with_appeals,
    };
    use crate::world::{JournalEntry, MAX_JOURNAL_VERSION};

    const NOW: u64 = 1_795_000_000_000;
    const WORLD: &str = "quiz-hall";

    fn key(byte: u8) -> SigningKey {
        SigningKey::from_bytes(&[byte; 32])
    }

    fn hex_of(key: &SigningKey) -> String {
        encode_hex(&key.verifying_key().to_bytes())
    }

    struct Owner;

    impl ReadAuthority for Owner {
        fn may_read(&self, reader: &str) -> bool {
            reader == "owner"
        }
    }

    /// Participant 7's key is `key(7)`: only they may appeal their own
    /// revocation.
    struct Revokee;

    impl AppealStanding for Revokee {
        fn may_appeal(&self, appellant: &str, entry: &TransparencyEntry) -> bool {
            match &entry.action {
                TransparencyAction::ParticipantRevoked { participant_id, .. } => {
                    u8::try_from(*participant_id).is_ok_and(|id| hex_of(&key(id)) == appellant)
                }
                _ => false,
            }
        }
    }

    fn journal() -> Vec<u8> {
        let mut out = Vec::new();
        for entry in [
            JournalEntry::Created {
                world_id: WORLD.to_owned(),
                version: MAX_JOURNAL_VERSION,
            },
            JournalEntry::Revoked {
                participant_id: 7,
                by: Some(3),
                revoked_at: Some(NOW - 1000),
            },
        ] {
            out.extend(serde_json::to_vec(&entry).unwrap());
            out.push(b'\n');
        }
        out
    }

    fn view(closures: &ClosureLog, appeals: &AppealLog) -> TransparencyView {
        let journal = journal();
        replay_with_appeals(
            journal.as_slice(),
            closures,
            appeals,
            &ViewRequest {
                world_id: WORLD,
                reader: "owner",
                limits: ViewLimits::default(),
            },
            &Owner,
        )
        .unwrap()
    }

    fn revocation_hash(view: &TransparencyView) -> String {
        view.world_actions.entries[0].action_hash().to_owned()
    }

    #[test]
    fn an_appeal_names_a_logged_action_and_enters_the_queue_as_its_own_item() {
        let closures = ClosureLog::new();
        let mut appeals = AppealLog::new();
        let before = view(&closures, &appeals);
        let contested = revocation_hash(&before);

        let signed = SignedAppeal::sign(&contested, "I was not spamming", NOW, &key(7)).unwrap();
        let fact = appeals
            .file(signed.clone(), &before, &Revokee, &closures, NOW, None)
            .unwrap();
        assert_eq!(fact.sequence, 1);
        assert_eq!(fact.id, signed.id());
        assert_eq!(fact.appeal().unwrap().action_hash, contested);

        let queue = ReviewQueue::rebuild_with_appeals(
            &ReportLog::new(),
            &appeals,
            &closures,
            &QueuePolicy::default(),
        )
        .unwrap();
        assert_eq!(queue.open(), []);
        assert_eq!(queue.open_appeals().len(), 1);
        assert_eq!(queue.open_appeals()[0].fact.id, fact.id);
        assert_eq!(queue.closed_appeals(), []);

        // Filing the same document again returns the logged fact.
        let again = appeals
            .file(signed, &before, &Revokee, &closures, NOW, None)
            .unwrap();
        assert_eq!(again, fact);
        assert_eq!(appeals.len(), 1);
    }

    #[test]
    fn appeals_are_refused_without_a_logged_action_standing_or_a_good_signature() {
        let closures = ClosureLog::new();
        let mut appeals = AppealLog::new();
        let current = view(&closures, &appeals);
        let contested = revocation_hash(&current);

        let unknown = SignedAppeal::sign(&"ab".repeat(32), "", NOW, &key(7)).unwrap();
        assert!(matches!(
            appeals.file(unknown, &current, &Revokee, &closures, NOW, None),
            Err(AppealError::UnknownAction(_))
        ));

        let stranger = SignedAppeal::sign(&contested, "", NOW, &key(8)).unwrap();
        assert!(matches!(
            appeals.file(stranger, &current, &Revokee, &closures, NOW, None),
            Err(AppealError::NoStanding(_))
        ));

        let future =
            SignedAppeal::sign(&contested, "", NOW + MAX_FUTURE_SKEW_MS + 1, &key(7)).unwrap();
        assert!(matches!(
            appeals.file(future, &current, &Revokee, &closures, NOW, None),
            Err(AppealError::FromTheFuture { .. })
        ));

        let mut forged = SignedAppeal::sign(&contested, "", NOW, &key(7)).unwrap();
        forged.appeal = forged.appeal.replace("\"detail\":\"\"", "\"detail\":\"x\"");
        assert_eq!(forged.verify(), Err(AppealError::Signature));
        assert!(
            appeals
                .file(forged, &current, &Revokee, &closures, NOW, None)
                .is_err()
        );

        assert!(matches!(
            SignedAppeal::sign("not-a-hash", "", NOW, &key(7)),
            Err(AppealError::Malformed(_))
        ));
        assert!(appeals.is_empty());
    }

    #[test]
    fn one_open_appeal_per_action_per_appellant() {
        let mut closures = ClosureLog::new();
        let mut appeals = AppealLog::new();
        let current = view(&closures, &appeals);
        let contested = revocation_hash(&current);

        let first = SignedAppeal::sign(&contested, "first", NOW, &key(7)).unwrap();
        let first = appeals
            .file(first, &current, &Revokee, &closures, NOW, None)
            .unwrap();
        let second = SignedAppeal::sign(&contested, "second", NOW + 1, &key(7)).unwrap();
        assert!(matches!(
            appeals.file(second.clone(), &current, &Revokee, &closures, NOW, None),
            Err(AppealError::AlreadyOpen(_))
        ));

        // Once the first is decided, a new appeal may be filed.
        let moderator = key(40);
        let decision =
            SignedClosure::sign(&first.id, ClosureDecision::NoAction, "", NOW, &moderator).unwrap();
        closures
            .close(
                decision,
                &Moderators::new([moderator.verifying_key()]),
                &appeals,
                NOW,
                None,
            )
            .unwrap();
        assert!(
            appeals
                .file(second, &current, &Revokee, &closures, NOW + 2, None)
                .is_ok()
        );
    }

    #[test]
    fn a_moderator_decides_an_appeal_through_the_same_closure_path_and_log() {
        let mut closures = ClosureLog::new();
        let mut appeals = AppealLog::new();
        let reports = ReportLog::new();
        let before = view(&closures, &appeals);
        let contested = revocation_hash(&before);
        let appeal = SignedAppeal::sign(&contested, "please review", NOW, &key(7)).unwrap();
        let appeal = appeals
            .file(appeal, &before, &Revokee, &closures, NOW, None)
            .unwrap();

        let moderator = key(40);
        let moderators = Moderators::new([moderator.verifying_key()]);

        // Only a moderator's decision closes it.
        let stranger =
            SignedClosure::sign(&appeal.id, ClosureDecision::Actioned, "", NOW, &key(9)).unwrap();
        assert!(matches!(
            closures.close(stranger, &moderators, &(&reports, &appeals), NOW, None),
            Err(QueueError::NotModerator(_))
        ));
        let upheld = SignedClosure::sign(
            &appeal.id,
            ClosureDecision::Actioned,
            "private moderator note",
            NOW + 5,
            &moderator,
        )
        .unwrap();
        let logged = closures
            .close(
                upheld.clone(),
                &moderators,
                &(&reports, &appeals),
                NOW,
                None,
            )
            .unwrap();
        assert!(matches!(
            closures.close(upheld, &moderators, &(&reports, &appeals), NOW, None),
            Err(QueueError::AlreadyClosed(_))
        ));

        let queue = ReviewQueue::rebuild_with_appeals(
            &reports,
            &appeals,
            &closures,
            &QueuePolicy::default(),
        )
        .unwrap();
        assert_eq!(queue.open_appeals(), []);
        assert_eq!(
            queue.closed_appeals()[0].closure.as_ref().unwrap().decision,
            ClosureDecision::Actioned
        );

        // The decision is in the same log, shown as an appeal decision
        // linked to the contested action, without the note.
        let after = view(&closures, &appeals);
        assert_eq!(after.appeal_decisions.total, 1);
        assert_eq!(
            after.appeal_decisions.entries[0].action,
            TransparencyAction::AppealDecided {
                sequence: logged.sequence,
                appeal_id: appeal.id,
                contested: contested.clone(),
                moderator: hex_of(&moderator),
                decision: ClosureDecision::Actioned,
                at_ms: NOW + 5,
            }
        );
        assert_eq!(after.report_closures.entries, [] as [TransparencyEntry; 0]);
        let json = serde_json::to_string(&after).unwrap();
        assert!(!json.contains("private moderator note"));
        assert!(!json.contains("please review"));

        // Upholding an appeal reverses nothing: the contested action is
        // still in the record, unchanged.
        assert_eq!(after.world_actions, before.world_actions);
        assert_eq!(revocation_hash(&after), contested);
    }

    #[test]
    fn the_log_appends_and_reloads_verified() {
        let dir = tempfile::tempdir().unwrap();
        let closures = ClosureLog::new();
        let mut appeals = AppealLog::new();
        let current = view(&closures, &appeals);
        let contested = revocation_hash(&current);
        let signed = SignedAppeal::sign(&contested, "on disk", NOW, &key(7)).unwrap();
        appeals
            .file(signed, &current, &Revokee, &closures, NOW, Some(dir.path()))
            .unwrap();

        let loaded = AppealLog::load(dir.path()).unwrap();
        assert_eq!(loaded, appeals);
        assert!(
            AppealLog::load(&dir.path().join("missing"))
                .unwrap()
                .is_empty()
        );

        let path = dir.path().join(APPEALS_FILE);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replace("on disk", "on dusk")).unwrap();
        assert!(AppealLog::load(dir.path()).is_err());
    }
}
