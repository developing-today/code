# cold-store-reaper

Archive files into Google Drive as a **cold, immutable store**, and restore
individual objects back out. Drive is never a CDN and never serves queries —
this tool is the write/read path for the archive tier described in the
project design (`~/workspace/google-drive-cold-store/DESIGN.md` in the
operator's notes; D1 database `cold-store-manifest` holds the hosted copy of
the same manifest schema).

## How it works

- **Bundles.** `pack` takes files and builds one bundle: each file is
  compressed as its own zstd frame (via the `zstd` CLI), and the frames are
  concatenated. The bundle is named `b_<first 16 hex of its sha256>.csb` and
  is immutable — a changed object set produces a new bundle, never an edit.
- **Manifest.** A local SQLite database (`schema.sql`, verbatim the canonical
  cold-store schema) records objects (sha256 of the _original_ bytes),
  bundles, and members with the byte offset/length of each object's frame
  inside its bundle, plus a `transfer_ledger` for the daily budget checks.
- **Upload.** `upload` sends a bundle to Drive with a resumable, chunked
  upload, then reads back Drive's `md5Checksum` + size and compares them to
  the local values. Only on a match is the bundle marked `verified`, its
  objects marked `cold`, and a ledger row written.
- **Fetch.** `fetch <sha256> <out>` looks the object up in the manifest,
  Range-downloads exactly its frame bytes from Drive, decompresses the one
  frame, and verifies the sha256 of the result before writing it out.

## The drive.file consequence (read this first)

The OAuth app ("Cold Store Reaper", in the operator's Google Cloud project)
holds **only** the `https://www.googleapis.com/auth/drive.file` scope. It can
see _nothing_ in Drive except files and folders it creates itself (or that
the user explicitly opens with it).

Therefore the tool **creates its own root folder** — `Cold Store Reaper` in
My Drive, with `bundles/`, `manifests/`, and `db-backups/` under it — on
first run (`ensure-folders`), and remembers the folder IDs in `state.json`.
Folder trees created by any other app (including trees a human made in the
Drive web UI for "the same" cold store) are **invisible to this app** and
must never be wired in by ID. If `state.json` is lost, `ensure-folders`
finds the existing tree again by name — it does not duplicate it.

## Credentials

No real credential values live in this repo, ever. The operator keeps the
OAuth client secret and the refresh token in **Bitwarden** and materializes
them into a local file only on the machine that runs the reaper:

```
$COLD_STORE_HOME/            # default: ~/.config/cold-store/
  credentials.json           # mode 600: {client_id, client_secret, refresh_token}
  state.json                 # mode 600: folder IDs + D1 pointers — no tokens
                             # (access tokens are kept in memory only)
  manifest.sqlite            # local manifest (schema.sql)
  bundles/                   # locally packed .csb bundles awaiting upload
```

One-time setup on a new host:

```sh
pkgs/cold-store-reaper/cold-store init            # creates home, template, manifest
# fill credentials.json (client_id/secret) from Bitwarden, mode stays 600
pkgs/cold-store-reaper/cold-store auth-url        # open the URL, sign in, grant
pkgs/cold-store-reaper/cold-store auth-exchange --code <code from the http://localhost redirect>
pkgs/cold-store-reaper/cold-store ensure-folders  # creates the app-owned Drive tree
```

`auth-url` requests offline access with `prompt=consent` so Google issues a
refresh token; `auth-exchange` stores it into `credentials.json` (still mode
600). Secret values are never printed by any command.

## Daily use

```sh
cold-store pack backup-2026-10-25.sql photos.tar      # -> bundles/b_<...>.csb, manifest rows
cold-store upload b_<...>                              # resumable upload + verify + ledger
cold-store fetch <object-sha256> ./restored.sql       # range-pull one object back
cold-store status                                     # counts, cold bytes, last-24h transfers,
                                                      # 700 GB/day hard stop + 3 TiB budget (see below)
cold-store sync-manifest                              # push the local manifest into D1
cold-store sync-manifest --dry-run                    # ... or just print the statement counts
cold-store backup-d1                                  # export the D1 manifest DB into db-backups/d1/
cold-store backup-supabase                            # file the newest Supabase dump from R2 into db-backups/supabase/
cold-store evict-scan                                 # dry-run: which R2 objects are safe to evict? (never deletes)
cold-store evict --apply                              # delete the evictable ones (each re-verified first)
cold-store prune-backups                              # retention plan (dry-run)
cold-store prune-backups --apply                      # ... and trash the excess
cold-store get-backup d1 <name> ./restored.sql        # download one backup in full
```

### Upload guardrails (enforced, not just reported)

`upload` and `put-backup` refuse **before any bytes are sent** when the
transfer would cross either limit in the manifest's `meta` table, using
the same ledger/meta accounting as `status` (one shared implementation,
so the readout and the refusal can never disagree):

- **Daily upload hard stop** — rolling-24h `transfer_ledger` upload
  bytes + this transfer > `daily_upload_hard_stop_bytes`
  (700,000,000,000 B = 700 GB, sized to stay under Google Drive's own
  ~750 GB/user/day upload ceiling).
- **Project budget** — verified cold bytes in Drive (sum of verified
  bundles) + this transfer > `project_budget_bytes`
  (3,298,534,883,328 B = 3 TiB).

A refusal is a `ColdStoreError` naming the limit, the current usage,
and the refused transfer size; nothing is uploaded and the bundle is
not marked `uploading`.

## DB backups (design section 5)

Database dumps are a separate, simpler class: single self-describing files,
uploaded **unbundled** into `db-backups/<db>/` under the app-owned root
(`ensure-folders` creates the `supabase/`, `turso/`, and `d1/` subfolders,
idempotently). Files land with dated names —
`<db>-<YYYYMMDDTHHMMSSZ>-<basename>` — and every upload writes a
`backup-upload` row to the transfer ledger.

```sh
cold-store put-backup supabase pgdump-2026-10-26.sql.zst   # prints the Drive file id
cold-store list-backups                                    # all DBs, newest first
cold-store list-backups --db turso
cold-store backup-d1                                       # export the D1 manifest DB itself
                                                           # (REST query API) and file it under d1/
cold-store backup-supabase                                 # file the newest R2 Supabase dump
                                                           # under supabase/ (see "R2" below)
cold-store get-backup d1 d1-<...>-cold-store-manifest-export.sql ./restored.sql
```

The `turso/` folder exists (created by `ensure-folders` like the
others) but has **no filing command yet**: there is no Turso export
source wired into the reaper, so nothing files there until one exists.

### Retention (design section 5)

`prune-backups [--db <db>] [--apply]` enforces **30 daily / 12 weekly /
12 monthly**, per DB folder, as a pure function of the folder listing —
never of the wall clock. For each `db-backups/<db>/` folder it lists the
files (already-trashed files are excluded), parses the timestamp out of
each dated filename (`<db>-<YYYYMMDDTHHMMSSZ>-<basename>`; names that do
not parse are **never** pruned, only reported as skipped-unparseable),
and computes the keep set as the union of:

- the newest backup of each of the **last 30 distinct calendar days**
  present in the folder,
- the newest backup of each of the **last 12 distinct ISO weeks**
  present,
- the newest backup of each of the **last 12 distinct calendar months**
  present.

Everything else is a prune candidate. The default is a dry run: the
plan is printed with a reason per file (`daily` / `weekly` / `monthly`
for keepers, `excess` for candidates). With `--apply`, each candidate
is **trashed** via `files.update {trashed: true}` — never permanently
deleted, so Drive's own trash grace applies — and one `transfer_ledger`
row per file is written (`direction='trash'`, `kind='backup-prune'`,
`ref=<filename>`).

One migration note: the `'trash'` ledger direction is a schema v1.1
extension (see `schema.sql`). Local manifests are migrated in place on
first open, and the **D1 copy of the schema already carries the same
CHECK change** (verified 2026-10-26, #984 — D1 already held real
'trash' ledger rows), so prune rows sync normally.

### Scripts

`scripts/` holds the two operator entry points (bash, `set -euo
pipefail`, no secrets stored or echoed — credentials arrive via the
environment only):

- `scripts/nightly-backup.sh` — the nightly run: `backup-d1`, then
  `backup-supabase` (only when both R2 credentials are set; a skip
  notice is printed otherwise — never an error), then
  `scripts/export-turso.sh`, then `prune-backups --apply`, then
  `sync-manifest`, then `status`. Steps are isolated: a failing step
  never starves the steps after it — every step is attempted, each
  prints an outcome line (`ok` / `FAILED` / `skipped`), the status
  summary always prints, and the script exits non-zero at the end if
  any step failed (guarded skips are not failures). Requires
  `CLOUDFLARE_API_TOKEN` in the environment
  (materialize from Bitwarden into the cron env);
  `R2_ACCESS_KEY_ID` / `R2_SECRET_ACCESS_KEY` are optional (they gate
  the Supabase step), as is `TURSO_PLATFORM_API_TOKEN` (it gates the
  Turso step).
- `scripts/export-turso.sh` — the Turso step: discovers every
  database in the Turso organization through the Platform API (no
  `turso` CLI needed), mints a per-database auth token, downloads
  the SQL dump each database's own libsql endpoint serves at
  `/dump`, rebuilds it into a standalone SQLite file (filed only
  when `PRAGMA integrity_check` passes), gzips it, and files it
  with `put-backup turso`. Each dump download allows
  `TURSO_DUMP_MAX_TIME_SECONDS` seconds (default 1800; a large
  database's dump is ~500 MB of SQL and outgrew the old fixed 300 s
  cap) and gets one whole-request retry on transport failure. When
  `TURSO_PLATFORM_API_TOKEN` is
  unset it prints a one-line skip notice and exits 0 — the nightly
  run never fails for a missing Turso setup — but a database that
  fails to export is named in a loud error line and makes the step
  exit non-zero once the remaining databases have been attempted.
  Token values are never printed.
- `scripts/restore-drill.sh` — the quarterly restore drill (design
  section 8): picks 3 pseudo-random cold objects from the local
  manifest, fetches each through the rehydrator Worker and verifies its
  sha256, then downloads the newest d1 backup with `get-backup` and
  checks it with `scripts/drill_compare.py`: loaded into in-memory
  SQLite, the backup must be a **content-equal subset** of live D1 —
  every backup row present with identical values, keyed by each
  table's primary key. (Row-count equality was the old check; it
  false-failed every healthy backup, because the manifest is
  append-mostly and the nightly run appends its own `transfer_ledger`
  rows after its export. Extra live rows are reported as a count
  note, never a failure.) If D1 is unreachable, the fallback oracle
  is the local manifest. Prints `DRILL PASS`/`DRILL FAIL` per item
  and a final verdict; exits non-zero on any failure. Requires
  `REHYDRATOR_API_KEY` and `CLOUDFLARE_API_TOKEN` in the environment.

## R2: Supabase filings and the eviction trial

The `supabase-backups` R2 bucket is wired in both directions: the
nightly job files its newest dump into Drive, and the reaper can evict
bucket objects the cold store provably holds. Access is a minimal
stdlib S3 client (SigV4) at the account's R2 host, with bucket-scoped
credentials from the environment only — materialize from Bitwarden,
never store:

```sh
R2_ACCESS_KEY_ID=<from Bitwarden: cloudflare.r2-backups-access-key-id> \
R2_SECRET_ACCESS_KEY=<from Bitwarden: cloudflare.r2-backups-secret-access-key> \
  cold-store backup-supabase
```

The host defaults to the account's `*.r2.cloudflarestorage.com`
endpoint (override with `COLD_STORE_R2_HOST`; tests stub the whole
base with `COLD_STORE_S3_BASE`).

### backup-supabase

Lists the bucket (paginated ListObjectsV2), picks the **newest backup
object by LastModified** — `pg/muse-dev/manifest.json` excluded; it is
the backup system's own index, not a dump — downloads it, and files it
via `put-backup` as a `supabase` backup. The download is verified
against the response's Content-Length, and a truncated body (observed
live through this sandbox's egress proxy) retries the whole GET, up
to 3 attempts. Filing is point-in-time, like `backup-d1`: unchanged
content is filed again, and the command prints the content sha256 so
the duplication is visible rather than silent.

If urllib still cannot deliver the object whole, the download falls
back to **one `curl` attempt** (curl does its own SigV4 signing via
`--aws-sigv4` and reads the proxy from the environment). Why: through
this sandbox's egress proxy, small responses truncate only
intermittently — retries absorb those — but multi-megabyte responses
truncate _systematically_ (live: every attempt cut at exactly
409,600 bytes), and curl is immune; the Supabase backup tooling
established the same replay-with-curl precedent, and the reaper
already shells out to a CLI for compression (`zstd`). The curl body
gets the same byte-count verification (the listing's size, or a HEAD
Content-Length when the caller has none), credentials are scrubbed
from any error text, and a missing or failing curl surfaces as a
named `ColdStoreError`, never a traceback.

### Eviction

```sh
cold-store evict-scan                              # verdicts only, never deletes
cold-store evict-scan --min-age-days 30 --keep-newest 5
cold-store evict --apply                           # delete the evictable, re-verified
```

`evict-scan` prints one verdict per bucket object (`manifest.json`
excluded). An object is **evictable** only if _all_ of these hold:

- **(a) hash known** — the key is content-addressed
  (`<sha256>.api.jsonl.zst`), so the manifest can be consulted at all;
- **(b) cold in a verified bundle** — the manifest has the object
  `state='cold'` and a member of a bundle whose state is `verified`
  (Drive upload MD5-checked at ingest);
- **(c) old enough** — LastModified at least `--min-age-days` ago
  (default 14);
- **(d) not protected** — not among the `--keep-newest` newest objects
  in the bucket (default 3).

Every other object gets the first failing reason: `hash-unknown`,
`not-in-manifest`, `bundle-unverified`, `too-young`, `kept-newest`
(age is checked before position — a young object always carries one
of the newest LastModified stamps, so position-first would mask the
age verdict). The scan is a pure read: it never deletes.

`evict` recomputes the scan and, without `--apply`, prints it and
exits non-zero — deletion requires the literal flag. With `--apply`,
each evictable object is **re-verified against the manifest at
deletion time** (the scan may be stale; nothing is deleted on a stale
verdict), then deleted via S3, marked `state='evicted'` in the
manifest, and recorded in the ledger (`direction='trash'`,
`kind='evict'`, `ref=<R2 key>`). One object's failure is reported and
does not abort the rest.

The `'evicted'` state is a schema v1.2 extension (see `schema.sql`):
local manifests are migrated in place on first open, and the **D1
copy of the schema already carries the same CHECK change** (verified
2026-10-26, #984: D1 `sqlite_master` read plus a probe `sync-manifest`
carrying an 'evicted' row succeeded; the probe rows were removed
afterwards), so evicted rows sync normally — the same applied state
the v1.1 `'trash'` direction was verified in.

## Manifest sync → D1

The local SQLite manifest is the reaper's working copy; the hosted copy
lives in the Cloudflare D1 database `cold-store-manifest`, where the
rehydrator Worker reads it. `sync-manifest` pushes all five tables
(objects, bundles, members, transfer_ledger, meta) with batched,
idempotent upserts (`INSERT … ON CONFLICT(<pk>) DO UPDATE`, primary keys
taken from `schema.sql`), so re-running is always safe and a failed run
resumes by simply running again. `--dry-run` prints per-table row and
statement counts and contacts nothing.

The API token comes from the environment only — materialize it from
Bitwarden, never store it:

```sh
CLOUDFLARE_API_TOKEN=<from Bitwarden> cold-store sync-manifest
```

Account/database default to the cold-store D1 database; override with
`COLD_STORE_CF_ACCOUNT_ID` / `COLD_STORE_CF_D1_DATABASE_ID` (env) or the
`cf_account_id` / `cf_d1_database_id` fields of `state.json`. The D1 base
URL is overridable via `COLD_STORE_D1_BASE` (used by tests).

## Rehydrator Worker

`worker/` holds the Cloudflare Worker that serves cold objects back:
`GET /fetch/<sha256>` (bearer auth) → D1 manifest lookup → a per-bundle
Durable Object single-flights a Drive Range pull of the one zstd frame,
decompresses it with a vendored pure-JS decoder, verifies the sha256, and
returns the bytes. See `worker/README.md` for endpoints, secrets, and the
API-driven deploy procedure.

## Configuration

| Knob                                 | Default                                                                                                                                                                  |
| ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Home dir                             | `$COLD_STORE_HOME` or `~/.config/cold-store` (or `--home`)                                                                                                               |
| Bundle target size (design constant) | 8 GiB (`pack` packs what it is given)                                                                                                                                    |
| Upload chunk size                    | 8 MiB                                                                                                                                                                    |
| Endpoints                            | Google; overridable via `COLD_STORE_TOKEN_URL`, `COLD_STORE_DRIVE_BASE`, `COLD_STORE_UPLOAD_BASE` (used by tests)                                                        |
| D1 sync                              | `CLOUDFLARE_API_TOKEN` from env only; account/db via `COLD_STORE_CF_ACCOUNT_ID` / `COLD_STORE_CF_D1_DATABASE_ID` or `state.json`; base via `COLD_STORE_D1_BASE`          |
| R2 (S3)                              | `R2_ACCESS_KEY_ID` / `R2_SECRET_ACCESS_KEY` from env only; host via `COLD_STORE_R2_HOST`, whole base via `COLD_STORE_S3_BASE` (tests); default bucket `supabase-backups` |
| zstd binary                          | `/usr/local/bin/zstd`, or `$COLD_STORE_ZSTD`                                                                                                                             |
| curl binary (S3 download fallback)   | `curl` on PATH, or `$COLD_STORE_CURL`                                                                                                                                    |

## Tests

Stdlib `unittest`, no network, no credentials — a fake Drive HTTP server
(token endpoint, folder create/list, resumable upload sessions with 308
resume, `files.get`, Range downloads), a stubbed D1 REST endpoint, and a
fake S3 endpoint (SigV4 capture, paginated listings, truncate-once
and truncate-always download modes) run in threads:

```sh
python3 -m unittest discover -s pkgs/cold-store-reaper/tests -v
```

Requirements: Python 3.12, the `zstd` CLI, and `curl` (runtime only,
for the S3 download fallback). Everything else is stdlib.

CI runs these suites — plus the worker local proof (`npm test` in
`worker/`) — on every pull request and push to `main` that touches
`pkgs/cold-store-reaper/**`, via `.github/workflows/cold-store-reaper.yml`
(#1427). The command above remains the way to run them locally.
