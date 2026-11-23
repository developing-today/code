# Add a command or a tool

Source: `pkgs/atlas-search/crates/atlas-search/src/main.rs`, `pkgs/atlas-search/crates/atlas-proto/query-parameters-v1.json`, `pkgs/atlas-search/crates/atlas-proto/src/lib.rs`, `pkgs/atlas-search/crates/atlas-search/src/server.rs`, `pkgs/fleet/crates/fleet-search/src/cli.rs` and `pkgs/atlas-search/worker/src/mcp.ts` in https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-search/src/main.rs — provenance only; this chapter is complete without following it.

"Verb" here means any new invocable surface: a subcommand on an Atlas
binary, or a tool on the Atlas MCP server. Both routes are end-to-end
checklists. A verb is not done when its handler works; it is done when the
parser, the defaults, every surface that mirrors it, the help text, the
tests and this book all describe the same verb — in the same change, per
[Keeping this book current](keeping-this-book-current.md).

## Part 1 — a new `atlas-search` verb

### 0. Pick the host binary first

The six binaries have two parser cultures, and a new verb inherits its
host's:

- **clap 4 derive**: `atlas-search` (`crates/atlas-search/src/main.rs`) and
  `atlas-stage` (`crates/atlas-search/src/bin/atlas-stage.rs`). Most new
  verbs belong on `atlas-search`.
- **Hand-rolled strict parsers**: `atlas-classify`, `atlas-eval`,
  `atlas-sweep` and `atlas-github-comments`, all in
  `crates/atlas-core/src/bin/`. These declare their whole command line in
  the source header and reject anything unaccounted for. Add a verb here
  only when the verb belongs to that binary's operating procedure.

Exit-code conventions differ by host and are part of its contract:
`atlas-search` uses 2 for usage/validation errors and 1 for runtime
failures; `atlas-classify` and `atlas-sweep` route _every_ failure to 2;
`atlas-github-comments` routes every failure to 1; `atlas-stage` prints
errors as `atlas-stage: &lt;msg&gt;` and exits 1. Match the host. Never invent a
new code for a shape the host already has.

### 1. The parser

Declare the verb in the host's parser: the subcommand, its positionals,
its flags with their help text, and any flattened argument groups it
shares. On `atlas-search`, backend flags (`--backend`, `--db`, `--vecdb`,
`--via-service`, `--service-url`) live in the flattened `BackendArgs`
group and vector flags in `VectorArgs` — reuse the group instead of
re-declaring its flags, so the new verb's backend behaviour cannot drift
from `search`'s.

Validation runs **before** any backend or service work, and violations
are usage errors. The existing shape: numeric ranges are checked in the
command handler together with `SearchParametersV1::validate` (for
example `--pool` must be 1..=400, `--expand` 0..=100), and the error is
printed as `atlas-search: &lt;msg&gt;` with exit 2. A flag that names a stage
that does not exist yet is a validation error, not a silent no-op — the
precedent is `--planner_enabled`, which the parser accepts and validation
refuses with "planner/spell stages are not implemented yet".

### 2. Defaults and `RunConfig`: the contract file is the source

If the verb takes a parameter that is part of the query contract — a
depth, a weight, a fusion constant, a mode — its default does **not**
live in the parser. It lives in
`crates/atlas-proto/query-parameters-v1.json`, the versioned contract
file the CLI defaults are derived from (`k` = 10, `pool` = 100,
`rrf_k` = 10, `fts_weight` = 1.0, `vector_weight` = 1.0,
`jaccard_depth` = 0, and the rest), with its typed form and `validate`
in atlas-proto's `SearchParametersV1`. That one file is what keeps the
CLI, the service and the fleet client agreeing on what an unstated
parameter means. A default hardcoded only in a parser is a second source
of truth and will diverge.

The same rule applies inside the library: a verb's run configuration is a
struct next to its implementation (the classification runner's
`RunConfig` in atlas-core's `classify` module is the shape), with its
defaults in one `Default` impl — not literals scattered through the
handler.

### 3. Service exposure — decide it explicitly

Ask, and record the answer in the change: does this verb work
`--via-service`?

- **If yes**: the work is in three more places. The request/response
  types go in `atlas-proto` (the crate both sides share); the route and
  handler go in the service (`crates/atlas-search/src/server.rs` and its
  `src/server/` submodules); the client path goes in the CLI/service
  layer. The service-side rules are not optional: the caller's ceiling
  is clamped to the authenticated client's cap
  (`effective = min(requested, cap)`), the clamp is recorded and
  reported in the envelope, and a request the service cannot serve in
  the requested shape falls back to standalone **loudly** — a stderr
  warning plus a note in the envelope's `notes`, never a silent reroute.
  The precedents are the search job protocol and its fallbacks, in
  [The service HTTP API](../reference/service-api.md).
- **If no**: say why, in the verb's help or the book chapter. The
  precedent is `get-chunk`: chunk reads are scoped to an existing job in
  the protocol, so there is no via-service path for it — requesting one
  produces the loud fallback note and the chunk is served standalone.
  `init` and `fetch-model` are the other standing examples: service
  administration stays on the standalone binary by design.

### 4. The fleet-search parity question

`fleet search` is a second surface over the same semantics, and every new
verb forces the question: does the fleet surface get this too?

- The fleet surface deliberately carries only the subset of
  `atlas-search`'s arguments that translates to the wire request or to
  routing. Standalone-only engine knobs (`--backend`, `--db`,
  `--vec-table`, the contract-tuning flags) are absent from it by design,
  and a flag the fleet parser rejects can never reach the standalone
  artifact through `fleet search` — it is a usage error first. "Fleet
  does not get it" is a legitimate answer; record it where the verb is
  documented.
- If fleet does get it: the touches are the parser and usage text in
  `pkgs/fleet/crates/fleet-search/src/cli.rs`, the client-side protocol
  types in its `proto.rs`, and — if the verb is a service role — the
  serve half in `serve.rs`/`server.rs`, which is a maintained copy of
  the atlas-search server.
- If the change touches either output writer — `pyjson.rs` (the
  Python-shaped JSON writer) or `render.rs` (the envelope renderer) — in
  either package, the byte-parity contract applies: the fleet copies
  must stay byte-identical to the atlas-search originals outside their
  pinned parity headers, and the parity guard
  (`.github/workflows/fleet-search-parity-guard.yml`, dispatch-only) is
  part of the change's verification. See [Building and testing](building.md).

### 5. Help text

Help text is a contract surface, not decoration. The verb's `--help`
must state every flag, its default as a literal value, and any behaviour
a caller would otherwise have to read source to learn. Where help text
and source end up disagreeing anyway, this book documents the **source**
and calls out the discrepancy until the help is fixed — see the worked
examples in [Building and testing](building.md).

### 6. Tests

- Parser and validation tests for every new flag: the accepted range,
  the boundary values, and the exact failure shape outside it.
- A named oracle for every behavioural expectation — the rule and the
  existing examples are in [Building and testing](building.md). If the
  verb changes an envelope or a SQL string, the expectation comes from
  the recorded parity fixtures (`fixtures/core-expectations.json`,
  recorded from the Python reference), regenerated with
  `gen_core_fixtures.py` when the change is deliberate.
- If the verb renders `--text` output, a golden file under
  `crates/atlas-search/tests/golden/` joins the CLI golden tests.
- If the verb reads or writes a store, a test against the miniature
  fixture corpus — including the ceiling case: content above the default
  ceiling is absent, not down-ranked.

### 7. The book, in the same change

Update the verb's reference chapter (the `atlas-search` chapters are
[atlas-search: search and get-chunk](../reference/atlas-search.md) and
[atlas-search: serve, init, fetch-model, traces and feedback](../reference/atlas-search-admin.md);
each auxiliary binary has its own), this chapter if the change alters
the pattern it teaches, and any chapter that states a default the verb
changes. The full contract is [Keeping this book current](keeping-this-book-current.md).

## Part 2 — a new MCP tool

The MCP surface is the Cloudflare Worker's `POST /mcp` endpoint
(`pkgs/atlas-search/worker/src/mcp.ts`): a stateless Streamable HTTP
server with exactly one tool today, `search`. Its reference is
[The Atlas MCP server](../reference/mcp.md). Adding a tool touches four
places, and the fourth is the one that usually decides whether the tool
may exist at all.

### 1. The tool definition and handler (`src/mcp.ts`)

- Add the tool's definition beside `SEARCH_TOOL` and return it from
  `tools/list`: a `name`, a `description` that states the surface's
  pinned context and ceiling in plain words (the `search` description
  does — copy its honesty), an `inputSchema` (JSON Schema; `query`-style
  required fields named in `required`), and `annotations`
  (`readOnlyHint`, `destructiveHint`, `idempotentHint`,
  `openWorldHint`) that tell the truth about the tool.
- Add the dispatch in `tools/call`: validate the tool name (unknown →
  JSON-RPC error `-32602`, `unknown tool: &lt;name&gt;`), then validate the
  arguments with the same error code and message shape the `search`
  tool uses (`arguments.query must be a string`,
  `arguments.query must not be empty` — emptiness judged **after**
  normalization).
- A successful call returns the payload twice, identically:
  `content[0].text` as the JSON string and `structuredContent` as the
  object, with `isError: false`. A failed execution returns
  `isError: true` with the error payload as the text content, no
  `structuredContent`, and the HTTP status stays 200.

### 2. The execution path

A search-like tool calls the **same** `executeSearch` in `src/search.ts`
as `POST /v1/search` — that is what makes an MCP search and a REST search
for the same normalized query and `k` share one Durable Object cache
entry, with the same ceiling pinning and cache keying. Do not write a
second path to the service. A tool that needs a new upstream call adds it
to `src/service.ts` (the `postJson` shape: service bearer plus the
Cloudflare Access pair, names only, never values in code or docs) and
validates the response in `src/space.ts`'s style before anything is
cached.

### 3. Worker tests

Extend `worker/test/mcp.test.ts` (and `worker.test.ts` if the tool shares
the REST path): the `tools/list` result, the call's success shape
(text content JSON-equals `structuredContent`), and every validation
failure. Note that the current suite asserts `tools/list` returns
**exactly one** tool — a second tool fails that assertion deliberately,
and updating it is part of the change, not a workaround.

### 4. The gate and identity consequences — read before building

This surface has **no per-client identity**, and a new tool inherits
that fact:

- The gate is checked once, in `src/index.ts`, before dispatch: one
  shared gate token (secret name `GATE_TOKEN`) for every gated route,
  `/mcp` included. A valid check yields a boolean — no caller name, no
  per-agent credential. `mcp.ts` receives only a `SearchEnv`
  (`{ QUERY_CACHE }`); it never sees the token.
- Every MCP caller therefore shares the one hosted identity: disclosure
  context `hosted-per-user`, sensitivity ceiling `personal`, both
  compile-time constants in `src/space.ts` that no request field, header
  or environment variable can change, pinned into the cache key and
  verified against the service's reported `effective_ceiling` before a
  result may be cached.
- So: a tool that needs a different ceiling, a different context,
  per-agent scoping or an audit identity **cannot be improvised on this
  endpoint**. Per-agent identity stamping, per-agent ceilings and corpus
  scoping are later slices of #8625, explicitly not built yet. A tool
  that is a write, or an operator/audit surface, does not belong here at
  all — the standing precedent is that cache purge (a write) and cache
  inspect (an operator surface) are deliberately **not** MCP tools and
  live on the gated REST surface in [The Worker HTTP API](../reference/worker-api.md).

If the tool passes that test — read-only, safe at the `personal`
ceiling, safe for every gate holder to call — build it by the steps
above, and update [The Atlas MCP server](../reference/mcp.md) in the
same change.
