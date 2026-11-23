# atlas-search: serve, init, fetch-model, traces and feedback

Source: `pkgs/atlas-search/crates/atlas-search/src/main.rs`, `pkgs/atlas-search/crates/atlas-search/src/server.rs`, `pkgs/atlas-search/crates/atlas-search/src/service.rs` and `pkgs/atlas-search/crates/atlas-search/src/fetch_model.rs` under https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-search/src/main.rs — provenance only; this chapter is complete without following it.

This chapter covers the non-query verbs of `atlas-search`: `serve` (run the service), `init` (generate its config), `fetch-model` (provision the embedding model), the five `traces` sub-verbs (the query-trace corpus) and `feedback` (thumbs votes on served results), plus `--version`. The query verbs are in [atlas-search: search and get-chunk](atlas-search.md).

## Invocation

```
atlas-search --version
atlas-search serve [OPTIONS]
atlas-search init [OPTIONS]
atlas-search fetch-model
atlas-search traces <COMMAND>
atlas-search feedback [OPTIONS] --query <QUERY>
```

`-V, --version` prints `atlas-search 0.1.0` and exits 0. `-h, --help` prints help. An unknown subcommand is a clap error, exit 2.

## `serve`: the Atlas service

```
atlas-search serve [OPTIONS]
```

`serve` runs the Atlas service: a loopback HTTP job protocol on port **18760** by default. Clients submit a search as a *job* rather than holding a request open: the submit returns a job handle, the caller waits for the full fused result (the one-shot CLI implies waiting; `--no-wait` returns the handle plus an empty first page instead), and completed jobs are retained for a TTL so their pages and chunk reads can be fetched by job scope — which is why `get-chunk` has no standalone service route of its own (see [atlas-search: search and get-chunk](atlas-search.md)). Callers authenticate with a per-client bearer token; the service clamps each caller's requested `--max-sensitivity` to that client's ceiling cap from the config's client table. The wire contract in full is [The service HTTP API](service-api.md); running the service in production is covered in [Running the service and the Worker](../operations/services.md).

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `--config` | path | `~/.local/state/atlas-search/service.json` | Service config path. Environment variable: `ATLAS_SEARCH_SERVICE_CONFIG` (a clap env binding on the flag; a set-but-empty value counts as absent, as the previous manual lookup treated it). |
| `--listen` | address | from the config (`127.0.0.1:18760`) | Listen address override. Loopback only. |
| `--job-ttl-ms` | integer (milliseconds) | `600000` | Completed-job TTL override: how long a finished job is retained. |
| `--hard-ttl-ms` | integer (milliseconds) | `3600000` | Submit hard-cap override: the longest a submitted job may run. |

Behaviour:

- `serve` always uses the Turso backend and the default model/embedder; there are no backend or model flags on this verb.
- A `shards` spec in the service config enables serve-mode shard fan-out, with periodic re-discovery of the shard set (default refresh 60 s, `DEFAULT_SHARD_REFRESH_SECONDS`).
- Config-load failures — missing config, config not mode 0600, empty client table, duplicate client names, a bad `ceiling_cap` — exit 2, with `run atlas-search init first` guidance where applicable. Runtime serve failures (for example, the port is already bound) exit 1.

## `init`: generate the service config

```
atlas-search init [OPTIONS]
```

`init` generates the service config (mode 0600) with the **v1 client table**: one named identity per expected caller, each with a freshly generated token and a ceiling cap — the highest sensitivity that identity may request (token values are secrets: they are generated into the config and never printed; this book names the identities and caps only).

| Client | Ceiling cap |
|---|---|
| `drew-cli` | `secret-bearing` |
| `openchamber` | `personal` |
| `fork-server` | `personal` |
| `sandbox-scripts` | `personal` |

The config document body is `{"listen": "127.0.0.1:18760", "vec_table": &lt;default vec table&gt;, "clients": [...]}` — the default vec table is `vec_snowflake_m_v2_768_f32`.

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `--config` | path | `~/.local/state/atlas-search/service.json` | Service config path. Environment variable: `ATLAS_SEARCH_SERVICE_CONFIG` (a clap env binding, as on `serve`). |
| `--force` | switch | off | Replace an existing config. This **rotates every client token**: all existing callers must be re-provisioned with the new tokens. |

Without `--force`, an existing config is an exit-2 error. On success the output prints the config path and each client's cap — never token values.

## `fetch-model`: provision the pinned model

```
atlas-search fetch-model
```

`fetch-model` has no options (only `-h, --help`). It downloads and verifies the pinned embedding model into the state dir, at `~/.local/state/atlas-search/model/&lt;rev&gt;/`:

| Fact | Value |
|---|---|
| Repository | `Snowflake/snowflake-arctic-embed-m-v2.0` |
| Revision | `95c2741480856aa9666782eb4afe11959938017f` |
| Files | `tokenizer.json` and `onnx/model.onnx` |
| Verification | Each file's sha256 is checked against the pinned hash (`tokenizer.json` begins `f1cc44ad…`, `onnx/model.onnx` begins `c0c53d7f…`; hashes are named here in abbreviated form only). |

An already-verifying snapshot is a no-op. A hash mismatch refuses the install. Failures exit 2. Model and runtime resolution at search time (including the `ATLAS_MODEL_DIR` and `ATLAS_ORT_LIB` variables) is covered in [Configuration and environment](configuration.md).

## `traces`: the statistics-and-reports corpus

```
atlas-search traces <COMMAND>
```

Every executed standalone search writes an `atlas-query-trace/v1` record (see [atlas-search: search and get-chunk](atlas-search.md)). The `traces` verbs ingest those records as a statistics-and-reports corpus and run retention and reporting over the store. Bare `atlas-search traces` with no subcommand is a clap error, exit 2.

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `--dir` | path | the trace sink's default dir, `~/.local/state/atlas-search/traces` | Traces directory. Present on all five sub-verbs. |
| `--today` | `YYYY-MM-DD` | today, UTC | The day the pass is computed for (retention, stats, report, normalise). Strictly calendar-validated. |
| `--feedback` | path | — (none) | A feedback JSONL file naming referenced query ids. **Repeatable** (retention, stats, report). |
| `--query` | string | — (none) | `ingest` only: only print documents matching this corpus query. |
| `--window-days` | integer | `7` | `stats` only: rolling window length in days. |
| `--feedback-dir` | path | the feedback sink's default dir | `normalise` only: the raw feedback store to read. |
| `--out-dir` | path | `&lt;feedback store&gt;/normalised` | `normalise` only: where the normalised output is written. |

The five sub-verbs:

| Sub-verb | Behaviour |
|---|---|
| `ingest` | Ingest trace JSONL into the statistics-and-reports corpus. With `--query`, only print the documents matching that corpus query. |
| `retention` | Run the 180-day hot retention pass: move older traces to the cold tier (directory name `cold`) and write a compaction aggregate (schema `atlas-trace-compaction/v1`). Takes `--dir`, `--today`, `--feedback`. |
| `stats` | Daily batch aggregation over stored traces, over a rolling window ending at `--today` (`--window-days`, default 7). Takes `--dir`, `--today`, `--window-days`, `--feedback`. |
| `report` | Weekly statistics report into the statistics-and-reports source set. Takes `--dir`, `--today`, `--feedback`. See the undocumented files below. |
| `normalise` | Agent normalisation pass over the raw feedback store, using the traces directory for query requester identities. Takes `--dir`, `--feedback-dir`, `--out-dir`, `--today`. |

**The report files.** `traces report` writes more than its one-line summary suggests: the verb writes the report JSON file **and a Markdown file**, `atlas-statistics-report-&lt;day&gt;.md`, into the traces dir, and reports both paths in its JSON output as `report_file` and `markdown_file`. The help text names both files.

`--today` values are strictly calendar-validated in `retention`, `stats`, `report` and `normalise`: an invalid date is exit 2, `--today must be a valid YYYY-MM-DD date, got &lt;value&gt;`. The retention horizon is 180 days (`HOT_RETENTION_DAYS`).

## `feedback`: thumbs votes on served results

```
atlas-search feedback [OPTIONS] --query <QUERY>
```

`feedback` casts a thumbs vote on a served result, or retrieves the feedback recorded for a query. Records use the `atlas-feedback/v1` schema and land in the local feedback store (default `~/.local/state/atlas-search/feedback`), which the `traces` verbs above consume — the feedback files passed with `--feedback` to `retention`, `stats` and `report`, and the store `normalise` reads. Feedback is recorded data only: no ranking consumer reads it today (see [Evaluation: golden sets and gates](../concepts/evaluation.md)).

`--query &lt;QUERY&gt;` is required: it is the query id the search response envelope carried.

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `--query` | string | — (required) | The query_id the search response envelope carried. |
| `--chunk` | integer | — (none) | The result's chunk_id; omit for a per-query vote. |
| `--up` | switch | off | Thumbs up. Conflicts with `--down`. |
| `--down` | switch | off | Thumbs down. Conflicts with `--up`. |
| `--comment` | string | — (none) | Optional comment, verbatim; `@file` reads the comment from a file. Comment size cap 256 KiB (`MAX_COMMENT_BYTES`). |
| `--opened` | switch | off | The result was opened/expanded before this vote. |
| `--requester-kind` | string | `user` | Requester kind: `drew`, `agent`, `user` or `anonymous` (the `REQUESTER_KINDS` constant). |
| `--requester-id` | string | `cli` | Stable requester id. |
| `--get` | switch | off | Retrieve the feedback for `--query` (and `--chunk`, when given) instead of casting a vote. |
| `--dir` | path | the feedback sink's default dir, `~/.local/state/atlas-search/feedback` | Feedback store directory. |
| `--via-service` | switch | off | Go through a running service instead of the local store. |
| `--service-url` | URL | the default service URL | Service base URL for `--via-service`. Environment variable: `ATLAS_SEARCH_URL`. |

**The vote-shape rule.** Without `--get`, exactly one of `--up` / `--down` is required — the help states the rule, and it is enforced at runtime: casting with neither is exit 2, `atlas-search: feedback needs exactly one of --up / --down (or --get to retrieve)`. Passing both is a clap conflict error, also exit 2. A `@file` comment whose file cannot be read is exit 1.

## Worked examples

Toy examples; paths and days are illustrative.

Provision a fresh service — config first, then the model, then serve:

```
$ atlas-search init
$ atlas-search fetch-model
$ atlas-search serve
```

Rotate every client token (existing callers must be re-provisioned afterwards):

```
$ atlas-search init --force
```

Run the retention pass as of a stated day, against a toy traces dir:

```
$ atlas-search traces retention --dir /tmp/toy-traces --today 2026-11-23
```

Produce the weekly report and its Markdown rendering:

```
$ atlas-search traces report --dir /tmp/toy-traces --today 2026-11-23
# writes the report JSON and /tmp/toy-traces/atlas-statistics-report-2026-11-23.md
```

Cast a thumbs-down vote on one chunk of a served result, then read the feedback back:

```
$ atlas-search feedback --query q-toy-1 --chunk 42 --down --requester-kind agent --requester-id cli
$ atlas-search feedback --query q-toy-1 --chunk 42 --get
```

Check the version:

```
$ atlas-search --version
atlas-search 0.1.0
```

## Failure shapes

- `serve` before `init`: a config-load error, exit 2, with `run atlas-search init first` guidance.
- `serve` with a config that is not mode 0600, has an empty client table, duplicate client names, or a bad `ceiling_cap`: exit 2.
- `serve` runtime failure (for example the loopback port is already in use): exit 1.
- `init` when a config already exists and `--force` is absent: exit 2.
- `fetch-model` download or verification failure, including a hash mismatch (which refuses the install): exit 2.
- `traces` with no subcommand: clap error, exit 2.
- `traces … --today` with an invalid date: `atlas-search: --today must be a valid YYYY-MM-DD date, got &lt;value&gt;` shape, exit 2.
- `feedback` with neither `--up` nor `--down` (and no `--get`): `atlas-search: feedback needs exactly one of --up / --down (or --get to retrieve)`, exit 2; with both: clap conflict, exit 2.
- `feedback --comment @file` where the file cannot be read: exit 1.

## Exit codes

| Code | Meaning on these verbs |
|---|---|
| 0 | Success. `--version` also exits 0. |
| 1 | `serve` runtime failures; a `feedback` `@file` comment that cannot be read. |
| 2 | Usage errors; `serve` config-load failures; `init` failures (including an existing config without `--force`); `fetch-model` failures; `traces` validation failures such as an invalid `--today`; `feedback` vote-shape errors (neither or both of `--up` / `--down`). |
