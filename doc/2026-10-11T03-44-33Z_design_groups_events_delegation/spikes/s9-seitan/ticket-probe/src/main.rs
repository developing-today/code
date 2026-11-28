//! Probe: what an iroh EndpointTicket carries, and whether a Seitan token can ride inside one.
//! Uses iroh-tickets 1.0.0 and iroh-base 1.3.0, as pinned in the spike lock. Nothing is dialled.

use std::{net::SocketAddr, str::FromStr};

use data_encoding::BASE32_NOPAD;
use iroh_base::{EndpointAddr, RelayUrl, SecretKey, TransportAddr};
use iroh_tickets::{Ticket, endpoint::EndpointTicket};

const TOKEN: &str = "4uywza+b3cga7rd6yc";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn main() {
    // A fixed key: no RNG needed. A documentation-range IP, and a relay host that is never contacted.
    let id = SecretKey::from_bytes(&[7u8; 32]).public();
    let relay: RelayUrl = "https://relay.example.test./".parse().unwrap();
    let ip: SocketAddr = "192.0.2.10:4433".parse().unwrap();
    let addr = EndpointAddr::from_parts(id, [TransportAddr::Relay(relay), TransportAddr::Ip(ip)]);
    let ticket = EndpointTicket::new(addr.clone());

    let text = ticket.to_string();
    println!("1. ticket string ({} chars): {text}", text.len());
    let payload = ticket.encode_bytes();
    println!("   postcard payload ({} bytes): {}", payload.len(), hex(&payload));
    println!("   layout (iroh-tickets endpoint.rs): variant 00 | EndpointId 32 bytes | addr count | addrs");
    println!("   fields: no signature, no expiry, no extension field");

    let parsed = EndpointTicket::from_str(&text).unwrap();
    println!("2. round trip equal: {}", parsed == ticket);
    println!("   leaks: id={}", parsed.endpoint_addr().id);
    for transport in parsed.endpoint_addr().addrs.iter() {
        println!("          transport {transport:?}");
    }

    // Try to carry the token: append its bytes after the postcard payload, then encode as a ticket string.
    let mut smuggled = payload.clone();
    smuggled.extend_from_slice(TOKEN.as_bytes());
    let smuggled_text = format!("endpoint{}", BASE32_NOPAD.encode(&smuggled).to_ascii_lowercase());
    let smuggled_parsed = EndpointTicket::from_str(&smuggled_text);
    println!("3. token appended after the payload ({} bytes):", smuggled.len());
    match &smuggled_parsed {
        Ok(t) => println!(
            "   parses: true  addr unchanged: {}  token recovered by any API: no (no accessor exists)",
            t.endpoint_addr() == ticket.endpoint_addr()
        ),
        Err(e) => println!("   parse error: {e}"),
    }

    // An unsigned ticket can be edited: swap the EndpointId and the string still parses.
    let other_id = SecretKey::from_bytes(&[8u8; 32]).public();
    let mut edited = payload.clone();
    edited[1..33].copy_from_slice(other_id.as_bytes());
    let edited_text = format!("endpoint{}", BASE32_NOPAD.encode(&edited).to_ascii_lowercase());
    let edited_parsed = EndpointTicket::from_str(&edited_text).unwrap();
    println!("4. EndpointId swapped inside the string: parses = true, id now {}", edited_parsed.endpoint_addr().id);

    // Two parses of the same string give the same address: nothing is consumed.
    let again = EndpointTicket::from_str(&text).unwrap();
    println!("5. parsed twice, same address: {}", again == parsed);
}
