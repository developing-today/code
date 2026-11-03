//! Versioned authentication for peer-discovery gossip announcements.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::discovery::{MAX_PEER_NAME_BYTES, PeerAnnouncement};

/// Magic prefix separates authenticated v2 gossip frames from legacy postcard payloads.
pub(crate) const PEER_ANNOUNCEMENT_FRAME_MAGIC: &[u8] = b"id-peer-announcement/v2\n";

const PEER_ANNOUNCEMENT_VERSION: u8 = 2;
const SIGNATURE_DOMAIN: &[u8] = b"developing-today/id/peer-announcement/v2\0";
const MAX_PEER_ANNOUNCEMENT_FRAME_BYTES: usize = 1_024;

/// Signed gossip envelope. The plain `PeerAnnouncement` remains unchanged for the
/// `/iroh-meta/2` response format; only gossip uses this versioned wrapper.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct SignedPeerAnnouncement {
    version: u8,
    announcement: PeerAnnouncement,
    signature: Vec<u8>,
}

impl SignedPeerAnnouncement {
    /// Sign an announcement, refusing to create a claim for another endpoint key.
    pub(crate) fn sign(announcement: PeerAnnouncement, key: &SigningKey) -> Option<Self> {
        if announcement.node_id.as_bytes() != key.verifying_key().as_bytes()
            || announcement
                .name
                .as_ref()
                .is_some_and(|name| name.len() > MAX_PEER_NAME_BYTES)
        {
            return None;
        }

        let mut signed = Self {
            version: PEER_ANNOUNCEMENT_VERSION,
            announcement,
            signature: Vec::new(),
        };
        signed.signature = key.sign(&signed.signing_bytes().ok()?).to_bytes().to_vec();
        Some(signed)
    }

    /// Encode this envelope with its versioned frame marker.
    pub(crate) fn encode_frame(&self) -> Result<Vec<u8>, postcard::Error> {
        let payload = postcard::to_allocvec(self)?;
        let mut frame = Vec::with_capacity(PEER_ANNOUNCEMENT_FRAME_MAGIC.len() + payload.len());
        frame.extend_from_slice(PEER_ANNOUNCEMENT_FRAME_MAGIC);
        frame.extend_from_slice(&payload);
        Ok(frame)
    }

    /// Decode and authenticate a gossip frame. Unsigned legacy payloads, unsupported
    /// versions, malformed frames, and forged claims all return `None`.
    pub(crate) fn decode_frame(frame: &[u8]) -> Option<PeerAnnouncement> {
        if frame.len() > MAX_PEER_ANNOUNCEMENT_FRAME_BYTES {
            return None;
        }
        let payload = frame.strip_prefix(PEER_ANNOUNCEMENT_FRAME_MAGIC)?;
        let signed = postcard::from_bytes::<Self>(payload).ok()?;
        signed.verify()
    }

    fn verify(&self) -> Option<PeerAnnouncement> {
        if self.version != PEER_ANNOUNCEMENT_VERSION
            || self
                .announcement
                .name
                .as_ref()
                .is_some_and(|name| name.len() > MAX_PEER_NAME_BYTES)
        {
            return None;
        }

        let verifying_key = VerifyingKey::from_bytes(self.announcement.node_id.as_bytes()).ok()?;
        let signature = Signature::from_slice(&self.signature).ok()?;
        verifying_key
            .verify_strict(&self.signing_bytes().ok()?, &signature)
            .ok()?;
        Some(self.announcement.clone())
    }

    /// Domain-separated canonical postcard encoding of every claimed field.
    fn signing_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(&(
            SIGNATURE_DOMAIN,
            self.version,
            &self.announcement.node_id,
            &self.announcement.name,
            self.announcement.blob_count,
            self.announcement.timestamp_secs,
        ))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use iroh_base::SecretKey;

    fn signing_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn announcement(key: &SigningKey) -> PeerAnnouncement {
        let secret = SecretKey::from_bytes(&key.to_bytes());
        PeerAnnouncement {
            node_id: secret.public(),
            name: Some("peer".to_string()),
            blob_count: 12,
            timestamp_secs: 1_700_000_000,
        }
    }

    #[test]
    fn signed_frame_round_trips_and_rejects_legacy_unsigned_bytes() {
        let key = signing_key(1);
        let announcement = announcement(&key);
        let signed = SignedPeerAnnouncement::sign(announcement.clone(), &key).unwrap();
        let frame = signed.encode_frame().unwrap();

        assert_eq!(
            SignedPeerAnnouncement::decode_frame(&frame),
            Some(announcement.clone())
        );
        assert!(
            SignedPeerAnnouncement::decode_frame(&postcard::to_allocvec(&announcement).unwrap())
                .is_none()
        );
    }

    #[test]
    fn signature_binds_every_claim_field_and_the_version() {
        let key = signing_key(2);
        let signed = SignedPeerAnnouncement::sign(announcement(&key), &key).unwrap();

        let mut tampered_name = signed.clone();
        tampered_name.announcement.name = Some("forged".to_string());
        assert!(tampered_name.verify().is_none());

        let mut tampered_count = signed.clone();
        tampered_count.announcement.blob_count += 1;
        assert!(tampered_count.verify().is_none());

        let mut tampered_timestamp = signed.clone();
        tampered_timestamp.announcement.timestamp_secs += 1;
        assert!(tampered_timestamp.verify().is_none());

        let mut tampered_node = signed.clone();
        tampered_node.announcement.node_id = SecretKey::from_bytes(&[3; 32]).public();
        assert!(tampered_node.verify().is_none());

        let mut unsupported_version = signed;
        unsupported_version.version = PEER_ANNOUNCEMENT_VERSION + 1;
        assert!(unsupported_version.verify().is_none());
    }

    #[test]
    fn signer_must_match_the_claimed_endpoint_id() {
        let claim_key = signing_key(4);
        let other_key = signing_key(5);
        assert!(SignedPeerAnnouncement::sign(announcement(&claim_key), &other_key).is_none());
    }

    #[test]
    fn oversized_names_and_frames_are_rejected() {
        let key = signing_key(6);
        let mut announcement = announcement(&key);
        announcement.name = Some("x".repeat(MAX_PEER_NAME_BYTES + 1));
        assert!(SignedPeerAnnouncement::sign(announcement, &key).is_none());

        let mut oversized_frame = PEER_ANNOUNCEMENT_FRAME_MAGIC.to_vec();
        oversized_frame.resize(MAX_PEER_ANNOUNCEMENT_FRAME_BYTES + 1, 0);
        assert!(SignedPeerAnnouncement::decode_frame(&oversized_frame).is_none());
    }
}
