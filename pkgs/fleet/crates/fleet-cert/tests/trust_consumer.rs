//! Root-set trust verbs salvaged from PR #14475 into the BT-2 trust
//! surface (#15612): selection, pinning, consumer-targeted export,
//! consumer-store drift, and the `ssh` refusal.
//!
//! Oracles: spec §3.5 (bundle membership: all / classical / PQ),
//! spec §44 (consumer table, OpenSSH trust is not X.509), SHA-256
//! fingerprints recomputed here over each exported certificate's
//! DER, self-signature re-verified through the chain code, and
//! OpenSSL as an independent parser when it is installed.

use fleet_cert::algorithm::AlgorithmId;
use fleet_cert::bundle::{load_bundle, pem_decode_all, sha256_hex};
use fleet_cert::chain::{TrustSelection, parse_cert, verify_signature};
use fleet_cert::engine;
use fleet_cert::graph::CrossSigningMode;
use fleet_cert::model::RootCreateRequest;
use fleet_cert::timeparse::Rfc3339;
use fleet_cert::trust_consumer::{
    TRUST_EXPORT_SCHEMA, TRUST_PINS_SCHEMA, TrustPin, assemble, bundle_file_name, check_pins,
    check_store, consumer_names, consumer_target, create_pins, parse_pem_certs, read_store_certs,
    refuse_ssh, write_export,
};
use std::path::{Path, PathBuf};

fn ts(s: &str) -> Rfc3339 {
    Rfc3339::parse(s).unwrap()
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "fleet-cert-trust-consumer-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn root_set(tmp: &TempDir, tag: &str, algorithms: Vec<AlgorithmId>, generation: u32) -> PathBuf {
    let dir = tmp.path().join(tag);
    let request = RootCreateRequest {
        organization: "Test Org LLC".into(),
        country: "US".into(),
        algorithms,
        generation,
        partition: "A".into(),
        cross_signing: CrossSigningMode::Both,
        not_before: ts("2026-01-01T00:00:00Z"),
        not_after: ts("2046-01-01T00:00:00Z"),
        allow_legacy_root_strength: false,
        private_store: None,
    };
    engine::create_root_set(&request, &dir).unwrap();
    dir
}

fn cli(args: &[&str]) -> i32 {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    fleet_cert::cli::run(&args)
}

#[test]
fn selections_admit_only_signature_proven_self_signed_roots() {
    let tmp = TempDir::new("select");
    let dir = root_set(
        &tmp,
        "roots",
        vec![
            AlgorithmId::Ed25519,
            AlgorithmId::P256,
            AlgorithmId::MlDsa44,
        ],
        1,
    );
    let bundle = load_bundle(&dir).unwrap();
    // The root set carries cross-certificates too (CrossSigningMode::Both);
    // none of them may surface as an anchor.
    assert!(
        bundle
            .manifest
            .certificates
            .iter()
            .any(|r| r.role == "root-cross")
    );

    let all = assemble(&bundle, TrustSelection::All).unwrap();
    let classical = assemble(&bundle, TrustSelection::Classical).unwrap();
    let pq = assemble(&bundle, TrustSelection::Pq).unwrap();
    assert_eq!(all.certificates.len(), 3);
    assert_eq!(classical.certificates.len(), 2);
    assert_eq!(pq.certificates.len(), 1);
    assert_eq!(
        pq.certificates[0].subject_algorithm,
        Some(AlgorithmId::MlDsa44)
    );
    assert!(
        classical
            .certificates
            .iter()
            .all(|c| c.subject_algorithm.is_some_and(|a| a.is_classical()))
    );

    // Recompute every anchor from its PEM: fingerprint over DER,
    // name match, and a self-verifying signature.
    let reparsed = parse_pem_certs(&all.pem).unwrap();
    assert_eq!(reparsed.len(), 3);
    for cert in &reparsed {
        assert_eq!(cert.fingerprint_sha256, sha256_hex(&cert.der));
        assert_eq!(cert.subject_name_der, cert.issuer_name_der);
        assert!(verify_signature(cert, cert));
    }
    let root_fps: Vec<&str> = bundle
        .manifest
        .certificates
        .iter()
        .filter(|r| r.role == "root")
        .map(|r| r.fingerprint_sha256.as_str())
        .collect();
    for cert in &all.certificates {
        assert!(root_fps.contains(&cert.fingerprint_sha256.as_str()));
    }

    // Deterministic: the same root set assembles the same bytes.
    let again = assemble(&bundle, TrustSelection::All).unwrap();
    assert_eq!(again.pem, all.pem);
    assert_eq!(again.sha256, sha256_hex(all.pem.as_bytes()));
    assert_eq!(bundle_file_name(TrustSelection::Pq), "roots-pq.pem");
}

#[test]
fn pinsets_catch_rotation_new_anchors_and_retargeting() {
    let tmp = TempDir::new("pins");
    let dir = root_set(
        &tmp,
        "roots",
        vec![AlgorithmId::Ed25519, AlgorithmId::P256],
        1,
    );
    let bundle = load_bundle(&dir).unwrap();
    let pins = create_pins(&bundle, TrustSelection::All).unwrap();
    assert_eq!(pins.schema, TRUST_PINS_SCHEMA);
    assert_eq!(pins.pins.len(), 2);
    let clean = check_pins(&bundle, TrustSelection::All, &pins);
    assert!(clean.iter().all(|c| c.ok), "{clean:?}");

    // An anchor the pinset does not hold is a new, unpinned anchor.
    let mut short = pins.clone();
    short.pins.pop();
    let checks = check_pins(&bundle, TrustSelection::All, &short);
    assert!(checks.iter().any(|c| c.name.starts_with("pin-unpinned")));

    // A pin the bundle no longer holds is a rotated-out anchor.
    let mut stale = pins.clone();
    stale.pins.push(TrustPin {
        subject_dn: "CN=Retired Root".into(),
        algorithm: "ed25519".into(),
        fingerprint_sha256: "00".repeat(32),
    });
    let checks = check_pins(&bundle, TrustSelection::All, &stale);
    assert!(checks.iter().any(|c| c.name.starts_with("pin-missing")));

    // Pins for another selection, or taken against another root set,
    // fail loudly rather than matching by accident.
    let classical_checks = check_pins(&bundle, TrustSelection::Classical, &pins);
    assert!(
        classical_checks
            .iter()
            .any(|c| c.name == "pins-trust" && !c.ok)
    );
    let other_dir = root_set(
        &tmp,
        "other",
        vec![AlgorithmId::Ed25519, AlgorithmId::P256],
        2,
    );
    let other = load_bundle(&other_dir).unwrap();
    let retarget = check_pins(&other, TrustSelection::All, &pins);
    assert!(retarget.iter().any(|c| c.name == "pins-target" && !c.ok));
    assert!(retarget.iter().any(|c| c.name.starts_with("pin-missing")));
}

#[test]
fn consumer_export_writes_hashable_pem_and_guidance() {
    let tmp = TempDir::new("export");
    let dir = root_set(
        &tmp,
        "roots",
        vec![AlgorithmId::Ed25519, AlgorithmId::MlDsa44],
        1,
    );
    let bundle = load_bundle(&dir).unwrap();

    let debian = consumer_target("ubuntu").expect("alias resolves");
    assert_eq!(debian.name, "debian");
    assert_eq!(debian.kind, TrustSelection::All);
    assert_eq!(
        consumer_target("Java").unwrap().kind,
        TrustSelection::Classical
    );
    assert_eq!(consumer_target("pq-only").unwrap().kind, TrustSelection::Pq);
    assert!(consumer_target("ssh").is_none());
    assert!(!consumer_names().contains(&"ssh"));

    let all = assemble(&bundle, debian.kind).unwrap();
    let out = tmp.path().join("debian-export");
    let manifest = write_export(&out, &bundle, debian.name, debian, &all).unwrap();
    assert_eq!(manifest.schema, TRUST_EXPORT_SCHEMA);
    assert_eq!(manifest.consumer, "debian");
    assert_eq!(manifest.bundle_id, bundle.manifest.bundle_id);
    assert_eq!(manifest.certificates.len(), 2);
    assert!(!manifest.install.is_empty() && !manifest.remove.is_empty());

    // The bundle file hashes to what the manifest records.
    let exported = std::fs::read_to_string(out.join(&manifest.bundle_file)).unwrap();
    assert_eq!(sha256_hex(exported.as_bytes()), manifest.bundle_sha256);
    assert_eq!(pem_decode_all(&exported).unwrap().len(), 2);

    // Each per-root file round-trips to the fingerprint it is listed under.
    for cert in &manifest.certificates {
        let text = std::fs::read_to_string(out.join(&cert.file)).unwrap();
        let blocks = pem_decode_all(&text).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].0, "CERTIFICATE");
        assert_eq!(sha256_hex(&blocks[0].1), cert.fingerprint_sha256);
        let info = parse_cert(&blocks[0].1).unwrap();
        assert_eq!(info.subject_dn, cert.subject_dn);
    }

    // Independent parser: OpenSSL reads every per-root file, when present.
    if std::process::Command::new("openssl")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        for cert in &manifest.certificates {
            let parsed = std::process::Command::new("openssl")
                .args(["x509", "-noout", "-subject", "-in"])
                .arg(out.join(&cert.file))
                .output()
                .unwrap();
            let unsupported_pq = cert.algorithm.contains("ml-dsa")
                && String::from_utf8_lossy(&parsed.stderr).contains("unsupported");
            assert!(
                parsed.status.success() || unsupported_pq,
                "OpenSSL rejected {}: {}",
                cert.file,
                String::from_utf8_lossy(&parsed.stderr)
            );
        }
    }

    // Re-exporting over an existing export is refused.
    let again = write_export(&out, &bundle, debian.name, debian, &all).unwrap_err();
    assert!(again.to_string().contains("already exists"), "{again}");

    let ssh = refuse_ssh().to_string();
    assert!(ssh.contains("not X.509"), "{ssh}");
}

#[test]
fn consumer_store_drift_in_both_directions() {
    let tmp = TempDir::new("store");
    let dir = root_set(
        &tmp,
        "roots",
        vec![
            AlgorithmId::Ed25519,
            AlgorithmId::P256,
            AlgorithmId::MlDsa44,
        ],
        1,
    );
    let bundle = load_bundle(&dir).unwrap();
    let all = assemble(&bundle, TrustSelection::All).unwrap();
    let classical = assemble(&bundle, TrustSelection::Classical).unwrap();

    // A single-file store holding exactly the bundle matches.
    let store_file = tmp.path().join("store.pem");
    std::fs::write(&store_file, &all.pem).unwrap();
    let store = read_store_certs(&store_file).unwrap();
    assert_eq!(store.len(), 3);
    assert!(check_store(&all, &store).iter().all(|c| c.ok));

    // Missing anchor: a stale consumer.
    assert!(
        check_store(&all, &store[..2])
            .iter()
            .any(|c| c.name.starts_with("store-missing"))
    );
    // Extra anchor: the classical bundle does not include the ML-DSA root.
    assert!(
        check_store(&classical, &store)
            .iter()
            .any(|c| c.name.starts_with("store-extra"))
    );

    // A directory store reads .pem / .crt / .cer files and ignores the rest.
    let store_dir = tmp.path().join("store.d");
    std::fs::create_dir_all(&store_dir).unwrap();
    for (i, cert) in classical.certificates.iter().enumerate() {
        let ext = if i == 0 { "crt" } else { "pem" };
        std::fs::write(
            store_dir.join(format!("anchor-{i}.{ext}")),
            fleet_cert::bundle::pem_encode("CERTIFICATE", &cert.der),
        )
        .unwrap();
    }
    std::fs::write(store_dir.join("README.txt"), "not a certificate").unwrap();
    let dir_store = read_store_certs(&store_dir).unwrap();
    assert_eq!(dir_store.len(), 2);
    assert!(check_store(&classical, &dir_store).iter().all(|c| c.ok));
}

#[test]
fn cli_root_set_trust_verbs() {
    let tmp = TempDir::new("cli");
    let dir = root_set(
        &tmp,
        "roots",
        vec![AlgorithmId::Ed25519, AlgorithmId::MlDsa44],
        1,
    );
    let root = dir.to_string_lossy().into_owned();
    let pins = tmp.path().join("pins.json");
    let pins_s = pins.to_string_lossy().into_owned();

    assert_eq!(cli(&["trust", "inspect", "--root", &root, "--json"]), 0);
    assert_eq!(
        cli(&["trust", "pin", "--root", &root, "--out", &pins_s, "--json"]),
        0
    );
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&pins).unwrap()).unwrap();
    assert_eq!(written["schema"], TRUST_PINS_SCHEMA);
    // Re-pinning over an existing pinset is refused.
    assert_eq!(cli(&["trust", "pin", "--root", &root, "--out", &pins_s]), 1);
    assert_eq!(
        cli(&["trust", "verify", "--root", &root, "--pins", &pins_s]),
        0
    );
    // A pinset for a different selection is drift (exit 1).
    assert_eq!(
        cli(&[
            "trust",
            "verify",
            "--root",
            &root,
            "--trust",
            "classical",
            "--pins",
            &pins_s
        ]),
        1
    );
    // verify needs something to compare against.
    assert_eq!(cli(&["trust", "verify", "--root", &root]), 2);

    let out = tmp.path().join("java");
    let out_s = out.to_string_lossy().into_owned();
    assert_eq!(
        cli(&[
            "trust", "export", "--root", &root, "--target", "java", "--out", &out_s
        ]),
        0
    );
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("export.json")).unwrap()).unwrap();
    assert_eq!(manifest["consumer"], "java");
    assert_eq!(manifest["trust"], "classical");
    assert_eq!(manifest["certificates"].as_array().unwrap().len(), 1);

    let other = tmp.path().join("x").to_string_lossy().into_owned();
    // ssh is refused as a policy failure; a contradicting --trust is usage.
    assert_eq!(
        cli(&[
            "trust", "export", "--root", &root, "--target", "ssh", "--out", &other
        ]),
        1
    );
    assert_eq!(
        cli(&[
            "trust", "export", "--root", &root, "--target", "java", "--trust", "all", "--out",
            &other
        ]),
        2
    );
    // --version belongs to the versioned trust bundle, not a consumer export.
    assert_eq!(
        cli(&[
            "trust",
            "export",
            "--root",
            &root,
            "--target",
            "debian",
            "--version",
            "1",
            "--out",
            &other
        ]),
        2
    );
    assert!(!Path::new(&other).exists());

    assert_eq!(cli(&["trust", "install-plan", "--target", "k8s"]), 0);
    assert_eq!(cli(&["trust", "install-plan", "--target", "ssh"]), 1);
    assert_eq!(cli(&["explain", "trust"]), 0);
}
