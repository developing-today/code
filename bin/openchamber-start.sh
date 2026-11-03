#!/bin/bash
# Laptop start wrapper for OpenChamber (fleet pattern, mirrors
# bin/commandcode-proxy-start.sh). The home-manager unit does not exec
# the serve script directly; it execs this wrapper with the serve
# script's store path as $1 (openchamberUnitStart in
# home/common/default.nix).
#
# openchamber#766: OpenChamber attaches to the single standing OpenCode
# server instead of spawning a managed instance. The attach is driven
# entirely by the three exports below. OPENCODE_PASSWORD is the standing
# server's password, read at start time from the server's own state
# file -- it is never in the Nix store, a unit file, or this repo. This
# is the durable form of the hand-applied
# ~/.config/systemd/user/openchamber.service.d/external-opencode.conf
# drop-in. Rollback: revert the unit to exec the serve script directly
# (no exports), daemon-reload, restart openchamber.service.
export HOME=/home/user
export PATH="/run/current-system/sw/bin:$HOME/.local/bin:/usr/local/bin:$PATH"

serve="$1"
if [ -z "$serve" ]; then
  echo "openchamber-start: no serve script argument given (the unit shim passes openchamberStart)" >&2
  exit 1
fi

state="$HOME/.local/state/opencode/service-prod.json"
if [ ! -r "$state" ]; then
  echo "openchamber-start: $state is not readable -- refusing to start OpenChamber without the standing OpenCode server's state (openchamber#766)" >&2
  exit 1
fi
export OPENCODE_PASSWORD="$(jq -r .password "$state")"
if [ -z "$OPENCODE_PASSWORD" ] || [ "$OPENCODE_PASSWORD" = "null" ]; then
  echo "openchamber-start: no password in $state -- refusing to start OpenChamber unattached (openchamber#766)" >&2
  exit 1
fi
export OPENCODE_HOST="${OPENCODE_HOST:-http://127.0.0.1:13420}"
export OPENCODE_SKIP_START=true

exec "$serve"
