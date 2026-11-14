//! Store port boundary test for ID/Atlas slice 9110-01 (issue #9418).
//!
//! Oracle: **invariant** — the architecture boundary names the blob, event,
//! world, and sync stores separately, and a fixture rejects their flattening.
//! Verifier: the seam-map classification table in `pkgs/id/ARCHITECTURE.md`
//! (section "Store Port Boundaries") plus the compile-time trait ownership
//! assignments below.
//!
//! Each of the four ports is owned by exactly one crate and one concrete
//! type family. The local [`StorePort`] trait is implemented once per port,
//! for that port's own type only, so assigning a second port's kind or
//! authority to a type is a compile error at the `impl` site, and the
//! runtime classification validator rejects any table that flattens two
//! ports into one kind.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::any::{Any, TypeId};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// One row of the seam-map classification: a single port, owned by a
/// single crate, holding a single authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PortEntry {
    kind: &'static str,
    owner_crate: &'static str,
    authority: &'static str,
}

/// Compile-time ownership assignment: a type may carry exactly one port.
trait StorePort {
    /// The port kind this type owns: `blob`, `event`, `world`, or `sync`.
    const ENTRY: PortEntry;
}

// Blob port: content-addressed bytes, owned by `id-store`.
impl StorePort for id_store::store::StoreType {
    const ENTRY: PortEntry = PortEntry {
        kind: "blob",
        owner_crate: "id-store",
        authority: "content-addressed blob bytes and name-to-hash tags",
    };
}

// Event port: the signed event log, owned by `id-events`.
impl StorePort for id_events::event_store::EventStore {
    const ENTRY: PortEntry = PortEntry {
        kind: "event",
        owner_crate: "id-events",
        authority: "signed event log: chains, heads, fork proofs, waiting pool",
    };
}

// World port: durable world state, owned by `id-worlds`. The fsynced
// journal is the world store's durable record; modules live beside it.
impl StorePort for id_worlds::world_store::FileJournal {
    const ENTRY: PortEntry = PortEntry {
        kind: "world",
        owner_crate: "id-worlds",
        authority: "durable world state: journal replay and installed modules",
    };
}

// Sync port: delivery only, owned by `id-sync`. The envelope outbox is
// sync's durable seam; it holds pending deliveries, never store authority.
impl StorePort for id_sync::envelope_outbox::Outbox {
    const ENTRY: PortEntry = PortEntry {
        kind: "sync",
        owner_crate: "id-sync",
        authority: "pending envelope delivery only; no stored-data authority",
    };
}

/// The classification the architecture document must agree with.
fn classification() -> Vec<PortEntry> {
    vec![
        id_store::store::StoreType::ENTRY,
        id_events::event_store::EventStore::ENTRY,
        id_worlds::world_store::FileJournal::ENTRY,
        id_sync::envelope_outbox::Outbox::ENTRY,
    ]
}

/// Why a classification table is not a valid port boundary map.
#[derive(Debug, PartialEq, Eq)]
enum FlatteningError {
    /// Two rows claim the same port kind: the ports were flattened.
    DuplicateKind(&'static str),
    /// Two rows claim the same authority: one store was made to own
    /// another store's decisions.
    DuplicateAuthority(&'static str),
    /// A required port is missing from the table.
    MissingKind(&'static str),
}

/// Validate a classification table against the boundary invariant:
/// exactly the four known kinds, each exactly once, each with its own
/// authority.
fn validate_classification(entries: &[PortEntry]) -> Result<(), FlatteningError> {
    let mut kinds = BTreeSet::new();
    let mut authorities = BTreeSet::new();
    for entry in entries {
        if !kinds.insert(entry.kind) {
            return Err(FlatteningError::DuplicateKind(entry.kind));
        }
        if !authorities.insert(entry.authority) {
            return Err(FlatteningError::DuplicateAuthority(entry.authority));
        }
    }
    for required in ["blob", "event", "world", "sync"] {
        if !kinds.contains(required) {
            return Err(FlatteningError::MissingKind(required));
        }
    }
    Ok(())
}

fn assert_distinct_types<A: Any, B: Any>() {
    assert_ne!(
        TypeId::of::<A>(),
        TypeId::of::<B>(),
        "port owner types must be distinct types, not one flattened store type"
    );
}

#[test]
fn each_port_is_a_distinct_type_with_a_distinct_owner() {
    // Compile-time ownership: the four `StorePort` impls above only
    // compile because each port kind is claimed by its own type. The
    // runtime assertions pin the resulting classification.
    assert_distinct_types::<id_store::store::StoreType, id_events::event_store::EventStore>();
    assert_distinct_types::<id_store::store::StoreType, id_worlds::world_store::FileJournal>();
    assert_distinct_types::<id_store::store::StoreType, id_sync::envelope_outbox::Outbox>();
    assert_distinct_types::<id_events::event_store::EventStore, id_worlds::world_store::FileJournal>(
    );
    assert_distinct_types::<id_events::event_store::EventStore, id_sync::envelope_outbox::Outbox>();
    assert_distinct_types::<id_worlds::world_store::FileJournal, id_sync::envelope_outbox::Outbox>(
    );

    let entries = classification();
    assert_eq!(entries.len(), 4);
    validate_classification(&entries).expect("the real classification is not flattened");
}

#[test]
fn fixture_rejects_flattening_all_stores_into_one() {
    // The flattening fixture: one generic "store" kind claiming every
    // port, the failure mode the port contract forbids ("store
    // abstraction" merging blob/event/world/sync authority).
    let flattened = vec![
        PortEntry {
            kind: "store",
            ..id_store::store::StoreType::ENTRY
        },
        PortEntry {
            kind: "store",
            ..id_events::event_store::EventStore::ENTRY
        },
        PortEntry {
            kind: "store",
            ..id_worlds::world_store::FileJournal::ENTRY
        },
        PortEntry {
            kind: "store",
            ..id_sync::envelope_outbox::Outbox::ENTRY
        },
    ];
    assert_eq!(
        validate_classification(&flattened),
        Err(FlatteningError::DuplicateKind("store")),
        "a fixture that flattens the four ports into one store kind must be rejected"
    );

    // Flattening by authority instead of by kind is rejected too: the
    // event log must not be made to own blob authority.
    let mut merged_authority = classification();
    merged_authority[1].authority = merged_authority[0].authority;
    assert_eq!(
        validate_classification(&merged_authority),
        Err(FlatteningError::DuplicateAuthority(
            "content-addressed blob bytes and name-to-hash tags"
        ))
    );

    // Dropping a port (treating sync as part of the blob store, so it
    // never appears) is a missing-kind rejection, not a silent pass.
    let missing_sync = &classification()[..3];
    assert_eq!(
        validate_classification(missing_sync),
        Err(FlatteningError::MissingKind("sync"))
    );
}

#[test]
fn architecture_doc_names_each_store_separately() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let doc = std::fs::read_to_string(manifest_dir.join("ARCHITECTURE.md"))
        .expect("pkgs/id/ARCHITECTURE.md must exist (consumer-owned file of 9110-01)");
    let section_start = doc
        .find("## Store Port Boundaries")
        .expect("ARCHITECTURE.md must carry the Store Port Boundaries section (9110-01)");
    let section = &doc[section_start..];

    // The classification table names each port with its owning crate
    // and concrete seam, separately — one row per port.
    for entry in classification() {
        assert!(
            section.contains(entry.kind),
            "boundary section must name the {} port",
            entry.kind
        );
        assert!(
            section.contains(entry.owner_crate),
            "boundary section must name {} as owner of the {} port",
            entry.owner_crate,
            entry.kind
        );
        assert!(
            section.contains(entry.authority),
            "boundary section must state the {} port's authority: {}",
            entry.kind,
            entry.authority
        );
    }
    for seam in [
        "StoreType",
        "EventStore",
        "FileJournal",
        "Outbox",
        "consumer-owned",
        "compatibility adapter",
    ] {
        assert!(
            section.contains(seam),
            "boundary section must mention {seam}"
        );
    }
    assert!(
        section.contains("does not merge"),
        "boundary section must state that a store abstraction does not merge the ports"
    );
}

#[test]
fn research_evidence_sources_exist() {
    // The evidence files the slice's seam map is built from (issue
    // #9418): if one moves, the map and this test must be revisited.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for rel in [
        "crates/id-store/src/store.rs",
        "crates/id-events/src/event_store/mod.rs",
        "crates/id-worlds/src/world_store.rs",
        "crates/id-sync/src/envelope_outbox.rs",
    ] {
        assert!(
            manifest_dir.join(rel).exists(),
            "seam-map evidence source {rel} must exist"
        );
    }
}
