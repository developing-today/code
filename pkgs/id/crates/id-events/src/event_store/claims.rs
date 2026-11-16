//! Tag claims over the store (Phase 5, P5.1 and P5.2).
//!
//! A tag event is applied like any content event: the resolution decides whether it applies, and
//! the store keeps it either way. Its operation is in its header (#182), so the stored event
//! carries the claim or clear marker. [`EventStore::tag_claims`] is the default view: it folds only
//! markers authorized by claim authorship or the subject's owning group's settled Admin role.
//! Every marker, including unauthorized ones, remains on the append-only event log.
//!
//! Removal stops new content (P5.6, decision on #195, 2026-11-16): the fold drops a
//! `tag.set` on a document when an effective removal of that document (see
//! [`super::documents::removal_marks`]) is in the set's causal past — its author had seen
//! the removal when signing. Sets concurrent with the removal are kept, and `tag.clear`
//! markers still fold: a clear adds no content. The dropped set stays on the append-only
//! log and in the raw/applied views; only the default view loses it.
//!
//! The fold is persisted (#183): [`EventStore::tag_claims`] stores its result in the `tag_fold`
//! table next to a fingerprint of everything the fold reads, and serves the persisted claims
//! while that fingerprint still matches. The fingerprint is per-author change tracking — each
//! author's folded event count and highest sequence number in `events`, plus a length-and-key
//! hash of each auxiliary table the resolution and the group projection read — never a single
//! scalar position: events sort by `(author, seq)`, so no global position exists to watermark.
//! Any change to a folded input changes the fingerprint, the persisted state is then treated as
//! inconsistent, and the fold is rebuilt in full and repopulated. Serving a call therefore costs
//! a key-only scan of the folded tables, not a decode-and-resolve of every stored event.

use std::collections::{BTreeMap, HashMap};

use p2panda_core::Hash;
use redb::{ReadableDatabase, ReadableMultimapTable, ReadableTable, ReadableTableMetadata};

use super::resolve::{self, author_of};
use super::{
    ACTIONS, APPLIED, CHAINS, DEVICE_OPS, EVENTS, EventStore, FORKS, GROUPS, HASHES, PROOF_HEADERS,
    StoreError, TAG_FOLD, corrupt,
};
use crate::groups::{Projection, Role as GroupRole};
use id_store::chain::Event;
use id_store::tag_claims::{Applied, Claim, TagOp, compacted_claims, is_tag_kind};
use id_store::tag_index::TagIndex;

/// The events an event names: its `after` hashes, and its backlink.
fn parents_of(event: &Event) -> Vec<Hash> {
    event
        .header
        .extensions
        .after
        .iter()
        .copied()
        .chain(event.header.backlink)
        .collect()
}

impl EventStore {
    /// The default-view claims in the store. An applied `tag.clear` hides a claim only when the
    /// marker's author is that claim's author, is an Admin of the group owning the subject, or
    /// signs as a device an Admin of that group authorized at Admin scope (issue #5656).
    /// Unauthorized markers remain stored and accessible through the raw/applied event APIs.
    ///
    /// The result is persisted in the `tag_fold` table (#183) and served from there while the
    /// fingerprint of the fold's inputs (see the module docs) still matches; a mismatch, a
    /// missing row, or an undecodable row rebuilds the fold in full and repopulates the table.
    ///
    /// # Errors
    ///
    /// Returns a redb error if the read fails, or [`StoreError::Corrupt`] for a stored tag event
    /// that carries no tag operation for its kind.
    pub fn tag_claims(&self) -> Result<Vec<Claim>, StoreError> {
        let fingerprint = self.fold_fingerprint()?;
        if let Some(claims) = self.persisted_tag_claims(&fingerprint)? {
            return Ok(claims);
        }
        let claims = self.fold_tag_claims()?;
        self.persist_tag_claims(&fingerprint, &claims)?;
        Ok(claims)
    }

    /// The full fold over the stored events: the body [`EventStore::tag_claims`] persists and
    /// serves while its fingerprint holds. Kept separate so the persisted path and the tests
    /// can hold the two side by side.
    fn fold_tag_claims(&self) -> Result<Vec<Claim>, StoreError> {
        let txn = self.db.begin_read()?;
        let (stored, forks, headers) = {
            let events = txn.open_table(EVENTS)?;
            let actions = txn.open_table(ACTIONS)?;
            let device_ops = txn.open_table(DEVICE_OPS)?;
            let forks = txn.open_table(FORKS)?;
            let proof_headers = txn.open_table(PROOF_HEADERS)?;
            resolve::load(&events, &actions, &device_ops, &forks, &proof_headers)?
        };
        let resolved = resolve::resolve(&stored, &forks, &headers)?;

        let mut parents: HashMap<Hash, Vec<Hash>> = HashMap::new();
        let mut subject_groups = HashMap::new();
        let mut applied = Vec::new();
        for item in &stored {
            let event = &item.event;
            parents.insert(event.id, parents_of(&event));
            if resolved.applied.contains(&event.id) {
                if let Some(group) = event.header.extensions.group {
                    subject_groups.insert(event.id, group);
                }
            }
            if !is_tag_kind(&event.header.extensions.kind) || !resolved.applied.contains(&event.id)
            {
                continue;
            }
            let Some(op) = event.header.extensions.tag.clone() else {
                return Err(corrupt(
                    "a stored tag event carries no tag operation".to_owned(),
                ));
            };
            applied.push(Applied {
                id: event.id,
                author: author_of(&event),
                op,
            });
        }
        // A fork proof header may be named by a clear and has parents, but never applies.
        for header in &headers {
            parents
                .entry(header.id)
                .or_insert_with(|| parents_of(header));
        }
        // P5.6 (#195): an effective removal stops new content on its document. Drop a
        // tag set whose causal past contains an effective removal of its subject;
        // concurrent sets and clears are kept (a clear adds no content).
        let marks = super::documents::removal_marks(
            &stored,
            &resolved.applied,
            resolved.projection.as_ref(),
        )?;
        let applied: Vec<Applied> = applied
            .into_iter()
            .filter(|event| match &event.op {
                TagOp::Set { subject, .. } => !marks
                    .get(subject)
                    .is_some_and(|removals| causal_past_contains(event.id, removals, &parents)),
                TagOp::Clear { .. } => true,
            })
            .collect();
        let default_view =
            default_view_events(&applied, &subject_groups, resolved.projection.as_ref());
        Ok(compacted_claims(&default_view, &parents))
    }

    /// The tag index over the live claims, built from [`EventStore::tag_claims`].
    ///
    /// # Errors
    ///
    /// Returns the errors of [`EventStore::tag_claims`].
    pub fn tag_index(&self) -> Result<TagIndex, StoreError> {
        Ok(TagIndex::new(&self.tag_claims()?))
    }

    /// The fingerprint of everything the tag fold reads, as bytes: for each author in
    /// `events`, their event count and highest sequence number (the per-author folded
    /// watermarks), a hash of the `events` keys, and for each auxiliary table the resolution
    /// or the group projection reads, its row count and a hash of its keys. Key-only scans:
    /// no event value is decoded. Two stores with the same fingerprint fold to the same
    /// claims; any kept event, fork proof, group row, action, device op, or applied-set
    /// change moves the fingerprint.
    fn fold_fingerprint(&self) -> Result<Vec<u8>, StoreError> {
        let txn = self.db.begin_read()?;
        let events = txn.open_table(EVENTS)?;
        let mut authors: BTreeMap<[u8; 32], (u64, u64)> = BTreeMap::new();
        let mut events_key_hash = Fnv1a::new();
        for entry in events.iter()? {
            let (key, _) = entry?;
            let key = key.value();
            events_key_hash.write(key);
            if key.len() == 40 {
                let mut author = [0u8; 32];
                author.copy_from_slice(&key[..32]);
                let seq = u64::from_be_bytes(key[32..40].try_into().unwrap_or([0; 8]));
                let slot = authors.entry(author).or_insert((0, 0));
                slot.0 += 1;
                slot.1 = slot.1.max(seq);
            }
        }
        let mut out = Vec::new();
        out.extend_from_slice(&(authors.len() as u64).to_be_bytes());
        for (author, (count, max_seq)) in &authors {
            out.extend_from_slice(author);
            out.extend_from_slice(&count.to_be_bytes());
            out.extend_from_slice(&max_seq.to_be_bytes());
        }
        out.extend_from_slice(&events_key_hash.finish().to_be_bytes());
        // The auxiliary tables, in a fixed order: actions, device_ops, forks,
        // proof_headers, groups, chains, hashes, applied.
        let actions = txn.open_table(ACTIONS)?;
        out.extend_from_slice(&table_fingerprint(&actions)?.to_be_bytes());
        let device_ops = txn.open_table(DEVICE_OPS)?;
        out.extend_from_slice(&table_fingerprint(&device_ops)?.to_be_bytes());
        let forks = txn.open_table(FORKS)?;
        out.extend_from_slice(&table_fingerprint(&forks)?.to_be_bytes());
        let proof_headers = txn.open_table(PROOF_HEADERS)?;
        out.extend_from_slice(&table_fingerprint(&proof_headers)?.to_be_bytes());
        let groups = txn.open_multimap_table(GROUPS)?;
        out.extend_from_slice(&multimap_fingerprint(&groups)?.to_be_bytes());
        let chains = txn.open_table(CHAINS)?;
        out.extend_from_slice(&table_fingerprint(&chains)?.to_be_bytes());
        let hashes = txn.open_table(HASHES)?;
        out.extend_from_slice(&table_fingerprint(&hashes)?.to_be_bytes());
        let applied = txn.open_table(APPLIED)?;
        out.extend_from_slice(&table_fingerprint(&applied)?.to_be_bytes());
        Ok(out)
    }

    /// The persisted claims, when a `tag_fold` row exists, decodes, and carries exactly
    /// `fingerprint`. A missing table or row, a fingerprint mismatch, and an undecodable
    /// row are all a miss: the caller rebuilds the fold in full.
    fn persisted_tag_claims(&self, fingerprint: &[u8]) -> Result<Option<Vec<Claim>>, StoreError> {
        let txn = self.db.begin_read()?;
        let Ok(table) = txn.open_table(TAG_FOLD) else {
            return Ok(None);
        };
        let Some(row) = table.get(TAG_FOLD_KEY)? else {
            return Ok(None);
        };
        Ok(decode_tag_fold(row.value(), fingerprint))
    }

    /// Persists `claims` under `fingerprint`, replacing any earlier row.
    fn persist_tag_claims(&self, fingerprint: &[u8], claims: &[Claim]) -> Result<(), StoreError> {
        let txn = self.db.begin_write()?;
        {
            let mut table = txn.open_table(TAG_FOLD)?;
            table.insert(
                TAG_FOLD_KEY,
                encode_tag_fold(fingerprint, claims).as_slice(),
            )?;
        }
        txn.commit()?;
        Ok(())
    }
}

/// The one row of the `tag_fold` table: the persisted fold and its fingerprint.
const TAG_FOLD_KEY: &[u8] = b"claims";

/// FNV-1a over a byte string: a small deterministic key hash for fingerprints. Not a
/// cryptographic hash — it only has to change when a table's keys change, and a collision
/// would additionally have to coincide with identical per-author watermarks and identical
/// row counts on every folded table to serve a stale fold.
#[derive(Debug)]
struct Fnv1a(u64);

impl Fnv1a {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

/// A table's fingerprint contribution: its row count mixed with a hash of its keys.
fn table_fingerprint<T>(table: &T) -> Result<u64, StoreError>
where
    T: ReadableTable<&'static [u8], &'static [u8]>,
{
    let mut hash = Fnv1a::new();
    hash.write(&table.len()?.to_be_bytes());
    for entry in table.iter()? {
        let (key, _) = entry?;
        hash.write(key.value());
    }
    Ok(hash.finish())
}

/// A multimap table's fingerprint contribution: a hash over every key and value pair.
fn multimap_fingerprint<T>(table: &T) -> Result<u64, StoreError>
where
    T: ReadableMultimapTable<&'static [u8], &'static [u8]>,
{
    let mut hash = Fnv1a::new();
    let mut rows = 0u64;
    for entry in table.iter()? {
        let (key, values) = entry?;
        for value in values {
            let value = value?;
            rows += 1;
            hash.write(key.value());
            hash.write(value.value());
        }
    }
    hash.write(&rows.to_be_bytes());
    Ok(hash.finish())
}

/// Encodes the persisted fold: a version byte, the fingerprint, then each claim's fields
/// in length-prefixed form. The encoding is this module's own; a row written by another
/// version fails the version check on decode and is rebuilt.
fn encode_tag_fold(fingerprint: &[u8], claims: &[Claim]) -> Vec<u8> {
    let mut out = vec![TAG_FOLD_VERSION];
    out.extend_from_slice(&(fingerprint.len() as u64).to_be_bytes());
    out.extend_from_slice(fingerprint);
    out.extend_from_slice(&(claims.len() as u64).to_be_bytes());
    for claim in claims {
        out.extend_from_slice(claim.id.as_bytes());
        out.extend_from_slice(&claim.author);
        out.extend_from_slice(claim.subject.as_bytes());
        write_text(&mut out, &claim.name);
        match &claim.value {
            None => out.push(0),
            Some(value) => {
                out.push(1);
                write_text(&mut out, value);
            }
        }
        out.push(match claim.scope {
            id_store::tag_claims::TagScope::Shared => 0,
            id_store::tag_claims::TagScope::Local => 1,
        });
    }
    out
}

fn write_text(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u64).to_be_bytes());
    out.extend_from_slice(text.as_bytes());
}

/// Decodes a persisted fold row, returning its claims only when the version matches and
/// the row's fingerprint equals `fingerprint`. Any malformed byte is a miss (`None`), so
/// a corrupt or foreign row falls back to a full rebuild, never to a wrong answer.
fn decode_tag_fold(row: &[u8], fingerprint: &[u8]) -> Option<Vec<Claim>> {
    let mut reader = FoldReader { bytes: row, at: 0 };
    if reader.byte()? != TAG_FOLD_VERSION {
        return None;
    }
    let stored_len = usize::try_from(reader.u64()?).ok()?;
    let stored = reader.take(stored_len)?;
    if stored != fingerprint {
        return None;
    }
    let count = usize::try_from(reader.u64()?).ok()?;
    let mut claims = Vec::with_capacity(count.min(1 << 20));
    for _ in 0..count {
        let id = Hash::from_bytes(reader.take(32)?.try_into().ok()?);
        let author: [u8; 32] = reader.take(32)?.try_into().ok()?;
        let subject = Hash::from_bytes(reader.take(32)?.try_into().ok()?);
        let name = reader.text()?;
        let value = match reader.byte()? {
            0 => None,
            1 => Some(reader.text()?),
            _ => return None,
        };
        let scope = match reader.byte()? {
            0 => id_store::tag_claims::TagScope::Shared,
            1 => id_store::tag_claims::TagScope::Local,
            _ => return None,
        };
        claims.push(Claim {
            id,
            author,
            subject,
            name,
            value,
            scope,
        });
    }
    if reader.at != row.len() {
        return None;
    }
    Some(claims)
}

/// The persisted fold encoding's version.
const TAG_FOLD_VERSION: u8 = 1;

/// A cursor over a persisted fold row; every read is bounds-checked and fails as `None`.
#[derive(Debug)]
struct FoldReader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> FoldReader<'a> {
    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(len)?;
        let slice = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(slice)
    }

    fn byte(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_be_bytes(self.take(8)?.try_into().ok()?))
    }

    fn text(&mut self) -> Option<String> {
        let len = usize::try_from(self.u64()?).ok()?;
        String::from_utf8(self.take(len)?.to_vec()).ok()
    }
}

/// Whether `event`'s causal past — its `after` hashes and its backlink, transitively —
/// contains any of `targets`. The graph is hash-committed, so a cycle would need a hash
/// preimage; the visited set keeps the walk finite regardless.
fn causal_past_contains(event: Hash, targets: &[Hash], parents: &HashMap<Hash, Vec<Hash>>) -> bool {
    let targets: std::collections::HashSet<&Hash> = targets.iter().collect();
    let mut seen: std::collections::HashSet<Hash> = std::collections::HashSet::new();
    let mut stack = vec![event];
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        if targets.contains(&id) {
            return true;
        }
        if let Some(parents_of_id) = parents.get(&id) {
            stack.extend(parents_of_id.iter().copied());
        }
    }
    false
}

/// Keep only the claims each marker is authorized to hide in the default view. The group's
/// settled projection is intentional: role is resolved from the store's current group state, not
/// replayed at the marker's causal position. A subject without a stored, applied event with a
/// group extension has no group-admin authority; there is no inferred owner for missing or
/// ungrouped subjects. A marker signed by a device key acts with the effective role the
/// settled projection gives that device in the subject's group (the authorizing account's
/// role capped by the grant's scope), so an Admin-scoped device of an Admin account hides a
/// claim exactly as the account itself would (issue #5656).
fn default_view_events(
    applied: &[Applied],
    subject_groups: &HashMap<Hash, Hash>,
    projection: Option<&Projection>,
) -> Vec<Applied> {
    let claims: HashMap<Hash, (id_core::waiting::AuthorKey, Hash)> = applied
        .iter()
        .filter_map(|event| match &event.op {
            TagOp::Set { subject, .. } => Some((event.id, (event.author, *subject))),
            TagOp::Clear { .. } => None,
        })
        .collect();

    applied
        .iter()
        .map(|event| {
            let mut event = event.clone();
            if let TagOp::Clear { claims: named } = &mut event.op {
                let clear_author = event.author;
                named.retain(|claim_id| {
                    let Some((claim_author, subject)) = claims.get(claim_id) else {
                        return false;
                    };
                    if clear_author == *claim_author {
                        return true;
                    }
                    let Some(group) = subject_groups.get(subject) else {
                        return false;
                    };
                    let Some(projection) = projection else {
                        return false;
                    };
                    projection
                        .group(group)
                        .and_then(|view| view.members.get(&clear_author))
                        == Some(&GroupRole::Admin)
                        || projection.device_role(group, &clear_author) == Some(GroupRole::Admin)
                });
            }
            event
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use id_core::waiting::WaitingLimits;
    use id_store::chain::{sign, ChainExt, Event};
    use id_store::tag_claims::{CLEAR_KIND, SET_KIND};
    use p2panda_core::SigningKey;

    fn open() -> (tempfile::TempDir, EventStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
        (dir, store)
    }

    fn ext(kind: &str, op: TagOp) -> ChainExt {
        ChainExt {
            kind: kind.to_owned(),
            group: None,
            after: Vec::new(),
            tag: Some(op),
            document: None,
            removes: None,
            note: None,
            link: None,
            checkpoint: None,
            rotation: None,
            chat: None,
            anchor: None,
        }
    }

    /// One author's chain of tag events, kept in order.
    struct Chain {
        key: SigningKey,
        seq: u32,
        last: Option<Hash>,
    }

    impl Chain {
        fn new(n: u8) -> Self {
            Self {
                key: SigningKey::from_bytes(&[n; 32]),
                seq: 0,
                last: None,
            }
        }

        fn event(&mut self, kind: &str, op: TagOp) -> Event {
            let event = sign(&self.key, self.seq, self.last, None, ext(kind, op)).unwrap();
            self.seq += 1;
            self.last = Some(event.id);
            event
        }

        fn set(&mut self, subject: Hash, name: &str, value: Option<&str>) -> Event {
            self.event(
                SET_KIND,
                TagOp::Set {
                    subject,
                    name: name.to_owned(),
                    value: value.map(str::to_owned),
                },
            )
        }

        fn clear(&mut self, claims: Vec<Hash>) -> Event {
            self.event(CLEAR_KIND, TagOp::Clear { claims })
        }
    }

    fn subject() -> Hash {
        Hash::digest(b"tag-fold-test-subject")
    }

    /// A store whose author set three claims on the subject and cleared one of them,
    /// plus one claim on a second subject: three live claims.
    fn claimed_store() -> (tempfile::TempDir, EventStore) {
        let (dir, mut store) = open();
        let mut chain = Chain::new(7);
        let s = subject();
        let e1 = chain.set(s, "city", Some("Seattle"));
        store.ingest(&e1.bytes, 0).unwrap();
        let e2 = chain.set(s, "city", Some("Portland"));
        store.ingest(&e2.bytes, 0).unwrap();
        let e3 = chain.set(s, "name", None);
        store.ingest(&e3.bytes, 0).unwrap();
        let other = Hash::digest(b"tag-fold-test-other-subject");
        let e4 = chain.set(other, "city", Some("Oslo"));
        store.ingest(&e4.bytes, 0).unwrap();
        let clear = chain.clear(vec![e3.id]);
        store.ingest(&clear.bytes, 0).unwrap();
        (dir, store)
    }

    #[test]
    fn test_persisted_fold_matches_full_rebuild() {
        // Oracle: the full fold (`fold_tag_claims`) over the same store.
        let (_dir, store) = claimed_store();
        let expected = store.fold_tag_claims().unwrap();
        assert_eq!(expected.len(), 3);
        // The first call rebuilds and persists; the second serves the persisted row.
        assert_eq!(store.tag_claims().unwrap(), expected);
        assert_eq!(store.tag_claims().unwrap(), expected);
        // The index built from the persisted claims answers like a fresh one.
        let index = store.tag_index().unwrap();
        let fresh = TagIndex::new(&expected);
        assert_eq!(index.subject(&subject()), fresh.subject(&subject()));
        assert_eq!(index.named("city"), fresh.named("city"));
        assert_eq!(index.len(), fresh.len());
    }

    #[test]
    fn test_persisted_fold_survives_reopen() {
        let (dir, store) = claimed_store();
        let before = store.tag_claims().unwrap();
        drop(store);

        let reopened = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
        // A row is actually persisted in the reopened store...
        let fingerprint = reopened.fold_fingerprint().unwrap();
        assert!(
            reopened
                .persisted_tag_claims(&fingerprint)
                .unwrap()
                .is_some(),
            "the fold row must survive the reopen"
        );
        // ...and what it serves is identical to a full rebuild of the reopened store.
        assert_eq!(reopened.tag_claims().unwrap(), before);
        assert_eq!(
            reopened.fold_tag_claims().unwrap(),
            before,
            "served claims must equal the full rebuild (oracle)"
        );
    }

    #[test]
    fn test_new_events_invalidate_the_persisted_fold() {
        let (_dir, mut store) = claimed_store();
        let before = store.tag_claims().unwrap();
        // Rebuild the author's chain position from the store: the claimed store
        // ingested five events (seq 0..=4), so the next event is seq 5, backlinked
        // to the clear. Recover the head from a fresh fold instead: use a second
        // author, whose chain starts empty — a new author is also a fingerprint change.
        let mut other = Chain::new(9);
        let extra = other.set(subject(), "extra", Some("x"));
        store.ingest(&extra.bytes, 0).unwrap();
        let after = store.tag_claims().unwrap();
        assert_eq!(after.len(), before.len() + 1);
        assert_eq!(after, store.fold_tag_claims().unwrap());
        // The extra claim's own author clears it: the live set is the original again.
        // Compared as sets: the fold's order follows the event graph, which has grown.
        let clear = other.clear(vec![extra.id]);
        store.ingest(&clear.bytes, 0).unwrap();
        let mut restored = store.tag_claims().unwrap();
        let mut expected = before.clone();
        restored.sort_by_key(|claim| *claim.id.as_bytes());
        expected.sort_by_key(|claim| *claim.id.as_bytes());
        assert_eq!(restored, expected);
    }

    #[test]
    fn test_corrupt_persisted_row_falls_back_to_a_full_rebuild() {
        let (_dir, store) = claimed_store();
        let expected = store.fold_tag_claims().unwrap();
        assert_eq!(store.tag_claims().unwrap(), expected);
        // Corrupt the persisted row in place: undecodable bytes must be a miss,
        // never a wrong answer, and the rebuild must repopulate the row.
        let txn = store.db.begin_write().unwrap();
        {
            let mut table = txn.open_table(TAG_FOLD).unwrap();
            table
                .insert(TAG_FOLD_KEY, [0xde, 0xad, 0xbe, 0xef].as_slice())
                .unwrap();
        }
        txn.commit().unwrap();
        assert_eq!(store.tag_claims().unwrap(), expected);
        let fingerprint = store.fold_fingerprint().unwrap();
        assert!(
            store.persisted_tag_claims(&fingerprint).unwrap().is_some(),
            "the rebuild must repopulate the persisted row"
        );
    }

    #[test]
    fn test_fold_encoding_round_trips() {
        let claims = vec![
            Claim {
                id: Hash::from_bytes([1; 32]),
                author: [2; 32],
                subject: Hash::from_bytes([3; 32]),
                name: "city".to_owned(),
                value: Some("Seattle".to_owned()),
                scope: id_store::tag_claims::TagScope::Shared,
            },
            Claim {
                id: Hash::from_bytes([4; 32]),
                author: [5; 32],
                subject: Hash::from_bytes([6; 32]),
                name: "name".to_owned(),
                value: None,
                scope: id_store::tag_claims::TagScope::Local,
            },
        ];
        let fingerprint = b"fingerprint-bytes";
        let encoded = encode_tag_fold(fingerprint, &claims);
        assert_eq!(decode_tag_fold(&encoded, fingerprint), Some(claims));
        // A different fingerprint, a truncated row, and a foreign row are all misses.
        assert_eq!(decode_tag_fold(&encoded, b"other"), None);
        assert_eq!(decode_tag_fold(&encoded[..4], fingerprint), None);
        assert_eq!(decode_tag_fold(&[9, 9, 9], fingerprint), None);
    }
}
