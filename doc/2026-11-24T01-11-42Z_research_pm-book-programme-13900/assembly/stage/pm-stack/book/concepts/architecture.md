# Architecture

The PM stack is 36 components in the survey's count: every subdirectory of `pm-stack/` named in the programme brief, plus the subdirectories and loose scripts present in the tree that the brief did not name (`cloudflare-export/`, `meta-exporter/`, `private-laptop-bridge/`, `docs/`, `scripts/`, `tests/`). This chapter groups the 36 into the six parts of [The shape of the system](../getting-started/shape.md) and states, for each component, what it does and what it connects to. Detail lives in the concept chapter for each part; this chapter is the map.

Runtime liveness is not part of this map. The survey's ground truth is the repository at commit `4d937b3` (2026-11-23); a component's last-commit date is evidence of repository activity, not of a running service, and any claim about a live process is UNVERIFIED unless a repository file states it as a dated record.

## Core services

Four Bun services on the laptop host `amd` carry the stack. One `nixos-rebuild switch` deploys them; their user units are declared in `nixos/services/pm-stack`.

| Component | Port | Job |
|---|---|---|
| Aggregator (`pm-stack/aggregator/`, `pkgs/pm-aggregator`) | 4870 | The dashboard: about 180 modules covering pages, health, quota, decisions, prompts, alerts, and sources; plus scripts for quota gathering, the digest, the fleet-feed collector, charge candidates, and decision-set production |
| Console (`pm-stack/console/`) | 4872 | The phone and owner app: a Bun server, a single-page app, an edge Worker, and an Android wrapper. Its modules split by concern — `ops.ts` for fleet verbs, `work.ts` for questions and asks, `plan.ts` for roadmap and intake, `actions.ts` for the audited action table, `push.ts` and `notify.ts`, `devices.ts`, `auth.ts`, `sync.ts`, `docs.ts` |
| Projector (`pm-stack/projector/`, `pkgs/pm-projector`) | 7801 | Links and projections. See [The projector and projections](projector.md) |
| pm-dispatch (`pm-stack/dispatch/`, `pkgs/pm-dispatch`) | 4879 | Authenticated launch-or-enqueue intake for OpenCode sessions, with a placement guard that fails before commit on a walled quota pool |

Around them: `pm-git-sync` keeps the laptop's primary clone on a clean current `main` and never restarts services; the tunnel unit publishes Kaneo and dispatch; and the unit files in `pm-stack/systemd/` carry the timers for host health, the invariant sweeper, the Kaneo sync guard, the quota watchdog, and the console expose, redeploy, and Android-release jobs. Which of those repo copies are installed as-is and which are rendered by Nix is UNVERIFIED.

## External apps, integrated

Four applications the stack did not write provide its boards, tables, planning surface, and one dispatch UI. The aggregator links to all four and reads three of them.

- **Kaneo** — the execution board. Container on port 5173, Postgres 16 sidecar, public as `https://kaneo.security.cab`. The deployed image is upstream v2.47.0 pinned by digest in `pm-stack/compose.yml`; the fork at `developing-today-forks/kaneo` is not what compose runs (D1). Four projects, one repository each, listed in `pm-stack/kaneo-projects.json`. The aggregator reads Kaneo read-only; Kaneo's own GitHub integration moves cards on push, pull request, merge, and issue close by status-transition automation held in Kaneo's Postgres. See [The Kaneo sync and the other PM apps](kaneo-sync.md).
- **Grist** — the roadmap tables. Container on port 8484. The aggregator reads its `Roadmap` table as one roadmap source; on a title collision a repository roadmap file wins and the Grist row's links are unioned in (D12). The compose file pins Grist by digest with no tag; the README's v1.7.21 for that digest is UNVERIFIED (D2).
- **Pad** — the planning layer. Container on port 7777, Pad v0.18.1. The aggregator reads no Pad planning data (D3); its repository content is seed plans. Pad is in the architecture as a human planning surface the stack links to and health-checks.
- **ready-for-agent** — a dispatch UI layer. Upstream npm package on port 6056, configured for the same four repositories as Kaneo, with merge policy OFF. The aggregator has a source adapter for it. How much dispatch traffic flows through it rather than through pm-dispatch is UNVERIFIED.

## Edge Workers and the platform registry

`pm-stack/platform/` is not a service. It is the registry: `workers.json` lists 20 Workers, and `workers.test.ts` holds that list to the repository's wrangler files. The platform README's count of eighteen is stale (D6). The console's Platform screen is drawn from this file. Four of the 20 belong to this stack's own operation:

| Worker | State in `workers.json` | Job |
|---|---|---|
| `pm-console-edge` | Deployed from Actions | Serves the console when the laptop is away; holds actions and answers for replay |
| `decision-events` | In service, deployed by hand | Decision-set activity events to Turso, to its own D1, and to a per-set Durable Object that fans out live to WebSocket subscribers |
| `voice-line` | In service | Push-to-talk over a WebSocket in a Durable Object; Workers AI speech-to-text and text-to-speech; answers are template-built from intent and a Turso queue snapshot, with no LLM |
| `pm-notify` (notify relay) | Not deployed, address null | The dead-man's switch design: heartbeat in, one silence push out. Step 1 only in code; the event-mirror and edge answer-intake routes are not routed (D5) |

The other 16 Workers in the registry belong to the wider estate and are covered by their own books; the registry is the shared list, not a claim that this stack operates them.

## Execution

Work is done by sessions, and three components govern how sessions start and stay honest:

- **pm-dispatch** (above) is the synchronous door: tracking issue, queue task, OpenCode session on the laptop server at `127.0.0.1:13420`, immediate claim for `dispatch-opencode` with a 6-hour lease.
- **The OpenCode claimant** (`pm-stack/opencode-claimant/`) is the standing lane. Each fire runs six steps: supervise and recover stuck bound sessions; gate candidates through `poll.py` across four queues; apply the concurrency cap — 36 claimant roots since 2026-11-05; launch, bind, and prompt; and log the fire. `launch_one.py` is the per-task immediate-launch path. See [The claimant: keeping sessions honest](claimant.md).
- **Scheduled jobs** (`pm-stack/opencode-scheduled-jobs/`) run OpenCode sessions on systemd timers. One pilot job ships: `quota-freshness-summary`, whose installer enables its timer only with `--enable` and whose README states the pilot is currently enabled. Its outputs are a latest log and a history file. See [Scheduled jobs and the history they keep](scheduled-jobs.md).

Supporting execution components: `provider-usage` snapshots provider and fleet state for failover model choice; `opencode-usage-export` exports usage from the OpenCode session store; `scheduled-history` reconciles OpenChamber scheduled-task runs against that same store and freezes terminal rows.

## Guards, watchdogs, and safety nets

Automation that moves cards and closes issues is fenced by components whose only job is to check it:

- **Kaneo sync guard** (`pm-stack/kaneo-sync-guard.py`) asserts every GitHub integration's status transitions, heals drift, and exits 1 loudly. It runs at deploy and hourly.
- **Closure watchdog** (`pm-stack/closure-watchdog.py`) polls issue-close events in the Kaneo-covered repositories and reopens an automation close when the issue still has open sub-issues or the closing pull request carried no closing keyword. It runs every 15 minutes by a Muse sandbox cron job — its scheduler lives outside the laptop stack.
- **Invariant sweeper** (`pm-stack/invariant-sweeper.py`) sweeps the four Kaneo-synced repositories on a slow cadence for three invariants: a closed issue with non-completed sub-issues, an open issue carrying a done status, and a Kaneo–GitHub state mismatch. Default mode reports only; `--act` repairs the first two under guard and still only reports the third.
- **Lane-guard** (`pm-stack/lane-guard/`) makes deterministic ownership and park-note checks for queue lanes, built after a blanket assignment caused more than 50 duplicate park notes in two days.
- **Health hooks** (`pm-stack/health-hooks/`) are failure-triggered, read-only troubleshoot-and-report hooks. The aggregator's stack sentinel fires a hook once on a transition to three consecutive failures and once on recovery. Incidents append to a JSONL file and post, best-effort, to the alert throat. No remediation ships in a hook.
- **Host health collector** (`pm-stack/host-health-collector.py`) runs as a user service on a 5-minute timer and writes the JSON snapshot the aggregator derives watchdog tier items from.
- **Fleet monitor** (`pm-stack/fleet-monitor/`) is two report-only Python monitors, including the weekly adapter for the aggregator's epic closure gate: a blocked epic is a finding, not a failure.

## Quota, usage, and exporters

The quota page is fed by a family of gatherers and exporters: the aggregator's own quota modules and gather script; `cloudflare-export` and `meta-exporter`, each a laptop exporter with service and timer (the Meta exporter's README records a flow limitation — an absent tab yields a null PAYG balance); `pm-stack/exporters/`, the browser-dependent watchdog core for sources reachable only through a logged-in website, where the only adapter present is a synthetic fixture and live per-provider production use is UNVERIFIED; and `quota-check`, a single helper that reads the `fleet-platform` and `fleet-mirror` Turso stores over the libsql HTTP protocol with per-database tokens fetched by name — explicitly not the Turso CLI's organisation token.

## Ledgers, sync, and shared contracts

- **External comms** (`pm-stack/external-comms/`) is the ledger and pusher for sends outside the estate's own repositories: a record CLI, SQL migrations, and a systemd pusher that delivers rate-limited GitHub writes from the outbox.
- **Shared schema** (`pm-stack/shared-schema/`) is the shared-store contract layer: the orchestration writer, migrations, and guards for the Turso stores more than one component writes. See [The shared schema](shared-schema.md).
- **Decision events** flow from the aggregator's sync script to Turso, to the decision-events Worker's D1, and to its Durable Objects.
- **Private laptop bridge** (`pm-stack/private-laptop-bridge/`) is Cloudflare-side bridge smoke tooling — bootstrap and upload smoke tests. What production surface it serves beyond smoke tests is UNVERIFIED.
- **Docs and operator scripts.** `pm-stack/docs/` was not enumerated in detail by the survey — its content beyond a last commit of 2026-11-18 is UNVERIFIED — and the design corpus lives at the repository root's `doc/` and as loose documents at the `pm-stack/` root, including the README that defines deployment, the port registry, the store-migration plan, the shared-schema map, and the design register, tracking manifest, and lineage manifest of 2026-11-11 and 2026-11-12.

Sources. Survey file `~/workspace/pm-book/survey/apps.md` (§1–§36 complete, Nix packages summary); `~/workspace/pm-book/survey/disagreements.md` (D1, D2, D5, D6); `~/workspace/pm-book/survey/website-pages.md` (§C, §D); repo paths `pm-stack/`, `pm-stack/platform/workers.json`, `pm-stack/PORTS.md`, `pkgs/pm-aggregator/`, `pkgs/pm-dispatch/`, `pkgs/pm-projector/`, `pkgs/pm-git-sync/`.
