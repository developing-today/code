//! Partition-scoped world routing guard core (issue #3898, Phase 2 slice 1).
//!
//! Pure decision functions for partition-scoped world lookup
//! `(jurisdiction, name)` with no cross-partition existence oracle.
//! Fail-closed: a wrong-partition lookup resolves exactly like a
//! nonexistent name, a foreign account is refused while the
//! guest-grant seam is disabled, and a shard naming a different
//! jurisdiction than the world is rejected.
//!
//! ## What is here
//!
//! - [`WorldPartitionKey`]: a validated `(jurisdiction, name)` lookup
//!   key. The jurisdiction rule mirrors [`valid_jurisdiction`] in
//!   `hosted_migration` (`[a-z0-9-]`, at most 63 bytes); the name rule
//!   mirrors [`validate_world_name`] in `id-worlds`
//!   (`crates/id-worlds/src/world_hub.rs`).
//! - [`resolve_world`]: partition lookup over a `name -> jurisdiction`
//!   map. Wrong partition and absent name both yield
//!   [`WorldRefusal::NotFound`], so a same-name world in another
//!   partition is indistinguishable from nonexistent (criterion 1).
//! - [`GuestPolicy::Disabled`] with [`check_guest`]: the explicit
//!   guest-grant seam, shipped disabled until the #3397
//!   guest-membership policy is settled. While disabled every grant
//!   state (absent, expired, revoked) is refused, including for
//!   participant-list and membership-metadata reads (criteria 2, 4).
//! - [`check_shard_partition`]: shard/coordinator creation naming a
//!   different jurisdiction than the world's stored jurisdiction is
//!   rejected (criterion 3).
//!
//! ## What is not here
//!
//! Persisting the world's jurisdiction at creation (`host.rs`
//! `WorldRecord` schema change), wiring these checks into the live
//! admission path (`world_hub.rs`, `actor.rs`), and the
//! TypeScript-to-Rust path supplying the caller's authoritative home
//! jurisdiction from the #3399 directory. Those consume this module;
//! this module decides.
//!
//! [`valid_jurisdiction`]: crate::hosted_migration::MAX_JURISDICTION_LEN
//! [`validate_world_name`]: https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-worlds/src/world_hub.rs

use std::collections::BTreeMap;

use thiserror::Error;

/// Longest accepted jurisdiction label; labels are `[a-z0-9-]` and the
/// design's labels (`us`, `eu`, `fedramp`) are far shorter. Mirrors
/// `hosted_migration::MAX_JURISDICTION_LEN`.
pub const MAX_JURISDICTION_LEN: usize = 63;

/// Longest accepted world name in bytes. Mirrors
/// `id-worlds::MAX_WORLD_NAME_BYTES`.
pub const MAX_PARTITION_WORLD_NAME_BYTES: usize = 64;

/// A validated partition-scoped world lookup key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct WorldPartitionKey {
    jurisdiction: String,
    name: String,
}

impl WorldPartitionKey {
    /// Build a key, rejecting malformed labels. Malformed input is the
    /// caller's own string, so a construction error reveals nothing
    /// about stored state.
    pub fn parse(jurisdiction: &str, name: &str) -> Result<Self, PartitionError> {
        if !valid_partition_jurisdiction(jurisdiction) {
            return Err(PartitionError::InvalidJurisdiction(
                jurisdiction.to_string(),
            ));
        }
        if !valid_partition_world_name(name) {
            return Err(PartitionError::InvalidName(name.to_string()));
        }
        Ok(Self {
            jurisdiction: jurisdiction.to_string(),
            name: name.to_string(),
        })
    }

    /// The partition this key addresses.
    pub fn jurisdiction(&self) -> &str {
        &self.jurisdiction
    }

    /// The world name this key addresses.
    pub fn name(&self) -> &str {
        &self.name
    }
}

fn valid_partition_jurisdiction(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= MAX_JURISDICTION_LEN
        && label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn valid_partition_world_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_PARTITION_WORLD_NAME_BYTES
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Construction-time key errors. These describe the caller's own input,
/// never stored state.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PartitionError {
    #[error("invalid jurisdiction label: {0:?}")]
    InvalidJurisdiction(String),
    #[error("invalid world name: {0:?}")]
    InvalidName(String),
}

/// Resolution-time refusal. `NotFound` covers both absent names and
/// wrong-partition lookups: the caller cannot tell them apart.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorldRefusal {
    #[error("world not found")]
    NotFound,
    #[error("shard jurisdiction does not match the world's jurisdiction")]
    PartitionMismatch,
}

/// Resolve `key` against the stored `name -> jurisdiction` map for a
/// caller whose home jurisdiction is `caller_home`.
///
/// - Absent name → [`WorldRefusal::NotFound`].
/// - Present name in a different partition than `key` → `NotFound`
///   (no existence signal leaks across partitions).
/// - Present name in `key`'s partition but `caller_home` elsewhere →
///   `NotFound` (foreign callers learn nothing).
/// - Otherwise → `Ok(())`.
pub fn resolve_world(
    key: &WorldPartitionKey,
    stored: &BTreeMap<String, String>,
    caller_home: &str,
) -> Result<(), WorldRefusal> {
    match stored.get(key.name()) {
        Some(stored_jurisdiction) if stored_jurisdiction == key.jurisdiction() => {}
        _ => return Err(WorldRefusal::NotFound),
    }
    if caller_home == key.jurisdiction() {
        Ok(())
    } else {
        Err(WorldRefusal::NotFound)
    }
}

/// The guest-grant policy. Only `Disabled` exists until the #3397
/// guest-membership policy (open question 1) is settled; enabling the
/// seam is a separate, reversible decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuestPolicy {
    /// No guest grant is honored. Every grant state is refused.
    #[default]
    Disabled,
}

/// A presented guest grant. Liveness (expiry/revocation) is evaluated
/// by the directory layer and passed in as plain facts, so this core
/// stays clock-free.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestGrant {
    /// Whether the grant is past its expiry.
    pub expired: bool,
    /// Whether the grant was revoked.
    pub revoked: bool,
}

/// Check a (possibly absent) guest grant against the policy. While
/// [`GuestPolicy::Disabled`] every input — absent, expired, revoked,
/// or valid — is refused with [`WorldRefusal::NotFound`], so the
/// refusal carries no grant-state signal either.
pub fn check_guest(_policy: GuestPolicy, _grant: Option<&GuestGrant>) -> Result<(), WorldRefusal> {
    Err(WorldRefusal::NotFound)
}

/// Reject shard/coordinator creation naming a different jurisdiction
/// than the world's stored jurisdiction. Creation-time input comes
/// from the creator, so a distinct [`WorldRefusal::PartitionMismatch`]
/// is safe here: it rejects the request, it does not answer a lookup.
pub fn check_shard_partition(
    world_jurisdiction: &str,
    shard_jurisdiction: &str,
) -> Result<(), WorldRefusal> {
    if world_jurisdiction == shard_jurisdiction {
        Ok(())
    } else {
        Err(WorldRefusal::PartitionMismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn stored_pair(world_jurisdiction: &str) -> BTreeMap<String, String> {
        BTreeMap::from([("arena".to_string(), world_jurisdiction.to_string())])
    }

    #[test]
    fn same_name_other_partition_is_indistinguishable_from_nonexistent() {
        let stored = stored_pair("eu");
        // "arena" exists, but in "eu" — asking from "us" must look
        // exactly like asking for a name that exists nowhere.
        let wrong_partition = WorldPartitionKey::parse("us", "arena").unwrap();
        let absent = WorldPartitionKey::parse("us", "nowhere").unwrap();
        assert_eq!(
            resolve_world(&wrong_partition, &stored, "us"),
            resolve_world(&absent, &stored, "us"),
        );
        assert_eq!(
            resolve_world(&wrong_partition, &stored, "us"),
            Err(WorldRefusal::NotFound),
        );
    }

    #[test]
    fn home_partition_caller_resolves() {
        let stored = stored_pair("us");
        let key = WorldPartitionKey::parse("us", "arena").unwrap();
        assert_eq!(resolve_world(&key, &stored, "us"), Ok(()));
    }

    #[test]
    fn foreign_caller_learns_nothing() {
        let stored = stored_pair("us");
        let key = WorldPartitionKey::parse("us", "arena").unwrap();
        assert_eq!(
            resolve_world(&key, &stored, "eu"),
            Err(WorldRefusal::NotFound),
        );
    }

    #[test]
    fn disabled_guest_policy_refuses_every_grant_state() {
        let expired = GuestGrant {
            expired: true,
            revoked: false,
        };
        let revoked = GuestGrant {
            expired: false,
            revoked: true,
        };
        let valid = GuestGrant {
            expired: false,
            revoked: false,
        };
        for grant in [None, Some(&expired), Some(&revoked), Some(&valid)] {
            assert_eq!(
                check_guest(GuestPolicy::Disabled, grant),
                Err(WorldRefusal::NotFound),
                "disabled policy must refuse grant state {grant:?}",
            );
        }
    }

    #[test]
    fn shard_jurisdiction_mismatch_is_rejected() {
        assert_eq!(
            check_shard_partition("us", "eu"),
            Err(WorldRefusal::PartitionMismatch),
        );
        assert_eq!(check_shard_partition("us", "us"), Ok(()));
    }

    #[test]
    fn malformed_labels_are_rejected_at_parse() {
        assert!(WorldPartitionKey::parse("", "arena").is_err());
        assert!(WorldPartitionKey::parse("US", "arena").is_err());
        assert!(WorldPartitionKey::parse("us", "").is_err());
        assert!(WorldPartitionKey::parse("us", "../escape").is_err());
    }

    /// Oracle: the no-existence-signal invariant — for any two distinct
    /// valid jurisdictions W (stored) and H (caller home), resolving a
    /// stored name from the wrong partition must equal resolving an
    /// absent name, and both must be `NotFound`.
    fn valid_label() -> impl Strategy<Value = String> {
        "[a-z0-9-]{1,12}".prop_map(|s: String| {
            if s.bytes().all(|b| b == b'-') {
                format!("a{s}")
            } else {
                s
            }
        })
    }

    proptest! {
        #[test]
        fn wrong_partition_equals_absent(
            stored_j in valid_label(),
            home_j in valid_label(),
        ) {
            prop_assume!(stored_j != home_j);
            let stored = stored_pair(&stored_j);
            let wrong = WorldPartitionKey::parse(&home_j, "arena").unwrap();
            let absent = WorldPartitionKey::parse(&home_j, "ghost").unwrap();
            prop_assert_eq!(
                resolve_world(&wrong, &stored, &home_j),
                resolve_world(&absent, &stored, &home_j),
            );
            prop_assert_eq!(
                resolve_world(&wrong, &stored, &home_j),
                Err(WorldRefusal::NotFound),
            );
        }
    }
}
