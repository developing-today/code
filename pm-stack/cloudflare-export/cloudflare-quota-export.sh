#!/usr/bin/env bash
# cloudflare-quota-export.sh — runtime wrapper for the Cloudflare
# exporter (developing-today/code#1048). Fetches the day-to-day API
# token from Bitwarden Secrets Manager BY NAME
# (cloudflare.api-token-day-to-day) into this process's environment
# only, then execs the exporter under Bun. The token value never
# lands in a file, a log, or the export document.
#
# The bws machine-account access token is read from the same
# 600-perm files the fleet's other bws loaders use
# (~/.config/bws/access-token on the laptop, ~/.config/bitwarden/
# access-token on the sandbox). No value is ever printed.
#
# Credential-loading failures do NOT exit silently: they route
# through the exporter with no Cloudflare token, so the exporter
# writes its no-token failure document (status + attemptedAt, no
# metrics) atomically over latest.json — a failed tick is never
# mistaken for the previous successful reading.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

fail_through_exporter() {
  echo "cloudflare-quota-export: $1" >&2
  shift
  exec env -u CLOUDFLARE_API_TOKEN bun "$SCRIPT_DIR/cloudflare-quota-export.mjs" "$@"
}

for f in "$HOME/.config/bws/access-token" "$HOME/.config/bitwarden/access-token"; do
  if [[ -f $f ]]; then
    export BWS_ACCESS_TOKEN="$(cat "$f")"
    break
  fi
done
if [[ -z ${BWS_ACCESS_TOKEN:-} ]]; then
  fail_through_exporter "no bws access token file found" "$@"
fi

SECRET_ID="$(bws secret list 2>/dev/null | jq -r '.[] | select(.key=="cloudflare.api-token-day-to-day") | .id' | head -n 1)" || SECRET_ID=""
if [[ -z $SECRET_ID ]]; then
  fail_through_exporter "Bitwarden secret cloudflare.api-token-day-to-day not found" "$@"
fi
CLOUDFLARE_API_TOKEN="$(bws secret get "$SECRET_ID" 2>/dev/null | jq -er .value)" || CLOUDFLARE_API_TOKEN=""
export CLOUDFLARE_API_TOKEN
if [[ -z $CLOUDFLARE_API_TOKEN ]]; then
  fail_through_exporter "could not read the day-to-day token value" "$@"
fi

exec bun "$SCRIPT_DIR/cloudflare-quota-export.mjs" "$@"
