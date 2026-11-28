#!/usr/bin/env bash
# Offline compatibility entry point (#8742). No arguments or live smoke mode.
set -euo pipefail
if (($# != 0)); then
  printf 'usage: %s (no arguments)\n' "${0##*/}" >&2
  exit 2
fi
script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
exec python3 -I "$script_dir/run_hermetic_tests.py"
