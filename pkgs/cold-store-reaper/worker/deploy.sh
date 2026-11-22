#!/usr/bin/env bash
# Uploads a staged cold-store-rehydrator version with `cf`, without
# changing production traffic. Use --deploy-version <id> only after
# staged parity is verified. GitHub Actions supplies its protected
# environment credentials directly; no Bitwarden/SOPS reads occur here.
set -euo pipefail
script_dir=${BASH_SOURCE[0]%/*}
if [ "$script_dir" = "${BASH_SOURCE[0]}" ]; then script_dir=.; fi
cd "$script_dir"
# Strictly validate the whole command line before credentials are loaded.
CSR_CHECK_ARGS_ONLY=1 node deploy.mjs "$@"
repo_root=$(git rev-parse --show-toplevel)
exec node deploy.mjs "$@"
