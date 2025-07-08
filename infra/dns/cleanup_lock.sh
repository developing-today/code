#!/usr/bin/env bash

set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail

echo "\$0=$0"
script_name="$0"
while [[ "$script_name" == -* ]]; do
    script_name="${script_name#-}"
done

dir="$(dirname -- "$(which -- "$script_name" 2>/dev/null || realpath -- "$script_name")")"
echo "dir: $dir"

if [ -f "$dir/.lock" ]; then
  echo "Lock file exists"
  # exit 1
else
  echo "Lock file does not exist"
fi
echo "Deleting lock file if it exists"
rm -f "$dir/.lock"
echo "Deleted lock file"
