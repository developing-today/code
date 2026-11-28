#!/usr/bin/env bash
set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail
TF_PARALLELISM="${TF_PARALLELISM:-1}"
echo "TF_PARALLELISM: $TF_PARALLELISM"
echo "\$0=$0"
script_name="$0"
while [[ $script_name == -* ]]; do
  script_name="${script_name#-}"
done
dir="$(dirname -- "$(which -- "$script_name" 2>/dev/null || realpath -- "$script_name")")"
echo "dir: $dir"

tfPlan=""
REFRESH_STATE="false"
while [[ $# -gt 0 ]]; do
  case $1 in
  -*)
    if [[ $1 == "--refresh" ]]; then
      REFRESH_STATE="true"
    fi
    ;;
  *)
    if [[ -z $tfPlan ]]; then
      tfPlan="$1"
    elif [[ $1 == "refresh" ]]; then
      REFRESH_STATE="true"
    fi
    ;;
  esac
  shift
done

tfPlan="${tfPlan:-"$dir/terraform.tfplan"}"
echo "tfplan: $tfPlan"
echo "REFRESH_STATE: $REFRESH_STATE"

if [[ $REFRESH_STATE == "false" ]]; then
  echo "No refresh argument provided, only removing terraform.tfstate backup files"
fi

if [ -n "${SKIP_PLAN:-}" ]; then
  echo "skipping tf plan"
  exit 0
fi

if [ -n "${SKIP_GENERATE:-}" ]; then
  echo "skipping generate dns config"
else
  echo "generating dns config"
  "$dir/generate_dns_config.sh"
  echo "successfully generated dns config"
fi

echo "removing old plan: $tfPlan"
rm -f "$tfPlan"
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

# Refuse to plan against a void or mismatched state (code#1943): the state
# at the backend path must be the state load.sh decrypted from
# terraform.tfstate.enc. init.sh restores + verifies it; check once more
# here so a SKIP_INIT run cannot plan against an empty state either.
if [ ! -s "$dir/terraform.tfstate" ]; then
  echo "Error: terraform.tfstate is missing or empty; refusing to plan against a void state" >&2
  export SKIP_SAVE=1 # do not let the EXIT trap save a state we just refused
  exit 1
fi
enc_lineage="$(sops -d "$dir/terraform.tfstate.enc" | grep -o '"lineage": *"[^"]*"' | head -1)"
state_lineage="$(grep -o '"lineage": *"[^"]*"' "$dir/terraform.tfstate" | head -1)"
if [ -z "$state_lineage" ] || [ "$state_lineage" != "$enc_lineage" ]; then
  echo "Error: terraform.tfstate lineage ($state_lineage) does not match terraform.tfstate.enc ($enc_lineage); refusing to plan against a mismatched state" >&2
  export SKIP_SAVE=1 # do not let the EXIT trap save a state we just refused
  exit 1
fi
echo "tfstate lineage verified against terraform.tfstate.enc: $state_lineage"

echo "running terraform plan"
if [ "$REFRESH_STATE" == "true" ]; then
  echo "also refreshing state"
  tofu -chdir="$dir" plan -parallelism=$TF_PARALLELISM -out="$tfPlan"
  echo "successfully planned & refreshed state"
else
  echo "plan only, not refreshing state"
  tofu -chdir="$dir" plan -parallelism=$TF_PARALLELISM -refresh=false -out="$tfPlan"
  echo "successfully planned"
fi
echo "successfully ran terraform plan"

rm -f "$dir/.lock"
