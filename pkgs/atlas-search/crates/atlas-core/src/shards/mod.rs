//! The atlas core tier as a consumer sees it: a set of TursoDB scan
//! shards (`atlas-core-<tier>-<topic>-<nn>`, issue #5669 shard map)
//! that together replace the monolith for the vector branch
//! (issue #5755, design #5667 section 4.2 / 5).
//!
//! * **Discovery** ([`discover`]): a [`ShardLister`] names candidate
//!   databases, each candidate's `atlas_manifest` table is the
//!   self-description that is read and cross-checked against the
//!   name (manifest fan-out at startup). New shards join by
//!   existing, never by code change. The registry-of-record
//!   alternative (a registry DB) is deliberately not used: the pilot
//!   registry DB was deleted in the 2026-11-06 spike cleanup and the
//!   staging-tier registry (#5669) tracks *staging units*, not scan
//!   shards. When that registry grows a shard table it becomes one
//!   more [`ShardLister`].
//! * **Tier ceiling is physical**: a [`TierCeiling`] decides which
//!   tiers are connected to at all. A shard above the ceiling (or a
//!   tier the connector holds no credential for) is never
//!   connected, never probed and never scanned. Enforcement points:
//!   [`discover`] (the service's maximum), [`ShardSet::select`]
//!   (the per-caller ceiling) and [`scan::scan_one`] (last line of
//!   defence). See [`TierCeiling`] for the ladder mapping.
//! * **Fan-out** ([`scan`]): exact cosine scan, top-k per shard, in
//!   parallel, merged by distance with internal shards winning ties.

pub mod local;
pub mod scan;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value as Json;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::filters::Sensitivity;
use crate::turso::{HttpTransport, PipelineTransport};
use crate::types::Stmt;

pub use local::{DirLister, LocalShardTransport};
pub use scan::{FanoutQuery, FanoutResult, ShardHit, ShardedBackend, fanout_scan, merge_top_k};

/// Name prefix shared by every core-tier database (scan shards and
/// content stores).
pub const SHARD_PREFIX: &str = "atlas-core-";

// ------------------------------------------------------------- tier -----

/// A physical sensitivity level (design 4.2): separate databases,
/// separate credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tier {
    Public,
    Internal,
    Secret,
}

impl Tier {
    pub const ALL: [Tier; 3] = [Tier::Public, Tier::Internal, Tier::Secret];

    pub fn name(self) -> &'static str {
        match self {
            Tier::Public => "public",
            Tier::Internal => "internal",
            Tier::Secret => "secret",
        }
    }

    pub fn parse(s: &str) -> Option<Tier> {
        match s {
            "public" => Some(Tier::Public),
            "internal" => Some(Tier::Internal),
            "secret" => Some(Tier::Secret),
            _ => None,
        }
    }

    /// Height on the ceiling ladder: a ceiling admits every tier at
    /// or below its own.
    pub fn level(self) -> u8 {
        match self {
            Tier::Public => 0,
            Tier::Internal => 1,
            Tier::Secret => 2,
        }
    }

    /// Merge preference when distances tie: internal (the default
    /// level, the bulk of the corpus) first, then public, then
    /// secret.
    pub fn merge_rank(self) -> u8 {
        match self {
            Tier::Internal => 0,
            Tier::Public => 1,
            Tier::Secret => 2,
        }
    }

    /// The environment variable naming this tier's read-only Turso
    /// token. Secret has its own variable: it is never derived from
    /// another tier's credential.
    pub fn token_env(self) -> &'static str {
        match self {
            Tier::Public => "ATLAS_TURSO_TOKEN_PUBLIC",
            Tier::Internal => "ATLAS_TURSO_TOKEN_INTERNAL",
            Tier::Secret => "ATLAS_TURSO_TOKEN_SECRET",
        }
    }
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The highest tier a caller may scan.
///
/// The row-level sensitivity ladder maps onto tiers as in the #5669
/// restore (secret-bearing -> secret; personal and local-only ->
/// internal; shareable -> public):
///
/// | `--max-sensitivity` | ceiling tier |
/// |---|---|
/// | shareable | public |
/// | personal (default), local-only | internal |
/// | secret-bearing | secret |
///
/// So the default fan-out can never include the secret tier; it
/// takes an explicit `secret-bearing` ceiling *and* a secret
/// credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TierCeiling(pub Tier);

impl TierCeiling {
    pub fn from_sensitivity(s: Sensitivity) -> TierCeiling {
        TierCeiling(match s {
            Sensitivity::Shareable => Tier::Public,
            Sensitivity::Personal | Sensitivity::LocalOnly => Tier::Internal,
            Sensitivity::SecretBearing => Tier::Secret,
        })
    }

    pub fn admits(self, tier: Tier) -> bool {
        tier.level() <= self.0.level()
    }
}

// ------------------------------------------------------------- names ----

/// `atlas-core-<tier>-<topic>-<nn>` -> (tier, topic, part).
/// Content stores (`atlas-core-<tier>-content`) and anything else do
/// not parse: they are not scan shards.
pub fn parse_shard_name(name: &str) -> Option<(Tier, String, u32)> {
    let rest = name.strip_prefix(SHARD_PREFIX)?;
    let (tier_s, rest) = rest.split_once('-')?;
    let tier = Tier::parse(tier_s)?;
    let (topic, nn) = rest.rsplit_once('-')?;
    if topic.is_empty() || nn.len() < 2 || !nn.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((tier, topic.to_string(), nn.parse().ok()?))
}

// ---------------------------------------------------------- endpoints ---

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShardLocation {
    /// A hosted Turso database (`<host>`, no scheme).
    Turso { host: String },
    /// A local SQLite/libSQL file (tests, embedded replicas).
    File(PathBuf),
}

/// A candidate database a lister found. Not yet trusted: its
/// manifest has not been read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardEndpoint {
    pub name: String,
    pub location: ShardLocation,
}

/// Where candidate shards come from. Implementations only *name*
/// databases; they never open one.
pub trait ShardLister {
    fn list(&self) -> Result<Vec<ShardEndpoint>>;
}

/// A fixed list (service config, tests).
pub struct StaticLister(pub Vec<ShardEndpoint>);

impl ShardLister for StaticLister {
    fn list(&self) -> Result<Vec<ShardEndpoint>> {
        Ok(self.0.clone())
    }
}

/// The Turso platform API database listing
/// (`GET /v1/organizations/{org}/databases`, response
/// `{"databases":[{"Name":..,"Hostname":..},..]}`). The bearer token
/// is a platform API token read from the environment variable named
/// by `token_env`; never logged.
pub struct TursoApiLister {
    pub org: String,
    pub token: String,
}

pub const TURSO_API_TOKEN_ENV: &str = "TURSO_API_TOKEN";

impl TursoApiLister {
    pub fn from_env(org: &str) -> Result<TursoApiLister> {
        match std::env::var(TURSO_API_TOKEN_ENV) {
            Ok(t) if !t.is_empty() => Ok(TursoApiLister {
                org: org.to_string(),
                token: t,
            }),
            _ => Err(Error::fatal(format!(
                "shard discovery: {TURSO_API_TOKEN_ENV} is not set"
            ))),
        }
    }
}

/// Parse the platform API listing into endpoints (only `atlas-core-`
/// names; everything else in the org is none of our business).
pub fn parse_database_list(body: &Json) -> Vec<ShardEndpoint> {
    body.get("databases")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
        .filter_map(|d| {
            let name = d.get("Name").and_then(Json::as_str)?;
            if !name.starts_with(SHARD_PREFIX) {
                return None;
            }
            let host = d
                .get("Hostname")
                .or_else(|| d.get("hostname"))
                .and_then(Json::as_str)?;
            Some(ShardEndpoint {
                name: name.to_string(),
                location: ShardLocation::Turso {
                    host: host.to_string(),
                },
            })
        })
        .collect()
}

impl ShardLister for TursoApiLister {
    fn list(&self) -> Result<Vec<ShardEndpoint>> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .into();
        let url = format!(
            "https://api.turso.tech/v1/organizations/{}/databases",
            self.org
        );
        let mut resp = agent
            .get(&url)
            .header("Authorization", &format!("Bearer {}", self.token))
            .call()
            .map_err(|e| Error::backend(format!("shard discovery: list request failed: {e}")))?;
        let status = resp.status().as_u16();
        let mut text = String::new();
        resp.body_mut()
            .as_reader()
            .read_to_string(&mut text)
            .map_err(|e| Error::backend(format!("shard discovery: list read failed: {e}")))?;
        if status != 200 {
            return Err(Error::fatal(format!(
                "shard discovery: database list returned HTTP {status}"
            )));
        }
        let body: Json = serde_json::from_str(&text)
            .map_err(|e| Error::backend(format!("shard discovery: bad list JSON: {e}")))?;
        Ok(parse_database_list(&body))
    }
}

// ---------------------------------------------------------- connector ---

/// A shard's query channel. The Turso HTTP transport and the local
/// file transport both qualify.
pub trait ShardTransport: PipelineTransport + Send + Sync {}
impl<T: PipelineTransport + Send + Sync> ShardTransport for T {}

/// Opens shards. `can_open` is the credential half of the physical
/// ceiling: a tier without a credential is never connected.
pub trait ShardConnector: Send + Sync {
    fn can_open(&self, tier: Tier) -> bool;
    fn connect(&self, endpoint: &ShardEndpoint, tier: Tier) -> Result<Arc<dyn ShardTransport>>;
}

/// Per-tier read-only Turso tokens. Held as values in memory only;
/// never printed ([`fmt::Debug`] shows tiers, not tokens).
#[derive(Clone, Default)]
pub struct TierCredentials {
    tokens: BTreeMap<Tier, String>,
}

impl fmt::Debug for TierCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TierCredentials")
            .field("tiers", &self.tokens.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl TierCredentials {
    pub fn new() -> TierCredentials {
        TierCredentials::default()
    }

    pub fn with(mut self, tier: Tier, token: impl Into<String>) -> TierCredentials {
        self.tokens.insert(tier, token.into());
        self
    }

    /// One token per tier from [`Tier::token_env`]; unset or empty
    /// variables leave the tier closed.
    pub fn from_env() -> TierCredentials {
        let mut c = TierCredentials::new();
        for tier in Tier::ALL {
            if let Ok(t) = std::env::var(tier.token_env())
                && !t.is_empty()
            {
                c.tokens.insert(tier, t);
            }
        }
        c
    }

    pub fn has(&self, tier: Tier) -> bool {
        self.tokens.contains_key(&tier)
    }
}

/// The production connector: Turso shards over HTTPS with the
/// tier's own token, local files directly (a file is already on
/// this machine; the tier ceiling still gates it).
pub struct StdConnector {
    pub creds: TierCredentials,
    /// Local files need no credential (they are already on this
    /// machine). The tier ceiling still gates them.
    pub local_files: bool,
}

impl StdConnector {
    pub fn local_files() -> StdConnector {
        StdConnector {
            creds: TierCredentials::new(),
            local_files: true,
        }
    }
}

impl ShardConnector for StdConnector {
    fn can_open(&self, tier: Tier) -> bool {
        self.local_files || self.creds.has(tier)
    }

    fn connect(&self, endpoint: &ShardEndpoint, tier: Tier) -> Result<Arc<dyn ShardTransport>> {
        match &endpoint.location {
            ShardLocation::Turso { host } => {
                let token = self.creds.tokens.get(&tier).ok_or_else(|| {
                    Error::fatal(format!(
                        "shard {}: no {tier}-tier credential ({})",
                        endpoint.name,
                        tier.token_env()
                    ))
                })?;
                Ok(Arc::new(HttpTransport::new(host, token.clone())))
            }
            ShardLocation::File(path) => Ok(Arc::new(LocalShardTransport::open(path)?)),
        }
    }
}

// ----------------------------------------------------------- manifest ---

/// The embedding model + pipeline identity that produced every
/// shard's vectors (#6060 decision record, contract points M1/M2,
/// #7646): one embedding model for every Atlas tier. Recorded per
/// shard in the manifest's `embedding_model` key and checked before
/// fan-out, so vectors embedded by different models are never merged
/// as if they were comparable.
pub const EMBEDDING_MODEL_ID: &str = "snowflake-arctic-embed-m-v2.0";

/// A shard's `atlas_manifest` (key/value self-description).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub shard: String,
    pub tier: Tier,
    pub topic: String,
    pub part: Option<u32>,
    pub expected_chunks: Option<u64>,
    pub metric: String,
    /// The `embedding_model` key: model + pipeline identity of the
    /// shard's vectors. `None` when the key is absent (a shard
    /// written before the key existed); discovery refuses `None`
    /// in strict mode (#7646).
    pub embedding_model: Option<String>,
}

impl Manifest {
    pub fn from_pairs(pairs: &BTreeMap<String, String>) -> Result<Manifest> {
        let need = |k: &str| {
            pairs
                .get(k)
                .cloned()
                .ok_or_else(|| Error::fatal(format!("atlas_manifest has no `{k}` key")))
        };
        let tier_s = need("tier")?;
        let tier = Tier::parse(&tier_s)
            .ok_or_else(|| Error::fatal(format!("atlas_manifest tier `{tier_s}` is unknown")))?;
        Ok(Manifest {
            shard: need("shard")?,
            tier,
            topic: need("topic")?,
            part: pairs.get("part").and_then(|p| p.parse().ok()),
            expected_chunks: pairs.get("expected_chunks").and_then(|p| p.parse().ok()),
            metric: need("metric")?,
            embedding_model: pairs.get("embedding_model").cloned(),
        })
    }

    /// Read the manifest over a shard transport.
    pub fn read(t: &dyn ShardTransport) -> Result<Manifest> {
        let rows = t
            .pipeline(&[Stmt::new(
                "SELECT \"key\" AS k, value AS v FROM atlas_manifest",
                vec![],
            )])?
            .into_iter()
            .next()
            .unwrap_or_default();
        let pairs: BTreeMap<String, String> = rows
            .iter()
            .filter_map(|r| Some((r.get_text("k")?.to_string(), r.get_text("v")?.to_string())))
            .collect();
        Manifest::from_pairs(&pairs)
    }
}

// ---------------------------------------------------------------- set ---

#[derive(Debug, Clone)]
pub struct ShardInfo {
    pub name: String,
    pub tier: Tier,
    pub topic: String,
    pub part: u32,
    pub expected_chunks: Option<u64>,
    /// The shard's validated `embedding_model` identity (#7646).
    pub embedding_model: String,
    /// `count(*)` of `chunks` at discovery time.
    pub chunks: u64,
    pub endpoint: ShardEndpoint,
}

/// A candidate that did not join the set, with the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    pub name: String,
    pub reason: String,
}

/// The discovered scan shards, ordered internal-first
/// (`merge_rank`, then topic, then part).
pub struct ShardSet {
    shards: Vec<ShardInfo>,
    transports: BTreeMap<String, Arc<dyn ShardTransport>>,
    vec_table: String,
    /// The highest tier this set was discovered under (the
    /// service's maximum). Per-caller ceilings can only narrow it.
    discovery_ceiling: TierCeiling,
    pub rejections: Vec<Rejection>,
}

impl ShardSet {
    pub fn shards(&self) -> &[ShardInfo] {
        &self.shards
    }

    pub fn vec_table(&self) -> &str {
        &self.vec_table
    }

    pub fn discovery_ceiling(&self) -> TierCeiling {
        self.discovery_ceiling
    }

    pub(crate) fn transport(&self, name: &str) -> Option<&Arc<dyn ShardTransport>> {
        self.transports.get(name)
    }

    /// The caller-ceiling enforcement point: the shards a caller
    /// with `ceiling` may scan, optionally scoped to `topics`
    /// (manifest topics; `None` = all). Fails closed when the
    /// ceiling's own tier was not part of this set's discovery
    /// (e.g. a secret-bearing caller against a set discovered
    /// without the secret credential) rather than quietly answering
    /// from fewer tiers than the caller asked for.
    pub fn select(
        &self,
        ceiling: TierCeiling,
        topics: Option<&[String]>,
    ) -> Result<Vec<&ShardInfo>> {
        if !self.discovery_ceiling.admits(ceiling.0) {
            return Err(Error::fatal(format!(
                "shard fan-out: ceiling `{}` is above what this consumer was opened for \
                 (`{}`); a {}-capable credential is required, explicitly",
                ceiling.0, self.discovery_ceiling.0, ceiling.0
            )));
        }
        Ok(self
            .shards
            .iter()
            .filter(|s| ceiling.admits(s.tier))
            .filter(|s| topics.is_none_or(|ts| ts.iter().any(|t| t == &s.topic)))
            .collect())
    }

    /// Canonical text naming the shard set and per-shard counts:
    /// the material for the result-cache snapshot id (#5668), so a
    /// re-shard or a promotion batch changes it.
    pub fn snapshot_material(&self) -> String {
        let mut lines: Vec<String> = self
            .shards
            .iter()
            .map(|s| format!("{}\t{}\t{}", s.name, s.tier, s.chunks))
            .collect();
        lines.sort();
        lines.join("\n")
    }

    /// Hex sha256 of [`ShardSet::snapshot_material`].
    pub fn snapshot_id(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.snapshot_material().as_bytes());
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }
}

// ---------------------------------------------------------- discovery ---

#[derive(Debug, Clone)]
pub struct DiscoveryOptions {
    /// The service's maximum tier. Shards above it are never
    /// connected.
    pub ceiling: TierCeiling,
    /// The embedding-model identity this consumer embeds queries
    /// with; shards recording a different (or no) identity are
    /// refused at probe (#7646). Defaults to the mandated model.
    pub embedding_model: String,
    pub vec_table: String,
    /// Integrity failures (unreadable manifest, manifest disagreeing
    /// with the shard name, wrong metric) normally abort discovery
    /// so a search never silently covers fewer shards than the
    /// corpus has. `lenient` records them as [`Rejection`]s instead.
    pub lenient: bool,
}

impl DiscoveryOptions {
    pub fn new(ceiling: TierCeiling) -> DiscoveryOptions {
        DiscoveryOptions {
            ceiling,
            embedding_model: EMBEDDING_MODEL_ID.to_string(),
            vec_table: crate::types::DEFAULT_VEC_TABLE.to_string(),
            lenient: false,
        }
    }
}

struct Candidate {
    endpoint: ShardEndpoint,
    tier: Tier,
    topic: String,
    part: u32,
}

fn probe(
    connector: &dyn ShardConnector,
    c: &Candidate,
    expected_embedding_model: &str,
) -> Result<(ShardInfo, Arc<dyn ShardTransport>)> {
    let t = connector.connect(&c.endpoint, c.tier)?;
    let m = Manifest::read(t.as_ref())
        .map_err(|e| Error::fatal(format!("shard {}: {e}", c.endpoint.name)))?;
    let mismatch = |what: &str, got: String, want: String| {
        Error::fatal(format!(
            "shard {}: manifest {what} `{got}` disagrees with its name (`{want}`)",
            c.endpoint.name
        ))
    };
    if m.shard != c.endpoint.name {
        return Err(mismatch("shard", m.shard, c.endpoint.name.clone()));
    }
    if m.tier != c.tier {
        return Err(mismatch("tier", m.tier.to_string(), c.tier.to_string()));
    }
    if m.topic != c.topic {
        return Err(mismatch("topic", m.topic, c.topic.clone()));
    }
    if m.metric != "cosine" {
        return Err(Error::fatal(format!(
            "shard {}: metric `{}` is not cosine; this consumer scans with vector_distance_cos",
            c.endpoint.name, m.metric
        )));
    }
    // M1/M2 (#7646): the shard's vectors must have been embedded by
    // the same model + pipeline as this consumer's query vectors.
    // A missing key is a refusal too — provenance unknown is not
    // provenance shared. Errors name the shard and both identities.
    match &m.embedding_model {
        None => {
            return Err(Error::fatal(format!(
                "shard {}: atlas_manifest has no `embedding_model` key; this consumer embeds with `{expected_embedding_model}` and a shard of unknown embedding provenance must not be scanned",
                c.endpoint.name
            )));
        }
        Some(got) if got != expected_embedding_model => {
            return Err(Error::fatal(format!(
                "shard {}: embedding_model `{got}` does not match this consumer's `{expected_embedding_model}`; vectors from different embedding models are not comparable",
                c.endpoint.name
            )));
        }
        _ => {}
    }
    let n = t
        .pipeline(&[Stmt::new("SELECT COUNT(*) AS n FROM chunks", vec![])])?
        .into_iter()
        .next()
        .and_then(|rows| rows.into_iter().next())
        .and_then(|r| r.get_i64("n"))
        .ok_or_else(|| {
            Error::backend(format!("shard {}: chunk count unreadable", c.endpoint.name))
        })?;
    Ok((
        ShardInfo {
            name: c.endpoint.name.clone(),
            tier: c.tier,
            topic: c.topic.clone(),
            part: m.part.unwrap_or(c.part),
            expected_chunks: m.expected_chunks,
            embedding_model: m.embedding_model.clone().unwrap_or_default(),
            chunks: n.max(0) as u64,
            endpoint: c.endpoint.clone(),
        },
        t,
    ))
}

/// Resolve the shard set: list candidates, drop everything that is
/// not a scan shard or sits above the ceiling / without a
/// credential (without connecting), then read each remaining
/// manifest in parallel.
pub fn discover(
    lister: &dyn ShardLister,
    connector: &dyn ShardConnector,
    opts: &DiscoveryOptions,
) -> Result<ShardSet> {
    if !connector.can_open(opts.ceiling.0) {
        return Err(Error::fatal(format!(
            "shard discovery: ceiling `{}` needs a {}-tier credential ({}); none is available",
            opts.ceiling.0,
            opts.ceiling.0,
            opts.ceiling.0.token_env()
        )));
    }
    let mut rejections = Vec::new();
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();
    for ep in lister.list()? {
        if !ep.name.starts_with(SHARD_PREFIX) {
            continue;
        }
        let Some((tier, topic, part)) = parse_shard_name(&ep.name) else {
            rejections.push(Rejection {
                name: ep.name,
                reason: "not a scan shard name (content store or unknown)".to_string(),
            });
            continue;
        };
        if !seen.insert(ep.name.clone()) {
            continue;
        }
        if !opts.ceiling.admits(tier) {
            rejections.push(Rejection {
                name: ep.name,
                reason: format!("tier {tier} is above the ceiling {}", opts.ceiling.0),
            });
            continue;
        }
        if !connector.can_open(tier) {
            rejections.push(Rejection {
                name: ep.name,
                reason: format!("no {tier}-tier credential"),
            });
            continue;
        }
        candidates.push(Candidate {
            endpoint: ep,
            tier,
            topic,
            part,
        });
    }
    candidates.sort_by(|a, b| {
        (a.tier.merge_rank(), &a.topic, a.part, &a.endpoint.name).cmp(&(
            b.tier.merge_rank(),
            &b.topic,
            b.part,
            &b.endpoint.name,
        ))
    });

    let probed: Vec<Result<(ShardInfo, Arc<dyn ShardTransport>)>> = std::thread::scope(|scope| {
        let handles: Vec<_> = candidates
            .iter()
            .map(|c| scope.spawn(move || probe(connector, c, &opts.embedding_model)))
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err(Error::backend("shard probe worker panicked")))
            })
            .collect()
    });

    let mut shards = Vec::new();
    let mut transports = BTreeMap::new();
    for (c, res) in candidates.iter().zip(probed) {
        match res {
            Ok((info, t)) => {
                transports.insert(info.name.clone(), t);
                shards.push(info);
            }
            Err(e) if opts.lenient => rejections.push(Rejection {
                name: c.endpoint.name.clone(),
                reason: e.to_string(),
            }),
            Err(e) => return Err(e),
        }
    }
    Ok(ShardSet {
        shards,
        transports,
        vec_table: opts.vec_table.clone(),
        discovery_ceiling: opts.ceiling,
        rejections,
    })
}

/// Org slug the hosted core shards live in (the `turso:` spec's
/// default; override with `turso:<org>`).
pub const DEFAULT_TURSO_ORG: &str = "dezren39";

/// Open the shard set named by a flag/env spec, for a caller whose
/// row-level ceiling is `ceiling`:
///
/// * `dir:<path>` — local `atlas-core-*.db` files (no credentials);
/// * `turso` / `turso:<org>` — hosted shards: names from the Turso
///   platform API (`TURSO_API_TOKEN`), per-tier read-only tokens from
///   `ATLAS_TURSO_TOKEN_{PUBLIC,INTERNAL,SECRET}`.
///
/// Discovery is opened at the caller's own ceiling, so the default
/// (personal) never lists, connects to, or needs a credential for the
/// secret tier.
pub fn open_from_spec(spec: &str, ceiling: TierCeiling, vec_table: &str) -> Result<Arc<ShardSet>> {
    let mut opts = DiscoveryOptions::new(ceiling);
    opts.vec_table = vec_table.to_string();
    let set = if let Some(dir) = spec.strip_prefix("dir:") {
        discover(
            &DirLister { dir: dir.into() },
            &StdConnector::local_files(),
            &opts,
        )?
    } else if spec == "turso" || spec.starts_with("turso:") {
        let org = spec.strip_prefix("turso:").unwrap_or(DEFAULT_TURSO_ORG);
        discover(
            &TursoApiLister::from_env(org)?,
            &StdConnector {
                creds: TierCredentials::from_env(),
                local_files: false,
            },
            &opts,
        )?
    } else {
        return Err(Error::fatal(format!(
            "shard spec `{spec}` is not `dir:<path>` or `turso[:<org>]`"
        )));
    };
    Ok(Arc::new(set))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn shard_names_parse() {
        assert_eq!(
            parse_shard_name("atlas-core-internal-work-engineering-01"),
            Some((Tier::Internal, "work-engineering".to_string(), 1))
        );
        assert_eq!(
            parse_shard_name("atlas-core-secret-main-01"),
            Some((Tier::Secret, "main".to_string(), 1))
        );
        assert_eq!(
            parse_shard_name("atlas-core-internal-misc-12"),
            Some((Tier::Internal, "misc".to_string(), 12))
        );
        for bad in [
            "atlas-core-internal-content",
            "atlas-core-secret-content",
            "atlas-core-bogus-media-01",
            "atlas-core-internal-media",
            "atlas-core-internal-media-1",
            "atlas-core-internal--01",
            "atlas",
            "atlas-ask-ledger",
        ] {
            assert_eq!(parse_shard_name(bad), None, "{bad}");
        }
    }

    #[test]
    fn ceiling_ladder_mapping() {
        let c = |s| TierCeiling::from_sensitivity(Sensitivity::from_name(s).unwrap());
        assert_eq!(c("shareable"), TierCeiling(Tier::Public));
        assert_eq!(c("personal"), TierCeiling(Tier::Internal));
        assert_eq!(c("local-only"), TierCeiling(Tier::Internal));
        assert_eq!(c("secret-bearing"), TierCeiling(Tier::Secret));
        assert!(c("personal").admits(Tier::Public));
        assert!(c("personal").admits(Tier::Internal));
        assert!(!c("personal").admits(Tier::Secret));
        assert!(!c("shareable").admits(Tier::Internal));
        assert!(c("secret-bearing").admits(Tier::Secret));
    }

    #[test]
    fn database_list_shape() {
        let body = json!({"databases": [
            {"Name": "atlas", "Hostname": "atlas-o.turso.io"},
            {"Name": "atlas-core-internal-media-01", "Hostname": "atlas-core-internal-media-01-o.turso.io"},
            {"Name": "atlas-core-secret-content", "Hostname": "atlas-core-secret-content-o.turso.io"},
        ]});
        let eps = parse_database_list(&body);
        assert_eq!(
            eps.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["atlas-core-internal-media-01", "atlas-core-secret-content"]
        );
        assert_eq!(
            eps[0].location,
            ShardLocation::Turso {
                host: "atlas-core-internal-media-01-o.turso.io".into()
            }
        );
        assert!(parse_database_list(&json!({})).is_empty());
    }

    #[test]
    fn credentials_debug_hides_tokens() {
        let c = TierCredentials::new().with(Tier::Internal, "s3cr3t-value");
        let shown = format!("{c:?}");
        assert!(!shown.contains("s3cr3t-value"));
        assert!(shown.contains("Internal"));
        assert!(c.has(Tier::Internal) && !c.has(Tier::Secret));
    }
}
