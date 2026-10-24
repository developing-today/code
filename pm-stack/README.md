# pm-stack — project-management stack deployment

Phase 1 of the project-management stack (design record:
[`doc/2026-10-24T11-05-38Z_design_project-management-stack/`](../doc/2026-10-24T11-05-38Z_design_project-management-stack/)).
This directory is the complete deployment definition. Everything lives
in this repository; the only pieces that may ever live elsewhere are
forks of upstream projects.

## What runs

| Piece      | Image                                    | Notes                                                                                 |
| ---------- | ---------------------------------------- | ------------------------------------------------------------------------------------- |
| Kaneo      | `ghcr.io/usekaneo/kaneo:2.47.0` (pinned) | Web UI + API on container port 5173, bound to host `127.0.0.1:5173` only              |
| PostgreSQL | `postgres:16-alpine`                     | Kaneo's database; named volume `pm-stack_postgres_data`; never exposed on a host port |

Public address: **https://kaneo.security.cab** — a Cloudflare Tunnel
(tunnel name `amd-6-pm-stack`, remotely managed) whose ingress maps
that hostname to `http://localhost:5173`. The tunnel connector runs as
the `pm-tunnel` systemd **user** unit (see `systemd/pm-tunnel.service`).
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
`cloudflare_tunnel_token`. The same values are also filed in Bitwarden
and 1Password under the names recorded in the design record.

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

Registration starts open so the first account can be created. Create
the admin account (the `kaneo_admin_*` credentials in SOPS) via the
sign-up page — or, equivalently, `POST /api/auth/sign-up/email` — then
close registration:

```bash
sed -i 's/^DISABLE_REGISTRATION=false/DISABLE_REGISTRATION=true/' .env
docker compose up -d --force-recreate kaneo
```

(`deploy.sh` preserves `DISABLE_REGISTRATION=true` once set.) Then
create a workspace and the first project from the UI or the API.

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
5. In Kaneo: create/open the project → Settings → Integrations →
   GitHub → connect `developing-today/code`, then **Import issues**.
   (API equivalents: `POST /api/github-integration/verify`,
   `POST /api/github-integration/project/{projectId}`,
   `POST /api/github-integration/import-issues`.)

What syncs (verified against Kaneo v2.47.0): issues ↔ tasks, comments
(create-only; edits/deletes do not sync), open/close both ways,
`status:*`/`priority:*` labels both ways, title/body edits. PRs are
read-only to Kaneo — they only move tasks (push → in-progress, PR
opened → in-review, merged → done). Bot-authored comments are not
imported inbound. Each Kaneo project connects to exactly one
repository (upstream issue usekaneo/kaneo#791).

## Day-2 operations

- **Status:** `docker compose ps`, `docker compose logs -f kaneo`,
  `systemctl --user status pm-tunnel`.
- **Restart with current env:** `./deploy.sh restart`. Environment
  changes require `./deploy.sh` (compose applies env on recreate, not
  restart).
- **Update Kaneo:** bump the pinned tag in `compose.yml` after reading
  the upstream release notes, then `./deploy.sh`. Never float on
  `:latest`.
- **Backups:** `docker compose exec postgres pg_dump -U kaneo kaneo`
  into the host's normal backup path; the SOPS file + this directory
  are the rest of the restore story.
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
└── systemd/
    └── pm-tunnel.service      # Cloudflare Tunnel connector (user unit)
```
