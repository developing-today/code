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
/// 3. `RUST_LOG` env var → return empty string to signal use of `EnvFilter` default
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
/// mutating process environment in parallel. `Some("")` for `rust_log`
/// means the variable is present, matching the previous `var().is_ok()`
/// presence check; the empty-string result is the caller's signal to use
/// the `RUST_LOG` value itself as the base filter.
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

    // 2. --log-level flag
    if let Some(level) = log_level_flag {
        return level.to_owned();
    }

    // 3. RUST_LOG env var - return empty to use EnvFilter's default behavior
    if rust_log.is_some() {
        return String::new();
    }

    // 4. LOG_LEVEL env var
    if let Some(level) = log_level_env {
        return level.to_owned();
    }

    // 5. DEBUG env var (truthy check)
    if let Some(debug) = debug_env {
        let is_truthy = !debug.is_empty() && debug != "0" && debug.to_lowercase() != "false";
        if is_truthy {
            return "debug".to_owned();
        }
    }

    // 6. Default to warn (as documented; --debug is the opt-in for debug)
    "warn".to_owned()
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
                    serve.value_source("bind")
                        == Some(clap::parser::ValueSource::CommandLine),
                    serve.value_source("port")
                        == Some(clap::parser::ValueSource::CommandLine),
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
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::new(&filter))
        .init();

    match cli.command {
        None => run_repl(None).await,
        Some(Command::Repl { node }) => run_repl(node).await,
        Some(Command::Serve {
            ephemeral,
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
        }) => {
            cmd_serve(ServeOptions {
                ephemeral,
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
    fn test_log_level_rust_log_presence_signals_env_filter() {
        // Present (even empty) RUST_LOG yields the empty sentinel; the
        // caller then uses the RUST_LOG value itself as the base filter.
        assert_eq!(
            resolve_log_level(false, None, Some("info"), Some("error"), Some("1")),
            ""
        );
        assert_eq!(resolve_log_level(false, None, Some(""), None, None), "");
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
