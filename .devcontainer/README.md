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
- `postCreateCommand` warms the `./src/id` dev shell, so Codespaces
  **prebuilds** (repo Settings → Codespaces → Prebuilds, trigger: on
  push, branch: main) snapshot a ready-to-work environment. Prebuilds run
  on Actions minutes, which are free/unlimited for this public repo.

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
