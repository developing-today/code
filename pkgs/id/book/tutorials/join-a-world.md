# Tutorial: join a world and run a guest app

This tutorial has two halves, and the split between them is the honest shape of the feature. The first half is fully local: scaffold a world program, run it in the offline scratch-world loop, and check the toolchain. No server, no token, and no network are involved. The second half needs a hosted world: a running `id serve --world` on some node, an admin token held by that node's operator, and a guest capability the operator issues to you. A reader working alone runs both halves on one machine by operating the serve themselves; a reader joining someone else's world starts at Step 4 with a node ID and a capability from that world's operator.

World commands are clients. `id serve` is the only command that hosts a world (the F13.8 client-only boundary, stated in its long help). The commands are documented in [World commands: joining and playing](../reference/cli-worlds.md) and [World commands: administering and verifying](../reference/cli-worlds-admin.md); the guest model itself is in [Guest apps and shells as consumers](../library/guests-and-shells.md).

## Part 1: the local loop

### Step 1: scaffold a world program

```sh
id world new my-world
```

`id world new <DIR>` is a local-only command. It scaffolds `main.roc`, `build.sh`, and `README.md` in the directory. The flag `--name <NAME>` sets the world name and defaults to `world`. The directory must be absent or empty.

### Step 2: run the scratch-world loop

```sh
id world dev my-world
```

`id world dev <DIR>` runs the fully offline scratch-world loop: it compiles the program and drives it locally, then prints one JSON report. Useful flags, with their literal defaults: `--seed <N>` defaults to 1; `--input <TEXT>` is repeatable and feeds one input per occurrence; `--fixture <FILE>` is repeatable; `--fuzz <FILE>` runs a fuzz fixture with `--fuzz-seed <N>` defaulting to 42 and `--fuzz-cases <N>` defaulting to 32; `--watch` re-runs on change, with `--watch-max-cycles <N>` defaulting to 1000 and `--watch-debounce-ms <N>` defaulting to 150. The flags `--platform <DIR>` and `--roc <BIN>` override the platform package and the Roc compiler; when unset, the compiler falls back to `roc` on the `PATH` and the platform to `./examples/roc-world`.

Edit `my-world/main.roc`, re-run `id world dev my-world`, and read the JSON report after each change. This loop is the whole local development story: compile, run, inspect records and output, repeat. Two more offline commands support it: `id world doctor` checks the Roc toolchain and related state (it exits non-zero only when a check fails; warnings do not fail it), and `id world measure --module <FILE>` measures a compiled module's fuel use offline and prints one JSON object.

`id world dev`, `id world measure`, `id world replay`, and `id world verify` exist only in builds with the `sandbox` feature, which the default build has (`default = ["sandbox", "world"]`). In a build without it, clap reports these four as unrecognized subcommands.

## Part 2: a hosted world

### Step 3: the operator starts a world host

On the host node, the operator runs:

```sh
id serve --world --world-admin-token `<TOKEN>` --world-module `<MODULE.wasm>`
```

`--world` hosts a multiplayer world and requires `--world-admin-token`. The token value is a secret: it is supplied by flag or by the environment variable named `ID_WORLD_ADMIN_TOKEN`, and it is never printed in this book — only the name is. `--world-module <FILE>` takes a precompiled Wasm module in the Roc ABI and also requires `--world`; loading it requires a build with the `sandbox` feature. The world name flag `--world-name <NAME>` defaults to `lobby`, and the world's files live in `.id-worlds/<NAME>/`. Instead of a precompiled module, the operator can let guests install Roc source: the flag `--world-source-only` (environment variable `ID_WORLD_SOURCE_ONLY`) restricts installs to Roc source and conflicts with `--world-module`.

The operator prints the host's node ID with `id id` and gives three things to each joining reader: the node ID, the world name if it is not `lobby`, and — after Step 4 — a capability.

### Step 4: the operator invites you

An invite is minted per participant:

```sh
id world invite `<NODE_ID>` --admin-token `<TOKEN>` --name Ada
```

`--name <NAME>` is required: it is the participant's display name. The flag `--role <ROLE>` accepts `spectator`, `observer`, `player`, `guest`, `delegator`, `builder`, or `moderator`, and its documented default is `guest`. The flag `--expires-in <DURATION>` takes values such as `12h` or `3d`; its documented default is 7 days and the maximum is 30 days. The flag `--uses <N>` requires a value of at least 1; omitted, the invite is unlimited. The command prints the guest capability. Treat that capability as a credential: it travels by the flag `--capability` or the environment variable named `ID_WORLD_CAPABILITY`, and command definitions mark both token flags hidden.

### Step 5: join

```sh
id world join `<NODE_ID>` --capability `<CAPABILITY>`
```

The flag `--world <NAME>` (environment variable `ID_WORLD`) selects a world other than the host's default. The flag `--via <iroh|ssh>` defaults to `iroh`; with `ssh`, the node argument is a `[user@]host[:port]` address instead of a node ID. The flag `--after <SEQ>` resumes from a journal sequence. Inside the session, input lines are chat by default; with `--command-mode`, stdin lines are parsed as session commands (`/chat`, `/records`, `/stats`, `/caps`, `/quit`, and others), and `--json` — which requires `--command-mode` — prints session output as JSON.

Once joined, the guest program runs on the host. Your inputs go to its `update`, its `view` comes back to your terminal, and its capability requests are answered by the host under the grants your capability carries — a request outside those grants returns a refusal event, not an error that ends the session (see [Worlds and capabilities as a library](../library/worlds-and-caps.md)).

### Step 6: read records and check capabilities

From a second terminal, while joined or after:

```sh
id world records `<NODE_ID>` --capability `<CAPABILITY>`
id world caps `<NODE_ID>` --capability `<CAPABILITY>`
```

`id world records` prints the world's published records; the flag `--prefix <PREFIX>` filters them. For tic-tac-toe, the records are the JSON object `{"board","plays","winner"}` that the guest publishes. `id world caps` with only a guest capability reads the capability state; changing grants (`--grant`, `--revoke`, `--ceiling`) requires the admin token as well.

## What success looks like

- Locally: `id world dev my-world` prints one JSON report with no compile error, and the report's records match what `main.roc` computes.
- Hosted: `id world join` connects, the guest's initial view renders in the terminal, inputs change the view, and `id world records` prints the records the guest publishes.
- The operator's side agrees: the world named `lobby` (or the chosen `--world-name`) is the one your session joined, visible in the host's session counts (`id world stats <NODE> --capability <C>` prints one JSON object).

## One failure shape

Run any `id world` command other than the four parser-gated ones against a binary built without the `world` feature — for example a bare `--no-default-features` build:

```sh
id world join `<NODE_ID>` --capability `<CAPABILITY>`
```

The full `world` command tree parses, and `--help` prints it, but the invocation fails at runtime with the message `this build has no world support` and exit code 1. No connection is attempted. The fix is a build with the default features (`sandbox` and `world`) — the Nix `id-web` package and a plain `cargo build` both qualify. The related local failure is the mirror image: on a build without `sandbox`, `id world dev` does not fail at runtime — it does not exist, and clap exits with code 2 reporting an unrecognized subcommand.
