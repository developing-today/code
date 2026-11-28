#!/usr/bin/env bash
# Archer Slack collector launcher (#2137).
# Fetches the app-level token from Bitwarden Secrets Manager BY NAME at
# start (value is never printed, logged, or written to disk), then execs
# the collector. Mirrors how archer-slack-review obtains its bot token.
#
# Repo form of the 2026-10-29 sandbox PoC launcher: paths resolve from
# this script's own directory instead of a hardcoded workspace home, so
# the same launcher works from a repo checkout on the deployment host.
# The Bitwarden tooling paths default to this fleet's seat layout and
# can be overridden with BWS_ENV / BWS_BIN.
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${BWS_ENV:-$HOME/tools/bws-env.sh}"
BWS="${BWS_BIN:-$HOME/tools/bin/bws}"
ID="$("$BWS" secret list -o json | jq -r '.[] | select(.key=="slack.archer-app-token") | .id' | head -n1)"
if [ -z "$ID" ]; then
  echo "collector launcher: slack.archer-app-token not found in Bitwarden" >&2
  exit 78
fi
SLACK_APP_TOKEN="$("$BWS" secret get "$ID" -o json | jq -j 'if type=="array" then .[0].value else .value end')"
export SLACK_APP_TOKEN
cd "$DIR"
exec "$DIR/.venv/bin/python" collector.py
