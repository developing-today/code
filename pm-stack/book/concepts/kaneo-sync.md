# The Kaneo sync and the other PM apps

## Kaneo: the execution board

Kaneo is the stack's execution board: a Kanban web UI and API in a container on `127.0.0.1:5173`, public as `https://kaneo.security.cab` through the `amd-6-pm-stack` Cloudflare Tunnel. A Postgres 16 sidecar holds its data in the `pm-stack_postgres_data` volume and exposes no host port.

The deployed image is the upstream image (D1). `pm-stack/compose.yml` pins `ghcr.io/usekaneo/kaneo:2.47.0` by digest. The aggregator's own program page text states that the board runs "our fork (developing-today-forks/kaneo) so multi-repo linking is on our schedule", and the `pm-stack/README.md` Phase-1 framing repeats the fork programme. The deployment file wins: the board that runs is upstream v2.47.0. The fork exists as a repository, and fleet references were repointed to it (code#5454); the fork is not what compose runs. This chapter follows the deployment file (D1).

Kaneo holds four projects in the workspace "Developing Today", one repository each, because upstream Kaneo allows one repository per project (`pm-stack/kaneo-projects.json`): `code` → developing-today/code, `opencode` → developing-today-forks/opencode, `openchamber` → developing-today-forks/openchamber, `nix` → dezren39/darwin-nix. `pm-stack/connect-repo.sh` reads that file to connect repositories.

## How the aggregator reads Kaneo

The aggregator reads Kaneo read-only. It signs in through `/api/auth/sign-in/email`, then reads `/api/workspace`, `/api/project`, and paginated `/api/task/tasks/\<projectId\>` (`aggregator/src/sources/live.ts`, `kaneoBoard()`). Cards become board items with subject `kaneo:\<slug\>/\<taskId\>`; deep links are built by `kaneoTaskUrl()` in `src/service-links.ts`. Board policy filters out retired "ROADMAP Done table" cards (`src/board-policy.ts`). Kaneo columns map to canonical statuses in `src/status-taxonomy.ts`: to-do, in-progress, in-flight, in-review, done. Poll diffs feed the activity and freshness machinery, including gap and recovery tracking. When a poll succeeds, the board provenance string reads "live (Kaneo; done aged out after 7 days…)".

A store-migration path off Kaneo Postgres exists and is gated: `pm-stack/scripts/kaneo-cutover.sh` and `pm-stack/STORE-MIGRATION-PLAN-6816.md` define a Kaneo-to-Supabase cutover with presync and promote modes, a fence, and an exact-parity gate.

## The "Kaneo PM bot"

No component in the repository carries that name. The bot-like actor is Kaneo's own GitHub integration: the developing.today GitHub App (manifest `pm-stack/github-app-manifest.json`, webhook `https://kaneo.security.cab/api/github-integration/webhook`). It acts by status-transition automation. It posts no comments and makes no decisions; it moves tasks when GitHub state changes (D1).

The transitions, per the fleet record in `pm-stack/KANEO-GITHUB-SYNC.md` (defect #7610):

- Branch push → task `in-progress`.
- PR open → task `in-review`.
- PR merge → task `in-review`. This is a deliberate change, made 2026-11-14. The upstream default, merge → `done`, closed merely-referenced issues seconds after merge; the record names the casualties: #5866, #5767, openchamber#1831, #2381, #5772, #9060, #9078, #9106, #9568.
- GitHub issue close → task `done`, by Kaneo's built-in fallback.
- Reverse direction: when a task reaches Done, the sync applies `status:done` labels and closes the GitHub issue.

The transition settings live only in Kaneo's Postgres, in `integration.config.statusTransitions` per project. Nothing in the repository sets them. If a separate "Kaneo PM bot" exists outside this repository — a Muse lane posting as a bot, for example — it is UNVERIFIED: no file under `pm-stack/` or `pkgs/pm-*` names one.

## The three guards

Three scripts keep the integration inside its rules.

**Sync guard** (`pm-stack/kaneo-sync-guard.py`). The guard asserts every GitHub integration's transitions: no transition targets `done`; merge → `in-review`; open → `in-review`; push → `in-progress`. It heals drift in compact-JSON form and exits 1 loudly on failure. `deploy.sh` runs it, and the `pm-kaneo-sync-guard` systemd timer runs it hourly. The verified record in `KANEO-GITHUB-SYNC.md` is dated 2026-11-14: clean on all 4 integrations, and induced drift healed in one run. Tests: `pm-stack/tests/test_kaneo_sync_guard.py`.

**Closure watchdog** (`pm-stack/closure-watchdog.py`). The watchdog polls issue-close events in the Kaneo-covered repositories and reopens an automation close when the issue still has open sub-issues or the closing PR carried no closing keyword. Legitimate closes — keyword closes and human closes — are untouched. Per `KANEO-GITHUB-SYNC.md`, the watchdog runs every 15 minutes by a Muse sandbox cron job; its scheduler lives outside the laptop stack. The original sandbox-only script was lost in the 2026-11-21 sandbox recovery; the repo copy and its tests are the durable replacement.

**Invariant sweeper** (`pm-stack/invariant-sweeper.py`). The sweeper runs a slow-cadence deep sweep across the four Kaneo-synced repositories and checks three invariants: (a) a closed issue with non-completed sub-issues, (b) an open issue carrying `status:done`, (c) a Kaneo↔GitHub state mismatch. Default mode reports only. With `--act` it repairs (a) and (b) under the watchdog's guards and still only reports (c). Its timer units live in `pm-stack/systemd/`. Invariant (a) is the same invariant the aggregator's epic closure gate enforces from the other side (see [The roadmap hierarchy: initiatives, projects, epics, aggregators](roadmap-hierarchy.md)).

## Grist: the roadmap tables

Grist is a spreadsheet-database (Grist OSS) in a container on `127.0.0.1:8484`, also reachable on the tailnet, with no tunnel hostname. The stack chose Grist over Baserow; the evaluation is in the PM design record, per the README.

Its version pin disagrees with its README (D2). The README's Phase-2 table states `gristlabs/grist-oss:v1.7.21`, "pinned by digest". `pm-stack/grist/compose.yml` names no tag at all: `gristlabs/grist-oss` pinned by digest only. Whether that digest is v1.7.21 is UNVERIFIED. This chapter follows the compose file (D2).

The aggregator reads Grist as a roadmap source: `gristRoadmap()` in `aggregator/src/sources/live.ts` reads `GET \<grist\>/api/docs/\<docId\>/tables/Roadmap/records` with a bearer API key and maps records to roadmap rows with subject `grist:\<docId\>/Roadmap/\<recId\>`. Seed and maintenance tooling lives in `pm-stack/grist/`: `seed/roadmap.json`, `seed.sh`, `deploy.sh`, `reload-doc.sh`, `update-choice-options.py` with `CHOICE-OPTIONS.md` (keeps Grist choice columns in sync), and `validate-roadmap-seed.ts`. Grist is one of three roadmap sources, and it loses title collisions to repo files; the merge rule is in [The roadmap hierarchy: initiatives, projects, epics, aggregators](roadmap-hierarchy.md) (D12).

## Pad: the planning layer

Pad (perpetualsoftware/pad) v0.18.1 is a Go binary with SQLite, in a container on `127.0.0.1:7777`, also reachable on the tailnet. Its compose file pins tag and digest together.

The wiring is deliberately thin, and one README sentence overstates it (D3). The README states: "the aggregator reads no Pad data — Pad is the planning layer only." The reconciled behaviour, which this chapter follows: the aggregator reads no Pad planning data — no source adapter consumes Pad content — and it does probe Pad for health and topology. The evidence for the probe half: `aggregator/src/config.ts` keeps a Pad base URL (default `http://127.0.0.1:7777`), `src/service-links.ts` lists `pad` as a service link alongside Kaneo and Grist, `server.ts` accepts `pad` as a stack-token exchange target, and `PORTS.md` records `/api/health` reporting Pad up on 7777 in the 2026-11-05 audit (D3).

Pad's content in the repo is seed only: `pad/seed/plans-pm-stack-design.md`, `plans-phase-1-as-built.md`, and `plans-phase-2-plan.md`, loaded by `pad/seed.sh`. No code in the repository reads Pad plans back. Actual human use of the Pad UI is UNVERIFIED from the repository.

## ready-for-agent: the dispatch UI layer

ready-for-agent is an upstream npm package (`ready-for-agent@1.1.0`, no container image) serving a UI and GraphQL API on port 6056, loopback and tailnet through a socket proxy, as a systemd user unit. Its config (`ready-for-agent/config.json`) registers the same four repositories as Kaneo. Each repository sets `defaultModel: "opencode-go/kimi-k3.1-pro"`, `mergePolicy: "OFF"` — the config's own words are "human merge always — never enable auto-merge here" — and `includeAllIssueAuthors: true`; none is paused. `install.sh` clones the repositories; `apply-config.sh` registers them through the GraphQL API idempotently. The aggregator reads it through a source adapter (`aggregator/src/sources/ready-for-agent.ts`). How much dispatch traffic flows through ready-for-agent against the stack's own dispatch service is UNVERIFIED. The dispatch service itself is treated in [Queues and dispatch methods, from the PM side](dispatch-methods.md).

## The projector, briefly

The projector (`pm-stack/projector/`, port 7801) links the apps in this chapter without becoming another source. Its README states its limit exactly: "It links and projects; it never becomes a fifth source of truth and never writes back to GitHub / Kaneo / Pad / Grist." It takes GitHub webhooks at `POST /github/webhook` (HMAC-checked, deduped by delivery GUID), reconciles by polling, and serves a link registry (`GET|POST|DELETE /links`) plus reads (`GET /items`, `GET /comments`, `GET /subjects/\<subject\>`). The aggregator consumes it through `aggregator/src/sources/projector.ts` for links and latest comments per subject.

## Deployment frame

One `nixos-rebuild switch` on laptop host `amd` deploys the stack. The user units `pm-aggregator`, `pm-projector`, `pm-dispatch`, and `pm-tunnel` are declared in `nixos/services/pm-stack`; Kaneo, Postgres, Pad, and Grist are containers. Ports are registered in `pm-stack/PORTS.md`: aggregator 4870, freshness IPC 4871, console 4872, dispatch 4879, Kaneo 5173, ready-for-agent 6056, Pad 7777, projector 7801, Grist 8484.

Sources. Survey `apps.md` §§1–4, 7; survey `roadmap.md` §2; survey `disagreements.md` D1, D2, D3, D12; code at laptop clone `~/git/code` at `4d937b3` as cited inline.
