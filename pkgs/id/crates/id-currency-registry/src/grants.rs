//! Grant service — slice G1 of the refundable-grants design
//! (#2065, issue #2294).
//!
//! The design (`doc/2026-10-29T12-10-00Z_design_refundable_grants_pending_transfers`,
//! sections 2.2–2.4, 4, and 6.3) grants premium tokens as one
//! `TigerBeetle` *pending* transfer per Steam key, so a refund that
//! arrives inside the window can void the hold instead of clawing
//! back posted balance. This module is the control-plane half of
//! that design:
//!
//! - the `grant` record of design section 2.3 ([`Grant`]), with the
//!   `correction_of` lineage field section 4 adds for
//!   void-and-reissue partial refunds;
//! - a store seam ([`GrantStore`]) with an in-memory implementation
//!   ([`InMemoryGrantStore`]) enforcing the same rules migration
//!   `0002` enforces — one live grant per `steam_key_hash`, a key
//!   redeems once ever — so the acceptance behaviour is testable
//!   without a database, exactly like the registry's [`Store`]
//!   seam;
//! - a pending-transfer ledger seam ([`PendingLedger`]) with an
//!   in-memory stub ([`StubLedger`]) enforcing `TigerBeetle`'s
//!   at-most-once resolution, so the service can be proven against
//!   the design's stub direction (issue #2292 gates only the
//!   dev-cluster acceptance runs, not this build);
//! - the service itself ([`GrantService`]): register / vest / void
//!   / partial refund / late-refund clawback, with the gateway's
//!   HTTP mapping (design section 2.4) left to the gateway
//!   layer — an [`GrantError::AlreadyResolved`] *is* the API's
//!   `409` outcome.
//!
//! The rule section 4 states plainly, enforced here: **partial
//! posting is for policy, void-and-reissue is for refunds.** Both
//! paths are implemented: [`GrantService::refund_partial`] is the
//! refund path, and [`GrantService::vest_partial`] is the policy
//! partial vest (#10070 item 4). A refund that arrives *after*
//! posting takes the third path, [`GrantService::refund_late`]:
//! the section 6.2 **clawback then debt** policy — attempt the
//! clawback transfer; on `exceeds_credits` the grant is marked
//! `debt`, a recorded receivable, never a negative balance. The
//! vesting automation /
//! reconciler that sweeps expired holds (section 6.3) is
//! likewise a separate issue; the
//! `Expired` state and the intent-first write order here are what
//! it will reconcile against.
//!
//! [`Store`]: crate::store::Store

use std::collections::BTreeMap;
use std::fmt;

use sha2::{Digest, Sha256};

use crate::factory::transfer_code;

/// The `TigerBeetle` transfer `code` for a grant's pending transfer.
///
/// The first code of the reserved pending-hold redemption range
/// (design section 2.2 asks for a dedicated grant range; the
/// factory's reserved table already sets `0xF000..=0xFFFF` aside
/// for pending-hold redemption, #2065).
pub const GRANT_TRANSFER_CODE: u16 = transfer_code::PENDING_HOLD_REDEMPTION_START;

/// The `TigerBeetle` transfer `code` for a grant's clawback
/// transfer (design section 6.2).
///
/// The first code of the factory's reserved correction / refund
/// range (`0x0400..=0x04FF`): a clawback is a plain transfer that
/// corrects a posted grant, not a pending-hold resolution, so it
/// does not ride the grant range above.
pub const CLAWBACK_TRANSFER_CODE: u16 = transfer_code::CORRECTION_START;

/// Default slack added to the vesting window.
///
/// It forms a pending transfer's `TigerBeetle` timeout (design
/// section 6.3 recommends +72h): the timeout is a safety net
/// against a dead service, never the vesting trigger.
pub const DEFAULT_TIMEOUT_SLACK_SECONDS: u64 = 72 * 60 * 60;

/// The `sha256` of a Steam key.
///
/// Design section 2.3: the key itself is never stored in the
/// ledger or the control plane, only its hash; the same hash's
/// first eight bytes ride on the transfer as `user_data_64`.
#[must_use]
pub fn steam_key_hash(steam_key: &str) -> [u8; 32] {
    let digest = Sha256::digest(steam_key.as_bytes());
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&digest);
    hash
}

/// The `user_data_64` form of a steam key hash: its first eight
/// bytes as a big-endian `u64`.
#[must_use]
pub fn steam_key_hash_u64(hash: &[u8; 32]) -> u64 {
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&hash[..8]);
    u64::from_be_bytes(bytes)
}

/// The lifecycle state of a grant (design sections 2.1 and 2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GrantState {
    /// The pending transfer is submitted; the hold is unresolved.
    Pending,
    /// The hold posted (`post_pending_transfer`); tokens are
    /// ordinary posted balance from here on.
    Posted,
    /// The hold was voided (`void_pending_transfer`) — the refund
    /// path.
    Voided,
    /// The hold hit its `TigerBeetle` timeout unresolved; a
    /// reconcilable state, not a loss (section 6.3).
    Expired,
    /// A late refund (after posting) is being clawed back.
    ClawbackPending,
    /// The clawback transfer settled.
    ClawbackSettled,
    /// The clawback failed because the holder already spent the
    /// balance; the bounded-exposure debt record (section 6).
    Debt,
}

impl GrantState {
    /// The state as stored in the `grant` table (migration `0002`
    /// checks `state` against exactly these strings).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Posted => "posted",
            Self::Voided => "voided",
            Self::Expired => "expired",
            Self::ClawbackPending => "clawback_pending",
            Self::ClawbackSettled => "clawback_settled",
            Self::Debt => "debt",
        }
    }

    /// Whether the grant's hold is already resolved (a further
    /// vest/void is the idempotent `409` outcome, never a second
    /// resolution).
    #[must_use]
    pub const fn is_resolved(self) -> bool {
        !matches!(self, Self::Pending)
    }
}

impl fmt::Display for GrantState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One row of the `grant` table.
///
/// Covers design section 2.3, plus `correction_of` from section
/// 4. The database is a projection with intent; `TigerBeetle` is
/// the truth about money — every state change is written
/// intent-first, transfer-second, confirm-third, so a crash in
/// between leaves a reconcilable row rather than an orphaned hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    /// Control-plane grant id; also `user_data_128` on the
    /// transfer.
    pub id: u128,
    /// `sha256` of the Steam key (the key itself is never stored).
    pub steam_key_hash: [u8; 32],
    /// The user the grant is for.
    pub user_id: String,
    /// The premium-token ledger (`TigerBeetle` ledger u32).
    pub ledger: u32,
    /// Grant amount in the ledger's smallest unit.
    pub amount: u128,
    /// Current lifecycle state.
    pub state: GrantState,
    /// The pending transfer holding the grant's value.
    pub pending_transfer_id: u128,
    /// The post or void transfer that resolved the hold, once
    /// resolved.
    pub resolution_transfer_id: Option<u128>,
    /// Window start (unix seconds): when the key was redeemed.
    pub window_started_at: u64,
    /// Window end (unix seconds): when the grant may vest. A
    /// reissue keeps its original grant's window — the refund
    /// window is anchored at redemption, not at reissue.
    pub window_ends_at: u64,
    /// Row creation time (unix seconds).
    pub created_at: u64,
    /// Row last-update time (unix seconds).
    pub updated_at: u64,
    /// How many times a ledger operation for this grant has been
    /// attempted (the reconciler's retry signal).
    pub attempt_count: u32,
    /// The most recent ledger error, if any.
    pub last_error: Option<String>,
    /// For a reissue after a partial refund: the grant it corrects
    /// (design section 4's lineage link).
    pub correction_of: Option<u128>,
}

/// A grant-store failure. Each variant mirrors a constraint in
/// migration `0002`, named so a caller can tell a uniqueness
/// rejection (a verdict about the request) from an unknown
/// reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantStoreError {
    /// A grant with this id already exists; ids are never reused.
    DuplicateGrantId(u128),
    /// A live grant already holds this `steam_key_hash` (a key
    /// redeems once, ever — design section 2.4).
    DuplicateKeyHash,
    /// No grant exists for this id.
    UnknownGrant(u128),
}

impl fmt::Display for GrantStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateGrantId(id) => write!(f, "grant {id} already exists"),
            Self::DuplicateKeyHash => {
                write!(f, "a live grant already holds this steam_key_hash")
            }
            Self::UnknownGrant(id) => write!(f, "grant {id} does not exist"),
        }
    }
}

impl std::error::Error for GrantStoreError {}

/// The grant store seam: the operations the service needs, with
/// migration `0002`'s constraints as the contract.
pub trait GrantStore {
    /// Insert a new grant row.
    ///
    /// # Errors
    ///
    /// [`GrantStoreError::DuplicateGrantId`] if the id is taken;
    /// [`GrantStoreError::DuplicateKeyHash`] if a live grant
    /// already holds the key hash. A *voided* grant's hash may be
    /// re-inserted only by its own correction chain: a reissue
    /// whose `correction_of` names the voided tip, with every
    /// other prior for the hash voided on that tip's ancestry
    /// (so A->B->C chains are accepted but forks and fresh
    /// registrations over voided grants are rejected).
    fn insert_grant(&mut self, grant: &Grant) -> Result<(), GrantStoreError>;

    /// Replace an existing grant row (the service's
    /// confirm-third write).
    ///
    /// # Errors
    ///
    /// [`GrantStoreError::UnknownGrant`] if the id does not exist.
    fn update_grant(&mut self, grant: &Grant) -> Result<(), GrantStoreError>;

    /// The grant with this id, if any.
    fn grant(&self, id: u128) -> Option<Grant>;

    /// The most recent grant recorded for this key hash, if any
    /// (live or resolved — register's idempotency check reads
    /// every state).
    fn grant_by_key_hash(&self, hash: &[u8; 32]) -> Option<Grant>;

    /// Every grant recorded for a user, in grant-id order (the
    /// `GET /users/{id}/grants` read).
    fn grants_for_user(&self, user_id: &str) -> Vec<Grant>;
}

/// In-memory [`GrantStore`] enforcing migration `0002`'s rules, so
/// behaviour proven against it is the behaviour the schema
/// enforces.
#[derive(Debug, Default)]
pub struct InMemoryGrantStore {
    by_id: BTreeMap<u128, Grant>,
    /// Key hash -> grant ids in insertion order (a hash can recur
    /// only across a voided grant and its corrections).
    by_hash: BTreeMap<[u8; 32], Vec<u128>>,
}

impl InMemoryGrantStore {
    /// A new, empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl GrantStore for InMemoryGrantStore {
    fn insert_grant(&mut self, grant: &Grant) -> Result<(), GrantStoreError> {
        if self.by_id.contains_key(&grant.id) {
            return Err(GrantStoreError::DuplicateGrantId(grant.id));
        }
        if let Some(ids) = self.by_hash.get(&grant.steam_key_hash) {
            // The incoming grant's correction ancestry: the voided
            // tip it corrects plus every grant that tip
            // transitively corrects. A chained reissue (A->B->C) is
            // a reissue of the voided tip B whose own ancestry
            // covers A; a fork off a non-tip, or a fresh
            // registration over a voided grant, names no such chain
            // and is still rejected.
            let mut ancestors: Vec<u128> = Vec::new();
            let mut cursor = grant.correction_of;
            while let Some(id) = cursor {
                if ancestors.contains(&id) {
                    break;
                }
                ancestors.push(id);
                cursor = self.by_id.get(&id).and_then(|g| g.correction_of);
            }
            for id in ids {
                match self.by_id.get(id) {
                    Some(existing)
                        if existing.state == GrantState::Voided
                            && ancestors.contains(&existing.id) => {}
                    _ => return Err(GrantStoreError::DuplicateKeyHash),
                }
            }
            // The correction must name a real voided tip holding
            // this same hash — a `correction_of` pointing nowhere,
            // at a live grant, or at another hash's row is not a
            // reissue of this chain.
            match grant.correction_of {
                Some(parent)
                    if ancestors.first() == Some(&parent)
                        && matches!(
                            self.by_id.get(&parent),
                            Some(p) if p.state == GrantState::Voided
                                && p.steam_key_hash == grant.steam_key_hash
                        ) => {}
                _ => return Err(GrantStoreError::DuplicateKeyHash),
            }
        }
        self.by_id.insert(grant.id, grant.clone());
        self.by_hash
            .entry(grant.steam_key_hash)
            .or_default()
            .push(grant.id);
        Ok(())
    }

    fn update_grant(&mut self, grant: &Grant) -> Result<(), GrantStoreError> {
        if !self.by_id.contains_key(&grant.id) {
            return Err(GrantStoreError::UnknownGrant(grant.id));
        }
        self.by_id.insert(grant.id, grant.clone());
        Ok(())
    }

    fn grant(&self, id: u128) -> Option<Grant> {
        self.by_id.get(&id).cloned()
    }

    fn grant_by_key_hash(&self, hash: &[u8; 32]) -> Option<Grant> {
        self.by_hash
            .get(hash)
            .and_then(|ids| ids.last())
            .and_then(|id| self.by_id.get(id).cloned())
    }

    fn grants_for_user(&self, user_id: &str) -> Vec<Grant> {
        self.by_id
            .values()
            .filter(|grant| grant.user_id == user_id)
            .cloned()
            .collect()
    }
}

/// One pending transfer to submit (design section 2.2): the whole
/// grant as a single hold, never a stream of small ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingTransferSpec {
    /// The transfer's id (also the grant row's
    /// `pending_transfer_id`).
    pub id: u128,
    /// Debits side: the issuer grant-pool account.
    pub debit_account_id: u128,
    /// Credits side: the holder's premium account.
    pub credit_account_id: u128,
    /// Hold amount.
    pub amount: u128,
    /// The premium-token ledger.
    pub ledger: u32,
    /// Transfer code ([`GRANT_TRANSFER_CODE`]).
    pub code: u16,
    /// The control-plane grant id.
    pub user_data_128: u128,
    /// Hash of the Steam key (never the key itself).
    pub user_data_64: u64,
    /// Safety-net timeout in seconds (window + slack, section
    /// 6.3).
    pub timeout_seconds: u64,
}

/// One clawback transfer to submit (design section 6.2).
///
/// A plain — not pending — transfer of already-posted balance
/// from the holder back to the grant pool, for a refund that
/// arrived after the grant posted. Unlike a hold resolution it
/// can fail on the money itself
/// ([`LedgerError::ExceedsCredits`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClawbackTransferSpec {
    /// The transfer's id.
    pub id: u128,
    /// Debits side: the holder's premium account.
    pub debit_account_id: u128,
    /// Credits side: the issuer grant-pool account.
    pub credit_account_id: u128,
    /// Clawback amount (the refunded part of the posted grant).
    pub amount: u128,
    /// The premium-token ledger.
    pub ledger: u32,
    /// Transfer code ([`CLAWBACK_TRANSFER_CODE`]).
    pub code: u16,
    /// The control-plane grant id.
    pub user_data_128: u128,
    /// Hash of the Steam key (never the key itself).
    pub user_data_64: u64,
}

/// A pending-ledger failure.
///
/// The `Already*` variants are *verdicts* — `TigerBeetle`'s
/// at-most-once resolution answering a repeat attempt — not
/// transport failures; the service maps them to its own
/// already-resolved outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerError {
    /// The hold already posted (a repeat resolution attempt).
    AlreadyPosted(u128),
    /// The hold already voided (a repeat resolution attempt).
    AlreadyVoided(u128),
    /// The hold already expired (a repeat resolution attempt).
    AlreadyExpired(u128),
    /// No transfer exists with this id.
    UnknownTransfer(u128),
    /// The resolution amount exceeds the held amount.
    AmountExceedsHold {
        /// The pending transfer.
        pending_id: u128,
        /// The held amount.
        held: u128,
        /// The attempted resolution amount.
        attempted: u128,
    },
    /// The clawback transfer would exceed the debit account's
    /// posted credits (`exceeds_credits`): the holder already
    /// spent the balance. A *verdict* — design section 6.2's
    /// debt trigger — not a transport failure; the transfer
    /// moved nothing. Carries the clawback transfer's id.
    ExceedsCredits(u128),
    /// A non-verdict cluster failure (transport, timeout,
    /// unavailable).
    Cluster(String),
}

impl fmt::Display for LedgerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyPosted(id) => write!(f, "pending transfer {id} already posted"),
            Self::AlreadyVoided(id) => write!(f, "pending transfer {id} already voided"),
            Self::AlreadyExpired(id) => write!(f, "pending transfer {id} already expired"),
            Self::UnknownTransfer(id) => write!(f, "pending transfer {id} does not exist"),
            Self::AmountExceedsHold {
                pending_id,
                held,
                attempted,
            } => write!(
                f,
                "resolution amount {attempted} exceeds the {held} held by transfer {pending_id}"
            ),
            Self::ExceedsCredits(id) => {
                write!(
                    f,
                    "clawback transfer {id} exceeds the holder's posted credits"
                )
            }
            Self::Cluster(message) => write!(f, "cluster error: {message}"),
        }
    }
}

impl std::error::Error for LedgerError {}

/// The ledger seam: the `TigerBeetle` operations the grant
/// service needs.
///
/// That is the three pending-transfer operations plus the
/// section 6.2 clawback transfer. The production implementation
/// is the gateway's `TigerBeetle` client; tests substitute
/// [`StubLedger`].
pub trait PendingLedger {
    /// Submit a pending transfer. Submitting the same transfer id
    /// twice with the same spec is an idempotent success (the
    /// service's intent-first retry path relies on it).
    ///
    /// # Errors
    ///
    /// [`LedgerError::Cluster`] on a non-verdict failure, or if
    /// the id is taken by a *different* spec.
    fn create_pending(&mut self, spec: &PendingTransferSpec) -> Result<(), LedgerError>;

    /// Post a hold (`post_pending_transfer`): all of `amount`
    /// becomes posted balance on the credit account; any unposted
    /// remainder of the hold is restored.
    ///
    /// # Errors
    ///
    /// The `Already*` verdicts on a repeat resolution;
    /// [`LedgerError::UnknownTransfer`] for an unknown id;
    /// [`LedgerError::AmountExceedsHold`] if `amount` exceeds the
    /// hold.
    fn post_pending(
        &mut self,
        pending_id: u128,
        resolution_id: u128,
        amount: u128,
    ) -> Result<(), LedgerError>;

    /// Void a hold (`void_pending_transfer`): the whole hold is
    /// released; nothing posts. Voiding is all-or-nothing — a
    /// partial refund voids and reissues (design section 4).
    ///
    /// # Errors
    ///
    /// The `Already*` verdicts on a repeat resolution;
    /// [`LedgerError::UnknownTransfer`] for an unknown id.
    fn void_pending(&mut self, pending_id: u128, resolution_id: u128) -> Result<(), LedgerError>;

    /// Submit a clawback transfer (design section 6.2): move the
    /// spec's amount of *posted* balance from the debit (holder)
    /// account back to the credit (grant pool) account.
    /// Submitting the same transfer id twice with the same spec
    /// returns the first attempt's verdict — settled, or the
    /// same [`LedgerError::ExceedsCredits`] — so a retry never
    /// moves the money twice.
    ///
    /// # Errors
    ///
    /// [`LedgerError::ExceedsCredits`] if the debit account's
    /// posted balance does not cover the amount (the transfer
    /// moves nothing); [`LedgerError::Cluster`] on a
    /// non-verdict failure, or if the id is taken by a
    /// *different* spec.
    fn clawback(&mut self, spec: &ClawbackTransferSpec) -> Result<(), LedgerError>;
}

/// The stub's record of one transfer.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StubTransfer {
    spec: PendingTransferSpec,
    state: GrantState,
    resolution_id: Option<u128>,
}

/// The stub's record of one clawback transfer: the spec and
/// whether it settled, so a resubmission returns the first
/// attempt's verdict instead of moving money twice.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StubClawback {
    spec: ClawbackTransferSpec,
    settled: bool,
}

/// In-memory [`PendingLedger`] enforcing `TigerBeetle`'s two-phase semantics.
///
/// As the design states them: a hold reserves `credits_pending`
/// on the credit account; exactly one resolution (post / void /
/// timeout) releases it, at most once; posting part restores the
/// remainder. The balances it tracks are the test oracle for "a
/// refund left no posted credit behind".
#[derive(Debug, Default)]
pub struct StubLedger {
    transfers: BTreeMap<u128, StubTransfer>,
    clawbacks: BTreeMap<u128, StubClawback>,
    credits_pending: BTreeMap<u128, u128>,
    credits_posted: BTreeMap<u128, u128>,
    debits_pending: BTreeMap<u128, u128>,
}

impl StubLedger {
    /// A new, empty stub ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Posted balance of an account (the spendable truth).
    #[must_use]
    pub fn credits_posted(&self, account_id: u128) -> u128 {
        self.credits_posted.get(&account_id).copied().unwrap_or(0)
    }

    /// Pending (held, unspendable) balance of an account.
    #[must_use]
    pub fn credits_pending(&self, account_id: u128) -> u128 {
        self.credits_pending.get(&account_id).copied().unwrap_or(0)
    }

    /// Simulate the safety-net timeout firing on a hold: the hold
    /// expires and the reservation is restored, unresolved
    /// (design section 2.1). A resolved hold ignores the timeout,
    /// as in `TigerBeetle`.
    pub fn expire_pending(&mut self, pending_id: u128) {
        let Some(transfer) = self.transfers.get_mut(&pending_id) else {
            return;
        };
        if transfer.state == GrantState::Pending {
            transfer.state = GrantState::Expired;
            let amount = transfer.spec.amount;
            let credit = transfer.spec.credit_account_id;
            let debit = transfer.spec.debit_account_id;
            release(&mut self.credits_pending, credit, amount);
            release(&mut self.debits_pending, debit, amount);
        }
    }

    /// Simulate the holder spending posted balance elsewhere
    /// (the precondition of design section 6.2's debt case: by
    /// the time a late refund arrives, some or all of the tokens
    /// are gone). Saturates at zero, like a balance that cannot
    /// go negative.
    pub fn spend_posted(&mut self, account_id: u128, amount: u128) {
        release(&mut self.credits_posted, account_id, amount);
    }

    fn resolution_verdict(&self, pending_id: u128) -> Result<&StubTransfer, LedgerError> {
        let transfer = self
            .transfers
            .get(&pending_id)
            .ok_or(LedgerError::UnknownTransfer(pending_id))?;
        match transfer.state {
            GrantState::Pending => Ok(transfer),
            GrantState::Posted => Err(LedgerError::AlreadyPosted(pending_id)),
            GrantState::Voided => Err(LedgerError::AlreadyVoided(pending_id)),
            _ => Err(LedgerError::AlreadyExpired(pending_id)),
        }
    }
}

fn release(map: &mut BTreeMap<u128, u128>, account: u128, amount: u128) {
    if let Some(balance) = map.get_mut(&account) {
        *balance = balance.saturating_sub(amount);
    }
}

fn add(map: &mut BTreeMap<u128, u128>, account: u128, amount: u128) {
    *map.entry(account).or_insert(0) += amount;
}

impl PendingLedger for StubLedger {
    fn create_pending(&mut self, spec: &PendingTransferSpec) -> Result<(), LedgerError> {
        if let Some(existing) = self.transfers.get(&spec.id) {
            if existing.spec == *spec {
                return Ok(());
            }
            return Err(LedgerError::Cluster(format!(
                "transfer {} already exists with a different spec",
                spec.id
            )));
        }
        self.transfers.insert(
            spec.id,
            StubTransfer {
                spec: spec.clone(),
                state: GrantState::Pending,
                resolution_id: None,
            },
        );
        add(
            &mut self.credits_pending,
            spec.credit_account_id,
            spec.amount,
        );
        add(&mut self.debits_pending, spec.debit_account_id, spec.amount);
        Ok(())
    }

    fn post_pending(
        &mut self,
        pending_id: u128,
        resolution_id: u128,
        amount: u128,
    ) -> Result<(), LedgerError> {
        let spec = self.resolution_verdict(pending_id)?.spec.clone();
        if amount > spec.amount {
            return Err(LedgerError::AmountExceedsHold {
                pending_id,
                held: spec.amount,
                attempted: amount,
            });
        }
        if let Some(transfer) = self.transfers.get_mut(&pending_id) {
            transfer.state = GrantState::Posted;
            transfer.resolution_id = Some(resolution_id);
        }
        release(
            &mut self.credits_pending,
            spec.credit_account_id,
            spec.amount,
        );
        release(&mut self.debits_pending, spec.debit_account_id, spec.amount);
        add(&mut self.credits_posted, spec.credit_account_id, amount);
        Ok(())
    }

    fn void_pending(&mut self, pending_id: u128, resolution_id: u128) -> Result<(), LedgerError> {
        let spec = self.resolution_verdict(pending_id)?.spec.clone();
        if let Some(transfer) = self.transfers.get_mut(&pending_id) {
            transfer.state = GrantState::Voided;
            transfer.resolution_id = Some(resolution_id);
        }
        release(
            &mut self.credits_pending,
            spec.credit_account_id,
            spec.amount,
        );
        release(&mut self.debits_pending, spec.debit_account_id, spec.amount);
        Ok(())
    }

    fn clawback(&mut self, spec: &ClawbackTransferSpec) -> Result<(), LedgerError> {
        if let Some(existing) = self.clawbacks.get(&spec.id) {
            if existing.spec != *spec {
                return Err(LedgerError::Cluster(format!(
                    "transfer {} already exists with a different spec",
                    spec.id
                )));
            }
            return if existing.settled {
                Ok(())
            } else {
                Err(LedgerError::ExceedsCredits(spec.id))
            };
        }
        if self.credits_posted(spec.debit_account_id) < spec.amount {
            self.clawbacks.insert(
                spec.id,
                StubClawback {
                    spec: spec.clone(),
                    settled: false,
                },
            );
            return Err(LedgerError::ExceedsCredits(spec.id));
        }
        release(&mut self.credits_posted, spec.debit_account_id, spec.amount);
        add(
            &mut self.credits_posted,
            spec.credit_account_id,
            spec.amount,
        );
        self.clawbacks.insert(
            spec.id,
            StubClawback {
                spec: spec.clone(),
                settled: true,
            },
        );
        Ok(())
    }
}

/// Grant-service configuration.
///
/// The window length is a product pin (design section 3), ruled
/// on issue #2296 to come from config — 14 days, the Steam refund
/// window — and *never* be hard-coded in the service; it arrives
/// here from the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrantConfig {
    /// Vesting window in seconds (window start = key redemption).
    pub window_seconds: u64,
    /// Slack added to the window to form the pending transfer's
    /// timeout (section 6.3).
    pub timeout_slack_seconds: u64,
}

impl GrantConfig {
    /// A config with the design's recommended timeout slack
    /// ([`DEFAULT_TIMEOUT_SLACK_SECONDS`]).
    #[must_use]
    pub const fn new(window_seconds: u64) -> Self {
        Self {
            window_seconds,
            timeout_slack_seconds: DEFAULT_TIMEOUT_SLACK_SECONDS,
        }
    }
}

/// A grant-service failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantError {
    /// The grant is already resolved; vest/void on it is the
    /// idempotent `409` outcome of design section 2.4, carrying
    /// the state the grant actually reached.
    AlreadyResolved {
        /// The grant.
        grant_id: u128,
        /// The state it already reached.
        state: GrantState,
    },
    /// No grant exists for this id.
    UnknownGrant(u128),
    /// The grant has not posted, so there is no posted balance
    /// to claw back — a refund of a grant that never posted is
    /// the void path ([`GrantService::void`] /
    /// [`GrantService::refund_partial`]), never a clawback.
    NotPosted {
        /// The grant.
        grant_id: u128,
        /// The state the grant is actually in.
        state: GrantState,
    },
    /// The amount is not usable for this operation (a zero grant,
    /// or an amount that is not a proper partial of the grant —
    /// a partial refund or a policy partial vest).
    InvalidAmount {
        /// What was attempted.
        attempted: u128,
        /// The grant's amount (the bound the attempt violated).
        grant_amount: u128,
    },
    /// The store rejected the write.
    Store(GrantStoreError),
    /// The ledger rejected the transfer — a non-verdict failure;
    /// the grant row keeps the attempt and the error for the
    /// reconciler.
    Ledger(LedgerError),
}

impl fmt::Display for GrantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyResolved { grant_id, state } => {
                write!(f, "grant {grant_id} is already resolved ({state})")
            }
            Self::UnknownGrant(id) => write!(f, "grant {id} does not exist"),
            Self::NotPosted { grant_id, state } => {
                write!(
                    f,
                    "grant {grant_id} has not posted (state {state}); nothing to claw back"
                )
            }
            Self::InvalidAmount {
                attempted,
                grant_amount,
            } => write!(
                f,
                "amount {attempted} is not valid against a grant amount of {grant_amount}"
            ),
            Self::Store(err) => write!(f, "{err}"),
            Self::Ledger(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for GrantError {}

impl From<GrantStoreError> for GrantError {
    fn from(err: GrantStoreError) -> Self {
        Self::Store(err)
    }
}

impl From<LedgerError> for GrantError {
    fn from(err: LedgerError) -> Self {
        Self::Ledger(err)
    }
}

/// What a caller asks [`GrantService::register`] to grant.
///
/// The account ids are the factory's derived ids (grant pool on
/// the issuer side, the holder's premium account on the credit
/// side); deriving them is the factory's job, not the service's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterGrant {
    /// The Steam key being redeemed (hashed by the service, never
    /// stored).
    pub steam_key: String,
    /// The user the grant is for.
    pub user_id: String,
    /// The premium-token ledger.
    pub ledger: u32,
    /// Grant amount.
    pub amount: u128,
    /// Debits side: the issuer grant-pool account id.
    pub grant_pool_account_id: u128,
    /// Credits side: the holder's premium account id.
    pub holder_account_id: u128,
}

/// The grant service, over a [`GrantStore`] and a [`PendingLedger`].
///
/// Stateless apart from its id counters: grant ids and transfer
/// ids are allocated monotonically from 1 in their own domains
/// (the production store allocates grant ids from the `grant`
/// table's sequence — migration `0002` — and transfer ids are
/// the caller's `TigerBeetle` ids; the counters here are the
/// in-memory slice's allocator, and the seam is the store/ledger
/// pair, not the counters).
#[derive(Debug)]
pub struct GrantService<S: GrantStore, L: PendingLedger> {
    store: S,
    ledger: L,
    config: GrantConfig,
    next_grant_id: u128,
    next_transfer_id: u128,
}

impl<S: GrantStore, L: PendingLedger> GrantService<S, L> {
    /// A service over the given store, ledger, and config.
    pub const fn new(store: S, ledger: L, config: GrantConfig) -> Self {
        Self {
            store,
            ledger,
            config,
            next_grant_id: 1,
            next_transfer_id: 1,
        }
    }

    /// The store (read access for callers and tests).
    pub const fn store(&self) -> &S {
        &self.store
    }

    /// The ledger (read access for callers and tests).
    pub const fn ledger(&self) -> &L {
        &self.ledger
    }

    /// The ledger, mutably (the reconciler's and tests' handle —
    /// e.g. firing a stub hold's timeout).
    pub const fn ledger_mut(&mut self) -> &mut L {
        &mut self.ledger
    }

    /// Register a grant for a redeemed Steam key (`POST /grants`,
    /// design section 2.4): record the intent, submit the pending
    /// transfer, confirm the row. Idempotent on `steam_key_hash` —
    /// a key redeems once, ever, so a repeat registration returns
    /// the existing grant unchanged instead of holding twice.
    ///
    /// # Errors
    ///
    /// [`GrantError::InvalidAmount`] for a zero amount;
    /// [`GrantError::Ledger`] if the cluster rejects the hold (the
    /// row remains, pending, with the attempt recorded for the
    /// reconciler).
    pub fn register(&mut self, request: &RegisterGrant, now: u64) -> Result<Grant, GrantError> {
        let hash = steam_key_hash(&request.steam_key);
        if let Some(existing) = self.store.grant_by_key_hash(&hash) {
            return Ok(existing);
        }
        if request.amount == 0 {
            return Err(GrantError::InvalidAmount {
                attempted: 0,
                grant_amount: 0,
            });
        }
        let grant_id = self.alloc_grant_id();
        let pending_id = self.alloc_transfer_id();
        let mut grant = Grant {
            id: grant_id,
            steam_key_hash: hash,
            user_id: request.user_id.clone(),
            ledger: request.ledger,
            amount: request.amount,
            state: GrantState::Pending,
            pending_transfer_id: pending_id,
            resolution_transfer_id: None,
            window_started_at: now,
            window_ends_at: now.saturating_add(self.config.window_seconds),
            created_at: now,
            updated_at: now,
            attempt_count: 0,
            last_error: None,
            correction_of: None,
        };
        self.store.insert_grant(&grant)?;
        let spec = self.pending_spec(
            &grant,
            request.grant_pool_account_id,
            request.holder_account_id,
        );
        grant.attempt_count += 1;
        if let Err(err) = self.ledger.create_pending(&spec) {
            grant.last_error = Some(err.to_string());
            grant.updated_at = now;
            self.store.update_grant(&grant)?;
            return Err(GrantError::Ledger(err));
        }
        grant.updated_at = now;
        self.store.update_grant(&grant)?;
        Ok(grant)
    }

    /// Vest a grant (`POST /grants/{id}/vest`): post its hold in
    /// full. The caller owns the window policy — the reconciler
    /// calls this at window close; the service enforces only
    /// at-most-once resolution.
    ///
    /// # Errors
    ///
    /// [`GrantError::UnknownGrant`]; [`GrantError::AlreadyResolved`]
    /// (the `409`) if the grant already resolved — including when
    /// the *ledger* reports the resolution the row had not yet
    /// confirmed, in which case the row is first synced to the
    /// ledger's truth.
    pub fn vest(&mut self, grant_id: u128, now: u64) -> Result<Grant, GrantError> {
        self.resolve(grant_id, now, Resolution::Vest)
    }

    /// Vest a proper part of a grant by policy (`POST
    /// /grants/{id}/vest` with an amount): post `vest_amount` of
    /// the hold; the ledger restores the unposted remainder
    /// (design section 4 — partial posting is for policy, e.g.
    /// grant tiers vesting a fraction at window close). A refund
    /// never takes this path: it voids and reissues via
    /// [`GrantService::refund_partial`], so a refunded amount can
    /// never end up posted.
    ///
    /// # Errors
    ///
    /// [`GrantError::UnknownGrant`]; [`GrantError::AlreadyResolved`]
    /// (the `409`) if the grant already resolved;
    /// [`GrantError::InvalidAmount`] unless
    /// `0 < vest_amount < grant.amount` — posting nothing is a
    /// [`GrantService::void`], posting everything is a plain
    /// [`GrantService::vest`].
    pub fn vest_partial(
        &mut self,
        grant_id: u128,
        vest_amount: u128,
        now: u64,
    ) -> Result<Grant, GrantError> {
        let grant = self
            .store
            .grant(grant_id)
            .ok_or(GrantError::UnknownGrant(grant_id))?;
        if grant.state.is_resolved() {
            return Err(GrantError::AlreadyResolved {
                grant_id,
                state: grant.state,
            });
        }
        if vest_amount == 0 || vest_amount >= grant.amount {
            return Err(GrantError::InvalidAmount {
                attempted: vest_amount,
                grant_amount: grant.amount,
            });
        }
        self.resolve(grant_id, now, Resolution::VestPartial(vest_amount))
    }

    /// Void a grant (`POST /grants/{id}/void`): release its hold
    /// in full — the refund path.
    ///
    /// # Errors
    ///
    /// As [`GrantService::vest`].
    pub fn void(&mut self, grant_id: u128, now: u64) -> Result<Grant, GrantError> {
        self.resolve(grant_id, now, Resolution::Void)
    }

    /// Refund a proper part of a pending grant (design section
    /// 4): void the whole hold, then reissue the remainder as a
    /// new pending grant linked by `correction_of`, keeping the
    /// original window. Returns the voided original and the
    /// reissue. Nothing posts, so a refund can never leave posted
    /// credit behind.
    ///
    /// # Errors
    ///
    /// [`GrantError::UnknownGrant`];
    /// [`GrantError::AlreadyResolved`] if the grant already
    /// resolved; [`GrantError::InvalidAmount`] unless
    /// `0 < refund_amount < grant.amount` (a full refund is a
    /// plain [`GrantService::void`]).
    pub fn refund_partial(
        &mut self,
        grant_id: u128,
        refund_amount: u128,
        grant_pool_account_id: u128,
        holder_account_id: u128,
        now: u64,
    ) -> Result<(Grant, Grant), GrantError> {
        let original = self
            .store
            .grant(grant_id)
            .ok_or(GrantError::UnknownGrant(grant_id))?;
        if original.state.is_resolved() {
            return Err(GrantError::AlreadyResolved {
                grant_id,
                state: original.state,
            });
        }
        if refund_amount == 0 || refund_amount >= original.amount {
            return Err(GrantError::InvalidAmount {
                attempted: refund_amount,
                grant_amount: original.amount,
            });
        }
        let voided = self.resolve(grant_id, now, Resolution::Void)?;
        let reissue_id = self.alloc_grant_id();
        let reissue_pending_id = self.alloc_transfer_id();
        let mut reissue = Grant {
            id: reissue_id,
            steam_key_hash: voided.steam_key_hash,
            user_id: voided.user_id.clone(),
            ledger: voided.ledger,
            amount: voided.amount - refund_amount,
            state: GrantState::Pending,
            pending_transfer_id: reissue_pending_id,
            resolution_transfer_id: None,
            window_started_at: voided.window_started_at,
            window_ends_at: voided.window_ends_at,
            created_at: now,
            updated_at: now,
            attempt_count: 0,
            last_error: None,
            correction_of: Some(voided.id),
        };
        self.store.insert_grant(&reissue)?;
        let spec = self.reissue_spec(&reissue, &voided, grant_pool_account_id, holder_account_id);
        reissue.attempt_count += 1;
        if let Err(err) = self.ledger.create_pending(&spec) {
            reissue.last_error = Some(err.to_string());
            reissue.updated_at = now;
            self.store.update_grant(&reissue)?;
            return Err(GrantError::Ledger(err));
        }
        reissue.updated_at = now;
        self.store.update_grant(&reissue)?;
        Ok((voided, reissue))
    }

    /// Refund a posted grant late — a refund that arrives
    /// after the hold posted (design section 4's last row) — by
    /// clawing `refund_amount` of posted balance back from the
    /// holder to the grant pool (design section 6.2).
    ///
    /// The section 6.2 policy adopted here is **clawback then
    /// debt**: if the holder's posted balance still covers the
    /// amount, the clawback transfer settles and the grant is
    /// [`GrantState::ClawbackSettled`]; if the holder already
    /// spent it, the transfer fails `exceeds_credits`, moves
    /// nothing, and the grant is marked [`GrantState::Debt`] —
    /// the bounded-exposure record the gateway reads to refuse
    /// new grants and redemptions for that user until settled.
    /// Debt is an outcome of the refund, not a service failure:
    /// both outcomes return `Ok`. The holder's balance never
    /// goes negative.
    ///
    /// For a policy partial vest ([`GrantService::vest_partial`])
    /// the caller passes the refunded part of the *posted*
    /// fraction: the grant row carries no posted-amount column
    /// (migration `0002`), so the bound enforced here is the
    /// grant amount.
    ///
    /// The write order is intent-first, transfer-second,
    /// confirm-third: the row moves to
    /// [`GrantState::ClawbackPending`] before the transfer is
    /// submitted, so a crash in between leaves a row the
    /// reconciler can see and retry — a retry re-enters here
    /// against the `ClawbackPending` row. A non-verdict ledger
    /// failure leaves the row `ClawbackPending` with the attempt
    /// and the error recorded. (A retry allocates a fresh
    /// transfer id; the production ledger deduplicates a
    /// retried clawback by the grant id in `user_data_128`, the
    /// same lookup that resolves the section 6.3 drift case.)
    ///
    /// # Errors
    ///
    /// [`GrantError::UnknownGrant`];
    /// [`GrantError::NotPosted`] if the grant is still pending
    /// (a refund before vest is the void path);
    /// [`GrantError::AlreadyResolved`] if the grant resolved
    /// any other way, or its clawback already finished —
    /// settled or debt (the `409` outcome);
    /// [`GrantError::InvalidAmount`] unless
    /// `0 < refund_amount <= grant.amount` (a zero refund is no
    /// refund; more than the grant was never posted);
    /// [`GrantError::Ledger`] on a non-verdict ledger failure.
    pub fn refund_late(
        &mut self,
        grant_id: u128,
        refund_amount: u128,
        grant_pool_account_id: u128,
        holder_account_id: u128,
        now: u64,
    ) -> Result<Grant, GrantError> {
        let mut grant = self
            .store
            .grant(grant_id)
            .ok_or(GrantError::UnknownGrant(grant_id))?;
        match grant.state {
            GrantState::Pending => {
                return Err(GrantError::NotPosted {
                    grant_id,
                    state: grant.state,
                });
            }
            GrantState::Posted | GrantState::ClawbackPending => {}
            state => {
                return Err(GrantError::AlreadyResolved { grant_id, state });
            }
        }
        if refund_amount == 0 || refund_amount > grant.amount {
            return Err(GrantError::InvalidAmount {
                attempted: refund_amount,
                grant_amount: grant.amount,
            });
        }
        // Intent first: the attempt is on the row before any
        // money moves.
        grant.state = GrantState::ClawbackPending;
        grant.attempt_count += 1;
        grant.updated_at = now;
        self.store.update_grant(&grant)?;
        // Transfer second.
        let spec = ClawbackTransferSpec {
            id: self.alloc_transfer_id(),
            debit_account_id: holder_account_id,
            credit_account_id: grant_pool_account_id,
            amount: refund_amount,
            ledger: grant.ledger,
            code: CLAWBACK_TRANSFER_CODE,
            user_data_128: grant.id,
            user_data_64: steam_key_hash_u64(&grant.steam_key_hash),
        };
        match self.ledger.clawback(&spec) {
            Ok(()) => {
                grant.state = GrantState::ClawbackSettled;
                grant.last_error = None;
            }
            Err(err @ LedgerError::ExceedsCredits(_)) => {
                // Section 6.2: the debt record *is* the outcome;
                // the error string stays on the row as the
                // recorded reason.
                grant.state = GrantState::Debt;
                grant.last_error = Some(err.to_string());
            }
            Err(err) => {
                grant.last_error = Some(err.to_string());
                grant.updated_at = now;
                self.store.update_grant(&grant)?;
                return Err(GrantError::Ledger(err));
            }
        }
        // Confirm third.
        grant.updated_at = now;
        self.store.update_grant(&grant)?;
        Ok(grant)
    }

    /// One grant by id (`GET /grants/{id}`).
    #[must_use]
    pub fn get(&self, grant_id: u128) -> Option<Grant> {
        self.store.grant(grant_id)
    }

    /// Every grant for a user (`GET /users/{id}/grants`).
    #[must_use]
    pub fn grants_for_user(&self, user_id: &str) -> Vec<Grant> {
        self.store.grants_for_user(user_id)
    }

    fn resolve(
        &mut self,
        grant_id: u128,
        now: u64,
        resolution: Resolution,
    ) -> Result<Grant, GrantError> {
        let mut grant = self
            .store
            .grant(grant_id)
            .ok_or(GrantError::UnknownGrant(grant_id))?;
        if grant.state.is_resolved() {
            return Err(GrantError::AlreadyResolved {
                grant_id,
                state: grant.state,
            });
        }
        let resolution_id = self.alloc_transfer_id();
        grant.attempt_count += 1;
        let outcome = match resolution {
            Resolution::Vest => {
                self.ledger
                    .post_pending(grant.pending_transfer_id, resolution_id, grant.amount)
            }
            Resolution::VestPartial(amount) => {
                self.ledger
                    .post_pending(grant.pending_transfer_id, resolution_id, amount)
            }
            Resolution::Void => self
                .ledger
                .void_pending(grant.pending_transfer_id, resolution_id),
        };
        match outcome {
            Ok(()) => {
                grant.state = match resolution {
                    Resolution::Vest | Resolution::VestPartial(_) => GrantState::Posted,
                    Resolution::Void => GrantState::Voided,
                };
                grant.resolution_transfer_id = Some(resolution_id);
                grant.last_error = None;
                grant.updated_at = now;
                self.store.update_grant(&grant)?;
                Ok(grant)
            }
            Err(
                err @ (LedgerError::AlreadyPosted(_)
                | LedgerError::AlreadyVoided(_)
                | LedgerError::AlreadyExpired(_)),
            ) => {
                // The ledger resolved the hold but the row never
                // confirmed it (a crash between transfer and
                // confirm): sync the row to the ledger's truth,
                // then answer with the same 409 outcome.
                grant.state = match err {
                    LedgerError::AlreadyPosted(_) => GrantState::Posted,
                    LedgerError::AlreadyVoided(_) => GrantState::Voided,
                    _ => GrantState::Expired,
                };
                grant.updated_at = now;
                self.store.update_grant(&grant)?;
                Err(GrantError::AlreadyResolved {
                    grant_id,
                    state: grant.state,
                })
            }
            Err(err) => {
                grant.last_error = Some(err.to_string());
                grant.updated_at = now;
                self.store.update_grant(&grant)?;
                Err(GrantError::Ledger(err))
            }
        }
    }

    fn pending_spec(
        &self,
        grant: &Grant,
        grant_pool_account_id: u128,
        holder_account_id: u128,
    ) -> PendingTransferSpec {
        PendingTransferSpec {
            id: grant.pending_transfer_id,
            debit_account_id: grant_pool_account_id,
            credit_account_id: holder_account_id,
            amount: grant.amount,
            ledger: grant.ledger,
            code: GRANT_TRANSFER_CODE,
            user_data_128: grant.id,
            user_data_64: steam_key_hash_u64(&grant.steam_key_hash),
            timeout_seconds: self
                .config
                .window_seconds
                .saturating_add(self.config.timeout_slack_seconds),
        }
    }

    fn reissue_spec(
        &self,
        reissue: &Grant,
        original: &Grant,
        grant_pool_account_id: u128,
        holder_account_id: u128,
    ) -> PendingTransferSpec {
        // The reissue holds the remainder of the original grant's
        // value between the same two accounts (the caller
        // re-derives them through the factory, as at register —
        // the grant row carries no account columns, design section
        // 2.3). Its timeout covers the *remaining* window: the
        // refund window is anchored at the original redemption.
        let remaining_window = original
            .window_ends_at
            .saturating_sub(original.window_started_at);
        PendingTransferSpec {
            id: reissue.pending_transfer_id,
            debit_account_id: grant_pool_account_id,
            credit_account_id: holder_account_id,
            amount: reissue.amount,
            ledger: reissue.ledger,
            code: GRANT_TRANSFER_CODE,
            user_data_128: reissue.id,
            user_data_64: steam_key_hash_u64(&reissue.steam_key_hash),
            timeout_seconds: remaining_window.saturating_add(self.config.timeout_slack_seconds),
        }
    }

    const fn alloc_grant_id(&mut self) -> u128 {
        let id = self.next_grant_id;
        self.next_grant_id += 1;
        id
    }

    const fn alloc_transfer_id(&mut self) -> u128 {
        let id = self.next_transfer_id;
        self.next_transfer_id += 1;
        id
    }
}

/// Which resolution a `resolve` call performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Resolution {
    Vest,
    VestPartial(u128),
    Void,
}

/// Forwarding so a caller can keep a handle to the store (the
/// gateway wiring and the acceptance tests both do).
impl<T: GrantStore + ?Sized> GrantStore for &mut T {
    fn insert_grant(&mut self, grant: &Grant) -> Result<(), GrantStoreError> {
        (**self).insert_grant(grant)
    }

    fn update_grant(&mut self, grant: &Grant) -> Result<(), GrantStoreError> {
        (**self).update_grant(grant)
    }

    fn grant(&self, id: u128) -> Option<Grant> {
        (**self).grant(id)
    }

    fn grant_by_key_hash(&self, hash: &[u8; 32]) -> Option<Grant> {
        (**self).grant_by_key_hash(hash)
    }

    fn grants_for_user(&self, user_id: &str) -> Vec<Grant> {
        (**self).grants_for_user(user_id)
    }
}

/// Forwarding so a caller can keep a handle to the ledger.
impl<T: PendingLedger + ?Sized> PendingLedger for &mut T {
    fn create_pending(&mut self, spec: &PendingTransferSpec) -> Result<(), LedgerError> {
        (**self).create_pending(spec)
    }

    fn post_pending(
        &mut self,
        pending_id: u128,
        resolution_id: u128,
        amount: u128,
    ) -> Result<(), LedgerError> {
        (**self).post_pending(pending_id, resolution_id, amount)
    }

    fn void_pending(&mut self, pending_id: u128, resolution_id: u128) -> Result<(), LedgerError> {
        (**self).void_pending(pending_id, resolution_id)
    }

    fn clawback(&mut self, spec: &ClawbackTransferSpec) -> Result<(), LedgerError> {
        (**self).clawback(spec)
    }
}
