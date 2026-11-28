//! Chained-message relay envelope and hop records: phase 3 of the #2036
//! messaging/relay protocol v1 (#3796), part A — the type layer only,
//! per the design record on #2036 (`doc/2026-11-01_design_id2036.md`,
//! §4.1, and invariants 1–3 in §5).
//!
//! A chained message is an origin-signed immutable envelope: origin ID,
//! group ID, origin sequence, predecessor references, class, and either
//! bounded inline content or a content hash/blob reference. Its stable
//! message ID is the BLAKE3 hash of the complete canonical signed
//! envelope. Relaying never changes the envelope, so the same envelope
//! verifies to the same message ID over a direct path or any number of
//! relay paths.
//!
//! Each relay may append a [`HopRecord`] — `(relay_id, previous_hop_id,
//! received_at)`, self-signed by the relay and bound by that signature
//! to the message ID. Hop records live OUTSIDE the envelope, in a
//! [`RelayedMessage`]'s hop chain: they are audit and loop-control
//! metadata only. A hop record cannot make an invalid origin message
//! valid, and its absence, alteration, or corruption does not change
//! the origin envelope's validity or message ID —
//! [`RelayedMessage::verify_envelope`] never looks at the chain. The
//! chain has its own check, [`check_hop_chain`], which the part B relay
//! gate calls before forwarding; nothing in this module forwards
//! anything, and membership/ingest authorization is part B's gate,
//! built on these types.
//!
//! The pure policy helpers are [`DedupeSet`] (a node forwards at most
//! once per message ID; different signed envelopes — forks — never
//! collapse under dedupe), loop/self-hop detection
//! ([`chain_contains_relay`], [`relay_refusal`]), and the four-hop
//! default cap ([`DEFAULT_MAX_HOPS`]) enforced by [`check_hop_chain`]
//! and [`RelayedMessage::append_hop`].

use std::collections::HashSet;

use anyhow::{Context, Result, bail};
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use unicode_normalization::is_nfc;

use crate::hex::{hex_encode, hex_to_array};

/// The event kind a chained message carries when it enters the durable
/// graph. Part B wires this to ingest; part A only names it, following
/// the `session.checkpoint` precedent in id-store.
pub const CHAINED_MESSAGE_KIND: &str = "chained.message";

/// The default relay cap: a member stops forwarding at four hops
/// (#2036 §4.1). A default, not a protocol constant — a chain check
/// may be given another cap.
pub const DEFAULT_MAX_HOPS: usize = 4;

/// The most hops one chain may carry under any cap. A chain longer
/// than this is malformed, not merely over a policy cap.
pub const MAX_HOPS: usize = 64;

/// The most bytes of inline content one envelope may carry. Larger
/// payloads travel as a [`ChainedContent::Blob`] reference, never as
/// unbounded inline bytes (#2036 §4.5, invariant 10).
pub const MAX_INLINE_CONTENT_BYTES: usize = 16_384;

/// The most predecessor references one envelope may name.
pub const MAX_PREDECESSORS: usize = 32;

/// The most bytes of text in a group ID or class name.
pub const MAX_TEXT_BYTES: usize = 128;

/// A message ID: the BLAKE3 hash of the canonical signed envelope, as
/// lower-case hex. Hop records are never an input to it.
pub type MessageId = String;

/// What a chained message carries: bounded inline bytes, or a
/// content-addressed blob reference.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainedContent {
    /// Bounded inline content, at most [`MAX_INLINE_CONTENT_BYTES`].
    Inline {
        /// The content bytes.
        bytes: Vec<u8>,
    },
    /// A blob held elsewhere, named by its BLAKE3 hash (lower-case
    /// hex) and size in bytes.
    Blob {
        /// BLAKE3 hash of the blob, 64 lower-case hex characters.
        hash: String,
        /// The blob's size in bytes.
        size: u64,
    },
}

/// An origin-signed immutable chained-message envelope. Anyone holding
/// it can check the signature against the origin's key without asking
/// a server. The envelope has no hop field by construction: hops
/// cannot enter the signed body or the message ID.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainedEnvelope {
    /// The origin's account ID: the hex of its Ed25519 public key,
    /// which is also the key the signature verifies against.
    pub origin: String,
    /// The group the message belongs to.
    pub group: String,
    /// The origin's sequence number for this message within the group.
    pub sequence: u64,
    /// Message IDs this message causally follows. None may repeat,
    /// and none may be this message's own ID (checked in
    /// [`ChainedEnvelope::verify`]).
    pub predecessors: Vec<MessageId>,
    /// The message class, as its author wrote it (e.g. `durable`):
    /// non-empty, NFC, at most [`MAX_TEXT_BYTES`] bytes.
    pub class: String,
    /// The carried content or blob reference.
    pub content: ChainedContent,
    /// When the origin made it, in Unix milliseconds.
    pub at: u64,
    /// Hex Ed25519 signature over the canonical body (every field
    /// above, in declaration order).
    pub signature: String,
}

/// The canonical envelope body the origin signature covers.
#[derive(Serialize)]
pub struct ChainedSignedBody<'a> {
    origin: &'a str,
    group: &'a str,
    sequence: u64,
    predecessors: &'a [MessageId],
    class: &'a str,
    content: &'a ChainedContent,
    at: u64,
}

impl ChainedEnvelope {
    /// The canonical body the signature covers.
    pub fn body(&self) -> ChainedSignedBody<'_> {
        ChainedSignedBody {
            origin: &self.origin,
            group: &self.group,
            sequence: self.sequence,
            predecessors: &self.predecessors,
            class: &self.class,
            content: &self.content,
            at: self.at,
        }
    }

    /// Sign an envelope with the origin's `key`. The origin ID is
    /// derived from the key, so an envelope cannot name an origin
    /// other than its signer.
    ///
    /// # Errors
    ///
    /// Fails if the fields are structurally invalid (see
    /// [`ChainedEnvelope::validate`]) or the body does not encode.
    pub fn signed(
        key: &SigningKey,
        group: &str,
        sequence: u64,
        predecessors: Vec<MessageId>,
        class: &str,
        content: ChainedContent,
        at: u64,
    ) -> Result<Self> {
        let mut envelope = Self {
            origin: hex_encode(&key.verifying_key().to_bytes()),
            group: group.to_owned(),
            sequence,
            predecessors,
            class: class.to_owned(),
            content,
            at,
            signature: String::new(),
        };
        envelope.validate()?;
        let signature = key.sign(&serde_json::to_vec(&envelope.body())?);
        envelope.signature = hex_encode(&signature.to_bytes());
        if envelope.predecessors.contains(&envelope.message_id()?) {
            bail!("a chained message may not name itself as a predecessor");
        }
        Ok(envelope)
    }

    /// The stable message ID: BLAKE3 over the canonical signed
    /// envelope (body plus signature, in declaration order). Hop
    /// records are not part of the envelope and never feed this hash.
    ///
    /// # Errors
    ///
    /// Fails only if the envelope does not encode.
    pub fn message_id(&self) -> Result<MessageId> {
        let canonical = serde_json::to_vec(self)?;
        Ok(hex_encode(blake3::hash(&canonical).as_bytes()))
    }

    /// Structural validity, without the signature: origin is a key,
    /// text fields are bounded NFC, predecessors are unique
    /// message-ID-shaped references within the count bound, and the
    /// content is within its bound.
    ///
    /// # Errors
    ///
    /// Fails with the first structural defect found.
    pub fn validate(&self) -> Result<()> {
        let origin_bytes =
            hex_to_array::<32>(&self.origin).context("malformed chained-message origin")?;
        VerifyingKey::from_bytes(&origin_bytes).context("chained-message origin is not a key")?;
        validate_text("group", &self.group)?;
        validate_text("class", &self.class)?;
        if self.predecessors.len() > MAX_PREDECESSORS {
            bail!("too many predecessors on a chained message");
        }
        let mut seen = HashSet::new();
        for predecessor in &self.predecessors {
            if hex_to_array::<32>(predecessor).is_none() {
                bail!("malformed predecessor message ID on a chained message");
            }
            if !seen.insert(predecessor) {
                bail!("a chained message names the same predecessor twice");
            }
        }
        match &self.content {
            ChainedContent::Inline { bytes } => {
                if bytes.len() > MAX_INLINE_CONTENT_BYTES {
                    bail!("inline chained-message content is over the bound");
                }
            }
            ChainedContent::Blob { hash, .. } => {
                if hex_to_array::<32>(hash).is_none() {
                    bail!("malformed blob hash on a chained message");
                }
            }
        }
        Ok(())
    }

    /// Full validity: structure plus the origin signature over the
    /// canonical body, checked against the origin's own key. This is
    /// the validate-before-forward check part B's gate builds on; it
    /// consults no hop record.
    ///
    /// # Errors
    ///
    /// Fails if the envelope is structurally invalid, names itself as
    /// a predecessor, or its signature does not verify.
    pub fn verify(&self) -> Result<()> {
        self.validate()?;
        if self.predecessors.contains(&self.message_id()?) {
            bail!("a chained message may not name itself as a predecessor");
        }
        let key = VerifyingKey::from_bytes(
            &hex_to_array::<32>(&self.origin).context("malformed chained-message origin")?,
        )
        .context("chained-message origin is not a key")?;
        let signature = Signature::from_bytes(
            &hex_to_array::<64>(&self.signature).context("malformed chained-message signature")?,
        );
        let body = serde_json::to_vec(&self.body())?;
        key.verify_strict(&body, &signature)
            .context("chained-message signature does not verify")
    }
}

/// One relay's self-signed hop record: audit and loop-control
/// metadata that lives outside the envelope. The signature binds the
/// record to one message ID, so a hop record cannot be transplanted
/// onto another message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HopRecord {
    /// The relay's account ID: the hex of its Ed25519 public key,
    /// which is also the key the signature verifies against.
    pub relay: String,
    /// The previous hop's relay ID, or `None` for the first hop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_hop: Option<String>,
    /// When the relay received the message, in Unix milliseconds.
    pub received_at: u64,
    /// Hex Ed25519 signature over the canonical hop body (message ID,
    /// relay, previous hop, receipt time).
    pub signature: String,
}

/// The canonical hop body the relay signature covers.
#[derive(Serialize)]
pub struct HopSignedBody<'a> {
    message_id: &'a str,
    relay: &'a str,
    previous_hop: Option<&'a str>,
    received_at: u64,
}

impl HopRecord {
    /// The canonical body the signature covers, for `message_id`.
    pub fn body<'a>(&'a self, message_id: &'a str) -> HopSignedBody<'a> {
        HopSignedBody {
            message_id,
            relay: &self.relay,
            previous_hop: self.previous_hop.as_deref(),
            received_at: self.received_at,
        }
    }

    /// Sign a hop record with the relay's `key` for `message_id`. The
    /// relay ID is derived from the key, so a record cannot name a
    /// relay other than its signer.
    ///
    /// # Errors
    ///
    /// Fails if `message_id` is malformed, `previous_hop` is the
    /// relay itself (a self-hop), or the body does not encode.
    pub fn signed(
        key: &SigningKey,
        message_id: &str,
        previous_hop: Option<&str>,
        received_at: u64,
    ) -> Result<Self> {
        let relay = hex_encode(&key.verifying_key().to_bytes());
        if previous_hop == Some(relay.as_str()) {
            bail!("a hop record may not name its own relay as the previous hop");
        }
        let mut record = Self {
            relay,
            previous_hop: previous_hop.map(str::to_owned),
            received_at,
            signature: String::new(),
        };
        record.validate(message_id)?;
        let signature = key.sign(&serde_json::to_vec(&record.body(message_id))?);
        record.signature = hex_encode(&signature.to_bytes());
        Ok(record)
    }

    /// Structural validity for a hop on `message_id`, without the
    /// signature: message ID and relay are well-formed, and the
    /// previous hop is neither malformed nor the relay itself.
    ///
    /// # Errors
    ///
    /// Fails with the first structural defect found.
    pub fn validate(&self, message_id: &str) -> Result<()> {
        if hex_to_array::<32>(message_id).is_none() {
            bail!("malformed message ID for a hop record");
        }
        let relay_bytes = hex_to_array::<32>(&self.relay).context("malformed hop-record relay")?;
        VerifyingKey::from_bytes(&relay_bytes).context("hop-record relay is not a key")?;
        if let Some(previous) = &self.previous_hop {
            if hex_to_array::<32>(previous).is_none() {
                bail!("malformed previous hop on a hop record");
            }
            if previous == &self.relay {
                bail!("a hop record names its own relay as the previous hop (self-hop)");
            }
        }
        Ok(())
    }

    /// Full validity for a hop on `message_id`: structure plus the
    /// relay signature over the canonical hop body.
    ///
    /// # Errors
    ///
    /// Fails if the record is structurally invalid or its signature
    /// does not verify for `message_id`.
    pub fn verify(&self, message_id: &str) -> Result<()> {
        self.validate(message_id)?;
        let key = VerifyingKey::from_bytes(
            &hex_to_array::<32>(&self.relay).context("malformed hop-record relay")?,
        )
        .context("hop-record relay is not a key")?;
        let signature = Signature::from_bytes(
            &hex_to_array::<64>(&self.signature).context("malformed hop-record signature")?,
        );
        let body = serde_json::to_vec(&self.body(message_id))?;
        key.verify_strict(&body, &signature)
            .context("hop-record signature does not verify")
    }
}

/// An envelope in transit: the immutable origin envelope plus the hop
/// chain relays have appended. The envelope is the validity input;
/// the chain is audit/loop metadata only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayedMessage {
    /// The origin-signed envelope, unchanged since the origin sent it.
    pub envelope: ChainedEnvelope,
    /// Hop records in relay order, first hop first. May be empty (a
    /// direct delivery) and may be stripped or corrupt without
    /// affecting the envelope — see [`RelayedMessage::verify_envelope`].
    pub hops: Vec<HopRecord>,
}

impl RelayedMessage {
    /// A direct delivery: the envelope with no hops yet.
    pub fn new(envelope: ChainedEnvelope) -> Self {
        Self {
            envelope,
            hops: Vec::new(),
        }
    }

    /// The message ID of the carried envelope.
    ///
    /// # Errors
    ///
    /// Fails only if the envelope does not encode.
    pub fn message_id(&self) -> Result<MessageId> {
        self.envelope.message_id()
    }

    /// Verify the origin envelope alone. Hop records are never
    /// consulted: a stripped, altered, or corrupt chain cannot change
    /// this outcome (#2036 §4.1).
    ///
    /// # Errors
    ///
    /// Fails exactly when [`ChainedEnvelope::verify`] fails.
    pub fn verify_envelope(&self) -> Result<()> {
        self.envelope.verify()
    }

    /// Check the hop chain alone (see [`check_hop_chain`]) under the
    /// default cap.
    ///
    /// # Errors
    ///
    /// Fails if the chain is malformed, looping, or over the cap.
    pub fn check_hops(&self) -> Result<()> {
        check_hop_chain(&self.message_id()?, &self.hops, DEFAULT_MAX_HOPS)
    }

    /// Append this relay's hop record, refusing first under the
    /// default cap and the loop rule, so a relay never signs a hop it
    /// should not forward.
    ///
    /// # Errors
    ///
    /// Fails if the existing chain is already invalid for this
    /// message, this relay is already in the chain, the cap is
    /// reached, or the new record cannot be signed.
    pub fn append_hop(&mut self, key: &SigningKey, received_at: u64) -> Result<HopRecord> {
        let message_id = self.message_id()?;
        check_hop_chain(&message_id, &self.hops, DEFAULT_MAX_HOPS)?;
        let relay = hex_encode(&key.verifying_key().to_bytes());
        if let Some(refusal) = relay_refusal(&self.hops, &relay, DEFAULT_MAX_HOPS) {
            bail!("relay refused: {refusal}");
        }
        let previous_hop = self.hops.last().map(|hop| hop.relay.as_str());
        let record = HopRecord::signed(key, &message_id, previous_hop, received_at)?;
        self.hops.push(record.clone());
        Ok(record)
    }
}

/// Why a relay must not forward a message, as a pure function of the
/// hop chain: the relay is already in the chain (loop/self-hop), or
/// the chain is at the cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelayRefusal {
    /// The relay already appears in the hop chain; forwarding would
    /// loop the message back through it.
    AlreadyInChain,
    /// The hop chain has reached the cap.
    CapReached,
}

impl std::fmt::Display for RelayRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyInChain => write!(f, "relay is already in the hop chain"),
            Self::CapReached => write!(f, "hop chain is at the cap"),
        }
    }
}

impl std::error::Error for RelayRefusal {}

/// Whether `relay` already appears in `hops` — the loop/self-hop test
/// a relay runs before forwarding.
pub fn chain_contains_relay(hops: &[HopRecord], relay: &str) -> bool {
    hops.iter().any(|hop| hop.relay == relay)
}

/// The refusal that applies if `relay` tried to forward a message
/// carrying `hops` under a cap of `max_hops`, or `None` if it may
/// forward (subject to the chain and envelope checks, which are
/// separate). Pure: no signature is consulted.
pub fn relay_refusal(hops: &[HopRecord], relay: &str, max_hops: usize) -> Option<RelayRefusal> {
    if chain_contains_relay(hops, relay) {
        return Some(RelayRefusal::AlreadyInChain);
    }
    if hops.len() >= max_hops {
        return Some(RelayRefusal::CapReached);
    }
    None
}

/// Check a hop chain for `message_id` under a cap of `max_hops`:
/// every record is structurally valid and signed by its relay for
/// this message, the records link in order (each names the previous
/// record's relay; the first names none), no relay repeats (no loop,
/// no self-hop), and the chain is within the cap. An empty chain is
/// valid — hops are optional audit metadata.
///
/// This check is about the chain only: it neither needs nor confers
/// envelope validity.
///
/// # Errors
///
/// Fails with the first defect found: over-cap, malformed or
/// mis-signed record, broken linkage, or a repeated relay.
pub fn check_hop_chain(message_id: &str, hops: &[HopRecord], max_hops: usize) -> Result<()> {
    if hops.len() > MAX_HOPS {
        bail!("hop chain is longer than any cap allows");
    }
    if hops.len() > max_hops {
        bail!("hop chain is over the cap of {max_hops}");
    }
    let mut previous: Option<&str> = None;
    let mut seen = HashSet::new();
    for hop in hops {
        hop.verify(message_id)?;
        if hop.previous_hop.as_deref() != previous {
            bail!("hop chain linkage is broken");
        }
        if !seen.insert(hop.relay.as_str()) {
            bail!("a relay appears twice in one hop chain (loop)");
        }
        previous = Some(hop.relay.as_str());
    }
    Ok(())
}

/// Message IDs a node has already seen, for duplicate suppression: a
/// node forwards a message at most once, however many paths it
/// arrives by. Keyed by message ID, so different signed envelopes —
/// forks — never collapse into one another.
#[derive(Clone, Debug, Default)]
pub struct DedupeSet {
    seen: HashSet<MessageId>,
}

impl DedupeSet {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `envelope`'s message ID. Returns `true` the first time
    /// an ID is observed (the caller should process/forward it) and
    /// `false` for every later observation (a duplicate to suppress).
    ///
    /// # Errors
    ///
    /// Fails only if the envelope does not encode.
    pub fn observe(&mut self, envelope: &ChainedEnvelope) -> Result<bool> {
        Ok(self.seen.insert(envelope.message_id()?))
    }

    /// Whether `message_id` has been observed.
    pub fn contains(&self, message_id: &str) -> bool {
        self.seen.contains(message_id)
    }

    /// How many distinct message IDs have been observed.
    pub fn len(&self) -> usize {
        self.seen.len()
    }

    /// Whether nothing has been observed yet.
    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}

/// Bounded NFC text for group IDs and class names.
fn validate_text(field: &str, text: &str) -> Result<()> {
    if text.is_empty() {
        bail!("chained-message {field} is empty");
    }
    if text.len() > MAX_TEXT_BYTES {
        bail!("chained-message {field} is over the byte bound");
    }
    if !is_nfc(text) {
        bail!("chained-message {field} is not NFC");
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    // Oracle for every test in this module: the #2036 design record
    // (`doc/2026-11-01_design_id2036.md` §4.1 and §5 invariants 1–3)
    // and task #3796 part A's acceptance list.

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn key_id(seed: u8) -> String {
        hex_encode(&key(seed).verifying_key().to_bytes())
    }

    fn envelope(origin_seed: u8, sequence: u64) -> ChainedEnvelope {
        ChainedEnvelope::signed(
            &key(origin_seed),
            "group-1",
            sequence,
            Vec::new(),
            "durable",
            ChainedContent::Inline {
                bytes: b"hello".to_vec(),
            },
            1_000,
        )
        .unwrap()
    }

    fn relayed_with_hops(hop_seeds: &[u8]) -> RelayedMessage {
        let mut relayed = RelayedMessage::new(envelope(1, 0));
        for (index, seed) in hop_seeds.iter().enumerate() {
            relayed
                .append_hop(&key(*seed), 2_000 + u64::try_from(index).unwrap())
                .unwrap();
        }
        relayed
    }

    #[test]
    fn message_id_is_stable_and_ignores_hops() {
        // Invariant 1: the same signed envelope verifies to the same
        // message ID over a direct path or any relay path.
        let direct = RelayedMessage::new(envelope(1, 0));
        let relayed = relayed_with_hops(&[2, 3]);
        assert_eq!(direct.message_id().unwrap(), relayed.message_id().unwrap());
        relayed.verify_envelope().unwrap();
        relayed.check_hops().unwrap();
    }

    #[test]
    fn envelope_verifies_identically_with_a_corrupt_chain() {
        // §4.1: hop alteration does not change origin validity — a
        // corrupt chain fails the chain check, never the envelope.
        let mut relayed = relayed_with_hops(&[2]);
        relayed.hops[0].signature = "00".repeat(64);
        relayed.verify_envelope().unwrap();
        assert!(relayed.check_hops().is_err());
    }

    #[test]
    fn altered_envelope_is_rejected() {
        // Invariant 1/2: tampering after signing breaks verification.
        let mut tampered = envelope(1, 0);
        tampered.content = ChainedContent::Inline {
            bytes: b"forged".to_vec(),
        };
        assert!(tampered.verify().is_err());

        let mut reorigined = envelope(1, 0);
        reorigined.origin = key_id(9);
        assert!(reorigined.verify().is_err());
    }

    #[test]
    fn duplicates_are_suppressed_and_forks_stay_distinct() {
        // Invariant 1: alternate paths dedupe by message ID; forks
        // (different signed envelopes) do not collapse.
        let mut dedupe = DedupeSet::new();
        let first = envelope(1, 0);
        assert!(dedupe.observe(&first).unwrap());
        assert!(!dedupe.observe(&first).unwrap());
        let fork = envelope(1, 1);
        assert_ne!(first.message_id().unwrap(), fork.message_id().unwrap());
        assert!(dedupe.observe(&fork).unwrap());
        assert_eq!(dedupe.len(), 2);
    }

    #[test]
    fn loop_and_self_hop_are_detected() {
        // Invariant 3: a node refuses its own prior hop.
        let relayed = relayed_with_hops(&[2, 3]);
        assert!(chain_contains_relay(&relayed.hops, &key_id(2)));
        assert_eq!(
            relay_refusal(&relayed.hops, &key_id(2), DEFAULT_MAX_HOPS),
            Some(RelayRefusal::AlreadyInChain)
        );
        // A hand-built chain repeating a relay fails the chain check.
        let message_id = relayed.message_id().unwrap();
        let first = HopRecord::signed(&key(2), &message_id, None, 2_000).unwrap();
        let looped = HopRecord::signed(&key(2), &message_id, Some(&key_id(2)), 2_001);
        assert!(looped.is_err());
        let mut chain = vec![first];
        chain.push(HopRecord::signed(&key(3), &message_id, Some(&key_id(2)), 2_001).unwrap());
        chain.push(HopRecord::signed(&key(2), &message_id, Some(&key_id(3)), 2_002).unwrap());
        assert!(check_hop_chain(&message_id, &chain, DEFAULT_MAX_HOPS).is_err());
    }

    #[test]
    fn malformed_hops_are_rejected() {
        // Acceptance: malformed-hop handling — bad signature, broken
        // linkage, and a hop transplanted from another message all
        // fail the chain check while the envelope stays valid.
        let relayed = relayed_with_hops(&[2, 3]);
        let message_id = relayed.message_id().unwrap();

        let mut bad_signature = relayed.hops.clone();
        bad_signature[0].received_at += 1;
        assert!(check_hop_chain(&message_id, &bad_signature, DEFAULT_MAX_HOPS).is_err());

        let mut broken_link = relayed.hops.clone();
        broken_link[1].previous_hop = None;
        assert!(check_hop_chain(&message_id, &broken_link, DEFAULT_MAX_HOPS).is_err());

        let other = envelope(1, 7);
        assert!(
            check_hop_chain(
                &other.message_id().unwrap(),
                &relayed.hops,
                DEFAULT_MAX_HOPS
            )
            .is_err()
        );
        relayed.verify_envelope().unwrap();
    }

    #[test]
    fn over_cap_chain_is_rejected() {
        // §4.1: forwarding stops at the four-hop default cap.
        let relayed = relayed_with_hops(&[2, 3, 4, 5]);
        relayed.check_hops().unwrap();
        assert_eq!(
            relay_refusal(&relayed.hops, &key_id(6), DEFAULT_MAX_HOPS),
            Some(RelayRefusal::CapReached)
        );
        let message_id = relayed.message_id().unwrap();
        let mut chain = relayed.hops.clone();
        chain.push(HopRecord::signed(&key(6), &message_id, Some(&key_id(5)), 3_000).unwrap());
        assert!(check_hop_chain(&message_id, &chain, DEFAULT_MAX_HOPS).is_err());
        // The same chain is valid under a larger configured cap: the
        // four-hop value is a default, not a protocol constant.
        check_hop_chain(&message_id, &chain, 8).unwrap();
    }

    #[test]
    fn blob_content_and_bounds_are_checked() {
        // Acceptance: content may be a blob reference; oversized
        // inline content is refused at signing time.
        let blob = ChainedEnvelope::signed(
            &key(1),
            "group-1",
            0,
            Vec::new(),
            "durable",
            ChainedContent::Blob {
                hash: hex_encode(blake3::hash(b"blob").as_bytes()),
                size: 4,
            },
            1_000,
        )
        .unwrap();
        blob.verify().unwrap();

        let oversized = ChainedEnvelope::signed(
            &key(1),
            "group-1",
            0,
            Vec::new(),
            "durable",
            ChainedContent::Inline {
                bytes: vec![0; MAX_INLINE_CONTENT_BYTES + 1],
            },
            1_000,
        );
        assert!(oversized.is_err());
    }
}
