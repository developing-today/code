//! The storage plane `storage`, second part of slice 9111-03 (#9111
//! record §3.4, §6.3, §6.4, §6.5 step 4).
//!
//! Oracle: the record's rules, stated before the code. Defaults are
//! `redb` with `fsync` for the local store (syncing) and the event log
//! (local only), and no object store. An absent key and an explicit
//! `null` both take the default. Unknown keys in `storage` fail closed.
//! An adapter or durability the build does not advertise is a load
//! error, never a runtime fallback; this build advertises `redb` with
//! `fsync` and no object-store adapter. A document without `storage`, or
//! one that spells out the defaults, keeps the digest it had before the
//! plane existed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id_config::{
    ADVERTISED_OBJECT_STORE_ADAPTERS, ADVERTISED_STORE_ADAPTERS, ConfigDocument, ConfigError,
    FieldClass, ResolvedConfig, SyncParticipation, explain, field_class, resolved_digest,
};
use serde_json::{Value, json};

fn base() -> Value {
    json!({
        "schema_version": 1,
        "config_id": "cfg-storage",
        "revision": 1,
        "mode": "federated-relay",
        "protocol_versions": ["id/2.0"],
        "ceilings": {
            "max_hops": 5,
            "max_bytes": 1_048_576,
            "max_time_ms": 30_000,
            "max_fan_out": 16
        }
    })
}

fn with_storage(storage: Value) -> Value {
    let mut doc = base();
    doc["storage"] = storage;
    doc
}

fn load(value: &Value) -> Result<ResolvedConfig, ConfigError> {
    ConfigDocument::parse(&value.to_string())?.resolve()
}

fn load_err(value: &Value) -> ConfigError {
    match load(value) {
        Ok(resolved) => panic!("expected rejection, resolved {resolved:?}"),
        Err(err) => err,
    }
}

#[test]
fn an_absent_section_takes_every_default() {
    let resolved = load(&base()).unwrap();
    let storage = &resolved.storage;
    assert_eq!(storage.local_store.adapter, "redb");
    assert_eq!(storage.local_store.durability, "fsync");
    assert_eq!(
        storage.local_store.sync_participation,
        SyncParticipation::Full
    );
    assert_eq!(storage.event_log.adapter, "redb");
    assert_eq!(storage.event_log.durability, "fsync");
    assert_eq!(
        storage.event_log.sync_participation,
        SyncParticipation::None
    );
    assert!(storage.object_store.is_none());
    assert!(storage.is_default());
}

#[test]
fn null_and_absent_keys_inherit_the_default() {
    let resolved = load(&with_storage(json!({
        "local_store": null,
        "event_log": { "adapter": null, "durability": null, "sync_participation": "full" },
        "object_store": null
    })))
    .unwrap();
    assert_eq!(resolved.storage.local_store.adapter, "redb");
    assert_eq!(resolved.storage.event_log.adapter, "redb");
    assert_eq!(resolved.storage.event_log.durability, "fsync");
    assert_eq!(
        resolved.storage.event_log.sync_participation,
        SyncParticipation::Full
    );
    assert!(resolved.storage.object_store.is_none());
}

#[test]
fn defaults_spelled_out_keep_the_digest() {
    let absent = resolved_digest(&load(&base()).unwrap()).unwrap();
    let spelled = resolved_digest(
        &load(&with_storage(json!({
            "local_store": { "adapter": "redb", "durability": "fsync", "sync_participation": "full" },
            "event_log": { "adapter": "redb", "durability": "fsync", "sync_participation": "none" }
        })))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(absent, spelled);
    let canonical = load(&base()).unwrap().canonical_json().unwrap();
    assert!(
        !canonical.contains("\"storage\""),
        "the default storage plane is omitted from the canonical encoding"
    );
}

#[test]
fn a_changed_binding_changes_the_digest() {
    let absent = resolved_digest(&load(&base()).unwrap()).unwrap();
    let changed = load(&with_storage(
        json!({ "event_log": { "sync_participation": "full" } }),
    ))
    .unwrap();
    assert!(changed.canonical_json().unwrap().contains("\"storage\""));
    assert_ne!(absent, resolved_digest(&changed).unwrap());
}

#[test]
fn an_unadvertised_store_adapter_is_a_load_error() {
    for store in ["local_store", "event_log"] {
        let err = load_err(&with_storage(json!({ store: { "adapter": "turso" } })));
        assert!(
            matches!(
                &err,
                ConfigError::UnadvertisedStorageAdapter { store: s, adapter } if *s == store && adapter == "turso"
            ),
            "{store}: {err:?}"
        );
    }
}

#[test]
fn an_unadvertised_durability_is_a_load_error() {
    let err = load_err(&with_storage(
        json!({ "local_store": { "durability": "none" } }),
    ));
    assert!(
        matches!(
            &err,
            ConfigError::UnadvertisedDurability { store: "local_store", adapter, durability }
                if adapter == "redb" && durability == "none"
        ),
        "{err:?}"
    );
}

#[test]
fn no_object_store_adapter_is_advertised_yet() {
    assert_eq!(ADVERTISED_OBJECT_STORE_ADAPTERS, [] as [&str; 0]);
    for adapter in ["fs", "s3", "r2"] {
        let err = load_err(&with_storage(
            json!({ "object_store": { "adapter": adapter } }),
        ));
        assert!(
            matches!(
                &err,
                ConfigError::UnadvertisedStorageAdapter { store: "object_store", adapter: a } if a == adapter
            ),
            "{adapter}: {err:?}"
        );
    }
}

#[test]
fn an_object_store_needs_an_adapter_and_a_non_empty_retention_class() {
    let missing = load_err(&with_storage(
        json!({ "object_store": { "retention_class": "default" } }),
    ));
    assert!(matches!(missing, ConfigError::Parse(_)), "{missing:?}");
    let empty = load_err(&with_storage(
        json!({ "object_store": { "adapter": "fs", "retention_class": " " } }),
    ));
    assert!(
        matches!(empty, ConfigError::EmptyRetentionClass),
        "{empty:?}"
    );
}

#[test]
fn unknown_keys_in_storage_fail_closed() {
    for storage in [
        json!({ "cache": {} }),
        json!({ "local_store": { "fallback": "memory" } }),
        json!({ "object_store": { "adapter": "fs", "bucket": "x" } }),
    ] {
        let err = load_err(&with_storage(storage.clone()));
        assert!(matches!(err, ConfigError::Parse(_)), "{storage}: {err:?}");
    }
    let err = load_err(&with_storage(
        json!({ "event_log": { "sync_participation": "partial" } }),
    ));
    assert!(matches!(err, ConfigError::Parse(_)), "{err:?}");
}

#[test]
fn the_advertised_table_is_redb_with_fsync() {
    assert_eq!(ADVERTISED_STORE_ADAPTERS.len(), 1);
    assert_eq!(ADVERTISED_STORE_ADAPTERS[0].name, "redb");
    assert_eq!(ADVERTISED_STORE_ADAPTERS[0].durabilities, &["fsync"]);
}

#[test]
fn storage_is_behavior() {
    assert_eq!(field_class("storage"), Some(FieldClass::Behavior));
    assert_eq!(
        field_class("storage.local_store.adapter"),
        Some(FieldClass::Behavior)
    );
}

#[test]
fn explain_names_the_source_of_each_binding() {
    let value = with_storage(json!({
        "event_log": { "sync_participation": "full", "durability": null }
    }));
    let explanation = explain(&value.to_string()).unwrap();
    let entry = |path: &str| {
        explanation
            .entries
            .iter()
            .find(|e| e.path == path)
            .unwrap_or_else(|| panic!("no explain entry for {path}"))
    };
    assert_eq!(entry("storage.event_log.sync_participation").value, "full");
    assert_eq!(
        entry("storage.event_log.sync_participation").source,
        "document"
    );
    assert_eq!(entry("storage.event_log.durability").value, "fsync");
    assert_eq!(entry("storage.event_log.durability").source, "default");
    assert_eq!(entry("storage.local_store.adapter").source, "default");
    assert!(
        explanation
            .entries
            .iter()
            .all(|e| !e.path.starts_with("storage.object_store")),
        "no object store is bound"
    );
}
