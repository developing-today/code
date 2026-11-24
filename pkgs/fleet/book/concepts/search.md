# Search: the Atlas corpus

The `search` domain (argv0 alias `atlas-search`) is the client surface for
Atlas: hybrid search over the fleet's session-history corpus. The domain has
two client verbs — `search` and `get-chunk` — and, under `fleet serve
search`, the search service itself. `init` and `fetch-model` deliberately
stay on the standalone atlas-search artifact; invoking them through fleet is
a usage error naming that artifact.

Source: https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-search/src/cli.rs

## Hybrid FTS + vector

A search is **hybrid**: a full-text (FTS) branch and a vector branch each
produce candidates, and the results are fused. The default mode is `hybrid`;
`--mode fts|vector` runs one branch. The embedding model is Snowflake
arctic-embed-m-v2.0 — the source constant is
`Snowflake/snowflake-arctic-embed-m-v2.0` (the help text's spelling drops
the org slash; the source spelling is the operative one).

Client flags with their defaults:

| Flag                                                            | Default                       | Meaning                                                                                             |
| --------------------------------------------------------------- | ----------------------------- | --------------------------------------------------------------------------------------------------- |
| `--k N`                                                         | 10                            | Final result count.                                                                                 |
| `--pool N`                                                      | 100                           | Per-branch candidates; validated 1..=400.                                                           |
| `--mode MODE`                                                   | `hybrid`                      | `hybrid \| fts \| vector`.                                                                          |
| `--kind CSV` / `--topic CSV` / `--project CSV` / `--client CSV` | —                             | Classification filters.                                                                             |
| `--since YYYY-MM-DD` / `--until YYYY-MM-DD`                     | —                             | Sessions created on/after / on/before (inclusive).                                                  |
| `--max-sensitivity LV`                                          | `personal`                    | `shareable \| personal \| local-only \| secret-bearing`. A _request_; the server clamps it (below). |
| `--group-by-session`                                            | off                           | Roll chunk hits up to sessions.                                                                     |
| `--expand N`                                                    | 0                             | Include N neighbouring chunks around each hit.                                                      |
| `--with-message`                                                | off                           | Include the parent message text.                                                                    |
| `--no-cache`                                                    | off                           | Bypass service cache _reads_; the fresh result still lands in the cache.                            |
| `--no-wait`                                                     | off                           | Do not block: print the service job handle and return. (The one-shot default waits for the job.)    |
| `--hydrate N`                                                   | 0                             | Hydrate the top N results to full content.                                                          |
| `--raw-rows[=branches]`                                         | —                             | Ungrouped rows: bare = flat fused rows; `=branches` = pre-fusion branch rows.                       |
| `--text`                                                        | off                           | Human-readable output instead of JSON (JSON is the default).                                        |
| `--model NAME`                                                  | Snowflake arctic-embed-m-v2.0 | Embedding model.                                                                                    |
| `--embedder NAME`                                               | `auto`                        | `auto \| fastembed \| st`.                                                                          |
| `--trunc N`                                                     | 0                             | Truncate vectors to N dims (0 = full).                                                              |
| `--vector-backend B`                                            | `scan`                        | `scan \| index-fill \| index-fixed`.                                                                |
| `--service-url URL`                                             | `http://127.0.0.1:18760`      | Service base URL; env `ATLAS_SEARCH_URL`, then fleet.toml `[search] url`, override it.              |

Standalone-only engine knobs (`--backend`, `--db`) are deliberately not part
of this surface.

`get-chunk <chunk-id> [--neighbors N] [--text]` fetches one chunk with its
provenance (session, client, sensitivity); `--neighbors N` includes N
chunks either side **in the same message**.

Invocation shapes: `fleet search <query> [options]`, and
`fleet search search <query> [options]` (atlas-search's own shape — the
leading `search` token is stripped). Exit codes mirror atlas-search: 0 ok,
2 usage/validation, 1 service-job/backend failure surfaced without a
fallback, 3 routing unavailable (no service and no standalone artifact).

## Via-service vs standalone, by seat

There are two routes, and the seat decides the default:

- **Via-service**: an in-process client speaking the Atlas service's
  loopback HTTP job protocol to the service at `--service-url` (default
  `http://127.0.0.1:18760`): submit a search job, wait for it (unless
  `--no-wait`), and render the result through writers ported for
  byte-for-byte shape compatibility with atlas-search. The backend behind
  the service is a read-only Turso corpus store with one shared Snowflake
  embedder.
- **Standalone**: there is no second embedder or backend in the fleet
  workspace — the client **delegates to the kept standalone `atlas-search`
  binary on PATH**, passing the caller's arguments in atlas-search's own
  shape (with `--via-service`/`--standalone` stripped, and `ATLAS_SEARCH_VIA`
  removed from the child's environment so an ambient value cannot silently
  re-route it), inheriting its stdout byte-for-byte. A missing artifact is
  exit 3, naming it as "the search domain's kept standalone".

Routing precedence: explicit `--via-service` / `--standalone` first (the
two together are a usage error); then env `ATLAS_SEARCH_VIA=service|standalone`;
then fleet.toml `[search] via`; then the seat default:

- **Sandbox seat → via-service**, with a **loud** standalone fallback (a
  warning on stderr naming the failure). The via-service client is
  proxy-blind; the sandbox reaches the laptop's loopback service through
  the loopback forward on the shared laptop-ssh control connection — the
  deployed atlas-search wrapper's pattern, not a second mechanism.
- **Laptop seat → standalone.** The seat signal is the service's own config
  file, `~/.local/state/atlas-search/service.json`: only the serving host
  has one.

`get-chunk` **always runs standalone** — it is job-scoped in the protocol,
so it cannot ride the service route. When via-service was _requested_ (by
flag, env, or config — the seat default does not count) it prints the loud
fallback note first.

## Sensitivity clamping

Terminology: the ladder's canonical names are
`public < internal < private < sensitive`. The flag values and identity
caps in this chapter are spelled as the service spells them today
(`shareable` / `personal` / `local-only` / `secret-bearing`) until the
identifier rename (#13506) lands.

The via-service client presents a per-seat bearer token — env
`ATLAS_SEARCH_TOKEN`, else the JSON file at
**`~/.config/atlas-search/client.json`** (a JSON object with one token
string field, file mode 0600). Identities are named in the service config,
and the **server** clamps `--max-sensitivity` to the identity's ceiling:
identity `drew-cli` → `secret-bearing`; identity `sandbox-scripts` →
`personal`. The fleet verb adds no ceiling logic of its own — a client can
ask for more than its ceiling, and the service silently serves at the
ceiling. A seat with no `client.json` cannot use the service route and
falls back to standalone.

## The serve search service shape

`fleet serve search` (also spelled `fleet search serve`, and reachable via
the `atlas-search serve` argv0 route) is the atlas-search service rehomed
into fleet — same config, same wire. Both spellings require the
`search-serve` capability, not the `search` client capability.

```
usage: fleet serve search [options]
  --config PATH     service config (default ~/.local/state/atlas-search/service.json;
                    env ATLAS_SEARCH_SERVICE_CONFIG)
  --listen ADDR     listen address override (loopback only)
  --job-ttl-ms N    completed-job TTL override, ms (default 600000)
  --hard-ttl-ms N   submit hard-cap override, ms (default 3600000)
```

Its guarantees:

- **One shared embedder per process**: a single lazily loaded embedder (one
  ONNX session, mutex-held), loaded on the first vector query only —
  FTS-only jobs and a cold service work without it. Its load state is what
  `/health` reports. The model is the Snowflake
  `snowflake-arctic-embed-m-v2.0` model at a pinned snapshot revision; the
  model directory resolves from `ATLAS_MODEL_DIR` or the Nix closure.
- **A mechanically read-only backend**: the Turso backend's token comes only
  from the service config chain, never the environment; the resolution chain
  is env, the Bitwarden item named `atlas-ro`, or the turso CLI's
  `--read-only` mint; the query layer issues nothing but SELECTs. There is
  no write path to opt into.
- **The job protocol** is a bounded async queue with in-memory temp state,
  per-client recovery listing, and dedupe keyed by the effective request
  (never across effective sensitivity ceilings).
- The engine is a verbatim copy of atlas-search's, held byte-for-byte by a
  parity check script and the code repo's `fleet-search-parity-guard`
  workflow. Rollback is pointing the unit's ExecStart back at
  `atlas-search serve`.

Exit codes for the service: 2 usage/config, 1 runtime failure; a running
service never exits 0.
