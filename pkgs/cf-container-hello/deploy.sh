#!/usr/bin/env bash
# Deploys the cf-container-hello Worker (see deploy.mjs). Pass
# --dry-run to validate without deploying. The lifecycle verbs
# (deploy / status / activate / version-bindings) used by the generic
# deploy CLI pass through unchanged.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
repo_root=$(git rev-parse --show-toplevel)
# shellcheck source=/dev/null
source "${repo_root}/lib/bitwarden-env.sh" >/dev/null
CLOUDFLARE_ACCOUNT_ID=$(sops decrypt --extract '["cloudflare_account_id"]' "${repo_root}/secrets/sops/common/cloudflare.yaml")
export CLOUDFLARE_ACCOUNT_ID
exec node deploy.mjs "$@"
