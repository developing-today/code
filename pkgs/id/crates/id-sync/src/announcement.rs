//! World and program announcements: the signed discovery object of
//! the #6287 discovery/registry/federation v1 design, section 3
//! (doc/2026-11-16_design_id6287_discovery_registry_federation_v1.md),
//! implemented for #10705 (phase-14 breakdown 1/5).
//!
//! An announcement is the exact section-3 field set — `world_id`,
//! `name`, `description`, an endpoint (or, for a program
//! announcement, a content hash in its place), the coarse
//! `occupancy` enum, `tags`, `visibility`, `issued_at` /
//! `expires_at` with a lifetime of at most 7 days, and a monotone
//! `seq` — encoded in the id stack's canonical CBOR profile
//! ([`id_core::canonical_cbor`]) so the signed bytes are stable,
//! and signed by the world key through the same ed25519 envelope
//! pattern as [`crate::envelope`] (the #1069 pattern): the signer
//! must be the key the object names, and verification is against
//! the `world_id` the object carries.
//!
//! The BEP44 budget is enforced, not just documented: an
//! announcement whose encoded form — or whose signed packet —
//! exceeds [`MAX_ANNOUNCEMENT_BYTES`] is rejected at encode/sign
//! time, and an over-budget byte string is rejected at decode
//! time. Expired announcements are void: [`Announcement::validate_at`]
//! rejects them against the verifier's clock. A seq that does not
//! strictly advance on the stored one never replaces it
//! ([`Announcement::check_seq_after`]).

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use id_core::canonical_cbor::{decode_canonical, encode_canonical, CborError, Value};
use thiserror::Error;

/// Maximum `name` length, in chars (design section 3).
pub const MAX_NAME_CHARS: usize = 64;
/// Maximum `description` length, in chars (design section 3).
pub const MAX_DESCRIPTION_CHARS: usize = 200;
/// Maximum number of `tags` (design section 3).
pub const MAX_TAGS: usize = 8;
/// Maximum length of one tag, in chars. The design caps tags only
/// as "short strings"; 32 chars is this profile's reading of
/// "short", and the byte budget below bounds the total regardless.
pub const MAX_TAG_CHARS: usize = 32;
/// Maximum announcement lifetime: 7 days, in seconds (design
/// section 3: lifetime at most 7 days).
pub const MAX_LIFETIME_SECS: u64 = 7 * 24 * 60 * 60;
/// The BEP44 budget, in bytes: the whole signed packet must fit
/// inside it (design section 3: the field caps keep the signed
/// packet under ~800 bytes, leaving headroom inside the ~1,000-byte
/// BEP44 limit).
pub const MAX_ANNOUNCEMENT_BYTES: usize = 1_000;

/// Coarse occupancy (design section 3, Q1): exact counts are a
/// privacy leak, so the schema carries only this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Occupancy {
    /// The world has room for more occupants.
    HasRoom,
    /// The world is approaching its occupant limit.
    NearFull,
    /// The world admits no more occupants.
    Full,
}

impl Occupancy {
    fn wire(self) -> &'static str {
        match self {
            Self::HasRoom => "has_room",
            Self::NearFull => "near_full",
            Self::Full => "full",
        }
    }

    fn from_wire(wire: &str) -> Option<Self> {
        match wire {
            "has_room" => Some(Self::HasRoom),
            "near_full" => Some(Self::NearFull),
            "full" => Some(Self::Full),
            _ => None,
        }
    }
}

/// Whether the world asks registries to list it (design section 3).
/// `private` is not a value here: private worlds publish nothing
/// at all (design section 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    /// Published to pkarr but not offered for registry listing.
    Unlisted,
    /// Published and offered for registry listing.
    Listed,
}

impl Visibility {
    fn wire(self) -> &'static str {
        match self {
            Self::Unlisted => "unlisted",
            Self::Listed => "listed",
        }
    }

    fn from_wire(wire: &str) -> Option<Self> {
        match wire {
            "unlisted" => Some(Self::Unlisted),
            "listed" => Some(Self::Listed),
            _ => None,
        }
    }
}

/// What the announcement is about (design section 3: "one schema,
/// two subjects"). A world announcement carries an endpoint; a
/// program (binary/module) announcement carries a content hash in
/// the endpoint's place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    /// A world: reachable at an iroh node, never a raw IP/port.
    World {
        /// The iroh node id (an ed25519 public key).
        node_id: [u8; 32],
        /// Optional relay hint for reaching the node.
        relay_hint: Option<String>,
    },
    /// A program: identified by the hash of its content (blake3,
    /// as elsewhere in the id stack).
    Program {
        /// The content hash replacing the endpoint.
        content_hash: [u8; 32],
    },
}

/// A world or program announcement, unsigned. Use
/// [`Announcement::sign`] to produce the signed packet form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Announcement {
    /// The world's identity: an ed25519 public key, also its
    /// pkarr key. The signature is by this key.
    pub world_id: [u8; 32],
    /// Display name (a claim, not an ID), at most
    /// [`MAX_NAME_CHARS`] chars.
    pub name: String,
    /// One-line description, at most [`MAX_DESCRIPTION_CHARS`]
    /// chars; longer text lives behind the endpoint.
    pub description: String,
    /// The endpoint (world) or content hash (program).
    pub subject: Subject,
    /// Coarse occupancy.
    pub occupancy: Occupancy,
    /// Discovery hints, at most [`MAX_TAGS`] short strings.
    /// Unsigned semantics: registries may filter on them, clients
    /// must not trust them.
    pub tags: Vec<String>,
    /// Listing visibility.
    pub visibility: Visibility,
    /// Issue time, unix seconds.
    pub issued_at: u64,
    /// Expiry, unix seconds: at most [`MAX_LIFETIME_SECS`] after
    /// `issued_at`. Expired announcements are void, not stale.
    pub expires_at: u64,
    /// The BEP44 sequence: monotone per world key. A lower-or-equal
    /// seq never replaces a higher one.
    pub seq: u64,
}

/// Every way an announcement can be rejected. Each rejection the
/// design requires — over-budget, expired, seq-regressing, over-cap
/// fields, a signature that does not verify — is one of these,
/// never a panic or a silent coercion.
#[derive(Debug, Error)]
pub enum AnnouncementError {
    /// The canonical CBOR layer rejected the bytes.
    #[error(transparent)]
    Cbor(#[from] CborError),
    /// The decoded map is not the exact section-3 field set.
    #[error("malformed announcement: {0}")]
    Malformed(String),
    /// `name` exceeds [`MAX_NAME_CHARS`] chars.
    #[error("name exceeds {MAX_NAME_CHARS} chars")]
    NameTooLong,
    /// `description` exceeds [`MAX_DESCRIPTION_CHARS`] chars.
    #[error("description exceeds {MAX_DESCRIPTION_CHARS} chars")]
    DescriptionTooLong,
    /// More than [`MAX_TAGS`] tags.
    #[error("more than {MAX_TAGS} tags")]
    TooManyTags,
    /// A tag exceeds [`MAX_TAG_CHARS`] chars.
    #[error("tag exceeds {MAX_TAG_CHARS} chars")]
    TagTooLong,
    /// `expires_at` is not after `issued_at`.
    #[error("expires_at must be after issued_at")]
    InvalidWindow,
    /// Lifetime exceeds [`MAX_LIFETIME_SECS`].
    #[error("lifetime exceeds 7 days")]
    LifetimeTooLong,
    /// The announcement is expired at the verifier's clock.
    #[error("announcement expired at {expires_at} (now {now})")]
    Expired {
        /// The announcement's expiry, unix seconds.
        expires_at: u64,
        /// The verifier's clock, unix seconds.
        now: u64,
    },
    /// The encoded form or signed packet exceeds the BEP44 budget.
    #[error("encoded size {size} exceeds the {MAX_ANNOUNCEMENT_BYTES}-byte BEP44 budget")]
    OverBudget {
        /// The offending encoded size, in bytes.
        size: usize,
    },
    /// The signing key is not the key `world_id` names.
    #[error("signing key is not the announcement's world_id")]
    WorldKeyMismatch,
    /// The signature does not verify against `world_id`.
    #[error("announcement signature does not verify")]
    BadSignature,
    /// A seq that does not strictly advance on the stored one.
    #[error("seq {offered} does not advance on stored seq {stored}")]
    SeqRegression {
        /// The seq already stored for this world key.
        stored: u64,
        /// The seq the new announcement offers.
        offered: u64,
    },
}

/// Result alias for announcement operations.
pub type Result<T> = std::result::Result<T, AnnouncementError>;

fn text(value: &str) -> Value {
    Value::Text(value.to_string())
}

fn bytes32(value: &[u8; 32]) -> Value {
    Value::Bytes(value.to_vec())
}

impl Announcement {
    /// Check the field caps and the lifetime window. This is the
    /// validity that holds regardless of the verifier's clock;
    /// [`Announcement::validate_at`] adds the expiry check.
    pub fn validate(&self) -> Result<()> {
        if self.name.chars().count() > MAX_NAME_CHARS {
            return Err(AnnouncementError::NameTooLong);
        }
        if self.description.chars().count() > MAX_DESCRIPTION_CHARS {
            return Err(AnnouncementError::DescriptionTooLong);
        }
        if self.tags.len() > MAX_TAGS {
            return Err(AnnouncementError::TooManyTags);
        }
        if self.tags.iter().any(|tag| tag.chars().count() > MAX_TAG_CHARS) {
            return Err(AnnouncementError::TagTooLong);
        }
        if self.expires_at <= self.issued_at {
            return Err(AnnouncementError::InvalidWindow);
        }
        if self.expires_at - self.issued_at > MAX_LIFETIME_SECS {
            return Err(AnnouncementError::LifetimeTooLong);
        }
        Ok(())
    }

    /// [`Announcement::validate`], plus: an announcement expired at
    /// `now` (unix seconds) is void and rejected.
    pub fn validate_at(&self, now: u64) -> Result<()> {
        self.validate()?;
        if self.expires_at <= now {
            return Err(AnnouncementError::Expired {
                expires_at: self.expires_at,
                now,
            });
        }
        Ok(())
    }

    /// The replacement rule (design section 3): this announcement
    /// may replace a stored one for the same world key only if its
    /// seq strictly advances on `stored_seq`. Callers check the
    /// world key matches before consulting the seq.
    pub fn check_seq_after(&self, stored_seq: u64) -> Result<()> {
        if self.seq <= stored_seq {
            return Err(AnnouncementError::SeqRegression {
                stored: stored_seq,
                offered: self.seq,
            });
        }
        Ok(())
    }

    /// The canonical CBOR body map for this announcement.
    fn to_value(&self) -> Value {
        let mut entries: Vec<(Value, Value)> = vec![
            (text("world_id"), bytes32(&self.world_id)),
            (text("name"), text(&self.name)),
            (text("description"), text(&self.description)),
            (text("occupancy"), text(self.occupancy.wire())),
            (
                text("tags"),
                Value::Array(self.tags.iter().map(|tag| text(tag)).collect()),
            ),
            (text("visibility"), text(self.visibility.wire())),
            (text("issued_at"), Value::UInt(self.issued_at)),
            (text("expires_at"), Value::UInt(self.expires_at)),
            (text("seq"), Value::UInt(self.seq)),
        ];
        match &self.subject {
            Subject::World {
                node_id,
                relay_hint,
            } => entries.push((
                text("endpoint"),
                Value::Map(vec![
                    (text("node_id"), bytes32(node_id)),
                    (
                        text("relay"),
                        match relay_hint {
                            Some(hint) => text(hint),
                            None => Value::Null,
                        },
                    ),
                ]),
            )),
            Subject::Program { content_hash } => {
                entries.push((text("content_hash"), bytes32(content_hash)));
            }
        }
        Value::Map(entries)
    }

    /// Encode the announcement body in canonical CBOR, enforcing
    /// the field caps and the BEP44 budget. These are the exact
    /// bytes the signature covers.
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let encoded = encode_canonical(&self.to_value())?;
        if encoded.len() > MAX_ANNOUNCEMENT_BYTES {
            return Err(AnnouncementError::OverBudget {
                size: encoded.len(),
            });
        }
        Ok(encoded)
    }

    /// Decode an announcement body from canonical CBOR bytes.
    /// Strict: non-canonical bytes, an over-budget input, an
    /// unknown or missing field, an over-cap field, and an invalid
    /// lifetime window all reject.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_ANNOUNCEMENT_BYTES {
            return Err(AnnouncementError::OverBudget { size: bytes.len() });
        }
        let announcement = Self::from_value(&decode_canonical(bytes)?)?;
        announcement.validate()?;
        Ok(announcement)
    }

    /// Parse a decoded canonical value as an announcement body.
    /// The field set is exact (design section 3: "exactly these
    /// fields"): a program announcement carries `content_hash`
    /// where a world announcement carries `endpoint`, and no other
    /// substitution or addition is accepted.
    fn from_value(value: &Value) -> Result<Self> {
        let Value::Map(entries) = value else {
            return Err(AnnouncementError::Malformed("body is not a map".into()));
        };
        let get = |key: &str| -> Option<&Value> {
            entries.iter().find_map(|(k, v)| match k {
                Value::Text(name) if name == key => Some(v),
                _ => None,
            })
        };
        let known = [
            "world_id",
            "name",
            "description",
            "endpoint",
            "content_hash",
            "occupancy",
            "tags",
            "visibility",
            "issued_at",
            "expires_at",
            "seq",
        ];
        for (key, _) in entries {
            let Value::Text(name) = key else {
                return Err(AnnouncementError::Malformed("non-text map key".into()));
            };
            if !known.contains(&name.as_str()) {
                return Err(AnnouncementError::Malformed(format!(
                    "unknown field: {name}"
                )));
            }
        }
        let world_id = get_bytes32(get("world_id"), "world_id")?;
        let name = get_text(get("name"), "name")?.to_string();
        let description = get_text(get("description"), "description")?.to_string();
        let subject = match (get("endpoint"), get("content_hash")) {
            (Some(endpoint), None) => {
                let Value::Map(endpoint_entries) = endpoint else {
                    return Err(AnnouncementError::Malformed(
                        "endpoint is not a map".into(),
                    ));
                };
                let endpoint_get = |key: &str| -> Option<&Value> {
                    endpoint_entries.iter().find_map(|(k, v)| match k {
                        Value::Text(name) if name == key => Some(v),
                        _ => None,
                    })
                };
                if endpoint_entries.len() != 2 {
                    return Err(AnnouncementError::Malformed(
                        "endpoint must carry exactly node_id and relay".into(),
                    ));
                }
                let node_id = get_bytes32(endpoint_get("node_id"), "endpoint.node_id")?;
                let relay_hint = match endpoint_get("relay") {
                    Some(Value::Null) => None,
                    Some(Value::Text(hint)) => Some(hint.clone()),
                    _ => {
                        return Err(AnnouncementError::Malformed(
                            "endpoint.relay is neither text nor null".into(),
                        ))
                    }
                };
                Subject::World {
                    node_id,
                    relay_hint,
                }
            }
            (None, Some(hash)) => Subject::Program {
                content_hash: get_bytes32(Some(hash), "content_hash")?,
            },
            _ => {
                return Err(AnnouncementError::Malformed(
                    "exactly one of endpoint / content_hash must be present".into(),
                ))
            }
        };
        let occupancy = get_text(get("occupancy"), "occupancy")
            .and_then(|wire| {
                Occupancy::from_wire(wire)
                    .ok_or_else(|| AnnouncementError::Malformed(format!("bad occupancy: {wire}")))
            })?;
        let tags = match get("tags") {
            Some(Value::Array(items)) => {
                let mut tags = Vec::with_capacity(items.len());
                for item in items {
                    let Value::Text(tag) = item else {
                        return Err(AnnouncementError::Malformed(
                            "tag is not text".into(),
                        ));
                    };
                    tags.push(tag.clone());
                }
                tags
            }
            _ => return Err(AnnouncementError::Malformed("tags is not an array".into())),
        };
        let visibility = get_text(get("visibility"), "visibility")
            .and_then(|wire| {
                Visibility::from_wire(wire).ok_or_else(|| {
                    AnnouncementError::Malformed(format!("bad visibility: {wire}"))
                })
            })?;
        Ok(Self {
            world_id,
            name,
            description,
            subject,
            occupancy,
            tags,
            visibility,
            issued_at: get_uint(get("issued_at"), "issued_at")?,
            expires_at: get_uint(get("expires_at"), "expires_at")?,
            seq: get_uint(get("seq"), "seq")?,
        })
    }

    /// Sign this announcement with the world key. Like the envelope
    /// pattern, a signer that is not the key the object names is
    /// refused here, at sign time: such a packet could never verify
    /// anywhere. The signature covers the canonical body encoding
    /// ([`Announcement::encode`]).
    pub fn sign(&self, key: &SigningKey) -> Result<SignedAnnouncement> {
        if key.verifying_key().to_bytes() != self.world_id {
            return Err(AnnouncementError::WorldKeyMismatch);
        }
        let body = self.encode()?;
        let signature = key.sign(&body).to_bytes();
        let signed = SignedAnnouncement {
            announcement: self.clone(),
            signature,
        };
        // The packet, not just the body, must fit the budget.
        if signed.to_bytes()?.len() > MAX_ANNOUNCEMENT_BYTES {
            return Err(AnnouncementError::OverBudget {
                size: signed.to_bytes()?.len(),
            });
        }
        Ok(signed)
    }
}

fn get_text<'a>(value: Option<&'a Value>, field: &str) -> Result<&'a str> {
    match value {
        Some(Value::Text(text)) => Ok(text),
        _ => Err(AnnouncementError::Malformed(format!(
            "field {field} missing or not text"
        ))),
    }
}

fn get_uint(value: Option<&Value>, field: &str) -> Result<u64> {
    match value {
        Some(Value::UInt(value)) => Ok(*value),
        _ => Err(AnnouncementError::Malformed(format!(
            "field {field} missing or not an unsigned integer"
        ))),
    }
}

fn get_bytes32(value: Option<&Value>, field: &str) -> Result<[u8; 32]> {
    match value {
        Some(Value::Bytes(bytes)) if bytes.len() == 32 => {
            let mut out = [0u8; 32];
            out.copy_from_slice(bytes);
            Ok(out)
        }
        _ => Err(AnnouncementError::Malformed(format!(
            "field {field} missing or not a 32-byte string"
        ))),
    }
}

/// An announcement plus its world-key signature: the packet that
/// is published (pkarr/BEP44) and verified by receivers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedAnnouncement {
    /// The announced object.
    pub announcement: Announcement,
    /// Ed25519 signature by the world key over
    /// [`Announcement::encode`] of the announced object.
    pub signature: [u8; 64],
}

impl SignedAnnouncement {
    /// Encode the signed packet in canonical CBOR: a map of the
    /// announcement body map and the signature bytes. Enforces the
    /// BEP44 budget on the whole packet.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let packet = Value::Map(vec![
            (text("announcement"), self.announcement.to_value()),
            (text("signature"), Value::Bytes(self.signature.to_vec())),
        ]);
        let encoded = encode_canonical(&packet)?;
        if encoded.len() > MAX_ANNOUNCEMENT_BYTES {
            return Err(AnnouncementError::OverBudget {
                size: encoded.len(),
            });
        }
        Ok(encoded)
    }

    /// Parse a signed packet. This does not verify: call
    /// [`SignedAnnouncement::verify`] (or `verify_at`) on the
    /// result, as with the envelope pattern.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_ANNOUNCEMENT_BYTES {
            return Err(AnnouncementError::OverBudget { size: bytes.len() });
        }
        let Value::Map(entries) = decode_canonical(bytes)? else {
            return Err(AnnouncementError::Malformed("packet is not a map".into()));
        };
        if entries.len() != 2 {
            return Err(AnnouncementError::Malformed(
                "packet must carry exactly announcement and signature".into(),
            ));
        }
        let get = |key: &str| -> Option<&Value> {
            entries.iter().find_map(|(k, v)| match k {
                Value::Text(name) if name == key => Some(v),
                _ => None,
            })
        };
        let announcement = match get("announcement") {
            Some(value) => Announcement::from_value(value)?,
            None => {
                return Err(AnnouncementError::Malformed(
                    "packet is missing announcement".into(),
                ))
            }
        };
        let signature = match get("signature") {
            Some(Value::Bytes(bytes)) if bytes.len() == 64 => {
                let mut out = [0u8; 64];
                out.copy_from_slice(bytes);
                out
            }
            _ => {
                return Err(AnnouncementError::Malformed(
                    "packet signature missing or not 64 bytes".into(),
                ))
            }
        };
        Ok(Self {
            announcement,
            signature,
        })
    }

    /// Check the signature against the world key the announcement
    /// names, and the announcement's own validity. Fails if the
    /// signature is malformed or does not verify, or the
    /// announcement itself is invalid.
    pub fn verify(&self) -> Result<()> {
        self.announcement.validate()?;
        let key = VerifyingKey::from_bytes(&self.announcement.world_id)
            .map_err(|_| AnnouncementError::BadSignature)?;
        let body = self.announcement.encode()?;
        key.verify_strict(&body, &Signature::from_bytes(&self.signature))
            .map_err(|_| AnnouncementError::BadSignature)
    }

    /// [`SignedAnnouncement::verify`], plus the expiry check
    /// against the verifier's clock (`now`, unix seconds): an
    /// expired announcement is void.
    pub fn verify_at(&self, now: u64) -> Result<()> {
        self.announcement.validate_at(now)?;
        self.verify()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hex::hex_encode;

    fn world_key() -> SigningKey {
        SigningKey::from_bytes(&[7; 32])
    }

    fn sample_world() -> Announcement {
        let key = world_key();
        Announcement {
            world_id: key.verifying_key().to_bytes(),
            name: "alpha world".to_string(),
            description: "a small test world".to_string(),
            subject: Subject::World {
                node_id: [9; 32],
                relay_hint: Some("https://relay.example.com".to_string()),
            },
            occupancy: Occupancy::HasRoom,
            tags: vec!["coop".to_string(), "pvp".to_string()],
            visibility: Visibility::Listed,
            issued_at: 1_763_000_000,
            expires_at: 1_763_000_000 + 3_600,
            seq: 41,
        }
    }

    fn sample_program() -> Announcement {
        let key = world_key();
        Announcement {
            subject: Subject::Program {
                content_hash: [3; 32],
            },
            ..sample_world()
        }
    }

    #[test]
    fn kat_world_announcement_body_and_signature() {
        // Known-answer vector: the canonical body bytes and the
        // signature over them are pinned, so any drift in the
        // field set, the encoding profile, or the signed bytes
        // breaks this test instead of silently changing the wire
        // format. (ed25519 signing is deterministic, RFC 8032.)
        let announcement = sample_world();
        let body = announcement.encode().expect("encode");
        assert_eq!(
            hex_encode(&body),
            "__KAT_WORLD_BODY__",
            "world announcement body KAT"
        );
        let signed = announcement.sign(&world_key()).expect("sign");
        assert_eq!(
            hex_encode(&signed.signature),
            "__KAT_WORLD_SIG__",
            "world announcement signature KAT"
        );
        assert_eq!(
            hex_encode(&signed.to_bytes().expect("packet")),
            "__KAT_WORLD_PACKET__",
            "world announcement packet KAT"
        );
    }

    #[test]
    fn kat_program_announcement_body() {
        // The program variant: same object, content hash in place
        // of the endpoint — pinned as its own vector.
        let body = sample_program().encode().expect("encode");
        assert_eq!(
            hex_encode(&body),
            "__KAT_PROGRAM_BODY__",
            "program announcement body KAT"
        );
    }

    #[test]
    fn roundtrip_world_and_program() {
        for announcement in [sample_world(), sample_program()] {
            let decoded =
                Announcement::decode(&announcement.encode().expect("encode")).expect("decode");
            assert_eq!(decoded, announcement);
            let signed = announcement.sign(&world_key()).expect("sign");
            let parsed =
                SignedAnnouncement::from_bytes(&signed.to_bytes().expect("packet")).expect("parse");
            assert_eq!(parsed, signed);
            parsed.verify().expect("verify");
            parsed
                .verify_at(announcement.issued_at + 60)
                .expect("verify before expiry");
        }
    }

    #[test]
    fn over_budget_announcement_is_rejected() {
        // Every field at its char cap, with 4-byte chars: the caps
        // pass, the byte budget does not — the encoding must
        // refuse rather than emit a packet pkarr cannot carry.
        let mut announcement = sample_world();
        announcement.name = "\u{1F600}".repeat(MAX_NAME_CHARS);
        announcement.description = "\u{1F600}".repeat(MAX_DESCRIPTION_CHARS);
        assert!(matches!(
            announcement.encode(),
            Err(AnnouncementError::OverBudget { .. })
        ));
        assert!(matches!(
            announcement.sign(&world_key()),
            Err(AnnouncementError::OverBudget { .. })
        ));
        // And an over-budget byte string is refused at decode.
        assert!(matches!(
            Announcement::decode(&vec![0u8; MAX_ANNOUNCEMENT_BYTES + 1]),
            Err(AnnouncementError::OverBudget { .. })
        ));
    }

    #[test]
    fn expired_announcement_is_rejected() {
        let announcement = sample_world();
        let signed = announcement.sign(&world_key()).expect("sign");
        assert!(matches!(
            signed.verify_at(announcement.expires_at),
            Err(AnnouncementError::Expired { .. })
        ));
        assert!(matches!(
            announcement.validate_at(announcement.expires_at + 1),
            Err(AnnouncementError::Expired { .. })
        ));
    }

    #[test]
    fn lifetime_over_seven_days_is_rejected() {
        let mut announcement = sample_world();
        announcement.expires_at = announcement.issued_at + MAX_LIFETIME_SECS + 1;
        assert!(matches!(
            announcement.validate(),
            Err(AnnouncementError::LifetimeTooLong)
        ));
        announcement.expires_at = announcement.issued_at + MAX_LIFETIME_SECS;
        assert!(announcement.validate().is_ok());
        announcement.expires_at = announcement.issued_at;
        assert!(matches!(
            announcement.validate(),
            Err(AnnouncementError::InvalidWindow)
        ));
    }

    #[test]
    fn seq_regression_is_rejected() {
        let announcement = sample_world();
        assert!(announcement.check_seq_after(announcement.seq - 1).is_ok());
        assert!(matches!(
            announcement.check_seq_after(announcement.seq),
            Err(AnnouncementError::SeqRegression { .. })
        ));
        assert!(matches!(
            announcement.check_seq_after(announcement.seq + 1),
            Err(AnnouncementError::SeqRegression { .. })
        ));
    }

    #[test]
    fn field_caps_are_rejected() {
        let mut announcement = sample_world();
        announcement.name = "n".repeat(MAX_NAME_CHARS + 1);
        assert!(matches!(
            announcement.validate(),
            Err(AnnouncementError::NameTooLong)
        ));
        announcement = sample_world();
        announcement.description = "d".repeat(MAX_DESCRIPTION_CHARS + 1);
        assert!(matches!(
            announcement.validate(),
            Err(AnnouncementError::DescriptionTooLong)
        ));
        announcement = sample_world();
        announcement.tags = (0..=MAX_TAGS).map(|i| format!("tag{i}")).collect();
        assert!(matches!(
            announcement.validate(),
            Err(AnnouncementError::TooManyTags)
        ));
        announcement = sample_world();
        announcement.tags = vec!["t".repeat(MAX_TAG_CHARS + 1)];
        assert!(matches!(
            announcement.validate(),
            Err(AnnouncementError::TagTooLong)
        ));
    }

    #[test]
    fn tampered_packet_fails_verification() {
        let mut signed = sample_world().sign(&world_key()).expect("sign");
        signed.announcement.name = "forged world".to_string();
        assert!(matches!(signed.verify(), Err(AnnouncementError::BadSignature)));
        let mut signed = sample_world().sign(&world_key()).expect("sign");
        signed.signature[0] ^= 0xff;
        assert!(matches!(signed.verify(), Err(AnnouncementError::BadSignature)));
    }

    #[test]
    fn signing_with_another_key_is_refused() {
        let other = SigningKey::from_bytes(&[8; 32]);
        assert!(matches!(
            sample_world().sign(&other),
            Err(AnnouncementError::WorldKeyMismatch)
        ));
    }

    #[test]
    fn unknown_and_missing_fields_are_rejected() {
        // An extra field breaks the exact section-3 field set.
        let mut value = sample_world().to_value();
        let Value::Map(entries) = &mut value else {
            panic!("body is a map");
        };
        entries.push((text("surprise"), Value::UInt(1)));
        let bytes = encode_canonical(&value).expect("encode");
        assert!(matches!(
            Announcement::decode(&bytes),
            Err(AnnouncementError::Malformed(_))
        ));
        // A missing field likewise.
        let mut value = sample_world().to_value();
        let Value::Map(entries) = &mut value else {
            panic!("body is a map");
        };
        entries.retain(|(k, _)| *k != text("seq"));
        let bytes = encode_canonical(&value).expect("encode");
        assert!(matches!(
            Announcement::decode(&bytes),
            Err(AnnouncementError::Malformed(_))
        ));
        // Trailing garbage after a valid body is not canonical.
        let mut bytes = sample_world().encode().expect("encode");
        bytes.push(0);
        assert!(Announcement::decode(&bytes).is_err());
    }
}
