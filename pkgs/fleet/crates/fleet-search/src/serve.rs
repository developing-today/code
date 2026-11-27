//! `fleet serve search` — the Atlas service role (fleet step 5
//! residual, developing-today/code#5604; design note §5/§7).
//!
//! The serve half of the atlas-search bin crate, rehomed. The
//! service engine, job manager and HTTP surface are [`crate::server`]
//! (a verbatim copy of atlas-search's `server.rs`, held byte-for-byte
//! by `.github/scripts/check-fleet-search-parity.sh`); this module is
//! the glue that crate's `main.rs` carried: the role's argument
//! surface, the lazy shared embedder, and the wiring of backend,
//! embedder and engine.
//!
//! What the role guarantees, all inherited from atlas-core and
//! pinned by tests here and in `server`:
//!
//!   * **One shared embedder per process.** A single lazily loaded
//!     `AtlasEmbedder` (one ONNX session, mutex-held) serves every
//!     client; its load state is what `/health` reports.
//!   * **Mechanically read-only.** The backend is
//!     `TursoBackend::new` — the token comes only from the
//!     read-only resolution chain (env, Bitwarden `atlas-ro`, the
//!     turso CLI's `--read-only` mint) and atlas-core issues nothing
//!     but SELECTs. There is no write path to opt into, and no local
//!     or explicit-token backend in this role.
//!   * **The job protocol**: bounded async queue, in-memory temp
//!     store, per-client recovery listing, dedupe keyed by the
//!     effective request (never across effective ceilings).
//!
//! The role keeps the standalone service's exact wire and config
//! (`~/.local/state/atlas-search/service.json`, loopback :18760), so
//! re-pointing the atlas unit between `fleet serve search` and the
//! standalone `atlas-search serve` is a one-line ExecStart change.
//! `init` (client-table generation) and `fetch-model` stay on the
//! standalone artifact.
//!
//! Exit codes mirror atlas-search: 2 usage/config, 1 runtime failure
//! (backend token, bind). A running service never exits 0.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use atlas_core::{AtlasEmbedder, Error as CoreError, FixedEmbedder, QueryEmbedder, TursoBackend};

use crate::server::{
    self, CoreSearchEngine, ServeOverrides, ServeSettings, SharedEmbedder,
    resolve_service_config_path,
};

const EXIT_FAILURE: i32 = 1;
const EXIT_USAGE: i32 = 2;

/// The pinned Snowflake snapshot revision. Must equal
/// `fetch_model::MODEL_REV` in atlas-search (a test pins that).
pub const MODEL_REV: &str = "95c2741480856aa9666782eb4afe11959938017f";

pub const SERVE_USAGE: &str = "\
usage: fleet serve search [options]

Run the Atlas search service in the foreground: the loopback HTTP job
protocol over one shared Snowflake embedder and a read-only Turso
backend. The atlas-search service, rehomed (#5604); same config, same
wire.

Options:
  --config PATH         service config (default
                        ~/.local/state/atlas-search/service.json;
                        env ATLAS_SEARCH_SERVICE_CONFIG)
  --listen ADDR         listen address override (loopback only)
  --job-ttl-ms N        completed-job TTL override, milliseconds
                        (default 600000)
  --hard-ttl-ms N       submit hard-cap override, milliseconds
                        (default 3600000)
  -h, --help            this help

`init` (client table) and `fetch-model` stay on the standalone
atlas-search artifact. Rollback: point the unit's ExecStart back at
`atlas-search serve`.";

/// Parsed role arguments.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ServeArgs {
    pub config: Option<PathBuf>,
    pub listen: Option<SocketAddr>,
    pub job_ttl_ms: Option<u64>,
    pub hard_ttl_ms: Option<u64>,
}

/// What the arguments ask for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    Help,
    Run(ServeArgs),
}

/// Parse `--flag value` / `--flag=value` (clap's two spellings, which
/// the standalone `atlas-search serve` accepts). Unknown flags and
/// stray positionals are usage errors naming the offender.
pub fn parse_args(args: &[String]) -> Result<Parsed, String> {
    let mut out = ServeArgs::default();
    let mut i = 0;
    while i < args.len() {
        let raw = args[i].as_str();
        i += 1;
        if matches!(raw, "-h" | "--help" | "help") {
            return Ok(Parsed::Help);
        }
        let (flag, inline) = match raw.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
            _ => (raw, None),
        };
        if !matches!(
            flag,
            "--config" | "--listen" | "--job-ttl-ms" | "--hard-ttl-ms"
        ) {
            return Err(format!("unexpected argument '{raw}'"));
        }
        let value = match inline {
            Some(v) => v,
            None => {
                let v = args
                    .get(i)
                    .ok_or_else(|| format!("{flag} needs a value"))?
                    .clone();
                i += 1;
                v
            }
        };
        match flag {
            "--config" => out.config = Some(PathBuf::from(value)),
            "--listen" => {
                out.listen = Some(
                    value
                        .parse()
                        .map_err(|e| format!("--listen '{value}' is not a socket address: {e}"))?,
                )
            }
            "--job-ttl-ms" => {
                out.job_ttl_ms = Some(
                    value
                        .parse()
                        .map_err(|e| format!("--job-ttl-ms '{value}' is not a number: {e}"))?,
                )
            }
            _ => {
                out.hard_ttl_ms = Some(
                    value
                        .parse()
                        .map_err(|e| format!("--hard-ttl-ms '{value}' is not a number: {e}"))?,
                )
            }
        }
    }
    Ok(Parsed::Run(out))
}

// ------------------------------------------------------------ embedder --

fn home_dir() -> PathBuf {
    // Home-resolution contract (issues #5640 / #5762): a missing or
    // empty HOME is an error naming HOME, never a substitute
    // directory - the old fallback to a literal "~" silently
    // resolved state/config/token paths relative to the CWD.
    match std::env::var_os("HOME") {
        Some(home) if !home.is_empty() => PathBuf::from(home),
        _ => panic!(
            "HOME is not set or is empty: fleet-search requires a non-empty HOME to resolve its state/config paths (issue #5640)"
        ),
    }
}

fn resolve_model_dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("ATLAS_MODEL_DIR")
        && !d.is_empty()
    {
        let p = PathBuf::from(&d);
        return if p.is_dir() { Some(p) } else { None };
    }
    let home = home_dir();
    let state = home.join(format!(".local/state/atlas-search/model/{MODEL_REV}"));
    if state.is_dir() {
        return Some(state);
    }
    let hf = home.join(format!(
        ".cache/huggingface/hub/models--Snowflake--snowflake-arctic-embed-m-v2.0/snapshots/{MODEL_REV}"
    ));
    if hf.is_dir() {
        return Some(hf);
    }
    None
}

fn resolve_ort_lib() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ATLAS_ORT_LIB")
        && !p.is_empty()
    {
        let pb = PathBuf::from(&p);
        return if pb.is_file() { Some(pb) } else { None };
    }
    let cand = home_dir().join(".local/state/atlas-search/ort/libonnxruntime.so");
    if cand.is_file() {
        return Some(cand);
    }
    // The packaged binary's closure path, baked in at build time
    // (pkgs/fleet/default.nix sets ATLAS_SEARCH_ORT_LIB_DEFAULT to
    // its onnxruntime store path, the atlas-search package's own
    // mechanism). Dev builds do not set it and never reach this.
    if let Some(p) = option_env!("ATLAS_SEARCH_ORT_LIB_DEFAULT") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    None
}

enum LoadedEmbedder {
    Real(Box<AtlasEmbedder>),
    Fixed(FixedEmbedder),
}

/// The role's lazy query embedder: the ONNX session loads on the
/// first vector query only, so FTS-only jobs and a cold service never
/// pay for (or fail on) a missing model. One instance is shared by
/// every client through [`SharedEmbedder`].
///
/// Test hook, as in atlas-search: `ATLAS_SEARCH_QVEC_FILE` (a JSON
/// array of floats) substitutes a fixed query vector.
pub struct LazyEmbedder {
    cell: OnceLock<Result<LoadedEmbedder, String>>,
}

impl Default for LazyEmbedder {
    fn default() -> Self {
        LazyEmbedder::new()
    }
}

impl LazyEmbedder {
    pub fn new() -> LazyEmbedder {
        LazyEmbedder {
            cell: OnceLock::new(),
        }
    }

    fn load(&self) -> Result<&LoadedEmbedder, CoreError> {
        self.cell
            .get_or_init(|| {
                if let Ok(f) = std::env::var("ATLAS_SEARCH_QVEC_FILE")
                    && !f.is_empty()
                {
                    let text = std::fs::read_to_string(&f)
                        .map_err(|e| format!("ATLAS_SEARCH_QVEC_FILE: cannot read {f}: {e}"))?;
                    let raw: Vec<f32> = serde_json::from_str::<Vec<f64>>(&text)
                        .map(|v| v.into_iter().map(|x| x as f32).collect())
                        .map_err(|e| {
                            format!("ATLAS_SEARCH_QVEC_FILE: bad JSON vector in {f}: {e}")
                        })?;
                    return Ok(LoadedEmbedder::Fixed(FixedEmbedder { raw }));
                }
                load_embedder().map(|e| LoadedEmbedder::Real(Box::new(e)))
            })
            .as_ref()
            .map_err(|m| CoreError::fatal(m.clone()))
    }
}

impl QueryEmbedder for LazyEmbedder {
    fn embed_query(&self, text: &str, trunc: usize) -> Result<Vec<f32>, CoreError> {
        match self.load()? {
            LoadedEmbedder::Real(e) => e.embed_query(text, trunc),
            LoadedEmbedder::Fixed(e) => e.embed_query(text, trunc),
        }
    }
}

fn load_embedder() -> Result<AtlasEmbedder, String> {
    let model_dir = resolve_model_dir().ok_or_else(|| {
        "Snowflake model not found (looked in ATLAS_MODEL_DIR, \
         ~/.local/state/atlas-search/model/<rev>, and the HF cache snapshot); \
         FTS-only jobs work without it"
            .to_string()
    })?;
    let ort_lib = resolve_ort_lib().ok_or_else(|| {
        "ONNX Runtime library not found: set ATLAS_ORT_LIB (the packaged \
         binary resolves it from its nix closure); FTS-only jobs work without it"
            .to_string()
    })?;
    let emb = atlas_embed::Embedder::load(&model_dir, &ort_lib)
        .map_err(|e| format!("embedder load failed: {e}"))?;
    Ok(AtlasEmbedder::new(emb))
}

// ---------------------------------------------------------------- run --

/// Wire the engine over a backend: the one shared embedder, its
/// observable state, the core engine. The backend is generic so the
/// contract can be exercised against a stub; the role itself passes
/// a read-only [`TursoBackend`] only (see [`run_with_args`]).
pub fn build_engine<B>(
    backend: B,
    embedder: Arc<dyn QueryEmbedder + Send + Sync>,
) -> Arc<dyn server::SearchEngine>
where
    B: atlas_core::SearchBackend + Send + Sync + 'static,
{
    let shared = Arc::new(SharedEmbedder::new(embedder));
    let state_fn = shared.state_fn();
    Arc::new(CoreSearchEngine::new(Arc::new(backend), shared, state_fn))
}

/// Load settings, open the read-only backend, run the service in the
/// foreground. Returns the process exit code (never 0 on success:
/// `serve_forever` does not return).
pub fn run_with_args(args: ServeArgs) -> i32 {
    let path = resolve_service_config_path(args.config.as_deref());
    let settings = match server::load_service_settings(
        &path,
        ServeOverrides {
            listen: args.listen,
            job_ttl_ms: args.job_ttl_ms,
            hard_ttl_ms: args.hard_ttl_ms,
        },
    ) {
        Ok(s) => s,
        Err(m) => return fail(EXIT_USAGE, &m),
    };
    run_settings(settings)
}

/// The shard discovery ceiling for serve mode (#6666 slice 1):
/// the highest client ceiling cap in the service config.
/// Discovery opens at the service maximum; per-caller ceilings
/// only narrow the set at query time (`ShardSet::select`).
fn serve_discovery_ceiling(clients: &[server::ClientConfig]) -> atlas_core::shards::TierCeiling {
    fn height(t: atlas_core::shards::Tier) -> u8 {
        match t {
            atlas_core::shards::Tier::Public => 0,
            atlas_core::shards::Tier::Internal => 1,
            atlas_core::shards::Tier::Secret => 2,
        }
    }
    let mut ceiling =
        atlas_core::shards::TierCeiling::from_sensitivity(atlas_core::Sensitivity::Shareable);
    for c in clients {
        let cap = atlas_core::shards::TierCeiling::from_sensitivity(
            atlas_core::Sensitivity::from_name(c.ceiling_cap.name())
                .unwrap_or(atlas_core::Sensitivity::Personal),
        );
        if height(cap.0) > height(ceiling.0) {
            ceiling = cap;
        }
    }
    ceiling
}

fn run_settings(settings: ServeSettings) -> i32 {
    // The role's backend base: the read-only Turso chain. This is
    // the whole write-posture story — see the module docs.
    let backend = match TursoBackend::new(&settings.vec_table) {
        Ok(b) => b,
        Err(e) => return fail(EXIT_FAILURE, &e.to_string()),
    };
    // Serve-mode shard fan-out (#6666 slice 1): with a `shards`
    // spec in the service config, the vector branch is served by
    // the shard set, re-discovered on the configured interval;
    // without one the monolith serves exactly as before.
    if let Some(spec) = &settings.shards {
        let ceiling = serve_discovery_ceiling(&settings.clients);
        let set = match atlas_core::shards::open_from_spec(spec, ceiling, &settings.vec_table) {
            Ok(s) => s,
            Err(e) => return fail(EXIT_FAILURE, &e.to_string()),
        };
        let mut backend = atlas_core::shards::ShardedBackend::new(backend, set).with_refresh(
            spec.clone(),
            ceiling,
            settings.shard_refresh,
        );
        // #3125 slice 7: attach the catalog/source-registry so served
        // get-chunk occurrence rows carry `root_id` for registered
        // locators; mirrors atlas-search `cmd_serve`.
        if let Some(path) = &settings.source_registry {
            match atlas_core::source_registry::open_source_registry(path) {
                Ok(conn) => backend = backend.with_source_registry(conn),
                Err(e) => return fail(EXIT_FAILURE, &e.to_string()),
            }
        }
        let engine = build_engine(backend, Arc::new(LazyEmbedder::new()));
        return match server::serve_forever(settings, engine) {
            Ok(()) => EXIT_FAILURE,
            Err(m) => fail(EXIT_FAILURE, &m),
        };
    }
    let engine = build_engine(backend, Arc::new(LazyEmbedder::new()));
    match server::serve_forever(settings, engine) {
        Ok(()) => EXIT_FAILURE,
        Err(m) => fail(EXIT_FAILURE, &m),
    }
}

fn fail(code: i32, msg: &str) -> i32 {
    eprintln!("fleet serve search: {msg}");
    code
}

/// `fleet serve search ...` entry point (args after the role name).
pub fn run(args: &[String]) -> i32 {
    match parse_args(args) {
        Ok(Parsed::Help) => {
            println!("{SERVE_USAGE}");
            0
        }
        Ok(Parsed::Run(a)) => run_with_args(a),
        Err(m) => {
            eprintln!("fleet serve search: {m}\n\n{SERVE_USAGE}");
            EXIT_USAGE
        }
    }
}

/// Used by tests and tooling: the default config path the role reads.
pub fn default_config_path() -> PathBuf {
    server::default_service_config_path()
}

#[cfg(test)]
mod parity_regression_tests {
    // #15653: Fleet mirrors of the Atlas service behaviors that drifted.
    // Oracles: Atlas `server.rs` (load_service_settings /
    // resolve_service_config_path / SearchEngine::chunk contract) and
    // atlas-core `run_get_chunk_with_context` (occurrences bounded by
    // grant ∩ context).
    use super::*;
    use atlas_core::shards::{Tier, TierCeiling};
    use atlas_core::types::{BranchPools, BranchQuery, ChunkDetail, MessageRow, NeighborRow};
    use atlas_core::{ChunkOccurrence, DisclosureContext, SearchBackend};
    use atlas_proto::Sensitivity as ProtoSensitivity;

    use std::os::unix::fs::PermissionsExt;

    fn write_config(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        path
    }

    fn service_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fleet-serve-parity-15653-{}-{}",
            tag,
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ceiling_alias_private_is_accepted_like_atlas() {
        // Oracle: Atlas `Sensitivity::from_ceiling_name`: legacy
        // "public/internal/private/sensitive" map onto the four rungs;
        // Fleet's service config must accept them too (old code used
        // strict `from_name` and rejected all four).
        let dir = service_dir("alias");
        let path = write_config(
            &dir,
            "alias.json",
            r#"{"clients": [{"name": "probe", "token": "tok", "ceiling_cap": "private"}]}"#,
        );
        let settings =
            server::load_service_settings(&path, server::ServeOverrides::default()).unwrap();
        assert_eq!(settings.clients.len(), 1);
        assert_eq!(
            settings.clients[0].ceiling_cap,
            ProtoSensitivity::LocalOnly,
            "private must map to local-only exactly like Atlas"
        );
        let path = write_config(
            &dir,
            "alias-internal.json",
            r#"{"clients": [{"name": "probe", "token": "tok", "ceiling_cap": "internal"}]}"#,
        );
        let settings =
            server::load_service_settings(&path, server::ServeOverrides::default()).unwrap();
        assert_eq!(settings.clients[0].ceiling_cap, ProtoSensitivity::Personal);
    }

    #[test]
    fn empty_config_flag_counts_as_absent_like_atlas() {
        // Oracle: Atlas `resolve_service_config_path` filters an empty
        // flag value (same reasoning as `service::base_url`).
        let old = std::env::var("ATLAS_SEARCH_SERVICE_CONFIG").ok();
        unsafe {
            std::env::remove_var("ATLAS_SEARCH_SERVICE_CONFIG");
        }
        let got = server::resolve_service_config_path(Some(std::path::Path::new("")));
        assert_eq!(got, server::default_service_config_path());
        if let Some(v) = old {
            unsafe {
                std::env::set_var("ATLAS_SEARCH_SERVICE_CONFIG", v);
            }
        }
    }

    #[test]
    fn source_registry_config_is_validated_like_atlas() {
        // Oracle: Atlas `load_service_settings` validation strings for
        // `source_registry` (requires shards; path must not be empty).
        let dir = service_dir("registry");
        let client = r#"{"name": "probe", "token": "tok", "ceiling_cap": "personal"}"#;
        let wired = write_config(
            &dir,
            "wired.json",
            &format!(
                r#"{{"shards": "dir:/tmp/shards", "source_registry": "/tmp/catalog.db", "clients": [{client}]}}"#
            ),
        );
        let settings =
            server::load_service_settings(&wired, server::ServeOverrides::default()).unwrap();
        assert_eq!(
            settings.source_registry.as_deref(),
            Some(std::path::Path::new("/tmp/catalog.db"))
        );
        let orphan = write_config(
            &dir,
            "orphan.json",
            &format!(r#"{{"source_registry": "/tmp/catalog.db", "clients": [{client}]}}"#),
        );
        let err = server::load_service_settings(&orphan, server::ServeOverrides::default())
            .unwrap_err();
        assert!(
            err.contains("source_registry requires shards"),
            "got: {err}"
        );
        let empty = write_config(
            &dir,
            "empty.json",
            &format!(
                r#"{{"shards": "dir:/tmp/shards", "source_registry": "  ", "clients": [{client}]}}"#
            ),
        );
        let err = server::load_service_settings(&empty, server::ServeOverrides::default())
            .unwrap_err();
        assert!(
            err.contains("source_registry path must not be empty"),
            "got: {err}"
        );
    }

    struct CeilingStub;

    fn personal_detail() -> ChunkDetail {
        ChunkDetail {
            chunk_id: 7,
            text: "personal chunk with secret provenance".to_string(),
            chunk_idx: 0,
            message_id: "m7".to_string(),
            m_role: Some("user".to_string()),
            session_id: "s7".to_string(),
            s_title: Some("stub".to_string()),
            s_client: Some("stub".to_string()),
            s_created_at: Some("2026-11-26T00:00:00Z".to_string()),
            l_kind: Some("chat".to_string()),
            l_topics: Some("[]".to_string()),
            l_projects: Some("[]".to_string()),
            eff_sensitivity: Some("personal".to_string()),
        }
    }

    fn secret_occurrence() -> ChunkOccurrence {
        ChunkOccurrence {
            shard: "atlas-core-secret-x-00".to_string(),
            tier: Tier::Secret,
            canonical_hash: "deadbeef".to_string(),
            source: "repo@commit:path".to_string(),
            span_start: 0,
            span_end: 4,
            observed_at: "2026-11-26T00:00:00Z".to_string(),
            root_id: Some("root-1".to_string()),
        }
    }

    impl SearchBackend for CeilingStub {
        fn name(&self) -> &'static str {
            "stub"
        }
        fn vec_table(&self) -> &str {
            atlas_core::DEFAULT_VEC_TABLE
        }
        fn notes(&self) -> Vec<String> {
            Vec::new()
        }
        fn branch_pools(
            &self,
            _q: &BranchQuery,
            _f: &atlas_core::Filters,
        ) -> Result<BranchPools, atlas_core::Error> {
            Ok(BranchPools::default())
        }
        fn chunk_details(&self, ids: &[i64]) -> Result<Vec<ChunkDetail>, atlas_core::Error> {
            Ok(ids
                .iter()
                .filter_map(|id| (*id == 7).then(personal_detail))
                .collect())
        }
        fn neighbors(
            &self,
            _message_id: &str,
            _chunk_idx: i64,
            _n: i64,
        ) -> Result<Vec<NeighborRow>, atlas_core::Error> {
            Ok(Vec::new())
        }
        fn message(&self, _id: &str) -> Result<Option<MessageRow>, atlas_core::Error> {
            Ok(None)
        }
        fn occurrences(
            &self,
            _canonical_hash: &str,
            _tier_ceiling: TierCeiling,
            caller_ceiling: atlas_core::Sensitivity,
        ) -> Result<Vec<ChunkOccurrence>, atlas_core::Error> {
            // Emulate the shard fan-out's ceiling gate: the secret-tier
            // locator is visible only to a secret-bearing caller.
            if caller_ceiling >= atlas_core::Sensitivity::SecretBearing {
                Ok(vec![secret_occurrence()])
            } else {
                Ok(Vec::new())
            }
        }
    }

    struct FixedEmbedder;

    impl atlas_core::QueryEmbedder for FixedEmbedder {
        fn embed_query(
            &self,
            _text: &str,
            _trunc: usize,
        ) -> Result<Vec<f32>, atlas_core::Error> {
            Err(atlas_core::Error::fatal("no embedder in test"))
        }
    }

    fn engine() -> server::CoreSearchEngine<CeilingStub, FixedEmbedder> {
        use std::sync::Arc;
        let state: Arc<dyn Fn() -> String + Send + Sync> = Arc::new(|| "test".to_string());
        server::CoreSearchEngine::new(Arc::new(CeilingStub), Arc::new(FixedEmbedder), state)
    }

    #[test]
    fn personal_caller_get_chunk_hides_secret_provenance() {
        // Oracle: `SearchEngine::chunk` is ceiling-checked and Fleet
        // threads the caller's ceiling into
        // `run_get_chunk_with_context`, whose occurrences are bounded
        // by grant ∩ context. A personal caller asking for a personal
        // chunk whose registry row is secret-tier must see no
        // secret-tier provenance; a secret caller sees it.
        use crate::server::SearchEngine;
        let engine = engine();
        let personal = engine.chunk(7, 0, ProtoSensitivity::Personal).unwrap();
        let leaked = personal
            .get("occurrences")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter().any(|o| {
                    o.get("tier").and_then(|t| t.as_str()) == Some(Tier::Secret.name())
                        || o.get("source").and_then(|s| s.as_str())
                            == Some("repo@commit:path")
                })
            })
            .unwrap_or(false);
        assert!(
            !leaked,
            "personal caller must not receive secret-tier occurrence provenance: {personal}"
        );
        let secret = engine
            .chunk(7, 0, ProtoSensitivity::SecretBearing)
            .unwrap();
        let seen = secret
            .get("occurrences")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter().any(|o| {
                    o.get("source").and_then(|s| s.as_str()) == Some("repo@commit:path")
                })
            })
            .unwrap_or(false);
        assert!(
            seen,
            "secret caller must still receive its own provenance: {secret}"
        );
        // The disclosure context import is load-bearing: the engine
        // path must stay on LocalOwner + caller ceiling, never the
        // standalone secret-bearing default alone.
        let _ = DisclosureContext::LocalOwner;
    }
}
