# Repository layout

Source: `pkgs/atlas-search/` in https://github.com/developing-today/code/blob/main/pkgs/atlas-search/Cargo.toml, https://github.com/developing-today/code/blob/main/pkgs/atlas-search/flake.nix and https://github.com/developing-today/code/blob/main/pkgs/atlas-search/default.nix, and the design of record at `doc/2026-11-04T08-07-13Z_design_atlas_binary_service/` — provenance only; this chapter is complete without following it.

Everything Atlas lives in one directory: `pkgs/atlas-search/` in the
`developing-today/code` repository. It is a **self-contained workspace** on the
`pkgs/id` pattern — its own Cargo workspace, its own Nix sub-flake, its own
pinned Rust toolchain — so the package can be built, tested and versioned
without the rest of the repository's build machinery. Other packages consume
it as it stands: `pkgs/fleet` path-depends on its crates, its workspace
manifest and its `schema/` directory, and reuses its vendored ONNX Runtime
package, so moving a file inside this tree can break a build outside it.

## The tree

```
pkgs/atlas-search/
  atlas_search.py        the Python reference implementation
  Cargo.toml  Cargo.lock the workspace manifest and lockfile
  rust-toolchain.toml    the pinned Rust toolchain (the sub-flake reads it)
  flake.nix  flake.lock  the package sub-flake
  default.nix            the package derivation (called by the sub-flake
                         and by home configuration)
  book.toml  book/       this book (config at the package root, sources in book/)
  crates/                the four Rust crates
  worker/                the Cloudflare Worker (TypeScript)
  eval/                  the evaluation harness scripts and golden sets
  schema/                versioned SQL DDL for the staging tier
  tools/                 auxiliary tools that live with the package
  fixtures/              parity fixtures and their generators
  test-vectors/          pinned known-answer test vectors
  onnxruntime/           the vendored ONNX Runtime Nix package
  docs/                  the three package-local design/operations notes
  tests/                 package-level tests (currently one Python test)
```

### `crates/` — the four crates

The workspace members, declared in the root `Cargo.toml` (edition 2024,
`rust-version` 1.96, licence MIT OR Apache-2.0):

| Crate | What it holds |
|---|---|
| `crates/atlas-core` | Search semantics as a library: filters and sensitivity ceilings, the FTS and vector branches, RRF fusion, the result envelope, the Turso and local backends, plus staging, classification, the ingest gate, evaluation and trace machinery. It also carries four of the six binaries, auto-discovered from `src/bin/`: `atlas-classify`, `atlas-eval`, `atlas-github-comments` and `atlas-sweep`. See [atlas-core: the engine as a library](../library/atlas-core.md). |
| `crates/atlas-embed` | The Snowflake Arctic embedder: dynamically loaded ONNX Runtime plus the model's own Unigram tokenizer. A single `src/lib.rs`, plus the Gate A parity test. See [atlas-embed: embeddings as a library](../library/atlas-embed.md). |
| `crates/atlas-proto` | The wire contract shared by the service and its clients: the job-protocol types and `query-parameters-v1.json`, the versioned file the query defaults are derived from. See [atlas-proto: the wire contract](../library/atlas-proto.md). |
| `crates/atlas-search` | The `atlas-search` binary: the CLI, the via-service client and `serve`, in `src/main.rs`, `src/server.rs` (with `src/server/` submodules), `src/service.rs` and `src/fetch_model.rs`, plus the Python-shaped JSON writer (`src/pyjson.rs`) and envelope renderer (`src/render.rs`). One extra binary lives here: `atlas-stage`, from `src/bin/atlas-stage.rs`. |

The full crate inventory is [The crates](../library/crates.md).

### `worker/` — the Cloudflare Worker

The hosted surface, in TypeScript: `src/index.ts` (routing and the gate),
`src/mcp.ts` (the MCP endpoint), `src/search.ts` (the one search execution
path), `src/service.ts` (the client for the laptop service), `src/cache_do.ts`
(the per-query Durable Object cache), `src/hot_cache.ts` (the D1 hot-cache
storage layer), `src/key.ts`, `src/space.ts` and `src/body.ts`. Beside the
sources: `test/` (the vitest suite), `migrations/` (the D1 migration
`0001_hot_cache.sql`), `package.json`, `wrangler.toml` (local development and
tests) and `cloudflare.config.ts` (the deploy source of truth — the two
configuration files must be kept in step). Its own `README.md` is the
Worker's operator notes. See [The service and the Worker](../concepts/service-and-worker.md).

### `eval/` — evaluation

The offline evaluation programme: `eval3.py` (the Python reference runner),
`gate5733.py` and its recorded results, the boundary-drift and overlap
experiments, an `eval/README.md`, and `fixtures/` holding the golden query
sets (`golden-311.json`, `heldout-nearexact-54.json`). See
[Evaluation: golden sets and gates](../concepts/evaluation.md).

### `schema/` — versioned DDL

Exactly three versioned SQL artifacts: `atlas_stage_v1.sql` (the staging
database schema), `stage_registry_v1.sql` (the staging registry) and
`gate_records_v1.sql` (the ingest-gate record). The versioning rule is stated
in the files and in the code that embeds them: bump only by adding a new
`vN` artifact; never edit an applied version in place. `atlas-core` embeds
these files with `include_str!`, which is why the Nix dependency layer in
[Building and testing](building.md) must include `schema/` in its source
tier. See [Data stores and schemas](../reference/data-stores.md).

### `tools/` — auxiliary tools

One tool: `note_promotion_sweep.py`, the pointer-stub sweep for note
promotions, with its tests in `tools/tests/`. Note what it is not: it is a
notes-placement tool that lives in this package, not a corpus-ingest tool.

### `fixtures/` — parity fixtures

The recorded evidence the parity gates run against: `embed-probes.json`
(Gate A embedder probes), `core-expectations.json` (Gate B envelopes and SQL
strings recorded from the Python reference), the miniature fixture corpus
(`fixture_corpus.sql` + `fixture_spec.json`), `eval-smoke.json`, the
generators that recorded them (`gen_fixtures.py`, `gen_core_fixtures.py`,
`make_fixture_db.py`), and `smoke_check.py` — the script the Nix package
runs as its install check.

### `test-vectors/` — known-answer vectors

One file today: `content-hash-v1.json`, pinning the classification
manifest's content hash with known-answer vectors — and recording, just as
explicitly, that the chunk `content_hash` producer is *not* pinned there
and that interop claims for it are forbidden until it is.

### `onnxruntime/` — the vendored runtime

A small Nix package (`default.nix`, plus a musl patch) vendoring the pinned
ONNX Runtime release in-tree (code#6765, the user's ruling 2026-11-08) rather
than taking the ambient nixpkgs version: the corpus and the Gate A fixtures
were embedded and certified under that release, so the library the binary
loads must be the same one. The main package compiles in
`ATLAS_SEARCH_ORT_LIB_DEFAULT` pointing at this package's
`libonnxruntime.so`. See [Embeddings: Snowflake Arctic and the parity contract](../concepts/embeddings.md).

### `docs/` — package-local notes

Exactly three files: `external-set-enumeration.md` (the external-corpus
candidate list, #6023), `memory-integration-design-8567.md` (the memory
integration decision, #8567) and `owner-inventory-matcher.md` (the Layer 3
owner-inventory matcher note). The large design archive — including the
design of record — does **not** live here; it lives in the repository-root
`doc/` directory.

### `atlas_search.py` — the Python reference

The Python implementation the Rust crates port, kept permanently as the
eval instrument and parity reference. The in-repo copy is 1,629 lines. (The
design of record describes a 1,177-line Python file; that number is true of
the older hatch-personal copy, not of this one — where a chapter cites "the
Python reference", it means this file.) Its own header points at a
`README.md` in this directory; no such file existed before this book — the
README it describes is the turso-atlas README in the laptop working tree.
See [atlas_search.py: the Python reference implementation](../reference/python-implementation.md).

### The sub-flake

`flake.nix` + `flake.lock` + `rust-toolchain.toml` make the package
independently buildable:

- Inputs: `nixpkgs` (nixos-unstable, the same track as `pkgs/id`), the
  `systems` and `flake-utils` helpers, `rust-overlay`, and `crane` — the
  build library that splits dependency compilation out of the package
  build (#8392), pinned to the revision the repository's root lock already
  carries.
- The toolchain is read from `rust-toolchain.toml` via rust-overlay
  (`pkgs.rust-bin.fromRustupToolchainFile`), so the Nix build, the dev
  shell and a rustup user all resolve the same pinned compiler.
- Outputs: `packages.atlas-search` (also `default`), `checks.default`, and
  `devShells.default` carrying the pinned toolchain.

`default.nix` is the derivation both routes call: the sub-flake passes it a
crane library and gets the split build; home configuration calls it without
one and gets the monolithic `buildRustPackage` build — same sources, same
checks, no split. See [Building and testing](building.md).

## How the built package differs from the design of record

The design of record (`doc/2026-11-04T08-07-13Z_design_atlas_binary_service/`,
§4) proposed this package as exactly the four crates plus `fixtures/` and a
package-local `doc/` directory, packaged with
`rustPlatform.buildRustPackage`. The four crates exist exactly as designed.
Everything around them grew beyond the tree in the design — state it as
fact, not as drift to be apologised for:

- `worker/`, `eval/`, `schema/`, `tools/`, `tests/` and `test-vectors/`
  were **not in the 4-crate design at all**. Neither was `onnxruntime/`
  (the design assumed nixpkgs' runtime; the vendored package is the later
  #6765 ruling), nor `atlas_search.py` at the package root (the design
  treated the Python as an external source of truth to port from; the
  migration, #5670, brought it in as the permanently kept reference).
- The package-local documentation directory is `docs/` (plural, three
  files), not the `doc/` (singular) the design's tree shows; there is no
  package-local `doc/`.
- A book — `book.toml` + `book/` — is the newest addition, added by the
  work this chapter is part of (#13251).

A chapter, a script or a review that reprints the design's tree as the
current layout is wrong. This chapter is the layout of record; when the
tree changes, this chapter changes in the same change — see
[Keeping this book current](keeping-this-book-current.md).
