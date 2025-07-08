#!/usr/bin/env bash
set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail

if [ -n "${SKIP_SAVE:-}" ]; then
  echo "skipping tfstate save"
  exit 0
fi

echo "\$0=$0"
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

if [ ! -f "$dir/terraform.tfstate" ] || [ ! -s "$dir/terraform.tfstate" ]; then
  echo "Error: terraform.tfstate does not exist or is empty" >&2
  exit 1
fi
echo "terraform.tfstate exists and is not empty, continuing... (dir=$dir)"

echo "Encrypting with sops: terraform.tfstate -> terraform.tfstate.enc (dir=$dir)"
sops -e "$dir/terraform.tfstate" > "$dir/terraform.tfstate.enc"
echo "Encrypted with sops: terraform.tfstate -> terraform.tfstate.enc (dir=$dir)"
