#!/usr/bin/env bash
# Verify the freshness operator entry point rejects unaccounted
# arguments before touching credentials, files, or systemd.
# Oracle: the script returns usage status 2 and names the argument.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
script="$here/run-freshness.sh"
set +e
output="$(bash "$script" --unexpected 2>&1)"
status=$?
set -e
if [[ $status -ne 2 || $output != *--unexpected* || $output != *Usage:* ]]; then
  printf 'FAIL: %s did not reject an unknown argument (status=%s)\n%s\n' \
    "$script" "$status" "$output" >&2
  exit 1
fi
echo 'test-freshness-arguments.sh: all checks passed'
