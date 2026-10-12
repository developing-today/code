//! Invitations: a pasted ticket, a link that names the inviter, and the inviter's signed event.
//!
//! Nothing here touches the network. The accept goes through an [`AcceptSender`] that records
//! each send, so a refusal can be checked as "no send happened".

#![allow(clippy::unwrap_used, clippy::expect_used)]

use anyhow::Result;
use ed25519_dalek::SigningKey;
use id::invites::{AcceptSender, Invitation, InviteEvent, RouteStatus, check_route, join};
use iroh_base::{EndpointAddr, SecretKey, TransportAddr};
use iroh_tickets::endpoint::EndpointTicket;

/// A ticket for the endpoint with the given seed, at an address that differs per seed.
fn ticket(seed: u8) -> EndpointTicket {
    let id = SecretKey::from_bytes(&[seed; 32]).public();
    let ip = format!("203.0.113.{seed}:4433").parse().unwrap();
    EndpointTicket::new(EndpointAddr::from_parts(id, [TransportAddr::Ip(ip)]))
}

fn inviter() -> SigningKey {
    SigningKey::from_bytes(&[0x11; 32])
}

fn attacker() -> SigningKey {
    SigningKey::from_bytes(&[0x22; 32])
}

fn to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .concat()
}

/// The inviter's account key as the link writes it.
fn by(key: &SigningKey) -> String {
    to_hex(&key.verifying_key().to_bytes())
}

/// A group ID: 32 bytes, all `byte`, as 64 hex characters.
fn group(byte: u8) -> String {
    to_hex(&[byte; 32])
}

/// A link that carries the ticket and the inviter's key, as P6.4 writes them.
fn link(ticket: &EndpointTicket, by: &str) -> String {
    format!("id:join?group={}&ticket={ticket}&by={by}", group(0xaa))
}

/// Records each accept that is sent. A test that expects a refusal asserts that this is empty.
#[derive(Default)]
struct Recorder {
    sent: Vec<(String, Vec<u8>)>,
}

impl AcceptSender for Recorder {
    fn send(&mut self, to: &EndpointTicket, accept: &[u8]) -> Result<()> {
        self.sent.push((to.to_string(), accept.to_vec()));
        Ok(())
    }
}

#[test]
fn a_pasted_ticket_parses_to_an_unsigned_invitation() {
    let ticket = ticket(7);
    let invitation = Invitation::parse(&ticket.to_string()).unwrap();
    assert_eq!(invitation.ticket().to_string(), ticket.to_string());
    assert_eq!(invitation.inviter(), None);
}

#[test]
fn a_pasted_ticket_with_surrounding_whitespace_is_trimmed() {
    let ticket = ticket(7);
    let pasted = format!("  \n{ticket}\t\n");
    let invitation = Invitation::parse(&pasted).unwrap();
    assert_eq!(invitation.ticket().to_string(), ticket.to_string());
}

#[test]
fn a_link_carries_the_inviter_key() {
    let ticket = ticket(7);
    let invitation = Invitation::parse(&link(&ticket, &by(&inviter()))).unwrap();
    assert_eq!(
        invitation.inviter(),
        Some(inviter().verifying_key().to_bytes())
    );
    assert_eq!(invitation.ticket().to_string(), ticket.to_string());
}

#[test]
fn a_link_without_an_inviter_key_is_refused() {
    let text = format!("id:join?group={}&ticket={}", group(0xaa), ticket(7));
    let error = Invitation::parse(&text).unwrap_err();
    assert!(format!("{error:#}").contains("by"), "{error:#}");
}

#[test]
fn malformed_input_is_refused_with_a_clear_error() {
    let ticket = ticket(7);
    let by = by(&inviter());
    let cases: Vec<(String, &str)> = vec![
        (String::new(), "endpoint ticket"),
        ("not a ticket".to_owned(), "endpoint ticket"),
        ("endpointzz".to_owned(), "endpoint ticket"),
        (
            format!("id:join?group={}&ticket={ticket}&by=abcd", group(0xaa)),
            "64 hex characters",
        ),
        (format!("id:join?ticket={ticket}&by={by}"), "group"),
        (
            format!("id:join?group=xyz&ticket={ticket}&by={by}"),
            "64 hex characters",
        ),
    ];
    for (text, expected) in cases {
        let error = Invitation::parse(&text).expect_err(&text);
        assert!(
            format!("{error:#}").contains(expected),
            "{text:?}: {error:#}"
        );
    }
}

#[test]
fn a_bare_ticket_with_no_event_is_unsigned_and_the_accept_is_sent() {
    let invitation = Invitation::parse(&ticket(7).to_string()).unwrap();
    let mut sent = Recorder::default();
    let status = join(&invitation, None, &mut sent, b"accept").unwrap();
    assert_eq!(status, RouteStatus::Unverified);
    assert_eq!(sent.sent.len(), 1);
    assert_eq!(sent.sent[0].1, b"accept");
}

#[test]
fn a_valid_signed_event_is_checked_and_the_accept_is_sent() {
    let ticket = ticket(7);
    let invitation = Invitation::parse(&link(&ticket, &by(&inviter()))).unwrap();
    let event = InviteEvent::sign(&inviter(), &ticket);
    let mut sent = Recorder::default();
    let status = join(&invitation, Some(&event), &mut sent, b"accept").unwrap();
    assert_eq!(status, RouteStatus::Verified);
    assert_eq!(sent.sent.len(), 1);
}

#[test]
fn a_bad_event_signature_refuses_the_accept_with_no_send() {
    let ticket = ticket(7);
    let invitation = Invitation::parse(&link(&ticket, &by(&inviter()))).unwrap();
    let mut event = InviteEvent::sign(&inviter(), &ticket);
    event.signature[0] ^= 1;
    let mut sent = Recorder::default();
    let error = join(&invitation, Some(&event), &mut sent, b"accept").unwrap_err();
    assert!(format!("{error:#}").contains("signature"), "{error:#}");
    assert!(sent.sent.is_empty(), "an accept was sent after a refusal");
}

#[test]
fn an_event_from_another_key_refuses_the_accept_with_no_send() {
    let ticket = ticket(7);
    let invitation = Invitation::parse(&link(&ticket, &by(&inviter()))).unwrap();
    let event = InviteEvent::sign(&attacker(), &ticket);
    let mut sent = Recorder::default();
    let error = join(&invitation, Some(&event), &mut sent, b"accept").unwrap_err();
    assert!(
        format!("{error:#}").contains("not signed by the inviter"),
        "{error:#}"
    );
    assert!(sent.sent.is_empty(), "an accept was sent after a refusal");
}

#[test]
fn an_event_for_another_ticket_refuses_the_accept_with_no_send() {
    let invitation = Invitation::parse(&link(&ticket(7), &by(&inviter()))).unwrap();
    let event = InviteEvent::sign(&inviter(), &ticket(8));
    let mut sent = Recorder::default();
    let error = join(&invitation, Some(&event), &mut sent, b"accept").unwrap_err();
    assert!(
        format!("{error:#}").contains("different ticket"),
        "{error:#}"
    );
    assert!(sent.sent.is_empty(), "an accept was sent after a refusal");
}

#[test]
fn a_bare_ticket_with_a_valid_event_is_unverified_because_no_key_anchors_it() {
    let ticket = ticket(7);
    let invitation = Invitation::parse(&ticket.to_string()).unwrap();
    let event = InviteEvent::sign(&inviter(), &ticket);
    assert_eq!(
        check_route(&invitation, Some(&event)).unwrap(),
        RouteStatus::Unverified
    );
}
