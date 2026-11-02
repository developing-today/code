#!/usr/bin/env bash
# Deploys the pkarr Worker via the cf versions flow. Pass --dry-run to
# validate without uploading, or --deploy-version <id> to create a
# deployment at an existing version (the rollback path).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
repo_root=$(git rev-parse --show-toplevel)
# shellcheck source=/dev/null
source "${repo_root}/lib/bitwarden-env.sh" >/dev/null
CLOUDFLARE_ACCOUNT_ID=$(sops decrypt --extract '["cloudflare_account_id"]' "${repo_root}/secrets/sops/common/cloudflare.yaml")
export CLOUDFLARE_ACCOUNT_ID
exec node deploy.mjs "$@"
