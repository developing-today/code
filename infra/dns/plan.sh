#!/usr/bin/env bash

set -exuo pipefail

REFRESH_STATE="${REFRESH_STATE:-false}"
if [ "${1:-}" == "refresh" ] || [ "${1:-}" == "--refresh" ]; then
  REFRESH_STATE="true"
elif [ "${1:-}" != "" ]; then
  echo "Invalid argument: $1"
  exit 1
else
  echo "No argument provided, only removing terraform.tfstate backup files"
fi

if [ -n "${SKIP_PLAN:-}" ]; then
  echo "skipping tf plan"
  exit 0
fi

dir="$(dirname -- "$(which -- "$0" 2>/dev/null || realpath -- "$0")")"
echo "dir: $dir"

outPlan="${1:-"$dir/terraform.tfplan"}"
echo "tfplan: $outPlan"

echo "generating dns config"
"$dir/generate_dns_config.sh"
echo "successfully generated dns config"

echo "removing old plan"
rm -f "$outPlan"
echo "successfully removed old plan"

echo "running terraform init"
"$dir/init.sh"
echo "successfully ran terraform init"

if [ -f "$dir/.lock" ]; then
  echo "lock file exists, delete it to continue"
  exit 1
fi
touch "$dir/.lock"
function cleanup() {
  echo "cleaning up"
  echo "deleting lock file"
  rm -f "$dir/.lock"
  echo "saving tfstate"
  "$dir/save.sh"
  echo "successfully saved tfstate"
  echo "done cleaning up"
}
trap cleanup EXIT

echo "running terraform plan"
if [ "$REFRESH_STATE" == "true" ]; then
  echo "also refreshing state"
  tofu -chdir="$dir" plan -parallelism=1 -out="$outPlan"
  echo "successfully planned & refreshed state"
else
  echo "plan only, not refreshing state"
  tofu -chdir="$dir" plan -parallelism=1 -refresh=false -out="$outPlan"
  echo "successfully planned"
fi
echo "successfully ran terraform plan"

rm -f "$dir/.lock"
