#!/bin/sh
# Sends one confirmation mail through Cloudflare Email Service.
# Used as `id serve --world-mail-command <this script>`; reads ID_MAIL_* from the environment.
set -eu
: "${CLOUDFLARE_API_TOKEN:?set CLOUDFLARE_API_TOKEN}"
: "${CLOUDFLARE_ACCOUNT_ID:?set CLOUDFLARE_ACCOUNT_ID}"
: "${ID_MAIL_TO:?}" "${ID_MAIL_SUBJECT:?}" "${ID_MAIL_BODY:?}"
from=${ID_MAIL_FROM:-id@security.cab}

payload=$(jq -n --arg from "$from" --arg to "$ID_MAIL_TO" \
  --arg subject "$ID_MAIL_SUBJECT" --arg text "$ID_MAIL_BODY" \
  '{from: $from, to: $to, subject: $subject, text: $text}')

response=$(printf 'header = "Authorization: Bearer %s"\n' "$CLOUDFLARE_API_TOKEN" |
  curl -fsS -K - -H 'Content-Type: application/json' --data-binary "$payload" \
    "https://api.cloudflare.com/client/v4/accounts/$CLOUDFLARE_ACCOUNT_ID/email/sending/send")

printf '%s' "$response" | jq -e '.success and ((.result.permanent_bounces | length) == 0)' >/dev/null
