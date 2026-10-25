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

| Env var                | Default                                        | Purpose                                                                |
| ---------------------- | ---------------------------------------------- | ---------------------------------------------------------------------- |
| `PM_AGG_SOURCE`        | `fixtures`                                     | `live` reads Kaneo/Grist/projector, falling back to fixtures per facet |
| `PM_AGG_PORT`          | `4870`                                         | HTTP port                                                              |
| `KANEO_URL`            | `http://127.0.0.1:5173`                        | Kaneo base URL (board reader)                                          |
| `KANEO_ADMIN_EMAIL`    | —                                              | Kaneo admin email (secret `kaneo.admin-password` pairs with it)        |
| `KANEO_ADMIN_PASSWORD` | —                                              | Kaneo admin password (Bitwarden `kaneo.admin-password`)                |
| `GRIST_URL`            | `http://127.0.0.1:8484`                        | Grist base URL                                                         |
| `GRIST_API_KEY`        | —                                              | Grist API key (Bitwarden `grist.api-key`)                              |
| `GRIST_ROADMAP_DOC`    | `czYHYKgG9SHcm9CPwiKEq1`                       | The `Program roadmap` document id (Phase 2 seed)                       |
| `PM_PROJECTOR_URL`     | `http://127.0.0.1:7801` is J's planned default | The projector + link registry base URL (for `/items`, `/comments`)     |

Secrets are read from the environment only and are never committed or
printed. Names here reference Bitwarden entries (`kaneo.*`, `grist.*`).

## What each facet reads

| Facet            | Live reader (when `PM_AGG_SOURCE=live`)                     | Fallback                            |
| ---------------- | ----------------------------------------------------------- | ----------------------------------- |
| Board            | Kaneo REST (`/api/task/tasks/<project>`, all four projects) | fixtures                            |
| Roadmap          | Grist REST (`/api/docs/<doc>/tables/Roadmap/records`)       | fixtures (the real Grist seed rows) |
| Items (refs)     | Projector `GET /items?refs=…`                               | fixtures                            |
| Comments         | Projector `GET /comments?refs=…`                            | fixtures                            |
| Quota (#851)     | (projector concern — not yet projected)                     | fixtures                            |
| Attention (#852) | (projector/watch-cron concern — not yet projected)          | fixtures                            |

The footer of every page states, facet by facet, whether it is live or
fixture-backed. When a live reader is unreachable, that facet alone
falls back to fixtures and the footer says why.

## What is fixture-backed vs live

- **Fixture-backed today:** items, comments, quota, and attention. The
  fixtures are shaped exactly like the live readers' output and sit
  behind the same `Source` interface (`src/source.ts`), so swapping is a
  config change, not a code change. Roadmap fixtures are the real Grist
  seed rows; the design record's known refs (#752, #754, #762, #763,
  nix #501–#508, opencode #365) are real; the rest are illustrative.
- **Live-capable today:** board (Kaneo) and roadmap (Grist) when the
  services are reachable on this host and the env vars are set. The
  projector's `/items` and `/comments` are wired but require Session J's
  service (`pm/projector`) to be running and `PM_PROJECTOR_URL` set.
  The live board is the curated in-flight set (in-progress / in-review /
  recent done columns; the to-do backlog is excluded). It is genuinely
  cross-repo only once the remaining repos are connected as Kaneo
  projects (Session H, `pm/connect-repos`) — today Kaneo has the `code`
  project, so GitHub-only work from the other three repos appears on the
  board through the fixtures until the projector/registry or H's
  connections feed them.

## Design notes — the two new views (#851, #852)

Both were added as **design issues** (2026-10-25) and are implemented
here as machine-first surfaces with a GUI over the same data.

- **#851 quota + throughput:** the data model is `QuotaPool` /
  `QuotaWindow` / `BurnSegment` / `QuotaView` (`src/model.ts`); wall
  detection and near-limit thresholds live in `src/derive.ts`
  (`poolState`, `NEAR_PCT = 85`). The per-source readers listed in the
  issue (Codex, OpenCode Go, CommandCode, Cline, DeepSeek, Cloudflare
  GraphQL, Turso, Meta, Antigravity, Open Design, GitHub billing) are a
  **projector concern** — the aggregator renders whatever the projector
  emits, so none of those OAuth/API credentials are needed here. No
  estimator duplicates the OpenChamber quota picker; picker values are
  logged, not re-estimated (standing rule 2026-10-24).
- **#852 needs-you:** the state taxonomy and classification rules are
  `AttentionState` + `classifyAttention`/`attentionSorted` in
  `src/derive.ts`. Ordering is stopped-for-question and errored first,
  then needs-poke / stuck-retrying / stalled-on-CLI, then in-motion;
  every non-in-motion entry carries its evidence inline. The raw feed
  is the projector's concern (it generalizes the id-train watch crons);
  the aggregator classifies and renders whatever it emits.

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
  sources/
    fixtures.ts   fixture data (real Grist seed + design-record refs)
    live.ts       Kaneo/Grist/projector readers with per-facet fallback
test/
  render.test.ts  unit derivations + render smoke
  boot.test.ts    boots the server, serves all views from fixtures
```
