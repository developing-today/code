//! `fleet cert batch validate | plan | apply | status` (spec §§6.5,
//! 25.4; #13730): a planned multi-operation run, such as an issuer
//! set plus the leaf set for a service, reviewed as one plan and
//! applied all-or-nothing over bundle directories.
//!
//! The contract:
//!
//! - **Typed, declarative input.** A `fleet.batch/v1` JSON document
//!   lists operations with ids, `after` dependencies, a command
//!   (`cert.issuer.create` | `cert.leaf.issue`), an output directory
//!   and typed arguments. References are the constrained `@<id>.bundle`
//!   / `@<id>.keys` forms; there is no `${...}` interpolation, shell
//!   expansion or host command anywhere in the parser.
//! - **Explicit signer sources (§24.3).** Every signing step names its
//!   signer key directory: an existing path, or `@<id>.keys` for keys
//!   an earlier step of the same batch creates. A bare `@<id>` is
//!   refused as a signer, and a leaf's signer must belong to the
//!   issuer it is issued from.
//! - **One reviewable plan.** `plan` resolves the DAG, checks every
//!   referenced bundle, key directory, profile, algorithm and validity
//!   window, writes nothing, and prints the batch digest. `apply`
//!   needs `--approve <digest>`, so what runs is what was reviewed.
//! - **All-or-nothing.** `apply` runs every step in a private staging
//!   directory under `<base>/.fleet-batch/`, then moves the finished
//!   bundles into place. A failing step, a failed expectation or a
//!   failed move removes the staging area and every bundle already
//!   moved; nothing is left half-applied. `--dry-run` runs every step
//!   the same way and then removes the staging area.
//! - **No accidental reissue.** A completed apply writes a receipt
//!   keyed by the idempotency key (default: the batch digest).
//!   Applying the same batch again returns that receipt and issues
//!   nothing; reusing the key for a different batch is refused.
//!
//! Root creation is a key ceremony and stays `fleet cert root create`;
//! a batch references an existing root set by path. External key
//! stores (`private_store`) are refused because their keys would be
//! written outside the staging area, where all-or-nothing cannot hold.

use crate::algorithm::{self, AlgorithmId};
use crate::bundle::{load_bundle, sha256_hex};
use crate::engine;
use crate::error::{CertError, Result};
use crate::graph::CrossSigningMode;
use crate::model::{
    BundleManifest, BundleType, IssuerCreateRequest, IssuerPurpose, LeafIssueRequest,
    SignerSource, resolve_slots,
};
use crate::profile;
use crate::timeparse::{CalendarDuration, ISSUER_DEFAULT_LIFETIME_YEARS, Rfc3339};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

/// The batch document schema (spec §§6.5, 25.4).
pub const BATCH_SCHEMA: &str = "fleet.batch/v1";
/// The plan / apply report schema.
pub const REPORT_SCHEMA: &str = "fleet.batch.report/v1";
/// The receipt written by a completed apply.
pub const RECEIPT_SCHEMA: &str = "fleet.batch.receipt/v1";
/// Batch state lives here, under the base directory.
pub const STATE_DIR: &str = ".fleet-batch";
/// The commands a batch can run.
pub const ISSUER_CREATE: &str = "cert.issuer.create";
pub const LEAF_ISSUE: &str = "cert.leaf.issue";
/// Upper bound on operations in one batch.
pub const MAX_OPERATIONS: usize = 256;

// ---------------------------------------------------------------- input

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBatch {
    schema: String,
    #[serde(default)]
    idempotency_key: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    operations: Vec<RawOp>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOp {
    id: String,
    #[serde(default)]
    after: Vec<String>,
    command: String,
    out: String,
    #[serde(default)]
    with: Map<String, Value>,
    #[serde(default)]
    expect: Option<Expect>,
}

/// Per-step expected results, checked inside the staging area before
/// anything is committed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    /// Exact number of certificate records in the produced bundle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub certificates: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct IssuerArgs {
    root: String,
    root_key_dir: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    ou: Option<String>,
    #[serde(default)]
    algorithms: Option<Vec<String>>,
    #[serde(default)]
    partitions: Option<Vec<String>>,
    #[serde(default)]
    cross_signing: Option<String>,
    #[serde(default)]
    permitted_dns: Vec<String>,
    #[serde(default)]
    not_before: Option<String>,
    #[serde(default)]
    not_after: Option<String>,
    #[serde(default)]
    valid_for: Option<String>,
    #[serde(default)]
    allow_legacy_root_strength: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct LeafArgs {
    issuer: String,
    issuer_key_dir: String,
    profile: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    cn: Option<String>,
    #[serde(default)]
    dns: Vec<String>,
    #[serde(default)]
    ip: Vec<String>,
    #[serde(default)]
    csr: Option<String>,
    #[serde(default)]
    algorithm: Option<String>,
    #[serde(default)]
    issuer_algorithm: Option<String>,
    #[serde(default)]
    issuer_partition: Option<String>,
    #[serde(default)]
    not_before: Option<String>,
    #[serde(default)]
    not_after: Option<String>,
    #[serde(default)]
    valid_for: Option<String>,
}

// ---------------------------------------------------------------- resolved batch

/// Where a step's parent bundle or signer keys come from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Source {
    /// An existing path (relative paths resolve against the base).
    Path(PathBuf),
    /// The bundle an earlier step of this batch produces.
    Bundle(String),
    /// The private key directory of an earlier step's bundle.
    Keys(String),
}

#[derive(Debug, Clone)]
enum Request {
    Issuer {
        slots: Vec<(AlgorithmId, String)>,
        name: String,
        purpose: IssuerPurpose,
        ou: Option<String>,
        cross_signing: CrossSigningMode,
        permitted_dns: Vec<String>,
        allow_legacy_root_strength: bool,
    },
    Leaf {
        profile: String,
        name: String,
        common_name: Option<String>,
        dns_names: Vec<String>,
        ip_addresses: Vec<std::net::IpAddr>,
        csr: Option<PathBuf>,
        key_algorithm: Option<AlgorithmId>,
        issuer_algorithm: Option<AlgorithmId>,
        issuer_partition: Option<String>,
    },
}

#[derive(Debug, Clone)]
struct Step {
    id: String,
    command: &'static str,
    after: Vec<String>,
    out: PathBuf,
    parent: Source,
    signer: Source,
    request: Request,
    not_before: Rfc3339,
    not_after: Rfc3339,
    not_before_defaulted: bool,
    expect: Option<Expect>,
}

/// A parsed, resolved and statically checked batch.
#[derive(Debug, Clone)]
pub struct Batch {
    /// SHA-256 of the batch document's bytes: the approval token.
    pub sha256: String,
    pub idempotency_key: Option<String>,
    /// Steps in execution (topological) order.
    steps: Vec<Step>,
    pub warnings: Vec<String>,
}

impl Batch {
    /// The receipt key: the idempotency key, else the batch digest.
    pub fn receipt_key(&self) -> String {
        self.idempotency_key
            .clone()
            .unwrap_or_else(|| format!("sha256-{}", self.sha256))
    }

    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Step ids in execution order.
    pub fn order(&self) -> Vec<String> {
        self.steps.iter().map(|s| s.id.clone()).collect()
    }
}

fn invalid(op: &str, msg: impl std::fmt::Display) -> CertError {
    CertError::validation(format!("operation '{op}': {msg}"))
}

fn is_slug(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn is_key(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

/// Refuse anything that looks like interpolation or shell syntax in
/// any string argument (spec §6.5: constrained typed references only).
fn scan_strings(op: &str, path: &str, v: &Value) -> Result<()> {
    match v {
        Value::String(s) => {
            for bad in ["${", "$(", "`"] {
                if s.contains(bad) {
                    return Err(invalid(
                        op,
                        format!(
                            "'{path}' contains '{bad}': batch arguments have no interpolation or shell \
                             expansion; reference earlier steps with @<id>.bundle or @<id>.keys"
                        ),
                    ));
                }
            }
            Ok(())
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                scan_strings(op, &format!("{path}[{i}]"), item)?;
            }
            Ok(())
        }
        Value::Object(map) => {
            for (k, item) in map {
                scan_strings(op, &format!("{path}.{k}"), item)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Validate an output directory: relative, plain components only, and
/// not inside the batch state directory.
fn check_out(op: &str, raw: &str) -> Result<PathBuf> {
    let path = PathBuf::from(raw);
    if raw.trim().is_empty() {
        return Err(invalid(op, "'out' is empty"));
    }
    let mut parts = Vec::new();
    for c in path.components() {
        match c {
            Component::Normal(p) => parts.push(p.to_string_lossy().to_string()),
            _ => {
                return Err(invalid(
                    op,
                    format!(
                        "'out' must be a relative path of plain directory names under the base \
                         (got '{raw}')"
                    ),
                ));
            }
        }
    }
    if parts.first().map(|p| p.as_str()) == Some(STATE_DIR) {
        return Err(invalid(op, format!("'out' may not be inside {STATE_DIR}/")));
    }
    Ok(parts.iter().collect())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Bundle,
    Keys,
}

fn parse_source(op: &str, field: &str, raw: &str, kind: Field) -> Result<Source> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(invalid(op, format!("'{field}' is empty")));
    }
    let Some(reference) = raw.strip_prefix('@') else {
        return Ok(Source::Path(PathBuf::from(raw)));
    };
    let (id, suffix) = match reference.split_once('.') {
        Some((id, suffix)) => (id, Some(suffix)),
        None => (reference, None),
    };
    if !is_slug(id, 63) {
        return Err(invalid(op, format!("'{field}' reference '{raw}' names no valid step id")));
    }
    match (kind, suffix) {
        (Field::Bundle, None) | (Field::Bundle, Some("bundle")) => Ok(Source::Bundle(id.into())),
        (Field::Keys, Some("keys")) => Ok(Source::Keys(id.into())),
        (Field::Keys, None) | (Field::Keys, Some("bundle")) => Err(invalid(
            op,
            format!(
                "'{field}' must name the signer's key directory explicitly (spec §24.3): \
                 write '@{id}.keys' or a key directory path, not '{raw}'"
            ),
        )),
        (Field::Bundle, Some("keys")) => Err(invalid(
            op,
            format!("'{field}' takes a bundle ('@{id}.bundle'), not a key directory"),
        )),
        (_, Some(other)) => Err(invalid(
            op,
            format!("'{field}' reference '{raw}' has unknown part '.{other}' (bundle | keys)"),
        )),
    }
}

fn parse_algo(op: &str, field: &str, raw: &Option<String>, warnings: &mut Vec<String>) -> Result<Option<AlgorithmId>> {
    match raw {
        Some(s) => {
            let (a, w) = AlgorithmId::parse(s).map_err(|e| invalid(op, format!("{field}: {e}")))?;
            if let Some(w) = w {
                warnings.push(format!("{op}: {w}"));
            }
            Ok(Some(a))
        }
        None => Ok(None),
    }
}

fn parse_validity(
    op: &str,
    not_before: &Option<String>,
    valid_for: &Option<String>,
    not_after: &Option<String>,
    default_lifetime: CalendarDuration,
) -> Result<(Rfc3339, Rfc3339, bool)> {
    let nb = not_before
        .as_deref()
        .map(Rfc3339::parse)
        .transpose()
        .map_err(|e| invalid(op, format!("not_before: {e}")))?;
    let vf = valid_for
        .as_deref()
        .map(CalendarDuration::parse)
        .transpose()
        .map_err(|e| invalid(op, format!("valid_for: {e}")))?;
    let na = not_after
        .as_deref()
        .map(Rfc3339::parse)
        .transpose()
        .map_err(|e| invalid(op, format!("not_after: {e}")))?;
    let defaulted = nb.is_none();
    let (nb, na) = crate::timeparse::resolve_validity(nb, vf, na, default_lifetime)
        .map_err(|e| invalid(op, e))?;
    Ok((nb, na, defaulted))
}

/// Resolve a non-reference input path against the base directory.
fn resolve_path(base: &Path, p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    }
}

/// Parse, resolve and statically check a batch document. Nothing is
/// written; existing inputs (root and issuer bundles, key directories,
/// CSRs) are read to check they are what the batch says they are.
pub fn load(bytes: &[u8], base: &Path) -> Result<Batch> {
    let sha256 = sha256_hex(bytes);
    let raw: RawBatch = serde_json::from_slice(bytes).map_err(|e| {
        CertError::validation(format!("the batch is not a valid {BATCH_SCHEMA} JSON document: {e}"))
    })?;
    if raw.schema != BATCH_SCHEMA {
        return Err(CertError::validation(format!(
            "unsupported batch schema '{}' (this build reads {BATCH_SCHEMA})",
            raw.schema
        )));
    }
    if let Some(mode) = &raw.mode
        && mode != "plan-first"
    {
        return Err(CertError::validation(format!(
            "unsupported batch mode '{mode}' (plan-first is the only mode: apply always needs \
             the reviewed plan's digest)"
        )));
    }
    if let Some(key) = &raw.idempotency_key
        && !is_key(key)
    {
        return Err(CertError::validation(format!(
            "idempotency_key '{key}' must be 1-128 letters, digits, '.', '_' or '-'"
        )));
    }
    if raw.operations.is_empty() {
        return Err(CertError::validation("the batch has no operations"));
    }
    if raw.operations.len() > MAX_OPERATIONS {
        return Err(CertError::validation(format!(
            "the batch has {} operations; the limit is {MAX_OPERATIONS}",
            raw.operations.len()
        )));
    }

    // Ids and dependency edges.
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    for (i, op) in raw.operations.iter().enumerate() {
        if !is_slug(&op.id, 63) {
            return Err(CertError::validation(format!(
                "operation id '{}' must be 1-63 lowercase letters, digits or '-'",
                op.id
            )));
        }
        if index.insert(op.id.clone(), i).is_some() {
            return Err(CertError::validation(format!("duplicate operation id '{}'", op.id)));
        }
    }
    for op in &raw.operations {
        let mut seen = BTreeSet::new();
        for dep in &op.after {
            if dep == &op.id {
                return Err(invalid(&op.id, "lists itself in 'after'"));
            }
            if !index.contains_key(dep) {
                return Err(invalid(&op.id, format!("'after' names unknown operation '{dep}'")));
            }
            if !seen.insert(dep) {
                return Err(invalid(&op.id, format!("'after' lists '{dep}' twice")));
            }
        }
    }

    // Stable topological order (Kahn; ready steps in file order).
    let n = raw.operations.len();
    let mut indegree: Vec<usize> = raw.operations.iter().map(|o| o.after.len()).collect();
    let mut done = vec![false; n];
    let mut order = Vec::with_capacity(n);
    while order.len() < n {
        let next = (0..n).find(|&i| !done[i] && indegree[i] == 0);
        let Some(i) = next else {
            let stuck: Vec<&str> = (0..n)
                .filter(|&i| !done[i])
                .map(|i| raw.operations[i].id.as_str())
                .collect();
            return Err(CertError::validation(format!(
                "the 'after' edges form a cycle among: {}",
                stuck.join(", ")
            )));
        };
        done[i] = true;
        order.push(i);
        for (j, op) in raw.operations.iter().enumerate() {
            if op.after.iter().any(|d| d == &raw.operations[i].id) {
                indegree[j] -= 1;
            }
        }
    }

    // Transitive ancestors, for the reference rule.
    let mut ancestors: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for &i in &order {
        let op = &raw.operations[i];
        let mut set = BTreeSet::new();
        for dep in &op.after {
            set.insert(dep.clone());
            if let Some(up) = ancestors.get(dep) {
                set.extend(up.iter().cloned());
            }
        }
        ancestors.insert(op.id.clone(), set);
    }

    // Outputs: unique and not nested inside one another.
    let mut outs: Vec<(String, PathBuf)> = Vec::new();
    for op in &raw.operations {
        let out = check_out(&op.id, &op.out)?;
        for (other, theirs) in &outs {
            if out.starts_with(theirs) || theirs.starts_with(&out) {
                return Err(invalid(
                    &op.id,
                    format!(
                        "'out' {} overlaps operation '{other}' ({}); every step writes its own directory",
                        out.display(),
                        theirs.display()
                    ),
                ));
            }
        }
        outs.push((op.id.clone(), out));
    }

    let command_of: BTreeMap<String, String> = raw
        .operations
        .iter()
        .map(|o| (o.id.clone(), o.command.clone()))
        .collect();

    let mut warnings = Vec::new();
    let mut steps = Vec::with_capacity(n);
    for &i in &order {
        let op = &raw.operations[i];
        let id = op.id.as_str();
        scan_strings(id, "with", &Value::Object(op.with.clone()))?;
        for forbidden in ["private_store", "private-store"] {
            if op.with.contains_key(forbidden) {
                return Err(CertError::policy(format!(
                    "operation '{id}': '{forbidden}' is not accepted in a batch: an external key \
                     store is written outside the staging area, so the batch could not be undone \
                     on failure. Create that bundle with its own `fleet cert` command."
                )));
            }
        }
        let with = Value::Object(op.with.clone());
        let out = outs[i].1.clone();
        let check_ref = |field: &str, src: &Source, want: &str| -> Result<()> {
            let target = match src {
                Source::Bundle(t) | Source::Keys(t) => t,
                Source::Path(_) => return Ok(()),
            };
            let Some(cmd) = command_of.get(target) else {
                return Err(invalid(id, format!("'{field}' references unknown operation '{target}'")));
            };
            if !ancestors[id].contains(target) {
                return Err(invalid(
                    id,
                    format!(
                        "'{field}' references '{target}', which is not in this operation's 'after' \
                         chain; list it so the dependency is explicit"
                    ),
                ));
            }
            if cmd != want {
                return Err(invalid(
                    id,
                    format!("'{field}' references '{target}', a {cmd} step; it needs a {want} step"),
                ));
            }
            Ok(())
        };
        let step = match op.command.as_str() {
            ISSUER_CREATE => {
                let a: IssuerArgs = serde_json::from_value(with)
                    .map_err(|e| invalid(id, format!("{ISSUER_CREATE} arguments: {e}")))?;
                let parent = parse_source(id, "root", &a.root, Field::Bundle)?;
                let signer = parse_source(id, "root_key_dir", &a.root_key_dir, Field::Keys)?;
                for (field, src) in [("root", &parent), ("root_key_dir", &signer)] {
                    if !matches!(src, Source::Path(_)) {
                        return Err(invalid(
                            id,
                            format!(
                                "'{field}' must be an existing root set and its key directory: \
                                 root creation is a key ceremony (`fleet cert root create`), \
                                 not a batch step"
                            ),
                        ));
                    }
                }
                let algos = match &a.algorithms {
                    Some(list) => {
                        let mut out = Vec::new();
                        for s in list {
                            out.push(parse_algo(id, "algorithms", &Some(s.clone()), &mut warnings)?.unwrap());
                        }
                        out
                    }
                    None => algorithm::DEFAULT_ALGORITHMS.to_vec(),
                };
                let parts: Option<Vec<String>> = a.partitions.as_ref().map(|p| {
                    p.iter()
                        .map(|x| x.trim().to_ascii_lowercase())
                        .filter(|x| !x.is_empty())
                        .collect()
                });
                let slots = resolve_slots(&algos, parts.as_deref()).map_err(|e| invalid(id, e))?;
                let purpose = IssuerPurpose::parse(a.purpose.as_deref().unwrap_or("general"))
                    .map_err(|e| invalid(id, e))?;
                let cross_signing = match &a.cross_signing {
                    Some(s) => CrossSigningMode::parse(s).map_err(|e| invalid(id, e))?,
                    None => CrossSigningMode::RootsOnly,
                };
                let (nb, na, defaulted) = parse_validity(
                    id,
                    &a.not_before,
                    &a.valid_for,
                    &a.not_after,
                    CalendarDuration::years(ISSUER_DEFAULT_LIFETIME_YEARS),
                )?;
                Step {
                    id: id.into(),
                    command: ISSUER_CREATE,
                    after: op.after.clone(),
                    out,
                    parent,
                    signer,
                    request: Request::Issuer {
                        slots,
                        name: a.name.clone().unwrap_or_else(|| "Base".into()),
                        purpose,
                        ou: a.ou.clone(),
                        cross_signing,
                        permitted_dns: a.permitted_dns.clone(),
                        allow_legacy_root_strength: a.allow_legacy_root_strength,
                    },
                    not_before: nb,
                    not_after: na,
                    not_before_defaulted: defaulted,
                    expect: op.expect.clone(),
                }
            }
            LEAF_ISSUE => {
                let a: LeafArgs = serde_json::from_value(with)
                    .map_err(|e| invalid(id, format!("{LEAF_ISSUE} arguments: {e}")))?;
                let parent = parse_source(id, "issuer", &a.issuer, Field::Bundle)?;
                let signer = parse_source(id, "issuer_key_dir", &a.issuer_key_dir, Field::Keys)?;
                check_ref("issuer", &parent, ISSUER_CREATE)?;
                check_ref("issuer_key_dir", &signer, ISSUER_CREATE)?;
                match (&parent, &signer) {
                    (Source::Bundle(p), Source::Keys(s)) if p != s => {
                        return Err(invalid(
                            id,
                            format!(
                                "signer '@{s}.keys' does not belong to issuer '@{p}': a leaf is \
                                 signed by its own issuer's keys"
                            ),
                        ));
                    }
                    (Source::Path(_), Source::Keys(s)) => {
                        return Err(invalid(
                            id,
                            format!("signer '@{s}.keys' belongs to a batch step, but the issuer is an existing bundle"),
                        ));
                    }
                    (Source::Bundle(p), Source::Path(_)) => {
                        return Err(invalid(
                            id,
                            format!(
                                "issuer '@{p}' is created by this batch, so its signer must be \
                                 '@{p}.keys', not a separate path"
                            ),
                        ));
                    }
                    _ => {}
                }
                let prof = profile::leaf_profile(&a.profile).map_err(|e| invalid(id, e))?;
                let ip_addresses = a
                    .ip
                    .iter()
                    .map(|s| {
                        s.trim()
                            .parse::<std::net::IpAddr>()
                            .map_err(|_| invalid(id, format!("ip '{s}' is not an IP address")))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let (nb, na, defaulted) =
                    parse_validity(id, &a.not_before, &a.valid_for, &a.not_after, prof.default_validity)?;
                Step {
                    id: id.into(),
                    command: LEAF_ISSUE,
                    after: op.after.clone(),
                    out,
                    parent,
                    signer,
                    request: Request::Leaf {
                        profile: a.profile.clone(),
                        name: a.name.clone().unwrap_or_else(|| id.to_string()),
                        common_name: a.cn.clone(),
                        dns_names: a.dns.clone(),
                        ip_addresses,
                        csr: a.csr.as_deref().map(PathBuf::from),
                        key_algorithm: parse_algo(id, "algorithm", &a.algorithm, &mut warnings)?,
                        issuer_algorithm: parse_algo(id, "issuer_algorithm", &a.issuer_algorithm, &mut warnings)?,
                        issuer_partition: a.issuer_partition.clone(),
                    },
                    not_before: nb,
                    not_after: na,
                    not_before_defaulted: defaulted,
                    expect: op.expect.clone(),
                }
            }
            "cert.root.create" => {
                return Err(CertError::policy(format!(
                    "operation '{id}': cert.root.create is a key ceremony and is not a batch step; \
                     run `fleet cert root create` and reference the root set by path"
                )));
            }
            other => {
                return Err(invalid(
                    id,
                    format!("unknown command '{other}' ({ISSUER_CREATE} | {LEAF_ISSUE})"),
                ));
            }
        };
        check_existing_inputs(&step, base)?;
        steps.push(step);
    }

    Ok(Batch {
        sha256,
        idempotency_key: raw.idempotency_key,
        steps,
        warnings,
    })
}

/// Existing (path) inputs must be what the step says they are.
fn check_existing_inputs(step: &Step, base: &Path) -> Result<()> {
    let id = step.id.as_str();
    if let Source::Path(p) = &step.parent {
        let dir = resolve_path(base, p);
        let bundle = load_bundle(&dir).map_err(|e| invalid(id, e))?;
        let want = if step.command == ISSUER_CREATE {
            BundleType::RootSet
        } else {
            BundleType::IssuerSet
        };
        if bundle.manifest.bundle_type != want {
            return Err(invalid(
                id,
                format!(
                    "{} is a {} bundle; this step needs a {}",
                    p.display(),
                    bundle.manifest.bundle_type.as_str(),
                    want.as_str()
                ),
            ));
        }
    }
    if let Source::Path(p) = &step.signer {
        let dir = resolve_path(base, p);
        if !dir.is_dir() {
            return Err(invalid(
                id,
                format!("signer key directory {} does not exist", p.display()),
            ));
        }
    }
    if let Request::Leaf { csr: Some(csr), .. } = &step.request {
        let f = resolve_path(base, csr);
        if !f.is_file() {
            return Err(invalid(id, format!("csr {} is not a readable file", csr.display())));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- reports

/// One operation in a plan or apply report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpReport {
    pub id: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after: Vec<String>,
    pub out: String,
    /// The parent bundle: a path, or `@<id>.bundle`.
    pub parent: String,
    /// The explicit signer source: a key directory path, or `@<id>.keys`.
    pub signer: String,
    pub not_before: String,
    pub not_after: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect: Option<Expect>,
    /// planned | dry-run | created | applied | missing | modified | conflict | pending
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fingerprints: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// The typed envelope every batch verb returns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub schema: String,
    pub batch_schema: String,
    pub batch_sha256: String,
    pub receipt_key: String,
    /// validated | planned | dry-run | applied | already-applied | pending | drift | conflict
    pub status: String,
    pub operations: Vec<OpReport>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// A completed apply's receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub schema: String,
    pub batch_sha256: String,
    pub receipt_key: String,
    pub applied_at: Rfc3339,
    pub operations: Vec<OpReport>,
}

fn source_text(src: &Source) -> String {
    match src {
        Source::Path(p) => p.display().to_string(),
        Source::Bundle(id) => format!("@{id}.bundle"),
        Source::Keys(id) => format!("@{id}.keys"),
    }
}

fn op_report(step: &Step, status: &str) -> OpReport {
    OpReport {
        id: step.id.clone(),
        command: step.command.to_string(),
        after: step.after.clone(),
        out: step.out.display().to_string(),
        parent: source_text(&step.parent),
        signer: source_text(&step.signer),
        not_before: if step.not_before_defaulted && status == "planned" {
            "at apply".into()
        } else {
            step.not_before.to_string()
        },
        not_after: step.not_after.to_string(),
        expect: step.expect.clone(),
        status: status.into(),
        bundle_id: None,
        fingerprints: vec![],
        manifest_sha256: None,
        warnings: vec![],
    }
}

fn report(batch: &Batch, status: &str, operations: Vec<OpReport>) -> Report {
    Report {
        schema: REPORT_SCHEMA.into(),
        batch_schema: BATCH_SCHEMA.into(),
        batch_sha256: batch.sha256.clone(),
        receipt_key: batch.receipt_key(),
        status: status.into(),
        operations,
        warnings: batch.warnings.clone(),
    }
}

/// The reviewable plan: the resolved DAG, nothing written.
pub fn plan(batch: &Batch) -> Report {
    let ops = batch.steps.iter().map(|s| op_report(s, "planned")).collect();
    report(batch, "planned", ops)
}

// ---------------------------------------------------------------- receipts and status

fn receipt_path(base: &Path, batch: &Batch) -> PathBuf {
    base.join(STATE_DIR)
        .join("receipts")
        .join(format!("{}.json", batch.receipt_key()))
}

fn read_receipt(base: &Path, batch: &Batch) -> Result<Option<Receipt>> {
    let path = receipt_path(base, batch);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let r: Receipt = serde_json::from_slice(&bytes).map_err(|e| {
                CertError::manifest(format!("batch receipt {} is unreadable: {e}", path.display()))
            })?;
            if r.schema != RECEIPT_SCHEMA {
                return Err(CertError::manifest(format!(
                    "batch receipt {} has schema '{}' (this build reads {RECEIPT_SCHEMA})",
                    path.display(),
                    r.schema
                )));
            }
            Ok(Some(r))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn manifest_sha(dir: &Path) -> Option<String> {
    std::fs::read(dir.join("manifest.json")).ok().map(|b| sha256_hex(&b))
}

/// Where this batch stands against the base directory: `pending`
/// (nothing applied, no output exists), `applied` (the receipt matches
/// every bundle on disk), `drift` (applied, but a bundle is missing or
/// changed), or `conflict` (outputs exist without this batch's
/// receipt, or the receipt key belongs to a different batch).
pub fn status(batch: &Batch, base: &Path) -> Result<Report> {
    let receipt = read_receipt(base, batch)?;
    let mut ops = Vec::new();
    let overall;
    match receipt {
        Some(r) if r.batch_sha256 != batch.sha256 => {
            for step in &batch.steps {
                ops.push(op_report(step, "conflict"));
            }
            let mut rep = report(batch, "conflict", ops);
            rep.warnings.push(format!(
                "receipt key '{}' was applied by a different batch (sha256 {})",
                batch.receipt_key(),
                r.batch_sha256
            ));
            return Ok(rep);
        }
        Some(r) => {
            let mut all_ok = true;
            for step in &batch.steps {
                let recorded = r.operations.iter().find(|o| o.id == step.id);
                let dir = base.join(&step.out);
                let mut op = recorded.cloned().unwrap_or_else(|| op_report(step, "missing"));
                op.status = match (recorded, manifest_sha(&dir)) {
                    (Some(rec), Some(sha)) if rec.manifest_sha256.as_deref() == Some(sha.as_str()) => {
                        "applied".into()
                    }
                    (_, Some(_)) => "modified".into(),
                    (_, None) => "missing".into(),
                };
                all_ok &= op.status == "applied";
                ops.push(op);
            }
            overall = if all_ok { "applied" } else { "drift" };
        }
        None => {
            let mut any = false;
            for step in &batch.steps {
                let exists = base.join(&step.out).exists();
                any |= exists;
                ops.push(op_report(step, if exists { "conflict" } else { "pending" }));
            }
            overall = if any { "conflict" } else { "pending" };
        }
    }
    Ok(report(batch, overall, ops))
}

/// `validate`: the static checks of [`load`] plus the base
/// directory's state; a batch that conflicts with existing output is
/// invalid.
pub fn validate(batch: &Batch, base: &Path) -> Result<Report> {
    let st = status(batch, base)?;
    match st.status.as_str() {
        "conflict" => Err(conflict_error(&st)),
        "drift" => Err(CertError::validation(
            "this batch was applied, but its bundles have since been changed or removed; \
             run `fleet cert batch status` for details",
        )),
        "applied" => Ok(Report {
            status: "already-applied".into(),
            ..st
        }),
        _ => {
            let ops = batch.steps.iter().map(|s| op_report(s, "planned")).collect();
            Ok(report(batch, "validated", ops))
        }
    }
}

fn conflict_error(st: &Report) -> CertError {
    let taken: Vec<String> = st
        .operations
        .iter()
        .filter(|o| o.status == "conflict")
        .map(|o| format!("{} ({})", o.out, o.id))
        .collect();
    if let Some(w) = st.warnings.iter().find(|w| w.starts_with("receipt key")) {
        return CertError::policy(format!(
            "{w}; refusing to reissue under a reused idempotency key"
        ));
    }
    CertError::policy(format!(
        "output already exists without this batch's receipt: {}; a batch applies only into new \
         directories",
        taken.join(", ")
    ))
}

// ---------------------------------------------------------------- apply

/// Options for [`apply`].
#[derive(Debug, Clone, Default)]
pub struct ApplyOptions {
    /// Run every step in staging, then discard it.
    pub dry_run: bool,
    /// The reviewed batch digest; required unless `dry_run`.
    pub approve: Option<String>,
}

fn unique_suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{nanos}", std::process::id())
}

#[cfg(unix)]
fn make_private_dir(path: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn make_private_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    Ok(())
}

/// Remove the staging area; a failure is reported, never hidden
/// (spec §6.5: loudly report cleanup failure; no secure-erase claim).
fn remove_staging(staging: &Path, state_created: Option<&Path>) -> Option<String> {
    let result = match std::fs::remove_dir_all(staging) {
        Ok(()) => None,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => Some(format!(
            "could not remove the staging area {} ({e}); it holds private keys: remove it by hand \
             (deletion is not a secure erase)",
            staging.display()
        )),
    };
    if let Some(state) = state_created {
        let _ = std::fs::remove_dir(state.join("staging"));
        let _ = std::fs::remove_dir(state);
    }
    result
}

fn resolve_for_run(src: &Source, base: &Path, staged: &BTreeMap<String, PathBuf>) -> PathBuf {
    match src {
        Source::Path(p) => resolve_path(base, p),
        Source::Bundle(id) => staged[id].clone(),
        Source::Keys(id) => staged[id].join("private/keys"),
    }
}

fn run_step(step: &Step, dir: &Path, base: &Path, staged: &BTreeMap<String, PathBuf>) -> Result<BundleManifest> {
    let parent = resolve_for_run(&step.parent, base, staged);
    let signer = SignerSource {
        key_dir: resolve_for_run(&step.signer, base, staged),
    };
    let manifest = match &step.request {
        Request::Issuer {
            slots,
            name,
            purpose,
            ou,
            cross_signing,
            permitted_dns,
            allow_legacy_root_strength,
        } => engine::create_issuer_set(
            &IssuerCreateRequest {
                root_bundle: parent,
                root_signer: signer,
                name: name.clone(),
                purpose: *purpose,
                ou: ou.clone(),
                slots: slots.clone(),
                cross_signing: *cross_signing,
                not_before: step.not_before,
                not_after: step.not_after,
                allow_legacy_root_strength: *allow_legacy_root_strength,
                permitted_dns: permitted_dns.clone(),
                private_store: None,
            },
            dir,
        )?,
        Request::Leaf {
            profile,
            name,
            common_name,
            dns_names,
            ip_addresses,
            csr,
            key_algorithm,
            issuer_algorithm,
            issuer_partition,
        } => engine::issue_leaf(
            &LeafIssueRequest {
                issuer_bundle: parent,
                issuer_signer: signer,
                profile: profile.clone(),
                name: name.clone(),
                common_name: common_name.clone(),
                dns_names: dns_names.clone(),
                ip_addresses: ip_addresses.clone(),
                csr: csr.as_ref().map(|c| resolve_path(base, c)),
                key_algorithm: *key_algorithm,
                issuer_algorithm: *issuer_algorithm,
                issuer_partition: issuer_partition.clone(),
                not_before: step.not_before,
                not_after: step.not_after,
                private_store: None,
            },
            dir,
        )?,
    };
    if let Some(expect) = &step.expect
        && let Some(want) = expect.certificates
        && manifest.certificates.len() != want
    {
        return Err(CertError::validation(format!(
            "expected {want} certificate(s), the step produced {}",
            manifest.certificates.len()
        )));
    }
    Ok(manifest)
}

/// Apply a batch all-or-nothing. With `dry_run`, every step runs in
/// staging and the staging area is then removed. Without it,
/// `approve` must equal the batch digest; the finished bundles move
/// into place and a receipt is written. Applying an already-applied
/// batch returns its receipt and issues nothing.
pub fn apply(batch: &Batch, base: &Path, opts: &ApplyOptions) -> Result<Report> {
    if !base.is_dir() {
        return Err(CertError::usage(format!(
            "base directory {} does not exist",
            base.display()
        )));
    }
    if !opts.dry_run {
        match opts.approve.as_deref() {
            None => {
                return Err(CertError::policy(format!(
                    "apply needs --approve <sha256> from the reviewed plan; this batch's digest is {} \
                     (run `fleet cert batch plan` to review it, or --dry-run to rehearse)",
                    batch.sha256
                )));
            }
            Some(a) if !a.eq_ignore_ascii_case(&batch.sha256) => {
                return Err(CertError::policy(format!(
                    "the batch changed since it was reviewed: approved {a}, the batch now hashes to {}",
                    batch.sha256
                )));
            }
            Some(_) => {}
        }
    }

    let st = status(batch, base)?;
    match st.status.as_str() {
        "applied" if !opts.dry_run => {
            return Ok(Report {
                status: "already-applied".into(),
                ..st
            });
        }
        "applied" => {
            return Err(CertError::policy(
                "this batch is already applied; a dry run would only rehearse issuing it again",
            ));
        }
        "drift" => {
            return Err(CertError::policy(
                "this batch was applied, but its bundles have since been changed or removed; \
                 refusing to reissue (run `fleet cert batch status`)",
            ));
        }
        "conflict" => return Err(conflict_error(&st)),
        _ => {}
    }

    let state = base.join(STATE_DIR);
    let state_created = if state.exists() { None } else { Some(state.as_path()) };
    let staging = state
        .join("staging")
        .join(format!("{}-{}", &batch.sha256[..12], unique_suffix()));
    make_private_dir(&staging)?;

    // Run every step in staging.
    let mut staged: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut results = Vec::new();
    for (i, step) in batch.steps.iter().enumerate() {
        let dir = staging.join(format!("{:03}-{}", i + 1, step.id));
        match run_step(step, &dir, base, &staged) {
            Ok(manifest) => {
                let mut op = op_report(step, if opts.dry_run { "dry-run" } else { "created" });
                op.bundle_id = Some(manifest.bundle_id.clone());
                op.fingerprints = manifest
                    .certificates
                    .iter()
                    .map(|c| c.fingerprint_sha256.clone())
                    .collect();
                op.manifest_sha256 = manifest_sha(&dir);
                op.warnings = manifest.warnings.clone();
                results.push(op);
                staged.insert(step.id.clone(), dir);
            }
            Err(e) => {
                let cleanup = remove_staging(&staging, state_created);
                let mut msg = format!(
                    "operation '{}' failed: {e}. Nothing was applied: every step ran in a staging \
                     area, which has been removed",
                    step.id
                );
                if let Some(c) = cleanup {
                    msg = format!("{msg}. WARNING: {c}");
                }
                return Err(match e {
                    CertError::Usage(_) => CertError::usage(msg),
                    CertError::Policy(_) => CertError::policy(msg),
                    CertError::Validation(_) => CertError::validation(msg),
                    _ => CertError::engine(msg),
                });
            }
        }
    }

    if opts.dry_run {
        let mut rep = report(batch, "dry-run", results);
        if let Some(c) = remove_staging(&staging, state_created) {
            rep.warnings.push(c);
        }
        return Ok(rep);
    }

    // Commit: move each finished bundle into place; undo on any failure.
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut made_dirs: Vec<PathBuf> = Vec::new();
    let mut failure: Option<String> = None;
    for step in &batch.steps {
        let target = base.join(&step.out);
        let from = staged[&step.id].clone();
        if target.exists() {
            failure = Some(format!("{} appeared while the batch ran", step.out.display()));
            break;
        }
        if let Some(parent) = target.parent() {
            let mut missing = Vec::new();
            let mut cur = parent.to_path_buf();
            while !cur.exists() {
                missing.push(cur.clone());
                if !cur.pop() {
                    break;
                }
            }
            if let Err(e) = std::fs::create_dir_all(parent) {
                failure = Some(format!("could not create {}: {e}", parent.display()));
                break;
            }
            missing.reverse();
            made_dirs.extend(missing);
        }
        if let Err(e) = std::fs::rename(&from, &target) {
            failure = Some(format!("could not move {} into place: {e}", step.out.display()));
            break;
        }
        moved.push((target, from));
    }
    if let Some(why) = failure {
        let mut undo_failures = Vec::new();
        for (target, from) in moved.iter().rev() {
            if let Err(e) = std::fs::rename(target, from) {
                undo_failures.push(format!("{} ({e})", target.display()));
            }
        }
        for dir in made_dirs.iter().rev() {
            let _ = std::fs::remove_dir(dir);
        }
        let cleanup = remove_staging(&staging, state_created);
        let mut msg = format!("commit failed: {why}. The bundles already moved were moved back");
        if !undo_failures.is_empty() {
            msg = format!(
                "commit failed: {why}. WARNING: these could not be moved back and must be removed \
                 by hand: {}",
                undo_failures.join(", ")
            );
        }
        if let Some(c) = cleanup {
            msg = format!("{msg}. WARNING: {c}");
        }
        return Err(CertError::engine(msg));
    }

    let mut rep = report(batch, "applied", results);
    let receipt = Receipt {
        schema: RECEIPT_SCHEMA.into(),
        batch_sha256: batch.sha256.clone(),
        receipt_key: batch.receipt_key(),
        applied_at: Rfc3339::now(),
        operations: rep.operations.clone(),
    };
    let path = receipt_path(base, batch);
    let written = path
        .parent()
        .map(std::fs::create_dir_all)
        .transpose()
        .and_then(|_| {
            std::fs::write(
                &path,
                serde_json::to_string_pretty(&receipt).unwrap_or_default() + "\n",
            )
        });
    if let Err(e) = written {
        rep.warnings.push(format!(
            "the bundles are in place, but the receipt {} could not be written ({e}); a re-run \
             will report a conflict instead of the receipt",
            path.display()
        ));
    }
    if let Some(c) = remove_staging(&staging, None) {
        rep.warnings.push(c);
    }
    let _ = std::fs::remove_dir(state.join("staging"));
    Ok(rep)
}

// ---------------------------------------------------------------- text

/// Human rendering of a report.
pub fn render_text(rep: &Report, file_label: &str, base_label: &str) -> String {
    let mut s = String::new();
    let headline = match rep.status.as_str() {
        "planned" => "Batch plan (nothing written)",
        "validated" => "Batch valid (nothing written)",
        "dry-run" => "Batch dry run (every step ran in staging, then staging was removed)",
        "applied" => "Batch applied",
        "already-applied" => "Batch already applied (nothing reissued)",
        "pending" => "Batch not applied yet",
        "drift" => "Batch applied, but its bundles changed since",
        "conflict" => "Batch conflicts with existing output",
        other => other,
    };
    s.push_str(&format!(
        "{headline}\n  schema {}  ·  {} operation(s)  ·  sha256 {}\n  receipt key {}\n\n",
        rep.batch_schema,
        rep.operations.len(),
        rep.batch_sha256,
        rep.receipt_key
    ));
    for (i, op) in rep.operations.iter().enumerate() {
        let after = if op.after.is_empty() {
            String::new()
        } else {
            format!("  (after: {})", op.after.join(", "))
        };
        s.push_str(&format!(
            "  {}. {}  {} -> {}  [{}]{after}\n",
            i + 1,
            op.id,
            op.command,
            op.out,
            op.status
        ));
        s.push_str(&format!("       parent: {}\n", op.parent));
        s.push_str(&format!("       signer: {}  (explicit, spec §24.3)\n", op.signer));
        s.push_str(&format!("       valid:  {} .. {}\n", op.not_before, op.not_after));
        if let Some(id) = &op.bundle_id {
            s.push_str(&format!("       bundle: {id}\n"));
        }
        for fp in &op.fingerprints {
            s.push_str(&format!("       cert:   {fp}\n"));
        }
    }
    for w in &rep.warnings {
        s.push_str(&format!("\nwarning: {w}"));
    }
    if !rep.warnings.is_empty() {
        s.push('\n');
    }
    if rep.status == "planned" || rep.status == "validated" {
        s.push_str(&format!(
            "\nTo apply exactly this batch:\n  fleet cert batch apply --file {file_label} --base {base_label} --approve {}\n\
             Add --dry-run to rehearse every step in a staging area that is then removed.\n",
            rep.batch_sha256
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_name_the_signer_explicitly() {
        assert_eq!(
            parse_source("x", "issuer", "@web", Field::Bundle).unwrap(),
            Source::Bundle("web".into())
        );
        assert_eq!(
            parse_source("x", "issuer_key_dir", "@web.keys", Field::Keys).unwrap(),
            Source::Keys("web".into())
        );
        let err = parse_source("x", "issuer_key_dir", "@web", Field::Keys).unwrap_err();
        assert!(err.to_string().contains("explicitly"), "{err}");
        assert!(parse_source("x", "issuer", "@web.keys", Field::Bundle).is_err());
        assert!(parse_source("x", "issuer", "@web.other", Field::Bundle).is_err());
        assert_eq!(
            parse_source("x", "issuer", "issuers/web", Field::Bundle).unwrap(),
            Source::Path(PathBuf::from("issuers/web"))
        );
    }

    #[test]
    fn outputs_stay_under_the_base() {
        assert_eq!(check_out("x", "leaves/svc").unwrap(), PathBuf::from("leaves/svc"));
        for bad in ["", "../escape", "/abs", "a/../b", "./a", ".fleet-batch/x"] {
            assert!(check_out("x", bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn no_interpolation_anywhere() {
        let v: Value = serde_json::json!({"a": ["ok", {"b": "${jobs.x}"}]});
        assert!(scan_strings("x", "with", &v).is_err());
        let v: Value = serde_json::json!({"a": "$(id)"});
        assert!(scan_strings("x", "with", &v).is_err());
        let v: Value = serde_json::json!({"a": "plain", "n": 3});
        assert!(scan_strings("x", "with", &v).is_ok());
    }
}
