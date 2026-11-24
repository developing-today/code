//! Signed keyset/authorization log for an identity.
//!
//! The record envelope follows [`crate::directory_log`]: canonical
//! `serde_json` signed bodies, SHA-256 content hashes, strict Ed25519
//! verification, and deterministic merge keys. The extra `prev_hash` makes
//! authorization position-dependent: revoked keys cannot authorize later
//! records by replaying a different branch.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use anyhow::{Context, Result, ensure};
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::directory::hex_to_array;
use id_core::hex::hex_encode;

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
    /// The terminal record of a guardian recovery (guardian recovery
    /// design, Decision 3), signed by the successor admin key. Plain
    /// [`fold`] never accepts it: it folds only under Rule R, which
    /// [`crate::recovery::check_rotation`] evaluates; folding it under
    /// checkpoint finality is that design's split, item 2.
    RecoveryRotation {
        request_id: String,
        approval_hashes: Vec<String>,
        successor_admin_key: String,
        successor_device_key: String,
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

pub(crate) fn body_bytes(author: &str, lamport: u64, prev_hash: &str, entry: &KeysetEntry) -> Result<Vec<u8>> {
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
        validate_author(&author, &entry)?;
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
        validate_author(&self.author, &self.entry)?;
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

/// Entries whose signer is fixed by their own content.
fn validate_author(author: &str, entry: &KeysetEntry) -> Result<()> {
    match entry {
        KeysetEntry::Genesis { admin_key, .. } => ensure!(
            author == admin_key,
            "genesis must be signed by its admin key"
        ),
        KeysetEntry::RecoveryRotation {
            successor_admin_key,
            ..
        } => ensure!(
            author == successor_admin_key,
            "a recovery rotation must be signed by its successor admin key"
        ),
        _ => {}
    }
    Ok(())
}

pub(crate) fn verify_signature(key: &VerifyingKey, body: &[u8], encoded: &str) -> Result<()> {
    let bytes = hex_to_array::<64>(encoded).context("malformed signature")?;
    ensure!(
        hex_encode(&bytes) == encoded,
        "signature is not canonically encoded"
    );
    key.verify_strict(body, &Signature::from_bytes(&bytes))
        .context("signature does not verify")
}

pub(crate) fn verifying_key(encoded: &str) -> Result<VerifyingKey> {
    let bytes = hex_to_array::<32>(encoded).context("malformed Ed25519 public key")?;
    ensure!(
        hex_encode(&bytes) == encoded,
        "public key is not canonically encoded"
    );
    VerifyingKey::from_bytes(&bytes).context("invalid Ed25519 public key")
}

pub(crate) fn validate_hash_or_empty(value: &str, allow_empty: bool, field: &str) -> Result<()> {
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
            // Canonical order (guardian recovery design, Decision 2):
            // strictly ascending, so equal sets hash equally and no
            // identity appears twice.
            ensure!(
                guardians.windows(2).all(|pair| pair[0] < pair[1]),
                "guardian identities must be unique and sorted"
            );
            for guardian in guardians {
                verifying_key(guardian).context("guardian identity")?;
            }
        }
        KeysetEntry::RecoveryRotation {
            request_id,
            approval_hashes,
            successor_admin_key,
            successor_device_key,
        } => {
            validate_hash_or_empty(request_id, false, "request_id")?;
            ensure!(
                !approval_hashes.is_empty(),
                "a recovery rotation names its approvals"
            );
            ensure!(
                approval_hashes.windows(2).all(|pair| pair[0] < pair[1]),
                "approval hashes must be unique and sorted"
            );
            for hash in approval_hashes {
                validate_hash_or_empty(hash, false, "approval hash")?;
            }
            verifying_key(successor_admin_key).context("successor admin key")?;
            verifying_key(successor_device_key).context("successor device key")?;
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

impl GuardianSet {
    /// Does moving from `self` to `next` only add guardians or raise the
    /// threshold? Such a change folds at once; any other is a delayed
    /// change (guardian recovery Decision 6).
    #[must_use]
    pub fn widens_to(&self, next: &Self) -> bool {
        next.threshold >= self.threshold
            && self
                .guardians
                .iter()
                .all(|guardian| next.guardians.contains(guardian))
    }
}

/// A guardian-set change that removes a guardian or lowers the
/// threshold.
///
/// It waits [`crate::recovery::RECOVERY_DELAY_SECONDS`] from
/// the directory's first sighting of `record_hash` before it replaces
/// the effective set (guardian recovery Decision 6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingGuardianSet {
    pub record_hash: String,
    pub set: GuardianSet,
}

/// Deterministic projection of a keyset chain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeysetState {
    pub identity_id: String,
    pub admin_keys: BTreeMap<String, AdminKeyState>,
    pub devices: BTreeMap<String, DeviceState>,
    pub idp_bindings: BTreeMap<String, BindingState>,
    pub guardian_set: Option<GuardianSet>,
    /// A delayed guardian-set change not yet in force.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_guardian_set: Option<PendingGuardianSet>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pending_guardian_set: &'a Option<PendingGuardianSet>,
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

    /// This state with its pending guardian-set change in force, if the
    /// directory first saw that change (`pending_seen`) at least the
    /// recovery delay before `at`. Unknown times never mature a change,
    /// so without timestamps the pre-change set stays in force.
    #[must_use]
    pub fn matured(&self, pending_seen: Option<u64>, at: Option<u64>) -> Self {
        let mut state = self.clone();
        let (Some(seen), Some(at)) = (pending_seen, at) else {
            return state;
        };
        let ripe = seen
            .checked_add(crate::recovery::RECOVERY_DELAY_SECONDS)
            .is_some_and(|due| due <= at);
        if ripe && let Some(pending) = state.pending_guardian_set.take() {
            state.guardian_set = Some(pending.set);
        }
        state
    }

    /// SHA-256 of the canonical folded state (cursor metadata is excluded).
    pub fn state_hash(&self) -> Result<String> {
        let body = StateHashBody {
            identity_id: &self.identity_id,
            admin_keys: &self.admin_keys,
            devices: &self.devices,
            idp_bindings: &self.idp_bindings,
            guardian_set: &self.guardian_set,
            pending_guardian_set: &self.pending_guardian_set,
        };
        Ok(hex_encode(&Sha256::digest(serde_json::to_vec(&body)?)))
    }
}

/// Fold a log from a caller-trusted genesis hash. Only that genesis
/// anchors the fold, so a replica cannot reset authorization by injecting
/// a new root after an admin rotation. The trusted hash must be persisted outside the
/// mergeable log (for example, alongside the identity's initial keyset).
/// Invalid/unauthorized continuations and their descendants are excluded. At
/// a fork, a branch that rotates away or revokes the author of a sibling wins
/// (revocation priority); otherwise the smallest valid `(lamport, author,
/// hash)` continuation wins.
/// `Checkpoint` records are snapshot attestations (ADR Decision 4 rule 6):
/// they never authorise anything and never advance the chain head, so a
/// device checkpoint can neither displace an admin record at a fork nor
/// wedge the head with an unbounded lamport value.
pub fn fold(log: &Log, trusted_genesis_hash: &str) -> Result<KeysetState> {
    fold_with_finality(log, trusted_genesis_hash, &BTreeSet::new())
}

/// [`fold`] under checkpoint finality (ADR Decision 4 rule 5).
///
/// Every record hash in `final_heads` is a head this replica once verified a
/// `Checkpoint` against, so the chain through it is fixed. At a fork,
/// a branch whose continuation passes through a final head wins over
/// any sibling, whatever its merge key and whatever it revokes: a
/// sibling arriving after finality is evidence only. Forks not on the
/// way to a final head follow the ordinary rules. `final_heads` is the
/// replica's own record, kept outside the mergeable log like the
/// trusted genesis; [`checkpoint_final_heads`] says which heads to add.
///
/// # Errors
///
/// Fails if the trusted genesis is missing or malformed.
pub fn fold_with_finality(
    log: &Log,
    trusted_genesis_hash: &str,
    final_heads: &BTreeSet<String>,
) -> Result<KeysetState> {
    fold_with_recovery(log, trusted_genesis_hash, final_heads, &NoRecovery)
}

/// How a fold judges `RecoveryRotation` records.
///
/// Guardian recovery's Rule R weighs annex records and directory
/// first-seen times that live beside the log, so the log alone cannot
/// say whether a rotation folds. [`fold`] and [`fold_with_finality`]
/// judge none (no rotation folds); [`fold_with_recovery`] takes a rule,
/// such as `crate::recovery::RecoveryEvidence`.
pub trait RecoveryRule {
    /// The state the `RecoveryRotation` `record` installs when it
    /// continues the fold `head`, before the head advances, or `None`
    /// when it must not fold. `head` is as folded, with any pending
    /// guardian change not yet matured.
    fn rotate(&self, head: &KeysetState, record: &KeysetRecord) -> Option<KeysetState>;

    /// When the directory first saw the keyset record with this hash,
    /// if known. The fold uses it to mature delayed guardian-set
    /// changes; with no times, no delayed change ever matures.
    fn first_seen(&self, _record_hash: &str) -> Option<u64> {
        None
    }
}

/// `state` with its pending guardian change matured as of when the
/// directory first saw `record`, the next record to fold onto it.
fn matured_before(
    rule: &dyn RecoveryRule,
    state: &KeysetState,
    record: &KeysetRecord,
) -> KeysetState {
    let pending_seen = state
        .pending_guardian_set
        .as_ref()
        .and_then(|pending| rule.first_seen(&pending.record_hash));
    state.matured(pending_seen, rule.first_seen(&record.hash))
}

/// The rule that folds no recovery rotation.
struct NoRecovery;

impl RecoveryRule for NoRecovery {
    fn rotate(&self, _head: &KeysetState, _record: &KeysetRecord) -> Option<KeysetState> {
        None
    }
}

/// [`fold_with_finality`] with recovery rotations judged by `rule`.
///
/// A rotation that `rule` accepts continues its pinned head like any
/// admin record, with two limits at a fork (the guardian recovery
/// design's split, item 2). It never outranks a final head. And it
/// takes no revocation priority: when the pinned head has another valid
/// continuation, a surviving key is still acting, so that branch wins
/// and the recovery is moot (any surviving key could cancel it anyway).
///
/// # Errors
///
/// Fails if the trusted genesis is missing or malformed.
pub fn fold_with_recovery(
    log: &Log,
    trusted_genesis_hash: &str,
    final_heads: &BTreeSet<String>,
    rule: &dyn RecoveryRule,
) -> Result<KeysetState> {
    let (state, lamport) = genesis_state(log, trusted_genesis_hash)?;
    let mut branches = HashMap::new();
    let fold = Fold {
        log,
        final_heads,
        rule,
    };
    Ok(fold.extend(state, lamport, &mut branches)?.state)
}

/// Heads on the canonical chain (folded under `final_heads`) that a
/// valid `Checkpoint` covers.
///
/// A covering checkpoint is one signed by a device key active at that
/// head whose record count and state hash match the fold of the chain
/// up to it (rule 6). A replica that finds such a head adds it to its
/// final heads; from then on that prefix cannot be displaced.
///
/// # Errors
///
/// Fails if the trusted genesis is missing or malformed.
pub fn checkpoint_final_heads(
    log: &Log,
    trusted_genesis_hash: &str,
    final_heads: &BTreeSet<String>,
) -> Result<BTreeSet<String>> {
    checkpoint_final_heads_with_recovery(log, trusted_genesis_hash, final_heads, &NoRecovery)
}

/// [`checkpoint_final_heads`] over the chain [`fold_with_recovery`]
/// picks, so a checkpoint taken after a recovery can finalize it.
///
/// # Errors
///
/// Fails if the trusted genesis is missing or malformed, or the
/// canonical chain cannot be replayed.
pub fn checkpoint_final_heads_with_recovery(
    log: &Log,
    trusted_genesis_hash: &str,
    final_heads: &BTreeSet<String>,
    rule: &dyn RecoveryRule,
) -> Result<BTreeSet<String>> {
    let checkpoints: Vec<&KeysetRecord> = log
        .records
        .values()
        .filter(|record| matches!(record.entry, KeysetEntry::Checkpoint { .. }))
        .collect();
    let mut covered = BTreeSet::new();
    if checkpoints.is_empty() {
        return Ok(covered);
    }
    for prefix in canonical_prefixes(log, trusted_genesis_hash, final_heads, rule)? {
        let verified = checkpoints.iter().any(|record| {
            record.prev_hash == prefix.head_hash && apply_entry(&mut prefix.clone(), record).is_ok()
        });
        if verified {
            covered.insert(prefix.head_hash.clone());
        }
    }
    Ok(covered)
}

/// The folded state at every record of the canonical chain, genesis
/// first.
fn canonical_prefixes(
    log: &Log,
    trusted_genesis_hash: &str,
    final_heads: &BTreeSet<String>,
    rule: &dyn RecoveryRule,
) -> Result<Vec<KeysetState>> {
    let head = fold_with_recovery(log, trusted_genesis_hash, final_heads, rule)?;
    let by_hash: HashMap<&str, &KeysetRecord> = log
        .records
        .values()
        .map(|record| (record.hash.as_str(), record))
        .collect();
    let mut chain = Vec::new();
    let mut cursor = head.head_hash.as_str();
    while cursor != trusted_genesis_hash {
        let record = by_hash
            .get(cursor)
            .context("canonical chain record missing from log")?;
        chain.push(*record);
        cursor = record.prev_hash.as_str();
    }
    let (mut state, _) = genesis_state(log, trusted_genesis_hash)?;
    let mut prefixes = vec![state.clone()];
    for record in chain.into_iter().rev() {
        if is_recovery(record) {
            state = rule
                .rotate(&state, record)
                .context("canonical recovery rotation no longer folds")?;
        } else {
            state = matured_before(rule, &state, record);
            apply_entry(&mut state, record)?;
        }
        state.head_hash.clone_from(&record.hash);
        state.records_folded = state
            .records_folded
            .checked_add(1)
            .context("keyset record count overflow")?;
        prefixes.push(state.clone());
    }
    Ok(prefixes)
}

/// The state at the trusted genesis, and the genesis lamport.
fn genesis_state(log: &Log, trusted_genesis_hash: &str) -> Result<(KeysetState, u64)> {
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
    // Only the trusted genesis anchors the fold. Any other genesis is
    // a competing root: it loses to the trusted anchor (ADR Decision 5)
    // and is ignored, never folded — a stray or hostile root carried in
    // by a merged replica log must not make the trusted chain
    // unfoldable.
    let genesis = roots
        .into_iter()
        .find(|record| record.hash == trusted_genesis_hash)
        .context("trusted keyset genesis is missing from log")?;

    let KeysetEntry::Genesis {
        admin_key,
        device_key,
        label,
    } = &genesis.entry
    else {
        anyhow::bail!("selected keyset root is not a genesis record");
    };
    let (admin_key, device_key, label) = (admin_key.clone(), device_key.clone(), label.clone());
    let state = KeysetState {
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
        pending_guardian_set: None,
        head_hash: genesis.hash.clone(),
        records_folded: 1,
    };
    Ok((state, genesis.lamport))
}

/// Where a branch of the chain leads: the state its canonical
/// continuation folds to, and every key a record on it took authority
/// from (an admin key rotated away, or a device key revoked).
#[derive(Clone)]
struct Branch {
    state: KeysetState,
    removed: BTreeSet<String>,
    /// Whether the branch passes through a checkpoint-final head.
    pinned: bool,
}

const fn is_recovery(record: &KeysetRecord) -> bool {
    matches!(record.entry, KeysetEntry::RecoveryRotation { .. })
}

/// One fold's fixed inputs.
struct Fold<'a> {
    log: &'a Log,
    final_heads: &'a BTreeSet<String>,
    rule: &'a dyn RecoveryRule,
}

impl Fold<'_> {
    /// The state a valid continuation of `state` installs, if `record`
    /// is one.
    fn continuation(&self, state: &KeysetState, record: &KeysetRecord) -> Option<KeysetState> {
        // A rotation is judged against the head as its request saw it,
        // so the rule matures pending changes itself.
        if is_recovery(record) {
            return self.rule.rotate(state, record);
        }
        let mut candidate = matured_before(self.rule, state, record);
        if !candidate.is_admin(&record.author) {
            return None;
        }
        apply_entry(&mut candidate, record).ok().map(|()| candidate)
    }

    /// Folds the canonical continuation of `state`, whose head was folded
    /// at `head_lamport`. At a fork, a branch that rotates away or revokes
    /// the key that authored a sibling wins regardless of merge key
    /// (revocation priority, ADR Decision 4 rule 5): otherwise a key still
    /// authorised at the shared parent could sign a sibling that sorts
    /// first and erase its own revocation. When no branch revokes another,
    /// or branches revoke each other, the smallest merge key wins.
    /// `branches` memoises each record's continuation, so a log folds in
    /// time proportional to its records however its forks nest.
    fn extend(
        &self,
        state: KeysetState,
        head_lamport: u64,
        branches: &mut HashMap<String, Branch>,
    ) -> Result<Branch> {
        let candidates: Vec<(&KeysetRecord, KeysetState)> = self
            .log
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
            .filter_map(|record| {
                self.continuation(&state, record)
                    .map(|candidate| (record, candidate))
            })
            .collect();
        if candidates.is_empty() {
            return Ok(Branch {
                state,
                removed: BTreeSet::new(),
                pinned: false,
            });
        }
        let mut followed = Vec::with_capacity(candidates.len());
        for (record, mut next_state) in candidates {
            let branch = if let Some(branch) = branches.get(&record.hash) {
                branch.clone()
            } else {
                let mut removed = removed_by(&state, &next_state);
                next_state.head_hash.clone_from(&record.hash);
                next_state.records_folded = next_state
                    .records_folded
                    .checked_add(1)
                    .context("keyset record count overflow")?;
                let tail = self.extend(next_state, record.lamport, branches)?;
                removed.extend(tail.removed);
                let branch = Branch {
                    state: tail.state,
                    removed,
                    pinned: tail.pinned || self.final_heads.contains(&record.hash),
                };
                branches.insert(record.hash.clone(), branch.clone());
                branch
            };
            followed.push((record, branch));
        }
        let chosen = choose(&followed).context("keyset fork with no candidate")?;
        Ok(followed.swap_remove(chosen).1)
    }
}

/// Fork choice among followed continuations. Checkpoint finality
/// outranks every other rule: a branch through a final head is the
/// chain. Otherwise a recovery rotation yields to any ordinary sibling,
/// then revocation priority, then the smallest merge key.
fn choose(followed: &[(&KeysetRecord, Branch)]) -> Option<usize> {
    let all: Vec<usize> = (0..followed.len()).collect();
    let pinned: Vec<usize> = all
        .iter()
        .copied()
        .filter(|index| followed[*index].1.pinned)
        .collect();
    let pool = if pinned.is_empty() {
        let ordinary: Vec<usize> = all
            .iter()
            .copied()
            .filter(|index| !is_recovery(followed[*index].0))
            .collect();
        let contenders = if ordinary.is_empty() { all } else { ordinary };
        let standing: Vec<usize> = contenders
            .iter()
            .copied()
            .filter(|index| {
                !contenders.iter().any(|other| {
                    other != index
                        && followed[*other]
                            .1
                            .removed
                            .contains(&followed[*index].0.author)
                })
            })
            .collect();
        if standing.is_empty() {
            contenders
        } else {
            standing
        }
    } else {
        pinned
    };
    pool.into_iter()
        .min_by_key(|index| followed[*index].0.key())
}

/// The keys one record took authority from: admin keys that were active
/// before it and are not after, and devices it revoked.
fn removed_by(before: &KeysetState, after: &KeysetState) -> BTreeSet<String> {
    let admins = before
        .admin_keys
        .keys()
        .filter(|key| before.is_admin(key) && !after.is_admin(key));
    let devices = before
        .devices
        .keys()
        .filter(|key| before.is_device(key) && !after.is_device(key));
    admins.chain(devices).cloned().collect()
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
            let next = GuardianSet {
                guardians: guardians.clone(),
                threshold: *threshold,
            };
            // Adding or raising folds at once and clears any pending
            // change, so restoring the current set cancels a pending
            // removal. Removing or lowering waits (Decision 6).
            let widens = state
                .guardian_set
                .as_ref()
                .is_none_or(|current| current.widens_to(&next));
            if widens {
                state.guardian_set = Some(next);
                state.pending_guardian_set = None;
            } else {
                state.pending_guardian_set = Some(PendingGuardianSet {
                    record_hash: record.hash.clone(),
                    set: next,
                });
            }
        }
        KeysetEntry::RecoveryRotation { .. } => {
            anyhow::bail!("a recovery rotation folds only under the recovery rule")
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

    /// Lamport 2 at the genesis head, with the rotation's merge key
    /// forced to sort after the sibling's: varying the label until it
    /// does is exactly the grinding the ADR warns about.
    fn sibling_sorting_before(rotation: &KeysetRecord, parent: &KeysetRecord) -> KeysetRecord {
        let admin = key(1);
        (0..)
            .map(|n| {
                append(
                    parent,
                    &admin,
                    rotation.lamport,
                    KeysetEntry::IdpBindingBound {
                        binding_id: format!("attacker-{n}"),
                    },
                    None,
                )
            })
            .find(|sibling| sibling.key() < rotation.key())
            .unwrap()
    }

    #[test]
    fn a_rotation_beats_a_sibling_from_the_key_it_rotates_away() {
        let first = genesis();
        let rotation = append(
            &first,
            &key(1),
            2,
            KeysetEntry::AdminKeyRotated {
                new_admin_key: public(5),
            },
            Some(&key(5)),
        );
        let sibling = sibling_sorting_before(&rotation, &first);
        let mut log = Log::default();
        for record in [&first, &rotation, &sibling] {
            log.insert(record.clone()).unwrap();
        }
        let state = fold(&log, &first.hash).unwrap();
        assert_eq!(state.head_hash, rotation.hash);
        assert!(state.is_admin(&public(5)));
        assert!(state.idp_bindings.is_empty(), "the sibling must not fold");
    }

    #[test]
    fn a_rotation_deeper_in_the_branch_still_wins_the_fork() {
        let first = genesis();
        let enrolled = append(
            &first,
            &key(1),
            2,
            KeysetEntry::DeviceEnrolled {
                device_key: public(2),
                label: "phone".to_owned(),
            },
            Some(&key(2)),
        );
        let rotation = append(
            &enrolled,
            &key(1),
            3,
            KeysetEntry::AdminKeyRotated {
                new_admin_key: public(5),
            },
            Some(&key(5)),
        );
        let sibling = sibling_sorting_before(&enrolled, &first);
        let mut log = Log::default();
        for record in [&first, &enrolled, &rotation, &sibling] {
            log.insert(record.clone()).unwrap();
        }
        let state = fold(&log, &first.hash).unwrap();
        assert_eq!(state.head_hash, rotation.hash);
        assert!(state.is_device(&public(2)));
        assert!(state.idp_bindings.is_empty());
    }

    #[test]
    fn mutual_rotations_fall_back_to_merge_key_order() {
        let first = genesis();
        let to_five = append(
            &first,
            &key(1),
            2,
            KeysetEntry::AdminKeyRotated {
                new_admin_key: public(5),
            },
            Some(&key(5)),
        );
        let to_six = append(
            &first,
            &key(1),
            2,
            KeysetEntry::AdminKeyRotated {
                new_admin_key: public(6),
            },
            Some(&key(6)),
        );
        let mut log = Log::default();
        for record in [&first, &to_five, &to_six] {
            log.insert(record.clone()).unwrap();
        }
        let winner = if to_five.key() < to_six.key() { 5 } else { 6 };
        let state = fold(&log, &first.hash).unwrap();
        assert!(state.is_admin(&public(winner)));
        assert_eq!(state.records_folded, 2);
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
        let unrelated_root = KeysetRecord::sign(
            &key(9),
            1,
            "",
            KeysetEntry::Genesis {
                admin_key: public(9),
                device_key: public(9),
                label: "unrelated identity".to_owned(),
            },
            None,
        )
        .unwrap();
        let mut log = Log::default();
        for record in [&trusted, &rotation, &forged_root, &unrelated_root] {
            log.insert(record.clone()).unwrap();
        }

        // Neither competing root resets or poisons the fold: both are
        // ignored, the trusted chain folds, and the rotation stands.
        let state = fold(&log, &trusted.hash).unwrap();
        assert!(!state.is_admin(&public(1)));
        assert!(state.is_admin(&public(4)));

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

    /// Genesis (admin 1, device 2), then the admin binds "x" at lamport
    /// 5, and device 2 checkpoints that head. Returns the log, the
    /// genesis, the bound head and the checkpoint.
    fn checkpointed_chain() -> (Log, KeysetRecord, KeysetRecord, KeysetRecord) {
        let admin = key(1);
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
        let bound = append(
            &first,
            &admin,
            5,
            KeysetEntry::IdpBindingBound {
                binding_id: "x".to_owned(),
            },
            None,
        );
        let mut log = Log::default();
        log.insert(first.clone()).unwrap();
        log.insert(bound.clone()).unwrap();
        let at_bound = fold(&log, &first.hash).unwrap();
        let checkpoint = append(
            &bound,
            &key(2),
            6,
            KeysetEntry::Checkpoint {
                state_hash: at_bound.state_hash().unwrap(),
                record_count: at_bound.records_folded,
            },
            None,
        );
        log.insert(checkpoint.clone()).unwrap();
        (log, first, bound, checkpoint)
    }

    #[test]
    fn a_verified_checkpoint_makes_its_head_final() {
        let (log, first, bound, _) = checkpointed_chain();
        let finals = checkpoint_final_heads(&log, &first.hash, &BTreeSet::new()).unwrap();
        assert_eq!(finals, BTreeSet::from([bound.hash]));
    }

    #[test]
    fn a_checkpoint_that_does_not_match_or_is_not_a_devices_finalizes_nothing() {
        let (mut log, first, bound, _) = checkpointed_chain();
        let at_bound = fold(&log, &first.hash).unwrap();
        // Signed by a key that is no device of this identity.
        let stranger = append(
            &bound,
            &key(9),
            7,
            KeysetEntry::Checkpoint {
                state_hash: at_bound.state_hash().unwrap(),
                record_count: at_bound.records_folded,
            },
            None,
        );
        // Signed by the device, but attesting the wrong state.
        let wrong = append(
            &bound,
            &key(2),
            8,
            KeysetEntry::Checkpoint {
                state_hash: first.hash.clone(),
                record_count: at_bound.records_folded,
            },
            None,
        );
        let mut bad = Log::default();
        for record in log.records() {
            if !matches!(record.entry, KeysetEntry::Checkpoint { .. }) {
                bad.insert(record.clone()).unwrap();
            }
        }
        bad.insert(stranger).unwrap();
        bad.insert(wrong).unwrap();
        assert!(
            checkpoint_final_heads(&bad, &first.hash, &BTreeSet::new())
                .unwrap()
                .is_empty()
        );
        // The good checkpoint still counts beside them.
        log = merge(&log, &bad);
        assert_eq!(
            checkpoint_final_heads(&log, &first.hash, &BTreeSet::new()).unwrap(),
            BTreeSet::from([bound.hash])
        );
    }

    #[test]
    fn a_late_sibling_with_a_smaller_merge_key_cannot_displace_a_final_head() {
        let (mut log, first, bound, _) = checkpointed_chain();
        let finals = checkpoint_final_heads(&log, &first.hash, &BTreeSet::new()).unwrap();
        let sibling = append(
            &first,
            &key(1),
            2,
            KeysetEntry::IdpBindingBound {
                binding_id: "y".to_owned(),
            },
            None,
        );
        log.insert(sibling.clone()).unwrap();
        // Without finality, the smaller merge key wins the fork.
        assert_eq!(fold(&log, &first.hash).unwrap().head_hash, sibling.hash);
        let state = fold_with_finality(&log, &first.hash, &finals).unwrap();
        assert_eq!(state.head_hash, bound.hash);
        assert_eq!(state.idp_bindings.get("x"), Some(&BindingState::Bound));
        assert!(!state.idp_bindings.contains_key("y"));
    }

    #[test]
    fn a_late_revoking_sibling_cannot_displace_a_final_head() {
        let (mut log, first, bound, _) = checkpointed_chain();
        let finals = checkpoint_final_heads(&log, &first.hash, &BTreeSet::new()).unwrap();
        // The admin rotates away, which would revoke the author of the
        // final branch under revocation priority.
        let rotation = append(
            &first,
            &key(1),
            9,
            KeysetEntry::AdminKeyRotated {
                new_admin_key: public(3),
            },
            Some(&key(3)),
        );
        log.insert(rotation.clone()).unwrap();
        assert_eq!(fold(&log, &first.hash).unwrap().head_hash, rotation.hash);
        let state = fold_with_finality(&log, &first.hash, &finals).unwrap();
        assert_eq!(state.head_hash, bound.hash);
        assert!(state.is_admin(&public(1)));
        // The final head still extends normally.
        let next = append(
            &bound,
            &key(1),
            10,
            KeysetEntry::IdpBindingRevoked {
                binding_id: "x".to_owned(),
            },
            None,
        );
        log.insert(next.clone()).unwrap();
        assert_eq!(
            fold_with_finality(&log, &first.hash, &finals)
                .unwrap()
                .head_hash,
            next.hash
        );
    }

    /// One step the generated-history property test can append. Indices
    /// pick from a small key universe so collisions (re-enrol, double
    /// revoke, rotate-to-self, unbind-before-bind) actually occur; the
    /// model oracle below decides what the fold must do with each.
    #[derive(Clone, Debug)]
    enum HistoryOp {
        Enroll(u8),
        Revoke(u8),
        Rotate(u8),
        Bind(u8),
        Unbind(u8),
        Checkpoint { max_lamport: bool },
        UnauthorizedEntry,
        FormerAdminEntry,
        CompetingGenesis(u8),
    }

    fn history_op() -> impl Strategy<Value = HistoryOp> {
        prop_oneof![
            3 => (2..=6u8).prop_map(HistoryOp::Enroll),
            2 => (2..=6u8).prop_map(HistoryOp::Revoke),
            2 => (2..=6u8).prop_map(HistoryOp::Rotate),
            2 => (0..=2u8).prop_map(HistoryOp::Bind),
            1 => (0..=2u8).prop_map(HistoryOp::Unbind),
            1 => any::<bool>().prop_map(|max_lamport| HistoryOp::Checkpoint { max_lamport }),
            1 => Just(HistoryOp::UnauthorizedEntry),
            1 => Just(HistoryOp::FormerAdminEntry),
            1 => (7..=9u8).prop_map(HistoryOp::CompetingGenesis),
        ]
    }

    /// Spec-level replay of a generated history: the oracle the fold is
    /// checked against. Valid operations extend the chain head and are
    /// applied to the model state exactly as apply_entry applies them;
    /// records that can never fold (unauthorised author, failed
    /// precondition, checkpoint, competing genesis) are emitted as noise
    /// extending the current head and change nothing.
    struct HistoryModel {
        admin_seed: u8,
        former_admin_seed: Option<u8>,
        lamport: u64,
        head: KeysetRecord,
        state: KeysetState,
        records: Vec<KeysetRecord>,
    }

    impl HistoryModel {
        fn new() -> Self {
            let first = genesis();
            let state = KeysetState {
                identity_id: public(1),
                admin_keys: BTreeMap::from([(public(1), AdminKeyState::Active)]),
                devices: BTreeMap::from([(
                    public(1),
                    DeviceState {
                        label: "first device".to_owned(),
                        enrolled_at: 1,
                        revoked: false,
                    },
                )]),
                idp_bindings: BTreeMap::new(),
                guardian_set: None,
                pending_guardian_set: None,
                head_hash: first.hash.clone(),
                records_folded: 1,
            };
            Self {
                admin_seed: 1,
                former_admin_seed: None,
                lamport: 1,
                head: first.clone(),
                state,
                records: vec![first],
            }
        }

        fn next_lamport(&mut self) -> u64 {
            self.lamport += 1;
            self.lamport
        }

        fn emit(&mut self, record: KeysetRecord, advance: bool) {
            if advance {
                self.state.head_hash = record.hash.clone();
                self.state.records_folded += 1;
                self.head = record.clone();
            }
            self.records.push(record);
        }

        fn apply(&mut self, op: &HistoryOp) {
            let admin = key(self.admin_seed);
            match op {
                HistoryOp::Enroll(seed) => {
                    let lamport = self.next_lamport();
                    let record = KeysetRecord::sign(
                        &admin,
                        lamport,
                        self.head.hash.clone(),
                        KeysetEntry::DeviceEnrolled {
                            device_key: public(*seed),
                            label: format!("device-{seed}"),
                        },
                        Some(&key(*seed)),
                    )
                    .unwrap();
                    self.state.devices.insert(
                        public(*seed),
                        DeviceState {
                            label: format!("device-{seed}"),
                            enrolled_at: lamport,
                            revoked: false,
                        },
                    );
                    self.emit(record, true);
                }
                HistoryOp::Revoke(seed) => {
                    let lamport = self.next_lamport();
                    let record = KeysetRecord::sign(
                        &admin,
                        lamport,
                        self.head.hash.clone(),
                        KeysetEntry::DeviceRevoked {
                            device_key: public(*seed),
                            reason: "test".to_owned(),
                        },
                        None,
                    )
                    .unwrap();
                    let valid = self
                        .state
                        .devices
                        .get(&public(*seed))
                        .is_some_and(|device| !device.revoked);
                    if valid {
                        self.state.devices.get_mut(&public(*seed)).unwrap().revoked = true;
                    }
                    self.emit(record, valid);
                }
                HistoryOp::Rotate(seed) => {
                    let lamport = self.next_lamport();
                    let record = KeysetRecord::sign(
                        &admin,
                        lamport,
                        self.head.hash.clone(),
                        KeysetEntry::AdminKeyRotated {
                            new_admin_key: public(*seed),
                        },
                        Some(&key(*seed)),
                    )
                    .unwrap();
                    if *seed == self.admin_seed {
                        self.emit(record, false);
                    } else {
                        self.state
                            .admin_keys
                            .insert(public(self.admin_seed), AdminKeyState::Rotated);
                        self.state
                            .admin_keys
                            .insert(public(*seed), AdminKeyState::Active);
                        self.former_admin_seed = Some(self.admin_seed);
                        self.admin_seed = *seed;
                        self.emit(record, true);
                    }
                }
                HistoryOp::Bind(id) => {
                    let lamport = self.next_lamport();
                    let binding_id = format!("binding-{id}");
                    let record = KeysetRecord::sign(
                        &admin,
                        lamport,
                        self.head.hash.clone(),
                        KeysetEntry::IdpBindingBound {
                            binding_id: binding_id.clone(),
                        },
                        None,
                    )
                    .unwrap();
                    self.state
                        .idp_bindings
                        .insert(binding_id, BindingState::Bound);
                    self.emit(record, true);
                }
                HistoryOp::Unbind(id) => {
                    let lamport = self.next_lamport();
                    let binding_id = format!("binding-{id}");
                    let record = KeysetRecord::sign(
                        &admin,
                        lamport,
                        self.head.hash.clone(),
                        KeysetEntry::IdpBindingRevoked {
                            binding_id: binding_id.clone(),
                        },
                        None,
                    )
                    .unwrap();
                    let valid =
                        self.state.idp_bindings.get(&binding_id) == Some(&BindingState::Bound);
                    if valid {
                        self.state
                            .idp_bindings
                            .insert(binding_id, BindingState::Revoked);
                    }
                    self.emit(record, valid);
                }
                HistoryOp::Checkpoint { max_lamport } => {
                    let lamport = if *max_lamport {
                        u64::MAX
                    } else {
                        self.next_lamport()
                    };
                    let record = KeysetRecord::sign(
                        &key(1),
                        lamport,
                        self.head.hash.clone(),
                        KeysetEntry::Checkpoint {
                            state_hash: self.state.state_hash().unwrap(),
                            record_count: self.state.records_folded,
                        },
                        None,
                    )
                    .unwrap();
                    self.emit(record, false);
                }
                HistoryOp::UnauthorizedEntry => {
                    let lamport = self.next_lamport();
                    let record = KeysetRecord::sign(
                        &key(42),
                        lamport,
                        self.head.hash.clone(),
                        KeysetEntry::DeviceEnrolled {
                            device_key: public(43),
                            label: "rogue".to_owned(),
                        },
                        Some(&key(43)),
                    )
                    .unwrap();
                    self.emit(record, false);
                }
                HistoryOp::FormerAdminEntry => {
                    if let Some(former) = self.former_admin_seed {
                        if self.state.admin_keys.get(&public(former))
                            != Some(&AdminKeyState::Active)
                        {
                            let lamport = self.next_lamport();
                            let record = KeysetRecord::sign(
                                &key(former),
                                lamport,
                                self.head.hash.clone(),
                                KeysetEntry::IdpBindingBound {
                                    binding_id: "former-admin".to_owned(),
                                },
                                None,
                            )
                            .unwrap();
                            self.emit(record, false);
                        }
                    }
                }
                HistoryOp::CompetingGenesis(seed) => {
                    let lamport = self.next_lamport();
                    let record = KeysetRecord::sign(
                        &key(*seed),
                        lamport,
                        "",
                        KeysetEntry::Genesis {
                            admin_key: public(*seed),
                            device_key: public(*seed),
                            label: "competing".to_owned(),
                        },
                        None,
                    )
                    .unwrap();
                    self.emit(record, false);
                }
            }
        }
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

    proptest! {
        // 64 cases keeps the generated-history oracle near a minute of
        // test time; each case still replays up to 40 operations.
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn generated_histories_fold_to_the_model_state(
            ops in prop::collection::vec(history_op(), 0..40),
        ) {
            let mut model = HistoryModel::new();
            for op in &ops {
                model.apply(op);
            }
            let genesis_hash = model.records[0].hash.clone();

            let mut forward = Log::default();
            for record in &model.records {
                prop_assert!(forward.insert(record.clone()).is_ok());
            }
            let mut reverse = Log::default();
            for record in model.records.iter().rev() {
                prop_assert!(reverse.insert(record.clone()).is_ok());
            }
            let split = model.records.len() / 2;
            let mut left = Log::default();
            let mut right = Log::default();
            for record in &model.records[..split] {
                left.insert(record.clone()).unwrap();
            }
            for record in &model.records[split..] {
                right.insert(record.clone()).unwrap();
            }
            let merged = merge(&left, &right);

            let expected = model.state.clone();
            for log in [&forward, &reverse, &merged] {
                prop_assert_eq!(fold(log, &genesis_hash).unwrap(), expected.clone());
            }
        }
    }
}
