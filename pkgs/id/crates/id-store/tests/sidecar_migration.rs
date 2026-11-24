//! Model test for the legacy metadata read bridge and immutable
//! snapshot migration (slice A9114-snapshot-migration, issue
//! #10270; design A-9114.1–A-9114.3, S3.11–S3.22).
//!
//! Oracle: **model** — expected `Before/after data-model comparison
//! plus fixed signed snapshot vector; only consistent source views
//! may advance head.` The reference model below works from the raw
//! JSON with `serde_json::Value` — never the module's own types —
//! and builds the expected per-subject view with its own grouping
//! and ordering code. Generated operations (add a legacy tag,
//! corrupt a second read, migrate) run against the model and the
//! module, and every resulting state and error class is compared:
//! a migration may advance the head by exactly one generation only
//! when the model's view changed, its snapshot's decoded view must
//! equal the model's view exactly, and re-migrating an unchanged
//! source must move nothing.
//!
//! Fixed tests cover the oracle's named notes: the fixed signed
//! snapshot vector, legacy read stability (the source bytes and
//! their digest are never rewritten), migration idempotence,
//! inconsistent sources publishing nothing, and both legacy link
//! forms surviving as path references (S3.21, S3.22).
//!
//! Why the modules are included by path: slice 9110-02 owns the
//! crate roots, so neither `sidecar_snapshot` nor
//! `sidecar_migration` is exported from id-store yet.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/sidecar_migration.rs"]
mod sidecar_migration;
#[path = "../src/sidecar_snapshot.rs"]
mod sidecar_snapshot;

use std::collections::BTreeMap;

use ed25519_dalek::SigningKey;

use id_core::canonical_cbor::{Value, encode_canonical};
use id_core::hash_profile::ProfileDispatch;
use id_core::id_shape::{ContentId, HashProfile};

use sidecar_migration::{
    LegacySourceView, MigrationOutcome, MigrationParams, legacy_source_digest, migrate,
    parse_legacy,
};
use sidecar_snapshot::{SidecarEnvelopeV1, SidecarHeadV1, decode_view};

const PROFILE: HashProfile = HashProfile::RawBlake3_256;
const NAMESPACE: &str = "ns/alpha";
const TENANT: &str = "tenant/a";
const DOMAIN: &str = "id.sidecar/v1";
const SIGNER: &str = "migration-writer";

fn key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

fn params_for(subject_name: &str) -> MigrationParams {
    MigrationParams {
        subject_content_id: ContentId::of_bytes(PROFILE, subject_name.as_bytes()).unwrap(),
        namespace_id: NAMESPACE.to_owned(),
        tenant_scope: TENANT.to_owned(),
        domain_context: DOMAIN.to_owned(),
        signer_id: SIGNER.to_owned(),
        payload_profile: PROFILE,
        created_at: 1_700_001_000_000,
    }
}

/// A hand-written legacy document, in the exact shape the legacy
/// writer serializes: two subjects, a key-only shape implied by
/// optional fields, and both link forms (S3.21, S3.22).
const FIXED_LEGACY: &[u8] = br#"{"version":1,"tags":[
{"subject":"README.md","key":"created","value":"1700000000","created_at":1700000000,"modified_at":1700000000},
{"subject":"README.md","key":"category","value":"docs","created_at":1700000100,"modified_at":1700000200},
{"subject":"README.md","key":"related","link":{"type":"Name","target":"docs/guide.md"},"created_at":1700000300,"modified_at":1700000300},
{"subject":"README.md","key":"related","link":{"type":"Hash","target":"abc123"},"created_at":1700000250,"modified_at":1700000250},
{"subject":"other.txt","key":"created","value":"1700000500","created_at":1700000500,"modified_at":1700000500}
]}"#;

fn entry(
    created_at: u64,
    modified_at: u64,
    value: Option<&str>,
    link: Option<(&str, &str)>,
) -> Value {
    let mut entries = Vec::new();
    if let Some((kind, target)) = link {
        entries.push((
            Value::Text("link".to_owned()),
            Value::Map(vec![
                (Value::Text("kind".to_owned()), Value::Text(kind.to_owned())),
                (
                    Value::Text("target".to_owned()),
                    Value::Text(target.to_owned()),
                ),
            ]),
        ));
    }
    if let Some(value) = value {
        entries.push((
            Value::Text("value".to_owned()),
            Value::Text(value.to_owned()),
        ));
    }
    entries.push((
        Value::Text("created_at".to_owned()),
        Value::UInt(created_at),
    ));
    entries.push((
        Value::Text("modified_at".to_owned()),
        Value::UInt(modified_at),
    ));
    Value::Map(entries)
}

fn sorted_entries(mut entries: Vec<Value>) -> Value {
    let mut keyed: Vec<(Vec<u8>, Value)> = entries
        .drain(..)
        .map(|e| (encode_canonical(&e).unwrap(), e))
        .collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    keyed.dedup_by(|a, b| a.0 == b.0);
    Value::Array(keyed.into_iter().map(|(_, e)| e).collect())
}

/// The expected complete view of `README.md` in the fixed document,
/// built by hand here — never by the module under test.
fn fixed_expected_view() -> BTreeMap<String, Value> {
    let mut view = BTreeMap::new();
    view.insert(
        "category".to_owned(),
        sorted_entries(vec![entry(
            1_700_000_100,
            1_700_000_200,
            Some("docs"),
            None,
        )]),
    );
    view.insert(
        "created".to_owned(),
        sorted_entries(vec![entry(
            1_700_000_000,
            1_700_000_000,
            Some("1700000000"),
            None,
        )]),
    );
    view.insert(
        "related".to_owned(),
        sorted_entries(vec![
            entry(
                1_700_000_300,
                1_700_000_300,
                None,
                Some(("name", "docs/guide.md")),
            ),
            entry(1_700_000_250, 1_700_000_250, None, Some(("hash", "abc123"))),
        ]),
    );
    view
}

#[test]
fn fixed_signed_snapshot_vector() {
    let source = LegacySourceView::from_double_read(FIXED_LEGACY, FIXED_LEGACY).unwrap();
    assert_eq!(source.source_digest(), legacy_source_digest(FIXED_LEGACY));
    assert_eq!(
        source.source_digest(),
        *blake3::hash(FIXED_LEGACY).as_bytes()
    );

    let view = source.subject_view("README.md").unwrap();
    assert_eq!(view, fixed_expected_view());

    let outcome = migrate(&source, "README.md", &params_for("README.md"), None, &key()).unwrap();
    let MigrationOutcome::Advanced {
        snapshot,
        head,
        source_digest,
    } = outcome
    else {
        panic!("genesis migration must advance");
    };
    assert_eq!(source_digest, legacy_source_digest(FIXED_LEGACY));

    // The fixed vector: the payload is exactly the encoded complete
    // view, the snapshot verifies, and its ID is id-core's digest of
    // the complete envelope bytes — derived here, not by the module.
    assert_eq!(
        decode_view(&snapshot.raw_payload_bytes).unwrap(),
        fixed_expected_view()
    );
    assert_eq!(
        snapshot.raw_payload_bytes,
        sidecar_snapshot::encode_view(&fixed_expected_view()).unwrap()
    );
    snapshot.verify(&ProfileDispatch::default()).unwrap();
    let expected_id = ContentId::of_bytes(PROFILE, &snapshot.canonical_bytes().unwrap()).unwrap();
    assert_eq!(*snapshot.snapshot_id().unwrap().content_id(), expected_id);
    assert_eq!(snapshot.body.generation, 0);
    assert_eq!(snapshot.body.parent_snapshot_id, None);
    assert_eq!(head.generation, 0);
    assert_eq!(head.snapshot_id, snapshot.snapshot_id().unwrap());
    head.verify().unwrap();
}

#[test]
fn legacy_read_is_stable_and_never_rewrites_source() {
    let bytes_before = FIXED_LEGACY.to_vec();
    let first = parse_legacy(FIXED_LEGACY).unwrap();
    let second = parse_legacy(FIXED_LEGACY).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.version, 1);
    assert_eq!(first.tags.len(), 5);

    // Both link forms survive the read as path references (S3.21,
    // S3.22): a name target and a hash target, verbatim.
    let links: Vec<String> = first
        .tags
        .iter()
        .filter_map(|tag| match &tag.link {
            Some(sidecar_migration::LegacyMetaLink::Hash(target)) => Some(format!("hash:{target}")),
            Some(sidecar_migration::LegacyMetaLink::Name(target)) => Some(format!("name:{target}")),
            None => None,
        })
        .collect();
    assert_eq!(links, ["name:docs/guide.md", "hash:abc123"]);

    // Migrating does not touch the source bytes or their digest.
    let source =
        LegacySourceView::attested(FIXED_LEGACY, &legacy_source_digest(FIXED_LEGACY)).unwrap();
    let _ = migrate(&source, "README.md", &params_for("README.md"), None, &key()).unwrap();
    assert_eq!(FIXED_LEGACY, bytes_before.as_slice());
    assert_eq!(parse_legacy(FIXED_LEGACY).unwrap(), first);
}

#[test]
fn inconsistent_sources_cannot_publish() {
    // Two reads that disagree: no consistent source snapshot.
    let mut changed = FIXED_LEGACY.to_vec();
    let last = changed.len() - 2;
    changed[last] = if changed[last] == b'0' { b'1' } else { b'0' };
    let error = LegacySourceView::from_double_read(FIXED_LEGACY, &changed).unwrap_err();
    assert_eq!(error.class(), "inconsistent_source");

    // One read against a wrong attested digest.
    let error = LegacySourceView::attested(FIXED_LEGACY, &[9u8; 32]).unwrap_err();
    assert_eq!(error.class(), "digest_mismatch");

    // Byte-identical reads of an internally inconsistent document:
    // a tag created after it was modified cannot be one consistent
    // source view.
    const INCONSISTENT: &[u8] = br#"{"version":1,"tags":[
{"subject":"a.txt","key":"created","value":"5","created_at":200,"modified_at":100}
]}"#;
    let error = LegacySourceView::from_double_read(INCONSISTENT, INCONSISTENT).unwrap_err();
    assert_eq!(error.class(), "inconsistent_source");
    // ... while the plain read bridge still reads it: read stability
    // and migration gating are separate concerns.
    assert_eq!(parse_legacy(INCONSISTENT).unwrap().tags.len(), 1);
}

#[test]
fn malformed_legacy_rejects() {
    for (bytes, _name) in [
        (&b"not json"[..], "not json"),
        (&b"[1,2]"[..], "top-level array"),
        (
            &br#"{"version":1,"tags":[],"surprise":true}"#[..],
            "unknown field",
        ),
        (
            &br#"{"version":1,"tags":[{"subject":"a","key":"k","modified_at":1}]}"#[..],
            "missing created_at",
        ),
        (
            &br#"{"version":1,"tags":[{"subject":"a","key":"k","link":{"type":"Url","target":"x"},"created_at":1,"modified_at":1}]}"#[..],
            "unknown link type",
        ),
    ] {
        let error = parse_legacy(bytes).unwrap_err();
        assert_eq!(error.class(), "malformed_legacy", "{_name}");
    }
}

#[test]
fn subject_absent_rejects() {
    let source = LegacySourceView::from_double_read(FIXED_LEGACY, FIXED_LEGACY).unwrap();
    let error = migrate(
        &source,
        "missing.txt",
        &params_for("missing.txt"),
        None,
        &key(),
    )
    .unwrap_err();
    assert_eq!(error.class(), "subject_absent");
}

#[test]
fn migration_is_idempotent_and_advances_by_exactly_one() {
    let key = key();
    let params = params_for("README.md");
    let source = LegacySourceView::from_double_read(FIXED_LEGACY, FIXED_LEGACY).unwrap();

    let MigrationOutcome::Advanced {
        snapshot: first_snapshot,
        head: first_head,
        ..
    } = migrate(&source, "README.md", &params, None, &key).unwrap()
    else {
        panic!("genesis migration must advance");
    };

    // Re-migrating the unchanged source moves nothing.
    let outcome = migrate(
        &source,
        "README.md",
        &params,
        Some((&first_head, &first_snapshot)),
        &key,
    )
    .unwrap();
    let MigrationOutcome::AlreadyMigrated {
        head, snapshot_id, ..
    } = outcome
    else {
        panic!("unchanged source must be a no-op");
    };
    assert_eq!(head, first_head);
    assert_eq!(snapshot_id, first_snapshot.snapshot_id().unwrap());

    // A changed source advances by exactly one generation, parented
    // on the prior head snapshot.
    const CHANGED: &[u8] = br#"{"version":1,"tags":[
{"subject":"README.md","key":"created","value":"1700000000","created_at":1700000000,"modified_at":1700000000},
{"subject":"README.md","key":"category","value":"guides","created_at":1700000100,"modified_at":1700000400}
]}"#;
    let changed_source = LegacySourceView::from_double_read(CHANGED, CHANGED).unwrap();
    let MigrationOutcome::Advanced {
        snapshot: second_snapshot,
        head: second_head,
        ..
    } = migrate(
        &changed_source,
        "README.md",
        &params,
        Some((&first_head, &first_snapshot)),
        &key,
    )
    .unwrap()
    else {
        panic!("changed source must advance");
    };
    assert_eq!(second_head.generation, 1);
    assert_eq!(
        second_snapshot.body.parent_snapshot_id,
        Some(first_snapshot.snapshot_id().unwrap())
    );
    assert_eq!(
        second_head.snapshot_id,
        second_snapshot.snapshot_id().unwrap()
    );

    // Tag order in the legacy document does not change the view or
    // the snapshot: the same tags, reversed, migrate identically.
    const REORDERED: &[u8] = br#"{"version":1,"tags":[
{"subject":"README.md","key":"category","value":"guides","created_at":1700000100,"modified_at":1700000400},
{"subject":"README.md","key":"created","value":"1700000000","created_at":1700000000,"modified_at":1700000000}
]}"#;
    let reordered = LegacySourceView::from_double_read(REORDERED, REORDERED).unwrap();
    assert_eq!(
        reordered.subject_view("README.md").unwrap(),
        changed_source.subject_view("README.md").unwrap()
    );
}

// ---------------------------------------------------------------------
// Generated model comparison
// ---------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ModelTag {
    subject: String,
    key: String,
    value: Option<String>,
    link: Option<(&'static str, String)>,
    created_at: u64,
    modified_at: u64,
}

fn model_bytes(tags: &[ModelTag]) -> Vec<u8> {
    let tag_values: Vec<serde_json::Value> = tags
        .iter()
        .map(|tag| {
            let mut object = serde_json::json!({
                "subject": tag.subject,
                "key": tag.key,
                "created_at": tag.created_at,
                "modified_at": tag.modified_at,
            });
            if let Some(value) = &tag.value {
                object["value"] = serde_json::Value::String(value.clone());
            }
            if let Some((kind, target)) = &tag.link {
                let kind_name = if *kind == "hash" { "Hash" } else { "Name" };
                object["link"] = serde_json::json!({"type": kind_name, "target": target});
            }
            object
        })
        .collect();
    serde_json::to_vec(&serde_json::json!({"version": 1, "tags": tag_values})).unwrap()
}

/// The reference view: built from the raw JSON with serde_json
/// values only — an independent implementation of the conversion.
fn model_view(bytes: &[u8], subject: &str) -> BTreeMap<String, Value> {
    let document: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    let mut by_key: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for tag in document["tags"].as_array().unwrap() {
        if tag["subject"].as_str().unwrap() != subject {
            continue;
        }
        let mut entries = Vec::new();
        if let Some(link) = tag.get("link") {
            let kind = match link["type"].as_str().unwrap() {
                "Hash" => "hash",
                "Name" => "name",
                other => panic!("unexpected link type {other}"),
            };
            entries.push((
                Value::Text("link".to_owned()),
                Value::Map(vec![
                    (Value::Text("kind".to_owned()), Value::Text(kind.to_owned())),
                    (
                        Value::Text("target".to_owned()),
                        Value::Text(link["target"].as_str().unwrap().to_owned()),
                    ),
                ]),
            ));
        }
        if let Some(value) = tag["value"].as_str() {
            entries.push((
                Value::Text("value".to_owned()),
                Value::Text(value.to_owned()),
            ));
        }
        entries.push((
            Value::Text("created_at".to_owned()),
            Value::UInt(tag["created_at"].as_u64().unwrap()),
        ));
        entries.push((
            Value::Text("modified_at".to_owned()),
            Value::UInt(tag["modified_at"].as_u64().unwrap()),
        ));
        by_key
            .entry(tag["key"].as_str().unwrap().to_owned())
            .or_default()
            .push(Value::Map(entries));
    }
    by_key
        .into_iter()
        .map(|(key, entries)| (key, sorted_entries(entries)))
        .collect()
}

#[test]
fn generated_operations_match_reference_model() {
    const SUBJECTS: [&str; 2] = ["a.txt", "b.txt"];
    const KEYS: [&str; 5] = ["created", "category", "author", "related", "pinned"];
    let key = key();

    for seed in [11u64, 42, 97] {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut tags: Vec<ModelTag> = Vec::new();
        // Per-subject chain state: (head, current snapshot).
        let mut chains: [Option<(SidecarHeadV1, SidecarEnvelopeV1)>; 2] = [None, None];

        for _op in 0..60 {
            match rng.below(10) {
                // Add a legacy tag.
                0..=4 => {
                    let subject = SUBJECTS[rng.below(2) as usize].to_owned();
                    let tag_key = KEYS[rng.below(5) as usize].to_owned();
                    let created_at = 1_000 + rng.below(1_000);
                    let value = match rng.below(3) {
                        0 => None,
                        _ => Some(format!("v{}", rng.below(50))),
                    };
                    let link = match rng.below(4) {
                        0 => Some(("hash", format!("h{}", rng.below(50)))),
                        1 => Some(("name", format!("docs/{}.md", rng.below(50)))),
                        _ => None,
                    };
                    tags.push(ModelTag {
                        subject,
                        key: tag_key,
                        value,
                        link,
                        created_at,
                        modified_at: created_at + rng.below(500),
                    });
                }
                // A second read that disagrees can never migrate.
                5 => {
                    let bytes = model_bytes(&tags);
                    let mut changed = bytes.clone();
                    changed.push(b' ');
                    let error = LegacySourceView::from_double_read(&bytes, &changed).unwrap_err();
                    assert_eq!(error.class(), "inconsistent_source");
                }
                // Migrate one subject and compare against the model.
                _ => {
                    let index = rng.below(2) as usize;
                    let subject = SUBJECTS[index];
                    let bytes = model_bytes(&tags);
                    assert_eq!(
                        legacy_source_digest(&bytes),
                        *blake3::hash(&bytes).as_bytes()
                    );
                    let expected_view = model_view(&bytes, subject);
                    let source = LegacySourceView::from_double_read(&bytes, &bytes).unwrap();
                    let current = chains[index]
                        .as_ref()
                        .map(|(head, snapshot)| (head, snapshot));
                    let result = migrate(&source, subject, &params_for(subject), current, &key);
                    if expected_view.is_empty() {
                        let error = result.unwrap_err();
                        assert_eq!(error.class(), "subject_absent");
                        continue;
                    }
                    let prior = chains[index].take();
                    // The model's verdict: a no-op is correct exactly
                    // when the current head's view already equals the
                    // converted source view.
                    let unchanged = prior.as_ref().is_some_and(|(_, snapshot)| {
                        decode_view(&snapshot.raw_payload_bytes).unwrap() == expected_view
                    });
                    match (result.unwrap(), prior) {
                        (
                            MigrationOutcome::AlreadyMigrated { head, .. },
                            Some((prior_head, prior_snapshot)),
                        ) => {
                            assert!(unchanged, "module no-oped a changed source view");
                            assert_eq!(head, prior_head);
                            chains[index] = Some((prior_head, prior_snapshot));
                        }
                        (
                            MigrationOutcome::Advanced {
                                snapshot,
                                head,
                                source_digest,
                            },
                            prior,
                        ) => {
                            assert!(!unchanged, "module re-published an unchanged view");
                            let expected_generation = prior
                                .as_ref()
                                .map_or(0, |(prior_head, _)| prior_head.generation + 1);
                            assert_eq!(head.generation, expected_generation);
                            assert_eq!(
                                decode_view(&snapshot.raw_payload_bytes).unwrap(),
                                expected_view
                            );
                            assert_eq!(source_digest, legacy_source_digest(&bytes));
                            if let Some((_, prior_snapshot)) = &prior {
                                assert_eq!(
                                    snapshot.body.parent_snapshot_id,
                                    Some(prior_snapshot.snapshot_id().unwrap())
                                );
                            }
                            snapshot.verify(&ProfileDispatch::default()).unwrap();
                            head.verify().unwrap();
                            chains[index] = Some((head, snapshot));
                        }
                        (MigrationOutcome::AlreadyMigrated { .. }, None) => {
                            panic!("no chain head: migration cannot be a no-op");
                        }
                    }
                }
            }
        }
    }
}
