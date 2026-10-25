# PM projector + link registry

The **projector** is the glue layer of the project-management stack (design
record: [`doc/2026-10-24T11-05-38Z_design_project-management-stack/`](../../doc/2026-10-24T11-05-38Z_design_project-management-stack/2026-10-24T11-05-38Z_design_project-management-stack.md)).
It **links and projects; it never becomes a fifth source of truth** and never
writes back to GitHub / Kaneo / Pad / Grist. GitHub stays the source of truth
for code work; this database is disposable — every row rebuilds from GitHub
plus the link writes.

It implements the two first-class aggregator requirements from the design
record:

1. **Cross-repo links** — a link registry attaching repo-qualified GitHub
   issue/PR references (`owner/repo#N`) to any board item (Kaneo task id) or
   roadmap row (Grist row id), regardless of which repo the item belongs to.
2. **Comments across repos** — a comment-activity projection across all four
   repos, **including PR-interior comments** (reviews and review/line comments)
   that Kaneo never syncs, queryable per board item / roadmap row as one
   unified stream.

## What it does

- **Webhook intake** (`POST /github/webhook`): HMAC-SHA256-verified GitHub App
  webhook receiver for `issues`, `issue_comment`, `pull_request`,
  `pull_request_review`, `pull_request_review_comment`, and `push` events.
  Delivery-GUID idempotency (a redelivered webhook applies at most once).
- **Reconciliation reads** (`POST /reconcile`, or `bun run src/cli.ts
reconcile`): polls the GitHub API with the GitHub App credentials so the
  projection self-heals beyond webhook delivery. Idempotent upserts keyed by
  repo-qualified IDs; per-(repo, stream) cursors make each poll incremental.
- **Link registry** (`GET|POST|DELETE /links`): the cross-repo reference store.
- **Joined read model + comment projection** (`GET /items`, `GET /comments`,
  `GET /subjects/<subject>`) and SQL views `pmp_v_links_items` /
  `pmp_v_subject_activity` for direct database reads.

## Storage

Turso (libSQL), per the design record. The registry's fields — a
repo-qualified `ref` key, an external `subject` key, a loose `relation`, plus
the comment-activity stream — are specific to this glue layer, but they are
**name-spaced** (`pmp_` prefix) so they can live in the fleet's existing
general/muse-meta database rather than requiring a dedicated database. Run
locally against a SQLite file (`file:` URL) or remotely via
`DATABASE_URL` + `TURSO_AUTH_TOKEN`.

## Secrets (referenced by name only — never commit values)

| Env var                  | What it is                                                                |
| ------------------------ | ------------------------------------------------------------------------- |
| `GITHUB_WEBHOOK_SECRET`  | the GitHub App's webhook secret (`github_webhook_secret`)                 |
| `GITHUB_APP_ID`          | the App's numeric id (`github_app_id`)                                    |
| `GITHUB_APP_PRIVATE_KEY` | the App PEM private key, or base64 of it (`github_private_key_base64`)    |
| `GITHUB_TOKEN`           | fallback plain token for repos the App cannot reach (e.g. `dezren39/nix`) |
| `TURSO_AUTH_TOKEN`       | libSQL/Turso auth token for a remote `DATABASE_URL`                       |
| `PM_PROJECTOR_API_TOKEN` | bearer token guarding link writes + reconcile                             |
| `DATABASE_URL`           | libSQL connection URL (defaults to `file:data/projector.db`)              |

`PM_REPOS` is a comma-separated allow-list of `owner/repo` names for intake and
reconcile (e.g. `developing-today/code,developing-today/opencode,
developing-today/openchamber,dezren39/nix`).

## Run it locally

```bash
cd pm-stack/projector
./scripts/setup-deps.sh   # provisions @libsql/client from the local cache
                          # (see "Dependency provisioning" below); or bun install
bun test                  # unit + e2e proof (fixture webhook sequence)
bun run start             # HTTP API on 127.0.0.1:7801
```

Environment is read from the process environment; copy `.env.example` to a
local source you don't commit.

### Endpoints

| Route                     | Method | Auth   | Purpose                                              |
| ------------------------- | ------ | ------ | ---------------------------------------------------- |
| `/health`                 | GET    | none   | ok + row counts                                      |
| `/github/webhook`         | POST   | HMAC   | GitHub App webhook intake                            |
| `/items?refs=a/b#1,c/d#2` | GET    | none   | items (or `?repo=`, `?state=`, `?kind=`, `?limit=`)  |
| `/comments?refs=…`        | GET    | none   | unified comment stream, newest first                 |
| `/comments?subject=…`     | GET    | none   | comment stream across everything linked to a subject |
| `/links?subject=…`        | GET    | none   | registry rows (or `?ref=`)                           |
| `/links`                  | POST   | bearer | create/update a link                                 |
| `/links?subject=…&ref=…`  | DELETE | bearer | remove a link                                        |
| `/subjects/<subject>`     | GET    | none   | joined view: links + live item state + activity      |
| `/reconcile`              | POST   | bearer | trigger a reconciliation poll                        |

**Subjects** are external keys:

- `kaneo:<projectSlug>/<taskId>` — a Kaneo board item (e.g. `kaneo:code/CODE-107`)
- `grist:<docId>/<tableId>/<rowId>` — a Grist roadmap row (e.g.
  `grist:czYHYKgG9SHcm9CPwiKEq1/Roadmap/5`); Grist row ids are only unique per
  table, hence the full path
- `gh:<ref>` — another item, for one-hop "related" links

`relation` is a loose label: `tracks` | `implements` | `relates`.

## Read schema for Session K (aggregator UI)

Session K can read the projection either over HTTP (set `PM_PROJECTOR_URL`,
default `http://127.0.0.1:7801`) or directly from the libSQL database via the
views:

- `pmp_v_links_items` — every link joined to the item's live `title`, `state`,
  `labels`, `draft`, `merged`, `author`, `updated_at`, `closed_at`.
- `pmp_v_subject_activity` — the unified activity stream per subject (comment
  `id`, `kind` in `issue_comment|review|review_comment`, `author`, `body`,
  `created_at`, `url`, `path`, `line`, `review_state`), newest first.
- Tables: `pmp_items`, `pmp_comments`, `pmp_links`, `pmp_deliveries`,
  `pmp_sync_state`, `pmp_push_events`. Schema: [`schema.sql`](schema.sql).

The board (Kaneo tasks across the four projects + GitHub-only WIP) is **not**
projected by the projector — Session K reads Kaneo and Grist directly for
their own layers, and the projector supplies only the cross-repo link and
comment layers those tools cannot provide.

## Layout

```text
pm-stack/projector/
├── README.md              # this file
├── schema.sql             # tables + read-model views
├── package.json
├── .env.example           # env names only, no values
├── scripts/
│   └── setup-deps.sh      # offline @libsql/client provisioning
├── fixtures/              # realistic webhook payload sequence (E2E proof)
├── src/
│   ├── index.ts           # entry point
│   ├── cli.ts             # one-off reconcile
│   ├── server.ts          # HTTP routes + HMAC-gated webhook
│   ├── webhook.ts         # HMAC-SHA256 verification (edge-gateway pattern)
│   ├── projector.ts       # event -> store rows (pure)
│   ├── reconcile.ts       # GitHub API reconciliation
│   ├── github.ts          # App JWT + installation token + pagination
│   └── store.ts           # libSQL store + queries
└── test/
    ├── store.test.ts
    ├── projector.test.ts
    └── e2e.test.ts        # fixture webhook sequence -> registry + read model
```

## Dependency provisioning (offline)

`bun install` needs the npm registry, which this laptop can't reach reliably
(it hangs resolving). A complete `@libsql/client@0.18.0` tree already exists in
the opencode-mem package cache, so `scripts/setup-deps.sh` copies the needed
packages (`@libsql/client` + `@libsql/core`, `@libsql/hrana-client`,
`@libsql/isomorphic-ws`, `@libsql/linux-x64-*`, `libsql`, `js-base64`,
`promise-limit`, `ws`) into `./node_modules`. `node_modules/` is gitignored.
When the registry is reachable, `bun install` in this directory is the normal
path.

## What is proven, what is stubbed

**Proven** (by `bun test`, including the e2e fixture sequence): webhook intake
with real HMAC-SHA256 verification and delivery-GUID idempotency; issue / PR /
review / review-comment / push projection; the link registry write path with
bearer auth; the joined `/subjects/<subject>` view; the unified comment stream
with PR-interior comments; and the monotonic upsert guard.

**Stubbed / not yet live**: reconciliation is implemented but not exercised
against the live GitHub API here (no App credentials are exported in this
environment, and the App is installed on `developing-today/code` only — the
`dezren39/nix` repo would need a separate installation or a `GITHUB_TOKEN`
fallback). Kaneo/Grist board projection is out of scope (Session K reads those
directly). The Cloudflare Worker packaging for the webhook receiver is a
follow-up; the receiver here follows the same HMAC discipline and is a
drop-in for that edge gateway's `/github/webhook` route.
