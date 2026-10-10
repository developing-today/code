#!/usr/bin/env bash
# Deploys the id-mail Worker. Code and routes come from this directory; the CALLERS
# secret is built from callers.json and the Bitwarden tokens it names. Safe to re-run.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
repo_root=$(git rev-parse --show-toplevel)
# shellcheck source=/dev/null
source "${repo_root}/lib/bitwarden-env.sh" >/dev/null

secrets=$(bws secret list -o json)
secret_value() {
  jq -r --arg key "${1}" '.[] | select(.key == $key) | .value' <<<"${secrets}"
}

CLOUDFLARE_API_TOKEN=$(secret_value cloudflare.api-token-account-wide)
export CLOUDFLARE_API_TOKEN
CLOUDFLARE_ACCOUNT_ID=$(sops decrypt --extract '["cloudflare_account_id"]' "${repo_root}/secrets/sops/common/cloudflare.yaml")
export CLOUDFLARE_ACCOUNT_ID

callers='{}'
for name in $(jq -r 'keys[]' callers.json); do
  entry=$(jq -c --arg name "${name}" '.[$name]' callers.json)
  token_key=$(jq -r '.tokenKey' <<<"${entry}")
  token=$(secret_value "${token_key}")
  if [[ -z "${token}" ]]; then
    echo "deploy: no Bitwarden secret named ${token_key} (caller ${name})" >&2
    exit 1
  fi
  froms=$(jq -r '.transports[]?.from[]?' <<<"${entry}")
  while read -r from; do
    if ! grep -qF "\"${from}\"" wrangler.toml; then
      echo "deploy: caller ${name} sends from ${from}, which wrangler.toml does not allow" >&2
      exit 1
    fi
  done <<<"${froms}"
  sha256=$(printf '%s' "${token}" | sha256sum | cut -d' ' -f1)
  callers=$(jq -c --arg name "${name}" --arg sha256 "${sha256}" --argjson entry "${entry}" \
    '. + {($name): {sha256: $sha256, transports: $entry.transports}}' <<<"${callers}")
done

printf '%s' "${callers}" | wrangler secret put CALLERS
wrangler deploy
