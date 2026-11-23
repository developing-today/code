# Installation and provisioning

Installing Atlas means three separate things, and this chapter keeps them
separate: building the binaries (code, delivered by Nix), provisioning the
embedding model and the ONNX Runtime (data and a shared library the binaries
load at run time), and provisioning the credentials and service state the
binaries read (tokens resolved by name, a service config written once by an
`init` verb). The Worker has a fourth, separate deploy path, covered at the
end. If you only want to search, the first two sections and the model section
are enough; the service sections matter when a seat will serve or use the
service route.

Source: https://github.com/developing-today/code/blob/main/pkgs/atlas-search/default.nix and https://github.com/developing-today/code/blob/main/pkgs/atlas-search/flake.nix

## The two Nix routes

Atlas lives in the repository at `pkgs/atlas-search/`, a self-contained
package with its own Cargo workspace, its own `flake.nix`, and a
`rust-toolchain.toml` pinning the Rust toolchain (the file itself reads
channel 1.99.1; the `flake.nix` comment and CI agree — see
[Building and testing](../development/building.md)).

**Route 1 — the package's own sub-flake.** From the repository root:

```sh
nix build path:./pkgs/atlas-search
```

The sub-flake exposes one package, `atlas-search` (also its `default`), and
a check that is the package itself. When a crane library is supplied — the
sub-flake supplies one — dependency compilation is split into its own
derivation keyed only on the manifests, the lockfile, the crate sources, and
`schema/`, so a workspace-source change rebuilds only the workspace crates.
Callers that do not pass a crane library get the same package through the
monolithic `rustPlatform.buildRustPackage` path: same sources, same checks,
no split.

**Route 2 — home-manager, via `callPackage`.** On the laptop, the home
configuration (`home/common/default.nix`) builds the same `default.nix` the
pkgs/id way:

```nix
atlasSearch = pkgs.callPackage ../../pkgs/atlas-search {
  rustPlatform = pkgs.makeRustPlatform {
    cargo = atlasSearchToolchain;
    rustc = atlasSearchToolchain;
  };
};
```

with `atlasSearchToolchain` read from `pkgs/atlas-search/rust-toolchain.toml`
through the id rust overlay. The resulting package is added to
`home.packages`, so the Rust binary is the resolved `atlas-search` command on
the laptop. The Python reference implementation is kept alongside it under
the name `atlas-search-py`; it is not the resolved command. The same home
configuration also declares the `atlas-search.service` user unit — see
[Quickstart](quickstart.md) for using the service and the operations part of
this book for running it.

There is no non-Nix install route for the Rust binaries. The Python
reference (`atlas_search.py`, at the package root) runs with the Python
standard library alone for full-text search and `get-chunk`, but it is a
parity and evaluation instrument, not the install path.

## What gets installed

One package installs exactly six binaries, all version `0.1.0` where a
version surface exists:

| Binary | What it is |
| --- | --- |
| `atlas-search` | The access layer: `search`, `get-chunk`, `serve`, `init`, `fetch-model`, `traces`, and `feedback`. The package's `mainProgram`. `--version` prints `atlas-search 0.1.0`. |
| `atlas-stage` | Staging units and the staging registry: `provision`, `lookup`, `ingest`, `items`, `promote`, `units`. It has no `--version` flag — passing one is a usage error, exit 2 (source behaviour; the other hand-rolled tools below likewise have no version surface). |
| `atlas-classify` | The classification operating procedure: `run`, `schema-review`, `schema-apply` against a corpus database. |
| `atlas-eval` | The offline ranking-evaluation runner: two positional arguments, an eval document and a `dev` or `test` split. |
| `atlas-sweep` | The ingestion sweep runner: pause/resume a source class, run one sweep request, manage dead letters, report. It creates no schedule; cadence is an operator decision. |
| `atlas-github-comments` | The GitHub-comments extraction feed: incremental, watermark-based extraction of the estate's issue and review comments into corpus records. |

The package build runs the test suite in its check phase and an install
check afterwards: `fixtures/smoke_check.py` runs one full-text-only query
and one embedded query against a fixture database and byte-compares the
results with the recorded Python envelopes, and probes that the packaged
ONNX Runtime loads with no environment override set (below).

## ONNX Runtime provisioning: vendored, loaded dynamically, never downloaded

Vector search needs ONNX Runtime (the shared library `libonnxruntime.so`)
to run the embedding model. Atlas's rule, learned during packaging and now
mechanised, is absolute: **the runtime is never downloaded at build time or
at run time.** The `ort` crate is built with its `load-dynamic` feature and
without its default prebuilt-binary fetch, and the library comes from the
Nix closure:

- The package vendors its own ONNX Runtime in-tree, at
  `pkgs/atlas-search/onnxruntime/`, pinned to the upstream release the
  corpus and the Gate A parity fixtures were certified under — version
  **1.31.1**. This is deliberately not the ambient nixpkgs version (which
  is older): the service aligns up to the corpus, never the corpus down to
  the library.
- `default.nix` compiles a default library path into the binary as the
  compile-time value `ATLAS_SEARCH_ORT_LIB_DEFAULT`, pointing at the
  vendored package's `libonnxruntime.so` in the Nix store. Because that
  path is a store reference, the runtime — and with it `libstdc++.so.6`
  and `libz.so.1`, which the library needs — is part of the package's
  runtime closure. You can verify that with
  `nix-store -qR &lt;out&gt;` on the build result.

At run time the binary resolves the library in this order:

1. The environment variable `ATLAS_ORT_LIB`, when set to an existing file.
2. A staged copy at `~/.local/state/atlas-search/ort/libonnxruntime.so`,
   when present.
3. The compiled-in `ATLAS_SEARCH_ORT_LIB_DEFAULT` (packaged builds only;
   development builds never set it).

One surface has an extra fallback: `atlas-stage promote` additionally
probes the Atlas virtualenv's
`~/atlas/v3-venv/lib/python3.14/site-packages/onnxruntime/capi/libonnxruntime.so.1.31.1`
before giving up. A missing runtime is a loud error, not a silent
full-text-only degradation — except that full-text-only search and
`get-chunk` never load the embedder at all, and so never fail on a missing
runtime or a missing model. The version matters, not just the presence:
the parity gate is version-matched, and a different runtime version
produces small numeric deltas (about 1e-7 per component against 1.27.1)
that fail the gate as not applicable rather than as a verdict.

## Model provisioning: the state dir and `fetch-model`

The embedding model is data, not code, and it is not in the Nix store. It
lives in the Atlas state directory:

```text
~/.local/state/atlas-search/model/<rev>/
```

where `&lt;rev&gt;` is the pinned HuggingFace revision
`95c2741480856aa9666782eb4afe11959938017f` of the model
`Snowflake/snowflake-arctic-embed-m-v2.0`. Provision it with:

```sh
atlas-search fetch-model
```

`fetch-model` takes no options. It downloads the pinned revision's
`tokenizer.json` and `onnx/model.onnx`, verifies both against the SHA-256
hashes recorded in the source, and installs the snapshot atomically. If the
snapshot already verifies, the verb is a no-op; a hash mismatch refuses the
install and exits 2. At run time, model resolution is: the environment
variable `ATLAS_MODEL_DIR` when it names an existing directory, else the
state-dir snapshot above, else the HuggingFace cache snapshot under
`~/.cache/huggingface/hub/` for the same pinned revision, else a
model-not-found error. A missing or mismatched model fails vector and
hybrid search loudly; full-text search and `get-chunk` keep working with
no model present, because neither loads the embedder.

## Service provisioning: `init`, the client table, and client files

The laptop service is provisioned once:

```sh
atlas-search init
```

This generates the service config at
`~/.local/state/atlas-search/service.json` (override with `--config` or the
environment variable `ATLAS_SEARCH_SERVICE_CONFIG`), with file mode 0600.
The document carries the listen address `127.0.0.1:18760`, the default
vector table, and the version-1 client table — named clients, each with a
freshly generated token and a ceiling cap: `drew-cli` (cap
`secret-bearing`), `openchamber` (cap `personal`), `fork-server` (cap
`personal`), `sandbox-scripts` (cap `personal`). The verb prints the config
path and each client's cap; it never prints a token value. Running `init`
when a config already exists is an error, exit 2; `--force` replaces the
config and rotates every client token. Raising a client's cap afterwards
is a local edit of this file by Drew — no verb does it.

A seat that will use the service route needs the matching client side: its
token is presented from the environment variable `ATLAS_SEARCH_TOKEN` when
set, else from the JSON file at `~/.config/atlas-search/client.json` (a
JSON object with one `token` string field, expected mode 0600; the path is
overridable with `ATLAS_SEARCH_CLIENT_CONFIG`). A seat with neither cannot
use the service route and falls back to standalone search, loudly. The
service itself is started by the home-manager unit as `fleet serve search`
(the same service, rehomed into the fleet binary); `atlas-search serve` is
the kept rollback, selected by one flag in the home configuration.

## The Turso token chain (names only)

The default search backend is the hosted Turso corpus, reached with a
**read-only** token. The service holds no write credential for any Turso
database, and every statement the backends build is a `SELECT`. The token
is resolved, in order, from:

1. The environment variable `ATLAS_TURSO_TOKEN`.
2. Bitwarden Secrets Manager, by name — the names tried are
   `turso.atlas-ro-token`, `turso.atlas-read-only-token`, and `atlas-ro`.
   (Reaching Bitwarden itself uses the `BWS_ACCESS_TOKEN` environment
   variable, or the fleet's `bws` environment helper when that is unset.)
3. A cached mint at `~/.cache/atlas-search/turso-token.json`, when it names
   the `atlas` database and is less than 20 hours old (file mode 0600).
4. A fresh read-only mint through the `turso` CLI:
   `turso db tokens create atlas --read-only -e 7d`, cached at mode 0600.

If no source resolves, the search fails with a fatal error naming the
sources it tried. Shard fan-out uses separate per-tier read-only tokens,
again by environment name only: `ATLAS_TURSO_TOKEN_PUBLIC`,
`ATLAS_TURSO_TOKEN_INTERNAL`, and `ATLAS_TURSO_TOKEN_SECRET`, plus
`TURSO_API_TOKEN` for shard discovery through the Turso platform API. A
tier with no token is simply not connected — and discovery is opened only
up to the caller's own ceiling, so a default `personal` caller never needs
the secret-tier variable. No token value appears in this book, in the
service config's printed output, or in any log the tools write.

## Worker deploy prerequisites

The Cloudflare Worker (`pkgs/atlas-search/worker/`) is not a Nix install;
it deploys separately, and only its prerequisites are listed here — the
deploy procedure itself is in the operations part of this book.

- **Tooling.** Local development and tests use `wrangler` (with
  `wrangler.toml` as the local configuration) and the Worker's `npm test`
  suite. Deploys go through the sanctioned `cf` binary's versions flow,
  never a silent overwrite; for deploys, `cloudflare.config.ts` is the
  source of truth and must be kept in step with `wrangler.toml`.
- **Bindings (names only).** A Durable Object binding `QUERY_CACHE` (class
  `QueryCache`, SQLite-backed), a D1 binding `HOT_CACHE` (database name
  `atlas-hot-cache`; the committed database id is a local placeholder,
  replaced with the real id when the database is created at deploy time),
  and the plain variables `SERVICE_BASE` (the tunnel hostname the Worker
  calls the laptop service on) and `HOT_CACHE_ENABLED` (committed value
  `"false"`).
- **Secrets (names only).** `GATE_TOKEN` (the inbound bearer gate, covering
  both the REST routes and `/mcp`), `SERVICE_TOKEN` (the service's
  `cf-worker` client token), and `ACCESS_CLIENT_ID` /
  `ACCESS_CLIENT_SECRET` (the Cloudflare Access service token for the
  tunnel hostname). At deploy time these are assembled from fleet secrets —
  names `atlas.search-worker-gate-token`, `atlas.search-cf-worker-token`,
  `cloudflare.access-atlas-search-client-id`, and
  `cloudflare.access-atlas-search-client-secret` — into a throwaway secrets
  file that is deleted after the upload; the `cf` CLI's own API token is
  the fleet secret named `cloudflare.api-token-account-wide`. No value
  lands in the repository.
- **The service behind it.** The Worker holds no Turso credentials and
  keeps no shard list. It needs the laptop service reachable through its
  tunnel hostname with the Access token above; while the service is
  unreachable the Worker degrades to 502 with nothing cached.

## Verifying the install

```sh
atlas-search --version
atlas-search search "weekend market rain plan" --mode fts --k 3 --text
```

The first prints `atlas-search 0.1.0`. The second runs a full-text-only
search — no model, no ONNX Runtime needed — and prints human-readable
results. Add a hybrid search (drop `--mode fts`) once `fetch-model` has
run, and you have verified the model and the runtime as well. That is the
whole first-run path; [Quickstart](quickstart.md) takes the same search
through all three surfaces.
