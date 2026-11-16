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
  Delivery-GUID dedupe: intake rejects a missing delivery id with 400, and
  a delivery counts as handled only once it applied or was ignored (see the
  projection contract below).
- **Reconciliation reads** (`POST /reconcile`, or `bun run src/cli.ts
reconcile`): polls the GitHub API with the GitHub App credentials so the
  projection self-heals beyond webhook delivery. Idempotent upserts keyed by
  repo-qualified IDs; per-(repo, stream) cursors make each poll incremental.
- **Link registry** (`GET|POST|DELETE /links`): the cross-repo reference store.
- **Joined read model + comment projection** (`GET /items`, `GET /comments`,
  `GET /subjects/<subject>`) and SQL views `pmp_v_links_items` /
  `pmp_v_subject_activity` for direct database reads.

## Projection contract (#4302, audited in #4303)

**A projection writes only what the event proves. An absent field proves
nothing — it neither defaults a value nor erases a stored one — and the only
timestamps a row carries are ones its own payload reported.**

The six decisions, as implemented:

1. **Delivery identity.** Webhook intake rejects a request with no
   `x-github-delivery` header (400 `missing_delivery_id`) before any write;
   every dedupe key is the delivery GUID. `recordPush` inherits the rule —
   its insert keys `ON CONFLICT(delivery_id) DO NOTHING`.
2. **Dedupe counts success only.** A delivery counts as handled iff its
   outcome is `applied` or `ignored`; error rows are audit-only and stay
   retryable. A later success replaces the delivery row's outcome while the
   prior error is retained in `pmp_delivery_errors`. One deliberate
   deviation: an `invalid_json` delivery is recorded as an error and is
   *not* counted as handled, so a redelivery is rejected 400 again rather
   than acked as a duplicate — a malformed body can never become valid
   under the same GUID, and the 400 keeps the failure visible (pinned by
   the e2e test "invalid JSON redelivery remains a 400").
3. **Comment identity is kind-namespaced.** The primary key is
   `(id, kind)`; every writer builds rows through the shared constructors
   (`commentCommon` for webhooks, the per-stream builders in reconcile), so
   an issue comment, a review, and a review comment with the same numeric
   id are three rows, never one.
4. **The comment guard key is the last-update time the payload reports** —
   `updated_at` for comments, `submitted_at` for reviews — never a
   fabricated timestamp. A payload carrying none of its kind's timestamps
   writes no row. (The webhook review writer fabricated delivery-time
   timestamps until the #4303 audit fixed it to derive from
   `submitted_at`, matching reconcile byte-for-byte.)
5. **Comment deletion is absorbing.** A tombstone wins over any later
   upsert regardless of guard key; the upsert's conflict clause requires
   `deleted = 0`.
6. **Item deletion removes the item, its links, and its live comments**,
   and plants an absorbing deletion marker (`pmp_item_deletions`, #6782):
   only a strictly newer item write clears it. Comment tombstones are
   retained — they are the absorbing guard of decision 5, and read paths
   exclude deleted rows.

Field absence on items follows the same rule (origins #4158/#4210): merge
fields (`draft`, `merged`, `merged_at`) are carried only by pulls-shaped
payloads, detected by key presence; an issues-shaped write leaves stored
values untouched (COALESCE in the upsert), and a pulls-shaped payload that
reports `merged_at: null` clears it.

### Writer inventory (Phase 4 audit, #4303)

Every writer of the four projected tables, dispositioned against the
contract. "Conforms" was verified against the code and tests this audit,
not assumed.

**`pmp_items`** — via `store.upsertItem(s)` unless noted:
- Webhook `issues` (all non-deleted actions) — conforms. The `transferred`
  action also lands here today; see disposition D-1 below.
- Webhook `issues` `deleted` — `store.deleteItem` — conforms (decision 6).
- Webhook `pull_request` — conforms.
- Webhook `issue_comment`, `pull_request_review`,
  `pull_request_review_comment` (parent-item refresh) — conforms.
- Reconcile `issues` stream — conforms (issues-shaped: merge fields absent,
  stored values preserved by the key-presence rule).
- Reconcile `prs` stream — conforms (pulls-shaped; also heals rows first
  written from issues-shaped data).

**`pmp_comments`** — via `store.upsertComment(s)` unless noted:
- Webhook `issue_comment` created/edited — `commentCommon` — conforms.
- Webhook `issue_comment` deleted — `store.markCommentDeleted` — conforms
  (absorbing tombstone, decision 5).
- Webhook `pull_request_review` submitted — **fixed in this audit**:
  created_at and the guard key were fabricated from delivery time; both
  now derive from `submitted_at` (decision 4).
- Webhook `pull_request_review_comment` created/edited/deleted — conforms.
- Reconcile `issue_comments` / `review_comments` streams — conforms (own
  row builders; rows with no payload timestamps are skipped).
- Reconcile `reviews` stream — conforms (`submitted_at`-derived).
- `store.deleteItem`'s comment delete (live comments only; tombstones
  retained as decision 5's guard) — conforms, see decision 6 note.

**`pmp_deliveries`** — `store.recordDelivery` from the webhook intake only:
outcomes `applied` / `ignored` / `error` — conforms (decision 2, with the
`invalid_json` deviation recorded there). Error detail is additionally
retained in `pmp_delivery_errors` (insert-or-ignore: the first error for a
delivery is the one kept).

**`pmp_push_events`** — webhook `push` branch via `store.recordPush` only
— conforms (decision 1: intake guarantees the delivery id; the insert is
`ON CONFLICT(delivery_id) DO NOTHING`; a redelivered push GUID yields one
row, proven by the e2e duplicate-delivery test).

**`pmp_links`** — `POST /links` → `store.upsertLink`, `DELETE /links` →
`store.deleteLink`; no other writers — conforms.

### Audit dispositions (#4303)

- **D-1, transferred issues.** Decided: **delete the source-repo row.** A
  `transferred` event proves the issue left the source repo; keeping a live
  row renders an issue that no longer exists there. Implementation is a
  named follow-up, not done blind here: the repo holds no captured
  `transferred` payload, and the payload's number semantics (source number
  vs new number in `payload.issue`) are unverified — deleting on an
  unverified key risks removing the wrong row. The follow-up captures a
  real payload as a fixture first, then wires the branch to
  `store.deleteItem`.
- **D-2, the item row's `new Date()` created-at fallback** (and
  `commentCommon`'s, for comment kinds): **deliberately local.** GitHub
  issue/PR/comment objects always carry `created_at` and `updated_at` in
  both webhook and API shapes — every fixture in this repo does — so the
  fallback is unreachable from the only two sources; it exists for
  synthetic payloads. Removing it would mean a skip-row redesign across
  the reconcile streams for no production effect. The one kind whose real
  payload lacks those fields — reviews — is fixed above.
- **D-3, `recordPush` dedupe under Phase 1:** verified, not assumed —
  intake 400 (above), conflict key on the delivery id, e2e duplicate test.
- **D-4, secondary comment fields** (`review_state`, `path` / `line`,
  `diff_hunk`): the comment upsert overwrites all fields on a
  guard-passing write — no COALESCE — and that is sound: each kind's
  events re-send the kind's full field set, the guard admits only the
  newest version of the comment, and reconcile writes the same full-row
  shape. A null in a newer full payload is the payload's truth (the field
  was cleared), not an absence. No change.

## Storage

Turso (libSQL), per the design record. The registry's fields — a
repo-qualified `ref` key, an external `subject` key, a loose `relation`, plus
the comment-activity stream — are specific to this glue layer, but they are
**name-spaced** (`pmp_` prefix) so they can live in the fleet's existing
general/muse-meta database rather than requiring a dedicated database. Run
locally against a SQLite file (`file:` URL) or remotely via
`DATABASE_URL` + `TURSO_AUTH_TOKEN`.

### Upgrading the comment identity schema

Existing databases with the old `pmp_comments(id)` primary key fail startup
with an explicit error. Stop all projector writers and back up the entire
database first, including the human-written `pmp_links` registry. Do not
delete the database: it may contain unrelated fleet data. Using the SQL client
for that database, run:

```sql
DROP VIEW IF EXISTS pmp_v_subject_activity;
DROP TABLE pmp_comments;
```

Start the upgraded projector to recreate the table and view, then run a full
reconciliation for every configured repository and check all stream errors.
The comment and review streams reread their full GitHub collections. This
rebuild cannot recover deleted comments or overwritten historical records
that GitHub no longer returns; retain the backup for those records. Links,
items, deliveries, cursors, and other applications' tables are untouched.

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
reconcile (e.g. `developing-today/code,developing-today-forks/opencode,
developing-today-forks/openchamber,dezren39/nix`).

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

| Route                                         | Method | Auth   | Purpose                                                           |
| --------------------------------------------- | ------ | ------ | ----------------------------------------------------------------- |
| `/health`                                     | GET    | none   | ok + row counts                                                   |
| `/github/webhook`                             | POST   | HMAC   | GitHub App webhook intake                                         |
| `/items?refs=a/b#1,c/d#2`                     | GET    | none   | items (or `?repo=`, `?state=`, `?kind=`, `?limit=`)               |
| `/comments?refs=…`                            | GET    | none   | unified comment stream, newest first                              |
| `/comments?subject=…`                         | GET    | none   | comment stream across everything linked to a subject              |
| `/links?subject=…`                            | GET    | none   | registry rows (or `?ref=`)                                        |
| `/links`                                      | POST   | bearer | create/update a link                                              |
| `/links?subject=…&ref=…`                      | DELETE | bearer | remove a link                                                     |
| `/subjects/<subject>`                         | GET    | none   | joined view: links + live item state + activity                   |
| `/hot?after=<seq>&limit=<n>&high_water=<seq>` | GET    | none   | hot-list page (ascending, exclusive cursor) + issued `high_water` |
| `/hot/checkpoints?consumer=<id>`              | GET    | none   | a consumer's acked checkpoint cursor                              |
| `/hot/checkpoints`                            | POST   | bearer | ack a checkpoint `{consumer, cursor, highWater}`                  |
| `/reconcile`                                  | POST   | bearer | trigger a reconciliation poll                                     |

### Integrated queue/PM view — machine + human faces (#7078, #2072 slice 3)

`bun run src/cli.ts queue-view --tasks <tasks.json> [--window-hours 24] [--consumer queue-view] [--no-ack]`
prints one read-only JSON view joining fleet queue task records (JSON
exported from the queue broker: an array, or `{ "tasks": [...] }`) with
this projector's slice-2 hot list, consumed through a consumer
checkpoint (pinned high-water; the checkpoint is acked only after the
view prints, unless `--no-ack`). The view lists queued items in claim
order, items claimed inside the window with their claiming
system/lane, snapshot counts by status (which reconcile with
`fleet queue health` when fed the full task list), and links to the
existing aggregator/projector trend stores. It writes nothing and
collects nothing new.

`--format html` on the same command prints the human face: the same
view rendered as one standalone HTML page/report (status counts,
queued in claim order, picked-up items with their claiming
system/lane, hot-list summary, and trend panels that link — never
copy — the same stores the machine face links). The read model is
identical for both faces; only the rendering differs
(`src/queue-view-html.ts`).

**Subjects** are external keys:

- `kaneo:<projectSlug>/<taskId>` — a Kaneo board item (e.g. `kaneo:code/CODE-107`)
- `grist:<docId>/<tableId>/<rowId>` — a Grist roadmap row (e.g.
  `grist:czYHYKgG9SHcm9CPwiKEq1/Roadmap/5`); Grist row ids are only unique per
  table, hence the full path
- `gh:<ref>` — another item, for one-hop "related" links

`relation` is a loose label: `tracks` | `implements` | `relates`.

## Hot list (#5204)

Alongside the item/comment projection, every handled webhook event also
appends one normalized row per sub-event to `pmp_hot_events` — a durable,
queryable change ledger consumers sweep instead of polling GitHub. Rows
carry a stable `event_id`, the delivery id, repo, event/action, the source
ref (`owner/repo#N`, or the git ref for pushes), actor, source link and
source timestamp where available, plus the received timestamp. Full raw
webhook bodies are never persisted. A push appends one row per commit.

- **Idempotency:** `event_id` is `"<x-github-delivery>:<sub-event>"`, so a
  redelivery of the same delivery appends nothing twice; distinct
  deliveries stay distinct rows. `seq` is the monotonic (not necessarily
  dense) pagination cursor.
- **Sweep protocol:** `GET /hot?after=<seq>&limit=<n>` returns events in
  ascending `seq` order strictly after the exclusive cursor, plus the
  `high_water` the server issued for the sweep. Pass that `high_water`
  back on later pages to pin the sweep to a fixed window; events arriving
  mid-sweep belong to the next sweep.
- **Checkpoints:** a consumer acks only after its sweep completes
  (`POST /hot/checkpoints`, bearer). Acks are monotonic (a stale ack never
  rewinds) and a cursor past the sweep's issued `high_water` is rejected,
  so a failed or partial sweep resumes from its old checkpoint and
  replays safely. The bound is enforced against server-controlled state:
  a `high_water` pin on `GET /hot` is clamped to the ledger's actual
  high-water, and an ack whose claimed `highWater` exceeds the ledger's
  current high-water is rejected (`high_water_past_server`) -- a caller
  cannot mint a future bound and skip events that arrive later.

Retention/ownership: the ledger lives in the same namespaced (`pmp_`)
database as the rest of the projection and is owned by the projector;
rows are disposable and rebuildable by replaying deliveries, like every
other projection here. Wiring the live GitHub App fanout/deployment that
feeds production consumers is a later slice — this slice is the store,
the projector mapping, and the HTTP protocol only.

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
