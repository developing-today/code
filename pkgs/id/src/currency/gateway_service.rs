//! The production currency gateway service (#5522 slice of design
//! #2043 phase 3; successor tracking on #10070 item 1).
//!
//! [`gateway_poc::GatewayPoc`] composed the control plane once, as
//! a proof of concept, and its write-up named exactly what a
//! production composition still owed:
//!
//! - **Item 13 — durable idempotency.** The PoC's receipt store is
//!   an in-process map keyed by (device, op, client request id);
//!   the write-up's own production answer is "a durable store
//!   keyed by the idempotency ID itself". That store is this
//!   module's [`IntentStore`]: every mutating request the gateway
//!   accepts becomes a [`GatewayIntent`] under its §4.3
//!   idempotency ID, and a retry — in this process or after a
//!   restart — is answered from the record, never re-executed
//!   blind.
//! - **Write-ahead execution (invariant 6).** For the value ops
//!   (`issue`, `transfer`) the intent is persisted in
//!   [`IntentState::Recorded`] state, carrying the exact
//!   pre-generated ledger transfer (whose ID is the idempotency
//!   ID, §4.3 step 2), *before* the transfer is submitted. A crash
//!   therefore leaves a record recovery can finish without
//!   minting new IDs: if the transfer is on the ledger, the intent
//!   finalizes as posted; if it is not, the stored transfer is
//!   resubmitted under the same ID, which the cluster dedups.
//! - **Item 5, partially — restart-safe service state.** The
//!   PoC's issued-supply counters and minter set live in service
//!   memory and recount from zero after a restart. Here they are
//!   rebuilt at [`GatewayService::open`] from the durable intent
//!   log (posted issues re-sum supply; posted grants re-form the
//!   minter set), the registry records are rebuilt from the
//!   register intents' recorded parameters, and currency key
//!   bindings recorded through
//!   [`GatewayService::record_definition`] are persisted too. The
//!   F1 store still cannot read grants back (registry-crate gap),
//!   so grants made outside this service's intent log are not
//!   visible to it; that part of item 5 stands.
//! - **Two-way reconciliation (invariant 7).**
//!   [`GatewayService::reconcile_intents`] joins the intent log
//!   against the ledger in both directions: every posted intent
//!   must name a ledger transfer that is present and identical,
//!   and every ledger transfer stamped with a graph object this
//!   gateway recorded must belong to an intent. Mismatches are
//!   [`IntentFinding`]s appended to the store's durable findings
//!   log — never silently dropped, never repaired by the check
//!   itself. This is the intent-level join; the F4 supply audit
//!   and graph-linkage job ([`super::reconcile`]) remains the
//!   ledger↔graph checker and is exposed unchanged as
//!   [`GatewayService::audit`].
//!
//! **Single logical writer (design §4.5).** All mutation runs
//! through this one object's `&mut self` pipeline; exactly one
//! service instance may own a given ledger backend and intent
//! store at a time. Leader election / hot standby is a deployment
//! property above this type, but the type is the seam it guards:
//! there is no second write path around it.
//!
//! **Scope, stated plainly.** The service executes the same op
//! set the PoC proved — `register_currency`, `grant_minter`,
//! `issue`, `transfer`. `convert` is phase 5 (#3801/#3802);
//! `freeze`/`sunset` are authorized by F3 but the registry crate
//! has no status-transition write yet, so the service refuses
//! them by name rather than pretending; `balance`/`history` are
//! reads and stay off the write pipeline (see
//! [`GatewayService::holder_balance`]). Abuse controls (design
//! §4.3 step 6) and graph chain-continuity validation remain as
//! absent as the PoC left them (its write-up items 10–11).
//!
//! **Idempotency-key discipline.** The idempotency ID is a hash
//! of (account, op, client request id). A client that reuses a
//! client request id for a *different* request gets the original
//! request's recorded outcome back; that is what an idempotency
//! key means, and it is the caller's side of the contract.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::time::Duration;

use id_currency_registry::{
    AccountKind, Factory, FactoryError, InMemoryStore, RegisterAccount,
    RegisterCurrency as RegistryRegister, Registry, RegistryError, Store as _, StoreError,
    SupplyPolicy,
};
use serde::{Deserialize, Serialize};

use crate::chain::{ChainError, Event};
use crate::currency::gateway::{
    Authorized, CurrencyOp, DeviceResolver, FixedDevices, GatewayError, GrantMinter, Issue,
    RegisterCurrency, Transfer, authorize, idempotency_id, verify_envelope,
};
use crate::currency::gateway_poc::{
    BackendCluster, GraphLog, PocError, Receipt, RegistryView, SignedReceipt, check_event,
    mint_currency_id, sign_receipt_with,
};
use crate::currency::graph::{
    BodyError, CurrencyBinding, CurrencyDefinition, CurrencyId, TokenError, TokenIssue,
    TokenTransfer,
};
use crate::currency_poc::ledger::{LedgerBackend, Transfer as LedgerTransfer, TransferError};
use crate::directory_auth::{ReplayGuard, Signed};
use crate::hosted_binding::hex_encode;

/// Everything that can refuse a gateway-service request or one of
/// its store operations, typed so a caller can tell a forged
/// envelope from a registry refusal from the cluster's verdict
/// from a durability failure.
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
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
    /// The cluster's verdict on the submitted transfer, verbatim.
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
    /// The op is authorized by F3 but outside this service's
    /// scope (see the module docs for which and why).
    #[error("operation is outside the gateway service scope: {0}")]
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
    /// The durable intent store could not read or write. A write
    /// failure on the write-ahead path refuses the request before
    /// any ledger submission: no durability, no execution.
    #[error("intent store failure: {0}")]
    IntentStore(String),
    /// This retry names a request the cluster already refused;
    /// the recorded verdict is replayed instead of resubmitting.
    #[error("cluster refused this request when it was first submitted: {0}")]
    StoredVerdict(String),
    /// The ledger's state contradicts the stored intent (a
    /// transfer with the intent's ID but different contents).
    /// Surfaced, never resolved by overwriting either side.
    #[error("ledger state contradicts the stored intent: {0}")]
    IntentConflict(String),
}

impl From<PocError> for ServiceError {
    fn from(error: PocError) -> Self {
        match error {
            PocError::Gateway(inner) => Self::Gateway(inner),
            PocError::Registry(inner) => Self::Registry(inner),
            PocError::Factory(inner) => Self::Factory(inner),
            PocError::Cluster(inner) => Self::Cluster(inner),
            PocError::Backend(inner) => Self::Backend(inner),
            PocError::GraphMismatch(inner) => Self::GraphMismatch(inner),
            PocError::UnknownCurrencyKey => Self::UnknownCurrencyKey,
            PocError::Unsupported(inner) => Self::Unsupported(inner),
            PocError::Token(inner) => Self::Token(inner),
            PocError::Body(inner) => Self::Body(inner),
            PocError::Chain(inner) => Self::Chain(inner),
        }
    }
}

/// Where a gateway intent stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentState {
    /// Persisted write-ahead, not yet finalized: the ledger
    /// submission either never happened or its outcome was lost
    /// with the process. Recovery (on retry, or the operator's
    /// reconciliation) resolves it against the ledger by transfer
    /// ID — never by resubmitting under a new ID.
    Recorded,
    /// The operation completed; for value ops the named transfer
    /// is on the ledger. The recorded receipt is the outcome.
    Posted,
    /// The cluster refused the submission. The refusal is the
    /// recorded outcome; a retry replays it.
    Failed,
}

/// The registration parameters of a `register_currency` intent,
/// kept so [`GatewayService::open`] can rebuild the registry
/// record after a restart (the registry crate's own durable
/// store adapter remains that crate's seam; the intent log is
/// this service's source of truth for what it registered).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationRecord {
    /// Currency display name.
    pub name: String,
    /// Currency symbol.
    pub symbol: String,
    /// Decimal scale.
    pub scale: u8,
    /// The finite supply cap.
    pub cap: u128,
}

/// The durable record of one accepted gateway request, keyed by
/// its §4.3 idempotency ID. One record is the whole join between
/// the signed authorization, the ledger operation it produced,
/// and the receipt that answers for both (invariant 7).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayIntent {
    /// The §4.3 idempotency ID: hash of account ‖ op ‖ client
    /// request id. Also the ledger transfer ID for value ops.
    pub idempotency: u128,
    /// The account the request acted for (post device resolution).
    pub account: String,
    /// The op name, as on the wire (`issue`, `transfer`, …).
    pub op: String,
    /// Lifecycle state.
    pub state: IntentState,
    /// The registry `currency_id`, once known.
    pub currency_id: Option<String>,
    /// The graph currency key (compact hash, hex), once known.
    pub currency_key: Option<String>,
    /// The TigerBeetle ledger, once allocated.
    pub ledger_id: Option<u32>,
    /// The exact pre-generated ledger transfer for value ops,
    /// stored before submission so a crash recovery resubmits
    /// byte-identical contents under the same ID.
    pub transfer: Option<LedgerTransfer>,
    /// The `user_data_128` graph stamp carried by the transfer.
    pub graph_stamp: Option<u128>,
    /// The graph object recording this write (event id, hex).
    pub graph_object: Option<String>,
    /// The account granted minting rights (grant intents only);
    /// [`GatewayService::open`] rebuilds the minter set from these.
    pub minter: Option<String>,
    /// The registration parameters (register intents only);
    /// [`GatewayService::open`] rebuilds the registry record
    /// from these, so a restarted service serves the currency
    /// without the caller re-registering.
    pub registration: Option<RegistrationRecord>,
    /// The signed receipt's body, once the intent finalizes.
    /// Re-signing it with the gateway key reproduces the original
    /// [`SignedReceipt`] (ed25519 signatures are deterministic).
    pub receipt: Option<Receipt>,
    /// The cluster's refusal, verbatim, for failed intents.
    pub failure: Option<String>,
}

impl GatewayIntent {
    /// A fresh write-ahead record for a value op.
    fn recorded(
        authorized: &Authorized,
        op: &CurrencyOp,
        currency_id: String,
        currency_key: &[u8; 32],
        ledger_id: u32,
        transfer: LedgerTransfer,
        graph_stamp: u128,
        graph_object: String,
    ) -> Self {
        Self {
            idempotency: authorized.idempotency,
            account: authorized.account.clone(),
            op: op.name().to_owned(),
            state: IntentState::Recorded,
            currency_id: Some(currency_id),
            currency_key: Some(hex_encode(currency_key)),
            ledger_id: Some(ledger_id),
            transfer: Some(transfer),
            graph_stamp: Some(graph_stamp),
            graph_object: Some(graph_object),
            minter: None,
            registration: None,
            receipt: None,
            failure: None,
        }
    }

    /// A finalized record for an op with no ledger transfer
    /// (registration, minter grant).
    fn posted_simple(
        authorized: &Authorized,
        op: &CurrencyOp,
        currency_id: String,
        currency_key: Option<&[u8; 32]>,
        ledger_id: u32,
        minter: Option<String>,
        registration: Option<RegistrationRecord>,
        receipt: Receipt,
    ) -> Self {
        Self {
            idempotency: authorized.idempotency,
            account: authorized.account.clone(),
            op: op.name().to_owned(),
            state: IntentState::Posted,
            currency_id: Some(currency_id),
            currency_key: currency_key.map(|key| hex_encode(key)),
            ledger_id: Some(ledger_id),
            transfer: None,
            graph_stamp: None,
            graph_object: None,
            minter,
            registration,
            receipt: Some(receipt),
            failure: None,
        }
    }

    /// The receipt this intent answers with, rebuilt from its
    /// recorded fields. Used when a recorded intent finalizes
    /// during recovery and when a value intent's stored receipt
    /// is re-issued.
    fn build_receipt(&self) -> Receipt {
        Receipt {
            op: self.op.clone(),
            account: self.account.clone(),
            idempotency: hex_encode(&self.idempotency.to_be_bytes()),
            currency_id: self.currency_id.clone(),
            currency: self.currency_key.clone(),
            ledger_id: self.ledger_id,
            transfer_id: self.transfer.as_ref().map(|t| hex_encode(&t.id.to_be_bytes())),
            graph_object: self.graph_object.clone(),
        }
    }
}

/// One drift between the intent log and the ledger, as
/// [`GatewayService::reconcile_intents`] finds it (invariant 7).
/// Findings are report-only: reconciliation never repairs,
/// deletes, or rewrites either side.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "finding", rename_all = "snake_case")]
pub enum IntentFinding {
    /// An intent still in `Recorded` state: its submission
    /// outcome was never finalized. A client retry resolves it;
    /// until then it is an operator-visible loose end.
    UnresolvedIntent {
        /// The intent's idempotency ID.
        idempotency: u128,
        /// The op the intent was executing.
        op: String,
    },
    /// A posted intent whose transfer is not on the ledger: the
    /// authorization claims value moved and the balance truth
    /// has no record of it.
    PostedIntentMissingTransfer {
        /// The intent's idempotency ID.
        idempotency: u128,
        /// The transfer ID that should be on the ledger.
        transfer_id: u128,
    },
    /// A posted intent whose ledger transfer differs from the
    /// transfer the intent stored (amount, accounts, ledger,
    /// stamp, or code): one of the two records is not describing
    /// the operation the other describes.
    PostedIntentTransferMismatch {
        /// The intent's idempotency ID.
        idempotency: u128,
        /// The transfer ID both sides name.
        transfer_id: u128,
    },
    /// An intent the cluster refused, whose transfer is on the
    /// ledger anyway: the refusal and the balance truth cannot
    /// both be the whole story.
    FailedIntentTransferPresent {
        /// The intent's idempotency ID.
        idempotency: u128,
        /// The transfer ID found on the ledger.
        transfer_id: u128,
    },
    /// A ledger transfer stamped with a graph object this
    /// gateway recorded, which no intent in the store produced:
    /// a ledger write without its authorization record.
    LedgerTransferWithoutIntent {
        /// The orphan transfer's ID.
        transfer_id: u128,
        /// The ledger it sits on.
        ledger_id: u32,
        /// The graph stamp it carries.
        user_data_128: u128,
    },
}

/// One findings-log line: a finding plus the time of the
/// reconciliation run that produced it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindingRecord {
    /// When the run happened (caller's clock, unix seconds).
    pub run_at: u64,
    /// The drift found.
    pub finding: IntentFinding,
}

/// The outcome of one [`GatewayService::reconcile_intents`] run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IntentReconReport {
    /// Intents examined.
    pub intents_checked: usize,
    /// Ledger transfers examined in the orphan direction.
    pub ledger_transfers_checked: usize,
    /// Every drift found (also appended to the durable log).
    pub findings: Vec<IntentFinding>,
}

impl IntentReconReport {
    /// Whether the run found no drift at all.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }
}

/// The durable store behind the gateway service: intent records
/// keyed by idempotency ID, currency key bindings, and the
/// append-only findings log. Implementations must make `put`
/// atomic enough that a crash mid-write leaves the previous
/// record or the new one, never a torn one
/// ([`FileIntentStore`] writes via temp file + rename + fsync).
pub trait IntentStore {
    /// The intent recorded under `idempotency`, if any.
    ///
    /// # Errors
    ///
    /// Returns a description of the read failure.
    fn get(&self, idempotency: u128) -> Result<Option<GatewayIntent>, String>;
    /// Insert or replace the intent under its idempotency ID.
    ///
    /// # Errors
    ///
    /// Returns a description of the write failure.
    fn put(&mut self, intent: &GatewayIntent) -> Result<(), String>;
    /// Every intent in the store, in idempotency order.
    ///
    /// # Errors
    ///
    /// Returns a description of the read failure.
    fn all(&self) -> Result<Vec<GatewayIntent>, String>;
    /// The registry `currency_id` bound to a graph currency key.
    ///
    /// # Errors
    ///
    /// Returns a description of the read failure.
    fn binding(&self, key: &[u8; 32]) -> Result<Option<String>, String>;
    /// Bind a graph currency key to a registry `currency_id`.
    ///
    /// # Errors
    ///
    /// Returns a description of the write failure.
    fn put_binding(&mut self, key: &[u8; 32], currency_id: &str) -> Result<(), String>;
    /// Every key binding, in key order.
    ///
    /// # Errors
    ///
    /// Returns a description of the read failure.
    fn bindings(&self) -> Result<Vec<([u8; 32], String)>, String>;
    /// Append one reconciliation run's findings to the durable
    /// log. Appending an empty set is a no-op.
    ///
    /// # Errors
    ///
    /// Returns a description of the write failure.
    fn append_findings(&mut self, findings: &[FindingRecord]) -> Result<(), String>;
}

/// An [`IntentStore`] in process memory. For tests and ephemeral
/// compositions; it provides the semantics, not the durability —
/// production compositions use [`FileIntentStore`].
#[derive(Debug, Default)]
pub struct InMemoryIntentStore {
    intents: BTreeMap<u128, GatewayIntent>,
    bindings: BTreeMap<[u8; 32], String>,
    findings: Vec<FindingRecord>,
}

impl InMemoryIntentStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Every finding appended so far, in append order.
    #[must_use]
    pub fn findings(&self) -> &[FindingRecord] {
        &self.findings
    }
}

impl IntentStore for InMemoryIntentStore {
    fn get(&self, idempotency: u128) -> Result<Option<GatewayIntent>, String> {
        Ok(self.intents.get(&idempotency).cloned())
    }

    fn put(&mut self, intent: &GatewayIntent) -> Result<(), String> {
        self.intents.insert(intent.idempotency, intent.clone());
        Ok(())
    }

    fn all(&self) -> Result<Vec<GatewayIntent>, String> {
        Ok(self.intents.values().cloned().collect())
    }

    fn binding(&self, key: &[u8; 32]) -> Result<Option<String>, String> {
        Ok(self.bindings.get(key).cloned())
    }

    fn put_binding(&mut self, key: &[u8; 32], currency_id: &str) -> Result<(), String> {
        self.bindings.insert(*key, currency_id.to_owned());
        Ok(())
    }

    fn bindings(&self) -> Result<Vec<([u8; 32], String)>, String> {
        Ok(self
            .bindings
            .iter()
            .map(|(key, value)| (*key, value.clone()))
            .collect())
    }

    fn append_findings(&mut self, findings: &[FindingRecord]) -> Result<(), String> {
        self.findings.extend_from_slice(findings);
        Ok(())
    }
}

/// An [`IntentStore`] on disk: one JSON file per intent under
/// `intents/`, a `bindings.json` map, and an append-only
/// `findings.jsonl`, all under one directory. Every record write
/// is temp-file + fsync + rename + directory fsync (the
/// conversion intent store's pattern,
/// [`crate::currency_poc::intent`]), so a crash leaves the old
/// record or the new one, never a torn one.
#[derive(Debug)]
pub struct FileIntentStore {
    dir: PathBuf,
    bindings: BTreeMap<[u8; 32], String>,
}

impl FileIntentStore {
    /// Open (creating if needed) the store rooted at `dir`,
    /// loading the persisted key bindings.
    ///
    /// # Errors
    ///
    /// Fails if the directory cannot be created or the bindings
    /// file cannot be read or parsed.
    pub fn open(dir: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(dir.join("intents"))
            .map_err(|error| format!("create intent store {}: {error}", dir.display()))?;
        let bindings = match fs::read(dir.join("bindings.json")) {
            Ok(bytes) => {
                let raw: BTreeMap<String, String> = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("parse bindings.json: {error}"))?;
                let mut parsed = BTreeMap::new();
                for (key_hex, currency_id) in raw {
                    let key = decode_key(&key_hex)
                        .ok_or_else(|| format!("bindings.json key {key_hex} is not 32-byte hex"))?;
                    parsed.insert(key, currency_id);
                }
                parsed
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(format!("read bindings.json: {error}")),
        };
        Ok(Self { dir, bindings })
    }

    fn intent_path(&self, idempotency: u128) -> PathBuf {
        self.dir
            .join("intents")
            .join(format!("{idempotency:032x}.json"))
    }

    fn bindings_path(&self) -> PathBuf {
        self.dir.join("bindings.json")
    }

    fn findings_path(&self) -> PathBuf {
        self.dir.join("findings.jsonl")
    }

    fn persist_bindings(&self) -> Result<(), String> {
        let raw: BTreeMap<String, String> = self
            .bindings
            .iter()
            .map(|(key, value)| (hex_encode(key), value.clone()))
            .collect();
        let bytes =
            serde_json::to_vec(&raw).map_err(|error| format!("encode bindings: {error}"))?;
        write_atomic(&self.bindings_path(), &bytes)
    }
}

/// Decode a 32-byte hex key, as [`FileIntentStore`] persists
/// bindings keys.
fn decode_key(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut key = [0_u8; 32];
    for (index, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(key)
}

/// Write `bytes` to `path` atomically: temp file in the same
/// directory, fsync, rename over the target, fsync the directory
/// so the rename itself is durable.
fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).map_err(|error| format!("write {}: {error}", tmp.display()))?;
    fs::File::open(&tmp)
        .and_then(|file| file.sync_all())
        .map_err(|error| format!("fsync {}: {error}", tmp.display()))?;
    fs::rename(&tmp, path)
        .map_err(|error| format!("rename to {}: {error}", path.display()))?;
    if let Some(parent) = path.parent() {
        fs::File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|error| format!("fsync dir {}: {error}", parent.display()))?;
    }
    Ok(())
}

impl IntentStore for FileIntentStore {
    fn get(&self, idempotency: u128) -> Result<Option<GatewayIntent>, String> {
        match fs::read(self.intent_path(idempotency)) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|error| format!("parse intent {idempotency:032x}: {error}")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("read intent {idempotency:032x}: {error}")),
        }
    }

    fn put(&mut self, intent: &GatewayIntent) -> Result<(), String> {
        let bytes = serde_json::to_vec(intent)
            .map_err(|error| format!("encode intent {:032x}: {error}", intent.idempotency))?;
        write_atomic(&self.intent_path(intent.idempotency), &bytes)
    }

    fn all(&self) -> Result<Vec<GatewayIntent>, String> {
        let mut intents = Vec::new();
        let entries = fs::read_dir(self.dir.join("intents"))
            .map_err(|error| format!("read intents dir: {error}"))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("read intents dir: {error}"))?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                let bytes =
                    fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
                let intent: GatewayIntent = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("parse {}: {error}", path.display()))?;
                intents.push(intent);
            }
        }
        intents.sort_by_key(|intent| intent.idempotency);
        Ok(intents)
    }

    fn binding(&self, key: &[u8; 32]) -> Result<Option<String>, String> {
        Ok(self.bindings.get(key).cloned())
    }

    fn put_binding(&mut self, key: &[u8; 32], currency_id: &str) -> Result<(), String> {
        self.bindings.insert(*key, currency_id.to_owned());
        self.persist_bindings()
    }

    fn bindings(&self) -> Result<Vec<([u8; 32], String)>, String> {
        Ok(self
            .bindings
            .iter()
            .map(|(key, value)| (*key, value.clone()))
            .collect())
    }

    fn append_findings(&mut self, findings: &[FindingRecord]) -> Result<(), String> {
        if findings.is_empty() {
            return Ok(());
        }
        let mut text = String::new();
        for record in findings {
            let line = serde_json::to_string(record)
                .map_err(|error| format!("encode finding: {error}"))?;
            text.push_str(&line);
            text.push('\n');
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.findings_path())
            .map_err(|error| format!("open findings log: {error}"))?;
        file.write_all(text.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("append findings log: {error}"))?;
        Ok(())
    }
}

/// How many transfers per ledger one reconciliation run scans
/// in the orphan direction. Bounded so the check stays cheap;
/// the F4 job ([`super::reconcile`]) is the unbounded auditor.
const ORPHAN_SCAN_LIMIT: usize = 1_000;

/// The production currency gateway: the single logical writer
/// (design §4.5) running the §4.3 pipeline over the landed F1
/// registry, F2 factory, and F3 envelope, with durable
/// write-ahead intents ([`IntentStore`]) between authorization
/// and ledger submission. Generic over the ledger backend (the
/// #2289 seam: in-memory in tests, TigerBeetle in production)
/// and the intent store ([`FileIntentStore`] in production).
pub struct GatewayService<B: LedgerBackend, I: IntentStore> {
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
    intents: I,
}

impl<B: LedgerBackend, I: IntentStore> GatewayService<B, I> {
    /// Open the service over `backend`, signing receipts with
    /// `gateway_key`, with durable state in `intents`.
    ///
    /// Opening rebuilds the service's state from the intent
    /// log (module docs, item 5): currency key bindings, the
    /// registry records themselves (posted register intents
    /// carry their registration parameters), issued supply per
    /// currency (posted issue intents), and the minter set
    /// (posted grant intents). The registry still runs over its
    /// in-memory store — the registry crate's durable store
    /// adapter is that crate's own seam (its `Store` trait) —
    /// but the intent log is the source of truth for what this
    /// service registered, so a restart serves every known
    /// currency without callers re-registering.
    ///
    /// # Errors
    ///
    /// Fails if the intent store cannot be read, or if a
    /// rebuilt registry record lands on a different ledger than
    /// the intent log recorded (the allocator is deterministic;
    /// a divergence means the log and the registry disagree
    /// about history, and opening refuses rather than serving a
    /// currency on the wrong ledger).
    pub fn open(
        backend: B,
        gateway_key: ed25519_dalek::SigningKey,
        intents: I,
    ) -> Result<Self, ServiceError> {
        let mut currency_ids = BTreeMap::new();
        for (key, currency_id) in intents.bindings().map_err(ServiceError::IntentStore)? {
            currency_ids.insert(key, currency_id);
        }
        let all = intents.all().map_err(ServiceError::IntentStore)?;
        let mut issued: BTreeMap<String, u128> = BTreeMap::new();
        let mut minters: BTreeMap<String, HashSet<String>> = BTreeMap::new();
        let mut registrations: Vec<&GatewayIntent> = Vec::new();
        for intent in &all {
            if intent.state != IntentState::Posted {
                continue;
            }
            match intent.op.as_str() {
                "issue" => {
                    if let (Some(currency_id), Some(transfer)) =
                        (&intent.currency_id, &intent.transfer)
                    {
                        *issued.entry(currency_id.clone()).or_default() += transfer.amount;
                    }
                }
                "grant_minter" => {
                    if let (Some(currency_id), Some(minter)) =
                        (&intent.currency_id, &intent.minter)
                    {
                        minters
                            .entry(currency_id.clone())
                            .or_default()
                            .insert(minter.clone());
                    }
                }
                "register_currency" => {
                    if intent.registration.is_some() {
                        registrations.push(intent);
                    }
                }
                _ => {}
            }
        }
        // Rebuild the registry records in ascending ledger
        // order, which reproduces the allocator's original
        // assignment order.
        registrations.sort_by_key(|intent| intent.ledger_id.unwrap_or(u32::MAX));
        let mut registry = Registry::new(InMemoryStore::new());
        for intent in registrations {
            let (Some(currency_id), Some(params)) =
                (&intent.currency_id, &intent.registration)
            else {
                continue;
            };
            let record = registry
                .register_currency(RegistryRegister {
                    currency_id: currency_id.clone(),
                    name: params.name.clone(),
                    symbol: params.symbol.clone(),
                    scale: params.scale,
                    issuer_id: intent.account.clone(),
                    supply_policy: SupplyPolicy {
                        mintable: true,
                        cap: Some(params.cap),
                        policy_uri: None,
                    },
                    graph_object_id: None,
                })
                .map_err(ServiceError::Registry)?;
            if Some(record.ledger_id) != intent.ledger_id {
                return Err(ServiceError::IntentStore(format!(
                    "registry rebuild diverged for {currency_id}: the intent log \
                     names ledger {:?}, the allocator assigned {}",
                    intent.ledger_id, record.ledger_id
                )));
            }
        }
        Ok(Self {
            registry,
            backend,
            devices: FixedDevices::new(),
            replays: ReplayGuard::after_boot(0, Duration::ZERO),
            gateway_key,
            currency_ids,
            minters,
            issued,
            indexed_accounts: HashSet::new(),
            graph: GraphLog::default(),
            intents,
        })
    }

    /// Attach a device (public key, lower-case hex) to the
    /// account it acts for (design §4.1b).
    pub fn attach_device(&mut self, device: &str, account: &str) {
        self.devices.attach(device, account);
    }

    /// The gateway's receipt-verifying key.
    #[must_use]
    pub fn verifying_key(&self) -> ed25519_dalek::VerifyingKey {
        self.gateway_key.verifying_key()
    }

    /// The graph log of stamped objects recorded by this
    /// service instance.
    #[must_use]
    pub const fn graph(&self) -> &GraphLog {
        &self.graph
    }

    /// The ledger backend, for read-side lookups.
    #[must_use]
    pub const fn backend(&self) -> &B {
        &self.backend
    }

    /// The intent store, for operator inspection.
    #[must_use]
    pub const fn intents(&self) -> &I {
        &self.intents
    }

    /// Consume the service and return its ledger backend, e.g.
    /// to hand the same cluster connection to a successor
    /// instance on restart.
    pub fn into_backend(self) -> B {
        self.backend
    }

    /// The registry record for a currency key, once its
    /// definition is recorded.
    #[must_use]
    pub fn registry_record(
        &self,
        key: &[u8; 32],
    ) -> Option<id_currency_registry::CurrencyRecord> {
        self.currency_ids
            .get(key)
            .and_then(|currency_id| self.registry.resolve(currency_id).cloned())
    }

    /// An account's spendable balance in a currency, read from
    /// the ledger (the balance truth), or `None` if the currency
    /// or account is unknown to the service.
    #[must_use]
    pub fn holder_balance(&self, key: &[u8; 32], owner: &str) -> Option<u128> {
        let record = self.registry_record(key)?;
        let spec =
            id_currency_registry::account_spec(record.ledger_id, AccountKind::Holder, owner, "");
        let balance = self.backend.account_balance(spec.id)?;
        Some(balance.spendable())
    }

    /// Record a currency's signed definition object (PoC
    /// write-up item 3): the issuer's canonical claim (§2.4),
    /// signed after registration allocated the ledger it names.
    /// Indexes the currency key for later envelopes, files the
    /// object in the graph log, and — beyond the PoC — persists
    /// the key binding in the intent store so a restarted
    /// service serves the currency without re-recording.
    ///
    /// # Errors
    ///
    /// Fails if the event does not verify, is not a definition,
    /// its body hash or author disagrees, the definition names
    /// no ledger or a ledger no registration allocated, the
    /// definition's fields disagree with the registry record, or
    /// the binding cannot be persisted.
    pub fn record_definition(
        &mut self,
        event: &Event,
        body: &[u8],
    ) -> Result<CurrencyId, ServiceError> {
        let event = crate::chain::decode(&event.bytes)?;
        if event.header.extensions.kind != super::graph::DEFINE_KIND {
            return Err(ServiceError::GraphMismatch(
                "event is not a currency definition",
            ));
        }
        if event.header.payload_hash != Some(p2panda_core::Hash::digest(body)) {
            return Err(ServiceError::GraphMismatch("definition body hash"));
        }
        let definition = CurrencyDefinition::from_body(body)?;
        let CurrencyBinding::Ledger { ledger } = definition.binding else {
            return Err(ServiceError::GraphMismatch(
                "definition does not name a ledger",
            ));
        };
        let record = self
            .registry
            .store()
            .currency_by_ledger(ledger)
            .ok_or(ServiceError::UnknownCurrencyKey)?
            .clone();
        let author = crate::currency::author_id(&event);
        if hex_encode(&author) != record.issuer_id {
            return Err(ServiceError::GraphMismatch(
                "definition author is not the currency issuer",
            ));
        }
        if definition.name != record.name
            || definition.symbol != record.symbol
            || definition.scale != record.scale
            || definition.supply.capped != record.supply_policy.cap.unwrap_or(u128::MAX)
        {
            return Err(ServiceError::GraphMismatch(
                "definition disagrees with the registry record",
            ));
        }
        let id = CurrencyId::from_definition(author, &definition)?;
        let key = *id.compact_hash().as_bytes();
        self.intents
            .put_binding(&key, &record.currency_id)
            .map_err(ServiceError::IntentStore)?;
        self.currency_ids.insert(key, record.currency_id);
        self.graph.record(&event, body);
        Ok(id)
    }

    /// Run one signed request through the pipeline (§4.3):
    /// verify, resolve, answer from the intent store if this
    /// request already has a recorded outcome, authorize, then
    /// execute. `graph` carries the client-signed graph object
    /// for the write ops (PoC write-up item 9).
    ///
    /// # Errors
    ///
    /// Returns the typed refusal of whichever step refused:
    /// envelope and authorization failures, registry and factory
    /// failures as themselves, the cluster's verdict on a
    /// submitted transfer, a stored verdict on a retry of a
    /// refused request, and intent-store failures (which refuse
    /// before any ledger submission on the write-ahead path).
    pub fn handle(
        &mut self,
        signed: &Signed,
        body: &[u8],
        now: u64,
        graph: Option<(&Event, &[u8])>,
    ) -> Result<SignedReceipt, ServiceError> {
        let verified = verify_envelope(&mut self.replays, signed, body, now)?;
        // The intent answer precedes authorization (§4.3's
        // ordering, PoC write-up item 13): a retry of a request
        // already served gets its recorded outcome back, keyed
        // by the idempotency ID itself. Resolving the device is
        // not authorization; the registry checks below still run
        // for every request without a recorded outcome.
        let account = self
            .devices
            .resolve_account(&verified.device)
            .ok_or(GatewayError::UnknownDevice)?;
        let idempotency = idempotency_id(&account, &verified.request);
        if let Some(intent) = self
            .intents
            .get(idempotency)
            .map_err(ServiceError::IntentStore)?
        {
            return self.replay(intent, graph);
        }
        let authorized = {
            let view = RegistryView::new(
                &self.registry,
                &self.currency_ids,
                &self.minters,
                &self.issued,
            );
            authorize(&view, &account, &verified.request.op)?;
            Authorized {
                account,
                idempotency,
            }
        };
        match &verified.request.op {
            CurrencyOp::RegisterCurrency(op) => self.register(&authorized, op),
            CurrencyOp::GrantMinter(op) => self.grant_minter(&authorized, op),
            CurrencyOp::Issue(op) => self.issue(&authorized, op, graph),
            CurrencyOp::Transfer(op) => self.transfer(&authorized, op, graph),
            other => Err(ServiceError::Unsupported(other.name())),
        }
    }

    /// Answer a retry from its recorded intent: the stored
    /// receipt for a posted intent, the stored verdict for a
    /// failed one, and ledger recovery for one left recorded by
    /// a crash (module docs, invariant 6).
    fn replay(
        &mut self,
        intent: GatewayIntent,
        graph: Option<(&Event, &[u8])>,
    ) -> Result<SignedReceipt, ServiceError> {
        match intent.state {
            IntentState::Posted => {
                if let Some((event, body)) = graph {
                    self.graph.record(event, body);
                }
                let receipt = intent.receipt.clone().ok_or_else(|| {
                    ServiceError::IntentStore(format!(
                        "posted intent {:032x} carries no receipt",
                        intent.idempotency
                    ))
                })?;
                Ok(sign_receipt_with(&self.gateway_key, receipt)?)
            }
            IntentState::Failed => Err(ServiceError::StoredVerdict(
                intent.failure.clone().unwrap_or_default(),
            )),
            IntentState::Recorded => {
                if let Some((event, body)) = graph {
                    self.graph.record(event, body);
                }
                self.recover_recorded(intent)
            }
        }
    }

    /// Resolve a write-ahead intent whose submission outcome was
    /// lost: look the stored transfer up by ID. Present and
    /// identical — the submission landed; finalize as posted.
    /// Absent — the submission never landed; resubmit the stored
    /// transfer under the same ID. Present but different — the
    /// ID was reused by another writer; that is a conflict to
    /// surface, never to overwrite.
    fn recover_recorded(&mut self, mut intent: GatewayIntent) -> Result<SignedReceipt, ServiceError> {
        let transfer = intent.transfer.clone().ok_or_else(|| {
            ServiceError::IntentStore(format!(
                "recorded intent {:032x} carries no transfer",
                intent.idempotency
            ))
        })?;
        match self.backend.try_lookup_transfer(transfer.id) {
            Ok(Some(found)) if found == transfer => {
                self.finalize_posted(&mut intent, true)
            }
            Ok(Some(_)) => Err(ServiceError::IntentConflict(format!(
                "transfer {} exists with contents other than intent {:032x} recorded",
                transfer.id, intent.idempotency
            ))),
            Ok(None) => match self.submit(transfer) {
                Ok(fresh) => self.finalize_posted(&mut intent, fresh),
                Err(error) => {
                    let verdict = match &error {
                        ServiceError::Cluster(cluster) => format!("{cluster:?}"),
                        other => other.to_string(),
                    };
                    intent.state = IntentState::Failed;
                    intent.failure = Some(verdict);
                    self.intents
                        .put(&intent)
                        .map_err(ServiceError::IntentStore)?;
                    Err(error)
                }
            },
            Err(error) => Err(ServiceError::Backend(error.to_string())),
        }
    }

    /// Mark an intent posted, count its supply effect once, and
    /// answer with its receipt. `fresh` says the transfer posted
    /// in this call (or was just found on the ledger); supply is
    /// counted here and only here, so finalization is the single
    /// place an issue enters the service's issued view — and the
    /// restart rebuild re-derives the same totals from the log.
    fn finalize_posted(
        &mut self,
        intent: &mut GatewayIntent,
        fresh: bool,
    ) -> Result<SignedReceipt, ServiceError> {
        if intent.op == "issue" && fresh {
            if let (Some(currency_id), Some(transfer)) =
                (&intent.currency_id, &intent.transfer)
            {
                *self.issued.entry(currency_id.clone()).or_default() += transfer.amount;
            }
        }
        let receipt = intent.build_receipt();
        intent.state = IntentState::Posted;
        intent.receipt = Some(receipt.clone());
        self.intents
            .put(intent)
            .map_err(ServiceError::IntentStore)?;
        Ok(sign_receipt_with(&self.gateway_key, receipt)?)
    }

    fn register(
        &mut self,
        authorized: &Authorized,
        op: &RegisterCurrency,
    ) -> Result<SignedReceipt, ServiceError> {
        let currency_id = mint_currency_id(&authorized.account, op);
        // A registration whose intent finalize was lost to a
        // crash is re-answered from the registry record it
        // already created: the minted id is deterministic, so a
        // record under it with this issuer *is* this request's
        // outcome.
        let record = match self.registry.resolve(&currency_id).cloned() {
            Some(record) if record.issuer_id == authorized.account => record,
            Some(_) => {
                return Err(ServiceError::Registry(RegistryError::Store(
                    StoreError::DuplicateCurrencyId(currency_id),
                )));
            }
            None => self.registry.register_currency(RegistryRegister {
                currency_id: currency_id.clone(),
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
            })?,
        };
        let accounts = {
            let mut cluster = BackendCluster::new(&mut self.backend);
            Factory::ensure_currency_accounts(&mut cluster, &record, &authorized.account, &[])?
        };
        self.index_account(&record, accounts.issuer, &record.issuer_id, AccountKind::Issuer)?;
        self.index_account(
            &record,
            accounts.fee,
            id_currency_registry::PLATFORM_OWNER_ID,
            AccountKind::Fee,
        )?;
        self.index_account(
            &record,
            accounts.burn,
            id_currency_registry::PLATFORM_OWNER_ID,
            AccountKind::Burn,
        )?;
        let receipt = Receipt {
            op: "register_currency".to_owned(),
            account: authorized.account.clone(),
            idempotency: hex_encode(&authorized.idempotency.to_be_bytes()),
            currency_id: Some(record.currency_id.clone()),
            currency: None,
            ledger_id: Some(record.ledger_id),
            transfer_id: None,
            graph_object: None,
        };
        let intent = GatewayIntent::posted_simple(
            authorized,
            &CurrencyOp::RegisterCurrency(op.clone()),
            record.currency_id.clone(),
            None,
            record.ledger_id,
            None,
            Some(RegistrationRecord {
                name: op.name.clone(),
                symbol: op.symbol.clone(),
                scale: op.scale,
                cap: op.cap,
            }),
            receipt.clone(),
        );
        self.intents
            .put(&intent)
            .map_err(ServiceError::IntentStore)?;
        Ok(sign_receipt_with(&self.gateway_key, receipt)?)
    }

    fn grant_minter(
        &mut self,
        authorized: &Authorized,
        op: &GrantMinter,
    ) -> Result<SignedReceipt, ServiceError> {
        let currency_id = self
            .currency_ids
            .get(&op.currency)
            .cloned()
            .ok_or(ServiceError::UnknownCurrencyKey)?;
        match self.registry.grant_minter(
            &currency_id,
            op.minter.clone(),
            authorized.account.clone(),
        ) {
            Ok(()) => {}
            // A grant whose intent finalize was lost to a crash
            // has already landed; the duplicate refusal *is* the
            // evidence, and the outcome is the grant.
            Err(RegistryError::Store(StoreError::DuplicateGrant)) => {}
            Err(error) => return Err(ServiceError::Registry(error)),
        }
        self.minters
            .entry(currency_id.clone())
            .or_default()
            .insert(op.minter.clone());
        let record = self
            .registry
            .resolve(&currency_id)
            .cloned()
            .ok_or(ServiceError::UnknownCurrencyKey)?;
        let receipt = Receipt {
            op: "grant_minter".to_owned(),
            account: authorized.account.clone(),
            idempotency: hex_encode(&authorized.idempotency.to_be_bytes()),
            currency_id: Some(currency_id.clone()),
            currency: Some(hex_encode(&op.currency)),
            ledger_id: Some(record.ledger_id),
            transfer_id: None,
            graph_object: None,
        };
        let intent = GatewayIntent::posted_simple(
            authorized,
            &CurrencyOp::GrantMinter(op.clone()),
            currency_id,
            Some(&op.currency),
            record.ledger_id,
            Some(op.minter.clone()),
            None,
            receipt.clone(),
        );
        self.intents
            .put(&intent)
            .map_err(ServiceError::IntentStore)?;
        Ok(sign_receipt_with(&self.gateway_key, receipt)?)
    }

    fn issue(
        &mut self,
        authorized: &Authorized,
        op: &Issue,
        graph: Option<(&Event, &[u8])>,
    ) -> Result<SignedReceipt, ServiceError> {
        let (event, body) =
            graph.ok_or(ServiceError::GraphMismatch("issue carries no graph object"))?;
        let record = self.record_for(&op.currency)?;
        let event = check_event(event, body, super::graph::ISSUE_KIND, &record.issuer_id)?;
        let token = TokenIssue::from_body(body)?;
        if token.currency.as_bytes() != &op.currency
            || token.amount != op.amount
            || hex_encode(&token.to) != op.to
        {
            return Err(ServiceError::GraphMismatch(
                "issue body disagrees with the request",
            ));
        }
        let issuer_tb = self.ensure_account(&record, &record.issuer_id, AccountKind::Issuer)?;
        let holder_tb = self.ensure_account(&record, &op.to, AccountKind::Holder)?;
        let stamp = GraphLog::stamp_of(&event);
        let transfer = LedgerTransfer {
            id: authorized.idempotency,
            debit_account_id: issuer_tb,
            credit_account_id: holder_tb,
            amount: op.amount,
            ledger: record.ledger_id,
            user_data_128: stamp,
            code: id_currency_registry::factory::transfer_code::ISSUANCE_START,
            linked: false,
        };
        // Write-ahead: the intent, carrying the exact transfer,
        // is durable before the ledger sees anything. If this
        // write fails the request refuses here — no durability,
        // no execution.
        let mut intent = GatewayIntent::recorded(
            authorized,
            &CurrencyOp::Issue(op.clone()),
            record.currency_id.clone(),
            &op.currency,
            record.ledger_id,
            transfer.clone(),
            stamp,
            event.id.to_hex(),
        );
        self.intents
            .put(&intent)
            .map_err(ServiceError::IntentStore)?;
        self.graph.record(&event, body);
        match self.submit(transfer) {
            Ok(fresh) => self.finalize_posted(&mut intent, fresh),
            Err(error) => {
                let verdict = match &error {
                    ServiceError::Cluster(cluster) => format!("{cluster:?}"),
                    other => other.to_string(),
                };
                intent.state = IntentState::Failed;
                intent.failure = Some(verdict);
                self.intents
                    .put(&intent)
                    .map_err(ServiceError::IntentStore)?;
                Err(error)
            }
        }
    }

    fn transfer(
        &mut self,
        authorized: &Authorized,
        op: &Transfer,
        graph: Option<(&Event, &[u8])>,
    ) -> Result<SignedReceipt, ServiceError> {
        let (event, body) = graph.ok_or(ServiceError::GraphMismatch(
            "transfer carries no graph object",
        ))?;
        let record = self.record_for(&op.currency)?;
        let event = check_event(event, body, super::graph::TRANSFER_KIND, &op.from)?;
        let token = TokenTransfer::from_body(body)?;
        if token.currency.as_bytes() != &op.currency
            || token.amount != op.amount
            || hex_encode(&token.to) != op.to
        {
            return Err(ServiceError::GraphMismatch(
                "transfer body disagrees with the request",
            ));
        }
        let from_tb = self.ensure_account(&record, &op.from, AccountKind::Holder)?;
        let to_tb = self.ensure_account(&record, &op.to, AccountKind::Holder)?;
        let stamp = GraphLog::stamp_of(&event);
        let transfer = LedgerTransfer {
            id: authorized.idempotency,
            debit_account_id: from_tb,
            credit_account_id: to_tb,
            amount: op.amount,
            ledger: record.ledger_id,
            user_data_128: stamp,
            code: id_currency_registry::factory::transfer_code::HOLDER_TRANSFER_START,
            linked: false,
        };
        let mut intent = GatewayIntent::recorded(
            authorized,
            &CurrencyOp::Transfer(op.clone()),
            record.currency_id.clone(),
            &op.currency,
            record.ledger_id,
            transfer.clone(),
            stamp,
            event.id.to_hex(),
        );
        self.intents
            .put(&intent)
            .map_err(ServiceError::IntentStore)?;
        self.graph.record(&event, body);
        match self.submit(transfer) {
            Ok(fresh) => self.finalize_posted(&mut intent, fresh),
            Err(error) => {
                let verdict = match &error {
                    ServiceError::Cluster(cluster) => format!("{cluster:?}"),
                    other => other.to_string(),
                };
                intent.state = IntentState::Failed;
                intent.failure = Some(verdict);
                self.intents
                    .put(&intent)
                    .map_err(ServiceError::IntentStore)?;
                Err(error)
            }
        }
    }

    /// The registry record for a currency key.
    fn record_for(
        &self,
        key: &[u8; 32],
    ) -> Result<id_currency_registry::CurrencyRecord, ServiceError> {
        self.registry_record(key)
            .ok_or(ServiceError::UnknownCurrencyKey)
    }

    /// Factory-ensure one account on the ledger and record it
    /// in the registry's account index the first time it appears.
    fn ensure_account(
        &mut self,
        record: &id_currency_registry::CurrencyRecord,
        owner: &str,
        kind: AccountKind,
    ) -> Result<u128, ServiceError> {
        let tb_id = {
            let mut cluster = BackendCluster::new(&mut self.backend);
            Factory::ensure_account(&mut cluster, record.ledger_id, owner, kind, "")?
        };
        self.index_account(record, tb_id, owner, kind)?;
        Ok(tb_id)
    }

    fn index_account(
        &mut self,
        record: &id_currency_registry::CurrencyRecord,
        tb_id: u128,
        owner: &str,
        kind: AccountKind,
    ) -> Result<(), ServiceError> {
        if !self.indexed_accounts.insert(tb_id) {
            return Ok(());
        }
        self.registry.register_account(RegisterAccount {
            currency_id: record.currency_id.clone(),
            account_id: format!("{tb_id:032x}"),
            owner_id: owner.to_owned(),
            kind,
            salt: String::new(),
        })?;
        Ok(())
    }

    /// Submit one transfer; `Ok(true)` when it posted fresh,
    /// `Ok(false)` when the cluster reported it already exists
    /// under the same ID (the idempotent-replay verdict: the
    /// value moved exactly once, under this intent's ID).
    fn submit(&mut self, transfer: LedgerTransfer) -> Result<bool, ServiceError> {
        match self.backend.create_transfers(&[transfer]).pop() {
            Some(Ok(())) => Ok(true),
            Some(Err(TransferError::Exists)) => Ok(false),
            Some(Err(error)) => Err(ServiceError::Cluster(error)),
            None => Err(ServiceError::Backend(
                "ledger backend returned no transfer verdict".to_owned(),
            )),
        }
    }

    /// Run the F4 reconciliation and supply audit (issue #2320,
    /// design section 7.3) over every currency this gateway
    /// serves. Report-only; unchanged from the PoC composition.
    #[must_use]
    pub fn audit(&self, recent_limit: usize) -> crate::currency::reconcile::ReconReport {
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

    /// The two-way intent↔ledger join (invariant 7): every
    /// posted intent's transfer must be on the ledger and
    /// identical to the transfer the intent stored; every failed
    /// intent's transfer must be absent; every ledger transfer
    /// stamped with a graph object this gateway recorded must
    /// belong to an intent. Findings are appended to the store's
    /// durable findings log before the report returns — a
    /// mismatch is a durable finding, never silently dropped —
    /// and the check itself repairs nothing.
    ///
    /// # Errors
    ///
    /// Fails if the intent store cannot be read or the findings
    /// cannot be persisted, or if a ledger enumeration fails.
    pub fn reconcile_intents(&mut self, run_at: u64) -> Result<IntentReconReport, ServiceError> {
        let intents = self.intents.all().map_err(ServiceError::IntentStore)?;
        let mut report = IntentReconReport {
            intents_checked: intents.len(),
            ..IntentReconReport::default()
        };
        let mut intent_transfer_ids: HashSet<u128> = HashSet::new();
        for intent in &intents {
            let Some(transfer) = &intent.transfer else {
                continue;
            };
            intent_transfer_ids.insert(transfer.id);
            match intent.state {
                IntentState::Recorded => {
                    report.findings.push(IntentFinding::UnresolvedIntent {
                        idempotency: intent.idempotency,
                        op: intent.op.clone(),
                    });
                }
                IntentState::Posted => {
                    match self.backend.try_lookup_transfer(transfer.id) {
                        Ok(Some(found)) if found == *transfer => {}
                        Ok(Some(_)) => {
                            report.findings.push(IntentFinding::PostedIntentTransferMismatch {
                                idempotency: intent.idempotency,
                                transfer_id: transfer.id,
                            });
                        }
                        Ok(None) => {
                            report.findings.push(IntentFinding::PostedIntentMissingTransfer {
                                idempotency: intent.idempotency,
                                transfer_id: transfer.id,
                            });
                        }
                        Err(error) => return Err(ServiceError::Backend(error.to_string())),
                    }
                }
                IntentState::Failed => {
                    match self.backend.try_lookup_transfer(transfer.id) {
                        Ok(Some(_)) => {
                            report.findings.push(IntentFinding::FailedIntentTransferPresent {
                                idempotency: intent.idempotency,
                                transfer_id: transfer.id,
                            });
                        }
                        Ok(None) => {}
                        Err(error) => return Err(ServiceError::Backend(error.to_string())),
                    }
                }
            }
        }
        // The orphan direction: ledger transfers carrying a
        // stamp this gateway's graph log resolves, which no
        // intent produced. Bounded per ledger
        // (ORPHAN_SCAN_LIMIT); the F4 audit is the unbounded
        // ledger↔graph checker.
        let mut ledgers: Vec<u32> = self
            .currency_ids
            .values()
            .filter_map(|currency_id| self.registry.resolve(currency_id))
            .map(|record| record.ledger_id)
            .collect();
        ledgers.sort_unstable();
        ledgers.dedup();
        for ledger in ledgers {
            let transfers = self
                .backend
                .try_transfers_on_ledger(ledger, ORPHAN_SCAN_LIMIT)
                .map_err(|error| ServiceError::Backend(error.to_string()))?;
            for transfer in transfers {
                report.ledger_transfers_checked += 1;
                if transfer.user_data_128 != 0
                    && self.graph.resolve(transfer.user_data_128).is_some()
                    && !intent_transfer_ids.contains(&transfer.id)
                {
                    report.findings.push(IntentFinding::LedgerTransferWithoutIntent {
                        transfer_id: transfer.id,
                        ledger_id: ledger,
                        user_data_128: transfer.user_data_128,
                    });
                }
            }
        }
        let records: Vec<FindingRecord> = report
            .findings
            .iter()
            .cloned()
            .map(|finding| FindingRecord { run_at, finding })
            .collect();
        self.intents
            .append_findings(&records)
            .map_err(ServiceError::IntentStore)?;
        Ok(report)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::currency::gateway::{
        CurrencyRequest, Freeze as GwFreeze, GrantMinter as GwGrantMinter, Issue as GwIssue,
        RegisterCurrency as GwRegister, Sunset as GwSunset, Transfer as GwTransfer, sign_request,
    };
    use crate::currency::gateway_poc::verify_receipt;
    use crate::currency::graph::{SupplyCap, sign_definition, sign_issue, sign_transfer};
    use crate::currency::issuer_id;
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

    struct Fixture<B: LedgerBackend, I: IntentStore> {
        svc: GatewayService<B, I>,
        gateway_vk: ed25519_dalek::VerifyingKey,
        issuer: Actor,
        alice: Actor,
        bob: Actor,
        mallory: Actor,
        nonce: u64,
    }

    fn fixture_with<B: LedgerBackend, I: IntentStore>(backend: B, store: I) -> Fixture<B, I> {
        let gateway_key = ed25519_dalek::SigningKey::from_bytes(&[250; 32]);
        let gateway_vk = gateway_key.verifying_key();
        let mut svc = GatewayService::open(backend, gateway_key, store).expect("service opens");
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

    fn send<B: LedgerBackend, I: IntentStore>(
        fx: &mut Fixture<B, I>,
        device_key: &ed25519_dalek::SigningKey,
        request: &CurrencyRequest,
        graph: Option<(&Event, &[u8])>,
    ) -> Result<SignedReceipt, ServiceError> {
        fx.nonce += 1;
        let nonce = format!("nonce-{}", fx.nonce);
        let (signed, body) =
            sign_request(device_key, request, NOW, &nonce).expect("test request signs");
        fx.svc.handle(&signed, &body, NOW, graph)
    }

    /// Register the fixture currency and record its definition;
    /// returns the receipt, the graph currency id, and the
    /// definition event.
    fn registered<B: LedgerBackend, I: IntentStore>(
        fx: &mut Fixture<B, I>,
    ) -> (SignedReceipt, CurrencyId, Event) {
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
    fn issue_to_alice<B: LedgerBackend, I: IntentStore>(
        fx: &mut Fixture<B, I>,
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

    /// One Alice → Bob transfer, presented with its graph object.
    fn transfer_alice_to_bob<B: LedgerBackend, I: IntentStore>(
        fx: &mut Fixture<B, I>,
        currency: &CurrencyId,
        client_request_id: u128,
        amount: u128,
        seq: u32,
        backlink: Option<p2panda_core::Hash>,
    ) -> Result<SignedReceipt, ServiceError> {
        let token = TokenTransfer {
            currency: currency.compact_hash(),
            amount,
            to: issuer_id(&fx.bob.account_key),
            seq_ref: seq,
            memo: None,
        };
        let event = sign_transfer(&fx.alice.account_key, seq, backlink, &token, vec![])
            .expect("transfer signs");
        let body = token.body().expect("body");
        let request = CurrencyRequest {
            client_request_id,
            op: CurrencyOp::Transfer(GwTransfer {
                currency: *currency.compact_hash().as_bytes(),
                from: fx.alice.account.clone(),
                to: fx.bob.account.clone(),
                amount,
            }),
        };
        let device_key = fx.alice.device_key.clone();
        send(fx, &device_key, &request, Some((&event, &body)))
    }

    /// A fresh empty directory for a file-backed store.
    fn temp_store_dir(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "id-gateway-service-{tag}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    /// The whole flow over the durable store: register → define →
    /// issue → transfer, with a retry answered from the intent
    /// record (not re-executed) and a clean reconciliation.
    #[test]
    fn flow_posts_once_and_replays_from_the_durable_store() {
        let dir = temp_store_dir("flow");
        let mut fx = fixture_with(
            InMemoryLedger::default(),
            FileIntentStore::open(dir.clone()).expect("store opens"),
        );
        let (register_receipt, currency, definition) = registered(&mut fx);
        assert!(verify_receipt(&fx.gateway_vk, &register_receipt));
        let key = *currency.compact_hash().as_bytes();

        let (receipt, _issue_event) = issue_to_alice(&mut fx, &currency, &definition, 2, 100, 1);
        assert!(verify_receipt(&fx.gateway_vk, &receipt));
        assert_eq!(fx.svc.holder_balance(&key, &fx.alice.account), Some(100));

        // Retry of the same client request: the stored receipt
        // answers, nothing posts twice.
        let (replay, _) = issue_to_alice(&mut fx, &currency, &definition, 2, 100, 1);
        assert_eq!(replay.receipt, receipt.receipt);
        assert_eq!(fx.svc.holder_balance(&key, &fx.alice.account), Some(100));

        let transfer_receipt =
            transfer_alice_to_bob(&mut fx, &currency, 4, 40, 0, None).expect("transfer posts");
        assert!(verify_receipt(&fx.gateway_vk, &transfer_receipt));
        assert_eq!(fx.svc.holder_balance(&key, &fx.alice.account), Some(60));
        assert_eq!(fx.svc.holder_balance(&key, &fx.bob.account), Some(40));

        // The durable record: three intents, all posted, readable
        // through a fresh store handle.
        let stored = FileIntentStore::open(dir.clone())
            .expect("store reopens")
            .all()
            .expect("intents read");
        assert_eq!(stored.len(), 3);
        assert!(stored.iter().all(|i| i.state == IntentState::Posted));

        // Invariant 7 holds on the happy path, and a clean run
        // appends nothing to the findings log.
        let report = fx.svc.reconcile_intents(NOW).expect("reconcile runs");
        assert!(report.is_clean(), "findings: {:?}", report.findings);
        assert_eq!(report.intents_checked, 3);
        let findings = fs::read_to_string(dir.join("findings.jsonl")).unwrap_or_default();
        assert!(findings.is_empty(), "clean run logged: {findings}");
        let _ = fs::remove_dir_all(&dir);
    }

    /// Restart: a new service instance over the same backend and
    /// the same intent store serves the currency without
    /// re-registration or re-recording the definition — the
    /// registry record, the key binding, the issued supply, and
    /// the minter set are all rebuilt from the intent log.
    #[test]
    fn restart_rebuilds_registry_supply_and_minters() {
        let dir = temp_store_dir("restart");
        let mut fx = fixture_with(
            InMemoryLedger::default(),
            FileIntentStore::open(dir.clone()).expect("store opens"),
        );
        let (_register, currency, definition) = registered(&mut fx);
        let key = *currency.compact_hash().as_bytes();

        // The issuer grants Alice minting rights.
        let grant = CurrencyRequest {
            client_request_id: 7,
            op: CurrencyOp::GrantMinter(GwGrantMinter {
                currency: key,
                minter: fx.alice.account.clone(),
            }),
        };
        let device_key = fx.issuer.device_key.clone();
        send(&mut fx, &device_key, &grant, None).expect("grant posts");

        let (receipt, issue_event) = issue_to_alice(&mut fx, &currency, &definition, 2, 100, 1);
        let backend = fx.svc.into_backend();

        // Restart: same backend, same store directory, fresh
        // service state.
        let mut fx = fixture_with(backend, FileIntentStore::open(dir.clone()).expect("reopen"));
        assert!(
            fx.svc.registry_record(&key).is_some(),
            "the registry record is rebuilt from the register intent",
        );

        // The recorded issue answers from the log; no double issue.
        let (replay, _) = issue_to_alice(&mut fx, &currency, &definition, 2, 100, 1);
        assert_eq!(replay.receipt, receipt.receipt);
        assert_eq!(fx.svc.holder_balance(&key, &fx.alice.account), Some(100));

        // Transfers work without re-recording the definition:
        // the key binding survived in the store.
        transfer_alice_to_bob(&mut fx, &currency, 4, 10, 0, None).expect("transfer after restart");
        assert_eq!(fx.svc.holder_balance(&key, &fx.bob.account), Some(10));

        // Issued supply was rebuilt: 100 of the 150 cap is spent,
        // so a further 60 is refused at authorization.
        let token = TokenIssue {
            currency: currency.compact_hash(),
            amount: 60,
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
        .expect("over-cap issue signs");
        let body = token.body().expect("body");
        let request = CurrencyRequest {
            client_request_id: 8,
            op: CurrencyOp::Issue(GwIssue {
                currency: key,
                amount: 60,
                to: fx.alice.account.clone(),
            }),
        };
        let device_key = fx.issuer.device_key.clone();
        let error = send(&mut fx, &device_key, &request, Some((&event, &body)))
            .expect_err("over the rebuilt cap");
        assert!(
            matches!(error, ServiceError::Gateway(GatewayError::CapExceeded)),
            "got {error:?}",
        );

        // The minter set was rebuilt: Bob (never granted) is
        // refused as a non-minter, while Alice (granted in the
        // previous life) passes authorization and is refused
        // later, at the graph check — her issue object is not
        // issuer-signed (PoC write-up item 12 stands).
        let attempts = [
            (
                fx.bob.device_key.clone(),
                fx.bob.account_key.clone(),
                fx.bob.account.clone(),
                false,
                9_u128,
            ),
            (
                fx.alice.device_key.clone(),
                fx.alice.account_key.clone(),
                fx.alice.account.clone(),
                true,
                10_u128,
            ),
        ];
        for (device_key, account_key, account, is_minter, client_request_id) in attempts {
            let token = TokenIssue {
                currency: currency.compact_hash(),
                amount: 10,
                to: issuer_id(&account_key),
                memo: None,
            };
            let event = sign_issue(&account_key, 0, None, &currency, &token, vec![])
                .expect("minter attempt signs");
            let body = token.body().expect("body");
            let request = CurrencyRequest {
                client_request_id,
                op: CurrencyOp::Issue(GwIssue {
                    currency: key,
                    amount: 10,
                    to: account,
                }),
            };
            let error = send(&mut fx, &device_key, &request, Some((&event, &body)))
                .expect_err("minter attempt cannot complete");
            if is_minter {
                assert!(
                    matches!(error, ServiceError::GraphMismatch(_)),
                    "the granted minter passes authorization, got {error:?}",
                );
            } else {
                assert!(
                    matches!(error, ServiceError::Gateway(GatewayError::NotAMinter)),
                    "the ungranted account is refused as non-minter, got {error:?}",
                );
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// Crash recovery: an intent left in `Recorded` state is
    /// resolved against the ledger on retry — finalized from the
    /// lookup when the submission landed, resubmitted under the
    /// same ID when it did not. Value moves exactly once either
    /// way (invariant 6).
    #[test]
    fn recorded_intent_recovers_after_crash() {
        let dir = temp_store_dir("crash");
        let mut fx = fixture_with(
            InMemoryLedger::default(),
            FileIntentStore::open(dir.clone()).expect("store opens"),
        );
        let (_register, currency, definition) = registered(&mut fx);
        let key = *currency.compact_hash().as_bytes();
        let (receipt, _event) = issue_to_alice(&mut fx, &currency, &definition, 2, 100, 1);

        // Simulate the crash window: the transfer posted but the
        // finalize was lost — the intent on disk is back in
        // Recorded state with no receipt.
        let mut store = FileIntentStore::open(dir.clone()).expect("store reopens");
        let mut issue_intent = store
            .all()
            .expect("intents read")
            .into_iter()
            .find(|i| i.op == "issue")
            .expect("issue intent stored");
        issue_intent.state = IntentState::Recorded;
        issue_intent.receipt = None;
        store.put(&issue_intent).expect("tamper writes");
        drop(store);

        // The retry finalizes from the ledger lookup: same
        // receipt, no second posting, supply counted once.
        let (recovered, _) = issue_to_alice(&mut fx, &currency, &definition, 2, 100, 1);
        assert_eq!(recovered.receipt, receipt.receipt);
        assert_eq!(fx.svc.holder_balance(&key, &fx.alice.account), Some(100));
        let stored = store_handle(&dir).all().expect("intents read");
        let issue_intent = stored.iter().find(|i| i.op == "issue").expect("intent");
        assert_eq!(issue_intent.state, IntentState::Posted);

        // The other half of the window: a Recorded intent whose
        // transfer never reached the ledger. Written directly to
        // the store, exactly as the crashed process left it.
        let token = TokenTransfer {
            currency: currency.compact_hash(),
            amount: 25,
            to: issuer_id(&fx.bob.account_key),
            seq_ref: 0,
            memo: None,
        };
        let event =
            sign_transfer(&fx.alice.account_key, 0, None, &token, vec![]).expect("transfer signs");
        let body = token.body().expect("body");
        let request = CurrencyRequest {
            client_request_id: 9,
            op: CurrencyOp::Transfer(GwTransfer {
                currency: key,
                from: fx.alice.account.clone(),
                to: fx.bob.account.clone(),
                amount: 25,
            }),
        };
        let idem = idempotency_id(&fx.alice.account, &request);
        let record = fx.svc.registry_record(&key).expect("record");
        let from_spec =
            id_currency_registry::account_spec(record.ledger_id, AccountKind::Holder, &fx.alice.account, "");
        let to_spec =
            id_currency_registry::account_spec(record.ledger_id, AccountKind::Holder, &fx.bob.account, "");
        let stamp = GraphLog::stamp_of(&event);
        let intent = GatewayIntent {
            idempotency: idem,
            account: fx.alice.account.clone(),
            op: "transfer".to_owned(),
            state: IntentState::Recorded,
            currency_id: Some(record.currency_id.clone()),
            currency_key: Some(hex_encode(&key)),
            ledger_id: Some(record.ledger_id),
            transfer: Some(LedgerTransfer {
                id: idem,
                debit_account_id: from_spec.id,
                credit_account_id: to_spec.id,
                amount: 25,
                ledger: record.ledger_id,
                user_data_128: stamp,
                code: id_currency_registry::factory::transfer_code::HOLDER_TRANSFER_START,
                linked: false,
            }),
            graph_stamp: Some(stamp),
            graph_object: Some(event.id.to_hex()),
            minter: None,
            registration: None,
            receipt: None,
            failure: None,
        };
        store_handle(&dir).put(&intent).expect("recorded intent writes");

        // The retry resubmits the stored transfer under the same
        // ID and finalizes.
        let device_key = fx.alice.device_key.clone();
        let recovered =
            send(&mut fx, &device_key, &request, Some((&event, &body))).expect("recovery posts");
        assert!(verify_receipt(&fx.gateway_vk, &recovered));
        assert_eq!(fx.svc.holder_balance(&key, &fx.alice.account), Some(75));
        assert_eq!(fx.svc.holder_balance(&key, &fx.bob.account), Some(25));
        let stored = store_handle(&dir).all().expect("intents read");
        let intent = stored.iter().find(|i| i.idempotency == idem).expect("intent");
        assert_eq!(intent.state, IntentState::Posted);
        let _ = fs::remove_dir_all(&dir);
    }

    fn store_handle(dir: &PathBuf) -> FileIntentStore {
        FileIntentStore::open(dir.clone()).expect("store opens")
    }

    /// A cluster refusal is a durable outcome: the intent is
    /// Failed on disk, and a retry replays the verdict instead of
    /// resubmitting.
    #[test]
    fn failed_verdict_is_durable_and_replayed() {
        let mut fx = fixture_with(InMemoryLedger::default(), InMemoryIntentStore::new());
        let (_register, currency, definition) = registered(&mut fx);
        let key = *currency.compact_hash().as_bytes();
        issue_to_alice(&mut fx, &currency, &definition, 2, 50, 1);

        // Alice holds 50 and tries to send 80: the cluster
        // refuses (the gateway submits without reading a balance).
        let error = transfer_alice_to_bob(&mut fx, &currency, 5, 80, 0, None)
            .expect_err("overdraft refused");
        assert!(matches!(error, ServiceError::Cluster(_)), "got {error:?}");

        let request = CurrencyRequest {
            client_request_id: 5,
            op: CurrencyOp::Transfer(GwTransfer {
                currency: key,
                from: fx.alice.account.clone(),
                to: fx.bob.account.clone(),
                amount: 80,
            }),
        };
        let idem = idempotency_id(&fx.alice.account, &request);
        let intent = fx
            .svc
            .intents()
            .get(idem)
            .expect("intent reads")
            .expect("failed intent recorded");
        assert_eq!(intent.state, IntentState::Failed);
        assert!(intent.failure.is_some());

        // The retry replays the stored verdict; nothing is
        // resubmitted and nothing moved.
        let error = transfer_alice_to_bob(&mut fx, &currency, 5, 80, 0, None)
            .expect_err("retry replays the verdict");
        assert!(
            matches!(error, ServiceError::StoredVerdict(_)),
            "got {error:?}",
        );
        assert_eq!(fx.svc.holder_balance(&key, &fx.bob.account), Some(0));
        assert!(
            fx.svc
                .backend()
                .try_lookup_transfer(idem)
                .expect("lookup works")
                .is_none(),
            "the refused transfer never reached the ledger",
        );
    }

    /// Invariant 7, exercised: each drift kind is found, named,
    /// and appended to the durable findings log; the check
    /// repairs nothing.
    #[test]
    fn reconcile_intents_reports_each_drift_kind() {
        let dir = temp_store_dir("recon");
        let mut fx = fixture_with(
            InMemoryLedger::default(),
            FileIntentStore::open(dir.clone()).expect("store opens"),
        );
        let (_register, currency, definition) = registered(&mut fx);
        let key = *currency.compact_hash().as_bytes();
        issue_to_alice(&mut fx, &currency, &definition, 2, 100, 1);
        transfer_alice_to_bob(&mut fx, &currency, 4, 40, 0, None).expect("transfer posts");

        let mut logged = 0_usize;
        let clean = fx.svc.reconcile_intents(NOW).expect("clean run");
        assert!(clean.is_clean(), "findings: {:?}", clean.findings);
        logged += clean.findings.len();

        let mut store = store_handle(&dir);
        let all = store.all().expect("intents read");
        let issue_intent = all.iter().find(|i| i.op == "issue").expect("issue intent").clone();
        let transfer_intent =
            all.iter().find(|i| i.op == "transfer").expect("transfer intent").clone();

        // (a) The stored transfer disagrees with the ledger's.
        let mut tampered = transfer_intent.clone();
        tampered.transfer.as_mut().expect("transfer").amount += 1;
        store.put(&tampered).expect("tamper writes");
        let report = fx.svc.reconcile_intents(NOW).expect("run a");
        assert!(
            report.findings.iter().any(|f| matches!(
                f,
                IntentFinding::PostedIntentTransferMismatch { transfer_id }
                if *transfer_id == transfer_intent.idempotency
            )),
            "run a findings: {:?}",
            report.findings,
        );
        logged += report.findings.len();
        store.put(&transfer_intent).expect("restore writes");

        // (b) An intent that never finalized.
        let mut tampered = issue_intent.clone();
        tampered.state = IntentState::Recorded;
        store.put(&tampered).expect("tamper writes");
        let report = fx.svc.reconcile_intents(NOW).expect("run b");
        assert!(
            report.findings.iter().any(|f| matches!(
                f,
                IntentFinding::UnresolvedIntent { idempotency, .. }
                if *idempotency == issue_intent.idempotency
            )),
            "run b findings: {:?}",
            report.findings,
        );
        logged += report.findings.len();
        store.put(&issue_intent).expect("restore writes");

        // (c) The intent record is gone; its ledger transfer is
        // an orphan — a ledger write without its authorization.
        fs::remove_file(
            dir.join("intents")
                .join(format!("{:032x}.json", transfer_intent.idempotency)),
        )
        .expect("intent file removed");
        let report = fx.svc.reconcile_intents(NOW).expect("run c");
        assert!(
            report.findings.iter().any(|f| matches!(
                f,
                IntentFinding::LedgerTransferWithoutIntent { transfer_id, .. }
                if *transfer_id == transfer_intent.idempotency
            )),
            "run c findings: {:?}",
            report.findings,
        );
        logged += report.findings.len();

        // (d) A posted intent whose transfer was never submitted.
        let mut fabricated = transfer_intent.clone();
        fabricated.idempotency = 0xbeef;
        fabricated.transfer.as_mut().expect("transfer").id = 0xbeef;
        store.put(&fabricated).expect("fabrication writes");
        let report = fx.svc.reconcile_intents(NOW).expect("run d");
        assert!(
            report.findings.iter().any(|f| matches!(
                f,
                IntentFinding::PostedIntentMissingTransfer { transfer_id, .. }
                if *transfer_id == 0xbeef
            )),
            "run d findings: {:?}",
            report.findings,
        );
        logged += report.findings.len();

        // Every finding from every run is in the durable log.
        let log = fs::read_to_string(dir.join("findings.jsonl")).expect("findings log exists");
        assert_eq!(log.lines().count(), logged);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Authorization refusals happen before the write-ahead
    /// point: a refused request leaves no intent behind.
    #[test]
    fn authorization_refusals_write_no_intent() {
        let mut fx = fixture_with(InMemoryLedger::default(), InMemoryIntentStore::new());
        let (_register, currency, _definition) = registered(&mut fx);
        let key = *currency.compact_hash().as_bytes();
        let count = |fx: &Fixture<InMemoryLedger, InMemoryIntentStore>| {
            fx.svc.intents().all().expect("intents read").len()
        };
        assert_eq!(count(&fx), 1, "only the register intent");

        // Mallory is neither issuer nor minter.
        let token = TokenIssue {
            currency: currency.compact_hash(),
            amount: 10,
            to: issuer_id(&fx.mallory.account_key),
            memo: None,
        };
        let event = sign_issue(&fx.issuer.account_key, 1, None, &currency, &token, vec![])
            .expect("issue signs");
        let body = token.body().expect("body");
        let request = CurrencyRequest {
            client_request_id: 2,
            op: CurrencyOp::Issue(GwIssue {
                currency: key,
                amount: 10,
                to: fx.mallory.account.clone(),
            }),
        };
        let device_key = fx.mallory.device_key.clone();
        let error = send(&mut fx, &device_key, &request, Some((&event, &body)))
            .expect_err("non-minter refused");
        assert!(
            matches!(error, ServiceError::Gateway(GatewayError::NotAMinter)),
            "got {error:?}",
        );
        assert_eq!(count(&fx), 1);

        // A single issue above the cap is refused at
        // authorization, before any intent exists.
        let token = TokenIssue {
            currency: currency.compact_hash(),
            amount: CAP + 1,
            to: issuer_id(&fx.alice.account_key),
            memo: None,
        };
        let event = sign_issue(&fx.issuer.account_key, 1, None, &currency, &token, vec![])
            .expect("issue signs");
        let body = token.body().expect("body");
        let request = CurrencyRequest {
            client_request_id: 3,
            op: CurrencyOp::Issue(GwIssue {
                currency: key,
                amount: CAP + 1,
                to: fx.alice.account.clone(),
            }),
        };
        let device_key = fx.issuer.device_key.clone();
        let error = send(&mut fx, &device_key, &request, Some((&event, &body)))
            .expect_err("over cap refused");
        assert!(
            matches!(error, ServiceError::Gateway(GatewayError::CapExceeded)),
            "got {error:?}",
        );
        assert_eq!(count(&fx), 1);
    }

    /// Ops the envelope authorizes but this service does not
    /// execute are refused by name, not silently mishandled:
    /// freeze/sunset wait on a registry status-transition write
    /// that does not exist yet.
    #[test]
    fn unsupported_ops_refuse_by_name() {
        let mut fx = fixture_with(InMemoryLedger::default(), InMemoryIntentStore::new());
        let (_register, currency, _definition) = registered(&mut fx);
        let key = *currency.compact_hash().as_bytes();
        for (id, op) in [
            (2_u128, CurrencyOp::Freeze(GwFreeze { currency: key })),
            (3_u128, CurrencyOp::Sunset(GwSunset { currency: key })),
        ] {
            let request = CurrencyRequest {
                client_request_id: id,
                op,
            };
            let device_key = fx.issuer.device_key.clone();
            let error = send(&mut fx, &device_key, &request, None)
                .expect_err("unsupported op refused");
            assert!(
                matches!(error, ServiceError::Unsupported(_)),
                "got {error:?}",
            );
        }
    }
}
