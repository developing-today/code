# atlas-sweep

Source: [pkgs/atlas-search/crates/atlas-core/src/bin/atlas-sweep.rs](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/bin/atlas-sweep.rs) and [pkgs/atlas-search/crates/atlas-core/src/sweep.rs](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/sweep.rs) (issue #7969).

`atlas-sweep` is the operator tool for the ingestion sweep: the periodic pass that walks a source class on a host, filters the candidates it finds, applies guardrails, and records what it did. The binary exposes the complete operator set for that core — pausing and resuming classes, running one sweep from a request file, managing the dead-letter store (items that failed repeatedly), and reading run reports. It creates and enables **no schedule**: cadence is an operator/platform decision, gated on Drew's go-ahead on #7969. State lives in a state directory the operator names on every invocation.

`--help` / `-h` prints the usage text to stdout, exit 0, and `--version` / `-V` prints `atlas-sweep 0.1.0`; the complete surface is otherwise in the source header, and any other unrecognised first token is an `unknown action` error.

## Invocation

```
atlas-sweep --state-dir DIR pause-class --class CLASS --host HOST
atlas-sweep --state-dir DIR resume-class --class CLASS --host HOST
atlas-sweep --state-dir DIR run-once REQUEST.json
atlas-sweep --state-dir DIR dead-letter list
atlas-sweep --state-dir DIR dead-letter retry REF
atlas-sweep --state-dir DIR dead-letter drop REF
atlas-sweep --state-dir DIR report [--run RUN_ID]
```

Those are all seven command forms. `--state-dir` is global and required; it is extracted from anywhere in the argument list before the action is read.

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `--state-dir` | directory path | required, no default | The sweep state directory (lock, pause state, dead-letter store, report rows). Missing: `atlas-sweep: missing --state-dir DIR`, exit 2. |
| `--class` | source class name | required for `pause-class`/`resume-class` | One of `boot-records`, `session-logs`, `chat-logs`, `statistics`, `reports`, `code-repo`. Unknown: `unknown source class "…"`, exit 2. |
| `--host` | host name | required for `pause-class`/`resume-class` | The host half of the class/host pair being paused or resumed. |
| `--run` | run id | — (whole cycle report) | For `report` only: print that one run's row instead of the whole cycle report. |
| `REQUEST.json` (positional) | path | required for `run-once` | An `atlas_core::sweep::RunRequest` JSON file (shape below). |
| `REF` (positional) | dead-letter reference | required for `dead-letter retry`/`drop` | The dead-letter entry to retry or drop. |

## Behaviour

### Pausing and resuming a class

`pause-class` and `resume-class` act on a (class, host) pair, identified by its `class_host_key`. Each prints one confirmation line, `pause-class &lt;class&gt;@&lt;host&gt;` or `resume-class &lt;class&gt;@&lt;host&gt;`. A run for a paused class ends with status `PausedClass`. Missing `--class` or `--host` is `missing --class` / `missing --host`, exit 2; any unknown extra argument is `unknown argument "…"`, exit 2.

### Running one sweep: `run-once`

`run-once` reads and parses the request file, then runs one sweep over its candidates. The `RunRequest` JSON carries: `run_id`, `batch_id`, `source_class` (a class name from the table above), `host`, `at` (ISO-8601), `now_unix` (lock freshness), `candidates` (an array of candidate facts: `source_ref`, `bytes`, an optional `content_address`, `secret_tier`, `terminal`, `has_substance`, `bot_or_heartbeat`, `allowlisted`, `final_version`, `partial_write`), `item_failures` (default `[]`), `measurements`, `envelope`, `embed_usd` (default 0), and an optional `embed_usd_median`.

Each candidate is filtered on its facts before any work is spent on it, and guardrails over the run's measurements can pause the run. Constants from the core: candidates over 256 MiB are rejected as oversize (`OVERSIZE_BYTES`); the run lock's time-to-live is 2 hours; an item is dead-lettered after 3 strikes; reports keep at most 100 rows. Run statuses are `Completed`, `OverlapSkip`, `PausedClass`, `Aborted`, and `GuardrailPaused`.

On success the outcome is printed as one JSON document: `{"status":…, "records":…, "report":…, "alerts":…}`. One overlap shape is **not** a failure: if a live run already holds the lock, `run-once` prints the bare line `overlap-skip` and exits 0.

**The secret-content holdout.** A candidate whose facts mark it as secret-tier is held out of the sweep: secret content is not embedded by the ordinary route. Secret candidates stay held out until the local embedder of #6059 — the secret-tier local embedder, designed so secret material is embedded locally with no egress — lands. Until then the sweep processes the non-secret candidates and leaves the secret ones for that route.

### The dead-letter store

Items that fail repeatedly (3 strikes) land in the dead-letter store, keyed by reference (`REF`):

- `dead-letter list` prints the stored refs, one per line. An empty store prints nothing and exits 0.
- `dead-letter retry &lt;REF&gt;` returns an entry to the sweep. Silent on success.
- `dead-letter drop &lt;REF&gt;` discards an entry. Silent on success.
- `dead-letter` with anything else — or with nothing — is `atlas-sweep: dead-letter needs list|retry REF|drop REF`, exit 2. `retry`/`drop` without a ref are `dead-letter retry needs a REF` / `dead-letter drop needs a REF`, exit 2.

### Reports

`report` with no `--run` prints the whole cycle report as one JSON array; an empty store prints `[]`, exit 0. `report --run &lt;RUN_ID&gt;` prints that run's row as one JSON object; an unknown id is `no report row for run &lt;id&gt;`, exit 2, and `--run` without a value is `report --run needs a RUN_ID`, exit 2.

See [Ingest today: staging, classification, promotion](../concepts/ingest.md) for where sweeps sit in the ingest pipeline.

## Worked example

Pausing one class on one host, checking the dead-letter store, and reading the cycle report (toy names throughout):

```
$ atlas-sweep --state-dir /tmp/toy-sweep-state pause-class --class chat-logs --host toy-host
pause-class chat-logs@toy-host

$ atlas-sweep --state-dir /tmp/toy-sweep-state dead-letter list
$

$ atlas-sweep --state-dir /tmp/toy-sweep-state report
[]
```

The empty `dead-letter list` output and the `[]` cycle report are both successes (exit 0): the store is empty, not broken.

## Failure shapes

Every failure in this binary goes through one path: `atlas-sweep: &lt;message&gt;` on stderr, exit 2. That includes missing `--state-dir`, a missing action (`missing action (pause-class|resume-class|run-once|dead-letter|report)`), an unknown action, an unknown source class, missing `--class`/`--host`/`REF`/`RUN_ID` values, an unreadable or unparseable `REQUEST.json`, and I/O and core errors from inside a run. There is no exit-1 path.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success — including an empty dead-letter list, an empty (`[]`) cycle report, and a `run-once` that printed `overlap-skip`. |
| 2 | Everything else: every usage error and every runtime failure. |
