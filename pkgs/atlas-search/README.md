# atlas-search

Atlas is a unified search index over every corpus Drew produces:
provenance-first storage, tiered sensitivity, and hybrid (full-text +
vector) retrieval. This package is the whole system in one workspace —
the retrieval engine, the command-line tools, the laptop service, and
the Cloudflare Worker that fronts it for hosted clients.

Program epic: #8983. Design of record:
`doc/2026-11-04T08-07-13Z_design_atlas_binary_service/` in the
repository-root `doc/` archive.

## The book

[`book/`](book/) (mdBook sources, [`book.toml`](book.toml) at this
package's root) is the complete, self-contained reference for the
system — every command, flag, default, schema, protocol and
operational procedure, plus the forward design. It is published at
https://developing-today.github.io/code/atlas/ and builds with
`nix build .#atlas-docs`. Start there; this README is only the
package's front door.

## Layout

- `crates/atlas-embed` — the embedding library (Snowflake
  arctic-embed-m-v2.0, native ONNX, parity-certified against the
  Python reference).
- `crates/atlas-core` — the engine: corpus stores, staging and
  promotion, classification, retrieval, evaluation, feedback.
- `crates/atlas-proto` — the wire contract shared by the CLI, the
  service and the Worker (including `query-parameters-v1.json`, the
  defaults contract).
- `crates/atlas-search` — the binaries: `atlas-search` (search,
  get-chunk, serve, init, fetch-model, traces, feedback), `atlas-stage`,
  `atlas-classify`, `atlas-eval`, `atlas-sweep`,
  `atlas-github-comments`.
- `worker/` — the Cloudflare Worker: the hosted REST API and the
  Atlas MCP server, with a per-query Durable Object cache and a D1
  hot-cache tier.
- `atlas_search.py` — the Python reference implementation the Rust
  was certified against; kept for parity.
- `eval/`, `schema/`, `tools/`, `fixtures/`, `test-vectors/` — the
  evaluation programme, the staging/gate schemas, and the operator
  tools.
- `docs/` — package-local design notes. The book supersedes these
  for reference purposes; they remain as design records.

## Build and run

This package is a self-contained workspace on the pkgs/id pattern,
with its own flake:

```sh
nix build path:./pkgs/atlas-search     # the packaged binaries
nix develop path:./pkgs/atlas-search   # the pinned toolchain shell
```

ONNX Runtime is vendored in-tree (`onnxruntime/`) and loaded
dynamically; it is never downloaded at build or run time.

The corpus itself lives outside this repo, on the laptop
(`~/atlas/`), and is never committed. Secrets are resolved by name
through the fleet secrets broker; no credential is stored in this
tree.

## Operator and contributor contract (DOC-01)

The contract with the humans and agents who operate and change
Atlas, in the shape of the fleet package's DOC-01 (#10728):

- **The surface is the code.** The binaries' parsers, the service's
  route table, the Worker's `mcp.ts` tool schemas and
  `query-parameters-v1.json` are the authority on what Atlas can do.
  Documentation — this README, `docs/`, and the book — must not
  present a designed spelling as shipped. Where a doc and the code
  disagree, the code wins and the disagreement is filed as an issue
  in the same pass it is found.
- **The book is part of the same change.** Any change to a command,
  a flag, a default, a config key, an MCP tool, a REST route, a
  schema, the embedding model, or an operational procedure updates
  the matching chapter of [`book/`](book/) in the same change. The
  rule is stated for readers in the book's own "Keeping this book
  current" chapter. `create-missing = false` in `book.toml` makes a
  missing chapter a build failure, never a stub, and the
  `atlas-docs.yml` workflow builds `.#atlas-docs` on `main` so a
  broken chapter fails by name.
- **Sensitivity is a boundary, not a convention.** Corpus tiers
  (public / internal / secret) and client ceilings (shareable <
  personal < local-only < secret-bearing) are enforced server-side,
  in the retrieval layer. No change may move enforcement into client
  goodwill, send secret-tier text to a hosted embedder or reranker,
  or reproduce secret-tier content in docs, issues or logs.
- **Model changes are migrations.** A new embedding model means a
  new model-tagged vector space, a shadow index, and a golden-set
  evaluation; cutover happens on the eval result, not on completion
  (see the book's "Change the embedding model" chapter).
