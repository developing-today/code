# The CLI

The `mcpx` command line, in three parts: how a session with it
normally goes, the two kinds of flag it accepts, and where the full
command reference lives. The CLI is one of the
[client interfaces](./interfaces.md#client-interfaces) — everything
it does goes through the daemon's `/v1` API, and anything a person
can do here, a script or a plugin can do there.

<a id="cli"></a>

## The CLI

### The loop

Almost every session is the same four steps (the same loop
`AGENTS.md` teaches agents):

1. `mcpx ls` — which namespaces exist. Cheap; run it whenever.
2. `mcpx search <words>` — find a tool by name or description.
3. `mcpx types <namespace>` — load signatures for that namespace
   only (or `mcpx types <namespace>.<tool>` for one tool, which is
   far cheaper). `mcpx catalog` fits every namespace into a token
   budget when you are still deciding.
4. `mcpx run <name>` or `mcpx exec '<code>'` — do the work.

Do not run `mcpx types` for every namespace: loading one costs a
few hundred tokens, loading all of them costs thousands and
defeats the point of keeping schemas out of context until asked.

### Output a program can read

Human output is for humans. Anything that has to be parsed gets
`--json` instead:

```sh
mcpx --json ls
mcpx --json run screenshot
```

`--json` returns one document — for a script run, the result, the
logs, the exit code and the duration. Never parse the human tables.

### Bare invocation, and a runnable first argument

Bare `mcpx` opens the TUI (the `tui` browser) when there is a
terminal to draw on, and prints help when there is not — a pipe
gets the old behaviour.

A first argument that is obviously a script runs without naming a
command: a path ending in `.ts`, `.js`, `.mts` or `.mjs`, any
existing file, or text that is obviously source (it contains
`(){};`, `await `, `console.`, `tools.` or `=>`) is handed to
`run` or `exec` as appropriate:

```sh
mcpx ./report.ts
mcpx 'await chrome_devtools.list_pages({})'
```

This is deliberately conservative: a bare word that is not a file
fails as an unknown command, because guessing that `mcpx serach`
was a snippet would replace a clear error with a baffling one.

### Exit codes worth knowing

| code | meaning                                                                                                                                                                                                                                                                              |
| ---- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 0    | success                                                                                                                                                                                                                                                                              |
| 1    | the command failed; the error is on stderr                                                                                                                                                                                                                                           |
| 2    | an unknown command, an unknown global flag, or a settings conflict — including a contradiction in a config file, which is reported once, before dispatch                                                                                                                             |
| 75   | `EX_TEMPFAIL`: a call stopped to ask a question (an elicitation). Not a failure, and not a cue to retry: the call continues as a task — answer with `mcpx elicit answer …`, then collect the original call's result with `mcpx task result <id>`. Retrying would start a second call |

### The man page and shell completions

Both are generated from the same command declarations as
everything else, so they cannot drift from the binary:

```sh
mcpx man
mcpx completion bash   # also: zsh, fish
```

The Nix package installs both for you: the man page to
`share/man/man1/mcpx.1`, and completions for all three shells.

<a id="cli-arguments"></a>

## CLI arguments

Flags come in **two kinds**, and most confusion about mcpx flags
is really about which kind one is looking at.

### Kind 1: hand-parsed global flags, before the subcommand

A short list, parsed by hand in `cmd/mcpx/main.go` before any
subcommand is chosen. These, and only these, have fixed global
meanings:

| flag              | what it does                                          |
| ----------------- | ----------------------------------------------------- |
| `--json`          | one machine-readable document instead of human output |
| `--tui`           | open the TUI browser (also a subcommand)              |
| `--profile <a,b>` | add profiles to the active set                        |
| `--skip-default`  | narrow the active set to exactly the named profiles   |
| `--all-profiles`  | activate every profile                                |
| `--config <file>` | use this config file alone                            |
| `--version`, `-v` | print the version and exit                            |
| `--help`, `-h`    | print help and exit                                   |

One extension to that list: a setting that **every** command
accepts is also accepted as a global flag, so
`mcpx --plumbing-strict-unknown-keys ls` means what
`mcpx ls --plumbing-strict-unknown-keys` does. An unrecognised
global flag is an error (exit 2), never a guess.

### Kind 2: registry flags, bound per command

Every other flag is generated from the settings registry and
**bound per command**: `parseFlags` calls `BindFlags` for the
command being run, which binds exactly the settings that command
reads (`internal/cli/root.go`, `internal/settings/bind.go`). A
command accepts the settings it reads — no more, no less — so the
flag list of `mcpx run` and the flag list of `mcpx ls` are
different lists, on purpose. Flags for internals are marked
`[plumbing]` in help rather than hidden.

Three consequences worth knowing:

- **The full inventory is generated, not listed here.** Every
  setting, its flag, its environment variable, its default and its
  scope are in [configuration.md](./configuration.md), which a
  test keeps identical to the registry; the environment names are
  in [environment.md](./environment.md). `mcpx help <command>`
  shows one command's own flags; `mcpx settings` shows the
  resolved values and where each came from.
- **Aliases are spellings of one setting.** A setting may publish
  shorter aliases (`--runtime` for `--script-runtime`). Giving two
  spellings of the same setting on one command line is an error
  that names both — they are the same setting, and mcpx will not
  guess which value you meant.
- **Flag names are derived, not chosen per setting.** The flag is
  derived mechanically from the setting's dotted path
  (`daemon.leaseTTL` becomes `--daemon-lease-ttl`), so the spelling
  in [configuration.md](./configuration.md) is authoritative;
  do not guess a spelling the table does not show.

Where a value can come from at all — built-in default, config
files, environment, presets, flags — and which source wins, is
the precedence chain in
[configuration.md](./configuration.md) ("Where a value can come
from"). This page only adds: a contradiction between sources that
the chain cannot resolve is reported once, up front, with exit
code 2, rather than by whichever command happens to read the
value first.

<a id="cli-commands"></a>

## CLI commands

The command table is **generated, and this page does not copy
it.** Every command is declared once in `Commands()`
(`internal/cli/manual.go`) — name, aliases, group, usage, detail,
examples — and from that declaration come `mcpx help`, the man
page, the shell completions, and the generated **CLI commands**
table in [parity.md](./parity.md), which lists each command
beside the operation it reaches. A test
(`TestEveryDispatchedCommandIsDeclared`) holds the declaration
and the binary's dispatch table equal, in both directions: no
dispatched command is missing from the list, and nothing on the
list fails to run. That parity is recent — `serve`, `api`,
`registry`, `adapter`, `openapi`, `man` and `completion` were
once dispatched but undeclared, and therefore invisible to help,
the man page and completions; the test exists so that cannot
happen silently again.

What prose can usefully add over the generated table is the
grouping `mcpx help` presents, and what each group is for:

| group             | what is in it                                                                                                                                                     |
| ----------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **running**       | doing the work: `run`, `exec`, `scripts`, `skills`, `recipes`, `prompt`, `proxy`, `embeddings`, `feedback`                                                        |
| **discovery**     | finding out what exists before paying for its schemas: `ls`, `catalog`, `types`, `search`, `call`, `batch`, `client`, `schema`, `prompts`, `resources`, `openapi` |
| **daemon**        | the daemon's own lifecycle: `status`, `daemons`, `refresh`, `reload`, `upgrade`, `restart`, `stop`, `supervise`, `daemon`, `serve`                                |
| **configuration** | shaping mcpx itself: `config`, `settings`, `servers`, `secrets`, `init`, `registry`, `api`, `adapter`, plus `help`, `man` and `completion`                        |
| **inspection**    | looking at what happened: `diagnose`, `elicit`, `doctor`, `tui`, `explore`, `dashboard`, `interactions`, `log`, `stats`                                           |
| **operations**    | one generated command per `/v1` operation (for example `mcpx artifact list`) — these exist the moment an operation is declared, with flags from its parameters    |

For any one command, `mcpx help <command>` is the reference: it
is printed from the same declaration the table is generated
from, so the two cannot disagree.
