#!/usr/bin/env bash

set -exuo pipefail

dir="$(dirname -- "$(which -- "$0" 2>/dev/null || realpath -- "$0")")"
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
