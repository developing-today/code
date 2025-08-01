#!/usr/bin/env bash

set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail

if [ -n "${SKIP_LOAD:-}" ]; then
  echo "skipping tfstate load"
  exit 0
fi

script_name="$0"
while [[ "$script_name" == -* ]]; do
    script_name="${script_name#-}"
done

dir="$(dirname -- "$(which -- "$script_name" 2>/dev/null || realpath -- "$script_name")")"
echo "dir: $dir"

if [ -f "$dir/.lock" ]; then
  echo "lock file exists, delete it to continue"
  exit 1
fi
touch "$dir/.lock"
function cleanup() {
  echo "cleaning up"
  echo "deleting lock file"
  rm -f "$dir/.lock"
  # echo "saving tfstate"
  # "$dir/save.sh"
  # echo "successfully saved tfstate"
  echo "done cleaning up"
}
trap cleanup EXIT

if [ ! -f "$dir/terraform.tfstate.enc" ] || [ ! -s "$dir/terraform.tfstate.enc" ]; then
  echo "Error: terraform.tfstate.enc does not exist or is empty" >&2
  exit 1
fi
echo "terraform.tfstate.enc exists and is not empty, continuing..."

echo "Decrypting with sops: terraform.tfstate.enc -> terraform.tfstate"
sops -d "$dir/terraform.tfstate.enc" > "$dir/terraform.tfstate"
echo "Decrypted with sops: terraform.tfstate.enc -> terraform.tfstate"
