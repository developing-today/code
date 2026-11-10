//! Five ways to carry a signature with an iroh `EndpointTicket` (research, 2026-10-11).
//!
//! A: the standard ticket, unsigned.
//! B: our envelope, prefix `idsig`, with the standard ticket string inside and the inviter's signature.
//! C: the standard ticket, plus a signed invite event that commits to the ticket's digest.
//! D: the standard ticket with 96 bytes appended (a signature, then the signer's key).
//! E: A plus C. The ticket stays standard; a signed event is checked when one is present.
//!
//! Nothing here dials. Keys use fixed seeds, so every string is reproducible.

use data_encoding::BASE32_NOPAD;
use iroh_base::{EndpointAddr, PublicKey, SecretKey, Signature, TransportAddr};
use iroh_tickets::{ParseError, Ticket, endpoint::EndpointTicket};
use serde::{Deserialize, Serialize};

/// Domain tags, so a signature made for one option cannot be replayed as another.
const DOMAIN_ENVELOPE: &[u8] = b"id-route/v1/envelope";
const DOMAIN_SUFFIX: &[u8] = b"id-route/v1/suffix";
const DOMAIN_EVENT: &[u8] = b"id-route/v1/invite-event";
const DOMAIN_DIGEST: &[u8] = b"id-route/v1/ticket-digest";

/// Length of the suffix in option D: a 64-byte signature, then a 32-byte key.
pub const SUFFIX_LEN: usize = 64 + 32;

pub fn endpoint_key() -> SecretKey {
    SecretKey::from_bytes(&[7; 32])
}

pub fn inviter_key() -> SecretKey {
    SecretKey::from_bytes(&[0x11; 32])
}

pub fn attacker_key() -> SecretKey {
    SecretKey::from_bytes(&[0x22; 32])
}

/// An address for `endpoint_key()`: an optional relay, then the given IP addresses.
pub fn sample_addr(relay: bool, ips: &[&str]) -> EndpointAddr {
    let mut addrs = Vec::new();
    if relay {
        addrs.push(TransportAddr::Relay(
            "https://relay.example.test./".parse().unwrap(),
        ));
    }
    addrs.extend(ips.iter().map(|ip| TransportAddr::Ip(ip.parse().unwrap())));
    EndpointAddr::from_parts(endpoint_key().public(), addrs)
}

/// Base32 (no padding, lowercase) with a prefix, the same shape as an iroh ticket string.
pub fn string_with_kind(kind: &str, payload: &[u8]) -> String {
    format!("{kind}{}", BASE32_NOPAD.encode(payload).to_ascii_lowercase())
}

/// Decodes a string made by `string_with_kind`, lets `edit` change the bytes, and encodes it again.
pub fn edit_payload(text: &str, kind: &str, edit: impl FnOnce(&mut Vec<u8>)) -> String {
    let body = text.strip_prefix(kind).expect("prefix");
    let mut bytes = BASE32_NOPAD
        .decode(body.to_ascii_uppercase().as_bytes())
        .expect("base32");
    edit(&mut bytes);
    string_with_kind(kind, &bytes)
}

/// Replaces the EndpointId inside a standard ticket string, byte for byte (bytes 1..33 of the payload).
pub fn tamper_id(text: &str, new_id: &PublicKey) -> String {
    edit_payload(text, "endpoint", |bytes| {
        bytes[1..33].copy_from_slice(new_id.as_bytes())
    })
}

/// Option A: the standard ticket, exactly as iroh writes it.
pub fn option_a(addr: &EndpointAddr) -> String {
    EndpointTicket::new(addr.clone()).to_string()
}

/// Option B: our envelope. Prefix `idsig`. The inner ticket is the standard string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedTicket {
    pub ticket: EndpointTicket,
    /// The signer's key as the envelope claims it. Only a key the caller trusts counts.
    pub signer: PublicKey,
    pub signature: Signature,
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    version: u8,
    ticket: String,
    signer: PublicKey,
    signature: Signature,
}

fn envelope_message(ticket: &EndpointTicket) -> Vec<u8> {
    [DOMAIN_ENVELOPE, &ticket.encode_bytes()].concat()
}

impl SignedTicket {
    pub fn sign(ticket: EndpointTicket, key: &SecretKey) -> Self {
        let signature = key.sign(&envelope_message(&ticket));
        Self {
            ticket,
            signer: key.public(),
            signature,
        }
    }

    /// Checks the signer against a key the caller already trusts, then checks the signature.
    pub fn verify_from(&self, expected: &PublicKey) -> Result<(), ParseError> {
        if self.signer != *expected {
            return Err(ParseError::verification_failed(
                "not signed by the expected inviter",
            ));
        }
        self.signer
            .verify(&envelope_message(&self.ticket), &self.signature)
            .map_err(|_| ParseError::verification_failed("bad signature"))
    }
}

impl Ticket for SignedTicket {
    const KIND: &'static str = "idsig";

    fn encode_bytes(&self) -> Vec<u8> {
        let envelope = Envelope {
            version: 1,
            ticket: self.ticket.to_string(),
            signer: self.signer,
            signature: self.signature,
        };
        postcard::to_stdvec(&envelope).expect("postcard serialization failed")
    }

    fn decode_bytes(bytes: &[u8]) -> Result<Self, ParseError> {
        // Strict on purpose: `from_bytes` would ignore trailing bytes, as iroh's own parser does.
        let (envelope, rest): (Envelope, &[u8]) = postcard::take_from_bytes(bytes)?;
        if !rest.is_empty() || envelope.version != 1 {
            return Err(ParseError::verification_failed(
                "trailing bytes or unknown version",
            ));
        }
        Ok(Self {
            ticket: envelope.ticket.parse::<EndpointTicket>()?,
            signer: envelope.signer,
            signature: envelope.signature,
        })
    }
}

/// The verdict a client reaches for a route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteStatus {
    /// Signed by the expected key, and the signature covers this ticket.
    Verified,
    /// No signature. The route still works, but nothing vouches for it.
    Unverified,
    /// A signature is present and wrong. Do not dial this ticket.
    Rejected(&'static str),
}

/// Option C's signed invite event. `author` is the inviter's key, and the signature covers the body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteEvent {
    pub body: InviteBody,
    pub signature: Signature,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteBody {
    pub group: [u8; 32],
    pub invite_id: [u8; 16],
    pub expires_ms: u64,
    pub max_uses: u32,
    pub invite_key: [u8; 32],
    /// BLAKE3 over the ticket's canonical bytes. The string form is not used.
    pub ticket_digest: [u8; 32],
    pub author: PublicKey,
}

/// The digest of a ticket's canonical bytes. Two strings for one ticket give the same digest.
pub fn ticket_digest(ticket: &EndpointTicket) -> [u8; 32] {
    *blake3::hash(&[DOMAIN_DIGEST, &ticket.encode_bytes()].concat()).as_bytes()
}

fn event_message(body: &InviteBody) -> Vec<u8> {
    [
        DOMAIN_EVENT,
        &postcard::to_stdvec(body).expect("postcard serialization failed"),
    ]
    .concat()
}

pub fn sign_invite(body: InviteBody, key: &SecretKey) -> InviteEvent {
    let signature = key.sign(&event_message(&body));
    InviteEvent { body, signature }
}

pub fn event_bytes(event: &InviteEvent) -> Vec<u8> {
    postcard::to_stdvec(event).expect("postcard serialization failed")
}

pub fn parse_event(bytes: &[u8]) -> Result<InviteEvent, ParseError> {
    let (event, rest): (InviteEvent, &[u8]) = postcard::take_from_bytes(bytes)?;
    if !rest.is_empty() {
        return Err(ParseError::verification_failed("trailing bytes"));
    }
    Ok(event)
}

/// Checks an invite event against a ticket. `anchor` is the inviter's key, from outside the ticket.
pub fn verify_invite(
    ticket: &EndpointTicket,
    event: &InviteEvent,
    anchor: &PublicKey,
) -> RouteStatus {
    if event.body.author != *anchor {
        return RouteStatus::Rejected("event not signed by the expected inviter");
    }
    if anchor
        .verify(&event_message(&event.body), &event.signature)
        .is_err()
    {
        return RouteStatus::Rejected("bad event signature");
    }
    if event.body.ticket_digest != ticket_digest(ticket) {
        return RouteStatus::Rejected("ticket is not the one the inviter signed");
    }
    RouteStatus::Verified
}

/// Option E: A plus C. The ticket is always standard. A signed event is checked when present.
/// With no event the route is unverified, not refused: the Seitan proof still guards the join.
pub fn check_route(
    ticket_text: &str,
    event: Option<&InviteEvent>,
    anchor: &PublicKey,
) -> Result<(EndpointTicket, RouteStatus), ParseError> {
    let ticket: EndpointTicket = ticket_text.parse()?;
    let status = match event {
        Some(event) => verify_invite(&ticket, event, anchor),
        None => RouteStatus::Unverified,
    };
    Ok((ticket, status))
}

/// Option D: the standard payload, then a 64-byte signature over `DOMAIN_SUFFIX || payload`, then the 32-byte signer key.
pub fn option_d(addr: &EndpointAddr, key: &SecretKey) -> String {
    let payload = EndpointTicket::new(addr.clone()).encode_bytes();
    let signature = key.sign(&[DOMAIN_SUFFIX, &payload].concat());
    let mut bytes = payload;
    bytes.extend_from_slice(&signature.to_bytes());
    bytes.extend_from_slice(key.public().as_bytes());
    string_with_kind("endpoint", &bytes)
}

/// Mirrors iroh-tickets' private wire format (`TicketWireFormat`, endpoint.rs:34-43, 129-138).
/// Only the shape matters: the variant index, the 32-byte ID, and a counted list of addresses.
#[derive(Serialize, Deserialize)]
enum WireMirror {
    Variant1(WireMirrorV1),
}

#[derive(Serialize, Deserialize)]
struct WireMirrorV1 {
    addr: WireMirrorAddr,
}

#[derive(Serialize, Deserialize)]
struct WireMirrorAddr {
    id: PublicKey,
    info: WireMirrorInfo,
}

#[derive(Serialize, Deserialize)]
struct WireMirrorInfo {
    addrs: Vec<TransportAddr>,
}

/// A ticket string split the way D's verifier sees it.
pub struct SuffixedTicket {
    /// What a standard client gets from `str::parse::<EndpointTicket>()`.
    pub ticket: EndpointTicket,
    /// The payload bytes as received. The signature covers these, not a re-encoding.
    pub payload: Vec<u8>,
    /// Whatever followed the payload. Empty for an unsigned ticket.
    pub suffix: Vec<u8>,
}

/// Splits option D (or any `endpoint` string) using the wire length, not a re-encoding.
pub fn parse_suffixed(text: &str) -> Result<SuffixedTicket, ParseError> {
    let ticket: EndpointTicket = text.parse()?;
    let body = text
        .strip_prefix("endpoint")
        .ok_or_else(|| ParseError::wrong_prefix("endpoint"))?;
    let bytes = BASE32_NOPAD.decode(body.to_ascii_uppercase().as_bytes())?;
    let (_wire, suffix): (WireMirror, &[u8]) = postcard::take_from_bytes(&bytes)?;
    let payload_len = bytes.len() - suffix.len();
    Ok(SuffixedTicket {
        ticket,
        payload: bytes[..payload_len].to_vec(),
        suffix: suffix.to_vec(),
    })
}

/// D's verdict. No suffix is unverified. A suffix must be exactly 96 bytes, made by the expected key.
pub fn verify_suffixed(split: &SuffixedTicket, anchor: &PublicKey) -> RouteStatus {
    if split.suffix.is_empty() {
        return RouteStatus::Unverified;
    }
    if split.suffix.len() != SUFFIX_LEN {
        return RouteStatus::Rejected("trailing bytes are not a signature");
    }
    let sig_bytes: [u8; 64] = split.suffix[..64].try_into().expect("64 bytes");
    let key_bytes: [u8; 32] = split.suffix[64..].try_into().expect("32 bytes");
    let Ok(signer) = PublicKey::from_bytes(&key_bytes) else {
        return RouteStatus::Rejected("suffix key is not a curve point");
    };
    if signer != *anchor {
        return RouteStatus::Rejected("suffix not signed by the expected inviter");
    }
    let message = [DOMAIN_SUFFIX, &split.payload].concat();
    match anchor.verify(&message, &Signature::from_bytes(&sig_bytes)) {
        Ok(()) => RouteStatus::Verified,
        Err(_) => RouteStatus::Rejected("bad signature"),
    }
}
