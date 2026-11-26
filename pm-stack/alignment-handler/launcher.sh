#!/usr/bin/env bash
# Alignment request handler service launcher (code#6651).
#
# This service consumes the existing Archer collector's SQLite store,
# and sends only in #alignment (the written channel allowlist). The
# bot token is resolved by name at service start and kept only in this
# process environment.
set -euo pipefail

if [[ "$#" -ne 0 ]]; then
  echo "usage: alignment-handler" >&2
  exit 2
fi

if ! SLACK_BOT_TOKEN="$(fleet secrets get slack.archer-bot-token)"; then
  echo "alignment-handler: fleet secrets get slack.archer-bot-token failed; refusing to start" >&2
  exit 78
fi
if [[ -z "${SLACK_BOT_TOKEN}" ]]; then
  echo "alignment-handler: slack.archer-bot-token resolved empty; refusing to start" >&2
  exit 78
fi
export SLACK_BOT_TOKEN

DB="${ARCHER_SLACK_DB:-$HOME/workspace/archer-slack/puller/state/archer-slack.db}"
mkdir -p "$(dirname "$DB")"
export ARCHER_SLACK_DB="$DB"
export ALIGNMENT_HANDLER_BOT_USER_ID="${ALIGNMENT_HANDLER_BOT_USER_ID:-U0CBTJGQQSZ}"

exec @python@ @handlerPy@ watch --db "$DB" --bot-user-id "$ALIGNMENT_HANDLER_BOT_USER_ID"
