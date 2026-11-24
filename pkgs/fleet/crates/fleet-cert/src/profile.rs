//! Leaf profiles, mirroring `pkgs/idp-pki`'s `idp-ca` (the interim
//! IdP CA tool) as the behavioural reference:
//!
//! | profile       | key        | key usage                        | EKU        | default validity |
//! |---------------|------------|----------------------------------|------------|------------------|
//! | `server`      | EC P-256   | digitalSignature                 | serverAuth | 730 days         |
//! | `client`      | EC P-256   | digitalSignature                 | clientAuth | 730 days         |
//! | `saml-signing`| RSA-2048   | digitalSignature, nonRepudiation (contentCommitment) | —          | 1095 days        |
//! | `oidc-signing`| RSA-2048   | digitalSignature, nonRepudiation (contentCommitment) | —          | 1095 days        |
//! | `ocsp-responder`| EC P-256 | digitalSignature                 | OCSP signing | 30 days          |
//!
//! All leaves carry `basicConstraints CA:false` (critical) and
//! SKI/AKI, as idp-ca's extension files do. The `server` profile
//! defaults its SAN to `DNS:<cn>` when no SAN is supplied, again as
//! idp-ca does.
//!
//! The `ocsp-responder` profile is the delegated OCSP signer
//! (RFC 6960 §3.2): EKU id-kp-OCSPSigning only, plus the
//! id-pkix-ocsp-nocheck extension. It is issued through the
//! ordinary leaf path and used by `fleet cert ocsp sign`.
//!
//! Validity note: the per-profile defaults above are idp-ca's. The
//! spec's generic unprofiled leaf default is 90 days (§5.4); a
//! profile's own default governs when a profile is named, and
//! `--valid-for`/`--not-after` always override.

use crate::algorithm::AlgorithmId;
use crate::error::{CertError, Result};
use crate::timeparse::CalendarDuration;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Maximum lifetime for custom leaf profiles. This matches the longest
/// built-in leaf policy; longer-lived credentials need a reviewed built-in.
pub const MAX_CUSTOM_VALIDITY_DAYS: u32 = 1095;

/// On-disk custom leaf profile format (JSON, schema v1).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomProfile {
    pub schema: String,
    pub name: String,
    pub key_algorithm: String,
    pub key_usages: Vec<String>,
    pub extended_key_usages: Vec<String>,
    pub default_validity_days: u32,
    pub max_validity_days: u32,
    #[serde(default)]
    pub san_defaults_to_cn: bool,
    #[serde(default)]
    pub notes: String,
}

/// Runtime profile values used by the signing engine. Constructed only
/// after resolving a built-in or validating a custom profile file.
#[derive(Debug, Clone)]
pub struct ResolvedProfile {
    pub name: String,
    pub key_algorithm: AlgorithmId,
    pub key_usages: Vec<rcgen::KeyUsagePurpose>,
    pub extended_key_usages: Vec<rcgen::ExtendedKeyUsagePurpose>,
    pub default_validity: CalendarDuration,
    pub max_validity_days: u32,
    pub san_defaults_to_cn: bool,
    pub ocsp_no_check: bool,
    pub is_custom: bool,
}

impl CustomProfile {
    pub fn scaffold(name: &str) -> Self {
        Self {
            schema: "fleet-cert-profile/v1".into(),
            name: name.into(),
            key_algorithm: "p256".into(),
            key_usages: vec!["digital-signature".into()],
            extended_key_usages: vec!["server-auth".into()],
            default_validity_days: 90,
            max_validity_days: 365,
            san_defaults_to_cn: true,
            notes: "Custom TLS server profile".into(),
        }
    }

    pub fn validate(&self) -> Result<ResolvedProfile> {
        if self.schema != "fleet-cert-profile/v1" {
            return Err(CertError::policy(format!(
                "unsupported profile schema '{}'",
                self.schema
            )));
        }
        if self.name.trim().is_empty()
            || self.name.len() > 64
            || !self
                .name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(CertError::policy(
                "profile name must be 1-64 lowercase ASCII letters, digits, or hyphens",
            ));
        }
        if leaf_profiles().iter().any(|p| p.name == self.name) {
            return Err(CertError::policy(format!(
                "custom profile name '{}' shadows a built-in profile",
                self.name
            )));
        }
        let (key_algorithm, _) = AlgorithmId::parse(&self.key_algorithm)?;
        let key_usages = self
            .key_usages
            .iter()
            .map(|s| parse_key_usage(s))
            .collect::<Result<Vec<_>>>()?;
        let extended_key_usages = self
            .extended_key_usages
            .iter()
            .map(|s| parse_extended_key_usage(s))
            .collect::<Result<Vec<_>>>()?;
        if key_usages.is_empty() {
            return Err(CertError::policy(
                "custom profile must declare at least one key usage",
            ));
        }
        if key_usages.iter().any(|usage| {
            matches!(
                usage,
                rcgen::KeyUsagePurpose::KeyCertSign | rcgen::KeyUsagePurpose::CrlSign
            )
        }) {
            return Err(CertError::policy(
                "custom leaf profiles cannot declare key-cert-sign or crl-sign",
            ));
        }
        if self.default_validity_days == 0 || self.default_validity_days > self.max_validity_days {
            return Err(CertError::policy(
                "default_validity_days must be positive and no greater than max_validity_days",
            ));
        }
        if self.max_validity_days > MAX_CUSTOM_VALIDITY_DAYS {
            return Err(CertError::policy(format!(
                "max_validity_days exceeds policy ceiling of {MAX_CUSTOM_VALIDITY_DAYS}"
            )));
        }
        Ok(ResolvedProfile {
            name: self.name.clone(),
            key_algorithm,
            key_usages,
            extended_key_usages,
            default_validity: CalendarDuration::days(i64::from(self.default_validity_days)),
            max_validity_days: self.max_validity_days,
            san_defaults_to_cn: self.san_defaults_to_cn,
            ocsp_no_check: false,
            is_custom: true,
        })
    }
}

fn parse_key_usage(value: &str) -> Result<rcgen::KeyUsagePurpose> {
    use rcgen::KeyUsagePurpose as K;
    match value {
        "digital-signature" => Ok(K::DigitalSignature),
        "content-commitment" => Ok(K::ContentCommitment),
        "key-encipherment" => Ok(K::KeyEncipherment),
        "data-encipherment" => Ok(K::DataEncipherment),
        "key-agreement" => Ok(K::KeyAgreement),
        "key-cert-sign" => Ok(K::KeyCertSign),
        "crl-sign" => Ok(K::CrlSign),
        "encipher-only" => Ok(K::EncipherOnly),
        "decipher-only" => Ok(K::DecipherOnly),
        _ => Err(CertError::policy(format!(
            "unsupported key usage '{value}'"
        ))),
    }
}

fn parse_extended_key_usage(value: &str) -> Result<rcgen::ExtendedKeyUsagePurpose> {
    use rcgen::ExtendedKeyUsagePurpose as E;
    match value {
        "server-auth" => Ok(E::ServerAuth),
        "client-auth" => Ok(E::ClientAuth),
        "code-signing" => Ok(E::CodeSigning),
        "email-protection" => Ok(E::EmailProtection),
        "time-stamping" => Ok(E::TimeStamping),
        "ocsp-signing" => Ok(E::OcspSigning),
        _ => Err(CertError::policy(format!(
            "unsupported extended key usage '{value}'"
        ))),
    }
}

pub fn load_custom_profile(path: &Path) -> Result<(CustomProfile, ResolvedProfile)> {
    let bytes = std::fs::read(path)
        .map_err(|e| CertError::manifest(format!("cannot read profile {}: {e}", path.display())))?;
    let profile: CustomProfile = serde_json::from_slice(&bytes)
        .map_err(|e| CertError::manifest(format!("malformed custom profile: {e}")))?;
    let resolved = profile.validate()?;
    Ok((profile, resolved))
}

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
    /// RFC 6960 §3.2 NoCheck: a delegated OCSP signing
    /// certificate carries id-pkix-ocsp-nocheck so verifiers do
    /// not try to revocation-check the responder itself. Only
    /// the `ocsp-responder` profile sets this.
    pub ocsp_no_check: bool,
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
        ocsp_no_check: false,
        notes: "TLS server leaf (idp-ca `server`): P-256, serverAuth, SAN defaults to DNS:<cn>.",
    },
    LeafProfile {
        name: "client",
        key_algorithm: AlgorithmId::P256,
        key_usages: &[rcgen::KeyUsagePurpose::DigitalSignature],
        extended_key_usages: &[rcgen::ExtendedKeyUsagePurpose::ClientAuth],
        default_validity: CalendarDuration::days(730),
        san_defaults_to_cn: false,
        ocsp_no_check: false,
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
        ocsp_no_check: false,
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
        ocsp_no_check: false,
        notes: "OIDC signing leaf (idp-ca `oidc-signing`): RSA-2048, digitalSignature + nonRepudiation (contentCommitment), no EKU.",
    },
    LeafProfile {
        name: "ocsp-responder",
        key_algorithm: AlgorithmId::P256,
        key_usages: &[rcgen::KeyUsagePurpose::DigitalSignature],
        extended_key_usages: &[rcgen::ExtendedKeyUsagePurpose::OcspSigning],
        default_validity: CalendarDuration::days(30),
        san_defaults_to_cn: false,
        ocsp_no_check: true,
        notes: "Delegated OCSP responder (RFC 6960 §3.2): P-256, digitalSignature, EKU id-kp-OCSPSigning only, OCSP NoCheck extension, 30d default validity. Issue with `leaf issue --profile ocsp-responder`; sign with `fleet cert ocsp sign --responder`.",
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

pub fn resolve_profile(name_or_path: &str) -> Result<ResolvedProfile> {
    if let Ok(profile) = leaf_profile(name_or_path) {
        return Ok(ResolvedProfile {
            name: profile.name.into(),
            key_algorithm: profile.key_algorithm,
            key_usages: profile.key_usages.to_vec(),
            extended_key_usages: profile.extended_key_usages.to_vec(),
            default_validity: profile.default_validity,
            // The custom-profile ceiling is not a new constraint on the
            // established built-in profiles and their existing callers.
            max_validity_days: u32::MAX,
            san_defaults_to_cn: profile.san_defaults_to_cn,
            ocsp_no_check: profile.ocsp_no_check,
            is_custom: false,
        });
    }
    load_custom_profile(Path::new(name_or_path)).map(|(_, profile)| profile)
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

    #[test]
    fn custom_profile_schema_and_policy_are_enforced() {
        let profile = CustomProfile::scaffold("device-auth");
        let resolved = profile.validate().expect("scaffold is valid by policy");
        assert_eq!(resolved.key_algorithm, AlgorithmId::P256);
        assert_eq!(resolved.max_validity_days, 365);

        let mut invalid = profile.clone();
        invalid.key_usages = vec!["not-a-usage".into()];
        assert!(
            invalid
                .validate()
                .unwrap_err()
                .to_string()
                .contains("unsupported key usage")
        );
        let mut invalid = profile.clone();
        invalid.extended_key_usages = vec!["unknown-purpose".into()];
        assert!(
            invalid
                .validate()
                .unwrap_err()
                .to_string()
                .contains("unsupported extended key usage")
        );
        let mut invalid = profile.clone();
        invalid.max_validity_days = MAX_CUSTOM_VALIDITY_DAYS + 1;
        assert!(
            invalid
                .validate()
                .unwrap_err()
                .to_string()
                .contains("policy ceiling")
        );
        let mut invalid = profile;
        invalid.schema = "fleet-cert-profile/v99".into();
        assert!(
            invalid
                .validate()
                .unwrap_err()
                .to_string()
                .contains("unsupported profile schema")
        );

        let mut invalid = CustomProfile::scaffold("ca-usage");
        invalid.key_usages = vec!["key-cert-sign".into()];
        assert!(
            invalid
                .validate()
                .unwrap_err()
                .to_string()
                .contains("leaf profiles")
        );
    }
}
