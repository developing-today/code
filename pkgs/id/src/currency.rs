//! Currency rate authorisations as signed ID-graph objects (#2290,
//! Phase 1 = #3800).
//!
//! A `RateAuthorisation` is an ordinary signed chained event
//! ([`crate::chain`]) whose body is the authorisation object, following
//! the [`crate::attestation`] precedent. The ledger-facing field set is
//! decided by the conversion-engine design (#2063 §2); the graph
//! representation is the task 5741 design record on #2290:
//!
//! - **Authorisation** (kind `currency.rate_authorisation`): one
//!   direction only — `source_ledger` → `dest_ledger` at the
//!   integer-rational rate `rate_num / rate_den`, applied scale-aware
//!   as `y = floor(x · rate_num / rate_den)` (rounding always down).
//!   The reverse direction is a separate object or none; neither
//!   implies the other.
//! - **Revocation** (kind `currency.rate_authorisation_revoke`): a
//!   signed event on the issuer's chain naming the authorisation, the
//!   version from which it is revoked, and the effective time.
//!   Revocation stops new conversions only; posted conversions are
//!   untouched (#2063 §2).
//!
//! State is a fold over the objects alone ([`fold_authorisation`]):
//! the live object is the highest version; a signed revoke covering
//! that version folds the authorisation as revoked, and no older
//! version revives in its place. A body naming an issuer other than
//! the event's verified author is invalid, not merely untrusted
//! (design record, gateway verification step 1).
//!
//! Library-only: no gateway or control-plane code lives here (that is
//! Phases 2–3, #3801/#3802).

use p2panda_core::{Hash, SeqNum, SigningKey};
use serde::{Deserialize, Serialize};

use crate::chain::{self, ChainExt, Event};

/// The event kind carrying a rate authorisation body.
pub const RATE_AUTHORISATION_KIND: &str = "currency.rate_authorisation";

/// The event kind carrying a rate authorisation revocation body.
pub const RATE_AUTHORISATION_REVOKE_KIND: &str = "currency.rate_authorisation_revoke";

/// An issuer identity: the 32 bytes of its signing key's verifying
/// key, the same bytes the event store uses as an author key.
pub type IssuerId = [u8; 32];

/// The issuer id of a signing key.
#[must_use]
pub fn issuer_id(key: &SigningKey) -> IssuerId {
    *key.verifying_key().as_bytes()
}

/// The author of a decoded (signature-verified) event, as an issuer id.
#[must_use]
pub fn author_id(event: &Event) -> IssuerId {
    *event.header.verifying_key.as_bytes()
}

/// Who may convert under an authorisation (#2063 §2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorisedCallers {
    /// Any caller.
    Any,
    /// Only the listed identities.
    Allowlist(Vec<IssuerId>),
    /// Only this one counterparty.
    Single(IssuerId),
}

impl AuthorisedCallers {
    /// Whether `caller` is authorised under this policy.
    #[must_use]
    pub fn allows(&self, caller: &IssuerId) -> bool {
        match self {
            Self::Any => true,
            Self::Allowlist(list) => list.contains(caller),
            Self::Single(only) => only == caller,
        }
    }
}

/// A rate authorisation body: LP `issuer_id` offers conversions from
/// `source_ledger` to `dest_ledger` at `rate_num / rate_den`, under
/// the amount, cap, caller, and time policy carried here. One
/// direction only; the reverse is a separate object or none.
///
/// A new version is a new signed event on the issuer's chain with the
/// same `authorisation_id` and `version + 1`. No in-place edits, ever.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateAuthorisation {
    /// Stable across versions of this authorisation.
    pub authorisation_id: u128,
    /// Monotonic per `authorisation_id`.
    pub version: u64,
    /// The offering LP; must equal the event author (verifying key).
    pub issuer_id: IssuerId,
    /// The ledger conversions draw from.
    pub source_ledger: String,
    /// The ledger conversions pay into.
    pub dest_ledger: String,
    /// Rate numerator (integer-rational rate, #2063 §2).
    pub rate_num: u128,
    /// Rate denominator; non-zero in a valid authorisation.
    pub rate_den: u128,
    /// Smallest convertible amount, in source-ledger units.
    pub min_amount: u128,
    /// Largest convertible amount, in source-ledger units.
    pub max_amount: u128,
    /// Total convertible volume, in source-ledger units.
    pub total_cap: u128,
    /// Who may convert under this authorisation.
    pub authorised_callers: AuthorisedCallers,
    /// No conversions before this time (unix seconds).
    pub not_before: u64,
    /// Mandatory expiry (unix seconds): no conversions at or after.
    pub expires_at: u64,
    /// Optional window (seconds) in which a posted conversion may be
    /// reversed at the original rate (#2063 §4.4).
    pub reversal_window_secs: Option<u64>,
    /// Optional fee in basis points; fees are non-refundable unless
    /// stated (#2063 §4.4).
    pub fee_bps: Option<u32>,
}

impl RateAuthorisation {
    /// The canonical body bytes this authorisation is signed over.
    ///
    /// # Errors
    ///
    /// Returns a JSON error if the authorisation cannot be serialized
    /// (not expected for this shape).
    pub fn body(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// Decodes an authorisation from a signed event's body bytes.
    ///
    /// # Errors
    ///
    /// Returns a JSON error if the bytes are not an authorisation object.
    pub fn from_body(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    /// Checks this body against its event's verified author.
    ///
    /// # Errors
    ///
    /// Returns [`ValidateError::IssuerMismatch`] if the body names an
    /// issuer other than the event author, and
    /// [`ValidateError::ZeroRateDenominator`] if `rate_den` is zero.
    pub fn validate(&self, author: &IssuerId) -> Result<(), ValidateError> {
        if self.issuer_id != *author {
            return Err(ValidateError::IssuerMismatch);
        }
        if self.rate_den == 0 {
            return Err(ValidateError::ZeroRateDenominator);
        }
        Ok(())
    }

    /// Converts a source amount at this authorisation's rate:
    /// `floor(amount · rate_num / rate_den)`, rounding always down in
    /// the LP's favour (#2063 §2). Callers apply ledger scale
    /// normalisation to the result.
    ///
    /// Returns `None` on overflow or a zero denominator.
    #[must_use]
    pub fn apply_rate(&self, amount: u128) -> Option<u128> {
        amount
            .checked_mul(self.rate_num)?
            .checked_div(self.rate_den)
    }
}

/// A revocation body: the issuer revokes `authorisation_id` from
/// version `revoked_version_from` at `effective_at`. Only the
/// issuer's signature revokes in the graph; the platform can only
/// suspend gateway acceptance locally (#2290 design record).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateAuthorisationRevoke {
    /// The authorisation being revoked.
    pub authorisation_id: u128,
    /// The first revoked version; this and later versions are revoked.
    pub revoked_version_from: u64,
    /// When the revocation takes effect (unix seconds).
    pub effective_at: u64,
    /// Optional human-readable reason.
    pub reason: Option<String>,
}

impl RateAuthorisationRevoke {
    /// The canonical body bytes this revocation is signed over.
    ///
    /// # Errors
    ///
    /// As [`RateAuthorisation::body`].
    pub fn body(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// Decodes a revocation from a signed event's body bytes.
    ///
    /// # Errors
    ///
    /// As [`RateAuthorisation::from_body`].
    pub fn from_body(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

/// Why an authorisation body is invalid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidateError {
    /// The body names an issuer other than the event author.
    IssuerMismatch,
    /// The rate denominator is zero.
    ZeroRateDenominator,
}

impl std::fmt::Display for ValidateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IssuerMismatch => {
                write!(
                    f,
                    "authorisation body names an issuer other than the event author"
                )
            }
            Self::ZeroRateDenominator => write!(f, "authorisation rate denominator is zero"),
        }
    }
}

impl std::error::Error for ValidateError {}

/// The folded state of one authorisation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthorisationState {
    /// The highest version stands and no signed revoke covers it.
    Live(RateAuthorisation),
    /// The highest version is covered by an issuer-signed revoke.
    /// No older version revives in its place: revocation is terminal
    /// for the versions it covers, and a superseded lower version was
    /// never live once a higher version existed.
    Revoked {
        /// The highest (revoked) version of the authorisation.
        authorisation: RateAuthorisation,
        /// The first revoked version, from the covering revoke.
        revoked_version_from: u64,
        /// When the covering revocation takes effect (unix seconds).
        effective_at: u64,
    },
}

/// Folds one authorisation's state from its signed objects alone.
///
/// `authorisations` pairs each authorisation event's verified author
/// with the body decoded from it; `revokes` does the same for revoke
/// events. The caller obtains authors from decoded events
/// ([`author_id`]) — [`chain::decode`] has already verified every
/// signature before this function sees an object.
///
/// Rules: bodies for other authorisation ids are ignored. A body
/// whose named issuer is not its event author is invalid and ignored,
/// as is a revoke signed by anyone other than the authorisation's
/// issuer. Among the valid versions the highest is the candidate:
/// if an issuer-signed revoke covers it (`revoked_version_from <=`
/// highest version) the state is [`AuthorisationState::Revoked`],
/// otherwise [`AuthorisationState::Live`]. Returns `None` when no
/// valid version of the authorisation exists.
#[must_use]
pub fn fold_authorisation(
    authorisation_id: u128,
    authorisations: &[(IssuerId, RateAuthorisation)],
    revokes: &[(IssuerId, RateAuthorisationRevoke)],
) -> Option<AuthorisationState> {
    let live = authorisations
        .iter()
        .filter(|(_, body)| body.authorisation_id == authorisation_id)
        .filter(|(author, body)| body.validate(author).is_ok())
        .max_by_key(|(_, body)| body.version)?;
    let issuer = live.1.issuer_id;
    let covering = revokes
        .iter()
        .filter(|(author, _)| *author == issuer)
        .filter(|(_, body)| body.authorisation_id == authorisation_id)
        .filter(|(_, body)| body.revoked_version_from <= live.1.version)
        .min_by_key(|(_, body)| body.revoked_version_from);
    match covering {
        Some((_, revoke)) => Some(AuthorisationState::Revoked {
            authorisation: live.1.clone(),
            revoked_version_from: revoke.revoked_version_from,
            effective_at: revoke.effective_at,
        }),
        None => Some(AuthorisationState::Live(live.1.clone())),
    }
}

/// Signs a rate authorisation as a `currency.rate_authorisation` event.
///
/// # Errors
///
/// Returns a chain error if the extension or backlink is invalid, or a
/// JSON error if the authorisation cannot be serialized.
pub fn sign_authorisation(
    key: &SigningKey,
    seq_num: SeqNum,
    backlink: Option<Hash>,
    authorisation: &RateAuthorisation,
    after: Vec<Hash>,
) -> Result<Event, SignError> {
    let body = authorisation.body()?;
    Ok(chain::sign(
        key,
        seq_num,
        backlink,
        Some(&body),
        ChainExt {
            kind: RATE_AUTHORISATION_KIND.to_owned(),
            group: None,
            after,
            tag: None,
            document: None,
            removes: None,
        },
    )?)
}

/// Signs a revocation as a `currency.rate_authorisation_revoke` event.
///
/// # Errors
///
/// As [`sign_authorisation`].
pub fn sign_revoke(
    key: &SigningKey,
    seq_num: SeqNum,
    backlink: Option<Hash>,
    revoke: &RateAuthorisationRevoke,
    after: Vec<Hash>,
) -> Result<Event, SignError> {
    let body = revoke.body()?;
    Ok(chain::sign(
        key,
        seq_num,
        backlink,
        Some(&body),
        ChainExt {
            kind: RATE_AUTHORISATION_REVOKE_KIND.to_owned(),
            group: None,
            after,
            tag: None,
            document: None,
            removes: None,
        },
    )?)
}

/// A signing failure: the object would not serialize, or the chain
/// refused the event.
#[derive(Debug)]
pub enum SignError {
    /// The object would not serialize to its canonical body.
    Json(serde_json::Error),
    /// The chain refused the event (extension or backlink rules).
    Chain(chain::ChainError),
}

impl From<serde_json::Error> for SignError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}

impl From<chain::ChainError> for SignError {
    fn from(err: chain::ChainError) -> Self {
        Self::Chain(err)
    }
}

impl std::fmt::Display for SignError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(err) => write!(f, "currency object would not serialize: {err}"),
            Self::Chain(err) => write!(f, "chain refused the currency event: {err:?}"),
        }
    }
}

impl std::error::Error for SignError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn lp_key() -> SigningKey {
        SigningKey::from_bytes(&[7; 32])
    }

    fn other_key() -> SigningKey {
        SigningKey::from_bytes(&[9; 32])
    }

    /// The #2290 worked example, forward direction: LP offers
    /// ABC→123 at 0.8123 (8123/10000).
    fn forward() -> RateAuthorisation {
        RateAuthorisation {
            authorisation_id: 7,
            version: 1,
            issuer_id: issuer_id(&lp_key()),
            source_ledger: "ABC".to_owned(),
            dest_ledger: "123".to_owned(),
            rate_num: 8123,
            rate_den: 10_000,
            min_amount: 1,
            max_amount: 10_000,
            total_cap: 1_000_000,
            authorised_callers: AuthorisedCallers::Any,
            not_before: 0,
            expires_at: 4_102_444_800,
            reversal_window_secs: None,
            fee_bps: None,
        }
    }

    /// The #2290 worked example, reverse direction: a separate object
    /// by the ABC-side LP, 123→ABC at 1.20 (120/100).
    fn reverse() -> RateAuthorisation {
        RateAuthorisation {
            authorisation_id: 8,
            version: 1,
            issuer_id: issuer_id(&other_key()),
            source_ledger: "123".to_owned(),
            dest_ledger: "ABC".to_owned(),
            rate_num: 120,
            rate_den: 100,
            min_amount: 1,
            max_amount: 10_000,
            total_cap: 500_000,
            authorised_callers: AuthorisedCallers::Any,
            not_before: 0,
            expires_at: 4_102_444_800,
            reversal_window_secs: Some(3600),
            fee_bps: Some(25),
        }
    }

    #[test]
    fn golden_forward_rate_applies_floor() {
        let auth = forward();
        // 100 ABC at 0.8123 yields floor(100 · 8123 / 10000) = 81
        // whole dest units before scale normalisation (#2290 record).
        assert_eq!(auth.apply_rate(100), Some(81));
        assert_eq!(auth.apply_rate(10_000), Some(8123));
    }

    #[test]
    fn golden_reverse_is_a_separate_object() {
        let fwd = forward();
        let rev = reverse();
        assert_eq!(rev.apply_rate(100), Some(120));
        // Neither object implies the other: folding one id never sees
        // the other direction's object.
        let objects = [
            (issuer_id(&lp_key()), fwd.clone()),
            (issuer_id(&other_key()), rev.clone()),
        ];
        assert_eq!(
            fold_authorisation(7, &objects, &[]),
            Some(AuthorisationState::Live(fwd))
        );
        assert_eq!(
            fold_authorisation(8, &objects, &[]),
            Some(AuthorisationState::Live(rev))
        );
    }

    #[test]
    fn body_naming_another_issuer_is_invalid() {
        let auth = forward();
        let stranger = issuer_id(&other_key());
        assert_eq!(auth.validate(&stranger), Err(ValidateError::IssuerMismatch));
        // And the fold ignores a mismatched body entirely.
        assert_eq!(fold_authorisation(7, &[(stranger, auth)], &[]), None);
    }

    #[test]
    fn superseded_version_does_not_fold_as_live() {
        let v1 = forward();
        let mut v2 = v1.clone();
        v2.version = 2;
        v2.rate_num = 8000;
        let author = issuer_id(&lp_key());
        let objects = [(author, v1), (author, v2.clone())];
        assert_eq!(
            fold_authorisation(7, &objects, &[]),
            Some(AuthorisationState::Live(v2))
        );
    }

    #[test]
    fn revoked_authorisation_folds_as_revoked() {
        let v1 = forward();
        let mut v2 = v1.clone();
        v2.version = 2;
        let author = issuer_id(&lp_key());
        let objects = [(author, v1), (author, v2.clone())];
        let revoke = RateAuthorisationRevoke {
            authorisation_id: 7,
            revoked_version_from: 2,
            effective_at: 1_700_000_000,
            reason: None,
        };
        assert_eq!(
            fold_authorisation(7, &objects, &[(author, revoke)]),
            Some(AuthorisationState::Revoked {
                authorisation: v2,
                revoked_version_from: 2,
                effective_at: 1_700_000_000,
            })
        );
    }

    #[test]
    fn revoke_from_a_stranger_does_not_revoke() {
        let auth = forward();
        let revoke = RateAuthorisationRevoke {
            authorisation_id: 7,
            revoked_version_from: 1,
            effective_at: 1_700_000_000,
            reason: None,
        };
        assert_eq!(
            fold_authorisation(
                7,
                &[(issuer_id(&lp_key()), auth.clone())],
                &[(issuer_id(&other_key()), revoke)],
            ),
            Some(AuthorisationState::Live(auth))
        );
    }

    #[test]
    fn signed_events_round_trip_through_the_fold() {
        let key = lp_key();
        let auth = forward();
        let event = sign_authorisation(&key, 0, None, &auth, vec![]).expect("sign authorisation");
        let decoded = chain::decode(&event.bytes).expect("decode authorisation");
        assert_eq!(author_id(&decoded), issuer_id(&key));
        let body =
            RateAuthorisation::from_body(&auth.body().expect("body bytes")).expect("decode body");
        assert_eq!(
            fold_authorisation(7, &[(author_id(&decoded), body)], &[]),
            Some(AuthorisationState::Live(auth))
        );
    }
}
