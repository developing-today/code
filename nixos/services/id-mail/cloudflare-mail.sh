# Sends one confirmation mail. Run by `id serve --world-mail-command`, which sets ID_MAIL_TO, ID_MAIL_SUBJECT and ID_MAIL_BODY.
# ID_MAIL_TRANSPORT picks how it leaves: rest (Cloudflare Email Sending API) or worker (the mail Worker's send_email binding).
: "${ID_MAIL_TO:?}" "${ID_MAIL_SUBJECT:?}" "${ID_MAIL_BODY:?}"
transport=${ID_MAIL_TRANSPORT:-rest}

case $transport in
rest)
  : "${CLOUDFLARE_API_TOKEN:?set CLOUDFLARE_API_TOKEN}"
  : "${CLOUDFLARE_ACCOUNT_ID:?set CLOUDFLARE_ACCOUNT_ID}"
  from=${ID_MAIL_FROM:-id@security.cab}
  payload=$(jq -n --arg from "$from" --arg to "$ID_MAIL_TO" \
    --arg subject "$ID_MAIL_SUBJECT" --arg text "$ID_MAIL_BODY" \
    '{from: $from, to: $to, subject: $subject, text: $text}')
  response=$(printf 'header = "Authorization: Bearer %s"\n' "$CLOUDFLARE_API_TOKEN" |
    curl -fsS -K - -H 'Content-Type: application/json' --data-binary "$payload" \
      "https://api.cloudflare.com/client/v4/accounts/$CLOUDFLARE_ACCOUNT_ID/email/sending/send")
  printf '%s' "$response" | jq -e '.success and ((.result.permanent_bounces | length) == 0)' >/dev/null
  ;;
worker)
  : "${ID_MAIL_WORKER_URL:?set ID_MAIL_WORKER_URL}"
  : "${ID_MAIL_WORKER_TOKEN:?set ID_MAIL_WORKER_TOKEN}"
  payload=$(jq -n --arg to "$ID_MAIL_TO" --arg subject "$ID_MAIL_SUBJECT" --arg text "$ID_MAIL_BODY" \
    '{to: $to, subject: $subject, text: $text}')
  printf 'header = "Authorization: Bearer %s"\n' "$ID_MAIL_WORKER_TOKEN" |
    curl -fsS -K - -H 'Content-Type: application/json' --data-binary "$payload" \
      "$ID_MAIL_WORKER_URL" >/dev/null
  ;;
*)
  echo "unknown ID_MAIL_TRANSPORT: $transport" >&2
  exit 2
  ;;
esac
