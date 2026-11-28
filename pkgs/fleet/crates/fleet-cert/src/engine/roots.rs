//! engine::roots — root-set creation, renewal and rotation
//! (#16255).
//!
//! Split from engine for #16255 (production-line guard): a
//! sequential, behavior-preserving module split. The parent
//! module re-exports every public item, so all crate paths are
//! unchanged.

use super::*;

/// `cert root create`: generate a root set — one self-signed root
/// per algorithm, plus the root-level cross-certificates the graph
/// policy and mode call for — and the §3.5 trust bundles.
pub fn create_root_set(req: &RootCreateRequest, out_dir: &Path) -> Result<BundleManifest> {
    create_root_set_at(req, out_dir, None)
}

/// [`create_root_set`] with the predecessor a rotation records.
fn create_root_set_at(
    req: &RootCreateRequest,
    out_dir: &Path,
    predecessor: Option<String>,
) -> Result<BundleManifest> {
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

    let root_ders: Vec<(AlgorithmId, &[u8])> =
        roots.iter().map(|r| (r.algo, r.der.as_slice())).collect();
    push_trust_bundles(&mut artifacts, &root_ders, &records);

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
    manifest.predecessor = predecessor;
    manifest.certificates = records;
    let manifest = write_bundle(out_dir, manifest, &artifacts)?;
    Ok(manifest)
}

/// `cert root renew` (§5.2): reissue every certificate of a root set
/// on the keys it already has.
///
/// Renewal is key reuse, so it runs only with the explicit opt-in.
/// Each self-signed root is signed again by its own key with the new
/// validity window, keeping its subject name byte for byte, its key
/// and its path length; each root-level cross-certificate is signed
/// again by the same signer root over the renewed subject. Issuer
/// sets certified by the predecessor keep validating against the
/// renewed roots, since keys and names are unchanged. The bundle
/// records `revision` = predecessor + 1 and the predecessor's id and
/// keeps its generation.
pub fn renew_root_set(
    req: &crate::model::RootRenewRequest,
    out_dir: &Path,
) -> Result<BundleManifest> {
    if !req.allow_key_reuse {
        return Err(CertError::policy(
            "root renew reissues the root certificates on the same keys; policy allows key \
             reuse only when asked for explicitly (--allow-key-reuse). Key rotation is a \
             separate verb (spec §5.2), and the default for a substantive replacement",
        ));
    }
    if req.not_after <= req.not_before {
        return Err(CertError::usage("not-after must be later than not-before"));
    }
    let old = load_bundle(&req.root_bundle)?;
    if old.manifest.bundle_type != BundleType::RootSet {
        return Err(CertError::usage(format!(
            "{} is a {} bundle, not a root set",
            req.root_bundle.display(),
            old.manifest.bundle_type.as_str()
        )));
    }
    let org = old.manifest.organization.clone();
    let country = old.manifest.country.clone();
    let root_records: Vec<&CertRecord> = old
        .manifest
        .certificates
        .iter()
        .filter(|r| r.role == "root")
        .collect();
    if root_records.is_empty() {
        return Err(CertError::manifest(
            "the root set holds no root certificates",
        ));
    }
    let partition = root_records[0].partition.clone();
    let gp = GenerationPartition::new(old.manifest.generation, &partition)?;
    let bundle_private = old.manifest.key_location == "bundle-private";

    struct Renewed {
        handle: CaHandle,
        public_key: Vec<u8>,
        der: Vec<u8>,
    }
    let mut renewed: BTreeMap<AlgorithmId, Renewed> = BTreeMap::new();
    let mut artifacts: Vec<Artifact> = Vec::new();
    let mut records = Vec::new();
    for record in &root_records {
        let what = format!("{} ({})", record.subject_dn, record.cert_path);
        let cn = extract_cn(&record.subject_dn)
            .ok_or_else(|| CertError::manifest(format!("{what} has no CN in its subject DN")))?;
        let old_der = first_certificate_der(&old, &record.cert_path)?;
        let old_info = parse_cert(&old_der)?;
        let facts = issued_facts(&old_der, &what)?;
        let mut handle = load_ca(
            &old,
            record,
            &req.root_signer,
            &org,
            &country,
            None,
            &cn,
            old_info.path_len.unwrap_or(1),
        )?;
        let params = ca_params_for(
            &org,
            &country,
            None,
            &cn,
            old_info.path_len.unwrap_or(1),
            req.not_before,
            req.not_after,
        );
        // Self-signed: the issuer is the subject, with its own key.
        handle.params = params.clone();
        let subject_key = ExistingPublicKey {
            raw: facts.public_key.clone(),
            algorithm: signature_algorithm_for(record.algorithm),
        };
        let der = handle.sign(&params, &subject_key)?.der().to_vec();
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
        let key_path = match (&record.key_path, bundle_private) {
            (Some(rel), true) => {
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
                Some(rel.clone())
            }
            _ => None,
        };
        artifacts.push(Artifact {
            rel_path: record.cert_path.clone(),
            kind: "certificate-pem".into(),
            visibility: "public".into(),
            bytes: pem_encode("CERTIFICATE", &der).into_bytes(),
        });
        records.push(record_from_der(
            &der,
            "root",
            &record.partition,
            true,
            record.cert_path.clone(),
            key_path,
        )?);
        renewed.insert(
            record.algorithm,
            Renewed {
                handle,
                public_key: facts.public_key,
                der,
            },
        );
    }
    for record in old
        .manifest
        .certificates
        .iter()
        .filter(|r| r.role == "root-cross")
    {
        let what = format!("{} ({})", record.subject_dn, record.cert_path);
        let signer_algo = record
            .signer_algorithm
            .ok_or_else(|| CertError::manifest(format!("{what} records no signer algorithm")))?;
        let (Some(subject), Some(signer)) =
            (renewed.get(&record.algorithm), renewed.get(&signer_algo))
        else {
            return Err(CertError::manifest(format!(
                "{what} names a root the set does not hold"
            )));
        };
        let mut params = subject.handle.params.clone();
        params.serial_number = Some(random_serial());
        let subject_key = ExistingPublicKey {
            raw: subject.public_key.clone(),
            algorithm: signature_algorithm_for(record.algorithm),
        };
        let der = signer.handle.sign(&params, &subject_key)?.der().to_vec();
        artifacts.push(Artifact {
            rel_path: record.cert_path.clone(),
            kind: "certificate-pem".into(),
            visibility: "public".into(),
            bytes: pem_encode("CERTIFICATE", &der).into_bytes(),
        });
        let mut rec = record_from_der(
            &der,
            "root-cross",
            &record.partition,
            false,
            record.cert_path.clone(),
            None,
        )?;
        rec.signer_algorithm = Some(signer_algo);
        records.push(rec);
    }
    let root_ders: Vec<(AlgorithmId, &[u8])> = root_records
        .iter()
        .filter_map(|r| {
            renewed
                .get(&r.algorithm)
                .map(|n| (r.algorithm, n.der.as_slice()))
        })
        .collect();
    push_trust_bundles(&mut artifacts, &root_ders, &records);

    let key_location = if bundle_private {
        "bundle-private"
    } else {
        "external-store"
    };
    let mut manifest = base_manifest(
        BundleType::RootSet,
        &old.manifest.name,
        &org,
        &country,
        None,
        old.manifest.generation,
        None,
        key_location,
        old.manifest.cross_signing.clone(),
        Vec::new(),
    )?;
    manifest.revision = old.manifest.revision + 1;
    manifest.predecessor = Some(old.manifest.bundle_id.clone());
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

/// `cert root rotate` (§5.2): a new root set with new keys, in the
/// next key generation. The algorithms, partition, organization,
/// country and cross-signing mode are the predecessor's; subject
/// names move from `.. A G1` to `.. A G2`. The predecessor is left
/// as it is, so its issuer sets and leaves keep validating for
/// relying parties that trust it until they move to the new roots.
pub fn rotate_root_set(
    req: &crate::model::RootRotateRequest,
    out_dir: &Path,
) -> Result<BundleManifest> {
    let old = load_bundle(&req.root_bundle)?;
    if old.manifest.bundle_type != BundleType::RootSet {
        return Err(CertError::usage(format!(
            "{} is a {} bundle, not a root set",
            req.root_bundle.display(),
            old.manifest.bundle_type.as_str()
        )));
    }
    let roots: Vec<&CertRecord> = old
        .manifest
        .certificates
        .iter()
        .filter(|r| r.role == "root")
        .collect();
    let first = roots
        .first()
        .ok_or_else(|| CertError::manifest("the root set holds no root certificates"))?;
    let cross_signing = match &old.manifest.cross_signing {
        Some(mode) => CrossSigningMode::parse(mode)?,
        None => CrossSigningMode::None,
    };
    let create = RootCreateRequest {
        organization: old.manifest.organization.clone(),
        country: old.manifest.country.clone(),
        algorithms: roots.iter().map(|r| r.algorithm).collect(),
        generation: old.manifest.generation + 1,
        partition: first.partition.to_ascii_uppercase(),
        cross_signing,
        not_before: req.not_before,
        not_after: req.not_after,
        allow_legacy_root_strength: req.allow_legacy_root_strength,
        private_store: req.private_store.clone(),
    };
    create_root_set_at(&create, out_dir, Some(old.manifest.bundle_id.clone()))
}
