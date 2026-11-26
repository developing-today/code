//! The storage plane of the config document: `storage`.
//!
//! Second part of slice 9111-03 in the #9111 record: §3.4 (storage
//! bindings), the object merge rule of §6.3 (an absent key inherits the
//! built-in default, an explicit `null` clears to it), §6.4 (unknown keys
//! in `storage` fail closed) and §6.5 step 4 (each adapter named in
//! `storage` is advertised by the binary with the needed capability).
//!
//! A binding names the adapter family behind one store and its declared
//! durability, sync participation and retention class. Which adapters
//! exist is not config: it is what this build advertises
//! ([`ADVERTISED_STORE_ADAPTERS`], [`ADVERTISED_OBJECT_STORE_ADAPTERS`]).
//! A binding that names an adapter or a durability the build does not
//! advertise rejects the document at load, never falls back at runtime
//! (record §3.4: "capability mismatch and invalid config are rejected
//! before operation side effects").
//!
//! This build advertises `redb` with `fsync` durability for the local
//! store and the event log, and no object-store adapter, so
//! `storage.object_store` can only be absent or `null` until one lands.

use serde::{Deserialize, Serialize};

use crate::ConfigError;

/// An adapter family this build can back a local store or the event log
/// with, and the durabilities it advertises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdvertisedStoreAdapter {
    /// The adapter's config name.
    pub name: &'static str,
    /// The durability values it can honour.
    pub durabilities: &'static [&'static str],
}

/// The local-store and event-log adapters this build advertises (record
/// §6.5 step 4).
pub const ADVERTISED_STORE_ADAPTERS: &[AdvertisedStoreAdapter] = &[AdvertisedStoreAdapter {
    name: "redb",
    durabilities: &["fsync"],
}];

/// The object-store adapters this build advertises (record §6.5 step 4).
/// None yet: the `fs`, `s3` and `r2` families of record §3.4 are named by
/// the design and have no implementation in this build.
pub const ADVERTISED_OBJECT_STORE_ADAPTERS: &[&str] = &[];

/// The default adapter for the local store and the event log (record
/// §3.4).
pub const DEFAULT_STORE_ADAPTER: &str = "redb";

/// The default durability (record §3.4).
pub const DEFAULT_DURABILITY: &str = "fsync";

/// The default object-store retention class (record §3.4).
pub const DEFAULT_RETENTION_CLASS: &str = "default";

/// Whether a store's records take part in sync (record §3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncParticipation {
    /// The store's records are synced.
    Full,
    /// The store is local only.
    None,
}

impl SyncParticipation {
    /// The wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::None => "none",
        }
    }
}

/// One store's binding as written. Every key is optional; an absent key
/// or an explicit `null` takes the built-in default for that store.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreBinding {
    /// The adapter family. Default `redb`.
    #[serde(default)]
    pub adapter: Option<String>,
    /// The declared durability. Default `fsync`.
    #[serde(default)]
    pub durability: Option<String>,
    /// Whether the store syncs. Default `full` for the local store,
    /// `none` for the event log.
    #[serde(default)]
    pub sync_participation: Option<SyncParticipation>,
}

/// The object-store binding as written. Naming one requires an adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectStoreBinding {
    /// The adapter family (record §3.4 names `fs`, `s3`, `r2`).
    pub adapter: String,
    /// The retention class. Default `default`.
    #[serde(default)]
    pub retention_class: Option<String>,
}

/// The `storage` section as written (record §3.4). An absent section, an
/// absent key, or an explicit `null` means the built-in default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Storage {
    /// The local persistent store.
    #[serde(default)]
    pub local_store: Option<StoreBinding>,
    /// The event log.
    #[serde(default)]
    pub event_log: Option<StoreBinding>,
    /// The object store. Absent or `null` means no object store.
    #[serde(default)]
    pub object_store: Option<ObjectStoreBinding>,
}

/// One store's effective binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedStoreBinding {
    /// The adapter family.
    pub adapter: String,
    /// The durability.
    pub durability: String,
    /// Whether the store syncs.
    pub sync_participation: SyncParticipation,
}

/// The effective object-store binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedObjectStoreBinding {
    /// The adapter family.
    pub adapter: String,
    /// The retention class.
    pub retention_class: String,
}

/// The effective storage plane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedStorage {
    /// The local persistent store.
    pub local_store: ResolvedStoreBinding,
    /// The event log.
    pub event_log: ResolvedStoreBinding,
    /// The object store, if one is bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_store: Option<ResolvedObjectStoreBinding>,
}

impl Default for ResolvedStorage {
    fn default() -> Self {
        Storage::default().resolved()
    }
}

impl ResolvedStorage {
    /// Whether this is the built-in default, which the canonical encoding
    /// omits so a document without `storage` keeps its digest.
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

impl StoreBinding {
    fn resolved(binding: Option<&Self>, sync: SyncParticipation) -> ResolvedStoreBinding {
        let binding = binding.cloned().unwrap_or_default();
        ResolvedStoreBinding {
            adapter: binding
                .adapter
                .unwrap_or_else(|| DEFAULT_STORE_ADAPTER.to_owned()),
            durability: binding
                .durability
                .unwrap_or_else(|| DEFAULT_DURABILITY.to_owned()),
            sync_participation: binding.sync_participation.unwrap_or(sync),
        }
    }
}

impl Storage {
    /// The effective bindings: every absent key takes its default.
    #[must_use]
    pub fn resolved(&self) -> ResolvedStorage {
        ResolvedStorage {
            local_store: StoreBinding::resolved(self.local_store.as_ref(), SyncParticipation::Full),
            event_log: StoreBinding::resolved(self.event_log.as_ref(), SyncParticipation::None),
            object_store: self
                .object_store
                .as_ref()
                .map(|binding| ResolvedObjectStoreBinding {
                    adapter: binding.adapter.clone(),
                    retention_class: binding
                        .retention_class
                        .clone()
                        .unwrap_or_else(|| DEFAULT_RETENTION_CLASS.to_owned()),
                }),
        }
    }

    /// Check every binding against what this build advertises.
    pub(crate) fn validate(&self) -> Result<(), ConfigError> {
        let resolved = self.resolved();
        for (store, binding) in [
            ("local_store", &resolved.local_store),
            ("event_log", &resolved.event_log),
        ] {
            let Some(advertised) = ADVERTISED_STORE_ADAPTERS
                .iter()
                .find(|adapter| adapter.name == binding.adapter)
            else {
                return Err(ConfigError::UnadvertisedStorageAdapter {
                    store,
                    adapter: binding.adapter.clone(),
                });
            };
            if !advertised
                .durabilities
                .contains(&binding.durability.as_str())
            {
                return Err(ConfigError::UnadvertisedDurability {
                    store,
                    adapter: binding.adapter.clone(),
                    durability: binding.durability.clone(),
                });
            }
        }
        if let Some(object_store) = &resolved.object_store {
            if object_store.retention_class.trim().is_empty() {
                return Err(ConfigError::EmptyRetentionClass);
            }
            if !ADVERTISED_OBJECT_STORE_ADAPTERS.contains(&object_store.adapter.as_str()) {
                return Err(ConfigError::UnadvertisedStorageAdapter {
                    store: "object_store",
                    adapter: object_store.adapter.clone(),
                });
            }
        }
        Ok(())
    }
}
