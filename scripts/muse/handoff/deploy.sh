#!/usr/bin/env bash
# Deploy the one-time handoff Worker. Credentials are read from BWS and passed via stdin/env.
set -euo pipefail
set +x

HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE/worker"

CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
STATE="$CONFIG_HOME/muse-agent-handoff"
BWS_FILE="${BWS_ACCESS_TOKEN_FILE:-$CONFIG_HOME/bitwarden/access-token}"
if [ ! -s "$BWS_FILE" ] && [ -s "$CONFIG_HOME/bws/access-token" ]; then
  BWS_FILE="$CONFIG_HOME/bws/access-token"
fi
mkdir -p "$STATE"
chmod 700 "$STATE"
[ -s "$BWS_FILE" ] || {
  echo "handoff deploy: missing BWS token file at $BWS_FILE" >&2
  exit 1
}
command -v bws >/dev/null || {
  echo "handoff deploy: bws is not on PATH" >&2
  exit 1
}
command -v wrangler >/dev/null || {
  echo "handoff deploy: wrangler is not on PATH" >&2
  exit 1
}

umask 077
[ -s "$STATE/admin-token" ] || python3 -c 'import secrets; print(secrets.token_urlsafe(48))' >"$STATE/admin-token"
[ -s "$STATE/encryption-key" ] || python3 -c 'import base64,secrets; print(base64.b64encode(secrets.token_bytes(32)).decode())' >"$STATE/encryption-key"
chmod 600 "$STATE/admin-token" "$STATE/encryption-key"

BWS_ACCESS_TOKEN="$(cat "$BWS_FILE")"
export BWS_ACCESS_TOKEN
CF_TOKEN="$(bws secret list -o json 2>/dev/null | python3 -c '
import json,sys
items=json.load(sys.stdin)
for item in items:
    if item.get("key") == "cloudflare.api-token-day-to-day":
        print(item.get("value",""), end="")
        break
')"
unset BWS_ACCESS_TOKEN
[ -n "$CF_TOKEN" ] || {
  echo "handoff deploy: BWS could not resolve cloudflare.api-token-day-to-day" >&2
  exit 1
}
export CLOUDFLARE_API_TOKEN="$CF_TOKEN"
unset CF_TOKEN

DEPLOY_LOG="$(mktemp)"
trap 'rm -f "$DEPLOY_LOG"' EXIT
wrangler deploy 2>&1 | tee "$DEPLOY_LOG"
ORIGIN="$(grep -Eo 'https://muse-agent-handoff\.[[:alnum:].-]+\.workers\.dev' "$DEPLOY_LOG" | tail -n 1 || true)"
[ -n "$ORIGIN" ] || {
  echo "handoff deploy: Worker deployed but its workers.dev URL was not detected; copy the URL from wrangler output and save it to $STATE/origin" >&2
  exit 1
}

wrangler secret put ADMIN_TOKEN <"$STATE/admin-token"
wrangler secret put HANDOFF_ENCRYPTION_KEY <"$STATE/encryption-key"
printf '%s\n' "$ORIGIN" >"$STATE/origin.tmp"
chmod 600 "$STATE/origin.tmp"
mv -f "$STATE/origin.tmp" "$STATE/origin"
echo "Handoff Worker ready at $ORIGIN"
echo "Generator admin token saved with mode 0600 under $STATE."
echo "Now run: python3 $HERE/create-url.py"
