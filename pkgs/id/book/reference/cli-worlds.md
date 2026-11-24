# World commands: joining and playing

Source: `pkgs/id/crates/id-cli/src/cli/world.rs` under https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-cli/src/cli/world.rs — provenance only; this chapter is complete without following it.

Every `id world` subcommand is a client. No `world` subcommand hosts a world: hosting is `id serve --world`, covered in [serve and publications](cli-serve.md). The commands in this chapter are the participant and authoring side: minting an invite, installing and compiling a program, reading records, joining and chatting, mirroring and downloading, and the local-only commands that scaffold, fork and inspect world directories on disk. The operator side — changing capabilities and profiles, drift, fixtures, replicas and journal files — is in [World commands: administering and verifying](cli-worlds-admin.md). The runtime model behind these commands is in [Worlds and the Roc guest runtime](../concepts/worlds.md) and [Capabilities, ceilings and providers](../concepts/capabilities.md).

Feature gating follows one rule: everything parses, and a missing feature fails at dispatch (disagreement CLI-10, resolved — gating was once inconsistent). The whole `world` tree parses in every build: in a build without the `world` feature, `id world --help` prints the full tree, but every `id world` invocation fails at runtime with `this build has no world support`, exit `1`. The four sandbox commands — `id world dev`, `id world measure`, `id world replay`, `id world verify` — likewise parse in every build and fail at dispatch in a build without `sandbox` with `this build has no sandbox support`, exit `1`; `id match-audit` follows the same pattern. The default build (`default = [sandbox, world]`) and both Nix packages (`id-web`, `id-lib`) include both features.

Exit behaviour, all `world` commands unless a command states otherwise: success exits `0`; a handler error prints `Error: <message>` to stderr and exits `1`; a clap parse error (unknown subcommand or flag, missing required argument, conflicting flags, bad value) prints the error and usage to stderr and exits `2`. There is no per-command exit-code table in the binary.

## Common flags

The flags below repeat in each world command's argument struct; they are not global flags. They are stated once here. Each command's own table lists only the flags specific to that command, plus its credential requirement where that differs.

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `<NODE>` | 64 hex characters | required positional | The host's node ID. With `join --via ssh` it is instead the SSH target (see `join`). |
| `--admin-token` | token | — (none) | The host's admin secret, as configured with `serve --world-admin-token`. Environment fallback: `ID_WORLD_ADMIN_TOKEN`. The value is never echoed; clap hides environment values for this flag. |
| `--capability` | capability string | — (none) | A guest capability minted by `world invite`. Environment fallback: `ID_WORLD_CAPABILITY`. The value is never echoed; clap hides environment values for this flag. |
| `--world` | name | host's default world | Which world on the host to use. Environment fallback: `ID_WORLD`. One command overrides this default: `world replica` defaults `--world` to `lobby` (see [World commands: administering and verifying](cli-worlds-admin.md)). |
| `--addr` | `IP:PORT` | — (none) | Direct socket address of the host; repeatable. Giving one skips discovery. |
| `--no-relay` | switch | off | Disable relay servers; direct connections only. |

A command needs the admin token, a capability, both, or neither, as its own section states. Token and capability values are secrets: this chapter names them and never prints a value. In the examples, `toy-cap-0001` and similar strings are invented toy credentials.

The toy host used throughout this chapter and [World commands: administering and verifying](cli-worlds-admin.md) is the invented node ID `0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef`.

## `invite`

Mints a guest capability on a world host. This is the only way a participant credential comes into existence, and it needs the host's admin token.

```
id world invite <NODE> --name <NAME> [--admin-token <TOKEN>] [OPTIONS]
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `<NODE>` | 64 hex characters | required | The host's node ID. |
| `--name` | string | required | Display name for the new participant. There is no default; omitting it is a clap error, exit `2`. |
| `--admin-token` | token | — (env `ID_WORLD_ADMIN_TOKEN`) | The host's admin secret. Required in effect: the host refuses the mint without it. |
| `--role` | `spectator\|observer\|player\|guest\|delegator\|builder\|moderator` | `guest` | Role preset. `spectator` sees the world but not the chat; `observer` also reads chat; `player` also inputs; `guest` also chats; `delegator` may also mint narrower capabilities. `builder` and `moderator` complete the set of seven accepted values. Any other value is a clap error, exit `2`. |
| `--expires-in` | duration, e.g. `12h`, `3d` | `7 days` | How long the invite works. The maximum is `30 days`. |
| `--uses` | integer | unlimited | How many committed actions the invite may take. Must be at least `1` when given. |

Behaviour: only the capability goes to stdout, so it can be captured into a variable or a file. Give that string to the participant; it is the `--capability` value (or the `ID_WORLD_CAPABILITY` environment value) for `records`, `mirror`, `download` and `join`. Losing it is losing the invite: re-invite rather than trying to recover it. The `--expires-in` and `--uses` bounds are checked locally, before the host is contacted.

Example — invite a toy participant named `toy-guest` as a player for 12 hours:

```
$ id world invite 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    --name toy-guest --role player --expires-in 12h --uses 100
```

Failures: a missing `--name` or an unknown `--role` value is a clap error, exit `2`. A wrong or missing admin token is refused by the host, exit `1`. An `--expires-in` value past the 30-day maximum, or a `--uses` of `0`, fails the local bounds check, exit `1`, before any connection is made.

## `install`

Uploads a compiled Wasm module built for the `id` Roc world ABI and activates it on the host.

```
id world install <NODE> <MODULE> [--admin-token <TOKEN>] [OPTIONS]
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `<NODE>` | 64 hex characters | required | The host's node ID. |
| `<MODULE>` | file path | required | The `.wasm` module to upload and activate. |
| `--admin-token` | token | — (env `ID_WORLD_ADMIN_TOKEN`) | The host's admin secret. |
| `--seed` | integer | `1` | Deterministic seed supplied to the guest initializer. Record it: `world replay` and `world verify` need the same seed later (see [World commands: administering and verifying](cli-worlds-admin.md)). |

Example — install a toy module with the default seed:

```
$ id world install 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    ./toy-world.wasm
```

Failures: a missing module file fails before any upload, exit `1`; the file is read and size-checked locally first, and a module over the world module size limit fails with `module exceeds <N> bytes`, exit `1`. A wrong admin token is refused by the host, exit `1`. A module not built for the Roc world ABI is refused by the host, exit `1`.

## `records`

Prints the world's structured records as one JSON object.

```
id world records <NODE> --capability <CAP> [OPTIONS]
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `<NODE>` | 64 hex characters | required | The host's node ID. |
| `--capability` | capability string | required (env `ID_WORLD_CAPABILITY`) | A guest capability for the world. |
| `--prefix` | string | — (none) | Only keys starting with this prefix. |

Example — read the records under a toy key prefix:

```
$ id world records 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    --capability toy-cap-0001 --prefix toy/
```

Failures: an expired, exhausted or unknown capability is refused by the host, exit `1`. Output on success is the JSON object on stdout; there is no text form.

## `mirror`

Replicates the world's records over iroh with an iroh-docs replica. It runs a local replica endpoint that accepts sync connections. It is not a world host: it serves the replica, not the world program.

```
id world mirror <NODE> --capability <CAP> [OPTIONS]
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `<NODE>` | 64 hex characters | required | The host's node ID. |
| `--capability` | capability string | required (env `ID_WORLD_CAPABILITY`) | A guest capability for the world. |
| `-f, --follow` | switch | off | Keep printing whenever the records change, instead of syncing once and printing the current records. |
| `--dir` | path | — (none) | Store the replica on disk so it survives between runs. Without it the replica is in memory for this run only. |
| `--timeout-secs` | integer, `0..=86400` | `30` | Give up after this many seconds without a completed sync. A value above `86400` is a clap error, exit `2`. |

Example — keep a persistent toy replica in `./toy-replica` and follow changes:

```
$ id world mirror 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    --capability toy-cap-0001 --dir ./toy-replica --follow
```

Failures: no completed sync inside `--timeout-secs` ends the command with an error, exit `1`. An invalid capability is refused by the host, exit `1`.

## `download`

Downloads the module backing the world's current state, verified by hash.

```
id world download <NODE> --capability <CAP> --output <FILE>
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `<NODE>` | 64 hex characters | required | The host's node ID. |
| `--capability` | capability string | required (env `ID_WORLD_CAPABILITY`) | A guest capability for the world. |
| `--output` | file path | required | Where to write the verified module bytes. Omitting it is a clap error, exit `2`. |

Behaviour: the command asks the host for the world's active module hash, downloads the module by that hash (verified by hash), writes the bytes to `--output`, and prints the module hash.

Example:

```
$ id world download 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    --capability toy-cap-0001 --output ./toy-downloaded.wasm
```

Failures: a world with no installed module fails with `world has no installed module to download`, exit `1`. A hash mismatch fails the command, exit `1`. An invalid capability is refused by the host, exit `1`.

## `join`

Joins a world and chats. Stdin lines are chat; `/input <hex>` sends opaque input. Server frames print as JSON lines on stdout.

```
id world join <NODE> --capability <CAP> [OPTIONS]
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `<NODE>` | 64 hex characters, or an SSH target with `--via ssh` | required | With `iroh`, the host's node ID. With `ssh`, `[user@]host[:port]` of the host's world SSH server. |
| `--capability` | capability string | required (env `ID_WORLD_CAPABILITY`) | A guest capability for the world. With `--via ssh` it is presented over the SSH channel. |
| `--after` | integer (sequence) | — (none) | Replay events after this sequence instead of receiving a full snapshot. |
| `--command-mode` | switch | off | Parse stdin lines as commands (`/chat`, `/records`, `/stats`, `/caps`, `/quit`, and the rest of the in-session command set) instead of sending every line as chat. |
| `--json` | switch | off | Emit local command responses as JSONL records on stdout. Requires `--command-mode`; giving it alone is a clap error, exit `2`. |
| `--via` | `iroh\|ssh` | `iroh` | How to reach the host: an iroh QUIC stream to the node ID, or an exec channel on the host's world SSH server, for networks and jump hosts that only pass SSH. |
| `--ssh-host-key` | fingerprint, `SHA256:...` | — (none) | With `--via ssh`: the host key fingerprint `id serve` prints. Without it, the key must be in `~/.ssh/known_hosts`. |

Example — join in command mode with JSONL responses, over SSH:

```
$ id world join toyuser@toyhost.example:2222 --capability toy-cap-0001 \
    --via ssh --command-mode --json
```

Example — a plain chat join over iroh, resuming after sequence 40:

```
$ id world join 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    --capability toy-cap-0001 --after 40
```

Failures: `--json` without `--command-mode` is a clap error, exit `2`. An unknown `--via` value is a clap error, exit `2`. An invalid capability is refused by the host, exit `1`. With `--via ssh`, a host key that matches neither `--ssh-host-key` nor `~/.ssh/known_hosts` fails the connection, exit `1`.

## `ssh-tunnel`

Reaches a host's world SSH server through iroh, so an ordinary `ssh` client can reach a world behind NAT. This command takes no capability and no admin token: the capability is the SSH password the `ssh` client presents.

```
id world ssh-tunnel <NODE> [--listen <ADDR> | --stdio] [OPTIONS]
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `<NODE>` | 64 hex characters | required | The host's node ID. |
| `--listen` | address | — (none) | Local address to listen on. Loopback only is recommended. Conflicts with `--stdio`. |
| `--stdio` | switch | off | Speak SSH on stdin/stdout, for use as an `ssh -o ProxyCommand`. Conflicts with `--listen`. |

Example — listen locally, then connect with a normal `ssh` client (the password is the guest capability):

```
$ id world ssh-tunnel 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    --listen 127.0.0.1:2222
$ ssh -p 2222 toy-world@127.0.0.1
```

Failures: `--listen` together with `--stdio` is a clap error, exit `2`. An unreachable host fails the tunnel, exit `1`.

## `compile`

Compiles Roc sources on the host and installs the result. The host rewrites the app's `pf: platform "..."` path to its staged copy of the world platform, so any relative platform path in the source works.

```
id world compile <NODE> <MAIN_FILE> [--admin-token <TOKEN>] [OPTIONS]
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `<NODE>` | 64 hex characters | required | The host's node ID. |
| `<MAIN_FILE>` | file path | required | The app's `main.roc`. |
| `-f, --file` | file path | — (none) | Additional source files; repeatable. Paths become file names on the host. |
| `--seed` | integer | random | Seed for the compiled program's initializer. Omitted means a random seed is chosen; contrast `install` and `dev`, where the seed defaults to `1`. |
| `--admin-token` | token | — (env `ID_WORLD_ADMIN_TOKEN`) | The host's admin secret. |
| `--native` | switch | off | Build a native worker (x64musl) instead of Wasm. The host must run with `--world-native` and a platform with native targets. |

Example — compile a toy app from its main file plus one extra source:

```
$ id world compile 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    ./toy-app/main.roc --file ./toy-app/helpers.roc --seed 7
```

Failures: a Roc compile error on the host fails the command, exit `1`. `--native` against a host that does not run with `--world-native` is refused, exit `1`. A wrong admin token is refused, exit `1`.

## `dev`

The offline authoring loop. It compiles a Roc world app locally and launches it in a disposable in-memory scratch world. It is fully offline: no host is contacted, nothing is installed into a named world, and the scratch world is discarded when the session ends. It prints one JSON report: the compiled module's provenance, the initial view, and the view after each `--input`.

This command exists only in `sandbox` builds; see the gating note at the top of this chapter.

```
id world dev <DIR> [OPTIONS]
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `<DIR>` | directory path | required | App directory holding the app's `main.roc` and its sibling `.roc` sources. |
| `--platform` | directory | the platform path `main.roc` declares, resolved against the app directory | World platform directory (or its `platform.roc` file). |
| `--roc` | binary | `ID_ROC_BIN`, then the bundled toolchain lookup the host compiler uses | Roc compiler binary. This flag has no clap environment fallback of its own; the default resolution consults `ID_ROC_BIN`. |
| `--seed` | integer | `1` | Deterministic seed for the scratch world's guest initializer. |
| `--input` | text | — (none) | Input to send to the scratch world after launch, as the dev guest; repeatable, in order. The view is reported after each input. |
| `--fixture` | file path | — (none) | Fixture to replay against the compiled candidate before launch; repeatable, in order. A fixture whose replayed records mismatch its recorded expectation blocks promotion: the candidate is not launched and the command fails. Fixtures recorded under an earlier module hash still replay (the cross-module regression mode) and the report says so. |
| `--fuzz` | file path | — (none) | Deterministic fuzz declaration file: a bounded, versioned JSON file naming a generated input alphabet and the invariants the compiled candidate must hold. Given, the command compiles the candidate and runs a fuzz campaign instead of launching a session. |
| `--fuzz-seed` | integer | `42` | Seed for the fuzz campaign's input generator. The same declaration, seed and case budget reproduce the same input sequences and the same first failure. |
| `--fuzz-cases` | integer | `32` | Bounded fuzz case budget. Cases are generated input sequences, each applied to a fresh scratch world. |
| `--watch` | switch | off | Watch the app's declared sources and hot-swap each debounced change into the scratch world. A failed or stale candidate leaves the last-good program running. |
| `--watch-max-cycles` | integer | `1000` | Bounded watch poll budget; the session ends on its own after this many polls. |
| `--watch-debounce-ms` | integer (milliseconds) | `150` | Debounce window for watch polls; change events inside the window coalesce into one candidate. |

Example — compile a toy app, launch it, and send two inputs:

```
$ id world dev ./toy-app --input "toy move one" --input "toy move two"
```

Example — run a toy fuzz campaign instead of a session:

```
$ id world dev ./toy-app --fuzz ./toy-fuzz.json --fuzz-seed 42 --fuzz-cases 32
```

Failures: a compile error fails the command, exit `1`, and no scratch world is launched. A fixture mismatch fails the command, exit `1`, and the candidate is not launched. In a build without the `sandbox` feature the subcommand does not exist: clap error, exit `2`.

## `caps` (read form)

Shows a world's capabilities. The change form (`--grant`, `--revoke`, `--ceiling`) is in [World commands: administering and verifying](cli-worlds-admin.md).

```
id world caps <NODE> [--capability <CAP>] [--admin-token <TOKEN>]
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `<NODE>` | 64 hex characters | required | The host's node ID. |
| `--capability` | capability string | — (env `ID_WORLD_CAPABILITY`) | A guest capability. Sufficient on its own for the read form. |
| `--admin-token` | token | — (env `ID_WORLD_ADMIN_TOKEN`) | The host's admin secret. Optional for the read form. |

Behaviour: with no `--grant`, `--revoke` or `--ceiling`, the command prints what the world has granted, what its program wants, and what it has used. It needs only a guest capability.

Example:

```
$ id world caps 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    --capability toy-cap-0001
```

Failures: an invalid capability is refused by the host, exit `1`.

## `profile` (read form)

Views a world's requested execution profile bounds. The write form (any limit flag) is in [World commands: administering and verifying](cli-worlds-admin.md).

```
id world profile <NODE> [--capability <CAP>] [--admin-token <TOKEN>]
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `<NODE>` | 64 hex characters | required | The host's node ID. |
| `--capability` | capability string | — (env `ID_WORLD_CAPABILITY`) | A guest capability; sufficient to inspect the profile. |
| `--admin-token` | token | — (env `ID_WORLD_ADMIN_TOKEN`) | The host's admin secret; the alternative credential for the read form. |

Behaviour: with no limit flag, the command prints the current persisted profile. It requires either `--admin-token` or a guest `--capability`.

Example:

```
$ id world profile 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    --capability toy-cap-0001
```

Failures: giving neither credential fails the command, exit `1`. An invalid credential is refused by the host, exit `1`.

## `new`

Scaffolds a new Roc world app directory. Local only; no host is contacted.

```
id world new <DIR> [--name <NAME>]
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `<DIR>` | directory path | required | Directory to create. It must be absent or empty. |
| `--name` | string | `world` | App name used in the README and the Wasm output. |

Behaviour: writes `main.roc`, `build.sh` and `README.md` into the directory. The platform path in the scaffold points at `../roc-world/platform.roc`, so create the directory next to a checkout of the `examples/roc-world` platform.

Example:

```
$ id world new ./toy-app --name toy-app
```

Failures: a directory that exists and is not empty fails the command, exit `1`, and nothing is written into it by this command.

## `fork`

Forks a durable world on disk into a new world. Local only; no host is contacted.

```
id world fork <SOURCE_DIR> <DIR> [OPTIONS]
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `<SOURCE_DIR>` | directory path | required | Directory of the world to fork (the one holding `journal.jsonl`). |
| `<DIR>` | directory path | required | Directory to create for the fork. It must be absent or empty. |
| `--name` | string (`a-z`, `0-9`, `-`, `_`) | destination directory's name | The new world's ID. The default is the ID a host serving that directory will open it under. |
| `--template` | switch | off | Mark the fork reusable as a template, recorded in its provenance (see `provenance` below). |
| `--json` | switch | off | Print one JSON object instead of text lines. |

Behaviour: copies the source world's journal and modules into the destination directory under a new world ID and records the fork provenance in the new journal. Only the journal and the modules are copied: no seed secret or other source file comes along. On success it prints `forked <DIR>`.

Example:

```
$ id world fork ./toy-world-dir ./toy-world-copy --name toy-world-copy
```

Failures: a destination that exists and is not empty fails the command, exit `1`. A source directory without a `journal.jsonl` fails the command, exit `1`.

## `provenance`

Prints a durable world's provenance from its journal. Local only; no host is contacted.

```
id world provenance <DIR> [--json]
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `<DIR>` | directory path | required | Directory of the world to inspect (the one holding `journal.jsonl`). |
| `--json` | switch | off | Print one JSON object instead of text lines. |

Behaviour: prints the world's ID and, for a fork, the world it was forked from, the module hash at the fork point, and whether it was marked a template.

Example:

```
$ id world provenance ./toy-world-copy
```

Failures: a directory without a readable `journal.jsonl` fails the command, exit `1`.

## `owner`

Prints a durable world's owner report: joins, on-disk storage, and which facts are unavailable and why. Local only: it reads the world directory, contacts no host, never wakes a world and writes nothing, so live-only facts (connected members, program runs) are reported unavailable. Only the user who owns the world directory, or root, may read it.

```
id world owner <DIR> [OPTIONS]
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `<DIR>` | directory path | required | Directory of the world to report on (the one holding `journal.jsonl`). |
| `--since` | duration, e.g. `12h`, `7d` | `7d` | The window the report covers, ending now. |
| `--json` | switch | off | Print one JSON object (schema version `1`) instead of text. |

Example:

```
$ id world owner ./toy-world-dir --since 12h --json
```

Failures: a directory the invoking user does not own (and is not root for) fails the command, exit `1`. A directory without a `journal.jsonl` fails the command, exit `1`.

## `list` (public directory form)

Lists the worlds a host serves. The admin form is in [World commands: administering and verifying](cli-worlds-admin.md).

```
id world list <NODE> [--json]
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `<NODE>` | 64 hex characters | required | The host's node ID. |
| `--admin-token` | token | — (env `ID_WORLD_ADMIN_TOKEN`) | Optional. Omit it to read the public directory; give it for the admin listing. |
| `--json` | switch | off | Print one JSON object instead of tab-separated lines. |

Behaviour: without `--admin-token` this reads the host's public directory: world names, loaded state and presence counts. It needs no credential.

Example:

```
$ id world list 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
```

Failures: an unreachable host fails the command, exit `1`.
