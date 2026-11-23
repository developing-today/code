# Building fleet

Fleet is one Rust workspace that produces one binary. Capability variants are selected by Cargo features; the installed binary is always named `fleet`, and hyphenated names (`fleet-serve`, `fleet-connect`, `repo-queue`, and so on) are argv0 personalities of that one binary, not separate artefacts. See [Repository layout](layout.md) for the workspace map and [Register a domain or capability](register-a-domain.md) for how features, capabilities, and profiles fit together.

## Build with Nix (the normal route)

1. From the repository root, build the package you need:

   ```sh
   nix build .#fleet
   ```

2. To build the full smoke set in one go:

   ```sh
   nix build .#fleet .#fleet-serve .#fleet-connect .#fleet-connect-static
   ```

3. Run the smoke suite over the results (see [Testing](testing.md)):

   ```sh
   ./pkgs/fleet/smoke.sh ./result ./result-1 ./result-2 ./result-3
   ```

The builder is `pkgs/fleet/default.nix`. Its internal `mkFleet` function takes a package name, a Cargo feature set, an optional argv0 alias, an optional size gate, a platform, and a static flag. Cargo flags are always `-p fleet`, plus `--no-default-features` when the feature set is not the default, plus `--features <comma-joined>` when the set is non-empty — so the Nix attribute _is_ the Cargo feature set.

### The six attributes

`default.nix` returns six attributes:

| Attribute              | Cargo features                 | Platform                         | Alias link      | Size gate |
| ---------------------- | ------------------------------ | -------------------------------- | --------------- | --------- |
| `fleet`                | default (= `full`)             | glibc                            | —               | no        |
| `fleet-serve`          | `serve`                        | glibc                            | `fleet-serve`   | no        |
| `fleet-connect`        | none (`--no-default-features`) | glibc                            | `fleet-connect` | **yes**   |
| `fleet-brokers`        | separate build, see below      | glibc                            | —               | no        |
| `fleet-static`         | default (= `full`)             | musl (`pkgsStatic.rustPlatform`) | —               | no        |
| `fleet-connect-static` | none                           | musl (`pkgsStatic.rustPlatform`) | `fleet-connect` | **yes**   |

Notes on each:

- `fleet` is the full build: every domain and every serve role in the registry runs for real in it.
- `fleet-serve` and `fleet-connect` install side by side under their hyphenated names as a symlink to the same binary (`ln -s fleet $out/bin/<alias>` in `postInstall`). Invoking through that name narrows the surface to that variant's profile — narrowing only, never widening.
- `fleet-static` is the full profile linked for foreign soil: the artefact non-Nix hosts install, and the artefact `publish-fleet.sh` ships. See [Release and rollout](release-and-rollout.md). A glibc-dynamic build names Nix store paths a foreign host does not have, which is why the static exists.
- `fleet-connect-static` is the same idea for the connect profile.
- `fleet-brokers` is a separate `buildRustPackage` invocation on `-p fleet-queue --features client,queue-broker,secrets-broker --bins`. It produces the three standalone artefacts `repo-queue`, `queue-broker`, and `secrets-broker` from `fleet-queue/src/bin/*.rs` (each binary declares its `required-features` in `fleet-queue`'s `Cargo.toml`). Its header calls it the migration's rollback inventory: the standalone brokers retire only at the epic's close, not before.

There is no `deploy` package in `pkgs/fleet/default.nix`. Deployment tooling is its own package tree, `pkgs/deploy/`. See [Release and rollout](release-and-rollout.md).

### The D7 2 MiB connect gate

Size, not speed, is the release profile's job. `pkgs/fleet/Cargo.toml` sets `[profile.release]` to `opt-level = "z"`, `lto = true`, `codegen-units = 1`, `strip = true`, against a design budget for the connect variant of target ≤ 1.5 MiB and hard ≤ 2 MiB, stripped.

The hard bound is enforced in the build. For size-gated attributes, `postFixup` in `default.nix`:

1. Strips explicitly and fully with `${binutils}/bin/strip` (the nixpkgs fixup phase only strips debug symbols, leaving the symbol table behind).
2. Reads the binary size with `stat -c %s`.
3. Fails the build if the size exceeds `connectMaxBytes = 2 * 1024 * 1024` (2,097,152 bytes).

The gate is enforced a second time in `smoke.sh`, which runs both connect flavours under `env -i` and checks them against the same bound. Anything that would push the connect build over the gate — for example the `opencode-probe` feature, whose `ureq` + rustls stack would break it, or the optional `fleet-secrets` dependency — is kept out of the connect feature set by construction.

### The crane deps split, and why the statics stay monolithic

When the root flake's `make-fleet` passes a `craneLib`, `default.nix` splits the build: dependencies are built once via crane's `buildDepsOnly`, keyed on the manifests and the lockfile, so a source change rebuilds only the workspace crates instead of the whole dependency tree.

Three attributes deliberately stay on the monolithic `buildRustPackage` path:

- `fleet-static` and `fleet-connect-static` — under `pkgsStatic`, crane applies `crt-static` to host build scripts, and a libc build script then fails to link.
- `fleet-brokers` — its `--bins` build and its lib-tests check split do not map onto crane's single `cargoExtraArgs`.

The build source is the whole `pkgs/` tree (a fileset union of `pkgs/fleet`, `../atlas-search/Cargo.toml`, `../atlas-search/crates`, and `../atlas-search/schema`), because `fleet-search` consumes the atlas crates by path. `sourceRoot` is the `fleet` directory inside it and `cargoLock` is `./Cargo.lock`.

The check phase runs `cargoWithProfile test --locked` — deliberately flagless and workspace-wide. Crane's default would append `-p fleet` and narrow the run to the binary crate's tests alone. `preCheck` points `HOME` at a build-temporary directory because some atlas-search server tests write under `$HOME/.local/state/atlas-search` and the sandbox `HOME` is not writable.

One environment detail: non-static, non-connect builds bake `ATLAS_SEARCH_ORT_LIB_DEFAULT` to the pinned ONNX Runtime library (ONNX Runtime is `dlopen`'d at runtime). The statics get no such default — a glibc `libonnxruntime` cannot be `dlopen`'d from a static binary — so the search-serve role in a static binary needs `ATLAS_ORT_LIB` supplied at runtime.

## Build with Cargo (inside the workspace)

1. Change into `pkgs/fleet/`.
2. Build the default (full) variant:

   ```sh
   cargo build --release
   ```

3. Build a narrowed variant by selecting features exactly as `default.nix` does. For the connect variant:

   ```sh
   cargo build --release -p fleet --no-default-features
   ```

   For the serve variant:

   ```sh
   cargo build --release -p fleet --no-default-features --features serve
   ```

4. Run the workspace tests the same way CI and the Nix check phase do:

   ```sh
   cargo test --workspace
   ```

Feature sets are defined in `crates/fleet/Cargo.toml` under `[features]`: `default = ["full"]`; `full` lists every domain feature plus `opencode-probe`; `serve` is the broker-side subset shipped as one feature for the Nix build. `proxy-connect` is not a feature at all — it is compiled into every variant.

### The toolchain pin

`pkgs/fleet/rust-toolchain.toml` pins channel `1.99.1` with components `rustfmt` and `clippy` and profile `default`. Its header warns: do not delete the file — it is required for Nix builds — and a channel bump may cause rustc itself to build, so check that nixpkgs has the new channel cached before committing a bump. The nightly CI workflow installs this same pin; see [Testing](testing.md).

`crates/fleet/build.rs` bakes the version facts into the binary: `FLEET_SOURCE_REV` (from the environment, else `git rev-parse`, else the honest string `unknown` — the Nix sandbox has no `.git`) and `FLEET_DERIVED_VERSION` (derived, never hand-maintained, from `git describe` over `fleet/v*` tags via the shared derivation in `src/derived_version.rs`, with fallbacks).

## Serialising heavy builds: `fleet-build-lock`

`pkgs/fleet/fleet-build-lock` serialises heavy builds (Cargo, Nix) so a host is never running more of them than it can bear. It implements the fleet build policy of at most `FLEET_BUILD_SLOTS` heavy builds at once on the host running it — **2 slots by default** — using `flock` slot files under `FLEET_BUILD_LOCKDIR` (default `~/.fleet-build-locks`).

Properties that matter:

- A build _waits_ for a free slot. Load alone never parks work; the lock queues, it does not reject.
- It sets `CARGO_BUILD_JOBS` to half the host's cores unless that variable is already set.
- Usage is `fleet-build-lock [--] command [args…]`, run on the building host (the laptop), for example via the laptop agent. `fleet shell --build` takes a slot through it automatically.

```sh
fleet-build-lock -- nix build .#fleet
fleet-build-lock -- cargo test --workspace
```
