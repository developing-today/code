# Hosted Metadata durability — #16096

## Scope and acceptance state (2026-11-27)

Issue #16096's module-scope-fixture evidence predates #15456. The existing
`buildDeps` already selects `D1MetadataStore` and `D1BackfillRunStore` for
`QUEUE_D1`; `wrangler.toml` binds that to `fleet-queue` database
`c17186b0-e27f-4b0b-9429-54c1ecf2f4fb`. No second metadata adapter is needed.

- **Implemented:** the stamp, query, ingestion and backfill doors consume the
  same resolved metadata store. `fleet_metadata` retains complete record JSON,
  including arbitrary Metadata extension fields, and the extracted keys.
  The `(repo, kind, number)` key is an upsert; `binding_id` is indexed.
  Unlike #9074's aggregator schema, other filters and link relation/status
  matching run in memory, not over normalized SQL indexes.
- **Covered by equivalent durable integration tests:** authenticated issue and
  PR creates, actual SQLite connection close/reopen, fresh `buildDeps`, exact
  complete record and Metadata readback without GitHub fallback, idempotent
  backfill, and backfill-run persistence across another reopen. This exercises
  the production D1 client against SQLite through the documented binding API;
  it is not evidence from Cloudflare's live D1 service.
- **Recovery fix:** a failed first-use schema operation no longer caches a
  rejected promise forever. Tests force both schema and INSERT failure: GitHub
  create remains 201 with `metadata_inserted: false`, then the same Worker
  heals through backfill after database recovery, without a second GitHub
  create. The healed JSON survives a connection close/reopen.
- **Unverified:** live D1 row readback and deployment of this recovery fix
  (the existing adapter deployment is confirmed below). Authenticated
  create traffic must remain disabled until the readback below is recorded.
  This slice changes no auth configuration, secrets, deployment, or bindings.

## Live verification attempt and remaining release check

Two independent read-only probes agree that the deploy identity available to
this lane cannot read the production D1 database:

- At approximately 2026-11-27 11:02 UTC this worker made a **read-only**
  Cloudflare D1 API request using the ambient account/token to the bound
  database's `/query` endpoint:

```sql
SELECT name FROM sqlite_master
WHERE type='table' AND name='fleet_metadata';
```

Cloudflare returned **HTTP 401**, error code `10000`, `Authentication error`.
No row, schema, token, or deployment was changed. This does not establish
whether the table exists or which Worker version is deployed. No live
readback is claimed.

The parent lane's live probe (2026-11-27, reported into this session)
found the deployed Worker at adapter PR #15459 (`c3add690`),
release `45781053761`, version `0fd14383` at 100% with D1 bound;
public `/healthz` returned 200 with `rp_configured: false` /
`tenancy_configured: false`, and the query door returned
503 `auth_not_configured`. The lane's Cloudflare deployment GET and
read-only D1 `SELECT` with its local token/account also returned
HTTP 401 code 10000. So deployment wiring is confirmed, but there is
still **no live row readback**: no exact complete-Metadata JSON has been
read back from the production D1 database.

Release verification (use the deploy identity that can access this D1 database):

1. Keep authenticated create traffic disabled. Confirm `QUEUE_D1` resolves to
   the database above and record the current deployed version and rollback
   version with `wrangler versions list` / `wrangler deployments list`.
   Use `.github/workflows/cf-worker-versions-release.yml` for version upload and
   deploy; do not replace the production deployment with local fixtures.
2. Read `fleet_metadata` through `wrangler d1 execute fleet-queue --remote`
   (from `pkgs/fleet/hosted-worker`) and preserve the output. Use a known
   existing stamped item, if present:

   ```sh
   npx wrangler d1 execute fleet-queue --remote --command \
     "SELECT repo,kind,number,binding_id,record_json FROM fleet_metadata LIMIT 1"
   ```

   Verify `record_json.metadata` equals the complete pm-metadata JSON in that
   item's GitHub body, including provenance/signature and unknown fields.
   Merely finding a table or a row count is not an exact-JSON readback.
3. If there is no row, use an isolated, restricted canary Worker/version bound
   to this D1 database and the real App/stamp configuration. Create one canary
   item using a narrowly scoped test credential; do not enable the public
   authenticated-create door. Record response status, item URL, full stamped
   JSON, and `metadata_inserted: true`.
4. Reinstantiate the canary Worker (record both version IDs), query that item
   using the same binding and no GitHub fallback, and compare its JSON with
   the create response and a direct D1 SQL read. Run backfill and verify the
   item is unchanged, not inserted again. Preserve outputs with timestamps.
5. Only after recording these results may a separate release enable general
   authenticated-create traffic. Rollback uses the previously recorded
   Cloudflare version; retain D1 rows (do not drop tables on rollback).

The SQLite integration tests are deliberately not a substitute for steps 2–4.
This issue must not be closed as fully accepted until that live evidence exists.

## Focused verification

```sh
cd pkgs/fleet/hosted-worker
node --experimental-transform-types --test test/d1-metadata-stores.test.ts
```

The new cases extend the existing registered test file and package `npm test`
entry, so no new test venue is required. Remote Fleet command runs and their
before/after outcomes are recorded in the PR evidence.
