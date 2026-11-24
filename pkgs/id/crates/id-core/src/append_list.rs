//! The server side of the append-list user-attribute protocol (#10982;
//! design #9127, `doc/2026-11-17_design_id9127_append_list_user_attributes.md`).
//!
//! An append-list attribute is an ordered list of submitted, signed
//! objects. The list changes only through the record's §3 ops, and
//! the server alone decides whether and where an entry lands:
//!
//! - [`AppendListServer::submit`] verifies, in the record's order, that
//!   the hash matches the object bytes, that the signature verifies
//!   against the submitter's key over the hash, that the attribute
//!   exists and is an append-list, and that it is OPENED. Only then is
//!   the entry appended (members) or queued (everyone else).
//! - The server assigns each appended entry the next central sequence
//!   number for that attribute: monotonic and gap-free, at append time,
//!   and for a pending entry at approval time (§4, §8.2).
//! - A new attribute is not OPENED (§5). A submission to an unopened or
//!   closed attribute is refused, not held.
//! - Pending submissions are verified before they are queued, carry no
//!   sequence, and are never returned by [`AppendListServer::read`]
//!   (§6, §7.5).
//! - Submission is idempotent per (attribute, hash, submitter) (§3); two
//!   submitters of the same object are two entries (§8.5).
//!
//! Every refusal is a [`Refusal`], whose text names the rule and carries
//! no submitted value.
//!
//! This module is the protocol's state and rules. The list holds
//! references (hash, sequence, submitter, signature); the object bytes
//! belong to the content plane and are only read here to check the hash.
//! Storage and the wire encoding are the callers' (§3 leaves the wire
//! encoding to the implementing packet).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use ed25519_dalek::{Signature, VerifyingKey};

/// A BLAKE3 content hash of an object's bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentHash(pub [u8; 32]);

impl ContentHash {
    /// The hash of `object`.
    #[must_use]
    pub fn of(object: &[u8]) -> Self {
        Self(*blake3::hash(object).as_bytes())
    }
}

/// A pending submission's identifier, unique on this server and
/// increasing in submission order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SubmissionId(pub u64);

/// One appended list entry: a reference to a signed object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The central sequence number: the entry's position in the list.
    pub seq: u64,
    /// The object's content hash.
    pub hash: ContentHash,
    /// The key that signed the hash.
    pub submitter: VerifyingKey,
    /// The submitter's signature over the hash.
    pub signature: Signature,
}

/// A verified submission waiting for an approver (§6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSubmission {
    /// The submission's identifier.
    pub id: SubmissionId,
    /// The object's content hash.
    pub hash: ContentHash,
    /// The key that signed the hash.
    pub submitter: VerifyingKey,
    /// The submitter's signature over the hash.
    pub signature: Signature,
}

/// What `submit` did with a verified submission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitOutcome {
    /// Appended at this sequence number.
    Appended(u64),
    /// Queued for approval.
    Pending(SubmissionId),
    /// A retry of a submission already appended at this sequence number.
    AlreadyAppended(u64),
    /// A retry of a submission already waiting for approval.
    AlreadyPending(SubmissionId),
}

/// The state of one submitter's submission of one object, as that
/// submitter may read it (§3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionState {
    /// Waiting for an approver.
    Pending(SubmissionId),
    /// Appended at this sequence number.
    Appended(u64),
    /// Rejected by an approver.
    Refused(SubmissionId),
}

/// Who besides the owner may act on an attribute. The owner is always a
/// member, an approver and an admin.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttributeConfig {
    /// Keys whose verified submissions append directly (§6).
    pub members: Vec<VerifyingKey>,
    /// Keys that may read the pending queue and approve or reject (§8.3).
    pub approvers: Vec<VerifyingKey>,
    /// Keys that may open and close the attribute (§5).
    pub admins: Vec<VerifyingKey>,
}

/// A refused protocol op. The text names the rule and never repeats a
/// submitted value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The hash does not match the object bytes.
    HashMismatch,
    /// The signature does not verify against the submitter's key over the hash.
    BadSignature,
    /// There is no attribute at that path.
    NoSuchAttribute,
    /// The attribute is not an append-list.
    NotAnAppendList,
    /// The attribute is not OPENED.
    NotOpened,
    /// The caller may not perform this op on this attribute.
    NotAuthorized,
    /// There is no pending submission with that identifier on this attribute.
    NoSuchSubmission,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::HashMismatch => "the hash does not match the object",
            Self::BadSignature => "the signature does not verify over the hash",
            Self::NoSuchAttribute => "no such attribute",
            Self::NotAnAppendList => "the attribute is not an append-list",
            Self::NotOpened => "the attribute is not open for submissions",
            Self::NotAuthorized => "not authorized for this attribute",
            Self::NoSuchSubmission => "no such pending submission",
        })
    }
}

impl std::error::Error for Refusal {}

/// Why an attribute could not be created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateError {
    /// An attribute already exists at that path.
    AlreadyExists,
}

impl fmt::Display for CreateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an attribute already exists at that path")
    }
}

impl std::error::Error for CreateError {}

type KeyBytes = [u8; 32];

#[derive(Debug, Clone)]
struct AppendList {
    owner: VerifyingKey,
    members: BTreeSet<KeyBytes>,
    approvers: BTreeSet<KeyBytes>,
    admins: BTreeSet<KeyBytes>,
    opened: bool,
    entries: Vec<Entry>,
    pending: BTreeMap<SubmissionId, PendingSubmission>,
    states: BTreeMap<(ContentHash, KeyBytes), SubmissionState>,
}

impl AppendList {
    fn new(owner: VerifyingKey, config: &AttributeConfig) -> Self {
        let keys = |list: &[VerifyingKey]| -> BTreeSet<KeyBytes> {
            list.iter()
                .map(VerifyingKey::to_bytes)
                .chain(std::iter::once(owner.to_bytes()))
                .collect()
        };
        Self {
            owner,
            members: keys(&config.members),
            approvers: keys(&config.approvers),
            admins: keys(&config.admins),
            opened: false,
            entries: Vec::new(),
            pending: BTreeMap::new(),
            states: BTreeMap::new(),
        }
    }

    fn append(&mut self, hash: ContentHash, submitter: VerifyingKey, signature: Signature) -> u64 {
        let seq = self.entries.len() as u64;
        self.entries.push(Entry {
            seq,
            hash,
            submitter,
            signature,
        });
        self.states
            .insert((hash, submitter.to_bytes()), SubmissionState::Appended(seq));
        seq
    }
}

#[derive(Debug, Clone)]
enum Slot {
    AppendList(Box<AppendList>),
    Other,
}

/// One server's append-list attributes, keyed by attribute path
/// (`geordi.png/comments`).
#[derive(Debug, Clone, Default)]
pub struct AppendListServer {
    attributes: BTreeMap<String, Slot>,
    next_submission: u64,
}

fn list<'a>(attributes: &'a BTreeMap<String, Slot>, path: &str) -> Result<&'a AppendList, Refusal> {
    match attributes.get(path) {
        None => Err(Refusal::NoSuchAttribute),
        Some(Slot::Other) => Err(Refusal::NotAnAppendList),
        Some(Slot::AppendList(list)) => Ok(list),
    }
}

fn list_mut<'a>(
    attributes: &'a mut BTreeMap<String, Slot>,
    path: &str,
) -> Result<&'a mut AppendList, Refusal> {
    match attributes.get_mut(path) {
        None => Err(Refusal::NoSuchAttribute),
        Some(Slot::Other) => Err(Refusal::NotAnAppendList),
        Some(Slot::AppendList(list)) => Ok(list),
    }
}

impl AppendListServer {
    /// An empty server.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an append-list attribute owned by `owner`. It starts not
    /// OPENED (§5).
    ///
    /// # Errors
    ///
    /// [`CreateError::AlreadyExists`] when the path is taken.
    pub fn create_append_list(
        &mut self,
        path: &str,
        owner: VerifyingKey,
        config: &AttributeConfig,
    ) -> Result<(), CreateError> {
        if self.attributes.contains_key(path) {
            return Err(CreateError::AlreadyExists);
        }
        self.attributes.insert(
            path.to_owned(),
            Slot::AppendList(Box::new(AppendList::new(owner, config))),
        );
        Ok(())
    }

    /// Records that an attribute of another type exists at `path`, so a
    /// submission to it is refused as not an append-list.
    ///
    /// # Errors
    ///
    /// [`CreateError::AlreadyExists`] when the path is taken.
    pub fn declare_other_attribute(&mut self, path: &str) -> Result<(), CreateError> {
        if self.attributes.contains_key(path) {
            return Err(CreateError::AlreadyExists);
        }
        self.attributes.insert(path.to_owned(), Slot::Other);
        Ok(())
    }

    /// `append-submit` (§3): verifies, then appends or queues.
    ///
    /// The checks run in the record's order: hash, signature, attribute,
    /// OPENED. A retry of a submission already appended or pending
    /// returns its existing state and changes nothing, whether or not the
    /// attribute is open now.
    ///
    /// # Errors
    ///
    /// The first [`Refusal`] in that order. Nothing is stored on any
    /// refusal.
    pub fn submit(
        &mut self,
        attribute: &str,
        object: &[u8],
        hash: &ContentHash,
        submitter: &VerifyingKey,
        signature: &Signature,
    ) -> Result<SubmitOutcome, Refusal> {
        if ContentHash::of(object) != *hash {
            return Err(Refusal::HashMismatch);
        }
        submitter
            .verify_strict(&hash.0, signature)
            .map_err(|_err| Refusal::BadSignature)?;
        let list = list_mut(&mut self.attributes, attribute)?;
        match list.states.get(&(*hash, submitter.to_bytes())) {
            Some(SubmissionState::Appended(seq)) => {
                return Ok(SubmitOutcome::AlreadyAppended(*seq));
            }
            Some(SubmissionState::Pending(id)) => return Ok(SubmitOutcome::AlreadyPending(*id)),
            Some(SubmissionState::Refused(_)) | None => {}
        }
        if !list.opened {
            return Err(Refusal::NotOpened);
        }
        if list.members.contains(&submitter.to_bytes()) {
            return Ok(SubmitOutcome::Appended(
                list.append(*hash, *submitter, *signature),
            ));
        }
        let id = SubmissionId(self.next_submission);
        self.next_submission += 1;
        list.pending.insert(
            id,
            PendingSubmission {
                id,
                hash: *hash,
                submitter: *submitter,
                signature: *signature,
            },
        );
        list.states
            .insert((*hash, submitter.to_bytes()), SubmissionState::Pending(id));
        Ok(SubmitOutcome::Pending(id))
    }

    /// `append-read` (§3): entries in server order with a sequence above
    /// `after`, at most `limit` of them. Pending submissions are never
    /// returned.
    ///
    /// # Errors
    ///
    /// [`Refusal::NoSuchAttribute`] or [`Refusal::NotAnAppendList`].
    pub fn read(
        &self,
        attribute: &str,
        after: Option<u64>,
        limit: Option<usize>,
    ) -> Result<&[Entry], Refusal> {
        let entries = &list(&self.attributes, attribute)?.entries;
        let start = after.map_or(0, |seq| {
            usize::try_from(seq).map_or(entries.len(), |seq| {
                seq.saturating_add(1).min(entries.len())
            })
        });
        let end = limit.map_or(entries.len(), |limit| {
            start.saturating_add(limit).min(entries.len())
        });
        Ok(&entries[start..end])
    }

    /// `append-pending-read` (§3): the pending queue in submission order,
    /// for the attribute's approvers.
    ///
    /// # Errors
    ///
    /// [`Refusal::NotAuthorized`] when `reader` is not an approver, or
    /// the attribute refusals.
    pub fn pending(
        &self,
        attribute: &str,
        reader: &VerifyingKey,
    ) -> Result<Vec<&PendingSubmission>, Refusal> {
        let list = list(&self.attributes, attribute)?;
        if !list.approvers.contains(&reader.to_bytes()) {
            return Err(Refusal::NotAuthorized);
        }
        Ok(list.pending.values().collect())
    }

    /// A submitter's own state for one object: pending, appended at a
    /// sequence, refused, or `None` when they never submitted it (§3).
    ///
    /// # Errors
    ///
    /// The attribute refusals.
    pub fn own_state(
        &self,
        attribute: &str,
        submitter: &VerifyingKey,
        hash: &ContentHash,
    ) -> Result<Option<SubmissionState>, Refusal> {
        Ok(list(&self.attributes, attribute)?
            .states
            .get(&(*hash, submitter.to_bytes()))
            .copied())
    }

    /// `append-approve` (§6): appends a pending submission, which takes
    /// the next sequence number now. A closed attribute takes no
    /// approvals (§5, §7.4).
    ///
    /// # Errors
    ///
    /// [`Refusal::NotAuthorized`], [`Refusal::NotOpened`],
    /// [`Refusal::NoSuchSubmission`], or the attribute refusals.
    pub fn approve(
        &mut self,
        attribute: &str,
        approver: &VerifyingKey,
        submission: SubmissionId,
    ) -> Result<u64, Refusal> {
        let list = list_mut(&mut self.attributes, attribute)?;
        if !list.approvers.contains(&approver.to_bytes()) {
            return Err(Refusal::NotAuthorized);
        }
        if !list.opened {
            return Err(Refusal::NotOpened);
        }
        let pending = list
            .pending
            .remove(&submission)
            .ok_or(Refusal::NoSuchSubmission)?;
        Ok(list.append(pending.hash, pending.submitter, pending.signature))
    }

    /// `append-reject` (§6): drops a pending submission and records the
    /// refusal against it. Allowed while closed, so an owner can empty
    /// the queue (§8.4).
    ///
    /// # Errors
    ///
    /// [`Refusal::NotAuthorized`], [`Refusal::NoSuchSubmission`], or the
    /// attribute refusals.
    pub fn reject(
        &mut self,
        attribute: &str,
        approver: &VerifyingKey,
        submission: SubmissionId,
    ) -> Result<(), Refusal> {
        let list = list_mut(&mut self.attributes, attribute)?;
        if !list.approvers.contains(&approver.to_bytes()) {
            return Err(Refusal::NotAuthorized);
        }
        let pending = list
            .pending
            .remove(&submission)
            .ok_or(Refusal::NoSuchSubmission)?;
        list.states.insert(
            (pending.hash, pending.submitter.to_bytes()),
            SubmissionState::Refused(submission),
        );
        Ok(())
    }

    /// `append-open` (§5), by the owner or an admin.
    ///
    /// # Errors
    ///
    /// [`Refusal::NotAuthorized`] or the attribute refusals.
    pub fn open(&mut self, attribute: &str, actor: &VerifyingKey) -> Result<(), Refusal> {
        self.set_opened(attribute, actor, true)
    }

    /// `append-close` (§5), by the owner or an admin. Appended entries
    /// stay readable and the pending queue is kept (§8.4).
    ///
    /// # Errors
    ///
    /// [`Refusal::NotAuthorized`] or the attribute refusals.
    pub fn close(&mut self, attribute: &str, actor: &VerifyingKey) -> Result<(), Refusal> {
        self.set_opened(attribute, actor, false)
    }

    fn set_opened(
        &mut self,
        attribute: &str,
        actor: &VerifyingKey,
        opened: bool,
    ) -> Result<(), Refusal> {
        let list = list_mut(&mut self.attributes, attribute)?;
        if !list.admins.contains(&actor.to_bytes()) {
            return Err(Refusal::NotAuthorized);
        }
        list.opened = opened;
        Ok(())
    }

    /// Whether the attribute is OPENED.
    ///
    /// # Errors
    ///
    /// The attribute refusals.
    pub fn is_opened(&self, attribute: &str) -> Result<bool, Refusal> {
        Ok(list(&self.attributes, attribute)?.opened)
    }

    /// The attribute's owner.
    ///
    /// # Errors
    ///
    /// The attribute refusals.
    pub fn owner(&self, attribute: &str) -> Result<VerifyingKey, Refusal> {
        Ok(list(&self.attributes, attribute)?.owner)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use ed25519_dalek::{Signer as _, SigningKey};

    use super::*;

    const ATTR: &str = "geordi.png/comments";

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn signed(signer: &SigningKey, object: &[u8]) -> (ContentHash, Signature) {
        let hash = ContentHash::of(object);
        (hash, signer.sign(&hash.0))
    }

    fn submit(
        server: &mut AppendListServer,
        signer: &SigningKey,
        object: &[u8],
    ) -> Result<SubmitOutcome, Refusal> {
        let (hash, signature) = signed(signer, object);
        server.submit(ATTR, object, &hash, &signer.verifying_key(), &signature)
    }

    /// Owner 1, member 2; outsiders are any other seed.
    fn opened_server() -> AppendListServer {
        let mut server = AppendListServer::new();
        server
            .create_append_list(
                ATTR,
                key(1).verifying_key(),
                &AttributeConfig {
                    members: vec![key(2).verifying_key()],
                    ..AttributeConfig::default()
                },
            )
            .unwrap();
        server.open(ATTR, &key(1).verifying_key()).unwrap();
        server
    }

    #[test]
    fn verification_runs_in_the_record_order_and_stores_nothing_on_refusal() {
        let mut server = AppendListServer::new();
        let signer = key(2);
        let (hash, signature) = signed(&signer, b"hello");
        let wrong = key(3).sign(&hash.0);
        let public = signer.verifying_key();

        // A bad hash is refused before the signature or the attribute is looked at.
        assert_eq!(
            server.submit(ATTR, b"other", &hash, &public, &wrong),
            Err(Refusal::HashMismatch)
        );
        // Then the signature, before the attribute.
        assert_eq!(
            server.submit(ATTR, b"hello", &hash, &public, &wrong),
            Err(Refusal::BadSignature)
        );
        assert_eq!(
            server.submit(ATTR, b"hello", &hash, &public, &signature),
            Err(Refusal::NoSuchAttribute)
        );
        server.declare_other_attribute("geordi.png/title").unwrap();
        assert_eq!(
            server.submit("geordi.png/title", b"hello", &hash, &public, &signature),
            Err(Refusal::NotAnAppendList)
        );

        server
            .create_append_list(ATTR, key(1).verifying_key(), &AttributeConfig::default())
            .unwrap();
        assert_eq!(
            server.submit(ATTR, b"hello", &hash, &public, &signature),
            Err(Refusal::NotOpened)
        );
        assert_eq!(server.read(ATTR, None, None).unwrap(), []);
        assert_eq!(
            server.pending(ATTR, &key(1).verifying_key()).unwrap(),
            [] as [&crate::append_list::PendingSubmission; 0]
        );
        assert_eq!(server.own_state(ATTR, &public, &hash), Ok(None));
    }

    #[test]
    fn a_new_attribute_is_not_opened_and_opening_releases_no_backlog() {
        let mut server = AppendListServer::new();
        server
            .create_append_list(ATTR, key(1).verifying_key(), &AttributeConfig::default())
            .unwrap();
        assert_eq!(server.is_opened(ATTR), Ok(false));
        assert_eq!(
            submit(&mut server, &key(5), b"early"),
            Err(Refusal::NotOpened)
        );
        server.open(ATTR, &key(1).verifying_key()).unwrap();
        assert_eq!(
            server.pending(ATTR, &key(1).verifying_key()).unwrap(),
            [] as [&crate::append_list::PendingSubmission; 0]
        );
        assert_eq!(server.read(ATTR, None, None).unwrap(), []);
    }

    #[test]
    fn members_append_gap_free_and_approval_takes_the_sequence_at_approval_time() {
        let mut server = opened_server();
        let owner = key(1).verifying_key();
        assert_eq!(
            submit(&mut server, &key(1), b"a"),
            Ok(SubmitOutcome::Appended(0))
        );
        let Ok(SubmitOutcome::Pending(id)) = submit(&mut server, &key(5), b"from outside") else {
            panic!("an outsider's verified submission is queued");
        };
        // A member submitting later still lands before the pending entry.
        assert_eq!(
            submit(&mut server, &key(2), b"b"),
            Ok(SubmitOutcome::Appended(1))
        );
        assert_eq!(server.approve(ATTR, &owner, id), Ok(2));
        let seqs: Vec<u64> = server
            .read(ATTR, None, None)
            .unwrap()
            .iter()
            .map(|e| e.seq)
            .collect();
        assert_eq!(seqs, vec![0, 1, 2]);
        assert_eq!(
            server.own_state(
                ATTR,
                &key(5).verifying_key(),
                &ContentHash::of(b"from outside")
            ),
            Ok(Some(SubmissionState::Appended(2)))
        );
    }

    #[test]
    fn pending_is_never_read_as_a_list_entry_and_only_approvers_see_the_queue() {
        let mut server = opened_server();
        submit(&mut server, &key(5), b"pending").unwrap();
        assert_eq!(server.read(ATTR, None, None).unwrap(), []);
        assert_eq!(
            server.pending(ATTR, &key(1).verifying_key()).unwrap().len(),
            1
        );
        assert_eq!(
            server.pending(ATTR, &key(2).verifying_key()),
            Err(Refusal::NotAuthorized)
        );
        assert_eq!(
            server.pending(ATTR, &key(5).verifying_key()),
            Err(Refusal::NotAuthorized)
        );
        // The submitter can still read their own state.
        assert!(matches!(
            server.own_state(ATTR, &key(5).verifying_key(), &ContentHash::of(b"pending")),
            Ok(Some(SubmissionState::Pending(_)))
        ));
    }

    #[test]
    fn retries_are_idempotent_and_two_submitters_are_two_entries() {
        let mut server = opened_server();
        assert_eq!(
            submit(&mut server, &key(2), b"same"),
            Ok(SubmitOutcome::Appended(0))
        );
        assert_eq!(
            submit(&mut server, &key(2), b"same"),
            Ok(SubmitOutcome::AlreadyAppended(0))
        );
        assert_eq!(
            submit(&mut server, &key(1), b"same"),
            Ok(SubmitOutcome::Appended(1))
        );
        let Ok(SubmitOutcome::Pending(id)) = submit(&mut server, &key(5), b"same") else {
            panic!("queued");
        };
        assert_eq!(
            submit(&mut server, &key(5), b"same"),
            Ok(SubmitOutcome::AlreadyPending(id))
        );
        assert_eq!(server.read(ATTR, None, None).unwrap().len(), 2);
        assert_eq!(
            server.pending(ATTR, &key(1).verifying_key()).unwrap().len(),
            1
        );
        // A retry after close reports the existing state and stores nothing.
        server.close(ATTR, &key(1).verifying_key()).unwrap();
        assert_eq!(
            submit(&mut server, &key(2), b"same"),
            Ok(SubmitOutcome::AlreadyAppended(0))
        );
        assert_eq!(
            submit(&mut server, &key(2), b"new"),
            Err(Refusal::NotOpened)
        );
    }

    #[test]
    fn a_rejection_is_recorded_for_the_submitter() {
        let mut server = opened_server();
        let owner = key(1).verifying_key();
        let Ok(SubmitOutcome::Pending(id)) = submit(&mut server, &key(5), b"spam") else {
            panic!("queued");
        };
        assert_eq!(
            server.reject(ATTR, &key(5).verifying_key(), id),
            Err(Refusal::NotAuthorized)
        );
        server.reject(ATTR, &owner, id).unwrap();
        assert_eq!(
            server.own_state(ATTR, &key(5).verifying_key(), &ContentHash::of(b"spam")),
            Ok(Some(SubmissionState::Refused(id)))
        );
        assert_eq!(
            server.approve(ATTR, &owner, id),
            Err(Refusal::NoSuchSubmission)
        );
        assert_eq!(server.read(ATTR, None, None).unwrap(), []);
    }

    #[test]
    fn close_stops_submissions_and_approvals_but_keeps_the_queue() {
        let mut server = opened_server();
        let owner = key(1).verifying_key();
        submit(&mut server, &key(2), b"kept").unwrap();
        let Ok(SubmitOutcome::Pending(first)) = submit(&mut server, &key(5), b"one") else {
            panic!("queued");
        };
        let Ok(SubmitOutcome::Pending(second)) = submit(&mut server, &key(6), b"two") else {
            panic!("queued");
        };
        server.close(ATTR, &owner).unwrap();
        assert_eq!(
            submit(&mut server, &key(2), b"late"),
            Err(Refusal::NotOpened)
        );
        assert_eq!(server.approve(ATTR, &owner, first), Err(Refusal::NotOpened));
        assert_eq!(server.read(ATTR, None, None).unwrap().len(), 1);
        // Rejection is still allowed, so an owner can empty the queue.
        server.reject(ATTR, &owner, second).unwrap();
        server.open(ATTR, &owner).unwrap();
        assert_eq!(server.approve(ATTR, &owner, first), Ok(1));
    }

    #[test]
    fn only_the_owner_and_admins_open_and_close_and_configured_approvers_approve() {
        let mut server = AppendListServer::new();
        let owner = key(1).verifying_key();
        let admin = key(3).verifying_key();
        let approver = key(4).verifying_key();
        server
            .create_append_list(
                ATTR,
                owner,
                &AttributeConfig {
                    approvers: vec![approver],
                    admins: vec![admin],
                    ..AttributeConfig::default()
                },
            )
            .unwrap();
        assert_eq!(server.open(ATTR, &approver), Err(Refusal::NotAuthorized));
        server.open(ATTR, &admin).unwrap();
        let Ok(SubmitOutcome::Pending(id)) = submit(&mut server, &key(5), b"x") else {
            panic!("queued");
        };
        assert_eq!(
            server.approve(ATTR, &admin, id),
            Err(Refusal::NotAuthorized)
        );
        assert_eq!(server.approve(ATTR, &approver, id), Ok(0));
        assert_eq!(server.owner(ATTR), Ok(owner));
        assert_eq!(
            server.create_append_list(ATTR, owner, &AttributeConfig::default()),
            Err(CreateError::AlreadyExists)
        );
    }

    #[test]
    fn reads_page_by_sequence() {
        let mut server = opened_server();
        for object in [b"0", b"1", b"2", b"3"] {
            submit(&mut server, &key(2), object).unwrap();
        }
        let seqs = |after, limit| -> Vec<u64> {
            server
                .read(ATTR, after, limit)
                .unwrap()
                .iter()
                .map(|e| e.seq)
                .collect()
        };
        assert_eq!(seqs(None, Some(2)), vec![0, 1]);
        assert_eq!(seqs(Some(1), None), vec![2, 3]);
        assert_eq!(seqs(Some(1), Some(1)), vec![2]);
        assert_eq!(seqs(Some(3), None), [] as [u64; 0]);
        assert_eq!(seqs(Some(u64::MAX), None), [] as [u64; 0]);
    }

    #[test]
    fn refusals_carry_no_submitted_value() {
        let all = [
            Refusal::HashMismatch,
            Refusal::BadSignature,
            Refusal::NoSuchAttribute,
            Refusal::NotAnAppendList,
            Refusal::NotOpened,
            Refusal::NotAuthorized,
            Refusal::NoSuchSubmission,
        ];
        for refusal in all {
            let text = refusal.to_string();
            assert!(!text.contains(ATTR) && !text.contains("geordi"), "{text}");
        }
    }

    mod properties {
        use proptest::prelude::*;

        use super::*;

        /// Seeds: 1 owner, 2 member, 5 and 6 outsiders.
        const SEEDS: [u8; 4] = [1, 2, 5, 6];
        const OBJECTS: [&[u8]; 3] = [b"alpha", b"beta", b"gamma"];

        #[derive(Debug, Clone, Copy)]
        enum Flaw {
            None,
            Hash,
            Signature,
        }

        #[derive(Debug, Clone, Copy)]
        enum Op {
            Submit {
                who: usize,
                object: usize,
                flaw: Flaw,
            },
            Approve {
                who: usize,
                pick: usize,
            },
            Reject {
                who: usize,
                pick: usize,
            },
            Open {
                who: usize,
            },
            Close {
                who: usize,
            },
        }

        fn op() -> impl Strategy<Value = Op> {
            let who = 0..SEEDS.len();
            let flaw = prop_oneof![
                6 => Just(Flaw::None),
                1 => Just(Flaw::Hash),
                1 => Just(Flaw::Signature),
            ];
            prop_oneof![
                6 => (who.clone(), 0..OBJECTS.len(), flaw)
                    .prop_map(|(who, object, flaw)| Op::Submit { who, object, flaw }),
                3 => (who.clone(), 0..4usize).prop_map(|(who, pick)| Op::Approve { who, pick }),
                1 => (who.clone(), 0..4usize).prop_map(|(who, pick)| Op::Reject { who, pick }),
                1 => who.clone().prop_map(|who| Op::Open { who }),
                1 => who.prop_map(|who| Op::Close { who }),
            ]
        }

        fn snapshot(server: &AppendListServer) -> (Vec<Entry>, Vec<PendingSubmission>, bool) {
            let list = list(&server.attributes, ATTR).unwrap();
            (
                list.entries.clone(),
                list.pending.values().cloned().collect(),
                list.opened,
            )
        }

        /// Keys and signatures computed once: ed25519 in a debug build is
        /// slow enough that signing on every generated op dominates the run.
        struct Table {
            public: Vec<VerifyingKey>,
            /// `signatures[who][object]`.
            signatures: Vec<Vec<Signature>>,
            /// `forged[object]`: a signature over the object's hash by a key
            /// that is not the submitter's.
            forged: Vec<Signature>,
        }

        fn table() -> &'static Table {
            static TABLE: std::sync::OnceLock<Table> = std::sync::OnceLock::new();
            TABLE.get_or_init(|| {
                let signers: Vec<SigningKey> = SEEDS.iter().map(|seed| key(*seed)).collect();
                Table {
                    public: signers.iter().map(SigningKey::verifying_key).collect(),
                    signatures: signers
                        .iter()
                        .map(|signer| {
                            OBJECTS
                                .iter()
                                .map(|object| signed(signer, object).1)
                                .collect()
                        })
                        .collect(),
                    forged: OBJECTS
                        .iter()
                        .map(|object| signed(&key(9), object).1)
                        .collect(),
                }
            })
        }

        fn apply(server: &mut AppendListServer, op: Op) -> (bool, Option<u64>) {
            // Returns (refused, appended sequence).
            let table = table();
            match op {
                Op::Submit { who, object, flaw } => {
                    let bytes = OBJECTS[object];
                    let (mut hash, mut signature) =
                        (ContentHash::of(bytes), table.signatures[who][object]);
                    match flaw {
                        Flaw::None => {}
                        Flaw::Hash => hash = ContentHash::of(b"not one of the objects"),
                        Flaw::Signature => signature = table.forged[object],
                    }
                    match server.submit(ATTR, bytes, &hash, &table.public[who], &signature) {
                        Ok(SubmitOutcome::Appended(seq)) => (false, Some(seq)),
                        Ok(_) => (false, None),
                        Err(_) => (true, None),
                    }
                }
                Op::Approve { who, pick } | Op::Reject { who, pick } => {
                    let actor = table.public[who];
                    let ids: Vec<SubmissionId> = list(&server.attributes, ATTR)
                        .unwrap()
                        .pending
                        .keys()
                        .copied()
                        .collect();
                    let id = if ids.is_empty() {
                        SubmissionId(u64::MAX)
                    } else {
                        ids[pick % ids.len()]
                    };
                    if matches!(op, Op::Approve { .. }) {
                        server
                            .approve(ATTR, &actor, id)
                            .map_or((true, None), |seq| (false, Some(seq)))
                    } else {
                        (server.reject(ATTR, &actor, id).is_err(), None)
                    }
                }
                Op::Open { who } => (server.open(ATTR, &table.public[who]).is_err(), None),
                Op::Close { who } => (server.close(ATTR, &table.public[who]).is_err(), None),
            }
        }

        proptest! {
            // 64 cases of up to 60 ops each. Every submission runs a strict
            // ed25519 verify, which is slow at opt-level 0; the nix check
            // profile builds tests optimised, where this is quick.
            #![proptest_config(ProptestConfig::with_cases(64))]

            /// Oracle: design §7. No op sequence produces a list state the
            /// invariants forbid: nothing unverified is stored, the order is
            /// the gap-free server sequence and existing entries never move,
            /// the list grows only by one append or one approval at a time,
            /// closed (or never opened) means no submission and no approval
            /// lands, and no pending submission is ever readable as an entry.
            #[test]
            fn no_op_sequence_breaks_the_invariants(ops in proptest::collection::vec(op(), 1..60)) {
                let known: BTreeSet<ContentHash> = OBJECTS.iter().map(|o| ContentHash::of(o)).collect();
                let mut server = AppendListServer::new();
                server
                    .create_append_list(
                        ATTR,
                        key(1).verifying_key(),
                        &AttributeConfig { members: vec![key(2).verifying_key()], ..AttributeConfig::default() },
                    )
                    .unwrap();
                for op in ops {
                    let (before_entries, before_pending, before_opened) = snapshot(&server);
                    let (refused, appended) = apply(&mut server, op);
                    let (entries, pending, _) = snapshot(&server);

                    // §7.1: every stored entry and pending submission verifies.
                    // Entries and pending submissions already checked after an
                    // earlier op are unchanged (the prefix check below), so only
                    // the new ones need verifying.
                    for (hash, submitter, signature) in entries
                        .iter()
                        .skip(before_entries.len())
                        .map(|e| (e.hash, e.submitter, e.signature))
                        .chain(
                            pending
                                .iter()
                                .filter(|p| !before_pending.contains(p))
                                .map(|p| (p.hash, p.submitter, p.signature)),
                        )
                    {
                        prop_assert!(known.contains(&hash));
                        prop_assert!(submitter.verify_strict(&hash.0, &signature).is_ok());
                    }
                    // §7.2 and §7.3: the old list is a prefix, the sequence is the
                    // position, and it grows by one only through an append.
                    prop_assert_eq!(&entries[..before_entries.len()], &*before_entries);
                    for (index, entry) in entries.iter().enumerate() {
                        prop_assert_eq!(entry.seq, index as u64);
                    }
                    let grew = entries.len() - before_entries.len();
                    prop_assert!(grew <= 1);
                    prop_assert_eq!(grew == 1, appended.is_some());
                    if let Some(seq) = appended {
                        prop_assert_eq!(seq, before_entries.len() as u64);
                    }
                    // A flawed submission is refused and stores nothing.
                    if let Op::Submit { flaw: Flaw::Hash | Flaw::Signature, .. } = op {
                        prop_assert!(refused);
                        prop_assert_eq!(&pending, &before_pending);
                    }
                    // §7.4: closed means closed.
                    if !before_opened {
                        prop_assert_eq!(grew, 0);
                        prop_assert!(pending.iter().all(|p| before_pending.contains(p)));
                    }
                    // Only approvers move the queue into the list.
                    if let Op::Approve { who, .. } = op
                        && SEEDS[who] != 1
                    {
                        prop_assert!(refused);
                    }
                    // §7.5: reads return exactly the entries, never a pending one.
                    let read = server.read(ATTR, None, None).unwrap();
                    prop_assert_eq!(read, &*entries);
                    for p in &pending {
                        prop_assert!(!read.iter().any(|e| e.hash == p.hash && e.submitter == p.submitter));
                    }
                }
            }
        }
    }
}
