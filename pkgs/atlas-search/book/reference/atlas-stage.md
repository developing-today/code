# atlas-stage

Source: `pkgs/atlas-search/crates/atlas-search/src/bin/atlas-stage.rs`, with the staging DDL in `pkgs/atlas-search/schema/atlas_stage_v1.sql` and `pkgs/atlas-search/schema/stage_registry_v1.sql`, under https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-search/src/bin/atlas-stage.rs — provenance only; this chapter is complete without following it.

`atlas-stage` provisions Atlas _staging units_ and reads the _staging registry_. A staging unit is one writer's isolated staging database: artifacts are staged into it first, classified there, and only later promoted into the core corpus. The registry is the store file that records every unit, its owner and its lifecycle state. The staging schema version is 1 (`STAGE_SCHEMA_VERSION`). The place of staging in the ingest flow is described in [Ingest today: staging, classification, promotion](../concepts/ingest.md).

**Where the source header disagrees with the source, the source wins.** The binary has **seven** verbs — `provision`, `lookup`, `ingest`, `ingest-memory-lancedb`, `items`, `promote`, `units`. `promote` needs the embedding model and ONNX runtime (see below); `ingest-memory-lancedb` is limited to the `agent-memory` unit. This chapter documents the actual CLI.

## Invocation

```
atlas-stage <COMMAND>

Commands:
  provision  Create a staging DB, apply the v1 schema, and register the unit
  lookup     Print one unit's registry record as JSON
  ingest     Stage one code/config artifact file as an embedding entry
  ingest-memory-lancedb  Stage records from an agent-memory LanceDB store
  items      List the items staged in a unit's staging DB
  promote    Embed a unit's staged chunks into a new core scan shard and promote the unit to merged
  units      List units in a lifecycle state (open|closing|merged)
```

`--version` / `-V` prints `atlas-stage 0.1.0` (the crate version), exit 0 — the command definition carries a version, as `atlas-search`'s does. `-h, --help` works as usual for a clap binary.

All output is one JSON object per line on stdout. Errors print `atlas-stage: &lt;msg&gt;` on stderr.

A vocabulary warning: `--sensitivity` on this binary takes the staging **tier** vocabulary — `public|internal|secret` — which is _not_ the four-level sensitivity ladder (`shareable < personal < local-only < secret-bearing`) used by search. The two vocabularies meet at classification and promotion time, not in these flags.

## `provision`

```
atlas-stage provision --db <DB> --unit <UNIT> --owner <OWNER> --registry <REGISTRY> [--sensitivity <SENSITIVITY>]
```

Creates the staging DB, applies the v1 schema, and registers the unit as `open`. Prints the unit record as JSON: `unit_id, db_uri, owner, sensitivity, state, schema_version, created_at, updated_at`.

| Flag            | Argument                   | Default      | Meaning                                                                          |
| --------------- | -------------------------- | ------------ | -------------------------------------------------------------------------------- |
| `--db`          | path                       | — (required) | The staging database file to create.                                             |
| `--unit`        | string                     | — (required) | The staging-unit id (queue task id, session id, epic id, …).                     |
| `--owner`       | string                     | — (required) | The unit's owner (the writer whose token may write here).                        |
| `--sensitivity` | `public\|internal\|secret` | `internal`   | Physical sensitivity level — the staging tier vocabulary, not the search ladder. |
| `--registry`    | path                       | — (required) | The registry store file (created if missing).                                    |

## `lookup`

```
atlas-stage lookup --unit <UNIT> --registry <REGISTRY>
```

Prints one unit's registry record as JSON. Both flags are required, with no defaults and no help prose.

| Flag         | Argument | Default      | Meaning                         |
| ------------ | -------- | ------------ | ------------------------------- |
| `--unit`     | string   | — (required) | The staging-unit id to look up. |
| `--registry` | path     | — (required) | The registry store file.        |

An unknown unit is an error: `no staging unit "&lt;unit&gt;" in the registry`, exit 1.

## `ingest`

```
atlas-stage ingest --db <DB> --unit <UNIT> --repo <REPO> --path <PATH> --file <FILE> --commit <COMMIT> [OPTIONS]
```

Stages one code/config artifact file as an embedding entry in an already-provisioned unit. Prints `{"item_id":…, "unit":…, "source":"&lt;repo&gt;:&lt;path&gt;@&lt;commit&gt;"}`.

| Flag            | Argument                   | Default         | Meaning                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| --------------- | -------------------------- | --------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--db`          | path                       | — (required)    | The unit's staging database file (already provisioned).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `--unit`        | string                     | — (required)    | The staging-unit id the entry belongs to.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `--repo`        | string                     | — (required)    | The repository the artifact came from.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `--path`        | string                     | — (required)    | The artifact's path inside the repo.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `--file`        | path                       | — (required)    | The local file whose bytes are staged.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `--commit`      | string                     | — (required)    | The commit SHA the bytes were read at.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `--kind`        | `prose\|code\|config`      | `config`        | The entry kind.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `--language`    | string                     | `""` (see note) | The artifact language (yaml, toml, rust, …). An empty language is replaced by the staged file's extension before the entry is built, as the help states.                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `--sensitivity` | `public\|internal\|secret` | `internal`      | Staging tier vocabulary, as in `provision`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `--segments`    | path                       | — (none)        | Optional JSON file declaring the artifact's source-native classification segments: an array of `{"start": &lt;byte&gt;, "end": &lt;byte&gt;, "sensitivity": shareable\|personal\|local-only\|secret-bearing, "certain": &lt;bool, default true&gt;}` — note the segment sensitivities use the four-level ladder, not the tier vocabulary. When given, the artifact is staged classified: boundary-safe chunks plus one persisted classification record per chunk. Segment files are parsed strictly — missing or mistyped fields, or an unknown sensitivity name, are errors, never defaults. |

## `ingest-memory-lancedb`

This command reads the native agent-memory LanceDB store and stages its records in the specified staging unit. It is intentionally restricted to the `agent-memory` unit; other unit IDs are rejected. Source provenance and rebuild metadata are retained, and repeating an ingest updates changed records without duplicating unchanged records.

Consult `atlas-stage ingest-memory-lancedb --help` for the exact required flags and defaults; the CLI is the source of truth for the store, database and registry paths.

## `items`

```
atlas-stage items --db <DB> [--unit <UNIT>]
```

Lists the items staged in a unit's staging DB, one JSON object per item: `id, unit_id, kind, source, sensitivity, chunks, meta`.

| Flag     | Argument | Default                 | Meaning                            |
| -------- | -------- | ----------------------- | ---------------------------------- |
| `--db`   | path     | — (required)            | The staging database file to read. |
| `--unit` | string   | — (all units in the DB) | Only this unit's items.            |

## `promote`

```
atlas-stage promote --registry <REGISTRY> --unit <UNIT> --shard-dir <SHARD_DIR> [--topic <TOPIC>]
```

Embeds the unit's staged chunks with the mandated model (the default Snowflake model, `Snowflake/snowflake-arctic-embed-m-v2.0`) into a new core scan shard — the aggregator embeds at promotion time — stamps embedding-model provenance on the shard, and promotes the unit to `merged` through the registry gate. Prints `{"unit":…, "shard":…, "shard_path":…, "items":…, "chunks":…, "dims":…}`.

| Flag          | Argument | Default      | Meaning                                                                 |
| ------------- | -------- | ------------ | ----------------------------------------------------------------------- |
| `--registry`  | path     | — (required) | The registry store file.                                                |
| `--unit`      | string   | — (required) | The staging-unit id to promote.                                         |
| `--shard-dir` | path     | — (required) | Directory the new core scan shard is written into (created if missing). |
| `--topic`     | string   | `codeconfig` | Shard topic for the promoted entries.                                   |

`promote` requires the embedding model and the ONNX runtime, resolved exactly as `atlas-search` resolves them (the `ATLAS_MODEL_DIR` / `ATLAS_ORT_LIB` variables; see [Configuration and environment](configuration.md)) — plus one extra fallback the search binary does not have: the ORT library at `~/atlas/v3-venv/lib/python3.14/site-packages/onnxruntime/capi/libonnxruntime.so.1.31.1`. A missing model or runtime is a loud exit-1 error, never a silent skip. This is the sense in which the source header's "all offline" framing is wrong: `promote` is a model-dependent verb.

## `units`

```
atlas-stage units --registry <REGISTRY> [--state <STATE>]
```

Lists units in a lifecycle state, one unit-record JSON object per line.

| Flag         | Argument                | Default      | Meaning                                                                                                              |
| ------------ | ----------------------- | ------------ | -------------------------------------------------------------------------------------------------------------------- |
| `--registry` | path                    | — (required) | The registry store file.                                                                                             |
| `--state`    | `open\|closing\|merged` | `open`       | The lifecycle state to list. An unknown value is an error: `unknown state "…" (want open\|closing\|merged)`, exit 1. |

## Worked example

Toy example; all names and paths are invented.

```
$ atlas-stage provision --db /tmp/toy-unit.db --unit toy-1 --owner toy-writer --registry /tmp/toy-registry.db
{"unit_id":"toy-1","db_uri":"/tmp/toy-unit.db","owner":"toy-writer","sensitivity":"internal","state":"open","schema_version":1,"created_at":"…","updated_at":"…"}

$ atlas-stage ingest --db /tmp/toy-unit.db --unit toy-1 --repo toy/repo --path config/toy.yaml --file /tmp/toy.yaml --commit 0000000000000000000000000000000000000000
{"item_id":1,"unit":"toy-1","source":"toy/repo:config/toy.yaml@0000000000000000000000000000000000000000"}

$ atlas-stage items --db /tmp/toy-unit.db --unit toy-1
{"id":1,"unit_id":"toy-1","kind":"config","source":"toy/repo:config/toy.yaml@0000000000000000000000000000000000000000","sensitivity":"internal","chunks":1,"meta":{}}

$ atlas-stage promote --registry /tmp/toy-registry.db --unit toy-1 --shard-dir /tmp/toy-shards
{"unit":"toy-1","shard":"…","shard_path":"/tmp/toy-shards/…","items":1,"chunks":1,"dims":768}

$ atlas-stage units --registry /tmp/toy-registry.db --state merged
{"unit_id":"toy-1","db_uri":"/tmp/toy-unit.db","owner":"toy-writer","sensitivity":"internal","state":"merged","schema_version":1,"created_at":"…","updated_at":"…"}
```

## Failure shapes

- Missing required flags or an unknown verb: a clap usage error, exit 2 — e.g. bare `atlas-stage lookup` fails on its required arguments.
- Unknown unit in `lookup`: `atlas-stage: no staging unit "&lt;unit&gt;" in the registry`, exit 1.
- Unknown sensitivity tier or unknown unit state: `atlas-stage: &lt;msg&gt;`, exit 1.
- A malformed `--segments` file (missing/mistyped field, unknown sensitivity name): an error, exit 1 — never a defaulted segment.
- `promote` with the model or ONNX runtime missing: a loud error, exit 1.
- Registry or staging-DB errors, file read failures: `atlas-stage: &lt;msg&gt;`, exit 1.

## Exit codes

| Code | Meaning                                                                                                                  |
| ---- | ------------------------------------------------------------------------------------------------------------------------ |
| 0    | Success.                                                                                                                 |
| 1    | **All** runtime errors: unknown sensitivity/state, missing unit, file reads, embedder/model absence, registry/DB errors. |
| 2    | Clap usage errors only: unknown verb, missing required flags.                                                            |
