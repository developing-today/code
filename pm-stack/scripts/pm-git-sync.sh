#!/usr/bin/env bash
# pm-git-sync — keep the laptop's primary code clone on a clean, current
# main, and redeploy the PM services from it (Drew, 2026-10-27).
#
# This is the repo copy of the script (pm-stack/scripts/); the pm-git-sync
# user unit is declared in nixos/services/pm-stack and runs THIS file out
# of the checkout. The pre-NixOS hand copy at ~/tools/pm-git-sync.sh is
# retired — do not edit it; edit this one.
#
# Rules:
# - If the clone is not on main, or the tree is dirty (any tracked
#   modification or untracked file), FAIL LOUDLY (journal) naming the
#   dirty paths and do NOT pull. A human/agent resolves the dirt first;
#   this service never stashes, commits, or discards anything.
# - Otherwise git fetch + git pull --ff-only.
# - If HEAD moved and a PM subtree changed, redeploy that service:
#   restart it, running its dependency step first when its dependency
#   inputs changed (bun install for the aggregator, setup-deps for the
#   projector).
set -uo pipefail
REPO="${PM_GIT_SYNC_REPO:-$HOME/git/code}"
BUN="${PM_GIT_SYNC_BUN:-/run/current-system/sw/bin/bun}"
cd "$REPO" || {
  echo "pm-git-sync: FAIL cannot cd $REPO"
  exit 1
}
branch="$(git branch --show-current)"
if [ "$branch" != "main" ]; then
  echo "pm-git-sync: FAIL $REPO is on branch '$branch', not main — not pulling"
  exit 1
fi
dirty="$(git status --porcelain)"
if [ -n "$dirty" ]; then
  echo "pm-git-sync: FAIL $REPO working tree is dirty — not pulling. Dirty paths:"
  echo "$dirty"
  exit 1
fi
if ! git fetch origin --quiet; then
  echo "pm-git-sync: FAIL git fetch origin in $REPO"
  exit 1
fi
old="$(git rev-parse HEAD)"
new="$(git rev-parse origin/main)"
if [ "$old" = "$new" ]; then
  echo "pm-git-sync: ok clean main, up to date at $old"
  exit 0
fi
if ! git pull --ff-only origin main; then
  echo "pm-git-sync: FAIL git pull --ff-only $old -> $new in $REPO"
  exit 1
fi
echo "pm-git-sync: updated $old -> $new"

redeploy() { # <subtree> <unit> <dep-step-or-empty>
  local subtree="$1" unit="$2" dep="$3"
  git diff --name-only "$old" "$new" -- "$subtree" | grep -q . || return 0
  if [ -n "$dep" ]; then
    (cd "$subtree" && eval "$dep") &&
      echo "pm-git-sync: dep step done for $subtree" ||
      echo "pm-git-sync: WARN dep step failed for $subtree"
  fi
  systemctl --user restart "$unit" &&
    echo "pm-git-sync: $unit restarted ($subtree changed)" ||
    echo "pm-git-sync: WARN $unit restart failed"
}

# The aggregator installs with bun when its dependency inputs change.
if git diff --name-only "$old" "$new" -- pm-stack/aggregator/package.json pm-stack/aggregator/bun.lock | grep -q .; then
  agg_dep="$BUN install --silent"
else
  agg_dep=""
fi
redeploy pm-stack/aggregator pm-aggregator.service "$agg_dep"

# The projector provisions @libsql/client via setup-deps.sh when its
# package.json changes (bun install cannot reach the registry reliably
# from this host; see pm-stack/projector/scripts/setup-deps.sh).
if git diff --name-only "$old" "$new" -- pm-stack/projector/package.json | grep -q .; then
  proj_dep="bash scripts/setup-deps.sh"
else
  proj_dep=""
fi
redeploy pm-stack/projector pm-projector.service "$proj_dep"

# pm-dispatch has no dependency step: its package.json carries only
# type-level devDependencies (the runtime is pure Bun), and its env
# files are rendered by dispatch/install.sh, not on git pulls — a
# subtree change is a plain restart.
redeploy pm-stack/dispatch pm-dispatch.service ""
