//! The tier map of the config document: `tiers`.
//!
//! Third part of slice 9111-03 in the #9111 record: §3.5 (tier
//! identifiers map to placement and retention), with the object merge
//! rule of §6.3 (an absent key takes the built-in default) and §6.4
//! (unknown keys in a tier fail closed).
//!
//! A record carries a signed `data_tier` or `storage_tier` identifier;
//! this map only says where records of each tier are placed and how long
//! they are kept. Changing the map never rewrites a record (record Q14),
//! and a tier label never authorizes access (record §3.5). So nothing
//! here grants or denies an operation by tier: callers ask
//! [`ResolvedTiers::lookup`] where a record goes, and an identifier the
//! map does not name is held in `default` placement and refused for any
//! placement-dependent operation ([`TierLookup::HeldInDefault`]), never
//! silently reclassified.
//!
//! With no `tiers` section every record is in the built-in `default`
//! tier: local placement, kept indefinitely. A document may restate or
//! change `default`, and it is always present in the resolved map.
//! A tier placed in the object store needs `storage.object_store` to be
//! bound; this build advertises no object-store adapter, so such a tier
//! is a load error until one lands.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::{ConfigError, Storage};

/// The identifier of the built-in tier every record falls in when the map
/// names nothing else (record §3.5).
pub const DEFAULT_TIER: &str = "default";

/// The retention that keeps records with no expiry.
pub const RETENTION_INDEFINITE: &str = "indefinite";

/// The longest a tier identifier may be.
pub const MAX_TIER_ID_LEN: usize = 64;

/// Where a tier's records are placed (record §3.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    /// The local persistent store.
    Local,
    /// The bound object store (`storage.object_store`).
    ObjectStore,
}

impl Placement {
    /// The wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::ObjectStore => "object_store",
        }
    }
}

/// One tier's policy as written. Every key is optional; an absent key or
/// an explicit `null` takes the built-in default (`local`, `indefinite`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TierPolicy {
    /// Where the tier's records go. Default `local`.
    #[serde(default)]
    pub placement: Option<Placement>,
    /// How long they are kept: `indefinite`, or a whole number of days
    /// written as `<n>d` with `n` at least 1. Default `indefinite`.
    #[serde(default)]
    pub retention: Option<String>,
}

/// One tier's effective policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedTier {
    /// Where the tier's records go.
    pub placement: Placement,
    /// How long they are kept, as written (`indefinite` or `<n>d`).
    pub retention: String,
}

impl ResolvedTier {
    /// The retention in days, or `None` for `indefinite`.
    #[must_use]
    pub fn retention_days(&self) -> Option<u32> {
        match parse_retention(&self.retention) {
            Some(Retention::Days(days)) => Some(days),
            Some(Retention::Indefinite) | None => None,
        }
    }
}

impl TierPolicy {
    fn resolved(&self) -> ResolvedTier {
        ResolvedTier {
            placement: self.placement.unwrap_or(Placement::Local),
            retention: self
                .retention
                .clone()
                .unwrap_or_else(|| RETENTION_INDEFINITE.to_owned()),
        }
    }
}

static BUILTIN_DEFAULT_TIER: LazyLock<ResolvedTier> =
    LazyLock::new(|| TierPolicy::default().resolved());

/// The effective tier map: every tier the document names, plus `default`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ResolvedTiers(pub BTreeMap<String, ResolvedTier>);

impl Default for ResolvedTiers {
    fn default() -> Self {
        resolved(&BTreeMap::new())
    }
}

/// Where a record with a given tier identifier goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TierLookup<'a> {
    /// The map names the tier.
    Mapped(&'a ResolvedTier),
    /// The map does not name the tier: the record is held in `default`
    /// placement and refused for any placement-dependent operation
    /// (record §3.5), never reclassified.
    HeldInDefault(&'a ResolvedTier),
}

impl TierLookup<'_> {
    /// The policy the record is held under.
    #[must_use]
    pub const fn policy(&self) -> &ResolvedTier {
        match self {
            Self::Mapped(policy) | Self::HeldInDefault(policy) => policy,
        }
    }

    /// Whether a placement-dependent operation (moving, expiring or
    /// re-placing the record) may proceed. Only a mapped tier allows it.
    #[must_use]
    pub const fn allows_placement_dependent_operation(&self) -> bool {
        matches!(self, Self::Mapped(_))
    }
}

impl ResolvedTiers {
    /// Whether this is the built-in map, which the canonical encoding
    /// omits so a document without `tiers` keeps its digest.
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The built-in `default` tier's effective policy. Always present.
    #[must_use]
    pub fn default_tier(&self) -> &ResolvedTier {
        // Resolution always inserts the default tier; a map built some
        // other way falls back to the built-in policy.
        self.0.get(DEFAULT_TIER).unwrap_or(&BUILTIN_DEFAULT_TIER)
    }

    /// Where a record carrying `tier` goes. Never reclassifies an unknown
    /// identifier; never answers whether access is allowed.
    #[must_use]
    pub fn lookup(&self, tier: &str) -> TierLookup<'_> {
        self.0.get(tier).map_or_else(
            || TierLookup::HeldInDefault(self.default_tier()),
            TierLookup::Mapped,
        )
    }
}

/// The effective map: each written tier with its defaults filled in, and
/// the built-in `default` tier when the document does not restate it.
pub(crate) fn resolved(tiers: &BTreeMap<String, TierPolicy>) -> ResolvedTiers {
    let mut map: BTreeMap<String, ResolvedTier> = tiers
        .iter()
        .map(|(id, policy)| (id.clone(), policy.resolved()))
        .collect();
    map.entry(DEFAULT_TIER.to_owned())
        .or_insert_with(|| TierPolicy::default().resolved());
    ResolvedTiers(map)
}

/// A parsed retention value.
enum Retention {
    Indefinite,
    Days(u32),
}

/// `indefinite`, or `<n>d` with `n >= 1`; `None` for anything else.
fn parse_retention(value: &str) -> Option<Retention> {
    if value == RETENTION_INDEFINITE {
        return Some(Retention::Indefinite);
    }
    let digits = value.strip_suffix('d')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    match digits.parse::<u32>() {
        Ok(days) if days >= 1 => Some(Retention::Days(days)),
        _ => None,
    }
}

fn is_valid_tier_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_TIER_ID_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Check every tier: its identifier, its retention, and that an
/// object-store placement has an object store to go to.
pub(crate) fn validate(
    tiers: &BTreeMap<String, TierPolicy>,
    storage: &Storage,
) -> Result<(), ConfigError> {
    for (id, policy) in tiers {
        if !is_valid_tier_id(id) {
            return Err(ConfigError::InvalidTierId(id.clone()));
        }
        let tier = policy.resolved();
        if parse_retention(&tier.retention).is_none() {
            return Err(ConfigError::InvalidTierRetention {
                tier: id.clone(),
                retention: tier.retention,
            });
        }
        if tier.placement == Placement::ObjectStore && storage.object_store.is_none() {
            return Err(ConfigError::TierNeedsObjectStore(id.clone()));
        }
    }
    Ok(())
}
