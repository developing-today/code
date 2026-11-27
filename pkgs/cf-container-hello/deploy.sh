#!/usr/bin/env bash
# Deploys the cf-container-hello Worker (see deploy.mjs). Verbs:
#   deploy.sh [--dry-run] [--json]             deploy (validate only with --dry-run)
#   deploy.sh status [--json]                  what Cloudflare is serving now
#   deploy.sh verify-bindings --version ID [--json]
#   deploy.sh rollback --version ID [--json]    re-activate a recorded version
# The deploy CLI's Cloudflare provider boundary drives the status /
# verify-bindings / rollback verbs (issue #5767, slice 4).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
repo_root=$(git rev-parse --show-toplevel)
# shellcheck source=/dev/null
source "${repo_root}/lib/bitwarden-env.sh" >/dev/null
CLOUDFLARE_ACCOUNT_ID=$(sops decrypt --extract '["cloudflare_account_id"]' "${repo_root}/secrets/sops/common/cloudflare.yaml")
export CLOUDFLARE_ACCOUNT_ID
exec node deploy.mjs "$@"
