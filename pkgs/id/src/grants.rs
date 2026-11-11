//! Grant lifecycle — phase-1 pure core (#2723, spec #2254).
//!
//! Implements the side-effect-free rules of
//! `doc/2026-10-30T11-55-00Z_design_grant_lifecycle.md` that the rest of
//! the grant service is built on:
//!
//! - §2.1 key canonicalisation and the key-fingerprint hash. The
//!   canonical form is the uniqueness identity of a key and is FROZEN
//!   for the life of a storefront family; per-storefront rules are
//!   validation only and may never change it.
//! - §2.2 the fingerprint registry's one-grant rule over an append-only
//!   record set: first registration wins, key state changes never
//!   delete a row, and re-presentation of a refunded or
//!   publisher-revoked key is flagged (abuse case 3).
//! - §1.2 the refund-window snapshot: window parameters are operator
//!   configuration copied onto the grant at registration, never
//!   constants in code (spec §9 open parameters are inputs here, not
//!   defaults). The pending transfer timeout is
//!   `refund_window + posting_grace`.
//! - §7 the grant state machine: every state in the #2042 diagram and
//!   exactly the transitions the spec implements — no orphan states,
//!   no unlisted transitions.
//!
//! Family semantics (§2.1): the fingerprint hash is namespaced by
//! `storefront_id`, so one key has a DIFFERENT fingerprint under each
//! namespace of its storefront family. The §2.2 uniqueness and §2.4
//! publisher-revocation checks run across the whole family, so they
//! operate on the key's fingerprint SET — one fingerprint per family
//! namespace, computed by the caller while it still holds the
//! presented key in memory ([`family_fingerprints`]). A registry row
//! records the full set, so a presentation or a revocation batch under
//! any family namespace matches the row. Family membership must be
//! declared before any registration touches the family ([`KeyRegistry::add_to_family`]
//! freezes it once rows exist), because a set computed before a
//! namespace joined the family cannot contain that namespace's
//! fingerprint.
//!
//! This module is deliberately pure (the `conversion` module pattern):
//! no database, no ledger, no clock. Storage (the `grant_keys` table),
//! storefront verification, and the TigerBeetle pending-transfer calls
//! live in the service layer that consumes these rules; the pending
//! hold itself is a ledger two-phase transfer (§1.1), not a flag in
//! this module.

use std::collections::HashMap;

use sha2::{Digest, Sha256};

/// Domain separator for the fingerprint hash (§2.1). Frozen: there is
/// deliberately no rule version in the hash input.
const FINGERPRINT_DOMAIN: &[u8] = b"id-grant-key-v1";

/// Canonicalise a presented key (§2.1): strip all whitespace and `-`
/// separator characters, uppercase ASCII. Non-ASCII characters are
/// passed through unchanged — per-storefront validation may reject
/// them, but canonicalisation may never alter them into a different
/// identity.
pub fn canonicalise_key(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Fingerprint of a canonical key under a storefront namespace (§2.1):
/// `SHA-256("id-grant-key-v1" || 0x00 || storefront_id || 0x00 ||
/// canonical_key)`, hex-encoded.
pub fn fingerprint(storefront_id: &str, canonical_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(FINGERPRINT_DOMAIN);
    hasher.update([0u8]);
    hasher.update(storefront_id.as_bytes());
    hasher.update([0u8]);
    hasher.update(canonical_key.as_bytes());
    hex_encode(&hasher.finalize())
}

/// Fingerprint of a raw presented key under one namespace:
/// canonicalise, then hash. The raw key must exist only in the
/// caller's memory — it is never stored, logged, or echoed by this
/// module (§2.1).
pub fn fingerprint_of_raw(storefront_id: &str, raw_key: &str) -> String {
    fingerprint(storefront_id, &canonicalise_key(raw_key))
}

/// The fingerprint SET of a raw presented key across a storefront
/// family: one fingerprint per namespace, sorted and deduplicated.
/// This set is the family-wide identity every §2.2/§2.4 check runs
/// against. Computed by the caller from the presented key, while the
/// key is still in memory; the canonical key itself is never stored.
pub fn family_fingerprints(namespaces: &[&str], raw_key: &str) -> Vec<String> {
    let canonical = canonicalise_key(raw_key);
    let mut set: Vec<String> = namespaces
        .iter()
        .map(|ns| fingerprint(ns, &canonical))
        .collect();
    set.sort();
    set.dedup();
    set
}

/// The correlation prefix logs may carry (§2.1): the first 12 hex
/// characters of a fingerprint. Never log more than this.
pub fn fingerprint_log_prefix(fingerprint: &str) -> &str {
    &fingerprint[..fingerprint.len().min(12)]
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Lifecycle state of a registered key (§2.2 `key_state`). Updated by
/// attestations; a row is never deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum KeyState {
    Granted,
    RevokedByPublisher,
    Refunded,
}

/// One append-only registry row (§2.2).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RegistryEntry {
    /// Every fingerprint this key has across its storefront family
    /// (one per namespace, as computed at registration). Any of them
    /// identifies the row.
    pub fingerprints: Vec<String>,
    /// The namespace the key was first registered under. Uniqueness is
    /// enforced across the whole storefront family; this records where
    /// the winning registration happened.
    pub storefront_id: String,
    pub grant_id: String,
    /// The ID that registered first — first registration wins.
    pub registering_id: String,
    /// Registration time, seconds (caller-supplied; this module has no
    /// clock).
    pub registered_at: u64,
    pub key_state: KeyState,
}

/// Why a presentation did not produce a grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterRejection {
    /// The key is already in the registry under this storefront
    /// family (§2.2 primary-key conflict). The existing grant is
    /// untouched.
    AlreadyGranted { existing: RegistryEntry },
    /// The key was revoked by its publisher (§2.4) before any grant
    /// existed under this family; presentations are rejected at
    /// §2.3 step 3.
    RevokedByPublisher { existing: RegistryEntry },
}

impl RegisterRejection {
    /// Abuse case 3: re-presenting a key whose existing row is
    /// `refunded` or `revoked_by_publisher` is a resale /
    /// re-presentation attempt and is flagged against the presenting
    /// ID. A plain duplicate of a live grant is not.
    pub fn is_re_presentation_attempt(&self) -> bool {
        let existing = match self {
            RegisterRejection::AlreadyGranted { existing } => existing,
            RegisterRejection::RevokedByPublisher { existing } => existing,
        };
        matches!(
            existing.key_state,
            KeyState::Refunded | KeyState::RevokedByPublisher
        )
    }
}

/// Family membership cannot change once it could orphan a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyFrozen {
    pub storefront_id: String,
    pub family_id: String,
}

/// Append-only fingerprint registry (§2.2). In-memory here; the
/// service layer backs the same rules with the `grant_keys` table,
/// whose primary key is the enforcement — this type exists so the
/// rules (family-wide scope, first-wins, state updates without
/// deletion) are testable without storage.
#[derive(Debug, Default)]
pub struct KeyRegistry {
    /// storefront_id -> family_id. Namespaces not declared here form
    /// singleton families of their own id.
    families: HashMap<String, String>,
    /// Append-only rows; never removed or reordered.
    rows: Vec<RegistryEntry>,
    /// Any fingerprint from a row's set -> index into `rows`.
    by_fingerprint: HashMap<String, usize>,
}

impl KeyRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare that `storefront_id` belongs to `family_id` (§2.1: a
    /// storefront whose key format changes gets a new namespace in the
    /// same family, and family-wide checks keep old keys recognised).
    ///
    /// Membership freezes once it could orphan a row: this fails if
    /// the namespace was already assigned to a different family, if
    /// any row was registered under the namespace, or if any row
    /// exists under the target family (a fingerprint set computed
    /// before this namespace joined cannot name it). Declare families
    /// from configuration at startup, before registrations.
    pub fn add_to_family(
        &mut self,
        storefront_id: &str,
        family_id: &str,
    ) -> Result<(), FamilyFrozen> {
        let frozen = || FamilyFrozen {
            storefront_id: storefront_id.to_string(),
            family_id: family_id.to_string(),
        };
        if let Some(existing) = self.families.get(storefront_id) {
            if existing != family_id {
                return Err(frozen());
            }
            return Ok(());
        }
        let family_touched = self.rows.iter().any(|row| {
            row.storefront_id == storefront_id
                || self.families.get(&row.storefront_id).map(String::as_str)
                    == Some(family_id)
                || row.storefront_id == family_id
        });
        if family_touched {
            return Err(frozen());
        }
        self.families
            .insert(storefront_id.to_string(), family_id.to_string());
        Ok(())
    }

    /// The namespaces of the family `storefront_id` belongs to
    /// (including itself), for computing [`family_fingerprints`].
    /// Sorted for determinism.
    pub fn family_namespaces(&self, storefront_id: &str) -> Vec<String> {
        let family = self
            .families
            .get(storefront_id)
            .cloned()
            .unwrap_or_else(|| storefront_id.to_string());
        let mut namespaces: Vec<String> = self
            .families
            .iter()
            .filter(|(_, fam)| **fam == family)
            .map(|(ns, _)| ns.clone())
            .collect();
        if !namespaces.iter().any(|ns| ns == storefront_id) {
            namespaces.push(storefront_id.to_string());
        }
        namespaces.sort();
        namespaces
    }

    /// Look up a row by ANY fingerprint from a key's family set.
    pub fn lookup(&self, fingerprint: &str) -> Option<&RegistryEntry> {
        self.by_fingerprint
            .get(fingerprint)
            .map(|idx| &self.rows[*idx])
    }

    /// Register a verified presentation (§2.3 step 4a).
    /// `fingerprints` is the key's full family set ([`family_fingerprints`]).
    /// First registration wins: if ANY fingerprint in the set already
    /// identifies a row, the presentation rejects and the existing row
    /// and grant are untouched.
    pub fn register(
        &mut self,
        storefront_id: &str,
        fingerprints: &[String],
        grant_id: &str,
        registering_id: &str,
        registered_at: u64,
    ) -> Result<RegistryEntry, RegisterRejection> {
        for fp in fingerprints {
            if let Some(existing) = self.lookup(fp) {
                let existing = existing.clone();
                return Err(match existing.key_state {
                    KeyState::RevokedByPublisher => {
                        RegisterRejection::RevokedByPublisher { existing }
                    }
                    _ => RegisterRejection::AlreadyGranted { existing },
                });
            }
        }
        let entry = RegistryEntry {
            fingerprints: fingerprints.to_vec(),
            storefront_id: storefront_id.to_string(),
            grant_id: grant_id.to_string(),
            registering_id: registering_id.to_string(),
            registered_at,
            key_state: KeyState::Granted,
        };
        let idx = self.rows.len();
        for fp in fingerprints {
            self.by_fingerprint.insert(fp.clone(), idx);
        }
        self.rows.push(entry.clone());
        Ok(entry)
    }

    /// Record one fingerprint from a publisher-revocation batch
    /// (§2.4). If the fingerprint already identifies a row, the row's
    /// state is updated and the previous state returned — the caller
    /// drives the grant-state effect (§1.4 void or §4 clawback) from
    /// it. Otherwise a tombstone row is inserted so future
    /// presentations reject at §2.3 step 3, and `None` is returned.
    /// (`storefront_id` is the batch document's namespace.)
    pub fn record_publisher_revocation(
        &mut self,
        storefront_id: &str,
        fingerprint: &str,
        registered_at: u64,
    ) -> Option<KeyState> {
        if let Some(idx) = self.by_fingerprint.get(fingerprint).copied() {
            let row = &mut self.rows[idx];
            let previous = row.key_state;
            row.key_state = KeyState::RevokedByPublisher;
            return Some(previous);
        }
        let idx = self.rows.len();
        self.rows.push(RegistryEntry {
            fingerprints: vec![fingerprint.to_string()],
            storefront_id: storefront_id.to_string(),
            grant_id: String::new(),
            registering_id: String::new(),
            registered_at,
            key_state: KeyState::RevokedByPublisher,
        });
        self.by_fingerprint.insert(fingerprint.to_string(), idx);
        None
    }

    /// Update a row's key state from an attestation (§2.2), addressed
    /// by any fingerprint from the key's family set. Never deletes.
    /// Returns false if the fingerprint is unknown.
    pub fn set_key_state(&mut self, fingerprint: &str, state: KeyState) -> bool {
        match self.by_fingerprint.get(fingerprint).copied() {
            Some(idx) => {
                self.rows[idx].key_state = state;
                true
            }
            None => false,
        }
    }

    /// Snapshot the registry for persistence (families + rows, in
    /// registration order). The service layer's store snapshot is
    /// built on this; the fingerprint index is derived state and is
    /// rebuilt by [`KeyRegistry::restore`], never stored.
    pub fn snapshot(&self) -> (HashMap<String, String>, Vec<RegistryEntry>) {
        (self.families.clone(), self.rows.clone())
    }

    /// Rebuild a registry from a [`KeyRegistry::snapshot`] pair,
    /// re-deriving the fingerprint index from the rows. Rows keep
    /// their registration order, so first-wins state is identical
    /// to the snapshotted registry.
    pub fn restore(families: HashMap<String, String>, rows: Vec<RegistryEntry>) -> Self {
        let mut by_fingerprint = HashMap::new();
        for (idx, row) in rows.iter().enumerate() {
            for fp in &row.fingerprints {
                by_fingerprint.insert(fp.clone(), idx);
            }
        }
        Self {
            families,
            rows,
            by_fingerprint,
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// Refund-window parameters snapshot (§1.2): copied onto the grant
/// record at registration, so later configuration changes never move
/// an existing grant's window. Values are operator configuration —
/// this type carries them, it does not choose them (§9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WindowSnapshot {
    /// PendingHold length for the grant's source, seconds.
    pub refund_window_secs: u64,
    /// Time after window end during which the service must post the
    /// pending transfer before its ledger timeout voids it.
    pub posting_grace_secs: u64,
}

impl WindowSnapshot {
    /// The pending transfer's ledger `timeout` (§1.2):
    /// `refund_window + posting_grace`, seconds, set at creation.
    pub fn pending_timeout_secs(&self) -> u64 {
        self.refund_window_secs + self.posting_grace_secs
    }
}

/// Grant states (§7). Every state in the #2042 diagram has exactly
/// one implementing mechanism in the spec; the transition table in
/// [`GrantState::can_transition`] is the whole machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GrantState {
    /// §2.3 registration in flight.
    Presented,
    /// §1: verified, issued as a ledger pending transfer.
    PendingHold,
    /// §1.3: window lapsed clean, hold posted.
    Active,
    /// §2.2/§2.3: already granted, or publisher-revoked before grant.
    Rejected,
    /// §1.4 void, or §4.2 completed clawback.
    Revoked,
    /// §4.2: for-cause notice landed, clawback in flight.
    ClawbackPending,
    /// §4.2/§4.3: balance already spent; partial clawback + debt.
    Shortfall,
    /// §4.3: shortfall debt offset or written off.
    Settled,
    /// §7: outstanding figure reached zero by consumption.
    Spent,
    /// §7: grant terms expired (a still-pending grant voids into this
    /// state — it was never spendable).
    Expired,
}

impl GrantState {
    /// Terminal states have no outgoing transitions; attestations
    /// arriving against them are recorded as no-ops (§3.2).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            GrantState::Rejected
                | GrantState::Revoked
                | GrantState::Settled
                | GrantState::Spent
                | GrantState::Expired
        )
    }

    /// The §7 transition table — the only legal moves. Anything not
    /// listed here does not exist.
    pub fn can_transition(&self, to: GrantState) -> bool {
        use GrantState::*;
        matches!(
            (self, to),
            (Presented, Rejected)
                | (Presented, PendingHold)
                | (PendingHold, Active)
                | (PendingHold, Revoked)
                | (PendingHold, Expired)
                | (Active, Spent)
                | (Active, ClawbackPending)
                | (Active, Expired)
                | (ClawbackPending, Revoked)
                | (ClawbackPending, Shortfall)
                | (Shortfall, Settled)
        )
    }

    /// Checked transition: returns the new state or the rejected pair.
    pub fn transition(&self, to: GrantState) -> Result<GrantState, (GrantState, GrantState)> {
        if self.can_transition(to) {
            Ok(to)
        } else {
            Err((*self, to))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEAM_KEY: &str = "ABCDE12345FGHIJ";

    #[test]
    fn canonicalisation_strips_separators_and_uppercases_ascii() {
        assert_eq!(canonicalise_key("abcde-12345 fghij"), STEAM_KEY);
        assert_eq!(canonicalise_key("  ab\tcd\nef  "), "ABCDEF");
        assert_eq!(canonicalise_key(STEAM_KEY), STEAM_KEY);
        // Already-canonical input is a fixed point.
        let once = canonicalise_key("ab-cd ef");
        assert_eq!(canonicalise_key(&once), once);
    }

    #[test]
    fn fingerprint_matches_spec_vector() {
        // SHA-256("id-grant-key-v1" || 0x00 || "steam" || 0x00 ||
        // "ABCDE12345FGHIJ"), computed independently of this code.
        assert_eq!(
            fingerprint("steam", STEAM_KEY),
            "106627753c23a3b8a7dd38d7078595d28a65aa7d5fa4d3e5f10527b5317fced7"
        );
        assert_eq!(
            fingerprint_of_raw("steam", "abcde-12345 fghij"),
            "106627753c23a3b8a7dd38d7078595d28a65aa7d5fa4d3e5f10527b5317fced7"
        );
        // Storefront namespace is part of the identity.
        assert_ne!(fingerprint("steam", STEAM_KEY), fingerprint("other", STEAM_KEY));
        assert_eq!(
            fingerprint_log_prefix(&fingerprint("steam", STEAM_KEY)),
            "106627753c23"
        );
    }

    #[test]
    fn registry_first_registration_wins() {
        let mut reg = KeyRegistry::new();
        let set = family_fingerprints(&["steam"], STEAM_KEY);
        let first = reg
            .register("steam", &set, "grant-1", "id-alice", 100)
            .expect("first registration succeeds");
        assert_eq!(first.key_state, KeyState::Granted);
        let err = reg
            .register("steam", &set, "grant-2", "id-bob", 200)
            .expect_err("second presentation of the same key rejects");
        match &err {
            RegisterRejection::AlreadyGranted { existing } => {
                assert_eq!(existing.grant_id, "grant-1");
                assert_eq!(existing.registering_id, "id-alice");
            }
            other => panic!("expected AlreadyGranted, got {other:?}"),
        }
        // A duplicate of a live grant is not an abuse flag, and the
        // existing row is untouched.
        assert!(!err.is_re_presentation_attempt());
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.lookup(&set[0]).unwrap().grant_id, "grant-1");
    }

    #[test]
    fn family_uniqueness_uses_namespace_derived_fingerprints() {
        // Regression test for the family rule (§2.1): the same raw key
        // hashes DIFFERENTLY under each namespace of a family, and the
        // one-grant rule must still hold across them — via the
        // fingerprint set, never via a shared hash.
        let mut reg = KeyRegistry::new();
        reg.add_to_family("steam-v1", "steam").unwrap();
        reg.add_to_family("steam-v2", "steam").unwrap();
        let raw = "abcde-12345 fghij";
        let namespaces = reg.family_namespaces("steam-v1");
        let set = family_fingerprints(
            &namespaces.iter().map(String::as_str).collect::<Vec<_>>(),
            raw,
        );
        assert_eq!(set.len(), 2, "one fingerprint per family namespace");
        assert_ne!(
            fingerprint_of_raw("steam-v1", raw),
            fingerprint_of_raw("steam-v2", raw)
        );
        reg.register("steam-v1", &set, "grant-1", "id-alice", 100)
            .unwrap();
        // A caller who only knows the v2 namespace still finds the row
        // through the v2 fingerprint alone.
        let v2_only = vec![fingerprint_of_raw("steam-v2", raw)];
        assert!(reg.lookup(&v2_only[0]).is_some());
        assert!(reg.register("steam-v2", &v2_only, "grant-2", "id-bob", 200).is_err());
        // A different family is a different universe.
        let gog = family_fingerprints(&["gog"], raw);
        assert!(reg.lookup(&gog[0]).is_none());
        assert!(reg.register("gog", &gog, "grant-3", "id-bob", 200).is_ok());
    }

    #[test]
    fn family_membership_freezes_once_rows_exist() {
        let mut reg = KeyRegistry::new();
        let set = family_fingerprints(&["steam-v1"], STEAM_KEY);
        reg.register("steam-v1", &set, "grant-1", "id-alice", 100)
            .unwrap();
        // Joining the family after a registration would orphan the
        // row's fingerprint set — refused for the namespace that owns
        // the rows (and therefore for the family it would join).
        assert!(reg.add_to_family("steam-v1", "steam").is_err());
    }

    #[test]
    fn re_presentation_of_refunded_or_revoked_key_is_flagged() {
        let mut reg = KeyRegistry::new();
        let set = family_fingerprints(&["steam"], STEAM_KEY);
        reg.register("steam", &set, "grant-1", "id-alice", 100)
            .unwrap();
        assert!(reg.set_key_state(&set[0], KeyState::Refunded));
        let err = reg
            .register("steam", &set, "grant-2", "id-bob", 200)
            .unwrap_err();
        assert!(err.is_re_presentation_attempt());
        // The row was updated, never deleted (§2.2).
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn publisher_revoked_key_rejects_before_any_grant() {
        let mut reg = KeyRegistry::new();
        let fp = fingerprint("steam", "ZZZZZ99999YYYYY");
        assert_eq!(
            reg.record_publisher_revocation("steam", &fp, 50),
            None,
            "no prior grant row"
        );
        let set = family_fingerprints(&["steam"], "ZZZZZ99999YYYYY");
        let err = reg
            .register("steam", &set, "grant-1", "id-alice", 100)
            .unwrap_err();
        assert!(matches!(err, RegisterRejection::RevokedByPublisher { .. }));
        assert!(err.is_re_presentation_attempt());
    }

    #[test]
    fn publisher_revocation_under_one_namespace_blocks_the_family() {
        // A §2.4 batch names fingerprints under the namespace the
        // publisher used; a presentation under a sibling namespace of
        // the same family must still reject.
        let mut reg = KeyRegistry::new();
        reg.add_to_family("steam-v1", "steam").unwrap();
        reg.add_to_family("steam-v2", "steam").unwrap();
        let raw = "abcde-12345 fghij";
        let namespaces = reg.family_namespaces("steam-v1");
        let set = family_fingerprints(
            &namespaces.iter().map(String::as_str).collect::<Vec<_>>(),
            raw,
        );
        reg.register("steam-v1", &set, "grant-1", "id-alice", 100)
            .unwrap();
        let batch_fp = fingerprint_of_raw("steam-v2", raw);
        let previous = reg.record_publisher_revocation("steam-v2", &batch_fp, 150);
        assert_eq!(previous, Some(KeyState::Granted));
        assert_eq!(
            reg.lookup(&fingerprint_of_raw("steam-v1", raw))
                .unwrap()
                .key_state,
            KeyState::RevokedByPublisher
        );
    }

    #[test]
    fn window_timeout_is_window_plus_grace() {
        let snap = WindowSnapshot {
            refund_window_secs: 14 * 24 * 60 * 60,
            posting_grace_secs: 3600,
        };
        assert_eq!(snap.pending_timeout_secs(), 14 * 24 * 60 * 60 + 3600);
    }

    #[test]
    fn state_machine_implements_exactly_the_spec_table() {
        use GrantState::*;
        let legal = [
            (Presented, Rejected),
            (Presented, PendingHold),
            (PendingHold, Active),
            (PendingHold, Revoked),
            (PendingHold, Expired),
            (Active, Spent),
            (Active, ClawbackPending),
            (Active, Expired),
            (ClawbackPending, Revoked),
            (ClawbackPending, Shortfall),
            (Shortfall, Settled),
        ];
        for (from, to) in legal {
            assert!(from.can_transition(to), "{from:?} -> {to:?} must be legal");
            assert_eq!(from.transition(to), Ok(to));
        }
        // The §0 rule: nothing skips PendingHold.
        assert!(!Presented.can_transition(Active));
        // No resurrection, no skipping the clawback choreography.
        assert!(!Revoked.can_transition(Active));
        assert!(!Active.can_transition(Revoked));
        assert!(!Active.can_transition(Settled));
        assert!(!PendingHold.can_transition(Spent));
        for terminal in [Rejected, Revoked, Settled, Spent, Expired] {
            assert!(terminal.is_terminal());
            for to in [Presented, PendingHold, Active, Settled] {
                assert!(
                    !terminal.can_transition(to),
                    "terminal {terminal:?} must have no outgoing transitions"
                );
            }
        }
    }
}

#[cfg(test)]
mod property_tests {
    //! Property/model coverage per the repo testing doctrine
    //! (AGENTS.md): canonicalisation and fingerprinting are ID-format
    //! code, the registry is checked against a first-wins append-only
    //! MODEL, and the state machine against its own table as oracle —
    //! generators hammer the invariants, examples alone do not.

    use super::*;
    use proptest::prelude::*;
    use std::collections::HashMap as ModelMap;

    /// Key-shaped strings: mixed case, digits, separators, whitespace.
    fn key_like() -> impl Strategy<Value = String> {
        proptest::collection::vec(
            prop_oneof![
                proptest::char::range('a', 'z'),
                proptest::char::range('A', 'Z'),
                proptest::char::range('0', '9'),
                Just('-'),
                Just(' '),
                Just('\t'),
            ],
            0..24,
        )
        .prop_map(|chars| chars.into_iter().collect())
    }

    proptest! {
        /// Canonicalisation is idempotent, and two presentations that
        /// differ only in separators/whitespace/case are the same key:
        /// same canonical form, same fingerprint, in every namespace.
        #[test]
        fn canonicalisation_idempotent_and_presentation_invariant(raw in key_like()) {
            let canonical = canonicalise_key(&raw);
            prop_assert_eq!(canonicalise_key(&canonical), canonical.clone());
            prop_assert_eq!(
                fingerprint_of_raw("steam", &raw),
                fingerprint("steam", &canonical)
            );
            // Canonical form contains no separators or lowercase ASCII.
            prop_assert!(canonical.chars().all(|c| c != '-' && !c.is_whitespace() && !c.is_ascii_lowercase()));
        }

        /// Registry model test: random operation sequences over a
        /// small key universe, checked against a first-registration-
        /// wins, append-only model. Oracle: the model.
        #[test]
        fn registry_matches_first_wins_append_only_model(
            ops in proptest::collection::vec((0usize..3, 0u8..3), 1..40)
        ) {
            const KEYS: [&str; 3] = ["AAAAA11111", "BBBBB22222", "CCCCC33333"];
            const NAMESPACES: [&str; 2] = ["steam-v1", "steam-v2"];
            let mut reg = KeyRegistry::new();
            reg.add_to_family("steam-v1", "steam").unwrap();
            reg.add_to_family("steam-v2", "steam").unwrap();
            let namespaces = reg.family_namespaces("steam-v1");
            let ns_refs: Vec<&str> = namespaces.iter().map(String::as_str).collect();

            // Model: canonical key -> (grant_id, key_state). Rows are
            // never removed; first registration wins.
            let mut model: ModelMap<&str, (String, KeyState)> = ModelMap::new();
            let mut grants = 0usize;
            for (key_idx, op) in ops {
                let key = KEYS[key_idx];
                let set = family_fingerprints(&ns_refs, key);
                match op {
                    // Present the key (under a rotating namespace).
                    0 => {
                        let ns = NAMESPACES[key_idx % NAMESPACES.len()];
                        let grant_id = format!("grant-{grants}");
                        match reg.register(ns, &set, &grant_id, "id-x", 100) {
                            Ok(_) => {
                                prop_assert!(!model.contains_key(key));
                                model.insert(key, (grant_id, KeyState::Granted));
                                grants += 1;
                            }
                            Err(rejection) => {
                                let (model_grant, model_state) = model
                                    .get(key)
                                    .expect("registry rejected a key the model has never seen");
                                let (existing, is_revoked_variant) = match &rejection {
                                    RegisterRejection::AlreadyGranted { existing } => {
                                        (existing, false)
                                    }
                                    RegisterRejection::RevokedByPublisher { existing } => {
                                        (existing, true)
                                    }
                                };
                                prop_assert_eq!(&existing.grant_id, model_grant);
                                prop_assert_eq!(existing.key_state, *model_state);
                                // The RevokedByPublisher variant is used
                                // exactly when the existing row is in
                                // that state.
                                prop_assert_eq!(
                                    is_revoked_variant,
                                    *model_state == KeyState::RevokedByPublisher
                                );
                            }
                        }
                    }
                    // Attestation: refund.
                    1 => {
                        let address = fingerprint_of_raw(NAMESPACES[0], key);
                        let known = reg.set_key_state(&address, KeyState::Refunded);
                        prop_assert_eq!(known, model.contains_key(key));
                        if let Some(entry) = model.get_mut(key) {
                            entry.1 = KeyState::Refunded;
                        }
                    }
                    // Publisher revocation via one namespace's fingerprint.
                    _ => {
                        let single = fingerprint_of_raw(NAMESPACES[0], key);
                        let previous = reg.record_publisher_revocation(NAMESPACES[0], &single, 100);
                        match model.get_mut(key) {
                            Some(entry) => {
                                prop_assert_eq!(previous, Some(entry.1));
                                entry.1 = KeyState::RevokedByPublisher;
                            }
                            None => {
                                prop_assert_eq!(previous, None);
                                model.insert(key, (String::new(), KeyState::RevokedByPublisher));
                            }
                        }
                    }
                }
                // Append-only: row count always equals the model's
                // distinct-key count, and every model row matches.
                prop_assert_eq!(reg.len(), model.len());
                for (key, (grant_id, state)) in &model {
                    let address = fingerprint_of_raw(NAMESPACES[0], key);
                    let row = reg.lookup(&address).expect("model row missing from registry");
                    prop_assert_eq!(&row.grant_id, grant_id);
                    prop_assert_eq!(row.key_state, *state);
                }
            }
        }

        /// State machine: random transition attempts from any state —
        /// an attempt succeeds IFF the §7 table lists it (the table is
        /// the oracle), and terminal states are absorbing.
        #[test]
        fn state_machine_accepts_exactly_the_table(
            from_idx in 0usize..10,
            to_idx in 0usize..10,
        ) {
            const STATES: [GrantState; 10] = [
                GrantState::Presented,
                GrantState::PendingHold,
                GrantState::Active,
                GrantState::Rejected,
                GrantState::Revoked,
                GrantState::ClawbackPending,
                GrantState::Shortfall,
                GrantState::Settled,
                GrantState::Spent,
                GrantState::Expired,
            ];
            let (from, to) = (STATES[from_idx], STATES[to_idx]);
            let table = [
                (GrantState::Presented, GrantState::Rejected),
                (GrantState::Presented, GrantState::PendingHold),
                (GrantState::PendingHold, GrantState::Active),
                (GrantState::PendingHold, GrantState::Revoked),
                (GrantState::PendingHold, GrantState::Expired),
                (GrantState::Active, GrantState::Spent),
                (GrantState::Active, GrantState::ClawbackPending),
                (GrantState::Active, GrantState::Expired),
                (GrantState::ClawbackPending, GrantState::Revoked),
                (GrantState::ClawbackPending, GrantState::Shortfall),
                (GrantState::Shortfall, GrantState::Settled),
            ];
            prop_assert_eq!(from.can_transition(to), table.contains(&(from, to)));
            if from.is_terminal() {
                prop_assert!(!from.can_transition(to));
            }
        }
    }
}
