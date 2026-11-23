//! Store authority contract test for ID/Atlas slice C9135-01 (issue #10167).
//!
//! Oracle: **research-matrix** — the report at
//! `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c/store-placement-research-report.json`
//! maps the blob/event/world/sync authority owners to source paths,
//! states exact hash/profile facts without conflation, and records the
//! Atlas `content_hash` producer status without inferred
//! interoperability. Verifier: the owned validator
//! (`validate_store_authority.py` in the same directory) checks the
//! same assertions and, with `--check-test`, invokes this test.
//!
//! The pinned map below is the contract: each authority is owned by
//! exactly one crate, evidenced by exactly one source seam, and
//! carries exactly one identity profile. A map that hands one
//! authority to another crate, conflates two identity profiles, or
//! drops an authority is rejected by [`validate_authority_map`] —
//! the flattening failure this slice exists to prevent (the same
//! failure `tests/store_port_boundary.rs` rejects for port kinds).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::PathBuf;

/// One authority row as the contract pins it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct AuthorityEntry {
    surface: &'static str,
    owner_crate: &'static str,
    /// Source seam path, relative to `pkgs/id/`.
    source_path: &'static str,
    identity_profile: &'static str,
}

/// The pinned authority map (C9135-01, from the research record).
fn pinned_map() -> Vec<AuthorityEntry> {
    vec![
        AuthorityEntry {
            surface: "blob",
            owner_crate: "id-store",
            source_path: "crates/id-store/src/store.rs",
            identity_profile: "iroh-blob-blake3",
        },
        AuthorityEntry {
            surface: "event",
            owner_crate: "id-events",
            source_path: "crates/id-events/src/event_store/mod.rs",
            identity_profile: "event-payload-blake3",
        },
        AuthorityEntry {
            surface: "world",
            owner_crate: "id-worlds",
            source_path: "crates/id-worlds/src/world_store.rs",
            identity_profile: "world-module-blake3",
        },
        AuthorityEntry {
            surface: "sync",
            owner_crate: "id-sync",
            source_path: "crates/id-sync/src/envelope_outbox.rs",
            identity_profile: "artifact-sha256",
        },
    ]
}

/// Why an authority map is not the contract map.
#[derive(Debug, PartialEq, Eq)]
enum AuthorityMapError {
    /// An authority is assigned to a crate that does not own it.
    WrongOwner { surface: String, owner: String },
    /// An authority carries an identity profile that is not its own,
    /// or two authorities share one profile: profiles were conflated.
    ConflatedProfile { surface: String, profile: String },
    /// A required authority is missing from the map.
    MissingSurface(&'static str),
    /// A surface outside the four authorities appears in the map.
    UnknownSurface(String),
}

/// Validate an authority map against the pinned contract: exactly the
/// four surfaces, each with its own owner and its own identity
/// profile, profiles pairwise distinct.
fn validate_authority_map(entries: &[AuthorityEntry]) -> Result<(), AuthorityMapError> {
    let pinned = pinned_map();
    let mut profiles = BTreeSet::new();
    for entry in entries {
        let Some(want) = pinned.iter().find(|p| p.surface == entry.surface) else {
            return Err(AuthorityMapError::UnknownSurface(entry.surface.to_string()));
        };
        if entry.owner_crate != want.owner_crate {
            return Err(AuthorityMapError::WrongOwner {
                surface: entry.surface.to_string(),
                owner: entry.owner_crate.to_string(),
            });
        }
        if entry.identity_profile != want.identity_profile
            || !profiles.insert(entry.identity_profile)
        {
            return Err(AuthorityMapError::ConflatedProfile {
                surface: entry.surface.to_string(),
                profile: entry.identity_profile.to_string(),
            });
        }
    }
    for want in &pinned {
        if !entries.iter().any(|e| e.surface == want.surface) {
            return Err(AuthorityMapError::MissingSurface(want.surface));
        }
    }
    Ok(())
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_root() -> PathBuf {
    manifest_dir()
        .parent()
        .and_then(|p| p.parent())
        .expect("pkgs/id must sit two levels below the repo root")
        .to_path_buf()
}

fn load_report() -> serde_json::Value {
    let path = repo_root().join(
        "doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c/\
         store-placement-research-report.json",
    );
    let text = std::fs::read_to_string(&path)
        .expect("the C9135-01 research report must exist (owned file of #10167)");
    serde_json::from_str(&text).expect("the research report must be valid JSON")
}

/// Build owned-map entries from the report JSON, asserting each
/// report source path names the pinned seam file.
fn entries_from_report(report: &serde_json::Value) -> Vec<AuthorityEntry> {
    let pinned = pinned_map();
    let authorities = report["authorities"]
        .as_array()
        .expect("report must carry an authorities array");
    let mut entries = Vec::new();
    for row in authorities {
        let surface = row["surface"].as_str().expect("surface must be a string");
        let want = pinned
            .iter()
            .find(|p| p.surface == surface)
            .unwrap_or_else(|| panic!("report names unknown surface {surface}"));
        // The report records repo-root-relative paths; the contract
        // pins the same seam relative to pkgs/id.
        let report_path = row["source_path"]
            .as_str()
            .expect("source_path must be a string");
        assert_eq!(
            report_path,
            format!("pkgs/id/{}", want.source_path),
            "report source path for {surface} must name the pinned seam"
        );
        entries.push(AuthorityEntry {
            surface: want.surface,
            owner_crate: match row["owner_crate"].as_str() {
                Some("id-store") => "id-store",
                Some("id-events") => "id-events",
                Some("id-worlds") => "id-worlds",
                Some("id-sync") => "id-sync",
                other => panic!("report names unpinned owner crate {other:?} for {surface}"),
            },
            source_path: want.source_path,
            identity_profile: match row["identity"]["profile"].as_str() {
                Some("iroh-blob-blake3") => "iroh-blob-blake3",
                Some("event-payload-blake3") => "event-payload-blake3",
                Some("world-module-blake3") => "world-module-blake3",
                Some("artifact-sha256") => "artifact-sha256",
                other => panic!("report names unpinned identity profile {other:?} for {surface}"),
            },
        });
    }
    entries
}

#[test]
fn report_maps_each_authority_to_its_owner_source_and_profile() {
    let report = load_report();
    assert_eq!(report["slice"], "C9135-01");
    assert_eq!(report["issue"], 10167);

    let entries = entries_from_report(&report);
    validate_authority_map(&entries).expect("the report's authority map is the contract map");

    // Every pinned seam file exists at the path the report names.
    for entry in &entries {
        assert!(
            manifest_dir().join(entry.source_path).exists(),
            "authority source seam {} must exist",
            entry.source_path
        );
    }

    // Atlas producer status is recorded, not inferred: unavailable,
    // no producer, and interop claims forbidden (slice 9110-R2).
    let atlas = &report["atlas_content_hash"];
    assert_eq!(atlas["status"], "producer_unavailable");
    assert!(atlas["producer"].is_null());
    assert_eq!(atlas["interop_claims"], "forbidden");
}

#[test]
fn identity_profiles_differ_in_facts_not_just_names() {
    let report = load_report();
    let facts = |profile: &str| -> (String, String) {
        for row in report["authorities"].as_array().unwrap() {
            let ident = &row["identity"];
            if ident["profile"] == profile {
                return (
                    ident["algorithm"].as_str().unwrap().to_string(),
                    ident["input"].as_str().unwrap().to_string(),
                );
            }
        }
        let atlas = &report["atlas_content_hash"];
        assert_eq!(atlas["profile"], profile, "unknown profile {profile}");
        (
            atlas["algorithm"].as_str().unwrap().to_string(),
            atlas["input"].as_str().unwrap().to_string(),
        )
    };
    // The two BLAKE3 profiles share an algorithm and differ in what
    // is hashed and where the commitment lives; the two SHA-256
    // profiles likewise. Neither pair is interchangeable.
    let blob = facts("iroh-blob-blake3");
    let event = facts("event-payload-blake3");
    assert_eq!(blob.0, event.0, "both profiles are BLAKE3-256");
    assert_ne!(blob, event, "blob and event BLAKE3 commitments differ in input");
    let artifact = facts("artifact-sha256");
    let atlas = facts("atlas-content-hash");
    assert_eq!(artifact.0, atlas.0, "both profiles are SHA-256");
    assert_ne!(
        artifact, atlas,
        "artifact SHA-256 over canonical JSON differs from observed Atlas SHA-256 over chunk text"
    );
    // Every forbidden equivalence in the report pairs profiles whose
    // recorded facts actually differ.
    for pair in report["non_conflation"]["forbidden_equivalences"]
        .as_array()
        .unwrap()
    {
        let (left, right) = (pair[0].as_str().unwrap(), pair[1].as_str().unwrap());
        assert_ne!(
            facts(left),
            facts(right),
            "forbidden equivalence {left} != {right} must rest on differing facts"
        );
    }
}

#[test]
fn source_evidence_carries_each_identity_mechanism() {
    // The mechanism markers the research record cites, checked
    // against the live sources: if a seam moves or its identity
    // mechanism changes, this test and the record must be revisited.
    let cases: &[(&str, &[&str])] = &[
        ("crates/id-store/src/store.rs", &["FsStore", "MemStore"]),
        (
            "crates/id-events/src/event_store/ingest.rs",
            &["Hash::digest"],
        ),
        ("crates/id-worlds/src/world_store.rs", &["journal.jsonl"]),
        ("crates/id-sync/src/artifact.rs", &["signed_bytes", "Sha256"]),
    ];
    for (rel, markers) in cases {
        let text = std::fs::read_to_string(manifest_dir().join(rel))
            .unwrap_or_else(|_| panic!("evidence source {rel} must exist"));
        for marker in *markers {
            assert!(
                text.contains(marker),
                "evidence source {rel} must carry marker {marker}"
            );
        }
    }
}

#[test]
fn fixture_rejects_flattened_and_conflated_maps() {
    // Handing the event log to the blob store's crate is an ownership
    // flattening, even with every surface present.
    let mut wrong_owner = pinned_map();
    wrong_owner[1].owner_crate = "id-store";
    assert_eq!(
        validate_authority_map(&wrong_owner),
        Err(AuthorityMapError::WrongOwner {
            surface: "event".to_string(),
            owner: "id-store".to_string(),
        })
    );

    // Recording the sync surface under the blob profile conflates a
    // SHA-256 authoring commitment with exact-byte BLAKE3 identity.
    let mut conflated = pinned_map();
    conflated[3].identity_profile = "iroh-blob-blake3";
    assert_eq!(
        validate_authority_map(&conflated),
        Err(AuthorityMapError::ConflatedProfile {
            surface: "sync".to_string(),
            profile: "iroh-blob-blake3".to_string(),
        })
    );

    // Dropping the world authority (treating the journal as part of
    // the event store) is a missing-surface rejection, not a pass.
    let missing_world = &pinned_map()[..3];
    assert_eq!(
        validate_authority_map(missing_world),
        Err(AuthorityMapError::MissingSurface("sync")),
        "the truncated fixture drops the last surface first"
    );
    let mut no_world = pinned_map();
    no_world.remove(2);
    assert_eq!(
        validate_authority_map(&no_world),
        Err(AuthorityMapError::MissingSurface("world"))
    );
}
