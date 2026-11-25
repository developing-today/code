# Building and testing

Source: `pkgs/id/justfile`, `pkgs/id/flake.nix`, `pkgs/id/Cargo.toml`, `pkgs/id/rust-toolchain.toml`, `pkgs/id/build.rs` and `pkgs/id/web/package.json` in https://github.com/developing-today/code/blob/main/pkgs/id/justfile — provenance only; this chapter is complete without following it.

All commands in this chapter run from `pkgs/id/` unless a command says otherwise. Every just recipe named here exists in `pkgs/id/justfile`; every Cargo flag and feature name exists in `pkgs/id/Cargo.toml`; every Nix package and check name exists in `pkgs/id/flake.nix`.

## The development shell and the toolchain pin

Enter the development shell from `pkgs/id/`:

```sh
nix develop
```

The sub-flake's `devShells.default` provides the build inputs, the native build inputs and the OpenSSL environment the packages use, so the Cargo, just, Bun and Roc commands below run with the pinned tool versions.

The toolchain pin is `pkgs/id/rust-toolchain.toml`:

- channel: `1.99.1`
- components: `rustfmt`, `clippy`, `rust-analyzer`, `llvm-tools`
- profile: `default`

The file is read by rustup directly and by the flake through rust-overlay. Its own header states that deleting it breaks `nix develop` and `nix build`. The pin is not the minimum supported version: the root `Cargo.toml` declares `rust-version = "1.96"`. The pin states what the project builds with; `rust-version` states the oldest compiler the manifest accepts.

## Building with Cargo

```sh
cargo build                        # default features: sandbox + world, no web UI
cargo build --features web         # the web variant
cargo build --release              # release profile, default features
cargo build --release --features web
```

The justfile wraps the same builds and tracks which variant was built last, in `target/.build-variant` (debug) and `target/.build-variant-release` (release):

| Recipe | What it runs |
|---|---|
| `just build` | `./scripts/build.sh web debug` — the web variant, debug profile. Alias: `build-web`. |
| `just build-lib` | `./scripts/build.sh lib debug` — the library variant: a plain `cargo build` with default features. |
| `just release` | `./scripts/build.sh web release`. Aliases: `build-release`, `build-web-release`. |
| `just release-lib` | `./scripts/build.sh lib release`. |
| `just build-web-cargo` | `cargo build --features web`. Alias: `build-cargo`. |
| `just build-lib-cargo` | `cargo build`. |
| `just release-web-cargo` | `cargo build --release --features web`. |
| `just release-lib-cargo` | `cargo build --release`. |
| `just clean` | `cargo clean`. |

The `-force` recipes (`build-web-force`, `build-lib-force`, `release-web-force`, `release-lib-force`) bypass the build script's freshness checks and rebuild the web assets or the binary directly.

## Feature flags

The root crate's `[features]` table in `pkgs/id/Cargo.toml` is the whole feature system. `id-cli`'s `Cargo.toml` mirrors the same names for the application layer.

| Feature | Definition in the root crate | Default |
|---|---|---|
| `sandbox` | Implies `world`; forwards to `id-core/sandbox`, `id-caps/sandbox`, `id-worlds/sandbox` and `id-cli/sandbox`. It is the wasmtime guest runtime: it gates the parser variants `id world dev`, `id world measure`, `id world replay`, `id world verify` and the whole `id match-audit` command. | On, through `default`. |
| `world` | Forwards to `id-core/world` and `id-cli/world`. Without it, the `id world` parser still prints its full help tree, but every `id world` invocation fails at runtime with the error `this build has no world support`. | On, through `default`. |
| `web` | Implies `world`; links the optional `id-web` dependency and forwards to `id-core/web` and `id-cli/web`. Without it, `id serve --web` fails at startup with an error naming the missing feature. | Off. |
| `ssh` | Implies `world`; forwards to `id-worlds/ssh` and `id-cli/ssh`. It is the world-over-SSH server inside `id serve --world-ssh-port`. The flag itself parses in every build. | Off. |
| `dht` | Forwards to `id-sync/dht` and `id-cli/dht`: mainline DHT address lookup and publishing. The `serve --dht` flag parses in every build; the feature controls the lookup code behind it. | Off. |
| `currency-poc-tb` | Links the optional TigerBeetle Rust client for the currency proof of concept. No command-line surface. | Off. |
| `default` | `["sandbox", "world"]`. | — |

### The CLI-2 split: `--no-default-features` is not the lib build

`cargo build --no-default-features` and the Nix `id-lib` package are different builds. The package README's Cargo quick-start formerly conflated them — disagreements CLI-2 and DES-10 in the book's research — and now uses plain `cargo build` for the lib variant. The facts, from `Cargo.toml` and `flake.nix`:

- A default build already has no web UI: `web` is opt-in and is not in `default`.
- `cargo build --no-default-features` removes `sandbox` and `world` as well. `id match-audit` and the four sandbox-gated `world` subcommands leave the parser entirely, and every other `id world` command fails at runtime.
- The Nix `id-lib` package sets `buildNoDefaultFeatures = true` and then re-adds exactly the two default features, with `buildFeatures = ["sandbox", "world"]`. The shipped lib build is therefore the default-feature build without `web` and without `ssh` — the same feature set as a plain `cargo build`.

The Nix `id-web` package, the flake's default package, builds with `buildFeatures = ["web", "sandbox", "ssh", "world"]`. Both packages install the same single binary as `bin/id`. See [Release and rollout](release-and-rollout.md).

## Web assets

The web variant embeds the frontend at compile time, so the frontend is built first. The justfile's `web` recipe (alias `web-build`) runs `./scripts/build.sh assets`; `web-assets-force` bypasses the freshness check:

```sh
cd web && bun install && bun run build
```

The `web/package.json` build script is three steps:

| Script | Command |
|---|---|
| `build:js` | `bun build src/main.ts --outdir=dist --minify --sourcemap=external --entry-naming=[name].[hash].js` |
| `build:css` | `node_modules/.bin/tailwindcss -i src/input.css -o dist/styles.css --minify` |
| `build:manifest` | `bun run scripts/build-manifest.ts` — renames the stylesheet to its hashed name and writes the asset manifest the templates consume. |

Output lands in `web/dist`. The root package's `build.rs` enforces the ordering: with the `web` feature on, a release-profile build fails when `web/dist` is missing or holds nothing but its `.gitkeep` placeholder, and the failure message names the `bun install && bun run build` command above. A non-release build creates the directory when it is absent, so `cargo check` and `cargo test` compile on a clean checkout.

## Testing

Tests run under cargo-nextest where the justfile uses it. The recipes:

| Recipe | What it runs |
|---|---|
| `just test-rust` | `cargo nextest run --all-features`, then `cargo test --all-features --doc` for the doc tests nextest does not run. Alias: `test-cargo`. |
| `just test` | `test-rust` plus `test-web-unit` plus `test-web-typecheck`. |
| `just test-sandbox` | `cargo nextest run --all-features -E 'not test(serve_tests)'` — the full Rust suite minus the tests that bind and listen on sockets, which the Nix sandbox forbids. |
| `just test-full` | `cargo nextest run --all-features --no-fail-fast`, then the `regression_replay` test target, then the `tuple_properties` target with `PROPTEST_CASES` defaulting to `4096` and `ID_TUPLE_FUZZ_CASES` defaulting to `20000`. |
| `just test-unit` | `cargo nextest run --all-features --lib`. Alias: `test-lib`. |
| `just test-crate-integration` | Five named integration targets in sub-crates the root shards do not cover: `id-caps` `roc_import_policy`, `id-cli` `aliases`, `id-core` `probe_petname_load`, `id-store` `store_traits`, `id-web` `web_router_security`. |
| `just test-native-shell` | `cargo nextest run -p id-native-shell`. A separate recipe because the crate is not a default member. |
| `just test-hw-shell` | `cargo nextest run -p id-hw-shell`. Same reason. |
| `just test-rust-examples` | `cargo test` against `examples/rust-world/Cargo.toml`, `examples/rust-counter/Cargo.toml`, `examples/rust-cells/Cargo.toml` and `examples/rust-orbit/Cargo.toml` — standalone crates outside the workspace. |
| `just test-integration` | `cargo nextest run --all-features --test cli_integration`. Alias: `test-int`. |
| `just test-web-unit` | `cd web && bun run test` (Vitest), then the build-manifest shell test. |
| `just test-web-typecheck` | `cd web && bun run typecheck` (`tsc --noEmit`). |
| `just roc-test` | `bash scripts/roc-test.sh` — the Roc library `expect` tests with the pinned Roc compiler. |
| `just test-e2e` | A `build` first, then one Playwright invocation per spec file in `e2e/tests/`, each with a fresh ephemeral server. `test-e2e-chromium` and `test-e2e-firefox` run one browser project each. |
| `just crash-gate` | `cargo test --release --features web --test crash`, skipping the `write_corpus` tests. Four split recipes — `crash-gate-sampled`, `crash-gate-sweep`, `crash-gate-control`, `crash-gate-ingest` — partition the same tests by name filter for the pull-request gate. |

Run one test by name with the recipe `test-one`, which takes the test name as its single argument and runs `cargo nextest run --all-features -E 'test(<NAME>)'` with the name substituted. Example with toy data: `just test-one my_test`.

### Checks: just and Nix

| Recipe or command | What it runs |
|---|---|
| `just cargo-check` | `cargo check --all-targets --all-features`. |
| `just clippy-lint` | `cargo clippy --all-targets --all-features`. `clippy-native-shell` runs clippy for the non-default `id-native-shell` member separately. |
| `just fmt-check` | `cargo-fmt-check` (`cargo fmt -- --check`) plus `web-fmt-check` (Biome format over the web and e2e sources). |
| `just ci` | The read-only CI set, in recipe order: `cargo-fmt-check`, `web-fmt-check`, `cargo-check`, `clippy-lint`, `clippy-native-shell`, `web-lint`, `test-sandbox`, `test-native-shell`, `test-hw-shell`, `test-rust-examples`, `test-web-unit`, `test-web-typecheck`, `roc-test`, `doc`, `build`, `release`. |
| `just check` | `fix` first (auto-fix formatting and lints), then `ci`. Alias: `check-all`. |
| `just doc` | `cargo doc --no-deps --document-private-items`. |
| `just test-nix` | `nix flake check` in `pkgs/id`. Aliases: `check-nix`, `nix-check`. |

The flake's `checks` set in `pkgs/id/flake.nix` exposes the CI set as individual named checks — among them `cargo-fmt-check`, `clippy-lint`, `clippy-native-shell`, `web-lint`, `test` (which runs the `test-sandbox` recipe), `test-unit`, `test-crate-integration`, `test-native-shell`, `test-hw-shell`, `test-rust-examples`, `test-int`, the four integration shards `test-shard-0` to `test-shard-3`, `roc-test`, `roc-examples`, `doc`, `cargo-check`, `prod-lines-check`, the five `crash-gate` checks and `just-recipes-drift` — plus NixOS VM checks for serve and end-to-end tests on Linux. The `default` check runs the `ci` recipe.

Two counts once circulated for this check set and disagreed — disagreement DES-4 in the book's research: `ARCHITECTURE.md` stated 11 CI checks in its build-system section and 27 checks in its testing section, and the justfile's `test-nix` comments also stated 27. All three were wrong. Measured on 2026-11-23 at `06450991e33`, `checks.x86_64-linux` has 50 attributes, from `nix eval '.#checks.x86_64-linux' --apply 'x: builtins.length (builtins.attrNames x)'` in `pkgs/id`; `ARCHITECTURE.md` now records that measurement and its method, and the justfile no longer states a fixed total.

At the repository level, `.github/workflows/id-check.yml` (workflow name `id`) runs the fast checks on pushes to `main` as a parallel matrix, one job per check listed in `.github/scripts/id-pr-checks.txt`, and runs the full `nix flake check` on its schedule and on manual dispatch. The end-to-end browser and NixOS VM checks run in `.github/workflows/id-e2e-nightly.yml`.

### Test counts are dated measurements

This book states no standing test totals (disagreement DES-7: no record names how its count was produced, so the package documents no longer state totals either). A count is a measurement of one run on one date, and it is recorded with its date or not at all. One count was supplied to this book's research from a separate work lane: an `id-sync` nextest run reported as 474 passed, attributed to the lane for issue #13291. That count is unverified here — it was not re-run for this chapter, and no date for the run was supplied with it — so it is recorded as a claim to re-measure, not as a fact about the current tree. To produce a current count for one crate, run `cargo nextest run --all-features -p id-sync` in `pkgs/id/` and record the summary line with the date and the commit.

## Running a node while developing

The serve recipes in the justfile:

| Recipe | What it runs |
|---|---|
| `just serve` | The web assets, then `cargo run --features web -- serve --web` with any extra arguments appended. |
| `just serve-lib` | `cargo run -- serve` with any extra arguments appended — no web UI. |
| `just serve-new` | A new named instance: `cargo run --features web -- --new=<NAME> serve --web --port 0`. An empty name asks for a random 8-hex-character instance name. |
| `just repl` | `cargo run -- repl`. |
| `just kill-serve` | Kills any running `id serve` processes, waits, then serves. This is also the justfile's default recipe: bare `just` restarts the server. |

`id serve` itself defaults to web port `3000` on bind address `127.0.0.1`; the full flag set is in [serve and publications](../reference/cli-serve.md).

Two port numbers are working conventions for shared nodes, not constants in the `pkgs/id` source — a search of the source tree for this chapter did not establish either number in code, so both are unverified as code facts and are recorded here as operator convention only: a live demo node is run on loopback port `3210`, kept separate from any real data, and the operator's production node is run on port `3005`. Neither number is a default; a stock `id serve --web` with no `--port` flag listens on `3000`.
