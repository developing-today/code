# Local D1 journal-fit PoC

From `pkgs/id/worker-poc`, run the whole smoke with:

```sh
node d1-journal-poc/smoke.mjs
```

It needs Node.js 22 or newer and `wrangler` available on `PATH`. The harness
starts its own local Wrangler Worker on a free ephemeral port, gives it a
unique temporary D1 persistence directory, and stops/removes both on exit. Its
separate config is always used with `--local`; the placeholder database ID is
not a remote database configuration. It does not use the existing Worker-PoC
Wrangler config or its Durable Object database.

The Worker stores submitted newline-terminated JSONL bytes directly as a D1
BLOB, assigning a per-world ordinal with an explicit `ORDER BY ordinal ASC`
when reading snapshots and tails. The smoke uses one synthetic world and the
current journal header version 1 (`created` and `caps_granted` entries). It
checks that a bookmark from an append is carried to a separate request and
that snapshot bytes plus a later tail are exactly the submitted JSONL bytes.

## Scope and limits

- It assumes #806's single writer per world. This is an ordinal-allocation
  fit probe, not a multi-writer conflict protocol.
- #5863's storage contract is still open; this schema is only for this PoC.
- There is no signature verification implementation here. The probe checks
  exact-byte storage and ordering only.
- A D1 bookmark is a read-consistency token, distinct from the stream cursor.
  This PoC's stream cursor is only the last returned ordinal; it is not the
  proposed production ordinal-plus-digest cursor.
- D1 documents `withSession()` sequential consistency and bookmark use across
  requests as a provider contract:
  [D1 `withSession`](https://developers.cloudflare.com/d1/worker-api/d1-database/#withsession)
  and [D1 read replication](https://developers.cloudflare.com/d1/best-practices/read-replication/).
  Local Wrangler cannot prove regional replica routing, production
  `served_by_region`/`served_by_primary`, or production replica behavior.
