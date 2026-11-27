# The workers

This chapter covers the Cloudflare Workers and doors of the estate as the coverage ledger assigns them. Fourteen are assigned to this book and receive full entries. The estate's other ten ledger rows are named in one table with their covering book and chapter. Every entry follows the same shape: what the worker is, its inputs and shape, its defaults where the survey gives them, its behaviour, and its failure shapes where the survey records them. Census date 2026-11-23; worker sources are wrangler configs and `.github/workflows/` on the laptop clone, `pm-stack/platform/workers.json`, and the coverage ledger.

## Count reconciliation (D6, D17, D41)

Three counts exist, and they count three different sets:

- **22 — the census count.** The coverage ledger's Table A censuses 22 workers and doors from wrangler configs, workflow files, the `pm-stack/` tree, and the security.cab door configs.
- **20 — the platform registry count.** `pm-stack/platform/workers.json` lists 20 workers: pm-console-edge, message-bridge, decision-events, pm-notify, mcpx, queue-transport-alert, github-ci, voice-line, muse-agent-handoff, desktop-commander, fleet-hosted, atlas-search, cold-store-rehydrator, developing-today-backup, pkarr, id-hosted-worker, id-worker-poc, id-d1-journal-poc, lab-ops-hub, cf-container-hello. Its own test (`workers.test.ts`) holds the JSON to the repo's wrangler files, so the JSON is the live registry list. The console's Platform view is drawn from this file.
- **18 — the platform README count.** `pm-stack/platform/README.md` states the repository holds eighteen Cloudflare Workers and that `workers.json` lists all eighteen. D6: the README's figures (eighteen listed, fourteen answered, two Actions-deployed) are stale against the file's own `checked: 2026-11-23` date; the JSON wins.

The census and the registry are not nested sets (D17): they share only **18** names. The census includes four entries the registry does not list: the `mail` worker, the exec door, the pm-stack aggregator worker, and the pm-stack dashboard worker. The registry includes two workers the original census table did not carry as rows: `cold-store-rehydrator` and `developing-today-backup`, censused by lane MISSED-WORKERS (survey `survey/missed-workers.md`) and given full entries in this chapter.

The final arithmetic, stated once and plainly (D41): **the ledger's 24 rows are 21 true Workers plus three rows that are not Workers** — the exec door (a tunnel ingress, not a Worker), the pm-aggregator (a laptop service, not a Worker), and the dashboard row (a duplicate: the dashboard IS the aggregator UI, and its only Worker, `pm-console-edge`, is a separate row in this chapter). The original census's 22 rows are therefore 19 Workers plus those same three non-worker rows; the registry lists 20; the census and the registry intersect at 18. The three non-worker verdicts are evidenced in survey `survey/worker-evidence.md` (the weak-evidence rows, D18, resolved there); their entries below describe what they actually are.

## `decision-events`

- **What it is:** The PM stack decision-events Worker (`pm-stack/decision-events-worker/wrangler.toml`). `platform/workers.json` records its state as "in service", deployed by hand.
- **Inputs and shape:** Routes are `POST /events` (alias `/webhook`), `GET /sets/&lt;setId&gt;/events`, `GET /sets/&lt;setId&gt;/state`, `GET /sets/&lt;setId&gt;/stream`, and `GET /health`. Authentication is a shared secret.
- **Behaviour:** Decision-set activity events land in three places: Turso (`fleet-platform`, table `decision_set_events`), the Worker's own D1 database (`schema.sql`), and a per-set Durable Object (`DecisionSetEvents`) that fans out live to WebSocket subscribers. The producer is the aggregator's `scripts/sync-decision-events.ts`, which drains the decision-event outbox to Turso and to this Worker.
- **Failure shapes:** Fail-closed with no secret configured.

## `voice-line`

- **What it is:** The PM stack voice line Worker (`pm-stack/voice-line-worker/wrangler.toml`; "in service" in `platform/workers.json`). It owns the `voice-line-ledger` Turso database.
- **Inputs and shape:** Push-to-talk runs over a WebSocket in a Durable Object. Speech-to-text and text-to-speech are Workers AI calls.
- **Behaviour:** The answer between transcription and speech is template-built from intent plus a Turso queue snapshot — no LLM call. The models are specified in [Model dependencies](models.md).
- **Failure shapes:** None recorded in the survey.

## `pm-notify`

- **What it is:** The notify relay Worker (`pm-stack/notify-relay/wrangler.toml`, worker name `pm-notify`): the dead-man's switch for the push system.
- **Inputs and shape:** The aggregator posts a heartbeat of about 60 seconds (`NOTIFY_RELAY_HEARTBEAT_MS`) and mirrors push subscriptions to the relay. A singleton Durable Object re-arms an alarm at last-beat + 150 seconds.
- **Behaviour:** On silence the relay sends exactly one `stack.silent` Web Push; on recovery, exactly one recovered push. The `stack.silent` pushes ride the `incident` preference class.
- **Deployment state (D5):** Step 1 only. The event-mirror route and the edge answer-intake routes are not routed yet (`notify-relay/src/index.ts` header), and `platform/workers.json` records `pm-notify` as "not deployed" with a null address. With no `PM_AGG_NOTIFY_RELAY_URL` set, the aggregator side is inert; with no heartbeats, the relay side is inert. Phone answers go to the laptop aggregator today.

## `pm-console-edge`

- **What it is:** The PM console edge Worker (`pm-stack/console/edge/wrangler.jsonc`, plus `edge/init/wrangler.jsonc`). Deployed from Actions as `pm-console-edge` per `platform/workers.json`.
- **Inputs and shape:** Sources are `console/edge/src/worker.ts`, `entry.ts`, `edge.ts`, and `direct.ts`. The route is attached by the host's `scripts/expose.py` after a health check.
- **Behaviour:** It serves the same console SPA and API at `pm.security.cab` when the laptop is away: actions are held by the edge and replayed through the console's audited action table when the host returns. `direct.ts` is the direct-to-host implementation; `edge.ts` is the held-action implementation.
- **Health contract (PR #16170):** `GET /console/health` is unauthenticated and reports `{version, deployedAt, ageSeconds, host}` plus the state body (`console/edge/src/entry.ts`). `version` is the full deployed commit id stamped as the `VERSION` var; `deployedAt` is the `DEPLOYED_AT` var stamped at deploy, normalised to ISO; `ageSeconds` counts seconds since deploy. Missing or future stamps report `deployedAt: null` and `ageSeconds: null` — never invented. The release gate and deploy workflow require the exact SHA with a fresh age. This is the edge Worker's health surface; the laptop console server's `GET /console/health` (`console/src/server.ts`) still returns `{ok:true}` and is documented under Console server routes.
- **Failure shapes:** UNVERIFIED: the exact edge-only route table, beyond the roles of `edge.ts` and `direct.ts` recorded in the survey.

## `pkarr`

- **What it is:** The Pkarr relay Worker (`pkgs/pkarr-worker/wrangler.jsonc`).
- **Inputs and shape:** A `PkarrStore` Durable Object, rate limiters, and DNS-over-HTTPS.
- **Behaviour:** The relay stores and serves Pkarr records under its rate limits.
- **Failure shapes:** None recorded in the survey.

## `lab-ops-hub`

- **What it is:** The lab ops hub Worker (`pkgs/lab-ops-hub/wrangler.jsonc`).
- **Inputs and shape:** Two Durable Objects, `LiveRoom` and `CrawlerContainer`; a D1 database, `labops-db`; an R2 bucket, `labops-files`.
- **Behaviour:** The hub combines live-room state and a crawler container behind one Worker.
- **Failure shapes:** None recorded in the survey.

## `cf-container-hello`

- **What it is:** The Cloudflare Containers hello and probe Worker (`pkgs/cf-container-hello/wrangler.jsonc`).
- **Inputs and shape:** Two container classes: `HelloContainer` and `CrawlerProbe`.
- **Behaviour:** A hello and probe surface for Cloudflare Containers: it exists to exercise the container platform, not to serve product traffic.
- **Failure shapes:** None recorded in the survey.

## `muse-agent-handoff`

- **What it is:** The agent handoff Worker (`scripts/muse/handoff/worker/wrangler.jsonc`).
- **Inputs and shape:** A `Handoff` Durable Object holds handoff state.
- **Behaviour:** Agent handoffs pass through this Worker's Durable Object.
- **Failure shapes:** None recorded in the survey.

## `github-ci`

- **What it is:** The GitHub CI helper Worker (`pkgs/github-ci/worker/`; workflow `github-ci-worker-tests.yml`).
- **Evidence caveat (ledger):** The name is taken from its directory and workflow; no wrangler config exists at the package root in the repo census.
- **Behaviour:** A CI helper for GitHub workflows.
- **Failure shapes:** None recorded in the survey.

## Exec door (security.cab) — not a Worker

- **What it is:** A path-scoped Cloudflare Tunnel ingress hostname, `exec.security.cab`, fronting the laptop's fleet queue broker — the public rendezvous for `fleet actions exec` (#13774), where GitHub Actions runner agents dial out and register with the broker. It is the third security.cab door alongside the commander and mcpx doors (#13404, #13657, #13774 family), but unlike those two it has no Worker in its path at all.
- **Inputs and shape:** The remotely managed sandbox-link tunnel carries an ingress rule for paths matching `^/actions-exec-(register|poll|io|result|close)$` on `exec.security.cab`, forwarded to the queue broker at `100.99.113.103:4801`. In code the door is a broker URL, not a Worker: `pkgs/fleet/crates/fleet/src/actions.rs` sets `DEFAULT_BROKER_URL` to `https://exec.security.cab`.
- **Behaviour:** There is no Access app in front, by design: the runner job's GitHub OIDC token is the authentication, verified at the broker. Path scoping is enforced at the tunnel — a tokenless `POST /actions-exec-poll` returns the broker's own 401, and other paths on the host return 404 (verified from outside in the #13774 Slice 5 deploy record).
- **Evidence verdict (D18):** NOT-A-WORKER (survey `survey/worker-evidence.md` §1). No wrangler config exists for it anywhere in the repo, and none could: the tunnel is remotely managed, so its ingress lives in Cloudflare, not in the repository. It leaves the worker census; its documentation stays in this book as a tunnel door.

## The pm-aggregator — not a Worker

- **What it is:** The PM dashboard application itself: a Bun/TypeScript server on the laptop (`pm-aggregator-ui`), a read-mostly projection over the projector and link registry, Kaneo, Pad and Grist, with the alert and push engine inside it. The #13844 Turso audit names it as a Cloudflare Worker writing `fleet-platform` through the Turso HTTP API; the audit got the store and the API right and the runtime wrong — it does write `fleet-platform` through the Turso HTTP API, but from the laptop, not from a Worker.
- **Where it runs:** Laptop host `amd`, as the systemd user service `pm-aggregator` (NixOS-packaged from `pkgs/pm-aggregator`), listening on port 4870 (loopback and tailnet) with the freshness IPC listener on 4871. It has no public route of its own; its public face is the console and edge surface.
- **Evidence verdict (D18):** NOT-A-WORKER (survey `survey/worker-evidence.md` §2). `pm-stack/aggregator/` contains no wrangler config, and `pm-stack/platform/workers.json` lists no aggregator entry. The application is covered in [The aggregator website, page by page](website-pages.md) and [The aggregator: one state, many sources](../concepts/aggregator.md).
- **Failure shapes:** None recorded in the survey.

## The dashboard row — a duplicate, not a Worker

- **What it is:** Not a separate thing at all. "The dashboard" is the aggregator's other name — the same laptop service as the entry above. The #13844 audit split one laptop application into two Workers that do not exist: its owner list names the aggregator and the dashboard separately, and its §G2 names a "dashboard Worker" reading the ingest families.
- **Evidence:** `pm-stack/aggregator/README.md` line 3 calls the aggregator UI "the dashboard Drew lives in"; `pm-stack/PORTS.md` names the single 4870 row "pm-aggregator (dashboard HTTP)" — one row, one service, both names; the console's own header states "The dashboard (pm-aggregator) shows the fleet; this service acts on it", and its `dashboardUrl` defaults to `http://127.0.0.1:4870`.
- **Evidence verdict (D18):** NOT-A-WORKER — a duplicate row (survey `survey/worker-evidence.md` §3). The only Worker anywhere in the dashboard's public surface is `pm-console-edge`, already a separate entry in this chapter. The dashboard row leaves the worker census as a duplicate of the aggregator row, and its documentation gap is the aggregator's gap.

## `cold-store-rehydrator`

- **What it is:** The cold store rehydrator Worker (`pkgs/cold-store-reaper/worker/wrangler.toml`): the read path of the Google Drive cold store. `platform/workers.json` (group "data") records its state as "in service", deployed by hand, at `https://cold-store-rehydrator.developing-today.workers.dev`.
- **Inputs and shape:** Routes are `GET /health` (no auth) and `GET /fetch/<sha256>` (Bearer `REHYDRATOR_API_KEY`). Bindings are a D1 database, `cold-store-manifest` (binding `MANIFEST`), and a `BundleFlight` Durable Object that single-flights work per bundle. Secrets are set by the deploy app from Bitwarden by name: `REHYDRATOR_API_KEY`, `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`, and `GOOGLE_REFRESH_TOKEN` — the same Google grant the Python reaper in the same package uses.
- **Behaviour:** Given an object's sha256, the Worker looks the object up in the D1 manifest (objects → members → bundles), and the `BundleFlight` object pulls exactly that member's zstd frame out of its Drive bundle by byte range, decompresses it, verifies the sha256, and returns the original bytes. Successful fetches carry `X-Cold-Store-Sha256` and `X-Cold-Store-Bundle` headers. The manifest is kept current by the reaper's `cold-store sync-manifest`; the v1 slice serves the object on the same request — filling R2 first and flipping the manifest state back to `hot` belong to the caller's layer.
- **Failure shapes:** `401` without the API key; `404` for an unknown sha (and for every sha until `sync-manifest` has run against a manifest containing verified bundles); `502` on any Drive, decode, or hash failure — bytes are never returned unverified, so a sha256 mismatch after decompression is a 502, not a best-effort serve.

## `developing-today-backup`

- **What it is:** The GitHub backup webhook Worker (`infra/github-backup/worker/wrangler.toml`): the webhook half of the GitHub estate mirror system. `platform/workers.json` (group "data") records its state as "in service", deployed by hand, at `https://developing-today-backup.developing-today.workers.dev`. The rest of the system — the mirror scripts in `infra/github-backup/bin/` — is not a Worker: it runs on the laptop under systemd user timers (`nixos/home/github-backup.nix`).
- **Inputs and shape:** Routes are `POST /github` (GitHub webhook), `GET /queue` and `POST /queue/ack` (both Bearer-authenticated for the laptop drainer), and `GET /health`. Its one binding is a KV namespace, `GHB_EVENTS`, holding the queued webhook events with a seven-day TTL. Secrets are set by the deploy app from Bitwarden by name: `GITHUB_WEBHOOK_SECRET` (the same value as on the `developing-today-backup` GitHub App) and `DRAIN_BEARER`.
- **Behaviour:** The GitHub App (`developing-today-backup`, metadata and contents read, push and repository events) sends webhooks to this Worker; the Worker verifies the `X-Hub-Signature-256` HMAC and queues `{id, repo, event, action, ts}` in KV. The laptop drains the queue every five minutes and fetches just those mirrors (per-repo coalescing fifteen minutes), acking an event only when its fetch landed. Events are a freshness signal only: every mirror is also fetched daily regardless, and a weekly reconciliation sweep re-inventories GitHub against the mirror tree — webhooks may be missed; the sweep may not.
- **Failure shapes:** Fail-closed with no webhook secret configured (500, #6951); `401` on a bad webhook signature or a missing drain bearer; a webhook payload with no repository is answered 202 and queues nothing; a declared or streamed webhook body over 25 MB is refused (413) before it is buffered.

## The estate's other ten workers

| Worker                  | Covering book and chapter (per the ledger)                                                                                                                                                                                                                           |
| ----------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `atlas-search`          | Atlas book — `concepts/service-and-worker.md`, `reference/worker-api.md`, `operations/services.md`. Covered. Published at [https://developing-today.github.io/code/atlas/](https://developing-today.github.io/code/atlas/)                                           |
| `message-bridge`        | Fleet book — `concepts/bridge.md`, `reference/bridge.md` (concept and CLI covered; Worker deployment detail is the fleet sideline extension). Covered. Published at [https://developing-today.github.io/code/fleet/](https://developing-today.github.io/code/fleet/) |
| `id-hosted-worker`      | ID book — `concepts/hosted.md`, `reference/hosted-worker.md`. Covered.                                                                                                                                                                                               |
| `desktop-commander`     | Fleet book — `concepts/commander.md` (The Commander door: Desktop Commander over MCP), landed in this programme (PR #13915). Covered. Published at [https://developing-today.github.io/code/fleet/](https://developing-today.github.io/code/fleet/)                  |
| `mcpx`                  | mcpx book — `worker.md` (The door Worker at mcpx.security.cab), landed in this programme (PR #13906). Covered.                                                                                                                                                       |
| `fleet-hosted`          | Fleet book — `concepts/fleet-hosted.md` (The hosted edition: fleet-hosted), landed in this programme (lane FLEET-SIDELINE-2, PR #13978). Covered.                                                                                                                    |
| `queue-transport-alert` | Fleet book — `concepts/queue-transport-alert.md` (The queue watchdog: queue-transport-alert), landed in this programme (lane FLEET-SIDELINE-2, PR #13978). Covered.                                                                                                  |
| `mail` (id-mail worker) | ID book — `reference/mail-worker.md` (The mail Worker API), landed in this programme (PR #13914). Covered.                                                                                                                                                           |
| `id-worker-poc`         | ID book — `reference/worker-poc.md` (id-worker-poc: the World Durable Object proof of concept), landed in this programme (PR #13914). Covered.                                                                                                                       |
| `id-d1-journal-poc`     | ID book — `reference/d1-journal-poc.md` (id-d1-journal-poc: the D1 journal proof of concept), landed in this programme (PR #13914). Covered.                                                                                                                         |

Sources: PM-CENSUS coverage ledger Table A (wrangler configs and `.github/workflows/` on the laptop clone, HEAD `4d937b36409`), PM-SURVEY survey files `apps.md` (§10, §11, §13, §14), `push.md`, `website-pages.md`, and `disagreements.md` (D5, D6); survey `missed-workers.md` (the two D17 entries and the set comparison) and survey `worker-evidence.md` (the three non-worker verdicts, D18), census date 2026-11-23.
