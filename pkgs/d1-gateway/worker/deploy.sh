#!/usr/bin/env bash
# d1-gateway deploy (Cloudflare versions flow).
#   deploy.sh                       upload a version of this commit, deploy it at 100%
#   deploy.sh --deploy-version ID   deploy an existing version (rollback)
# Needs CLOUDFLARE_API_TOKEN and CLOUDFLARE_ACCOUNT_ID in the env.
# The very first deploy creates the Worker with `wrangler deploy`
# (versions upload can't create one); every later deploy is a version.
set -euo pipefail
: "${CLOUDFLARE_API_TOKEN:?CLOUDFLARE_API_TOKEN is required}"
: "${CLOUDFLARE_ACCOUNT_ID:?CLOUDFLARE_ACCOUNT_ID is required}"
pkg="${D1_GATEWAY_PKG:?}"
worker=d1-gateway
health=https://d1-gateway.developing-today.workers.dev/health
uuid_re='^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'

rollback=""
if [[ ${1-} == --deploy-version ]]; then
  rollback="${2-}"
  [[ $rollback =~ $uuid_re ]] || {
    echo "--deploy-version needs a version id" >&2
    exit 2
  }
elif [[ $# -ne 0 ]]; then
  echo "usage: $0 [--deploy-version <id>]" >&2
  exit 2
fi

sha="${GITHUB_SHA:-$(git rev-parse HEAD 2>/dev/null || echo unknown)}"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
cp -r "$pkg"/. "$stage"/
chmod -R u+w "$stage"
cd "$stage"

if [[ -n $rollback ]]; then
  wrangler versions deploy "$rollback@100%" --yes --message "rollback to $rollback"
  vid="$rollback"
else
  code=$(curl -s -o /dev/null -w '%{http_code}' \
    -H "Authorization: Bearer $CLOUDFLARE_API_TOKEN" \
    "https://api.cloudflare.com/client/v4/accounts/$CLOUDFLARE_ACCOUNT_ID/workers/scripts/$worker/settings")
  if [[ $code == 404 ]]; then
    echo "d1-gateway does not exist yet; creating it with wrangler deploy" >&2
    out=$(wrangler deploy --var "VERSION:$sha" 2>&1 | tee /dev/stderr)
    vid=$(grep -oE 'Current Version ID: [0-9a-f-]{36}' <<<"$out" | awk '{print $4}' | tail -1)
  else
    out=$(wrangler versions upload --tag "git-${sha:0:12}" --message "$sha" --var "VERSION:$sha" 2>&1 | tee /dev/stderr)
    vid=$(grep -oE 'Worker Version ID: [0-9a-f-]{36}' <<<"$out" | awk '{print $4}' | tail -1)
    [[ $vid =~ $uuid_re ]] || {
      echo "could not read the uploaded version id" >&2
      exit 1
    }
    wrangler versions deploy "$vid@100%" --yes --message "$sha"
  fi
fi

for _ in $(seq 1 36); do
  got=$(curl -fsS "$health" 2>/dev/null | jq -r '.version // empty' || true)
  if [[ -n ${vid-} && $got == "$vid" ]]; then
    echo "d1-gateway serves version $vid"
    exit 0
  fi
  if [[ -z ${vid-} && -n $got ]]; then
    echo "d1-gateway serves version $got"
    exit 0
  fi
  sleep 5
done
echo "d1-gateway /health never reported version ${vid-?}" >&2
exit 1
