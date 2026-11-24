# atlas-classify

Source: `pkgs/atlas-search/crates/atlas-core/src/bin/atlas-classify.rs`, with defaults in `pkgs/atlas-search/crates/atlas-core/src/classify.rs`, under https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/bin/atlas-classify.rs — provenance only; this chapter is complete without following it.

`atlas-classify` is the classification operating procedure tool: it sweeps a corpus database for chunks that still carry only a default or unresolved classification and writes a real classification record for each, and it manages changes to the classification schema itself (review a schema change, then apply a reviewed one). Its place in the ingest flow is described in [Ingest today: staging, classification, promotion](../concepts/ingest.md).

Two facts govern everything about this binary, and both are stated plainly here because its own help states neither:

- **Exit code 2 is the only failure code — for everything.** Usage errors _and_ runtime/classification failures all exit 2. This binary has **no exit-1 path**. (`--help` and `--version` are not failures: both print to stdout and exit 0 — see below.)
- **The usage text states the defaults.** Its final line lists every default; the tables below repeat them.

## Invocation and the strict parser

The parser is hand-rolled and strict — not clap. The whole command line is declared in the source header and parsed before any file is opened; anything unaccounted for is a usage error. The complete help text, verbatim (it is also the source header's contract):

```
usage:
  atlas-classify --db CORPUS run [--commit] [--window-days N] [--actor A] [--now TS] [--schema-version N] [--max-decisions N] [--manifest-out PATH]
  atlas-classify --db CORPUS schema-review --old-schema F --new-schema F [--reviewer-model M] [--reviewer-model-version V] [--manifest-out PATH]
  atlas-classify --db CORPUS schema-apply --review F --dispositions F [--now TS] [--manifest-out PATH]
  defaults: --window-days 90; --actor atlas-classify/remainder-sweep; --schema-version 1; --reviewer-model atlas-classify; --reviewer-model-version the crate version; --now the current time; --max-decisions unbounded
```

**`--help` / `-h`** prints the usage text to **stdout** and exits **0**, from any position in the argv; **`--version` / `-V`** prints `atlas-classify 0.1.0`, exit 0. Every other invocation shape other than the three documented actions — bare invocation, unknown flags, unknown actions, a flag given twice, a missing flag value — prints the usage text to **stderr** and exits **2**.

The global flag:

| Flag   | Argument | Default      | Meaning                                                                                                                                                                                                                       |
| ------ | -------- | ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--db` | path     | — (required) | The corpus database. Required, and must be given **exactly once** (`--db given twice` is an error). It may appear anywhere in the argv: the parser extracts it before dispatching on the first remaining token as the action. |

Counting the global `--db`, the three actions' surfaces total 17 flags; the per-action tables below list each action's own flags, with `--manifest-out` counted in each action that carries it, as it is a separate flag slot in each.

On success, all three actions print their document — run manifest, review batch, or apply report — as pretty JSON on stdout, and also write it to `--manifest-out` when that flag is given.

## `run`: the remainder sweep

```
atlas-classify --db CORPUS run [--commit] [--window-days N] [--actor A] [--now TS] [--schema-version N] [--max-decisions N] [--manifest-out PATH]
```

`run` is **a dry-run unless `--commit` is given**. Without `--commit` it enumerates only unclassified, default-state, or unresolved embeds, prints the run manifest as JSON, and writes nothing. With `--commit`, it writes one classification record per candidate into the corpus's `classification_records` table; chunks and vectors are never modified. Runs are deterministic and resumable. The manifest format is `atlas-classify-run/1`; the method is `remainder-sweep`, method version `1`.

| Flag               | Argument           | Default                                                                  | Meaning                                                                                                                        |
| ------------------ | ------------------ | ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------ |
| `--commit`         | switch             | off                                                                      | Commit the run: write one classification record per candidate. Without it, the run is a dry-run that only prints the manifest. |
| `--window-days`    | integer (i64, ≥ 0) | `90` (the `DEFAULT_WINDOW_DAYS` constant; also stated in the usage text) | The relatedness window, in days.                                                                                               |
| `--actor`          | string             | `atlas-classify/remainder-sweep` (the run configuration's default)       | The actor name recorded for the run's decisions.                                                                               |
| `--now`            | string (timestamp) | current time as ISO-8601                                                 | The run's notion of now.                                                                                                       |
| `--schema-version` | integer (u64)      | `1` (the `CURRENT_SCHEMA_VERSION` constant)                              | The classification schema version to classify under.                                                                           |
| `--max-decisions`  | integer (u64)      | none (run to completion)                                                 | Cap the run at N decisions.                                                                                                    |
| `--manifest-out`   | path               | — (none)                                                                 | Also write the manifest to this file. The manifest is always printed to stdout as pretty JSON regardless.                      |

Non-integer or negative values are exit-2 errors naming the flag, e.g. `--window-days must be an integer`, `--window-days must not be negative`.

## `schema-review`: compare two schema documents

```
atlas-classify --db CORPUS schema-review --old-schema F --new-schema F [--reviewer-model M] [--reviewer-model-version V] [--manifest-out PATH]
```

Compares two schema documents section by section — changed sections only, unless the new document declares a breaking-change event — and prints the review batch: proposals plus model/config/schema provenance, in format `atlas-classify-schema-review/1`. It writes nothing to the DB.

| Flag                       | Argument | Default                                                 | Meaning                                                                                  |
| -------------------------- | -------- | ------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `--old-schema`             | path     | — (required)                                            | The current schema document.                                                             |
| `--new-schema`             | path     | — (required)                                            | The proposed schema document.                                                            |
| `--reviewer-model`         | string   | `atlas-classify`                                        | The reviewing model name recorded in the batch's provenance.                             |
| `--reviewer-model-version` | string   | `0.1.0` (the crate version, from the build environment) | The reviewing model version recorded in the batch's provenance.                          |
| `--manifest-out`           | path     | — (none)                                                | Also write the review batch to this file; it is always printed to stdout as pretty JSON. |

## `schema-apply`: apply a reviewed schema change

```
atlas-classify --db CORPUS schema-apply --review F --dispositions F [--now TS] [--manifest-out PATH]
```

Applies a review batch's proposals. Each proposal is applied **only** with a reviewed disposition naming a reviewer and a rationale; lowerings (proposals that lower a classification) without one are refused. The report format is `atlas-classify-schema-apply/1`.

| Flag             | Argument           | Default      | Meaning                                                                            |
| ---------------- | ------------------ | ------------ | ---------------------------------------------------------------------------------- |
| `--review`       | path               | — (required) | The review batch file produced by `schema-review`.                                 |
| `--dispositions` | path               | — (required) | The dispositions file: one reviewed disposition per proposal to apply.             |
| `--now`          | string (timestamp) | current time | The apply's notion of now.                                                         |
| `--manifest-out` | path               | — (none)     | Also write the report to this file; it is always printed to stdout as pretty JSON. |

## Worked examples

Toy examples; the corpus and files are invented.

Preview a sweep of a toy corpus without writing anything (no `--commit`):

```
$ atlas-classify --db /tmp/toy-corpus.db run
{
  "format": "atlas-classify-run/1",
  "method": "remainder-sweep",
  "…": "…"
}
```

Commit the same sweep, capped at ten decisions, and keep the manifest:

```
$ atlas-classify --db /tmp/toy-corpus.db run --commit --max-decisions 10 --manifest-out /tmp/toy-manifest.json
```

Review a toy schema change, then apply it with reviewed dispositions:

```
$ atlas-classify --db /tmp/toy-corpus.db schema-review --old-schema /tmp/toy-schema-old.json --new-schema /tmp/toy-schema-new.json --manifest-out /tmp/toy-review.json
$ atlas-classify --db /tmp/toy-corpus.db schema-apply --review /tmp/toy-review.json --dispositions /tmp/toy-dispositions.json
```

Asking for help prints the usage text to stdout and exits 0:

```
$ atlas-classify --help
usage:
  atlas-classify --db CORPUS run [--commit] [--window-days N] [--actor A] [--now TS] [--schema-version N] [--max-decisions N] [--manifest-out PATH]
  …
$ echo $?
0
```

## Failure shapes

All failures print `atlas-classify: &lt;msg&gt;` (or, for usage errors, the usage text) to stderr and exit **2**. There are no other failure shapes.

- Bare invocation, an unknown action, an unknown flag, a duplicated flag, or a missing flag value: the usage text on stderr, exit 2.
- `--db` missing, or given twice (`--db given twice`): exit 2.
- A required action flag missing (e.g. `schema-review` without `--new-schema`): exit 2.
- A malformed numeric value: an error naming the flag (`--window-days must be an integer`, `--window-days must not be negative`), exit 2.
- A lowering in `schema-apply` with no reviewed disposition naming a reviewer and rationale: refused, exit 2.
- Runtime and classification failures — an unreadable corpus, a failed classification write: exit 2, exactly like a usage error. There is no way to distinguish the two classes by exit code; read the message.

## Exit codes

| Code | Meaning                                                                                                                  |
| ---- | ------------------------------------------------------------------------------------------------------------------------ |
| 0    | Success: the manifest, review batch, or apply report was printed (and written to `--manifest-out` when given).           |
| 2    | **Everything else** — usage errors, `--help`, and runtime/classification failures alike. This binary has no exit-1 path. |
