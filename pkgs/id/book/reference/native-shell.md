# id-native-shell

Source: https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-native-shell/src/main.rs

`id-native-shell` is the second binary in `pkgs/id`: a Bevy desktop shell that connects to one world on one node and renders it in a window. It is a workspace member of `pkgs/id` but **not** a default member, so a normal `id` build never builds it. Unlike the `id` binary, which is parsed by clap, this binary has a hand-rolled argument parser with no clap dependency. The parser's behaviour — including what it refuses — is the whole command-line surface.

For the terminal equivalent of the same connection, see [World commands: joining and playing](cli-worlds.md); for the browser equivalent, see [The web UI: pages and HTTP API](web-ui.md).

## Usage

The usage string, verbatim:

```text
usage: id-native-shell [--node <node-id> --capability <cap>] [--world <name>] [--addr <ip:port>]... [--no-relay]
```

## Arguments

| Argument | Value | Rule |
|---|---|---|
| `--node <node-id>` | 64-hex node ID | Must be given together with `--capability`. |
| `--capability <cap>` | Capability string | Must be given together with `--node`. |
| `--world <name>` | World name | Selects the world on the node. |
| `--addr <ip:port>` | Socket address | Repeatable. Each occurrence adds a direct address and skips discovery for it. |
| `--no-relay` | Flag, no value | Direct connections only; relay use is disabled. |

### The pairing rule

`--node` and `--capability` must be given together. Either one alone is an argument error: the usage text is printed to stderr and the process exits with status `2`.

There is no way to connect with a node ID and no capability, and no way to supply a capability without naming its node, from this command line.

### Environment

One environment variable is read by this binary:

| Name | Effect |
|---|---|
| `ID_SHELL_FILE_READ_ROOT` | When set, configures an additional restriction on the native shell's file-read root. When unset, the file-read root is not additionally restricted or configured by environment. |

## There is no `--help` and no `--version`

The parser recognises exactly the five arguments above. `--help` is not one of them, and neither is `--version`.

- `id-native-shell --help` is an error: usage text on stderr, exit status `2`.
- `id-native-shell --version` is an error the same way.
- Any unknown flag is an error the same way.

This is deliberate parser behaviour, not a missing feature flag: the hand-rolled parser has no help or version branch at all.

## Exit behaviour

| Outcome | Exit status |
|---|---|
| Argument error (unknown flag, `--help`, `--version`, unpaired `--node`/`--capability`) | `2`, usage text on stderr |
| Session-start failure | `2`, error on stderr |
| Clean window close | `0` |

`id-native-shell` is the only binary in the workspace with a hand-coded exit code. The `id` binary exits `1` on handler errors and `2` only on clap parse errors; here both argument errors and session-start failures exit `2`.

## Starting unconnected

With no target — no `--node`/`--capability` pair — the window still opens. It opens on an empty view and announces that it is not connected. Nothing is rendered from any world until a connected invocation supplies a node and a capability.

## Worked example

Toy data: a node whose ID is written here as `NODE_ID`, a guest capability written as `CAP`, and a world named `lobby`.

Connected, direct address, no relay:

```text
$ id-native-shell --node NODE_ID --capability CAP --world lobby --addr 192.0.2.10:11204 --no-relay
```

Connected, discovery left to defaults:

```text
$ id-native-shell --node NODE_ID --capability CAP
```

Unconnected (empty view):

```text
$ id-native-shell
```

Failures:

```text
$ id-native-shell --node NODE_ID
usage: id-native-shell [--node <node-id> --capability <cap>] [--world <name>] [--addr <ip:port>]... [--no-relay]
$ echo $?
2

$ id-native-shell --help
usage: id-native-shell [--node <node-id> --capability <cap>] [--world <name>] [--addr <ip:port>]... [--no-relay]
$ echo $?
2
```

The first failure is the pairing rule: `--capability` is missing. The second is the absence of a help flag: `--help` is an unknown flag to this parser.
