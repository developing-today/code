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

use crate::archive;
use crate::backup::{self, BackupError, Manifest};
use crate::central::{CentralError, Op, Pipeline};
use crate::read;
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
  rebase         --namespace NS KEY            build a conflicted key on central's
                                               head and re-queue the overlay
  revert         --namespace NS KEY --to V     append a copy of version V
  sync                                         pull namespace definitions, then
                                               sweep this session's intents
                                               (and archive them when due)
  backup                                       sync, then archive this
                                               session's unswept writes now

host store (operators):
  snapshot       DEST                          write a one-file snapshot to DEST
  restore        SNAPSHOT                      restore SNAPSHOT as this host's store
  restore        --archive                     adopt this session's archived writes

central namespaces (operators, need kv-admin):
  namespace define NS (--shared SCOPE | --private)
                                               define or update NS on central
  namespace list                               print every central namespace

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
namespace is refused with unknown_namespace, and get and list also read
the caller's central keys (cached for at most 1 s; --allow-stale reads an
expired cache when central is unreachable). sync places a value over the
namespace's inline threshold as a blob: it streams the bytes to the upload
door beside FLEET_KV_URL (the token needs the kv-upload door) in the
partition FLEET_KV_BLOB_JURISDICTION and FLEET_KV_BLOB_TIER, then writes
the reference; without those two such values wait as pending_needs_phase3.
--force-blob is not wired yet and is refused with pending_needs_phase3
(exit 69).

Exit codes: 0 ok, 1 typed refusal (malformed_key, not_found, conflict,
unknown_namespace, backup_target_exists, not_a_snapshot, ...),
2 usage error, 69 unavailable.
";

const GET_USAGE: &str = "usage: fleet kv get [--global | --allow-stale] [--verbose] --namespace NS KEY\n\nPrint the value's bytes exactly: this host's overlay first, then (after a sync)\nthe central head, cached for at most 1 s. A missing or masked key exits 1 with\nnot_found. Central unreachable: unavailable, or stale for an expired cached head\n(exit 69); --allow-stale reads that head anyway. --verbose prints the source to\nstderr. --global reads the authoritative central head instead.\n";
const SET_USAGE: &str = "usage: fleet kv set [--global [--expect VERSION]] --namespace NS KEY (VALUE | --stdin)\n\nStore bytes as a new local version and queue it for the sweep. Prints the version.\n--global writes central directly by compare-and-swap; --expect VERSION (0 for\nabsent) fails with conflict unless central is still at VERSION.\n";
const LIST_USAGE: &str = "usage: fleet kv list [--global | --allow-stale] [--verbose] --namespace NS [--include-tombstones]\n\nOne line per key: KEY<TAB>VERSION<TAB>ORIGIN, where ORIGIN is local, central,\nboth or masked. --verbose adds each central row a local row hides\n(shadowed-by-local). --allow-stale lists local rows when central is unreachable.\n";
const DELETE_USAGE: &str = "usage: fleet kv delete [--global [--expect VERSION]] --namespace NS KEY\n\nWrite a masking tombstone: the key reads absent here and sweeps as a mask.\n--global writes the tombstone to central directly by compare-and-swap.\n";
const SYNC_USAGE: &str = "usage: fleet kv sync [--archive]\n\nPull this host's namespace definitions from central, then sweep this session's\npending intents into central by compare-and-swap. Prints a summary line, then\none line per conflict or waiting key. When the last archive checkpoint is over\n15 min old (or --archive), it then archives the writes that did not sweep:\ninline values up to the namespace threshold only; larger values are listed as\nprotection_unavailable_needs_phase3 and never uploaded. 15 min is an online\ntarget, not a recovery point: unarchived writes die with this disk.\n";
const BACKUP_USAGE: &str = "usage: fleet kv backup\n\nThe same as `fleet kv sync --archive`: sync, then checkpoint this session's\nunswept writes to the central survival archive now.\n";
const REBASE_USAGE: &str = "usage: fleet kv rebase --namespace NS KEY\n\nResolve a conflicted key: read central's head, make it the key's base, cancel\nthe key's pending intents and queue one new intent for this session's overlay\nvalue against that base (none after clear-override: the key follows central).\nPrints `rebased onto=VERSION requeued=VERSION|none`. Run `fleet kv sync` next.\n";
const CLEAR_USAGE: &str = "usage: fleet kv clear-override --namespace NS KEY\n\nDrop the local overlay row, advance the key's fence and cancel unsent sweep\nintents. History is kept. Busy for over 5 s: clear_busy_try_again.\n";
const SNAPSHOT_USAGE: &str = "usage: fleet kv snapshot DEST\n\nWrite a consistent one-file snapshot of this host's whole store (WAL included)\nto DEST, with a manifest row inside it. Never overwrites DEST. Prints the manifest.\n";
const RESTORE_USAGE: &str = "usage: fleet kv restore SNAPSHOT\n       fleet kv restore --archive\n\nRestore SNAPSHOT as this host's store after checking its manifest, schema\nversion and integrity. Refuses when a store (or its WAL) already exists.\nPrints the manifest.\n\n--archive instead adopts this session's last central archive checkpoint as new\nlocal writes (queued to sweep). Keys this store already has history for keep\ntheir local rows; keys the archive holds no bytes for are listed as\nprotection_unavailable_needs_phase3. No checkpoint: archive_not_found.\n";
const NAMESPACE_USAGE: &str = "usage: fleet kv namespace define NS (--shared SCOPE | --private) [--global-flag] [--blob-threshold BYTES]\n       fleet kv namespace list\n\nDefine a central namespace, or print them all. Needs the central store and an\nadmin credential (the hosted door's kv-admin role); the local store is not\nopened. --shared SCOPE gives every agent one shared scope; --private gives each\n(owner, context) its own. Defining NS again keeps its settings unless a flag\nchanges them, raises its policy epoch, and refuses to change shared to private\nor back (namespace_scope_change). --global-flag marks NS as global (a\ndesignation only). --blob-threshold is the inline ceiling, at most 1 MiB\n(default 256 KiB). Hosts see a change after their next `fleet kv sync`.\n\ndefine prints `namespace=NS scope=SCOPE epoch=N`; list prints one line per\nnamespace: NS<TAB>SCOPE<TAB>EPOCH<TAB>THRESHOLD<TAB>global|-, where SCOPE is\nshared:ID or private.\n";
const REVERT_USAGE: &str = "usage: fleet kv revert --namespace NS KEY --to VERSION\n       fleet kv revert NS KEY --to VERSION\n\nAppend a new version equal to VERSION from this key's local history.\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verb {
    Get,
    Set,
    List,
    Delete,
    ClearOverride,
    Rebase,
    Revert,
    Sync,
    Backup,
    Snapshot,
    Restore,
    Namespace,
}

impl Verb {
    fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "get" => Self::Get,
            "set" => Self::Set,
            "list" => Self::List,
            "delete" => Self::Delete,
            "clear-override" => Self::ClearOverride,
            "rebase" => Self::Rebase,
            "revert" => Self::Revert,
            "sync" => Self::Sync,
            "backup" => Self::Backup,
            "snapshot" => Self::Snapshot,
            "restore" => Self::Restore,
            "namespace" => Self::Namespace,
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
            Self::Rebase => REBASE_USAGE,
            Self::Revert => REVERT_USAGE,
            Self::Sync => SYNC_USAGE,
            Self::Backup => BACKUP_USAGE,
            Self::Snapshot => SNAPSHOT_USAGE,
            Self::Restore => RESTORE_USAGE,
            Self::Namespace => NAMESPACE_USAGE,
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
        allow_stale: bool,
        verbose: bool,
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
        allow_stale: bool,
        verbose: bool,
    },
    Delete {
        namespace: String,
        key: String,
        global: Option<Expect>,
    },
    /// `sync`; `archive` forces a checkpoint (`sync --archive`, `backup`).
    Sync {
        archive: bool,
    },
    /// `restore --archive`: adopt this partition's archived checkpoint.
    RestoreArchive,
    ClearOverride {
        namespace: String,
        key: String,
    },
    Rebase {
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
    /// `namespace define`: write a central namespace definition.
    NamespaceDefine {
        namespace: String,
        scope: Option<crate::central::Scope>,
        global_flag: bool,
        blob_threshold: Option<u64>,
    },
    /// `namespace list`: print every central namespace definition.
    NamespaceList,
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
            Self::Rebase { .. } => "rebase",
            Self::Revert { .. } => "revert",
            Self::Sync { archive: false } => "sync",
            Self::Sync { archive: true } => "backup",
            Self::RestoreArchive => "restore-archive",
            Self::Snapshot { .. } => "snapshot",
            Self::Restore { .. } => "restore",
            Self::NamespaceDefine { .. } => "namespace-define",
            Self::NamespaceList => "namespace-list",
        })
    }

    /// Commands that need the central plane before anything else.
    fn needs_central(&self) -> bool {
        match self {
            Self::Get { global, .. } | Self::List { global, .. } => *global,
            Self::Set { global, .. } | Self::Delete { global, .. } => global.is_some(),
            Self::Sync { .. } | Self::RestoreArchive | Self::Rebase { .. } => true,
            Self::NamespaceDefine { .. } | Self::NamespaceList => true,
            _ => false,
        }
    }

    /// Central admin commands: they never open the local store.
    fn is_admin(&self) -> bool {
        matches!(self, Self::NamespaceDefine { .. } | Self::NamespaceList)
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
    if verb == Verb::Namespace {
        if global {
            return Err(usage(
                "--global applies to get, set, list and delete, not `kv namespace`",
            ));
        }
        return parse_namespace(&args[skip..]);
    }

    let mut namespace: Option<String> = None;
    let mut include_tombstones = false;
    let mut stdin = false;
    let mut to: Option<u64> = None;
    let mut global = global;
    let mut expect: Option<u64> = None;
    let mut allow_stale = false;
    let mut verbose = false;
    let mut archive = false;
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
            "--allow-stale" if matches!(verb, Verb::Get | Verb::List) && inline.is_none() => {
                allow_stale = true;
            }
            "--verbose" if matches!(verb, Verb::Get | Verb::List) && inline.is_none() => {
                verbose = true;
            }
            "--archive" if matches!(verb, Verb::Sync | Verb::Restore) && inline.is_none() => {
                archive = true;
            }
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
    if allow_stale && global {
        return Err(usage(
            "--allow-stale reads the local cache; --global always reads central",
        ));
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
        if archive {
            expect(0)?;
            return Ok(Command::RestoreArchive);
        }
        expect(1)?;
        let path = PathBuf::from(positionals.remove(0));
        return Ok(if verb == Verb::Snapshot {
            Command::Snapshot { dest: path }
        } else {
            Command::Restore { snapshot: path }
        });
    }
    if matches!(verb, Verb::Sync | Verb::Backup) {
        if namespace.is_some() {
            return Err(usage(format!(
                "`kv {first}` acts on every namespace and takes no --namespace"
            )));
        }
        expect(0)?;
        return Ok(Command::Sync {
            archive: archive || verb == Verb::Backup,
        });
    }
    let namespace = namespace.ok_or_else(|| usage("--namespace NS is required"))?;
    Ok(match verb {
        Verb::Get => {
            expect(1)?;
            Command::Get {
                namespace,
                key: positionals.remove(0),
                global,
                allow_stale,
                verbose,
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
                allow_stale,
                verbose,
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
        Verb::Rebase => {
            expect(1)?;
            Command::Rebase {
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
        Verb::Sync | Verb::Backup | Verb::Snapshot | Verb::Restore | Verb::Namespace => {
            unreachable!("sync, host and namespace verbs returned above")
        }
    })
}

/// The largest `--blob-threshold`: the local inline ceiling.
const MAX_BLOB_THRESHOLD: u64 = crate::working::INLINE_LIMIT as u64;

/// `namespace define NS (--shared SCOPE | --private) [...]` and
/// `namespace list`.
fn parse_namespace(args: &[String]) -> Result<Command, ParseError> {
    let Some(action) = args.first() else {
        return Err(usage("`kv namespace` needs `define` or `list`"));
    };
    if matches!(action.as_str(), "-h" | "--help" | "help") {
        return Ok(Command::Help(NAMESPACE_USAGE));
    }
    let mut scope: Option<crate::central::Scope> = None;
    let mut global_flag = false;
    let mut blob_threshold: Option<u64> = None;
    let mut positionals: Vec<String> = Vec::new();
    let mut rest = args[1..].iter();
    let mut only_positionals = false;
    while let Some(arg) = rest.next() {
        if only_positionals || !arg.starts_with("--") {
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
        let define = action == "define";
        match flag {
            "--" => only_positionals = true,
            "--help" | "-h" => return Ok(Command::Help(NAMESPACE_USAGE)),
            "--shared" if define => {
                if scope.is_some() {
                    return Err(usage("give one of --shared SCOPE or --private"));
                }
                let scope_id = take_value("--shared")?;
                if scope_id.is_empty() || scope_id.starts_with("private:") {
                    return Err(usage(format!(
                        "--shared needs a scope id that does not start with `private:`, got `{scope_id}`"
                    )));
                }
                scope = Some(crate::central::Scope::Shared(scope_id));
            }
            "--private" if define && inline.is_none() => {
                if scope.is_some() {
                    return Err(usage("give one of --shared SCOPE or --private"));
                }
                scope = Some(crate::central::Scope::Private);
            }
            "--global-flag" if define && inline.is_none() => global_flag = true,
            "--blob-threshold" if define => {
                let raw = take_value("--blob-threshold")?;
                let bytes = raw
                    .parse::<u64>()
                    .ok()
                    .filter(|bytes| (1..=MAX_BLOB_THRESHOLD).contains(bytes))
                    .ok_or_else(|| {
                        usage(format!(
                            "--blob-threshold needs a byte count from 1 to {MAX_BLOB_THRESHOLD}, got `{raw}`"
                        ))
                    })?;
                blob_threshold = Some(bytes);
            }
            _ => {
                return Err(usage(format!(
                    "unknown argument `{arg}` for `kv namespace {action}`"
                )));
            }
        }
    }
    match action.as_str() {
        "define" => {
            if positionals.len() != 1 {
                return Err(usage(format!(
                    "`kv namespace define` takes 1 argument (the namespace), got {}",
                    positionals.len()
                )));
            }
            let namespace = positionals.remove(0);
            crate::validate_namespace(&namespace)
                .map_err(|_| usage(format!("`{namespace}` is not a valid namespace")))?;
            Ok(Command::NamespaceDefine {
                namespace,
                scope,
                global_flag,
                blob_threshold,
            })
        }
        "list" => {
            if !positionals.is_empty() {
                return Err(usage("`kv namespace list` takes no arguments"));
            }
            Ok(Command::NamespaceList)
        }
        other => Err(usage(format!(
            "unknown `kv namespace` action `{other}`: use define or list"
        ))),
    }
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

/// What opening the central plane yields: the pipeline, `None` when no
/// central store is configured, or why a configured one cannot be used.
pub type CentralOpened = Result<Option<Box<dyn Pipeline>>, CentralError>;

/// The central plane the environment configures: `FLEET_KV_URL` with the
/// bearer token in the file `FLEET_KV_TOKEN_FILE`.
pub fn central_from_env() -> CentralOpened {
    #[cfg(feature = "http")]
    {
        Ok(crate::http::HttpPipeline::from_env()?
            .map(|pipeline| Box::new(pipeline) as Box<dyn Pipeline>))
    }
    #[cfg(not(feature = "http"))]
    {
        Ok(None)
    }
}

/// The blob plane the environment configures: the upload door beside
/// `FLEET_KV_URL` and the partition in `FLEET_KV_BLOB_JURISDICTION` and
/// `FLEET_KV_BLOB_TIER`. `None` keeps over-threshold values local.
type BlobPlaneOpened =
    Result<Option<(Box<dyn crate::sweep::BlobUploader>, crate::blob::Partition)>, CentralError>;

fn blob_plane_from_env() -> BlobPlaneOpened {
    #[cfg(feature = "http")]
    {
        Ok(
            crate::http::HttpBlobUploader::from_env()?.map(|(uploader, partition)| {
                (
                    Box::new(uploader) as Box<dyn crate::sweep::BlobUploader>,
                    partition,
                )
            }),
        )
    }
    #[cfg(not(feature = "http"))]
    {
        Ok(None)
    }
}

fn not_configured() -> CentralError {
    CentralError::NotConfigured(
        "set FLEET_KV_URL and FLEET_KV_TOKEN_FILE to reach the central store".into(),
    )
}

/// The central plane, opened at most once and only when a command needs it.
struct CentralHandle<'a> {
    opener: &'a dyn Fn() -> CentralOpened,
    opened: std::cell::OnceCell<CentralOpened>,
}

impl<'a> CentralHandle<'a> {
    fn new(opener: &'a dyn Fn() -> CentralOpened) -> Self {
        Self {
            opener,
            opened: std::cell::OnceCell::new(),
        }
    }

    /// The configured pipeline, `None` when none is configured.
    fn get(&self) -> Result<Option<&dyn Pipeline>, &CentralError> {
        match self.opened.get_or_init(|| (self.opener)()) {
            Ok(primary) => Ok(primary.as_deref()),
            Err(error) => Err(error),
        }
    }

    /// The pipeline a command cannot run without.
    fn required(&self) -> Result<&dyn Pipeline, CentralError> {
        match self.get() {
            Ok(Some(primary)) => Ok(primary),
            Ok(None) => Err(not_configured()),
            Err(error) => Err(clone_error(error)),
        }
    }
}

fn clone_error(error: &CentralError) -> CentralError {
    match error {
        CentralError::Unavailable(message) => CentralError::Unavailable(message.clone()),
        CentralError::NotConfigured(message) => CentralError::NotConfigured(message.clone()),
        other => CentralError::Store(other.to_string()),
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
    let central = CentralHandle::new(central);
    if command.needs_central()
        && let Err(error) = central.required()
    {
        return refuse_central(err, &error);
    }
    if command.is_admin() {
        log_operation(&command, err, &fleet_config::ctx::Ctx::from_env());
        return match central.required() {
            Ok(primary) => execute_admin(command, primary, out, err),
            Err(error) => refuse_central(err, &error),
        };
    }
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
        &central,
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
    let central = CentralHandle::new(central);
    if command.needs_central()
        && let Err(error) = central.required()
    {
        return refuse_central(err, &error);
    }
    execute(command, store, identity, &central, stdin, out, err, ctx)
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

/// The central plane a merged read of `namespace` consults: only when the
/// host has the namespace's definition cached, and only then is central
/// opened. A configured central store that cannot be opened is a refusal,
/// never a silent local-only read.
fn central_for<'a>(
    store: &LocalStore,
    namespace: &str,
    central: &'a CentralHandle<'_>,
) -> Result<Option<&'a dyn Pipeline>, CentralError> {
    match sync::lookup(store, namespace) {
        Ok(sync::Lookup::Known(_)) => {}
        _ => return Ok(None),
    }
    central.get().map_err(clone_error)
}

fn refuse_read(err: &mut dyn Write, error: &read::ReadError) -> i32 {
    let _ = writeln!(err, "error: {}: {error}", error.code());
    match error {
        read::ReadError::Central(central) => central_exit(central),
        _ if error.is_unavailable() => EXIT_UNAVAILABLE,
        _ => EXIT_REFUSED,
    }
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
        Command::Sync { archive: forced } => {
            let configured = match blob_plane_from_env() {
                Ok(configured) => configured,
                Err(error) => return refuse_central(err, &error),
            };
            let plane = configured
                .as_ref()
                .map(|(uploader, partition)| crate::sweep::BlobPlane {
                    uploader: uploader.as_ref(),
                    partition: partition.clone(),
                });
            let report = match sync::sync_with_blobs(
                store,
                identity,
                primary,
                identity.owner_user_id,
                &mut sleep,
                plane.as_ref(),
            ) {
                Ok(report) => report,
                Err(error) => {
                    let _ = writeln!(err, "error: {}: {error}", error.code());
                    return match &error {
                        crate::sweep::SweepError::Central(central) => central_exit(central),
                        crate::sweep::SweepError::Local(_) => EXIT_REFUSED,
                    };
                }
            };
            if let Err(error) = write_sync_report(out, &report) {
                return finish(Err(error), err);
            }
            let now = read::now_ms();
            let due = match archive::due(store, identity, now) {
                Ok(due) => due,
                Err(error) => return refuse(err, &error),
            };
            if !(forced || due) {
                return finish(Ok(()), err);
            }
            match archive::checkpoint(store, identity, primary, now) {
                Ok(checkpoint) => {
                    let _ = writeln!(err, "note: {}", checkpoint.protection_gap());
                    write_checkpoint(out, &checkpoint)
                }
                Err(error) => return refuse_archive(err, &error),
            }
        }
        Command::Rebase { namespace, key } => {
            let definition =
                match crate::central::resolve(primary, crate::central::REGISTRY_ID, &namespace) {
                    Ok(definition) => definition,
                    Err(error) => return refuse_central(err, &error),
                };
            match crate::sweep::rebase(store, identity, primary, &definition, &namespace, &key) {
                Ok(rebased) => match rebased.requeued {
                    Some(version) => {
                        writeln!(out, "rebased onto={} requeued={version}", rebased.onto)
                    }
                    None => writeln!(out, "rebased onto={} requeued=none", rebased.onto),
                },
                Err(error) => {
                    let _ = writeln!(err, "error: {}: {error}", error.code());
                    return match &error {
                        crate::sweep::SweepError::Central(central) => central_exit(central),
                        crate::sweep::SweepError::Local(_) => EXIT_REFUSED,
                    };
                }
            }
        }
        Command::RestoreArchive => match archive::restore(store, identity, primary) {
            Ok(restored) => write_restored(out, &restored),
            Err(error) => return refuse_archive(err, &error),
        },
        command @ (Command::NamespaceDefine { .. } | Command::NamespaceList) => {
            return execute_admin(command, primary, out, err);
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

/// The central admin verbs, against an opened central plane.
fn execute_admin(
    command: Command,
    primary: &dyn Pipeline,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    use crate::central::{self, Namespace, REGISTRY_ID, Scope};
    let written = match command {
        Command::NamespaceDefine {
            namespace,
            scope,
            global_flag,
            blob_threshold,
        } => {
            let existing = match central::namespaces(primary, REGISTRY_ID) {
                Ok(defined) => defined
                    .into_iter()
                    .find(|definition| definition.namespace == namespace),
                Err(error) => return refuse_central(err, &error),
            };
            let mut definition = match (existing, scope) {
                (Some(existing), Some(scope)) => {
                    let changes_kind = matches!(
                        (&existing.scope, &scope),
                        (Scope::Shared(_), Scope::Private) | (Scope::Private, Scope::Shared(_))
                    );
                    if changes_kind {
                        let _ = writeln!(
                            err,
                            "error: namespace_scope_change: `{namespace}` is {}; changing a namespace between shared and private would strand its keys",
                            describe_scope(&existing.scope)
                        );
                        return EXIT_REFUSED;
                    }
                    Namespace { scope, ..existing }
                }
                (Some(existing), None) => existing,
                (None, Some(Scope::Shared(scope_id))) => Namespace::shared(&namespace, &scope_id),
                (None, Some(Scope::Private)) => Namespace::private(&namespace),
                (None, None) => {
                    let _ = writeln!(
                        err,
                        "error: usage: `{namespace}` is not defined yet: give --shared SCOPE or --private"
                    );
                    return EXIT_USAGE;
                }
            };
            definition.global_flag |= global_flag;
            if let Some(bytes) = blob_threshold {
                definition.blob_threshold = bytes;
            }
            // `define_namespace` stores one past the previous epoch, so
            // hosts that cached the old definition see the change.
            match central::define_namespace(primary, &definition) {
                Ok(epoch) => writeln!(
                    out,
                    "namespace={} scope={} epoch={epoch}",
                    definition.namespace,
                    describe_scope(&definition.scope)
                ),
                Err(error) => return refuse_central(err, &error),
            }
        }
        Command::NamespaceList => match central::namespaces(primary, REGISTRY_ID) {
            Ok(defined) => defined.iter().try_for_each(|definition| {
                writeln!(
                    out,
                    "{}\t{}\t{}\t{}\t{}",
                    definition.namespace,
                    describe_scope(&definition.scope),
                    definition.policy_epoch,
                    definition.blob_threshold,
                    if definition.global_flag {
                        "global"
                    } else {
                        "-"
                    }
                )
            }),
            Err(error) => return refuse_central(err, &error),
        },
        _ => {
            let _ = writeln!(err, "error: usage: this verb is not a central admin verb");
            return EXIT_USAGE;
        }
    };
    finish(written, err)
}

fn describe_scope(scope: &crate::central::Scope) -> String {
    match scope {
        crate::central::Scope::Shared(scope_id) => format!("shared:{scope_id}"),
        crate::central::Scope::Private => "private".to_string(),
    }
}

fn refuse_archive(err: &mut dyn Write, error: &archive::ArchiveError) -> i32 {
    let _ = writeln!(err, "error: {}: {error}", error.code());
    match error {
        archive::ArchiveError::Central(central) => central_exit(central),
        _ => EXIT_REFUSED,
    }
}

fn write_checkpoint(out: &mut dyn Write, checkpoint: &archive::Checkpoint) -> std::io::Result<()> {
    writeln!(
        out,
        "archive at_ms={} archived={} unprotected={}",
        checkpoint.at_ms,
        checkpoint.archived(),
        checkpoint.unprotected().count()
    )?;
    for key in checkpoint.unprotected() {
        writeln!(
            out,
            "unprotected\t{}\t{}\t{}",
            key.namespace,
            key.key,
            archive::PROTECTION_UNAVAILABLE
        )?;
    }
    Ok(())
}

fn write_restored(out: &mut dyn Write, restored: &archive::Restored) -> std::io::Result<()> {
    writeln!(
        out,
        "restored checkpoint_at_ms={} adopted={} kept_local={} unprotected={}",
        restored.checkpoint_at_ms,
        restored.adopted.len(),
        restored.kept_local.len(),
        restored.unprotected.len()
    )?;
    for (namespace, key) in &restored.kept_local {
        writeln!(out, "kept_local\t{namespace}\t{key}")?;
    }
    for (namespace, key) in &restored.unprotected {
        writeln!(
            out,
            "unprotected\t{namespace}\t{key}\t{}",
            archive::PROTECTION_UNAVAILABLE
        )?;
    }
    Ok(())
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
    central: &CentralHandle<'_>,
    stdin: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
    ctx: &fleet_config::ctx::Ctx,
) -> i32 {
    log_operation(&command, err, ctx);
    if command.needs_central() {
        return match central.required() {
            Ok(primary) => execute_central(command, store, identity, primary, stdin, out, err),
            Err(error) => refuse_central(err, &error),
        };
    }
    if let Some(namespace) = local_namespace(&command)
        && let Err(error) = sync::require_known(store, namespace)
    {
        return refuse(err, &error);
    }
    let written = match command {
        Command::Help(text) => out.write_all(text.as_bytes()),
        Command::Get {
            namespace,
            key,
            allow_stale,
            verbose,
            ..
        } => {
            // The overlay answers first, without touching central.
            match store.get_to(identity, &namespace, &key, out) {
                Ok(true) => {
                    if verbose {
                        let _ = writeln!(err, "fleet-kv: source=local");
                    }
                    return finish(Ok(()), err);
                }
                Ok(false) => {}
                Err(error) => return refuse(err, &error),
            }
            let primary = match central_for(store, &namespace, central) {
                Ok(primary) => primary,
                Err(code) => return refuse_central(err, &code),
            };
            let reader = read::Reader {
                store,
                identity,
                central: primary,
                allow_stale,
                now_ms: read::now_ms(),
            };
            match reader.get(&namespace, &key, out) {
                Ok(read::Found::Value(source)) => {
                    if verbose || matches!(source, read::Source::Snapshot { .. }) {
                        let _ = writeln!(err, "fleet-kv: source={source}");
                    }
                    Ok(())
                }
                Ok(read::Found::Absent | read::Found::Masked) => {
                    let _ = writeln!(
                        err,
                        "error: not_found: no live value for `{key}` in `{namespace}`"
                    );
                    return EXIT_REFUSED;
                }
                Err(error) => return refuse_read(err, &error),
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
            allow_stale,
            verbose,
            ..
        } => {
            let primary = match central_for(store, &namespace, central) {
                Ok(primary) => primary,
                Err(code) => return refuse_central(err, &code),
            };
            let reader = read::Reader {
                store,
                identity,
                central: primary,
                allow_stale,
                now_ms: read::now_ms(),
            };
            match reader.list(&namespace, include_tombstones, verbose) {
                Ok(list) => {
                    if let Some(reason) = &list.central_skipped {
                        let _ = writeln!(
                            err,
                            "fleet-kv: central rows left out (--allow-stale): {reason}"
                        );
                    }
                    list.entries.iter().try_for_each(|entry| {
                        writeln!(
                            out,
                            "{}\t{}\t{}",
                            entry.key,
                            entry.version,
                            entry.origin.as_str()
                        )
                    })
                }
                Err(error) => return refuse_read(err, &error),
            }
        }
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
        Command::Sync { .. }
        | Command::RestoreArchive
        | Command::Rebase { .. }
        | Command::NamespaceDefine { .. }
        | Command::NamespaceList => {
            let _ = writeln!(err, "error: usage: this verb needs the central store");
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
            &|| Ok(None),
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
                &|| Ok(Some(Box::new(shared.clone()) as Box<dyn Pipeline>)),
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
        // The unswept key makes a first archive checkpoint due.
        let printed = text(&synced);
        assert!(
            printed.starts_with(
                "namespaces=1 committed=1 conflicts=0 waiting=1\nwaiting\telsewhere\tagent\tunknown_namespace\narchive at_ms="
            ),
            "{printed}"
        );
        assert!(
            printed.ends_with(" archived=1 unprotected=0\n"),
            "{printed}"
        );
        assert!(
            synced.err.contains("not a recovery point"),
            "{}",
            synced.err
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
    fn central_cli_backup_archives_and_restore_archive_adopts() {
        let central = Central::new();
        let lost = LocalStore::in_memory().expect("store");
        assert_eq!(central.invoke(&lost, "owner", &["sync"], b"").code, EXIT_OK);
        // Nothing unswept and checkpoint not forced: no archive line.
        let quiet = central.invoke(&lost, "owner", &["sync"], b"");
        assert_eq!(
            text(&quiet),
            "namespaces=1 committed=0 conflicts=0 waiting=0\n"
        );
        assert_eq!(
            invoke(
                &lost,
                &["set", "--namespace", "status", "agent", "working"],
                b""
            )
            .code,
            EXIT_OK
        );
        let backup = central.invoke(&lost, "owner", &["backup"], b"");
        assert_eq!(backup.code, EXIT_OK, "{}", backup.err);
        // The key swept, so the forced checkpoint archives an empty set.
        assert!(
            text(&backup).contains("archive at_ms="),
            "{}",
            text(&backup)
        );
        assert!(text(&backup).ends_with(" archived=0 unprotected=0\n"));
        let usage = central.invoke(&lost, "owner", &["restore", "--archive", "extra"], b"");
        assert_eq!(usage.code, EXIT_USAGE);
        let missing = central.invoke(&lost, "nobody", &["restore", "--archive"], b"");
        assert_eq!(missing.code, EXIT_REFUSED);
        assert!(
            missing.err.starts_with("error: archive_not_found"),
            "{}",
            missing.err
        );
        // Without central, the archive verbs refuse before opening the store.
        let unconfigured = invoke(&lost, &["backup"], b"");
        assert_eq!(unconfigured.code, EXIT_UNAVAILABLE);
        assert!(
            unconfigured
                .err
                .starts_with("error: central_not_configured"),
            "{}",
            unconfigured.err
        );
        let restored_fresh = LocalStore::in_memory().expect("fresh");
        let restored = central.invoke(&restored_fresh, "owner", &["restore", "--archive"], b"");
        assert_eq!(restored.code, EXIT_OK, "{}", restored.err);
        assert!(
            text(&restored).starts_with("restored checkpoint_at_ms="),
            "{}",
            text(&restored)
        );
        assert!(text(&restored).ends_with(" adopted=0 kept_local=0 unprotected=0\n"));
    }

    #[test]
    fn central_cli_namespace_define_list_and_hosts_follow() {
        let central = Central::new();
        let store = LocalStore::in_memory().expect("store");
        let listed = central.invoke(&store, "admin", &["namespace", "list"], b"");
        assert_eq!(listed.code, EXIT_OK, "{}", listed.err);
        assert_eq!(text(&listed), "status\tshared:fleet-status\t1\t262144\t-\n");

        let defined = central.invoke(
            &store,
            "admin",
            &[
                "namespace",
                "define",
                "notes",
                "--private",
                "--blob-threshold",
                "4096",
            ],
            b"",
        );
        assert_eq!(defined.code, EXIT_OK, "{}", defined.err);
        assert_eq!(text(&defined), "namespace=notes scope=private epoch=1\n");

        // Defining again keeps the scope, adds the flag and raises the epoch.
        let flagged = central.invoke(
            &store,
            "admin",
            &["namespace", "define", "status", "--global-flag"],
            b"",
        );
        assert_eq!(flagged.code, EXIT_OK, "{}", flagged.err);
        assert_eq!(
            text(&flagged),
            "namespace=status scope=shared:fleet-status epoch=2\n"
        );
        let listed = central.invoke(&store, "admin", &["namespace", "list"], b"");
        assert_eq!(
            text(&listed),
            "notes\tprivate\t1\t4096\t-\nstatus\tshared:fleet-status\t2\t262144\tglobal\n"
        );

        // Shared to private would strand the shared keys.
        let changed = central.invoke(
            &store,
            "admin",
            &["namespace", "define", "status", "--private"],
            b"",
        );
        assert_eq!(changed.code, EXIT_REFUSED);
        assert!(
            changed.err.starts_with("error: namespace_scope_change:"),
            "{}",
            changed.err
        );
        // A new namespace needs a scope.
        let bare = central.invoke(&store, "admin", &["namespace", "define", "fresh"], b"");
        assert_eq!(bare.code, EXIT_USAGE, "{}", bare.err);

        // A host's next sync learns the new namespace.
        let synced = central.invoke(&store, "owner", &["sync"], b"");
        assert_eq!(synced.code, EXIT_OK, "{}", synced.err);
        assert!(
            text(&synced).starts_with("namespaces=2 "),
            "{}",
            text(&synced)
        );
        assert_eq!(
            invoke(&store, &["set", "--namespace", "notes", "k", "v"], b"").code,
            EXIT_OK
        );
    }

    #[test]
    fn central_cli_namespace_verbs_never_open_the_local_store() {
        let central = Central::new();
        let shared = SharedPrimary(central.primary.clone());
        let opener =
            || -> CentralOpened { Ok(Some(Box::new(shared.clone()) as Box<dyn Pipeline>)) };
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_opening_with_central(
            &args(&["namespace", "define", "board", "--shared", "fleet-board"]),
            || -> Opened { panic!("namespace define opened the local store") },
            || -> RestoreTarget { panic!("namespace define asked for a restore target") },
            &opener,
            &mut &b""[..],
            &mut out,
            &mut err,
        );
        assert_eq!(code, EXIT_OK, "{}", String::from_utf8_lossy(&err));
        assert_eq!(out, b"namespace=board scope=shared:fleet-board epoch=1\n");
        let mut out = Vec::new();
        let code = run_opening_with_central(
            &args(&["namespace", "list"]),
            || -> Opened { panic!("namespace list opened the local store") },
            || -> RestoreTarget { panic!("namespace list asked for a restore target") },
            &|| Ok(None),
            &mut &b""[..],
            &mut out,
            &mut err,
        );
        assert_eq!(code, EXIT_UNAVAILABLE);
        assert!(out.is_empty());
    }

    #[test]
    fn central_cli_namespace_usage_errors_touch_nothing() {
        let store = LocalStore::in_memory().expect("store");
        for words in [
            &["namespace"][..],
            &["namespace", "drop", "x"][..],
            &["namespace", "list", "extra"][..],
            &["namespace", "define"][..],
            &["namespace", "define", "x", "--shared", "a", "--private"][..],
            &["namespace", "define", "x", "--shared", "private:abc"][..],
            &[
                "namespace",
                "define",
                "x",
                "--private",
                "--blob-threshold",
                "0",
            ][..],
            &[
                "namespace",
                "define",
                "x",
                "--private",
                "--blob-threshold",
                "1048577",
            ][..],
            &["namespace", "list", "--private"][..],
            &["--global", "namespace", "list"][..],
        ] {
            let outcome = invoke(&store, words, b"");
            assert_eq!(outcome.code, EXIT_USAGE, "{words:?}: {}", outcome.err);
        }
        let help = invoke(&store, &["namespace", "--help"], b"");
        assert_eq!(help.code, EXIT_OK);
        assert!(text(&help).starts_with("usage: fleet kv namespace define"));
        // Without a central store the admin verbs are unavailable.
        let unconfigured = invoke(&store, &["namespace", "list"], b"");
        assert_eq!(unconfigured.code, EXIT_UNAVAILABLE, "{}", unconfigured.err);
    }

    #[test]
    fn central_cli_rebase_unblocks_a_conflicted_key() {
        let central = Central::new();
        let (first, second) = (
            LocalStore::in_memory().expect("store"),
            LocalStore::in_memory().expect("store"),
        );
        assert_eq!(
            central
                .invoke(
                    &first,
                    "first",
                    &["set", "--namespace", "status", "claim", "one"],
                    b""
                )
                .code,
            EXIT_OK
        );
        assert_eq!(
            central
                .invoke(
                    &second,
                    "second",
                    &["set", "--namespace", "status", "claim", "two"],
                    b""
                )
                .code,
            EXIT_OK
        );
        assert_eq!(
            central.invoke(&first, "first", &["sync"], b"").code,
            EXIT_OK
        );
        let lost = central.invoke(&second, "second", &["sync"], b"");
        assert!(
            text(&lost).starts_with("namespaces=1 committed=0 conflicts=1"),
            "{}",
            text(&lost)
        );
        let rebased = central.invoke(
            &second,
            "second",
            &["rebase", "--namespace", "status", "claim"],
            b"",
        );
        assert_eq!(rebased.code, EXIT_OK, "{}", rebased.err);
        assert_eq!(text(&rebased), "rebased onto=1 requeued=1\n");
        let won = central.invoke(&second, "second", &["sync"], b"");
        assert!(
            text(&won).starts_with("namespaces=1 committed=1 conflicts=0"),
            "{}",
            text(&won)
        );
        let unknown = central.invoke(
            &second,
            "second",
            &["rebase", "--namespace", "nowhere", "claim"],
            b"",
        );
        assert_eq!(unknown.code, EXIT_REFUSED);
        assert!(
            unknown.err.starts_with("error: unknown_namespace"),
            "{}",
            unknown.err
        );
        let usage = central.invoke(&second, "second", &["rebase", "--namespace", "status"], b"");
        assert_eq!(usage.code, EXIT_USAGE);
    }

    #[test]
    fn central_cli_plain_reads_merge_local_and_central() {
        let central = Central::new();
        let store = LocalStore::in_memory().expect("store");
        let me = |words: &[&str]| central.invoke(&store, "owner", words, b"");
        assert_eq!(
            me(&["set", "--global", "--namespace", "status", "shared", "c"]).code,
            EXIT_OK
        );
        // Before this host syncs, plain reads are local only.
        assert_eq!(
            me(&["get", "--namespace", "status", "shared"]).code,
            EXIT_REFUSED
        );
        assert_eq!(me(&["sync"]).code, EXIT_OK);
        let fell_through = me(&["get", "--verbose", "--namespace", "status", "shared"]);
        assert_eq!(fell_through.out, b"c", "{}", fell_through.err);
        assert!(
            fell_through.err.contains("source=central@1"),
            "{}",
            fell_through.err
        );
        assert_eq!(
            me(&["set", "--namespace", "status", "mine", "l"]).code,
            EXIT_OK
        );
        let listed = me(&["list", "--namespace", "status"]);
        assert_eq!(text(&listed), "mine\t1\tlocal\nshared\t1\tcentral\n");
        assert_eq!(
            me(&[
                "get",
                "--global",
                "--allow-stale",
                "--namespace",
                "status",
                "x"
            ])
            .code,
            EXIT_USAGE
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
