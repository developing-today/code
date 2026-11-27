//! F5 proof of concept (#2321): the currency control plane of the
//! #2062 design, composed end to end over a single currency.
//!
//! **This is a PoC, not the permanent currency service.** It wires
//! the three landed slices together exactly once, along the fixed
//! pipeline of design §4.3:
//!
//! - **F1, the registry** (#2317, `id-currency-registry`): ledger
//!   allocation and the currency record — which u32 means what.
//! - **F2, the account factory** (#2318, same crate): deterministic
//!   account IDs and creation-fixed flags, driven here through its
//!   [`AccountCluster`] seam onto the #2289 ledger seam
//!   ([`crate::currency_poc::ledger::LedgerBackend`]), so the same
//!   service runs against the in-memory backend in unit tests and a
//!   real TigerBeetle cluster in the live tests.
//! - **F3, the gateway envelope** (#2319, [`super::gateway`]):
//!   signature verification (`verify_strict` only), device → account
//!   resolution, registry authorization, and the deterministic
//!   idempotency ID.
//!
//! What this module adds is steps 6–9 of §4.3, which no slice owned:
//! building the batch (factory `ensure_account`, a transfer whose
//! `user_data_128` stamps the signed graph object recording the
//! business event, §7.1), submitting it once, and returning a signed
//! receipt. The holder account carries the factory's
//! `debits_must_not_exceed_credits` flag, so an overdraft is rejected
//! by the cluster — the gateway never reads a balance before
//! submitting, so there is no check-then-write to race.
//!
//! ## Where the PoC guessed (the #2321 write-up)
//!
//! The design and the three slices disagree or stay silent at the
//! seams; every place this composition had to decide is listed here
//! for the review round.
//!
//! 1. **Currency key ↔ registry id.** F3 addresses a currency by
//!    `[u8; 32]`; F1 by a string `currency_id`. The PoC takes the 32
//!    bytes to be the graph [`CurrencyId::compact_hash`] and keeps a
//!    hash → `currency_id` index in the service. The design never
//!    names this mapping.
//! 2. **Minting the `currency_id`.** F3's `register_currency` body
//!    carries no `currency_id`, and §2.3 says the registry mints it
//!    but gives no scheme. The PoC derives
//!    `cur-<16 hex>` from BLAKE3 over
//!    `issuer ‖ name ‖ symbol ‖ scale ‖ cap`, so a client can
//!    predict its currency's id and a duplicate registration fails
//!    in F1 as a duplicate id.
//! 3. **The definition object arrives out of envelope.**
//!    §2.4 makes the signed graph definition the canonical record,
//!    but it names the ledger the registry allocates *during*
//!    registration, so it can only be signed afterwards — and
//!    neither F3's op set nor F1's API (no `graph_object_id`
//!    update) carries it. The PoC adds
//!    [`GatewayPoc::record_definition`] as a separate step and
//!    leaves the registry row's `graph_object_id` unset; the
//!    service's graph log carries the linkage instead.
//! 4. **`user_data_128` is a truncation.** Graph object ids are
//!    32-byte hashes; `user_data_128` holds 16 bytes. The PoC
//!    stamps the first 16 bytes of the event id (read little-endian,
//!    the factory derivation's convention) and resolves stamp →
//!    event in the graph log. A production design must answer the
//!    collision question this sidesteps.
//! 5. **Minters and issued supply are service-tracked.** F1's
//!    `Store` trait inserts grants but cannot read them back, and
//!    holds no supply counter, so the F3 authorization view keeps
//!    both in service memory. A production adapter over Postgres
//!    reads the `currency_issuer` table and the issuer-account
//!    counters (F4's audit inputs). Supply counts a fresh posting
//!    once; an idempotent replay returns the original receipt
//!    without re-counting, and a restarted service recounts from
//!    zero until F4 reconciliation rebuilds it.
//! 6. **Liquidity accounts are not created.** Registration calls
//!    `ensure_currency_accounts` with no directions: issuer, fee,
//!    and burn exist, but no fee is charged on the basic path (the
//!    design does not price it) and conversions are out of scope.
//! 7. **Account `code`.** F2's `AccountSpec` carries no TigerBeetle
//!    `code`; the PoC writes the factory's kind byte
//!    (issuer 1 … holder 5) as the chart-of-accounts code.
//! 8. **Receipt shape.** §4.3 step 9 names the receipt's contents
//!    but no encoding or signing scheme. The PoC signs canonical
//!    CBOR with the gateway's ed25519 key under the domain tag
//!    `id.currency.receipt.v1`, and verifies with `verify_strict`.
//! 9. **Graph events are client-signed and client-supplied.** The
//!    gateway holds no user keys (§1.3), so the issuer or holder
//!    signs the graph object and presents it with the envelope;
//!    the service checks author, kind, payload hash, and field
//!    agreement with the op before stamping its id. §5's
//!    walkthrough shows the gateway writing to the graph after
//!    commit, which needs key custody or a delegation the design
//!    does not specify.
//! 10. **Graph chain state is not validated.** Sequence/backlink
//!     continuity of a presented event is not checked here; chain
//!     semantics belong to the fold (#2290). Events are accepted
//!     as standalone signed objects whose fields match the op.
//! 11. **Abuse controls are absent.** §4.3 step 6 (budgets and
//!     rate limits, #2032/#2033) is not implemented; the pipeline
//!     runs steps 1–5 and 7–9 only.
//! 12. **Minter issuance has no graph object.** The graph
//!     contracts bind `token.issue` events to the issuer's key, so
//!     a granted minter — authorized by F3 to mint — cannot
//!     produce the graph object this PoC requires per write.
//!     Until minter-signed issue events exist, the PoC's flow
//!     effectively limits issuance to the issuer.
//! 13. **Replays short-circuit before authorization.** F3's
//!     `authorize` is not replay-aware: it re-checks the supply
//!     cap against current issued supply, so a retry of an
//!     already-posted issue is refused once later issuance has
//!     moved supply — the PoC's first flow test hit exactly this.
//!     §4.3's own ordering (idempotency is step 2, authorization
//!     step 5) puts the replay answer first, so the service keeps
//!     a receipt store keyed by (device, op, client request id)
//!     — the pre-authorization projection of the idempotency ID —
//!     and returns the stored receipt. A production store is
//!     durable and keyed by the idempotency ID itself.

use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use ed25519_dalek::Signer as _;
use id_currency_registry::{
    AccountCluster, AccountKind, AccountSpec, ClusterError, CreateOutcome,
    CurrencyRecord as RegistryCurrency, CurrencyStatus, Factory, FactoryError, InMemoryStore,
    RegisterAccount, RegisterCurrency as RegistryRegister, Registry, RegistryError, Store,
    SupplyPolicy, factory::transfer_code,
};
use p2panda_core::Hash;
use serde::{Deserialize, Serialize};

use crate::chain::{ChainError, Event};
use crate::currency::author_id;
use crate::currency::gateway::{
    Authorized, CurrencyOp, CurrencyRecord as GatewayCurrency, CurrencyRegistry, FixedDevices,
    GatewayError, GrantMinter, Issue, RegisterCurrency, Transfer, admit, verify_envelope,
};
use crate::currency::graph::{
    BodyError, CurrencyBinding, CurrencyDefinition, CurrencyId, TokenError, TokenIssue,
    TokenTransfer,
};
use crate::currency_poc::ledger::{
    Account as LedgerAccount, AccountError, AccountFlags as LedgerFlags, LedgerBackend,
    Transfer as LedgerTransfer, TransferError,
};
use crate::directory_auth::{ReplayGuard, Signed};
use crate::hosted_binding::hex_encode;

/// Everything that can refuse a PoC request, typed so a caller can
/// tell a forged envelope from a registry refusal from the
/// cluster's own verdict.
#[derive(Debug, thiserror::Error)]
pub enum PocError {
    /// The F3 pipeline refused the request (envelope, window,
    /// replay, or a §4.2 authorization check).
    #[error(transparent)]
    Gateway(#[from] GatewayError),
    /// The F1 registry refused (duplicate id or symbol, unknown
    /// currency, non-issuer grant).
    #[error(transparent)]
    Registry(#[from] RegistryError),
    /// The F2 factory could not reach the cluster.
    #[error(transparent)]
    Factory(#[from] FactoryError),
    /// The cluster's verdict on the submitted transfer, verbatim —
    /// an overdraft arrives here as
    /// [`TransferError::DebitsMustNotExceedCredits`], decided by
    /// the cluster, never by a gateway pre-check.
    #[error("cluster refused the transfer: {0:?}")]
    Cluster(TransferError),
    /// The ledger backend failed outside a per-item verdict.
    #[error("ledger backend failure: {0}")]
    Backend(String),
    /// The presented graph object does not match the request
    /// (wrong kind, author, body hash, or fields).
    #[error("graph object does not match the request: {0}")]
    GraphMismatch(&'static str),
    /// The request names a currency key the gateway has no
    /// definition indexed for.
    #[error("currency is not known to the gateway")]
    UnknownCurrencyKey,
    /// The op is authorized by F3 but outside this PoC's scope.
    #[error("operation is outside the PoC scope: {0}")]
    Unsupported(&'static str),
    /// A graph body failed its own validation.
    #[error(transparent)]
    Token(#[from] TokenError),
    /// A graph body failed to encode or decode.
    #[error(transparent)]
    Body(#[from] BodyError),
    /// A presented graph event's bytes failed chain verification.
    #[error(transparent)]
    Chain(#[from] ChainError),
}

/// The signed graph objects the gateway has stamped into ledger
/// writes, by their `user_data_128` stamp (write-up item 4).
#[derive(Debug, Default)]
pub struct GraphLog {
    events: BTreeMap<u128, Event>,
    bodies: BTreeMap<u128, Vec<u8>>,
}

impl GraphLog {
    /// The `user_data_128` stamp for a graph event: the first 16
    /// bytes of its id, read little-endian.
    #[must_use]
    pub fn stamp_of(event: &Event) -> u128 {
        let mut bytes = [0_u8; 16];
        bytes.copy_from_slice(&event.id.as_bytes()[..16]);
        u128::from_le_bytes(bytes)
    }

    /// Resolve a stamp read back from the ledger to the graph
    /// object it records: the event and its body bytes.
    #[must_use]
    pub fn resolve(&self, stamp: u128) -> Option<(&Event, &[u8])> {
        Some((
            self.events.get(&stamp)?,
            self.bodies.get(&stamp)?.as_slice(),
        ))
    }

    pub(crate) fn record(&mut self, event: &Event, body: &[u8]) -> u128 {
        let stamp = Self::stamp_of(event);
        self.events.insert(stamp, event.clone());
        self.bodies.insert(stamp, body.to_vec());
        stamp
    }
}

/// The receipt body (§4.3 step 9): the idempotency ID, the ledger
/// transfer ID, and the graph object id, so a client or auditor can
/// reconcile the ledger write against the graph.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    /// The op that produced this receipt.
    pub op: String,
    /// The account that acted.
    pub account: String,
    /// The deterministic idempotency ID (hex).
    pub idempotency: String,
    /// The registry `currency_id`, once registered.
    pub currency_id: Option<String>,
    /// The graph currency key (compact hash, hex), once the
    /// definition is recorded.
    pub currency: Option<String>,
    /// The TigerBeetle ledger, once allocated.
    pub ledger_id: Option<u32>,
    /// The ledger transfer ID (hex); the idempotency ID again,
    /// because the gateway submits under it (§4.3 step 2).
    pub transfer_id: Option<String>,
    /// The graph object recording this write (event id, hex).
    pub graph_object: Option<String>,
}

impl Receipt {
    /// The canonical CBOR body the gateway signs.
    ///
    /// # Errors
    ///
    /// Fails if the receipt does not serialize canonically.
    pub fn body(&self) -> Result<Vec<u8>, PocError> {
        Ok(super::graph::canonical_body(self)?)
    }

    /// Decode a canonical receipt body.
    ///
    /// # Errors
    ///
    /// Fails if the bytes are not the canonical encoding of a
    /// receipt.
    pub fn from_body(bytes: &[u8]) -> Result<Self, PocError> {
        Ok(super::graph::decode_body(bytes)?)
    }
}

/// A [`Receipt`] with the gateway's signature over its canonical
/// body (write-up item 8).
#[derive(Clone, Debug)]
pub struct SignedReceipt {
    /// The receipt itself.
    pub receipt: Receipt,
    /// The exact bytes that were signed.
    pub body: Vec<u8>,
    /// The gateway's ed25519 signature over the domain-tagged body.
    pub signature: ed25519_dalek::Signature,
}

/// The domain tag prefixed to a receipt body before signing.
const RECEIPT_DOMAIN: &[u8] = b"id.currency.receipt.v1\n";

fn receipt_message(body: &[u8]) -> Vec<u8> {
    let mut message = RECEIPT_DOMAIN.to_vec();
    message.extend_from_slice(body);
    message
}

/// Verify a signed receipt against the gateway's verifying key:
/// the signature must be a strict verification over the body, and
/// the body must decode to the receipt it travels with.
#[must_use]
pub fn verify_receipt(verifying_key: &ed25519_dalek::VerifyingKey, signed: &SignedReceipt) -> bool {
    if verifying_key
        .verify_strict(&receipt_message(&signed.body), &signed.signature)
        .is_err()
    {
        return false;
    }
    Receipt::from_body(&signed.body).is_ok_and(|decoded| decoded == signed.receipt)
}

fn u128_hex(value: u128) -> String {
    hex_encode(&value.to_be_bytes())
}

/// The F3 authorization view over the F1 registry (write-up
/// items 1 and 5). Shared with the production gateway service
/// ([`super::gateway_service`]) so the two compositions read the
/// registry identically.
pub(crate) struct RegistryView<'a> {
    registry: &'a Registry<InMemoryStore>,
    currency_ids: &'a BTreeMap<[u8; 32], String>,
    minters: &'a BTreeMap<String, HashSet<String>>,
    issued: &'a BTreeMap<String, u128>,
}

impl<'a> RegistryView<'a> {
    pub(crate) fn new(
        registry: &'a Registry<InMemoryStore>,
        currency_ids: &'a BTreeMap<[u8; 32], String>,
        minters: &'a BTreeMap<String, HashSet<String>>,
        issued: &'a BTreeMap<String, u128>,
    ) -> Self {
        Self {
            registry,
            currency_ids,
            minters,
            issued,
        }
    }
}

impl CurrencyRegistry for RegistryView<'_> {
    fn currency(&self, key: &[u8; 32]) -> Option<GatewayCurrency> {
        let currency_id = self.currency_ids.get(key)?;
        let record = self.registry.resolve(currency_id)?;
        Some(GatewayCurrency {
            issuer_id: record.issuer_id.clone(),
            active: record.status == CurrencyStatus::Active,
            cap: record.supply_policy.cap.unwrap_or(u128::MAX),
            issued: self.issued.get(currency_id).copied().unwrap_or(0),
        })
    }

    fn is_minter(&self, key: &[u8; 32], account: &str) -> bool {
        self.currency_ids
            .get(key)
            .and_then(|currency_id| self.minters.get(currency_id))
            .is_some_and(|set| set.contains(account))
    }
}

/// The F2 factory's cluster seam over the #2289 ledger seam
/// (write-up item 7).
pub(crate) struct BackendCluster<'a, B: LedgerBackend> {
    backend: &'a mut B,
}

impl<'a, B: LedgerBackend> BackendCluster<'a, B> {
    pub(crate) fn new(backend: &'a mut B) -> Self {
        Self { backend }
    }
}

impl<B: LedgerBackend> AccountCluster for BackendCluster<'_, B> {
    fn create_account(&mut self, spec: &AccountSpec) -> Result<CreateOutcome, ClusterError> {
        let account = LedgerAccount {
            id: spec.id,
            ledger: spec.ledger_id,
            code: u16::from(spec.kind.code()),
            flags: LedgerFlags {
                debits_must_not_exceed_credits: spec.flags.debits_must_not_exceed_credits,
                credits_must_not_exceed_debits: spec.flags.credits_must_not_exceed_debits,
                history: spec.flags.history,
            },
        };
        match self.backend.create_accounts(&[account]).pop() {
            Some(Ok(())) => Ok(CreateOutcome::Created),
            Some(Err(AccountError::Exists)) => Ok(CreateOutcome::Exists),
            Some(Err(error)) => Err(ClusterError {
                message: format!("{error:?}"),
            }),
            None => Err(ClusterError {
                message: "ledger backend returned no account verdict".to_owned(),
            }),
        }
    }
}

/// The composed PoC gateway: one logical writer holding the F1
/// registry, an F2-driven ledger backend, the F3 pipeline state,
/// and the graph log.
pub struct GatewayPoc<B: LedgerBackend> {
    registry: Registry<InMemoryStore>,
    backend: B,
    devices: FixedDevices,
    replays: ReplayGuard,
    gateway_key: ed25519_dalek::SigningKey,
    currency_ids: BTreeMap<[u8; 32], String>,
    minters: BTreeMap<String, HashSet<String>>,
    issued: BTreeMap<String, u128>,
    indexed_accounts: HashSet<u128>,
    graph: GraphLog,
    receipts: BTreeMap<(String, &'static str, u128), SignedReceipt>,
}

impl<B: LedgerBackend> GatewayPoc<B> {
    /// A gateway over `backend`, signing receipts with
    /// `gateway_key`. The replay guard starts at boot 0 with an
    /// empty window, as the F3 fixtures construct it.
    #[must_use]
    pub fn new(backend: B, gateway_key: ed25519_dalek::SigningKey) -> Self {
        Self {
            registry: Registry::new(InMemoryStore::new()),
            backend,
            devices: FixedDevices::new(),
            replays: ReplayGuard::after_boot(0, Duration::ZERO),
            gateway_key,
            currency_ids: BTreeMap::new(),
            minters: BTreeMap::new(),
            issued: BTreeMap::new(),
            indexed_accounts: HashSet::new(),
            graph: GraphLog::default(),
            receipts: BTreeMap::new(),
        }
    }

    /// Attach a device (public key, lower-case hex) to the account
    /// it acts for (design §4.1b).
    pub fn attach_device(&mut self, device: &str, account: &str) {
        self.devices.attach(device, account);
    }

    /// The gateway's receipt-verifying key.
    #[must_use]
    pub fn verifying_key(&self) -> ed25519_dalek::VerifyingKey {
        self.gateway_key.verifying_key()
    }

    /// The graph log of stamped objects.
    #[must_use]
    pub const fn graph(&self) -> &GraphLog {
        &self.graph
    }

    /// The ledger backend, for reconciling writes (transfer and
    /// balance lookups by id or stamp).
    #[must_use]
    pub const fn backend(&self) -> &B {
        &self.backend
    }

    /// Run the F4 reconciliation and supply audit (issue #2320,
    /// design section 7.3) over every currency this gateway
    /// serves: per currency, the supply audit and the linkage /
    /// amount checks over the most recent `recent_limit`
    /// transfers. Report-only — this reads the registry, the
    /// ledger, and the graph log, and changes none of them.
    #[must_use]
    pub fn reconcile(&self, recent_limit: usize) -> crate::currency::reconcile::ReconReport {
        let mut report = crate::currency::reconcile::ReconReport::default();
        for key in self.currency_ids.keys() {
            let Some(record) = self.registry_record(key) else {
                continue;
            };
            let (recon, findings) = crate::currency::reconcile::audit_currency(
                &record,
                &self.backend,
                &self.graph,
                recent_limit,
            );
            report.currencies.push(recon);
            report.findings.extend(findings);
        }
        report
    }

    /// The registry record for a currency key, once its definition
    /// is recorded.
    #[must_use]
    pub fn registry_record(&self, key: &[u8; 32]) -> Option<RegistryCurrency> {
        self.currency_ids
            .get(key)
            .and_then(|currency_id| self.registry.resolve(currency_id).cloned())
    }

    /// The spendable balance (credits − debits posted) of `owner`'s
    /// holder account in the currency named by `key`, read from the
    /// ledger — never consulted on the write path.
    #[must_use]
    pub fn holder_balance(&self, key: &[u8; 32], owner: &str) -> Option<u128> {
        let record = self.registry_record(key)?;
        let spec =
            id_currency_registry::account_spec(record.ledger_id, AccountKind::Holder, owner, "");
        let balance = self.backend.account_balance(spec.id)?;
        Some(balance.spendable())
    }

    /// Record a currency's signed definition object (write-up
    /// item 3): the issuer's canonical claim (§2.4), signed after
    /// registration allocated the ledger it names. Indexes the
    /// currency key for later envelopes and files the object in
    /// the graph log.
    ///
    /// # Errors
    ///
    /// Fails if the event does not verify, is not a definition, its
    /// body hash or author disagrees, the definition names no
    /// ledger or a ledger no registration allocated, or the
    /// definition's fields disagree with the registry record.
    pub fn record_definition(
        &mut self,
        event: &Event,
        body: &[u8],
    ) -> Result<CurrencyId, PocError> {
        let event = crate::chain::decode(&event.bytes)?;
        if event.header.extensions.kind != super::graph::DEFINE_KIND {
            return Err(PocError::GraphMismatch(
                "event is not a currency definition",
            ));
        }
        if event.header.payload_hash != Some(Hash::digest(body)) {
            return Err(PocError::GraphMismatch("definition body hash"));
        }
        let definition = CurrencyDefinition::from_body(body)?;
        let CurrencyBinding::Ledger { ledger } = definition.binding else {
            return Err(PocError::GraphMismatch("definition does not name a ledger"));
        };
        let record = self
            .registry
            .store()
            .currency_by_ledger(ledger)
            .ok_or(PocError::UnknownCurrencyKey)?
            .clone();
        let author = author_id(&event);
        if hex_encode(&author) != record.issuer_id {
            return Err(PocError::GraphMismatch(
                "definition author is not the currency issuer",
            ));
        }
        if definition.name != record.name
            || definition.symbol != record.symbol
            || definition.scale != record.scale
            || definition.supply.capped != record.supply_policy.cap.unwrap_or(u128::MAX)
        {
            return Err(PocError::GraphMismatch(
                "definition disagrees with the registry record",
            ));
        }
        let id = CurrencyId::from_definition(author, &definition)?;
        self.currency_ids
            .insert(*id.compact_hash().as_bytes(), record.currency_id);
        self.graph.record(&event, body);
        Ok(id)
    }

    /// Run one signed request through the pipeline (§4.3):
    /// verify, resolve, authorize, then execute the ops this PoC
    /// covers (`register_currency`, `grant_minter`, `issue`,
    /// `transfer`). `graph` carries the client-signed graph object
    /// for the write ops (write-up item 9).
    ///
    /// # Errors
    ///
    /// Returns the typed refusal of whichever step refused:
    /// envelope and authorization failures as
    /// [`PocError::Gateway`], registry and factory failures as
    /// themselves, and the cluster's verdict on a submitted
    /// transfer as [`PocError::Cluster`].
    pub fn handle(
        &mut self,
        signed: &Signed,
        body: &[u8],
        now: u64,
        graph: Option<(&Event, &[u8])>,
    ) -> Result<SignedReceipt, PocError> {
        let verified = verify_envelope(&mut self.replays, signed, body, now)?;
        // §4.3 step 2 (idempotency) precedes step 5 (authorize):
        // a retry of a request this process already served gets
        // its original receipt back without re-authorization
        // (write-up item 13).
        let replay_key = (
            verified.device.clone(),
            verified.request.op.name(),
            verified.request.client_request_id,
        );
        if let Some(stored) = self.receipts.get(&replay_key) {
            return Ok(stored.clone());
        }
        let authorized = {
            let view = RegistryView {
                registry: &self.registry,
                currency_ids: &self.currency_ids,
                minters: &self.minters,
                issued: &self.issued,
            };
            admit(&self.devices, &view, &verified)?
        };
        let receipt = match &verified.request.op {
            CurrencyOp::RegisterCurrency(op) => self.register(&authorized, op),
            CurrencyOp::GrantMinter(op) => self.grant_minter(&authorized, op),
            CurrencyOp::Issue(op) => self.issue(&authorized, op, graph),
            CurrencyOp::Transfer(op) => self.transfer(&authorized, op, graph),
            other => Err(PocError::Unsupported(other.name())),
        }?;
        self.receipts.insert(replay_key, receipt.clone());
        Ok(receipt)
    }

    fn register(
        &mut self,
        authorized: &Authorized,
        op: &RegisterCurrency,
    ) -> Result<SignedReceipt, PocError> {
        let record = self.registry.register_currency(RegistryRegister {
            currency_id: mint_currency_id(&authorized.account, op),
            name: op.name.clone(),
            symbol: op.symbol.clone(),
            scale: op.scale,
            issuer_id: authorized.account.clone(),
            supply_policy: SupplyPolicy {
                mintable: true,
                cap: Some(op.cap),
                policy_uri: None,
            },
            graph_object_id: None,
        })?;
        let accounts = {
            let mut cluster = BackendCluster {
                backend: &mut self.backend,
            };
            Factory::ensure_currency_accounts(&mut cluster, &record, &authorized.account, &[])?
        };
        self.index_account(
            &record,
            accounts.issuer,
            &record.issuer_id,
            AccountKind::Issuer,
            "",
        )?;
        self.index_account(
            &record,
            accounts.fee,
            id_currency_registry::PLATFORM_OWNER_ID,
            AccountKind::Fee,
            "",
        )?;
        self.index_account(
            &record,
            accounts.burn,
            id_currency_registry::PLATFORM_OWNER_ID,
            AccountKind::Burn,
            "",
        )?;
        self.sign_receipt(Receipt {
            op: "register_currency".to_owned(),
            account: authorized.account.clone(),
            idempotency: u128_hex(authorized.idempotency),
            currency_id: Some(record.currency_id.clone()),
            currency: None,
            ledger_id: Some(record.ledger_id),
            transfer_id: None,
            graph_object: None,
        })
    }

    fn grant_minter(
        &mut self,
        authorized: &Authorized,
        op: &GrantMinter,
    ) -> Result<SignedReceipt, PocError> {
        let currency_id = self
            .currency_ids
            .get(&op.currency)
            .cloned()
            .ok_or(PocError::UnknownCurrencyKey)?;
        self.registry
            .grant_minter(&currency_id, op.minter.clone(), authorized.account.clone())?;
        self.minters
            .entry(currency_id.clone())
            .or_default()
            .insert(op.minter.clone());
        let record = self
            .registry
            .resolve(&currency_id)
            .cloned()
            .ok_or(PocError::UnknownCurrencyKey)?;
        self.sign_receipt(Receipt {
            op: "grant_minter".to_owned(),
            account: authorized.account.clone(),
            idempotency: u128_hex(authorized.idempotency),
            currency_id: Some(currency_id),
            currency: Some(hex_encode(&op.currency)),
            ledger_id: Some(record.ledger_id),
            transfer_id: None,
            graph_object: None,
        })
    }

    fn issue(
        &mut self,
        authorized: &Authorized,
        op: &Issue,
        graph: Option<(&Event, &[u8])>,
    ) -> Result<SignedReceipt, PocError> {
        let (event, body) =
            graph.ok_or(PocError::GraphMismatch("issue carries no graph object"))?;
        let record = self.record_for(&op.currency)?;
        let event = check_event(event, body, super::graph::ISSUE_KIND, &record.issuer_id)?;
        let token = TokenIssue::from_body(body)?;
        if token.currency.as_bytes() != &op.currency
            || token.amount != op.amount
            || hex_encode(&token.to) != op.to
        {
            return Err(PocError::GraphMismatch(
                "issue body disagrees with the request",
            ));
        }
        let issuer_tb =
            self.ensure_account(&record, &record.issuer_id.clone(), AccountKind::Issuer)?;
        let holder_tb = self.ensure_account(&record, &op.to.clone(), AccountKind::Holder)?;
        let stamp = GraphLog::stamp_of(&event);
        let fresh = self.submit(LedgerTransfer {
            id: authorized.idempotency,
            debit_account_id: issuer_tb,
            credit_account_id: holder_tb,
            amount: op.amount,
            ledger: record.ledger_id,
            user_data_128: stamp,
            code: transfer_code::ISSUANCE_START,
            linked: false,
        })?;
        if fresh {
            *self.issued.entry(record.currency_id.clone()).or_default() += op.amount;
        }
        self.graph.record(&event, body);
        self.sign_receipt(Receipt {
            op: "issue".to_owned(),
            account: authorized.account.clone(),
            idempotency: u128_hex(authorized.idempotency),
            currency_id: Some(record.currency_id.clone()),
            currency: Some(hex_encode(&op.currency)),
            ledger_id: Some(record.ledger_id),
            transfer_id: Some(u128_hex(authorized.idempotency)),
            graph_object: Some(event.id.to_hex()),
        })
    }

    fn transfer(
        &mut self,
        authorized: &Authorized,
        op: &Transfer,
        graph: Option<(&Event, &[u8])>,
    ) -> Result<SignedReceipt, PocError> {
        let (event, body) =
            graph.ok_or(PocError::GraphMismatch("transfer carries no graph object"))?;
        let record = self.record_for(&op.currency)?;
        let event = check_event(event, body, super::graph::TRANSFER_KIND, &op.from)?;
        let token = TokenTransfer::from_body(body)?;
        if token.currency.as_bytes() != &op.currency
            || token.amount != op.amount
            || hex_encode(&token.to) != op.to
        {
            return Err(PocError::GraphMismatch(
                "transfer body disagrees with the request",
            ));
        }
        let from_tb = self.ensure_account(&record, &op.from.clone(), AccountKind::Holder)?;
        let to_tb = self.ensure_account(&record, &op.to.clone(), AccountKind::Holder)?;
        let stamp = GraphLog::stamp_of(&event);
        self.submit(LedgerTransfer {
            id: authorized.idempotency,
            debit_account_id: from_tb,
            credit_account_id: to_tb,
            amount: op.amount,
            ledger: record.ledger_id,
            user_data_128: stamp,
            code: transfer_code::HOLDER_TRANSFER_START,
            linked: false,
        })?;
        self.graph.record(&event, body);
        self.sign_receipt(Receipt {
            op: "transfer".to_owned(),
            account: authorized.account.clone(),
            idempotency: u128_hex(authorized.idempotency),
            currency_id: Some(record.currency_id.clone()),
            currency: Some(hex_encode(&op.currency)),
            ledger_id: Some(record.ledger_id),
            transfer_id: Some(u128_hex(authorized.idempotency)),
            graph_object: Some(event.id.to_hex()),
        })
    }

    /// The registry record for a currency key.
    fn record_for(&self, key: &[u8; 32]) -> Result<RegistryCurrency, PocError> {
        self.registry_record(key)
            .ok_or(PocError::UnknownCurrencyKey)
    }

    /// Factory-ensure one account on the ledger and record it in
    /// the registry's account index the first time it appears.
    fn ensure_account(
        &mut self,
        record: &RegistryCurrency,
        owner: &str,
        kind: AccountKind,
    ) -> Result<u128, PocError> {
        let tb_id = {
            let mut cluster = BackendCluster {
                backend: &mut self.backend,
            };
            Factory::ensure_account(&mut cluster, record.ledger_id, owner, kind, "")?
        };
        self.index_account(record, tb_id, owner, kind, "")?;
        Ok(tb_id)
    }

    fn index_account(
        &mut self,
        record: &RegistryCurrency,
        tb_id: u128,
        owner: &str,
        kind: AccountKind,
        salt: &str,
    ) -> Result<(), PocError> {
        if !self.indexed_accounts.insert(tb_id) {
            return Ok(());
        }
        self.registry.register_account(RegisterAccount {
            currency_id: record.currency_id.clone(),
            account_id: format!("{tb_id:032x}"),
            owner_id: owner.to_owned(),
            kind,
            salt: salt.to_owned(),
        })?;
        Ok(())
    }

    /// Submit one transfer; `Ok(true)` when it posted fresh,
    /// `Ok(false)` when the cluster reported it already exists
    /// (an idempotent replay of the same request).
    fn submit(&mut self, transfer: LedgerTransfer) -> Result<bool, PocError> {
        match self.backend.create_transfers(&[transfer]).pop() {
            Some(Ok(())) => Ok(true),
            Some(Err(TransferError::Exists)) => Ok(false),
            Some(Err(error)) => Err(PocError::Cluster(error)),
            None => Err(PocError::Backend(
                "ledger backend returned no transfer verdict".to_owned(),
            )),
        }
    }

    fn sign_receipt(&self, receipt: Receipt) -> Result<SignedReceipt, PocError> {
        sign_receipt_with(&self.gateway_key, receipt)
    }
}

/// Sign a receipt with a gateway key (the production gateway
/// service signs its receipts through the same scheme, so a
/// receipt verifies identically whichever composition issued it).
pub(crate) fn sign_receipt_with(
    key: &ed25519_dalek::SigningKey,
    receipt: Receipt,
) -> Result<SignedReceipt, PocError> {
    let body = receipt.body()?;
    let signature = key.sign(&receipt_message(&body));
    Ok(SignedReceipt {
        receipt,
        body,
        signature,
    })
}

/// Verify a presented graph event for a write op: its bytes must
/// decode (signature and canonical form), its kind must be the
/// op's kind, its payload hash must commit to the presented body,
/// and its author must be `expected_author` (an account id, hex).
pub(crate) fn check_event(
    event: &Event,
    body: &[u8],
    kind: &str,
    expected_author: &str,
) -> Result<Event, PocError> {
    let event = crate::chain::decode(&event.bytes)?;
    if event.header.extensions.kind != kind {
        return Err(PocError::GraphMismatch("graph object kind"));
    }
    if event.header.payload_hash != Some(Hash::digest(body)) {
        return Err(PocError::GraphMismatch("graph object body hash"));
    }
    if hex_encode(&author_id(&event)) != expected_author {
        return Err(PocError::GraphMismatch("graph object author"));
    }
    Ok(event)
}

/// The PoC's `currency_id` minting scheme (write-up item 2).
pub(crate) fn mint_currency_id(issuer: &str, op: &RegisterCurrency) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"id.currency.poc.currency-id.v1");
    hasher.update(issuer.as_bytes());
    hasher.update(op.name.as_bytes());
    hasher.update(op.symbol.as_bytes());
    hasher.update(&[op.scale]);
    hasher.update(&op.cap.to_le_bytes());
    let digest = hasher.finalize();
    format!("cur-{}", hex_encode(&digest.as_bytes()[..8]))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::currency::gateway::{
        CurrencyRequest, GrantMinter as GwGrantMinter, Issue as GwIssue,
        RegisterCurrency as GwRegister, Transfer as GwTransfer, idempotency_id, sign_request,
    };
    use crate::currency::graph::{
        CurrencyBinding, SupplyCap, sign_definition, sign_issue, sign_transfer,
    };
    use crate::currency::issuer_id;
    use crate::currency::reconcile::ReconFinding;
    use crate::currency_poc::ledger::InMemoryLedger;
    use p2panda_core::SigningKey as PandaKey;

    const NOW: u64 = 10_000;
    const CAP: u128 = 150;

    /// One actor: a device key (envelope signer) and a separate
    /// account key (graph signer), per design §4.1b.
    struct Actor {
        device_key: ed25519_dalek::SigningKey,
        account_key: PandaKey,
        account: String,
        device: String,
    }

    fn actor(device_seed: u8, account_seed: u8) -> Actor {
        let device_key = ed25519_dalek::SigningKey::from_bytes(&[device_seed; 32]);
        let device = hex_encode(&device_key.verifying_key().to_bytes());
        let account_key = PandaKey::from_bytes(&[account_seed; 32]);
        let account = hex_encode(&issuer_id(&account_key));
        Actor {
            device_key,
            account_key,
            account,
            device,
        }
    }

    struct Fixture<B: LedgerBackend> {
        svc: GatewayPoc<B>,
        gateway_vk: ed25519_dalek::VerifyingKey,
        issuer: Actor,
        alice: Actor,
        bob: Actor,
        mallory: Actor,
        nonce: u64,
    }

    fn fixture_with<B: LedgerBackend>(backend: B) -> Fixture<B> {
        let gateway_key = ed25519_dalek::SigningKey::from_bytes(&[250; 32]);
        let gateway_vk = gateway_key.verifying_key();
        let mut svc = GatewayPoc::new(backend, gateway_key);
        let issuer = actor(1, 11);
        let alice = actor(2, 22);
        let bob = actor(3, 33);
        let mallory = actor(4, 44);
        for a in [&issuer, &alice, &bob, &mallory] {
            svc.attach_device(&a.device, &a.account);
        }
        Fixture {
            svc,
            gateway_vk,
            issuer,
            alice,
            bob,
            mallory,
            nonce: 0,
        }
    }

    fn send<B: LedgerBackend>(
        fx: &mut Fixture<B>,
        device_key: &ed25519_dalek::SigningKey,
        request: &CurrencyRequest,
        graph: Option<(&Event, &[u8])>,
    ) -> Result<SignedReceipt, PocError> {
        fx.nonce += 1;
        let nonce = format!("nonce-{}", fx.nonce);
        let (signed, body) =
            sign_request(device_key, request, NOW, &nonce).expect("test request signs");
        fx.svc.handle(&signed, &body, NOW, graph)
    }

    /// Register the fixture currency and record its definition;
    /// returns the currency id, the graph currency id, and the
    /// definition event.
    fn registered<B: LedgerBackend>(fx: &mut Fixture<B>) -> (SignedReceipt, CurrencyId, Event) {
        let request = CurrencyRequest {
            client_request_id: 1,
            op: CurrencyOp::RegisterCurrency(GwRegister {
                name: "PoC Dollar".to_owned(),
                symbol: "POCD".to_owned(),
                scale: 2,
                cap: CAP,
            }),
        };
        let device_key = fx.issuer.device_key.clone();
        let receipt = send(fx, &device_key, &request, None).expect("register");
        assert_eq!(
            receipt.receipt.ledger_id,
            Some(id_currency_registry::CURRENCY_RANGE_START),
            "the first currency takes the first ledger of the currency range",
        );
        let definition = CurrencyDefinition {
            name: "PoC Dollar".to_owned(),
            symbol: "POCD".to_owned(),
            scale: 2,
            supply: SupplyCap { capped: CAP },
            binding: CurrencyBinding::Ledger {
                ledger: id_currency_registry::CURRENCY_RANGE_START,
            },
            memo: None,
        };
        let (currency, event) =
            sign_definition(&fx.issuer.account_key, 0, None, &definition, vec![])
                .expect("definition signs");
        let body = definition.body().expect("definition body");
        let recorded = fx
            .svc
            .record_definition(&event, &body)
            .expect("definition records");
        assert_eq!(recorded, currency);
        (receipt, currency, event)
    }

    /// One issuer-signed issuance of `amount` to Alice, presented
    /// with its graph object; returns the receipt and the event.
    fn issue_to_alice<B: LedgerBackend>(
        fx: &mut Fixture<B>,
        currency: &CurrencyId,
        definition: &Event,
        client_request_id: u128,
        amount: u128,
        seq: u32,
    ) -> (SignedReceipt, Event) {
        let token = TokenIssue {
            currency: currency.compact_hash(),
            amount,
            to: issuer_id(&fx.alice.account_key),
            memo: None,
        };
        let event = sign_issue(
            &fx.issuer.account_key,
            seq,
            Some(definition.id),
            currency,
            &token,
            vec![],
        )
        .expect("issue event signs");
        let body = token.body().expect("issue body");
        let request = CurrencyRequest {
            client_request_id,
            op: CurrencyOp::Issue(GwIssue {
                currency: *currency.compact_hash().as_bytes(),
                amount,
                to: fx.alice.account.clone(),
            }),
        };
        let device_key = fx.issuer.device_key.clone();
        let receipt = send(fx, &device_key, &request, Some((&event, &body))).expect("issue posts");
        (receipt, event)
    }

    /// The whole acceptance flow over the in-memory backend:
    /// register → define → issue → transfer, with the cluster-side
    /// overdraft and the gateway-side cap and minter refusals.
    #[test]
    fn issue_transfer_flow_over_in_memory_ledger() {
        let mut fx = fixture_with(InMemoryLedger::default());
        let (register_receipt, currency, definition) = registered(&mut fx);
        assert!(verify_receipt(&fx.gateway_vk, &register_receipt));
        let key = *currency.compact_hash().as_bytes();

        // Issue 100 to Alice; the transfer on the ledger carries
        // the issue event's stamp, and the stamp resolves back.
        let (receipt, issue_event) = issue_to_alice(&mut fx, &currency, &definition, 2, 100, 1);
        assert!(verify_receipt(&fx.gateway_vk, &receipt));
        assert_eq!(receipt.receipt.op, "issue");
        assert_eq!(
            receipt.receipt.graph_object.as_deref(),
            Some(issue_event.id.to_hex().as_str()),
        );
        let stamp = GraphLog::stamp_of(&issue_event);
        let stamped = fx.svc.backend().transfers_with_user_data_128(stamp);
        assert_eq!(stamped.len(), 1);
        assert_eq!(stamped[0].amount, 100);
        assert_eq!(stamped[0].code, transfer_code::ISSUANCE_START);
        let (resolved, resolved_body) = fx.svc.graph().resolve(stamp).expect("stamp resolves");
        assert_eq!(resolved.id, issue_event.id);
        assert_eq!(
            TokenIssue::from_body(resolved_body)
                .expect("issue body")
                .amount,
            100,
        );
        assert_eq!(fx.svc.holder_balance(&key, &fx.alice.account), Some(100));

        // Idempotent replay: the same client request returns a
        // receipt and posts nothing new.
        let (replay, _) = issue_to_alice(&mut fx, &currency, &definition, 2, 100, 1);
        assert_eq!(replay.receipt, receipt.receipt);
        assert_eq!(fx.svc.holder_balance(&key, &fx.alice.account), Some(100));

        // The cap is the gateway's (F3 reads issued supply): a
        // second 100 would take supply to 200 over the cap of 150.
        let token = TokenIssue {
            currency: currency.compact_hash(),
            amount: 100,
            to: issuer_id(&fx.alice.account_key),
            memo: None,
        };
        let event = sign_issue(
            &fx.issuer.account_key,
            2,
            Some(issue_event.id),
            &currency,
            &token,
            vec![],
        )
        .expect("second issue signs");
        let body = token.body().expect("body");
        let request = CurrencyRequest {
            client_request_id: 3,
            op: CurrencyOp::Issue(GwIssue {
                currency: key,
                amount: 100,
                to: fx.alice.account.clone(),
            }),
        };
        let device_key = fx.issuer.device_key.clone();
        let error =
            send(&mut fx, &device_key, &request, Some((&event, &body))).expect_err("over cap");
        assert!(
            matches!(error, PocError::Gateway(GatewayError::CapExceeded)),
            "got {error:?}",
        );

        // Transfer 40 Alice → Bob.
        let token = TokenTransfer {
            currency: currency.compact_hash(),
            amount: 40,
            to: issuer_id(&fx.bob.account_key),
            seq_ref: 0,
            memo: None,
        };
        let event =
            sign_transfer(&fx.alice.account_key, 0, None, &token, vec![]).expect("transfer signs");
        let body = token.body().expect("body");
        let request = CurrencyRequest {
            client_request_id: 4,
            op: CurrencyOp::Transfer(GwTransfer {
                currency: key,
                from: fx.alice.account.clone(),
                to: fx.bob.account.clone(),
                amount: 40,
            }),
        };
        let device_key = fx.alice.device_key.clone();
        let receipt =
            send(&mut fx, &device_key, &request, Some((&event, &body))).expect("transfer posts");
        assert!(verify_receipt(&fx.gateway_vk, &receipt));
        let idem = idempotency_id(&fx.alice.account, &request);
        let posted = fx
            .svc
            .backend()
            .lookup_transfer(idem)
            .expect("transfer is on the ledger");
        assert_eq!(posted.user_data_128, GraphLog::stamp_of(&event));
        assert_eq!(posted.code, transfer_code::HOLDER_TRANSFER_START);
        assert_eq!(fx.svc.holder_balance(&key, &fx.alice.account), Some(60));
        assert_eq!(fx.svc.holder_balance(&key, &fx.bob.account), Some(40));

        // Overdraft: Alice holds 60 and tries to send 61. The
        // gateway submits without reading a balance; the cluster
        // refuses, and nothing moves.
        let token = TokenTransfer {
            currency: currency.compact_hash(),
            amount: 61,
            to: issuer_id(&fx.bob.account_key),
            seq_ref: 1,
            memo: None,
        };
        let event = sign_transfer(&fx.alice.account_key, 1, Some(event.id), &token, vec![])
            .expect("overdraft signs");
        let body = token.body().expect("body");
        let request = CurrencyRequest {
            client_request_id: 5,
            op: CurrencyOp::Transfer(GwTransfer {
                currency: key,
                from: fx.alice.account.clone(),
                to: fx.bob.account.clone(),
                amount: 61,
            }),
        };
        let device_key = fx.alice.device_key.clone();
        let error = send(&mut fx, &device_key, &request, Some((&event, &body)))
            .expect_err("overdraft refused");
        assert!(
            matches!(
                error,
                PocError::Cluster(TransferError::DebitsMustNotExceedCredits)
            ),
            "the cluster, not the gateway, refuses the overdraft; got {error:?}",
        );
        assert_eq!(fx.svc.holder_balance(&key, &fx.alice.account), Some(60));
        assert_eq!(fx.svc.holder_balance(&key, &fx.bob.account), Some(40));

        // Mallory is neither issuer nor minter: F3 refuses before
        // anything is built.
        let request = CurrencyRequest {
            client_request_id: 6,
            op: CurrencyOp::Issue(GwIssue {
                currency: key,
                amount: 1,
                to: fx.mallory.account.clone(),
            }),
        };
        let device_key = fx.mallory.device_key.clone();
        let error = send(&mut fx, &device_key, &request, None).expect_err("not a minter");
        assert!(
            matches!(error, PocError::Gateway(GatewayError::NotAMinter)),
            "got {error:?}",
        );

        // Grants are the issuer's alone; then they land.
        let request = CurrencyRequest {
            client_request_id: 7,
            op: CurrencyOp::GrantMinter(GwGrantMinter {
                currency: key,
                minter: fx.alice.account.clone(),
            }),
        };
        let device_key = fx.alice.device_key.clone();
        let error = send(&mut fx, &device_key, &request, None).expect_err("not the issuer");
        assert!(
            matches!(error, PocError::Gateway(GatewayError::NotIssuer)),
            "got {error:?}",
        );
        let device_key = fx.issuer.device_key.clone();
        let receipt = send(&mut fx, &device_key, &request, None).expect("grant lands");
        assert_eq!(receipt.receipt.op, "grant_minter");
    }

    /// A definition that disagrees with the registry record — here
    /// signed before registration, naming a ledger nobody allocated
    /// — is refused, and a receipt with a tampered body does not
    /// verify.
    #[test]
    fn definition_mismatch_and_receipt_tamper_are_refused() {
        let mut fx = fixture_with(InMemoryLedger::default());
        let definition = CurrencyDefinition {
            name: "PoC Dollar".to_owned(),
            symbol: "POCD".to_owned(),
            scale: 2,
            supply: SupplyCap { capped: CAP },
            binding: CurrencyBinding::Ledger { ledger: 77 },
            memo: None,
        };
        let (_currency, event) =
            sign_definition(&fx.issuer.account_key, 0, None, &definition, vec![])
                .expect("definition signs");
        let body = definition.body().expect("body");
        let error = fx
            .svc
            .record_definition(&event, &body)
            .expect_err("unallocated ledger");
        assert!(
            matches!(error, PocError::UnknownCurrencyKey),
            "got {error:?}"
        );

        let (receipt, _currency, _definition) = registered(&mut fx);
        let mut tampered = receipt.clone();
        tampered.body.push(0);
        assert!(!verify_receipt(&fx.gateway_vk, &tampered));
    }

    /// The same flow against a real TigerBeetle cluster: the
    /// acceptance run of #2321. Needs the pinned binary
    /// (`TIGERBEETLE_POC_BINARY` or `tigerbeetle` on PATH).
    #[cfg(feature = "currency-poc-tb")]
    #[test]
    #[ignore = "needs a local TigerBeetle binary; run with --ignored"]
    fn issue_transfer_flow_over_tigerbeetle() {
        use crate::currency_poc::harness::TestCluster;
        use crate::currency_poc::tb_ledger::TigerBeetleLedger;

        let cluster = TestCluster::start(30_031).expect("cluster starts");
        let backend = TigerBeetleLedger::connect(&cluster.address).expect("connect");
        let mut fx = fixture_with(backend);
        let (_register, currency, definition) = registered(&mut fx);
        let key = *currency.compact_hash().as_bytes();

        let (_receipt, issue_event) = issue_to_alice(&mut fx, &currency, &definition, 2, 100, 1);
        let stamp = GraphLog::stamp_of(&issue_event);
        let stamped = fx.svc.backend().transfers_with_user_data_128(stamp);
        assert_eq!(stamped.len(), 1, "the stamped transfer is on the cluster");
        assert_eq!(stamped[0].amount, 100);
        assert_eq!(fx.svc.holder_balance(&key, &fx.alice.account), Some(100));

        let token = TokenTransfer {
            currency: currency.compact_hash(),
            amount: 40,
            to: issuer_id(&fx.bob.account_key),
            seq_ref: 0,
            memo: None,
        };
        let event =
            sign_transfer(&fx.alice.account_key, 0, None, &token, vec![]).expect("transfer signs");
        let body = token.body().expect("body");
        let request = CurrencyRequest {
            client_request_id: 4,
            op: CurrencyOp::Transfer(GwTransfer {
                currency: key,
                from: fx.alice.account.clone(),
                to: fx.bob.account.clone(),
                amount: 40,
            }),
        };
        let device_key = fx.alice.device_key.clone();
        send(&mut fx, &device_key, &request, Some((&event, &body))).expect("transfer posts");
        assert_eq!(fx.svc.holder_balance(&key, &fx.alice.account), Some(60));
        assert_eq!(fx.svc.holder_balance(&key, &fx.bob.account), Some(40));

        // The cluster refuses the overdraft on the real binary too.
        let token = TokenTransfer {
            currency: currency.compact_hash(),
            amount: 61,
            to: issuer_id(&fx.bob.account_key),
            seq_ref: 1,
            memo: None,
        };
        let event = sign_transfer(&fx.alice.account_key, 1, Some(event.id), &token, vec![])
            .expect("overdraft signs");
        let body = token.body().expect("body");
        let request = CurrencyRequest {
            client_request_id: 5,
            op: CurrencyOp::Transfer(GwTransfer {
                currency: key,
                from: fx.alice.account.clone(),
                to: fx.bob.account.clone(),
                amount: 61,
            }),
        };
        let device_key = fx.alice.device_key.clone();
        let error = send(&mut fx, &device_key, &request, Some((&event, &body)))
            .expect_err("overdraft refused");
        assert!(
            matches!(
                error,
                PocError::Cluster(TransferError::DebitsMustNotExceedCredits)
            ),
            "got {error:?}",
        );
        assert_eq!(fx.svc.holder_balance(&key, &fx.alice.account), Some(60));
    }

    /// One Alice -> Bob transfer of `amount` through the gateway,
    /// returning the graph event the transfer was stamped with.
    fn transfer_to_bob(
        fx: &mut Fixture<InMemoryLedger>,
        currency: &CurrencyId,
        client_request_id: u128,
        amount: u128,
        seq: u32,
    ) -> Event {
        let key = *currency.compact_hash().as_bytes();
        let token = TokenTransfer {
            currency: currency.compact_hash(),
            amount,
            to: issuer_id(&fx.bob.account_key),
            seq_ref: u64::from(seq),
            memo: None,
        };
        let event = sign_transfer(&fx.alice.account_key, seq, None, &token, vec![])
            .expect("transfer signs");
        let body = token.body().expect("body");
        let request = CurrencyRequest {
            client_request_id,
            op: CurrencyOp::Transfer(GwTransfer {
                currency: key,
                from: fx.alice.account.clone(),
                to: fx.bob.account.clone(),
                amount,
            }),
        };
        let device_key = fx.alice.device_key.clone();
        send(fx, &device_key, &request, Some((&event, &body))).expect("transfer posts");
        event
    }

    /// Post one transfer straight onto the ledger, bypassing the
    /// gateway entirely: the divergence-injection tool of the F4
    /// tests. The gateway's own writes are all stamped and
    /// supply-consistent, so drift can only arrive this way —
    /// which is exactly the failure mode section 7 exists to
    /// detect.
    fn inject_transfer(
        fx: &mut Fixture<InMemoryLedger>,
        id: u128,
        debit_account_id: u128,
        credit_account_id: u128,
        amount: u128,
        stamp: u128,
    ) {
        let results = fx.svc.backend.create_transfers(&[LedgerTransfer {
            id,
            debit_account_id,
            credit_account_id,
            amount,
            ledger: id_currency_registry::CURRENCY_RANGE_START,
            user_data_128: stamp,
            code: transfer_code::HOLDER_TRANSFER_START,
            linked: false,
        }]);
        assert_eq!(results, vec![Ok(())]);
    }

    /// The factory-derived ledger account id of one actor's
    /// account of `kind` in the fixture currency.
    fn derived_id(fx: &Fixture<InMemoryLedger>, owner: &str, kind: AccountKind) -> u128 {
        id_currency_registry::account_spec(
            id_currency_registry::CURRENCY_RANGE_START,
            kind,
            owner,
            "",
        )
        .id
    }

    /// F4 acceptance (#2320, design section 7.3): a seeded
    /// cluster with a known supply reports a match. Oracle: the
    /// fixture's own posted balances — issue 100 to Alice, move
    /// 40 to Bob; issued 100 must equal Alice 60 + Bob 40 + fee 0
    /// + burn 0, and both transfers' stamps resolve with agreeing
    /// amounts, so the report is clean.
    #[test]
    fn reconcile_reports_match_for_seeded_cluster() {
        let mut fx = fixture_with(InMemoryLedger::default());
        let (_receipt, currency, definition) = registered(&mut fx);
        let key = *currency.compact_hash().as_bytes();
        let (_receipt, _event) = issue_to_alice(&mut fx, &currency, &definition, 2, 100, 1);
        let _event = transfer_to_bob(&mut fx, &currency, 3, 40, 0);
        assert_eq!(fx.svc.holder_balance(&key, &fx.alice.account), Some(60));
        assert_eq!(fx.svc.holder_balance(&key, &fx.bob.account), Some(40));

        let report = fx.svc.reconcile(100);
        assert!(report.is_clean(), "findings: {:?}", report.findings);
        assert_eq!(report.currencies.len(), 1);
        let recon = &report.currencies[0];
        assert_eq!(
            recon.currency_id,
            fx.svc
                .registry_record(&key)
                .expect("registry record")
                .currency_id
        );
        assert_eq!(recon.ledger_id, id_currency_registry::CURRENCY_RANGE_START);
        let supply = recon.supply.as_ref().expect("supply audit ran");
        assert_eq!((supply.issued, supply.accounted), (100, 100));
        assert!(supply.matches());
        assert_eq!(recon.transfers_checked, 2);
        assert_eq!(recon.unstamped_transfers, 0);
    }

    /// Oracle: an off-chart sink receives exactly one unit. The discrepancy
    /// stays one after returning 2^127 units to the issuer, even though both
    /// accounts now have lifetime counters beyond the signed report range.
    #[test]
    fn reconcile_preserves_unit_drift_for_full_width_posted_counters() {
        let mut fx = fixture_with(InMemoryLedger::default());
        let (_, currency, definition) = registered(&mut fx);
        let key = *currency.compact_hash().as_bytes();
        issue_to_alice(&mut fx, &currency, &definition, 2, 1, 1);
        let issuer = derived_id(&fx, &fx.issuer.account, AccountKind::Issuer);
        let holder = derived_id(&fx, &fx.alice.account, AccountKind::Holder);
        let sink = 987_654_321;
        assert_eq!(
            fx.svc.backend.create_accounts(&[LedgerAccount {
                id: sink,
                ledger: id_currency_registry::CURRENCY_RANGE_START,
                code: 9,
                flags: LedgerFlags::default(),
            }]),
            vec![Ok(())]
        );
        let large = 1_u128 << 127;
        inject_transfer(&mut fx, 0x13542_0001, issuer, holder, large, 0);
        inject_transfer(&mut fx, 0x13542_0002, holder, sink, 1, 0);
        let currency_id = fx
            .svc
            .registry_record(&key)
            .expect("registered")
            .currency_id;
        let ledger_id = id_currency_registry::CURRENCY_RANGE_START;
        let report = fx.svc.reconcile(0);
        assert!(report.currencies[0].supply.is_none());
        assert_eq!(
            report.findings,
            vec![ReconFinding::SupplyOutsideReportRange {
                currency_id: currency_id.clone(),
                ledger_id,
                issued: (large + 1).to_string(),
                accounted: large.to_string(),
                difference: "1".to_owned(),
            }],
        );
        let serialized = serde_json::to_string(&report).expect("serialize wide totals");
        let restored: crate::currency::reconcile::ReconReport =
            serde_json::from_str(&serialized).expect("restore wide totals");
        assert_eq!(report, restored);
        assert!(!restored.is_clean());

        inject_transfer(&mut fx, 0x13542_0003, holder, issuer, large, 0);
        let report = fx.svc.reconcile(0);
        assert_eq!(
            report.findings,
            vec![ReconFinding::SupplyMismatch {
                currency_id,
                ledger_id,
                issued: 1,
                accounted: 0,
            }],
        );
        assert_eq!(
            report.currencies[0].supply.as_ref().unwrap().difference(),
            1
        );
    }

    /// F4 acceptance (#2320): a transfer whose `user_data_128`
    /// has no graph object is reported, not silently ignored —
    /// with the currency, the ledger, and the transfer named.
    #[test]
    fn reconcile_reports_unresolved_graph_link() {
        let mut fx = fixture_with(InMemoryLedger::default());
        let (_receipt, currency, definition) = registered(&mut fx);
        let key = *currency.compact_hash().as_bytes();
        let (_receipt, _event) = issue_to_alice(&mut fx, &currency, &definition, 2, 100, 1);

        let issuer_tb = derived_id(&fx, &fx.issuer.account, AccountKind::Issuer);
        let alice_tb = derived_id(&fx, &fx.alice.account, AccountKind::Holder);
        inject_transfer(&mut fx, 0x2320_0001, issuer_tb, alice_tb, 5, 0xdead_beef);

        let report = fx.svc.reconcile(100);
        assert_eq!(
            report.findings,
            vec![ReconFinding::UnresolvedGraphLink {
                currency_id: fx
                    .svc
                    .registry_record(&key)
                    .expect("registry record")
                    .currency_id,
                ledger_id: id_currency_registry::CURRENCY_RANGE_START,
                transfer_id: 0x2320_0001,
                user_data_128: 0xdead_beef,
            }],
        );
    }

    /// F4 acceptance (#2320): an injected divergence between a
    /// transfer and its graph object is reported with the
    /// currency, the ledger id, and the offending object. The
    /// injected transfer reuses the real transfer event's stamp
    /// but moves 7 where the object records 40; the money itself
    /// stays supply-consistent, so amount drift is the only
    /// finding.
    #[test]
    fn reconcile_reports_amount_drift_with_currency_ledger_and_object() {
        let mut fx = fixture_with(InMemoryLedger::default());
        let (_receipt, currency, definition) = registered(&mut fx);
        let key = *currency.compact_hash().as_bytes();
        let (_receipt, _event) = issue_to_alice(&mut fx, &currency, &definition, 2, 100, 1);
        let transfer_event = transfer_to_bob(&mut fx, &currency, 3, 40, 0);

        let issuer_tb = derived_id(&fx, &fx.issuer.account, AccountKind::Issuer);
        let alice_tb = derived_id(&fx, &fx.alice.account, AccountKind::Holder);
        inject_transfer(
            &mut fx,
            0x2320_0002,
            issuer_tb,
            alice_tb,
            7,
            GraphLog::stamp_of(&transfer_event),
        );

        let report = fx.svc.reconcile(100);
        assert_eq!(
            report.findings,
            vec![ReconFinding::AmountDrift {
                currency_id: fx
                    .svc
                    .registry_record(&key)
                    .expect("registry record")
                    .currency_id,
                ledger_id: id_currency_registry::CURRENCY_RANGE_START,
                transfer_id: 0x2320_0002,
                graph_object: transfer_event.id.to_hex(),
                ledger_amount: 7,
                graph_amount: 40,
            }],
        );
    }

    /// F4 acceptance (#2320): supply that leaves the audited
    /// accounts breaks the supply audit, with both sides of the
    /// comparison named. The sink account's code is no audited
    /// kind, so the 10 Alice sends it vanish from `accounted`
    /// while `issued` still counts them.
    #[test]
    fn reconcile_reports_supply_mismatch_when_supply_leaves_audited_accounts() {
        let mut fx = fixture_with(InMemoryLedger::default());
        let (_receipt, currency, definition) = registered(&mut fx);
        let key = *currency.compact_hash().as_bytes();
        let (_receipt, _event) = issue_to_alice(&mut fx, &currency, &definition, 2, 100, 1);

        let sink: u128 = 987_654_321;
        let results = fx.svc.backend.create_accounts(&[LedgerAccount {
            id: sink,
            ledger: id_currency_registry::CURRENCY_RANGE_START,
            code: 9,
            flags: LedgerFlags::default(),
        }]);
        assert_eq!(results, vec![Ok(())]);
        let alice_tb = derived_id(&fx, &fx.alice.account, AccountKind::Holder);
        inject_transfer(&mut fx, 0x2320_0003, alice_tb, sink, 10, 0);

        let report = fx.svc.reconcile(100);
        assert_eq!(
            report.findings,
            vec![ReconFinding::SupplyMismatch {
                currency_id: fx
                    .svc
                    .registry_record(&key)
                    .expect("registry record")
                    .currency_id,
                ledger_id: id_currency_registry::CURRENCY_RANGE_START,
                issued: 100,
                accounted: 90,
            }],
        );
        assert_eq!(report.currencies[0].unstamped_transfers, 1);
    }
}
