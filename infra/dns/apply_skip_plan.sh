#!/usr/bin/env bash

set -exuo pipefail

# if [ -n "${SKIP_APPLY:-}" ]; then
#   echo "skipping tf apply"
#   exit 0
# fi

dir="$(dirname -- "$(which -- "$0" 2>/dev/null || realpath -- "$0")")"
echo "dir: $dir"

SKIP_PLAN=1 "$dir/apply.sh"
