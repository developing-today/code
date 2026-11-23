# World commands: administering and verifying

Source: `pkgs/id/crates/id-cli/src/cli/world.rs` under https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-cli/src/cli/world.rs — provenance only; this chapter is complete without following it.

This chapter is the operator side of `id world`: changing a live world's capabilities, profile, isolation and file scope; diagnosing a setup; reconciling a world to a `world.toml`; checking sessions against the shell contract; recording, replaying and verifying fixtures offline; replicating a world durably; creating worlds; and managing a world directory's journal file. The participant side is in [World commands: joining and playing](cli-worlds.md).

The common per-command flags — `<NODE>`, `--admin-token` (environment fallback `ID_WORLD_ADMIN_TOKEN`), `--capability` (environment fallback `ID_WORLD_CAPABILITY`), `--world` (environment fallback `ID_WORLD`), `--addr` and `--no-relay` — are stated once in [World commands: joining and playing](cli-worlds.md) and are not repeated in the tables below. Each table below lists the flags specific to the command, plus its credential requirement where that is the point of the command.

Exit behaviour, all commands in this chapter unless a command states otherwise: success exits `0`; a handler error prints `Error: <message>` to stderr and exits `1`; a clap parse error prints the error and usage to stderr and exits `2`. Three commands define their own non-zero behaviour in source and state it in their sections: `doctor`, `journal check`, and the offline pair `replay` / `verify`. `measure` and `contract` also narrow their failure conditions, stated in their sections.

Feature gating follows the pattern documented in [World commands: joining and playing](cli-worlds.md) (disagreement CLI-10): `measure`, `replay` and `verify` are gated on the `sandbox` cargo feature at the parser level and do not exist in a build without it (clap error, exit `2`); every other command in this chapter parses in every build and fails at runtime with `this build has no world support`, exit `1`, in a build without the `world` feature.

The toy host used in the examples is the invented node ID `0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef`. Token and capability values in examples (`toy-cap-0001` and similar) are invented toy credentials; real values are never printed by these commands and are named, never valued, in this chapter.

## `caps` (grant, revoke and ceiling changes)

Changes a world's capabilities. The read form is in [World commands: joining and playing](cli-worlds.md).

```
id world caps <NODE> [--grant <CAP>]... [--revoke <CAP>]... [--ceiling <CAPS>] [--admin-token <TOKEN>]
```

| Flag            | Argument                                        | Default                        | Meaning                                                                               |
| --------------- | ----------------------------------------------- | ------------------------------ | ------------------------------------------------------------------------------------- |
| `<NODE>`        | 64 hex characters                               | required                       | The host's node ID.                                                                   |
| `--grant`       | capability name                                 | — (none)                       | Capability to grant; repeatable. `*` grants the whole catalog.                        |
| `--revoke`      | capability name                                 | — (none)                       | Capability to revoke; repeatable.                                                     |
| `--ceiling`     | capability names, comma-separated or repeatable | — (none)                       | Capability ceiling to set. Only known capabilities; `*` is not accepted in a ceiling. |
| `--admin-token` | token                                           | — (env `ID_WORLD_ADMIN_TOKEN`) | The host's admin secret. Required for any change.                                     |
| `--capability`  | capability string                               | — (env `ID_WORLD_CAPABILITY`)  | A guest capability. Sufficient for reading; never sufficient for a change.            |

Behaviour: any of `--grant`, `--revoke` or `--ceiling` turns the invocation into a change, and a change needs the admin token. A guest capability alone cannot change anything. The background model is in [Capabilities, ceilings and providers](../concepts/capabilities.md).

Example — grant one toy capability, revoke another, and set a two-capability ceiling:

```
$ id world caps 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    --grant toy.cap.read --revoke toy.cap.write \
    --ceiling toy.cap.read,toy.cap.chat
```

Failures: a change attempted without the admin token, or with only a guest capability, is refused, exit `1`. An unknown capability name in `--ceiling`, or `*` in `--ceiling`, is refused, exit `1`.

## `profile` (limit writes)

Persists a world's requested execution profile bounds. The read form is in [World commands: joining and playing](cli-worlds.md).

```
id world profile <NODE> [--wasm-fuel <N>] [--wasm-memory-bytes <N>] [...] [--admin-token <TOKEN>]
```

| Flag                        | Argument               | Default                        | Meaning                                                                             |
| --------------------------- | ---------------------- | ------------------------------ | ----------------------------------------------------------------------------------- |
| `<NODE>`                    | 64 hex characters      | required                       | The host's node ID.                                                                 |
| `--wasm-fuel`               | integer                | — (none)                       | Requested Wasm fuel limit per transaction/event. Visible alias: `--fuel`.           |
| `--wasm-memory-bytes`       | integer (bytes)        | — (none)                       | Requested Wasm linear memory in bytes. Visible alias: `--wasm-memory`.              |
| `--wasm-message-bytes`      | integer (bytes)        | — (none)                       | Requested Wasm message payload limit in bytes. Visible alias: `--wasm-message`.     |
| `--native-memory-bytes`     | integer (bytes)        | — (none)                       | Requested native address-space cap in bytes. Visible alias: `--native-memory`.      |
| `--native-cpu-seconds`      | integer (seconds)      | — (none)                       | Requested native CPU cap in seconds. Visible alias: `--native-cpu`.                 |
| `--native-call-deadline-ms` | integer (milliseconds) | — (none)                       | Requested native call deadline in milliseconds. Visible alias: `--native-deadline`. |
| `--admin-token`             | token                  | — (env `ID_WORLD_ADMIN_TOKEN`) | The host's admin secret. Required when any limit flag is given.                     |

Behaviour: giving any limit flag persists the new profile and requires the admin token. Giving none prints the current persisted profile (the read form). The limit flags take bytes, seconds and milliseconds as raw integers; contrast the serve-side runtime flags in [serve and publications](cli-serve.md), which take MiB and KiB units.

Example — set a toy fuel limit using the short alias:

```
$ id world profile 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    --fuel 5000000
```

Failures: a limit flag without the admin token is refused, exit `1`.

## `doctor`

Diagnoses a broken world setup and prints the fix for each problem. It takes no `<NODE>`: every check is local.

```
id world doctor [OPTIONS]
```

| Flag         | Argument                                        | Default                                                   | Meaning                                                                                                                                                                           |
| ------------ | ----------------------------------------------- | --------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--roc-bin`  | binary                                          | `ID_ROC_BIN`, then the pinned nightly, then `roc` on PATH | Roc compiler binary to check. Environment fallback: `ID_ROC_BIN`.                                                                                                                 |
| `--platform` | directory                                       | `ID_ROC_PLATFORM`, or `./examples/roc-world`              | World platform directory to check. Environment fallback: `ID_ROC_PLATFORM`.                                                                                                       |
| `--journal`  | file path                                       | — (none)                                                  | World journal (`journal.jsonl`) to check for format errors.                                                                                                                       |
| `--ceiling`  | capability names, comma-separated or repeatable | — (none)                                                  | Capability ceiling to check. Only known capabilities, no `*`. Without it, the ceiling check reports that none is configured.                                                      |
| `--profile`  | file path (JSON)                                | — (none)                                                  | Execution profile to check, as a JSON file whose fields are the execution-profile bounds, e.g. `{"wasm_fuel": 0}`. Without it, the profile check reports that none is configured. |
| `--json`     | switch                                          | off                                                       | Print one machine-readable report instead of text.                                                                                                                                |

Behaviour: checks the Roc toolchain, the platform build, the capability ceiling, the execution profile, the journal format and the clock. Each problem is printed with its fix.

Exit behaviour, defined in source: `doctor` exits non-zero only when a check FAILs. A warning does not fail the command.

Example:

```
$ id world doctor --journal ./toy-world-dir/journal.jsonl --json
```

Failures: a FAIL check exits `1`. Warnings alone exit `0`.

## `diff`

Shows the drift between a `world.toml` file and a live world. Read-only: it changes nothing on the host.

```
id world diff <NODE> --file <FILE> --capability <CAP>
```

| Flag           | Argument          | Default                              | Meaning                                                                                              |
| -------------- | ----------------- | ------------------------------------ | ---------------------------------------------------------------------------------------------------- |
| `<NODE>`       | 64 hex characters | required                             | The host's node ID.                                                                                  |
| `--file`       | file path         | required                             | Path to the `world.toml` to compare against.                                                         |
| `--capability` | capability string | required (env `ID_WORLD_CAPABILITY`) | A guest capability, enough to read the world. No admin token is needed or accepted for this command. |

Behaviour: prints the drift report and stays informational: drift alone does not fail the command. If the manifest's plan is one an `apply` would refuse (the schedule-horizon check), `diff` prints `warning: \`world apply\` would refuse this manifest: <error>`to stderr and still exits`0`.

Example:

```
$ id world diff 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    --file ./toy-world.toml --capability toy-cap-0001
```

Failures: a missing or malformed `world.toml` fails the command, exit `1`. An invalid capability is refused by the host, exit `1`. Drift itself is not a failure.

## `apply`

Reconciles a live world to a `world.toml`. Idempotent and journaled by the host.

```
id world apply <NODE> --file <FILE> --admin-token <TOKEN> --capability <CAP> [--dry-run]
```

| Flag            | Argument          | Default                               | Meaning                                                                                                                |
| --------------- | ----------------- | ------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `<NODE>`        | 64 hex characters | required                              | The host's node ID.                                                                                                    |
| `--file`        | file path         | required                              | Path to the `world.toml` to apply.                                                                                     |
| `--admin-token` | token             | required (env `ID_WORLD_ADMIN_TOKEN`) | The host's admin secret.                                                                                               |
| `--capability`  | capability string | required (env `ID_WORLD_CAPABILITY`)  | A guest capability, used to read the current state. Applying reads before it writes, so both credentials are required. |
| `--dry-run`     | switch            | off                                   | Print the plan only; change nothing.                                                                                   |

Behaviour: prints the plan and applies it. Every step — module install, grants, ceiling, profile, schedules — is applied through the admin operations and journaled by the host. When the plan is empty the command prints `no drift` and changes nothing. A full apply prints `applied <N> step(s)`; a partial apply prints `applied <N> of <M> step(s); the rest were reported but not applied`. With `--dry-run` it prints the plan and then `dry run: nothing changed`. Before any step runs, the plan passes the schedule-horizon check: a plan that fails it is refused whole, so no partial apply happens on that account.

Example — preview, then apply, a toy manifest:

```
$ id world apply 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    --file ./toy-world.toml --capability toy-cap-0001 --dry-run
$ id world apply 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    --file ./toy-world.toml --capability toy-cap-0001
```

Failures: omitting either credential is refused, exit `1`. A plan that fails the schedule-horizon check is refused before any step runs, exit `1`, with no partial apply.

## `contract`

Prints the versioned shell contract — the frame protocol every shell speaks — or checks a recorded session against it. It takes no `<NODE>`.

```
id world contract [--check <FILE> | --check-suite <FILE>]
```

| Flag            | Argument  | Default  | Meaning                                                                                                                                                         |
| --------------- | --------- | -------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--check`       | file path | — (none) | Transcript to check. `-` reads stdin. Without `--check` or `--check-suite`, the command prints the shell contract itself. Conflicts with `--check-suite`.       |
| `--check-suite` | file path | — (none) | Suite manifest to run (`contract/shell-suite.v1.json`): every shell's capability statement and recorded session, checked in one pass. Conflicts with `--check`. |

Behaviour: a transcript has one JSON object per line, of the form `{"from": "client"|"server", "frame": {...}}`. The check prints each violation.

Exit behaviour: the command fails, exit `1`, if there is any violation under `--check`, or if any shell fails under `--check-suite`. Printing the contract with no flags exits `0`.

Example — check a toy recorded session from stdin:

```
$ cat ./toy-session.jsonl | id world contract --check -
```

Failures: `--check` together with `--check-suite` is a clap error, exit `2`. A malformed transcript fails the check, exit `1`.

## `measure`

Measures a compiled `.wasm` world program's fuel use, offline. No host is contacted. This command exists only in `sandbox` builds.

```
id world measure --module <FILE> [OPTIONS]
```

| Flag       | Argument   | Default  | Meaning                                                             |
| ---------- | ---------- | -------- | ------------------------------------------------------------------- |
| `--module` | file path  | required | Wasm module built for the `id` Roc world ABI.                       |
| `--seed`   | integer    | `1`      | Deterministic seed supplied to the guest initializer.               |
| `--input`  | hex string | — (none) | Event to apply as participant 1, hex-encoded; repeatable, in order. |
| `--width`  | integer    | — (none) | Viewport width for the `layout` call. Requires `--height`.          |
| `--height` | integer    | — (none) | Viewport height for the `layout` call. Requires `--width`.          |

Behaviour: compiles the module with the default limits, runs `view`, then `layout` (with `--width`/`--height`), `records`, `wants` and `snapshot`, then each `--input` as participant 1 followed by a `view`. It prints one JSON object with, per call kind, the calls made, total and worst fuel, and the worst call as a percent of the fuel limit.

Exit behaviour: non-zero only if the module fails to compile or a call traps.

Example:

```
$ id world measure --module ./toy-world.wasm --width 80 --height 24
```

Failures: `--width` without `--height` (or the reverse) is a clap error, exit `2`. A module that fails to compile, or a call that traps, exits `1`. In a build without the `sandbox` feature the subcommand does not exist: clap error, exit `2`.

## `logs`

Tails a world's recent events, bounded by retention and a deadline.

```
id world logs <NODE> --capability <CAP> [OPTIONS]
```

| Flag           | Argument             | Default                              | Meaning                                                                                                    |
| -------------- | -------------------- | ------------------------------------ | ---------------------------------------------------------------------------------------------------------- |
| `<NODE>`       | 64 hex characters    | required                             | The host's node ID.                                                                                        |
| `--capability` | capability string    | required (env `ID_WORLD_CAPABILITY`) | A guest capability for the world.                                                                          |
| `--seconds`    | integer, `0..=86400` | `5`                                  | Stop after this many seconds. The maximum is `86400` (24 hours); a larger value is a clap error, exit `2`. |

Example — tail a toy world for 30 seconds:

```
$ id world logs 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    --capability toy-cap-0001 --seconds 30
```

Failures: an invalid capability is refused by the host, exit `1`.

## `stats`

Prints a world's module, capability and profile state as one JSON object.

```
id world stats <NODE> --capability <CAP>
```

| Flag           | Argument          | Default                              | Meaning                           |
| -------------- | ----------------- | ------------------------------------ | --------------------------------- |
| `<NODE>`       | 64 hex characters | required                             | The host's node ID.               |
| `--capability` | capability string | required (env `ID_WORLD_CAPABILITY`) | A guest capability for the world. |

Example:

```
$ id world stats 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    --capability toy-cap-0001
```

Failures: an invalid capability is refused by the host, exit `1`. There is no text form; stdout is the JSON object.

## `record`

Writes a world's retained journal history and records to a replayable fixture (JSON Lines). The fixture is the input to `replay` and `verify` below. Host events require the host's admin token.

```
id world record <NODE> --admin-token <TOKEN> [--out <FILE>]
```

| Flag            | Argument          | Default                               | Meaning                                            |
| --------------- | ----------------- | ------------------------------------- | -------------------------------------------------- |
| `<NODE>`        | 64 hex characters | required                              | The host's node ID.                                |
| `--admin-token` | token             | required (env `ID_WORLD_ADMIN_TOKEN`) | The host's admin secret.                           |
| `--out`         | file path         | stdout                                | Write the fixture here instead of standard output. |

Example:

```
$ id world record 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    --out ./toy-fixture.jsonl
```

Failures: a wrong admin token is refused by the host, exit `1`. Note what the fixture does not carry: the install seed. `replay` and `verify` take it as a separate `--seed` argument, supplied by the admin who installed the module.

## `replay`

Replays a recorded fixture offline through a module and compares records. No host is contacted. This command exists only in `sandbox` builds.

```
id world replay --module <FILE> --fixture <FILE> --seed <N> [--allow-partial]
```

| Flag              | Argument  | Default  | Meaning                                                                          |
| ----------------- | --------- | -------- | -------------------------------------------------------------------------------- |
| `--module`        | file path | required | The Wasm module the fixture was recorded under.                                  |
| `--fixture`       | file path | required | Fixture written by `world record`.                                               |
| `--seed`          | integer   | required | The seed the module was installed with. The fixture does not carry it.           |
| `--allow-partial` | switch    | off      | Replay a window that does not start at sequence 1. Its records are not compared. |

Behaviour: prints a verdict line first — `records match`, `records differ`, or `records not compared (<reason>)` — followed by the replayed records and view.

Exit behaviour: exits non-zero when the records differ.

Example:

```
$ id world replay --module ./toy-world.wasm --fixture ./toy-fixture.jsonl --seed 1
```

Failures: omitting any of `--module`, `--fixture` or `--seed` is a clap error, exit `2`. A record mismatch exits `1` after the verdict line. In a build without the `sandbox` feature the subcommand does not exist: clap error, exit `2`.

## `verify`

Verifies a recorded fixture's determinism: replays it twice offline through a module and compares the records after every event. No host is contacted. This command exists only in `sandbox` builds.

```
id world verify --module <FILE> --fixture <FILE> --seed <N> [--allow-partial]
```

| Flag              | Argument  | Default  | Meaning                                                                                                                                                                  |
| ----------------- | --------- | -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `--module`        | file path | required | The Wasm module the fixture was recorded under.                                                                                                                          |
| `--fixture`       | file path | required | Fixture written by `world record`.                                                                                                                                       |
| `--seed`          | integer   | required | The seed the module was installed with. The fixture does not carry it.                                                                                                   |
| `--allow-partial` | switch    | off      | Replay a window that does not start at sequence 1. Its records are not compared against the fixture's header, but the two replays are still compared against each other. |

Behaviour: prints a verdict line first. Verification covers the events captured by the admin-authorized fixture, including host-originated events in their recorded order. It proves determinism for that history, not correctness of host policy.

Exit behaviour: exits non-zero at the first divergence, or when the two replays agree with each other but mismatch the fixture's header records (that header verdict is printed, as `replay` prints it).

Example:

```
$ id world verify --module ./toy-world.wasm --fixture ./toy-fixture.jsonl --seed 1
```

Failures: omitting any of `--module`, `--fixture` or `--seed` is a clap error, exit `2`. A divergence exits `1` after the verdict line. In a build without the `sandbox` feature the subcommand does not exist: clap error, exit `2`.

## `list` (admin form)

Lists the worlds a host serves, using the admin listing. The public directory form is in [World commands: joining and playing](cli-worlds.md).

```
id world list <NODE> --admin-token <TOKEN> [--json]
```

| Flag            | Argument          | Default                        | Meaning                                                                                                       |
| --------------- | ----------------- | ------------------------------ | ------------------------------------------------------------------------------------------------------------- |
| `<NODE>`        | 64 hex characters | required                       | The host's node ID.                                                                                           |
| `--admin-token` | token             | — (env `ID_WORLD_ADMIN_TOKEN`) | The host's admin secret. Giving it selects the admin listing; omitting it reads the public directory instead. |
| `--json`        | switch            | off                            | Print one JSON object instead of tab-separated lines.                                                         |

Example:

```
$ id world list 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef --json
```

Failures: a wrong admin token is refused by the host, exit `1`. An unreachable host fails the command, exit `1`.

## `isolation`

Sets whether a world's directory may be read from other unisolated worlds. Worlds are isolated by default; writes never cross worlds.

```
id world isolation <NODE> <isolated|unisolated> --admin-token <TOKEN>
```

| Flag            | Argument               | Default                               | Meaning                                                                                                                           |
| --------------- | ---------------------- | ------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `<NODE>`        | 64 hex characters      | required                              | The host's node ID.                                                                                                               |
| `<MODE>`        | `isolated\|unisolated` | required positional                   | `isolated`: no cross-world reads. `unisolated`: readable from other unisolated worlds. Any other value is a clap error, exit `2`. |
| `--admin-token` | token                  | required (env `ID_WORLD_ADMIN_TOKEN`) | The host's admin secret.                                                                                                          |

Example:

```
$ id world isolation 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    unisolated
```

Failures: a wrong admin token is refused, exit `1`.

## `cross-world-write`

Lets other unisolated worlds write into a world, or stops them.

```
id world cross-world-write <NODE> <on|off> --admin-token <TOKEN>
```

| Flag            | Argument          | Default                               | Meaning                                                                                                       |
| --------------- | ----------------- | ------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| `<NODE>`        | 64 hex characters | required                              | The host's node ID.                                                                                           |
| `<MODE>`        | `on\|off`         | required positional                   | `on`: other unisolated worlds may write here. `off`: they may not. Any other value is a clap error, exit `2`. |
| `--admin-token` | token             | required (env `ID_WORLD_ADMIN_TOKEN`) | The host's admin secret.                                                                                      |

Example:

```
$ id world cross-world-write 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    on
```

Failures: a wrong admin token is refused, exit `1`.

## `force-cross-world-write`

Forces cross-world writes on for an unisolated world, overriding its own setting. There is no mode positional and no off form in this command: it is force-on only. Moderators cannot undo a force.

```
id world force-cross-world-write <NODE> --admin-token <TOKEN>
```

| Flag            | Argument          | Default                               | Meaning                  |
| --------------- | ----------------- | ------------------------------------- | ------------------------ |
| `<NODE>`        | 64 hex characters | required                              | The host's node ID.      |
| `--admin-token` | token             | required (env `ID_WORLD_ADMIN_TOKEN`) | The host's admin secret. |

Example:

```
$ id world force-cross-world-write 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
```

Failures: a wrong admin token is refused, exit `1`. Passing a mode such as `off` is a clap error (unexpected argument), exit `2`.

## `file-scope`

Sets a world's file scope.

```
id world file-scope <NODE> <confined|unrestricted> --admin-token <TOKEN> [--write-override <on|off>]
```

| Flag               | Argument                 | Default                               | Meaning                                                                                                        |
| ------------------ | ------------------------ | ------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| `<NODE>`           | 64 hex characters        | required                              | The host's node ID.                                                                                            |
| `<SCOPE>`          | `confined\|unrestricted` | required positional                   | `confined`: the world's own sandbox root. `unrestricted`: any path. Any other value is a clap error, exit `2`. |
| `--write-override` | `on\|off`                | — (none)                              | Admin file write override for an isolated world.                                                               |
| `--admin-token`    | token                    | required (env `ID_WORLD_ADMIN_TOKEN`) | The host's admin secret.                                                                                       |

Example:

```
$ id world file-scope 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    confined --write-override off
```

Failures: a wrong admin token is refused, exit `1`. An invalid `--write-override` value is a clap error, exit `2`.

## `replica`

Follows a pinned writer into a durable replica, opens it under the policy-B admission gate, and serves it.

```
id world replica <NODE> --dir <DIR> --admin-token <TOKEN> [OPTIONS]
```

| Flag            | Argument          | Default                               | Meaning                                                                                                                                                                                                       |
| --------------- | ----------------- | ------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `<NODE>`        | 64 hex characters | required                              | The writer host's node ID.                                                                                                                                                                                    |
| `--dir`         | directory path    | required                              | Replica directory: the durable journal lives here and is opened in place on later runs (created on first follow). Omitting it is a clap error, exit `2`.                                                      |
| `--admin-token` | token             | required (env `ID_WORLD_ADMIN_TOKEN`) | Admin secret of the writer host, authorizing the raw journal feed.                                                                                                                                            |
| `--world`       | name              | `lobby`                               | World to replicate (the writer's world name). Environment fallback: `ID_WORLD`. This is the only world command where `--world` has a clap default value.                                                      |
| `--source`      | 64 hex characters | the writer's node ID                  | Pinned writer source key: the replication stream and the admission lease must both verify against it. The default holds because the writer's node ID is the key the writer signs its replication stream with. |
| `--lease`       | file path (JSON)  | — (none)                              | A writer-signed admission lease to hold while serving: new spectators are admitted only while it verifies (policy B). Without one the replica serves gated, admitting no new spectators.                      |
| `--once`        | switch            | off                                   | Sync once from the writer, print the replica status as JSON, and exit without serving. Conflicts with `--follow`.                                                                                             |
| `--follow`      | switch            | off                                   | Keep following the writer (reconnecting across sessions), printing one JSON line per committed cursor; never serves. Conflicts with `--once`.                                                                 |

Behaviour: in its default mode the command syncs the replica directory from the writer's signed journal feed, prints the replica status (source, world, cursor, staleness) as one JSON object, then serves the replica's world sessions on this node. `--once` stops after the status print. `--follow` keeps the replica journal current without serving.

Example — take one sync of a toy world, print the status, and exit:

```
$ id world replica 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    --dir ./toy-replica --world lobby --once
```

Failures: `--once` together with `--follow` is a clap error, exit `2`. A replication stream or lease that does not verify against `--source` is refused, exit `1`. A wrong admin token is refused by the writer host, exit `1`.

## `create`

Creates a world on a host.

```
id world create <NODE> <NAME> --admin-token <TOKEN>
```

| Flag            | Argument          | Default                               | Meaning                                                    |
| --------------- | ----------------- | ------------------------------------- | ---------------------------------------------------------- |
| `<NODE>`        | 64 hex characters | required                              | The host's node ID.                                        |
| `<NAME>`        | string            | required positional                   | Name of the new world. Charset: `a-z`, `0-9`, `-` and `_`. |
| `--admin-token` | token             | required (env `ID_WORLD_ADMIN_TOKEN`) | The host's admin secret.                                   |

Behaviour: on success the command prints `created`; if the world already exists on the host it prints `exists`.

Example:

```
$ id world create 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    toy-arena
```

Failures: a name outside the charset is refused, exit `1`. A wrong admin token is refused, exit `1`.

## `journal check` and `journal backup`

Inspects or copies a durable world directory's `journal.jsonl`. Local only; no host is contacted. The local read-only reports over the same directory are `fork`, `provenance` and `owner` in [World commands: joining and playing](cli-worlds.md).

```
id world journal check <DIR> [--json]
id world journal backup <DIR> <OUT>
```

### `journal check`

| Flag     | Argument       | Default  | Meaning                                                            |
| -------- | -------------- | -------- | ------------------------------------------------------------------ |
| `<DIR>`  | directory path | required | Directory of the world to check (the one holding `journal.jsonl`). |
| `--json` | switch         | off      | Print one JSON object instead of text lines.                       |

Behaviour: reports the journal's format version and validity, flagging a journal written in a newer format than this binary can read.

Exit behaviour, defined in source: exits non-zero when the journal is invalid or written in a newer format than the binary reads.

Example:

```
$ id world journal check ./toy-world-dir
```

### `journal backup`

| Flag    | Argument       | Default             | Meaning                                                              |
| ------- | -------------- | ------------------- | -------------------------------------------------------------------- |
| `<DIR>` | directory path | required            | Directory of the world to back up (the one holding `journal.jsonl`). |
| `<OUT>` | file path      | required positional | The backup file to write. It must not already exist.                 |

Behaviour: copies the journal to the backup file, byte for byte, so the copy restores the world exactly. The journal must check valid first: the backup runs the same validity check before copying.

Example:

```
$ id world journal backup ./toy-world-dir ./toy-world-journal-backup.jsonl
```

Failures: an `<OUT>` path that already exists fails the command, exit `1`, and the existing file is not overwritten. A journal that does not check valid fails the backup, exit `1`.
