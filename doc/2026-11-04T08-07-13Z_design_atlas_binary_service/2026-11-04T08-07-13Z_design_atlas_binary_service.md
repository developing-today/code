# Atlas as a Rust binary and a multi-client search service

Design record, 2026-11-04. The design for Drew's 2026-11-04 ruling
"b and c" on dezren39/hatch-personal#21: Atlas search ships as
**(b)** a Rust binary vendored from this repo, and **(c)** a
multi-client search service built on that binary. This note is the
design only — no implementation. Implementation is tracked by the
epic and step issues filed alongside it (linked from the epic).

Facts are from the Python implementation as deployed
(`atlas_search.py`, 1,177 lines, source of truth in
dezren39/hatch-personal `workspace/turso-atlas/scripts/`), the
2026-10-31 v3 bake-off record, the 2026-11-03 Snowflake production
swap and its verification, the fork's native-embeddings spike
(developing-today/opencode#1554 and its step-2 backend,
opencode#1559), and the fleet's existing service patterns (the
OpenCode server, the pm-stack projector). Measurements quoted are
the ones those records carry; where a number is a proposal rather
than a measurement, the text says so.

## The current shape

Atlas is a read-only hybrid search over Drew's canonical
conversation corpus: 107,639 chunks in a Turso/libsql database
(`atlas-dezren39.aws-us-east-1.turso.io`), every chunk embedded
with Snowflake `snowflake-arctic-embed-m-v2.0` at full 768
dimensions (`vec_snowflake_m_v2_768_f32`, typed `F32_BLOB(768)`
with a `libsql_vector_idx` index), plus a dormant bge-small set
kept as fallback. Production search, decided 2026-10-31 and live
since the 2026-11-03 swap, is one-stage hybrid: an FTS5 (BM25)
branch and a vector (cosine) branch, fused with reciprocal rank
fusion at **k=10, pool 100 per branch** (best measured row on the
golden 311: 189 hits@10 / MRR 0.3894).

Three properties of the Python define what any successor must
preserve exactly:

1. **Sensitivity is a caller ceiling, enforced before ranking.**
   `SENS_ORDER = [shareable, personal, local-only, secret-bearing]`;
   the default ceiling is `personal`. Classification filters are
   SQL pre-filters applied to both branches _before_ ranking, so a
   restricted chunk is never ranked, scored, or snippeted for a
   caller whose ceiling excludes it. Reaching `secret-bearing`
   requires spelling the ceiling out explicitly.
2. **Query embedding is part of the measured system.** Queries go
   through sentence-transformers with `prompt_name="query"` (the
   model's query prompt); documents were embedded prompt-free.
   The vectors, the tokenizer, and the prompt handling are one
   contract — the 2026-11-03 delta-embed gate proved the pipeline
   reproduces stored vectors bit-for-bit (cosine 1.00000000 on the
   gate chunks) when the contract is held.
3. **Two backends, one semantics.** A Turso backend (libSQL HTTPS
   pipeline, read-only token resolved from Bitwarden by name or
   minted read-only via the `turso` CLI) and a local backend
   (SQLite publish files; side stores joined by **content hash**,
   never chunk_id — the id spaces differ between stores). The
   eval harness (eval3.py, golden-queries-v3.json) runs against
   the local form; production callers use Turso.

Measured costs today: a cold CLI run is ~35 s wall, dominated by
sentence-transformers model load; the filtered vector branch
costs ~6 s (pool-100, with the sensitivity join), ~15 s
server-side for session-filtered queries (the publish schema has
no secondary indexes; the chunk→message→session join chain runs
over remote storage); FTS and unfiltered queries are ~0.5–4 s.

Operational facts that motivate this design, all observed
2026-11-03: the Python wrapper/venv broke **twice in one evening**
on `LD_LIBRARY_PATH` (the desktop session's pipewire path starves
numpy of libstdc++/libz; repaired with probe hacks in
hatch-personal PRs #18/#19); the fork's spike proved an in-process
onnxruntime embedder with the model's own SentencePiece Unigram
tokenizer reproduces the Python Snowflake vectors **bit-identically**
(delta 0.0, token ids equal on every probe incl. mixed Japanese);
and the production atlas writer is currently pathological (a
one-row insert took 1,345.5 s server-side during the swap —
dezren39/hatch-personal#20), which constrains anything that would
write near that database.

The vector backend is **not** decided by this note. The Python
already carries three `--vector-backend` modes — `scan` (exact
`vector_distance_cos` over the pre-filtered allowed set; the
production default), `index-fixed` (one `vector_top_k` lookup,
filters applied after), `index-fill` (escalating index depth
until the post-filter pool fills, cap 3200) — and an A/B on the
golden 311 against production data is running now to settle
which deserves to be the default. The design treats the backend
as a pluggable strategy behind one trait; the A/B winner is the
implemented default, the losers stay behind the flag for
re-measurement.

## Decision summary

| #   | Question                      | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| --- | ----------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | One artifact, two modes?      | **One Rust binary**, `atlas-search`, with one-shot CLI subcommands and a `serve` subcommand. CLI is standalone-capable by default and can route through the service (`--via-service`); it is never a thin-client-only tool.                                                                                                                                                                                                                              |
| 2   | Service protocol / temp store | Job protocol over HTTP+JSON: submit → workers run branches concurrently → fused ranking lands in an **in-memory temp store** (TTL, per-job capability token) → clients page an immutable ranking. Fetch-to-fill escalation lives _inside_ the job's vector branch, never in client paging.                                                                                                                                                               |
| 3   | Clients, exposure, auth       | Named clients only (Drew's CLI, OpenChamber/fork sessions on amd, sandbox scripts via tailnet). Bind 127.0.0.1 + `tailscale serve` on the MagicDNS name, same pattern as the OpenCode server. Per-client bearer tokens in a 0600 state file (service-prod.json pattern), **each token carrying a maximum sensitivity ceiling the server clamps to**. Aster is not a v1 client.                                                                           |
| 4   | Layout / packaging / deploy   | Self-contained `pkgs/atlas-search/` (pkgs/id pattern): workspace of four crates, `default.nix` via `rustPlatform.buildRustPackage`, ONNX Runtime loaded dynamically from the nix closure (which must carry libstdc++ + zlib). Model artifact provisioned to the state dir at deploy time, pinned revision + sha256, **not** in the nix store. Deploy as a systemd **user service** declared in home-manager; state under `~/.local/state/atlas-search/`. |
| 5   | Parity + migration            | Three gates: embedder bit-identity on a fixed probe set; golden-311 tie vs the Python production path on the production corpus (tolerances stated in §5); the A/B-winning backend as the implemented default. Migration is additive-first in four phases; the Python remains installed as the fallback and stays the eval reference permanently.                                                                                                         |
| 6   | Non-goals                     | No writes to the production atlas DB (the service holds only a read-only token), no corpus re-embedding, no publish-schema changes, no client-side ceiling widening, no public exposure, no Aster client in v1.                                                                                                                                                                                                                                          |

## 1. One artifact, two modes

**Recommendation: one binary, two modes.** `atlas-search` ships
the one-shot CLI surface (`search`, `get-chunk`, mirroring the
Python's commands and JSON envelope) and a `serve` subcommand
that runs the multi-client service, in a single derivation, a
single release, a single version string. This follows the two
precedents that already work in the fleet: pkgs/id ships one
binary whose whole graph is optimized together, and the fork's
OpenCode server is one binary run in serve mode.

The alternative — a standalone CLI binary and a separate server
binary — was rejected for one concrete reason: the search
semantics are the product. Filters, ceiling enforcement, fusion,
and the envelope format must be identical in both modes, or the
golden-parity gate (§5) has to be passed twice against two
implementations that will drift. Two _crates_ (a core library
both modes consume) give the separation without the drift; two
_binaries_ only add a second release artifact to keep in lockstep.
The cost of one binary is real but small: the serve code
(HTTP stack, worker pool) is compiled into the CLI too. It is
dead weight of a few MB in a binary whose runtime resident set is
dominated by the ~768-dim embedder session anyway.

### What serve mode holds, and what it costs

Serve mode is a long-lived process whose value is exactly the
state the one-shot CLI pays to rebuild on every call:

- **The resident embedder.** One loaded ONNX session + tokenizer
  for Snowflake, initialized lazily on first query, shared by all
  workers. The fork's native-embeddings measurements put a
  resident Snowflake session in the few-hundred-MB RSS range with
  a ~1 s lazy load — paid once per service lifetime instead of
  once per query. This is the single largest latency win the
  service offers (the ~35 s Python cold start is mostly this).
- **A connection pool to Turso.** HTTP keep-alive connections to
  the libsql endpoint, so branch queries skip connection setup
  and share the read-only token resolution (resolved once at
  startup from the same Bitwarden names / CLI mint path the
  Python uses, refreshed by the service, never per query).
- **The temp store and job registry** (§2), and **the worker
  pool** with its concurrency semaphores.

### Standalone-capable vs thin client

The CLI keeps both paths, deliberately:

- **Standalone (default).** The CLI embeds the query itself and
  talks to Turso (or a local publish file) directly — byte-for-
  byte the Python's operating mode. Every existing caller
  (scripts, lanes, the eval harness's production checks) keeps
  working with no daemon up. A laptop with no service running
  still has full Atlas.
- **Via service (`--via-service`, or `ATLAS_SEARCH_VIA=service`).**
  The CLI submits the query to the local service and pages the
  result. It gains: no model load (warm embedder), shared Turso
  pool, the service's deeper caches. It loses: independence — a
  down service must fall back loudly to standalone, never fail
  silently or hang; the flag's contract is _prefer service, fall
  back with a note in the envelope's `notes`_, matching how the
  Python reports backend fallbacks today.

Making the CLI thin-client-only was rejected: it would convert
every script that calls `atlas-search` into a dependent of one
daemon's uptime, and it would make the golden-parity gate
meaningless for the standalone environments (sandbox copies,
one-off laptop use) where the tool is also used. The daemon is
an accelerator and a sharing mechanism, not a new single point
of failure.

## 2. Service protocol: jobs, a temp store, pull-until-satisfied

Drew's sketch — clients submit queries, workers execute branches
concurrently, results land in a temp store, clients pull until
their pools are satisfied — is adopted with one sharpening:
**what the client pages is the fused ranking, and the ranking is
immutable once computed.** Fetch-to-fill depth escalation is a
property of the vector _branch's_ execution inside the job
(exactly the `index-fill` semantics the Python's A/B modes
already define), not of client paging. This keeps RRF correct by
construction: fusion runs once, over complete branch pools, and
paging slices a finished list — a client can never observe a
ranking whose fusion math changes between page 1 and page 2
because a branch was still filling.

### Endpoints (HTTP + JSON, versioned path)

| Endpoint                                          | Semantics                                                                                                                                                                                                                                                                                          |
| ------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `POST /v1/search`                                 | One-shot convenience: submit + wait + first page in a single call, for simple clients. Same job machinery underneath.                                                                                                                                                                              |
| `POST /v1/search/jobs`                            | Submit a job `{query, filters, ceiling, pool, vector_backend}` → `202 {job_id, job_token}`. The server clamps `ceiling` to the caller's token cap (§3) and records the _effective_ ceiling on the job.                                                                                             |
| `GET /v1/search/jobs/{id}`                        | Job status: `queued / running / done / failed / expired`, branch states, effective ceiling, pool actually used, notes.                                                                                                                                                                             |
| `GET /v1/search/jobs/{id}/results?cursor=&limit=` | Pages of the fused ranking, in the Python's envelope shape (results carry `rank_sources` and fused `score`; page carries `next_cursor` or `null`). Cursor is opaque; it encodes an offset into the job's immutable ranking. Only the job's `job_token` (or the owning client's token) may page it. |
| `DELETE /v1/search/jobs/{id}`                     | Early release of the job's stored results.                                                                                                                                                                                                                                                         |
| `GET /v1/search/jobs/{id}/chunk/{chunk_id}`       | `get-chunk` equivalent (chunk + neighbors) under the job's effective ceiling — a chunk outside the ceiling returns the same shape as a missing chunk, never a leak of its existence tier.                                                                                                          |
| `GET /health`                                     | Liveness + embedder state (cold/warming/ready), queue depth. No auth, no data.                                                                                                                                                                                                                     |

A job's requested `pool` is the per-branch pool depth (default
100, the production value). A client wanting a deeper fused list
asks for a larger pool at submit time; it does not page its way
to depth — paging beyond the computed ranking returns the end of
the list, not a deeper search. A follow-up job at a larger pool
is cheap to submit and is the honest mechanism, because a deeper
pool is a _different search_ (the pool sweep moved MRR
materially), not a longer page of the same one.

### The temp store: in-memory, TTL'd, capability-gated

Three candidates were considered:

- **A scratch Turso/libsql database** — rejected on evidence,
  not taste: the production atlas writer is currently stalled at
  ~22 min per write (hatch-personal#20); a temp store whose
  writes queue behind _any_ remote writer inherits a failure
  mode that has nothing to do with search, and every branch
  write pays a network round trip for data with a 10-minute
  useful life.
- **A local SQLite file in the service's state dir** — workable,
  and the documented fallback if job _history_ ever becomes a
  requirement (debugging, audit). Rejected as the default
  because persistence is a liability here (below), and a single
  service process has no need of a second writer-serialized
  store for data measured in kilobytes per job: a full job is
  two branch pools (≤ pool rows each at the default, ≤ 3,200
  rows at the index-fill cap) of `(chunk_id, rank, score)` plus
  a fused ranking of ≤ pool result entries with snippets.
- **In-memory (chosen).** A sharded map keyed by job_id, holding
  branch pools as they land and the fused ranking when fusion
  completes, behind a TTL sweeper. Results are _reproducible_
  (a lost job is a resubmit, not a data loss), _small_ (above),
  and _perishable by design_:

What survives a service restart: **nothing in the temp store,
and that is the requirement, not a gap.** A job's results contain
corpus snippets at the _job's effective ceiling_ — the most
sensitive data the service touches. Leaving them on disk would
extend the exposure of ceiling-gated content beyond the query
that authorized it, into a file any later process (or backup)
can read without a token. In-memory + TTL (proposed: 10 minutes
from completion, 60 minutes hard cap from submit) + a per-job
capability token means stored results exist only while the
service that enforced the ceiling is alive to keep enforcing
it. A restart fails every in-flight job `expired`; clients
resubmit. Search is idempotent and read-only, so this is safe.

### Job lifecycle and backpressure

`submit → queued → running (branches concurrent) → done →
paged → expired/released`. Branch workers are a bounded pool
(tokio tasks behind semaphores). The scarce resource is not CPU
on amd — it is **Turso-side query cost**: a filtered vector scan
is ~6–15 s of server-side work, so unbounded concurrent jobs
would queue seconds of remote work per client whim and degrade
everyone, including the Python fallback path sharing the same
database. Proposed limits (config, tuned after first real load,
not load-bearing for the design):

- Global concurrent branch executions: **4** (at most 4 remote
  branch queries in flight; FTS branches are cheap but share the
  semaphore accounting so a flood of any kind is bounded).
- Per-client concurrent jobs: **2**; per-client queued jobs: 8.
- Pool depth: default 100, max **400** at submit; index-fill
  escalation inside the branch caps at **3,200** (the Python
  A/B's cap) regardless of requested pool.
- Saturation behavior: submit beyond limits returns **429 with
  `Retry-After`** and the queue depth in the body — jobs are
  cheap to reject and expensive to half-run; a queued job that
  waits longer than its TTL-to-start (proposed 120 s) fails
  `expired` rather than executing stale intent.

Clients pulling deep or often are not a backpressure concern
once fusion is done — paging is map lookups. The pull pattern
that _is_ bounded is job creation, which is where the cost is.

## 3. Clients, exposure, auth

### The named client set (v1)

| Client                        | Where it runs | Transport          | Ceiling cap (proposed)                                                                                                    |
| ----------------------------- | ------------- | ------------------ | ------------------------------------------------------------------------------------------------------------------------- |
| Drew's `atlas-search` CLI     | laptop (amd)  | loopback           | `secret-bearing` — Drew's own tool keeps the Python's full range, still requiring the ceiling to be spelled out per query |
| OpenChamber sessions          | laptop (amd)  | loopback           | `personal` (the current default)                                                                                          |
| Fork-server (OpenCode) agents | laptop (amd)  | loopback           | `personal`                                                                                                                |
| Sandbox scripts / Muse lanes  | sandbox VM    | tailnet (MagicDNS) | `personal`                                                                                                                |

**Aster (Dot) is not a v1 client.** It is an external-facing
assistant with its own usage rules and logging; giving it a token
is a separate decision for Drew, taken with its data-sharing
implications in view, not a default consequence of the service
existing. The protocol does not preclude adding it later — it is
one more token row — but nothing in v1 issues it one.

Anything not in the table gets no token. In particular there is
no anonymous mode and no "trusted network" mode: tailnet
membership gets a client _reachability_, never authorization.

### Exposure

Bind `127.0.0.1` on amd, fronted by `tailscale serve` under the
machine's MagicDNS name — the OpenCode server's exact pattern
(127.0.0.1:13420 locally, `https://amd-6.raptor-pumpkinseed.ts.net`
over the tailnet). Proposed port **18760** (the fleet's 187xx
door range); the packaging step must verify it against the
fleet port inventory (`check-proxy-port-fallbacks.sh` and
`home/common/default.nix`) and take the next free 187xx if it is
taken. Never a public listener, never a Cloudflare tunnel: the
corpus is Drew's conversation history, and the tailnet is the
perimeter.

### Auth: per-client tokens with ceiling caps

The OpenCode server's pattern — a password in a 0600 JSON file
under `~/.local/state/<service>/`, read at startup — is adopted
and extended in the one direction Atlas's privacy model demands:

- `~/.local/state/atlas-search/service.json` (0600) holds the
  service config: listen address, and a **client table**:
  `{ name, token (generated, high-entropy), ceiling_cap }`.
  Tokens are generated by the service's `init` verb, stored only
  in this file and in each client's own config (the CLI reads
  its token from `~/.config/atlas-search/client.json`, also
  0600; sandbox copies from their local equivalent). Token
  _values_ never appear in the repo, in logs, or in this design;
  they are referenced by client name everywhere else.
- Every request carries `Authorization: Bearer <token>` —
  including on loopback (the OpenCode server password-auths
  loopback too; precedent kept).
- **The clamp is the security property.** A job's requested
  ceiling is clamped server-side: `effective = min(requested,
token.ceiling_cap)` under `SENS_ORDER`, and the effective
  ceiling is what the branch pre-filters are built from, what is
  recorded on the job, and what the envelope reports. A client
  cannot widen its own ceiling by asking, by paging another
  job (job_tokens are per-job capabilities), or by the
  `get-chunk` endpoint (ceiling-checked, §2). Raising a cap is
  an edit to `service.json` on amd by Drew — a deliberate,
  local, auditable act, mirroring how reaching `secret-bearing`
  in the Python requires spelling it out.
- Rotation: replace the token in `service.json`, restart (or
  SIGHUP-reload; the packaging step picks one and documents it),
  update the one client config. No other state mentions tokens.

## 4. Layout in developing-today/code, packaging, deploy

Drew's rule is that all services and binaries live in this repo.
Atlas's _tooling_ historically lives in hatch-personal
(`workspace/turso-atlas`) because that is where the corpus
pipeline grew; the binary and service are new code and go here,
in a self-contained package following the pkgs/id pattern (its
own Cargo workspace, `flake.nix`, `default.nix`,
`rust-toolchain.toml`):

```
pkgs/atlas-search/
  Cargo.toml            # workspace
  Cargo.lock
  rust-toolchain.toml   # pinned toolchain, pkgs/id pattern
  flake.nix
  default.nix           # rustPlatform.buildRustPackage (see below)
  crates/
    atlas-core/         # search semantics, no process I/O policy
    atlas-embed/        # Snowflake embedder (ort + Unigram tokenizer)
    atlas-search/       # the binary: CLI subcommands + serve subcommand
    atlas-proto/        # job protocol types shared by serve + via-service client
  fixtures/
    embed-probes.json   # fixed probe texts + Python-recorded ids/vectors (§5a)
    golden-311.json     # pointer/copy policy for golden-queries-v3 (see §5b)
  doc/                  # package-local notes (protocol reference lives here once built)
```

**Crate boundaries** (the split follows the seams the Python
already has — `Filters` / backends / `embed_query` / `run_search`
— it does not invent new ones):

- `atlas-core` — filters→SQL construction, sensitivity ceiling
  logic, branch query builders, RRF fusion, the result envelope
  types, and a `SearchBackend` trait with two implementations:
  Turso (libsql HTTPS pipeline) and Local (SQLite files, side
  stores joined by content hash). Local is in scope for **CLI
  mode** (the eval harness and offline publish copies are real
  uses); serve mode v1 exposes the Turso backend only. Signature
  sketch of the seam that matters:

  ```rust
  trait SearchBackend {
      fn branch_pools(&self, q: &BranchQuery, filter: &FilterSet,
                      ceiling: Sensitivity)
          -> Result<BranchPools>;   // FTS + vector pools, pre-filtered
      fn chunk_details(&self, ids: &[ChunkId], ceiling: Sensitivity)
          -> Result<Vec<ChunkDetail>>;
  }
  enum VectorBackend { Scan, IndexFixed, IndexFill }  // §5c picks the default
  ```

- `atlas-embed` — the Snowflake embedder and nothing else:
  dynamic-loaded ONNX Runtime session, the model's own
  `tokenizer.json` (SentencePiece Unigram, Metaspace) loaded via
  the `tokenizers` crate, query-prompt application replicating
  ST's `prompt_name="query"` behavior exactly, CLS pooling,
  full-768 output, lazy init behind a once-cell. Its entire
  reason to exist as a separate crate is the §5a parity gate,
  which tests it without the rest of the system.
- `atlas-proto` — the §2 protocol's request/response types, so
  the serve implementation and the CLI's via-service client
  cannot drift apart in serde definitions.
- `atlas-search` (bin) — argument parsing, the one-shot commands
  (core + embed + chosen backend, standalone), the via-service
  client (proto + HTTP), and `serve` (HTTP server, worker pool,
  in-memory temp store, token/ceiling enforcement — all policy
  lives here and in core, none in proto).

### The nix derivation and the ONNX loader lesson

`default.nix` uses `rustPlatform.buildRustPackage` with
`cargoLock.lockFile` (outputHashes as needed), per the pkgs/id
precedent. Two hard requirements come from the fork's
native-embeddings work (opencode#1554/#1559) and are stated here
so the packaging step cannot rediscover them:

1. **ONNX Runtime is loaded dynamically from the closure, never
   downloaded at build or run time.** The `ort` crate's default
   behavior fetches a prebuilt onnxruntime during the build —
   forbidden in the nix sandbox and a reproducibility hole
   besides. The derivation builds with dynamic loading against
   nixpkgs' `onnxruntime` package, and the binary resolves it
   from its runtime closure.
2. **The closure must carry `libstdc++.so.6` and `libz.so.1`.**
   The fork verified its packaged server fails loudly without
   them and passes with the gcc lib + zlib store paths in the
   loader path. For a nix-native binary this is ordinary
   `buildInputs`/`runtimeDependencies` hygiene (gcc's lib gets
   into the closure via the onnxruntime package's own deps, but
   the packaging step must _verify_ with `ldd`/closure inspection
   and a smoke run — not assume), plus a post-install check that
   starts the binary and runs one FTS-only and one embedded
   query against a fixture DB.

### Model artifact provisioning

The Snowflake model (ONNX graph + `tokenizer.json` + configs,
pinned to one HuggingFace revision) is **data, not code**, and
is provisioned like the corpus itself — outside the nix store:

- A one-shot `atlas-search fetch-model` verb (also run by the
  deploy step) downloads the pinned revision into
  `~/.local/state/atlas-search/model/<rev>/`, verifies every
  file against sha256 hashes recorded **in the repo** (the
  revision + hashes are code-reviewed constants; the bytes are
  not in git and not in the store), and atomically renames the
  completed directory into place. Re-running is a no-op when the
  pinned set verifies.
- The binary takes the model dir from config/env
  (`ATLAS_MODEL_DIR`), fails **loudly** at embedder init if it is
  missing or hash-mismatched, and — matching the Python's
  degradation shape — FTS-only search and `get-chunk` keep
  working with no model present; only vector/hybrid queries need
  it.
- Trade-off, stated plainly: a nix-store fixed-output fetch
  would be hermetic and generation-atomic (old generations keep
  their model), at the cost of duplicating a few hundred MB per
  generation that references it and a brittle FOD restamp cycle
  (this fleet has already eaten one FOD hash rot, opencode#1580).
  The state-dir approach risks a machine whose model dir was
  hand-deleted; the loud-failure + `fetch-model` repair verb is
  the mitigation. This is flagged as open question Q1 (§7) —
  Drew may prefer hermetic, and flipping is a packaging-local
  change that touches nothing else in this design.

### Deploy shape on amd

- A systemd **user service** (`atlas-search.service`) declared in
  this repo's home-manager config, `ExecStart` = the packaged
  binary's `serve` verb, `Restart=on-failure`, reading config
  from `~/.local/state/atlas-search/service.json`. A user unit
  (not the OpenCode server's login-session pattern) is right
  here because atlas-serve reads everything from files — the
  login-session pattern exists for the OpenCode server's captured
  environment and password plumbing, which this service does not
  share. The unit is inert until the cutover phase (§5) enables
  it; packaging lands it disabled.
- State: `~/.local/state/atlas-search/` holds `service.json`,
  `model/`, and logs. Nothing else writes there.
- The Turso read-only token the service uses is resolved exactly
  as the Python resolves it (Bitwarden by name, or the `turso`
  CLI mint cache) — one credential, read-only, shared with the
  CLI's standalone path. **The service holds no write credential
  for any Turso database** (§6).

## 5. Parity gates and migration

The Python is the reference implementation and stays the eval
instrument permanently (eval3.py is the thing that produces
verdicts; a successor that grades itself would be worthless).
The binary earns each promotion by passing a gate, in this
order. Migration doctrine is additive-first throughout: the new
side is added alongside, never by removing the old side first.

### Gate A — embedder parity (bit-identical)

Before any search code matters, `atlas-embed` must reproduce
the Python pipeline exactly:

- A fixed probe set (`fixtures/embed-probes.json`): plain ASCII,
  mixed Japanese, a ~700-token long text, an empty-adjacent
  string, and each of several texts in both document form
  (prompt-free) and query form (ST query prompt applied) —
  recorded **once** from the Python sentence-transformers
  pipeline: token ids and output vectors.
- The gate: token ids exactly equal on every probe; output
  vectors **bit-identical** (delta 0.0). The fork's spike
  achieved delta 0.0 in-process, so bit-identity is the bar, not
  an aspiration — a platform that cannot hit it fails the gate
  loudly rather than earning a tolerance. (Padding semantics are
  the known trap: pad right, pad id 1, attention mask 0, matching
  the sidecar behavior the spike reverse-engineered; batch
  composition must not change a vector.)
- The probe fixtures are generated by a recorded, rerunnable
  Python script kept with the fixtures, so a model revision bump
  regenerates them through the same ceremony.

### Gate B — search parity (golden 311 tie)

With the embedder proven, the binary's search must tie the
Python's production path **on the production corpus** (107,639
chunks — the reference numbers are the Python run against the
same corpus on the same day, not the historical v3-corpus
189/0.3894, which used a smaller corpus):

- hits@10 within **±2 questions** of the Python's, overall MRR
  within **±0.005**, and no category (exact-fact / paraphrase /
  dictated-garbled / cross-session) regressing by more than
  **0.01 MRR**.
- Same configuration both sides: hybrid, RRF k=10, pool 100,
  default ceiling, same vector backend (whichever §5c selects).
- Run through the Python's own harness shape (eval3.py adapted
  to call the binary), so the comparison is paired per question
  and rank-level diffs are inspectable, not just aggregates.

### Gate C — the A/B winner is the implemented default

The running A/B (scan vs index-fill vs index-fixed, golden 311,
production corpus) settles the default `VectorBackend`. Core
implements all three strategies behind the trait regardless —
they already exist as Python modes and the flag is how they get
re-measured — but the _default_ is whichever the A/B crowns, and
Gate B is run with that default on both sides. If the A/B is
still running when the core step starts, the core step starts
with `scan` (the incumbent) and the default flips by a one-line
change + a Gate B rerun when the verdict lands; the design does
not block on it and does not pre-judge it.

### Migration phases

0. **Build alongside.** Packages land; the binary is invocable
   by explicit path; the service unit exists but is disabled.
   The Python is untouched and remains what `atlas-search`
   resolves to everywhere.
1. **CLI opt-in.** The binary installs under the name
   `atlas-search` in its own profile position _alongside_ the
   Python wrapper, which is renamed (not removed) to
   `atlas-search-py`. Callers opt in per environment. Gates A+B
   must be green before this phase.
2. **Service opt-in.** The unit is enabled on amd;
   `--via-service` works from laptop and sandbox CLIs; the
   service's clients table contains exactly the §3 v1 rows.
   Standalone remains the default everywhere.
3. **Default flip (Drew's call, per surface).** Home-manager
   swaps which implementation `atlas-search` resolves to,
   surface by surface (laptop first, sandbox after a soak).
   The Python stays installed and documented as the fallback
   and the permanent eval reference. Rollback at every phase is
   "point the name back" — there is no data migration anywhere
   in this program, because the system is read-only end to end.

## 6. What the service does NOT do

- **No writes to the production atlas DB.** Publishing, top-ups,
  and index maintenance stay with the existing Python pipeline.
  Mechanically enforced: the service's configuration contains
  only a read-only Turso token; there is no write credential on
  its code path to misuse.
- **No re-embedding of the corpus, ever.** The service embeds
  _queries_ only. Corpus embedding (initial passes, deltas) is
  the publish pipeline's job, with its own gates.
- **No publish-schema changes.** The missing secondary indexes
  behind the ~15 s filtered-query cost are the publish
  pipeline's concern; the service lives with the schema it is
  given (and its backpressure limits, §2, are sized to that
  reality).
- **No ceiling widening by clients.** Ceilings clamp server-side
  to token caps (§3); there is no request field, header, or job
  option that raises a caller's effective ceiling, and results
  are never cached across jobs or clients (the temp store is
  per-job, per-token, TTL'd — a second client repeating a query
  runs its own job under its own cap).
- **No public or LAN exposure.** Loopback + tailnet only (§3).
- **No Aster client in v1** (§3), and no generic "agent API
  key" — every client is a named row in `service.json`.
- **No replacement of the eval harness.** eval3.py and the
  Python CLI remain the measurement reference (§5).

## 7. Implementation step map and open questions

Steps (each is a filed issue under the program epic; gates are
the issue's exit criteria):

1. **Embedder parity spike** — `atlas-embed` crate + probe
   fixtures generated from the Python. Gate A. No dependencies.
2. **Core search library** — `atlas-core`: filters, ceilings,
   branch builders, fusion, envelope, both backends, all three
   vector strategies. Exit: core-level tests reproduce Python
   branch/fusion behavior on fixture data. No dependency on 1
   (fixture vectors stand in for the embedder).
3. **CLI mode** — the binary's one-shot commands at full Python
   surface parity (search, get-chunk, FTS-only, text rendering,
   local + Turso backends, `--via-service` client stub that
   errors "no service" until 4 exists). Depends on 1 + 2.
4. **Serve mode** — HTTP job protocol (§2), worker pool,
   in-memory temp store, token/ceiling enforcement (§3),
   one-shot endpoint. Depends on 2 (and uses 1 through 3's
   embedder wiring).
5. **Nix packaging + deploy** — `default.nix` with the §4 loader
   requirements verified, `fetch-model` + model pin constants,
   the (disabled) user unit, port-inventory check. Depends on
   3 + 4.
6. **Parity gate + cutover** — Gate B run and recorded, phases
   1–3 of §5 executed per Drew's per-surface calls, Python
   renamed-but-retained. Depends on 5 and on Gate A (step 1).

Open questions the design could not settle — each with the
recommendation Drew is being asked to ratify or flip:

- **Q1 — Model provisioning: state-dir fetch (recommended) vs
  nix-store fixed-output fetch.** §4 argues state-dir (weights
  are data; avoids another FOD restamp cycle) and accepts the
  hand-deletion risk with a loud failure + repair verb. The
  hermetic alternative is defensible in this fleet and is a
  packaging-local flip. _Recommendation: state-dir; flip only if
  generation-atomic model rollback becomes a stated need._
- **Q2 — Which client tokens, if any, get a cap above
  `personal` in v1?** §3 proposes Drew's own CLI only. Agent
  clients at `personal` cannot see local-only material (e.g.
  some of Drew's own working sessions), which will occasionally
  make agent-run searches thinner than Drew's own. _Recommendation:
  v1 as proposed — only Drew's CLI above `personal`; revisit
  per-client after real usage shows the cost, since a cap raise
  is a one-line `service.json` edit and a cap granted early is
  exposure that cannot be un-seen._
- **Q3 — When the service is healthy, should the sandbox CLI
  default to `--via-service`?** The sandbox has no local corpus
  and reaches amd over the tailnet anyway; the service gives it
  the warm embedder. _Recommendation: yes at phase 3, with the
  loud standalone fallback (§1) as the safety property; laptop
  CLI keeps standalone as its default until Drew says otherwise,
  since the laptop is where offline/local-publish use lives._
