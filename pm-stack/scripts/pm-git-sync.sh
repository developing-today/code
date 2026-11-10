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
# - Redeploys are owed until they succeed (#2391): if a service's
#   dependency step fails, that service is NOT restarted in that run;
#   the redeploy is recorded in the state dir and retried on every
#   subsequent run — including runs where HEAD does not move — until
#   the dependency step and the restart both succeed. A failed restart
#   is likewise recorded and retried. While any redeploy is owed, the
#   run FAILS LOUDLY (non-zero exit) so the state is visible in the
#   journal instead of a silent WARN with exit 0.
# - Successful deployments are tracked per unit (#7132): each unit's
#   last successfully deployed commit is recorded, and a unit
#   redeploys when its subtree at HEAD differs from the subtree at
#   ITS deployed commit — not merely when it changed across the
#   global baseline range. Without this, a unit restarted during a
#   partially failed batch is stranded if a later commit reverts its
#   subtree to the baseline content: the baseline-range diff is
#   empty, the rollback restart is skipped, and the baseline
#   advances while the unit keeps running the reverted content.
set -uo pipefail
REPO="${PM_GIT_SYNC_REPO:-$HOME/git/code}"
BUN="${PM_GIT_SYNC_BUN:-/run/current-system/sw/bin/bun}"
SYSTEMCTL="${PM_GIT_SYNC_SYSTEMCTL:-systemctl}"
STATE_DIR="${PM_GIT_SYNC_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/pm-git-sync}"
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

# The baseline is the last HEAD for which every changed subtree's
# redeploy completed. Keeping it across runs also catches a hand-pulled
# HEAD: in that case HEAD already equals origin/main, but still differs
# from this deployed baseline.
DEPLOYED_HEAD_FILE="$STATE_DIR/deployed-head"
current="$(git rev-parse HEAD)"
if [ -f "$DEPLOYED_HEAD_FILE" ]; then
  old="$(cat "$DEPLOYED_HEAD_FILE")"
  if ! git cat-file -e "$old^{commit}" 2>/dev/null; then
    echo "pm-git-sync: FAIL deployed-head baseline '$old' is not a commit in $REPO"
    exit 1
  fi
else
  # On the first run, assume the currently checked-out tree is the
  # deployed one; subsequent runs retain the exact deployed baseline.
  old="$current"
  if ! mkdir -p "$STATE_DIR" || ! printf '%s\n' "$old" >"$DEPLOYED_HEAD_FILE"; then
    echo "pm-git-sync: FAIL could not initialize deployed-head baseline in $STATE_DIR"
    exit 1
  fi
fi

# --- owed redeploys (#2391) -------------------------------------------------
# One record per unit: $STATE_DIR/owed-<unit>, line 1 = subtree,
# line 2 = dependency step (may be empty). Records are written only
# from the fixed dep-step templates below, never from outside input.
owed_file() { echo "$STATE_DIR/owed-$1"; }

record_owed() { # <unit> <subtree> <dep-step-or-empty>
  mkdir -p "$STATE_DIR" || return 1
  printf '%s\n%s\n' "$2" "$3" >"$(owed_file "$1")"
}

clear_owed() { # <unit>
  local f
  f="$(owed_file "$1")"
  if [ -e "$f" ]; then rm -f "$f"; fi
}

# --- deployed commits per unit (#7132) ---------------------------------------
# One record per unit: $STATE_DIR/deployed-<unit> holds the commit
# whose subtree the unit was last successfully restarted onto.
deployed_file() { echo "$STATE_DIR/deployed-$1"; }

record_deployed() { # <unit>
  mkdir -p "$STATE_DIR" || return 1
  printf '%s\n' "$new" >"$(deployed_file "$1")"
}

ATTEMPTED=" "
OWED_PERSIST_FAILED=0
DEPLOYED_PERSIST_FAILED=0

# attempt_redeploy <subtree> <unit> <dep-step-or-empty>
# Runs the dependency step (when given) and then the restart. A failed
# dependency step aborts the restart. Any failure records the redeploy
# as owed; success clears any owed record.
attempt_redeploy() {
  local subtree="$1" unit="$2" dep="$3"
  ATTEMPTED="$ATTEMPTED$unit "
  # Merge a persisted dependency obligation into this attempt: if an
  # earlier run owed this unit a dependency step, a newer pull that
  # does not itself change the dependency inputs must not restart the
  # service past the still-unresolved install (#2391, Codex P1).
  local of
  of="$(owed_file "$unit")"
  if [ -z "$dep" ] && [ -e "$of" ]; then
    dep="$(sed -n '2p' "$of")"
  fi
  if [ -n "$dep" ]; then
    if (cd "$subtree" && eval "$dep"); then
      echo "pm-git-sync: dep step done for $subtree"
    else
      echo "pm-git-sync: FAIL dep step failed for $subtree — $unit NOT restarted; redeploy owed"
      record_owed "$unit" "$subtree" "$dep" || OWED_PERSIST_FAILED=1
      return 1
    fi
  fi
  if "$SYSTEMCTL" --user restart "$unit"; then
    echo "pm-git-sync: $unit restarted ($subtree)"
    clear_owed "$unit"
    record_deployed "$unit" || DEPLOYED_PERSIST_FAILED=1
    return 0
  fi
  echo "pm-git-sync: FAIL $unit restart failed — redeploy owed"
  record_owed "$unit" "$subtree" "$dep" || OWED_PERSIST_FAILED=1
  return 1
}

# Retry every owed redeploy not already attempted in this run.
retry_owed() {
  local f unit subtree dep
  for f in "$STATE_DIR"/owed-*; do
    if [ ! -e "$f" ]; then continue; fi
    unit="${f##*/owed-}"
    case "$ATTEMPTED" in
    *" $unit "*) continue ;;
    esac
    subtree="$(sed -n '1p' "$f")"
    dep="$(sed -n '2p' "$f")"
    echo "pm-git-sync: retrying owed redeploy for $unit ($subtree)"
    attempt_redeploy "$subtree" "$unit" "$dep" || true
  done
}

owed_summary() { # prints owed unit names, one per line
  local f
  for f in "$STATE_DIR"/owed-*; do
    if [ -e "$f" ]; then echo "${f##*/owed-}"; fi
  done
}

# finish <ok-message>: retry owed redeploys, then exit loudly while
# any redeploy is still owed.
finish() {
  retry_owed
  local owed
  owed="$(owed_summary)"
  if [ "$OWED_PERSIST_FAILED" -ne 0 ]; then
    echo "pm-git-sync: FAIL could not persist an owed-redeploy record in $STATE_DIR — a failed redeploy may not be retried"
    exit 1
  fi
  if [ "$DEPLOYED_PERSIST_FAILED" -ne 0 ]; then
    echo "pm-git-sync: FAIL could not persist a deployed-commit record in $STATE_DIR — a successful redeploy may be misjudged on a later run"
    exit 1
  fi
  if [ -n "$owed" ]; then
    echo "pm-git-sync: FAIL redeploy(s) still owed for: $(echo "$owed" | tr '\n' ' ')"
    exit 1
  fi
  local baseline_tmp="$DEPLOYED_HEAD_FILE.tmp.$$"
  if ! printf '%s\n' "$new" >"$baseline_tmp" || ! mv -f "$baseline_tmp" "$DEPLOYED_HEAD_FILE"; then
    rm -f "$baseline_tmp"
    echo "pm-git-sync: FAIL could not persist deployed-head baseline $new in $STATE_DIR"
    exit 1
  fi
  echo "$1"
  exit 0
}

remote="$(git rev-parse origin/main)"
if [ "$current" != "$remote" ]; then
  if ! git pull --ff-only origin main; then
    echo "pm-git-sync: FAIL git pull --ff-only $current -> $remote in $REPO"
    exit 1
  fi
fi
new="$(git rev-parse HEAD)"
if ! git merge-base --is-ancestor "$old" "$new"; then
  echo "pm-git-sync: FAIL deployed-head baseline $old is not an ancestor of current HEAD $new"
  exit 1
fi
if [ "$old" = "$new" ]; then
  finish "pm-git-sync: ok clean main, up to date at $new"
fi
echo "pm-git-sync: updated deployed baseline $old -> $new"

# redeploy <subtree> <unit> <dep-step> <dep-input-path...>
# A unit redeploys when its subtree at HEAD differs from the subtree
# at the commit it last deployed successfully (its deployed record,
# or the global baseline when it has none). The dependency step runs
# when the dependency inputs changed across that same per-unit range.
redeploy() {
  local subtree="$1" unit="$2" dep_step="$3"
  shift 3
  local base="$old" f
  f="$(deployed_file "$unit")"
  if [ -e "$f" ]; then base="$(sed -n '1p' "$f")"; fi
  if [ "$(git rev-parse "$base:$subtree" 2>/dev/null)" = "$(git rev-parse "$new:$subtree" 2>/dev/null)" ]; then
    return 0
  fi
  local dep=""
  if [ $# -gt 0 ] && git diff --name-only "$base" "$new" -- "$@" | grep -q .; then
    dep="$dep_step"
  fi
  attempt_redeploy "$subtree" "$unit" "$dep" || true
}

# The aggregator installs with bun when its dependency inputs change.
redeploy pm-stack/aggregator pm-aggregator.service "$BUN install --silent" pm-stack/aggregator/package.json pm-stack/aggregator/bun.lock

# The projector provisions @libsql/client via setup-deps.sh when its
# package.json changes (bun install cannot reach the registry reliably
# from this host; see pm-stack/projector/scripts/setup-deps.sh).
redeploy pm-stack/projector pm-projector.service "bash scripts/setup-deps.sh" pm-stack/projector/package.json

# pm-dispatch has no dependency step: its package.json carries only
# type-level devDependencies (the runtime is pure Bun), and its env
# files are rendered by dispatch/install.sh, not on git pulls — a
# subtree change is a plain restart.
redeploy pm-stack/dispatch pm-dispatch.service ""

finish "pm-git-sync: updated $old -> $new, redeploys complete"
