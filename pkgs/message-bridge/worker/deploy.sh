#!/usr/bin/env bash
# Deploys the message-bridge Worker (see deploy.mjs). Pass --dry-run to
# validate without deploying, or --print-urls to print the connector and
# page URLs (they contain the access token) without deploying.
set -euo pipefail
# Check arguments before Bitwarden or sops are touched, so a typo never
# triggers a credential prompt or a decrypt.
if [ "$#" -gt 1 ]; then
  echo "usage: deploy.sh [--dry-run | --print-urls]" >&2
  exit 2
fi
for arg in "$@"; do
  case "$arg" in
  --dry-run | --print-urls) ;;
  *)
    echo "unrecognized argument: $arg" >&2
    echo "usage: deploy.sh [--dry-run | --print-urls]" >&2
    exit 2
    ;;
  esac
done
cd "$(dirname "${BASH_SOURCE[0]}")"
repo_root=$(git rev-parse --show-toplevel)
if [ -z "${BWS_ACCESS_TOKEN:-}" ]; then
  # shellcheck source=/dev/null
  source "${repo_root}/lib/bitwarden-env.sh" >/dev/null
fi
if [ "${1:-}" != "--print-urls" ]; then
  CLOUDFLARE_ACCOUNT_ID=$(sops decrypt --extract '["cloudflare_account_id"]' "${repo_root}/secrets/sops/common/cloudflare.yaml")
  export CLOUDFLARE_ACCOUNT_ID
fi
exec node deploy.mjs "$@"
