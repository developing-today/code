//! `fleet secrets` — resolve secrets by name, as a verb
//! (step 6, developing-today/code#5554).
//!
//! The domain layer over the fleet-secrets platform: until this
//! step the resolver chain existed with no shipped consumer (the
//! bin's only reference was a type assertion). Two verbs:
//!
//!   * `get <name>` — the default chain: the secrets-broker framed
//!     exchange through the session proxy first, the `bws` CLI as
//!     fallback (fleet-secrets' `Resolver::default_chain`, reused
//!     as-is). The value goes to stdout and nowhere else — errors
//!     are the platform's value-free shapes, printed to stderr.
//!   * `list` — names only, from the broker's `GET /secrets`. A
//!     broker failure is loud (stderr + exit 3): a `bws` listing
//!     has different semantics (it enumerates the vault, not the
//!     broker's cache), so it is never silently substituted.
//!
//! Exit codes: 0 ok · 2 usage · 3 unresolved / broker unavailable
//! (3 matches the dispatch domain's unresolved-token exit, #5432).

use fleet_config::Config;
use fleet_secrets::{BrokerSource, Resolver};

const EXIT_OK: i32 = 0;
const EXIT_USAGE: i32 = 2;
const EXIT_UNRESOLVED: i32 = 3;

fn usage() -> i32 {
    eprintln!("usage: fleet secrets <command> [args...]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!(
        "  get <name>   Resolve a secret by name (broker first, Bitwarden fallback); value to stdout"
    );
    eprintln!("  list         List the secrets-broker's cached names");
    EXIT_USAGE
}

pub fn run(config: &Config, args: &[String]) -> i32 {
    match args.first().map(|s| s.as_str()) {
        Some("get") => cmd_get(config, &args[1..]),
        Some("list") => cmd_list(config, &args[1..]),
        _ => usage(),
    }
}

fn cmd_get(config: &Config, args: &[String]) -> i32 {
    // Exactly one name, and not a flag: a leading '-' would be a
    // caller bug (or a `--help` one position too deep), never a
    // secret name worth resolving.
    if args.len() != 1 || args[0].starts_with('-') {
        eprintln!("usage: fleet secrets get <name>");
        return EXIT_USAGE;
    }
    let name = &args[0];
    match Resolver::default_chain(config).resolve(name) {
        Ok(value) => {
            println!("{}", value.expose());
            EXIT_OK
        }
        Err(e) => {
            eprintln!("fleet secrets: {e}");
            EXIT_UNRESOLVED
        }
    }
}

fn cmd_list(config: &Config, args: &[String]) -> i32 {
    if !args.is_empty() {
        eprintln!("usage: fleet secrets list");
        return EXIT_USAGE;
    }
    let source = BrokerSource {
        broker_socket: config.broker_socket(),
        token_file: config.secrets_token_file(),
    };
    match source.list_names() {
        Ok(names) => {
            for name in names {
                println!("{name}");
            }
            EXIT_OK
        }
        Err(e) => {
            eprintln!("fleet secrets: {e}");
            EXIT_UNRESOLVED
        }
    }
}
