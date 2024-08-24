#!/usr/bin/env bash

set -exuo pipefail

TF_PARALLELISM="${TF_PARALLELISM:-1}"
echo "TF_PARALLELISM: $TF_PARALLELISM"

if [ -n "${SKIP_APPLY:-}" ]; then
  echo "skipping tf apply"
  exit 0
fi

echo "\$0=$0"
script_name="$0"
while [[ "$script_name" == -* ]]; do
    script_name="${script_name#-}"
done
dir="$(dirname -- "$(which -- "$script_name" 2>/dev/null || realpath -- "$script_name")")"
echo "dir: $dir"

outPlan=""
while [[ $# -gt 0 ]]; do
    case $1 in
        -*)
            shift
            ;;
        *)
            outPlan="$1"
            break
            ;;
    esac
    shift
done

outPlan="${outPlan:-"$dir/terraform.tfplan"}"
echo "tfplan: $outPlan"

echo "generating tfplan"
"$dir/plan.sh" "$outPlan"
echo "successfully generated tfplan"

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

echo "applying tfplan"
tofu -chdir="$dir" apply -auto-approve -input=false -parallelism=$TF_PARALLELISM "$outPlan"
echo "successfully applied tfplan"
