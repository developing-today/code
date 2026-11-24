# Running the stack as services

The PM stack deploys on laptop host `amd` with one `nixos-rebuild switch`, per `pm-stack/README.md`. User units run the first-party services; containers run Kaneo, Postgres, Pad, and Grist; Cloudflare Workers run the edge services. Ports are registered in `pm-stack/PORTS.md`. This chapter lists the services as deployed in the surveyed tree, and names the one gap in that picture.

## First-party user units

The deployment frame names four units declared in the NixOS module `nixos/services/pm-stack`:

| Unit | Service | Port |
|---|---|---|
| `pm-aggregator` | The aggregator dashboard (`pm-stack/aggregator/`) | 4870, plus freshness IPC on 4871 |
| `pm-projector` | The projector (`pm-stack/projector/`) | 7801 |
| `pm-dispatch` | The dispatch intake service (`pm-stack/dispatch/`) | 4879, public as `dispatch.security.cab` |
| `pm-tunnel` | The Cloudflare Tunnel unit (`pm-stack/systemd/pm-tunnel.service`) | — |

The console runs as unit `pm-console` on port 4872 (`pm-stack/console/`). It has no `pkgs/pm-console` package; its deploy runs through `console/install.sh` and `console/redeploy.sh` and the edge release path. Whether the NixOS module builds the console another way is UNVERIFIED from the surveys.

ready-for-agent runs as a systemd user unit from `pm-stack/ready-for-agent/systemd/`, serving its UI and GraphQL on port 6056.

## Unit and timer files in `pm-stack/systemd/`

The repo carries these unit files:

- `pm-aggregator`
- `pm-console`
- `pm-dispatch`
- `pm-tunnel`
- `pm-host-health` (service and timer) — the host-health collector, on a 5-minute timer.
- `pm-invariant-sweeper` (service and timer) — the invariant sweep across the Kaneo-synced repos.
- `pm-kaneo-sync-guard` (service and timer) — the sync guard, hourly.
- `pm-quota-watchdog` (service and timer)
- `pm-console-expose` (service and timer)
- `pm-console-redeploy` (service and timer)
- `pm-console-android-release` (service and timer) — builds and releases the console Android APK; the console itself serves the APK.

Units also live beside their components: `aggregator/systemd/` for the aggregator's units, `ready-for-agent/systemd/` for its unit, and `pm-stack/external-comms/` for the external-comms pusher service and timer.

Which of the repo copies are installed as-is and which are rendered by Nix is UNVERIFIED from the surveys. The NixOS module is the deploy-time declaration for the main units; the repo files are the record to read, with that caveat attached.

Since #8171, pm-git-sync never restarts PM services. Deploy is by merge, then `nixos-rebuild switch`.

## Containers

`pm-stack/compose.yml` and the per-app compose files run:

- **Kaneo** — container port 5173, bound to `127.0.0.1:5173`, public as `https://kaneo.security.cab` through the `amd-6-pm-stack` tunnel. The image is the upstream `ghcr.io/usekaneo/kaneo:2.47.0`, pinned by digest (D1).
- **Postgres 16** — Kaneo's sidecar, volume `pm-stack_postgres_data`, no host port.
- **Pad** — `127.0.0.1:7777`, plus tailnet.
- **Grist** — `127.0.0.1:8484`, plus tailnet, from `pm-stack/grist/compose.yml`. Its image line carries a digest and no tag (D2).

## Workers

The coverage ledger assigns 12 Workers to this book. Their deployment state, as surveyed:

| Worker | Config | State as surveyed |
|---|---|---|
| `decision-events` | `pm-stack/decision-events-worker/wrangler.toml` | In service; deployed by hand |
| `voice-line` | `pm-stack/voice-line-worker/wrangler.toml` | In service |
| `pm-notify` (notify-relay) | `pm-stack/notify-relay/wrangler.toml` | **Not deployed** — `platform/workers.json` records a null address (D5) |
| `pm-console-edge` | `pm-stack/console/edge/wrangler.jsonc`, plus `edge/init/wrangler.jsonc` | Deployed from Actions; its route is attached by the host script `scripts/expose.py` after a health check |
| `pkarr` | `pkgs/pkarr-worker/wrangler.jsonc` | Pkarr relay Worker, with its Durable Object store and rate limiters |
| `lab-ops-hub` | `pkgs/lab-ops-hub/wrangler.jsonc` | Worker with LiveRoom and CrawlerContainer Durable Objects, D1 `labops-db`, R2 `labops-files` |
| `cf-container-hello` | `pkgs/cf-container-hello/wrangler.jsonc` | Containers probe Worker |
| `muse-agent-handoff` | `scripts/muse/handoff/worker/wrangler.jsonc` | Handoff Worker, Handoff Durable Object |
| `github-ci` | `pkgs/github-ci/worker/` | CI helper Worker; the name comes from its directory and workflow — no wrangler config sits at the package root |
| exec door (security.cab) | — | No wrangler config found in the repo census; documented in home config and the #13404 / #13657 / #13774 issue family |
| pm-stack aggregator worker | — | Named by the #13844 audit as a Cloudflare Worker writing `fleet-platform`; no wrangler config exists in `pm-stack/aggregator/`, and its deploy path is UNVERIFIED |
| pm-stack dashboard worker | — | Named by the audit as a reader and writer of `fleet-platform` ingest families; no wrangler config found in the census |

The platform registry itself, `pm-stack/platform/workers.json`, lists 20 Workers across the estate (D6). The console's Platform screen at `#more/platform` is drawn from that file.

The D5 caveat governs the `pm-notify` row and every procedure that touches it: the relay's heartbeat target, its `stack.silent` push, and its answer-intake routes do not exist in production. Laptop silence sends nothing through the relay today. Procedures in this book that involve phone answers route them to the laptop aggregator. See [Health checks and the heal stack](health-and-healing.md) for the detection layer that does run.

## Operating the services from the console

The console's Services view, `#more/services`, lists the systemd units the console manages. Per unit it offers an action through `POST /console/api/services/&lt;unit&gt;/&lt;action&gt;` and logs through `GET /console/api/services/&lt;unit&gt;/logs?lines=`. The manageable set is an allowlist in `console/src/ops.ts`, in `manageable()`. A unit outside the allowlist is operated from the host, not from the phone.

## Port registry

Read `pm-stack/PORTS.md` for the registry. The load-bearing assignments: 4870 aggregator, 4871 freshness IPC, 4872 console, 4879 dispatch, 5173 Kaneo, 6056 ready-for-agent, 7777 Pad, 7801 projector, 8484 Grist. The registry's last audit date in the surveyed tree is 2026-11-05.

Sources. survey/apps.md §1–§8, §26, §36, and the Nix packages summary; coverage-ledger.md Table A; survey/disagreements.md D1, D2, D5, D6. Ground truth: laptop clone at 4d937b3, 2026-11-23.
