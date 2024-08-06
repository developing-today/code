#!/usr/bin/env bash

set -exuo pipefail

RM_ALL_TFSTATE_FILES=false
if [ "$1" == "all" ] || [ "$1" == "--all" ]; then
  RM_ALL_TFSTATE_FILES=true
# elseif it is not empty throw
elif [ ! -z "$1" ]; then
  echo "Invalid argument: $1"
  exit 1
else
  echo "No argument provided, only removing terraform.tfstate backup files"
fi

dir="$(dirname -- "$(readlink -f -- "$0")")"
echo "dir: $dir"

ls "$dir/terraform.tfstate.backup"
echo "Removing terraform.tfstate.backup"
rm terraform.tfstate.backup

ls "$dir/terraform.tfstate.*.backup"
echo "Removing all terraform.tfstate.*.backup files"
rm terraform.tfstate.*.backup

ls "$dir/terraform.tfstate"
if [ "$RM_ALL_TFSTATE_FILES" = true ]; then
  echo "Removing terraform.tfstate"
  rm terraform.tfstate
else
  echo "Not removing terraform.tfstate, use 'all' or '--all' as argument to remove it"
fi
