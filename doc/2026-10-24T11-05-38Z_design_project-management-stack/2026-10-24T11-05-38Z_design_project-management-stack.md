# Project-management stack: layered tools, thin glue, one aggregator

Status: **approved shape (Drew, 2026-10-24); Phase 1 in progress.**
Deployment lives in [`pm-stack/`](../../pm-stack/) in this repository.

## Request and intent

Drew wants project management at two levels across the four working
repositories (`developing-today/code`, `developing-today/opencode`,
`developing-today/openchamber`, `dezren39/nix`): (1) a curated
work-in-progress board — features and fixes actually in flight, not
every open issue; and (2) a higher-level roadmap. After reviewing the
open-source options under his license rule (OSS, no AGPL/GPL; EPL
tolerated), the decision is a **composition of single-purpose tools
plus a thin layer of our own glue and UI**, rather than any one
all-in-one product (the deep-sync all-in-one candidate, Huly, needs
~14 services; the curated-board need is cross-repository, which no
candidate does natively).

Repository topology rule (Drew): **everything for this stack lives in
this repository** — design, deployment, glue, UI. The only work that
belongs in other repositories is a fork of an upstream project, if one
is ever needed. Secrets are SOPS-encrypted in this repo
(`secrets/pm-stack.sops.yaml`) and mirrored to Bitwarden + 1Password.

## Layers and ownership

Each layer has exactly one source of truth; nothing competes.

| Layer | Tool | Owns | Notes |
| --- | --- | --- | --- |
| Code work | **GitHub** | Issues, PRs, comments, reviews across all repos | The non-negotiable centre; everything else orbits it |
| Execution board | **Kaneo** (MIT, self-hosted) | Per-repo task boards | Two-way sync with GitHub (below); one project per repo |
| Dispatch | **ready-for-agent** (MIT) | Turning a labelled issue into an agent run + PR | The sweep fleet, productised; requires OpenCode 2 (the fleet now runs it) — later phase |
| Planning | **Pad** (Apache-2.0) | Plans, conventions, playbooks, ideas-in-progress | Local-first, per-agent write attribution, MCP server — later phase |
| Tables | **Grist** (Apache-2.0) or **Baserow** (core MIT) | Roadmap, idea register, inventories, trackers | Choice deliberately deferred — later phase |
| Glue | **ours** (this repo) | The link registry + projections | Webhook-fed projector; never a source of truth |
| Aggregator UI | **ours** (this repo) | The two views below | Read-mostly projection; disposable without data loss |

Licenses verified from source 2026-10-24: Kaneo MIT; ready-for-agent
MIT; Pad Apache-2.0; Grist Apache-2.0; Baserow core MIT (its
premium/enterprise trees are commercial and out of scope). Candidates
rejected on license alone included Plane (AGPL + paid GitHub
integration), Focalboard (AGPL source), Leantime (AGPL), Planka and
NocoDB (no longer OSS licenses).

## Why Kaneo fits the board layer (verified at v2.47.0)

Checked against Kaneo's docs and API source, not its marketing page:

- **Sync is genuinely two-way for issues**: comments (create-only —
  edits and deletions do not propagate), open/close in both
  directions, workflow columns carried by `status:*` labels synced
  both ways, `priority:*` and custom labels, title/body edits.
  Import brings existing open issues with labels and comments.
- **PRs are read-only to it**: it never comments on or edits a PR.
  PR/branch events only move the linked task (push → in-progress,
  PR opened → in-review, merged → done; mappings are configurable).
- **Fleet-relevant caveats**: comments authored by bot/app identities
  (Renovate, Dependabot) are skipped inbound (echo prevention) —
  acceptable because the fleet posts as Drew's user account, which
  syncs normally. Assignees do not sync. Comment edits go stale on
  the far side.
- **The structural gap**: one repository per Kaneo project
  (upstream usekaneo/kaneo#791, open, low priority). Four repos mean
  four projects — which is precisely the seam the aggregator UI
  exists to cover.
- Footprint: two containers (app + PostgreSQL 16), an official
  pinned image, a REST API and an MCP server — the fleet can read
  and write it natively.

Sync runs through **a GitHub App we register ourselves** (self-hosting
means we set its permissions: Issues read/write; Pull requests,
Contents, Metadata read; events Issues, Issue comments, Pull
requests, Push). Kaneo's hosted-product permission choices do not
bind us.

## Data flow

```text
GitHub (issues/PRs/comments, 4 repos)
   │  webhooks (our GitHub App)          ▲ sync (Kaneo's integration)
   ▼                                     │
Kaneo projects (one per repo) ───────────┘
   │  REST / MCP
   ▼
Projector (ours, small Worker/service) ──► link registry (Turso)
   ▲            joins by ID: issue URL ↔ Kaneo task ↔ Pad note ↔ table row
   │
Pad (plans) · Grist/Baserow (roadmap rows) · ready-for-agent (run state)
   │
   ▼
Aggregator UI (ours): cross-repo WIP board + roadmap view
```

The glue **links and projects; it never becomes a fifth source of
truth** and never performs clever two-way writes beyond what Kaneo
already does safely. If the UI rots, every layer underneath still
works standalone — that is the design's safety property.

## The two views (aggregator UI, last phase)

1. **Cross-repo WIP board** — the curated in-flight set across all
   four repos (Kaneo's four projects merged, plus anything tracked
   only in GitHub), with an execution strip: what is implementing,
   what awaits review, what merged.
2. **Roadmap view** — the table layer's roadmap rendered as a
   timeline (themes/quarters), each row drilling down to its GitHub
   issues and their live state from the projection.

## A day in the life (target state)

An idea matures in Pad → becomes a roadmap row → spawns issues in the
right repo → they appear on that repo's Kaneo board automatically →
an issue is labelled ready → ready-for-agent (or the sweep fleet)
implements it → the PR walks the Kaneo card to in-review → merge
closes the issue and ticks the roadmap row forward. Drew lives in
the aggregator UI; the fleet lives in GitHub, Kaneo, and Pad through
their APIs/MCP surfaces.

## Phases

- **Phase 1 (in progress)**: this design record; Kaneo in containers
  on the laptop (`pm-stack/`), public webhook endpoint via Cloudflare
  Tunnel (`kaneo.security.cab`), our GitHub App installed on
  `developing-today/code` only, end-to-end sync verification
  (import, webhook issue creation, comment both ways, close).
- **Phase 2**: connect the remaining repos (opencode, openchamber,
  nix) as their own Kaneo projects; ready-for-agent wired alongside
  the sweep fleet for label-driven dispatch.
- **Phase 3**: Pad as the planning layer, linked from issues.
- **Phase 4**: roadmap/register tables (Grist vs Baserow decided
  then, on trial), the projector + link registry, and the aggregator
  UI with the two views.

## Deployment shape: containers now, Kubernetes later

Phase 1 runs as Docker Compose on the Framework laptop — two
containers, data in a Docker volume on the host's persistent disk,
the connector for the tunnel as a systemd user unit. Nothing in the
app definitions is laptop-specific: host specifics are isolated in
`pm-stack/deploy.sh` (paths) and the unit file, secrets are rendered
from SOPS, and Kaneo's configuration is pure environment — so the
same values move to the basement-cluster Kubernetes (upstream Helm
chart + a Secret) when the cluster is ready, per the cluster goal.
A NixOS/home-manager module expressing the laptop deployment is a
known follow-up, tracked with the laptop's other declarative-module
debt; the compose definition in this repo is the source of truth
until then.

## Costs and risks

- Marginal cost $0: self-hosted on hardware we own; Cloudflare Tunnel
  on the existing Workers Paid plan; SOPS/age already in use.
- Maintenance: four upstreams to update (Kaneo pinned per release;
  never float on `:latest`).
- The discipline risk is the glue growing ambitions. It stays a
  projector + link table.
- Kaneo is a small-core project (2-person core). Its failure mode is
  cheap: MIT code, one PostgreSQL database, GitHub remains the
  truth — abandoning it loses a view, not data.

## Phase 1 as-built (2026-10-24)

Done and verified:

- `pm-stack/` in this repo: pinned compose (Kaneo 2.47.0 +
  PostgreSQL 16), `deploy.sh` (SOPS → rendered env → compose up →
  health gate), tunnel unit, runbook, GitHub App manifest.
- Secrets generated and stored three ways: `secrets/pm-stack.sops.yaml`
  (recipients: laptop + sandbox; new path rule in `.sops.yaml`),
  Bitwarden, and 1Password (Dev vault). Names: `kaneo.admin-password`,
  `kaneo.postgres-password`, `kaneo.auth-secret`,
  `kaneo.notification-encryption-key`,
  `cloudflare.tunnel.amd-6-pm-stack-token`.
- Running on the laptop (`~/pm-stack`): both containers healthy;
  Kaneo answers on `127.0.0.1:5173` only.
- Cloudflare Tunnel `amd-6-pm-stack` (remotely managed) +
  `kaneo.security.cab` CNAME: `https://kaneo.security.cab/api/health`
  returns ok from the public internet; the connector runs as the
  `pm-tunnel` systemd user unit.
- Kaneo bootstrapped via its API: admin account for Drew, workspace
  "Developing Today", project `code`; registration then closed
  (`DISABLE_REGISTRATION=true`) and the container recreated with the
  data intact.

Pending one browser step (GitHub offers no API for app creation):

- Register the GitHub App from `pm-stack/github-app-manifest.json`
  (manifest flow, org `developing-today`), exchange the returned code
  for the App ID / slug / PEM / webhook secret, drop them into the
  SOPS file, and install the app on `developing-today/code` only.
- Then: re-run `deploy.sh`, connect the project
  (`POST /api/github-integration/verify` + `.../project/{id}`),
  import issues (104 open at the time of writing), and run the
  four sync checks (GitHub issue → Kaneo task via webhook; comment
  each way; close from Kaneo). Results land in the Phase 1 PR and a
  follow-up note here.
