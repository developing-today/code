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
//! and tag rules are the ones in [`crate::documents`] and [`crate::tag_claims`].

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use p2panda_core::{Hash, SigningKey};

use crate::chain::{ChainExt, Event, MAX_AFTER, sign};
use crate::documents::{DOCUMENT_KIND, DocumentRef, REMOVAL_KIND};
use crate::event_store::{EventStore, Ingest};
use crate::notes::{
    AppliedNote, CREATE_KIND as NOTE_CREATE_KIND, NoteOp, NoteRecord,
    REVISE_KIND as NOTE_REVISE_KIND, check_note_op, fold_notes,
};
use crate::search::{Hit, ValueMatch, search_store};
use crate::tag_claims::{CLEAR_KIND, Claim, MAX_CLEAR_CLAIMS, SET_KIND, TagOp, check_op};
use crate::waiting::{AuthorKey, WaitingLimits};

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

/// The event store, and the key the node signs its events with.
#[derive(Debug)]
pub struct LocalEvents {
    store: EventStore,
    key: SigningKey,
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
        Ok(Self { store, key })
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
        for (name, value) in tags {
            check_tag(name, value.as_deref())?;
        }
        let id = self.record(DOCUMENT_KIND, None, Some(document.clone()), None, None, &[])?;
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

    /// Sets the tag `name` on `subject`, with `value` if given. The new claim's ID is the event's.
    ///
    /// # Errors
    ///
    /// Fails if the name or value is refused, or the store cannot record the event.
    pub fn set_tag(&mut self, subject: Hash, name: &str, value: Option<&str>) -> Result<Hash> {
        let op = TagOp::Set {
            subject,
            name: name.to_owned(),
            value: value.map(str::to_owned),
        };
        self.record(SET_KIND, Some(op), None, None, None, &[])
    }

    /// Clears every live claim named `name` on `subject`, whoever set it. Nothing to clear is an
    /// error.
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
        let claims: Vec<Claim> = self
            .store
            .tag_claims()?
            .into_iter()
            .filter(|claim| claim.subject == subject && claim.name == name)
            .collect();
        if claims.is_empty() {
            bail!("no live claim on {} is named {name}", subject.to_hex());
        }
        let count = claims.len();
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
                Some(TagOp::Clear { claims: ids }),
                None,
                None,
                None,
                &required,
            )?);
        }
        let event = last.context("a clear with claims records at least one event")?;
        Ok((count, event))
    }

    /// The live claims on `subject`, sorted by claim ID.
    ///
    /// # Errors
    ///
    /// Fails if the store cannot be read.
    pub fn tags_on(&self, subject: Hash) -> Result<Vec<Claim>> {
        Ok(self
            .store
            .tag_claims()?
            .into_iter()
            .filter(|claim| claim.subject == subject)
            .collect())
    }

    /// The live claims that match `name`, and `value` if given, as [`crate::search`] returns them.
    ///
    /// # Errors
    ///
    /// Fails if the store cannot be read.
    pub fn search(&self, name: &str, value: Option<&str>) -> Result<Vec<Hit>> {
        let matching = value.map_or(ValueMatch::Any, ValueMatch::Exact);
        Ok(search_store(&self.store, name, matching)?)
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
        self.record(NOTE_CREATE_KIND, None, None, None, Some(op), &[])
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
        self.record(NOTE_REVISE_KIND, None, None, None, Some(op), &required)
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
        // Quarantine invalid remote revisions (PR #3908 Codex P1): ingest
        // applies a signed note.revise after checking only its shape, so a
        // peer can land a revision naming an unknown note or parent, and
        // the strict fold would then fail every note query in the store --
        // notes(), note_history(), and revise_note() with it. Drop,
        // repeatedly (a revision that names a dropped one as parent goes
        // too), every revise whose note has no create here or whose
        // parents are not revisions of that note. The events stay in the
        // store; valid notes fold exactly as before.
        let mut applied = applied;
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
            if applied.len() == before {
                break;
            }
        }
        Ok(fold_notes(&applied)?)
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
        tag: Option<TagOp>,
        document: Option<DocumentRef>,
        removes: Option<Hash>,
        note: Option<NoteOp>,
        required: &[Hash],
    ) -> Result<Hash> {
        let author = self.author();
        let after = self.bounded_after(required)?;
        let chain = self.store.chain(&author)?.unwrap_or_default();
        let ext = ChainExt {
            kind: kind.to_owned(),
            group: None,
            after,
            tag,
            document,
            removes,
            note,
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
        // split by author. Exactly those claims are removed: a claim with a
        // different name on the same subject, and a same-name claim on a
        // different subject, both survive.
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
        let remaining = events.tags_on(subject).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].name, "other");
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
        assert!(events
            .revise_note(note, "text/markdown", b"bad", &[id(222)])
            .is_err());
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
            },
        )
        .unwrap();
        let ingested = events.store.ingest(&foreign.bytes, now_ms()).unwrap();
        assert!(matches!(ingested, Ingest::Accepted { .. }), "{ingested:?}");
        let notes = events.notes().unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].id, note);
        assert!(events.note_history(note).is_ok());
        assert!(events
            .revise_note(note, "text/markdown", b"second", &[note])
            .is_ok());
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
        assert!(events
            .tags_on(subject)
            .unwrap()
            .iter()
            .all(|claim| claim.name != "topic"));
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
