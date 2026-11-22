// Portal send and delivery operations.

use super::*;

impl WorldActor {
    /// Record one F12.14 portal audit shape (code#1068 wiring
    /// slice) in this world's retained audit log, in the exact
    /// fields the kernel built it in.
    pub(in crate::world::actor) fn audit_portal_shape(
        &self,
        shape: &id_caps::portal::PortalAuditShape,
    ) {
        self.audit_record(
            "portal",
            shape.action,
            shape.parameters.clone(),
            shape.outcome,
        );
    }

    /// Refuse a portal send on this (sending) world: record the
    /// refusal shape in this world's audit log and return it.
    pub(in crate::world::actor) fn refuse_portal_send(
        &self,
        refusal: PortalRefusal,
        from_world: &str,
        to_world: &str,
        message_bytes: usize,
    ) -> PortalSendError {
        self.audit_portal_shape(&id_caps::portal::audit_for_refusal(
            id_caps::portal::PORTAL_SEND_ACTION,
            &refusal,
            from_world,
            to_world,
            message_bytes,
        ));
        PortalSendError::Refused(refusal)
    }

    /// Phase 1 of a portal send (F12.14, code#1068 wiring slice):
    /// this world must be writable — a read-only replica, a
    /// storage-failed writer, or an actor that handed off its write
    /// lease originates no portal effects (CodeRabbit finding on
    /// PR #10840) — and must itself grant `target`. Writability is a
    /// transport/fencing fact, so it answers `Unavailable` (no
    /// policy decision, no audit entry); a missing or revoked local
    /// grant is the kernel's refusal, audited here. This phase is
    /// actor-local: it runs no cross-world await, so the command
    /// loop is never held while waiting on a peer actor.
    pub(in crate::world::actor) fn portal_send_precheck(
        &self,
        target: &str,
        bytes: usize,
    ) -> Result<(), PortalSendError> {
        if let Err(error) = self.check_writable() {
            return Err(PortalSendError::Unavailable(format!(
                "portal send fenced: {error}"
            )));
        }
        let from_world = self.core.world_id().to_owned();
        match self.core.portal_grants.state(target) {
            Some(PortalGrantState::Granted) => Ok(()),
            Some(PortalGrantState::Revoked) => Err(self.refuse_portal_send(
                PortalRefusal::SenderRevoked {
                    peer: target.to_owned(),
                },
                &from_world,
                target,
                bytes,
            )),
            None => Err(self.refuse_portal_send(
                PortalRefusal::SenderNotGranted {
                    peer: target.to_owned(),
                },
                &from_world,
                target,
                bytes,
            )),
        }
    }

    /// Phase 3 of a portal send: the full mutual check, run by the
    /// actor against the peer book the handle fetched through the
    /// transport while this loop was free (phase 2). Re-runs the
    /// writability fence and the local grant check (a revocation
    /// between phases refuses here), then `check_portal_message`
    /// in full, then the delivered-envelope bound, then the
    /// per-ordered-pair rate admission. On success replies with
    /// this world's id, the source name the transport delivers
    /// under. Every refusal is audited as `portal.send` refused.
    pub(in crate::world::actor) fn portal_send_admit(
        &mut self,
        target: &str,
        peer_grants: &PortalGrants,
        message: &str,
    ) -> Result<String, PortalSendError> {
        let from_world = self.core.world_id().to_owned();
        let bytes = message.len();
        if let Err(error) = self.check_writable() {
            return Err(PortalSendError::Unavailable(format!(
                "portal send fenced: {error}"
            )));
        }
        if let Err(refusal) = id_caps::portal::check_portal_message(
            &self.core.portal_grants,
            peer_grants,
            &from_world,
            target,
            bytes,
        ) {
            return Err(self.refuse_portal_send(refusal, &from_world, target, bytes));
        }
        // The 16 KiB bound is on the raw message; the delivered
        // envelope is the serialized host event, whose escaping can
        // carry it past the world event ceiling (a 16 KiB message
        // of control characters escapes to ~96 KiB). Measure the
        // envelope here so an undeliverable message refuses as
        // message-too-large on the send side, with the envelope
        // size and the event ceiling it exceeded, instead of being
        // dropped by delivery as a bare Unavailable.
        let envelope_bytes = portal_message_event(&from_world, message).len();
        if envelope_bytes > crate::world_caps::MAX_EVENT_BYTES {
            return Err(self.refuse_portal_send(
                PortalRefusal::MessageTooLarge {
                    bytes: envelope_bytes,
                    max: crate::world_caps::MAX_EVENT_BYTES,
                },
                &from_world,
                target,
                bytes,
            ));
        }
        let now_ms = self.core.clock().now_ms();
        if let Err(refusal) = self
            .portal_rate
            .entry(target.to_owned())
            .or_default()
            .admit(now_ms)
        {
            return Err(self.refuse_portal_send(refusal, &from_world, target, bytes));
        }
        Ok(from_world)
    }

    /// Phase 5 of a portal send: record the send-side audit shape
    /// for the delivery outcome the handle observed in phase 4.
    /// `None` records `portal.send` accepted; `Some(refusal)`
    /// records the refusal shape and replies `Refused`, mirroring
    /// the target's verdict back to the caller.
    pub(in crate::world::actor) fn portal_send_record(
        &self,
        target: &str,
        bytes: usize,
        refusal: Option<PortalRefusal>,
    ) -> Result<(), PortalSendError> {
        let from_world = self.core.world_id().to_owned();
        match refusal {
            Some(refusal) => Err(self.refuse_portal_send(refusal, &from_world, target, bytes)),
            None => {
                self.audit_portal_shape(&id_caps::portal::audit_for_send(
                    &from_world,
                    target,
                    bytes,
                ));
                Ok(())
            }
        }
    }

    /// Accept (or refuse) one portal message arriving from `source`
    /// (F12.14, code#1068 wiring slice). The receiving side re-checks
    /// its own book — the send-side check read a snapshot, and a
    /// grant revoked in between must refuse here as revoked — and
    /// the size bound, then delivers the message to the program as a
    /// host event naming its source. Acceptance records
    /// `portal.deliver` accepted in this world's audit log; refusal
    /// records `portal.deliver` refused with the refusal's code.
    pub(in crate::world::actor) async fn accept_portal_delivery(
        &mut self,
        source: &str,
        message: String,
    ) -> Result<(), PortalSendError> {
        let to_world = self.core.world_id().to_owned();
        let bytes = message.len();
        let refuse = |actor: &Self, refusal: PortalRefusal| {
            actor.audit_portal_shape(&id_caps::portal::audit_for_refusal(
                id_caps::portal::PORTAL_DELIVER_ACTION,
                &refusal,
                source,
                &to_world,
                bytes,
            ));
            PortalSendError::Refused(refusal)
        };
        match self.core.portal_grants.state(source) {
            Some(PortalGrantState::Granted) => {}
            Some(PortalGrantState::Revoked) => {
                return Err(refuse(
                    self,
                    PortalRefusal::PeerRevoked {
                        peer: source.to_owned(),
                    },
                ));
            }
            None => {
                return Err(refuse(
                    self,
                    PortalRefusal::PeerNotGranted {
                        peer: source.to_owned(),
                    },
                ));
            }
        }
        if bytes > id_caps::portal::MAX_PORTAL_MESSAGE_BYTES {
            return Err(refuse(
                self,
                PortalRefusal::MessageTooLarge {
                    bytes,
                    max: id_caps::portal::MAX_PORTAL_MESSAGE_BYTES,
                },
            ));
        }
        // The delivered envelope is the serialized host event: a
        // raw message inside the 16 KiB bound can still escape past
        // the world event ceiling, in which case delivery would
        // drop it and misreport the cause as Unavailable. Refuse
        // it here as message-too-large, carrying the envelope size
        // and the event ceiling it exceeded (the send side runs
        // the same measurement in PortalSendAdmit).
        let event = portal_message_event(source, &message);
        if event.len() > crate::world_caps::MAX_EVENT_BYTES {
            return Err(refuse(
                self,
                PortalRefusal::MessageTooLarge {
                    bytes: event.len(),
                    max: crate::world_caps::MAX_EVENT_BYTES,
                },
            ));
        }
        // Full writability fence, not just the replica flag: an
        // actor that handed off its write lease (or whose storage
        // failed) must not acknowledge a mutation past the
        // ownership boundary (CodeRabbit finding on PR #10840).
        if let Err(error) = self.check_writable() {
            return Err(PortalSendError::Unavailable(format!(
                "portal delivery fenced: {error}"
            )));
        }
        if !self.deliver(event).await {
            return Err(PortalSendError::Unavailable(
                "target program did not accept the portal message".to_owned(),
            ));
        }
        self.audit_portal_shape(&id_caps::portal::audit_for_deliver(
            source, &to_world, bytes,
        ));
        Ok(())
    }
}
