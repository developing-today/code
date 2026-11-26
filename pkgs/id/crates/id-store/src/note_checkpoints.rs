//! Note checkpoints for collaborative editing (#3819, Phase 3 of the #2048
//! Zettelkasten design): the checkpoint core that ties a live editing
//! session to the durable note graph [`crate::notes`] defines.
//!
//! The split this module enforces is the phase's first rule: editor steps
//! stay ephemeral (they live in the session, never in the graph), and what
//! crosses into the graph is a *checkpoint* — one signed `note.revise`
//! event (a [`NoteOp::Revise`]) naming the note's stable ID, the verified
//! blob content the editor had reached, and the parent revision(s) it
//! builds on. A checkpoint is published only at an explicit point (save,
//! blur, session end — the session owner's call), never per keystroke.
//!
//! Concurrency is the graph's, unchanged: two sessions that checkpoint
//! from the same head produce two heads, and nothing here picks a winner.
//! [`NoteCheckpoints::checkpoint_on_heads`] refuses a multi-head note —
//! joining heads is a decision, and it is made only through the explicit
//! [`NoteCheckpoints::merge`]. A session working from a stale head names
//! that head as its parent and forks, exactly as an offline edit should.
//!
//! The log is reconstructible: [`NoteCheckpoints::from_events`] rebuilds
//! it from the applied note events alone (a restart, a sync from a peer),
//! and the fold is order-independent, so the rebuilt heads are the heads
//! the live log had. Policies are the one thing the events do not carry:
//! the host persists the [`NoteCheckpoints::policies`] snapshot
//! alongside the events and re-states it with
//! [`NoteCheckpoints::seed_policies`], or a rebuilt log treats every
//! note as public. Persistence and transport of the events are the
//! event store's business, as for every other kind; this core owns the
//! checkpoint rules and nothing else.

use std::collections::HashMap;
use std::sync::Arc;

use p2panda_core::Hash;
use serde::{Deserialize, Serialize};

use id_core::waiting::AuthorKey;

use crate::documents::DocumentRef;
use crate::notes::{
    AppliedNote, CREATE_KIND, NoteError, NoteFoldError, NoteOp, NoteRecord, REVISE_KIND,
    check_note_op, fold_notes,
};

/// Why a checkpoint operation was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CheckpointError {
    /// The operation breaks the note rules ([`check_note_op`]).
    #[error(transparent)]
    Note(#[from] NoteError),
    /// The resulting event set does not fold ([`fold_notes`]).
    #[error(transparent)]
    Fold(#[from] NoteFoldError),
    /// The named note has no create event in the log.
    #[error("note {0} has no create event in the checkpoint log")]
    UnknownNote(Hash),
    /// A follow-on checkpoint was asked to build on the current heads
    /// of a note that has more than one: joining heads is an explicit
    /// merge, never a side effect of saving.
    #[error("note {note} has heads {heads:?}; a merge must name them explicitly")]
    ExplicitMergeRequired {
        /// The note whose heads would have been joined silently.
        note: Hash,
        /// The current heads, in ID order.
        heads: Vec<Hash>,
    },
    /// The author holds no grant that lets them do this with the
    /// note (#2048 Q2: appends are by write grant, attributed; policy
    /// changes are by admin grant or the note's owner).
    #[error("author {author:?} holds no grant permitting this on note {note}")]
    NotAuthorized {
        /// The note the author was refused on.
        note: Hash,
        /// The key that signed (or tried to sign) the refused event.
        author: AuthorKey,
    },
    /// A merge was asked for on a note with fewer than two heads:
    /// there is nothing concurrent to join.
    #[error("note {note} has heads {heads:?}; a merge joins at least two")]
    NotConcurrent {
        /// The note the merge was asked for.
        note: Hash,
        /// The current heads, in ID order.
        heads: Vec<Hash>,
    },
}

/// What a principal may do with a note, least to most (#2048 Q3's
/// own ladder for notes: read, write, admin). This is the note
/// layer's vocabulary, deliberately separate from the group layer's
/// wider `Role`: a group resolver maps its roles onto these three
/// (Read -> Read, Write/Manage -> Write, Admin -> Admin) when it
/// answers for a group grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum NoteRole {
    /// May read the note's content.
    Read,
    /// May append revisions to the note.
    Write,
    /// May append, and may change the note's access policy.
    Admin,
}

/// Who a grant names: one account key, or a group (whose members'
/// roles a [`GroupRoles`] resolver answers for).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotePrincipal {
    /// A single account, by its key.
    User(AuthorKey),
    /// A group, by its ID.
    Group(Hash),
}

/// One access grant on a note: a principal and what it may do.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteGrant {
    /// Who the grant names.
    pub principal: NotePrincipal,
    /// What the named principal may do.
    pub role: NoteRole,
}

/// A note's access policy (#2048 Q3, ledger shared-notes-read-3819:
/// notes are public by default, or read/write/admin to one or more
/// groups and users).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoteAccess {
    /// Public by default: anyone may read the note, and anyone may
    /// append a revision — appends are signed, so a public append
    /// is still an attributed revision, never a silent overwrite
    /// (#2048 Q2).
    #[default]
    Public,
    /// Scoped: only the listed grants (and the note's owner, who is
    /// always Admin) hold any access. An author with no grant at or
    /// above the needed role is refused.
    Scoped(Vec<NoteGrant>),
}

/// One note's recorded access policy: the durable form of a policy.
/// The event log carries no policy, so a host that persists the log
/// persists these alongside [`NoteCheckpoints::events`] and re-states
/// them with [`NoteCheckpoints::seed_policies`] after a rebuild —
/// otherwise a rebuilt log treats every note as public. The owner is
/// the note's create event's author; a record naming anyone else is
/// refused by `seed_policies`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotePolicyRecord {
    /// The note's stable ID: its `note.create` event's ID.
    pub note: Hash,
    /// The note's owner: its create event's author, who always holds Admin.
    pub owner: AuthorKey,
    /// The note's access policy.
    pub access: NoteAccess,
}

/// Resolves group membership roles for access checks: given a group
/// and a member key, the member's note role in that group, or
/// `None` when the member holds none. The host builds this over its
/// folded group state (in `id-web`, the groups projection); the
/// checkpoint core never folds groups itself. [`GroupRoles::none`]
/// is the resolver that knows no groups — user grants and public
/// notes still work in full.
#[derive(Clone)]
pub struct GroupRoles {
    inner: Arc<dyn Fn(Hash, AuthorKey) -> Option<NoteRole> + Send + Sync>,
}

impl GroupRoles {
    /// A resolver over a closure answering group roles.
    pub fn new(
        resolver: impl Fn(Hash, AuthorKey) -> Option<NoteRole> + Send + Sync + 'static,
    ) -> Self {
        Self {
            inner: Arc::new(resolver),
        }
    }

    /// The resolver that knows no groups.
    #[must_use]
    pub fn none() -> Self {
        Self::new(|_, _| None)
    }

    /// The member's note role in the group, if any.
    #[must_use]
    pub fn role(&self, group: Hash, member: AuthorKey) -> Option<NoteRole> {
        (self.inner)(group, member)
    }
}

impl std::fmt::Debug for GroupRoles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GroupRoles(..)")
    }
}

impl Default for GroupRoles {
    fn default() -> Self {
        Self::none()
    }
}

/// One note's recorded access state: who created it (the owner, who
/// always holds Admin and retains grant/revoke, #2048 Q2) and its
/// policy. Policies are application state layered over the event
/// log: the log's events carry no policy, so a log rebuilt with
/// [`NoteCheckpoints::from_events`] treats every note as
/// owner-created and [`NoteAccess::Public`] until the host re-states
/// its policies with [`NoteCheckpoints::seed_policies`] (from the
/// snapshot [`NoteCheckpoints::policies`] returns). A policy that is
/// never re-stated is a policy silently reopened to public.
#[derive(Clone, Debug, PartialEq, Eq)]
struct NotePolicy {
    owner: AuthorKey,
    access: NoteAccess,
}

/// The checkpoint log for collaborative notes: the applied note events
/// checkpoints are made of, folded on demand into per-note records and
/// heads. Events are appended only after the whole log still folds, so
/// a refused checkpoint leaves the log exactly as it was.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NoteCheckpoints {
    events: Vec<AppliedNote>,
    /// Per-note access policies, keyed by note ID. Only a note with
    /// an explicitly set policy has an entry (see
    /// [`Self::create_with_access`], [`Self::set_access`]). A note
    /// with no entry is owner-created (by its create event's author)
    /// and [`NoteAccess::Public`]: the Q3 default — that is what
    /// [`Self::access`] reports. [`crate::read_authority`] instead
    /// falls such a note through to the user default, then the server
    /// default, before public, so "no policy set" stays observable.
    policies: HashMap<Hash, NotePolicy>,
}

impl NoteCheckpoints {
    /// An empty log: no notes, no checkpoints.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuilds a log from applied note events — the restart/sync path:
    /// the events are the durable state, and the fold they produce is
    /// the state the live log had. Policies do NOT travel with the
    /// events: the rebuilt log treats every note as public until the
    /// host re-states them with [`Self::seed_policies`]; use
    /// [`Self::from_events_with_policies`] when the host has both.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::Fold`] when the events do not fold
    /// (duplicate IDs, a revision naming an unknown note or parent, a
    /// cycle).
    pub fn from_events(events: Vec<AppliedNote>) -> Result<Self, CheckpointError> {
        fold_notes(&events)?;
        Ok(Self {
            events,
            policies: HashMap::new(),
        })
    }

    /// [`Self::from_events`], with the policies the events do not
    /// carry: the rebuild restores the boundary exactly as the live
    /// log had it. Seed order matters — the events first (the notes
    /// must exist), then the policies.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Self::from_events`], and of
    /// [`Self::seed_policies`] for a record naming an unknown note
    /// or an owner other than the note's create author.
    pub fn from_events_with_policies(
        events: Vec<AppliedNote>,
        policies: Vec<NotePolicyRecord>,
    ) -> Result<Self, CheckpointError> {
        let mut rebuilt = Self::from_events(events)?;
        rebuilt.seed_policies(policies)?;
        Ok(rebuilt)
    }

    /// The applied note events, in the order they were recorded. This
    /// is the durable form of the log: persist or sync these and
    /// [`Self::from_events`] rebuilds the rest.
    #[must_use]
    pub fn events(&self) -> &[AppliedNote] {
        &self.events
    }

    /// The recorded access policy of every note that has one, in
    /// note ID order: the durable form of the policies, to persist
    /// alongside [`Self::events`] and re-state with
    /// [`Self::seed_policies`] after a rebuild. Only recorded
    /// policies appear — a note with none is absent, not reported
    /// as [`NoteAccess::Public`]: reporting the effective policy
    /// here would re-state a Public entry on rebuild and make "no
    /// policy set" unobservable, closing the #5678 fall-through
    /// (user default, then server default) after a restart. A note
    /// with no record still reads as Public through [`Self::access`].
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::Fold`] when the recorded events do
    /// not fold. Events are validated before they are recorded, so a
    /// log built through this type's methods does not produce this.
    pub fn policies(&self) -> Result<Vec<NotePolicyRecord>, CheckpointError> {
        let mut records = Vec::new();
        for record in self.notes()? {
            if let Some((owner, access)) = self.recorded_access(record.id) {
                records.push(NotePolicyRecord {
                    note: record.id,
                    owner,
                    access,
                });
            }
        }
        Ok(records)
    }

    /// Re-states policies after a rebuild from events (the
    /// restart/sync path): each record replaces the note's policy.
    /// This is a trusted restore of the host's own persisted
    /// snapshot, not a policy change — it takes no author and runs
    /// no admin check — so it refuses corrupt input instead: a
    /// record for a note the log does not have, or naming an owner
    /// other than the note's create event's author. On any error the
    /// policies are unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::UnknownNote`] for a record naming
    /// a note with no create event in the log, and
    /// [`CheckpointError::Note`] for a record naming the wrong owner.
    pub fn seed_policies(
        &mut self,
        policies: Vec<NotePolicyRecord>,
    ) -> Result<(), CheckpointError> {
        let mut candidate = self.policies.clone();
        for record in &policies {
            let (owner, _) = self
                .access(record.note)
                .ok_or(CheckpointError::UnknownNote(record.note))?;
            if record.owner != owner {
                return Err(CheckpointError::Note(NoteError::Field(format!(
                    "policy for note {} names owner {:?}, its create event names {:?}",
                    record.note.to_hex(),
                    record.owner,
                    owner,
                ))));
            }
            candidate.insert(
                record.note,
                NotePolicy {
                    owner,
                    access: record.access.clone(),
                },
            );
        }
        self.policies = candidate;
        Ok(())
    }

    /// Every note in the log, folded, in note ID order.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::Fold`] when the recorded events do
    /// not fold. Events are validated before they are recorded, so a
    /// log built through this type's methods does not produce this.
    pub fn notes(&self) -> Result<Vec<NoteRecord>, CheckpointError> {
        Ok(fold_notes(&self.events)?)
    }

    /// One note's folded record, if the log has its create event.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::Fold`] on the same condition as
    /// [`Self::notes`].
    pub fn record(&self, note: Hash) -> Result<Option<NoteRecord>, CheckpointError> {
        Ok(self.notes()?.into_iter().find(|record| record.id == note))
    }

    /// A note's current heads, in ID order: the revisions no other
    /// revision names as a parent. Empty when the note is unknown.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::Fold`] on the same condition as
    /// [`Self::notes`].
    pub fn heads(&self, note: Hash) -> Result<Vec<Hash>, CheckpointError> {
        Ok(self
            .record(note)?
            .map(|record| record.heads)
            .unwrap_or_default())
    }

    /// Establishes a note in the log: records the `note.create` event
    /// whose ID is the note's stable ID, carrying its first content.
    /// The create is also the note's first revision and its only head.
    /// It records no access policy: the note reads as
    /// [`NoteAccess::Public`] (the Q3 default) until a policy is set
    /// with [`Self::create_with_access`] or [`Self::set_access`] — or
    /// a default applies through [`crate::read_authority`], which
    /// falls a note with no recorded policy through to the user
    /// default, then the server default, before public. Recording
    /// the default here would make "no policy set" unobservable and
    /// the #5678 fall-through unreachable.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::Note`] when the content breaks the
    /// document rules, and [`CheckpointError::Fold`] when the event ID
    /// is already in the log.
    pub fn create(
        &mut self,
        event_id: Hash,
        author: AuthorKey,
        content: DocumentRef,
    ) -> Result<NoteRecord, CheckpointError> {
        let op = NoteOp::Create { content };
        check_note_op(CREATE_KIND, &op)?;
        self.append(AppliedNote {
            id: event_id,
            author,
            op,
        })?;
        // A just-created note always folds; should it not, report the
        // note as unknown rather than panic.
        self.record(event_id)?
            .ok_or(CheckpointError::UnknownNote(event_id))
    }

    /// Publishes one checkpoint of `note`: a signed revision naming the
    /// parents the caller chose. Naming a head extends it; naming an
    /// older revision forks from it, and the fork is a head of its own —
    /// no checkpoint ever overwrites or retires another session's head.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::UnknownNote`] when the log has no
    /// create event for `note`, [`CheckpointError::Note`] when the
    /// revision breaks the note rules (no parents, a parent named
    /// twice, too many parents, a content rule), and
    /// [`CheckpointError::Fold`] when a parent is not a revision of
    /// this note or the event ID is already in the log. On any error
    /// the log is unchanged.
    /// Establishes a note with an access policy (see
    /// [`Self::create`], which records no policy: a note with none
    /// reads as [`NoteAccess::Public`]). The creator is recorded as the
    /// note's owner.
    ///
    /// # Errors
    ///
    /// The errors of [`Self::create`].
    pub fn create_with_access(
        &mut self,
        event_id: Hash,
        author: AuthorKey,
        content: DocumentRef,
        access: NoteAccess,
    ) -> Result<NoteRecord, CheckpointError> {
        let record = self.create(event_id, author, content)?;
        self.policies.insert(
            event_id,
            NotePolicy {
                owner: author,
                access,
            },
        );
        Ok(record)
    }

    /// A note's access policy, and its owner's key. `None` for a
    /// note the log does not have. A note with no recorded policy is
    /// reported with its create event's author as owner and
    /// [`NoteAccess::Public`].
    #[must_use]
    pub fn access(&self, note: Hash) -> Option<(AuthorKey, NoteAccess)> {
        if let Some(policy) = self.policies.get(&note) {
            return Some((policy.owner, policy.access.clone()));
        }
        let record = self.record(note).ok()??;
        // The owner is the create event's author: the revision whose
        // ID is the note's ID. Revisions are in ID order, so the
        // first sorted revision need not be the create — a later
        // revision's ID may sort before the note's ID.
        let owner = record.revision(note)?.author;
        Some((owner, NoteAccess::Public))
    }

    /// A note's recorded access policy, if the log holds one for
    /// it. `None` both for a note the log does not have and for a
    /// known note with no recorded policy — unlike [`Self::access`],
    /// which reports the latter as [`NoteAccess::Public`]. Callers
    /// that must tell those apart (the #5678 resolution order in
    /// [`crate::read_authority` falls a known note with no recorded
    /// policy through to the user default, then the server default)
    /// use this; callers that only need the effective policy use
    /// `access`.
    #[must_use]
    pub fn recorded_access(&self, note: Hash) -> Option<(AuthorKey, NoteAccess)> {
        self.policies
            .get(&note)
            .map(|policy| (policy.owner, policy.access.clone()))
    }

    /// The role `author` effectively holds on `note`: the owner is
    /// always [`NoteRole::Admin`]; on a public note every author
    /// holds [`NoteRole::Write`] (read included); on a scoped note
    /// the best matching grant wins — a direct user grant, or a
    /// group grant the `groups` resolver answers for. `None` when
    /// the author holds nothing on a scoped note, or the note is
    /// unknown. The grant logic lives in
    /// [`crate::read_authority::role_for`]; this applies it to the
    /// note's recorded policy (a note with no recorded policy reads
    /// as [`NoteAccess::Public`] — the full #5678 order, with the
    /// user and server defaults, is [`crate::read_authority::may_read`]).
    #[must_use]
    pub fn effective_role(
        &self,
        note: Hash,
        author: AuthorKey,
        groups: &GroupRoles,
    ) -> Option<NoteRole> {
        let (owner, access) = self.access(note)?;
        crate::read_authority::role_for(owner, &access, author, groups)
    }

    /// Whether `author` may read `note` (any role at all on a
    /// scoped note; everyone on a public one).
    #[must_use]
    pub fn may_read(&self, note: Hash, author: AuthorKey, groups: &GroupRoles) -> bool {
        self.effective_role(note, author, groups).is_some()
    }

    /// Whether `author` may append a revision to `note` (#2048 Q2:
    /// a write grant or better; the owner always may).
    #[must_use]
    pub fn may_append(&self, note: Hash, author: AuthorKey, groups: &GroupRoles) -> bool {
        self.effective_role(note, author, groups)
            .is_some_and(|role| role >= NoteRole::Write)
    }

    /// Replaces a note's access policy. Only an author holding
    /// [`NoteRole::Admin`] on the note — the owner, or an admin
    /// grant — may do this: grant/revoke stays with the owner and
    /// their admins (#2048 Q2).
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::UnknownNote`] for a note the log
    /// does not have, and [`CheckpointError::NotAuthorized`] when
    /// `by` holds less than Admin.
    pub fn set_access(
        &mut self,
        note: Hash,
        by: AuthorKey,
        access: NoteAccess,
        groups: &GroupRoles,
    ) -> Result<(), CheckpointError> {
        let (owner, _) = self
            .access(note)
            .ok_or(CheckpointError::UnknownNote(note))?;
        if self.effective_role(note, by, groups) != Some(NoteRole::Admin) {
            return Err(CheckpointError::NotAuthorized { note, author: by });
        }
        self.policies.insert(note, NotePolicy { owner, access });
        Ok(())
    }

    /// The append half of the access boundary, shared by the
    /// checked publish paths.
    fn authorize_append(
        &self,
        note: Hash,
        author: AuthorKey,
        groups: &GroupRoles,
    ) -> Result<(), CheckpointError> {
        if self.may_append(note, author, groups) {
            Ok(())
        } else {
            Err(CheckpointError::NotAuthorized { note, author })
        }
    }

    /// [`Self::checkpoint`], gated on the access boundary: the
    /// author must hold a write grant on the note (#2048 Q2).
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::NotAuthorized`] when the author
    /// holds less than [`NoteRole::Write`], and the errors of
    /// [`Self::checkpoint`] otherwise. On any error the log is
    /// unchanged.
    pub fn checkpoint_checked(
        &mut self,
        note: Hash,
        event_id: Hash,
        author: AuthorKey,
        content: DocumentRef,
        parents: Vec<Hash>,
        groups: &GroupRoles,
    ) -> Result<NoteRecord, CheckpointError> {
        if self.record(note)?.is_none() {
            return Err(CheckpointError::UnknownNote(note));
        }
        self.authorize_append(note, author, groups)?;
        self.checkpoint(note, event_id, author, content, parents)
    }

    /// [`Self::checkpoint_on_heads`], gated on the access boundary.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::NotAuthorized`] when the author
    /// holds less than [`NoteRole::Write`], and the errors of
    /// [`Self::checkpoint_on_heads`] otherwise.
    pub fn checkpoint_on_heads_checked(
        &mut self,
        note: Hash,
        event_id: Hash,
        author: AuthorKey,
        content: DocumentRef,
        groups: &GroupRoles,
    ) -> Result<NoteRecord, CheckpointError> {
        let heads = self.heads(note)?;
        match heads.len() {
            0 => Err(CheckpointError::UnknownNote(note)),
            1 => {
                self.authorize_append(note, author, groups)?;
                self.checkpoint(note, event_id, author, content, heads)
            }
            _ => Err(CheckpointError::ExplicitMergeRequired { note, heads }),
        }
    }

    /// [`Self::merge`], gated on the access boundary: merging is an
    /// append, so it takes the same write grant.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::NotAuthorized`] when the author
    /// holds less than [`NoteRole::Write`], and the errors of
    /// [`Self::merge`] otherwise.
    pub fn merge_checked(
        &mut self,
        note: Hash,
        event_id: Hash,
        author: AuthorKey,
        content: DocumentRef,
        groups: &GroupRoles,
    ) -> Result<NoteRecord, CheckpointError> {
        let heads = self.heads(note)?;
        if heads.len() < 2 {
            return Err(CheckpointError::NotConcurrent { note, heads });
        }
        self.authorize_append(note, author, groups)?;
        self.checkpoint(note, event_id, author, content, heads)
    }

    pub fn checkpoint(
        &mut self,
        note: Hash,
        event_id: Hash,
        author: AuthorKey,
        content: DocumentRef,
        parents: Vec<Hash>,
    ) -> Result<NoteRecord, CheckpointError> {
        if self.record(note)?.is_none() {
            return Err(CheckpointError::UnknownNote(note));
        }
        let op = NoteOp::Revise {
            note,
            content,
            parents,
            proposal: None,
        };
        check_note_op(REVISE_KIND, &op)?;
        self.append(AppliedNote {
            id: event_id,
            author,
            op,
        })?;
        // The note existed before the append, so it still folds; should
        // it not, report it as unknown rather than panic.
        self.record(note)?.ok_or(CheckpointError::UnknownNote(note))
    }

    /// Publishes a checkpoint on the note's current head — the ordinary
    /// save path, for a note with exactly one head.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::ExplicitMergeRequired`], naming the
    /// heads, when the note has more than one head: saving must not
    /// silently join concurrent work. Returns the errors of
    /// [`Self::checkpoint`] otherwise, including
    /// [`CheckpointError::UnknownNote`] for an unknown note (which has
    /// no heads to build on).
    pub fn checkpoint_on_heads(
        &mut self,
        note: Hash,
        event_id: Hash,
        author: AuthorKey,
        content: DocumentRef,
    ) -> Result<NoteRecord, CheckpointError> {
        let heads = self.heads(note)?;
        match heads.len() {
            0 => Err(CheckpointError::UnknownNote(note)),
            1 => self.checkpoint(note, event_id, author, content, heads),
            _ => Err(CheckpointError::ExplicitMergeRequired { note, heads }),
        }
    }

    /// Explicitly merges a note's concurrent heads: one revision naming
    /// every current head as its parent, after which the merge is the
    /// note's only head. This is the only operation that retires more
    /// than one head, and it exists only as a deliberate call.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::NotConcurrent`], naming the heads,
    /// when the note has fewer than two heads, and the errors of
    /// [`Self::checkpoint`] otherwise.
    pub fn merge(
        &mut self,
        note: Hash,
        event_id: Hash,
        author: AuthorKey,
        content: DocumentRef,
    ) -> Result<NoteRecord, CheckpointError> {
        let heads = self.heads(note)?;
        if heads.len() < 2 {
            return Err(CheckpointError::NotConcurrent { note, heads });
        }
        self.checkpoint(note, event_id, author, content, heads)
    }

    /// Records one event after proving the whole log still folds with
    /// it; a failed proof records nothing.
    fn append(&mut self, event: AppliedNote) -> Result<(), CheckpointError> {
        let mut candidate = self.events.clone();
        candidate.push(event);
        fold_notes(&candidate)?;
        self.events = candidate;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn hash(byte: u8) -> Hash {
        Hash::from_bytes([byte; 32])
    }

    fn author(byte: u8) -> AuthorKey {
        [byte; 32]
    }

    fn content(bytes: &[u8]) -> DocumentRef {
        DocumentRef::for_bytes("text/markdown", bytes).unwrap()
    }

    #[test]
    fn test_a_checkpoint_chains_on_the_head_and_the_note_id_is_stable() {
        let mut log = NoteCheckpoints::new();
        let created = log.create(hash(10), author(1), content(b"first")).unwrap();
        assert_eq!(created.id, hash(10));
        assert_eq!(created.heads, vec![hash(10)]);

        let saved = log
            .checkpoint_on_heads(hash(10), hash(11), author(1), content(b"second"))
            .unwrap();
        assert_eq!(
            saved.id,
            hash(10),
            "the note ID never becomes a revision ID"
        );
        assert_eq!(saved.revisions.len(), 2);
        assert_eq!(saved.heads, vec![hash(11)]);
        assert_eq!(saved.revision(hash(11)).unwrap().parents, vec![hash(10)]);
        assert!(
            saved
                .revision(hash(11))
                .unwrap()
                .content
                .verify_content(b"second")
                .is_ok()
        );
    }

    #[test]
    fn test_concurrent_checkpoints_stay_two_heads_until_an_explicit_merge() {
        let mut log = NoteCheckpoints::new();
        log.create(hash(10), author(1), content(b"first")).unwrap();
        // Two sessions checkpoint from the same head: neither wins.
        log.checkpoint(
            hash(10),
            hash(11),
            author(1),
            content(b"alice"),
            vec![hash(10)],
        )
        .unwrap();
        let forked = log
            .checkpoint(
                hash(10),
                hash(12),
                author(2),
                content(b"bob"),
                vec![hash(10)],
            )
            .unwrap();
        assert_eq!(forked.heads, vec![hash(11), hash(12)]);

        // The ordinary save path refuses to join them silently, and
        // the refusal changes nothing.
        let err = log
            .checkpoint_on_heads(hash(10), hash(13), author(1), content(b"sneaky"))
            .unwrap_err();
        assert_eq!(
            err,
            CheckpointError::ExplicitMergeRequired {
                note: hash(10),
                heads: vec![hash(11), hash(12)],
            }
        );
        assert_eq!(log.heads(hash(10)).unwrap(), vec![hash(11), hash(12)]);

        // The explicit merge names both heads and becomes the one head.
        let merged = log
            .merge(hash(10), hash(13), author(1), content(b"joined"))
            .unwrap();
        assert_eq!(merged.heads, vec![hash(13)]);
        assert_eq!(
            merged.revision(hash(13)).unwrap().parents,
            vec![hash(11), hash(12)]
        );
        // And a merge with nothing concurrent left is refused.
        assert_eq!(
            log.merge(hash(10), hash(14), author(1), content(b"again"))
                .unwrap_err(),
            CheckpointError::NotConcurrent {
                note: hash(10),
                heads: vec![hash(13)],
            }
        );
    }

    #[test]
    fn test_a_checkpoint_from_a_stale_head_forks_instead_of_overwriting() {
        let mut log = NoteCheckpoints::new();
        log.create(hash(10), author(1), content(b"first")).unwrap();
        log.checkpoint_on_heads(hash(10), hash(11), author(1), content(b"second"))
            .unwrap();
        // A session that never saw revision 11 checkpoints from 10:
        // its work is a second head, not a lost update to 11.
        let forked = log
            .checkpoint(
                hash(10),
                hash(12),
                author(2),
                content(b"offline"),
                vec![hash(10)],
            )
            .unwrap();
        assert_eq!(forked.heads, vec![hash(11), hash(12)]);
    }

    #[test]
    fn test_a_refused_checkpoint_leaves_the_log_unchanged() {
        let mut log = NoteCheckpoints::new();
        log.create(hash(10), author(1), content(b"first")).unwrap();
        let before = log.events().to_vec();
        // A parent that is not a revision of this note.
        assert!(
            log.checkpoint(hash(10), hash(11), author(1), content(b"x"), vec![hash(99)])
                .is_err()
        );
        // No parents at all.
        assert!(
            log.checkpoint(hash(10), hash(11), author(1), content(b"x"), vec![])
                .is_err()
        );
        // An unknown note.
        assert_eq!(
            log.checkpoint_on_heads(hash(77), hash(11), author(1), content(b"x"))
                .unwrap_err(),
            CheckpointError::UnknownNote(hash(77))
        );
        assert_eq!(log.events(), before.as_slice());
        assert_eq!(log.heads(hash(10)).unwrap(), vec![hash(10)]);
    }

    #[test]
    fn test_the_log_rebuilds_from_its_events_with_the_same_heads() {
        let mut log = NoteCheckpoints::new();
        log.create(hash(10), author(1), content(b"first")).unwrap();
        log.checkpoint(
            hash(10),
            hash(11),
            author(1),
            content(b"alice"),
            vec![hash(10)],
        )
        .unwrap();
        log.checkpoint(
            hash(10),
            hash(12),
            author(2),
            content(b"bob"),
            vec![hash(10)],
        )
        .unwrap();
        // Restart/sync: the events alone rebuild the log, and the
        // rebuild does not depend on the order they arrived in.
        let mut reversed = log.events().to_vec();
        reversed.reverse();
        let rebuilt = NoteCheckpoints::from_events(reversed).unwrap();
        assert_eq!(rebuilt.notes().unwrap(), log.notes().unwrap());
        assert_eq!(rebuilt.heads(hash(10)).unwrap(), vec![hash(11), hash(12)]);
    }

    // ------------------------------------------------ access boundary --

    fn scoped_log() -> (NoteCheckpoints, Hash) {
        // Owner 1 creates a note scoped to: user 2 Read, user 3
        // Write, group 9 Write (membership answered by the resolver
        // each test supplies).
        let mut log = NoteCheckpoints::new();
        let access = NoteAccess::Scoped(vec![
            NoteGrant {
                principal: NotePrincipal::User(author(2)),
                role: NoteRole::Read,
            },
            NoteGrant {
                principal: NotePrincipal::User(author(3)),
                role: NoteRole::Write,
            },
            NoteGrant {
                principal: NotePrincipal::Group(hash(9)),
                role: NoteRole::Write,
            },
        ]);
        let created = log
            .create_with_access(hash(10), author(1), content(b"first"), access)
            .unwrap();
        (log, created.id)
    }

    #[test]
    fn test_public_notes_accept_any_attributed_append() {
        let mut log = NoteCheckpoints::new();
        let created = log.create(hash(10), author(1), content(b"first")).unwrap();
        let groups = GroupRoles::none();
        // Q3 default: public. A stranger's append is a signed,
        // attributed revision — allowed, and recorded under their key.
        let saved = log
            .checkpoint_on_heads_checked(
                created.id,
                hash(11),
                author(7),
                content(b"second"),
                &groups,
            )
            .unwrap();
        assert_eq!(saved.revisions.last().unwrap().author, author(7));
        assert!(log.may_read(created.id, author(8), &groups));
    }

    #[test]
    fn test_scoped_note_refuses_appends_without_a_write_grant() {
        let (mut log, note) = scoped_log();
        let groups = GroupRoles::none();
        let before = log.events().to_vec();

        // A read grant is not a write grant.
        let refused =
            log.checkpoint_on_heads_checked(note, hash(11), author(2), content(b"x"), &groups);
        assert_eq!(
            refused.unwrap_err(),
            CheckpointError::NotAuthorized {
                note,
                author: author(2)
            }
        );
        // No grant at all.
        assert!(matches!(
            log.checkpoint_checked(
                note,
                hash(11),
                author(8),
                content(b"x"),
                vec![note],
                &groups
            ),
            Err(CheckpointError::NotAuthorized { .. })
        ));
        assert_eq!(log.events(), before.as_slice(), "a refusal records nothing");
        assert!(!log.may_read(note, author(8), &groups));
        assert!(log.may_read(note, author(2), &groups));

        // The write-grant user appends; the owner always may.
        log.checkpoint_on_heads_checked(note, hash(11), author(3), content(b"second"), &groups)
            .unwrap();
        log.checkpoint_on_heads_checked(note, hash(12), author(1), content(b"third"), &groups)
            .unwrap();
    }

    #[test]
    fn test_group_grants_resolve_through_the_group_resolver() {
        let (mut log, note) = scoped_log();
        // Author 5 is in group 9 (the resolver says Write); author 6
        // is not in any group.
        let groups = GroupRoles::new(|group, member| {
            if group == hash(9) && member == author(5) {
                Some(NoteRole::Write)
            } else {
                None
            }
        });
        log.checkpoint_on_heads_checked(note, hash(11), author(5), content(b"via group"), &groups)
            .unwrap();
        assert!(matches!(
            log.checkpoint_on_heads_checked(note, hash(12), author(6), content(b"nope"), &groups),
            Err(CheckpointError::NotAuthorized { .. })
        ));
        // A resolver reporting a lesser role than the grant caps at
        // the lesser role: Read in the group does not satisfy a
        // Write grant.
        let lesser = GroupRoles::new(|_, _| Some(NoteRole::Read));
        assert!(!log.may_append(note, author(5), &lesser));
    }

    #[test]
    fn test_policy_changes_take_admin_and_the_owner_retains_it() {
        let (mut log, note) = scoped_log();
        let groups = GroupRoles::none();
        // A write-grant holder cannot re-scope the note.
        assert!(matches!(
            log.set_access(note, author(3), NoteAccess::Public, &groups),
            Err(CheckpointError::NotAuthorized { .. })
        ));
        // The owner can: opening the note up makes a stranger's
        // append lawful.
        log.set_access(note, author(1), NoteAccess::Public, &groups)
            .unwrap();
        log.checkpoint_on_heads_checked(note, hash(11), author(8), content(b"now public"), &groups)
            .unwrap();
        // An admin grant holder can re-scope too.
        log.set_access(
            note,
            author(1),
            NoteAccess::Scoped(vec![NoteGrant {
                principal: NotePrincipal::User(author(4)),
                role: NoteRole::Admin,
            }]),
            &groups,
        )
        .unwrap();
        log.set_access(note, author(4), NoteAccess::Public, &groups)
            .unwrap();
    }

    #[test]
    fn test_merge_checked_enforces_the_boundary() {
        let (mut log, note) = scoped_log();
        let groups = GroupRoles::none();
        // Fork the note into two heads with the raw graph appends
        // (two granted writers working from the same head).
        log.checkpoint(note, hash(11), author(3), content(b"a"), vec![note])
            .unwrap();
        log.checkpoint(note, hash(12), author(1), content(b"b"), vec![note])
            .unwrap();
        assert_eq!(log.heads(note).unwrap().len(), 2);
        // A read-grant holder cannot merge; a write-grant holder can.
        assert!(matches!(
            log.merge_checked(note, hash(13), author(2), content(b"merged"), &groups),
            Err(CheckpointError::NotAuthorized { .. })
        ));
        let merged = log
            .merge_checked(note, hash(13), author(3), content(b"merged"), &groups)
            .unwrap();
        assert_eq!(merged.heads, vec![hash(13)]);
    }

    #[test]
    fn test_a_scoped_policy_survives_a_restart_rebuild() {
        // Oracle: a restart rebuilds the log from its durable form,
        // and the access boundary must hold exactly as before the
        // restart — a scoped note must not silently reopen to public.
        let (log, note) = scoped_log();
        let groups = GroupRoles::none();
        assert!(!log.may_append(note, author(8), &groups));

        // Restart/sync: the events alone rebuild the log, and the
        // policies snapshot re-states what the events do not carry.
        let events = log.events().to_vec();
        let policies = log.policies().unwrap();
        let mut rebuilt = NoteCheckpoints::from_events(events).unwrap();
        rebuilt.seed_policies(policies).unwrap();

        assert!(!rebuilt.may_append(note, author(8), &groups));
        assert!(rebuilt.may_append(note, author(3), &groups));
        assert!(rebuilt.may_read(note, author(2), &groups));
        rebuilt
            .checkpoint_on_heads_checked(note, hash(11), author(3), content(b"second"), &groups)
            .unwrap();
        assert!(matches!(
            rebuilt.checkpoint_on_heads_checked(
                note,
                hash(12),
                author(8),
                content(b"nope"),
                &groups
            ),
            Err(CheckpointError::NotAuthorized { .. })
        ));
    }

    #[test]
    fn test_seed_policies_refuses_unknown_notes_and_wrong_owners() {
        let (log, note) = scoped_log();
        let events = log.events().to_vec();
        let mut rebuilt = NoteCheckpoints::from_events(events).unwrap();
        // Before any seeding the rebuild holds no recorded policy
        // for the note: it reads as public through access(), but
        // the snapshot is empty — a note with no recorded policy
        // contributes no record.
        let unseeded = rebuilt.policies().unwrap();
        assert_eq!(unseeded, [] as [NotePolicyRecord; 0]);
        assert_eq!(rebuilt.recorded_access(note), None);
        assert_eq!(
            rebuilt.access(note).unwrap(),
            (author(1), NoteAccess::Public)
        );
        // A policy for a note the rebuilt log does not have: the
        // host seeded policies before events, or the wrong log.
        let unknown = NotePolicyRecord {
            note: hash(77),
            owner: author(1),
            access: NoteAccess::Public,
        };
        assert_eq!(
            rebuilt.seed_policies(vec![unknown]).unwrap_err(),
            CheckpointError::UnknownNote(hash(77))
        );
        // A policy naming anyone but the create author is corrupt
        // input, not a transfer of ownership.
        let wrong_owner = NotePolicyRecord {
            note,
            owner: author(2),
            access: NoteAccess::Public,
        };
        assert!(matches!(
            rebuilt.seed_policies(vec![wrong_owner]).unwrap_err(),
            CheckpointError::Note(_)
        ));
        // Either refusal leaves the policies exactly as they were.
        assert_eq!(rebuilt.policies().unwrap(), unseeded);
        // And the correct snapshot still re-states the boundary.
        rebuilt.seed_policies(log.policies().unwrap()).unwrap();
        assert_eq!(rebuilt.policies().unwrap(), log.policies().unwrap());
    }

    #[test]
    fn test_restored_owner_comes_from_the_create_event_not_the_first_sorted_revision() {
        // A revision whose ID sorts before the note's create event ID,
        // by another author: the fold lists it first, but the owner is
        // still the create event's author.
        let mut log = NoteCheckpoints::new();
        let note = log
            .create(hash(10), author(1), content(b"first"))
            .unwrap()
            .id;
        log.checkpoint(note, hash(9), author(2), content(b"earlier id"), vec![note])
            .unwrap();
        assert_eq!(
            log.notes().unwrap()[0].revisions.first().unwrap().author,
            author(2),
            "the fixture really does put another author's revision first"
        );
        let groups = GroupRoles::none();
        // The live log names the creator through access(): a note
        // with no recorded policy falls back to its create event's
        // author, and the snapshot carries no record for it.
        assert_eq!(log.access(note).unwrap().0, author(1));
        assert_eq!(log.policies().unwrap(), [] as [NotePolicyRecord; 0]);

        // Restart: the events rebuild the log with no policies, and
        // the snapshot for a plain note is empty — the owner still
        // comes from the create event, never from the first sorted
        // revision.
        let events = log.events().to_vec();
        let policies = log.policies().unwrap();
        let rebuilt = NoteCheckpoints::from_events(events.clone()).unwrap();
        assert_eq!(rebuilt.access(note).unwrap().0, author(1));
        assert_eq!(rebuilt.policies().unwrap(), [] as [NotePolicyRecord; 0]);
        let restored = NoteCheckpoints::from_events_with_policies(events, policies).unwrap();
        assert_eq!(restored.access(note).unwrap().0, author(1));
        assert!(restored.may_append(note, author(1), &groups));
        // And the two-step seeding path accepts the same snapshot.
        let mut reseeded = NoteCheckpoints::from_events(log.events().to_vec()).unwrap();
        reseeded.seed_policies(log.policies().unwrap()).unwrap();
        assert_eq!(reseeded.policies().unwrap(), log.policies().unwrap());
    }

    #[test]
    fn test_policies_snapshot_serializes_for_the_host_to_persist() {
        let (log, _) = scoped_log();
        let policies = log.policies().unwrap();
        let json = serde_json::to_string(&policies).unwrap();
        let restored: Vec<NotePolicyRecord> = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, policies);
    }

    #[test]
    fn test_policies_snapshots_only_recorded_policies() {
        // A scoped note and a plain note: the snapshot carries one
        // record — the scoped note's. The plain note reads as Public
        // through access() but contributes no record, so a rebuild
        // re-states nothing for it and its fall-through survives.
        let (mut log, scoped) = scoped_log();
        let plain = log
            .create(hash(20), author(1), content(b"plain"))
            .unwrap()
            .id;
        let policies = log.policies().unwrap();
        assert_eq!(policies.len(), 1);
        assert_eq!(policies[0].note, scoped);
        assert_eq!(log.recorded_access(plain), None);

        let rebuilt =
            NoteCheckpoints::from_events_with_policies(log.events().to_vec(), policies).unwrap();
        assert_eq!(rebuilt.recorded_access(scoped), log.recorded_access(scoped));
        assert_eq!(rebuilt.recorded_access(plain), None);
        assert_eq!(rebuilt.policies().unwrap(), log.policies().unwrap());
    }
}
