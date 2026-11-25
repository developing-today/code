//! Argument parsing and dispatch for `fleet kv`.
//!
//! Parsing is strict and finishes before any store access: an unknown flag, a
//! missing `--namespace` or a malformed value is a usage error (exit 2) and
//! touches nothing. Typed store refusals print `error: <code>: <message>` and
//! exit 1. `--global` and `sync` reach the central plane (code#13316): when
//! it is not configured or cannot be reached they exit 69 before the local
//! store is opened. `--force-blob` is refused as `pending_needs_phase3`
//! (exit 69) because the blob plane arrives in Phase 3.

use std::io::{Read, Write};

use std::path::PathBuf;

use crate::backup::{self, BackupError, Manifest};
use crate::central::{CentralError, Op, Pipeline};
use crate::sync::{self, GlobalError};
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
  sync                                         pull namespace definitions, then
                                               sweep this session's intents

host store (operators):
  snapshot       DEST                          write a one-file snapshot to DEST
  restore        SNAPSHOT                      restore SNAPSHOT as this host's store

Values of any size are accepted: up to 1 MiB they are stored inline, and
larger values stream into a content-addressed working file beside the
store (`get` streams them back and checks their hash).

Keys are scoped to this session's launch identity (owner, context and
session). With no trusted launch-identity record every verb but help
refuses with identity_unavailable (exit 69).

--global pins get, set, list and delete to the central store: it reads
the authoritative head (no cache) and writes by compare-and-swap against
the head it read, or against --expect VERSION. A moved head is a typed
conflict (exit 1) whose proposed value central keeps. The central store
is FLEET_KV_URL with a bearer token in the file FLEET_KV_TOKEN_FILE;
without them --global and sync exit 69 with central_not_configured.

After the first sync this host knows which namespaces exist: any other
namespace is refused with unknown_namespace. --force-blob (blob placement)
arrives in Phase 3 and is refused with pending_needs_phase3 (exit 69).

Exit codes: 0 ok, 1 typed refusal (malformed_key, not_found, conflict,
unknown_namespace, backup_target_exists, not_a_snapshot, ...),
2 usage error, 69 unavailable.
";

const GET_USAGE: &str = "usage: fleet kv get [--global] --namespace NS KEY\n\nPrint the value's bytes exactly. A missing or masked key exits 1 with not_found.\n--global reads the authoritative central head instead of this host's overlay.\n";
const SET_USAGE: &str = "usage: fleet kv set [--global [--expect VERSION]] --namespace NS KEY (VALUE | --stdin)\n\nStore bytes as a new local version and queue it for the sweep. Prints the version.\n--global writes central directly by compare-and-swap; --expect VERSION (0 for\nabsent) fails with conflict unless central is still at VERSION.\n";
const LIST_USAGE: &str = "usage: fleet kv list [--global] --namespace NS [--include-tombstones]\n\nOne line per key: KEY<TAB>VERSION<TAB>ORIGIN (local, masked or central).\n";
const DELETE_USAGE: &str = "usage: fleet kv delete [--global [--expect VERSION]] --namespace NS KEY\n\nWrite a masking tombstone: the key reads absent here and sweeps as a mask.\n--global writes the tombstone to central directly by compare-and-swap.\n";
const SYNC_USAGE: &str = "usage: fleet kv sync\n\nPull this host's namespace definitions from central, then sweep this session's\npending intents into central by compare-and-swap. Prints a summary line, then\none line per conflict or waiting key.\n";
const CLEAR_USAGE: &str = "usage: fleet kv clear-override --namespace NS KEY\n\nDrop the local overlay row, advance the key's fence and cancel unsent sweep\nintents. History is kept. Busy for over 5 s: clear_busy_try_again.\n";
const SNAPSHOT_USAGE: &str = "usage: fleet kv snapshot DEST\n\nWrite a consistent one-file snapshot of this host's whole store (WAL included)\nto DEST, with a manifest row inside it. Never overwrites DEST. Prints the manifest.\n";
const RESTORE_USAGE: &str = "usage: fleet kv restore SNAPSHOT\n\nRestore SNAPSHOT as this host's store after checking its manifest, schema\nversion and integrity. Refuses when a store (or its WAL) already exists.\nPrints the manifest.\n";
const REVERT_USAGE: &str = "usage: fleet kv revert --namespace NS KEY --to VERSION\n       fleet kv revert NS KEY --to VERSION\n\nAppend a new version equal to VERSION from this key's local history.\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verb {
    Get,
    Set,
    List,
    Delete,
    ClearOverride,
    Revert,
    Sync,
    Snapshot,
    Restore,
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
            "sync" => Self::Sync,
            "snapshot" => Self::Snapshot,
            "restore" => Self::Restore,
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
            Self::Sync => SYNC_USAGE,
            Self::Snapshot => SNAPSHOT_USAGE,
            Self::Restore => RESTORE_USAGE,
        }
    }

    /// Verbs that `--global` can pin to the central store.
    fn takes_global(self) -> bool {
        matches!(self, Self::Get | Self::Set | Self::List | Self::Delete)
    }

    /// Host-store verbs act on the whole store, not on one namespace.
    fn is_host(self) -> bool {
        matches!(self, Self::Snapshot | Self::Restore)
    }
}

/// A `--global` write's compare-and-swap base: the head it reads, or the
/// version the caller names with `--expect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    CurrentHead,
    Version(u64),
}

impl Expect {
    fn version(self) -> Option<u64> {
        match self {
            Self::CurrentHead => None,
            Self::Version(version) => Some(version),
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
        global: bool,
    },
    Set {
        namespace: String,
        key: String,
        value: ValueSource,
        global: Option<Expect>,
    },
    List {
        namespace: String,
        include_tombstones: bool,
        global: bool,
    },
    Delete {
        namespace: String,
        key: String,
        global: Option<Expect>,
    },
    Sync,
    ClearOverride {
        namespace: String,
        key: String,
    },
    Revert {
        namespace: String,
        key: String,
        to: u64,
    },
    Snapshot {
        dest: PathBuf,
    },
    Restore {
        snapshot: PathBuf,
    },
}

impl Command {
    fn operation(&self) -> Option<&'static str> {
        Some(match self {
            Self::Help(_) => return None,
            Self::Get { .. } => "get",
            Self::Set { .. } => "set",
            Self::List { .. } => "list",
            Self::Delete { .. } => "delete",
            Self::ClearOverride { .. } => "clear-override",
            Self::Revert { .. } => "revert",
            Self::Sync => "sync",
            Self::Snapshot { .. } => "snapshot",
            Self::Restore { .. } => "restore",
        })
    }

    /// Commands that need the central plane before anything else.
    fn needs_central(&self) -> bool {
        match self {
            Self::Get { global, .. } | Self::List { global, .. } => *global,
            Self::Set { global, .. } | Self::Delete { global, .. } => global.is_some(),
            Self::Sync => true,
            _ => false,
        }
    }
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
    if args.iter().any(|arg| arg == "--force-blob") {
        return Err(ParseError::Central("--force-blob"));
    }
    // `--global` may come before the verb, as in `fleet kv --global get`.
    let (global, first) = if first == "--global" {
        match args.get(1) {
            Some(verb) => (true, verb),
            None => return Err(usage("--global needs a verb")),
        }
    } else {
        (false, first)
    };
    let skip = if global { 2 } else { 1 };
    let verb = Verb::parse(first).ok_or_else(|| usage(format!("unknown verb `{first}`")))?;

    let mut namespace: Option<String> = None;
    let mut include_tombstones = false;
    let mut stdin = false;
    let mut to: Option<u64> = None;
    let mut global = global;
    let mut expect: Option<u64> = None;
    let mut positionals: Vec<String> = Vec::new();
    let mut rest = args[skip..].iter();
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
            "--global" if verb.takes_global() && inline.is_none() => global = true,
            "--expect" if matches!(verb, Verb::Set | Verb::Delete) => {
                let raw = take_value("--expect")?;
                let version = raw
                    .parse::<u64>()
                    .map_err(|_| usage(format!("--expect needs a version number, got `{raw}`")))?;
                expect = Some(version);
            }
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

    if global && !verb.takes_global() {
        return Err(usage(format!(
            "--global applies to get, set, list and delete, not `kv {first}`"
        )));
    }
    if expect.is_some() && !global {
        return Err(usage("--expect VERSION needs --global"));
    }
    let write_scope = global.then(|| expect.map_or(Expect::CurrentHead, Expect::Version));
    if verb == Verb::Revert && namespace.is_none() && positionals.len() == 2 {
        namespace = Some(positionals.remove(0));
    }
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
    if verb.is_host() {
        if namespace.is_some() {
            return Err(usage(format!(
                "`kv {first}` acts on the whole host store and takes no --namespace"
            )));
        }
        expect(1)?;
        let path = PathBuf::from(positionals.remove(0));
        return Ok(if verb == Verb::Snapshot {
            Command::Snapshot { dest: path }
        } else {
            Command::Restore { snapshot: path }
        });
    }
    if verb == Verb::Sync {
        if namespace.is_some() {
            return Err(usage(
                "`kv sync` acts on every namespace and takes no --namespace",
            ));
        }
        expect(0)?;
        return Ok(Command::Sync);
    }
    let namespace = namespace.ok_or_else(|| usage("--namespace NS is required"))?;
    Ok(match verb {
        Verb::Get => {
            expect(1)?;
            Command::Get {
                namespace,
                key: positionals.remove(0),
                global,
            }
        }
        Verb::Set => {
            if stdin {
                expect(1)?;
                Command::Set {
                    namespace,
                    key: positionals.remove(0),
                    value: ValueSource::Stdin,
                    global: write_scope,
                }
            } else {
                expect(2)?;
                let value = positionals.remove(1);
                Command::Set {
                    namespace,
                    key: positionals.remove(0),
                    value: ValueSource::Inline(value),
                    global: write_scope,
                }
            }
        }
        Verb::List => {
            expect(0)?;
            Command::List {
                namespace,
                include_tombstones,
                global,
            }
        }
        Verb::Delete => {
            expect(1)?;
            Command::Delete {
                namespace,
                key: positionals.remove(0),
                global: write_scope,
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
        Verb::Sync | Verb::Snapshot | Verb::Restore => {
            unreachable!("sync and host verbs returned above")
        }
    })
}

fn refuse(err: &mut dyn Write, error: &KvError) -> i32 {
    let _ = writeln!(err, "error: {}: {error}", error.code());
    EXIT_REFUSED
}

fn refuse_backup(err: &mut dyn Write, error: &BackupError) -> i32 {
    let _ = writeln!(err, "error: {}: {error}", error.code());
    EXIT_REFUSED
}

fn write_manifest(out: &mut dyn Write, manifest: &Manifest) -> std::io::Result<()> {
    writeln!(
        out,
        "manifest={} taken_at_ms={} schema_version={} overlay_rows={} pinned_files={}",
        manifest.id,
        manifest.taken_at_ms,
        manifest.schema_version,
        manifest.overlay_rows,
        manifest.pinned_files.len()
    )
}

fn log_operation(command: &Command, err: &mut dyn Write, ctx: &fleet_config::ctx::Ctx) {
    if let (Some(operation), Some(ctx_id)) = (command.operation(), ctx.id()) {
        // Log only the propagated join key and operation. KV values,
        // namespaces, keys, paths, and the store's separate scope identity
        // can all carry user data and do not belong in this record.
        let _ = writeln!(err, "fleet-kv: op={operation} FLEET_CTX_ID={ctx_id}");
    }
}

fn report_parse_error(error: ParseError, err: &mut dyn Write) -> i32 {
    match error {
        ParseError::Central(flag) => {
            let _ = writeln!(
                err,
                "error: pending_needs_phase3: {flag} needs the blob plane, which arrives in Phase 3"
            );
            EXIT_UNAVAILABLE
        }
        ParseError::Usage(message) => {
            let _ = writeln!(err, "error: usage: {message}\n\n{USAGE}");
            EXIT_USAGE
        }
    }
}

/// What opening the central plane yields.
pub type CentralOpened = Result<Box<dyn Pipeline>, CentralError>;

/// The central plane the environment configures: `FLEET_KV_URL` with the
/// bearer token in the file `FLEET_KV_TOKEN_FILE`, or
/// `central_not_configured`.
pub fn central_from_env() -> CentralOpened {
    #[cfg(feature = "http")]
    {
        match crate::http::HttpPipeline::from_env()? {
            Some(pipeline) => Ok(Box::new(pipeline)),
            None => Err(CentralError::NotConfigured(format!(
                "set {} and {} to reach the central store",
                crate::http::URL_VAR,
                crate::http::TOKEN_FILE_VAR
            ))),
        }
    }
    #[cfg(not(feature = "http"))]
    {
        Err(CentralError::NotConfigured(
            "this build has no central client (fleet-kv `http` feature)".into(),
        ))
    }
}

fn central_exit(error: &CentralError) -> i32 {
    match error {
        CentralError::Unavailable(_) | CentralError::NotConfigured(_) => EXIT_UNAVAILABLE,
        _ => EXIT_REFUSED,
    }
}

fn refuse_central(err: &mut dyn Write, error: &CentralError) -> i32 {
    let _ = writeln!(err, "error: {}: {error}", error.code());
    central_exit(error)
}

fn refuse_global(err: &mut dyn Write, error: &GlobalError) -> i32 {
    let _ = writeln!(err, "error: {}: {error}", error.code());
    if error.is_unavailable() {
        EXIT_UNAVAILABLE
    } else {
        EXIT_REFUSED
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

/// Where `restore` writes: this host's store path, once the launch identity
/// checked out; or a typed refusal code and message.
pub type RestoreTarget = Result<PathBuf, (&'static str, String)>;

/// Run one invocation, opening the store and identity only once the whole
/// command line has parsed. `restore` never opens the store: it asks
/// `restore_target` (which checks the launch identity) where the store
/// belongs. An open failure is reported as `error: <code>: <message>` and
/// exits 69.
pub fn run_opening(
    args: &[String],
    open: impl FnOnce() -> Opened,
    restore_target: impl FnOnce() -> RestoreTarget,
    stdin: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    run_opening_with_central(
        args,
        open,
        restore_target,
        &central_from_env,
        stdin,
        out,
        err,
    )
}

/// [`run_opening`] with an explicit central plane. A command that needs
/// central opens it first, so an unconfigured or unreachable central plane
/// exits 69 before the local store is opened or created.
pub fn run_opening_with_central(
    args: &[String],
    open: impl FnOnce() -> Opened,
    restore_target: impl FnOnce() -> RestoreTarget,
    central: &dyn Fn() -> CentralOpened,
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
    if let Command::Restore { snapshot } = &command {
        let target = match restore_target() {
            Ok(target) => target,
            Err((code, message)) => {
                let _ = writeln!(err, "error: {code}: {message}");
                return EXIT_UNAVAILABLE;
            }
        };
        log_operation(&command, err, &fleet_config::ctx::Ctx::from_env());
        return match backup::restore(snapshot, &target) {
            Ok(manifest) => finish(write_manifest(out, &manifest), err),
            Err(error) => refuse_backup(err, &error),
        };
    }
    let opened_central = if command.needs_central() {
        match central() {
            Ok(primary) => Some(primary),
            Err(error) => return refuse_central(err, &error),
        }
    } else {
        None
    };
    let (store, launch) = match open() {
        Ok(opened) => opened,
        Err((code, message)) => {
            let _ = writeln!(err, "error: {code}: {message}");
            return EXIT_UNAVAILABLE;
        }
    };
    execute(
        command,
        &store,
        &launch.identity(),
        opened_central.as_deref(),
        stdin,
        out,
        err,
        &fleet_config::ctx::Ctx::from_env(),
    )
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
    run_with_central(args, store, identity, &central_from_env, stdin, out, err)
}

/// [`run`] with an explicit central plane, opened only when the command
/// needs it.
pub fn run_with_central(
    args: &[String],
    store: &LocalStore,
    identity: &Identity<'_>,
    central: &dyn Fn() -> CentralOpened,
    stdin: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    run_with_context(
        args,
        store,
        identity,
        central,
        stdin,
        out,
        err,
        &fleet_config::ctx::Ctx::from_env(),
    )
}

#[allow(clippy::too_many_arguments)]
fn run_with_context(
    args: &[String],
    store: &LocalStore,
    identity: &Identity<'_>,
    central: &dyn Fn() -> CentralOpened,
    stdin: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
    ctx: &fleet_config::ctx::Ctx,
) -> i32 {
    let command = match parse(args) {
        Ok(command) => command,
        Err(error) => return report_parse_error(error, err),
    };
    let opened = if command.needs_central() {
        match central() {
            Ok(primary) => Some(primary),
            Err(error) => return refuse_central(err, &error),
        }
    } else {
        None
    };
    execute(
        command,
        store,
        identity,
        opened.as_deref(),
        stdin,
        out,
        err,
        ctx,
    )
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

/// Sleeps between central retries.
fn sleep(delay: std::time::Duration) {
    std::thread::sleep(delay);
}

/// The namespace a local verb names, once the host's cache allows it.
fn local_namespace(command: &Command) -> Option<&str> {
    match command {
        Command::Get { namespace, .. }
        | Command::Set { namespace, .. }
        | Command::List { namespace, .. }
        | Command::Delete { namespace, .. }
        | Command::Revert { namespace, .. } => Some(namespace.as_str()),
        _ => None,
    }
}

/// The `--global` verbs and `sync`, against an opened central plane.
fn execute_central(
    command: Command,
    store: &LocalStore,
    identity: &Identity<'_>,
    primary: &dyn Pipeline,
    stdin: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let written = match command {
        Command::Get { namespace, key, .. } => {
            match sync::global_get(primary, identity, &namespace, &key, out) {
                Ok(_) => Ok(()),
                Err(error) => return refuse_global(err, &error),
            }
        }
        Command::List {
            namespace,
            include_tombstones,
            ..
        } => match sync::global_list(primary, identity, &namespace, include_tombstones) {
            Ok(entries) => entries.iter().try_for_each(|entry| {
                let origin = if entry.live { "central" } else { "masked" };
                writeln!(out, "{}\t{}\t{origin}", entry.key, entry.version)
            }),
            Err(error) => return refuse_global(err, &error),
        },
        Command::Set {
            namespace,
            key,
            value,
            global,
        } => {
            let value = match value {
                ValueSource::Inline(value) => value.into_bytes(),
                // Bounded by the largest inline value central can hold; the
                // namespace's own threshold is checked again before the CAS.
                ValueSource::Stdin => {
                    match sync::read_bounded(stdin, crate::working::INLINE_LIMIT as u64) {
                        Ok(value) => value,
                        Err(error) => return refuse_global(err, &error),
                    }
                }
            };
            let expect = global.and_then(Expect::version);
            match sync::global_write(
                store,
                primary,
                identity,
                &namespace,
                &key,
                Op::Set(value),
                expect,
                &mut sleep,
            ) {
                Ok(version) => writeln!(out, "{version}"),
                Err(error) => return refuse_global(err, &error),
            }
        }
        Command::Delete {
            namespace,
            key,
            global,
        } => {
            let expect = global.and_then(Expect::version);
            match sync::global_write(
                store,
                primary,
                identity,
                &namespace,
                &key,
                Op::Tombstone,
                expect,
                &mut sleep,
            ) {
                Ok(version) => writeln!(out, "{version}"),
                Err(error) => return refuse_global(err, &error),
            }
        }
        Command::Sync => {
            let report =
                match sync::sync(store, identity, primary, identity.owner_user_id, &mut sleep) {
                    Ok(report) => report,
                    Err(error) => {
                        let _ = writeln!(err, "error: {}: {error}", error.code());
                        return match &error {
                            crate::sweep::SweepError::Central(central) => central_exit(central),
                            crate::sweep::SweepError::Local(_) => EXIT_REFUSED,
                        };
                    }
                };
            write_sync_report(out, &report)
        }
        _ => {
            let _ = writeln!(
                err,
                "error: usage: this verb does not use the central store"
            );
            return EXIT_USAGE;
        }
    };
    finish(written, err)
}

fn write_sync_report(out: &mut dyn Write, report: &sync::SyncReport) -> std::io::Result<()> {
    writeln!(
        out,
        "namespaces={} committed={} conflicts={} waiting={}",
        report.namespaces,
        report.sweep.committed(),
        report.sweep.conflicts(),
        report.sweep.waiting.len()
    )?;
    for swept in &report.sweep.swept {
        if let crate::central::Outcome::Conflict {
            expected, actual, ..
        } = swept.outcome
        {
            writeln!(
                out,
                "conflict\t{}\t{}\texpected={expected}\tactual={actual}",
                swept.namespace, swept.key
            )?;
        }
    }
    for waiting in &report.sweep.waiting {
        writeln!(
            out,
            "waiting\t{}\t{}\t{}",
            waiting.namespace, waiting.key, waiting.reason
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn execute(
    command: Command,
    store: &LocalStore,
    identity: &Identity<'_>,
    central: Option<&dyn Pipeline>,
    stdin: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
    ctx: &fleet_config::ctx::Ctx,
) -> i32 {
    log_operation(&command, err, ctx);
    if command.needs_central() {
        let Some(primary) = central else {
            let error = CentralError::NotConfigured("the central store was not opened".into());
            return refuse_central(err, &error);
        };
        return execute_central(command, store, identity, primary, stdin, out, err);
    }
    if let Some(namespace) = local_namespace(&command)
        && let Err(error) = sync::require_known(store, namespace)
    {
        return refuse(err, &error);
    }
    let written = match command {
        Command::Help(text) => out.write_all(text.as_bytes()),
        Command::Get { namespace, key, .. } => {
            match store.get_to(identity, &namespace, &key, out) {
                Ok(true) => Ok(()),
                Ok(false) => {
                    let _ = writeln!(
                        err,
                        "error: not_found: no live value for `{key}` in `{namespace}`"
                    );
                    return EXIT_REFUSED;
                }
                Err(error) => return refuse(err, &error),
            }
        }
        Command::Set {
            namespace,
            key,
            value,
            ..
        } => {
            let stored = match value {
                ValueSource::Inline(value) => {
                    store.set(identity, &namespace, &key, value.as_bytes())
                }
                ValueSource::Stdin => store.set_reader(identity, &namespace, &key, stdin),
            };
            match stored {
                Ok(version) => writeln!(out, "{version}"),
                // Reading stdin or writing a working file failed: io_error.
                Err(error) => return refuse(err, &error),
            }
        }
        Command::List {
            namespace,
            include_tombstones,
            ..
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
        Command::Delete { namespace, key, .. } => match store.delete(identity, &namespace, &key) {
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
        Command::Sync => {
            let _ = writeln!(err, "error: usage: `kv sync` needs the central store");
            return EXIT_USAGE;
        }
        Command::Snapshot { dest } => match store.snapshot(&dest) {
            Ok(manifest) => write_manifest(out, &manifest),
            Err(error) => return refuse_backup(err, &error),
        },
        Command::Restore { .. } => {
            // Restoring replaces the store, so it cannot run against one
            // that is already open: only `run_opening` restores.
            let _ = writeln!(
                err,
                "error: usage: `kv restore` runs through `fleet kv`, not against an open store"
            );
            return EXIT_USAGE;
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
        invoke_with_context(
            store,
            words,
            stdin,
            &fleet_config::ctx::Ctx::from_pairs(std::iter::empty::<(&str, &str)>()),
        )
    }

    fn invoke_with_context(
        store: &LocalStore,
        words: &[&str],
        stdin: &[u8],
        ctx: &fleet_config::ctx::Ctx,
    ) -> Outcome {
        let identity = Identity::new("owner", "context", "partition");
        let mut input = stdin;
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with_context(
            &args(words),
            store,
            &identity,
            &|| {
                Err(CentralError::NotConfigured(
                    "no central in this test".into(),
                ))
            },
            &mut input,
            &mut out,
            &mut err,
            ctx,
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
    fn operation_log_joins_origin_context_without_logging_key_or_value() {
        let store = LocalStore::in_memory().expect("store");
        let ctx = fleet_config::ctx::Ctx::from_pairs([("FLEET_CTX_ID", "task-22655")]);

        let set = invoke_with_context(
            &store,
            &["set", "--namespace", "private", "api-key", "secret-value"],
            b"",
            &ctx,
        );

        assert_eq!(set.code, EXIT_OK, "{}", set.err);
        assert_eq!(set.err, "fleet-kv: op=set FLEET_CTX_ID=task-22655\n");
        assert!(!set.err.contains("api-key"));
        assert!(!set.err.contains("secret-value"));

        let get = invoke_with_context(
            &store,
            &["get", "--namespace", "private", "api-key"],
            b"",
            &ctx,
        );
        assert_eq!(get.code, EXIT_OK, "{}", get.err);
        assert_eq!(get.out, b"secret-value");
        assert_eq!(get.err, "fleet-kv: op=get FLEET_CTX_ID=task-22655\n");

        let no_ctx = invoke(&store, &["get", "--namespace", "private", "api-key"], b"");
        assert!(no_ctx.err.is_empty(), "{}", no_ctx.err);
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
    fn global_without_a_central_store_is_unavailable_and_writes_nothing() {
        let store = LocalStore::in_memory().expect("store");
        for words in [
            &["get", "--global", "--namespace", "notes", "k"][..],
            &["set", "--namespace", "notes", "--global", "k", "v"][..],
            &["--global", "list", "--namespace", "notes"][..],
            &[
                "delete",
                "--global",
                "--expect",
                "3",
                "--namespace",
                "notes",
                "k",
            ][..],
            &["sync"][..],
        ] {
            let outcome = invoke(&store, words, b"");
            assert_eq!(outcome.code, EXIT_UNAVAILABLE, "{words:?}");
            assert!(
                outcome.err.starts_with("error: central_not_configured"),
                "{}",
                outcome.err
            );
        }
        let blob = invoke(
            &store,
            &["set", "--namespace", "notes", "--force-blob", "k", "v"],
            b"",
        );
        assert_eq!(blob.code, EXIT_UNAVAILABLE);
        assert!(
            blob.err.starts_with("error: pending_needs_phase3"),
            "{}",
            blob.err
        );
        let list = invoke(&store, &["list", "--namespace", "notes"], b"");
        assert_eq!(text(&list), "");
    }

    struct Central {
        primary: std::sync::Arc<crate::central::SqlitePrimary>,
    }

    impl Central {
        fn new() -> Self {
            let primary = crate::central::SqlitePrimary::in_memory().expect("primary");
            crate::central::define_namespace(
                &primary,
                &crate::central::Namespace::shared("status", "fleet-status"),
            )
            .expect("define");
            Self {
                primary: std::sync::Arc::new(primary),
            }
        }

        fn invoke(&self, store: &LocalStore, user: &str, words: &[&str], stdin: &[u8]) -> Outcome {
            let identity = Identity::new(user, "context", "partition");
            let mut input = stdin;
            let mut out = Vec::new();
            let mut err = Vec::new();
            let shared = SharedPrimary(self.primary.clone());
            let code = run_with_context(
                &args(words),
                store,
                &identity,
                &|| Ok(Box::new(shared.clone()) as Box<dyn Pipeline>),
                &mut input,
                &mut out,
                &mut err,
                &fleet_config::ctx::Ctx::from_pairs(std::iter::empty::<(&str, &str)>()),
            );
            Outcome {
                code,
                out,
                err: String::from_utf8(err).expect("utf8 stderr"),
            }
        }
    }

    /// A shared primary that the CLI can own as a boxed pipeline.
    #[derive(Clone)]
    struct SharedPrimary(std::sync::Arc<crate::central::SqlitePrimary>);

    impl Pipeline for SharedPrimary {
        fn transaction(
            &self,
            statements: &[crate::central::Statement],
        ) -> crate::central::Result<Vec<crate::central::Rows>> {
            self.0.transaction(statements)
        }
    }

    #[test]
    fn central_cli_global_round_trip_and_expect_conflict() {
        let central = Central::new();
        let store = LocalStore::in_memory().expect("store");
        let set = central.invoke(
            &store,
            "u1",
            &["--global", "set", "--namespace", "status", "claim", "mine"],
            b"",
        );
        assert_eq!(
            (set.code, text(&set)),
            (EXIT_OK, "1\n".into()),
            "{}",
            set.err
        );
        let get = central.invoke(
            &store,
            "u2",
            &["get", "--global", "--namespace", "status", "claim"],
            b"",
        );
        assert_eq!((get.code, get.out), (EXIT_OK, b"mine".to_vec()));
        // The global write left no local overlay row.
        let local = invoke(&store, &["get", "--namespace", "status", "claim"], b"");
        assert_eq!(local.code, EXIT_REFUSED);
        let stale = central.invoke(
            &store,
            "u2",
            &[
                "set",
                "--global",
                "--expect",
                "0",
                "--namespace",
                "status",
                "claim",
                "theirs",
            ],
            b"",
        );
        assert_eq!(stale.code, EXIT_REFUSED);
        assert!(stale.err.starts_with("error: conflict:"), "{}", stale.err);
        let listed = central.invoke(
            &store,
            "u1",
            &["list", "--global", "--namespace", "status"],
            b"",
        );
        assert_eq!(text(&listed), "claim\t1\tcentral\n");
        let deleted = central.invoke(
            &store,
            "u1",
            &[
                "delete",
                "--global",
                "--expect",
                "1",
                "--namespace",
                "status",
                "claim",
            ],
            b"",
        );
        assert_eq!(text(&deleted), "2\n", "{}", deleted.err);
        let masked = central.invoke(
            &store,
            "u1",
            &[
                "list",
                "--global",
                "--include-tombstones",
                "--namespace",
                "status",
            ],
            b"",
        );
        assert_eq!(text(&masked), "claim\t2\tmasked\n");
        let unknown = central.invoke(
            &store,
            "u1",
            &["get", "--global", "--namespace", "nowhere", "k"],
            b"",
        );
        assert_eq!(unknown.code, EXIT_REFUSED);
        assert!(
            unknown.err.starts_with("error: unknown_namespace:"),
            "{}",
            unknown.err
        );
    }

    #[test]
    fn central_cli_sync_pulls_sweeps_and_then_refuses_unknown_namespaces() {
        let central = Central::new();
        let store = LocalStore::in_memory().expect("store");
        // Before the first sync any valid namespace works locally.
        for words in [
            &["set", "--namespace", "status", "agent", "working"][..],
            &["set", "--namespace", "elsewhere", "agent", "here"][..],
        ] {
            assert_eq!(invoke(&store, words, b"").code, EXIT_OK);
        }
        let synced = central.invoke(&store, "owner", &["sync"], b"");
        assert_eq!(synced.code, EXIT_OK, "{}", synced.err);
        assert_eq!(
            text(&synced),
            "namespaces=1 committed=1 conflicts=0 waiting=1\nwaiting\telsewhere\tagent\tunknown_namespace\n"
        );
        let global = central.invoke(
            &store,
            "owner",
            &["get", "--global", "--namespace", "status", "agent"],
            b"",
        );
        assert_eq!(global.out, b"working");
        // After the pull, an undefined namespace is a typed refusal locally.
        let refused = invoke(&store, &["set", "--namespace", "elsewhere", "k", "v"], b"");
        assert_eq!(refused.code, EXIT_REFUSED);
        assert!(
            refused.err.starts_with("error: unknown_namespace:"),
            "{}",
            refused.err
        );
        assert_eq!(
            invoke(&store, &["set", "--namespace", "status", "k", "v"], b"").code,
            EXIT_OK
        );
        // clear-override stays available: it only drops local state.
        assert_eq!(
            invoke(
                &store,
                &["clear-override", "--namespace", "elsewhere", "agent"],
                b""
            )
            .code,
            EXIT_OK
        );
    }

    #[test]
    fn global_flags_parse_strictly() {
        let store = LocalStore::in_memory().expect("store");
        for words in [
            &["revert", "--global", "--namespace", "n", "k", "--to", "1"][..],
            &["--global", "clear-override", "--namespace", "n", "k"][..],
            &["--global"][..],
            &["set", "--expect", "1", "--namespace", "n", "k", "v"][..],
            &[
                "set",
                "--global",
                "--expect",
                "x",
                "--namespace",
                "n",
                "k",
                "v",
            ][..],
            &["get", "--global", "--expect", "1", "--namespace", "n", "k"][..],
            &["sync", "--namespace", "n"][..],
            &["sync", "extra"][..],
        ] {
            let outcome = invoke(&store, words, b"");
            assert_eq!(outcome.code, EXIT_USAGE, "{words:?}: {}", outcome.err);
        }
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
                &args(&["set", "--force-blob", "--namespace", "n", "k", "v"]),
                &mut out,
                &mut err
            ),
            Some(EXIT_UNAVAILABLE)
        );
        assert_eq!(
            preflight(
                &args(&["list", "--global", "--namespace", "n"]),
                &mut out,
                &mut err
            ),
            None
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
            || panic!("must not resolve a restore target"),
            &mut input,
            &mut out,
            &mut err,
        );
        assert_eq!(code, EXIT_USAGE);

        let mut err = Vec::new();
        let code = run_opening(
            &args(&["get", "--namespace", "n", "k"]),
            || Err(("identity_unavailable", "no record".into())),
            || panic!("only restore resolves a target"),
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
            || panic!("only restore resolves a target"),
            &mut input,
            &mut out,
            &mut Vec::new(),
        );
        assert_eq!(code, EXIT_OK);
        assert_eq!(out, b"1\n");
    }

    fn scratch(label: &str) -> PathBuf {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "fleet-kv-cli-{label}-{}-{suffix}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    fn launch_identity() -> crate::launch::LaunchIdentity {
        crate::launch::LaunchIdentity {
            owner_user_id: "owner".into(),
            context_id: "context".into(),
            partition_id: "partition".into(),
        }
    }

    #[test]
    fn host_verbs_parse_strictly() {
        let store = LocalStore::in_memory().expect("store");
        for words in [
            &["snapshot"][..],
            &["snapshot", "a", "b"],
            &["snapshot", "--namespace", "n", "dest"],
            &["restore", "--namespace=n", "snap"],
            &["restore", "--to", "1", "snap"],
        ] {
            let outcome = invoke(&store, words, b"");
            assert_eq!(outcome.code, EXIT_USAGE, "{words:?}: {}", outcome.err);
        }
        let help = invoke(&store, &["snapshot", "--help"], b"");
        assert_eq!(help.code, EXIT_OK);
        assert!(text(&help).starts_with("usage: fleet kv snapshot DEST"));
        let restore = invoke(&store, &["restore", "snap"], b"");
        assert_eq!(
            restore.code, EXIT_USAGE,
            "restore never runs on an open store"
        );
    }

    #[test]
    fn snapshot_then_restore_through_the_cli() {
        let dir = scratch("host");
        let source = dir.join("source.db");
        let snapshot = dir.join("snap.db");
        let target = dir.join("restored.db");
        let store = LocalStore::open(&source).expect("open");
        assert_eq!(
            invoke(&store, &["set", "--namespace", "notes", "k", "v"], b"").code,
            EXIT_OK
        );
        let taken = invoke(
            &store,
            &["snapshot", snapshot.to_str().expect("utf8 path")],
            b"",
        );
        assert_eq!(taken.code, EXIT_OK, "{}", taken.err);
        let line = text(&taken);
        assert!(line.starts_with("manifest=1 "), "{line}");
        assert!(line.ends_with(" overlay_rows=1 pinned_files=0\n"), "{line}");
        let again = invoke(
            &store,
            &["snapshot", snapshot.to_str().expect("utf8 path")],
            b"",
        );
        assert_eq!(again.code, EXIT_REFUSED);
        assert!(
            again.err.starts_with("error: backup_target_exists: "),
            "{}",
            again.err
        );

        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut input: &[u8] = b"";
        let code = run_opening(
            &args(&["restore", snapshot.to_str().expect("utf8 path")]),
            || panic!("restore must not open the store"),
            || Ok(target.clone()),
            &mut input,
            &mut out,
            &mut err,
        );
        assert_eq!(code, EXIT_OK, "{}", String::from_utf8_lossy(&err));
        assert_eq!(String::from_utf8(out).expect("utf8"), line);
        let restored = LocalStore::open(&target).expect("open restored");
        let me = launch_identity();
        assert_eq!(
            restored.get(&me.identity(), "notes", "k").expect("get"),
            Some(b"v".to_vec())
        );

        let mut err = Vec::new();
        let code = run_opening(
            &args(&["restore", snapshot.to_str().expect("utf8 path")]),
            || panic!("restore must not open the store"),
            || Ok(target.clone()),
            &mut input,
            &mut Vec::new(),
            &mut err,
        );
        assert_eq!(code, EXIT_REFUSED);
        assert!(
            String::from_utf8(err)
                .expect("utf8")
                .starts_with("error: backup_target_exists: ")
        );

        let mut err = Vec::new();
        let code = run_opening(
            &args(&["restore", snapshot.to_str().expect("utf8 path")]),
            || panic!("restore must not open the store"),
            || Err(("identity_unavailable", "no record".into())),
            &mut input,
            &mut Vec::new(),
            &mut err,
        );
        assert_eq!(code, EXIT_UNAVAILABLE);
        let _ = std::fs::remove_dir_all(dir);
    }
}
