//! Retained-event ingress through the fixed admission pipeline
//! (issue #10192, slice 9110-06; design:
//! `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/crosscut/versioned-design.md`,
//! "Operation pipeline (mandatory ordering)").
//!
//! # Carrier contract
//!
//! A retained event travels as a [`SignedEnvelope`] whose payload is
//! the event's canonical bytes — exactly the bytes
//! [`chain::decode`] accepts — with domain [`EVENTS_DOMAIN`] and
//! context equal to the authenticated tenant scope. The envelope
//! signer must be the event's author: the decoded event's
//! verifying key must equal the envelope body's key. That binding
//! is what lets the fixed gate judge an event it cannot see
//! inside: the gate verifies the envelope's signature, signer
//! binding, tenant authorization and policy, and this adapter adds
//! the carried object's semantics (chain decode + author binding)
//! as part of the same fixed gate, before any hook runs.
//!
//! # Ordering
//!
//! [`EventsAdmission::admit`] runs, in the design's mandatory
//! order:
//!
//! 1. **Carrier gate** — the envelope's domain, the payload's
//!    chain decode, and the author binding. A failure here is a
//!    gate refusal: no hook runs, no ledger entry is made, and
//!    nothing reaches the store.
//! 2. **Pipeline** — [`AdmissionPipeline::run`]: the fixed
//!    envelope gate, the replay ledger, deterministically ordered
//!    prepare hooks, commit-time reauthorization, the commit step,
//!    and at-least-once outbox delivery.
//! 3. **Commit** — the commit step ingests the admitted payload
//!    into the [`EventStore`]. The store re-verifies the chain
//!    event itself (its fork, dependency and waiting logic are the
//!    store's own checks, unchanged by this slice) and returns its
//!    decision.
//!
//! # Decided outcomes, not unknown commits
//!
//! The store's decision — accepted, voided, duplicate, waiting,
//! fork, or refused — is a *decided* outcome of the admission, so
//! the commit step succeeds and the pipeline records the operation
//! committed, with the store's decision carried in the outcome. A
//! re-presentation of the same operation then resolves as
//! already-committed and the caller resolves by readback (a refused
//! event reads back as absent). Only a store *error* (a redb or
//! corruption failure, whose outcome is genuinely unknown) fails
//! the commit step, leaving the operation resumable in the ledger.
//!
//! # What this slice does not do
//!
//! The production callers (`EventStore::ingest` from the sync
//! receiver and the batch path) still take raw event bytes: peers
//! send raw bytes today, so routing those callers through this
//! adapter needs the envelope-carrying producer path, which is a
//! follow-on slice. Bodies travelling beside an event
//! (`ingest_with_body`) are likewise out of scope: the admitted
//! object here is the event bytes, and a body arrives through the
//! store's existing attach path. The inventory
//! (`tests/operation-path-inventory.md`, P-WR-02) records the
//! pipeline-backed entry point and keeps the row `partial` until
//! the callers migrate.
//!
//! [`SignedEnvelope`]: id_core::signed_envelope::Envelope
//! [`chain::decode`]: id_store::chain::decode

use id_core::admission_gate::{AdmissionContext, AdmissionReceipt, AdmissionRejection};
use id_core::signed_envelope::{Envelope, EnvelopeError};
use id_store::admission::{
    AdmissionPipeline, Annotation, HookId, LifecycleRejection, OperationId, PipelineOutcome,
    QueuedOperation,
};
use id_store::chain::{self, Event};

use crate::event_store::{EventStore, Ingest};

/// Signing domain of the event carrier envelope: the payload of
/// an envelope in this domain is one event's canonical bytes.
pub const EVENTS_DOMAIN: &str = "id/events";

/// The id-events retained-data ingress path, routed through the
/// fixed admission pipeline. The pipeline (gate, reauthorizer,
/// ledger, journal, hooks) is supplied by the caller, already
/// built; this adapter owns only the carrier contract — envelope
/// domain, payload decode, author binding — and the commit step
/// that ingests the admitted event into the store.
pub struct EventsAdmission {
    pipeline: AdmissionPipeline,
}

impl EventsAdmission {
    /// An admission adapter over a ready pipeline. The pipeline's
    /// gate policy must admit [`EVENTS_DOMAIN`]; the adapter
    /// refuses any other domain as part of its carrier check.
    #[must_use]
    pub fn new(pipeline: AdmissionPipeline) -> Self {
        Self { pipeline }
    }

    /// The pipeline this adapter runs operations through, for
    /// hook registration and outbox retry.
    #[must_use]
    pub fn pipeline(&self) -> &AdmissionPipeline {
        &self.pipeline
    }

    /// Admits one retained event through the fixed pipeline and
    /// ingests it into `store`.
    ///
    /// `operation_id` binds the caller, tenant, peer and envelope
    /// together for replay/idempotency; `envelope` carries the
    /// event bytes under the carrier contract; `context` is the
    /// authenticated tenant/scope context; `now_ms` is the store's
    /// clock. The store's decision is returned in the outcome; a
    /// refused event is a decided outcome, not a commit failure.
    ///
    /// # Errors
    ///
    /// [`LifecycleRejection`] naming the stage that refused: the
    /// carrier gate (an envelope that is not an events carrier, a
    /// payload that is not a chain event, or an envelope signer
    /// who is not the event's author), the pipeline gate, replay,
    /// prepare (veto or contract), reauthorization, or a commit
    /// step the store failed with an unknown outcome.
    pub fn admit(
        &self,
        store: &mut EventStore,
        operation_id: OperationId,
        envelope: &Envelope,
        context: &AdmissionContext,
        now_ms: u64,
    ) -> Result<EventsAdmissionOutcome, LifecycleRejection> {
        // Carrier gate, part of the fixed gate: the carried
        // object's semantics, checked before any hook runs.
        check_carrier(envelope)?;

        let mut ingest_slot: Option<Ingest> = None;
        let outcome = self
            .pipeline
            .run(operation_id, envelope, context, |_commit| {
                match store.ingest(&envelope.payload, now_ms) {
                    Ok(ingest) => {
                        ingest_slot = Some(ingest);
                        Ok(())
                    }
                    Err(err) => Err(format!("event store ingest failed: {err}")),
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
                Some(ingest) => Ok(EventsAdmissionOutcome::Committed {
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
                Ok(EventsAdmissionOutcome::AlreadyCommitted { receipt })
            }
        }
    }
}

/// The outcome of one pipeline-backed event admission.
#[derive(Debug)]
pub enum EventsAdmissionOutcome {
    /// The operation committed in this run; `ingest` is the
    /// store's decided outcome for the event (accepted, voided,
    /// duplicate, waiting, fork, or refused).
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
        /// What the store decided for the admitted event.
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
/// an events carrier, its payload must decode as a chain event,
/// and the envelope signer must be that event's author. Failures
/// are gate refusals, so no hook ever sees a carried object that
/// fails them.
fn check_carrier(envelope: &Envelope) -> Result<Event, LifecycleRejection> {
    if envelope.body.domain != EVENTS_DOMAIN {
        return Err(LifecycleRejection::Gate(AdmissionRejection::Malformed(
            EnvelopeError::Field {
                field: "domain",
                problem: "is not the events carrier domain",
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

    const RAW: HashProfile = HashProfile::RawBlake3_256;
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
            payload_id: PayloadId::of(RAW, &event.bytes).unwrap(),
            payload_length: event.bytes.len() as u64,
        };
        Envelope {
            body,
            payload: event.bytes.clone(),
            signature: [0_u8; 64],
        }
    }

    #[test]
    fn carrier_check_accepts_a_matching_event_and_envelope() {
        let event = chain_event(11);
        let key = SigningKey::from_bytes(&[11; 32]);
        let envelope = carrier_envelope(&event, EVENTS_DOMAIN, *key.verifying_key().as_bytes());
        let decoded = check_carrier(&envelope).unwrap();
        assert_eq!(decoded.id, event.id);
    }

    #[test]
    fn carrier_check_refuses_a_foreign_domain() {
        let event = chain_event(11);
        let key = SigningKey::from_bytes(&[11; 32]);
        let envelope = carrier_envelope(&event, "id/other", *key.verifying_key().as_bytes());
        let err = check_carrier(&envelope).unwrap_err();
        assert_eq!(err.stage(), "gate");
        assert_eq!(err.class(), "malformed");
    }

    #[test]
    fn carrier_check_refuses_bytes_that_are_not_a_chain_event() {
        let event = chain_event(11);
        let key = SigningKey::from_bytes(&[11; 32]);
        let mut envelope = carrier_envelope(&event, EVENTS_DOMAIN, *key.verifying_key().as_bytes());
        envelope.payload = vec![9; 8];
        let err = check_carrier(&envelope).unwrap_err();
        assert_eq!(err.stage(), "gate");
        assert_eq!(err.class(), "malformed");
    }

    #[test]
    fn carrier_check_refuses_an_envelope_signer_who_is_not_the_author() {
        let event = chain_event(11);
        let other = SigningKey::from_bytes(&[22; 32]);
        let envelope = carrier_envelope(&event, EVENTS_DOMAIN, *other.verifying_key().as_bytes());
        let err = check_carrier(&envelope).unwrap_err();
        assert_eq!(err.stage(), "gate");
        assert_eq!(err.class(), "signer_binding");
    }
}
