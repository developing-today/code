//! PoC: mutual-concession match attestation on the tic-tac-toe example
//! (#2248, from the #2037 investigation). **Proof of concept only** —
//! nothing here is a production attestation feature, and no production
//! API is built on it without the review round the epic (#2056) requires.
//!
//! The PoC composes the existing primitives — signed chained events
//! ([`crate::chain`]) and the event store — with two new object shapes,
//! carried as ordinary signed events whose bodies are the JSON objects:
//!
//! - **Genesis agreement** (kind `attestation.genesis`): one event per
//!   player, both signing the identical agreement body
//!   `{game_id, logic_hash, players, witness, objection_window_secs}`.
//!   The witness slot is present and unused in this PoC; the objection
//!   window is a parameter only (objection handling is #2038/#2039).
//! - **Result claim** (kind `attestation.claim`): one event per player
//!   at match end, `{agreement, outcome, final_state_hash, frontiers}`
//!   — the agreement hash it answers, the outcome it asserts, the hash
//!   of the final game state, and both chain frontiers it had seen.
//!
//! A **viewer** ([`compute_state`]) computes the match state from the
//! objects alone: `Proposed` until both players have signed the same
//! genesis and concurred, `Agreed` when both claims concur on outcome
//! and final-state hash, `Disputed` when both claims exist and differ
//! (the wrong-winner negative case can never compute `Agreed`).
//!
//! Reconstruction is admission, not deserialization: derive conclusions from
//! signature-verified authors and compare claims with the selected agreement.
//! Reject, ignore, or dispute inconsistent input according to the API's rules;
//! never silently repair it or let an input's own claims establish authority.
//! New fields and restore paths must be checked against verifier-owned state.

use p2panda_core::{Hash, SeqNum, SigningKey};
use serde::{Deserialize, Serialize};

use crate::chain::{self, ChainExt, Event};

/// The event kind carrying a genesis agreement body.
pub const GENESIS_KIND: &str = "attestation.genesis";

/// The event kind carrying a result claim body.
pub const CLAIM_KIND: &str = "attestation.claim";

/// A player identity: the 32 bytes of its signing key's verifying key,
/// the same bytes the event store uses as an author key.
pub type PlayerId = [u8; 32];

/// The player id of a signing key.
#[must_use]
pub fn player_id(key: &SigningKey) -> PlayerId {
    *key.verifying_key().as_bytes()
}

/// The author of a decoded (signature-verified) event, as a player id.
#[must_use]
pub fn author_id(event: &Event) -> PlayerId {
    *event.header.verifying_key.as_bytes()
}

/// A genesis agreement: both players sign this identical body, each in
/// its own `attestation.genesis` event, before the match.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenesisAgreement {
    /// The game being played (for the PoC, the tic-tac-toe example id).
    pub game_id: String,
    /// Hash of the game logic (for the PoC, of the example's `main.roc`).
    pub logic_hash: Hash,
    /// Both players, in seat order.
    pub players: [PlayerId; 2],
    /// The named witness slot. Present per #2037, unused in this PoC:
    /// no witness decision flow exists here.
    pub witness: Option<PlayerId>,
    /// The objection window parameter, in seconds. Present per #2037;
    /// objection handling itself is out of scope (#2038/#2039).
    pub objection_window_secs: u64,
}

impl GenesisAgreement {
    /// The canonical body bytes this agreement is signed over.
    ///
    /// # Errors
    ///
    /// Returns a JSON error if the agreement cannot be serialized
    /// (not expected for this shape).
    pub fn body(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// The agreement hash a result claim names to answer this genesis.
    ///
    /// # Errors
    ///
    /// As [`GenesisAgreement::body`].
    pub fn agreement_hash(&self) -> Result<Hash, serde_json::Error> {
        Ok(Hash::digest(&self.body()?))
    }

    /// Decodes an agreement from a signed event's body bytes.
    ///
    /// # Errors
    ///
    /// Returns a JSON error if the bytes are not an agreement object.
    pub fn from_body(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

/// One player's signed result claim at match end.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResultClaim {
    /// The hash of the genesis agreement this claim answers.
    pub agreement: Hash,
    /// The outcome the claimant asserts (for the PoC, the winning
    /// player's id as hex, or `"draw"`).
    pub outcome: String,
    /// Hash of the final game state the claimant replayed.
    pub final_state_hash: Hash,
    /// Both chain frontiers the claimant had seen at match end: the
    /// latest event ids of both players' chains.
    pub frontiers: Vec<Hash>,
}

impl ResultClaim {
    /// The canonical body bytes this claim is signed over.
    ///
    /// # Errors
    ///
    /// As [`GenesisAgreement::body`].
    pub fn body(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// Decodes a claim from a signed event's body bytes.
    ///
    /// # Errors
    ///
    /// Returns a JSON error if the bytes are not a claim object.
    pub fn from_body(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

/// The match state a viewer computes from the attestation objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchState {
    /// No complete genesis pair yet, or only one player's claim so far.
    Proposed,
    /// Both players signed the same genesis and their claims concur on
    /// outcome and final-state hash.
    Agreed,
    /// Both players claimed and their claims differ — never `Agreed`.
    Disputed,
}

/// Signs a genesis agreement as an `attestation.genesis` event.
///
/// # Errors
///
/// Returns a chain error if the extension or backlink is invalid, or a
/// JSON error if the agreement cannot be serialized.
pub fn sign_genesis(
    key: &SigningKey,
    seq_num: SeqNum,
    backlink: Option<Hash>,
    agreement: &GenesisAgreement,
    after: Vec<Hash>,
) -> Result<Event, SignError> {
    let body = agreement.body()?;
    Ok(chain::sign(
        key,
        seq_num,
        backlink,
        Some(&body),
        ChainExt {
            kind: GENESIS_KIND.to_owned(),
            group: None,
            after,
            tag: None,
            document: None,
            removes: None,
        },
    )?)
}

/// Signs a result claim as an `attestation.claim` event.
///
/// # Errors
///
/// As [`sign_genesis`].
pub fn sign_claim(
    key: &SigningKey,
    seq_num: SeqNum,
    backlink: Option<Hash>,
    claim: &ResultClaim,
    after: Vec<Hash>,
) -> Result<Event, SignError> {
    let body = claim.body()?;
    Ok(chain::sign(
        key,
        seq_num,
        backlink,
        Some(&body),
        ChainExt {
            kind: CLAIM_KIND.to_owned(),
            group: None,
            after,
            tag: None,
            document: None,
            removes: None,
        },
    )?)
}

/// Computes the match state from the objects alone.
///
/// `genesis` pairs each genesis event's verified author with the
/// agreement decoded from its body; `claims` pairs each claim event's
/// verified author with its decoded claim. The caller obtains authors
/// from decoded events ([`author_id`]) — [`chain::decode`] has already
/// verified every signature before this function sees an object.
///
/// Rules: without both players signing the *identical* agreement the
/// match is `Proposed`. With a complete genesis, one claim is still
/// `Proposed`; two claims (one per player) that concur on outcome and
/// final-state hash are `Agreed`; two claims that differ in either are
/// `Disputed`. A player claiming twice with different claims is a
/// fork of position and computes `Disputed`, never `Agreed`. Claims
/// answering a different agreement, or from non-players, are ignored.
#[must_use]
pub fn compute_state(
    agreement: &GenesisAgreement,
    genesis: &[(PlayerId, GenesisAgreement)],
    claims: &[(PlayerId, ResultClaim)],
) -> MatchState {
    let genesis_ok = agreement.players.iter().all(|player| {
        genesis
            .iter()
            .any(|(author, signed)| author == player && signed == agreement)
    });
    if !genesis_ok {
        return MatchState::Proposed;
    }
    let Ok(agreement_hash) = agreement.agreement_hash() else {
        return MatchState::Proposed;
    };
    let mut per_player: [Option<&ResultClaim>; 2] = [None, None];
    for (index, player) in agreement.players.iter().enumerate() {
        for (author, claim) in claims {
            if author == player && claim.agreement == agreement_hash {
                match per_player[index] {
                    None => per_player[index] = Some(claim),
                    Some(first) if first != claim => return MatchState::Disputed,
                    Some(_) => {}
                }
            }
        }
    }
    match (per_player[0], per_player[1]) {
        (Some(a), Some(b)) => {
            if a.outcome == b.outcome && a.final_state_hash == b.final_state_hash {
                MatchState::Agreed
            } else {
                MatchState::Disputed
            }
        }
        _ => MatchState::Proposed,
    }
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
            Self::Json(err) => write!(f, "attestation object would not serialize: {err}"),
            Self::Chain(err) => write!(f, "chain refused the attestation event: {err:?}"),
        }
    }
}

impl std::error::Error for SignError {}
