//! `fleet cert batch validate | plan | apply | status` (#13730, spec
//! §§6.5, 25.4), driven end to end over a synthetic hierarchy:
//!
//! - plan resolves the DAG (steps listed out of order), checks every
//!   input, and writes nothing;
//! - a dry run executes every step and leaves the base untouched;
//! - a failing step (here, a failed expectation on the last step)
//!   rolls back everything, including steps that had succeeded;
//! - apply needs the reviewed digest, commits once, writes a receipt,
//!   and never reissues: a second apply returns the receipt, a changed
//!   batch under the same idempotency key is refused, and drift on
//!   disk is reported;
//! - the parser refuses ambient signers, undeclared references,
//!   cycles, interpolation, external key stores, root ceremonies and
//!   paths that escape the base.

use fleet_cert::batch::{self, ApplyOptions};
use fleet_cert::bundle::load_bundle;
use fleet_cert::engine;
use fleet_cert::graph::CrossSigningMode;
use fleet_cert::model::RootCreateRequest;
use fleet_cert::timeparse::Rfc3339;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "fleet-cert-batch-{tag}-{}-{nanos}",
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

fn ts(s: &str) -> Rfc3339 {
    Rfc3339::parse(s).unwrap()
}

/// A base directory holding one Ed25519 root set at `roots/g1`.
fn base_with_root(tag: &str) -> TempDir {
    let tmp = TempDir::new(tag);
    engine::create_root_set(
        &RootCreateRequest {
            organization: "Test Org LLC".into(),
            country: "US".into(),
            algorithms: vec![fleet_cert::algorithm::AlgorithmId::Ed25519],
            generation: 1,
            partition: "A".into(),
            cross_signing: CrossSigningMode::Both,
            not_before: ts("2026-01-01T00:00:00Z"),
            not_after: ts("2046-01-01T00:00:00Z"),
            allow_legacy_root_strength: false,
            private_store: None,
        },
        &tmp.path().join("roots/g1"),
    )
    .unwrap();
    tmp
}

/// An issuer set plus two leaves for a service, listed out of order:
/// the DAG, not the file order, decides execution.
fn rollout() -> Value {
    json!({
        "schema": "fleet.batch/v1",
        "idempotency_key": "svc-rollout-1",
        "mode": "plan-first",
        "operations": [
            {
                "id": "svc",
                "after": ["web"],
                "command": "cert.leaf.issue",
                "out": "leaves/svc",
                "with": {
                    "issuer": "@web.bundle",
                    "issuer_key_dir": "@web.keys",
                    "profile": "server",
                    "dns": ["svc.example.test"],
                    "not_before": "2026-01-01T00:00:00Z",
                    "valid_for": "90d"
                },
                "expect": { "certificates": 1 }
            },
            {
                "id": "web",
                "command": "cert.issuer.create",
                "out": "issuers/web",
                "with": {
                    "root": "roots/g1",
                    "root_key_dir": "roots/g1/private/keys",
                    "name": "Internal Web",
                    "algorithms": ["ed25519"],
                    "not_before": "2026-01-01T00:00:00Z",
                    "not_after": "2031-01-01T00:00:00Z"
                }
            },
            {
                "id": "api",
                "after": ["svc"],
                "command": "cert.leaf.issue",
                "out": "leaves/api",
                "with": {
                    "issuer": "@web",
                    "issuer_key_dir": "@web.keys",
                    "profile": "server",
                    "dns": ["api.example.test"],
                    "not_before": "2026-01-01T00:00:00Z",
                    "valid_for": "90d"
                }
            }
        ]
    })
}

fn bytes(v: &Value) -> Vec<u8> {
    serde_json::to_vec_pretty(v).unwrap()
}

fn sha(b: &[u8]) -> String {
    Sha256::digest(b)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

/// Every path under `dir`, relative, sorted: the "nothing changed" oracle.
fn listing(dir: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            out.push(path.strip_prefix(root).unwrap().display().to_string());
            if path.is_dir() {
                walk(root, &path, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

fn op(v: &mut Value, id: &str) -> Value {
    v["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == id)
        .unwrap()
        .clone()
}

fn set_op(v: &mut Value, id: &str, new: Value) {
    for o in v["operations"].as_array_mut().unwrap() {
        if o["id"] == id {
            *o = new.clone();
        }
    }
}

#[test]
fn plan_orders_the_dag_and_writes_nothing() {
    let tmp = base_with_root("plan");
    let before = listing(tmp.path());
    let doc = bytes(&rollout());
    let b = batch::load(&doc, tmp.path()).unwrap();
    assert_eq!(b.order(), vec!["web", "svc", "api"]);
    assert_eq!(b.sha256, sha(&doc));
    assert_eq!(b.receipt_key(), "svc-rollout-1");

    let plan = batch::plan(&b);
    assert_eq!(plan.status, "planned");
    let signers: Vec<&str> = plan.operations.iter().map(|o| o.signer.as_str()).collect();
    assert_eq!(signers, vec!["roots/g1/private/keys", "@web.keys", "@web.keys"]);
    let parents: Vec<&str> = plan.operations.iter().map(|o| o.parent.as_str()).collect();
    assert_eq!(parents, vec!["roots/g1", "@web.bundle", "@web.bundle"]);
    assert!(plan.operations.iter().all(|o| o.fingerprints.is_empty()));

    let valid = batch::validate(&b, tmp.path()).unwrap();
    assert_eq!(valid.status, "validated");
    let text = batch::render_text(&plan, "rollout.json", "base");
    assert!(text.contains(&format!("--approve {}", b.sha256)), "{text}");
    assert_eq!(batch::status(&b, tmp.path()).unwrap().status, "pending");
    assert_eq!(listing(tmp.path()), before, "plan and validate must write nothing");
}

#[test]
fn dry_run_runs_every_step_and_leaves_no_trace() {
    let tmp = base_with_root("dry");
    let before = listing(tmp.path());
    let b = batch::load(&bytes(&rollout()), tmp.path()).unwrap();
    let rep = batch::apply(
        &b,
        tmp.path(),
        &ApplyOptions {
            dry_run: true,
            approve: None,
        },
    )
    .unwrap();
    assert_eq!(rep.status, "dry-run");
    assert_eq!(rep.operations.len(), 3);
    for o in &rep.operations {
        assert_eq!(o.status, "dry-run");
        assert!(!o.fingerprints.is_empty(), "{} produced no certificate", o.id);
        assert!(o.bundle_id.is_some());
    }
    assert_eq!(rep.operations[1].fingerprints.len(), 1);
    assert_eq!(listing(tmp.path()), before, "a dry run must leave the base as it was");
}

#[test]
fn a_failing_step_rolls_back_everything() {
    let tmp = base_with_root("rollback");
    let before = listing(tmp.path());
    let mut v = rollout();
    let mut api = op(&mut v, "api");
    api["expect"] = json!({ "certificates": 5 });
    set_op(&mut v, "api", api);
    let doc = bytes(&v);
    let b = batch::load(&doc, tmp.path()).unwrap();
    let err = batch::apply(
        &b,
        tmp.path(),
        &ApplyOptions {
            dry_run: false,
            approve: Some(b.sha256.clone()),
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("operation 'api' failed"), "{err}");
    assert!(err.contains("expected 5 certificate"), "{err}");
    assert!(err.contains("Nothing was applied"), "{err}");
    assert_eq!(
        listing(tmp.path()),
        before,
        "the issuer and the first leaf succeeded in staging but must not survive"
    );
}

#[test]
fn apply_commits_once_and_never_reissues() {
    let tmp = base_with_root("apply");
    let doc = bytes(&rollout());
    let b = batch::load(&doc, tmp.path()).unwrap();

    // The reviewed digest is required, and must match.
    let none = batch::apply(&b, tmp.path(), &ApplyOptions::default()).unwrap_err();
    assert!(none.to_string().contains("--approve"), "{none}");
    let wrong = batch::apply(
        &b,
        tmp.path(),
        &ApplyOptions {
            dry_run: false,
            approve: Some("0".repeat(64)),
        },
    )
    .unwrap_err();
    assert!(wrong.to_string().contains("changed since it was reviewed"), "{wrong}");
    assert!(!tmp.path().join("issuers").exists());

    let approve = ApplyOptions {
        dry_run: false,
        approve: Some(b.sha256.clone()),
    };
    let rep = batch::apply(&b, tmp.path(), &approve).unwrap();
    assert_eq!(rep.status, "applied");

    // The bundles are in place, verified, and chained to each other.
    let issuer = load_bundle(&tmp.path().join("issuers/web")).unwrap();
    let svc = load_bundle(&tmp.path().join("leaves/svc")).unwrap();
    let api = load_bundle(&tmp.path().join("leaves/api")).unwrap();
    for leaf in [&svc, &api] {
        let parent = leaf.manifest.parent.as_ref().unwrap();
        assert_eq!(parent.bundle_id, issuer.manifest.bundle_id);
        let cert = &leaf.manifest.certificates[0];
        assert!(
            issuer
                .manifest
                .certificates
                .iter()
                .any(|c| c.ski_hex == cert.aki_hex && c.subject_dn == cert.issuer_dn),
            "leaf {} is not signed by the batch's issuer",
            leaf.manifest.name
        );
    }
    assert_eq!(svc.manifest.name, "svc");
    assert_eq!(rep.operations[1].fingerprints, vec![svc.manifest.certificates[0].fingerprint_sha256.clone()]);

    // Only the receipt remains in the state directory.
    let state = listing(&tmp.path().join(".fleet-batch"));
    assert_eq!(state, vec!["receipts", "receipts/svc-rollout-1.json"]);
    assert_eq!(batch::status(&b, tmp.path()).unwrap().status, "applied");

    // A second apply reissues nothing.
    let svc_manifest = std::fs::read(tmp.path().join("leaves/svc/manifest.json")).unwrap();
    let again = batch::apply(&b, tmp.path(), &approve).unwrap();
    assert_eq!(again.status, "already-applied");
    assert_eq!(
        std::fs::read(tmp.path().join("leaves/svc/manifest.json")).unwrap(),
        svc_manifest
    );
    assert_eq!(batch::validate(&b, tmp.path()).unwrap().status, "already-applied");

    // A different batch under the same idempotency key is refused.
    let mut changed = rollout();
    let mut api_op = op(&mut changed, "api");
    api_op["with"]["dns"] = json!(["api2.example.test"]);
    set_op(&mut changed, "api", api_op);
    let b2 = batch::load(&bytes(&changed), tmp.path()).unwrap();
    let refused = batch::apply(
        &b2,
        tmp.path(),
        &ApplyOptions {
            dry_run: false,
            approve: Some(b2.sha256.clone()),
        },
    )
    .unwrap_err();
    assert!(refused.to_string().contains("different batch"), "{refused}");

    // Drift on disk is reported, and apply refuses to paper over it.
    std::fs::remove_dir_all(tmp.path().join("leaves/api")).unwrap();
    let st = batch::status(&b, tmp.path()).unwrap();
    assert_eq!(st.status, "drift");
    assert_eq!(st.operations[2].status, "missing");
    assert!(batch::apply(&b, tmp.path(), &approve).is_err());
}

#[test]
fn the_parser_refuses_unsafe_batches() {
    let tmp = base_with_root("refuse");
    let cases: Vec<(&str, Box<dyn Fn(&mut Value)>, &str)> = vec![
        (
            "bare signer reference",
            Box::new(|v| {
                let mut o = op(v, "svc");
                o["with"]["issuer_key_dir"] = json!("@web");
                set_op(v, "svc", o);
            }),
            "explicitly",
        ),
        (
            "reference outside the after chain",
            Box::new(|v| {
                let mut o = op(v, "api");
                o["after"] = json!([]);
                set_op(v, "api", o);
            }),
            "not in this operation's 'after'",
        ),
        (
            "cycle",
            Box::new(|v| {
                let mut o = op(v, "web");
                o["after"] = json!(["api"]);
                set_op(v, "web", o);
            }),
            "cycle",
        ),
        (
            "external key store",
            Box::new(|v| {
                let mut o = op(v, "svc");
                o["with"]["private_store"] = json!("/var/keys");
                set_op(v, "svc", o);
            }),
            "private_store",
        ),
        (
            "interpolation",
            Box::new(|v| {
                let mut o = op(v, "svc");
                o["with"]["issuer"] = json!("${jobs.web.bundle_ref}");
                set_op(v, "svc", o);
            }),
            "interpolation",
        ),
        (
            "root ceremony",
            Box::new(|v| {
                let mut o = op(v, "web");
                o["command"] = json!("cert.root.create");
                set_op(v, "web", o);
            }),
            "key ceremony",
        ),
        (
            "root from the batch",
            Box::new(|v| {
                let mut o = op(v, "web");
                o["with"]["root"] = json!("@svc");
                set_op(v, "web", o);
            }),
            "existing root set",
        ),
        (
            "escaping out",
            Box::new(|v| {
                let mut o = op(v, "api");
                o["out"] = json!("../api");
                set_op(v, "api", o);
            }),
            "relative path",
        ),
        (
            "nested outs",
            Box::new(|v| {
                let mut o = op(v, "api");
                o["out"] = json!("leaves/svc/api");
                set_op(v, "api", o);
            }),
            "overlaps",
        ),
        (
            "signer of another source",
            Box::new(|v| {
                let mut o = op(v, "svc");
                o["with"]["issuer_key_dir"] = json!("roots/g1/private/keys");
                set_op(v, "svc", o);
            }),
            "must be '@web.keys'",
        ),
        (
            "unknown argument",
            Box::new(|v| {
                let mut o = op(v, "svc");
                o["with"]["bogus"] = json!(true);
                set_op(v, "svc", o);
            }),
            "unknown field",
        ),
        (
            "missing root bundle",
            Box::new(|v| {
                let mut o = op(v, "web");
                o["with"]["root"] = json!("roots/nope");
                set_op(v, "web", o);
            }),
            "manifest",
        ),
        (
            "wrong schema",
            Box::new(|v| {
                v["schema"] = json!("fleet.batch/v0");
            }),
            "unsupported batch schema",
        ),
    ];
    for (name, mutate, want) in cases {
        let mut v = rollout();
        mutate(&mut v);
        let err = batch::load(&bytes(&v), tmp.path())
            .map(|_| ())
            .expect_err(name)
            .to_string();
        assert!(err.contains(want), "{name}: {err}");
    }

    // Existing output without this batch's receipt is a conflict.
    let b = batch::load(&bytes(&rollout()), tmp.path()).unwrap();
    std::fs::create_dir_all(tmp.path().join("leaves/svc")).unwrap();
    let err = batch::validate(&b, tmp.path()).unwrap_err().to_string();
    assert!(err.contains("already exists"), "{err}");
    assert!(batch::apply(&b, tmp.path(), &ApplyOptions { dry_run: true, approve: None }).is_err());
}

#[test]
fn cli_round_trip() {
    let tmp = base_with_root("cli");
    let file = tmp.path().join("rollout.json");
    let doc = bytes(&rollout());
    std::fs::write(&file, &doc).unwrap();
    let f = file.display().to_string();
    let base = tmp.path().display().to_string();
    let run = |items: &[&str]| fleet_cert::run(&items.iter().map(|s| s.to_string()).collect::<Vec<_>>());

    assert_eq!(run(&["batch"]), 2);
    assert_eq!(run(&["batch", "--help"]), 0);
    assert_eq!(run(&["batch", "plan", "--file", &f]), 2, "--base is required");
    assert_eq!(run(&["batch", "plan", "--file", &f, "--base", &base, "--json"]), 0);
    assert_eq!(run(&["batch", "validate", "--file", &f, "--base", &base]), 0);
    assert_eq!(run(&["batch", "apply", "--file", &f, "--base", &base]), 1, "no --approve");
    assert_eq!(run(&["batch", "apply", "--file", &f, "--base", &base, "--dry-run"]), 0);
    assert!(!tmp.path().join("issuers").exists());
    let digest = sha(&doc);
    assert_eq!(run(&["batch", "apply", "--file", &f, "--base", &base, "--approve", &digest]), 0);
    assert!(tmp.path().join("leaves/api/manifest.json").exists());
    assert_eq!(run(&["batch", "status", "--file", &f, "--base", &base]), 0);
    assert_eq!(run(&["batch", "apply", "--file", &f, "--base", &base, "--approve", &digest]), 0);
    assert_eq!(run(&["batch", "resume", "--file", &f, "--base", &base]), 1);
    assert_eq!(run(&["batch", "bogus"]), 2);
    std::fs::remove_dir_all(tmp.path().join("leaves/svc")).unwrap();
    assert_eq!(run(&["batch", "status", "--file", &f, "--base", &base]), 1, "drift");
}
