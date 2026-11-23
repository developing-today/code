#!/usr/bin/env bash
# Archer Slack collector service launcher (#2137 S2) — the packaged
# entry point the laptop's systemd user unit runs
# (nixos/services/archer-slack). run.sh remains the checkout/manual
# launcher (Bitwarden-direct); this is the deployment form.
#
# Fetches the app-level token BY NAME at start via the fleet secrets
# route (fleet secrets get: broker-first, Bitwarden fallback inside
# the verb — the estate's default fetch route, Drew 2026-11-04). The
# value lands only in this process's environment: never printed,
# logged, or written to disk. Ensures the store's directory exists,
# then execs the collector with the packaged interpreter.
#
# Argument contract (#3352): this launcher declares no arguments of
# its own; any arguments are passed verbatim to collector.py, whose
# argparse surface (--db / --channels) is the strict parser for them.
# Configuration normally arrives via ARCHER_SLACK_DB /
# ARCHER_WATCH_CHANNELS, which collector.py reads natively; the unit
# sets both. `fleet` is resolved from PATH (the unit pins a PATH that
# includes the fleet install dir).
set -euo pipefail

if ! SLACK_APP_TOKEN="$(fleet secrets get slack.archer-app-token)"; then
  echo "archer-slack-collector: fleet secrets get slack.archer-app-token failed; refusing to start tokenless" >&2
  exit 78
fi
if [ -z "${SLACK_APP_TOKEN}" ]; then
  echo "archer-slack-collector: slack.archer-app-token resolved empty; refusing to start" >&2
  exit 78
fi
export SLACK_APP_TOKEN

DB="${ARCHER_SLACK_DB:-$HOME/workspace/archer-slack/puller/state/archer-slack.db}"
mkdir -p "$(dirname "$DB")"
export ARCHER_SLACK_DB="$DB"

exec @python@ @collectorPy@ "$@"
