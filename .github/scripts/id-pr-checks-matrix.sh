#!/usr/bin/env bash
# Emit the ordered matrix, using the guard's list normalization. The guard
# rejects duplicates; collapse them here as a defensive measure as well.
set -euo pipefail

sed -e 's/\r$//' -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' "${1:-.github/scripts/id-pr-checks.txt}" |
  awk '!/^#/ && NF && !seen[$0]++' |
  jq -R . | jq -cs .
