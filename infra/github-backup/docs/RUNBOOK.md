# github-backup RUNBOOK

All commands are copy-paste. `$GB` = this directory in a checkout of
developing-today/code. On the laptop the checkout is `~/git/code`, so:

```sh
GB=~/git/code/infra/github-backup
ROOT=~/backups/github        # backup tree (config.json: backup_root)
```

Secrets are referenced BY NAME from Bitwarden Secrets Manager; source
`~/tools/bws-env.sh` in any shell that runs `bws` or `app_setup.py`.

---

## 1. Initial install (a machine that already has the tree)

```sh
cd ~/git/code && git pull
$GB/systemd/install-units.sh
systemctl --user list-timers 'github-backup-*'
```

## 2. Initial build of the tree (or rebuild from scratch)

```sh
mkdir -p $ROOT
python3 $GB/bin/reconcile.py        # writes plan.json, clones everything missing
python3 $GB/bin/manifest.py         # writes MANIFEST.json + FAMILIES.md
du -sh $ROOT
python3 -c "import json;print(json.load(open('$ROOT/MANIFEST.json'))['totals'])"
```

`reconcile.py` is resumable: re-running skips verified mirrors and only
clones what is missing. The clone pass is parallel (4 workers) and takes
hours on first run for the full estate (~850 mirrors).

Spot-verify a mirror end to end:

```sh
git -C $ROOT/dezren39/hatch-personal.git fsck
git clone $ROOT/dezren39/hatch-personal.git /tmp/restore-test && \
  git -C /tmp/restore-test log -1 --oneline
```

## 3. The GitHub App (one-time)

The App must be created by a human click (GitHub requires it); everything
after is scripted.

```sh
source ~/tools/bws-env.sh
# 1. deploy the Worker first (§5) and set GHB_WORKER_URL to its URL.
export GHB_WORKER_URL="https://developing-today-backup.<your-subdomain>.workers.dev"
# 2. store the Worker's URL + a drain bearer in Bitwarden (drainer reads them):
bws secret create github-app.backup-worker-url "$GHB_WORKER_URL" "$(bws project list | jq -r '.[0].id')"
python3 -c "import secrets;print(secrets.token_hex(32))"  # <- put this value in BOTH:
bws secret create github-app.backup-worker-bearer "<value>" "$(bws project list | jq -r '.[0].id')"  # Bitwarden (this name)
#    ...and the Worker:  wrangler secret put DRAIN_BEARER
# 3. print the App manifest + click path:
python3 $GB/bin/app_setup.py manifest
# 4. submit the manifest as dezren39 at https://github.com/settings/apps/new
#    (manifest flow). GitHub redirects with ?code=XXXX. Then:
python3 $GB/bin/app_setup.py convert XXXX     # credentials -> Bitwarden by name
python3 $GB/bin/app_setup.py webhook-secret   # webhook secret -> Bitwarden by name
# 5. set the SAME webhook secret on the App (GitHub UI). The Worker
#    side is set from Bitwarden by name during deploy (§5); to redo it
#    by hand: cd $GB/worker && wrangler secret put GITHUB_WEBHOOK_SECRET
```

### Installing the App on an owner (do per owner)

1. Open `https://github.com/settings/apps/developing-today-backup/installations`
   (for an org: `https://github.com/organizations/<ORG>/settings/apps/developing-today-backup/installations`).
2. Install on the account/org, repository access = **All repositories**
   (so next week's new repos are covered without a revisit).
3. Verify:

```sh
python3 $GB/bin/app_setup.py install-report
```

Owners where Drew is only a member (NixOS, vertexinc) cannot install the
App — they stay sweep-only; that is expected and recorded in config.json.

## 4. Adding a new owner/org to the mirror set + sweep

```sh
# 1. config edit only — add the login to "owners" in:
$EDITOR $GB/config.json
# 2. commit + PR the config change (developing-today/code), merge, pull on laptop.
# 3. install the App on the new org (§3) if Drew is an admin there.
# 4. pick up the new owner immediately instead of waiting for Sunday:
python3 $GB/bin/reconcile.py
```

A new giant fork appearing anywhere is NOT auto-cloned: the sweep lists
it under "pending Drew review" in `reconcile-*.md`. To approve one, add
it to `force_include` (with a reason) in config.json and re-run reconcile.

## 5. Worker deploy / upgrade

Deploys follow the shape of the repo's Cloudflare Worker deployment
standard (`doc/2026-10-31T23-00-00Z_design_cloudflare_worker_deployment_standard.md`):
a just recipe calls the nix flake app, secrets are gathered at deploy
time (Bitwarden by name + SOPS account id), a version is uploaded
first (tagged `git-<sha12>`), and going live is a separate deployment
at 100% — so rollback is `--deploy-version <id>` at an old version.

```sh
just deploy-developing-today-backup     # from the repo root
# equivalently: nix run .#developing-today-backup-deploy
curl https://developing-today-backup.developing-today.workers.dev/health
```

The deploy script drives wrangler's versions commands rather than
the standard's cf CLI: this Worker is a dependency-free plain-JS
worker configured by `wrangler.toml`, while the cf flow is built
around a TypeScript `cf/config` project (tool gap recorded on #3215).
The deploy also (re)sets the Worker secrets `DRAIN_BEARER` and
`GITHUB_WEBHOOK_SECRET` from Bitwarden by name; no secret is written
to disk or printed.

First time only — the KV namespace the Worker binds:

```sh
cd $GB/worker
wrangler kv namespace create GHB_EVENTS   # paste the id into wrangler.toml
```

History: the Worker was first deployed 2026-10-31 as
`hatch-repo-backup-webhook` (bare wrangler) and renamed the same day
to `developing-today-backup` through this flow; the old Worker was
deleted after the rename was verified live.

## 6. Upgrade (machines)

```sh
cd ~/git/code && git pull
$GB/systemd/install-units.sh              # reinstalls units from the checkout
systemctl --user restart github-backup-drain.timer
```

## 7. Fresh-laptop reinstall

```sh
# prerequisites: gh auth login (as dezren39), ~/tools/bws-env.sh present,
# code cloned to ~/git/code, laptop timers pattern available.
GB=~/git/code/infra/github-backup
# Option A — copy the tree from the old machine (fast, keeps alternates valid
# only if paths are identical; the tree root must stay ~/backups/github):
rsync -a old-laptop:~/backups/github/ ~/backups/github/
# Option B — rebuild from GitHub (config + MANIFEST are the spec):
mkdir -p ~/backups/github
python3 $GB/bin/reconcile.py && python3 $GB/bin/manifest.py
# then:
$GB/systemd/install-units.sh
```

## 8. Restore from a mirror

```sh
# normal repo:
git clone ~/backups/github/<owner>/<repo>.git ./<repo>
# cold/off-site copy of one repo (materializes alternates — safe to move):
git -C ~/backups/github/<owner>/<repo>.git bundle create /tmp/<repo>.bundle --all
git bundle verify /tmp/<repo>.bundle
git clone /tmp/<repo>.bundle ./<repo>
```

## 9. Disaster recovery (tree lost, GitHub intact)

Config (this repo) + Bitwarden are the only inputs. §7 Option B rebuilds
the entire tree; MANIFEST.json regenerates at the end. Exclusions and
gaps in config.json reproduce exactly the intended non-coverage.

## 10. Health checks

```sh
systemctl --user list-timers 'github-backup-*'
tail -20 ~/backups/github/fetch.log
ls ~/backups/github/reconcile-*.md | tail -3
python3 $GB/bin/manifest.py   # refresh totals after any manual change
gh api rate_limit --jq '.resources.core'   # personal-token pool (sweep)
python3 $GB/bin/app_setup.py install-report  # App installations
```
