//! Model test for the per-scope location index harvested from
//! complete signed sidecars (slice A9115-complete-harvest, issue
//! #10255; design A-9115.2, S3.26–S3.38).
//!
//! Oracle: **model** — expected `Rebuilt index equals fold of
//! validated signed sidecars and tombstones at scope epoch.` The
//! reference model below is an independent fold over plain maps:
//! it tracks, per (content, locator), the state the design's rules
//! produce — assertions harvest fresh (or unverified for
//! `stored_unverified`), absence in a later complete snapshot
//! changes nothing, a delete tombstone wins over every prior
//! state, a stale tombstone never resurrects a deleted location,
//! and the last event in fold order decides everything else.
//! Generated operations run against the model and against the
//! module, and every resulting state and error class is compared
//! after every event; at the end, a rebuild from a shuffled copy
//! of the same records must equal the incremental fold exactly,
//! and the persisted form must round-trip.
//!
//! Fixed tests cover the design's named rules: all locator kinds
//! harvest (S3.30–S3.33), no silent omission (S3.29), tombstone
//! discipline (S3.38), scope/signer binding, and the ordering
//! rejections.
//!
//! Why the modules are included by path: slice 9110-02 owns the
//! crate roots, so neither `sidecar_snapshot` nor `location_index`
//! is exported from id-store yet.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/location_index.rs"]
mod location_index;
#[path = "../src/sidecar_snapshot.rs"]
mod sidecar_snapshot;

use std::collections::BTreeMap;

use ed25519_dalek::SigningKey;

use id_core::canonical_cbor::Value;
use id_core::hash_profile::ProfileDispatch;
use id_core::id_shape::{ContentId, HashProfile};
use id_core::location_ref::TypedLocator;

use location_index::{
    AccessClass, LocationIndex, LocationState, LocationTombstoneV1, TOMBSTONE_DOMAIN,
    TombstoneBodyV1, TombstoneKind,
};
use sidecar_snapshot::{SidecarBodyV1, SidecarEnvelopeV1, encode_view};

const PROFILE: HashProfile = HashProfile::RawBlake3_256;
const NAMESPACE: &str = "ns/alpha";
const TENANT: &str = "tenant/a";
const DOMAIN: &str = "id.sidecar/v1";
const SIGNER: &str = "sidecar-writer";

fn key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

fn other_key() -> SigningKey {
    SigningKey::from_bytes(&[9u8; 32])
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn content_id_value(id: &ContentId) -> Value {
    Value::Map(vec![
        (text("digest"), Value::Bytes(id.digest().to_vec())),
        (text("profile"), text(id.profile().token())),
    ])
}

fn content(seed: &str) -> ContentId {
    ContentId::of_bytes(PROFILE, seed.as_bytes()).unwrap()
}

/// One assertion as the test intends it, before encoding.
#[derive(Debug, Clone)]
struct TestAssertion {
    content: ContentId,
    locator: String,
    class: &'static str,
    observed_at: u64,
    occurrence: Option<String>,
    version: Option<String>,
    signer: String,
    namespace: String,
    tenant: String,
}

fn assertion(content: ContentId, locator: &str, class: &'static str) -> TestAssertion {
    TestAssertion {
        content,
        locator: locator.to_owned(),
        class,
        observed_at: 1_000,
        occurrence: None,
        version: None,
        signer: SIGNER.to_owned(),
        namespace: NAMESPACE.to_owned(),
        tenant: TENANT.to_owned(),
    }
}

/// Hand-encoded assertion value (independent of the module's own
/// encoder, so a shared encoding bug cannot hide).
fn assertion_value(a: &TestAssertion) -> Value {
    let mut entries: Vec<(&str, Value)> = vec![
        ("access_class", text(a.class)),
        ("content_id", content_id_value(&a.content)),
        ("locator", text(&a.locator)),
        ("namespace", text(&a.namespace)),
        ("observed_at", Value::UInt(a.observed_at)),
        ("signer", text(&a.signer)),
        ("tenant_scope", text(&a.tenant)),
    ];
    if let Some(reference) = &a.occurrence {
        entries.push(("occurrence_ref", text(reference)));
    }
    if let Some(reference) = &a.version {
        entries.push(("source_version_ref", text(reference)));
    }
    entries.sort_by(|x, y| x.0.len().cmp(&y.0.len()).then_with(|| x.0.cmp(y.0)));
    Value::Map(
        entries
            .into_iter()
            .map(|(k, v)| (text(k), v))
            .collect::<Vec<_>>(),
    )
}

fn snapshot(
    generation: u64,
    parent: Option<sidecar_snapshot::SnapshotId>,
    assertions: &[TestAssertion],
    signer_key: &SigningKey,
) -> SidecarEnvelopeV1 {
    let mut view: BTreeMap<String, Value> = BTreeMap::new();
    if !assertions.is_empty() {
        view.insert(
            "locations".to_owned(),
            Value::Array(assertions.iter().map(assertion_value).collect()),
        );
    }
    let payload = encode_view(&view).unwrap();
    let body = SidecarBodyV1::for_payload(
        content("subject-1"),
        NAMESPACE.to_owned(),
        TENANT.to_owned(),
        generation,
        parent,
        1_700_000_000_000 + generation,
        DOMAIN.to_owned(),
        SIGNER.to_owned(),
        signer_key.verifying_key().to_bytes(),
        PROFILE,
        &payload,
    )
    .unwrap();
    SidecarEnvelopeV1::sign(body, payload, signer_key).unwrap()
}

fn tombstone(
    basis: u64,
    sequence: u64,
    kind: TombstoneKind,
    target: &ContentId,
    locator: &str,
    signer_key: &SigningKey,
) -> LocationTombstoneV1 {
    let body = TombstoneBodyV1 {
        basis_generation: basis,
        content_id: *target,
        domain_context: TOMBSTONE_DOMAIN.to_owned(),
        kind,
        locator: TypedLocator::parse_tagged(locator).unwrap(),
        namespace_id: NAMESPACE.to_owned(),
        observed_at: 2_000 + sequence,
        sequence,
        signer_id: SIGNER.to_owned(),
        key: signer_key.verifying_key().to_bytes(),
        suite: "ed25519".to_owned(),
        tenant_scope: TENANT.to_owned(),
    };
    LocationTombstoneV1::sign(body, signer_key).unwrap()
}

fn empty_index() -> LocationIndex {
    LocationIndex::new(NAMESPACE, TENANT).unwrap()
}

const R2_A: &str = "r2/acct/bucket/key-1";
const R2_B: &str = "r2/acct/bucket/key-2";
const LAPTOP: &str = "laptop/host-1/home/user/file.txt";
const DRIVE: &str = "drive/acct/file-9/https://drive.example/9";

#[test]
fn genesis_harvests_every_locator_kind_and_class() {
    let mut index = empty_index();
    let a = content("content-a");
    let assertions = vec![
        assertion(a, R2_A, "public"),
        assertion(a, LAPTOP, "scope"),
        assertion(a, DRIVE, "scope"),
        assertion(a, R2_B, "stored_unverified"),
    ];
    let env = snapshot(0, None, &assertions, &key());
    let outcome = index
        .apply_snapshot(&env, &ProfileDispatch::default())
        .unwrap();
    assert_eq!(outcome.asserted, 4);
    assert_eq!(outcome.fresh, 3);
    assert_eq!(outcome.unverified, 1);
    assert_eq!(outcome.withheld, 0);
    assert_eq!(index.len(), 4);

    let locations = index.locations_for(&a);
    assert_eq!(locations.len(), 4);
    let by_locator: BTreeMap<String, LocationState> = locations
        .iter()
        .map(|e| (e.locator.to_tagged_string(), e.state))
        .collect();
    assert_eq!(by_locator[R2_A], LocationState::Fresh);
    assert_eq!(by_locator[LAPTOP], LocationState::Fresh);
    assert_eq!(by_locator[DRIVE], LocationState::Fresh);
    assert_eq!(by_locator[R2_B], LocationState::Unverified);

    let cursor = index.cursor();
    assert_eq!(cursor.head_generation, Some(0));
    assert_eq!(
        cursor.head_snapshot_id.unwrap().content_id().digest(),
        env.snapshot_id().unwrap().content_id().digest()
    );
}

#[test]
fn absence_in_a_later_snapshot_erases_nothing() {
    let mut index = empty_index();
    let a = content("content-a");
    let env0 = snapshot(0, None, &[assertion(a, R2_A, "scope")], &key());
    index
        .apply_snapshot(&env0, &ProfileDispatch::default())
        .unwrap();
    // Generation 1 asserts a different location only; the first
    // location is absent from the complete view but has no
    // tombstone, so it stays fresh (S3.38).
    let env1 = snapshot(
        1,
        Some(env0.snapshot_id().unwrap()),
        &[assertion(a, LAPTOP, "scope")],
        &key(),
    );
    index
        .apply_snapshot(&env1, &ProfileDispatch::default())
        .unwrap();
    assert_eq!(index.len(), 2);
    let locator = TypedLocator::parse_tagged(R2_A).unwrap();
    assert_eq!(
        index.entry(&a, &locator).unwrap().state,
        LocationState::Fresh
    );
}

#[test]
fn tombstone_discipline_delete_wins_and_reassertion_revives() {
    let mut index = empty_index();
    let a = content("content-a");
    let env0 = snapshot(0, None, &[assertion(a, R2_A, "scope")], &key());
    index
        .apply_snapshot(&env0, &ProfileDispatch::default())
        .unwrap();
    let locator = TypedLocator::parse_tagged(R2_A).unwrap();

    index
        .apply_tombstone(&tombstone(0, 0, TombstoneKind::Stale, &a, R2_A, &key()))
        .unwrap();
    assert_eq!(
        index.entry(&a, &locator).unwrap().state,
        LocationState::Stale
    );

    index
        .apply_tombstone(&tombstone(0, 1, TombstoneKind::Delete, &a, R2_A, &key()))
        .unwrap();
    assert_eq!(
        index.entry(&a, &locator).unwrap().state,
        LocationState::Deleted
    );

    // A stale tombstone after the delete is absorbed, not a
    // resurrection.
    let env1 = snapshot(1, Some(env0.snapshot_id().unwrap()), &[], &key());
    index
        .apply_snapshot(&env1, &ProfileDispatch::default())
        .unwrap();
    index
        .apply_tombstone(&tombstone(1, 0, TombstoneKind::Stale, &a, R2_A, &key()))
        .unwrap();
    assert_eq!(
        index.entry(&a, &locator).unwrap().state,
        LocationState::Deleted
    );

    // A fresh assertion in a later generation revives the
    // location.
    let env2 = snapshot(
        2,
        Some(env1.snapshot_id().unwrap()),
        &[assertion(a, R2_A, "scope")],
        &key(),
    );
    index
        .apply_snapshot(&env2, &ProfileDispatch::default())
        .unwrap();
    let entry = index.entry(&a, &locator).unwrap();
    assert_eq!(entry.state, LocationState::Fresh);
    assert_eq!(entry.tombstoned_by, None);
    assert_eq!(entry.asserted_generation, Some(2));
}

#[test]
fn tombstone_before_any_assertion_creates_the_entry() {
    let mut index = empty_index();
    let a = content("content-a");
    let env0 = snapshot(0, None, &[], &key());
    index
        .apply_snapshot(&env0, &ProfileDispatch::default())
        .unwrap();
    index
        .apply_tombstone(&tombstone(0, 0, TombstoneKind::Delete, &a, DRIVE, &key()))
        .unwrap();
    let locator = TypedLocator::parse_tagged(DRIVE).unwrap();
    let entry = index.entry(&a, &locator).unwrap();
    assert_eq!(entry.state, LocationState::Deleted);
    assert_eq!(entry.access_class, None);
    assert!(entry.tombstoned_by.is_some());
}

#[test]
fn fold_rejections_carry_their_classes() {
    let dispatch = ProfileDispatch::default();
    let a = content("content-a");

    // Non-genesis first snapshot.
    let mut index = empty_index();
    let env = snapshot(1, None, &[], &key());
    let err = index.apply_snapshot(&env, &dispatch).unwrap_err();
    assert_eq!(err.class(), "generation_mismatch");

    // Wrong parent.
    let mut index = empty_index();
    let env0 = snapshot(0, None, &[], &key());
    index.apply_snapshot(&env0, &dispatch).unwrap();
    let wrong_parent = snapshot(1, Some(env.snapshot_id().unwrap()), &[], &key());
    let err = index.apply_snapshot(&wrong_parent, &dispatch).unwrap_err();
    assert_eq!(err.class(), "parent_mismatch");

    // Assertion signer is not the body signer: folds nothing.
    let mut index = empty_index();
    let mut bad = assertion(a, R2_A, "scope");
    bad.signer = "mallory".to_owned();
    let env = snapshot(0, None, &[bad], &key());
    let err = index.apply_snapshot(&env, &dispatch).unwrap_err();
    assert_eq!(err.class(), "assertion_signer_mismatch");
    assert!(index.is_empty());
    assert_eq!(index.cursor().head_generation, None);

    // Assertion scope is not the body scope.
    let mut index = empty_index();
    let mut bad = assertion(a, R2_A, "scope");
    bad.namespace = "ns/other".to_owned();
    let env = snapshot(0, None, &[bad], &key());
    let err = index.apply_snapshot(&env, &dispatch).unwrap_err();
    assert_eq!(err.class(), "scope_mismatch");

    // Tampered snapshot signature: the sidecar's own rejection.
    let mut index = empty_index();
    let mut env = snapshot(0, None, &[], &key());
    env.signature[0] ^= 1;
    let err = index.apply_snapshot(&env, &dispatch).unwrap_err();
    assert_eq!(err.class(), "sidecar");

    // Tombstone ahead of the folded head.
    let mut index = empty_index();
    let env0 = snapshot(0, None, &[], &key());
    index.apply_snapshot(&env0, &dispatch).unwrap();
    let err = index
        .apply_tombstone(&tombstone(3, 0, TombstoneKind::Delete, &a, R2_A, &key()))
        .unwrap_err();
    assert_eq!(err.class(), "basis_ahead");

    // Duplicate tombstone folds out of order.
    let t = tombstone(0, 0, TombstoneKind::Stale, &a, R2_A, &key());
    index.apply_tombstone(&t).unwrap();
    let err = index.apply_tombstone(&t).unwrap_err();
    assert_eq!(err.class(), "out_of_order");

    // Tampered tombstone signature.
    let mut t = tombstone(0, 1, TombstoneKind::Stale, &a, R2_A, &key());
    t.signature[0] ^= 1;
    let err = index.apply_tombstone(&t).unwrap_err();
    assert_eq!(err.class(), "bad_signature");

    // A tombstone signed by another key than its body commits is
    // a key mismatch at signing time.
    let body = TombstoneBodyV1 {
        basis_generation: 0,
        content_id: a,
        domain_context: TOMBSTONE_DOMAIN.to_owned(),
        kind: TombstoneKind::Stale,
        locator: TypedLocator::parse_tagged(R2_A).unwrap(),
        namespace_id: NAMESPACE.to_owned(),
        observed_at: 1,
        sequence: 0,
        signer_id: SIGNER.to_owned(),
        key: key().verifying_key().to_bytes(),
        suite: "ed25519".to_owned(),
        tenant_scope: TENANT.to_owned(),
    };
    let err = LocationTombstoneV1::sign(body, &other_key()).unwrap_err();
    assert_eq!(err.class(), "key_mismatch");
}

/// The reference model: an independent fold over plain maps.
#[derive(Debug, Default)]
struct Model {
    entries: BTreeMap<(String, String), ModelEntry>,
    head_generation: Option<u64>,
    last_order: Option<(u64, u8, u64)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ModelEntry {
    state: LocationState,
    occurrence: Option<String>,
    version: Option<String>,
    observed_at: u64,
    class: Option<AccessClass>,
    signer: String,
    asserted_generation: Option<u64>,
    asserted_digest: Option<[u8; 32]>,
    tombstone_digest: Option<[u8; 32]>,
}

impl Model {
    fn apply_snapshot(
        &mut self,
        generation: u64,
        snapshot_digest: [u8; 32],
        assertions: &[TestAssertion],
    ) {
        for a in assertions {
            let class = AccessClass::from_token(a.class).unwrap();
            let state = if a.class == "stored_unverified" {
                LocationState::Unverified
            } else {
                LocationState::Fresh
            };
            self.entries.insert(
                (content_key(&a.content), a.locator.clone()),
                ModelEntry {
                    state,
                    occurrence: a.occurrence.clone(),
                    version: a.version.clone(),
                    observed_at: a.observed_at,
                    class: Some(class),
                    signer: a.signer.clone(),
                    asserted_generation: Some(generation),
                    asserted_digest: Some(snapshot_digest),
                    tombstone_digest: None,
                },
            );
        }
        self.head_generation = Some(generation);
        self.last_order = Some((generation, 0, 0));
    }

    fn apply_tombstone(
        &mut self,
        basis: u64,
        sequence: u64,
        kind: TombstoneKind,
        target: &ContentId,
        locator: &str,
        tombstone_digest: [u8; 32],
        observed_at: u64,
    ) {
        let key = (content_key(target), locator.to_owned());
        match self.entries.get_mut(&key) {
            Some(entry) => {
                if !(entry.state == LocationState::Deleted && kind == TombstoneKind::Stale) {
                    entry.state = match kind {
                        TombstoneKind::Stale => LocationState::Stale,
                        TombstoneKind::Delete => LocationState::Deleted,
                    };
                }
                entry.observed_at = observed_at;
                entry.signer = SIGNER.to_owned();
                entry.tombstone_digest = Some(tombstone_digest);
            }
            None => {
                self.entries.insert(
                    key,
                    ModelEntry {
                        state: match kind {
                            TombstoneKind::Stale => LocationState::Stale,
                            TombstoneKind::Delete => LocationState::Deleted,
                        },
                        occurrence: None,
                        version: None,
                        observed_at,
                        class: None,
                        signer: SIGNER.to_owned(),
                        asserted_generation: None,
                        asserted_digest: None,
                        tombstone_digest: Some(tombstone_digest),
                    },
                );
            }
        }
        self.last_order = Some((basis, 1, sequence));
    }
}

fn content_key(id: &ContentId) -> String {
    format!("{}/{}", id.profile().token(), hex::encode(id.digest()))
}

fn project(index: &LocationIndex) -> BTreeMap<(String, String), ModelEntry> {
    index
        .entries()
        .map(|e| {
            (
                (content_key(&e.content_id), e.locator.to_tagged_string()),
                ModelEntry {
                    state: e.state,
                    occurrence: e.occurrence_ref.clone(),
                    version: e.source_version_ref.clone(),
                    observed_at: e.observed_at,
                    class: e.access_class,
                    signer: e.signer.clone(),
                    asserted_generation: e.asserted_generation,
                    asserted_digest: e.asserted_by.map(|id| *id.content_id().digest()),
                    tombstone_digest: e.tombstoned_by.map(|id| *id.content_id().digest()),
                },
            )
        })
        .collect()
}

/// A tiny deterministic RNG (xorshift64*) so the generated stream
/// is identical on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[test]
fn generated_operations_match_the_model() {
    let dispatch = ProfileDispatch::default();
    let contents = [
        content("content-a"),
        content("content-b"),
        content("content-c"),
    ];
    let locators = [R2_A, R2_B, LAPTOP, DRIVE];
    let classes = ["public", "scope", "stored_unverified"];

    for seed in [1u64, 42, 0x5eed] {
        let mut rng = Rng(seed);
        let mut index = empty_index();
        let mut model = Model::default();
        let mut snapshots: Vec<SidecarEnvelopeV1> = Vec::new();
        let mut tombstones: Vec<LocationTombstoneV1> = Vec::new();
        let mut generation: u64 = 0;
        let mut parent = None;
        let mut tomb_sequence: u64 = 0;

        for step in 0..140 {
            let roll = rng.below(10);
            if roll < 6 || model.head_generation.is_none() {
                // Snapshot with a random assertion set.
                let count = rng.below(5);
                let mut assertions = Vec::new();
                for _ in 0..count {
                    let mut a = assertion(
                        contents[rng.below(3) as usize],
                        locators[rng.below(4) as usize],
                        classes[rng.below(3) as usize],
                    );
                    a.observed_at = 1_000 + rng.below(50);
                    if rng.below(2) == 0 {
                        a.occurrence = Some(format!("occ-{}", rng.below(7)));
                    }
                    if rng.below(2) == 0 {
                        a.version = Some(format!("ver-{}", rng.below(7)));
                    }
                    assertions.push(a);
                }
                let env = snapshot(generation, parent, &assertions, &key());
                let digest = *env.snapshot_id().unwrap().content_id().digest();
                index.apply_snapshot(&env, &dispatch).unwrap();
                model.apply_snapshot(generation, digest, &assertions);
                parent = Some(env.snapshot_id().unwrap());
                snapshots.push(env);
                generation += 1;
                tomb_sequence = 0;
            } else {
                // Tombstone against the current head.
                let basis = generation - 1;
                let target = contents[rng.below(3) as usize];
                let locator = locators[rng.below(4) as usize];
                let kind = if rng.below(2) == 0 {
                    TombstoneKind::Stale
                } else {
                    TombstoneKind::Delete
                };
                let t = tombstone(basis, tomb_sequence, kind, &target, locator, &key());
                let digest = *t.tombstone_id().unwrap().content_id().digest();
                let observed_at = t.body.observed_at;
                index.apply_tombstone(&t).unwrap();
                model.apply_tombstone(
                    basis,
                    tomb_sequence,
                    kind,
                    &target,
                    locator,
                    digest,
                    observed_at,
                );
                tombstones.push(t);
                tomb_sequence += 1;
            }
            assert_eq!(
                project(&index),
                model.entries,
                "state diverged after step {step} (seed {seed})"
            );
            let cursor = index.cursor();
            assert_eq!(cursor.head_generation, model.head_generation);
        }

        // Rebuild from a shuffled copy of the same records equals
        // the incremental fold at the same scope epoch — the
        // issue's oracle sentence.
        let mut shuffled_snapshots = snapshots.clone();
        let mut shuffled_tombstones = tombstones.clone();
        for i in (1..shuffled_snapshots.len()).rev() {
            let j = (rng.below(i as u64 + 1)) as usize;
            shuffled_snapshots.swap(i, j);
        }
        for i in (1..shuffled_tombstones.len()).rev() {
            let j = (rng.below(i as u64 + 1)) as usize;
            shuffled_tombstones.swap(i, j);
        }
        let rebuilt = LocationIndex::rebuild(
            NAMESPACE,
            TENANT,
            &shuffled_snapshots,
            &shuffled_tombstones,
            &dispatch,
        )
        .unwrap();
        assert_eq!(rebuilt, index, "rebuild diverged (seed {seed})");
        assert_eq!(rebuilt.cursor(), index.cursor());

        // The persisted form round-trips byte-exactly and decodes
        // to the same projection.
        let bytes = index.encode().unwrap();
        let decoded = LocationIndex::decode(&bytes).unwrap();
        assert_eq!(decoded, index);
        assert_eq!(decoded.encode().unwrap(), bytes);
    }
}

#[test]
fn assertion_and_tombstone_values_round_trip() {
    let a = content("content-a");
    let assertion = assertion(a, DRIVE, "stored_unverified");
    let value = assertion_value(&assertion);
    let parsed = location_index::LocationAssertion::from_value(&value).unwrap();
    assert_eq!(parsed.content_id, a);
    assert_eq!(parsed.locator.to_tagged_string(), DRIVE);
    assert_eq!(parsed.access_class, AccessClass::StoredUnverified);
    assert_eq!(parsed.to_value().unwrap(), value);

    let t = tombstone(2, 5, TombstoneKind::Delete, &a, R2_B, &key());
    let bytes = t.canonical_bytes().unwrap();
    let decoded = LocationTombstoneV1::decode(&bytes).unwrap();
    assert_eq!(decoded, t);
    decoded.verify().unwrap();
    assert_eq!(
        decoded.tombstone_id().unwrap().content_id().digest(),
        t.tombstone_id().unwrap().content_id().digest()
    );
}
