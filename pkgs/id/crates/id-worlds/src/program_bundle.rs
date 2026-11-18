//! Signed program bundles: one file that carries a program anywhere
//! (#8280; design of record `doc/2026-11-10T21-24-00Z_design_id_program_ecosystem/`,
//! decisions D1-D3).
//!
//! A bundle holds the payload (the program), a manifest stating the D1
//! identity (author key, name, version, content hash), the F12.7 ABI
//! descriptor, fork lineage, and two Ed25519 signatures by the author:
//!
//! - the **bundle signature** over the exact manifest bytes and the payload,
//!   so neither can change without breaking provenance (D2, D3: lineage is
//!   structural, not a removable courtesy field);
//! - the **module signature**, the F12.17 world-module signature over the
//!   payload ([`module_signing_message`]), so a sideloaded bundle installs
//!   into a world that keeps a publisher trust list without a second signing
//!   step.
//!
//! Everything here is a pure function over bytes: export, verify and fork
//! need no server, registry or network (invariant 1, "sideloading always
//! works"). `WorldService::install_bundle` (under the `sandbox` feature)
//! sideloads a verified bundle into a world and records its provenance.
//!
//! # File layout
//!
//! ```text
//! BUNDLE_MAGIC
//! u32 little-endian manifest length | manifest JSON
//! u32 little-endian seal length     | seal JSON
//! payload bytes (the rest of the file)
//! ```
//!
//! The manifest is signed as the exact bytes in the file, so verification
//! never depends on re-serialising it. Both JSON segments refuse duplicated
//! keys (#8794) and unknown fields.

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::world_session::{
    MAX_WORLD_MODULE_BYTES, ModuleSignature, decode_hex, encode_hex, module_hash,
    module_signing_message,
};

/// The first bytes of every bundle file.
pub const BUNDLE_MAGIC: &[u8] = b"developing-today/id-program-bundle/v1\n";

/// Domain separator for the bundle signature. The signed message is this
/// prefix, the BLAKE3 digest of the exact manifest bytes, then the BLAKE3
/// digest of the payload.
const BUNDLE_SIGNING_DOMAIN: &[u8] = b"developing-today/id-program-bundle/v1\0";

/// Largest manifest segment accepted, in bytes.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;

/// Largest seal segment accepted, in bytes. Two hex signatures and their
/// field names fit in far less.
const MAX_SEAL_BYTES: usize = 1024;

/// Most ancestors a lineage may name. A fork of a fork carries the whole
/// chain (D3), so this bounds the manifest, not how often a program may be
/// forked in practice.
pub const MAX_LINEAGE_DEPTH: usize = 256;

/// Longest program name, in bytes.
pub const MAX_NAME_BYTES: usize = 128;

/// Longest licence statement, in bytes.
pub const MAX_LICENCE_BYTES: usize = 4096;

/// Longest discovery description, in bytes.
pub const MAX_DESCRIPTION_BYTES: usize = 4096;

/// Most categories one manifest may assert.
pub const MAX_CATEGORIES: usize = 8;

/// Longest category, in bytes. A category is lower-case ASCII letters,
/// digits and `-`, so it reads the same in every registry's taxonomy.
pub const MAX_CATEGORY_BYTES: usize = 64;

/// The guest ABI family programs target today (F12.7, `examples/roc-world/ABI.md`).
pub const PLAZA_ABI_FAMILY: &str = "plaza";

/// The guest ABI version programs target today; `plaza_abi_version` returns it.
pub const PLAZA_ABI_VERSION: u32 = 1;

/// A program's portable identity (D1): author key, per-author name, the
/// author's version claim, and the content hash that is the fact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramIdentity {
    /// The author's Ed25519 public key, lower-case hex.
    pub author: String,
    /// The program's name, scoped to its author.
    pub name: String,
    /// The author's monotonic version for this name.
    pub version: u64,
    /// BLAKE3 of the payload, lower-case hex (the world install hash).
    pub content_hash: String,
}

/// The F12.7 guest ABI a payload targets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbiDescriptor {
    /// The ABI family, `plaza` today.
    pub family: String,
    /// The ABI version within the family.
    pub version: u32,
}

impl AbiDescriptor {
    /// The descriptor for the ABI every current world hosts.
    #[must_use]
    pub fn plaza_v1() -> Self {
        Self {
            family: PLAZA_ABI_FAMILY.to_owned(),
            version: PLAZA_ABI_VERSION,
        }
    }
}

/// The signed statement a bundle makes about its payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleManifest {
    /// The author's Ed25519 public key, lower-case hex.
    pub author: String,
    /// The program's name, scoped to its author.
    pub name: String,
    /// The author's monotonic version for this name.
    pub version: u64,
    /// BLAKE3 of the payload, lower-case hex.
    pub content_hash: String,
    /// The guest ABI the payload targets.
    pub abi: AbiDescriptor,
    /// Every ancestor this program was forked from, oldest first; empty for
    /// an original. The last entry is the direct parent.
    pub lineage: Vec<ProgramIdentity>,
    /// The author's licence statement, recorded but not adjudicated (D3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub licence: Option<String>,
    /// Discovery metadata: the author's description, signed with the
    /// manifest, so a registry may index it but never alter it (v2 §1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Discovery metadata: the author's categories, signed with the
    /// manifest. An operator's own tags are a separate overlay (v3 G2).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
}

impl BundleManifest {
    /// This program's own D1 identity.
    #[must_use]
    pub fn identity(&self) -> ProgramIdentity {
        ProgramIdentity {
            author: self.author.clone(),
            name: self.name.clone(),
            version: self.version,
            content_hash: self.content_hash.clone(),
        }
    }

    /// The direct parent, if this program is a fork.
    #[must_use]
    pub fn parent(&self) -> Option<&ProgramIdentity> {
        self.lineage.last()
    }
}

/// The two signatures a bundle carries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Seal {
    bundle_signature: String,
    module_signature: String,
}

/// What the author states when exporting or forking a program.
#[derive(Clone, Copy, Debug)]
pub struct ProgramSpec<'a> {
    /// The program's name, scoped to the signing author.
    pub name: &'a str,
    /// The signing author's version for this name.
    pub version: u64,
    /// An optional licence statement.
    pub licence: Option<&'a str>,
    /// An optional discovery description.
    pub description: Option<&'a str>,
    /// Discovery categories, each lower-case ASCII letters, digits and `-`.
    pub categories: &'a [&'a str],
}

/// The check a bundle failed. Every refusal names exactly one.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BundleError {
    /// The file is not a well-formed bundle container.
    #[error("bundle refused at the framing check: {0}")]
    Framing(&'static str),
    /// The manifest or seal is malformed or states an invalid field.
    #[error("bundle refused at the manifest check: {0}")]
    Manifest(String),
    /// The manifest names an ABI this build does not host.
    #[error("bundle refused at the abi check: {0}")]
    Abi(String),
    /// The fork lineage is malformed.
    #[error("bundle refused at the lineage check: {0}")]
    Lineage(String),
    /// The payload is empty or too large.
    #[error("bundle refused at the payload check: {0}")]
    Payload(String),
    /// The payload's BLAKE3 hash differs from the manifest's content hash.
    #[error(
        "bundle refused at the content_hash check: the payload hashes to {actual}, the manifest states {stated}"
    )]
    ContentHash {
        /// The hash the manifest states.
        stated: String,
        /// The hash of the payload in the file.
        actual: String,
    },
    /// The bundle signature does not verify under the manifest's author key.
    #[error(
        "bundle refused at the bundle_signature check: the manifest and payload are not signed by the stated author"
    )]
    BundleSignature,
    /// The F12.17 module signature does not verify under the author key.
    #[error(
        "bundle refused at the module_signature check: the payload's module signature is not the stated author's"
    )]
    ModuleSignature,
}

impl BundleError {
    /// The stable name of the check that failed.
    #[must_use]
    pub const fn check(&self) -> &'static str {
        match self {
            Self::Framing(_) => "framing",
            Self::Manifest(_) => "manifest",
            Self::Abi(_) => "abi",
            Self::Lineage(_) => "lineage",
            Self::Payload(_) => "payload",
            Self::ContentHash { .. } => "content_hash",
            Self::BundleSignature => "bundle_signature",
            Self::ModuleSignature => "module_signature",
        }
    }
}

/// A bundle's signed statement without its payload: the exact manifest JSON
/// and the bundle signature over it (design D6's registry record).
///
/// A record verifies on its own, because the bundle signature covers the
/// manifest bytes and the payload's digest, which the manifest states as its
/// content hash. A registry may index records but cannot alter one without
/// breaking its signature.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleRecord {
    /// The bundle's manifest, exactly as signed.
    pub manifest: String,
    /// The bundle signature, lower-case hex.
    pub bundle_signature: String,
}

impl BundleRecord {
    /// Verify the record offline and return its manifest.
    ///
    /// # Errors
    ///
    /// The manifest, abi or lineage check fails, or the bundle signature
    /// does not verify under the manifest's author key.
    pub fn verify(&self) -> Result<BundleManifest, BundleError> {
        if self.manifest.len() > MAX_MANIFEST_BYTES {
            return Err(BundleError::Manifest(format!(
                "the manifest is {} bytes; the limit is {MAX_MANIFEST_BYTES}",
                self.manifest.len()
            )));
        }
        let manifest: BundleManifest =
            id_core::json_unique::from_slice_without_duplicate_keys(self.manifest.as_bytes())
                .map_err(|e| BundleError::Manifest(e.to_string()))?;
        let author = check_manifest(&manifest)?;
        let digest = parse_hex::<32>(&manifest.content_hash).ok_or_else(|| {
            BundleError::Manifest("content_hash is not 32 bytes of lower-case hex".to_owned())
        })?;
        let signature =
            parse_signature(&self.bundle_signature).ok_or(BundleError::BundleSignature)?;
        author
            .verify_strict(
                &bundle_signing_message_for(self.manifest.as_bytes(), &digest),
                &signature,
            )
            .map_err(|_| BundleError::BundleSignature)?;
        Ok(manifest)
    }

    /// Whether `bundle` is the bundle this record describes.
    #[must_use]
    pub fn describes(&self, bundle: &ProgramBundle) -> bool {
        *self == bundle.record()
    }
}

/// Why a sideloaded bundle was not installed into a world.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BundleInstallError {
    /// The bundle itself failed verification; the error names the check.
    #[error(transparent)]
    Bundle(BundleError),
    /// The bundle verified, but the world refused the install exactly as it
    /// would refuse the same signed upload.
    #[error("the verified bundle was not installed: {0:?}")]
    Install(crate::world_session::InstallError),
}

/// A bundle whose framing, manifest, lineage, ABI, content hash and both
/// signatures have all verified. The only way to hold one is through
/// [`ProgramBundle::decode`], [`export`] or [`fork`].
#[derive(Clone, Debug)]
pub struct ProgramBundle {
    manifest: BundleManifest,
    manifest_bytes: Vec<u8>,
    seal: Seal,
    payload: Vec<u8>,
}

impl ProgramBundle {
    /// Parse and fully verify a bundle file, offline.
    ///
    /// # Errors
    ///
    /// The first check the bytes fail, in the order framing, manifest, abi,
    /// lineage, payload, content hash, bundle signature, module signature.
    pub fn decode(bytes: &[u8]) -> Result<Self, BundleError> {
        let rest = bytes
            .strip_prefix(BUNDLE_MAGIC)
            .ok_or(BundleError::Framing(
                "the file does not start with the bundle magic",
            ))?;
        let (manifest_bytes, rest) = take_segment(rest, MAX_MANIFEST_BYTES, "manifest")?;
        let (seal_bytes, payload) = take_segment(rest, MAX_SEAL_BYTES, "seal")?;
        let manifest: BundleManifest =
            id_core::json_unique::from_slice_without_duplicate_keys(manifest_bytes)
                .map_err(|e| BundleError::Manifest(e.to_string()))?;
        let seal: Seal = id_core::json_unique::from_slice_without_duplicate_keys(seal_bytes)
            .map_err(|e| BundleError::Manifest(format!("seal: {e}")))?;
        let author = check_manifest(&manifest)?;
        check_payload(payload)?;
        let actual = module_hash(payload);
        if actual != manifest.content_hash {
            return Err(BundleError::ContentHash {
                stated: manifest.content_hash,
                actual,
            });
        }
        let bundle_signature =
            parse_signature(&seal.bundle_signature).ok_or(BundleError::BundleSignature)?;
        author
            .verify_strict(
                &bundle_signing_message(manifest_bytes, payload),
                &bundle_signature,
            )
            .map_err(|_| BundleError::BundleSignature)?;
        let module_signature =
            parse_signature(&seal.module_signature).ok_or(BundleError::ModuleSignature)?;
        author
            .verify_strict(&module_signing_message(payload), &module_signature)
            .map_err(|_| BundleError::ModuleSignature)?;
        Ok(Self {
            manifest,
            manifest_bytes: manifest_bytes.to_vec(),
            seal,
            payload: payload.to_vec(),
        })
    }

    /// The bundle file. The manifest and payload are byte for byte the ones
    /// that were signed; the seal is written in its canonical form.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            BUNDLE_MAGIC.len() + self.manifest_bytes.len() + self.payload.len() + 256,
        );
        out.extend_from_slice(BUNDLE_MAGIC);
        push_segment(&mut out, &self.manifest_bytes);
        let seal = serde_json::to_vec(&self.seal).unwrap_or_default();
        push_segment(&mut out, &seal);
        out.extend_from_slice(&self.payload);
        out
    }

    /// The verified manifest.
    #[must_use]
    pub const fn manifest(&self) -> &BundleManifest {
        &self.manifest
    }

    /// This program's D1 identity.
    #[must_use]
    pub fn identity(&self) -> ProgramIdentity {
        self.manifest.identity()
    }

    /// The program payload; its hash is the manifest's content hash.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// The bundle's registry record: its manifest and signature, without the
    /// payload.
    #[must_use]
    pub fn record(&self) -> BundleRecord {
        BundleRecord {
            manifest: String::from_utf8_lossy(&self.manifest_bytes).into_owned(),
            bundle_signature: self.seal.bundle_signature.clone(),
        }
    }

    /// The verified bundle signature, lower-case hex.
    #[must_use]
    pub fn bundle_signature(&self) -> &str {
        &self.seal.bundle_signature
    }

    /// The install record a world keeps for this bundle (design D4). The
    /// policy is pinned: an update is a new install decision.
    #[must_use]
    pub fn provenance(&self) -> crate::world::BundleProvenance {
        crate::world::BundleProvenance {
            schema_version: 1,
            program: self.identity(),
            abi: self.manifest.abi.clone(),
            lineage: self.manifest.lineage.clone(),
            licence: self.manifest.licence.clone(),
            bundle_signature: self.seal.bundle_signature.clone(),
            policy: crate::world::InstallPolicy::Pinned,
        }
    }

    /// The F12.17 signature to present when installing the payload into a
    /// world with a publisher trust list.
    #[must_use]
    pub fn module_signature(&self) -> ModuleSignature {
        ModuleSignature {
            public_key: self.manifest.author.clone(),
            signature: self.seal.module_signature.clone(),
        }
    }
}

/// Export an original program (empty lineage) as a signed bundle.
///
/// # Errors
///
/// The spec or payload fails the same checks [`ProgramBundle::decode`]
/// applies; nothing is signed in that case.
pub fn export(
    payload: Vec<u8>,
    spec: ProgramSpec<'_>,
    author: &SigningKey,
) -> Result<ProgramBundle, BundleError> {
    seal(payload, spec, Vec::new(), author)
}

/// Fork `parent` as a new program signed by `author` (D3).
///
/// The fork's lineage is the parent's lineage followed by the parent's own
/// identity, so a fork of a fork names every ancestor and none can be
/// dropped without breaking the bundle signature. Forking needs no
/// permission from the parent's author and no registry. The payload may be
/// the parent's unchanged or new work; either way it is the forking
/// author's from this point on.
///
/// # Errors
///
/// The lineage would exceed [`MAX_LINEAGE_DEPTH`], or the spec or payload
/// fails the checks [`ProgramBundle::decode`] applies.
pub fn fork(
    parent: &ProgramBundle,
    payload: Vec<u8>,
    spec: ProgramSpec<'_>,
    author: &SigningKey,
) -> Result<ProgramBundle, BundleError> {
    let mut lineage = parent.manifest.lineage.clone();
    lineage.push(parent.identity());
    seal(payload, spec, lineage, author)
}

fn seal(
    payload: Vec<u8>,
    spec: ProgramSpec<'_>,
    lineage: Vec<ProgramIdentity>,
    author: &SigningKey,
) -> Result<ProgramBundle, BundleError> {
    let manifest = BundleManifest {
        author: encode_hex(&author.verifying_key().to_bytes()),
        name: spec.name.to_owned(),
        version: spec.version,
        content_hash: module_hash(&payload),
        abi: AbiDescriptor::plaza_v1(),
        lineage,
        licence: spec.licence.map(str::to_owned),
        description: spec.description.map(str::to_owned),
        categories: spec.categories.iter().map(|c| (*c).to_owned()).collect(),
    };
    check_manifest(&manifest)?;
    check_payload(&payload)?;
    let manifest_bytes =
        serde_json::to_vec(&manifest).map_err(|e| BundleError::Manifest(e.to_string()))?;
    if manifest_bytes.len() > MAX_MANIFEST_BYTES {
        return Err(BundleError::Manifest(format!(
            "the manifest is {} bytes; the limit is {MAX_MANIFEST_BYTES}",
            manifest_bytes.len()
        )));
    }
    let seal = Seal {
        bundle_signature: encode_hex(
            &author
                .sign(&bundle_signing_message(&manifest_bytes, &payload))
                .to_bytes(),
        ),
        module_signature: encode_hex(&author.sign(&module_signing_message(&payload)).to_bytes()),
    };
    Ok(ProgramBundle {
        manifest,
        manifest_bytes,
        seal,
        payload,
    })
}

fn bundle_signing_message(manifest_bytes: &[u8], payload: &[u8]) -> Vec<u8> {
    bundle_signing_message_for(manifest_bytes, blake3::hash(payload).as_bytes())
}

/// The bundle signature's message, given the payload's BLAKE3 digest. The
/// digest is the manifest's content hash, so a [`BundleRecord`] verifies
/// without its payload.
fn bundle_signing_message_for(manifest_bytes: &[u8], payload_digest: &[u8; 32]) -> Vec<u8> {
    let mut message = BUNDLE_SIGNING_DOMAIN.to_vec();
    message.extend_from_slice(blake3::hash(manifest_bytes).as_bytes());
    message.extend_from_slice(payload_digest);
    message
}

fn take_segment<'a>(
    bytes: &'a [u8],
    limit: usize,
    what: &'static str,
) -> Result<(&'a [u8], &'a [u8]), BundleError> {
    let (length, rest) = bytes.split_first_chunk::<4>().ok_or(BundleError::Framing(
        "the file ends inside a segment length",
    ))?;
    let length = usize::try_from(u32::from_le_bytes(*length))
        .map_err(|_| BundleError::Framing("a segment length does not fit this platform"))?;
    if length > limit {
        return Err(BundleError::Framing(if what == "manifest" {
            "the manifest segment is over its size limit"
        } else {
            "the seal segment is over its size limit"
        }));
    }
    if rest.len() < length {
        return Err(BundleError::Framing("the file ends inside a segment"));
    }
    Ok(rest.split_at(length))
}

fn push_segment(out: &mut Vec<u8>, segment: &[u8]) {
    let length = u32::try_from(segment.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(segment);
}

/// Canonical lower-case hex of exactly `N` bytes.
fn parse_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let bytes: [u8; N] = decode_hex(text)?.try_into().ok()?;
    (encode_hex(&bytes) == text).then_some(bytes)
}

fn parse_signature(text: &str) -> Option<Signature> {
    parse_hex::<64>(text).map(|bytes| Signature::from_bytes(&bytes))
}

fn check_identity(identity: &ProgramIdentity) -> Result<VerifyingKey, String> {
    let author = parse_hex::<32>(&identity.author)
        .ok_or_else(|| "author is not 32 bytes of lower-case hex".to_owned())?;
    let author = VerifyingKey::from_bytes(&author)
        .map_err(|_| "author is not a valid Ed25519 public key".to_owned())?;
    if identity.name.is_empty() {
        return Err("name is empty".to_owned());
    }
    if identity.name.len() > MAX_NAME_BYTES {
        return Err(format!(
            "name is {} bytes; the limit is {MAX_NAME_BYTES}",
            identity.name.len()
        ));
    }
    if identity.name.chars().any(char::is_control) {
        return Err("name contains a control character".to_owned());
    }
    if parse_hex::<32>(&identity.content_hash).is_none() {
        return Err("content_hash is not 32 bytes of lower-case hex".to_owned());
    }
    Ok(author)
}

fn check_manifest(manifest: &BundleManifest) -> Result<VerifyingKey, BundleError> {
    let author = check_identity(&manifest.identity()).map_err(BundleError::Manifest)?;
    if let Some(licence) = &manifest.licence
        && licence.len() > MAX_LICENCE_BYTES
    {
        return Err(BundleError::Manifest(format!(
            "licence is {} bytes; the limit is {MAX_LICENCE_BYTES}",
            licence.len()
        )));
    }
    check_discovery(manifest).map_err(BundleError::Manifest)?;
    if manifest.abi != AbiDescriptor::plaza_v1() {
        return Err(BundleError::Abi(format!(
            "the payload targets {} v{}; this build hosts {PLAZA_ABI_FAMILY} v{PLAZA_ABI_VERSION}",
            manifest.abi.family, manifest.abi.version
        )));
    }
    if manifest.lineage.len() > MAX_LINEAGE_DEPTH {
        return Err(BundleError::Lineage(format!(
            "{} ancestors; the limit is {MAX_LINEAGE_DEPTH}",
            manifest.lineage.len()
        )));
    }
    let own = manifest.identity();
    for (index, ancestor) in manifest.lineage.iter().enumerate() {
        check_identity(ancestor)
            .map_err(|reason| BundleError::Lineage(format!("ancestor {index}: {reason}")))?;
        if *ancestor == own {
            return Err(BundleError::Lineage(format!(
                "ancestor {index} is this program's own identity"
            )));
        }
    }
    Ok(author)
}

/// The discovery metadata's bounds: a description within
/// [`MAX_DESCRIPTION_BYTES`], and at most [`MAX_CATEGORIES`] distinct
/// categories in the portable form.
fn check_discovery(manifest: &BundleManifest) -> Result<(), String> {
    if let Some(description) = &manifest.description
        && description.len() > MAX_DESCRIPTION_BYTES
    {
        return Err(format!(
            "description is {} bytes; the limit is {MAX_DESCRIPTION_BYTES}",
            description.len()
        ));
    }
    if manifest.categories.len() > MAX_CATEGORIES {
        return Err(format!(
            "{} categories; the limit is {MAX_CATEGORIES}",
            manifest.categories.len()
        ));
    }
    for (index, category) in manifest.categories.iter().enumerate() {
        let portable = !category.is_empty()
            && category.len() <= MAX_CATEGORY_BYTES
            && category
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !portable {
            return Err(format!(
                "category {index} must be 1 to {MAX_CATEGORY_BYTES} lower-case ASCII letters, digits or '-'"
            ));
        }
        if manifest.categories[..index].contains(category) {
            return Err(format!("category {category:?} is repeated"));
        }
    }
    Ok(())
}

fn check_payload(payload: &[u8]) -> Result<(), BundleError> {
    if payload.is_empty() {
        return Err(BundleError::Payload("the payload is empty".to_owned()));
    }
    if payload.len() > MAX_WORLD_MODULE_BYTES {
        return Err(BundleError::Payload(format!(
            "the payload is {} bytes; the world module limit is {MAX_WORLD_MODULE_BYTES}",
            payload.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const PAYLOAD: &[u8] = b"\0asm\x01\0\0\0 program payload";

    fn alice() -> SigningKey {
        SigningKey::from_bytes(&[1; 32])
    }

    fn bob() -> SigningKey {
        SigningKey::from_bytes(&[2; 32])
    }

    fn spec(name: &str, version: u64) -> ProgramSpec<'_> {
        ProgramSpec {
            name,
            version,
            licence: Some("MIT"),
            description: None,
            categories: &[],
        }
    }

    fn original() -> ProgramBundle {
        export(PAYLOAD.to_vec(), spec("chess", 3), &alice()).unwrap()
    }

    /// Offsets of the manifest and payload inside an encoded bundle.
    fn layout(bytes: &[u8]) -> (std::ops::Range<usize>, usize) {
        let start = BUNDLE_MAGIC.len() + 4;
        let manifest_len =
            u32::from_le_bytes(bytes[BUNDLE_MAGIC.len()..start].try_into().unwrap()) as usize;
        let seal_start = start + manifest_len;
        let seal_len =
            u32::from_le_bytes(bytes[seal_start..seal_start + 4].try_into().unwrap()) as usize;
        (start..seal_start, seal_start + 4 + seal_len)
    }

    /// Replace the manifest segment, keeping the seal and payload.
    fn with_manifest(bytes: &[u8], manifest: &[u8]) -> Vec<u8> {
        let (range, _) = layout(bytes);
        let mut out = BUNDLE_MAGIC.to_vec();
        push_segment(&mut out, manifest);
        out.extend_from_slice(&bytes[range.end..]);
        out
    }

    #[test]
    fn export_round_trips_through_decode_offline() {
        let bundle = original();
        let bytes = bundle.to_bytes();
        let decoded = ProgramBundle::decode(&bytes).unwrap();
        assert_eq!(decoded.manifest(), bundle.manifest());
        assert_eq!(decoded.payload(), PAYLOAD);
        assert_eq!(decoded.to_bytes(), bytes, "re-encoding is byte-identical");
        let identity = decoded.identity();
        assert_eq!(
            identity.author,
            encode_hex(&alice().verifying_key().to_bytes())
        );
        assert_eq!(identity.name, "chess");
        assert_eq!(identity.version, 3);
        assert_eq!(identity.content_hash, module_hash(PAYLOAD));
        assert!(decoded.manifest().lineage.is_empty());
        assert_eq!(decoded.manifest().licence.as_deref(), Some("MIT"));
        assert_eq!(decoded.manifest().abi, AbiDescriptor::plaza_v1());
    }

    #[test]
    fn module_signature_passes_a_trust_listed_world_install_check() {
        let bundle = original();
        let signature = bundle.module_signature();
        let trusted = [alice().verifying_key().to_bytes()];
        crate::world_session::verify_module_signature(&trusted, bundle.payload(), Some(&signature))
            .expect("the author's bundle installs where the author is trusted");
        let untrusted = [bob().verifying_key().to_bytes()];
        assert!(
            crate::world_session::verify_module_signature(
                &untrusted,
                bundle.payload(),
                Some(&signature)
            )
            .is_err()
        );
    }

    #[test]
    fn a_tampered_payload_is_refused_at_the_content_hash_check() {
        let mut bytes = original().to_bytes();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        let error = ProgramBundle::decode(&bytes).unwrap_err();
        assert_eq!(error.check(), "content_hash", "{error}");
        assert!(error.to_string().contains("content_hash"), "{error}");
    }

    #[test]
    fn a_tampered_manifest_is_refused_at_the_bundle_signature_check() {
        let bytes = original().to_bytes();
        let (range, _) = layout(&bytes);
        let manifest = std::str::from_utf8(&bytes[range]).unwrap();
        for (from, to) in [
            (r#""version":3"#, r#""version":4"#),
            (r#""name":"chess""#, r#""name":"chesz""#),
            (r#""licence":"MIT""#, r#""licence":"GPL""#),
        ] {
            assert!(manifest.contains(from), "{manifest}");
            let edited = manifest.replacen(from, to, 1);
            let error =
                ProgramBundle::decode(&with_manifest(&bytes, edited.as_bytes())).unwrap_err();
            assert_eq!(error.check(), "bundle_signature", "{from}: {error}");
        }
    }

    #[test]
    fn a_reserialised_manifest_with_the_same_meaning_is_still_refused() {
        let bytes = original().to_bytes();
        let (range, _) = layout(&bytes);
        let mut pretty = serde_json::to_vec_pretty(
            &serde_json::from_slice::<serde_json::Value>(&bytes[range]).unwrap(),
        )
        .unwrap();
        pretty.push(b'\n');
        let error = ProgramBundle::decode(&with_manifest(&bytes, &pretty)).unwrap_err();
        assert_eq!(error.check(), "bundle_signature", "{error}");
    }

    #[test]
    fn a_swapped_author_is_refused_at_the_bundle_signature_check() {
        let bytes = original().to_bytes();
        let (range, _) = layout(&bytes);
        let manifest = std::str::from_utf8(&bytes[range]).unwrap();
        let edited = manifest.replacen(
            &encode_hex(&alice().verifying_key().to_bytes()),
            &encode_hex(&bob().verifying_key().to_bytes()),
            1,
        );
        let error = ProgramBundle::decode(&with_manifest(&bytes, edited.as_bytes())).unwrap_err();
        assert_eq!(error.check(), "bundle_signature", "{error}");
    }

    #[test]
    fn a_foreign_module_signature_is_refused_at_the_module_signature_check() {
        let mut forged = original();
        forged.seal.module_signature =
            encode_hex(&bob().sign(&module_signing_message(PAYLOAD)).to_bytes());
        let error = ProgramBundle::decode(&forged.to_bytes()).unwrap_err();
        assert_eq!(error.check(), "module_signature", "{error}");
    }

    #[test]
    fn framing_faults_are_refused_at_the_framing_check() {
        let bytes = original().to_bytes();
        let (range, _) = layout(&bytes);
        for (label, bad) in [
            ("empty", Vec::new()),
            ("no magic", bytes[1..].to_vec()),
            ("cut in a length", bytes[..BUNDLE_MAGIC.len() + 2].to_vec()),
            ("cut in the manifest", bytes[..range.end - 1].to_vec()),
        ] {
            let error = ProgramBundle::decode(&bad).unwrap_err();
            assert_eq!(error.check(), "framing", "{label}: {error}");
        }
        let mut oversized = BUNDLE_MAGIC.to_vec();
        oversized.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            ProgramBundle::decode(&oversized).unwrap_err().check(),
            "framing"
        );
    }

    #[test]
    fn a_manifest_with_a_repeated_or_unknown_key_is_refused_by_name() {
        let bytes = original().to_bytes();
        let (range, _) = layout(&bytes);
        let manifest = std::str::from_utf8(&bytes[range]).unwrap();
        let repeated = manifest.replacen('{', r#"{"version":9,"#, 1);
        let error = ProgramBundle::decode(&with_manifest(&bytes, repeated.as_bytes())).unwrap_err();
        assert_eq!(error.check(), "manifest", "{error}");
        assert!(error.to_string().contains(r#""version""#), "{error}");
        let unknown = manifest.replacen('{', r#"{"publisher":"x","#, 1);
        let error = ProgramBundle::decode(&with_manifest(&bytes, unknown.as_bytes())).unwrap_err();
        assert_eq!(error.check(), "manifest", "{error}");
    }

    #[test]
    fn an_unhosted_abi_is_refused_at_the_abi_check() {
        let bytes = original().to_bytes();
        let (range, _) = layout(&bytes);
        let manifest = std::str::from_utf8(&bytes[range]).unwrap();
        let edited = manifest.replacen(r#""version":1}"#, r#""version":2}"#, 1);
        assert_ne!(edited, manifest);
        let error = ProgramBundle::decode(&with_manifest(&bytes, edited.as_bytes())).unwrap_err();
        assert_eq!(error.check(), "abi", "{error}");
        assert!(error.to_string().contains("plaza v2"), "{error}");
    }

    #[test]
    fn export_refuses_bad_specs_and_payloads_before_signing() {
        let key = alice();
        let check = |payload: &[u8], spec: ProgramSpec<'_>| {
            export(payload.to_vec(), spec, &key).unwrap_err().check()
        };
        assert_eq!(check(PAYLOAD, spec("", 1)), "manifest");
        assert_eq!(check(PAYLOAD, spec("a\nb", 1)), "manifest");
        let long = "x".repeat(MAX_NAME_BYTES + 1);
        assert_eq!(check(PAYLOAD, spec(&long, 1)), "manifest");
        assert_eq!(check(b"", spec("chess", 1)), "payload");
        let huge = vec![0; MAX_WORLD_MODULE_BYTES + 1];
        assert_eq!(check(&huge, spec("chess", 1)), "payload");
    }

    #[test]
    fn discovery_metadata_is_signed_and_bounded() {
        let key = alice();
        let with = |description: Option<&str>, categories: &[&str]| {
            export(
                PAYLOAD.to_vec(),
                ProgramSpec {
                    description,
                    categories,
                    ..spec("chess", 1)
                },
                &key,
            )
        };
        let bundle = with(Some("The classic board game."), &["board", "two-player"]).unwrap();
        let decoded = ProgramBundle::decode(&bundle.to_bytes()).unwrap();
        assert_eq!(
            decoded.manifest().description.as_deref(),
            Some("The classic board game.")
        );
        assert_eq!(decoded.manifest().categories, ["board", "two-player"]);
        let bytes = bundle.to_bytes();
        let edited = String::from_utf8_lossy(&bytes).replacen("two-player", "one-player", 1);
        assert_eq!(
            ProgramBundle::decode(edited.as_bytes())
                .unwrap_err()
                .check(),
            "bundle_signature",
            "a registry cannot alter signed discovery metadata"
        );
        assert!(
            !String::from_utf8_lossy(&spec_bundle_bytes()).contains("categories"),
            "absent metadata is not serialised"
        );

        let check = |description: Option<&str>, categories: &[&str]| {
            with(description, categories).unwrap_err().check()
        };
        let long = "x".repeat(MAX_DESCRIPTION_BYTES + 1);
        assert_eq!(check(Some(&long), &[]), "manifest");
        assert_eq!(check(None, &["Board"]), "manifest");
        assert_eq!(check(None, &[""]), "manifest");
        assert_eq!(check(None, &["board", "board"]), "manifest");
        let many: Vec<String> = (0..=MAX_CATEGORIES).map(|i| format!("c{i}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        assert_eq!(check(None, &many), "manifest");
    }

    fn spec_bundle_bytes() -> Vec<u8> {
        export(PAYLOAD.to_vec(), spec("chess", 1), &alice())
            .unwrap()
            .to_bytes()
    }

    #[test]
    fn a_fork_of_a_fork_carries_the_whole_ordered_chain() {
        let parent = original();
        let child = fork(
            &parent,
            b"bob's edit".to_vec(),
            ProgramSpec {
                name: "chess-variant",
                version: 1,
                licence: None,
                description: None,
                categories: &[],
            },
            &bob(),
        )
        .unwrap();
        let carol = SigningKey::from_bytes(&[3; 32]);
        let grandchild = fork(&child, PAYLOAD.to_vec(), spec("chess", 1), &carol).unwrap();
        let decoded = ProgramBundle::decode(&grandchild.to_bytes()).unwrap();
        assert_eq!(
            decoded.manifest().lineage,
            vec![parent.identity(), child.identity()],
            "oldest first, the direct parent last"
        );
        assert_eq!(decoded.manifest().parent(), Some(&child.identity()));
        assert_eq!(
            decoded.identity().author,
            encode_hex(&carol.verifying_key().to_bytes()),
            "the fork is the forking author's work"
        );
        assert_ne!(decoded.identity(), parent.identity());
    }

    #[test]
    fn a_lineage_edit_breaks_the_bundle_signature() {
        let child = fork(&original(), PAYLOAD.to_vec(), spec("chess", 1), &bob()).unwrap();
        let bytes = child.to_bytes();
        let (range, _) = layout(&bytes);
        let mut manifest: BundleManifest = serde_json::from_slice(&bytes[range]).unwrap();
        manifest.lineage.clear();
        let dropped = serde_json::to_vec(&manifest).unwrap();
        let error = ProgramBundle::decode(&with_manifest(&bytes, &dropped)).unwrap_err();
        assert_eq!(
            error.check(),
            "bundle_signature",
            "dropping attribution: {error}"
        );
    }

    #[test]
    fn a_record_verifies_without_its_payload_and_refuses_edits() {
        let bundle = fork(&original(), PAYLOAD.to_vec(), spec("chess", 1), &bob()).unwrap();
        let record = bundle.record();
        assert_eq!(record.verify().unwrap(), *bundle.manifest());
        assert!(record.describes(&bundle));
        assert!(!record.describes(&original()));

        let edited = BundleRecord {
            manifest: record
                .manifest
                .replacen(r#""version":1"#, r#""version":2"#, 1),
            ..record.clone()
        };
        assert_eq!(edited.verify().unwrap_err().check(), "bundle_signature");
        let foreign = BundleRecord {
            bundle_signature: original().record().bundle_signature,
            ..record.clone()
        };
        assert_eq!(foreign.verify().unwrap_err().check(), "bundle_signature");
        let repeated = BundleRecord {
            manifest: record.manifest.replacen('{', r#"{"name":"x","#, 1),
            ..record
        };
        assert_eq!(repeated.verify().unwrap_err().check(), "manifest");
    }

    #[test]
    fn a_lineage_naming_the_program_itself_is_refused() {
        let parent = original();
        let error = fork(&parent, PAYLOAD.to_vec(), spec("chess", 3), &alice()).unwrap_err();
        assert_eq!(error.check(), "lineage", "{error}");
    }
}
