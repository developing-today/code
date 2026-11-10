//! `fleet search` — the Atlas search domain (fleet step 5,
//! developing-today/code#5433; design note §2/§6: search joins
//! last, after the #5350 step-6 cutover, and joins *unchanged*).
//!
//! The domain is the atlas-search **client surface** over the live
//! service contract:
//!
//!   * `search <query>` — hybrid FTS+vector search with the
//!     classification filters, mirroring `atlas-search search`'s
//!     flags one-for-one where they translate to the wire request.
//!   * `get-chunk <id>` — one chunk with provenance, mirroring
//!     `atlas-search get-chunk`.
//!
//! Routing follows the Atlas Q3 ruling, the same defaults the
//! deployed atlas-search clients carry:
//!
//!   * **sandbox seat → via-service by default**, with a LOUD
//!     standalone fallback. The via-service client is proxy-blind
//!     (#5597); the sandbox reaches the laptop's loopback service
//!     through the loopback forward on the shared laptop-ssh
//!     control connection — the deployed `atlas-search` wrapper's
//!     established pattern, mirrored in
//!     [`client::ensure_sandbox_forward`], not a second mechanism.
//!   * **laptop seat → standalone by default.** The seat signal is
//!     the service's own config file (`~/.local/state/atlas-search/
//!     service.json`, written by `atlas-search init`): only the
//!     serving host has one.
//!
//! Explicit routing always wins: `--via-service` / `--standalone`,
//! then `ATLAS_SEARCH_VIA=service`, then fleet.toml `[search] via`.
//!
//! "Standalone" for this domain means the kept standalone
//! `atlas-search` artifact on PATH (design §7's rollback
//! inventory): the fleet workspace deliberately carries no second
//! embedder/backend, so a standalone search delegates to that
//! binary with the caller's arguments in atlas-search's own shape,
//! inheriting its stdout byte-for-byte. Via-service searches run
//! the job protocol in-process (see [`client`]) and render through
//! the ported Python-compatible writers (see `pyjson`/`render`),
//! so both routes print what atlas-search prints.
//!
//! Ceilings are enforced **server-side**: the client sends the
//! requested ceiling and the host's client identity (atlas's
//! client.json chain — sandbox-scripts is `personal`, drew-cli is
//! `secret-bearing`) caps it. The fleet verb adds no ceiling logic
//! of its own.
//!
//! The service role is *not* this domain: it is `fleet serve search`
//! (see `crate::serve`, #5604), and the bin routes
//! `search serve` there before this client sees it. `init` and
//! `fetch-model` stay atlas-search verbs and are usage errors
//! naming that.
//!
//! Exit codes mirror atlas-search: 0 ok · 2 usage/validation ·
//! 1 service-job/backend failure surfaced without a fallback ·
//! 3 routing unavailable (no service and no standalone artifact).

use crate::{client, proto, pyjson, render};

use std::process::{Command, Stdio};

use fleet_config::Config;
use serde_json::Value as Json;

use proto::{FilterSpec, SearchMode, SearchRequest, Sensitivity, VectorBackend};

const EXIT_OK: i32 = 0;
const EXIT_FAILURE: i32 = 1;
const EXIT_USAGE: i32 = 2;
const EXIT_UNAVAILABLE: i32 = 3;

const DEFAULT_MODEL: &str = "Snowflake/snowflake-arctic-embed-m-v2.0";

const SEARCH_USAGE: &str = "\
usage: fleet search <query> [options]
       fleet search search <query> [options]   (atlas-search's own shape)
       fleet search get-chunk <chunk-id> [--neighbors N] [--text]

Hybrid FTS+vector search over the Atlas corpus, with classification
filters — the atlas-search client surface (step 5, #5433).

Search options (mirroring `atlas-search search`):
  --k N                 final results (default 10)
  --pool N              per-branch candidates, 1..=400 (default 100)
  --mode MODE           hybrid | fts | vector (default hybrid)
  --kind CSV            classification kind filter
  --topic CSV           topic filter
  --project CSV         project filter
  --client CSV          client filter
  --since YYYY-MM-DD    sessions created on/after (inclusive)
  --until YYYY-MM-DD    sessions created on/before (inclusive)
  --max-sensitivity LV  shareable | personal | local-only | secret-bearing
                        (default personal; the server clamps it to this
                        client's ceiling — drew-cli secret-bearing,
                        sandbox-scripts personal)
  --group-by-session    roll chunk hits up to sessions
  --expand N            include N neighboring chunks around each hit
  --with-message        include the parent message text
  --text                human-readable output instead of JSON
  --model NAME          embedding model (default Snowflake arctic-embed-m-v2.0)
  --embedder NAME       auto | fastembed | st (default auto)
  --trunc N             truncate vectors to N dims (0 = full)
  --vector-backend B    scan | index-fill | index-fixed (default scan)
  --via-service         prefer the atlas service (job protocol)
  --standalone          run standalone (the atlas-search artifact)
  --service-url URL     service base URL (env ATLAS_SEARCH_URL;
                        default http://127.0.0.1:18760)

Routing defaults (Atlas Q3): the sandbox seat prefers the service,
with a loud standalone fallback; the laptop seat (the serving host)
defaults to standalone. get-chunk is job-scoped in the protocol, so
it always runs standalone, with a loud note when via-service is in
effect — atlas-search's own shape for that verb.";

/// Which route a search takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    ViaService,
    Standalone,
}

/// Parsed `search` verb arguments (the subset of atlas-search's
/// SearchArgs that translates to the wire request or to routing;
/// standalone-only engine knobs like --backend/--db are not part
/// of this surface — the design's "no more").
#[derive(Debug, Clone)]
struct SearchArgs {
    query: String,
    k: i64,
    pool: i64,
    mode: String,
    kind: Option<String>,
    topic: Option<String>,
    project: Option<String>,
    client_filter: Option<String>,
    since: Option<String>,
    until: Option<String>,
    max_sensitivity: String,
    group_by_session: bool,
    expand: i64,
    with_message: bool,
    text: bool,
    model: String,
    embedder: String,
    trunc: i64,
    vector_backend: String,
    via_flag: bool,
    standalone_flag: bool,
    service_url: Option<String>,
}

impl Default for SearchArgs {
    fn default() -> Self {
        SearchArgs {
            query: String::new(),
            k: 10,
            pool: 100,
            mode: "hybrid".to_string(),
            kind: None,
            topic: None,
            project: None,
            client_filter: None,
            since: None,
            until: None,
            max_sensitivity: "personal".to_string(),
            group_by_session: false,
            expand: 0,
            with_message: false,
            text: false,
            model: DEFAULT_MODEL.to_string(),
            embedder: "auto".to_string(),
            trunc: 0,
            vector_backend: "scan".to_string(),
            via_flag: false,
            standalone_flag: false,
            service_url: None,
        }
    }
}

/// (exit code, message) — the caller prints `fleet search: <message>`.
type CliError = (i32, String);

fn usage_err(message: impl Into<String>) -> CliError {
    (EXIT_USAGE, message.into())
}

/// Validate one count flag against the service's request contract
/// (atlas-search's `count`, mirroring the server's
/// `validate_request`): `--k` at least 1, `--pool` between 1 and
/// 400, and the zero-is-valid counts merely non-negative.
/// Out-of-range values are a fatal (exit 2), never a silent clamp
/// to 0 (#5407 — the rule and the messages are atlas's).
fn count(flag: &str, v: i64, min: i64, max: Option<i64>) -> Result<usize, CliError> {
    if v >= min && max.is_none_or(|m| v <= m) {
        return Ok(usize::try_from(v).expect("range-checked count converts"));
    }
    let msg = match (min, max) {
        (_, Some(m)) => format!("{flag} must be between {min} and {m} (design section 2)"),
        (1, None) => format!("{flag} must be at least 1"),
        _ => format!("{flag} must not be negative"),
    };
    Err((EXIT_USAGE, msg))
}

fn split_csv(value: Option<&str>) -> Vec<String> {
    // atlas-core's split_csv: comma-split, trimmed, empties dropped.
    match value {
        None => Vec::new(),
        Some(v) => v
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
    }
}

/// Take the value for a `--flag value` pair, or fail as usage.
fn take_value(args: &[String], i: &mut usize, flag: &str) -> Result<String, CliError> {
    *i += 1;
    args.get(*i)
        .cloned()
        .ok_or_else(|| usage_err(format!("{flag} needs a value")))
}

fn parse_i64(flag: &str, value: &str) -> Result<i64, CliError> {
    value
        .parse::<i64>()
        .map_err(|_| usage_err(format!("{flag}: invalid integer {value:?}")))
}

/// Parse the search arguments (everything after the optional
/// `search` subcommand token). Exactly one positional: the query.
fn parse_search_args(args: &[String]) -> Result<SearchArgs, CliError> {
    let mut out = SearchArgs::default();
    let mut query: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "--k" => {
                let v = take_value(args, &mut i, "--k")?;
                out.k = parse_i64("--k", &v)?;
            }
            "--pool" => {
                let v = take_value(args, &mut i, "--pool")?;
                out.pool = parse_i64("--pool", &v)?;
            }
            "--mode" => out.mode = take_value(args, &mut i, "--mode")?,
            "--kind" => out.kind = Some(take_value(args, &mut i, "--kind")?),
            "--topic" => out.topic = Some(take_value(args, &mut i, "--topic")?),
            "--project" => out.project = Some(take_value(args, &mut i, "--project")?),
            "--client" => out.client_filter = Some(take_value(args, &mut i, "--client")?),
            "--since" => out.since = Some(take_value(args, &mut i, "--since")?),
            "--until" => out.until = Some(take_value(args, &mut i, "--until")?),
            "--max-sensitivity" => {
                out.max_sensitivity = take_value(args, &mut i, "--max-sensitivity")?
            }
            "--group-by-session" => out.group_by_session = true,
            "--expand" => {
                let v = take_value(args, &mut i, "--expand")?;
                out.expand = parse_i64("--expand", &v)?;
            }
            "--with-message" => out.with_message = true,
            "--text" => out.text = true,
            "--model" => out.model = take_value(args, &mut i, "--model")?,
            "--embedder" => out.embedder = take_value(args, &mut i, "--embedder")?,
            "--trunc" => {
                let v = take_value(args, &mut i, "--trunc")?;
                out.trunc = parse_i64("--trunc", &v)?;
            }
            "--vector-backend" => {
                out.vector_backend = take_value(args, &mut i, "--vector-backend")?
            }
            "--via-service" => out.via_flag = true,
            "--standalone" => out.standalone_flag = true,
            "--service-url" => out.service_url = Some(take_value(args, &mut i, "--service-url")?),
            "--help" | "-h" => return Err((EXIT_OK, "__help__".to_string())),
            other if other.starts_with('-') => {
                return Err(usage_err(format!("unknown option {other:?}")));
            }
            other => {
                if query.is_some() {
                    return Err(usage_err(format!(
                        "unexpected extra argument {other:?} (the query is one positional)"
                    )));
                }
                query = Some(other.to_string());
            }
        }
        i += 1;
    }
    out.query = query.ok_or_else(|| usage_err("a query is required"))?;
    if out.via_flag && out.standalone_flag {
        return Err(usage_err(
            "--via-service and --standalone are mutually exclusive",
        ));
    }
    Ok(out)
}

/// Routing resolution, in precedence order: explicit flags, the
/// atlas env, fleet.toml `[search] via`, then the Q3 host default
/// (serving host → standalone; any other seat → via-service).
fn resolve_route(parsed: &SearchArgs, config: &Config) -> Route {
    if parsed.via_flag {
        return Route::ViaService;
    }
    if parsed.standalone_flag {
        return Route::Standalone;
    }
    if client::via_env() {
        return Route::ViaService;
    }
    match config.search_via().as_deref() {
        Some("service") => return Route::ViaService,
        Some("standalone") => return Route::Standalone,
        _ => {}
    }
    if client::serving_host_marker().exists() {
        Route::Standalone
    } else {
        Route::ViaService
    }
}

/// Translate the parsed surface into the wire request — the same
/// translation atlas-search's `proto_search_request` performs,
/// including its counts validation first (#5407).
fn proto_request(parsed: &SearchArgs) -> Result<SearchRequest, CliError> {
    let k = count("--k", parsed.k, 1, None)?;
    let pool = count("--pool", parsed.pool, 1, Some(400))?;
    let expand = count("--expand", parsed.expand, 0, None)?;
    let trunc = count("--trunc", parsed.trunc, 0, None)?;
    let mode = SearchMode::from_name(&parsed.mode)
        .ok_or_else(|| usage_err(format!("--mode: unknown mode {:?}", parsed.mode)))?;
    let vector_backend = VectorBackend::from_name(&parsed.vector_backend).ok_or_else(|| {
        usage_err(format!(
            "--vector-backend: unknown backend {:?}",
            parsed.vector_backend
        ))
    })?;
    match parsed.embedder.as_str() {
        "auto" | "fastembed" | "st" => {}
        other => {
            return Err(usage_err(format!("--embedder: unknown embedder {other:?}")));
        }
    }
    Ok(SearchRequest {
        query: parsed.query.clone(),
        mode,
        filters: FilterSpec {
            kind: split_csv(parsed.kind.as_deref()),
            topic: split_csv(parsed.topic.as_deref()),
            project: split_csv(parsed.project.as_deref()),
            client: split_csv(parsed.client_filter.as_deref()),
            since: parsed.since.clone(),
            until: parsed.until.clone(),
        },
        // An unrecognized ceiling name is NOT silently clamped
        // here: atlas-search declines the via-service attempt and
        // lets the standalone path produce its exact validation
        // fatal. The caller checks this case before submitting.
        ceiling: Sensitivity::from_name(&parsed.max_sensitivity).unwrap_or(Sensitivity::Personal),
        k,
        pool,
        group_by_session: parsed.group_by_session,
        expand,
        with_message: parsed.with_message,
        model: parsed.model.clone(),
        embedder: parsed.embedder.clone(),
        trunc,
        vector_backend,
        page_limit: None,
    })
}

fn print_output(out: &Json, text: bool) {
    if text {
        println!("{}", render::render_text(out));
    } else {
        println!("{}", pyjson::dumps_pretty(out));
    }
}

/// The shipped fallback contract, atlas-search's wording with the
/// fleet prefix: warn loudly on stderr...
fn via_fallback_warning(detail: &str) {
    eprintln!("fleet search: warning: --via-service: {detail}; falling back to standalone");
}

/// Run the standalone atlas-search artifact with the given
/// atlas-shaped argv, inheriting stdio, and propagate its exit
/// code. Absent artifact → exit 3 (routing unavailable), never a
/// silent success.
fn delegate(argv: &[String]) -> i32 {
    let mut cmd = Command::new("atlas-search");
    cmd.args(argv)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    // The delegation is a *standalone* run: never let an ambient
    // ATLAS_SEARCH_VIA from the caller's environment silently
    // re-route it (the sandbox wrapper makes its own via decision
    // from its health probe, as it does for direct callers).
    cmd.env_remove("ATLAS_SEARCH_VIA");
    match cmd.status() {
        Ok(status) => status.code().unwrap_or(EXIT_FAILURE),
        Err(e) => {
            eprintln!(
                "fleet search: the standalone atlas-search artifact is not available on PATH ({e}); \
                 standalone searches and get-chunk need it (it is the search domain's kept standalone, design section 7)"
            );
            EXIT_UNAVAILABLE
        }
    }
}

/// The standalone argv for a search invocation: the caller's raw
/// args with the `search` subcommand made explicit and the
/// fleet-only routing flags removed (atlas-search has no
/// --standalone; its --via-service must not ride along either,
/// since this run exists precisely because via was declined or
/// already failed).
fn standalone_search_argv(raw: &[String]) -> Vec<String> {
    let mut out = vec!["search".to_string()];
    let mut i = 0;
    while i < raw.len() {
        match raw[i].as_str() {
            "--via-service" | "--standalone" => {}
            "--service-url" => {
                i += 1; // also drop its value
            }
            other => out.push(other.to_string()),
        }
        i += 1;
    }
    out
}

fn cmd_search(config: &Config, raw: &[String]) -> i32 {
    // atlas-search's own subcommand token, when present, is
    // consumed; `fleet search <query>` is the same surface with
    // the token implied.
    let body: &[String] = if raw.first().map(|s| s.as_str()) == Some("search") {
        &raw[1..]
    } else {
        raw
    };
    let parsed = match parse_search_args(body) {
        Ok(p) => p,
        Err((EXIT_OK, _)) => {
            println!("{SEARCH_USAGE}");
            return EXIT_OK;
        }
        Err((code, msg)) => {
            eprintln!("fleet search: {msg}");
            return code;
        }
    };
    match resolve_route(&parsed, config) {
        Route::Standalone => delegate(&standalone_search_argv(body)),
        Route::ViaService => {
            // Count validation comes first (#5407): before any
            // service attempt, so an out-of-range count fails
            // identically whether or not a service is running.
            let request = match proto_request(&parsed) {
                Ok(r) => r,
                Err((code, msg)) => {
                    eprintln!("fleet search: {msg}");
                    return code;
                }
            };
            if Sensitivity::from_name(&parsed.max_sensitivity).is_none() {
                // Let the standalone path produce atlas-search's
                // exact validation fatal rather than clamping here.
                return delegate(&standalone_search_argv(body));
            }
            let base = parsed
                .service_url
                .clone()
                .or_else(|| config.search_url())
                .map(|u| u.trim_end_matches('/').to_string())
                .unwrap_or_else(|| client::base_url(None));
            // The sandbox seat's reachability step (the wrapper
            // pattern); a no-op where the service is directly
            // reachable or no wrapper tooling exists.
            client::ensure_sandbox_forward(&base);
            match client::search(&base, &request) {
                Ok(out) => {
                    print_output(&out, parsed.text);
                    EXIT_OK
                }
                Err(detail) => {
                    via_fallback_warning(&detail);
                    delegate(&standalone_search_argv(body))
                }
            }
        }
    }
}

fn cmd_get_chunk(config: &Config, raw: &[String]) -> i32 {
    // raw includes the `get-chunk` token. The verb always serves
    // standalone (job-scoped in the protocol — atlas-search's own
    // shape); when via-service was *requested* (flag, env, or
    // config — the seat default does not request it), say so
    // loudly first, with the probed detail, mirroring
    // atlas-search's note.
    let via_requested = raw.iter().any(|a| a == "--via-service")
        || client::via_env()
        || config.search_via().as_deref() == Some("service");
    if via_requested {
        let base = client::base_url(None);
        via_fallback_warning(&client::get_chunk_fallback_detail(&base));
    }
    let mut argv: Vec<String> = vec!["get-chunk".to_string()];
    argv.extend(raw[1..].iter().cloned());
    delegate(&argv)
}

/// The domain entry point: `fleet search ...` (and the
/// `atlas-search` argv0 route, whose args arrive in atlas-search's
/// own shape).
pub fn run(config: &Config, args: &[String]) -> i32 {
    match args.first().map(|s| s.as_str()) {
        None => {
            eprintln!("{SEARCH_USAGE}");
            EXIT_USAGE
        }
        Some("get-chunk") => cmd_get_chunk(config, args),
        Some("serve") | Some("init") | Some("fetch-model") => {
            eprintln!(
                "fleet search: '{0}' is an atlas-search service verb, not part of the fleet \
                 search client surface — the service role is `fleet serve search`; `init` and \
                 `fetch-model` stay on the standalone atlas-search artifact",
                args[0]
            );
            EXIT_USAGE
        }
        Some("--version") | Some("-V") => {
            // The version shortcut takes no trailing arguments: like
            // the queue domain (#5603), a trailing argument is a usage
            // error naming it, not silently discarded success — a real
            // verb after the flag () was
            // swallowed whole with exit 0 (#5607).
            if let Some(extra) = args.get(1) {
                eprintln!("fleet search: unexpected argument {extra} for {}", args[0]);
                eprintln!("{SEARCH_USAGE}");
                return EXIT_USAGE;
            }
            println!("fleet search {}", env!("CARGO_PKG_VERSION"));
            EXIT_OK
        }
        _ => cmd_search(config, args),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn version_shortcut_rejects_trailing_arguments() {
        // #5607:  printed the version and exited
        // 0, swallowing a real verb. Bare flags stay accepted.
        let config = Config::default();
        for flag in ["--version", "-V"] {
            assert_eq!(
                run(&config, &args(&[flag, "--bogus"])),
                EXIT_USAGE,
                "{flag} must reject a trailing argument"
            );
            assert_eq!(
                run(&config, &args(&[flag, "get-chunk"])),
                EXIT_USAGE,
                "{flag} must reject a trailing verb"
            );
            assert_eq!(
                run(&config, &args(&[flag])),
                EXIT_OK,
                "bare {flag} stays accepted"
            );
        }
    }

    #[test]
    fn count_validation_mirrors_the_service_contract() {
        // The oracle is atlas-search's own count test (#5407).
        assert_eq!(count("--k", 1, 1, None), Ok(1));
        assert_eq!(count("--k", 10, 1, None), Ok(10));
        assert_eq!(
            count("--k", 0, 1, None).unwrap_err(),
            (2, "--k must be at least 1".to_string())
        );
        assert!(count("--k", -1, 1, None).is_err());
        assert_eq!(count("--pool", 400, 1, Some(400)), Ok(400));
        assert_eq!(
            count("--pool", 401, 1, Some(400)).unwrap_err(),
            (
                2,
                "--pool must be between 1 and 400 (design section 2)".to_string()
            )
        );
        assert!(count("--pool", 0, 1, Some(400)).is_err());
        for flag in ["--expand", "--trunc"] {
            assert_eq!(count(flag, 0, 0, None), Ok(0), "{flag}");
            assert_eq!(
                count(flag, -1, 0, None).unwrap_err(),
                (2, format!("{flag} must not be negative")),
                "{flag}"
            );
        }
    }

    #[test]
    fn parse_defaults_match_atlas_search() {
        let p = parse_search_args(&args(&["rust error handling"])).unwrap();
        assert_eq!(p.query, "rust error handling");
        assert_eq!(p.k, 10);
        assert_eq!(p.pool, 100);
        assert_eq!(p.mode, "hybrid");
        assert_eq!(p.max_sensitivity, "personal");
        let req = proto_request(&p).unwrap();
        assert_eq!(req.k, 10);
        assert_eq!(req.pool, 100);
        assert_eq!(req.ceiling, Sensitivity::Personal);
        assert_eq!(req.mode, proto::SearchMode::Hybrid);
        assert_eq!(req.vector_backend, proto::VectorBackend::Scan);
    }

    #[test]
    fn parse_filters_and_flags_translate() {
        let p = parse_search_args(&args(&[
            "q",
            "--kind",
            "note, decision",
            "--since",
            "2026-01-01",
            "--max-sensitivity",
            "secret-bearing",
            "--group-by-session",
            "--mode",
            "fts",
            "--k",
            "25",
        ]))
        .unwrap();
        let req = proto_request(&p).unwrap();
        assert_eq!(
            req.filters.kind,
            vec!["note".to_string(), "decision".to_string()]
        );
        assert_eq!(req.filters.since.as_deref(), Some("2026-01-01"));
        assert_eq!(req.ceiling, Sensitivity::SecretBearing);
        assert!(req.group_by_session);
        assert_eq!(req.mode, proto::SearchMode::Fts);
        assert_eq!(req.k, 25);
    }

    #[test]
    fn parse_rejects_conflicts_and_extras() {
        assert!(parse_search_args(&args(&["q", "--via-service", "--standalone"])).is_err());
        assert!(parse_search_args(&args(&["q", "extra"])).is_err());
        assert!(parse_search_args(&args(&["--k"])).is_err());
        assert!(parse_search_args(&args(&[])).is_err());
        assert!(parse_search_args(&args(&["q", "--bogus"])).is_err());
        assert!(
            proto_request(&parse_search_args(&args(&["q", "--pool", "401"])).unwrap()).is_err()
        );
    }

    #[test]
    fn standalone_argv_strips_routing_flags_and_names_the_verb() {
        let raw = args(&[
            "q",
            "--via-service",
            "--service-url",
            "http://x:1",
            "--k",
            "5",
        ]);
        assert_eq!(
            standalone_search_argv(&raw),
            args(&["search", "q", "--k", "5"])
        );
        let plain = args(&["hello world", "--text"]);
        assert_eq!(
            standalone_search_argv(&plain),
            args(&["search", "hello world", "--text"])
        );
    }

    #[test]
    fn split_csv_matches_atlas_core() {
        assert_eq!(split_csv(None), Vec::<String>::new());
        assert_eq!(split_csv(Some("")), Vec::<String>::new());
        assert_eq!(
            split_csv(Some("a, b ,,c")),
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }
}
