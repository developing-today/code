//! Typed requests and the bundle manifest schema (spec §7.1).
//!
//! The CLI parses flags into these requests; the engine consumes
//! them. Nothing in the engine reads argv, the environment, or a
//! config file — in particular, **signer sources are explicit
//! request fields** (§24.3): an issuer or leaf creation request
//! cannot even be constructed without naming the directory that
//! holds the signing keys.

use crate::algorithm::AlgorithmId;
use crate::graph::CrossSigningMode;
use crate::timeparse::Rfc3339;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The manifest schema id written into every bundle (§7.1:
/// schema-versioned `manifest.json` is the canonical record).
pub const MANIFEST_SCHEMA: &str = "fleet-cert-manifest/v1";

/// An explicitly passed signer source (§24.3): the directory holding
/// a CA bundle's private keys (its `private/keys` directory, or an
/// external store directory named at creation time). Root/issuer
/// signing operations take this as a required argument — there is
/// no ambient key discovery anywhere in this crate.
#[derive(Debug, Clone)]
pub struct SignerSource {
    pub key_dir: PathBuf,
}

/// `cert root create` (§§6.2, 24.3).
#[derive(Debug, Clone)]
pub struct RootCreateRequest {
    pub organization: String,
    pub country: String,
    pub algorithms: Vec<AlgorithmId>,
    pub generation: u32,
    /// Partition label (`A`); a root set has one partition per
    /// generation in this slice.
    pub partition: String,
    pub cross_signing: CrossSigningMode,
    pub not_before: Rfc3339,
    pub not_after: Rfc3339,
    /// The §24.1(1) explicit opt-in for legacy-gated algorithms.
    pub allow_legacy_root_strength: bool,
    /// Where private keys go: `None` = the bundle's own
    /// `private/keys`; `Some(dir)` = that external store directory
    /// (recorded in the manifest as a location class, never synced).
    pub private_store: Option<PathBuf>,
}

/// Issuer set purpose (§24.4): drives CN/OU derivation, EKU-less CA
/// profile, and (for web) optional permitted-DNS name constraints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IssuerPurpose {
    General,
    WebServer,
}

impl IssuerPurpose {
    pub fn parse(input: &str) -> crate::Result<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "general" | "base" => Ok(IssuerPurpose::General),
            "web" | "web-server" | "webserver" => Ok(IssuerPurpose::WebServer),
            other => Err(crate::CertError::usage(format!(
                "unknown issuer purpose '{other}' (general | web-server)"
            ))),
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            IssuerPurpose::General => "general",
            IssuerPurpose::WebServer => "web-server",
        }
    }
}

/// `cert issuer create` (§§6.2, 24.4, 24.5).
#[derive(Debug, Clone)]
pub struct IssuerCreateRequest {
    /// The root bundle directory (public material + manifest).
    pub root_bundle: PathBuf,
    /// REQUIRED explicit signer source for the root keys (§24.3).
    pub root_signer: SignerSource,
    pub name: String,
    pub purpose: IssuerPurpose,
    pub ou: Option<String>,
    /// (algorithm, partition) key slots to create — resolved from
    /// the `--algorithms` × `--partitions` grammar by the caller
    /// ([`resolve_slots`]).
    pub slots: Vec<(AlgorithmId, String)>,
    pub cross_signing: CrossSigningMode,
    pub not_before: Rfc3339,
    pub not_after: Rfc3339,
    pub allow_legacy_root_strength: bool,
    /// Permitted DNS name constraints (web purpose, §24.4).
    pub permitted_dns: Vec<String>,
    pub private_store: Option<PathBuf>,
}

/// `cert leaf issue` (§§6.2, 24.6).
#[derive(Debug, Clone)]
pub struct LeafIssueRequest {
    /// The issuer bundle directory.
    pub issuer_bundle: PathBuf,
    /// REQUIRED explicit signer source for the issuer key (§24.3).
    pub issuer_signer: SignerSource,
    /// Leaf profile name (see [`crate::profile`]).
    pub profile: String,
    /// Delivery name (directory + manifest identity).
    pub name: String,
    pub common_name: Option<String>,
    pub dns_names: Vec<String>,
    /// Subject key algorithm; `None` = the profile's key algorithm.
    pub key_algorithm: Option<AlgorithmId>,
    /// Which issuer key signs: (algorithm, partition). `None` =
    /// (the leaf key algorithm if the profile's is a CA-supported
    /// pairing, else the first slot), partition A.
    pub issuer_algorithm: Option<AlgorithmId>,
    pub issuer_partition: Option<String>,
    pub not_before: Rfc3339,
    pub not_after: Rfc3339,
    pub private_store: Option<PathBuf>,
}

/// Resolve the `--algorithms` × `--partitions` grammar into key
/// slots (§24.4):
/// - partitions given → Cartesian product (the documented default:
///   `--algorithms ed25519,p256 --partitions A,B` gives every
///   algorithm both partitions);
/// - partitions omitted + a repeated algorithm in the list → the
///   repeats take successive partitions in input order
///   (`rsa,rsa,ed25519` → RSA A, RSA B, Ed25519 A);
/// - partitions omitted, no repeats → partition A for each.
pub fn resolve_slots(
    algorithms: &[AlgorithmId],
    partitions: Option<&[String]>,
) -> crate::Result<Vec<(AlgorithmId, String)>> {
    use std::collections::BTreeMap;
    match partitions {
        Some(parts) => {
            let mut out = Vec::new();
            for algo in algorithms {
                for part in parts {
                    let slot = (*algo, part.to_ascii_lowercase());
                    if out.contains(&slot) {
                        return Err(crate::CertError::usage(format!(
                            "duplicate issuer slot {} partition {}",
                            algo.name(),
                            part
                        )));
                    }
                    out.push(slot);
                }
            }
            Ok(out)
        }
        None => {
            let mut counts: BTreeMap<AlgorithmId, usize> = BTreeMap::new();
            let mut out = Vec::new();
            for algo in algorithms {
                let idx = counts.entry(*algo).or_insert(0);
                let part = crate::naming::partition_label(*idx);
                *idx += 1;
                out.push((*algo, part));
            }
            Ok(out)
        }
    }
}

/// The kind of a bundle (§7.1 `bundle_type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BundleType {
    RootSet,
    IssuerSet,
    Leaf,
}

impl BundleType {
    pub fn as_str(&self) -> &'static str {
        match self {
            BundleType::RootSet => "root-set",
            BundleType::IssuerSet => "issuer-set",
            BundleType::Leaf => "leaf",
        }
    }
}

/// One certificate in a bundle (§7.1 per-certificate record).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertRecord {
    /// `root` | `root-cross` | `issuer` | `leaf`.
    pub role: String,
    /// Subject key algorithm.
    pub algorithm: AlgorithmId,
    /// Signing key algorithm (`None` = self-signed).
    pub signer_algorithm: Option<AlgorithmId>,
    /// Partition label (lowercase) of the subject key.
    pub partition: String,
    /// The variant a chain builder should prefer for this key.
    #[serde(default)]
    pub primary: bool,
    pub subject_dn: String,
    pub issuer_dn: String,
    pub serial_hex: String,
    pub ski_hex: String,
    pub aki_hex: String,
    pub fingerprint_sha256: String,
    pub not_before: Rfc3339,
    pub not_after: Rfc3339,
    pub is_ca: bool,
    pub path_len: Option<u8>,
    /// Bundle-relative PEM path.
    pub cert_path: String,
    /// Bundle-relative PKCS#8 path when the key lives in the bundle;
    /// `None` when the key lives in an external store.
    pub key_path: Option<String>,
}

/// One file in a bundle's index (§7.1 file index with checksums).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub path: String,
    pub kind: String,
    pub sha256: String,
    pub bytes: u64,
    /// `public` | `private` (§7.1 visibility).
    pub visibility: String,
}

/// A reference to a parent bundle (§7.1 `parent_refs`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParentRef {
    pub bundle_type: BundleType,
    pub bundle_id: String,
    pub name: String,
    /// SHA-256 fingerprints of the parent's anchor certificates.
    pub anchor_fingerprints: Vec<String>,
}

/// The canonical bundle manifest (§7.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleManifest {
    pub schema: String,
    pub bundle_id: String,
    pub bundle_type: BundleType,
    pub bundle_version: u32,
    pub name: String,
    pub slug: String,
    pub organization: String,
    pub country: String,
    pub purpose: Option<String>,
    pub generation: u32,
    pub created_at: Rfc3339,
    /// fleet-cert crate version that created the bundle.
    pub engine_version: String,
    pub parent: Option<ParentRef>,
    /// `bundle-private` | `external-store` (§7.1: the manifest
    /// records the custody class, never a secret value).
    pub key_location: String,
    pub cross_signing: Option<String>,
    pub certificates: Vec<CertRecord>,
    pub files: Vec<FileRecord>,
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_cartesian_and_zipped() {
        let algos = [AlgorithmId::Ed25519, AlgorithmId::P256];
        let parts = vec!["a".to_string(), "b".to_string()];
        let slots = resolve_slots(&algos, Some(&parts)).unwrap();
        assert_eq!(slots.len(), 4);
        // Zipped repeats: rsa,rsa,ed25519 → A, B, A (§24.4).
        let algos = [
            AlgorithmId::Rsa4096,
            AlgorithmId::Rsa4096,
            AlgorithmId::Ed25519,
        ];
        let slots = resolve_slots(&algos, None).unwrap();
        assert_eq!(
            slots,
            vec![
                (AlgorithmId::Rsa4096, "a".to_string()),
                (AlgorithmId::Rsa4096, "b".to_string()),
                (AlgorithmId::Ed25519, "a".to_string()),
            ]
        );
    }
}
