#!/usr/bin/env bash
# Builds and pushes only the lab-ops-hub crawler container image
# (see image-push.mjs). The separate GHA version workflow releases Worker code.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
LOH_CHECK_ARGS_ONLY=1 node image-push.mjs "$@"
repo_root=$(git rev-parse --show-toplevel)
# shellcheck source=/dev/null
source "${repo_root}/lib/bitwarden-env.sh" >/dev/null
CLOUDFLARE_ACCOUNT_ID=$(sops decrypt --extract '["cloudflare_account_id"]' "${repo_root}/secrets/sops/common/cloudflare.yaml")
export CLOUDFLARE_ACCOUNT_ID
exec node image-push.mjs "$@"
