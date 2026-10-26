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
```

**Retention is NOT enforced yet**: the design's 30 daily / 12 weekly /
12 monthly policy (trash, not delete) is pending a later version. Until
then, backups accumulate and pruning is manual.

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
