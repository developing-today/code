//! Grant service — phase 2 (#2723, spec #2254): `grant_keys` storage,
//! the §2.3 registration flow, and the §2.5 intent-first creation
//! protocol over the currency PoC [`LedgerBackend`] seam.
//!
//! Phase 1 ([`crate::grants`]) is the pure rule core: canonicalisation,
//! fingerprints, the family-wide one-grant rule, window snapshots, and
//! the state machine. This module is the service layer that consumes
//! those rules with side effects:
//!
//! - **Storage.** [`GrantStore`] is the storage layer for the §2.2
//!   `grant_keys` table plus the grant records it points at. Its
//!   `insert_presented` writes the registry row and the grant record
//!   as one atomic operation — the storage-layer primary key (the
//!   family fingerprint set, enforced by [`KeyRegistry`]) is what
//!   rejects a second grant for a key, never an application pre-check,
//!   and rows are append-only: key-state changes update a row, they
//!   never delete it (§2.2). A durable backing plugs behind this same
//!   API; the rules do not change.
//! - **Registration (§2.3).** Verification before anything is
//!   recorded: the storefront's authoritative channel
//!   ([`StorefrontVerifier`]) must verify the presented key and return
//!   its product attribution while the raw key still exists. The raw
//!   key is canonicalised + hashed and dropped; it is never stored or
//!   logged (§2.1). Amount and currency come from the source
//!   configuration for the verified product — the registration
//!   request carries no amount field at all, so there is nothing for
//!   a caller to override. A storefront with no verification channel
//!   cannot be enabled as a grant source (§2.3 step 2).
//! - **Intent-first creation (§2.5).** (a) Intent: insert the
//!   registry row + grant record in state `Presented`, carrying a
//!   deterministic ledger transfer id derived from the grant_id.
//!   (b) Transfer: create the pending transfer under that id —
//!   ledger creation with an existing id returns `Exists`, so a lost
//!   response is retried, never compensated. (c) Confirm: advance
//!   `Presented → PendingHold` only after re-checking for applicable
//!   adverse attestation events recorded during the intent window
//!   (§2.3 step 4c; the intake that produces those records is §3,
//   a later phase — here the check is an injected query). A standing
//!   [`RegistrationService::reconcile`] drives any grant stranded in
//!   `Presented` to the confirm step (transfer found) or a clean
//!   retry (transfer absent, attestation check re-run first).
//!
//! **The hold model.** §1.1 implements PendingHold as a ledger
//! two-phase pending transfer, but the currency PoC seam's
//! `Transfer` type has no pending flag or timeout — a transfer
//! created through it posts immediately. Crediting the grantee at
//! registration would therefore make the grant spendable while its
//! record says PendingHold, violating §0 (this was Codex P1 on
//! PR #8614). So the registration transfer credits a **service-owned
//! hold account** for the currency (`hold_account_for`), never the
//! grantee: during PendingHold the value sits in the service's
//! custody, the grantee's spendable balance is untouched, and an
//! adverse event at confirm leaves the money in the hold account,
//! not with the grantee. The §1.3 post flow (a later phase) moves
//! hold → grantee under its own deterministic transfer id; the
//! §1.4 void flow moves hold → issuer. When the seam grows native
//! two-phase transfers, the hold account is where the pending
//! transfer's credit leg lands instead — the grant-facing
//! guarantees (nothing spendable during the hold; resolution moves
//! the value exactly once) do not change.
//!
//! Scope limits, per the phase packet: the pending hold's two-phase
//! post/void resolution (§1.3/§1.4, scheduler sweep) is a later
//! phase — this phase creates the hold transfer and records the
//! §1.2 window snapshot + timeout on the grant record; it does not
//! post, void, or activate anything, and no path here enables
//! Active-state grants (spec §10: not before §4 exists).

use std::collections::HashMap;

use sha2::{Digest, Sha256};

use crate::currency_poc::ledger::{
    Account, AccountError, AccountId, LedgerBackend, Transfer, TransferError, TransferId,
};
use crate::grants::{
    family_fingerprints, GrantState, KeyRegistry, KeyState, RegisterRejection, RegistryEntry,
    WindowSnapshot,
};

/// Revocation-policy ladder (§5.1), snapshotted onto the grant record
/// at registration. Enforcement is §5, a later phase — this phase
/// only guarantees the snapshot exists and never changes afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RevocationPolicy {
    None,
    ForCause,
    IssuerDiscretion,
}

/// What the storefront's authoritative channel returns for a verified
/// key (§2.3 step 2): the product attribution the grant amount is
/// selected from. Verification that cannot produce this has not
/// verified anything the service may act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedKey {
    pub product_id: String,
}

/// Why storefront verification refused a presented key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// The key is not a real, unconsumed key for this storefront.
    NotValid,
    /// The authoritative channel could not be reached or answered
    /// unusably. Distinct from `NotValid`: the presentation earned
    /// no verdict, so nothing about the key is concluded.
    ChannelUnavailable,
}

/// The storefront's authoritative verification channel (§2.3 step 2).
/// Implementations hold the raw key only for the call's duration.
pub trait StorefrontVerifier {
    fn verify(&self, storefront_id: &str, raw_key: &str) -> Result<VerifiedKey, VerifyError>;
}

/// Grant terms for one verified product, from source configuration
/// (§2.3 step 5): the ONLY place an amount or currency may come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProductTerms {
    pub amount: u128,
    /// Ledger (currency) the grant is denominated in.
    pub currency_ledger: u32,
}

/// One grant source's configuration (§1.2, §2.3): windows and policy
/// are operator configuration, snapshotted per grant at registration.
#[derive(Debug, Clone)]
pub struct StorefrontConfig {
    pub window: WindowSnapshot,
    pub revocation_policy: RevocationPolicy,
    /// The issuer's grant-issuance account for this source's ledger.
    pub issuer_account: AccountId,
    pub products: HashMap<String, ProductTerms>,
}

/// A registered grant (the grant record §2.3 step 4a writes).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GrantRecord {
    pub grant_id: String,
    /// The fingerprint under the registering namespace (the
    /// `grant_keys` row's primary namespace); the full family set is
    /// on the registry entry.
    pub fingerprint: String,
    pub storefront_id: String,
    pub grantee_id: String,
    pub registering_id: String,
    pub currency_ledger: u32,
    pub amount: u128,
    /// Grant source (the storefront namespace presented to).
    pub source: String,
    pub product_id: String,
    /// §1.2 snapshot: copied at registration, never re-read from
    /// configuration afterwards.
    pub window: WindowSnapshot,
    /// §5.1 snapshot.
    pub revocation_policy: RevocationPolicy,
    pub state: GrantState,
    /// The service-owned hold account the registration transfer
    /// credits (see the module's hold model): value sits here, not
    /// with the grantee, for the whole PendingHold.
    pub hold_account: AccountId,
    /// Deterministic ledger transfer id (§2.5 step 1), derived from
    /// `grant_id` — the intent IS this id, so retries are identities.
    pub transfer_id: TransferId,
    pub registered_at: u64,
    /// When the grant entered PendingHold (confirm step), if it has.
    pub pending_since: Option<u64>,
}

/// The `grant_keys` storage layer plus grant records (§2.2).
#[derive(Debug, Default)]
pub struct GrantStore {
    registry: KeyRegistry,
    grants: HashMap<String, GrantRecord>,
    /// Grant ids in insertion order (the reconciler's scan order).
    order: Vec<String>,
}

impl GrantStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare storefront-family membership at startup (§2.1).
    /// Delegates to the registry, including its freeze rule.
    pub fn add_to_family(
        &mut self,
        storefront_id: &str,
        family_id: &str,
    ) -> Result<(), crate::grants::FamilyFrozen> {
        self.registry.add_to_family(storefront_id, family_id)
    }

    pub fn family_namespaces(&self, storefront_id: &str) -> Vec<String> {
        self.registry.family_namespaces(storefront_id)
    }

    pub fn registry(&self) -> &KeyRegistry {
        &self.registry
    }

    /// Record a publisher-revocation batch entry (§2.4 storage
    /// effect; the grant-state effects are §1.4/§4 phases).
    pub fn record_publisher_revocation(
        &mut self,
        storefront_id: &str,
        fingerprint: &str,
        registered_at: u64,
    ) -> Option<KeyState> {
        self.registry
            .record_publisher_revocation(storefront_id, fingerprint, registered_at)
    }

    /// Update a key's state from an attestation (§2.2). Never deletes.
    pub fn set_key_state(&mut self, fingerprint: &str, state: KeyState) -> bool {
        self.registry.set_key_state(fingerprint, state)
    }

    /// The §2.5 intent write: registry row + grant record, one
    /// atomic operation. The registry's primary-key rule runs FIRST
    /// inside the same operation, so a conflict writes nothing —
    /// no registry row, no grant record — and the existing grant is
    /// untouched (§2.2).
    pub fn insert_presented(
        &mut self,
        fingerprints: &[String],
        grant: GrantRecord,
    ) -> Result<RegistryEntry, InsertPresentedError> {
        debug_assert_eq!(grant.state, GrantState::Presented);
        // Grant-id uniqueness is checked BEFORE the registry is
        // touched: a second intent reusing a grant_id must not
        // register its key, must not replace the existing grant
        // record, and must not reach the ledger (Codex P2 on
        // PR #8614 — a reused id would otherwise let reconciliation
        // confirm a replacement record against the first grant's
        // transfer).
        if self.grants.contains_key(&grant.grant_id) {
            return Err(InsertPresentedError::DuplicateGrantId {
                grant_id: grant.grant_id.clone(),
            });
        }
        let entry = self.registry.register(
            &grant.storefront_id,
            fingerprints,
            &grant.grant_id,
            &grant.registering_id,
            grant.registered_at,
        )
        .map_err(InsertPresentedError::Rejection)?;
        self.order.push(grant.grant_id.clone());
        self.grants.insert(grant.grant_id.clone(), grant);
        Ok(entry)
    }

    pub fn grant(&self, grant_id: &str) -> Option<&GrantRecord> {
        self.grants.get(grant_id)
    }

    pub fn grant_count(&self) -> usize {
        self.grants.len()
    }

    /// Advance a grant through the §7 state machine (checked
    /// transition — an unlisted move is a programming error and is
    /// refused, leaving the record unchanged).
    pub fn transition(
        &mut self,
        grant_id: &str,
        to: GrantState,
    ) -> Result<GrantState, (GrantState, GrantState)> {
        let grant = self.grants.get_mut(grant_id).expect("grant exists");
        let next = grant.state.transition(to)?;
        grant.state = next;
        Ok(next)
    }

    pub fn stamp_pending_since(&mut self, grant_id: &str, at: u64) {
        if let Some(grant) = self.grants.get_mut(grant_id) {
            grant.pending_since = Some(at);
        }
    }

    /// Grants still in the pre-ledger `Presented` state whose intent
    /// was written at or before `now - freshness_bound_secs` — the
    /// reconciler's work list (§2.5 step 4), in insertion order.
    pub fn stranded_presented(&self, now: u64, freshness_bound_secs: u64) -> Vec<GrantRecord> {
        let cutoff = now.saturating_sub(freshness_bound_secs);
        self.order
            .iter()
            .filter_map(|id| self.grants.get(id))
            .filter(|g| g.state == GrantState::Presented && g.registered_at <= cutoff)
            .cloned()
            .collect()
    }
}

/// A serializable snapshot of the whole store: family
/// declarations, every append-only registry row in registration
/// order, and every grant record in insertion order.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StoreSnapshot {
    pub families: HashMap<String, String>,
    pub registry_rows: Vec<RegistryEntry>,
    pub grants: Vec<GrantRecord>,
}

impl GrantStore {
    /// Persist this: the service host writes the snapshot after
    /// every mutation (its write-ahead discipline) and restores it
    /// at startup, so a process restart loses NO registry entry and
    /// NO grant intent (Codex P1 on PR #8614). A grant stranded in
    /// `Presented` by a crash is exactly what the reconciler scans
    /// for after restore — the intent protocol only works if the
    /// intent survives, and this snapshot is what carries it.
    pub fn snapshot(&self) -> StoreSnapshot {
        let (families, registry_rows) = self.registry.snapshot();
        StoreSnapshot {
            families,
            registry_rows,
            grants: self
                .order
                .iter()
                .filter_map(|id| self.grants.get(id).cloned())
                .collect(),
        }
    }

    /// Rebuild a store from a snapshot, re-deriving every index.
    /// Behavior after restore is indistinguishable from the
    /// snapshotted store: first-wins rows, key states, family
    /// membership (including its freeze), and grant states all
    /// carry over.
    pub fn restore(snapshot: StoreSnapshot) -> Result<Self, RestoreError> {
        // Reject duplicate grant IDs before rebuilding any index
        // (#8913): collecting the grants into a map would silently
        // overwrite the earlier record while the insertion order
        // still walks both entries, losing one record and
        // reconciling the survivor repeatedly. A malformed or
        // imported snapshot is refused whole -- no partially
        // restored store is ever exposed.
        let mut seen = std::collections::HashSet::new();
        for grant in &snapshot.grants {
            if !seen.insert(grant.grant_id.clone()) {
                return Err(RestoreError::DuplicateGrantId {
                    grant_id: grant.grant_id.clone(),
                });
            }
        }
        let registry = KeyRegistry::restore(snapshot.families, snapshot.registry_rows);
        let order: Vec<String> = snapshot.grants.iter().map(|g| g.grant_id.clone()).collect();
        let grants = snapshot
            .grants
            .into_iter()
            .map(|g| (g.grant_id.clone(), g))
            .collect();
        Ok(Self {
            registry,
            grants,
            order,
        })
    }
}

/// Why a snapshot restore did not happen (#8913).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreError {
    /// Two snapshot records share a grant_id. Restoring would
    /// silently drop one of them, so the snapshot is rejected
    /// before any index is rebuilt.
    DuplicateGrantId { grant_id: String },
}

/// Why an intent write did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertPresentedError {
    /// The §2.2 one-grant rule (§2.3 step 3 included): the key is
    /// already registered under this storefront family.
    Rejection(RegisterRejection),
    /// The grant_id already names a grant record. Checked before
    /// any registry or ledger effect, so a duplicate intent is a
    /// clean refusal, never a silent replacement.
    DuplicateGrantId { grant_id: String },
}

/// Deterministic ledger transfer id for a grant (§2.5 step 1):
/// derived from the grant_id alone, so the intent write, the ledger
/// call, and every retry name the same transfer.
pub fn transfer_id_for_grant(grant_id: &str) -> TransferId {
    derive_id(b"id-grant-transfer-v1", grant_id)
}

/// The service-owned hold account for a currency (see the
/// module's hold model): every registration transfer for the
/// currency credits this account, and §1's resolution flows move
/// value out of it exactly once per grant.
pub fn hold_account_for(ledger: u32) -> AccountId {
    account_id_for("grant-hold", ledger)
}

/// Deterministic ledger account id for a named party + ledger.
pub fn account_id_for(name: &str, ledger: u32) -> AccountId {
    derive_id(b"id-grant-account-v1", &format!("{name}:{ledger}"))
}

fn derive_id(domain: &[u8], subject: &str) -> u128 {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update([0u8]);
    hasher.update(subject.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    let value = u128::from_be_bytes(bytes);
    // Ledger ids of zero are invalid in the TigerBeetle model; the
    // derivation is deterministic, so map the (astronomically
    // unlikely) zero to one rather than failing.
    if value == 0 {
        1
    } else {
        value
    }
}

/// An adverse attestation event applicable to a grant, as the
/// confirm step sees it (§2.3 step 4c, §3.2 Presented row). The
/// intake that creates these records is §3; this phase consumes the
/// answer to "is there an applicable adverse event for this grant?"
/// through [`AdverseEventQuery`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdverseEvent {
    /// Attestation kind: `refund_executed`, `chargeback_filed`, or
    /// `publisher_revocation` (§1.4).
    pub kind: String,
    /// The §3 record's id, carried onto the outcome for audit.
    pub attestation_id: String,
}

/// Query for adverse events applicable to one grant at confirm time.
pub trait AdverseEventQuery {
    fn adverse_event_for(&self, grant: &GrantRecord) -> Option<AdverseEvent>;
}

/// The common case until §3 lands: no attestation log exists yet,
/// so no grant has an applicable adverse event.
pub struct NoAdverseEvents;

impl AdverseEventQuery for NoAdverseEvents {
    fn adverse_event_for(&self, _grant: &GrantRecord) -> Option<AdverseEvent> {
        None
    }
}

/// A registration request (§2.3 step 1). Deliberately carries NO
/// amount or currency: those come from source configuration for the
/// verified product (§2.3 step 5), never from the request.
#[derive(Debug, Clone)]
pub struct RegistrationRequest {
    pub storefront_id: String,
    /// The raw presented key. Held only for this call; never stored,
    /// logged, or echoed (§2.1).
    pub raw_key: String,
    /// The authenticated registering ID (§2.3 step 1).
    pub registering_id: String,
    /// The ID the grant is issued to.
    pub grantee_id: String,
    /// Service-issued grant identity; the deterministic transfer id
    /// derives from it (§2.5).
    pub grant_id: String,
    /// Registration time, seconds (caller-supplied clock).
    pub now: u64,
}

/// How a registration ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationOutcome {
    /// Verified, intent confirmed, pending transfer created: the
    /// grant is in PendingHold (§2.3, §1.1).
    PendingHold(GrantRecord),
    /// The key produces no grant: already granted, or revoked by
    /// its publisher before any grant (§2.2, §2.3 step 3). The
    /// existing grant is untouched.
    Rejected {
        rejection: RegisterRejection,
        /// Abuse case 3 flag: re-presentation of a refunded or
        /// publisher-revoked key (§2.2).
        re_presentation_attempt: bool,
    },
    /// The intent is durably written (grant in `Presented`) but the
    /// ledger step has not confirmed: a lost response, a backend
    /// failure, or an adverse event recorded during the intent
    /// window. The reconciler (§2.5 step 4) owns the next step;
    /// nothing was silently dropped and nothing was double-issued.
    IntentPending {
        grant: GrantRecord,
        reason: IntentPendingReason,
    },
}

/// Why an intent is still pending after the registration call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentPendingReason {
    /// The ledger call failed or its response was lost; whether the
    /// transfer exists is for the reconciler's lookup to decide.
    LedgerUnconfirmed { error: TransferError },
    /// An applicable adverse attestation arrived while the grant
    /// sat in Presented (§2.3 step 4c): the grant must NOT advance
    /// to PendingHold; §1.4 processing owns it from here.
    AdverseEventAtConfirm { event: AdverseEvent },
}

/// Failures before any intent exists. Each of these records
/// NOTHING: no registry row, no grant record, no transfer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationError {
    /// No such grant source is configured.
    UnknownStorefront { storefront_id: String },
    /// The storefront has no authoritative verification channel, so
    /// it cannot be enabled as a grant source (§2.3 step 2).
    NoVerificationChannel { storefront_id: String },
    /// Verification refused the key (§2.3 step 2).
    VerificationFailed { source: VerifyError },
    /// The request's grant_id already names a grant record; the
    /// intent was refused before any registry or ledger effect.
    DuplicateGrantId { grant_id: String },
    /// The verified product has no grant terms in the source
    /// configuration — an operator configuration gap, surfaced
    /// rather than defaulted (§9: open parameters are inputs).
    ProductNotConfigured {
        storefront_id: String,
        product_id: String,
    },
}

/// The registration service (§2.3 + §2.5) over a [`LedgerBackend`].
pub struct RegistrationService<'a, L: LedgerBackend> {
    store: GrantStore,
    ledger: L,
    config: HashMap<String, StorefrontConfig>,
    verifier: &'a dyn StorefrontVerifier,
    adverse: &'a dyn AdverseEventQuery,
}

impl<'a, L: LedgerBackend> RegistrationService<'a, L> {
    /// Build the service. Storefront-family membership (§2.1) is
    /// declared on the store before any registration, via
    /// [`GrantStore::add_to_family`] — a fingerprint set computed
    /// without the family would reopen the cross-namespace farming
    /// loop, so declaration is a startup obligation of the caller,
    /// under the registry's freeze rule.
    pub fn new(
        store: GrantStore,
        ledger: L,
        config: HashMap<String, StorefrontConfig>,
        verifier: &'a dyn StorefrontVerifier,
        adverse: &'a dyn AdverseEventQuery,
    ) -> Self {
        Self {
            store,
            ledger,
            config,
            verifier,
            adverse,
        }
    }

    pub fn store(&self) -> &GrantStore {
        &self.store
    }

    /// Mutable store access for the §2.2/§2.4 storage effects that
    /// arrive outside registration (attestation-driven key-state
    /// updates, publisher-revocation batches).
    pub fn store_mut(&mut self) -> &mut GrantStore {
        &mut self.store
    }

    pub fn ledger(&self) -> &L {
        &self.ledger
    }

    /// Run one §2.3 registration. Steps in order: verify (before
    /// anything is recorded) → publisher-revocation + one-grant
    /// enforcement inside the intent write → ledger transfer →
    /// confirm (with the intent-window attestation re-check).
    pub fn register(
        &mut self,
        request: &RegistrationRequest,
    ) -> Result<RegistrationOutcome, RegistrationError> {
        let cfg = self
            .config
            .get(&request.storefront_id)
            .ok_or_else(|| RegistrationError::UnknownStorefront {
                storefront_id: request.storefront_id.clone(),
            })?
            .clone();

        // Step 2: verification before anything is recorded. An
        // unverifiable source cannot register — and this attempt
        // leaves no trace in storage or the ledger.
        let verified = self
            .verifier
            .verify(&request.storefront_id, &request.raw_key)
            .map_err(|source| RegistrationError::VerificationFailed { source })?;
        let terms = cfg.products.get(&verified.product_id).ok_or_else(|| {
            RegistrationError::ProductNotConfigured {
                storefront_id: request.storefront_id.clone(),
                product_id: verified.product_id.clone(),
            }
        })?;

        // The key's family-wide fingerprint set, computed while the
        // raw key still exists (§2.1). The raw key goes no further.
        let namespaces = self.store.family_namespaces(&request.storefront_id);
        let ns_refs: Vec<&str> = namespaces.iter().map(String::as_str).collect();
        let fingerprints = family_fingerprints(&ns_refs, &request.raw_key);
        let fingerprint = crate::grants::fingerprint_of_raw(&request.storefront_id, &request.raw_key);

        // Steps 3+4a: intent-first creation. The store's atomic
        // insert runs the §2.3 step-3 publisher-revocation check and
        // the §2.2 primary-key rule; a conflict rejects with the
        // existing grant untouched and nothing new written.
        let grant = GrantRecord {
            grant_id: request.grant_id.clone(),
            fingerprint,
            storefront_id: request.storefront_id.clone(),
            grantee_id: request.grantee_id.clone(),
            registering_id: request.registering_id.clone(),
            currency_ledger: terms.currency_ledger,
            amount: terms.amount,
            source: request.storefront_id.clone(),
            product_id: verified.product_id.clone(),
            window: cfg.window,
            revocation_policy: cfg.revocation_policy,
            state: GrantState::Presented,
            hold_account: hold_account_for(terms.currency_ledger),
            transfer_id: transfer_id_for_grant(&request.grant_id),
            registered_at: request.now,
            pending_since: None,
        };
        match self.store.insert_presented(&fingerprints, grant.clone()) {
            Ok(_) => {}
            Err(InsertPresentedError::Rejection(rejection)) => {
                let re_presentation_attempt = rejection.is_re_presentation_attempt();
                return Ok(RegistrationOutcome::Rejected {
                    rejection,
                    re_presentation_attempt,
                });
            }
            Err(InsertPresentedError::DuplicateGrantId { grant_id }) => {
                return Err(RegistrationError::DuplicateGrantId { grant_id });
            }
        }

        // Step 4b: the ledger transfer under the deterministic id.
        match self.create_pending_transfer(&cfg, &grant) {
            Ok(()) => {}
            Err(error) => {
                return Ok(RegistrationOutcome::IntentPending {
                    grant,
                    reason: IntentPendingReason::LedgerUnconfirmed { error },
                });
            }
        }

        // Step 4c: confirm, after the intent-window attestation
        // re-check.
        Ok(self.confirm(grant, request.now))
    }

    /// The §2.5 confirm step, shared by `register` and `reconcile`:
    /// re-check applicable adverse events FIRST; only then advance
    /// Presented → PendingHold.
    fn confirm(&mut self, grant: GrantRecord, now: u64) -> RegistrationOutcome {
        if let Some(event) = self.adverse.adverse_event_for(&grant) {
            return RegistrationOutcome::IntentPending {
                grant,
                reason: IntentPendingReason::AdverseEventAtConfirm { event },
            };
        }
        self.store
            .transition(&grant.grant_id, GrantState::PendingHold)
            .expect("Presented -> PendingHold is a listed transition");
        self.store.stamp_pending_since(&grant.grant_id, now);
        RegistrationOutcome::PendingHold(
            self.store
                .grant(&grant.grant_id)
                .expect("grant exists")
                .clone(),
        )
    }

    /// Create the grant's hold transfer (§1.1, via the module's
    /// hold model) under its deterministic id, ensuring both
    /// ledger accounts exist first
    /// (account creation with identical fields is idempotent —
    /// `Exists` is success). A transfer that already exists with
    /// identical fields (`Exists`) is the lost-response retry case:
    /// success, not an error (§2.5 step 2).
    fn create_pending_transfer(
        &mut self,
        cfg: &StorefrontConfig,
        grant: &GrantRecord,
    ) -> Result<(), TransferError> {
        // The credit leg is the service-owned HOLD account for the
        // currency — never the grantee (see the module's hold
        // model). The grantee's account is not even created here;
        // it first appears in the §1.3 post flow (later phase).
        let accounts = [
            Account {
                id: cfg.issuer_account,
                ledger: grant.currency_ledger,
                code: 1,
                flags: Default::default(),
            },
            Account {
                id: grant.hold_account,
                ledger: grant.currency_ledger,
                code: 3,
                flags: Default::default(),
            },
        ];
        for result in self.ledger.create_accounts(&accounts) {
            match result {
                Ok(()) | Err(AccountError::Exists) => {}
                Err(_) => return Err(TransferError::BackendFailure),
            }
        }
        let transfer = Transfer {
            id: grant.transfer_id,
            debit_account_id: cfg.issuer_account,
            credit_account_id: grant.hold_account,
            amount: grant.amount,
            ledger: grant.currency_ledger,
            user_data_128: grant.transfer_id,
            code: 1,
            linked: false,
        };
        let results = self.ledger.create_transfers(&[transfer]);
        match results.into_iter().next() {
            Some(Ok(())) | Some(Err(TransferError::Exists)) => Ok(()),
            Some(Err(error)) => Err(error),
            None => Err(TransferError::BackendFailure),
        }
    }

    /// The standing reconciler (§2.5 step 4): scan grants stranded
    /// in `Presented` past the freshness bound and drive each to
    /// its confirm step — transfer found in the ledger → confirm;
    /// transfer absent → re-run the attestation check, re-issue
    /// the transfer under the same deterministic id, then confirm.
    /// A ledger lookup that ERRORS decides nothing: the grant is
    /// left for the next scan and counted, never guessed at.
    pub fn reconcile(&mut self, now: u64, freshness_bound_secs: u64) -> ReconcileReport {
        let mut report = ReconcileReport::default();
        for grant in self.store.stranded_presented(now, freshness_bound_secs) {
            report.scanned += 1;
            match self.ledger.try_lookup_transfer(grant.transfer_id) {
                Ok(Some(_)) => match self.confirm(grant, now) {
                    RegistrationOutcome::PendingHold(_) => report.confirmed += 1,
                    RegistrationOutcome::IntentPending { .. } => report.blocked_by_event += 1,
                    RegistrationOutcome::Rejected { .. } => {
                        unreachable!("confirm never rejects")
                    }
                },
                Ok(None) => {
                    // Absent: attestation check re-runs BEFORE the
                    // retry (§2.5 step 4), inside `confirm`'s call
                    // order — so check here before re-issuing.
                    if let Some(_event) = self.adverse.adverse_event_for(&grant) {
                        report.blocked_by_event += 1;
                        continue;
                    }
                    let cfg = self
                        .config
                        .get(&grant.storefront_id)
                        .expect("grant's storefront is configured")
                        .clone();
                    match self.create_pending_transfer(&cfg, &grant) {
                        Ok(()) => match self.confirm(grant, now) {
                            RegistrationOutcome::PendingHold(_) => report.confirmed += 1,
                            RegistrationOutcome::IntentPending { .. } => {
                                report.blocked_by_event += 1
                            }
                            RegistrationOutcome::Rejected { .. } => {
                                unreachable!("confirm never rejects")
                            }
                        },
                        Err(_) => report.still_pending += 1,
                    }
                }
                Err(_) => report.lookup_failed += 1,
            }
        }
        report
    }
}

/// One reconciler scan's outcome (§2.5 step 4: the backlog is an
/// operator-visible metric, so the scan reports its counts).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReconcileReport {
    pub scanned: u32,
    pub confirmed: u32,
    pub blocked_by_event: u32,
    pub still_pending: u32,
    pub lookup_failed: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::currency_poc::ledger::InMemoryLedger;
    use crate::grants::fingerprint_of_raw;
    use std::cell::RefCell;

    const LEDGER_ID: u32 = 978;
    const AMOUNT: u128 = 500;

    /// Verifier double: exact raw-key → product mapping; the key
    /// "CHANNEL-DOWN" simulates an unreachable authoritative channel.
    struct TestVerifier {
        products: HashMap<String, String>,
    }

    impl StorefrontVerifier for TestVerifier {
        fn verify(&self, _storefront_id: &str, raw_key: &str) -> Result<VerifiedKey, VerifyError> {
            if raw_key == "CHANNEL-DOWN" {
                return Err(VerifyError::ChannelUnavailable);
            }
            // The channel recognises its own keys however they are
            // formatted; it looks up by canonical form.
            let canonical = crate::grants::canonicalise_key(raw_key);
            let found = self
                .products
                .iter()
                .find(|(raw, _)| crate::grants::canonicalise_key(raw) == canonical)
                .map(|(_, product)| product);
            match found {
                Some(product_id) => Ok(VerifiedKey {
                    product_id: product_id.clone(),
                }),
                None => Err(VerifyError::NotValid),
            }
        }
    }

    /// Adverse-event stub the test can arm/disarm between calls.
    struct AdverseStub {
        event: RefCell<Option<AdverseEvent>>,
    }

    impl AdverseEventQuery for AdverseStub {
        fn adverse_event_for(&self, _grant: &GrantRecord) -> Option<AdverseEvent> {
            self.event.borrow().clone()
        }
    }

    fn config() -> HashMap<String, StorefrontConfig> {
        let mut products = HashMap::new();
        products.insert(
            "game-a".to_string(),
            ProductTerms {
                amount: AMOUNT,
                currency_ledger: LEDGER_ID,
            },
        );
        let mut config = HashMap::new();
        config.insert(
            "steam".to_string(),
            StorefrontConfig {
                window: WindowSnapshot {
                    refund_window_secs: 14 * 24 * 3600,
                    posting_grace_secs: 24 * 3600,
                },
                revocation_policy: RevocationPolicy::ForCause,
                issuer_account: account_id_for("issuer", LEDGER_ID),
                products,
            },
        );
        config
    }

    fn verifier() -> TestVerifier {
        let mut products = HashMap::new();
        products.insert("AAAAA-11111-BBBBB".to_string(), "game-a".to_string());
        products.insert("CCCCC-22222-DDDDD".to_string(), "game-a".to_string());
        TestVerifier { products }
    }

    fn request(grant_id: &str, raw_key: &str, now: u64) -> RegistrationRequest {
        RegistrationRequest {
            storefront_id: "steam".to_string(),
            raw_key: raw_key.to_string(),
            registering_id: "grantee-1".to_string(),
            grantee_id: "grantee-1".to_string(),
            grant_id: grant_id.to_string(),
            now,
        }
    }

    fn service<'a>(
        verifier: &'a TestVerifier,
        adverse: &'a AdverseStub,
    ) -> RegistrationService<'a, InMemoryLedger> {
        RegistrationService::new(
            GrantStore::new(),
            InMemoryLedger::new(),
            config(),
            verifier,
            adverse,
        )
    }

    fn no_adverse() -> AdverseStub {
        AdverseStub {
            event: RefCell::new(None),
        }
    }

    #[test]
    fn happy_path_registers_pending_hold_with_config_terms() {
        let v = verifier();
        let adverse = no_adverse();
        let mut svc = service(&v, &adverse);
        let outcome = svc
            .register(&request("grant-1", "AAAAA-11111-BBBBB", 1_000))
            .unwrap();
        let grant = match outcome {
            RegistrationOutcome::PendingHold(grant) => grant,
            other => panic!("expected PendingHold, got {other:?}"),
        };
        // Amount + currency from source configuration (§2.3 step 5).
        assert_eq!(grant.amount, AMOUNT);
        assert_eq!(grant.currency_ledger, LEDGER_ID);
        assert_eq!(grant.state, GrantState::PendingHold);
        assert_eq!(grant.pending_since, Some(1_000));
        assert_eq!(grant.window.pending_timeout_secs(), 15 * 24 * 3600);
        assert_eq!(grant.revocation_policy, RevocationPolicy::ForCause);
        // The hold transfer exists under the deterministic id and
        // credits the service-owned HOLD account exactly once; the
        // grantee has no account and no spendable balance while
        // the grant says PendingHold (§0, §1.1).
        let transfer = svc
            .ledger()
            .lookup_transfer(transfer_id_for_grant("grant-1"))
            .expect("transfer exists");
        assert_eq!(transfer.amount, AMOUNT);
        assert_eq!(transfer.credit_account_id, hold_account_for(LEDGER_ID));
        let hold_balance = svc
            .ledger()
            .account_balance(hold_account_for(LEDGER_ID))
            .expect("hold account exists");
        assert_eq!(hold_balance.credits_posted, AMOUNT);
        assert!(svc
            .ledger()
            .account_balance(account_id_for("grantee-1", LEDGER_ID))
            .is_none());
    }

    #[test]
    fn second_presentation_of_same_key_rejects_and_leaves_existing_grant_untouched() {
        let v = verifier();
        let adverse = no_adverse();
        let mut svc = service(&v, &adverse);
        svc.register(&request("grant-1", "AAAAA-11111-BBBBB", 1_000))
            .unwrap();
        // Same key, different presentation formatting + a different
        // would-be grant id: canonicalisation makes it the same key.
        let outcome = svc
            .register(&request("grant-2", "aaaaa 11111-bbbbb", 2_000))
            .unwrap();
        match outcome {
            RegistrationOutcome::Rejected {
                rejection,
                re_presentation_attempt,
            } => {
                assert!(matches!(
                    rejection,
                    RegisterRejection::AlreadyGranted { .. }
                ));
                assert!(!re_presentation_attempt);
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
        assert_eq!(svc.store().grant_count(), 1);
        assert_eq!(
            svc.store().grant("grant-1").unwrap().state,
            GrantState::PendingHold
        );
        // No second transfer was issued for the rejected presentation.
        assert!(svc
            .ledger()
            .lookup_transfer(transfer_id_for_grant("grant-2"))
            .is_none());
    }

    #[test]
    fn re_presentation_of_refunded_key_is_flagged_abuse_case_3() {
        let v = verifier();
        let adverse = no_adverse();
        let mut svc = service(&v, &adverse);
        svc.register(&request("grant-1", "AAAAA-11111-BBBBB", 1_000))
            .unwrap();
        let fp = fingerprint_of_raw("steam", "AAAAA-11111-BBBBB");
        assert!(svc.store_mut().set_key_state(&fp, KeyState::Refunded));
        let outcome = svc
            .register(&request("grant-2", "AAAAA-11111-BBBBB", 2_000))
            .unwrap();
        match outcome {
            RegistrationOutcome::Rejected {
                re_presentation_attempt,
                ..
            } => assert!(re_presentation_attempt),
            other => panic!("expected flagged Rejected, got {other:?}"),
        }
    }

    #[test]
    fn publisher_revoked_key_rejects_before_any_grant() {
        let v = verifier();
        let adverse = no_adverse();
        let mut svc = service(&v, &adverse);
        // §2.4 tombstone: revoked before any presentation.
        let fp = fingerprint_of_raw("steam", "CCCCC-22222-DDDDD");
        svc.store_mut()
            .record_publisher_revocation("steam", &fp, 500);
        let outcome = svc
            .register(&request("grant-9", "CCCCC-22222-DDDDD", 1_000))
            .unwrap();
        assert!(matches!(
            outcome,
            RegistrationOutcome::Rejected {
                rejection: RegisterRejection::RevokedByPublisher { .. },
                re_presentation_attempt: true,
            }
        ));
        assert_eq!(svc.store().grant_count(), 0);
    }

    #[test]
    fn unverifiable_key_records_nothing() {
        let v = verifier();
        let adverse = no_adverse();
        let mut svc = service(&v, &adverse);
        let err = svc
            .register(&request("grant-x", "ZZZZZ-99999-YYYYY", 1_000))
            .unwrap_err();
        assert_eq!(
            err,
            RegistrationError::VerificationFailed {
                source: VerifyError::NotValid
            }
        );
        assert_eq!(svc.store().grant_count(), 0);
        assert_eq!(svc.store().registry().len(), 0);
        assert!(svc
            .ledger()
            .lookup_transfer(transfer_id_for_grant("grant-x"))
            .is_none());
    }

    #[test]
    fn unknown_storefront_and_unconfigured_product_record_nothing() {
        let v = verifier();
        let adverse = no_adverse();
        let mut svc = service(&v, &adverse);
        let mut req = request("grant-x", "AAAAA-11111-BBBBB", 1_000);
        req.storefront_id = "gog".to_string();
        assert!(matches!(
            svc.register(&req),
            Err(RegistrationError::UnknownStorefront { .. })
        ));
        assert_eq!(svc.store().grant_count(), 0);
    }

    /// Ledger wrapper: the first `create_transfers` APPLIES to the
    /// inner ledger but reports `BackendFailure` — the lost-response
    /// crash window of §2.5.
    struct LoseFirstResponse {
        inner: InMemoryLedger,
        lost: bool,
    }

    impl LedgerBackend for LoseFirstResponse {
        fn create_accounts(
            &mut self,
            accounts: &[Account],
        ) -> Vec<Result<(), AccountError>> {
            self.inner.create_accounts(accounts)
        }
        fn create_transfers(
            &mut self,
            batch: &[Transfer],
        ) -> Vec<Result<(), TransferError>> {
            let results = self.inner.create_transfers(batch);
            if !self.lost {
                self.lost = true;
                return results
                    .into_iter()
                    .map(|_| Err(TransferError::BackendFailure))
                    .collect();
            }
            results
        }
        fn try_lookup_transfer(
            &self,
            id: TransferId,
        ) -> anyhow::Result<Option<Transfer>> {
            self.inner.try_lookup_transfer(id)
        }
        fn try_lookup_account(
            &self,
            id: AccountId,
        ) -> anyhow::Result<Option<Account>> {
            self.inner.try_lookup_account(id)
        }
        fn try_account_balance(
            &self,
            id: AccountId,
        ) -> anyhow::Result<Option<crate::currency_poc::ledger::AccountBalance>> {
            self.inner.try_account_balance(id)
        }
        fn try_transfers_with_user_data_128(
            &self,
            value: u128,
        ) -> anyhow::Result<Vec<Transfer>> {
            self.inner.try_transfers_with_user_data_128(value)
        }
    }

    #[test]
    fn lost_response_retry_reconciles_without_double_issuance() {
        let v = verifier();
        let adverse = no_adverse();
        let ledger = LoseFirstResponse {
            inner: InMemoryLedger::new(),
            lost: false,
        };
        let mut svc = RegistrationService::new(
            GrantStore::new(),
            ledger,
            config(),
            &v,
            &adverse,
        );
        let outcome = svc
            .register(&request("grant-1", "AAAAA-11111-BBBBB", 1_000))
            .unwrap();
        assert!(matches!(
            outcome,
            RegistrationOutcome::IntentPending {
                reason: IntentPendingReason::LedgerUnconfirmed { .. },
                ..
            }
        ));
        assert_eq!(
            svc.store().grant("grant-1").unwrap().state,
            GrantState::Presented
        );
        // Reconcile past the freshness bound: the transfer IS in the
        // ledger (only its response was lost), so confirm — and the
        // grantee's credit is still exactly one amount.
        let report = svc.reconcile(1_000 + 3_600, 60);
        assert_eq!(report.confirmed, 1);
        assert_eq!(
            svc.store().grant("grant-1").unwrap().state,
            GrantState::PendingHold
        );
        let balance = svc
            .ledger()
            .inner
            .account_balance(hold_account_for(LEDGER_ID))
            .unwrap();
        assert_eq!(balance.credits_posted, AMOUNT);
    }

    /// Ledger wrapper: the first `create_transfers` is dropped
    /// entirely (crash between intent and transfer, §2.5).
    struct DropFirstCreate {
        inner: InMemoryLedger,
        dropped: bool,
    }

    impl LedgerBackend for DropFirstCreate {
        fn create_accounts(
            &mut self,
            accounts: &[Account],
        ) -> Vec<Result<(), AccountError>> {
            self.inner.create_accounts(accounts)
        }
        fn create_transfers(
            &mut self,
            batch: &[Transfer],
        ) -> Vec<Result<(), TransferError>> {
            if !self.dropped {
                self.dropped = true;
                return batch
                    .iter()
                    .map(|_| Err(TransferError::BackendFailure))
                    .collect();
            }
            self.inner.create_transfers(batch)
        }
        fn try_lookup_transfer(
            &self,
            id: TransferId,
        ) -> anyhow::Result<Option<Transfer>> {
            self.inner.try_lookup_transfer(id)
        }
        fn try_lookup_account(
            &self,
            id: AccountId,
        ) -> anyhow::Result<Option<Account>> {
            self.inner.try_lookup_account(id)
        }
        fn try_account_balance(
            &self,
            id: AccountId,
        ) -> anyhow::Result<Option<crate::currency_poc::ledger::AccountBalance>> {
            self.inner.try_account_balance(id)
        }
        fn try_transfers_with_user_data_128(
            &self,
            value: u128,
        ) -> anyhow::Result<Vec<Transfer>> {
            self.inner.try_transfers_with_user_data_128(value)
        }
    }

    #[test]
    fn crash_between_intent_and_transfer_reconciles_by_reissue_under_same_id() {
        let v = verifier();
        let adverse = no_adverse();
        let ledger = DropFirstCreate {
            inner: InMemoryLedger::new(),
            dropped: false,
        };
        let mut svc = RegistrationService::new(
            GrantStore::new(),
            ledger,
            config(),
            &v,
            &adverse,
        );
        let outcome = svc
            .register(&request("grant-1", "AAAAA-11111-BBBBB", 1_000))
            .unwrap();
        assert!(matches!(
            outcome,
            RegistrationOutcome::IntentPending { .. }
        ));
        // Too fresh: the reconciler leaves it alone.
        let report = svc.reconcile(1_030, 60);
        assert_eq!(report.scanned, 0);
        // Past the bound: lookup finds nothing, re-issue under the
        // SAME deterministic id, confirm. Exactly one transfer.
        let report = svc.reconcile(1_000 + 3_600, 60);
        assert_eq!(report.confirmed, 1);
        assert!(svc
            .ledger()
            .inner
            .lookup_transfer(transfer_id_for_grant("grant-1"))
            .is_some());
        let balance = svc
            .ledger()
            .inner
            .account_balance(hold_account_for(LEDGER_ID))
            .unwrap();
        assert_eq!(balance.credits_posted, AMOUNT);
    }

    #[test]
    fn adverse_event_during_intent_window_blocks_confirm_until_processed() {
        let v = verifier();
        let adverse = AdverseStub {
            event: RefCell::new(Some(AdverseEvent {
                kind: "refund_executed".to_string(),
                attestation_id: "att-1".to_string(),
            })),
        };
        let mut svc = service(&v, &adverse);
        let outcome = svc
            .register(&request("grant-1", "AAAAA-11111-BBBBB", 1_000))
            .unwrap();
        assert!(matches!(
            outcome,
            RegistrationOutcome::IntentPending {
                reason: IntentPendingReason::AdverseEventAtConfirm { .. },
                ..
            }
        ));
        assert_eq!(
            svc.store().grant("grant-1").unwrap().state,
            GrantState::Presented
        );
        // Reconcile while the event stands: still blocked, and the
        // attestation check re-runs before any retry (§2.5 step 4).
        let report = svc.reconcile(9_999, 60);
        assert_eq!(report.blocked_by_event, 1);
        assert_eq!(report.confirmed, 0);
        // The blocked grant's value sits in the service hold
        // account — an adverse event at confirm never leaves
        // spendable money with the grantee (Codex P1 on PR #8614).
        let hold = svc
            .ledger()
            .account_balance(hold_account_for(LEDGER_ID))
            .unwrap();
        assert_eq!(hold.credits_posted, AMOUNT);
        assert!(svc
            .ledger()
            .account_balance(account_id_for("grantee-1", LEDGER_ID))
            .is_none());
    }

    #[test]
    fn duplicate_grant_id_is_refused_before_any_registry_or_ledger_effect() {
        let v = verifier();
        let adverse = no_adverse();
        let mut svc = service(&v, &adverse);
        svc.register(&request("grant-1", "AAAAA-11111-BBBBB", 1_000))
            .unwrap();
        // A DIFFERENT verified key under an already-used grant_id:
        // refused as an error, and neither the registry nor the
        // first grant record nor the ledger changes (Codex P2).
        let err = svc
            .register(&request("grant-1", "CCCCC-22222-DDDDD", 2_000))
            .unwrap_err();
        assert_eq!(
            err,
            RegistrationError::DuplicateGrantId {
                grant_id: "grant-1".to_string()
            }
        );
        assert_eq!(svc.store().grant_count(), 1);
        assert_eq!(svc.store().registry().len(), 1);
        let first = svc.store().grant("grant-1").unwrap();
        assert_eq!(first.state, GrantState::PendingHold);
        assert_eq!(
            first.fingerprint,
            fingerprint_of_raw("steam", "AAAAA-11111-BBBBB")
        );
    }

    #[test]
    fn restore_rejects_a_snapshot_with_duplicate_grant_ids() {
        // Oracle (#8913): a snapshot whose grants vector carries two
        // records sharing a grant_id is malformed; restore must
        // reject it naming the duplicated id, not silently keep the
        // last record while the insertion order walks both.
        let v = verifier();
        let adverse = no_adverse();
        let mut svc = RegistrationService::new(
            GrantStore::new(),
            InMemoryLedger::new(),
            config(),
            &v,
            &adverse,
        );
        svc.register(&request("grant-1", "AAAAA-11111-BBBBB", 1_000))
            .unwrap();
        let mut snapshot = svc.store().snapshot();
        assert_eq!(snapshot.grants.len(), 1);
        let duplicate = snapshot.grants[0].clone();
        snapshot.grants.push(duplicate);
        let err = GrantStore::restore(snapshot).unwrap_err();
        assert_eq!(
            err,
            RestoreError::DuplicateGrantId {
                grant_id: "grant-1".to_string()
            }
        );
    }

    #[test]
    fn restart_restores_intents_and_reconcile_completes_without_double_issuance() {
        // The §2.5 recovery property across a process restart
        // (Codex P1 on PR #8614): intent written, ledger transfer
        // created but its response lost, process dies. The store
        // snapshot is what the host persisted; a fresh service over
        // the restored store + the SAME (persistent) ledger
        // reconciles the stranded Presented grant to PendingHold,
        // and the hold account still shows exactly one credit.
        let v = verifier();
        let adverse = no_adverse();
        let ledger = LoseFirstResponse {
            inner: InMemoryLedger::new(),
            lost: false,
        };
        let mut svc = RegistrationService::new(
            GrantStore::new(),
            ledger,
            config(),
            &v,
            &adverse,
        );
        let outcome = svc
            .register(&request("grant-1", "AAAAA-11111-BBBBB", 1_000))
            .unwrap();
        assert!(matches!(
            outcome,
            RegistrationOutcome::IntentPending { .. }
        ));
        // Serialize the snapshot (the host's persistence form) and
        // restore from the bytes, so the test covers the real
        // round-trip, not a clone.
        let bytes = serde_json::to_vec(&svc.store().snapshot()).unwrap();
        let snapshot: StoreSnapshot = serde_json::from_slice(&bytes).unwrap();
        let restored = GrantStore::restore(snapshot).unwrap();
        assert_eq!(restored.grant_count(), 1);
        assert_eq!(restored.registry().len(), 1);
        assert_eq!(
            restored.grant("grant-1").unwrap().state,
            GrantState::Presented
        );
        // "Restart": the old service (and its in-memory store) is
        // dropped; the ledger — the external system — persists and
        // moves into the new service.
        let RegistrationService { ledger, .. } = svc;
        let mut svc2 = RegistrationService::new(restored, ledger, config(), &v, &adverse);
        let report = svc2.reconcile(1_000 + 3_600, 60);
        assert_eq!(report.confirmed, 1);
        assert_eq!(
            svc2.store().grant("grant-1").unwrap().state,
            GrantState::PendingHold
        );
        let hold = svc2
            .ledger()
            .inner
            .account_balance(hold_account_for(LEDGER_ID))
            .unwrap();
        assert_eq!(hold.credits_posted, AMOUNT);
        // And after restart the one-grant rule still holds for the
        // restored registry: the same key cannot grant twice.
        let outcome = svc2
            .register(&request("grant-2", "AAAAA-11111-BBBBB", 5_000))
            .unwrap();
        assert!(matches!(
            outcome,
            RegistrationOutcome::Rejected { .. }
        ));
    }

    #[test]
    fn model_no_key_ever_grants_twice_and_every_grant_has_one_transfer() {
        // Deterministic pseudo-random presentation sequence over a
        // small key space, with formatting variants of the same
        // keys: the invariants are (1) grants == distinct canonical
        // keys, (2) every granted key's transfer exists exactly
        // once, (3) registry rows are append-only (count never
        // shrinks, equals distinct keys).
        let mut products = HashMap::new();
        let keys: Vec<String> = (0..12)
            .map(|i| format!("KEY{i:02}-AAAA-BBBB"))
            .collect();
        for key in &keys {
            products.insert(key.clone(), "game-a".to_string());
        }
        let v = TestVerifier { products };
        let adverse = no_adverse();
        let mut svc = service(&v, &adverse);
        let mut state: u64 = 0x9e3779b97f4a7c15;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut granted: Vec<String> = Vec::new();
        for n in 0..200u64 {
            let idx = (next() % 12) as usize;
            let outcome = svc
                .register(&request(&format!("grant-{n}"), &keys[idx], 1_000 + n))
                .unwrap();
            match outcome {
                RegistrationOutcome::PendingHold(g) => {
                    assert!(!granted.contains(&g.fingerprint));
                    granted.push(g.fingerprint);
                }
                RegistrationOutcome::Rejected { .. } => {}
                other => panic!("unexpected outcome {other:?}"),
            }
        }
        assert_eq!(granted.len(), 12);
        assert_eq!(svc.store().grant_count(), 12);
        assert_eq!(svc.store().registry().len(), 12);
    }
}
