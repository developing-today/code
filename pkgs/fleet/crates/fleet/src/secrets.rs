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

const GET_USAGE: &str = "\
Usage: fleet secrets get <name>

Resolve a secret by name: the secrets-broker first, Bitwarden (`bws`)
as fallback. The value goes to stdout and nowhere else.";

const LIST_USAGE: &str = "\
Usage: fleet secrets list

List the names in the secrets-broker's cache (names only, never values).";

/// The usage a `--help`/`-h` flag anywhere in a leaf verb's arguments
/// asks for, if any (#5627). Every other fleet leaf verb answers a help
/// flag with its usage on stdout at exit 0 (dispatch, search, proxy —
/// #5560, #5619); the help check runs before arity and before any
/// resolver or broker contact, so a help request is never mistaken for
/// a secret name or a listing attempt. Non-help stray arguments keep
/// the usage-error path in the verbs below.
fn leaf_help_usage(verb: &str, args: &[String]) -> Option<&'static str> {
    if !args.iter().any(|a| a == "--help" || a == "-h") {
        return None;
    }
    match verb {
        "get" => Some(GET_USAGE),
        "list" => Some(LIST_USAGE),
        _ => None,
    }
}

fn cmd_get(config: &Config, args: &[String]) -> i32 {
    if let Some(usage_text) = leaf_help_usage("get", args) {
        println!("{usage_text}");
        return EXIT_OK;
    }
    // Exactly one name, and not a flag: a leading '-' would be a
    // caller bug (help flags are answered above), never a secret
    // name worth resolving.
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
    if let Some(usage_text) = leaf_help_usage("list", args) {
        println!("{usage_text}");
        return EXIT_OK;
    }
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    #[test]
    fn get_help_forms_name_the_get_usage() {
        for form in [
            args(&["--help"]),
            args(&["-h"]),
            args(&["some-name", "--help"]),
            args(&["--help", "some-name"]),
        ] {
            assert_eq!(leaf_help_usage("get", &form), Some(GET_USAGE));
        }
    }

    #[test]
    fn list_help_forms_name_the_list_usage() {
        for form in [args(&["--help"]), args(&["-h"]), args(&["--help", "x"])] {
            assert_eq!(leaf_help_usage("list", &form), Some(LIST_USAGE));
        }
    }

    #[test]
    fn non_help_arguments_ask_for_no_usage() {
        assert_eq!(leaf_help_usage("get", &args(&["some-name"])), None);
        assert_eq!(leaf_help_usage("get", &args(&[])), None);
        assert_eq!(leaf_help_usage("list", &args(&["stray"])), None);
        assert_eq!(leaf_help_usage("bogus", &args(&["--help"])), None);
    }
}
