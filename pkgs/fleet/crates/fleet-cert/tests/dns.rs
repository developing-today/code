//! `fleet cert dns plan | render | verify` (#13724), proved against
//! the contract's §7 synthetic fixture oracle
//! (`doc/2026-11-25T_cert_dns_caa_dane_plan.md`):
//!
//! - golden change-set: `plan` output equals the checked-in
//!   `tests/fixtures/dns/golden-change-set.json` byte for byte, with
//!   only the fixture's per-run values (bundle ids, fingerprints and
//!   the association digests of freshly generated keys) substituted;
//! - digest recomputation: every TLSA association is recomputed here
//!   by an independent path — a hand-written DER walk to the
//!   certificate's SubjectPublicKeyInfo — never by the planner's code;
//! - the CAA correction case: a Fleet-CA-only fixture yields zero CAA
//!   records and the explanatory prerequisite, and naming the Fleet
//!   CA as a CAA issuer is refused;
//! - no network / no write: the CLI writes only under `--out`, and its
//!   argument surface has no provider, endpoint or credential flag;
//! - no-change: `plan --current` fed the desired set reports no changes.

use fleet_cert::bundle::load_bundle;
use fleet_cert::dns::{self, ChangeSet, DnsPolicy};
use fleet_cert::engine;
use fleet_cert::graph::CrossSigningMode;
use fleet_cert::model::{
    IssuerCreateRequest, IssuerPurpose, LeafIssueRequest, RootCreateRequest, SignerSource,
};
use fleet_cert::timeparse::Rfc3339;
use sha2::{Digest, Sha256};
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
            "fleet-cert-dns-{tag}-{}-{nanos}",
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

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/dns");
const GENERATED_AT: &str = "2026-06-01T00:00:00Z";

/// Root set → Base issuer (one Ed25519 key, partition a) → `svc-leaf`
/// server leaf. Returns (root, issuer, leaf) bundle directories.
fn hierarchy(tmp: &TempDir) -> (PathBuf, PathBuf, PathBuf) {
    use fleet_cert::algorithm::AlgorithmId;
    let root = tmp.path().join("roots");
    engine::create_root_set(
        &RootCreateRequest {
            organization: "Test Org LLC".into(),
            country: "US".into(),
            algorithms: vec![AlgorithmId::Ed25519],
            generation: 1,
            partition: "A".into(),
            cross_signing: CrossSigningMode::Both,
            not_before: ts("2026-01-01T00:00:00Z"),
            not_after: ts("2046-01-01T00:00:00Z"),
            allow_legacy_root_strength: false,
            private_store: None,
        },
        &root,
    )
    .unwrap();
    let issuer = tmp.path().join("base");
    engine::create_issuer_set(
        &IssuerCreateRequest {
            root_bundle: root.clone(),
            root_signer: SignerSource {
                key_dir: root.join("private/keys"),
            },
            name: "Base".into(),
            purpose: IssuerPurpose::General,
            ou: None,
            slots: vec![(AlgorithmId::Ed25519, "a".to_string())],
            cross_signing: CrossSigningMode::Both,
            not_before: ts("2026-01-01T00:00:00Z"),
            not_after: ts("2031-01-01T00:00:00Z"),
            allow_legacy_root_strength: false,
            permitted_dns: vec![],
            private_store: None,
        },
        &issuer,
    )
    .unwrap();
    let leaf = tmp.path().join("svc-leaf");
    engine::issue_leaf(
        &LeafIssueRequest {
            issuer_bundle: issuer.clone(),
            issuer_signer: SignerSource {
                key_dir: issuer.join("private/keys"),
            },
            profile: "server".into(),
            name: "svc-leaf".into(),
            common_name: None,
            dns_names: vec!["svc.example.test".into()],
            ip_addresses: vec![],
            csr: None,
            key_algorithm: None,
            issuer_algorithm: None,
            issuer_partition: None,
            not_before: ts("2026-01-01T00:00:00Z"),
            not_after: ts("2026-04-01T00:00:00Z"),
            private_store: None,
        },
        &leaf,
    )
    .unwrap();
    (root, issuer, leaf)
}

// ------------------------------------------------ independent oracle

/// One DER TLV at the start of `buf`: (tag, header+content length,
/// content range). Hand-written so the oracle shares no parsing code
/// with the planner (which goes through x509-parser).
fn tlv(buf: &[u8]) -> (u8, usize, std::ops::Range<usize>) {
    let tag = buf[0];
    let first = buf[1] as usize;
    let (len, hdr) = if first < 0x80 {
        (first, 2)
    } else {
        let n = first & 0x7f;
        let mut len = 0usize;
        for b in &buf[2..2 + n] {
            len = (len << 8) | *b as usize;
        }
        (len, 2 + n)
    };
    (tag, hdr + len, hdr..hdr + len)
}

/// SubjectPublicKeyInfo of a certificate (RFC 5280 §4.1): the 7th
/// TBSCertificate element when the explicit [0] version is present.
fn spki_by_hand(cert: &[u8]) -> Vec<u8> {
    let (tag, _, body) = tlv(cert);
    assert_eq!(tag, 0x30, "Certificate is a SEQUENCE");
    let cert_body = &cert[body];
    let (tag, _, tbs_range) = tlv(cert_body);
    assert_eq!(tag, 0x30, "TBSCertificate is a SEQUENCE");
    let mut rest = &cert_body[tbs_range];
    // version [0] (optional), serial, signature, issuer, validity, subject
    let (tag, total, _) = tlv(rest);
    if tag == 0xa0 {
        rest = &rest[total..];
    }
    for _ in 0..5 {
        let (_, total, _) = tlv(rest);
        rest = &rest[total..];
    }
    let (tag, total, _) = tlv(rest);
    assert_eq!(tag, 0x30, "SubjectPublicKeyInfo is a SEQUENCE");
    rest[..total].to_vec()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// (first certificate of `role`: DER, fingerprint, algorithm name,
/// partition), read straight from the bundle files.
fn first_cert(bundle_dir: &Path, role: &str) -> (Vec<u8>, String, String, String) {
    let b = load_bundle(bundle_dir).unwrap();
    let rec = b
        .manifest
        .certificates
        .iter()
        .find(|c| c.role == role)
        .unwrap_or_else(|| panic!("no {role} certificate in {}", bundle_dir.display()));
    let pem = std::fs::read_to_string(bundle_dir.join(&rec.cert_path)).unwrap();
    let b64: String = pem
        .lines()
        .skip_while(|l| !l.starts_with("-----BEGIN CERTIFICATE"))
        .skip(1)
        .take_while(|l| !l.starts_with("-----END"))
        .collect();
    let der = fleet_cert::bundle::base64_decode(&b64).unwrap();
    let alg = rec.algorithm.info().name.to_string();
    (
        der,
        rec.fingerprint_sha256.clone(),
        alg,
        rec.partition.to_ascii_lowercase(),
    )
}

fn bundle_id(dir: &Path) -> String {
    load_bundle(dir).unwrap().manifest.bundle_id
}

fn policy() -> DnsPolicy {
    dns::read_policy(&Path::new(FIXTURES).join("policy.json")).unwrap()
}

fn golden_plan(issuer: &Path, leaf: &Path) -> ChangeSet {
    let bundles = vec![load_bundle(issuer).unwrap(), load_bundle(leaf).unwrap()];
    dns::plan_with(
        "example.test",
        dns::DEFAULT_TTL,
        &bundles,
        &policy(),
        None,
        ts(GENERATED_AT),
    )
    .unwrap()
}

// ------------------------------------------------ tests

#[test]
fn plan_matches_the_golden_change_set() {
    // Oracle: the checked-in golden change-set (contract §3 shape) with
    // digests recomputed by the independent DER walk above.
    let tmp = TempDir::new("golden");
    let (_root, issuer, leaf) = hierarchy(&tmp);
    let cs = golden_plan(&issuer, &leaf);
    let actual = dns::render_json(&cs).unwrap();

    let (leaf_der, leaf_fp, leaf_alg, leaf_part) = first_cert(&leaf, "leaf");
    let (issuer_der, issuer_fp, _, _) = first_cert(&issuer, "issuer");
    let mut ids = [bundle_id(&issuer), bundle_id(&leaf)];
    ids.sort();
    let expected = std::fs::read_to_string(Path::new(FIXTURES).join("golden-change-set.json"))
        .unwrap()
        .replace("{{BUNDLE_1}}", &ids[0])
        .replace("{{BUNDLE_2}}", &ids[1])
        .replace(
            "{{LEAF_SPKI_SHA256}}",
            &hex(&Sha256::digest(spki_by_hand(&leaf_der))),
        )
        .replace(
            "{{ISSUER_SPKI_SHA256}}",
            &hex(&Sha256::digest(spki_by_hand(&issuer_der))),
        )
        .replace("{{LEAF_FP}}", &leaf_fp)
        .replace("{{LEAF_ALG}}", &leaf_alg)
        .replace("{{LEAF_PART}}", &leaf_part)
        .replace("{{ISSUER_FP}}", &issuer_fp);
    assert_eq!(actual.trim_end(), expected.trim_end());

    // The same digests through the full-certificate selector and
    // SHA-512 matching, against the independent path.
    assert_eq!(
        dns::association(&leaf_der, 0, 1).unwrap(),
        hex(&Sha256::digest(&leaf_der))
    );
    assert_eq!(
        dns::association(&leaf_der, 1, 2).unwrap(),
        hex(&sha2::Sha512::digest(spki_by_hand(&leaf_der)))
    );
}

#[test]
fn a_fleet_ca_only_fixture_has_no_caa() {
    // Oracle: contract §1 / spec §24.8 — CAA never names the private CA.
    let tmp = TempDir::new("caa");
    let (root, issuer, _leaf) = hierarchy(&tmp);
    let bundles = vec![load_bundle(&root).unwrap(), load_bundle(&issuer).unwrap()];
    let cs = dns::plan_with(
        "example.test",
        dns::DEFAULT_TTL,
        &bundles,
        &DnsPolicy::default(),
        None,
        ts(GENERATED_AT),
    )
    .unwrap();
    assert!(cs.records.iter().all(|r| r.record_type != "CAA"));
    assert!(cs.records.is_empty());
    assert!(cs.prerequisites.iter().any(|p| p == dns::FLEET_CA_CAA_NOTE));

    // Naming the Fleet CA (by its organization) as a CAA issuer is a
    // contract error with an explanation, not a rendered record.
    let mut naming = DnsPolicy::default();
    naming.caa.push(dns::CaaPolicy {
        name: "pki".into(),
        issue: vec!["Test Org LLC".into()],
        issuewild: vec![],
        iodef: None,
    });
    let err = dns::plan_with(
        "example.test",
        dns::DEFAULT_TTL,
        &bundles,
        &naming,
        None,
        ts(GENERATED_AT),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("private Fleet CA"), "{err}");
}

#[test]
fn current_snapshot_diffs_without_fabricating_changes() {
    // Oracle: contract §2 — equal sets report no changes; deletes only
    // at the names the plan manages.
    let tmp = TempDir::new("current");
    let (_root, issuer, leaf) = hierarchy(&tmp);
    let desired = golden_plan(&issuer, &leaf);
    let snap = tmp.path().join("current.json");
    std::fs::write(&snap, dns::render_json(&desired).unwrap()).unwrap();
    let bundles = vec![load_bundle(&issuer).unwrap(), load_bundle(&leaf).unwrap()];
    let same = dns::plan_with(
        "example.test",
        dns::DEFAULT_TTL,
        &bundles,
        &policy(),
        Some(&snap),
        ts(GENERATED_AT),
    )
    .unwrap();
    assert!(same.is_no_change(), "{:?}", same.records);
    assert_eq!(same.records.len(), desired.records.len());

    // A stale issuer at a managed name is deleted; a record at a name
    // the plan does not manage is left alone; a missing record is created.
    let snapshot = serde_json::json!([
        {"type": "CAA", "name": "pki.example.test.", "ttl": 3600, "value": "0 issue \"old-ca.example\""},
        {"type": "CAA", "name": "other.example.test.", "ttl": 3600, "value": "0 issue \"old-ca.example\""},
        {"type": "CAA", "name": "pki.example.test", "ttl": 3600, "value": "0 ISSUE \"letsencrypt.org\""}
    ]);
    std::fs::write(&snap, snapshot.to_string()).unwrap();
    let diff = dns::plan_with(
        "example.test",
        dns::DEFAULT_TTL,
        &bundles,
        &policy(),
        Some(&snap),
        ts(GENERATED_AT),
    )
    .unwrap();
    let action = |value: &str, name: &str| {
        diff.records
            .iter()
            .find(|r| r.value == value && r.name == name)
            .map(|r| r.action.clone())
    };
    assert_eq!(
        action("0 issue \"old-ca.example\"", "pki.example.test.").as_deref(),
        Some("delete")
    );
    assert_eq!(action("0 issue \"old-ca.example\"", "other.example.test."), None);
    assert_eq!(
        action("0 issue \"letsencrypt.org\"", "pki.example.test.").as_deref(),
        Some("no-op")
    );
    assert!(
        diff.records
            .iter()
            .filter(|r| r.record_type == "TLSA")
            .all(|r| r.action == "create")
    );
}

#[test]
fn verify_proposes_the_plan_and_catches_tampering() {
    // Oracle: contract §2 verify checks + §5 state vocabulary.
    let tmp = TempDir::new("verify");
    let (_root, issuer, leaf) = hierarchy(&tmp);
    let cs = golden_plan(&issuer, &leaf);
    let bundles = vec![issuer.clone(), leaf.clone()];
    let report = dns::verify(&cs, "golden", &bundles).unwrap();
    assert!(
        report.valid,
        "{:?}",
        report.checks.iter().filter(|c| !c.ok).collect::<Vec<_>>()
    );
    assert_eq!(report.state, "proposed");
    let wording = serde_json::to_string(&report).unwrap().to_ascii_lowercase();
    assert!(!wording.contains("published to"), "{wording}");

    let tlsa_index = cs
        .records
        .iter()
        .position(|r| r.record_type == "TLSA")
        .unwrap();
    let invalid = |mutate: &dyn Fn(&mut ChangeSet), bundles: &[PathBuf]| {
        let mut bad = cs.clone();
        mutate(&mut bad);
        let r = dns::verify(&bad, "bad", bundles).unwrap();
        assert!(!r.valid);
        assert_eq!(r.state, "invalid");
    };
    // A flipped digest no longer recomputes from any bundle certificate.
    invalid(
        &|c| {
            let v = &mut c.records[tlsa_index].value;
            let last = v.pop().unwrap();
            v.push(if last == '0' { '1' } else { '0' });
        },
        &bundles,
    );
    // PKIX usages are deferred in v1.
    invalid(
        &|c| {
            let v = c.records[tlsa_index].value.clone();
            c.records[tlsa_index].value = format!("1{}", &v[1..]);
        },
        &bundles,
    );
    // A TLSA record must be marked DNSSEC-gated.
    invalid(&|c| c.records[tlsa_index].requires.clear(), &bundles);
    // Without bundles nothing can be recomputed.
    invalid(&|_| {}, &[]);
    // A CAA record naming the Fleet CA is refused on verify too.
    invalid(
        &|c| {
            let i = c.records.iter().position(|r| r.record_type == "CAA").unwrap();
            c.records[i].value = "0 issue \"fleet-ca\"".into();
        },
        &bundles,
    );
    // Out-of-zone owner and a zero TTL.
    invalid(&|c| c.records[0].name = "svc.elsewhere.test.".into(), &bundles);
    invalid(&|c| c.records[0].ttl = 0, &bundles);
}

#[test]
fn render_is_pure_and_marks_the_state() {
    // Oracle: contract §2 render rules (same change-set in, same bytes
    // out) and §5 wording.
    let tmp = TempDir::new("render");
    let (_root, issuer, leaf) = hierarchy(&tmp);
    let cs = golden_plan(&issuer, &leaf);
    let zone = dns::render_zone(&cs);
    assert_eq!(zone, dns::render_zone(&cs));
    assert!(zone.contains("PROPOSED"));
    let tlsa = cs.records.iter().find(|r| r.record_type == "TLSA").unwrap();
    assert!(zone.contains(&format!("{}\t3600\tIN\tTLSA\t{}", tlsa.name, tlsa.value)));
    assert!(zone.contains("pki.example.test.\t3600\tIN\tCAA\t0 issue \"letsencrypt.org\""));

    // A delete is a comment in the zone fragment, never a live record.
    let mut with_delete = cs.clone();
    with_delete.records[0].action = "delete".into();
    let z = dns::render_zone(&with_delete);
    assert!(z.contains(&format!("; DELETE {}\t", with_delete.records[0].name)));

    // JSON renders back to the same change-set.
    let json = dns::render_json(&cs).unwrap();
    let back: ChangeSet = serde_json::from_str(&json).unwrap();
    assert_eq!(dns::render_json(&back).unwrap(), json);
    let yaml = dns::render_yaml(&cs);
    assert!(yaml.starts_with("schema: fleet.pki.dns/v1\nzone: example.test\n"));
    assert!(yaml.contains("    requires: [dnssec]\n"));
}

#[test]
fn cli_is_offline_and_writes_only_under_out() {
    // Oracle: contract §2/§7 — no provider, endpoint or credential
    // flag exists, and the only files written are under --out.
    let usage = fleet_cert::cli::DNS_USAGE.to_ascii_lowercase();
    for word in [
        "--provider",
        "--endpoint",
        "--server",
        "--token",
        "--credential",
        "--api",
        "--resolver",
        "--nameserver",
    ] {
        assert!(!usage.contains(word), "dns usage offers {word}");
    }
    let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    for flag in ["--provider", "--resolver", "--token"] {
        assert_eq!(
            fleet_cert::run(&args(&[
                "dns",
                "plan",
                "--zone",
                "example.test",
                flag,
                "x"
            ])),
            2,
            "{flag} must be an unknown option"
        );
    }
    assert_eq!(fleet_cert::run(&args(&["dns", "apply"])), 1);

    let tmp = TempDir::new("cli");
    let (_root, issuer, leaf) = hierarchy(&tmp);
    let listing = |dir: &Path| {
        let mut all = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p.clone());
                }
                all.push(p);
            }
        }
        all.sort();
        all
    };
    let before = listing(tmp.path());
    let out = tmp.path().join("out");
    let policy = Path::new(FIXTURES).join("policy.json");
    let code = fleet_cert::run(&args(&[
        "dns",
        "plan",
        "--zone",
        "example.test",
        "--bundle",
        issuer.to_str().unwrap(),
        "--bundle",
        leaf.to_str().unwrap(),
        "--policy",
        policy.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
        "--dry-run",
        "--report",
        "json",
    ]));
    assert_eq!(code, 0);
    let change_set = out.join("dns-change-set.json");
    let code = fleet_cert::run(&args(&[
        "dns",
        "render",
        "--change-set",
        change_set.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]));
    assert_eq!(code, 0);
    let code = fleet_cert::run(&args(&[
        "dns",
        "verify",
        "--change-set",
        change_set.to_str().unwrap(),
        "--bundle",
        issuer.to_str().unwrap(),
        "--bundle",
        leaf.to_str().unwrap(),
    ]));
    assert_eq!(code, 0, "the planned change-set verifies");
    let after = listing(tmp.path());
    let new: Vec<_> = after.iter().filter(|p| !before.contains(p)).collect();
    assert!(!new.is_empty());
    for p in &new {
        assert!(p.starts_with(&out), "wrote outside --out: {}", p.display());
    }
    assert!(out.join("dns.zone").is_file());
}
