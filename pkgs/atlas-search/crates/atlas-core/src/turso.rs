//! The Turso backend: libsql HTTPS pipeline (`/v2/pipeline`) with a
//! read-only token, mirroring the Python's `TursoBackend` and its
//! token resolution order (env -> Bitwarden by name -> turso CLI
//! mint cache -> turso CLI mint). It never issues writes: every
//! statement this crate builds is a SELECT, and the token it holds
//! is read-only by construction of the resolution chain.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde_json::{Map, Value as Json, json};

use crate::VectorBackend;
use crate::backend::SearchBackend;
use crate::error::{Error, Result};
use crate::filters::{
    Filters, TURSO_SHAPE, branch_queries, fts_stmt, survivor_dist_stmt, turso_allowed_sens_stmt,
    turso_allowed_session_stmt, vector_top_k_stmt,
};
use crate::manifest::{LoadedManifest, Manifest, builtin_manifest, parse_manifest_rows};
use crate::types::{
    BranchHit, BranchPools, BranchQuery, ChunkDetail, MessageRow, NeighborRow, Row, Stmt, Value,
    detail_from_row,
};

pub const TURSO_HOST: &str = "atlas-dezren39.aws-us-east-1.turso.io";
pub const TURSO_DB: &str = "atlas";
const BWS_TOKEN_NAMES: [&str; 3] = [
    "turso.atlas-ro-token",
    "turso.atlas-read-only-token",
    "atlas-ro",
];

// ------------------------------------------------------------ hrana -----

fn hrana_arg(v: &Value) -> Json {
    match v {
        Value::Null => json!({"type": "null"}),
        Value::Integer(i) => json!({"type": "integer", "value": i.to_string()}),
        Value::Float(f) => json!({"type": "float", "value": f}),
        Value::Text(s) => json!({"type": "text", "value": s}),
        Value::Blob(b) => json!({
            "type": "blob",
            "value": base64::engine::general_purpose::STANDARD.encode(b),
        }),
    }
}

fn hrana_val(v: &Json) -> Value {
    if v.is_null() {
        return Value::Null;
    }
    match v.get("type").and_then(Json::as_str) {
        Some("integer") => v
            .get("value")
            .and_then(Json::as_str)
            .and_then(|s| s.parse::<i64>().ok())
            .map(Value::Integer)
            .unwrap_or(Value::Null),
        Some("float") => v
            .get("value")
            .and_then(Json::as_f64)
            .map(Value::Float)
            .unwrap_or(Value::Null),
        Some("text") => v
            .get("value")
            .and_then(Json::as_str)
            .map(|s| Value::Text(s.to_string()))
            .unwrap_or(Value::Null),
        Some("blob") => v
            .get("value")
            .and_then(Json::as_str)
            .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
            .map(Value::Blob)
            .unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

/// The query-vector JSON the Python builds with
/// `json.dumps([float(x) for x in qvec])`.
pub fn qvec_json(qvec: &[f32]) -> String {
    let vals: Vec<f64> = qvec.iter().map(|x| *x as f64).collect();
    serde_json::to_string(&vals).unwrap_or_else(|_| "[]".to_string())
}

// --------------------------------------------------------- transport ----

/// The pipeline seam: one batched call, one row-set per statement
/// that produced a result. The HTTP implementation is production;
/// tests replay recorded fixtures through a fake.
pub trait PipelineTransport {
    fn pipeline(&self, stmts: &[Stmt]) -> Result<Vec<Vec<Row>>>;

    /// Execute a pipeline and return optional per-result row meters. The
    /// default preserves compatibility with transports without meters.
    fn pipeline_metered(
        &self,
        stmts: &[Stmt],
    ) -> Result<(Vec<Vec<Row>>, Vec<Option<StatementUsage>>)> {
        let rows = self.pipeline(stmts)?;
        let usage = vec![None; rows.len()];
        Ok((rows, usage))
    }
}

/// Per-statement Turso scan/write counters. Missing or malformed counters
/// remain unknown rather than being reported as zero.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatementUsage {
    pub rows_read: Option<u64>,
    pub rows_written: Option<u64>,
}

pub struct HttpTransport {
    agent: ureq::Agent,
    url: String,
    token: String,
}

impl HttpTransport {
    pub fn new(host: &str, token: String) -> HttpTransport {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(300)))
            .http_status_as_error(false)
            .build()
            .into();
        HttpTransport {
            agent,
            url: format!("https://{host}/v2/pipeline"),
            token,
        }
    }

    fn pipeline_response(&self, stmts: &[Stmt]) -> Result<Json> {
        let mut requests: Vec<Json> = stmts
            .iter()
            .map(|s| {
                json!({
                    "type": "execute",
                    "stmt": {
                        "sql": s.sql,
                        "args": s.args.iter().map(hrana_arg).collect::<Vec<_>>(),
                    },
                })
            })
            .collect();
        requests.push(json!({"type": "close"}));
        let body = json!({"requests": requests}).to_string();
        let mut resp = self
            .agent
            .post(&self.url)
            .header("Authorization", &format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .send(body.as_str())
            .map_err(|e| Error::backend(format!("turso backend: request failed: {e}")))?;
        let status = resp.status().as_u16();
        let mut text = String::new();
        resp.body_mut()
            .as_reader()
            .read_to_string(&mut text)
            .map_err(|e| Error::backend(format!("turso backend: read failed: {e}")))?;
        if status != 200 {
            let snippet: String = text.chars().take(200).collect();
            return Err(Error::fatal(format!(
                "turso backend: HTTP {status}: {snippet}"
            )));
        }
        serde_json::from_str(&text)
            .map_err(|e| Error::backend(format!("turso backend: bad JSON: {e}")))
    }
}

impl PipelineTransport for HttpTransport {
    fn pipeline(&self, stmts: &[Stmt]) -> Result<Vec<Vec<Row>>> {
        let out = self.pipeline_response(stmts)?;
        Ok(parse_pipeline_response(&out)?.0)
    }

    fn pipeline_metered(
        &self,
        stmts: &[Stmt],
    ) -> Result<(Vec<Vec<Row>>, Vec<Option<StatementUsage>>)> {
        let out = self.pipeline_response(stmts)?;
        parse_pipeline_response(&out)
    }
}

fn parse_pipeline_response(out: &Json) -> Result<(Vec<Vec<Row>>, Vec<Option<StatementUsage>>)> {
    let mut results = Vec::new();
    let mut usage = Vec::new();
    for res in out
        .get("results")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
    {
        if res.get("type").and_then(Json::as_str) == Some("error") {
            let msg = res
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(Json::as_str)
                .unwrap_or("");
            return Err(Error::fatal(format!("turso backend: SQL error: {msg}")));
        }
        let Some(result) = res
            .get("response")
            .and_then(|r| r.get("result"))
            .filter(|r| !r.is_null())
        else {
            continue;
        };
        let rows_read = result.get("rows_read").and_then(Json::as_u64);
        let rows_written = result.get("rows_written").and_then(Json::as_u64);
        let meter = (result.get("rows_read").is_some() || result.get("rows_written").is_some())
            .then_some(StatementUsage {
                rows_read,
                rows_written,
            });
        let cols: Vec<String> = result
            .get("cols")
            .and_then(Json::as_array)
            .map(|cs| {
                cs.iter()
                    .filter_map(|c| c.get("name").and_then(Json::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let mut rows = Vec::new();
        if let Some(raw_rows) = result.get("rows").and_then(Json::as_array) {
            for raw in raw_rows {
                let mut pairs = Vec::new();
                if let Some(cells) = raw.as_array() {
                    for (i, cell) in cells.iter().enumerate() {
                        if let Some(name) = cols.get(i) {
                            pairs.push((name.clone(), hrana_val(cell)));
                        }
                    }
                }
                rows.push(Row::new(pairs));
            }
        }
        results.push(rows);
        usage.push(meter);
    }
    Ok((results, usage))
}

// ------------------------------------------------------------- token ----

fn home() -> PathBuf {
    // Home-resolution contract (issues #5640 / #5762): a missing or
    // empty HOME is an error naming HOME, never a substitute
    // directory - the old fallback to a literal "~" silently
    // resolved state/config/token paths relative to the CWD.
    match std::env::var_os("HOME") {
        Some(home) if !home.is_empty() => PathBuf::from(home),
        _ => panic!(
            "HOME is not set or is empty: atlas-core requires a non-empty HOME to resolve its state/config paths (issue #5640)"
        ),
    }
}

fn token_cache_path() -> PathBuf {
    home().join(".cache/atlas-search/turso-token.json")
}

/// Run a command to completion with a timeout (the Python's
/// subprocess timeouts, coarsely). Returns stdout on success.
fn run_cmd(prog: &str, args: &[&str], envs: &[(&str, &str)], timeout: Duration) -> Option<String> {
    let mut cmd = Command::new(prog);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().ok()?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let mut out = String::new();
                child.stdout.take()?.read_to_string(&mut out).ok()?;
                return Some(out);
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return None,
        }
    }
}

/// The Python's `bws_secret`: fetch one Bitwarden Secrets Manager
/// value by name (the bws CLI, access token from the environment or
/// the fleet's bws-env script). Errors are opaque — the caller
/// treats any failure as "not that source".
fn bws_secret(name: &str) -> Option<String> {
    let access = match std::env::var("BWS_ACCESS_TOKEN") {
        Ok(t) if !t.is_empty() => t,
        _ => {
            let script = "source ~/tools/bws-env.sh >/dev/null 2>&1; \
                          printf %s \"$BWS_ACCESS_TOKEN\"";
            run_cmd("bash", &["-c", script], &[], Duration::from_secs(30))?
        }
    };
    if access.is_empty() {
        return None;
    }
    let bws = home().join("tools/bin/bws");
    if !bws.exists() {
        return None;
    }
    let bws = bws.to_string_lossy().into_owned();
    let envs = [("BWS_ACCESS_TOKEN", access.as_str())];
    for _ in 0..3 {
        let attempt = (|| {
            let list = run_cmd(
                &bws,
                &["secret", "list", "-o", "json"],
                &envs,
                Duration::from_secs(60),
            )?;
            let items: Json = serde_json::from_str(&list).ok()?;
            let mut sid = None;
            for item in items.as_array()? {
                if item.get("key").and_then(Json::as_str) == Some(name) {
                    sid = item.get("id").and_then(Json::as_str).map(str::to_string);
                    break;
                }
            }
            let sid = sid?;
            let got = run_cmd(
                &bws,
                &["secret", "get", &sid, "-o", "json"],
                &envs,
                Duration::from_secs(60),
            )?;
            let val: Json = serde_json::from_str(&got).ok()?;
            let v = match &val {
                Json::Array(a) => a.first()?.get("value")?,
                other => other.get("value")?,
            };
            v.as_str().map(str::to_string)
        })();
        if attempt.is_some() {
            return attempt;
        }
    }
    None
}

/// The Python's `resolve_turso_token` order, returning (token,
/// source label). Failure carries the Python's composed die text.
pub fn resolve_turso_token() -> Result<(String, String)> {
    if let Ok(t) = std::env::var("ATLAS_TURSO_TOKEN")
        && !t.is_empty()
    {
        return Ok((t, "env ATLAS_TURSO_TOKEN".to_string()));
    }
    for name in BWS_TOKEN_NAMES {
        if let Some(t) = bws_secret(name) {
            return Ok((t, format!("bitwarden:{name}")));
        }
    }
    // Cached read-only token minted via the turso CLI (< 20 h old).
    if let Ok(text) = std::fs::read_to_string(token_cache_path())
        && let Ok(cache) = serde_json::from_str::<Map<String, Json>>(&text)
    {
        let fresh = cache
            .get("fetched_at")
            .and_then(Json::as_f64)
            .map(|ts| {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs_f64())
                    .unwrap_or(0.0);
                now - ts < 20.0 * 3600.0
            })
            .unwrap_or(false);
        if cache.get("db").and_then(Json::as_str) == Some(TURSO_DB)
            && fresh
            && let Some(t) = cache.get("token").and_then(Json::as_str)
        {
            return Ok((
                t.to_string(),
                "cache (turso CLI read-only mint)".to_string(),
            ));
        }
    }
    // Mint read-only via the turso CLI (its tokens carry claim a=ro).
    if let Some(out) = run_cmd(
        "turso",
        &[
            "db",
            "tokens",
            "create",
            TURSO_DB,
            "--read-only",
            "-e",
            "7d",
        ],
        &[],
        Duration::from_secs(60),
    ) {
        let token = out.trim().to_string();
        if token.matches('.').count() == 2 {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            let cache = json!({"db": TURSO_DB, "token": token, "fetched_at": now});
            if let Some(parent) = token_cache_path().parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .mode(0o600)
                    .open(token_cache_path())
                {
                    use std::io::Write;
                    let _ = f.write_all(cache.to_string().as_bytes());
                }
            }
            return Ok((token, "turso CLI read-only mint".to_string()));
        }
    }
    Err(Error::fatal(
        "turso backend: token did not resolve (no ATLAS_TURSO_TOKEN, no \
         Bitwarden atlas-ro secret, turso CLI mint unavailable). \
         Set ATLAS_TURSO_TOKEN or log the turso CLI in.",
    ))
}

// ----------------------------------------------------------- backend ----

pub struct TursoBackend<T: PipelineTransport = HttpTransport> {
    transport: T,
    vec_table: String,
    pub token_source: String,
    /// The effective classification-manifest surface (design §7):
    /// the loaded manifest (or the built-in fallback) and its
    /// provenance, in the same pattern as `LocalBackend`.
    pub manifest: Manifest,
    pub manifest_source: String,
    notes: Vec<String>,
}

impl TursoBackend<HttpTransport> {
    /// Connect with a token from the standard resolution chain.
    /// Manifest problems never fail construction (§7): the loader
    /// falls back to the built-in surface, loudly.
    pub fn new(vec_table: &str) -> Result<TursoBackend<HttpTransport>> {
        let (token, source) = resolve_turso_token()?;
        let transport = HttpTransport::new(TURSO_HOST, token);
        let loaded = load_manifest_via(&transport);
        let notes = manifest_notes(&loaded);
        Ok(TursoBackend {
            transport,
            vec_table: vec_table.to_string(),
            token_source: source,
            manifest_source: loaded.describe(),
            manifest: loaded.manifest,
            notes,
        })
    }

    /// Connect with an explicit token (tests, service config).
    /// Manifest problems never fail construction (§7).
    pub fn with_token(host: &str, token: String, vec_table: &str) -> TursoBackend<HttpTransport> {
        let transport = HttpTransport::new(host, token);
        let loaded = load_manifest_via(&transport);
        let notes = manifest_notes(&loaded);
        TursoBackend {
            transport,
            vec_table: vec_table.to_string(),
            token_source: "explicit".to_string(),
            manifest_source: loaded.describe(),
            manifest: loaded.manifest,
            notes,
        }
    }
}

/// The load-time notes a manifest outcome produces: §7's loud
/// fallback warning, when the surface fell back to the built-in
/// manifest.
fn manifest_notes(loaded: &LoadedManifest) -> Vec<String> {
    loaded
        .warning
        .as_ref()
        .map(|w| {
            vec![format!(
                "WARNING: classification manifest rejected ({w}) — built-in filter surface applies"
            )]
        })
        .unwrap_or_default()
}

/// Load the classification manifest over the pipeline (design
/// §§3/7): a presence check against `sqlite_master`, then the
/// stored-shape row SELECT, parsed by the transport-free
/// `parse_manifest_rows`. This never fails: a transport or
/// presence failure means the built-in absent surface; an
/// unreadable or invalid table means the built-in surface with a
/// loud warning (§7: fall back, loudly, never widen).
fn load_manifest_via<T: PipelineTransport>(transport: &T) -> LoadedManifest {
    let absent = || LoadedManifest {
        manifest: builtin_manifest(),
        source: crate::manifest::ManifestSource::BuiltinAbsent,
        warning: None,
    };
    let fallback = |warning: String| LoadedManifest {
        manifest: builtin_manifest(),
        source: crate::manifest::ManifestSource::BuiltinFallback,
        warning: Some(warning),
    };
    let presence = Stmt::new(
        "SELECT COUNT(*) AS n FROM sqlite_master WHERE type='table' \
         AND name='classification_manifest'",
        vec![],
    );
    let results = match transport.pipeline(std::slice::from_ref(&presence)) {
        Ok(r) => r,
        Err(_) => return absent(),
    };
    let present = results
        .into_iter()
        .next()
        .and_then(|rows| rows.into_iter().next())
        .and_then(|row| {
            row.get_i64("n").or_else(|| match row.first_value() {
                Some(Value::Integer(i)) => Some(*i),
                Some(Value::Float(f)) => Some(*f as i64),
                _ => None,
            })
        })
        .unwrap_or(0)
        > 0;
    if !present {
        return absent();
    }
    let rows_stmt = Stmt::new(
        "SELECT manifest_version, content_hash, name, shape, semantics, label_column \
         FROM classification_manifest ORDER BY rowid",
        vec![],
    );
    let rows = match transport.pipeline(std::slice::from_ref(&rows_stmt)) {
        Ok(r) => r.into_iter().next().unwrap_or_default(),
        Err(e) => return fallback(format!("unreadable: {e}")),
    };
    let mut raw: Vec<crate::manifest::ManifestRow> = Vec::with_capacity(rows.len());
    for row in &rows {
        let Some(version) = row.get_i64("manifest_version") else {
            return fallback(
                "unreadable: missing manifest_version in classification_manifest row".to_string(),
            );
        };
        let Some(hash) = row.get_text("content_hash") else {
            return fallback(
                "unreadable: missing content_hash in classification_manifest row".to_string(),
            );
        };
        raw.push((
            version,
            hash.to_string(),
            row.get_text("name").map(str::to_string),
            row.get_text("shape").map(str::to_string),
            row.get_text("semantics").map(str::to_string),
            row.get_text("label_column").map(str::to_string),
        ));
    }
    match parse_manifest_rows(raw) {
        Ok(manifest) => LoadedManifest {
            manifest,
            source: crate::manifest::ManifestSource::Table {
                schema: "turso".to_string(),
            },
            warning: None,
        },
        Err(e) => fallback(e.to_string()),
    }
}

impl<T: PipelineTransport> TursoBackend<T> {
    /// The transport this backend speaks over (test/diagnostic
    /// access; the pipeline contract is the hrana HTTP shape).
    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub fn with_transport(transport: T, vec_table: &str) -> TursoBackend<T> {
        let loaded = load_manifest_via(&transport);
        let notes = manifest_notes(&loaded);
        TursoBackend {
            transport,
            vec_table: vec_table.to_string(),
            token_source: "injected transport".to_string(),
            manifest_source: loaded.describe(),
            manifest: loaded.manifest,
            notes,
        }
    }

    fn execute(&self, stmt: &Stmt) -> Result<Vec<Row>> {
        Ok(self
            .transport
            .pipeline(std::slice::from_ref(stmt))?
            .into_iter()
            .next()
            .unwrap_or_default())
    }

    /// The Python's `turso_allowed_ids`: ceiling set intersected
    /// with the session-filter set when any session filter applies.
    pub fn allowed_ids(&self, filters: &Filters) -> Result<HashSet<i64>> {
        let rows = self.execute(&turso_allowed_sens_stmt(filters))?;
        let mut allowed: HashSet<i64> = rows.iter().filter_map(|r| r.get_i64("cid")).collect();
        if let Some(stmt) = turso_allowed_session_stmt(filters) {
            let rows = self.execute(&stmt)?;
            let sess: HashSet<i64> = rows.iter().filter_map(|r| r.get_i64("cid")).collect();
            allowed = allowed.intersection(&sess).copied().collect();
        }
        Ok(allowed)
    }

    /// The Python's `turso_index_branch`.
    pub fn index_branch(
        &self,
        filters: &Filters,
        qvec_json: &str,
        pool: usize,
        vb: VectorBackend,
    ) -> Result<Vec<BranchHit>> {
        let allowed = self.allowed_ids(filters)?;
        let mut survivors: Vec<i64> = Vec::new();
        for depth in index_depths(pool, vb) {
            let rows = self.execute(&vector_top_k_stmt(&self.vec_table, qvec_json, depth))?;
            survivors = rows
                .iter()
                .filter_map(|r| r.get_i64("id"))
                .filter(|id| allowed.contains(id))
                .take(pool)
                .collect();
            if survivors.len() >= pool {
                break;
            }
        }
        if survivors.is_empty() {
            return Ok(Vec::new());
        }
        let rows = self.execute(&survivor_dist_stmt(&self.vec_table, qvec_json, &survivors))?;
        let dist: HashMap<i64, f64> = rows
            .iter()
            .filter_map(|r| Some((r.get_i64("cid")?, r.get_f64("dist")?)))
            .collect();
        Ok(survivors
            .iter()
            .map(|cid| BranchHit {
                chunk_id: *cid,
                dist: dist.get(cid).copied(),
            })
            .collect())
    }
}

/// Retrieval depths for the index modes (the Python's loop):
/// fixed = `[pool]`; fill = pool doubling while < 3200, then 3200.
pub fn index_depths(pool: usize, vb: VectorBackend) -> Vec<usize> {
    match vb {
        VectorBackend::IndexFixed => vec![pool],
        VectorBackend::IndexFill => {
            if pool == 0 {
                // The Python loops forever here (0 doubles to 0);
                // a zero pool is a degenerate request, not a hang
                // this port reproduces.
                return vec![0];
            }
            let mut depths = Vec::new();
            let mut d = pool;
            while d < 3200 {
                depths.push(d);
                d *= 2;
            }
            depths.push(3200);
            depths
        }
        VectorBackend::Scan => Vec::new(),
    }
}

impl<T: PipelineTransport> SearchBackend for TursoBackend<T> {
    fn name(&self) -> &'static str {
        "turso"
    }

    fn vec_table(&self) -> &str {
        &self.vec_table
    }

    fn notes(&self) -> Vec<String> {
        self.notes.clone()
    }

    fn branch_pools(&self, q: &BranchQuery, filters: &Filters) -> Result<BranchPools> {
        let qvec_json = q.query_vector.as_ref().map(|v| qvec_json(v));
        let mut pools = BranchPools::default();
        match qvec_json.as_deref() {
            // Index strategies: FTS rides its own pipeline call,
            // then the index branch (the Python's call pattern).
            Some(qj) if q.vector_backend != VectorBackend::Scan => {
                if let Some(me) = &q.match_expr {
                    let stmt = fts_stmt(TURSO_SHAPE, filters, me, q.pool);
                    let results = self.transport.pipeline(std::slice::from_ref(&stmt))?;
                    if let Some(rows) = results.into_iter().next() {
                        pools.fts = rows
                            .iter()
                            .filter_map(|r| {
                                Some(BranchHit {
                                    chunk_id: r.get_i64("cid")?,
                                    dist: None,
                                })
                            })
                            .collect();
                    }
                }
                pools.vector = self.index_branch(filters, qj, q.pool, q.vector_backend)?;
            }
            // Scan (and FTS-only): both branch statements in ONE
            // pipeline call, results consumed in statement order.
            _ => {
                let stmts = branch_queries(
                    TURSO_SHAPE,
                    &self.vec_table,
                    filters,
                    q.match_expr.as_deref(),
                    qvec_json.as_deref(),
                    q.pool,
                );
                let results = self.transport.pipeline(&stmts)?;
                let mut i = 0;
                if q.match_expr.is_some() {
                    if let Some(rows) = results.get(i) {
                        pools.fts = rows
                            .iter()
                            .filter_map(|r| {
                                Some(BranchHit {
                                    chunk_id: r.get_i64("cid")?,
                                    dist: None,
                                })
                            })
                            .collect();
                    }
                    i += 1;
                }
                if qvec_json.is_some()
                    && let Some(rows) = results.get(i)
                {
                    pools.vector = rows
                        .iter()
                        .filter_map(|r| {
                            Some(BranchHit {
                                chunk_id: r.get_i64("cid")?,
                                dist: r.get_f64("dist"),
                            })
                        })
                        .collect();
                }
            }
        }
        Ok(pools)
    }

    fn chunk_details(&self, ids: &[i64]) -> Result<Vec<ChunkDetail>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let sql = crate::filters::details_sql(TURSO_SHAPE, ids.len());
        let args: Vec<Value> = ids.iter().map(|i| Value::Integer(*i)).collect();
        let rows = self.execute(&Stmt::new(sql, args))?;
        Ok(rows.iter().filter_map(detail_from_row).collect())
    }

    fn neighbors(&self, message_id: &str, chunk_idx: i64, n: i64) -> Result<Vec<NeighborRow>> {
        let rows = self.execute(&Stmt::new(
            crate::filters::NEIGHBORS_SQL,
            vec![
                Value::text(message_id),
                Value::Integer(chunk_idx - n),
                Value::Integer(chunk_idx + n),
            ],
        ))?;
        Ok(rows
            .iter()
            .filter_map(|r| {
                Some(NeighborRow {
                    chunk_id: r.get_i64("id")?,
                    chunk_idx: r.get_i64("chunk_idx")?,
                    text: r.get_text("text")?.to_string(),
                })
            })
            .collect())
    }

    fn message(&self, message_id: &str) -> Result<Option<MessageRow>> {
        let rows = self.execute(&Stmt::new(
            crate::filters::MESSAGE_SQL,
            vec![Value::text(message_id)],
        ))?;
        Ok(rows.first().and_then(|r| {
            Some(MessageRow {
                id: r.get_text("id")?.to_string(),
                role: r.get_text("role").map(str::to_string),
                text: r.get_text("text").unwrap_or("").to_string(),
            })
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depths_fixed_and_fill() {
        assert_eq!(index_depths(100, VectorBackend::IndexFixed), vec![100]);
        assert_eq!(
            index_depths(100, VectorBackend::IndexFill),
            vec![100, 200, 400, 800, 1600, 3200]
        );
        assert_eq!(
            index_depths(3, VectorBackend::IndexFill),
            vec![3, 6, 12, 24, 48, 96, 192, 384, 768, 1536, 3072, 3200]
        );
        assert_eq!(index_depths(3200, VectorBackend::IndexFill), vec![3200]);
    }

    #[test]
    fn hrana_roundtrip_shapes() {
        assert_eq!(
            hrana_arg(&Value::Integer(7)),
            json!({"type": "integer", "value": "7"})
        );
        assert_eq!(hrana_arg(&Value::Null), json!({"type": "null"}));
        assert_eq!(
            hrana_val(&json!({"type": "integer", "value": "42"})),
            Value::Integer(42)
        );
        assert_eq!(
            hrana_val(&json!({"type": "text", "value": "x"})),
            Value::text("x")
        );
        assert_eq!(hrana_val(&Json::Null), Value::Null);
    }

    #[test]
    fn pipeline_response_captures_optional_per_statement_usage() {
        let response = json!({
            "results": [
                {"type":"ok","response":{"type":"execute","result":{
                    "cols":[{"name":"n"}],
                    "rows":[[{"type":"integer","value":"1"}]],
                    "rows_read":17,
                    "rows_written":3
                }}},
                {"type":"ok","response":{"type":"execute","result":null}},
                {"type":"ok","response":{"type":"execute","result":{
                    "cols":[],"rows":[],"rows_read":"unknown"
                }}}
            ]
        });

        let (rows, usage) = parse_pipeline_response(&response).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0].get("n"), Some(&Value::Integer(1)));
        assert_eq!(usage.len(), 2);
        assert_eq!(
            usage[0],
            Some(StatementUsage {
                rows_read: Some(17),
                rows_written: Some(3)
            })
        );
        assert_eq!(
            usage[1],
            Some(StatementUsage {
                rows_read: None,
                rows_written: None
            })
        );
    }

    #[test]
    fn pipeline_transport_default_reports_unknown_usage_not_zero() {
        let transport = ManifestStub {
            present: false,
            rows: Vec::new(),
            fail_rows: false,
        };
        let (rows, usage) = transport
            .pipeline_metered(&[Stmt::new("SELECT count(*) FROM sqlite_master", vec![])])
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(usage, vec![None]);
    }

    #[test]
    fn pipeline_usage_error_result_stays_a_query_error() {
        let response = json!({
            "results": [{
                "type": "error",
                "error": {"message": "fixture SQL failure"}
            }]
        });
        assert!(parse_pipeline_response(&response).is_err());
    }

    /// Stub pipeline serving a canned manifest surface: presence
    /// count plus manifest rows (or a row-fetch error). Anything
    /// else is a test bug.
    struct ManifestStub {
        present: bool,
        rows: Vec<Row>,
        fail_rows: bool,
    }

    impl PipelineTransport for ManifestStub {
        fn pipeline(&self, stmts: &[Stmt]) -> Result<Vec<Vec<Row>>> {
            let mut out = Vec::new();
            for s in stmts {
                if s.sql.contains("sqlite_master") {
                    out.push(vec![Row::new(vec![(
                        "n".to_string(),
                        Value::Integer(i64::from(self.present)),
                    )])]);
                } else if s.sql.contains("classification_manifest") {
                    if self.fail_rows {
                        return Err(Error::backend("stub: manifest rows unreadable"));
                    }
                    out.push(self.rows.clone());
                } else {
                    panic!("unexpected SQL in manifest stub: {}", s.sql);
                }
            }
            Ok(out)
        }
    }

    fn manifest_rows(m: &crate::manifest::Manifest) -> Vec<Row> {
        let mut rows = vec![Row::new(vec![
            ("manifest_version".to_string(), Value::Integer(m.version)),
            (
                "content_hash".to_string(),
                Value::text(m.content_hash.clone()),
            ),
            ("name".to_string(), Value::Null),
            ("shape".to_string(), Value::Null),
            ("semantics".to_string(), Value::Null),
            ("label_column".to_string(), Value::Null),
        ])];
        for d in &m.dimensions {
            rows.push(Row::new(vec![
                ("manifest_version".to_string(), Value::Integer(m.version)),
                (
                    "content_hash".to_string(),
                    Value::text(m.content_hash.clone()),
                ),
                ("name".to_string(), Value::text(d.name.clone())),
                ("shape".to_string(), Value::text(d.shape.name().to_string())),
                (
                    "semantics".to_string(),
                    Value::text(d.semantics.name().to_string()),
                ),
                (
                    "label_column".to_string(),
                    d.label_column
                        .clone()
                        .map(Value::text)
                        .unwrap_or(Value::Null),
                ),
            ]));
        }
        rows
    }

    #[test]
    fn turso_manifest_absent_is_builtin_without_warning() {
        use crate::backend::SearchBackend;
        let be = TursoBackend::with_transport(
            ManifestStub {
                present: false,
                rows: Vec::new(),
                fail_rows: false,
            },
            "vec_snowflake_m_v2_768_f32",
        );
        assert_eq!(be.manifest, builtin_manifest());
        assert!(
            be.manifest_source.contains("no classification_manifest"),
            "source: {}",
            be.manifest_source
        );
        assert!(be.notes().is_empty());
    }

    #[test]
    fn turso_manifest_valid_table_loads() {
        use crate::backend::SearchBackend;
        let stored = builtin_manifest();
        let rows = manifest_rows(&stored);
        let be = TursoBackend::with_transport(
            ManifestStub {
                present: true,
                rows,
                fail_rows: false,
            },
            "vec_snowflake_m_v2_768_f32",
        );
        assert_eq!(be.manifest, stored);
        assert!(
            be.manifest_source
                .contains("classification_manifest (turso)"),
            "source: {}",
            be.manifest_source
        );
        assert!(be.notes().is_empty());
    }

    #[test]
    fn turso_manifest_rejected_falls_back_loudly() {
        use crate::backend::SearchBackend;
        // Present but invalid: unknown shape fails §6 validation.
        let rows = vec![
            Row::new(vec![
                ("manifest_version".to_string(), Value::Integer(1)),
                ("content_hash".to_string(), Value::text("x")),
                ("name".to_string(), Value::Null),
                ("shape".to_string(), Value::Null),
                ("semantics".to_string(), Value::Null),
                ("label_column".to_string(), Value::Null),
            ]),
            Row::new(vec![
                ("manifest_version".to_string(), Value::Integer(1)),
                ("content_hash".to_string(), Value::text("x")),
                ("name".to_string(), Value::text("kind")),
                ("shape".to_string(), Value::text("raw_sql")),
                ("semantics".to_string(), Value::text("exact")),
                ("label_column".to_string(), Value::text("kind")),
            ]),
        ];
        let be = TursoBackend::with_transport(
            ManifestStub {
                present: true,
                rows,
                fail_rows: false,
            },
            "vec_snowflake_m_v2_768_f32",
        );
        assert_eq!(be.manifest, builtin_manifest());
        assert!(
            be.manifest_source.contains("rejected"),
            "source: {}",
            be.manifest_source
        );
        let notes = be.notes();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("WARNING: classification manifest rejected"));

        // Present but unreadable: the row fetch fails.
        let be = TursoBackend::with_transport(
            ManifestStub {
                present: true,
                rows: Vec::new(),
                fail_rows: true,
            },
            "vec_snowflake_m_v2_768_f32",
        );
        assert_eq!(be.manifest, builtin_manifest());
        assert!(be.manifest_source.contains("rejected"));
        assert_eq!(be.notes().len(), 1);
    }
}
