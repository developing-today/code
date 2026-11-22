#!/usr/bin/env bash
# redeploy.sh — put merged console code into service on the host.
#
# pm-console-redeploy.timer runs this every minute. It compares the
# console's code in the checkout (which pm-git-sync keeps at main) with
# the code that was last put into service. When they differ it first
# starts the new code in a self-check that listens on nothing, with the
# service's own settings; only if that passes does it run install.sh,
# which restarts the service. Code that does not start is left out of
# service, the old code keeps running, and this unit fails, which the
# console's Services screen and its notifications show. It is tried
# again only when the checkout moves to different code.
#
# Usage: bash redeploy.sh
set -euo pipefail

if [ "$#" -ne 0 ]; then
  echo "usage: bash redeploy.sh   (it takes no arguments)" >&2
  exit 2
fi

cd "$(dirname "$0")"
SRC="$PWD"
REPO="$(git -C "$SRC" rev-parse --show-toplevel)"
STATE="$HOME/.local/state/pm-console"
BUN="${PM_CONSOLE_BUN:-/run/current-system/sw/bin/bun}"
UNITS=(pm-console.service pm-console-expose.service pm-console-expose.timer pm-console-redeploy.service pm-console-redeploy.timer)

# What is in the checkout: the console's tree, and the units it runs
# under. A checkout that is part-way through an update (files that do
# not match its commit yet) is left for the next run.
paths=(pm-stack/console)
for unit in "${UNITS[@]}"; do paths+=("pm-stack/systemd/$unit"); done
if ! git -C "$REPO" diff --quiet HEAD -- "${paths[@]}"; then
  exit 0
fi
want="$(for path in "${paths[@]}"; do git -C "$REPO" rev-parse "HEAD:$path"; done | tr '\n' ' ')"
have="$(cat "$STATE/deployed" 2>/dev/null || true)"
if [ "$want" = "$have" ]; then
  exit 0
fi
if [ "$want" = "$(cat "$STATE/deploy-failed" 2>/dev/null || true)" ]; then
  echo "redeploy: this code did not pass its self-check (see $STATE/redeploy.log); the code in service is unchanged" >&2
  exit 1
fi

mkdir -p "$STATE"
# The service's own settings, one KEY=value a line, passed as they are.
settings=()
if [ -f "$STATE/.env" ]; then
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
    '' | '#'* | ';'*) continue ;;
    *=*) settings+=("$line") ;;
    esac
  done <"$STATE/.env"
fi
if ! (cd "$SRC" && env "${settings[@]}" "$BUN" run src/server.ts --check) >"$STATE/redeploy.log" 2>&1; then
  printf '%s' "$want" >"$STATE/deploy-failed"
  echo "redeploy: the new code does not start; the code in service is unchanged:" >&2
  tail -n 20 "$STATE/redeploy.log" >&2
  exit 1
fi

bash "$SRC/install.sh"
printf '%s' "$want" >"$STATE/deployed"
rm -f "$STATE/deploy-failed"
echo "redeploy: $(git -C "$REPO" rev-parse --short HEAD) is in service"
