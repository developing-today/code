//! CLI entry point for `id`, a peer-to-peer file sharing tool.
//!
//! This binary provides commands for storing, retrieving, and sharing content
//! using content-addressed storage backed by Iroh.

use anyhow::Result;
use clap::{CommandFactory, Parser};

// Import from library
use id::{
    Cli, Command, PeekOptions, PeersOptions, SearchOptions, ServeOptions, cmd_find, cmd_get_multi,
    cmd_gethash, cmd_id, cmd_list, cmd_migrate_tags, cmd_peek, cmd_peers, cmd_put_hash,
    cmd_put_multi, cmd_search, cmd_serve, cmd_show, cmd_tag, run_repl,
};

/// Determine the log level based on CLI flags and environment variables.
///
/// Priority order (first match wins):
/// 1. `--debug` flag → "debug"
/// 2. `--log-level <LEVEL>` flag → specified level
/// 3. `RUST_LOG` env var (non-empty) → return empty string to signal use of `EnvFilter` default
/// 4. `LOG_LEVEL` env var → value
/// 5. `DEBUG` env var (if truthy) → "debug"
/// 6. Default → "warn"
fn get_log_level(cli: &Cli) -> String {
    resolve_log_level(
        cli.debug,
        cli.log_level.as_deref(),
        std::env::var("RUST_LOG").ok().as_deref(),
        std::env::var("LOG_LEVEL").ok().as_deref(),
        std::env::var("DEBUG").ok().as_deref(),
    )
}

/// Pure log-level resolution: the environment values are passed in (rather
/// than read here) so tests can pin the full precedence table without
/// mutating process environment in parallel. A non-empty `rust_log` yields
/// the empty-string sentinel: the caller's signal to use the `RUST_LOG`
/// value itself as the base filter. An empty or whitespace-only `rust_log`
/// (or `log_level_env`) carries no configuration and is treated as unset:
/// build and CI environments routinely export these variables empty
/// (nixpkgs' cargo hooks export `RUST_LOG=`), and an empty base filter
/// would leave only the appended per-module caps in effect, silently
/// discarding the documented default level for every other target.
fn resolve_log_level(
    debug_flag: bool,
    log_level_flag: Option<&str>,
    rust_log: Option<&str>,
    log_level_env: Option<&str>,
    debug_env: Option<&str>,
) -> String {
    // 1. --debug flag takes highest precedence
    if debug_flag {
        return "debug".to_owned();
    }

    // 2. --log-level flag (likewise, empty or blank means unconfigured:
    //    an empty return is the RUST_LOG sentinel below, so a blank flag
    //    must never produce it -- #5442)
    if let Some(level) = log_level_flag.filter(|value| !value.trim().is_empty()) {
        return level.trim().to_owned();
    }

    // 3. RUST_LOG env var - return empty to use EnvFilter's default behavior.
    //    Present-but-empty is not a filter specification; treat as unset.
    if rust_log.is_some_and(|value| !value.trim().is_empty()) {
        return String::new();
    }

    // 4. LOG_LEVEL env var (likewise, empty means unconfigured)
    if let Some(level) = log_level_env.filter(|value| !value.trim().is_empty()) {
        return level.trim().to_owned();
    }

    // 5. DEBUG env var (truthy check)
    if let Some(debug) = debug_env {
        let debug = debug.trim();
        let is_truthy = !debug.is_empty() && debug != "0" && debug.to_lowercase() != "false";
        if is_truthy {
            return "debug".to_owned();
        }
    }

    // 6. Default to warn (as documented; --debug is the opt-in for debug)
    "warn".to_owned()
}

/// Initialize tracing from the resolved level `filter` and the
/// structured-logging configuration (#1791; see
/// `id::structured_logging` for the `ID_LOG_*` variables).
///
/// The console destination (stderr) always exists; the file
/// destination is added when configured. Each destination gets its
/// own filter, so their levels are independent. A configured file
/// that cannot be opened, or an unrecognized `ID_LOG_TZ`, warns on
/// stderr (before tracing starts) and logging continues without that
/// piece rather than failing the process over its own logs.
fn init_logging(filter: &str, config: &id::structured_logging::LogConfig) {
    use id::structured_logging::{LogFormat, Rfc3339TzTimer, SharedFileWriter};
    use tracing_subscriber::{Layer, Registry, fmt, layer::SubscriberExt, util::SubscriberInitExt};

    if config.tz_fell_back {
        eprintln!("warning: ID_LOG_TZ value not recognized; log timestamps use UTC");
    }
    if config.format_fell_back {
        eprintln!("warning: ID_LOG_FORMAT value not recognized; logs use text format");
    }
    let timer = Rfc3339TzTimer { zone: config.zone };

    let console_layer: Box<dyn Layer<Registry> + Send + Sync> = match config.format {
        LogFormat::Json => fmt::layer()
            .json()
            .with_writer(std::io::stderr)
            .with_timer(timer)
            .with_current_span(true)
            .with_span_list(true)
            .with_filter(tracing_subscriber::EnvFilter::new(filter))
            .boxed(),
        LogFormat::Text => fmt::layer()
            .with_writer(std::io::stderr)
            .with_timer(timer)
            .with_filter(tracing_subscriber::EnvFilter::new(filter))
            .boxed(),
    };

    let file_layer: Option<Box<dyn Layer<Registry> + Send + Sync>> =
        config
            .file
            .as_ref()
            .and_then(|path| match SharedFileWriter::open(path) {
                Ok(writer) => {
                    let file_filter = config.file_filter_string(filter);
                    let layer: Box<dyn Layer<Registry> + Send + Sync> = match config.format {
                        LogFormat::Json => fmt::layer()
                            .json()
                            .with_writer(move || writer.clone())
                            .with_timer(timer)
                            .with_current_span(true)
                            .with_span_list(true)
                            .with_ansi(false)
                            .with_filter(tracing_subscriber::EnvFilter::new(&file_filter))
                            .boxed(),
                        LogFormat::Text => fmt::layer()
                            .fmt_fields(id::structured_logging::PlainFields::new())
                            .with_writer(move || writer.clone())
                            .with_timer(timer)
                            .with_ansi(false)
                            .with_filter(tracing_subscriber::EnvFilter::new(&file_filter))
                            .boxed(),
                    };
                    Some(layer)
                }
                Err(error) => {
                    eprintln!(
                        "warning: could not open log file {} ({error}); logging to stderr only",
                        path.display()
                    );
                    None
                }
            });

    // Layers are collected into one Vec of boxes on the Registry:
    // stacking two individually boxed layers does not type-check,
    // because a layer boxed for Registry is not a Layer for the
    // already-layered stack.
    let mut layers: Vec<Box<dyn Layer<Registry> + Send + Sync>> = vec![console_layer];
    if let Some(layer) = file_layer {
        layers.push(layer);
    }
    tracing_subscriber::registry().with(layers).init();
}

/// Per-module overrides to suppress noisy third-party modules.
/// These are appended to whatever base level is active, unless the user
/// explicitly sets a level for that module in `RUST_LOG`
/// (e.g. `RUST_LOG=debug,mainline::rpc=trace` keeps `mainline::rpc` at trace).
const NOISY_MODULE_FILTERS: &[&str] = &[
    "mainline::rpc=info",
    "distributed_topic_tracker::crypto::record=info",
    "rustls=info",
    "iroh::address_lookup::mdns=info",
    "rustyline=info",
    "hickory_proto::error=info",
    "hickory_proto::udp::udp_client_stream=info",
    "swarm_discovery::receiver=info",
    "acto::tokio=info",
    "swarm_discovery::sender=info",
    "swarm_discovery::socket=info",
];

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI first so we can use flags for logging configuration
    let cli = Cli::parse_from(id::cli::normalize_args(std::env::args_os()));

    // #2145: `--web-listen` conflicts only with an *explicitly-set*
    // `--bind`/`--port`; clap defaults must not count, so read the value
    // sources from a matches pass (`Cli::parse` above already validated
    // the arguments and would have exited on any error).
    let (bind_explicit, port_explicit) = {
        let matches = Cli::command().get_matches();
        matches
            .subcommand_matches("serve")
            .map(|serve| {
                (
                    serve.value_source("bind") == Some(clap::parser::ValueSource::CommandLine),
                    serve.value_source("port") == Some(clap::parser::ValueSource::CommandLine),
                )
            })
            .unwrap_or((false, false))
    };

    // Switch to the data directory if specified.
    // All data paths (.iroh-store, .iroh-key, .iroh-serve.lock) are CWD-relative,
    // so changing the working directory early makes everything use the right location
    // with zero changes to path logic throughout the codebase.
    if let Some(ref data_dir) = cli.data_dir {
        std::fs::create_dir_all(data_dir)?;
        std::env::set_current_dir(data_dir)?;
    } else if let Some(ref name) = cli.new {
        // --new [NAME] creates .iroh/<name>/ under the current directory
        let name = if name.is_empty() {
            // Generate a short random name (8 hex chars)
            use rand::RngExt;
            let mut rng = rand::rng();
            let n: u32 = rng.random();
            format!("{n:08x}")
        } else {
            // Validate: name must be a simple identifier (no path separators or ..)
            if name.contains('/') || name.contains('\\') || name.contains("..") {
                anyhow::bail!("--new name must be a simple identifier, not a path: {name:?}");
            }
            name.clone()
        };
        let data_dir = std::path::PathBuf::from(".iroh").join(&name);
        if data_dir.exists() {
            anyhow::bail!(
                "instance directory already exists: {}  (use --data-dir to reuse an existing directory)",
                data_dir.display()
            );
        }
        std::fs::create_dir_all(&data_dir)?;
        std::env::set_current_dir(&data_dir)?;
    }

    // Initialize tracing with the determined log level
    let log_level = get_log_level(&cli);

    // Resolve the base filter string: either from RUST_LOG or our computed level
    let base = if log_level.is_empty() {
        // RUST_LOG is set — use it as the base
        std::env::var("RUST_LOG").unwrap_or_else(|_| "debug".to_owned())
    } else {
        log_level.clone()
    };

    // Append noisy-module filters, unless:
    // - the base level is "trace" (user wants everything), or
    // - the user explicitly set a level for that module
    //   (e.g. RUST_LOG=debug,mainline::rpc=trace).
    let mut filter = base.clone();
    let skip_noisy = base.trim() == "trace";
    if !skip_noisy {
        for module_filter in NOISY_MODULE_FILTERS {
            // module_filter is "module::path=level"; extract the module prefix
            let module_prefix = module_filter.split('=').next().unwrap_or(module_filter);
            if !filter.contains(module_prefix) {
                filter.push(',');
                filter.push_str(module_filter);
            }
        }
    }
    // Structured-logging configuration (#1791): format, file
    // destination with its own level, and timestamp timezone all come
    // from the ID_LOG_* environment variables; unset means the
    // historical plain-text stderr stream.
    let log_config = id::structured_logging::LogConfig::resolve(
        std::env::var("ID_LOG_FORMAT").ok().as_deref(),
        std::env::var("ID_LOG_FILE").ok().as_deref(),
        std::env::var("ID_LOG_FILE_LEVEL").ok().as_deref(),
        std::env::var("ID_LOG_TZ").ok().as_deref(),
    );
    init_logging(&filter, &log_config);

    match cli.command {
        None => run_repl(None).await,
        Some(Command::Repl { node }) => run_repl(node).await,
        Some(Command::Serve(args)) => {
            let id::cli::ServeArgs {
                ephemeral,
                server_config,
                no_relay,
                no_gossip,
                web,
                port,
                bootstrap,
                topic,
                topic_secret,
                no_default_bootstrap,
                no_default_topic,
                replace_defaults,
                no_mdns,
                dht,
                iroh_port,
                bind,
                web_listen,
                web_token,
                web_cookie_secure,
                world,
                world_admin_token,
                world_module,
                world_mail_outbox,
                world_mail_command,
                world_mail_smtp,
                world_mail_from,
                world_name,
                world_ssh_port,
                world_checkpoint_every,
                world_max_open,
                world_max_sessions,
                world_idle_secs,
                world_boot_window_secs,
                world_caps,
                world_cap_policy,
                world_allowed_caps,
                roc_bin,
                roc_platform,
                world_native,
                world_source_only,
                world_module_trust_keys,
                world_runtime,
                allow_node,
                open_writes,
                ..
            } = args;
            cmd_serve(ServeOptions {
                ephemeral,
                server_config,
                groups_log: cli.groups_log,
                no_relay,
                no_gossip,
                web,
                port,
                bootstrap,
                topic,
                topic_secret,
                no_default_bootstrap,
                no_default_topic,
                replace_defaults,
                no_mdns,
                dht,
                iroh_port,
                bind,
                web_listen,
                bind_explicit,
                port_explicit,
                web_token,
                web_cookie_secure,
                world,
                world_admin_token,
                world_module,
                world_mail_outbox,
                world_mail_command,
                world_mail_smtp,
                world_mail_from,
                world_name,
                world_ssh_port,
                world_checkpoint_every,
                world_max_open,
                world_max_sessions,
                world_idle_secs,
                world_boot_window_secs,
                world_caps,
                world_cap_policy,
                world_allowed_caps,
                roc_bin,
                roc_platform,
                world_native,
                world_source_only,
                world_module_trust_keys,
                world_runtime,
                allow_node,
                open_writes,
            })
            .await
        }
        Some(Command::Id { client }) => cmd_id(client).await,
        Some(Command::Key { command }) => id::commands::cmd_key(command).await,
        Some(Command::Peers {
            gossip,
            rpc,
            depth,
            max_peers,
            timeout,
            bootstrap,
            topic,
            topic_secret,
            no_default_bootstrap,
            no_default_topic,
            replace_defaults,
            no_relay,
            no_mdns,
            node,
        }) => {
            let options = PeersOptions {
                gossip,
                rpc,
                depth,
                max_peers,
                timeout_secs: timeout,
                bootstrap,
                topic,
                topic_secret,
                no_default_bootstrap,
                no_default_topic,
                replace_defaults,
                no_relay,
                no_mdns,
            };
            cmd_peers(node, options).await
        }
        Some(Command::List { node, no_relay }) => cmd_list(node, no_relay).await,
        Some(Command::Tag(tag_cmd)) => cmd_tag(tag_cmd).await,
        Some(Command::Group { json, command }) => {
            id::commands::cmd_group(cli.groups_log, json, command).await
        }
        Some(Command::Delegate(args)) => id::commands::cmd_delegate(cli.groups_log, args).await,
        Some(Command::Revoke(args)) => id::commands::cmd_revoke(cli.groups_log, args).await,
        Some(Command::Doc { json, command }) => {
            id::commands::cmd_doc(cli.event_store, json, command).await
        }
        Some(Command::Claim { json, command }) => {
            id::commands::cmd_claim(cli.event_store, json, command).await
        }
        #[cfg(feature = "sandbox")]
        Some(Command::MatchAudit(command)) => id::commands::cmd_match_audit(command),
        #[cfg(feature = "world")]
        Some(Command::World(world_cmd)) => id::commands::cmd_world(*world_cmd).await,
        #[cfg(not(feature = "world"))]
        Some(Command::World(_)) => anyhow::bail!("this build has no world support"),
        Some(Command::MigrateTags) => cmd_migrate_tags().await,
        Some(Command::GetHash { hash, output }) => cmd_gethash(&hash, &output).await,
        Some(Command::Put {
            files,
            content,
            stdin,
            hash_only,
            no_relay,
        }) => cmd_put_multi(files, content, stdin, hash_only, no_relay).await,
        Some(Command::PutHash { source }) => cmd_put_hash(&source).await,
        Some(Command::Get {
            sources,
            stdin,
            hash,
            name_only,
            stdout,
            no_relay,
        }) => cmd_get_multi(sources, stdin, hash, name_only, stdout, no_relay).await,
        Some(Command::Cat {
            sources,
            stdin,
            hash,
            name_only,
            no_relay,
        }) => cmd_get_multi(sources, stdin, hash, name_only, true, no_relay).await,
        Some(Command::Find {
            queries,
            name,
            stdout,
            all,
            dir,
            format,
            first,
            last,
            count,
            exclude,
            node,
            no_relay,
        }) => {
            let options = SearchOptions::new(first, last, count, exclude);
            cmd_find(
                queries, name, stdout, all, dir, &format, options, node, no_relay,
            )
            .await
        }
        Some(Command::Search {
            queries,
            name,
            all,
            dir,
            format,
            first,
            last,
            count,
            exclude,
            node,
            no_relay,
        }) => {
            let options = SearchOptions::new(first, last, count, exclude);
            cmd_search(queries, name, all, dir, &format, options, node, no_relay).await
        }
        Some(Command::Show {
            queries,
            name,
            all,
            output,
            first,
            last,
            exclude,
            node,
            no_relay,
        }) => {
            let options = SearchOptions::new(first, last, false, exclude);
            cmd_show(queries, name, all, output, options, node, no_relay).await
        }
        Some(Command::Peek {
            queries,
            name,
            lines,
            head_only,
            tail_only,
            chars,
            words,
            quiet,
            output,
            all,
            first,
            last,
            exclude,
            node,
            no_relay,
        }) => {
            let search_opts = SearchOptions::new(first, last, false, exclude);
            let peek_opts = PeekOptions {
                lines,
                head_only,
                tail_only,
                chars,
                words,
                quiet,
            };
            cmd_peek(
                queries,
                name,
                all,
                output,
                peek_opts,
                search_opts,
                node,
                no_relay,
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_log_level;

    #[test]
    fn test_log_level_defaults_to_warn_without_flags_or_env() {
        assert_eq!(resolve_log_level(false, None, None, None, None), "warn");
    }

    #[test]
    fn test_log_level_debug_flag_wins_over_everything() {
        assert_eq!(
            resolve_log_level(true, Some("error"), Some("info"), Some("info"), None),
            "debug"
        );
    }

    #[test]
    fn test_log_level_flag_beats_env() {
        assert_eq!(
            resolve_log_level(false, Some("error"), Some("info"), Some("info"), Some("1")),
            "error"
        );
    }

    #[test]
    fn test_log_level_flag_empty_or_blank_is_unset() {
        // #5442: a blank flag must fall through like an empty RUST_LOG /
        // LOG_LEVEL does -- the empty string is the RUST_LOG sentinel, so
        // returning it here would silently log at debug (or RUST_LOG).
        assert_eq!(resolve_log_level(false, Some(""), None, None, None), "warn");
        assert_eq!(
            resolve_log_level(false, Some("  "), None, None, None),
            "warn"
        );
        assert_eq!(
            resolve_log_level(false, Some(""), None, Some("info"), None),
            "info"
        );
        assert_eq!(
            resolve_log_level(false, Some("  "), Some("info"), None, None),
            ""
        );
    }

    #[test]
    fn test_log_level_flag_and_env_values_are_trimmed() {
        assert_eq!(
            resolve_log_level(false, Some(" info "), None, None, None),
            "info"
        );
        assert_eq!(
            resolve_log_level(false, None, None, Some(" info "), None),
            "info"
        );
    }

    #[test]
    fn test_log_level_debug_env_padded_falsey_values_fall_through_to_warn() {
        for value in [" false ", "0 ", " FALSE", "  false"] {
            assert_eq!(
                resolve_log_level(false, None, None, None, Some(value)),
                "warn",
                "DEBUG={value:?}"
            );
        }
        assert_eq!(
            resolve_log_level(false, None, None, None, Some(" true ")),
            "debug"
        );
    }

    #[test]
    fn test_log_level_rust_log_presence_signals_env_filter() {
        // A non-empty RUST_LOG yields the empty sentinel; the caller then
        // uses the RUST_LOG value itself as the base filter.
        assert_eq!(
            resolve_log_level(false, None, Some("info"), Some("error"), Some("1")),
            ""
        );
        // Present-but-empty RUST_LOG is unconfigured, not a filter: it
        // falls through to the remaining sources and the default. (An
        // empty base filter silently muted every non-capped target in
        // environments that export RUST_LOG= , e.g. nix build sandboxes.)
        assert_eq!(resolve_log_level(false, None, Some(""), None, None), "warn");
        assert_eq!(
            resolve_log_level(false, None, Some("  "), None, None),
            "warn"
        );
        assert_eq!(
            resolve_log_level(false, None, Some(""), Some("info"), None),
            "info"
        );
        // Empty LOG_LEVEL is likewise unconfigured.
        assert_eq!(resolve_log_level(false, None, None, Some(""), None), "warn");
    }

    #[test]
    fn test_log_level_log_level_env_beats_debug_env() {
        assert_eq!(
            resolve_log_level(false, None, None, Some("info"), Some("1")),
            "info"
        );
    }

    #[test]
    fn test_log_level_debug_env_truthy_values() {
        for value in ["1", "true", "TRUE", "yes"] {
            assert_eq!(
                resolve_log_level(false, None, None, None, Some(value)),
                "debug",
                "DEBUG={value}"
            );
        }
    }

    #[test]
    fn test_log_level_debug_env_falsey_values_fall_through_to_warn() {
        for value in ["", "0", "false", "FALSE"] {
            assert_eq!(
                resolve_log_level(false, None, None, None, Some(value)),
                "warn",
                "DEBUG={value:?}"
            );
        }
    }
}
