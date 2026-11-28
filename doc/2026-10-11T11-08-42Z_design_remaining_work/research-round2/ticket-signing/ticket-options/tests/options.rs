//! Round-trip and compatibility checks for options A to E, run against iroh-tickets 1.0.0.
//! `cargo test -- --nocapture` also prints example strings and the length table.

use data_encoding::BASE32_NOPAD;
use iroh_base::{EndpointAddr, PublicKey, Signature, TransportAddr};
use iroh_tickets::{ParseError, Ticket, endpoint::EndpointTicket};
use ticket_options::*;

fn addr() -> EndpointAddr {
    sample_addr(true, &["192.0.2.10:4433"])
}

fn body(ticket: &EndpointTicket, author: PublicKey) -> InviteBody {
    InviteBody {
        group: [0x33; 32],
        invite_id: [0x44; 16],
        expires_ms: 1_800_000_000_000,
        max_uses: 1,
        invite_key: [0x55; 32],
        ticket_digest: ticket_digest(ticket),
        author,
    }
}

/// The same test as pkgs/id's `check_target` (src/envelope_outbox.rs:394-404): a ticket, else a bare node ID.
fn accepted_as_target(text: &str) -> bool {
    text.parse::<EndpointTicket>().is_ok() || text.parse::<PublicKey>().is_ok()
}

// A: the standard ticket.

#[test]
fn a_is_the_standard_string_and_round_trips() {
    let text = option_a(&addr());
    let parsed: EndpointTicket = text.parse().expect("the standard parser accepts A");
    assert_eq!(parsed.endpoint_addr(), &addr());
    assert_eq!(parsed.to_string(), text);
    assert_eq!(text.len(), 124, "S9 measured 124 for relay + one IPv4");
}

#[test]
fn a_trailing_bytes_are_silently_ignored_at_every_length() {
    let payload = EndpointTicket::new(addr()).encode_bytes();
    for n in 0..=300usize {
        let mut bytes = payload.clone();
        bytes.extend((0..n).map(|i| (i * 37 + 11) as u8));
        let text = string_with_kind("endpoint", &bytes);
        let parsed: EndpointTicket = text
            .parse()
            .unwrap_or_else(|e| panic!("{n} trailing bytes: {e}"));
        assert_eq!(parsed.endpoint_addr(), &addr(), "{n} trailing bytes");
    }
}

#[test]
fn a_swapped_endpoint_id_still_parses_so_a_plain_ticket_proves_nothing() {
    let other = attacker_key().public();
    let parsed: EndpointTicket = tamper_id(&option_a(&addr()), &other)
        .parse()
        .expect("the standard parser accepts an edited ID");
    assert_eq!(parsed.endpoint_addr().id, other);
}

// B: our envelope with a new prefix.

#[test]
fn b_round_trips_through_our_parser_and_verifies() {
    let inviter = inviter_key().public();
    let st = SignedTicket::sign(EndpointTicket::new(addr()), &inviter_key());
    let text = st.encode_string();
    let parsed = SignedTicket::decode_string(&text).expect("our parser accepts B");
    assert_eq!(parsed, st);
    parsed.verify_from(&inviter).expect("valid signature");
    println!("B string ({} chars): {text}", text.len());
}

#[test]
fn b_is_rejected_by_the_standard_parser_as_expected() {
    let text = SignedTicket::sign(EndpointTicket::new(addr()), &inviter_key()).encode_string();
    let err = text
        .parse::<EndpointTicket>()
        .expect_err("the standard parser must not accept B");
    assert!(matches!(err, ParseError::Kind { .. }), "got {err}");
    println!("standard EndpointTicket parser on B: {err}");
    println!(
        "pkgs/id check_target accepts B: {}",
        accepted_as_target(&text)
    );
    assert!(!accepted_as_target(&text));
}

#[test]
fn b_rejects_a_changed_ticket_and_a_wrong_signer() {
    let inviter = inviter_key().public();
    let st = SignedTicket::sign(EndpointTicket::new(addr()), &inviter_key());
    let changed = SignedTicket {
        ticket: EndpointTicket::new(sample_addr(false, &["192.0.2.99:4433"])),
        ..st.clone()
    };
    let err = changed.verify_from(&inviter).expect_err("changed ticket");
    assert!(err.to_string().contains("bad signature"), "got {err}");
    println!("B changed ticket: {err}");

    let other = SignedTicket::sign(EndpointTicket::new(addr()), &attacker_key());
    let err = other.verify_from(&inviter).expect_err("wrong signer");
    assert!(err.to_string().contains("not signed by the expected inviter"));
    println!("B wrong signer: {err}");
}

#[test]
fn b_rejects_trailing_bytes_that_iroh_would_ignore() {
    let st = SignedTicket::sign(EndpointTicket::new(addr()), &inviter_key());
    let mut bytes = st.encode_bytes();
    bytes.push(0);
    assert!(SignedTicket::decode_string(&string_with_kind("idsig", &bytes)).is_err());
}

// C: the standard ticket, plus a signed invite event.

#[test]
fn c_verifies_the_unchanged_standard_ticket_against_the_signed_event() {
    let inviter = inviter_key();
    let text = option_a(&addr());
    let ticket: EndpointTicket = text.parse().expect("standard");
    let event = sign_invite(body(&ticket, inviter.public()), &inviter);
    let bytes = event_bytes(&event);
    let event = parse_event(&bytes).expect("event parses");
    assert_eq!(
        verify_invite(&ticket, &event, &inviter.public()),
        RouteStatus::Verified
    );
    println!(
        "C invite event: {} bytes (stored with the chain, not pasted)",
        bytes.len()
    );
}

#[test]
fn c_rejects_a_swapped_endpoint_id_that_the_standard_parser_accepts() {
    let inviter = inviter_key();
    let ticket: EndpointTicket = option_a(&addr()).parse().unwrap();
    let event = sign_invite(body(&ticket, inviter.public()), &inviter);
    let forged: EndpointTicket = tamper_id(&option_a(&addr()), &attacker_key().public())
        .parse()
        .expect("the standard parser accepts the edited ID");
    assert_eq!(
        verify_invite(&forged, &event, &inviter.public()),
        RouteStatus::Rejected("ticket is not the one the inviter signed")
    );
}

#[test]
fn c_rejects_an_event_from_another_key_or_with_a_forged_signature() {
    let inviter = inviter_key();
    let ticket: EndpointTicket = option_a(&addr()).parse().unwrap();
    let other_author = sign_invite(body(&ticket, attacker_key().public()), &attacker_key());
    assert_eq!(
        verify_invite(&ticket, &other_author, &inviter.public()),
        RouteStatus::Rejected("event not signed by the expected inviter")
    );
    // Names the inviter as author, but the signature is the attacker's.
    let forged = sign_invite(body(&ticket, inviter.public()), &attacker_key());
    assert_eq!(
        verify_invite(&ticket, &forged, &inviter.public()),
        RouteStatus::Rejected("bad event signature")
    );
}

// D: the standard ticket with a suffix.

#[test]
fn d_is_accepted_by_the_standard_parser_with_the_same_address() {
    let text = option_d(&addr(), &inviter_key());
    assert!(text.starts_with("endpoint"), "prefix unchanged");
    let parsed: EndpointTicket = text.parse().expect("standard parser accepts D");
    assert_eq!(parsed.endpoint_addr(), &addr());
    println!("D string ({} chars): {text}", text.len());
}

#[test]
fn d_verifies_a_valid_suffix_and_rejects_each_tamper() {
    let inviter = inviter_key().public();
    let text = option_d(&addr(), &inviter_key());
    assert_eq!(
        verify_suffixed(&parse_suffixed(&text).unwrap(), &inviter),
        RouteStatus::Verified
    );

    // One bit flipped inside the signature.
    let flipped = edit_payload(&text, "endpoint", |b| {
        let i = b.len() - SUFFIX_LEN + 5;
        b[i] ^= 1;
    });
    assert_eq!(
        verify_suffixed(&parse_suffixed(&flipped).unwrap(), &inviter),
        RouteStatus::Rejected("bad signature")
    );

    // The EndpointId changed and the suffix did not. The standard parser still accepts it.
    let moved = tamper_id(&text, &attacker_key().public());
    let split = parse_suffixed(&moved).expect("the standard parser accepts the edit");
    assert_eq!(
        verify_suffixed(&split, &inviter),
        RouteStatus::Rejected("bad signature")
    );

    // A valid signature, checked against a different expected key.
    let split = parse_suffixed(&text).unwrap();
    assert_eq!(
        verify_suffixed(&split, &attacker_key().public()),
        RouteStatus::Rejected("suffix not signed by the expected inviter")
    );

    // Suffix removed: a plain ticket, which is unverified rather than refused.
    let stripped = edit_payload(&text, "endpoint", |b| b.truncate(b.len() - SUFFIX_LEN));
    assert!(stripped.parse::<EndpointTicket>().is_ok());
    assert_eq!(
        verify_suffixed(&parse_suffixed(&stripped).unwrap(), &inviter),
        RouteStatus::Unverified
    );

    // A suffix of another length is not a signature.
    let junk = edit_payload(&text, "endpoint", |b| b.push(0));
    assert_eq!(
        verify_suffixed(&parse_suffixed(&junk).unwrap(), &inviter),
        RouteStatus::Rejected("trailing bytes are not a signature")
    );
}

#[test]
fn d_is_dropped_when_a_standard_client_reads_and_writes_the_ticket() {
    let text = option_d(&addr(), &inviter_key());
    let reread = text.parse::<EndpointTicket>().unwrap().to_string();
    assert_ne!(reread, text);
    assert_eq!(reread, option_a(&addr()));
}

#[test]
fn d_split_follows_the_wire_length_because_non_canonical_payloads_parse() {
    // Hand-built wire bytes: the same relay address listed twice. iroh's BTreeSet keeps one.
    let relay = TransportAddr::Relay("https://relay.example.test./".parse().unwrap());
    let mut payload = vec![0x00]; // variant index of Variant1
    payload.extend(postcard::to_stdvec(&endpoint_key().public()).unwrap());
    payload.extend(postcard::to_stdvec(&2usize).unwrap());
    let entry = postcard::to_stdvec(&relay).unwrap();
    payload.extend(&entry);
    payload.extend(&entry);
    let suffix = [0xAB; SUFFIX_LEN];
    let text = string_with_kind("endpoint", &[payload.clone(), suffix.to_vec()].concat());

    let parsed: EndpointTicket = text
        .parse()
        .expect("the standard parser accepts the duplicate");
    assert_eq!(parsed.endpoint_addr(), &sample_addr(true, &[]));

    let split = parse_suffixed(&text).unwrap();
    assert_eq!(split.payload, payload);
    assert_eq!(split.suffix, suffix);

    // Re-encoding the parsed ticket gives a shorter payload, so a split by re-encoding is wrong.
    let reencoded = parsed.encode_bytes().len();
    println!(
        "wire payload {} bytes, canonical re-encoding {} bytes",
        payload.len(),
        reencoded
    );
    assert_ne!(reencoded, payload.len());
}

#[test]
fn a_signature_made_for_option_d_is_refused_as_option_b() {
    // D signs DOMAIN_SUFFIX || payload. Reusing that signature as B's must fail.
    let text = option_d(&addr(), &inviter_key());
    let split = parse_suffixed(&text).unwrap();
    let sig_bytes: [u8; 64] = split.suffix[..64].try_into().unwrap();
    let reused = SignedTicket {
        ticket: EndpointTicket::new(addr()),
        signer: inviter_key().public(),
        signature: Signature::from_bytes(&sig_bytes),
    };
    assert!(reused.verify_from(&inviter_key().public()).is_err());
}

// E: A plus C, checked automatically when present.

#[test]
fn e_a_plain_ticket_is_accepted_and_unverified_not_refused() {
    let (ticket, status) =
        check_route(&option_a(&addr()), None, &inviter_key().public()).unwrap();
    assert_eq!(status, RouteStatus::Unverified);
    assert_eq!(ticket.endpoint_addr(), &addr());
}

#[test]
fn e_a_valid_event_verifies_and_a_mismatched_one_is_rejected() {
    let inviter = inviter_key();
    let text = option_a(&addr());
    let ticket: EndpointTicket = text.parse().unwrap();
    let event = sign_invite(body(&ticket, inviter.public()), &inviter);

    let (_, status) = check_route(&text, Some(&event), &inviter.public()).unwrap();
    assert_eq!(status, RouteStatus::Verified);

    let edited = tamper_id(&text, &attacker_key().public());
    let (_, status) = check_route(&edited, Some(&event), &inviter.public()).unwrap();
    assert_eq!(
        status,
        RouteStatus::Rejected("ticket is not the one the inviter signed")
    );
}

// Paste rules. The parser is shared by every option, so this is printed once for all.

#[test]
fn paste_rules_the_parser_applies_to_the_pasted_string() {
    let text = option_a(&addr());
    let upper_body = format!("endpoint{}", text["endpoint".len()..].to_ascii_uppercase());
    let cases = [
        ("exact copy", text.clone()),
        ("trailing newline", format!("{text}\n")),
        ("trailing space", format!("{text} ")),
        ("leading space", format!(" {text}")),
        ("upper-case prefix", text.replacen("endpoint", "ENDPOINT", 1)),
        ("upper-case body", upper_body.clone()),
    ];
    for (label, candidate) in &cases {
        match candidate.parse::<EndpointTicket>() {
            Ok(_) => println!("paste {label}: accepted"),
            Err(e) => println!("paste {label}: rejected: {e}"),
        }
    }
    assert!(text.parse::<EndpointTicket>().is_ok());
    // Same ticket, different string: why a digest must cover the canonical bytes, not the text.
    assert_eq!(
        upper_body.parse::<EndpointTicket>().unwrap(),
        text.parse::<EndpointTicket>().unwrap()
    );
    // The same rules apply to our own prefix, because both use Ticket::decode_string.
    let b = SignedTicket::sign(EndpointTicket::new(addr()), &inviter_key()).encode_string();
    let b_upper = format!("idsig{}", b["idsig".len()..].to_ascii_uppercase());
    println!("paste B upper-case body: {:?}", SignedTicket::decode_string(&b_upper).is_ok());
    println!(
        "paste B trailing newline: {:?}",
        SignedTicket::decode_string(&format!("{b}\n")).is_ok()
    );
    let d = option_d(&addr(), &inviter_key());
    println!(
        "paste D trailing newline: {:?}",
        format!("{d}\n").parse::<EndpointTicket>().is_ok()
    );
}

// Lengths and examples. Run with --nocapture.

#[test]
fn print_lengths_and_examples() {
    let inviter = inviter_key();
    let event = sign_invite(
        body(&EndpointTicket::new(addr()), inviter.public()),
        &inviter,
    );
    println!("C invite event: {} bytes", event_bytes(&event).len());
    println!(
        "C event inline in a link, base32: {} chars",
        BASE32_NOPAD.encode_len(event_bytes(&event).len())
    );
    println!(
        "one 32-byte key as base32 (link trust anchor): {} chars",
        BASE32_NOPAD.encode_len(32)
    );
    println!("A example: {}", option_a(&addr()));
    println!("D example: {}", option_d(&addr(), &inviter_key()));

    let sets: [(&str, EndpointAddr); 3] = [
        ("relay only", sample_addr(true, &[])),
        ("relay + 1 IPv4", sample_addr(true, &["192.0.2.10:4433"])),
        (
            "relay + 2 IPv4 + 1 IPv6",
            sample_addr(
                true,
                &["192.0.2.10:4433", "198.51.100.7:4433", "[2001:db8::7]:4433"],
            ),
        ),
    ];
    println!(
        "| address set | A (and C, E pasted) | B string inside | B payload inside (computed) | D sig + key | D sig only (computed) |"
    );
    println!("|---|---|---|---|---|---|");
    for (label, addr) in sets {
        let ticket = EndpointTicket::new(addr.clone());
        let payload = ticket.encode_bytes();
        let a = option_a(&addr);
        let st = SignedTicket::sign(ticket, &inviter);
        let b = st.encode_string();
        let compact =
            postcard::to_stdvec(&(1u8, payload.clone(), inviter.public(), st.signature)).unwrap();
        let b_compact = "idsig".len() + BASE32_NOPAD.encode_len(compact.len());
        let d = option_d(&addr, &inviter_key());
        let d_sig_only = "endpoint".len() + BASE32_NOPAD.encode_len(payload.len() + 64);
        println!(
            "| {label} (payload {} bytes) | {} | {} | {} | {} | {} |",
            payload.len(),
            a.len(),
            b.len(),
            b_compact,
            d.len(),
            d_sig_only
        );
    }
}
