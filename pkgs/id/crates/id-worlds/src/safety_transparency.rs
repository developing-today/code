//! The operator's transparency log view (#14466).
//!
//! A read-only, replayable projection over the durable moderation
//! record. Design: creator public-world safety v3 outline,
//! `doc/2026-11-24T16-12-00Z_design_id6310_creator_public_world_safety_v3_outline.md`,
//! sections 1, 2, 4 question 3 and 6 item 3.
//!
//! The view is never a second store. [`replay`] reads the two records
//! the operator already holds and returns a [`TransparencyView`]:
//!
//! - the world journal's moderation entries, the v1 durable moderation
//!   record (v1 design `doc/2026-11-18_design_id6308_creator_owner_basics_v1.md`
//!   section 5): today that is a `Revoked` entry, with the moderator
//!   participant who revoked (`None` means the admin) and the world-clock
//!   time when the journal carries one; and
//! - the [`ClosureLog`] of moderator decisions on safety reports
//!   (#14465). Appeal decisions (#14467) land in the same log;
//!   [`replay_with_appeals`] shows them as their own section.
//!
//! Replaying the same inputs always yields the same view, entry for
//! entry, so anyone holding the records can check it.
//!
//! # Sensitivity (v1 design section 7)
//!
//! Moderation records are sensitive world data:
//!
//! - Only a reader the caller's [`ReadAuthority`] accepts may replay the
//!   view (outline question 3: the operator's owner/admin read path).
//!   A refusal is [`TransparencyError::NotAuthorized`], never an empty
//!   view.
//! - The view carries stable ids only. A closure's free-text note, a
//!   report's detail and the reporter's key are never copied into it.
//! - Each section is bounded by [`ViewLimits`]; the counts still cover
//!   the whole record, and `truncated` says when entries were left out.
//! - A malformed, foreign, unknown-version or oversized journal is
//!   refused whole, as a distinct [`TransparencyError`], and a torn last
//!   line is reported, so a partial view never looks complete.
//!
//! Each entry carries an [`TransparencyEntry::action_hash`]: the BLAKE3
//! hash of the entry's canonical JSON under a domain separator. It is
//! the stable handle an appeal can name (outline question 1).

use std::fmt;
use std::io::{self, BufRead, Read};

use serde::Serialize;

use crate::safety_appeal::AppealLog;
use crate::safety_review_queue::{ClosureDecision, ClosureLog, QueueError};
use crate::world::{JournalEntry, MAX_JOURNAL_VERSION, journal_version_required_by};
use crate::world_session::encode_hex;

/// Domain separator for an entry's action hash.
pub const ACTION_HASH_DOMAIN: &[u8] = b"developing-today/id-safety-action/v1\0";

/// Major version of the view's JSON shape.
pub const TRANSPARENCY_SCHEMA_VERSION: u32 = 1;

/// Who may read the moderation record. The binding to the world role
/// model (owner/admin) is the caller's; this module only asks.
pub trait ReadAuthority {
    /// Whether this reader (as the caller names it) may read the
    /// operator's moderation record.
    fn may_read(&self, reader: &str) -> bool;
}

/// Bounds on what one view returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewLimits {
    /// Most entries per section; the newest are kept.
    pub max_entries: usize,
    /// Longest journal line accepted, in bytes.
    pub max_line_bytes: usize,
}

impl Default for ViewLimits {
    fn default() -> Self {
        Self {
            max_entries: 200,
            max_line_bytes: 1024 * 1024,
        }
    }
}

/// Who took a world moderation action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorldActor {
    /// The world's admin.
    Admin,
    /// A participant with the moderator scope.
    Moderator {
        /// The moderator participant.
        participant_id: u64,
    },
}

/// One moderation action, as the view shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum TransparencyAction {
    /// A participant's capability was revoked (journal `Revoked`).
    ParticipantRevoked {
        /// One-based line of the journal entry.
        journal_line: u64,
        /// Whose capability was revoked.
        participant_id: u64,
        /// Who revoked it.
        by: WorldActor,
        /// World-clock time, when the journal recorded one.
        #[serde(skip_serializing_if = "Option::is_none")]
        at_ms: Option<u64>,
    },
    /// A moderator closed a safety report.
    ReportClosed {
        /// Position in the closure log.
        sequence: u64,
        /// The report's id.
        report_id: String,
        /// The moderator's public key, lower-case hex.
        moderator: String,
        /// What was decided.
        decision: ClosureDecision,
        /// When the moderator decided, Unix milliseconds.
        at_ms: u64,
    },
    /// A moderator decided an appeal (#14467). `Actioned` upholds the
    /// appeal; any reversal is a separate action through the role
    /// model, never automatic.
    AppealDecided {
        /// Position in the closure log.
        sequence: u64,
        /// The appeal's id.
        appeal_id: String,
        /// The action hash of the action the appeal contests.
        contested: String,
        /// The moderator's public key, lower-case hex.
        moderator: String,
        /// What was decided.
        decision: ClosureDecision,
        /// When the moderator decided, Unix milliseconds.
        at_ms: u64,
    },
}

/// One entry of the view.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TransparencyEntry {
    /// The action.
    #[serde(flatten)]
    pub action: TransparencyAction,
    /// BLAKE3 hex of the action's canonical JSON, under
    /// [`ACTION_HASH_DOMAIN`].
    pub action_hash: String,
}

impl TransparencyEntry {
    fn new(action: TransparencyAction) -> Self {
        let action_hash = action_hash(&action);
        Self {
            action,
            action_hash,
        }
    }

    /// The stable hash of this entry's action.
    #[must_use]
    pub fn action_hash(&self) -> &str {
        &self.action_hash
    }
}

/// The hash an appeal names an action by.
#[must_use]
pub fn action_hash(action: &TransparencyAction) -> String {
    // Serialising a plain enum of strings and integers cannot fail.
    let json = serde_json::to_vec(action).unwrap_or_default();
    let mut hasher = blake3::Hasher::new();
    hasher.update(ACTION_HASH_DOMAIN);
    hasher.update(&json);
    encode_hex(hasher.finalize().as_bytes())
}

/// One bounded section of the view.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ViewSection {
    /// Every action in the whole record.
    pub total: u64,
    /// The newest actions, oldest first, at most
    /// [`ViewLimits::max_entries`].
    pub entries: Vec<TransparencyEntry>,
    /// Whether older entries were left out.
    pub truncated: bool,
}

impl ViewSection {
    fn push(&mut self, entry: TransparencyEntry, max: usize) {
        self.total += 1;
        if max == 0 {
            self.truncated = true;
            return;
        }
        if self.entries.len() == max {
            self.entries.remove(0);
            self.truncated = true;
        }
        self.entries.push(entry);
    }
}

/// The read-only transparency view.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TransparencyView {
    /// [`TRANSPARENCY_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The world whose journal was read.
    pub world_id: String,
    /// Moderation actions from the world journal, in journal order.
    pub world_actions: ViewSection,
    /// Report closures, in closure-log order.
    pub report_closures: ViewSection,
    /// Appeal decisions, in closure-log order.
    pub appeal_decisions: ViewSection,
    /// Whether the journal's last line was cut off (and not read).
    pub journal_torn_tail: bool,
}

impl TransparencyView {
    /// Every entry of every section: world actions, report closures,
    /// then appeal decisions.
    pub fn entries(&self) -> impl Iterator<Item = &TransparencyEntry> {
        self.world_actions
            .entries
            .iter()
            .chain(self.report_closures.entries.iter())
            .chain(self.appeal_decisions.entries.iter())
    }

    /// The entry with this action hash, if the view holds it.
    #[must_use]
    pub fn find(&self, action_hash: &str) -> Option<&TransparencyEntry> {
        self.entries()
            .find(|entry| entry.action_hash == action_hash)
    }
}

/// Why no view was produced. Each is distinct, so a caller never shows
/// a refusal as an empty view.
#[derive(Debug)]
pub enum TransparencyError {
    /// The reader may not read the moderation record.
    NotAuthorized,
    /// The journal does not start with its header.
    MissingHeader,
    /// The journal belongs to another world.
    WrongWorld {
        /// The world named in the header.
        found: String,
    },
    /// The header's version is outside what this binary reads.
    UnsupportedJournalVersion {
        /// The header version.
        version: u32,
    },
    /// A line is not a journal entry, or repeats the header.
    Malformed {
        /// One-based line number.
        line: u64,
    },
    /// An entry needs a newer journal version than its header declares.
    EntryNewerThanHeader {
        /// One-based line number.
        line: u64,
        /// The version the entry needs.
        required: u32,
    },
    /// A line is longer than [`ViewLimits::max_line_bytes`].
    OversizedLine {
        /// One-based line number.
        line: u64,
    },
    /// A closure in the closure log no longer verifies.
    Closure(QueueError),
    /// An appeal in the appeal log no longer verifies.
    Appeal(String),
    /// Reading the journal failed.
    Io(io::Error),
}

impl fmt::Display for TransparencyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAuthorized => write!(f, "not authorized to read the moderation record"),
            Self::MissingHeader => write!(f, "world journal does not start with a header"),
            Self::WrongWorld { found } => write!(f, "journal belongs to world {found:?}"),
            Self::UnsupportedJournalVersion { version } => {
                write!(f, "unsupported world journal version {version}")
            }
            Self::Malformed { line } => write!(f, "journal line {line} is not a journal entry"),
            Self::EntryNewerThanHeader { line, required } => write!(
                f,
                "journal line {line} needs journal version {required}, newer than its header"
            ),
            Self::OversizedLine { line } => write!(f, "journal line {line} exceeds the line limit"),
            Self::Closure(error) => write!(f, "closure log: {error}"),
            Self::Appeal(error) => write!(f, "appeal log: {error}"),
            Self::Io(error) => write!(f, "read journal: {error}"),
        }
    }
}

impl std::error::Error for TransparencyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

/// What to replay.
#[derive(Clone, Copy, Debug)]
pub struct ViewRequest<'a> {
    /// The world whose journal is read.
    pub world_id: &'a str,
    /// Who asks.
    pub reader: &'a str,
    /// Output bounds.
    pub limits: ViewLimits,
}

/// Replay the transparency view from a world journal and the closure
/// log.
///
/// # Errors
///
/// Any [`TransparencyError`]: the reader is refused, or the journal is
/// refused whole rather than shown in part as if complete.
pub fn replay<R: BufRead>(
    journal: R,
    closures: &ClosureLog,
    request: &ViewRequest<'_>,
    authority: &impl ReadAuthority,
) -> Result<TransparencyView, TransparencyError> {
    replay_with_appeals(journal, closures, &AppealLog::new(), request, authority)
}

/// [`replay`], with the appeal log: a closure that names an appeal
/// shows as an [`TransparencyAction::AppealDecided`] entry.
///
/// # Errors
///
/// As [`replay`], or an appeal no longer verifies.
pub fn replay_with_appeals<R: BufRead>(
    mut journal: R,
    closures: &ClosureLog,
    appeals: &AppealLog,
    request: &ViewRequest<'_>,
    authority: &impl ReadAuthority,
) -> Result<TransparencyView, TransparencyError> {
    if !authority.may_read(request.reader) {
        return Err(TransparencyError::NotAuthorized);
    }
    let limits = request.limits;
    let mut view = TransparencyView {
        schema_version: TRANSPARENCY_SCHEMA_VERSION,
        world_id: request.world_id.to_owned(),
        world_actions: ViewSection::default(),
        report_closures: ViewSection::default(),
        appeal_decisions: ViewSection::default(),
        journal_torn_tail: false,
    };
    let mut header_version = None;
    let mut buf = Vec::new();
    let mut line = 0_u64;
    loop {
        line += 1;
        let Some(terminated) = next_line(&mut journal, &mut buf, limits.max_line_bytes, line)?
        else {
            break;
        };
        if !terminated {
            view.journal_torn_tail = true;
            break;
        }
        let text = buf.strip_suffix(b"\n").unwrap_or(&buf);
        if text.is_empty() {
            continue;
        }
        let entry: JournalEntry = serde_json::from_slice(text)
            .map_err(|_parse_error| TransparencyError::Malformed { line })?;
        let Some(version) = header_version else {
            let JournalEntry::Created { world_id, version } = entry else {
                return Err(TransparencyError::MissingHeader);
            };
            if world_id != request.world_id {
                return Err(TransparencyError::WrongWorld { found: world_id });
            }
            if !(1..=MAX_JOURNAL_VERSION).contains(&version) {
                return Err(TransparencyError::UnsupportedJournalVersion { version });
            }
            header_version = Some(version);
            continue;
        };
        let required = journal_version_required_by(&entry);
        if required > version {
            return Err(TransparencyError::EntryNewerThanHeader { line, required });
        }
        match entry {
            JournalEntry::Created { .. } => return Err(TransparencyError::Malformed { line }),
            JournalEntry::Revoked {
                participant_id,
                by,
                revoked_at,
            } => view.world_actions.push(
                TransparencyEntry::new(TransparencyAction::ParticipantRevoked {
                    journal_line: line,
                    participant_id,
                    by: by.map_or(WorldActor::Admin, |participant_id| WorldActor::Moderator {
                        participant_id,
                    }),
                    at_ms: revoked_at,
                }),
                limits.max_entries,
            ),
            _ => {}
        }
    }
    if header_version.is_none() && !view.journal_torn_tail {
        return Err(TransparencyError::MissingHeader);
    }
    for fact in closures.facts() {
        let closure = fact.signed.verify().map_err(TransparencyError::Closure)?;
        if let Some(appeal) = appeals.get(&closure.report_id) {
            let appeal = appeal
                .appeal()
                .map_err(|e| TransparencyError::Appeal(e.to_string()))?;
            view.appeal_decisions.push(
                TransparencyEntry::new(TransparencyAction::AppealDecided {
                    sequence: fact.sequence,
                    appeal_id: closure.report_id,
                    contested: appeal.action_hash,
                    moderator: closure.moderator,
                    decision: closure.decision,
                    at_ms: closure.decided_at_ms,
                }),
                limits.max_entries,
            );
            continue;
        }
        view.report_closures.push(
            TransparencyEntry::new(TransparencyAction::ReportClosed {
                sequence: fact.sequence,
                report_id: closure.report_id,
                moderator: closure.moderator,
                decision: closure.decision,
                at_ms: closure.decided_at_ms,
            }),
            limits.max_entries,
        );
    }
    Ok(view)
}

fn next_line<R: BufRead>(
    journal: &mut R,
    buf: &mut Vec<u8>,
    max: usize,
    line: u64,
) -> Result<Option<bool>, TransparencyError> {
    buf.clear();
    let cap = u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1);
    let read = journal
        .by_ref()
        .take(cap)
        .read_until(b'\n', buf)
        .map_err(TransparencyError::Io)?;
    if read == 0 {
        return Ok(None);
    }
    let terminated = buf.last() == Some(&b'\n');
    if buf.len() > max || (!terminated && read as u64 == cap) {
        return Err(TransparencyError::OversizedLine { line });
    }
    Ok(Some(terminated))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use ed25519_dalek::SigningKey;

    use super::*;
    use crate::safety_report::{HostedScope, ReportLog, ReportReason, ReportTarget, SignedReport};
    use crate::safety_review_queue::{Moderators, SignedClosure};
    use crate::world::Records;

    const NOW: u64 = 1_795_000_000_000;
    const WORLD: &str = "quiz-hall";

    struct Admins(&'static [&'static str]);

    impl ReadAuthority for Admins {
        fn may_read(&self, reader: &str) -> bool {
            self.0.contains(&reader)
        }
    }

    const ADMINS: Admins = Admins(&["owner"]);

    fn key(byte: u8) -> SigningKey {
        SigningKey::from_bytes(&[byte; 32])
    }

    fn request(reader: &str) -> ViewRequest<'_> {
        ViewRequest {
            world_id: WORLD,
            reader,
            limits: ViewLimits::default(),
        }
    }

    fn journal(entries: &[JournalEntry]) -> Vec<u8> {
        let mut out = Vec::new();
        for entry in entries {
            out.extend(serde_json::to_vec(entry).unwrap());
            out.push(b'\n');
        }
        out
    }

    fn header() -> JournalEntry {
        JournalEntry::Created {
            world_id: WORLD.to_owned(),
            version: MAX_JOURNAL_VERSION,
        }
    }

    fn revoked(participant_id: u64, by: Option<u64>, at: Option<u64>) -> JournalEntry {
        JournalEntry::Revoked {
            participant_id,
            by,
            revoked_at: at,
        }
    }

    /// A report log with one report, and a closure log closing it with
    /// a free-text note.
    fn closed_report() -> (ReportLog, ClosureLog, String) {
        let mut scope = HostedScope::new();
        scope.host(WORLD, &Records::new()).unwrap();
        let mut reports = ReportLog::new();
        let report = SignedReport::sign(
            ReportTarget::HostedWorld {
                world_id: WORLD.to_owned(),
            },
            ReportReason::Abuse,
            "private detail from the reporter",
            NOW - 100,
            &key(1),
        )
        .unwrap();
        let fact = reports.file(report, &scope, NOW, None).unwrap();
        let moderator = key(40);
        let moderators = Moderators::new([moderator.verifying_key()]);
        let mut closures = ClosureLog::new();
        let closure = SignedClosure::sign(
            &fact.id,
            ClosureDecision::Actioned,
            "private moderator note",
            NOW - 10,
            &moderator,
        )
        .unwrap();
        closures
            .close(closure, &moderators, &reports, NOW, None)
            .unwrap();
        (reports, closures, fact.id)
    }

    #[test]
    fn view_lists_journal_moderation_and_closures_in_order() {
        let (_reports, closures, report_id) = closed_report();
        let bytes = journal(&[
            header(),
            revoked(7, None, Some(NOW - 50)),
            revoked(8, Some(3), None),
        ]);
        let view = replay(bytes.as_slice(), &closures, &request("owner"), &ADMINS).unwrap();

        let actions: Vec<_> = view.entries().map(|entry| entry.action.clone()).collect();
        assert_eq!(
            actions,
            vec![
                TransparencyAction::ParticipantRevoked {
                    journal_line: 2,
                    participant_id: 7,
                    by: WorldActor::Admin,
                    at_ms: Some(NOW - 50),
                },
                TransparencyAction::ParticipantRevoked {
                    journal_line: 3,
                    participant_id: 8,
                    by: WorldActor::Moderator { participant_id: 3 },
                    at_ms: None,
                },
                TransparencyAction::ReportClosed {
                    sequence: 1,
                    report_id,
                    moderator: encode_hex(&key(40).verifying_key().to_bytes()),
                    decision: ClosureDecision::Actioned,
                    at_ms: NOW - 10,
                },
            ]
        );
        assert_eq!(view.world_actions.total, 2);
        assert_eq!(view.report_closures.total, 1);
        assert!(!view.journal_torn_tail);
    }

    #[test]
    fn replay_is_deterministic_and_hashes_name_actions() {
        let (_reports, closures, _id) = closed_report();
        let bytes = journal(&[header(), revoked(7, None, Some(NOW))]);
        let first = replay(bytes.as_slice(), &closures, &request("owner"), &ADMINS).unwrap();
        let second = replay(bytes.as_slice(), &closures, &request("owner"), &ADMINS).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            serde_json::to_string(&first).unwrap(),
            serde_json::to_string(&second).unwrap()
        );

        let hashes: Vec<_> = first
            .entries()
            .map(|e| e.action_hash().to_owned())
            .collect();
        assert_eq!(hashes.len(), 2);
        assert_ne!(hashes[0], hashes[1]);
        for hash in &hashes {
            assert_eq!(hash.len(), 64);
            assert_eq!(first.find(hash).unwrap().action_hash(), hash);
        }
        // A different action hashes differently.
        let other = journal(&[header(), revoked(7, Some(1), Some(NOW))]);
        let other = replay(
            other.as_slice(),
            &ClosureLog::new(),
            &request("owner"),
            &ADMINS,
        )
        .unwrap();
        assert!(
            first
                .find(other.world_actions.entries[0].action_hash())
                .is_none()
        );
    }

    #[test]
    fn only_an_authorized_reader_gets_a_view() {
        let bytes = journal(&[header(), revoked(7, None, None)]);
        let refused = replay(
            bytes.as_slice(),
            &ClosureLog::new(),
            &request("visitor"),
            &ADMINS,
        );
        assert!(matches!(refused, Err(TransparencyError::NotAuthorized)));
    }

    #[test]
    fn view_carries_no_free_text_or_reporter_identity() {
        let (reports, closures, _id) = closed_report();
        let bytes = journal(&[header()]);
        let view = replay(bytes.as_slice(), &closures, &request("owner"), &ADMINS).unwrap();
        let json = serde_json::to_string(&view).unwrap();
        assert!(!json.contains("private moderator note"));
        assert!(!json.contains("private detail"));
        let reporter = &reports.facts()[0].report().unwrap().reporter;
        assert!(!json.contains(reporter.as_str()));
    }

    #[test]
    fn sections_are_bounded_but_counts_cover_the_record() {
        let mut entries = vec![header()];
        entries.extend((1..=5).map(|id| revoked(id, None, Some(NOW + id))));
        let bytes = journal(&entries);
        let mut req = request("owner");
        req.limits.max_entries = 2;
        let view = replay(bytes.as_slice(), &ClosureLog::new(), &req, &ADMINS).unwrap();
        assert_eq!(view.world_actions.total, 5);
        assert!(view.world_actions.truncated);
        let kept: Vec<_> = view
            .world_actions
            .entries
            .iter()
            .map(|entry| match &entry.action {
                TransparencyAction::ParticipantRevoked { participant_id, .. } => *participant_id,
                TransparencyAction::ReportClosed { .. }
                | TransparencyAction::AppealDecided { .. } => panic!("not a world action"),
            })
            .collect();
        assert_eq!(kept, vec![4, 5]);
    }

    #[test]
    fn a_bad_journal_is_refused_whole() {
        let closures = ClosureLog::new();
        let owner = request("owner");
        let replay_bytes = |bytes: &[u8]| replay(bytes, &closures, &owner, &ADMINS);

        let no_header = journal(&[revoked(7, None, None)]);
        assert!(matches!(
            replay_bytes(&no_header),
            Err(TransparencyError::MissingHeader)
        ));
        assert!(matches!(
            replay_bytes(b""),
            Err(TransparencyError::MissingHeader)
        ));

        let foreign = journal(&[JournalEntry::Created {
            world_id: "elsewhere".to_owned(),
            version: 1,
        }]);
        assert!(matches!(
            replay_bytes(&foreign),
            Err(TransparencyError::WrongWorld { found }) if found == "elsewhere"
        ));

        let future = journal(&[JournalEntry::Created {
            world_id: WORLD.to_owned(),
            version: MAX_JOURNAL_VERSION + 1,
        }]);
        assert!(matches!(
            replay_bytes(&future),
            Err(TransparencyError::UnsupportedJournalVersion { .. })
        ));

        let mut garbage = journal(&[header()]);
        garbage.extend_from_slice(b"not json\n");
        assert!(matches!(
            replay_bytes(&garbage),
            Err(TransparencyError::Malformed { line: 2 })
        ));

        // A timed revoke needs the join-times journal version.
        let old = journal(&[
            JournalEntry::Created {
                world_id: WORLD.to_owned(),
                version: 1,
            },
            revoked(7, None, Some(NOW)),
        ]);
        assert!(matches!(
            replay_bytes(&old),
            Err(TransparencyError::EntryNewerThanHeader { line: 2, .. })
        ));

        let mut req = request("owner");
        req.limits.max_line_bytes = 8;
        let long = journal(&[header()]);
        assert!(matches!(
            replay(long.as_slice(), &closures, &req, &ADMINS),
            Err(TransparencyError::OversizedLine { line: 1 })
        ));
    }

    #[test]
    fn a_torn_last_line_is_reported_not_hidden() {
        let mut bytes = journal(&[header(), revoked(7, None, None)]);
        bytes.extend_from_slice(b"{\"entry\":\"revoked\",\"partici");
        let view = replay(
            bytes.as_slice(),
            &ClosureLog::new(),
            &request("owner"),
            &ADMINS,
        )
        .unwrap();
        assert!(view.journal_torn_tail);
        assert_eq!(view.world_actions.total, 1);
    }
}
