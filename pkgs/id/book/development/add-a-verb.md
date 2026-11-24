# Add a command

Source: `pkgs/id/crates/id-cli/src/cli/mod.rs`, `pkgs/id/crates/id-cli/src/cli/world.rs`, `pkgs/id/crates/id-cli/src/commands/mod.rs`, `pkgs/id/src/main.rs` and `pkgs/id/crates/id-cli/src/repl/runner.rs` in https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-cli/src/cli/mod.rs — provenance only; this chapter is complete without following it.

"Command" here means a new invocable surface on the `id` binary: a top-level command or a subcommand under an existing family. A command is not done when its handler works. It is done when the parser, the handler, the dispatch, the help text, every surface that mirrors it, the tests and this book all describe the same command — in the same change, per [Keeping this book current](keeping-this-book-current.md).

The second binary, `id-native-shell`, has a hand-rolled argument parser in `crates/id-native-shell/src/main.rs` and no clap. This chapter covers the `id` binary only.

## 1. The parser: `crates/id-cli/src/cli/`

The clap derive parser lives in the `id-cli` crate, in `src/cli/`:

| File                                                                     | What it declares                                                                                                                              |
| ------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `cli/mod.rs`                                                             | The `Cli` struct with the global flags, the top-level `Command` enum, the `serve` arguments, and the families that have no file of their own. |
| `cli/world.rs`                                                           | The `world` family: one argument struct per subcommand.                                                                                       |
| `cli/tag.rs`                                                             | The `tag` family.                                                                                                                             |
| `cli/groups.rs`                                                          | The `group` family, plus `delegate` and `revoke`.                                                                                             |
| `cli/gallery.rs`                                                         | The `gallery` family.                                                                                                                         |
| `cli/walk.rs`                                                            | The `walk` family.                                                                                                                            |
| `cli/claim.rs`, `cli/doc.rs`, `cli/writer_sets.rs`, `cli/match_audit.rs` | The `claim`, `doc`, `writer-sets` and `match-audit` families.                                                                                 |
| `cli/tests.rs`                                                           | The parser test suite for the whole `cli` module.                                                                                             |

Declare the new command in the file for its family: a variant on the family's command enum, an argument struct with one `#[arg]` field per flag or positional, and doc comments on both. The doc comments are the help text — clap renders them — so each one states what the argument does, its literal default where it has one, and any requirement or conflict a caller would otherwise have to read the handler to learn. Follow the family's existing conventions for the repeated shapes:

- World client commands repeat the same connection fields in each argument struct — the node positional, `--admin-token` with env `ID_WORLD_ADMIN_TOKEN`, `--capability` with env `ID_WORLD_CAPABILITY`, `--world` with env `ID_WORLD`, `--addr`, `--no-relay`. They are not global flags. Copy the block from a sibling subcommand; do not invent a new spelling.
- A flag with an environment fallback declares it in the `#[arg(env = "...")]` attribute, with `hide_env_values = true` when the value is a token or capability.
- An optional-value flag declares `num_args = 0..=1` and states its missing-value meaning in its doc comment. The precedent is `--new` on the top-level parser and `--first` / `--last` on the search family.
- A value that must be validated at parse time gets a value parser, so a bad value is a clap error with exit status `2`, not a handler error. The precedent is the result-limit parser shared by `find`, `search`, `show` and `peek`, which rejects `0` with the message `zero is not a valid limit: use at least 1`.

If the new syntax interacts with argument rewriting, check `cli::normalize_args` in `cli/mod.rs`: the top-level entry point rewrites a bare `--new` followed by a subcommand name so the subcommand is not swallowed as the instance name. A new optional-value flag in the leading position needs the same treatment.

### Decide the feature gate before writing the handler

Three gating shapes exist in the current parser, and a new command takes one of them deliberately:

| Shape                                               | Precedent                                                                                                                             | Behaviour in a build without the feature                                                                    |
| --------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| Parser variant gated with `#[cfg(feature = "...")]` | The `sandbox` variants `world dev`, `world measure`, `world replay`, `world verify`, and the whole `match-audit` command.             | The command does not exist: clap reports an unrecognized subcommand, and the help tree does not show it.    |
| Parser unconditional, dispatch gated                | The whole `world` family: the `Command` variant is unconditional, and `src/main.rs` carries a gated dispatch arm plus a fallback arm. | The help tree prints in full, and every invocation fails at runtime with `this build has no world support`. |
| Parser unconditional, handler guard                 | `serve --web` and `serve --server-config`.                                                                                            | The flag parses, and startup fails with an error that names the missing feature and the rebuild flag.       |

The feature names are defined in the root `Cargo.toml` and mirrored in `crates/id-cli/Cargo.toml`; the definitions and defaults are in [Building and testing](building.md). A command that needs wasmtime takes the first shape. A command in a family that must stay discoverable in help takes the second or third, and its help text states the requirement.

## 2. The handler: `crates/id-cli/src/commands/`

Handlers live in `src/commands/` in the `id-cli` crate, one file per family: `put.rs`, `get.rs`, `find.rs`, `list.rs`, `tag.rs`, `world.rs` with a `world/` submodule directory, `serve.rs` with a `serve/` submodule directory, `group.rs`, `gallery.rs`, `walk.rs`, and the rest. Add the handler to its family's file, or give a new family its own file, and register the module in `commands/mod.rs`.

Handler rules, from the existing code:

- A handler returns `anyhow::Result<()>`. An error propagated out of `main` prints `Error:` followed by the error chain to stderr and exits with status `1`. There is no per-command exit-code table and no `process::exit` call in the `id` binary or in `id-cli`.
- Multi-target handlers aggregate partial failures into one error instead of failing on the first target. The precedents are `put` and `get`, whose aggregated messages begin `some puts failed:` and `some gets failed:`.
- Empty input is guarded explicitly, with a message that names the missing input. Precedents: `no files provided`, `no sources provided`.
- A handler that operates locally and remotely goes through the family's existing chooser instead of opening the store directly. For file commands that is the meta-client layer in `id-cli` (`MetaClient` in `src/meta_client.rs`); the local-serve lock file decides whether operations proxy to a running `serve`.

## 3. The dispatch: `pkgs/id/src/main.rs`

The binary entry point stays in the root package. It imports the parser and the handlers through the facade — `id::{Cli, Command, ...}` — and matches on the parsed `Command`. A new top-level command needs a match arm there. A new subcommand inside an existing family dispatches through that family's handler entry point and needs no change in `main.rs`, unless the family itself is feature-gated at dispatch: the `world` arm is the precedent, with one arm compiled under the `world` feature and one fallback arm compiled without it.

Running `id` with no subcommand starts the REPL. A new command does not change that; a new global flag does interact with it, because the REPL entry is the `None` arm of the same match.

## 4. The REPL: decide explicitly

The interactive REPL does not reuse the clap parser. A line is whitespace-split and matched against literal patterns in `crates/id-cli/src/repl/runner.rs`, with input preprocessing in `crates/id-cli/src/repl/input.rs` and the session state in `crates/id-cli/src/commands/repl.rs` (`ReplContext`). The full REPL surface is [The interactive REPL](../reference/repl.md).

Ask, and record the answer in the change: does this command belong in the REPL too?

- The REPL carries the file-operation set only: `list`, `put`, `get`, `cat`, `find`, `search`, `show`, `peek`, the `tag` family, and its own session commands. It has no `serve`, `world`, `group`, `doc`, `claim`, `key`, `keyset`, `gallery` or `walk` commands, and a command from those families does not get a REPL form.
- If the command is a file operation and it gets a REPL form: add the match arm in `runner.rs`, add any flag parsing to the hand parser the neighbouring commands use — the REPL re-implements each flag itself, and clap defaults do not carry over — and add the command to the REPL's own `help` output in the same file. Note the standing trap: the REPL's internal default for `find` and `search` format is `union`, where the clap commands default to `tag`. A new shared flag must state its default in both parsers, and the two defaults must be the same unless the difference is deliberate and documented in both places.
- The REPL also has commands with no clap counterpart at all — `delete`, `rename` and `copy` exist only there. A REPL-only command is a legitimate shape for a session operation, but it is a decision, not an accident: the top-level CLI's inability to perform the operation must be stated in the reference chapter.

## 5. Tests

- Parser tests in `crates/id-cli/src/cli/tests.rs`: the new command parses with its defaults, each flag parses in its accepted range, conflicting flags conflict, and any spelling that must stay rejected stays rejected. The precedent for a rejection test is the pair asserting that `serve --replace-defaults` and `peers --replace-defaults` are parse errors.
- Handler tests beside the family's existing tests, and an integration case in `pkgs/id/tests/cli_integration.rs` when the command drives the built binary end to end. The integration harness locates the binary through the `ID_BINARY` environment variable when it is set.
- Run the family first, then the sandbox-safe suite: `cargo nextest run --all-features -p id-cli` for the crate, and `just test-sandbox` for the workspace set. The recipes and their exact commands are in [Building and testing](building.md).
- Every behavioural expectation names its oracle: the recorded output, fixture or invariant the test checks against. A test that only asserts the command exits `0` proves the parser ran, not that the command did the work.

## 6. The book, in the same change

Update the reference chapter that covers the family — the top-level map is [The id command line](../reference/cli.md), and each family has its own chapter in the reference section — plus this chapter if the change alters the pattern it teaches, plus any chapter that states a default the new command changes. A new flag with an environment fallback also updates [Configuration and environment](../reference/configuration.md). The full contract, including the two build gates that enforce the book's own completeness, is [Keeping this book current](keeping-this-book-current.md).
