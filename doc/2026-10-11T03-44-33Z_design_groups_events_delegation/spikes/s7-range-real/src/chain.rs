//! Real signed events: one device chain per author, with 256-byte bodies.
//!
//! Headers are p2panda-core's `Header` with the design's extension, signed by the chain's key.
//! An event ID is BLAKE3 over the header bytes, which is p2panda's operation hash.

use anyhow::{Result, anyhow, ensure};
use p2panda_core::{AnyHeader, Hash, Header, SigningKey};
use serde::{Deserialize, Serialize};

/// Events per chain, as in S1's setup, so control bytes compare directly.
pub const EVENTS_PER_CHAIN: u32 = 50;
pub const BODY_LEN: usize = 256;

/// The design's extension fields (S1's `ChainExt`), with no `after` hashes and no group.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainExt {
    pub kind: String,
    pub after: Vec<Hash>,
    pub group: Option<Hash>,
    pub domain: String,
}

fn ext() -> ChainExt {
    ChainExt {
        kind: "group.policy".into(),
        after: Vec::new(),
        group: None,
        domain: "id.events.v1".into(),
    }
}

/// Signing key for chain `i`. Deterministic, so every store built by this crate agrees.
pub fn chain_key(i: usize) -> SigningKey {
    SigningKey::from_bytes(blake3::hash(&(i as u64).to_be_bytes()).as_bytes())
}

/// The 256-byte body of event `seq` on the chain of `author`.
pub fn body(author: &[u8; 32], seq: u32) -> Vec<u8> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(author);
    hasher.update(&seq.to_be_bytes());
    let mut out = vec![0u8; BODY_LEN];
    hasher.finalize_xof().fill(&mut out);
    out
}

/// One signed event: header bytes and body bytes, as stored and as sent.
#[derive(Clone, Debug)]
pub struct Event {
    pub author: [u8; 32],
    pub seq: u32,
    pub header: Vec<u8>,
    pub body: Vec<u8>,
}

impl Event {
    pub fn id(&self) -> [u8; 32] {
        *Hash::digest(&self.header).as_bytes()
    }
}

/// Events `0..count` of the chain signed by `key`, each linked to the one before.
pub fn chain_events(key: &SigningKey, count: u32) -> Vec<Event> {
    let author = *key.verifying_key().as_bytes();
    let mut out = Vec::with_capacity(count as usize);
    let mut prev: Option<Hash> = None;
    for seq in 0..count {
        let body = body(&author, seq);
        let header = Header::<ChainExt>::builder()
            .seq_num(seq)
            .backlink(prev)
            .body(&body)
            .build(key, ext())
            .encode();
        prev = Some(Hash::digest(&header));
        out.push(Event { author, seq, header, body });
    }
    out
}

/// What a received event checked out to be.
#[derive(Debug)]
pub struct Verified {
    pub id: [u8; 32],
    pub author: [u8; 32],
    pub seq: u32,
    pub backlink: Option<[u8; 32]>,
}

/// Checks received bytes without `Header::verify`. `AnyHeader::decode` verifies the signature.
/// Then the header must re-encode to the same bytes, and the body must match `payload_size`
/// and `payload_hash`.
pub fn verify(header: &[u8], body: &[u8]) -> Result<Verified> {
    let h = AnyHeader::decode(header).map_err(|e| anyhow!("header rejected: {e:?}"))?;
    ensure!(h.encode() == header, "header is not canonical");
    ensure!(
        h.payload_size as usize == body.len(),
        "body is {} bytes, header says {}",
        body.len(),
        h.payload_size
    );
    match h.payload_hash {
        Some(hash) => ensure!(hash == Hash::digest(body), "body hash mismatch"),
        None => ensure!(body.is_empty(), "body without a payload hash"),
    }
    Ok(Verified {
        id: *Hash::digest(header).as_bytes(),
        author: *h.verifying_key.as_bytes(),
        seq: h.seq_num,
        backlink: h.backlink.map(|b| *b.as_bytes()),
    })
}

/// A stored record: u16 header length, header bytes, body bytes.
pub fn record(header: &[u8], body: &[u8]) -> Vec<u8> {
    let mut out = (header.len() as u16).to_be_bytes().to_vec();
    out.extend_from_slice(header);
    out.extend_from_slice(body);
    out
}

/// Splits a record into header and body.
pub fn split_record(record: &[u8]) -> Result<(&[u8], &[u8])> {
    ensure!(record.len() >= 2, "record shorter than its length field");
    let n = u16::from_be_bytes([record[0], record[1]]) as usize;
    ensure!(record.len() >= 2 + n, "record shorter than its header length");
    Ok((&record[2..2 + n], &record[2 + n..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_events_verify_and_link() {
        let key = chain_key(3);
        let events = chain_events(&key, 5);
        let mut prev: Option<[u8; 32]> = None;
        for ev in &events {
            let v = verify(&ev.header, &ev.body).expect("verifies");
            assert_eq!(v.id, ev.id());
            assert_eq!(v.seq, ev.seq);
            assert_eq!(v.backlink, prev);
            // The typed hash agrees with BLAKE3 over the received bytes for untouched headers.
            let typed = Header::<ChainExt>::decode(&ev.header).expect("typed decode");
            assert_eq!(typed.hash().as_bytes(), &v.id);
            prev = Some(v.id);
        }
    }

    #[test]
    fn verify_rejects_tampering() {
        let ev = &chain_events(&chain_key(1), 2)[1];
        // One bit flipped inside the 64 signature bytes. Nothing else changes, so only the
        // signature check can reject it.
        let sig = AnyHeader::decode(&ev.header).unwrap().signature.to_bytes();
        let at = ev.header.windows(64).position(|w| w == sig).expect("signature is in the header");
        let mut bad_sig = ev.header.clone();
        bad_sig[at] ^= 1;
        let err = AnyHeader::decode(&bad_sig).expect_err("AnyHeader::decode checks the signature");
        eprintln!("bit flip in signature rejected by AnyHeader::decode: {err:?}");
        assert!(verify(&bad_sig, &ev.body).is_err());
        let mut bad_body = ev.body.clone();
        bad_body[0] ^= 1;
        assert!(verify(&ev.header, &bad_body).is_err());
        assert!(verify(&ev.header, &ev.body[..10]).is_err());
    }
}
