# Design: the Cloudflare Worker deployment standard

- Date: 2026-10-31
- Status: **pilot implemented** — the pkarr Worker (`pkgs/pkarr-worker`,
  issue #1829) is the first slice, deployed through this standard.
- Issue: #1828 (design + owner ruling). Owner ruling, 2026-10-28: "move
  forward with this design"; re-issued 2026-10-31 with the explicit
  instruction to deploy via **cf versions**.
- Evidence: the Workers Versions & Deployments research (2026-10-29) —
  versions and deployments are separate API objects; rollback is a new
  deployment at an old version id; versions carry a git SHA tag/message;
  a `?deployable=true` filter pre-checks deployability; the account
  retains a rolling window of versions (100 in the research).

## The shape

Every deployable Worker in this repo deploys the same way:

```
just deploy-<name>            (root.just recipe)
  └─ nix run .#<name>-deploy  (flake app: a writeShellApplication)
       ├─ impure step: gather secrets at deploy time
       │    - account id from SOPS (secrets/sops/common/cloudflare.yaml)
       │    - API token from Bitwarden, by secret name
       └─ pure step: a deploy script in the package drives the cf CLI
            1. cf workers versions create --tag git-<sha12>
               --message "deploy <full-sha> ..."
            2. REST pre-check: version ∈ ?deployable=true set
            3. cf workers deployments create --versions
               '[{"version_id": "...", "percentage": 100}]'
```

Rules carried from the #1828 ruling:

- **cf first.** The `cf` CLI (`pkgs/cf`) is the deploy tool; wrangler is
  the fallback only where cf's surface genuinely cannot do a step, and
  any such gap is recorded on the pilot issue. For the pkarr pilot, no
  fallback was needed: cf covers versions create/list and deployments
  create/list.
- **No plaintext secrets in the repo or the Nix store.** Secrets are
  gathered impurely at deploy time (Bitwarden Secrets Manager + SOPS)
  and passed to cf through the environment only.
- **Never deploy via a NixOS switch.** A Worker deploy is its own
  entrypoint, run from a checkout on a machine that can decrypt the
  SOPS file (the laptop's age key), not a system activation.
- **Upload ≠ deploy.** A version is uploaded and stamped first; going
  live is a separate deployment object at 100%. That separation is
  what makes rollback a first-class operation instead of a redeploy:
  `deploy.mjs --deploy-version <id>` creates a deployment at any
  retained version, forward or back.
- **Bootstrap exception.** `cf workers versions create` cannot create
  a brand-new Worker (found by the pilot's first deploy run; recorded
  on #1829). A Worker's first-ever deploy therefore goes through
  `cf deploy` with the same tag/message, which creates version 1 and
  its deployment together; the versions flow governs everything
  after.
- **State handling is deferrable.** No terraform/terranix in the pilot;
  the Worker's own config (`cloudflare.config.ts`, mirrored by
  `wrangler.jsonc` for dev/tests) is the resource declaration.

## The pilot

`pkgs/pkarr-worker` implements the standard end to end:

- `deploy.nix` — `writeShellApplication` with `cf`, `bws`, `sops`,
  `git`, `nodejs` as runtime inputs; exposed as flake package
  `pkarrDeploy` and app `pkarr-deploy` (`lib/default.nix`).
- `deploy.sh` — sources `lib/bitwarden-env.sh`, SOPS-extracts the
  account id, execs the deploy script.
- `deploy.mjs` — resolves the token from Bitwarden by name
  (`cloudflare.api-token-day-to-day` by default), installs dev
  dependencies on a fresh checkout, then runs the versions flow above.
- `root.just` — the `deploy-pkarr` recipe (`[group('pkarr')]`).

Precedent: the mail Worker (`nixos/services/id-mail/worker`) established
the just → nix run → secret-gathering → deploy-script shape; this
standard keeps that shape and swaps its wrangler deploy for the cf
versions flow.

## Rollout beyond the pilot

Other Workers adopt the standard by adding the same three files to
their package and a `deploy-<name>` recipe; nothing in the flow is
pkarr-specific except names and the token secret name. Known gaps and
follow-ups are tracked on #1829.
