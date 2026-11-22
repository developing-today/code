#!/usr/bin/env bash
# Uploads an immutable cold-store-rehydrator version with `cf`, without
# changing production traffic. Use --deploy-version <id> plus the exact
# --expected-baseline fingerprint only after private staging evidence.
# GitHub Actions supplies the dedicated
# Worker token and account id only through the protected environment.
set -euo pipefail
script_dir=${BASH_SOURCE[0]%/*}
if [ "$script_dir" = "${BASH_SOURCE[0]}" ]; then script_dir=.; fi
cd "$script_dir"
# Strictly validate the whole command line before credentials are loaded.
CSR_CHECK_ARGS_ONLY=1 node deploy.mjs "$@"
repo_root=$(git rev-parse --show-toplevel)
if [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
  : "${COLD_STORE_REHYDRATOR_CF_API_TOKEN:?dedicated environment secret is required in Actions}"
  export CLOUDFLARE_ACCOUNT_ID="${COLD_STORE_REHYDRATOR_CF_ACCOUNT_ID:?environment variable is required in Actions}"
else
  # Preserve the existing local operator flow. The Cloudflare credential is
  # resolved by deploy.mjs from CLOUDFLARE_API_TOKEN or Bitwarden; the
  # runtime refresh/API secrets are never read or copied here.
  # shellcheck source=/dev/null
  source "${repo_root}/lib/bitwarden-env.sh" >/dev/null
  export CLOUDFLARE_ACCOUNT_ID
  CLOUDFLARE_ACCOUNT_ID=$(sops decrypt --extract '["cloudflare_account_id"]' "${repo_root}/secrets/sops/common/cloudflare.yaml")
fi
exec node deploy.mjs "$@"
