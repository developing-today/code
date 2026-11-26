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
//! | `broker-server` | EC P-256 | digitalSignature                 | serverAuth | 90 days          |
//! | `broker-client` | EC P-256 | digitalSignature                 | clientAuth | 90 days          |
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
//! The two `broker-*` profiles are the fleet brokers' TLS leaves
//! (broker-TLS design of record, ADR-0001, slice BT-1). They differ
//! from `server`/`client` in four ways, all enforced at issuance:
//!
//! - **Issuer:** the Base issuer set only (purpose `general`); either
//!   A/B partition signs.
//! - **Key custody:** the key is generated on the broker's host and
//!   only a CSR reaches the issuer (`leaf issue --csr`). Issuer-side
//!   key generation is refused, so a broker leaf key is never
//!   written into a bundle or an external store: it is the
//!   machine-local leaf-key credential class (never exported,
//!   escrowed or replicated; a lost key is replaced by re-issuing).
//! - **SANs ([`SanPolicy::Broker`]):** at least one internal DNS name
//!   under `.internal` and at least one tailnet IP SAN; wildcards,
//!   public names and non-tailnet IPs are refused. The CN, when
//!   given, must be one of the DNS names.
//! - **Lifetime and revocation:** 90 days, renewed automatically once
//!   [`LeafProfile::renew_within`] of the lifetime is left; checked
//!   by CRL only. No OCSP URL and no AIA is embedded, and no CDP
//!   either until a real signed CRL is published for the Base set.
//!
//! ML-DSA broker leaves are deferred: the profiles fix P-256, and a
//! CSR with any other key is refused.
//!
//! Validity note: the per-profile defaults above are idp-ca's. The
//! spec's generic unprofiled leaf default is 90 days (§5.4); a
//! profile's own default governs when a profile is named, and
//! `--valid-for`/`--not-after` always override.

use std::net::IpAddr;

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
    /// Which SANs the profile accepts (custom profiles: [`SanPolicy::Free`]).
    pub san_policy: SanPolicy,
    /// Where the subject key comes from (custom profiles:
    /// [`KeyCustody::IssuerOrCsr`]).
    pub key_custody: KeyCustody,
    /// The issuer set purpose the profile must issue from (`None` = any;
    /// custom profiles: `None`).
    pub issuer_purpose: Option<&'static str>,
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
            san_policy: SanPolicy::Free,
            key_custody: KeyCustody::IssuerOrCsr,
            issuer_purpose: None,
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

/// Which SANs a profile accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SanPolicy {
    /// Whatever the request names (the idp-ca profiles).
    Free,
    /// Broker leaves: internal DNS names plus a tailnet IP, nothing
    /// else (see [`check_broker_sans`]).
    Broker,
}

/// Where a profile's subject key comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCustody {
    /// The issuer generates the key and stores it with the leaf
    /// (bundle-private or an external store), or the request brings
    /// a CSR.
    IssuerOrCsr,
    /// The key is generated on the subject's host and only a CSR is
    /// accepted; issuer-side generation is refused.
    HostCsrOnly,
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
    /// Which SANs the profile accepts.
    pub san_policy: SanPolicy,
    /// Where the subject key comes from.
    pub key_custody: KeyCustody,
    /// The issuer set purpose the profile must issue from (`None` =
    /// any). The broker profiles require the Base set (`general`).
    pub issuer_purpose: Option<&'static str>,
    /// Automated-renewal contract: renew once this much of the
    /// validity is left (`None` = no renewal contract).
    pub renew_within: Option<CalendarDuration>,
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
        san_policy: SanPolicy::Free,
        key_custody: KeyCustody::IssuerOrCsr,
        issuer_purpose: None,
        renew_within: None,
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
        san_policy: SanPolicy::Free,
        key_custody: KeyCustody::IssuerOrCsr,
        issuer_purpose: None,
        renew_within: None,
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
        san_policy: SanPolicy::Free,
        key_custody: KeyCustody::IssuerOrCsr,
        issuer_purpose: None,
        renew_within: None,
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
        san_policy: SanPolicy::Free,
        key_custody: KeyCustody::IssuerOrCsr,
        issuer_purpose: None,
        renew_within: None,
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
        san_policy: SanPolicy::Free,
        key_custody: KeyCustody::IssuerOrCsr,
        issuer_purpose: None,
        renew_within: None,
        notes: "Delegated OCSP responder (RFC 6960 §3.2): P-256, digitalSignature, EKU id-kp-OCSPSigning only, OCSP NoCheck extension, 30d default validity. Issue with `leaf issue --profile ocsp-responder`; sign with `fleet cert ocsp sign --responder`.",
    },
    LeafProfile {
        name: "broker-server",
        key_algorithm: AlgorithmId::P256,
        key_usages: &[rcgen::KeyUsagePurpose::DigitalSignature],
        extended_key_usages: &[rcgen::ExtendedKeyUsagePurpose::ServerAuth],
        default_validity: CalendarDuration::days(90),
        san_defaults_to_cn: false,
        ocsp_no_check: false,
        san_policy: SanPolicy::Broker,
        key_custody: KeyCustody::HostCsrOnly,
        issuer_purpose: Some("general"),
        renew_within: Some(CalendarDuration::days(30)),
        notes: "Fleet broker TLS server leaf (broker-TLS ADR-0001): P-256, serverAuth, 90d, renewed with 30d left, Base issuer set only, CSR from an on-host key only, SANs = .internal DNS names + a tailnet IP, CRL-only revocation (no OCSP or AIA).",
    },
    LeafProfile {
        name: "broker-client",
        key_algorithm: AlgorithmId::P256,
        key_usages: &[rcgen::KeyUsagePurpose::DigitalSignature],
        extended_key_usages: &[rcgen::ExtendedKeyUsagePurpose::ClientAuth],
        default_validity: CalendarDuration::days(90),
        san_defaults_to_cn: false,
        ocsp_no_check: false,
        san_policy: SanPolicy::Broker,
        key_custody: KeyCustody::HostCsrOnly,
        issuer_purpose: Some("general"),
        renew_within: Some(CalendarDuration::days(30)),
        notes: "Fleet broker TLS client leaf (broker-TLS ADR-0001): P-256, clientAuth, 90d, renewed with 30d left, Base issuer set only, CSR from an on-host key only, SANs = .internal DNS names + a tailnet IP, CRL-only revocation (no OCSP or AIA).",
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

/// The tailnet's address ranges: Tailscale assigns IPv4 from the
/// CGNAT block 100.64.0.0/10 and IPv6 from fd7a:115c:a1e0::/48.
pub fn is_tailnet_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            o[0] == 100 && (o[1] & 0xc0) == 0x40
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            s[0] == 0xfd7a && s[1] == 0x115c && s[2] == 0xa1e0
        }
    }
}

/// One internal DNS name: lower-case LDH labels ending in the
/// program's `.internal` zone, no wildcard, not an IP literal.
fn check_internal_dns(name: &str) -> std::result::Result<(), String> {
    if name.contains('*') {
        return Err(format!(
            "'{name}' is a wildcard; broker leaves name one host"
        ));
    }
    if name.parse::<IpAddr>().is_ok() {
        return Err(format!(
            "'{name}' is an IP address; pass it with --ip, not --dns"
        ));
    }
    let lower = name.trim_end_matches('.');
    if lower != lower.to_ascii_lowercase() {
        return Err(format!("'{name}' must be lower case"));
    }
    if lower.len() > 253 {
        return Err(format!("'{name}' is longer than 253 characters"));
    }
    let Some(host) = lower.strip_suffix(".internal") else {
        return Err(format!(
            "'{name}' is not under .internal; broker leaves carry no public names"
        ));
    };
    if host.is_empty() {
        return Err(format!(
            "'{name}' names the .internal zone itself, not a host"
        ));
    }
    for label in host.split('.') {
        let ok = !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !ok {
            return Err(format!("'{name}' has an invalid label '{label}'"));
        }
    }
    Ok(())
}

/// The broker SAN policy (broker-TLS design §4): at least one
/// internal DNS name and at least one tailnet IP; every DNS name is
/// internal, every IP is on the tailnet, and the CN (when given) is
/// one of the DNS names. Refused as a policy error, before anything
/// is signed.
pub fn check_broker_sans(
    profile: &str,
    common_name: Option<&str>,
    dns_names: &[String],
    ip_addresses: &[IpAddr],
) -> Result<()> {
    let refuse = |why: String| CertError::policy(format!("{profile}: {why}"));
    if dns_names.is_empty() {
        return Err(refuse(
            "needs at least one internal DNS name (--dns <host>.internal)".into(),
        ));
    }
    for name in dns_names {
        check_internal_dns(name).map_err(&refuse)?;
    }
    if ip_addresses.is_empty() {
        return Err(refuse(
            "needs the broker's tailnet IP as an IP SAN (--ip <100.64.0.0/10 or fd7a:115c:a1e0::/48 address>)"
                .into(),
        ));
    }
    for ip in ip_addresses {
        if !is_tailnet_ip(ip) {
            return Err(refuse(format!(
                "IP {ip} is not a tailnet address (100.64.0.0/10 or fd7a:115c:a1e0::/48)"
            )));
        }
    }
    if let Some(cn) = common_name {
        if !dns_names.iter().any(|d| d == cn) {
            return Err(refuse(format!(
                "CN '{cn}' must be one of the leaf's DNS names"
            )));
        }
    }
    Ok(())
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
            san_policy: profile.san_policy,
            key_custody: profile.key_custody,
            issuer_purpose: profile.issuer_purpose,
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
    fn broker_profiles_follow_the_design() {
        for (name, eku) in [
            ("broker-server", rcgen::ExtendedKeyUsagePurpose::ServerAuth),
            ("broker-client", rcgen::ExtendedKeyUsagePurpose::ClientAuth),
        ] {
            let p = leaf_profile(name).unwrap();
            assert_eq!(p.key_algorithm, AlgorithmId::P256);
            assert_eq!(p.key_usages, [rcgen::KeyUsagePurpose::DigitalSignature]);
            assert_eq!(p.extended_key_usages, [eku]);
            assert_eq!(p.default_validity.days, 90);
            assert_eq!(p.renew_within.map(|d| d.days), Some(30));
            assert_eq!(p.san_policy, SanPolicy::Broker);
            assert_eq!(p.key_custody, KeyCustody::HostCsrOnly);
            assert_eq!(p.issuer_purpose, Some("general"));
            assert!(!p.san_defaults_to_cn);
            assert!(!p.ocsp_no_check);
        }
    }

    #[test]
    fn tailnet_ranges() {
        for ok in [
            "100.64.0.1",
            "100.99.113.103",
            "100.127.255.254",
            "fd7a:115c:a1e0::1",
        ] {
            assert!(is_tailnet_ip(&ok.parse().unwrap()), "{ok}");
        }
        for bad in [
            "100.63.255.255",
            "100.128.0.1",
            "10.0.0.1",
            "8.8.8.8",
            "fd7a:115c:a1e1::1",
            "::1",
        ] {
            assert!(!is_tailnet_ip(&bad.parse().unwrap()), "{bad}");
        }
    }

    #[test]
    fn broker_san_policy() {
        let dns = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let ip: IpAddr = "100.99.113.103".parse().unwrap();
        let good = dns(&["queue-broker.fleet.internal"]);
        check_broker_sans("broker-server", None, &good, &[ip]).unwrap();
        check_broker_sans(
            "broker-server",
            Some("queue-broker.fleet.internal"),
            &good,
            &[ip, "fd7a:115c:a1e0::5".parse().unwrap()],
        )
        .unwrap();
        let refused = |cn: Option<&str>, d: &[&str], ips: &[&str]| {
            let ips: Vec<IpAddr> = ips.iter().map(|s| s.parse().unwrap()).collect();
            check_broker_sans("broker-server", cn, &dns(d), &ips).is_err()
        };
        let tip = "100.99.113.103";
        assert!(refused(None, &[], &[tip]), "no DNS name");
        assert!(
            refused(None, &["queue-broker.fleet.internal"], &[]),
            "no IP"
        );
        assert!(refused(None, &["*.fleet.internal"], &[tip]), "wildcard");
        assert!(
            refused(None, &["broker.example.com"], &[tip]),
            "public name"
        );
        assert!(refused(None, &["internal"], &[tip]), "bare zone");
        assert!(refused(None, &[".internal"], &[tip]), "empty label");
        assert!(refused(None, &["Broker.internal"], &[tip]), "upper case");
        assert!(refused(None, &["-bad.internal"], &[tip]), "leading hyphen");
        assert!(refused(None, &["a_b.internal"], &[tip]), "underscore");
        assert!(refused(None, &["100.99.113.103"], &[tip]), "IP as DNS");
        assert!(refused(None, &["b.internal"], &["192.168.1.4"]), "LAN IP");
        assert!(
            refused(None, &["b.internal"], &[tip, "8.8.8.8"]),
            "public IP"
        );
        assert!(
            refused(Some("broker.example.com"), &["b.internal"], &[tip]),
            "CN outside the SANs"
        );
    }

    #[test]
    fn custom_profile_schema_and_policy_are_enforced() {
        let profile = CustomProfile::scaffold("device-auth");
        let resolved = profile.validate().expect("scaffold is valid by policy");
        assert_eq!(resolved.key_algorithm, AlgorithmId::P256);
        assert_eq!(resolved.max_validity_days, 365);

        let mut invalid = profile.clone();
        invalid.key_usages = vec!["not-a-usage".into()];
        assert!(invalid
            .validate()
            .unwrap_err()
            .to_string()
            .contains("unsupported key usage"));
        let mut invalid = profile.clone();
        invalid.extended_key_usages = vec!["unknown-purpose".into()];
        assert!(invalid
            .validate()
            .unwrap_err()
            .to_string()
            .contains("unsupported extended key usage"));
        let mut invalid = profile.clone();
        invalid.max_validity_days = MAX_CUSTOM_VALIDITY_DAYS + 1;
        assert!(invalid
            .validate()
            .unwrap_err()
            .to_string()
            .contains("policy ceiling"));
        let mut invalid = profile;
        invalid.schema = "fleet-cert-profile/v99".into();
        assert!(invalid
            .validate()
            .unwrap_err()
            .to_string()
            .contains("unsupported profile schema"));

        let mut invalid = CustomProfile::scaffold("ca-usage");
        invalid.key_usages = vec!["key-cert-sign".into()];
        assert!(invalid
            .validate()
            .unwrap_err()
            .to_string()
            .contains("leaf profiles"));
    }
}
