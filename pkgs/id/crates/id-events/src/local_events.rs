//! Documents and tags, signed into the event store from one node (P6.7c).
//!
//! `id doc` and `id claim` act through [`LocalEvents`]. Each action signs one event with the node's
//! key, names every other author's head as its `after`, and ingests the event with
//! [`EventStore::ingest`], the path sync uses. The store checks the signature and applies the event.
//! So these events are signed, unlike the group log's (P6.7a), which trusts the caller's name.
//!
//! The CLI opens one store directory, named by `--event-store`, and nodes on one machine that
//! share the directory see each other's events, as they share the group log. Between machines,
//! `id serve` moves the events: it shares this store with the `/id-range-sync/1` range-sync sessions
//! in [`crate::event_sync`], so a revision one node signs into its store is in the sync set the
//! next session carries to a peer (#792).
//!
//! The reads walk every applied event, which is fine for the CLI and is not an index. The document
//! and tag rules are the ones in [`id_store::documents`] and [`id_store::tag_claims`].

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use p2panda_core::{Hash, SigningKey};

use id_store::chain::{ChainExt, Event, MAX_AFTER, sign};
use id_store::chat::{CHAT_MESSAGE_KIND, ChatMessage, ChatRecord, SendRate};
use id_store::documents::{DOCUMENT_KIND, DocumentRef, REMOVAL_KIND};
use crate::event_store::{Block, BlockKind, BlockLookup, EventStore, Ingest};
use crate::local_claims::LocalClaims;
use id_store::notes::{
    AppliedNote, CREATE_KIND as NOTE_CREATE_KIND, NoteFoldError, NoteOp, NoteRecord,
    REVISE_KIND as NOTE_REVISE_KIND, check_note_op, fold_notes,
};
use id_store::search::{Hit, ValueMatch, search_store};
use id_store::tag_claims::{
    CLEAR_KIND, Claim, LocalClaim, MAX_CLEAR_CLAIMS, SET_KIND, TagOp, TagScope, check_op,
};
use id_core::waiting::{AuthorKey, WaitingLimits};

/// The directory the event store lives in, under the data directory, unless `--event-store` names
/// another.
pub const DIR_NAME: &str = "events";

/// One event that is part of a document's history, as the CLI lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// The `document.put` that makes the document. Its ID is the document's.
    Document {
        /// The event's ID.
        id: Hash,
        /// The key that signed it.
        author: AuthorKey,
        /// The content the event names.
        document: DocumentRef,
    },
    /// A `tag.set` on `subject`.
    Set {
        /// The event's ID, which is the claim's.
        id: Hash,
        /// The key that signed it.
        author: AuthorKey,
        /// The event the claim is about.
        subject: Hash,
        /// The tag name.
        name: String,
        /// The tag value, if any.
        value: Option<String>,
    },
    /// A `tag.clear` that names the claims it removes.
    Clear {
        /// The event's ID.
        id: Hash,
        /// The key that signed it.
        author: AuthorKey,
        /// The claims it names.
        claims: Vec<Hash>,
    },
    /// A `note.create`: the note it establishes. Its ID is the note's stable ID.
    NoteCreate {
        /// The event's ID, which is also the note's ID.
        id: Hash,
        /// The key that signed it.
        author: AuthorKey,
        /// The note's first content.
        content: DocumentRef,
    },
    /// A `note.revise` of the note it names.
    NoteRevise {
        /// The event's ID, which is the revision's ID.
        id: Hash,
        /// The key that signed it.
        author: AuthorKey,
        /// The note's stable ID.
        note: Hash,
        /// The revision's content.
        content: DocumentRef,
        /// The revisions this one builds on.
        parents: Vec<Hash>,
    },
    /// A `document.remove` of the document it names.
    Remove {
        /// The event's ID.
        id: Hash,
        /// The key that signed it.
        author: AuthorKey,
        /// The document it removes.
        document: Hash,
    },
}

impl Entry {
    /// The event's ID.
    #[must_use]
    pub const fn id(&self) -> Hash {
        match self {
            Self::Document { id, .. }
            | Self::Set { id, .. }
            | Self::Clear { id, .. }
            | Self::NoteCreate { id, .. }
            | Self::NoteRevise { id, .. }
            | Self::Remove { id, .. } => *id,
        }
    }

    /// The key that signed the event.
    #[must_use]
    pub const fn author(&self) -> AuthorKey {
        match self {
            Self::Document { author, .. }
            | Self::Set { author, .. }
            | Self::Clear { author, .. }
            | Self::NoteCreate { author, .. }
            | Self::NoteRevise { author, .. }
            | Self::Remove { author, .. } => *author,
        }
    }

    /// The event's kind, as its header names it.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Document { .. } => DOCUMENT_KIND,
            Self::Set { .. } => SET_KIND,
            Self::Clear { .. } => CLEAR_KIND,
            Self::NoteCreate { .. } => NOTE_CREATE_KIND,
            Self::NoteRevise { .. } => NOTE_REVISE_KIND,
            Self::Remove { .. } => REMOVAL_KIND,
        }
    }
}

/// The latest applied event of one author: the head of its chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Head {
    /// The author.
    pub author: AuthorKey,
    /// The author's last applied event.
    pub event: Hash,
    /// That event's sequence number, counted from 0.
    pub seq: u64,
}

/// A published document: its `document.put`'s ID, the reference that event names, and the IDs of
/// the tag claims set with it, in the order the tags were given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Published {
    /// The document's ID: the `document.put` event's ID.
    pub id: Hash,
    /// The content the document names.
    pub document: DocumentRef,
    /// The IDs of the claims set with the document, one per tag.
    pub claims: Vec<Hash>,
}

/// Every applied event's entry by ID, and every applied event's parents by ID.
type Log = (HashMap<Hash, Entry>, HashMap<Hash, Vec<Hash>>);

/// Folds store note events while quarantining malformed remote revisions.
///
/// Unknown/foreign-parent revisions are removed first. If the remaining same-note graph is
/// cyclic, remove a revision identified by the strict fold and repeat so its dependents are
/// quarantined too; unrelated healthy notes remain readable.
fn fold_remote_notes(
    mut applied: Vec<AppliedNote>,
) -> std::result::Result<Vec<NoteRecord>, NoteFoldError> {
    loop {
        let owners: HashMap<Hash, Hash> = applied
            .iter()
            .map(|event| match &event.op {
                NoteOp::Create { .. } => (event.id, event.id),
                NoteOp::Revise { note, .. } => (event.id, *note),
            })
            .collect();
        let before = applied.len();
        applied.retain(|event| match &event.op {
            NoteOp::Create { .. } => true,
            NoteOp::Revise { note, parents, .. } => {
                owners.get(note) == Some(note)
                    && parents
                        .iter()
                        .all(|parent| owners.get(parent) == Some(note))
            }
        });
        if applied.len() != before {
            continue;
        }

        match fold_notes(&applied) {
            Err(NoteFoldError::CyclicParent { revision, .. }) => {
                applied.retain(|event| event.id != revision);
            }
            result => return result,
        }
    }
}

/// The event store, and the key the node signs its events with.
#[derive(Debug)]
pub struct LocalEvents {
    store: EventStore,
    /// The node's own local-only claims (#1793): never signed, never synced.
    local: LocalClaims,
    key: SigningKey,
    /// Send-rate admission for this node's own chat sends (#3950).
    chat_rate: SendRate,
}

impl LocalEvents {
    /// Opens the store in `dir`, creating it if it does not exist. Events are signed with `key`.
    ///
    /// # Errors
    ///
    /// Fails if the store cannot be opened, for example while another process has it open.
    pub fn open(dir: &Path, key: SigningKey) -> Result<Self> {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        let store = EventStore::open(dir, WaitingLimits::default(), now_ms())?;
        let local = LocalClaims::open(dir).context("open the local claim store")?;
        Ok(Self {
            store,
            local,
            key,
            chat_rate: SendRate::default(),
        })
    }

    /// Runs one range-sync session over `conn` against this node's store: the session
    /// [`crate::event_store::sync::sync_over`] drives, with this store as its sync set.
    /// `initiator` is the side that dialed. Every event that arrives is decided by the
    /// store's own ingest, so a tampered or unsigned event is refused, never applied.
    ///
    /// # Errors
    ///
    /// Returns an error if the peer breaks the protocol, the session passes `deadline`,
    /// `gate` refuses the peer, or the store fails.
    pub async fn sync_session(
        &mut self,
        conn: &iroh::endpoint::Connection,
        initiator: bool,
        deadline: std::time::Duration,
        gate: &crate::pull_policy::PullGate,
    ) -> Result<crate::event_store::sync::Outcome> {
        crate::event_store::sync::sync_over(
            &mut self.store,
            conn,
            initiator,
            crate::event_store::range::Params::default(),
            now_ms(),
            deadline,
            gate,
        )
        .await
    }

    /// The account the node's events are signed as.
    #[must_use]
    pub fn author(&self) -> AuthorKey {
        *self.key.verifying_key().as_bytes()
    }

    /// Adds `block` to the operator block list (#2245), replacing any row
    /// that names the same kind and value. The enforcement seams
    /// (`id::abuse`, `id::blob_gate`, [`crate::pull_policy`])
    /// read the list live, so a block lands on the next check.
    ///
    /// # Errors
    ///
    /// Fails if the block row cannot be written.
    pub fn add_block(&self, block: &Block) -> Result<()> {
        self.store
            .add_block(block)
            .context("add the operator block row")
    }

    /// Removes the operator block naming (`kind`, `value`), returning
    /// whether a row was removed (#2245).
    ///
    /// # Errors
    ///
    /// Fails if the block row cannot be removed.
    pub fn remove_block(&self, kind: BlockKind, value: &[u8]) -> Result<bool> {
        self.store
            .remove_block(kind, value)
            .context("remove the operator block row")
    }

    /// Every operator block row, expired ones included, in (kind, value)
    /// order (#2245).
    ///
    /// # Errors
    ///
    /// Fails if the block rows cannot be read.
    pub fn blocks(&self) -> Result<Vec<Block>> {
        self.store.blocks().context("read the operator block rows")
    }

    /// Whether (`kind`, `value`) is blocked right now, judged against the
    /// wall clock exactly as the enforcement seams judge it (#2245).
    /// Reads go through [`crate::event_store::BlockList`], the same trait
    /// `id::abuse`, `id::blob_gate` and [`crate::pull_policy`]
    /// consult, so a web-written row and an enforced row are one row.
    #[must_use]
    pub fn is_blocked(&self, kind: BlockKind, value: &[u8]) -> bool {
        crate::event_store::BlockList::is_blocked(&self.store, kind, value)
    }

    /// The distinguishing form of [`LocalEvents::is_blocked`] (#8750):
    /// a failed lookup is preserved as
    /// [`BlockLookup::Indeterminate`] for the operator surfaces,
    /// instead of erased to "not blocked".
    #[must_use]
    pub fn lookup_block(&self, kind: BlockKind, value: &[u8]) -> BlockLookup {
        crate::event_store::BlockList::lookup(&self.store, kind, value)
    }

    /// Publishes `content` as a document of `media_type`, with a tag for each of `tags`. The event
    /// names the content by its BLAKE3 hash and size. The bytes are not kept here: they are a blob.
    /// Every tag is checked before anything is recorded, so a bad tag records nothing.
    ///
    /// # Errors
    ///
    /// Fails if the media type or a tag is refused, or the store cannot record the events.
    pub fn publish(
        &mut self,
        media_type: &str,
        content: &[u8],
        tags: &[(String, Option<String>)],
    ) -> Result<Published> {
        let document = DocumentRef::for_bytes(media_type, content)?;
        let mut seen = HashSet::with_capacity(tags.len());
        for (name, value) in tags {
            check_tag(name, value.as_deref())?;
            if !seen.insert((name, value)) {
                bail!("a publish names the same tag twice");
            }
        }
        let id = self.record(
            DOCUMENT_KIND,
            None,
            None,
            Some(document.clone()),
            None,
            None,
            None,
            &[],
        )?;
        let mut claims = Vec::with_capacity(tags.len());
        for (name, value) in tags {
            claims.push(self.set_tag(id, name, value.as_deref())?);
        }
        Ok(Published {
            id,
            document,
            claims,
        })
    }

    /// Sends `message` as a `chat.message.v1` event in the channel `group`, and returns
    /// the event's ID.
    ///
    /// Admission is refused before anything is signed when the node's key is not a member
    /// of the group under its policy ([`EventStore::roster`]), and when the node's sends
    /// inside the trailing rate window already number the configured maximum
    /// ([`SendRate`], the world-chat starting configuration of 50 per 10 seconds, #3950).
    ///
    /// # Errors
    ///
    /// Fails if the message is refused by [`ChatMessage::check`], the node is not a
    /// member, the send rate is exhausted, or the store cannot record the event.
    pub fn send_chat(&mut self, group: Hash, message: ChatMessage) -> Result<Hash> {
        message.check()?;
        let author = self.author();
        if !self.store.roster(&group)?.contains(&author) {
            bail!(
                "the node's key is not a member of the channel group, so its chat write is refused at admission"
            );
        }
        if !self.chat_rate.admit(now_ms()) {
            bail!(
                "the chat send rate is exhausted: at most {} messages in {} ms",
                id_store::chat::SEND_RATE_MAX,
                id_store::chat::SEND_RATE_WINDOW_MS
            );
        }
        self.record(
            CHAT_MESSAGE_KIND,
            Some(group),
            None,
            None,
            None,
            None,
            Some(message),
            &[],
        )
    }

    /// The chat timeline of `group` as this node may see it: the store's read, gated on
    /// this node's membership and cut by the group's history policy (see
    /// [`EventStore::chat_timeline`]).
    ///
    /// # Errors
    ///
    /// Fails if the node is not a member of the group, or the store cannot be read.
    pub fn chat_timeline(&self, group: Hash) -> Result<Vec<ChatRecord>> {
        Ok(self.store.chat_timeline(&group, &self.author())?)
    }

    /// Sets the tag `name` on `subject`, with `value` if given. The new claim's ID is the event's.
    ///
    /// The claim is shared: it is signed into the event store and published to peers, exactly as
    /// before scopes existed. Use [`LocalEvents::set_tag_scoped`] to choose the scope.
    ///
    /// # Errors
    ///
    /// Fails if the name or value is refused, or the store cannot record the event.
    pub fn set_tag(&mut self, subject: Hash, name: &str, value: Option<&str>) -> Result<Hash> {
        self.set_tag_scoped(subject, name, value, TagScope::Shared)
    }

    /// Sets the tag `name` on `subject`, with `value` if given, at the chosen `scope` (#1793).
    ///
    /// A [`TagScope::Shared`] claim is a signed `tag.set` event, exactly as [`LocalEvents::set_tag`]
    /// records it, and its ID is the event's. A [`TagScope::Local`] claim is never signed and never
    /// enters the event store, so it never reaches another node; its ID is derived from its content
    /// ([`id_store::tag_claims::LocalClaim::id_for`]). Both kinds are returned by
    /// [`LocalEvents::tags_on`], [`LocalEvents::search`] and [`LocalEvents::review_tags`], each
    /// carrying its scope.
    ///
    /// # Errors
    ///
    /// Fails if the name or value is refused, or the store cannot record the event (shared) or
    /// write the local claim file (local).
    pub fn set_tag_scoped(
        &mut self,
        subject: Hash,
        name: &str,
        value: Option<&str>,
        scope: TagScope,
    ) -> Result<Hash> {
        match scope {
            TagScope::Local => Ok(self.local.set(&self.author(), subject, name, value)?),
            TagScope::Shared => {
                let op = TagOp::Set {
                    subject,
                    name: name.to_owned(),
                    value: value.map(str::to_owned),
                };
                self.record(SET_KIND, None, Some(op), None, None, None, None, &[])
            }
        }
    }

    /// Changes the scope of every live claim named `name` on `subject` that this node authored.
    ///
    /// Moving a claim to [`TagScope::Local`] records a local claim and then clears the shared one;
    /// moving it to [`TagScope::Shared`] records a shared claim and then removes the local one. The
    /// claim keeps its name and value, so the move changes only where it lives. Claims authored by
    /// another node are not touched: this node cannot rewrite another node's signed claim, and a
    /// local claim is this node's own by construction.
    ///
    /// Returns the number of claims moved.
    ///
    /// # Errors
    ///
    /// Fails if the store cannot be read or written, or the local claim file cannot be written.
    pub fn set_tag_scope(&mut self, subject: Hash, name: &str, scope: TagScope) -> Result<usize> {
        let me = self.author();
        let mut moved = 0;
        let shared: Vec<Claim> = self
            .store
            .tag_claims()?
            .into_iter()
            .filter(|claim| claim.subject == subject && claim.name == name && claim.author == me)
            .collect();
        match scope {
            TagScope::Local => {
                for claim in &shared {
                    self.local
                        .set(&me, subject, &claim.name, claim.value.as_deref())?;
                    moved += 1;
                }
                if !shared.is_empty() {
                    self.clear_claim_ids(&shared.iter().map(|claim| claim.id).collect::<Vec<_>>())?;
                }
            }
            TagScope::Shared => {
                let local = self.local.on_subject(&subject);
                for claim in local
                    .iter()
                    .filter(|claim| claim.name == name && claim.author == me)
                {
                    self.set_tag(subject, &claim.name, claim.value.as_deref())?;
                    moved += 1;
                }
                if moved > 0 {
                    self.local.clear(&me, subject, name)?;
                }
            }
        }
        Ok(moved)
    }

    /// Clears only the named claim IDs, in batches that fit the wire limit. Used by scope moves
    /// so changing this node's scope cannot accidentally clear another author's claims with the
    /// same name.
    fn clear_claim_ids(&mut self, claims: &[Hash]) -> Result<()> {
        let author = self.author();
        let required: Vec<Hash> = self
            .frontier()?
            .into_iter()
            .find(|head| head.author == author)
            .map(|head| vec![head.event])
            .unwrap_or_default();
        for group in claims.chunks(MAX_CLEAR_CLAIMS) {
            self.record(
                CLEAR_KIND,
                None,
                Some(TagOp::Clear {
                    claims: group.to_vec(),
                }),
                None,
                None,
                None,
                None,
                &required,
            )?;
        }
        Ok(())
    }

    /// Clears every live claim named `name` on `subject`, whoever set it, by recording
    /// `tag.clear` markers naming them. Nothing to clear is an error. A marker hides a claim
    /// in the default view only when this node's key authored the claim or is an admin of the
    /// group owning the subject (#2090); the markers are recorded either way.
    ///
    /// One `tag.clear` event names at most [`MAX_CLEAR_CLAIMS`] claims, so a name with more live
    /// claims than that is cleared by several events, in the order [`EventStore::tag_claims`]
    /// returns the claims. The returned count is the total cleared across all of those events,
    /// and the returned ID is the final event's — the one whose recording completes the clear.
    ///
    /// The events are append-only: if recording one of them fails, the ones already recorded
    /// stay, the error is returned, and calling this again clears what is left.
    ///
    /// A claim is removed only when it sits in the clear's causal past, so each sub-clear
    /// names the current head of every author whose claims it names as a required head of
    /// its `after` frontier (see [`LocalEvents::bounded_after`]). When the claims span more
    /// authors than the frontier cap ([`MAX_AFTER`]) or more claims than one clear may name
    /// ([`MAX_CLEAR_CLAIMS`]), the clear is split: several `tag.clear` events are recorded,
    /// partitioning the claims by author into groups that fit both caps, in author order.
    /// The returned count is the total across all sub-clears, and the returned event ID is
    /// the last sub-clear's -- the one after which every named claim is cleared.
    ///
    /// # Errors
    ///
    /// Fails if no live claim matches, or the store cannot record an event.
    pub fn clear_tag(&mut self, subject: Hash, name: &str) -> Result<(usize, Hash)> {
        // Local claims are this node's own and are removed from the local store; shared claims
        // are cleared by `tag.clear` events, as before (#1793).
        let local_ids: Vec<Hash> = self
            .local
            .on_subject(&subject)
            .into_iter()
            .filter(|claim| claim.name == name)
            .map(|claim| claim.id)
            .collect();
        let removed_local = self.local.clear(&self.author(), subject, name)?;
        let claims: Vec<Claim> = self
            .store
            .tag_claims()?
            .into_iter()
            .filter(|claim| claim.subject == subject && claim.name == name)
            .collect();
        if claims.is_empty() {
            if removed_local > 0 {
                // Nothing shared to clear: the local claims are gone, and the last local claim's
                // ID stands in for the event ID a shared clear would have returned.
                let last = local_ids.last().copied().unwrap_or(subject);
                return Ok((removed_local, last));
            }
            bail!("no live claim on {} is named {name}", subject.to_hex());
        }
        let count = claims.len() + removed_local;
        // The current head of each author, for the required tier of each sub-clear.
        let heads: HashMap<AuthorKey, Hash> = self
            .frontier()?
            .into_iter()
            .map(|head| (head.author, head.event))
            .collect();
        // Partition the claims by author into groups fitting both caps, in author order.
        let mut by_author: HashMap<AuthorKey, Vec<Claim>> = HashMap::new();
        for claim in claims {
            by_author.entry(claim.author).or_default().push(claim);
        }
        let mut authors: Vec<AuthorKey> = by_author.keys().copied().collect();
        authors.sort_unstable();
        let mut groups: Vec<Vec<Claim>> = Vec::new();
        let mut group: Vec<Claim> = Vec::new();
        let mut group_authors = 0usize;
        for author in authors {
            let authored = by_author.remove(&author).unwrap_or_default();
            // One author alone can hold more claims than a single clear may
            // name (#1720), so pack each author's claims in cap-sized segments.
            for segment in authored.chunks(MAX_CLEAR_CLAIMS) {
                if !group.is_empty()
                    && (group_authors == MAX_AFTER
                        || group.len() + segment.len() > MAX_CLEAR_CLAIMS)
                {
                    groups.push(std::mem::take(&mut group));
                    group_authors = 0;
                }
                group_authors += 1;
                group.extend(segment.iter().cloned());
            }
        }
        if !group.is_empty() {
            groups.push(group);
        }
        let mut last = None;
        for group in groups {
            let mut required: Vec<Hash> = Vec::new();
            let mut seen: HashSet<AuthorKey> = HashSet::new();
            for claim in &group {
                if seen.insert(claim.author) {
                    if let Some(head) = heads.get(&claim.author) {
                        required.push(*head);
                    }
                }
            }
            let ids: Vec<Hash> = group.iter().map(|claim| claim.id).collect();
            last = Some(self.record(
                CLEAR_KIND,
                None,
                Some(TagOp::Clear { claims: ids }),
                None,
                None,
                None,
                None,
                &required,
            )?);
        }
        let event = last.context("a clear with claims records at least one event")?;
        Ok((count, event))
    }

    /// The live claims on `subject` in the default view, sorted by claim ID.
    ///
    /// # Errors
    ///
    /// Fails if the store cannot be read.
    pub fn tags_on(&self, subject: Hash) -> Result<Vec<Claim>> {
        let mut claims: Vec<Claim> = self
            .store
            .tag_claims()?
            .into_iter()
            .filter(|claim| claim.subject == subject)
            .collect();
        claims.extend(
            self.local
                .on_subject(&subject)
                .iter()
                .map(LocalClaim::as_claim),
        );
        claims.sort_by_key(|claim| *claim.id.as_bytes());
        Ok(claims)
    }

    /// Every live claim this node can see — shared and local — for the review listing (#1793),
    /// sorted by scope then claim ID, so the local-only claims are reviewable on their own.
    ///
    /// # Errors
    ///
    /// Fails if the store cannot be read.
    pub fn review_tags(&self) -> Result<Vec<Claim>> {
        let mut claims = self.store.tag_claims()?;
        claims.extend(self.local.claims().iter().map(LocalClaim::as_claim));
        claims.sort_by_key(|claim| (claim.scope == TagScope::Local, *claim.id.as_bytes()));
        Ok(claims)
    }

    /// The live claims that match `name`, and `value` if given, as [`id_store::search`] returns them.
    ///
    /// # Errors
    ///
    /// Fails if the store cannot be read.
    pub fn search(&self, name: &str, value: Option<&str>) -> Result<Vec<Hit>> {
        let matching = value.map_or(ValueMatch::Any, ValueMatch::Exact);
        let mut hits = search_store(&self.store, name, matching)?;
        // Local claims are this node's own, so they are part of this node's search answer (#1793).
        // They are folded in per subject, so a subject with both kinds reports one hit.
        for claim in self.local.matching(name, value) {
            let entry = claim.as_claim();
            match hits.iter_mut().find(|hit| hit.subject == claim.subject) {
                Some(hit) => hit.claims.push(entry),
                None => hits.push(Hit {
                    subject: claim.subject,
                    claims: vec![entry],
                }),
            }
        }
        Ok(hits)
    }

    /// Creates a note with `content` as its first content. The returned ID is the note's
    /// stable ID: the `note.create` event's ID. The bytes are not kept here: they are a
    /// blob, named by the content reference the event carries.
    ///
    /// # Errors
    ///
    /// Fails if the media type is refused, or the store cannot record the event.
    pub fn create_note(&mut self, media_type: &str, content: &[u8]) -> Result<Hash> {
        let content = DocumentRef::for_bytes(media_type, content)?;
        let op = NoteOp::Create { content };
        check_note_op(NOTE_CREATE_KIND, &op)?;
        self.record(
            NOTE_CREATE_KIND,
            None,
            None,
            None,
            None,
            Some(op),
            None,
            &[],
        )
    }

    /// Adds one revision with `content` to the note `note`, building on `parents`. The
    /// parents must be revisions of that note already in the store; naming two heads is
    /// an explicit merge. The new revision's ID is returned.
    ///
    /// # Errors
    ///
    /// Fails if the note is not in the store, a parent is not a revision of it, the
    /// operation breaks a note rule, or the store cannot record the event.
    pub fn revise_note(
        &mut self,
        note: Hash,
        media_type: &str,
        content: &[u8],
        parents: &[Hash],
    ) -> Result<Hash> {
        let record = self.note_history(note)?;
        for parent in parents {
            if record.revision(*parent).is_none() {
                bail!(
                    "{} is not a revision of note {}",
                    parent.to_hex(),
                    note.to_hex()
                );
            }
        }
        // Reserve the parent authors' current heads in the new event's
        // `after` frontier (PR #3908 Codex P2): past the frontier cap, a
        // parent author omitted from `after` breaks dependency closure --
        // a receiver could apply this revision before its note or parents
        // arrive and then fail its note queries with UnknownNote or
        // UnknownParent. `tag.clear` already reserves claim authors' heads
        // the same way.
        let parent_authors: HashSet<AuthorKey> = parents
            .iter()
            .filter_map(|parent| record.revision(*parent))
            .map(|revision| revision.author)
            .collect();
        let required: Vec<Hash> = self
            .frontier()?
            .into_iter()
            .filter(|head| parent_authors.contains(&head.author))
            .map(|head| head.event)
            .collect();
        let content = DocumentRef::for_bytes(media_type, content)?;
        let op = NoteOp::Revise {
            note,
            content,
            parents: parents.to_vec(),
        };
        check_note_op(NOTE_REVISE_KIND, &op)?;
        self.record(
            NOTE_REVISE_KIND,
            None,
            None,
            None,
            None,
            Some(op),
            None,
            &required,
        )
    }

    /// Every note in the store, in note ID order, each with its revisions and heads.
    ///
    /// # Errors
    ///
    /// Fails if the store cannot be read, or its note events do not fold.
    pub fn notes(&self) -> Result<Vec<NoteRecord>> {
        let (entries, _) = self.log()?;
        let applied: Vec<AppliedNote> = entries
            .values()
            .filter_map(|entry| match entry {
                Entry::NoteCreate {
                    id,
                    author,
                    content,
                } => Some(AppliedNote {
                    id: *id,
                    author: *author,
                    op: NoteOp::Create {
                        content: content.clone(),
                    },
                }),
                Entry::NoteRevise {
                    id,
                    author,
                    note,
                    content,
                    parents,
                } => Some(AppliedNote {
                    id: *id,
                    author: *author,
                    op: NoteOp::Revise {
                        note: *note,
                        content: content.clone(),
                        parents: parents.clone(),
                    },
                }),
                _ => None,
            })
            .collect();
        // Ingest checks only a revision's shape, not whether its note graph
        // can be folded. Quarantine invalid remote revisions so one bad event
        // cannot disable notes(), note_history(), or revise_note() for the
        // healthy notes in this store. The events stay in the store.
        Ok(fold_remote_notes(applied)?)
    }

    /// The note `note`: its revisions in ID order and its current heads.
    ///
    /// # Errors
    ///
    /// Fails if `note` is not the ID of a `note.create` in the store, or the store
    /// cannot be read.
    pub fn note_history(&self, note: Hash) -> Result<NoteRecord> {
        self.notes()?
            .into_iter()
            .find(|record| record.id == note)
            .with_context(|| {
                format!(
                    "{} is not a note: no note.create has that event ID",
                    note.to_hex()
                )
            })
    }

    /// The head of each author's chain, in author order.
    ///
    /// # Errors
    ///
    /// Fails if the store cannot be read.
    pub fn frontier(&self) -> Result<Vec<Head>> {
        Ok(self
            .store
            .chains()?
            .into_iter()
            .filter_map(|(author, chain)| {
                chain.head.map(|event| Head {
                    author,
                    event,
                    seq: u64::from(chain.next_seq).saturating_sub(1),
                })
            })
            .collect())
    }

    /// The events on `document` other than its `document.put`: the tags set on it, the clears of
    /// those tags, and any removal. They are in causal order.
    ///
    /// # Errors
    ///
    /// Fails if `document` is not a document, or the store cannot be read.
    pub fn events_on(&self, document: Hash) -> Result<Vec<Entry>> {
        Ok(self
            .history(document)?
            .into_iter()
            .filter(|entry| !matches!(entry, Entry::Document { .. }))
            .collect())
    }

    /// The document's events in causal order: its `document.put` first, then the events of
    /// [`LocalEvents::events_on`]. Each event comes after the events it names as its causes.
    ///
    /// # Errors
    ///
    /// Fails if `document` is not the ID of a `document.put`, or the store cannot be read.
    pub fn history(&self, document: Hash) -> Result<Vec<Entry>> {
        let (entries, parents) = self.log()?;
        if !matches!(entries.get(&document), Some(Entry::Document { .. })) {
            bail!(
                "{} is not a document: no document.put has that event ID",
                document.to_hex()
            );
        }
        let order = causal_order(&parents);
        // The claims set on this document. A clear of any of them belongs to the document too.
        let claims_on: HashSet<Hash> = order
            .iter()
            .filter_map(|id| match entries.get(id) {
                Some(Entry::Set { subject, .. }) if *subject == document => Some(*id),
                _ => None,
            })
            .collect();
        Ok(order
            .iter()
            .filter_map(|id| entries.get(id))
            .filter(|entry| match entry {
                Entry::Document { id, .. } => *id == document,
                Entry::Set { subject, .. } => *subject == document,
                Entry::Clear { claims, .. } => claims.iter().any(|claim| claims_on.contains(claim)),
                Entry::Remove {
                    document: removed, ..
                } => *removed == document,
                Entry::NoteCreate { .. } | Entry::NoteRevise { .. } => false,
            })
            .cloned()
            .collect())
    }

    /// Every applied event: its entry if it is a document or tag event, and its parents, which are
    /// its `after` and its backlink.
    fn log(&self) -> Result<Log> {
        let mut entries = HashMap::new();
        let mut parents = HashMap::new();
        for (author, _) in self.store.chains()? {
            for event in self.store.applied_events(&author, None, None, usize::MAX)? {
                parents.insert(event.id, parents_of(&event));
                if let Some(entry) = entry_of(&event) {
                    entries.insert(event.id, entry);
                }
            }
        }
        Ok((entries, parents))
    }

    /// The heads of the other authors, which a new event names as its `after`. The node's own head
    /// is the new event's backlink, so it is not named twice.
    fn other_heads(&self) -> Result<Vec<Hash>> {
        let author = self.author();
        Ok(self
            .frontier()?
            .into_iter()
            .filter(|head| head.author != author)
            .map(|head| head.event)
            .collect())
    }

    /// The `after` frontier for a new event: a dependency frontier, not an author census
    /// (#1736). `after` is capped at [`MAX_AFTER`] hashes by the wire format, and events naming
    /// more are refused by every existing node, so the frontier must fit the cap no matter how
    /// many authors the store holds.
    ///
    /// When every other author's head fits (the common case), all of them are named, in
    /// frontier order, exactly as before. Past the cap, the frontier is built in two tiers:
    /// first the `required` heads -- for a `tag.clear`, the current head of each author of a
    /// named claim, because a claim is removed only when it sits in the clear's causal past,
    /// which the author's head in `after` guarantees through that author's backlink chain --
    /// then the remaining slots fill with the other authors' heads in frontier order.
    /// Heads past the cap are simply not named: the events they head stay in the store, and
    /// naming a head is about causal past, not about membership, so a later event loses
    /// nothing but the claim of having seen them.
    fn bounded_after(&self, required: &[Hash]) -> Result<Vec<Hash>> {
        let heads = self.other_heads()?;
        if heads.len() <= MAX_AFTER {
            return Ok(heads);
        }
        let current: HashSet<Hash> = heads.iter().copied().collect();
        let mut after: Vec<Hash> = Vec::with_capacity(MAX_AFTER);
        let mut named: HashSet<Hash> = HashSet::new();
        for head in required {
            if after.len() == MAX_AFTER {
                break;
            }
            if current.contains(head) && named.insert(*head) {
                after.push(*head);
            }
        }
        for head in heads {
            if after.len() == MAX_AFTER {
                break;
            }
            if named.insert(head) {
                after.push(head);
            }
        }
        Ok(after)
    }

    /// Signs one event with the node's key, after a bounded frontier of the other authors'
    /// heads (see [`LocalEvents::bounded_after`]), and ingests it. `required` names heads the
    /// frontier must include when it is capped; most kinds require none, a `tag.clear`
    /// requires its claim authors' heads, and a `note.revise` its parent authors' heads.
    /// The store must apply it, or the call fails.
    fn record(
        &mut self,
        kind: &str,
        group: Option<Hash>,
        tag: Option<TagOp>,
        document: Option<DocumentRef>,
        removes: Option<Hash>,
        note: Option<NoteOp>,
        chat: Option<ChatMessage>,
        required: &[Hash],
    ) -> Result<Hash> {
        let author = self.author();
        let after = self.bounded_after(required)?;
        let chain = self.store.chain(&author)?.unwrap_or_default();
        let ext = ChainExt {
            kind: kind.to_owned(),
            group,
            after,
            tag,
            document,
            removes,
            note,
            link: None,
            checkpoint: None,
            rotation: None,
            chat,
        };
        let event = sign(&self.key, chain.next_seq, chain.head, None, ext)?;
        match self.store.ingest(&event.bytes, now_ms())? {
            Ingest::Accepted { id, .. } => Ok(id),
            refused => bail!("the store did not apply the {kind} event: {refused:?}"),
        }
    }
}

/// Checks a tag's name and value as the store will, before anything is signed. The subject is not
/// part of the check, so a placeholder stands in for it.
fn check_tag(name: &str, value: Option<&str>) -> Result<()> {
    let op = TagOp::Set {
        subject: Hash::from_bytes([0; 32]),
        name: name.to_owned(),
        value: value.map(str::to_owned),
    };
    check_op(SET_KIND, &op)?;
    Ok(())
}

/// The entry of a document or tag event, or `None` for any other event.
fn entry_of(event: &Event) -> Option<Entry> {
    let id = event.id;
    let author = *event.header.verifying_key.as_bytes();
    let ext = &event.header.extensions;
    match (
        ext.kind.as_str(),
        &ext.tag,
        &ext.document,
        ext.removes,
        &ext.note,
    ) {
        (DOCUMENT_KIND, _, Some(document), _, _) => Some(Entry::Document {
            id,
            author,
            document: document.clone(),
        }),
        (NOTE_CREATE_KIND, _, _, _, Some(NoteOp::Create { content })) => Some(Entry::NoteCreate {
            id,
            author,
            content: content.clone(),
        }),
        (
            NOTE_REVISE_KIND,
            _,
            _,
            _,
            Some(NoteOp::Revise {
                note,
                content,
                parents,
            }),
        ) => Some(Entry::NoteRevise {
            id,
            author,
            note: *note,
            content: content.clone(),
            parents: parents.clone(),
        }),
        (
            SET_KIND,
            Some(TagOp::Set {
                subject,
                name,
                value,
                ..
            }),
            _,
            _,
            _,
        ) => Some(Entry::Set {
            id,
            author,
            subject: *subject,
            name: name.clone(),
            value: value.clone(),
        }),
        (CLEAR_KIND, Some(TagOp::Clear { claims }), _, _, _) => Some(Entry::Clear {
            id,
            author,
            claims: claims.clone(),
        }),
        (REMOVAL_KIND, _, _, Some(document), _) => Some(Entry::Remove {
            id,
            author,
            document,
        }),
        _ => None,
    }
}

/// The events an event names as its causes: its `after` hashes and its backlink.
fn parents_of(event: &Event) -> Vec<Hash> {
    let mut named = event.header.extensions.after.clone();
    named.extend(event.header.backlink);
    named
}

/// Every event, each after the events it names. The walk starts from the lowest ID and visits
/// parents in ID order, so the order is the same whatever order the events were stored in.
fn causal_order(parents: &HashMap<Hash, Vec<Hash>>) -> Vec<Hash> {
    let mut roots: Vec<Hash> = parents.keys().copied().collect();
    roots.sort_by_key(|id| *id.as_bytes());
    let mut visited: HashSet<Hash> = HashSet::new();
    let mut order = Vec::with_capacity(parents.len());
    for root in roots {
        let mut stack = vec![(root, false)];
        while let Some((id, expanded)) = stack.pop() {
            if expanded {
                order.push(id);
                continue;
            }
            if !visited.insert(id) {
                continue;
            }
            stack.push((id, true));
            let mut named = parents.get(&id).cloned().unwrap_or_default();
            named.sort_by_key(|parent| *parent.as_bytes());
            // Pushed in reverse, so the lowest ID is popped, and so expanded, first.
            for parent in named.into_iter().rev() {
                if !visited.contains(&parent) {
                    stack.push((parent, false));
                }
            }
        }
    }
    order
}

/// The wall clock in milliseconds, for the store's waiting limits.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn id(byte: u8) -> Hash {
        Hash::from_bytes([byte; 32])
    }

    fn key(byte: u8) -> SigningKey {
        SigningKey::from_bytes(&[byte; 32])
    }

    /// Opens a one-shot node for foreign author `byte` in the shared store
    /// and has it set one claim named `name` on `subject`.
    fn foreign_claim(dir: &Path, byte: u8, subject: Hash, name: &str) {
        let mut events = LocalEvents::open(dir, key(byte)).unwrap();
        events.set_tag(subject, name, Some("x")).unwrap();
    }

    #[test]
    fn test_record_still_works_with_more_authors_than_the_after_cap() {
        // Regression for #1736: with 33+ other authors in the store, the
        // uncapped frontier made every record() fail validation, so the
        // node could never write again.
        for foreign in [33u8, 100] {
            let dir = tempfile::TempDir::new().unwrap();
            let subject = id(200);
            for byte in 1..=foreign {
                foreign_claim(dir.path(), byte, subject, "topic");
            }
            let mut events = LocalEvents::open(dir.path(), key(0)).unwrap();
            let published = events.publish("text/plain", b"hello", &[]).unwrap();
            events.set_tag(published.id, "topic", Some("mine")).unwrap();
            let (cleared, _) = events.clear_tag(published.id, "topic").unwrap();
            assert_eq!(cleared, 1);
        }
    }

    #[test]
    fn test_clear_spanning_more_authors_than_the_cap_clears_exactly_its_claims() {
        // Regression for #1736: 40 authors hold a claim named "topic" on the
        // subject -- more distinct authors than MAX_AFTER, so the clear must
        // split by author. The markers name exactly those claims, and each
        // named claim sits in its marker's causal past, which is what removes
        // a claim in the fold (id_store::tag_claims). A claim with a different
        // name on the same subject, and a same-name claim on a different
        // subject, are named by no marker.
        //
        // The view the claims are read through is role-aware (#2090, the
        // owner decision on #185): the default view hides a claim only when
        // the marker's poster is the claim's author or an admin of the group
        // owning the subject. The clearing node here (key 0) is neither, and
        // the subject has no owning group, so its markers do not hide the
        // claims in tags_on -- the naming and causal-past assertions are the
        // part #1736 broke, and they are what this test pins.
        let dir = tempfile::TempDir::new().unwrap();
        let subject = id(201);
        let other_subject = id(202);
        for byte in 1..=40u8 {
            foreign_claim(dir.path(), byte, subject, "topic");
        }
        foreign_claim(dir.path(), 1, subject, "other");
        foreign_claim(dir.path(), 2, other_subject, "topic");
        let mut events = LocalEvents::open(dir.path(), key(0)).unwrap();
        let (cleared, _) = events.clear_tag(subject, "topic").unwrap();
        assert_eq!(cleared, 40);

        let (entries, parents) = events.log().unwrap();
        let mut claim_author: HashMap<Hash, AuthorKey> = HashMap::new();
        let mut topic_claims: HashSet<Hash> = HashSet::new();
        for entry in entries.values() {
            if let Entry::Set {
                id,
                author,
                subject: on,
                name,
                ..
            } = entry
            {
                claim_author.insert(*id, *author);
                if *on == subject && name == "topic" {
                    topic_claims.insert(*id);
                }
            }
        }
        assert_eq!(topic_claims.len(), 40);
        let mut named: HashSet<Hash> = HashSet::new();
        let mut markers = 0usize;
        for entry in entries.values() {
            let Entry::Clear { id, claims, .. } = entry else {
                continue;
            };
            markers += 1;
            // One marker names claims of at most MAX_AFTER distinct authors:
            // the split respects the same cap as the frontier it rides on.
            let authors: HashSet<AuthorKey> =
                claims.iter().map(|claim| claim_author[claim]).collect();
            assert!(authors.len() <= MAX_AFTER, "a marker spans {authors:?}");
            // Each named claim sits in this marker's causal past.
            let seen = parents
                .get(id)
                .map(|named_parents| id_store::tag_claims::causal_past(named_parents, &parents))
                .unwrap_or_default();
            for claim in claims {
                assert!(named.insert(*claim), "a claim is named by two markers");
                assert!(
                    seen.contains(claim),
                    "a named claim is outside its marker's causal past"
                );
            }
        }
        // The clear split, and together the markers name exactly the 40
        // "topic" claims on the subject -- nothing else.
        assert!(markers > 1, "the clear did not split: {markers} marker(s)");
        assert_eq!(named, topic_claims);
        // The default view (#2090): this poster's markers hide nothing, so
        // every claim on the subject is still listed.
        assert_eq!(events.tags_on(subject).unwrap().len(), 41);
        assert_eq!(events.tags_on(other_subject).unwrap().len(), 1);
    }

    #[test]
    fn test_notes_revise_concurrently_and_merge_explicitly() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut events = LocalEvents::open(dir.path(), key(7)).unwrap();
        let note = events.create_note("text/markdown", b"first").unwrap();
        let a = events
            .revise_note(note, "text/markdown", b"branch a", &[note])
            .unwrap();
        let b = events
            .revise_note(note, "text/markdown", b"branch b", &[note])
            .unwrap();
        let record = events.note_history(note).unwrap();
        assert_eq!(record.id, note);
        assert_eq!(record.heads, {
            let mut heads = vec![a, b];
            heads.sort_by_key(|id| *id.as_bytes());
            heads
        });
        let merged = events
            .revise_note(note, "text/markdown", b"merged", &[a, b])
            .unwrap();
        let record = events.note_history(note).unwrap();
        assert_eq!(record.heads, vec![merged]);
        assert_eq!(record.revisions.len(), 4);
        // A parent that is not a revision of this note is refused before signing.
        assert!(
            events
                .revise_note(note, "text/markdown", b"bad", &[id(222)])
                .is_err()
        );
    }

    #[test]
    fn test_invalid_remote_revision_does_not_disable_valid_notes() {
        // PR #3908 Codex P1: a signed note.revise naming an unknown note
        // is applied by ingest (shape is all it checks); it must not make
        // notes(), note_history(), or revise_note() fail for valid notes.
        let dir = tempfile::TempDir::new().unwrap();
        let mut events = LocalEvents::open(dir.path(), key(7)).unwrap();
        let note = events.create_note("text/markdown", b"first").unwrap();
        let foreign = sign(
            &key(9),
            0,
            None,
            None,
            ChainExt {
                kind: NOTE_REVISE_KIND.to_owned(),
                group: None,
                after: Vec::new(),
                tag: None,
                document: None,
                removes: None,
                note: Some(NoteOp::Revise {
                    note: id(222),
                    content: DocumentRef::for_bytes("text/markdown", b"bogus").unwrap(),
                    parents: vec![id(223)],
                }),
                link: None,
                checkpoint: None,
                rotation: None,

                chat: None,
            },
        )
        .unwrap();
        let ingested = events.store.ingest(&foreign.bytes, now_ms()).unwrap();
        assert!(matches!(ingested, Ingest::Accepted { .. }), "{ingested:?}");
        let notes = events.notes().unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].id, note);
        assert!(events.note_history(note).is_ok());
        assert!(
            events
                .revise_note(note, "text/markdown", b"second", &[note])
                .is_ok()
        );
    }

    #[test]
    fn test_cyclic_remote_revisions_do_not_disable_healthy_notes() {
        fn applied_create(byte: u8) -> AppliedNote {
            AppliedNote {
                id: id(byte),
                author: [1; 32],
                op: NoteOp::Create {
                    content: DocumentRef::for_bytes("text/markdown", b"note").unwrap(),
                },
            }
        }

        fn applied_revise(event: u8, note: u8, parents: &[u8]) -> AppliedNote {
            AppliedNote {
                id: id(event),
                author: [1; 32],
                op: NoteOp::Revise {
                    note: id(note),
                    content: DocumentRef::for_bytes("text/markdown", b"revision").unwrap(),
                    parents: parents.iter().map(|parent| id(*parent)).collect(),
                },
            }
        }

        for cyclic in [
            vec![applied_revise(21, 20, &[21])],
            vec![applied_revise(21, 20, &[22]), applied_revise(22, 20, &[21])],
            vec![
                applied_revise(21, 20, &[22]),
                applied_revise(22, 20, &[23]),
                applied_revise(23, 20, &[21]),
            ],
        ] {
            let mut events = vec![
                applied_create(10),
                applied_revise(11, 10, &[10]),
                applied_create(20),
            ];
            events.extend(cyclic);
            events.push(applied_revise(24, 20, &[21]));

            let notes = fold_remote_notes(events).unwrap();
            assert_eq!(notes.len(), 2);
            assert_eq!(notes[0].id, id(10));
            assert_eq!(notes[0].revisions.len(), 2);
            assert_eq!(notes[1].id, id(20));
            assert_eq!(notes[1].revisions.len(), 1);
        }
    }

    #[test]
    fn test_revise_reserves_parent_author_head_past_the_after_cap() {
        // PR #3908 Codex P2: with more other authors than MAX_AFTER, the
        // parent author's head must still be named in the revision's
        // `after`, or a receiver could apply the revision before its
        // parents arrive.
        let dir = tempfile::TempDir::new().unwrap();
        let subject = id(210);
        for byte in 1..=40u8 {
            foreign_claim(dir.path(), byte, subject, "topic");
        }
        let mut parent_events = LocalEvents::open(dir.path(), key(99)).unwrap();
        let note = parent_events
            .create_note("text/markdown", b"first")
            .unwrap();
        drop(parent_events);
        let mut events = LocalEvents::open(dir.path(), key(0)).unwrap();
        let revision = events
            .revise_note(note, "text/markdown", b"second", &[note])
            .unwrap();
        let author = events.author();
        let applied = events
            .store
            .applied_events(&author, None, None, usize::MAX)
            .unwrap();
        let event = applied.iter().find(|e| e.id == revision).unwrap();
        assert!(event.header.extensions.after.contains(&note));
    }

    #[test]
    fn test_causal_order_puts_every_event_after_its_parents() {
        // 1 and 2 are roots. 3 names 1, and 4 names 3 and 2. 5 names nothing it did not see.
        let parents = HashMap::from([
            (id(1), Vec::new()),
            (id(2), Vec::new()),
            (id(3), vec![id(1)]),
            (id(4), vec![id(3), id(2)]),
            (id(5), Vec::new()),
        ]);
        let order = causal_order(&parents);
        assert_eq!(order.len(), 5);
        let position = |event: Hash| order.iter().position(|&found| found == event).unwrap();
        assert!(position(id(1)) < position(id(3)));
        assert!(position(id(3)) < position(id(4)));
        assert!(position(id(2)) < position(id(4)));
        // The same events in another map give the same order.
        assert_eq!(causal_order(&parents), order);
    }

    #[test]
    fn test_a_tag_that_the_store_would_refuse_is_refused_before_publish() {
        assert!(check_tag("topic", Some("plans")).is_ok());
        assert!(check_tag("", None).is_err());
        assert!(check_tag("e\u{301}", None).is_err());
    }

    #[test]
    fn test_publish_refuses_a_repeated_tag_pair_before_recording() {
        // Regression for #4772: a repeated exact (name, value) pair in one
        // publish call recorded two identical live claims. The batch must be
        // refused before anything is recorded, like the sibling validators
        // (tag.clear claim lists, ChainExt.after, note.revise parents).
        let (_dir, mut events) = open_events();
        assert!(events.frontier().unwrap().is_empty());
        for tags in [
            vec![
                ("city".to_owned(), Some("Seattle".to_owned())),
                ("city".to_owned(), Some("Seattle".to_owned())),
            ],
            vec![("city".to_owned(), None), ("city".to_owned(), None)],
        ] {
            assert!(events.publish("text/plain", b"hello", &tags).is_err());
            assert!(
                events.frontier().unwrap().is_empty(),
                "a refused publish records nothing"
            );
        }
    }

    #[test]
    fn test_publish_accepts_distinct_pairs_sharing_a_name() {
        // Same name with different values, and bare plus valued, are distinct
        // claims and must stay legal — only the exact pair repeats refuse.
        let (_dir, mut events) = open_events();
        let tags = vec![
            ("city".to_owned(), Some("Seattle".to_owned())),
            ("city".to_owned(), Some("Portland".to_owned())),
            ("city".to_owned(), None),
        ];
        let published = events.publish("text/plain", b"hello", &tags).unwrap();
        assert_eq!(published.claims.len(), 3);
        assert_eq!(events.tags_on(published.id).unwrap().len(), 3);
    }

    #[test]
    fn test_local_claim_is_reviewable_but_never_enters_the_shared_store() {
        let (_dir, mut events) = open_events();
        let subject = publish_subject(&mut events);
        let before = events.store.sync_entries().unwrap();

        let id = events
            .set_tag_scoped(subject, "city", Some("Oslo"), TagScope::Local)
            .unwrap();

        let visible = events.tags_on(subject).unwrap();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, id);
        assert_eq!(visible[0].scope, TagScope::Local);
        assert!(events.store.tag_claims().unwrap().is_empty());
        assert_eq!(events.store.sync_entries().unwrap(), before);
        assert_eq!(events.review_tags().unwrap(), visible);
        assert_eq!(
            events.search("city", Some("Oslo")).unwrap()[0].claims,
            visible
        );
    }

    #[test]
    fn test_scope_edit_moves_a_local_claim_to_shared_and_back() {
        let (_dir, mut events) = open_events();
        let subject = publish_subject(&mut events);
        let local_id = events
            .set_tag_scoped(subject, "city", Some("Oslo"), TagScope::Local)
            .unwrap();

        assert_eq!(
            events
                .set_tag_scope(subject, "city", TagScope::Shared)
                .unwrap(),
            1
        );
        let shared = events.tags_on(subject).unwrap();
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].scope, TagScope::Shared);
        assert_ne!(shared[0].id, local_id);
        assert!(events.local.claims().is_empty());
        assert_eq!(events.store.tag_claims().unwrap(), shared);

        assert_eq!(
            events
                .set_tag_scope(subject, "city", TagScope::Local)
                .unwrap(),
            1
        );
        let local = events.tags_on(subject).unwrap();
        assert_eq!(local.len(), 1);
        assert_eq!(local[0].scope, TagScope::Local);
        assert!(events.store.tag_claims().unwrap().is_empty());
    }

    #[test]
    fn test_scope_edit_to_shared_publishes_only_own_local_claims() {
        // Two identities can share one event store with different data
        // directories; the local store merged into this view can therefore
        // hold another author's local-only claims. Moving a tag to shared
        // must publish only this node's own claims and leave the foreign
        // local claim unpublished and intact (Codex review, PR #7344).
        let (_dir, mut events) = open_events();
        let subject = publish_subject(&mut events);
        events
            .set_tag_scoped(subject, "city", Some("Oslo"), TagScope::Local)
            .unwrap();
        let foreign: AuthorKey = [9; 32];
        events
            .local
            .set(&foreign, subject, "city", Some("Bergen"))
            .unwrap();

        assert_eq!(
            events
                .set_tag_scope(subject, "city", TagScope::Shared)
                .unwrap(),
            1
        );

        let shared = events.store.tag_claims().unwrap();
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].value.as_deref(), Some("Oslo"));
        let remaining = events.local.claims();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].author, foreign);
        assert_eq!(remaining[0].value.as_deref(), Some("Bergen"));
    }

    #[test]
    fn test_clear_tag_removes_local_claims_without_signing_a_clear() {
        let (_dir, mut events) = open_events();
        let subject = publish_subject(&mut events);
        let before = events.store.sync_entries().unwrap();
        events
            .set_tag_scoped(subject, "city", Some("Oslo"), TagScope::Local)
            .unwrap();

        let (count, _) = events.clear_tag(subject, "city").unwrap();

        assert_eq!(count, 1);
        assert!(events.tags_on(subject).unwrap().is_empty());
        assert_eq!(events.store.sync_entries().unwrap(), before);
    }

    fn open_events() -> (tempfile::TempDir, LocalEvents) {
        let dir = tempfile::tempdir().unwrap();
        let events = LocalEvents::open(dir.path(), SigningKey::from_bytes(&[7; 32])).unwrap();
        (dir, events)
    }

    fn publish_subject(events: &mut LocalEvents) -> Hash {
        events.publish("text/plain", b"hello", &[]).unwrap().id
    }

    fn set_claims(events: &mut LocalEvents, subject: Hash, name: &str, count: usize) {
        for i in 0..count {
            events.set_tag(subject, name, Some(&i.to_string())).unwrap();
        }
    }

    fn clear_chunks(events: &LocalEvents, subject: Hash) -> Vec<(Hash, usize)> {
        events
            .events_on(subject)
            .unwrap()
            .into_iter()
            .filter_map(|entry| match entry {
                Entry::Clear { id, claims, .. } => Some((id, claims.len())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn test_clear_tag_chunks_a_clear_over_the_claim_cap() {
        // Regression for #1720: a name with more live claims than one tag.clear
        // may name (MAX_CLEAR_CLAIMS) could never be cleared at all.
        let (_dir, mut events) = open_events();
        let subject = publish_subject(&mut events);
        let other_subject = publish_subject(&mut events);
        set_claims(&mut events, subject, "topic", MAX_CLEAR_CLAIMS + 1);
        events.set_tag(subject, "keep", None).unwrap();
        events.set_tag(other_subject, "topic", None).unwrap();

        let (count, event) = events.clear_tag(subject, "topic").unwrap();

        assert_eq!(count, MAX_CLEAR_CLAIMS + 1);
        let chunks = clear_chunks(&events, subject);
        assert_eq!(
            chunks.iter().map(|(_, size)| *size).collect::<Vec<_>>(),
            vec![MAX_CLEAR_CLAIMS, 1]
        );
        // The returned ID is the final chunk's, the one that completes the clear.
        assert_eq!(chunks.last().unwrap().0, event);
        assert!(
            events
                .tags_on(subject)
                .unwrap()
                .iter()
                .all(|claim| claim.name != "topic")
        );
        // Other names, and the same name on other subjects, are untouched.
        assert_eq!(events.tags_on(subject).unwrap().len(), 1);
        assert_eq!(events.tags_on(other_subject).unwrap().len(), 1);
    }

    #[test]
    fn test_clear_tag_chunks_600_claims_into_three_events() {
        let (_dir, mut events) = open_events();
        let subject = publish_subject(&mut events);
        set_claims(&mut events, subject, "topic", 600);

        let (count, event) = events.clear_tag(subject, "topic").unwrap();

        assert_eq!(count, 600);
        let chunks = clear_chunks(&events, subject);
        assert_eq!(
            chunks.iter().map(|(_, size)| *size).collect::<Vec<_>>(),
            vec![
                MAX_CLEAR_CLAIMS,
                MAX_CLEAR_CLAIMS,
                600 - 2 * MAX_CLEAR_CLAIMS
            ]
        );
        assert_eq!(chunks.last().unwrap().0, event);
        assert!(events.tags_on(subject).unwrap().is_empty());
    }

    #[test]
    fn test_clear_tag_at_or_under_the_cap_records_one_event() {
        let (_dir, mut events) = open_events();
        let subject = publish_subject(&mut events);
        set_claims(&mut events, subject, "topic", 3);

        let (count, event) = events.clear_tag(subject, "topic").unwrap();

        assert_eq!(count, 3);
        assert_eq!(clear_chunks(&events, subject), vec![(event, 3)]);
        // Nothing left to clear is still an error.
        assert!(events.clear_tag(subject, "topic").is_err());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod chat_tests {
    use super::*;
    use id_core::membership::AuthId;
    use id_core::membership::body::encode_body;
    use p2panda_auth::group::{GroupAction, GroupMember};

    fn key(n: u8) -> SigningKey {
        SigningKey::from_bytes(&[n; 32])
    }

    fn individual(key: &SigningKey) -> GroupMember<AuthId> {
        GroupMember::Individual(AuthId::Key(*key.verifying_key().as_bytes()))
    }

    /// The channel create event (Alice creates, Bob is a member) and its encoded body.
    fn create_event(alice: &SigningKey, bob: &SigningKey) -> (Event, Vec<u8>) {
        let action = GroupAction::Create {
            initial_members: vec![
                (individual(alice), p2panda_auth::Access::manage()),
                (individual(bob), p2panda_auth::Access::write()),
            ],
        };
        let body = encode_body(&action).unwrap();
        let ext = ChainExt {
            kind: "group.op".to_owned(),
            group: None,
            after: Vec::new(),
            tag: None,
            document: None,
            removes: None,
            chat: None,

            note: None,
            link: None,
            checkpoint: None,
            rotation: None,
        };
        let event = sign(alice, 0, None, Some(&body), ext).unwrap();
        (event, body)
    }

    /// A node whose store already holds the channel create, as sync would have brought it.
    fn node(dir: &Path, key: SigningKey, create: &Event, body: &[u8]) -> LocalEvents {
        let mut node = LocalEvents::open(dir, key).unwrap();
        node.store
            .ingest_with_body(&create.bytes, Some(body), 0)
            .unwrap();
        node
    }

    /// Moves `from`'s applied chat events into `to`'s store, as a sync session would.
    fn exchange(from: &LocalEvents, to: &mut LocalEvents) {
        let events = from
            .store
            .applied_events(&from.author(), None, None, usize::MAX)
            .unwrap();
        for event in &events {
            if event.header.extensions.kind == CHAT_MESSAGE_KIND {
                to.store.ingest(&event.bytes, now_ms()).unwrap();
            }
        }
    }

    #[test]
    fn test_two_members_exchange_chat_and_a_non_member_is_refused() {
        let (alice, bob, carol) = (key(51), key(52), key(53));
        let (create, body) = create_event(&alice, &bob);
        let group = create.id;
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let dir_c = tempfile::tempdir().unwrap();
        let mut node_a = node(dir_a.path(), alice, &create, &body);
        let mut node_b = node(dir_b.path(), bob, &create, &body);
        let mut node_c = node(dir_c.path(), carol, &create, &body);

        // A non-member's write is refused at admission: nothing is signed or recorded.
        let refused = node_c.send_chat(group, ChatMessage::new("carol here", Vec::new()).unwrap());
        assert!(refused.is_err());
        assert!(node_c.store.chat_records(&group).unwrap().is_empty());

        let first = node_a
            .send_chat(
                group,
                ChatMessage::new("hello from alice", Vec::new()).unwrap(),
            )
            .unwrap();
        let second = node_b
            .send_chat(
                group,
                ChatMessage::new("hello from bob", Vec::new()).unwrap(),
            )
            .unwrap();

        exchange(&node_a, &mut node_b);
        exchange(&node_b, &mut node_a);

        let timeline_a = node_a.chat_timeline(group).unwrap();
        let timeline_b = node_b.chat_timeline(group).unwrap();
        let texts_a: Vec<&str> = timeline_a
            .iter()
            .map(|record| record.message.text.as_str())
            .collect();
        let texts_b: Vec<&str> = timeline_b
            .iter()
            .map(|record| record.message.text.as_str())
            .collect();
        assert_eq!(texts_a, texts_b);
        assert_eq!(texts_a.len(), 2);
        let ids: Vec<Hash> = timeline_a.iter().map(|record| record.id).collect();
        assert!(ids.contains(&first));
        assert!(ids.contains(&second));
    }

    #[test]
    fn test_send_rate_admits_the_configured_burst_and_refuses_past_it() {
        let (alice, bob) = (key(54), key(55));
        let (create, body) = create_event(&alice, &bob);
        let group = create.id;
        let dir = tempfile::tempdir().unwrap();
        let mut node = node(dir.path(), alice, &create, &body);
        // The configuration seam (#3950): a small burst in a wide window, so the test
        // does not depend on how fast the sends happen. The default configuration is
        // the world-chat 50 per 10 seconds, unit-tested in the chat module.
        node.chat_rate = SendRate::new(id_store::chat::ChatRateConfig {
            max: 3,
            window_ms: 60_000,
        });
        for index in 0..3 {
            node.send_chat(
                group,
                ChatMessage::new(&format!("message {index}"), Vec::new()).unwrap(),
            )
            .unwrap();
        }
        let refused = node.send_chat(group, ChatMessage::new("one too many", Vec::new()).unwrap());
        assert!(refused.is_err());
    }
}
