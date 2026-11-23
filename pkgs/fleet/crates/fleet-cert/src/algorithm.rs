//! The algorithm registry (spec §§3.1, 6.3, 24.1).
//!
//! One table is the source of truth for every algorithm `fleet cert`
//! knows: its canonical id, its X.509 identifiers (RFC 9881 for
//! ML-DSA-44), whether the engine in this build can actually generate
//! and sign with it, and the policy class that governs it. Nothing
//! else in the crate hardcodes an algorithm fact.
//!
//! Support reality (verified 2026-11-23 against rcgen 0.14.10 +
//! aws-lc-rs on OpenSSL 3.5.9 as the independent validator): all five
//! registered algorithms generate, sign, and cross-sign for real —
//! ML-DSA-44 included, with the standardized RFC 9881 identifiers.
//! There is no stubbed algorithm in this registry.

use crate::error::{CertError, Result};
use serde::{Deserialize, Serialize};

/// A registered key/signature algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AlgorithmId {
    Ed25519,
    P256,
    Rsa4096,
    Rsa2048,
    MlDsa44,
}

/// Whether the engine in this build can generate keys and sign.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SupportStatus {
    Supported,
    Unsupported,
}

/// The policy class an algorithm sits in (spec §24.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PolicyClass {
    /// In the default algorithm set.
    Default,
    /// Supported, but not in any default set; name it explicitly.
    Supported,
    /// Legacy/compatibility only. CA use requires the explicit
    /// `--allow-legacy-root-strength` opt-in (spec §24.1(1): RSA-2048
    /// supplies ~112-bit security and is questionable as a new
    /// long-lived root; leaf/compatibility use stays allowed).
    LegacyGated,
}

/// One registry row.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct AlgorithmInfo {
    pub id: AlgorithmId,
    /// Canonical algorithm id (`ed25519`, `p256`, ...).
    pub name: &'static str,
    pub display_name: &'static str,
    /// SubjectPublicKeyInfo algorithm OID, dotted string.
    pub spki_oid: &'static str,
    /// Signature algorithm OID, dotted string.
    pub signature_oid: &'static str,
    pub support: SupportStatus,
    pub policy: PolicyClass,
    /// Post-quantum (FIPS 204) — drives PQ trust selection (§3.2).
    pub post_quantum: bool,
    pub notes: &'static str,
}

impl AlgorithmId {
    pub fn info(self) -> AlgorithmInfo {
        match self {
            AlgorithmId::Ed25519 => AlgorithmInfo {
                id: self,
                name: "ed25519",
                display_name: "Ed25519",
                spki_oid: "1.3.101.112",
                signature_oid: "1.3.101.112",
                support: SupportStatus::Supported,
                policy: PolicyClass::Default,
                post_quantum: false,
                notes: "Preferred classical performance root (spec §3.1).",
            },
            AlgorithmId::P256 => AlgorithmInfo {
                id: self,
                name: "p256",
                display_name: "P-256",
                spki_oid: "1.2.840.10045.2.1",
                signature_oid: "1.2.840.10045.4.3.2",
                support: SupportStatus::Supported,
                policy: PolicyClass::Default,
                post_quantum: false,
                notes: "ECDSA P-256 with SHA-256 (RFC 5758).",
            },
            AlgorithmId::Rsa4096 => AlgorithmInfo {
                id: self,
                name: "rsa-4096",
                display_name: "RSA-4096",
                spki_oid: "1.2.840.113549.1.1.1",
                signature_oid: "1.2.840.113549.1.1.11",
                support: SupportStatus::Supported,
                policy: PolicyClass::Default,
                post_quantum: false,
                notes: "Classical compatibility root; RSA/SHA-256 (spec §3.1).",
            },
            AlgorithmId::Rsa2048 => AlgorithmInfo {
                id: self,
                name: "rsa-2048",
                display_name: "RSA-2048",
                spki_oid: "1.2.840.113549.1.1.1",
                signature_oid: "1.2.840.113549.1.1.11",
                support: SupportStatus::Supported,
                policy: PolicyClass::LegacyGated,
                post_quantum: false,
                notes: "Legacy/compatibility profile (spec §24.1(1)): ~112-bit \
                        classical security. CA/root use requires \
                        --allow-legacy-root-strength; leaf use is allowed policy.",
            },
            AlgorithmId::MlDsa44 => AlgorithmInfo {
                id: self,
                name: "ml-dsa-44",
                display_name: "ML-DSA-44",
                spki_oid: "2.16.840.1.101.3.4.3.17",
                signature_oid: "2.16.840.1.101.3.4.3.17",
                support: SupportStatus::Supported,
                policy: PolicyClass::Default,
                post_quantum: true,
                notes: "FIPS 204 ML-DSA-44 with the standardized RFC 9881 \
                        identifiers — not the pre-standard Dilithium2 scheme \
                        (spec §3.1). NIST security category 2.",
            },
        }
    }

    pub fn name(self) -> &'static str {
        self.info().name
    }

    pub fn display_name(self) -> &'static str {
        self.info().display_name
    }

    pub fn is_post_quantum(self) -> bool {
        self.info().post_quantum
    }

    pub fn is_classical(self) -> bool {
        !self.is_post_quantum()
    }

    pub fn is_supported(self) -> bool {
        matches!(self.info().support, SupportStatus::Supported)
    }

    /// Parse one algorithm name or documented alias (spec §6.3).
    /// Returns the algorithm plus an optional warning the caller must
    /// surface (the `dilithium2` deprecated alias). Ambiguous input —
    /// `dsa` above all — is rejected with suggestions, never guessed.
    pub fn parse(input: &str) -> Result<(AlgorithmId, Option<String>)> {
        let norm = input.trim().to_ascii_lowercase().replace('_', "-");
        let warn = |a: AlgorithmId, w: &str| Ok((a, Some(w.to_string())));
        match norm.as_str() {
            "ed25519" | "ed" => Ok((AlgorithmId::Ed25519, None)),
            "p256" | "p-256" | "secp256r1" | "ecc" | "ecdsa" | "ec" => {
                Ok((AlgorithmId::P256, None))
            }
            "rsa-4096" | "rsa4096" | "rsa" => Ok((AlgorithmId::Rsa4096, None)),
            "rsa-2048" | "rsa2048" => Ok((AlgorithmId::Rsa2048, None)),
            "ml-dsa-44" | "ml-dsa" | "mldsa" | "mldsa44" | "ml-dsa44" => {
                Ok((AlgorithmId::MlDsa44, None))
            }
            "dilithium2" => warn(
                AlgorithmId::MlDsa44,
                "alias 'dilithium2' is deprecated: it maps to standardized \
                 ML-DSA-44 (RFC 9881); pre-standard Dilithium2 keys are NOT \
                 byte-compatible and are never silently equated (spec §6.3)",
            ),
            "dsa" => Err(CertError::usage(
                "'dsa' is ambiguous and is rejected (spec §6.3): did you mean \
                 'ml-dsa-44' (post-quantum) or 'ecdsa'/'p256'?",
            )),
            other => Err(CertError::usage(format!(
                "unknown algorithm '{other}': known algorithms are \
                 ed25519, p256, rsa-4096, rsa-2048 (legacy), ml-dsa-44"
            ))),
        }
    }
}

/// Every registered algorithm, in canonical registry order.
pub fn registry() -> Vec<AlgorithmInfo> {
    [
        AlgorithmId::Ed25519,
        AlgorithmId::P256,
        AlgorithmId::Rsa4096,
        AlgorithmId::Rsa2048,
        AlgorithmId::MlDsa44,
    ]
    .iter()
    .map(|a| a.info())
    .collect()
}

/// The default root/issuer algorithm set (spec §§3.1, 24.3 example):
/// the three classical roots plus the ML-DSA-44 root. RSA-2048 is
/// deliberately absent — it is the legacy opt-in, not a default.
pub const DEFAULT_ALGORITHMS: &[AlgorithmId] = &[
    AlgorithmId::Ed25519,
    AlgorithmId::P256,
    AlgorithmId::Rsa4096,
    AlgorithmId::MlDsa44,
];

/// The classical subset of the default set (trust bundle
/// `roots-classical`, spec §3.5).
pub const CLASSICAL_ALGORITHMS: &[AlgorithmId] = &[
    AlgorithmId::Ed25519,
    AlgorithmId::P256,
    AlgorithmId::Rsa4096,
];

/// Parse a comma-separated algorithm list (spec §6.3 `--algorithms`
/// replacement semantics). Order is preserved; warnings are collected
/// for the caller to surface. Duplicates are preserved — a repeated
/// algorithm is meaningful input (distinct partition keys, §24.4) and
/// each caller decides what duplicates mean in its context.
pub fn parse_algorithm_list(input: &str) -> Result<(Vec<AlgorithmId>, Vec<String>)> {
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    for part in input.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (algo, warn) = AlgorithmId::parse(part)?;
        if let Some(w) = warn {
            warnings.push(w);
        }
        out.push(algo);
    }
    if out.is_empty() {
        return Err(CertError::usage("empty algorithm list"));
    }
    Ok((out, warnings))
}

/// Enforce the CA policy gate: any legacy-gated algorithm in a CA
/// (root or issuer) set requires the explicit opt-in (spec §24.1(1)).
pub fn check_ca_policy(algorithms: &[AlgorithmId], allow_legacy: bool) -> Result<()> {
    for algo in algorithms {
        if matches!(algo.info().policy, PolicyClass::LegacyGated) && !allow_legacy {
            return Err(CertError::policy(format!(
                "algorithm {} is a legacy/compatibility profile: CA use requires \
                 the explicit --allow-legacy-root-strength opt-in (spec §24.1(1))",
                algo.name()
            )));
        }
        if !algo.is_supported() {
            return Err(CertError::policy(format!(
                "algorithm {} is registered as UNSUPPORTED in this build",
                algo.name()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_covers_the_spec_root_set() {
        let reg = registry();
        assert_eq!(reg.len(), 5);
        let defaults: Vec<&str> = DEFAULT_ALGORITHMS.iter().map(|a| a.name()).collect();
        assert_eq!(defaults, ["ed25519", "p256", "rsa-4096", "ml-dsa-44"]);
        // Exactly one PQ algorithm, and it carries the RFC 9881 OID.
        let pq: Vec<&AlgorithmInfo> = reg.iter().filter(|i| i.post_quantum).collect();
        assert_eq!(pq.len(), 1);
        assert_eq!(pq[0].signature_oid, "2.16.840.1.101.3.4.3.17");
    }

    #[test]
    fn aliases_normalize_and_dsa_is_rejected() {
        assert_eq!(AlgorithmId::parse("rsa").unwrap().0, AlgorithmId::Rsa4096);
        assert_eq!(AlgorithmId::parse("P-256").unwrap().0, AlgorithmId::P256);
        assert_eq!(AlgorithmId::parse("ed").unwrap().0, AlgorithmId::Ed25519);
        let (a, w) = AlgorithmId::parse("dilithium2").unwrap();
        assert_eq!(a, AlgorithmId::MlDsa44);
        assert!(w.is_some(), "dilithium2 alias must carry the warning");
        assert!(AlgorithmId::parse("dsa").is_err());
        assert!(AlgorithmId::parse("rsa-8192").is_err());
    }

    #[test]
    fn legacy_gate() {
        assert!(check_ca_policy(&[AlgorithmId::Ed25519], false).is_ok());
        assert!(check_ca_policy(&[AlgorithmId::Rsa2048], false).is_err());
        assert!(check_ca_policy(&[AlgorithmId::Rsa2048], true).is_ok());
    }

    #[test]
    fn list_parsing_preserves_duplicates() {
        let (list, _) = parse_algorithm_list("rsa,rsa,ed25519").unwrap();
        assert_eq!(
            list,
            [AlgorithmId::Rsa4096, AlgorithmId::Rsa4096, AlgorithmId::Ed25519]
        );
    }
}
