//! Invitations: a pasted ticket, a link that names the inviter, and the inviter's signed event.
//!
//! Nothing here touches the network. The accept goes through an [`AcceptSender`] that records
//! each send, so a refusal can be checked as "no send happened".

#![allow(clippy::unwrap_used, clippy::expect_used)]

use anyhow::Result;
use ed25519_dalek::SigningKey;
use id::groups::Role;
use id::invites::{AcceptSender, Invitation, InviteEvent, RouteStatus, check_route, join};
use iroh_base::{EndpointAddr, SecretKey, TransportAddr};
use iroh_tickets::endpoint::EndpointTicket;

/// The time the joiner checks against, in milliseconds.
const NOW: u64 = 1_800_000_000_000;
/// The invite's expiry: one day after `NOW`.
const EXPIRES: u64 = NOW + 86_400_000;
/// Where the role byte sits in an invite event's bytes: after the inviter, the digest and the group.
const ROLE_AT: usize = 96;

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

/// A link that carries the ticket and the inviter's key.
fn link(ticket: &EndpointTicket, by: &str) -> String {
    format!("id:join?group={}&ticket={ticket}&by={by}", group(0xaa))
}

/// The inviter's event for `ticket`: group `0xaa`, role `Write`, expiring at `EXPIRES`.
fn event(key: &SigningKey, ticket: &EndpointTicket) -> InviteEvent {
    InviteEvent::sign(key, &[0xaa; 32], Role::Write, EXPIRES, ticket)
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
fn test_a_link_with_duplicate_fields_is_refused() {
    let t7 = ticket(7);
    let dup_ticket = ticket(8);
    let invite_by = by(&inviter());
    let other_by = "22".repeat(32);
    let g1 = group(0xaa);
    let g2 = group(0xbb);

    // Duplicate group (both valid, invalid first, invalid last)
    let cases = vec![
        format!("id:join?group={g1}&group={g2}&ticket={t7}&by={invite_by}"),
        format!("id:join?group=invalid&group={g1}&ticket={t7}&by={invite_by}"),
        format!("id:join?group={g1}&group=invalid&ticket={t7}&by={invite_by}"),
    ];
    for text in cases {
        let error = Invitation::parse(&text).expect_err(&text);
        assert!(format!("{error:#}").contains("group"), "{text}: {error:#}");
    }

    // Duplicate by (both valid, invalid first, invalid last)
    let cases = vec![
        format!("id:join?group={g1}&ticket={t7}&by={invite_by}&by={other_by}"),
        format!("id:join?group={g1}&ticket={t7}&by=invalid&by={invite_by}"),
        format!("id:join?group={g1}&ticket={t7}&by={invite_by}&by=invalid"),
    ];
    for text in cases {
        let error = Invitation::parse(&text).expect_err(&text);
        assert!(format!("{error:#}").contains("by"), "{text}: {error:#}");
    }

    // Duplicate ticket (both valid, invalid first, invalid last)
    let cases = vec![
        format!("id:join?group={g1}&ticket={t7}&ticket={dup_ticket}&by={invite_by}"),
        format!("id:join?group={g1}&ticket=invalid&ticket={t7}&by={invite_by}"),
        format!("id:join?group={g1}&ticket={t7}&ticket=invalid&by={invite_by}"),
    ];
    for text in cases {
        let error = Invitation::parse(&text).expect_err(&text);
        assert!(format!("{error:#}").contains("ticket"), "{text}: {error:#}");
    }

    // Unknown fields keep current handling (ignored, even when repeated)
    let text = format!("id:join?group={g1}&ticket={t7}&by={invite_by}&extra=1&extra=2");
    let invitation = Invitation::parse(&text).expect("link with unknown fields parses");
    match invitation {
        Invitation::Link {
            group,
            by: inviter_by,
            ticket: linked,
        } => {
            assert_eq!(linked.to_string(), t7.to_string());
            assert_eq!(group, [0xaa; 32]);
            assert_eq!(inviter_by, inviter().verifying_key().to_bytes());
        }
        Invitation::Ticket(_) => panic!("expected Link"),
    }
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
    let status = join(&invitation, None, NOW, &mut sent, b"accept").unwrap();
    assert_eq!(status, RouteStatus::Unverified);
    assert_eq!(sent.sent.len(), 1);
    assert_eq!(sent.sent[0].1, b"accept");
}

#[test]
fn a_valid_signed_event_is_checked_and_the_accept_is_sent() {
    let ticket = ticket(7);
    let invitation = Invitation::parse(&link(&ticket, &by(&inviter()))).unwrap();
    let event = event(&inviter(), &ticket);
    let mut sent = Recorder::default();
    let status = join(&invitation, Some(&event), NOW, &mut sent, b"accept").unwrap();
    assert_eq!(status, RouteStatus::Verified);
    assert_eq!(sent.sent.len(), 1);
}

#[test]
fn a_bad_event_signature_refuses_the_accept_with_no_send() {
    let ticket = ticket(7);
    let invitation = Invitation::parse(&link(&ticket, &by(&inviter()))).unwrap();
    let mut event = event(&inviter(), &ticket);
    event.signature[0] ^= 1;
    let mut sent = Recorder::default();
    let error = join(&invitation, Some(&event), NOW, &mut sent, b"accept").unwrap_err();
    assert!(format!("{error:#}").contains("signature"), "{error:#}");
    assert!(sent.sent.is_empty(), "an accept was sent after a refusal");
}

#[test]
fn an_event_from_another_key_refuses_the_accept_with_no_send() {
    let ticket = ticket(7);
    let invitation = Invitation::parse(&link(&ticket, &by(&inviter()))).unwrap();
    let event = event(&attacker(), &ticket);
    let mut sent = Recorder::default();
    let error = join(&invitation, Some(&event), NOW, &mut sent, b"accept").unwrap_err();
    assert!(
        format!("{error:#}").contains("not signed by the inviter"),
        "{error:#}"
    );
    assert!(sent.sent.is_empty(), "an accept was sent after a refusal");
}

#[test]
fn an_event_for_another_ticket_refuses_the_accept_with_no_send() {
    let invitation = Invitation::parse(&link(&ticket(7), &by(&inviter()))).unwrap();
    let event = event(&inviter(), &ticket(8));
    let mut sent = Recorder::default();
    let error = join(&invitation, Some(&event), NOW, &mut sent, b"accept").unwrap_err();
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
    let event = event(&inviter(), &ticket);
    assert_eq!(
        check_route(&invitation, Some(&event), NOW).unwrap(),
        RouteStatus::Unverified
    );
}

#[test]
fn an_invite_event_names_its_group_role_and_expiry() {
    let event = event(&inviter(), &ticket(7));
    assert_eq!(event.body.inviter, inviter().verifying_key().to_bytes());
    assert_eq!(event.body.group, [0xaa; 32]);
    assert_eq!(event.body.role, Role::Write);
    assert_eq!(event.body.expires_ms, EXPIRES);
}

#[test]
fn an_invite_event_round_trips_through_its_bytes_for_every_role() {
    for role in [
        Role::Pull,
        Role::Read,
        Role::Write,
        Role::Manage,
        Role::Admin,
    ] {
        let ticket = ticket(7);
        let event = InviteEvent::sign(&inviter(), &[0xaa; 32], role, EXPIRES, &ticket);
        let bytes = event.to_bytes();
        assert_eq!(InviteEvent::from_bytes(&bytes).unwrap(), event);
    }
}

#[test]
fn a_malformed_invite_event_is_refused() {
    let bytes = event(&inviter(), &ticket(7)).to_bytes();
    let short = &bytes[..bytes.len() - 1];
    let error = InviteEvent::from_bytes(short).unwrap_err();
    assert!(format!("{error:#}").contains("bytes"), "{error:#}");

    let trailing = [bytes.clone(), vec![0]].concat();
    assert!(InviteEvent::from_bytes(&trailing).is_err());

    let mut unknown_role = bytes;
    unknown_role[ROLE_AT] = 9;
    let error = InviteEvent::from_bytes(&unknown_role).unwrap_err();
    assert!(format!("{error:#}").contains("role"), "{error:#}");
}

#[test]
fn changing_the_role_or_the_expiry_breaks_the_signature() {
    let ticket = ticket(7);
    let invitation = Invitation::parse(&link(&ticket, &by(&inviter()))).unwrap();

    let mut promoted = event(&inviter(), &ticket);
    promoted.body.role = Role::Admin;
    let mut sent = Recorder::default();
    let error = join(&invitation, Some(&promoted), NOW, &mut sent, b"accept").unwrap_err();
    assert!(format!("{error:#}").contains("signature"), "{error:#}");

    let mut extended = event(&inviter(), &ticket);
    extended.body.expires_ms += 1;
    let error = join(&invitation, Some(&extended), NOW, &mut sent, b"accept").unwrap_err();
    assert!(format!("{error:#}").contains("signature"), "{error:#}");
    assert!(sent.sent.is_empty(), "an accept was sent after a refusal");
}

#[test]
fn an_expired_invite_refuses_the_accept_with_no_send() {
    let ticket = ticket(7);
    let invitation = Invitation::parse(&link(&ticket, &by(&inviter()))).unwrap();
    let event = event(&inviter(), &ticket);
    let mut sent = Recorder::default();

    let before = join(&invitation, Some(&event), EXPIRES - 1, &mut sent, b"accept");
    assert_eq!(before.unwrap(), RouteStatus::Verified);

    let error = join(&invitation, Some(&event), EXPIRES, &mut sent, b"accept").unwrap_err();
    assert!(format!("{error:#}").contains("expired"), "{error:#}");
    assert_eq!(sent.sent.len(), 1, "only the accept before expiry was sent");
}

#[test]
fn an_event_for_another_group_refuses_the_accept_with_no_send() {
    let ticket = ticket(7);
    let invitation = Invitation::parse(&link(&ticket, &by(&inviter()))).unwrap();
    let event = InviteEvent::sign(&inviter(), &[0xbb; 32], Role::Write, EXPIRES, &ticket);
    let mut sent = Recorder::default();
    let error = join(&invitation, Some(&event), NOW, &mut sent, b"accept").unwrap_err();
    assert!(format!("{error:#}").contains("another group"), "{error:#}");
    assert!(sent.sent.is_empty(), "an accept was sent after a refusal");
}

#[test]
fn a_link_is_the_ticket_in_url_form() {
    let ticket = ticket(7);
    let key = inviter().verifying_key().to_bytes();
    let link = Invitation::Link {
        group: [0xaa; 32],
        by: key,
        ticket: ticket.clone(),
    };
    let text = link.to_string();
    assert_eq!(text, link_text(&ticket));
    assert_eq!(Invitation::parse(&text).unwrap(), link);

    let bare = Invitation::Ticket(ticket.clone());
    assert_eq!(bare.to_string(), ticket.to_string());
}

/// The link the writer should produce for `ticket`, from the inviter's key and group `0xaa`.
fn link_text(ticket: &EndpointTicket) -> String {
    format!(
        "id:join?group={}&ticket={ticket}&by={}",
        group(0xaa),
        by(&inviter())
    )
}

#[test]
fn the_link_carries_exactly_the_group_ticket_and_inviter() {
    // Address exposure (N30) is documented as this list: a link adds no field beyond these three.
    let link = Invitation::Link {
        group: [0xaa; 32],
        by: inviter().verifying_key().to_bytes(),
        ticket: ticket(7),
    };
    let text = link.to_string();
    let query = text.strip_prefix("id:join?").unwrap();
    let names: Vec<&str> = query
        .split('&')
        .map(|pair| pair.split_once('=').unwrap().0)
        .collect();
    assert_eq!(names, ["group", "ticket", "by"]);
}
