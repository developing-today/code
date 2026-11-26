#!/usr/bin/env bash
# Deploy the fleet-git-rehost Worker.
# Command line: [--dry-run]. Anything else is a usage error (exit 2)
# before any credential is fetched or wrangler runs (#3352).
#
# The Cloudflare account token is fetched from the vault by name
# (cloudflare.api-token-account-wide) and piped to wrangler on stdin
# via pkgs/cf/wrangler-authed.py; it never lands in an argument, a
# file, or this script's output.
if [[ $# -gt 1 || ($# -eq 1 && $1 != "--dry-run") ]]; then
  printf 'Unknown argument: %q\n' "${1:-}" >&2
  printf 'Usage: %s [--dry-run]\n' "$0" >&2
  exit 2
fi
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
repo_root=$(git rev-parse --show-toplevel)
args=(deploy)
if [[ ${1:-} == "--dry-run" ]]; then
  args+=(--dry-run)
fi
~/tools/fleet secrets get cloudflare.api-token-account-wide |
  python3 "$repo_root/pkgs/cf/wrangler-authed.py" "${args[@]}"
