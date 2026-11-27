//! Registry lists: signed, mirrorable discovery lists.
//!
//! From the discovery/registry/federation design (doc
//! 2026-11-16_design_id6287_discovery_registry_federation_v1.md
//! section 5; implementation breakdown item 3/5, #10707).
//!
//! A registry list is a file signed by the registry's own key. It
//! carries the registry key, its inclusion policy statement, entries
//! that each hold a world's [`SignedAnnouncement`] verbatim, and
//! tombstones that record a delisting with its reason, so a reader can
//! tell "delisted" from "never seen".
//!
//! Inclusion is an opinion and provenance is the announcement's own
//! signature, and the format keeps the two apart:
//!
//! - The registry signature covers the list: which worlds this
//!   registry includes, when, and in what order. It says nothing about
//!   whether a world is real.
//! - Each entry's announcement keeps its own world-key signature,
//!   copied byte for byte. [`SignedRegistryList::verify`] checks every
//!   one against its own `world_id`, so a registry that edits an
//!   announcement, even while re-signing the list, produces a list that
//!   fails to verify.
//! - A [`Registry`] publisher takes signed announcements and never a
//!   world key: listing is not custody (section 4), and no registry
//!   operation can repoint a listed world.
//! - The list carries no access or grant fields. A registry inclusion
//!   is discovery only and never overrides a portal grant (#1068).
//!
//! Mirroring ([`Registry::import`]) copies another registry's
//! announcements, still self-signed, and applies the mirror's own
//! policy and its own tombstones; the mirror then signs only the
//! subset it chose. Two registries disagreeing about a world is the
//! expected state (section 5).
//!
//! Unlisted worlds are never in a registry (section 6): they are
//! refused at inclusion and skipped on import.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::announcement::{SignedAnnouncement, Visibility, byte_array};

/// Maximum length of a registry's inclusion policy statement, in chars.
pub const MAX_POLICY_CHARS: usize = 2_000;
/// Maximum length of a tombstone's reason, in chars.
pub const MAX_REASON_CHARS: usize = 200;

/// One included world: its announcement verbatim, and this registry's
/// record of when and at which registry sequence it was included.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryEntry {
    /// The world's own signed announcement, unmodified.
    pub announcement: SignedAnnouncement,
    /// When this registry included this announcement, in Unix seconds.
    pub included_at: u64,
    /// The registry sequence number of this inclusion.
    pub registry_seq: u64,
}

/// A delisting: the world, when, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tombstone {
    /// The delisted world's key.
    #[serde(with = "byte_array")]
    pub world_id: [u8; 32],
    /// When the registry delisted it, in Unix seconds.
    pub removed_at: u64,
    /// The registry's stated reason, at most [`MAX_REASON_CHARS`] chars.
    pub reason: String,
    /// The registry sequence number of this delisting.
    pub registry_seq: u64,
}

/// A registry list, unsigned. Entries and tombstones are sorted by
/// world key, so the same registry state always encodes the same bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryList {
    /// The registry's public key: the key the list signature verifies against.
    #[serde(with = "byte_array")]
    pub registry_key: [u8; 32],
    /// The registry's inclusion policy statement, in its own words.
    pub policy: String,
    /// The registry's sequence at publication: no entry or tombstone
    /// carries a higher one, and a later list from the same registry
    /// carries a higher one.
    pub seq: u64,
    /// When the list was published, in Unix seconds.
    pub published_at: u64,
    /// Included worlds, sorted by world key.
    pub entries: Vec<RegistryEntry>,
    /// Delisted worlds, sorted by world key.
    pub tombstones: Vec<Tombstone>,
}

impl RegistryList {
    /// The canonical CBOR encoding of the list: the bytes the registry
    /// signature covers.
    ///
    /// # Errors
    ///
    /// Fails if the list does not encode.
    pub fn encode_body(&self) -> Result<Vec<u8>> {
        Ok(cbor_core::Value::serialized(self)
            .context("registry list does not encode as CBOR")?
            .encode())
    }

    /// The included entry for `world_id`, if any.
    #[must_use]
    pub fn entry(&self, world_id: &[u8; 32]) -> Option<&RegistryEntry> {
        self.entries
            .iter()
            .find(|entry| entry.announcement.announcement.world_id == *world_id)
    }

    /// The tombstone for `world_id`, if this registry delisted it.
    #[must_use]
    pub fn tombstone(&self, world_id: &[u8; 32]) -> Option<&Tombstone> {
        self.tombstones
            .iter()
            .find(|tombstone| tombstone.world_id == *world_id)
    }

    /// The entries whose announcements are still live at `now`.
    /// Expired announcements are void (section 3), whatever list holds them.
    pub fn live_entries(&self, now: u64) -> impl Iterator<Item = &RegistryEntry> {
        self.entries
            .iter()
            .filter(move |entry| now < entry.announcement.announcement.expires_at)
    }

    /// The list's own consistency rules, apart from any signature.
    fn check_shape(&self) -> Result<()> {
        if self.policy.chars().count() > MAX_POLICY_CHARS {
            bail!("registry policy exceeds {MAX_POLICY_CHARS} chars");
        }
        let mut worlds = BTreeSet::new();
        let mut seqs = BTreeSet::new();
        let mut previous: Option<[u8; 32]> = None;
        for entry in &self.entries {
            let world = entry.announcement.announcement.world_id;
            if previous.is_some_and(|p| p >= world) {
                bail!("registry entries are not in strict world-key order");
            }
            previous = Some(world);
            if entry.announcement.announcement.visibility != Visibility::Listed {
                bail!("registry lists an unlisted announcement");
            }
            worlds.insert(world);
            if !seqs.insert(entry.registry_seq) || entry.registry_seq > self.seq {
                bail!("registry entry sequence is repeated or above the list sequence");
            }
        }
        let mut previous: Option<[u8; 32]> = None;
        for tombstone in &self.tombstones {
            if previous.is_some_and(|p| p >= tombstone.world_id) {
                bail!("registry tombstones are not in strict world-key order");
            }
            previous = Some(tombstone.world_id);
            if !worlds.insert(tombstone.world_id) {
                bail!("a world is both listed and delisted");
            }
            if tombstone.reason.chars().count() > MAX_REASON_CHARS {
                bail!("tombstone reason exceeds {MAX_REASON_CHARS} chars");
            }
            if !seqs.insert(tombstone.registry_seq) || tombstone.registry_seq > self.seq {
                bail!("registry tombstone sequence is repeated or above the list sequence");
            }
        }
        Ok(())
    }
}

/// A [`RegistryList`] plus the registry key's ed25519 signature over
/// its canonical encoding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedRegistryList {
    /// The list.
    pub list: RegistryList,
    /// The registry key's signature over [`RegistryList::encode_body`].
    #[serde(with = "byte_array")]
    pub signature: [u8; 64],
}

impl SignedRegistryList {
    /// Checks the list against the registry key it names: the list
    /// signature, the list's consistency rules, and every entry's own
    /// announcement signature against its world key. It does not
    /// check expiry; see [`RegistryList::live_entries`].
    ///
    /// Whether to follow this registry at all is the reader's setting:
    /// use [`SignedRegistryList::verify_from`] to also require a key.
    ///
    /// # Errors
    ///
    /// Fails on the first rule or signature that does not hold.
    pub fn verify(&self) -> Result<()> {
        let key = VerifyingKey::from_bytes(&self.list.registry_key)
            .context("registry key is not a key")?;
        let body = self.list.encode_body()?;
        key.verify_strict(&body, &Signature::from_bytes(&self.signature))
            .context("registry list signature does not verify")?;
        self.list.check_shape()?;
        for entry in &self.list.entries {
            entry
                .announcement
                .verify()
                .context("a listed announcement does not verify against its own world key")?;
        }
        Ok(())
    }

    /// [`SignedRegistryList::verify`], and the list must be signed by
    /// `registry_key`.
    ///
    /// # Errors
    ///
    /// Fails when the list names another registry key, or on any
    /// [`SignedRegistryList::verify`] failure.
    pub fn verify_from(&self, registry_key: &[u8; 32]) -> Result<()> {
        if self.list.registry_key != *registry_key {
            bail!("registry list is signed by a different registry key");
        }
        self.verify()
    }

    /// The list file: CBOR of the whole signed list.
    ///
    /// # Errors
    ///
    /// Fails if the list does not encode.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        Ok(cbor_core::Value::serialized(self)
            .context("signed registry list does not encode as CBOR")?
            .encode())
    }

    /// Decodes a list file. Decoding does not verify; call
    /// [`SignedRegistryList::verify`] on the result.
    ///
    /// # Errors
    ///
    /// Fails if `bytes` does not decode.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let value = cbor_core::Value::decode(bytes).context("registry list is not CBOR")?;
        value
            .deserialized()
            .context("registry list does not decode")
    }
}

/// What a [`Registry::import`] did with each world in the source list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// Worlds now included in this registry from the source.
    pub included: Vec<[u8; 32]>,
    /// Worlds this registry already had at the same or a newer seq.
    pub kept_own: Vec<[u8; 32]>,
    /// Worlds this registry's policy declined.
    pub declined: Vec<[u8; 32]>,
    /// Worlds this registry has delisted; its own tombstone stands.
    pub delisted_here: Vec<[u8; 32]>,
    /// Worlds whose announcements had expired at import time.
    pub expired: Vec<[u8; 32]>,
    /// Worlds the source has delisted. Reported, not applied: the
    /// source's delisting is its opinion, not this registry's.
    pub source_tombstones: Vec<[u8; 32]>,
}

/// A registry publisher: holds the registry key and the registry's own
/// inclusion state, and publishes signed lists. It never holds a world key.
#[derive(Debug)]
pub struct Registry {
    key: SigningKey,
    policy: String,
    seq: u64,
    entries: BTreeMap<[u8; 32], RegistryEntry>,
    tombstones: BTreeMap<[u8; 32], Tombstone>,
}

impl Registry {
    /// A new, empty registry with its key and inclusion policy statement.
    ///
    /// # Errors
    ///
    /// Fails when the policy exceeds [`MAX_POLICY_CHARS`].
    pub fn new(key: SigningKey, policy: impl Into<String>) -> Result<Self> {
        let policy = policy.into();
        if policy.chars().count() > MAX_POLICY_CHARS {
            bail!("registry policy exceeds {MAX_POLICY_CHARS} chars");
        }
        Ok(Self {
            key,
            policy,
            seq: 0,
            entries: BTreeMap::new(),
            tombstones: BTreeMap::new(),
        })
    }

    /// Resumes a registry from its last published list, which must be
    /// this key's and must verify.
    ///
    /// # Errors
    ///
    /// Fails when the list is another registry's or does not verify.
    pub fn resume(key: SigningKey, published: &SignedRegistryList) -> Result<Self> {
        published.verify_from(&key.verifying_key().to_bytes())?;
        let list = &published.list;
        Ok(Self {
            key,
            policy: list.policy.clone(),
            seq: list.seq,
            entries: list
                .entries
                .iter()
                .map(|entry| (entry.announcement.announcement.world_id, entry.clone()))
                .collect(),
            tombstones: list
                .tombstones
                .iter()
                .map(|tombstone| (tombstone.world_id, tombstone.clone()))
                .collect(),
        })
    }

    /// This registry's public key.
    #[must_use]
    pub fn registry_key(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }

    /// Includes a world's signed announcement, verbatim. Including a
    /// delisted world is this registry's decision to relist it, and
    /// removes the tombstone. Re-including the same announcement is a
    /// no-op that returns its existing registry sequence.
    ///
    /// # Errors
    ///
    /// Fails when the announcement does not verify or has expired at
    /// `now`, is not `listed`, or does not advance the seq of the
    /// announcement already included for that world.
    pub fn include(&mut self, announcement: SignedAnnouncement, now: u64) -> Result<u64> {
        announcement.verify_at(now)?;
        if announcement.announcement.visibility != Visibility::Listed {
            bail!("an unlisted announcement is never submitted to a registry");
        }
        let world = announcement.announcement.world_id;
        if let Some(existing) = self.entries.get(&world) {
            if existing.announcement == announcement {
                return Ok(existing.registry_seq);
            }
            announcement
                .announcement
                .accepts_after(Some(&existing.announcement.announcement))?;
        }
        self.seq += 1;
        self.tombstones.remove(&world);
        self.entries.insert(
            world,
            RegistryEntry {
                announcement,
                included_at: now,
                registry_seq: self.seq,
            },
        );
        Ok(self.seq)
    }

    /// Delists a world with a reason, leaving a tombstone. A world this
    /// registry never listed may be delisted too: the tombstone then
    /// records that this registry declines it.
    ///
    /// # Errors
    ///
    /// Fails when the reason exceeds [`MAX_REASON_CHARS`].
    pub fn delist(&mut self, world_id: [u8; 32], reason: &str, now: u64) -> Result<u64> {
        if reason.chars().count() > MAX_REASON_CHARS {
            bail!("tombstone reason exceeds {MAX_REASON_CHARS} chars");
        }
        self.seq += 1;
        self.entries.remove(&world_id);
        self.tombstones.insert(
            world_id,
            Tombstone {
                world_id,
                removed_at: now,
                reason: reason.to_owned(),
                registry_seq: self.seq,
            },
        );
        Ok(self.seq)
    }

    /// Imports another registry's list as a mirror. The source list must
    /// verify (and, when `source_key` is given, be that registry's).
    /// Each listed world is then included here only if its announcement
    /// is live at `now`, this registry has not delisted it, this
    /// registry has no announcement for it at the same or a newer seq,
    /// and `accept` says yes. The source's tombstones are reported,
    /// not applied.
    ///
    /// # Errors
    ///
    /// Fails, and imports nothing, when the source list does not verify.
    pub fn import(
        &mut self,
        source: &SignedRegistryList,
        source_key: Option<&[u8; 32]>,
        now: u64,
        mut accept: impl FnMut(&SignedAnnouncement) -> bool,
    ) -> Result<ImportReport> {
        match source_key {
            Some(key) => source.verify_from(key)?,
            None => source.verify()?,
        }
        let mut report = ImportReport::default();
        for entry in &source.list.entries {
            let announcement = &entry.announcement;
            let world = announcement.announcement.world_id;
            if now >= announcement.announcement.expires_at {
                report.expired.push(world);
            } else if self.tombstones.contains_key(&world) {
                report.delisted_here.push(world);
            } else if self.entries.get(&world).is_some_and(|own| {
                own.announcement.announcement.seq >= announcement.announcement.seq
            }) {
                report.kept_own.push(world);
            } else if !accept(announcement) {
                report.declined.push(world);
            } else {
                self.include(announcement.clone(), now)?;
                report.included.push(world);
            }
        }
        report.source_tombstones = source
            .list
            .tombstones
            .iter()
            .map(|tombstone| tombstone.world_id)
            .collect();
        Ok(report)
    }

    /// Publishes the current state as a signed list. Entries whose
    /// announcements have expired at `now` are dropped first: an expired
    /// announcement is void, and dropping it is not a delisting.
    ///
    /// # Errors
    ///
    /// Fails if the list does not encode.
    pub fn publish(&mut self, now: u64) -> Result<SignedRegistryList> {
        self.entries
            .retain(|_, entry| now < entry.announcement.announcement.expires_at);
        let list = RegistryList {
            registry_key: self.registry_key(),
            policy: self.policy.clone(),
            seq: self.seq,
            published_at: now,
            entries: self.entries.values().cloned().collect(),
            tombstones: self.tombstones.values().cloned().collect(),
        };
        let signature = self.key.sign(&list.encode_body()?).to_bytes();
        Ok(SignedRegistryList { list, signature })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::announcement::{Announcement, Occupancy, Subject};

    const NOW: u64 = 1_763_500_000;
    const DAY: u64 = 24 * 60 * 60;

    fn world(seed: u8, name: &str, tags: &[&str], seq: u64) -> SignedAnnouncement {
        world_with(seed, name, tags, seq, Visibility::Listed)
    }

    fn world_with(
        seed: u8,
        name: &str,
        tags: &[&str],
        seq: u64,
        visibility: Visibility,
    ) -> SignedAnnouncement {
        let key = SigningKey::from_bytes(&[seed; 32]);
        let announcement = Announcement {
            world_id: key.verifying_key().to_bytes(),
            name: name.to_owned(),
            description: String::new(),
            subject: Subject::Endpoint {
                node_id: [seed; 32],
                relay: None,
            },
            occupancy: Occupancy::HasRoom,
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
            visibility,
            issued_at: NOW,
            expires_at: NOW + 3 * DAY,
            seq,
        };
        SignedAnnouncement::sign(&key, announcement).unwrap()
    }

    fn world_expiring(seed: u8, name: &str, seq: u64, expires_at: u64) -> SignedAnnouncement {
        let key = SigningKey::from_bytes(&[seed; 32]);
        let announcement = Announcement {
            world_id: key.verifying_key().to_bytes(),
            name: name.to_owned(),
            description: String::new(),
            subject: Subject::Endpoint {
                node_id: [seed; 32],
                relay: None,
            },
            occupancy: Occupancy::HasRoom,
            tags: Vec::new(),
            visibility: Visibility::Listed,
            issued_at: NOW,
            expires_at,
            seq,
        };
        SignedAnnouncement::sign(&key, announcement).unwrap()
    }

    fn registry(seed: u8, policy: &str) -> Registry {
        Registry::new(SigningKey::from_bytes(&[seed; 32]), policy).unwrap()
    }

    fn id(announcement: &SignedAnnouncement) -> [u8; 32] {
        announcement.announcement.world_id
    }

    #[test]
    fn a_published_list_verifies_round_trips_and_is_deterministic() {
        let mut reg = registry(100, "worlds we have visited");
        reg.include(world(1, "harbor", &["calm"], 1), NOW).unwrap();
        reg.include(world(2, "forge", &["busy"], 1), NOW).unwrap();
        reg.delist(id(&world(3, "spam", &[], 1)), "impersonation", NOW)
            .unwrap();

        let list = reg.publish(NOW).unwrap();
        list.verify().unwrap();
        list.verify_from(&reg.registry_key()).unwrap();
        assert!(list.verify_from(&[0; 32]).is_err());

        let bytes = list.to_bytes().unwrap();
        let decoded = SignedRegistryList::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, list);
        decoded.verify().unwrap();
        assert_eq!(reg.publish(NOW).unwrap().to_bytes().unwrap(), bytes);
        assert_eq!(list.list.seq, 3);
        assert_eq!(list.list.entries.len(), 2);
        assert_eq!(list.list.tombstones[0].reason, "impersonation");
    }

    #[test]
    fn two_registries_list_and_delist_the_same_world_independently() {
        let harbor = world(1, "harbor", &["calm"], 1);
        let mut a = registry(100, "everything we have checked");
        let mut b = registry(101, "calm worlds only, curated");
        a.include(harbor.clone(), NOW).unwrap();
        b.include(harbor.clone(), NOW).unwrap();
        b.delist(id(&harbor), "no longer calm", NOW + 10).unwrap();

        let a_list = a.publish(NOW + 20).unwrap();
        let b_list = b.publish(NOW + 20).unwrap();
        a_list.verify().unwrap();
        b_list.verify().unwrap();
        // A lists it with the world's own signature untouched; B has a
        // tombstone with its reason and no entry.
        assert_eq!(
            a_list.list.entry(&id(&harbor)).unwrap().announcement,
            harbor
        );
        assert!(a_list.list.tombstone(&id(&harbor)).is_none());
        assert!(b_list.list.entry(&id(&harbor)).is_none());
        assert_eq!(
            b_list.list.tombstone(&id(&harbor)).unwrap().reason,
            "no longer calm"
        );

        // B relists by its own decision; the tombstone goes.
        b.include(harbor.clone(), NOW + 30).unwrap();
        let b_list = b.publish(NOW + 30).unwrap();
        b_list.verify().unwrap();
        assert!(b_list.list.tombstone(&id(&harbor)).is_none());
        assert!(b_list.list.entry(&id(&harbor)).is_some());
    }

    #[test]
    fn a_mirror_keeps_announcements_valid_and_its_own_decisions() {
        let calm = world(1, "harbor", &["calm"], 1);
        let busy = world(2, "forge", &["busy"], 1);
        let banned = world(3, "squat", &["calm"], 1);
        let mut source = registry(100, "everything");
        for announcement in [&calm, &busy, &banned] {
            source.include((*announcement).clone(), NOW).unwrap();
        }
        source
            .delist(id(&world(4, "gone", &[], 1)), "dead", NOW)
            .unwrap();
        let source_list = source.publish(NOW).unwrap();

        let mut mirror = registry(101, "calm worlds, mirrored");
        mirror
            .delist(id(&banned), "squatting a known name", NOW)
            .unwrap();
        let report = mirror
            .import(&source_list, Some(&source.registry_key()), NOW + 5, |a| {
                a.announcement.tags.iter().any(|tag| tag == "calm")
            })
            .unwrap();
        assert_eq!(report.included, vec![id(&calm)]);
        assert_eq!(report.declined, vec![id(&busy)]);
        assert_eq!(report.delisted_here, vec![id(&banned)]);
        assert_eq!(
            report.source_tombstones,
            vec![id(&world(4, "gone", &[], 1))]
        );

        let mirrored = mirror.publish(NOW + 5).unwrap();
        mirrored.verify_from(&mirror.registry_key()).unwrap();
        // The announcement is the world's own, byte for byte.
        assert_eq!(
            mirrored
                .list
                .entry(&id(&calm))
                .unwrap()
                .announcement
                .to_bytes()
                .unwrap(),
            calm.to_bytes().unwrap()
        );
        // The mirror's own tombstone stands; the source's is not applied.
        assert!(mirrored.list.tombstone(&id(&banned)).is_some());
        assert!(
            mirrored
                .list
                .tombstone(&id(&world(4, "gone", &[], 1)))
                .is_none()
        );

        // A newer announcement from the world wins on re-import; an older
        // one never replaces what the mirror has.
        let mut source2 = registry(100, "everything");
        source2
            .include(world(1, "harbor", &["calm"], 2), NOW)
            .unwrap();
        let report = mirror
            .import(&source2.publish(NOW).unwrap(), None, NOW + 6, |_| true)
            .unwrap();
        assert_eq!(report.included, vec![id(&calm)]);
        let report = mirror
            .import(&source_list, None, NOW + 7, |_| true)
            .unwrap();
        assert_eq!(report.kept_own, vec![id(&calm)]);
        assert_eq!(
            mirror
                .publish(NOW + 7)
                .unwrap()
                .list
                .entry(&id(&calm))
                .unwrap()
                .announcement
                .announcement
                .seq,
            2
        );
    }

    #[test]
    fn a_third_hop_mirror_verifies_with_only_the_last_hops_counters() {
        // Oracle: design #16051 §7 mirror rule, interop clause — A
        // publishes, B mirrors, C fetches from B; C's copy verifies
        // against B's key, each announcement still verifies against its
        // own world key, and neither A's nor B's `seq` appears in C's
        // counters. Expired-at-import announcements are reported and
        // never cross a hop.
        let harbor = world(1, "harbor", &["calm"], 1);
        let gone = world(3, "squat", &[], 1);
        let mut a = registry(100, "everything");
        a.include(harbor.clone(), NOW).unwrap();
        a.delist(id(&gone), "dead", NOW).unwrap();
        let a_list = a.publish(NOW).unwrap();

        let mut b = registry(101, "calm worlds, mirrored");
        let report = b
            .import(&a_list, Some(&a.registry_key()), NOW + 5, |_| true)
            .unwrap();
        assert_eq!(report.included, vec![id(&harbor)]);
        // B adopts A's delisting as a NEW tombstone under B's own key,
        // its own reason and its own registry_seq — not a forwarding
        // (§3 rule 3).
        b.delist(id(&gone), "dead, also here", NOW + 6).unwrap();
        let b_list = b.publish(NOW + 7).unwrap();
        let b_seq = b_list.list.seq;

        let mut c = registry(102, "downstream reader");
        let report = c
            .import(&b_list, Some(&b.registry_key()), NOW + 10, |_| true)
            .unwrap();
        assert_eq!(report.included, vec![id(&harbor)]);
        // B's tombstone is a notice for C, not an applied veto.
        assert_eq!(report.source_tombstones, vec![id(&gone)]);
        let c_list = c.publish(NOW + 10).unwrap();
        c_list.verify_from(&c.registry_key()).unwrap();
        assert!(c_list.verify_from(&b.registry_key()).is_err());

        // The announcement crosses both hops byte-for-byte: C, who only
        // ever saw B's list, can still check it against the world key.
        let entry = c_list.list.entry(&id(&harbor)).unwrap();
        assert_eq!(
            entry.announcement.to_bytes().unwrap(),
            harbor.to_bytes().unwrap()
        );
        entry.announcement.verify_at(NOW + 10).unwrap();

        // C's counters are C's own: neither A's nor B's list seq leaks
        // into C's published seq or entry registry_seq.
        assert_ne!(c_list.list.seq, a_list.list.seq);
        assert_ne!(c_list.list.seq, b_seq);
        assert_eq!(c_list.list.seq, 1);
        assert_eq!(entry.registry_seq, 1);
        // B's adopted tombstone does not cross into C's list.
        assert!(c_list.list.tombstone(&id(&gone)).is_none());
        assert_eq!(c_list.list.tombstones.len(), 0);

        // A -> B -> A converges: A re-importing B's list re-verifies the
        // still self-signed announcement and keeps its own seq.
        let report = a
            .import(&b_list, Some(&b.registry_key()), NOW + 11, |_| true)
            .unwrap();
        assert!(report.included.is_empty());
        assert_eq!(report.kept_own, vec![id(&harbor)]);
        assert_eq!(report.source_tombstones, vec![id(&gone)]);
        let a_again = a.publish(NOW + 11).unwrap();
        assert_eq!(a_again.list.seq, a_list.list.seq);
        assert_eq!(
            a_again.list.entry(&id(&harbor)).unwrap().announcement,
            harbor
        );

        // An announcement that has expired by the time C fetches is
        // reported and never crosses the hop, even though the mirror
        // still carries it live.
        let expiring = world_expiring(4, "fleeting", 1, NOW + 30);
        let mut short = registry(103, "short-lived");
        short.include(expiring.clone(), NOW).unwrap();
        let short_list = short.publish(NOW).unwrap();
        let mut mirror = registry(104, "mirror");
        mirror.import(&short_list, None, NOW + 1, |_| true).unwrap();
        let mirror_list = mirror.publish(NOW + 1).unwrap();
        assert!(mirror_list.list.entry(&id(&expiring)).is_some());
        let mut late = registry(105, "late reader");
        let report = late.import(&mirror_list, None, NOW + 40, |_| true).unwrap();
        assert_eq!(report.expired, vec![id(&expiring)]);
        assert!(report.included.is_empty());
        assert!(late.publish(NOW + 40).unwrap().list.entries.is_empty());
    }

    #[test]
    fn a_registry_cannot_forge_or_edit_an_announcement() {
        let mut reg = registry(100, "policy");
        reg.include(world(1, "harbor", &[], 1), NOW).unwrap();
        let honest = reg.publish(NOW).unwrap();

        // Editing the list without re-signing breaks the registry signature.
        let mut tampered = honest.clone();
        tampered.list.policy = "a different policy".to_owned();
        assert!(tampered.verify().is_err());

        // Editing an announcement and re-signing the list with the
        // registry key still fails: the announcement's own signature is
        // the world's, and the registry does not hold that key.
        let mut forged = honest.list;
        forged.entries[0].announcement.announcement.name = "harbor-official".to_owned();
        let signature = SigningKey::from_bytes(&[100; 32])
            .sign(&forged.encode_body().unwrap())
            .to_bytes();
        let forged = SignedRegistryList {
            list: forged,
            signature,
        };
        let err = forged.verify().unwrap_err();
        assert!(format!("{err:#}").contains("own world key"), "{err:#}");

        // A mirror refuses to import from a list that does not verify.
        let mut mirror = registry(101, "mirror");
        assert!(mirror.import(&forged, None, NOW, |_| true).is_err());
        assert_eq!(
            mirror.publish(NOW).unwrap().list.entries,
            [] as [RegistryEntry; 0]
        );
    }

    #[test]
    fn inclusion_rules() {
        let mut reg = registry(100, "policy");
        // Unlisted announcements never enter a registry.
        let unlisted = world_with(1, "quiet", &[], 1, Visibility::Unlisted);
        assert!(reg.include(unlisted, NOW).is_err());
        // Expired announcements are void.
        assert!(reg.include(world(2, "old", &[], 1), NOW + 3 * DAY).is_err());
        // The seq must advance; the same announcement is a no-op.
        let first = world(3, "harbor", &[], 5);
        let seq = reg.include(first.clone(), NOW).unwrap();
        assert_eq!(reg.include(first, NOW + 1).unwrap(), seq);
        assert!(reg.include(world(3, "harbor", &[], 4), NOW).is_err());
        // A different announcement at the same seq does not replace it.
        assert!(reg.include(world(3, "harbor-x", &[], 5), NOW).is_err());
        assert!(reg.include(world(3, "harbor-2", &[], 6), NOW).is_ok());
        // Policy and reason caps.
        assert!(
            Registry::new(
                SigningKey::from_bytes(&[1; 32]),
                "x".repeat(MAX_POLICY_CHARS + 1)
            )
            .is_err()
        );
        assert!(
            reg.delist([9; 32], &"y".repeat(MAX_REASON_CHARS + 1), NOW)
                .is_err()
        );
    }

    #[test]
    fn publishing_drops_expired_announcements_without_a_tombstone() {
        let mut reg = registry(100, "policy");
        reg.include(world(1, "harbor", &[], 1), NOW).unwrap();
        let list = reg.publish(NOW + 3 * DAY).unwrap();
        list.verify().unwrap();
        assert_eq!(list.list.entries, [] as [RegistryEntry; 0]);
        assert_eq!(list.list.tombstones, [] as [Tombstone; 0]);
    }

    #[test]
    fn list_shape_rules_are_enforced_on_verify() {
        let key = SigningKey::from_bytes(&[100; 32]);
        let resign = |list: RegistryList| SignedRegistryList {
            signature: key.sign(&list.encode_body().unwrap()).to_bytes(),
            list,
        };
        let mut reg = registry(100, "policy");
        let harbor = world(1, "harbor", &[], 1);
        reg.include(harbor.clone(), NOW).unwrap();
        reg.include(world(2, "forge", &[], 1), NOW).unwrap();
        let good = reg.publish(NOW).unwrap().list;

        // A world both listed and delisted.
        let mut both = good.clone();
        both.tombstones.push(Tombstone {
            world_id: id(&harbor),
            removed_at: NOW,
            reason: String::new(),
            registry_seq: 3,
        });
        both.seq = 3;
        assert!(resign(both).verify().is_err());

        // Entries out of order.
        let mut unordered = good.clone();
        unordered.entries.reverse();
        assert!(resign(unordered).verify().is_err());

        // A sequence above the list's own.
        let mut ahead = good.clone();
        ahead.seq = 1;
        assert!(resign(ahead).verify().is_err());

        // An unlisted announcement smuggled in.
        let mut smuggled = good;
        smuggled.entries[0].announcement = world_with(1, "harbor", &[], 1, Visibility::Unlisted);
        smuggled
            .entries
            .sort_by_key(|entry| entry.announcement.announcement.world_id);
        assert!(resign(smuggled).verify().is_err());
    }

    #[test]
    fn a_registry_resumes_from_its_own_published_list_only() {
        let mut reg = registry(100, "policy");
        reg.include(world(1, "harbor", &[], 1), NOW).unwrap();
        reg.delist([7; 32], "spam", NOW).unwrap();
        let published = reg.publish(NOW).unwrap();

        let mut resumed = Registry::resume(SigningKey::from_bytes(&[100; 32]), &published).unwrap();
        assert_eq!(resumed.publish(NOW).unwrap(), published);
        assert_eq!(resumed.include(world(2, "forge", &[], 1), NOW).unwrap(), 3);
        assert!(Registry::resume(SigningKey::from_bytes(&[101; 32]), &published).is_err());
    }
}
