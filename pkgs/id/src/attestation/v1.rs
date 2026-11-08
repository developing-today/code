//! Versioned v1 attestation event objects and registry counting policy.
//!
//! This module keeps the original mutual-concession PoC API stable while
//! defining the additional signed objects used by the v1 reducer.

use p2panda_core::{Hash, SeqNum, SigningKey};
use serde::{Deserialize, Serialize};

use super::{GenesisAgreement, PlayerId, SignError, player_id};
use crate::chain::{self, ChainExt, Event};

pub const VERSION: u8 = 1;
pub const WITNESS_DECISION_KIND: &str = "attestation.witness_decision";
pub const OBJECTION_KIND: &str = "attestation.objection";

/// Evidence a named witness considered when issuing a decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WitnessEvidence {
    /// Both player claims and both submitted logs were available.
    Complete {
        claim_a: Hash,
        claim_b: Hash,
        log_hash_a: Hash,
        log_hash_b: Hash,
    },
    /// One player did not submit by the pre-agreed deadline. The referenced
    /// non-receipt statement is evidence, not a substitute for the agreement's
    /// submission rule or a proof of receipt time.
    Partial {
        received: Vec<Hash>,
        missing_from: PlayerId,
        deadline_secs: u64,
        non_receipt_statement: Hash,
    },
}

/// A witness's signed decision for one agreement and pinned logic version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WitnessDecision {
    pub version: u8,
    pub agreement: Hash,
    pub logic_hash: Hash,
    pub outcome: String,
    pub evidence: WitnessEvidence,
}

impl WitnessDecision {
    pub fn body(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn from_body(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

/// A signed objection. It appends evidence and never edits the target object.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Objection {
    pub version: u8,
    pub target: Hash,
    pub grounds: String,
    pub author_frontier: Vec<Hash>,
}

impl Objection {
    pub fn body(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn from_body(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

/// Signs a witness decision, refusing witness-shopping and stale agreement
/// or logic references before creating the signed event.
pub fn sign_witness_decision(
    key: &SigningKey,
    seq_num: SeqNum,
    backlink: Option<Hash>,
    agreement: &GenesisAgreement,
    decision: &WitnessDecision,
    after: Vec<Hash>,
) -> Result<Event, V1SignError> {
    if agreement.witness != Some(player_id(key)) {
        return Err(V1SignError::NotNamedWitness);
    }
    if decision.version != VERSION || decision.agreement != agreement.agreement_hash()? {
        return Err(V1SignError::WrongAgreement);
    }
    if decision.logic_hash != agreement.logic_hash {
        return Err(V1SignError::WrongLogic);
    }
    sign_body(
        key,
        seq_num,
        backlink,
        decision.body()?,
        WITNESS_DECISION_KIND,
        after,
    )
    .map_err(V1SignError::from)
}

/// Signs an append-only objection from either seated player. Objections are
/// evaluated against the agreed policy and evidence graph by the reducer.
pub fn sign_objection(
    key: &SigningKey,
    seq_num: SeqNum,
    backlink: Option<Hash>,
    agreement: &GenesisAgreement,
    objection: &Objection,
    after: Vec<Hash>,
) -> Result<Event, V1SignError> {
    if !agreement.players.contains(&player_id(key)) {
        return Err(V1SignError::NotPlayer);
    }
    if objection.version != VERSION {
        return Err(V1SignError::WrongAgreement);
    }
    sign_body(
        key,
        seq_num,
        backlink,
        objection.body()?,
        OBJECTION_KIND,
        after,
    )
    .map_err(V1SignError::from)
}

fn sign_body(
    key: &SigningKey,
    seq_num: SeqNum,
    backlink: Option<Hash>,
    body: Vec<u8>,
    kind: &str,
    after: Vec<Hash>,
) -> Result<Event, SignError> {
    Ok(chain::sign(
        key,
        seq_num,
        backlink,
        Some(&body),
        ChainExt {
            kind: kind.to_owned(),
            group: None,
            after,
            tag: None,
            document: None,
            removes: None,
            note: None,
            link: None,
            checkpoint: None,
            rotation: None,
        },
    )?)
}

/// A reducer result. Agreed records matching player claims; the v1 reducer
/// advances that result to Final immediately unless dependent evidence is
/// invalid or an objection exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum V1MatchState {
    Proposed,
    Agreed,
    Final,
    Witnessed,
    Disputed,
    Void,
}

/// Registry-local policy: this projects signed state into a count and has no
/// authority to modify the match or its signed objects.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CountingPolicy {
    pub accepted_states: Vec<V1MatchState>,
    pub accepted_witnesses: Vec<PlayerId>,
    pub accepted_logic_hashes: Vec<Hash>,
}

impl CountingPolicy {
    #[must_use]
    pub fn counts(&self, agreement: &GenesisAgreement, state: V1MatchState) -> bool {
        self.accepted_states.contains(&state)
            && (self.accepted_logic_hashes.is_empty()
                || self.accepted_logic_hashes.contains(&agreement.logic_hash))
            && agreement
                .witness
                .map_or(true, |witness| self.accepted_witnesses.contains(&witness))
    }
}

#[derive(Debug)]
pub enum V1SignError {
    Sign(SignError),
    Json(serde_json::Error),
    NotNamedWitness,
    NotPlayer,
    WrongAgreement,
    WrongLogic,
}

impl From<SignError> for V1SignError {
    fn from(error: SignError) -> Self {
        Self::Sign(error)
    }
}

impl From<serde_json::Error> for V1SignError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl std::fmt::Display for V1SignError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sign(error) => write!(f, "{error}"),
            Self::Json(error) => write!(f, "attestation v1 object would not serialize: {error}"),
            Self::NotNamedWitness => {
                write!(f, "only the witness named in genesis may issue a decision")
            }
            Self::NotPlayer => write!(f, "only a seated player may object"),
            Self::WrongAgreement => write!(
                f,
                "attestation v1 object names the wrong agreement or version"
            ),
            Self::WrongLogic => write!(
                f,
                "witness decision names logic other than the genesis-pinned logic"
            ),
        }
    }
}

impl std::error::Error for V1SignError {}
