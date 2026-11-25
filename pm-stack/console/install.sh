#!/usr/bin/env bash
# install.sh — install or refresh the console's units on this machine
# from the release this file is part of, restart the console, and wait
# for it to answer as that release.
#
# redeploy.sh runs it once it has made a release current; that is the
# only way it is meant to run. Safe to run again: it never overwrites an
# existing environment file.
#
# Usage: bash install.sh
set -euo pipefail

if [ "$#" -ne 0 ]; then
  echo "usage: bash install.sh   (it takes no arguments)" >&2
  exit 2
fi

cd "$(dirname "$0")"
SRC="$(pwd -P)"
SHARE="$HOME/.local/share/pm-console"
SERVED="$(cd "$SHARE/current/pm-stack/console" 2>/dev/null && pwd -P || true)"
if [ "$SRC" != "$SERVED" ]; then
  echo "install.sh: this is not the release in service ($SHARE/current). Run redeploy.sh: it makes a release from main and then runs this." >&2
  exit 1
fi
STATE="$HOME/.local/state/pm-console"
UNIT_SRC="$(dirname "$SRC")/systemd/pm-console.service"
UNIT_DST="$HOME/.config/systemd/user/pm-console.service"

mkdir -p "$STATE" "$(dirname "$UNIT_DST")"
chmod 700 "$STATE"
if [ ! -f "$STATE/.env" ]; then
  # No issuer yet: requests from the edge are refused until
  # scripts/expose.py writes the sign-in settings.
  umask 077
  printf 'PM_CONSOLE_PORT=4872\nPM_CONSOLE_OWNER_EMAILS=\n' >"$STATE/.env"
fi
install -m 644 "$UNIT_SRC" "$UNIT_DST"
# One timer keeps the public address as declared, and turns Google
# sign-in on by itself once its two secrets exist. Another puts main's
# console code into service, so a merge needs nobody at this machine.
# The third signs the Android build GitHub makes, here where the key is,
# and releases it.
TIMERS=(pm-console-expose pm-console-redeploy pm-console-android-release)
for timer in "${TIMERS[@]}"; do
  for unit in "$timer.service" "$timer.timer"; do
    install -m 644 "$(dirname "$SRC")/systemd/$unit" "$(dirname "$UNIT_DST")/$unit"
  done
done
systemctl --user daemon-reload
systemctl --user enable pm-console.service >/dev/null
systemctl --user restart pm-console.service
systemctl --user enable --now "${TIMERS[@]/%/.timer}" >/dev/null

# systemd strips one pair of quotes from an EnvironmentFile value, so
# PM_CONSOLE_PORT="4872" is a valid line: read it the same way.
PORT="$(sed -n 's/^[[:space:]]*PM_CONSOLE_PORT=//p' "$STATE/.env" | tail -1 | tr -d '[:space:]')"
PORT="${PORT#[\"\']}"
PORT="${PORT%[\"\']}"
PORT="${PORT:-4872}"
case "$PORT" in
*[!0-9]*)
  echo "install.sh: PM_CONSOLE_PORT in $STATE/.env is not a number: $PORT" >&2
  exit 1
  ;;
esac
# The console says which commit it was started from. It has come up
# when it answers as this release's: an answer from anything else (the
# code before it, still running) is not that.
WANT=""
if [ -f "$SRC/.release" ]; then
  WANT="$(sed -n 's/^commit=\([0-9a-f]\{12\}\).*/\1/p' "$SRC/.release" | head -n 1)"
fi
WAIT="${PM_CONSOLE_WAIT_SECONDS:-20}"
answered=no
got=""
for _ in $(seq 1 "$WAIT"); do
  if said="$(curl -fsS "http://127.0.0.1:$PORT/console/health" 2>/dev/null)"; then
    answered=yes
    got="$(printf '%s' "$said" | sed -n 's/.*"version":"\([0-9a-f]\{12\}\)".*/\1/p')"
    if [ -z "$WANT" ] || [ "$got" = "$WANT" ]; then
      echo "pm-console: ${WANT:-this release} is running on http://127.0.0.1:$PORT/console/"
      exit 0
    fi
  fi
  sleep 1
done
if [ "$answered" = yes ]; then
  echo "pm-console: it answers its health check as ${got:-a build that does not say which it is}, not as $WANT; see: journalctl --user -u pm-console -n 50" >&2
else
  echo "pm-console: did not answer its health check; see: journalctl --user -u pm-console -n 50" >&2
fi
exit 1
