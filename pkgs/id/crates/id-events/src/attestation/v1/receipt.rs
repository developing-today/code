//! Witness fallback, independent receipts and closure (#2039 phase 5).
//!
//! The design (`doc/2026-11-01_design_id2039.md` §4, Open decisions 1 and
//! 5) names one pre-agreed witness plus an optional pre-named fallback, with
//! the fallback delay pinned before play, and requires independent receipt
//! evidence before any state is protocol-final. This module holds the
//! signed objects for that flow:
//!
//! - [`WitnessTerms`], carried in the genesis agreement: the fallback
//!   witness and its delay, the independent receipt issuer, and the cap on
//!   the objection window counted from first receipt.
//! - [`Receipt`] (kind `attestation.receipt`): the issuer's signed record
//!   that it received an object from a party, or served one to a party, at a
//!   time on its own clock. Only the issuer named in genesis can sign one;
//!   a player's or witness's own frontier never establishes receipt time.
//! - [`Closure`] (kind `attestation.closure`): the issuer's signed statement
//!   that a witness decision's objection window closed. The reducer checks
//!   it against the receipts before treating a decision as final.
//!
//! Offline matches are explicit: with no receipt issuer the terms forbid a
//! fallback (nobody can prove the primary stayed silent past the delay), and
//! the reducer reports a witnessed result as not provably final rather than
//! guessing at time.

use p2panda_core::{Hash, SeqNum, SigningKey};
use serde::{Deserialize, Serialize};

use super::{V1SignError, VERSION, sign_body};
use crate::attestation::{GenesisAgreement, PlayerId, player_id};
use id_store::chain::Event;

/// Event kind of an issuer receipt.
pub const RECEIPT_KIND: &str = "attestation.receipt";
/// Event kind of an issuer closure statement.
pub const CLOSURE_KIND: &str = "attestation.closure";

/// Witness fallback and receipt terms, pinned in genesis before play.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WitnessTerms {
    /// A pre-named fallback witness. It may decide only once independent
    /// receipts show the primary witness stayed silent for
    /// `fallback_delay_secs` after the issuer first received a claim.
    pub fallback: Option<PlayerId>,
    /// Seconds the primary witness has, from the issuer's first receipt of a
    /// claim, before a fallback decision can take effect.
    pub fallback_delay_secs: u64,
    /// The independent receipt issuer (an observer or registry service).
    pub receipt_issuer: Option<PlayerId>,
    /// The longest a party's objection window may run, counted from the
    /// issuer's first receipt of the decision. A party the issuer never
    /// served gets the cap; one it served gets the agreement's
    /// `objection_window_secs` from that service, never past the cap.
    pub objection_cap_secs: u64,
}

/// Why a set of [`WitnessTerms`] cannot be used with an agreement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TermsError {
    /// A fallback needs a primary witness to fall back from.
    FallbackWithoutWitness,
    /// The fallback is the primary witness.
    FallbackIsWitness,
    /// The fallback or the receipt issuer is a seated player.
    PartyIsPlayer,
    /// A fallback needs a positive delay.
    ZeroFallbackDelay,
    /// A fallback needs a receipt issuer to prove the delay elapsed.
    FallbackWithoutIssuer,
    /// The receipt issuer is one of the witnesses; it cannot attest to
    /// receipt of its own decisions.
    IssuerIsWitness,
    /// A receipt issuer needs a positive objection cap no shorter than the
    /// agreement's objection window.
    CapTooShort,
}

impl std::fmt::Display for TermsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::FallbackWithoutWitness => "a fallback witness needs a primary witness",
            Self::FallbackIsWitness => "the fallback witness is the primary witness",
            Self::PartyIsPlayer => "a witness or receipt issuer cannot be a seated player",
            Self::ZeroFallbackDelay => "a fallback witness needs a positive delay",
            Self::FallbackWithoutIssuer => {
                "a fallback witness needs a receipt issuer to prove the delay elapsed"
            }
            Self::IssuerIsWitness => "the receipt issuer cannot be a witness",
            Self::CapTooShort => {
                "the objection cap must be positive and at least the objection window"
            }
        })
    }
}

impl std::error::Error for TermsError {}

impl WitnessTerms {
    /// Checks these terms against the agreement that carries them.
    ///
    /// # Errors
    ///
    /// Returns the first [`TermsError`] the terms break.
    pub fn validate(&self, agreement: &GenesisAgreement) -> Result<(), TermsError> {
        let seated = |party: &PlayerId| agreement.players.contains(party);
        if let Some(fallback) = &self.fallback {
            let Some(witness) = &agreement.witness else {
                return Err(TermsError::FallbackWithoutWitness);
            };
            if fallback == witness {
                return Err(TermsError::FallbackIsWitness);
            }
            if seated(fallback) {
                return Err(TermsError::PartyIsPlayer);
            }
            if self.fallback_delay_secs == 0 {
                return Err(TermsError::ZeroFallbackDelay);
            }
            if self.receipt_issuer.is_none() {
                return Err(TermsError::FallbackWithoutIssuer);
            }
        }
        if let Some(issuer) = &self.receipt_issuer {
            if seated(issuer) {
                return Err(TermsError::PartyIsPlayer);
            }
            if agreement.witness.as_ref() == Some(issuer) || self.fallback.as_ref() == Some(issuer)
            {
                return Err(TermsError::IssuerIsWitness);
            }
            if self.objection_cap_secs == 0
                || self.objection_cap_secs < agreement.objection_window_secs
            {
                return Err(TermsError::CapTooShort);
            }
        }
        Ok(())
    }

    /// The terms an agreement runs under: its own, or the defaults (no
    /// fallback, no issuer) when it carries none.
    #[must_use]
    pub fn of(agreement: &GenesisAgreement) -> Self {
        agreement.witness_terms.clone().unwrap_or_default()
    }

    /// Whether `party` may sign a witness decision under `agreement`: the
    /// primary witness, or the pre-named fallback.
    #[must_use]
    pub fn may_decide(agreement: &GenesisAgreement, party: &PlayerId) -> bool {
        agreement.witness.as_ref() == Some(party)
            || agreement
                .witness_terms
                .as_ref()
                .and_then(|terms| terms.fallback.as_ref())
                == Some(party)
    }
}

/// What the issuer saw happen to the receipt's subject.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReceiptEvent {
    /// The issuer received the subject from this party.
    Received {
        /// Who handed the subject to the issuer.
        from: PlayerId,
    },
    /// The issuer served (delivered) the subject to this party.
    Served {
        /// Who the issuer delivered the subject to.
        to: PlayerId,
    },
}

/// The receipt issuer's signed record of one receipt or service.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    /// Object version ([`VERSION`]).
    pub version: u8,
    /// The agreement hash this receipt belongs to.
    pub agreement: Hash,
    /// The object received or served.
    pub subject: Hash,
    /// What happened.
    pub event: ReceiptEvent,
    /// When, in seconds on the issuer's clock.
    pub at_secs: u64,
}

impl Receipt {
    /// The canonical body bytes.
    ///
    /// # Errors
    ///
    /// Returns a JSON error if the receipt cannot be serialized.
    pub fn body(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// Decodes a receipt, refusing repeated object keys.
    ///
    /// # Errors
    ///
    /// Returns a JSON error if the bytes are not a receipt.
    pub fn from_body(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        id_core::json_unique::from_slice_without_duplicate_keys(bytes)
    }
}

/// The receipt issuer's signed statement that a decision's objection window
/// closed at `closed_at_secs`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Closure {
    /// Object version ([`VERSION`]).
    pub version: u8,
    /// The agreement hash this closure belongs to.
    pub agreement: Hash,
    /// The witness decision whose window closed.
    pub decision: Hash,
    /// When the window closed, in seconds on the issuer's clock.
    pub closed_at_secs: u64,
}

impl Closure {
    /// The canonical body bytes.
    ///
    /// # Errors
    ///
    /// Returns a JSON error if the closure cannot be serialized.
    pub fn body(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// Decodes a closure, refusing repeated object keys.
    ///
    /// # Errors
    ///
    /// Returns a JSON error if the bytes are not a closure.
    pub fn from_body(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        id_core::json_unique::from_slice_without_duplicate_keys(bytes)
    }
}

fn check_issuer(
    key: &SigningKey,
    agreement: &GenesisAgreement,
    version: u8,
    named: Hash,
) -> Result<(), V1SignError> {
    let terms = WitnessTerms::of(agreement);
    terms
        .validate(agreement)
        .map_err(V1SignError::InvalidTerms)?;
    if terms.receipt_issuer != Some(player_id(key)) {
        return Err(V1SignError::NotReceiptIssuer);
    }
    if version != VERSION || named != agreement.agreement_hash()? {
        return Err(V1SignError::WrongAgreement);
    }
    Ok(())
}

/// Signs an issuer receipt. Only the receipt issuer named in the
/// agreement's [`WitnessTerms`] may sign one.
///
/// # Errors
///
/// [`V1SignError::NotReceiptIssuer`] for any other signer,
/// [`V1SignError::InvalidTerms`] for unusable terms, and
/// [`V1SignError::WrongAgreement`] for a receipt naming another agreement.
pub fn sign_receipt(
    key: &SigningKey,
    seq_num: SeqNum,
    backlink: Option<Hash>,
    agreement: &GenesisAgreement,
    receipt: &Receipt,
    after: Vec<Hash>,
) -> Result<Event, V1SignError> {
    check_issuer(key, agreement, receipt.version, receipt.agreement)?;
    sign_body(key, seq_num, backlink, receipt.body()?, RECEIPT_KIND, after)
        .map_err(V1SignError::from)
}

/// Signs an issuer closure statement. Only the receipt issuer named in the
/// agreement's [`WitnessTerms`] may sign one; the reducer still checks it
/// against the receipts.
///
/// # Errors
///
/// As [`sign_receipt`].
pub fn sign_closure(
    key: &SigningKey,
    seq_num: SeqNum,
    backlink: Option<Hash>,
    agreement: &GenesisAgreement,
    closure: &Closure,
    after: Vec<Hash>,
) -> Result<Event, V1SignError> {
    check_issuer(key, agreement, closure.version, closure.agreement)?;
    sign_body(key, seq_num, backlink, closure.body()?, CLOSURE_KIND, after)
        .map_err(V1SignError::from)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{
        Closure, Receipt, ReceiptEvent, TermsError, WitnessTerms, sign_closure, sign_receipt,
    };
    use crate::attestation::v1::{
        V1SignError, VERSION, WitnessDecision, WitnessEvidence, sign_witness_decision,
    };
    use crate::attestation::{GenesisAgreement, PlayerId, player_id};
    use p2panda_core::{Hash, SigningKey};

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn id(seed: u8) -> PlayerId {
        player_id(&key(seed))
    }

    fn agreement(terms: Option<WitnessTerms>) -> GenesisAgreement {
        GenesisAgreement {
            game_id: "tic-tac-toe".to_owned(),
            logic_hash: Hash::digest(b"logic"),
            players: [id(1), id(2)],
            witness: Some(id(3)),
            objection_window_secs: 3600,
            witness_terms: terms,
        }
    }

    fn terms() -> WitnessTerms {
        WitnessTerms {
            fallback: Some(id(5)),
            fallback_delay_secs: 600,
            receipt_issuer: Some(id(6)),
            objection_cap_secs: 7200,
        }
    }

    #[test]
    fn unset_terms_keep_the_phase_four_agreement_bytes() {
        let plain = agreement(None);
        let body = String::from_utf8(plain.body().unwrap()).unwrap();
        assert!(!body.contains("witness_terms"), "{body}");
        let decoded = GenesisAgreement::from_body(plain.body().unwrap().as_slice()).unwrap();
        assert_eq!(decoded, plain);
        let with = agreement(Some(terms()));
        assert_ne!(
            with.agreement_hash().unwrap(),
            plain.agreement_hash().unwrap()
        );
        let decoded = GenesisAgreement::from_body(with.body().unwrap().as_slice()).unwrap();
        assert_eq!(decoded, with);
    }

    #[test]
    fn terms_validation() {
        let check = |edit: fn(&mut GenesisAgreement), expected: Result<(), TermsError>| {
            let mut agreement = agreement(Some(terms()));
            edit(&mut agreement);
            let terms = WitnessTerms::of(&agreement);
            assert_eq!(terms.validate(&agreement), expected);
        };
        check(|_| {}, Ok(()));
        check(|a| a.witness_terms = None, Ok(()));
        check(
            |a| a.witness = None,
            Err(TermsError::FallbackWithoutWitness),
        );
        check(
            |a| a.witness_terms.as_mut().unwrap().fallback = a.witness,
            Err(TermsError::FallbackIsWitness),
        );
        check(
            |a| a.witness_terms.as_mut().unwrap().fallback = Some(a.players[0]),
            Err(TermsError::PartyIsPlayer),
        );
        check(
            |a| a.witness_terms.as_mut().unwrap().fallback_delay_secs = 0,
            Err(TermsError::ZeroFallbackDelay),
        );
        check(
            |a| a.witness_terms.as_mut().unwrap().receipt_issuer = None,
            Err(TermsError::FallbackWithoutIssuer),
        );
        check(
            |a| a.witness_terms.as_mut().unwrap().receipt_issuer = Some(a.players[1]),
            Err(TermsError::PartyIsPlayer),
        );
        check(
            |a| a.witness_terms.as_mut().unwrap().receipt_issuer = a.witness,
            Err(TermsError::IssuerIsWitness),
        );
        check(
            |a| a.witness_terms.as_mut().unwrap().objection_cap_secs = 60,
            Err(TermsError::CapTooShort),
        );
        // Offline: a primary witness alone, no issuer, is fine.
        check(
            |a| {
                a.witness_terms = Some(super::WitnessTerms::default());
            },
            Ok(()),
        );
    }

    fn receipt(agreement: &GenesisAgreement) -> Receipt {
        Receipt {
            version: VERSION,
            agreement: agreement.agreement_hash().unwrap(),
            subject: Hash::digest(b"claim"),
            event: ReceiptEvent::Received { from: id(1) },
            at_secs: 10,
        }
    }

    #[test]
    fn only_the_named_issuer_signs_receipts_and_closures() {
        let agreement = agreement(Some(terms()));
        let receipt = receipt(&agreement);
        let event = sign_receipt(&key(6), 0, None, &agreement, &receipt, Vec::new()).unwrap();
        assert_eq!(event.header.extensions.kind, super::RECEIPT_KIND);
        for seat in [1, 2, 3, 5, 9] {
            assert!(matches!(
                sign_receipt(&key(seat), 0, None, &agreement, &receipt, Vec::new()),
                Err(V1SignError::NotReceiptIssuer)
            ));
        }
        let closure = Closure {
            version: VERSION,
            agreement: agreement.agreement_hash().unwrap(),
            decision: Hash::digest(b"decision"),
            closed_at_secs: 9000,
        };
        assert!(sign_closure(&key(6), 1, None, &agreement, &closure, Vec::new()).is_ok());
        assert!(matches!(
            sign_closure(&key(3), 0, None, &agreement, &closure, Vec::new()),
            Err(V1SignError::NotReceiptIssuer)
        ));
        let mut stale = closure;
        stale.agreement = Hash::digest(b"another agreement");
        assert!(matches!(
            sign_closure(&key(6), 1, None, &agreement, &stale, Vec::new()),
            Err(V1SignError::WrongAgreement)
        ));
        let mut broken = agreement;
        broken.witness_terms.as_mut().unwrap().objection_cap_secs = 0;
        assert!(matches!(
            sign_receipt(&key(6), 0, None, &broken, &receipt, Vec::new()),
            Err(V1SignError::InvalidTerms(TermsError::CapTooShort))
        ));
    }

    #[test]
    fn receipts_and_closures_round_trip_and_refuse_duplicate_keys() {
        let agreement = agreement(Some(terms()));
        let receipt = receipt(&agreement);
        assert_eq!(
            Receipt::from_body(&receipt.body().unwrap()).unwrap(),
            receipt
        );
        let served = Receipt {
            event: ReceiptEvent::Served { to: id(2) },
            ..receipt
        };
        assert_eq!(Receipt::from_body(&served.body().unwrap()).unwrap(), served);
        let closure = Closure {
            version: VERSION,
            agreement: agreement.agreement_hash().unwrap(),
            decision: Hash::digest(b"decision"),
            closed_at_secs: 9000,
        };
        let body = closure.body().unwrap();
        assert_eq!(Closure::from_body(&body).unwrap(), closure);
        let mut doubled = String::from_utf8(body).unwrap();
        doubled.insert_str(1, "\"closed_at_secs\":1,");
        assert!(Closure::from_body(doubled.as_bytes()).is_err());
    }

    #[test]
    fn the_fallback_may_sign_a_decision_and_nobody_else() {
        let agreement = agreement(Some(terms()));
        let decision = WitnessDecision {
            version: VERSION,
            agreement: agreement.agreement_hash().unwrap(),
            logic_hash: agreement.logic_hash,
            outcome: "draw".to_owned(),
            evidence: WitnessEvidence::Complete {
                claim_a: Hash::digest(b"claim a"),
                claim_b: Hash::digest(b"claim b"),
                log_hash_a: Hash::digest(b"log a"),
                log_hash_b: Hash::digest(b"log b"),
            },
        };
        for seat in [3, 5] {
            assert!(
                sign_witness_decision(&key(seat), 0, None, &agreement, &decision, Vec::new())
                    .is_ok(),
                "{seat}"
            );
        }
        for seat in [1, 6, 9] {
            assert!(matches!(
                sign_witness_decision(&key(seat), 0, None, &agreement, &decision, Vec::new()),
                Err(V1SignError::NotNamedWitness)
            ));
        }
        // Without the terms, the fallback is just another key.
        let plain = self::agreement(None);
        let decision = WitnessDecision {
            agreement: plain.agreement_hash().unwrap(),
            ..decision
        };
        assert!(matches!(
            sign_witness_decision(&key(5), 0, None, &plain, &decision, Vec::new()),
            Err(V1SignError::NotNamedWitness)
        ));
    }
}
