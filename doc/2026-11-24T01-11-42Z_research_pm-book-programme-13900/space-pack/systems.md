# Project Management Space — Systems: every component and worker

> Pack file 2 of 8 · Programme developing-today/code#13900 · Coordinator PM-BOOK
> Sources: `survey/apps.md` (36 components, §1–§36), `coverage-ledger.md`
> Table A (22 workers/doors), `survey/push.md`, `survey/models.md`,
> `survey/website-pages.md`, `survey/disagreements.md` (D1–D14).
> Ground truth: laptop clone `~/git/code` at `4d937b3` (2026-11-23).
>
> **Entry shape (Space-card-ready):** name | one-liner | detail paragraph |
> status. Status vocabulary per the pack brief: **live** / **undeployed** /
> **PoC**. Where the survey marks a fact UNVERIFIED, the entry says so —
> runtime liveness was not probed in the survey; "live" means the repo
> records the component as deployed / in service / actively maintained, not
> that a process was observed running. Book-coverage status is separate and
> lives in `coverage.md`.
>
> **Count:** Part A = the 36 components from `apps.md`. Part B = the 22
> workers from the coverage ledger. Four workers appear in both parts
> (decision-events, voice-line, pm-notify/notify-relay, pm-console-edge) —
> they are entered once in Part A and cross-referenced in Part B, so the
> unique total is **36 + 18 = 54 systems**.

---

## Part A — The 36 components (`survey/apps.md` §1–§36)

### A1. Kaneo — execution board

- **One-liner:** Kanban web UI + API; the execution board the aggregator reads.
- **Detail:** External app, container port 5173 bound to `127.0.0.1:5173`, public as `https://kaneo.security.cab` via the `amd-6-pm-stack` Cloudflare Tunnel; Postgres 16 sidecar. Deployment pins the **upstream** image `ghcr.io/usekaneo/kaneo:2.47.0` — the aggregator's program text claims "our fork"; code wins (D1). Four projects in `pm-stack/kaneo-projects.json` (workspace "Developing Today"): `code`, `opencode`, `openchamber`, `nix` — one repo each. The aggregator reads Kaneo **read-only** (sign-in, workspace/project/task APIs); cards become board items with subject `kaneo:<slug>/<taskId>`. The "Kaneo PM bot" is Kaneo's own GitHub integration (the developing.today GitHub App): status-transition automation only — branch push → `in-progress`, PR open → `in-review`, PR merge → `in-review` (changed 2026-11-14; upstream default `done` closed merely-referenced issues), issue close → `done`, and the reverse sync applies `status:done` / closes the issue at Done. Settings live only in Kaneo's Postgres. Guarded by the sync guard (A32), closure watchdog (A29), and invariant sweeper (A31). A gated Kaneo → Supabase store-migration path exists (`scripts/kaneo-cutover.sh`).
- **Status:** live

### A2. Grist — roadmap tables

- **One-liner:** Grist OSS spreadsheet/database holding roadmap tables.
- **Detail:** Container on `127.0.0.1:8484` + tailnet, no tunnel hostname. Chosen over Baserow. The aggregator's roadmap source alongside repo files: `gristRoadmap()` reads the `Roadmap` table (Title/Area/Status/Horizon/Links; subject `grist:<docId>/Roadmap/<recId>`). Seed in `grist/seed/roadmap.json` (217 lines, also bundled into the pm-aggregator Nix package); choice-column sync tooling in `grist/`. Version: README says `v1.7.21` pinned by digest; `grist/compose.yml` names no tag, only a digest — whether that digest is v1.7.21 is UNVERIFIED (D2). On title collisions with repo roadmap files, the repo file wins (#1738) — see `roadmap-mechanics.md`.
- **Status:** live

### A3. Pad — planning layer

- **One-liner:** Pad v0.18.1 planning UI; deliberately thin integration.
- **Detail:** Go binary + SQLite, container `127.0.0.1:7777` + tailnet, digest-pinned with tag. The aggregator reads **no Pad planning data** — Pad is the planning layer only. The aggregator still carries Pad config (`PM_AGG_PAD_URL`), a service link, a health/topology probe, and stack-token exchange accepts `pad` as a target (D3: "reads no Pad data" means no planning data; health is consumed). Content is seed docs only (`pad/seed/plans-*.md`); no code reads Pad plans back. Actual human use of the Pad UI: UNVERIFIED from the repo.
- **Status:** live

### A4. ready-for-agent — dispatch UI layer

- **One-liner:** Upstream dispatch UI + GraphQL over the four core repos.
- **Detail:** Upstream npm package `ready-for-agent@1.1.0` (no container image), UI + GraphQL on port 6056, loopback + tailnet via socket proxy, systemd user unit. Config (`ready-for-agent/config.json`): the same four repos as Kaneo, each `defaultModel: "opencode-go/kimi-k3.1-pro"`, `mergePolicy: "OFF"` (human merge always), `includeAllIssueAuthors: true`, none paused. `install.sh` clones the repos; `apply-config.sh` registers them via GraphQL idempotently. Aggregator source adapter `aggregator/src/sources/ready-for-agent.ts` (`PM_AGG_RFA_URL`). How much dispatch traffic flows through it vs pm-dispatch (A8): UNVERIFIED.
- **Status:** live

### A5. Aggregator — the dashboard

- **One-liner:** Bun dashboard; one state, many sources; the stack's front door.
- **Detail:** User unit `pm-aggregator`, port 4870. ~180 modules in `src/` (pages/health/quota/decisions/prompts/alerts/sources), `scripts/` (quota gather, digest, fleet-feed collector, charge candidates, decision-set producers), systemd units, Nix package `pkgs/pm-aggregator/package.nix` (also wraps gather-quota, generate-digest, export-turso-stores, quota-weekly-review). Publishes 29 routes with machine twins, the alert throat, the decisions store, and Web Push sending. Full surface in `survey/website-pages.md`. Makes no LLM/embedding calls (`survey/models.md`). Last commit 2026-11-23 — the most active component in the stack.
- **Status:** live

### A6. Console — the phone/owner app

- **One-liner:** Bun + SPA owner app; the phone surface for questions and actions.
- **Detail:** Port 4872, unit `pm-console`, base path `/console`. SPA (`public/index.html`, `public/app.js`, `public/outbox.js`, `public/sw.js`) with 21 hash views; Cloudflare edge Worker (`console/edge/`, deployed from Actions as `pm-console-edge`); Android TWA/APK (`console/android/`, built/released by a systemd timer; the APK is served by the console itself). Modules: `ops.ts` (fleet verbs), `work.ts` (questions/needs/asks), `plan.ts` (roadmap/intake), `actions.ts` (audited action table), `push.ts`/`notify.ts`, `devices.ts`, `auth.ts`, `sync.ts` (edge-held actions), `docs.ts`. Every action is audit-logged before it is taken; when the laptop is unreachable the edge holds the action and replays it through the same table. There is **no** `pkgs/pm-console` — deploy is `console/install.sh` / `redeploy.sh` / the edge release. Last commit 2026-11-23.
- **Status:** live

### A7. Projector + link registry

- **One-liner:** Bun service that links items across sources and projects views.
- **Detail:** Port 7801 (`pm-stack/projector/`, `pkgs/pm-projector/`). Webhook intake `POST /github/webhook` (HMAC, delivery-GUID dedupe), reconcile polls, link registry `GET|POST|DELETE /links`, reads `GET /items`, `GET /comments`, `GET /subjects/<subject>`, SQL views `pmp_v_links_items` / `pmp_v_subject_activity`, schema `projector/schema.sql`, HTML queue view (`src/queue-view{,-html}.ts` — route path UNVERIFIED). Never becomes a fifth source of truth; never writes back to GitHub / Kaneo / Pad / Grist. Aggregator consumes it via `aggregator/src/sources/projector.ts`. Last commit 2026-11-23.
- **Status:** live

### A8. pm-dispatch

- **One-liner:** Bun intake service: authenticated launch-or-enqueue of agent sessions.
- **Detail:** Port 4879, public as `dispatch.security.cab` (`pm-stack/dispatch/`, `pkgs/pm-dispatch/`). `POST /v1/sessions` creates the tracking GitHub issue (label `source/dispatch`), enqueues a repo-task/v1 queue task, creates an OpenCode session on the laptop server (`127.0.0.1:13420`), and immediately claims the task for `dispatch-opencode` with a 6 h lease. Placement guard (`src/placement-guard.ts`) pins explicit OpenCode Go / ClinePass model hints to the active account's quota pool before launch and fails pre-commit on a walled pool. Passes a caller-supplied model hint to OpenCode; makes no inference of its own. API-only, no web UI. Last commit 2026-11-23.
- **Status:** live

### A9. pm-git-sync

- **One-liner:** Packaged bash keeping the laptop's primary clone on a clean current `main`.
- **Detail:** `pm-stack/scripts/pm-git-sync.sh`, `pkgs/pm-git-sync/` (v0.3.0). Fails loudly on tracked dirt, quarantines untracked files (never deletes), tags aside empty rebuild-marker commits, otherwise `fetch` + `pull --ff-only`. Since #8171 it never restarts PM services — deploy is by merge → `nixos-rebuild switch`.
- **Status:** live

### A10. decision-events-worker

- **One-liner:** Cloudflare Worker fanning decision-set activity out live. [= ledger worker `decision-events`]
- **Detail:** `pm-stack/decision-events-worker/`; `decision-events` in `platform/workers.json`: state "in service", deployed "by hand". Decision-set activity events land in Turso (fleet-platform `decision_set_events`), in its D1 (`schema.sql`), and in a per-set Durable Object (`DecisionSetEvents`) that fans out live to WebSocket subscribers. Routes: `POST /events` (alias `/webhook`), `GET /sets/<setId>/events|state|stream`, `GET /health` — shared-secret auth, fail-closed with no secret. Producer: aggregator `scripts/sync-decision-events.ts`. Last commit 2026-11-17.
- **Status:** live

### A11. notify-relay

- **One-liner:** Cloudflare Worker dead-man's switch for "the laptop itself is silent". [= ledger worker `pm-notify`]
- **Detail:** `pm-stack/notify-relay/`, Worker name `pm-notify`. The aggregator POSTs a ~60 s heartbeat + mirrored push subscriptions; a singleton Durable Object re-arms an alarm at last-beat + 150 s; on silence it sends exactly one `stack.silent` Web Push, on recovery exactly one recovered push. **Step 1 only:** the event-mirror and edge answer-intake routes are **not routed yet** (`notify-relay/src/index.ts` header). `platform/workers.json` state: **"not deployed"**, address null (D5). With no `PM_AGG_NOTIFY_RELAY_URL` the aggregator side is inert and with no heartbeats the relay side is inert. Last commit 2026-11-23.
- **Status:** undeployed

### A12. health-hooks

- **One-liner:** Failure-triggered, read-only troubleshoot-and-report hooks.
- **Detail:** `pm-stack/health-hooks/`: Python runner `run.py` + wrappers `pm-stack-reachable.sh`, `opencode-working.sh` (#9571). The aggregator's `scripts/stack-sentinel.ts` fires a hook once on a 3-consecutive-failure transition and once on recovery; secondary fire-and-forget callers are the claimant cap step / `launch_one.py` and the aggregator live source. Incidents append JSONL to `~/.local/state/pm-aggregator/health-hook-incidents.jsonl` and best-effort POST to the alert throat with the `stackHealth` extension; one shared 60 s deadline; no remediation ships. Last commit 2026-11-22.
- **Status:** live

### A13. platform — the Workers registry

- **One-liner:** Not a service: the registry file the console's Platform screen is drawn from.
- **Detail:** `pm-stack/platform/`: `workers.json` (**20 Workers**, checked 2026-11-23 — D6) + `README.md` (what "joined the platform" means: listed, Actions-deployed, version-reporting, named secrets, visible in the app) + `workers.test.ts` (holds the list to the repo's wrangler files). The README's "eighteen Workers" figure is stale. Last commit 2026-11-23.
- **Status:** live (registry; not a running service)

### A14. voice-line-worker

- **One-liner:** Cloudflare Worker push-to-talk line; the stack's only in-tree inference. [= ledger worker `voice-line`]
- **Detail:** `pm-stack/voice-line-worker/`; Worker `voice-line`, "in service" in `platform/workers.json`. Push-to-talk over a WebSocket in a Durable Object; Workers AI STT (`@cf/openai/whisper-large-v3-turbo`) and TTS (`@cf/myshell-ai/melotts`) via `env.AI.run(...)`. The answer between STT and TTS is **not** an LLM: `matchIntent(transcript)` + `buildAnswer(intent, snapshot)` over a Turso queue snapshot — template answers for recognised intents (e.g. `queue_state`). Owns the `voice-line-ledger` Turso DB. Last commit 2026-11-23.
- **Status:** live

### A15. exporters — browser-dependent quota sources

- **One-liner:** Watchdog core for quota sources reachable only through a logged-in website.
- **Detail:** `pm-stack/exporters/` (#6032). `src/watchdog-state.ts` (failure classes + repair ladder, pure), `src/watchdog-escalation.ts` (rung-4 hand-off: incident issue first, then queue task, NEEDS DREW surfacing line), `src/adapters.ts` + `adapters/fixture.json`, `src/watchdog.ts` (oneshot tick), `producers/producer-template.mjs`. Mistral first; Claude console and Open Design named as later adopters. The decision core is built and tested; **the only adapter present is the synthetic fixture** — real provider adapters are follow-ons, so live per-provider production use is UNVERIFIED. Last commit 2026-11-23.
- **Status:** PoC (core built; fixture adapter only)

### A16. external-comms — ledger and pusher

- **One-liner:** The external-comms ledger/pusher for rate-limited GitHub writes.
- **Detail:** `pm-stack/external-comms/`: SQL migrations `migration-0001-agent-external-comms.sql`, `migration-0002-…-turso.sql`, `record-external-send` CLI, a systemd pusher (`external-comms-pusher.{service,timer}`) that delivers rate-limited GitHub writes from the outbox, `backfill-2026-11-05.jsonl`, `test_push_quarantine.py`. Last commit 2026-11-21.
- **Status:** live

### A17. fleet-monitor

- **One-liner:** Two report-only Python monitors over fleet/PM invariants.
- **Detail:** `pm-stack/fleet-monitor/`: `epic_close_check.py` (weekly adapter for the aggregator's native epic closure gate; a blocked epic is a finding, not a failure) and `review_loop_checks.py`; both with tests. Last commit 2026-11-22.
- **Status:** live

### A18. lane-guard

- **One-liner:** Deterministic ownership/park-note checks for queue lanes.
- **Detail:** `pm-stack/lane-guard/`: `lane_guard.py` — `hold` (only explicit markers count: `held`, `user/owned`, `do-not-auto-edit`; assignees and `work/assigned` never count), `note` (suppresses repeat park notes). Built after repo-sweeper's blanket assignment caused 50+ duplicate park notes in two days (#7616). Pure functions over GitHub JSON + a `gh` wrapper; tests included. Last commit 2026-11-23.
- **Status:** live

### A19. opencode-claimant — the claimant

- **One-liner:** The OpenCode execution lane that keeps claimed sessions honest and moving.
- **Detail:** `pm-stack/opencode-claimant/` (#3026). `claimant.py` — 6-step fire: supervision/recovery of stuck bound sessions → candidate gate via `poll.py` across four queues → concurrency cap (36 claimant roots since 2026-11-05) → launch+bind+prompt → logging to `~/.local/state/opencode-claimant/fires.jsonl`. Plus `launch_one.py` (per-task immediate launch, #8187), `dispatch_batch.py`, `send_to_opencode.py`, `babysitter.py`, `nudges.py`, migrations `migration-0021…0026.sql`, `prompt.md`, a static `index.html` (what it displays: UNVERIFIED), and a large test suite. Last commit 2026-11-23 — actively developed.
- **Status:** live

### A20. opencode-scheduled-jobs

- **One-liner:** Scheduled OpenCode jobs; one pilot job in production.
- **Detail:** `pm-stack/opencode-scheduled-jobs/`: `jobs.json` + `quota-freshness-summary` — a systemd-timer OpenCode session that summarises quota freshness; installer `install-laptop.sh` (timer enabled only with `--enable`; README states "this pilot is currently enabled"). Outputs `~/.local/state/opencode-scheduled/quota-freshness-summary/{latest.log,history.jsonl}`. Last commit 2026-11-22.
- **Status:** live (single pilot job, enabled)

### A21. opencode-usage-export

- **One-liner:** Exports OpenCode usage from the session store for quota/usage surfaces.
- **Detail:** `pm-stack/opencode-usage-export/`: `opencode-usage-export.mjs` + service/timer; reads the OpenCode session store (`opencode.db`, the #2072 exporter the scheduled-history collector also reads). Last commit 2026-11-17.
- **Status:** live

### A22. provider-usage

- **One-liner:** Provider usage / OpenCode fleet snapshot for the "Jules keeper job".
- **Detail:** `pm-stack/provider-usage/`: `check-usage.py` reads local state + provider endpoints and writes a snapshot JSON the keeper consults for failover model choice (paths point into a goal workspace — this is a Muse-side job's tooling kept in the repo). `reconcile.py` + tests. Last commit 2026-11-23.
- **Status:** live

### A23. quota-check

- **One-liner:** Single helper: per-DB Turso mirror reads for fleet-platform / fleet-mirror.
- **Detail:** `pm-stack/quota-check/`: `turso-mirror-read.py` (#6828) — libsql HTTP reads with per-DB tokens fetched by name (`fleet secrets get`, BWS fallback once), explicitly _not_ the Turso CLI's org token. Last commit 2026-11-23.
- **Status:** live

### A24. scheduled-history

- **One-liner:** Polling reconciler for OpenChamber scheduled-task runs.
- **Detail:** `pm-stack/scheduled-history/`: `collect.py` keys runs by (task id, lastScheduledFor) from `openchamber schedule list --json`, enriches token/cost from the OpenCode session store, freezes terminal rows (#2174); `watchdog.py`; service+timer units; tests. Last commit 2026-11-17.
- **Status:** live

### A25. shared-schema

- **One-liner:** The shared-store contract layer for orchestration state.
- **Detail:** `pm-stack/shared-schema/`: `ORCHESTRATION.md` + `orchestration-writer.py` (one-transaction-per-pass writer for orchestration state to Turso fleet-platform, #5005/#3142), `migrations/`, `ainventory-turso-guards.sql`, migration/orchestration tests. Inventory context: `pm-stack/SHARED-SCHEMA-MAP.md`. Last commit 2026-11-23.
- **Status:** live

### A26. systemd — unit files

- **One-liner:** The repo copies of the stack's systemd units and timers.
- **Detail:** `pm-stack/systemd/`: `pm-aggregator`, `pm-console`, `pm-dispatch`, `pm-tunnel`, `pm-host-health` (+timer), `pm-invariant-sweeper` (+timer), `pm-kaneo-sync-guard` (+timer), `pm-quota-watchdog` (+timer), `pm-console-expose` (+timer), `pm-console-redeploy` (+timer), `pm-console-android-release` (+timer). The NixOS module (`nixos/services/pm-stack`) is the deploy-time declaration for the main units — which repo copies are installed as-is vs rendered by Nix is UNVERIFIED.
- **Status:** live (declarations; Nix rendering split UNVERIFIED)

### A27. cloudflare-export

- **One-liner:** Exports Cloudflare quota/usage into the quota system.
- **Detail:** `pm-stack/cloudflare-export/` (present in the tree; not in the original survey brief's list): `cloudflare-quota-export.mjs` + service/timer. Last commit 2026-11-17.
- **Status:** live

### A28. meta-exporter

- **One-liner:** Exports Meta quota/usage into the quota system.
- **Detail:** `pm-stack/meta-exporter/` (present in the tree; not in the brief's list): `meta-quota-export.mjs` + service/timer. README notes a GraphQL/embed flow limitation (absent tab → `paygBalanceUsd: null`). Last commit 2026-11-21.
- **Status:** live

### A29. Loose script: `closure-watchdog.py`

- **One-liner:** Reopens automation closes that violate the epic-closure invariant.
- **Detail:** Polls issue-close events in the Kaneo-covered repos and auto-reopens an automation close when the issue still has open sub-issues or the closing PR carried no closing keyword; legitimate (keyword/human) closes are untouched (#10105). Runs every 15 minutes **by a Muse sandbox cron job** — its scheduler lives outside the laptop stack; the original sandbox-only script was lost in the 2026-11-21 sandbox recovery and this repo copy (+ `pm-stack/tests/test_closure_watchdog.py`) is the durable replacement.
- **Status:** live (scheduled from the Muse sandbox, not the laptop stack)

### A30. Loose script: `host-health-collector.py`

- **One-liner:** Laptop host-health snapshot the packaged aggregator cannot collect itself.
- **Detail:** User service on a 5-minute timer (`pm-host-health.{service,timer}`); writes the JSON snapshot the aggregator derives watchdog tier items from (#10141): claimant mode file, backup last-run, user-unit + timer states for pm-git-sync / pm-projector-reconcile / pm-kaneo-sync-guard / pm-dashboard-backup. Feeds the `/health` page assembly.
- **Status:** live

### A31. Loose script: `invariant-sweeper.py`

- **One-liner:** Slow-cadence deep sweep for Kaneo↔GitHub state invariants.
- **Detail:** Across the four Kaneo-synced repos (#10139): (a) closed issue with non-completed sub-issues, (b) open issue carrying `status:done`, (c) Kaneo↔GitHub state mismatch. Default mode reports only; `--act` repairs (a)/(b) under the watchdog's guards and still only reports (c). Timer units in `systemd/`.
- **Status:** live

### A32. Loose script: `kaneo-sync-guard.py`

- **One-liner:** Asserts and heals Kaneo GitHub-integration status transitions.
- **Detail:** Asserts every GitHub integration's transitions (no target `done`; merge→in-review, open→in-review, push→in-progress), heals drift in compact-JSON form, exits 1 loudly. Run by `deploy.sh` and hourly by `pm-stack/systemd/pm-kaneo-sync-guard.{service,timer}`; tests `pm-stack/tests/test_kaneo_sync_guard.py`. Verified record: 2026-11-14 clean on all 4 integrations, induced drift healed in one run. See A1.
- **Status:** live

### A33. `pm-stack/scripts/` — operator scripts

- **One-liner:** Operator scripts for sync, cutover, fencing, and restore rehearsal.
- **Detail:** `pm-git-sync.sh` (A9), `kaneo-cutover.sh` (A1), `fence-check.sh` (the cutover fence the cutover driver refuses to run without), `restore-rehearsal.sh`, plus smoke/fence/git-sync test scripts. Last commit 2026-11-23.
- **Status:** live

### A34. `pm-stack/private-laptop-bridge/`

- **One-liner:** Cloudflare-side bridge smoke tooling.
- **Detail:** `bootstrap-smoke.mjs`, `smoke.mjs`, `smoke-upload.mjs` + tests, `cloudflare.config.ts`. What production surface it serves beyond smoke tests: UNVERIFIED. Last commit 2026-11-23.
- **Status:** PoC (smoke tooling; production role UNVERIFIED)

### A35. `pm-stack/docs/`

- **One-liner:** The stack's in-tree docs directory.
- **Detail:** Content UNVERIFIED in detail in the survey — the directory's last commit is 2026-11-18; the design/docs corpus for the stack lives mostly at repo-root `doc/` and as loose `pm-stack/*.md` (A36).
- **Status:** live (docs; contents UNVERIFIED in detail)

### A36. Loose docs & config at `pm-stack/` root

- **One-liner:** Deployment definition, port registry, and the design-record corpus.
- **Detail:** `README.md` (deployment definition), `PORTS.md` (port registry, audited 2026-11-05), `compose.yml` + `deploy.sh` + `connect-repo.sh`, `github-app-manifest.json`, `BOOTSTRAP-CREDENTIALS.md`, `COLD-RESTORE-RUNBOOK.md`, `STORE-MIGRATION-PLAN-6816.md`, `SHARED-SCHEMA-MAP.md`, `SHARED-SCHEMA-MIGRATION-PLAN.md`, `decisions-ledger-system.md`, `needs-user-actions-design-2026-10-26.md`, `design-register-2026-11-11.md`, `design-tracking-manifest-2026-11-11.md`, `design-reprioritization-2026-11-12.md`, `lineage-manifest-2026-11-12.md`, `project-tracking-evaluation-2026-10-23.md`, `KANEO-GITHUB-SYNC.md`, `kaneo-projects.json`. Design docs are treated in `roadmap-mechanics.md`; disagreements with code in `survey/disagreements.md`.
- **Status:** live (docs/config)

### Nix packages summary (from `apps.md`)

`pkgs/pm-aggregator` (aggregator + 4 script binaries, bundles `kaneo-projects.json` and `grist/seed/roadmap.json`), `pkgs/pm-dispatch`, `pkgs/pm-projector`, `pkgs/pm-git-sync` (script wrapper, v0.3.0). There is **no** `pkgs/pm-console` (see A6). `pkgs/cf` is wrangler tooling, not a Worker. The `cloudflare-export` item in `pm-stack/` is a laptop systemd exporter, not a Worker.

---

## Part B — The 22 workers/doors (`coverage-ledger.md` Table A)

Book-coverage for every row is in `coverage.md`; this part records identity
and deployment status only. Rows marked [= A*n*] are the same system as the
Part A entry — not double-counted in the 54-system total.

### B1. `atlas-search`

- **One-liner:** Atlas search service Worker fronting the Turso corpus.
- **Detail:** `pkgs/atlas-search/worker/wrangler.toml`. Fronts the Atlas Turso corpus with a D1 hot cache (#9134). Covered by the atlas book (`concepts/service-and-worker.md`, `reference/worker-api.md`, `operations/services.md`).
- **Status:** live

### B2. `id-hosted-worker`

- **One-liner:** ID hosted-edition Worker (UserDO, fail-closed routing).
- **Detail:** `pkgs/id/hosted-worker/wrangler.toml`. Covered by the id book (`concepts/hosted.md`, `reference/hosted-worker.md`).
- **Status:** live

### B3. `message-bridge`

- **One-liner:** Bridge messaging Worker (MessageStore DO).
- **Detail:** `pkgs/message-bridge/worker/wrangler.jsonc`. Standalone build in flight, deployed by `message-bridge-deploy.yml`. Covered by the fleet book (`concepts/bridge.md`, `reference/bridge.md` — concept/CLI covered; Worker deployment detail is the fleet sideline extension).
- **Status:** live

### B4. `desktop-commander` — commander door

- **One-liner:** Desktop Commander MCP behind Access at `commander.security.cab`.
- **Detail:** `pkgs/desktop-commander/worker/wrangler.toml`, plus the commander-link bridge door (same Worker, not a separate Worker). Fleet-book gap; programme sideline assigns it to the fleet book.
- **Status:** live

### B5. `mcpx` — mcpx door

- **One-liner:** MCP gate/proxy at `mcpx.security.cab` with `?ns=` enforcement.
- **Detail:** `pkgs/mcpx/worker/wrangler.toml`, plus the mcpx-link bridge door (same Worker). The mcpx book's `remote.md` covers the daemon, not the Worker — book gap assigned to the mcpx book.
- **Status:** live

### B6. `mail` — id-mail worker

- **One-liner:** Address-confirmation mail worker for id.
- **Detail:** `nixos/services/id-mail/worker/`; routes `mail.security.cab`, `id-mail.security.cab`, …; retires `id-mail`. Id-book gap (no mail-worker chapter in the id book SUMMARY).
- **Status:** live

### B7. `id-worker-poc`

- **One-liner:** ID Worker proof-of-concept (World DO).
- **Detail:** `pkgs/id/worker-poc/wrangler.toml`. No covering chapter; id-book gap.
- **Status:** PoC

### B8. `id-d1-journal-poc`

- **One-liner:** ID D1 journal proof-of-concept Worker.
- **Detail:** `pkgs/id/worker-poc/d1-journal-poc/wrangler.toml`. No covering chapter; id-book gap.
- **Status:** PoC

### B9. `fleet-hosted`

- **One-liner:** Fleet hosted Worker (webhook-route dual ingestion).
- **Detail:** `pkgs/fleet/hosted-worker/wrangler.toml`. No hosted-Worker chapter in the fleet book SUMMARY — fleet-book gap.
- **Status:** live

### B10. `queue-transport-alert`

- **One-liner:** Alerts on queue-transport/broker route health via an Access service token.
- **Detail:** `pkgs/queue-transport-alert/worker/wrangler.jsonc`. No covering chapter — fleet-book gap.
- **Status:** live

### B11. `decision-events` [= A10]

- **One-liner:** PM decision-events Worker; writes `decision_set_events`, sibling D1 `decision-events`.
- **Detail:** See A10. PM-book gap (no covering chapter before this programme).
- **Status:** live

### B12. `voice-line` [= A14]

- **One-liner:** PM voice line Worker (speech route, #9800); owns `voice-line-ledger` DB.
- **Detail:** See A14. PM-book gap.
- **Status:** live

### B13. `pm-notify` (notify-relay) [= A11]

- **One-liner:** PM notify relay Worker — push/notification relay; it is a Worker.
- **Detail:** See A11. PM-book gap. Registry state "not deployed" (D5).
- **Status:** undeployed

### B14. `pm-console-edge`

- **One-liner:** PM console edge Worker serving the console SPA/API at `pm.security.cab`.
- **Detail:** `pm-stack/console/edge/wrangler.jsonc` + `edge/init/wrangler.jsonc`; route attached by host `scripts/expose.py` after a health check. Serves the same SPA/API when the laptop is away, holding actions until the host returns. Deployed from Actions as `pm-console-edge` per `platform/workers.json`. PM-book gap. (Component aspect: A6.)
- **Status:** live

### B15. `pkarr`

- **One-liner:** Pkarr relay Worker (PkarrStore DO, rate limiters, DoH).
- **Detail:** `pkgs/pkarr-worker/wrangler.jsonc`. PM-book gap.
- **Status:** live

### B16. `lab-ops-hub`

- **One-liner:** Lab ops hub Worker (LiveRoom + CrawlerContainer DOs).
- **Detail:** `pkgs/lab-ops-hub/wrangler.jsonc`; D1 `labops-db`, R2 `labops-files`. PM-book gap.
- **Status:** live

### B17. `cf-container-hello`

- **One-liner:** Cloudflare Containers hello/probe Worker (HelloContainer, CrawlerProbe).
- **Detail:** `pkgs/cf-container-hello/wrangler.jsonc`. PM-book gap.
- **Status:** PoC (hello/probe)

### B18. `muse-agent-handoff`

- **One-liner:** Agent handoff Worker (Handoff DO).
- **Detail:** `scripts/muse/handoff/worker/wrangler.jsonc`. PM-book gap.
- **Status:** live

### B19. `github-ci`

- **One-liner:** GitHub CI helper Worker.
- **Detail:** `pkgs/github-ci/worker/`; `github-ci-worker-tests.yml`. Name taken from its directory/workflow; no wrangler config at package root. PM-book gap.
- **Status:** live

### B20. exec door (security.cab)

- **One-liner:** Third security.cab door alongside commander/mcpx.
- **Detail:** Issues #13404/#13657/#13774 family. **No wrangler config found in the repo census**; documented only in home config/issues. PM-book gap. Deployment state UNVERIFIED from the repo.
- **Status:** undeployed (no deploy config found — UNVERIFIED; treat as not evidenced as deployed)

### B21. pm-stack aggregator worker

- **One-liner:** The aggregator named as a Cloudflare Worker by the #13844 audit.
- **Detail:** The audit names the aggregator as a Cloudflare Worker writing `fleet-platform` via the Turso HTTP API. **No wrangler config in `pm-stack/aggregator/` in the repo census** (deploy path unverified). Distinct from the Bun aggregator service (A5) unless a deploy record proves otherwise. PM-book gap.
- **Status:** undeployed (no deploy config found — UNVERIFIED)

### B22. pm-stack dashboard worker

- **One-liner:** The dashboard Worker named by the #13844 audit as reader/writer of `fleet-platform`.
- **Detail:** The audit names the dashboard Worker as reader/writer of `fleet-platform` ingest families. No wrangler config found in the repo census. PM-book gap.
- **Status:** undeployed (no deploy config found — UNVERIFIED)

---

## Registry note — two further names in `workers.json`

`platform/workers.json` lists 20 Workers (D6). Two names in that registry do
not appear in the ledger's 22-worker census and are recorded here so the
Space build does not lose them: `cold-store-rehydrator` and
`developing-today-backup`. The ledger (settled, 22 workers) is the
completion-test authority — see `coverage.md`. Commander/mcpx "link"
hostnames are bridge doors of the same two Workers, not separate Workers.
