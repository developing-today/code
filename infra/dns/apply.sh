#!/usr/bin/env bash

set -exuo pipefail

dir="$(dirname -- "$(which -- "$0" 2>/dev/null || realpath -- "$0")")"
echo "dir: $dir"

outPlan="${1:-"$dir/terraform.tfplan"}"
echo "tfplan: $outPlan"

echo "generating tfplan"
$dir/plan.sh
echo "successfully generated tfplan"

if [ -f "$dir/.lock" ]; then
  echo "lock file exists, delete it to continue"
  exit 1
fi
trap 'rm -f "$dir/.lock"' EXIT

echo "applying tfplan"
tofu -chdir="$dir" apply -auto-approve -input=false "$outPlan"
echo "successfully applied tfplan"

echo "saving tfstate"
$dir/save.sh
echo "successfully saved tfstate"
