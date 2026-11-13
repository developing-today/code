//! Signed keyset/authorization log for an identity.
//!
//! The record envelope follows [`crate::directory_log`]: canonical
//! `serde_json` signed bodies, SHA-256 content hashes, strict Ed25519
//! verification, and deterministic merge keys. The extra `prev_hash` makes
//! authorization position-dependent: revoked keys cannot authorize later
//! records by replaying a different branch.

use std::collections::BTreeMap;

use anyhow::{Context, Result, ensure};
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{directory::hex_to_array, world::hex_encode};

/// Deterministic order for concurrent records extending one chain head.
pub type MergeKey = (u64, String, String);

/// One authorization-log event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KeysetEntry {
    Genesis {
        admin_key: String,
        device_key: String,
        label: String,
    },
    DeviceEnrolled {
        device_key: String,
        label: String,
    },
    DeviceRevoked {
        device_key: String,
        reason: String,
    },
    AdminKeyRotated {
        new_admin_key: String,
    },
    IdpBindingBound {
        binding_id: String,
    },
    IdpBindingRevoked {
        binding_id: String,
    },
    GuardianSetUpdated {
        guardians: Vec<String>,
        threshold: u32,
    },
    Checkpoint {
        state_hash: String,
        record_count: u64,
    },
}

/// A signed keyset change. `countersignature` proves possession of a newly
/// enrolled device key or replacement admin key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeysetRecord {
    pub author: String,
    pub lamport: u64,
    pub prev_hash: String,
    pub hash: String,
    pub entry: KeysetEntry,
    pub signature: String,
    pub countersignature: Option<String>,
}

#[derive(Serialize)]
struct SignedBody<'a> {
    author: &'a str,
    lamport: u64,
    prev_hash: &'a str,
    entry: &'a KeysetEntry,
}

fn body_bytes(author: &str, lamport: u64, prev_hash: &str, entry: &KeysetEntry) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&SignedBody {
        author,
        lamport,
        prev_hash,
        entry,
    })?)
}

impl KeysetRecord {
    /// Sign an entry and, when required, countersign it with the enrolled or
    /// replacement key. Signers refuse records that `verify` would reject.
    pub fn sign(
        key: &SigningKey,
        lamport: u64,
        prev_hash: impl Into<String>,
        entry: KeysetEntry,
        countersigner: Option<&SigningKey>,
    ) -> Result<Self> {
        validate_entry(&entry)?;
        let author = hex_encode(&key.verifying_key().to_bytes());
        let prev_hash = prev_hash.into();
        validate_hash_or_empty(
            &prev_hash,
            matches!(&entry, KeysetEntry::Genesis { .. }),
            "prev_hash",
        )?;
        let expected_counter = countersignature_key(&entry);
        match (expected_counter, countersigner) {
            (Some(expected), Some(counter)) => ensure!(
                expected == hex_encode(&counter.verifying_key().to_bytes()),
                "countersigner does not match the key named by the entry"
            ),
            (Some(_), None) => anyhow::bail!("this entry requires a countersigner"),
            (None, Some(_)) => anyhow::bail!("this entry does not accept a countersigner"),
            (None, None) => {}
        }
        if let KeysetEntry::Genesis { admin_key, .. } = &entry {
            ensure!(
                author == *admin_key,
                "genesis must be signed by its admin key"
            );
        }
        let body = body_bytes(&author, lamport, &prev_hash, &entry)?;
        let hash = hex_encode(&Sha256::digest(&body));
        let signature = hex_encode(&key.sign(&body).to_bytes());
        let countersignature =
            countersigner.map(|counter| hex_encode(&counter.sign(&body).to_bytes()));
        Ok(Self {
            author,
            lamport,
            prev_hash,
            hash,
            entry,
            signature,
            countersignature,
        })
    }

    /// Merge-order position.
    #[must_use]
    pub fn key(&self) -> MergeKey {
        (self.lamport, self.author.clone(), self.hash.clone())
    }

    /// Verify the canonical body, hash, author signature, and any required
    /// proof-of-possession countersignature.
    pub fn verify(&self) -> Result<()> {
        validate_entry(&self.entry)?;
        validate_hash_or_empty(
            &self.prev_hash,
            matches!(&self.entry, KeysetEntry::Genesis { .. }),
            "prev_hash",
        )?;
        let author = verifying_key(&self.author).context("malformed author key")?;
        if let KeysetEntry::Genesis { admin_key, .. } = &self.entry {
            ensure!(
                self.author == *admin_key,
                "genesis must be signed by its admin key"
            );
        }
        let body = body_bytes(&self.author, self.lamport, &self.prev_hash, &self.entry)?;
        ensure!(
            hex_encode(&Sha256::digest(&body)) == self.hash,
            "keyset record hash does not match its content"
        );
        verify_signature(&author, &body, &self.signature).context("author signature")?;
        match (
            countersignature_key(&self.entry),
            self.countersignature.as_deref(),
        ) {
            (Some(expected), Some(signature)) => {
                let key = verifying_key(expected).context("malformed countersigner key")?;
                verify_signature(&key, &body, signature).context("countersignature")?;
            }
            (Some(_), None) => anyhow::bail!("required countersignature is missing"),
            (None, Some(_)) => anyhow::bail!("unexpected countersignature"),
            (None, None) => {}
        }
        Ok(())
    }
}

fn verify_signature(key: &VerifyingKey, body: &[u8], encoded: &str) -> Result<()> {
    let bytes = hex_to_array::<64>(encoded).context("malformed signature")?;
    ensure!(
        hex_encode(&bytes) == encoded,
        "signature is not canonically encoded"
    );
    key.verify_strict(body, &Signature::from_bytes(&bytes))
        .context("signature does not verify")
}

fn verifying_key(encoded: &str) -> Result<VerifyingKey> {
    let bytes = hex_to_array::<32>(encoded).context("malformed Ed25519 public key")?;
    ensure!(
        hex_encode(&bytes) == encoded,
        "public key is not canonically encoded"
    );
    VerifyingKey::from_bytes(&bytes).context("invalid Ed25519 public key")
}

fn validate_hash_or_empty(value: &str, allow_empty: bool, field: &str) -> Result<()> {
    if allow_empty && value.is_empty() {
        return Ok(());
    }
    let bytes = hex_to_array::<32>(value).with_context(|| format!("malformed {field}"))?;
    ensure!(
        hex_encode(&bytes) == value,
        "{field} is not canonically encoded"
    );
    Ok(())
}

fn countersignature_key(entry: &KeysetEntry) -> Option<&str> {
    match entry {
        KeysetEntry::DeviceEnrolled { device_key, .. } => Some(device_key),
        KeysetEntry::AdminKeyRotated { new_admin_key } => Some(new_admin_key),
        _ => None,
    }
}

fn validate_entry(entry: &KeysetEntry) -> Result<()> {
    match entry {
        KeysetEntry::Genesis {
            admin_key,
            device_key,
            ..
        } => {
            verifying_key(admin_key).context("genesis admin key")?;
            verifying_key(device_key).context("genesis device key")?;
        }
        KeysetEntry::DeviceEnrolled { device_key, .. }
        | KeysetEntry::DeviceRevoked { device_key, .. } => {
            verifying_key(device_key).context("device key")?;
        }
        KeysetEntry::AdminKeyRotated { new_admin_key } => {
            verifying_key(new_admin_key).context("replacement admin key")?;
        }
        KeysetEntry::IdpBindingBound { binding_id }
        | KeysetEntry::IdpBindingRevoked { binding_id } => {
            ensure!(
                !binding_id.trim().is_empty(),
                "binding_id must not be empty"
            );
        }
        KeysetEntry::GuardianSetUpdated {
            guardians,
            threshold,
        } => {
            ensure!(*threshold > 0, "guardian threshold must be positive");
            ensure!(
                usize::try_from(*threshold).unwrap_or(usize::MAX) <= guardians.len(),
                "guardian threshold exceeds guardian count"
            );
            let mut unique = guardians.clone();
            unique.sort();
            unique.dedup();
            ensure!(
                unique.len() == guardians.len(),
                "guardian identities must be unique"
            );
            for guardian in guardians {
                verifying_key(guardian).context("guardian identity")?;
            }
        }
        KeysetEntry::Checkpoint { state_hash, .. } => {
            validate_hash_or_empty(state_hash, false, "state_hash")?;
        }
    }
    Ok(())
}

/// A set of cryptographically valid records, deduplicated by merge key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Log {
    records: BTreeMap<MergeKey, KeysetRecord>,
}

impl Log {
    /// Insert a verified record; invalid records leave the log unchanged.
    pub fn insert(&mut self, record: KeysetRecord) -> Result<bool> {
        record.verify()?;
        let key = record.key();
        if self.records.contains_key(&key) {
            return Ok(false);
        }
        self.records.insert(key, record);
        Ok(true)
    }

    /// Records in deterministic merge-key order.
    pub fn records(&self) -> impl Iterator<Item = &KeysetRecord> {
        self.records.values()
    }
}

/// Set union of verified records; commutative, associative, and idempotent.
#[must_use]
pub fn merge(local: &Log, remote: &Log) -> Log {
    let mut merged = local.clone();
    for (key, record) in &remote.records {
        merged
            .records
            .entry(key.clone())
            .or_insert_with(|| record.clone());
    }
    merged
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdminKeyState {
    Active,
    Rotated,
    Revoked,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceState {
    pub label: String,
    pub enrolled_at: u64,
    pub revoked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingState {
    Bound,
    Revoked,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardianSet {
    pub guardians: Vec<String>,
    pub threshold: u32,
}

/// Deterministic projection of a keyset chain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeysetState {
    pub identity_id: String,
    pub admin_keys: BTreeMap<String, AdminKeyState>,
    pub devices: BTreeMap<String, DeviceState>,
    pub idp_bindings: BTreeMap<String, BindingState>,
    pub guardian_set: Option<GuardianSet>,
    pub head_hash: String,
    pub records_folded: u64,
}

#[derive(Serialize)]
struct StateHashBody<'a> {
    identity_id: &'a str,
    admin_keys: &'a BTreeMap<String, AdminKeyState>,
    devices: &'a BTreeMap<String, DeviceState>,
    idp_bindings: &'a BTreeMap<String, BindingState>,
    guardian_set: &'a Option<GuardianSet>,
}

impl KeysetState {
    /// Is `key` an active, unrevoked device key?
    #[must_use]
    pub fn is_device(&self, key: &str) -> bool {
        self.devices.get(key).is_some_and(|device| !device.revoked)
    }

    /// Is `key` currently authorized to administer the identity?
    #[must_use]
    pub fn is_admin(&self, key: &str) -> bool {
        self.admin_keys.get(key) == Some(&AdminKeyState::Active)
    }

    /// SHA-256 of the canonical folded state (cursor metadata is excluded).
    pub fn state_hash(&self) -> Result<String> {
        let body = StateHashBody {
            identity_id: &self.identity_id,
            admin_keys: &self.admin_keys,
            devices: &self.devices,
            idp_bindings: &self.idp_bindings,
            guardian_set: &self.guardian_set,
        };
        Ok(hex_encode(&Sha256::digest(serde_json::to_vec(&body)?)))
    }
}

/// Fold a log from a caller-trusted genesis hash. Any competing genesis is
/// rejected, so a replica cannot reset authorization by injecting a new root
/// after an admin rotation. The trusted hash must be persisted outside the
/// mergeable log (for example, alongside the identity's initial keyset).
/// Invalid/unauthorized continuations and their descendants are excluded; the
/// smallest valid `(lamport, author, hash)` continuation wins each fork.
/// `Checkpoint` records are snapshot attestations (ADR Decision 4 rule 6):
/// they never authorise anything and never advance the chain head, so a
/// device checkpoint can neither displace an admin record at a fork nor
/// wedge the head with an unbounded lamport value.
pub fn fold(log: &Log, trusted_genesis_hash: &str) -> Result<KeysetState> {
    validate_hash_or_empty(trusted_genesis_hash, false, "trusted_genesis_hash")?;
    let roots = log
        .records
        .values()
        .filter(|record| {
            record.prev_hash.is_empty() && matches!(&record.entry, KeysetEntry::Genesis { .. })
        })
        .filter(|record| {
            matches!(&record.entry, KeysetEntry::Genesis { admin_key, .. } if admin_key == &record.author)
        })
        .collect::<Vec<_>>();
    ensure!(
        roots
            .iter()
            .any(|record| record.hash == trusted_genesis_hash),
        "trusted keyset genesis is missing from log"
    );
    ensure!(
        roots
            .iter()
            .all(|record| record.hash == trusted_genesis_hash),
        "keyset log contains a competing genesis"
    );
    let genesis = roots[0];

    let KeysetEntry::Genesis {
        admin_key,
        device_key,
        label,
    } = &genesis.entry
    else {
        anyhow::bail!("selected keyset root is not a genesis record");
    };
    let (admin_key, device_key, label) = (admin_key.clone(), device_key.clone(), label.clone());
    let mut state = KeysetState {
        identity_id: admin_key.clone(),
        admin_keys: BTreeMap::from([(admin_key, AdminKeyState::Active)]),
        devices: BTreeMap::from([(
            device_key,
            DeviceState {
                label,
                enrolled_at: genesis.lamport,
                revoked: false,
            },
        )]),
        idp_bindings: BTreeMap::new(),
        guardian_set: None,
        head_hash: genesis.hash.clone(),
        records_folded: 1,
    };
    let mut head_lamport = genesis.lamport;

    loop {
        let next = log
            .records
            .values()
            .filter(|record| {
                record.prev_hash == state.head_hash
                    && record.lamport > head_lamport
                    && !matches!(
                        &record.entry,
                        KeysetEntry::Genesis { .. } | KeysetEntry::Checkpoint { .. }
                    )
            })
            .filter(|record| state.is_admin(&record.author))
            .filter_map(|record| {
                let mut candidate = state.clone();
                apply_entry(&mut candidate, record)
                    .ok()
                    .map(|()| (record, candidate))
            })
            .min_by_key(|(record, _)| record.key());
        let Some((record, mut next_state)) = next else {
            break;
        };
        next_state.head_hash = record.hash.clone();
        next_state.records_folded = next_state
            .records_folded
            .checked_add(1)
            .context("keyset record count overflow")?;
        head_lamport = record.lamport;
        state = next_state;
    }
    Ok(state)
}

fn apply_entry(state: &mut KeysetState, record: &KeysetRecord) -> Result<()> {
    let authorized_admin = state.is_admin(&record.author);
    match &record.entry {
        KeysetEntry::Genesis { .. } => anyhow::bail!("genesis is only valid at the start of a log"),
        KeysetEntry::DeviceEnrolled { device_key, label } => {
            ensure!(
                authorized_admin,
                "device enrollment requires an active admin"
            );
            state.devices.insert(
                device_key.clone(),
                DeviceState {
                    label: label.clone(),
                    enrolled_at: record.lamport,
                    revoked: false,
                },
            );
        }
        KeysetEntry::DeviceRevoked { device_key, .. } => {
            ensure!(
                authorized_admin,
                "device revocation requires an active admin"
            );
            let device = state
                .devices
                .get_mut(device_key)
                .context("unknown device key")?;
            ensure!(!device.revoked, "device is already revoked");
            device.revoked = true;
        }
        KeysetEntry::AdminKeyRotated { new_admin_key } => {
            ensure!(authorized_admin, "admin rotation requires an active admin");
            ensure!(
                new_admin_key != &record.author,
                "admin key cannot rotate to itself"
            );
            state
                .admin_keys
                .insert(record.author.clone(), AdminKeyState::Rotated);
            state
                .admin_keys
                .insert(new_admin_key.clone(), AdminKeyState::Active);
        }
        KeysetEntry::IdpBindingBound { binding_id } => {
            ensure!(authorized_admin, "IdP binding requires an active admin");
            state
                .idp_bindings
                .insert(binding_id.clone(), BindingState::Bound);
        }
        KeysetEntry::IdpBindingRevoked { binding_id } => {
            ensure!(
                authorized_admin,
                "IdP binding revocation requires an active admin"
            );
            ensure!(
                state.idp_bindings.get(binding_id) == Some(&BindingState::Bound),
                "IdP binding is not active"
            );
            state
                .idp_bindings
                .insert(binding_id.clone(), BindingState::Revoked);
        }
        KeysetEntry::GuardianSetUpdated {
            guardians,
            threshold,
        } => {
            ensure!(authorized_admin, "guardian update requires an active admin");
            state.guardian_set = Some(GuardianSet {
                guardians: guardians.clone(),
                threshold: *threshold,
            });
        }
        KeysetEntry::Checkpoint {
            state_hash,
            record_count,
        } => {
            ensure!(
                state.is_device(&record.author),
                "checkpoint requires an active device"
            );
            ensure!(
                *record_count == state.records_folded,
                "checkpoint record count does not match folded prefix"
            );
            ensure!(
                *state_hash == state.state_hash()?,
                "checkpoint state hash does not match folded prefix"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn public(seed: u8) -> String {
        hex_encode(&key(seed).verifying_key().to_bytes())
    }

    fn genesis() -> KeysetRecord {
        let admin = key(1);
        KeysetRecord::sign(
            &admin,
            1,
            "",
            KeysetEntry::Genesis {
                admin_key: public(1),
                device_key: public(1),
                label: "first device".to_owned(),
            },
            None,
        )
        .unwrap()
    }

    fn append(
        prior: &KeysetRecord,
        author: &SigningKey,
        lamport: u64,
        entry: KeysetEntry,
        countersigner: Option<&SigningKey>,
    ) -> KeysetRecord {
        KeysetRecord::sign(author, lamport, prior.hash.clone(), entry, countersigner).unwrap()
    }

    #[test]
    fn signed_body_uses_directory_log_canonical_json_field_order() {
        let record = genesis();
        let body = body_bytes(
            &record.author,
            record.lamport,
            &record.prev_hash,
            &record.entry,
        )
        .unwrap();
        let text = String::from_utf8(body).unwrap();
        assert!(text.starts_with("{\"author\":"));
        assert!(text.contains("\",\"lamport\":1,\"prev_hash\":\"\",\"entry\":{"));
        assert_eq!(hex_encode(&Sha256::digest(text.as_bytes())), record.hash);
        record.verify().unwrap();
    }

    #[test]
    fn enrollment_requires_device_proof_of_possession() {
        let genesis = genesis();
        let admin = key(1);
        let device = key(2);
        let entry = KeysetEntry::DeviceEnrolled {
            device_key: public(2),
            label: "laptop".to_owned(),
        };
        assert!(KeysetRecord::sign(&admin, 2, genesis.hash.clone(), entry.clone(), None).is_err());
        let record = append(&genesis, &admin, 2, entry, Some(&device));
        record.verify().unwrap();
    }

    #[test]
    fn revocation_is_prospective_and_removes_device_authority() {
        let first = genesis();
        let admin = key(1);
        let device = key(2);
        let enrolled = append(
            &first,
            &admin,
            2,
            KeysetEntry::DeviceEnrolled {
                device_key: public(2),
                label: "tablet".to_owned(),
            },
            Some(&device),
        );
        let revoked = append(
            &enrolled,
            &admin,
            3,
            KeysetEntry::DeviceRevoked {
                device_key: public(2),
                reason: "lost".to_owned(),
            },
            None,
        );
        let mut log = Log::default();
        for record in [&first, &enrolled, &revoked] {
            log.insert(record.clone()).unwrap();
        }
        let state = fold(&log, &first.hash).unwrap();
        assert!(state.is_admin(&public(1)));
        assert!(!state.is_device(&public(2)));
        assert_eq!(state.devices[&public(2)].enrolled_at, 2);
    }

    #[test]
    fn fork_chooses_smallest_merge_key_and_merge_is_order_independent() {
        let first = genesis();
        let admin = key(1);
        let left = append(
            &first,
            &admin,
            2,
            KeysetEntry::DeviceEnrolled {
                device_key: public(2),
                label: "left".to_owned(),
            },
            Some(&key(2)),
        );
        let right = append(
            &first,
            &admin,
            2,
            KeysetEntry::DeviceEnrolled {
                device_key: public(3),
                label: "right".to_owned(),
            },
            Some(&key(3)),
        );
        let mut a = Log::default();
        let mut b = Log::default();
        for record in [&first, &left, &right] {
            a.insert(record.clone()).unwrap();
        }
        for record in [&right, &left, &first] {
            b.insert(record.clone()).unwrap();
        }
        assert_eq!(
            fold(&a, &first.hash).unwrap(),
            fold(&b, &first.hash).unwrap()
        );
        let winner = if left.key() < right.key() {
            &left
        } else {
            &right
        };
        assert!(
            fold(&a, &first.hash)
                .unwrap()
                .devices
                .contains_key(match &winner.entry {
                    KeysetEntry::DeviceEnrolled { device_key, .. } => device_key,
                    _ => unreachable!(),
                })
        );
    }

    #[test]
    fn competing_genesis_cannot_reset_a_rotated_admin() {
        let trusted = genesis();
        let former_admin = key(1);
        let replacement_admin = key(4);
        let rotation = append(
            &trusted,
            &former_admin,
            2,
            KeysetEntry::AdminKeyRotated {
                new_admin_key: public(4),
            },
            Some(&replacement_admin),
        );
        let forged_root = KeysetRecord::sign(
            &former_admin,
            0,
            "",
            KeysetEntry::Genesis {
                admin_key: public(1),
                device_key: public(1),
                label: "replacement root".to_owned(),
            },
            None,
        )
        .unwrap();
        let mut log = Log::default();
        for record in [&trusted, &rotation, &forged_root] {
            log.insert(record.clone()).unwrap();
        }

        assert!(fold(&log, &trusted.hash).is_err());

        let mut trusted_only = Log::default();
        trusted_only.insert(trusted.clone()).unwrap();
        trusted_only.insert(rotation).unwrap();
        let state = fold(&trusted_only, &trusted.hash).unwrap();
        assert!(!state.is_admin(&public(1)));
        assert!(state.is_admin(&public(4)));
    }

    #[test]
    fn noncanonical_signature_encoding_is_rejected() {
        // hex_to_array accepts both hex cases, but keys and hashes are held
        // to the canonical lowercase encoding; signatures must be too, or
        // two encodings of one envelope share a merge key yet compare
        // unequal and merge stops being order-independent.
        let mut record = genesis();
        record.signature = record.signature.to_uppercase();
        assert!(record.verify().is_err());
        let mut log = Log::default();
        assert!(log.insert(record).is_err());

        let first = genesis();
        let enrolled = append(
            &first,
            &key(1),
            2,
            KeysetEntry::DeviceEnrolled {
                device_key: public(2),
                label: "tablet".to_owned(),
            },
            Some(&key(2)),
        );
        let mut forged = enrolled.clone();
        forged.countersignature = forged
            .countersignature
            .map(|signature| signature.to_uppercase());
        assert_ne!(forged, enrolled);
        assert!(forged.verify().is_err());
    }

    #[test]
    fn device_checkpoint_cannot_displace_admin_revocation() {
        // Genesis enrols key(2) as the device; the admin revokes it at
        // lamport 3 while the device posts a checkpoint of the
        // genesis-only prefix at lamport 2. A checkpoint never authorises
        // anything (ADR Decision 4 rule 6), so it must not win the fork
        // and resurrect the revoked device.
        let admin = key(1);
        let device = key(2);
        let first = KeysetRecord::sign(
            &admin,
            1,
            "",
            KeysetEntry::Genesis {
                admin_key: public(1),
                device_key: public(2),
                label: "phone".to_owned(),
            },
            None,
        )
        .unwrap();
        let mut prefix_log = Log::default();
        prefix_log.insert(first.clone()).unwrap();
        let prefix = fold(&prefix_log, &first.hash).unwrap();
        let checkpoint = append(
            &first,
            &device,
            2,
            KeysetEntry::Checkpoint {
                state_hash: prefix.state_hash().unwrap(),
                record_count: prefix.records_folded,
            },
            None,
        );
        let revoked = append(
            &first,
            &admin,
            3,
            KeysetEntry::DeviceRevoked {
                device_key: public(2),
                reason: "lost".to_owned(),
            },
            None,
        );
        let mut log = Log::default();
        for record in [&first, &checkpoint, &revoked] {
            log.insert(record.clone()).unwrap();
        }
        let state = fold(&log, &first.hash).unwrap();
        assert!(!state.is_device(&public(2)));
        assert_eq!(state.head_hash, revoked.hash);
    }

    #[test]
    fn checkpoint_cannot_wedge_the_fold_head() {
        // A checkpoint carrying an unbounded lamport must not become the
        // head: no later record could extend it, wedging the log.
        let first = genesis();
        let mut prefix_log = Log::default();
        prefix_log.insert(first.clone()).unwrap();
        let prefix = fold(&prefix_log, &first.hash).unwrap();
        let checkpoint = append(
            &first,
            &key(1),
            u64::MAX,
            KeysetEntry::Checkpoint {
                state_hash: prefix.state_hash().unwrap(),
                record_count: prefix.records_folded,
            },
            None,
        );
        let enrolled = append(
            &first,
            &key(1),
            2,
            KeysetEntry::DeviceEnrolled {
                device_key: public(3),
                label: "desktop".to_owned(),
            },
            Some(&key(3)),
        );
        let mut log = Log::default();
        for record in [&first, &checkpoint, &enrolled] {
            log.insert(record.clone()).unwrap();
        }
        let state = fold(&log, &first.hash).unwrap();
        assert!(state.is_device(&public(3)));
        assert_eq!(state.head_hash, enrolled.hash);
    }

    proptest! {
        #[test]
        fn deterministic_fork_resolution_is_invariant_under_insertion_order(reverse in any::<bool>()) {
            let first = genesis();
            let admin = key(1);
            let left = append(&first, &admin, 2, KeysetEntry::IdpBindingBound { binding_id: "left".to_owned() }, None);
            let right = append(&first, &admin, 2, KeysetEntry::IdpBindingBound { binding_id: "right".to_owned() }, None);
            let mut records = vec![first.clone(), left.clone(), right.clone()];
            if reverse { records.reverse(); }
            let mut log = Log::default();
            for record in records { log.insert(record).unwrap(); }
            let state = fold(&log, &first.hash).unwrap();
            let winner = if left.key() < right.key() { "left" } else { "right" };
            prop_assert!(state.idp_bindings.contains_key(winner));
            prop_assert_eq!(state.idp_bindings.len(), 1);
        }
    }
}
