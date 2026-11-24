//! The `fleet cert` CLI: a thin shell over the crate API (spec
//! §6.4). Each verb parses flags into a typed request in
//! [`crate::model`], calls [`crate::engine`], and renders text or,
//! with `--json`, one JSON value on stdout.
//!
//! Exit codes follow the fleet contract: 0 ok · 2 usage · 1
//! operational/policy/validation failure.

use crate::algorithm::{self, AlgorithmId};
use crate::bundle::{list_bundles, load_bundle};
use crate::chain::TrustSelection;
use crate::engine;
use crate::error::{CertError, Result};
use crate::graph::CrossSigningMode;
use crate::model::{
    BundleType, IssuerCreateRequest, IssuerPurpose, LeafIssueRequest, RootCreateRequest,
    SignerSource,
};
use crate::profile;
use crate::timeparse::{CalendarDuration, Rfc3339};
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyUsagePurpose};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// Full usage for the cert domain (also what `fleet cert --help`
/// prints through the bin crate's help carve-out).
pub const CERT_USAGE: &str = "\
Usage: fleet cert <group> <verb> [options]
       fleet pki  <group> <verb> [options]   (alias)

Groups and verbs (spec §6.1 tree; status as built):
  root     create | inspect | list | validate
  issuer   create | inspect | list
  leaf     issue | inspect | list
  chain    build | test
  bundle   inspect
  publish  plan | verify (apply | status deferred, production-gated)
  profile  list | show
  algorithms list | describe
  explain  [topic]
  doctor

Common options:
  --json                  Emit one JSON value instead of text.
  --not-before <rfc3339>  Validity start (default: now).
  --not-after <rfc3339>   Exact validity end (mutually exclusive with --valid-for).
  --valid-for <duration>  Lifetime from not-before: 20y, 90d, '15 years 7 days'.

Signing operations never discover keys ambiently: issuer/leaf
creation requires the signer's key directory as an explicit
argument (spec §24.3). Run `fleet cert explain signer-source`
for the rule, `fleet cert doctor` for what is built vs not.
";

pub fn run(args: &[String]) -> i32 {
    match run_inner(args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("fleet cert: {e}");
            e.exit_code()
        }
    }
}

// ---------------------------------------------------------------- parsing

/// A minimal flag parser in the fleet-agent style: `--flag value`,
/// `--flag=value`, boolean switches, repeatable value flags.
struct Parsed {
    positionals: Vec<String>,
    values: BTreeMap<String, Vec<String>>,
    bools: BTreeSet<String>,
}

impl Parsed {
    fn get(&self, name: &str) -> Option<&str> {
        self.values
            .get(name)
            .and_then(|v| v.last())
            .map(|s| s.as_str())
    }
    fn all(&self, name: &str) -> Vec<String> {
        self.values.get(name).cloned().unwrap_or_default()
    }
    fn has(&self, name: &str) -> bool {
        self.bools.contains(name) || self.values.contains_key(name)
    }
    fn require(&self, name: &str, what: &str) -> Result<String> {
        self.get(name)
            .map(|s| s.to_string())
            .ok_or_else(|| CertError::usage(format!("{what} requires --{name} <value>")))
    }
    fn positional(&self, idx: usize) -> Option<&str> {
        self.positionals.get(idx).map(|s| s.as_str())
    }
}

fn parse_args(args: &[String], value_flags: &[&str], bool_flags: &[&str]) -> Result<Parsed> {
    let mut parsed = Parsed {
        positionals: Vec::new(),
        values: BTreeMap::new(),
        bools: BTreeSet::new(),
    };
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if let Some(flag) = arg.strip_prefix("--") {
            let (name, inline) = match flag.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (flag, None),
            };
            if bool_flags.contains(&name) {
                if inline.is_some() {
                    return Err(CertError::usage(format!("--{name} takes no value")));
                }
                parsed.bools.insert(name.to_string());
            } else if value_flags.contains(&name) {
                let value = match inline {
                    Some(v) => v,
                    None => {
                        i += 1;
                        args.get(i)
                            .cloned()
                            .ok_or_else(|| CertError::usage(format!("--{name} needs a value")))?
                    }
                };
                parsed
                    .values
                    .entry(name.to_string())
                    .or_default()
                    .push(value);
            } else {
                return Err(CertError::usage(format!(
                    "unknown option --{name} (run 'fleet cert --help')"
                )));
            }
        } else if arg.starts_with('-') && arg.len() > 1 && arg != "-" {
            return Err(CertError::usage(format!(
                "unknown option {arg} (options are long-form --flag)"
            )));
        } else {
            parsed.positionals.push(arg.clone());
        }
        i += 1;
    }
    Ok(parsed)
}

fn json_mode(args: &[String]) -> bool {
    args.iter().any(|a| a == "--json")
}

fn emit(json_out: bool, text: String, value: Value) -> i32 {
    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
        );
    } else {
        print!("{text}");
    }
    0
}

/// The shared validity flags (§5.4/§6.3).
fn validity(p: &Parsed, default_lifetime: CalendarDuration) -> Result<(Rfc3339, Rfc3339)> {
    let nb = p.get("not-before").map(Rfc3339::parse).transpose()?;
    let vf = p
        .get("valid-for")
        .map(CalendarDuration::parse)
        .transpose()?;
    let na = p.get("not-after").map(Rfc3339::parse).transpose()?;
    crate::timeparse::resolve_validity(nb, vf, na, default_lifetime)
}

const TIME_FLAGS: &[&str] = &["not-before", "not-after", "valid-for"];

fn signer_source(p: &Parsed, flag: &str, what: &str) -> Result<SignerSource> {
    match p.get(flag) {
        Some(dir) => Ok(SignerSource {
            key_dir: PathBuf::from(dir),
        }),
        None => Err(CertError::policy(format!(
            "{what} requires an explicit signer source: --{flag} <key-dir> \
             (spec §24.3 — root/issuer signing never uses ambient key discovery)"
        ))),
    }
}

fn algorithms_arg(p: &Parsed) -> Result<Vec<AlgorithmId>> {
    match p.get("algorithms") {
        Some(csv) => {
            let (algos, warnings) = algorithm::parse_algorithm_list(csv)?;
            for w in warnings {
                eprintln!("warning: {w}");
            }
            Ok(algos)
        }
        None => Ok(algorithm::DEFAULT_ALGORITHMS.to_vec()),
    }
}

fn cross_arg(p: &Parsed, default: CrossSigningMode) -> Result<CrossSigningMode> {
    match p.get("cross-signing") {
        Some(s) => CrossSigningMode::parse(s),
        None => Ok(default),
    }
}

// ---------------------------------------------------------------- dispatch

fn run_inner(args: &[String]) -> Result<i32> {
    if args.is_empty() {
        print!("{CERT_USAGE}");
        return Ok(2);
    }
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h" || args[0] == "help") {
        print!("{CERT_USAGE}");
        return Ok(0);
    }
    let json_out = json_mode(args);
    let rest = &args[1..];
    match args[0].as_str() {
        "root" => root_cmd(rest, json_out),
        "issuer" => issuer_cmd(rest, json_out),
        "leaf" => leaf_cmd(rest, json_out),
        "chain" => chain_cmd(rest, json_out),
        "bundle" => bundle_cmd(rest, json_out),
        "publish" => publish_cmd(rest, json_out),
        "profile" => profile_cmd(rest, json_out),
        "algorithms" => algorithms_cmd(rest, json_out),
        "explain" => explain_cmd(rest, json_out),
        "doctor" => doctor_cmd(rest, json_out),
        other => Err(CertError::usage(format!(
            "unknown cert group '{other}' — run 'fleet cert --help' for the surface"
        ))),
    }
}

// ---------------------------------------------------------------- root

fn root_cmd(args: &[String], json_out: bool) -> Result<i32> {
    let verb = args.first().map(|s| s.as_str()).unwrap_or("");
    let rest = &args[1.min(args.len())..];
    match verb {
        "create" => {
            let mut flags = vec![
                "out",
                "organization",
                "country",
                "algorithms",
                "generation",
                "partition",
                "cross-signing",
                "private-store",
            ];
            flags.extend(TIME_FLAGS);
            let p = parse_args(
                rest,
                &flags,
                &["allow-legacy-root-strength", "private-in-bundle", "json"],
            )?;
            // §24.3 ceremony: the key destination must be explicit.
            let private_store = match (p.get("private-store"), p.has("private-in-bundle")) {
                (Some(dir), _) => Some(PathBuf::from(dir)),
                (None, true) => None,
                (None, false) => {
                    return Err(CertError::policy(
                        "root creation is a ceremony: name the root key destination \
                         explicitly — --private-store <dir> or --private-in-bundle \
                         (spec §24.3)",
                    ));
                }
            };
            let generation: u32 = p
                .get("generation")
                .map(|s| {
                    s.parse()
                        .map_err(|_| CertError::usage(format!("bad --generation '{s}'")))
                })
                .transpose()?
                .unwrap_or(1);
            let (nb, na) = validity(
                &p,
                CalendarDuration::years(crate::timeparse::ROOT_DEFAULT_LIFETIME_YEARS),
            )?;
            let req = RootCreateRequest {
                organization: p
                    .get("organization")
                    .unwrap_or("developing.today LLC")
                    .to_string(),
                country: p.get("country").unwrap_or("US").to_string(),
                algorithms: algorithms_arg(&p)?,
                generation,
                partition: p.get("partition").unwrap_or("A").to_string(),
                cross_signing: cross_arg(&p, CrossSigningMode::Both)?,
                not_before: nb,
                not_after: na,
                allow_legacy_root_strength: p.has("allow-legacy-root-strength"),
                private_store,
            };
            let out = p.require("out", "root create")?;
            let manifest = engine::create_root_set(&req, &PathBuf::from(&out))?;
            Ok(emit(
                json_out,
                manifest_text("Root set created", &manifest, &out),
                manifest_json(&manifest, &out),
            ))
        }
        "inspect" => inspect_cmd(rest, json_out, Some(BundleType::RootSet)),
        "list" => list_cmd(rest, json_out, BundleType::RootSet),
        "validate" => {
            let p = parse_args(rest, &["bundle"], &["json"])?;
            let dir = p
                .get("bundle")
                .map(|s| s.to_string())
                .or_else(|| p.positional(0).map(|s| s.to_string()))
                .ok_or_else(|| CertError::usage("root validate needs a bundle directory"))?;
            let checks = engine::validate_root_bundle(&PathBuf::from(&dir))?;
            let all_ok = checks.iter().all(|c| c.ok);
            let mut text = format!("Root bundle validation: {dir}\n");
            for c in &checks {
                text.push_str(&format!(
                    "  [{}] {} — {}\n",
                    if c.ok { "ok" } else { "FAIL" },
                    c.name,
                    c.detail
                ));
            }
            let value = json!({
                "bundle": dir,
                "valid": all_ok,
                "checks": checks.iter().map(|c| json!({"name": c.name, "ok": c.ok, "detail": c.detail})).collect::<Vec<_>>(),
            });
            let code = emit(json_out, text, value);
            Ok(if all_ok { code } else { 1 })
        }
        other => Err(CertError::usage(format!(
            "unknown root verb '{other}' (create | inspect | list | validate)"
        ))),
    }
}

// ---------------------------------------------------------------- issuer

fn issuer_cmd(args: &[String], json_out: bool) -> Result<i32> {
    let verb = args.first().map(|s| s.as_str()).unwrap_or("");
    let rest = &args[1.min(args.len())..];
    match verb {
        "create" => {
            let mut flags = vec![
                "root",
                "root-key-dir",
                "out",
                "name",
                "purpose",
                "ou",
                "algorithms",
                "partitions",
                "cross-signing",
                "permitted-dns",
                "private-store",
            ];
            flags.extend(TIME_FLAGS);
            let p = parse_args(rest, &flags, &["allow-legacy-root-strength", "json"])?;
            let root = p.require("root", "issuer create")?;
            let signer = signer_source(&p, "root-key-dir", "issuer create")?;
            let algos = algorithms_arg(&p)?;
            let partitions: Option<Vec<String>> = p.get("partitions").map(|s| {
                s.split(',')
                    .map(|x| x.trim().to_ascii_lowercase())
                    .filter(|x| !x.is_empty())
                    .collect()
            });
            let slots = crate::model::resolve_slots(&algos, partitions.as_deref())?;
            let (nb, na) = validity(
                &p,
                CalendarDuration::years(crate::timeparse::ISSUER_DEFAULT_LIFETIME_YEARS),
            )?;
            let req = IssuerCreateRequest {
                root_bundle: PathBuf::from(&root),
                root_signer: signer,
                name: p.get("name").unwrap_or("Base").to_string(),
                purpose: IssuerPurpose::parse(p.get("purpose").unwrap_or("general"))?,
                ou: p.get("ou").map(|s| s.to_string()),
                slots,
                cross_signing: cross_arg(&p, CrossSigningMode::RootsOnly)?,
                not_before: nb,
                not_after: na,
                allow_legacy_root_strength: p.has("allow-legacy-root-strength"),
                permitted_dns: p.all("permitted-dns"),
                private_store: p.get("private-store").map(PathBuf::from),
            };
            let out = p.require("out", "issuer create")?;
            let manifest = engine::create_issuer_set(&req, &PathBuf::from(&out))?;
            Ok(emit(
                json_out,
                manifest_text("Issuer set created", &manifest, &out),
                manifest_json(&manifest, &out),
            ))
        }
        "inspect" => inspect_cmd(rest, json_out, Some(BundleType::IssuerSet)),
        "list" => list_cmd(rest, json_out, BundleType::IssuerSet),
        other => Err(CertError::usage(format!(
            "unknown issuer verb '{other}' (create | inspect | list)"
        ))),
    }
}

// ---------------------------------------------------------------- leaf

fn leaf_cmd(args: &[String], json_out: bool) -> Result<i32> {
    let verb = args.first().map(|s| s.as_str()).unwrap_or("");
    let rest = &args[1.min(args.len())..];
    match verb {
        "issue" => {
            let mut flags = vec![
                "issuer",
                "issuer-key-dir",
                "out",
                "profile",
                "name",
                "cn",
                "dns",
                "algorithm",
                "issuer-algorithm",
                "issuer-partition",
                "private-store",
            ];
            flags.extend(TIME_FLAGS);
            let p = parse_args(rest, &flags, &["json"])?;
            let issuer = p.require("issuer", "leaf issue")?;
            let signer = signer_source(&p, "issuer-key-dir", "leaf issue")?;
            let profile_name = p.require("profile", "leaf issue")?;
            let prof = profile::leaf_profile(&profile_name)?;
            let name = p
                .get("name")
                .map(|s| s.to_string())
                .or_else(|| p.positional(0).map(|s| s.to_string()))
                .ok_or_else(|| CertError::usage("leaf issue requires --name <name>"))?;
            let (nb, na) = validity(&p, prof.default_validity)?;
            let parse_algo = |flag: &str| -> Result<Option<AlgorithmId>> {
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
            };
            let req = LeafIssueRequest {
                issuer_bundle: PathBuf::from(&issuer),
                issuer_signer: signer,
                profile: profile_name,
                name,
                common_name: p.get("cn").map(|s| s.to_string()),
                dns_names: p.all("dns"),
                key_algorithm: parse_algo("algorithm")?,
                issuer_algorithm: parse_algo("issuer-algorithm")?,
                issuer_partition: p.get("issuer-partition").map(|s| s.to_string()),
                not_before: nb,
                not_after: na,
                private_store: p.get("private-store").map(PathBuf::from),
            };
            let out = p.require("out", "leaf issue")?;
            let manifest = engine::issue_leaf(&req, &PathBuf::from(&out))?;
            Ok(emit(
                json_out,
                manifest_text("Leaf issued", &manifest, &out),
                manifest_json(&manifest, &out),
            ))
        }
        "inspect" => inspect_cmd(rest, json_out, Some(BundleType::Leaf)),
        "list" => list_cmd(rest, json_out, BundleType::Leaf),
        other => Err(CertError::usage(format!(
            "unknown leaf verb '{other}' (issue | inspect | list)"
        ))),
    }
}

// ---------------------------------------------------------------- chain

fn chain_cmd(args: &[String], json_out: bool) -> Result<i32> {
    let verb = args.first().map(|s| s.as_str()).unwrap_or("");
    let rest = &args[1.min(args.len())..];
    match verb {
        "build" | "test" => {
            let p = parse_args(
                rest,
                &["leaf", "issuer", "root", "trust", "out", "at"],
                &["include-root", "json"],
            )?;
            let leaf_dir = p.require("leaf", "chain")?;
            let root_dir = p.require("root", "chain")?;
            let leaf_bundle = load_bundle(&PathBuf::from(&leaf_dir))?;
            let root_bundle = load_bundle(&PathBuf::from(&root_dir))?;
            let mut pool_bundles = vec![&root_bundle];
            let mut issuer_bundles = Vec::new();
            for dir in p.all("issuer") {
                issuer_bundles.push(load_bundle(&PathBuf::from(dir))?);
            }
            for b in &issuer_bundles {
                pool_bundles.push(b);
            }
            let trust = TrustSelection::parse(p.get("trust").unwrap_or("all"))?;
            let at = p
                .get("at")
                .map(Rfc3339::parse)
                .transpose()?
                .unwrap_or_else(Rfc3339::now);
            let report = engine::validate_leaf(&leaf_bundle, &pool_bundles, trust, at)?;
            if verb == "test" {
                let mut text = format!(
                    "Chain test: {} (trust: {})\n",
                    leaf_bundle.manifest.name, report.trust_selection
                );
                if !report.path.is_empty() {
                    text.push_str("  path:\n");
                    for dn in &report.path {
                        text.push_str(&format!("    {dn}\n"));
                    }
                }
                for c in &report.checks {
                    text.push_str(&format!(
                        "  [{}] {} — {}\n",
                        if c.ok { "ok" } else { "FAIL" },
                        c.name,
                        c.detail
                    ));
                }
                text.push_str(&format!(
                    "  result: {}\n",
                    if report.valid { "VALID" } else { "INVALID" }
                ));
                let value =
                    serde_json::to_value(&report).map_err(|e| CertError::engine(e.to_string()))?;
                let code = emit(json_out, text, value);
                return Ok(if report.valid { code } else { 1 });
            }
            // build: emit the chain PEM (leaf + intermediates,
            // anchor omitted unless --include-root, §3.5).
            if !report.valid {
                return Err(CertError::validation(format!(
                    "no valid chain to build under trust selection {}: {}",
                    trust.as_str(),
                    report
                        .checks
                        .iter()
                        .filter(|c| !c.ok)
                        .map(|c| c.name.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
            let pool = engine::collect_pool(&pool_bundles)?;
            let leaf_record = leaf_bundle
                .manifest
                .certificates
                .iter()
                .find(|r| r.role == "leaf")
                .ok_or_else(|| CertError::manifest("leaf bundle has no leaf record"))?;
            let leaf_pem = crate::bundle::read_artifact(&leaf_bundle, &leaf_record.cert_path)?;
            let leaf_blocks =
                crate::bundle::pem_decode_all(std::str::from_utf8(&leaf_pem).unwrap_or(""))?;
            let leaf_der = leaf_blocks
                .into_iter()
                .find(|(l, _)| l == "CERTIFICATE")
                .map(|(_, d)| d)
                .ok_or_else(|| CertError::manifest("no CERTIFICATE block in leaf"))?;
            let leaf_info = crate::chain::parse_cert(&leaf_der)?;
            debug_assert_eq!(leaf_info.fingerprint_sha256, report.path_fingerprints[0]);
            let path = crate::chain::build_path(&leaf_info, &pool, trust)
                .ok_or_else(|| CertError::validation("path disappeared during build"))?;
            let mut pem = String::new();
            let take = if p.has("include-root") {
                path.len()
            } else {
                path.len().saturating_sub(1).max(1)
            };
            for cert in path.iter().take(take) {
                pem.push_str(&crate::bundle::pem_encode("CERTIFICATE", &cert.der));
            }
            match p.get("out") {
                Some(out_path) => {
                    std::fs::write(out_path, &pem)?;
                    Ok(emit(
                        json_out,
                        format!("Chain written to {out_path} ({} certificates)\n", take),
                        json!({"out": out_path, "certificates": take, "path": report.path}),
                    ))
                }
                None => Ok(emit(
                    json_out,
                    pem.clone(),
                    json!({"chain_pem": pem, "path": report.path}),
                )),
            }
        }
        other => Err(CertError::usage(format!(
            "unknown chain verb '{other}' (build | test)"
        ))),
    }
}

// ---------------------------------------------------------------- bundle/profile inspect+list

fn manifest_text(verb: &str, m: &crate::model::BundleManifest, dir: &str) -> String {
    let mut text = format!(
        "{verb}: {} ({}) at {dir}\n  bundle_id: {}\n  certificates: {}\n",
        m.name,
        m.bundle_type.as_str(),
        m.bundle_id,
        m.certificates.len()
    );
    for c in &m.certificates {
        text.push_str(&format!(
            "  {:10} {:10} {:40} sha256:{}.. {}\n",
            c.role,
            c.algorithm.name(),
            c.subject_dn,
            &c.fingerprint_sha256[..12],
            if c.primary { "primary" } else { "" }
        ));
    }
    for w in &m.warnings {
        text.push_str(&format!("  warning: {w}\n"));
    }
    text
}

fn manifest_json(m: &crate::model::BundleManifest, dir: &str) -> Value {
    let mut v = serde_json::to_value(m).unwrap_or_else(|_| json!({}));
    if let Some(obj) = v.as_object_mut() {
        obj.insert("dir".to_string(), json!(dir));
    }
    v
}

fn inspect_cmd(args: &[String], json_out: bool, want: Option<BundleType>) -> Result<i32> {
    let p = parse_args(args, &["bundle"], &["json"])?;
    let dir = p
        .get("bundle")
        .map(|s| s.to_string())
        .or_else(|| p.positional(0).map(|s| s.to_string()))
        .ok_or_else(|| CertError::usage("inspect needs a bundle directory"))?;
    let bundle = load_bundle(&PathBuf::from(&dir))?;
    if let Some(w) = want {
        if bundle.manifest.bundle_type != w {
            return Err(CertError::usage(format!(
                "{dir} is a {} bundle, not a {}",
                bundle.manifest.bundle_type.as_str(),
                w.as_str()
            )));
        }
    }
    let m = &bundle.manifest;
    let mut text = manifest_text("Bundle", m, &dir);
    text.push_str(&format!(
        "  organization: {} ({})\n  generation: {}\n  key location: {}\n  files: {} (all checksums verified)\n",
        m.organization,
        m.country,
        m.generation,
        m.key_location,
        m.files.len()
    ));
    if let Some(parent) = &m.parent {
        text.push_str(&format!(
            "  parent: {} {} ({})\n",
            parent.bundle_type.as_str(),
            parent.name,
            parent.bundle_id
        ));
    }
    Ok(emit(json_out, text, manifest_json(m, &dir)))
}

fn bundle_cmd(args: &[String], json_out: bool) -> Result<i32> {
    let verb = args.first().map(|s| s.as_str()).unwrap_or("");
    let rest = &args[1.min(args.len())..];
    match verb {
        "inspect" => inspect_cmd(rest, json_out, None),
        other => Err(CertError::usage(format!(
            "unknown bundle verb '{other}' (inspect; pack/unpack/validate/diff are follow-ups under #9157)"
        ))),
    }
}

// ---------------------------------------------------------------- publish (slice 1: plan + verify read-only)

fn publish_cmd(args: &[String], json_out: bool) -> Result<i32> {
    let verb = args.first().map(|s| s.as_str()).unwrap_or("");
    let rest = &args[1.min(args.len())..];
    match verb {
        "plan" => {
            let p = parse_args(
                rest,
                &["bundle", "host", "aia-base-url", "dns-target", "format", "out"],
                &["json"],
            )?;
            let dir = p
                .get("bundle")
                .map(|s| s.to_string())
                .or_else(|| p.positional(0).map(|s| s.to_string()))
                .ok_or_else(|| CertError::usage("publish plan needs a bundle directory"))?;
            // --aia-base-url overrides --host when both are given.
            let host = p
                .get("aia-base-url")
                .map(|s| s.to_string())
                .or_else(|| p.get("host").map(|s| s.to_string()))
                .unwrap_or_else(|| crate::publish::DEFAULT_PUBLISH_HOST.to_string());
            let req = crate::model::PublishPlanRequest {
                bundle: PathBuf::from(&dir),
                host,
                dns_target: p.get("dns-target").map(|s| s.to_string()),
            };
            let plan = crate::publish::plan_bundle(&req)?;
            let format = p.get("format").unwrap_or("text").to_ascii_lowercase();
            let value = serde_json::to_value(&plan)
                .map_err(|e| CertError::engine(e.to_string()))?;
            let text = crate::publish::render_plan_text(&plan);
            let out_text = match format.as_str() {
                "json" => serde_json::to_string_pretty(&value)
                    .map_err(|e| CertError::engine(e.to_string()))?,
                "yaml" | "yml" => crate::publish::render_plan_yaml(&plan),
                "text" => text.clone(),
                other => {
                    return Err(CertError::usage(format!(
                        "unknown --format '{other}' (text | json | yaml)"
                    )));
                }
            };
            if let Some(out) = p.get("out") {
                std::fs::write(out, format!("{out_text}\n"))?;
            }
            if format == "yaml" && !json_out {
                print!("{out_text}\n");
                return Ok(0);
            }
            Ok(emit(json_out, format!("{text}"), value))
        }
        "verify" => {
            let p = parse_args(rest, &["bundle", "published-dir", "host"], &["json"])?;
            let dir = p
                .get("bundle")
                .map(|s| s.to_string())
                .or_else(|| p.positional(0).map(|s| s.to_string()))
                .ok_or_else(|| CertError::usage("publish verify needs a bundle directory"))?;
            let pub_dir = p
                .get("published-dir")
                .map(|s| s.to_string())
                .or_else(|| p.positional(1).map(|s| s.to_string()))
                .ok_or_else(|| {
                    CertError::usage(
                        "publish verify needs --published-dir <mirror> (read-only local check)",
                    )
                })?;
            let req = crate::model::PublishVerifyRequest {
                bundle: PathBuf::from(&dir),
                published_dir: PathBuf::from(&pub_dir),
                host: p.get("host").map(|s| s.to_string()),
            };
            let report = crate::publish::verify_bundle(&req)?;
            let mut text = format!(
                "Publish verify: {} against {}\n",
                report.bundle, report.published_dir
            );
            for c in &report.checks {
                text.push_str(&format!(
                    "  [{}] {} — {}\n",
                    if c.ok { "ok" } else { "FAIL" },
                    c.name,
                    c.detail
                ));
            }
            text.push_str(&format!(
                "  result: {}\n",
                if report.valid { "IN SYNC" } else { "OUT OF SYNC" }
            ));
            let value = serde_json::to_value(&report)
                .map_err(|e| CertError::engine(e.to_string()))?;
            let code = emit(json_out, text, value);
            Ok(if report.valid { code } else { 1 })
        }
        "apply" | "status" | "stage" | "check" | "commit" | "rollback" | "render" => {
            Err(CertError::policy(format!(
                "publish {verb} is deferred to a later production-gated slice (per the #9155 gate map): \
                 slice 1 implements plan + verify read-only only; never executes against live endpoints without gate clearance"
            )))
        }
        other => Err(CertError::usage(format!(
            "unknown publish verb '{other}' (plan | verify; apply | status are a later gated slice)"
        ))),
    }
}

fn list_cmd(args: &[String], json_out: bool, want: BundleType) -> Result<i32> {
    let p = parse_args(args, &["dir"], &["json"])?;
    let dir = p.get("dir").unwrap_or(".").to_string();
    let found = list_bundles(&PathBuf::from(&dir), Some(want));
    let mut text = format!("{} bundles under {dir}:\n", want.as_str());
    if found.is_empty() {
        text.push_str("  (none)\n");
    }
    for (path, m) in &found {
        text.push_str(&format!(
            "  {} — {} ({}, {} certificates, created {})\n",
            path.display(),
            m.name,
            m.bundle_id,
            m.certificates.len(),
            m.created_at
        ));
    }
    let value = json!({
        "dir": dir,
        "bundle_type": want.as_str(),
        "bundles": found.iter().map(|(path, m)| json!({
            "dir": path.display().to_string(),
            "bundle_id": m.bundle_id,
            "name": m.name,
            "certificates": m.certificates.len(),
            "created_at": m.created_at.to_string_canonical(),
        })).collect::<Vec<_>>(),
    });
    Ok(emit(json_out, text, value))
}

fn profile_cmd(args: &[String], json_out: bool) -> Result<i32> {
    let verb = args.first().map(|s| s.as_str()).unwrap_or("list");
    let rest = &args[1.min(args.len())..];
    match verb {
        "list" => {
            let mut text = String::from("Leaf profiles (mirroring pkgs/idp-pki idp-ca):\n");
            for prof in profile::leaf_profiles() {
                text.push_str(&format!(
                    "  {:12} key={:8} validity={}d  {}\n",
                    prof.name,
                    prof.key_algorithm.name(),
                    prof.default_validity.days,
                    prof.notes
                ));
            }
            let value = json!({
                "profiles": profile::leaf_profiles().iter().map(|p| json!({
                    "name": p.name,
                    "key_algorithm": p.key_algorithm.name(),
                    "key_usages": p.key_usages.iter().map(|k| format!("{k:?}")).collect::<Vec<_>>(),
                    "extended_key_usages": p.extended_key_usages.iter().map(|k| format!("{k:?}")).collect::<Vec<_>>(),
                    "default_validity_days": p.default_validity.days,
                    "san_defaults_to_cn": p.san_defaults_to_cn,
                    "notes": p.notes,
                })).collect::<Vec<_>>(),
            });
            Ok(emit(json_out, text, value))
        }
        "show" => {
            let name = rest
                .first()
                .ok_or_else(|| CertError::usage("profile show needs a profile name"))?;
            let prof = profile::leaf_profile(name)?;
            let text = format!(
                "{}: key={} validity={}d san-from-cn={}\n  {}\n",
                prof.name,
                prof.key_algorithm.name(),
                prof.default_validity.days,
                prof.san_defaults_to_cn,
                prof.notes
            );
            let value = json!({
                "name": prof.name,
                "key_algorithm": prof.key_algorithm.name(),
                "default_validity_days": prof.default_validity.days,
                "san_defaults_to_cn": prof.san_defaults_to_cn,
                "notes": prof.notes,
            });
            Ok(emit(json_out, text, value))
        }
        other => Err(CertError::usage(format!(
            "unknown profile verb '{other}' (list | show)"
        ))),
    }
}

// ---------------------------------------------------------------- algorithms

fn algorithms_cmd(args: &[String], json_out: bool) -> Result<i32> {
    let verb = args.first().map(|s| s.as_str()).unwrap_or("list");
    match verb {
        "list" => {
            let reg = algorithm::registry();
            let mut text = String::from("Registered algorithms (spec §3.1/§24.1):\n");
            for info in &reg {
                text.push_str(&format!(
                    "  {:10} {:9} support={:?} policy={:?} sig-oid={}\n",
                    info.name, info.display_name, info.support, info.policy, info.signature_oid
                ));
            }
            let value = json!({
                "algorithms": reg.iter().map(|i| json!({
                    "id": i.name,
                    "display_name": i.display_name,
                    "spki_oid": i.spki_oid,
                    "signature_oid": i.signature_oid,
                    "support": format!("{:?}", i.support).to_lowercase(),
                    "policy": format!("{:?}", i.policy).to_lowercase(),
                    "post_quantum": i.post_quantum,
                    "notes": i.notes,
                })).collect::<Vec<_>>(),
            });
            Ok(emit(json_out, text, value))
        }
        "describe" => {
            let name = args
                .get(1)
                .ok_or_else(|| CertError::usage("algorithms describe needs an algorithm name"))?;
            let (algo, warn) = AlgorithmId::parse(name)?;
            let info = algo.info();
            let mut text = format!(
                "{} ({})\n  spki oid: {}\n  signature oid: {}\n  support: {:?}\n  policy: {:?}\n  post-quantum: {}\n  {}\n",
                info.name,
                info.display_name,
                info.spki_oid,
                info.signature_oid,
                info.support,
                info.policy,
                info.post_quantum,
                info.notes
            );
            if let Some(w) = &warn {
                text.push_str(&format!("  warning: {w}\n"));
            }
            let value = json!({
                "id": info.name,
                "display_name": info.display_name,
                "spki_oid": info.spki_oid,
                "signature_oid": info.signature_oid,
                "support": format!("{:?}", info.support).to_lowercase(),
                "policy": format!("{:?}", info.policy).to_lowercase(),
                "post_quantum": info.post_quantum,
                "notes": info.notes,
                "warning": warn,
            });
            Ok(emit(json_out, text, value))
        }
        other => Err(CertError::usage(format!(
            "unknown algorithms verb '{other}' (list | describe)"
        ))),
    }
}

// ---------------------------------------------------------------- explain / doctor

const NOT_BUILT: &[(&str, &str)] = &[
    (
        "revoke / crl",
        "revocation + signed CRLs (spec §13 phase F)",
    ),
    (
        "ocsp",
        "OCSP — only behind a functional responder (spec §4.3)",
    ),
    (
        "publish apply/status",
        "live AIA/CDP publication apply + status (production-gated follow-up; slice 1 ships plan + verify read-only)",
    ),
    ("dns", "DNS plan/render/verify (spec §24.7)"),
    (
        "trust",
        "trust-store install/export (spec §3.5 follow-through)",
    ),
    (
        "key",
        "key inspect/import/export/convert-public (spec §6.1)",
    ),
    ("format", "format list/describe/export/convert (spec §7.5)"),
    (
        "profile validate/scaffold",
        "profile validation + scaffolding (spec §6.1)",
    ),
    (
        "algorithms benchmark/doctor",
        "algorithm benchmarks (spec §6.1)",
    ),
    ("batch", "batch plan/validate/apply (spec §6.5)"),
    ("inventory", "inventory list/show/search/audit (spec §6.1)"),
    (
        "root/issuer renew/rotate/replace",
        "lifecycle verbs (spec §5.2)",
    ),
    (
        "bundle pack/unpack/diff/migrate",
        "bundle packaging verbs (spec §7)",
    ),
];

fn explain_cmd(args: &[String], json_out: bool) -> Result<i32> {
    let topic = args.first().map(|s| s.as_str()).unwrap_or("overview");
    let body = match topic {
        "overview" => "fleet cert implements the §6.1 command tree's core: root, issuer, leaf, \
chain, bundle, profile, algorithms, explain, doctor. Signing is library-first (§6.4): \
the CLI is a thin shell over the fleet-cert crate. Cross-signing is policy data \
(§§3.2/24.2), time is Gregorian calendar arithmetic (§5.4), and ML-DSA-44 is a \
first-class algorithm with RFC 9881 identifiers. Topics: signer-source, \
cross-signing, pq-policy, time, profiles."
            .to_string(),
        "signer-source" => "Spec §24.3: a root private key is used only when the operator \
explicitly provides an authorized root source. In this CLI that is --root-key-dir \
(for issuer create) and --issuer-key-dir (for leaf issue): directories holding the \
signer's PKCS#8 keys. There is no default lookup, no config-file discovery, and no \
environment fallback — a missing flag is a policy error, not a prompt. Root creation \
is a ceremony the other way: it must name its key destination (--private-store or \
--private-in-bundle) before any key is generated."
            .to_string(),
        "cross-signing" => "The cross-sign graph is data (crate::graph::GraphPolicy), never a \
hardcoded loop. Root level (§3.2): classical roots cross-certify each other's keys and \
the ML-DSA key; the ML-DSA root certifies nothing classical. Issuer level (§24.2): \
issuer keys are directly cross-signed under every eligible root with the same \
asymmetry. --cross-signing selects none | roots-only | issuers-direct | both."
            .to_string(),
        "pq-policy" => "Post-quantum assurance is a verifier policy (§3.2), not an issuance \
property. `chain test --trust pq` trusts only the ML-DSA root AND requires every key \
and signature in the path to be ML-DSA-44; a classical cross-certificate over the \
ML-DSA key does not create a PQ path for a classical leaf. `roots-all` trust gives \
compatibility, not PQ assurance."
            .to_string(),
        "time" => "Lifetimes are Gregorian calendar arithmetic (§5.4): 20y is twenty \
calendar years from the start date (Feb 29 + 1y = Feb 28), never a fixed second \
count. Defaults: root 20y, issuer 5y, leaf 90d unprofiled (idp-ca profiles carry \
their own defaults). --not-after takes an exact RFC3339 instant and is mutually \
exclusive with --valid-for. Every bundle manifest records the resolved RFC3339 \
window."
            .to_string(),
        "profiles" => "Leaf profiles mirror pkgs/idp-pki's idp-ca: server (P-256, \
serverAuth, 730d, SAN defaults to DNS:<cn>), client (P-256, clientAuth, 730d), \
saml-signing and oidc-signing (RSA-2048, digitalSignature + nonRepudiation, 1095d, \
no EKU). See `fleet cert profile list`."
            .to_string(),
        other => {
            return Err(CertError::usage(format!(
                "unknown explain topic '{other}' (overview | signer-source | cross-signing | pq-policy | time | profiles)"
            )));
        }
    };
    Ok(emit(
        json_out,
        format!("{body}\n"),
        json!({"topic": topic, "explanation": body}),
    ))
}

fn doctor_cmd(_args: &[String], json_out: bool) -> Result<i32> {
    // Live engine self-test per supported algorithm: generate,
    // self-sign, verify — the honest form of "supported".
    let mut algo_results = Vec::new();
    let mut all_ok = true;
    for info in algorithm::registry() {
        let ok = engine_self_test(info.id);
        if !ok {
            all_ok = false;
        }
        algo_results.push((info, ok));
    }
    let openssl = std::process::Command::new("openssl")
        .arg("version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    let mut text =
        String::from("fleet cert doctor\n  engine self-test (generate + self-sign + verify):\n");
    for (info, ok) in &algo_results {
        text.push_str(&format!(
            "    {:10} {}\n",
            info.name,
            if *ok { "ok" } else { "FAIL" }
        ));
    }
    text.push_str(&format!(
        "  openssl (independent validator): {}\n",
        openssl.as_deref().unwrap_or("not found")
    ));
    text.push_str("  built: root, issuer, leaf, chain, bundle (inspect), publish (plan/verify read-only), profile (list/show), algorithms (list/describe), explain, doctor\n");
    text.push_str("  not built yet (tracked as child issues under #9157):\n");
    for (name, what) in NOT_BUILT {
        text.push_str(&format!("    {name} — {what}\n"));
    }
    let value = json!({
        "engine_self_test": algo_results.iter().map(|(i, ok)| json!({"algorithm": i.name, "ok": ok})).collect::<Vec<_>>(),
        "openssl": openssl,
        "built": ["root", "issuer", "leaf", "chain", "bundle-inspect", "publish-plan/verify", "profile-list/show", "algorithms-list/describe", "explain", "doctor"],
        "not_built": NOT_BUILT.iter().map(|(n, w)| json!({"surface": n, "scope": w})).collect::<Vec<_>>(),
    });
    let code = emit(json_out, text, value);
    Ok(if all_ok { code } else { 1 })
}

fn engine_self_test(algo: AlgorithmId) -> bool {
    let Ok(key) = engine::generate_key(algo) else {
        return false;
    };
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("empty SAN list");
    params
        .distinguished_name
        .push(DnType::CommonName, "fleet cert doctor self-test");
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let now = Rfc3339::now();
    params.not_before = now.0;
    params.not_after = (now.0 + time::Duration::days(1));
    let Ok(cert) = params.self_signed(&key) else {
        return false;
    };
    match crate::chain::parse_cert(cert.der()) {
        Ok(info) => crate::chain::verify_signature(&info, &info),
        Err(_) => false,
    }
}
