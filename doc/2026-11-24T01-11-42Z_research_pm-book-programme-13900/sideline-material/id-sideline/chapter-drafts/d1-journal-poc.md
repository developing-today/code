# id-d1-journal-poc: the D1 journal proof of concept

Source: https://github.com/developing-today/code/blob/main/pkgs/id/worker-poc/d1-journal-poc/wrangler.toml, https://github.com/developing-today/code/blob/main/pkgs/id/worker-poc/d1-journal-poc/README.md, https://github.com/developing-today/code/blob/main/pkgs/id/worker-poc/d1-journal-poc/worker.mjs, https://github.com/developing-today/code/blob/main/pkgs/id/worker-poc/d1-journal-poc/smoke.mjs

`id-d1-journal-poc` is a **local-only fit probe, not a service and not a production path**. It is a subfolder of `pkgs/id/worker-poc/` — the package covered in [id-worker-poc: the World Durable Object proof of concept](worker-poc.md) — with its own Wrangler configuration and its own Worker name, which is why it is covered under that exact name here. [The hosted Worker API](hosted-worker.md) and [The hosted edition](../concepts/hosted.md) mention it in passing; this chapter is its full record.

| Item | Value |
|---|---|
| Worker name | `id-d1-journal-poc` |
| Source | `pkgs/id/worker-poc/d1-journal-poc/` — entry point `worker.mjs` |
| Compatibility date | `2025-03-01` |
| Database | D1 binding `DB`, database name `id-d1-journal-poc`, database ID `00000000-0000-0000-0000-000000000001` — a **placeholder**, used only by local Wrangler with `--local` |
| Status | A completed probe with a passing local smoke. It is never deployed: its harness always supplies its own temporary persistence directory and the `--local` flag. |

## What it was for, and what it proved

The parent proof of concept keeps a world's journal in a Durable Object's SQLite storage. This probe asks the neighbouring storage question: does **Cloudflare D1** fit the same journal job — storing a world's journal entries as exact bytes, in a durable order, and reading a consistent snapshot and tail back across separate requests?

What it establishes, within its stated limits:

- Submitted newline-terminated JSONL bytes can be stored directly as a D1 BLOB and read back **byte-exact**: the smoke checks that the snapshot bytes plus a later tail are exactly the submitted JSONL bytes.
- A per-world ordinal can be allocated by the insert itself, from that world's durable tail, and returned atomically with a D1 session bookmark; a bookmark from an append, carried to a separate request in the `x-d1-bookmark` header, gives that request a consistent read.
- Reads order explicitly (`ORDER BY ordinal ASC`) rather than relying on insertion or row order.

It is an ordinal-allocation and exact-byte fit probe. Its own README is explicit about what it does **not** establish:

- It assumes #806's single writer per world. It is not a multi-writer conflict protocol.
- The #5863 storage contract was still open at its writing; this schema is only for this probe.
- There is no signature verification here: exact-byte storage and ordering only, nothing about the signed journal's semantics.
- Local Wrangler cannot prove regional replica routing, production `served_by_region` / `served_by_primary`, or production replica behaviour. The D1 `withSession()` sequential-consistency and bookmark behaviour it relies on is a provider contract, cited in its README from Cloudflare's D1 documentation.
- A D1 bookmark is a read-consistency token, **distinct from the stream cursor**. This probe's stream cursor is only the last returned ordinal; it is not the proposed production ordinal-plus-digest cursor.

## The surface

One binding, from `wrangler.toml`:

| Binding | Value |
|---|---|
| `DB` | D1 database, database name `id-d1-journal-poc`, placeholder database ID above. |

There are no routes, no Durable Objects, no secrets and no environment variables. The synthetic world is a constant in the source, `WORLD_ID = "d1-journal-poc-world"`; there is no world parameter.

Routes, from `worker.mjs`:

| Method | Path | Purpose |
|---|---|---|
| GET | `/health` | Answers the plain text `ok`. Does not touch the database or create the schema. |
| POST | `/append` | Append one journal entry (shape below). Answers `{world_id, ordinal, bookmark}`. |
| GET | `/snapshot` | Every entry for the world, in ordinal order, as one JSONL body. |
| GET | `/tail` | Entries after the `?after=` ordinal (default `0`), in ordinal order, as one JSONL body. |

Any other method/path combination is `404 {"error":"not found"}`.

`/snapshot` and `/tail` answer with content type `application/x-ndjson` and four headers that describe the returned range: `x-journal-first-ordinal`, `x-journal-last-ordinal`, `x-journal-entry-count` and `x-journal-stream-cursor`. On an empty result the first ordinal is `0` and the last ordinal — and the stream cursor — fall back to the requested `after` value for `/tail`, or `0` for `/snapshot`.

## Entries and the store

The table, created lazily on the first `/append`, `/snapshot` or `/tail`:

```text
journal_entries (
  world_id    TEXT NOT NULL,
  ordinal     INTEGER NOT NULL CHECK (ordinal > 0),
  entry_bytes BLOB NOT NULL,
  PRIMARY KEY (world_id, ordinal)
)
```

An append body must be exactly one entry: newline-terminated JSONL bytes, with no interior newline, decoding as one UTF-8 JSON object. Only two entry kinds are accepted, both from journal header version `1` (`JOURNAL_VERSION`):

| Entry | Required shape |
|---|---|
| `created` | `entry.world_id` must be the synthetic world and `entry.version` must be `1`. |
| `caps_granted` | `entry.granted` must be an array of strings. |

The append runs in a `first-primary` session: the insert computes the next ordinal as `COALESCE(MAX(ordinal), 0) + 1` for the world inside the insert statement itself, then re-queries the inserted row's ordinal under the same session and returns it with the session bookmark. That single-writer assumption (#806) is what makes the max-plus-one allocation sound here.

Failure shapes, all JSON `{"error": ...}`:

| Status | Cause |
|---|---|
| `400` | The body is not exactly one newline-terminated entry; the line is not valid UTF-8 JSON or not a JSON object; a `created` entry names another world or a version other than `1`; a `caps_granted` entry's `granted` is not a string array; the entry kind is neither of the two; or `after` on `/tail` is not a non-negative safe integer. Each refusal's message names its specific cause. |
| `404` | Any other method/path combination: `{"error":"not found"}`. |
| `500` | Anything else, including a D1 result the code cannot interpret — for example an append that returns no ordinal or no session bookmark, or a stored value that is not a byte array, an `ArrayBuffer` or a typed-array view when read back. The message is the thrown error's message. |

## Running its proof

From `pkgs/id/worker-poc/`:

```text
node d1-journal-poc/smoke.mjs
```

The harness needs Node.js 22 or newer and `wrangler` on `PATH`. It starts its own local Wrangler Worker on a free ephemeral port with a unique temporary D1 persistence directory, runs the checks — one synthetic world, the version-1 `created` and `caps_granted` entries, a bookmark carried from an append to a separate request, snapshot bytes plus a later tail compared byte-for-byte against the submitted JSONL — and stops and removes both the Worker and the directory on exit. It does not use the parent package's Wrangler configuration or its Durable Object database.

## Where its ideas landed, and what it is not

- The probe is evidence for the serverless storage question, not a component of the answer in production form. The design thread it feeds — D1 versus Turso for the serverless edition (#5681), the D1 journal fit design and spike (#5864, from #5681), and the #5863 storage contract it names as still open — is carried in this book's research and design chapters, including [Adopt the IndexedDB request-batch profile](../research/store.md) and [Design: hosted and serverless editions](../design/hosted-editions.md). [Data stores and schemas](data-stores.md) records its journal header beside the other stores: version `1`, entries `created` and `caps_granted`, and the note that a D1 bookmark is not a stream cursor.
- Nothing references it as a dependency: the hosted Worker (`pkgs/id/hosted-worker`) uses a Durable Object's SQLite storage and R2, not D1, and no NixOS module, host configuration or workflow deploys or invokes this Worker. The placeholder database ID names no real database.
- It is not a second journal implementation to build on. A reader who needs the journal's production semantics starts from the native event store and the signed-journal design records, not from this probe's `journal_entries` table.
