# Survey dump — id-d1-journal-poc (`pkgs/id/worker-poc/d1-journal-poc`)

Files: `wrangler.toml`, `README.md`, `worker.mjs`, `smoke.mjs`.

## wrangler.toml (verbatim)

```toml
name = "id-d1-journal-poc"
main = "worker.mjs"
compatibility_date = "2025-03-01"

# This placeholder ID is used only by local Wrangler with --local. The smoke
# always supplies its own temporary --persist-to directory and never deploys.
[[d1_databases]]
binding = "DB"
database_name = "id-d1-journal-poc"
database_id = "00000000-0000-0000-0000-000000000001"
```

No routes, no Durable Objects, no vars, no secrets.

## README.md (read in full)

- Title: "Local D1 journal-fit PoC".
- Run: from `pkgs/id/worker-poc`, `node d1-journal-poc/smoke.mjs`. Needs
  Node.js 22+ and `wrangler` on PATH. The harness starts its own local
  Wrangler Worker on a free ephemeral port, gives it a unique temporary D1
  persistence directory, and stops/removes both on exit. Its separate config
  is always used with `--local`; the placeholder database ID is not a remote
  database configuration. It does not use the existing Worker-PoC Wrangler
  config or its Durable Object database.
- What it does: stores submitted newline-terminated JSONL bytes directly as
  a D1 BLOB, assigning a per-world ordinal with an explicit
  `ORDER BY ordinal ASC` when reading snapshots and tails. The smoke uses one
  synthetic world and the current journal header version 1 (`created` and
  `caps_granted` entries). It checks that a bookmark from an append is
  carried to a separate request and that snapshot bytes plus a later tail are
  exactly the submitted JSONL bytes.
- Scope and limits (verbatim substance):
  - Assumes #806's single writer per world. An ordinal-allocation fit probe,
    not a multi-writer conflict protocol.
  - #5863's storage contract is still open; this schema is only for this PoC.
  - No signature verification implementation. The probe checks exact-byte
    storage and ordering only.
  - A D1 bookmark is a read-consistency token, distinct from the stream
    cursor. This PoC's stream cursor is only the last returned ordinal; it is
    not the proposed production ordinal-plus-digest cursor.
  - D1 documents `withSession()` sequential consistency and bookmark use
    across requests as a provider contract (links to Cloudflare D1 docs:
    worker-api/d1-database/#withsession and best-practices/read-replication).
    Local Wrangler cannot prove regional replica routing, production
    `served_by_region`/`served_by_primary`, or production replica behaviour.

## worker.mjs (read in full)

Constants: `WORLD_ID = "d1-journal-poc-world"`, `JOURNAL_VERSION = 1`.
Schema (created lazily via a memoised `ensureSchema` promise, reset on
failure):

```sql
CREATE TABLE IF NOT EXISTS journal_entries (
  world_id TEXT NOT NULL,
  ordinal INTEGER NOT NULL CHECK (ordinal > 0),
  entry_bytes BLOB NOT NULL,
  PRIMARY KEY (world_id, ordinal)
)
```

Routes (single fetch handler):
- `GET /health` → plain `new Response("ok")`. Returns BEFORE the try block
  and before ensureSchema — does not touch the DB.
- `POST /append`:
  - ensureSchema; `parseEntryBytes(new Uint8Array(await request.arrayBuffer()))`.
  - Entry validation: length ≥ 2 and last byte 0x0a (newline-terminated),
    else 400 "expected one newline-terminated JSONL entry"; any interior
    0x0a → 400 "expected exactly one JSONL entry per append"; UTF-8 fatal
    decode + JSON.parse of the line, else 400 "entry must be valid UTF-8
    JSON"; must be a non-array object, else 400 "entry must be a JSON
    object"; `entry.entry === "created"` requires `world_id === WORLD_ID`
    and `version === 1`, else 400 "created entry must name the synthetic
    world and journal version 1"; `entry.entry === "caps_granted"` requires
    `granted` to be an array of strings, else 400 "caps_granted entry must
    contain a string array"; any other kind → 400 "POC accepts only created
    and caps_granted entries".
  - Insert in a `first-primary` session:
    `INSERT INTO journal_entries (world_id, ordinal, entry_bytes) VALUES
    (?, (SELECT COALESCE(MAX(ordinal), 0) + 1 FROM journal_entries WHERE
    world_id = ?), ?)` — comment cites #806's one-writer-per-world
    assumption; the INSERT allocates the next ordinal from the durable tail
    and returns it atomically.
  - Re-queries `SELECT ordinal … WHERE world_id = ? AND rowid = ?` with
    `inserted.meta.last_row_id` under the same session; takes the session
    bookmark (`session.getBookmark()`, string or `.bookmark`); if ordinal or
    a non-empty bookmark is missing → throw Error("D1 append did not return
    an ordinal and session bookmark") → 500.
  - Answers JSON `{world_id: WORLD_ID, ordinal, bookmark}`.
- `GET /snapshot`: ensureSchema; session from `sessionFor` (request header
  `x-d1-bookmark` → `db.withSession(bookmark)`, else `db.withSession()`);
  all rows for the world `ORDER BY ordinal ASC`; `jsonlResponse(rows)`.
- `GET /tail`: ensureSchema; `after = parseAfter(url)` (`?after=` default
  "0"; must be a non-negative safe integer, else 400 "after must be a
  non-negative safe integer"); rows with `ordinal > after` in ordinal order;
  `jsonlResponse(rows, after)`.
- Anything else → `404 {"error":"not found"}` (JSON).
- Error handling: `HttpError` → its status; anything else → 500; body
  `{"error": <message>}` JSON.

`jsonlResponse(rows, emptyCursor = 0)`: concatenates the BLOB bytes
(`blobBytes` accepts ArrayBuffer, ArrayBuffer views, or — because local
Wrangler's D1 returns BLOBs as number arrays — an array of integers 0..255,
else throws "D1 returned a non-BLOB journal value"); content type
`application/x-ndjson; charset=utf-8`; headers `x-journal-first-ordinal`
(first row's ordinal or 0), `x-journal-last-ordinal` (last row's ordinal, or
`emptyCursor` when no rows), `x-journal-entry-count`,
`x-journal-stream-cursor` (= last ordinal; in-code comment: "This POC's
stream cursor is an ordinal; it is not a D1 bookmark.").

## Design-thread references found in the book (grep)

- research/current-issue-map.md + research/issue-map.md: #5681 "design(id):
  serverless edition storage — D1 vs Turso vs both for id on Cloudflare
  Workers" (next proof: compare D1 and Turso operation semantics for the
  actual Worker journal workload); #5864 "id: design: D1 journal fit (design
  + spike) (from #5681)" (next proof: compare durable ordering, checkpoint
  and crash behavior on the D1 journal path).
- research/design-review.md: #5681 recorded decision — Drew chose both
  engines with roles, Turso preferred and D1 first-class; the signed journal
  owns logical recovery.
- research/store.md ("Adopt the IndexedDB request-batch profile"): #5681
  retains D1 in its assigned roles; the #5863 serverless journal/
  materialization contract defines signed append, fork refusal, range
  completeness, watermarks, snapshot/delta rebuild — passing the record
  profile establishes none of those higher semantics on its own.
- research/archive-review.md: design record
  `doc/2026-11-17_decision_id5864_d1_journal_fit.md` — "Design: D1 journal
  fit — ordering, sessions, and replication against the signed journal",
  child decision record under #5864; instruction to carry the #6350 spike
  limitations into a test matrix before generalizing D1 guarantees.
- design/hosted-editions.md: lists the same 2026-11-17 D1 journal fit record
  among the hosted/serverless design records.
- design/roadmap-and-issue-map.md design-record index: row 376,
  `2026-11-17_decision_id5864_d1_journal_fit.md` → Design: hosted and
  serverless editions.
- reference/data-stores.md: row "d1-journal-poc journal header | Version 1;
  entries include created and caps_granted. A D1 bookmark is not a stream
  cursor."
- Hosted worker check: `pkgs/id/hosted-worker` uses Durable Object SQLite +
  R2; no D1 binding — the probe is not a dependency of it (grep for D1 in
  the hosted-worker package/docs found no D1 binding).
