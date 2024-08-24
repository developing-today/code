#!/usr/bin/env bash
set -exuo pipefail
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

echo "Listing tainted resources:"

if [ ! -f "$dir/tainted_resources.json" ]; then
  $dir/list_tainted_resources.sh
fi
if [ ! -f "$dir/tainted_resources.json" ]; then
  echo "No tainted resources found."
  exit 0
fi
tainted_resources=$(jq -r '.[]' "$dir/tainted_resources.json")

if [ -z "$tainted_resources" ]; then
  echo "No tainted resources found."
  exit 0
fi

echo "$tainted_resources"

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

echo "$tainted_resources" | while read -r resource; do
  echo "Removing $resource from state..."
  tofu -chdir="$dir" state rm "$resource"
done
run_id=$(date +%s)
log_dir="$dir/logs/tainted_resources/$run_id"
mkdir -p "$log_dir"
echo "backing up tainted resources to $log_dir/$run_id.deleted_tainted_resources.json"
cp $dir/tainted_resources.json $log_dir/$run_id.deleted_tainted_resources.json
echo "Listing complete."

echo "removing tainted_resources.json"
rm -f "$dir/tainted_resources.json"

echo "All tainted resources have been removed from the state."
