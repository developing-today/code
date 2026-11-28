#!/usr/bin/env bash
# Verify new R2 operator entry points reject unaccounted arguments before
# touching credentials, files, or systemd.
# Oracle: each script returns usage status 2 and names the offending argument.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
for script in "$here/run-r2.sh" "$here/install-units.sh"; do
  set +e
  output="$(bash "$script" --unexpected 2>&1)"
  status=$?
  set -e
  if [[ $status -ne 2 || $output != *--unexpected* || $output != *Usage:* ]]; then
    printf 'FAIL: %s did not reject an unknown argument (status=%s)\n%s\n' \
      "$script" "$status" "$output" >&2
    exit 1
  fi
done
echo 'test-r2-arguments.sh: all checks passed'
