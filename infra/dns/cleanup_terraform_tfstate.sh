#!/usr/bin/env bash

set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail

RM_ALL_TFSTATE_FILES=false
if [ "${1:-}" == "all" ] || [ "${1:-}" == "--all" ]; then
  RM_ALL_TFSTATE_FILES=true
elif [ "${1:-}" != "" ]; then
  echo "Invalid argument: $1"
  exit 1
else
  echo "No argument provided, only removing terraform.tfstate backup files"
fi

echo "\$0=$0"
script_name="$0"
while [[ "$script_name" == -* ]]; do
    script_name="${script_name#-}"
done

dir="$(dirname -- "$(readlink -f -- "$script_name")")"
echo "dir: $dir"

if [ -f "$dir/terraform.tfstate.backup" ]; then
  ls "$dir/terraform.tfstate.backup"
  echo "Removing terraform.tfstate.backup"
  rm "$dir/terraform.tfstate.backup"
else
  echo "No terraform.tfstate.backup file found"
fi

if [ "$(ls -1 "$dir/terraform.tfstate.*.backup" 2>/dev/null | wc -l)" -eq 0 ]; then
  echo "No terraform.tfstate.*.backup files found"
else
  ls "$dir/terraform.tfstate.*.backup"
  echo "Removing all terraform.tfstate.*.backup files"
  rm "$dir/terraform.tfstate.*.backup"
fi

if [ -f "$dir/terraform.tfstate" ]; then
  ls "$dir/terraform.tfstate"
  if [ "$RM_ALL_TFSTATE_FILES" = true ]; then
    echo "Removing terraform.tfstate"
    rm "$dir/terraform.tfstate"
  else
    echo "Not removing terraform.tfstate, use 'all' or '--all' as argument to remove it"
  fi
else
  echo "No terraform.tfstate file found"
fi
