#!/usr/bin/env bash
# Deploys the lab-ops-hub Worker + its crawler container image as one
# command (see deploy.mjs). Pass --dry-run to validate the Worker
# config without pushing the image or deploying.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
# Reject malformed arguments before sourcing Bitwarden/SOPS credentials.
LOH_CHECK_ARGS_ONLY=1 node deploy.mjs "$@"
repo_root=$(git rev-parse --show-toplevel)
# shellcheck source=/dev/null
source "${repo_root}/lib/bitwarden-env.sh" >/dev/null
CLOUDFLARE_ACCOUNT_ID=$(sops decrypt --extract '["cloudflare_account_id"]' "${repo_root}/secrets/sops/common/cloudflare.yaml")
export CLOUDFLARE_ACCOUNT_ID
exec node deploy.mjs "$@"
