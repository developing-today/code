//! Certificate parsing, signature verification, chain building and
//! validation — the verifier half of the engine.
//!
//! Parsing is x509-parser; signature verification is aws-lc-rs (the
//! same backend that signs), covering every registered algorithm
//! including ML-DSA-44. Path building matches issuer→subject by raw
//! DER name plus AKI→SKI, then *proves* each link cryptographically
//! — a name match alone never validates.
//!
//! Trust selection is a verifier policy (§3.2/§3.5): `classical`
//! trusts only classical roots, `pq` trusts only the ML-DSA root
//! **and** requires every key and signature in the path to be
//! ML-DSA-44. Providing a classical cross-certificate cannot smuggle
//! a classical path past a PQ selection — the separation is enforced
//! here, not by issuance.

use crate::algorithm::AlgorithmId;
use crate::bundle::sha256_hex;
use crate::error::{CertError, Result};
use crate::timeparse::Rfc3339;
use serde::Serialize;
use x509_parser::extensions::ParsedExtension;
use x509_parser::prelude::*;

/// A parsed certificate with everything validation needs.
#[derive(Debug, Clone)]
pub struct CertInfo {
    pub der: Vec<u8>,
    pub fingerprint_sha256: String,
    pub subject_dn: String,
    pub issuer_dn: String,
    /// Raw DER of the subject / issuer Name (exact-match key for
    /// path building — string DNs are display only).
    pub subject_name_der: Vec<u8>,
    pub issuer_name_der: Vec<u8>,
    pub serial_hex: String,
    pub ski: Option<Vec<u8>>,
    pub aki: Option<Vec<u8>>,
    pub not_before: Rfc3339,
    pub not_after: Rfc3339,
    pub is_ca: bool,
    pub path_len: Option<u8>,
    /// Subject key algorithm, when it is a registered one.
    pub subject_algorithm: Option<AlgorithmId>,
    /// Signature algorithm OID (dotted).
    pub signature_oid: String,
    /// Subject signature-algorithm classification: true when the
    /// signature over this certificate is ML-DSA-44.
    pub signature_is_pq: bool,
    tbs: Vec<u8>,
    signature: Vec<u8>,
    spki_bytes: Vec<u8>,
    spki_oid: String,
}

pub fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Extract the raw TBS bytes from a certificate DER blob: the first
/// element inside the outer SEQUENCE.
fn tbs_bytes(der: &[u8]) -> Result<Vec<u8>> {
    fn read_len(der: &[u8], mut i: usize) -> Result<(usize, usize)> {
        if i >= der.len() {
            return Err(CertError::manifest("truncated DER"));
        }
        let first = der[i];
        i += 1;
        if first < 0x80 {
            return Ok((first as usize, i));
        }
        let n = (first & 0x7f) as usize;
        if n == 0 || n > 4 || i + n > der.len() {
            return Err(CertError::manifest("bad DER length"));
        }
        let mut len = 0usize;
        for k in 0..n {
            len = (len << 8) | der[i + k] as usize;
        }
        Ok((len, i + n))
    }
    if der.is_empty() || der[0] != 0x30 {
        return Err(CertError::manifest(
            "certificate DER does not start with a SEQUENCE",
        ));
    }
    let (_, content_start) = read_len(der, 1)?;
    if content_start >= der.len() || der[content_start] != 0x30 {
        return Err(CertError::manifest("certificate TBS is not a SEQUENCE"));
    }
    let (tbs_len, tbs_content) = read_len(der, content_start + 1)?;
    let tbs_end = tbs_content + tbs_len;
    if tbs_end > der.len() {
        return Err(CertError::manifest("truncated TBS"));
    }
    Ok(der[content_start..tbs_end].to_vec())
}

/// Classify a subject key from its SPKI OID (+ size for RSA).
fn classify_subject(spki_oid: &str, spki_bytes: &[u8]) -> Option<AlgorithmId> {
    match spki_oid {
        "1.3.101.112" => Some(AlgorithmId::Ed25519),
        "1.2.840.10045.2.1" => Some(AlgorithmId::P256),
        "2.16.840.1.101.3.4.3.17" => Some(AlgorithmId::MlDsa44),
        "1.2.840.113549.1.1.1" => {
            // RSAPublicKey inside the BIT STRING: ~270 bytes total
            // for 2048-bit, ~526 for 4096-bit moduli.
            if spki_bytes.len() > 400 {
                Some(AlgorithmId::Rsa4096)
            } else {
                Some(AlgorithmId::Rsa2048)
            }
        }
        _ => None,
    }
}

/// Parse one certificate from DER.
pub fn parse_cert(der: &[u8]) -> Result<CertInfo> {
    let (_, cert) = parse_x509_certificate(der)
        .map_err(|e| CertError::manifest(format!("certificate parse failed: {e}")))?;
    let tbs = &cert.tbs_certificate;
    let (bc_is_ca, bc_path_len) = match tbs.basic_constraints() {
        Ok(Some(bc)) => (bc.value.ca, bc.value.path_len_constraint.map(|n| n as u8)),
        _ => (false, None),
    };
    // Extension lookup by OID string (2.5.29.14 SKI, 2.5.29.35 AKI).
    let ext_by_oid = |oid: &str| {
        tbs.extensions()
            .iter()
            .find(|e| e.oid.to_id_string() == oid)
    };
    let ski = match ext_by_oid("2.5.29.14") {
        Some(ext) => match ext.parsed_extension() {
            ParsedExtension::SubjectKeyIdentifier(ki) => Some(ki.0.to_vec()),
            _ => None,
        },
        None => None,
    };
    let aki = match ext_by_oid("2.5.29.35") {
        Some(ext) => match ext.parsed_extension() {
            ParsedExtension::AuthorityKeyIdentifier(aki) => {
                aki.key_identifier.as_ref().map(|ki| ki.0.to_vec())
            }
            _ => None,
        },
        None => None,
    };
    let spki_oid = tbs.subject_pki.algorithm.algorithm.to_id_string();
    let spki_bytes = tbs.subject_pki.subject_public_key.data.to_vec();
    let signature_oid = cert.signature_algorithm.algorithm.to_id_string();
    Ok(CertInfo {
        der: der.to_vec(),
        fingerprint_sha256: sha256_hex(der),
        subject_dn: tbs.subject.to_string(),
        issuer_dn: tbs.issuer.to_string(),
        subject_name_der: tbs.subject.as_raw().to_vec(),
        issuer_name_der: tbs.issuer.as_raw().to_vec(),
        serial_hex: hex_bytes(&tbs.serial.to_bytes_be()),
        ski,
        aki,
        not_before: Rfc3339(tbs.validity.not_before.to_datetime()),
        not_after: Rfc3339(tbs.validity.not_after.to_datetime()),
        is_ca: bc_is_ca,
        path_len: bc_path_len,
        subject_algorithm: classify_subject(&spki_oid, &spki_bytes),
        signature_is_pq: signature_oid == "2.16.840.1.101.3.4.3.17",
        signature_oid,
        tbs: tbs_bytes(der)?,
        signature: cert.signature_value.data.to_vec(),
        spki_bytes,
        spki_oid,
    })
}

/// Verify `subject`'s signature using `issuer`'s public key.
/// Returns false for any mismatch or unsupported pairing — never
/// panics on untrusted input.
pub fn verify_signature(issuer: &CertInfo, subject: &CertInfo) -> bool {
    use aws_lc_rs::signature::{
        ECDSA_P256_SHA256_ASN1, ED25519, ML_DSA_44, RSA_PKCS1_2048_8192_SHA256, UnparsedPublicKey,
    };
    let key = &issuer.spki_bytes;
    let ok = match subject.signature_oid.as_str() {
        "1.3.101.112" => UnparsedPublicKey::new(&ED25519, key)
            .verify(&subject.tbs, &subject.signature)
            .is_ok(),
        "1.2.840.10045.4.3.2" => UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, key)
            .verify(&subject.tbs, &subject.signature)
            .is_ok(),
        "1.2.840.113549.1.1.11" => UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, key)
            .verify(&subject.tbs, &subject.signature)
            .is_ok(),
        "2.16.840.1.101.3.4.3.17" => UnparsedPublicKey::new(&ML_DSA_44, key)
            .verify(&subject.tbs, &subject.signature)
            .is_ok(),
        _ => false,
    };
    ok
}

/// Which roots a validation trusts (§3.5 trust bundles).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrustSelection {
    /// Every self-signed root in the set (`roots-all`).
    All,
    /// Classical roots only (`roots-classical`).
    Classical,
    /// The ML-DSA root only, with an all-ML-DSA path requirement
    /// (`roots-pq`, §3.2 verifier policy).
    Pq,
}

impl TrustSelection {
    pub fn parse(input: &str) -> Result<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "all" | "roots-all" => Ok(TrustSelection::All),
            "classical" | "roots-classical" => Ok(TrustSelection::Classical),
            "pq" | "roots-pq" => Ok(TrustSelection::Pq),
            other => Err(CertError::usage(format!(
                "unknown trust selection '{other}' (all | classical | pq)"
            ))),
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            TrustSelection::All => "all",
            TrustSelection::Classical => "classical",
            TrustSelection::Pq => "pq",
        }
    }
    fn root_admitted(&self, root: &CertInfo) -> bool {
        match self {
            TrustSelection::All => true,
            TrustSelection::Classical => root
                .subject_algorithm
                .map(|a| a.is_classical())
                .unwrap_or(false),
            TrustSelection::Pq => root.subject_algorithm == Some(AlgorithmId::MlDsa44),
        }
    }
}

/// One named check in a validation report.
#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

/// The outcome of a chain validation.
#[derive(Debug, Clone, Serialize)]
pub struct ChainReport {
    pub valid: bool,
    pub trust_selection: String,
    /// The built path, leaf first, anchor last (subject DNs).
    pub path: Vec<String>,
    pub path_fingerprints: Vec<String>,
    pub checks: Vec<Check>,
}

fn is_self_signed(cert: &CertInfo) -> bool {
    cert.subject_name_der == cert.issuer_name_der && verify_signature(cert, cert)
}

/// Build a path from `leaf` through `pool` (intermediates and roots,
/// any order) to a self-signed anchor admitted by `trust`. Every
/// link is signature-verified.
///
/// The search is iterative-deepening: the *shortest* valid path
/// wins. That matters for cross-signing — a directly cross-signed
/// issuer variant (§24.2) yields a shorter path than routing the
/// same key through a root-level cross-certificate (§3.2), and the
/// longer route can be genuinely invalid: a cross-certificate is a
/// CA certificate in the path, so an anchor with pathLenConstraint
/// 1 cannot top a leaf → issuer → cross → root path (2 CA below).
/// Path-length feasibility is therefore pruned during the search
/// (anchor constraints enforced, RFC 5280 §6.1 reading), not
/// discovered after it.
pub fn build_path(
    leaf: &CertInfo,
    pool: &[CertInfo],
    trust: TrustSelection,
) -> Option<Vec<CertInfo>> {
    for limit in 1..=6 {
        if let Some(path) = dfs(leaf, pool, trust, 0, limit, 0) {
            return Some(path);
        }
    }
    None
}

fn dfs(
    current: &CertInfo,
    pool: &[CertInfo],
    trust: TrustSelection,
    depth: usize,
    limit: usize,
    ca_below_current: usize,
) -> Option<Vec<CertInfo>> {
    // Anchor case: current is itself a trusted self-signed root.
    if is_self_signed(current) && trust.root_admitted(current) {
        return Some(vec![current.clone()]);
    }
    if depth >= limit {
        return None;
    }
    // Candidate issuers: name match, AKI→SKI when both present,
    // a signature that actually verifies, CA flag set, and a
    // path-length budget that fits the CAs already below it.
    let mut candidates: Vec<&CertInfo> = pool
        .iter()
        .filter(|cand| cand.subject_name_der == current.issuer_name_der)
        .filter(|cand| match (&current.aki, &cand.ski) {
            (Some(aki), Some(ski)) => aki == ski,
            _ => true,
        })
        .filter(|cand| cand.is_ca || is_self_signed(cand))
        .filter(|cand| verify_signature(cand, current))
        .filter(|cand| {
            let ca_below_cand = ca_below_current + usize::from(depth > 0);
            match cand.path_len {
                Some(pl) => ca_below_cand <= pl as usize,
                None => true,
            }
        })
        .collect();
    // Prefer anchors, then primaries-by-name stability.
    candidates.sort_by_key(|c| (!is_self_signed(c), c.fingerprint_sha256.clone()));
    for cand in candidates {
        let ca_below_cand = ca_below_current + usize::from(depth > 0);
        if let Some(mut tail) = dfs(cand, pool, trust, depth + 1, limit, ca_below_cand) {
            let mut path = vec![current.clone()];
            path.append(&mut tail);
            return Some(path);
        }
    }
    None
}

/// Validate a leaf against a pool and a trust selection at time
/// `at`, producing a per-check report. Checks: path exists,
/// signatures (inherent to path building), per-certificate validity
/// window, CA flags + path length on every CA in the path, and the
/// trust selection's own rule (for `pq`: every key and signature in
/// the path is ML-DSA-44).
pub fn validate_chain(
    leaf: &CertInfo,
    pool: &[CertInfo],
    trust: TrustSelection,
    at: Rfc3339,
) -> ChainReport {
    let mut checks = Vec::new();
    let path = build_path(leaf, pool, trust);
    match &path {
        Some(p) => checks.push(Check {
            name: "path-builds".into(),
            ok: true,
            detail: format!("{} certificates to the anchor", p.len()),
        }),
        None => checks.push(Check {
            name: "path-builds".into(),
            ok: false,
            detail: format!(
                "no verified path from {} to a {} trust anchor",
                leaf.subject_dn,
                trust.as_str()
            ),
        }),
    }
    let mut valid = path.is_some();
    if let Some(p) = &path {
        // Validity windows.
        for cert in p {
            let ok = cert.not_before <= at && at <= cert.not_after;
            if !ok {
                valid = false;
            }
            checks.push(Check {
                name: format!("validity:{}", cert.subject_dn),
                ok,
                detail: format!("{} .. {}", cert.not_before, cert.not_after),
            });
        }
        // CA flags + path length: every non-leaf in the path must be
        // a CA, and the number of CA certificates below it must fit
        // its pathLenConstraint.
        for (i, cert) in p.iter().enumerate().skip(1) {
            let ok_ca = cert.is_ca;
            if !ok_ca {
                valid = false;
            }
            checks.push(Check {
                name: format!("ca-flag:{}", cert.subject_dn),
                ok: ok_ca,
                detail: if ok_ca {
                    "CA:true".into()
                } else {
                    "not a CA".into()
                },
            });
            if let Some(pl) = cert.path_len {
                let below = (i - 1) as u8; // CA certs strictly below this one
                let ok = below <= pl;
                if !ok {
                    valid = false;
                }
                checks.push(Check {
                    name: format!("pathlen:{}", cert.subject_dn),
                    ok,
                    detail: format!("{below} CA below, constraint {pl}"),
                });
            }
        }
        // Signature spot-check is inherent to build_path; record it.
        checks.push(Check {
            name: "signatures-verify".into(),
            ok: true,
            detail: "every link verified with the issuer's public key".into(),
        });
        // Trust-selection rule.
        match trust {
            TrustSelection::Pq => {
                let all_pq = p.iter().all(|c| {
                    c.subject_algorithm == Some(AlgorithmId::MlDsa44) && c.signature_is_pq
                });
                // The anchor's own signature is self-referential; it
                // is ML-DSA by the subject check. Non-anchor links
                // must carry ML-DSA signatures.
                if !all_pq {
                    valid = false;
                }
                checks.push(Check {
                    name: "pq-only-path".into(),
                    ok: all_pq,
                    detail: if all_pq {
                        "every key and signature in the path is ML-DSA-44".into()
                    } else {
                        "a classical key or signature appears in the path".into()
                    },
                });
            }
            TrustSelection::Classical => {
                let anchor = p.last();
                let ok = anchor
                    .map(|a| {
                        a.subject_algorithm
                            .map(|x| x.is_classical())
                            .unwrap_or(false)
                    })
                    .unwrap_or(false);
                if !ok {
                    valid = false;
                }
                checks.push(Check {
                    name: "classical-anchor".into(),
                    ok,
                    detail: "anchor is a classical root".into(),
                });
            }
            TrustSelection::All => {}
        }
    }
    ChainReport {
        valid,
        trust_selection: trust.as_str().to_string(),
        path: path
            .as_ref()
            .map(|p| p.iter().map(|c| c.subject_dn.clone()).collect())
            .unwrap_or_default(),
        path_fingerprints: path
            .as_ref()
            .map(|p| p.iter().map(|c| c.fingerprint_sha256.clone()).collect())
            .unwrap_or_default(),
        checks,
    }
}
