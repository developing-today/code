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

/// Write one line to a writer, classifying the outcome (#5569).
/// A broken pipe means the downstream consumer closed early
/// (`fleet secrets list | head`): that is a quiet, successful exit
/// for a Unix filter, not an error. Every other write error stays
/// loud. `println!` must not be used for streamed output: it panics
/// on EPIPE.
#[derive(Debug, PartialEq, Eq)]
enum LineWrite {
    Written,
    BrokenPipe,
    Failed(std::io::ErrorKind),
}

fn write_line(w: &mut impl std::io::Write, line: &str) -> LineWrite {
    match writeln!(w, "{line}") {
        Ok(()) => LineWrite::Written,
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => LineWrite::BrokenPipe,
        Err(e) => LineWrite::Failed(e.kind()),
    }
}

/// Print `text` (a single value) to stdout; broken pipe is a quiet
/// success (#5569), other write errors are loud.
fn print_value(text: &str) -> i32 {
    let mut out = std::io::stdout().lock();
    match write_line(&mut out, text) {
        LineWrite::Written | LineWrite::BrokenPipe => EXIT_OK,
        LineWrite::Failed(kind) => {
            eprintln!("fleet secrets: failed writing to stdout: {kind}");
            EXIT_UNRESOLVED
        }
    }
}

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
        Ok(value) => print_value(value.expose()),
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
    // #6218: on the laptop the broker's own socket answers
    // directly (no session broker exists there); elsewhere the
    // session-broker source reads the same route.
    let names_result = match fleet_secrets::DirectBrokerSource::from_config(config) {
        Some(direct) => direct.list_names(),
        None => BrokerSource {
            broker_socket: config.broker_socket(),
            token_file: config.secrets_token_file(),
        }
        .list_names(),
    };
    match names_result {
        Ok(names) => {
            let mut out = std::io::stdout().lock();
            for name in &names {
                match write_line(&mut out, name) {
                    LineWrite::Written => {}
                    // Downstream closed early (`| head`): quiet exit.
                    LineWrite::BrokenPipe => return EXIT_OK,
                    LineWrite::Failed(kind) => {
                        eprintln!("fleet secrets: failed writing to stdout: {kind}");
                        return EXIT_UNRESOLVED;
                    }
                }
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

    struct FailingWriter(std::io::ErrorKind);

    impl std::io::Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from(self.0))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn write_line_maps_broken_pipe_to_quiet_outcome() {
        let mut w = FailingWriter(std::io::ErrorKind::BrokenPipe);
        assert_eq!(write_line(&mut w, "name"), LineWrite::BrokenPipe);
    }

    #[test]
    fn write_line_keeps_other_errors_loud() {
        let mut w = FailingWriter(std::io::ErrorKind::StorageFull);
        assert_eq!(
            write_line(&mut w, "name"),
            LineWrite::Failed(std::io::ErrorKind::StorageFull)
        );
    }

    #[test]
    fn write_line_reports_success() {
        let mut w: Vec<u8> = Vec::new();
        assert_eq!(write_line(&mut w, "name"), LineWrite::Written);
        assert_eq!(String::from_utf8(w).unwrap(), "name\n");
    }

    #[test]
    fn non_help_arguments_ask_for_no_usage() {
        assert_eq!(leaf_help_usage("get", &args(&["some-name"])), None);
        assert_eq!(leaf_help_usage("get", &args(&[])), None);
        assert_eq!(leaf_help_usage("list", &args(&["stray"])), None);
        assert_eq!(leaf_help_usage("bogus", &args(&["--help"])), None);
    }
}
