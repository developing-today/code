//! Persistent signed [`ListVersion`] store (#8201 store slice;
//! design `doc/2026-11-10T19-52-00Z_design_id_sanctions_boundary/`
//! §D3, parent design `doc/2026-11-08_design_id5683.md` §5).
//!
//! [`crate::sanctions`] is the data model and the evaluator; this
//! module is the lifecycle that feeds it. A list version moves
//! through three recorded states (design §D3):
//!
//! - **staged** — content present, signature unverified. A staged
//!   version enforces nothing.
//! - **validated** — the signature over the canonical encoding
//!   verified against the supplier key, the schema checked, and
//!   the entry-count delta against the previous version recorded.
//! - **activated** — an operator action made this version the
//!   list's enforceable one, at its `effective_from`. Activating a
//!   version closes the previously activated one (its stored
//!   `effective_to` is set to the new version's `effective_from`;
//!   that is lifecycle bookkeeping on the store's copy — list
//!   *content* is never edited, and a version's entries, source,
//!   and signature are immutable from staging on).
//!
//! Enforcement of the transitions is the store's whole job: an
//! unvalidated or bad-signature version cannot be activated, a
//! staged `(id, version)` can never be overwritten or re-staged,
//! and versions stage monotonically per list. Evaluation reads
//! through [`ListVersionStore::evaluate_active`], so every
//! decision names the exact activated list version behind it
//! (the decision record itself is [`crate::sanctions::evaluate`]'s,
//! which already stamps the list reference, §5.1).
//!
//! Persistence is a serde snapshot of the whole store
//! ([`ListVersionStore::save`] / [`ListVersionStore::load`]):
//! states, validation records, and activation stamps round-trip
//! verbatim, so a decision taken before a restart stays
//! reproducible after it. The store lives in `id-core` beside the
//! model it serves; where the snapshot bytes are kept is the
//! caller's choice.
//!
//! The canonical encoding a supplier signs is
//! [`canonical_list_bytes`]: the JSON serialization of the
//! version with its `signature` field emptied, fields in
//! declaration order, entries in published order. It is a
//! definition, not a discovery — a signature over any other
//! encoding does not verify here, by design.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::boundary::EnforcementPoint;
use crate::hex::hex_encode;
use crate::sanctions::{
    ListVersion, SanctionsBoundary, SanctionsDecision, SanctionsEvaluationError, evaluate,
};

/// The recorded lifecycle state of one stored version (design §D3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListVersionState {
    /// Content present, signature unverified.
    Staged,
    /// Signature verified, schema checked, delta recorded.
    Validated,
    /// The list's enforceable version (subject to its
    /// `effective_from`/`effective_to` window).
    Activated,
    /// Was activated, then closed by a newer activation.
    Superseded,
}

/// What validation established about a version, recorded at
/// validation time so the activation decision is auditable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationRecord {
    /// Hex of the supplier verifying key the signature checked
    /// against.
    pub supplier_key: String,
    /// Entry count of this version minus the previous version's
    /// (the previous version is the highest lower version the
    /// store holds, in any state; for a first version the delta
    /// is its own entry count and `previous_version` is `None`).
    pub entry_count_delta: i64,
    /// The version the delta was taken against, if any.
    pub previous_version: Option<u64>,
}

/// One version in the store: the immutable list content plus its
/// lifecycle bookkeeping.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredListVersion {
    /// The list content as supplied. Never edited after staging,
    /// except the store closing `effective_to` when a newer
    /// version supersedes this one (design §D3).
    pub version: ListVersion,
    /// Where the version is in its lifecycle.
    pub state: ListVersionState,
    /// Set at validation; `None` while staged.
    pub validation: Option<ValidationRecord>,
    /// Unix seconds at which the operator activated it, if it
    /// was ever activated.
    pub activated_at: Option<u64>,
}

/// Why the store refused an operation. Every refusal is typed so
/// an operator surface can name it; no refusal mutates the store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListStoreError {
    /// A version with this `(id, version)` is already stored, in
    /// some state — staged content is never overwritten.
    DuplicateVersion {
        /// List id.
        id: String,
        /// Version number.
        version: u64,
    },
    /// Versions stage monotonically per list; this one does not
    /// exceed the highest version already held.
    NonMonotoneVersion {
        /// List id.
        id: String,
        /// Version offered.
        version: u64,
        /// Highest version already held for the list.
        highest: u64,
    },
    /// No list with this id is stored.
    UnknownList {
        /// List id.
        id: String,
    },
    /// The list is stored, but not this version.
    UnknownVersion {
        /// List id.
        id: String,
        /// Version number.
        version: u64,
    },
    /// The version is not in the state the operation requires.
    WrongState {
        /// List id.
        id: String,
        /// Version number.
        version: u64,
        /// State required by the operation.
        expected: ListVersionState,
        /// State the version is actually in.
        actual: ListVersionState,
    },
    /// The signature does not verify against the supplier key.
    /// The version stays staged.
    BadSignature {
        /// List id.
        id: String,
        /// Version number.
        version: u64,
    },
    /// The stored `signature` is not hex for a 64-byte ed25519
    /// signature at all.
    MalformedSignature {
        /// List id.
        id: String,
        /// Version number.
        version: u64,
    },
    /// A schema check failed (duplicate entry keys, empty entry
    /// key). The version stays staged.
    Schema {
        /// List id.
        id: String,
        /// Version number.
        version: u64,
        /// What failed.
        problem: String,
    },
    /// Evaluation was asked for a list with no version in force.
    NoActiveVersion {
        /// List id.
        id: String,
    },
    /// The evaluation itself refused (wrong list, not active,
    /// out of scope — see [`SanctionsEvaluationError`]).
    Evaluation(SanctionsEvaluationError),
    /// Snapshot read/write failed.
    Io(String),
    /// Snapshot bytes did not decode.
    Codec(String),
}

impl fmt::Display for ListStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateVersion { id, version } => {
                write!(f, "list {id} version {version} is already stored")
            }
            Self::NonMonotoneVersion {
                id,
                version,
                highest,
            } => write!(
                f,
                "list {id} version {version} does not exceed highest stored version {highest}"
            ),
            Self::UnknownList { id } => write!(f, "no stored list with id {id}"),
            Self::UnknownVersion { id, version } => {
                write!(f, "list {id} has no stored version {version}")
            }
            Self::WrongState {
                id,
                version,
                expected,
                actual,
            } => write!(
                f,
                "list {id} version {version} is {actual:?}, operation requires {expected:?}"
            ),
            Self::BadSignature { id, version } => {
                write!(f, "list {id} version {version}: signature does not verify")
            }
            Self::MalformedSignature { id, version } => write!(
                f,
                "list {id} version {version}: signature is not a hex ed25519 signature"
            ),
            Self::Schema {
                id,
                version,
                problem,
            } => write!(f, "list {id} version {version}: schema: {problem}"),
            Self::NoActiveVersion { id } => {
                write!(f, "list {id} has no activated version in force")
            }
            Self::Evaluation(e) => write!(f, "evaluation refused: {e:?}"),
            Self::Io(m) => write!(f, "store snapshot io: {m}"),
            Self::Codec(m) => write!(f, "store snapshot codec: {m}"),
        }
    }
}

impl std::error::Error for ListStoreError {}

/// The canonical bytes a list signature covers: the version's
/// JSON serialization with the `signature` field emptied, fields
/// in declaration order, entries in published order.
#[must_use]
pub fn canonical_list_bytes(version: &ListVersion) -> Vec<u8> {
    let unsigned = ListVersion {
        signature: String::new(),
        ..version.clone()
    };
    serde_json::to_vec(&unsigned).unwrap_or_default()
}

/// Sign a version's canonical bytes with a supplier key and
/// return the hex signature to carry in [`ListVersion::signature`].
/// (The operator-side helper; the store itself only verifies.)
#[must_use]
pub fn sign_list(key: &SigningKey, version: &ListVersion) -> String {
    hex_encode(&key.sign(&canonical_list_bytes(version)).to_bytes())
}

fn decode_signature(id: &str, version: u64, hex: &str) -> Result<Signature, ListStoreError> {
    let malformed = || ListStoreError::MalformedSignature {
        id: id.to_string(),
        version,
    };
    if hex.len() != 128 {
        return Err(malformed());
    }
    let mut bytes = [0u8; 64];
    for (i, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
        let hi = char::from(pair[0]).to_digit(16).ok_or_else(&malformed)?;
        let lo = char::from(pair[1]).to_digit(16).ok_or_else(&malformed)?;
        bytes[i] = u8::try_from(hi * 16 + lo).map_err(|_| malformed())?;
    }
    Ok(Signature::from_bytes(&bytes))
}

/// The persistent signed list-version store. One store holds any
/// number of lists; each list's versions are keyed by their
/// monotone version number.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListVersionStore {
    lists: BTreeMap<String, BTreeMap<u64, StoredListVersion>>,
}

impl ListVersionStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stage a new version: content present, signature
    /// unverified, enforcing nothing. Refuses a duplicate
    /// `(id, version)` in any state and a version that does not
    /// exceed the list's highest held version.
    pub fn stage(&mut self, version: ListVersion) -> Result<(), ListStoreError> {
        let versions = self.lists.entry(version.id.clone()).or_default();
        if versions.contains_key(&version.version) {
            return Err(ListStoreError::DuplicateVersion {
                id: version.id,
                version: version.version,
            });
        }
        if let Some(&highest) = versions.keys().next_back() {
            if version.version <= highest {
                return Err(ListStoreError::NonMonotoneVersion {
                    id: version.id,
                    version: version.version,
                    highest,
                });
            }
        }
        versions.insert(
            version.version,
            StoredListVersion {
                version,
                state: ListVersionState::Staged,
                validation: None,
                activated_at: None,
            },
        );
        Ok(())
    }

    /// Validate a staged version against its supplier key:
    /// schema checks, then signature verification over
    /// [`canonical_list_bytes`], then the entry-count delta
    /// against the previous version is recorded. A refusal
    /// leaves the version staged.
    pub fn validate(
        &mut self,
        id: &str,
        version: u64,
        supplier_key: &VerifyingKey,
    ) -> Result<ValidationRecord, ListStoreError> {
        // State + schema first, under one mutable borrow; the
        // signature string and entry count are copied out so the
        // borrow is dropped before verification and the delta
        // read, which borrow the store immutably.
        let (signature_hex, entry_count) = {
            let stored = self.stored_mut(id, version)?;
            if stored.state != ListVersionState::Staged {
                return Err(ListStoreError::WrongState {
                    id: id.to_string(),
                    version,
                    expected: ListVersionState::Staged,
                    actual: stored.state,
                });
            }
            // Schema: entry keys are the match surface; an empty
            // or duplicated key makes the version's meaning
            // ambiguous.
            let mut keys = BTreeSet::new();
            for entry in &stored.version.entries {
                let key = entry.key();
                if key.is_empty() {
                    return Err(ListStoreError::Schema {
                        id: id.to_string(),
                        version,
                        problem: "entry with an empty match key".to_string(),
                    });
                }
                if !keys.insert(key.to_string()) {
                    return Err(ListStoreError::Schema {
                        id: id.to_string(),
                        version,
                        problem: format!("duplicate entry key {key}"),
                    });
                }
            }
            (
                stored.version.signature.clone(),
                stored.version.entries.len(),
            )
        };
        let signature = decode_signature(id, version, &signature_hex)?;
        let content = self
            .version(id, version)
            .ok_or_else(|| ListStoreError::UnknownVersion {
                id: id.to_string(),
                version,
            })?
            .clone();
        supplier_key
            .verify_strict(&canonical_list_bytes(&content), &signature)
            .map_err(|_| ListStoreError::BadSignature {
                id: id.to_string(),
                version,
            })?;
        let (previous_version, previous_count) = self
            .lists
            .get(id)
            .and_then(|versions| {
                versions
                    .range(..version)
                    .next_back()
                    .map(|(v, s)| (Some(*v), s.version.entries.len()))
            })
            .unwrap_or((None, 0));
        let record = ValidationRecord {
            supplier_key: hex_encode(supplier_key.as_bytes()),
            entry_count_delta: entry_count as i64 - previous_count as i64,
            previous_version,
        };
        let stored = self.stored_mut(id, version)?;
        stored.validation = Some(record.clone());
        stored.state = ListVersionState::Validated;
        Ok(record)
    }

    /// Activate a validated version: it becomes the list's
    /// enforceable version at its `effective_from`, and the
    /// previously activated version is closed (superseded, its
    /// stored `effective_to` set to this version's
    /// `effective_from`). Activating anything but a validated
    /// version is refused.
    pub fn activate(&mut self, id: &str, version: u64, now: u64) -> Result<(), ListStoreError> {
        let stored = self.stored_mut(id, version)?;
        if stored.state != ListVersionState::Validated {
            return Err(ListStoreError::WrongState {
                id: id.to_string(),
                version,
                expected: ListVersionState::Validated,
                actual: stored.state,
            });
        }
        let effective_from = stored.version.effective_from;
        if let Some(versions) = self.lists.get_mut(id) {
            for other in versions.values_mut() {
                if other.state == ListVersionState::Activated {
                    other.state = ListVersionState::Superseded;
                    other.version.effective_to = Some(effective_from);
                }
            }
        }
        let stored = self.stored_mut(id, version)?;
        stored.state = ListVersionState::Activated;
        stored.activated_at = Some(now);
        Ok(())
    }

    /// The version of `id` in force at `now`: an activated (or
    /// formerly activated, still inside its window) version whose
    /// [`ListVersion::is_active_at`] holds. `None` when the list
    /// enforces nothing at `now`.
    #[must_use]
    pub fn active_version(&self, id: &str, now: u64) -> Option<&ListVersion> {
        self.lists.get(id)?.values().rev().find_map(|stored| {
            (stored.activated_at.is_some() && stored.version.is_active_at(now))
                .then_some(&stored.version)
        })
    }

    /// Any stored version by reference, whatever its state — the
    /// reproducibility read: a past decision's list version stays
    /// retrievable forever.
    #[must_use]
    pub fn version(&self, id: &str, version: u64) -> Option<&ListVersion> {
        self.lists.get(id)?.get(&version).map(|s| &s.version)
    }

    /// The lifecycle state of a stored version, if it exists.
    #[must_use]
    pub fn state_of(&self, id: &str, version: u64) -> Option<ListVersionState> {
        self.lists.get(id)?.get(&version).map(|s| s.state)
    }

    /// Evaluate a subject against the list version currently in
    /// force for `boundary`'s list. The decision is
    /// [`crate::sanctions::evaluate`]'s and carries the list
    /// id + version it was taken under.
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate_active(
        &self,
        boundary: &SanctionsBoundary,
        point: EnforcementPoint,
        subject_country: Option<&str>,
        subject_native_ids: &BTreeSet<String>,
        subject_names: &BTreeSet<String>,
        subject_has_existing_access: bool,
        now: u64,
    ) -> Result<SanctionsDecision, ListStoreError> {
        let list = self.active_version(&boundary.list_id, now).ok_or_else(|| {
            ListStoreError::NoActiveVersion {
                id: boundary.list_id.clone(),
            }
        })?;
        evaluate(
            boundary,
            list,
            point,
            subject_country,
            subject_native_ids,
            subject_names,
            subject_has_existing_access,
            now,
        )
        .map_err(ListStoreError::Evaluation)
    }

    /// Persist the whole store as a JSON snapshot at `path`
    /// (written to a sibling temp file, then renamed over `path`,
    /// so a crash mid-write never leaves a torn snapshot).
    pub fn save(&self, path: &Path) -> Result<(), ListStoreError> {
        let bytes =
            serde_json::to_vec_pretty(self).map_err(|e| ListStoreError::Codec(e.to_string()))?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &bytes).map_err(|e| ListStoreError::Io(e.to_string()))?;
        std::fs::rename(&tmp, path).map_err(|e| ListStoreError::Io(e.to_string()))?;
        Ok(())
    }

    /// Load a store from a snapshot written by
    /// [`ListVersionStore::save`].
    pub fn load(path: &Path) -> Result<Self, ListStoreError> {
        let bytes = std::fs::read(path).map_err(|e| ListStoreError::Io(e.to_string()))?;
        serde_json::from_slice(&bytes).map_err(|e| ListStoreError::Codec(e.to_string()))
    }

    fn stored_mut(
        &mut self,
        id: &str,
        version: u64,
    ) -> Result<&mut StoredListVersion, ListStoreError> {
        let versions = self
            .lists
            .get_mut(id)
            .ok_or_else(|| ListStoreError::UnknownList { id: id.to_string() })?;
        versions
            .get_mut(&version)
            .ok_or_else(|| ListStoreError::UnknownVersion {
                id: id.to_string(),
                version,
            })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::boundary::ListVersionRef;
    use crate::sanctions::{ListSource, SanctionsEntry};

    fn supplier() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn signed_list(version: u64, effective_from: u64, entries: Vec<SanctionsEntry>) -> ListVersion {
        let mut list = ListVersion {
            id: "operator-list".to_string(),
            version,
            source: ListSource::OperatorSupplied {
                derivation: "derived from the operator's chosen published list".to_string(),
            },
            effective_from,
            effective_to: None,
            entries,
            signature: String::new(),
        };
        list.signature = sign_list(&supplier(), &list);
        list
    }

    fn country(code: &str) -> SanctionsEntry {
        SanctionsEntry::Country {
            code: code.to_string(),
        }
    }

    fn boundary() -> SanctionsBoundary {
        SanctionsBoundary::new(
            "operator-list".to_string(),
            BTreeSet::from([EnforcementPoint::AccountCreation, EnforcementPoint::Login]),
            "request review from the server operator".to_string(),
        )
        .unwrap()
    }

    #[test]
    fn lifecycle_staged_validated_activated_and_superseded() {
        // Oracle: design §D3 — the three recorded states, in
        // order, and activation closing the previous version.
        let key = supplier().verifying_key();
        let mut store = ListVersionStore::new();
        store
            .stage(signed_list(1, 100, vec![country("CU")]))
            .unwrap();
        assert_eq!(
            store.state_of("operator-list", 1),
            Some(ListVersionState::Staged)
        );
        // A staged version enforces nothing.
        assert!(store.active_version("operator-list", 150).is_none());

        let record = store.validate("operator-list", 1, &key).unwrap();
        assert_eq!(record.previous_version, None);
        assert_eq!(record.entry_count_delta, 1);
        store.activate("operator-list", 1, 100).unwrap();
        assert_eq!(
            store.active_version("operator-list", 150).unwrap().version,
            1
        );

        store
            .stage(signed_list(2, 200, vec![country("CU"), country("IR")]))
            .unwrap();
        let record = store.validate("operator-list", 2, &key).unwrap();
        assert_eq!(record.previous_version, Some(1));
        assert_eq!(record.entry_count_delta, 1);
        store.activate("operator-list", 2, 200).unwrap();

        assert_eq!(
            store.state_of("operator-list", 1),
            Some(ListVersionState::Superseded)
        );
        assert_eq!(
            store.version("operator-list", 1).unwrap().effective_to,
            Some(200)
        );
        // Before v2's window the (closed) v1 still serves; at and
        // after it, v2 does.
        assert_eq!(
            store.active_version("operator-list", 150).unwrap().version,
            1
        );
        assert_eq!(
            store.active_version("operator-list", 250).unwrap().version,
            2
        );
    }

    #[test]
    fn activation_refuses_unvalidated_and_bad_signature_stays_staged() {
        // Oracle: task 20085 acceptance — no activation of an
        // unvalidated or bad-signature version.
        let key = supplier().verifying_key();
        let mut store = ListVersionStore::new();
        store
            .stage(signed_list(1, 100, vec![country("CU")]))
            .unwrap();
        assert!(matches!(
            store.activate("operator-list", 1, 100),
            Err(ListStoreError::WrongState { .. })
        ));

        let mut forged = signed_list(2, 100, vec![country("CU")]);
        forged.signature = sign_list(&SigningKey::from_bytes(&[9u8; 32]), &forged);
        store.stage(forged).unwrap();
        assert!(matches!(
            store.validate("operator-list", 2, &key),
            Err(ListStoreError::BadSignature { .. })
        ));
        assert_eq!(
            store.state_of("operator-list", 2),
            Some(ListVersionState::Staged)
        );
        assert!(matches!(
            store.activate("operator-list", 2, 100),
            Err(ListStoreError::WrongState { .. })
        ));
    }

    #[test]
    fn staged_versions_are_immutable_and_monotone() {
        // Oracle: design §D2 immutability — a stored (id, version)
        // can never be re-staged, and versions only move forward.
        let mut store = ListVersionStore::new();
        store
            .stage(signed_list(2, 100, vec![country("CU")]))
            .unwrap();
        assert!(matches!(
            store.stage(signed_list(2, 100, vec![country("IR")])),
            Err(ListStoreError::DuplicateVersion { .. })
        ));
        assert!(matches!(
            store.stage(signed_list(1, 100, vec![country("CU")])),
            Err(ListStoreError::NonMonotoneVersion { .. })
        ));
        // The originally staged content is untouched.
        assert_eq!(
            store.version("operator-list", 2).unwrap().entries,
            vec![country("CU")]
        );
    }

    #[test]
    fn evaluate_active_names_the_list_version_and_refuses_without_one() {
        // Oracle: task 20085 acceptance — enforcement outcomes
        // carry list id + version; nothing enforces before
        // activation.
        let key = supplier().verifying_key();
        let mut store = ListVersionStore::new();
        let b = boundary();
        let ids = BTreeSet::new();
        let names = BTreeSet::new();
        assert!(matches!(
            store.evaluate_active(
                &b,
                EnforcementPoint::Login,
                Some("CU"),
                &ids,
                &names,
                false,
                150
            ),
            Err(ListStoreError::NoActiveVersion { .. })
        ));
        store
            .stage(signed_list(1, 100, vec![country("CU")]))
            .unwrap();
        store.validate("operator-list", 1, &key).unwrap();
        store.activate("operator-list", 1, 100).unwrap();
        let decision = store
            .evaluate_active(
                &b,
                EnforcementPoint::Login,
                Some("CU"),
                &ids,
                &names,
                false,
                150,
            )
            .unwrap();
        assert_eq!(
            decision.list,
            ListVersionRef {
                id: "operator-list".to_string(),
                version: 1
            }
        );
    }

    #[test]
    fn snapshot_round_trip_preserves_lifecycle() {
        // Oracle: the store is persistent — states and records
        // survive a save/load cycle verbatim.
        let key = supplier().verifying_key();
        let mut store = ListVersionStore::new();
        store
            .stage(signed_list(1, 100, vec![country("CU")]))
            .unwrap();
        store.validate("operator-list", 1, &key).unwrap();
        store.activate("operator-list", 1, 100).unwrap();
        let dir = std::env::temp_dir().join(format!("list-store-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("store.json");
        store.save(&path).unwrap();
        let loaded = ListVersionStore::load(&path).unwrap();
        assert_eq!(loaded, store);
        assert_eq!(
            loaded.active_version("operator-list", 150).unwrap().version,
            1
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
