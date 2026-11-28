# Project-management stack: layered tools, thin glue, one aggregator

Status: **approved shape (Drew, 2026-10-24); Phase 1 complete
(PR #752); Phase 2 complete 2026-10-24 — dispatch, planning, and
table layers live; connecting the remaining repos to Kaneo is in
progress (projects created, app installs owner-gated — see the
connect-repos as-built); aggregator UI scoped, not yet built.**
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

| Layer           | Tool                         | Owns                                             | Notes                                                                                                                      |
| --------------- | ---------------------------- | ------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------- |
| Code work       | **GitHub**                   | Issues, PRs, comments, reviews across all repos  | The non-negotiable centre; everything else orbits it                                                                       |
| Execution board | **Kaneo** (MIT, self-hosted) | Per-repo task boards                             | Two-way sync with GitHub (below); one project per repo                                                                     |
| Dispatch        | **ready-for-agent** (MIT)    | Turning a labelled issue into an agent run + PR  | The sweep fleet, productised; requires OpenCode 2 (the fleet runs it). Live on the laptop since Phase 2; manual merge only |
| Planning        | **Pad** (Apache-2.0)         | Plans, conventions, playbooks, ideas-in-progress | Local-first, per-agent write attribution, MCP server. Live on the laptop since Phase 2                                     |
| Tables          | **Grist** (Apache-2.0)       | Roadmap, idea register, inventories, trackers    | Grist vs Baserow decided in Phase 2 (evaluation below); live on the laptop                                                 |
| Glue            | **ours** (this repo)         | The link registry + projections                  | Webhook-fed projector; never a source of truth                                                                             |
| Aggregator UI   | **ours** (this repo)         | The two views below                              | Read-mostly projection; disposable without data loss                                                                       |

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
  exists to cover. We are not waiting on upstream to close it:
  see "Decided: we fork Kaneo" under Phases.
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

### Aggregator requirements (scoped 2026-10-24, Phase 2)

Two requirements are first-class for the aggregator step — both
exist precisely because of a Kaneo limit, and both live in the
link registry + projector, not in any board tool (Drew,
2026-10-24):

1. **Cross-repo links.** A board item or task view in the
   aggregator must be able to link **any** GitHub issue or PR from
   **any** of the four repos and show it live — state, title,
   status — regardless of which repo the item itself belongs to.
   Kaneo cannot do this: it binds exactly one repository per
   project (upstream usekaneo/kaneo#791), so a `code` task can
   never natively reference an `opencode` PR. The aggregator's
   link registry is where cross-repo references live, and the
   projector keeps their displayed state current from GitHub.
2. **Comments across repos.** The aggregator must surface comment
   activity for linked issues and PRs across all repos in one
   place — a single activity stream per board item / roadmap row,
   not four repo-shaped silos. Today that exists nowhere: Kaneo
   syncs comments create-only, in both directions, but only
   within a project's own connected repo (proven in Phase 1 on
   issue #754), and PR-interior comments (reviews, line comments)
   sync nowhere at all. The projector reads them from GitHub
   directly; the aggregator renders, it does not write back.

Neither is built in Phase 2 — they are the UI step's defining
requirements, recorded now so the projector's link registry is
designed with the fields they need (repo-qualified issue/PR
references per item, and a comment-activity projection keyed by
them).

## A day in the life (target state)

An idea matures in Pad → becomes a roadmap row → spawns issues in the
right repo → they appear on that repo's Kaneo board automatically →
an issue is labelled ready → ready-for-agent (or the sweep fleet)
implements it → the PR walks the Kaneo card to in-review → merge
closes the issue and ticks the roadmap row forward. Drew lives in
the aggregator UI; the fleet lives in GitHub, Kaneo, and Pad through
their APIs/MCP surfaces.

## Phases

**Numbering note (2026-10-24):** the list below is the original
sketch, kept for the record. Execution reordered it: what shipped
as **Phase 2** is the dispatch layer (ready-for-agent) plus the
planning layer (Pad — the sketch's Phase 3) plus the table layer
(Grist — the first half of the sketch's Phase 4). Connecting the
remaining repos to Kaneo — the sketch's Phase 2 — is still
outstanding and is the next board step. The projector + link
registry and the aggregator UI remain the final step. The as-built
sections at the end of this record are authoritative; this list is
the sketch they diverged from.

- **Phase 1 (complete, PR #752)**: this design record; Kaneo in containers
  on the laptop (`pm-stack/`), public webhook endpoint via Cloudflare
  Tunnel (`kaneo.security.cab`), our GitHub App installed on
  `developing-today/code` only, end-to-end sync verification
  (import, webhook issue creation, comment both ways, close).
- **Phase 2 (complete 2026-10-24)**: ready-for-agent dispatch
  (manual merge only) proven on a trial issue; Pad as the planning
  layer, seeded from this record; Grist as the table layer, seeded
  with a real roadmap; the Kaneo registration-posture fix (see the
  Phase 1 correction below).
- **Phase 2 in the original sketch (in progress)**: connect the
  remaining repos (opencode, openchamber, nix) as their own Kaneo
  projects — projects created 2026-10-25; the app installs are
  owner-gated (see the connect-repos as-built at the end).
- **Phase 3 in the original sketch**: Pad as the planning layer,
  linked from issues. (Shipped inside Phase 2.)
- **Phase 4 in the original sketch**: roadmap/register tables
  (Grist vs Baserow decided then, on trial — decided and shipped
  inside Phase 2), the projector + link registry, and the
  aggregator UI with the two views. The projector + UI remain the
  final step, scoped under "The two views" above.

## Decided: we fork Kaneo (Drew, 2026-10-24)

Rather than wait on upstream for the GitHub-integration gaps, we
fork `usekaneo/kaneo` (MIT) under the `developing-today` org — our
schedule governs, not upstream's. Fork scope, as future work
(starting after the current Phase 2 layers land; **not** a
Phase 2 deliverable):

1. **Cross-repo support** — a project/board able to link issues
   and PRs from multiple repositories, superseding the
   one-repo-per-project limit (upstream usekaneo/kaneo#791 is the
   reference point).
2. **Deeper GitHub integration** — PR-interior sync (comments,
   reviews), and comment edit/delete propagation where feasible,
   closing the create-only gap Phase 1 verified around.

Topology follows the standing rule: the fork is upstream-fork
work, so it lives in its own repository (`developing-today/kaneo`);
deployment of our fork build still lives here, in `pm-stack/`
(the fork rev pinned like every other input). Upstream PRs back
to `usekaneo/kaneo` are welcome but optional. The aggregator
requirements above remain valid either way — the aggregator reads
GitHub and every layer regardless of what the board tool grows.

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

## Roadmap table layer: Grist vs Baserow (decided in Phase 2)

**Decision: Grist.** Evaluated against the roadmap/register need
(cross-repo milestones, the cluster idea register, goal-tracking
views) under Drew's license filter:

- **License shape.** `gristlabs/grist-oss` is a single clean
  Apache-2.0 image. Baserow's core is MIT, but the roadmap-critical
  views (Kanban, Calendar, Timeline), JSON/XML export, and RBAC
  live in its proprietary premium/enterprise trees — deleting
  those trees to stay MIT-only deletes the features the layer
  exists for. (Use the `grist-oss` image, not `gristlabs/grist`,
  which can toggle in a proprietary full edition.)
- **Self-host cost.** Grist is one container (~100–200 MB RAM for
  documents of our size). Baserow's all-in-one bundles PostgreSQL +
  Redis + Caddy + Celery (~2 GB, 2-core minimum), and Baserow
  itself "highly recommends" an external PostgreSQL.
- **Exportability.** A `.grist` file is plain portable SQLite —
  data, attachments, layout, and history in one file we can open
  anywhere. Baserow's free UI export is per-table CSV; its full
  export is a ZIP only Baserow can re-import.
- **API.** Baserow's one clear win: per-table scoped database
  tokens and a canonical OpenAPI spec. Grist's API key is
  all-or-nothing for its user; we mitigate by giving automation
  the dedicated install-admin account and nothing else. Grist's
  REST API is otherwise sufficient for the projector.
- **Known Grist gaps.** No native Kanban/Gantt views — acceptable
  by design, because the aggregator UI renders board and timeline
  itself from the projection rather than embedding the table tool.

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

Completed 2026-10-24 (full record: PR #752, comment
`issuecomment-6383895207`):

- GitHub App **developing.today** — slug `developing-today`,
  App ID 5454422, owned by the `developing-today` org (registered via
  the manifest flow, then renamed at the owner's request; the rename
  changed only name/slug). Credentials (App ID, client ID/secret,
  webhook secret, PEM private key) filed three ways and
  hash-verified in each: SOPS (`github_app_id`, `github_app_name`,
  `github_app_client_id`, `github_app_client_secret`,
  `github_webhook_secret`, `github_private_key_base64` — decrypt-
  and-hash verified under both age recipients, PEM byte-exact),
  Bitwarden, and 1Password (Dev vault), as `kaneo.github-app-*`.
  The staged conversion response was shredded after filing.
- Installation 174792676 on `developing-today/code` **only**
  (verified via the app API). Permissions Issues read/write;
  Pull requests, Contents, Metadata read; events Issues, Issue
  comments, Pull requests, Push; webhook
  `https://kaneo.security.cab/api/github-integration/webhook`
  (deliveries return 200; they were 400 before the webhook secret
  was configured).
- Project `code` connected and issues imported: **105 imported,
  0 updated, 0 skipped** — matching a live recount of open issues
  (the 104 quoted earlier in this record was already one stale).
  105 tasks + 105 issue links (+3 PR links); 19 pre-existing issue
  comments came across as activity entries. One implementation
  note: connecting a project requires the admin's linked GitHub
  account, and this instance reports `accountLinkingAvailable:
false` (GitHub SSO is not configured), so the link was asserted
  directly in Kaneo's database — a row recording the admin's true
  GitHub identity (`dezren39`) — after which
  `POST /api/github-integration/verify` independently confirmed the
  installation and permissions server-side.
- All four sync checks **passed** (throwaway issue #754 ↔ task
  CODE-106, kept closed/in-Done as the record): (a) issue created on
  GitHub → task in Kaneo via webhook; (b) comment in Kaneo →
  appeared on GitHub authored by `developing-today[bot]`;
  (c) comment on GitHub → appeared in Kaneo; (d) task moved to Done
  in Kaneo → GitHub issue closed (`state_reason: completed`).

### Correction (2026-10-24, found live the same day)

Registration did not stay closed. `deploy.sh` preserved
`DISABLE_REGISTRATION=true` only when the existing rendered `.env`
already carried it, and `.env.example` defaulted to `false` — so a
Phase 1 re-render silently dropped the closed posture and public
registration at `kaneo.security.cab` was open again. Fixed live
first (rendered env set to `true`, container recreated; a sign-up
probe now returns `REGISTRATION_NOT_ALLOWED` and health is ok),
then fixed in the repo with Phase 2: `deploy.sh` now renders
registration **closed by default**, and opening it — only ever to
bootstrap the first account — takes an explicit one-render opt-out
(`KANEO_OPEN_REGISTRATION=1 ./deploy.sh`); `.env.example` matches.

Recorded from the open window: the instance carries a second,
isolated workspace named **test**, owned by a temporary guest
account (`serene-monkfish`, created 2026-10-24 12:05Z, during the
open window / the owner's own exploring). Verified in the database:
it has **no membership in the Developing Today workspace** and no
access to the `code` project. Left in place unless the owner says
otherwise.

## Phase 2 as-built (2026-10-24)

All three remaining layers are live on the laptop (`amd-6`),
deployed from `pm-stack/` in this repo, alongside Kaneo (which
stayed healthy and untouched throughout).

### Dispatch — ready-for-agent 1.1.0

- Upstream ships no container image (an npm package booting a
  compiled Bun binary), so it runs as a systemd **user unit** in
  the same declarative style as `pm-tunnel`
  (`pm-stack/ready-for-agent/`; pinned `PINNED_VERSION=1.1.0`;
  installed into `~/.local` because the host's npm prefix is the
  read-only Nix store). UI + GraphQL on loopback `127.0.0.1:6056`
  only; state in `~/.local/share/ready-for-agent/` (SQLite).
- `developing-today/code` is registered from a full clone at
  `~/pm-stack/ready-for-agent/repos/code` and configured:
  **Merge Policy OFF** (human merge — the harness's safe default,
  pinned explicitly), **Include all Issue Authors** on, unpaused,
  build model `opencode-go/kimi-k3.1-pro` on the laptop's OpenCode
  backend (the fleet fork build v2.0.30; backend status READY).
  GitHub auth is the host's `gh` login (`dezren39`). No auto-merge
  path is enabled; per-item "Implement With" pins were deliberately
  not used.
- Registration quirk (recorded in `install.sh`): the
  `ready-for-agent add` CLI performs its git inspection in-process
  and returns `NOT_A_GIT_REPOSITORY` for valid repos when run from
  an interactive SSH shell environment on this host; the identical
  inspection succeeds from the service's clean environment.
  Registration was therefore done through the harness's own
  GraphQL API (`addLocalRepository`), which is also how the
  settings above were applied. The harness creates per-work-item
  worktrees under `/tmp/ready-for-agent/…` (its own choice).

### Planning — Pad 0.18.1

- Container `ghcr.io/perpetualsoftware/pad:0.18.1`, pinned by
  digest (`pm-stack/pad/`); data in a named volume. Bound to
  `127.0.0.1:7777` and the tailnet address `100.99.113.103:7777`
  only — **no tunnel hostname** (decided: planning content does
  not need a public endpoint; loopback + tailnet is the posture).
  Health: `GET /api/v1/health` → `{"status":"ok","version":"0.18.1"}`
  (`/healthz` only hits the SPA fallback).
- Workspace **Developing Today** (slug `developing-today`), admin
  Drewry Pope / drewrypope@gmail.com. The startup template created
  the collections the design implies: **tasks, ideas, plans, docs,
  conventions, playbooks**. Seeded with three Plan items: the
  PM-stack design & phase plan, the Phase 1 as-built, and the
  Phase 2 plan (seed content in `pm-stack/pad/seed/`).
- Bootstrap quirk: `pad init` must run **inside** the server
  container (`docker compose exec`) — bootstrap is loopback/
  token-gated. Seeding is pure REST (workspace-scoped routes) and
  idempotent, so it survives container recreates.
- Auth quirk: the `PAD_API_KEY` environment variable appears inert
  on 0.18.1 (Bearer with it fails "Invalid token format"); REST
  automation uses a minted user token (`pad_` prefix). The minted
  token `pm-stack-automation` (scopes `*`) **expires 2027-01-22**
  (90-day default) — renewal is owed before then. Secrets
  `pad.api-key`, `pad.admin-password`, `pad.api-token` are filed
  in SOPS + Bitwarden + 1Password, hash-verified.

### Tables — Grist 1.7.21

- Container `gristlabs/grist-oss:v1.7.21`, pinned by digest
  (`pm-stack/grist/`); data in a named volume at `/persist`. Bound
  to `127.0.0.1:8484` and the tailnet address `100.99.113.103:8484`
  only; no tunnel hostname. Healthy since first boot.
- Bootstrap is the boot-key flow (`POST /boot/login`); the boot
  key lives in the home DB (`/persist/home.sqlite3`,
  `activations` table) because the container emits no logs.
  Admin: drewrypope@gmail.com, install admin. An API key was
  minted via `POST /api/profile/apiKey` for automation.
- Seeded: workspace **Roadmap & registers**, document **Program
  roadmap** (`czYHYKgG9SHcm9CPwiKEq1`), table **Roadmap** (Title;
  Area + Status as Choice; Horizon/Source/Notes) with **16 real
  rows**, read back and verified: the PM-stack phases (5), the
  basement-cluster register headlines (6 — Talos/Cozystack via
  TALM, hardware inventory + firmware/BMC audit, storage tiers,
  day-one services, spare-PC runners/quorum witnesses, the ESP32
  wing), the cloud item (1 — OCI Always Free, blocked on
  CPCOE-29381), and the four-repo fleet (4, with open issue/PR
  counts as of 2026-10-24: code 109, opencode 27, openchamber 66,
  nix 201). Seeding is scripted (`seed.sh` + `seed/roadmap.json`)
  and reruns skip existing rows.
- API quirks on 1.7.21 (for the projector's benefit): `POST
/api/docs/{id}/apply` takes the bare UserAction array as its
  body (not `{"actions": …}`); create-workspace/doc endpoints
  return bare ids; there is no `GET /api/workspaces/{id}/docs` —
  documents ride inside the org's workspace list. Secrets
  `grist.session-secret`, `grist.admin-password`, `grist.api-key`,
  `grist.boot-key` are filed in SOPS + Bitwarden + 1Password,
  hash-verified.

### Trial — ready-for-agent end to end

A deliberately trivial issue was dispatched end to end: **#762**
("Add the PM-stack design record to doc/README.md's starting
points"), labelled `ready-for-agent`, implemented via
`implementNow`. The harness created its worktree, installed
dependencies, ran the laptop's OpenCode 2
(`opencode-go/kimi-k3.1-pro`), and opened **PR #763** with
**exactly** the one requested line added to `doc/README.md` —
verified by reading the full diff. No auto-merge occurred: the
PR's auto-merge is unset, and the work item settled in the
harness's `NEEDS_HUMAN` state, whose own status message confirms
auto-merge is disabled by the repository's Merge Policy and a
human merge is expected (the harness then detects the merge on
its next refresh and completes the item by itself). PR #763 is
left open for human review, as a dispatch trial should be.

### Phase close

Phase 2 ends with the three layers live (Kaneo board, Pad
planning, Grist tables, ready-for-agent dispatch) and the
**aggregator UI scoped but not built**: the two views and the two
first-class requirements under "The two views" above, fed by the
projector + link registry whose keys are repo-qualified issue/PR
references per item and a comment-activity projection keyed by
them. That is the next push, after the remaining repos are
connected to Kaneo and the Kaneo fork (decided above) begins.

## Connect-repos as-built (2026-10-25)

Connecting the remaining repos (branch `pm/connect-repos`):

- **Projects created** in workspace "Developing Today", mirroring
  `code` (same icon; the default columns To Do / In Progress /
  In Review / Done, verified identical): `opencode`
  (`f16a7r9g54df44z055dirvr3`), `openchamber`
  (`z5qb6tmgo5idx7o8bnv7rqyv`), `nix`
  (`kslqc1uhul68zaepdpcpsdml`). The registry is
  `pm-stack/kaneo-projects.json`; `pm-stack/connect-repo.sh` does
  `status` (installation coverage + counts, read-only) and
  `connect <slug>` (verify → connect → import until done → counts).
- **Installs are owner-gated; no API route exists.** Verified live:
  installation 174792676 still covers `developing-today/code` only.
  `PUT /user/installations/{id}/repositories/{repo_id}` as `dezren39`
  (an org admin) returns 403 "You do not have permission to modify
  this app on developing-today. Please contact an Organization
  Owner." — the endpoint needs an org owner's session; the app's own
  JWT cannot add repositories, no endpoint creates an installation,
  and the app is private (`"public": false` in the manifest), so the
  personal-account install for `dezren39/nix` is an owner browser
  step too (it also needs the app made installable beyond the org,
  or a second app). Kaneo's own verify agrees: for both new repos it
  returns "Repository access could not be verified. GitHub repository
  admin permission and an installed Kaneo App are required".
  Remaining owner steps are recorded in the session's final report.
- **Not done yet, by that gate:** the integration links, the
  one-time imports, and the throwaway-issue sync proofs for the
  three repos. Each is one `./connect-repo.sh connect <slug>` run
  once its install lands.
- Verified along the way (2026-10-25): the app JWT shows
  installation 174792676 covering `developing-today/code` only and
  no installation on the other three repos; GitHub open issues
  (excluding PRs) are code 139, opencode 27, openchamber 53,
  nix 184, while Kaneo holds 127 tasks and 127 issue links for
  `code` and 0 tasks — with a null GitHub integration — for each
  new project. The `code` gap is issues opened since the Phase 1
  import; imports update existing tasks, so a re-import closes it
  (not re-run here — not needed for the connect-repos step).
