//! Writer-signed admission leases for read replicas (F12.13 phase 5,
//! policy B).
//!
//! Design §8 of `doc/2026-11-01_design_id806.md` left one decision open:
//! whether a replica may admit NEW spectators while the writer is
//! unreachable, given it cannot learn about revocations made after its
//! last verified journal cursor. The ruling on #806 (comments 6599883036
//! and 6634842315) adopted **policy B**:
//!
//! - A replica may admit new spectators during a writer outage only
//!   under a **writer-signed admission lease**.
//! - When the lease expires, new admissions stop until the replica
//!   resyncs with the writer (and obtains a fresh lease).
//! - The lease duration is configurable at issuance.
//! - Existing verified sessions and read serving at the last verified
//!   cursor are unaffected by lease expiry — the lease gates only the
//!   admission of new spectators.
//!
//! This module is the lease primitive: the signed payload, its
//! domain-separated canonical encoding, issuance, and verification
//! against a replica's pinned source. [`ReplicaAdmissionGate`] is the
//! enforcement seam: the world actor's token-admission path checks it
//! for read-only replicas served with a gate (see
//! `world::serve_replica_with_admission`), so a new spectator is
//! admitted offline only when [`SignedAdmissionLease::verify`] passes
//! for the current instant. The operator CLI that fetches the stream
//! and the lease into a replica directory is the remaining phase 5
//! surface.
//!
//! Security properties, mirroring the replication envelope's rules:
//!
//! - The signature binds every lease field under a dedicated domain
//!   tag, so a lease can never be reinterpreted as a replication frame
//!   (or vice versa), and no field can be altered after signing —
//!   extending `expires_at_unix_ms` invalidates the signature.
//! - Verification requires the signer to be the replica's pinned
//!   source, the lease's world to be the replica's world, and the
//!   current time to lie inside `[issued_at_unix_ms,
//!   expires_at_unix_ms)`.
//! - The signed duration is bounded by
//!   [`MAX_ADMISSION_LEASE_DURATION_MS`] at both issuance and
//!   verification: policy B exists to bound revocation staleness, so
//!   an over-long lease is refused even if its signature is valid.

use std::collections::BTreeSet;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

/// Domain separator bound into every admission-lease signature, so a
/// lease signature can never be reinterpreted by another protocol in
/// this crate (design §4.3's domain-separation rule, applied to the
/// phase 5 lease).
pub const ADMISSION_LEASE_DOMAIN: &[u8] = b"developing-today/id/world-admission-lease/v1\0";

/// Version of the lease payload defined by this module. A verifier
/// refuses any other version; a future incompatible contract bumps
/// this (and the domain tag with it).
pub const ADMISSION_LEASE_VERSION: u8 = 1;

/// Longest duration a single admission lease may span: 7 days. Policy
/// B bounds how long a replica may keep admitting new spectators
/// without resyncing; a writer that wants a longer offline window
/// re-issues on resync rather than signing a longer lease.
pub const MAX_ADMISSION_LEASE_DURATION_MS: u64 = 7 * 24 * 60 * 60 * 1000;

/// The signed payload of an admission lease: the writer (the lease's
/// `source`, the same key that signs the world's replication stream)
/// authorizes replicas of `world_id` to admit new spectators from
/// `issued_at_unix_ms` until `expires_at_unix_ms` (exclusive).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionLease {
    /// Lease payload version; must equal [`ADMISSION_LEASE_VERSION`].
    pub version: u8,
    /// The world this lease authorizes offline admissions for.
    pub world_id: String,
    /// Public key of the issuing writer; must equal the replica's
    /// pinned source for the lease to verify.
    pub source: [u8; 32],
    /// Start of the admission window, unix milliseconds.
    pub issued_at_unix_ms: u64,
    /// End of the admission window (exclusive), unix milliseconds.
    pub expires_at_unix_ms: u64,
}

impl AdmissionLease {
    /// Length of the signed admission window in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        self.expires_at_unix_ms
            .saturating_sub(self.issued_at_unix_ms)
    }
}

/// An [`AdmissionLease`] plus its Ed25519 signature — the artifact a
/// writer hands a replica (at sync time, alongside the journal feed)
/// and the replica checks before admitting a new spectator while the
/// writer is unreachable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedAdmissionLease {
    /// The lease; every field is signed.
    pub lease: AdmissionLease,
    /// Ed25519 signature over [`Self::signing_bytes`], by the key whose
    /// public half is `lease.source`.
    pub signature: Vec<u8>,
}

impl SignedAdmissionLease {
    /// Issue and sign a lease for `world_id` starting at `now_unix_ms`
    /// and lasting `duration_ms` (the configurable lease duration of
    /// the policy B ruling).
    ///
    /// Returns `None` — issuing nothing — when the duration is zero,
    /// exceeds [`MAX_ADMISSION_LEASE_DURATION_MS`], or the window end
    /// overflows: a writer never mints a lease a verifier would refuse
    /// on duration grounds.
    pub fn issue(
        world_id: &str,
        key: &SigningKey,
        now_unix_ms: u64,
        duration_ms: u64,
    ) -> Option<Self> {
        if duration_ms == 0 || duration_ms > MAX_ADMISSION_LEASE_DURATION_MS {
            return None;
        }
        let expires_at_unix_ms = now_unix_ms.checked_add(duration_ms)?;
        let mut signed = Self {
            lease: AdmissionLease {
                version: ADMISSION_LEASE_VERSION,
                world_id: world_id.to_owned(),
                source: *key.verifying_key().as_bytes(),
                issued_at_unix_ms: now_unix_ms,
                expires_at_unix_ms,
            },
            signature: Vec::new(),
        };
        signed.signature = key.sign(&signed.signing_bytes().ok()?).to_bytes().to_vec();
        Some(signed)
    }

    /// Verify this lease for a replica of `world_id` that pins
    /// `pinned_source` as its writer, at time `now_unix_ms`.
    ///
    /// # Errors
    ///
    /// The first failing clause of the policy B admission rule, as an
    /// [`AdmissionLeaseError`]: unsupported version, a signer other
    /// than the pinned source, a lease for another world, an invalid
    /// signature, a signed duration above the bound, a window that has
    /// not started yet, or a window that has expired (at expiry new
    /// admissions stop until resync — design §8, policy B).
    pub fn verify(
        &self,
        pinned_source: [u8; 32],
        world_id: &str,
        now_unix_ms: u64,
    ) -> Result<(), AdmissionLeaseError> {
        if self.lease.version != ADMISSION_LEASE_VERSION {
            return Err(AdmissionLeaseError::UnsupportedVersion);
        }
        if self.lease.source != pinned_source {
            return Err(AdmissionLeaseError::WrongSource);
        }
        if self.lease.world_id != world_id {
            return Err(AdmissionLeaseError::WrongWorld);
        }
        if self.lease.duration_ms() > MAX_ADMISSION_LEASE_DURATION_MS {
            return Err(AdmissionLeaseError::ExcessiveDuration);
        }
        let verifying_key = VerifyingKey::from_bytes(&self.lease.source)
            .map_err(|_| AdmissionLeaseError::BadSignature)?;
        let signature = Signature::from_slice(&self.signature)
            .map_err(|_| AdmissionLeaseError::BadSignature)?;
        verifying_key
            .verify_strict(
                &self
                    .signing_bytes()
                    .map_err(|_| AdmissionLeaseError::BadSignature)?,
                &signature,
            )
            .map_err(|_| AdmissionLeaseError::BadSignature)?;
        if now_unix_ms < self.lease.issued_at_unix_ms {
            return Err(AdmissionLeaseError::NotYetValid);
        }
        if now_unix_ms >= self.lease.expires_at_unix_ms {
            return Err(AdmissionLeaseError::Expired);
        }
        Ok(())
    }

    /// Policy B in one predicate: may a replica of `world_id` pinned to
    /// `pinned_source` admit a NEW spectator at `now_unix_ms` under
    /// this lease? Existing sessions and read serving are not governed
    /// by this check and must not be gated on it.
    pub fn admits_new_spectator(
        &self,
        pinned_source: [u8; 32],
        world_id: &str,
        now_unix_ms: u64,
    ) -> bool {
        self.verify(pinned_source, world_id, now_unix_ms).is_ok()
    }

    /// Milliseconds of admission window left at `now_unix_ms` (zero
    /// once expired), for operator staleness reporting.
    pub fn remaining_ms(&self, now_unix_ms: u64) -> u64 {
        self.lease.expires_at_unix_ms.saturating_sub(now_unix_ms)
    }

    /// Domain-separated canonical encoding of every lease field: what
    /// the signature binds.
    fn signing_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(&(
            ADMISSION_LEASE_DOMAIN,
            self.lease.version,
            &self.lease.world_id,
            &self.lease.source,
            self.lease.issued_at_unix_ms,
            self.lease.expires_at_unix_ms,
        ))
    }
}

/// Why an admission lease was refused. Every refusal means: do not
/// admit the new spectator under this lease.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AdmissionLeaseError {
    /// The lease payload version is not one this verifier speaks.
    #[error("unsupported admission lease version")]
    UnsupportedVersion,
    /// The lease was issued by a key other than the pinned source.
    #[error("admission lease signer is not the pinned source")]
    WrongSource,
    /// The lease authorizes a different world.
    #[error("admission lease is for a different world")]
    WrongWorld,
    /// The signature is malformed or does not verify over the lease.
    #[error("admission lease signature does not verify")]
    BadSignature,
    /// The signed window is longer than the policy B bound.
    #[error("admission lease duration exceeds the maximum")]
    ExcessiveDuration,
    /// The admission window has not started yet.
    #[error("admission lease is not yet valid")]
    NotYetValid,
    /// The admission window has ended; new admissions stop until
    /// resync (policy B).
    #[error("admission lease has expired")]
    Expired,
    /// The replica holds no admission lease at all: while the writer
    /// is unreachable there is nothing to admit new spectators under.
    #[error("no admission lease held")]
    NoLease,
}

/// The policy-B gate a served replica applies at spectator admission
/// (F12.13 phase 5): participants who were present at the verified
/// journal cursor are existing sessions and are never gated — the
/// ruling leaves existing verified sessions and read serving
/// unaffected — while a participant who was not present is a NEW
/// spectator, admitted only under a currently valid lease from the
/// pinned writer ([`SignedAdmissionLease::verify`]). A gate holding
/// no lease admits no new spectators.
#[derive(Clone, Debug)]
pub struct ReplicaAdmissionGate {
    lease: Option<SignedAdmissionLease>,
    pinned_source: [u8; 32],
    world_id: String,
    present_at_cursor: BTreeSet<u64>,
}

impl ReplicaAdmissionGate {
    /// Build the gate for a replica of `world_id` pinned to
    /// `pinned_source`, from the participants present at its verified
    /// cursor (`present_at_cursor`, the restore's presence set) and
    /// the lease it currently holds, if any.
    pub fn new(
        lease: Option<SignedAdmissionLease>,
        pinned_source: [u8; 32],
        world_id: impl Into<String>,
        present_at_cursor: BTreeSet<u64>,
    ) -> Self {
        Self {
            lease,
            pinned_source,
            world_id: world_id.into(),
            present_at_cursor,
        }
    }

    /// May `participant_id` be admitted at `now_unix_ms`? Existing
    /// participants (present at the cursor) always may; a new
    /// spectator needs the held lease to verify for this instant.
    ///
    /// # Errors
    ///
    /// The lease verification failure, or
    /// [`AdmissionLeaseError::NoLease`] when the gate holds no lease.
    pub fn admit(&self, participant_id: u64, now_unix_ms: u64) -> Result<(), AdmissionLeaseError> {
        if self.present_at_cursor.contains(&participant_id) {
            return Ok(());
        }
        match &self.lease {
            Some(lease) => lease.verify(self.pinned_source, &self.world_id, now_unix_ms),
            None => Err(AdmissionLeaseError::NoLease),
        }
    }

    /// Whether `participant_id` counts as an existing participant
    /// (present at the verified cursor) rather than a new spectator.
    pub fn is_existing(&self, participant_id: u64) -> bool {
        self.present_at_cursor.contains(&participant_id)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const WORLD: &str = "world-lease-test";
    const ISSUED: u64 = 1_700_000_000_000;
    const HOUR_MS: u64 = 60 * 60 * 1000;

    fn writer() -> SigningKey {
        SigningKey::from_bytes(&[7_u8; 32])
    }

    fn source_of(key: &SigningKey) -> [u8; 32] {
        *key.verifying_key().as_bytes()
    }

    fn lease(duration_ms: u64) -> SignedAdmissionLease {
        SignedAdmissionLease::issue(WORLD, &writer(), ISSUED, duration_ms).unwrap()
    }

    /// Sign a hand-built lease (for cases `issue` refuses to mint).
    fn sign(lease: AdmissionLease, key: &SigningKey) -> SignedAdmissionLease {
        let mut signed = SignedAdmissionLease {
            lease,
            signature: Vec::new(),
        };
        signed.signature = key
            .sign(&signed.signing_bytes().unwrap())
            .to_bytes()
            .to_vec();
        signed
    }

    // Oracle: policy B ruling (#806 comment 6634842315) — under a valid
    // writer-signed lease, a replica may admit new spectators during a
    // writer outage.
    #[test]
    fn test_valid_lease_admits_new_spectator_during_the_window() {
        let signed = lease(2 * HOUR_MS);
        assert_eq!(
            signed.verify(source_of(&writer()), WORLD, ISSUED + HOUR_MS),
            Ok(())
        );
        assert!(signed.admits_new_spectator(source_of(&writer()), WORLD, ISSUED));
        assert_eq!(signed.remaining_ms(ISSUED + HOUR_MS), HOUR_MS);
    }

    // Oracle: policy B ruling — when the lease expires, new admissions
    // stop until resync. The expiry instant itself is outside the
    // window (window is [issued, expires)).
    #[test]
    fn test_expired_lease_stops_new_admissions() {
        let signed = lease(HOUR_MS);
        assert_eq!(
            signed.verify(source_of(&writer()), WORLD, ISSUED + HOUR_MS),
            Err(AdmissionLeaseError::Expired)
        );
        assert!(!signed.admits_new_spectator(source_of(&writer()), WORLD, ISSUED + HOUR_MS));
        assert_eq!(signed.remaining_ms(ISSUED + HOUR_MS), 0);
    }

    // Oracle: policy B ruling — the lease is *writer*-signed; a lease
    // from any other key admits nobody, however fresh.
    #[test]
    fn test_lease_from_an_unpinned_signer_is_refused() {
        let stranger = SigningKey::from_bytes(&[9_u8; 32]);
        let signed = SignedAdmissionLease::issue(WORLD, &stranger, ISSUED, HOUR_MS).unwrap();
        assert_eq!(
            signed.verify(source_of(&writer()), WORLD, ISSUED),
            Err(AdmissionLeaseError::WrongSource)
        );
    }

    // Oracle: design §5 invariant 4's binding rule applied to leases —
    // every artifact is bound to its world; a lease minted for one
    // world authorizes no other.
    #[test]
    fn test_lease_for_another_world_is_refused() {
        let signed = lease(HOUR_MS);
        assert_eq!(
            signed.verify(source_of(&writer()), "world-other", ISSUED),
            Err(AdmissionLeaseError::WrongWorld)
        );
    }

    // Oracle: the signature binds every field (module security
    // properties) — extending the expiry after signing, the obvious
    // forgery against policy B, invalidates the lease.
    #[test]
    fn test_tampered_expiry_invalidates_the_signature() {
        let mut signed = lease(HOUR_MS);
        signed.lease.expires_at_unix_ms += HOUR_MS;
        assert_eq!(
            signed.verify(source_of(&writer()), WORLD, ISSUED),
            Err(AdmissionLeaseError::BadSignature)
        );
    }

    // Oracle: an admission window is [issued, expires) — a lease
    // presented before its window (clock skew or a pre-dated forgery
    // attempt) admits nobody yet.
    #[test]
    fn test_lease_before_its_window_is_not_yet_valid() {
        let signed = lease(HOUR_MS);
        assert_eq!(
            signed.verify(source_of(&writer()), WORLD, ISSUED - 1),
            Err(AdmissionLeaseError::NotYetValid)
        );
    }

    // Oracle: policy B bounds revocation staleness (module doc) — a
    // signed window above the maximum is refused even with a valid
    // signature from the pinned writer, and `issue` never mints one
    // (nor a zero-length lease).
    #[test]
    fn test_overlong_and_zero_durations_are_refused() {
        assert!(SignedAdmissionLease::issue(WORLD, &writer(), ISSUED, 0).is_none());
        assert!(
            SignedAdmissionLease::issue(
                WORLD,
                &writer(),
                ISSUED,
                MAX_ADMISSION_LEASE_DURATION_MS + 1
            )
            .is_none()
        );
        let overlong = sign(
            AdmissionLease {
                version: ADMISSION_LEASE_VERSION,
                world_id: WORLD.to_owned(),
                source: source_of(&writer()),
                issued_at_unix_ms: ISSUED,
                expires_at_unix_ms: ISSUED + MAX_ADMISSION_LEASE_DURATION_MS + 1,
            },
            &writer(),
        );
        assert_eq!(
            overlong.verify(source_of(&writer()), WORLD, ISSUED),
            Err(AdmissionLeaseError::ExcessiveDuration)
        );
    }

    // Oracle: version discipline mirrors the replication envelope —
    // an unknown lease version is refused, never best-effort parsed.
    #[test]
    fn test_unknown_lease_version_is_refused() {
        let future = sign(
            AdmissionLease {
                version: ADMISSION_LEASE_VERSION + 1,
                world_id: WORLD.to_owned(),
                source: source_of(&writer()),
                issued_at_unix_ms: ISSUED,
                expires_at_unix_ms: ISSUED + HOUR_MS,
            },
            &writer(),
        );
        assert_eq!(
            future.verify(source_of(&writer()), WORLD, ISSUED),
            Err(AdmissionLeaseError::UnsupportedVersion)
        );
    }
}
