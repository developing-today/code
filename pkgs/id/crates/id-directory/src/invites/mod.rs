//! Invitations: a pasted ticket, or a link that names the inviter, and the inviter's signed event.
//!
//! The check runs before the accept is sent. An event is checked when one is present. A bad
//! signature, or an event that does not match the link or the ticket, refuses the join, and
//! nothing is sent. A ticket with no event is still accepted, as [`RouteStatus::Unverified`].
//!
//! # Forms
//!
//! - A **ticket** is a standard iroh `EndpointTicket`, pasted as it is. Any iroh client parses it.
//!   It names no inviter, so no event can be checked against a key, and the join is unverified.
//! - A **link** is the ticket in URL form, `id:join?group=G&ticket=T&by=K`. `G` is the group ID,
//!   `K` the inviter's account key, both 64 hex characters. `by` is the trust anchor: an event
//!   counts as verified only when it is signed by `K`.
//! - An **invite event** is signed by the inviter. It names the group, the role, the expiry, and
//!   the ticket (by digest). It is stored in the inviter's chain and fetched by the joiner; it is
//!   not pasted.
//!
//! # What an address exposes (N30)
//!
//! Address exposure is documented, not hidden. Authorization does not stop it.
//!
//! - **Who learns the inviter's address from the link.** Anyone who holds the link learns the
//!   inviter's `EndpointId` and every address the ticket lists: a relay URL, and any IP:port the
//!   inviter chose to include. A ticket can list no IP at all (a relay-only route). A ticket has
//!   no expiry, so it keeps working for as long as its addresses do.
//! - **The joiner's address.** When the joiner sends the accept, the receiving node learns the
//!   joiner's `EndpointId` and addresses. Under QUIC address discovery the joiner's public address
//!   also reaches any peer it connects to. "Authorize before connecting" refuses peers after
//!   they connect, and the candidates arrive first (round 2, iroh-allowlist). It is access
//!   control, not privacy.
//! - **What the link adds.** Only the group ID, the ticket and `by`, which is identifying but
//!   not secret. It ties the inviter to the group for anyone who sees the link.
//! - **What the link does not reveal.** The role and the expiry (those live in the signed event,
//!   which a joiner fetches from the group's node). The group's members, history or policy. Any
//!   join secret: the Seitan key (P6.1) is typed in, or travels in a fragment, never in this
//!   format. Whether the invite was used or revoked. The inviter's device keys or chain.
//! - **The event, once stored.** Any node that stores it sees the author (the inviter's key), the
//!   group ID, the kind, the size and the timing, in any scheme. The body is encrypted in an
//!   end-to-end group and readable in a `scheme: none` group, so there the role and expiry are
//!   visible to anyone who can pull the event. The ticket digest does not reveal the ticket's
//!   addresses, but anyone who holds the ticket can confirm it.

mod event;
mod invitation;

use anyhow::{Result, ensure};
use iroh_tickets::endpoint::EndpointTicket;

pub use event::{InviteBody, InviteEvent};
pub use invitation::Invitation;

use event::ticket_digest;

/// Where an accept is sent. The real sender dials the ticket. Tests record the calls.
pub trait AcceptSender {
    /// Sends `accept` to the endpoint named by `to`.
    ///
    /// # Errors
    ///
    /// Fails if the send fails.
    fn send(&mut self, to: &EndpointTicket, accept: &[u8]) -> Result<()>;
}

/// What the check established about the route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteStatus {
    /// The inviter's signed event, checked against the inviter's key from the link, vouches for
    /// the ticket.
    Verified,
    /// No event could be checked against an inviter key. The join may go ahead, with a label.
    Unverified,
}

/// Checks an event (when present) against an invitation, at `now_ms`. It does not send anything.
///
/// # Errors
///
/// Fails if the event is signed by another key than the link names, has a bad signature, is for
/// a different ticket, or has expired.
pub fn check_route(
    invitation: &Invitation,
    event: Option<&InviteEvent>,
    now_ms: u64,
) -> Result<RouteStatus> {
    let Some(event) = event else {
        return Ok(RouteStatus::Unverified);
    };
    if let Some(inviter) = invitation.inviter() {
        ensure!(
            event.body.inviter == inviter,
            "the invite event is not signed by the inviter in the link"
        );
    }
    event.verify_signature()?;
    if let Invitation::Link { group, .. } = invitation {
        ensure!(
            event.body.group == *group,
            "the invite event is for another group"
        );
    }
    ensure!(
        event.body.ticket_digest == ticket_digest(invitation.ticket()),
        "the invite event is for a different ticket"
    );
    ensure!(
        now_ms < event.body.expires_ms,
        "the invite expired at {} ms",
        event.body.expires_ms
    );
    Ok(match invitation.inviter() {
        Some(_) => RouteStatus::Verified,
        None => RouteStatus::Unverified,
    })
}

/// Checks the event (when present) against the invitation at `now_ms`, then sends the accept over
/// the ticket.
///
/// A refused check returns an error, and nothing is sent.
///
/// # Errors
///
/// Fails if the check refuses the invitation, or if the send fails.
pub fn join(
    invitation: &Invitation,
    event: Option<&InviteEvent>,
    now_ms: u64,
    sender: &mut impl AcceptSender,
    accept: &[u8],
) -> Result<RouteStatus> {
    let status = check_route(invitation, event, now_ms)?;
    sender.send(invitation.ticket(), accept)?;
    Ok(status)
}
