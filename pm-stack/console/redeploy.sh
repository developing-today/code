#!/usr/bin/env bash
# redeploy.sh — put main's console code into service on this machine.
#
# The console runs from a copy of main that is its own, under
# ~/.local/share/pm-console:
#
#   repo/                a clone that fetches only main from GitHub, and
#                        only the console's part of the tree
#   releases/<commit>/   the console's files at one commit of main
#   current              -> releases/<commit>: what is in service
#
# Nothing else works there. Before, the console ran from the checkout
# the fleet's lanes work in and was whatever that checkout held: main's
# code from the day before, or another branch's (#13833).
#
# pm-console-redeploy.timer runs this every minute. It asks GitHub for
# main. When the console's code there, or its units, are not what is in
# service, it lays that commit's files out as a new release and starts
# them in a self-check that listens on nothing, with the service's own
# settings. Only if that passes is the release made current and its
# install.sh run, which restarts the service. A release that does not
# start, or does not come up in service, does not stay: the one before
# it is put back, this unit fails (which the console's Services screen
# and its notifications show), and that code is not tried again until
# main's console code changes.
#
# It runs the same from anywhere, and the first run on a machine makes
# the copy.
#
# Usage: bash redeploy.sh
set -euo pipefail

if [ "$#" -ne 0 ]; then
  echo "usage: bash redeploy.sh   (it takes no arguments)" >&2
  exit 2
fi

SHARE="$HOME/.local/share/pm-console"
STATE="$HOME/.local/state/pm-console"
REMOTE="${PM_CONSOLE_REMOTE:-https://github.com/developing-today/code.git}"
BRANCH=main
BUN="${PM_CONSOLE_BUN:-/run/current-system/sw/bin/bun}"
UNITS=(pm-console.service pm-console-expose.service pm-console-expose.timer pm-console-redeploy.service pm-console-redeploy.timer
  pm-console-android-release.service pm-console-android-release.timer)
# What of the repository the console needs on this machine: itself, the
# units it runs under, and the dashboard's list of its own pages.
CONSOLE=pm-stack/console
PAGES=pm-stack/aggregator/src/pages.ts
# How long GitHub may be out of reach before saying so is a failure.
UNREACHED_GRACE_SECONDS=1800
# Releases kept besides the one in service and the one before it.
KEEP=3

mkdir -p "$SHARE/releases" "$STATE"
REPO="$SHARE/repo"
# One run at a time: a second one started meanwhile has nothing to add.
if command -v flock >/dev/null 2>&1; then
  exec 9>"$SHARE/.lock"
  flock -n 9 || exit 0
fi

# The clone, made if it is not there and put right if it has been
# changed: it asks GitHub for main alone, takes no file's contents until
# a checkout needs them, and checks out only the console's part.
if [ ! -d "$REPO/.git" ]; then
  rm -rf "$REPO"
  git init -q -b "$BRANCH" "$REPO"
fi
git -C "$REPO" config remote.origin.url "$REMOTE"
git -C "$REPO" config remote.origin.fetch "+refs/heads/$BRANCH:refs/remotes/origin/$BRANCH"
git -C "$REPO" config remote.origin.promisor true
git -C "$REPO" config remote.origin.partialclonefilter blob:none
git -C "$REPO" config core.sparseCheckout true
git -C "$REPO" config core.sparseCheckoutCone false
mkdir -p "$REPO/.git/info"
printf '/%s/\n/pm-stack/systemd/\n/%s\n' "$CONSOLE" "$PAGES" >"$REPO/.git/info/sparse-checkout"

if ! git -C "$REPO" fetch --quiet --depth 1 --filter=blob:none origin "$BRANCH" 2>"$STATE/fetch.log"; then
  now="$(date +%s)"
  first="$(cat "$STATE/unreached" 2>/dev/null || true)"
  case "$first" in
  '' | *[!0-9]*) first="$now" && printf '%s\n' "$first" >"$STATE/unreached" ;;
  esac
  if [ "$((now - first))" -ge "$UNREACHED_GRACE_SECONDS" ]; then
    echo "redeploy: GitHub has not answered for $(((now - first) / 60)) minutes; the code in service is unchanged:" >&2
    tail -n 5 "$STATE/fetch.log" >&2
    exit 1
  fi
  exit 0
fi
rm -f "$STATE/unreached"
sha="$(git -C "$REPO" rev-parse --verify "refs/remotes/origin/$BRANCH^{commit}")"

# What main holds of the console: its tree, each unit, and the list of
# pages. (Names in a tree, so nothing has to be downloaded to tell.)
tree="$(git -C "$REPO" rev-parse --verify "$sha:$CONSOLE")"
want="$tree"
for unit in "${UNITS[@]}"; do
  want+=" $(git -C "$REPO" rev-parse --verify "$sha:pm-stack/systemd/$unit")"
done
want+=" $(git -C "$REPO" rev-parse --verify --quiet "$sha:$PAGES" || echo none)"
have="$(cat "$STATE/deployed" 2>/dev/null || true)"
if [ "$want" = "$have" ] && [ -f "$SHARE/current/$CONSOLE/src/server.ts" ]; then
  exit 0
fi
if [ "$want" = "$(cat "$STATE/deploy-failed" 2>/dev/null || true)" ]; then
  echo "redeploy: main's console code did not go into service when it was tried (see $STATE/redeploy.log); the code in service is unchanged" >&2
  exit 1
fi

# That commit's files, laid out as a release of their own. The clone's
# working tree is made exactly the commit first: nothing edited there,
# and nothing left behind from a commit before.
git -C "$REPO" checkout --quiet --force --detach "$sha"
git -C "$REPO" clean -qfdx
release="$SHARE/releases/$sha"
rm -rf "$release" "$release.new"
mkdir -p "$release.new"
cp -a "$REPO/pm-stack" "$release.new/pm-stack"
printf 'commit=%s\ntree=%s\n' "$sha" "$tree" >"$release.new/$CONSOLE/.release"
mv "$release.new" "$release"

# The service's own settings, read the way systemd reads them: one
# KEY=value a line, and one pair of quotes around a value is not part of
# the value (install.sh reads PM_CONSOLE_PORT="4872" the same way).
settings=()
if [ -f "$STATE/.env" ]; then
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
    '' | '#'* | ';'*) continue ;;
    *=*)
      key="${line%%=*}"
      value="${line#*=}"
      case "$value" in
      \"*\") value="${value#\"}" && value="${value%\"}" ;;
      \'*\') value="${value#\'}" && value="${value%\'}" ;;
      esac
      settings+=("$key=$value")
      ;;
    esac
  done <"$STATE/.env"
fi
if ! (cd "$release/$CONSOLE" && env "${settings[@]}" "$BUN" run src/server.ts --check) >"$STATE/redeploy.log" 2>&1; then
  printf '%s' "$want" >"$STATE/deploy-failed"
  rm -rf "$release"
  echo "redeploy: main's console code does not start; the code in service is unchanged:" >&2
  tail -n 20 "$STATE/redeploy.log" >&2
  exit 1
fi

# Into service: the link is moved in one step, and the release's own
# install.sh restarts the service and waits for it to answer as itself.
serve() {
  ln -sfn "$1" "$SHARE/current.new"
  mv -T "$SHARE/current.new" "$SHARE/current"
  bash "$SHARE/current/$CONSOLE/install.sh"
}
before="$(readlink "$SHARE/current" 2>/dev/null || true)"
if ! serve "releases/$sha"; then
  printf '%s' "$want" >"$STATE/deploy-failed"
  if [ -n "$before" ] && [ "$before" != "releases/$sha" ] && [ -f "$SHARE/$before/$CONSOLE/install.sh" ]; then
    if serve "$before"; then
      echo "redeploy: ${sha:0:12} did not come up in service; ${before#releases/} is back in service" >&2
    else
      echo "redeploy: ${sha:0:12} did not come up in service, and neither did ${before#releases/} when it was put back" >&2
    fi
  else
    echo "redeploy: ${sha:0:12} did not come up in service, and there is no release before it to put back" >&2
  fi
  exit 1
fi
printf '%s' "$want" >"$STATE/deployed"
rm -f "$STATE/deploy-failed"

# Old releases are cleared away: the newest few are kept, and always
# the one in service and the one before it.
kept=0
while IFS= read -r name; do
  case "$name" in
  *[!0-9a-f]* | '') continue ;;
  esac
  [ "${#name}" -eq 40 ] || continue
  if [ "releases/$name" = "releases/$sha" ] || [ "releases/$name" = "$before" ]; then
    continue
  fi
  kept=$((kept + 1))
  if [ "$kept" -gt "$((KEEP - 2))" ]; then
    rm -rf "${SHARE:?}/releases/$name"
  fi
done < <(ls -1t "$SHARE/releases")
echo "redeploy: ${sha:0:12} is in service"
