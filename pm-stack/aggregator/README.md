# pm-aggregator-ui

The PM-stack **aggregator UI** — the dashboard Drew lives in while the
fleet works in GitHub, Kaneo, and Pad underneath. It is a **read-mostly
projection**: it never writes back to GitHub, Kaneo, Pad, or Grist, and
it is disposable by design — if it rots, every layer underneath still
works standalone (design record, "Data flow").

Design record (source of truth):
[`doc/2026-10-24T11-05-38Z_design_project-management-stack/`](../../doc/2026-10-24T11-05-38Z_design_project-management-stack/).

## The four views

| Route             | View                                                                                                                                                    | Machine-first JSON    |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------- |
| `/`               | **Cross-repo WIP board** — the curated in-flight set across all four repos, with an execution strip per card (implementing → review → merged → queued). | `/api/board.json`     |
| `/roadmap`        | **Roadmap** — Grist's `Program roadmap` table as a timeline (theme × quarter lane); each row drills down to its linked issues/PRs with live state.      | `/api/roadmap.json`   |
| `/quota`          | **Quota + throughput** (#851) — all pooled quotas, resets, burn rates, and model-choice guidance.                                                       | `/api/quota.json`     |
| `/attention`      | **Needs you** (#852) — every live agent session, classified and attention-ordered, with the evidence inline.                                            | `/api/attention.json` |
| `/item/<subject>` | Drill-down for one board card or roadmap row: linked refs + a unified comment stream.                                                                   | —                     |

Each view is **machine-first**: the `/api/*.json` endpoint returns the
exact same data the GUI renders, so Muse can poll it in one call.

## First-class requirements (from the design record)

1. **Cross-repo links.** Any board card or roadmap row links any GitHub
   issue/PR from any of the four repos (`developing-today/{code,opencode,openchamber}`,
   `dezren39/nix`), served live from the projector's link registry. Kaneo
   binds one repo per project and cannot do this; the registry keys are
   repo-qualified (`owner/repo#N`).
2. **Unified comment stream.** One activity stream per item covering its
   linked issues/PRs across all repos, PR-interior comments included
   (reviews and line comments), from the projector's comment projection.

Both render in the board cards (execution strip) and the item drill-down.

## Run it

```bash
cd pm-stack/aggregator
bun install
bun run dev          # http://127.0.0.1:4870
```

Tests (unit derivations + render + a boot smoke test that serves every
view from fixtures):

```bash
bun test
```

## Configuration (env, secrets by NAME only)

Everything defaults to **fixtures** and works with no configuration:

| Env var                | Default                  | Purpose                                                                                 |
| ---------------------- | ------------------------ | --------------------------------------------------------------------------------------- |
| `PM_AGG_SOURCE`        | `fixtures`               | `live` reads Kaneo/Grist/projector, falling back to fixtures per facet                  |
| `PM_AGG_PORT`          | `4870`                   | HTTP port                                                                               |
| `PM_AGG_HOSTS`         | `127.0.0.1`              | Comma-separated bind addresses (deployment: loopback + tailnet; no public route)        |
| `PM_AGG_FEED_FILE`     | `./fleet-feed.json`      | The fleet feed (quota + sandbox attention) written by the collector (below)             |
| `PM_AGG_OPENCHAMBER_STATE` | `~/.config/openchamber/managed-opencode/state.json` | Laptop OpenCode server port + password source (laptop attention, live) |
| `KANEO_URL`            | `http://127.0.0.1:5173`  | Kaneo base URL (board reader)                                                           |
| `KANEO_ADMIN_EMAIL`    | —                        | Kaneo admin email (secret `kaneo.admin-password` pairs with it)                         |
| `KANEO_ADMIN_PASSWORD` | —                        | Kaneo admin password (Bitwarden `kaneo.admin-password`)                                 |
| `GRIST_URL`            | `http://127.0.0.1:8484`  | Grist base URL                                                                          |
| `GRIST_API_KEY`        | —                        | Grist API key (Bitwarden `grist.api-key`)                                               |
| `GRIST_ROADMAP_DOC`    | `czYHYKgG9SHcm9CPwiKEq1` | The `Program roadmap` document id (Phase 2 seed)                                        |
| `PM_PROJECTOR_URL`     | `http://127.0.0.1:7801`  | Projector + link registry base URL (`/items`, `/comments`, `/links`); `off` disables it |

Secrets are read from the environment only and are never committed or
printed. Names here reference Bitwarden entries (`kaneo.*`, `grist.*`).

## What each facet reads

| Facet            | Live reader (when `PM_AGG_SOURCE=live`)                                      | Fallback                            |
| ---------------- | ---------------------------------------------------------------------------- | ----------------------------------- |
| Board            | Kaneo REST (`/api/task/tasks/<project>`, all four projects) + registry links | fixtures                            |
| Roadmap          | Grist REST (`/api/docs/<doc>/tables/Roadmap/records`) + registry links       | fixtures (the real Grist seed rows) |
| Registry links   | Projector `GET /links?subject=…`, unioned into board cards and roadmap rows  | Kaneo external links / text refs    |
| Items (refs)     | Projector `GET /items?refs=…`                                                | fixtures                            |
| Comments         | Projector `GET /comments?refs=…`                                             | fixtures                            |
| Quota (#851)     | Fleet feed file (`fleet-feed.json`; see "The fleet feed" below)              | fixtures when no feed exists        |
| Attention (#852) | Laptop OpenCode server live (state file, 45s cache) + sandbox via fleet feed | fixtures when neither is reachable  |

The projector's wire schema is its `schema.sql` (Session J, PR #859):
snake_case fields, `state` is only `open|closed` with `merged` as a
separate 0/1 flag, `labels`/`assignees` are JSON array strings, and
comment rows carry `created_at` / `review_state`. The mapping to this
app's read model lives in `src/sources/live.ts`
(`mapProjectorItem` / `mapProjectorComment` / `parseProjectorLabels`),
covered by `test/projector.test.ts` — including a stub-server test that
refs survive the query string (`#` is percent-encoded, otherwise it
would start a URL fragment and the projector would never see the
number). Registry subjects are `kaneo:<projectSlug>/<taskId>` and
`grist:<docId>/<tableId>/<rowId>`; board cards and roadmap rows are
enriched with `GET /links?subject=…` so cross-repo links (the
requirement Kaneo cannot provide) render live.

The footer of every page states, facet by facet, whether it is live or
fixture-backed. When a live reader is unreachable, that facet alone
falls back to fixtures and the footer says why.

## The fleet feed (quota + sandbox attention, slice 1 — #890)

The sandbox OpenCode server binds its own loopback and the quota
sources of record live on the sandbox, so those two facets travel as
a file. `scripts/collect-fleet-feed.ts` runs on the sandbox:

- **Quota** — per-pool extractors (`src/quota-extract.ts`) over the
  stewardship quota view's dated reads. Fail-soft by design: a pool
  whose read pattern stops matching renders with its identity and a
  "no current read" note — never a vanished pool, never an invented
  number. Live per-pool burn (active sessions + their token totals)
  is attributed from the sandbox server at collection time, and
  model-choice guidance (`src/quota-guidance.ts`) is computed then.
  The pool map is the corrected one from the provider-quota
  registry: exactly one Meta pool (Muse API High Usage, `meta`
  provider the only door); no "muse plan" pool exists.
- **Sandbox attention** — sessions mapped by the shared OpenCode v2
  mapper (`src/sources/opencode.ts`); keeper supervision entries
  from `jules-review-state.json` annotate matching sessions.

`scripts/push-fleet-feed.sh` pushes the feed to the laptop over the
tailnet proxy route; a systemd timer on the sandbox runs it every
~5 minutes (`systemd/fleet-feed-collector.{service,timer}`). The
aggregator states the feed's age in the page provenance; a stale
feed keeps rendering.

**Deployment (laptop):** `install.sh` copies the package to
`~/pm-stack/aggregator`, renders `.env` from
`~/secrets/pm-stack.sops.yaml` (keys by name), installs
`../systemd/pm-aggregator.service` as a user unit, and health-checks
loopback + tailnet. Slice 2 (per #890) replaces the feed with
projector-ized live per-source feeds.

## What is fixture-backed vs live

- **Fixture-backed today:** nothing, when the stack is up — board,
  roadmap, items, comments, quota, and attention all read live
  sources, each falling back to fixtures independently when its
  source is unreachable (the fixtures are shaped like the live
  readers' output and sit behind the same `Source` interface,
  `src/source.ts`). Roadmap fixtures are
  the real Grist seed rows; the design record's known refs (#752, #754,
  #762, #763, nix #501–#508, opencode #365) are real; the rest are
  illustrative.
- **Live-capable today:** board (Kaneo), roadmap (Grist), and — since
  Session J's projector landed (PR #859) — registry links, live item
  state, and the comment projection via `PM_PROJECTOR_URL`
  (default `http://127.0.0.1:7801`). Each facet falls back to fixtures
  independently when its service is unreachable. The live board is the
  curated in-flight set (in-progress / in-review / recent done columns;
  the to-do backlog is excluded). Kaneo project slugs map to repos as
  in `pm-stack/kaneo-projects.json` (Session H): `code`, `opencode`,
  `openchamber` under `developing-today/`, `nix` under `dezren39/`.

## Design notes — the two new views (#851, #852)

Both were added as **design issues** (2026-10-25) and are implemented
here as machine-first surfaces with a GUI over the same data.

- **#851 quota + throughput:** the data model is `QuotaPool` /
  `QuotaWindow` / `BurnSegment` / `QuotaView` (`src/model.ts`); wall
  detection and near-limit thresholds live in `src/derive.ts`
  (`poolState`, `NEAR_PCT = 85`). Slice 1 (#890) reads quotas from
  the fleet feed (above) rather than the projector; per-source live
  readers (projector-ized) are slice 2. No estimator duplicates the
  OpenChamber quota picker; picker values are logged, not
  re-estimated (standing rule 2026-10-24).
- **#852 needs-you:** the state taxonomy and classification rules are
  `AttentionState` + `classifyAttention`/`attentionSorted` in
  `src/derive.ts`. Ordering is stopped-for-question and errored first,
  then needs-poke / stuck-retrying / stalled-on-CLI, then in-motion;
  every non-in-motion entry carries its evidence inline. Slice 1
  maps the OpenCode v2 servers directly (`src/sources/opencode.ts`);
  a projector-ized session feed is slice 2.

Neither view vendors anything from the CCC reference design
(`claude-command-center`, FSL-1.1-MIT) — only its idea of a
machine-first data surface with a GUI on top is used.

## Layout

```
src/
  model.ts      read model (refs, board, roadmap, quota, attention)
  source.ts     the one Source interface
  derive.ts     execution strip, timeline lanes, quota/attention rules
  render.ts     pure HTML rendering of every view
  server.ts     Bun HTTP server (read-only routes)
  config.ts     env config
  feed.ts       fleet-feed file format + loader
  quota-extract.ts  per-pool extractors over the quota view's dated reads
  quota-guidance.ts headroom / burn-before-reset / do-not-start derivation
  sources/
    fixtures.ts   fixture data (real Grist seed + design-record refs)
    live.ts       Kaneo/Grist/projector readers with per-facet fallback
    opencode.ts   OpenCode v2 server -> SessionFeedEntry mapper (shared)
scripts/
  collect-fleet-feed.ts  sandbox collector: quota + sandbox attention -> feed
  push-fleet-feed.sh     collect + push the feed to the laptop (~5 min timer)
systemd/
  fleet-feed-collector.{service,timer}  sandbox-side collector units
test/
  render.test.ts    unit derivations + render smoke
  boot.test.ts      boots the server, serves all views from fixtures
  projector.test.ts projector wire-schema mapping + stub-server reads
  opencode.test.ts  mapper against a stub OpenCode v2 server
  quota.test.ts     extractors + guidance against the dated-read format
```
