# The crates

Source: `pkgs/atlas-search/Cargo.toml`, `crates/atlas-embed/Cargo.toml`,
`crates/atlas-core/Cargo.toml`, `crates/atlas-proto/Cargo.toml`,
`crates/atlas-search/Cargo.toml`, `flake.nix` and `default.nix` in
https://github.com/developing-today/code/blob/main/pkgs/atlas-search/ —
provenance only; this chapter is complete without following it.

Atlas is one Cargo workspace of four crates, in `pkgs/atlas-search/crates/`.
The workspace is the whole dependency story: the embedder knows nothing
about search, the wire contract knows nothing about either, the engine
composes the two, and the binary crate composes the engine into commands
and a service. Nothing outside the workspace is needed to build any of it —
the package is self-contained, with its own flake, on the same pattern as
the `pkgs/id` package.

```text
pkgs/atlas-search/Cargo.toml      workspace: edition 2024, rust-version 1.96
  crates/atlas-embed/             the Snowflake embedder (ONNX + tokenizer)
  crates/atlas-core/              search semantics, backends, ingest, eval
  crates/atlas-proto/             the service wire contract + query parameters
  crates/atlas-search/            the binaries: CLI, service, stage tool
```

Workspace facts, common to all four crates: edition 2024, rust-version 1.96,
version 0.1.0, licence MIT OR Apache-2.0. The release profile deliberately
carries no fast-math-style flags: embedder numerics are a parity contract,
and a compiler flag that changed floating-point summation order would break
it silently.

## The map

| Crate          | Role                                                                                                                                                               | What depends on it                                                                                                      | Where its binaries live                                                                                     |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `atlas-embed`  | The Snowflake `snowflake-arctic-embed-m-v2.0` embedder: dynamically loaded ONNX Runtime, the model's own Unigram tokenizer, CLS pooling, full 768-dimension output | `atlas-core`                                                                                                            | None — library only. It ships a Gate A test and a timing example instead                                    |
| `atlas-core`   | The engine: search semantics, both backends, shard fan-out, staging, classification, the ingest gate, evaluation, traces and feedback                              | `atlas-search`; also the fleet binary's search domain, which consumes the workspace crates as-is                        | Four binaries in its own `src/bin/`: `atlas-classify`, `atlas-eval`, `atlas-github-comments`, `atlas-sweep` |
| `atlas-proto`  | The shared HTTP job-protocol types for the service and its via-service clients, plus the versioned query-parameter contract                                        | `atlas-core` and `atlas-search`; the server and the CLI client both depend on it so their wire definitions cannot drift | None — serde types only: no HTTP, no storage, no search policy                                              |
| `atlas-search` | The process surface: the one-shot CLI, the via-service client, the loopback HTTP service, model fetching, and the staging tool                                     | Nothing in the workspace; it is the top of the graph. Outside the workspace, the fleet binary re-homes its serve half   | `atlas-search` from `src/main.rs`, and `atlas-stage` from `src/bin/atlas-stage.rs`                          |

Dependency direction is one-way throughout: `atlas-search` depends on
`atlas-core` and `atlas-proto`; `atlas-core` depends on `atlas-embed` and
`atlas-proto`; `atlas-embed` and `atlas-proto` depend on no sibling crate.
A change to the wire contract therefore recompiles everything above it, and
a change to the embedder recompiles nothing below the engine.

## atlas-embed

One file (`src/lib.rs`), two load-bearing dependencies: the `ort` crate
built with dynamic loading and no default features — so it never downloads a
prebuilt ONNX Runtime at build time, which the Nix sandbox forbids — and the
`tokenizers` crate for the model's own tokenizer. Construction is explicit
and eager: loading takes a model directory and a runtime library path,
resolved by the caller, never read from the environment by this crate. Its
whole public surface is the `Embedder` type, its tokenise and embed methods,
and a pairwise-summation helper that replicates NumPy's summation order
bit for bit, because the parity gate grants no tolerance for a
differently ordered accumulation. See
[atlas-embed: embeddings as a library](atlas-embed.md).

## atlas-core

The large crate: 46 public modules and about 26,000 lines in `src/`,
including the shard subpackage. It is a semantics-exact port of the Python
reference implementation — see
[atlas_search.py: the Python reference implementation](../reference/python-implementation.md)
— with the search ceiling and filters applied inside the branch queries,
before any ranking. It owns both data backends (the Turso HTTPS pipeline
backend and the local SQLite publish-file backend), the staging and registry
machinery, the classification operating procedure, and the offline
evaluation harness. Four operator binaries are auto-discovered from its
`src/bin/`, and its examples include a live smoke test, a retrieval proof
and a shard benchmark. The full module map is
[atlas-core: the engine as a library](atlas-core.md).

## atlas-proto

Serde types only. It defines the job protocol — search requests, job
creation and status, result pages, error and health responses, and the embed
request and response — and it embeds the versioned defaults file
`query-parameters-v1.json`, which is the single source the CLI defaults are
derived from (k = 10, pool = 100, RRF k = 10, and the rest of the v1
contract). Sensitivity clamping does _not_ live here: this crate carries the
requested ceiling and the server's reported effective ceiling, and the
server does the clamping. One wart to know: its `Sensitivity`, `SearchMode`
and `VectorBackend` types are wire duplicates of `atlas-core`'s types of the
same names — same spellings and orderings, defined independently in both
crates. See [atlas-proto: the wire contract](atlas-proto.md).

## atlas-search

The binary crate. Its modules are the CLI entry point (`main.rs`), the
service (`server.rs` with its `server/` submodules), the via-service client
(`service.rs`), model fetching (`fetch_model.rs`), and the two output
writers (`pyjson.rs`, the Python-JSON writer, and `render.rs`, the envelope
renderer) whose byte parity the fleet port is guarded against. It builds
two binaries: `atlas-search` itself — the access layer with the `search`,
`get-chunk`, `serve`, `init`, `fetch-model`, `traces` and `feedback` verbs —
and `atlas-stage`, the staging-unit tool. The fleet binary's serve role is a
maintained copy of this crate's `server.rs`, and its search client delegates
standalone searches to the installed `atlas-search` artifact; neither fact
changes this crate's place at the top of the workspace graph.

## The self-contained sub-flake

`pkgs/atlas-search/` is a self-contained package on the `pkgs/id` pattern:
its own Cargo workspace, its own `flake.nix` and `default.nix`, and its own
`rust-toolchain.toml`, from which consumers read the toolchain. It also
carries, as siblings of `crates/`, everything the crates need at build or
test time: `schema/` (the versioned staging, registry and gate DDL),
`fixtures/` (the parity fixtures), `eval/` (the Python evaluation lineage),
`worker/` (the Cloudflare Worker), `onnxruntime/` (the vendored ONNX Runtime
package the build pins and points the binary at, rather than an ambient
system runtime), the book itself, and `atlas_search.py` at the package root.

Two consequences follow. First, the workspace can be built, tested and
vendored without the rest of the repository: the fleet package path-depends
on this package's crates, workspace manifest and schema directory, and
reuses its vendored ONNX Runtime, precisely because the package is
self-contained. Second, the model is _not_ part of the package: the
Snowflake model snapshot is data, provisioned into a state directory by the
`fetch-model` verb and verified against hashes recorded in the repo — see
[Installation and provisioning](../getting-started/installation.md).

Worked example — building one crate and running one binary's tests from the
package root:

```text
cargo build -p atlas-core
cargo test -p atlas-search --all-targets
nix build .#atlas-docs        # this book
```

## Failure shapes

- A missing chapter file is a book build failure, never a stub: the book
  configuration sets `create-missing = false`.
- The embedder's Gate A parity test is ignored in ordinary CI: it requires
  the pinned model snapshot and a version-matched ONNX Runtime, and against
  a different runtime version it reports not-applicable rather than a parity
  verdict.
- Because `atlas-proto` and `atlas-core` define their shared vocabulary
  types independently, a spelling change made in only one crate compiles
  cleanly and drifts the wire. The contract file and the parity tests are
  the guards; there is no shared definition to lean on.
