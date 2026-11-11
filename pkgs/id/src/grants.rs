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
//!   publisher-revoked key is flagged (abuse case 3). Uniqueness and
//!   publisher-revocation checks run across the whole storefront
//!   family, not a single namespace.
//! - §1.2 the refund-window snapshot: window parameters are operator
//!   configuration copied onto the grant at registration, never
//!   constants in code (spec §9 open parameters are inputs here, not
//!   defaults). The pending transfer timeout is
//!   `refund_window + posting_grace`.
//! - §7 the grant state machine: every state in the #2042 diagram and
//!   exactly the transitions the spec implements — no orphan states,
//!   no unlisted transitions.
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

/// Fingerprint of a raw presented key: canonicalise, then hash. The
/// raw key must exist only in the caller's memory — it is never
/// stored, logged, or echoed by this module (§2.1).
pub fn fingerprint_of_raw(storefront_id: &str, raw_key: &str) -> String {
    fingerprint(storefront_id, &canonicalise_key(raw_key))
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyState {
    Granted,
    RevokedByPublisher,
    Refunded,
}

/// One append-only registry row (§2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryEntry {
    pub fingerprint: String,
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
    /// The fingerprint is already in the registry under this
    /// storefront family (§2.2 primary-key conflict). The existing
    /// grant is untouched.
    AlreadyGranted { existing: RegistryEntry },
    /// The fingerprint was revoked by its publisher (§2.4) before any
    /// grant existed under this family; presentations are rejected at
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

/// Append-only fingerprint registry (§2.2) over a set of storefront
/// families. In-memory here; the service layer backs the same rules
/// with the `grant_keys` table, whose (family, fingerprint) primary
/// key is the enforcement — this type exists so the rules (family-wide
/// scope, first-wins, state updates without deletion) are testable
/// without storage.
#[derive(Debug, Default)]
pub struct KeyRegistry {
    /// storefront_id -> family_id. Namespaces not registered here form
    /// singleton families of their own id.
    families: HashMap<String, String>,
    /// (family_id, fingerprint) -> row.
    rows: HashMap<(String, String), RegistryEntry>,
}

impl KeyRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare that `storefront_id` belongs to `family_id` (§2.1: a
    /// storefront whose key format changes gets a new namespace in the
    /// same family, and family-wide checks keep old keys recognised).
    pub fn add_to_family(&mut self, storefront_id: &str, family_id: &str) {
        self.families
            .insert(storefront_id.to_string(), family_id.to_string());
    }

    fn family_of(&self, storefront_id: &str) -> String {
        self.families
            .get(storefront_id)
            .cloned()
            .unwrap_or_else(|| storefront_id.to_string())
    }

    /// Look up a fingerprint anywhere in the storefront's family.
    pub fn lookup(&self, storefront_id: &str, fingerprint: &str) -> Option<&RegistryEntry> {
        self.rows
            .get(&(self.family_of(storefront_id), fingerprint.to_string()))
    }

    /// Register a verified presentation (§2.3 step 4a): insert the row
    /// for `(family, fingerprint)`. First registration wins; a conflict
    /// rejects and leaves the existing row and grant untouched.
    pub fn register(
        &mut self,
        storefront_id: &str,
        fingerprint: &str,
        grant_id: &str,
        registering_id: &str,
        registered_at: u64,
    ) -> Result<RegistryEntry, RegisterRejection> {
        let key = (self.family_of(storefront_id), fingerprint.to_string());
        if let Some(existing) = self.rows.get(&key) {
            let existing = existing.clone();
            return Err(match existing.key_state {
                KeyState::RevokedByPublisher => {
                    RegisterRejection::RevokedByPublisher { existing }
                }
                _ => RegisterRejection::AlreadyGranted { existing },
            });
        }
        let entry = RegistryEntry {
            fingerprint: fingerprint.to_string(),
            storefront_id: storefront_id.to_string(),
            grant_id: grant_id.to_string(),
            registering_id: registering_id.to_string(),
            registered_at,
            key_state: KeyState::Granted,
        };
        self.rows.insert(key, entry.clone());
        Ok(entry)
    }

    /// Record a publisher-revocation batch row (§2.4) for a key that
    /// has no grant yet, so future presentations reject at §2.3 step 3.
    /// If a grant row already exists, its state is updated instead —
    /// the caller drives the grant-state effect (§1.4 void or §4
    /// clawback) from the returned previous state.
    pub fn record_publisher_revocation(
        &mut self,
        storefront_id: &str,
        fingerprint: &str,
        registered_at: u64,
    ) -> Option<KeyState> {
        let key = (self.family_of(storefront_id), fingerprint.to_string());
        if let Some(row) = self.rows.get_mut(&key) {
            let previous = row.key_state;
            row.key_state = KeyState::RevokedByPublisher;
            return Some(previous);
        }
        self.rows.insert(
            key,
            RegistryEntry {
                fingerprint: fingerprint.to_string(),
                storefront_id: storefront_id.to_string(),
                grant_id: String::new(),
                registering_id: String::new(),
                registered_at,
                key_state: KeyState::RevokedByPublisher,
            },
        );
        None
    }

    /// Update a row's key state from an attestation (§2.2). Never
    /// deletes. Returns false if the fingerprint is unknown to the
    /// family.
    pub fn set_key_state(
        &mut self,
        storefront_id: &str,
        fingerprint: &str,
        state: KeyState,
    ) -> bool {
        let key = (self.family_of(storefront_id), fingerprint.to_string());
        match self.rows.get_mut(&key) {
            Some(row) => {
                row.key_state = state;
                true
            }
            None => false,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    #[test]
    fn canonicalisation_strips_separators_and_uppercases_ascii() {
        assert_eq!(canonicalise_key("abcde-12345 fghij"), "ABCDE12345FGHIJ");
        assert_eq!(canonicalise_key("  ab\tcd\nef  "), "ABCDEF");
        assert_eq!(canonicalise_key("ABCDE12345FGHIJ"), "ABCDE12345FGHIJ");
        // Already-canonical input is a fixed point.
        let once = canonicalise_key("ab-cd ef");
        assert_eq!(canonicalise_key(&once), once);
    }

    #[test]
    fn fingerprint_matches_spec_vector() {
        // SHA-256("id-grant-key-v1" || 0x00 || "steam" || 0x00 ||
        // "ABCDE12345FGHIJ"), computed independently of this code.
        assert_eq!(
            fingerprint("steam", "ABCDE12345FGHIJ"),
            "106627753c23a3b8a7dd38d7078595d28a65aa7d5fa4d3e5f10527b5317fced7"
        );
        assert_eq!(
            fingerprint_of_raw("steam", "abcde-12345 fghij"),
            "106627753c23a3b8a7dd38d7078595d28a65aa7d5fa4d3e5f10527b5317fced7"
        );
        // Storefront namespace is part of the identity.
        assert_ne!(
            fingerprint("steam", "ABCDE12345FGHIJ"),
            fingerprint("other", "ABCDE12345FGHIJ")
        );
        assert_eq!(
            fingerprint_log_prefix(&fingerprint("steam", "ABCDE12345FGHIJ")),
            "106627753c23"
        );
    }

    #[test]
    fn registry_first_registration_wins() {
        let mut reg = KeyRegistry::new();
        let fp = fingerprint("steam", "ABCDE12345FGHIJ");
        let first = reg
            .register("steam", &fp, "grant-1", "id-alice", 100)
            .expect("first registration succeeds");
        assert_eq!(first.key_state, KeyState::Granted);
        let err = reg
            .register("steam", &fp, "grant-2", "id-bob", 200)
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
        assert_eq!(reg.lookup("steam", &fp).unwrap().grant_id, "grant-1");
    }

    #[test]
    fn registry_uniqueness_spans_the_storefront_family() {
        let mut reg = KeyRegistry::new();
        reg.add_to_family("steam-v1", "steam");
        reg.add_to_family("steam-v2", "steam");
        let fp = fingerprint("steam-v1", "ABCDE12345FGHIJ");
        reg.register("steam-v1", &fp, "grant-1", "id-alice", 100)
            .unwrap();
        // Same fingerprint presented under the family's new namespace
        // is still the same key (§2.1 family rule).
        assert!(reg.lookup("steam-v2", &fp).is_some());
        assert!(reg
            .register("steam-v2", &fp, "grant-2", "id-bob", 200)
            .is_err());
        // A different family is a different universe.
        assert!(reg.lookup("gog", &fp).is_none());
        assert!(reg.register("gog", &fp, "grant-3", "id-bob", 200).is_ok());
    }

    #[test]
    fn re_presentation_of_refunded_or_revoked_key_is_flagged() {
        let mut reg = KeyRegistry::new();
        let fp = fingerprint("steam", "ABCDE12345FGHIJ");
        reg.register("steam", &fp, "grant-1", "id-alice", 100)
            .unwrap();
        assert!(reg.set_key_state("steam", &fp, KeyState::Refunded));
        let err = reg
            .register("steam", &fp, "grant-2", "id-bob", 200)
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
        let err = reg
            .register("steam", &fp, "grant-1", "id-alice", 100)
            .unwrap_err();
        assert!(matches!(
            err,
            RegisterRejection::RevokedByPublisher { .. }
        ));
        assert!(err.is_re_presentation_attempt());
    }

    #[test]
    fn publisher_revocation_updates_existing_grant_row() {
        let mut reg = KeyRegistry::new();
        let fp = fingerprint("steam", "ABCDE12345FGHIJ");
        reg.register("steam", &fp, "grant-1", "id-alice", 100)
            .unwrap();
        let previous = reg.record_publisher_revocation("steam", &fp, 150);
        assert_eq!(previous, Some(KeyState::Granted));
        assert_eq!(
            reg.lookup("steam", &fp).unwrap().key_state,
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
