//! The `fleet cert` CLI: a thin shell over the crate API (spec §6.4).
//! Verbs parse flags into typed requests, call the library, and
//! render either human text or, with `--json`, one JSON value on
//! stdout. This first slice lands the registry surface
//! (`algorithms`); the issuance verbs land in the following commits
//! of the same slice.

use crate::algorithm;
use crate::error::{CertError, Result};
use serde_json::{json, Value};

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

fn wants_help(args: &[String]) -> bool {
    args.iter().any(|a| a == "--help" || a == "-h" || a == "help")
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

fn run_inner(args: &[String]) -> Result<i32> {
    if args.is_empty() || wants_help(args) && args.len() == 1 {
        print!("{CERT_USAGE}");
        return Ok(if args.is_empty() { 2 } else { 0 });
    }
    let json_out = json_mode(args);
    match args[0].as_str() {
        "algorithms" => algorithms_cmd(&args[1..], json_out),
        other => Err(CertError::usage(format!(
            "unknown cert group '{other}' — run 'fleet cert --help' for the surface"
        ))),
    }
}

fn algorithms_cmd(args: &[String], json_out: bool) -> Result<i32> {
    let verb = args.first().map(|s| s.as_str()).unwrap_or("list");
    match verb {
        "list" => {
            let reg = algorithm::registry();
            let mut text = String::from("Registered algorithms (spec §3.1/§24.1):\n");
            for info in &reg {
                text.push_str(&format!(
                    "  {:10} {:9} support={:?} policy={:?} sig-oid={}\n",
                    info.name,
                    info.display_name,
                    info.support,
                    info.policy,
                    info.signature_oid
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
            let name = args.get(1).ok_or_else(|| {
                CertError::usage("algorithms describe needs an algorithm name")
            })?;
            let (algo, warn) = algorithm::AlgorithmId::parse(name)?;
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
