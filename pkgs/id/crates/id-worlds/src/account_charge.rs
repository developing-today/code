//! World→account ownership for guest-burn charges (#1067, F12.10
//! per-account quotas; design:
//! `doc/2026-11-23_design_id1067-account-quotas.md`).
//!
//! Every guest call is charged to an account at the
//! [`crate::guest_burn`] funnel. Which account is the ownership
//! question the issue carried through slices 1–2; Drewry decided it
//! on 2026-11-22: **the calling account pays by default**, with an
//! **admin registry** entry and the world's **creator principal**
//! as per-world overrides. Resolution order:
//!
//! 1. an explicit registry entry naming an account wins;
//! 2. a world configured for creator-principal charging charges
//!    its recorded creator — and a world so configured with *no*
//!    recorded creator is an error, never a silent fall-through to
//!    the caller (charging the wrong account quietly is the failure
//!    the rejected directory-derived option would have baked in);
//! 3. otherwise the calling account pays;
//! 4. a call carrying no caller identity at this layer (the actor's
//!    events know participant ids, not accounts — see the design
//!    doc) is charged to the world's own charge key as its account
//!    bucket, visible in the budget's per-account counters.
//!
//! The registry is runtime configuration, like
//! [`crate::world_limits::RuntimeLimits`]: an admin-maintained JSON
//! mapping, installed once by the serve path, never journaled.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// How a world with no explicit registry account is charged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChargeMode {
    /// The calling account pays (the decided default).
    #[default]
    Caller,
    /// The world's recorded creator principal pays.
    Creator,
}

/// One world's entry in the account registry: an explicit account
/// override, a charging-mode opt-in, or both.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldChargeConfig {
    /// The account this world's burns are charged to, overriding
    /// every other resolution step. `None` = no explicit override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// The charging mode used when `account` is `None`.
    pub mode: ChargeMode,
}

/// The admin registry mapping worlds to their charge configuration
/// (option (b) of the ownership decision). World ids are the same
/// charge keys the composition layer installs on each guest.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldAccountRegistry {
    /// Per-world charge configuration, by world id.
    pub worlds: BTreeMap<String, WorldChargeConfig>,
}

impl WorldAccountRegistry {
    /// An empty registry: every world resolves by its local config.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one world's charge configuration.
    pub fn insert(&mut self, world: impl Into<String>, config: WorldChargeConfig) {
        self.worlds.insert(world.into(), config);
    }

    /// One world's registry configuration, if it has an entry.
    pub fn config_for(&self, world: &str) -> Option<&WorldChargeConfig> {
        self.worlds.get(world)
    }

    /// One world's explicit registry account, if its entry names
    /// one (the override that wins resolution outright).
    pub fn account_for(&self, world: &str) -> Option<&str> {
        self.config_for(world)
            .and_then(|config| config.account.as_deref())
    }

    /// Whether the registry names no world at all.
    pub fn is_empty(&self) -> bool {
        self.worlds.is_empty()
    }

    /// Parse a registry from its JSON file shape:
    /// `{ "worlds": { "<world-id>": { "account": "...", "mode": "creator" } } }`.
    ///
    /// # Errors
    ///
    /// The JSON is malformed or names an unknown field or mode.
    pub fn from_json_str(json: &str) -> Result<Self> {
        serde_json::from_str(json).context("parse world account registry")
    }

    /// Load a registry from a JSON file. A missing file is an
    /// error, not an empty registry: an admin who configured a
    /// registry path and typo'd it must not silently lose every
    /// override.
    ///
    /// # Errors
    ///
    /// The file cannot be read, or its contents do not parse.
    pub fn load_json(path: &Path) -> Result<Self> {
        let json = std::fs::read_to_string(path)
            .with_context(|| format!("read world account registry {}", path.display()))?;
        Self::from_json_str(&json)
    }

    /// The process-wide registry every restored guest draws from:
    /// the one the serve path installed, or an empty registry when
    /// it installed none.
    pub fn shared() -> Arc<Self> {
        let slot = Self::slot()
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Arc::clone(&slot.registry)
    }

    /// Install the process-wide registry (the serve path, once, at
    /// startup, before any world opens). The first install wins —
    /// one hub, one registry — and the return value says whether
    /// this call installed it; a later install attempt changes
    /// nothing.
    pub fn install_shared(registry: Self) -> bool {
        let mut slot = Self::slot()
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.installed {
            return false;
        }
        slot.registry = Arc::new(registry);
        slot.installed = true;
        true
    }

    fn slot() -> &'static std::sync::RwLock<RegistrySlot> {
        static SLOT: OnceLock<std::sync::RwLock<RegistrySlot>> = OnceLock::new();
        SLOT.get_or_init(|| {
            std::sync::RwLock::new(RegistrySlot {
                registry: Arc::new(Self::new()),
                installed: false,
            })
        })
    }
}

/// The shared-registry slot: the current registry plus whether the
/// serve path has claimed it yet (see
/// [`WorldAccountRegistry::install_shared`]).
struct RegistrySlot {
    registry: Arc<WorldAccountRegistry>,
    installed: bool,
}

/// A refused world hold against an account's world-count quota
/// (#1067, F12.10).
///
/// Typed and recoverable, like [`FuelRateRefusal`]: the account is
/// unharmed, nothing was created, and the same creation can succeed
/// after the account frees a world or the quota is raised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountWorldRefusal {
    /// The account that asked for one world too many.
    pub account: String,
    /// The account's world-count limit.
    pub limit: u32,
    /// Worlds the account already holds.
    pub held: u32,
}

impl std::fmt::Display for AccountWorldRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "account {} holds {} worlds, at its {}-world quota",
            self.account, self.held, self.limit
        )
    }
}

impl std::error::Error for AccountWorldRefusal {}

/// Per-account world-count ledger (#1067, F12.10).
///
/// Deterministic accounting, no I/O: `try_acquire` records one
/// world's hold under an account, refusing a hold past the limit
/// whole — nothing is recorded and `held_by` does not move — so a
/// refused creation leaves the ledger exactly as it was. Recording
/// a world the account already holds is idempotent (a reopen, not
/// a new hold) and never refused. A `max_worlds` of `0` means no
/// bound, mirroring the per-account burn bound's `0`, so an
/// unconfigured hub runs exactly as before.
///
/// The ledger counts holds the hub attributed, in hub lifetime:
/// worlds that predate the attribution (created before a restart,
/// or through a path that carries no account) are not counted
/// against any account. Persisting ownership across restarts is a
/// later slice; storage-byte accounting stays out of this one.
#[derive(Debug, Clone, Default)]
pub struct AccountWorldQuota {
    max_worlds: u32,
    held: BTreeMap<String, BTreeSet<String>>,
}

impl AccountWorldQuota {
    /// A fresh ledger enforcing `max_worlds` worlds per account.
    #[must_use]
    pub const fn new(max_worlds: u32) -> Self {
        Self {
            max_worlds,
            held: BTreeMap::new(),
        }
    }

    /// The per-account world limit this ledger enforces (`0` = unbounded).
    #[must_use]
    pub const fn max_worlds(&self) -> u32 {
        self.max_worlds
    }

    /// Worlds currently held by `account`.
    #[must_use]
    pub fn held_by(&self, account: &str) -> u32 {
        self.held
            .get(account)
            .map_or(0, |worlds| worlds.len() as u32)
    }

    /// Whether `account` already holds `world`.
    #[must_use]
    pub fn contains(&self, account: &str, world: &str) -> bool {
        self.held
            .get(account)
            .is_some_and(|worlds| worlds.contains(world))
    }

    /// Record `world` under `account`.
    ///
    /// Returns whether the world is newly held: `true` for a new
    /// hold, `false` when the account already held it (a reopen).
    ///
    /// # Errors
    ///
    /// Returns [`AccountWorldRefusal`] — recording nothing — when
    /// the account is already at its limit and does not hold this
    /// world.
    pub fn try_acquire(&mut self, account: &str, world: &str) -> Result<bool, AccountWorldRefusal> {
        if self.contains(account, world) {
            return Ok(false);
        }
        let held = self.held_by(account);
        if self.max_worlds > 0 && held >= self.max_worlds {
            return Err(AccountWorldRefusal {
                account: account.to_owned(),
                limit: self.max_worlds,
                held,
            });
        }
        self.held
            .entry(account.to_owned())
            .or_default()
            .insert(world.to_owned());
        Ok(true)
    }

    /// Forget `world` under `account`, dropping empty accounts.
    /// Saturates: releasing a world the account does not hold
    /// changes nothing.
    pub fn release(&mut self, account: &str, world: &str) {
        if let Some(worlds) = self.held.get_mut(account) {
            worlds.remove(world);
            if worlds.is_empty() {
                self.held.remove(account);
            }
        }
    }
}

/// Why a charge could not be resolved to an account.
#[derive(Debug, thiserror::Error)]
pub enum ChargeResolutionError {
    /// The world is configured for creator-principal charging but
    /// carries no recorded creator. Failing visibly — the call does
    /// not run — is the decided behaviour: falling back to the
    /// caller would charge an account nobody chose for this world.
    #[error(
        "world {world} is configured for creator-principal charging but has no recorded creator"
    )]
    CreatorNotRecorded {
        /// The world's charge key (its world id).
        world: String,
    },
}

/// Resolve the account one guest call is charged to, in the
/// decision's precedence order. `config` is the world's effective
/// charge configuration (its registry entry where it has one, else
/// its locally installed config); `creator` is the world's recorded
/// creator where it carries one; `caller` is the calling account
/// where the composition layer knows it.
///
/// # Errors
///
/// [`ChargeResolutionError::CreatorNotRecorded`] when the world is
/// configured for creator charging and carries no creator.
pub fn resolve_charge_account(
    world: &str,
    config: &WorldChargeConfig,
    creator: Option<&str>,
    caller: Option<&str>,
) -> Result<String, ChargeResolutionError> {
    if let Some(account) = &config.account {
        return Ok(account.clone());
    }
    if config.mode == ChargeMode::Creator {
        return creator.map(str::to_owned).ok_or_else(|| {
            ChargeResolutionError::CreatorNotRecorded {
                world: world.to_owned(),
            }
        });
    }
    Ok(caller.map_or_else(|| world.to_owned(), str::to_owned))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn config(account: Option<&str>, mode: ChargeMode) -> WorldChargeConfig {
        WorldChargeConfig {
            account: account.map(str::to_owned),
            mode,
        }
    }

    #[test]
    fn a_registry_account_wins_over_creator_and_caller() {
        // Oracle (#1067 decision): precedence is registry, then
        // creator-principal configuration, then the caller — an
        // explicit entry beats both other candidates present.
        let resolved = resolve_charge_account(
            "world-1",
            &config(Some("acct-registry"), ChargeMode::Creator),
            Some("acct-creator"),
            Some("acct-caller"),
        )
        .unwrap();
        assert_eq!(resolved, "acct-registry");
    }

    #[test]
    fn creator_mode_charges_the_recorded_creator_not_the_caller() {
        let resolved = resolve_charge_account(
            "world-1",
            &config(None, ChargeMode::Creator),
            Some("acct-creator"),
            Some("acct-caller"),
        )
        .unwrap();
        assert_eq!(resolved, "acct-creator");
    }

    #[test]
    fn creator_mode_without_a_recorded_creator_fails_visibly() {
        // The caller is present and chargeable — the resolution
        // must still refuse rather than quietly charge them.
        let error = resolve_charge_account(
            "world-1",
            &config(None, ChargeMode::Creator),
            None,
            Some("acct-caller"),
        )
        .unwrap_err();
        assert!(
            matches!(error, ChargeResolutionError::CreatorNotRecorded { .. }),
            "expected CreatorNotRecorded, got {error:?}"
        );
        assert!(
            error.to_string().contains("world-1"),
            "the error names the world: {error}"
        );
    }

    #[test]
    fn the_default_charges_the_calling_account() {
        let resolved = resolve_charge_account(
            "world-1",
            &config(None, ChargeMode::Caller),
            Some("acct-creator"),
            Some("acct-caller"),
        )
        .unwrap();
        assert_eq!(resolved, "acct-caller");
    }

    #[test]
    fn a_callerless_call_charges_the_worlds_own_bucket() {
        // No caller identity reaches this layer for actor-internal
        // calls; the charge lands on the world's own key, where the
        // budget's per-account counters show it under the world's
        // name — never on some other account.
        let resolved =
            resolve_charge_account("world-1", &config(None, ChargeMode::Caller), None, None)
                .unwrap();
        assert_eq!(resolved, "world-1");
    }

    #[test]
    fn the_registry_parses_its_json_shape_and_round_trips() {
        let registry = WorldAccountRegistry::from_json_str(
            r#"{
                "worlds": {
                    "world-a": { "account": "acct-admin" },
                    "world-b": { "mode": "creator" },
                    "world-c": { "account": "acct-both", "mode": "creator" }
                }
            }"#,
        )
        .unwrap();
        assert_eq!(registry.account_for("world-a"), Some("acct-admin"));
        assert_eq!(registry.account_for("world-b"), None);
        assert_eq!(
            registry.config_for("world-b").unwrap().mode,
            ChargeMode::Creator
        );
        assert_eq!(registry.account_for("world-c"), Some("acct-both"));
        assert_eq!(registry.config_for("world-missing"), None);

        let reparsed =
            WorldAccountRegistry::from_json_str(&serde_json::to_string(&registry).unwrap())
                .unwrap();
        assert_eq!(reparsed, registry);
    }

    #[test]
    fn a_malformed_registry_is_an_error_not_an_empty_one() {
        assert!(WorldAccountRegistry::from_json_str("{ not json").is_err());
        assert!(
            WorldAccountRegistry::from_json_str(r#"{ "worlds": { "w": { "mode": "bogus" } } }"#)
                .is_err()
        );
    }

    #[test]
    fn an_account_at_its_world_limit_refuses_a_new_world_but_not_a_reopen() {
        // Oracle (#1067 F12.10 world-count slice): an account holding
        // `max_worlds` worlds is refused a further new world, while
        // reopening a world it already holds is idempotent and never
        // refused, and another account is unaffected.
        let mut quota = AccountWorldQuota::new(1);
        assert!(quota.try_acquire("acct-a", "w1").is_ok());
        let refusal = quota.try_acquire("acct-a", "w2").unwrap_err();
        assert_eq!(refusal.account, "acct-a");
        assert_eq!(refusal.limit, 1);
        assert!(quota.try_acquire("acct-a", "w1").is_ok());
        assert!(quota.try_acquire("acct-b", "w2").is_ok());
        assert_eq!(quota.held_by("acct-a"), 1);
    }
}
