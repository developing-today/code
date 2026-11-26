//! The blob object-plane lifecycle (code#13318, design §9, D011, D012).
//!
//! [`MemoryPlane`] is the reference model of the object plane a fleet kv
//! endpoint fronts. It holds bytes in memory, so it is a model and a test
//! double, not storage; the endpoint slice puts the same rules in front of
//! component F's adapters. The rules it pins down:
//!
//! - **Staging.** `stage` streams bytes, recomputes length and BLAKE3-256 on
//!   receipt and admits them only when they match what the pusher asserted.
//!   Admitted bytes are `verified` under a staging pin. Nothing staged or
//!   verified can be read.
//! - **Publication.** `publish` is the service's verification mark. Only the
//!   central publish-CAS (the CAS plus reference registration on the shared
//!   primary) makes a reference row readable; the plane is told about live
//!   references through [`Roots`]. A published object whose CAS never
//!   lands is an orphan: staging TTL never reaps it, only GC after grace.
//! - **Reads.** A hash grants nothing. `mint` issues a five-minute,
//!   object- and operation-scoped capability, and only for a reference that
//!   is live in the roots. `read` checks the capability's MAC and expiry and
//!   re-verifies the bytes before serving them.
//! - **GC.** Per `(jurisdiction, tier)`: an unreferenced published object
//!   becomes a candidate, waits out the grace period, is re-verified
//!   against the roots and is deleted under a new fence epoch with an audit
//!   event. A reference that appears during grace clears candidacy. A
//!   publish-CAS that lands after the delete finds no live object and the
//!   writer stages the bytes again.

use std::collections::BTreeMap;
use std::io::{Read, Write};

use crate::blob::{self, BlobError, Digest, KvBlobRefV1, Partition};

/// How long a capability lives (design §9 default).
pub const CAPABILITY_TTL_MS: i64 = 5 * 60 * 1000;
/// How long verified bytes wait for publication before staging reaps them.
pub const STAGING_TTL_MS: i64 = 60 * 60 * 1000;
/// How long an unreferenced published object stays a GC candidate.
pub const GRACE_MS: i64 = 24 * 60 * 60 * 1000;

/// The central reference state GC and capability minting consult: every
/// root that names content (published rows, retained versions, conflict
/// proposals, unexpired archive and snapshot references, dedup pins).
pub trait Roots {
    fn is_referenced(&self, partition: &Partition, digest: &str) -> bool;
}

/// What a capability lets its holder do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Read,
}

impl Operation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
        }
    }
}

/// A short-lived, object- and operation-scoped capability. The MAC binds
/// every field, so it cannot be moved to another object, partition,
/// operation or expiry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capability {
    pub digest: String,
    pub partition: Partition,
    pub operation: Operation,
    pub expires_at_ms: i64,
    pub mac: [u8; 32],
}

/// The receipt for verified, staged bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub partition: Partition,
    pub digest: String,
    pub bytes_len: u64,
}

/// One fenced delete, as the audit log records it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deletion {
    pub partition: Partition,
    pub digest: String,
    pub epoch: u64,
    pub candidate_since_ms: i64,
    pub reverified_at_ms: i64,
}

/// What one sweep of a partition did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sweep {
    pub marked: usize,
    pub cleared: usize,
    pub deleted: Vec<Deletion>,
}

/// A refusal from the object plane.
#[derive(Debug)]
pub enum PlaneError {
    Blob(BlobError),
    /// No live verified or published object for this reference: it was
    /// never staged, staging reaped it, or GC deleted it. Stage it again.
    NotStaged,
    /// No live reference names this object, so no capability is minted.
    NoLiveReference,
    CapabilityExpired,
    /// The capability's MAC does not match its fields.
    CapabilityInvalid,
}

impl PlaneError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Blob(error) => error.code(),
            Self::NotStaged => "blob_not_staged",
            Self::NoLiveReference => "no_live_reference",
            Self::CapabilityExpired => "capability_expired",
            Self::CapabilityInvalid => "capability_invalid",
        }
    }
}

impl From<BlobError> for PlaneError {
    fn from(error: BlobError) -> Self {
        Self::Blob(error)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum State {
    /// Verified bytes under the staging pin.
    Verified { pinned_until_ms: i64 },
    /// Marked published; retired only by GC.
    Published,
    /// Unreferenced at `since_ms`; deleted after grace unless a reference
    /// appears.
    Candidate { since_ms: i64 },
}

#[derive(Clone, Debug)]
struct Object {
    bytes: Vec<u8>,
    state: State,
}

/// The in-memory object plane model. Objects are keyed by partition and
/// digest, so dedup never crosses a partition.
pub struct MemoryPlane {
    key: [u8; 32],
    objects: BTreeMap<(Partition, String), Object>,
    epoch: u64,
    audit: Vec<Deletion>,
}

impl MemoryPlane {
    /// A plane whose capabilities are MACed with `key`.
    pub fn new(key: [u8; 32]) -> Self {
        Self {
            key,
            objects: BTreeMap::new(),
            epoch: 0,
            audit: Vec::new(),
        }
    }

    /// Stream `reader` into the staging area of `partition`. The bytes are
    /// admitted only when their length and BLAKE3-256 match `asserted`;
    /// otherwise nothing is kept. Bytes already published in the partition
    /// are deduplicated: the existing object stays as it is.
    pub fn stage(
        &mut self,
        partition: &Partition,
        asserted: &Digest,
        reader: &mut dyn Read,
        now_ms: i64,
    ) -> Result<Receipt, PlaneError> {
        let wanted = KvBlobRefV1 {
            digest: asserted.digest.clone(),
            bytes_len: asserted.bytes_len,
            partition: partition.clone(),
            locator: String::from("staging"),
        };
        let mut bytes = Vec::new();
        blob::verify_copy(&wanted, reader, &mut bytes)?;
        let key = (partition.clone(), asserted.digest.clone());
        let pinned_until_ms = now_ms.saturating_add(STAGING_TTL_MS);
        match self.objects.get_mut(&key) {
            Some(Object {
                state: State::Verified {
                    pinned_until_ms: pin,
                },
                ..
            }) => *pin = (*pin).max(pinned_until_ms),
            Some(_) => {}
            None => {
                self.objects.insert(
                    key,
                    Object {
                        bytes,
                        state: State::Verified { pinned_until_ms },
                    },
                );
            }
        }
        Ok(Receipt {
            partition: partition.clone(),
            digest: asserted.digest.clone(),
            bytes_len: asserted.bytes_len,
        })
    }

    /// Mark verified bytes published and return the reference the
    /// publish-CAS will register. From here only GC retires the object.
    pub fn publish(&mut self, receipt: &Receipt, now_ms: i64) -> Result<KvBlobRefV1, PlaneError> {
        let key = (receipt.partition.clone(), receipt.digest.clone());
        let object = self.objects.get_mut(&key).ok_or(PlaneError::NotStaged)?;
        if let State::Verified { pinned_until_ms } = object.state {
            if now_ms >= pinned_until_ms {
                return Err(PlaneError::NotStaged);
            }
            object.state = State::Published;
        }
        Ok(KvBlobRefV1 {
            digest: receipt.digest.clone(),
            bytes_len: receipt.bytes_len,
            partition: receipt.partition.clone(),
            locator: locator(&receipt.partition, &receipt.digest),
        })
    }

    /// Whether a reference's object is published and not deleted: the
    /// check a publish-CAS makes before it registers the reference.
    pub fn is_live(&self, reference: &KvBlobRefV1) -> bool {
        matches!(
            self.objects
                .get(&(reference.partition.clone(), reference.digest.clone()))
                .map(|object| &object.state),
            Some(State::Published | State::Candidate { .. })
        )
    }

    /// Drop verified bytes whose staging pin expired. Published objects are
    /// never touched here.
    pub fn reap_staging(&mut self, now_ms: i64) -> usize {
        let before = self.objects.len();
        self.objects.retain(|_, object| match object.state {
            State::Verified { pinned_until_ms } => now_ms < pinned_until_ms,
            _ => true,
        });
        before - self.objects.len()
    }

    /// Mint a read capability for `reference`, only when the roots hold a
    /// live reference to it and its object is live.
    pub fn mint(
        &self,
        reference: &KvBlobRefV1,
        operation: Operation,
        roots: &dyn Roots,
        now_ms: i64,
    ) -> Result<Capability, PlaneError> {
        if !roots.is_referenced(&reference.partition, &reference.digest) {
            return Err(PlaneError::NoLiveReference);
        }
        if !self.is_live(reference) {
            return Err(PlaneError::NotStaged);
        }
        let mut capability = Capability {
            digest: reference.digest.clone(),
            partition: reference.partition.clone(),
            operation,
            expires_at_ms: now_ms.saturating_add(CAPABILITY_TTL_MS),
            mac: [0; 32],
        };
        capability.mac = self.mac(&capability);
        Ok(capability)
    }

    /// Serve the bytes a capability names into `out`, re-verifying them
    /// first. A capability is good until, not at, its expiry.
    pub fn read(
        &self,
        capability: &Capability,
        now_ms: i64,
        out: &mut dyn Write,
    ) -> Result<u64, PlaneError> {
        let expected = blake3::Hash::from_bytes(self.mac(capability));
        if expected != blake3::Hash::from_bytes(capability.mac) {
            return Err(PlaneError::CapabilityInvalid);
        }
        if now_ms >= capability.expires_at_ms {
            return Err(PlaneError::CapabilityExpired);
        }
        let object = self
            .objects
            .get(&(capability.partition.clone(), capability.digest.clone()))
            .filter(|object| !matches!(object.state, State::Verified { .. }))
            .ok_or(PlaneError::NotStaged)?;
        let wanted = KvBlobRefV1 {
            digest: capability.digest.clone(),
            bytes_len: object.bytes.len() as u64,
            partition: capability.partition.clone(),
            locator: locator(&capability.partition, &capability.digest),
        };
        // Verify into a buffer first so corrupt bytes are never served.
        let mut verified = Vec::with_capacity(object.bytes.len());
        blob::verify_copy(&wanted, &mut &object.bytes[..], &mut verified)?;
        out.write_all(&verified)
            .map_err(|error| PlaneError::Blob(BlobError::Io(error)))?;
        Ok(verified.len() as u64)
    }

    /// One GC pass over `partition` against `roots`, at `now_ms`.
    pub fn sweep(&mut self, partition: &Partition, roots: &dyn Roots, now_ms: i64) -> Sweep {
        let mut sweep = Sweep::default();
        let mut doomed = Vec::new();
        for ((object_partition, digest), object) in &mut self.objects {
            if object_partition != partition {
                continue;
            }
            let referenced = roots.is_referenced(partition, digest);
            match object.state {
                State::Verified { .. } => {}
                State::Published if !referenced => {
                    object.state = State::Candidate { since_ms: now_ms };
                    sweep.marked += 1;
                }
                State::Published => {}
                State::Candidate { .. } if referenced => {
                    object.state = State::Published;
                    sweep.cleared += 1;
                }
                State::Candidate { since_ms } => {
                    if now_ms.saturating_sub(since_ms) >= GRACE_MS {
                        doomed.push((digest.clone(), since_ms));
                    }
                }
            }
        }
        for (digest, since_ms) in doomed {
            // Re-verify at grace expiry, then delete under a new fence.
            if roots.is_referenced(partition, &digest) {
                if let Some(object) = self.objects.get_mut(&(partition.clone(), digest)) {
                    object.state = State::Published;
                    sweep.cleared += 1;
                }
                continue;
            }
            self.epoch += 1;
            self.objects.remove(&(partition.clone(), digest.clone()));
            let deletion = Deletion {
                partition: partition.clone(),
                digest,
                epoch: self.epoch,
                candidate_since_ms: since_ms,
                reverified_at_ms: now_ms,
            };
            self.audit.push(deletion.clone());
            sweep.deleted.push(deletion);
        }
        sweep
    }

    /// Every fenced delete so far, in order.
    pub fn audit(&self) -> &[Deletion] {
        &self.audit
    }

    fn mac(&self, capability: &Capability) -> [u8; 32] {
        let binding = format!(
            "fleet-kv-capability-v1\n{}\n{}\n{}\n{}\n{}",
            capability.operation.as_str(),
            capability.partition.jurisdiction,
            capability.partition.tier,
            capability.digest,
            capability.expires_at_ms
        );
        *blake3::keyed_hash(&self.key, binding.as_bytes()).as_bytes()
    }

    #[cfg(test)]
    fn corrupt(&mut self, partition: &Partition, digest: &str) {
        if let Some(object) = self
            .objects
            .get_mut(&(partition.clone(), digest.to_string()))
        {
            object.bytes[0] ^= 0xff;
        }
    }
}

fn locator(partition: &Partition, digest: &str) -> String {
    format!(
        "memory:{}/{}/{digest}",
        partition.jurisdiction, partition.tier
    )
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::BTreeSet;

    /// The roots as a set of `(partition, digest)` pairs.
    #[derive(Default)]
    struct Refs(BTreeSet<(Partition, String)>);

    impl Roots for Refs {
        fn is_referenced(&self, partition: &Partition, digest: &str) -> bool {
            self.0.contains(&(partition.clone(), digest.to_string()))
        }
    }

    impl Refs {
        /// The publish-CAS: registers the reference only while its object
        /// is live, as the shared primary orders it against the fence.
        fn publish_cas(&mut self, plane: &MemoryPlane, reference: &KvBlobRefV1) -> bool {
            if plane.is_live(reference) {
                self.0
                    .insert((reference.partition.clone(), reference.digest.clone()));
                true
            } else {
                false
            }
        }

        fn drop_ref(&mut self, reference: &KvBlobRefV1) {
            self.0
                .remove(&(reference.partition.clone(), reference.digest.clone()));
        }
    }

    fn eu() -> Partition {
        Partition::new("eu", "standard")
    }

    fn staged(plane: &mut MemoryPlane, partition: &Partition, bytes: &[u8], now: i64) -> Receipt {
        let asserted = blob::digest(&mut &bytes[..]).expect("digest");
        plane
            .stage(partition, &asserted, &mut &bytes[..], now)
            .expect("stage")
    }

    fn published(
        plane: &mut MemoryPlane,
        partition: &Partition,
        bytes: &[u8],
        now: i64,
    ) -> KvBlobRefV1 {
        let receipt = staged(plane, partition, bytes, now);
        plane.publish(&receipt, now).expect("publish")
    }

    fn read_all(
        plane: &MemoryPlane,
        capability: &Capability,
        now: i64,
    ) -> Result<Vec<u8>, PlaneError> {
        let mut out = Vec::new();
        plane.read(capability, now, &mut out)?;
        Ok(out)
    }

    #[test]
    fn blob_staging_refuses_bytes_that_do_not_match_the_assertion() {
        let mut plane = MemoryPlane::new([7; 32]);
        let asserted = blob::digest(&mut &b"the real bytes"[..]).expect("digest");
        let refused = plane
            .stage(&eu(), &asserted, &mut &b"other bytes!!!"[..], 0)
            .expect_err("mismatch");
        assert_eq!(refused.code(), "blob_digest_mismatch");
        let refused = plane
            .stage(&eu(), &asserted, &mut &b"the real"[..], 0)
            .expect_err("short");
        assert_eq!(refused.code(), "blob_length_mismatch");
        assert_eq!(plane.objects.len(), 0, "nothing is kept");
    }

    #[test]
    fn blob_staged_and_verified_objects_are_invisible_and_reaped() {
        let mut plane = MemoryPlane::new([7; 32]);
        let receipt = staged(&mut plane, &eu(), b"value", 0);
        let reference = KvBlobRefV1 {
            digest: receipt.digest.clone(),
            bytes_len: receipt.bytes_len,
            partition: eu(),
            locator: locator(&eu(), &receipt.digest),
        };
        // Not published: no CAS can register it and nothing can read it.
        assert!(!plane.is_live(&reference));
        let mut refs = Refs::default();
        refs.0.insert((eu(), receipt.digest.clone()));
        assert_eq!(
            plane
                .mint(&reference, Operation::Read, &refs, 0)
                .expect_err("staged")
                .code(),
            "blob_not_staged"
        );
        // A sweep leaves staging alone; the staging TTL reaps it.
        assert_eq!(
            plane.sweep(&eu(), &Refs::default(), GRACE_MS * 2),
            Sweep::default()
        );
        assert_eq!(plane.reap_staging(STAGING_TTL_MS - 1), 0);
        assert_eq!(plane.reap_staging(STAGING_TTL_MS), 1);
        assert_eq!(
            plane
                .publish(&receipt, STAGING_TTL_MS)
                .expect_err("reaped")
                .code(),
            "blob_not_staged"
        );
    }

    #[test]
    fn blob_published_orphans_wait_for_grace_and_are_never_staging_reaped() {
        let mut plane = MemoryPlane::new([7; 32]);
        let receipt = staged(&mut plane, &eu(), b"orphan", 0);
        let reference = plane.publish(&receipt, 1).expect("publish");
        // The CAS never lands. Staging TTL does not reap a published object.
        assert_eq!(plane.reap_staging(STAGING_TTL_MS * 10), 0);
        assert!(plane.is_live(&reference));
        let roots = Refs::default();
        assert_eq!(plane.sweep(&eu(), &roots, 100).marked, 1);
        assert!(
            plane
                .sweep(&eu(), &roots, 100 + GRACE_MS - 1)
                .deleted
                .is_empty()
        );
        let deleted = plane.sweep(&eu(), &roots, 100 + GRACE_MS).deleted;
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].epoch, 1);
        assert_eq!(deleted[0].candidate_since_ms, 100);
        assert_eq!(plane.audit(), &deleted[..]);
        assert!(!plane.is_live(&reference));
    }

    #[test]
    fn blob_a_reference_during_grace_clears_candidacy() {
        let mut plane = MemoryPlane::new([7; 32]);
        let receipt = staged(&mut plane, &eu(), b"late reference", 0);
        let reference = plane.publish(&receipt, 0).expect("publish");
        let mut refs = Refs::default();
        assert_eq!(plane.sweep(&eu(), &refs, 10).marked, 1);
        // The publish-CAS lands during grace: still live, so it registers.
        assert!(refs.publish_cas(&plane, &reference));
        let sweep = plane.sweep(&eu(), &refs, 10 + GRACE_MS);
        assert_eq!((sweep.cleared, sweep.deleted.len()), (1, 0));
        assert!(plane.is_live(&reference));
    }

    #[test]
    fn blob_a_cas_after_the_fence_finds_nothing_and_the_writer_restages() {
        let mut plane = MemoryPlane::new([7; 32]);
        let receipt = staged(&mut plane, &eu(), b"raced", 0);
        let reference = plane.publish(&receipt, 0).expect("publish");
        let mut refs = Refs::default();
        plane.sweep(&eu(), &refs, 0);
        assert_eq!(plane.sweep(&eu(), &refs, GRACE_MS).deleted.len(), 1);
        assert!(
            !refs.publish_cas(&plane, &reference),
            "no live object to name"
        );
        // The writer still holds the bytes: re-staging is a re-upload.
        let again = staged(&mut plane, &eu(), b"raced", GRACE_MS + 1);
        let reference = plane.publish(&again, GRACE_MS + 1).expect("publish");
        assert!(refs.publish_cas(&plane, &reference));
    }

    #[test]
    fn blob_retained_history_pins_survive_sweeps() {
        let mut plane = MemoryPlane::new([7; 32]);
        let mut refs = Refs::default();
        let old = published(&mut plane, &eu(), b"version one", 0);
        assert!(refs.publish_cas(&plane, &old));
        let new = published(&mut plane, &eu(), b"version two", 1);
        assert!(refs.publish_cas(&plane, &new));
        // The head moved to version two; version one stays a root because
        // its version row is retained.
        for step in 0..4 {
            let sweep = plane.sweep(&eu(), &refs, step * GRACE_MS);
            assert_eq!(sweep, Sweep::default());
        }
        assert!(plane.is_live(&old) && plane.is_live(&new));
        // Once the version row is gone, the old object is collected.
        refs.drop_ref(&old);
        plane.sweep(&eu(), &refs, 10 * GRACE_MS);
        let deleted = plane.sweep(&eu(), &refs, 11 * GRACE_MS).deleted;
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].digest, old.digest);
        assert!(plane.is_live(&new));
    }

    #[test]
    fn blob_capabilities_need_a_live_reference_and_expire_at_five_minutes() {
        let mut plane = MemoryPlane::new([7; 32]);
        let reference = published(&mut plane, &eu(), b"secret bytes", 0);
        // Knowing the hash is not enough: no reference, no capability.
        let refused = plane
            .mint(&reference, Operation::Read, &Refs::default(), 0)
            .expect_err("unreferenced");
        assert_eq!(refused.code(), "no_live_reference");
        let mut refs = Refs::default();
        assert!(refs.publish_cas(&plane, &reference));
        let capability = plane
            .mint(&reference, Operation::Read, &refs, 1_000)
            .expect("mint");
        assert_eq!(capability.expires_at_ms, 1_000 + CAPABILITY_TTL_MS);
        assert_eq!(
            read_all(&plane, &capability, 1_000).expect("read"),
            b"secret bytes"
        );
        assert_eq!(
            read_all(&plane, &capability, 1_000 + CAPABILITY_TTL_MS - 1).expect("read"),
            b"secret bytes"
        );
        assert_eq!(
            read_all(&plane, &capability, 1_000 + CAPABILITY_TTL_MS)
                .expect_err("expired")
                .code(),
            "capability_expired"
        );
    }

    #[test]
    fn blob_capabilities_cannot_be_retargeted_or_forged() {
        let mut plane = MemoryPlane::new([7; 32]);
        let mut refs = Refs::default();
        let mine = published(&mut plane, &eu(), b"mine", 0);
        let theirs = published(&mut plane, &eu(), b"theirs", 0);
        assert!(refs.publish_cas(&plane, &mine) && refs.publish_cas(&plane, &theirs));
        let capability = plane.mint(&mine, Operation::Read, &refs, 0).expect("mint");

        let mut retargeted = capability.clone();
        retargeted.digest = theirs.digest.clone();
        let mut extended = capability.clone();
        extended.expires_at_ms += CAPABILITY_TTL_MS;
        let mut moved = capability.clone();
        moved.partition = Partition::new("us", "standard");
        let forged = plane_with_other_key_mint(&mine);
        for bad in [retargeted, extended, moved, forged] {
            assert_eq!(
                read_all(&plane, &bad, 0)
                    .expect_err("bad capability")
                    .code(),
                "capability_invalid"
            );
        }
    }

    fn plane_with_other_key_mint(reference: &KvBlobRefV1) -> Capability {
        let mut other = MemoryPlane::new([9; 32]);
        let reference = published(&mut other, &reference.partition, b"mine", 0);
        let mut refs = Refs::default();
        assert!(refs.publish_cas(&other, &reference));
        other
            .mint(&reference, Operation::Read, &refs, 0)
            .expect("mint")
    }

    #[test]
    fn blob_corrupt_replicas_are_never_served() {
        let mut plane = MemoryPlane::new([7; 32]);
        let mut refs = Refs::default();
        let reference = published(&mut plane, &eu(), b"fragile", 0);
        assert!(refs.publish_cas(&plane, &reference));
        let capability = plane
            .mint(&reference, Operation::Read, &refs, 0)
            .expect("mint");
        plane.corrupt(&eu(), &reference.digest);
        let mut out = Vec::new();
        let refused = plane.read(&capability, 0, &mut out).expect_err("corrupt");
        assert_eq!(refused.code(), "blob_digest_mismatch");
        assert!(out.is_empty(), "no corrupt byte reaches the reader");
    }

    #[test]
    fn blob_dedup_and_gc_stay_inside_their_partition() {
        let mut plane = MemoryPlane::new([7; 32]);
        let us = Partition::new("us", "standard");
        let mut refs = Refs::default();
        let in_eu = published(&mut plane, &eu(), b"same bytes", 0);
        let in_us = published(&mut plane, &us, b"same bytes", 0);
        assert_eq!(in_eu.digest, in_us.digest);
        assert_ne!(
            in_eu.locator, in_us.locator,
            "two objects, no cross-partition dedup"
        );
        assert!(refs.publish_cas(&plane, &in_eu));
        // The EU reference does not keep the US object alive, and a US sweep
        // never touches EU.
        plane.sweep(&us, &refs, 0);
        assert_eq!(plane.sweep(&us, &refs, GRACE_MS).deleted.len(), 1);
        assert!(plane.is_live(&in_eu) && !plane.is_live(&in_us));
        // A reference resolves only in its own partition.
        assert_eq!(
            in_eu.resolve_in(&us).expect_err("cross partition").code(),
            "partition_mismatch"
        );
    }

    #[derive(Clone, Debug)]
    enum Step {
        Stage(u8),
        Publish(u8),
        Cas(u8),
        DropRef(u8),
        Sweep,
        Reap,
        Advance(i64),
    }

    fn step() -> impl Strategy<Value = Step> {
        prop_oneof![
            (0u8..4).prop_map(Step::Stage),
            (0u8..4).prop_map(Step::Publish),
            (0u8..4).prop_map(Step::Cas),
            (0u8..4).prop_map(Step::DropRef),
            Just(Step::Sweep),
            Just(Step::Reap),
            prop_oneof![
                Just(1i64),
                Just(GRACE_MS / 2),
                Just(GRACE_MS),
                Just(STAGING_TTL_MS),
            ]
            .prop_map(Step::Advance),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// Over any interleaving of staging, publication, publish-CAS,
        /// reference drops, sweeps, reaps and time:
        /// - a reference is only ever registered for a published, live
        ///   object (publish before reference);
        /// - every registered reference stays live and readable (GC never
        ///   deletes a root);
        /// - every delete happened with no live reference at re-verify,
        ///   after grace, under a strictly increasing fence epoch.
        #[test]
        fn blob_gc_safety_over_reference_interleavings(
            steps in proptest::collection::vec(step(), 1..60),
        ) {
            let mut plane = MemoryPlane::new([7; 32]);
            let mut refs = Refs::default();
            let mut receipts: BTreeMap<u8, Receipt> = BTreeMap::new();
            let mut published: BTreeMap<u8, KvBlobRefV1> = BTreeMap::new();
            let mut now = 0i64;
            let mut last_epoch = 0u64;
            for step in steps {
                match step {
                    Step::Stage(item) => {
                        let bytes = vec![item; 3 + usize::from(item)];
                        receipts.insert(item, staged(&mut plane, &eu(), &bytes, now));
                    }
                    Step::Publish(item) => {
                        if let Some(receipt) = receipts.get(&item)
                            && let Ok(reference) = plane.publish(receipt, now)
                        {
                            published.insert(item, reference);
                        }
                    }
                    Step::Cas(item) => {
                        if let Some(reference) = published.get(&item) {
                            let live = plane.is_live(reference);
                            prop_assert_eq!(refs.publish_cas(&plane, reference), live);
                        }
                    }
                    Step::DropRef(item) => {
                        if let Some(reference) = published.get(&item) {
                            refs.drop_ref(reference);
                        }
                    }
                    Step::Sweep => {
                        let sweep = plane.sweep(&eu(), &refs, now);
                        for deletion in &sweep.deleted {
                            prop_assert!(!refs.is_referenced(&deletion.partition, &deletion.digest));
                            prop_assert!(deletion.reverified_at_ms - deletion.candidate_since_ms >= GRACE_MS);
                            prop_assert!(deletion.epoch > last_epoch);
                            last_epoch = deletion.epoch;
                        }
                    }
                    Step::Reap => {
                        plane.reap_staging(now);
                    }
                    Step::Advance(by) => now += by,
                }
                for (partition, digest) in &refs.0 {
                    let reference = published
                        .values()
                        .find(|reference| &reference.digest == digest && &reference.partition == partition)
                        .expect("only published references are registered");
                    prop_assert!(plane.is_live(reference), "a root was collected");
                    let capability = plane.mint(reference, Operation::Read, &refs, now).expect("mint");
                    let mut out = Vec::new();
                    prop_assert_eq!(plane.read(&capability, now, &mut out).expect("read"), reference.bytes_len);
                }
            }
            prop_assert_eq!(plane.audit().len() as u64, last_epoch);
        }
    }
}
