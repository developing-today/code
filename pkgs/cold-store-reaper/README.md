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
  state.json                 # folder IDs + cached access token (no secrets)
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
                                                      # 700 GB/day hard-stop + 3 TiB budget checks
cold-store sync-manifest                              # push the local manifest into D1
cold-store sync-manifest --dry-run                    # ... or just print the statement counts
cold-store backup-d1                                  # export the D1 manifest DB into db-backups/d1/
cold-store prune-backups                              # retention plan (dry-run)
cold-store prune-backups --apply                      # ... and trash the excess
cold-store get-backup d1 <name> ./restored.sql        # download one backup in full
```

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
cold-store get-backup d1 d1-<...>-cold-store-manifest-export.sql ./restored.sql
```

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

One migration caveat: the `'trash'` ledger direction is a schema v1.1
extension (see `schema.sql`). Local manifests are migrated in place on
first open, but the **D1 copy of the schema needs the same one-line
CHECK change** before prune rows can sync — until then `sync-manifest`
fails on the `transfer_ledger` statement with D1's own error text.

### Scripts

`scripts/` holds the two operator entry points (bash, `set -euo
pipefail`, no secrets stored or echoed — credentials arrive via the
environment only):

- `scripts/nightly-backup.sh` — the nightly run: `backup-d1`, then
  `prune-backups --apply`, then `sync-manifest`, then `status`. Any
  failing step aborts the run non-zero. Requires `CLOUDFLARE_API_TOKEN`
  in the environment (materialize from Bitwarden into the cron env).
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

| Knob                                 | Default                                                                                                                                                         |
| ------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Home dir                             | `$COLD_STORE_HOME` or `~/.config/cold-store` (or `--home`)                                                                                                      |
| Bundle target size (design constant) | 8 GiB (`pack` packs what it is given)                                                                                                                           |
| Upload chunk size                    | 8 MiB                                                                                                                                                           |
| Endpoints                            | Google; overridable via `COLD_STORE_TOKEN_URL`, `COLD_STORE_DRIVE_BASE`, `COLD_STORE_UPLOAD_BASE` (used by tests)                                               |
| D1 sync                              | `CLOUDFLARE_API_TOKEN` from env only; account/db via `COLD_STORE_CF_ACCOUNT_ID` / `COLD_STORE_CF_D1_DATABASE_ID` or `state.json`; base via `COLD_STORE_D1_BASE` |
| zstd binary                          | `/usr/local/bin/zstd`, or `$COLD_STORE_ZSTD`                                                                                                                    |

## Tests

Stdlib `unittest`, no network, no credentials — a fake Drive HTTP server
(token endpoint, folder create/list, resumable upload sessions with 308
resume, `files.get`, Range downloads) runs in a thread:

```sh
python3 -m unittest discover -s pkgs/cold-store-reaper/tests -v
```

Requirements: Python 3.12, the `zstd` CLI. Everything else is stdlib.
