//! First step of the #9106 walking skeleton (#12232): a signed record,
//! named by an `id://` address, checked by a reader that holds only the
//! address and the bytes.
//!
//! Every other substrate test includes its module by `#[path]`, so it
//! tests a private copy. This one imports from `id_core` and nothing
//! else: it fails to compile if `signed_envelope`, `hash_profile`,
//! `id_uri_parse`, `id_uri_format` or the `graph` children stop being
//! part of the crate, and it fails to pass if any of them is stubbed,
//! because each step consumes the previous step's output.
//!
//! The path:
//!
//! 1. The writer signs a graph record. Its identity is the envelope ID.
//! 2. The writer names the record with the one canonical
//!    `id://<hint>/<namespace>/v1/content/raw-blake3-256.<digest>` text.
//! 3. The reader parses that text, checks the resolver hint against the
//!    resolver it selected, and gets the content ID back.
//! 4. The reader verifies the bytes it was given against the address.
//! 5. The reader decodes the envelope, verifies the signature and reads
//!    the triples, with the same record ID as provenance.
//!
//! Storing the bytes, syncing them and reporting a conflicting write are
//! the next slices of #12232; nothing here touches a store or a network.
//!
//! Oracles: differential (the reader's record equals the writer's),
//! metamorphic (any single-byte change to the bytes, and any other
//! signer, must change the outcome), and a reference digest computed
//! here with `blake3` directly rather than through the code under test.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ed25519_dalek::SigningKey;
use id_core::graph::record::{GraphRecord, OpenedRecord, RECORD_ID_PROFILE, RecordError, open};
use id_core::graph::term::{Term, Triple};
use id_core::hash_profile::{ProfileDispatch, VerifyError};
use id_core::id_shape::{ContentId, HashProfile};
use id_core::id_uri_format::resolve_context;
use id_core::id_uri_parse::{IdUriV1, UriParseError, UriTarget};
use id_core::signed_envelope::{Envelope, EnvelopeId, LogicalGraphId};

const RESOLVER: &str = "node-b.example";
const NAMESPACE: &str = "notes";
const CONTEXT: &str = "namespace/notes";

fn writer_key() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}

fn graph() -> LogicalGraphId {
    LogicalGraphId::new([0x11; 32])
}

fn note(text: &[u8]) -> GraphRecord {
    GraphRecord::new(
        graph(),
        vec![Triple::new(
            Term::typed(1, b"urn:note:1".to_vec()),
            Term::typed(2, b"body".to_vec()),
            Term::typed(3, text.to_vec()),
        )],
    )
}

/// What the writer hands over: the address text and the envelope bytes.
struct Published {
    address: String,
    bytes: Vec<u8>,
    signed: OpenedRecord,
}

/// Steps 1 and 2: sign, then name the record by its envelope ID.
fn publish(record: &GraphRecord, key: &SigningKey) -> Published {
    let signed = record.sign(CONTEXT, "node-a", key).unwrap();
    let bytes = signed.envelope.canonical_bytes().unwrap();
    let address = IdUriV1::new(
        RESOLVER,
        NAMESPACE,
        UriTarget::Content(*signed.id.content_id()),
    )
    .unwrap()
    .format();
    Published {
        address,
        bytes,
        signed,
    }
}

/// Why the reader refused what it was given. The inner errors are read
/// only by the failure messages below.
#[derive(Debug)]
#[allow(dead_code)]
enum Refused {
    Address(UriParseError),
    NotARecordAddress,
    Bytes(VerifyError),
    Envelope(String),
    Record(RecordError),
    AddressNamesAnotherRecord,
}

/// Steps 3 to 5, as a reader that trusts nothing but its own resolver
/// choice: the address text and the bytes both come from outside.
fn read(address: &str, bytes: &[u8], selected_resolver: &str) -> Result<OpenedRecord, Refused> {
    let dispatch = ProfileDispatch::default();
    let uri = IdUriV1::parse(address).map_err(Refused::Address)?;
    resolve_context(uri.resolver_hint(), selected_resolver).map_err(Refused::Address)?;
    let UriTarget::Content(named) = uri.target() else {
        return Err(Refused::NotARecordAddress);
    };
    dispatch
        .verify_bytes(named, bytes)
        .map_err(Refused::Bytes)?;
    let envelope = Envelope::decode(bytes).map_err(|e| Refused::Envelope(e.to_string()))?;
    let opened = open(envelope, &dispatch).map_err(Refused::Record)?;
    if opened.id.content_id() != named {
        return Err(Refused::AddressNamesAnotherRecord);
    }
    Ok(opened)
}

#[test]
fn a_signed_record_is_read_back_from_its_address_and_bytes() {
    let published = publish(&note(b"first draft"), &writer_key());

    let opened = read(&published.address, &published.bytes, RESOLVER).unwrap();

    // Differential: the reader holds exactly what the writer signed.
    assert_eq!(opened, published.signed);
    assert_eq!(
        opened.envelope.body.key,
        writer_key().verifying_key().to_bytes()
    );
    let assertions: Vec<_> = opened.assertions().collect();
    assert_eq!(assertions.len(), 1);
    assert_eq!(assertions[0].record, published.signed.id);
    assert_eq!(assertions[0].graph, graph());
    assert_eq!(assertions[0].triple, &note(b"first draft").triples[0]);
}

#[test]
fn the_address_is_the_blake3_of_the_envelope_bytes() {
    // Reference oracle: the digest in the address is recomputed here with
    // `blake3` itself, not with the profile dispatch under test.
    let published = publish(&note(b"first draft"), &writer_key());
    let digest = blake3::hash(&published.bytes);
    assert_eq!(RECORD_ID_PROFILE, HashProfile::RawBlake3_256);
    assert_eq!(
        published.address,
        format!(
            "id://{RESOLVER}/{NAMESPACE}/v1/content/raw-blake3-256.{}",
            digest.to_hex()
        )
    );
    // The address text has one spelling: parsing and formatting it again
    // returns the same text, and it names the writer's envelope ID.
    let parsed = IdUriV1::parse(&published.address).unwrap();
    assert_eq!(parsed.format(), published.address);
    assert_eq!(
        parsed.target(),
        &UriTarget::Content(ContentId::new(
            HashProfile::RawBlake3_256,
            *digest.as_bytes()
        ))
    );
    assert_eq!(
        EnvelopeId::from_content_id(ContentId::new(
            HashProfile::RawBlake3_256,
            *digest.as_bytes()
        )),
        published.signed.id
    );
}

#[test]
fn any_changed_byte_is_refused_before_the_envelope_is_decoded() {
    // Metamorphic: flipping any one bit of the bytes changes the outcome
    // from "read" to "refused at the address check". Every byte position
    // is tried, so a field the address did not cover would show up here.
    let published = publish(&note(b"first draft"), &writer_key());
    for position in 0..published.bytes.len() {
        let mut changed = published.bytes.clone();
        changed[position] ^= 0x01;
        match read(&published.address, &changed, RESOLVER) {
            Err(Refused::Bytes(_)) => {}
            other => panic!("byte {position}: expected a refusal at the address, got {other:?}"),
        }
    }
    // Truncation and extension are refused the same way.
    let short = &published.bytes[..published.bytes.len() - 1];
    assert!(matches!(
        read(&published.address, short, RESOLVER),
        Err(Refused::Bytes(_))
    ));
    let mut long = published.bytes.clone();
    long.push(0);
    assert!(matches!(
        read(&published.address, &long, RESOLVER),
        Err(Refused::Bytes(_))
    ));
}

#[test]
fn another_signer_or_another_payload_is_another_address() {
    let first = publish(&note(b"first draft"), &writer_key());
    let other_key = publish(&note(b"first draft"), &SigningKey::from_bytes(&[8; 32]));
    let other_text = publish(&note(b"second draft"), &writer_key());

    assert_ne!(first.address, other_key.address);
    assert_ne!(first.address, other_text.address);
    assert_ne!(other_key.address, other_text.address);

    // Each address reads only its own bytes. Bytes that are a valid
    // signed record, offered under another record's address, are refused.
    for (address, bytes) in [
        (&first.address, &other_key.bytes),
        (&first.address, &other_text.bytes),
        (&other_key.address, &first.bytes),
        (&other_text.address, &first.bytes),
    ] {
        assert!(matches!(
            read(address, bytes, RESOLVER),
            Err(Refused::Bytes(_))
        ));
    }
    for published in [&first, &other_key, &other_text] {
        assert_eq!(
            read(&published.address, &published.bytes, RESOLVER).unwrap(),
            published.signed
        );
    }
    // Signing the same record with the same key is deterministic: the
    // same address and the same bytes, so a re-publish is not a new record.
    let again = publish(&note(b"first draft"), &writer_key());
    assert_eq!(again.address, first.address);
    assert_eq!(again.bytes, first.bytes);
}

#[test]
fn the_address_is_only_followed_under_the_selected_resolver() {
    let published = publish(&note(b"first draft"), &writer_key());

    // The authority is a hint. A reader that selected another resolver
    // refuses the address instead of letting the text pick one for it.
    match read(&published.address, &published.bytes, "node-c.example") {
        Err(Refused::Address(e)) => assert_eq!(e, UriParseError::ResolverContextMismatch),
        other => panic!("expected a resolver mismatch, got {other:?}"),
    }

    // An address of another version or shape is refused by class, not
    // read as something else.
    let digest = published.address.rsplit('.').next().unwrap().to_owned();
    for (text, class) in [
        (
            format!("id://{RESOLVER}/{NAMESPACE}/v2/content/raw-blake3-256.{digest}"),
            UriParseError::UnsupportedCriticalVersion.class(),
        ),
        (
            format!("id://{RESOLVER}/{NAMESPACE}/v1/record/raw-blake3-256.{digest}"),
            UriParseError::UnknownCriticalShape.class(),
        ),
    ] {
        match read(&text, &published.bytes, RESOLVER) {
            Err(Refused::Address(e)) => assert_eq!(e.class(), class, "{text}"),
            other => panic!("{text}: expected an address refusal, got {other:?}"),
        }
    }

    // An array address is a valid address, and it does not name a record.
    let array = format!("id://{RESOLVER}/{NAMESPACE}/v1/array");
    assert!(matches!(
        read(&array, &published.bytes, RESOLVER),
        Err(Refused::NotARecordAddress)
    ));
}

#[test]
fn bytes_that_match_their_address_but_are_not_a_record_are_refused() {
    // The address check alone proves the bytes are the named bytes, not
    // that they are a signed record. Arbitrary bytes under their own
    // correct address must still be refused, at the envelope.
    let junk = b"not an envelope".to_vec();
    let address = IdUriV1::new(
        RESOLVER,
        NAMESPACE,
        UriTarget::Content(ContentId::new(
            HashProfile::RawBlake3_256,
            *blake3::hash(&junk).as_bytes(),
        )),
    )
    .unwrap()
    .format();
    assert!(matches!(
        read(&address, &junk, RESOLVER),
        Err(Refused::Envelope(_))
    ));

    // A real envelope whose signature was replaced, re-addressed so the
    // address check passes, is refused at the signature.
    let published = publish(&note(b"first draft"), &writer_key());
    let mut forged = Envelope::decode(&published.bytes).unwrap();
    forged.signature[0] ^= 0x01;
    let forged_bytes = forged.canonical_bytes().unwrap();
    let forged_address = IdUriV1::new(
        RESOLVER,
        NAMESPACE,
        UriTarget::Content(ContentId::new(
            HashProfile::RawBlake3_256,
            *blake3::hash(&forged_bytes).as_bytes(),
        )),
    )
    .unwrap()
    .format();
    assert_ne!(forged_address, published.address);
    assert!(matches!(
        read(&forged_address, &forged_bytes, RESOLVER),
        Err(Refused::Record(_))
    ));
}
