//! `cert root replace` and `cert issuer replace` (spec §5.2): the
//! replacement orchestration over the renew/rotate verbs.
//!
//! Replace is explicit intent to move a hierarchy to new keys in one
//! step. `root replace` rotates the root set, moves every named issuer
//! set onto the new roots with an overlap (the old roots also certify
//! each new issuer key), and verifies every new chain: primaries to
//! the new roots, overlap certificates to the old roots. `issuer
//! replace` rotates the named issuer sets under the root set they
//! already chain to. Both leave the predecessors untouched and return
//! the distribution, activation and retirement steps that follow,
//! with the date after which the old material can be retired.
//!
//! Everything lands under one new output directory, which is removed
//! again if any step fails, so a failed replacement leaves nothing
//! half-made behind.

use crate::bundle::{Bundle, load_bundle, pem_decode_all, read_artifact};
use crate::chain::{Check, TrustSelection, parse_cert, validate_chain};
use crate::engine;
use crate::error::{CertError, Result};
use crate::model::{
    BundleManifest, BundleType, CertRecord, IssuerRotateRequest, RootRotateRequest, SignerSource,
};
use crate::timeparse::Rfc3339;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// A replacement request. `rotate_roots` selects `root replace`.
#[derive(Debug, Clone)]
pub struct ReplaceRequest {
    /// The root set in use now.
    pub root_bundle: PathBuf,
    /// REQUIRED explicit signer source for its keys (§24.3): they
    /// sign the overlap (`root replace`) or the new issuer keys
    /// (`issuer replace`).
    pub root_signer: SignerSource,
    /// The issuer sets to move; each must chain to `root_bundle`.
    pub issuer_bundles: Vec<PathBuf>,
    pub rotate_roots: bool,
    pub not_before: Rfc3339,
    pub root_not_after: Rfc3339,
    pub issuer_not_after: Rfc3339,
    pub allow_legacy_root_strength: bool,
    /// `None` = new keys inside the new bundles; `Some(dir)` = an
    /// external store, with `roots/` and `issuers/<slug>/` beneath it.
    pub private_store: Option<PathBuf>,
}

/// One bundle the replacement wrote.
#[derive(Debug, Clone, Serialize)]
pub struct Replaced {
    pub dir: String,
    pub predecessor: String,
    pub manifest: BundleManifest,
}

/// What a replacement did and what is left to do.
#[derive(Debug, Clone, Serialize)]
pub struct ReplaceReport {
    pub root: Option<Replaced>,
    pub issuers: Vec<Replaced>,
    /// Every new chain, verified.
    pub checks: Vec<Check>,
    /// The last expiry among the predecessor issuer certificates:
    /// nothing they signed can be valid after it, so the old
    /// material can be retired then.
    pub retire_old_after: Rfc3339,
    pub next_steps: Vec<String>,
}

/// Run a replacement into `out_dir`, which must not exist yet.
pub fn replace(req: &ReplaceRequest, out_dir: &Path) -> Result<ReplaceReport> {
    let verb = if req.rotate_roots {
        "root replace"
    } else {
        "issuer replace"
    };
    if out_dir.exists() {
        return Err(CertError::usage(format!(
            "{verb} writes a new directory; {} already exists",
            out_dir.display()
        )));
    }
    if req.issuer_bundles.is_empty() {
        return Err(CertError::usage(format!(
            "{verb} needs at least one issuer set to move (--issuer <dir>, repeatable)"
        )));
    }
    let old_root = load_bundle(&req.root_bundle)?;
    if old_root.manifest.bundle_type != BundleType::RootSet {
        return Err(CertError::usage(format!(
            "{} is a {} bundle, not a root set",
            req.root_bundle.display(),
            old_root.manifest.bundle_type.as_str()
        )));
    }
    // Preflight every issuer set before anything is generated.
    let mut olds: Vec<(PathBuf, Bundle)> = Vec::new();
    for path in &req.issuer_bundles {
        let bundle = load_bundle(path)?;
        if bundle.manifest.bundle_type != BundleType::IssuerSet {
            return Err(CertError::usage(format!(
                "{} is a {} bundle, not an issuer set",
                path.display(),
                bundle.manifest.bundle_type.as_str()
            )));
        }
        let slug = &bundle.manifest.slug;
        if olds.iter().any(|(_, b)| &b.manifest.slug == slug) {
            return Err(CertError::usage(format!(
                "two issuer sets share the name slug '{slug}'; replace them separately"
            )));
        }
        olds.push((path.clone(), bundle));
    }
    // Every directory this run creates, so a failure removes them all.
    let mut created: Vec<PathBuf> = vec![out_dir.to_path_buf()];
    if let Some(store) = &req.private_store {
        if req.rotate_roots {
            created.push(store.join("roots"));
        }
        for (_, b) in &olds {
            created.push(store.join("issuers").join(&b.manifest.slug));
        }
    }
    for dir in &created[1..] {
        if dir.exists() {
            return Err(CertError::usage(format!(
                "{verb} writes new keys to {}, which already exists",
                dir.display()
            )));
        }
    }

    let result = run(req, verb, out_dir, &old_root, &olds);
    if result.is_err() {
        for dir in &created {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
    result
}

fn run(
    req: &ReplaceRequest,
    verb: &str,
    out_dir: &Path,
    old_root: &Bundle,
    olds: &[(PathBuf, Bundle)],
) -> Result<ReplaceReport> {
    let mut report_root = None;
    let (root_dir, root_signer) = if req.rotate_roots {
        let dir = out_dir.join("roots");
        let store = req.private_store.as_ref().map(|s| s.join("roots"));
        let manifest = engine::rotate_root_set(
            &RootRotateRequest {
                root_bundle: req.root_bundle.clone(),
                not_before: req.not_before,
                not_after: req.root_not_after,
                allow_legacy_root_strength: req.allow_legacy_root_strength,
                private_store: store.clone(),
            },
            &dir,
        )?;
        let signer = SignerSource {
            key_dir: store.unwrap_or_else(|| dir.join("private/keys")),
        };
        report_root = Some(Replaced {
            dir: dir.display().to_string(),
            predecessor: old_root.manifest.bundle_id.clone(),
            manifest,
        });
        (dir, signer)
    } else {
        (req.root_bundle.clone(), req.root_signer.clone())
    };

    let mut issuers = Vec::new();
    for (path, old) in olds {
        let dir = out_dir.join("issuers").join(&old.manifest.slug);
        let manifest = engine::rotate_issuer_set(
            &IssuerRotateRequest {
                issuer_bundle: path.clone(),
                root_bundle: root_dir.clone(),
                root_signer: root_signer.clone(),
                not_before: req.not_before,
                not_after: req.issuer_not_after,
                allow_legacy_root_strength: req.allow_legacy_root_strength,
                private_store: req
                    .private_store
                    .as_ref()
                    .map(|s| s.join("issuers").join(&old.manifest.slug)),
                overlap_root_bundle: req.rotate_roots.then(|| req.root_bundle.clone()),
                overlap_root_signer: req.rotate_roots.then(|| req.root_signer.clone()),
            },
            &dir,
        )?;
        issuers.push(Replaced {
            dir: dir.display().to_string(),
            predecessor: old.manifest.bundle_id.clone(),
            manifest,
        });
    }

    // Verify every new chain: primaries and cross variants to the
    // roots now in force, overlap certificates to the old roots.
    let new_root = load_bundle(&root_dir)?;
    let new_pool = engine::collect_pool(&[&new_root])?;
    let old_pool = engine::collect_pool(&[old_root])?;
    let mut checks = Vec::new();
    for replaced in &issuers {
        let bundle = load_bundle(Path::new(&replaced.dir))?;
        for record in &bundle.manifest.certificates {
            let (pool, anchors) = match record.role.as_str() {
                "issuer" => (&new_pool, "the new roots"),
                "issuer-overlap" => (&old_pool, "the old roots"),
                _ => continue,
            };
            let cert = parse_cert(&first_der(&bundle, record)?)?;
            let chain = validate_chain(&cert, pool, TrustSelection::All, record.not_before);
            let check = Check {
                name: format!("chain:{}", record.cert_path),
                ok: chain.valid,
                detail: format!(
                    "{} in {} to {anchors}: {}",
                    record.subject_dn,
                    replaced.manifest.name,
                    if chain.valid {
                        format!("{} certificates", chain.path.len())
                    } else {
                        "no valid path".to_string()
                    }
                ),
            };
            if !check.ok {
                return Err(CertError::validation(format!(
                    "{verb} made a certificate that does not validate: {}",
                    check.detail
                )));
            }
            checks.push(check);
        }
    }

    let retire_old_after = olds
        .iter()
        .flat_map(|(_, b)| b.manifest.certificates.iter())
        .filter(|r| r.role == "issuer" || r.role == "issuer-overlap")
        .map(|r| r.not_after)
        .max()
        .unwrap_or(req.not_before);
    let issuer_dirs: Vec<String> = issuers.iter().map(|r| r.dir.clone()).collect();
    let mut next_steps = Vec::new();
    if let Some(root) = &report_root {
        next_steps.push(format!(
            "Distribute the new roots alongside the old ones: `fleet cert trust export` from {} \
             and {}, then stage, acknowledge and activate the bundle on every relying party.",
            req.root_bundle.display(),
            root.dir
        ));
    }
    next_steps.push(format!(
        "Issue new leaves only from {}; move each existing leaf with `fleet cert leaf rotate \
         --issuer <new set>` (a CSR-issued leaf needs a new CSR from its host).",
        issuer_dirs.join(", ")
    ));
    next_steps.push(format!(
        "Stop issuing from the predecessor issuer sets now; their leaves stay valid until they \
         expire, at the latest {retire_old_after}."
    ));
    if report_root.is_some() {
        next_steps.push(format!(
            "After {retire_old_after}, retire the old roots from trust bundles (`fleet cert trust \
             retire`); until then the overlap certificates keep new leaves valid for relying \
             parties that have not moved."
        ));
    }
    Ok(ReplaceReport {
        root: report_root,
        issuers,
        checks,
        retire_old_after,
        next_steps,
    })
}

fn first_der(bundle: &Bundle, record: &CertRecord) -> Result<Vec<u8>> {
    let pem = read_artifact(bundle, &record.cert_path)?;
    let text = std::str::from_utf8(&pem)
        .map_err(|_| CertError::manifest(format!("{} is not UTF-8 PEM", record.cert_path)))?;
    pem_decode_all(text)?
        .into_iter()
        .find(|(label, _)| label == "CERTIFICATE")
        .map(|(_, der)| der)
        .ok_or_else(|| CertError::manifest(format!("no CERTIFICATE block in {}", record.cert_path)))
}
