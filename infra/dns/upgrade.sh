#!/usr/bin/env bash

set -exuo pipefail

echo "\$0=$0"
script_name="$0"
while [[ "$script_name" == -* ]]; do
    script_name="${script_name#-}"
done

dir="$(dirname -- "$(which -- "$script_name" 2>/dev/null || realpath -- "$script_name")")"
echo "dir: $dir"

echo "init.sh -upgrade"
"$dir/init.sh" -upgrade
echo "init.sh -upgrade done"
