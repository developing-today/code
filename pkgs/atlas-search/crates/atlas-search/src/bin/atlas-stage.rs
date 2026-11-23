//! atlas-stage — provision Atlas staging units and read the staging
//! registry (developing-today/code#5669, slice 1: the staging tier's
//! versioned schema, provisioning tool and registry; design
//! `doc/2026-11-04T21-27-34Z_design_atlas_scaler_revision` section 4.1).
//!
//! Six verbs:
//! * `atlas-stage provision --db <path> --unit <id> --owner <o>
//!   [--sensitivity internal] --registry <path>` — create the unit's
//!   staging DB, apply the v1 schema, and register the unit as `open`.
//! * `atlas-stage lookup --unit <id> --registry <path>` — print the
//!   registry record as JSON.
//! * `atlas-stage units --state open --registry <path>` — list units
//!   in a lifecycle state (what the aggregator's poll reads).
//! * `atlas-stage ingest --db <path> --unit <id> --repo <r>
//!   --path <p> --file <f> --commit <sha> [--kind config]
//!   [--language yaml] [--sensitivity internal]
//!   [--segments <segments.json>]` — stage one
//!   code/config artifact as an embedding entry (#1518): one
//!   `items` row plus its `chunks_raw` in the unit's staging DB.
//!   With `--segments`, the artifact is staged classified (#12628):
//!   boundary-safe chunks from the declared classification segments,
//!   with one persisted classification record per chunk.
//! * `atlas-stage items --db <path> [--unit <id>]` — list the
//!   items staged in a unit's DB, with chunk counts.
//!
//! * `atlas-stage promote --registry <path> --unit <id>
//!   --shard-dir <dir> [--topic codeconfig]` — the aggregator's
//!   embed-at-promotion step (#5669 aggregator slice, #1518): embed
//!   the unit's staged chunks with the mandated model, write a new
//!   core scan shard in the shard dir, stamp the unit DB's
//!   embedding-model provenance, and promote the unit to `merged`
//!   through the registry's M6 gate. Needs the embedding model +
//!   ONNX runtime (ATLAS_MODEL_DIR / ATLAS_ORT_LIB, the same
//!   resolution the atlas-search binary uses).
//!
//! Five of the six are offline: a staging DB is a local libSQL/SQLite
//! file today; hosted Turso creation is the aggregator slice's
//! deployment surface, not this foundation. `promote` is the
//! exception: it needs the embedding model + ONNX runtime, as its
//! entry above states.

use std::path::PathBuf;
use std::process::ExitCode;

use atlas_core::shards::Tier;
use atlas_core::stage::{Registry, UnitState};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "atlas-stage",
    version,
    about = "Provision Atlas staging units and read the staging registry (#5669)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a staging DB, apply the v1 schema, and register the unit.
    Provision {
        /// The staging database file to create.
        #[arg(long)]
        db: PathBuf,
        /// The staging-unit id (queue task id, session id, epic id, ...).
        #[arg(long)]
        unit: String,
        /// The unit's owner (the writer whose token may write here).
        #[arg(long)]
        owner: String,
        /// Physical sensitivity level (public|internal|secret; default internal).
        #[arg(long, default_value = "internal")]
        sensitivity: String,
        /// The registry store file (created if missing).
        #[arg(long)]
        registry: PathBuf,
    },
    /// Print one unit's registry record as JSON.
    Lookup {
        #[arg(long)]
        unit: String,
        #[arg(long)]
        registry: PathBuf,
    },
    /// Stage one code/config artifact file as an embedding entry (#1518).
    Ingest {
        /// The unit's staging database file (already provisioned).
        #[arg(long)]
        db: PathBuf,
        /// The staging-unit id the entry belongs to.
        #[arg(long)]
        unit: String,
        /// The repository the artifact came from.
        #[arg(long)]
        repo: String,
        /// The artifact's path inside the repo.
        #[arg(long)]
        path: String,
        /// The local file whose bytes are staged.
        #[arg(long)]
        file: PathBuf,
        /// The commit SHA the bytes were read at.
        #[arg(long)]
        commit: String,
        /// The entry kind (prose|code|config).
        #[arg(long, default_value = "config")]
        kind: String,
        /// The artifact language (yaml, toml, rust, ...); when empty,
        /// the staged file's extension is used.
        #[arg(long, default_value = "")]
        language: String,
        /// Physical sensitivity level (public|internal|secret; default internal).
        #[arg(long, default_value = "internal")]
        sensitivity: String,
        /// Optional JSON file declaring the artifact's source-native
        /// classification segments (#12628): an array of
        /// {"start": <byte>, "end": <byte>, "sensitivity":
        /// shareable|personal|local-only|secret-bearing,
        /// "certain": <bool, default true>}. When given, the
        /// artifact is staged classified: boundary-safe chunks plus
        /// one persisted classification record per chunk.
        #[arg(long)]
        segments: Option<PathBuf>,
    },
    /// List the items staged in a unit's staging DB.
    Items {
        /// The staging database file to read.
        #[arg(long)]
        db: PathBuf,
        /// Only this unit's items (default: all units in the DB).
        #[arg(long)]
        unit: Option<String>,
    },
    /// Embed a unit's staged chunks into a new core scan shard and
    /// promote the unit to merged (aggregator embed-at-promotion).
    Promote {
        /// The registry store file.
        #[arg(long)]
        registry: PathBuf,
        /// The staging-unit id to promote.
        #[arg(long)]
        unit: String,
        /// Directory the new core scan shard is written into
        /// (created if missing).
        #[arg(long)]
        shard_dir: PathBuf,
        /// Shard topic for the promoted entries.
        #[arg(long, default_value = "codeconfig")]
        topic: String,
    },
    /// List units in a lifecycle state (open|closing|merged).
    Units {
        #[arg(long, default_value = "open")]
        state: String,
        #[arg(long)]
        registry: PathBuf,
    },
}

fn now_utc() -> String {
    // RFC 3339, second precision: the registry records when, not
    // sub-second detail. Derived from the system clock without pulling
    // a datetime crate into atlas-core.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // civil-from-days (Howard Hinnant's algorithm), days since 1970-01-01.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!("{year:04}-{month:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

fn parse_tier(s: &str) -> Result<Tier, String> {
    Tier::parse(s).ok_or_else(|| format!("unknown sensitivity {s:?} (want public|internal|secret)"))
}

fn parse_state(s: &str) -> Result<UnitState, String> {
    UnitState::parse(s).ok_or_else(|| format!("unknown state {s:?} (want open|closing|merged)"))
}

/// Load the declared classification segments for `ingest
/// --segments`: a JSON array of {"start", "end", "sensitivity",
/// "certain"?} objects over the artifact's byte offsets. Parsed
/// strictly — a missing or mistyped field, or an unknown sensitivity
/// name, is a usage error, never a silent default.
fn load_segments(
    path: &PathBuf,
) -> Result<Vec<atlas_core::boundary::ClassificationSegment>, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("reading segments {}: {e}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| format!("parsing segments {}: {e}", path.display()))?;
    let array = value
        .as_array()
        .ok_or("segments file must be a JSON array of segment objects")?;
    let mut out = Vec::with_capacity(array.len());
    for (i, entry) in array.iter().enumerate() {
        let field = |name: &str| {
            entry
                .get(name)
                .ok_or_else(|| format!("segments[{i}]: missing field {name:?}"))
        };
        let start = field("start")?
            .as_u64()
            .ok_or_else(|| format!("segments[{i}]: \"start\" must be an unsigned integer"))?
            as usize;
        let end = field("end")?
            .as_u64()
            .ok_or_else(|| format!("segments[{i}]: \"end\" must be an unsigned integer"))?
            as usize;
        let sensitivity_name = field("sensitivity")?
            .as_str()
            .ok_or_else(|| format!("segments[{i}]: \"sensitivity\" must be a string"))?;
        let sensitivity = atlas_core::filters::Sensitivity::from_name(sensitivity_name)
            .ok_or_else(|| {
                format!(
                    "segments[{i}]: unknown sensitivity {sensitivity_name:?} \
                     (want shareable|personal|local-only|secret-bearing)"
                )
            })?;
        let certain = match entry.get("certain") {
            None => true,
            Some(v) => v
                .as_bool()
                .ok_or_else(|| format!("segments[{i}]: \"certain\" must be a boolean"))?,
        };
        out.push(atlas_core::boundary::ClassificationSegment {
            start,
            end,
            sensitivity,
            certain,
        });
    }
    Ok(out)
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Provision {
            db,
            unit,
            owner,
            sensitivity,
            registry,
        } => {
            let sensitivity = parse_tier(&sensitivity)?;
            let registry = Registry::open(&registry).map_err(|e| e.to_string())?;
            let record = registry
                .provision(&db, &unit, &owner, sensitivity, &now_utc())
                .map_err(|e| e.to_string())?;
            println!("{}", unit_json(&record));
            Ok(())
        }
        Command::Lookup { unit, registry } => {
            let registry = Registry::open(&registry).map_err(|e| e.to_string())?;
            match registry.lookup(&unit).map_err(|e| e.to_string())? {
                Some(record) => {
                    println!("{}", unit_json(&record));
                    Ok(())
                }
                None => Err(format!("no staging unit {unit:?} in the registry")),
            }
        }
        Command::Ingest {
            db,
            unit,
            repo,
            path,
            file,
            commit,
            kind,
            language,
            sensitivity,
            segments,
        } => {
            let sensitivity = parse_tier(&sensitivity)?;
            let text = std::fs::read_to_string(&file)
                .map_err(|e| format!("reading {}: {e}", file.display()))?;
            let language = if language.is_empty() {
                file.extension()
                    .and_then(|x| x.to_str())
                    .unwrap_or("")
                    .to_string()
            } else {
                language
            };
            let entry = atlas_core::stage::ArtifactEntry {
                kind,
                repo,
                path,
                commit_sha: commit,
                language,
                sensitivity,
                text,
            };
            let item_id = match &segments {
                Some(segments_path) => {
                    let declared = load_segments(segments_path)?;
                    atlas_core::stage::stage_classified_artifact_in_db(
                        &db,
                        &unit,
                        &entry,
                        &declared,
                        &now_utc(),
                    )
                    .map_err(|e| e.to_string())?
                }
                None => atlas_core::stage::stage_artifact_in_db(&db, &unit, &entry, &now_utc())
                    .map_err(|e| e.to_string())?,
            };
            println!(
                "{{\"item_id\":{item_id},\"unit\":{},\"source\":{}}}",
                json_str(&unit),
                json_str(&format!(
                    "{}:{}@{}",
                    entry.repo, entry.path, entry.commit_sha
                )),
            );
            Ok(())
        }
        Command::Items { db, unit } => {
            let items = atlas_core::stage::list_staged_items_in_db(&db, unit.as_deref())
                .map_err(|e| e.to_string())?;
            for item in &items {
                println!(
                    "{{\"id\":{},\"unit_id\":{},\"kind\":{},\"source\":{},\"sensitivity\":{},\"chunks\":{},\"meta\":{}}}",
                    item.id,
                    json_str(&item.unit_id),
                    json_str(&item.kind),
                    item.source
                        .as_deref()
                        .map(json_str)
                        .unwrap_or_else(|| "null".to_string()),
                    json_str(&item.sensitivity),
                    item.chunk_count,
                    item.meta.as_deref().unwrap_or("null"),
                );
            }
            Ok(())
        }
        Command::Promote {
            registry,
            unit,
            shard_dir,
            topic,
        } => {
            let registry = Registry::open(&registry).map_err(|e| e.to_string())?;
            let embedder = load_doc_embedder()?;
            let report = atlas_core::aggregate::embed_unit_into_shard(
                &registry,
                &unit,
                &shard_dir,
                &topic,
                &embedder,
                &now_utc(),
            )
            .map_err(|e| e.to_string())?;
            println!(
                "{{\"unit\":{},\"shard\":{},\"shard_path\":{},\"items\":{},\"chunks\":{},\"dims\":{}}}",
                json_str(&report.unit_id),
                json_str(&report.shard_name),
                json_str(&report.shard_path.display().to_string()),
                report.items,
                report.chunks,
                report.dims,
            );
            Ok(())
        }
        Command::Units { state, registry } => {
            let state = parse_state(&state)?;
            let registry = Registry::open(&registry).map_err(|e| e.to_string())?;
            let units = registry.units_in_state(state).map_err(|e| e.to_string())?;
            for u in &units {
                println!("{}", unit_json(u));
            }
            Ok(())
        }
    }
}

/// Build the production document embedder for `promote`: the
/// pinned Snowflake model via atlas-embed, resolved exactly as the
/// atlas-search binary resolves it (ATLAS_MODEL_DIR, then the
/// state-dir / HF-cache snapshots; ATLAS_ORT_LIB, then the state-dir
/// runtime). A missing model or runtime is a loud error, never a
/// silent downgrade.
fn load_doc_embedder() -> Result<atlas_core::AtlasEmbedder, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".to_string())?;
    let model_dir = std::env::var("ATLAS_MODEL_DIR")
        .ok()
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .or_else(|| {
            let state = home.join(".local/state/atlas-search/model/95c2741480856aa9666782eb4afe11959938017f");
            if state.is_dir() {
                return Some(state);
            }
            let hf = home.join(".cache/huggingface/hub/models--Snowflake--snowflake-arctic-embed-m-v2.0/snapshots/95c2741480856aa9666782eb4afe11959938017f");
            if hf.is_dir() { Some(hf) } else { None }
        })
        .ok_or_else(|| {
            "Snowflake model not found (looked in ATLAS_MODEL_DIR, ~/.local/state/atlas-search/model/<rev>, and the HF cache snapshot)".to_string()
        })?;
    let ort_lib = std::env::var("ATLAS_ORT_LIB")
        .ok()
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .or_else(|| {
            let cand = home.join(".local/state/atlas-search/ort/libonnxruntime.so");
            if cand.is_file() {
                return Some(cand);
            }
            let venv = home.join("atlas/v3-venv/lib/python3.14/site-packages/onnxruntime/capi/libonnxruntime.so.1.31.1");
            if venv.is_file() { Some(venv) } else { None }
        })
        .ok_or_else(|| {
            "ONNX Runtime library not found: set ATLAS_ORT_LIB".to_string()
        })?;
    let emb = atlas_embed::Embedder::load(&model_dir, &ort_lib)
        .map_err(|e| format!("embedder load failed: {e}"))?;
    Ok(atlas_core::AtlasEmbedder::new(emb))
}

fn unit_json(unit: &atlas_core::stage::StagingUnit) -> String {
    format!(
        "{{\"unit_id\":{},\"db_uri\":{},\"owner\":{},\"sensitivity\":{},\"state\":{},\"schema_version\":{},\"created_at\":{},\"updated_at\":{}}}",
        json_str(&unit.unit_id),
        json_str(&unit.db_uri),
        json_str(&unit.owner),
        json_str(unit.sensitivity.name()),
        json_str(unit.state.name()),
        unit.schema_version,
        json_str(&unit.created_at),
        json_str(&unit.updated_at),
    )
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("atlas-stage: {e}");
            ExitCode::FAILURE
        }
    }
}
