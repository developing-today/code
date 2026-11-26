//! Thread resolution and change proposals as signed events (#7849 slice 1,
//! decision record `doc/2026-11-08_decision_id5874_thread_resolution.md`).
//!
//! Four additive event kinds sit beside `chat.message.v1` on the same protocol:
//!
//! - [`CHAT_RESOLUTION_KIND`] — a thread is resolved into an edit, a decision,
//!   or no action (§2 of the record);
//! - [`CHAT_PROPOSAL_KIND`] — a concrete proposed revision of an object, pinned
//!   to its base and carried by blob link (§3);
//! - [`CHAT_PROPOSAL_REJECTION_KIND`] — an object writer rejects a proposal,
//!   with a blob-linked reason;
//! - [`CHAT_PROPOSAL_WITHDRAWAL_KIND`] — the proposer withdraws a proposal.
//!   The record names withdrawal without spelling its kind; this module names
//!   it `chat.proposal-withdrawal.v1` beside the rejection kind.
//!
//! Nothing here is stored state. [`project_threads`] folds a set of these events
//! (plus the admitted edits that cite proposals, which the object models own)
//! into a [`ThreadProjection`], and every state a client shows is re-derived from
//! the event set alone (record §5 invariant 1). The fold:
//!
//! - drops duplicate deliveries by event ID (first delivery wins) and returns
//!   every list sorted, so arrival order never changes the result (invariant 4);
//! - keeps, per thread, the causally maximal resolutions: a later resolution
//!   re-resolves the thread, and concurrent resolutions from two heads are kept
//!   side by side — no head is silently chosen (§2);
//! - derives each proposal's lifecycle from the events that cite it: an
//!   admitted edit citing the proposal on the same object is the acceptance
//!   (there is no approval write), a rejection by an object writer rejects it, a
//!   withdrawal by its proposer withdraws it, and a later proposal by the same
//!   proposer against the same (object, base) that cites it supersedes it (§3);
//! - refuses events whose signer lacks the authority the record gives that act
//!   (§4): a resolution from someone who is neither a thread participant nor a
//!   writer on an object the thread is anchored to, a rejection from a
//!   non-writer, a withdrawal from anyone but the proposer. A refused event
//!   changes no state; its ID is listed in [`ThreadProjection::refused`] so the
//!   audit can see it. The authority answers come from the caller
//!   ([`ThreadAuthority`]), so this module adds no second permission system.
//!
//! When several closing events apply to one proposal, the strongest wins in a
//! fixed order — accepted, rejected, withdrawn, superseded — and every closing
//! event stays listed. Acceptance is an edit already made, so nothing undoes it.
//!
//! Visibility is a query-time view ([`ThreadProjection::visible_to`]), the
//! `note_links` mechanism: a resolution or proposal whose thread or object the
//! viewer cannot read is dropped whole, so neither the object nor its existence
//! crosses by edge (invariant 6).
//!
//! Causal order uses the same parents chat messages carry: `after` (the nearest
//! thread events on this event's causal paths, as the caller projects them) and
//! the author's own backlink.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use p2panda_core::Hash;
use serde::{Deserialize, Serialize};

use crate::chat::{ChatLink, ChatLinkKind};
use crate::chat_anchor::ObjectRef;
use id_core::waiting::AuthorKey;

/// The kind of a thread resolution event.
pub const CHAT_RESOLUTION_KIND: &str = "chat.resolution.v1";
/// The kind of a change proposal event.
pub const CHAT_PROPOSAL_KIND: &str = "chat.proposal.v1";
/// The kind of a proposal rejection event.
pub const CHAT_PROPOSAL_REJECTION_KIND: &str = "chat.proposal-rejection.v1";
/// The kind of a proposal withdrawal event.
pub const CHAT_PROPOSAL_WITHDRAWAL_KIND: &str = "chat.proposal-withdrawal.v1";

/// Whether `kind` is one of this module's four event kinds.
#[must_use]
pub fn is_thread_event_kind(kind: &str) -> bool {
    matches!(
        kind,
        CHAT_RESOLUTION_KIND
            | CHAT_PROPOSAL_KIND
            | CHAT_PROPOSAL_REJECTION_KIND
            | CHAT_PROPOSAL_WITHDRAWAL_KIND
    )
}

/// Why a thread event was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ThreadEventError {
    /// A field is missing, present where it must not be, or malformed.
    #[error("{0}")]
    Field(String),
    /// The body is not the one the event's kind names.
    #[error("a {kind:?} event carries the wrong body")]
    KindMismatch {
        /// The kind of the event.
        kind: String,
    },
}

/// What a resolution says the discussion came to (§2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResolutionKind {
    /// The discussion produced an edit on an object.
    Edit,
    /// The discussion settled a question, recorded in the summary.
    Decision,
    /// Considered; nothing to change.
    NoAction,
}

/// The body of a `chat.resolution.v1` event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatResolution {
    /// The root event ID of the thread being resolved.
    pub thread: Hash,
    /// What the discussion came to.
    pub kind: ResolutionKind,
    /// For `edit`: the object the discussion edited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<ObjectRef>,
    /// For `edit`: the edit event ID (the resulting revision) it produced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit: Option<Hash>,
    /// For `decision`: the decision text, by blob link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<ChatLink>,
}

impl ChatResolution {
    /// Resolves `thread` into `edit` on `object`.
    #[must_use]
    pub fn edit(thread: Hash, object: ObjectRef, edit: Hash) -> Self {
        Self {
            thread,
            kind: ResolutionKind::Edit,
            object: Some(object),
            edit: Some(edit),
            summary: None,
        }
    }

    /// Resolves `thread` as a decision whose text is the blob `summary`.
    ///
    /// # Errors
    ///
    /// Returns [`ThreadEventError::Field`] when `summary` is not a valid blob link.
    pub fn decision(thread: Hash, summary: ChatLink) -> Result<Self, ThreadEventError> {
        let resolution = Self {
            thread,
            kind: ResolutionKind::Decision,
            object: None,
            edit: None,
            summary: Some(summary),
        };
        resolution.check()?;
        Ok(resolution)
    }

    /// Resolves `thread` with no action.
    #[must_use]
    pub fn no_action(thread: Hash) -> Self {
        Self {
            thread,
            kind: ResolutionKind::NoAction,
            object: None,
            edit: None,
            summary: None,
        }
    }

    /// Checks that the fields match the kind: `edit` names an object and an
    /// edit and nothing else, `decision` a blob summary and nothing else,
    /// `no-action` none of them.
    ///
    /// # Errors
    ///
    /// Returns [`ThreadEventError::Field`] naming the mismatch.
    pub fn check(&self) -> Result<(), ThreadEventError> {
        let (object, edit, summary) = (
            self.object.is_some(),
            self.edit.is_some(),
            self.summary.is_some(),
        );
        let ok = match self.kind {
            ResolutionKind::Edit => object && edit && !summary,
            ResolutionKind::Decision => !object && !edit && summary,
            ResolutionKind::NoAction => !object && !edit && !summary,
        };
        if !ok {
            return Err(ThreadEventError::Field(format!(
                "a {:?} resolution carries the wrong fields: an edit names an object and \
                 an edit, a decision a summary, a no-action neither",
                self.kind
            )));
        }
        if let Some(summary) = &self.summary {
            check_blob(summary, "decision summary")?;
        }
        Ok(())
    }
}

/// The body of a `chat.proposal.v1` event (§3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatProposal {
    /// The object the proposal would change.
    pub object: ObjectRef,
    /// The exact revision the proposal is against — pinned, never "latest".
    pub base: Hash,
    /// The full proposed revision content, by blob link (not a diff).
    pub content: ChatLink,
    /// The thread the proposal came out of.
    pub thread: Hash,
    /// An earlier proposal by the same proposer against the same (object,
    /// base) that this one supersedes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<Hash>,
}

impl ChatProposal {
    /// Checks that the content is a valid blob link.
    ///
    /// # Errors
    ///
    /// Returns [`ThreadEventError::Field`] when it is not.
    pub fn check(&self) -> Result<(), ThreadEventError> {
        check_blob(&self.content, "proposal content")
    }
}

/// The body of a `chat.proposal-rejection.v1` event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatProposalRejection {
    /// The proposal being rejected.
    pub proposal: Hash,
    /// Why, by blob link.
    pub reason: ChatLink,
}

/// The body of a `chat.proposal-withdrawal.v1` event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatProposalWithdrawal {
    /// The proposal being withdrawn.
    pub proposal: Hash,
}

/// One thread event body, tagged by the kind its event carries. In an event
/// header it is encoded with its variant name as well, so a body never decodes
/// as another kind's; the header's check still requires the two to agree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThreadEvent {
    /// A `chat.resolution.v1` body.
    Resolution(ChatResolution),
    /// A `chat.proposal.v1` body.
    Proposal(ChatProposal),
    /// A `chat.proposal-rejection.v1` body.
    Rejection(ChatProposalRejection),
    /// A `chat.proposal-withdrawal.v1` body.
    Withdrawal(ChatProposalWithdrawal),
}

impl ThreadEvent {
    /// The kind of the event that carries this body.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Resolution(_) => CHAT_RESOLUTION_KIND,
            Self::Proposal(_) => CHAT_PROPOSAL_KIND,
            Self::Rejection(_) => CHAT_PROPOSAL_REJECTION_KIND,
            Self::Withdrawal(_) => CHAT_PROPOSAL_WITHDRAWAL_KIND,
        }
    }

    /// Checks the body's own field rules.
    ///
    /// # Errors
    ///
    /// Returns the body's error.
    pub fn check(&self) -> Result<(), ThreadEventError> {
        match self {
            Self::Resolution(resolution) => resolution.check(),
            Self::Proposal(proposal) => proposal.check(),
            Self::Rejection(rejection) => check_blob(&rejection.reason, "rejection reason"),
            Self::Withdrawal(_) => Ok(()),
        }
    }
}

/// Checks a thread event body against the kind of the event that carries it.
///
/// # Errors
///
/// Returns [`ThreadEventError::KindMismatch`] when `kind` is not the body's kind,
/// and the body's own error otherwise.
pub fn check_thread_event(kind: &str, event: &ThreadEvent) -> Result<(), ThreadEventError> {
    if kind != event.kind() {
        return Err(ThreadEventError::KindMismatch {
            kind: kind.to_owned(),
        });
    }
    event.check()
}

fn check_blob(link: &ChatLink, label: &str) -> Result<(), ThreadEventError> {
    if link.kind != ChatLinkKind::Blob {
        return Err(ThreadEventError::Field(format!(
            "the {label} must be a blob link"
        )));
    }
    link.check()
        .map_err(|error| ThreadEventError::Field(format!("the {label}: {error}")))
}

/// An applied thread event, as the fold reads it. Signature verification
/// happens where events are admitted; a record here is already signed by
/// `author`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadEventRecord {
    /// The event's ID.
    pub id: Hash,
    /// The key that signed the event.
    pub author: AuthorKey,
    /// The nearest thread events on this event's causal paths.
    pub after: Vec<Hash>,
    /// The author's own previous event, a causal parent like any `after` entry.
    pub backlink: Option<Hash>,
    /// The body.
    pub event: ThreadEvent,
}

/// An admitted edit event on an object that cites a proposal as its
/// provenance. The object's own model admits the edit (only a writer can make
/// one, §4); the fold reads the citation, which lives inside the signed edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcceptingEdit {
    /// The edit event's ID.
    pub id: Hash,
    /// The key that signed the edit.
    pub author: AuthorKey,
    /// The object the edit is on.
    pub object: ObjectRef,
    /// The proposal the edit cites.
    pub proposal: Hash,
}

/// The authority answers the fold needs, supplied by the caller from the
/// group policy and the objects' own admission rules.
pub trait ThreadAuthority {
    /// Whether `author` may resolve `thread`: a participant in it, or a
    /// writer on an object it is anchored to.
    fn may_resolve(&self, thread: &Hash, author: &AuthorKey) -> bool;
    /// Whether `author` has write authority on `object`.
    fn may_write(&self, object: &ObjectRef, author: &AuthorKey) -> bool;
}

/// One projected resolution event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolutionRecord {
    /// The resolution event's ID.
    pub id: Hash,
    /// The resolver.
    pub author: AuthorKey,
    /// The resolution body.
    pub resolution: ChatResolution,
}

/// A thread's folded resolution: its causally maximal resolutions. One head
/// is a settled resolution; more than one are concurrent resolutions, shown
/// side by side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadResolution {
    /// The thread's root event ID.
    pub thread: Hash,
    /// The causally maximal resolutions, sorted by event ID.
    pub heads: Vec<ResolutionRecord>,
}

impl ThreadResolution {
    /// Whether concurrent resolutions remain unmerged.
    #[must_use]
    pub fn is_concurrent(&self) -> bool {
        self.heads.len() > 1
    }
}

/// A proposal's derived lifecycle state (§3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProposalState {
    /// Nothing below has happened.
    Proposed,
    /// An admitted edit on the object cites it.
    Accepted,
    /// An object writer rejected it.
    Rejected,
    /// Its proposer withdrew it.
    Withdrawn,
    /// A later proposal by its proposer against the same (object, base) cites it.
    Superseded,
}

/// One projected proposal and everything that closed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalRecord {
    /// The proposal event's ID.
    pub id: Hash,
    /// The proposer.
    pub author: AuthorKey,
    /// The proposal body.
    pub proposal: ChatProposal,
    /// The derived state.
    pub state: ProposalState,
    /// Admitted edits citing it, sorted.
    pub accepted_by: Vec<Hash>,
    /// Rejections by object writers, sorted by event ID.
    pub rejected_by: Vec<(Hash, ChatLink)>,
    /// Withdrawals by the proposer, sorted.
    pub withdrawn_by: Vec<Hash>,
    /// Later proposals that supersede it, sorted.
    pub superseded_by: Vec<Hash>,
}

impl ProposalRecord {
    /// Whether the proposal's base is no longer a head of its object, given the
    /// object's current `heads`. Staleness is an observation, not a state: a
    /// writer may still accept a stale proposal by making the edit.
    #[must_use]
    pub fn is_stale(&self, heads: &[Hash]) -> bool {
        !heads.contains(&self.proposal.base)
    }
}

/// Where a proposal's base stands against its object's current heads, as an
/// editor shows it beside the proposal (§3, §8 item 2). Staleness is an
/// observation made here, at presentation time, never a stored state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BaseStanding {
    /// The proposal is still open and its base is one of the object's heads:
    /// accepting it is a plain edit on that head.
    Current,
    /// The proposal is still open but its base is no longer a head. The
    /// object's current heads are named so the editor can show what changed
    /// since; a writer may still accept it by making the edit.
    Stale {
        /// The object's current heads, sorted.
        heads: Vec<Hash>,
    },
    /// The proposal is accepted, rejected, withdrawn or superseded, so its
    /// base no longer matters.
    Closed,
}

/// One proposal against an object, with where its base stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalReview<'a> {
    /// The projected proposal.
    pub record: &'a ProposalRecord,
    /// Where its base stands against the object's current heads.
    pub base: BaseStanding,
}

/// The deterministic projection of a set of thread events.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThreadProjection {
    /// Folded resolutions, sorted by thread.
    pub resolutions: Vec<ThreadResolution>,
    /// Folded proposals, sorted by event ID.
    pub proposals: Vec<ProposalRecord>,
    /// Events refused for missing authority, a malformed body, or naming a
    /// proposal that is not in the set, sorted.
    pub refused: Vec<Hash>,
}

impl ThreadProjection {
    /// The folded resolution of `thread`, if it has one.
    #[must_use]
    pub fn resolution_of(&self, thread: &Hash) -> Option<&ThreadResolution> {
        self.resolutions
            .iter()
            .find(|entry| entry.thread == *thread)
    }

    /// The resolved-into direction from an object revision back to the
    /// discussion: every head resolution of kind `edit` naming `edit` on
    /// `object`. Read off the same edge set as [`Self::resolution_of`].
    #[must_use]
    pub fn resolutions_producing(&self, object: &ObjectRef, edit: &Hash) -> Vec<&ResolutionRecord> {
        self.resolutions
            .iter()
            .flat_map(|entry| entry.heads.iter())
            .filter(|record| {
                record.resolution.kind == ResolutionKind::Edit
                    && record.resolution.object.as_ref() == Some(object)
                    && record.resolution.edit.as_ref() == Some(edit)
            })
            .collect()
    }

    /// The proposals against `object`, in projection order.
    #[must_use]
    pub fn proposals_for(&self, object: &ObjectRef) -> Vec<&ProposalRecord> {
        self.proposals
            .iter()
            .filter(|record| record.proposal.object == *object)
            .collect()
    }

    /// The proposals against `object` as its editor reviews them, given the
    /// object's current `heads`: each in projection order with where its base
    /// stands. An open proposal whose base is not among `heads` is
    /// [`BaseStanding::Stale`] and names those heads.
    #[must_use]
    pub fn review(&self, object: &ObjectRef, heads: &[Hash]) -> Vec<ProposalReview<'_>> {
        let mut current: Vec<Hash> = heads.to_vec();
        current.sort_by_key(|id| *id.as_bytes());
        current.dedup();
        self.proposals_for(object)
            .into_iter()
            .map(|record| {
                let base = if record.state != ProposalState::Proposed {
                    BaseStanding::Closed
                } else if record.is_stale(&current) {
                    BaseStanding::Stale {
                        heads: current.clone(),
                    }
                } else {
                    BaseStanding::Current
                };
                ProposalReview { record, base }
            })
            .collect()
    }

    /// The projection one viewer may see. `can_read_thread` and
    /// `can_read_object` answer for the viewer; anything naming a thread or
    /// object they cannot read is dropped whole, so neither its content nor
    /// its existence leaks through an edge. Refused event IDs are audit data
    /// about events, so the view keeps none of them.
    #[must_use]
    pub fn visible_to(
        &self,
        can_read_thread: &dyn Fn(&Hash) -> bool,
        can_read_object: &dyn Fn(&ObjectRef) -> bool,
    ) -> Self {
        let resolutions = self
            .resolutions
            .iter()
            .filter(|entry| can_read_thread(&entry.thread))
            .filter_map(|entry| {
                let heads: Vec<ResolutionRecord> = entry
                    .heads
                    .iter()
                    .filter(|record| {
                        record
                            .resolution
                            .object
                            .as_ref()
                            .is_none_or(can_read_object)
                    })
                    .cloned()
                    .collect();
                (!heads.is_empty()).then(|| ThreadResolution {
                    thread: entry.thread,
                    heads,
                })
            })
            .collect();
        let proposals: Vec<ProposalRecord> = self
            .proposals
            .iter()
            .filter(|record| {
                can_read_thread(&record.proposal.thread) && can_read_object(&record.proposal.object)
            })
            .cloned()
            .collect();
        let shown: HashSet<Hash> = proposals.iter().map(|record| record.id).collect();
        let proposals = proposals
            .into_iter()
            .map(|mut record| {
                // A superseding proposal on a thread the viewer cannot read
                // would leak that thread's existence through the edge.
                record.superseded_by.retain(|id| shown.contains(id));
                if record.state == ProposalState::Superseded && record.superseded_by.is_empty() {
                    record.state = ProposalState::Proposed;
                }
                record
            })
            .collect();
        Self {
            resolutions,
            proposals,
            refused: Vec::new(),
        }
    }
}

fn follows(record: Hash, ancestor: Hash, parents: &HashMap<Hash, Vec<Hash>>) -> bool {
    let mut stack = parents.get(&record).cloned().unwrap_or_default();
    let mut seen: HashSet<Hash> = HashSet::new();
    while let Some(id) = stack.pop() {
        if id == ancestor {
            return true;
        }
        if seen.insert(id) {
            if let Some(grandparents) = parents.get(&id) {
                stack.extend(grandparents.iter().copied());
            }
        }
    }
    false
}

/// Folds thread events and the admitted edits that cite proposals into the
/// deterministic [`ThreadProjection`]. The result depends only on the sets
/// given, never on their order or on duplicate deliveries.
#[must_use]
pub fn project_threads(
    events: impl IntoIterator<Item = ThreadEventRecord>,
    edits: impl IntoIterator<Item = AcceptingEdit>,
    authority: &dyn ThreadAuthority,
) -> ThreadProjection {
    let mut by_id: BTreeMap<[u8; 32], ThreadEventRecord> = BTreeMap::new();
    for record in events {
        by_id.entry(*record.id.as_bytes()).or_insert(record);
    }
    let mut edits_by_id: BTreeMap<[u8; 32], AcceptingEdit> = BTreeMap::new();
    for edit in edits {
        edits_by_id.entry(*edit.id.as_bytes()).or_insert(edit);
    }
    let mut parents: HashMap<Hash, Vec<Hash>> = HashMap::new();
    for record in by_id.values() {
        let mut list = record.after.clone();
        list.extend(record.backlink);
        parents.insert(record.id, list);
    }

    let mut refused: BTreeSet<[u8; 32]> = BTreeSet::new();
    let mut proposals: BTreeMap<[u8; 32], ProposalRecord> = BTreeMap::new();
    let mut resolutions: BTreeMap<[u8; 32], Vec<ResolutionRecord>> = BTreeMap::new();

    for record in by_id.values() {
        if record.event.check().is_err() {
            refused.insert(*record.id.as_bytes());
            continue;
        }
        match &record.event {
            ThreadEvent::Resolution(resolution) => {
                if authority.may_resolve(&resolution.thread, &record.author) {
                    resolutions
                        .entry(*resolution.thread.as_bytes())
                        .or_default()
                        .push(ResolutionRecord {
                            id: record.id,
                            author: record.author,
                            resolution: resolution.clone(),
                        });
                } else {
                    refused.insert(*record.id.as_bytes());
                }
            }
            ThreadEvent::Proposal(proposal) => {
                proposals.insert(
                    *record.id.as_bytes(),
                    ProposalRecord {
                        id: record.id,
                        author: record.author,
                        proposal: proposal.clone(),
                        state: ProposalState::Proposed,
                        accepted_by: Vec::new(),
                        rejected_by: Vec::new(),
                        withdrawn_by: Vec::new(),
                        superseded_by: Vec::new(),
                    },
                );
            }
            ThreadEvent::Rejection(_) | ThreadEvent::Withdrawal(_) => {}
        }
    }

    // Closing events, now that every proposal is known.
    let snapshot: BTreeMap<[u8; 32], (AuthorKey, ChatProposal)> = proposals
        .iter()
        .map(|(id, record)| (*id, (record.author, record.proposal.clone())))
        .collect();
    for record in by_id.values() {
        if refused.contains(record.id.as_bytes()) {
            continue;
        }
        match &record.event {
            ThreadEvent::Rejection(rejection) => {
                let Some((_, target)) = snapshot.get(rejection.proposal.as_bytes()) else {
                    refused.insert(*record.id.as_bytes());
                    continue;
                };
                if !authority.may_write(&target.object, &record.author) {
                    refused.insert(*record.id.as_bytes());
                    continue;
                }
                if let Some(entry) = proposals.get_mut(rejection.proposal.as_bytes()) {
                    entry
                        .rejected_by
                        .push((record.id, rejection.reason.clone()));
                }
            }
            ThreadEvent::Withdrawal(withdrawal) => {
                let Some((proposer, _)) = snapshot.get(withdrawal.proposal.as_bytes()) else {
                    refused.insert(*record.id.as_bytes());
                    continue;
                };
                if *proposer != record.author {
                    refused.insert(*record.id.as_bytes());
                    continue;
                }
                if let Some(entry) = proposals.get_mut(withdrawal.proposal.as_bytes()) {
                    entry.withdrawn_by.push(record.id);
                }
            }
            ThreadEvent::Proposal(proposal) => {
                let Some(earlier) = proposal.supersedes else {
                    continue;
                };
                let valid = earlier != record.id
                    && snapshot
                        .get(earlier.as_bytes())
                        .is_some_and(|(proposer, target)| {
                            *proposer == record.author
                                && target.object == proposal.object
                                && target.base == proposal.base
                        });
                if valid {
                    if let Some(entry) = proposals.get_mut(earlier.as_bytes()) {
                        entry.superseded_by.push(record.id);
                    }
                }
            }
            ThreadEvent::Resolution(_) => {}
        }
    }
    for edit in edits_by_id.values() {
        if let Some(entry) = proposals.get_mut(edit.proposal.as_bytes())
            && entry.proposal.object == edit.object
        {
            entry.accepted_by.push(edit.id);
        }
    }

    let proposals: Vec<ProposalRecord> = proposals
        .into_values()
        .map(|mut record| {
            record.accepted_by.sort_by_key(|id| *id.as_bytes());
            record.rejected_by.sort_by_key(|(id, _)| *id.as_bytes());
            record.withdrawn_by.sort_by_key(|id| *id.as_bytes());
            record.superseded_by.sort_by_key(|id| *id.as_bytes());
            record.state = if !record.accepted_by.is_empty() {
                ProposalState::Accepted
            } else if !record.rejected_by.is_empty() {
                ProposalState::Rejected
            } else if !record.withdrawn_by.is_empty() {
                ProposalState::Withdrawn
            } else if !record.superseded_by.is_empty() {
                ProposalState::Superseded
            } else {
                ProposalState::Proposed
            };
            record
        })
        .collect();

    let resolutions = resolutions
        .into_values()
        .map(|candidates| {
            let thread = candidates[0].resolution.thread;
            let mut heads: Vec<ResolutionRecord> = candidates
                .iter()
                .filter(|record| {
                    !candidates.iter().any(|other| {
                        other.id != record.id && follows(other.id, record.id, &parents)
                    })
                })
                .cloned()
                .collect();
            heads.sort_by_key(|record| *record.id.as_bytes());
            ThreadResolution { thread, heads }
        })
        .collect();

    ThreadProjection {
        resolutions,
        proposals,
        refused: refused.into_iter().map(Hash::from_bytes).collect(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn hash(seed: u8) -> Hash {
        Hash::digest([seed])
    }

    fn author(seed: u8) -> AuthorKey {
        [seed; 32]
    }

    fn object(seed: u8) -> ObjectRef {
        ObjectRef::new(hash(100 + seed), author(100 + seed))
    }

    fn blob(seed: u8) -> ChatLink {
        ChatLink {
            kind: ChatLinkKind::Blob,
            hash: hash(200 + seed),
            author: author(1),
            hint: None,
        }
    }

    /// Everyone may resolve and write, except keys listed as outsiders.
    struct Open {
        outsiders: Vec<AuthorKey>,
        writers: Option<Vec<AuthorKey>>,
    }

    impl ThreadAuthority for Open {
        fn may_resolve(&self, _thread: &Hash, author: &AuthorKey) -> bool {
            !self.outsiders.contains(author)
        }
        fn may_write(&self, _object: &ObjectRef, author: &AuthorKey) -> bool {
            !self.outsiders.contains(author)
                && self
                    .writers
                    .as_ref()
                    .is_none_or(|writers| writers.contains(author))
        }
    }

    fn open() -> Open {
        Open {
            outsiders: Vec::new(),
            writers: None,
        }
    }

    fn rec(id: u8, who: u8, after: &[u8], event: ThreadEvent) -> ThreadEventRecord {
        ThreadEventRecord {
            id: hash(id),
            author: author(who),
            after: after.iter().map(|seed| hash(*seed)).collect(),
            backlink: None,
            event,
        }
    }

    fn proposal(thread: u8, obj: u8, base: u8, supersedes: Option<u8>) -> ThreadEvent {
        ThreadEvent::Proposal(ChatProposal {
            object: object(obj),
            base: hash(base),
            content: blob(obj),
            thread: hash(thread),
            supersedes: supersedes.map(hash),
        })
    }

    #[test]
    fn kinds_are_recognised_and_checked_against_bodies() {
        assert!(is_thread_event_kind("chat.resolution.v1"));
        assert!(is_thread_event_kind("chat.proposal-withdrawal.v1"));
        assert!(!is_thread_event_kind("chat.message.v1"));
        let body = ThreadEvent::Resolution(ChatResolution::no_action(hash(1)));
        assert!(check_thread_event(CHAT_RESOLUTION_KIND, &body).is_ok());
        assert!(matches!(
            check_thread_event(CHAT_PROPOSAL_KIND, &body),
            Err(ThreadEventError::KindMismatch { .. })
        ));
    }

    #[test]
    fn resolution_fields_must_match_their_kind() {
        let mut edit = ChatResolution::edit(hash(1), object(1), hash(2));
        assert!(edit.check().is_ok());
        edit.summary = Some(blob(1));
        assert!(edit.check().is_err());
        let mut no_action = ChatResolution::no_action(hash(1));
        no_action.edit = Some(hash(2));
        assert!(no_action.check().is_err());
        let event_link = ChatLink {
            kind: ChatLinkKind::Event,
            ..blob(1)
        };
        assert!(ChatResolution::decision(hash(1), event_link).is_err());
        assert!(ChatResolution::decision(hash(1), blob(1)).is_ok());
    }

    #[test]
    fn bodies_round_trip_as_json_text() {
        // Terminal parity (§5 invariant 5): each act is one body a terminal
        // can author and read back as text.
        let bodies = [
            serde_json::to_string(&ChatResolution::edit(hash(1), object(1), hash(2))).unwrap(),
            serde_json::to_string(&ChatResolution::decision(hash(1), blob(1)).unwrap()).unwrap(),
        ];
        for text in bodies {
            let back: ChatResolution = serde_json::from_str(&text).unwrap();
            assert!(back.check().is_ok());
            assert_eq!(serde_json::to_string(&back).unwrap(), text);
        }
        let ThreadEvent::Proposal(body) = proposal(1, 1, 3, Some(9)) else {
            panic!("a proposal")
        };
        let text = serde_json::to_string(&body).unwrap();
        assert_eq!(serde_json::from_str::<ChatProposal>(&text).unwrap(), body);
        assert!(
            serde_json::from_str::<ChatProposalWithdrawal>(r#"{"proposal":"00","x":1}"#).is_err()
        );
    }

    #[test]
    fn a_later_resolution_re_resolves_the_thread() {
        let first = rec(
            1,
            1,
            &[],
            ThreadEvent::Resolution(ChatResolution::no_action(hash(50))),
        );
        let later = rec(
            2,
            2,
            &[1],
            ThreadEvent::Resolution(ChatResolution::edit(hash(50), object(1), hash(60))),
        );
        let projection = project_threads([first, later], [], &open());
        let entry = projection.resolution_of(&hash(50)).unwrap();
        assert!(!entry.is_concurrent());
        assert_eq!(entry.heads[0].id, hash(2));
        assert_eq!(
            projection
                .resolutions_producing(&object(1), &hash(60))
                .len(),
            1
        );
        assert!(
            projection
                .resolutions_producing(&object(1), &hash(61))
                .is_empty()
        );
    }

    #[test]
    fn concurrent_resolutions_are_kept_side_by_side() {
        let a = rec(
            1,
            1,
            &[],
            ThreadEvent::Resolution(ChatResolution::no_action(hash(50))),
        );
        let b = rec(
            2,
            2,
            &[],
            ThreadEvent::Resolution(ChatResolution::decision(hash(50), blob(2)).unwrap()),
        );
        let projection = project_threads([a, b], [], &open());
        let entry = projection.resolution_of(&hash(50)).unwrap();
        assert!(entry.is_concurrent());
        assert_eq!(entry.heads.len(), 2);
    }

    #[test]
    fn a_resolution_by_an_outsider_is_refused() {
        let event = rec(
            1,
            9,
            &[],
            ThreadEvent::Resolution(ChatResolution::no_action(hash(50))),
        );
        let authority = Open {
            outsiders: vec![author(9)],
            writers: None,
        };
        let projection = project_threads([event], [], &authority);
        assert!(projection.resolutions.is_empty());
        assert_eq!(projection.refused, vec![hash(1)]);
    }

    #[test]
    fn acceptance_is_the_citing_edit_itself() {
        let events = [rec(1, 1, &[], proposal(50, 1, 3, None))];
        let wrong_object = AcceptingEdit {
            id: hash(70),
            author: author(2),
            object: object(2),
            proposal: hash(1),
        };
        let projection = project_threads(events.clone(), [wrong_object], &open());
        assert_eq!(projection.proposals[0].state, ProposalState::Proposed);
        let edit = AcceptingEdit {
            id: hash(71),
            author: author(2),
            object: object(1),
            proposal: hash(1),
        };
        let projection = project_threads(events, [edit], &open());
        assert_eq!(projection.proposals[0].state, ProposalState::Accepted);
        assert_eq!(projection.proposals[0].accepted_by, vec![hash(71)]);
    }

    #[test]
    fn review_marks_an_open_proposal_stale_against_the_current_heads() {
        let events = [
            rec(1, 1, &[], proposal(50, 1, 3, None)),
            rec(2, 1, &[], proposal(51, 1, 4, None)),
            rec(3, 2, &[], proposal(52, 2, 3, None)),
        ];
        let accepted = AcceptingEdit {
            id: hash(70),
            author: author(2),
            object: object(1),
            proposal: hash(2),
        };
        let projection = project_threads(events, [accepted], &open());

        // Base 3 is still a head: the open proposal is current.
        let review = projection.review(&object(1), &[hash(3), hash(5)]);
        assert_eq!(review.len(), 2);
        let open_one = review.iter().find(|r| r.record.id == hash(1)).unwrap();
        assert_eq!(open_one.base, BaseStanding::Current);
        let closed = review.iter().find(|r| r.record.id == hash(2)).unwrap();
        assert_eq!(closed.base, BaseStanding::Closed);

        // The object moved on: the open proposal is stale and names the
        // heads, sorted and once each; the accepted one stays closed.
        let review = projection.review(&object(1), &[hash(6), hash(5), hash(6)]);
        let mut heads = vec![hash(5), hash(6)];
        heads.sort_by_key(|id| *id.as_bytes());
        let open_one = review.iter().find(|r| r.record.id == hash(1)).unwrap();
        assert_eq!(open_one.base, BaseStanding::Stale { heads });
        let closed = review.iter().find(|r| r.record.id == hash(2)).unwrap();
        assert_eq!(closed.base, BaseStanding::Closed);

        // Another object's proposals are not in this review.
        assert!(review.iter().all(|r| r.record.proposal.object == object(1)));
        assert_eq!(projection.review(&object(2), &[hash(3)]).len(), 1);
    }

    #[test]
    fn only_a_writer_rejects_and_only_the_proposer_withdraws() {
        let reject = |id, who| {
            rec(
                id,
                who,
                &[1],
                ThreadEvent::Rejection(ChatProposalRejection {
                    proposal: hash(1),
                    reason: blob(5),
                }),
            )
        };
        let withdraw = |id, who| {
            rec(
                id,
                who,
                &[1],
                ThreadEvent::Withdrawal(ChatProposalWithdrawal { proposal: hash(1) }),
            )
        };
        let authority = Open {
            outsiders: Vec::new(),
            writers: Some(vec![author(2)]),
        };
        let projection = project_threads(
            [
                rec(1, 1, &[], proposal(50, 1, 3, None)),
                reject(2, 3),
                withdraw(3, 4),
            ],
            [],
            &authority,
        );
        assert_eq!(projection.proposals[0].state, ProposalState::Proposed);
        assert_eq!(projection.refused.len(), 2);

        let projection = project_threads(
            [rec(1, 1, &[], proposal(50, 1, 3, None)), reject(2, 2)],
            [],
            &authority,
        );
        assert_eq!(projection.proposals[0].state, ProposalState::Rejected);
        let projection = project_threads(
            [rec(1, 1, &[], proposal(50, 1, 3, None)), withdraw(3, 1)],
            [],
            &authority,
        );
        assert_eq!(projection.proposals[0].state, ProposalState::Withdrawn);
    }

    #[test]
    fn supersession_needs_the_same_proposer_object_and_base() {
        let first = rec(1, 1, &[], proposal(50, 1, 3, None));
        let projection = project_threads(
            [first.clone(), rec(2, 1, &[1], proposal(50, 1, 3, Some(1)))],
            [],
            &open(),
        );
        assert_eq!(projection.proposals[0].state, ProposalState::Superseded);
        // Another proposer, or a newer base, is a new proposal, not a supersession.
        for other in [
            rec(2, 2, &[1], proposal(50, 1, 3, Some(1))),
            rec(2, 1, &[1], proposal(50, 1, 4, Some(1))),
        ] {
            let projection = project_threads([first.clone(), other], [], &open());
            assert_eq!(projection.proposals[0].state, ProposalState::Proposed);
        }
    }

    #[test]
    fn a_closing_event_for_an_unknown_proposal_is_refused() {
        let projection = project_threads(
            [rec(
                1,
                1,
                &[],
                ThreadEvent::Withdrawal(ChatProposalWithdrawal { proposal: hash(99) }),
            )],
            [],
            &open(),
        );
        assert_eq!(projection.refused, vec![hash(1)]);
    }

    #[test]
    fn staleness_is_an_observation_against_current_heads() {
        let projection = project_threads([rec(1, 1, &[], proposal(50, 1, 3, None))], [], &open());
        let record = &projection.proposals[0];
        assert!(!record.is_stale(&[hash(3)]));
        assert!(record.is_stale(&[hash(4)]));
        assert_eq!(record.state, ProposalState::Proposed);
    }

    #[test]
    fn the_view_drops_unreadable_objects_and_threads_whole() {
        let events = [
            rec(
                1,
                1,
                &[],
                ThreadEvent::Resolution(ChatResolution::edit(hash(50), object(1), hash(60))),
            ),
            rec(
                2,
                1,
                &[],
                ThreadEvent::Resolution(ChatResolution::no_action(hash(51))),
            ),
            rec(3, 1, &[], proposal(50, 1, 3, None)),
            rec(4, 1, &[], proposal(52, 2, 3, None)),
        ];
        let projection = project_threads(events, [], &open());
        let hidden_object = object(1);
        let view =
            projection.visible_to(&|thread| *thread != hash(52), &|obj| *obj != hidden_object);
        assert!(view.resolution_of(&hash(50)).is_none());
        assert!(view.resolution_of(&hash(51)).is_some());
        assert!(view.proposals.is_empty());
    }

    fn arb_event() -> impl Strategy<Value = ThreadEventRecord> {
        (
            0u8..12,
            1u8..4,
            prop::collection::vec(0u8..12, 0..3),
            0u8..6,
            0u8..4,
            0u8..3,
        )
            .prop_map(|(id, who, after, choice, thread, target)| {
                let event = match choice {
                    0 => ThreadEvent::Resolution(ChatResolution::no_action(hash(50 + thread))),
                    1 => ThreadEvent::Resolution(ChatResolution::edit(
                        hash(50 + thread),
                        object(target),
                        hash(60 + target),
                    )),
                    2 => proposal(50 + thread, target, 3, None),
                    3 => proposal(50 + thread, target, 3, Some(target)),
                    4 => ThreadEvent::Rejection(ChatProposalRejection {
                        proposal: hash(target),
                        reason: blob(target),
                    }),
                    _ => ThreadEvent::Withdrawal(ChatProposalWithdrawal {
                        proposal: hash(target),
                    }),
                };
                rec(id, who, &after, event)
            })
    }

    fn arb_edit() -> impl Strategy<Value = AcceptingEdit> {
        (20u8..24, 0u8..3, 0u8..12).prop_map(|(id, target, cited)| AcceptingEdit {
            id: hash(id),
            author: author(2),
            object: object(target),
            proposal: hash(cited),
        })
    }

    fn authority() -> Open {
        Open {
            outsiders: vec![author(3)],
            writers: Some(vec![author(1), author(2)]),
        }
    }

    proptest! {
        // §5 invariant 4: same event set, any order, any duplication, same fold.
        #[test]
        fn fold_is_independent_of_order_and_duplicates(
            events in prop::collection::vec(arb_event(), 0..14),
            edits in prop::collection::vec(arb_edit(), 0..4),
            seed in any::<u64>(),
        ) {
            // First delivery wins on a shared ID, so dedupe before shuffling.
            let mut unique: BTreeMap<[u8; 32], ThreadEventRecord> = BTreeMap::new();
            for event in events {
                unique.entry(*event.id.as_bytes()).or_insert(event);
            }
            let events: Vec<ThreadEventRecord> = unique.into_values().collect();
            let mut edit_set: BTreeMap<[u8; 32], AcceptingEdit> = BTreeMap::new();
            for edit in edits {
                edit_set.entry(*edit.id.as_bytes()).or_insert(edit);
            }
            let edits: Vec<AcceptingEdit> = edit_set.into_values().collect();
            let base = project_threads(events.clone(), edits.clone(), &authority());

            let mut shuffled = events.clone();
            shuffled.extend(events.iter().take(3).cloned());
            let mut state = seed;
            for index in (1..shuffled.len()).rev() {
                state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                let pick = usize::try_from(state >> 33).unwrap_or(0) % (index + 1);
                shuffled.swap(index, pick);
            }
            let mut edits_rev = edits.clone();
            edits_rev.reverse();
            edits_rev.extend(edits.iter().cloned());
            prop_assert_eq!(project_threads(shuffled, edits_rev, &authority()), base);
        }

        // §5 invariant 6: nothing a viewer cannot read survives the view.
        #[test]
        fn view_never_names_an_unreadable_thread_or_object(
            events in prop::collection::vec(arb_event(), 0..14),
            edits in prop::collection::vec(arb_edit(), 0..4),
            hidden_thread in 0u8..4,
            hidden_target in 0u8..3,
        ) {
            let projection = project_threads(events, edits, &authority());
            let hidden_thread = hash(50 + hidden_thread);
            let hidden_object = object(hidden_target);
            let view = projection.visible_to(&|thread| *thread != hidden_thread, &|obj| *obj != hidden_object);
            prop_assert!(view.refused.is_empty());
            for entry in &view.resolutions {
                prop_assert!(entry.thread != hidden_thread);
                prop_assert!(!entry.heads.is_empty());
                for head in &entry.heads {
                    prop_assert!(head.resolution.object != Some(hidden_object));
                }
            }
            let shown: HashSet<Hash> = view.proposals.iter().map(|record| record.id).collect();
            for record in &view.proposals {
                prop_assert!(record.proposal.thread != hidden_thread);
                prop_assert!(record.proposal.object != hidden_object);
                for id in &record.superseded_by {
                    prop_assert!(shown.contains(id));
                }
            }
        }

        // Concurrent heads: every head is causally maximal, and no resolution
        // that is maximal is dropped.
        #[test]
        fn heads_are_exactly_the_causally_maximal_resolutions(
            events in prop::collection::vec(arb_event(), 0..14),
        ) {
            let mut unique: BTreeMap<[u8; 32], ThreadEventRecord> = BTreeMap::new();
            for event in events {
                unique.entry(*event.id.as_bytes()).or_insert(event);
            }
            let events: Vec<ThreadEventRecord> = unique.into_values().collect();
            let parents: HashMap<Hash, Vec<Hash>> = events
                .iter()
                .map(|record| (record.id, record.after.clone()))
                .collect();
            let projection = project_threads(events.clone(), [], &authority());
            for entry in &projection.resolutions {
                let admitted: Vec<&ThreadEventRecord> = events
                    .iter()
                    .filter(|record| {
                        record.author != author(3)
                            && matches!(&record.event, ThreadEvent::Resolution(body) if body.thread == entry.thread)
                    })
                    .collect();
                for record in &admitted {
                    let maximal = !admitted
                        .iter()
                        .any(|other| other.id != record.id && follows(other.id, record.id, &parents));
                    let listed = entry.heads.iter().any(|head| head.id == record.id);
                    prop_assert_eq!(maximal, listed);
                }
            }
        }
    }
}
