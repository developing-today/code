#!/usr/bin/env bash
# Deploys the message-bridge Worker through the cf versions flow (see
# deploy.mjs for the modes):
#   deploy.sh                              full deploy (sets ACCESS_TOKEN)
#   deploy.sh --dry-run                    build and check, upload nothing
#   deploy.sh --code-only                  CI: Cloudflare creds from env
#   deploy.sh --deploy-version <id> [--dry-run]   roll back / redeploy
#   deploy.sh --print-urls                 connector and page URLs
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
# Check the arguments before Bitwarden or sops are touched, so a typo
# never triggers a credential prompt or a decrypt.
MB_CHECK_ARGS_ONLY=1 node deploy.mjs "$@"

# CI hands over both Cloudflare values; --code-only and --deploy-version
# then need nothing else. Everything else reads Bitwarden and sops.
mode="${1:-}"
if [ -n "${CLOUDFLARE_API_TOKEN:-}" ] && [ -n "${CLOUDFLARE_ACCOUNT_ID:-}" ] &&
  { [ "$mode" = "--code-only" ] || [ "$mode" = "--deploy-version" ]; }; then
  exec node deploy.mjs "$@"
fi
if [ "$mode" = "--code-only" ]; then
  exec node deploy.mjs "$@" # fails with the missing variable named
fi
repo_root=$(git rev-parse --show-toplevel)
if [ -z "${BWS_ACCESS_TOKEN:-}" ]; then
  # shellcheck source=/dev/null
  source "${repo_root}/lib/bitwarden-env.sh" >/dev/null
fi
if [ "$mode" != "--print-urls" ] && [ -z "${CLOUDFLARE_ACCOUNT_ID:-}" ]; then
  CLOUDFLARE_ACCOUNT_ID=$(sops decrypt --extract '["cloudflare_account_id"]' "${repo_root}/secrets/sops/common/cloudflare.yaml")
  export CLOUDFLARE_ACCOUNT_ID
fi
exec node deploy.mjs "$@"
