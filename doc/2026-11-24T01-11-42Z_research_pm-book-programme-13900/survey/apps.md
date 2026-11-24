# PM stack — integrated apps & components census (PM-SURVEY, 2026-11-23)

Ground truth: laptop clone `~/git/code` at `4d937b3` (2026-11-23), via `~/tools/laptop-ssh.sh`. "Last commit" is `git log -1` for that path on the laptop — evidence of repo activity, **not** of a running service. Runtime liveness was not probed in this survey; anything about a live process is marked UNVERIFIED unless a repo file states it as a dated record. Component count: **36 sections** (§1–§36), covering every `pm-stack/` subdirectory named in the brief plus the extra subdirectories and loose scripts actually present (`cloudflare-export/`, `meta-exporter/`, `private-laptop-bridge/`, `docs/`, `scripts/`, `tests/`).

Deployment frame for §1–§6 (`pm-stack/README.md`): one `nixos-rebuild switch` on laptop host `amd` deploys the stack; user units `pm-aggregator`, `pm-projector`, `pm-dispatch`, `pm-tunnel` are declared in `nixos/services/pm-stack`; Kaneo/Postgres/Pad/Grist are containers; ports are registered in `pm-stack/PORTS.md` (4870 aggregator, 4871 freshness IPC, 4872 console, 4879 dispatch, 5173 Kaneo, 6056 ready-for-agent, 7777 Pad, 7801 projector, 8484 Grist).

## 1. Kaneo — execution board (external app, our fork exists, upstream image deployed)
- What: Kanban web UI + API, container port 5173 bound to `127.0.0.1:5173`, public as `https://kaneo.security.cab` via the `amd-6-pm-stack` Cloudflare Tunnel (`pm-stack/compose.yml`, `pm-stack/README.md`, `pm-stack/systemd/pm-tunnel.service`). Postgres 16 sidecar, volume `pm-stack_postgres_data`, no host port.
- Version conflict — code wins: `pm-stack/compose.yml` pins the **upstream** image `ghcr.io/usekaneo/kaneo:2.47.0@sha256:3df62194…`; `pm-stack/aggregator/src/program.ts` claims the board runs "our fork (developing-today-forks/kaneo)". See `disagreements.md` D1.
- Projects: `pm-stack/kaneo-projects.json` — workspace "Developing Today", 4 projects, one repo each (upstream allows one repo per project): `code` → developing-today/code, `opencode` → developing-today-forks/opencode, `openchamber` → developing-today-forks/openchamber, `nix` → dezren39/darwin-nix; GitHub App id in that file, installation ids per project. `pm-stack/connect-repo.sh` reads it.
- Wiring in: the aggregator reads Kaneo **read-only** — sign-in via `/api/auth/sign-in/email`, then `/api/workspace`, `/api/project`, paginated `/api/task/tasks/<projectId>` (`pm-stack/aggregator/src/sources/live.ts`, `kaneoBoard()`); cards become board items with subject `kaneo:<slug>/<taskId>` and deep links built by `kaneoTaskUrl()` in `src/service-links.ts`. Board policy filters retired "ROADMAP Done table" cards (`src/board-policy.ts`); Kaneo columns map to canonical statuses in `src/status-taxonomy.ts` (to-do/in-progress/in-flight/in-review/done). Poll diffs feed activity/freshness (`kaneoChangeDeliveries`, gap/recovery tracking in `live.ts`).
- **The "Kaneo PM bot"**: no component in this repo is named that. The bot-like actor is **Kaneo's own GitHub integration** (the developing.today GitHub App, manifest `pm-stack/github-app-manifest.json`, webhook `https://kaneo.security.cab/api/github-integration/webhook`): it posts/decides by *status-transition automation*, not by comments. Per `pm-stack/KANEO-GITHUB-SYNC.md` (fleet record, defect #7610): on branch push → task `in-progress`; on PR open → `in-review`; on PR merge → `in-review` (changed 2026-11-14; upstream default `done` closed merely-referenced issues seconds after merge — #5866, #5767, openchamber#1831, #2381, #5772, #9060, #9078, #9106, #9568); on GitHub issue close → `done` (built-in fallback), and the reverse sync applies `status:done` labels / closes the GitHub issue when a task reaches Done. The settings live **only in Kaneo's Postgres** (`integration.config.statusTransitions` per project) — "nothing in this repository sets them" (same file). If a separate "Kaneo PM bot" (e.g. a Muse lane posting as a bot) exists outside this repo, it is UNVERIFIED — no file under `pm-stack/` or `pkgs/pm-*` names one.
- Guard: `pm-stack/kaneo-sync-guard.py` asserts every GitHub integration's transitions (no target `done`; merge→in-review, open→in-review, push→in-progress), heals drift in compact-JSON form, exits 1 loudly; run by `deploy.sh` and hourly by `pm-stack/systemd/pm-kaneo-sync-guard.{service,timer}`; tests `pm-stack/tests/test_kaneo_sync_guard.py`. Verified record in `KANEO-GITHUB-SYNC.md`: 2026-11-14 clean on all 4 integrations, induced drift healed in one run.
- Safety nets around the bot: `pm-stack/closure-watchdog.py` (§29) and `pm-stack/invariant-sweeper.py` (§30).
- Usage evidence: sync rule actively maintained (record updated 2026-11-22, guard last commit 2026-11-17); aggregator board provenance string is "live (Kaneo; done aged out after 7 days…)" when the poll succeeds (`live.ts`). A store-migration path off Kaneo Postgres exists: `pm-stack/scripts/kaneo-cutover.sh` + `pm-stack/STORE-MIGRATION-PLAN-6816.md` (gated Kaneo → Supabase cutover, presync/promote modes, fence + exact-parity gate).

## 2. Grist — roadmap tables (`pm-stack/grist/`)
- What: Grist OSS spreadsheet/database, container on `127.0.0.1:8484` + tailnet, no tunnel hostname (`pm-stack/README.md`). Chosen over Baserow (evaluation in the PM design record, per README).
- Version conflict: README says `gristlabs/grist-oss:v1.7.21` pinned by digest; `pm-stack/grist/compose.yml` line 28 is `gristlabs/grist-oss@sha256:d761de16…` with **no tag** — whether that digest is v1.7.21 is UNVERIFIED. See D2.
- Wiring in: the aggregator's **roadmap source of record alongside repo files** — `gristRoadmap()` in `aggregator/src/sources/live.ts` reads `GET <grist>/api/docs/<docId>/tables/Roadmap/records` with a bearer API key and maps records to `RoadmapRow` (Title/Area/Status/Horizon/Links, subject `grist:<docId>/Roadmap/<recId>`, `gristUrl` deep link). Config in `aggregator/src/config.ts` (`grist.baseUrl/docId/apiKey`).
- Seed & maintenance: `grist/seed/roadmap.json` (217 lines; also bundled into the pm-aggregator Nix package because `fixtures.ts` imports it by relative path — `pkgs/pm-aggregator/package.nix`), `grist/seed.sh`, `grist/deploy.sh`, `grist/reload-doc.sh`, `grist/update-choice-options.py` + `grist/CHOICE-OPTIONS.md` (keeps Grist choice columns in sync), `grist/validate-roadmap-seed.ts`, `grist/test_update_choice_options.py`.
- Usage evidence: last commit 2026-11-17; aggregator provenance requires the Grist read to be "live" for the roadmap reasonableness check (`server.ts`: `roadmapProv.startsWith("live")`). Repo-file roadmaps now compete with it — see `roadmap.md` (merge rule: repo file wins on title collision, #1738).

## 3. Pad — planning layer (`pm-stack/pad/`)
- What: Pad (perpetualsoftware/pad) v0.18.1, Go binary + SQLite, container `127.0.0.1:7777` + tailnet (`pad/compose.yml`, digest-pinned).
- Wiring in: deliberately thin. `pm-stack/README.md`: "the aggregator reads no Pad data — Pad is the planning layer only." The aggregator still carries Pad config (`PM_AGG_PAD_URL`, default `http://127.0.0.1:7777`, `aggregator/src/config.ts`), a Pad service link (`src/service-links.ts`: `pad` alongside kaneo/grist), a health/topology probe, and stack-token exchange accepts `pad` as a target (`server.ts`: "target must be kaneo, pad, or grist"). `PORTS.md` records the aggregator's `/api/health` reporting Pad up on 7777 (2026-11-05 audit) — health consumption, not data consumption. See D3.
- Content: only seed docs — `pad/seed/plans-pm-stack-design.md`, `plans-phase-1-as-built.md`, `plans-phase-2-plan.md`, `pad/seed.sh`. No code in this repo reads Pad plans back.
- Usage evidence: last commit 2026-11-17 (same commit as Grist — a bulk pin/deploy change, by timestamp). Actual human use of the Pad UI: UNVERIFIED from this repo.

## 4. ready-for-agent — dispatch UI layer (`pm-stack/ready-for-agent/`)
- What: upstream npm package `ready-for-agent@1.1.0` (no container image), UI + GraphQL on 6056, loopback + tailnet via socket proxy (`pm-stack/README.md`). Runs as a systemd user unit.
- Config: `ready-for-agent/config.json` — 4 repositories (code, opencode, openchamber, nix — the same four as Kaneo), each `defaultModel: "opencode-go/kimi-k3.1-pro"`, `mergePolicy: "OFF"` ("human merge always — never enable auto-merge here"), `includeAllIssueAuthors: true`, none paused. `install.sh` clones the repos; `apply-config.sh` registers them via the GraphQL API idempotently; tests `test-apply-config.sh`, `test-install.sh`; unit in `ready-for-agent/systemd/`.
- Wiring in: aggregator source adapter `aggregator/src/sources/ready-for-agent.ts`, config `PM_AGG_RFA_URL`. Usage evidence: last commit 2026-11-17; how much dispatch traffic actually flows through it vs `pm-stack/dispatch/` (§10): UNVERIFIED.

## 5. Aggregator (`pm-stack/aggregator/`) — the dashboard
Bun app, user unit `pm-aggregator`, port 4870; ~180 modules in `src/` (pages/health/quota/decisions/prompts/alerts/sources), `scripts/` (quota gather, digest, fleet-feed collector, charge candidates, decision-set producers), `opencode-tools/session-identity.ts`, systemd units in `aggregator/systemd/`, Nix package `pkgs/pm-aggregator/package.nix` (also wraps gather-quota, generate-digest, export-turso-stores, quota-weekly-review binaries). Full surface in `website-pages.md`. Last commit 2026-11-23 — the most active component in the stack.

## 6. Console (`pm-stack/console/`) — the phone/owner app
Bun app (port 4872, unit `pm-console`) + SPA + Cloudflare edge Worker (`console/edge/`, deployed from Actions as `pm-console-edge` per `platform/workers.json`) + Android TWA/APK (`console/android/`, built/released by `pm-stack/systemd/pm-console-android-release.{service,timer}`; the APK is served by the console itself). Modules: `ops.ts` (fleet verbs), `work.ts` (questions/needs/asks), `plan.ts` (roadmap/intake), `actions.ts` (audited action table), `push.ts`/`notify.ts`, `devices.ts`, `auth.ts`, `sync.ts` (edge-held actions), `docs.ts` (named docs incl. the questions doc). Full surface in `website-pages.md` §E–F. Last commit 2026-11-23.

## 7. Projector + link registry (`pm-stack/projector/`, `pkgs/pm-projector/`)
Bun service, port 7801. "It links and projects; it never becomes a fifth source of truth and never writes back to GitHub / Kaneo / Pad / Grist" (`projector/README.md`). Webhook intake `POST /github/webhook` (HMAC, delivery-GUID dedupe), reconcile polls (`src/reconcile.ts`, `src/cli.ts`), link registry `GET|POST|DELETE /links`, reads `GET /items`, `GET /comments`, `GET /subjects/<subject>`, SQL views `pmp_v_links_items` / `pmp_v_subject_activity`, schema `projector/schema.sql`, plus an HTML queue view (`src/queue-view{,-html}.ts`). Aggregator consumes it via `aggregator/src/sources/projector.ts` (links + latest comments per subject). Last commit 2026-11-23.

## 8. pm-dispatch (`pm-stack/dispatch/`, `pkgs/pm-dispatch/`)
Bun intake service, port 4879, public as `dispatch.security.cab`. Authenticated launch-or-enqueue: `POST /v1/sessions` creates the tracking GitHub issue (label `source/dispatch`), enqueues a repo-task/v1 queue task, creates an OpenCode session on the laptop server (127.0.0.1:13420), and immediately claims the task for `dispatch-opencode` with a 6 h lease (`dispatch/README.md`). A placement guard (`src/placement-guard.ts`) pins explicit OpenCode Go / ClinePass model hints to the active account's quota pool before launch and fails pre-commit on a walled pool. Example caller workflow: `dispatch/examples/dispatch-workflow.yml`. Last commit 2026-11-23.

## 9. pm-git-sync (`pm-stack/scripts/pm-git-sync.sh`, `pkgs/pm-git-sync/`)
Packaged bash (v0.3.0 in `pkgs/pm-git-sync/package.nix`): keeps the laptop's primary clone on a clean current `main` — fails loudly on tracked dirt, quarantines untracked files (never deletes), tags aside empty rebuild-marker commits, otherwise `fetch` + `pull --ff-only`. Since #8171 it never restarts PM services (deploy is by merge → `nixos-rebuild switch`).

## 10. decision-events-worker (`pm-stack/decision-events-worker/`)
Cloudflare Worker (`decision-events` in `platform/workers.json`: state "in service", deployed "by hand"). Decision-set activity events land in Turso (fleet-platform `decision_set_events`), in its D1 (`schema.sql`), and in a per-set Durable Object (`DecisionSetEvents`) that fans out live to WebSocket subscribers. Routes: `POST /events` (alias `/webhook`), `GET /sets/<setId>/events|state|stream`, `GET /health` — shared-secret auth, fail-closed with no secret. Producer: aggregator `scripts/sync-decision-events.ts`. Last commit 2026-11-17. Full flow in `push.md`.

## 11. notify-relay (`pm-stack/notify-relay/`)
Cloudflare Worker `pm-notify` — the dead-man's switch. Aggregator POSTs a ~60 s heartbeat + mirrored push subscriptions; a singleton Durable Object re-arms an alarm at last-beat + 150 s; on silence it sends exactly one `stack.silent` Web Push, on recovery exactly one recovered push. Step 1 only: event-mirror and edge answer-intake routes are **not routed yet** (`notify-relay/src/index.ts` header, `notify-relay/README.md`). `platform/workers.json` state: **"not deployed"**, address null. Last commit 2026-11-23. See `push.md` and D5.

## 12. health-hooks (`pm-stack/health-hooks/`)
Python runner `run.py` + wrappers `pm-stack-reachable.sh`, `opencode-working.sh` (#9571). Failure-triggered, read-only troubleshoot-and-report hooks: the aggregator's `scripts/stack-sentinel.ts` fires a hook once on a 3-consecutive-failure transition and once on recovery; secondary fire-and-forget callers are the claimant cap step / `launch_one.py` (`lane-launch`) and the aggregator live source (`aggregator-tick`). Incidents append JSONL to `~/.local/state/pm-aggregator/health-hook-incidents.jsonl` and best-effort POST to the alert throat with the `stackHealth` extension; one shared 60 s deadline; no remediation ships. Last commit 2026-11-22.

## 13. platform (`pm-stack/platform/`)
Not a service: the Workers registry — `workers.json` (20 Workers, checked 2026-11-23; see D6) + `README.md` (what "joined the platform" means: listed, Actions-deployed, version-reporting, named secrets, visible in the app) + `workers.test.ts` (holds the list to the repo's wrangler files). The console's Platform screen is drawn from this file. Last commit 2026-11-23.

## 14. voice-line-worker (`pm-stack/voice-line-worker/`)
Cloudflare Worker `voice-line` ("in service" in `platform/workers.json`): push-to-talk over a WebSocket in a Durable Object; Workers AI STT/TTS; answers are template-built from intent + a Turso queue snapshot — no LLM (see `models.md`). Last commit 2026-11-23.

## 15. exporters (`pm-stack/exporters/`)
Browser-dependent quota sources (#6032): watchdog core for sources reachable only through a logged-in website (Mistral first; Claude console and Open Design named as later adopters). `src/watchdog-state.ts` (failure classes + repair ladder, pure), `src/watchdog-escalation.ts` (rung-4 hand-off: incident issue first, then queue task, NEEDS DREW surfacing line), `src/adapters.ts` + `adapters/fixture.json` (the only adapter present is the synthetic fixture — real provider adapters are follow-ons, per its README), `src/watchdog.ts` (oneshot tick), `producers/producer-template.mjs`. Last commit 2026-11-23. Usage: the decision core is built and tested; live per-provider production use is UNVERIFIED (only the fixture adapter ships).

## 16. external-comms (`pm-stack/external-comms/`)
The external-comms ledger/pusher: SQL migrations `migration-0001-agent-external-comms.sql`, `migration-0002-…-turso.sql`, `record-external-send` CLI, a systemd pusher (`external-comms-pusher.{service,timer}`) that delivers rate-limited GitHub writes from the outbox, `backfill-2026-11-05.jsonl`, `test_push_quarantine.py`. Last commit 2026-11-21.

## 17. fleet-monitor (`pm-stack/fleet-monitor/`)
Two report-only Python monitors: `epic_close_check.py` (weekly adapter for the aggregator's native epic closure gate; a blocked epic is a finding, not a failure) and `review_loop_checks.py`; both with tests. Last commit 2026-11-22.

## 18. lane-guard (`pm-stack/lane-guard/`)
`lane_guard.py` — deterministic ownership/park-note checks for queue lanes: `hold` (only explicit markers count — `held`, `user/owned`, `do-not-auto-edit`; assignees and `work/assigned` never count), `note` (suppresses repeat park notes), built after repo-sweeper's blanket assignment caused 50+ duplicate park notes in two days (#7616, per its docstring). Pure functions over GitHub JSON + a `gh` wrapper; tests included. Last commit 2026-11-23.

## 19. opencode-claimant (`pm-stack/opencode-claimant/`)
The OpenCode execution lane (#3026): `claimant.py` (6-step fire: supervision/recovery of stuck bound sessions → candidate gate via `poll.py` across four queues → concurrency cap, 36 claimant roots since 2026-11-05 → launch+bind+prompt → logging to `~/.local/state/opencode-claimant/fires.jsonl`), `launch_one.py` (per-task immediate launch, #8187), `dispatch_batch.py`, `send_to_opencode.py`, `babysitter.py`, `nudges.py`, migrations `migration-0021…0026.sql`, `prompt.md`, a static `index.html`, and a large test suite. Last commit 2026-11-23 — actively developed (slice reports `SLICE1_*.md/txt` are historical).

## 20. opencode-scheduled-jobs (`pm-stack/opencode-scheduled-jobs/`)
One pilot job: `jobs.json` + `quota-freshness-summary` — a systemd-timer OpenCode session that summarises quota freshness; installer `install-laptop.sh` (timer enabled only with `--enable`; its README states "this pilot is currently enabled"), outputs `~/.local/state/opencode-scheduled/quota-freshness-summary/{latest.log,history.jsonl}`. Last commit 2026-11-22.

## 21. opencode-usage-export (`pm-stack/opencode-usage-export/`)
`opencode-usage-export.mjs` + service/timer: exports OpenCode usage from the session store (`opencode.db`, the #2072 exporter the scheduled-history collector also reads) for quota/usage surfaces. Last commit 2026-11-17.

## 22. provider-usage (`pm-stack/provider-usage/`)
`check-usage.py` — provider usage/OpenCode fleet snapshot for the "Jules keeper job": reads local state + provider endpoints, writes a snapshot JSON the keeper consults for failover model choice (paths point into a goal workspace — `~/workspace/goals/keep-the-four-repos-healthy/hidden_files/…`, i.e. this is a Muse-side job's tooling kept in the repo). `reconcile.py` + tests. Last commit 2026-11-23.

## 23. quota-check (`pm-stack/quota-check/`)
Single helper: `turso-mirror-read.py` (#6828) — per-DB libsql HTTP reads of the fleet-platform / fleet-mirror Turso DBs with per-DB tokens fetched by name (`fleet secrets get`, BWS fallback once), explicitly *not* the Turso CLI's org token. Last commit 2026-11-23.

## 24. scheduled-history (`pm-stack/scheduled-history/`)
`collect.py` — polling reconciler for OpenChamber scheduled-task runs (#2174): keys runs by (task id, lastScheduledFor) from `openchamber schedule list --json`, enriches token/cost from the OpenCode session store, freezes terminal rows; `watchdog.py`; service+timer units; tests. Last commit 2026-11-17.

## 25. shared-schema (`pm-stack/shared-schema/`)
The shared-store contract layer: `ORCHESTRATION.md` + `orchestration-writer.py` (one-transaction-per-pass writer for orchestration state to Turso fleet-platform, #5005/#3142), `migrations/`, `ainventory-turso-guards.sql`, migration/orchestration tests. Inventory context: `pm-stack/SHARED-SCHEMA-MAP.md` (see `roadmap.md`). Last commit 2026-11-23.

## 26. systemd (`pm-stack/systemd/`)
Unit files: `pm-aggregator`, `pm-console`, `pm-dispatch`, `pm-tunnel`, `pm-host-health` (+timer), `pm-invariant-sweeper` (+timer), `pm-kaneo-sync-guard` (+timer), `pm-quota-watchdog` (+timer), `pm-console-expose` (+timer), `pm-console-redeploy` (+timer), `pm-console-android-release` (+timer). Note: the NixOS module (`nixos/services/pm-stack`) is the deploy-time declaration for the main units per `pm-stack/README.md` — which of these repo copies are installed as-is vs rendered by Nix is UNVERIFIED.

## 27. cloudflare-export (`pm-stack/cloudflare-export/`) — not in the brief's list; present in the tree
`cloudflare-quota-export.mjs` + service/timer: exports Cloudflare quota/usage into the quota system. Last commit 2026-11-17.

## 28. meta-exporter (`pm-stack/meta-exporter/`) — not in the brief's list; present in the tree
`meta-quota-export.mjs` + service/timer: exports Meta quota/usage; its README notes a GraphQL/embed flow limitation (absent tab → `paygBalanceUsd: null`). Last commit 2026-11-21.

## 29. Loose script: `closure-watchdog.py`
Polls issue-close events in the Kaneo-covered repos and auto-reopens an automation close when the issue still has open sub-issues or the closing PR carried no closing keyword; legitimate (keyword/human) closes are untouched (docstring, #10105). `KANEO-GITHUB-SYNC.md` records it runs every 15 minutes **by a Muse sandbox cron job** — i.e. its scheduler lives outside the laptop stack; the original sandbox-only script was lost in the 2026-11-21 sandbox recovery and this repo copy (+ `pm-stack/tests/test_closure_watchdog.py`) is the durable replacement.

## 30. Loose script: `host-health-collector.py`
See `website-pages.md` §C. User service on a 5-minute timer (`pm-host-health.{service,timer}`); writes the JSON snapshot the aggregator derives watchdog tier items from (#10141).

## 31. Loose script: `invariant-sweeper.py`
Slow-cadence deep sweep across the four Kaneo-synced repos (#10139): (a) closed issue with non-completed sub-issues, (b) open issue carrying `status:done`, (c) Kaneo↔GitHub state mismatch. Default mode reports only; `--act` repairs (a)/(b) under the watchdog's guards and still only reports (c). Timer units in `systemd/`.

## 32. Loose script: `kaneo-sync-guard.py`
See §1 (Guard).

## 33. `pm-stack/scripts/` — operator scripts
`pm-git-sync.sh` (§9), `kaneo-cutover.sh` (§1), `fence-check.sh` (the cutover fence the cutover driver refuses to run without), `restore-rehearsal.sh`, plus smoke/fence/git-sync test scripts. Last commit 2026-11-23.

## 34. `pm-stack/private-laptop-bridge/` — not in the brief's list; present in the tree
Cloudflare-side bridge smoke tooling: `bootstrap-smoke.mjs`, `smoke.mjs`, `smoke-upload.mjs` + tests, `cloudflare.config.ts`. What production surface it serves beyond smoke tests: UNVERIFIED. Last commit 2026-11-23.

## 35. `pm-stack/docs/`
Only `pm-stack/docs/` content found: UNVERIFIED in detail — the directory's last commit is 2026-11-18; the design/docs corpus for the stack lives mostly at repo-root `doc/` and as loose `pm-stack/*.md` (§36).

## 36. Loose docs & config at `pm-stack/` root
`README.md` (deployment definition), `PORTS.md` (port registry, audited 2026-11-05), `compose.yml` + `deploy.sh` + `connect-repo.sh`, `github-app-manifest.json`, `BOOTSTRAP-CREDENTIALS.md`, `COLD-RESTORE-RUNBOOK.md`, `STORE-MIGRATION-PLAN-6816.md`, `SHARED-SCHEMA-MAP.md`, `SHARED-SCHEMA-MIGRATION-PLAN.md`, `decisions-ledger-system.md`, `needs-user-actions-design-2026-10-26.md`, `design-register-2026-11-11.md`, `design-tracking-manifest-2026-11-11.md`, `design-reprioritization-2026-11-12.md`, `lineage-manifest-2026-11-12.md`, `project-tracking-evaluation-2026-10-23.md`, `KANEO-GITHUB-SYNC.md`, `kaneo-projects.json`. Design docs are treated in `roadmap.md`; disagreements with code in `disagreements.md`.

### Nix packages summary (`pkgs/`)
`pkgs/pm-aggregator` (aggregator + 4 script binaries, bundles `kaneo-projects.json` and `grist/seed/roadmap.json`), `pkgs/pm-dispatch`, `pkgs/pm-projector`, `pkgs/pm-git-sync` (script wrapper, v0.3.0). There is **no** `pkgs/pm-console` — the console is not Nix-packaged in the tree surveyed (its deploy is `console/install.sh` / `redeploy.sh` / the edge release). UNVERIFIED: whether the NixOS module builds the console another way.
