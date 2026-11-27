//! Moderation and deletion state machine (A-GRAPH-9130-01, #10238).
//!
//! This module keeps author/group requests, moderation workflow, projection
//! hiding and server-local true deletion distinct. Each accepted transition
//! appends signed evidence; a batch is atomic and idempotent. The state is
//! deliberately local: applying an action to one replica never mutates the
//! signed evidence or serving state of another replica.
//!
//! The #14232 round-trip layer adds a reason-carrying request envelope
//! delivered to the relevant authority holders (tag author, group admins,
//! server owners) and Grant/Decline transitions: a grant produces a true
//! delete under the server's policy path with its own evidence action, a
//! decline is recorded in evidence and state with no delete.

use std::collections::BTreeSet;
use std::fmt;

use p2panda_core::{Signature, SigningKey, VerifyingKey};

use crate::canonical_cbor::{Value, encode_canonical};

/// Signing domain for deletion evidence.
pub const DELETION_DOMAIN: &str = "id.graph-deletion/v1";

/// A stable object identity supplied by the graph/content layer.
pub type ObjectId = [u8; 32];

/// The state visible to this server's projection/storage policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeletionState {
    /// The object has no local deletion or hide state.
    Available,
    /// An author/group request is retained as evidence and awaits workflow.
    DeleteRequested { request_id: u64 },
    /// The request is in the bounded moderation queue.
    ModerationQueued { request_id: u64, ticket_id: u64 },
    /// Serving is hidden locally while evidence and bytes remain.
    HiddenInProjection {
        prior: Box<DeletionState>,
        ticket_id: u64,
    },
    /// A delete request was declined by an authority holder; the decline
    /// is recorded and the object was NOT deleted (#14232).
    DeleteDeclined { request_id: u64 },
    /// The server's controlled bytes were physically deleted under policy.
    TrueDeletedOnThisServer { receipt_id: u64 },
}

/// One signed append-only state transition/evidence record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeletionEvidence {
    /// Monotonic local evidence sequence.
    pub sequence: u64,
    /// Action token covered by the signature.
    pub action: String,
    /// Target object covered by the signature.
    pub object: ObjectId,
    /// Actor or policy principal.
    pub actor: String,
    /// Request/ticket identity, when applicable.
    pub ticket_id: u64,
    /// Policy or human-readable rationale.
    pub rationale: String,
    /// Signer public key.
    pub signer: [u8; 32],
    /// Ed25519 signature over the canonical evidence fields.
    pub signature: [u8; 64],
}

impl DeletionEvidence {
    /// Verify this evidence independently of the state machine.
    pub fn verify(&self) -> Result<(), DeletionError> {
        let key = VerifyingKey::from_bytes(&self.signer).map_err(|_| DeletionError::BadKey)?;
        if !key.verify(
            &preimage(
                self.sequence,
                &self.action,
                &self.object,
                &self.actor,
                self.ticket_id,
                &self.rationale,
                &self.signer,
            )?,
            &Signature::from_bytes(&self.signature),
        ) {
            return Err(DeletionError::BadSignature);
        }
        Ok(())
    }
}

/// A reason-carrying delete request as delivered to authority holders
/// (#14232). The envelope is the delivery record: who the request was
/// routed to (tag author, group admins, server owners — as actor
/// labels) and the requester's reason, retained alongside the signed
/// evidence so a later grant or decline can be checked against it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteRequestEnvelope {
    /// The request this envelope carries.
    pub request_id: u64,
    /// The requester's reason, carried from the request path.
    pub reason: String,
    /// Authority holders the request was delivered to.
    pub delivered_to: Vec<String>,
}

/// A requested state-machine action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeletionAction {
    /// Author/group request for the next bounded batch.
    Request { request_id: u64 },
    /// Reason-carrying request delivered to authority holders (#14232).
    RequestReasoned {
        request_id: u64,
        reason: String,
        delivered_to: Vec<String>,
    },
    /// An authority holder grants the request: a true delete under the
    /// server's policy path, distinct from an admin delete (#14232).
    Grant { request_id: u64, policy_id: String },
    /// An authority holder declines the request: recorded, no delete
    /// (#14232).
    Decline { request_id: u64, reason: String },
    /// Put a request into the moderation queue.
    Queue { request_id: u64, ticket_id: u64 },
    /// Hide serving in a local projection.
    Hide { ticket_id: u64 },
    /// Clear a local projection hide without erasing evidence.
    ClearHide { ticket_id: u64 },
    /// Server-policy true delete; reference override is explicit.
    AdminTrueDelete {
        policy_id: String,
        references_override: bool,
    },
}

/// Result of one action or batch application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionOutcome {
    /// The requested transition changed local state.
    Applied,
    /// The action was already represented by the current state.
    Idempotent,
    /// References or the retention horizon deferred collection.
    Deferred,
    /// A previously applied batch was replayed.
    BatchAlreadyApplied,
}

/// Why an action was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeletionError {
    /// A required label or identifier is malformed.
    Invalid(&'static str),
    /// The action does not fit the current local state.
    InvalidTransition(&'static str),
    /// The object is permanently deleted on this server.
    AlreadyDeleted,
    /// The signer key cannot be parsed.
    BadKey,
    /// Evidence signature verification failed.
    BadSignature,
    /// The batch contains no actions.
    EmptyBatch,
}

impl fmt::Display for DeletionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(problem) => f.write_str(problem),
            Self::InvalidTransition(problem) => f.write_str(problem),
            Self::AlreadyDeleted => f.write_str("object is true-deleted on this server"),
            Self::BadKey => f.write_str("deletion signer key is invalid"),
            Self::BadSignature => f.write_str("deletion evidence signature is invalid"),
            Self::EmptyBatch => f.write_str("deletion batch must not be empty"),
        }
    }
}

impl std::error::Error for DeletionError {}

/// Local state and append-only evidence for one object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeletionStateMachine {
    object: ObjectId,
    state: DeletionState,
    evidence: Vec<DeletionEvidence>,
    deliveries: Vec<DeleteRequestEnvelope>,
    applied_batches: BTreeSet<u64>,
    next_sequence: u64,
}

impl DeletionStateMachine {
    /// Start an available local projection for `object`.
    #[must_use]
    pub const fn new(object: ObjectId) -> Self {
        Self {
            object,
            state: DeletionState::Available,
            evidence: Vec::new(),
            deliveries: Vec::new(),
            applied_batches: BTreeSet::new(),
            next_sequence: 0,
        }
    }

    /// The object identity.
    #[must_use]
    pub const fn object(&self) -> ObjectId {
        self.object
    }

    /// Current local state.
    #[must_use]
    pub const fn state(&self) -> &DeletionState {
        &self.state
    }

    /// All retained evidence, including deferred decisions.
    #[must_use]
    pub fn evidence(&self) -> &[DeletionEvidence] {
        &self.evidence
    }

    /// All reason-carrying request envelopes delivered so far (#14232).
    #[must_use]
    pub fn deliveries(&self) -> &[DeleteRequestEnvelope] {
        &self.deliveries
    }

    /// Apply one action. Repeating a state-equivalent action is idempotent;
    /// true deletion never reverses.
    pub fn apply(
        &mut self,
        actor: &str,
        action: DeletionAction,
        key: &SigningKey,
    ) -> Result<TransitionOutcome, DeletionError> {
        self.validate_actor(actor)?;
        let ticket_id = action_ticket(&action);
        match action {
            DeletionAction::Request { request_id } => match self.state {
                DeletionState::Available => {
                    self.state = DeletionState::DeleteRequested { request_id };
                    self.append(
                        "delete_requested",
                        actor,
                        request_id,
                        "author/group request",
                        key,
                    )?;
                    Ok(TransitionOutcome::Applied)
                }
                DeletionState::DeleteRequested {
                    request_id: current,
                } if current == request_id => Ok(TransitionOutcome::Idempotent),
                DeletionState::TrueDeletedOnThisServer { .. } => Err(DeletionError::AlreadyDeleted),
                _ => Err(DeletionError::InvalidTransition(
                    "delete request requires an available object",
                )),
            },
            DeletionAction::RequestReasoned {
                request_id,
                reason,
                delivered_to,
            } => {
                checked_label(&reason)?;
                if delivered_to.is_empty() {
                    return Err(DeletionError::Invalid(
                        "a delete request must be delivered to at least one authority holder",
                    ));
                }
                for holder in &delivered_to {
                    checked_label(holder)?;
                }
                match self.state {
                    DeletionState::Available | DeletionState::DeleteDeclined { .. } => {
                        self.state = DeletionState::DeleteRequested { request_id };
                        self.deliveries.push(DeleteRequestEnvelope {
                            request_id,
                            reason: reason.clone(),
                            delivered_to,
                        });
                        self.append("delete_requested", actor, request_id, &reason, key)?;
                        Ok(TransitionOutcome::Applied)
                    }
                    DeletionState::DeleteRequested {
                        request_id: current,
                    } if current == request_id => Ok(TransitionOutcome::Idempotent),
                    DeletionState::TrueDeletedOnThisServer { .. } => {
                        Err(DeletionError::AlreadyDeleted)
                    }
                    _ => Err(DeletionError::InvalidTransition(
                        "reasoned delete request requires an available object",
                    )),
                }
            }
            DeletionAction::Grant {
                request_id,
                policy_id,
            } => {
                checked_label(&policy_id)?;
                match self.state {
                    DeletionState::DeleteRequested {
                        request_id: current,
                    }
                    | DeletionState::ModerationQueued {
                        request_id: current,
                        ..
                    } if current == request_id => {
                        self.state = DeletionState::TrueDeletedOnThisServer {
                            receipt_id: self.next_sequence + 1,
                        };
                        self.append(
                            "delete_granted",
                            actor,
                            request_id,
                            &format!("policy={policy_id};request_id={request_id}"),
                            key,
                        )?;
                        Ok(TransitionOutcome::Applied)
                    }
                    DeletionState::TrueDeletedOnThisServer { .. } => {
                        Ok(TransitionOutcome::Idempotent)
                    }
                    _ => Err(DeletionError::InvalidTransition(
                        "grant requires the matching outstanding delete request",
                    )),
                }
            }
            DeletionAction::Decline { request_id, reason } => {
                checked_label(&reason)?;
                match self.state {
                    DeletionState::DeleteRequested {
                        request_id: current,
                    }
                    | DeletionState::ModerationQueued {
                        request_id: current,
                        ..
                    } if current == request_id => {
                        self.state = DeletionState::DeleteDeclined { request_id };
                        self.append("delete_declined", actor, request_id, &reason, key)?;
                        Ok(TransitionOutcome::Applied)
                    }
                    DeletionState::DeleteDeclined {
                        request_id: current,
                    } if current == request_id => Ok(TransitionOutcome::Idempotent),
                    DeletionState::TrueDeletedOnThisServer { .. } => {
                        Err(DeletionError::AlreadyDeleted)
                    }
                    _ => Err(DeletionError::InvalidTransition(
                        "decline requires the matching outstanding delete request",
                    )),
                }
            }
            DeletionAction::Queue {
                request_id,
                ticket_id,
            } => match self.state {
                DeletionState::DeleteRequested {
                    request_id: current,
                } if current == request_id => {
                    self.state = DeletionState::ModerationQueued {
                        request_id,
                        ticket_id,
                    };
                    self.append(
                        "moderation_queued",
                        actor,
                        ticket_id,
                        "bounded moderation queue",
                        key,
                    )?;
                    Ok(TransitionOutcome::Applied)
                }
                DeletionState::ModerationQueued {
                    request_id: current_request,
                    ticket_id: current_ticket,
                } if current_request == request_id && current_ticket == ticket_id => {
                    Ok(TransitionOutcome::Idempotent)
                }
                DeletionState::TrueDeletedOnThisServer { .. } => Err(DeletionError::AlreadyDeleted),
                _ => Err(DeletionError::InvalidTransition(
                    "moderation queue requires the matching delete request",
                )),
            },
            DeletionAction::Hide { ticket_id } => {
                if matches!(self.state, DeletionState::TrueDeletedOnThisServer { .. }) {
                    return Err(DeletionError::AlreadyDeleted);
                }
                if matches!(self.state, DeletionState::HiddenInProjection { ticket_id: current, .. } if current == ticket_id)
                {
                    return Ok(TransitionOutcome::Idempotent);
                }
                let prior = self.state.clone();
                self.state = DeletionState::HiddenInProjection {
                    prior: Box::new(prior),
                    ticket_id,
                };
                self.append(
                    "hidden_in_projection",
                    actor,
                    ticket_id,
                    "moderation projection",
                    key,
                )?;
                Ok(TransitionOutcome::Applied)
            }
            DeletionAction::ClearHide { ticket_id } => match self.state.clone() {
                DeletionState::HiddenInProjection {
                    prior,
                    ticket_id: current,
                } if current == ticket_id => {
                    self.state = *prior;
                    self.append(
                        "hide_cleared",
                        actor,
                        ticket_id,
                        "projection review clear",
                        key,
                    )?;
                    Ok(TransitionOutcome::Applied)
                }
                DeletionState::TrueDeletedOnThisServer { .. } => Err(DeletionError::AlreadyDeleted),
                _ => Ok(TransitionOutcome::Idempotent),
            },
            DeletionAction::AdminTrueDelete {
                policy_id,
                references_override,
            } => {
                checked_label(&policy_id)?;
                if matches!(self.state, DeletionState::TrueDeletedOnThisServer { .. }) {
                    return Ok(TransitionOutcome::Idempotent);
                }
                self.state = DeletionState::TrueDeletedOnThisServer {
                    receipt_id: self.next_sequence + 1,
                };
                self.append(
                    "true_deleted_on_this_server",
                    actor,
                    ticket_id,
                    &format!("policy={policy_id};references_override={references_override}"),
                    key,
                )?;
                Ok(TransitionOutcome::Applied)
            }
        }
    }

    /// Apply a batch atomically. Replaying the same batch ID changes nothing.
    pub fn apply_batch(
        &mut self,
        batch_id: u64,
        actor: &str,
        actions: &[DeletionAction],
        key: &SigningKey,
    ) -> Result<TransitionOutcome, DeletionError> {
        if actions.is_empty() {
            return Err(DeletionError::EmptyBatch);
        }
        if self.applied_batches.contains(&batch_id) {
            return Ok(TransitionOutcome::BatchAlreadyApplied);
        }
        let mut next = self.clone();
        for action in actions {
            next.apply(actor, action.clone(), key)?;
        }
        next.applied_batches.insert(batch_id);
        *self = next;
        Ok(TransitionOutcome::Applied)
    }

    /// Run the ordinary local GC rule. References or an unfinished
    /// retention/replication horizon defer collection; only an explicit
    /// admin override may bypass that rule through [`DeletionAction`].
    pub fn gc_sweep(
        &mut self,
        actor: &str,
        policy_id: &str,
        references_remain: bool,
        horizon_elapsed: bool,
        key: &SigningKey,
    ) -> Result<TransitionOutcome, DeletionError> {
        self.validate_actor(actor)?;
        checked_label(policy_id)?;
        if matches!(self.state, DeletionState::TrueDeletedOnThisServer { .. }) {
            return Ok(TransitionOutcome::Idempotent);
        }
        if references_remain || !horizon_elapsed {
            self.append(
                "gc_deferred",
                actor,
                0,
                &format!(
                    "policy={policy_id};references_remain={references_remain};horizon_elapsed={horizon_elapsed}"
                ),
                key,
            )?;
            return Ok(TransitionOutcome::Deferred);
        }
        self.state = DeletionState::TrueDeletedOnThisServer {
            receipt_id: self.next_sequence + 1,
        };
        self.append(
            "true_deleted_on_this_server",
            actor,
            0,
            &format!("policy={policy_id};gc=true"),
            key,
        )?;
        Ok(TransitionOutcome::Applied)
    }

    fn validate_actor(&self, actor: &str) -> Result<(), DeletionError> {
        checked_label(actor)
    }

    fn append(
        &mut self,
        action: &str,
        actor: &str,
        ticket_id: u64,
        rationale: &str,
        key: &SigningKey,
    ) -> Result<(), DeletionError> {
        self.next_sequence += 1;
        let signer = *key.verifying_key().as_bytes();
        let signature = key
            .sign(&preimage(
                self.next_sequence,
                action,
                &self.object,
                actor,
                ticket_id,
                rationale,
                &signer,
            )?)
            .to_bytes();
        self.evidence.push(DeletionEvidence {
            sequence: self.next_sequence,
            action: action.to_owned(),
            object: self.object,
            actor: actor.to_owned(),
            ticket_id,
            rationale: rationale.to_owned(),
            signer,
            signature,
        });
        Ok(())
    }
}

fn action_ticket(action: &DeletionAction) -> u64 {
    match action {
        DeletionAction::Request { request_id }
        | DeletionAction::RequestReasoned { request_id, .. }
        | DeletionAction::Grant { request_id, .. }
        | DeletionAction::Decline { request_id, .. } => *request_id,
        DeletionAction::Queue { ticket_id, .. }
        | DeletionAction::Hide { ticket_id }
        | DeletionAction::ClearHide { ticket_id } => *ticket_id,
        DeletionAction::AdminTrueDelete { .. } => 0,
    }
}

fn checked_label(label: &str) -> Result<(), DeletionError> {
    if label.is_empty() || label.len() > 256 || label.chars().any(char::is_control) {
        Err(DeletionError::Invalid(
            "actor, policy and rationale labels are invalid",
        ))
    } else {
        Ok(())
    }
}

fn preimage(
    sequence: u64,
    action: &str,
    object: &ObjectId,
    actor: &str,
    ticket_id: u64,
    rationale: &str,
    signer: &[u8; 32],
) -> Result<Vec<u8>, DeletionError> {
    let value = Value::Map(vec![
        (
            Value::Text("action".to_owned()),
            Value::Text(action.to_owned()),
        ),
        (
            Value::Text("actor".to_owned()),
            Value::Text(actor.to_owned()),
        ),
        (
            Value::Text("object".to_owned()),
            Value::Bytes(object.to_vec()),
        ),
        (
            Value::Text("rationale".to_owned()),
            Value::Text(rationale.to_owned()),
        ),
        (Value::Text("sequence".to_owned()), Value::UInt(sequence)),
        (
            Value::Text("signer".to_owned()),
            Value::Bytes(signer.to_vec()),
        ),
        (Value::Text("ticket_id".to_owned()), Value::UInt(ticket_id)),
        (Value::Text("v".to_owned()), Value::UInt(1)),
    ]);
    let mut bytes = DELETION_DOMAIN.as_bytes().to_vec();
    bytes.push(0);
    bytes.extend(
        encode_canonical(&value).map_err(|_| DeletionError::Invalid("evidence encoding failed"))?,
    );
    Ok(bytes)
}
