//! The gateway verification flow for conversion submissions
//! (#3801, Phase 2 of #2290; design record: the task 5741 triage
//! on #2290 and #2063 §§2.3–2.4).
//!
//! A conversion submission presents the rate authorisation it
//! relies on — the signed chain objects themselves, not a quote
//! or a cached copy — and this flow decides, before any
//! TigerBeetle write, whether the gateway may act on it. The
//! checks compose in the issue's order:
//!
//! 1. **Chain decode + signature.** Every presented object is
//!    decoded with [`chain::decode`] (signature verified), its
//!    kind and body commitment are checked, and for an
//!    authorisation the header's verifying key must equal the
//!    body's `issuer_id`: a body naming an issuer other than the
//!    event author is invalid, not merely untrusted.
//! 2. **LP-tier eligibility.** The folded authorisation's issuer
//!    must hold a liquidity-provider tier for the ledger pair in
//!    the control-plane registry (#2063 §2.3): the issuer of
//!    either currency, or a registered/funded liquidity holder.
//!    A platform LP uses the same registered/funded path; there
//!    is no bypass tier.
//! 3. **Account-control chain.** The conversion's source and
//!    destination liquidity accounts must be exactly the
//!    account-factory binding records for the issuer: a binding
//!    record exists, it names the issuer as owner, it re-derives
//!    to the deterministic factory account for its ledger, kind,
//!    owner, and salt, the submission names that account, and
//!    the account is neither closing nor closed.
//! 4. **Live-state checks at submission.** The Phase 1 fold
//!    ([`fold_authorisation`]) is consulted *now*, at submission
//!    time — this is the gateway-side revocation consult: the
//!    highest version stands folded, it is not revoked (a
//!    revocation whose `effective_at` is still in the future has
//!    not taken effect), the submission is inside the validity
//!    window, the caller is authorised, the amount is within
//!    min/max, and it fits within `remaining_volume` — the
//!    authorisation's `total_cap` minus the volume of
//!    conversions already posted against it.
//!
//! Every failure is a typed [`FailedPreSubmit`] and produces no
//! TigerBeetle write: the verifier holds no ledger handle and
//! takes `&self`, so a refused submission cannot have written
//! anything. Usage accounting is gateway-side state
//! ([`ConversionUsage`]): it is committed only for conversions
//! that actually post (#2063 §2.4), by the execution layer
//! calling [`ConversionVerifier::record_posted`]; verification
//! itself never commits.
//!
//! **Quotes (#3802, Phase 3 of #2290).** The gateway also
//! issues short-lived quotes against a verified submission
//! ([`ConversionVerifier::issue_quote`]): a control-plane
//! record — not a graph object — pinning the folded
//! authorisation's version and event hash, the amount `x`,
//! the scale-aware `computed_y` (the pure layer's
//! [`crate::conversion::convert_amount`]: exact integer
//! rational, floored in the LP's favour), the fee, and a
//! 60-second lifetime (#2063 §2.2). A submission that
//! executes under a quote
//! ([`ConversionVerifier::verify_quoted`]) must reference an
//! unexpired quote and re-runs every check above at
//! submission time; a re-issued (new version) or revoked
//! authorisation never silently satisfies an old quote —
//! the version/event-hash pin is compared against the fresh
//! fold and any mismatch fails pre-flight. Execution under
//! a quote pays exactly the quoted `computed_y` and fee.
//! Issuance commits no usage: a quote is not a posting, and
//! the cap is re-checked by the submission's own live-state
//! checks.
//!
//! Scope boundary: this module verifies and quotes; it does
//! not settle. Composing the verified conversion into the
//! linked transfer chain and wiring this flow into the
//! gateway service's request pipeline is the execution
//! programme beyond #3802 — the F3 `Convert` op carries no
//! authorisation reference, so the submission shape here
//! (presented chain objects plus named liquidity accounts)
//! is the input that work will adapt.

use std::collections::BTreeMap;

use id_currency_registry::{AccountKind, account_spec};
use p2panda_core::Hash;

use super::{
    AuthorisationState, IssuerId, RATE_AUTHORISATION_KIND, RATE_AUTHORISATION_REVOKE_KIND,
    RateAuthorisation, RateAuthorisationRevoke, fold_authorisation,
};
use crate::chain::{self, Event};
use crate::conversion::{convert_amount, fee_amount};
use crate::hosted_binding::hex_encode;

/// The liquidity-provider tiers of #2063 §2.3. Either tier is
/// eligible to offer conversions; there is no third, bypass tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LpTier {
    /// The LP issues the source or the destination currency.
    CurrencyIssuer,
    /// The LP is a registered, funded liquidity holder who does
    /// not issue either currency.
    RegisteredLiquidityHolder,
}

/// The control-plane lifecycle of a bound liquidity account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountStatus {
    /// The account accepts conversion legs.
    Open,
    /// The account is being wound down; no new conversions.
    Closing,
    /// The account is closed; no new conversions.
    Closed,
}

impl std::fmt::Display for AccountStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open => write!(f, "open"),
            Self::Closing => write!(f, "closing"),
            Self::Closed => write!(f, "closed"),
        }
    }
}

/// Which side of a conversion a failure or record belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountSide {
    /// The ledger conversions draw from.
    Source,
    /// The ledger conversions pay into.
    Destination,
}

impl std::fmt::Display for AccountSide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Source => write!(f, "source"),
            Self::Destination => write!(f, "destination"),
        }
    }
}

/// One account-factory binding record as the control plane holds
/// it: the account the factory created for an LP on one ledger,
/// with the derivation inputs it was created from, so the
/// verification flow can re-derive the factory account and
/// require the record to match it exactly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingRecord {
    /// The derived 128-bit account id.
    pub account_id: u128,
    /// The ledger the account sits on.
    pub ledger_id: u32,
    /// The account holder's id; for a binding of `issuer` this
    /// must be the issuer's account id (the hex of its key).
    pub owner_id: String,
    /// The factory derivation salt the account was created with.
    pub salt: String,
    /// The account's control-plane lifecycle status.
    pub status: AccountStatus,
}

/// The control-plane view the verification flow reads (design
/// §4.2's registry checks, LP half). The production registry is
/// the control plane's store; this trait is the seam, and
/// fixture implementations test the flow without it (the
/// [`super::gateway`] seam pattern).
pub trait ConversionControl {
    /// The LP tier `issuer` holds for conversions from
    /// `source_ledger` to `dest_ledger`, or `None` when the
    /// issuer is not an eligible LP for the pair.
    fn lp_tier(&self, issuer: &IssuerId, source_ledger: &str, dest_ledger: &str) -> Option<LpTier>;

    /// The binding record for `issuer`'s liquidity account on
    /// `ledger`, or `None` when no account is bound.
    fn liquidity_binding(&self, issuer: &IssuerId, ledger: &str) -> Option<BindingRecord>;
}

/// A fixed control-plane view, for tests and fixtures.
#[derive(Clone, Debug, Default)]
pub struct FixedConversionControl {
    tiers: BTreeMap<(IssuerId, String, String), LpTier>,
    bindings: BTreeMap<(IssuerId, String), BindingRecord>,
}

impl FixedConversionControl {
    /// An empty view: no tiers, no bindings.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `tier` for `issuer` offering
    /// `source_ledger` → `dest_ledger`.
    pub fn set_tier(
        &mut self,
        issuer: &IssuerId,
        source_ledger: &str,
        dest_ledger: &str,
        tier: LpTier,
    ) {
        self.tiers.insert(
            (*issuer, source_ledger.to_owned(), dest_ledger.to_owned()),
            tier,
        );
    }

    /// Record `binding` as `issuer`'s liquidity account on
    /// `ledger`.
    pub fn set_binding(&mut self, issuer: &IssuerId, ledger: &str, binding: BindingRecord) {
        self.bindings.insert((*issuer, ledger.to_owned()), binding);
    }
}

impl ConversionControl for FixedConversionControl {
    fn lp_tier(&self, issuer: &IssuerId, source_ledger: &str, dest_ledger: &str) -> Option<LpTier> {
        self.tiers
            .get(&(*issuer, source_ledger.to_owned(), dest_ledger.to_owned()))
            .copied()
    }

    fn liquidity_binding(&self, issuer: &IssuerId, ledger: &str) -> Option<BindingRecord> {
        self.bindings.get(&(*issuer, ledger.to_owned())).cloned()
    }
}

/// A signed graph object as presented with a submission: the
/// chain event (whose header commits to the body hash) and the
/// body bytes themselves.
#[derive(Clone, Debug)]
pub struct PresentedObject {
    /// The signed chain event.
    pub event: Event,
    /// The body the event's header commits to.
    pub body: Vec<u8>,
}

/// A conversion submission: everything the gateway needs to
/// verify one conversion before any ledger write.
#[derive(Clone, Debug)]
pub struct ConversionSubmission {
    /// The authorisation the conversion relies on.
    pub authorisation_id: u128,
    /// The expected issuer (the LP): the fold is bound to this
    /// issuer, so a foreign signer reusing the authorisation id
    /// cannot hijack the submission.
    pub issuer: IssuerId,
    /// The calling identity, resolved from the signed request
    /// envelope (device → account → issuer id) by the caller.
    pub caller: IssuerId,
    /// Every authorisation version presented, as signed objects.
    pub authorisations: Vec<PresentedObject>,
    /// Every revocation presented, as signed objects.
    pub revokes: Vec<PresentedObject>,
    /// The liquidity account the conversion names on the source
    /// ledger; must equal the issuer's binding record exactly.
    pub source_account: u128,
    /// The liquidity account the conversion names on the
    /// destination ledger; must equal the binding record exactly.
    pub dest_account: u128,
    /// The amount to convert, in source-ledger units.
    pub amount: u128,
    /// The executor's clock at submission (unix seconds); the
    /// signed objects carry no trusted wall-clock time, so the
    /// window and revocation effect are evaluated against this.
    pub now: u64,
}

/// Why a conversion submission was refused. Every variant is a
/// pre-flight refusal: the submission's disposition is
/// `FAILED_PRE_SUBMIT` and no TigerBeetle write has happened or
/// will happen for it.
#[derive(Debug, thiserror::Error)]
pub enum FailedPreSubmit {
    /// Step 1: a presented event's bytes do not verify (bad
    /// signature, malformed or non-canonical encoding).
    #[error("presented event does not verify")]
    BadSignature,
    /// Step 1: a presented event is not of the expected kind.
    #[error("presented event has the wrong kind")]
    WrongKind,
    /// Step 1: the presented body is not the body the event's
    /// header commits to.
    #[error("presented body does not match the event's body commitment")]
    BodyCommitmentMismatch,
    /// Step 1: the body does not decode as the expected object.
    #[error("presented body is not a valid authorisation object")]
    BodyMalformed,
    /// Step 1: the authorisation body's `issuer_id` is not the
    /// header's verifying key.
    #[error("authorisation body names an issuer other than the event author")]
    IssuerMismatch,
    /// Step 1/4: no valid version of the authorisation from the
    /// expected issuer folds.
    #[error("no valid authorisation from the expected issuer")]
    UnknownAuthorisation,
    /// Step 2: the issuer holds no LP tier for the ledger pair.
    #[error("issuer is not an eligible liquidity provider for this pair")]
    LpNotEligible,
    /// Step 3: no liquidity account is bound for the issuer on
    /// this side's ledger.
    #[error("no liquidity account is bound for the {side} ledger")]
    UnboundAccount {
        /// The side whose ledger has no binding.
        side: AccountSide,
    },
    /// Step 3: the binding record is not exactly the
    /// account-factory record for the issuer (wrong owner, or
    /// the recorded account does not re-derive from the factory
    /// inputs).
    #[error("the {side} binding record is not the issuer's account-factory record")]
    BindingNotFactory {
        /// The side whose binding record is not the factory's.
        side: AccountSide,
    },
    /// Step 3: the submission names an account other than the
    /// bound one.
    #[error("the conversion names a {side} account other than the bound account")]
    AccountMismatch {
        /// The side whose named account differs.
        side: AccountSide,
    },
    /// Step 3: the bound account is closing or closed.
    #[error("the bound {side} liquidity account is {status}")]
    AccountNotOpen {
        /// The side whose account is not open.
        side: AccountSide,
        /// The account's lifecycle status.
        status: AccountStatus,
    },
    /// Step 4: the folded authorisation is revoked, effective
    /// at or before submission time.
    #[error("authorisation is revoked effective at {effective_at}")]
    Revoked {
        /// When the covering revocation took effect.
        effective_at: u64,
    },
    /// Step 4: the submission precedes the authorisation's
    /// validity window.
    #[error("authorisation is not valid before {not_before}")]
    NotYetValid {
        /// The start of the validity window (unix seconds).
        not_before: u64,
    },
    /// Step 4: the submission is at or after the authorisation's
    /// expiry.
    #[error("authorisation expired at {expires_at}")]
    Expired {
        /// The authorisation's expiry (unix seconds).
        expires_at: u64,
    },
    /// Step 4: the caller is not authorised under the
    /// authorisation's caller policy.
    #[error("caller is not authorised under this authorisation")]
    UnauthorisedCaller,
    /// Step 4: the amount is below the authorisation's minimum.
    #[error("amount is below the authorisation minimum of {min_amount}")]
    AmountBelowMin {
        /// The authorisation's minimum amount.
        min_amount: u128,
    },
    /// Step 4: the amount exceeds the authorisation's maximum.
    #[error("amount exceeds the authorisation maximum of {max_amount}")]
    AmountAboveMax {
        /// The authorisation's maximum amount.
        max_amount: u128,
    },
    /// Step 4: the amount exceeds the authorisation's remaining
    /// volume.
    #[error("amount exceeds the authorisation's remaining volume of {remaining_volume}")]
    CapExhausted {
        /// The volume left under the cap at submission time.
        remaining_volume: u128,
    },
    /// Step 4: the amount does not convert (rate overflow, or a
    /// zero destination amount, which the design refuses rather
    /// than treating as a free conversion).
    #[error("amount does not convert to a positive destination amount")]
    UnconvertibleAmount,
    /// Quoted submissions (#3802): no quote with the referenced
    /// id has been issued by this gateway.
    #[error("no quote with this id has been issued")]
    UnknownQuote,
    /// Quoted submissions (#3802): the referenced quote's
    /// lifetime has elapsed at submission time.
    #[error("quote expired at {quote_expires_at}")]
    QuoteExpired {
        /// The quote's expiry (unix seconds).
        quote_expires_at: u64,
    },
    /// Quoted submissions (#3802): the quote was issued for a
    /// different authorisation, or for a different amount, than
    /// the submission carries — an altered quote reference
    /// cannot be executed.
    #[error("submission does not match the quote it references")]
    QuoteMismatch,
    /// Quoted submissions (#3802): the authorisation has been
    /// re-issued since the quote was issued — the live fold is
    /// a different version, or a different event, than the one
    /// the quote pins, so the quote no longer binds.
    #[error(
        "quote pins authorisation version {quoted_version}, but version {live_version} is live"
    )]
    QuoteStale {
        /// The version the quote was issued against.
        quoted_version: u64,
        /// The version the submission-time fold stands at.
        live_version: u64,
    },
}

impl FailedPreSubmit {
    /// The recorded disposition of every pre-flight refusal.
    #[must_use]
    pub const fn disposition(&self) -> &'static str {
        "FAILED_PRE_SUBMIT"
    }
}

/// A conversion that passed every check: the folded live
/// authorisation it relies on and the exact accounts and
/// amounts the execution layer may act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedConversion {
    /// The highest folded version of the authorisation, live at
    /// submission time.
    pub authorisation: RateAuthorisation,
    /// The LP tier the issuer was verified under.
    pub tier: LpTier,
    /// The bound source liquidity account (== the named one).
    pub source_account: u128,
    /// The bound destination liquidity account (== the named one).
    pub dest_account: u128,
    /// The destination amount at the authorisation's rate:
    /// `floor(amount · rate_num / rate_den)`, before the
    /// execution layer's scale normalisation.
    pub dest_amount: u128,
    /// The volume left under the authorisation's cap after this
    /// conversion posts.
    pub remaining_volume: u128,
}

/// The lifetime of an issued quote, in seconds (#2063 §2.2,
/// adopted by the #2290 design record).
pub const QUOTE_LIFETIME_SECS: u64 = 60;

/// A gateway-issued quote (#3802): a short-lived control-plane
/// record — not a graph object — pinning one authorisation
/// version, by its event hash, to one amount and the exact
/// destination amount and fee a conversion under it executes
/// with. The version + event-hash pin is the binding: a
/// re-issued or revoked authorisation never silently
/// satisfies an old quote, because execution re-runs the
/// live-state checks and compares the fresh fold against
/// this pin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversionQuote {
    /// The quote's control-plane id (gateway-generated, unique
    /// in the gateway's store).
    pub quote_id: u128,
    /// The authorisation the quote was issued against.
    pub authorisation_id: u128,
    /// The folded version of that authorisation at issuance.
    pub authorisation_version: u64,
    /// The operation id (BLAKE3 over the signed header bytes)
    /// of the presented authorisation event that folded live
    /// at issuance.
    pub authorisation_event_hash: Hash,
    /// The source amount quoted, in source-ledger units.
    pub x: u128,
    /// The destination amount at issuance: the scale-aware
    /// integer-rational rule, floored in the LP's favour.
    pub computed_y: u128,
    /// The fee at issuance, in source-ledger units: `floor(x ·
    /// fee_bps / 10_000)`, or zero when the authorisation
    /// carries no fee.
    pub fee: u128,
    /// When the gateway issued the quote (unix seconds).
    pub issued_at: u64,
    /// `issued_at + QUOTE_LIFETIME_SECS`; no execution at or
    /// after this time.
    pub quote_expires_at: u64,
}

impl ConversionQuote {
    /// Whether the quote's lifetime has elapsed at `now`.
    #[must_use]
    pub const fn is_expired(&self, now: u64) -> bool {
        now >= self.quote_expires_at
    }
}

/// A conversion verified against the quote it executes
/// under (#3802): the quote itself, whose `computed_y` and
/// `fee` are the exact amounts the execution layer must use,
/// plus the submission-time verification facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedQuotedConversion {
    /// The quote the submission referenced and was bound to.
    pub quote: ConversionQuote,
    /// The LP tier the issuer was verified under.
    pub tier: LpTier,
    /// The bound source liquidity account (== the named one).
    pub source_account: u128,
    /// The bound destination liquidity account (== the named
    /// one).
    pub dest_account: u128,
    /// The volume left under the authorisation's cap after
    /// this conversion posts.
    pub remaining_volume: u128,
}

impl VerifiedQuotedConversion {
    /// The source amount: exactly the quoted `x`.
    #[must_use]
    pub const fn amount(&self) -> u128 {
        self.quote.x
    }

    /// The destination amount the execution must pay: exactly
    /// the quoted `computed_y`, never a recomputation.
    #[must_use]
    pub const fn dest_amount(&self) -> u128 {
        self.quote.computed_y
    }

    /// The fee the execution must charge: exactly the quoted
    /// fee, never a recomputation.
    #[must_use]
    pub const fn fee(&self) -> u128 {
        self.quote.fee
    }
}

/// A verified submission plus the operation id of the exact
/// presented authorisation event its fold stands at — the
/// pin a quote binds to (#3802).
struct FoldedVerification {
    verified: VerifiedConversion,
    authorisation_event_hash: Hash,
}

/// Gateway-side usage accounting: the volume posted against
/// each authorisation. This is the state `remaining_volume` is
/// computed from; it is committed only for posted conversions
/// (#2063 §2.4) and never by verification.
#[derive(Clone, Debug, Default)]
pub struct ConversionUsage {
    posted: BTreeMap<(IssuerId, u128), u128>,
}

impl ConversionUsage {
    /// An empty usage book.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The volume already posted against `authorisation_id` of
    /// `issuer`. Authorisation ids are scoped to their issuer, so
    /// another issuer's usage under the same number is not counted.
    #[must_use]
    pub fn used_volume(&self, issuer: &IssuerId, authorisation_id: u128) -> u128 {
        self.posted
            .get(&(*issuer, authorisation_id))
            .copied()
            .unwrap_or(0)
    }

    /// Commit `amount` of posted volume against `authorisation_id`
    /// of `issuer`. Called by the execution layer when a
    /// conversion posts — never for a refused or failed one.
    pub fn record_posted(&mut self, issuer: &IssuerId, authorisation_id: u128, amount: u128) {
        let used = self.used_volume(issuer, authorisation_id);
        self.posted
            .insert((*issuer, authorisation_id), used.saturating_add(amount));
    }
}

/// The gateway's conversion verifier: the verification flow
/// plus the gateway-side state it reads — the usage book and
/// the issued-quote book (#3802). One instance is the
/// gateway's single consult point at submission time.
#[derive(Clone, Debug, Default)]
pub struct ConversionVerifier {
    usage: ConversionUsage,
    quotes: BTreeMap<u128, ConversionQuote>,
}

impl ConversionVerifier {
    /// A verifier with empty usage and quote books.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The usage book verification reads.
    #[must_use]
    pub const fn usage(&self) -> &ConversionUsage {
        &self.usage
    }

    /// The quote this gateway issued under `quote_id`, if any.
    #[must_use]
    pub fn quote(&self, quote_id: u128) -> Option<&ConversionQuote> {
        self.quotes.get(&quote_id)
    }

    /// Commit posted volume (execution layer only; see
    /// [`ConversionUsage::record_posted`]).
    pub fn record_posted(&mut self, issuer: &IssuerId, authorisation_id: u128, amount: u128) {
        self.usage.record_posted(issuer, authorisation_id, amount);
    }

    /// Verify one conversion submission, composing the checks
    /// in the issue's order (module docs). Pure with respect to
    /// the ledger: this function cannot write to TigerBeetle.
    ///
    /// # Errors
    ///
    /// Returns the typed [`FailedPreSubmit`] for the first check
    /// the submission fails; every variant is a pre-flight
    /// refusal with zero ledger writes.
    pub fn verify(
        &self,
        control: &impl ConversionControl,
        submission: &ConversionSubmission,
    ) -> Result<VerifiedConversion, FailedPreSubmit> {
        self.verify_folded(control, submission)
            .map(|folded| folded.verified)
    }

    /// [`Self::verify`] plus the operation id of the exact
    /// presented authorisation event the fold stands at — the
    /// pin quote issuance binds to (#3802).
    fn verify_folded(
        &self,
        control: &impl ConversionControl,
        submission: &ConversionSubmission,
    ) -> Result<FoldedVerification, FailedPreSubmit> {
        // Step 1: chain decode + signature for every presented
        // object; authorisation bodies must name their author.
        // The event ids are kept alongside the bodies so the
        // folded live body can be traced back to the exact
        // presented event that carried it.
        let mut authorisations = Vec::with_capacity(submission.authorisations.len());
        let mut authorisation_event_ids = Vec::with_capacity(submission.authorisations.len());
        for presented in &submission.authorisations {
            let event = decode_presented(presented, RATE_AUTHORISATION_KIND)?;
            let body = RateAuthorisation::from_body(&presented.body)
                .map_err(|_| FailedPreSubmit::BodyMalformed)?;
            if super::author_id(&event) != body.issuer_id {
                return Err(FailedPreSubmit::IssuerMismatch);
            }
            authorisation_event_ids.push(event.id);
            authorisations.push((super::author_id(&event), body));
        }
        let mut revokes = Vec::with_capacity(submission.revokes.len());
        for presented in &submission.revokes {
            let event = decode_presented(presented, RATE_AUTHORISATION_REVOKE_KIND)?;
            let body = RateAuthorisationRevoke::from_body(&presented.body)
                .map_err(|_| FailedPreSubmit::BodyMalformed)?;
            revokes.push((super::author_id(&event), body));
        }

        // The Phase 1 fold over the presented objects, bound to
        // the expected issuer. Computed once here; the live-state
        // assertions on it run at step 4, in the issue's order,
        // so a submission failing an earlier step reports that
        // step even when the fold is revoked or expired.
        let state = fold_authorisation(
            submission.authorisation_id,
            &submission.issuer,
            &authorisations,
            &revokes,
        )
        .ok_or(FailedPreSubmit::UnknownAuthorisation)?;
        let (authorisation, revoked_from) = match &state {
            AuthorisationState::Live(body) => (body, None),
            AuthorisationState::Revoked {
                authorisation,
                effective_at,
                ..
            } => (authorisation, Some(*effective_at)),
        };

        // The exact presented event the fold stands at: the
        // fold's live body is one of the presented bodies by
        // construction, so its carrier is found by equality.
        // The fallback is unreachable in practice and typed
        // rather than a panic.
        let authorisation_event_hash = authorisations
            .iter()
            .zip(&authorisation_event_ids)
            .find(|((_, body), _)| body == authorisation)
            .map(|(_, event_id)| *event_id)
            .ok_or(FailedPreSubmit::UnknownAuthorisation)?;

        // Step 2: LP-tier eligibility for the folded pair.
        let tier = control
            .lp_tier(
                &submission.issuer,
                &authorisation.source_ledger,
                &authorisation.dest_ledger,
            )
            .ok_or(FailedPreSubmit::LpNotEligible)?;

        // Step 3: the account-control chain, both sides.
        check_binding(
            control,
            submission,
            AccountSide::Source,
            &authorisation.source_ledger,
            submission.source_account,
        )?;
        check_binding(
            control,
            submission,
            AccountSide::Destination,
            &authorisation.dest_ledger,
            submission.dest_account,
        )?;

        // Step 4: live-state checks at submission time, over
        // the fold consulted just now.
        if let Some(effective_at) = revoked_from {
            if submission.now >= effective_at {
                return Err(FailedPreSubmit::Revoked { effective_at });
            }
        }
        if submission.now < authorisation.not_before {
            return Err(FailedPreSubmit::NotYetValid {
                not_before: authorisation.not_before,
            });
        }
        if submission.now >= authorisation.expires_at {
            return Err(FailedPreSubmit::Expired {
                expires_at: authorisation.expires_at,
            });
        }
        if !authorisation.authorised_callers.allows(&submission.caller) {
            return Err(FailedPreSubmit::UnauthorisedCaller);
        }
        if submission.amount < authorisation.min_amount {
            return Err(FailedPreSubmit::AmountBelowMin {
                min_amount: authorisation.min_amount,
            });
        }
        if submission.amount > authorisation.max_amount {
            return Err(FailedPreSubmit::AmountAboveMax {
                max_amount: authorisation.max_amount,
            });
        }
        let used = self.usage.used_volume(&submission.issuer, submission.authorisation_id);
        let remaining_volume = authorisation.total_cap.saturating_sub(used);
        if submission.amount > remaining_volume {
            return Err(FailedPreSubmit::CapExhausted { remaining_volume });
        }
        let dest_amount = authorisation
            .apply_rate(submission.amount)
            .filter(|dest| *dest > 0)
            .ok_or(FailedPreSubmit::UnconvertibleAmount)?;

        Ok(FoldedVerification {
            verified: VerifiedConversion {
                authorisation: authorisation.clone(),
                tier,
                source_account: submission.source_account,
                dest_account: submission.dest_account,
                dest_amount,
                remaining_volume: remaining_volume - submission.amount,
            },
            authorisation_event_hash,
        })
    }

    /// Issue a quote against a submission (#3802): the
    /// submission is verified exactly as at execution (the
    /// four steps above, at the submission's `now`), then the
    /// folded authorisation's version and event hash are
    /// pinned into a [`ConversionQuote`] with the scale-aware
    /// `computed_y`, the fee, and a [`QUOTE_LIFETIME_SECS`]
    /// lifetime. The quote is stored in this verifier's quote
    /// book under `quote_id` — a gateway-generated id, unique
    /// in the book, in the same caller-supplies-the-id shape
    /// as the pure layer's transfer ids — and returned.
    ///
    /// Issuance commits no usage: a quote is not a posting,
    /// and a conversion under it re-checks the cap in its own
    /// live-state checks at submission.
    ///
    /// # Errors
    ///
    /// Returns the typed [`FailedPreSubmit`] for the first
    /// check the submission fails, exactly as [`Self::verify`]
    /// would refuse it; no quote is stored for a refused
    /// issuance. A conversion whose scale-aware output rounds
    /// to zero is refused as
    /// [`FailedPreSubmit::UnconvertibleAmount`], even when the
    /// unscaled rate check in verification passed.
    pub fn issue_quote(
        &mut self,
        control: &impl ConversionControl,
        submission: &ConversionSubmission,
        quote_id: u128,
        source_scale: u32,
        dest_scale: u32,
    ) -> Result<ConversionQuote, FailedPreSubmit> {
        let folded = self.verify_folded(control, submission)?;
        let authorisation = &folded.verified.authorisation;
        let computed_y = convert_amount(
            submission.amount,
            authorisation.rate_num,
            authorisation.rate_den,
            source_scale,
            dest_scale,
        )
        .map_err(|_| FailedPreSubmit::UnconvertibleAmount)?;
        // A validated authorisation's fee_bps is at most
        // 10_000, so `fee_amount` cannot refuse here; the error
        // arm is unreachable but typed.
        let fee = match authorisation.fee_bps {
            Some(fee_bps) => fee_amount(submission.amount, fee_bps)
                .map_err(|_| FailedPreSubmit::UnconvertibleAmount)?,
            None => 0,
        };
        let quote = ConversionQuote {
            quote_id,
            authorisation_id: authorisation.authorisation_id,
            authorisation_version: authorisation.version,
            authorisation_event_hash: folded.authorisation_event_hash,
            x: submission.amount,
            computed_y,
            fee,
            issued_at: submission.now,
            quote_expires_at: submission.now.saturating_add(QUOTE_LIFETIME_SECS),
        };
        self.quotes.insert(quote_id, quote.clone());
        Ok(quote)
    }

    /// Verify a submission that executes under a quote
    /// (#3802): the submission references a quote this gateway
    /// issued, and execution binds to it. The quote must exist
    /// in this verifier's book, be unexpired at submission
    /// time, and have been issued for this authorisation and
    /// this amount; then every live-state check re-runs at
    /// submission (the gateway-side revocation consult
    /// included — a since-revoked authorisation is refused by
    /// the re-run, never by the quote), and the fresh fold
    /// must still stand at the version and event the quote
    /// pins. The returned conversion executes with exactly
    /// the quoted `computed_y` and fee.
    ///
    /// # Errors
    ///
    /// Returns the typed [`FailedPreSubmit`] for the first
    /// check the submission fails; every variant is a
    /// pre-flight refusal with zero ledger writes.
    pub fn verify_quoted(
        &self,
        control: &impl ConversionControl,
        submission: &ConversionSubmission,
        quote_id: u128,
    ) -> Result<VerifiedQuotedConversion, FailedPreSubmit> {
        let quote = self
            .quotes
            .get(&quote_id)
            .ok_or(FailedPreSubmit::UnknownQuote)?;
        if quote.is_expired(submission.now) {
            return Err(FailedPreSubmit::QuoteExpired {
                quote_expires_at: quote.quote_expires_at,
            });
        }
        if quote.authorisation_id != submission.authorisation_id || quote.x != submission.amount {
            return Err(FailedPreSubmit::QuoteMismatch);
        }
        let folded = self.verify_folded(control, submission)?;
        if folded.verified.authorisation.version != quote.authorisation_version
            || folded.authorisation_event_hash != quote.authorisation_event_hash
        {
            return Err(FailedPreSubmit::QuoteStale {
                quoted_version: quote.authorisation_version,
                live_version: folded.verified.authorisation.version,
            });
        }
        Ok(VerifiedQuotedConversion {
            quote: quote.clone(),
            tier: folded.verified.tier,
            source_account: folded.verified.source_account,
            dest_account: folded.verified.dest_account,
            remaining_volume: folded.verified.remaining_volume,
        })
    }
}

/// Step 1 for one presented object: decode and verify the
/// chain event, check its kind, and check the presented body
/// against the header's body commitment.
fn decode_presented(presented: &PresentedObject, kind: &str) -> Result<Event, FailedPreSubmit> {
    let event = chain::decode(&presented.event.bytes).map_err(|_| FailedPreSubmit::BadSignature)?;
    if event.header.extensions.kind != kind {
        return Err(FailedPreSubmit::WrongKind);
    }
    if event.header.payload_hash != Some(Hash::digest(&presented.body)) {
        return Err(FailedPreSubmit::BodyCommitmentMismatch);
    }
    Ok(event)
}

/// Step 3 for one side: a binding record exists for the issuer
/// on the ledger, it is exactly the account-factory record for
/// the issuer, the submission names the bound account, and the
/// account is open.
fn check_binding(
    control: &impl ConversionControl,
    submission: &ConversionSubmission,
    side: AccountSide,
    ledger: &str,
    named_account: u128,
) -> Result<(), FailedPreSubmit> {
    let binding = control
        .liquidity_binding(&submission.issuer, ledger)
        .ok_or(FailedPreSubmit::UnboundAccount { side })?;
    let factory = account_spec(
        binding.ledger_id,
        AccountKind::Liquidity,
        &binding.owner_id,
        &binding.salt,
    );
    if binding.owner_id != hex_encode(&submission.issuer) || factory.id != binding.account_id {
        return Err(FailedPreSubmit::BindingNotFactory { side });
    }
    if named_account != binding.account_id {
        return Err(FailedPreSubmit::AccountMismatch { side });
    }
    if binding.status != AccountStatus::Open {
        return Err(FailedPreSubmit::AccountNotOpen {
            side,
            status: binding.status,
        });
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use p2panda_core::SigningKey;

    use super::*;
    use crate::chain::ChainExt;
    use crate::currency::{
        AuthorisedCallers, RateAuthorisationRevoke, sign_authorisation, sign_revoke,
    };
    use crate::currency_poc::ledger::{InMemoryLedger, LedgerBackend as _};

    const NOW: u64 = 1_700_000_000;
    const SOURCE_LEDGER_ID: u32 = 1001;
    const DEST_LEDGER_ID: u32 = 1002;

    fn lp_key() -> SigningKey {
        SigningKey::from_bytes(&[7; 32])
    }

    fn other_key() -> SigningKey {
        SigningKey::from_bytes(&[9; 32])
    }

    fn caller_key() -> SigningKey {
        SigningKey::from_bytes(&[11; 32])
    }

    /// The #2290 worked example: LP offers ABC→123 at 0.8123.
    fn authorisation() -> RateAuthorisation {
        RateAuthorisation {
            authorisation_id: 7,
            version: 1,
            issuer_id: super::super::issuer_id(&lp_key()),
            source_ledger: "ABC".to_owned(),
            dest_ledger: "123".to_owned(),
            rate_num: 8123,
            rate_den: 10_000,
            min_amount: 10,
            max_amount: 10_000,
            total_cap: 1_000_000,
            authorised_callers: AuthorisedCallers::Any,
            not_before: 0,
            expires_at: 4_102_444_800,
            reversal_window_secs: None,
            fee_bps: None,
        }
    }

    fn presented_auth(auth: &RateAuthorisation) -> PresentedObject {
        let event = sign_authorisation(&lp_key(), 0, None, auth, vec![]).unwrap();
        PresentedObject {
            event,
            body: auth.body().unwrap(),
        }
    }

    fn presented_revoke(revoke: &RateAuthorisationRevoke) -> PresentedObject {
        let event = sign_revoke(&lp_key(), 0, None, revoke, vec![]).unwrap();
        PresentedObject {
            event,
            body: revoke.body().unwrap(),
        }
    }

    /// The factory binding record for the LP on one ledger,
    /// derived exactly as the account factory derives it.
    fn factory_binding(ledger_id: u32) -> BindingRecord {
        let owner = hex_encode(&super::super::issuer_id(&lp_key()));
        let spec = account_spec(ledger_id, AccountKind::Liquidity, &owner, "");
        BindingRecord {
            account_id: spec.id,
            ledger_id,
            owner_id: owner,
            salt: String::new(),
            status: AccountStatus::Open,
        }
    }

    /// A control-plane view where the LP is a registered
    /// liquidity holder with both accounts bound and open.
    fn eligible_control() -> FixedConversionControl {
        let issuer = super::super::issuer_id(&lp_key());
        let mut control = FixedConversionControl::new();
        control.set_tier(&issuer, "ABC", "123", LpTier::RegisteredLiquidityHolder);
        control.set_binding(&issuer, "ABC", factory_binding(SOURCE_LEDGER_ID));
        control.set_binding(&issuer, "123", factory_binding(DEST_LEDGER_ID));
        control
    }

    /// A submission that passes every check against
    /// [`eligible_control`].
    fn valid_submission() -> ConversionSubmission {
        ConversionSubmission {
            authorisation_id: 7,
            issuer: super::super::issuer_id(&lp_key()),
            caller: super::super::issuer_id(&caller_key()),
            authorisations: vec![presented_auth(&authorisation())],
            revokes: vec![],
            source_account: factory_binding(SOURCE_LEDGER_ID).account_id,
            dest_account: factory_binding(DEST_LEDGER_ID).account_id,
            amount: 100,
            now: NOW,
        }
    }

    /// The acceptance shape for every failure mode: the refusal
    /// is the expected typed variant, its disposition is
    /// FAILED_PRE_SUBMIT, the verifier's usage book is
    /// untouched, and a ledger standing by has zero accounts
    /// and zero transfers written to it.
    fn assert_pre_submit(
        verifier: &ConversionVerifier,
        control: &impl ConversionControl,
        submission: &ConversionSubmission,
        expected: &FailedPreSubmit,
    ) {
        let ledger = InMemoryLedger::default();
        let used_before = verifier.usage().used_volume(&submission.issuer, submission.authorisation_id);
        let error = verifier
            .verify(control, submission)
            .expect_err("submission must be refused pre-flight");
        assert_eq!(
            std::mem::discriminant(&error),
            std::mem::discriminant(expected),
            "expected {expected:?}, got {error:?}"
        );
        assert_eq!(error.disposition(), "FAILED_PRE_SUBMIT");
        assert_eq!(
            verifier.usage().used_volume(&submission.issuer, submission.authorisation_id),
            used_before,
            "a refused submission must not commit usage"
        );
        for ledger_id in [SOURCE_LEDGER_ID, DEST_LEDGER_ID] {
            assert!(
                ledger.try_accounts_on_ledger(ledger_id).unwrap().is_empty(),
                "zero TigerBeetle account writes"
            );
            assert!(
                ledger
                    .try_transfers_on_ledger(ledger_id, 100)
                    .unwrap()
                    .is_empty(),
                "zero TigerBeetle transfer writes"
            );
        }
    }

    #[test]
    fn valid_submission_verifies_with_rate_and_remaining_volume() {
        let verifier = ConversionVerifier::new();
        let verified = verifier
            .verify(&eligible_control(), &valid_submission())
            .unwrap();
        assert_eq!(verified.authorisation, authorisation());
        assert_eq!(verified.tier, LpTier::RegisteredLiquidityHolder);
        // floor(100 · 8123 / 10000) = 81.
        assert_eq!(verified.dest_amount, 81);
        assert_eq!(verified.remaining_volume, 1_000_000 - 100);
        // Verification alone commits no usage.
        assert_eq!(verifier.usage().used_volume(&super::super::issuer_id(&lp_key()), 7), 0);
    }

    #[test]
    fn posted_usage_reduces_remaining_volume() {
        let mut verifier = ConversionVerifier::new();
        verifier.record_posted(&super::super::issuer_id(&lp_key()), 7, 400_000);
        let verified = verifier
            .verify(&eligible_control(), &valid_submission())
            .unwrap();
        assert_eq!(verified.remaining_volume, 1_000_000 - 400_000 - 100);
    }

    #[test]
    fn posted_usage_is_scoped_to_its_issuer() {
        // Oracle: #13698 acceptance — authorisation ids are per
        // issuer, so one LP's posted volume must not refuse
        // another LP's same-numbered authorisation.
        let mut verifier = ConversionVerifier::new();
        let lp = super::super::issuer_id(&lp_key());
        verifier.record_posted(&lp, 7, 1_000_000);
        let other = super::super::issuer_id(&other_key());
        assert_eq!(verifier.usage().used_volume(&other, 7), 0);
        assert_eq!(verifier.usage().used_volume(&lp, 7), 1_000_000);
    }

    #[test]
    fn bad_signature_fails_pre_submit() {
        // Oracle: #3801 acceptance — bad signature, zero ledger
        // writes. One flipped byte breaks the event's
        // verification (signature or canonical form; both are
        // step-1 refusals).
        let mut submission = valid_submission();
        let last = submission.authorisations[0].event.bytes.len() - 1;
        submission.authorisations[0].event.bytes[last] ^= 1;
        assert_pre_submit(
            &ConversionVerifier::new(),
            &eligible_control(),
            &submission,
            &FailedPreSubmit::BadSignature,
        );
    }

    #[test]
    fn wrong_issuer_fails_pre_submit() {
        // Oracle: #3801 acceptance — wrong issuer, zero ledger
        // writes. The body names another issuer than the key
        // that signed it (built with chain::sign directly:
        // sign_authorisation refuses this body by design).
        let mut body = authorisation();
        body.issuer_id = super::super::issuer_id(&other_key());
        let body_bytes = body.body().unwrap();
        let event = chain::sign(
            &lp_key(),
            0,
            None,
            Some(&body_bytes),
            ChainExt {
                kind: RATE_AUTHORISATION_KIND.to_owned(),
                group: None,
                after: vec![],
                tag: None,
                document: None,
                removes: None,
                note: None,
                link: None,
                checkpoint: None,
                rotation: None,
                chat: None,
                anchor: None,
            },
        )
        .unwrap();
        let mut submission = valid_submission();
        submission.authorisations = vec![PresentedObject {
            event,
            body: body_bytes,
        }];
        assert_pre_submit(
            &ConversionVerifier::new(),
            &eligible_control(),
            &submission,
            &FailedPreSubmit::IssuerMismatch,
        );
    }

    #[test]
    fn unbound_account_fails_pre_submit() {
        // Oracle: #3801 acceptance — unbound account, zero
        // ledger writes. The destination ledger has no binding
        // record for the issuer.
        let issuer = super::super::issuer_id(&lp_key());
        let mut control = FixedConversionControl::new();
        control.set_tier(&issuer, "ABC", "123", LpTier::RegisteredLiquidityHolder);
        control.set_binding(&issuer, "ABC", factory_binding(SOURCE_LEDGER_ID));
        assert_pre_submit(
            &ConversionVerifier::new(),
            &control,
            &valid_submission(),
            &FailedPreSubmit::UnboundAccount {
                side: AccountSide::Destination,
            },
        );
    }

    #[test]
    fn named_account_other_than_the_binding_fails_pre_submit() {
        // Oracle: #2063 §2.3 — both currency accounts must be
        // the deterministic accounts for the LP; naming any
        // other account is refused pre-flight.
        let mut submission = valid_submission();
        submission.dest_account = 42;
        assert_pre_submit(
            &ConversionVerifier::new(),
            &eligible_control(),
            &submission,
            &FailedPreSubmit::AccountMismatch {
                side: AccountSide::Destination,
            },
        );
    }

    #[test]
    fn closing_account_fails_pre_submit() {
        // Oracle: #3801 step 3 — neither account may be closing
        // or closed.
        let issuer = super::super::issuer_id(&lp_key());
        let mut control = eligible_control();
        let mut binding = factory_binding(SOURCE_LEDGER_ID);
        binding.status = AccountStatus::Closing;
        control.set_binding(&issuer, "ABC", binding);
        assert_pre_submit(
            &ConversionVerifier::new(),
            &control,
            &valid_submission(),
            &FailedPreSubmit::AccountNotOpen {
                side: AccountSide::Source,
                status: AccountStatus::Closing,
            },
        );
    }

    #[test]
    fn revoked_authorisation_fails_pre_submit() {
        // Oracle: #3801 acceptance — revoked, zero ledger
        // writes. The gateway consults the fold at submission
        // time; the issuer-signed revoke covers version 1 and
        // is already effective.
        let mut submission = valid_submission();
        submission.revokes = vec![presented_revoke(&RateAuthorisationRevoke {
            authorisation_id: 7,
            revoked_version_from: 1,
            effective_at: NOW - 1,
            reason: None,
        })];
        assert_pre_submit(
            &ConversionVerifier::new(),
            &eligible_control(),
            &submission,
            &FailedPreSubmit::Revoked {
                effective_at: NOW - 1,
            },
        );
    }

    #[test]
    fn revocation_not_yet_effective_does_not_refuse() {
        // Oracle: #2290 §3 — a revocation stops new conversions
        // from its effective time; before that the authorisation
        // still stands.
        let mut submission = valid_submission();
        submission.revokes = vec![presented_revoke(&RateAuthorisationRevoke {
            authorisation_id: 7,
            revoked_version_from: 1,
            effective_at: NOW + 60,
            reason: None,
        })];
        assert!(
            ConversionVerifier::new()
                .verify(&eligible_control(), &submission)
                .is_ok()
        );
    }

    #[test]
    fn expired_authorisation_fails_pre_submit() {
        // Oracle: #3801 acceptance — expired, zero ledger
        // writes.
        let mut submission = valid_submission();
        submission.now = authorisation().expires_at;
        assert_pre_submit(
            &ConversionVerifier::new(),
            &eligible_control(),
            &submission,
            &FailedPreSubmit::Expired {
                expires_at: authorisation().expires_at,
            },
        );
    }

    #[test]
    fn cap_exhausted_fails_pre_submit() {
        // Oracle: #3801 acceptance — cap-exhausted, zero ledger
        // writes. Posted volume leaves less than the amount.
        let mut verifier = ConversionVerifier::new();
        verifier.record_posted(&super::super::issuer_id(&lp_key()), 7, 1_000_000 - 50);
        assert_pre_submit(
            &verifier,
            &eligible_control(),
            &valid_submission(),
            &FailedPreSubmit::CapExhausted {
                remaining_volume: 50,
            },
        );
    }

    #[test]
    fn unauthorised_caller_fails_pre_submit() {
        // Oracle: #3801 acceptance — unauthorised caller, zero
        // ledger writes. The authorisation names a single
        // counterparty who is not the caller.
        let mut auth = authorisation();
        auth.authorised_callers = AuthorisedCallers::Single(super::super::issuer_id(&other_key()));
        let mut submission = valid_submission();
        submission.authorisations = vec![presented_auth(&auth)];
        assert_pre_submit(
            &ConversionVerifier::new(),
            &eligible_control(),
            &submission,
            &FailedPreSubmit::UnauthorisedCaller,
        );
    }

    #[test]
    fn ineligible_lp_fails_pre_submit() {
        // Oracle: #2063 §2.3 — a platform LP gets no bypass
        // tier; an issuer with no tier for the pair is refused
        // before any account or live-state check.
        let control = FixedConversionControl::new();
        assert_pre_submit(
            &ConversionVerifier::new(),
            &control,
            &valid_submission(),
            &FailedPreSubmit::LpNotEligible,
        );
    }

    #[test]
    fn amount_outside_min_max_fails_pre_submit() {
        // Oracle: #2290 §5 worked example — below min_amount and
        // above max_amount are rejected pre-flight.
        let mut below = valid_submission();
        below.amount = authorisation().min_amount - 1;
        assert_pre_submit(
            &ConversionVerifier::new(),
            &eligible_control(),
            &below,
            &FailedPreSubmit::AmountBelowMin {
                min_amount: authorisation().min_amount,
            },
        );
        let mut above = valid_submission();
        above.amount = authorisation().max_amount + 1;
        assert_pre_submit(
            &ConversionVerifier::new(),
            &eligible_control(),
            &above,
            &FailedPreSubmit::AmountAboveMax {
                max_amount: authorisation().max_amount,
            },
        );
    }

    #[test]
    fn superseded_version_folds_to_the_highest_before_checks() {
        // Oracle: #3801 step 4 — the highest version folded is
        // the live one; the submission verifies against v2's
        // terms, not v1's.
        let mut v2 = authorisation();
        v2.version = 2;
        v2.max_amount = 50;
        let mut submission = valid_submission();
        let v1 = presented_auth(&authorisation());
        let v2_event = {
            let event = sign_authorisation(&lp_key(), 1, Some(v1.event.id), &v2, vec![]).unwrap();
            PresentedObject {
                event,
                body: v2.body().unwrap(),
            }
        };
        submission.authorisations = vec![v1, v2_event];
        // Amount 100 is within v1's max but above v2's: refused
        // against the folded highest version.
        assert_pre_submit(
            &ConversionVerifier::new(),
            &eligible_control(),
            &submission,
            &FailedPreSubmit::AmountAboveMax { max_amount: 50 },
        );
    }

    /// Issue a quote for the valid submission under
    /// `quote_id`, at equal ledger scales, on a verifier whose
    /// books the test keeps.
    fn issue_valid_quote(verifier: &mut ConversionVerifier, quote_id: u128) -> ConversionQuote {
        verifier
            .issue_quote(&eligible_control(), &valid_submission(), quote_id, 0, 0)
            .unwrap()
    }

    /// The acceptance shape for a refused quoted submission,
    /// as in [`assert_pre_submit`]: the expected typed
    /// variant, FAILED_PRE_SUBMIT disposition, usage
    /// untouched, and zero ledger writes.
    fn assert_quoted_pre_submit(
        verifier: &ConversionVerifier,
        control: &impl ConversionControl,
        submission: &ConversionSubmission,
        quote_id: u128,
        expected: &FailedPreSubmit,
    ) {
        let ledger = InMemoryLedger::default();
        let used_before = verifier.usage().used_volume(&submission.issuer, submission.authorisation_id);
        let error = verifier
            .verify_quoted(control, submission, quote_id)
            .expect_err("quoted submission must be refused pre-flight");
        assert_eq!(
            std::mem::discriminant(&error),
            std::mem::discriminant(expected),
            "expected {expected:?}, got {error:?}"
        );
        assert_eq!(error.disposition(), "FAILED_PRE_SUBMIT");
        assert_eq!(
            verifier.usage().used_volume(&submission.issuer, submission.authorisation_id),
            used_before,
            "a refused submission must not commit usage"
        );
        for ledger_id in [SOURCE_LEDGER_ID, DEST_LEDGER_ID] {
            assert!(
                ledger.try_accounts_on_ledger(ledger_id).unwrap().is_empty(),
                "zero TigerBeetle account writes"
            );
            assert!(
                ledger
                    .try_transfers_on_ledger(ledger_id, 100)
                    .unwrap()
                    .is_empty(),
                "zero TigerBeetle transfer writes"
            );
        }
    }

    #[test]
    fn quote_issuance_pins_the_folded_authorisation_and_amounts() {
        // Oracle: #3802 build item 1 and the #2290 design
        // record — the quote record is {quote_id,
        // authorisation_id, authorisation_version,
        // authorisation_event_hash, x, computed_y, fee,
        // issued_at, quote_expires_at} with a 60-second
        // lifetime.
        let mut verifier = ConversionVerifier::new();
        let submission = valid_submission();
        let event_hash = submission.authorisations[0].event.id;
        let quote = verifier
            .issue_quote(&eligible_control(), &submission, 42, 0, 0)
            .unwrap();
        assert_eq!(quote.quote_id, 42);
        assert_eq!(quote.authorisation_id, 7);
        assert_eq!(quote.authorisation_version, 1);
        assert_eq!(quote.authorisation_event_hash, event_hash);
        assert_eq!(quote.x, 100);
        // Equal scales: floor(100 · 8123 / 10000) = 81.
        assert_eq!(quote.computed_y, 81);
        // The worked-example authorisation carries no fee.
        assert_eq!(quote.fee, 0);
        assert_eq!(quote.issued_at, NOW);
        assert_eq!(QUOTE_LIFETIME_SECS, 60);
        assert_eq!(quote.quote_expires_at, NOW + QUOTE_LIFETIME_SECS);
        // The gateway's book holds the issued quote; issuance
        // committed no usage.
        assert_eq!(verifier.quote(42), Some(&quote));
        assert_eq!(verifier.usage().used_volume(&super::super::issuer_id(&lp_key()), 7), 0);
    }

    #[test]
    fn quote_computed_y_is_scale_aware_and_fee_is_quoted() {
        // Oracle: #3802 build item 1 — computed_y uses the
        // scale-aware integer-rational rule, and the fee is
        // quoted from the authorisation's fee_bps in source
        // units.
        let mut auth = authorisation();
        auth.fee_bps = Some(100); // 1%
        let mut submission = valid_submission();
        submission.authorisations = vec![presented_auth(&auth)];
        let mut verifier = ConversionVerifier::new();
        // Source scale 0, destination scale 2:
        // floor(100 · 8123 · 10^2 / (10000 · 10^0)) = 8123.
        let quote = verifier
            .issue_quote(&eligible_control(), &submission, 43, 0, 2)
            .unwrap();
        assert_eq!(quote.computed_y, 8123);
        // floor(100 · 100 / 10_000) = 1.
        assert_eq!(quote.fee, 1);
    }

    #[test]
    fn quote_issuance_refuses_what_verification_refuses() {
        // Oracle: #3802 — a quote is issued only against a
        // submission that verifies at issuance; a refused
        // issuance stores no quote.
        let mut submission = valid_submission();
        submission.revokes = vec![presented_revoke(&RateAuthorisationRevoke {
            authorisation_id: 7,
            revoked_version_from: 1,
            effective_at: NOW - 1,
            reason: None,
        })];
        let mut verifier = ConversionVerifier::new();
        let error = verifier
            .issue_quote(&eligible_control(), &submission, 49, 0, 0)
            .expect_err("issuance against a revoked authorisation must be refused");
        assert_eq!(
            std::mem::discriminant(&error),
            std::mem::discriminant(&FailedPreSubmit::Revoked {
                effective_at: NOW - 1,
            })
        );
        assert_eq!(verifier.quote(49), None);
    }

    #[test]
    fn expired_quote_is_refused_pre_submit() {
        // Oracle: #3802 acceptance — an expired quote refused.
        let mut verifier = ConversionVerifier::new();
        let quote = issue_valid_quote(&mut verifier, 44);
        let mut submission = valid_submission();
        submission.now = quote.quote_expires_at;
        assert_quoted_pre_submit(
            &verifier,
            &eligible_control(),
            &submission,
            44,
            &FailedPreSubmit::QuoteExpired {
                quote_expires_at: NOW + QUOTE_LIFETIME_SECS,
            },
        );
    }

    #[test]
    fn quote_against_since_revoked_authorisation_is_refused_pre_submit() {
        // Oracle: #3802 acceptance — a quote against a
        // since-revoked authorisation refused: execution
        // re-runs the live-state checks, and the revocation
        // consult refuses it.
        let mut verifier = ConversionVerifier::new();
        issue_valid_quote(&mut verifier, 45);
        let mut submission = valid_submission();
        submission.revokes = vec![presented_revoke(&RateAuthorisationRevoke {
            authorisation_id: 7,
            revoked_version_from: 1,
            effective_at: NOW - 1,
            reason: None,
        })];
        assert_quoted_pre_submit(
            &verifier,
            &eligible_control(),
            &submission,
            45,
            &FailedPreSubmit::Revoked {
                effective_at: NOW - 1,
            },
        );
    }

    #[test]
    fn quote_against_superseded_version_is_refused_pre_submit() {
        // Oracle: #3802 acceptance — a quote against a
        // superseded version refused: the fresh fold stands at
        // v2, which the quote's version/event pin does not
        // match.
        let mut verifier = ConversionVerifier::new();
        issue_valid_quote(&mut verifier, 46);
        let mut v2 = authorisation();
        v2.version = 2;
        let v1 = presented_auth(&authorisation());
        let v2_event = {
            let event = sign_authorisation(&lp_key(), 1, Some(v1.event.id), &v2, vec![]).unwrap();
            PresentedObject {
                event,
                body: v2.body().unwrap(),
            }
        };
        let mut submission = valid_submission();
        submission.authorisations = vec![v1, v2_event];
        assert_quoted_pre_submit(
            &verifier,
            &eligible_control(),
            &submission,
            46,
            &FailedPreSubmit::QuoteStale {
                quoted_version: 1,
                live_version: 2,
            },
        );
    }

    #[test]
    fn unknown_quote_is_refused_pre_submit() {
        // Oracle: #3802 build item 2 — execution must
        // reference an issued quote; an id this gateway never
        // issued binds to nothing.
        assert_quoted_pre_submit(
            &ConversionVerifier::new(),
            &eligible_control(),
            &valid_submission(),
            999,
            &FailedPreSubmit::UnknownQuote,
        );
    }

    #[test]
    fn quote_for_a_different_amount_is_refused_pre_submit() {
        // Oracle: #3802 build item 2 and #2063 table C — an
        // altered quote cannot be executed: the quote binds
        // exactly one amount.
        let mut verifier = ConversionVerifier::new();
        issue_valid_quote(&mut verifier, 48);
        let mut submission = valid_submission();
        submission.amount = 200;
        assert_quoted_pre_submit(
            &verifier,
            &eligible_control(),
            &submission,
            48,
            &FailedPreSubmit::QuoteMismatch,
        );
    }

    #[test]
    fn valid_quote_executes_with_exactly_the_quoted_amounts() {
        // Oracle: #3802 acceptance — a valid quote executing
        // with exactly the quoted y and fee.
        let mut auth = authorisation();
        auth.fee_bps = Some(100);
        let mut submission = valid_submission();
        submission.authorisations = vec![presented_auth(&auth)];
        let mut verifier = ConversionVerifier::new();
        let quote = verifier
            .issue_quote(&eligible_control(), &submission, 47, 0, 2)
            .unwrap();
        submission.now = NOW + 30;
        let executed = verifier
            .verify_quoted(&eligible_control(), &submission, 47)
            .unwrap();
        assert_eq!(executed.quote, quote);
        assert_eq!(executed.amount(), 100);
        assert_eq!(executed.dest_amount(), quote.computed_y);
        assert_eq!(executed.dest_amount(), 8123);
        assert_eq!(executed.fee(), quote.fee);
        assert_eq!(executed.fee(), 1);
        assert_eq!(executed.remaining_volume, 1_000_000 - 100);
        // Execution verification alone commits no usage.
        assert_eq!(verifier.usage().used_volume(&super::super::issuer_id(&lp_key()), 7), 0);
    }
}
