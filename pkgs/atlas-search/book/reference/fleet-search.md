# fleet search and fleet serve search

Source: [pkgs/fleet/crates/fleet-search/src/cli.rs](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-search/src/cli.rs), [pkgs/fleet/crates/fleet-search/src/serve.rs](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-search/src/serve.rs), and [pkgs/fleet/crates/fleet-search/src/client.rs](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-search/src/client.rs).

This chapter covers the fleet domain surface over Atlas, from the Atlas side. The `fleet` binary registers a `search` domain (argv0 alias `atlas-search`) that presents the Atlas corpus search to fleet seats, and a `serve` role that runs the Atlas service itself. The same surface is covered from the fleet side in the fleet book; here the emphasis is on how the two verbs route, what they deliberately omit relative to [atlas-search: search and get-chunk](atlas-search.md), and where fleet's behaviour differs from the standalone binary's.

There are two client verbs and one service verb:

- `fleet search` — hybrid full-text + vector search over the Atlas corpus, with classification filters.
- `fleet search get-chunk` — fetch one chunk with its provenance.
- `fleet serve search` — run the Atlas service in the foreground (the loopback HTTP job protocol over one shared Snowflake embedder and a read-only Turso backend; the atlas-search service rehomed, #5604 — same config, same wire, same port 18760).

## Invocation

```
fleet search <query> [options]
fleet search search <query> [options]        (atlas-search's own shape; the leading `search` token is consumed)
fleet search get-chunk <chunk-id> [--neighbors N] [--text]
fleet serve search [options]
```

The query is a single positional: `fleet search &lt;query&gt; …` and `fleet search search &lt;query&gt; …` are the same surface. A second positional is a usage error: `unexpected extra argument "…" (the query is one positional)`, exit 2. `fleet search` with no arguments prints the usage text (the shared `SEARCH_USAGE` text) to stderr and exits 2; `--help`/`-h` anywhere in the search arguments prints it and exits 0. Bare `fleet search --version` (or `-V`) prints `fleet search 0.1.0` and exits 0 — any trailing argument after `--version` is a usage error, exit 2.

Parser mechanics worth knowing: for search flags, both spellings are parsed — `--flag value` and `--flag=value` — matching the serve parser below and the standalone binaries (the search parser splits on `=` the same way, #13281). A value attached to a switch (`--text=true`) is a usage error. Every value flag requires a following token (`&lt;flag&gt; needs a value`, exit 2); a non-integer value for an integer flag is `&lt;flag&gt;: invalid integer "…"`, exit 2; any unrecognised `-`-prefixed token is `unknown option "…"`, exit 2.

## The search flags

| Flag                    | Argument                        | Default                                   | Meaning                                                                                                                                                                                                                                                                                                                         |
| ----------------------- | ------------------------------- | ----------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--k`                   | integer                         | `10`                                      | Final results. Validated ≥ 1.                                                                                                                                                                                                                                                                                                   |
| `--pool`                | integer                         | `100`                                     | Per-branch candidates, validated 1..=400.                                                                                                                                                                                                                                                                                       |
| `--mode`                | `hybrid\|fts\|vector`           | `hybrid`                                  | Which retrieval branches run.                                                                                                                                                                                                                                                                                                   |
| `--kind`                | CSV                             | —                                         | Classification kind filter (CSV split, trimmed, empties dropped).                                                                                                                                                                                                                                                               |
| `--topic`               | CSV                             | —                                         | Topic filter.                                                                                                                                                                                                                                                                                                                   |
| `--project`             | CSV                             | —                                         | Project filter.                                                                                                                                                                                                                                                                                                                 |
| `--client`              | CSV                             | —                                         | Client filter.                                                                                                                                                                                                                                                                                                                  |
| `--since`               | `YYYY-MM-DD`                    | —                                         | Sessions created on/after the date (inclusive). Passed through unvalidated by this parser.                                                                                                                                                                                                                                      |
| `--until`               | `YYYY-MM-DD`                    | —                                         | Sessions created on/before the date (inclusive). Passed through unvalidated.                                                                                                                                                                                                                                                    |
| `--max-sensitivity`     | ladder name                     | `personal`                                | Caller ceiling on the ladder `shareable &lt; personal &lt; local-only &lt; secret-bearing`. The server clamps it to the client identity's ceiling (see Provisioning). An unrecognised name is not clamped here: the via-service attempt is declined and the standalone artifact produces atlas-search's exact validation fatal. |
| `--group-by-session`    | (switch)                        | off                                       | Roll chunk hits up to sessions.                                                                                                                                                                                                                                                                                                 |
| `--expand`              | integer                         | `0`                                       | Include N neighbouring chunks around each hit. Validated 0..=100, atlas-search's cap.                                                                                                                                                                                                                                           |
| `--with-message`        | (switch)                        | off                                       | Include the parent message text.                                                                                                                                                                                                                                                                                                |
| `--no-cache`            | (switch)                        | off                                       | Bypass service cache reads; the fresh result still lands in the cache.                                                                                                                                                                                                                                                          |
| `--no-wait`             | (switch)                        | off                                       | Do not block: print the service job handle and return. There is no `--wait` flag on this surface; waiting is implied (`wait = !no_wait`).                                                                                                                                                                                       |
| `--hydrate`             | integer                         | `0`                                       | Hydrate the top N results to full chunk + message content. Validated ≤ 1000, atlas-search's cap.                                                                                                                                                                                                                                |
| `--raw-rows[=branches]` | optional value                  | off                                       | Ungrouped rows: bare `--raw-rows` for flat fused rows, `--raw-rows=branches` for pre-fusion branch rows. Any other `=value` is a usage error.                                                                                                                                                                                   |
| `--text`                | (switch)                        | off                                       | Human-readable output instead of JSON.                                                                                                                                                                                                                                                                                          |
| `--model`               | name                            | `Snowflake/snowflake-arctic-embed-m-v2.0` | Embedding model. (The fleet help text once abbreviated this to "Snowflake arctic-embed-m-v2.0"; it now spells the full name, matching the parser's default.)                                                                                                                                                                    |
| `--embedder`            | `auto\|fastembed\|st`           | `auto`                                    | Query embedder selection.                                                                                                                                                                                                                                                                                                       |
| `--trunc`               | integer                         | `0`                                       | Truncate vectors to N dims (0 = full). Validated ≥ 0.                                                                                                                                                                                                                                                                           |
| `--vector-backend`      | `scan\|index-fill\|index-fixed` | `scan`                                    | Turso vector branch strategy.                                                                                                                                                                                                                                                                                                   |
| `--via-service`         | (switch)                        | off                                       | Prefer the Atlas service (job protocol). Conflicts with `--standalone`: giving both is a usage error, exit 2.                                                                                                                                                                                                                   |
| `--standalone`          | (switch)                        | off                                       | Run standalone via the atlas-search artifact. This flag is fleet-only; atlas-search has no `--standalone`.                                                                                                                                                                                                                      |
| `--service-url`         | URL                             | resolution chain below                    | Service base URL (env `ATLAS_SEARCH_URL`; default `http://127.0.0.1:18760`).                                                                                                                                                                                                                                                    |

`get-chunk` documents two flags: `--neighbors &lt;N&gt;` (include N chunks either side in the same message) and `--text` (human-readable output). In fact the fleet verb parses none of its arguments: after answering `--help` itself, it forwards every argument after the `get-chunk` token verbatim to `atlas-search get-chunk …`. So the artifact's full get-chunk surface — `--context`, `--backend`, `--db`, `--vecdb`, `--via-service`, `--service-url` — also works through this verb; the fleet help documents the delegation (#13281). Conversely, a mistyped flag is caught, if at all, by the artifact, with atlas-search's wording and exit codes.

### The 20 atlas-search flags fleet omits

The fleet help names its surface the subset of `atlas-search search` that translates to the wire request or to routing (an earlier wording claimed it was "mirroring" atlas-search, and an earlier count here said 21; the enumeration below is 20 names, verified against atlas-search's argument parser, #13281). The omitted flags fall in these categories:

- **Disclosure**: `--context`.
- **Backend and storage selection** (standalone-only engine knobs): `--backend`, `--db`, `--vecdb`, `--vec-table`, `--shards`.
- **The query-parameter contract group**: `--parameters_version`, `--shard_top_k`, `--global_top_k`, `--fts_weight`, `--vector_weight`, `--jaccard_depth`, `--jaccard_threshold`, `--rrf_k`.
- **Unimplemented-stage and variant controls**: `--planner_enabled`, `--spell_enabled`, `--query_class_hint`, `--variant`, `--raw`.
- **Wait control**: `--wait` (fleet implies waiting and offers only `--no-wait`).

This is a hard boundary, not a pass-through gap: on the standalone route the caller's raw argv is delegated to the atlas-search artifact with `--via-service`, `--standalone`, and `--service-url &lt;value&gt;` stripped — but the fleet parser runs first, so a flag it rejects never reaches the artifact. It is a usage error at the fleet layer.

### `--expand` and `--hydrate` validation

Fleet's via-service request builder applies the same caps atlas-search does (#13281): `--expand` is validated 0..=100 (atlas-core's `MAX_EXPAND_NEIGHBORS`) and `--hydrate` at most 1000 (atlas-proto's request-validation cap), with the same fatal-not-clamped shape. (Before #13281 the fleet builder checked only ≥ 0 and left the caps to the server on submit, or to the delegated artifact on the standalone route.)

## Routing: which route a search takes

A search runs one of two ways: **via-service** (submitted to a running Atlas service over the job protocol) or **standalone** (delegated to the `atlas-search` artifact, which embeds the query in-process and talks to the backend directly). Resolution order, highest first:

1. **Flags**: `--via-service` → via-service; `--standalone` → standalone.
2. **Environment**: `ATLAS_SEARCH_VIA=service` → via-service. (Only the exact value `service` counts.)
3. **fleet.toml `[search]`**: `via = "service"` or `via = "standalone"`. Any other value is ignored and resolution falls through.
4. **Seat default** (Atlas Q3): if the serving-host marker exists — the service config file, resolved via `ATLAS_SEARCH_SERVICE_CONFIG` else `~/.local/state/atlas-search/service.json` — the seat is the **laptop seat** and defaults to **standalone**. Otherwise the seat is a **sandbox seat** and defaults to **via-service**.

The service URL resolves on its own chain: `--service-url` flag → fleet.toml `[search] url` → `ATLAS_SEARCH_URL` → `http://127.0.0.1:18760`.

The sandbox seat's via-service preference comes with a **loud standalone fallback**: a via-service failure of any kind prints `fleet search: warning: --via-service: &lt;detail&gt;; falling back to standalone` on stderr and delegates to the artifact. The delegation strips `ATLAS_SEARCH_VIA` from the child's environment, so an ambient value cannot silently re-route the standalone run. If no standalone artifact is available on PATH either, that is exit 3 (see Exit codes).

`get-chunk` never takes the service route: chunk reads are job-scoped in the protocol, so the verb always runs standalone. When via-service was _requested_ — a `--via-service` token in the raw arguments, `ATLAS_SEARCH_VIA=service`, or fleet.toml `[search] via = "service"` (the seat default alone does not count) — fleet first prints its loud fallback warning, then delegates.

## Provisioning: the via-service token and the server-side clamp

A via-service client presents a per-seat bearer token, resolved in this order:

1. The `ATLAS_SEARCH_TOKEN` environment variable.
2. The JSON file at `~/.config/atlas-search/client.json` (path overridable via `ATLAS_SEARCH_CLIENT_CONFIG`): a JSON object with one `token` string field, expected mode 0600.

Identities are named in the service config, and the **server clamps `--max-sensitivity` to the identity's ceiling** — the client cannot widen its own grant by asking. Named examples: `drew-cli` is capped at `secret-bearing`; `sandbox-scripts` at `personal`. A seat with no `client.json` cannot use the service route and falls back to standalone.

## `fleet serve search`: the service role

```
usage: fleet serve search [options]

  --config PATH         service config (default ~/.local/state/atlas-search/service.json; env ATLAS_SEARCH_SERVICE_CONFIG)
  --listen ADDR         listen address override (loopback only)
  --job-ttl-ms N        completed-job TTL override, milliseconds (default 600000)
  --hard-ttl-ms N       submit hard-cap override, milliseconds (default 3600000)
  -h, --help            this help
```

The serve parser accepts both `--flag value` and `--flag=value` spellings (unlike the search parser). `-h`/`--help`/`help` print the help and exit 0; unknown flags and stray positionals are `unexpected argument '&lt;raw&gt;'`, exit 2; a non-numeric TTL or a non-socket `--listen` value exits 2 naming the value. `init` (the client table) and `fetch-model` stay on the standalone atlas-search artifact. The role's server code is a copy of atlas-search's `server.rs`, parity-checked in CI; config, wire, port (18760), and client table are identical to `atlas-search serve`.

### Source-wins note: `fleet search serve`

`fleet help search` lists `serve` among the domain's commands, while the client parser in `cli.rs` contains a branch that rejects `serve` (with `init` and `fetch-model`) as "an atlas-search service verb, not part of the fleet search client surface". The source comment and the live behaviour agree that the rejection branch is **dead for `serve`**: the fleet binary routes `search serve` to the serve role _before_ the client parser sees it, so `fleet search serve` actually starts the service — probed live, it attempted to bind `127.0.0.1:18760` and failed with `Address already in use`, exit 1. The branch is still live for `init` and `fetch-model`, which do produce the usage error, exit 2. This chapter states the routed behaviour: `fleet search serve` is `fleet serve search`. (The dead-for-`serve` branch's text was corrected in #13281: `serve` now has its own arm naming the routed shape, separate from the `init`/`fetch-model` rejection.)

## Worked examples

```
$ fleet search 'toy queue lease' --k 5 --text
$ fleet search search 'toy build failure' --mode fts --since 2026-11-01
$ fleet search get-chunk 112081 --neighbors 2 --text
```

The first uses the short shape on whatever route the seat resolves; the second spells the leading `search` token (atlas-search's own shape) and restricts the search to the FTS branch; the third fetches one chunk and its two neighbours on each side, rendered as text — always via the standalone artifact, per the routing rules above.

## Failure shapes

- Usage errors — unknown option, missing flag value, invalid integer, `--via-service` with `--standalone`, a second positional, trailing arguments after `--version`, and the rejected `init`/`fetch-model` verbs: message plus usage on stderr, exit 2.
- Via-service failure with a standalone artifact available: the loud warning on stderr, then the standalone result — the command succeeds; the fallback is reported, not hidden.
- A service-job or backend failure surfaced without a fallback, and a delegated artifact's exit 1, propagate as exit 1. The serve role's runtime failures (backend token, bind) are also exit 1.
- No service route and no standalone artifact on PATH: `fleet search: the standalone atlas-search artifact is not available on PATH (…)`, exit 3.
- A fleet build that cannot serve the `search` domain at all answers with a stub and exit 69, the fleet-wide convention.

## Exit codes

| Code | Meaning                                                                                                                                                 |
| ---- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 0    | Success — including `--help` and bare `--version`. (A running `fleet serve search` service does not exit 0; 0 there is help only.)                      |
| 1    | Runtime failure: a service-job/backend failure with no fallback, the delegated artifact's exit 1, or a serve-role runtime failure such as a bind error. |
| 2    | Usage/validation: unknown options, missing values, conflicts, rejected verbs, and serve-role usage/config errors.                                       |
| 3    | Routing unavailable: no service route and the standalone atlas-search artifact is not on PATH.                                                          |
| 69   | Domain stub: this fleet build cannot serve the `search` domain.                                                                                         |
