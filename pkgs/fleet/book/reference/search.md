# fleet search

`fleet search` is the atlas-search client surface over the live Atlas service contract: **hybrid FTS+vector search over the Atlas corpus**, with classification filters. Search "joins unchanged" after the service cutover; the service role itself is `fleet serve search` (see [fleet serve](serve.md)). Two verbs: `search` (the default) and `get-chunk`. `init` and `fetch-model` are usage errors naming the standalone artifact — they stay on the standalone atlas-search artifact.

See also [Search: the Atlas corpus](../concepts/search.md).

Invocation shapes:

```
fleet search <query> [options]
fleet search search <query> [options]   (atlas-search's own shape — the leading `search` token is stripped)
fleet search get-chunk <chunk-id> [options]
```

## Routing (both verbs)

- **Via-service route:** an in-process client speaking the Atlas service's **loopback HTTP job protocol** to the service at `--service-url` (default `http://127.0.0.1:18760`): submit a search job, wait for it (unless `--no-wait`, which prints the job handle), render the result so output matches atlas-search in shape. The backend behind the service is a read-only Turso corpus store with one shared Snowflake embedder.
- **Standalone route:** there is no second embedder/backend in the fleet workspace — the client **delegates to the kept standalone `atlas-search` binary on PATH**, passing the caller's arguments in atlas-search's own shape (with `--via-service`/`--standalone` stripped, and `ATLAS_SEARCH_VIA` removed from the child's environment so an ambient value can't silently re-route it), inheriting its stdout byte-for-byte. A missing artifact is exit 3 with a message naming it as "the search domain's kept standalone".
- **Precedence:** explicit `--via-service` / `--standalone` first; then env `ATLAS_SEARCH_VIA=service|standalone`; then fleet.toml `[search] via`; then the seat default:
  - **Sandbox seat → via-service**, with a **loud** standalone fallback (stderr: "fleet search: warning: --via-service: {detail}; falling back to standalone"). The via-service client reaches the laptop's loopback service through the loopback forward on the shared laptop-ssh control connection.
  - **Laptop seat → standalone.** The seat signal is the _service's own config file_ `~/.local/state/atlas-search/service.json`: only the serving host has one.
- **Provisioning / sensitivity ceilings:** the via-service client presents a per-seat bearer token — env `ATLAS_SEARCH_TOKEN`, else the JSON file at **`~/.config/atlas-search/client.json`** (a JSON object with one token string field, file mode 0600). Identities are named in the service config; the **server** clamps `--max-sensitivity` to the identity's ceiling: identity `drew-cli` → `secret-bearing`, identity `sandbox-scripts` → `personal`. The fleet verb adds no ceiling logic of its own. A seat with no client.json cannot use the service route and falls back to standalone.

## fleet search search

```
fleet search <query> [options]
fleet search search <query> [options]
```

| Flag                    | Argument             | Default                                   | Meaning                                                                                                                      |
| ----------------------- | -------------------- | ----------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| `<query>` (positional)  | text                 | required                                  | The search query (a single positional).                                                                                      |
| `--k`                   | `N`                  | `10`                                      | Final result count.                                                                                                          |
| `--pool`                | `N`                  | `100`                                     | Per-branch candidates, validated `1..=400`.                                                                                  |
| `--mode`                | `MODE`               | `hybrid`                                  | `hybrid \| fts \| vector`.                                                                                                   |
| `--kind`                | `CSV`                | —                                         | Classification kind filter.                                                                                                  |
| `--topic`               | `CSV`                | —                                         | Topic filter.                                                                                                                |
| `--project`             | `CSV`                | —                                         | Project filter.                                                                                                              |
| `--client`              | `CSV`                | —                                         | Client filter.                                                                                                               |
| `--since`               | `YYYY-MM-DD`         | —                                         | Sessions created on/after (inclusive).                                                                                       |
| `--until`               | `YYYY-MM-DD`         | —                                         | Sessions created on/before (inclusive).                                                                                      |
| `--max-sensitivity`     | `LV`                 | `personal`                                | `shareable \| personal \| local-only \| secret-bearing`. A _request_; the server clamps it to the client identity's ceiling. |
| `--group-by-session`    | —                    | off                                       | Roll chunk hits up to sessions.                                                                                              |
| `--expand`              | `N`                  | `0`                                       | Include N neighbouring chunks around each hit (validated `0..=100`).                                                         |
| `--with-message`        | —                    | off                                       | Include the parent message text.                                                                                             |
| `--no-cache`            | —                    | off                                       | Bypass service cache _reads_; the fresh result still lands in the cache.                                                     |
| `--no-wait`             | —                    | off                                       | Do not block: print the service job handle and return. (The one-shot default waits for the job.)                             |
| `--hydrate`             | `N`                  | `0`                                       | Hydrate the top N results to full content (validated ≤ 1000).                                                                |
| `--raw-rows[=branches]` | optional `=branches` | —                                         | Ungrouped rows: bare = flat fused rows; `=branches` = pre-fusion branch rows.                                                |
| `--text`                | —                    | off                                       | Human-readable output instead of JSON (default output is JSON).                                                              |
| `--model`               | `NAME`               | `Snowflake/snowflake-arctic-embed-m-v2.0` | Embedding model.                                                                                                             |
| `--embedder`            | `NAME`               | `auto`                                    | `auto \| fastembed \| st`.                                                                                                   |
| `--trunc`               | `N`                  | `0`                                       | Truncate vectors to N dims (0 = full).                                                                                       |
| `--vector-backend`      | `B`                  | `scan`                                    | `scan \| index-fill \| index-fixed`.                                                                                         |
| `--via-service`         | —                    | seat default                              | Prefer the Atlas service (job protocol). Mutually exclusive with `--standalone` (usage error).                               |
| `--standalone`          | —                    | seat default                              | Run standalone (delegate to the atlas-search artifact). Mutually exclusive with `--via-service`.                             |
| `--service-url`         | `URL`                | `http://127.0.0.1:18760`                  | Service base URL; env `ATLAS_SEARCH_URL`, then fleet.toml `[search] url`, override it.                                       |

Standalone-only engine knobs (`--backend`, `--db`) are deliberately not part of this surface.

### Behaviour

The search is hybrid FTS+vector over the Atlas corpus with the classification filters above — the subset of `atlas-search search`'s flags that translates to the wire request or to routing. Both `--flag value` and `--flag=value` spellings are accepted. Default output is JSON; `--text` renders human-readable output. With `--no-wait` on the service route, the command prints the service job handle and returns instead of blocking for the result.

### Exit codes

- `0` — ok.
- `1` — service-job/backend failure surfaced without a fallback.
- `2` — usage/validation (including `--via-service` with `--standalone`, or `--pool` outside `1..=400`).
- `3` — routing unavailable (no service and no standalone artifact).

### Example

```console
$ fleet search 'queue broker lease' --k 5 --text
```

Runs a hybrid search for "queue broker lease" and prints the top 5 hits as text.

## fleet search get-chunk

```
fleet search get-chunk <chunk-id> [--neighbors N] [--text]
```

| Flag                      | Argument | Default  | Meaning                                               |
| ------------------------- | -------- | -------- | ----------------------------------------------------- |
| `<chunk-id>` (positional) | chunk id | required | The chunk to fetch.                                   |
| `--neighbors`             | `N`      | `0`      | Include N chunks either side **in the same message**. |
| `--text`                  | —        | off      | Human-readable output instead of JSON.                |
| `-h`, `--help`            | —        | —        | Print this verb's own usage.                          |

### Behaviour

Fetches one chunk with its provenance (session, client, sensitivity). The verb parses nothing beyond answering `--help` itself: every other argument is forwarded verbatim to the standalone atlas-search artifact, so atlas-search get-chunk's own options (`--context`, `--backend`, `--db`, `--vecdb`, `--via-service`, `--service-url`) work through this verb unchanged. **`get-chunk` always runs standalone** — it is job-scoped in the service protocol, so it cannot ride the service route; when via-service was _requested_ (flag/env/config — the seat default doesn't count) it prints the loud fallback note first, then delegates to the standalone artifact.

### Exit codes

- `0` — ok.
- `1` — backend failure surfaced without a fallback.
- `2` — usage/validation.
- `3` — routing unavailable (no standalone artifact).

### Example

```console
$ fleet search get-chunk 112081 --neighbors 2 --text
```

Fetches chunk 112081 with two neighbouring chunks on each side, rendered as text.
