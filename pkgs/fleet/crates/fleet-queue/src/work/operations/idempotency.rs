//! Idempotency identity and stable structured receipts — REL-03
//! (developing-today/code#9513; design parent #9209, program #9196).
//!
//! Design record:
//! `doc/2026-11-25T20-47-34Z_design_rel03_idempotency_identity_receipts/`.
//!
//! This is the scoped acceptance ledger for the shared
//! `fleet.command/v1` envelope ([`Command`]). It is the pure model
//! of the record's §3–§8, like its `work` siblings: it holds receipts
//! in memory and does no I/O. Persistence (REL-04/REL-07 with the
//! #9198 store owner), delivery (REL-04) and read-back (REL-06) are
//! not part of this slice.
//!
//! Rules the model enforces:
//!
//! * **Scope tuple.** A receipt is keyed by
//!   `(principal_ref, authority_id, idempotency_key)`. The principal
//!   comes from the auth layer, never from `actor_ref`, so the same
//!   key under two principals (or two authorities) is two operations.
//! * **Lookup order (§4).** Validate, key format, digest
//!   recomputation, authorization, dry-run branch, then the scoped
//!   lookup. Every failing step returns a typed error and writes
//!   nothing.
//! * **Replay writes nothing.** Same scope and digest returns the
//!   stored receipt as it stands now. A different digest is a
//!   conflict that names only the stored `operation_id`.
//! * **Stable identity block.** The fields fixed at first acceptance
//!   serialize to the same bytes for the life of the operation; only
//!   `status`, its timestamp and effect evidence move.
//! * **Legacy records** are reported as `uncertain` and never
//!   accepted as new or reported as a matching replay.
//!
//! The DSP-01 webhook ledger in the parent module keeps its key-only
//! behavior until DSP-04 supplies the work pair for webhook commands
//! (the record's §3.2 staged step); moving it onto this ledger is the
//! remaining part of the record's §7 delta.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::work::contracts::{
    AuthenticatedPrincipal, AuthorizationDecision, Command, ContractError, Effect, Outcome,
    ResultError, evaluate_authorization, is_canonical_decimal,
};

/// The key prefix reserved for keys minted by the webhook forwarder.
pub const FORWARDER_KEY_PREFIX: &str = "ghcmd:";
/// The longest accepted idempotency key, in bytes.
pub const MAX_KEY_BYTES: usize = 200;
/// Warning attached to a replay that presented a different
/// `operation_id` than the stored one.
pub const WARNING_OPERATION_ID_SUPERSEDED: &str = "operation_id_superseded";

/// Who minted the idempotency key being presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    /// A CLI or API caller.
    Caller,
    /// The webhook forwarder (the only source allowed `ghcmd:` keys).
    WebhookForwarder,
}

/// The idempotency scope: the lookup key of the ledger (§3.1).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Scope {
    /// From the authenticated principal, never `actor_ref`.
    pub principal_ref: String,
    /// The command's `work.authority_id`.
    pub authority_id: String,
    /// The presented key, opaque beyond its format rules.
    pub idempotency_key: String,
}

/// Where the operation came from, fixed at first acceptance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// The command's `source_ref`.
    pub source_ref: String,
    /// The command's `source_event_ref`.
    pub source_event_ref: Option<String>,
    /// The command's `session_ref`.
    pub session_ref: Option<String>,
    /// The delivery that carried the first acceptance.
    pub delivery_id: Option<String>,
}

/// The immutable part of a receipt (§5.1). Its serialized bytes are
/// identical on every read for the life of the operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityBlock {
    /// The operation's identity.
    pub operation_id: String,
    /// The scope tuple.
    pub scope: Scope,
    /// The digest the ledger computed (never the carried one unchecked).
    pub request_digest: String,
    /// The command kind.
    pub kind: String,
    /// The target work, as `(authority_id, work_id)`.
    pub work_authority_id: String,
    /// The target work id (canonical decimal string).
    pub work_id: String,
    /// Ledger-monotonic acceptance sequence, as a decimal string.
    pub accepted_sequence: String,
    /// Authority clock at acceptance, verbatim.
    pub accepted_at: String,
    /// Where the operation came from.
    pub provenance: Provenance,
}

impl IdentityBlock {
    /// The block's canonical serialized bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("identity block serialization is infallible")
    }
}

/// Receipt status (REL-04's names). Unknown wire values are rejected,
/// never aliased.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptStatus {
    /// Accepted, not yet sent.
    Pending,
    /// Being delivered.
    Delivering,
    /// A delivery attempt failed in a way that may be retried.
    RetryableFailure,
    /// Remote effects confirmed.
    Confirmed,
    /// A request left and its answer was lost.
    Unknown,
    /// A competing state exists at the remote.
    Conflict,
    /// The operation failed and will not be retried.
    TerminalFailure,
}

impl ReceiptStatus {
    /// The wire spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            ReceiptStatus::Pending => "pending",
            ReceiptStatus::Delivering => "delivering",
            ReceiptStatus::RetryableFailure => "retryable_failure",
            ReceiptStatus::Confirmed => "confirmed",
            ReceiptStatus::Unknown => "unknown",
            ReceiptStatus::Conflict => "conflict",
            ReceiptStatus::TerminalFailure => "terminal_failure",
        }
    }

    /// Whether the status never moves again.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            ReceiptStatus::Confirmed | ReceiptStatus::Conflict | ReceiptStatus::TerminalFailure
        )
    }

    /// The interim transition table this model enforces until REL-04
    /// owns it: forward through delivery, `unknown` only settles by
    /// read-back (confirmed, terminal failure, or proven not sent),
    /// and a terminal status never moves.
    pub fn allows(self, to: ReceiptStatus) -> bool {
        use ReceiptStatus::*;
        matches!(
            (self, to),
            (Pending, Delivering)
                | (Pending, TerminalFailure)
                | (Delivering, Confirmed)
                | (Delivering, Unknown)
                | (Delivering, RetryableFailure)
                | (Delivering, TerminalFailure)
                | (Delivering, Conflict)
                | (RetryableFailure, Delivering)
                | (RetryableFailure, TerminalFailure)
                | (Unknown, Confirmed)
                | (Unknown, TerminalFailure)
                | (Unknown, Conflict)
                | (Unknown, RetryableFailure)
        )
    }
}

/// The stored record every result for an operation renders from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredReceipt {
    /// Fixed at first acceptance.
    pub identity: IdentityBlock,
    /// Moves only along [`ReceiptStatus::allows`].
    pub status: ReceiptStatus,
    /// When `status` last changed (authority clock, verbatim).
    pub status_changed_at: String,
    /// Requested / observed / verified evidence; append or fill only.
    #[serde(default)]
    pub effects: Vec<Effect>,
}

impl StoredReceipt {
    /// The `CommandResult.outcome` this receipt renders as (§5.2).
    pub fn outcome(&self) -> Outcome {
        let verified = self.effects.iter().filter(|e| e.verified.is_some()).count();
        match self.status {
            ReceiptStatus::Pending => Outcome::Accepted,
            ReceiptStatus::Delivering | ReceiptStatus::RetryableFailure => Outcome::PendingSync,
            ReceiptStatus::Confirmed if verified == self.effects.len() => Outcome::VerifiedComplete,
            ReceiptStatus::Confirmed => Outcome::Partial,
            ReceiptStatus::Unknown => Outcome::Uncertain,
            ReceiptStatus::Conflict => Outcome::Conflict,
            ReceiptStatus::TerminalFailure if verified > 0 => Outcome::Partial,
            ReceiptStatus::TerminalFailure => Outcome::Rejected,
        }
    }

    /// Parse a stored receipt from JSON; an unknown status or a
    /// non-decimal sequence is a schema failure.
    pub fn from_json(text: &str) -> Result<StoredReceipt, ContractError> {
        let receipt: StoredReceipt =
            serde_json::from_str(text).map_err(|err| ContractError::InvalidSchema {
                detail: format!("stored receipt does not parse: {err}"),
            })?;
        if !is_canonical_decimal(&receipt.identity.accepted_sequence) {
            return Err(ContractError::InvalidSchema {
                detail: "accepted_sequence must be a canonical decimal string".to_string(),
            });
        }
        Ok(receipt)
    }

    /// Serialize to JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("receipt serialization is infallible")
    }
}

/// A record accepted before this contract: no ledger digest, no scope.
/// Kept exactly as it was; nothing is backfilled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyRecord {
    /// The key it was stored under.
    pub idempotency_key: String,
    /// Whatever fields it had, verbatim.
    pub fields: BTreeMap<String, String>,
}

/// Why a presentation was not accepted. Each one wrote nothing.
#[derive(Debug, Clone, PartialEq)]
pub enum AcceptError {
    /// The envelope, or its carried digest, failed validation.
    Contract(ContractError),
    /// The key breaks §3.3 (length, characters, reserved prefix).
    InvalidKey {
        /// What was wrong.
        detail: String,
    },
    /// The authenticated principal may not run this command now.
    AuthDenied,
    /// The scope already holds a different intent.
    IdempotencyConflict {
        /// The stored operation, the only thing echoed.
        operation_id: String,
    },
    /// A first acceptance presented an `operation_id` another scope owns.
    OperationIdReused {
        /// The id that is already taken.
        operation_id: String,
    },
    /// The key matches a record from before this contract.
    LegacyOperation {
        /// The record as stored.
        record: LegacyRecord,
    },
}

impl AcceptError {
    /// The stable wire code.
    pub fn code(&self) -> &'static str {
        match self {
            AcceptError::Contract(error) => error.code(),
            AcceptError::InvalidKey { .. } => "invalid_schema",
            AcceptError::AuthDenied => "AUTH_DENIED",
            AcceptError::IdempotencyConflict { .. } => "IDEMPOTENCY_KEY_REUSED",
            AcceptError::OperationIdReused { .. } => "OPERATION_ID_REUSED",
            AcceptError::LegacyOperation { .. } => "LEGACY_OPERATION",
        }
    }

    /// The result outcome this failure renders as.
    pub fn outcome(&self) -> Outcome {
        match self {
            AcceptError::Contract(_) | AcceptError::InvalidKey { .. } | AcceptError::AuthDenied => {
                Outcome::Rejected
            }
            AcceptError::IdempotencyConflict { .. } | AcceptError::OperationIdReused { .. } => {
                Outcome::Conflict
            }
            AcceptError::LegacyOperation { .. } => Outcome::Uncertain,
        }
    }

    /// Whether retrying the same presentation could succeed.
    pub fn retryable(&self) -> bool {
        false
    }

    /// The suggested next action, when one exists.
    pub fn next_action(&self) -> Option<&'static str> {
        match self {
            AcceptError::IdempotencyConflict { .. } => {
                Some("use a new idempotency key for a new intent")
            }
            AcceptError::OperationIdReused { .. } => Some("use a new operation_id"),
            _ => None,
        }
    }

    /// The typed error carried inside a result envelope.
    pub fn to_result_error(&self) -> ResultError {
        ResultError {
            code: self.code().to_string(),
            message: self.to_string(),
        }
    }
}

impl fmt::Display for AcceptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AcceptError::Contract(error) => write!(f, "{error}"),
            AcceptError::InvalidKey { detail } => write!(f, "invalid_schema: {detail}"),
            AcceptError::AuthDenied => write!(f, "AUTH_DENIED"),
            AcceptError::IdempotencyConflict { operation_id } => write!(
                f,
                "IDEMPOTENCY_KEY_REUSED: the key already names operation {operation_id}"
            ),
            AcceptError::OperationIdReused { operation_id } => {
                write!(f, "OPERATION_ID_REUSED: {operation_id}")
            }
            AcceptError::LegacyOperation { record } => write!(
                f,
                "LEGACY_OPERATION: {} predates scoped receipts",
                record.idempotency_key
            ),
        }
    }
}

impl std::error::Error for AcceptError {}

/// Why a receipt update was refused. Each one wrote nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiptError {
    /// No stored operation has this id.
    UnknownOperation,
    /// The status table does not allow the move.
    TransitionRefused {
        /// The current status.
        from: ReceiptStatus,
        /// The requested status.
        to: ReceiptStatus,
    },
    /// A verified value is fixed once set.
    VerifiedValueFixed {
        /// The effect's field.
        field: String,
    },
}

impl ReceiptError {
    /// The stable code.
    pub fn code(&self) -> &'static str {
        match self {
            ReceiptError::UnknownOperation => "receipt_operation_unknown",
            ReceiptError::TransitionRefused { .. } => "receipt_transition_refused",
            ReceiptError::VerifiedValueFixed { .. } => "receipt_verified_value_fixed",
        }
    }
}

impl fmt::Display for ReceiptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReceiptError::UnknownOperation => write!(f, "{}", self.code()),
            ReceiptError::TransitionRefused { from, to } => {
                write!(f, "{}: {} -> {}", self.code(), from.as_str(), to.as_str())
            }
            ReceiptError::VerifiedValueFixed { field } => write!(f, "{}: {field}", self.code()),
        }
    }
}

impl std::error::Error for ReceiptError {}

/// A dry run's read-only verdict (§4 step 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DryRunVerdict {
    /// The scope is absent; a real presentation would accept.
    WouldAccept,
    /// Same scope and digest; a real presentation would replay.
    WouldReplay {
        /// The stored operation.
        operation_id: String,
    },
    /// Same scope, different digest.
    WouldConflict {
        /// The stored operation.
        operation_id: String,
    },
}

/// What an accepted presentation produced.
#[derive(Debug, Clone, PartialEq)]
pub enum Acceptance {
    /// First acceptance: the receipt was stored and handed to the producer.
    Accepted {
        /// The new receipt.
        receipt: StoredReceipt,
    },
    /// The scope already held this intent; nothing was written.
    Replayed {
        /// The stored receipt as it stands now.
        receipt: StoredReceipt,
        /// Non-fatal warnings, e.g. [`WARNING_OPERATION_ID_SUPERSEDED`].
        warnings: Vec<String>,
    },
    /// A dry run; nothing was written.
    DryRun(DryRunVerdict),
}

impl Acceptance {
    /// The wire `duplicate` flag.
    pub fn duplicate(&self) -> bool {
        matches!(self, Acceptance::Replayed { .. })
    }

    /// The receipt, when one is returned.
    pub fn receipt(&self) -> Option<&StoredReceipt> {
        match self {
            Acceptance::Accepted { receipt } | Acceptance::Replayed { receipt, .. } => {
                Some(receipt)
            }
            Acceptance::DryRun(_) => None,
        }
    }
}

/// Check a key against §3.3.
pub fn validate_key(key: &str, source: KeySource) -> Result<(), AcceptError> {
    if key.is_empty() || key.len() > MAX_KEY_BYTES {
        return Err(AcceptError::InvalidKey {
            detail: format!("idempotency_key must be 1 to {MAX_KEY_BYTES} bytes"),
        });
    }
    if !key.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return Err(AcceptError::InvalidKey {
            detail: "idempotency_key must be printable ASCII without whitespace".to_string(),
        });
    }
    if source != KeySource::WebhookForwarder && key.starts_with(FORWARDER_KEY_PREFIX) {
        return Err(AcceptError::InvalidKey {
            detail: format!("the `{FORWARDER_KEY_PREFIX}` prefix is reserved for the forwarder"),
        });
    }
    Ok(())
}

/// One presentation to the ledger.
#[derive(Debug, Clone, Copy)]
pub struct Presentation<'a> {
    /// The command as received.
    pub command: &'a Command,
    /// The principal the auth layer supplied, if any.
    pub principal: Option<&'a AuthenticatedPrincipal>,
    /// The capability this command kind needs.
    pub required_capability: &'a str,
    /// Who minted the key.
    pub source: KeySource,
    /// Authority clock now, verbatim; stored only on first acceptance.
    pub now: &'a str,
}

/// The scoped acceptance ledger: scope map plus operation-id index.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OperationLedger {
    receipts: BTreeMap<Scope, StoredReceipt>,
    operation_index: BTreeMap<String, Scope>,
    legacy: BTreeMap<String, LegacyRecord>,
    handed_off: Vec<String>,
    next_sequence: u64,
}

impl OperationLedger {
    /// An empty ledger.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load a record from before this contract, exactly as it was.
    pub fn import_legacy(&mut self, record: LegacyRecord) {
        self.legacy.insert(record.idempotency_key.clone(), record);
    }

    /// The receipt stored for a scope.
    pub fn receipt(&self, scope: &Scope) -> Option<&StoredReceipt> {
        self.receipts.get(scope)
    }

    /// The receipt stored for an operation id.
    pub fn receipt_by_operation(&self, operation_id: &str) -> Option<&StoredReceipt> {
        self.operation_index
            .get(operation_id)
            .and_then(|scope| self.receipts.get(scope))
    }

    /// How many operations are stored.
    pub fn len(&self) -> usize {
        self.receipts.len()
    }

    /// Whether the ledger holds no operation.
    pub fn is_empty(&self) -> bool {
        self.receipts.is_empty()
    }

    /// Operations handed to the producer, in acceptance order. Each
    /// is handed off exactly once, on first acceptance.
    pub fn handed_off(&self) -> &[String] {
        &self.handed_off
    }

    /// Run the §4 acceptance order for one presentation.
    pub fn accept(&mut self, presentation: Presentation<'_>) -> Result<Acceptance, AcceptError> {
        let command = presentation.command;
        command.validate().map_err(AcceptError::Contract)?;
        validate_key(&command.idempotency_key, presentation.source)?;
        command
            .verify_request_digest()
            .map_err(AcceptError::Contract)?;
        let digest = command
            .compute_request_digest()
            .map_err(AcceptError::Contract)?;
        let principal_ref = match evaluate_authorization(
            command,
            presentation.principal,
            presentation.required_capability,
        ) {
            AuthorizationDecision::Authorized { principal_ref } => principal_ref,
            AuthorizationDecision::Denied { .. } => return Err(AcceptError::AuthDenied),
        };
        let scope = Scope {
            principal_ref,
            authority_id: command.work.authority_id.clone(),
            idempotency_key: command.idempotency_key.clone(),
        };

        if let Some(stored) = self.receipts.get(&scope) {
            let operation_id = stored.identity.operation_id.clone();
            if stored.identity.request_digest != digest {
                return if command.dry_run {
                    Ok(Acceptance::DryRun(DryRunVerdict::WouldConflict {
                        operation_id,
                    }))
                } else {
                    Err(AcceptError::IdempotencyConflict { operation_id })
                };
            }
            if command.dry_run {
                return Ok(Acceptance::DryRun(DryRunVerdict::WouldReplay {
                    operation_id,
                }));
            }
            let mut warnings = Vec::new();
            if command.operation_id != operation_id {
                warnings.push(WARNING_OPERATION_ID_SUPERSEDED.to_string());
            }
            return Ok(Acceptance::Replayed {
                receipt: stored.clone(),
                warnings,
            });
        }

        if let Some(record) = self.legacy.get(&command.idempotency_key) {
            return Err(AcceptError::LegacyOperation {
                record: record.clone(),
            });
        }
        if self.operation_index.contains_key(&command.operation_id) {
            return Err(AcceptError::OperationIdReused {
                operation_id: command.operation_id.clone(),
            });
        }
        if command.dry_run {
            return Ok(Acceptance::DryRun(DryRunVerdict::WouldAccept));
        }

        self.next_sequence += 1;
        let receipt = StoredReceipt {
            identity: IdentityBlock {
                operation_id: command.operation_id.clone(),
                scope: scope.clone(),
                request_digest: digest,
                kind: command.kind.clone(),
                work_authority_id: command.work.authority_id.clone(),
                work_id: command.work.work_id.clone(),
                accepted_sequence: self.next_sequence.to_string(),
                accepted_at: presentation.now.to_string(),
                provenance: Provenance {
                    source_ref: command.source_ref.clone(),
                    source_event_ref: command.source_event_ref.clone(),
                    session_ref: command.session_ref.clone(),
                    delivery_id: command.delivery_id.clone(),
                },
            },
            status: ReceiptStatus::Pending,
            status_changed_at: presentation.now.to_string(),
            effects: Vec::new(),
        };
        self.operation_index
            .insert(command.operation_id.clone(), scope.clone());
        self.receipts.insert(scope, receipt.clone());
        self.handed_off.push(command.operation_id.clone());
        Ok(Acceptance::Accepted { receipt })
    }

    fn stored_mut(&mut self, operation_id: &str) -> Result<&mut StoredReceipt, ReceiptError> {
        let scope = self
            .operation_index
            .get(operation_id)
            .ok_or(ReceiptError::UnknownOperation)?;
        self.receipts
            .get_mut(scope)
            .ok_or(ReceiptError::UnknownOperation)
    }

    /// Move an operation's status. The identity block never changes.
    pub fn set_status(
        &mut self,
        operation_id: &str,
        to: ReceiptStatus,
        at: &str,
    ) -> Result<(), ReceiptError> {
        let stored = self.stored_mut(operation_id)?;
        if !stored.status.allows(to) {
            return Err(ReceiptError::TransitionRefused {
                from: stored.status,
                to,
            });
        }
        stored.status = to;
        stored.status_changed_at = at.to_string();
        Ok(())
    }

    /// Append or fill one effect's evidence. A verified value is never
    /// cleared or replaced.
    pub fn record_effect(
        &mut self,
        operation_id: &str,
        effect: Effect,
    ) -> Result<(), ReceiptError> {
        let stored = self.stored_mut(operation_id)?;
        match stored.effects.iter_mut().find(|e| e.field == effect.field) {
            None => stored.effects.push(effect),
            Some(existing) => {
                if let (Some(old), Some(new)) = (&existing.verified, &effect.verified) {
                    if old != new {
                        return Err(ReceiptError::VerifiedValueFixed {
                            field: effect.field,
                        });
                    }
                }
                if effect.requested.is_some() {
                    existing.requested = effect.requested;
                }
                if effect.observed.is_some() {
                    existing.observed = effect.observed;
                }
                if effect.verified.is_some() {
                    existing.verified = effect.verified;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::work::contracts::{COMMAND_SCHEMA_VERSION, Reason, WorkRef};

    const CAP: &str = "work.write";
    const NOW: &str = "2026-11-26T12:00:00Z";

    fn principal(name: &str) -> AuthenticatedPrincipal {
        AuthenticatedPrincipal {
            principal_ref: format!("auth:principal:{name}"),
            capabilities: vec![CAP.to_string()],
        }
    }

    fn command(operation_id: &str, key: &str, band: &str) -> Command {
        let mut command = Command {
            schema_version: COMMAND_SCHEMA_VERSION.to_string(),
            operation_id: operation_id.to_string(),
            idempotency_key: key.to_string(),
            request_digest: String::new(),
            work: WorkRef::new("fleet-local", "842").unwrap(),
            kind: "priority.set_band".to_string(),
            arguments: serde_json::json!({ "band": band, "profile_version": "3" }),
            actor_ref: "github:user:octocat".to_string(),
            source_ref: "github:developing-today/code".to_string(),
            source_event_ref: Some("github:delivery:9f2c".to_string()),
            delivery_id: Some("delivery-1".to_string()),
            source_revision: None,
            session_ref: None,
            expected_work_version: Some("41".to_string()),
            lease_generation: None,
            observed_snapshot_ref: None,
            owner_policy_ref: Some("owner-priority-policy:v3".to_string()),
            reason: Reason {
                code: "label_band_change".to_string(),
                evidence_refs: vec!["github:event:1".to_string(), "github:event:2".to_string()],
                confidence: None,
            },
            dry_run: false,
        };
        reseal(&mut command);
        command
    }

    fn reseal(command: &mut Command) {
        command.request_digest = command.compute_request_digest().unwrap();
    }

    fn present<'a>(command: &'a Command, who: &'a AuthenticatedPrincipal) -> Presentation<'a> {
        Presentation {
            command,
            principal: Some(who),
            required_capability: CAP,
            source: KeySource::Caller,
            now: NOW,
        }
    }

    fn forwarded<'a>(command: &'a Command, who: &'a AuthenticatedPrincipal) -> Presentation<'a> {
        Presentation {
            source: KeySource::WebhookForwarder,
            ..present(command, who)
        }
    }

    fn accepted(outcome: Result<Acceptance, AcceptError>) -> StoredReceipt {
        match outcome {
            Ok(Acceptance::Accepted { receipt }) => receipt,
            other => panic!("expected first acceptance, got {other:?}"),
        }
    }

    #[test]
    fn pmo_rel_same_key_same_digest_replays_receipt() {
        let mut ledger = OperationLedger::new();
        let alice = principal("alice");
        let first = command("op-1", "key-1", "next-up");
        let stored = accepted(ledger.accept(present(&first, &alice)));

        let mut again = first.clone();
        again.delivery_id = Some("delivery-2".to_string());
        let replay = ledger.accept(present(&again, &alice)).unwrap();
        assert!(replay.duplicate());
        let receipt = replay.receipt().unwrap();
        assert_eq!(receipt.identity.to_bytes(), stored.identity.to_bytes());
        assert_eq!(
            receipt.identity.provenance.delivery_id.as_deref(),
            Some("delivery-1")
        );
        assert_eq!(ledger.handed_off(), ["op-1".to_string()]);
        assert_eq!(ledger.len(), 1);
    }

    #[test]
    fn pmo_rel_same_key_different_digest_conflicts() {
        let mut ledger = OperationLedger::new();
        let alice = principal("alice");
        accepted(ledger.accept(present(&command("op-1", "key-1", "next-up"), &alice)));
        let before = ledger.clone();

        let changed = command("op-2", "key-1", "backlog");
        let error = ledger.accept(present(&changed, &alice)).unwrap_err();
        assert_eq!(
            error,
            AcceptError::IdempotencyConflict {
                operation_id: "op-1".to_string()
            }
        );
        assert_eq!(error.code(), "IDEMPOTENCY_KEY_REUSED");
        assert_eq!(error.outcome(), Outcome::Conflict);
        assert!(!error.retryable());
        assert_eq!(
            error.next_action(),
            Some("use a new idempotency key for a new intent")
        );
        assert!(!error.to_string().contains("next-up"));
        assert_eq!(ledger, before);
    }

    #[test]
    fn pmo_rel_edited_slash_command_argument_conflicts() {
        let mut ledger = OperationLedger::new();
        let bot = principal("forwarder");
        let key = "ghcmd:v1:developing-today/code#9513:comment:77:priority";
        let mut slash = command("op-slash", key, "unused");
        slash.kind = "github.slash_command.priority".to_string();
        slash.arguments = serde_json::json!({ "value": "high" });
        reseal(&mut slash);
        accepted(ledger.accept(forwarded(&slash, &bot)));

        let mut edited = slash.clone();
        edited.operation_id = "op-slash-edit".to_string();
        edited.arguments = serde_json::json!({ "value": "low" });
        reseal(&mut edited);
        let error = ledger.accept(forwarded(&edited, &bot)).unwrap_err();
        assert_eq!(error.code(), "IDEMPOTENCY_KEY_REUSED");
        assert_eq!(ledger.len(), 1);
        assert_eq!(ledger.handed_off().len(), 1);
    }

    #[test]
    fn pmo_rel_key_scope_is_per_principal_and_authority() {
        let mut ledger = OperationLedger::new();
        let alice = principal("alice");
        let bob = principal("bob");
        let a = accepted(ledger.accept(present(&command("op-a", "shared", "next-up"), &alice)));

        // Bob presents Alice's key with a different intent: his own scope.
        let b = accepted(ledger.accept(present(&command("op-b", "shared", "backlog"), &bob)));
        assert_ne!(a.identity.scope, b.identity.scope);
        assert_eq!(b.identity.scope.principal_ref, "auth:principal:bob");

        // A second authority under Alice is a third operation.
        let mut other = command("op-c", "shared", "next-up");
        other.work = WorkRef::new("fleet-remote", "842").unwrap();
        reseal(&mut other);
        let c = accepted(ledger.accept(present(&other, &alice)));
        assert_eq!(c.identity.scope.authority_id, "fleet-remote");
        assert_eq!(ledger.len(), 3);

        // actor_ref never picks the scope.
        let mut spoof = command("op-d", "shared", "next-up");
        spoof.actor_ref = "auth:principal:alice".to_string();
        reseal(&mut spoof);
        let replay = ledger.accept(present(&spoof, &bob));
        assert!(
            matches!(replay, Err(AcceptError::IdempotencyConflict { ref operation_id }) if operation_id == "op-b")
        );
    }

    #[test]
    fn pmo_rel_replay_after_revocation_is_denied_not_disclosed() {
        let mut ledger = OperationLedger::new();
        let alice = principal("alice");
        let first = command("op-1", "key-1", "next-up");
        accepted(ledger.accept(present(&first, &alice)));
        let before = ledger.clone();

        let revoked = AuthenticatedPrincipal {
            principal_ref: alice.principal_ref.clone(),
            capabilities: Vec::new(),
        };
        let error = ledger.accept(present(&first, &revoked)).unwrap_err();
        assert_eq!(error, AcceptError::AuthDenied);
        assert_eq!(error.code(), "AUTH_DENIED");
        assert!(!error.to_string().contains("op-1"));
        let unauthenticated = ledger.accept(Presentation {
            principal: None,
            ..present(&first, &alice)
        });
        assert_eq!(unauthenticated, Err(AcceptError::AuthDenied));
        assert_eq!(ledger, before);
    }

    #[test]
    fn pmo_rel_digest_ignores_key_order_and_excluded_fields() {
        let mut ledger = OperationLedger::new();
        let alice = principal("alice");
        let mut first = command("op-1", "key-1", "x");
        first.arguments =
            serde_json::from_str(r#"{"band":"next-up","profile_version":"3"}"#).unwrap();
        reseal(&mut first);
        accepted(ledger.accept(present(&first, &alice)));

        let mut reordered = first.clone();
        reordered.arguments =
            serde_json::from_str(r#"{"profile_version":"3","band":"next-up"}"#).unwrap();
        reordered.delivery_id = Some("delivery-9".to_string());
        reordered.operation_id = "op-renamed".to_string();
        reseal(&mut reordered);
        assert_eq!(reordered.request_digest, first.request_digest);
        match ledger.accept(present(&reordered, &alice)).unwrap() {
            Acceptance::Replayed { receipt, warnings } => {
                assert_eq!(receipt.identity.operation_id, "op-1");
                assert_eq!(warnings, vec![WARNING_OPERATION_ID_SUPERSEDED.to_string()]);
            }
            other => panic!("expected replay, got {other:?}"),
        }

        let mut dry = first.clone();
        dry.dry_run = true;
        reseal(&mut dry);
        assert_eq!(dry.request_digest, first.request_digest);
        assert_eq!(
            ledger.accept(present(&dry, &alice)).unwrap(),
            Acceptance::DryRun(DryRunVerdict::WouldReplay {
                operation_id: "op-1".to_string()
            })
        );
    }

    #[test]
    fn pmo_rel_digest_changes_with_array_order_and_preconditions() {
        let mut ledger = OperationLedger::new();
        let alice = principal("alice");
        let first = command("op-1", "key-1", "next-up");
        accepted(ledger.accept(present(&first, &alice)));

        let mut reordered = first.clone();
        reordered.operation_id = "op-2".to_string();
        reordered.reason.evidence_refs.reverse();
        reseal(&mut reordered);
        assert_ne!(reordered.request_digest, first.request_digest);
        assert_eq!(
            ledger
                .accept(present(&reordered, &alice))
                .unwrap_err()
                .code(),
            "IDEMPOTENCY_KEY_REUSED"
        );

        let mut newer = first.clone();
        newer.operation_id = "op-3".to_string();
        newer.expected_work_version = Some("42".to_string());
        reseal(&mut newer);
        assert_ne!(newer.request_digest, first.request_digest);
        assert_eq!(
            ledger.accept(present(&newer, &alice)).unwrap_err().code(),
            "IDEMPOTENCY_KEY_REUSED"
        );
        assert_eq!(ledger.len(), 1);
    }

    #[test]
    fn pmo_rel_carried_digest_mismatch_rejected_before_lookup() {
        let mut ledger = OperationLedger::new();
        let alice = principal("alice");
        let first = command("op-1", "key-1", "next-up");
        accepted(ledger.accept(present(&first, &alice)));
        let before = ledger.clone();

        let mut forged = command("op-2", "key-1", "backlog");
        forged.request_digest = first.request_digest.clone();
        let error = ledger.accept(present(&forged, &alice)).unwrap_err();
        assert_eq!(error.code(), "invalid_schema");
        assert_eq!(error.outcome(), Outcome::Rejected);

        let mut fresh = command("op-3", "key-new", "next-up");
        fresh.request_digest = format!("sha256:{}", "a".repeat(64));
        assert_eq!(
            ledger.accept(present(&fresh, &alice)).unwrap_err().code(),
            "invalid_schema"
        );
        assert_eq!(ledger, before);
    }

    #[test]
    fn pmo_rel_timeout_after_side_effect_replays_uncertain_not_new() {
        let mut ledger = OperationLedger::new();
        let alice = principal("alice");
        let merge = command("op-merge", "merge-1", "next-up");
        accepted(ledger.accept(present(&merge, &alice)));
        ledger
            .set_status("op-merge", ReceiptStatus::Delivering, "t1")
            .unwrap();
        ledger
            .set_status("op-merge", ReceiptStatus::Unknown, "t2")
            .unwrap();

        let mut retry = merge.clone();
        retry.delivery_id = Some("delivery-retry".to_string());
        let replay = ledger.accept(present(&retry, &alice)).unwrap();
        let receipt = replay.receipt().unwrap();
        assert!(replay.duplicate());
        assert_eq!(receipt.identity.operation_id, "op-merge");
        assert_eq!(receipt.status, ReceiptStatus::Unknown);
        assert_eq!(receipt.outcome(), Outcome::Uncertain);
        assert_eq!(ledger.handed_off(), ["op-merge".to_string()]);
        assert_eq!(ledger.len(), 1);
    }

    #[test]
    fn pmo_rel_replay_writes_nothing() {
        let mut ledger = OperationLedger::new();
        let alice = principal("alice");
        let first = command("op-1", "key-1", "next-up");
        accepted(ledger.accept(present(&first, &alice)));
        ledger
            .set_status("op-1", ReceiptStatus::Delivering, "t1")
            .unwrap();
        let before = ledger.clone();
        let before_debug = format!("{before:?}");
        for attempt in 0..25 {
            let mut again = first.clone();
            again.delivery_id = Some(format!("delivery-{attempt}"));
            again.operation_id = format!("op-retry-{attempt}");
            reseal(&mut again);
            let replay = ledger
                .accept(Presentation {
                    now: "2027-01-01T00:00:00Z",
                    ..present(&again, &alice)
                })
                .unwrap();
            assert!(replay.duplicate());
            assert_eq!(replay.receipt().unwrap().outcome(), Outcome::PendingSync);
        }
        assert_eq!(ledger, before);
        assert_eq!(format!("{ledger:?}"), before_debug);
    }

    #[test]
    fn pmo_rel_dry_run_creates_no_operation() {
        let mut ledger = OperationLedger::new();
        let alice = principal("alice");
        let mut probe = command("op-1", "key-1", "next-up");
        probe.dry_run = true;
        reseal(&mut probe);
        let empty = ledger.clone();
        assert_eq!(
            ledger.accept(present(&probe, &alice)).unwrap(),
            Acceptance::DryRun(DryRunVerdict::WouldAccept)
        );
        assert_eq!(ledger, empty);
        assert!(ledger.is_empty());

        accepted(ledger.accept(present(&command("op-1", "key-1", "next-up"), &alice)));
        let before = ledger.clone();
        assert_eq!(
            ledger.accept(present(&probe, &alice)).unwrap(),
            Acceptance::DryRun(DryRunVerdict::WouldReplay {
                operation_id: "op-1".to_string()
            })
        );
        let mut conflicting = command("op-2", "key-1", "backlog");
        conflicting.dry_run = true;
        reseal(&mut conflicting);
        assert_eq!(
            ledger.accept(present(&conflicting, &alice)).unwrap(),
            Acceptance::DryRun(DryRunVerdict::WouldConflict {
                operation_id: "op-1".to_string()
            })
        );
        assert_eq!(ledger, before);
        assert_eq!(ledger.handed_off().len(), 1);
    }

    #[test]
    fn pmo_rel_operation_id_collision_conflicts() {
        let mut ledger = OperationLedger::new();
        let alice = principal("alice");
        accepted(ledger.accept(present(&command("op-1", "key-1", "next-up"), &alice)));
        let before = ledger.clone();

        let reuse = command("op-1", "key-2", "backlog");
        let error = ledger.accept(present(&reuse, &alice)).unwrap_err();
        assert_eq!(
            error,
            AcceptError::OperationIdReused {
                operation_id: "op-1".to_string()
            }
        );
        assert_eq!(error.code(), "OPERATION_ID_REUSED");
        assert_eq!(error.outcome(), Outcome::Conflict);
        assert_eq!(ledger, before);

        let mut renamed = command("op-other", "key-1", "next-up");
        reseal(&mut renamed);
        match ledger.accept(present(&renamed, &alice)).unwrap() {
            Acceptance::Replayed { receipt, warnings } => {
                assert_eq!(receipt.identity.operation_id, "op-1");
                assert_eq!(warnings, vec![WARNING_OPERATION_ID_SUPERSEDED.to_string()]);
            }
            other => panic!("expected replay, got {other:?}"),
        }
    }

    #[test]
    fn pmo_rel_forwarder_key_prefix_reserved() {
        let mut ledger = OperationLedger::new();
        let alice = principal("alice");
        let forged = command(
            "op-1",
            "ghcmd:v1:developing-today/code#1:comment:2:park",
            "x",
        );
        let error = ledger.accept(present(&forged, &alice)).unwrap_err();
        assert_eq!(error.code(), "invalid_schema");
        assert!(ledger.is_empty());
        accepted(ledger.accept(forwarded(&forged, &alice)));

        for bad in ["", "has space", "tab\tkey", "ünicode"] {
            assert!(matches!(
                validate_key(bad, KeySource::Caller),
                Err(AcceptError::InvalidKey { .. })
            ));
        }
        assert!(validate_key(&"k".repeat(MAX_KEY_BYTES), KeySource::Caller).is_ok());
        assert!(validate_key(&"k".repeat(MAX_KEY_BYTES + 1), KeySource::Caller).is_err());
    }

    #[test]
    fn pmo_rel_legacy_operation_never_fabricates_receipt() {
        let mut ledger = OperationLedger::new();
        let legacy = LegacyRecord {
            idempotency_key: "old-key".to_string(),
            fields: BTreeMap::from([
                ("status".to_string(), "legacy_unknown".to_string()),
                ("verb".to_string(), "park".to_string()),
            ]),
        };
        ledger.import_legacy(legacy.clone());
        let before = ledger.clone();
        let alice = principal("alice");
        let error = ledger
            .accept(present(&command("op-1", "old-key", "next-up"), &alice))
            .unwrap_err();
        assert_eq!(error.code(), "LEGACY_OPERATION");
        assert_eq!(error.outcome(), Outcome::Uncertain);
        assert_eq!(error, AcceptError::LegacyOperation { record: legacy });
        assert_eq!(ledger, before);
        assert!(ledger.handed_off().is_empty());
    }

    #[test]
    fn pmo_rel_receipt_identity_stable_across_status_changes() {
        let mut ledger = OperationLedger::new();
        let alice = principal("alice");
        let stored = accepted(ledger.accept(present(&command("op-1", "key-1", "next-up"), &alice)));
        let bytes = stored.identity.to_bytes();
        let path = [
            ReceiptStatus::Delivering,
            ReceiptStatus::Unknown,
            ReceiptStatus::Confirmed,
        ];
        for (step, status) in path.into_iter().enumerate() {
            ledger
                .set_status("op-1", status, &format!("t{step}"))
                .unwrap();
            let now = ledger.receipt_by_operation("op-1").unwrap();
            assert_eq!(now.identity.to_bytes(), bytes);
            assert_eq!(now.status, status);
        }
        ledger
            .record_effect(
                "op-1",
                Effect {
                    field: "band".to_string(),
                    requested: Some("next-up".to_string()),
                    observed: None,
                    verified: None,
                },
            )
            .unwrap();
        let partial = ledger.receipt_by_operation("op-1").unwrap();
        assert_eq!(partial.outcome(), Outcome::Partial);
        ledger
            .record_effect(
                "op-1",
                Effect {
                    field: "band".to_string(),
                    requested: None,
                    observed: Some("next-up".to_string()),
                    verified: Some("next-up".to_string()),
                },
            )
            .unwrap();
        let done = ledger.receipt_by_operation("op-1").unwrap();
        assert_eq!(done.outcome(), Outcome::VerifiedComplete);
        assert_eq!(done.effects[0].requested.as_deref(), Some("next-up"));
        assert_eq!(done.identity.to_bytes(), bytes);

        assert_eq!(
            ledger.record_effect(
                "op-1",
                Effect {
                    field: "band".to_string(),
                    requested: None,
                    observed: None,
                    verified: Some("backlog".to_string()),
                },
            ),
            Err(ReceiptError::VerifiedValueFixed {
                field: "band".to_string()
            })
        );
        assert_eq!(
            ledger.set_status("op-1", ReceiptStatus::Delivering, "t9"),
            Err(ReceiptError::TransitionRefused {
                from: ReceiptStatus::Confirmed,
                to: ReceiptStatus::Delivering,
            })
        );
        assert_eq!(
            ledger.set_status("op-missing", ReceiptStatus::Delivering, "t9"),
            Err(ReceiptError::UnknownOperation)
        );
    }

    #[test]
    fn pmo_rel_outcome_rendering_follows_status_table() {
        let alice = principal("alice");
        let mut ledger = OperationLedger::new();
        let mut receipt =
            accepted(ledger.accept(present(&command("op-1", "key-1", "next-up"), &alice)));
        let cases = [
            (ReceiptStatus::Pending, Outcome::Accepted),
            (ReceiptStatus::Delivering, Outcome::PendingSync),
            (ReceiptStatus::RetryableFailure, Outcome::PendingSync),
            (ReceiptStatus::Confirmed, Outcome::VerifiedComplete),
            (ReceiptStatus::Unknown, Outcome::Uncertain),
            (ReceiptStatus::Conflict, Outcome::Conflict),
            (ReceiptStatus::TerminalFailure, Outcome::Rejected),
        ];
        for (status, outcome) in cases {
            receipt.status = status;
            assert_eq!(receipt.outcome(), outcome, "{}", status.as_str());
        }
        receipt.effects.push(Effect {
            field: "band".to_string(),
            requested: Some("next-up".to_string()),
            observed: Some("next-up".to_string()),
            verified: Some("next-up".to_string()),
        });
        receipt.status = ReceiptStatus::TerminalFailure;
        assert_eq!(receipt.outcome(), Outcome::Partial);
    }

    #[test]
    fn pmo_rel_concurrent_first_acceptance_single_operation() {
        let ledger = Arc::new(Mutex::new(OperationLedger::new()));
        let alice = principal("alice");
        let handles: Vec<_> = (0..8)
            .map(|worker| {
                let ledger = Arc::clone(&ledger);
                let alice = alice.clone();
                std::thread::spawn(move || {
                    // Half the workers present the same intent, half a
                    // different one, all under one scope.
                    let band = if worker % 2 == 0 {
                        "next-up"
                    } else {
                        "backlog"
                    };
                    let mine = command(&format!("op-{worker}"), "race-key", band);
                    let result = ledger.lock().unwrap().accept(present(&mine, &alice));
                    (band, result)
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let ledger = ledger.lock().unwrap();
        assert_eq!(ledger.len(), 1);
        assert_eq!(ledger.handed_off().len(), 1);
        let winner = ledger.handed_off()[0].clone();
        let winner_digest = ledger
            .receipt_by_operation(&winner)
            .unwrap()
            .identity
            .request_digest
            .clone();
        let mut firsts = 0;
        for (band, result) in results {
            let presented = command("probe", "race-key", band).request_digest;
            match result {
                Ok(Acceptance::Accepted { receipt }) => {
                    firsts += 1;
                    assert_eq!(receipt.identity.operation_id, winner);
                }
                Ok(Acceptance::Replayed { receipt, .. }) => {
                    assert_eq!(receipt.identity.operation_id, winner);
                    assert_eq!(presented, winner_digest);
                }
                Err(AcceptError::IdempotencyConflict { operation_id }) => {
                    assert_eq!(operation_id, winner);
                    assert_ne!(presented, winner_digest);
                }
                other => panic!("unexpected race outcome {other:?}"),
            }
        }
        assert_eq!(firsts, 1);
    }

    #[test]
    fn pmo_rel_receipt_fixture_round_trips() {
        let golden = r#"{"identity":{"operation_id":"op_fixture_1","scope":{"principal_ref":"auth:principal:drew","authority_id":"fleet-local","idempotency_key":"key-1"},"request_digest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","kind":"priority.set_band","work_authority_id":"fleet-local","work_id":"842","accepted_sequence":"18446744073709551616","accepted_at":"2026-11-26T12:00:00Z","provenance":{"source_ref":"github:developing-today/code","source_event_ref":null,"session_ref":null,"delivery_id":"delivery-1"}},"status":"retryable_failure","status_changed_at":"2026-11-26T12:01:00Z","effects":[{"field":"band","requested":"next-up","observed":null,"verified":null}]}"#;
        let receipt = StoredReceipt::from_json(golden).unwrap();
        assert_eq!(receipt.status, ReceiptStatus::RetryableFailure);
        assert_eq!(receipt.identity.accepted_sequence, "18446744073709551616");
        assert_eq!(receipt.to_json(), golden);

        let unknown_status = golden.replace("\"retryable_failure\"", "\"retrying\"");
        assert_eq!(
            StoredReceipt::from_json(&unknown_status)
                .unwrap_err()
                .code(),
            "invalid_schema"
        );
        let camel = golden.replace("\"retryable_failure\"", "\"RetryableFailure\"");
        assert!(StoredReceipt::from_json(&camel).is_err());
        let numeric = golden.replace("\"18446744073709551616\"", "7");
        assert!(StoredReceipt::from_json(&numeric).is_err());
        let padded = golden.replace("\"18446744073709551616\"", "\"07\"");
        assert!(StoredReceipt::from_json(&padded).is_err());
    }

    /// Reference-model oracle (§9): an explicit scope map plus
    /// operation-id index, driven by a deterministic generator over
    /// accept / replay / conflict / revoke / status steps and compared
    /// with the ledger after every step.
    #[test]
    fn pmo_rel_reference_model_agrees_on_generated_sequences() {
        type ModelKey = (String, String, String);
        for seed in 1u64..=40 {
            let mut state = seed;
            let mut next = move |bound: u64| {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (state >> 33) % bound
            };
            let mut ledger = OperationLedger::new();
            let mut model: BTreeMap<ModelKey, (String, String)> = BTreeMap::new();
            let mut ids: BTreeMap<String, ModelKey> = BTreeMap::new();
            for step in 0..60u64 {
                let who = ["alice", "bob"][next(2) as usize];
                let key = format!("k{}", next(3));
                let band = ["next-up", "backlog"][next(2) as usize];
                let op = format!("op-{}", next(12));
                let revoked = next(10) == 0;
                let mut cmd = command(&op, &key, band);
                if next(4) == 0 {
                    cmd.work = WorkRef::new("fleet-remote", "842").unwrap();
                    reseal(&mut cmd);
                }
                let mut actor = principal(who);
                if revoked {
                    actor.capabilities.clear();
                }
                let result = ledger.accept(present(&cmd, &actor));
                let scope: ModelKey = (
                    actor.principal_ref.clone(),
                    cmd.work.authority_id.clone(),
                    key.clone(),
                );
                let expected = if revoked {
                    "denied"
                } else if let Some((digest, _)) = model.get(&scope) {
                    if *digest == cmd.request_digest {
                        "replay"
                    } else {
                        "conflict"
                    }
                } else if ids.contains_key(&op) {
                    "op_reused"
                } else {
                    model.insert(scope.clone(), (cmd.request_digest.clone(), op.clone()));
                    ids.insert(op.clone(), scope.clone());
                    "accept"
                };
                let observed = match &result {
                    Ok(Acceptance::Accepted { .. }) => "accept",
                    Ok(Acceptance::Replayed { .. }) => "replay",
                    Err(AcceptError::IdempotencyConflict { .. }) => "conflict",
                    Err(AcceptError::OperationIdReused { .. }) => "op_reused",
                    Err(AcceptError::AuthDenied) => "denied",
                    other => panic!("seed {seed} step {step}: unexpected {other:?}"),
                };
                assert_eq!(observed, expected, "seed {seed} step {step}");

                if next(5) == 0 {
                    if let Some(id) = ids.keys().nth(next(ids.len().max(1) as u64) as usize) {
                        let before = ledger.receipt_by_operation(id).unwrap().clone();
                        let to = [
                            ReceiptStatus::Delivering,
                            ReceiptStatus::Unknown,
                            ReceiptStatus::Confirmed,
                        ][next(3) as usize];
                        let moved = ledger.set_status(id, to, "t");
                        let after = ledger.receipt_by_operation(id).unwrap();
                        assert_eq!(moved.is_ok(), before.status.allows(to));
                        assert_eq!(after.identity, before.identity);
                    }
                }

                assert_eq!(ledger.len(), model.len());
                assert_eq!(ledger.handed_off().len(), model.len());
                for (scope, (digest, operation_id)) in &model {
                    let stored = ledger
                        .receipt(&Scope {
                            principal_ref: scope.0.clone(),
                            authority_id: scope.1.clone(),
                            idempotency_key: scope.2.clone(),
                        })
                        .unwrap();
                    assert_eq!(&stored.identity.request_digest, digest);
                    assert_eq!(&stored.identity.operation_id, operation_id);
                    assert_eq!(
                        ledger
                            .receipt_by_operation(operation_id)
                            .unwrap()
                            .identity
                            .scope,
                        stored.identity.scope
                    );
                }
            }
        }
    }
}
