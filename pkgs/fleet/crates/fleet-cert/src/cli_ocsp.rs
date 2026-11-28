//! `fleet cert ocsp` command group (sign | inspect | test).
//!
//! Split out of `cli.rs` (#16255 production-line-guard slice): the OCSP
//! verb cluster plus its private helpers. Entry point is [`ocsp_cmd`],
//! called from the `ocsp` dispatch arm in `cli.rs`.

use crate::algorithm::AlgorithmId;
use crate::bundle::{load_bundle, pem_decode_all, read_artifact};
use crate::chain::{CertInfo, parse_cert};
use crate::cli::{Parsed, emit, parse_args};
use crate::error::{CertError, Result};
use crate::model::{BundleType, CertRecord, SignerSource};
use crate::timeparse::{CalendarDuration, Rfc3339};
use serde_json::json;
use std::path::PathBuf;

// ---------------------------------------------------------------- ocsp

pub(crate) fn ocsp_cmd(args: &[String], json_out: bool) -> Result<i32> {
    let verb = args.first().map(|s| s.as_str()).unwrap_or("");
    let rest = &args[1.min(args.len())..];
    match verb {
        "sign" => ocsp_sign_cmd(rest, json_out),
        "inspect" => ocsp_inspect_cmd(rest, json_out),
        "test" => ocsp_test_cmd(rest, json_out),
        other => Err(CertError::usage(format!(
            "unknown ocsp verb '{other}' (sign | inspect | test)"
        ))),
    }
}

/// Parse the DER of the certificate a bundle record points at.
pub(crate) fn cert_from_record(
    bundle: &crate::bundle::Bundle,
    record: &CertRecord,
) -> Result<CertInfo> {
    let pem = read_artifact(bundle, &record.cert_path)?;
    let blocks = pem_decode_all(
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
    parse_cert(&der)
}

/// Select the primary issuer record of an issuer bundle: the one
/// whose SKI matches the leaf's AKI when a leaf is known, narrowed
/// by --issuer-algorithm / --issuer-partition when given. Ambiguity
/// is a usage error, never a guess.
pub(crate) fn resolve_issuer_record(
    bundle: &crate::bundle::Bundle,
    leaf: Option<&CertInfo>,
    algorithm: Option<AlgorithmId>,
    partition: Option<&str>,
) -> Result<CertRecord> {
    let mut candidates: Vec<&CertRecord> = bundle
        .manifest
        .certificates
        .iter()
        .filter(|r| r.role == "issuer" && r.primary)
        .collect();
    if let Some(algo) = algorithm {
        candidates.retain(|r| r.algorithm == algo);
    }
    if let Some(part) = partition {
        let part = part.to_ascii_lowercase();
        candidates.retain(|r| r.partition == part);
    }
    if let Some(leaf) = leaf {
        if let Some(aki) = &leaf.aki {
            let aki_hex = crate::chain::hex_bytes(aki);
            let matched: Vec<&CertRecord> = candidates
                .iter()
                .filter(|r| r.ski_hex == aki_hex)
                .copied()
                .collect();
            if !matched.is_empty() {
                candidates = matched;
            }
        }
    }
    match candidates.len() {
        1 => Ok(candidates[0].clone()),
        0 => Err(CertError::usage(
            "the issuer bundle has no primary issuer record matching the selection",
        )),
        n => Err(CertError::usage(format!(
            "the issuer selection matches {n} primary issuer records; narrow it with \
             --leaf (AKI match), --issuer-algorithm or --issuer-partition"
        ))),
    }
}

/// The key file name a direct issuer signing uses inside the
/// signer source (the bundle convention load_ca applies).
pub(crate) fn issuer_key_file(record: &CertRecord) -> String {
    record
        .key_path
        .as_deref()
        .and_then(|p| {
            std::path::Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| format!("{}-{}.pk8", record.algorithm.name(), record.partition))
}

pub(crate) fn parse_algorithm_flag(p: &Parsed, flag: &str) -> Result<Option<AlgorithmId>> {
    match p.get(flag) {
        Some(s) => {
            let (a, w) = AlgorithmId::parse(s)?;
            if let Some(w) = w {
                eprintln!("warning: {w}");
            }
            Ok(Some(a))
        }
        None => Ok(None),
    }
}

pub(crate) fn ocsp_sign_cmd(rest: &[String], json_out: bool) -> Result<i32> {
    let p = parse_args(
        rest,
        &[
            "leaf",
            "issuer",
            "serial",
            "responder",
            "responder-key-dir",
            "issuer-key-dir",
            "issuer-algorithm",
            "issuer-partition",
            "status",
            "revocation-time",
            "reason",
            "hash",
            "this-update",
            "next-update",
            "out",
        ],
        &["pem", "json"],
    )?;
    let issuer_dir = p.require("issuer", "ocsp sign")?;
    let issuer_bundle = load_bundle(&PathBuf::from(&issuer_dir))?;
    if issuer_bundle.manifest.bundle_type != BundleType::IssuerSet {
        return Err(CertError::usage(format!(
            "{issuer_dir} is a {} bundle, not an issuer set",
            issuer_bundle.manifest.bundle_type.as_str()
        )));
    }
    // The leaf the response is about: a leaf bundle (serial + AKI
    // issuer selection) or an explicit --serial.
    let mut leaf_info: Option<CertInfo> = None;
    let leaf_serial = if let Some(leaf_dir) = p.get("leaf") {
        let leaf_bundle = load_bundle(&PathBuf::from(leaf_dir))?;
        let record = leaf_bundle
            .manifest
            .certificates
            .iter()
            .find(|r| r.role == "leaf")
            .ok_or_else(|| CertError::manifest(format!("{leaf_dir} has no leaf record")))?;
        let info = cert_from_record(&leaf_bundle, record)?;
        let serial = crate::ocsp::unhex(&info.serial_hex)?;
        leaf_info = Some(info);
        serial
    } else if let Some(serial) = p.get("serial") {
        crate::ocsp::unhex(serial)?
    } else {
        return Err(CertError::usage(
            "ocsp sign needs the leaf: --leaf <bundle> or --serial <hex>",
        ));
    };
    let issuer_record = resolve_issuer_record(
        &issuer_bundle,
        leaf_info.as_ref(),
        parse_algorithm_flag(&p, "issuer-algorithm")?,
        p.get("issuer-partition"),
    )?;
    let issuer = cert_from_record(&issuer_bundle, &issuer_record)?;

    // The signer (§24.3: always an explicit source).
    let signer = match (
        p.get("responder"),
        p.get("responder-key-dir"),
        p.get("issuer-key-dir"),
    ) {
        (Some(responder_dir), Some(key_dir), _) => {
            let responder_bundle = load_bundle(&PathBuf::from(responder_dir))?;
            let record = responder_bundle
                .manifest
                .certificates
                .iter()
                .find(|r| r.role == "leaf")
                .ok_or_else(|| {
                    CertError::manifest(format!("{responder_dir} has no leaf record"))
                })?;
            let cert = cert_from_record(&responder_bundle, record)?;
            if !crate::ocsp::cert_has_ocsp_signing_eku(&cert.der)? {
                return Err(CertError::policy(format!(
                    "the responder certificate at {responder_dir} does not carry the \
                     OCSP-signing EKU; issue one with: fleet cert leaf issue \
                     --profile ocsp-responder"
                )));
            }
            crate::ocsp::OcspSigner::Delegated {
                cert,
                key_file: "leaf.pk8".to_string(),
                source: SignerSource {
                    key_dir: PathBuf::from(key_dir),
                },
            }
        }
        (Some(_), None, _) => {
            return Err(CertError::usage(
                "ocsp sign --responder requires --responder-key-dir <dir>",
            ));
        }
        (None, _, Some(key_dir)) => crate::ocsp::OcspSigner::Direct {
            issuer_record_algorithm: issuer_record.algorithm,
            key_file: issuer_key_file(&issuer_record),
            source: SignerSource {
                key_dir: PathBuf::from(key_dir),
            },
        },
        (None, _, None) => {
            return Err(CertError::policy(
                "ocsp sign requires an explicit signer source (spec §24.3): \
                 --responder <bundle> --responder-key-dir <dir> for a delegated \
                 responder, or --issuer-key-dir <dir> for direct issuer signing",
            ));
        }
    };

    let this_update = p
        .get("this-update")
        .map(Rfc3339::parse)
        .transpose()?
        .unwrap_or_else(Rfc3339::now);
    let next_update = match p.get("next-update") {
        Some(s) => Some(Rfc3339::parse(s)?),
        None => Some(CalendarDuration::days(7).add_to(this_update)?),
    };
    let status = match p.get("status").unwrap_or("good") {
        "good" => crate::ocsp::OcspStatus::Good,
        "unknown" => crate::ocsp::OcspStatus::Unknown,
        "revoked" => crate::ocsp::OcspStatus::Revoked {
            revocation_time: p
                .get("revocation-time")
                .map(Rfc3339::parse)
                .transpose()?
                .unwrap_or(this_update),
            reason: p.get("reason").map(crate::ocsp::parse_reason).transpose()?,
        },
        other => {
            return Err(CertError::usage(format!(
                "unknown --status '{other}' (good | revoked | unknown)"
            )));
        }
    };
    let hash = p
        .get("hash")
        .map(crate::ocsp::OcspHash::parse)
        .transpose()?
        .unwrap_or(crate::ocsp::OcspHash::Sha1);
    let req = crate::ocsp::OcspSignRequest {
        issuer: issuer.clone(),
        leaf_serial: leaf_serial.clone(),
        status,
        hash,
        produced_at: Rfc3339::now(),
        this_update,
        next_update,
        signer,
    };
    let der = crate::ocsp::sign_response(&req)?;
    let out = p.require("out", "ocsp sign")?;
    let (bytes, encoding) = if p.has("pem") {
        (
            crate::bundle::pem_encode("OCSP RESPONSE", &der).into_bytes(),
            "pem",
        )
    } else {
        (der.clone(), "der")
    };
    std::fs::write(&out, &bytes)?;
    let signer_desc = match &req.signer {
        crate::ocsp::OcspSigner::Direct { .. } => {
            format!("direct (issuer {})", issuer.subject_dn)
        }
        crate::ocsp::OcspSigner::Delegated { cert, .. } => {
            format!("delegated responder {}", cert.subject_dn)
        }
    };
    let text = format!(
        "OCSP response written: {out} ({encoding}, {} bytes)\n  status: {}\n  leaf serial: {}\n  issuer: {}\n  signer: {signer_desc}\n  cert-id hash: {}\n  this-update: {}\n  next-update: {}\n",
        bytes.len(),
        status.label(),
        crate::ocsp::hex(&leaf_serial),
        issuer.subject_dn,
        hash.name(),
        this_update,
        next_update
            .map(|t| t.to_string_canonical())
            .unwrap_or_else(|| "none".to_string()),
    );
    let value = json!({
        "out": out,
        "encoding": encoding,
        "bytes": bytes.len(),
        "status": status.label(),
        "leaf_serial": crate::ocsp::hex(&leaf_serial),
        "issuer": issuer.subject_dn,
        "signer": signer_desc,
        "cert_id_hash": hash.name(),
        "this_update": this_update.to_string_canonical(),
        "next_update": next_update.map(|t| t.to_string_canonical()),
    });
    Ok(emit(json_out, text, value))
}

pub(crate) fn read_response_bytes(path: &str) -> Result<Vec<u8>> {
    let raw = std::fs::read(path)?;
    if raw.starts_with(b"-----") {
        let text = std::str::from_utf8(&raw)
            .map_err(|_| CertError::manifest(format!("{path} is not UTF-8 PEM")))?;
        let blocks = pem_decode_all(text)?;
        blocks
            .into_iter()
            .next()
            .map(|(_, der)| der)
            .ok_or_else(|| CertError::manifest(format!("no PEM block in {path}")))
    } else {
        Ok(raw)
    }
}

pub(crate) fn ocsp_inspect_cmd(rest: &[String], json_out: bool) -> Result<i32> {
    let p = parse_args(
        rest,
        &[
            "response",
            "issuer",
            "leaf",
            "serial",
            "issuer-algorithm",
            "issuer-partition",
            "at",
        ],
        &["json"],
    )?;
    let path = p
        .get("response")
        .map(|s| s.to_string())
        .or_else(|| p.positional(0).map(|s| s.to_string()))
        .ok_or_else(|| CertError::usage("ocsp inspect needs a response file"))?;
    let der = read_response_bytes(&path)?;
    let info = crate::ocsp::parse_response(&der)?;
    if !info.successful {
        let text = format!("OCSP response: {path}\n  responseStatus: not successful\n");
        let value = json!({"response": path, "successful": false});
        let code = emit(json_out, text, value);
        return Ok(if code == 0 { 1 } else { code });
    }
    let mut text = format!(
        "OCSP response: {path}\n  produced-at: {}\n  signature algorithm: {}\n",
        info.produced_at, info.signature_algorithm_oid
    );
    if let Some(rk) = &info.responder_key_hash {
        text.push_str(&format!("  responder-id (byKey sha1): {rk}\n"));
    }
    let mut embedded_json = Vec::new();
    for cert_der in &info.embedded_certs {
        if let Ok(ci) = parse_cert(cert_der) {
            text.push_str(&format!(
                "  embedded cert: {} (fingerprint {})\n",
                ci.subject_dn, ci.fingerprint_sha256
            ));
            embedded_json.push(json!({
                "subject": ci.subject_dn,
                "fingerprint_sha256": ci.fingerprint_sha256,
            }));
        }
    }
    let mut singles_json = Vec::new();
    for s in &info.singles {
        text.push_str(&format!(
            "  single: serial {} status {} this-update {} next-update {}\n    cert-id: hash {} issuer-name-hash {} issuer-key-hash {}\n",
            s.serial_hex,
            s.status.label(),
            s.this_update,
            s.next_update
                .map(|t| t.to_string_canonical())
                .unwrap_or_else(|| "none".to_string()),
            s.hash_algorithm_oid,
            s.issuer_name_hash,
            s.issuer_key_hash,
        ));
        singles_json.push(json!({
            "serial": s.serial_hex,
            "status": s.status.label(),
            "this_update": s.this_update.to_string_canonical(),
            "next_update": s.next_update.map(|t| t.to_string_canonical()),
            "hash_algorithm": s.hash_algorithm_oid,
            "issuer_name_hash": s.issuer_name_hash,
            "issuer_key_hash": s.issuer_key_hash,
        }));
    }
    let mut value = json!({
        "response": path,
        "successful": true,
        "produced_at": info.produced_at.to_string_canonical(),
        "signature_algorithm": info.signature_algorithm_oid,
        "responder_key_hash": info.responder_key_hash,
        "embedded_certs": embedded_json,
        "singles": singles_json,
    });

    // Optional full verification against an issuer bundle.
    if let Some(issuer_dir) = p.get("issuer") {
        let issuer_bundle = load_bundle(&PathBuf::from(issuer_dir))?;
        let mut leaf_info: Option<CertInfo> = None;
        let leaf_serial = if let Some(leaf_dir) = p.get("leaf") {
            let leaf_bundle = load_bundle(&PathBuf::from(leaf_dir))?;
            let record = leaf_bundle
                .manifest
                .certificates
                .iter()
                .find(|r| r.role == "leaf")
                .ok_or_else(|| CertError::manifest(format!("{leaf_dir} has no leaf record")))?;
            let li = cert_from_record(&leaf_bundle, record)?;
            let serial = crate::ocsp::unhex(&li.serial_hex)?;
            leaf_info = Some(li);
            serial
        } else if let Some(serial) = p.get("serial") {
            crate::ocsp::unhex(serial)?
        } else {
            return Err(CertError::usage(
                "ocsp inspect --issuer also needs the expected leaf: --leaf <bundle> \
                 or --serial <hex> (verification must not trust the response's own serial)",
            ));
        };
        let issuer_record = resolve_issuer_record(
            &issuer_bundle,
            leaf_info.as_ref(),
            parse_algorithm_flag(&p, "issuer-algorithm")?,
            p.get("issuer-partition"),
        )?;
        let issuer = cert_from_record(&issuer_bundle, &issuer_record)?;
        let at = p
            .get("at")
            .map(Rfc3339::parse)
            .transpose()?
            .unwrap_or_else(Rfc3339::now);
        let checks = crate::ocsp::verify_response(&der, &issuer, &leaf_serial, at)?;
        let all_ok = checks.iter().all(|c| c.ok);
        text.push_str(&format!(
            "  verification at {}:\n",
            at.to_string_canonical()
        ));
        for c in &checks {
            text.push_str(&format!(
                "    [{}] {} — {}\n",
                if c.ok { "ok" } else { "FAIL" },
                c.name,
                c.detail
            ));
        }
        text.push_str(&format!(
            "  result: {}\n",
            if all_ok { "VALID" } else { "INVALID" }
        ));
        value["verification"] = json!({
            "at": at.to_string_canonical(),
            "valid": all_ok,
            "checks": checks.iter().map(|c| json!({"name": c.name, "ok": c.ok, "detail": c.detail})).collect::<Vec<_>>(),
        });
        let code = emit(json_out, text, value);
        return Ok(if all_ok { code } else { 1 });
    }
    Ok(emit(json_out, text, value))
}

pub(crate) fn ocsp_test_cmd(rest: &[String], json_out: bool) -> Result<i32> {
    let p = parse_args(rest, &["dir"], &["json"])?;
    let (dir, keep) = match p.get("dir") {
        Some(d) => (PathBuf::from(d), true),
        None => {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            (
                std::env::temp_dir().join(format!(
                    "fleet-cert-ocsp-test-{}-{nanos}",
                    std::process::id()
                )),
                false,
            )
        }
    };
    std::fs::create_dir_all(&dir)?;
    let result = crate::ocsp::synthetic_e2e(&dir);
    if !keep {
        let _ = std::fs::remove_dir_all(&dir);
    }
    let checks = result?;
    let all_ok = checks.iter().all(|c| c.ok);
    let mut text = format!(
        "OCSP synthetic end-to-end ({}):\n",
        if keep {
            dir.display().to_string()
        } else {
            "temp dir, removed".to_string()
        }
    );
    for c in &checks {
        text.push_str(&format!(
            "  [{}] {} — {}\n",
            if c.ok { "ok" } else { "FAIL" },
            c.name,
            c.detail
        ));
    }
    text.push_str(&format!(
        "  result: {}\n",
        if all_ok { "PASS" } else { "FAIL" }
    ));
    let value = json!({
        "valid": all_ok,
        "kept_dir": if keep { Some(dir.display().to_string()) } else { None },
        "checks": checks.iter().map(|c| json!({"name": c.name, "ok": c.ok, "detail": c.detail})).collect::<Vec<_>>(),
    });
    let code = emit(json_out, text, value);
    Ok(if all_ok { code } else { 1 })
}
