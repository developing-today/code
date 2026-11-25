//! Argument parsing and dispatch for `fleet kv`.
//!
//! Parsing is strict and finishes before any store access: an unknown flag, a
//! missing `--namespace` or a malformed value is a usage error (exit 2) and
//! touches nothing. Typed store refusals print `error: <code>: <message>` and
//! exit 1. `--global` is refused as `unimplemented_central_in_phase1`
//! (exit 69) because the central plane is not built yet.

use std::io::{Read, Write};

use crate::{Identity, KvError, LocalStore};

pub const EXIT_OK: i32 = 0;
pub const EXIT_REFUSED: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_UNAVAILABLE: i32 = 69;

pub const USAGE: &str = "\
usage: fleet kv <verb> --namespace NS [args]

Identity-scoped key/value bytes in this host's local store.

verbs:
  get            --namespace NS KEY            print the value's bytes
  set            --namespace NS KEY VALUE      store VALUE (or --stdin)
  list           --namespace NS                list keys (--include-tombstones)
  delete         --namespace NS KEY            write a masking tombstone
  clear-override --namespace NS KEY            drop the local overlay row
  revert         --namespace NS KEY --to V     append a copy of version V

Keys are scoped to this session's launch identity (owner, context and
session). With no trusted launch-identity record every verb but help
refuses with identity_unavailable (exit 69).

--global (central store) and --force-blob (blob placement) belong to the
central plane, which is not built yet: both are refused with
unimplemented_central_in_phase1 (exit 69).

Exit codes: 0 ok, 1 typed refusal (malformed_key, not_found, ...),
2 usage error, 69 unavailable.
";

const GET_USAGE: &str = "usage: fleet kv get --namespace NS KEY\n\nPrint the value's bytes exactly. A missing or masked key exits 1 with not_found.\n";
const SET_USAGE: &str = "usage: fleet kv set --namespace NS KEY (VALUE | --stdin)\n\nStore bytes as a new local version and queue it for the sweep. Prints the version.\n";
const LIST_USAGE: &str = "usage: fleet kv list --namespace NS [--include-tombstones]\n\nOne line per key: KEY<TAB>VERSION<TAB>ORIGIN (local or masked).\n";
const DELETE_USAGE: &str = "usage: fleet kv delete --namespace NS KEY\n\nWrite a masking tombstone: the key reads absent here and sweeps as a mask.\n";
const CLEAR_USAGE: &str = "usage: fleet kv clear-override --namespace NS KEY\n\nDrop the local overlay row, advance the key's fence and cancel unsent sweep\nintents. History is kept. Busy for over 5 s: clear_busy_try_again.\n";
const REVERT_USAGE: &str = "usage: fleet kv revert --namespace NS KEY --to VERSION\n       fleet kv revert NS KEY --to VERSION\n\nAppend a new version equal to VERSION from this key's local history.\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verb {
    Get,
    Set,
    List,
    Delete,
    ClearOverride,
    Revert,
}

impl Verb {
    fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "get" => Self::Get,
            "set" => Self::Set,
            "list" => Self::List,
            "delete" => Self::Delete,
            "clear-override" => Self::ClearOverride,
            "revert" => Self::Revert,
            _ => return None,
        })
    }

    fn usage(self) -> &'static str {
        match self {
            Self::Get => GET_USAGE,
            Self::Set => SET_USAGE,
            Self::List => LIST_USAGE,
            Self::Delete => DELETE_USAGE,
            Self::ClearOverride => CLEAR_USAGE,
            Self::Revert => REVERT_USAGE,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ValueSource {
    Inline(String),
    Stdin,
}

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Help(&'static str),
    Get {
        namespace: String,
        key: String,
    },
    Set {
        namespace: String,
        key: String,
        value: ValueSource,
    },
    List {
        namespace: String,
        include_tombstones: bool,
    },
    Delete {
        namespace: String,
        key: String,
    },
    ClearOverride {
        namespace: String,
        key: String,
    },
    Revert {
        namespace: String,
        key: String,
        to: u64,
    },
}

#[derive(Debug, PartialEq, Eq)]
enum ParseError {
    Usage(String),
    Central(&'static str),
}

fn usage(message: impl Into<String>) -> ParseError {
    ParseError::Usage(message.into())
}

fn parse(args: &[String]) -> Result<Command, ParseError> {
    let Some(first) = args.first() else {
        return Ok(Command::Help(USAGE));
    };
    if matches!(first.as_str(), "-h" | "--help" | "help") {
        return Ok(Command::Help(USAGE));
    }
    for flag in ["--global", "--force-blob"] {
        if args.iter().any(|arg| arg == flag) {
            return Err(ParseError::Central(flag));
        }
    }
    let verb = Verb::parse(first).ok_or_else(|| usage(format!("unknown verb `{first}`")))?;

    let mut namespace: Option<String> = None;
    let mut include_tombstones = false;
    let mut stdin = false;
    let mut to: Option<u64> = None;
    let mut positionals: Vec<String> = Vec::new();
    let mut rest = args[1..].iter();
    let mut only_positionals = false;
    while let Some(arg) = rest.next() {
        if only_positionals || !arg.starts_with("--") || arg == "-" {
            positionals.push(arg.clone());
            continue;
        }
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) => (flag, Some(value.to_string())),
            None => (arg.as_str(), None),
        };
        let mut take_value = |name: &str| -> Result<String, ParseError> {
            match inline.clone() {
                Some(value) => Ok(value),
                None => rest
                    .next()
                    .cloned()
                    .ok_or_else(|| usage(format!("{name} needs a value"))),
            }
        };
        match flag {
            "--" => only_positionals = true,
            "--help" | "-h" => return Ok(Command::Help(verb.usage())),
            "--namespace" => {
                if namespace.is_some() {
                    return Err(usage("--namespace given twice"));
                }
                namespace = Some(take_value("--namespace")?);
            }
            "--include-tombstones" if verb == Verb::List && inline.is_none() => {
                include_tombstones = true;
            }
            "--stdin" if verb == Verb::Set && inline.is_none() => stdin = true,
            "--to" if verb == Verb::Revert => {
                let raw = take_value("--to")?;
                let version = raw
                    .parse::<u64>()
                    .ok()
                    .filter(|version| *version > 0)
                    .ok_or_else(|| usage(format!("--to needs a version number, got `{raw}`")))?;
                to = Some(version);
            }
            _ => return Err(usage(format!("unknown argument `{arg}` for `kv {first}`"))),
        }
    }

    if verb == Verb::Revert && namespace.is_none() && positionals.len() == 2 {
        namespace = Some(positionals.remove(0));
    }
    let namespace = namespace.ok_or_else(|| usage("--namespace NS is required"))?;
    let expect = |count: usize| -> Result<(), ParseError> {
        if positionals.len() == count {
            Ok(())
        } else {
            Err(usage(format!(
                "`kv {first}` takes {count} argument(s), got {}",
                positionals.len()
            )))
        }
    };
    Ok(match verb {
        Verb::Get => {
            expect(1)?;
            Command::Get {
                namespace,
                key: positionals.remove(0),
            }
        }
        Verb::Set => {
            if stdin {
                expect(1)?;
                Command::Set {
                    namespace,
                    key: positionals.remove(0),
                    value: ValueSource::Stdin,
                }
            } else {
                expect(2)?;
                let value = positionals.remove(1);
                Command::Set {
                    namespace,
                    key: positionals.remove(0),
                    value: ValueSource::Inline(value),
                }
            }
        }
        Verb::List => {
            expect(0)?;
            Command::List {
                namespace,
                include_tombstones,
            }
        }
        Verb::Delete => {
            expect(1)?;
            Command::Delete {
                namespace,
                key: positionals.remove(0),
            }
        }
        Verb::ClearOverride => {
            expect(1)?;
            Command::ClearOverride {
                namespace,
                key: positionals.remove(0),
            }
        }
        Verb::Revert => {
            expect(1)?;
            let to = to.ok_or_else(|| usage("--to VERSION is required"))?;
            Command::Revert {
                namespace,
                key: positionals.remove(0),
                to,
            }
        }
    })
}

fn refuse(err: &mut dyn Write, error: &KvError) -> i32 {
    let _ = writeln!(err, "error: {}: {error}", error.code());
    EXIT_REFUSED
}

fn report_parse_error(error: ParseError, err: &mut dyn Write) -> i32 {
    match error {
        ParseError::Central(flag) => {
            let _ = writeln!(
                err,
                "error: unimplemented_central_in_phase1: {flag} needs the central store, which is not built yet"
            );
            EXIT_UNAVAILABLE
        }
        ParseError::Usage(message) => {
            let _ = writeln!(err, "error: usage: {message}\n\n{USAGE}");
            EXIT_USAGE
        }
    }
}

/// What opening the store yields: the store and the caller's identity, or a
/// typed refusal code and message.
pub type Opened = Result<(LocalStore, crate::launch::LaunchIdentity), (&'static str, String)>;

/// Validate a whole command line without touching anything. Returns the
/// exit code when the invocation is already answered (help, a usage error
/// or the `--global` refusal) and `None` when it needs the store.
pub fn preflight(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> Option<i32> {
    match parse(args) {
        Ok(Command::Help(text)) => Some(match out.write_all(text.as_bytes()) {
            Ok(()) => EXIT_OK,
            Err(_) => EXIT_REFUSED,
        }),
        Ok(_) => None,
        Err(error) => Some(report_parse_error(error, err)),
    }
}

/// Run one invocation, opening the store and identity only once the whole
/// command line has parsed. An open failure is reported as
/// `error: <code>: <message>` and exits 69.
pub fn run_opening(
    args: &[String],
    open: impl FnOnce() -> Opened,
    stdin: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let command = match parse(args) {
        Ok(command) => command,
        Err(error) => return report_parse_error(error, err),
    };
    if let Command::Help(text) = command {
        return finish(out.write_all(text.as_bytes()), err);
    }
    let (store, launch) = match open() {
        Ok(opened) => opened,
        Err((code, message)) => {
            let _ = writeln!(err, "error: {code}: {message}");
            return EXIT_UNAVAILABLE;
        }
    };
    execute(command, &store, &launch.identity(), stdin, out, err)
}

/// Run one `fleet kv` invocation against `store` as `identity`.
pub fn run(
    args: &[String],
    store: &LocalStore,
    identity: &Identity<'_>,
    stdin: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    match parse(args) {
        Ok(command) => execute(command, store, identity, stdin, out, err),
        Err(error) => report_parse_error(error, err),
    }
}

fn finish(written: std::io::Result<()>, err: &mut dyn Write) -> i32 {
    match written {
        Ok(()) => EXIT_OK,
        Err(error) => {
            let _ = writeln!(err, "error: output_failed: {error}");
            EXIT_REFUSED
        }
    }
}

fn execute(
    command: Command,
    store: &LocalStore,
    identity: &Identity<'_>,
    stdin: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let written = match command {
        Command::Help(text) => out.write_all(text.as_bytes()),
        Command::Get { namespace, key } => match store.get(identity, &namespace, &key) {
            Ok(Some(value)) => out.write_all(&value),
            Ok(None) => {
                let _ = writeln!(
                    err,
                    "error: not_found: no live value for `{key}` in `{namespace}`"
                );
                return EXIT_REFUSED;
            }
            Err(error) => return refuse(err, &error),
        },
        Command::Set {
            namespace,
            key,
            value,
        } => {
            let bytes = match value {
                ValueSource::Inline(value) => value.into_bytes(),
                ValueSource::Stdin => {
                    let mut bytes = Vec::new();
                    if let Err(error) = stdin.read_to_end(&mut bytes) {
                        let _ = writeln!(err, "error: stdin_unreadable: {error}");
                        return EXIT_REFUSED;
                    }
                    bytes
                }
            };
            match store.set(identity, &namespace, &key, &bytes) {
                Ok(version) => writeln!(out, "{version}"),
                Err(error) => return refuse(err, &error),
            }
        }
        Command::List {
            namespace,
            include_tombstones,
        } => match store.list(identity, &namespace, include_tombstones) {
            Ok(entries) => entries.iter().try_for_each(|entry| {
                writeln!(
                    out,
                    "{}\t{}\t{}",
                    entry.key,
                    entry.version,
                    entry.origin.as_str()
                )
            }),
            Err(error) => return refuse(err, &error),
        },
        Command::Delete { namespace, key } => match store.delete(identity, &namespace, &key) {
            Ok(version) => writeln!(out, "{version}"),
            Err(error) => return refuse(err, &error),
        },
        Command::ClearOverride { namespace, key } => {
            match store.clear_override(identity, &namespace, &key) {
                Ok(true) => writeln!(out, "cleared"),
                Ok(false) => writeln!(out, "no-override"),
                Err(error) => return refuse(err, &error),
            }
        }
        Command::Revert { namespace, key, to } => {
            match store.revert(identity, &namespace, &key, to) {
                Ok(version) => writeln!(out, "{version}"),
                Err(error) => return refuse(err, &error),
            }
        }
    };
    finish(written, err)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_string()).collect()
    }

    struct Outcome {
        code: i32,
        out: Vec<u8>,
        err: String,
    }

    fn invoke(store: &LocalStore, words: &[&str], stdin: &[u8]) -> Outcome {
        let identity = Identity::new("owner", "context", "partition");
        let mut input = stdin;
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(
            &args(words),
            store,
            &identity,
            &mut input,
            &mut out,
            &mut err,
        );
        Outcome {
            code,
            out,
            err: String::from_utf8(err).expect("utf8 stderr"),
        }
    }

    fn text(outcome: &Outcome) -> String {
        String::from_utf8(outcome.out.clone()).expect("utf8 stdout")
    }

    #[test]
    fn set_get_list_delete_round_trip() {
        let store = LocalStore::in_memory().expect("store");
        let set = invoke(
            &store,
            &["set", "--namespace", "notes", "a/b", "hello"],
            b"",
        );
        assert_eq!(set.code, EXIT_OK, "{}", set.err);
        assert_eq!(text(&set), "1\n");

        let get = invoke(&store, &["get", "--namespace=notes", "a/b"], b"");
        assert_eq!(get.code, EXIT_OK);
        assert_eq!(get.out, b"hello");

        let list = invoke(&store, &["list", "--namespace", "notes"], b"");
        assert_eq!(text(&list), "a/b\t1\tlocal\n");

        let delete = invoke(&store, &["delete", "--namespace", "notes", "a/b"], b"");
        assert_eq!(text(&delete), "2\n");
        let gone = invoke(&store, &["get", "--namespace", "notes", "a/b"], b"");
        assert_eq!(gone.code, EXIT_REFUSED);
        assert!(gone.err.starts_with("error: not_found"), "{}", gone.err);
        assert_eq!(
            text(&invoke(&store, &["list", "--namespace", "notes"], b"")),
            ""
        );
        assert_eq!(
            text(&invoke(
                &store,
                &["list", "--namespace", "notes", "--include-tombstones"],
                b""
            )),
            "a/b\t2\tmasked\n"
        );
    }

    #[test]
    fn set_reads_binary_bytes_from_stdin() {
        let store = LocalStore::in_memory().expect("store");
        let bytes = [0u8, 255, 10, 13, 0];
        let set = invoke(
            &store,
            &["set", "--namespace", "blobs", "--stdin", "raw"],
            &bytes,
        );
        assert_eq!(set.code, EXIT_OK, "{}", set.err);
        let get = invoke(&store, &["get", "--namespace", "blobs", "raw"], b"");
        assert_eq!(get.out, bytes);
    }

    #[test]
    fn a_100_kib_value_succeeds() {
        let store = LocalStore::in_memory().expect("store");
        let value: Vec<u8> = (0..100 * 1024).map(|i| (i % 251) as u8).collect();
        let set = invoke(
            &store,
            &["set", "--namespace", "big", "--stdin", "v"],
            &value,
        );
        assert_eq!(set.code, EXIT_OK, "{}", set.err);
        assert_eq!(
            invoke(&store, &["get", "--namespace", "big", "v"], b"").out,
            value
        );
    }

    #[test]
    fn global_is_refused_before_any_store_access() {
        let store = LocalStore::in_memory().expect("store");
        for words in [
            &["get", "--global", "--namespace", "notes", "k"][..],
            &["set", "--namespace", "notes", "--global", "k", "v"][..],
            &["--global", "list", "--namespace", "notes"][..],
            &["set", "--namespace", "notes", "--force-blob", "k", "v"][..],
        ] {
            let outcome = invoke(&store, words, b"");
            assert_eq!(outcome.code, EXIT_UNAVAILABLE);
            assert!(
                outcome
                    .err
                    .starts_with("error: unimplemented_central_in_phase1"),
                "{}",
                outcome.err
            );
        }
        let list = invoke(&store, &["list", "--namespace", "notes"], b"");
        assert_eq!(text(&list), "");
    }

    #[test]
    fn strict_parsing_refuses_before_writing() {
        let store = LocalStore::in_memory().expect("store");
        for words in [
            &["set", "notes", "k", "v"][..],
            &["set", "--namespace", "notes", "k"][..],
            &["set", "--namespace", "notes", "k", "v", "extra"][..],
            &[
                "set",
                "--namespace",
                "notes",
                "--include-tombstones",
                "k",
                "v",
            ][..],
            &["get", "--namespace", "notes", "--stdin", "k"][..],
            &["list", "--namespace", "notes", "--namespace", "other"][..],
            &["frobnicate", "--namespace", "notes"][..],
            &["revert", "--namespace", "notes", "k"][..],
            &["revert", "--namespace", "notes", "k", "--to", "0"][..],
            &["revert", "--namespace", "notes", "k", "--to", "x"][..],
            &["get", "--namespace"][..],
        ] {
            let outcome = invoke(&store, words, b"");
            assert_eq!(outcome.code, EXIT_USAGE, "{words:?}: {}", outcome.err);
            assert!(outcome.err.starts_with("error: usage:"), "{}", outcome.err);
        }
        assert_eq!(
            text(&invoke(&store, &["list", "--namespace", "notes"], b"")),
            ""
        );
    }

    #[test]
    fn malformed_key_and_namespace_are_typed_refusals() {
        let store = LocalStore::in_memory().expect("store");
        let key = invoke(&store, &["set", "--namespace", "notes", "a//b", "v"], b"");
        assert_eq!(key.code, EXIT_REFUSED);
        assert!(key.err.starts_with("error: malformed_key:"), "{}", key.err);
        let namespace = invoke(&store, &["set", "--namespace", "0bad", "k", "v"], b"");
        assert_eq!(namespace.code, EXIT_REFUSED);
        assert!(
            namespace.err.starts_with("error: malformed_namespace:"),
            "{}",
            namespace.err
        );
    }

    #[test]
    fn revert_accepts_positional_namespace_and_appends() {
        let store = LocalStore::in_memory().expect("store");
        invoke(&store, &["set", "--namespace", "notes", "k", "one"], b"");
        invoke(&store, &["set", "--namespace", "notes", "k", "two"], b"");
        let revert = invoke(&store, &["revert", "notes", "k", "--to", "1"], b"");
        assert_eq!(revert.code, EXIT_OK, "{}", revert.err);
        assert_eq!(text(&revert), "3\n");
        assert_eq!(
            invoke(&store, &["get", "--namespace", "notes", "k"], b"").out,
            b"one"
        );
        let unknown = invoke(
            &store,
            &["revert", "--namespace", "notes", "k", "--to=9"],
            b"",
        );
        assert_eq!(unknown.code, EXIT_REFUSED);
        assert!(
            unknown.err.starts_with("error: unknown_version:"),
            "{}",
            unknown.err
        );
    }

    #[test]
    fn clear_override_reports_whether_a_row_existed() {
        let store = LocalStore::in_memory().expect("store");
        invoke(&store, &["set", "--namespace", "notes", "k", "v"], b"");
        let cleared = invoke(
            &store,
            &["clear-override", "--namespace", "notes", "k"],
            b"",
        );
        assert_eq!(text(&cleared), "cleared\n");
        let again = invoke(
            &store,
            &["clear-override", "--namespace", "notes", "k"],
            b"",
        );
        assert_eq!(text(&again), "no-override\n");
    }

    #[test]
    fn help_prints_without_a_namespace() {
        let store = LocalStore::in_memory().expect("store");
        assert!(text(&invoke(&store, &[], b"")).contains("usage: fleet kv"));
        assert!(text(&invoke(&store, &["revert", "--help"], b"")).contains("--to VERSION"));
    }

    #[test]
    fn preflight_answers_help_usage_and_global_without_the_store() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        assert_eq!(
            preflight(&args(&["--help"]), &mut out, &mut err),
            Some(EXIT_OK)
        );
        assert_eq!(
            preflight(&args(&["get", "k"]), &mut out, &mut err),
            Some(EXIT_USAGE)
        );
        assert_eq!(
            preflight(
                &args(&["list", "--global", "--namespace", "n"]),
                &mut out,
                &mut err
            ),
            Some(EXIT_UNAVAILABLE)
        );
        assert_eq!(
            preflight(&args(&["list", "--namespace", "n"]), &mut out, &mut err),
            None
        );
    }

    #[test]
    fn run_opening_never_opens_for_bad_arguments_and_fails_closed() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut input: &[u8] = b"";
        let code = run_opening(
            &args(&["get", "--namespace", "n", "k", "extra"]),
            || panic!("must not open on a usage error"),
            &mut input,
            &mut out,
            &mut err,
        );
        assert_eq!(code, EXIT_USAGE);

        let mut err = Vec::new();
        let code = run_opening(
            &args(&["get", "--namespace", "n", "k"]),
            || Err(("identity_unavailable", "no record".into())),
            &mut input,
            &mut out,
            &mut err,
        );
        assert_eq!(code, EXIT_UNAVAILABLE);
        assert_eq!(
            String::from_utf8(err).expect("utf8"),
            "error: identity_unavailable: no record\n"
        );

        let mut out = Vec::new();
        let code = run_opening(
            &args(&["set", "--namespace", "n", "k", "v"]),
            || {
                Ok((
                    LocalStore::in_memory().expect("store"),
                    crate::launch::LaunchIdentity {
                        owner_user_id: "o".into(),
                        context_id: "c".into(),
                        partition_id: "ses_p".into(),
                    },
                ))
            },
            &mut input,
            &mut out,
            &mut Vec::new(),
        );
        assert_eq!(code, EXIT_OK);
        assert_eq!(out, b"1\n");
    }
}
