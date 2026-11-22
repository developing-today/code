#!/usr/bin/env bash
# install.sh — install or refresh the pm-console user unit on the host
# that runs the pm stack. Safe to run again: it never overwrites an
# existing environment file.
#
# Usage: bash install.sh
set -euo pipefail

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
systemctl --user daemon-reload
systemctl --user enable pm-console.service >/dev/null
systemctl --user restart pm-console.service

PORT="$(sed -n 's/^PM_CONSOLE_PORT=//p' "$STATE/.env" | tail -1)"
PORT="${PORT:-4872}"
for _ in 1 2 3 4 5 6 7 8 9 10; do
  if curl -fsS "http://127.0.0.1:$PORT/console/health" >/dev/null 2>&1; then
    echo "pm-console: running on http://127.0.0.1:$PORT/console/"
    exit 0
  fi
  sleep 1
done
echo "pm-console: did not answer its health check; see: journalctl --user -u pm-console -n 50" >&2
exit 1
