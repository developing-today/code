//! F12.14 portals — the mutual-grant policy kernel (code#1068).
//!
//! A portal is cross-world messaging that exists only where the
//! admins of BOTH worlds have granted it for the peer world
//! (Phase 12 plan, workstream E: "Cross-world messaging through a
//! capability that both worlds' admins have granted, with bounded
//! rate and size and an audit entry on each side"). This module is
//! the pure policy half of that feature: per-world, per-peer grant
//! state, the mutual check a send must pass, the size/rate bounds,
//! the refusal codes, and the audit-entry shapes the transport
//! records on each side. It performs no I/O and owns no transport;
//! the wiring that carries a message between two world actors and
//! writes these shapes into each world's F12.6 audit log
//! (`id-worlds` `world::audit::WorldAuditLog`) is the follow-on
//! slice, and it must refuse exactly what [`check_portal_message`]
//! refuses.
//!
//! Registration posture: a portal is NOT a Wasm provider, so it
//! takes no entry in the F12.4 provider catalog — the plan's
//! authority-creep rule bars portals from adding a Wasm import,
//! and a catalog entry would do exactly that. The registration
//! surface for a portal is the admin grant itself: a world's
//! [`PortalGrants`] book is its registry of portal peers, written
//! only by that world's admin, and a portal to a peer exists only
//! while both books name each other [`PortalGrantState::Granted`].
//! Portals are therefore off by default (an empty book grants
//! nothing), per-peer, and revocable on either side.
//!
//! Bounds follow the `world_caps` style: named constants, enforced
//! at the check, with the limit carried in the refusal so a refusal
//! is auditable without re-reading the code.

use std::collections::BTreeMap;

/// Largest portal message, in bytes. Portal messages are chat-class
/// payloads, not blob transfers (the blob gate is a separate,
/// one-sided path whose semantics this feature must not change):
/// a quarter of the 64 KiB world event ceiling
/// (`world_caps::MAX_EVENT_BYTES`).
pub const MAX_PORTAL_MESSAGE_BYTES: usize = 16 * 1024;

/// Messages admitted per ordered world pair per rate window.
pub const PORTAL_RATE_LIMIT: u32 = 32;

/// Portal rate window length, in milliseconds.
pub const PORTAL_RATE_WINDOW_MS: u64 = 60_000;

/// Audit action recorded on the sending world for a portal send
/// (accepted or refused). Matches the F12.6 audit log's stable
/// action-name posture (`world::audit::AuditRecord::action`).
pub const PORTAL_SEND_ACTION: &str = "portal.send";

/// Audit action recorded on the receiving world when a portal
/// message is delivered.
pub const PORTAL_DELIVER_ACTION: &str = "portal.deliver";

/// Whether one world currently grants portal messaging with a peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortalGrantState {
    /// The world's admin granted the portal; messages may pass
    /// (subject to the peer's own grant and the bounds).
    Granted,
    /// The world's admin granted and later revoked the portal. A
    /// revoked grant refuses differently from a grant that never
    /// existed, so the revocation itself stays auditable.
    Revoked,
}

/// One world's portal grant book: the admin-written registry of
/// portal peers for a single world. Worlds hold one book each; the
/// mutual check reads the sender's book and the receiver's book.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PortalGrants {
    by_peer: BTreeMap<String, PortalGrantState>,
}

impl PortalGrants {
    /// An empty book: no portals, the off-by-default state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the admin's grant of a portal to `peer`. Re-granting
    /// a revoked peer restores it to [`PortalGrantState::Granted`].
    pub fn grant(&mut self, peer: &str) {
        self.by_peer
            .insert(peer.to_owned(), PortalGrantState::Granted);
    }

    /// Revoke a previously granted portal to `peer`. Returns
    /// whether a live grant existed: revoking a peer that was never
    /// granted (or already revoked) changes nothing and reports
    /// `false`, so an admin surface can name the no-op.
    pub fn revoke(&mut self, peer: &str) -> bool {
        match self.by_peer.get_mut(peer) {
            Some(state) if *state == PortalGrantState::Granted => {
                *state = PortalGrantState::Revoked;
                true
            }
            _ => false,
        }
    }

    /// Restore one peer's recorded state from durable storage (the
    /// wiring slice's journal replay): `Some(state)` records exactly
    /// that state — including a [`PortalGrantState::Revoked`]
    /// tombstone for a peer that was never live-granted in this
    /// process — and `None` removes the peer from the book entirely.
    pub fn restore_state(&mut self, peer: &str, state: Option<PortalGrantState>) {
        match state {
            Some(state) => {
                self.by_peer.insert(peer.to_owned(), state);
            }
            None => {
                self.by_peer.remove(peer);
            }
        }
    }

    /// The recorded state for `peer`, or `None` when this world has
    /// never granted that peer a portal.
    #[must_use]
    pub fn state(&self, peer: &str) -> Option<PortalGrantState> {
        self.by_peer.get(peer).copied()
    }

    /// Whether this world currently grants a portal to `peer`.
    #[must_use]
    pub fn allows(&self, peer: &str) -> bool {
        self.state(peer) == Some(PortalGrantState::Granted)
    }

    /// Every peer this book names, in name order, with its state.
    pub fn peers(&self) -> impl Iterator<Item = (&str, PortalGrantState)> {
        self.by_peer.iter().map(|(p, s)| (p.as_str(), *s))
    }
}

/// Why a portal message was refused. Each variant names its subject
/// exhaustively — a refusal that cannot say what it refused is not
/// auditable (the `ResolutionRefusal` posture, applied to portals).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortalRefusal {
    /// The sending world has never granted a portal to the peer.
    SenderNotGranted {
        /// The peer world the sender never granted.
        peer: String,
    },
    /// The receiving world has never granted a portal to the sender.
    PeerNotGranted {
        /// The sending world the receiver never granted.
        peer: String,
    },
    /// The sending world granted and revoked the portal.
    SenderRevoked {
        /// The peer world whose grant was revoked.
        peer: String,
    },
    /// The receiving world granted and revoked the portal.
    PeerRevoked {
        /// The sending world whose grant was revoked.
        peer: String,
    },
    /// The message exceeds [`MAX_PORTAL_MESSAGE_BYTES`].
    MessageTooLarge {
        /// The message's size in bytes.
        bytes: usize,
        /// The bound it exceeded.
        max: usize,
    },
    /// The ordered pair exhausted its rate window.
    RateLimited {
        /// The limit that was exhausted.
        limit: u32,
        /// The window the limit applies to, in milliseconds.
        window_ms: u64,
    },
}

impl PortalRefusal {
    /// The stable machine-readable refusal code recorded in audit
    /// parameters. Codes are part of the audit surface: renaming one
    /// is a breaking change to every consumer of the audit stream.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::SenderNotGranted { .. } => "portal.sender_not_granted",
            Self::PeerNotGranted { .. } => "portal.peer_not_granted",
            Self::SenderRevoked { .. } => "portal.sender_revoked",
            Self::PeerRevoked { .. } => "portal.peer_revoked",
            Self::MessageTooLarge { .. } => "portal.message_too_large",
            Self::RateLimited { .. } => "portal.rate_limited",
        }
    }
}

/// The mutual-grant and bounds check one portal message must pass.
/// Grant checks run before bounds checks: a world that has not
/// granted the portal learns nothing about the bounds, and a
/// granted pair's oversize message is refused as oversize, never
/// as ungranted. Rate admission is stateful and checked separately
/// through [`PortalRateWindow`], so this function stays a pure
/// function of the two books and the message size.
///
/// `from_world` is the sending world's id and `to_world` the
/// receiving world's id; `sender_grants` is the sending world's
/// book and `receiver_grants` the receiving world's book.
pub fn check_portal_message(
    sender_grants: &PortalGrants,
    receiver_grants: &PortalGrants,
    from_world: &str,
    to_world: &str,
    message_bytes: usize,
) -> Result<(), PortalRefusal> {
    match sender_grants.state(to_world) {
        Some(PortalGrantState::Granted) => {}
        Some(PortalGrantState::Revoked) => {
            return Err(PortalRefusal::SenderRevoked {
                peer: to_world.to_owned(),
            });
        }
        None => {
            return Err(PortalRefusal::SenderNotGranted {
                peer: to_world.to_owned(),
            });
        }
    }
    match receiver_grants.state(from_world) {
        Some(PortalGrantState::Granted) => {}
        Some(PortalGrantState::Revoked) => {
            return Err(PortalRefusal::PeerRevoked {
                peer: from_world.to_owned(),
            });
        }
        None => {
            return Err(PortalRefusal::PeerNotGranted {
                peer: from_world.to_owned(),
            });
        }
    }
    if message_bytes > MAX_PORTAL_MESSAGE_BYTES {
        return Err(PortalRefusal::MessageTooLarge {
            bytes: message_bytes,
            max: MAX_PORTAL_MESSAGE_BYTES,
        });
    }
    Ok(())
}

/// Rate admission for one ordered world pair. The transport holds
/// one window per (sender, receiver) pair it serves; the window is
/// a fixed interval of [`PORTAL_RATE_WINDOW_MS`] admitting at most
/// [`PORTAL_RATE_LIMIT`] messages, counted from the first message
/// after the previous window expired. Time is injected (`now_ms`,
/// the same millisecond clock the audit log stamps) so the policy
/// is deterministic under test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortalRateWindow {
    window_start_ms: u64,
    admitted: u32,
}

impl Default for PortalRateWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl PortalRateWindow {
    /// A fresh window with nothing admitted.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            window_start_ms: 0,
            admitted: 0,
        }
    }

    /// Try to admit one message at `now_ms`. On refusal the caller
    /// records [`PortalRefusal::RateLimited`] and the window is
    /// unchanged, so a refused message never consumes quota.
    pub fn admit(&mut self, now_ms: u64) -> Result<(), PortalRefusal> {
        if now_ms >= self.window_start_ms.saturating_add(PORTAL_RATE_WINDOW_MS) {
            self.window_start_ms = now_ms;
            self.admitted = 0;
        }
        if self.admitted >= PORTAL_RATE_LIMIT {
            return Err(PortalRefusal::RateLimited {
                limit: PORTAL_RATE_LIMIT,
                window_ms: PORTAL_RATE_WINDOW_MS,
            });
        }
        self.admitted += 1;
        Ok(())
    }

    /// Messages admitted in the current window.
    #[must_use]
    pub const fn admitted(&self) -> u32 {
        self.admitted
    }
}

/// The shape of one portal audit entry, in the exact fields the
/// F12.6 audit log records (`world::audit::AuditRecord`): a stable
/// `action`, an `outcome` of `accepted` or `refused`, and
/// non-secret `parameters`. The transport builds one of these for
/// the sending world and one for the receiving world for every
/// portal message, so both sides' logs carry the event with the
/// same parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct PortalAuditShape {
    /// `portal.send` on the sending world, `portal.deliver` on the
    /// receiving world.
    pub action: &'static str,
    /// `accepted` or `refused`.
    pub outcome: &'static str,
    /// Non-secret parameters: the two world ids, the message size,
    /// and the refusal code when refused.
    pub parameters: serde_json::Value,
}

fn audit_parameters(
    from_world: &str,
    to_world: &str,
    message_bytes: usize,
    refusal: Option<&PortalRefusal>,
) -> serde_json::Value {
    let mut params = serde_json::json!({
        "from_world": from_world,
        "to_world": to_world,
        "bytes": message_bytes,
    });
    if let Some(refusal) = refusal {
        params["refusal"] = serde_json::Value::String(refusal.code().to_owned());
    }
    params
}

/// The sending world's audit shape for an accepted send.
#[must_use]
pub fn audit_for_send(from_world: &str, to_world: &str, message_bytes: usize) -> PortalAuditShape {
    PortalAuditShape {
        action: PORTAL_SEND_ACTION,
        outcome: "accepted",
        parameters: audit_parameters(from_world, to_world, message_bytes, None),
    }
}

/// The receiving world's audit shape for an accepted delivery.
#[must_use]
pub fn audit_for_deliver(
    from_world: &str,
    to_world: &str,
    message_bytes: usize,
) -> PortalAuditShape {
    PortalAuditShape {
        action: PORTAL_DELIVER_ACTION,
        outcome: "accepted",
        parameters: audit_parameters(from_world, to_world, message_bytes, None),
    }
}

/// The audit shape for a refused portal message, for whichever
/// world's log records the refusal (the sender's for a send
/// refusal; the receiver's when it refuses a delivery). The
/// refusal code is in the parameters, so the audit stream alone
/// explains why the message did not pass.
#[must_use]
pub fn audit_for_refusal(
    action: &'static str,
    refusal: &PortalRefusal,
    from_world: &str,
    to_world: &str,
    message_bytes: usize,
) -> PortalAuditShape {
    PortalAuditShape {
        action,
        outcome: "refused",
        parameters: audit_parameters(from_world, to_world, message_bytes, Some(refusal)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn granted_book(peer: &str) -> PortalGrants {
        let mut book = PortalGrants::new();
        book.grant(peer);
        book
    }

    // Oracle: the F12.14 exit proof — two worlds exchange a message
    // only when both sides have granted the portal. The full grant
    // matrix: both granted admits; either side ungranted refuses,
    // naming that side.
    #[test]
    fn message_passes_only_when_both_sides_granted() {
        let a = granted_book("world-b");
        let b = granted_book("world-a");
        assert_eq!(
            check_portal_message(&a, &b, "world-a", "world-b", 128),
            Ok(())
        );

        let empty = PortalGrants::new();
        assert_eq!(
            check_portal_message(&empty, &b, "world-a", "world-b", 128),
            Err(PortalRefusal::SenderNotGranted {
                peer: "world-b".to_owned()
            })
        );
        assert_eq!(
            check_portal_message(&a, &empty, "world-a", "world-b", 128),
            Err(PortalRefusal::PeerNotGranted {
                peer: "world-a".to_owned()
            })
        );
        assert_eq!(
            check_portal_message(&empty, &empty, "world-a", "world-b", 128),
            Err(PortalRefusal::SenderNotGranted {
                peer: "world-b".to_owned()
            })
        );
    }

    // Oracle: grants are per-peer — a portal to one world opens
    // nothing toward any other world (off by default, per-peer).
    #[test]
    fn grants_are_per_peer() {
        let a = granted_book("world-c");
        let b = granted_book("world-a");
        assert_eq!(
            check_portal_message(&a, &b, "world-a", "world-b", 128),
            Err(PortalRefusal::SenderNotGranted {
                peer: "world-b".to_owned()
            })
        );
    }

    // Oracle: revocation on EITHER side refuses, and a revoked
    // grant refuses as revoked — distinct from never granted, so
    // the audit stream can tell the two apart.
    #[test]
    fn revocation_on_either_side_refuses_as_revoked() {
        let mut a = granted_book("world-b");
        let mut b = granted_book("world-a");
        assert!(a.revoke("world-b"));
        assert_eq!(
            check_portal_message(&a, &b, "world-a", "world-b", 128),
            Err(PortalRefusal::SenderRevoked {
                peer: "world-b".to_owned()
            })
        );
        // Re-grant restores the sender side; now revoke the peer's.
        a.grant("world-b");
        assert!(b.revoke("world-a"));
        assert_eq!(
            check_portal_message(&a, &b, "world-a", "world-b", 128),
            Err(PortalRefusal::PeerRevoked {
                peer: "world-a".to_owned()
            })
        );
        // Revoking what was never granted is a named no-op.
        assert!(!PortalGrants::new().revoke("world-x"));
    }

    // Oracle: the size bound is enforced at its exact edge — a
    // message at the bound passes the bounds check, one byte over
    // refuses with the bound named.
    #[test]
    fn size_bound_enforced_at_the_edge() {
        let a = granted_book("world-b");
        let b = granted_book("world-a");
        assert_eq!(
            check_portal_message(&a, &b, "world-a", "world-b", MAX_PORTAL_MESSAGE_BYTES),
            Ok(())
        );
        assert_eq!(
            check_portal_message(&a, &b, "world-a", "world-b", MAX_PORTAL_MESSAGE_BYTES + 1),
            Err(PortalRefusal::MessageTooLarge {
                bytes: MAX_PORTAL_MESSAGE_BYTES + 1,
                max: MAX_PORTAL_MESSAGE_BYTES
            })
        );
    }

    // Oracle: the rate bound admits exactly the limit per window,
    // refuses beyond it without consuming quota, and admits again
    // once the window rolls.
    #[test]
    fn rate_window_limits_and_rolls() {
        let mut window = PortalRateWindow::new();
        let start = 1_000_000u64;
        for _ in 0..PORTAL_RATE_LIMIT {
            assert_eq!(window.admit(start), Ok(()));
        }
        assert_eq!(window.admitted(), PORTAL_RATE_LIMIT);
        assert_eq!(
            window.admit(start + 1),
            Err(PortalRefusal::RateLimited {
                limit: PORTAL_RATE_LIMIT,
                window_ms: PORTAL_RATE_WINDOW_MS
            })
        );
        assert_eq!(
            window.admitted(),
            PORTAL_RATE_LIMIT,
            "refusal consumed quota"
        );
        assert_eq!(window.admit(start + PORTAL_RATE_WINDOW_MS), Ok(()));
        assert_eq!(window.admitted(), 1);
    }

    // Oracle: every outcome produces the audit shape its side
    // records — send/deliver accepted shapes name the pair and the
    // size; a refusal shape carries the refusal's stable code, so
    // both worlds' F12.6 logs explain the event on their own.
    #[test]
    fn audit_shapes_cover_send_deliver_and_refusal() {
        let send = audit_for_send("world-a", "world-b", 42);
        assert_eq!(send.action, "portal.send");
        assert_eq!(send.outcome, "accepted");
        assert_eq!(send.parameters["from_world"], "world-a");
        assert_eq!(send.parameters["to_world"], "world-b");
        assert_eq!(send.parameters["bytes"], 42);
        assert!(send.parameters.get("refusal").is_none());

        let deliver = audit_for_deliver("world-a", "world-b", 42);
        assert_eq!(deliver.action, "portal.deliver");
        assert_eq!(deliver.outcome, "accepted");

        let refusal = PortalRefusal::PeerRevoked {
            peer: "world-a".to_owned(),
        };
        assert_eq!(refusal.code(), "portal.peer_revoked");
        let refused = audit_for_refusal(PORTAL_SEND_ACTION, &refusal, "world-a", "world-b", 42);
        assert_eq!(refused.action, "portal.send");
        assert_eq!(refused.outcome, "refused");
        assert_eq!(refused.parameters["refusal"], "portal.peer_revoked");
    }
}
