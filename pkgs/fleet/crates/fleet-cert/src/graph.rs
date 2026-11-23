//! The cross-signing graph as data/policy (spec §§3.2, 24.2).
//!
//! The graph is **never** a hardcoded four-by-four loop in the
//! engine. It is two explicit edge tables — one for root-level
//! cross-certificates (§3.2), one for directly cross-signed issuer
//! variants (§24.2) — plus a mode that selects how much of the table
//! a given creation run realizes. A future algorithm or a changed
//! policy edits the tables, not the engine.
//!
//! The invariant both tables encode: classical signers may certify
//! every subject (including the ML-DSA subject — a classical
//! cross-certificate over the ML-DSA key is how classical-only
//! clients reach the PQ hierarchy), while the ML-DSA signer
//! certifies **only** ML-DSA subjects. No ML-DSA→classical edge
//! exists, by policy: a PQ-enforced verifier trusting only the
//! ML-DSA root must never be handed a path in which the ML-DSA key
//! vouched for a classical key (§3.2 rationale).

use crate::algorithm::AlgorithmId;
use crate::error::{CertError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// How much of the graph a creation run realizes (spec §24.2:
/// `--cross-signing roots-only|issuers-direct|both|none`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CrossSigningMode {
    /// No cross-certificates at all; each key has only its primary
    /// (self-signed for roots, same-algorithm-signed for issuers).
    None,
    /// Cross-sign at the root level only; issuer keys get only their
    /// primary variant. Recommended where graph simplicity matters.
    RootsOnly,
    /// Issuer keys are directly cross-signed under every eligible
    /// root; no root-level cross-certificates.
    IssuersDirect,
    /// Both levels (the §24.2 reference-run shape).
    Both,
}

impl CrossSigningMode {
    pub fn parse(input: &str) -> Result<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "none" => Ok(CrossSigningMode::None),
            "roots-only" | "roots" => Ok(CrossSigningMode::RootsOnly),
            "issuers-direct" | "direct" => Ok(CrossSigningMode::IssuersDirect),
            "both" => Ok(CrossSigningMode::Both),
            other => Err(CertError::usage(format!(
                "unknown cross-signing mode '{other}' (none | roots-only | issuers-direct | both)"
            ))),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            CrossSigningMode::None => "none",
            CrossSigningMode::RootsOnly => "roots-only",
            CrossSigningMode::IssuersDirect => "issuers-direct",
            CrossSigningMode::Both => "both",
        }
    }

    pub fn root_level_active(&self) -> bool {
        matches!(self, CrossSigningMode::RootsOnly | CrossSigningMode::Both)
    }

    pub fn issuer_level_active(&self) -> bool {
        matches!(self, CrossSigningMode::IssuersDirect | CrossSigningMode::Both)
    }
}

/// One directed edge: `signer` may certify `subject`'s key.
pub type Edge = (AlgorithmId, AlgorithmId);

/// The policy tables. Rows are data; adding an algorithm or changing
/// who may sign whom is an edit here, reviewed like policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphPolicy {
    /// Root-level cross-sign edges (§3.2), excluding self-pairs
    /// (self-signing is not an edge, it is the primary certificate).
    pub root_edges: BTreeSet<Edge>,
    /// Issuer-level direct cross-sign edges (§24.2), excluding the
    /// primary same-algorithm signer.
    pub issuer_edges: BTreeSet<Edge>,
}

impl GraphPolicy {
    /// The spec default: §§3.2 + 24.2 matrices. Classical signers
    /// certify all subjects; the ML-DSA signer certifies only the
    /// ML-DSA subject.
    pub fn spec_default() -> Self {
        let classical = [
            AlgorithmId::Ed25519,
            AlgorithmId::P256,
            AlgorithmId::Rsa4096,
            AlgorithmId::Rsa2048,
        ];
        let all_subjects = [
            AlgorithmId::Ed25519,
            AlgorithmId::P256,
            AlgorithmId::Rsa4096,
            AlgorithmId::Rsa2048,
            AlgorithmId::MlDsa44,
        ];
        let mut root_edges = BTreeSet::new();
        let mut issuer_edges = BTreeSet::new();
        for signer in classical {
            for subject in all_subjects {
                if signer != subject {
                    root_edges.insert((signer, subject));
                    issuer_edges.insert((signer, subject));
                }
            }
        }
        // The ML-DSA signer's only non-self edge would be to a
        // classical subject — forbidden — so it contributes no edges
        // at all. (Its self-pair is the primary certificate.)
        GraphPolicy {
            root_edges,
            issuer_edges,
        }
    }

    /// May `signer` certify `subject` at the root level? Self-pairs
    /// are always allowed (they are the primary certificate, not a
    /// cross edge).
    pub fn root_allows(&self, signer: AlgorithmId, subject: AlgorithmId) -> bool {
        signer == subject || self.root_edges.contains(&(signer, subject))
    }

    /// May `signer` directly certify an issuer key of `subject`
    /// algorithm? The primary same-algorithm signer always may.
    pub fn issuer_allows(&self, signer: AlgorithmId, subject: AlgorithmId) -> bool {
        signer == subject || self.issuer_edges.contains(&(signer, subject))
    }

    /// The non-self signers eligible to cross-certify a root key of
    /// `subject` algorithm, in registry order.
    pub fn root_signers_for(&self, subject: AlgorithmId) -> Vec<AlgorithmId> {
        self.root_edges
            .iter()
            .filter(|(_, s)| *s == subject)
            .map(|(signer, _)| *signer)
            .collect()
    }

    /// The non-primary signers eligible to directly certify an issuer
    /// key of `subject` algorithm, in registry order.
    pub fn issuer_signers_for(&self, subject: AlgorithmId) -> Vec<AlgorithmId> {
        self.issuer_edges
            .iter()
            .filter(|(_, s)| *s == subject)
            .map(|(signer, _)| *signer)
            .collect()
    }

    /// Expected root-level certificate count for a root set with the
    /// given algorithms under `mode`: one self-signed per algorithm
    /// plus one cross-certificate per realized edge whose signer and
    /// subject are both present (§3.2 inventory: 4 + 9 = 13 for the
    /// default set).
    pub fn expected_root_certificates(
        &self,
        algorithms: &[AlgorithmId],
        mode: CrossSigningMode,
    ) -> usize {
        let mut n = algorithms.len();
        if mode.root_level_active() {
            n += self
                .root_edges
                .iter()
                .filter(|(signer, subject)| {
                    algorithms.contains(signer) && algorithms.contains(subject)
                })
                .count();
        }
        n
    }

    /// Expected issuer-variant count for one issuer key of `subject`
    /// algorithm under `mode`, given which root algorithms exist:
    /// the primary variant plus one per realized direct edge.
    pub fn expected_issuer_variants(
        &self,
        subject: AlgorithmId,
        root_algorithms: &[AlgorithmId],
        mode: CrossSigningMode,
    ) -> usize {
        let mut n = 1;
        if mode.issuer_level_active() {
            n += self
                .issuer_signers_for(subject)
                .iter()
                .filter(|signer| root_algorithms.contains(signer))
                .count();
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algorithm::DEFAULT_ALGORITHMS;

    #[test]
    fn spec_inventory_counts() {
        let g = GraphPolicy::spec_default();
        // §3.2: 4 self-signed + 9 cross = 13 root certificates.
        assert_eq!(
            g.expected_root_certificates(DEFAULT_ALGORITHMS, CrossSigningMode::Both),
            13
        );
        assert_eq!(
            g.expected_root_certificates(DEFAULT_ALGORITHMS, CrossSigningMode::RootsOnly),
            13
        );
        assert_eq!(
            g.expected_root_certificates(DEFAULT_ALGORITHMS, CrossSigningMode::None),
            4
        );
        // §24.2 per partition: classical issuer keys get 3 variants
        // under `both` when counted per signer-root presence in the
        // default root set (primary + 2 classical crosses... the
        // ML-DSA root may not sign them), ML-DSA issuer keys get 4.
        assert_eq!(
            g.expected_issuer_variants(AlgorithmId::Ed25519, DEFAULT_ALGORITHMS, CrossSigningMode::Both),
            3
        );
        assert_eq!(
            g.expected_issuer_variants(AlgorithmId::MlDsa44, DEFAULT_ALGORITHMS, CrossSigningMode::Both),
            4
        );
        assert_eq!(
            g.expected_issuer_variants(AlgorithmId::MlDsa44, DEFAULT_ALGORITHMS, CrossSigningMode::RootsOnly),
            1
        );
    }

    #[test]
    fn ml_dsa_never_signs_classical() {
        let g = GraphPolicy::spec_default();
        for subject in [
            AlgorithmId::Ed25519,
            AlgorithmId::P256,
            AlgorithmId::Rsa4096,
            AlgorithmId::Rsa2048,
        ] {
            assert!(!g.root_allows(AlgorithmId::MlDsa44, subject));
            assert!(!g.issuer_allows(AlgorithmId::MlDsa44, subject));
        }
        // Classical signers may certify the ML-DSA subject.
        assert!(g.root_allows(AlgorithmId::Ed25519, AlgorithmId::MlDsa44));
        assert!(g.issuer_allows(AlgorithmId::Rsa4096, AlgorithmId::MlDsa44));
    }

    #[test]
    fn mode_parsing() {
        assert_eq!(
            CrossSigningMode::parse("both").unwrap(),
            CrossSigningMode::Both
        );
        assert!(CrossSigningMode::parse("everything").is_err());
        assert!(CrossSigningMode::Both.root_level_active());
        assert!(CrossSigningMode::Both.issuer_level_active());
        assert!(!CrossSigningMode::RootsOnly.issuer_level_active());
    }
}
