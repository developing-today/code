//! `fleet cert dns plan | render | verify` — offline CAA and
//! TLSA/DANE planning (spec §6.1 tree, §4.4 record shape, §24.8 CAA
//! semantics), built against the contract in
//! `doc/2026-11-25T_cert_dns_caa_dane_plan.md` (#13724).
//!
//! Every function here is offline by construction: inputs are local
//! bundle directories, a local policy file and an optional recorded
//! snapshot file; outputs are values the caller may write locally.
//! Nothing opens a socket, queries a resolver or talks to a DNS
//! provider, and no argument anywhere names a provider, endpoint or
//! credential. Live apply and live proof are behind the #9155
//! production gate and are not part of this module.
//!
//! The three states the contract keeps apart (§5): a change-set that
//! passes [`verify`] is **proposed**. Published and proven are
//! outside this module, and no output here claims either.

use crate::bundle::{Bundle, load_bundle, pem_decode_all, read_artifact};
use crate::chain::Check;
use crate::error::{CertError, Result};
use crate::timeparse::Rfc3339;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Schema id of a DNS change-set (contract §3).
pub const DNS_SCHEMA: &str = "fleet.pki.dns/v1";

/// Schema id of the planner's input policy file.
pub const DNS_POLICY_SCHEMA: &str = "fleet.pki.dns-policy/v1";

/// Default TTL (contract §3, the §4.4 example).
pub const DEFAULT_TTL: u32 = 3600;

/// RFC 2181 §8: a TTL is an unsigned 31-bit value. Zero is refused
/// here because a zero-TTL CAA/TLSA record is never what a plan means.
pub const MAX_TTL: u32 = 2_147_483_647;

/// The prerequisite every change-set with a TLSA record carries (§4).
pub const DNSSEC_PREREQUISITE: &str = "The zone is DNSSEC-signed and the signing is maintained: a TLSA record in an unsigned zone is a hint any on-path attacker can rewrite, so these records stay proposed until a DNSSEC-validating read-back proves them (contract §4, §5).";

/// The prerequisite that pins the corrected CAA premise (§1).
pub const FLEET_CA_CAA_NOTE: &str = "No CAA record names the Fleet CA: CAA constrains publicly trusted issuers only, and a private CA is neither a CAA checker nor bound by CAA (spec §24.8). CAA records appear only for names that also take public ACME certificates.";

/// Record actions (contract §3).
pub const ACTIONS: &[&str] = &["create", "conditional-create", "no-op", "delete"];

/// One DNS record proposal. The first five fields are exactly the
/// landed [`crate::publish::DnsRecord`] shape, so a publish-plan
/// consumer reads these unchanged.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DnsChange {
    #[serde(rename = "type")]
    pub record_type: String,
    pub name: String,
    pub ttl: u32,
    pub value: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl DnsChange {
    /// The identity a snapshot diff compares on: type, owner name and
    /// rdata (case-folded where DNS folds case). TTL is not identity.
    fn identity(&self) -> (String, String, String) {
        (
            self.record_type.to_ascii_uppercase(),
            self.name.to_ascii_lowercase(),
            normalize_rdata(&self.record_type, &self.value),
        )
    }
}

/// The versioned change-set (contract §3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeSet {
    pub schema: String,
    pub zone: String,
    pub generated_at: Rfc3339,
    pub source_bundles: Vec<String>,
    pub records: Vec<DnsChange>,
    pub prerequisites: Vec<String>,
}

impl ChangeSet {
    /// True when every record is a no-op: the plan proposes nothing.
    pub fn is_no_change(&self) -> bool {
        self.records.iter().all(|r| r.action == "no-op")
    }
}

/// Planner policy input (`--policy`, schema [`DNS_POLICY_SCHEMA`]).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DnsPolicy {
    #[serde(default)]
    pub schema: Option<String>,
    /// Public names that also take public ACME certificates (§24.8).
    #[serde(default)]
    pub caa: Vec<CaaPolicy>,
    /// Service endpoints whose TLS key is pinned with TLSA.
    #[serde(default)]
    pub tlsa: Vec<TlsaPolicy>,
}

/// CAA input for one owner name.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaaPolicy {
    pub name: String,
    /// Public issuer domains allowed to issue (`issue`).
    #[serde(default)]
    pub issue: Vec<String>,
    /// Public issuer domains allowed to issue wildcards (`issuewild`).
    #[serde(default)]
    pub issuewild: Vec<String>,
    /// Optional incident-report URL (`iodef`, `mailto:` or `https:`).
    #[serde(default)]
    pub iodef: Option<String>,
}

/// TLSA input for one endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TlsaPolicy {
    pub host: String,
    pub port: u16,
    #[serde(default = "default_proto")]
    pub proto: String,
    /// Bundle id, name or slug of one of the `--bundle` inputs.
    /// Optional when exactly one supplied bundle can serve the usage.
    #[serde(default)]
    pub bundle: Option<String>,
    /// 3 = DANE-EE (default), 2 = DANE-TA.
    #[serde(default = "default_usage")]
    pub usage: u8,
    /// 1 = SPKI (default), 0 = full certificate.
    #[serde(default = "default_selector")]
    pub selector: u8,
    /// 1 = SHA-256 (default), 2 = SHA-512.
    #[serde(default = "default_matching")]
    pub matching: u8,
    /// Narrow the certificate: its SHA-256 fingerprint (hex).
    #[serde(default)]
    pub fingerprint: Option<String>,
    /// Narrow the certificate: key algorithm id (e.g. `ed25519`).
    #[serde(default)]
    pub algorithm: Option<String>,
    /// Narrow the certificate: key partition (e.g. `a`).
    #[serde(default)]
    pub partition: Option<String>,
    /// Who terminates TLS at this name (recorded in the note so review
    /// can catch a CDN or tunnel in front of the key, §4).
    #[serde(default)]
    pub endpoint: Option<String>,
}

fn default_proto() -> String {
    "tcp".to_string()
}
fn default_usage() -> u8 {
    3
}
fn default_selector() -> u8 {
    1
}
fn default_matching() -> u8 {
    1
}

/// `dns plan` inputs.
#[derive(Debug, Clone)]
pub struct DnsPlanRequest {
    pub zone: String,
    pub bundles: Vec<PathBuf>,
    pub policy: Option<PathBuf>,
    pub current: Option<PathBuf>,
    pub ttl: Option<u32>,
}

/// `dns verify` report: one named check per item, the
/// [`crate::publish::PublishVerifyReport`] shape plus the state the
/// change-set reached.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsVerifyReport {
    pub change_set: String,
    pub zone: String,
    pub valid: bool,
    /// `proposed` when valid, `invalid` otherwise. Never `published`
    /// or `proven`: those states need live evidence (§5).
    pub state: String,
    pub checks: Vec<Check>,
}

// ---------------------------------------------------------------- names

/// Normalize a zone apex: lowercase, no trailing dot.
pub fn normalize_zone(zone: &str) -> Result<String> {
    let z = zone.trim().trim_end_matches('.').to_ascii_lowercase();
    if !is_hostname(&z, false) {
        return Err(CertError::usage(format!(
            "--zone '{zone}' is not a well-formed DNS name"
        )));
    }
    Ok(z)
}

/// Absolute owner name (trailing dot) for `name` under `zone`.
/// `name` may be absolute or relative to the zone, or `@` for the apex.
fn owner_in_zone(name: &str, zone: &str) -> Result<String> {
    let n = name.trim().to_ascii_lowercase();
    let fq = if n == "@" || n.is_empty() {
        zone.to_string()
    } else if let Some(abs) = n.strip_suffix('.') {
        abs.to_string()
    } else if n == zone || n.ends_with(&format!(".{zone}")) {
        n
    } else {
        format!("{n}.{zone}")
    };
    if !(fq == zone || fq.ends_with(&format!(".{zone}"))) {
        return Err(CertError::validation(format!(
            "record name '{name}' is outside zone '{zone}'"
        )));
    }
    if !is_hostname(&fq, true) {
        return Err(CertError::validation(format!(
            "record name '{name}' is not a well-formed DNS name"
        )));
    }
    Ok(format!("{fq}."))
}

/// RFC 1035/1123 label rules; `allow_underscore` admits the
/// `_port._proto` labels TLSA owner names use.
fn is_hostname(name: &str, allow_underscore: bool) -> bool {
    let name = name.strip_suffix('.').unwrap_or(name);
    if name.is_empty() || name.len() > 253 {
        return false;
    }
    name.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| {
                c.is_ascii_lowercase()
                    || c.is_ascii_digit()
                    || c == '-'
                    || (allow_underscore && c == '_')
            })
    })
}

/// A public CAA issuer domain (RFC 8659 §4.2 issuer-domain-name):
/// at least two labels, no underscores.
fn is_issuer_domain(value: &str) -> bool {
    let v = value.to_ascii_lowercase();
    v.contains('.') && is_hostname(&v, false)
}

/// TLSA owner name for an endpoint (RFC 6698 §3).
pub fn tlsa_owner(port: u16, proto: &str, host_fqdn: &str) -> String {
    format!("_{port}._{}.{host_fqdn}", proto.to_ascii_lowercase())
}

// ---------------------------------------------------------------- CAA

/// What names the private Fleet CA, for the §1 refusal: the generic
/// tokens and every supplied bundle's organization, name, slug and id.
fn fleet_ca_names(bundles: &[Bundle]) -> BTreeSet<String> {
    let mut names: BTreeSet<String> = ["fleet", "fleet-ca", "fleet ca", "fleetca", "fleet.ca"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    for b in bundles {
        let m = &b.manifest;
        for s in [&m.organization, &m.name, &m.slug, &m.bundle_id] {
            names.insert(s.trim().to_ascii_lowercase());
        }
    }
    names
}

/// Refuse a CAA issuer value that names the private Fleet CA or is
/// not a public issuer domain (contract §1).
fn check_caa_issuer(value: &str, fleet_names: &BTreeSet<String>) -> std::result::Result<(), String> {
    let v = value.trim().to_ascii_lowercase();
    if fleet_names.contains(&v) {
        return Err(format!(
            "CAA issuer '{value}' names the private Fleet CA. CAA constrains publicly trusted \
             issuers only; a private CA is not bound by CAA, so such a record would authorize \
             and forbid nothing (contract §1, spec §24.8). Leave the Fleet CA out of CAA."
        ));
    }
    if v == ";" {
        // RFC 8659 §4.2: an empty issuer value forbids all issuance.
        return Ok(());
    }
    if !is_issuer_domain(&v) {
        return Err(format!(
            "CAA issuer '{value}' is not a public issuer domain (e.g. letsencrypt.org); \
             CAA names only publicly trusted issuers (contract §1)"
        ));
    }
    Ok(())
}

fn caa_records(
    policy: &CaaPolicy,
    zone: &str,
    ttl: u32,
    fleet_names: &BTreeSet<String>,
) -> Result<Vec<DnsChange>> {
    let owner = owner_in_zone(&policy.name, zone)?;
    let mut out = Vec::new();
    let note = Some(
        "Pins public ACME issuance for this name to the listed public issuers (spec §24.8)."
            .to_string(),
    );
    for (tag, list) in [("issue", &policy.issue), ("issuewild", &policy.issuewild)] {
        for issuer in list {
            check_caa_issuer(issuer, fleet_names).map_err(CertError::validation)?;
            out.push(DnsChange {
                record_type: "CAA".into(),
                name: owner.clone(),
                ttl,
                value: format!("0 {tag} \"{}\"", issuer.trim().to_ascii_lowercase()),
                action: String::new(),
                requires: vec![],
                note: note.clone(),
            });
        }
    }
    if let Some(iodef) = &policy.iodef {
        let i = iodef.trim();
        if !(i.starts_with("mailto:") || i.starts_with("https://")) || i.contains('"') {
            return Err(CertError::validation(format!(
                "CAA iodef '{iodef}' must be a mailto: or https:// URL (RFC 8659 §4.4)"
            )));
        }
        out.push(DnsChange {
            record_type: "CAA".into(),
            name: owner.clone(),
            ttl,
            value: format!("0 iodef \"{i}\""),
            action: String::new(),
            requires: vec![],
            note: note.clone(),
        });
    }
    if policy.issue.is_empty() && policy.issuewild.is_empty() {
        return Err(CertError::validation(format!(
            "CAA policy for '{}' lists no public issuer; use [\";\"] to forbid all public issuance",
            policy.name
        )));
    }
    Ok(out)
}

// ---------------------------------------------------------------- TLSA

/// One certificate a TLSA association could pin.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub bundle_id: String,
    pub role: String,
    pub algorithm: String,
    pub partition: String,
    pub primary: bool,
    pub fingerprint_sha256: String,
    pub der: Vec<u8>,
}

/// Every certificate in a loaded bundle, DER-decoded from its PEM.
pub fn bundle_candidates(bundle: &Bundle) -> Result<Vec<Candidate>> {
    let mut out = Vec::new();
    for rec in &bundle.manifest.certificates {
        let pem = read_artifact(bundle, &rec.cert_path)?;
        let text = String::from_utf8(pem).map_err(|e| {
            CertError::manifest(format!("{} is not UTF-8 PEM: {e}", rec.cert_path))
        })?;
        let blocks = pem_decode_all(&text)?;
        let der = blocks
            .into_iter()
            .find(|(label, _)| label == "CERTIFICATE")
            .map(|(_, der)| der)
            .ok_or_else(|| {
                CertError::manifest(format!("{} holds no CERTIFICATE block", rec.cert_path))
            })?;
        out.push(Candidate {
            bundle_id: bundle.manifest.bundle_id.clone(),
            role: rec.role.clone(),
            algorithm: rec.algorithm.info().name.to_string(),
            partition: rec.partition.to_ascii_lowercase(),
            primary: rec.primary,
            fingerprint_sha256: rec.fingerprint_sha256.to_ascii_lowercase(),
            der,
        });
    }
    Ok(out)
}

/// The SubjectPublicKeyInfo DER of a certificate.
pub fn spki_der(cert_der: &[u8]) -> Result<Vec<u8>> {
    let (_, cert) = x509_parser::prelude::parse_x509_certificate(cert_der)
        .map_err(|e| CertError::validation(format!("certificate does not parse: {e}")))?;
    Ok(cert.tbs_certificate.subject_pki.raw.to_vec())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The TLSA certificate association data, lowercase hex (RFC 6698 §2.1).
pub fn association(cert_der: &[u8], selector: u8, matching: u8) -> Result<String> {
    let selected = match selector {
        0 => cert_der.to_vec(),
        1 => spki_der(cert_der)?,
        other => {
            return Err(CertError::validation(format!(
                "TLSA selector {other} is not 0 (full certificate) or 1 (SPKI)"
            )));
        }
    };
    match matching {
        1 => Ok(hex(&Sha256::digest(&selected))),
        2 => Ok(hex(&Sha512::digest(&selected))),
        0 => Err(CertError::validation(
            "TLSA matching type 0 (exact match) is not planned in v1; use 1 (SHA-256) or 2 (SHA-512)",
        )),
        other => Err(CertError::validation(format!(
            "TLSA matching type {other} is not 1 (SHA-256) or 2 (SHA-512)"
        ))),
    }
}

/// Which certificate roles a TLSA usage pins.
fn roles_for_usage(usage: u8) -> Result<&'static [&'static str]> {
    match usage {
        3 => Ok(&["leaf"]),
        2 => Ok(&["issuer", "root"]),
        0 | 1 => Err(CertError::validation(format!(
            "TLSA usage {usage} (PKIX) is deferred in v1: it layers WebPKI path validation over \
             DANE (contract §4). Use 3 (DANE-EE) or 2 (DANE-TA)."
        ))),
        other => Err(CertError::validation(format!(
            "TLSA usage {other} is not a defined certificate usage"
        ))),
    }
}

fn bundle_matches(b: &Bundle, key: &str) -> bool {
    let k = key.trim().to_ascii_lowercase();
    let m = &b.manifest;
    [&m.bundle_id, &m.name, &m.slug]
        .iter()
        .any(|s| s.to_ascii_lowercase() == k)
}

fn tlsa_record(
    policy: &TlsaPolicy,
    zone: &str,
    ttl: u32,
    bundles: &[Bundle],
) -> Result<(DnsChange, String)> {
    let roles = roles_for_usage(policy.usage)?;
    let proto = policy.proto.to_ascii_lowercase();
    if !matches!(proto.as_str(), "tcp" | "udp" | "sctp") {
        return Err(CertError::validation(format!(
            "TLSA proto '{}' is not tcp, udp or sctp",
            policy.proto
        )));
    }
    if policy.port == 0 {
        return Err(CertError::validation("TLSA port 0 is not a service port"));
    }
    let host_fqdn = owner_in_zone(&policy.host, zone)?;
    let owner = tlsa_owner(policy.port, &proto, &host_fqdn);

    let pool: Vec<&Bundle> = match &policy.bundle {
        Some(key) => {
            let found: Vec<&Bundle> = bundles.iter().filter(|b| bundle_matches(b, key)).collect();
            if found.is_empty() {
                return Err(CertError::validation(format!(
                    "TLSA entry for {owner} names bundle '{key}', which is not among the --bundle inputs"
                )));
            }
            found
        }
        None => bundles.iter().collect(),
    };
    let mut candidates = Vec::new();
    for b in pool {
        candidates.extend(
            bundle_candidates(b)?
                .into_iter()
                .filter(|c| roles.contains(&c.role.as_str())),
        );
    }
    if let Some(fp) = &policy.fingerprint {
        let fp = fp.trim().to_ascii_lowercase().replace(':', "");
        candidates.retain(|c| c.fingerprint_sha256 == fp);
    }
    if let Some(alg) = &policy.algorithm {
        candidates.retain(|c| c.algorithm.eq_ignore_ascii_case(alg.trim()));
    }
    if let Some(part) = &policy.partition {
        candidates.retain(|c| c.partition.eq_ignore_ascii_case(part.trim()));
    }
    // Distinct associations: cross-certificates over one key share an
    // SPKI, so selector 1 collapses them. With selector 0 prefer the
    // primary variant when one exists.
    let mut by_assoc: BTreeMap<String, Vec<Candidate>> = BTreeMap::new();
    for c in candidates {
        let a = association(&c.der, policy.selector, policy.matching)?;
        by_assoc.entry(a).or_default().push(c);
    }
    if policy.selector == 0 && by_assoc.len() > 1 {
        let primaries: BTreeMap<String, Vec<Candidate>> = by_assoc
            .iter()
            .filter(|(_, cs)| cs.iter().any(|c| c.primary))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if primaries.len() == 1 {
            by_assoc = primaries;
        }
    }
    let (assoc, chosen) = match by_assoc.len() {
        0 => {
            return Err(CertError::validation(format!(
                "no {} certificate in the supplied bundles fits the TLSA entry for {owner}",
                roles.join(" or ")
            )));
        }
        1 => by_assoc.into_iter().next().unwrap(),
        n => {
            let options: Vec<String> = by_assoc
                .values()
                .map(|cs| {
                    let c = &cs[0];
                    format!("{} {} partition {} ({})", c.role, c.algorithm, c.partition, c.fingerprint_sha256)
                })
                .collect();
            return Err(CertError::validation(format!(
                "TLSA entry for {owner} matches {n} distinct keys; narrow it with fingerprint, \
                 algorithm or partition: {}",
                options.join("; ")
            )));
        }
    };
    let c = &chosen[0];
    let endpoint = policy
        .endpoint
        .clone()
        .unwrap_or_else(|| "unstated: the operator must confirm this host terminates TLS itself".into());
    let pin = match policy.usage {
        3 => "DANE-EE",
        _ => "DANE-TA",
    };
    Ok((
        DnsChange {
            record_type: "TLSA".into(),
            name: owner,
            ttl,
            value: format!(
                "{} {} {} {}",
                policy.usage, policy.selector, policy.matching, assoc
            ),
            action: String::new(),
            requires: vec!["dnssec".into()],
            note: Some(format!(
                "{pin} pin of the {} {} key (partition {}, certificate {}); TLS terminated by: {endpoint}",
                c.role, c.algorithm, c.partition, c.fingerprint_sha256
            )),
        },
        c.bundle_id.clone(),
    ))
}

// ---------------------------------------------------------------- plan

/// Rdata normalized for comparison: case-folded tags and hex,
/// collapsed whitespace.
fn normalize_rdata(record_type: &str, value: &str) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    match record_type.to_ascii_uppercase().as_str() {
        "TLSA" | "CAA" => collapsed.to_ascii_lowercase(),
        _ => collapsed,
    }
}

/// Read a recorded snapshot: a change-set file or a bare JSON array
/// of records. Data an operator captured earlier, never a live query.
pub fn read_snapshot(path: &Path) -> Result<Vec<DnsChange>> {
    let text = std::fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&text)?;
    let records = match value {
        serde_json::Value::Array(_) => value,
        serde_json::Value::Object(mut o) => o.remove("records").ok_or_else(|| {
            CertError::usage(format!(
                "--current {} is neither a record array nor a change-set with records",
                path.display()
            ))
        })?,
        _ => {
            return Err(CertError::usage(format!(
                "--current {} is not a JSON record snapshot",
                path.display()
            )));
        }
    };
    #[derive(Deserialize)]
    struct Snap {
        #[serde(rename = "type")]
        record_type: String,
        name: String,
        #[serde(default)]
        ttl: u32,
        value: String,
    }
    let snaps: Vec<Snap> = serde_json::from_value(records)?;
    Ok(snaps
        .into_iter()
        .map(|s| DnsChange {
            record_type: s.record_type.to_ascii_uppercase(),
            name: if s.name.ends_with('.') {
                s.name.to_ascii_lowercase()
            } else {
                format!("{}.", s.name.to_ascii_lowercase())
            },
            ttl: s.ttl,
            value: s.value,
            action: String::new(),
            requires: vec![],
            note: None,
        })
        .collect())
}

/// Load the planner policy file.
pub fn read_policy(path: &Path) -> Result<DnsPolicy> {
    let text = std::fs::read_to_string(path)?;
    let policy: DnsPolicy = serde_json::from_str(&text)?;
    if let Some(s) = &policy.schema
        && s != DNS_POLICY_SCHEMA
    {
        return Err(CertError::usage(format!(
            "policy schema '{s}' is not {DNS_POLICY_SCHEMA}"
        )));
    }
    Ok(policy)
}

/// Compute the desired CAA/TLSA set and the change-set (contract §2).
pub fn plan(req: &DnsPlanRequest) -> Result<ChangeSet> {
    let zone = normalize_zone(&req.zone)?;
    let ttl = req.ttl.unwrap_or(DEFAULT_TTL);
    if ttl == 0 || ttl > MAX_TTL {
        return Err(CertError::usage(format!(
            "--ttl {ttl} is outside 1..={MAX_TTL} (RFC 2181 §8)"
        )));
    }
    let bundles = req
        .bundles
        .iter()
        .map(|p| load_bundle(p))
        .collect::<Result<Vec<_>>>()?;
    let policy = match &req.policy {
        Some(p) => read_policy(p)?,
        None => DnsPolicy::default(),
    };
    plan_with(&zone, ttl, &bundles, &policy, req.current.as_deref(), Rfc3339::now())
}

/// The planner over already-loaded inputs (deterministic but for
/// `generated_at`, which the caller passes).
pub fn plan_with(
    zone: &str,
    ttl: u32,
    bundles: &[Bundle],
    policy: &DnsPolicy,
    current: Option<&Path>,
    generated_at: Rfc3339,
) -> Result<ChangeSet> {
    let fleet_names = fleet_ca_names(bundles);
    let mut desired = Vec::new();
    let mut managed_names: BTreeSet<(String, String)> = BTreeSet::new();
    let mut used_bundles = BTreeSet::new();
    for caa in &policy.caa {
        let records = caa_records(caa, zone, ttl, &fleet_names)?;
        managed_names.insert(("CAA".into(), owner_in_zone(&caa.name, zone)?));
        desired.extend(records);
    }
    for t in &policy.tlsa {
        let (record, bundle_id) = tlsa_record(t, zone, ttl, bundles)?;
        managed_names.insert(("TLSA".into(), record.name.clone()));
        used_bundles.insert(bundle_id);
        desired.push(record);
    }
    // One record per identity.
    let mut seen = BTreeSet::new();
    desired.retain(|r| seen.insert(r.identity()));

    let mut records = Vec::new();
    match current {
        None => {
            for mut r in desired {
                r.action = "conditional-create".into();
                records.push(r);
            }
        }
        Some(path) => {
            let snapshot = read_snapshot(path)?;
            let current_ids: BTreeSet<_> = snapshot.iter().map(|r| r.identity()).collect();
            let desired_ids: BTreeSet<_> = desired.iter().map(|r| r.identity()).collect();
            for mut r in desired {
                r.action = if current_ids.contains(&r.identity()) {
                    "no-op".into()
                } else {
                    "create".into()
                };
                records.push(r);
            }
            // Deletes only at the (type, owner) pairs this plan manages,
            // so a snapshot's other records are never swept up.
            for mut r in snapshot {
                let managed = managed_names.contains(&(r.record_type.clone(), r.name.clone()));
                if managed && !desired_ids.contains(&r.identity()) {
                    r.action = "delete".into();
                    if r.ttl == 0 {
                        r.ttl = ttl;
                    }
                    r.note = Some("In the recorded snapshot but not in the desired set.".into());
                    records.push(r);
                }
            }
        }
    }
    records.sort_by(|a, b| {
        (a.name.as_str(), a.record_type.as_str(), a.value.as_str(), a.action.as_str()).cmp(&(
            b.name.as_str(),
            b.record_type.as_str(),
            b.value.as_str(),
            b.action.as_str(),
        ))
    });

    let mut prerequisites = vec![FLEET_CA_CAA_NOTE.to_string()];
    if records.iter().any(|r| r.record_type == "TLSA") {
        prerequisites.push(DNSSEC_PREREQUISITE.to_string());
    }
    prerequisites.push(
        "This change-set is a proposal produced offline; nothing was queried or written to live DNS. Applying it is a production-gated step (#9155).".to_string(),
    );
    let mut source_bundles: Vec<String> = used_bundles.into_iter().collect();
    source_bundles.sort();
    Ok(ChangeSet {
        schema: DNS_SCHEMA.to_string(),
        zone: zone.to_string(),
        generated_at,
        source_bundles,
        records,
        prerequisites,
    })
}

// ---------------------------------------------------------------- render

/// Read a change-set file (JSON).
pub fn read_change_set(path: &Path) -> Result<ChangeSet> {
    let text = std::fs::read_to_string(path)?;
    let cs: ChangeSet = serde_json::from_str(&text)?;
    if cs.schema != DNS_SCHEMA {
        return Err(CertError::usage(format!(
            "{} has schema '{}', not {DNS_SCHEMA}",
            path.display(),
            cs.schema
        )));
    }
    Ok(cs)
}

/// Canonical JSON (the review and verify unit).
pub fn render_json(cs: &ChangeSet) -> Result<String> {
    serde_json::to_string_pretty(cs).map_err(|e| CertError::engine(e.to_string()))
}

fn yaml_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// YAML presentation of the change-set (contract §3 field order).
/// Hand-rolled so the crate takes no YAML dependency; JSON stays
/// canonical.
pub fn render_yaml(cs: &ChangeSet) -> String {
    let mut out = String::new();
    out.push_str(&format!("schema: {}\n", cs.schema));
    out.push_str(&format!("zone: {}\n", cs.zone));
    out.push_str(&format!("generated_at: {}\n", yaml_quote(&cs.generated_at.to_string())));
    if cs.source_bundles.is_empty() {
        out.push_str("source_bundles: []\n");
    } else {
        out.push_str("source_bundles:\n");
        for b in &cs.source_bundles {
            out.push_str(&format!("  - {}\n", yaml_quote(b)));
        }
    }
    if cs.records.is_empty() {
        out.push_str("records: []\n");
    } else {
        out.push_str("records:\n");
        for r in &cs.records {
            out.push_str(&format!(
                "  - type: {}\n    name: {}\n    ttl: {}\n    value: {}\n    action: {}\n",
                r.record_type,
                r.name,
                r.ttl,
                yaml_quote(&r.value),
                r.action
            ));
            if !r.requires.is_empty() {
                out.push_str(&format!("    requires: [{}]\n", r.requires.join(", ")));
            }
            if let Some(n) = &r.note {
                out.push_str(&format!("    note: {}\n", yaml_quote(n)));
            }
        }
    }
    out.push_str("prerequisites:\n");
    for p in &cs.prerequisites {
        out.push_str(&format!("  - {}\n", yaml_quote(p)));
    }
    out
}

/// BIND zone fragment (RFC 1035 presentation, §4.4). Records to
/// create or keep are resource records; deletes appear as comments,
/// since a zone fragment states what should exist.
pub fn render_zone(cs: &ChangeSet) -> String {
    let mut out = format!(
        "; {} change-set for {} generated {}\n; PROPOSED: rendered offline; not published to or proven against live DNS.\n",
        cs.schema, cs.zone, cs.generated_at
    );
    for p in &cs.prerequisites {
        out.push_str(&format!("; prerequisite: {p}\n"));
    }
    if cs.records.is_empty() {
        out.push_str("; (no records)\n");
    }
    for r in &cs.records {
        if let Some(n) = &r.note {
            out.push_str(&format!("; {}: {n}\n", r.action));
        } else {
            out.push_str(&format!("; {}\n", r.action));
        }
        let line = format!("{}\t{}\tIN\t{}\t{}", r.name, r.ttl, r.record_type, r.value);
        if r.action == "delete" {
            out.push_str(&format!("; DELETE {line}\n"));
        } else {
            out.push_str(&format!("{line}\n"));
        }
    }
    out
}

/// Human-readable change-set summary.
pub fn render_text(cs: &ChangeSet) -> String {
    let mut out = format!(
        "DNS change-set ({}) for {}: {} record(s)\n",
        cs.schema,
        cs.zone,
        cs.records.len()
    );
    if cs.is_no_change() {
        out.push_str("  no changes: the desired set equals the recorded current set\n");
    }
    for r in &cs.records {
        out.push_str(&format!(
            "  {:18} {} {} ttl={} {}\n",
            r.action, r.record_type, r.name, r.ttl, r.value
        ));
    }
    for p in &cs.prerequisites {
        out.push_str(&format!("  prerequisite: {p}\n"));
    }
    out.push_str("  state: proposed (offline); nothing was published.\n");
    out
}

// ---------------------------------------------------------------- verify

fn parse_caa(value: &str) -> std::result::Result<(u8, String, String), String> {
    let mut parts = value.trim().splitn(3, char::is_whitespace);
    let flags = parts
        .next()
        .and_then(|f| f.parse::<u8>().ok())
        .ok_or("CAA flags are not a number 0..255")?;
    let tag = parts.next().ok_or("CAA tag missing")?.to_ascii_lowercase();
    let raw = parts.next().ok_or("CAA value missing")?.trim();
    let val = raw
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .ok_or("CAA value must be a quoted string")?;
    Ok((flags, tag, val.to_string()))
}

fn parse_tlsa(value: &str) -> std::result::Result<(u8, u8, u8, String), String> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.len() < 4 {
        return Err("TLSA rdata needs usage, selector, matching type and data".into());
    }
    let num = |s: &str, what: &str| s.parse::<u8>().map_err(|_| format!("TLSA {what} '{s}' is not a number"));
    let data = parts[3..].concat().to_ascii_lowercase();
    if data.is_empty() || !data.chars().all(|c| c.is_ascii_hexdigit()) || data.len() % 2 != 0 {
        return Err("TLSA association data is not even-length hex".into());
    }
    Ok((num(parts[0], "usage")?, num(parts[1], "selector")?, num(parts[2], "matching type")?, data))
}

/// Verify a change-set offline against the bundles (contract §2).
/// Returns one named check per item; `valid` means **proposed**,
/// never published or proven (§5).
pub fn verify(cs: &ChangeSet, label: &str, bundle_dirs: &[PathBuf]) -> Result<DnsVerifyReport> {
    let bundles = bundle_dirs
        .iter()
        .map(|p| load_bundle(p))
        .collect::<Result<Vec<_>>>()?;
    let mut candidates = Vec::new();
    for b in &bundles {
        candidates.extend(bundle_candidates(b)?);
    }
    let fleet_names = fleet_ca_names(&bundles);
    let mut checks = Vec::new();
    checks.push(Check {
        name: "schema".into(),
        ok: cs.schema == DNS_SCHEMA,
        detail: format!("schema {} (expected {DNS_SCHEMA})", cs.schema),
    });
    let zone_ok = normalize_zone(&cs.zone).is_ok();
    checks.push(Check {
        name: "zone".into(),
        ok: zone_ok,
        detail: format!("zone apex {}", cs.zone),
    });
    let zone = cs.zone.trim_end_matches('.').to_ascii_lowercase();
    let mut any_tlsa = false;
    for (i, r) in cs.records.iter().enumerate() {
        let name = format!("record[{i}]:{} {}", r.record_type, r.name);
        let mut problems: Vec<String> = Vec::new();
        let mut details: Vec<String> = Vec::new();
        let fq = r.name.trim_end_matches('.');
        if !r.name.ends_with('.') {
            problems.push("owner name has no trailing dot".into());
        }
        if !is_hostname(&r.name.to_ascii_lowercase(), true) {
            problems.push("owner name is not well-formed".into());
        } else if !(fq.eq_ignore_ascii_case(&zone) || fq.to_ascii_lowercase().ends_with(&format!(".{zone}"))) {
            problems.push(format!("owner name is outside zone {zone}"));
        }
        if r.ttl == 0 || r.ttl > MAX_TTL {
            problems.push(format!("ttl {} outside 1..={MAX_TTL}", r.ttl));
        }
        if !ACTIONS.contains(&r.action.as_str()) {
            problems.push(format!("action '{}' is not one of {}", r.action, ACTIONS.join("|")));
        }
        match r.record_type.as_str() {
            "CAA" => match parse_caa(&r.value) {
                Err(e) => problems.push(e),
                Ok((flags, tag, val)) => {
                    if flags != 0 && flags != 128 {
                        problems.push(format!("CAA flags {flags} are not 0 or 128"));
                    }
                    match tag.as_str() {
                        "issue" | "issuewild" => {
                            let issuer = val.split(';').next().unwrap_or("").trim();
                            let issuer = if issuer.is_empty() { ";" } else { issuer };
                            match check_caa_issuer(issuer, &fleet_names) {
                                Ok(()) => details.push(format!("{tag} names public issuer {issuer}")),
                                Err(e) => problems.push(e),
                            }
                        }
                        "iodef" => {
                            if !(val.starts_with("mailto:") || val.starts_with("https://")) {
                                problems.push("CAA iodef is not mailto: or https://".into());
                            }
                        }
                        other => problems.push(format!("CAA tag '{other}' is not issue, issuewild or iodef")),
                    }
                }
            },
            "TLSA" => {
                any_tlsa = true;
                let labels: Vec<&str> = r.name.split('.').collect();
                let owner_ok = labels.len() >= 3
                    && labels[0].strip_prefix('_').is_some_and(|p| p.parse::<u16>().is_ok_and(|n| n > 0))
                    && matches!(labels[1], "_tcp" | "_udp" | "_sctp");
                if !owner_ok {
                    problems.push("TLSA owner is not _<port>._<proto>.<host> (RFC 6698 §3)".into());
                }
                if !r.requires.iter().any(|g| g == "dnssec") {
                    problems.push("TLSA record is not marked requires: [dnssec] (contract §4)".into());
                }
                match parse_tlsa(&r.value) {
                    Err(e) => problems.push(e),
                    Ok((usage, selector, matching, data)) => match roles_for_usage(usage) {
                        Err(e) => problems.push(e.to_string()),
                        Ok(roles) => {
                            if candidates.is_empty() {
                                problems.push("no --bundle given: the association cannot be recomputed".into());
                            } else {
                                let mut hit = None;
                                for c in &candidates {
                                    match association(&c.der, selector, matching) {
                                        Ok(a) if a == data => {
                                            hit = Some(c);
                                            break;
                                        }
                                        Ok(_) => {}
                                        Err(e) => {
                                            problems.push(e.to_string());
                                            break;
                                        }
                                    }
                                }
                                match hit {
                                    None if problems.is_empty() => problems.push(
                                        "association matches no certificate in the supplied bundles".into(),
                                    ),
                                    None => {}
                                    Some(c) if !roles.contains(&c.role.as_str()) => problems.push(format!(
                                        "association pins a {} certificate, but usage {usage} pins {}",
                                        c.role,
                                        roles.join(" or ")
                                    )),
                                    Some(c) => details.push(format!(
                                        "association recomputed from {} {} key in bundle {} (proposed: not proven until a DNSSEC-validating read-back)",
                                        c.role, c.algorithm, c.bundle_id
                                    )),
                                }
                            }
                        }
                    },
                }
            }
            other => problems.push(format!("record type {other} is outside fleet.pki.dns/v1 (CAA, TLSA)")),
        }
        let ok = problems.is_empty();
        checks.push(Check {
            name,
            ok,
            detail: if ok {
                if details.is_empty() { "well-formed".into() } else { details.join("; ") }
            } else {
                problems.join("; ")
            },
        });
    }
    if any_tlsa {
        let has = cs.prerequisites.iter().any(|p| p == DNSSEC_PREREQUISITE);
        checks.push(Check {
            name: "dnssec-prerequisite".into(),
            ok: has,
            detail: if has {
                "TLSA records are listed as requiring a DNSSEC-signed zone".into()
            } else {
                "TLSA records present but the DNSSEC prerequisite is missing (contract §4)".into()
            },
        });
    }
    let valid = checks.iter().all(|c| c.ok);
    Ok(DnsVerifyReport {
        change_set: label.to_string(),
        zone: cs.zone.clone(),
        valid,
        state: if valid { "proposed" } else { "invalid" }.to_string(),
        checks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_names_stay_in_zone() {
        // Oracle: RFC 1035 owner-name rules and the zone-cut rule
        // that a zone only holds names at or below its apex.
        assert_eq!(owner_in_zone("@", "example.test").unwrap(), "example.test.");
        assert_eq!(owner_in_zone("pki", "example.test").unwrap(), "pki.example.test.");
        assert_eq!(
            owner_in_zone("pki.example.test.", "example.test").unwrap(),
            "pki.example.test."
        );
        assert!(owner_in_zone("other.test.", "example.test").is_err());
        assert!(owner_in_zone("bad_label", "example.test").is_ok()); // underscores admitted for owners
        assert!(owner_in_zone("-bad", "example.test").is_err());
        assert_eq!(tlsa_owner(443, "TCP", "pki.example.test."), "_443._tcp.pki.example.test.");
    }

    #[test]
    fn caa_refuses_the_fleet_ca_and_non_domains() {
        // Oracle: contract §1 / spec §24.8 — CAA names public issuers only.
        let names = fleet_ca_names(&[]);
        assert!(check_caa_issuer("letsencrypt.org", &names).is_ok());
        assert!(check_caa_issuer(";", &names).is_ok());
        let e = check_caa_issuer("fleet-ca", &names).unwrap_err();
        assert!(e.contains("private Fleet CA"), "{e}");
        assert!(check_caa_issuer("Fleet", &names).is_err());
        assert!(check_caa_issuer("internal", &names).is_err());
    }

    #[test]
    fn rdata_parsers_read_presentation_format() {
        // Oracle: RFC 8659 §4.1.1 and RFC 6698 §2.2 presentation formats.
        assert_eq!(
            parse_caa("0 issue \"letsencrypt.org\"").unwrap(),
            (0, "issue".into(), "letsencrypt.org".into())
        );
        assert!(parse_caa("0 issue letsencrypt.org").is_err());
        assert_eq!(
            parse_tlsa("3 1 1 AB CD").unwrap(),
            (3, 1, 1, "abcd".into())
        );
        assert!(parse_tlsa("3 1 1 xyz").is_err());
        assert!(roles_for_usage(1).is_err());
        assert!(roles_for_usage(0).is_err());
    }
}
