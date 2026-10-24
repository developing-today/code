//! Documents and tags, signed into the event store from one node (P6.7c).
//!
//! `id doc` and `id claim` act through [`LocalEvents`]. Each action signs one event with the node's
//! key, names every other author's head as its `after`, and ingests the event with
//! [`EventStore::ingest`], the path sync uses. The store checks the signature and applies the event.
//! So these events are signed, unlike the group log's (P6.7a), which trusts the caller's name.
//!
//! The store stands in for sync. The CLI opens one store directory, named by `--event-store`, and
//! nodes on one machine that share the directory see each other's events, as they share the group
//! log. Nothing here moves events between machines.
//!
//! The reads walk every applied event, which is fine for the CLI and is not an index. The document
//! and tag rules are the ones in [`crate::documents`] and [`crate::tag_claims`].

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use p2panda_core::{Hash, SigningKey};

use crate::chain::{ChainExt, Event, sign};
use crate::documents::{DOCUMENT_KIND, DocumentRef, REMOVAL_KIND};
use crate::event_store::{EventStore, Ingest};
use crate::search::{Hit, ValueMatch, search_store};
use crate::tag_claims::{CLEAR_KIND, Claim, SET_KIND, TagOp, check_op};
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
        let id = self.record(DOCUMENT_KIND, None, Some(document.clone()), None)?;
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
        self.record(SET_KIND, Some(op), None, None)
    }

    /// Clears every live claim named `name` on `subject`, whoever set it. It returns how many it
    /// cleared, and the clear's event ID. Nothing to clear is an error.
    ///
    /// # Errors
    ///
    /// Fails if no live claim matches, or the store cannot record the event.
    pub fn clear_tag(&mut self, subject: Hash, name: &str) -> Result<(usize, Hash)> {
        let claims: Vec<Hash> = self
            .store
            .tag_claims()?
            .into_iter()
            .filter(|claim| claim.subject == subject && claim.name == name)
            .map(|claim| claim.id)
            .collect();
        if claims.is_empty() {
            bail!("no live claim on {} is named {name}", subject.to_hex());
        }
        let count = claims.len();
        let event = self.record(CLEAR_KIND, Some(TagOp::Clear { claims }), None, None)?;
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

    /// Signs one event with the node's key, after every other author's head, and ingests it. The
    /// store must apply it, or the call fails.
    fn record(
        &mut self,
        kind: &str,
        tag: Option<TagOp>,
        document: Option<DocumentRef>,
        removes: Option<Hash>,
    ) -> Result<Hash> {
        let author = self.author();
        let after = self.other_heads()?;
        let chain = self.store.chain(&author)?.unwrap_or_default();
        let ext = ChainExt {
            kind: kind.to_owned(),
            group: None,
            after,
            tag,
            document,
            removes,
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
    match (ext.kind.as_str(), &ext.tag, &ext.document, ext.removes) {
        (DOCUMENT_KIND, _, Some(document), _) => Some(Entry::Document {
            id,
            author,
            document: document.clone(),
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
        ) => Some(Entry::Set {
            id,
            author,
            subject: *subject,
            name: name.clone(),
            value: value.clone(),
        }),
        (CLEAR_KIND, Some(TagOp::Clear { claims }), _, _) => Some(Entry::Clear {
            id,
            author,
            claims: claims.clone(),
        }),
        (REMOVAL_KIND, _, _, Some(document)) => Some(Entry::Remove {
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
}
