# pm-stack — project-management stack deployment

Phases 1–2 of the project-management stack (design record:
[`doc/2026-10-24T11-05-38Z_design_project-management-stack/`](../doc/2026-10-24T11-05-38Z_design_project-management-stack/)).
This directory is the complete deployment definition. Everything lives
in this repository; the only pieces that may ever live elsewhere are
forks of upstream projects.

## What runs

| Piece      | Image                                    | Notes                                                                                 |
| ---------- | ---------------------------------------- | ------------------------------------------------------------------------------------- |
| Kaneo      | `ghcr.io/usekaneo/kaneo:2.47.0` (pinned) | Web UI + API on container port 5173, bound to host `127.0.0.1:5173` only              |
| PostgreSQL | `postgres:16-alpine`                     | Kaneo's database; named volume `pm-stack_postgres_data`; never exposed on a host port |

Phase 2 (2026-10-24) added three layers, each in its own subdirectory
with the same deploy shape (SOPS → mode-600 env → compose/unit →
health gate):

| Layer     | Piece           | Version / pin                                                    | Reach                                                                                 | Deployment definition |
| --------- | --------------- | ---------------------------------------------------------------- | ------------------------------------------------------------------------------------- | --------------------- |
| Dispatch  | ready-for-agent | npm `ready-for-agent@1.1.0` (no container image exists upstream) | UI + GraphQL on `6056`, all interfaces (loopback + tailnet + LAN)                     | `ready-for-agent/`    |
| Planning  | Pad             | `ghcr.io/perpetualsoftware/pad:0.18.1`, pinned by digest         | `127.0.0.1:7777` + tailnet `amd-6.raptor-pumpkinseed.ts.net:7777`; no tunnel hostname | `pad/`                |
| Tables    | Grist           | `gristlabs/grist-oss:v1.7.21`, pinned by digest                  | `127.0.0.1:8484` + tailnet `amd-6.raptor-pumpkinseed.ts.net:8484`; no tunnel hostname | `grist/`              |
| Dashboard | Aggregator      | this repo (Bun app, user unit `pm-aggregator`)                   | `127.0.0.1:4870` + tailnet `amd-6.raptor-pumpkinseed.ts.net:4870`; no public route    | `aggregator/`         |
| Intake    | pm-dispatch     | this repo (Bun app, user unit `pm-dispatch`)                     | `127.0.0.1:4879` + tailnet; public via tunnel as `dispatch.security.cab`              | `dispatch/`           |

ready-for-agent runs as a systemd **user unit** (like `pm-tunnel`)
because upstream ships only an npm package; Pad and Grist run as
containers per the standing container rule. Grist was chosen over
Baserow — the evaluation is in the design record.

Public address: **https://kaneo.security.cab** — a Cloudflare Tunnel
(tunnel name `amd-6-pm-stack`, remotely managed) whose ingress maps
that hostname to `http://localhost:5173`. The tunnel connector runs as
the `pm-tunnel` systemd **user** unit (see `systemd/pm-tunnel.service`);
the same tunnel also maps `dispatch.security.cab` to
`http://localhost:4879` (pm-dispatch, added 2026-10-30). On `amd`, one
`nixos-rebuild switch` deploys the whole stack: the user units
(`pm-aggregator`, `pm-projector`, `pm-dispatch`, `pm-tunnel`) and the
Kaneo stack's lifecycle are declared in
[`../nixos/services/pm-stack`](../nixos/services/pm-stack).
The tunnel has to exist before the GitHub App can be registered,
because the app's webhook URL is
`https://kaneo.security.cab/api/github-integration/webhook`.

Current host: the Framework laptop (`amd-6`). Host specifics are
isolated in `deploy.sh` (two env vars) and the systemd unit paths;
everything else is host-independent so the stack can move to the
Kubernetes cluster later (the same values map onto a Secret + the
upstream Helm chart's env).

## Secrets

All secrets live in [`../secrets/pm-stack.sops.yaml`](../secrets/pm-stack.sops.yaml)
(SOPS/age, recipients: laptop + sandbox — see the path rule in
`../.sops.yaml`). Keys: `postgres_password`, `auth_secret`,
`notification_secret_encryption_key`, `kaneo_admin_email`,
`kaneo_admin_password`, `github_app_id`, `github_app_name`,
`github_webhook_secret`, `github_private_key_base64`,
`cloudflare_tunnel_token` — plus the Phase 2 additions:
`pad_api_key`, `pad_admin_email`, `pad_admin_password`,
`pad_api_token`, `grist_session_secret`, `grist_admin_password`,
`grist_api_key`, `grist_boot_key`. The same values are also filed in
Bitwarden and 1Password under the names recorded in the design
record (Bitwarden/1Password names use dots, e.g. `pad.api-token`,
`grist.boot-key`).

`deploy.sh` decrypts that file on the host and renders two mode-600
files beside `compose.yml`:

- `.env` — consumed by both containers (see `.env.example` for the
  shape and the SOPS→env mapping);
- `tunnel.env` — the tunnel token for the `pm-tunnel` unit.

Never commit `.env`/`tunnel.env`; never hand-edit them (re-render
instead). Changing `AUTH_SECRET` signs everyone out; changing
`POSTGRES_PASSWORD` after first initialisation breaks the database
login until the database password is changed to match.

## Bring it up (first time, on the host)

```bash
# 1. Check out this repo (or copy this directory + ../secrets) and cd in
cd pm-stack

# 2. Render env and start the stack (waits for the health endpoint)
./deploy.sh

# 3. Install + start the tunnel connector
install -Dm644 systemd/pm-tunnel.service ~/.config/systemd/user/pm-tunnel.service
systemctl --user daemon-reload
systemctl --user enable --now pm-tunnel.service

# 4. Verify
curl --fail http://127.0.0.1:5173/api/health
curl --fail https://kaneo.security.cab/api/health
```

## First account + lock down registration

Registration renders **closed** by default — a re-render must never
silently reopen it (it did once, 2026-10-24; see the design record's
Phase 1 as-built correction). To bootstrap the first account, render
once with the explicit opt-out, create the account, then render
again to close:

```bash
# 1. One render with registration open
KANEO_OPEN_REGISTRATION=1 ./deploy.sh

# 2. Create the admin account (the kaneo_admin_* credentials in
#    SOPS) via the sign-up page — or, equivalently,
#    POST /api/auth/sign-up/email

# 3. Close registration again (renders DISABLE_REGISTRATION=true and
#    recreates the container)
./deploy.sh
```

Then create a workspace and the first project from the UI or the
API.

## GitHub App (repository sync)

Kaneo syncs through a GitHub App that **we** register (one app serves
the whole instance; install it per repository). Registration is a
one-time browser step because GitHub offers no API for it:

1. Signed in as a `developing-today` org owner, use the app-manifest
   flow with [`github-app-manifest.json`](github-app-manifest.json):
   POST it as the `manifest` field to
   `https://github.com/organizations/developing-today/settings/apps/new`.
   The manifest requests exactly the permissions Kaneo documents —
   Issues: read/write; Pull requests, Contents, Metadata: read; events
   Issues, Issue comments, Pull requests, Push — and sets the webhook
   URL above.
2. GitHub redirects to `https://kaneo.security.cab/?code=…`. Exchange
   the code within the hour:
   `POST https://api.github.com/app-manifests/{code}/conversions`
   returns the App ID, slug, PEM private key, and webhook secret.
3. Put those four values into `secrets/pm-stack.sops.yaml`
   (`github_app_id`, `github_app_name`, `github_webhook_secret`,
   `github_private_key_base64` = base64 of the PEM), re-encrypt
   (`sops -e -i`), file copies in Bitwarden + 1Password, then re-run
   `./deploy.sh` on the host.
4. Install the app on the repositories to sync (GitHub → org settings →
   GitHub Apps → Install App; Phase 1: `developing-today/code` only).
   Adding a repository to an installation cannot be done with the
   app's own credentials — the REST endpoint needs an org owner's (or,
   for a personal account, the account owner's) GitHub session, so
   this step stays a browser step.
5. In Kaneo: create/open the project → Settings → Integrations →
   GitHub → connect the repository, then **Import issues**.
   (API equivalents: `POST /api/github-integration/verify`,
   `POST /api/github-integration/project/{projectId}`,
   `POST /api/github-integration/import-issues`.) The projects, their
   repositories, and their installations are registered in
   [`kaneo-projects.json`](kaneo-projects.json); once the app is
   installed on a repository, `./connect-repo.sh connect <slug>`
   performs verify → connect → import (resuming until done) and
   prints the GitHub open-issue count next to the Kaneo task, issue
   link, and not-in-Done counts. `./connect-repo.sh status` reports
   the same for every project without changing anything.

What syncs (verified against Kaneo v2.47.0): issues ↔ tasks, comments
(create-only; edits/deletes do not sync), open/close both ways,
`status:*`/`priority:*` labels both ways, title/body edits. PRs are
read-only to Kaneo — they only move tasks (push → in-progress, PR
opened → in-review, merged → done). Bot-authored comments are not
imported inbound. Each Kaneo project connects to exactly one
repository (upstream issue usekaneo/kaneo#791).

## Phase 2 components

### ready-for-agent (dispatch layer)

```bash
cd ready-for-agent
./install.sh          # pinned npm package + user unit + health gate
./install.sh clone    # one-time: clone the repos it serves into ./repos/
```

- **Register a repo** through the harness's GraphQL API (loopback
  `http://127.0.0.1:6056/graphql`): mutation
  `addLocalRepository(path: "<absolute path to a full clone>")`.
  Do **not** use the `ready-for-agent add` CLI from an interactive
  SSH shell on this host — its in-process git inspection returns
  `NOT_A_GIT_REPOSITORY` for valid repos there (it succeeds from
  the service's clean environment, which is what GraphQL uses).
- **Configure a repo** with `updateRepositorySettings` (a
  full-object input — every field is required, so read the current
  settings first) and `unpauseRepository`. Fleet posture for
  `developing-today/code`: `mergePolicy: OFF` (human merge),
  `includeAllIssueAuthors: true`, `defaultModel:
opencode-go/kimi-k3.1-pro` on the OpenCode backend. A repo with
  no build model set refuses Implement ("No build model set").
- **Dispatch:** label an issue `ready-for-agent`, then Implement
  from the UI, via GraphQL `implementNow(repositoryId, nativeId:
"<issue number>")`, or via the CLI `ready-for-agent intake
<owner/repo>` (starts **all** current candidates — use with
  care). The harness creates a worktree under
  `/tmp/ready-for-agent/<owner>/<repo>/…`, runs the host's
  OpenCode 2, and opens a PR. GitHub auth is the host's `gh` login.
- **Never enable auto-merge.** "Implement With" and "Implement
  All with Auto-merge" write per-item merge-policy pins; the fleet
  posture is human merge, always.
- Status: `ready-for-agent status developing-today/code`;
  backend health is the `agentBackendStatuses` GraphQL query
  (OpenCode should read `kind: READY`).

### Pad (planning layer)

```bash
cd pad
./deploy.sh   # SOPS -> .env, compose up, health gate on /api/v1/health
./seed.sh     # idempotent REST seed of the Plan items in seed/
```

- **Health:** `GET http://127.0.0.1:7777/api/v1/health` →
  `{"status":"ok",…}`. (`/healthz` only hits the SPA fallback.)
- **Bootstrap (first time only):** `pad init` must run **inside**
  the server container (`docker compose exec pad pad init`) —
  bootstrap is loopback/token-gated (token at
  `/data/.bootstrap-token`). Create the admin from the
  `pad_admin_*` SOPS credentials and the workspace
  **Developing Today**; the startup template creates the tasks,
  ideas, plans, docs, conventions, and playbooks collections.
- **API automation:** REST uses minted user tokens (`pad_`
  prefix, Bearer). The `PAD_API_KEY` env var is inert on 0.18.1.
  Mint a token named `pm-stack-automation` (scopes `*`) and file
  it as `pad_api_token`. **Tokens expire after 90 days** (the
  current one on 2027-01-22) — re-mint and re-file in SOPS +
  Bitwarden + 1Password before expiry.
- Seed content lives in `seed/*.md`; the seed talks to
  workspace-scoped REST routes
  (`/api/v1/workspaces/developing-today/collections/plans/items`)
  and skips items that already exist.

### Grist (table layer)

```bash
cd grist
./deploy.sh   # SOPS -> .env, compose up, health gate
./seed.sh     # idempotent seed: workspace, doc, Roadmap table + rows
```

- **Bootstrap (first time only):** the boot-key flow —
  `POST /boot/login {bootKey, adminEmail}`. The boot key lives in
  the home DB because the container writes no logs: copy
  `/persist/home.sqlite3` out of the volume and read
  `activations.prefs` → `envVars.GRIST_BOOT_KEY`. It is filed as
  `grist_boot_key`. The admin (`grist_admin_*`) is the install
  admin.
- **API key:** `POST /api/profile/apiKey` as the admin returns a
  raw key; file it as `grist_api_key`. The key is all-or-nothing
  for its user — the dedicated admin account is the mitigation.
- **API quirks (1.7.21):** `POST /api/docs/{id}/apply` takes the
  bare UserAction array as the body; create-workspace/doc
  endpoints return bare ids; there is no
  `GET /api/workspaces/{id}/docs` (docs ride inside the org's
  workspace list).
- Seeded content: workspace **Roadmap & registers**, document
  **Program roadmap**, table **Roadmap**, rows from
  `seed/roadmap.json` (the PM-stack phases, the cluster register
  headlines, the four-repo fleet).

## Day-2 operations

- **Status:** `docker compose ps`, `docker compose logs -f kaneo`,
  `systemctl --user status pm-tunnel`. Phase 2 pieces:
  `docker compose -f pad/compose.yml ps`, `curl --fail
http://127.0.0.1:7777/api/v1/health` (Pad), `docker compose -f
grist/compose.yml ps` (Grist), `systemctl --user status
ready-for-agent` + `ready-for-agent status
developing-today/code` (dispatch).
- **Restart with current env:** `./deploy.sh restart`. Environment
  changes require `./deploy.sh` (compose applies env on recreate, not
  restart). Each Phase 2 subdirectory has the same `deploy.sh`
  contract (Pad, Grist); ready-for-agent refreshes via
  `ready-for-agent/install.sh`.
- **Update Kaneo:** bump the pinned tag in `compose.yml` after reading
  the upstream release notes, then `./deploy.sh`. Never float on
  `:latest`. Same rule per component: Pad/Grist image pins (tag +
  digest) in their `compose.yml`, ready-for-agent's `PINNED_VERSION`
  in `ready-for-agent/install.sh`.
- **Pad API token:** expires 90 days after minting (current:
  2027-01-22) — re-mint, re-file (SOPS + both vaults), restart
  nothing (the token is used by external automation, not the
  container).
- **Backups:** `docker compose exec postgres pg_dump -U kaneo kaneo`
  into the host's normal backup path; the SOPS file + this directory
  are the rest of the restore story. Pad's data is the named volume
  `pad_pad_data`; a Grist document backs up as its `.grist` file
  (SQLite) under the volume's `/persist/docs`, plus `home.sqlite3`
  for the install itself.
- **Move hosts:** copy this directory + `../secrets`, ensure the new
  host's age key is a SOPS recipient, run `./deploy.sh`, install the
  unit, done. The tunnel is host-agnostic (token-based); only one
  connector should run at a time.

## Layout

```text
pm-stack/
├── README.md                  # this runbook
├── compose.yml                # pinned Kaneo 2.47.0 + PostgreSQL 16
├── .env.example               # configuration shape (no real values)
├── deploy.sh                  # SOPS -> .env/tunnel.env, compose up, health gate
├── github-app-manifest.json   # one-time GitHub App registration
├── kaneo-projects.json        # projects <-> repositories <-> installations
├── connect-repo.sh            # status | connect <slug>: verify, link, import
├── systemd/
│   └── pm-tunnel.service      # Cloudflare Tunnel connector (user unit)
├── pad/                       # Phase 2 planning layer (Pad 0.18.1)
│   ├── compose.yml            # pinned image + digest, loopback + tailnet
│   ├── deploy.sh              # same deploy contract as the top level
│   ├── .env.example
│   ├── seed.sh                # idempotent REST seed
│   └── seed/                  # Plan item content (Markdown)
├── grist/                     # Phase 2 table layer (Grist 1.7.21)
│   ├── compose.yml            # pinned grist-oss image + digest
│   ├── deploy.sh
│   ├── .env.example
│   ├── seed.sh                # idempotent seed via the REST API
│   └── seed/roadmap.json      # Roadmap rows
└── ready-for-agent/           # Phase 2 dispatch layer (ready-for-agent 1.1.0)
    ├── install.sh             # pinned npm install + unit + clone helper
    └── systemd/
        └── ready-for-agent.service
```
