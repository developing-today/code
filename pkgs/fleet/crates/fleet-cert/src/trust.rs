//! `fleet cert trust` — trust-bundle assembly, pinning,
//! consumer-targeted export, and drift checks (spec §§3.5, 6.1, 7.5,
//! 13 phase D, 44).
//!
//! Library-first (spec §6.4): the CLI verbs in [`crate::cli`] are thin
//! shells over this module. Three rails hold throughout:
//! - Only self-signed roots become anchors. Selection filters the
//!   root *records* of a root set, then each candidate is re-parsed
//!   and proven self-signed (name match + signature verification) —
//!   the `role` label alone never admits an anchor.
//! - Nothing here installs trust anywhere. `export` writes PEM files
//!   plus a manifest carrying per-consumer install/verify/remove
//!   guidance as *documentation*; there is no privileged-install verb
//!   (production-gated per the #9155 gate map, spec §44).
//! - Native OpenSSH trust is not X.509 (spec §44 table): the `ssh`
//!   consumer is refused with an explanation, not a bundle.

use crate::algorithm::AlgorithmId;
use crate::bundle::{Bundle, pem_decode_all, pem_encode, read_artifact, sha256_hex};
use crate::chain::{CertInfo, Check, TrustSelection, parse_cert, verify_signature};
use crate::error::{CertError, Result};
use crate::timeparse::Rfc3339;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Which self-signed roots a trust bundle carries (spec §3.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustBundleKind {
    /// Every self-signed root (`roots-all.pem`): compatible, but not
    /// inherently PQ-secure (§3.5).
    All,
    /// The self-signed classical roots (`roots-classical.pem`).
    Classical,
    /// The ML-DSA root alone (`roots-pq.pem`), with the all-ML-DSA
    /// path rule enforced by the chain validator (§3.2).
    Pq,
}

impl TrustBundleKind {
    pub fn parse(input: &str) -> Result<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "all" | "roots-all" => Ok(TrustBundleKind::All),
            "classical" | "roots-classical" => Ok(TrustBundleKind::Classical),
            "pq" | "roots-pq" => Ok(TrustBundleKind::Pq),
            other => Err(CertError::usage(format!(
                "unknown trust bundle '{other}' (all | classical | pq)"
            ))),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            TrustBundleKind::All => "all",
            TrustBundleKind::Classical => "classical",
            TrustBundleKind::Pq => "pq",
        }
    }

    /// The §3.5 file name for this bundle.
    pub fn file_name(&self) -> &'static str {
        match self {
            TrustBundleKind::All => "roots-all.pem",
            TrustBundleKind::Classical => "roots-classical.pem",
            TrustBundleKind::Pq => "roots-pq.pem",
        }
    }

    /// The chain-validator trust selection with the same membership.
    pub fn trust_selection(&self) -> TrustSelection {
        match self {
            TrustBundleKind::All => TrustSelection::All,
            TrustBundleKind::Classical => TrustSelection::Classical,
            TrustBundleKind::Pq => TrustSelection::Pq,
        }
    }

    /// Whether a subject-key algorithm belongs in this bundle.
    pub fn admits(&self, algo: AlgorithmId) -> bool {
        match self {
            TrustBundleKind::All => true,
            TrustBundleKind::Classical => algo.is_classical(),
            TrustBundleKind::Pq => algo.is_post_quantum(),
        }
    }
}

/// A trust bundle assembled from a root set: the admitted anchors,
/// their concatenated PEM, and the SHA-256 of that PEM.
#[derive(Debug, Clone)]
pub struct AssembledBundle {
    pub kind: TrustBundleKind,
    pub certificates: Vec<CertInfo>,
    pub pem: String,
    pub sha256: String,
}

/// Parse one PEM bundle's CERTIFICATE blocks (spec §7.5: PEM text is
/// the interchange shape; non-PEM text between blocks is ignored).
pub fn parse_pem_certs(pem_text: &str) -> Result<Vec<CertInfo>> {
    let mut out = Vec::new();
    for (label, der) in pem_decode_all(pem_text)? {
        if label != "CERTIFICATE" {
            continue;
        }
        out.push(parse_cert(&der)?);
    }
    Ok(out)
}

/// Assemble a §3.5 trust bundle from a root set's self-signed roots.
///
/// Only records with role `root` are candidates, and each candidate
/// must prove self-signed (subject/issuer name match plus a
/// self-verifying signature) — a `root-cross` record sharing the
/// same key never becomes an anchor, and cross-certs are never
/// silently added as trust anchors (§3.5).
pub fn assemble(root_bundle: &Bundle, kind: TrustBundleKind) -> Result<AssembledBundle> {
    if root_bundle.manifest.bundle_type != crate::model::BundleType::RootSet {
        return Err(CertError::usage(format!(
            "{} is a {} bundle, not a root set",
            root_bundle.dir.display(),
            root_bundle.manifest.bundle_type.as_str()
        )));
    }
    let mut certs = Vec::new();
    for record in root_bundle
        .manifest
        .certificates
        .iter()
        .filter(|r| r.role == "root")
    {
        let bytes = read_artifact(root_bundle, &record.cert_path)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            CertError::manifest(format!("{} is not valid UTF-8 PEM", record.cert_path))
        })?;
        let parsed = parse_pem_certs(text)?;
        let Some(info) = parsed.into_iter().next() else {
            return Err(CertError::manifest(format!(
                "root record {} indexes no certificate",
                record.cert_path
            )));
        };
        if info.subject_name_der != info.issuer_name_der || !verify_signature(&info, &info) {
            return Err(CertError::manifest(format!(
                "root record {} is not self-signed; refusing to anchor it",
                record.cert_path
            )));
        }
        match info.subject_algorithm {
            Some(algo) if kind.admits(algo) => certs.push(info),
            Some(algo) => {
                let _ = algo;
            }
            None => {
                return Err(CertError::manifest(format!(
                    "root {} uses an unregistered subject algorithm ({}); cannot classify it for a {} bundle",
                    info.subject_dn,
                    info.spki_oid(),
                    kind.as_str()
                )));
            }
        }
    }
    // Deterministic order: sort by fingerprint so the same root set
    // always assembles the same bytes.
    certs.sort_by(|a, b| a.fingerprint_sha256.cmp(&b.fingerprint_sha256));
    let mut pem = String::new();
    for cert in &certs {
        pem.push_str(&pem_encode("CERTIFICATE", &cert.der));
    }
    let sha256 = sha256_hex(pem.as_bytes());
    Ok(AssembledBundle {
        kind,
        certificates: certs,
        pem,
        sha256,
    })
}

// ---------------------------------------------------------------- pins

/// Schema id for the pinset files `trust pin` writes.
pub const TRUST_PINS_SCHEMA: &str = "fleet-cert-trust-pins/v1";

/// One pinned anchor: a subject name bound to the SHA-256 fingerprint
/// of the exact DER the operator approved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustPin {
    pub subject_dn: String,
    pub algorithm: String,
    pub fingerprint_sha256: String,
}

/// A pinset: the operator-approved anchor set for one trust bundle,
/// with the bundle identity it was taken against so a later
/// generation is a loud mismatch, not a silent pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustPins {
    pub schema: String,
    pub bundle_id: String,
    pub bundle_name: String,
    pub trust: String,
    pub created_at: Rfc3339,
    pub pins: Vec<TrustPin>,
}

/// Record the current bundle fingerprints as a pinset the operator
/// can store, review, and later check drift against.
pub fn create_pins(root_bundle: &Bundle, kind: TrustBundleKind) -> Result<TrustPins> {
    let assembled = assemble(root_bundle, kind)?;
    let mut pins: Vec<TrustPin> = assembled
        .certificates
        .iter()
        .map(|c| TrustPin {
            subject_dn: c.subject_dn.clone(),
            algorithm: c
                .subject_algorithm
                .map(|a| a.name().to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            fingerprint_sha256: c.fingerprint_sha256.clone(),
        })
        .collect();
    pins.sort_by(|a, b| a.fingerprint_sha256.cmp(&b.fingerprint_sha256));
    Ok(TrustPins {
        schema: TRUST_PINS_SCHEMA.to_string(),
        bundle_id: root_bundle.manifest.bundle_id.clone(),
        bundle_name: root_bundle.manifest.name.clone(),
        trust: kind.as_str().to_string(),
        created_at: Rfc3339::now(),
        pins,
    })
}

/// Check a pinset against the live bundle: every pin must still
/// resolve to a bundle anchor, and every bundle anchor must be
/// pinned. A rotated-out anchor, a new unpinned anchor, or pins taken
/// against another bundle generation each fail loudly.
pub fn check_pins(root_bundle: &Bundle, kind: TrustBundleKind, pins: &TrustPins) -> Vec<Check> {
    let assembled = match assemble(root_bundle, kind) {
        Ok(a) => a,
        Err(e) => {
            return vec![Check {
                name: "bundle-assembles".into(),
                ok: false,
                detail: e.to_string(),
            }];
        }
    };
    let mut checks = Vec::new();
    if pins.schema != TRUST_PINS_SCHEMA {
        checks.push(Check {
            name: "pins-schema".into(),
            ok: false,
            detail: format!(
                "unsupported pinset schema '{}' (expected {TRUST_PINS_SCHEMA})",
                pins.schema
            ),
        });
    }
    if pins.trust != kind.as_str() {
        checks.push(Check {
            name: "pins-trust".into(),
            ok: false,
            detail: format!(
                "pinset is for {} trust but verification requested {}",
                pins.trust,
                kind.as_str()
            ),
        });
    }
    if pins.bundle_id != root_bundle.manifest.bundle_id {
        checks.push(Check {
            name: "pins-target".into(),
            ok: false,
            detail: format!(
                "pinset was taken against bundle {} but the directory holds {} — rotation or replacement, re-pin explicitly",
                pins.bundle_id, root_bundle.manifest.bundle_id
            ),
        });
    }
    let live: std::collections::BTreeMap<&str, &CertInfo> = assembled
        .certificates
        .iter()
        .map(|c| (c.fingerprint_sha256.as_str(), c))
        .collect();
    for pin in &pins.pins {
        match live.get(pin.fingerprint_sha256.as_str()) {
            Some(_) => {}
            None => checks.push(Check {
                name: format!("pin-missing:{}", pin.subject_dn),
                ok: false,
                detail: format!(
                    "pinned anchor {} ({}) is no longer in the {} bundle — rotated out or replaced",
                    pin.subject_dn,
                    &pin.fingerprint_sha256[..12.min(pin.fingerprint_sha256.len())],
                    kind.as_str()
                ),
            }),
        }
    }
    let pinned: std::collections::BTreeSet<&str> = pins
        .pins
        .iter()
        .map(|p| p.fingerprint_sha256.as_str())
        .collect();
    for cert in &assembled.certificates {
        if !pinned.contains(cert.fingerprint_sha256.as_str()) {
            checks.push(Check {
                name: format!("pin-unpinned:{}", cert.subject_dn),
                ok: false,
                detail: format!(
                    "bundle anchor {} ({}) is not in the pinset — new anchor, pin it explicitly",
                    cert.subject_dn,
                    &cert.fingerprint_sha256[..12.min(cert.fingerprint_sha256.len())]
                ),
            });
        }
    }
    if checks.is_empty() {
        checks.push(Check {
            name: "pins-match".into(),
            ok: true,
            detail: format!(
                "{} pins match {} bundle anchors ({} trust)",
                pins.pins.len(),
                assembled.certificates.len(),
                kind.as_str()
            ),
        });
    }
    checks
}

// ------------------------------------------------------------ consumers

/// A named consumer class from the §44 interop matrix: which trust
/// bundle it takes plus install/verify/remove guidance. The guidance
/// is documentation carried in the export manifest — `export` never
/// touches the consumer's stores.
#[derive(Debug, Clone)]
pub struct ConsumerTarget {
    /// Canonical target name (`--target` value).
    pub name: &'static str,
    /// Also accepted spellings.
    pub aliases: &'static [&'static str],
    /// Which bundle the target takes. `None` is unused (every table
    /// row takes a bundle); `ssh` is refused before lookup.
    pub kind: TrustBundleKind,
    /// Why this target takes that bundle (the §44 caveat).
    pub note: &'static str,
    /// Install steps for the operator (docs, not executed).
    pub install: &'static str,
    /// How to verify the install (docs, not executed).
    pub verify: &'static str,
    /// How to remove/roll back (docs, not executed).
    pub remove: &'static str,
}

const CONSUMERS: &[ConsumerTarget] = &[
    ConsumerTarget {
        name: "debian",
        aliases: &["ubuntu", "debian/ubuntu", "linux-deb"],
        kind: TrustBundleKind::All,
        note: "Full-compatibility client: takes all four self-signed roots. Some distro trust updates expect .crt files with particular PEM contents (§44).",
        install: "Copy the per-root .crt.pem files to /usr/local/share/ca-certificates/ (suffix .crt) and run update-ca-certificates. Requires root; prefer a change window over silent automation.",
        verify: "openssl verify -CAfile /etc/ssl/certs/ca-certificates.crt against a leaf chain issued by this fleet; compare anchor fingerprints with export.json.",
        remove: "Delete the copied .crt files and re-run update-ca-certificates; re-verify the anchors are gone.",
    },
    ConsumerTarget {
        name: "rhel",
        aliases: &["fedora", "rhel/fedora", "linux-rpm"],
        kind: TrustBundleKind::All,
        note: "Full-compatibility client: takes all four roots. System NSS/OpenSSL and application trust may differ (§44).",
        install: "Place the per-root PEM files under the update-ca-trust anchors path and run update-ca-trust extract. Requires root.",
        verify: "trust list | grep -i <org>; openssl verify against a fleet-issued chain; compare fingerprints with export.json.",
        remove: "Remove the anchor files, re-run update-ca-trust extract, and re-verify.",
    },
    ConsumerTarget {
        name: "alpine",
        aliases: &["alpine/minimal", "containers", "distroless"],
        kind: TrustBundleKind::All,
        note: "Full-compatibility client. Do not assume a CA path exists in scratch/distroless images; inject at image build and test the image (§44).",
        install: "Bake the bundle PEM into the image (ca-certificates package or a copied bundle) at build time; never bake private leaf keys into images.",
        verify: "Run the image's TLS client against a fleet-issued endpoint in a test harness; compare fingerprints with export.json.",
        remove: "Rebuild the image without the bundle; never mutate a running minimal image's trust by hand.",
    },
    ConsumerTarget {
        name: "windows",
        aliases: &["win", "gpo", "mdm-windows"],
        kind: TrustBundleKind::All,
        note: "Full-compatibility client. Trust placement requires authorization; chain-engine and revocation policies vary (§44).",
        install: "Import the per-root PEMs into LocalMachine (fleet-wide) or CurrentUser via certutil or an enterprise/GPO/MDM push, with privileged approval.",
        verify: "certutil -store root; TLS-probe a fleet-issued endpoint from the target account; compare fingerprints with export.json.",
        remove: "Remove the roots from the store (certutil -delstore) or retract the GPO/MDM profile; re-verify.",
    },
    ConsumerTarget {
        name: "macos",
        aliases: &["mac", "keychain"],
        kind: TrustBundleKind::All,
        note: "Full-compatibility client. Import alone may not set the intended trust behavior (§44): set explicit trust policy.",
        install: "Import the per-root PEMs into a managed certificate profile or the System keychain, then set explicit trust policy. Requires admin.",
        verify: "security find-certificate + a TLS probe from the target context; compare fingerprints with export.json.",
        remove: "Delete the keychain items or retract the profile; re-verify the policy is gone.",
    },
    ConsumerTarget {
        name: "java",
        aliases: &["jvm", "keytool", "cacerts"],
        kind: TrustBundleKind::Classical,
        note: "Takes the classical roots only: a Java runtime might not support ML-DSA, and JVM trust is classpath/runtime specific (§44).",
        install: "Import the per-root classical PEMs into a dedicated truststore with keytool -importcert (never edit the JDK-global cacerts for one fleet); restarts may be required.",
        verify: "keytool -list on the truststore plus a TLS probe from the JVM; compare fingerprints with export.json.",
        remove: "keytool -delete the aliases (or retire the dedicated truststore); restart affected JVMs and re-verify.",
    },
    ConsumerTarget {
        name: "firefox",
        aliases: &["nss", "firefox/nss", "certutil"],
        kind: TrustBundleKind::All,
        note: "Full-compatibility client. Browser profile and OS integration vary by version (§44).",
        install: "Add the per-root PEMs with certutil -A to the NSS database (or the enterprise-roots path), per Firefox version.",
        verify: "Browse a fleet-issued endpoint in the target profile; compare fingerprints with export.json.",
        remove: "certutil -D the nicknames (or retract the enterprise policy); re-verify in a fresh profile.",
    },
    ConsumerTarget {
        name: "chrome",
        aliases: &["chromium"],
        kind: TrustBundleKind::All,
        note: "Takes all four roots via the platform store on most OSes, but do not assert Chrome uses the same store everywhere (§44).",
        install: "Install via the OS trust path (see debian/rhel/windows/macos targets) or enterprise policy; no Chrome-specific private store exists on most platforms.",
        verify: "TLS-probe a fleet-issued endpoint in Chrome on the target OS; compare fingerprints with export.json.",
        remove: "Retract from the OS store or policy; re-verify on the same OS.",
    },
    ConsumerTarget {
        name: "node",
        aliases: &["nodejs", "electron", "python", "node/python", "certifi"],
        kind: TrustBundleKind::All,
        note: "Language runtimes may bundle their own CA sets (certifi, system OpenSSL, NODE_EXTRA_CA_CERTS): added system roots may not affect bundled or pinned CAs (§44).",
        install: "Point NODE_EXTRA_CA_CERTS (or the certifi/requests context) at the bundle PEM for the target app; do not edit the language's bundled store in place.",
        verify: "HTTPS-probe a fleet-issued endpoint from the runtime; compare fingerprints with export.json.",
        remove: "Unset the override / restore the previous bundle; re-probe.",
    },
    ConsumerTarget {
        name: "go",
        aliases: &["golang", "rustls", "rust", "go/rust"],
        kind: TrustBundleKind::All,
        note: "Takes all four roots; algorithms and X.509 extensions differ by library/release, so record the tested versions (§44).",
        install: "Load the bundle PEM into the app's root pool (system pool or an explicit pool file); for webpki/rustls verify ML-DSA support in the pinned version first.",
        verify: "TLS-probe a fleet-issued endpoint with the pinned library versions; compare fingerprints with export.json.",
        remove: "Revert the pool configuration; re-probe with the previous roots.",
    },
    ConsumerTarget {
        name: "android",
        aliases: &["ios", "mobile", "mdm-mobile"],
        kind: TrustBundleKind::Classical,
        note: "Takes the classical roots: PQ support may be absent, and some apps do not trust user-installed roots — prefer managed profiles/MDM (§44).",
        install: "Push the classical per-root PEMs via a managed root profile (MDM); user-installed roots are a fallback, not the plan.",
        verify: "Probe a fleet-issued endpoint from the managed app profile with network-security-config logging; compare fingerprints with export.json.",
        remove: "Retract the MDM profile; confirm the anchors are gone on-device.",
    },
    ConsumerTarget {
        name: "kubernetes",
        aliases: &["k8s", "trust-manager", "cert-manager"],
        kind: TrustBundleKind::All,
        note: "trust-manager distributes public root bundles via ConfigMaps/Secrets; it is not a way to distribute private keys (§44).",
        install: "Publish the bundle PEM through trust-manager (Bundle resource → ConfigMap/Secret per namespace); cert-manager private CAs stay separate.",
        verify: "Mount the generated ConfigMap in a test pod and openssl-verify a fleet-issued chain; compare fingerprints with export.json.",
        remove: "Delete the Bundle resource; confirm the generated ConfigMaps are garbage-collected.",
    },
    ConsumerTarget {
        name: "firmware",
        aliases: &["embedded", "esp32", "iot"],
        kind: TrustBundleKind::All,
        note: "Takes all four roots, but verify each implemented algorithm in the target firmware first: ML-DSA certs and signatures are substantially larger than Ed25519; prefer per-root DER profiles for flash/RAM (§44).",
        install: "Convert the needed per-root PEM to DER for the firmware profile and flash it through the device's key provisioning flow; size the partition for ML-DSA.",
        verify: "On-device TLS handshake against a fleet-issued endpoint in a hardware-in-the-loop test; compare fingerprints with export.json.",
        remove: "Re-provision the trust partition without the fleet roots; re-run the handshake test (must fail closed).",
    },
    ConsumerTarget {
        name: "pq-only",
        aliases: &["pq", "pq-enforced", "ml-dsa-only"],
        kind: TrustBundleKind::Pq,
        note: "PQ-enforced clients install the ML-DSA trust anchor alone and enforce all-ML-DSA path signatures where the application library permits (§44).",
        install: "Install only roots-pq.pem through the client's trust mechanism; do not add classical roots to a PQ-enforcing store.",
        verify: "Validate an all-ML-DSA chain (fleet cert chain test --trust pq) and confirm a classical chain is rejected; compare fingerprints with export.json.",
        remove: "Remove the ML-DSA anchor; confirm PQ validation fails closed.",
    },
];

/// Look up a consumer target by name or alias (case-insensitive).
pub fn consumer_target(name: &str) -> Option<&'static ConsumerTarget> {
    let want = name.trim().to_ascii_lowercase();
    CONSUMERS
        .iter()
        .find(|c| c.name == want || c.aliases.iter().any(|a| *a == want))
}

/// Every known `--target` name (for usage text).
pub fn consumer_names() -> Vec<&'static str> {
    CONSUMERS.iter().map(|c| c.name).collect()
}

/// Refuse the `ssh` target: native OpenSSH certificates are not
/// X.509, so root X.509 PEMs cannot substitute for SSH trusted CA
/// keys (§44 table). This is a policy refusal, not a missing
/// feature.
pub fn refuse_ssh() -> CertError {
    CertError::policy(
        "target ssh takes no X.509 trust bundle: native OpenSSH certificates are not X.509 — \
        use @cert-authority lines, TrustedUserCAKeys, and host/user principal rules with the \
        fleet SSH credential objects instead (spec §44)",
    )
}

// -------------------------------------------------------------- export

/// Schema id for the export manifests `trust export` writes.
pub const TRUST_EXPORT_SCHEMA: &str = "fleet-cert-trust-export/v1";

/// One exported anchor in the export manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportCertificate {
    pub subject_dn: String,
    pub algorithm: String,
    pub fingerprint_sha256: String,
    /// Export-relative path of the per-root PEM file.
    pub file: String,
}

/// The export manifest: what was exported, for whom, from which
/// bundle — plus the per-consumer install/verify/remove guidance as
/// documentation. The bundle PEM hash lets the operator detect a
/// hand-edited export before installing it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportManifest {
    pub schema: String,
    pub consumer: String,
    pub trust: String,
    pub bundle_id: String,
    pub bundle_name: String,
    pub created_at: Rfc3339,
    pub bundle_file: String,
    pub bundle_sha256: String,
    pub certificates: Vec<ExportCertificate>,
    pub install: String,
    pub verify: String,
    pub remove: String,
    pub note: String,
}

/// Write a consumer-targeted export: the bundle PEM (`roots-<kind>.pem`),
/// one PEM per anchor (`per-root/<algo>.pem`), and `export.json`.
///
/// The output directory must not already contain an export manifest —
/// overwriting an export is an operator decision, not a default.
pub fn write_export(
    out_dir: &Path,
    root_bundle: &Bundle,
    consumer: &str,
    target: &ConsumerTarget,
    assembled: &AssembledBundle,
) -> Result<ExportManifest> {
    let manifest_path = out_dir.join("export.json");
    if manifest_path.exists() {
        return Err(CertError::policy(format!(
            "export already exists at {} (export.json present); remove it explicitly before re-exporting",
            out_dir.display()
        )));
    }
    std::fs::create_dir_all(out_dir.join("per-root"))?;
    let bundle_file = assembled.kind.file_name().to_string();
    std::fs::write(out_dir.join(&bundle_file), &assembled.pem)?;
    let mut certificates = Vec::new();
    for cert in &assembled.certificates {
        let algo_name = cert
            .subject_algorithm
            .map(|a| a.name().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let file = format!("per-root/{algo_name}.pem");
        std::fs::write(out_dir.join(&file), pem_encode("CERTIFICATE", &cert.der))?;
        certificates.push(ExportCertificate {
            subject_dn: cert.subject_dn.clone(),
            algorithm: algo_name,
            fingerprint_sha256: cert.fingerprint_sha256.clone(),
            file,
        });
    }
    certificates.sort_by(|a, b| a.fingerprint_sha256.cmp(&b.fingerprint_sha256));
    let manifest = ExportManifest {
        schema: TRUST_EXPORT_SCHEMA.to_string(),
        consumer: consumer.to_string(),
        trust: assembled.kind.as_str().to_string(),
        bundle_id: root_bundle.manifest.bundle_id.clone(),
        bundle_name: root_bundle.manifest.name.clone(),
        created_at: Rfc3339::now(),
        bundle_file,
        bundle_sha256: assembled.sha256.clone(),
        certificates,
        install: target.install.to_string(),
        verify: target.verify.to_string(),
        remove: target.remove.to_string(),
        note: target.note.to_string(),
    };
    let text = serde_json::to_string_pretty(&manifest)?;
    std::fs::write(&manifest_path, text + "\n")?;
    Ok(manifest)
}

// ---------------------------------------------------------------- store

/// Read a consumer trust store: a PEM file, or a directory whose
/// `.pem`/`.crt`/`.cer` files are each parsed. Fingerprints identify
/// certificates, so formatting differences never count as drift.
pub fn read_store_certs(store: &Path) -> Result<Vec<CertInfo>> {
    let meta = std::fs::metadata(store)
        .map_err(|e| CertError::manifest(format!("store {} unreadable: {e}", store.display())))?;
    let mut files = Vec::new();
    if meta.is_dir() {
        let mut entries: Vec<_> = std::fs::read_dir(store)?
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.extension()
                        .and_then(|x| x.to_str())
                        .map(|x| ["pem", "crt", "cer"].contains(&x.to_ascii_lowercase().as_str()))
                        .unwrap_or(false)
            })
            .collect();
        entries.sort();
        files = entries;
    } else {
        files.push(store.to_path_buf());
    }
    let mut out = Vec::new();
    for file in &files {
        let bytes = std::fs::read(file)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            CertError::manifest(format!(
                "store file {} is not valid UTF-8 PEM",
                file.display()
            ))
        })?;
        out.extend(parse_pem_certs(text)?);
    }
    Ok(out)
}

/// Drift-check a consumer store against the freshly assembled bundle:
/// every bundle anchor must be in the store, and the store must hold
/// nothing the bundle does not. Either direction fails loudly —
/// a missing anchor is a stale consumer, an unknown extra is a
/// possible injection, and both deserve an operator, not silence.
pub fn check_store(assembled: &AssembledBundle, store: &[CertInfo]) -> Vec<Check> {
    let live: std::collections::BTreeSet<&str> = assembled
        .certificates
        .iter()
        .map(|c| c.fingerprint_sha256.as_str())
        .collect();
    let held: std::collections::BTreeSet<&str> = store
        .iter()
        .map(|c| c.fingerprint_sha256.as_str())
        .collect();
    let by_fp = |fp: &str| {
        store
            .iter()
            .find(|c| c.fingerprint_sha256 == fp)
            .map(|c| c.subject_dn.clone())
            .unwrap_or_else(|| format!("sha256:{fp}"))
    };
    let mut checks = Vec::new();
    for cert in &assembled.certificates {
        if !held.contains(cert.fingerprint_sha256.as_str()) {
            checks.push(Check {
                name: format!("store-missing:{}", cert.subject_dn),
                ok: false,
                detail: format!(
                    "bundle anchor {} ({}) is not in the consumer store — stale consumer, re-export",
                    cert.subject_dn,
                    &cert.fingerprint_sha256[..12.min(cert.fingerprint_sha256.len())]
                ),
            });
        }
    }
    for fp in held.difference(&live) {
        checks.push(Check {
            name: format!("store-extra:{fp}"),
            ok: false,
            detail: format!(
                "consumer store holds {} which is not a bundle anchor — unknown trust, investigate before keeping",
                by_fp(fp)
            ),
        });
    }
    if checks.is_empty() {
        checks.push(Check {
            name: "store-matches".into(),
            ok: true,
            detail: format!(
                "consumer store holds exactly the {} {} anchor(s)",
                assembled.certificates.len(),
                assembled.kind.as_str()
            ),
        });
    }
    checks
}
