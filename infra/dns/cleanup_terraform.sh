#!/usr/bin/env bash

set -exuo pipefail

RM_ALL_TFSTATE_FILES=false
if [ "${1:-}" == "all" ] || [ "${1:-}" == "--all" ]; then
  RM_ALL_TFSTATE_FILES=true
elif [ "${1:-}" != "" ]; then
  echo "Invalid argument: $1"
  exit 1
fi

dir="$(dirname -- "$(which -- "$0" 2>/dev/null || realpath -- "$0")")"
echo "dir: $dir"

"$dir/cleanup_terraform_tfplan.sh"

if [ "$RM_ALL_TFSTATE_FILES" == "true" ]; then
  "$dir/cleanup_terraform_tfstate.sh" --all
else
  "$dir/cleanup_terraform_tfstate.sh"
fi
