#!/usr/bin/env bash

set -exuo pipefail

dir="$(dirname -- "$(which -- "$0" 2>/dev/null || realpath -- "$0")")"
echo "dir: $dir"

echo "init.sh -upgrade"
$dir/init.sh -upgrade
echo "init.sh -upgrade done"
