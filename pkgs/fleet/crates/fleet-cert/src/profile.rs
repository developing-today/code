//! Leaf profiles, mirroring `pkgs/idp-pki`'s `idp-ca` (the interim
//! IdP CA tool) as the behavioural reference:
//!
//! | profile       | key        | key usage                        | EKU        | default validity |
//! |---------------|------------|----------------------------------|------------|------------------|
//! | `server`      | EC P-256   | digitalSignature                 | serverAuth | 730 days         |
//! | `client`      | EC P-256   | digitalSignature                 | clientAuth | 730 days         |
//! | `saml-signing`| RSA-2048   | digitalSignature, nonRepudiation (contentCommitment) | —          | 1095 days        |
//! | `oidc-signing`| RSA-2048   | digitalSignature, nonRepudiation (contentCommitment) | —          | 1095 days        |
//!
//! All leaves carry `basicConstraints CA:false` (critical) and
//! SKI/AKI, as idp-ca's extension files do. The `server` profile
//! defaults its SAN to `DNS:<cn>` when no SAN is supplied, again as
//! idp-ca does.
//!
//! Validity note: the per-profile defaults above are idp-ca's. The
//! spec's generic unprofiled leaf default is 90 days (§5.4); a
//! profile's own default governs when a profile is named, and
//! `--valid-for`/`--not-after` always override.

use crate::algorithm::AlgorithmId;
use crate::error::{CertError, Result};
use crate::timeparse::CalendarDuration;

/// A leaf certificate profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeafProfile {
    pub name: &'static str,
    /// The subject key algorithm the profile generates by default.
    pub key_algorithm: AlgorithmId,
    /// Key usage bits, as rcgen names.
    pub key_usages: &'static [rcgen::KeyUsagePurpose],
    /// Extended key usages (empty = no EKU extension, as in idp-ca's
    /// signing profiles).
    pub extended_key_usages: &'static [rcgen::ExtendedKeyUsagePurpose],
    /// Default validity when the caller gives no time flags.
    pub default_validity: CalendarDuration,
    /// idp-ca behaviour: a `server` leaf with no SANs gets
    /// `DNS:<cn>`.
    pub san_defaults_to_cn: bool,
    pub notes: &'static str,
}

const PROFILES: &[LeafProfile] = &[
    LeafProfile {
        name: "server",
        key_algorithm: AlgorithmId::P256,
        key_usages: &[rcgen::KeyUsagePurpose::DigitalSignature],
        extended_key_usages: &[rcgen::ExtendedKeyUsagePurpose::ServerAuth],
        default_validity: CalendarDuration::days(730),
        san_defaults_to_cn: true,
        notes: "TLS server leaf (idp-ca `server`): P-256, serverAuth, SAN defaults to DNS:<cn>.",
    },
    LeafProfile {
        name: "client",
        key_algorithm: AlgorithmId::P256,
        key_usages: &[rcgen::KeyUsagePurpose::DigitalSignature],
        extended_key_usages: &[rcgen::ExtendedKeyUsagePurpose::ClientAuth],
        default_validity: CalendarDuration::days(730),
        san_defaults_to_cn: false,
        notes: "TLS client leaf (idp-ca `client`): P-256, clientAuth.",
    },
    LeafProfile {
        name: "saml-signing",
        key_algorithm: AlgorithmId::Rsa2048,
        key_usages: &[
            rcgen::KeyUsagePurpose::DigitalSignature,
            rcgen::KeyUsagePurpose::ContentCommitment,
        ],
        extended_key_usages: &[],
        default_validity: CalendarDuration::days(1095),
        san_defaults_to_cn: false,
        notes: "SAML signing leaf (idp-ca `saml-signing`): RSA-2048, digitalSignature + nonRepudiation (contentCommitment), no EKU.",
    },
    LeafProfile {
        name: "oidc-signing",
        key_algorithm: AlgorithmId::Rsa2048,
        key_usages: &[
            rcgen::KeyUsagePurpose::DigitalSignature,
            rcgen::KeyUsagePurpose::ContentCommitment,
        ],
        extended_key_usages: &[],
        default_validity: CalendarDuration::days(1095),
        san_defaults_to_cn: false,
        notes: "OIDC signing leaf (idp-ca `oidc-signing`): RSA-2048, digitalSignature + nonRepudiation (contentCommitment), no EKU.",
    },
];

/// All leaf profiles, in registry order.
pub fn leaf_profiles() -> &'static [LeafProfile] {
    PROFILES
}

/// Look a profile up by name.
pub fn leaf_profile(name: &str) -> Result<&'static LeafProfile> {
    PROFILES
        .iter()
        .find(|p| p.name == name.trim().to_ascii_lowercase())
        .ok_or_else(|| {
            CertError::usage(format!(
                "unknown leaf profile '{name}' (known: {})",
                PROFILES
                    .iter()
                    .map(|p| p.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_mirror_idp_ca() {
        let server = leaf_profile("server").unwrap();
        assert_eq!(server.key_algorithm, AlgorithmId::P256);
        assert_eq!(server.default_validity.days, 730);
        assert!(server.san_defaults_to_cn);
        let saml = leaf_profile("saml-signing").unwrap();
        assert_eq!(saml.key_algorithm, AlgorithmId::Rsa2048);
        assert_eq!(saml.default_validity.days, 1095);
        assert!(saml.extended_key_usages.is_empty());
        assert_eq!(
            saml.key_usages,
            [
                rcgen::KeyUsagePurpose::DigitalSignature,
                rcgen::KeyUsagePurpose::ContentCommitment
            ]
        );
        assert!(leaf_profile("web").is_err());
    }
}
