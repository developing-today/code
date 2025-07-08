#!/usr/bin/env bash

set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail

if [ -n "${SKIP_INIT:-}" ]; then
  echo "skipping tf init"
  exit 0
fi

echo "\$0=$0"
script_name="$0"
while [[ "$script_name" == -* ]]; do
    script_name="${script_name#-}"
done

dir="$(dirname -- "$(readlink -f -- "$script_name")")"
echo "dir: $dir"

echo "loading tfstate"
"$dir/load.sh"
echo "successfully loaded tfstate"

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


echo "initializing tofu..."
tofu -chdir="$dir" init -upgrade -backend-config="path=$dir/terraform.tfstate" "$@"
echo "tofu initialized."
