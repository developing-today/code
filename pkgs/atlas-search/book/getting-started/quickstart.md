# Quickstart

The five-minute path. It assumes Atlas is installed and the model is
provisioned (see [Installation and provisioning](installation.md)) and runs
one search three ways — standalone, through the fleet client, and through
the hosted MCP endpoint — because those are the three ways anything in the
fleet actually searches. Then it follows one hit back to its chunk. Every
query and every result in this chapter is a toy example, invented for this
page; no real corpus text appears here.

Source: https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-search/src/main.rs and https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-search/src/cli.rs

The toy query, used unchanged on all three surfaces:

```text
weekend market rain plan
```

## 1. Standalone: `atlas-search search`

```sh
atlas-search search "weekend market rain plan" --k 5
```

Standalone is the default mode and it is complete: the query is embedded
in-process, the corpus is talked to directly (the hosted Turso backend when
a token resolves, else the local database file), and the answer is printed
as one JSON envelope on stdout. Nothing needs to be running. Add `--text`
for a human-readable rendering of the same envelope:

```sh
atlas-search search "weekend market rain plan" --k 5 --text
```

Two things worth knowing on the first run. First, the embedder is lazy: a
full-text-only search (`--mode fts`) and `get-chunk` never load the model,
so they work on a machine where `fetch-model` has not run yet, while hybrid
and vector search fail loudly there. Second, every standalone search writes
a query-trace record and returns its `query_id` in the envelope; a failure
to write the trace fails the search. Traces are the raw material of the
statistics and feedback chapters later in this book.

## 2. Through the fleet client: `fleet search`

```sh
fleet search "weekend market rain plan" --k 5 --text
```

`fleet search` is the fleet binary's search domain — the surface an agent
seat uses. The spelling `fleet search search "weekend market rain plan"`
is the same surface with `atlas-search`'s own shape; the leading `search`
token is consumed and the two forms are identical. The flags mirror the
standalone search's common subset: `--k` (default 10), `--pool` (default
100), `--mode` (default `hybrid`), the classification filters `--kind`,
`--topic`, `--project`, `--client`, the date filters `--since` / `--until`,
`--max-sensitivity` (default `personal`), `--expand`, `--with-message`,
`--hydrate`, `--text`, and the routing flags below. Engine-only knobs (the
backend and database flags, the query-parameter contract group) are not
part of this surface; the reference chapter
[fleet search and fleet serve search](../reference/fleet-search.md) lists
exactly which flags exist here and which do not.

Routing is the one behavioural difference from the standalone binary, and
it is seat-dependent by design (the Atlas Q3 routing decision):

- An explicit `--via-service` or `--standalone` flag wins, then the
  environment variable `ATLAS_SEARCH_VIA=service`, then the fleet config's
  `[search] via` setting.
- Otherwise the seat default applies: the **sandbox seat prefers the
  service**; the **laptop seat** — the serving host, recognised by the
  presence of the service config file — **defaults to standalone**.

Whichever route is chosen, a via-service failure is never silent: the
client prints
`fleet search: warning: --via-service: &lt;detail&gt;; falling back to standalone`
on stderr and delegates to the standalone artifact, whose output it
inherits byte-for-byte. If neither route is available — no service answer
and no standalone artifact on `PATH` — the command exits 3 and says so.
The via-service route needs the seat's client token, presented from
`ATLAS_SEARCH_TOKEN` or from `~/.config/atlas-search/client.json`; the
server clamps the requested ceiling to that client identity's cap.

## 3. Through the hosted endpoint: an MCP `tools/call`

The Cloudflare Worker at
`https://atlas-search.developing-today.workers.dev` exposes the same search
as a stateless MCP (Model Context Protocol) endpoint at `POST /mcp`, gated
by the same bearer token as its REST routes. In production the laptop's
mcpx daemon is connected to it as its `atlas` server, with the gate token
expanded from the daemon's environment at run time; the raw protocol shape
below is what that connection carries. The token is an environment
variable's value here — never a literal in a command you keep:

```sh
curl -s https://atlas-search.developing-today.workers.dev/mcp \
  -H "Authorization: Bearer $ATLAS_SEARCH_GATE_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
          "name": "search",
          "arguments": { "query": "weekend market rain plan", "k": 5 }
        }
      }'
```

The endpoint has exactly one tool, named `search`, with a required `query`
string and an optional `k` (an integer from 1 to 50, default 10 — the
Worker clamps any other value into that range). A successful call answers
with a JSON-RPC result whose `content` holds the search envelope as a JSON
text block and whose `structuredContent` holds the identical envelope as an
object, with `isError: false`. That envelope is the Worker's wrapper —
`cache` (`"miss"` or `"hit"`), `key` (the cache key), `query` (the
normalised query), `result` (the service's search envelope, passed through
verbatim), and `meta` (timestamps, hit count, timings, and the pinned model
and dimensions) — over exactly the same execution path as the Worker's
`POST /v1/search` REST route. An MCP search and a REST search for the same
normalised query and `k` share one cache entry; a repeat query is served
from the per-query Durable Object cache and reports `"cache": "hit"`.

Two pins define this surface and no request field can change them: the
disclosure context is `hosted-per-user` and the sensitivity ceiling is
`personal`. Every MCP caller shares that one hosted identity today. The
full protocol treatment is in [The Atlas MCP server](../reference/mcp.md).

## Following a hit: `get-chunk`

A search hit names a `chunk_id`. Fetch that chunk, with its provenance and
a little context on either side:

```sh
atlas-search get-chunk 900001 --neighbors 2 --text
fleet search get-chunk 900001 --neighbors 2 --text
```

Both print the chunk's text together with the session, client, and
sensitivity it came from, plus `N` neighbouring chunks from the same
message on either side (`--neighbors` default 0, maximum 100). The fleet
form always runs through the standalone artifact: `get-chunk` is
job-scoped in the service protocol, so there is no via-service path for
it, and when via-service is in effect the client says so loudly before
delegating. A chunk id that does not exist exits 1 with
`atlas-search: chunk &lt;id&gt; not found`.

## What the JSON result contains

The standalone and fleet searches print one envelope. Abridged, with toy
values:

```json
{
  "query": "weekend market rain plan",
  "mode": "hybrid",
  "backend": "turso",
  "filters_applied": { "max_sensitivity": "personal" },
  "results": [
    {
      "chunk_id": 900001,
      "session_id": "toy-session-1",
      "score": 0.181818,
      "rank_sources": { "fts": 1, "vector": 1 },
      "text": "if the market moves indoors when it rains, the stall list goes on the door",
      "session": {
        "title": "Toy example session",
        "client": "toy-client",
        "created_at": "2026-01-05T14:23:00Z",
        "kind": "chat",
        "topics": ["markets"],
        "projects": [],
        "sensitivity": "personal"
      },
      "vector_distance": 0.001881
    }
  ],
  "vector": {
    "model": "Snowflake/snowflake-arctic-embed-m-v2.0",
    "table": "vec_snowflake_m_v2_768_f32",
    "embedder": "auto",
    "trunc": null,
    "pool": 100,
    "vector_backend": "scan"
  },
  "notes": []
}
```

Reading it, top to bottom: `query`, `mode`, and `backend` record what was
asked and which backend answered. `filters_applied` records the filters
and ceiling that were actually enforced — check this field, not your
command line, when a result surprises you. Each entry in `results` carries
its `chunk_id` (the id `get-chunk` takes), its fused `score`, its
`rank_sources` (the rank it held in each branch that found it — a hit
found by only one branch has only that branch's entry), its `text`, a
`session` object with the provenance (title, client, creation time, kind,
topics, projects, sensitivity), and, for vector-branch hits, its
`vector_distance`. The `vector` block records the embedding space the
search ran in — model, vector table, embedder, truncation, pool, and
vector backend — because a score is only meaningful inside one space.
`notes` is where the surfaces speak up: a via-service fallback, a missing
local vector store, a shard fan-out diagnostic, and similar facts are
spliced here rather than printed only to a log. A served (via-service)
envelope additionally reports the `effective_ceiling` the server enforced
after clamping, which can be lower than the ceiling you requested — never
higher. With `--group-by-session`, `results` rolls chunk hits up to
sessions instead (score = the session's best chunk, plus a hit count);
with `--with-message`, each hit also carries its parent message's text,
truncated at 50,000 characters.

## The sensitivity default, in one paragraph

Every search in this chapter ran at the default ceiling, `personal`, on
the ladder `shareable < personal < local-only < secret-bearing`: chunks
classified `local-only` or `secret-bearing` were excluded inside the
branch queries, before any ranking happened, so they are absent from the
results rather than ranked low in them. Raising the ceiling is always
explicit — `--max-sensitivity secret-bearing` on the CLI (the ceiling
must be spelled out in full), the server's clamp to your client identity's
cap on the service route, and no mechanism at all on the Worker, which is
pinned to `personal`. Intersecting with the ceiling, each surface also
carries a disclosure context (the CLI's search defaults to
`fleet-internal`; `get-chunk`, the owner's explicit-id tool, defaults to
`local-owner`; the Worker is pinned to `hosted-per-user`), and the
effective ceiling is the lower of the two — neither widens the other. The
full model is
[Sensitivity tiers and client ceilings](../concepts/sensitivity.md).

## Where to go next

[The shape of the system](shape.md) explains why the three surfaces above
return the same envelope — one engine, three access surfaces, two
execution modes — and the reference chapter
[atlas-search: search and get-chunk](../reference/atlas-search.md) gives
every flag, default, failure shape, and exit code for the commands used
here.
