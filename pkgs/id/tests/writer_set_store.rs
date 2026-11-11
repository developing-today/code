//! Store-backed writer-set wiring (writer-set spec §§4–5; issue #6690,
//! follow-up to #5545): a writer-set event ingested with its body is
//! retained by the store, reads back checked against its header, expands
//! over the store's resolved group state, and is found by the discovery
//! query. The oracle is the pure projection itself
//! ([`id::idset::view::expand_set`]) fed the same set: the store path must
//! agree with it, and a body that disagrees with its header must never
//! become a usable object.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use id::chain::{ChainExt, sign};
use id::event_store::{EventStore, Ingest, WriterSetStoreError};
use id::idset::view::expand_set;
use id::idset::{
    Entry, Excludes, RESCIND_KIND, Rescind, WRITER_KIND, WriterSet, encode_rescind_body,
    encode_writer_body,
};
use id::waiting::{AuthorKey, WaitingLimits};
use p2panda_core::{Hash, SigningKey};

fn author_of(key: &SigningKey) -> AuthorKey {
    *key.verifying_key().as_bytes()
}

fn ext(kind: &str) -> ChainExt {
    ChainExt {
        kind: kind.to_owned(),
        group: None,
        after: vec![],
        tag: None,
        document: None,
        removes: None,
        note: None,
        link: None,
        checkpoint: None,
        rotation: None,
        chat: None,
    }
}

fn store() -> (tempfile::TempDir, EventStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
    (dir, store)
}

const ALICE: u8 = 21;
const BOB: u8 = 22;
const CAROL: u8 = 23;

fn key_author(n: u8) -> AuthorKey {
    [n; 32]
}

/// A set for scope `metrics`: Alice and Bob admitted, Bob excluded again —
/// exclusions win, so the expansion admits Alice only.
fn metrics_set(owner: AuthorKey, version: u64) -> WriterSet {
    WriterSet {
        set_id: WriterSet::derive_set_id(&owner, "metrics"),
        version,
        owner,
        scope: Some("metrics".to_owned()),
        entries: vec![
            Entry::Key {
                author: key_author(ALICE),
                include_history: true,
            },
            Entry::Key {
                author: key_author(BOB),
                include_history: false,
            },
        ],
        excludes: Excludes(vec![Entry::Key {
            author: key_author(BOB),
            include_history: false,
        }]),
    }
}

/// Signs `set` at its own version on `owner`'s chain and ingests it with its
/// body. Returns the publishing event's ID.
fn publish(
    store: &mut EventStore,
    key: &SigningKey,
    set: &WriterSet,
    backlink: Option<Hash>,
) -> Hash {
    let body = encode_writer_body(set).unwrap();
    let event = sign(
        key,
        u32::try_from(set.version).unwrap(),
        backlink,
        Some(&body),
        ext(WRITER_KIND),
    )
    .unwrap();
    let result = store
        .ingest_with_body(&event.bytes, Some(&body), 0)
        .unwrap();
    assert!(
        matches!(result, Ingest::Accepted { .. }),
        "writer-set event was not accepted: {result:?}"
    );
    event.id
}

#[test]
fn test_stored_writer_set_reads_back_and_expands_like_the_pure_projection() {
    let (_dir, mut store) = store();
    let key = SigningKey::from_bytes(&[7; 32]);
    let owner = author_of(&key);
    let set = metrics_set(owner, 0);
    let id = publish(&mut store, &key, &set, None);

    // The retained body reads back as the published object.
    assert_eq!(store.stored_writer_set(&id).unwrap(), Some(set.clone()));
    assert_eq!(
        store.writer_set_body(&id).unwrap(),
        Some(encode_writer_body(&set).unwrap())
    );

    // The store expansion agrees with the pure projection over an empty
    // group snapshot set (the set names no groups).
    let resolution = Hash::from([9; 32]);
    let expanded = store
        .expand_stored_writer_set(&id, &resolution, false)
        .unwrap();
    let oracle = expand_set(&set, &[], &resolution, &[], false).unwrap();
    assert_eq!(expanded, oracle);
    assert!(expanded.contains(&key_author(ALICE)));
    assert!(!expanded.contains(&key_author(BOB)));
    assert!(!expanded.contains(&key_author(CAROL)));
    assert!(!expanded.is_public());

    // Retro expansion: only Alice's entry is marked include-history, and
    // she is admitted either way; Bob stays excluded.
    let retro = store
        .expand_stored_writer_set(&id, &resolution, true)
        .unwrap();
    assert!(retro.contains(&key_author(ALICE)));
    assert!(!retro.contains(&key_author(BOB)));
}

#[test]
fn test_writer_set_without_body_is_not_a_stored_object() {
    let (_dir, mut store) = store();
    let key = SigningKey::from_bytes(&[8; 32]);
    let owner = author_of(&key);
    let set = metrics_set(owner, 0);
    let body = encode_writer_body(&set).unwrap();
    let event = sign(&key, 0, None, Some(&body), ext(WRITER_KIND)).unwrap();

    // Ingested through the header-only path, the event is stored and
    // applied, but no body was retained, so there is no object to read.
    let result = store.ingest(&event.bytes, 0).unwrap();
    assert!(matches!(result, Ingest::Accepted { .. }));
    assert_eq!(store.stored_writer_set(&event.id).unwrap(), None);
    assert!(matches!(
        store.expand_stored_writer_set(&event.id, &Hash::from([9; 32]), false),
        Err(WriterSetStoreError::NoSuchSet(_))
    ));
}

#[test]
fn test_discovery_finds_sets_by_scope_with_their_expansions() {
    let (_dir, mut store) = store();
    let key = SigningKey::from_bytes(&[7; 32]);
    let owner = author_of(&key);

    let metrics = metrics_set(owner, 0);
    let metrics_id = publish(&mut store, &key, &metrics, None);

    // A second version of an unrelated set on the same chain.
    let other = WriterSet {
        set_id: WriterSet::derive_set_id(&owner, "other"),
        version: 1,
        owner,
        scope: Some("other".to_owned()),
        entries: vec![Entry::Public {
            include_history: false,
        }],
        excludes: Excludes(vec![]),
    };
    let body = encode_writer_body(&other).unwrap();
    let other_event = sign(&key, 1, Some(metrics_id), Some(&body), ext(WRITER_KIND)).unwrap();
    let result = store
        .ingest_with_body(&other_event.bytes, Some(&body), 0)
        .unwrap();
    assert!(matches!(result, Ingest::Accepted { .. }));

    let resolution = Hash::from([9; 32]);
    let hits = store
        .discover_writer_sets(Some("metrics"), &resolution)
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].event_id, metrics_id);
    assert_eq!(hits[0].set, metrics);
    assert!(hits[0].expanded.contains(&key_author(ALICE)));
    assert!(!hits[0].expanded.contains(&key_author(BOB)));

    // Unscoped discovery returns both sets, in event-ID order.
    let all = store.discover_writer_sets(None, &resolution).unwrap();
    assert_eq!(all.len(), 2);
    assert!(all[0].event_id.as_bytes() <= all[1].event_id.as_bytes());
    let public_hit = all
        .iter()
        .find(|hit| hit.event_id == other_event.id)
        .unwrap();
    assert!(public_hit.expanded.is_public());
    assert!(public_hit.expanded.contains(&key_author(CAROL)));

    // A scope nothing published matches nothing.
    assert!(
        store
            .discover_writer_sets(Some("nope"), &resolution)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn test_group_entry_expands_over_the_store_snapshot() {
    let (_dir, mut store) = store();
    let key = SigningKey::from_bytes(&[7; 32]);
    let owner = author_of(&key);
    let unknown_group = Hash::digest(b"no such group in this store");

    // The snapshot of a group the store does not know is empty, pinned
    // under the caller's tuple.
    let resolution = Hash::from([9; 32]);
    let snapshot = store
        .group_members_snapshot(&unknown_group, vec![], &resolution)
        .unwrap();
    assert_eq!(snapshot.group, unknown_group);
    assert_eq!(snapshot.resolution, resolution);
    assert!(snapshot.members.is_empty());

    // A set whose only entry is that group admits nobody through the store
    // path — the snapshot exists, so the projection runs and finds no
    // members, instead of failing for a missing snapshot.
    let set = WriterSet {
        set_id: WriterSet::derive_set_id(&owner, "grouped"),
        version: 0,
        owner,
        scope: Some("grouped".to_owned()),
        entries: vec![Entry::Group {
            id: unknown_group,
            role_min: id::groups::Role::Read,
            include_history: false,
        }],
        excludes: Excludes(vec![]),
    };
    let id = publish(&mut store, &key, &set, None);
    let expanded = store
        .expand_stored_writer_set(&id, &resolution, false)
        .unwrap();
    assert!(!expanded.contains(&key_author(ALICE)));
    assert!(!expanded.is_public());
}

#[test]
fn test_stored_rescind_reads_back() {
    let (_dir, mut store) = store();
    let key = SigningKey::from_bytes(&[7; 32]);
    let owner = author_of(&key);
    let set = metrics_set(owner, 0);
    let set_id = publish(&mut store, &key, &set, None);

    let rescind = Rescind {
        set_id: set.set_id,
        from_version: 0,
        target: Entry::Key {
            author: key_author(ALICE),
            include_history: false,
        },
        through: None,
    };
    let body = encode_rescind_body(&rescind).unwrap();
    let event = sign(&key, 1, Some(set_id), Some(&body), ext(RESCIND_KIND)).unwrap();
    let result = store
        .ingest_with_body(&event.bytes, Some(&body), 0)
        .unwrap();
    assert!(matches!(result, Ingest::Accepted { .. }));

    assert_eq!(store.stored_rescind(&event.id).unwrap(), Some(rescind));
    // Kinds do not cross: the rescind is not a set, the set is not a rescind.
    assert_eq!(store.stored_writer_set(&event.id).unwrap(), None);
    assert_eq!(store.stored_rescind(&set_id).unwrap(), None);
}
