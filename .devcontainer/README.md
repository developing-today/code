# Codespaces pilot — developing-today/code

Pilot for the fleet Codespaces proposal (tracked goal: "Efficient use of
Codespaces, Actions, and Blacksmith"). Codespaces here are **disposable,
prebuilt, per-repo work boxes**: overflow compute and anywhere access,
not a server and not a laptop replacement.

## Start one

GitHub repo page → **Code → Codespaces → Create codespace on \<branch\>**,
or open any PR branch in a codespace to verify it in a clean environment.
Default machine: **2-core** (GitHub Pro includes 180 core-hours/month =
90 clock hours at 2-core; use 4-core only for build-heavy sessions).

## How it works

- The image (Dockerfile) is thin: Ubuntu + direnv + single-user Nix with
  flakes enabled. The repo's own flakes provide the real toolchain via
  `.envrc`, exactly as on the laptop.
- `postCreateCommand` warms the `./src/id` dev shell (Rust/web tooling)
  and verifies the image-provided tools. Go 1.25 comes from a devcontainer
  feature — the repo's flakes do not provide Go; on the laptop it comes
  from home-manager.
- **Prebuilds are NOT available for this repo** (verified 2026-10-25):
  the repo is owned by the `developing-today` organization, repository
  Codespaces/prebuild settings require an organization Team/Enterprise
  plan, and the repo settings page returns 404 on the current plan.
  Codespaces themselves bill to the personal Pro account (user
  ownership). Until that changes, the pattern is: keep one codespace per
  active repo, **stop it instead of deleting it** between sessions
  (restart is fast; a fresh create pays the full image build + dev-shell
  realization, ~27 min in the pilot's first run), and delete it when the
  workstream ends.

## Rules of the house

- **Delete the codespace when the work merges.** Codespaces storage
  (20 GB included across live codespaces) is the meter that bites.
- Set a short personal idle timeout (Settings → Codespaces, ~15 min) —
  core-hours burn while a codespace sits running.
- API keys and tokens go in **Codespaces secrets**, never in this config.
- Not for: long-running daemons (idle-stop kills them), GPU work (none
  available), or anything that needs the home LAN/tailnet.

## After the home server farm exists

Codespaces shrink to the anywhere/travel/disposable layer; this
devcontainer stays as the environment spec the farm images mirror.
