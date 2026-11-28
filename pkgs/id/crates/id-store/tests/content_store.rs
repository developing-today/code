//! Model test for the immutable content core store (slice
//! A9120-content-store, issue #10244; design A-9120.1, S4.19–S4.22).
//!
//! Oracle: **model** — a store-state model in which an object exists if
//! and only if an ingest delivered exactly the declared length and the
//! declared digest; no partial or mismatched stream is ever accepted.
//! Generated operation sequences run against the model and against the
//! store, over both an in-memory backend and the real Iroh blob store,
//! and every resulting state and error class is compared. The model
//! decides with Iroh's own `Hash::new` and plain length comparison; it
//! calls nothing in the module under test.
//!
//! Fixed tests cover the notes on the slice: a truncated stream does
//! not commit, a digest mismatch does not commit, and the Iroh store
//! adapter reads blobs stored the old way under their old IDs.
//!
//! Why the modules are included by path: slice 9110-02 owns the crate
//! roots, so `content_store` (id-store) and `content_id` (id-core) are
//! not exported yet. `content_store` names its dependency as
//! `super::content_id`, so `content_id` is included at this crate root
//! under that name, and its own `crate::id_shape` resolves to the
//! exported `id_core::id_shape` through the re-export below.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// Test backends implement an async trait over synchronous maps.
#![allow(clippy::unused_async_trait_impl)]

mod id_shape {
    pub use id_core::id_shape::*;
}

#[path = "../../id-core/src/content_id.rs"]
#[allow(dead_code)]
mod content_id;

#[path = "../src/content_store.rs"]
mod content_store;

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use content_id::{ContentTextForm, from_iroh_hash, parse_content_text, raw_content_id};
use content_store::{BlobBackend, ContentStore, ContentStoreConfig, ContentStoreError};
use id_core::id_shape::{ContentId, HashProfile, ParsePolicy};
use iroh_blobs::Hash;
use iroh_blobs::store::mem::MemStore;
use proptest::prelude::*;

// ---------------------------------------------------------------- backends

/// In-memory blob backend keyed the way Iroh keys blobs.
#[derive(Debug, Default)]
struct MapBackend {
    blobs: Mutex<BTreeMap<[u8; 32], Vec<u8>>>,
    commits: AtomicUsize,
}

impl MapBackend {
    fn commits(&self) -> usize {
        self.commits.load(Ordering::SeqCst)
    }

    fn len(&self) -> usize {
        self.blobs.lock().unwrap().len()
    }
}

impl BlobBackend for MapBackend {
    async fn has(&self, hash: Hash) -> Result<bool, anyhow::Error> {
        Ok(self.blobs.lock().unwrap().contains_key(hash.as_bytes()))
    }

    async fn read(&self, hash: Hash) -> Result<Option<Vec<u8>>, anyhow::Error> {
        Ok(self.blobs.lock().unwrap().get(hash.as_bytes()).cloned())
    }

    async fn commit(&self, bytes: Vec<u8>) -> Result<Hash, anyhow::Error> {
        self.commits.fetch_add(1, Ordering::SeqCst);
        let hash = Hash::new(&bytes);
        self.blobs.lock().unwrap().insert(*hash.as_bytes(), bytes);
        Ok(hash)
    }
}

/// A backend that returns other bytes than it was given.
#[derive(Debug, Default)]
struct CorruptBackend(MapBackend);

impl BlobBackend for CorruptBackend {
    async fn has(&self, hash: Hash) -> Result<bool, anyhow::Error> {
        self.0.has(hash).await
    }

    async fn read(&self, hash: Hash) -> Result<Option<Vec<u8>>, anyhow::Error> {
        Ok(self.0.read(hash).await?.map(|mut bytes| {
            bytes.push(0);
            bytes
        }))
    }

    async fn commit(&self, bytes: Vec<u8>) -> Result<Hash, anyhow::Error> {
        self.0.commit(bytes).await
    }
}

/// A backend that reports the wrong hash for what it stored.
#[derive(Debug, Default)]
struct MislabelingBackend;

impl BlobBackend for MislabelingBackend {
    async fn has(&self, _hash: Hash) -> Result<bool, anyhow::Error> {
        Ok(false)
    }

    async fn read(&self, _hash: Hash) -> Result<Option<Vec<u8>>, anyhow::Error> {
        Ok(None)
    }

    async fn commit(&self, _bytes: Vec<u8>) -> Result<Hash, anyhow::Error> {
        Ok(Hash::EMPTY)
    }
}

// ---------------------------------------------------------------- helpers

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn map_store() -> ContentStore<MapBackend> {
    ContentStore::new(MapBackend::default(), ContentStoreConfig::default())
}

fn id_of(bytes: &[u8]) -> ContentId {
    from_iroh_hash(&Hash::new(bytes))
}

/// A reader that counts how many times it is read.
struct CountingReader<'a> {
    inner: &'a [u8],
    reads: &'a AtomicUsize,
}

impl Read for CountingReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.inner.read(buf)
    }
}

/// A reader that yields some bytes and then fails.
struct FailingReader<'a>(&'a [u8]);

impl Read for FailingReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.0.is_empty() {
            return Err(io::Error::other("connection reset"));
        }
        self.0.read(buf)
    }
}

// ---------------------------------------------------------------- fixed tests

#[test]
fn exact_stream_commits_and_reads_back() {
    runtime().block_on(async {
        let store = map_store();
        assert_eq!(store.config(), ContentStoreConfig::default());
        let bytes = b"immutable bytes".to_vec();
        let id = id_of(&bytes);
        assert_eq!(id, raw_content_id(&bytes));
        assert!(!store.contains(&id).await.unwrap());
        let ingested = store
            .ingest(&id, bytes.len() as u64, bytes.as_slice())
            .await
            .unwrap();
        assert_eq!(ingested.id, id);
        assert_eq!(ingested.bytes, bytes.len() as u64);
        assert!(ingested.newly_stored);
        assert!(store.contains(&id).await.unwrap());
        assert_eq!(store.get(&id).await.unwrap(), Some(bytes.clone()));
        // A second ingest still verifies, and stores nothing new.
        let again = store
            .ingest(&id, bytes.len() as u64, bytes.as_slice())
            .await
            .unwrap();
        assert!(!again.newly_stored);
        assert_eq!(store.backend().commits(), 1);
    });
}

#[test]
fn truncated_stream_does_not_commit() {
    runtime().block_on(async {
        let store = map_store();
        let bytes = vec![7u8; 200_000];
        let id = id_of(&bytes);
        for cut in [0, 1, 65_536, 199_999] {
            let error = store
                .ingest(&id, bytes.len() as u64, &bytes[..cut])
                .await
                .unwrap_err();
            assert_eq!(error.class(), "truncated", "cut {cut}");
            assert!(matches!(
                error,
                ContentStoreError::Truncated { declared: 200_000, received } if received == cut as u64
            ));
        }
        assert_eq!(store.backend().commits(), 0);
        assert!(!store.contains(&id).await.unwrap());
    });
}

#[test]
fn overlong_stream_does_not_commit_and_reads_one_byte_past() {
    runtime().block_on(async {
        let store = map_store();
        let bytes = b"declared".to_vec();
        let id = id_of(&bytes);
        let mut longer = bytes.clone();
        longer.extend_from_slice(&vec![0u8; 1_000_000]);
        let mut reader = longer.as_slice();
        let error = store
            .ingest(&id, bytes.len() as u64, &mut reader)
            .await
            .unwrap_err();
        assert_eq!(error.class(), "overlong");
        // Only the declared length plus one byte was consumed.
        assert_eq!(reader.len(), longer.len() - bytes.len() - 1);
        assert_eq!(store.backend().commits(), 0);
    });
}

#[test]
fn digest_mismatch_does_not_commit() {
    runtime().block_on(async {
        let store = map_store();
        let declared = id_of(b"what was promised");
        let error = store
            .ingest(&declared, 17, &b"what was delivered"[..17])
            .await
            .unwrap_err();
        assert_eq!(error.class(), "digest_mismatch");
        assert_eq!(store.backend().commits(), 0);
        assert!(!store.contains(&declared).await.unwrap());
        assert!(
            !store
                .contains(&id_of(&b"what was delivered"[..17]))
                .await
                .unwrap()
        );
    });
}

#[test]
fn wrong_stream_for_a_stored_object_is_still_rejected() {
    runtime().block_on(async {
        let store = map_store();
        let bytes = b"stored".to_vec();
        let id = id_of(&bytes);
        store.ingest(&id, 6, bytes.as_slice()).await.unwrap();
        let error = store.ingest(&id, 6, &b"forged"[..]).await.unwrap_err();
        assert_eq!(error.class(), "digest_mismatch");
        assert_eq!(store.get(&id).await.unwrap(), Some(bytes));
    });
}

#[test]
fn read_failure_does_not_commit() {
    runtime().block_on(async {
        let store = map_store();
        let bytes = b"half of it".to_vec();
        let error = store
            .ingest(&id_of(&bytes), 10, FailingReader(&bytes[..4]))
            .await
            .unwrap_err();
        assert_eq!(error.class(), "io");
        assert_eq!(store.backend().commits(), 0);
    });
}

#[test]
fn other_profile_is_unstorable_before_any_read() {
    runtime().block_on(async {
        let store = map_store();
        let atlas = ContentId::of_bytes(HashProfile::AtlasChunkSha256Utf8, b"abc").unwrap();
        let reads = AtomicUsize::new(0);
        let error = store
            .ingest(
                &atlas,
                3,
                CountingReader {
                    inner: b"abc",
                    reads: &reads,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.class(), "profile_mismatch");
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        assert_eq!(
            store.contains(&atlas).await.unwrap_err().class(),
            "profile_mismatch"
        );
        assert_eq!(
            store.get(&atlas).await.unwrap_err().class(),
            "profile_mismatch"
        );
    });
}

#[test]
fn policy_without_raw_profile_rejects_before_any_read() {
    runtime().block_on(async {
        let store = ContentStore::new(
            MapBackend::default(),
            ContentStoreConfig {
                policy: ParsePolicy::only(HashProfile::AtlasChunkSha256Utf8),
                ..ContentStoreConfig::default()
            },
        );
        let id = id_of(b"abc");
        let reads = AtomicUsize::new(0);
        let error = store
            .ingest(
                &id,
                3,
                CountingReader {
                    inner: b"abc",
                    reads: &reads,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.class(), "unsupported_profile");
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        assert_eq!(
            store.contains(&id).await.unwrap_err().class(),
            "unsupported_profile"
        );
    });
}

#[test]
fn declared_length_over_limit_rejects_before_any_read() {
    runtime().block_on(async {
        let store = ContentStore::new(
            MapBackend::default(),
            ContentStoreConfig {
                max_object_bytes: 4,
                ..ContentStoreConfig::default()
            },
        );
        let reads = AtomicUsize::new(0);
        let error = store
            .ingest(
                &id_of(b"abcde"),
                5,
                CountingReader {
                    inner: b"abcde",
                    reads: &reads,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.class(), "resource_limit");
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        // At the limit is fine.
        store
            .ingest(&id_of(b"abcd"), 4, &b"abcd"[..])
            .await
            .unwrap();
    });
}

#[test]
fn corrupt_backend_bytes_are_never_returned() {
    runtime().block_on(async {
        let store = ContentStore::new(CorruptBackend::default(), ContentStoreConfig::default());
        let id = id_of(b"good");
        store.ingest(&id, 4, &b"good"[..]).await.unwrap();
        assert_eq!(store.get(&id).await.unwrap_err().class(), "corrupt");
    });
}

#[test]
fn backend_that_mislabels_a_commit_is_reported() {
    runtime().block_on(async {
        let store = ContentStore::new(MislabelingBackend, ContentStoreConfig::default());
        let error = store.ingest(&id_of(b"x"), 1, &b"x"[..]).await.unwrap_err();
        assert_eq!(error.class(), "backend_mismatch");
    });
}

#[test]
fn iroh_adapter_reads_blobs_stored_the_old_way_unchanged() {
    runtime().block_on(async {
        let iroh = MemStore::new();
        let legacy = b"stored before the content core existed".to_vec();
        // The existing write path: straight into the Iroh blob store.
        let tag = iroh.blobs().add_bytes(legacy.clone()).await.unwrap();
        let store = ContentStore::new(iroh.blobs().clone(), ContentStoreConfig::default());
        // The old hash, as a content ID and in Iroh's own text form.
        let id = from_iroh_hash(&tag.hash);
        let from_text = parse_content_text(
            &tag.hash.to_string(),
            ContentTextForm::IrohLegacy,
            ParsePolicy::ALL,
        )
        .unwrap();
        assert_eq!(from_text, id);
        assert!(store.contains(&id).await.unwrap());
        assert_eq!(store.get(&id).await.unwrap(), Some(legacy.clone()));
        // Ingesting the same bytes again finds them; nothing is rewritten.
        let again = store
            .ingest(&id, legacy.len() as u64, legacy.as_slice())
            .await
            .unwrap();
        assert!(!again.newly_stored);
        // A new object goes in through the same Iroh store and comes out
        // under the hash Iroh itself computes.
        let fresh = b"stored through the content core".to_vec();
        let fresh_id = id_of(&fresh);
        store
            .ingest(&fresh_id, fresh.len() as u64, fresh.as_slice())
            .await
            .unwrap();
        assert_eq!(
            iroh.blobs()
                .get_bytes(Hash::new(&fresh))
                .await
                .unwrap()
                .to_vec(),
            fresh
        );
        // A truncated ingest leaves no complete blob in Iroh.
        let lost = b"never complete".to_vec();
        let lost_id = id_of(&lost);
        let error = store.ingest(&lost_id, 14, &lost[..7]).await.unwrap_err();
        assert_eq!(error.class(), "truncated");
        assert!(!iroh.blobs().has(Hash::new(&lost)).await.unwrap());
    });
}

// ---------------------------------------------------------------- model

/// One generated operation.
#[derive(Debug, Clone)]
enum Op {
    /// Ingest a stream derived from `source`, declared as `declared`.
    Ingest {
        declared: usize,
        length_delta: i8,
        source: usize,
        edit: Edit,
    },
    Contains(usize),
    Get(usize),
}

/// How the stream differs from its source object.
#[derive(Debug, Clone)]
enum Edit {
    None,
    Truncate(usize),
    Append(Vec<u8>),
    Flip(usize),
}

fn apply(edit: &Edit, source: &[u8]) -> Vec<u8> {
    let mut out = source.to_vec();
    match edit {
        Edit::None => {}
        Edit::Truncate(cut) => out.truncate(cut % (source.len() + 1)),
        Edit::Append(extra) => out.extend_from_slice(extra),
        Edit::Flip(at) => {
            if !out.is_empty() {
                let at = at % out.len();
                out[at] ^= 0x01;
            }
        }
    }
    out
}

/// What the model expects an ingest to do: `Ok(newly_stored)` or an
/// error class.
fn model_ingest(
    state: &mut BTreeSet<[u8; 32]>,
    declared: &[u8],
    declared_len: u64,
    stream: &[u8],
) -> Result<bool, &'static str> {
    let received = stream.len() as u64;
    if received < declared_len {
        return Err("truncated");
    }
    if received > declared_len {
        return Err("overlong");
    }
    let digest = *Hash::new(declared).as_bytes();
    if *Hash::new(stream).as_bytes() != digest {
        return Err("digest_mismatch");
    }
    Ok(state.insert(digest))
}

fn op_strategy(objects: usize) -> impl Strategy<Value = Op> {
    let edit = prop_oneof![
        3 => Just(Edit::None),
        1 => any::<usize>().prop_map(Edit::Truncate),
        1 => proptest::collection::vec(any::<u8>(), 1..8).prop_map(Edit::Append),
        1 => any::<usize>().prop_map(Edit::Flip),
    ];
    prop_oneof![
        4 => (
            0..objects,
            prop_oneof![3 => Just(0i8), 1 => -2i8..=2],
            prop_oneof![1 => Just(None), 1 => (0..objects).prop_map(Some)],
            edit,
        )
            .prop_map(|(declared, length_delta, source, edit)| Op::Ingest {
                declared,
                length_delta,
                // Half the streams come from the declared object itself.
                source: source.unwrap_or(declared),
                edit,
            }),
        1 => (0..objects).prop_map(Op::Contains),
        1 => (0..objects).prop_map(Op::Get),
    ]
}

fn scenario() -> impl Strategy<Value = (Vec<Vec<u8>>, Vec<Op>)> {
    proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..300), 1..5).prop_flat_map(
        |objects| {
            let ops = proptest::collection::vec(op_strategy(objects.len()), 1..24);
            (Just(objects), ops)
        },
    )
}

/// Runs a scenario against the model and `store`; `initial` is what the
/// backend holds before the first operation.
async fn run_model<B: BlobBackend + Sync>(
    store: &ContentStore<B>,
    objects: &[Vec<u8>],
    ops: &[Op],
    initial: BTreeSet<[u8; 32]>,
) -> Result<(), TestCaseError> {
    let mut state = initial;
    for op in ops {
        match op {
            Op::Ingest {
                declared,
                length_delta,
                source,
                edit,
            } => {
                let declared_bytes = &objects[*declared];
                let declared_len =
                    (declared_bytes.len() as u64).saturating_add_signed(i64::from(*length_delta));
                let stream = apply(edit, &objects[*source]);
                let expected = model_ingest(&mut state, declared_bytes, declared_len, &stream);
                let actual = store
                    .ingest(&id_of(declared_bytes), declared_len, stream.as_slice())
                    .await
                    .map(|ingested| ingested.newly_stored)
                    .map_err(|error| error.class());
                prop_assert_eq!(actual, expected, "{:?}", op);
            }
            Op::Contains(index) => {
                let bytes = &objects[*index];
                prop_assert_eq!(
                    store.contains(&id_of(bytes)).await.unwrap(),
                    state.contains(Hash::new(bytes).as_bytes())
                );
            }
            Op::Get(index) => {
                let bytes = &objects[*index];
                let expected = state
                    .contains(Hash::new(bytes).as_bytes())
                    .then(|| bytes.clone());
                prop_assert_eq!(store.get(&id_of(bytes)).await.unwrap(), expected);
            }
        }
        // After every operation, every object exists exactly when the
        // model says it does.
        for bytes in objects {
            prop_assert_eq!(
                store.contains(&id_of(bytes)).await.unwrap(),
                state.contains(Hash::new(bytes).as_bytes())
            );
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn map_backend_matches_the_model((objects, ops) in scenario()) {
        let store = map_store();
        runtime().block_on(run_model(&store, &objects, &ops, BTreeSet::new()))?;
        // Only verified objects were ever written.
        prop_assert!(store.backend().len() <= objects.len());
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn iroh_backend_matches_the_model((objects, ops) in scenario()) {
        runtime().block_on(async {
            let iroh = MemStore::new();
            let store = ContentStore::new(iroh.blobs().clone(), ContentStoreConfig::default());
            // Iroh treats the empty blob as always present.
            let initial = BTreeSet::from([*Hash::EMPTY.as_bytes()]);
            run_model(&store, &objects, &ops, initial).await
        })?;
    }
}
