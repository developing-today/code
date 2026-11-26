//! The program gallery registry core (#8281; design of record
//! `doc/2026-11-10T21-24-00Z_design_id_program_ecosystem/`, decision D6).
//!
//! A registry holds verified program bundles and answers four operations:
//! publish, list, fetch and unpublish. It is transport-free: the same
//! [`Registry`] backs a self-hosted reference registry, a large hosted one,
//! or a test, and a later transport (iroh, pkarr) only moves its inputs and
//! outputs.
//!
//! Validity is protocol, admission is policy (D6). Every publish must verify
//! as a bundle ([`ProgramBundle::decode`]), and the registry refuses a second
//! bundle that claims an (author, name, version) already published with a
//! different content hash: that is a provenance violation (D1, invariant 2)
//! and is never resolved silently. Which verified bundles a registry lists is
//! its operator's [`AdmissionPolicy`].
//!
//! Clients read records from any number of registries and check them with
//! [`BundleRecord::verify`] and [`provenance_conflicts`], so no registry,
//! ours included, has to be trusted for correctness.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};

use crate::program_bundle::{
    BundleError, BundleManifest, BundleRecord, ProgramBundle, ProgramIdentity,
};
use crate::world_session::{decode_hex, encode_hex};

/// Domain separator for an author's signed unpublish request.
const UNPUBLISH_DOMAIN: &[u8] = b"developing-today/id-program-registry-unpublish/v1\0";

/// File extension of a stored bundle in a registry directory.
const BUNDLE_EXTENSION: &str = "idpb";

/// File extension of a stored withdrawal (an accepted unpublish).
const WITHDRAWN_EXTENSION: &str = "withdrawn";

/// An operator's choice of which verified bundles the registry accepts.
pub trait AdmissionPolicy: Send + Sync + std::fmt::Debug {
    /// Accept or refuse a bundle that already passed protocol verification.
    ///
    /// # Errors
    ///
    /// The operator's reason for refusing, shown to the publisher.
    fn admit(&self, manifest: &BundleManifest) -> Result<(), String>;
}

/// Admit every verified bundle: the posture of an open public registry.
#[derive(Clone, Copy, Debug, Default)]
pub struct AdmitAll;

impl AdmissionPolicy for AdmitAll {
    fn admit(&self, _manifest: &BundleManifest) -> Result<(), String> {
        Ok(())
    }
}

/// Admit only bundles signed by the listed author keys (lower-case hex).
#[derive(Clone, Debug, Default)]
pub struct AdmitAuthors {
    authors: BTreeSet<String>,
}

impl AdmitAuthors {
    /// A policy admitting exactly these author keys.
    #[must_use]
    pub fn new(authors: impl IntoIterator<Item = VerifyingKey>) -> Self {
        Self {
            authors: authors
                .into_iter()
                .map(|key| encode_hex(&key.to_bytes()))
                .collect(),
        }
    }
}

impl AdmissionPolicy for AdmitAuthors {
    fn admit(&self, manifest: &BundleManifest) -> Result<(), String> {
        if self.authors.contains(&manifest.author) {
            Ok(())
        } else {
            Err("this registry only lists its operator's chosen authors".to_owned())
        }
    }
}

/// Why a registry operation was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    /// The bundle failed protocol verification; the error names the check.
    #[error(transparent)]
    Invalid(#[from] BundleError),
    /// The bundle verified, but the operator's policy does not list it.
    #[error("not admitted by this registry's policy: {0}")]
    NotAdmitted(String),
    /// Another bundle already holds this (author, name, version).
    #[error(
        "provenance violation: {} already published {} version {} with content hash {}; this bundle's is {offered}",
        existing.author, existing.name, existing.version, existing.content_hash
    )]
    ProvenanceViolation {
        /// The identity already published.
        existing: ProgramIdentity,
        /// The content hash of the refused bundle.
        offered: String,
    },
    /// The author withdrew this exact content; it cannot be republished.
    #[error("the author withdrew the bundle with content hash {0}; publish a new version instead")]
    Withdrawn(String),
    /// No bundle with this content hash is listed.
    #[error("no bundle with content hash {0} is listed")]
    NotFound(String),
    /// The unpublish request is not signed by the program's author.
    #[error("unpublish refused: the request is not signed by the program's author")]
    UnpublishSignature,
    /// Reading or writing the registry directory failed.
    #[error("registry storage: {0}")]
    Storage(String),
}

impl RegistryError {
    /// A stable code for the refusal, for wire protocols and logs.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "invalid_bundle",
            Self::NotAdmitted(_) => "not_admitted",
            Self::ProvenanceViolation { .. } => "provenance_violation",
            Self::Withdrawn(_) => "withdrawn",
            Self::NotFound(_) => "not_found",
            Self::UnpublishSignature => "unpublish_signature",
            Self::Storage(_) => "storage",
        }
    }
}

/// Which records [`Registry::list`] returns. Empty fields match everything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListQuery {
    /// Only this author's programs (lower-case hex key).
    pub author: Option<String>,
    /// Only programs with this name.
    pub name: Option<String>,
}

/// A program registry: verified bundles keyed by content hash.
///
/// [`Registry::in_memory`] keeps everything in memory; [`Registry::open_dir`]
/// also writes each accepted bundle and withdrawal to a directory and loads
/// them back, which is all a small self-hosted registry needs.
#[derive(Debug)]
pub struct Registry {
    bundles: BTreeMap<String, ProgramBundle>,
    versions: BTreeMap<(String, String, u64), String>,
    withdrawn: BTreeSet<String>,
    policy: Box<dyn AdmissionPolicy>,
    dir: Option<PathBuf>,
}

impl Registry {
    /// An empty registry that keeps nothing on disk.
    #[must_use]
    pub fn in_memory(policy: impl AdmissionPolicy + 'static) -> Self {
        Self {
            bundles: BTreeMap::new(),
            versions: BTreeMap::new(),
            withdrawn: BTreeSet::new(),
            policy: Box::new(policy),
            dir: None,
        }
    }

    /// Open (creating if needed) a registry stored in `dir`.
    ///
    /// Every stored bundle is verified again on load. A stored file that no
    /// longer verifies, or whose name is not its content hash, stops the
    /// open rather than being skipped, so a damaged registry is noticed.
    ///
    /// # Errors
    ///
    /// The directory cannot be read or created, or a stored file is invalid.
    pub fn open_dir(
        dir: impl Into<PathBuf>,
        policy: impl AdmissionPolicy + 'static,
    ) -> Result<Self, RegistryError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).map_err(|e| storage(&dir, &e))?;
        let mut registry = Self::in_memory(policy);
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map_err(|e| storage(&dir, &e))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<_, _>>()
            .map_err(|e| storage(&dir, &e))?;
        paths.sort();
        for path in paths {
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            match path.extension().and_then(|extension| extension.to_str()) {
                Some(BUNDLE_EXTENSION) => {
                    let bytes = std::fs::read(&path).map_err(|e| storage(&path, &e))?;
                    let bundle = ProgramBundle::decode(&bytes).map_err(|e| {
                        RegistryError::Storage(format!("{} does not verify: {e}", path.display()))
                    })?;
                    if bundle.manifest().content_hash != stem {
                        return Err(RegistryError::Storage(format!(
                            "{} is not named for its content hash",
                            path.display()
                        )));
                    }
                    registry.insert(bundle)?;
                }
                Some(WITHDRAWN_EXTENSION) => {
                    registry.withdrawn.insert(stem.to_owned());
                }
                _ => {}
            }
        }
        for hash in &registry.withdrawn {
            if registry.bundles.contains_key(hash) {
                return Err(RegistryError::Storage(format!(
                    "{hash} is both listed and withdrawn"
                )));
            }
        }
        registry.dir = Some(dir);
        Ok(registry)
    }

    /// Publish a bundle file.
    ///
    /// Publishing the same bytes again is accepted and changes nothing.
    ///
    /// # Errors
    ///
    /// The bundle does not verify, the policy refuses it, its content was
    /// withdrawn by its author, another bundle holds its (author, name,
    /// version), or it cannot be stored.
    pub fn publish(&mut self, bytes: &[u8]) -> Result<BundleRecord, RegistryError> {
        let bundle = ProgramBundle::decode(bytes)?;
        let hash = bundle.manifest().content_hash.clone();
        if self.withdrawn.contains(&hash) {
            return Err(RegistryError::Withdrawn(hash));
        }
        if let Some(existing) = self.bundles.get(&hash)
            && existing.identity() == bundle.identity()
        {
            return Ok(existing.record());
        }
        self.check_version_free(&bundle)?;
        self.policy
            .admit(bundle.manifest())
            .map_err(RegistryError::NotAdmitted)?;
        if let Some(dir) = &self.dir {
            write_atomically(
                &dir.join(format!("{hash}.{BUNDLE_EXTENSION}")),
                &bundle.to_bytes(),
            )?;
        }
        let record = bundle.record();
        self.insert(bundle)?;
        Ok(record)
    }

    /// The records matching `query`, ordered by author, name and version.
    #[must_use]
    pub fn list(&self, query: &ListQuery) -> Vec<BundleRecord> {
        self.list_where(query, |_| true)
    }

    /// The records matching `query` whose manifest `keep` accepts, in the
    /// same order as [`Self::list`].
    #[must_use]
    pub fn list_where(
        &self,
        query: &ListQuery,
        keep: impl Fn(&BundleManifest) -> bool,
    ) -> Vec<BundleRecord> {
        self.versions
            .iter()
            .filter(|((author, name, _), _)| {
                query.author.as_ref().is_none_or(|wanted| wanted == author)
                    && query.name.as_ref().is_none_or(|wanted| wanted == name)
            })
            .filter_map(|(_, hash)| self.bundles.get(hash))
            .filter(|bundle| keep(bundle.manifest()))
            .map(ProgramBundle::record)
            .collect()
    }

    /// The record and identity of the bundle with this content hash, if
    /// held.
    #[must_use]
    pub fn get(&self, content_hash: &str) -> Option<(BundleRecord, ProgramIdentity)> {
        self.bundles
            .get(content_hash)
            .map(|bundle| (bundle.record(), bundle.identity()))
    }

    /// The bundle file with this content hash, if listed.
    #[must_use]
    pub fn fetch(&self, content_hash: &str) -> Option<Vec<u8>> {
        self.bundles.get(content_hash).map(ProgramBundle::to_bytes)
    }

    /// Withdraw a bundle on its author's signed request (see
    /// [`sign_unpublish`]). The withdrawal is final for that exact content:
    /// the same bytes cannot be republished here, so a replayed request has
    /// nothing left to remove. The author publishes a new version instead.
    ///
    /// # Errors
    ///
    /// No such bundle is listed, the signature is not the author's, or the
    /// withdrawal cannot be stored.
    pub fn unpublish(
        &mut self,
        content_hash: &str,
        signature: &str,
    ) -> Result<BundleRecord, RegistryError> {
        let bundle = self
            .bundles
            .get(content_hash)
            .ok_or_else(|| RegistryError::NotFound(content_hash.to_owned()))?;
        let author = decode_hex(&bundle.manifest().author)
            .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
            .and_then(|bytes| VerifyingKey::from_bytes(&bytes).ok())
            .ok_or(RegistryError::UnpublishSignature)?;
        let signature = decode_hex(signature)
            .and_then(|bytes| <[u8; 64]>::try_from(bytes).ok())
            .map(|bytes| Signature::from_bytes(&bytes))
            .ok_or(RegistryError::UnpublishSignature)?;
        author
            .verify_strict(&unpublish_message(content_hash), &signature)
            .map_err(|_| RegistryError::UnpublishSignature)?;
        if let Some(dir) = &self.dir {
            write_atomically(
                &dir.join(format!("{content_hash}.{WITHDRAWN_EXTENSION}")),
                encode_hex(&signature.to_bytes()).as_bytes(),
            )?;
        }
        let record = self.drop_listing(content_hash)?;
        self.withdrawn.insert(content_hash.to_owned());
        Ok(record)
    }

    /// Delist a bundle on the operator's own authority (admission is
    /// policy). Unlike [`Self::unpublish`], the author may publish the same
    /// content again.
    ///
    /// # Errors
    ///
    /// No such bundle is listed, or the stored file cannot be removed.
    pub fn remove(&mut self, content_hash: &str) -> Result<BundleRecord, RegistryError> {
        if !self.bundles.contains_key(content_hash) {
            return Err(RegistryError::NotFound(content_hash.to_owned()));
        }
        self.drop_listing(content_hash)
    }

    /// How many bundles are listed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bundles.len()
    }

    /// Whether no bundle is listed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bundles.is_empty()
    }

    fn check_version_free(&self, bundle: &ProgramBundle) -> Result<(), RegistryError> {
        let identity = bundle.identity();
        let key = (identity.author, identity.name, identity.version);
        if let Some(existing) = self.versions.get(&key).and_then(|h| self.bundles.get(h)) {
            return Err(RegistryError::ProvenanceViolation {
                existing: existing.identity(),
                offered: identity.content_hash,
            });
        }
        Ok(())
    }

    fn insert(&mut self, bundle: ProgramBundle) -> Result<(), RegistryError> {
        self.check_version_free(&bundle)?;
        let identity = bundle.identity();
        let hash = identity.content_hash.clone();
        if self.bundles.contains_key(&hash) {
            // The same payload under a second identity (a fork that kept the
            // parent's bytes, say) shares the content-addressed file name.
            return Err(RegistryError::Storage(format!(
                "content hash {hash} is already listed under another identity"
            )));
        }
        self.versions.insert(
            (identity.author, identity.name, identity.version),
            hash.clone(),
        );
        self.bundles.insert(hash, bundle);
        Ok(())
    }

    fn drop_listing(&mut self, content_hash: &str) -> Result<BundleRecord, RegistryError> {
        if let Some(dir) = &self.dir {
            let path = dir.join(format!("{content_hash}.{BUNDLE_EXTENSION}"));
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(storage(&path, &e)),
            }
        }
        let bundle = self
            .bundles
            .remove(content_hash)
            .ok_or_else(|| RegistryError::NotFound(content_hash.to_owned()))?;
        let identity = bundle.identity();
        self.versions
            .remove(&(identity.author, identity.name, identity.version));
        Ok(bundle.record())
    }
}

/// The message an author signs to withdraw the bundle with `content_hash`.
#[must_use]
pub fn unpublish_message(content_hash: &str) -> Vec<u8> {
    let mut message = UNPUBLISH_DOMAIN.to_vec();
    message.extend_from_slice(content_hash.as_bytes());
    message
}

/// Sign a request to withdraw the bundle with `content_hash`, as its author.
#[must_use]
pub fn sign_unpublish(content_hash: &str, author: &SigningKey) -> String {
    encode_hex(&author.sign(&unpublish_message(content_hash)).to_bytes())
}

/// Two verified records that claim the same (author, name, version) with
/// different content hashes (D1: a provenance violation to surface).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvenanceConflict {
    /// One claimed identity.
    pub first: ProgramIdentity,
    /// The other, with a different content hash.
    pub second: ProgramIdentity,
}

/// Find provenance violations among records gathered from any registries.
///
/// Records that do not verify are ignored here; they carry no author claim.
/// Each conflicting pair is reported once, in the order the records came.
#[must_use]
pub fn provenance_conflicts(records: &[BundleRecord]) -> Vec<ProvenanceConflict> {
    let mut seen: BTreeMap<(String, String, u64), Vec<ProgramIdentity>> = BTreeMap::new();
    let mut conflicts = Vec::new();
    for manifest in records.iter().filter_map(|record| record.verify().ok()) {
        let identity = manifest.identity();
        let claims = seen
            .entry((
                identity.author.clone(),
                identity.name.clone(),
                identity.version,
            ))
            .or_default();
        if claims.contains(&identity) {
            continue;
        }
        for earlier in claims.iter() {
            conflicts.push(ProvenanceConflict {
                first: earlier.clone(),
                second: identity.clone(),
            });
        }
        claims.push(identity);
    }
    conflicts
}

fn storage(path: &Path, error: &std::io::Error) -> RegistryError {
    RegistryError::Storage(format!("{}: {error}", path.display()))
}

/// Write `bytes` to `path` through a sibling temporary file and a rename, so
/// a crash never leaves a half-written bundle under its final name.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), RegistryError> {
    let dir = path
        .parent()
        .ok_or_else(|| RegistryError::Storage(format!("{} has no parent", path.display())))?;
    let mut staged = tempfile::NamedTempFile::new_in(dir).map_err(|e| storage(dir, &e))?;
    std::io::Write::write_all(&mut staged, bytes).map_err(|e| storage(path, &e))?;
    staged.as_file().sync_all().map_err(|e| storage(path, &e))?;
    staged.persist(path).map_err(|e| storage(path, &e.error))?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::program_bundle::{ProgramSpec, export, fork};

    fn alice() -> SigningKey {
        SigningKey::from_bytes(&[1; 32])
    }

    fn bob() -> SigningKey {
        SigningKey::from_bytes(&[2; 32])
    }

    fn bundle(payload: &[u8], name: &str, version: u64, author: &SigningKey) -> ProgramBundle {
        export(
            payload.to_vec(),
            ProgramSpec {
                name,
                version,
                licence: None,
                description: None,
                categories: &[],
            },
            author,
        )
        .unwrap()
    }

    #[test]
    fn publish_list_fetch_round_trip_and_the_fetched_bundle_verifies() {
        let mut registry = Registry::in_memory(AdmitAll);
        let chess = bundle(b"chess v1", "chess", 1, &alice());
        let go = bundle(b"go v1", "go", 1, &bob());
        let record = registry.publish(&chess.to_bytes()).unwrap();
        assert!(record.describes(&chess));
        registry.publish(&go.to_bytes()).unwrap();
        assert_eq!(
            registry.publish(&chess.to_bytes()).unwrap(),
            record,
            "republishing the same bytes is idempotent"
        );
        assert_eq!(registry.len(), 2);

        let all = registry.list(&ListQuery::default());
        assert_eq!(all.len(), 2);
        let by_bob = registry.list(&ListQuery {
            author: Some(go.manifest().author.clone()),
            name: None,
        });
        assert_eq!(by_bob, vec![go.record()]);
        let named = registry.list(&ListQuery {
            author: None,
            name: Some("chess".to_owned()),
        });
        assert_eq!(named, vec![chess.record()]);

        let fetched = registry.fetch(&chess.manifest().content_hash).unwrap();
        let verified = ProgramBundle::decode(&fetched).unwrap();
        assert!(record.describes(&verified));
        assert_eq!(registry.fetch(&"0".repeat(64)), None);
    }

    #[test]
    fn invalid_bundles_are_refused_as_protocol_not_policy() {
        let mut registry = Registry::in_memory(AdmitAll);
        let mut bytes = bundle(b"chess", "chess", 1, &alice()).to_bytes();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        let error = registry.publish(&bytes).unwrap_err();
        assert_eq!(error.code(), "invalid_bundle");
        assert!(error.to_string().contains("content_hash"), "{error}");
        assert!(registry.is_empty());
    }

    #[test]
    fn the_operator_policy_decides_admission() {
        let mut registry = Registry::in_memory(AdmitAuthors::new([alice().verifying_key()]));
        registry
            .publish(&bundle(b"chess", "chess", 1, &alice()).to_bytes())
            .unwrap();
        let error = registry
            .publish(&bundle(b"go", "go", 1, &bob()).to_bytes())
            .unwrap_err();
        assert_eq!(error.code(), "not_admitted");
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn a_second_hash_for_the_same_version_is_a_provenance_violation() {
        let mut registry = Registry::in_memory(AdmitAll);
        let first = bundle(b"chess", "chess", 1, &alice());
        let second = bundle(b"chess, edited", "chess", 1, &alice());
        registry.publish(&first.to_bytes()).unwrap();
        let error = registry.publish(&second.to_bytes()).unwrap_err();
        assert_eq!(
            error,
            RegistryError::ProvenanceViolation {
                existing: first.identity(),
                offered: second.manifest().content_hash.clone(),
            }
        );
        registry
            .publish(&bundle(b"chess, edited", "chess", 2, &alice()).to_bytes())
            .expect("a new version is fine");
        registry
            .publish(&bundle(b"chess by bob", "chess", 1, &bob()).to_bytes())
            .expect("names are per author");
    }

    #[test]
    fn only_the_author_can_unpublish_and_withdrawal_is_final_for_that_content() {
        let mut registry = Registry::in_memory(AdmitAll);
        let chess = bundle(b"chess", "chess", 1, &alice());
        let hash = chess.manifest().content_hash.clone();
        registry.publish(&chess.to_bytes()).unwrap();

        let forged = sign_unpublish(&hash, &bob());
        assert_eq!(
            registry.unpublish(&hash, &forged).unwrap_err(),
            RegistryError::UnpublishSignature
        );
        let other = sign_unpublish(&"0".repeat(64), &alice());
        assert_eq!(
            registry.unpublish(&hash, &other).unwrap_err(),
            RegistryError::UnpublishSignature,
            "a signature for another hash does not transfer"
        );

        let signature = sign_unpublish(&hash, &alice());
        assert!(
            registry
                .unpublish(&hash, &signature)
                .unwrap()
                .describes(&chess)
        );
        assert!(registry.is_empty());
        assert_eq!(
            registry.publish(&chess.to_bytes()).unwrap_err(),
            RegistryError::Withdrawn(hash.clone())
        );
        assert_eq!(
            registry.unpublish(&hash, &signature).unwrap_err(),
            RegistryError::NotFound(hash)
        );
    }

    #[test]
    fn an_operator_removal_is_not_a_withdrawal() {
        let mut registry = Registry::in_memory(AdmitAll);
        let chess = bundle(b"chess", "chess", 1, &alice());
        registry.publish(&chess.to_bytes()).unwrap();
        registry.remove(&chess.manifest().content_hash).unwrap();
        assert_eq!(registry.list(&ListQuery::default()), [] as [BundleRecord; 0]);
        registry
            .publish(&chess.to_bytes())
            .expect("the author may publish again after an operator removal");
    }

    #[test]
    fn a_directory_registry_survives_reopen_and_keeps_withdrawals() {
        let dir = tempfile::tempdir().unwrap();
        let chess = bundle(b"chess", "chess", 1, &alice());
        let go = bundle(b"go", "go", 1, &bob());
        {
            let mut registry = Registry::open_dir(dir.path(), AdmitAll).unwrap();
            registry.publish(&chess.to_bytes()).unwrap();
            registry.publish(&go.to_bytes()).unwrap();
            let hash = go.manifest().content_hash.clone();
            registry
                .unpublish(&hash, &sign_unpublish(&hash, &bob()))
                .unwrap();
        }
        let mut reopened = Registry::open_dir(dir.path(), AdmitAll).unwrap();
        assert_eq!(reopened.list(&ListQuery::default()), vec![chess.record()]);
        assert_eq!(
            reopened.publish(&go.to_bytes()).unwrap_err().code(),
            "withdrawn"
        );

        let stray = dir
            .path()
            .join(format!("{}.{BUNDLE_EXTENSION}", "0".repeat(64)));
        std::fs::write(&stray, chess.to_bytes()).unwrap();
        let error = Registry::open_dir(dir.path(), AdmitAll).unwrap_err();
        assert_eq!(error.code(), "storage", "{error}");
    }

    #[test]
    fn clients_see_provenance_conflicts_across_registries() {
        let first = bundle(b"chess", "chess", 1, &alice());
        let second = bundle(b"chess, edited", "chess", 1, &alice());
        let unrelated = bundle(b"go", "go", 1, &alice());
        let mut here = Registry::in_memory(AdmitAll);
        let mut there = Registry::in_memory(AdmitAll);
        here.publish(&first.to_bytes()).unwrap();
        here.publish(&unrelated.to_bytes()).unwrap();
        there.publish(&second.to_bytes()).unwrap();
        there.publish(&first.to_bytes()).unwrap_err();

        let mut gathered = here.list(&ListQuery::default());
        gathered.extend(there.list(&ListQuery::default()));
        gathered.push(first.record());
        assert_eq!(
            provenance_conflicts(&gathered),
            vec![ProvenanceConflict {
                first: first.identity(),
                second: second.identity(),
            }]
        );

        let forged = BundleRecord {
            manifest: second
                .record()
                .manifest
                .replacen(r#""name":"chess""#, r#""name":"go""#, 1),
            ..second.record()
        };
        assert!(
            provenance_conflicts(&[unrelated.record(), forged]).is_empty(),
            "an unverifiable record makes no claim"
        );
    }

    #[test]
    fn a_fork_lists_with_its_lineage() {
        let mut registry = Registry::in_memory(AdmitAll);
        let parent = bundle(b"chess", "chess", 1, &alice());
        let child = fork(
            &parent,
            b"chess, bob's rules".to_vec(),
            ProgramSpec {
                name: "chess",
                version: 1,
                licence: None,
                description: None,
                categories: &[],
            },
            &bob(),
        )
        .unwrap();
        registry.publish(&child.to_bytes()).unwrap();
        let listed = registry.list(&ListQuery::default());
        assert_eq!(listed[0].verify().unwrap().lineage, vec![parent.identity()]);
    }
}
