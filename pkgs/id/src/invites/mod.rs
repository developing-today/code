//! Invitations: a pasted ticket, or a link that names the inviter, and the inviter's signed event.
//!
//! The check runs before the accept is sent. An event is checked when one is present. A bad
//! signature, or an event that does not match the link or the ticket, refuses the join, and
//! nothing is sent. A ticket with no event is still accepted, as [`RouteStatus::Unverified`].

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

/// Checks an event (when present) against an invitation. It does not send anything.
///
/// # Errors
///
/// Fails if the event is signed by another key than the link names, has a bad signature, or is
/// for a different ticket.
pub fn check_route(invitation: &Invitation, event: Option<&InviteEvent>) -> Result<RouteStatus> {
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
    ensure!(
        event.body.ticket_digest == ticket_digest(invitation.ticket()),
        "the invite event is for a different ticket"
    );
    Ok(match invitation.inviter() {
        Some(_) => RouteStatus::Verified,
        None => RouteStatus::Unverified,
    })
}

/// Checks the event (when present) against the invitation, then sends the accept over the ticket.
///
/// A refused check returns an error, and nothing is sent.
///
/// # Errors
///
/// Fails if the check refuses the invitation, or if the send fails.
pub fn join(
    invitation: &Invitation,
    event: Option<&InviteEvent>,
    sender: &mut impl AcceptSender,
    accept: &[u8],
) -> Result<RouteStatus> {
    let status = check_route(invitation, event)?;
    sender.send(invitation.ticket(), accept)?;
    Ok(status)
}
