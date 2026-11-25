//! World announcements: the signed discovery object of the
//! discovery/registry/federation design (doc
//! 2026-11-16_design_id6287_discovery_registry_federation_v1.md
//! section 3; implementation breakdown item 1/5, #10705).
//!
//! One schema, two subjects: a world announcement carries an iroh
//! endpoint (node id + relay hint, never a raw IP/port); a program
//! announcement replaces the endpoint with a content hash. The whole
//! signed packet is CBOR and must fit the pkarr packet budget
//! ([`MAX_PACKET_BYTES`]); the field caps keep a sane packet under
//! ~800 bytes. Signing follows the envelope machinery's pattern
//! (#1069): the signature covers the canonical body encoding and
//! verifies against the key the object itself names — here the
//! world key, which is also the announcement's `world_id`.

use anyhow::{Context, Result, bail};
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

/// Maximum display-name length, in chars (design section 3).
pub const MAX_NAME_CHARS: usize = 64;
/// Maximum description length, in chars (design section 3).
pub const MAX_DESCRIPTION_CHARS: usize = 200;
/// Maximum number of tags (design section 3).
pub const MAX_TAGS: usize = 8;
/// Maximum tag length, in chars — the implementation's pin of the
/// design's "short strings", sized so eight full tags cannot crowd
/// the packet budget on their own.
pub const MAX_TAG_CHARS: usize = 32;
/// Maximum relay-hint length, in chars, sized to the packet budget.
pub const MAX_RELAY_CHARS: usize = 200;
/// Maximum announcement lifetime: 7 days, in seconds (design
/// section 3 — expired announcements are void, not stale).
pub const MAX_LIFETIME_SECONDS: u64 = 7 * 24 * 60 * 60;
/// The budget for the whole signed packet, in bytes: the largest
/// payload one pkarr DNS packet can carry (1000 bytes, less the
/// `_ida.<z32>` TXT record framing; #10706,
/// [`crate::announcement_pkarr`]). Anything `to_bytes` accepts can
/// therefore be published.
pub const MAX_PACKET_BYTES: usize = 915;

/// Coarse occupancy (design section 3, Q1): exact counts are a
/// privacy leak and the runtime does not track them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Occupancy {
    /// The world has room for more members.
    HasRoom,
    /// The world is approaching its member cap.
    NearFull,
    /// The world admits no new members.
    Full,
}

/// Whether a registry may list the world (design section 3):
/// `private` worlds publish nothing at all, so there is no variant
/// for them here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    /// Discoverable by direct lookup, not listed in registries.
    Unlisted,
    /// Registries may list the world.
    Listed,
}

/// What the announcement is about: exactly one of an endpoint (a
/// world) or a content hash (a program) — one schema, two subjects
/// (design section 3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Subject {
    /// A world endpoint: an iroh node id plus an optional relay
    /// hint. Never a raw IP/port.
    Endpoint {
        /// The host's iroh node id (an ed25519 public key).
        #[serde(with = "byte_array")]
        node_id: [u8; 32],
        /// A relay URL hint, when the host publishes one.
        relay: Option<String>,
    },
    /// A program (binary/module), named by its content hash.
    Program {
        /// The program's content hash (blake3).
        #[serde(with = "byte_array")]
        content_hash: [u8; 32],
    },
}

/// A world (or program) announcement, unsigned. The signed form is
/// [`SignedAnnouncement`]; the field set is exactly design
/// section 3's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Announcement {
    /// The world's identity: its ed25519 public key, also its
    /// pkarr key and the key the signature must verify against.
    #[serde(with = "byte_array")]
    pub world_id: [u8; 32],
    /// Display name (a claim, not an ID), at most
    /// [`MAX_NAME_CHARS`] chars.
    pub name: String,
    /// One-line description, at most [`MAX_DESCRIPTION_CHARS`]
    /// chars; longer text lives behind the endpoint.
    pub description: String,
    /// The endpoint or content hash being announced.
    pub subject: Subject,
    /// Coarse occupancy.
    pub occupancy: Occupancy,
    /// Discovery hints, at most [`MAX_TAGS`] short strings.
    /// Unsigned semantics: registries may filter on tags, clients
    /// must not trust them.
    pub tags: Vec<String>,
    /// Registry visibility.
    pub visibility: Visibility,
    /// When the announcement was issued, in Unix seconds.
    pub issued_at: u64,
    /// When the announcement becomes void, in Unix seconds; at
    /// most [`MAX_LIFETIME_SECONDS`] after `issued_at`.
    pub expires_at: u64,
    /// The BEP44 sequence: monotone per world key. A lower-or-equal
    /// seq never replaces a higher one.
    pub seq: u64,
}

impl Announcement {
    /// The design's construction rules: field caps, a lifetime of
    /// at most 7 days, and a relay hint that is a URL, never a raw
    /// IP/port.
    ///
    /// # Errors
    ///
    /// Fails on the first violated cap or malformed field.
    pub fn validate(&self) -> Result<()> {
        if self.name.chars().count() > MAX_NAME_CHARS {
            bail!("announcement name exceeds {MAX_NAME_CHARS} chars");
        }
        if self.description.chars().count() > MAX_DESCRIPTION_CHARS {
            bail!("announcement description exceeds {MAX_DESCRIPTION_CHARS} chars");
        }
        if self.tags.len() > MAX_TAGS {
            bail!("announcement carries more than {MAX_TAGS} tags");
        }
        for tag in &self.tags {
            if tag.chars().count() > MAX_TAG_CHARS {
                bail!("announcement tag exceeds {MAX_TAG_CHARS} chars");
            }
        }
        if let Subject::Endpoint { relay, .. } = &self.subject {
            if let Some(relay) = relay {
                validate_relay_hint(relay)?;
            }
        }
        let lifetime = self
            .expires_at
            .checked_sub(self.issued_at)
            .context("announcement expires before it is issued")?;
        if lifetime == 0 || lifetime > MAX_LIFETIME_SECONDS {
            bail!("announcement lifetime must be within 1..={MAX_LIFETIME_SECONDS} seconds");
        }
        Ok(())
    }

    /// [`Announcement::validate`] plus the acceptance rule that an
    /// expired announcement is void at `now` (Unix seconds).
    ///
    /// # Errors
    ///
    /// Fails on any construction violation, or when `now` has
    /// reached `expires_at`.
    pub fn validate_at(&self, now: u64) -> Result<()> {
        self.validate()?;
        if now >= self.expires_at {
            bail!("announcement is expired");
        }
        Ok(())
    }

    /// The sequence rule (design section 3): this announcement may
    /// replace `previous` only if it is the same world's and its
    /// seq is strictly higher. With no previous announcement, any
    /// seq is acceptable.
    ///
    /// # Errors
    ///
    /// Fails when the world differs or the seq does not advance.
    pub fn accepts_after(&self, previous: Option<&Announcement>) -> Result<()> {
        if let Some(previous) = previous {
            if previous.world_id != self.world_id {
                bail!("announcement sequence is per world key: world differs");
            }
            if self.seq <= previous.seq {
                bail!(
                    "announcement seq {} does not advance past {}",
                    self.seq,
                    previous.seq
                );
            }
        }
        Ok(())
    }

    /// The canonical CBOR encoding of the announcement body — the
    /// bytes the signature covers.
    ///
    /// # Errors
    ///
    /// Fails if the body does not encode.
    pub fn encode_body(&self) -> Result<Vec<u8>> {
        Ok(cbor_core::Value::serialized(self)
            .context("announcement body does not encode as CBOR")?
            .encode())
    }
}

/// An [`Announcement`] plus its world key's ed25519 signature over
/// the body's canonical CBOR encoding — the #1069 envelope pattern
/// (a signature the receiver checks against the key the object
/// names, with no server to ask).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedAnnouncement {
    /// The announced object.
    pub announcement: Announcement,
    /// The world key's signature over
    /// [`Announcement::encode_body`].
    #[serde(with = "byte_array")]
    pub signature: [u8; 64],
}

impl SignedAnnouncement {
    /// Validate and sign `announcement` with the world key.
    ///
    /// `key` must be the announcement's world key: verification
    /// checks the signature against `world_id`, so an announcement
    /// signed by any other key could never verify anywhere. That
    /// mismatch is refused here, at sign time (the envelope rule,
    /// #8843).
    ///
    /// # Errors
    ///
    /// Fails if the announcement violates a construction rule, if
    /// `key` is not the world key, or if the body does not encode.
    pub fn sign(key: &SigningKey, announcement: Announcement) -> Result<Self> {
        announcement.validate()?;
        if key.verifying_key().to_bytes() != announcement.world_id {
            bail!(
                "announcement world_id is not the signing key: it verifies only against its world key"
            );
        }
        let body = announcement.encode_body()?;
        let signature = key.sign(&body);
        Ok(Self {
            announcement,
            signature: signature.to_bytes(),
        })
    }

    /// Check the signature against the announcement's world key,
    /// after checking the construction rules.
    ///
    /// # Errors
    ///
    /// Fails on any construction violation or when the signature
    /// does not match the body and world key.
    pub fn verify(&self) -> Result<()> {
        self.announcement.validate()?;
        let key = VerifyingKey::from_bytes(&self.announcement.world_id)
            .context("announcement world_id is not a key")?;
        let signature = Signature::from_bytes(&self.signature);
        let body = self.announcement.encode_body()?;
        key.verify_strict(&body, &signature)
            .context("announcement signature does not verify")
    }

    /// [`SignedAnnouncement::verify`] plus the expiry rule at `now`
    /// (Unix seconds): an expired announcement is void.
    ///
    /// # Errors
    ///
    /// Fails on any verification failure, or when `now` has
    /// reached the announcement's `expires_at`.
    pub fn verify_at(&self, now: u64) -> Result<()> {
        self.announcement.validate_at(now)?;
        self.verify()
    }

    /// The signed packet: CBOR of the whole signed object, held to
    /// the pkarr packet budget.
    ///
    /// # Errors
    ///
    /// Fails if the packet does not encode or exceeds
    /// [`MAX_PACKET_BYTES`] — an over-budget announcement is
    /// rejected here, at encode time, never published truncated.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let bytes = cbor_core::Value::serialized(self)
            .context("signed announcement does not encode as CBOR")?
            .encode();
        if bytes.len() > MAX_PACKET_BYTES {
            bail!(
                "signed announcement is {} bytes, over the {MAX_PACKET_BYTES}-byte pkarr budget",
                bytes.len()
            );
        }
        Ok(bytes)
    }

    /// Decode a signed packet. The packet is held to the same
    /// budget on the way in; decoding does not verify — call
    /// [`SignedAnnouncement::verify`] (or `verify_at`) on the
    /// result.
    ///
    /// # Errors
    ///
    /// Fails if `bytes` exceeds the budget or does not decode.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_PACKET_BYTES {
            bail!(
                "signed announcement is {} bytes, over the {MAX_PACKET_BYTES}-byte pkarr budget",
                bytes.len()
            );
        }
        let value = cbor_core::Value::decode(bytes).context("signed announcement is not CBOR")?;
        value
            .deserialized()
            .context("signed announcement does not decode")
    }
}

/// A relay hint is a relay URL, never a raw IP/port (design
/// section 3): it must carry a scheme and a non-IP host.
fn validate_relay_hint(relay: &str) -> Result<()> {
    if relay.chars().count() > MAX_RELAY_CHARS {
        bail!("relay hint exceeds {MAX_RELAY_CHARS} chars");
    }
    let Some((_, rest)) = relay.split_once("://") else {
        bail!("relay hint is not a URL");
    };
    let host = rest.split(['/', ':', '?', '#']).next().unwrap_or_default();
    if host.is_empty() || host.parse::<std::net::IpAddr>().is_ok() {
        bail!("relay hint names a raw IP host; a relay hint is a name, never a raw IP/port");
    }
    Ok(())
}

/// Serde for fixed byte arrays as CBOR byte strings (a `[u8; N]`
/// would otherwise encode as N separate integers and blow the
/// packet budget).
pub(crate) mod byte_array {
    use serde::{Deserializer, Serializer, de::Error as _, de::Visitor};

    pub fn serialize<S, const N: usize>(value: &[u8; N], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(value)
    }

    pub fn deserialize<'de, D, const N: usize>(deserializer: D) -> Result<[u8; N], D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_bytes(ByteArrayVisitor::<N>)
    }

    struct ByteArrayVisitor<const N: usize>;

    impl<'de, const N: usize> Visitor<'de> for ByteArrayVisitor<N> {
        type Value = [u8; N];

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "a {N}-byte string")
        }

        fn visit_byte_buf<E: serde::de::Error>(self, value: Vec<u8>) -> Result<Self::Value, E> {
            value
                .try_into()
                .map_err(|_| E::custom(format!("byte string is not {N} bytes")))
        }

        fn visit_bytes<E: serde::de::Error>(self, value: &[u8]) -> Result<Self::Value, E> {
            value
                .try_into()
                .map_err(|_| E::custom(format!("byte string is not {N} bytes")))
        }

        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let mut out = [0_u8; N];
            for slot in &mut out {
                *slot = seq
                    .next_element()?
                    .ok_or_else(|| A::Error::custom("byte array is short"))?;
            }
            Ok(out)
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::hex::hex_encode;

    const ISSUED: u64 = 1_763_500_000;

    const WORLD_BODY_HEX: &str = "aa637365711829646e616d6566686172626f726474616773826463616c6d6474657374677375626a656374a168656e64706f696e74a26572656c6179781a68747470733a2f2f72656c61792e6578616d706c652e6e65742f676e6f64655f696458201398f62c6d1a457c51ba6a4b5f3dbd2f69fca93216218dc8997e416bd17d93ca68776f726c645f69645820ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c696973737565645f61741a691cdfe0696f63637570616e6379686861735f726f6f6d6a657870697265735f61741a6920d4606a7669736962696c697479666c69737465646b6465736372697074696f6e7261207175696574207465737420776f726c64";
    const WORLD_SIGNATURE_HEX: &str = "0ba4d643e3c7e3428b8a3e320f024b180d0d3711847ca094ed466456840325669b4981e3f57d70888a2053df906b59f3f27dca44488a8a78b93a1a0ed3dd4800";
    const WORLD_PACKET_HEX: &str = "a2697369676e617475726558400ba4d643e3c7e3428b8a3e320f024b180d0d3711847ca094ed466456840325669b4981e3f57d70888a2053df906b59f3f27dca44488a8a78b93a1a0ed3dd48006c616e6e6f756e63656d656e74aa637365711829646e616d6566686172626f726474616773826463616c6d6474657374677375626a656374a168656e64706f696e74a26572656c6179781a68747470733a2f2f72656c61792e6578616d706c652e6e65742f676e6f64655f696458201398f62c6d1a457c51ba6a4b5f3dbd2f69fca93216218dc8997e416bd17d93ca68776f726c645f69645820ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c696973737565645f61741a691cdfe0696f63637570616e6379686861735f726f6f6d6a657870697265735f61741a6920d4606a7669736962696c697479666c69737465646b6465736372697074696f6e7261207175696574207465737420776f726c64";
    const PROGRAM_BODY_HEX: &str = "aa6373657101646e616d6564746f6f6c647461677380677375626a656374a16770726f6772616da16c636f6e74656e745f686173685820dceaf1f8ada4da2d710209256bf5880460c3fddd49c0f1ddc08cd09c9444eb6368776f726c645f69645820fd1724385aa0c75b64fb78cd602fa1d991fdebf76b13c58ed702eac835e9f618696973737565645f61741a691cdfe0696f63637570616e6379686861735f726f6f6d6a657870697265735f61741a691e31606a7669736962696c69747968756e6c69737465646b6465736372697074696f6e60";
    const PROGRAM_SIGNATURE_HEX: &str = "b7410f0e24739f78f6bafb6ddecde8ad3b001132d285e98dd109a9bdbbfb2ac8377ef37fc41c8b9f0cb9c120acf59b31db609dede3c405c77c90550298269509";
    const PROGRAM_PACKET_HEX: &str = "a2697369676e61747572655840b7410f0e24739f78f6bafb6ddecde8ad3b001132d285e98dd109a9bdbbfb2ac8377ef37fc41c8b9f0cb9c120acf59b31db609dede3c405c77c905502982695096c616e6e6f756e63656d656e74aa6373657101646e616d6564746f6f6c647461677380677375626a656374a16770726f6772616da16c636f6e74656e745f686173685820dceaf1f8ada4da2d710209256bf5880460c3fddd49c0f1ddc08cd09c9444eb6368776f726c645f69645820fd1724385aa0c75b64fb78cd602fa1d991fdebf76b13c58ed702eac835e9f618696973737565645f61741a691cdfe0696f63637570616e6379686861735f726f6f6d6a657870697265735f61741a691e31606a7669736962696c69747968756e6c69737465646b6465736372697074696f6e60";

    #[test]
    fn print_vectors() {
        let (key, announcement) = world_announcement();
        let signed = SignedAnnouncement::sign(&key, announcement.clone()).unwrap();
        println!(
            "WORLD_BODY={}",
            hex_encode(&announcement.encode_body().unwrap())
        );
        println!("WORLD_SIG={}", hex_encode(&signed.signature));
        println!("WORLD_PACKET={}", hex_encode(&signed.to_bytes().unwrap()));
        let (key2, announcement2) = program_announcement();
        let signed2 = SignedAnnouncement::sign(&key2, announcement2.clone()).unwrap();
        println!(
            "PROGRAM_BODY={}",
            hex_encode(&announcement2.encode_body().unwrap())
        );
        println!("PROGRAM_SIG={}", hex_encode(&signed2.signature));
        println!(
            "PROGRAM_PACKET={}",
            hex_encode(&signed2.to_bytes().unwrap())
        );
    }

    fn world_announcement() -> (SigningKey, Announcement) {
        let key = SigningKey::from_bytes(&[7; 32]);
        let node = SigningKey::from_bytes(&[8; 32]);
        let announcement = Announcement {
            world_id: key.verifying_key().to_bytes(),
            name: "harbor".to_owned(),
            description: "a quiet test world".to_owned(),
            subject: Subject::Endpoint {
                node_id: node.verifying_key().to_bytes(),
                relay: Some("https://relay.example.net/".to_owned()),
            },
            occupancy: Occupancy::HasRoom,
            tags: vec!["calm".to_owned(), "test".to_owned()],
            visibility: Visibility::Listed,
            issued_at: ISSUED,
            expires_at: ISSUED + 3 * 24 * 60 * 60,
            seq: 41,
        };
        (key, announcement)
    }

    fn program_announcement() -> (SigningKey, Announcement) {
        let key = SigningKey::from_bytes(&[9; 32]);
        let announcement = Announcement {
            world_id: key.verifying_key().to_bytes(),
            name: "tool".to_owned(),
            description: String::new(),
            subject: Subject::Program {
                content_hash: *blake3::hash(b"program-bytes").as_bytes(),
            },
            occupancy: Occupancy::HasRoom,
            tags: Vec::new(),
            visibility: Visibility::Unlisted,
            issued_at: ISSUED,
            expires_at: ISSUED + 24 * 60 * 60,
            seq: 1,
        };
        (key, announcement)
    }

    #[test]
    fn kat_world_announcement_vectors() {
        let (key, announcement) = world_announcement();
        let body = announcement.encode_body().unwrap();
        assert_eq!(hex_encode(&body), WORLD_BODY_HEX);
        let signed = SignedAnnouncement::sign(&key, announcement).unwrap();
        assert_eq!(hex_encode(&signed.signature), WORLD_SIGNATURE_HEX);
        let packet = signed.to_bytes().unwrap();
        assert_eq!(hex_encode(&packet), WORLD_PACKET_HEX);
        assert!(packet.len() <= MAX_PACKET_BYTES);
        signed.verify().unwrap();
    }

    #[test]
    fn kat_program_announcement_vectors() {
        let (key, announcement) = program_announcement();
        let body = announcement.encode_body().unwrap();
        assert_eq!(hex_encode(&body), PROGRAM_BODY_HEX);
        let signed = SignedAnnouncement::sign(&key, announcement).unwrap();
        assert_eq!(hex_encode(&signed.signature), PROGRAM_SIGNATURE_HEX);
        let packet = signed.to_bytes().unwrap();
        assert_eq!(hex_encode(&packet), PROGRAM_PACKET_HEX);
        assert!(packet.len() <= MAX_PACKET_BYTES);
        signed.verify().unwrap();
    }

    #[test]
    fn packet_round_trips_through_bytes() {
        let (key, announcement) = world_announcement();
        let signed = SignedAnnouncement::sign(&key, announcement).unwrap();
        let decoded = SignedAnnouncement::from_bytes(&signed.to_bytes().unwrap()).unwrap();
        assert_eq!(decoded, signed);
        decoded.verify_at(ISSUED + 60).unwrap();
    }

    #[test]
    fn a_tampered_announcement_does_not_verify() {
        let (key, announcement) = world_announcement();
        let mut signed = SignedAnnouncement::sign(&key, announcement).unwrap();
        signed.announcement.name = "harbour".to_owned();
        assert!(signed.verify().is_err());
    }

    #[test]
    fn signing_with_a_foreign_key_is_refused() {
        let (_, announcement) = world_announcement();
        let foreign = SigningKey::from_bytes(&[3; 32]);
        assert!(SignedAnnouncement::sign(&foreign, announcement).is_err());
    }

    #[test]
    fn an_expired_announcement_is_void() {
        let (key, announcement) = world_announcement();
        let expires = announcement.expires_at;
        let signed = SignedAnnouncement::sign(&key, announcement).unwrap();
        assert!(signed.verify_at(expires - 1).is_ok());
        assert!(signed.verify_at(expires).is_err());
    }

    #[test]
    fn seq_must_advance_per_world() {
        let (_, announcement) = world_announcement();
        let mut next = announcement.clone();
        next.seq = announcement.seq + 1;
        next.accepts_after(Some(&announcement)).unwrap();
        // Equal and lower seqs never replace.
        announcement.accepts_after(Some(&announcement)).unwrap_err();
        let mut lower = announcement.clone();
        lower.seq -= 1;
        lower.accepts_after(Some(&announcement)).unwrap_err();
        // A different world is a different sequence entirely.
        let (_, other) = program_announcement();
        other.accepts_after(Some(&announcement)).unwrap_err();
        announcement.accepts_after(None).unwrap();
    }

    #[test]
    fn over_budget_packets_are_rejected() {
        let (key, mut announcement) = world_announcement();
        // Max-length fields in four-byte chars blow the byte budget
        // while respecting every char cap.
        announcement.name = "\u{754c}".repeat(MAX_NAME_CHARS);
        announcement.description = "\u{754c}".repeat(MAX_DESCRIPTION_CHARS);
        let signed = SignedAnnouncement::sign(&key, announcement).unwrap();
        assert!(signed.to_bytes().is_err());
        let oversized = vec![0_u8; MAX_PACKET_BYTES + 1];
        assert!(SignedAnnouncement::from_bytes(&oversized).is_err());
    }

    #[test]
    fn construction_caps_are_enforced() {
        let (key, announcement) = world_announcement();
        let mut long_name = announcement.clone();
        long_name.name = "n".repeat(MAX_NAME_CHARS + 1);
        assert!(SignedAnnouncement::sign(&key, long_name).is_err());

        let mut many_tags = announcement.clone();
        many_tags.tags = (0..=MAX_TAGS).map(|i| format!("tag{i}")).collect();
        assert!(many_tags.validate().is_err());

        let mut long_lifetime = announcement.clone();
        long_lifetime.expires_at = long_lifetime.issued_at + MAX_LIFETIME_SECONDS + 1;
        assert!(long_lifetime.validate().is_err());

        let mut raw_ip_relay = announcement.clone();
        raw_ip_relay.subject = Subject::Endpoint {
            node_id: [1; 32],
            relay: Some("https://203.0.113.7:8443/".to_owned()),
        };
        assert!(raw_ip_relay.validate().is_err());
    }
}
