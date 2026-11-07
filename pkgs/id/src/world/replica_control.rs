//! Signed cursor-bound replica refreshes and Policy B admission leases.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

use super::ReplicationCursor;

const LEASE_DOMAIN: &[u8] = b"id-world-replica-admission-lease-v1";
const HEADER_DOMAIN: &[u8] = b"id-world-replica-refresh-header-v1";
const CHUNK_DOMAIN: &[u8] = b"id-world-replica-refresh-chunk-v1";

/// Maximum writer-issued admission lease lifetime.
pub const MAX_ADMISSION_LEASE_SECS: u64 = 300;
/// Maximum encoded sidecar refresh transfer.
pub const MAX_REPLICA_REFRESH_BYTES: usize = 64 * 1024 * 1024;
/// Chunk size stays comfortably within the common one-megabyte frame limit.
pub const REPLICA_REFRESH_CHUNK_BYTES: usize = 64 * 1024;

/// Writer-signed Policy B admission lease for one exact verified cursor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedAdmissionLease {
    /// Fields covered by the Ed25519 signature.
    pub body: AdmissionLeaseBody,
    /// Ed25519 signature bytes.
    pub signature: Vec<u8>,
}

/// Admission predicate captured by a serving replica. The gate is checked
/// once when a new world session resolves, never for frames in an already
/// verified session.
#[derive(Clone, Debug)]
pub struct ReplicaAdmission {
    source: [u8; 32],
    world_id: String,
    cursor: ReplicationCursor,
    lease: SignedAdmissionLease,
}

impl ReplicaAdmission {
    /// Build an admission gate for a fully verified, immutable service view.
    #[must_use]
    pub fn new(
        source: [u8; 32],
        world_id: String,
        cursor: ReplicationCursor,
        lease: SignedAdmissionLease,
    ) -> Self {
        Self {
            source,
            world_id,
            cursor,
            lease,
        }
    }

    /// Check whether a newly opened world session may be admitted.
    pub fn check_new_session(&self) -> Result<()> {
        self.check_new_session_at(now_unix_secs()?)
    }

    /// Deterministic lease check used by acceptance tests and operators that
    /// compare a recorded lease against an explicit clock reading.
    pub fn check_new_session_at(&self, now_unix_secs: u64) -> Result<()> {
        self.lease.verify_at(
            self.source,
            &self.world_id,
            self.cursor,
            now_unix_secs,
        )
    }
}

/// Canonical signed lease fields.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionLeaseBody {
    /// Lease contract version.
    pub version: u8,
    /// The world this lease covers.
    pub world_id: String,
    /// Pinned writer node identity.
    pub source: [u8; 32],
    /// Exact journal cursor for which new sessions are authorized.
    pub cursor: ReplicationCursor,
    /// Lease issue time in Unix seconds.
    pub issued_at_unix_secs: u64,
    /// Lease expiry time in Unix seconds; new sessions are denied at expiry.
    pub expires_at_unix_secs: u64,
}

impl SignedAdmissionLease {
    /// Issue the implementation's conservative five-minute lease.
    pub fn issue(
        world_id: &str,
        source: [u8; 32],
        cursor: ReplicationCursor,
        now_unix_secs: u64,
        key: &SigningKey,
    ) -> Result<Self> {
        let body = AdmissionLeaseBody {
            version: 1,
            world_id: world_id.to_owned(),
            source,
            cursor,
            issued_at_unix_secs: now_unix_secs,
            expires_at_unix_secs: now_unix_secs.saturating_add(MAX_ADMISSION_LEASE_SECS),
        };
        let signature = sign(LEASE_DOMAIN, &body, key)?;
        Ok(Self { body, signature })
    }

    /// Verify signer pin, world, exact cursor and finite validity window.
    pub fn verify_at(
        &self,
        expected_source: [u8; 32],
        expected_world: &str,
        expected_cursor: ReplicationCursor,
        now_unix_secs: u64,
    ) -> Result<()> {
        ensure!(self.body.version == 1, "unsupported admission lease version");
        ensure!(self.body.source == expected_source, "admission lease source mismatch");
        ensure!(self.body.world_id == expected_world, "admission lease world mismatch");
        ensure!(self.body.cursor == expected_cursor, "admission lease cursor mismatch");
        ensure!(
            self.body.expires_at_unix_secs > self.body.issued_at_unix_secs,
            "admission lease validity window is empty"
        );
        ensure!(
            self.body
                .expires_at_unix_secs
                .saturating_sub(self.body.issued_at_unix_secs)
                <= MAX_ADMISSION_LEASE_SECS,
            "admission lease lifetime exceeds the maximum"
        );
        ensure!(
            self.body.issued_at_unix_secs <= now_unix_secs.saturating_add(30),
            "admission lease was issued in the future"
        );
        ensure!(now_unix_secs < self.body.expires_at_unix_secs, "admission lease expired");
        verify(LEASE_DOMAIN, &self.body, &self.signature, &expected_source)
    }

    /// Verify the signed lease binding without requiring it to remain
    /// unexpired. This is used when opening a local replica for status or
    /// recovery; serving always calls [`Self::verify_at`] first.
    pub fn verify_signature(
        &self,
        expected_source: [u8; 32],
        expected_world: &str,
        expected_cursor: ReplicationCursor,
    ) -> Result<()> {
        ensure!(self.body.version == 1, "unsupported admission lease version");
        ensure!(self.body.source == expected_source, "admission lease source mismatch");
        ensure!(self.body.world_id == expected_world, "admission lease world mismatch");
        ensure!(self.body.cursor == expected_cursor, "admission lease cursor mismatch");
        ensure!(
            self.body.expires_at_unix_secs > self.body.issued_at_unix_secs
                && self.body.expires_at_unix_secs.saturating_sub(self.body.issued_at_unix_secs)
                    <= MAX_ADMISSION_LEASE_SECS,
            "admission lease validity window is invalid"
        );
        verify(LEASE_DOMAIN, &self.body, &self.signature, &expected_source)
    }

    /// Verify at the current system time.
    pub fn verify_now(
        &self,
        expected_source: [u8; 32],
        expected_world: &str,
        expected_cursor: ReplicationCursor,
    ) -> Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock is before Unix epoch")?
            .as_secs();
        self.verify_at(expected_source, expected_world, expected_cursor, now)
    }
}

/// Authenticated data needed by a durable replica at the signed cursor.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplicaRefreshPayload {
    /// Active module hash named by the journal's restored program state.
    pub module_hash: Option<String>,
    /// Module bytes whose hash must equal `module_hash`.
    pub module_bytes: Option<Vec<u8>>,
    /// Complete authorization facts for subjects in the replicated journal.
    pub directory_facts: Vec<crate::directory::DirectoryEntry>,
    /// Writer-signed Policy B lease for this cursor.
    pub admission_lease: SignedAdmissionLease,
}

/// Canonical signed transfer metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicaRefreshHeaderBody {
    /// Refresh protocol version.
    pub version: u8,
    /// Replicated world ID.
    pub world_id: String,
    /// Pinned source node ID.
    pub source: [u8; 32],
    /// Fresh request nonce.
    pub nonce: [u8; 16],
    /// Exact verified journal cursor bound to this refresh.
    pub cursor: ReplicationCursor,
    /// Total encoded payload length.
    pub payload_len: u64,
    /// Number of signed chunks following the header.
    pub chunk_count: u32,
    /// BLAKE3 digest of the full encoded payload.
    pub payload_digest: [u8; 32],
}

/// Signed refresh header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedReplicaRefreshHeader {
    /// Signed header body.
    pub body: ReplicaRefreshHeaderBody,
    /// Ed25519 signature bytes.
    pub signature: Vec<u8>,
}

impl SignedReplicaRefreshHeader {
    /// Sign one transfer header.
    pub fn sign(body: ReplicaRefreshHeaderBody, key: &SigningKey) -> Result<Self> {
        Ok(Self {
            signature: sign(HEADER_DOMAIN, &body, key)?,
            body,
        })
    }

    /// Check trust and the request context before allocating the payload.
    pub fn verify(
        &self,
        source: [u8; 32],
        world: &str,
        nonce: [u8; 16],
        cursor: ReplicationCursor,
    ) -> Result<()> {
        ensure!(self.body.version == 1, "unsupported refresh version");
        ensure!(self.body.source == source, "refresh source mismatch");
        ensure!(self.body.world_id == world, "refresh world mismatch");
        ensure!(self.body.nonce == nonce, "refresh nonce mismatch");
        ensure!(self.body.cursor == cursor, "refresh cursor mismatch");
        let payload_len = usize::try_from(self.body.payload_len).context("refresh payload length")?;
        ensure!(payload_len <= MAX_REPLICA_REFRESH_BYTES, "refresh payload exceeds bound");
        let expected_chunks = payload_len.div_ceil(REPLICA_REFRESH_CHUNK_BYTES);
        ensure!(
            usize::try_from(self.body.chunk_count).ok() == Some(expected_chunks),
            "refresh chunk count does not match payload length"
        );
        verify(HEADER_DOMAIN, &self.body, &self.signature, &source)
    }
}

/// A single cursor- and nonce-bound transfer chunk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicaRefreshChunkBody {
    /// Chunk protocol version.
    pub version: u8,
    /// Replicated world ID.
    pub world_id: String,
    /// Pinned source node ID.
    pub source: [u8; 32],
    /// Fresh request nonce.
    pub nonce: [u8; 16],
    /// Verified cursor for the transfer.
    pub cursor: ReplicationCursor,
    /// Zero-based chunk index.
    pub index: u32,
    /// Chunk bytes.
    pub payload: Vec<u8>,
}

/// Signed refresh chunk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedReplicaRefreshChunk {
    /// Signed chunk body.
    pub body: ReplicaRefreshChunkBody,
    /// Ed25519 signature bytes.
    pub signature: Vec<u8>,
}

impl SignedReplicaRefreshChunk {
    /// Sign one chunk.
    pub fn sign(body: ReplicaRefreshChunkBody, key: &SigningKey) -> Result<Self> {
        Ok(Self {
            signature: sign(CHUNK_DOMAIN, &body, key)?,
            body,
        })
    }

    /// Verify source, transfer context, ordering, and per-frame bound.
    pub fn verify(
        &self,
        source: [u8; 32],
        world: &str,
        nonce: [u8; 16],
        cursor: ReplicationCursor,
        index: u32,
        count: u32,
    ) -> Result<()> {
        ensure!(self.body.version == 1, "unsupported refresh chunk version");
        ensure!(self.body.source == source, "refresh chunk source mismatch");
        ensure!(self.body.world_id == world, "refresh chunk world mismatch");
        ensure!(self.body.nonce == nonce, "refresh chunk nonce mismatch");
        ensure!(self.body.cursor == cursor, "refresh chunk cursor mismatch");
        ensure!(self.body.index == index && index < count, "refresh chunk ordering mismatch");
        ensure!(
            self.body.payload.len() <= REPLICA_REFRESH_CHUNK_BYTES,
            "refresh chunk exceeds bound"
        );
        verify(CHUNK_DOMAIN, &self.body, &self.signature, &source)
    }
}

/// Signed refresh plus its exact serialized payload, stored in each committed
/// replica generation so open/serve can revalidate every sidecar.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplicaRefreshRecord {
    /// Signed transfer header.
    pub header: SignedReplicaRefreshHeader,
    /// Exact postcard-encoded refresh payload.
    pub payload_bytes: Vec<u8>,
}

impl ReplicaRefreshRecord {
    /// Verify signed bytes and decode the authenticated payload.
    pub fn verify_decode(
        &self,
        source: [u8; 32],
        world: &str,
        cursor: ReplicationCursor,
    ) -> Result<ReplicaRefreshPayload> {
        self.header.verify(
            source,
            world,
            self.header.body.nonce,
            cursor,
        )?;
        ensure!(
            self.header.body.payload_len
                == u64::try_from(self.payload_bytes.len()).context("refresh payload length")?,
            "refresh manifest payload length mismatch"
        );
        ensure!(
            blake3::hash(&self.payload_bytes).as_bytes() == &self.header.body.payload_digest,
            "refresh manifest payload digest mismatch"
        );
        postcard::from_bytes(&self.payload_bytes).context("decode authenticated replica refresh")
    }
}

/// Build a signed header and individually signed chunks for a refresh.
pub fn sign_refresh_transfer(
    world: &str,
    source: [u8; 32],
    nonce: [u8; 16],
    cursor: ReplicationCursor,
    payload_bytes: &[u8],
    key: &SigningKey,
) -> Result<(SignedReplicaRefreshHeader, Vec<SignedReplicaRefreshChunk>)> {
    ensure!(payload_bytes.len() <= MAX_REPLICA_REFRESH_BYTES, "refresh payload exceeds bound");
    let chunk_count = u32::try_from(payload_bytes.len().div_ceil(REPLICA_REFRESH_CHUNK_BYTES))
        .context("refresh has too many chunks")?;
    let header = SignedReplicaRefreshHeader::sign(
        ReplicaRefreshHeaderBody {
            version: 1,
            world_id: world.to_owned(),
            source,
            nonce,
            cursor,
            payload_len: u64::try_from(payload_bytes.len()).context("refresh payload length")?,
            chunk_count,
            payload_digest: *blake3::hash(payload_bytes).as_bytes(),
        },
        key,
    )?;
    let chunks = payload_bytes
        .chunks(REPLICA_REFRESH_CHUNK_BYTES)
        .enumerate()
        .map(|(index, payload)| {
            Ok(SignedReplicaRefreshChunk::sign(
                ReplicaRefreshChunkBody {
                    version: 1,
                    world_id: world.to_owned(),
                    source,
                    nonce,
                    cursor,
                    index: u32::try_from(index).context("refresh chunk index")?,
                    payload: payload.to_vec(),
                },
                key,
            )?)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((header, chunks))
}

fn sign<T: Serialize>(domain: &[u8], body: &T, key: &SigningKey) -> Result<Vec<u8>> {
    let bytes = postcard::to_allocvec(&(domain, body)).context("encode signed replica control")?;
    Ok(key.sign(&bytes).to_bytes().to_vec())
}

fn verify<T: Serialize>(domain: &[u8], body: &T, signature: &[u8], source: &[u8; 32]) -> Result<()> {
    let key = VerifyingKey::from_bytes(source).context("invalid pinned source key")?;
    let signature = Signature::from_slice(signature).context("invalid source signature encoding")?;
    let bytes = postcard::to_allocvec(&(domain, body)).context("encode signed replica control")?;
    key.verify_strict(&bytes, &signature)
        .context("replica control signature verification failed")
}

/// Unix seconds used by the lease boundary.
pub fn now_unix_secs() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_secs())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::world::{chain_base_cursor, frame_digest};

    #[test]
    fn policy_b_lease_is_pinned_cursor_bound_and_expires() {
        let key = SigningKey::from_bytes(&[9; 32]);
        let source = key.verifying_key().to_bytes();
        let cursor = chain_base_cursor("lobby", &source, 1);
        let lease = SignedAdmissionLease::issue("lobby", source, cursor, 100, &key).unwrap();

        assert!(lease.verify_at(source, "lobby", cursor, 101).is_ok());
        assert!(lease.verify_at([8; 32], "lobby", cursor, 101).is_err());
        assert!(lease.verify_at(source, "other", cursor, 101).is_err());
        assert!(lease.verify_at(source, "lobby", cursor, 100 + MAX_ADMISSION_LEASE_SECS).is_err());
        assert!(lease.verify_at(source, "lobby", frame_cursor(cursor), 101).is_err());

        let gate = ReplicaAdmission::new(source, "lobby".to_owned(), cursor, lease);
        assert!(gate.check_new_session_at(101).is_ok());
        assert!(gate
            .check_new_session_at(100 + MAX_ADMISSION_LEASE_SECS)
            .is_err());
    }

    #[test]
    fn refresh_chunks_are_nonce_cursor_order_and_digest_bound() {
        let key = SigningKey::from_bytes(&[6; 32]);
        let source = key.verifying_key().to_bytes();
        let cursor = chain_base_cursor("lobby", &source, 1);
        let nonce = [3; 16];
        let payload = vec![4; REPLICA_REFRESH_CHUNK_BYTES + 7];
        let (header, chunks) =
            sign_refresh_transfer("lobby", source, nonce, cursor, &payload, &key).unwrap();

        header.verify(source, "lobby", nonce, cursor).unwrap();
        assert!(header.verify(source, "lobby", [5; 16], cursor).is_err());
        let mut restored = Vec::new();
        for (index, chunk) in chunks.iter().enumerate() {
            chunk
                .verify(
                    source,
                    "lobby",
                    nonce,
                    cursor,
                    u32::try_from(index).unwrap(),
                    header.body.chunk_count,
                )
                .unwrap();
            restored.extend_from_slice(&chunk.body.payload);
        }
        assert_eq!(restored, payload);
        assert!(chunks[0]
            .verify(source, "lobby", nonce, cursor, 1, header.body.chunk_count)
            .is_err());

        let mut altered = chunks[0].clone();
        altered.body.payload[0] ^= 1;
        assert!(altered
            .verify(source, "lobby", nonce, cursor, 0, header.body.chunk_count)
            .is_err());
    }

    #[test]
    fn signed_refresh_record_revalidates_exact_payload() {
        let key = SigningKey::from_bytes(&[2; 32]);
        let source = key.verifying_key().to_bytes();
        let cursor = chain_base_cursor("lobby", &source, 1);
        let lease = SignedAdmissionLease::issue("lobby", source, cursor, 100, &key).unwrap();
        let payload = ReplicaRefreshPayload {
            module_hash: None,
            module_bytes: None,
            directory_facts: Vec::new(),
            admission_lease: lease,
        };
        let payload_bytes = postcard::to_allocvec(&payload).unwrap();
        let (header, _) =
            sign_refresh_transfer("lobby", source, [1; 16], cursor, &payload_bytes, &key).unwrap();
        let record = ReplicaRefreshRecord {
            header,
            payload_bytes,
        };
        assert!(record.verify_decode(source, "lobby", cursor).is_ok());
        let mut tampered = record.clone();
        tampered.payload_bytes.push(0);
        assert!(tampered.verify_decode(source, "lobby", cursor).is_err());
    }

    fn frame_cursor(cursor: ReplicationCursor) -> ReplicationCursor {
        ReplicationCursor {
            ordinal: cursor.ordinal.saturating_add(1),
            digest: frame_digest(&cursor.digest),
        }
    }
}
