//! CLI entry point for `id`, a peer-to-peer file sharing tool.
//!
//! This binary provides commands for storing, retrieving, and sharing content
//! using content-addressed storage backed by Iroh.

use anyhow::Result;
use clap::Parser;

// Import from library
use id::{
    Cli, Command, PeekOptions, PeersOptions, SearchOptions, cmd_find, cmd_get_multi, cmd_gethash,
    cmd_id, cmd_list, cmd_peek, cmd_peers, cmd_put_hash, cmd_put_multi, cmd_search, cmd_serve,
    cmd_show, run_repl,
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
    // 1. --debug flag takes highest precedence
    if cli.debug {
        return "debug".to_owned();
    }

    // 2. --log-level flag
    if let Some(ref level) = cli.log_level {
        return level.clone();
    }

    // 3. RUST_LOG env var - return empty to use EnvFilter's default behavior
    if std::env::var("RUST_LOG").is_ok() {
        return String::new();
    }

    // 4. LOG_LEVEL env var
    if let Ok(level) = std::env::var("LOG_LEVEL") {
        return level;
    }

    // 5. DEBUG env var (truthy check)
    if let Ok(debug) = std::env::var("DEBUG") {
        let is_truthy = !debug.is_empty() && debug != "0" && debug.to_lowercase() != "false";
        if is_truthy {
            return "debug".to_owned();
        }
    }

    // 6. Default to debug
    "debug".to_owned()
}

/// Per-module overrides to suppress noisy third-party modules.
/// These are appended to whatever base level is active, unless the user
/// explicitly sets a level for that module in `RUST_LOG`
/// (e.g. `RUST_LOG=debug,mainline::rpc=trace` keeps mainline::rpc at trace).
const NOISY_MODULE_FILTERS: &[&str] = &[
    "mainline::rpc=info",
    "distributed_topic_tracker::crypto::record=info",
    "rustls=info",
    "hickory_proto::error=info",
    "hickory_proto::udp::udp_client_stream=info",
    "swarm_discovery::receiver=info",
    "acto::tokio=info",
    "swarm_discovery::sender=info",
    "swarm_discovery::socket=info"
];

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI first so we can use flags for logging configuration
    let cli = Cli::parse();

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
        }) => {
            cmd_serve(
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
            )
            .await
        }
        Some(Command::Id) => cmd_id().await,
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
