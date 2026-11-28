#!/usr/bin/env bash
# install.sh — install or refresh the pm-console user unit on the host
# that runs the pm stack. Safe to run again: it never overwrites an
# existing environment file.
#
# Usage: bash install.sh
set -euo pipefail

if [ "$#" -ne 0 ]; then
  echo "usage: bash install.sh   (it takes no arguments)" >&2
  exit 2
fi

cd "$(dirname "$0")"
SRC="$PWD"
EXPECTED="$HOME/git/code/pm-stack/console"
if [ "$SRC" != "$EXPECTED" ]; then
  echo "install.sh: run from the primary checkout at $EXPECTED (the unit runs that checkout)" >&2
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
# sign-in on by itself once its two secrets exist. Another puts merged
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
for _ in 1 2 3 4 5 6 7 8 9 10; do
  if curl -fsS "http://127.0.0.1:$PORT/console/health" >/dev/null 2>&1; then
    echo "pm-console: running on http://127.0.0.1:$PORT/console/"
    exit 0
  fi
  sleep 1
done
echo "pm-console: did not answer its health check; see: journalctl --user -u pm-console -n 50" >&2
exit 1
