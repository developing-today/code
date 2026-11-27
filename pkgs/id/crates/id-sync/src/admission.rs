//! Sync-side retained-data ingress through the fixed admission pipeline
//! (issue #10192, slice 9110-06; design:
//! `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/crosscut/versioned-design.md`,
//! "Operation pipeline (mandatory ordering)").
//!
//! # Carrier contract
//!
//! An inbound sync payload travels as a [`SignedEnvelope`] whose
//! payload is one chain event's canonical bytes — exactly the bytes
//! [`chain::decode`] accepts — with domain [`SYNC_DOMAIN`] and
//! context equal to the authenticated tenant scope. The envelope
//! signer must be the event's author: the decoded event's verifying
//! key must equal the envelope body's key. That binding is what lets
//! the fixed gate judge a sync payload it cannot see inside: the
//! gate verifies the envelope's signature, signer binding, tenant
//! authorization and policy, and this adapter adds the carried
//! object's semantics as part of the same fixed gate, before any
//! hook runs.
//!
//! # Federation composition
//!
//! The carrier check consults the B02 emergency revocation registry
//! ([`RevocationRegistry::admission_decision`]): a key or peer the
//! registry blocks at admission time is a gate refusal — no hook
//! runs, no ledger entry is made. This adapter mints no second
//! replay ledger of its own: operation-idempotency is the injected
//! pipeline ledger's binding, and content-idempotency is the
//! receiving store's ingest decision (a duplicate presentation
//! commits as a duplicate, resolved by readback). The B03
//! windowed-freshness evaluation stays at the log-sync session
//! layer that owns presentation timing (P-SY-01); this adapter
//! defers to it rather than duplicating a freshness ledger.
//!
//! # Ordering
//!
//! [`SyncAdmission::admit`] runs, in the design's mandatory order:
//!
//! 1. **Carrier gate** — the envelope's domain, the payload's chain
//!    decode, the author binding, and the B02 revocation consult. A
//!    failure here is a gate refusal: no hook runs, no ledger entry
//!    is made, and nothing reaches the store.
//! 2. **Pipeline** — [`AdmissionPipeline::run`]: the fixed envelope
//!    gate, the replay ledger, deterministically ordered prepare
//!    hooks, commit-time reauthorization, the commit step, and
//!    at-least-once outbox delivery.
//! 3. **Commit** — the commit step presents the admitted payload to
//!    the receiving store's ingest path (the P-SY-01 contract: sync
//!    retains through the store's ingest, never beside it). The
//!    store's decision — accepted, voided, duplicate, waiting,
//!    fork, or refused — is a *decided* outcome of the admission,
//!    so the commit step succeeds and the pipeline records the
//!    operation committed. Only a store *error* (a redb or
//!    corruption failure, whose outcome is genuinely unknown)
//!    fails the commit step, leaving the operation resumable in
//!    the ledger.
//!
//! # What this slice does not do
//!
//! The live log-sync receiver still takes raw sync bytes: routing
//! it through this adapter needs the envelope-carrying producer
//! path, which is a follow-on slice. The policy-epoch source behind
//! the [`Reauthorizer`] seam and durable [`OperationLedger`] /
//! [`OutboxJournal`] implementations for the hosted adapters are the
//! same follow-ons named by the id-store slice. The inventory
//! (`pkgs/id/tests/operation-path-inventory.md`, P-SY-01) records
//! the pipeline-backed entry point and keeps the row `partial`
//! until the receiver migrates.
//!
//! [`SignedEnvelope`]: id_core::signed_envelope::Envelope
//! [`chain::decode`]: id_store::chain::decode
//! [`OperationLedger`]: id_store::admission::OperationLedger
//! [`OutboxJournal`]: id_store::admission::OutboxJournal
//! [`Reauthorizer`]: id_store::admission::Reauthorizer

use std::sync::{Arc, Mutex};

use id_core::admission_gate::{AdmissionContext, AdmissionReceipt, AdmissionRejection};
use id_core::signed_envelope::{Envelope, EnvelopeError};
use id_events::event_store::{EventStore, Ingest};
use id_store::admission::{
    AdmissionPipeline, Annotation, HookId, LifecycleRejection, OperationId, PipelineOutcome,
    QueuedOperation,
};
use id_store::chain::{self, Event};

use crate::federation::slices::b02_key_revocation::{Decision, DenyReason, RevocationRegistry};

/// Signing domain of the sync carrier envelope: the payload of an
/// envelope in this domain is one sync event's canonical bytes.
pub const SYNC_DOMAIN: &str = "id/sync";

/// The id-sync retained-data ingress path, routed through the fixed
/// admission pipeline. The pipeline (gate, reauthorizer, ledger,
/// journal, hooks) is supplied by the caller, already built, and the
/// B02 revocation registry is shared with the federation emergency
/// path; this adapter owns only the carrier contract — envelope
/// domain, payload decode, author binding, revocation consult — and
/// the commit step that presents the admitted event to the
/// receiving store.
pub struct SyncAdmission {
    pipeline: AdmissionPipeline,
    revocations: Arc<Mutex<RevocationRegistry>>,
}

impl SyncAdmission {
    /// An admission adapter over a ready pipeline and the shared B02
    /// revocation registry. The pipeline's gate policy must admit
    /// [`SYNC_DOMAIN`]; the adapter refuses any other domain as part
    /// of its carrier check.
    #[must_use]
    pub fn new(pipeline: AdmissionPipeline, revocations: Arc<Mutex<RevocationRegistry>>) -> Self {
        Self {
            pipeline,
            revocations,
        }
    }

    /// The pipeline this adapter runs operations through, for hook
    /// registration and outbox retry.
    #[must_use]
    pub fn pipeline(&self) -> &AdmissionPipeline {
        &self.pipeline
    }

    /// Admits one inbound sync event through the fixed pipeline and
    /// presents it to `store`.
    ///
    /// `operation_id` binds the caller, tenant, peer and envelope
    /// together for replay/idempotency; `envelope` carries the sync
    /// bytes under the carrier contract; `context` is the
    /// authenticated tenant/scope context whose principal is the
    /// presenting peer for the B02 consult; `now_ms` is the store's
    /// clock and the revocation instant. The store's decision is
    /// returned in the outcome; a duplicate or waiting event is a
    /// decided outcome, not a commit failure.
    ///
    /// # Errors
    ///
    /// [`LifecycleRejection`] naming the stage that refused: the
    /// carrier gate (an envelope that is not a sync carrier, a
    /// payload that is not a chain event, an envelope signer who is
    /// not the event author, or a key/peer the B02 registry blocks),
    /// the pipeline gate, replay, prepare (veto or contract),
    /// reauthorization, or a commit step the store failed with an
    /// unknown outcome.
    pub fn admit(
        &self,
        store: &mut EventStore,
        operation_id: OperationId,
        envelope: &Envelope,
        context: &AdmissionContext,
        now_ms: u64,
    ) -> Result<SyncAdmissionOutcome, LifecycleRejection> {
        // Carrier gate, part of the fixed gate: the carried
        // object's semantics, checked before any hook runs.
        let revocations = self
            .revocations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        check_carrier(envelope, context, now_ms, &revocations)?;
        drop(revocations);

        let mut ingest_slot: Option<Ingest> = None;
        let outcome = self
            .pipeline
            .run(operation_id, envelope, context, |_commit| {
                match store.ingest(&envelope.payload, now_ms) {
                    Ok(ingest) => {
                        ingest_slot = Some(ingest);
                        Ok(())
                    }
                    Err(err) => Err(format!("receiving store ingest failed: {err}")),
                }
            })?;
        match outcome {
            PipelineOutcome::Committed {
                receipt,
                annotations,
                pending_outbox,
                follow_ups,
                policy_epoch,
            } => match ingest_slot {
                Some(ingest) => Ok(SyncAdmissionOutcome::Committed {
                    receipt,
                    annotations,
                    pending_outbox,
                    follow_ups,
                    policy_epoch,
                    ingest,
                }),
                None => Err(LifecycleRejection::CommitFailed {
                    problem: "the commit step did not record its ingest outcome".to_owned(),
                }),
            },
            PipelineOutcome::AlreadyCommitted { receipt } => {
                Ok(SyncAdmissionOutcome::AlreadyCommitted { receipt })
            }
        }
    }
}

/// The outcome of one pipeline-backed sync admission.
#[derive(Debug)]
pub enum SyncAdmissionOutcome {
    /// The operation committed in this run; `ingest` is the
    /// receiving store's decided outcome for the event (accepted,
    /// voided, duplicate, waiting, fork, or refused).
    Committed {
        /// The gate receipt.
        receipt: AdmissionReceipt,
        /// The prepare phase's annotations, in hook order.
        annotations: Vec<Annotation>,
        /// Outbox hooks whose delivery is pending (recorded in
        /// the journal for retry; the commit stands).
        pending_outbox: Vec<HookId>,
        /// Follow-up operations hooks requested during this run.
        follow_ups: Vec<QueuedOperation>,
        /// The policy epoch reauthorization judged at commit.
        policy_epoch: u64,
        /// What the receiving store decided for the admitted event.
        ingest: Ingest,
    },
    /// The operation was already committed by an earlier run;
    /// the caller resolves by readback. No hook ran and the store
    /// was not re-ingested.
    AlreadyCommitted {
        /// The original admission's receipt.
        receipt: AdmissionReceipt,
    },
}

/// The carrier check, part of the fixed gate: the envelope must be
/// a sync carrier, its payload must decode as a chain event, the
/// envelope signer must be that event's author, and neither the
/// envelope key nor the presenting peer may be blocked by the B02
/// revocation registry at admission time. Failures are gate
/// refusals, so no hook ever sees a carried object that fails them.
fn check_carrier(
    envelope: &Envelope,
    context: &AdmissionContext,
    now_ms: u64,
    revocations: &RevocationRegistry,
) -> Result<Event, LifecycleRejection> {
    if envelope.body.domain != SYNC_DOMAIN {
        return Err(LifecycleRejection::Gate(AdmissionRejection::Malformed(
            EnvelopeError::Field {
                field: "domain",
                problem: "is not the sync carrier domain",
            },
        )));
    }
    let event = chain::decode(&envelope.payload).map_err(|_| {
        LifecycleRejection::Gate(AdmissionRejection::Malformed(EnvelopeError::Field {
            field: "payload",
            problem: "is not a decodable chain event",
        }))
    })?;
    if *event.header.verifying_key.as_bytes() != envelope.body.key {
        return Err(LifecycleRejection::Gate(
            AdmissionRejection::SignerBinding {
                signer_id: envelope.body.signer_id.clone(),
                problem: "the envelope signer is not the event author",
            },
        ));
    }
    match revocations.admission_decision(&envelope.body.key, context.principal(), now_ms) {
        Decision::Allow => {}
        Decision::Deny(DenyReason::RevokedKey) => {
            return Err(LifecycleRejection::Gate(AdmissionRejection::Policy {
                problem: "the envelope key is revoked at admission time".to_owned(),
            }));
        }
        Decision::Deny(DenyReason::DisabledPeer) => {
            return Err(LifecycleRejection::Gate(AdmissionRejection::Policy {
                problem: "the presenting peer is disabled at admission time".to_owned(),
            }));
        }
    }
    Ok(event)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use id_core::id_shape::HashProfile;
    use id_core::signed_envelope::{Body, PayloadId, SUITE_ED25519};
    use id_store::chain::{ChainExt, sign};
    use p2panda_core::SigningKey;

    use crate::federation::slices::b02_key_revocation::{
        RevocationReason, RevocationRecord, RevocationSubject,
    };

    const TENANT: &str = "tenant-a";

    fn chain_event(seed: u8) -> Event {
        let key = SigningKey::from_bytes(&[seed; 32]);
        let ext = ChainExt {
            kind: "message".to_owned(),
            group: None,
            after: Vec::new(),
            tag: None,
            document: None,
            removes: None,
            note: None,
            link: None,
            checkpoint: None,
            rotation: None,
            chat: None,
            anchor: None,
            thread: None,
        };
        sign(&key, 0, None, None, ext).unwrap()
    }

    fn carrier_envelope(event: &Event, domain: &str, key: [u8; 32]) -> Envelope {
        let body = Body {
            domain: domain.to_string(),
            context: TENANT.to_string(),
            subject: None,
            graph: None,
            predecessor: None,
            signer_id: "alice".to_string(),
            key,
            suite: SUITE_ED25519.to_string(),
            payload_id: PayloadId::of(HashProfile::RawBlake3_256, &event.bytes).unwrap(),
            payload_length: event.bytes.len() as u64,
        };
        Envelope {
            body,
            payload: event.bytes.clone(),
            signature: [0_u8; 64],
        }
    }

    fn context() -> AdmissionContext {
        AdmissionContext::new("alice", TENANT, "admission:write").unwrap()
    }

    fn empty_revocations() -> RevocationRegistry {
        RevocationRegistry::new()
    }

    fn revoked_key_registry(key: [u8; 32]) -> RevocationRegistry {
        let mut registry = RevocationRegistry::new();
        registry
            .apply_revocation(RevocationRecord {
                record_id: "rev-1".to_owned(),
                subject: RevocationSubject::Key(key),
                effective_from: 0,
                effective_until: None,
                reason: RevocationReason::KeyCompromise,
                recorded_at: 0,
            })
            .unwrap();
        registry
    }

    fn disabled_peer_registry(peer: &str) -> RevocationRegistry {
        let mut registry = RevocationRegistry::new();
        registry
            .apply_revocation(RevocationRecord {
                record_id: "rev-1".to_owned(),
                subject: RevocationSubject::Peer(peer.to_owned()),
                effective_from: 0,
                effective_until: None,
                reason: RevocationReason::PeerCompromise,
                recorded_at: 0,
            })
            .unwrap();
        registry
    }

    #[test]
    fn carrier_check_accepts_a_matching_event_and_envelope() {
        let event = chain_event(11);
        let key = SigningKey::from_bytes(&[11; 32]);
        let envelope = carrier_envelope(&event, SYNC_DOMAIN, *key.verifying_key().as_bytes());
        let decoded = check_carrier(&envelope, &context(), 0, &empty_revocations()).unwrap();
        assert_eq!(decoded.id, event.id);
    }

    #[test]
    fn carrier_check_refuses_a_foreign_domain() {
        let event = chain_event(11);
        let key = SigningKey::from_bytes(&[11; 32]);
        let envelope = carrier_envelope(&event, "id/other", *key.verifying_key().as_bytes());
        let err = check_carrier(&envelope, &context(), 0, &empty_revocations()).unwrap_err();
        assert_eq!(err.stage(), "gate");
        assert_eq!(err.class(), "malformed");
    }

    #[test]
    fn carrier_check_refuses_bytes_that_are_not_a_chain_event() {
        let event = chain_event(11);
        let key = SigningKey::from_bytes(&[11; 32]);
        let mut envelope = carrier_envelope(&event, SYNC_DOMAIN, *key.verifying_key().as_bytes());
        envelope.payload = vec![9; 8];
        let err = check_carrier(&envelope, &context(), 0, &empty_revocations()).unwrap_err();
        assert_eq!(err.stage(), "gate");
        assert_eq!(err.class(), "malformed");
    }

    #[test]
    fn carrier_check_refuses_an_envelope_signer_who_is_not_the_author() {
        let event = chain_event(11);
        let other = SigningKey::from_bytes(&[22; 32]);
        let envelope = carrier_envelope(&event, SYNC_DOMAIN, *other.verifying_key().as_bytes());
        let err = check_carrier(&envelope, &context(), 0, &empty_revocations()).unwrap_err();
        assert_eq!(err.stage(), "gate");
        assert_eq!(err.class(), "signer_binding");
    }

    #[test]
    fn carrier_check_refuses_a_revoked_key_before_any_hook() {
        let event = chain_event(11);
        let key = SigningKey::from_bytes(&[11; 32]);
        let key_bytes = *key.verifying_key().as_bytes();
        let envelope = carrier_envelope(&event, SYNC_DOMAIN, key_bytes);
        let registry = revoked_key_registry(key_bytes);
        let err = check_carrier(&envelope, &context(), 0, &registry).unwrap_err();
        assert_eq!(err.stage(), "gate");
        assert_eq!(err.class(), "policy");
    }

    #[test]
    fn carrier_check_refuses_a_disabled_presenting_peer() {
        let event = chain_event(11);
        let key = SigningKey::from_bytes(&[11; 32]);
        let envelope = carrier_envelope(&event, SYNC_DOMAIN, *key.verifying_key().as_bytes());
        let registry = disabled_peer_registry("alice");
        let err = check_carrier(&envelope, &context(), 0, &registry).unwrap_err();
        assert_eq!(err.stage(), "gate");
        assert_eq!(err.class(), "policy");
    }
}
