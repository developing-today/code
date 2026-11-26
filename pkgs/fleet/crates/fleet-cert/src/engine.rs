//! The issuance engine: key generation, root/issuer/leaf creation,
//! and bundle-level validation, over rcgen + aws-lc-rs.
//!
//! Invariants the engine keeps:
//! - Every certificate it writes is recorded in the bundle manifest
//!   with facts **parsed back from the issued DER** (never from the
//!   request), so the manifest cannot drift from the artifacts.
//! - Cross-signing realizes the [`crate::graph`] policy tables; the
//!   engine contains no per-algorithm-pair logic.
//! - Signing keys are loaded only from an explicitly passed
//!   [`SignerSource`] (§24.3). `create_root_set` generates fresh
//!   keys because creation *is* the ceremony; every later signing
//!   operation takes its signer from a request field.
//! - Validity windows are clamped to the signer's window with the
//!   clamp recorded as a manifest warning (§5.4: a leaf must not
//!   outlive its issuer).

use crate::algorithm::{AlgorithmId, check_ca_policy};
use crate::bundle::{
    Artifact, Bundle, load_bundle, pem_encode, read_artifact, sha256_hex, write_bundle,
};
use crate::chain::{CertInfo, ChainReport, TrustSelection, parse_cert, validate_chain};
use crate::error::{CertError, Result};
use crate::graph::{CrossSigningMode, GraphPolicy};
use crate::model::{
    BundleManifest, BundleType, CertRecord, IssuerCreateRequest, LeafIssueRequest, MANIFEST_SCHEMA,
    ParentRef, RootCreateRequest,
};
use crate::naming::{GenerationPartition, slugify};
use crate::profile::{check_broker_sans, resolve_profile, KeyCustody, SanPolicy};
use crate::timeparse::Rfc3339;
use rcgen::{
    BasicConstraints, CertificateParams, DnType, DnValue, IsCa, Issuer, KeyPair, KeyUsagePurpose,
    PKCS_ECDSA_P256_SHA256, PKCS_ED25519, PKCS_ML_DSA_44, PKCS_RSA_SHA256, RsaKeySize,
    SerialNumber,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- keys

/// Generate a fresh key pair for a registered algorithm.
pub fn generate_key(algo: AlgorithmId) -> Result<KeyPair> {
    let key = match algo {
        AlgorithmId::Ed25519 => KeyPair::generate_for(&PKCS_ED25519)?,
        AlgorithmId::P256 => KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?,
        AlgorithmId::MlDsa44 => KeyPair::generate_for(&PKCS_ML_DSA_44)?,
        AlgorithmId::Rsa4096 => KeyPair::generate_rsa_for(&PKCS_RSA_SHA256, RsaKeySize::_4096)?,
        AlgorithmId::Rsa2048 => KeyPair::generate_rsa_for(&PKCS_RSA_SHA256, RsaKeySize::_2048)?,
    };
    Ok(key)
}

/// An ML-DSA-44 signer wrapping aws-lc-rs directly.
///
/// rcgen 0.14 generates ML-DSA keys but cannot *reload* them: its
/// PKCS#8 loaders have no PQ branch (the typed loader panics, the
/// detecting loader returns CouldNotParseKeyPair). The signing
/// traits rcgen exposes are public, though, so this wrapper carries
/// a reloaded `PqdsaKeyPair` through the same `Issuer` machinery as
/// every classical key — one TBS builder, one extension profile,
/// no second certificate engine.
pub struct MlDsa44Signer {
    key: aws_lc_rs::signature::PqdsaKeyPair,
    public_key: Vec<u8>,
}

impl MlDsa44Signer {
    pub fn from_pkcs8(der: &[u8]) -> Result<Self> {
        let key = aws_lc_rs::signature::PqdsaKeyPair::from_pkcs8(
            &aws_lc_rs::signature::ML_DSA_44_SIGNING,
            der,
        )
        .map_err(|e| CertError::engine(format!("ML-DSA-44 key load failed: {e}")))?;
        let public_key = aws_lc_rs::signature::KeyPair::public_key(&key)
            .as_ref()
            .to_vec();
        Ok(MlDsa44Signer { key, public_key })
    }

    pub fn public_key(&self) -> &[u8] {
        &self.public_key
    }
}

/// Parse a PKCS#8 key using the explicitly named algorithm and return
/// only its public key bytes. Private material is never part of the
/// result, which is the metadata boundary used by `key inspect`.
pub fn inspect_key(algo: AlgorithmId, pkcs8: &[u8]) -> Result<Vec<u8>> {
    match algo {
        AlgorithmId::MlDsa44 => Ok(MlDsa44Signer::from_pkcs8(pkcs8)?.public_key().to_vec()),
        AlgorithmId::Ed25519 => Ok(KeyPair::from_pkcs8_der_and_sign_algo(
            &rustls_pki_types::PrivatePkcs8KeyDer::from(pkcs8.to_vec()),
            &PKCS_ED25519,
        )?
        .public_key_raw()
        .to_vec()),
        AlgorithmId::P256 => Ok(KeyPair::from_pkcs8_der_and_sign_algo(
            &rustls_pki_types::PrivatePkcs8KeyDer::from(pkcs8.to_vec()),
            &PKCS_ECDSA_P256_SHA256,
        )?
        .public_key_raw()
        .to_vec()),
        AlgorithmId::Rsa4096 | AlgorithmId::Rsa2048 => Ok(KeyPair::from_pkcs8_der_and_sign_algo(
            &rustls_pki_types::PrivatePkcs8KeyDer::from(pkcs8.to_vec()),
            &PKCS_RSA_SHA256,
        )?
        .public_key_raw()
        .to_vec()),
    }
}

/// DER-encode the X.509 SubjectPublicKeyInfo (RFC 5280 §4.1.2.7) for a
/// public key as [`inspect_key`] returns it: the raw Ed25519 point, the
/// uncompressed P-256 point, the PKCS#1 `RSAPublicKey`, or the FIPS 204
/// ML-DSA-44 public key. The algorithm identifiers are the registry's
/// SPKI OIDs: `rsaEncryption` carries a NULL parameter, P-256 carries
/// the `prime256v1` named curve, and Ed25519 / ML-DSA-44 (RFC 8410,
/// RFC 9881) carry none.
pub fn public_key_spki_der(algo: AlgorithmId, public_key: &[u8]) -> Vec<u8> {
    const ED25519: &[u8] = &[0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70];
    const P256: &[u8] = &[
        0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86,
        0x48, 0xce, 0x3d, 0x03, 0x01, 0x07,
    ];
    const RSA: &[u8] = &[
        0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01, 0x05, 0x00,
    ];
    const ML_DSA_44: &[u8] = &[
        0x30, 0x0b, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x03, 0x11,
    ];
    let algorithm_identifier = match algo {
        AlgorithmId::Ed25519 => ED25519,
        AlgorithmId::P256 => P256,
        AlgorithmId::Rsa4096 | AlgorithmId::Rsa2048 => RSA,
        AlgorithmId::MlDsa44 => ML_DSA_44,
    };
    let mut bit_string = Vec::with_capacity(public_key.len() + 1);
    bit_string.push(0); // no unused bits
    bit_string.extend_from_slice(public_key);
    let mut body = algorithm_identifier.to_vec();
    der_push(&mut body, 0x03, &bit_string);
    let mut spki = Vec::with_capacity(body.len() + 4);
    der_push(&mut spki, 0x30, &body);
    spki
}

/// Append one DER TLV (definite-length encoding) to `out`.
fn der_push(out: &mut Vec<u8>, tag: u8, content: &[u8]) {
    out.push(tag);
    let len = content.len();
    if len < 0x80 {
        out.push(len as u8);
    } else {
        let bytes = len.to_be_bytes();
        let skip = bytes.iter().take_while(|b| **b == 0).count();
        out.push(0x80 | (bytes.len() - skip) as u8);
        out.extend_from_slice(&bytes[skip..]);
    }
    out.extend_from_slice(content);
}

impl rcgen::PublicKeyData for MlDsa44Signer {
    fn der_bytes(&self) -> &[u8] {
        &self.public_key
    }
    fn algorithm(&self) -> &'static rcgen::SignatureAlgorithm {
        &PKCS_ML_DSA_44
    }
}

impl rcgen::SigningKey for MlDsa44Signer {
    fn sign(&self, msg: &[u8]) -> std::result::Result<Vec<u8>, rcgen::Error> {
        let mut signature = vec![0; self.key.algorithm().signature_len()];
        self.key
            .sign(msg, &mut signature)
            .map_err(|_| rcgen::Error::RingUnspecified)?;
        Ok(signature)
    }
}

/// A loaded CA signing key, whatever the algorithm.
pub enum CaSigner {
    Classical(KeyPair),
    MlDsa(Box<MlDsa44Signer>),
}

/// Load a signing key from an explicit signer source: the PKCS#8
/// file named `file_name` inside `source.key_dir`. This is the only
/// key-loading path in the crate (§24.3).
pub fn load_signer_key(
    source: &crate::model::SignerSource,
    file_name: &str,
    algo: AlgorithmId,
) -> Result<CaSigner> {
    let path = source.key_dir.join(file_name);
    let der = std::fs::read(&path).map_err(|e| {
        CertError::policy(format!(
            "signer source {} does not provide key {file_name}: {e} \
             (signing requires an explicitly passed signer source, spec §24.3)",
            source.key_dir.display()
        ))
    })?;
    match algo {
        AlgorithmId::MlDsa44 => Ok(CaSigner::MlDsa(Box::new(MlDsa44Signer::from_pkcs8(&der)?))),
        _ => {
            let sign_alg: &'static rcgen::SignatureAlgorithm = match algo {
                AlgorithmId::Ed25519 => &PKCS_ED25519,
                AlgorithmId::P256 => &PKCS_ECDSA_P256_SHA256,
                AlgorithmId::Rsa4096 | AlgorithmId::Rsa2048 => &PKCS_RSA_SHA256,
                AlgorithmId::MlDsa44 => unreachable!(),
            };
            Ok(CaSigner::Classical(KeyPair::from_pkcs8_der_and_sign_algo(
                &rustls_pki_types::PrivatePkcs8KeyDer::from(der),
                sign_alg,
            )?))
        }
    }
}

fn random_serial() -> SerialNumber {
    let mut bytes = [0u8; 16];
    aws_lc_rs::rand::fill(&mut bytes).expect("system RNG available");
    // Keep the serial a positive 16-byte integer with the top bits
    // set (high entropy, never a small or negative encoding).
    bytes[0] = (bytes[0] & 0x3f) | 0x40;
    SerialNumber::from_slice(&bytes)
}

// ---------------------------------------------------------------- DNs

/// Build a distinguished name with the crate's one encoding
/// convention: country as PrintableString (as in rcgen's examples),
/// everything else as UTF8String. Creation and later reconstruction
/// (when a stored CA signs again) use this same function, so issuer
/// names in children byte-match the CA's subject.
fn distinguished_name(
    organization: &str,
    country: &str,
    ou: Option<&str>,
    cn: &str,
) -> rcgen::DistinguishedName {
    let mut dn = rcgen::DistinguishedName::new();
    if !country.is_empty() {
        if let Ok(ps) = <rcgen::string::PrintableString as TryFrom<&str>>::try_from(country) {
            dn.push(DnType::CountryName, DnValue::PrintableString(ps));
        }
    }
    dn.push(DnType::OrganizationName, organization);
    if let Some(ou) = ou {
        dn.push(DnType::OrganizationalUnitName, ou);
    }
    dn.push(DnType::CommonName, cn);
    dn
}

/// Rebuild rcgen params for a stored CA certificate so it can sign:
/// subject DN reconstructed with the crate convention, CA profile
/// reapplied. The validity in the returned params is the *child's*
/// concern; the issuer's own window is enforced by clamping before
/// this is called.
fn ca_params_for(
    org: &str,
    country: &str,
    ou: Option<&str>,
    cn: &str,
    path_len: u8,
    not_before: Rfc3339,
    not_after: Rfc3339,
) -> CertificateParams {
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("empty SAN list is valid");
    params.distinguished_name = distinguished_name(org, country, ou, cn);
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(path_len));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params.not_before = not_before.0;
    params.not_after = not_after.0;
    params.serial_number = Some(random_serial());
    params.use_authority_key_identifier_extension = true;
    params
}

/// A loaded CA: everything needed to sign with it.
struct CaHandle {
    params: CertificateParams,
    signer: CaSigner,
    cert: CertInfo,
}

impl CaHandle {
    /// Sign `subject_params` for `subject_key` with this CA.
    fn sign(
        &self,
        subject_params: &CertificateParams,
        subject_key: &impl rcgen::PublicKeyData,
    ) -> Result<rcgen::Certificate> {
        match &self.signer {
            CaSigner::Classical(key) => Ok(
                subject_params.signed_by(subject_key, &Issuer::new(self.params.clone(), key))?
            ),
            CaSigner::MlDsa(signer) => Ok(subject_params.signed_by(
                subject_key,
                &Issuer::new(self.params.clone(), signer.as_ref()),
            )?),
        }
    }
}

/// Load a CA (cert + key) from a bundle record through an explicit
/// signer source. The record supplies the cert path and the key file
/// name; the key bytes come only from `signer`.
fn load_ca(
    bundle: &Bundle,
    record: &CertRecord,
    signer: &crate::model::SignerSource,
    org: &str,
    country: &str,
    ou: Option<&str>,
    cn: &str,
    path_len: u8,
) -> Result<CaHandle> {
    let pem = read_artifact(bundle, &record.cert_path)?;
    let blocks = crate::bundle::pem_decode_all(
        std::str::from_utf8(&pem)
            .map_err(|_| CertError::manifest(format!("{} is not UTF-8 PEM", record.cert_path)))?,
    )?;
    let der = blocks
        .into_iter()
        .find(|(label, _)| label == "CERTIFICATE")
        .map(|(_, der)| der)
        .ok_or_else(|| {
            CertError::manifest(format!("no CERTIFICATE block in {}", record.cert_path))
        })?;
    let cert = parse_cert(&der)?;
    // The key file name comes from the manifest record when the key
    // lives in the bundle; for external-store custody the creation
    // naming convention applies (`<algo>.pk8` roots,
    // `<algo>-<partition>.pk8` issuers) inside the store directory
    // the caller passes as the signer source.
    let key_file = record
        .key_path
        .as_deref()
        .and_then(|p| {
            Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| match record.role.as_str() {
            "root" => format!("{}.pk8", record.algorithm.name()),
            _ => format!("{}-{}.pk8", record.algorithm.name(), record.partition),
        });
    let signer_key = load_signer_key(signer, &key_file, record.algorithm)?;
    let params = ca_params_for(
        org,
        country,
        ou,
        cn,
        path_len,
        cert.not_before,
        cert.not_after,
    );
    Ok(CaHandle {
        params,
        signer: signer_key,
        cert,
    })
}

// ---------------------------------------------------------------- records

/// Build a manifest record from the issued DER (facts parsed back,
/// never copied from the request).
fn record_from_der(
    der: &[u8],
    role: &str,
    partition: &str,
    primary: bool,
    cert_path: String,
    key_path: Option<String>,
) -> Result<CertRecord> {
    let info = parse_cert(der)?;
    Ok(CertRecord {
        role: role.to_string(),
        algorithm: info.subject_algorithm.ok_or_else(|| {
            CertError::engine("issued certificate has an unregistered subject algorithm")
        })?,
        signer_algorithm: None, // filled by caller when not self-signed
        partition: partition.to_string(),
        primary,
        subject_dn: info.subject_dn.clone(),
        issuer_dn: info.issuer_dn.clone(),
        serial_hex: info.serial_hex.clone(),
        ski_hex: info
            .ski
            .as_deref()
            .map(crate::chain::hex_bytes)
            .unwrap_or_default(),
        aki_hex: info
            .aki
            .as_deref()
            .map(crate::chain::hex_bytes)
            .unwrap_or_default(),
        fingerprint_sha256: info.fingerprint_sha256.clone(),
        not_before: info.not_before,
        not_after: info.not_after,
        is_ca: info.is_ca,
        path_len: info.path_len,
        cert_path,
        key_path,
    })
}

fn base_manifest(
    bundle_type: BundleType,
    name: &str,
    organization: &str,
    country: &str,
    purpose: Option<String>,
    generation: u32,
    parent: Option<ParentRef>,
    key_location: &str,
    cross_signing: Option<String>,
    warnings: Vec<String>,
) -> Result<BundleManifest> {
    Ok(BundleManifest {
        schema: MANIFEST_SCHEMA.to_string(),
        bundle_id: String::new(), // set by caller once a fingerprint exists
        bundle_type,
        bundle_version: 1,
        name: name.to_string(),
        slug: slugify(name)?,
        organization: organization.to_string(),
        country: country.to_string(),
        purpose,
        generation,
        revision: 1,
        predecessor: None,
        created_at: Rfc3339::now(),
        engine_version: env!("CARGO_PKG_VERSION").to_string(),
        parent,
        key_location: key_location.to_string(),
        cross_signing,
        certificates: Vec::new(),
        files: Vec::new(),
        warnings,
    })
}

fn key_rel_path(file_name: &str) -> String {
    format!("private/keys/{file_name}")
}

/// Where a created bundle's keys physically go, and the manifest's
/// custody class for them.
fn key_destination(out_dir: &Path, private_store: &Option<PathBuf>) -> (PathBuf, &'static str) {
    match private_store {
        Some(store) => (store.clone(), "external-store"),
        None => (out_dir.join("private/keys"), "bundle-private"),
    }
}

/// Write key files for a creation run. Keys in an external store are
/// written there directly (0600); keys in the bundle go through the
/// artifact list so they are checksummed in the manifest.
fn stage_key(
    artifacts: &mut Vec<Artifact>,
    external_dir: Option<&Path>,
    file_name: &str,
    key: &KeyPair,
) -> Result<Option<String>> {
    let der = key.serialize_der();
    match external_dir {
        Some(dir) => {
            std::fs::create_dir_all(dir)?;
            let path = dir.join(file_name);
            std::fs::write(&path, &der)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
            Ok(None)
        }
        None => {
            let rel = key_rel_path(file_name);
            artifacts.push(Artifact {
                rel_path: rel.clone(),
                kind: "private-key-pkcs8".into(),
                visibility: "private".into(),
                bytes: der,
            });
            Ok(Some(rel))
        }
    }
}

// ---------------------------------------------------------------- roots

/// `cert root create`: generate a root set — one self-signed root
/// per algorithm, plus the root-level cross-certificates the graph
/// policy and mode call for — and the §3.5 trust bundles.
pub fn create_root_set(req: &RootCreateRequest, out_dir: &Path) -> Result<BundleManifest> {
    check_ca_policy(&req.algorithms, req.allow_legacy_root_strength)?;
    let mut seen = Vec::new();
    for algo in &req.algorithms {
        if seen.contains(algo) {
            return Err(CertError::usage(format!(
                "duplicate algorithm {} in a root set (one key per algorithm per generation/partition)",
                algo.name()
            )));
        }
        seen.push(*algo);
    }
    let gp = GenerationPartition::new(req.generation, &req.partition)?;
    let graph = GraphPolicy::spec_default();
    let (key_dir, key_location) = key_destination(out_dir, &req.private_store);
    let external = if req.private_store.is_some() {
        Some(key_dir.as_path())
    } else {
        None
    };

    // Generate keys and self-signed roots.
    struct RootKey {
        algo: AlgorithmId,
        key: KeyPair,
        params: CertificateParams,
        der: Vec<u8>,
        key_path: Option<String>,
    }
    let mut artifacts: Vec<Artifact> = Vec::new();
    let mut roots: Vec<RootKey> = Vec::new();
    for algo in &req.algorithms {
        let key = generate_key(*algo)?;
        let cn = format!(
            "{} Root {} CA {}",
            req.organization,
            algo.display_name(),
            gp.cn_tail()
        );
        let params = ca_params_for(
            &req.organization,
            &req.country,
            None,
            &cn,
            1,
            req.not_before,
            req.not_after,
        );
        let cert = params.self_signed(&key)?;
        let key_path = stage_key(
            &mut artifacts,
            external,
            &format!("{}.pk8", algo.name()),
            &key,
        )?;
        roots.push(RootKey {
            algo: *algo,
            key,
            params,
            der: cert.der().to_vec(),
            key_path,
        });
    }

    let mut records = Vec::new();
    let mut warnings = Vec::new();
    // Self-signed records + public artifacts.
    for root in &roots {
        let rel = format!("public/roots/{}/root.crt.pem", root.algo.name());
        artifacts.push(Artifact {
            rel_path: rel.clone(),
            kind: "certificate-pem".into(),
            visibility: "public".into(),
            bytes: pem_encode("CERTIFICATE", &root.der).into_bytes(),
        });
        records.push(record_from_der(
            &root.der,
            "root",
            &gp.partition,
            true,
            rel,
            root.key_path.clone(),
        )?);
    }
    // Cross-certificates, realized from the policy table.
    if req.cross_signing.root_level_active() {
        for subject in &roots {
            for signer_algo in graph.root_signers_for(subject.algo) {
                let Some(signer) = roots.iter().find(|r| r.algo == signer_algo) else {
                    continue;
                };
                let issuer = Issuer::new(signer.params.clone(), &signer.key);
                let mut cross_params = subject.params.clone();
                cross_params.serial_number = Some(random_serial());
                let cert = cross_params.signed_by(&subject.key, &issuer)?;
                let der = cert.der().to_vec();
                let rel = format!(
                    "public/cross/{}/signed-by-{}.crt.pem",
                    subject.algo.name(),
                    signer_algo.name()
                );
                artifacts.push(Artifact {
                    rel_path: rel.clone(),
                    kind: "certificate-pem".into(),
                    visibility: "public".into(),
                    bytes: pem_encode("CERTIFICATE", &der).into_bytes(),
                });
                let mut rec = record_from_der(&der, "root-cross", &gp.partition, false, rel, None)?;
                rec.signer_algorithm = Some(signer_algo);
                records.push(rec);
            }
        }
    } else if req.cross_signing.issuer_level_active() {
        warnings.push(
            "cross-signing mode issuers-direct creates no root-level cross-certificates".into(),
        );
    }

    // Trust bundles (§3.5).
    let mut bundle_pem = |name: &str, ders: &[&Vec<u8>]| {
        let mut text = String::new();
        for der in ders {
            text.push_str(&pem_encode("CERTIFICATE", der));
        }
        artifacts.push(Artifact {
            rel_path: format!("public/bundles/{name}"),
            kind: "trust-bundle-pem".into(),
            visibility: "public".into(),
            bytes: text.into_bytes(),
        });
    };
    let all: Vec<&Vec<u8>> = roots.iter().map(|r| &r.der).collect();
    bundle_pem("roots-all.pem", &all);
    let classical: Vec<&Vec<u8>> = roots
        .iter()
        .filter(|r| r.algo.is_classical())
        .map(|r| &r.der)
        .collect();
    if !classical.is_empty() {
        bundle_pem("roots-classical.pem", &classical);
    }
    let pq: Vec<&Vec<u8>> = roots
        .iter()
        .filter(|r| r.algo.is_post_quantum())
        .map(|r| &r.der)
        .collect();
    if !pq.is_empty() {
        bundle_pem("roots-pq.pem", &pq);
    }
    let cross_ders: Vec<Vec<u8>> = records
        .iter()
        .filter(|r| r.role == "root-cross")
        .filter_map(|r| {
            artifacts
                .iter()
                .find(|a| a.rel_path == r.cert_path)
                .map(|a| a.bytes.clone())
        })
        .collect();
    if !cross_ders.is_empty() {
        // cross-certs.pem holds the PEM text of every cross cert.
        let mut text = String::new();
        for pem in &cross_ders {
            text.push_str(std::str::from_utf8(pem).unwrap_or(""));
        }
        artifacts.push(Artifact {
            rel_path: "public/bundles/cross-certs.pem".into(),
            kind: "trust-bundle-pem".into(),
            visibility: "public".into(),
            bytes: text.into_bytes(),
        });
    }

    let name = format!("Root Set {}", gp.machine_id().to_ascii_uppercase());
    let mut manifest = base_manifest(
        BundleType::RootSet,
        &name,
        &req.organization,
        &req.country,
        None,
        req.generation,
        None,
        key_location,
        Some(req.cross_signing.as_str().to_string()),
        warnings,
    )?;
    manifest.bundle_id = format!(
        "rootset-{}-{}",
        gp.machine_id(),
        &records
            .first()
            .map(|r| r.fingerprint_sha256[..12].to_string())
            .unwrap_or_default()
    );
    manifest.certificates = records;
    let manifest = write_bundle(out_dir, manifest, &artifacts)?;
    Ok(manifest)
}

// ---------------------------------------------------------------- issuers

/// `cert issuer create`: generate an issuer set — independent keys
/// per (algorithm, partition) slot, each certified by its primary
/// root plus the direct cross-sign variants the policy realizes.
/// Root signing keys come only from `req.root_signer` (§24.3).
pub fn create_issuer_set(req: &IssuerCreateRequest, out_dir: &Path) -> Result<BundleManifest> {
    let slot_algos: Vec<AlgorithmId> = req.slots.iter().map(|(a, _)| *a).collect();
    check_ca_policy(&slot_algos, req.allow_legacy_root_strength)?;
    let root_bundle = load_bundle(&req.root_bundle)?;
    if root_bundle.manifest.bundle_type != BundleType::RootSet {
        return Err(CertError::usage(format!(
            "{} is a {} bundle, not a root set",
            req.root_bundle.display(),
            root_bundle.manifest.bundle_type.as_str()
        )));
    }
    let root_records: Vec<&CertRecord> = root_bundle
        .manifest
        .certificates
        .iter()
        .filter(|r| r.role == "root")
        .collect();
    let root_algos: Vec<AlgorithmId> = root_records.iter().map(|r| r.algorithm).collect();
    for algo in &slot_algos {
        if !root_algos.contains(algo) {
            return Err(CertError::usage(format!(
                "the root set has no {} root to sign a {} issuer",
                algo.name(),
                algo.name()
            )));
        }
    }
    let graph = GraphPolicy::spec_default();
    let (key_dir, key_location) = key_destination(out_dir, &req.private_store);
    let external = if req.private_store.is_some() {
        Some(key_dir.as_path())
    } else {
        None
    };
    let ou = req.ou.clone().or_else(|| match req.purpose {
        crate::model::IssuerPurpose::WebServer => Some(format!("{} Services", req.name)),
        crate::model::IssuerPurpose::General => None,
    });

    // Load every root signer handle up front (fail before keygen if
    // the signer source is incomplete).
    let mut root_handles: BTreeMap<AlgorithmId, CaHandle> = BTreeMap::new();
    for record in &root_records {
        // Rebuild the root's CN from its record facts: the subject DN
        // was produced by this crate's convention; extract the CN.
        let root_cn = extract_cn(&record.subject_dn).ok_or_else(|| {
            CertError::manifest(format!(
                "root record for {} has no CN in its subject DN",
                record.algorithm.name()
            ))
        })?;
        let handle = load_ca(
            &root_bundle,
            record,
            &req.root_signer,
            &root_bundle.manifest.organization,
            &root_bundle.manifest.country,
            None,
            &root_cn,
            1,
        )?;
        root_handles.insert(record.algorithm, handle);
    }

    let mut artifacts: Vec<Artifact> = Vec::new();
    let mut records = Vec::new();
    let mut warnings = Vec::new();
    for (algo, partition) in &req.slots {
        let gp = GenerationPartition::new(root_bundle.manifest.generation, partition)?;
        let key = generate_key(*algo)?;
        let cn = format!(
            "{} {} Issuing {} CA {}",
            root_bundle.manifest.organization,
            req.name,
            algo.display_name(),
            gp.cn_tail()
        );
        // Clamp the issuer window to the primary root's window.
        let primary_root = &root_handles[algo];
        let (nb, na) = clamp_window(
            req.not_before,
            req.not_after,
            primary_root.cert.not_before,
            primary_root.cert.not_after,
            &cn,
            &mut warnings,
        )?;
        let mut params = ca_params_for(
            &root_bundle.manifest.organization,
            &root_bundle.manifest.country,
            ou.as_deref(),
            &cn,
            0,
            nb,
            na,
        );
        if req.purpose == crate::model::IssuerPurpose::WebServer && !req.permitted_dns.is_empty() {
            params.name_constraints = Some(rcgen::NameConstraints {
                permitted_subtrees: req
                    .permitted_dns
                    .iter()
                    .map(|d| rcgen::GeneralSubtree::DnsName(d.clone()))
                    .collect(),
                excluded_subtrees: Vec::new(),
            });
        }
        let key_file = format!("{}-{}.pk8", algo.name(), partition);
        let key_path = stage_key(&mut artifacts, external, &key_file, &key)?;

        // Signer set: primary first, then policy direct signers.
        let mut signers = vec![*algo];
        if req.cross_signing.issuer_level_active() {
            for signer in graph.issuer_signers_for(*algo) {
                if root_algos.contains(&signer) {
                    signers.push(signer);
                }
            }
        }
        for (i, signer_algo) in signers.iter().enumerate() {
            let signer = &root_handles[signer_algo];
            // Each variant's window clamps to its own signer's window.
            let (snb, sna) = clamp_window(
                nb,
                na,
                signer.cert.not_before,
                signer.cert.not_after,
                &cn,
                &mut warnings,
            )?;
            let mut variant_params = params.clone();
            variant_params.not_before = snb.0;
            variant_params.not_after = sna.0;
            variant_params.serial_number = Some(random_serial());
            let cert = signer.sign(&variant_params, &key)?;
            let der = cert.der().to_vec();
            let rel = format!(
                "public/certs/{}/{}/signed-by-{}.crt.pem",
                algo.name(),
                partition,
                signer_algo.name()
            );
            artifacts.push(Artifact {
                rel_path: rel.clone(),
                kind: "certificate-pem".into(),
                visibility: "public".into(),
                bytes: pem_encode("CERTIFICATE", &der).into_bytes(),
            });
            let mut rec =
                record_from_der(&der, "issuer", partition, i == 0, rel, key_path.clone())?;
            rec.signer_algorithm = Some(*signer_algo);
            records.push(rec);
        }
    }

    let parent = ParentRef {
        bundle_type: BundleType::RootSet,
        bundle_id: root_bundle.manifest.bundle_id.clone(),
        name: root_bundle.manifest.name.clone(),
        anchor_fingerprints: root_records
            .iter()
            .map(|r| r.fingerprint_sha256.clone())
            .collect(),
    };
    let generation = root_bundle.manifest.generation;
    let mut manifest = base_manifest(
        BundleType::IssuerSet,
        &req.name,
        &root_bundle.manifest.organization,
        &root_bundle.manifest.country,
        Some(req.purpose.as_str().to_string()),
        generation,
        Some(parent),
        key_location,
        Some(req.cross_signing.as_str().to_string()),
        warnings,
    )?;
    manifest.bundle_id = format!(
        "issuerset-{}-{}",
        manifest.slug,
        &records
            .first()
            .map(|r| r.fingerprint_sha256[..12].to_string())
            .unwrap_or_default()
    );
    manifest.certificates = records;
    let manifest = write_bundle(out_dir, manifest, &artifacts)?;
    Ok(manifest)
}

/// A subject public key carried over from an issued certificate, so
/// a renewal can certify the key without loading (or ever needing)
/// its private half. `raw` is the SPKI bit-string content: the same
/// bytes [`inspect_key`] returns for the matching private key.
struct ExistingPublicKey {
    raw: Vec<u8>,
    algorithm: &'static rcgen::SignatureAlgorithm,
}

impl rcgen::PublicKeyData for ExistingPublicKey {
    fn der_bytes(&self) -> &[u8] {
        &self.raw
    }
    fn algorithm(&self) -> &'static rcgen::SignatureAlgorithm {
        self.algorithm
    }
}

fn signature_algorithm_for(algo: AlgorithmId) -> &'static rcgen::SignatureAlgorithm {
    match algo {
        AlgorithmId::Ed25519 => &PKCS_ED25519,
        AlgorithmId::P256 => &PKCS_ECDSA_P256_SHA256,
        AlgorithmId::Rsa4096 | AlgorithmId::Rsa2048 => &PKCS_RSA_SHA256,
        AlgorithmId::MlDsa44 => &PKCS_ML_DSA_44,
    }
}

/// What a renewal carries over from one issued issuer certificate:
/// its subject public key and its permitted DNS name constraints.
struct IssuedFacts {
    public_key: Vec<u8>,
    permitted_dns: Vec<String>,
}

fn issued_facts(der: &[u8], what: &str) -> Result<IssuedFacts> {
    use x509_parser::extensions::{GeneralName, ParsedExtension};
    let (_, cert) = x509_parser::prelude::parse_x509_certificate(der)
        .map_err(|e| CertError::manifest(format!("{what}: certificate parse failed: {e}")))?;
    let tbs = &cert.tbs_certificate;
    let mut permitted_dns = Vec::new();
    for ext in tbs.extensions() {
        if let ParsedExtension::NameConstraints(nc) = ext.parsed_extension() {
            if nc.excluded_subtrees.as_ref().is_some_and(|v| !v.is_empty()) {
                return Err(CertError::policy(format!(
                    "{what} carries excluded name constraints, which renewal cannot reproduce"
                )));
            }
            for subtree in nc.permitted_subtrees.iter().flatten() {
                match &subtree.base {
                    GeneralName::DNSName(name) => permitted_dns.push(name.to_string()),
                    other => {
                        return Err(CertError::policy(format!(
                            "{what} carries a non-DNS permitted name constraint ({other:?}), \
                             which renewal cannot reproduce"
                        )));
                    }
                }
            }
        }
    }
    Ok(IssuedFacts {
        public_key: tbs.subject_pki.subject_public_key.data.to_vec(),
        permitted_dns,
    })
}

fn first_certificate_der(bundle: &Bundle, rel_path: &str) -> Result<Vec<u8>> {
    let pem = read_artifact(bundle, rel_path)?;
    crate::bundle::pem_decode_all(
        std::str::from_utf8(&pem)
            .map_err(|_| CertError::manifest(format!("{rel_path} is not UTF-8 PEM")))?,
    )?
    .into_iter()
    .find(|(label, _)| label == "CERTIFICATE")
    .map(|(_, der)| der)
    .ok_or_else(|| CertError::manifest(format!("no CERTIFICATE block in {rel_path}")))
}

/// `cert issuer renew` (§5.2): reissue every certificate of an
/// issuer set on the keys it already has, signed by the same roots.
///
/// Renewal is key reuse, so it runs only with the explicit opt-in.
/// Each issuer certificate keeps its subject name byte for byte, its
/// key, its OU and name constraints and its signer; it gets a fresh
/// serial and the new validity window (clamped to its signer's). The
/// new bundle records `revision` = predecessor + 1 and the
/// predecessor's bundle id, and keeps the predecessor's generation:
/// nothing in it claims the keys rotated. Issuer private keys are
/// never needed to sign; for bundle-private custody they are copied
/// into the new bundle (after checking each matches its certificate)
/// so the renewed set can issue leaves on its own.
pub fn renew_issuer_set(
    req: &crate::model::IssuerRenewRequest,
    out_dir: &Path,
) -> Result<BundleManifest> {
    if !req.allow_key_reuse {
        return Err(CertError::policy(
            "issuer renew reissues the issuer certificates on the same keys; policy allows key \
             reuse only when asked for explicitly (--allow-key-reuse). Key rotation is a \
             separate verb (spec §5.2)",
        ));
    }
    let old = load_bundle(&req.issuer_bundle)?;
    if old.manifest.bundle_type != BundleType::IssuerSet {
        return Err(CertError::usage(format!(
            "{} is a {} bundle, not an issuer set",
            req.issuer_bundle.display(),
            old.manifest.bundle_type.as_str()
        )));
    }
    let root_bundle = load_bundle(&req.root_bundle)?;
    if root_bundle.manifest.bundle_type != BundleType::RootSet {
        return Err(CertError::usage(format!(
            "{} is a {} bundle, not a root set",
            req.root_bundle.display(),
            root_bundle.manifest.bundle_type.as_str()
        )));
    }
    let root_records: Vec<&CertRecord> = root_bundle
        .manifest
        .certificates
        .iter()
        .filter(|r| r.role == "root")
        .collect();
    // The root set must be the one the issuer set was made under:
    // same bundle id and the same anchors.
    let parent = old.manifest.parent.as_ref().ok_or_else(|| {
        CertError::manifest("the issuer set records no parent root set to renew under")
    })?;
    let mut anchors: Vec<&str> = root_records
        .iter()
        .map(|r| r.fingerprint_sha256.as_str())
        .collect();
    anchors.sort_unstable();
    let mut recorded: Vec<&str> = parent
        .anchor_fingerprints
        .iter()
        .map(String::as_str)
        .collect();
    recorded.sort_unstable();
    if parent.bundle_id != root_bundle.manifest.bundle_id || anchors != recorded {
        return Err(CertError::policy(format!(
            "{} is not the root set this issuer set chains to (recorded parent {})",
            req.root_bundle.display(),
            parent.bundle_id
        )));
    }

    let mut root_handles: BTreeMap<AlgorithmId, CaHandle> = BTreeMap::new();
    for record in &root_records {
        let root_cn = extract_cn(&record.subject_dn).ok_or_else(|| {
            CertError::manifest(format!(
                "root record for {} has no CN in its subject DN",
                record.algorithm.name()
            ))
        })?;
        let handle = load_ca(
            &root_bundle,
            record,
            &req.root_signer,
            &root_bundle.manifest.organization,
            &root_bundle.manifest.country,
            None,
            &root_cn,
            1,
        )?;
        root_handles.insert(record.algorithm, handle);
    }

    let bundle_private = old.manifest.key_location == "bundle-private";
    let mut artifacts: Vec<Artifact> = Vec::new();
    let mut copied_keys: Vec<String> = Vec::new();
    let mut records = Vec::new();
    let mut warnings = Vec::new();
    let issuers: Vec<&CertRecord> = old
        .manifest
        .certificates
        .iter()
        .filter(|r| r.role == "issuer")
        .collect();
    if issuers.is_empty() {
        return Err(CertError::manifest(
            "the issuer set holds no issuer certificates",
        ));
    }
    for record in issuers {
        let what = format!("{} ({})", record.subject_dn, record.cert_path);
        let signer_algo = record.signer_algorithm.unwrap_or(record.algorithm);
        let signer = root_handles.get(&signer_algo).ok_or_else(|| {
            CertError::policy(format!(
                "{what} was signed by the {} root, which the root set no longer holds",
                signer_algo.name()
            ))
        })?;
        let old_der = first_certificate_der(&old, &record.cert_path)?;
        let old_info = parse_cert(&old_der)?;
        let facts = issued_facts(&old_der, &what)?;
        let cn = extract_cn(&record.subject_dn)
            .ok_or_else(|| CertError::manifest(format!("{what} has no CN in its subject DN")))?;
        let ou = extract_ou(&record.subject_dn);
        let (nb, na) = clamp_window(
            req.not_before,
            req.not_after,
            signer.cert.not_before,
            signer.cert.not_after,
            &cn,
            &mut warnings,
        )?;
        let mut params = ca_params_for(
            &old.manifest.organization,
            &old.manifest.country,
            ou.as_deref(),
            &cn,
            old_info.path_len.unwrap_or(0),
            nb,
            na,
        );
        if !facts.permitted_dns.is_empty() {
            params.name_constraints = Some(rcgen::NameConstraints {
                permitted_subtrees: facts
                    .permitted_dns
                    .iter()
                    .map(|d| rcgen::GeneralSubtree::DnsName(d.clone()))
                    .collect(),
                excluded_subtrees: Vec::new(),
            });
        }
        let subject_key = ExistingPublicKey {
            raw: facts.public_key.clone(),
            algorithm: signature_algorithm_for(record.algorithm),
        };
        let cert = signer.sign(&params, &subject_key)?;
        let der = cert.der().to_vec();
        let new_info = parse_cert(&der)?;
        if new_info.subject_name_der != old_info.subject_name_der {
            return Err(CertError::engine(format!(
                "{what}: the reissued subject name does not match the original byte for byte"
            )));
        }
        if crate::dns::spki_der(&der)? != crate::dns::spki_der(&old_der)? {
            return Err(CertError::engine(format!(
                "{what}: the reissued certificate does not carry the original key"
            )));
        }

        // Bundle-private keys move into the renewed bundle, once per
        // key, after a check that the key is the certificate's.
        let key_path = match (&record.key_path, bundle_private) {
            (Some(rel), true) => {
                if !copied_keys.contains(rel) {
                    let pkcs8 = read_artifact(&old, rel)?;
                    if inspect_key(record.algorithm, &pkcs8)? != facts.public_key {
                        return Err(CertError::policy(format!(
                            "{rel} does not hold the private key of {what}"
                        )));
                    }
                    artifacts.push(Artifact {
                        rel_path: rel.clone(),
                        kind: "private-key-pkcs8".into(),
                        visibility: "private".into(),
                        bytes: pkcs8,
                    });
                    copied_keys.push(rel.clone());
                }
                Some(rel.clone())
            }
            _ => None,
        };
        let rel = record.cert_path.clone();
        artifacts.push(Artifact {
            rel_path: rel.clone(),
            kind: "certificate-pem".into(),
            visibility: "public".into(),
            bytes: pem_encode("CERTIFICATE", &der).into_bytes(),
        });
        let mut rec = record_from_der(
            &der,
            "issuer",
            &record.partition,
            record.primary,
            rel,
            key_path,
        )?;
        rec.signer_algorithm = record.signer_algorithm;
        records.push(rec);
    }

    let parent = ParentRef {
        bundle_type: BundleType::RootSet,
        bundle_id: root_bundle.manifest.bundle_id.clone(),
        name: root_bundle.manifest.name.clone(),
        anchor_fingerprints: root_records
            .iter()
            .map(|r| r.fingerprint_sha256.clone())
            .collect(),
    };
    let key_location = if bundle_private {
        "bundle-private"
    } else {
        "external-store"
    };
    let mut manifest = base_manifest(
        BundleType::IssuerSet,
        &old.manifest.name,
        &old.manifest.organization,
        &old.manifest.country,
        old.manifest.purpose.clone(),
        old.manifest.generation,
        Some(parent),
        key_location,
        old.manifest.cross_signing.clone(),
        warnings,
    )?;
    manifest.revision = old.manifest.revision + 1;
    manifest.predecessor = Some(old.manifest.bundle_id.clone());
    manifest.bundle_id = format!(
        "issuerset-{}-{}",
        manifest.slug,
        &records
            .first()
            .map(|r| r.fingerprint_sha256[..12].to_string())
            .unwrap_or_default()
    );
    manifest.certificates = records;
    let manifest = write_bundle(out_dir, manifest, &artifacts)?;
    Ok(manifest)
}

fn extract_cn(dn: &str) -> Option<String> {
    // rcgen/x509-parser DN rendering: "CN=..., O=..., C=..." — take
    // the CN RDN value (crate-produced DNs have no escaped commas in
    // the CN by construction of our naming).
    for part in dn.split(", ") {
        if let Some(v) = part.strip_prefix("CN=") {
            return Some(v.to_string());
        }
    }
    None
}

fn clamp_window(
    nb: Rfc3339,
    na: Rfc3339,
    signer_nb: Rfc3339,
    signer_na: Rfc3339,
    what: &str,
    warnings: &mut Vec<String>,
) -> Result<(Rfc3339, Rfc3339)> {
    let nb2 = nb.max(signer_nb);
    let na2 = na.min(signer_na);
    if nb2 != nb || na2 != na {
        warnings.push(format!(
            "{what}: validity clamped to the signer's window ({signer_nb} .. {signer_na})"
        ));
    }
    if na2 <= nb2 {
        return Err(CertError::policy(format!(
            "{what}: requested validity {nb} .. {na} does not intersect the signer's window {signer_nb} .. {signer_na}"
        )));
    }
    Ok((nb2, na2))
}

// ---------------------------------------------------------------- leaves

/// `cert leaf issue`: issue one leaf from an issuer bundle, signed
/// by the selected issuer key loaded from `req.issuer_signer`.
pub fn issue_leaf(req: &LeafIssueRequest, out_dir: &Path) -> Result<BundleManifest> {
    let profile = resolve_profile(&req.profile)?;
    // Request-level policy first: nothing is loaded or signed for a
    // request the profile refuses.
    if profile.san_policy == SanPolicy::Broker {
        check_broker_sans(
            &profile.name,
            req.common_name.as_deref(),
            &req.dns_names,
            &req.ip_addresses,
        )?;
    }
    let csr = match &req.csr {
        Some(path) => {
            if req.private_store.is_some() {
                return Err(CertError::usage(
                    "--private-store has nothing to hold for a CSR-issued leaf: its key stays on the subject's host",
                ));
            }
            Some(read_csr(path)?)
        }
        None if profile.key_custody == KeyCustody::HostCsrOnly => {
            return Err(CertError::policy(format!(
                "{} leaves are issued only from a CSR made on the subject's host (--csr <file>); \
                 the issuer never generates or holds their key",
                profile.name
            )));
        }
        None => None,
    };
    let key_algo = match &csr {
        Some((_, csr_algo)) => {
            if let Some(asked) = req.key_algorithm {
                if asked != *csr_algo {
                    return Err(CertError::usage(format!(
                        "--algorithm {} does not match the CSR's {} key",
                        asked.name(),
                        csr_algo.name()
                    )));
                }
            }
            if profile.key_custody == KeyCustody::HostCsrOnly && *csr_algo != profile.key_algorithm
            {
                return Err(CertError::policy(format!(
                    "{} leaves carry {} keys; the CSR's key is {}",
                    profile.name,
                    profile.key_algorithm.name(),
                    csr_algo.name()
                )));
            }
            *csr_algo
        }
        None => req.key_algorithm.unwrap_or(profile.key_algorithm),
    };
    let issuer_bundle = load_bundle(&req.issuer_bundle)?;
    if issuer_bundle.manifest.bundle_type != BundleType::IssuerSet {
        return Err(CertError::usage(format!(
            "{} is a {} bundle, not an issuer set",
            req.issuer_bundle.display(),
            issuer_bundle.manifest.bundle_type.as_str()
        )));
    }
    if let Some(wanted) = profile.issuer_purpose {
        let got = issuer_bundle.manifest.purpose.as_deref();
        if got != Some(wanted) {
            return Err(CertError::policy(format!(
                "{} leaves issue only from a {wanted} (Base) issuer set; {} is {}",
                profile.name,
                req.issuer_bundle.display(),
                got.unwrap_or("of no recorded purpose")
            )));
        }
    }
    if profile.is_custom {
        if let Some(requested) = req.key_algorithm {
            if requested != profile.key_algorithm {
                return Err(CertError::policy(format!(
                    "profile '{}' fixes key algorithm {}, not {}",
                    profile.name,
                    profile.key_algorithm.name(),
                    requested.name()
                )));
            }
        }
    }
    let issuer_partition = req
        .issuer_partition
        .clone()
        .unwrap_or_else(|| "a".to_string())
        .to_ascii_lowercase();
    // The primary variant record for the selected issuer slot. The
    // signer selection defaults to the requested issuer algorithm,
    // then the leaf key's algorithm, then the set's first primary
    // slot in the partition — a signing profile whose key algorithm
    // has no CA slot (idp-ca's RSA-2048 signing leaves) still signs
    // from a real issuer key.
    let wanted = req.issuer_algorithm.unwrap_or(key_algo);
    let slot_record = issuer_bundle
        .manifest
        .certificates
        .iter()
        .filter(|r| r.role == "issuer" && r.primary && r.partition == issuer_partition)
        .find(|r| r.algorithm == wanted)
        .or_else(|| {
            issuer_bundle
                .manifest
                .certificates
                .iter()
                .find(|r| r.role == "issuer" && r.primary && r.partition == issuer_partition)
        })
        .ok_or_else(|| {
            CertError::usage(format!(
                "issuer set has no primary {} partition {} CA (slots: {})",
                wanted.name(),
                issuer_partition.to_ascii_uppercase(),
                issuer_bundle
                    .manifest
                    .certificates
                    .iter()
                    .filter(|r| r.role == "issuer" && r.primary)
                    .map(|r| format!("{}-{}", r.algorithm.name(), r.partition))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;
    let issuer_cn = extract_cn(&slot_record.subject_dn).ok_or_else(|| {
        CertError::manifest("issuer record has no CN in its subject DN".to_string())
    })?;
    let issuer_ou = extract_ou(&slot_record.subject_dn);
    let issuer = load_ca(
        &issuer_bundle,
        slot_record,
        &req.issuer_signer,
        &issuer_bundle.manifest.organization,
        &issuer_bundle.manifest.country,
        issuer_ou.as_deref(),
        &issuer_cn,
        0,
    )?;

    let mut warnings = Vec::new();
    let (nb, na) = clamp_window(
        req.not_before,
        req.not_after,
        issuer.cert.not_before,
        issuer.cert.not_after,
        &req.name,
        &mut warnings,
    )?;
    if na.0 - nb.0 > time::Duration::days(i64::from(profile.max_validity_days)) {
        return Err(CertError::policy(format!(
            "profile '{}' validity exceeds its {} day ceiling",
            profile.name, profile.max_validity_days
        )));
    }

    // Subject + SANs.
    let mut sans: Vec<String> = req.dns_names.clone();
    let cn = req
        .common_name
        .clone()
        .or_else(|| sans.first().cloned())
        .unwrap_or_else(|| req.name.clone());
    if sans.is_empty() && req.ip_addresses.is_empty() && profile.san_defaults_to_cn {
        sans.push(cn.clone());
    }
    let mut params =
        CertificateParams::new(sans.clone()).map_err(|e| CertError::engine(e.to_string()))?;
    params.subject_alt_names.extend(
        req.ip_addresses
            .iter()
            .map(|ip| rcgen::SanType::IpAddress(*ip)),
    );
    params.distinguished_name = distinguished_name(
        &issuer_bundle.manifest.organization,
        &issuer_bundle.manifest.country,
        None,
        &cn,
    );
    params.is_ca = IsCa::ExplicitNoCa;
    params.key_usages = profile.key_usages;
    params.extended_key_usages = profile.extended_key_usages;
    params.not_before = nb.0;
    params.not_after = na.0;
    params.serial_number = Some(random_serial());
    params.use_authority_key_identifier_extension = true;
    if profile.ocsp_no_check {
        // RFC 6960 §3.2: a delegated OCSP signing certificate
        // carries id-pkix-ocsp-nocheck (extnValue = DER NULL) so
        // verifiers do not revocation-check the responder itself.
        params
            .custom_extensions
            .push(rcgen::CustomExtension::from_oid_content(
                &[1, 3, 6, 1, 5, 5, 7, 48, 1, 5],
                vec![0x05, 0x00],
            ));
    }
    // Leaves embed no AIA, OCSP URL or CDP: revocation is checked by
    // CRL, and a CDP is embedded only once a real signed CRL is
    // published for the issuer (the broker profiles' CRL-only rule).
    let mut artifacts = Vec::new();
    let (der, key_path, key_location) = match &csr {
        Some((public_key, _)) => {
            let cert = issuer.sign(&params, public_key)?;
            (cert.der().to_vec(), None, "host-local")
        }
        None => {
            let key = generate_key(key_algo)?;
            let cert = issuer.sign(&params, &key)?;
            let (key_dir, key_location) = key_destination(out_dir, &req.private_store);
            let external = if req.private_store.is_some() {
                Some(key_dir.as_path())
            } else {
                None
            };
            let key_path = stage_key(&mut artifacts, external, "leaf.pk8", &key)?;
            (cert.der().to_vec(), key_path, key_location)
        }
    };
    let leaf_rel = "leaf.crt.pem".to_string();
    artifacts.push(Artifact {
        rel_path: leaf_rel.clone(),
        kind: "certificate-pem".into(),
        visibility: "public".into(),
        bytes: pem_encode("CERTIFICATE", &der).into_bytes(),
    });
    // chain.pem: leaf + the primary issuer variant.
    let issuer_pem = read_artifact(&issuer_bundle, &slot_record.cert_path)?;
    let mut chain = pem_encode("CERTIFICATE", &der);
    chain.push_str(std::str::from_utf8(&issuer_pem).unwrap_or(""));
    artifacts.push(Artifact {
        rel_path: "chain.pem".into(),
        kind: "chain-pem".into(),
        visibility: "public".into(),
        bytes: chain.into_bytes(),
    });
    let mut rec = record_from_der(&der, "leaf", &issuer_partition, true, leaf_rel, key_path)?;
    rec.signer_algorithm = Some(slot_record.algorithm);

    let parent = ParentRef {
        bundle_type: BundleType::IssuerSet,
        bundle_id: issuer_bundle.manifest.bundle_id.clone(),
        name: issuer_bundle.manifest.name.clone(),
        anchor_fingerprints: vec![slot_record.fingerprint_sha256.clone()],
    };
    let mut manifest = base_manifest(
        BundleType::Leaf,
        &req.name,
        &issuer_bundle.manifest.organization,
        &issuer_bundle.manifest.country,
        Some(req.profile.clone()),
        issuer_bundle.manifest.generation,
        Some(parent),
        key_location,
        None,
        warnings,
    )?;
    manifest.bundle_id = format!("leaf-{}-{}", manifest.slug, &rec.fingerprint_sha256[..12]);
    manifest.certificates = vec![rec];
    let manifest = write_bundle(out_dir, manifest, &artifacts)?;
    Ok(manifest)
}

/// Read a PKCS#10 CSR (PEM or DER), verify its self-signature, and
/// return the key it carries with that key's registered algorithm.
/// Only the key is used: the CSR's subject, SANs and requested
/// extensions are ignored, since the issuance request and the
/// profile decide what the leaf says.
fn read_csr(path: &Path) -> Result<(rcgen::PublicKey, AlgorithmId)> {
    let raw = std::fs::read(path)
        .map_err(|e| CertError::usage(format!("cannot read CSR {}: {e}", path.display())))?;
    let der = match std::str::from_utf8(&raw) {
        Ok(text) if text.contains("-----BEGIN") => crate::bundle::pem_decode_all(text)?
            .into_iter()
            .find(|(label, _)| label == "CERTIFICATE REQUEST" || label == "NEW CERTIFICATE REQUEST")
            .map(|(_, der)| der)
            .ok_or_else(|| {
                CertError::usage(format!(
                    "{} holds no CERTIFICATE REQUEST block",
                    path.display()
                ))
            })?,
        _ => raw,
    };
    let algo = {
        use x509_parser::prelude::FromDer;
        let (_, parsed) =
            x509_parser::certification_request::X509CertificationRequest::from_der(&der)
                .map_err(|e| CertError::usage(format!("{} is not a CSR: {e}", path.display())))?;
        let spki = &parsed.certification_request_info.subject_pki;
        crate::chain::classify_subject(
            &spki.algorithm.algorithm.to_id_string(),
            &spki.subject_public_key.data,
        )
        .ok_or_else(|| {
            CertError::usage(format!(
                "the CSR in {} carries a key of an unregistered algorithm",
                path.display()
            ))
        })?
    };
    let params = rcgen::CertificateSigningRequestParams::from_der(
        &rustls_pki_types::CertificateSigningRequestDer::from(der),
    )
    .map_err(|e| {
        CertError::usage(format!(
            "the CSR in {} was refused (its signature must verify): {e}",
            path.display()
        ))
    })?;
    Ok((params.public_key, algo))
}

fn extract_ou(dn: &str) -> Option<String> {
    for part in dn.split(", ") {
        if let Some(v) = part.strip_prefix("OU=") {
            return Some(v.to_string());
        }
    }
    None
}

// ---------------------------------------------------------------- validation

/// Validate a root bundle end to end: manifest checksums (via
/// load), every self-signed root's self-signature, every cross
/// certificate's signature against its signer root, and the
/// realized graph against the policy's expected inventory.
pub fn validate_root_bundle(dir: &Path) -> Result<Vec<crate::chain::Check>> {
    let bundle = load_bundle(dir)?;
    if bundle.manifest.bundle_type != BundleType::RootSet {
        return Err(CertError::usage(format!(
            "{} is not a root set bundle",
            dir.display()
        )));
    }
    let mut checks = vec![crate::chain::Check {
        name: "manifest-checksums".into(),
        ok: true,
        detail: format!("{} files verified", bundle.manifest.files.len()),
    }];
    let mut infos: BTreeMap<String, CertInfo> = BTreeMap::new();
    for record in &bundle.manifest.certificates {
        let pem = read_artifact(&bundle, &record.cert_path)?;
        let blocks = crate::bundle::pem_decode_all(std::str::from_utf8(&pem).unwrap_or(""))?;
        let der = blocks
            .into_iter()
            .find(|(l, _)| l == "CERTIFICATE")
            .map(|(_, d)| d)
            .ok_or_else(|| CertError::manifest("no CERTIFICATE block"))?;
        infos.insert(record.fingerprint_sha256.clone(), parse_cert(&der)?);
    }
    let by_algo: BTreeMap<AlgorithmId, &CertRecord> = bundle
        .manifest
        .certificates
        .iter()
        .filter(|r| r.role == "root")
        .map(|r| (r.algorithm, r))
        .collect();
    for record in &bundle.manifest.certificates {
        let info = &infos[&record.fingerprint_sha256];
        let (ok, detail) = if record.role == "root" {
            (
                crate::chain::verify_signature(info, info),
                "self-signature verifies".to_string(),
            )
        } else {
            let signer_algo = record.signer_algorithm.unwrap_or(record.algorithm);
            match by_algo.get(&signer_algo) {
                Some(signer_rec) => {
                    let signer_info = &infos[&signer_rec.fingerprint_sha256];
                    (
                        crate::chain::verify_signature(signer_info, info),
                        format!("signed by {} root", signer_algo.name()),
                    )
                }
                None => (false, format!("signer {} root missing", signer_algo.name())),
            }
        };
        checks.push(crate::chain::Check {
            name: format!("signature:{}", record.subject_dn),
            ok,
            detail,
        });
    }
    // Graph inventory vs policy.
    let algos: Vec<AlgorithmId> = by_algo.keys().copied().collect();
    let mode = bundle
        .manifest
        .cross_signing
        .as_deref()
        .map(CrossSigningMode::parse)
        .transpose()?
        .unwrap_or(CrossSigningMode::Both);
    let expected = GraphPolicy::spec_default().expected_root_certificates(&algos, mode);
    let actual = bundle.manifest.certificates.len();
    checks.push(crate::chain::Check {
        name: "graph-inventory".into(),
        ok: expected == actual,
        detail: format!(
            "policy expects {expected} root-level certificates for {} under {}, bundle carries {actual}",
            algos
                .iter()
                .map(|a| a.name())
                .collect::<Vec<_>>()
                .join(","),
            mode.as_str()
        ),
    });
    Ok(checks)
}

/// Collect every certificate from a set of bundles into one pool
/// (for chain building/testing across bundle boundaries).
pub fn collect_pool(bundles: &[&Bundle]) -> Result<Vec<CertInfo>> {
    let mut out = Vec::new();
    for bundle in bundles {
        for record in &bundle.manifest.certificates {
            let pem = read_artifact(bundle, &record.cert_path)?;
            let blocks = crate::bundle::pem_decode_all(std::str::from_utf8(&pem).unwrap_or(""))?;
            for (label, der) in blocks {
                if label == "CERTIFICATE" {
                    out.push(parse_cert(&der)?);
                }
            }
        }
    }
    Ok(out)
}

/// Validate a leaf bundle's certificate against issuer + root
/// bundles under a trust selection.
pub fn validate_leaf(
    leaf_bundle: &Bundle,
    pool_bundles: &[&Bundle],
    trust: TrustSelection,
    at: Rfc3339,
) -> Result<ChainReport> {
    let leaf_record = leaf_bundle
        .manifest
        .certificates
        .iter()
        .find(|r| r.role == "leaf")
        .ok_or_else(|| CertError::manifest("leaf bundle has no leaf record"))?;
    let pem = read_artifact(leaf_bundle, &leaf_record.cert_path)?;
    let blocks = crate::bundle::pem_decode_all(std::str::from_utf8(&pem).unwrap_or(""))?;
    let der = blocks
        .into_iter()
        .find(|(l, _)| l == "CERTIFICATE")
        .map(|(_, d)| d)
        .ok_or_else(|| CertError::manifest("no CERTIFICATE block in leaf"))?;
    let leaf = parse_cert(&der)?;
    let pool = collect_pool(pool_bundles)?;
    Ok(validate_chain(&leaf, &pool, trust, at))
}

/// Fingerprint helper kept public for reports.
pub fn fingerprint_of(der: &[u8]) -> String {
    sha256_hex(der)
}
