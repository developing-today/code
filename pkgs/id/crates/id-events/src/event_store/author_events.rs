//! The `AuthorEvents` index and the view's candidate retrieval (writer-set
//! spec §4.2; §9 item 3 of #3789).
//!
//! The index is `(author, seq) → (event_id, kind, subject?)`, built by
//! ranging the existing `events` table per author. That table is keyed by
//! position, persisted, and written once per event at ingest, so the index
//! is persisted and incremental by construction: a read costs the rows in
//! the author's range, never a pass over the store (the failure of the
//! rebuilt-per-call `TagIndex`). It records `kind` and filters on none, so a
//! `didit.record` comes back exactly as a `tag.set` does.
//!
//! [`EventStore::view_candidates`] is §4.2's three retrieval queries:
//!
//! 1. the author range of every author *historically* admitted up to the
//!    selected set version, so a removed author's earlier claims still reach
//!    the predicate (§3.2);
//! 2. the causal past of each retro-admitting version, so a claim predating
//!    every version can be imported by `include_history` (§3.1);
//! 3. each named exception, fetched by ID (§4.4).
//!
//! Retrieval decides nothing: it returns a superset, and
//! [`crate::idset::project::project`] decides what counts.
//! [`EventStore::causal_snapshot`] closes the inputs over their causal
//! dependencies to build the [`Snapshot`] that projection evaluates.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use p2panda_core::{Hash, SeqNum};
use redb::{ReadableDatabase, ReadableTable};

use id_core::waiting::AuthorKey;
use id_store::chain::Event;
use id_store::tag_claims::TagOp;

use super::{APPLIED, EVENTS, EventStore, StoreError, decode_stored, position_key};
use crate::idset::project::{Snapshot, SnapshotEvent};
use crate::idset::view::{ExpandedWriters, ProjectionError};

/// One row of the `AuthorEvents` index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorEvent {
    /// The signer.
    pub author: AuthorKey,
    /// Its position in the signer's chain.
    pub seq: SeqNum,
    /// The event's ID.
    pub id: Hash,
    /// The header's `kind`, whatever it is.
    pub kind: String,
    /// The subject the header names, when it names one: a `tag.set`'s
    /// subject event, in hex. Kinds that carry subjects in their body are
    /// left `None`; a kind adapter supplies those.
    pub subject: Option<String>,
    /// Its direct causal dependencies: the backlink, then the signed `after`.
    pub deps: Vec<Hash>,
    /// Whether the event applies under the store's resolution.
    pub applied: bool,
}

impl AuthorEvent {
    fn from_event(event: &Event, applied: bool) -> Self {
        let ext = &event.header.extensions;
        let mut deps: Vec<Hash> = event.header.backlink.into_iter().collect();
        for dep in &ext.after {
            if !deps.contains(dep) {
                deps.push(*dep);
            }
        }
        let subject = match &ext.tag {
            Some(TagOp::Set { subject, .. }) => Some(subject.to_hex()),
            _ => None,
        };
        Self {
            author: *event.header.verifying_key.as_bytes(),
            seq: event.header.seq_num,
            id: event.id,
            kind: ext.kind.clone(),
            subject,
            deps,
            applied,
        }
    }

    /// This row as an event of a projection [`Snapshot`].
    #[must_use]
    pub fn snapshot_event(&self) -> SnapshotEvent {
        SnapshotEvent {
            id: self.id,
            author: self.author,
            kind: self.kind.clone(),
            subject: self.subject.clone(),
            deps: self.deps.clone(),
        }
    }
}

/// The inputs of §4.2's retrieval, taken from the selected set's history.
#[derive(Clone, Debug, Default)]
pub struct CandidateQuery {
    /// `expand(v)` for every admission version `v ∈ adm_versions(S)`: the
    /// historical union, not only the version in force.
    pub historical: Vec<ExpandedWriters>,
    /// The event IDs of the admission versions whose `expand_retro` is
    /// non-empty: their causal pasts are scanned for imported history.
    pub retro_versions: Vec<Hash>,
    /// The view's exception IDs.
    pub exceptions: Vec<Hash>,
}

/// What retrieval returned, and what it touched.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Candidates {
    /// Every retrieved event, by ID. A superset of what counts.
    pub events: BTreeMap<Hash, AuthorEvent>,
    /// The authors whose ranges were scanned. Retrieval scans no others.
    pub scanned_authors: BTreeSet<AuthorKey>,
}

impl Candidates {
    /// The retrieved IDs, as [`crate::idset::project::project`] takes them.
    #[must_use]
    pub fn ids(&self) -> Vec<Hash> {
        self.events.keys().copied().collect()
    }
}

impl EventStore {
    /// The `AuthorEvents` rows of `author` with sequence numbers in
    /// `(after, until]`, in sequence order, at most `limit` of them, whether
    /// or not they apply. `None` leaves that side open.
    ///
    /// # Errors
    ///
    /// Returns a redb error if the read fails, or [`StoreError::Corrupt`].
    pub fn author_events(
        &self,
        author: &AuthorKey,
        after: Option<SeqNum>,
        until: Option<SeqNum>,
        limit: usize,
    ) -> Result<Vec<AuthorEvent>, StoreError> {
        let first = match after {
            Some(after) => after.checked_add(1),
            None => Some(0),
        };
        let Some(first) = first else {
            return Ok(Vec::new());
        };
        let last = until.unwrap_or(SeqNum::MAX);
        if first > last {
            return Ok(Vec::new());
        }
        let txn = self.db.begin_read()?;
        let applied = txn.open_table(APPLIED)?;
        let events = txn.open_table(EVENTS)?;
        let (start, end) = (position_key(author, first), position_key(author, last));
        let mut found = Vec::new();
        for row in events.range(start.as_slice()..=end.as_slice())? {
            if found.len() >= limit {
                break;
            }
            let (_position, bytes) = row?;
            let event = decode_stored(bytes.value())?;
            let is_applied = applied.get(event.id.as_bytes().as_slice())?.is_some();
            found.push(AuthorEvent::from_event(&event, is_applied));
        }
        Ok(found)
    }

    /// The `AuthorEvents` row of the event with this ID, if it is stored.
    ///
    /// # Errors
    ///
    /// Returns a redb error if the read fails, or [`StoreError::Corrupt`].
    pub fn author_event(&self, id: &Hash) -> Result<Option<AuthorEvent>, StoreError> {
        let Some(event) = self.event_by_id(id)? else {
            return Ok(None);
        };
        let applied = self.event_applied(id)?;
        Ok(Some(AuthorEvent::from_event(&event, applied)))
    }

    /// §4.2's candidate retrieval for one view: the historical author
    /// ranges, the retro-admitting versions' causal pasts, and the named
    /// exceptions. A public admission scans every author the store holds.
    /// An exception the store lacks is skipped: it cannot fire (§4.4).
    ///
    /// # Errors
    ///
    /// Returns a redb error if a read fails, or [`StoreError::Corrupt`].
    pub fn view_candidates(&self, query: &CandidateQuery) -> Result<Candidates, StoreError> {
        let mut candidates = Candidates::default();
        let public = query.historical.iter().any(ExpandedWriters::is_public);
        if public {
            for (author, _chain) in self.chains()? {
                candidates.scanned_authors.insert(author);
            }
        }
        for expanded in &query.historical {
            candidates
                .scanned_authors
                .extend(expanded.explicit_authors().iter().copied());
        }
        for author in &candidates.scanned_authors {
            for row in self.author_events(author, None, None, usize::MAX)? {
                candidates.events.insert(row.id, row);
            }
        }
        for version in &query.retro_versions {
            for row in self.causal_rows(&[*version])?.into_values() {
                if row.id != *version {
                    candidates.events.insert(row.id, row);
                }
            }
        }
        for id in &query.exceptions {
            if let Some(row) = self.author_event(id)? {
                candidates.events.insert(row.id, row);
            }
        }
        Ok(candidates)
    }

    /// The [`Snapshot`] a projection evaluates: `roots` and every stored
    /// event in their causal past, with whether each applies. A dependency
    /// the store lacks is left out, so the projection reports it as
    /// [`ProjectionError::MissingEvent`] instead of evaluating around it.
    ///
    /// # Errors
    ///
    /// Returns a redb error if a read fails, or [`StoreError::Corrupt`].
    pub fn causal_snapshot(&self, roots: &[Hash]) -> Result<Snapshot, StoreError> {
        let mut snapshot = Snapshot::new();
        for row in self.causal_rows(roots)?.into_values() {
            snapshot
                .insert(row.snapshot_event(), row.applied)
                .map_err(|err: ProjectionError| StoreError::Corrupt(err.to_string()))?;
        }
        Ok(snapshot)
    }

    /// `roots` and their stored causal past, each event read once.
    fn causal_rows(&self, roots: &[Hash]) -> Result<BTreeMap<Hash, AuthorEvent>, StoreError> {
        let mut rows = BTreeMap::new();
        let mut queue: VecDeque<Hash> = roots.iter().copied().collect();
        while let Some(id) = queue.pop_front() {
            if rows.contains_key(&id) {
                continue;
            }
            if let Some(row) = self.author_event(&id)? {
                queue.extend(row.deps.iter().copied());
                rows.insert(id, row);
            }
        }
        Ok(rows)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::idset::project::{OwnerVersion, Scope, View, WhyNotCounted, project};
    use crate::idset::{Entry, Excludes, WriterSet};
    use id_core::waiting::WaitingLimits;
    use id_store::chain::{ChainExt, sign};
    use id_store::event_types::Ingest;
    use p2panda_core::SigningKey;

    struct Writer {
        key: SigningKey,
        head: Option<Hash>,
        next: SeqNum,
    }

    impl Writer {
        fn new(n: u8) -> Self {
            Self {
                key: SigningKey::from_bytes(&[n; 32]),
                head: None,
                next: 0,
            }
        }

        fn author(&self) -> AuthorKey {
            *self.key.verifying_key().as_bytes()
        }

        /// Sign and ingest the next event of this chain.
        fn write(&mut self, store: &mut EventStore, kind: &str, after: &[Hash]) -> Hash {
            let ext = ChainExt {
                kind: kind.to_owned(),
                group: None,
                after: after.to_vec(),
                tag: None,
                document: None,
                removes: None,
                note: None,
                link: None,
                checkpoint: None,
                rotation: None,
                chat: None,
                anchor: None,
                thread: None,
            };
            let event = sign(&self.key, self.next, self.head, None, ext).unwrap();
            assert!(
                matches!(
                    store.ingest(&event.bytes, 0).unwrap(),
                    Ingest::Accepted { .. }
                ),
                "{kind} at {} is accepted",
                self.next
            );
            self.head = Some(event.id);
            self.next += 1;
            event.id
        }
    }

    fn open_temp() -> (tempfile::TempDir, EventStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
        (dir, store)
    }

    fn key_entry(author: AuthorKey, include_history: bool) -> Entry {
        Entry::Key {
            author,
            include_history,
        }
    }

    fn set(version: u64, owner: AuthorKey, entries: Vec<Entry>) -> WriterSet {
        WriterSet {
            set_id: Hash::from([200; 32]),
            version,
            owner,
            scope: None,
            entries,
            excludes: Excludes::default(),
        }
    }

    #[test]
    fn non_tag_kinds_are_indexed_with_their_kind() {
        let (_dir, mut store) = open_temp();
        let mut alice = Writer::new(2);
        let record = alice.write(&mut store, "didit.record", &[]);
        let note = alice.write(&mut store, "note.custom", &[]);
        let rows = store
            .author_events(&alice.author(), None, None, 10)
            .unwrap();
        let kinds: Vec<(SeqNum, Hash, &str)> = rows
            .iter()
            .map(|row| (row.seq, row.id, row.kind.as_str()))
            .collect();
        assert_eq!(
            kinds,
            vec![(0, record, "didit.record"), (1, note, "note.custom")]
        );
        assert!(rows.iter().all(|row| row.applied));
        assert_eq!(rows[1].deps, vec![record]);
        assert_eq!(
            store
                .author_events(&alice.author(), Some(0), None, 10)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store
                .author_events(&alice.author(), None, Some(0), 0)
                .unwrap(),
            [] as [AuthorEvent; 0]
        );
    }

    #[test]
    fn a_removed_authors_earlier_claims_are_retrieved_and_still_count() {
        let (_dir, mut store) = open_temp();
        let mut owner = Writer::new(1);
        let mut alice = Writer::new(2);
        let mut bob = Writer::new(3);
        let config = owner.write(&mut store, "view.config", &[]);
        let v1_set = set(1, owner.author(), vec![key_entry(alice.author(), false)]);
        let v1 = owner.write(&mut store, "idset.writer", &[]);
        let early = alice.write(&mut store, "didit.record", &[v1]);
        let v2_set = set(2, owner.author(), vec![key_entry(bob.author(), false)]);
        let v2 = owner.write(&mut store, "idset.writer", &[early]);
        let late = alice.write(&mut store, "didit.record", &[v2]);
        let bobs = bob.write(&mut store, "didit.record", &[v2]);

        let resolution = Hash::from([250; 32]);
        let versions = vec![
            OwnerVersion::admission(v1, &v1_set, &[config], &resolution, &[]).unwrap(),
            OwnerVersion::admission(v2, &v2_set, &[v1, early], &resolution, &[]).unwrap(),
        ];
        let historical = versions
            .iter()
            .map(|v| match &v.body {
                crate::idset::project::VersionBody::Admission { expand, .. } => expand.clone(),
                crate::idset::project::VersionBody::Rescind { .. } => unreachable!(),
            })
            .collect();
        let candidates = store
            .view_candidates(&CandidateQuery {
                historical,
                ..CandidateQuery::default()
            })
            .unwrap();
        // The cost claim: only the historical writers' ranges are touched.
        assert_eq!(
            candidates.scanned_authors,
            BTreeSet::from([alice.author(), bob.author()])
        );
        assert!(candidates.events.contains_key(&early));

        let view = View {
            view_id: Hash::from([150; 32]),
            config_seq: 0,
            config_event: config,
            owner: owner.author(),
            set_id: v2_set.set_id,
            set_seq: 2,
            frontier: vec![late, bobs],
            resolution,
            subject_scope: Scope::Any,
            kind_scope: Scope::Only(BTreeSet::from(["didit.record".to_owned()])),
            exceptions: BTreeSet::new(),
        };
        let snapshot = store
            .causal_snapshot(&[config, v1, v2, late, bobs])
            .unwrap();
        let projection = project(&view, &versions, &snapshot, &candidates.ids()).unwrap();
        let counted: BTreeSet<Hash> = projection.counted.iter().map(|hit| hit.claim).collect();
        assert_eq!(counted, BTreeSet::from([early, bobs]));
        let removed = projection
            .uncounted
            .iter()
            .find(|hit| hit.claim == late)
            .unwrap();
        assert_eq!(removed.reason, WhyNotCounted::AuthorRemoved(2));
    }

    #[test]
    fn retro_versions_reach_history_and_exceptions_reach_strangers() {
        let (_dir, mut store) = open_temp();
        let mut owner = Writer::new(1);
        let mut carol = Writer::new(4);
        let mut stranger = Writer::new(9);
        let mut bystander = Writer::new(8);
        let backlog = carol.write(&mut store, "didit.record", &[]);
        let named = stranger.write(&mut store, "didit.record", &[]);
        let unrelated = bystander.write(&mut store, "didit.record", &[]);
        let v1 = owner.write(&mut store, "idset.writer", &[backlog]);

        let nobody = ExpandedWriters::default();
        let candidates = store
            .view_candidates(&CandidateQuery {
                historical: vec![nobody],
                retro_versions: vec![v1],
                exceptions: vec![named, Hash::from([77; 32])],
            })
            .unwrap();
        assert!(candidates.scanned_authors.is_empty(), "no range is scanned");
        let ids: BTreeSet<Hash> = candidates.ids().into_iter().collect();
        // The retro version's past brings Carol's backlog; the exception
        // brings the stranger's claim; the version itself and anything
        // nobody pointed at stay out; a missing exception does not fire.
        assert_eq!(ids, BTreeSet::from([backlog, named]));
        assert!(!ids.contains(&unrelated));
        assert!(!ids.contains(&v1));
    }

    #[test]
    fn a_public_admission_scans_every_author() {
        let (_dir, mut store) = open_temp();
        let mut owner = Writer::new(1);
        let mut alice = Writer::new(2);
        let claim = alice.write(&mut store, "didit.record", &[]);
        let v1_set = set(
            1,
            owner.author(),
            vec![Entry::Public {
                include_history: false,
            }],
        );
        let v1 = owner.write(&mut store, "idset.writer", &[claim]);
        let version =
            OwnerVersion::admission(v1, &v1_set, &[claim], &Hash::from([250; 32]), &[]).unwrap();
        let crate::idset::project::VersionBody::Admission { expand, .. } = version.body else {
            unreachable!()
        };
        let candidates = store
            .view_candidates(&CandidateQuery {
                historical: vec![expand],
                ..CandidateQuery::default()
            })
            .unwrap();
        assert_eq!(
            candidates.scanned_authors,
            BTreeSet::from([owner.author(), alice.author()])
        );
        assert!(candidates.events.contains_key(&claim));
    }

    #[test]
    fn an_unknown_root_is_left_out_of_the_snapshot() {
        let (_dir, mut store) = open_temp();
        let mut alice = Writer::new(2);
        let claim = alice.write(&mut store, "didit.record", &[]);
        let snapshot = store
            .causal_snapshot(&[claim, Hash::from([66; 32])])
            .unwrap();
        assert!(snapshot.applied(&claim));
        assert!(!snapshot.applied(&Hash::from([66; 32])));
    }
}
