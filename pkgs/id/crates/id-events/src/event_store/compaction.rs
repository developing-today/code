//! Retention and compaction for the event store (#3797, protocol v1 phase 4).
//!
//! The store otherwise keeps every verified event forever. Compaction is the
//! first reclamation path: under a caller-declared policy it drops the bytes
//! of applied live-class events below a per-author retain frontier, leaving a
//! residue that keeps the event present and applied:
//!
//! - the event's `events`, `bodies` and `groups` rows are removed;
//! - its `hashes` row stays, so presence — and with it `after` resolution and
//!   duplicate detection — never goes away;
//! - its `applied` row stays, and the resolution counts it applied (see
//!   `resolve`), so chains, fork proofs and dependency resolution read the
//!   same as before the bytes went;
//! - a `compacted` row records the residue: the event's ID and its causal
//!   edges (its backlink and its `after` frontier), so the resolution's
//!   causal graph — the graph the membership layer judges positions over —
//!   is the same graph it was before compaction.
//!
//! Durable classes are never compacted, even when a policy names them live
//! (see [`is_durable_kind`]). An event is also kept while its author holds
//! any fork proof (a later fork below an existing point needs the stored
//! bytes), while it is kept as a fork proof header, while a waiting event
//! depends on it, and while it carries a membership action or a device
//! operation (their effects live in the group fold, not only in their
//! bytes). A kind the policy does not declare live is retained: the policy
//! is the caller's retention declaration, and compaction fails closed.
//!
//! The `compacted` table is gained lazily, like `tag_fold`: a store created
//! before the table existed gains it on the first compaction, so no format
//! change is needed.

//! Composition with the two older paths that predate residue (#13401):
//! range sync does not advertise compacted positions — the advertisement
//! is exactly the set whose bytes `sync_bytes` can serve, since an
//! unservable offer aborts the session — and `prune_behind_cut` walks a
//! residue as the causal-graph node it still is, erasing its presence when
//! it falls behind the cut and keeping it as a chain position when it
//! survives (Option A; see that function).

use std::collections::{HashMap, HashSet};

use p2panda_core::{Hash, SeqNum};
use redb::{ReadableMultimapTable, ReadableTable, WriteTransaction};

use id_core::waiting::AuthorKey;

use super::{
    ACTIONS, APPLIED, BODIES, CHAINS, COMPACTED, Chain, DEVICE_OPS, EVENTS, EventStore, FORKS,
    GROUPS, PROOF_HEADERS, PROOF_IDS, StoreError, WAITING_BY_DEP, corrupt, decode_stored,
    lowest_proof, position_key, split_position,
};

/// The caller's retention policy for one compaction pass: which kinds are
/// live-class, and how much of each author's recent history is always kept.
///
/// The policy is the channel/session retention declaration of the #2036
/// design (§4.6) in caller form: the design's per-channel policy object,
/// supplied at compaction time rather than read from event metadata. A kind
/// that is durable by construction (see [`is_durable_kind`]) is retained
/// whatever this policy says.
#[derive(Clone, Debug, Default)]
pub struct CompactionPolicy {
    /// The event kinds the caller declares live (bounded-retention) class.
    pub live_kinds: HashSet<String>,
    /// How many of an author's most recent applied events are always
    /// retained. Events below `next_seq - retain_recent` may be compacted;
    /// the author's applied head is never compacted, whatever this says.
    pub retain_recent: u32,
}

/// Whether `kind` is durable by construction, per the class table of the
/// #2034 investigation (§3/§7): identity and membership material, signed
/// durable content, attestations, writer-set and device operations, media,
/// chat anchors, thread resolutions and proposals, talk-page posts and session
/// checkpoints are kept forever. Checkpoint events
/// in particular are never deleted by live-data compaction (design §4.6).
pub(super) fn is_durable_kind(kind: &str) -> bool {
    const DURABLE_EXACT: [&str; 6] = [
        "identity",
        "issuer.rotation",
        "chat.anchor.v1",
        "session.checkpoint",
        "group",
        // Talk-page posts are never edited or dropped (#5834 invariant 1):
        // a superseded post stays stored, so compaction keeps it too.
        id_store::talk_page::TALK_POST_KIND,
    ];
    const DURABLE_PREFIX: [&str; 8] = [
        "group.",
        "document.",
        "tag.",
        "note.",
        "attestation.",
        "idset.",
        "device.",
        "media.",
    ];
    DURABLE_EXACT.contains(&kind)
        || DURABLE_PREFIX.iter().any(|prefix| kind.starts_with(prefix))
        // Resolutions, proposals and what closed them are the audit trail of a
        // discussion (#7849, decision record #5874 §4), so compaction keeps them.
        || id_store::thread_resolution::is_thread_event_kind(kind)
}

/// What the store keeps of a compacted event: its ID and its causal edges.
/// The position (author and sequence number) is the row's key, as in
/// `events`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Residue {
    pub author: AuthorKey,
    pub seq: SeqNum,
    pub id: Hash,
    pub backlink: Option<Hash>,
    pub after: Vec<Hash>,
}

/// Encodes a residue row's value: the event ID, the backlink (a flag byte,
/// then the hash when present), the `after` count, then the `after` hashes.
fn encode_residue(residue: &Residue) -> Result<Vec<u8>, StoreError> {
    let count = u32::try_from(residue.after.len())
        .map_err(|_err| corrupt("a residue names too many dependencies"))?;
    let mut out = Vec::with_capacity(32 + 1 + 32 + 4 + 32 * residue.after.len());
    out.extend_from_slice(residue.id.as_bytes());
    match residue.backlink {
        Some(backlink) => {
            out.push(1);
            out.extend_from_slice(backlink.as_bytes());
        }
        None => out.push(0),
    }
    out.extend_from_slice(&count.to_be_bytes());
    for dep in &residue.after {
        out.extend_from_slice(dep.as_bytes());
    }
    Ok(out)
}

/// Decodes a residue row's value. The row's key names the position, so the
/// author and sequence number come from there.
fn decode_residue(author: AuthorKey, seq: SeqNum, bytes: &[u8]) -> Result<Residue, StoreError> {
    fn hash_at(bytes: &[u8], offset: usize) -> Result<Hash, StoreError> {
        let raw: [u8; 32] = bytes
            .get(offset..offset + 32)
            .and_then(|slice| slice.try_into().ok())
            .ok_or_else(|| corrupt("a residue is shorter than its contents"))?;
        Ok(Hash::from(raw))
    }
    let id = hash_at(bytes, 0)?;
    let mut offset = 32;
    let backlink = match bytes.get(offset) {
        Some(0) => {
            offset += 1;
            None
        }
        Some(1) => {
            offset += 1;
            let backlink = hash_at(bytes, offset)?;
            offset += 32;
            Some(backlink)
        }
        _ => return Err(corrupt("a residue has a bad backlink flag")),
    };
    let count: [u8; 4] = bytes
        .get(offset..offset + 4)
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(|| corrupt("a residue is shorter than its contents"))?;
    offset += 4;
    let count = u32::from_be_bytes(count) as usize;
    let mut after = Vec::with_capacity(count);
    for _ in 0..count {
        after.push(hash_at(bytes, offset)?);
        offset += 32;
    }
    if offset != bytes.len() {
        return Err(corrupt("a residue has trailing bytes"));
    }
    Ok(Residue {
        author,
        seq,
        id,
        backlink,
        after,
    })
}

/// Every residue row in the table, in position order.
pub(super) fn load_residue(
    table: &impl ReadableTable<&'static [u8], &'static [u8]>,
) -> Result<Vec<Residue>, StoreError> {
    let mut found = Vec::new();
    for row in table.iter()? {
        let (position, value) = row?;
        let (author, seq) = split_position(position.value())?;
        found.push(decode_residue(author, seq, value.value())?);
    }
    Ok(found)
}

/// The residue at `(author, seq)`, when that position was compacted.
pub(super) fn residue_at(
    table: &impl ReadableTable<&'static [u8], &'static [u8]>,
    author: &AuthorKey,
    seq: SeqNum,
) -> Result<Option<Residue>, StoreError> {
    let key = position_key(author, seq);
    match table.get(key.as_slice())? {
        Some(row) => Ok(Some(decode_residue(*author, seq, row.value())?)),
        None => Ok(None),
    }
}

/// Every residue row, read through a read transaction. A store that has
/// never been compacted may not have the table yet; that reads as no
/// residue, not as an error.
pub(super) fn load_residue_read(txn: &redb::ReadTransaction) -> Result<Vec<Residue>, StoreError> {
    match txn.open_table(COMPACTED) {
        Ok(table) => load_residue(&table),
        Err(redb::TableError::TableDoesNotExist(_)) => Ok(Vec::new()),
        Err(err) => Err(err.into()),
    }
}

impl EventStore {
    /// Compacts the stored events `policy` makes eligible, in one write
    /// transaction: all of them, or none. Returns how many events were
    /// compacted. See the module docs for what a compacted event keeps and
    /// what it loses (its bytes, and only its bytes).
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Unusable`] if a past write failed and the pool
    /// could not be rebuilt, a redb error if the write fails, or
    /// [`StoreError::Corrupt`] if a stored row does not decode.
    pub fn compact(&mut self, policy: &CompactionPolicy) -> Result<usize, StoreError> {
        if self.unusable {
            return Err(StoreError::Unusable);
        }
        let txn = self.db.begin_write()?;
        let count = compact_in(&txn, policy)?;
        txn.commit()?;
        Ok(count)
    }
}

/// The compaction pass itself, inside the caller's transaction.
fn compact_in(txn: &WriteTransaction, policy: &CompactionPolicy) -> Result<usize, StoreError> {
    // The per-author state the eligibility rules read.
    let mut chains: HashMap<AuthorKey, Chain> = HashMap::new();
    {
        let table = txn.open_table(CHAINS)?;
        for row in table.iter()? {
            let (author, bytes) = row?;
            let author: AuthorKey = author
                .value()
                .try_into()
                .map_err(|_err| corrupt("a chain row has a short author key"))?;
            chains.insert(author, Chain::decode(bytes.value())?);
        }
    }
    let forks = txn.open_table(FORKS)?;
    let applied = txn.open_table(APPLIED)?;
    let proof_ids = txn.open_table(PROOF_IDS)?;
    let proof_headers = txn.open_table(PROOF_HEADERS)?;
    let actions = txn.open_table(ACTIONS)?;
    let device_ops = txn.open_table(DEVICE_OPS)?;
    let by_dep = txn.open_multimap_table(WAITING_BY_DEP)?;

    let mut proof_by_author: HashMap<AuthorKey, bool> = HashMap::new();
    let mut eligible: Vec<(Residue, Option<Hash>)> = Vec::new();
    {
        let events = txn.open_table(EVENTS)?;
        for row in events.iter()? {
            let (position, bytes) = row?;
            let (author, seq) = split_position(position.value())?;
            let event = decode_stored(bytes.value())?;
            let kind = &event.header.extensions.kind;
            if !policy.live_kinds.contains(kind) || is_durable_kind(kind) {
                continue;
            }
            // The author's applied frontier: below it, and never the head.
            let Some(chain) = chains.get(&author) else {
                continue;
            };
            if chain.frozen {
                continue;
            }
            let proof = if let Some(found) = proof_by_author.get(&author) {
                *found
            } else {
                let found = lowest_proof(&forks, &author)?.is_some();
                proof_by_author.insert(author, found);
                found
            };
            if proof {
                continue;
            }
            let next = u64::from(chain.next_seq);
            let frontier = next.saturating_sub(u64::from(policy.retain_recent));
            if u64::from(seq) >= frontier || u64::from(seq) + 1 >= next {
                continue;
            }
            let id = event.id;
            if applied.get(id.as_bytes().as_slice())?.is_none() {
                continue;
            }
            // Kept while it is a fork proof header, while it carries an
            // action or a device operation, or while a waiting event
            // depends on it.
            if proof_ids.get(id.as_bytes().as_slice())?.is_some()
                || proof_headers.get(id.as_bytes().as_slice())?.is_some()
                || actions.get(id.as_bytes().as_slice())?.is_some()
                || device_ops.get(id.as_bytes().as_slice())?.is_some()
            {
                continue;
            }
            let waited_on = by_dep
                .get(id.as_bytes().as_slice())?
                .next()
                .transpose()?
                .is_some();
            if waited_on {
                continue;
            }
            eligible.push((
                Residue {
                    author,
                    seq,
                    id,
                    backlink: event.header.backlink,
                    after: event.header.extensions.after.clone(),
                },
                event.header.extensions.group,
            ));
        }
    }

    let mut events = txn.open_table(EVENTS)?;
    let mut bodies = txn.open_table(BODIES)?;
    let mut groups = txn.open_multimap_table(GROUPS)?;
    let mut compacted = txn.open_table(COMPACTED)?;
    for (residue, group) in &eligible {
        let key = position_key(&residue.author, residue.seq);
        events.remove(key.as_slice())?;
        bodies.remove(residue.id.as_bytes().as_slice())?;
        if let Some(group) = group {
            groups.remove(group.as_bytes().as_slice(), key.as_slice())?;
        }
        compacted.insert(key.as_slice(), encode_residue(residue)?.as_slice())?;
    }
    Ok(eligible.len())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_possible_truncation
)]
mod tests {
    use super::*;
    use crate::event_store::{Ingest, Validity};
    use id_core::waiting::WaitingLimits;
    use id_store::chain::{ChainExt, Event, sign};
    use p2panda_core::SigningKey;

    fn key(n: u8) -> SigningKey {
        SigningKey::from_bytes(&[n; 32])
    }

    fn author(key: &SigningKey) -> AuthorKey {
        *key.verifying_key().as_bytes()
    }

    fn ext(kind: &str, group: Option<Hash>, after: Vec<Hash>) -> ChainExt {
        ChainExt {
            kind: kind.to_owned(),
            group,
            after,
            tag: None,
            document: (kind == "document.put").then(|| id_store::documents::DocumentRef {
                media_type: "text/plain".to_owned(),
                content: Hash::digest(b"document"),
                size: 8,
            }),
            removes: None,
            note: None,
            link: None,
            checkpoint: None,
            rotation: None,
            chat: (kind == "chat.message.v1")
                .then(|| id_store::chat::ChatMessage::new("hello", Vec::new()).unwrap()),
            anchor: None,
            thread: None,
        }
    }

    /// A valid session checkpoint payload for `key`'s session: segment
    /// `segment_seq`, naming `prev` as the predecessor checkpoint.
    fn checkpoint_payload(
        key: &SigningKey,
        segment_seq: u64,
        prev: Option<Hash>,
    ) -> id_store::session_checkpoint::SessionCheckpoint {
        use id_store::session_checkpoint::{
            MacScope, SegmentCheckpoint, SessionCheckpoint, SessionHeader,
        };
        SessionCheckpoint {
            header: SessionHeader {
                session_id: Hash::digest(b"session"),
                purpose: "collab editing".to_owned(),
                participants: vec![key.verifying_key()],
                started_ms: 1_000,
                nonce: Hash::digest(b"nonce"),
                mac_key_commitment: Hash::digest(b"mac key"),
                mac_scope: MacScope::Shared,
            },
            segment: SegmentCheckpoint {
                segment_seq,
                prev_checkpoint: prev,
                root: Hash::digest(b"root"),
                event_count: 3,
                start_ms: 1_000,
                end_ms: 1_500,
                audit_blob: None,
            },
        }
    }

    /// The first `count` checkpoints of `key`'s session, each naming the one
    /// before as its predecessor.
    fn checkpoint_chain(key: &SigningKey, count: u32) -> Vec<Event> {
        let mut events = Vec::new();
        for seq in 0..count {
            let prev = events.last().map(|event: &Event| event.id);
            let mut ext = ext("session.checkpoint", None, Vec::new());
            ext.checkpoint = Some(checkpoint_payload(key, u64::from(seq), prev));
            events.push(sign(key, SeqNum::from(seq), prev, None, ext).unwrap());
        }
        events
    }

    /// The first `count` events of `key`'s chain in `kind`, each naming the
    /// one before as its backlink.
    fn chain_of(key: &SigningKey, count: u32, kind: &str) -> Vec<Event> {
        let mut events = Vec::new();
        for seq in 0..count {
            let backlink = events.last().map(|event: &Event| event.id);
            let event = sign(
                key,
                SeqNum::from(seq),
                backlink,
                None,
                ext(kind, None, Vec::new()),
            )
            .unwrap();
            events.push(event);
        }
        events
    }

    /// A rival's extensions: a chat message whose text differs from the
    /// original's, so the rival is a distinct event at the same position.
    fn rival_ext() -> ChainExt {
        let mut ext = ext("chat.message.v1", None, Vec::new());
        ext.chat = Some(id_store::chat::ChatMessage::new("a rival", Vec::new()).unwrap());
        ext
    }

    fn open_temp() -> (tempfile::TempDir, EventStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
        (dir, store)
    }

    fn policy(kinds: &[&str], retain_recent: u32) -> CompactionPolicy {
        CompactionPolicy {
            live_kinds: kinds.iter().map(ToString::to_string).collect(),
            retain_recent,
        }
    }

    fn ingest_all(store: &mut EventStore, events: &[Event]) {
        for event in events {
            assert!(
                matches!(
                    store.ingest(&event.bytes, 0).unwrap(),
                    Ingest::Accepted { .. }
                ),
                "ingest of seq {} was not accepted",
                u64::from(event.header.seq_num)
            );
        }
    }

    #[test]
    fn durable_kinds_are_never_compacted_even_when_declared_live() {
        let (_dir, mut store) = open_temp();
        let documents = chain_of(&key(41), 4, "document.put");
        ingest_all(&mut store, &documents);
        let checkpoints = checkpoint_chain(&key(42), 3);
        ingest_all(&mut store, &checkpoints);
        let count = store
            .compact(&policy(&["document.put", "session.checkpoint"], 0))
            .unwrap();
        assert_eq!(count, 0);
        for (signer, events) in [(key(41), &documents), (key(42), &checkpoints)] {
            for event in events {
                assert_eq!(
                    store
                        .event(&author(&signer), event.header.seq_num)
                        .unwrap()
                        .map(|found| found.id),
                    Some(event.id),
                    "a durable event lost its bytes"
                );
            }
        }
        store.check_invariants().unwrap();
    }

    #[test]
    fn only_policy_declared_kinds_compact() {
        let (_dir, mut store) = open_temp();
        let chat = chain_of(&key(43), 4, "chat.message.v1");
        ingest_all(&mut store, &chat);
        let records = chain_of(&key(44), 4, "didit.record");
        ingest_all(&mut store, &records);
        let count = store.compact(&policy(&["chat.message.v1"], 1)).unwrap();
        assert_eq!(count, 3, "only the declared live kind compacts");
        for event in &records {
            assert!(
                store
                    .event(&author(&key(44)), event.header.seq_num)
                    .unwrap()
                    .is_some(),
                "an undeclared kind lost its bytes"
            );
        }
        store.check_invariants().unwrap();
    }

    #[test]
    fn live_events_compact_below_the_frontier_and_stay_present_and_applied() {
        let (_dir, mut store) = open_temp();
        let signer = key(45);
        let who = author(&signer);
        let events = chain_of(&signer, 6, "chat.message.v1");
        ingest_all(&mut store, &events);
        let count = store.compact(&policy(&["chat.message.v1"], 2)).unwrap();
        assert_eq!(count, 4);
        // Compaction is idempotent: a second pass finds nothing new.
        assert_eq!(store.compact(&policy(&["chat.message.v1"], 2)).unwrap(), 0);
        for (index, event) in events.iter().enumerate() {
            let stored = store.event(&who, event.header.seq_num).unwrap();
            if index < 4 {
                assert!(stored.is_none(), "compacted seq {index} kept its bytes");
                assert!(store.event_by_id(&event.id).unwrap().is_none());
            } else {
                assert_eq!(stored.map(|found| found.id), Some(event.id));
            }
            // Present and applied either way: presence never goes away.
            assert_eq!(
                store.validity_now(&event.id).unwrap(),
                Validity::Applies,
                "seq {index} changed its verdict"
            );
        }
        let chain = store.chain(&who).unwrap().expect("a chain row");
        assert_eq!(u64::from(chain.next_seq), 6);
        assert_eq!(chain.head, Some(events[5].id));
        store.check_invariants().unwrap();
    }

    #[test]
    fn a_mixed_chain_keeps_its_durable_and_checkpoint_events() {
        let (_dir, mut store) = open_temp();
        let signer = key(46);
        let who = author(&signer);
        let kinds = [
            "chat.message.v1",
            "session.checkpoint",
            "chat.message.v1",
            "document.put",
            "chat.message.v1",
            "chat.message.v1",
        ];
        let mut events = Vec::new();
        for (seq, kind) in kinds.iter().enumerate() {
            let backlink = events.last().map(|event: &Event| event.id);
            let mut ext = ext(kind, None, Vec::new());
            if *kind == "session.checkpoint" {
                ext.checkpoint = Some(checkpoint_payload(&signer, 0, None));
            }
            let event = sign(&signer, SeqNum::from(seq as u32), backlink, None, ext).unwrap();
            events.push(event);
        }
        ingest_all(&mut store, &events);
        let count = store
            .compact(&policy(
                &["chat.message.v1", "session.checkpoint", "document.put"],
                1,
            ))
            .unwrap();
        assert_eq!(count, 3, "only the live events below the frontier");
        for (index, event) in events.iter().enumerate() {
            let stored = store.event(&who, event.header.seq_num).unwrap();
            match index {
                1 | 3 | 5 => assert!(
                    stored.is_some(),
                    "seq {index} (durable, checkpoint, or head) lost its bytes"
                ),
                _ => assert!(stored.is_none(), "seq {index} kept its bytes"),
            }
        }
        store.check_invariants().unwrap();
    }

    #[test]
    fn an_event_naming_a_compacted_event_in_after_is_still_accepted() {
        let (_dir, mut store) = open_temp();
        let alice = key(47);
        let events = chain_of(&alice, 4, "chat.message.v1");
        ingest_all(&mut store, &events);
        assert_eq!(store.compact(&policy(&["chat.message.v1"], 1)).unwrap(), 3);
        // Bob's event names Alice's compacted first event as a dependency:
        // the hash stayed, so the dependency is present and Bob applies.
        let bob = key(48);
        let dependent = sign(
            &bob,
            SeqNum::from(0u32),
            None,
            None,
            ext("didit.record", None, vec![events[0].id]),
        )
        .unwrap();
        assert!(matches!(
            store.ingest(&dependent.bytes, 0).unwrap(),
            Ingest::Accepted { .. }
        ));
        assert_eq!(
            store.validity_now(&dependent.id).unwrap(),
            Validity::Applies
        );
        store.check_invariants().unwrap();
    }

    #[test]
    fn the_authors_next_event_applies_after_compaction() {
        let (_dir, mut store) = open_temp();
        let signer = key(49);
        let who = author(&signer);
        let events = chain_of(&signer, 4, "chat.message.v1");
        ingest_all(&mut store, &events);
        assert_eq!(store.compact(&policy(&["chat.message.v1"], 1)).unwrap(), 3);
        let next = sign(
            &signer,
            SeqNum::from(4u32),
            Some(events[3].id),
            None,
            ext("chat.message.v1", None, Vec::new()),
        )
        .unwrap();
        assert!(matches!(
            store.ingest(&next.bytes, 0).unwrap(),
            Ingest::Accepted { .. }
        ));
        assert_eq!(store.validity_now(&next.id).unwrap(), Validity::Applies);
        let chain = store.chain(&who).unwrap().expect("a chain row");
        assert_eq!(u64::from(chain.next_seq), 5);
        assert_eq!(chain.head, Some(next.id));
        store.check_invariants().unwrap();
    }

    #[test]
    fn a_forked_authors_events_are_never_compacted_and_proofs_survive() {
        let (_dir, mut store) = open_temp();
        // Alice forks at seq 2: nothing of hers may compact (rule: an author
        // holding a fork proof keeps every stored byte).
        let alice = key(50);
        let alice_events = chain_of(&alice, 4, "chat.message.v1");
        ingest_all(&mut store, &alice_events);
        let rival = sign(
            &alice,
            SeqNum::from(2u32),
            Some(alice_events[1].id),
            None,
            rival_ext(),
        )
        .unwrap();
        assert!(matches!(
            store.ingest(&rival.bytes, 0).unwrap(),
            Ingest::Fork { .. }
        ));
        // Bob's identical chain compacts normally in the same pass.
        let bob = key(51);
        let bob_events = chain_of(&bob, 4, "chat.message.v1");
        ingest_all(&mut store, &bob_events);
        let count = store.compact(&policy(&["chat.message.v1"], 1)).unwrap();
        assert_eq!(count, 3, "only Bob's events compact");
        for event in &alice_events {
            assert!(
                store
                    .event(&author(&alice), event.header.seq_num)
                    .unwrap()
                    .is_some(),
                "a forked author's event lost its bytes"
            );
        }
        // Alice's fork proof still resolves, and Bob's chain still verifies.
        let proof = store
            .fork_proof(&author(&alice))
            .unwrap()
            .expect("a fork proof");
        assert_eq!(u64::from(proof.seq), 2);
        assert_eq!(
            store.validity_now(&bob_events[0].id).unwrap(),
            Validity::Applies
        );
        // A fresh ingest after compaction runs the full resolution over the
        // residue: Bob's next event applies and his chain advances.
        let next = sign(
            &bob,
            SeqNum::from(4u32),
            Some(bob_events[3].id),
            None,
            ext("chat.message.v1", None, Vec::new()),
        )
        .unwrap();
        assert!(matches!(
            store.ingest(&next.bytes, 0).unwrap(),
            Ingest::Accepted { .. }
        ));
        let chain = store.chain(&author(&bob)).unwrap().expect("a chain row");
        assert_eq!(u64::from(chain.next_seq), 5);
        store.check_invariants().unwrap();
    }

    #[test]
    fn a_rival_at_a_compacted_position_is_parked_never_forked() {
        let (_dir, mut store) = open_temp();
        let signer = key(52);
        let who = author(&signer);
        let events = chain_of(&signer, 4, "chat.message.v1");
        ingest_all(&mut store, &events);
        assert_eq!(store.compact(&policy(&["chat.message.v1"], 1)).unwrap(), 3);
        // A rival at compacted seq 1: its bytes conflict with an event whose
        // bytes are gone, so no fork proof can be written from them. The
        // rival is held (parked), the chain is untouched, and no proof
        // appears.
        let rival = sign(
            &signer,
            SeqNum::from(1u32),
            Some(events[0].id),
            None,
            rival_ext(),
        )
        .unwrap();
        assert!(matches!(
            store.ingest(&rival.bytes, 0).unwrap(),
            Ingest::Waiting { .. }
        ));
        assert!(store.waiting_event(&rival.id).unwrap().is_some());
        assert!(store.fork_proof(&who).unwrap().is_none());
        assert_eq!(
            store.validity_now(&events[1].id).unwrap(),
            Validity::Applies,
            "the compacted event kept its verdict"
        );
        let chain = store.chain(&who).unwrap().expect("a chain row");
        assert_eq!(u64::from(chain.next_seq), 4);
        store.check_invariants().unwrap();
    }

    #[test]
    fn group_membership_survives_compacting_the_groups_messages() {
        use id_core::membership::AuthId;
        use p2panda_auth::group::GroupAction;

        let (_dir, mut store) = open_temp();
        let alice = key(55);
        let who = author(&alice);
        // Alice creates a group she manages. The group is the create's ID.
        let create = GroupAction::Create {
            initial_members: vec![(
                p2panda_auth::group::GroupMember::Individual(AuthId::Key(who)),
                p2panda_auth::Access::manage(),
            )],
        };
        let create_body = id_core::membership::body::encode_body(&create).unwrap();
        let create_event = sign(
            &alice,
            SeqNum::from(0u32),
            None,
            Some(&create_body),
            ext("group.op", None, Vec::new()),
        )
        .unwrap();
        let group = create_event.id;
        store
            .ingest_with_body(&create_event.bytes, Some(&create_body), 0)
            .unwrap();
        // Three messages in the group; the first names the create as its
        // dependency, the rest chain by backlink.
        let mut messages: Vec<Event> = Vec::new();
        for index in 1..=3u32 {
            let (backlink, after) = if index == 1 {
                (Some(create_event.id), vec![create_event.id])
            } else {
                (Some(messages[(index - 2) as usize].id), Vec::new())
            };
            let message = sign(
                &alice,
                SeqNum::from(index),
                backlink,
                None,
                ext("chat.message.v1", Some(group), after),
            )
            .unwrap();
            messages.push(message);
        }
        ingest_all(&mut store, &messages);
        assert_eq!(store.roster(&group).unwrap(), vec![who]);
        // The create is durable and carries the group's action; the first
        // two messages compact, the head message is retained.
        let count = store.compact(&policy(&["chat.message.v1"], 0)).unwrap();
        assert_eq!(count, 2);
        assert_eq!(
            store.roster(&group).unwrap(),
            vec![who],
            "the roster reads through the residue"
        );
        // A further message still applies: its causal past runs over the
        // compacted messages' residue edges to the create.
        let next = sign(
            &alice,
            SeqNum::from(4u32),
            Some(messages[2].id),
            None,
            ext("chat.message.v1", Some(group), Vec::new()),
        )
        .unwrap();
        assert!(matches!(
            store.ingest(&next.bytes, 0).unwrap(),
            Ingest::Accepted { .. }
        ));
        assert_eq!(store.validity_now(&next.id).unwrap(), Validity::Applies);
        store.check_invariants().unwrap();
    }

    #[test]
    fn reopening_a_compacted_store_keeps_chains_proofs_and_verdicts() {
        let (dir, mut store) = open_temp();
        let alice = key(53);
        let alice_events = chain_of(&alice, 4, "chat.message.v1");
        ingest_all(&mut store, &alice_events);
        let bob = key(54);
        let bob_events = chain_of(&bob, 3, "chat.message.v1");
        ingest_all(&mut store, &bob_events);
        let rival = sign(
            &bob,
            SeqNum::from(1u32),
            Some(bob_events[0].id),
            None,
            rival_ext(),
        )
        .unwrap();
        assert!(matches!(
            store.ingest(&rival.bytes, 0).unwrap(),
            Ingest::Fork { .. }
        ));
        assert_eq!(store.compact(&policy(&["chat.message.v1"], 1)).unwrap(), 3);
        drop(store);
        let store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
        store.check_invariants().unwrap();
        let chain = store.chain(&author(&alice)).unwrap().expect("a chain row");
        assert_eq!(u64::from(chain.next_seq), 4);
        assert_eq!(chain.head, Some(alice_events[3].id));
        assert_eq!(
            store.validity_now(&alice_events[0].id).unwrap(),
            Validity::Applies
        );
        assert!(
            store.fork_proof(&author(&bob)).unwrap().is_some(),
            "the fork proof survived the reopen"
        );
    }
    fn residue_count(store: &EventStore) -> usize {
        use redb::ReadableDatabase;
        load_residue_read(&store.db.begin_read().unwrap())
            .unwrap()
            .len()
    }

    /// Oracle (#13401, Option A): a residue behind the cut that nothing
    /// protects is erased like a stored event. Its presence goes (the
    /// verdict is `Unknown`, the residue row and the chain row are gone),
    /// so the same events ingest again as new, not as duplicates.
    #[test]
    fn prune_erases_residue_behind_the_cut_with_its_presence() {
        let (_dir, mut store) = open_temp();
        let signer = key(51);
        let who = author(&signer);
        let events = chain_of(&signer, 6, "chat.message.v1");
        ingest_all(&mut store, &events);
        assert_eq!(
            store.compact(&policy(&["chat.message.v1"], 2)).unwrap(),
            4,
            "seqs 0..3 are residue, seqs 4 and 5 stay stored"
        );
        let pruned = store.prune_behind_cut(&[events[5].id], &[]).unwrap();
        assert_eq!(pruned, 6, "four residue positions and two stored events");
        assert_eq!(residue_count(&store), 0);
        assert_eq!(store.chain(&who).unwrap(), None);
        assert!(store.sync_entries().unwrap().is_empty());
        for (index, event) in events.iter().enumerate() {
            assert_eq!(
                store.validity_now(&event.id).unwrap(),
                Validity::Unknown,
                "seq {index} kept its presence through the prune"
            );
        }
        store.check_invariants().unwrap();
        ingest_all(&mut store, &events);
        let chain = store.chain(&who).unwrap().expect("a chain row");
        assert_eq!(u64::from(chain.next_seq), 6);
        store.check_invariants().unwrap();
    }

    /// Oracle (#13401, Option A): the chain rewrite counts a surviving
    /// residue position as applied. With the residue prefix protected and
    /// the stored tail pruned, the chain ends at the last residue, which is
    /// its head, and the author's next event chains onto it.
    #[test]
    fn prune_keeps_a_surviving_residue_as_the_chain_head() {
        let (_dir, mut store) = open_temp();
        let signer = key(53);
        let who = author(&signer);
        let events = chain_of(&signer, 6, "chat.message.v1");
        ingest_all(&mut store, &events);
        assert_eq!(store.compact(&policy(&["chat.message.v1"], 2)).unwrap(), 4);
        let pruned = store
            .prune_behind_cut(&[events[5].id], &[events[3].id])
            .unwrap();
        assert_eq!(
            pruned, 2,
            "only the stored tail is behind the cut and unprotected"
        );
        assert_eq!(residue_count(&store), 4);
        let chain = store.chain(&who).unwrap().expect("a chain row");
        assert_eq!(u64::from(chain.next_seq), 4);
        assert_eq!(
            chain.head,
            Some(events[3].id),
            "the head is the last residue"
        );
        for (index, event) in events.iter().enumerate() {
            let expected = if index < 4 {
                Validity::Applies
            } else {
                Validity::Unknown
            };
            assert_eq!(
                store.validity_now(&event.id).unwrap(),
                expected,
                "seq {index}"
            );
        }
        store.check_invariants().unwrap();
        ingest_all(&mut store, &events[4..5]);
        let chain = store.chain(&who).unwrap().expect("a chain row");
        assert_eq!(u64::from(chain.next_seq), 5);
        assert_eq!(chain.head, Some(events[4].id));
        store.check_invariants().unwrap();
    }

    /// Oracle (#13401, Option A): the walks follow a residue's recorded
    /// edges. Bob's surviving event names Alice's compacted seq 1 in
    /// `after`; that residue and its backlinked residue survive, the rest
    /// of Alice's chain behind the cut (residue and stored alike) goes,
    /// and Bob's event still rests on a present dependency.
    #[test]
    fn prune_keeps_residue_a_surviving_event_depends_on() {
        let (_dir, mut store) = open_temp();
        let alice = key(54);
        let events = chain_of(&alice, 6, "chat.message.v1");
        ingest_all(&mut store, &events);
        let dependent = sign(
            &key(55),
            SeqNum::from(0u32),
            None,
            None,
            ext("didit.record", None, vec![events[1].id]),
        )
        .unwrap();
        ingest_all(&mut store, std::slice::from_ref(&dependent));
        assert_eq!(store.compact(&policy(&["chat.message.v1"], 2)).unwrap(), 4);
        let pruned = store.prune_behind_cut(&[events[5].id], &[]).unwrap();
        assert_eq!(pruned, 4, "Alice's seqs 2..5: two residue, two stored");
        assert_eq!(residue_count(&store), 2);
        let chain = store.chain(&author(&alice)).unwrap().expect("a chain row");
        assert_eq!(u64::from(chain.next_seq), 2);
        assert_eq!(chain.head, Some(events[1].id));
        assert_eq!(
            store.validity_now(&dependent.id).unwrap(),
            Validity::Applies
        );
        assert_eq!(
            store.validity_now(&events[1].id).unwrap(),
            Validity::Applies
        );
        assert_eq!(
            store.validity_now(&events[2].id).unwrap(),
            Validity::Unknown
        );
        store.check_invariants().unwrap();
    }

    /// Oracle (#13401): a compaction pass that compacted nothing leaves
    /// no residue, and the prune runs exactly as it did before residue
    /// existed.
    #[test]
    fn prune_runs_when_compaction_compacted_nothing() {
        let (_dir, mut store) = open_temp();
        let events = chain_of(&key(52), 4, "chat.message.v1");
        ingest_all(&mut store, &events);
        assert_eq!(
            store.compact(&policy(&["didit.record"], 0)).unwrap(),
            0,
            "no event matches the policy, so nothing is residue"
        );
        let pruned = store.prune_behind_cut(&[events[3].id], &[]).unwrap();
        assert_eq!(pruned, 4);
        store.check_invariants().unwrap();
    }
}
