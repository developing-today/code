# Aggregator internals

The aggregator is a Bun application in `pm-stack/aggregator/`. It runs as the user unit `pm-aggregator` on port 4870, bound to loopback and the tailnet, and it is public only behind the console gate at `pm.security.cab`. Its Nix package is `pkgs/pm-aggregator/package.nix`. This chapter describes its module shape, its state handling, and its ingest families, at the depth the surveys support.

For the surface the aggregator serves, page by page, the reference chapters carry the census. This chapter is about the code under that surface.

## Module shape

The survey counts ~180 modules in `src/`. The surveys name these families:

- **Pages and rendering** — `pages.ts` (the page manifest: 23 pages, their nav groups, and the sub-route parent map), `render.ts` (the renderers), `server.ts` (routing, the HTTP and WebSocket surface, and inline client code for the answer pages).
- **Health** — `health.ts`, `stack-health.ts`, `host-health.ts`, `build-health.ts`, the `watchdog*.ts` and `slo*.ts` families. One health assembly feeds both `/health` and `/api/health`.
- **Quota** — the `quota-*.ts` family: current state, forecast, anomaly, reweight, admission, and weekly review; with the charges family (`charges-{catalog,collect,match}.ts`), the spend family (`spend-{attribution,cards,dollars,rollups}.ts`), `saas-extract.ts`, and `cost-anomaly.ts`.
- **Decisions and questions** — `decisions.ts` (decision sets and resolutions), `needs-drew.ts` (the needs-Drew census and the single-question store), `notifications.ts` (candidate builder and class registry), `alerts.ts` (the alert throat store and engine), `push.ts` (Web Push send).
- **Prompts** — `prompts.ts`, `prompt-ws.ts`, `prompt-notify.ts`, `prompt-widget.ts`, `prompt-attachments.ts`.
- **Actions and comments** — `actions.ts` (dashboard actions and their result path), `comments.ts` (dashboard-native comments and the queue of comments waiting on a loop).
- **Sources** — the `src/sources/` adapters, one per upstream system (next section).
- **Domain modules** — `model.ts` (the shared types: tier items, Project pool items, session records), `epics.ts` and `epic-reparent.ts` and `epic-graph.ts`, `weights.ts`, `projects.ts`, `program.ts`, `project-config.ts`, `project-doc.ts`, `roadmap-annotations.ts` and `roadmap-reconcile.ts`, `board-policy.ts`, `status-taxonomy.ts`, `service-links.ts`, `search-view.ts`, `aa.ts` and `aa-batches.ts`, `outage-queue.ts`, `config.ts`, `secrets.ts`, `stack-token.ts` and `break-glass.ts`, `notify-relay.ts` (the relay client), `decision-events.ts` and `decision-events-sync.ts`, `sw-source.ts` (the service worker source).

A module-by-module catalogue beyond these named families is UNVERIFIED from the surveys. The surveys name families and load-bearing files; they do not index all 180 modules.

Scripts live in `aggregator/scripts/`: quota gather and forecast snapshots, the digest producer, the fleet-feed collector, charge candidates, the decision-set producers (`raise-decision-set.ts`, `submit-decision-set.ts`), the decision-events sync, the stack sentinel, and the epic closure gate and reparent scripts. The Nix package wraps four script binaries beside the server: gather-quota, generate-digest, export-turso-stores, and quota-weekly-review. The package also bundles `kaneo-projects.json` and the Grist seed roadmap, because `fixtures.ts` imports the seed by relative path.

Session identity tooling sits in `aggregator/opencode-tools/session-identity.ts`.

## State handling

The aggregator holds no planning data of its own as a source of truth. Its state handling has four shapes:

1. **Projections from sources.** Pages render from projections the source adapters build. Provenance travels with the data: a roadmap read carries its source string, and the server checks it — the roadmap reasonableness check requires the Grist read's provenance to start with "live". The board's provenance string records a live Kaneo poll in its own words when the poll succeeds.
2. **Its own stores for its own surfaces.** Decision resolutions land in the decisions store (`recordResolutions` in `decisions.ts`, first resolution wins, drafts pruned on resolution). Alerts land in the Turso-backed alerts tables: incidents, push subscriptions, and the `notification_sends` log. Comments land in the comments store. Actions land in the actions store for consumers to poll.
3. **Conflicts surfaced, not resolved.** Where sources disagree — a Project Status against derived state, two roadmap rows with one title — the aggregator surfaces the disagreement on `/conflicts` with both values and its age. The one merge rule in code is the roadmap title rule: a repo-file row wins over a Grist row with the same title, the Grist row's links union into the survivor, and the duplicate is noted in provenance (#1738).
4. **Render accounting.** The #1147 render registry records each manifest page's last render and its outcome, and `/api/health` reports it per page. One gap stands in that accounting (D8): `/decisions/&lt;setId&gt;` is absent from `SUBROUTE_PARENTS`, so `manifestRouteFor` returns null for it and decision-set detail renders sit outside the registry.

Two manifest facts belong with state handling, because a reader of `pages.ts` alone gets them wrong:

- The `decisions` manifest entry declares no API twin. The server implements the decisions API regardless: submit, defer, request-info, and events per set, plus the decision-sets list and submit routes (D7).
- The single-question answer store is named two ways in code comments, and the exact file a single-question answer writes is UNVERIFIED (D9). The endpoint and the ledger application are the documented facts.

## Ingest families

The source adapters in `src/sources/` ingest from:

- **GitHub and the Project pool** — issues, pull requests, and Project #4 items, read through the fleet feed. Project fields arrive verbatim: Status, Area, Horizon, Kind.
- **Kaneo** — `live.ts`, `kaneoBoard()`: sign-in, workspace, projects, and paged tasks, read-only. Poll diffs feed activity and freshness, with gap and recovery tracking in the same module.
- **Grist** — `live.ts`, `gristRoadmap()`: the Roadmap table's records, mapped to roadmap rows with their deep links.
- **Repo files** — `sources/repo-roadmap.ts`: `planning/roadmaps/*.md` in developing-today/code and `ROADMAP.md` in dezren39/darwin-nix, with horizon validation in `roadmap-horizon.ts`.
- **The projector** — `sources/projector.ts`: links and the latest comments per subject. See [Projector internals](projector-internals.md).
- **ready-for-agent** — `sources/ready-for-agent.ts`, configured by `PM_AGG_RFA_URL`.
- **Pad** — no data ingest. The aggregator carries a Pad base URL in `config.ts`, a Pad service link, and a health and topology probe, and its stack-token exchange accepts `pad` as a target. It reads no Pad planning data (D3).
- **Provider usage endpoints** — the quota adapters (`claude-adapter.ts`, `clinepass-adapter.ts`, `token-adapter.ts`) read usage windows from provider endpoints. These are accounting reads. The aggregator makes no LLM calls and no embedding calls; see the models chapter of this book for the full statement.
- **Host state** — the snapshot `pm-stack/host-health-collector.py` writes on its 5-minute timer: claimant mode file, backup last-run, and user-unit and timer states for pm-git-sync, pm-projector-reconcile, pm-kaneo-sync-guard, and pm-dashboard-backup. The packaged aggregator cannot collect that state itself; the collector exists for that reason.
- **Webhooks and intake posts** — `POST /webhooks/github` (metadata webhook), `POST /api/alerts` (the alert throat, bearer intake token by secret name), `POST /api/push/subscriptions`, and the prompt and comment intake routes.

Freshness is a first-class output: `/freshness/stream` serves server-sent events, `/api/freshness-events.json` serves the same family as JSON, and feed section ages appear on `/health`.

## Configuration and secrets

`src/config.ts` holds the service configuration: source base URLs and credentials per source, including the Grist base URL, doc id, and API key, and the Pad base URL under `PM_AGG_PAD_URL` with default `http://127.0.0.1:7777`. Secrets arrive by name through `src/secrets.ts` — the alerts intake token as `pm-aggregator.alerts-intake-token`, the VAPID private key as `pm-aggregator.vapid-private-key`. Values are never in the chapter, the config file, or the logs this chapter describes.

Break-glass and stack-token exchange live in `src/stack-token.ts` and `src/break-glass.ts`, with `scripts/issue-break-glass-token.ts` as the issuing script.

Sources. survey/apps.md §2–§5, §36; survey/website-pages.md §A–§D, §F; survey/models.md §2; survey/disagreements.md D3, D7, D8, D9. Ground truth: laptop clone at 4d937b3, 2026-11-23.
