//! X.509 v2 CRL generation, inspection and verification (RFC 5280 §§5.1–5.3).
//!
//! CRLs are signed by an explicitly selected CA key and contain revocation
//! records carried by the supplied, checksum-verified bundles. This module
//! does not publish CRLs or invent distribution URLs.

use crate::bundle::{load_bundle, pem_decode_all};
use crate::chain::{CertInfo, hex_bytes, parse_cert, verify_signed_bytes};
use crate::engine::{CaSigner, load_signer_key};
use crate::error::{CertError, Result};
use crate::model::{BundleType, CertRecord, SignerSource};
use crate::timeparse::Rfc3339;
use rcgen::{
    BasicConstraints, CertificateParams, IsCa, Issuer, KeyIdMethod, KeyUsagePurpose,
    RevocationReason, RevokedCertParams, SerialNumber,
};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use x509_parser::prelude::FromDer;

#[derive(Debug, Clone)]
pub struct GenerateRequest {
    pub issuer_bundle: PathBuf,
    pub signer: SignerSource,
    pub revocation_bundles: Vec<PathBuf>,
    pub this_update: Rfc3339,
    pub next_update: Rfc3339,
    /// Monotonically increasing CRL number chosen by the issuer operator.
    pub crl_number: u64,
    /// Optional prior CRL used to reject a non-increasing number.
    pub previous_crl: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RevokedEntry {
    pub serial_hex: String,
    pub revocation_time: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CrlInfo {
    pub issuer: String,
    pub crl_number: Option<String>,
    pub this_update: String,
    pub next_update: Option<String>,
    pub revoked: Vec<RevokedEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VerifyReport {
    pub valid: bool,
    pub signature_valid: bool,
    pub issuer_matches: bool,
    pub fresh: bool,
    pub revocations_match: bool,
    pub checks: Vec<String>,
}

fn issuer_record<'a>(
    bundle: &'a crate::bundle::Bundle,
    selector: Option<&str>,
) -> Result<&'a CertRecord> {
    let records: Vec<_> = bundle
        .manifest
        .certificates
        .iter()
        .filter(|r| matches!(r.role.as_str(), "issuer" | "root"))
        .filter(|r| r.primary || r.role == "root")
        .collect();
    let matches: Vec<_> = if let Some(selector) = selector {
        records
            .into_iter()
            .filter(|r| {
                r.fingerprint_sha256.eq_ignore_ascii_case(selector)
                    || r.algorithm.name().eq_ignore_ascii_case(selector)
                    || r.subject_dn == selector
            })
            .collect()
    } else {
        records
    };
    match matches.as_slice() {
        [record] => Ok(record),
        [] if selector.is_some() => Err(CertError::usage(format!(
            "no primary CA matches --issuer '{}'",
            selector.unwrap_or_default()
        ))),
        [] => Err(CertError::manifest(
            "issuer bundle contains no primary CA certificate",
        )),
        _ => Err(CertError::usage(
            "issuer selection is ambiguous; select a CA by fingerprint",
        )),
    }
}

fn cert_info(bundle: &crate::bundle::Bundle, record: &CertRecord) -> Result<CertInfo> {
    let pem = std::fs::read(bundle.dir.join(&record.cert_path))?;
    let blocks = pem_decode_all(
        std::str::from_utf8(&pem)
            .map_err(|_| CertError::manifest("issuer certificate is not UTF-8 PEM"))?,
    )?;
    let der = blocks
        .into_iter()
        .find(|(label, _)| label == "CERTIFICATE")
        .map(|(_, der)| der)
        .ok_or_else(|| CertError::manifest("issuer bundle has no CERTIFICATE block"))?;
    let info = parse_cert(&der)?;
    if !info
        .fingerprint_sha256
        .eq_ignore_ascii_case(&record.fingerprint_sha256)
        || !info.serial_hex.eq_ignore_ascii_case(&record.serial_hex)
    {
        return Err(CertError::manifest(format!(
            "certificate record does not match certificate file {}",
            record.cert_path
        )));
    }
    Ok(info)
}

fn revocation_matches_bundle(
    bundle: &crate::bundle::Bundle,
    entry: &crate::model::RevocationRecord,
    issuer: &CertInfo,
) -> Result<bool> {
    for record in &bundle.manifest.certificates {
        if !record.serial_hex.eq_ignore_ascii_case(&entry.serial_hex)
            || !record
                .fingerprint_sha256
                .eq_ignore_ascii_case(&entry.fingerprint_sha256)
        {
            continue;
        }
        let certificate = cert_info(bundle, record)?;
        return Ok(certificate.issuer_name_der == issuer.subject_name_der);
    }
    Ok(false)
}

pub(crate) fn read_crl(path: &Path) -> Result<Vec<u8>> {
    let bytes = std::fs::read(path)?;
    if bytes
        .iter()
        .copied()
        .skip_while(u8::is_ascii_whitespace)
        .take(10)
        .collect::<Vec<_>>()
        .starts_with(b"-----BEGIN")
    {
        let text =
            std::str::from_utf8(&bytes).map_err(|_| CertError::manifest("CRL PEM is not UTF-8"))?;
        pem_decode_all(text)?
            .into_iter()
            .find(|(label, _)| label == "X509 CRL" || label == "CRL")
            .map(|(_, der)| der)
            .ok_or_else(|| CertError::manifest("PEM file contains no X509 CRL block"))
    } else {
        Ok(bytes)
    }
}

fn cn_and_ou(dn: &str, prefix: &str) -> Option<String> {
    dn.split(", ")
        .find_map(|part| part.strip_prefix(prefix).map(str::to_owned))
}

fn signer_file(record: &CertRecord) -> String {
    record
        .key_path
        .as_deref()
        .and_then(|p| Path::new(p).file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| {
            if record.role == "root" {
                format!("{}.pk8", record.algorithm.name())
            } else {
                format!("{}-{}.pk8", record.algorithm.name(), record.partition)
            }
        })
}

fn reason(name: &str) -> Result<Option<RevocationReason>> {
    Ok(Some(match name {
        "unspecified" => RevocationReason::Unspecified,
        "key-compromise" => RevocationReason::KeyCompromise,
        "ca-compromise" => RevocationReason::CaCompromise,
        "affiliation-changed" => RevocationReason::AffiliationChanged,
        "superseded" => RevocationReason::Superseded,
        "cessation-of-operation" => RevocationReason::CessationOfOperation,
        "privilege-withdrawn" => RevocationReason::PrivilegeWithdrawn,
        other => {
            return Err(CertError::manifest(format!(
                "unsupported CRL reason '{other}'"
            )));
        }
    }))
}

fn reason_code(name: &str) -> Option<u8> {
    Some(match name {
        "unspecified" => 0,
        "key-compromise" => 1,
        "ca-compromise" => 2,
        "affiliation-changed" => 3,
        "superseded" => 4,
        "cessation-of-operation" => 5,
        "privilege-withdrawn" => 9,
        _ => return None,
    })
}

/// Generate one CRL for a primary issuer/root certificate. Revocation bundles
/// may include leaf deliveries (issuer CRL) or CA sets (root CRL); entries are
/// selected by issuer DN so cross-signed certificates are attributable to the
/// CA key that signed them.
pub fn generate(req: &GenerateRequest, selector: Option<&str>) -> Result<Vec<u8>> {
    if req.crl_number == 0 {
        return Err(CertError::usage("CRL number must be positive"));
    }
    if req.next_update <= req.this_update {
        return Err(CertError::usage("CRL --next-update must be after --at"));
    }
    let bundle = load_bundle(&req.issuer_bundle)?;
    if !matches!(
        bundle.manifest.bundle_type,
        BundleType::IssuerSet | BundleType::RootSet
    ) {
        return Err(CertError::usage(
            "CRL issuer must be a root-set or issuer-set bundle",
        ));
    }
    let record = issuer_record(&bundle, selector)?;
    let info = cert_info(&bundle, record)?;
    if !info.is_ca || !info.crl_sign {
        return Err(CertError::policy(
            "selected issuer certificate is not a CRL-signing CA",
        ));
    }
    if req.this_update.0 < info.not_before.0 || req.this_update.0 > info.not_after.0 {
        return Err(CertError::policy(
            "CRL thisUpdate is outside the issuer certificate validity period",
        ));
    }
    let signer = load_signer_key(&req.signer, &signer_file(record), record.algorithm)?;
    if !signer.public_key_matches(&info.spki_bytes) {
        return Err(CertError::policy(
            "signing key does not match the selected issuer certificate",
        ));
    }
    if let Some(previous_path) = &req.previous_crl {
        let previous_der = read_crl(previous_path)?;
        let (_, previous) =
            x509_parser::revocation_list::CertificateRevocationList::from_der(&previous_der)
                .map_err(|e| CertError::manifest(format!("previous CRL parse failed: {e}")))?;
        if previous.issuer().as_raw() != info.subject_name_der {
            return Err(CertError::usage(
                "previous CRL issuer does not match the selected issuer certificate",
            ));
        }
        let previous_number = previous
            .crl_number()
            .ok_or_else(|| CertError::manifest("previous CRL has no CRL number"))?
            .to_string()
            .parse::<u64>();
        if previous_number.map_or(true, |number| req.crl_number <= number) {
            return Err(CertError::usage(
                "CRL number must be greater than the previous CRL number",
            ));
        }
    }
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("empty SAN list");
    params.distinguished_name = crate::engine::distinguished_name(
        &bundle.manifest.organization,
        &bundle.manifest.country,
        cn_and_ou(&record.subject_dn, "OU=").as_deref(),
        &cn_and_ou(&record.subject_dn, "CN=").unwrap_or_else(|| record.subject_dn.clone()),
    );
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params.not_before = info.not_before.0;
    params.not_after = info.not_after.0;
    let mut revoked = Vec::new();
    let mut seen_serials = BTreeSet::new();
    for path in &req.revocation_bundles {
        let evidence = load_bundle(path)?;
        for entry in &evidence.manifest.revocations {
            // For an issuing CA, its CRL covers only certificates it issued;
            // for roots, each recorded cert's issuer DN must match this root.
            let record_match = revocation_matches_bundle(&bundle, entry, &info)?;
            let evidence_match = revocation_matches_bundle(&evidence, entry, &info)?;
            if (record_match || evidence_match) && entry.revoked_at <= req.this_update {
                let serial = decode_hex(&entry.serial_hex)?;
                let serial_key = hex_bytes(&serial);
                if !seen_serials.insert(serial_key) {
                    return Err(CertError::manifest(format!(
                        "duplicate revocation record for serial {}",
                        entry.serial_hex
                    )));
                }
                revoked.push(RevokedCertParams {
                    serial_number: SerialNumber::from(serial),
                    revocation_time: entry.revoked_at.0,
                    reason_code: reason(&entry.reason)?,
                    invalidity_date: None,
                });
            }
        }
    }
    revoked.sort_by(|a, b| a.serial_number.to_bytes().cmp(&b.serial_number.to_bytes()));
    let crl_params = rcgen::CertificateRevocationListParams {
        this_update: req.this_update.0,
        next_update: req.next_update.0,
        crl_number: SerialNumber::from(req.crl_number),
        issuing_distribution_point: None,
        revoked_certs: revoked,
        key_identifier_method: KeyIdMethod::Sha256,
    };
    let crl = match &signer {
        CaSigner::Classical(key) => crl_params.signed_by(&Issuer::new(params.clone(), key))?,
        CaSigner::MlDsa(key) => crl_params.signed_by(&Issuer::new(params, key.as_ref()))?,
    };
    let der = crl.der().as_ref().to_vec();
    let (_, parsed) = x509_parser::revocation_list::CertificateRevocationList::from_der(&der)
        .map_err(|e| CertError::engine(format!("generated CRL parse failed: {e}")))?;
    if parsed.issuer().as_raw() != info.subject_name_der {
        return Err(CertError::policy(
            "generated CRL issuer name does not match the selected certificate",
        ));
    }
    Ok(der)
}

fn decode_hex(text: &str) -> Result<Vec<u8>> {
    if text.is_empty() || text.len() % 2 != 0 {
        return Err(CertError::manifest(format!("odd serial hex '{text}'")));
    }
    if !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CertError::manifest(format!("invalid serial hex '{text}'")));
    }
    let bytes: Vec<u8> = (0..text.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&text[i..i + 2], 16)
                .map_err(|_| CertError::manifest(format!("invalid serial hex '{text}'")))
        })
        .collect::<Result<_>>()?;
    if bytes.len() > 20 {
        return Err(CertError::manifest("certificate serial exceeds 20 octets"));
    }
    Ok(bytes)
}

pub fn inspect(der: &[u8]) -> Result<CrlInfo> {
    let (remaining, crl) = x509_parser::revocation_list::CertificateRevocationList::from_der(der)
        .map_err(|e| CertError::manifest(format!("CRL parse failed: {e}")))?;
    if !remaining.is_empty() {
        return Err(CertError::manifest("trailing bytes after CRL DER"));
    }
    let revoked = crl
        .iter_revoked_certificates()
        .map(|entry| RevokedEntry {
            serial_hex: hex_bytes(
                &entry
                    .raw_serial()
                    .iter()
                    .skip_while(|b| **b == 0)
                    .copied()
                    .collect::<Vec<_>>(),
            ),
            revocation_time: entry
                .revocation_date
                .to_datetime()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_default(),
            reason: entry.reason_code().map(|(_, reason)| reason.to_string()),
        })
        .collect();
    Ok(CrlInfo {
        issuer: crl.issuer().to_string(),
        crl_number: crl.crl_number().map(ToString::to_string),
        this_update: crl
            .last_update()
            .to_datetime()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default(),
        next_update: crl.next_update().map(|t| {
            t.to_datetime()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_default()
        }),
        revoked,
    })
}

pub fn verify(
    der: &[u8],
    issuer_bundle: &Path,
    evidence_paths: &[PathBuf],
    at: Rfc3339,
) -> Result<VerifyReport> {
    let bundle = load_bundle(issuer_bundle)?;
    let (remaining, crl) = x509_parser::revocation_list::CertificateRevocationList::from_der(der)
        .map_err(|e| CertError::manifest(format!("CRL parse failed: {e}")))?;
    if !remaining.is_empty() {
        return Err(CertError::manifest("trailing bytes after CRL DER"));
    }
    let mut matching_cert = None;
    for record in
        bundle.manifest.certificates.iter().filter(|r| {
            matches!(r.role.as_str(), "issuer" | "root") && (r.primary || r.role == "root")
        })
    {
        let candidate = cert_info(&bundle, record)?;
        if candidate.subject_name_der == crl.issuer().as_raw() {
            if matching_cert.is_some() {
                return Err(CertError::validation(
                    "CRL issuer name is ambiguous in the supplied bundle; select a bundle with one matching CA",
                ));
            }
            matching_cert = Some(candidate);
        }
    }
    let cert = matching_cert.ok_or_else(|| {
        CertError::validation(
            "CRL issuer does not match a primary certificate in the supplied bundle",
        )
    })?;
    let signature_oid = crl.signature_algorithm.algorithm.to_id_string();
    let signature_valid = verify_signed_bytes(
        &cert,
        &signature_oid,
        crl.tbs_cert_list.as_ref(),
        &crl.signature_value.data,
    );
    let issuer_matches = cert.subject_name_der == crl.issuer().as_raw();
    let fresh = crl.last_update().to_datetime() <= at.0
        && crl.next_update().is_some_and(|n| at.0 <= n.to_datetime());
    let signature_algorithm_matches = crl.signature_algorithm == crl.tbs_cert_list.signature;
    let issuer_usable =
        cert.is_ca && cert.crl_sign && cert.not_before.0 <= at.0 && at.0 <= cert.not_after.0;
    let evidence: Vec<_> = evidence_paths
        .iter()
        .map(|path| load_bundle(path))
        .collect::<Result<_>>()?;
    let mut revocations_match = true;
    for entry in crl.iter_revoked_certificates() {
        let serial = hex_bytes(
            &entry
                .raw_serial()
                .iter()
                .skip_while(|b| **b == 0)
                .copied()
                .collect::<Vec<_>>(),
        );
        let mut matched = false;
        for evidence_bundle in &evidence {
            for record in &evidence_bundle.manifest.revocations {
                if record.serial_hex.eq_ignore_ascii_case(&serial)
                    && record.revoked_at.0 == entry.revocation_date.to_datetime()
                    && reason_code(&record.reason)
                        == Some(entry.reason_code().map(|(_, reason)| reason.0).unwrap_or(0))
                    && revocation_matches_bundle(evidence_bundle, record, &cert)?
                {
                    matched = true;
                    break;
                }
            }
            if matched {
                break;
            }
        }
        if !matched {
            revocations_match = false;
            break;
        }
    }
    let mut checks = Vec::new();
    if signature_valid {
        checks.push("signature verifies with issuer certificate public key".into());
    } else {
        checks.push("CRL signature is invalid or unsupported".into());
    }
    if issuer_matches {
        checks.push("CRL issuer name matches issuer certificate".into());
    } else {
        checks.push("CRL issuer name does not match issuer certificate".into());
    }
    if fresh {
        checks.push("CRL is current at the requested time".into());
    } else {
        checks.push("CRL is not current at the requested time".into());
    }
    if signature_algorithm_matches {
        checks.push("inner and outer signature algorithms match".into());
    } else {
        checks.push("inner and outer signature algorithms differ".into());
    }
    if issuer_usable {
        checks.push("issuer is a CRL-signing CA valid at the requested time".into());
    } else {
        checks.push("issuer is not a valid CRL-signing CA at the requested time".into());
    }
    if revocations_match {
        checks.push("listed serials occur in issuer bundle revocation records".into());
    } else {
        checks.push("CRL contains a serial absent from issuer bundle revocation records".into());
    }
    let valid = signature_valid
        && issuer_matches
        && fresh
        && revocations_match
        && signature_algorithm_matches
        && issuer_usable;
    Ok(VerifyReport {
        valid,
        signature_valid,
        issuer_matches,
        fresh,
        revocations_match,
        checks,
    })
}
