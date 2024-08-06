#!/usr/bin/env bash
set -exuo pipefail
dir="$(dirname -- "$(readlink -f -- "$0")")"
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

echo "Listing tainted resources:"
run_id=$(date +%s)
tainted_resources=$(tofu -chdir="$dir" show -json | jq -r '.values.root_module.resources[] | select(.tainted == true) | .address')
echo "tainted_resources:\n$tainted_resources"
if [ -z "$tainted_resources" ]; then
  echo "no tainted resources found"
  if [ -f "$dir/tainted_resources.json" ]; then
    echo "backing up tainted resources to $dir/tainted_resources.$run_id.json"
    cp "$dir/tainted_resources.json" "$dir/tainted_resources.$run_id.json"
    rm -f "$dir/tainted_resources.json"
  fi
else
  echo "$tainted_resources" | jq -nR '[inputs]' > "$dir/tainted_resources.json"
  echo "tainted resources written to $dir/tainted_resources.json"
  echo "tainted_resources.json:"
  cat "$dir/tainted_resources.json"
  log_dir="$dir/logs/tainted_resources/$run_id"
  mkdir -p "$log_dir"
  echo "backing up tainted resources to $log_dir/$run_id.tainted_resources.json"
  cp $dir/tainted_resources.json $log_dir/$run_id.tainted_resources.json
  echo "Listing complete."
fi
