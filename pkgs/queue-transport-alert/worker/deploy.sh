#!/usr/bin/env bash
# Deploys the queue-transport-alert Worker through the cf versions flow
# (see deploy.mjs). Pass --dry-run to build without uploading, or
# --deploy-version <id> to deploy (roll back to) an existing version.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
# Check arguments before Bitwarden or sops are touched, so a typo never
# triggers a credential prompt, a decrypt or a deploy of HEAD.
node deploy-lib.mjs "$@"
if [ "${QTA_GHA:-}" = 1 ]; then
  exec node deploy.mjs "$@"
fi
repo_root=$(git rev-parse --show-toplevel)
if [ -z "${BWS_ACCESS_TOKEN:-}" ]; then
  # shellcheck source=/dev/null
  source "${repo_root}/lib/bitwarden-env.sh" >/dev/null
fi
CLOUDFLARE_ACCOUNT_ID=$(sops decrypt --extract '["cloudflare_account_id"]' "${repo_root}/secrets/sops/common/cloudflare.yaml")
export CLOUDFLARE_ACCOUNT_ID
exec node deploy.mjs "$@"
