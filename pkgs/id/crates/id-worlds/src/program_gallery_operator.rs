//! The gallery's operator layer (#10004; v3 design
//! `doc/2026-11-14T13-33-00Z_design_id_program_ecosystem_v3/`, G2, G4, G5).
//!
//! Everything here is the operator's speech, never the author's, and is
//! kept apart from signed records so no surface can confuse the two:
//!
//! - [`SignedCollection`]: a featured collection, an ordered list of
//!   program identities signed by the operator's own key and attributed to
//!   it (G4). Any host can publish collections the same way, and a
//!   collection never changes the records it lists.
//! - [`OperatorOverlay`]: the operator's own tags for programs and authors
//!   (G2), stored and served as operator data beside, never inside, the
//!   author-signed categories.
//! - [`AuthorPage`]: every record one author key published, addressed by
//!   that key alone (G5), with the operator's tags for the author shown as
//!   operator data.

use std::collections::BTreeMap;
use std::path::Path;

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::program_bundle::{
    BundleRecord, ProgramIdentity, check_identity, is_portable_category, parse_hex,
};
use crate::world_session::encode_hex;

/// Domain separator for an operator's collection signature.
const COLLECTION_DOMAIN: &[u8] = b"developing-today/id-gallery-collection/v1\0";

/// Most programs one collection may list.
pub const MAX_COLLECTION_ITEMS: usize = 500;

/// Longest collection title, in bytes.
pub const MAX_TITLE_BYTES: usize = 200;

/// Most operator tags on one program or author.
pub const MAX_OPERATOR_TAGS: usize = 16;

/// Largest signed collection document accepted, in bytes.
pub const MAX_COLLECTION_BYTES: usize = 256 * 1024;

/// A featured collection, as the operator states it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Collection {
    /// The operator's Ed25519 public key, lower-case hex: whose voice this is.
    pub operator: String,
    /// The collection's address under the operator, in the portable
    /// category form (lower-case ASCII letters, digits and `-`).
    pub name: String,
    /// The title shown with the collection.
    pub title: String,
    /// The operator's monotonic revision of this collection.
    pub revision: u64,
    /// The featured programs, in the operator's order.
    pub items: Vec<ProgramIdentity>,
}

/// Why a collection, overlay or author key was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OperatorError {
    /// The collection document is malformed or out of bounds.
    #[error("collection: {0}")]
    Collection(String),
    /// The collection is not signed by the operator it names.
    #[error("collection signature: not signed by the operator it names")]
    Signature,
    /// An operator tag is not in the portable form, or there are too many.
    #[error("overlay: {0}")]
    Overlay(String),
    /// An author key is not 32 bytes of lower-case hex.
    #[error("author key is not 32 bytes of lower-case hex")]
    AuthorKey,
    /// Reading or writing operator data failed.
    #[error("operator storage: {0}")]
    Storage(String),
}

/// A collection with the operator's signature over its exact JSON.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedCollection {
    /// The collection's exact JSON, as signed.
    pub collection: String,
    /// The operator's Ed25519 signature, lower-case hex.
    pub signature: String,
}

impl SignedCollection {
    /// Sign `items` as the operator's collection `name`.
    ///
    /// # Errors
    ///
    /// The name, title or items are out of bounds.
    pub fn sign(
        name: &str,
        title: &str,
        revision: u64,
        items: Vec<ProgramIdentity>,
        operator: &SigningKey,
    ) -> Result<Self, OperatorError> {
        let collection = Collection {
            operator: encode_hex(&operator.verifying_key().to_bytes()),
            name: name.to_owned(),
            title: title.to_owned(),
            revision,
            items,
        };
        check_collection(&collection)?;
        let collection = serde_json::to_string(&collection)
            .map_err(|e| OperatorError::Collection(e.to_string()))?;
        let signature = encode_hex(&operator.sign(&collection_message(&collection)).to_bytes());
        Ok(Self {
            collection,
            signature,
        })
    }

    /// The collection, once its signature and bounds verify.
    ///
    /// # Errors
    ///
    /// The document is malformed, out of bounds, or not signed by the
    /// operator it names.
    pub fn verify(&self) -> Result<Collection, OperatorError> {
        if self.collection.len() > MAX_COLLECTION_BYTES {
            return Err(OperatorError::Collection(format!(
                "the document is {} bytes; the limit is {MAX_COLLECTION_BYTES}",
                self.collection.len()
            )));
        }
        let collection: Collection =
            id_core::json_unique::from_slice_without_duplicate_keys(self.collection.as_bytes())
                .map_err(|e| OperatorError::Collection(e.to_string()))?;
        check_collection(&collection)?;
        let operator = parse_hex::<32>(&collection.operator)
            .and_then(|bytes| VerifyingKey::from_bytes(&bytes).ok())
            .ok_or(OperatorError::Signature)?;
        let signature = parse_hex::<64>(&self.signature)
            .map(|bytes| Signature::from_bytes(&bytes))
            .ok_or(OperatorError::Signature)?;
        operator
            .verify_strict(&collection_message(&self.collection), &signature)
            .map_err(|_| OperatorError::Signature)?;
        Ok(collection)
    }
}

fn collection_message(collection: &str) -> Vec<u8> {
    let mut message = COLLECTION_DOMAIN.to_vec();
    message.extend_from_slice(blake3::hash(collection.as_bytes()).as_bytes());
    message
}

fn check_collection(collection: &Collection) -> Result<(), OperatorError> {
    if !is_portable_category(&collection.name) {
        return Err(OperatorError::Collection(
            "the name must be lower-case ASCII letters, digits or '-'".to_owned(),
        ));
    }
    if collection.title.len() > MAX_TITLE_BYTES || collection.title.chars().any(char::is_control) {
        return Err(OperatorError::Collection(format!(
            "the title must be at most {MAX_TITLE_BYTES} bytes with no control characters"
        )));
    }
    if collection.items.len() > MAX_COLLECTION_ITEMS {
        return Err(OperatorError::Collection(format!(
            "{} items; the limit is {MAX_COLLECTION_ITEMS}",
            collection.items.len()
        )));
    }
    for (index, item) in collection.items.iter().enumerate() {
        check_identity(item)
            .map_err(|reason| OperatorError::Collection(format!("item {index}: {reason}")))?;
    }
    Ok(())
}

/// The operator's own tags, kept apart from author-signed categories (G2).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorOverlay {
    /// Tags per program, keyed by content hash.
    #[serde(default)]
    pub programs: BTreeMap<String, Vec<String>>,
    /// Tags per author, keyed by author key.
    #[serde(default)]
    pub authors: BTreeMap<String, Vec<String>>,
}

impl OperatorOverlay {
    /// Check every key and tag.
    ///
    /// # Errors
    ///
    /// A key is not 32 bytes of lower-case hex, a tag is not in the portable
    /// form, or one entry has too many tags.
    pub fn check(&self) -> Result<(), OperatorError> {
        for (key, tags) in self.programs.iter().chain(&self.authors) {
            if parse_hex::<32>(key).is_none() {
                return Err(OperatorError::Overlay(format!(
                    "{key:?} is not 32 bytes of lower-case hex"
                )));
            }
            if tags.len() > MAX_OPERATOR_TAGS {
                return Err(OperatorError::Overlay(format!(
                    "{key} has {} tags; the limit is {MAX_OPERATOR_TAGS}",
                    tags.len()
                )));
            }
            if let Some(tag) = tags.iter().find(|tag| !is_portable_category(tag)) {
                return Err(OperatorError::Overlay(format!(
                    "tag {tag:?} must be lower-case ASCII letters, digits or '-'"
                )));
            }
        }
        Ok(())
    }

    /// The operator's tags for the programs among `records`, by content
    /// hash. Records with none are left out.
    #[must_use]
    pub fn tags_for(&self, records: &[BundleRecord]) -> BTreeMap<String, Vec<String>> {
        records
            .iter()
            .filter_map(|record| record.verify().ok())
            .filter_map(|manifest| {
                self.programs
                    .get(&manifest.content_hash)
                    .map(|tags| (manifest.content_hash, tags.clone()))
            })
            .collect()
    }
}

/// One author's page, addressed by key alone (G5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorPage {
    /// The author's key: the page's canonical address.
    pub author: String,
    /// Every record the key published here, by name and version.
    pub records: Vec<BundleRecord>,
    /// The operator's tags for this author: operator data, not the author's.
    pub operator_tags: Vec<String>,
}

/// Check that `author` is an author key's canonical form.
///
/// # Errors
///
/// It is not 32 bytes of lower-case hex.
pub fn check_author_key(author: &str) -> Result<(), OperatorError> {
    parse_hex::<32>(author)
        .map(|_| ())
        .ok_or(OperatorError::AuthorKey)
}

/// The operator data stored beside a gallery's bundles.
#[derive(Clone, Debug, Default)]
pub struct OperatorData {
    /// Verified collections, keyed by (operator key, name).
    pub collections: BTreeMap<(String, String), SignedCollection>,
    /// The operator's tags.
    pub overlay: OperatorOverlay,
}

/// Directory, under a gallery directory, holding operator data.
pub const OPERATOR_DIR: &str = "operator";
const COLLECTIONS_DIR: &str = "collections";
const OVERLAY_FILE: &str = "overlay.json";

impl OperatorData {
    /// Load operator data from `gallery_dir/operator`, if present. Every
    /// collection is verified; one that does not verify stops the load.
    ///
    /// # Errors
    ///
    /// A file cannot be read or does not verify.
    pub fn load(gallery_dir: &Path) -> Result<Self, OperatorError> {
        let root = gallery_dir.join(OPERATOR_DIR);
        let mut data = Self::default();
        let overlay = root.join(OVERLAY_FILE);
        if overlay.is_file() {
            let bytes = std::fs::read(&overlay).map_err(|e| storage(&overlay, &e))?;
            data.overlay = id_core::json_unique::from_slice_without_duplicate_keys(&bytes)
                .map_err(|e| OperatorError::Overlay(format!("{}: {e}", overlay.display())))?;
            data.overlay.check()?;
        }
        let collections = root.join(COLLECTIONS_DIR);
        if collections.is_dir() {
            let mut paths: Vec<_> = std::fs::read_dir(&collections)
                .map_err(|e| storage(&collections, &e))?
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<Result<_, _>>()
                .map_err(|e| storage(&collections, &e))?;
            paths.sort();
            for path in paths {
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let bytes = std::fs::read(&path).map_err(|e| storage(&path, &e))?;
                let signed: SignedCollection =
                    id_core::json_unique::from_slice_without_duplicate_keys(&bytes).map_err(
                        |e| OperatorError::Collection(format!("{}: {e}", path.display())),
                    )?;
                data.insert_collection(signed)?;
            }
        }
        Ok(data)
    }

    /// Add or replace a verified collection, keeping the higher revision.
    /// Returns whether it was kept.
    ///
    /// # Errors
    ///
    /// The collection does not verify.
    pub fn insert_collection(&mut self, signed: SignedCollection) -> Result<bool, OperatorError> {
        let collection = signed.verify()?;
        let key = (collection.operator, collection.name);
        if let Some(existing) = self.collections.get(&key)
            && existing.verify()?.revision >= collection.revision
        {
            return Ok(false);
        }
        self.collections.insert(key, signed);
        Ok(true)
    }

    /// Write a verified collection under `gallery_dir/operator/collections`.
    ///
    /// # Errors
    ///
    /// The collection does not verify, or the file cannot be written.
    pub fn save_collection(
        gallery_dir: &Path,
        signed: &SignedCollection,
    ) -> Result<(), OperatorError> {
        let collection = signed.verify()?;
        let dir = gallery_dir.join(OPERATOR_DIR).join(COLLECTIONS_DIR);
        std::fs::create_dir_all(&dir).map_err(|e| storage(&dir, &e))?;
        let path = dir.join(format!(
            "{}-{}.json",
            &collection.operator[..16],
            collection.name
        ));
        let bytes =
            serde_json::to_vec_pretty(signed).map_err(|e| OperatorError::Storage(e.to_string()))?;
        std::fs::write(&path, bytes).map_err(|e| storage(&path, &e))
    }

    /// Write the operator overlay to `gallery_dir/operator/overlay.json`.
    ///
    /// # Errors
    ///
    /// The overlay does not check, or the file cannot be written.
    pub fn save_overlay(
        gallery_dir: &Path,
        overlay: &OperatorOverlay,
    ) -> Result<(), OperatorError> {
        overlay.check()?;
        let dir = gallery_dir.join(OPERATOR_DIR);
        std::fs::create_dir_all(&dir).map_err(|e| storage(&dir, &e))?;
        let path = dir.join(OVERLAY_FILE);
        let bytes = serde_json::to_vec_pretty(overlay)
            .map_err(|e| OperatorError::Storage(e.to_string()))?;
        std::fs::write(&path, bytes).map_err(|e| storage(&path, &e))
    }
}

fn storage(path: &Path, error: &std::io::Error) -> OperatorError {
    OperatorError::Storage(format!("{}: {error}", path.display()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::program_bundle::{ProgramSpec, export};

    fn operator() -> SigningKey {
        SigningKey::from_bytes(&[9; 32])
    }

    fn identity(name: &str) -> ProgramIdentity {
        export(
            name.as_bytes().to_vec(),
            ProgramSpec {
                name,
                version: 1,
                licence: None,
                description: None,
                categories: &[],
            },
            &SigningKey::from_bytes(&[1; 32]),
        )
        .unwrap()
        .identity()
    }

    #[test]
    fn a_collection_verifies_under_its_operator_and_refuses_edits() {
        let items = vec![identity("chess"), identity("go")];
        let signed =
            SignedCollection::sign("staff-picks", "Staff picks", 1, items.clone(), &operator())
                .unwrap();
        let collection = signed.verify().unwrap();
        assert_eq!(
            collection.operator,
            encode_hex(&operator().verifying_key().to_bytes())
        );
        assert_eq!(collection.items, items);

        let reordered = SignedCollection {
            collection: signed.collection.replacen("chess", "chesz", 1),
            ..signed.clone()
        };
        assert!(reordered.verify().is_err());

        let other = SigningKey::from_bytes(&[8; 32]);
        let forged = SignedCollection {
            signature: encode_hex(
                &other
                    .sign(&collection_message(&signed.collection))
                    .to_bytes(),
            ),
            ..signed
        };
        assert_eq!(forged.verify().unwrap_err(), OperatorError::Signature);
    }

    #[test]
    fn collection_bounds_are_refused_before_signing() {
        let sign = |name: &str, title: &str| {
            SignedCollection::sign(name, title, 1, Vec::new(), &operator())
        };
        assert!(sign("Staff Picks", "x").is_err());
        assert!(sign("picks", &"t".repeat(MAX_TITLE_BYTES + 1)).is_err());
        assert!(sign("picks", "a\nb").is_err());
        let mut bad = identity("chess");
        bad.content_hash = "zz".to_owned();
        assert!(SignedCollection::sign("picks", "t", 1, vec![bad], &operator()).is_err());
    }

    #[test]
    fn overlay_tags_must_be_portable_and_keyed_by_hex() {
        let hash = identity("chess").content_hash;
        let mut overlay = OperatorOverlay::default();
        overlay
            .programs
            .insert(hash.clone(), vec!["miscategorised".to_owned()]);
        assert!(overlay.check().is_ok());
        overlay
            .programs
            .insert(hash, vec!["Not Portable".to_owned()]);
        assert!(overlay.check().is_err());
        let mut overlay = OperatorOverlay::default();
        overlay
            .authors
            .insert("not-a-key".to_owned(), vec!["verified".to_owned()]);
        assert!(overlay.check().is_err());
    }

    #[test]
    fn operator_data_round_trips_and_keeps_the_newest_revision() {
        let dir = tempfile::tempdir().unwrap();
        let first =
            SignedCollection::sign("picks", "Picks", 1, vec![identity("chess")], &operator())
                .unwrap();
        let second =
            SignedCollection::sign("picks", "Picks", 2, vec![identity("go")], &operator()).unwrap();
        OperatorData::save_collection(dir.path(), &second).unwrap();
        let mut overlay = OperatorOverlay::default();
        overlay.authors.insert(
            encode_hex(&operator().verifying_key().to_bytes()),
            vec!["staff".to_owned()],
        );
        OperatorData::save_overlay(dir.path(), &overlay).unwrap();

        let mut loaded = OperatorData::load(dir.path()).unwrap();
        assert_eq!(loaded.overlay, overlay);
        assert_eq!(loaded.collections.len(), 1);
        assert!(
            !loaded.insert_collection(first).unwrap(),
            "an older revision is not kept"
        );
        let kept = loaded
            .collections
            .values()
            .next()
            .unwrap()
            .verify()
            .unwrap();
        assert_eq!(kept.revision, 2);

        let path = dir
            .path()
            .join(OPERATOR_DIR)
            .join(COLLECTIONS_DIR)
            .join("forged.json");
        let forged = SignedCollection {
            signature: "00".repeat(64),
            ..second
        };
        std::fs::write(&path, serde_json::to_vec(&forged).unwrap()).unwrap();
        assert_eq!(
            OperatorData::load(dir.path()).unwrap_err(),
            OperatorError::Signature
        );
    }

    #[test]
    fn author_keys_are_checked() {
        assert!(check_author_key(&"ab".repeat(32)).is_ok());
        assert!(check_author_key(&"AB".repeat(32)).is_err());
        assert!(check_author_key("chess").is_err());
    }
}
