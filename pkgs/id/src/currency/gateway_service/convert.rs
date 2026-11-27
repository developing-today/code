//! `convert` through the gateway service (#14393, slice 1 of
//! #5536; design `doc/2026-11-01_design_id2063.md` §2.2–§2.4).
//!
//! A conversion is the one value op that cannot be authorized
//! from the F3 request alone: it executes at a rate an LP signed,
//! so the request presents that rate authorisation (and any
//! revocations) as signed chain objects, alongside the signed
//! envelope, the way `issue` and `transfer` present their graph
//! objects. [`GatewayService::handle_convert`] runs the request
//! through the same pipeline as every other write:
//!
//! 1. **Envelope, idempotency, F3 authorization.** The envelope
//!    verifies, the device resolves to its account, and a retry of
//!    a request with a recorded intent is answered from that
//!    record — before anything else runs. F3 then authorizes the
//!    op (active source currency, positive amount, the caller
//!    controls `from`); the service adds the destination's side
//!    (known, active, not the source).
//! 2. **Conversion verification (#3801/#3802).** The presented
//!    authorisation is verified by the gateway's
//!    [`ConversionVerifier`] against the production
//!    [`GatewayConversionControl`] binding: signatures, LP tier,
//!    the LP's account binding on both ledgers, revocation,
//!    window, caller, min/max and remaining cap. A request naming
//!    a quote executes exactly the quoted amounts. The folded
//!    authorisation must be for exactly the requested pair. Every
//!    refusal here writes nothing: no intent, no ledger change.
//! 3. **Write-ahead.** The pure layer builds the one linked chain
//!    ([`build_conversion_batch`]: source leg, optional fee leg,
//!    destination leg, every leg stamped with the conversion id in
//!    `user_data_128`) with transfer ids derived once from the
//!    idempotency id. The intent — every leg, the amounts, the
//!    authorisation it consumes — is persisted *before* the chain
//!    is submitted, and the volume is reserved against the
//!    authorisation's cap in the same step.
//! 4. **One atomic submission.** Every leg created: posted. Every
//!    leg already present and identical: posted (an earlier
//!    submission landed). A definite cluster refusal: the intent
//!    is marked failed with the verdict and the reservation is
//!    released. Anything else — a lost answer, a mix of present
//!    and absent legs, a leg with other contents — leaves the
//!    intent recorded for recovery, which never mints new ids.
//!
//! **Recovery** (a retry of a recorded conversion) looks every
//! leg up by id: all present and identical finalizes as posted,
//! none present resubmits the identical chain, and a partial or
//! conflicting chain is surfaced as [`ServiceError::IntentConflict`]
//! and stays recorded (quarantined) for the operator — a linked
//! chain is atomic, so a partial one means another writer touched
//! these ids. [`GatewayService::reconcile_intents`] joins every
//! leg against the ledger like any other intent transfer, and
//! [`GatewayService::open`] rebuilds the usage book from the
//! intent log (posted conversions, and recorded ones whose
//! reservation is still held).
//!
//! **Production control binding.** [`GatewayConversionControl`]
//! is the [`ConversionControl`] seam over the service's own
//! registry and ledger. An authorisation names its ledgers by the
//! currencies' graph keys (lower-case hex), the identity every
//! other gateway op uses. An LP is eligible as a
//! [`LpTier::CurrencyIssuer`] when it issues either currency, or
//! as a [`LpTier::RegisteredLiquidityHolder`] when it already
//! holds an account on both ledgers. Its conversion inventory on
//! each side is its own **holder** account — the factory's
//! constrained kind (`debits_must_not_exceed_credits`, history
//! on) — so the destination leg is guarded by the cluster, not by
//! a balance read, and no conversion can pay out value that was
//! never issued under the destination's supply cap. This is the
//! design's "constrained inventory" requirement met with the
//! existing constrained kind; the shared, unconstrained
//! `liquidity` kind is never treated as inventory. Fees go to the
//! source currency issuer's `fee` account. The registry crate
//! still keeps no account read-back (the F1 gap), so the binding
//! derives accounts from the factory and asks the ledger whether
//! they exist; a currency that is frozen or sunset reads as a
//! closing or closed binding.
//!
//! **Quotes** are held in the verifier's in-memory book with a
//! 60-second lifetime; a restart drops them, and a conversion
//! naming a dropped quote is refused as an unknown quote before
//! any write.
//!
//! **Conversion receipts** (#15830, Phase G of #5536; design
//! #2063 §3 invariants 10–11). A conversion answers with the
//! generic signed [`Receipt`](crate::currency::gateway_poc::Receipt)
//! like every other op — that shared wire format is unchanged.
//! Alongside it, [`GatewayService::conversion_receipt`] builds a
//! [`ConversionReceipt`] from the stored intent: the conversion
//! id, the authorisation id and exact offer version it executed
//! under, the quote id when any, every leg's transfer id with its
//! role, both currencies and ledgers, the three amounts, and the
//! outcome — so a posted or reversed conversion is auditable from
//! its receipt to every ledger leg without reading the intent
//! log's internals. A conversion executed as a **reversal** is a
//! separate conversion whose presentation names the original
//! conversion id ([`ConversionPresentation::reversal_of`]); the
//! reference is carried on the record and the receipt.

use std::collections::BTreeMap;

use id_currency_registry::{
    AccountKind, CurrencyRecord, CurrencyStatus, InMemoryStore, Registry, account_spec,
};
use serde::{Deserialize, Serialize};

use super::{GatewayIntent, GatewayService, IntentState, IntentStore, ServiceError, decode_key};
use crate::conversion::{
    CODE_CONVERSION_DESTINATION, CODE_CONVERSION_FEE, CODE_CONVERSION_SOURCE,
    ConversionBatchRequest, build_conversion_batch, fee_amount,
};
use crate::currency::IssuerId;
use crate::currency::conversion::{
    AccountStatus, BindingRecord, ConversionControl, ConversionQuote, ConversionSubmission, LpTier,
    PresentedObject,
};
use crate::currency::gateway::{
    Authorized, Convert, CurrencyOp, DeviceResolver, GatewayError, authorize, idempotency_id,
    verify_envelope,
};
use crate::currency::gateway_poc::{RegistryView, SignedReceipt, sign_receipt_with};
use crate::currency_poc::ledger::{LedgerBackend, TransferError};
use crate::directory_auth::Signed;
use crate::hosted_binding::hex_encode;

/// What a `convert` request presents beside its signed envelope:
/// the rate authorisation it executes under, as the LP's signed
/// chain objects, and optionally a quote this gateway issued.
#[derive(Clone, Debug)]
pub struct ConversionPresentation {
    /// The LP (the authorisation's issuer).
    pub issuer: IssuerId,
    /// The authorisation the conversion relies on.
    pub authorisation_id: u128,
    /// Every authorisation version presented, as signed objects.
    pub authorisations: Vec<PresentedObject>,
    /// Every revocation presented, as signed objects.
    pub revokes: Vec<PresentedObject>,
    /// A quote issued by [`GatewayService::quote_convert`]; when
    /// present the conversion executes exactly its amounts.
    pub quote_id: Option<u128>,
    /// The conversion this one reverses, when it is executed as
    /// a reversal (design invariant 10): a reversal is a separate
    /// conversion, and its record and receipt name the original
    /// conversion's id so the pair is auditable in both
    /// directions. The reference is declared by the presenter; it
    /// is an audit link, not an authorization input.
    pub reversal_of: Option<u128>,
}

/// The terms of one conversion, kept on its intent so a restart
/// can rebuild the authorisation usage book and an operator can
/// read what the chain committed to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversionRecord {
    /// The LP's id (hex of its key).
    pub lp: String,
    /// The authorisation the conversion consumed.
    pub authorisation_id: u128,
    /// The authorisation version it executed under.
    pub authorisation_version: u64,
    /// The quote it executed under, if any.
    pub quote_id: Option<u128>,
    /// The destination registry `currency_id`.
    pub dest_currency_id: String,
    /// The destination graph currency key (hex).
    pub dest_currency_key: String,
    /// The destination ledger.
    pub dest_ledger_id: u32,
    /// Source principal, in source raw units.
    pub amount_in: u128,
    /// Source fee, in source raw units.
    pub fee: u128,
    /// Destination amount, in destination raw units.
    pub amount_out: u128,
    /// The conversion this one reverses, when it was executed as
    /// a reversal (design invariant 10). Absent on records stored
    /// before the field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reversal_of: Option<u128>,
}

/// The role of one leg of a conversion chain, in chain order:
/// the caller's principal moves to the LP on the source leg, the
/// fee (when any) is paid on its own leg, and the LP pays the
/// caller on the destination leg.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversionLegRole {
    /// Caller → LP, source currency, `amount_in`.
    Source,
    /// Caller → the source issuer's fee account, `fee`.
    Fee,
    /// LP → caller, destination currency, `amount_out`.
    Destination,
}

/// One ledger leg of a conversion, as its [`ConversionReceipt`]
/// names it: enough to look the transfer up on its ledger.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversionLegRef {
    /// The leg's role in the chain.
    pub role: ConversionLegRole,
    /// The leg's ledger transfer id.
    pub transfer_id: u128,
    /// The ledger the leg sits on.
    pub ledger_id: u32,
    /// The leg's amount, in its ledger's raw units.
    pub amount: u128,
}

/// The outcome of a conversion, as its [`ConversionReceipt`]
/// reports it: the intent's lifecycle state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversionOutcome {
    /// Written ahead, not yet finalized (a lost answer recovery
    /// has not resolved it).
    Recorded,
    /// Every leg is on the ledger.
    Posted,
    /// The cluster refused the chain; nothing posted.
    Failed,
}

/// The conversion-specific receipt body (#15830, Phase G of
/// #5536; design #2063 §3 invariant 11): everything a receipt
/// holder needs to audit one conversion from the receipt to every
/// ledger leg. It accompanies the generic signed
/// [`Receipt`](crate::currency::gateway_poc::Receipt) a conversion
/// answers with — that shared wire format is unchanged — and is
/// rebuilt deterministically from the conversion's stored intent,
/// so it reads identically on the posting path, on a replay, and
/// after a restart.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversionReceipt {
    /// The conversion id (the intent's idempotency id, stamped
    /// in `user_data_128` on every leg).
    pub conversion_id: u128,
    /// The LP's id (hex of its key).
    pub lp: String,
    /// The authorisation the conversion consumed.
    pub authorisation_id: u128,
    /// The exact offer (authorisation) version it executed under.
    pub authorisation_version: u64,
    /// The quote it executed under, if any.
    pub quote_id: Option<u128>,
    /// Every leg of the chain, in chain order, each with its
    /// transfer id, ledger and amount.
    pub legs: Vec<ConversionLegRef>,
    /// The source registry `currency_id`.
    pub source_currency_id: Option<String>,
    /// The source graph currency key (hex).
    pub source_currency_key: Option<String>,
    /// The source ledger.
    pub source_ledger_id: Option<u32>,
    /// The destination registry `currency_id`.
    pub dest_currency_id: String,
    /// The destination graph currency key (hex).
    pub dest_currency_key: String,
    /// The destination ledger.
    pub dest_ledger_id: u32,
    /// Source principal, in source raw units.
    pub amount_in: u128,
    /// Source fee, in source raw units.
    pub fee: u128,
    /// Destination amount, in destination raw units.
    pub amount_out: u128,
    /// The conversion's outcome.
    pub outcome: ConversionOutcome,
    /// The conversion this one reverses, when it was executed as
    /// a reversal (design invariant 10).
    pub reversal_of: Option<u128>,
}

/// The role a stored leg plays in its chain: the leg's transfer
/// code names it (the batch builder stamps one code per role);
/// the positional reading of the chain (source first, destination
/// last, fee between) covers a leg whose code predates the codes.
fn leg_role(code: u16, index: usize, count: usize) -> ConversionLegRole {
    match code {
        CODE_CONVERSION_SOURCE => ConversionLegRole::Source,
        CODE_CONVERSION_FEE => ConversionLegRole::Fee,
        CODE_CONVERSION_DESTINATION => ConversionLegRole::Destination,
        _ if index == 0 => ConversionLegRole::Source,
        _ if index + 1 == count => ConversionLegRole::Destination,
        _ => ConversionLegRole::Fee,
    }
}

impl GatewayIntent {
    /// The [`ConversionReceipt`] this intent answers for, when it
    /// records a conversion: the intent's terms and stored chain,
    /// read as the Phase G receipt (module docs). `None` for
    /// every other op, and for a conversion intent whose terms
    /// were never recorded.
    #[must_use]
    pub fn conversion_receipt(&self) -> Option<ConversionReceipt> {
        let record = self.conversion.as_ref()?;
        let count = self.legs.len();
        let legs = self
            .legs
            .iter()
            .enumerate()
            .map(|(index, leg)| ConversionLegRef {
                role: leg_role(leg.code, index, count),
                transfer_id: leg.id,
                ledger_id: leg.ledger,
                amount: leg.amount,
            })
            .collect();
        let outcome = match self.state {
            IntentState::Recorded => ConversionOutcome::Recorded,
            IntentState::Posted => ConversionOutcome::Posted,
            IntentState::Failed => ConversionOutcome::Failed,
        };
        Some(ConversionReceipt {
            conversion_id: self.idempotency,
            lp: record.lp.clone(),
            authorisation_id: record.authorisation_id,
            authorisation_version: record.authorisation_version,
            quote_id: record.quote_id,
            legs,
            source_currency_id: self.currency_id.clone(),
            source_currency_key: self.currency_key.clone(),
            source_ledger_id: self.ledger_id,
            dest_currency_id: record.dest_currency_id.clone(),
            dest_currency_key: record.dest_currency_key.clone(),
            dest_ledger_id: record.dest_ledger_id,
            amount_in: record.amount_in,
            fee: record.fee,
            amount_out: record.amount_out,
            outcome,
            reversal_of: record.reversal_of,
        })
    }
}

/// The production [`ConversionControl`] binding: the seam read
/// over the gateway's registry records, its currency-key index
/// and its ledger (module docs).
pub struct GatewayConversionControl<'a, B: LedgerBackend> {
    registry: &'a Registry<InMemoryStore>,
    currency_ids: &'a BTreeMap<[u8; 32], String>,
    backend: &'a B,
}

impl<B: LedgerBackend> std::fmt::Debug for GatewayConversionControl<'_, B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayConversionControl")
            .field("currencies", &self.currency_ids.len())
            .finish_non_exhaustive()
    }
}

impl<'a, B: LedgerBackend> GatewayConversionControl<'a, B> {
    fn record(&self, ledger: &str) -> Option<&'a CurrencyRecord> {
        let key = decode_key(ledger)?;
        let currency_id = self.currency_ids.get(&key)?;
        self.registry.resolve(currency_id)
    }

    fn holds_account(&self, record: &CurrencyRecord, owner: &str) -> bool {
        let spec = account_spec(record.ledger_id, AccountKind::Holder, owner, "");
        self.backend.account_balance(spec.id).is_some()
    }
}

impl<B: LedgerBackend> ConversionControl for GatewayConversionControl<'_, B> {
    fn lp_tier(&self, issuer: &IssuerId, source_ledger: &str, dest_ledger: &str) -> Option<LpTier> {
        let source = self.record(source_ledger)?;
        let dest = self.record(dest_ledger)?;
        let owner = hex_encode(issuer);
        if source.issuer_id == owner || dest.issuer_id == owner {
            return Some(LpTier::CurrencyIssuer);
        }
        (self.holds_account(source, &owner) && self.holds_account(dest, &owner))
            .then_some(LpTier::RegisteredLiquidityHolder)
    }

    fn liquidity_binding(&self, issuer: &IssuerId, ledger: &str) -> Option<BindingRecord> {
        let record = self.record(ledger)?;
        let owner = hex_encode(issuer);
        let spec = account_spec(record.ledger_id, AccountKind::Holder, &owner, "");
        let status = match record.status {
            CurrencyStatus::Active => AccountStatus::Open,
            CurrencyStatus::Frozen => AccountStatus::Closing,
            CurrencyStatus::Sunset => AccountStatus::Closed,
        };
        Some(BindingRecord {
            account_id: spec.id,
            ledger_id: record.ledger_id,
            owner_id: owner,
            salt: String::new(),
            kind: AccountKind::Holder,
            status,
        })
    }
}

/// How one submission of a conversion chain came out.
enum ChainOutcome {
    /// Every leg was created now, or every leg was already
    /// present identically: the conversion is posted.
    Posted,
    /// The cluster definitely refused the chain; nothing posted.
    Refused(TransferError),
}

/// The transfer ids of a conversion's legs, chosen once from its
/// idempotency id: the source leg is the idempotency id itself
/// (as for every other value op), and each later leg is a
/// domain-separated hash of it and the leg's position.
fn leg_ids(idempotency: u128, count: usize) -> Vec<u128> {
    (0..count)
        .map(|index| {
            if index == 0 {
                return idempotency;
            }
            let mut hasher = blake3::Hasher::new();
            hasher.update(b"id.gateway.convert-leg/v1");
            hasher.update(&idempotency.to_be_bytes());
            hasher.update(&[u8::try_from(index).unwrap_or(u8::MAX)]);
            let mut bytes = [0_u8; 16];
            bytes.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
            u128::from_be_bytes(bytes)
        })
        .collect()
}

impl<B: LedgerBackend, I: IntentStore> GatewayService<B, I> {
    /// The production conversion control binding over this
    /// service's registry and ledger.
    #[must_use]
    pub fn conversion_control(&self) -> GatewayConversionControl<'_, B> {
        GatewayConversionControl {
            registry: &self.registry,
            currency_ids: &self.currency_ids,
            backend: &self.backend,
        }
    }

    /// The ledger backend, mutably, so a test can stage what a
    /// crash or a second writer leaves behind.
    #[cfg(test)]
    pub(crate) fn backend_mut_for_tests(&mut self) -> &mut B {
        &mut self.backend
    }

    /// The volume already committed against an authorisation
    /// (posted conversions plus held reservations).
    #[must_use]
    pub fn conversion_usage(&self, lp: &IssuerId, authorisation_id: u128) -> u128 {
        self.conversions.usage().used_volume(lp, authorisation_id)
    }

    /// The [`ConversionReceipt`] for a recorded conversion, by
    /// conversion id: the Phase G audit read (module docs).
    /// `Ok(None)` when no intent carries that id, or the intent
    /// is not a conversion.
    ///
    /// # Errors
    ///
    /// An intent-store read failure.
    pub fn conversion_receipt(
        &self,
        conversion_id: u128,
    ) -> Result<Option<ConversionReceipt>, ServiceError> {
        Ok(self
            .intents
            .get(conversion_id)
            .map_err(ServiceError::IntentStore)?
            .and_then(|intent| intent.conversion_receipt()))
    }

    /// Run one signed `convert` request through the pipeline
    /// (module docs): verify, resolve, answer from the intent
    /// store if the request already has a recorded outcome,
    /// authorize, verify the presented authorisation, then
    /// write ahead and submit the linked chain.
    ///
    /// # Errors
    ///
    /// Envelope and F3 refusals, a conversion refusal
    /// ([`ServiceError::Conversion`], no write), a mismatched
    /// pair or quote, the batch builder's refusal, the cluster's
    /// verdict on the chain, a stored verdict on a retry, an
    /// intent-store failure (before any submission), or an
    /// intent conflict on recovery.
    pub fn handle_convert(
        &mut self,
        signed: &Signed,
        body: &[u8],
        now: u64,
        presented: &ConversionPresentation,
    ) -> Result<SignedReceipt, ServiceError> {
        let verified = verify_envelope(&mut self.replays, signed, body, now)?;
        let CurrencyOp::Convert(op) = &verified.request.op else {
            return Err(ServiceError::Unsupported(
                "handle_convert executes only convert requests",
            ));
        };
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
            return self.replay(intent, None);
        }
        self.authorize_convert(&account, &verified.request.op)?;
        let authorized = Authorized {
            account,
            idempotency,
        };
        self.convert(&authorized, op, presented, now)
    }

    /// Issue a quote for a `convert` request (#3802): the signed
    /// request is verified and authorized exactly as for
    /// execution, the presented authorisation is verified at
    /// `now`, and the quote is held for its lifetime under
    /// `quote_id`. Nothing is written to the intent log or the
    /// ledger.
    ///
    /// # Errors
    ///
    /// As [`Self::handle_convert`] up to verification, plus the
    /// verifier's quote refusals (a live quote already holds the
    /// id).
    pub fn quote_convert(
        &mut self,
        signed: &Signed,
        body: &[u8],
        now: u64,
        presented: &ConversionPresentation,
        quote_id: u128,
    ) -> Result<ConversionQuote, ServiceError> {
        let verified = verify_envelope(&mut self.replays, signed, body, now)?;
        let CurrencyOp::Convert(op) = &verified.request.op else {
            return Err(ServiceError::Unsupported(
                "quote_convert quotes only convert requests",
            ));
        };
        let account = self
            .devices
            .resolve_account(&verified.device)
            .ok_or(GatewayError::UnknownDevice)?;
        self.authorize_convert(&account, &verified.request.op)?;
        let source = self.record_for(&op.source)?;
        let dest = self.record_for(&op.dest)?;
        let submission = self.submission(&account, op, presented, &source, &dest, now)?;
        let control = GatewayConversionControl {
            registry: &self.registry,
            currency_ids: &self.currency_ids,
            backend: &self.backend,
        };
        // Quote only what would execute: the folded
        // authorisation must be for exactly this pair.
        let folded = self.conversions.verify(&control, &submission)?;
        check_pair(&folded.authorisation, op)?;
        Ok(self.conversions.issue_quote(
            &control,
            &submission,
            quote_id,
            u32::from(source.scale),
            u32::from(dest.scale),
        )?)
    }

    /// F3 authorization plus the destination's side, which the
    /// F3 table does not check.
    fn authorize_convert(&self, account: &str, op: &CurrencyOp) -> Result<(), ServiceError> {
        let view = RegistryView::new(
            &self.registry,
            &self.currency_ids,
            &self.minters,
            &self.issued,
        );
        authorize(&view, account, op)?;
        if let CurrencyOp::Convert(convert) = op {
            if convert.source == convert.dest {
                return Err(ServiceError::ConversionMismatch(
                    "source and destination are the same currency",
                ));
            }
            let dest = self.record_for(&convert.dest)?;
            if dest.status != CurrencyStatus::Active {
                return Err(GatewayError::InactiveCurrency.into());
            }
        }
        Ok(())
    }

    /// The verifier's submission for this request: the caller,
    /// the LP's bound inventory account on each side, and the
    /// presented objects.
    fn submission(
        &self,
        account: &str,
        op: &Convert,
        presented: &ConversionPresentation,
        source: &CurrencyRecord,
        dest: &CurrencyRecord,
        now: u64,
    ) -> Result<ConversionSubmission, ServiceError> {
        let caller = decode_key(account).ok_or(ServiceError::ConversionMismatch(
            "the calling account is not a key identity",
        ))?;
        let lp = hex_encode(&presented.issuer);
        Ok(ConversionSubmission {
            authorisation_id: presented.authorisation_id,
            issuer: presented.issuer,
            caller,
            authorisations: presented.authorisations.clone(),
            revokes: presented.revokes.clone(),
            source_account: account_spec(source.ledger_id, AccountKind::Holder, &lp, "").id,
            dest_account: account_spec(dest.ledger_id, AccountKind::Holder, &lp, "").id,
            amount: op.amount,
            now,
        })
    }

    fn convert(
        &mut self,
        authorized: &Authorized,
        op: &Convert,
        presented: &ConversionPresentation,
        now: u64,
    ) -> Result<SignedReceipt, ServiceError> {
        let source = self.record_for(&op.source)?;
        let dest = self.record_for(&op.dest)?;
        let submission =
            self.submission(&authorized.account, op, presented, &source, &dest, now)?;
        let (authorisation, quoted) = {
            let control = GatewayConversionControl {
                registry: &self.registry,
                currency_ids: &self.currency_ids,
                backend: &self.backend,
            };
            // A quoted request answers the quote's own refusals
            // first (unknown, expired, mismatched, stale).
            let quoted = match presented.quote_id {
                Some(quote_id) => Some(self.conversions.verify_quoted(
                    &control,
                    &submission,
                    quote_id,
                )?),
                None => None,
            };
            let verified = self.conversions.verify(&control, &submission)?;
            (verified.authorisation, quoted)
        };
        check_pair(&authorisation, op)?;

        // The fee leg exists only when the fee is positive; the
        // fee account is the source issuer's constrained `fee`
        // account.
        let fee_preview = match authorisation.fee_bps {
            Some(fee_bps) => fee_amount(op.amount, fee_bps)
                .map_err(|error| ServiceError::ConversionBatch(format!("{error:?}")))?,
            None => 0,
        };
        let caller_source = self.ensure_account(&source, &op.from, AccountKind::Holder)?;
        let caller_dest = self.ensure_account(&dest, &op.from, AccountKind::Holder)?;
        let lp = hex_encode(&presented.issuer);
        let lp_source = self.ensure_account(&source, &lp, AccountKind::Holder)?;
        let lp_dest = self.ensure_account(&dest, &lp, AccountKind::Holder)?;
        let fee_account = if fee_preview > 0 {
            Some(self.ensure_account(&source, &source.issuer_id, AccountKind::Fee)?)
        } else {
            None
        };
        let leg_count = if fee_preview > 0 { 3 } else { 2 };
        let request = ConversionBatchRequest {
            conversion_id: authorized.idempotency,
            transfer_ids: leg_ids(authorized.idempotency, leg_count),
            caller_source,
            lp_source,
            fee_account,
            lp_dest,
            caller_dest,
            source_ledger: source.ledger_id,
            dest_ledger: dest.ledger_id,
            amount_in: op.amount,
            rate_num: authorisation.rate_num,
            rate_den: authorisation.rate_den,
            source_scale: u32::from(source.scale),
            dest_scale: u32::from(dest.scale),
            fee_bps: authorisation.fee_bps,
            min_amount: authorisation.min_amount,
            max_amount: authorisation.max_amount,
        };
        let (amounts, legs) = build_conversion_batch(&request)
            .map_err(|error| ServiceError::ConversionBatch(format!("{error:?}")))?;
        if let Some(quoted) = &quoted
            && (amounts.amount_out != quoted.dest_amount() || amounts.fee != quoted.fee())
        {
            return Err(ServiceError::ConversionMismatch(
                "the chain's amounts differ from the quote's",
            ));
        }

        let record = ConversionRecord {
            lp,
            authorisation_id: authorisation.authorisation_id,
            authorisation_version: authorisation.version,
            quote_id: presented.quote_id,
            dest_currency_id: dest.currency_id.clone(),
            dest_currency_key: hex_encode(&op.dest),
            dest_ledger_id: dest.ledger_id,
            amount_in: amounts.amount_in,
            fee: amounts.fee,
            amount_out: amounts.amount_out,
            reversal_of: presented.reversal_of,
        };
        let mut intent = GatewayIntent {
            idempotency: authorized.idempotency,
            account: authorized.account.clone(),
            op: CurrencyOp::Convert(op.clone()).name().to_owned(),
            state: IntentState::Recorded,
            currency_id: Some(source.currency_id.clone()),
            currency_key: Some(hex_encode(&op.source)),
            ledger_id: Some(source.ledger_id),
            transfer: None,
            legs,
            conversion: Some(record),
            graph_stamp: None,
            graph_object: None,
            minter: None,
            registration: None,
            receipt: None,
            failure: None,
        };
        // Write-ahead, and the cap reservation with it: the
        // intent, carrying every leg, is durable before the
        // ledger sees anything; if this write fails the request
        // refuses here.
        self.intents
            .put(&intent)
            .map_err(ServiceError::IntentStore)?;
        self.conversions.record_posted(
            &presented.issuer,
            authorisation.authorisation_id,
            amounts.amount_in,
        );
        self.settle_chain(&mut intent)
    }

    /// Submit a recorded conversion's chain and resolve the
    /// intent from the cluster's answer (module docs, step 4).
    fn settle_chain(&mut self, intent: &mut GatewayIntent) -> Result<SignedReceipt, ServiceError> {
        match self.submit_chain(intent)? {
            ChainOutcome::Posted => self.finalize_conversion(intent),
            ChainOutcome::Refused(verdict) => {
                intent.state = IntentState::Failed;
                intent.failure = Some(format!("{verdict:?}"));
                self.intents
                    .put(intent)
                    .map_err(ServiceError::IntentStore)?;
                self.release_reservation(intent);
                Err(ServiceError::Cluster(verdict))
            }
        }
    }

    /// One submission of the stored chain. A lost or malformed
    /// answer, a leg that exists with other contents, or a chain
    /// that is partly present is not a verdict: it is returned as
    /// an error and the intent stays recorded.
    fn submit_chain(&mut self, intent: &GatewayIntent) -> Result<ChainOutcome, ServiceError> {
        let results = self.backend.create_transfers(&intent.legs);
        if results.len() != intent.legs.len() {
            return Err(ServiceError::Backend(format!(
                "ledger backend returned {} verdicts for a {}-leg chain",
                results.len(),
                intent.legs.len()
            )));
        }
        if results.iter().all(Result::is_ok)
            || results
                .iter()
                .all(|result| matches!(result, Err(TransferError::Exists)))
        {
            return Ok(ChainOutcome::Posted);
        }
        if results.iter().any(|result| {
            matches!(
                result,
                Err(TransferError::Exists | TransferError::ExistsWithDifferentFields)
            )
        }) {
            return Err(ServiceError::IntentConflict(format!(
                "conversion {:032x}: part of its chain already exists on the ledger",
                intent.idempotency
            )));
        }
        // A linked chain fails as a whole: the first verdict that
        // is not the chain-failure echo is the cause.
        let cause = results
            .into_iter()
            .filter_map(Result::err)
            .find(|error| !matches!(error, TransferError::LinkedEventFailed))
            .unwrap_or(TransferError::LinkedEventFailed);
        Ok(ChainOutcome::Refused(cause))
    }

    /// Mark a conversion posted and answer with its receipt. The
    /// cap reservation taken at write-ahead stands as the posted
    /// usage.
    fn finalize_conversion(
        &mut self,
        intent: &mut GatewayIntent,
    ) -> Result<SignedReceipt, ServiceError> {
        let receipt = intent.build_receipt();
        intent.state = IntentState::Posted;
        intent.receipt = Some(receipt.clone());
        self.intents
            .put(intent)
            .map_err(ServiceError::IntentStore)?;
        Ok(sign_receipt_with(&self.gateway_key, receipt)?)
    }

    fn release_reservation(&mut self, intent: &GatewayIntent) {
        if let Some(record) = &intent.conversion
            && let Some(lp) = decode_key(&record.lp)
        {
            self.conversions
                .release(&lp, record.authorisation_id, record.amount_in);
        }
    }

    /// Resolve a recorded conversion whose outcome was lost
    /// (module docs, recovery): every leg present and identical
    /// finalizes, none present resubmits the identical chain, and
    /// anything else is a conflict that stays recorded.
    pub(super) fn recover_conversion(
        &mut self,
        mut intent: GatewayIntent,
    ) -> Result<SignedReceipt, ServiceError> {
        if intent.legs.is_empty() {
            return Err(ServiceError::IntentStore(format!(
                "recorded conversion {:032x} carries no legs",
                intent.idempotency
            )));
        }
        let mut present = 0_usize;
        for leg in &intent.legs {
            match self.backend.try_lookup_transfer(leg.id) {
                Ok(Some(found)) if found == *leg => present += 1,
                Ok(Some(_)) => {
                    return Err(ServiceError::IntentConflict(format!(
                        "conversion {:032x}: leg {} exists with other contents",
                        intent.idempotency, leg.id
                    )));
                }
                Ok(None) => {}
                Err(error) => return Err(ServiceError::Backend(error.to_string())),
            }
        }
        if present == intent.legs.len() {
            return self.finalize_conversion(&mut intent);
        }
        if present > 0 {
            return Err(ServiceError::IntentConflict(format!(
                "conversion {:032x}: {present} of {} legs are on the ledger",
                intent.idempotency,
                intent.legs.len()
            )));
        }
        self.settle_chain(&mut intent)
    }
}

/// The folded authorisation must convert exactly the requested
/// pair, in the requested direction.
fn check_pair(
    authorisation: &crate::currency::RateAuthorisation,
    op: &Convert,
) -> Result<(), ServiceError> {
    if authorisation.source_ledger != hex_encode(&op.source)
        || authorisation.dest_ledger != hex_encode(&op.dest)
    {
        return Err(ServiceError::ConversionMismatch(
            "the authorisation is for another currency pair",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;
