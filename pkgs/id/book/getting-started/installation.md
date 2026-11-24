# Installation and provisioning

ID builds from the `developing-today/code` repository, in `pkgs/id/`. This
chapter covers the two build routes the package README documents — Nix
with `just`, and Cargo directly — the feature split that decides what a
given build contains, the data directory a node creates, and the version
check that proves the binary you built is the binary you are running.

All commands below run inside `pkgs/id/`.

## Enter the development shell (Nix route)

```bash
nix develop
```

The dev shell provides the full tool set the README lists: Rust 1.99.1,
clippy, rustfmt, cargo-llvm-cov, cargo-audit, cargo-machete, `just`, Bun,
TypeScript, Chromium, and Firefox. The Cargo route below needs only Rust
for the library build, and Rust plus Bun for the web build.

## Build with just

```bash
# Library build (no web UI, no Bun required)
just build-lib

# Full build with the web UI (requires Bun)
just build
```

`just build-lib` runs the `lib` variant of `scripts/build.sh`;
`just build` runs the `web` variant, which builds the TypeScript frontend
in `pkgs/id/web/` first and embeds the result in the binary. The matching
serve recipes are `just serve-lib` (which runs
`cargo run -- serve`) and `just serve` (which runs
`cargo run --features web -- serve --web`).

The two Nix packages in `pkgs/id/flake.nix` install the same single `id`
binary under two feature sets:

| Nix package | Features | Web UI | World SSH serving |
| --- | --- | --- | --- |
| `id-web` (the default package) | `web`, `sandbox`, `ssh`, `world` | yes | yes |
| `id-lib` | `sandbox`, `world` (built with default features off) | no | no |

Build them with `just build-nix` (the web variant) and
`just build-nix-lib` (which runs `nix build .#id-lib`).

## Build with Cargo

```bash
# Default build: sandbox and world, no web UI
cargo build

# Web UI build: build the frontend assets first, then enable the feature
cd web && bun install && bun run build && cd ..
cargo build --features web
```

The root package's feature table is the authority:

- `default = [sandbox, world]`. A plain `cargo build` therefore includes
  the wasmtime world sandbox (`sandbox`, which itself implies `world`)
  and every world command, and excludes the web UI and world SSH serving.
- `web` is a separate opt-in feature. It is not part of the default set.
- `ssh = [world, …]` adds world serving over SSH.
- `dht` adds mainline DHT address lookup. It is off in the default build.
- `currency-poc-tb` adds the TigerBeetle client for the currency proof of
  concept. It adds no CLI surface.

### The `--no-default-features` trap (disagreement CLI-2, resolved)

The README's Cargo quick start once presented this command as the
library-only build:

```bash
cargo build --no-default-features
```

The source disagrees with the old framing, and this book follows the source.
Because `default = [sandbox, world]`, `--no-default-features` removes
`sandbox` and `world` as well as leaving `web` off. The concrete losses
in a `--no-default-features` build are:

- `id match-audit` and the four sandbox world commands — `id world dev`,
  `id world measure`, `id world replay`, `id world verify` — still parse
  and print their `--help`, but fail at dispatch with the error
  `this build has no sandbox support`.
- Every other `id world …` command still parses and still prints its full
  `--help` tree, but fails at runtime with the error
  `this build has no world support`.
- `id serve --web` fails at startup with an error stating the binary was
  built without the `web` feature.
- `id serve --server-config <FILE>` fails at startup with an error
  stating the binary was built without the `world` feature.

What a `--no-default-features` build keeps: the file commands, `tag`,
`group`, `delegate`, `revoke`, `doc`, `claim`, `writer-sets`, `walk`,
`gallery`, `key`, `keyset`, `peers`, and the REPL. `id walk` is not
feature-gated anywhere and works in every build.

The Nix `id-lib` package is the build the old README command intended: it is
built with default features off and then re-adds exactly the two features
the old README command left out — `buildNoDefaultFeatures = true` with
`buildFeatures = [sandbox, world]`. If you want the `id-lib` feature set
from Cargo directly, the equivalent is:

```bash
cargo build --no-default-features --features sandbox,world
```

## Provision a data directory

An `id` node keeps all of its state in one data directory. The default
data directory is the current working directory. Two global flags, given
before the subcommand, change it:

- `-d, --data-dir <DIR>` — use `<DIR>` as the base directory. It is
  created if missing. The process changes directory into it at startup.
- `--new [<NAME>]` — create a fresh instance directory at
  `.iroh/<NAME>/` under the current directory and use it. The name is
  optional: a bare `--new` generates a random 8-hex name. The command
  fails if the directory already exists, and the name must not contain
  `/`, `\`, or `..`. `--new` conflicts with `--data-dir`.

Inside the data directory, the layout is:

| Path | What it is |
| --- | --- |
| `.iroh-store/` | The Iroh blob store (redb plus data files), with the Iroh docs store under `.iroh-store/docs/`. The constant is `STORE_PATH = ".iroh-store"` in `id-store`. |
| `.iroh-key` | The node's own key file. The client key file is `.iroh-key-client`. |
| `.iroh-serve.lock` | The lock file a running `id serve` holds. Its presence is how the REPL and other commands detect the local serve and proxy operations to it. The constant is `SERVE_LOCK = ".iroh-serve.lock"` in `id-core`. |
| `.iroh-meta/` | The tag-store registry directory; the registry file inside it is `.iroh-meta/registry.json`. |
| `.id-worlds/<NAME>/` | Per-world files for a hosted world, where `<NAME>` is the world name (default `lobby`). |
| `events/` | The event store directory used by `doc`, `claim`, and `writer-sets`. This is the runtime default of the global `--event-store <DIR>` flag; a relative value is relative to the data directory. |
| `groups.jsonl` | The group log used by `group`, `delegate`, and `revoke`. This is the runtime default of the global `--groups-log <FILE>` flag; a relative value is relative to the data directory. |

One disagreement is worth knowing before you go looking for the lock:
the module documentation in `crates/id-cli/src/commands/serve.rs` once
drew the lock file as `.id-serve-lock`. That spelling was wrong
(disagreement CLI-5, resolved — the module documentation now matches).
The constant, the `--data-dir` help text, and the code all use
`.iroh-serve.lock`.

`id serve --ephemeral` skips provisioning entirely: it uses in-memory
stores, and all content is lost when the process exits.

Identity provisioning is separate from the data directory. A node
creates its key on first use of the store. The `id key` and `id keyset`
command families export, import, enrol, and recover keys; passphrases
are always hidden prompts, never flags. Those commands are covered in
[Groups, documents, claims and keys](../reference/cli-groups-identity.md).

## Verify the install

```bash
id --version
```

A correct build prints two lines:

```text
id 0.1.0
defaults: file sha256=<hash> mode=append
```

The first line is the version, `0.1.0`. The second line is the
embedded-defaults provenance, installed by `pkgs/id/src/main.rs`: it
states which defaults source is in force for peer discovery — the
embedded `defaults.json` file (with its SHA-256 and its `mode`, `append`
in the shipped file) or the built-in equivalent. A one-line version
output means the binary in front of you is an older build whose surface
also differs from the source in other ways (disagreement CLI-4): older
Nix store builds accept a `serve --replace-defaults` flag the current
parser rejects, spell the defaults file `defaults.conf`, and have no
`walk` command.

Check which `id` you are running before trusting the output. The name
collides with the GNU coreutils `id` program by design — the platform
even has an `id id` command that prints the node's identity — and a
system `id` on the `PATH` prints user and group IDs instead of the two
lines above. If `id --version` does not print both lines, the binary on
your `PATH` is not the one this chapter built.

Next: [Quickstart](quickstart.md) runs one complete
store, fetch, tag, and find pass against a fresh data directory.
