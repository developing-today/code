#!/usr/bin/env bash

set -exuo pipefail

dir="$(dirname -- "$(which -- "$0" 2>/dev/null || realpath -- "$0")")"
echo "dir: $dir"

if [ -f "$dir/.lock" ]; then
  echo "lock file exists, delete it to continue"
  exit 1
fi
trap 'rm -f "$dir/.lock"' EXIT

if [ ! -f "$dir/terraform.tfstate.enc" ] || [ ! -s "$dir/terraform.tfstate.enc" ]; then
  echo "Error: terraform.tfstate.enc does not exist or is empty" >&2
  exit 1
fi
echo "terraform.tfstate.enc exists and is not empty, continuing..."

echo "Decrypting with sops: terraform.tfstate.enc -> terraform.tfstate"
sops -d "$dir/terraform.tfstate.enc" > "$dir/terraform.tfstate"
echo "Decrypted with sops: terraform.tfstate.enc -> terraform.tfstate"
