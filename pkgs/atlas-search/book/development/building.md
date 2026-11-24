# Building and testing

Source: `pkgs/atlas-search/flake.nix`, `pkgs/atlas-search/default.nix` and `pkgs/atlas-search/rust-toolchain.toml` in https://github.com/developing-today/code/blob/main/pkgs/atlas-search/flake.nix, and the workflows `.github/workflows/atlas-search-pr.yml`, `.github/workflows/atlas-search-worker-tests.yml` and `.github/workflows/fleet-search-parity-guard.yml` — provenance only; this chapter is complete without following it.

There are two build routes and they must agree: the Nix package build (what
ships) and cargo in the dev shell (what you iterate with). The package
layout these routes operate on is [Repository layout](layout.md).

## The Nix build

From the repository root:

```sh
nix build path:./pkgs/atlas-search
```

That builds the package through its own sub-flake (`pkgs/atlas-search/flake.nix`),
which reads the pinned toolchain from `rust-toolchain.toml` via rust-overlay
and calls `default.nix`. The same `default.nix` is called by home
configuration (`pkgs.callPackage ../../pkgs/atlas-search { … }`) when the
laptop's service package is built — with one difference in build shape,
covered under the crane split below.

What the package build does beyond compiling:

- **ONNX Runtime is never downloaded**, at build time or run time. The
  `ort` crate is built with dynamic loading against the vendored in-tree
  `onnxruntime/` package, and `ATLAS_SEARCH_ORT_LIB_DEFAULT` is compiled
  into the binary pointing at that package's `libonnxruntime.so` — which
  also puts `libstdc++.so.6` and `libz.so.1` in the runtime closure.
- **The test suite runs in the check phase** (`doCheck = true`), with
  `HOME` redirected to the build's temp dir (the server tests write traces
  under the state directory and fail against the sandbox's unwritable
  home) and `git` provided (the atlas-core detection tests spawn it).
- **An install check proves the loader lesson**: `fixtures/smoke_check.py`
  runs one FTS-only and one embedded query against the installed binary
  with no `ATLAS_ORT_LIB` set, so a closure that cannot load the runtime
  fails the build instead of failing on the laptop.

### The crane dependency split (#8392)

When a crane library is supplied — the sub-flake supplies one — dependency
compilation is split out of the package build. Crane's `buildDepsOnly`
compiles every registry dependency as its **own derivation**, keyed only on
what can change dependency compilation. Two source tiers make that keying
honest:

| Tier            | Contents                                         | Feeds                                 |
| --------------- | ------------------------------------------------ | ------------------------------------- |
| Dependency tier | `Cargo.toml`, `Cargo.lock`, `crates/`, `schema/` | `buildDepsOnly` only                  |
| Package tier    | the dependency tier plus `fixtures/`             | the package build and its check phase |

`schema/` is in the dependency tier because atlas-core embeds the staging
DDL from it with `include_str!` — a schema edit really can change a crate
build. `fixtures/` is deliberately **not**: the test suite reads fixtures
by relative path, so the check phase needs them, but a fixture edit must
not rebuild the whole dependency tree. `eval/`, `worker/` and any in-tree
`target/` are in neither tier.

The consequence a contributor feels: a change to workspace sources rebuilds
just the workspace crates; a change to `Cargo.lock` rebuilds the dependency
layer, once. Callers that do not pass a crane library (home configuration)
keep the monolithic `buildRustPackage` build — same sources, same checks,
no split.

## Cargo in the dev shell

Enter the sub-flake's dev shell, which carries exactly the pinned
toolchain:

```sh
cd pkgs/atlas-search
nix develop
cargo test --all-targets
cargo test -p atlas-core
cargo test -p atlas-search
```

Per-crate runs mirror what CI does (below). The toolchain pin lives in
exactly one place: `rust-toolchain.toml` reads `channel = "1.99.1"` (with
`rustfmt` and `clippy` components, profile `default`), mirroring the
pkgs/id pin. Every other source states the same figure: CI installs
**1.99.1** (`rustup toolchain install 1.99.1 --profile minimal`), the
`flake.nix` comment describes the pin as "the pinned toolchain
(`rust-toolchain.toml`, channel 1.99.1) — the same rustup toolchain the
step 1–4 gates ran under on the laptop", and the laptop's home
configuration builds its Atlas toolchain from the same file. (Until
2026-11-23 the CI workflow and the comments still said 1.99.0, so CI
tested a different compiler than the Nix build shipped; #13286 records
the alignment.) The file also carries the warning that changing the
channel may cause rustc itself to build, so a channel bump is committed
only once nixpkgs has the new compiler cached.
Separately, the workspace manifest's `rust-version = 1.96` is a _minimum_
(the oldest compiler the code claims to support), not the pin.

One release-profile rule with no exceptions: the release profile carries
no fast-math-style flags. The embedder's numerics are a parity contract
(Gate A, in [Embeddings: Snowflake Arctic and the parity contract](../concepts/embeddings.md)),
and the comment in `Cargo.toml` exists so a future profile edit has to
argue with it.

## What CI actually runs

House policy is CI on `main` only: workflows are not merge gates, and
Atlas's run on the nightly-plus-dispatch pattern the user set on 2026-11-16.
State the triggers as they are in the files, not as the filenames or
headers claim — all three Atlas workflows disagree with their own
presentation somewhere, and the `on:` block is the truth in each case.

### `atlas-search-pr.yml` — the Rust checks

- **Triggers: nightly schedule (`17 7 * * *`) and `workflow_dispatch`
  only.** Despite the `-pr` filename, no pull-request or push trigger
  exists in the file; the header cites the 2026-11-16 nightly-main-CI
  policy, and the name is the stale part.
- One job ("basic: atlas-search Rust checks"), `ubuntu-latest`, 30-minute
  timeout, `contents: read`: checkout, install the 1.99.1 toolchain, then
  four test steps — one per crate, each of the shape:

  ```sh
  cargo +1.99.1 test --manifest-path pkgs/atlas-search/Cargo.toml --all-targets -p atlas-core
  ```

  for `-p atlas-search`, `-p atlas-core`, `-p atlas-proto`, `-p atlas-embed`.

- There is **no `cargo fmt --check`**: formatting is not a merge gate in
  this repository; it is auto-fixed on main.
- atlas-embed's Gate A model test stays `#[ignore]`d in this venue: it
  requires `ATLAS_ORT_LIB` and the pinned model snapshot, and its verdict
  is version-matched to ONNX Runtime 1.31.1 — it runs on the laptop, not
  in CI.

### `atlas-search-worker-tests.yml` — the Worker vitest suite (#7683)

- **Triggers as written in the file: `workflow_dispatch` and the nightly
  schedule (`17 7 * * *`).** The header prose says the suite runs "on
  pushes to main that touch `pkgs/atlas-search/worker/` and on dispatch" —
  header and trigger disagree within the one file; the trigger block is
  what runs.
- Why it exists: before it, the Worker's vitest suite was run by no
  workflow, flake check or recipe — only by hand via `npm test`.
- One job, `ubuntu-latest`, 15 minutes: checkout, Node 24, then a guard
  step that **fails the run if any of `test/key.test.ts`,
  `test/space.test.ts`, `test/worker.test.ts`, `package.json` or
  `package-lock.json` is missing** — a green run can never be a silently
  skipped suite — then `npm ci` and `npm test` in `pkgs/atlas-search/worker`
  (vitest with `@cloudflare/vitest-pool-workers`). The suite also covers
  the MCP endpoint, the body cap and the hot cache
  (`mcp.test.ts`, `body.test.ts`, `hot_cache.test.ts`).
- Run it locally the same way: `cd pkgs/atlas-search/worker && npm ci && npm test`;
  `npm run typecheck` runs `tsc --noEmit`.

### `fleet-search-parity-guard.yml` — the byte-parity guard

- **Trigger: `workflow_dispatch` only.** Nothing runs it automatically on
  a change to either guarded file; the guard exists, the gating does not.
- Pure bash on `ubuntu-latest`, 5 minutes, no Nix and no toolchain: first
  the guard's own contract test suite
  (`bash .github/scripts/check-fleet-search-parity.test.sh`), then the
  comparison itself
  (`bash .github/scripts/check-fleet-search-parity.sh`).
- What it compares: the fleet-search copies of the Python-JSON writer and
  the envelope renderer —
  `pkgs/fleet/crates/fleet-search/src/{pyjson,render}.rs` against
  `pkgs/atlas-search/crates/atlas-search/src/{pyjson,render}.rs` —
  byte-for-byte outside the pinned parity header in each copy (#5920,
  contract origin #5433). Touch either file and this guard is part of
  your change's verification, dispatch it yourself; see
  [Add a command or a tool](add-a-verb.md).

## The named-oracle rule for tests

Every automated test names its **oracle**: the independent source of truth
its expected values come from, stated in the test or its module doc. A test
whose expectation was produced by running the code under test is not a
test of anything but stability. The Atlas suites already work this way;
new tests follow the same shape:

- The shard fan-out tests run the identical statement over a union store —
  "the monolith exact-scan oracle" — and compare the sharded result
  against it (`crates/atlas-core/tests/shards.rs`).
- The join assembler's oracle is the version text itself: assembled bytes
  must equal the version's own bytes for the paragraph, byte for byte
  (invariant J1).
- The embed-cache tests use a counting raw embedder as "the provider-call
  oracle": repeat queries must keep its call count at 1.
- The staging tests carry an independent hash oracle over the archive's
  own rows and a lossless-partition oracle: chunks in index order rejoin
  to the staged text exactly.
- Parity expectations are recorded from the _other_ implementation:
  `fixtures/core-expectations.json` was recorded from the production
  Python by `gen_core_fixtures.py`, and atlas-core's parity tests must
  reproduce every envelope and SQL string in it. Regenerate a fixture with
  its generator when behaviour changes deliberately; never hand-edit an
  expectation to match new output.

## The golden CLI output tests

`crates/atlas-search/tests/golden/` holds three small text files —
`text_search_flat.txt`, `text_search_grouped.txt` and
`text_get_chunk.txt` — the expected `--text` renderings of a search
(flat and grouped by session) and of a `get-chunk`, recorded from the
miniature fixture corpus and asserted byte-for-byte by the atlas-search
CLI tests (`tests/cli_parity.rs`). The fixture corpus is built so the
goldens also pin the ceiling behaviour: its secret-bearing chunk carries
the query's exact vector and FTS terms and must be **absent** from the
default-ceiling rendering — not down-ranked — and present, at the top,
at a secret-bearing ceiling.

Do not confuse these with the golden _query_ sets. The output goldens pin
how results are **rendered**; golden-311 (311 judged queries, in
`eval/fixtures/`) pins how well the system **retrieves**, and is scored by
the evaluation harness in [Evaluation: golden sets and gates](../concepts/evaluation.md).
Changing rendering touches the first; changing ranking, the model or
chunking touches the second — and a model change is cut over on the
second, per [Change the embedding model](change-the-model.md).
