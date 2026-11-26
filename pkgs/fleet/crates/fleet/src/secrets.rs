//! `fleet secrets` — resolve secrets by name, as a verb
//! (step 6, developing-today/code#5554).
//!
//! The domain layer over the fleet-secrets platform: until this
//! step the resolver chain existed with no shipped consumer (the
//! bin's only reference was a type assertion). Three verbs:
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
//!   * `set <name>` — write a secret (#15433, #10358 S4/D11): the
//!     value is read from stdin, never argv, the name is mapped by
//!     the profile's `Scope` exactly as `get` maps it, and the
//!     write goes through fleet-secrets' `BwsStore`. `--store` /
//!     `--vault` select the write store for this call; a
//!     `1password` write exits 3 until an `op` write path exists.
//!
//! Exit codes: 0 ok · 2 usage · 3 unresolved / broker unavailable
//! (3 matches the dispatch domain's unresolved-token exit, #5432).

use fleet_config::Config;
use fleet_secrets::{BrokerSource, BwsStore, Resolver, Scope, SecretError};

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
    eprintln!(
        "  set <name>   Store a secret in Bitwarden; the value is read from stdin, never argv"
    );
    EXIT_USAGE
}

pub fn run(config: &Config, args: &[String]) -> i32 {
    match args.first().map(|s| s.as_str()) {
        Some("get") => cmd_get(config, &args[1..]),
        Some("list") => cmd_list(config, &args[1..]),
        Some("set") => cmd_set(config, &args[1..]),
        _ => usage(),
    }
}

const GET_USAGE: &str = "\
Usage: fleet secrets get <name>

Resolve a secret by name: the secrets-broker first, Bitwarden (`bws`)
as fallback. The value goes to stdout and nowhere else.

With a prefix configured ($FLEET_SECRETS_PREFIX, else `[secrets] prefix`
in fleet.toml), a name reads `<prefix>.<name>`; a name that already
starts with `<prefix>.` is used as is.";

const LIST_USAGE: &str = "\
Usage: fleet secrets list

List the names in the secrets-broker's cache (names only, never values).

With a prefix configured ($FLEET_SECRETS_PREFIX, else `[secrets] prefix`
in fleet.toml), only names under `<prefix>.` are listed, shown without
it. A broker that does not support a scoped list is exit 3.";

const SET_USAGE: &str = "\
Usage: fleet secrets set <name> [--store <store>] [--vault <vault>]

Store a secret in Bitwarden through `bws`. The value is read from
stdin (a single trailing newline is stripped) and never taken from
argv, printed, or logged.

With a prefix configured ($FLEET_SECRETS_PREFIX, else `[secrets] prefix`
in fleet.toml), the name is written as `<prefix>.<name>`; a name that
already starts with `<prefix>.` is used as is — the same mapping `get`
applies, so a scoped `set` is read back by a scoped `get`.

--store is `bitwarden` (the default) or `1password`; --vault names
the Bitwarden project to write into, overriding `[secrets]
bws_project_id` / $FLEET_BWS_PROJECT_ID. The $FLEET_SECRETS_STORE and
$FLEET_SECRETS_VAULT variables are the flag fallbacks. Writing to
1password exits 3: no `op` write path exists yet.";

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
        "set" => Some(SET_USAGE),
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
    // #10358 S1: a configured prefix maps the name once, before any
    // source sees it; with none configured the scope is the identity.
    let scope = match Scope::from_config(config) {
        Ok(scope) => scope,
        Err(e) => {
            eprintln!("fleet secrets: secrets prefix: {e}");
            return EXIT_USAGE;
        }
    };
    match Resolver::default_chain(config).resolve_scoped(&scope, name) {
        Ok(value) => print_value(value.expose()),
        Err(e) => {
            eprintln!("fleet secrets: {e}");
            EXIT_UNRESOLVED
        }
    }
}

/// The parsed arguments of `fleet secrets set` (#15433). The value
/// is deliberately not a field: it arrives on stdin only, so it can
/// never be supplied (or leaked) through argv.
#[derive(Debug, PartialEq, Eq)]
struct SetArgs {
    name: String,
    store: Option<String>,
    vault: Option<String>,
}

/// Parse `set` arguments: exactly one positional name plus the
/// `--store` / `--vault` flags, in any order. Anything else — no
/// name, a second positional (the shape a value-on-argv attempt
/// takes), an unknown flag, a flag without its value — is a usage
/// error. An empty flag value counts as unset (#10358 D9).
fn parse_set_args(args: &[String]) -> Result<SetArgs, ()> {
    let mut parsed = SetArgs {
        name: String::new(),
        store: None,
        vault: None,
    };
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--store" | "--vault" => {
                let value = iter.next().ok_or(())?;
                if value.starts_with('-') {
                    return Err(());
                }
                let slot = if arg == "--store" {
                    &mut parsed.store
                } else {
                    &mut parsed.vault
                };
                *slot = if value.is_empty() {
                    None
                } else {
                    Some(value.clone())
                };
            }
            _ if arg.starts_with('-') => return Err(()),
            _ => {
                if !parsed.name.is_empty() {
                    return Err(());
                }
                parsed.name = arg.clone();
            }
        }
    }
    if parsed.name.is_empty() {
        return Err(());
    }
    Ok(parsed)
}

/// Which store a `set` writes to, decided before stdin is read or
/// any store is contacted. `Err` carries the exit code and a
/// value-free message: an unknown store is a usage error (exit 2,
/// #10358 §5), a `1password` write is a clear exit 3 until an `op`
/// write path exists (#15433), and `--vault` without a store is a
/// usage error (the D8 rule that a vault needs its store named).
#[derive(Debug, PartialEq, Eq)]
enum SetStore {
    /// Bitwarden through `BwsStore`; the vault, when present, is the
    /// project id the write goes into, overriding `bws_project_id`.
    Bitwarden { project: Option<String> },
}

fn set_store_choice(store: Option<&str>, vault: Option<&str>) -> Result<SetStore, (i32, String)> {
    match store {
        None => {
            if vault.is_some() {
                Err((
                    EXIT_USAGE,
                    "--vault needs --store: a vault is only meaningful for a named store"
                        .to_string(),
                ))
            } else {
                Ok(SetStore::Bitwarden { project: None })
            }
        }
        Some("bitwarden") => Ok(SetStore::Bitwarden {
            project: vault.map(str::to_string),
        }),
        Some("1password") => Err((
            EXIT_UNRESOLVED,
            "writing to 1password is not supported yet: no `op` write path exists".to_string(),
        )),
        Some(other) => Err((
            EXIT_USAGE,
            format!("unknown secrets store {other:?}: expected \"bitwarden\" or \"1password\""),
        )),
    }
}

/// The value `set` stores, from the raw stdin text: a single
/// trailing newline (and the `\r` of a CRLF pair) is the line
/// terminator a pipe or heredoc adds, not part of the secret.
/// Everything else is stored exactly as read.
fn stdin_value(raw: &str) -> &str {
    match raw.strip_suffix('\n') {
        Some(body) => body.strip_suffix('\r').unwrap_or(body),
        None => raw,
    }
}

/// A secret writer for `set`: the seam the §6 matrix tests drive
/// with a fake store. Production writes through [`BwsStore`].
trait SecretWriter {
    fn write_secret(&self, name: &str, value: &str) -> Result<(), SecretError>;
}

impl SecretWriter for BwsStore {
    fn write_secret(&self, name: &str, value: &str) -> Result<(), SecretError> {
        self.store(name, value)
    }
}

/// Store `value` under `name` mapped through `scope` (#10358 D11:
/// `set` is symmetric with `get`). The oracle is the design record's
/// §6 matrix: the writer is asked for exactly `scope.to_full(name)`,
/// a refused mapping never reaches the writer, and the exit codes
/// match `get` — 0 stored, 3 on a refused name or a failed write.
fn run_set_with_writer(writer: &dyn SecretWriter, scope: &Scope, name: &str, value: &str) -> i32 {
    let full = match scope.to_full(name) {
        Ok(full) => full,
        Err(e) => {
            eprintln!("fleet secrets: {e}");
            return EXIT_UNRESOLVED;
        }
    };
    match writer.write_secret(&full, value) {
        Ok(()) => EXIT_OK,
        Err(e) => {
            eprintln!("fleet secrets: {e}");
            EXIT_UNRESOLVED
        }
    }
}

fn cmd_set(config: &Config, args: &[String]) -> i32 {
    if let Some(usage_text) = leaf_help_usage("set", args) {
        println!("{usage_text}");
        return EXIT_OK;
    }
    let parsed = match parse_set_args(args) {
        Ok(parsed) => parsed,
        Err(()) => {
            eprintln!("usage: fleet secrets set <name> [--store <store>] [--vault <vault>]");
            return EXIT_USAGE;
        }
    };
    // Flag, then environment (#10358 D9 precedence, applied to the
    // set-only store/vault selection until S3 generalises it).
    let env_store = std::env::var("FLEET_SECRETS_STORE").ok();
    let env_vault = std::env::var("FLEET_SECRETS_VAULT").ok();
    let store = parsed
        .store
        .as_deref()
        .or_else(|| env_store.as_deref().filter(|v| !v.is_empty()));
    let vault = parsed
        .vault
        .as_deref()
        .or_else(|| env_vault.as_deref().filter(|v| !v.is_empty()));
    let SetStore::Bitwarden { project } = match set_store_choice(store, vault) {
        Ok(choice) => choice,
        Err((code, message)) => {
            eprintln!("fleet secrets: {message}");
            return code;
        }
    };
    let scope = match Scope::from_config(config) {
        Ok(scope) => scope,
        Err(e) => {
            eprintln!("fleet secrets: secrets prefix: {e}");
            return EXIT_USAGE;
        }
    };
    let writer = match project {
        Some(project) => BwsStore::new(project),
        None => match BwsStore::from_config(config) {
            Some(store) => store,
            None => {
                eprintln!(
                    "fleet secrets: no Bitwarden project is configured for writes: set \
                     [secrets] bws_project_id or $FLEET_BWS_PROJECT_ID, or pass \
                     --store bitwarden --vault <project-id>"
                );
                return EXIT_UNRESOLVED;
            }
        },
    };
    let mut raw = String::new();
    if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut raw) {
        eprintln!("fleet secrets: failed reading the value from stdin: {e}");
        return EXIT_UNRESOLVED;
    }
    run_set_with_writer(&writer, &scope, &parsed.name, stdin_value(&raw))
}

/// The names a scoped `list` prints (#10358 D5): only stored names
/// under the prefix, each shown as [`Scope::display`] gives it, in the
/// broker's order.
fn scoped_listing(scope: &Scope, stored: &[String]) -> Vec<String> {
    stored
        .iter()
        .filter_map(|name| scope.display(name))
        .collect()
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
    // #10358 S2: a configured prefix asks the broker for its names only
    // (`GET /secrets?prefix=P`) and filters again here, so output is
    // right against any broker; an unscoped list sends no query.
    let scope = match Scope::from_config(config) {
        Ok(scope) => scope,
        Err(e) => {
            eprintln!("fleet secrets: secrets prefix: {e}");
            return EXIT_USAGE;
        }
    };
    let names_result = match fleet_secrets::DirectBrokerSource::from_config(config) {
        Some(direct) => direct.list_names_in(&scope),
        None => BrokerSource::new(config.broker_socket(), config.secrets_token_file())
            .list_names_in(&scope),
    };
    match names_result {
        Ok(names) => {
            let mut out = std::io::stdout().lock();
            for name in &scoped_listing(&scope, &names) {
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
    fn scoped_list_shows_only_names_under_the_prefix() {
        // #10358 S2, design record §6.
        let stored = args(&["alpha", "42.alpha", "42.42.x", "420.y", "43.z"]);
        let scope = Scope::prefixed("42").unwrap();
        assert_eq!(scoped_listing(&scope, &stored), ["alpha", "42.42.x"]);
        // No prefix: every name, in the broker's order.
        assert_eq!(scoped_listing(&Scope::none(), &stored), stored);
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
    fn set_help_forms_name_the_set_usage() {
        for form in [
            args(&["--help"]),
            args(&["-h"]),
            args(&["some-name", "--help"]),
        ] {
            assert_eq!(leaf_help_usage("set", &form), Some(SET_USAGE));
        }
    }

    #[test]
    fn set_args_take_one_name_and_the_store_flags() {
        let plain = parse_set_args(&args(&["alpha"])).unwrap();
        assert_eq!(plain.name, "alpha");
        assert_eq!(plain.store, None);
        assert_eq!(plain.vault, None);
        let flagged = parse_set_args(&args(&[
            "--store",
            "bitwarden",
            "alpha",
            "--vault",
            "proj-1",
        ]))
        .unwrap();
        assert_eq!(flagged.name, "alpha");
        assert_eq!(flagged.store.as_deref(), Some("bitwarden"));
        assert_eq!(flagged.vault.as_deref(), Some("proj-1"));
        // An empty flag value counts as unset (#10358 D9).
        let empty = parse_set_args(&args(&["--store", "", "alpha"])).unwrap();
        assert_eq!(empty.store, None);
    }

    #[test]
    fn set_args_refuse_usage_errors() {
        // No name; a second positional — the shape of a value passed
        // on argv, which `set` never accepts; an unknown flag; a
        // flag without its value; a flag value that is itself a flag.
        for form in [
            args(&[]),
            args(&["alpha", "the-value-on-argv"]),
            args(&["--bogus", "alpha"]),
            args(&["alpha", "--store"]),
            args(&["alpha", "--store", "--vault"]),
            args(&["-x"]),
        ] {
            assert!(parse_set_args(&form).is_err(), "{form:?}");
        }
    }

    #[test]
    fn set_store_choice_covers_the_store_matrix() {
        // Unconfigured: Bitwarden with the configured project.
        assert_eq!(
            set_store_choice(None, None).unwrap(),
            SetStore::Bitwarden { project: None }
        );
        // bitwarden + vault: the vault is the write project (D8).
        assert_eq!(
            set_store_choice(Some("bitwarden"), Some("proj-9")).unwrap(),
            SetStore::Bitwarden {
                project: Some("proj-9".to_string())
            }
        );
        // 1password writes exit 3 with a clear message (#15433).
        let (code, message) = set_store_choice(Some("1password"), Some("vault")).unwrap_err();
        assert_eq!(code, EXIT_UNRESOLVED);
        assert!(message.contains("1password"), "{message}");
        // Unknown store and vault-without-store are usage errors.
        assert_eq!(
            set_store_choice(Some("vaultwarden"), None).unwrap_err().0,
            EXIT_USAGE
        );
        assert_eq!(
            set_store_choice(None, Some("proj-9")).unwrap_err().0,
            EXIT_USAGE
        );
    }

    #[test]
    fn stdin_value_strips_one_trailing_newline_only() {
        assert_eq!(stdin_value("value\n"), "value");
        assert_eq!(stdin_value("value\r\n"), "value");
        assert_eq!(stdin_value("value"), "value");
        assert_eq!(stdin_value("value\n\n"), "value\n");
        assert_eq!(stdin_value("va\nlue\n"), "va\nlue");
        assert_eq!(stdin_value(""), "");
    }

    /// A fake store for the `set` matrix: records the names it was
    /// asked to write, never a value.
    struct RecordingStore {
        names: std::cell::RefCell<Vec<String>>,
        fail: bool,
    }

    impl RecordingStore {
        fn new(fail: bool) -> RecordingStore {
            RecordingStore {
                names: std::cell::RefCell::new(Vec::new()),
                fail,
            }
        }
    }

    impl SecretWriter for RecordingStore {
        fn write_secret(&self, name: &str, _value: &str) -> Result<(), SecretError> {
            self.names.borrow_mut().push(name.to_string());
            if self.fail {
                return Err(SecretError::Unavailable {
                    source: "fake",
                    name: name.to_string(),
                    detail: "fake store failure".to_string(),
                });
            }
            Ok(())
        }
    }

    #[test]
    fn set_runs_the_design_matrix_through_a_fake_store() {
        // #10358 design record §6, the `set` half of the nine rows:
        // (caller name, stored name the fake store must see).
        let rows = [
            ("alpha", Some("42.alpha")),
            ("42.alpha", Some("42.alpha")),
            ("42.x", Some("42.x")),
            ("42.42.x", Some("42.42.x")),
            ("42", Some("42.42")),
            ("42.", None),
            ("420.x", Some("42.420.x")),
        ];
        let scope = Scope::prefixed("42").unwrap();
        for (caller, stored) in rows {
            let fake = RecordingStore::new(false);
            let code = run_set_with_writer(&fake, &scope, caller, "fixture-value");
            match stored {
                Some(expected) => {
                    assert_eq!(code, EXIT_OK, "{caller}");
                    assert_eq!(
                        fake.names.borrow().as_slice(),
                        [expected.to_string()],
                        "{caller}"
                    );
                }
                None => {
                    // Row 8: refused before the store is contacted.
                    assert_eq!(code, EXIT_UNRESOLVED, "{caller}");
                    assert!(fake.names.borrow().is_empty(), "{caller}");
                }
            }
        }
    }

    #[test]
    fn set_with_no_config_writes_the_name_unchanged() {
        // The no-config oracle (#15433): Scope::none() is the
        // identity, so every §6 row is written exactly as named.
        let fake = RecordingStore::new(false);
        for caller in ["alpha", "42.alpha", "42.42.x", "420.x"] {
            assert_eq!(
                run_set_with_writer(&fake, &Scope::none(), caller, "fixture-value"),
                EXIT_OK,
                "{caller}"
            );
        }
        assert_eq!(
            fake.names.borrow().join(","),
            "alpha,42.alpha,42.42.x,420.x"
        );
    }

    #[test]
    fn set_store_failure_is_exit_3() {
        let fake = RecordingStore::new(true);
        assert_eq!(
            run_set_with_writer(&fake, &Scope::none(), "alpha", "fixture-value"),
            EXIT_UNRESOLVED
        );
    }

    #[test]
    fn non_help_arguments_ask_for_no_usage() {
        assert_eq!(leaf_help_usage("get", &args(&["some-name"])), None);
        assert_eq!(leaf_help_usage("get", &args(&[])), None);
        assert_eq!(leaf_help_usage("list", &args(&["stray"])), None);
        assert_eq!(leaf_help_usage("bogus", &args(&["--help"])), None);
    }
}
