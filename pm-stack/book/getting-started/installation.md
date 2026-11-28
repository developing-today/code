# Installation and bootstrap

The PM stack installs on one laptop host and at the Cloudflare edge. The laptop host is `amd`. One `nixos-rebuild switch` on that host deploys the stack, per `pm-stack/README.md`. This chapter states what that deployment contains, what runs as a container instead, what is deployed by other routes, and which bootstrap steps the survey could not verify. Steps the survey does not establish are marked UNVERIFIED; this book does not invent commands for them.

Start from the system as a whole in [The shape of the system](shape.md), then return here for the parts.

## The deployed shape

| Part            | How it runs                                                                         | Port                                        |
| --------------- | ----------------------------------------------------------------------------------- | ------------------------------------------- |
| Aggregator      | Bun app, systemd user unit `pm-aggregator`                                          | 4870 (freshness IPC on 4871)                |
| Console         | Bun app, systemd user unit `pm-console`                                             | 4872                                        |
| Projector       | Bun service, Nix package `pkgs/pm-projector`                                        | 7801                                        |
| pm-dispatch     | Bun intake service, public as `dispatch.security.cab`                               | 4879                                        |
| Tunnel          | systemd user unit `pm-tunnel`, the `amd-6-pm-stack` Cloudflare Tunnel               | —                                           |
| Kaneo           | Container, upstream image (D1), Postgres 16 sidecar                                 | 5173, loopback                              |
| Grist           | Container                                                                           | 8484, loopback and tailnet                  |
| Pad             | Container, Go binary and SQLite                                                     | 7777, loopback and tailnet                  |
| ready-for-agent | Upstream npm package `ready-for-agent@1.1.0`, systemd user unit, no container image | 6056, loopback and tailnet via socket proxy |

Ports are registered in `pm-stack/PORTS.md`, audited 2026-11-05. The aggregator dashboard binds `127.0.0.1:4870` and the tailnet; in public it is reached only behind the console gate at `pm.security.cab`, where the console proxies non-`/console` requests to the dashboard for a signed-in owner.

## The NixOS deployment

The user units `pm-aggregator`, `pm-projector`, `pm-dispatch`, and `pm-tunnel` are declared in the NixOS module `nixos/services/pm-stack`. Deploying is a merge to the repository followed by `nixos-rebuild switch` on `amd`; since issue #8171, `pm-git-sync` never restarts PM services itself — deployment is by merge and rebuild, not by the sync script.

Nix packages in the tree are `pkgs/pm-aggregator` (the aggregator plus four script binaries: gather-quota, generate-digest, export-turso-stores, quota-weekly-review; it also bundles `pm-stack/kaneo-projects.json` and `pm-stack/grist/seed/roadmap.json`), `pkgs/pm-dispatch`, `pkgs/pm-projector`, and `pkgs/pm-git-sync` (a script wrapper, version 0.3.0).

There is no `pkgs/pm-console`, and the console is the one PM-stack app NixOS does **not** declare (T4, survey top-up 1): `pm-console` runs as the hand-installed user unit `pm-console.service`, copied from `pm-stack/systemd/pm-console.service` by `pm-stack/console/install.sh`, executing `bun run src/server.ts` on port 4872 from its own copy of `main` under `~/.local/share/pm-console` (a release per commit, fetched from GitHub; not the primary checkout, which it ran from until 2026-11-24, #13833), and it is kept current by its own per-minute redeploy timer (`pm-console-redeploy.timer`), not by `nixos-rebuild switch`. A grep for `pm-console` over `nixos/` and `home/` returns nothing. Its edge release is below.

Repo copies of further unit files live in `pm-stack/systemd/`: `pm-host-health` with its timer, `pm-invariant-sweeper` with its timer, `pm-kaneo-sync-guard` with its timer, `pm-quota-watchdog` with its timer, `pm-console-expose` with its timer, `pm-console-redeploy` with its timer, and `pm-console-android-release` with its timer. Of these repo copies, only the `pm-console*` family is installed from this directory by a live script (`pm-stack/console/install.sh`); the `pm-aggregator`, `pm-dispatch`, and `pm-tunnel` copies in the same directory are the pre-NixOS files the NixOS module header says are kept beside that history in git (T4, survey top-up 1).

## The containers

Kaneo, its Postgres, Pad, and Grist are containers, defined in `pm-stack/compose.yml` and the per-app compose files.

- **Kaneo.** The compose file pins the upstream image `ghcr.io/usekaneo/kaneo:2.47.0` by digest. A fork exists at `developing-today-forks/kaneo`, and the aggregator's program text says the board runs that fork; the deployment code wins — the deployed board is upstream v2.47.0 (D1). Kaneo is public as `https://kaneo.security.cab` through the tunnel. Its four projects and their repositories are listed in `pm-stack/kaneo-projects.json`; `pm-stack/connect-repo.sh` reads that file.
- **Grist.** `pm-stack/grist/compose.yml` pins `gristlabs/grist-oss` by digest with no tag. The README names v1.7.21; whether the pinned digest is v1.7.21 is UNVERIFIED (D2). Seed and maintenance scripts live in `pm-stack/grist/`: `seed.sh`, `deploy.sh`, `reload-doc.sh`.
- **Pad.** `pm-stack/pad/compose.yml` pins Pad v0.18.1 by tag and digest. Its content in the repository is seed documents only, under `pm-stack/pad/seed/`, installed by `pad/seed.sh`.
- **ready-for-agent.** No container. `pm-stack/ready-for-agent/install.sh` clones the four repositories; `apply-config.sh` registers them through the GraphQL API; `config.json` sets each repository's default model and a merge policy of OFF — a human merges, always.

## The edge and the phone

Three parts of the stack do not deploy by the laptop rebuild:

1. **The console edge Worker** (`pm-console-edge`) is deployed from GitHub Actions, per `pm-stack/platform/workers.json`. It serves the same console at `pm.security.cab` when the laptop is unreachable, and holds actions until the host returns.
2. **The decision-events Worker** is in service in `workers.json` and is recorded there as deployed by hand.
3. **The Android app** is a trusted web activity wrapper around the console. The `pm-console-android-release` service and timer build and release it; the APK is served by the console itself to a signed-in owner.

The notify relay Worker, `pm-notify`, is recorded in `workers.json` as not deployed, with a null address (D5). Do not treat the relay as part of a working installation: the dead-man's switch and the edge answer routes do not exist in production.

The Workers registry itself is `pm-stack/platform/workers.json`, which lists 20 Workers. The platform README still says eighteen; the JSON is the live list, held to the repository's wrangler files by `workers.test.ts` (D6).

## Keeping the laptop clone current

`pm-git-sync` (`pm-stack/scripts/pm-git-sync.sh`, packaged as `pkgs/pm-git-sync`) keeps the laptop's primary clone on a clean, current `main`. It fails loudly on tracked dirt, quarantines untracked files and never deletes them, tags aside empty rebuild-marker commits, and otherwise fetches and pulls with fast-forward only. A dirty primary clone is therefore a stop condition for deployment, not a state to deploy over.

## Bootstrap

The reconciled first-run order below is assembled from four places in `pm-stack/README.md` plus `pm-stack/BOOTSTRAP-CREDENTIALS.md` (survey top-up 1, T1). The docs give no single numbered bootstrap list; the order is a reconciliation, and its seams are inferred only from the README's own dependency statements. Two points stay UNVERIFIED and are marked where they stand: the container first-start order inside `nixos-rebuild switch` (no document enumerates it; only NixOS unit `After`/`Wants` dependencies are verified), and which tunnel install path a fresh host should use today (the README hand-installs `pm-tunnel.service` even though the NixOS module also declares `pm-tunnel`; the docs do not say which path wins).

0. **Cold-restore stub warning.** `pm-stack/COLD-RESTORE-RUNBOOK.md` is not the runbook; it is a stub pointing at the operative runbook, `doc/pm-stack/COLD-RESTORE-RUNBOOK.md`, since the 2026-11-22 rehearsal slice.
1. **Roots of trust first, on a new computer only (cold restore).** In order: Drew signs into Bitwarden and mints a BWS machine-account token (the token is not stored in BWS, by design); with it, fetch by name the restic passwords and the R2 and Supabase secrets for the backup and restore legs; Drew signs into Google interactively so rclone's `drive` remote gets a fresh OAuth token (this cannot be automated and is stored in no backup); fetch the 1Password service-account tokens from BWS by name. The SOPS age key is explicitly unresolved, tracked in its own gap issue, #10295 — referenced here, not solved.
2. **Check out the repo and enter the stack directory.** On laptop host `amd` only: check out the repo (or copy `pm-stack/` plus `../secrets`) and `cd pm-stack`.
3. **Apply the NixOS-managed runtime.** `nixos-rebuild switch`. The README's gloss: "This owns Pad, Grist, ready-for-agent, Kaneo, and PostgreSQL." The live manager is NixOS: `nixos/services/pm-apps` owns the `pm-grist` and `pm-pad` OCI containers and the ready-for-agent systemd user unit; `nixos/services/pm-stack` owns the `pm-kaneo` user service (the Compose-backed Kaneo/PostgreSQL stack) plus the other PM user services and timers. The console is not in this step — it is hand-installed (see the NixOS deployment section above).
4. **Install and start the tunnel connector.** The README's bring-up step hand-installs the unit: `install -Dm644 systemd/pm-tunnel.service ~/.config/systemd/user/pm-tunnel.service`, `systemctl --user daemon-reload`, `systemctl --user enable --now pm-tunnel.service`. The tunnel is the Cloudflare Tunnel named `amd-6-pm-stack`, remotely managed, mapping `kaneo.security.cab` to `http://localhost:5173`. UNVERIFIED: whether this hand-install or the NixOS module's own `pm-tunnel` declaration is the path a fresh host uses today — both facts stand in the docs; neither is chosen there.
5. **Verify.** `curl --fail http://127.0.0.1:5173/api/health` and `curl --fail https://kaneo.security.cab/api/health`.
6. **What `deploy.sh` does when the Kaneo stack is (re)deployed.** It decrypts `../secrets/pm-stack.sops.yaml` (SOPS/age) and renders mode-600 files beside `compose.yml`: `.env` and the tunnel token files. Its path is: render, `docker compose pull`, `docker compose up -d --force-recreate`, poll `http://127.0.0.1:5173/api/health` up to 30 times at 5-second intervals, then run `kaneo-sync-guard.py` — a failed guard marks the deploy failed.
7. **Create the first Kaneo account, then close registration again.** Registration renders closed by default, and a re-render must never silently reopen it: (1) `KANEO_OPEN_REGISTRATION=1 ./deploy.sh` — one render with registration open; (2) create the admin account (the `kaneo_admin_*` credentials in SOPS) via the sign-up page or `POST /api/auth/sign-up/email`; (3) `./deploy.sh` again, which renders `DISABLE_REGISTRATION=true` and recreates the container. Then create a workspace and the first project from the UI or the API.
8. **Register the GitHub App — after the tunnel exists.** The tunnel has to exist first, because the app's webhook URL is `https://kaneo.security.cab/api/github-integration/webhook`. Registration is a one-time browser step (GitHub offers no API for it): POST `pm-stack/github-app-manifest.json` as the `manifest` field to the org's app-creation URL; exchange the returned code within the hour via `POST https://api.github.com/app-manifests/{code}/conversions`; put the four returned values into `secrets/pm-stack.sops.yaml`, re-encrypt, file copies in Bitwarden and 1Password, and re-run `./deploy.sh`; install the app on the repositories to sync (Phase 1: `developing-today/code` only); then connect the repository in Kaneo's project settings and import issues. After installation, `./connect-repo.sh connect <slug>` performs verify, connect, and import, and `./connect-repo.sh status` reports every project without changing anything.
9. **Phase 2 first-time bootstraps, each inside its already-running container or service.** Pad: `pad init` runs inside the NixOS-managed `pm-pad` container (for example `docker exec pm-pad pad init`); create the admin from the `pad_admin_*` SOPS credentials and the workspace **Developing Today**, mint a REST token named `pm-stack-automation`, file it as `pad_api_token`, and run `cd pad && ./seed.sh`. Grist: the boot-key flow — `POST /boot/login` with the boot key and admin email; the admin is the `grist_admin_*` account; `POST /api/profile/apiKey` as the admin returns the key filed as `grist_api_key`; then `cd grist && ./seed.sh`. ready-for-agent: `cd ready-for-agent && ./install.sh`, then `./install.sh clone` (the one-time clone of the repos it serves); every install run finishes by applying `ready-for-agent/config.json` through the GraphQL API via `apply-config.sh`.
10. **Cold restore has its own start order** (a restore, not a first install), in `doc/pm-stack/COLD-RESTORE-RUNBOOK.md`: declare and fence the old writer; provision the restore host after the roots-of-trust order in step 1; restore Kaneo, then Grist, then Pad, each with its integrity checks; start and smoke; measure RTO/RPO; record the drill report.

Two bootstrap facts are established and worth stating before any install:

- The Kaneo GitHub integration's status-transition settings live only in Kaneo's Postgres, in `integration.config.statusTransitions` per project. Nothing in the repository sets them. After a fresh Kaneo install or a Postgres restore, those settings must be checked directly, and `pm-stack/kaneo-sync-guard.py` is the guard that asserts them: no transition targets `done`, merge goes to in-review, open goes to in-review, push goes to in-progress. The guard runs at deploy time and hourly on its timer, and exits 1 loudly on drift it cannot heal.
- The aggregator's Grist read needs the Grist base URL, document id, and API key from its configuration (`pm-stack/aggregator/src/config.ts`). Secret values are not in this book; they are resolved by name through the fleet secrets route at run time.

## After installation

Prove the seat in the order [Quickstart](quickstart.md) uses: open the console, read the Now view, open the aggregator health page at `/health` — its machine twin is `/api/health`, with no `.json` suffix — and check that the manifest pages report render state. Runtime liveness of any individual service at the moment you read this is UNVERIFIED from the repository; the health page is the live source, not this book.

Sources. Survey files `~/workspace/pm-book/survey/apps.md` (§1–§6, §13, §26, §36, Nix packages summary), `~/workspace/pm-book/survey/website-pages.md` (introduction, §C), `~/workspace/pm-book/survey/disagreements.md` (D1, D2, D5, D6); repo paths `pm-stack/README.md`, `pm-stack/PORTS.md`, `pm-stack/compose.yml`, `pm-stack/grist/compose.yml`, `pm-stack/pad/compose.yml`, `pm-stack/platform/workers.json`, `nixos/services/pm-stack`, `pkgs/pm-aggregator/package.nix`, `pkgs/pm-git-sync/package.nix`.
