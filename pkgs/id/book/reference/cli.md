# The id command line

Source: `pkgs/id/src/main.rs` and `pkgs/id/crates/id-cli/src/cli/mod.rs` under https://github.com/developing-today/code/blob/main/pkgs/id/ — provenance only; this chapter is complete without following it.

`id` is a single binary (version `0.1.0`) described in its own help as "An iroh-based peer-to-peer file sharing CLI". This chapter covers the top-level invocation, the six global flags, logging configuration, and exit behaviour. The command families themselves are covered by the sibling reference chapters listed in the command-family map at the end.

## Invocation

```
id [GLOBAL FLAGS] [COMMAND] [COMMAND FLAGS] [ARGUMENTS]
id
id repl [NODE]
id --help
id --version
```

Three invocation rules decide what a bare or lightly-flagged `id` does:

1. **No subcommand starts the REPL.** Running `id` with no subcommand starts the interactive REPL (`src/main.rs`: `None => run_repl(None)`). The REPL is a separate, hand-written parser, not the clap parser documented here; see [The interactive REPL](repl.md).
2. **The remote-node positional rule.** For the file commands, if the first positional argument is a 64-character hex node ID, that argument is consumed as the remote node and the remaining arguments operate on that node. The rule, and its exceptions (`send` takes explicit node arguments; the search family uses `--node`), are specified in [File commands: put, get, find and kin](cli-files.md).
3. **`--no-relay` is per-command, not global.** There is no global `--no-relay`. Each command that can operate on a remote node defines its own `--no-relay` flag, which disables relay servers for that operation and restricts the connection to direct paths. It is defined on `serve`, on every file command, and on the search family.

`id --version` prints two lines: the short version (`id 0.1.0`) and a second line giving the provenance of the embedded defaults, in the form `defaults: file sha256=… mode=append` or the built-in equivalent. The two-line form is the long version installed by `src/main.rs`.

## Global flags

The six global flags are defined with clap `global = true`: each one is accepted at the top level and on every subcommand, before or after the subcommand name. Every command also accepts `-h`/`--help`, and the top level accepts `-V`/`--version`.

| Flag               | Argument          | Default                              | Meaning                                                                                                                                                                                                                                                    |
| ------------------ | ----------------- | ------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--debug`          | none (switch)     | off                                  | Enable debug logging. Takes precedence over `--log-level` and over every logging environment variable.                                                                                                                                                     |
| `--log-level`      | `LEVEL`           | none                                 | Set the log level. The help text lists `trace`, `debug`, `info`, `warn`, `error`; the value is not a clap value-enum, so any string reaches the tracing filter. Overrides the logging environment variables, but not `--debug`.                            |
| `-d`, `--data-dir` | `DIR`             | current directory                    | Base directory for `.iroh-store`, `.iroh-key` and `.iroh-serve.lock`. Created if it does not exist. The process changes its working directory into `DIR` at startup. Conflicts with `--new`.                                                               |
| `--new`            | `NAME` (optional) | none                                 | Create a new instance directory `.iroh/<NAME>/` under the current directory and change into it. A missing value means the empty string, which generates a random 8-hex-character name. Fails if the directory already exists. Conflicts with `--data-dir`. |
| `--groups-log`     | `FILE`            | `groups.jsonl` in the data directory | The group log read and appended to by `group`, `delegate` and `revoke`. A relative path is resolved relative to the data directory.                                                                                                                        |
| `--event-store`    | `DIR`             | `events` in the data directory       | The event store read and appended to by `doc`, `claim`, `talk` and `writer-sets`. A relative path is resolved relative to the data directory.                                                                                                              |

Behaviour notes:

- `--data-dir` is the mechanism for running several independent instances on one machine: each instance gets its own data directory, and client commands take the same flag to target a specific instance.
- `--new` validates its name before creating anything: a name containing `/`, `\` or `..` fails with `--new name must be a simple identifier, not a path`. A name whose directory `.iroh/<NAME>/` already exists fails with `instance directory already exists: … (use --data-dir to reuse an existing directory)`.
- A bare `--new` followed by a subcommand name is rewritten before parsing (`normalize_args` in `cli/mod.rs`): `id --new serve` becomes `id --new= serve`, so `serve` is parsed as the subcommand and not swallowed as the instance name. The empty value still means "random name".

Worked example (toy data):

```
$ id --new demo-node put notes.txt
$ id --new demo-node list
notes.txt
$ id --data-dir .iroh/demo-node list
notes.txt
```

The first command creates `.iroh/demo-node/`, stores `notes.txt` there, and the second lists it back through the same named instance. The third reaches the same store by naming the directory directly.

## Log-level resolution

The log level is resolved by hand-written code in `src/main.rs`, not by clap environment fallback. The order is fixed:

1. `--debug` — wins over everything; the level is `debug`.
2. `--log-level <LEVEL>` — the flag value.
3. `RUST_LOG` — if non-empty, it is used verbatim as a tracing `EnvFilter`, so per-module directives such as `debug,mainline::rpc=trace` keep their meaning.
4. `LOG_LEVEL` — the environment value.
5. `DEBUG` — if truthy, the level is `debug`. Truthy means: not empty, not `0`, and not `false` (compared case-insensitively).
6. Default: `warn`.

Blank values count as unset at every step: an empty or whitespace-only `--log-level`, `RUST_LOG` or `LOG_LEVEL` falls through to the next step.

After resolution, `src/main.rs` appends fixed per-module filters that cap a fixed list of noisy modules at `info`, unless the base filter is `trace` or the filter already names that module.

## Structured logging environment

Four further environment variables configure the log output itself. They are read by hand in `src/main.rs` and `src/structured_logging.rs`; none of them is a clap argument.

| Variable            | Values                                                        | Default             | Meaning                                                                                                                                                                                                                                                             |
| ------------------- | ------------------------------------------------------------- | ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ID_LOG_FORMAT`     | `text`, `json`                                                | `text`              | Output format. `json` emits one JSON object per line, with span and trace context when present.                                                                                                                                                                     |
| `ID_LOG_FILE`       | path                                                          | none (stderr only)  | A second destination: events are appended to this file in the same format as the console destination, without ANSI escapes. No size or retention limit is imposed; the writer follows rename-and-create external rotation and must not be used with `copytruncate`. |
| `ID_LOG_FILE_LEVEL` | filter                                                        | the console filter  | An independent filter for the file destination; it accepts anything a tracing `EnvFilter` accepts, for example `debug` or `id=trace,iroh=info`.                                                                                                                     |
| `ID_LOG_TZ`         | IANA name, `UTC`, `local`, or a fixed offset such as `+05:30` | host local timezone | Timezone for log timestamps. Unset and `local` both mean the host local zone.                                                                                                                                                                                       |

Failure behaviour for these variables is warn-and-continue: an unrecognised `ID_LOG_FORMAT` value prints `warning: ID_LOG_FORMAT value not recognized; logs use text format` on stderr and logging continues in text format; an unrecognised `ID_LOG_TZ` value prints `warning: ID_LOG_TZ value not recognized; log timestamps use UTC` and timestamps fall back to UTC; a log file that cannot be opened prints a warning naming the path and the error, and logging continues to stderr only.

## Exit behaviour

`main` returns an `anyhow::Result<()>`, and the exit code follows from that single fact plus clap's own error handling:

| Outcome                                                                                                    | Exit code | Output                             |
| ---------------------------------------------------------------------------------------------------------- | --------- | ---------------------------------- |
| Success                                                                                                    | 0         | The command's normal output.       |
| `--help`, `help <command>`, `--version`                                                                    | 0         | Help or version text on stdout.    |
| Handler error (any error returned by a command)                                                            | 1         | `Error: <anyhow chain>` on stderr. |
| Clap parse error (unknown subcommand or flag, missing required argument, conflicting flags, invalid value) | 2         | `error: …` plus usage on stderr.   |

There is no per-command exit-code table and no `process::exit` call anywhere in the `id` binary or in `id-cli`: every handler error exits 1, whatever the command. Value-parser failures are clap errors and exit 2; for example `--first 0` fails with `zero is not a valid limit: use at least 1`.

The REPL is the one exception to the handler-error row: inside a REPL session a failed command prints `error: <e>` and the session continues, and the REPL process exits 0 on `quit`, `exit`, `q`, Ctrl+D, or a second Ctrl+C. REPL startup failures (for example an invalid node ID) are handler errors and exit 1. The separate `id-native-shell` binary has its own hand-coded exit codes and is covered in [id-native-shell](native-shell.md).

Worked example (toy data):

```
$ id get not-there.txt
Error: file not found
$ echo $?
1
$ id put --bogus-flag notes.txt
error: unexpected argument '--bogus-flag' found
$ echo $?
2
```

## Command-family map

The parser defines 32 top-level commands and 113 leaf commands. The reference chapters divide them as follows:

| Family                          | Commands                                                                                                           | Chapter                                                            |
| ------------------------------- | ------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------ |
| File storage and retrieval      | `put`, `put-hash`, `send`, `get`, `get-hash`, `cat`, `list`, `migrate-tags`                                        | [File commands: put, get, find and kin](cli-files.md)              |
| Search                          | `find`, `search`, `show`, `peek`                                                                                   | [File commands: put, get, find and kin](cli-files.md)              |
| Tags                            | `tag set`, `tag del`, `tag list`, `tag search`                                                                     | [The tag family](cli-tags.md)                                      |
| Hosting and publications        | `serve`, `publish`, `unpublish`, `publications`                                                                    | [serve and publications](cli-serve.md)                             |
| Worlds (playing)                | `world invite`, `world join`, `world records`, `world mirror`, `world download`, and kin                           | [World commands: joining and playing](cli-worlds.md)               |
| Worlds (administering)          | `world install`, `world compile`, `world caps`, `world doctor`, and kin                                            | [World commands: administering and verifying](cli-worlds-admin.md) |
| Gallery, match-audit, walk      | `gallery …`, `match-audit …`, `walk …`                                                                             | [Gallery, match-audit and walk](cli-gallery-walk.md)               |
| Groups, documents, claims, keys | `group …`, `delegate`, `revoke`, `doc …`, `claim …`, `talk …`, `writer-sets …`, `id`, `key …`, `keyset …`, `peers` | [Groups, documents, claims and keys](cli-groups-identity.md)       |
| Hosted bridge                   | `hosted heads`, `hosted push`, `hosted pull`                                                                       | [Groups, documents, claims and keys](cli-groups-identity.md)       |
| Host mode                       | `host run`, `host list`, `host start`, `host stop`, `host archive`, `host restore`, `host pin`, `host wake-mode`, `host key`, `host export`, `host import`, `host drill`, `host snapshots`, `host inspect` | [Host mode and graph navigation](cli-host-nav.md)                  |
| Graph navigation                | `nav neighbors`, `nav backlinks`, `nav threads`, `nav history`, `nav path`                                         | [Host mode and graph navigation](cli-host-nav.md)                  |
| Interactive shell               | `repl` (alias `shell`), bare `id`                                                                                  | [The interactive REPL](repl.md)                                    |
| Desktop shell                   | the separate `id-native-shell` binary                                                                              | [id-native-shell](native-shell.md)                                 |

Cross-cutting configuration — `defaults.json`, the full environment variable list, and feature-gated builds — is covered in [Configuration and environment](configuration.md). The on-disk layout behind `--data-dir`, `--groups-log` and `--event-store` is covered in [Data stores and schemas](data-stores.md).
