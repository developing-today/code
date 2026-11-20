#!/usr/bin/env bash
# pm-git-sync — keep the laptop's primary code clone on a clean, current
# main (Drew, 2026-10-27).
#
# This is the repo copy of the script (pm-stack/scripts/); the pm-git-sync
# user unit is declared in nixos/services/pm-stack and runs the packaged
# copy from the Nix store (pkgs/pm-git-sync). The pre-NixOS hand copy at
# ~/tools/pm-git-sync.sh is retired — do not edit it; edit this one.
#
# Deploy model (2026-11-10, code#8171): the PM services (pm-projector,
# pm-aggregator, pm-dispatch) are Nix packages built from this repo
# (pkgs/pm-*) and run from the store. Code changes deploy by merge ->
# nixos-rebuild switch, NOT by pulling this clone and restarting units.
# Until 2026-11-10 this script also restarted those services when their
# subtree changed, running dependency steps first and retrying failed
# redeploys until they landed (#2391); that machinery — and its harness
# pm-git-sync.test.sh — retired with the checkout-run model, so a pull
# here never restarts a PM service.
#
# What remains is the clone hygiene the rest of the fleet relies on:
# - If the clone is not on main, or has any tracked modification
#   (staged or not), FAIL LOUDLY (journal) naming the dirty paths and do
#   NOT pull. A human/agent resolves that first; this service never
#   stashes, commits, or discards anything.
# - Untracked (not ignored) files are MOVED ASIDE, not left to block the
#   pull (2026-11-20): each one goes to
#   $PM_GIT_SYNC_QUARANTINE/<UTC stamp>/<same relative path>
#   (default ~/backups/pm-git-sync-quarantine), and every move is logged.
#   Agents and scheduled sessions whose cwd is this clone kept dropping
#   ledgers, logs and dashboards here (#5209, #5389, #5562, #8170, and
#   three stalls on 2026-11-17/19/20), and every one stopped the laptop
#   behind main until someone moved the file by hand. Nothing is deleted:
#   the file keeps its content and mtime in the quarantine folder.
# - Otherwise git fetch + git pull --ff-only.
set -uo pipefail
# Argument contract (#3352): this script takes no command-line
# arguments; anything given is a usage error before any work runs.
if [ "$#" -gt 0 ]; then
  echo "pm-git-sync: unexpected argument '$1'; usage: pm-git-sync (set the clone with PM_GIT_SYNC_REPO)" >&2
  exit 2
fi
REPO="${PM_GIT_SYNC_REPO:-$HOME/git/code}"
cd "$REPO" || {
  echo "pm-git-sync: FAIL cannot cd $REPO"
  exit 1
}
branch="$(git branch --show-current)"
if [ "$branch" != "main" ]; then
  echo "pm-git-sync: FAIL $REPO is on branch '$branch', not main — not pulling"
  exit 1
fi
dirty="$(git status --porcelain --untracked-files=no)"
if [ -n "$dirty" ]; then
  echo "pm-git-sync: FAIL $REPO working tree is dirty — not pulling. Dirty paths:"
  echo "$dirty"
  exit 1
fi
if [ -n "$(git ls-files --others --exclude-standard | head -n 1)" ]; then
  QUARANTINE_ROOT="${PM_GIT_SYNC_QUARANTINE:-$HOME/backups/pm-git-sync-quarantine}"
  qdir="$QUARANTINE_ROOT/$(date -u +%Y%m%dT%H%M%SZ)-$$"
  moved=0
  while IFS= read -r -d '' path; do
    dest="$qdir/$path"
    if ! mkdir -p "$(dirname "$dest")" || ! mv -n -- "$path" "$dest" || [ -e "$path" ]; then
      echo "pm-git-sync: FAIL could not move untracked '$path' to $dest — not pulling"
      exit 1
    fi
    echo "pm-git-sync: moved untracked '$path' -> $dest"
    moved=$((moved + 1))
  done < <(git ls-files --others --exclude-standard -z)
  echo "pm-git-sync: moved $moved untracked file(s) out of $REPO into $qdir"
  if [ -n "$(git status --porcelain)" ]; then
    echo "pm-git-sync: FAIL $REPO still dirty after moving untracked files — not pulling. Dirty paths:"
    git status --porcelain
    exit 1
  fi
fi
if ! git fetch origin --quiet; then
  echo "pm-git-sync: FAIL git fetch origin in $REPO"
  exit 1
fi
current="$(git rev-parse HEAD)"
remote="$(git rev-parse origin/main)"
if [ "$current" = "$remote" ]; then
  echo "pm-git-sync: ok clean main, up to date at $current"
  exit 0
fi
if ! git pull --ff-only origin main; then
  echo "pm-git-sync: FAIL git pull --ff-only $current -> $remote in $REPO"
  exit 1
fi
echo "pm-git-sync: updated $current -> $(git rev-parse HEAD)"
