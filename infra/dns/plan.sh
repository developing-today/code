#!/usr/bin/env bash

set -exuo pipefail

if [ -n "${SKIP_PLAN:-}" ]; then
  echo "skipping tf plan"
  exit 0
fi

dir="$(dirname -- "$(which -- "$0" 2>/dev/null || realpath -- "$0")")"
echo "dir: $dir"

outPlan="${1:-"$dir/terraform.tfplan"}"
echo "tfplan: $outPlan"

echo "generating dns config"
$dir/generate_dns_config.sh
echo "successfully generated dns config"

echo "removing old plan"
rm -f "$outPlan"
echo "successfully removed old plan"

echo "running terraform init"
$dir/init.sh
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
  $dir/save.sh
  echo "successfully saved tfstate"
  echo "done cleaning up"
}
trap cleanup EXIT

echo "running terraform plan"
tofu -chdir="$dir" plan -out="$outPlan"
echo "successfully ran terraform plan"

rm -f "$dir/.lock"
