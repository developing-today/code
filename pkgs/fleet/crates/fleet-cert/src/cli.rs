//! The `fleet cert` CLI: a thin shell over the crate API (spec
//! §6.4). Each verb parses flags into a typed request in
//! [`crate::model`], calls [`crate::engine`], and renders text or,
//! with `--json`, one JSON value on stdout.
//!
//! Exit codes follow the fleet contract: 0 ok · 2 usage · 1
//! operational/policy/validation failure.

use crate::algorithm::{self, AlgorithmId};
use crate::algorithm_lab;
use crate::bundle::{list_bundles, load_bundle, pem_decode_all, read_artifact};
use crate::chain::{CertInfo, TrustSelection, parse_cert};
use crate::engine;
use crate::error::{CertError, Result};
use crate::graph::CrossSigningMode;
use crate::model::{
    BundleType, CertRecord, IssuerCreateRequest, IssuerPurpose, LeafIssueRequest,
    RootCreateRequest, SignerSource,
};
use crate::profile;
use crate::timeparse::{CalendarDuration, Rfc3339};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Full usage for the cert domain (also what `fleet cert --help`
/// prints through the bin crate's help carve-out).
pub const CERT_USAGE: &str = "\
Usage: fleet cert <group> <verb> [options]
       fleet pki  <group> <verb> [options]   (alias)

Groups and verbs (spec §6.1 tree; status as built):
  root     create | renew | rotate | replace | inspect | list | validate
  issuer   create | renew | rotate | replace | inspect | list
  leaf     issue | renew | rotate | inspect | list
  ocsp     sign | inspect | test
  revoke   leaf | issuer | root
  crl      generate | inspect | verify
  chain    build | test
  bundle   inspect
  format   convert | verify
  inventory list | show | search | audit
  key      generate | inspect
  publish  plan | apply --bundle <dir> --staging-dir <dir> | verify (status deferred, production-gated)
  dns      plan | render | verify (offline CAA + TLSA/DANE change-sets)
  batch    validate | plan | apply | status (reviewed, all-or-nothing)
  trust    export | stage | ack | activate | retire | status | inspect | pin | verify | install-plan
  profile  list | show | validate | scaffold
  algorithms list | describe | benchmark | doctor
  explain  [topic]
  doctor

Format conversion:
  Certificates retain the original PEM/DER behavior. Private keys use
  --algorithm <id>; conversion requires --out and creates an owner-only file.
  PKCS#12 identity archives (encrypted, password via --password-file):
    export: convert --to pkcs12 --bundle <bundle-with-one-identity> --password-file <f> --out <archive.p12>
    import: convert --from pkcs12 --in <archive.p12> --password-file <f> --out <new-dir>
            (writes certificate.der, private-key.pk8 owner-only, chain-NN.der)
    verify: verify --in <archive.p12> --password-file <f> (redacts key material)
  Public JWK (Ed25519 and P-256 only; RSA and ML-DSA are unsupported):
    export: convert --to jwk --algorithm <id> [--in <key.pk8> | --bundle <dir>] [--out <key.jwk>]
    import: convert --from jwk --to pem|der --in <key.jwk> [--out <key.pub>]
    verify: verify --in <key.jwk> --algorithm <id> --key <key.pk8> (proves the JWK matches the DER key)

Common options:
  --json                  Emit one JSON value instead of text.
  --not-before <rfc3339>  Validity start (default: now).
  --not-after <rfc3339>   Exact validity end (mutually exclusive with --valid-for).
  --valid-for <duration>  Lifetime from not-before: 20y, 90d, '15 years 7 days'.

Trust over a root set (documentation/output only; nothing installs trust):
  trust inspect --root <root-set-dir> [--trust all|classical|pq]
  trust pin --root <root-set-dir> [--trust ...] --out <pinset.json>
  trust verify --root <root-set-dir> [--trust ...] --pins <pinset.json> [--store <pem-or-dir>]
  trust export --root <root-set-dir> --target <consumer> [--trust ...] --out <dir>
  trust install-plan --target <consumer>
See `fleet cert explain trust` for the safety model.

Signing operations never discover keys ambiently: issuer/leaf
creation requires the signer's key directory as an explicit
argument (spec §24.3). Run `fleet cert explain signer-source`
for the rule, `fleet cert doctor` for what is built vs not.

CRLs:
  crl generate --issuer <bundle> --issuer-key-dir <dir> --crl-number <n> --out <file>
      [--issuer-algorithm <fingerprint|algorithm|subject-DN>] [--revocations <bundle>]...
      [--at <rfc3339>] [--next-update <rfc3339>] [--previous-crl <file>] [--pem]
  crl inspect <file> [--json]
  crl verify --crl <file> --issuer <bundle> [--revocations <bundle>]... [--at <rfc3339>]
  --previous-crl makes generation reject a CRL number that is not greater than its predecessor.
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
        "ocsp" => ocsp_cmd(rest, json_out),
        "revoke" => revoke_cmd(rest, json_out),
        "crl" => crl_cmd(rest, json_out),
        "chain" => chain_cmd(rest, json_out),
        "bundle" => bundle_cmd(rest, json_out),
        "format" => format_cmd(rest, json_out),
        "inventory" => inventory_cmd(rest, json_out),
        "key" => key_cmd(rest, json_out),
        "publish" => publish_cmd(rest, json_out),
        "dns" => dns_cmd(rest, json_out),
        "batch" => batch_cmd(rest, json_out),
        "trust" => trust_cmd(rest, json_out),
        "profile" => profile_cmd(rest, json_out),
        "algorithms" => algorithms_cmd(rest, json_out),
        "explain" => explain_cmd(rest, json_out),
        "doctor" => doctor_cmd(rest, json_out),
        other => Err(CertError::usage(format!(
            "unknown cert group '{other}' — run 'fleet cert --help' for the surface"
        ))),
    }
}

// ---------------------------------------------------------------- inventory (§6.1 read-only)

fn inventory_scan_args(p: &Parsed) -> Result<crate::inventory::InventoryRequest> {
    let dir = p
        .get("dir")
        .map(|s| s.to_string())
        .or_else(|| p.positional(0).map(|s| s.to_string()))
        .unwrap_or_else(|| ".".to_string());
    let at = p
        .get("at")
        .map(Rfc3339::parse)
        .transpose()?
        .unwrap_or_else(Rfc3339::now);
    let ca_within = p
        .get("ca-within")
        .map(CalendarDuration::parse)
        .transpose()?
        .unwrap_or(crate::inventory::DEFAULT_CA_WITHIN);
    let leaf_within = p
        .get("leaf-within")
        .map(CalendarDuration::parse)
        .transpose()?
        .unwrap_or(crate::inventory::DEFAULT_LEAF_WITHIN);
    Ok(crate::inventory::InventoryRequest {
        dir: PathBuf::from(dir),
        at,
        ca_within,
        leaf_within,
    })
}

fn inventory_cmd(args: &[String], json_out: bool) -> Result<i32> {
    let verb = args.first().map(|s| s.as_str()).unwrap_or("");
    let rest = &args[1.min(args.len())..];
    match verb {
        "list" => {
            let p = parse_args(rest, &["dir", "at", "ca-within", "leaf-within"], &["json"])?;
            let req = inventory_scan_args(&p)?;
            let report = crate::inventory::audit(&req)?;
            let (text, value) = crate::inventory::render_list(&report);
            Ok(emit(json_out, text, value))
        }
        "show" => {
            let p = parse_args(
                rest,
                &["bundle", "dir", "at", "ca-within", "leaf-within"],
                &["json"],
            )?;
            let bundle_dir = p
                .get("bundle")
                .map(|s| s.to_string())
                .or_else(|| p.positional(0).map(|s| s.to_string()))
                .ok_or_else(|| CertError::usage("inventory show needs a bundle directory"))?;
            let mut req = inventory_scan_args(&p)?;
            // `list_bundles` considers the scan root itself when it is
            // a bundle, so pointing the scan at the bundle directory
            // audits exactly that bundle — including parseable-but-
            // unverifiable ones.
            req.dir = PathBuf::from(&bundle_dir);
            let report = crate::inventory::audit(&req)?;
            let at = report.at;
            let found = report.bundles.into_iter().next().ok_or_else(|| {
                CertError::usage(format!("{bundle_dir} is not a bundle directory"))
            })?;
            let (text, value) = crate::inventory::render_show(&found, at);
            Ok(emit(json_out, text, value))
        }
        "search" => {
            let p = parse_args(
                rest,
                &[
                    "dir",
                    "at",
                    "ca-within",
                    "leaf-within",
                    "subject",
                    "fingerprint",
                    "role",
                    "algorithm",
                ],
                &["expired", "json"],
            )?;
            let req = inventory_scan_args(&p)?;
            let filter = crate::inventory::SearchFilter {
                subject: p.get("subject").map(|s| s.to_string()),
                fingerprint: p.get("fingerprint").map(|s| s.to_string()),
                role: p.get("role").map(|s| s.to_string()),
                algorithm: p.get("algorithm").map(|s| s.to_string()),
                expired: p.has("expired"),
            };
            let hits = crate::inventory::search(
                &req.dir,
                req.at,
                req.ca_within,
                req.leaf_within,
                &filter,
            )?;
            let (text, value) = crate::inventory::render_search(&hits, req.at);
            Ok(emit(json_out, text, value))
        }
        "audit" => {
            let p = parse_args(rest, &["dir", "at", "ca-within", "leaf-within"], &["json"])?;
            let req = inventory_scan_args(&p)?;
            let report = crate::inventory::audit(&req)?;
            let failed = report.totals.expired > 0 || report.totals.drift_findings > 0;
            let (text, value) = crate::inventory::render_audit(&report);
            let code = emit(json_out, text, value);
            Ok(if failed { 1 } else { code })
        }
        other => Err(CertError::usage(format!(
            "unknown inventory verb '{other}' (list | show | search | audit)"
        ))),
    }
}

// ---------------------------------------------------------------- standalone key ceremony

fn key_cmd(args: &[String], json_out: bool) -> Result<i32> {
    let verb = args.first().map(|s| s.as_str()).unwrap_or("");
    let rest = &args[1.min(args.len())..];
    match verb {
        "generate" => {
            let p = parse_args(rest, &["algorithm", "out"], &["json"])?;
            let algorithm = parse_key_algorithm(&p)?;
            let out = PathBuf::from(p.require("out", "key generate")?);
            let key = engine::generate_key(algorithm)?;
            let der = key.serialize_der();
            // Key generation is an explicit ceremony: create a new
            // owner-readable file, and never replace an existing key.
            write_new_file(&out, &der, true, "private key")?;
            let value = json!({"algorithm": algorithm.info().name, "private_key": out.display().to_string(), "encoding": "PKCS#8 DER"});
            Ok(emit(
                json_out,
                format!(
                    "Generated {} private key at {} (PKCS#8 DER)\n",
                    algorithm.info().name,
                    out.display()
                ),
                value,
            ))
        }
        "inspect" => {
            let p = parse_args(rest, &["algorithm", "in"], &["json"])?;
            let algorithm = parse_key_algorithm(&p)?;
            let path = PathBuf::from(p.require("in", "key inspect")?);
            let pkcs8 = std::fs::read(&path).map_err(|e| {
                CertError::policy(format!("cannot read private key {}: {e}", path.display()))
            })?;
            let public = engine::inspect_key(algorithm, &pkcs8)?;
            let fingerprint = crate::bundle::sha256_hex(&public);
            let value = json!({
                "algorithm": algorithm.info().name,
                "private_key": path.display().to_string(),
                "encoding": "PKCS#8 DER",
                "public_key_fingerprint_sha256": fingerprint,
                "public_key_bytes": public.len(),
                "private_material_emitted": false,
            });
            Ok(emit(
                json_out,
                format!(
                    "{} private key\n  encoding: PKCS#8 DER\n  public-key SHA-256: {fingerprint}\n  public-key bytes: {}\n  private material: redacted\n",
                    algorithm.info().name,
                    public.len()
                ),
                value,
            ))
        }
        "import" => {
            let p = parse_args(rest, &["algorithm", "in", "out"], &["json"])?;
            let algorithm = parse_key_algorithm(&p)?;
            let input = PathBuf::from(p.require("in", "key import")?);
            let out = PathBuf::from(p.require("out", "key import")?);
            let raw = std::fs::read(&input).map_err(|e| {
                CertError::policy(format!(
                    "cannot read key to import {}: {e}",
                    input.display()
                ))
            })?;
            let pkcs8 = pkcs8_from_der_or_pem(&raw, &input)?;
            // The key must load under the explicitly named algorithm
            // before anything is written: a wrong algorithm or a
            // non-PKCS#8 file is refused, and nothing is created.
            let public = engine::inspect_key(algorithm, &pkcs8)?;
            let fingerprint = crate::bundle::sha256_hex(&public);
            write_new_file(&out, &pkcs8, true, "private key")?;
            let value = json!({
                "algorithm": algorithm.info().name,
                "source": input.display().to_string(),
                "private_key": out.display().to_string(),
                "encoding": "PKCS#8 DER",
                "public_key_fingerprint_sha256": fingerprint,
                "private_material_emitted": false,
            });
            Ok(emit(
                json_out,
                format!(
                    "Imported {} private key into {} (PKCS#8 DER)\n  public-key SHA-256: {fingerprint}\n",
                    algorithm.info().name,
                    out.display()
                ),
                value,
            ))
        }
        "export" => {
            let p = parse_args(rest, &["algorithm", "in", "out", "format"], &["json"])?;
            let algorithm = parse_key_algorithm(&p)?;
            let input = PathBuf::from(p.require("in", "key export")?);
            let out = PathBuf::from(p.require("out", "key export")?);
            let pem = match p.get("format").unwrap_or("pem") {
                "pem" => true,
                "der" => false,
                other => {
                    return Err(CertError::usage(format!(
                        "unknown key export format '{other}' (pem | der)"
                    )));
                }
            };
            let pkcs8 = std::fs::read(&input).map_err(|e| {
                CertError::policy(format!("cannot read private key {}: {e}", input.display()))
            })?;
            // Export is the public half only (SubjectPublicKeyInfo).
            // Private-key export and escrow stay behind the custody
            // gates of #9155; nothing here writes private material.
            let public = engine::inspect_key(algorithm, &pkcs8)?;
            let spki = engine::public_key_spki_der(algorithm, &public);
            let fingerprint = crate::bundle::sha256_hex(&public);
            let bytes = if pem {
                crate::bundle::pem_encode("PUBLIC KEY", &spki).into_bytes()
            } else {
                spki
            };
            write_new_file(&out, &bytes, false, "public key")?;
            let encoding = if pem {
                "SubjectPublicKeyInfo PEM"
            } else {
                "SubjectPublicKeyInfo DER"
            };
            let value = json!({
                "algorithm": algorithm.info().name,
                "private_key": input.display().to_string(),
                "public_key": out.display().to_string(),
                "encoding": encoding,
                "public_key_fingerprint_sha256": fingerprint,
                "private_material_emitted": false,
            });
            Ok(emit(
                json_out,
                format!(
                    "Exported {} public key to {} ({encoding})\n  public-key SHA-256: {fingerprint}\n  private material: not exported\n",
                    algorithm.info().name,
                    out.display()
                ),
                value,
            ))
        }
        "rotate" => {
            let p = parse_args(rest, &["algorithm", "in", "out"], &["json"])?;
            let algorithm = parse_key_algorithm(&p)?;
            let input = PathBuf::from(p.require("in", "key rotate")?);
            let out = PathBuf::from(p.require("out", "key rotate")?);
            let old_pkcs8 = std::fs::read(&input).map_err(|e| {
                CertError::policy(format!(
                    "cannot read private key to rotate {}: {e}",
                    input.display()
                ))
            })?;
            // Rotation (§6.1, V3-S19) is a new key generation, never a
            // reuse: the old key must load under the named algorithm,
            // the successor is a fresh key of that algorithm in a new
            // owner-only file, and the old file is left untouched.
            // Retiring the old key is a separate, explicit step.
            let old_public = engine::inspect_key(algorithm, &old_pkcs8)?;
            let key = engine::generate_key(algorithm)?;
            let der = key.serialize_der();
            let new_public = engine::inspect_key(algorithm, &der)?;
            if new_public == old_public {
                return Err(CertError::engine(
                    "rotation produced the same public key as the old one; nothing written",
                ));
            }
            write_new_file(&out, &der, true, "private key")?;
            let old_fingerprint = crate::bundle::sha256_hex(&old_public);
            let new_fingerprint = crate::bundle::sha256_hex(&new_public);
            let value = json!({
                "algorithm": algorithm.info().name,
                "previous_private_key": input.display().to_string(),
                "previous_public_key_fingerprint_sha256": old_fingerprint,
                "private_key": out.display().to_string(),
                "encoding": "PKCS#8 DER",
                "public_key_fingerprint_sha256": new_fingerprint,
                "previous_key_retained": true,
                "private_material_emitted": false,
            });
            Ok(emit(
                json_out,
                format!(
                    "Rotated {} key: new private key at {} (PKCS#8 DER)\n  previous public-key SHA-256: {old_fingerprint}\n  new public-key SHA-256: {new_fingerprint}\n  previous key: kept at {} (retire it separately)\n",
                    algorithm.info().name,
                    out.display(),
                    input.display()
                ),
                value,
            ))
        }
        other => Err(CertError::usage(format!(
            "unknown key verb '{other}' (generate | inspect | import | export | rotate)"
        ))),
    }
}

/// Write `bytes` to a new file at `path`, refusing to replace anything
/// already there. A private file is created owner-only (0600 on Unix).
/// A failed write removes the partial file.
fn write_new_file(path: &Path, bytes: &[u8], private: bool, what: &str) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        if private {
            options.mode(0o600);
        }
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut file = options.open(path).map_err(|e| {
        CertError::policy(format!(
            "cannot create {what} {} without replacing an existing file: {e}",
            path.display()
        ))
    })?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| {
            let _ = std::fs::remove_file(path);
            CertError::engine(format!("writing {what} {} failed: {e}", path.display()))
        })
}

/// The PKCS#8 DER inside a key file: the bytes as they are, or the one
/// `PRIVATE KEY` block of a PEM file. Encrypted PKCS#8 and legacy
/// (`RSA PRIVATE KEY`, `EC PRIVATE KEY`) blocks are refused by name.
fn pkcs8_from_der_or_pem(raw: &[u8], path: &Path) -> Result<Vec<u8>> {
    let Ok(text) = std::str::from_utf8(raw) else {
        return Ok(raw.to_vec());
    };
    if !text.contains("-----BEGIN ") {
        return Ok(raw.to_vec());
    }
    let blocks = pem_decode_all(text)?;
    let mut keys = blocks
        .into_iter()
        .filter(|(label, _)| label.ends_with("PRIVATE KEY"));
    match (keys.next(), keys.next()) {
        (Some((label, der)), None) if label == "PRIVATE KEY" => Ok(der),
        (Some((label, _)), None) => Err(CertError::policy(format!(
            "{} holds a '{label}' block; key import takes an unencrypted PKCS#8 'PRIVATE KEY'",
            path.display()
        ))),
        (None, _) => Err(CertError::policy(format!(
            "{} holds no PEM 'PRIVATE KEY' block",
            path.display()
        ))),
        (Some(_), Some(_)) => Err(CertError::policy(format!(
            "{} holds more than one private key; import one key per file",
            path.display()
        ))),
    }
}

fn parse_key_algorithm(p: &Parsed) -> Result<AlgorithmId> {
    let name = p.require("algorithm", "key operation")?;
    parse_algorithm_name(&name)
}

fn parse_algorithm_name(name: &str) -> Result<AlgorithmId> {
    AlgorithmId::parse(name).map(|(algorithm, warning)| {
        if let Some(warning) = warning {
            eprintln!("warning: {warning}");
        }
        algorithm
    })
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
        "renew" => {
            let mut flags = vec!["root", "root-key-dir", "out"];
            flags.extend(TIME_FLAGS);
            let p = parse_args(rest, &flags, &["allow-key-reuse", "json"])?;
            let root = p
                .get("root")
                .map(|s| s.to_string())
                .or_else(|| p.positional(0).map(|s| s.to_string()))
                .ok_or_else(|| CertError::usage("root renew requires --root <root-dir>"))?;
            let signer = signer_source(&p, "root-key-dir", "root renew")?;
            let (nb, na) = validity(
                &p,
                CalendarDuration::years(crate::timeparse::ROOT_DEFAULT_LIFETIME_YEARS),
            )?;
            let req = crate::model::RootRenewRequest {
                root_bundle: PathBuf::from(&root),
                root_signer: signer,
                not_before: nb,
                not_after: na,
                allow_key_reuse: p.has("allow-key-reuse"),
            };
            let out = p.require("out", "root renew")?;
            let manifest = engine::renew_root_set(&req, &PathBuf::from(&out))?;
            Ok(emit(
                json_out,
                manifest_text("Root set renewed", &manifest, &out),
                manifest_json(&manifest, &out),
            ))
        }
        "rotate" => {
            let mut flags = vec!["root", "out", "private-store"];
            flags.extend(TIME_FLAGS);
            let p = parse_args(
                rest,
                &flags,
                &["allow-legacy-root-strength", "private-in-bundle", "json"],
            )?;
            let root = p
                .get("root")
                .map(|s| s.to_string())
                .or_else(|| p.positional(0).map(|s| s.to_string()))
                .ok_or_else(|| CertError::usage("root rotate requires --root <root-dir>"))?;
            // §24.3 ceremony: the new keys' destination must be explicit.
            let private_store = match (p.get("private-store"), p.has("private-in-bundle")) {
                (Some(dir), _) => Some(PathBuf::from(dir)),
                (None, true) => None,
                (None, false) => {
                    return Err(CertError::policy(
                        "root rotation is a ceremony: name the new root key destination \
                         explicitly — --private-store <dir> or --private-in-bundle \
                         (spec §24.3)",
                    ));
                }
            };
            let (nb, na) = validity(
                &p,
                CalendarDuration::years(crate::timeparse::ROOT_DEFAULT_LIFETIME_YEARS),
            )?;
            let req = crate::model::RootRotateRequest {
                root_bundle: PathBuf::from(&root),
                not_before: nb,
                not_after: na,
                allow_legacy_root_strength: p.has("allow-legacy-root-strength"),
                private_store,
            };
            let out = p.require("out", "root rotate")?;
            let manifest = engine::rotate_root_set(&req, &PathBuf::from(&out))?;
            Ok(emit(
                json_out,
                manifest_text("Root set rotated", &manifest, &out),
                manifest_json(&manifest, &out),
            ))
        }
        "replace" => replace_cmd(rest, json_out, true),
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
            "unknown root verb '{other}' (create | renew | rotate | replace | inspect | list | validate)"
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
        "renew" => {
            let mut flags = vec!["issuer", "root", "root-key-dir", "out"];
            flags.extend(TIME_FLAGS);
            let p = parse_args(rest, &flags, &["allow-key-reuse", "json"])?;
            let issuer = p
                .get("issuer")
                .map(|s| s.to_string())
                .or_else(|| p.positional(0).map(|s| s.to_string()))
                .ok_or_else(|| {
                    CertError::usage("issuer renew requires --issuer <issuer-set-dir>")
                })?;
            let root = p.require("root", "issuer renew")?;
            let signer = signer_source(&p, "root-key-dir", "issuer renew")?;
            let (nb, na) = validity(
                &p,
                CalendarDuration::years(crate::timeparse::ISSUER_DEFAULT_LIFETIME_YEARS),
            )?;
            let req = crate::model::IssuerRenewRequest {
                issuer_bundle: PathBuf::from(&issuer),
                root_bundle: PathBuf::from(&root),
                root_signer: signer,
                not_before: nb,
                not_after: na,
                allow_key_reuse: p.has("allow-key-reuse"),
            };
            let out = p.require("out", "issuer renew")?;
            let manifest = engine::renew_issuer_set(&req, &PathBuf::from(&out))?;
            Ok(emit(
                json_out,
                manifest_text("Issuer set renewed", &manifest, &out),
                manifest_json(&manifest, &out),
            ))
        }
        "rotate" => {
            let mut flags = vec![
                "issuer",
                "root",
                "root-key-dir",
                "out",
                "private-store",
                "overlap-root",
                "overlap-root-key-dir",
            ];
            flags.extend(TIME_FLAGS);
            let p = parse_args(rest, &flags, &["allow-legacy-root-strength", "json"])?;
            let issuer = p
                .get("issuer")
                .map(|s| s.to_string())
                .or_else(|| p.positional(0).map(|s| s.to_string()))
                .ok_or_else(|| {
                    CertError::usage("issuer rotate requires --issuer <issuer-set-dir>")
                })?;
            let root = p.require("root", "issuer rotate")?;
            let signer = signer_source(&p, "root-key-dir", "issuer rotate")?;
            let (nb, na) = validity(
                &p,
                CalendarDuration::years(crate::timeparse::ISSUER_DEFAULT_LIFETIME_YEARS),
            )?;
            let req = crate::model::IssuerRotateRequest {
                issuer_bundle: PathBuf::from(&issuer),
                root_bundle: PathBuf::from(&root),
                root_signer: signer,
                not_before: nb,
                not_after: na,
                allow_legacy_root_strength: p.has("allow-legacy-root-strength"),
                private_store: p.get("private-store").map(PathBuf::from),
                overlap_root_bundle: p.get("overlap-root").map(PathBuf::from),
                overlap_root_signer: p.get("overlap-root-key-dir").map(|dir| SignerSource {
                    key_dir: PathBuf::from(dir),
                }),
            };
            let out = p.require("out", "issuer rotate")?;
            let manifest = engine::rotate_issuer_set(&req, &PathBuf::from(&out))?;
            Ok(emit(
                json_out,
                manifest_text("Issuer set rotated", &manifest, &out),
                manifest_json(&manifest, &out),
            ))
        }
        "replace" => replace_cmd(rest, json_out, false),
        "inspect" => inspect_cmd(rest, json_out, Some(BundleType::IssuerSet)),
        "list" => list_cmd(rest, json_out, BundleType::IssuerSet),
        other => Err(CertError::usage(format!(
            "unknown issuer verb '{other}' (create | renew | rotate | replace | inspect | list)"
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
                "ip",
                "csr",
                "algorithm",
                "issuer-algorithm",
                "issuer-partition",
                "private-store",
            ];
            flags.extend(TIME_FLAGS);
            let p = parse_args(rest, &flags, &["json"])?;
            let issuer = p.require("issuer", "leaf issue")?;
            let ip_addresses = p
                .all("ip")
                .iter()
                .map(|s| {
                    s.trim()
                        .parse::<std::net::IpAddr>()
                        .map_err(|_| CertError::usage(format!("--ip '{s}' is not an IP address")))
                })
                .collect::<Result<Vec<_>>>()?;
            let signer = signer_source(&p, "issuer-key-dir", "leaf issue")?;
            let profile_name = p.require("profile", "leaf issue")?;
            let prof = profile::resolve_profile(&profile_name)?;
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
                ip_addresses,
                csr: p.get("csr").map(PathBuf::from),
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
        "renew" => {
            let mut flags = vec!["leaf", "issuer", "issuer-key-dir", "out"];
            flags.extend(TIME_FLAGS);
            let p = parse_args(rest, &flags, &["allow-key-reuse", "json"])?;
            let leaf = p
                .get("leaf")
                .map(|s| s.to_string())
                .or_else(|| p.positional(0).map(|s| s.to_string()))
                .ok_or_else(|| CertError::usage("leaf renew requires --leaf <leaf-dir>"))?;
            let issuer = p.require("issuer", "leaf renew")?;
            let signer = signer_source(&p, "issuer-key-dir", "leaf renew")?;
            // The default lifetime is the leaf's own profile's.
            let old = load_bundle(&PathBuf::from(&leaf))?;
            let prof =
                profile::resolve_profile(old.manifest.purpose.as_deref().ok_or_else(|| {
                    CertError::manifest("the leaf bundle records no profile to renew under")
                })?)?;
            let (nb, na) = validity(&p, prof.default_validity)?;
            let req = crate::model::LeafRenewRequest {
                leaf_bundle: PathBuf::from(&leaf),
                issuer_bundle: PathBuf::from(&issuer),
                issuer_signer: signer,
                not_before: nb,
                not_after: na,
                allow_key_reuse: p.has("allow-key-reuse"),
            };
            let out = p.require("out", "leaf renew")?;
            let manifest = engine::renew_leaf(&req, &PathBuf::from(&out))?;
            Ok(emit(
                json_out,
                manifest_text("Leaf renewed", &manifest, &out),
                manifest_json(&manifest, &out),
            ))
        }
        "rotate" => {
            let mut flags = vec![
                "leaf",
                "issuer",
                "issuer-key-dir",
                "out",
                "csr",
                "private-store",
            ];
            flags.extend(TIME_FLAGS);
            let p = parse_args(rest, &flags, &["json"])?;
            let leaf = p
                .get("leaf")
                .map(|s| s.to_string())
                .or_else(|| p.positional(0).map(|s| s.to_string()))
                .ok_or_else(|| CertError::usage("leaf rotate requires --leaf <leaf-dir>"))?;
            let issuer = p.require("issuer", "leaf rotate")?;
            let signer = signer_source(&p, "issuer-key-dir", "leaf rotate")?;
            let old = load_bundle(&PathBuf::from(&leaf))?;
            let prof =
                profile::resolve_profile(old.manifest.purpose.as_deref().ok_or_else(|| {
                    CertError::manifest("the leaf bundle records no profile to rotate under")
                })?)?;
            let (nb, na) = validity(&p, prof.default_validity)?;
            let req = crate::model::LeafRotateRequest {
                leaf_bundle: PathBuf::from(&leaf),
                issuer_bundle: PathBuf::from(&issuer),
                issuer_signer: signer,
                not_before: nb,
                not_after: na,
                csr: p.get("csr").map(PathBuf::from),
                private_store: p.get("private-store").map(PathBuf::from),
            };
            let out = p.require("out", "leaf rotate")?;
            let manifest = engine::rotate_leaf(&req, &PathBuf::from(&out))?;
            Ok(emit(
                json_out,
                manifest_text("Leaf rotated", &manifest, &out),
                manifest_json(&manifest, &out),
            ))
        }
        "inspect" => inspect_cmd(rest, json_out, Some(BundleType::Leaf)),
        "list" => list_cmd(rest, json_out, BundleType::Leaf),
        other => Err(CertError::usage(format!(
            "unknown leaf verb '{other}' (issue | renew | rotate | inspect | list)"
        ))),
    }
}

/// `root replace` / `issuer replace` (§5.2): the replacement
/// orchestration in [`crate::replace`].
fn replace_cmd(rest: &[String], json_out: bool, rotate_roots: bool) -> Result<i32> {
    let verb = if rotate_roots {
        "root replace"
    } else {
        "issuer replace"
    };
    let p = parse_args(
        rest,
        &[
            "root",
            "root-key-dir",
            "issuer",
            "out",
            "private-store",
            "not-before",
            "root-valid-for",
            "issuer-valid-for",
        ],
        &["allow-legacy-root-strength", "private-in-bundle", "json"],
    )?;
    let root = p.require("root", verb)?;
    let signer = signer_source(&p, "root-key-dir", verb)?;
    // §24.3 ceremony: the new keys' destination must be explicit.
    let private_store = match (p.get("private-store"), p.has("private-in-bundle")) {
        (Some(dir), _) => Some(PathBuf::from(dir)),
        (None, true) => None,
        (None, false) => {
            return Err(CertError::policy(format!(
                "{verb} makes new keys: name their destination explicitly — \
                 --private-store <dir> or --private-in-bundle (spec §24.3)"
            )));
        }
    };
    let nb = p.get("not-before").map(Rfc3339::parse).transpose()?;
    let window = |flag: &str, years: i64| -> Result<(Rfc3339, Rfc3339)> {
        let vf = p.get(flag).map(CalendarDuration::parse).transpose()?;
        crate::timeparse::resolve_validity(nb, vf, None, CalendarDuration::years(years))
    };
    let (not_before, root_not_after) = window(
        "root-valid-for",
        crate::timeparse::ROOT_DEFAULT_LIFETIME_YEARS,
    )?;
    let (_, issuer_not_after) = window(
        "issuer-valid-for",
        crate::timeparse::ISSUER_DEFAULT_LIFETIME_YEARS,
    )?;
    let req = crate::replace::ReplaceRequest {
        root_bundle: PathBuf::from(&root),
        root_signer: signer,
        issuer_bundles: p.all("issuer").into_iter().map(PathBuf::from).collect(),
        rotate_roots,
        not_before,
        root_not_after,
        issuer_not_after,
        allow_legacy_root_strength: p.has("allow-legacy-root-strength"),
        private_store,
    };
    let out = p.require("out", verb)?;
    let report = crate::replace::replace(&req, &PathBuf::from(&out))?;
    let mut text = String::new();
    if let Some(root) = &report.root {
        text.push_str(&format!(
            "Root set rotated: {} at {}\n",
            root.manifest.bundle_id, root.dir
        ));
    }
    for issuer in &report.issuers {
        text.push_str(&format!(
            "Issuer set replaced: {} ({}) at {}\n",
            issuer.manifest.name, issuer.manifest.bundle_id, issuer.dir
        ));
    }
    text.push_str(&format!(
        "Chains verified: {}\nOld material can retire after {}\nNext steps:\n",
        report.checks.len(),
        report.retire_old_after
    ));
    for (i, step) in report.next_steps.iter().enumerate() {
        text.push_str(&format!("  {}. {step}\n", i + 1));
    }
    let value = serde_json::to_value(&report).unwrap_or_else(|_| json!({}));
    Ok(emit(json_out, text, value))
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

// ---------------------------------------------------------------- revoke

/// `fleet cert revoke leaf|issuer|root` (§6.1): append a revocation
/// record to the target bundle's manifest `revocations/` tree.
fn revoke_cmd(args: &[String], json_out: bool) -> Result<i32> {
    let target = match args.first().map(|s| s.as_str()) {
        Some(t) if !t.starts_with('-') => crate::model::RevokeTarget::parse(t)?,
        _ => {
            return Err(CertError::usage(
                "revoke needs a target: leaf | issuer | root (spec §6.1)",
            ));
        }
    };
    let rest = &args[1.min(args.len())..];
    let p = parse_args(rest, &["bundle", "reason", "at"], &["json"])?;
    let dir = p
        .get("bundle")
        .map(|s| s.to_string())
        .or_else(|| p.positional(0).map(|s| s.to_string()))
        .ok_or_else(|| CertError::usage("revoke needs a bundle directory (--bundle <dir>)"))?;
    let reason = p.get("reason").unwrap_or("unspecified").to_string();
    let at = p
        .get("at")
        .map(Rfc3339::parse)
        .transpose()?
        .unwrap_or_else(Rfc3339::now);
    let req = crate::revoke::RevokeRequest {
        bundle: PathBuf::from(&dir),
        target,
        reason,
        revoked_at: at,
    };
    let records = crate::revoke::revoke_bundle(&req)?;
    let mut text = format!(
        "Revoked {} certificate(s) in {dir} ({}):\n",
        records.len(),
        target.as_str()
    );
    for r in &records {
        text.push_str(&format!(
            "  {:10} {:40} {} at {} — {}\n",
            r.cert_role, r.subject_dn, r.reason, r.revoked_at, r.path
        ));
    }
    let value = json!({
        "bundle": dir,
        "target": target.as_str(),
        "revocations": records.iter().map(|r| json!({
            "path": r.path,
            "cert_role": r.cert_role,
            "subject_dn": r.subject_dn,
            "fingerprint_sha256": r.fingerprint_sha256,
            "serial_hex": r.serial_hex,
            "algorithm": r.algorithm.name(),
            "partition": r.partition,
            "reason": r.reason,
            "revoked_at": r.revoked_at.to_string_canonical(),
        })).collect::<Vec<_>>(),
    });
    Ok(emit(json_out, text, value))
}

fn crl_cmd(args: &[String], json_out: bool) -> Result<i32> {
    let verb = args.first().map(|s| s.as_str()).unwrap_or("");
    let rest = &args[1.min(args.len())..];
    match verb {
        "generate" => {
            let p = parse_args(
                rest,
                &[
                    "issuer",
                    "issuer-algorithm",
                    "issuer-key-dir",
                    "out",
                    "at",
                    "next-update",
                    "revocations",
                    "crl-number",
                    "previous-crl",
                ],
                &["pem", "json"],
            )?;
            let issuer = p.require("issuer", "crl generate")?;
            let key_dir = p.require("issuer-key-dir", "crl generate")?;
            let at = p
                .get("at")
                .map(Rfc3339::parse)
                .transpose()?
                .unwrap_or_else(Rfc3339::now);
            let next = match p.get("next-update") {
                Some(value) => Rfc3339::parse(value)?,
                None => CalendarDuration::days(7).add_to(at)?,
            };
            let crl_number = p
                .require("crl-number", "crl generate")?
                .parse::<u64>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or_else(|| CertError::usage("--crl-number must be a positive integer"))?;
            let req = crate::crl::GenerateRequest {
                issuer_bundle: PathBuf::from(issuer),
                signer: SignerSource {
                    key_dir: PathBuf::from(key_dir),
                },
                revocation_bundles: p
                    .all("revocations")
                    .into_iter()
                    .map(PathBuf::from)
                    .collect(),
                this_update: at,
                next_update: next,
                crl_number,
                previous_crl: p.get("previous-crl").map(PathBuf::from),
            };
            let der = crate::crl::generate(&req, p.get("issuer-algorithm"))?;
            let out = p.require("out", "crl generate")?;
            let bytes = if p.has("pem") {
                crate::bundle::pem_encode("X509 CRL", &der).into_bytes()
            } else {
                der.clone()
            };
            std::fs::write(&out, &bytes)?;
            Ok(emit(
                json_out,
                format!(
                    "CRL written to {out} ({} bytes, {} revoked certificates)\n",
                    bytes.len(),
                    crate::crl::inspect(&der)?.revoked.len()
                ),
                json!({"out": out, "encoding": if p.has("pem") {"pem"} else {"der"}, "bytes": bytes.len(), "crl": crate::crl::inspect(&der)?}),
            ))
        }
        "inspect" => {
            let p = parse_args(rest, &["crl"], &["json"])?;
            let path = p
                .get("crl")
                .or_else(|| p.positional(0))
                .ok_or_else(|| CertError::usage("crl inspect requires a CRL file"))?;
            let der = read_crl(PathBuf::from(path).as_path())?;
            let info = crate::crl::inspect(&der)?;
            let entries = info
                .revoked
                .iter()
                .map(|r| {
                    format!(
                        "    {} {} {}\n",
                        r.serial_hex,
                        r.revocation_time,
                        r.reason.as_deref().unwrap_or("unspecified")
                    )
                })
                .collect::<String>();
            let text = format!(
                "CRL: {}\n  number: {}\n  thisUpdate: {}\n  nextUpdate: {}\n  revoked: {}\n{}",
                info.issuer,
                info.crl_number.as_deref().unwrap_or("(missing)"),
                info.this_update,
                info.next_update.as_deref().unwrap_or("(missing)"),
                info.revoked.len(),
                entries
            );
            Ok(emit(
                json_out,
                text,
                serde_json::to_value(info).unwrap_or_else(|_| json!({})),
            ))
        }
        "verify" => {
            let p = parse_args(rest, &["crl", "issuer", "at", "revocations"], &["json"])?;
            let crl_path = p
                .get("crl")
                .or_else(|| p.positional(0))
                .ok_or_else(|| CertError::usage("crl verify requires a CRL file (--crl)"))?;
            let issuer = p.require("issuer", "crl verify")?;
            let at = p
                .get("at")
                .map(Rfc3339::parse)
                .transpose()?
                .unwrap_or_else(Rfc3339::now);
            let der = read_crl(PathBuf::from(crl_path).as_path())?;
            let evidence = p
                .all("revocations")
                .into_iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>();
            let report = crate::crl::verify(&der, &PathBuf::from(issuer), &evidence, at)?;
            let code = emit(
                json_out,
                format!(
                    "CRL verification: {}\n{}\n",
                    if report.valid { "VALID" } else { "INVALID" },
                    report
                        .checks
                        .iter()
                        .map(|c| format!("  - {c}\n"))
                        .collect::<String>()
                ),
                serde_json::to_value(&report).unwrap_or_else(|_| json!({})),
            );
            Ok(if report.valid { code } else { 1 })
        }
        other => Err(CertError::usage(format!(
            "unknown crl verb '{other}' (generate | inspect | verify)"
        ))),
    }
}

fn read_crl(path: &std::path::Path) -> Result<Vec<u8>> {
    crate::crl::read_crl(path)
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
    if let Some(predecessor) = &m.predecessor {
        if m.revision > 1 {
            text.push_str(&format!(
                "  revision: r{} on the same keys (renews {predecessor})\n",
                m.revision
            ));
        } else {
            text.push_str(&format!(
                "  generation: g{} on new keys (replaces {predecessor})\n",
                m.generation
            ));
        }
    }
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
                &[
                    "bundle",
                    "host",
                    "aia-base-url",
                    "dns-target",
                    "format",
                    "out",
                ],
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
            let value =
                serde_json::to_value(&plan).map_err(|e| CertError::engine(e.to_string()))?;
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
                if report.valid {
                    "IN SYNC"
                } else {
                    "OUT OF SYNC"
                }
            ));
            let value =
                serde_json::to_value(&report).map_err(|e| CertError::engine(e.to_string()))?;
            let code = emit(json_out, text, value);
            Ok(if report.valid { code } else { 1 })
        }
        "apply" => {
            let p = parse_args(rest, &["bundle", "staging-dir"], &["json"])?;
            if !p.positionals.is_empty() {
                return Err(CertError::usage(
                    "publish apply accepts only --bundle <dir> and --staging-dir <local-dir>; live endpoint targets are not supported",
                ));
            }
            let bundle = p
                .get("bundle")
                .map(PathBuf::from)
                .ok_or_else(|| CertError::usage("publish apply needs --bundle <dir>"))?;
            let staging = p.get("staging-dir").map(PathBuf::from).ok_or_else(|| {
                CertError::usage("publish apply requires --staging-dir <local-dir>")
            })?;
            let report = crate::publish::apply_staging(&bundle, &staging)?;
            let text = format!(
                "Publish staging apply: {} -> {}\\n  wrote {} public artifacts\\n",
                report.bundle,
                report.staging_dir,
                report.written.len()
            );
            let value =
                serde_json::to_value(&report).map_err(|e| CertError::engine(e.to_string()))?;
            Ok(emit(json_out, text, value))
        }
        "status" | "stage" | "check" | "commit" | "rollback" | "render" => {
            Err(CertError::policy(format!(
                "publish {verb} is deferred to a later production-gated slice (per the #9155 gate map): \
                 live endpoint publication remains unavailable; local staging apply has no network path"
            )))
        }
        other => Err(CertError::usage(format!(
            "unknown publish verb '{other}' (plan | verify; apply | status are a later gated slice)"
        ))),
    }
}

// ---------------------------------------------------------------- dns (offline plan | render | verify, #13724)

/// Usage for `fleet cert dns`. The verbs take no provider, endpoint
/// or credential argument: they are offline by construction (contract
/// §2), and a test pins that against this text.
pub const DNS_USAGE: &str = "\
Usage: fleet cert dns plan   --zone <apex> [--bundle <dir>]... [--policy <file>]
                             [--current <snapshot-file>] [--ttl <seconds>]
                             [--out <dir>] [--report json|yaml|text]
       fleet cert dns render --change-set <file> [--format zone|json|yaml]
                             [--out <dir>] [--report json|yaml|text]
       fleet cert dns verify --change-set <file> [--bundle <dir>]...
                             [--report json|yaml|text]

All three verbs are offline: they read local files and write only
under --out. Nothing queries a resolver or a DNS provider. A
change-set that verifies is PROPOSED; publishing and proving it are
production-gated (#9155) and not part of this command. --dry-run is
accepted and changes nothing (every verb is already a dry run).

Policy file (--policy, JSON, schema fleet.pki.dns-policy/v1):
  {\"caa\":  [{\"name\": \"pki\", \"issue\": [\"letsencrypt.org\"]}],
   \"tlsa\": [{\"host\": \"pki\", \"port\": 443, \"bundle\": \"<id|name|slug>\"}]}
  TLSA defaults: usage 3 (DANE-EE), selector 1 (SPKI), matching 1
  (SHA-256); usage 2 pins an issuer (DANE-TA). CAA never names the
  private Fleet CA (spec §24.8).
";

const DNS_PLAN_FLAGS: &[&str] = &[
    "zone", "bundle", "policy", "current", "ttl", "out", "report",
];
const DNS_RENDER_FLAGS: &[&str] = &["change-set", "format", "out", "report"];
const DNS_VERIFY_FLAGS: &[&str] = &["change-set", "bundle", "report"];
const DNS_BOOL_FLAGS: &[&str] = &["json", "dry-run"];

fn dns_report_format(p: &Parsed, json_out: bool) -> Result<String> {
    if json_out {
        return Ok("json".into());
    }
    let f = p.get("report").unwrap_or("text").to_ascii_lowercase();
    match f.as_str() {
        "json" | "text" => Ok(f),
        "yaml" | "yml" => Ok("yaml".into()),
        other => Err(CertError::usage(format!(
            "unknown --report '{other}' (json | yaml | text)"
        ))),
    }
}

fn write_out(dir: &str, file: &str, body: &str) -> Result<PathBuf> {
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(file);
    std::fs::write(&path, body)?;
    Ok(path)
}

fn dns_cmd(args: &[String], json_out: bool) -> Result<i32> {
    let verb = args.first().map(|s| s.as_str()).unwrap_or("");
    let rest = &args[1.min(args.len())..];
    match verb {
        "" | "--help" | "-h" | "help" => {
            print!("{DNS_USAGE}");
            Ok(if verb.is_empty() { 2 } else { 0 })
        }
        "plan" => {
            let p = parse_args(rest, DNS_PLAN_FLAGS, DNS_BOOL_FLAGS)?;
            let ttl = p
                .get("ttl")
                .map(|t| {
                    t.parse::<u32>().map_err(|_| {
                        CertError::usage(format!("--ttl '{t}' is not a whole number of seconds"))
                    })
                })
                .transpose()?;
            let req = crate::dns::DnsPlanRequest {
                zone: p.require("zone", "dns plan")?,
                bundles: p.all("bundle").into_iter().map(PathBuf::from).collect(),
                policy: p.get("policy").map(PathBuf::from),
                current: p.get("current").map(PathBuf::from),
                ttl,
            };
            let cs = crate::dns::plan(&req)?;
            let canonical = crate::dns::render_json(&cs)?;
            if let Some(out) = p.get("out") {
                write_out(out, "dns-change-set.json", &format!("{canonical}\n"))?;
            }
            let body = match dns_report_format(&p, json_out)?.as_str() {
                "json" => format!("{canonical}\n"),
                "yaml" => crate::dns::render_yaml(&cs),
                _ => crate::dns::render_text(&cs),
            };
            print!("{body}");
            Ok(0)
        }
        "render" => {
            let p = parse_args(rest, DNS_RENDER_FLAGS, DNS_BOOL_FLAGS)?;
            let path = p.require("change-set", "dns render")?;
            let cs = crate::dns::read_change_set(Path::new(&path))?;
            let format = p.get("format").unwrap_or("zone").to_ascii_lowercase();
            let (file, body) = match format.as_str() {
                "zone" => ("dns.zone", crate::dns::render_zone(&cs)),
                "json" => (
                    "dns-change-set.json",
                    format!("{}\n", crate::dns::render_json(&cs)?),
                ),
                "yaml" | "yml" => ("dns-change-set.yaml", crate::dns::render_yaml(&cs)),
                other => {
                    return Err(CertError::usage(format!(
                        "unknown --format '{other}' (zone | json | yaml)"
                    )));
                }
            };
            if let Some(out) = p.get("out") {
                let written = write_out(out, file, &body)?;
                let report = dns_report_format(&p, json_out)?;
                let value = json!({"change_set": path, "format": format, "written": written.display().to_string()});
                let text = format!("rendered {format} to {}\n", written.display());
                return Ok(emit(report == "json", text, value));
            }
            print!("{body}");
            Ok(0)
        }
        "verify" => {
            let p = parse_args(rest, DNS_VERIFY_FLAGS, DNS_BOOL_FLAGS)?;
            let path = p.require("change-set", "dns verify")?;
            let cs = crate::dns::read_change_set(Path::new(&path))?;
            let bundles: Vec<PathBuf> = p.all("bundle").into_iter().map(PathBuf::from).collect();
            let report = crate::dns::verify(&cs, &path, &bundles)?;
            let value =
                serde_json::to_value(&report).map_err(|e| CertError::engine(e.to_string()))?;
            let mut text = format!("DNS verify: {} ({})\n", report.change_set, report.zone);
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
                if report.valid {
                    "PROPOSED (valid offline; not published or proven)"
                } else {
                    "INVALID"
                }
            ));
            let format = dns_report_format(&p, json_out)?;
            let code = if format == "yaml" {
                // The verify report is small; YAML falls back to JSON,
                // which YAML 1.2 parsers read unchanged.
                println!(
                    "{}",
                    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".into())
                );
                0
            } else {
                emit(format == "json", text, value)
            };
            Ok(if report.valid { code } else { 1 })
        }
        "apply" | "status" | "publish" | "check" | "prove" => Err(CertError::policy(format!(
            "dns {verb} is not part of this verb group: live DNS writes and live proof are \
             production-gated (#9155) and undesigned on purpose; plan | render | verify are offline"
        ))),
        other => Err(CertError::usage(format!(
            "unknown dns verb '{other}' (plan | render | verify)"
        ))),
    }
}

// ---------------------------------------------------------------- batch (§6.5)

pub const BATCH_USAGE: &str = "\
Usage: fleet cert batch <verb> --file <batch.json|-> --base <dir> [options]

Verbs:
  validate   Check the batch and the base directory; writes nothing.
  plan       Show the resolved DAG and the batch digest; writes nothing.
  apply      Run every step in a staging area under <base>/.fleet-batch/,
             then move the finished bundles into place, all or nothing.
             Needs --approve <sha256> from the reviewed plan.
             --dry-run runs every step, then removes the staging area.
  status     Compare the batch's receipt with the bundles on disk.

A batch is fleet.batch/v1 JSON: operations with id, after, command
(cert.issuer.create | cert.leaf.issue), out (relative to --base),
with (typed arguments) and expect. Steps reference each other only as
@<id>.bundle and @<id>.keys; signer key directories are always named
explicitly (spec §24.3). There is no interpolation or shell expansion.
Root creation stays `fleet cert root create`. A completed apply writes
a receipt, so applying the same batch again reissues nothing.

Options:
  --file <path|->     The batch document (- reads stdin).
  --base <dir>        Directory that out paths and relative inputs resolve against.
  --approve <sha256>  apply: the digest printed by plan.
  --dry-run           apply: rehearse in staging, then discard.
  --json              Emit the typed report as JSON.
";

fn batch_input(p: &Parsed, verb: &str) -> Result<(crate::batch::Batch, PathBuf, String, String)> {
    let file = p.require("file", &format!("batch {verb}"))?;
    let base_label = p.require("base", &format!("batch {verb}"))?;
    let base = PathBuf::from(&base_label);
    if !base.is_dir() {
        return Err(CertError::usage(format!(
            "--base {} is not a directory",
            base.display()
        )));
    }
    let bytes = if file == "-" {
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut std::io::stdin(), &mut buf)?;
        buf
    } else {
        std::fs::read(&file)
            .map_err(|e| CertError::usage(format!("cannot read --file {file}: {e}")))?
    };
    let batch = crate::batch::load(&bytes, &base)?;
    Ok((batch, base, file, base_label))
}

fn batch_emit(json_out: bool, rep: &crate::batch::Report, file: &str, base: &str) -> Result<i32> {
    let value = serde_json::to_value(rep)?;
    Ok(emit(
        json_out,
        crate::batch::render_text(rep, file, base),
        value,
    ))
}

fn batch_cmd(args: &[String], json_out: bool) -> Result<i32> {
    let verb = args.first().map(|s| s.as_str()).unwrap_or("");
    let rest = &args[1.min(args.len())..];
    match verb {
        "" | "--help" | "-h" | "help" => {
            print!("{BATCH_USAGE}");
            Ok(if verb.is_empty() { 2 } else { 0 })
        }
        "validate" | "plan" | "status" => {
            let p = parse_args(rest, &["file", "base"], &["json"])?;
            let (batch, base, file, base_label) = batch_input(&p, verb)?;
            let rep = match verb {
                "validate" => crate::batch::validate(&batch, &base)?,
                "plan" => crate::batch::plan(&batch),
                _ => crate::batch::status(&batch, &base)?,
            };
            let code = batch_emit(json_out, &rep, &file, &base_label)?;
            Ok(
                if verb == "status" && rep.status != "applied" && rep.status != "pending" {
                    1
                } else {
                    code
                },
            )
        }
        "apply" => {
            let p = parse_args(rest, &["file", "base", "approve"], &["json", "dry-run"])?;
            let (batch, base, file, base_label) = batch_input(&p, verb)?;
            let rep = crate::batch::apply(
                &batch,
                &base,
                &crate::batch::ApplyOptions {
                    dry_run: p.has("dry-run"),
                    approve: p.get("approve").map(|s| s.to_string()),
                },
            )?;
            batch_emit(json_out, &rep, &file, &base_label)
        }
        "resume" => Err(CertError::policy(
            "batch apply is all-or-nothing: a failed apply leaves nothing to resume, and applying \
             a completed batch again returns its receipt without reissuing. Run `fleet cert batch \
             status` to see where a batch stands.",
        )),
        other => Err(CertError::usage(format!(
            "unknown batch verb '{other}' (validate | plan | apply | status)"
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

fn format_cmd(args: &[String], json_out: bool) -> Result<i32> {
    let verb = args.first().map(|s| s.as_str()).unwrap_or("");
    let rest = &args[1.min(args.len())..];
    match verb {
        "convert" => {
            let p = parse_args(
                rest,
                &[
                    "in",
                    "to",
                    "from",
                    "out",
                    "bundle",
                    "password-file",
                    "algorithm",
                ],
                &["json"],
            )?;
            if let Some(from) = p.get("from") {
                if from == "jwk" {
                    if p.get("bundle").is_some() || p.get("password-file").is_some() {
                        return Err(CertError::usage(
                            "format convert --from jwk takes no --bundle or --password-file",
                        ));
                    }
                    let input = p.require("in", "format convert --from jwk")?;
                    let target = p.require("to", "format convert --from jwk")?;
                    let to = crate::format::Encoding::parse(&target)?;
                    let bytes = std::fs::read(&input)
                        .map_err(|e| CertError::usage(format!("cannot read {input}: {e}")))?;
                    let text = std::str::from_utf8(&bytes)
                        .map_err(|_| CertError::validation("JWK input is not UTF-8 JSON"))?;
                    let value: Value = serde_json::from_str(text).map_err(|e| {
                        CertError::validation(format!("JWK input is not a JSON object: {e}"))
                    })?;
                    // Strict public-only subset: private members, unknown
                    // members, RSA, and ML-DSA are rejected here, never
                    // silently dropped.
                    let (algorithm, raw) = crate::jwk::public_to_raw(&value)?;
                    let spki = engine::public_key_spki_der(algorithm, &raw);
                    let out = match to {
                        crate::format::Encoding::Pem => {
                            crate::bundle::pem_encode("PUBLIC KEY", &spki).into_bytes()
                        }
                        crate::format::Encoding::Der => spki,
                    };
                    let fingerprint = crate::bundle::sha256_hex(&raw);
                    match p.get("out") {
                        Some(path) => {
                            std::fs::write(path, &out).map_err(|e| {
                                CertError::engine(format!("cannot write {path}: {e}"))
                            })?;
                            return Ok(emit(
                                json_out,
                                format!(
                                    "wrote {} public key ({}) from JWK {input} to {path}\n  public-key SHA-256: {fingerprint}\n  private material: not exported\n",
                                    algorithm.name(),
                                    to.as_str(),
                                ),
                                json!({ "out": path, "kind": "public-key", "algorithm": algorithm.name(), "format": to.as_str(), "from": "jwk", "bytes": out.len(), "public_key_fingerprint_sha256": fingerprint, "private_material_emitted": false }),
                            ));
                        }
                        None => {
                            use std::io::Write;
                            std::io::stdout().write_all(&out).map_err(|e| {
                                CertError::engine(format!("cannot write stdout: {e}"))
                            })?;
                            return Ok(0);
                        }
                    }
                }
                if from != "pkcs12" {
                    return Err(CertError::usage(format!(
                        "unknown format convert --from '{from}' (pkcs12)"
                    )));
                }
                if p.get("to").is_some() {
                    return Err(CertError::usage(
                        "format convert --from pkcs12 takes no --to; output is DER files in --out",
                    ));
                }
                if p.get("bundle").is_some() || p.get("algorithm").is_some() {
                    return Err(CertError::usage(
                        "format convert --from pkcs12 takes no --bundle or --algorithm",
                    ));
                }
                let input = p.require("in", "format convert --from pkcs12")?;
                let password_path =
                    PathBuf::from(p.require("password-file", "format convert --from pkcs12")?);
                let out_dir = PathBuf::from(p.require("out", "format convert --from pkcs12")?);
                let password = std::fs::read_to_string(&password_path).map_err(|e| {
                    CertError::policy(format!(
                        "cannot read PKCS#12 password file {}: {e}",
                        password_path.display()
                    ))
                })?;
                let password = password.trim_end_matches(['\r', '\n']);
                let bytes = std::fs::read(&input)
                    .map_err(|e| CertError::usage(format!("cannot read {input}: {e}")))?;
                let imported = crate::pkcs12::import(&bytes, password)?;
                std::fs::create_dir(&out_dir).map_err(|e| {
                    CertError::policy(format!(
                        "cannot create PKCS#12 import directory {}: {e}",
                        out_dir.display()
                    ))
                })?;
                let mut created: Vec<PathBuf> = Vec::new();
                let cert_path = out_dir.join("certificate.der");
                if let Err(e) = write_new_file(
                    &cert_path,
                    &imported.certificate_der,
                    false,
                    "imported certificate",
                ) {
                    for path in created.iter().rev() {
                        let _ = std::fs::remove_file(path);
                    }
                    let _ = std::fs::remove_dir(&out_dir);
                    return Err(e);
                }
                created.push(cert_path.clone());
                let key_path = out_dir.join("private-key.pk8");
                if let Err(e) = write_new_file(
                    &key_path,
                    &imported.private_key_pkcs8_der,
                    true,
                    "imported private key",
                ) {
                    for path in created.iter().rev() {
                        let _ = std::fs::remove_file(path);
                    }
                    let _ = std::fs::remove_dir(&out_dir);
                    return Err(e);
                }
                created.push(key_path.clone());
                let mut chain_paths: Vec<PathBuf> = Vec::new();
                for (index, der) in imported.chain_der.iter().enumerate() {
                    let path = out_dir.join(format!("chain-{index:02}.der"));
                    match write_new_file(&path, der, false, "imported chain certificate") {
                        Ok(()) => {
                            created.push(path.clone());
                            chain_paths.push(path);
                        }
                        Err(e) => {
                            for path in created.iter().rev() {
                                let _ = std::fs::remove_file(path);
                            }
                            let _ = std::fs::remove_dir(&out_dir);
                            return Err(e);
                        }
                    }
                }
                let fingerprint = crate::bundle::sha256_hex(&imported.certificate_der);
                return Ok(emit(
                    json_out,
                    format!(
                        "imported PKCS#12 identity to {} (leaf sha256 {}, {} chain certificates; private key owner-only)\n",
                        out_dir.display(),
                        fingerprint,
                        imported.chain_der.len()
                    ),
                    json!({
                        "out": out_dir,
                        "format": "pkcs12",
                        "from": "pkcs12",
                        "certificate": cert_path,
                        "private_key": key_path,
                        "chain": chain_paths,
                        "certificate_sha256": fingerprint,
                        "chain_certificates": imported.chain_der.len(),
                        "private_material_emitted": true,
                    }),
                ));
            }
            let target = p.require("to", "format convert")?;
            let algorithm = p.get("algorithm").map(parse_algorithm_name).transpose()?;
            if target == "jwk" {
                if p.get("password-file").is_some() {
                    return Err(CertError::usage(
                        "format convert --to jwk takes no --password-file (the JWK holds public material only)",
                    ));
                }
                let algorithm = algorithm.ok_or_else(|| {
                    CertError::usage("format convert --to jwk requires --algorithm <id>")
                })?;
                // Keys in bundles: select by registry algorithm among the
                // bundle's private-key identities, mirroring the PKCS#12
                // export's exactly-one-identity rule.
                let (source, key_der) = match (p.get("in"), p.get("bundle")) {
                    (Some(input), None) => {
                        let bytes = std::fs::read(input)
                            .map_err(|e| CertError::usage(format!("cannot read {input}: {e}")))?;
                        (
                            input.to_string(),
                            crate::format::private_key_der(&bytes, algorithm)?,
                        )
                    }
                    (None, Some(bundle_dir)) => {
                        let bundle = load_bundle(&PathBuf::from(bundle_dir))?;
                        let identities: Vec<_> = bundle
                            .manifest
                            .certificates
                            .iter()
                            .filter(|c| c.key_path.is_some() && c.algorithm == algorithm)
                            .collect();
                        if identities.len() != 1 {
                            return Err(CertError::validation(format!(
                                "JWK export requires exactly one {} private-key identity in bundle; found {}",
                                algorithm.name(),
                                identities.len()
                            )));
                        }
                        let key_path = identities[0]
                            .key_path
                            .as_deref()
                            .expect("filtered key path");
                        let key_bytes = read_artifact(&bundle, key_path)?;
                        (
                            bundle_dir.to_string(),
                            crate::format::private_key_der(&key_bytes, algorithm)?,
                        )
                    }
                    _ => {
                        return Err(CertError::usage(
                            "format convert --to jwk requires exactly one of --in <key> or --bundle <dir>",
                        ));
                    }
                };
                let jwk = crate::jwk::public_from_pkcs8(algorithm, &key_der)?;
                // Re-parse the export through the strict reader: the
                // bytes on disk must be exactly the public subset.
                let (_, raw) = crate::jwk::public_to_raw(&jwk)?;
                let fingerprint = crate::bundle::sha256_hex(&raw);
                let mut text = serde_json::to_string_pretty(&jwk)
                    .map_err(|e| CertError::engine(format!("cannot render JWK: {e}")))?;
                text.push('\n');
                let bytes = text.into_bytes();
                match p.get("out") {
                    Some(path) => {
                        write_new_file(Path::new(path), &bytes, false, "public JWK")?;
                        return Ok(emit(
                            json_out,
                            format!(
                                "wrote public JWK for {} key ({source}) to {path}\n  public-key SHA-256: {fingerprint}\n  private material: not exported\n",
                                algorithm.name(),
                            ),
                            json!({ "out": path, "kind": "jwk", "algorithm": algorithm.name(), "format": "jwk", "source": source, "bytes": bytes.len(), "public_key_fingerprint_sha256": fingerprint, "private_material_emitted": false }),
                        ));
                    }
                    None => {
                        use std::io::Write;
                        std::io::stdout()
                            .write_all(&bytes)
                            .map_err(|e| CertError::engine(format!("cannot write stdout: {e}")))?;
                        return Ok(0);
                    }
                }
            }
            if target == "pkcs12" {
                let bundle_dir = p.require("bundle", "format convert --to pkcs12")?;
                let password_path =
                    PathBuf::from(p.require("password-file", "format convert --to pkcs12")?);
                let password = std::fs::read_to_string(&password_path).map_err(|e| {
                    CertError::policy(format!(
                        "cannot read PKCS#12 password file {}: {e}",
                        password_path.display()
                    ))
                })?;
                let password = password.trim_end_matches(['\r', '\n']);
                let bundle = load_bundle(&PathBuf::from(&bundle_dir))?;
                let identities: Vec<_> = bundle
                    .manifest
                    .certificates
                    .iter()
                    .filter(|c| c.key_path.is_some())
                    .collect();
                if identities.len() != 1 {
                    return Err(CertError::validation(format!(
                        "PKCS#12 export requires exactly one private-key identity in bundle; found {}",
                        identities.len()
                    )));
                }
                let identity = identities[0];
                let certificate_bytes = read_artifact(&bundle, &identity.cert_path)?;
                let certificate_der = crate::format::certificate_der(&certificate_bytes)?;
                let key_path = identity.key_path.as_deref().expect("filtered key path");
                let key_der = read_artifact(&bundle, key_path)?;
                let chain_der = bundle
                    .manifest
                    .certificates
                    .iter()
                    .filter(|c| c.cert_path != identity.cert_path)
                    .map(|c| {
                        read_artifact(&bundle, &c.cert_path)
                            .and_then(|bytes| crate::format::certificate_der(&bytes))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let archive =
                    crate::pkcs12::export(&certificate_der, &key_der, &chain_der, password)?;
                let path = PathBuf::from(p.require("out", "format convert --to pkcs12")?);
                write_new_file(&path, &archive, true, "PKCS#12 archive")?;
                return Ok(emit(
                    json_out,
                    format!(
                        "wrote password-protected PKCS#12 identity to {}\n",
                        path.display()
                    ),
                    json!({"out": path, "format": "pkcs12", "bundle": bundle_dir, "bytes": archive.len(), "private_material_emitted": true}),
                ));
            }
            let input = p.require("in", "format convert")?;
            let to = crate::format::Encoding::parse(&target)?;
            let bytes = std::fs::read(&input)
                .map_err(|e| CertError::usage(format!("cannot read {input}: {e}")))?;

            if let Some(algorithm) = algorithm {
                // Private-key material is never written to stdout and the
                // destination is created owner-only without replacing an
                // existing file.
                let path = p.require("out", "private-key format convert")?;
                let out = crate::format::convert_private_key(&bytes, to, algorithm)?;
                write_new_file(Path::new(&path), &out, true, "converted private key")?;
                return Ok(emit(
                    json_out,
                    format!(
                        "wrote {} private-key encoding ({}, {})\n",
                        path,
                        algorithm.name(),
                        to.as_str()
                    ),
                    json!({
                        "out": path,
                        "kind": "private-key",
                        "algorithm": algorithm.name(),
                        "format": to.as_str(),
                        "bytes": out.len(),
                    }),
                ));
            }

            // Without --algorithm, preserve the original certificate path,
            // including its stdout behavior and overwrite semantics.
            let out = crate::format::convert(&bytes, to)?;
            match p.get("out") {
                Some(path) => {
                    std::fs::write(path, &out)
                        .map_err(|e| CertError::engine(format!("cannot write {path}: {e}")))?;
                    Ok(emit(
                        json_out,
                        format!("wrote {} ({})\n", path, to.as_str()),
                        json!({ "out": path, "format": to.as_str(), "bytes": out.len() }),
                    ))
                }
                None => {
                    use std::io::Write;
                    std::io::stdout()
                        .write_all(&out)
                        .map_err(|e| CertError::engine(format!("cannot write stdout: {e}")))?;
                    Ok(0)
                }
            }
        }
        "verify" => {
            let p = parse_args(
                rest,
                &["in", "password-file", "algorithm", "key"],
                &["json"],
            )?;
            let input = p.require("in", "format verify")?;
            let algorithm = p.get("algorithm").map(parse_algorithm_name).transpose()?;
            let bytes = std::fs::read(&input)
                .map_err(|e| CertError::usage(format!("cannot read {input}: {e}")))?;
            if let Some(key_path) = p.get("key") {
                // JWK verification proves the JWK matches the canonical
                // DER key material: the JWK alone can only be
                // well-formedness, never byte-faithfulness.
                if p.get("password-file").is_some() {
                    return Err(CertError::usage(
                        "format verify of a JWK takes no --password-file",
                    ));
                }
                let algorithm = algorithm.ok_or_else(|| {
                    CertError::usage(
                        "format verify of a JWK requires --algorithm <id> and --key <pkcs8>",
                    )
                })?;
                let text = std::str::from_utf8(&bytes)
                    .map_err(|_| CertError::validation("JWK input is not UTF-8 JSON"))?;
                let value: Value = serde_json::from_str(text).map_err(|e| {
                    CertError::validation(format!("JWK input is not a JSON object: {e}"))
                })?;
                let key_bytes = std::fs::read(key_path)
                    .map_err(|e| CertError::usage(format!("cannot read {key_path}: {e}")))?;
                let key_der = crate::format::private_key_der(&key_bytes, algorithm)?;
                crate::jwk::verify_public_roundtrip(algorithm, &key_der, &value)?;
                let (_, raw) = crate::jwk::public_to_raw(&value)?;
                let fingerprint = crate::bundle::sha256_hex(&raw);
                return Ok(emit(
                    json_out,
                    format!(
                        "{input}: well-formed public JWK ({}) matching {key_path} (public-key SHA-256 {fingerprint}); JWK round trip preserves the DER key\n",
                        algorithm.name(),
                    ),
                    json!({
                        "in": input,
                        "kind": "jwk",
                        "algorithm": algorithm.name(),
                        "key": key_path,
                        "public_key_fingerprint_sha256": fingerprint,
                        "private_material_emitted": false,
                    }),
                ));
            }
            if let Some(password_path) = p.get("password-file") {
                let password_path = PathBuf::from(password_path);
                let password = std::fs::read_to_string(&password_path).map_err(|e| {
                    CertError::policy(format!(
                        "cannot read PKCS#12 password file {}: {e}",
                        password_path.display()
                    ))
                })?;
                let password = password.trim_end_matches(['\r', '\n']);
                let verified = crate::pkcs12::verify(&bytes, password)?;
                return Ok(emit(
                    json_out,
                    format!(
                        "{input}: valid password-protected PKCS#12 identity (leaf sha256 {}, {} chain certificates); key material redacted\n",
                        verified.certificate_sha256, verified.chain_certificates
                    ),
                    json!({"in": input, "format": "pkcs12", "certificate_sha256": verified.certificate_sha256, "chain_certificates": verified.chain_certificates, "private_material_emitted": false}),
                ));
            }

            if let Some(algorithm) = algorithm {
                let verified = crate::format::verify_private_key(&bytes, algorithm)?;
                return Ok(emit(
                    json_out,
                    format!(
                        "{input}: well-formed {} PKCS#8 private key ({} input, {} DER bytes); PEM and DER round trips preserve the key\n",
                        algorithm.name(),
                        verified.input.as_str(),
                        verified.der_len
                    ),
                    json!({
                        "in": input,
                        "kind": "private-key",
                        "algorithm": algorithm.name(),
                        "input": verified.input.as_str(),
                        "der_bytes": verified.der_len,
                    }),
                ));
            }
            let v = crate::format::verify(&bytes)?;
            Ok(emit(
                json_out,
                format!(
                    "{input}: well-formed X.509 certificate ({} input, {} DER bytes, sha256 {}); PEM and DER round trips match\n",
                    v.input.as_str(),
                    v.der_len,
                    v.sha256
                ),
                json!({ "in": input, "input": v.input.as_str(), "der_bytes": v.der_len, "sha256": v.sha256 }),
            ))
        }
        other => Err(CertError::usage(format!(
            "unknown format verb '{other}' (convert | verify)"
        ))),
    }
}

fn san_policy_name(policy: profile::SanPolicy) -> &'static str {
    match policy {
        profile::SanPolicy::Free => "free",
        profile::SanPolicy::Broker => "broker",
    }
}

fn key_custody_name(custody: profile::KeyCustody) -> &'static str {
    match custody {
        profile::KeyCustody::IssuerOrCsr => "issuer-or-csr",
        profile::KeyCustody::HostCsrOnly => "host-csr-only",
    }
}

fn profile_cmd(args: &[String], json_out: bool) -> Result<i32> {
    let verb = args.first().map(|s| s.as_str()).unwrap_or("list");
    let rest = &args[1.min(args.len())..];
    match verb {
        "list" => {
            let mut text = String::from("Leaf profiles (mirroring pkgs/idp-pki idp-ca):\n");
            for prof in profile::leaf_profiles() {
                text.push_str(&format!(
                    "  {:14} key={:8} validity={}d  {}\n",
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
                    "san_policy": san_policy_name(p.san_policy),
                    "key_custody": key_custody_name(p.key_custody),
                    "issuer_purpose": p.issuer_purpose,
                    "renew_within_days": p.renew_within.map(|d| d.days),
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
                "san_policy": san_policy_name(prof.san_policy),
                "key_custody": key_custody_name(prof.key_custody),
                "issuer_purpose": prof.issuer_purpose,
                "renew_within_days": prof.renew_within.map(|d| d.days),
                "notes": prof.notes,
            });
            Ok(emit(json_out, text, value))
        }
        "scaffold" => {
            let p = parse_args(rest, &["name", "out"], &["json"])?;
            let name = p
                .get("name")
                .or_else(|| p.positional(0))
                .ok_or_else(|| CertError::usage("profile scaffold needs --name <name>"))?;
            let out = p.require("out", "profile scaffold")?;
            let scaffold = profile::CustomProfile::scaffold(name);
            scaffold.validate()?;
            let bytes = serde_json::to_vec_pretty(&scaffold)
                .map_err(|e| CertError::engine(e.to_string()))?;
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&out)
                .map_err(|e| CertError::engine(format!("cannot create profile {out}: {e}")))?;
            file.write_all(&bytes)
                .map_err(|e| CertError::engine(format!("cannot write profile {out}: {e}")))?;
            Ok(emit(
                json_out,
                format!("Scaffolded custom profile {name} at {out}\n"),
                json!({"name": name, "path": out, "profile": scaffold}),
            ))
        }
        "validate" => {
            let p = parse_args(rest, &["file"], &["json"])?;
            let path = p
                .get("file")
                .or_else(|| p.positional(0))
                .ok_or_else(|| CertError::usage("profile validate needs <file>"))?;
            let (custom, resolved) = profile::load_custom_profile(&PathBuf::from(path))?;
            let text = format!(
                "Valid custom profile {} (key={}, validity={}d, ceiling={}d)\n",
                custom.name,
                resolved.key_algorithm.name(),
                custom.default_validity_days,
                custom.max_validity_days
            );
            Ok(emit(
                json_out,
                text,
                json!({"valid": true, "profile": custom}),
            ))
        }
        other => Err(CertError::usage(format!(
            "unknown profile verb '{other}' (list | show | validate | scaffold)"
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
        "benchmark" => {
            let p = parse_args(&args[1..], &["iterations"], &["json"])?;
            if p.positionals.len() > 1 {
                return Err(CertError::usage(
                    "algorithms benchmark takes one comma-separated algorithm list",
                ));
            }
            let iterations = match p.get("iterations") {
                Some(v) => v.parse::<usize>().map_err(|_| {
                    CertError::usage(format!("--iterations must be a whole number, got '{v}'"))
                })?,
                None => algorithm_lab::DEFAULT_ITERATIONS,
            };
            let iterations = algorithm_lab::check_iterations(iterations)?;
            let (infos, warnings) = algorithm_lab::select_algorithms(p.positional(0))?;
            let host = algorithm_lab::host_info();
            let mut results = Vec::new();
            for info in &infos {
                results.push(algorithm_lab::benchmark_algorithm(info.id, iterations)?);
            }
            let mut text = format!(
                "fleet cert algorithms benchmark ({iterations} samples per operation, microseconds)\n  host: {} {} · {} cpus · {}{}\n",
                host.os,
                host.arch,
                host.cpus,
                host.backend,
                if host.debug_build {
                    " · DEBUG build (numbers not representative)"
                } else {
                    ""
                }
            );
            text.push_str(&format!(
                "  {:10} {:7} {:>9} {:>9} {:>9} {:>9} {:>9}\n",
                "algorithm", "op", "min", "p50", "p95", "p99", "max"
            ));
            for r in &results {
                for (op, t) in [
                    ("keygen", &r.keygen),
                    ("sign", &r.sign),
                    ("verify", &r.verify),
                ] {
                    text.push_str(&format!(
                        "  {:10} {:7} {:>9} {:>9} {:>9} {:>9} {:>9}\n",
                        r.algorithm, op, t.min_us, t.p50_us, t.p95_us, t.p99_us, t.max_us
                    ));
                }
                text.push_str(&format!(
                    "  {:10} sizes: key {} B · cert {} B · signature {} B\n",
                    r.algorithm,
                    r.private_key_pkcs8_bytes,
                    r.certificate_der_bytes,
                    r.signature_bytes
                ));
            }
            text.push_str(
                "  Evidence for this host and build only, never a CI baseline (spec §26.2).\n",
            );
            for w in &warnings {
                text.push_str(&format!("  warning: {w}\n"));
            }
            let value = json!({
                "iterations": iterations,
                "unit": "microseconds",
                "host": host,
                "results": results,
                "warnings": warnings,
            });
            Ok(emit(json_out, text, value))
        }
        "doctor" => {
            let p = parse_args(&args[1..], &[], &["json"])?;
            if p.positionals.len() > 1 {
                return Err(CertError::usage(
                    "algorithms doctor takes one comma-separated algorithm list",
                ));
            }
            let (infos, warnings) = algorithm_lab::select_algorithms(p.positional(0))?;
            let env = algorithm_lab::OpensslEnv::detect();
            let reports: Vec<_> = infos
                .iter()
                .map(|i| algorithm_lab::doctor_algorithm(i.id, &env))
                .collect();
            let all_ok = reports.iter().all(|r| r.ok());
            let mut text = format!(
                "fleet cert algorithms doctor\n  openssl: {}{}\n",
                env.version.as_deref().unwrap_or("not found"),
                if env.version.is_some() {
                    if env.ml_dsa {
                        " (ML-DSA available)"
                    } else {
                        " (no ML-DSA)"
                    }
                } else {
                    ""
                }
            );
            for r in &reports {
                text.push_str(&format!(
                    "  {} {}\n",
                    r.algorithm,
                    if r.ok() { "ok" } else { "FAIL" }
                ));
                for probe in &r.probes {
                    let status = match probe.status {
                        algorithm_lab::ProbeStatus::Ok => "ok",
                        algorithm_lab::ProbeStatus::Fail => "FAIL",
                        algorithm_lab::ProbeStatus::Skipped => "skipped",
                    };
                    text.push_str(&format!(
                        "    {:17} {:7} {}\n",
                        probe.probe, status, probe.detail
                    ));
                }
            }
            for w in &warnings {
                text.push_str(&format!("  warning: {w}\n"));
            }
            let value = json!({
                "ok": all_ok,
                "openssl": env,
                "algorithms": reports,
                "warnings": warnings,
            });
            let code = emit(json_out, text, value);
            Ok(if all_ok { code } else { 1 })
        }
        other => Err(CertError::usage(format!(
            "unknown algorithms verb '{other}' (list | describe | benchmark | doctor)"
        ))),
    }
}

// ---------------------------------------------------------------- trust (§3.5, BT-2)

fn trust_cmd(args: &[String], json_out: bool) -> Result<i32> {
    let verb = args.first().map(|s| s.as_str()).unwrap_or("");
    let rest = if args.is_empty() { &[][..] } else { &args[1..] };
    match verb {
        "export" => {
            let p = parse_args(
                rest,
                &["root", "out", "version", "name", "at", "target", "trust"],
                &["json"],
            )?;
            if p.has("target") {
                return trust_consumer_export(&p, json_out);
            }
            if p.has("trust") {
                return Err(CertError::usage(
                    "trust export --trust applies to a consumer export (--target <consumer>); \
                     the versioned trust bundle always carries every §3.5 file",
                ));
            }
            let root_bundles: Vec<PathBuf> = p.all("root").into_iter().map(PathBuf::from).collect();
            if root_bundles.is_empty() {
                return Err(CertError::usage(
                    "trust export requires --root <root-set-bundle-dir> (repeatable)",
                ));
            }
            let out = p.require("out", "trust export")?;
            let version_raw = p.require("version", "trust export")?;
            let version: u32 = version_raw.parse().map_err(|_| {
                CertError::usage(format!(
                    "trust export --version '{version_raw}' is not a version number"
                ))
            })?;
            let at = p
                .get("at")
                .map(Rfc3339::parse)
                .transpose()?
                .unwrap_or_else(Rfc3339::now);
            let manifest = crate::trust::export_trust_bundle(
                &crate::trust::TrustExportRequest {
                    root_bundles,
                    version,
                    name: p.get("name").unwrap_or("Fleet Trust Bundle").to_string(),
                    at,
                },
                &PathBuf::from(&out),
            )?;
            let anchors = manifest
                .certificates
                .iter()
                .filter(|c| c.role == "root")
                .count();
            let text = format!(
                "Trust bundle exported: {} (version {}, {} anchors, {} files)\n  out: {}\n",
                manifest.bundle_id,
                manifest.bundle_version,
                anchors,
                manifest.files.len(),
                out
            );
            let value = json!({
                "bundle_id": manifest.bundle_id,
                "bundle_version": manifest.bundle_version,
                "anchors": anchors,
                "files": manifest.files.iter().map(|f| f.path.clone()).collect::<Vec<_>>(),
                "out": out,
            });
            Ok(emit(json_out, text, value))
        }
        "stage" => {
            let p = parse_args(rest, &["bundle", "store"], &["json"])?;
            let bundle = p.require("bundle", "trust stage")?;
            let store = p.require("store", "trust stage")?;
            let vref = crate::trust::stage_bundle(&PathBuf::from(&store), &PathBuf::from(&bundle))?;
            let text = format!(
                "Staged trust bundle version {} ({}) in {}\n",
                vref.version, vref.bundle_id, store
            );
            Ok(emit(json_out, text, serde_json::to_value(&vref)?))
        }
        "ack" => {
            let p = parse_args(rest, &["store", "version", "at"], &["json"])?;
            let store = p.require("store", "trust ack")?;
            let version = p
                .get("version")
                .map(|v| {
                    v.parse::<u32>().map_err(|_| {
                        CertError::usage(format!(
                            "trust ack --version '{v}' is not a version number"
                        ))
                    })
                })
                .transpose()?;
            let at = p
                .get("at")
                .map(Rfc3339::parse)
                .transpose()?
                .unwrap_or_else(Rfc3339::now);
            let ack = crate::trust::acknowledge(&PathBuf::from(&store), version, at)?;
            let text = format!(
                "Acknowledged trust bundle version {} ({}) at {}\n",
                ack.version, ack.bundle_id, ack.acknowledged_at
            );
            Ok(emit(json_out, text, serde_json::to_value(&ack)?))
        }
        "activate" => {
            let p = parse_args(rest, &["store", "version"], &["json"])?;
            let store = p.require("store", "trust activate")?;
            let version = p
                .get("version")
                .map(|v| {
                    v.parse::<u32>().map_err(|_| {
                        CertError::usage(format!(
                            "trust activate --version '{v}' is not a version number"
                        ))
                    })
                })
                .transpose()?;
            let vref = crate::trust::activate(&PathBuf::from(&store), version)?;
            let text = format!(
                "Active trust bundle is now version {} ({})\n",
                vref.version, vref.bundle_id
            );
            Ok(emit(json_out, text, serde_json::to_value(&vref)?))
        }
        "retire" => {
            let p = parse_args(rest, &["store", "version"], &["json"])?;
            let store = p.require("store", "trust retire")?;
            let version_raw = p.require("version", "trust retire")?;
            let version: u32 = version_raw.parse().map_err(|_| {
                CertError::usage(format!(
                    "trust retire --version '{version_raw}' is not a version number"
                ))
            })?;
            crate::trust::retire(&PathBuf::from(&store), version)?;
            let text = format!("Retired trust bundle version {version}\n");
            Ok(emit(json_out, text, json!({"retired": version})))
        }
        "status" => {
            let p = parse_args(rest, &["store"], &["json"])?;
            let store = p.require("store", "trust status")?;
            let status = crate::trust::store_status(&PathBuf::from(&store))?;
            let fmt_ref = |r: &Option<crate::trust::VersionRef>| match r {
                Some(v) => format!("{} ({})", v.version, v.bundle_id),
                None => "none".to_string(),
            };
            let mut text = format!(
                "Trust store {}\n  active:   {}\n  previous: {}\n  staged:   {}\n  installed: {}\n",
                store,
                fmt_ref(&status.active),
                fmt_ref(&status.previous),
                fmt_ref(&status.staged),
                status
                    .installed_versions
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            for ack in &status.acks {
                text.push_str(&format!(
                    "  ack: version {} ({}) at {}\n",
                    ack.version, ack.bundle_id, ack.acknowledged_at
                ));
            }
            Ok(emit(json_out, text, serde_json::to_value(&status)?))
        }
        "inspect" => {
            let p = parse_args(rest, &["root", "trust"], &["json"])?;
            let root_dir = p.require("root", "trust inspect")?;
            let kind = TrustSelection::parse(p.get("trust").unwrap_or("all"))?;
            let bundle = load_bundle(&PathBuf::from(&root_dir))?;
            let assembled = crate::trust_consumer::assemble(&bundle, kind)?;
            let mut text = format!(
                "Trust selection {} ({}) from {}: {} anchor(s)\n",
                crate::trust_consumer::bundle_file_name(kind),
                kind.as_str(),
                bundle.manifest.name,
                assembled.certificates.len()
            );
            for c in &assembled.certificates {
                text.push_str(&format!(
                    "  {} {} {}\n",
                    &c.fingerprint_sha256[..12.min(c.fingerprint_sha256.len())],
                    c.subject_algorithm.map(|a| a.name()).unwrap_or("unknown"),
                    c.subject_dn
                ));
            }
            text.push_str(&format!("  sha256: {}\n", assembled.sha256));
            let anchors: Vec<Value> = assembled
                .certificates
                .iter()
                .map(|c| {
                    json!({
                        "subject_dn": c.subject_dn,
                        "algorithm": c.subject_algorithm.map(|a| a.name()),
                        "fingerprint_sha256": c.fingerprint_sha256,
                    })
                })
                .collect();
            let value = json!({
                "trust": kind.as_str(),
                "bundle": bundle.manifest.name,
                "bundle_id": bundle.manifest.bundle_id,
                "anchors": anchors,
                "bundle_sha256": assembled.sha256,
            });
            Ok(emit(json_out, text, value))
        }
        "pin" => {
            let p = parse_args(rest, &["root", "trust", "out"], &["json"])?;
            let root_dir = p.require("root", "trust pin")?;
            let kind = TrustSelection::parse(p.get("trust").unwrap_or("all"))?;
            let out = p.require("out", "trust pin")?;
            if Path::new(&out).exists() {
                return Err(CertError::policy(format!(
                    "pinset already exists at {out}; remove it explicitly before re-pinning"
                )));
            }
            let bundle = load_bundle(&PathBuf::from(&root_dir))?;
            let pins = crate::trust_consumer::create_pins(&bundle, kind)?;
            std::fs::write(&out, serde_json::to_string_pretty(&pins)? + "\n")?;
            let text = format!(
                "Pinned {} {} anchor(s) -> {out}\n",
                pins.pins.len(),
                kind.as_str()
            );
            Ok(emit(json_out, text, serde_json::to_value(&pins)?))
        }
        "verify" => {
            let p = parse_args(rest, &["root", "trust", "pins", "store"], &["json"])?;
            let root_dir = p.require("root", "trust verify")?;
            let kind = TrustSelection::parse(p.get("trust").unwrap_or("all"))?;
            let pins_path = p.get("pins");
            let store_path = p.get("store");
            if pins_path.is_none() && store_path.is_none() {
                return Err(CertError::usage(
                    "trust verify requires --pins <pinset.json> and/or --store <pem-or-dir>",
                ));
            }
            let bundle = load_bundle(&PathBuf::from(&root_dir))?;
            let mut checks = Vec::new();
            if let Some(path) = pins_path {
                let pins: crate::trust_consumer::TrustPins =
                    serde_json::from_str(&std::fs::read_to_string(path)?)?;
                checks.extend(crate::trust_consumer::check_pins(&bundle, kind, &pins));
            }
            if let Some(path) = store_path {
                let assembled = crate::trust_consumer::assemble(&bundle, kind)?;
                let store = crate::trust_consumer::read_store_certs(Path::new(path))?;
                checks.extend(crate::trust_consumer::check_store(&assembled, &store));
            }
            let ok = checks.iter().all(|c| c.ok);
            let mut text = format!(
                "Trust verify ({}): {}\n",
                kind.as_str(),
                if ok { "OK" } else { "DRIFT" }
            );
            for c in &checks {
                text.push_str(&format!(
                    "  [{}] {}: {}\n",
                    if c.ok { "ok" } else { "FAIL" },
                    c.name,
                    c.detail
                ));
            }
            let code = emit(
                json_out,
                text,
                json!({"trust": kind.as_str(), "ok": ok, "checks": checks}),
            );
            Ok(if ok { code } else { 1 })
        }
        "install-plan" => {
            let p = parse_args(rest, &["target"], &["json"])?;
            let name = p.require("target", "trust install-plan")?;
            let target = trust_target(&name)?;
            let text = format!(
                "Install plan for {} ({} trust; documentation only, nothing installed):\n  install: {}\n  verify: {}\n  remove: {}\n  note: {}\n",
                target.name,
                target.kind.as_str(),
                target.install,
                target.verify,
                target.remove,
                target.note
            );
            let value = json!({
                "consumer": target.name,
                "trust": target.kind.as_str(),
                "bundle_file": crate::trust_consumer::bundle_file_name(target.kind),
                "install": target.install,
                "verify": target.verify,
                "remove": target.remove,
                "note": target.note,
            });
            Ok(emit(json_out, text, value))
        }
        other => Err(CertError::usage(format!(
            "unknown trust verb '{other}' (export | stage | ack | activate | retire | status | inspect | pin | verify | install-plan)"
        ))),
    }
}

/// Resolve a `--target` consumer, refusing `ssh` (not X.509, §44).
fn trust_target(name: &str) -> Result<&'static crate::trust_consumer::ConsumerTarget> {
    if name.trim().eq_ignore_ascii_case("ssh") {
        return Err(crate::trust_consumer::refuse_ssh());
    }
    crate::trust_consumer::consumer_target(name).ok_or_else(|| {
        CertError::usage(format!(
            "unknown trust target '{name}' ({} | ssh is refused)",
            crate::trust_consumer::consumer_names().join(" | ")
        ))
    })
}

/// `trust export --target <consumer>`: PEM files plus per-consumer
/// install/verify/remove guidance from one root set. Writes files
/// only; never touches a consumer's store.
fn trust_consumer_export(p: &Parsed, json_out: bool) -> Result<i32> {
    for flag in ["version", "name", "at"] {
        if p.has(flag) {
            return Err(CertError::usage(format!(
                "trust export --target writes a consumer export from one root set; \
                 --{flag} applies to the versioned trust bundle (omit --target)"
            )));
        }
    }
    let roots = p.all("root");
    let [root_dir] = roots.as_slice() else {
        return Err(CertError::usage(
            "trust export --target requires exactly one --root <root-set-bundle-dir>",
        ));
    };
    let target_name = p.require("target", "trust export")?;
    let out_dir = p.require("out", "trust export")?;
    let target = trust_target(&target_name)?;
    let kind = match p.get("trust") {
        Some(flag) => {
            let want = TrustSelection::parse(flag)?;
            if want != target.kind {
                return Err(CertError::usage(format!(
                    "target {} takes the {} bundle, not {}",
                    target.name,
                    target.kind.as_str(),
                    want.as_str()
                )));
            }
            want
        }
        None => target.kind,
    };
    let bundle = load_bundle(&PathBuf::from(root_dir))?;
    let assembled = crate::trust_consumer::assemble(&bundle, kind)?;
    let manifest = crate::trust_consumer::write_export(
        &PathBuf::from(&out_dir),
        &bundle,
        target.name,
        target,
        &assembled,
    )?;
    let text = format!(
        "Trust export for {} ({}): {} anchor(s) -> {out_dir}\n  manifest: {out_dir}/export.json\n",
        target.name,
        kind.as_str(),
        assembled.certificates.len(),
    );
    Ok(emit(json_out, text, serde_json::to_value(&manifest)?))
}

// ---------------------------------------------------------------- explain / doctor

const NOT_BUILT: &[(&str, &str)] = &[
    (
        "publish status",
        "live AIA/CDP publication status (production-gated; local staging apply is supported)",
    ),
    (
        "key import/export/escrow/restore/rotate",
        "remaining standalone key ceremonies (spec §6.1)",
    ),
    (
        "format",
        "format list/describe/export (spec §7.5; convert/verify is built)",
    ),
    (
        "bundle pack/unpack/diff/migrate",
        "bundle packaging verbs (spec §7)",
    ),
];

fn explain_cmd(args: &[String], json_out: bool) -> Result<i32> {
    let topic = args.first().map(|s| s.as_str()).unwrap_or("overview");
    let body = match topic {
        "overview" => {
            "fleet cert implements the §6.1 command tree's core: root, issuer, leaf, ocsp, \
chain, revoke, crl, bundle, inventory (list | show | search | audit, read-only), publish (plan | verify read-only), trust (export | stage | ack | activate | retire | status — the §3.5 trust bundle and the broker-client trust store, BT-2), profile, algorithms, explain, doctor. Signing is library-first (§6.4): \
the CLI is a thin shell over the fleet-cert crate. Cross-signing is policy data \
(§§3.2/24.2), time is Gregorian calendar arithmetic (§5.4), and ML-DSA-44 is a \
first-class algorithm with RFC 9881 identifiers. Topics: signer-source, \
cross-signing, pq-policy, time, profiles."
                .to_string()
        }
        "ocsp" => "OCSP (RFC 6960) is built as a responder core, not a URL: \
fleet cert ocsp sign produces real signed responses from an issuer bundle — \
directly with the issuer key, or with a delegated responder certificate \
issued under the ocsp-responder leaf profile (EKU id-kp-OCSPSigning + \
NoCheck): spec §4.3’s signing-authorized responder. ocsp inspect parses a \
response and, with --issuer and --leaf/--serial, verifies it against the \
crate chain code: signature, responder authorization, CertID binding, \
freshness. ocsp test runs the synthetic end-to-end (good and revoked \
served and verified locally). No OCSP URL is embedded anywhere by these \
verbs; serving these responses over HTTP is a deployment of this core."
            .to_string(),
        "crl" => "CRLs are generated from checksum-verified revocation bundles and signed by \
the selected primary CA certificate. `crl generate` requires an explicit --issuer-key-dir and \
positive --crl-number; use --previous-crl to reject a non-increasing number. `--issuer-algorithm` \
accepts an issuer fingerprint, algorithm, or subject DN and errors if that selector is ambiguous. \
Only Ed25519, P-256, RSA-SHA256, and ML-DSA-44 signatures are supported. `crl inspect` parses \
the CRL; `crl verify` checks issuer, signature, freshness, and supplied revocation evidence. \
Output is local DER unless --pem is requested. This command does not publish a distribution point."
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
no EKU). Two more are the fleet brokers' TLS leaves (broker-TLS ADR-0001): \
broker-server and broker-client (P-256, serverAuth or clientAuth, 90d, renewed with 30d \
left). They issue only from a Base (general) issuer set, only from a CSR made on the \
broker's host (--csr; the issuer never holds the key), and only with internal SANs: \
at least one --dns name under .internal and a tailnet --ip (100.64.0.0/10 or \
fd7a:115c:a1e0::/48); wildcards, public names and other IPs are refused. Revocation \
is CRL-only: no OCSP URL or AIA is embedded. See `fleet cert profile list`."
            .to_string(),
        "trust" => "Two trust surfaces share one rule: only signature-proven \
self-signed roots become anchors (the record's role label alone never admits one). \
The versioned trust bundle (`trust export --root ... --version N`) feeds the \
broker-client store's stage -> ack -> activate -> retire rollover. The root-set verbs \
work on one root set directly: `trust inspect` lists the anchors a selection admits, \
`trust pin` records approved fingerprints, `trust verify --pins/--store` reports \
rotation, new anchors and consumer-store drift, and `trust export --target <consumer>` \
writes PEM files plus that consumer's install/verify/remove guidance. None of them \
installs trust: live trust-store mutation is production-gated (#9155, spec §44). \
`ssh` is refused because native OpenSSH CA trust is not X.509."
            .to_string(),
        other => {
            return Err(CertError::usage(format!(
                "unknown explain topic '{other}' (overview | signer-source | cross-signing | pq-policy | time | trust | profiles | crl)"
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
        let ok = algorithm_lab::engine_self_test(info.id);
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
    text.push_str("  built: root, issuer (create/renew/rotate/inspect/list), leaf (issue/renew/inspect/list), key (generate/inspect/import/export public), format (convert/verify: PEM/DER certificate+key, PKCS#12, JWK), chain, revoke, crl, bundle (inspect), inventory (list/show/search/audit read-only), publish (plan/verify read-only, apply to local staging), dns (plan/render/verify offline), batch (validate/plan/apply/status), trust (export/stage/ack/activate/retire/status/inspect/pin/verify/install-plan), profile (list/show/validate/scaffold), algorithms (list/describe/benchmark/doctor), explain, doctor\n");
    text.push_str("  not built yet (tracked as child issues under #9157):\n");
    for (name, what) in NOT_BUILT {
        text.push_str(&format!("    {name} — {what}\n"));
    }
    let value = json!({
        "engine_self_test": algo_results.iter().map(|(i, ok)| json!({"algorithm": i.name, "ok": ok})).collect::<Vec<_>>(),
        "openssl": openssl,
        "built": ["root", "root-renew", "root-rotate", "root-replace", "issuer", "issuer-replace", "issuer-renew", "issuer-rotate", "leaf", "leaf-renew", "leaf-rotate", "key-generate/inspect", "format-convert/verify", "chain", "revoke", "crl", "bundle-inspect", "inventory-list/show/search/audit", "publish-plan/apply-staging/verify", "dns-plan/render/verify", "batch-validate/plan/apply/status", "trust-export/stage/ack/activate/retire/status/inspect/pin/verify/install-plan", "profile-list/show/validate/scaffold", "algorithms-list/describe/benchmark/doctor", "explain", "doctor"],
        "not_built": NOT_BUILT.iter().map(|(n, w)| json!({"surface": n, "scope": w})).collect::<Vec<_>>(),
    });
    let code = emit(json_out, text, value);
    Ok(if all_ok { code } else { 1 })
}

#[cfg(test)]
mod key_ceremony_tests {
    use super::*;

    fn scratch_dir(tag: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("fleet-cert-{tag}-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&dir).expect("create isolated test directory");
        dir
    }

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn exported_spki_matches_the_certificate_engine_for_every_algorithm() {
        // Oracle: rcgen writes the SubjectPublicKeyInfo of every
        // certificate it signs; `public_key_spki_der` must produce the
        // same bytes from the public key `inspect_key` returns.
        for algo in [
            AlgorithmId::Ed25519,
            AlgorithmId::P256,
            AlgorithmId::Rsa2048,
            AlgorithmId::MlDsa44,
        ] {
            let key = engine::generate_key(algo).expect("generate");
            let cert = rcgen::CertificateParams::new(Vec::<String>::new())
                .expect("params")
                .self_signed(&key)
                .expect("self-signed certificate");
            let (_, parsed) =
                x509_parser::parse_x509_certificate(cert.der()).expect("parse certificate");
            let public = engine::inspect_key(algo, &key.serialize_der()).expect("inspect");
            assert_eq!(
                engine::public_key_spki_der(algo, &public),
                parsed.tbs_certificate.subject_pki.raw,
                "SPKI mismatch for {}",
                algo.info().name
            );
        }
    }

    #[test]
    fn import_takes_pem_or_der_pkcs8_and_refuses_the_wrong_algorithm() {
        let dir = scratch_dir("key-import");
        let key = engine::generate_key(AlgorithmId::Ed25519).expect("generate");
        let der = key.serialize_der();
        let pem_path = dir.join("external.pem");
        std::fs::write(&pem_path, crate::bundle::pem_encode("PRIVATE KEY", &der)).unwrap();
        let pem_text = pem_path.to_string_lossy().into_owned();

        // A wrong algorithm is refused and nothing is written.
        let wrong = dir.join("wrong.pk8");
        let wrong_text = wrong.to_string_lossy().into_owned();
        assert!(
            key_cmd(
                &args(&[
                    "import",
                    "--algorithm",
                    "p256",
                    "--in",
                    &pem_text,
                    "--out",
                    &wrong_text
                ]),
                true,
            )
            .is_err()
        );
        assert!(!wrong.exists());

        // The PEM block imports as the same PKCS#8 DER, owner-only.
        let out = dir.join("imported.pk8");
        let out_text = out.to_string_lossy().into_owned();
        assert!(
            key_cmd(
                &args(&[
                    "import",
                    "--algorithm",
                    "ed25519",
                    "--in",
                    &pem_text,
                    "--out",
                    &out_text
                ]),
                true,
            )
            .is_ok()
        );
        assert_eq!(std::fs::read(&out).unwrap(), der);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&out).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        // An existing destination is never replaced.
        assert!(
            key_cmd(
                &args(&[
                    "import",
                    "--algorithm",
                    "ed25519",
                    "--in",
                    &pem_text,
                    "--out",
                    &out_text
                ]),
                true,
            )
            .is_err()
        );

        // Raw DER imports too; a legacy PEM label is refused by name.
        let der_path = dir.join("external.der");
        std::fs::write(&der_path, &der).unwrap();
        let der_out = dir.join("from-der.pk8");
        assert!(
            key_cmd(
                &args(&[
                    "import",
                    "--algorithm",
                    "ed25519",
                    "--in",
                    &der_path.to_string_lossy(),
                    "--out",
                    &der_out.to_string_lossy(),
                ]),
                true,
            )
            .is_ok()
        );
        assert_eq!(std::fs::read(&der_out).unwrap(), der);
        let legacy = dir.join("legacy.pem");
        std::fs::write(&legacy, crate::bundle::pem_encode("EC PRIVATE KEY", &der)).unwrap();
        let err = pkcs8_from_der_or_pem(&std::fs::read(&legacy).unwrap(), &legacy)
            .expect_err("legacy label refused");
        assert!(err.to_string().contains("EC PRIVATE KEY"));
        std::fs::remove_dir_all(dir).expect("remove isolated test directory");
    }

    #[test]
    fn format_convert_and_verify_round_trip_keys_for_every_algorithm() {
        // Oracle: the generated PKCS#8 DER bytes are the input identity;
        // converting DER -> PEM -> DER must reproduce those exact bytes.
        let dir = scratch_dir("format-key-roundtrip");
        // Match the bundle's documented key_path layout; convert/verify act
        // on the referenced key artifact without rewriting its manifest.
        let key_dir = dir.join("bundle/private/keys");
        std::fs::create_dir_all(&key_dir).expect("create bundle key path");
        for algorithm in [
            AlgorithmId::Ed25519,
            AlgorithmId::P256,
            AlgorithmId::Rsa2048,
            AlgorithmId::Rsa4096,
            AlgorithmId::MlDsa44,
        ] {
            let key = engine::generate_key(algorithm).expect("generate key");
            let expected_der = key.serialize_der();
            let input = key_dir.join(format!("{}.pk8", algorithm.name()));
            let pem = dir.join(format!("{}.pem", algorithm.name()));
            let round_trip = dir.join(format!("{}-roundtrip.pk8", algorithm.name()));
            std::fs::write(&input, &expected_der).expect("write source key");

            let input_text = input.to_string_lossy().into_owned();
            let pem_text = pem.to_string_lossy().into_owned();
            let round_trip_text = round_trip.to_string_lossy().into_owned();
            let algorithm_name = algorithm.name();
            format_cmd(
                &args(&[
                    "convert",
                    "--in",
                    &input_text,
                    "--to",
                    "pem",
                    "--algorithm",
                    algorithm_name,
                    "--out",
                    &pem_text,
                ]),
                true,
            )
            .unwrap_or_else(|error| panic!("{} DER-to-PEM failed: {error}", algorithm.name()));

            let pem_bytes = std::fs::read(&pem).expect("read PEM output");
            assert!(pem_bytes.starts_with(b"-----BEGIN PRIVATE KEY-----"));
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    std::fs::metadata(&pem)
                        .expect("PEM metadata")
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600,
                    "private-key conversion output must be owner-only"
                );
            }
            format_cmd(
                &args(&["verify", "--in", &pem_text, "--algorithm", algorithm_name]),
                true,
            )
            .unwrap_or_else(|error| panic!("{} PEM verify failed: {error}", algorithm.name()));
            format_cmd(
                &args(&[
                    "convert",
                    "--in",
                    &pem_text,
                    "--to",
                    "der",
                    "--algorithm",
                    algorithm_name,
                    "--out",
                    &round_trip_text,
                ]),
                true,
            )
            .unwrap_or_else(|error| panic!("{} PEM-to-DER failed: {error}", algorithm.name()));
            let round_trip_der = std::fs::read(&round_trip).expect("read DER output");
            assert_eq!(round_trip_der, expected_der, "{}", algorithm.name());
            format_cmd(
                &args(&[
                    "verify",
                    "--in",
                    &round_trip_text,
                    "--algorithm",
                    algorithm_name,
                ]),
                true,
            )
            .unwrap_or_else(|error| panic!("{} DER verify failed: {error}", algorithm.name()));
        }
        std::fs::remove_dir_all(dir).expect("remove isolated test directory");
    }

    #[test]
    fn format_key_conversion_rejects_malformed_trailing_and_mismatched_material() {
        let dir = scratch_dir("format-key-invalid");
        let key = engine::generate_key(AlgorithmId::Ed25519).expect("generate key");
        let der = key.serialize_der();
        let cases = [
            ("trailing.der", [der.clone(), vec![0]].concat(), "ed25519"),
            ("mismatch.der", der.clone(), "p256"),
            (
                "multi.pem",
                format!(
                    "{}{}",
                    crate::bundle::pem_encode("PRIVATE KEY", &der),
                    crate::bundle::pem_encode("PRIVATE KEY", &der)
                )
                .into_bytes(),
                "ed25519",
            ),
            (
                "text.pem",
                format!("{}trailing", crate::bundle::pem_encode("PRIVATE KEY", &der)).into_bytes(),
                "ed25519",
            ),
            (
                "legacy-label.pem",
                crate::bundle::pem_encode("RSA PRIVATE KEY", &der).into_bytes(),
                "ed25519",
            ),
            (
                "encrypted-label.pem",
                crate::bundle::pem_encode("ENCRYPTED PRIVATE KEY", &der).into_bytes(),
                "ed25519",
            ),
            (
                "leading-text.pem",
                format!("prefix{}", crate::bundle::pem_encode("PRIVATE KEY", &der)).into_bytes(),
                "ed25519",
            ),
        ];
        for (name, bytes, algorithm) in cases {
            let input = dir.join(name);
            let output = dir.join(format!("{name}.out"));
            std::fs::write(&input, bytes).expect("write invalid source");
            let input_text = input.to_string_lossy().into_owned();
            let output_text = output.to_string_lossy().into_owned();
            assert!(
                format_cmd(
                    &args(&[
                        "convert",
                        "--in",
                        &input_text,
                        "--to",
                        "pem",
                        "--algorithm",
                        algorithm,
                        "--out",
                        &output_text,
                    ]),
                    true,
                )
                .is_err(),
                "accepted invalid key case {name}"
            );
            assert!(!output.exists(), "wrote output for invalid key case {name}");
        }

        // RSA-2048 and RSA-4096 share a signature scheme, so enforce the
        // fleet-cert profile size as well as successful PKCS#8 parsing.
        let rsa_der = engine::generate_key(AlgorithmId::Rsa2048)
            .expect("generate RSA-2048")
            .serialize_der();
        let rsa_input = dir.join("rsa-2048.pk8");
        let rsa_output = dir.join("rsa-mismatch.pem");
        std::fs::write(&rsa_input, rsa_der).expect("write RSA-2048 key");
        assert!(
            format_cmd(
                &args(&[
                    "convert",
                    "--in",
                    &rsa_input.to_string_lossy(),
                    "--to",
                    "pem",
                    "--algorithm",
                    "rsa-4096",
                    "--out",
                    &rsa_output.to_string_lossy(),
                ]),
                true,
            )
            .is_err(),
            "accepted an RSA key with the wrong registered size"
        );
        assert!(!rsa_output.exists());

        let input = dir.join("valid.pk8");
        std::fs::write(&input, &der).expect("write valid key");
        assert!(
            format_cmd(
                &args(&[
                    "convert",
                    "--in",
                    &input.to_string_lossy(),
                    "--to",
                    "pem",
                    "--algorithm",
                    "ed25519",
                ]),
                true,
            )
            .is_err(),
            "private key conversion must require an explicit output path"
        );

        let existing = dir.join("existing.pem");
        let sentinel = b"do not replace this file";
        std::fs::write(&existing, sentinel).expect("write existing destination");
        assert!(
            format_cmd(
                &args(&[
                    "convert",
                    "--in",
                    &input.to_string_lossy(),
                    "--to",
                    "pem",
                    "--algorithm",
                    "ed25519",
                    "--out",
                    &existing.to_string_lossy(),
                ]),
                true,
            )
            .is_err(),
            "private key conversion must not replace an existing file"
        );
        assert_eq!(std::fs::read(&existing).unwrap(), sentinel);

        let cert_key = engine::generate_key(AlgorithmId::P256).expect("generate certificate key");
        let cert = rcgen::CertificateParams::new(Vec::<String>::new())
            .expect("certificate params")
            .self_signed(&cert_key)
            .expect("self-signed certificate");
        let cert_input = dir.join("not-a-private-key.der");
        let cert_output = dir.join("not-a-private-key.pem");
        std::fs::write(&cert_input, cert.der()).expect("write certificate");
        assert!(
            format_cmd(
                &args(&[
                    "convert",
                    "--in",
                    &cert_input.to_string_lossy(),
                    "--to",
                    "pem",
                    "--algorithm",
                    "p256",
                    "--out",
                    &cert_output.to_string_lossy(),
                ]),
                true,
            )
            .is_err(),
            "a certificate must not be accepted as private-key input"
        );
        assert!(!cert_output.exists());
        std::fs::remove_dir_all(dir).expect("remove isolated test directory");
    }

    #[test]
    fn format_jwk_export_verify_and_import_round_trip_for_supported_algorithms() {
        // Oracle: `inspect_key` public bytes are the input identity;
        // JWK export -> strict re-parse -> SPKI import must agree with
        // the engine's own SPKI encoding, and `verify --key` must pin
        // the JWK to the canonical DER key material.
        let dir = scratch_dir("format-jwk-roundtrip");
        for algorithm in [AlgorithmId::Ed25519, AlgorithmId::P256] {
            let name = algorithm.name();
            let key = engine::generate_key(algorithm).expect("generate key");
            let der = key.serialize_der();
            let key_path = dir.join(format!("{name}.pk8"));
            std::fs::write(&key_path, &der).expect("write source key");
            let key_text = key_path.to_string_lossy().into_owned();

            let jwk_path = dir.join(format!("{name}.jwk"));
            let jwk_text = jwk_path.to_string_lossy().into_owned();
            format_cmd(
                &args(&[
                    "convert",
                    "--to",
                    "jwk",
                    "--algorithm",
                    name,
                    "--in",
                    &key_text,
                    "--out",
                    &jwk_text,
                ]),
                true,
            )
            .unwrap_or_else(|error| panic!("{name} JWK export failed: {error}"));

            // Strict public-only subset: the stored value carries no
            // private member and re-parses to the engine's own public
            // bytes.
            let stored: Value =
                serde_json::from_slice(&std::fs::read(&jwk_path).expect("read JWK"))
                    .expect("exported JWK is a JSON object");
            assert!(stored.get("d").is_none(), "{name} JWK must not carry 'd'");
            let (parsed_algorithm, raw) =
                crate::jwk::public_to_raw(&stored).expect("strict JWK re-parse");
            assert_eq!(parsed_algorithm, algorithm);
            assert_eq!(raw, engine::inspect_key(algorithm, &der).expect("inspect"));

            // `verify --key` pins the JWK to the canonical DER key.
            format_cmd(
                &args(&[
                    "verify",
                    "--in",
                    &jwk_text,
                    "--algorithm",
                    name,
                    "--key",
                    &key_text,
                ]),
                true,
            )
            .unwrap_or_else(|error| panic!("{name} JWK verify failed: {error}"));

            // The same JWK verified against a different key is refused.
            let other = engine::generate_key(algorithm).expect("generate other key");
            let other_path = dir.join(format!("{name}-other.pk8"));
            std::fs::write(&other_path, other.serialize_der()).expect("write other key");
            assert!(
                format_cmd(
                    &args(&[
                        "verify",
                        "--in",
                        &jwk_text,
                        "--algorithm",
                        name,
                        "--key",
                        &other_path.to_string_lossy(),
                    ]),
                    true,
                )
                .is_err(),
                "{name} JWK must not verify against a different key"
            );

            // Import legs: JWK -> SPKI PEM and DER.
            let expected_spki = engine::public_key_spki_der(algorithm, &raw);
            let pem_path = dir.join(format!("{name}.pub.pem"));
            let pem_text = pem_path.to_string_lossy().into_owned();
            format_cmd(
                &args(&[
                    "convert", "--from", "jwk", "--to", "pem", "--in", &jwk_text, "--out",
                    &pem_text,
                ]),
                true,
            )
            .unwrap_or_else(|error| panic!("{name} JWK-to-PEM failed: {error}"));
            let pem_bytes = std::fs::read(&pem_path).expect("read PEM output");
            assert!(pem_bytes.starts_with(b"-----BEGIN PUBLIC KEY-----"));
            assert!(!pem_bytes.windows(7).any(|w| w == b"PRIVATE"));

            let der_path = dir.join(format!("{name}.pub.der"));
            let der_text = der_path.to_string_lossy().into_owned();
            format_cmd(
                &args(&[
                    "convert", "--from", "jwk", "--to", "der", "--in", &jwk_text, "--out",
                    &der_text,
                ]),
                true,
            )
            .unwrap_or_else(|error| panic!("{name} JWK-to-DER failed: {error}"));
            assert_eq!(
                std::fs::read(&der_path).expect("read DER output"),
                expected_spki
            );
        }
        std::fs::remove_dir_all(dir).expect("remove isolated test directory");
    }

    #[test]
    fn format_jwk_export_reads_the_bundles_exactly_one_identity() {
        // Oracle: a single-algorithm root set with no cross-signing
        // holds exactly one private-key identity (§3.2 arithmetic);
        // the `--bundle` leg must find it without `--in`, and the
        // exported JWK must verify against the bundle's own key file.
        let dir = scratch_dir("format-jwk-bundle");
        let bundle_dir = dir.join("roots");
        let manifest = engine::create_root_set(
            &RootCreateRequest {
                organization: "Test Org LLC".into(),
                country: "US".into(),
                algorithms: vec![AlgorithmId::Ed25519],
                generation: 1,
                partition: "A".into(),
                cross_signing: CrossSigningMode::None,
                not_before: Rfc3339::parse("2026-01-01T00:00:00Z").expect("not-before"),
                not_after: Rfc3339::parse("2046-01-01T00:00:00Z").expect("not-after"),
                allow_legacy_root_strength: false,
                private_store: None,
            },
            &bundle_dir,
        )
        .expect("create single-algorithm root set");
        assert_eq!(manifest.certificates.len(), 1);
        let bundle_text = bundle_dir.to_string_lossy().into_owned();

        let jwk_path = dir.join("bundle-ed25519.jwk");
        let jwk_text = jwk_path.to_string_lossy().into_owned();
        format_cmd(
            &args(&[
                "convert",
                "--to",
                "jwk",
                "--algorithm",
                "ed25519",
                "--bundle",
                &bundle_text,
                "--out",
                &jwk_text,
            ]),
            true,
        )
        .expect("bundle JWK export failed");
        let bundle = load_bundle(&bundle_dir).expect("reload bundle");
        let key_path = bundle.manifest.certificates[0]
            .key_path
            .as_deref()
            .expect("root identity keeps its key in the bundle");
        format_cmd(
            &args(&[
                "verify",
                "--in",
                &jwk_text,
                "--algorithm",
                "ed25519",
                "--key",
                &bundle_dir.join(key_path).to_string_lossy(),
            ]),
            true,
        )
        .expect("bundle JWK must verify against the bundle key");

        // No P-256 identity lives in this bundle: the same export
        // with a non-resident algorithm is refused, not guessed.
        let missing = dir.join("bundle-p256.jwk");
        assert!(
            format_cmd(
                &args(&[
                    "convert",
                    "--to",
                    "jwk",
                    "--algorithm",
                    "p256",
                    "--bundle",
                    &bundle_text,
                    "--out",
                    &missing.to_string_lossy(),
                ]),
                true,
            )
            .is_err(),
            "bundle JWK export must refuse a non-resident algorithm"
        );
        assert!(!missing.exists());
        std::fs::remove_dir_all(dir).expect("remove isolated test directory");
    }

    #[test]
    fn format_jwk_rejects_unsupported_algorithms_private_material_and_bad_flags() {
        let dir = scratch_dir("format-jwk-invalid");
        // RSA-2048, RSA-4096, and ML-DSA-44 have no registered public
        // JWK form: export is refused and nothing is written.
        for algorithm in [
            AlgorithmId::Rsa2048,
            AlgorithmId::Rsa4096,
            AlgorithmId::MlDsa44,
        ] {
            let name = algorithm.name();
            let key = engine::generate_key(algorithm).expect("generate key");
            let key_path = dir.join(format!("{name}.pk8"));
            std::fs::write(&key_path, key.serialize_der()).expect("write key");
            let out = dir.join(format!("{name}.jwk"));
            assert!(
                format_cmd(
                    &args(&[
                        "convert",
                        "--to",
                        "jwk",
                        "--algorithm",
                        name,
                        "--in",
                        &key_path.to_string_lossy(),
                        "--out",
                        &out.to_string_lossy(),
                    ]),
                    true,
                )
                .is_err(),
                "{name} JWK export must be refused"
            );
            assert!(!out.exists(), "{name} JWK export must write nothing");
        }

        let ed = engine::generate_key(AlgorithmId::Ed25519).expect("generate key");
        let ed_der = ed.serialize_der();
        let ed_path = dir.join("ed25519.pk8");
        std::fs::write(&ed_path, &ed_der).expect("write key");
        let ed_text = ed_path.to_string_lossy().into_owned();
        let jwk_path = dir.join("ed25519.jwk");
        let jwk_text = jwk_path.to_string_lossy().into_owned();
        format_cmd(
            &args(&[
                "convert",
                "--to",
                "jwk",
                "--algorithm",
                "ed25519",
                "--in",
                &ed_text,
                "--out",
                &jwk_text,
            ]),
            true,
        )
        .expect("baseline JWK export");

        // `--algorithm` is required: the key bytes alone do not name
        // the JWK family.
        let no_algo = dir.join("no-algo.jwk");
        assert!(
            format_cmd(
                &args(&[
                    "convert",
                    "--to",
                    "jwk",
                    "--in",
                    &ed_text,
                    "--out",
                    &no_algo.to_string_lossy(),
                ]),
                true,
            )
            .is_err(),
            "JWK export must require --algorithm"
        );
        assert!(!no_algo.exists());

        // `--password-file` is meaningless for public-only JWK output.
        let with_password = dir.join("with-password.jwk");
        assert!(
            format_cmd(
                &args(&[
                    "convert",
                    "--to",
                    "jwk",
                    "--algorithm",
                    "ed25519",
                    "--in",
                    &ed_text,
                    "--out",
                    &with_password.to_string_lossy(),
                    "--password-file",
                    &ed_text,
                ]),
                true,
            )
            .is_err(),
            "JWK export must refuse --password-file"
        );
        assert!(!with_password.exists());

        // `--from jwk` import takes neither `--bundle` nor
        // `--password-file`.
        for flag in ["--bundle", "--password-file"] {
            assert!(
                format_cmd(
                    &args(&[
                        "convert",
                        "--from",
                        "jwk",
                        "--to",
                        "pem",
                        "--in",
                        &jwk_text,
                        "--out",
                        &dir.join("flagged.pem").to_string_lossy(),
                        flag,
                        &ed_text,
                    ]),
                    true,
                )
                .is_err(),
                "--from jwk must refuse {flag}"
            );
        }

        // A JWK carrying the private member `d` is refused at verify
        // and at import: private material never crosses the JWK legs.
        let mut smuggled: Value =
            serde_json::from_slice(&std::fs::read(&jwk_path).expect("read JWK"))
                .expect("JWK is a JSON object");
        smuggled["d"] = Value::from("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
        let smuggled_path = dir.join("smuggled.jwk");
        std::fs::write(
            &smuggled_path,
            serde_json::to_string_pretty(&smuggled).expect("render JWK"),
        )
        .expect("write smuggled JWK");
        let smuggled_text = smuggled_path.to_string_lossy().into_owned();
        assert!(
            format_cmd(
                &args(&[
                    "verify",
                    "--in",
                    &smuggled_text,
                    "--algorithm",
                    "ed25519",
                    "--key",
                    &ed_text,
                ]),
                true,
            )
            .is_err(),
            "verify must refuse a JWK carrying 'd'"
        );
        assert!(
            format_cmd(
                &args(&[
                    "convert",
                    "--from",
                    "jwk",
                    "--to",
                    "pem",
                    "--in",
                    &smuggled_text,
                    "--out",
                    &dir.join("smuggled.pem").to_string_lossy(),
                ]),
                true,
            )
            .is_err(),
            "import must refuse a JWK carrying 'd'"
        );

        // Verifying a JWK without `--key` cannot prove byte-faithfulness.
        assert!(
            format_cmd(
                &args(&["verify", "--in", &jwk_text, "--algorithm", "ed25519"]),
                true,
            )
            .is_err(),
            "JWK verify must require --key"
        );
        std::fs::remove_dir_all(dir).expect("remove isolated test directory");
    }

    #[test]
    fn rotate_writes_a_fresh_key_and_keeps_the_old_one() {
        let dir = scratch_dir("key-rotate");
        for (name, algorithm) in [
            ("ed25519", AlgorithmId::Ed25519),
            ("p256", AlgorithmId::P256),
        ] {
            let old = engine::generate_key(algorithm).expect("generate");
            let old_der = old.serialize_der();
            let old_path = dir.join(format!("{name}-g1.pk8"));
            std::fs::write(&old_path, &old_der).unwrap();
            let old_text = old_path.to_string_lossy().into_owned();
            let new_path = dir.join(format!("{name}-g2.pk8"));
            let new_text = new_path.to_string_lossy().into_owned();
            let rotate = |out: &str| {
                key_cmd(
                    &args(&[
                        "rotate",
                        "--algorithm",
                        name,
                        "--in",
                        &old_text,
                        "--out",
                        out,
                    ]),
                    true,
                )
            };
            assert!(rotate(&new_text).is_ok());
            // The old key file is untouched.
            assert_eq!(std::fs::read(&old_path).unwrap(), old_der);
            // The successor loads under the same algorithm and is a
            // different key.
            let new_der = std::fs::read(&new_path).unwrap();
            let old_public = engine::inspect_key(algorithm, &old_der).unwrap();
            let new_public = engine::inspect_key(algorithm, &new_der).unwrap();
            assert_ne!(old_public, new_public);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    std::fs::metadata(&new_path).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
            // An existing destination is never replaced, the old path
            // included.
            assert!(rotate(&new_text).is_err());
            assert_eq!(std::fs::read(&new_path).unwrap(), new_der);
            assert!(rotate(&old_text).is_err());
            assert_eq!(std::fs::read(&old_path).unwrap(), old_der);
        }
        // A key that does not load under the named algorithm is refused
        // and nothing is written.
        let ed = engine::generate_key(AlgorithmId::Ed25519).expect("generate");
        let ed_path = dir.join("ed-as-p256.pk8");
        std::fs::write(&ed_path, ed.serialize_der()).unwrap();
        let refused = dir.join("never-written.pk8");
        assert!(
            key_cmd(
                &args(&[
                    "rotate",
                    "--algorithm",
                    "p256",
                    "--in",
                    &ed_path.to_string_lossy(),
                    "--out",
                    &refused.to_string_lossy(),
                ]),
                true,
            )
            .is_err()
        );
        assert!(!refused.exists());
        std::fs::remove_dir_all(dir).expect("remove isolated test directory");
    }

    #[test]
    fn export_writes_only_the_public_key_and_never_replaces_a_file() {
        let dir = scratch_dir("key-export");
        let key = engine::generate_key(AlgorithmId::P256).expect("generate");
        let der = key.serialize_der();
        let key_path = dir.join("p256.pk8");
        std::fs::write(&key_path, &der).unwrap();
        let key_text = key_path.to_string_lossy().into_owned();
        let public = engine::inspect_key(AlgorithmId::P256, &der).unwrap();
        let spki = engine::public_key_spki_der(AlgorithmId::P256, &public);

        let pem_out = dir.join("p256.pub.pem");
        let pem_text = pem_out.to_string_lossy().into_owned();
        assert!(
            key_cmd(
                &args(&[
                    "export",
                    "--algorithm",
                    "p256",
                    "--in",
                    &key_text,
                    "--out",
                    &pem_text
                ]),
                true,
            )
            .is_ok()
        );
        let written = std::fs::read_to_string(&pem_out).unwrap();
        let blocks = pem_decode_all(&written).unwrap();
        assert_eq!(blocks, vec![("PUBLIC KEY".to_string(), spki.clone())]);
        assert!(!written.contains("PRIVATE"));
        assert!(
            key_cmd(
                &args(&[
                    "export",
                    "--algorithm",
                    "p256",
                    "--in",
                    &key_text,
                    "--out",
                    &pem_text
                ]),
                true,
            )
            .is_err()
        );

        let der_out = dir.join("p256.pub.der");
        assert!(
            key_cmd(
                &args(&[
                    "export",
                    "--algorithm",
                    "p256",
                    "--in",
                    &key_text,
                    "--out",
                    &der_out.to_string_lossy(),
                    "--format",
                    "der",
                ]),
                true,
            )
            .is_ok()
        );
        assert_eq!(std::fs::read(&der_out).unwrap(), spki);
        assert!(
            key_cmd(
                &args(&[
                    "export",
                    "--algorithm",
                    "p256",
                    "--in",
                    &key_text,
                    "--out",
                    &dir.join("x").to_string_lossy(),
                    "--format",
                    "jwk",
                ]),
                true,
            )
            .is_err()
        );
        std::fs::remove_dir_all(dir).expect("remove isolated test directory");
    }

    #[test]
    fn generate_creates_private_file_once_and_inspect_redacts_material() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("fleet-cert-key-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&dir).expect("create isolated test directory");
        let path = dir.join("p256.pk8");
        let path_text = path.to_string_lossy().into_owned();
        let generated = key_cmd(
            &[
                "generate".into(),
                "--algorithm".into(),
                "p256".into(),
                "--out".into(),
                path_text.clone(),
            ],
            true,
        );
        assert!(generated.is_ok());
        let bytes = std::fs::read(&path).expect("generated PKCS#8 key");
        assert!(!bytes.is_empty());
        assert!(engine::inspect_key(AlgorithmId::P256, &bytes).is_ok());
        assert!(
            key_cmd(
                &[
                    "inspect".into(),
                    "--algorithm".into(),
                    "p256".into(),
                    "--in".into(),
                    path_text.clone(),
                ],
                true,
            )
            .is_ok()
        );
        assert!(
            key_cmd(
                &[
                    "generate".into(),
                    "--algorithm".into(),
                    "p256".into(),
                    "--out".into(),
                    path_text,
                ],
                true,
            )
            .is_err()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::remove_dir_all(dir).expect("remove isolated test directory");
    }
}

// ---------------------------------------------------------------- ocsp

fn ocsp_cmd(args: &[String], json_out: bool) -> Result<i32> {
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
fn cert_from_record(bundle: &crate::bundle::Bundle, record: &CertRecord) -> Result<CertInfo> {
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
fn resolve_issuer_record(
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
fn issuer_key_file(record: &CertRecord) -> String {
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

fn parse_algorithm_flag(p: &Parsed, flag: &str) -> Result<Option<AlgorithmId>> {
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

fn ocsp_sign_cmd(rest: &[String], json_out: bool) -> Result<i32> {
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

fn read_response_bytes(path: &str) -> Result<Vec<u8>> {
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

fn ocsp_inspect_cmd(rest: &[String], json_out: bool) -> Result<i32> {
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

fn ocsp_test_cmd(rest: &[String], json_out: bool) -> Result<i32> {
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
