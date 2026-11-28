//! Regression tests for the stress generator's output lifecycle
//! (#13181): a generation interrupted before its manifest is
//! published must be recoverable on retry, and no directory whose
//! contents cannot be verified as a stress generation may ever be
//! deleted to make room for one.
//!
//! The oracle throughout is identity, not presence: deletion is
//! gated on the staging ownership marker or the manifest's type +
//! schema version, both defined in `common::stress_output` — the
//! same code the `stress_gen` bench profile runs.

mod common;

use std::path::{Path, PathBuf};

use common::loadgen::GENERATOR_VERSION;
use common::stress_output::{
    prepare_stress_output, publish_stress_output, stress_dir_is_completed_generation,
    stress_staging_path,
};
use id::world::{
    JournalEntry, Participant, WorldCore, WorldEvent, WorldEventKind, WorldLimits, WorldScopes,
};

const SEED: u64 = 0x5891;

fn temp_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "id-stress-gen-recovery-{}-{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test root");
    dir
}

fn sample_manifest(seed: u64) -> serde_json::Value {
    serde_json::json!({
        "type": "stress-manifest",
        "schema_version": 1,
        "generator_version": GENERATOR_VERSION,
        "seed": seed,
        "tier": "quick",
        "worlds": [],
        "totals": {"worlds": 0, "measured_worlds": 0, "events": 0},
    })
}

/// Write one measured world in the store's journal format under
/// `root` and prove the store restores it at the generated
/// sequence — the same verification the generator performs before
/// recording a world in the manifest.
fn write_and_verify_world(root: &Path, name: &str, events: u64) {
    let mut entries = vec![
        JournalEntry::Created {
            world_id: name.to_owned(),
            version: 1,
        },
        JournalEntry::Issued {
            participant: Participant {
                id: 1,
                display_name: "stress-0".to_owned(),
            },
            digest: "0".repeat(64),
            scopes: WorldScopes::GUEST.bits(),
            parent: None,
            expires_at: None,
            uses: None,
            used: 0,
            subject: None,
            issued_at: None,
        },
    ];
    entries.extend((1..=events).map(|sequence| JournalEntry::Committed {
        event: WorldEvent {
            sequence,
            participant_id: 1,
            kind: WorldEventKind::Chat(format!("message {sequence}")),
        },
    }));
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).expect("create world dir");
    let mut bytes = Vec::new();
    for entry in &entries {
        serde_json::to_writer(&mut bytes, entry).expect("serialize journal entry");
        bytes.push(b'\n');
    }
    std::fs::write(dir.join("journal.jsonl"), &bytes).expect("write journal");
    let text = String::from_utf8(bytes).expect("journal is utf-8");
    let read_back: Vec<JournalEntry> = text
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()
        .expect("re-parse journal");
    let restored = WorldCore::restore(name, WorldLimits::default(), read_back)
        .expect("restore generated world");
    assert_eq!(restored.core.current_sequence(), events);
}

/// A generation that dies before publishing leaves a marked
/// staging directory; the retry with the same (seed, tier, output)
/// recovers it and publishes a complete fixture whose world
/// restores from the published location.
#[test]
fn interrupted_generation_recovers_on_retry() {
    let root = temp_root("recover");
    let out = root.join("fixture");
    // First attempt: staging prepared, one world materialized,
    // then the run dies before the manifest is published.
    let staging = prepare_stress_output(&out, SEED, "quick", GENERATOR_VERSION).unwrap();
    write_and_verify_world(&staging, "active-0000", 3);
    assert!(
        !out.exists(),
        "nothing may be published before the manifest"
    );
    // Retry: the leftover staging is verified as ours, recovered,
    // and the run publishes a complete, verifiable fixture.
    let staging = prepare_stress_output(&out, SEED, "quick", GENERATOR_VERSION).unwrap();
    write_and_verify_world(&staging, "active-0000", 3);
    let manifest_path = publish_stress_output(&out, &staging, &sample_manifest(SEED)).unwrap();
    assert!(stress_dir_is_completed_generation(&out));
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["type"], "stress-manifest");
    assert!(!staging.exists(), "publish renames the staging directory");
    // The published world restores from its published location.
    let text = std::fs::read_to_string(out.join("active-0000/journal.jsonl")).unwrap();
    let read_back: Vec<JournalEntry> = text
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()
        .unwrap();
    let restored = WorldCore::restore("active-0000", WorldLimits::default(), read_back).unwrap();
    assert_eq!(restored.core.current_sequence(), 3);
    std::fs::remove_dir_all(&root).unwrap();
}

/// A completed generation is replaced in place by the next run
/// (the pre-#13181 regeneration behaviour, now at publish time).
#[test]
fn completed_generation_is_replaced_on_republish() {
    let root = temp_root("replace");
    let out = root.join("fixture");
    let staging = prepare_stress_output(&out, SEED, "quick", GENERATOR_VERSION).unwrap();
    write_and_verify_world(&staging, "active-0000", 3);
    publish_stress_output(&out, &staging, &sample_manifest(SEED)).unwrap();
    // Regenerate with a different seed and a longer world: the
    // verified completed generation is accepted and replaced.
    let staging = prepare_stress_output(&out, SEED + 1, "quick", GENERATOR_VERSION).unwrap();
    write_and_verify_world(&staging, "active-0000", 5);
    publish_stress_output(&out, &staging, &sample_manifest(SEED + 1)).unwrap();
    let journal = std::fs::read_to_string(out.join("active-0000/journal.jsonl")).unwrap();
    // Created + one Issued + five events: the new generation, not
    // a merge of the two.
    assert_eq!(journal.lines().count(), 7);
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["seed"], SEED + 1);
    std::fs::remove_dir_all(&root).unwrap();
}

/// An output root whose manifest is foreign — present, but not a
/// stress manifest — is never deleted, however it got there.
#[test]
fn foreign_output_root_is_never_deleted() {
    let root = temp_root("foreign");
    let out = root.join("fixture");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("manifest.json"), "{\"type\":\"someone-else\"}\n").unwrap();
    std::fs::write(out.join("keep.txt"), "precious").unwrap();
    let err = prepare_stress_output(&out, SEED, "quick", GENERATOR_VERSION).unwrap_err();
    assert!(
        format!("{err:#}").contains("refusing to overwrite unverified content"),
        "{err:#}"
    );
    assert_eq!(
        std::fs::read_to_string(out.join("keep.txt")).unwrap(),
        "precious"
    );
    assert_eq!(
        std::fs::read_to_string(out.join("manifest.json")).unwrap(),
        "{\"type\":\"someone-else\"}\n"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

/// A partial generation written by the pre-#13181 code (worlds but
/// no manifest, and no staging marker to verify ownership) is
/// refused with the cleanup action named, and preserved.
#[test]
fn manifestless_output_root_refuses_with_cleanup_guidance() {
    let root = temp_root("manifestless");
    let out = root.join("fixture");
    let world = out.join("idle-00000");
    std::fs::create_dir_all(&world).unwrap();
    std::fs::write(world.join("journal.jsonl"), "{}\n").unwrap();
    let err = prepare_stress_output(&out, SEED, "quick", GENERATOR_VERSION).unwrap_err();
    assert!(
        format!("{err:#}").contains("remove the directory and re-run"),
        "{err:#}"
    );
    assert!(world.join("journal.jsonl").exists(), "nothing was deleted");
    std::fs::remove_dir_all(&root).unwrap();
}

/// A staging directory without a verifiable ownership marker is
/// foreign content, not a recoverable leftover.
#[test]
fn unmarked_staging_directory_is_never_deleted() {
    let root = temp_root("staging-foreign");
    let out = root.join("fixture");
    let staging = stress_staging_path(&out).unwrap();
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::write(staging.join("keep.txt"), "precious").unwrap();
    let err = prepare_stress_output(&out, SEED, "quick", GENERATOR_VERSION).unwrap_err();
    assert!(format!("{err:#}").contains("ownership marker"), "{err:#}");
    assert_eq!(
        std::fs::read_to_string(staging.join("keep.txt")).unwrap(),
        "precious"
    );
    std::fs::remove_dir_all(&root).unwrap();
}
