#!/usr/bin/env bash

set -exuo pipefail

dir="$(dirname -- "$(readlink -f -- "$0")")"
echo "dir: $dir"

echo "loading tfstate"
$dir/load.sh
echo "successfully loaded tfstate"

if [ -f "$dir/.lock" ]; then
  echo "lock file exists, delete it to continue"
  exit 1
fi
trap 'rm -f "$dir/.lock"' EXIT

echo "initializing tofu..."
tofu -chdir="$dir" init -backend-config="path=$dir/terraform.tfstate" "$@"
echo "tofu initialized."
